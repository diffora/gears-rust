//! Tests for multipart upload and upload idempotency.
//!
//! The server-authoritative multipart model (multipart-coordinator feature) is
//! exercised here. Part bytes no longer flow through the control plane — the
//! control plane returns a parts plan with signed sidecar URLs. Tests simulate
//! the sidecar's side by:
//!
//!   1. Getting the plan from `initiate_multipart_upload`.
//!   2. Fetching the backend upload handle from the session row.
//!   3. Writing part bytes via `backend.upload_part(path, handle, n, data)` —
//!      the path a production sidecar would take for a `multipart_native` backend.
//!   4. Persisting the part row via `MultipartStore::upsert_multipart_part`
//!      (simulating the sidecar's SDK callback to the control plane).
//!   5. Calling `complete_multipart_upload`.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]

use std::sync::Arc;

use bytes::Bytes;
use sea_orm::{ConnectionTrait, Database, Statement};
use sea_orm_migration::MigratorTrait;
use toolkit_db::migration_runner::run_migrations_for_testing;
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use toolkit_security::SecurityContext;
use uuid::Uuid;

use file_storage::domain::authz::TenantOnlyAuthorizer;
use file_storage::domain::data_plane::DataPlaneService;
use file_storage::domain::error::DomainError;
use file_storage::domain::idempotency::compute_request_hash;
use file_storage::domain::multipart::MultipartPlan;
use file_storage::domain::multipart_service::MultipartService;
use file_storage::domain::policy::{PolicyBody, PolicyScope, SizeLimits};
use file_storage::domain::policy_service::PolicyService;
use file_storage::domain::ports::{DataPlanePort, MultipartStore, PolicyStore};
use file_storage::domain::service::{FileService, ServiceConfig};
use file_storage::infra::backend::{
    BackendRegistry, InMemoryBackend, LocalFsBackend, StorageBackend,
};
use file_storage::infra::signed_url::Issuer;
use file_storage::infra::storage::Store;
use file_storage::infra::storage::migrations::Migrator;
use file_storage_sdk::{CustomMetadataEntry, NewFile, OwnerKind};

const GTS: &str = "gts.cf.fstorage.file.type.v1~x.test.v1~";

/// Build a fresh migrated SQLite DB, returning both the pooled `DBProvider`
/// (for the service under test) and the raw DSN (for the idempotency
/// mismatch tests below, which need a second, independent raw connection to
/// inspect/tamper with `idempotency_keys` rows directly — there is no
/// production API for either, by design: a stored record is immutable once
/// written).
async fn build_db_with_dsn() -> (Arc<DBProvider<DbError>>, String) {
    let mut path = std::env::temp_dir();
    path.push(format!("cf-fs-mp-test-{}.db", Uuid::now_v7().simple()));
    let mut file = path.to_string_lossy().replace('\\', "/");
    if !file.starts_with('/') {
        file.insert(0, '/');
    }
    let dsn = format!("sqlite://{file}?mode=rwc");
    let opts = ConnectOpts {
        max_conns: Some(1),
        min_conns: Some(1),
        ..Default::default()
    };
    let db = connect_db(&dsn, opts).await.expect("connect sqlite");
    run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("migrations");
    (Arc::new(DBProvider::new(db)), dsn)
}

async fn build_db() -> Arc<DBProvider<DbError>> {
    build_db_with_dsn().await.0
}

/// Build both `FileService` and `MultipartService` sharing the same store,
/// backends, and authorizer.
async fn build_service_with_config(
    idempotency_ttl_secs: u64,
) -> (Arc<FileService>, Arc<MultipartService>, DataPlaneService) {
    let db = build_db().await;
    let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new("mem"));
    let backends = BackendRegistry::new(vec![backend], "mem").expect("registry");
    let issuer = Arc::new(Issuer::generate(3600).expect("issuer"));
    let authorizer: Arc<dyn file_storage::domain::authz::Authorizer> =
        Arc::new(TenantOnlyAuthorizer);
    let cfg = ServiceConfig {
        default_url_ttl_secs: 3600,
        sidecar_base_url: "http://sidecar.test".to_owned(),
        default_page_size: 50,
        max_page_size: 1000,
        idempotency_ttl_secs,
    };
    let store = Store::new(Arc::clone(&db));
    let svc = Arc::new(FileService::new(
        store.clone(),
        backends.clone(),
        Arc::clone(&issuer),
        Arc::clone(&authorizer),
        cfg,
        None,
        None,
    ));
    let msvc = Arc::new(MultipartService::new(
        Arc::new(store) as Arc<dyn MultipartStore>,
        backends,
        Arc::clone(&authorizer),
        None,
        issuer,
        "http://sidecar.test".to_owned(),
        3600,
    ));
    let dp = DataPlaneService::new(Arc::clone(&svc) as Arc<dyn DataPlanePort>);
    (svc, msvc, dp)
}

async fn build_service() -> (Arc<FileService>, Arc<MultipartService>, DataPlaneService) {
    build_service_with_config(86400).await
}

/// Build a `FileService` alone (no `MultipartService`) plus the raw SQLite
/// DSN, for the idempotency-replay body-mismatch tests (P2 remediation 2.1)
/// below.
async fn build_file_service_with_dsn(idempotency_ttl_secs: u64) -> (Arc<FileService>, String) {
    let (db, dsn) = build_db_with_dsn().await;
    let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new("mem"));
    let backends = BackendRegistry::new(vec![backend], "mem").expect("registry");
    let issuer = Arc::new(Issuer::generate(3600).expect("issuer"));
    let authorizer: Arc<dyn file_storage::domain::authz::Authorizer> =
        Arc::new(TenantOnlyAuthorizer);
    let cfg = ServiceConfig {
        default_url_ttl_secs: 3600,
        sidecar_base_url: "http://sidecar.test".to_owned(),
        default_page_size: 50,
        max_page_size: 1000,
        idempotency_ttl_secs,
    };
    let store = Store::new(Arc::clone(&db));
    let svc = Arc::new(FileService::new(
        store, backends, issuer, authorizer, cfg, None, None,
    ));
    (svc, dsn)
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut acc, b| {
        write!(acc, "{b:02x}").expect("writing to a String cannot fail");
        acc
    })
}

/// Raw-SQL row count over `files`, bypassing the service/store layer —
/// used to assert that a rejected idempotency replay never creates a second
/// file (P2 remediation 2.1).
async fn count_files_rows(dsn: &str) -> i64 {
    let conn = Database::connect(dsn).await.expect("raw connect");
    let row = conn
        .query_one(Statement::from_string(
            conn.get_database_backend(),
            "SELECT COUNT(*) AS c FROM files".to_owned(),
        ))
        .await
        .expect("count query")
        .expect("one row");
    row.try_get::<i64>("", "c").expect("i64 column c")
}

/// Overwrite the `request_hash` of a live idempotency row directly via raw
/// SQL — there is no production API to do this (a stored record is
/// immutable once written). Used only to simulate a hash computed for a
/// different owner than the row's own primary key, to exercise the "owner is
/// covered by the hash" leg of the mismatch check: `owner_kind`/`owner_id`
/// are themselves part of `idempotency_keys`' composite primary key, so an
/// ordinary replay with a genuinely different owner can never even find the
/// original row (see `idempotency_different_owner_different_file`, which is
/// the correct, already-covered behavior for that case — a fresh,
/// independent file gets created, not a conflict).
async fn tamper_request_hash(
    dsn: &str,
    tenant_id: Uuid,
    owner_kind: &str,
    owner_id: Uuid,
    key: &str,
    request_hash: &[u8],
) {
    let conn = Database::connect(dsn).await.expect("raw connect");
    // `sea_orm`'s sqlite driver binds `Uuid` columns as a raw 16-byte BLOB
    // (not a hyphenated TEXT string), unlike the plain single-quoted string
    // literals `migration_test.rs` uses against its own hand-written DDL
    // inserts — so the composite-key match here must use `X'...'` blob
    // literals for `tenant_id`/`owner_id`, built from `Uuid::as_bytes()`, to
    // agree with what the entity layer actually persisted.
    let tenant_hex = hex_encode(tenant_id.as_bytes());
    let owner_hex = hex_encode(owner_id.as_bytes());
    let hash_hex = hex_encode(request_hash);
    let sql = format!(
        "UPDATE idempotency_keys SET request_hash = X'{hash_hex}' \
             WHERE tenant_id = X'{tenant_hex}' AND owner_kind = '{owner_kind}' \
             AND owner_id = X'{owner_hex}' AND idempotency_key = '{key}'"
    );
    let res = conn
        .execute(Statement::from_string(conn.get_database_backend(), sql))
        .await
        .expect("tamper request_hash");
    assert_eq!(
        res.rows_affected(),
        1,
        "tamper UPDATE must hit exactly the one row created by the test setup"
    );
}

/// Like `build_service` but also returns a `PolicyService` so tests can
/// configure size-limit policies.
async fn build_service_with_policy() -> (
    Arc<FileService>,
    Arc<MultipartService>,
    Arc<PolicyService>,
    DataPlaneService,
) {
    let db = build_db().await;
    let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new("mem"));
    let backends = BackendRegistry::new(vec![backend], "mem").expect("registry");
    let issuer = Arc::new(Issuer::generate(3600).expect("issuer"));
    let authorizer: Arc<dyn file_storage::domain::authz::Authorizer> =
        Arc::new(TenantOnlyAuthorizer);
    let cfg = ServiceConfig {
        default_url_ttl_secs: 3600,
        sidecar_base_url: "http://sidecar.test".to_owned(),
        default_page_size: 50,
        max_page_size: 1000,
        idempotency_ttl_secs: 86400,
    };
    let store = Store::new(Arc::clone(&db));
    let policy_store: Arc<dyn PolicyStore> = Arc::new(store.clone());
    let svc = Arc::new(FileService::new(
        store.clone(),
        backends.clone(),
        Arc::clone(&issuer),
        Arc::clone(&authorizer),
        cfg,
        None,
        None,
    ));
    let msvc = Arc::new(MultipartService::new(
        Arc::new(store) as Arc<dyn MultipartStore>,
        backends,
        Arc::clone(&authorizer),
        None,
        Arc::clone(&issuer),
        "http://sidecar.test".to_owned(),
        3600,
    ));
    let dp = DataPlaneService::new(Arc::clone(&svc) as Arc<dyn DataPlanePort>);
    let psvc = Arc::new(PolicyService::new(policy_store, authorizer));
    (svc, msvc, psvc, dp)
}

fn ctx(tenant: Uuid) -> SecurityContext {
    SecurityContext::builder()
        .subject_id(Uuid::now_v7())
        .subject_tenant_id(tenant)
        .build()
        .expect("ctx")
}

fn new_file() -> NewFile {
    NewFile {
        owner_kind: OwnerKind::User,
        owner_id: Uuid::now_v7(),
        name: "upload.bin".to_owned(),
        gts_file_type: GTS.to_owned(),
        mime_type: "application/octet-stream".to_owned(),
        custom_metadata: vec![],
    }
}

/// Simulate the sidecar writing a part for a `multipart_native` backend.
///
/// The production sidecar for a native-multipart backend calls:
///   1. `backend.upload_part(path, handle, part_number, data)` — stores part
///      bytes in the backend's native multipart state.
///   2. `store.upsert_multipart_part(...)` — records the part row (ETag, hash,
///      size) in the control-plane DB so `complete` can assemble correctly.
///
/// This function performs both steps so tests don't have to duplicate the dance.
async fn simulate_sidecar_put_part(
    store: &Arc<dyn MultipartStore>,
    backend: &Arc<dyn StorageBackend>,
    plan: &MultipartPlan,
    backend_path: &str,
    backend_handle: &str,
    part_number: u32,
    data: Bytes,
) {
    let part = plan
        .parts
        .iter()
        .find(|p| p.part_number == part_number)
        .unwrap_or_else(|| panic!("part {part_number} not in plan"));

    // Simulate the sidecar's size enforcement gate (FEATURE §4, point 2).
    assert_eq!(
        data.len() as u64,
        part.size,
        "part {part_number}: simulated sidecar size enforcement — body len {} != plan size {}",
        data.len(),
        part.size,
    );

    // Upload through the backend's native multipart path (upload_part => keyed
    // by the upload handle for later assembly in complete_multipart).
    let (backend_etag, part_hash) = backend
        .upload_part(backend_path, backend_handle, part_number, data)
        .await
        .expect("backend upload_part");

    let size = i64::try_from(part.size).unwrap();
    let now = time::OffsetDateTime::now_utc();
    let part_number_i32 = i32::try_from(part_number).unwrap();

    // Persist the part row (sidecar calls back via SDK).
    store
        .upsert_multipart_part(
            plan.upload_id,
            part_number_i32,
            &backend_etag,
            part_hash,
            size,
            now,
        )
        .await
        .unwrap();
}

// -- 1. Multipart happy path --------------------------------------------------

/// Server-authoritative multipart: initiate returns a plan, sidecar simulated
/// part writes (via native upload_part), complete assembles.
///
/// @cpt-cf-file-storage-fr-multipart-upload
#[tokio::test]
async fn multipart_happy_path_in_memory() {
    let db = build_db().await;
    let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new("mem"));
    let backends = BackendRegistry::new(vec![Arc::clone(&backend)], "mem").expect("registry");
    let issuer = Arc::new(Issuer::generate(3600).expect("issuer"));
    let authorizer: Arc<dyn file_storage::domain::authz::Authorizer> =
        Arc::new(TenantOnlyAuthorizer);
    let cfg = ServiceConfig {
        default_url_ttl_secs: 3600,
        sidecar_base_url: "http://sidecar.test".to_owned(),
        default_page_size: 50,
        max_page_size: 1000,
        idempotency_ttl_secs: 86400,
    };
    let store = Store::new(Arc::clone(&db));
    let multipart_store: Arc<dyn MultipartStore> = Arc::new(store.clone());
    let svc = Arc::new(FileService::new(
        store.clone(),
        backends.clone(),
        Arc::clone(&issuer),
        Arc::clone(&authorizer),
        cfg,
        None,
        None,
    ));
    let msvc = Arc::new(MultipartService::new(
        Arc::clone(&multipart_store),
        backends,
        Arc::clone(&authorizer),
        None,
        issuer,
        "http://sidecar.test".to_owned(),
        3600,
    ));
    let dp = DataPlaneService::new(Arc::clone(&svc) as Arc<dyn DataPlanePort>);
    let ctx = ctx(Uuid::now_v7());

    // Create the file (pending, no content yet).
    let ticket = svc.create_file(&ctx, new_file(), None).await.unwrap();

    // Declare total size = 13 bytes ("Hello, World!").
    let declared_size = 13u64;
    let plan = msvc
        .initiate_multipart_upload(
            &ctx,
            ticket.file_id,
            "application/octet-stream",
            declared_size,
            None,
            None,
        )
        .await
        .unwrap();

    assert_eq!(plan.parts.len(), 1, "13 bytes fits in one part");
    assert!(!plan.upload_id.is_nil());

    // Verify the single plan entry.
    let p = &plan.parts[0];
    assert_eq!(p.part_number, 1);
    assert_eq!(p.offset, 0);
    assert_eq!(p.size, declared_size);
    assert!(!p.upload_url.is_empty());

    // Retrieve the backend_upload_handle from the session row so we can feed it
    // to `backend.upload_part` (production sidecar would get it from the token
    // claims; in tests we fetch it directly).
    let session = multipart_store
        .get_multipart_upload(plan.upload_id)
        .await
        .unwrap()
        .expect("session must exist");
    let backend_path = format!("/{}/{}", ticket.file_id, plan.version_id);

    // Simulate the sidecar: write part 1 via native multipart.
    let data = Bytes::from_static(b"Hello, World!");
    simulate_sidecar_put_part(
        &multipart_store,
        &backend,
        &plan,
        &backend_path,
        &session.backend_upload_handle,
        1,
        data,
    )
    .await;

    // Complete: the service assembles the backend blobs and finalizes the
    // version row. Internally calls `backend.complete_multipart(path, handle, parts)`.
    msvc.complete_multipart_upload(&ctx, ticket.file_id, plan.upload_id)
        .await
        .unwrap();

    // Bind the completed version (version is now `Available`).
    svc.bind(&ctx, ticket.file_id, plan.version_id, None)
        .await
        .unwrap();

    // Read back via data plane.
    let content = dp
        .read_content(&ctx, ticket.file_id, plan.version_id, None)
        .await
        .unwrap();
    assert_eq!(content, Bytes::from_static(b"Hello, World!"));
}

// -- 1c. Completing an already-completed session is rejected ------------------

/// A second `complete_multipart_upload` call for the same `upload_id`, after
/// the first call already finalized the version and flipped the session to
/// `completed`, must be rejected — not silently re-accepted or allowed to
/// re-finalize the version.
///
/// This is the session-level guard (`MultipartUploadNotInProgress`), which
/// sits in front of the P2 0.4 version-level CAS guard in
/// `VersionRepo::finalize`: this test confirms the session-level guard alone
/// already rejects the replay here, so the version-level guard is
/// defense-in-depth behind it, not the only line of defense.
///
/// @cpt-cf-file-storage-fr-multipart-upload
#[tokio::test]
async fn multipart_complete_after_already_finalized_is_rejected() {
    let db = build_db().await;
    let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new("mem"));
    let backends = BackendRegistry::new(vec![Arc::clone(&backend)], "mem").expect("registry");
    let issuer = Arc::new(Issuer::generate(3600).expect("issuer"));
    let authorizer: Arc<dyn file_storage::domain::authz::Authorizer> =
        Arc::new(TenantOnlyAuthorizer);
    let cfg = ServiceConfig {
        default_url_ttl_secs: 3600,
        sidecar_base_url: "http://sidecar.test".to_owned(),
        default_page_size: 50,
        max_page_size: 1000,
        idempotency_ttl_secs: 86400,
    };
    let store = Store::new(Arc::clone(&db));
    let multipart_store: Arc<dyn MultipartStore> = Arc::new(store.clone());
    let svc = Arc::new(FileService::new(
        store.clone(),
        backends.clone(),
        Arc::clone(&issuer),
        Arc::clone(&authorizer),
        cfg,
        None,
        None,
    ));
    let msvc = Arc::new(MultipartService::new(
        Arc::clone(&multipart_store),
        backends,
        Arc::clone(&authorizer),
        None,
        issuer,
        "http://sidecar.test".to_owned(),
        3600,
    ));
    let ctx = ctx(Uuid::now_v7());

    let ticket = svc.create_file(&ctx, new_file(), None).await.unwrap();
    let declared_size = 13u64;
    let plan = msvc
        .initiate_multipart_upload(
            &ctx,
            ticket.file_id,
            "application/octet-stream",
            declared_size,
            None,
            None,
        )
        .await
        .unwrap();
    let session = multipart_store
        .get_multipart_upload(plan.upload_id)
        .await
        .unwrap()
        .expect("session must exist");
    let backend_path = format!("/{}/{}", ticket.file_id, plan.version_id);
    simulate_sidecar_put_part(
        &multipart_store,
        &backend,
        &plan,
        &backend_path,
        &session.backend_upload_handle,
        1,
        Bytes::from_static(b"Hello, World!"),
    )
    .await;

    // First complete: succeeds, finalizes the version, flips the session to
    // `completed`.
    msvc.complete_multipart_upload(&ctx, ticket.file_id, plan.upload_id)
        .await
        .unwrap();

    // Second complete for the same upload_id: the session is no longer
    // `in_progress`, so this must be rejected before ever touching the
    // version-level CAS.
    let err = msvc
        .complete_multipart_upload(&ctx, ticket.file_id, plan.upload_id)
        .await
        .unwrap_err();
    assert!(
        matches!(err, DomainError::MultipartUploadNotInProgress { .. }),
        "expected MultipartUploadNotInProgress, got {err:?}"
    );
}

// -- 1b. Full lifecycle: create -> multipart upload -> bind -> delete ---------

/// A multipart-uploaded file must be fully removable end to end: create it,
/// upload its content through the server-authoritative multipart flow, complete
/// + bind it, confirm it exists and is readable, then delete it and confirm the
/// file (and its versions, via FK cascade) are gone.
///
/// @cpt-cf-file-storage-fr-multipart-upload
/// @cpt-cf-file-storage-fr-audit-trail
#[tokio::test]
async fn multipart_full_lifecycle_create_to_delete() {
    let db = build_db().await;
    let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new("mem"));
    let backends = BackendRegistry::new(vec![Arc::clone(&backend)], "mem").expect("registry");
    let issuer = Arc::new(Issuer::generate(3600).expect("issuer"));
    let authorizer: Arc<dyn file_storage::domain::authz::Authorizer> =
        Arc::new(TenantOnlyAuthorizer);
    let cfg = ServiceConfig {
        default_url_ttl_secs: 3600,
        sidecar_base_url: "http://sidecar.test".to_owned(),
        default_page_size: 50,
        max_page_size: 1000,
        idempotency_ttl_secs: 86400,
    };
    let store = Store::new(Arc::clone(&db));
    let multipart_store: Arc<dyn MultipartStore> = Arc::new(store.clone());
    let svc = FileService::new(
        store.clone(),
        backends.clone(),
        Arc::clone(&issuer),
        Arc::clone(&authorizer),
        cfg,
        None,
        None,
    );
    let msvc = MultipartService::new(
        Arc::clone(&multipart_store),
        backends,
        Arc::clone(&authorizer),
        None,
        issuer,
        "http://sidecar.test".to_owned(),
        3600,
    );
    let ctx = ctx(Uuid::now_v7());

    // Create -> initiate -> upload the single part -> complete -> bind.
    let ticket = svc.create_file(&ctx, new_file(), None).await.unwrap();
    let declared_size = 13u64;
    let plan = msvc
        .initiate_multipart_upload(
            &ctx,
            ticket.file_id,
            "application/octet-stream",
            declared_size,
            None,
            None,
        )
        .await
        .unwrap();
    let session = multipart_store
        .get_multipart_upload(plan.upload_id)
        .await
        .unwrap()
        .expect("session must exist");
    let backend_path = format!("/{}/{}", ticket.file_id, plan.version_id);
    simulate_sidecar_put_part(
        &multipart_store,
        &backend,
        &plan,
        &backend_path,
        &session.backend_upload_handle,
        1,
        Bytes::from_static(b"Hello, World!"),
    )
    .await;
    msvc.complete_multipart_upload(&ctx, ticket.file_id, plan.upload_id)
        .await
        .unwrap();
    svc.bind(&ctx, ticket.file_id, plan.version_id, None)
        .await
        .unwrap();

    // The multipart file exists and has its bound version before deletion.
    svc.get_file(&ctx, ticket.file_id)
        .await
        .expect("file must exist before delete");
    assert!(
        svc.list_versions(&ctx, ticket.file_id, None, 0)
            .await
            .unwrap()
            .iter()
            .any(|v| v.version_id == plan.version_id),
        "the completed multipart version must be present before delete",
    );

    // Delete the multipart-uploaded file (If-Match `*` = unconditional).
    svc.delete_file(&ctx, ticket.file_id, Some("*"))
        .await
        .expect("delete must succeed");

    // The file — and its versions via FK cascade — must be gone.
    assert!(
        matches!(
            svc.get_file(&ctx, ticket.file_id).await,
            Err(DomainError::FileNotFound { .. })
        ),
        "file must be FileNotFound after delete",
    );
}

// -- 2. LocalFsBackend rejects multipart -------------------------------------

/// @cpt-cf-file-storage-fr-multipart-upload
#[tokio::test]
async fn multipart_rejected_on_local_fs() {
    let db = build_db().await;
    let tmp = std::env::temp_dir().join(format!("cf-fs-localfs-{}", Uuid::now_v7().simple()));
    std::fs::create_dir_all(&tmp).unwrap();
    let local: Arc<dyn StorageBackend> = Arc::new(LocalFsBackend::new("local-fs", &tmp));
    let backends = BackendRegistry::new(vec![local], "local-fs").expect("registry");
    let issuer = Arc::new(Issuer::generate(3600).expect("issuer"));
    let authorizer: Arc<dyn file_storage::domain::authz::Authorizer> =
        Arc::new(TenantOnlyAuthorizer);
    let cfg = ServiceConfig {
        default_url_ttl_secs: 3600,
        sidecar_base_url: "http://sidecar.test".to_owned(),
        default_page_size: 50,
        max_page_size: 1000,
        idempotency_ttl_secs: 86400,
    };
    let store = Store::new(Arc::clone(&db));
    let svc = Arc::new(FileService::new(
        store.clone(),
        backends.clone(),
        Arc::clone(&issuer),
        Arc::clone(&authorizer),
        cfg,
        None,
        None,
    ));
    let msvc = Arc::new(MultipartService::new(
        Arc::new(store) as Arc<dyn MultipartStore>,
        backends,
        authorizer,
        None,
        issuer,
        "http://sidecar.test".to_owned(),
        3600,
    ));

    let ctx = ctx(Uuid::now_v7());
    let ticket = svc.create_file(&ctx, new_file(), None).await.unwrap();

    let err = msvc
        .initiate_multipart_upload(
            &ctx,
            ticket.file_id,
            "application/octet-stream",
            1024,
            None,
            None,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, DomainError::MultipartNotSupported { .. }),
        "expected MultipartNotSupported, got {err:?}"
    );
}

// -- 3. Initiate returns a coherent parts plan --------------------------------

/// The server computes the plan deterministically:
/// - `parts = ceil(declared_size / part_size)`.
/// - Last part's `size = declared_size - (n-1) * part_size`.
/// - Sum of all parts' sizes == declared_size.
///
/// @cpt-cf-file-storage-fr-multipart-upload
#[tokio::test]
async fn initiate_returns_coherent_parts_plan() {
    let (svc, msvc, _dp) = build_service().await;
    let ctx = ctx(Uuid::now_v7());
    let ticket = svc.create_file(&ctx, new_file(), None).await.unwrap();

    // Use the minimum valid preferred_part_size to force multiple parts.
    // (P2 remediation 2.11 rejects any preferred_part_size below
    // `DEFAULT_MIN_PART_SIZE`, so this can no longer use tiny byte values —
    // scale everything up by the same 5:5:3 ratio the original test used.)
    let part_size = 5 * 1024 * 1024u64; // DEFAULT_MIN_PART_SIZE
    let declared_size = 2 * part_size + 3;
    let preferred_part_size = Some(part_size); // forces plan: [part_size, part_size, 3]
    let plan = msvc
        .initiate_multipart_upload(
            &ctx,
            ticket.file_id,
            "application/octet-stream",
            declared_size,
            preferred_part_size,
            Some(3),
        )
        .await
        .unwrap();

    assert!(!plan.upload_id.is_nil());
    assert!(!plan.parts.is_empty());
    assert_eq!(plan.part_hash_algorithm, "SHA-256");

    // Verify plan invariants.
    let mut total = 0u64;
    let mut prev_offset = 0u64;
    for (i, p) in plan.parts.iter().enumerate() {
        assert_eq!(
            p.part_number as usize,
            i + 1,
            "parts must be 1-based sequential"
        );
        assert_eq!(p.offset, prev_offset, "offset must be contiguous");
        assert!(p.size > 0, "part size must be positive");
        assert!(!p.upload_url.is_empty(), "upload_url must not be empty");
        assert!(
            p.upload_url.contains("sidecar.test"),
            "upload_url must point at sidecar"
        );
        assert!(
            p.upload_url.contains("fs-token"),
            "upload_url must contain fs-token"
        );
        total += p.size;
        prev_offset += p.size;
    }
    assert_eq!(
        total, declared_size,
        "sum of part sizes must equal declared_size"
    );
}

// -- 4. Idempotency: same key returns same file --------------------------------

/// @cpt-cf-file-storage-fr-upload-idempotency
#[tokio::test]
async fn idempotency_same_key_returns_same_file() {
    let (svc, _msvc, _dp) = build_service().await;
    let ctx = ctx(Uuid::now_v7());

    let mut nf = new_file();
    let owner_id = nf.owner_id;
    let key = "idem-key-1".to_owned();

    let t1 = svc
        .create_file(&ctx, nf.clone(), Some(key.clone()))
        .await
        .unwrap();

    // Second request with the same key -> same file_id returned.
    nf.owner_id = owner_id; // same owner
    let t2 = svc.create_file(&ctx, nf, Some(key)).await.unwrap();

    assert_eq!(
        t1.file_id, t2.file_id,
        "idempotent retry must return the same file_id"
    );
    assert_eq!(t1.version_id, t2.version_id);
}

// -- 4b. Idempotency replay body-match verification (P2 remediation 2.1) -----

/// A retry with the same `idempotency_key` but a different `name` must be
/// rejected with `409 Conflict` instead of silently replaying the original
/// ticket, and must never create a second file.
///
/// @cpt-cf-file-storage-fr-upload-idempotency
#[tokio::test]
async fn idempotency_replay_with_diverging_name_returns_conflict() {
    let (svc, dsn) = build_file_service_with_dsn(86400).await;
    let ctx = ctx(Uuid::now_v7());
    let key = "diverging-name-key".to_owned();

    let mut nf = new_file();
    nf.name = "original.bin".to_owned();
    svc.create_file(&ctx, nf.clone(), Some(key.clone()))
        .await
        .unwrap();

    nf.name = "different.bin".to_owned();
    let err = svc.create_file(&ctx, nf, Some(key)).await.unwrap_err();
    assert!(
        matches!(err, DomainError::Conflict { .. }),
        "expected Conflict on a diverging name, got {err:?}"
    );
    assert_eq!(
        count_files_rows(&dsn).await,
        1,
        "a rejected replay must not create a second file"
    );
}

/// Same as above, but the divergence is in `custom_metadata` — proving the
/// canonicalization actually covers metadata and not just the scalar fields.
///
/// @cpt-cf-file-storage-fr-upload-idempotency
#[tokio::test]
async fn idempotency_replay_with_diverging_metadata_returns_conflict() {
    let (svc, dsn) = build_file_service_with_dsn(86400).await;
    let ctx = ctx(Uuid::now_v7());
    let key = "diverging-metadata-key".to_owned();

    let mut nf = new_file();
    nf.custom_metadata = vec![CustomMetadataEntry {
        key: "tag".to_owned(),
        value: "a".to_owned(),
    }];
    svc.create_file(&ctx, nf.clone(), Some(key.clone()))
        .await
        .unwrap();

    nf.custom_metadata = vec![CustomMetadataEntry {
        key: "tag".to_owned(),
        value: "b".to_owned(),
    }];
    let err = svc.create_file(&ctx, nf, Some(key)).await.unwrap_err();
    assert!(
        matches!(err, DomainError::Conflict { .. }),
        "expected Conflict on diverging metadata, got {err:?}"
    );
    assert_eq!(
        count_files_rows(&dsn).await,
        1,
        "a rejected replay must not create a second file"
    );
}

/// `owner_kind`/`owner_id` are themselves part of `idempotency_keys`'
/// composite primary key `(tenant_id, owner_kind, owner_id,
/// idempotency_key)`, so an ordinary replay with a genuinely different owner
/// can never even find the original row — it takes the already-covered
/// "different owner -> different file" path (see
/// `idempotency_different_owner_different_file`), not this conflict path.
/// To still exercise the owner leg of the hash comparison itself (guarding
/// against a future regression that silently drops `owner_id` from the
/// canonicalization), this test tampers the stored `request_hash` directly
/// to look as if it had been computed for a different owner than the row's
/// own primary key, then replays with the row's *actual* owner and expects
/// the recomputed (correct) hash to disagree with the tampered one.
///
/// @cpt-cf-file-storage-fr-upload-idempotency
#[tokio::test]
async fn idempotency_replay_with_diverging_owner_returns_conflict() {
    let (svc, dsn) = build_file_service_with_dsn(86400).await;
    let ctx = ctx(Uuid::now_v7());
    let key = "diverging-owner-key".to_owned();

    let nf = new_file();
    svc.create_file(&ctx, nf.clone(), Some(key.clone()))
        .await
        .unwrap();

    let other_owner = Uuid::now_v7();
    let tampered_hash = compute_request_hash(
        nf.owner_kind.as_str(),
        other_owner,
        &nf.name,
        &nf.gts_file_type,
        &nf.mime_type,
        &[],
    );
    tamper_request_hash(
        &dsn,
        ctx.subject_tenant_id(),
        nf.owner_kind.as_str(),
        nf.owner_id,
        &key,
        &tampered_hash,
    )
    .await;

    let err = svc.create_file(&ctx, nf, Some(key)).await.unwrap_err();
    assert!(
        matches!(err, DomainError::Conflict { .. }),
        "expected Conflict when the stored hash reflects a different owner, got {err:?}"
    );
    assert_eq!(
        count_files_rows(&dsn).await,
        1,
        "a rejected replay must not create a second file"
    );
}

// -- 5. Different owner -> different file -------------------------------------

/// @cpt-cf-file-storage-fr-upload-idempotency
#[tokio::test]
async fn idempotency_different_owner_different_file() {
    let (svc, _msvc, _dp) = build_service().await;
    let tenant = Uuid::now_v7();
    let ctx_a = ctx(tenant);
    let ctx_b = ctx(tenant); // same tenant, different subject (different owner_id in NewFile)

    let key = "shared-key".to_owned();

    let mut nf_a = new_file();
    nf_a.owner_id = Uuid::now_v7();
    let mut nf_b = new_file();
    nf_b.owner_id = Uuid::now_v7(); // different owner_id

    let t_a = svc
        .create_file(&ctx_a, nf_a, Some(key.clone()))
        .await
        .unwrap();
    let t_b = svc.create_file(&ctx_b, nf_b, Some(key)).await.unwrap();

    assert_ne!(
        t_a.file_id, t_b.file_id,
        "different owners must get distinct files even with the same key"
    );
}

// -- 6. Idempotency expiry creates a fresh file --------------------------------

/// @cpt-cf-file-storage-fr-upload-idempotency
#[tokio::test]
async fn idempotency_expiry_creates_new_file() {
    // Very short TTL: 1 second.
    let (svc, _msvc, _dp) = build_service_with_config(1).await;
    let ctx = ctx(Uuid::now_v7());
    let mut nf = new_file();
    let owner_id = nf.owner_id;

    let key = "expiry-key".to_owned();
    let t1 = svc
        .create_file(&ctx, nf.clone(), Some(key.clone()))
        .await
        .unwrap();

    // Wait for the key to expire.
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    nf.owner_id = owner_id;
    let t2 = svc.create_file(&ctx, nf, Some(key)).await.unwrap();

    assert_ne!(
        t1.file_id, t2.file_id,
        "after expiry, the same key must create a new file"
    );
}

// -- 7. Size enforcement at initiate time (CodeRabbit F2 fix) -----------------

/// Declaring a total size that exceeds the policy limit at initiate time
/// must be rejected immediately -- before any backend state is created.
///
/// This is the DESIGN §4.6 (server-authoritative) fix for CodeRabbit F2: the
/// control plane gates the declared total size at initiate so that an
/// oversized upload cannot be started at all, not merely rejected at complete.
///
/// @cpt-cf-file-storage-fr-multipart-upload
/// @cpt-cf-file-storage-fr-size-limits-policy
#[tokio::test]
async fn initiate_multipart_rejected_when_declared_size_exceeds_policy_limit() {
    let (svc, msvc, psvc, _dp) = build_service_with_policy().await;
    let tenant = Uuid::now_v7();
    let ctx = ctx(tenant);
    let owner = Uuid::now_v7();

    // Set a 10-byte cap at tenant level.
    psvc.set_policy(
        &ctx,
        PolicyScope::Tenant,
        None,
        PolicyBody {
            size_limits: SizeLimits {
                max_bytes: Some(10),
                ..SizeLimits::default()
            },
            ..PolicyBody::default()
        },
    )
    .await
    .unwrap();

    let ticket = svc
        .create_file(
            &ctx,
            NewFile {
                owner_kind: OwnerKind::User,
                owner_id: owner,
                name: "large.bin".to_owned(),
                gts_file_type: GTS.to_owned(),
                mime_type: "application/octet-stream".to_owned(),
                custom_metadata: vec![],
            },
            None,
        )
        .await
        .unwrap();

    // Initiate with declared_size = 11 bytes > 10-byte cap -> must be rejected.
    let err = msvc
        .initiate_multipart_upload(
            &ctx,
            ticket.file_id,
            "application/octet-stream",
            11,
            None,
            None,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, DomainError::PolicySizeExceeded { .. }),
        "expected PolicySizeExceeded at initiate, got {err:?}"
    );
}

/// Declaring a total size within the policy limit succeeds.
///
/// @cpt-cf-file-storage-fr-multipart-upload
/// @cpt-cf-file-storage-fr-size-limits-policy
#[tokio::test]
async fn initiate_multipart_allowed_when_declared_size_within_policy_limit() {
    let (svc, msvc, psvc, _dp) = build_service_with_policy().await;
    let tenant = Uuid::now_v7();
    let ctx = ctx(tenant);
    let owner = Uuid::now_v7();

    // Set a 100-byte cap at tenant level.
    psvc.set_policy(
        &ctx,
        PolicyScope::Tenant,
        None,
        PolicyBody {
            size_limits: SizeLimits {
                max_bytes: Some(100),
                ..SizeLimits::default()
            },
            ..PolicyBody::default()
        },
    )
    .await
    .unwrap();

    let ticket = svc
        .create_file(
            &ctx,
            NewFile {
                owner_kind: OwnerKind::User,
                owner_id: owner,
                name: "small.bin".to_owned(),
                gts_file_type: GTS.to_owned(),
                mime_type: "application/octet-stream".to_owned(),
                custom_metadata: vec![],
            },
            None,
        )
        .await
        .unwrap();

    // Initiate with declared_size = 50 bytes <= 100-byte cap -> must be accepted.
    let plan = msvc
        .initiate_multipart_upload(
            &ctx,
            ticket.file_id,
            "application/octet-stream",
            50,
            None,
            None,
        )
        .await
        .unwrap();
    assert!(!plan.upload_id.is_nil());
}

// -- 7b. `preferred_part_size` range validation (P2 remediation 2.11) --------

/// A client-controlled `preferred_part_size` near `u64::MAX` must be
/// rejected up front with `DomainError::Validation`, not passed through to
/// `compute_plan` where it could overflow the part-size arithmetic or drive
/// a huge `Vec::with_capacity` allocation.
///
/// @cpt-cf-file-storage-fr-multipart-upload
#[tokio::test]
async fn initiate_multipart_rejects_absurd_preferred_part_size() {
    let (svc, msvc, _dp) = build_service().await;
    let ctx = ctx(Uuid::now_v7());
    let ticket = svc.create_file(&ctx, new_file(), None).await.unwrap();

    let err = msvc
        .initiate_multipart_upload(
            &ctx,
            ticket.file_id,
            "application/octet-stream",
            1024,
            Some(u64::MAX),
            None,
        )
        .await
        .unwrap_err();

    assert!(
        matches!(err, DomainError::Validation { .. }),
        "expected Validation for an absurd preferred_part_size, got {err:?}"
    );
}

// -- 8. Per-part signed URLs carry valid multipart tokens ---------------------

/// Each upload_url in the plan must be a valid fs-token-bearing sidecar URL
/// that the Verifier can decode with correct multipart claims.
///
/// @cpt-cf-file-storage-fr-multipart-upload (FEATURE §4)
#[tokio::test]
async fn initiate_plan_urls_carry_valid_multipart_tokens() {
    use file_storage::infra::signed_url::Op;

    let db = build_db().await;
    let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new("mem"));
    let backends = BackendRegistry::new(vec![Arc::clone(&backend)], "mem").expect("registry");
    let issuer = Arc::new(Issuer::generate(3600).expect("issuer"));
    let verifier = issuer.verifier();

    let authorizer: Arc<dyn file_storage::domain::authz::Authorizer> =
        Arc::new(TenantOnlyAuthorizer);
    let cfg = ServiceConfig {
        default_url_ttl_secs: 3600,
        sidecar_base_url: "http://sidecar.test".to_owned(),
        default_page_size: 50,
        max_page_size: 1000,
        idempotency_ttl_secs: 86400,
    };
    let store = Store::new(Arc::clone(&db));
    let svc = Arc::new(FileService::new(
        store.clone(),
        backends.clone(),
        Arc::clone(&issuer),
        Arc::clone(&authorizer),
        cfg,
        None,
        None,
    ));
    let msvc = Arc::new(MultipartService::new(
        Arc::new(store) as Arc<dyn MultipartStore>,
        backends,
        authorizer,
        None,
        Arc::clone(&issuer),
        "http://sidecar.test".to_owned(),
        3600,
    ));

    let ctx = ctx(Uuid::now_v7());
    let ticket = svc.create_file(&ctx, new_file(), None).await.unwrap();

    // P2 remediation 2.11 rejects any preferred_part_size below
    // `DEFAULT_MIN_PART_SIZE`, so this uses the minimum valid part size
    // scaled up from the original tiny-byte example (5:5:3 ratio).
    let part_size = 5 * 1024 * 1024u64; // DEFAULT_MIN_PART_SIZE
    let declared_size = 2 * part_size + 3;
    let plan = msvc
        .initiate_multipart_upload(
            &ctx,
            ticket.file_id,
            "application/octet-stream",
            declared_size,
            Some(part_size),
            None,
        )
        .await
        .unwrap();

    let now = time::OffsetDateTime::now_utc();
    for p in &plan.parts {
        // Extract the token from the URL query parameter.
        let url = &p.upload_url;
        let token_start = url.find("fs-token=").expect("fs-token in URL") + "fs-token=".len();
        let token = &url[token_start..];

        let claims = verifier.verify(token, now).expect("token must verify");

        // Verify op.
        assert_eq!(
            claims.op,
            Op::MultipartPart,
            "op must be MultipartPart for part {}",
            p.part_number
        );
        // Verify scoping claims.
        assert_eq!(claims.file_id, ticket.file_id);
        assert_eq!(claims.version_id, plan.version_id);
        // Verify multipart claims match the plan.
        assert_eq!(claims.multipart.upload_id, plan.upload_id);
        assert_eq!(claims.multipart.part_number, p.part_number);
        assert_eq!(claims.multipart.offset, p.offset);
        assert_eq!(
            claims.multipart.size, p.size,
            "size claim must match plan for part {}",
            p.part_number
        );
    }
}

// -- 9. Real default topology: rejected until a multipart_native backend ----
// is configured as the default (P2 0.2 structural fix group A) --------------

/// Locks in TODAY's real behavior against the *actual* default backend
/// topology `gear.rs`/`build_backend_registry` wires up: `local-fs` is always
/// present and the default; the non-durable `memory` backend only joins when
/// `FileStorageConfig::enable_in_memory_backend` is set, which defaults to
/// `false` (P2 0.5). `LocalFsBackend.multipart_native == false`
/// (`docs/features/multipart-coordinator.md`'s new caveat), so initiate must
/// be rejected against the real default config.
///
/// This deliberately does NOT reuse `gear.rs::build_backend_registry` (a
/// private fn, unreachable from an external integration-test crate) --
/// instead it mirrors its logic verbatim and asserts the flag it branches on
/// is still `false` by default, so a silent flip of either the default or the
/// backend's capability is caught here rather than only in production.
///
/// Flip the assertion once a real default-topology backend sets
/// `multipart_native: true` (S3, item 1.7).
///
/// @cpt-cf-file-storage-fr-multipart-upload
#[tokio::test]
async fn multipart_initiate_against_real_default_topology_is_rejected_until_backend_supports_it() {
    use file_storage::config::FileStorageConfig;

    let db = build_db().await;
    let cfg = FileStorageConfig::default();
    assert!(
        !cfg.enable_in_memory_backend,
        "this test locks in the REAL default topology (local-fs only); if this \
         default flips, the doc caveat in multipart-coordinator.md and this test \
         both need updating"
    );

    // Mirror `gear.rs::build_backend_registry` exactly.
    let mut backend_list: Vec<Arc<dyn StorageBackend>> =
        vec![Arc::new(LocalFsBackend::new("local-fs", &cfg.storage_root))];
    if cfg.enable_in_memory_backend {
        backend_list.push(Arc::new(InMemoryBackend::new("memory")));
    }
    let backends = BackendRegistry::new(backend_list, "local-fs").expect("registry");

    let issuer = Arc::new(Issuer::generate(3600).expect("issuer"));
    let authorizer: Arc<dyn file_storage::domain::authz::Authorizer> =
        Arc::new(TenantOnlyAuthorizer);
    let svc_cfg = ServiceConfig {
        default_url_ttl_secs: 3600,
        sidecar_base_url: "http://sidecar.test".to_owned(),
        default_page_size: 50,
        max_page_size: 1000,
        idempotency_ttl_secs: 86400,
    };
    let store = Store::new(Arc::clone(&db));
    let svc = Arc::new(FileService::new(
        store.clone(),
        backends.clone(),
        Arc::clone(&issuer),
        Arc::clone(&authorizer),
        svc_cfg,
        None,
        None,
    ));
    let msvc = Arc::new(MultipartService::new(
        Arc::new(store) as Arc<dyn MultipartStore>,
        backends,
        authorizer,
        None,
        issuer,
        "http://sidecar.test".to_owned(),
        3600,
    ));

    let ctx = ctx(Uuid::now_v7());
    let ticket = svc.create_file(&ctx, new_file(), None).await.unwrap();

    let err = msvc
        .initiate_multipart_upload(
            &ctx,
            ticket.file_id,
            "application/octet-stream",
            1024,
            None,
            None,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, DomainError::MultipartNotSupported { .. }),
        "expected MultipartNotSupported against the real default topology, got {err:?}"
    );
}

// -- 10. Report-part callback: complete assembles from REPORTED parts, ------
// not a structurally empty list (P2 0.2 structural fix group B) -------------

/// Drives the sidecar's new report-part callback end to end, in-process:
/// initiate -> for each planned part, call `handlers::report_multipart_part`
/// through a minimal real `axum::Router` (a route-registration smoke check
/// for the new route, exercising token verification + JSON decoding +
/// `MultipartService::report_part` for real) -> `complete_multipart_upload`.
///
/// Before P2 0.2 group B, nothing ever called
/// `MultipartStore::upsert_multipart_part` in a real deployment, so
/// `complete_multipart_upload`'s `list_multipart_parts` was always
/// structurally empty. This test asserts the DB state directly via the
/// `multipart_upload_part` entity -- NOT via `list_multipart_parts`, which is
/// the very method under test and would make the assertion tautological.
///
/// @cpt-cf-file-storage-fr-multipart-upload
#[tokio::test]
async fn multipart_complete_uses_reported_parts_not_empty_list() {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::post;
    use sea_orm::EntityTrait;
    use toolkit_db::secure::SecureEntityExt;
    use toolkit_security::AccessScope;
    use tower::ServiceExt;

    use file_storage::api::rest::handlers;
    use file_storage::domain::multipart::DEFAULT_MIN_PART_SIZE;
    use file_storage::infra::signed_url::Verifier;
    use file_storage::infra::storage::entity::multipart_upload_part;

    let db = build_db().await;
    let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new("mem"));
    let backends = BackendRegistry::new(vec![Arc::clone(&backend)], "mem").expect("registry");
    let issuer = Arc::new(Issuer::generate(3600).expect("issuer"));
    let verifier: Arc<Verifier> = Arc::new(issuer.verifier());
    let authorizer: Arc<dyn file_storage::domain::authz::Authorizer> =
        Arc::new(TenantOnlyAuthorizer);
    let cfg = ServiceConfig {
        default_url_ttl_secs: 3600,
        sidecar_base_url: "http://sidecar.test".to_owned(),
        default_page_size: 50,
        max_page_size: 1000,
        idempotency_ttl_secs: 86400,
    };
    let store = Store::new(Arc::clone(&db));
    let svc = Arc::new(FileService::new(
        store.clone(),
        backends.clone(),
        Arc::clone(&issuer),
        Arc::clone(&authorizer),
        cfg,
        None,
        None,
    ));
    let msvc = Arc::new(MultipartService::new(
        Arc::new(store.clone()) as Arc<dyn MultipartStore>,
        backends,
        authorizer,
        None,
        Arc::clone(&issuer),
        "http://sidecar.test".to_owned(),
        3600,
    ));

    let ctx = ctx(Uuid::now_v7());
    let ticket = svc.create_file(&ctx, new_file(), None).await.unwrap();

    // Force a multi-part plan: `preferred_part_size` is floored to
    // `DEFAULT_MIN_PART_SIZE` (`compute_plan`), so declaring just over 2x that
    // floor plans exactly 3 parts: [min, min, 3]. No real bytes are ever
    // written to the backend in this test (only the report-part metadata
    // callback is exercised), so a large declared_size costs nothing.
    let declared_size = 2 * DEFAULT_MIN_PART_SIZE + 3;
    let plan = msvc
        .initiate_multipart_upload(
            &ctx,
            ticket.file_id,
            "application/octet-stream",
            declared_size,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        plan.parts.len(),
        3,
        "declared_size = 2*min + 3 must plan exactly 3 parts"
    );

    // A minimal real router carrying only the report-part route + its two
    // extensions -- exercises `handlers::report_multipart_part` (token
    // verification, path-param binding, JSON decoding) for real, not just
    // the domain method.
    let router = Router::new()
        .route(
            "/api/file-storage/v1/files/{file_id}/versions/{version_id}/multipart/{upload_id}/parts/{part_number}/report",
            post(handlers::report_multipart_part),
        )
        .layer(axum::Extension(Arc::clone(&verifier)))
        .layer(axum::Extension(Arc::clone(&msvc)));

    let mut expected_total: i64 = 0;
    for part in &plan.parts {
        let token_start =
            part.upload_url.find("fs-token=").expect("fs-token in URL") + "fs-token=".len();
        let token = &part.upload_url[token_start..];

        let size = i64::try_from(part.size).unwrap();
        expected_total += size;
        let body = serde_json::json!({
            "backend_etag": format!("etag-{}", part.part_number),
            "hash_hex": hex::encode([u8::try_from(part.part_number % 256).unwrap(); 32]),
            "size": size,
        });

        let uri = format!(
            "/api/file-storage/v1/files/{}/versions/{}/multipart/{}/parts/{}/report",
            ticket.file_id, plan.version_id, plan.upload_id, part.part_number
        );
        let req = Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .header("x-fs-token", token)
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();

        let resp = router.clone().oneshot(req).await.expect("router dispatch");
        assert_eq!(
            resp.status(),
            StatusCode::NO_CONTENT,
            "report_multipart_part must succeed for part {}",
            part.part_number
        );
    }

    // Complete: must assemble from the REPORTED parts, not a structurally
    // empty list.
    msvc.complete_multipart_upload(&ctx, ticket.file_id, plan.upload_id)
        .await
        .unwrap();

    // Assert the DB state directly via the entity, NOT via
    // `list_multipart_parts` -- the very method under test.
    let conn = db.conn().expect("conn");
    let rows = multipart_upload_part::Entity::find()
        .secure()
        .scope_with(&AccessScope::allow_all())
        .all(&conn)
        .await
        .expect("query multipart_upload_parts directly");
    assert_eq!(
        rows.len(),
        plan.parts.len(),
        "multipart_upload_parts must have exactly one row per reported part"
    );
    let db_total: i64 = rows.iter().map(|r| r.size).sum();
    assert_eq!(db_total, expected_total);

    let version = store
        .get_version(ticket.file_id, plan.version_id)
        .await
        .unwrap()
        .expect("version row must exist");
    assert_eq!(
        version.size, db_total,
        "completed version size must equal the sum of reported part sizes"
    );
}

// -- 11. Table-driven: multipart accept/reject tracks backend capability ----

/// Table-driven per P2 0.2: a `local-fs`-only registry rejects initiate
/// (`multipart_native == false`); a `memory`-only registry accepts it
/// (`multipart_native == true`). Complements the single-case
/// `multipart_rejected_on_local_fs` / `multipart_happy_path_in_memory` tests
/// by pinning both sides of the same capability gate in one place.
///
/// @cpt-cf-file-storage-fr-multipart-upload
#[tokio::test]
async fn multipart_initiate_rejected_when_backend_not_multipart_native() {
    struct Case {
        name: &'static str,
        backend: fn() -> Arc<dyn StorageBackend>,
        backend_id: &'static str,
        expect_multipart_supported: bool,
    }

    let cases = [
        Case {
            name: "local-fs-only registry",
            backend: || {
                let tmp =
                    std::env::temp_dir().join(format!("cf-fs-mpn-{}", Uuid::now_v7().simple()));
                std::fs::create_dir_all(&tmp).expect("create tmp dir");
                Arc::new(LocalFsBackend::new("local-fs", tmp)) as Arc<dyn StorageBackend>
            },
            backend_id: "local-fs",
            expect_multipart_supported: false,
        },
        Case {
            name: "memory-only registry",
            backend: || Arc::new(InMemoryBackend::new("memory")) as Arc<dyn StorageBackend>,
            backend_id: "memory",
            expect_multipart_supported: true,
        },
    ];

    for case in cases {
        let db = build_db().await;
        let backend = (case.backend)();
        let backends = BackendRegistry::new(vec![backend], case.backend_id).expect("registry");
        let issuer = Arc::new(Issuer::generate(3600).expect("issuer"));
        let authorizer: Arc<dyn file_storage::domain::authz::Authorizer> =
            Arc::new(TenantOnlyAuthorizer);
        let cfg = ServiceConfig {
            default_url_ttl_secs: 3600,
            sidecar_base_url: "http://sidecar.test".to_owned(),
            default_page_size: 50,
            max_page_size: 1000,
            idempotency_ttl_secs: 86400,
        };
        let store = Store::new(Arc::clone(&db));
        let svc = Arc::new(FileService::new(
            store.clone(),
            backends.clone(),
            Arc::clone(&issuer),
            Arc::clone(&authorizer),
            cfg,
            None,
            None,
        ));
        let msvc = Arc::new(MultipartService::new(
            Arc::new(store) as Arc<dyn MultipartStore>,
            backends,
            authorizer,
            None,
            issuer,
            "http://sidecar.test".to_owned(),
            3600,
        ));

        let ctx = ctx(Uuid::now_v7());
        let ticket = svc.create_file(&ctx, new_file(), None).await.unwrap();

        let result = msvc
            .initiate_multipart_upload(
                &ctx,
                ticket.file_id,
                "application/octet-stream",
                1024,
                None,
                None,
            )
            .await;

        if case.expect_multipart_supported {
            assert!(
                result.is_ok(),
                "case '{}': expected multipart to be accepted, got {:?}",
                case.name,
                result.err()
            );
        } else {
            let err = result.unwrap_err();
            assert!(
                matches!(err, DomainError::MultipartNotSupported { .. }),
                "case '{}': expected MultipartNotSupported, got {err:?}",
                case.name
            );
        }
    }
}
