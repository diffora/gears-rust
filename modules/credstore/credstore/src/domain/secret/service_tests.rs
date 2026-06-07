//! Unit tests for [`Service`] using domain fakes.

use std::sync::Arc;

use authz_resolver_sdk::PolicyEnforcer;
use credstore_sdk::{
    CredStorePluginClientV1, OwnerId, SecretRef, SecretValue, SharingMode, TenantId,
};
use uuid::Uuid;

use crate::domain::error::DomainError;
use crate::domain::secret::model::{SecretRow, SecretStatus};
use crate::domain::secret::repo::SecretRepo;
use crate::domain::secret::service::Service;
use modkit_security::AccessScope;

use crate::domain::secret::test_support::{
    FakeDir, FakeMetrics, FakePlugin, FakePluginSelector, FakeSecretRepo, NoopMetrics,
    deny_enforcer, failing_enforcer, make_ctx, mock_enforcer,
};

fn key(s: &str) -> SecretRef {
    SecretRef::new(s).expect("valid key")
}

fn make_service(
    repo: Arc<FakeSecretRepo>,
    dir: Arc<FakeDir>,
    enforcer: PolicyEnforcer,
    plugin: Arc<FakePlugin>,
    metrics: Arc<FakeMetrics>,
) -> Service {
    let selector = Arc::new(FakePluginSelector::new(plugin));
    Service::new(repo, dir, enforcer, selector, metrics, 60, 300)
}

fn make_service_noop(
    repo: Arc<FakeSecretRepo>,
    dir: Arc<FakeDir>,
    enforcer: PolicyEnforcer,
    plugin: Arc<FakePlugin>,
) -> Service {
    let selector = Arc::new(FakePluginSelector::new(plugin));
    Service::new(
        repo,
        dir,
        enforcer,
        selector,
        Arc::new(NoopMetrics),
        60,
        300,
    )
}

// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn get_own_tenant_secret_returns_hit_own() {
    let tenant = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let ctx = make_ctx(subject, tenant);
    let k = key("mykey");

    let plugin = FakePlugin::new();
    let repo = Arc::new(FakeSecretRepo::new());
    let metrics = FakeMetrics::new();

    // Pre-seed the plugin with a value.
    let plugin_key_owner: Option<&OwnerId> = None;
    plugin
        .put(
            &ctx,
            &TenantId(tenant),
            &k,
            SecretValue::from("v1"),
            plugin_key_owner,
        )
        .await
        .expect("put");

    // Seed repo with active row.
    repo.seed(SecretRow {
        id: Uuid::new_v4(),
        tenant_id: TenantId(tenant),
        reference: "mykey".into(),
        sharing: SharingMode::Tenant,
        owner_id: OwnerId(subject),
        status: SecretStatus::Active,
        version: 1,
    });

    let svc = make_service(
        repo,
        Arc::new(FakeDir::single(tenant)),
        mock_enforcer(),
        plugin,
        metrics.clone(),
    );

    let result = svc.get(&ctx, &k).await.expect("get ok");
    assert!(result.is_some());
    let resp = result.unwrap();
    assert!(!resp.is_inherited);
    assert_eq!(resp.owner_tenant_id.0, tenant);
    assert_eq!(
        metrics.last_read_outcome(),
        Some(crate::domain::ports::metrics::ReadOutcome::HitOwn)
    );
}

#[tokio::test]
async fn get_records_pdp_dependency_metric() {
    use crate::domain::ports::metrics::{Dep, DepOp, Outcome};
    let tenant = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let ctx = make_ctx(subject, tenant);
    let k = key("mykey");

    let plugin = FakePlugin::new();
    let repo = Arc::new(FakeSecretRepo::new());
    let metrics = FakeMetrics::new();
    plugin
        .put(&ctx, &TenantId(tenant), &k, SecretValue::from("v1"), None)
        .await
        .expect("put");
    repo.seed(SecretRow {
        id: Uuid::new_v4(),
        tenant_id: TenantId(tenant),
        reference: "mykey".into(),
        sharing: SharingMode::Tenant,
        owner_id: OwnerId(subject),
        status: SecretStatus::Active,
        version: 1,
    });
    let svc = make_service(
        repo,
        Arc::new(FakeDir::single(tenant)),
        mock_enforcer(),
        plugin,
        metrics.clone(),
    );

    svc.get(&ctx, &k).await.expect("get ok");

    assert!(
        metrics
            .deps()
            .contains(&(Dep::Pdp, DepOp::Evaluate, Outcome::Success)),
        "get must record a PDP dependency metric, got {:?}",
        metrics.deps()
    );
}

#[tokio::test]
async fn put_records_pdp_dependency_metric() {
    use crate::domain::ports::metrics::{Dep, DepOp, Outcome};
    let tenant = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let ctx = make_ctx(subject, tenant);
    let k = key("putkey");

    let metrics = FakeMetrics::new();
    let svc = make_service(
        Arc::new(FakeSecretRepo::new()),
        Arc::new(FakeDir::single(tenant)),
        mock_enforcer(),
        FakePlugin::new(),
        metrics.clone(),
    );

    svc.put(
        &ctx,
        &k,
        SecretValue::from("v"),
        SharingMode::Tenant,
        false,
        None,
    )
    .await
    .expect("put ok");

    assert!(
        metrics
            .deps()
            .contains(&(Dep::Pdp, DepOp::Evaluate, Outcome::Success)),
        "put must record a PDP dependency metric, got {:?}",
        metrics.deps()
    );
}

#[tokio::test]
async fn delete_records_pdp_dependency_metric() {
    use crate::domain::ports::metrics::{Dep, DepOp, Outcome};
    let tenant = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let ctx = make_ctx(subject, tenant);
    let k = key("delkey");

    let metrics = FakeMetrics::new();
    let svc = make_service(
        Arc::new(FakeSecretRepo::new()),
        Arc::new(FakeDir::single(tenant)),
        mock_enforcer(),
        FakePlugin::new(),
        metrics.clone(),
    );

    // 404 (no row), but the PDP scope evaluation still runs first.
    _ = svc.delete(&ctx, &k, None).await;

    assert!(
        metrics
            .deps()
            .contains(&(Dep::Pdp, DepOp::Evaluate, Outcome::Success)),
        "delete must record a PDP dependency metric, got {:?}",
        metrics.deps()
    );
}

#[tokio::test]
async fn get_inherited_shared_from_parent_sets_is_inherited() {
    let child = Uuid::new_v4();
    let parent = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let ctx = make_ctx(subject, child);
    let k = key("shared-key");

    let plugin = FakePlugin::new();
    let repo = Arc::new(FakeSecretRepo::new());

    // Seed plugin with value at parent tenant, no owner (shared).
    plugin
        .put(
            &ctx,
            &TenantId(parent),
            &k,
            SecretValue::from("parent-val"),
            None,
        )
        .await
        .expect("put");

    // Seed repo: parent has a shared active row.
    repo.seed(SecretRow {
        id: Uuid::new_v4(),
        tenant_id: TenantId(parent),
        reference: "shared-key".into(),
        sharing: SharingMode::Shared,
        owner_id: OwnerId(Uuid::nil()),
        status: SecretStatus::Active,
        version: 1,
    });

    // chain: [child, parent]
    let dir = Arc::new(FakeDir::new(vec![child, parent]));
    let svc = make_service_noop(repo, dir, mock_enforcer(), plugin);

    let result = svc.get(&ctx, &k).await.expect("get ok");
    assert!(result.is_some());
    let resp = result.unwrap();
    assert!(resp.is_inherited, "must be inherited from parent");
    assert_eq!(resp.owner_tenant_id.0, parent);
    assert_eq!(resp.sharing, SharingMode::Shared);
}

#[tokio::test]
async fn get_tenant_mode_not_inherited_by_child() {
    let child = Uuid::new_v4();
    let parent = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let ctx = make_ctx(subject, child);
    let k = key("tenant-key");

    let plugin = FakePlugin::new();
    let repo = Arc::new(FakeSecretRepo::new());

    // Parent has a tenant-mode row; tenant-mode is NOT inherited by children.
    repo.seed(SecretRow {
        id: Uuid::new_v4(),
        tenant_id: TenantId(parent),
        reference: "tenant-key".into(),
        sharing: SharingMode::Tenant,
        owner_id: OwnerId(Uuid::nil()),
        status: SecretStatus::Active,
        version: 1,
    });

    let dir = Arc::new(FakeDir::new(vec![child, parent]));
    let svc = make_service_noop(repo, dir, mock_enforcer(), plugin);

    let result = svc.get(&ctx, &k).await.expect("get ok");
    // Tenant-mode secret at parent → not visible from child → Miss.
    assert!(result.is_none());
}

#[tokio::test]
async fn get_private_owner_match_only() {
    let tenant = Uuid::new_v4();
    let owner_a = Uuid::new_v4();
    let owner_b = Uuid::new_v4();
    // Subject is B; row belongs to A (private).
    let ctx = make_ctx(owner_b, tenant);
    let k = key("private-key");

    let plugin = FakePlugin::new();
    let repo = Arc::new(FakeSecretRepo::new());

    // A has a private row.
    let row_id = Uuid::new_v4();
    repo.seed(SecretRow {
        id: row_id,
        tenant_id: TenantId(tenant),
        reference: "private-key".into(),
        sharing: SharingMode::Private,
        owner_id: OwnerId(owner_a),
        status: SecretStatus::Active,
        version: 1,
    });
    // Seed plugin for A.
    plugin
        .put(
            &make_ctx(owner_a, tenant),
            &TenantId(tenant),
            &k,
            SecretValue::from("secret-a"),
            Some(&OwnerId(owner_a)),
        )
        .await
        .expect("put");

    let svc = make_service_noop(
        repo,
        Arc::new(FakeDir::single(tenant)),
        mock_enforcer(),
        plugin,
    );

    // B can't see A's private secret.
    let result = svc.get(&ctx, &k).await.expect("get ok");
    assert!(
        result.is_none(),
        "private row for A must not be visible to B"
    );
}

#[tokio::test]
async fn get_shadowing_private_beats_inherited() {
    let child = Uuid::new_v4();
    let parent = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let ctx = make_ctx(subject, child);
    let k = key("shadow-key");

    let plugin = FakePlugin::new();
    let repo = Arc::new(FakeSecretRepo::new());

    // Child has a private row for subject.
    repo.seed(SecretRow {
        id: Uuid::new_v4(),
        tenant_id: TenantId(child),
        reference: "shadow-key".into(),
        sharing: SharingMode::Private,
        owner_id: OwnerId(subject),
        status: SecretStatus::Active,
        version: 1,
    });
    // Parent has a shared row.
    repo.seed(SecretRow {
        id: Uuid::new_v4(),
        tenant_id: TenantId(parent),
        reference: "shadow-key".into(),
        sharing: SharingMode::Shared,
        owner_id: OwnerId(Uuid::nil()),
        status: SecretStatus::Active,
        version: 1,
    });

    // Seed plugin: child private, parent shared.
    plugin
        .put(
            &ctx,
            &TenantId(child),
            &k,
            SecretValue::from("child-private"),
            Some(&OwnerId(subject)),
        )
        .await
        .expect("put child private");
    plugin
        .put(
            &ctx,
            &TenantId(parent),
            &k,
            SecretValue::from("parent-shared"),
            None,
        )
        .await
        .expect("put parent shared");

    let dir = Arc::new(FakeDir::new(vec![child, parent]));
    let svc = make_service_noop(repo, dir, mock_enforcer(), plugin);

    let result = svc.get(&ctx, &k).await.expect("get ok");
    assert!(result.is_some());
    let resp = result.unwrap();
    assert!(
        !resp.is_inherited,
        "child-private must shadow parent-shared"
    );
    assert_eq!(resp.owner_tenant_id.0, child);
    assert_eq!(resp.value.as_bytes(), b"child-private");
}

#[tokio::test]
async fn put_create_conflict_returns_conflict() {
    let tenant = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let ctx = make_ctx(subject, tenant);
    let k = key("existing-key");

    let plugin = FakePlugin::new();
    let repo = Arc::new(FakeSecretRepo::new());

    // Seed existing active row.
    repo.seed(SecretRow {
        id: Uuid::new_v4(),
        tenant_id: TenantId(tenant),
        reference: "existing-key".into(),
        sharing: SharingMode::Tenant,
        owner_id: OwnerId(subject),
        status: SecretStatus::Active,
        version: 1,
    });

    let svc = make_service_noop(
        repo,
        Arc::new(FakeDir::single(tenant)),
        mock_enforcer(),
        plugin,
    );

    let result = svc
        .put(
            &ctx,
            &k,
            SecretValue::from("val"),
            SharingMode::Tenant,
            true,
            None,
        )
        .await;
    assert!(matches!(result, Err(DomainError::Conflict)));
}

#[tokio::test]
async fn put_shared_coexists_with_private() {
    let tenant = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let ctx = make_ctx(subject, tenant);
    let k = key("priv-key");

    let plugin = FakePlugin::new();
    let repo = Arc::new(FakeSecretRepo::new());

    // Existing private row.
    repo.seed(SecretRow {
        id: Uuid::new_v4(),
        tenant_id: TenantId(tenant),
        reference: "priv-key".into(),
        sharing: SharingMode::Private,
        owner_id: OwnerId(subject),
        status: SecretStatus::Active,
        version: 1,
    });

    let svc = make_service_noop(
        repo.clone(),
        Arc::new(FakeDir::single(tenant)),
        mock_enforcer(),
        plugin,
    );

    // PUT Shared for the same ref CREATES a coexisting non-private secret — it
    // does not "transition" the private one. Per design §4.1 a tenant/shared and
    // a private secret coexist under one reference (distinct partial-unique keys).
    let result = svc
        .put(
            &ctx,
            &k,
            SecretValue::from("val"),
            SharingMode::Shared,
            false,
            None,
        )
        .await;
    assert!(result.is_ok(), "expected coexistence Ok, got {result:?}");

    let counts = repo.inventory().await.expect("inventory");
    assert_eq!(counts.private, 1, "private secret must remain");
    assert_eq!(counts.shared, 1, "shared secret created alongside private");
}

#[tokio::test]
async fn delete_only_own_tenant_404_when_inherited_only() {
    let child = Uuid::new_v4();
    let parent = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let ctx = make_ctx(subject, child);
    let k = key("ancestor-key");

    let plugin = FakePlugin::new();
    let repo = Arc::new(FakeSecretRepo::new());

    // Only the parent has the row (shared).
    repo.seed(SecretRow {
        id: Uuid::new_v4(),
        tenant_id: TenantId(parent),
        reference: "ancestor-key".into(),
        sharing: SharingMode::Shared,
        owner_id: OwnerId(Uuid::nil()),
        status: SecretStatus::Active,
        version: 1,
    });

    let svc = make_service_noop(
        repo,
        Arc::new(FakeDir::single(child)), // child-only chain for find_own
        mock_enforcer(),
        plugin,
    );

    let result = svc.delete(&ctx, &k, None).await;
    assert!(
        matches!(result, Err(DomainError::NotFound)),
        "must return NotFound for inherited-only key"
    );
}

#[tokio::test]
async fn read_gate_denied_when_tenant_out_of_scope() {
    let tenant = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let ctx = make_ctx(subject, tenant);
    let k = key("some-key");

    let plugin = FakePlugin::new();
    let repo = Arc::new(FakeSecretRepo::with_scope_allows(false));
    let metrics = FakeMetrics::new();

    let svc = make_service(
        repo,
        Arc::new(FakeDir::single(tenant)),
        mock_enforcer(),
        plugin,
        metrics.clone(),
    );

    let result = svc.get(&ctx, &k).await;
    assert!(
        matches!(result, Err(DomainError::AccessDenied { .. })),
        "expected AccessDenied"
    );
    assert_eq!(
        metrics.cross_tenant_denied_count(),
        1,
        "cross_tenant_denied metric must be recorded"
    );
}

// ── H2: UnsupportedTransition maps to CredStoreError::UnsupportedTransition ──

#[test]
fn unsupported_transition_domain_error_maps_to_sdk_variant() {
    use credstore_sdk::CredStoreError;

    use crate::domain::error::DomainError;

    let domain_err = DomainError::UnsupportedTransition {
        detail: "cannot move between private and tenant/shared".to_owned(),
    };
    let sdk_err = CredStoreError::from(domain_err);
    assert!(
        matches!(sdk_err, CredStoreError::UnsupportedTransition { .. }),
        "expected UnsupportedTransition, got {sdk_err:?}"
    );
    assert!(
        !matches!(sdk_err, CredStoreError::InvalidSecretRef { .. }),
        "must not map to InvalidSecretRef"
    );
}

// ── M3: PDP-denied / eval-failed after Authorizer-port removal ───────────────

#[tokio::test]
async fn get_returns_access_denied_when_pdp_denies() {
    let tenant = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let ctx = make_ctx(subject, tenant);
    let k = key("some-key");

    let svc = make_service_noop(
        Arc::new(FakeSecretRepo::new()),
        Arc::new(FakeDir::single(tenant)),
        deny_enforcer(),
        FakePlugin::new(),
    );

    let result = svc.get(&ctx, &k).await;
    assert!(
        matches!(result, Err(DomainError::AccessDenied { .. })),
        "expected AccessDenied from deny_enforcer, got {result:?}"
    );
}

#[tokio::test]
async fn get_returns_service_unavailable_when_pdp_fails() {
    let tenant = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let ctx = make_ctx(subject, tenant);
    let k = key("some-key");

    let svc = make_service_noop(
        Arc::new(FakeSecretRepo::new()),
        Arc::new(FakeDir::single(tenant)),
        failing_enforcer(),
        FakePlugin::new(),
    );

    let result = svc.get(&ctx, &k).await;
    assert!(
        matches!(result, Err(DomainError::ServiceUnavailable { .. })),
        "expected ServiceUnavailable from failing_enforcer, got {result:?}"
    );
}

#[tokio::test]
async fn put_returns_access_denied_when_pdp_denies() {
    let tenant = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let ctx = make_ctx(subject, tenant);
    let k = key("some-key");

    let svc = make_service_noop(
        Arc::new(FakeSecretRepo::new()),
        Arc::new(FakeDir::single(tenant)),
        deny_enforcer(),
        FakePlugin::new(),
    );

    let result = svc
        .put(
            &ctx,
            &k,
            SecretValue::from("v"),
            SharingMode::Tenant,
            false,
            None,
        )
        .await;
    assert!(
        matches!(result, Err(DomainError::AccessDenied { .. })),
        "expected AccessDenied from deny_enforcer, got {result:?}"
    );
}

#[tokio::test]
async fn delete_returns_access_denied_when_pdp_denies() {
    let tenant = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let ctx = make_ctx(subject, tenant);
    let k = key("some-key");

    let svc = make_service_noop(
        Arc::new(FakeSecretRepo::new()),
        Arc::new(FakeDir::single(tenant)),
        deny_enforcer(),
        FakePlugin::new(),
    );

    let result = svc.delete(&ctx, &k, None).await;
    assert!(
        matches!(result, Err(DomainError::AccessDenied { .. })),
        "expected AccessDenied from deny_enforcer, got {result:?}"
    );
}

// ── map_plugin_err (pure mapping) ─────────────────────────────────────────────

#[test]
fn map_plugin_err_covers_all_variants() {
    use credstore_sdk::CredStoreError;

    use crate::domain::secret::service::map_plugin_err;

    assert!(matches!(
        map_plugin_err(CredStoreError::NotFound),
        DomainError::NotFound
    ));
    assert!(matches!(
        map_plugin_err(CredStoreError::AccessDenied),
        DomainError::AccessDenied { .. }
    ));
    assert!(matches!(
        map_plugin_err(CredStoreError::ServiceUnavailable {
            detail: "x".to_owned(),
            retry_after: Some(std::time::Duration::from_secs(2)),
        }),
        DomainError::ServiceUnavailable { .. }
    ));
    assert!(matches!(
        map_plugin_err(CredStoreError::NoPluginAvailable),
        DomainError::ServiceUnavailable { .. }
    ));
    assert!(matches!(
        map_plugin_err(CredStoreError::Conflict),
        DomainError::Conflict
    ));
    assert!(matches!(
        map_plugin_err(CredStoreError::InvalidSecretRef {
            reason: "r".to_owned()
        }),
        DomainError::Internal { .. }
    ));
    assert!(matches!(
        map_plugin_err(CredStoreError::UnsupportedTransition {
            detail: "d".to_owned()
        }),
        DomainError::Internal { .. }
    ));
    assert!(matches!(
        map_plugin_err(CredStoreError::Internal("boom".to_owned())),
        DomainError::Internal { .. }
    ));
}

#[test]
fn no_plugin_available_maps_to_distinct_non_retryable_unavailable() {
    use credstore_sdk::CredStoreError;

    use crate::domain::secret::service::map_plugin_err;

    let mapped = map_plugin_err(CredStoreError::NoPluginAvailable);
    // Operator misconfiguration: a distinct, stable detail and no `retry_after`,
    // distinguishable from a transient backend outage.
    assert!(
        matches!(
            &mapped,
            DomainError::ServiceUnavailable { detail, retry_after, .. }
                if detail.contains("no storage plugin registered") && retry_after.is_none()
        ),
        "NoPluginAvailable must map to a distinct non-retryable ServiceUnavailable, got {mapped:?}"
    );
}

// ── getter + reaper loop ──────────────────────────────────────────────────────

#[test]
fn reaper_tick_secs_reflects_config() {
    let svc = make_service_noop(
        Arc::new(FakeSecretRepo::new()),
        Arc::new(FakeDir::single(Uuid::new_v4())),
        mock_enforcer(),
        FakePlugin::new(),
    );
    assert_eq!(svc.reaper_tick_secs(), 60);
}

#[tokio::test]
async fn reap_and_refresh_reaps_provisioning_and_refreshes_inventory() {
    let tenant = Uuid::new_v4();
    let owner = Uuid::new_v4();
    let repo = Arc::new(FakeSecretRepo::new());
    repo.seed(SecretRow {
        id: Uuid::new_v4(),
        tenant_id: TenantId(tenant),
        reference: "prov".to_owned(),
        sharing: SharingMode::Tenant,
        owner_id: OwnerId(owner),
        status: SecretStatus::Provisioning,
        version: 1,
    });
    repo.seed(SecretRow {
        id: Uuid::new_v4(),
        tenant_id: TenantId(tenant),
        reference: "act".to_owned(),
        sharing: SharingMode::Shared,
        owner_id: OwnerId(owner),
        status: SecretStatus::Active,
        version: 1,
    });
    let svc = make_service(
        repo.clone(),
        Arc::new(FakeDir::single(tenant)),
        mock_enforcer(),
        FakePlugin::new(),
        FakeMetrics::new(),
    );

    svc.reap_and_refresh().await;
    assert_eq!(
        repo.inventory().await.expect("inventory").provisioning,
        0,
        "the stuck provisioning row was reaped"
    );
}

// ── private-mode + backend-miss branches ──────────────────────────────────────

#[tokio::test]
async fn put_create_then_update_private_secret() {
    let tenant = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let ctx = make_ctx(subject, tenant);
    let k = key("priv-key");
    let svc = make_service(
        Arc::new(FakeSecretRepo::new()),
        Arc::new(FakeDir::single(tenant)),
        mock_enforcer(),
        FakePlugin::new(),
        FakeMetrics::new(),
    );

    // Create saga with Private sharing (plugin_owner = Some).
    svc.put(
        &ctx,
        &k,
        SecretValue::new(b"s".to_vec()),
        SharingMode::Private,
        false,
        None,
    )
    .await
    .expect("create private");
    // Update path on the existing private row (still private).
    svc.put(
        &ctx,
        &k,
        SecretValue::new(b"s2".to_vec()),
        SharingMode::Private,
        false,
        None,
    )
    .await
    .expect("update private");

    let got = svc.get(&ctx, &k).await.expect("get").expect("some");
    assert_eq!(got.sharing, SharingMode::Private);
}

#[tokio::test]
async fn get_resolved_row_without_backend_value_is_not_found() {
    let tenant = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let ctx = make_ctx(subject, tenant);
    let k = key("ghost");
    let repo = Arc::new(FakeSecretRepo::new());
    // Active row exists, but the plugin never received a value for it.
    repo.seed(SecretRow {
        id: Uuid::new_v4(),
        tenant_id: TenantId(tenant),
        reference: "ghost".to_owned(),
        sharing: SharingMode::Tenant,
        owner_id: OwnerId(subject),
        status: SecretStatus::Active,
        version: 1,
    });
    let svc = make_service(
        repo,
        Arc::new(FakeDir::single(tenant)),
        mock_enforcer(),
        FakePlugin::new(),
        FakeMetrics::new(),
    );

    let err = svc
        .get(&ctx, &k)
        .await
        .expect_err("row resolves but backend has no value");
    assert!(matches!(err, DomainError::NotFound));
}

#[tokio::test]
async fn delete_private_secret_removes_row() {
    let tenant = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let ctx = make_ctx(subject, tenant);
    let k = key("del-priv");
    let svc = make_service(
        Arc::new(FakeSecretRepo::new()),
        Arc::new(FakeDir::single(tenant)),
        mock_enforcer(),
        FakePlugin::new(),
        FakeMetrics::new(),
    );

    svc.put(
        &ctx,
        &k,
        SecretValue::new(b"x".to_vec()),
        SharingMode::Private,
        false,
        None,
    )
    .await
    .expect("create");
    svc.delete(&ctx, &k, None).await.expect("delete private");
    assert!(svc.get(&ctx, &k).await.expect("get").is_none());
}

// ── versioning (option a) ─────────────────────────────────────────────────────

#[tokio::test]
async fn create_starts_at_version_one_then_overwrite_bumps() {
    let tenant = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let ctx = make_ctx(subject, tenant);
    let k = key("ver");
    let repo = Arc::new(FakeSecretRepo::new());
    let svc = make_service(
        repo.clone(),
        Arc::new(FakeDir::single(tenant)),
        mock_enforcer(),
        FakePlugin::new(),
        FakeMetrics::new(),
    );

    svc.put(
        &ctx,
        &k,
        SecretValue::from("v1"),
        SharingMode::Tenant,
        false,
        None,
    )
    .await
    .expect("create");
    let row = repo
        .find_for_write(
            &AccessScope::for_tenant(tenant),
            TenantId(tenant),
            OwnerId(subject),
            &k,
            SharingMode::Tenant,
        )
        .await
        .expect("find")
        .expect("row");
    assert_eq!(row.version, 1, "create seeds version 1");

    svc.put(
        &ctx,
        &k,
        SecretValue::from("v2"),
        SharingMode::Tenant,
        false,
        None,
    )
    .await
    .expect("overwrite");
    let row = repo
        .find_for_write(
            &AccessScope::for_tenant(tenant),
            TenantId(tenant),
            OwnerId(subject),
            &k,
            SharingMode::Tenant,
        )
        .await
        .expect("find")
        .expect("row");
    assert_eq!(row.version, 2, "overwrite bumps to 2");
}

#[tokio::test]
async fn put_create_race_resolves_to_update() {
    let tenant = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let ctx = make_ctx(subject, tenant);
    let k = key("race");

    // The winner's row exists but is still Provisioning (invisible to
    // find_for_write). The fake promotes it to Active when our insert conflicts,
    // simulating the winner finishing its saga — so the bounded retry resolves
    // to the update path deterministically.
    let repo = Arc::new(FakeSecretRepo::with_promote_on_conflict(true));
    repo.seed(SecretRow {
        id: Uuid::new_v4(),
        tenant_id: TenantId(tenant),
        reference: "race".into(),
        sharing: SharingMode::Tenant,
        owner_id: OwnerId(subject),
        status: SecretStatus::Provisioning,
        version: 1,
    });
    let svc = make_service_noop(
        repo.clone(),
        Arc::new(FakeDir::single(tenant)),
        mock_enforcer(),
        FakePlugin::new(),
    );

    // PUT (create_only = false) loses the create race, then resolves to update.
    svc.put(
        &ctx,
        &k,
        SecretValue::from("v"),
        SharingMode::Tenant,
        false,
        None,
    )
    .await
    .expect("race resolves to update");
    let row = repo
        .find_for_write(
            &AccessScope::for_tenant(tenant),
            TenantId(tenant),
            OwnerId(subject),
            &k,
            SharingMode::Tenant,
        )
        .await
        .expect("find")
        .expect("row");
    assert_eq!(row.version, 2, "the loser touched the winner's row: 1 -> 2");
}

#[tokio::test]
async fn put_create_race_exhausted_returns_conflict() {
    let tenant = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let ctx = make_ctx(subject, tenant);
    let k = key("stuck");

    // Winner stuck mid-saga: a Provisioning row that never promotes. PUT's
    // bounded retry never sees it Active -> retry-safe 409.
    let repo = Arc::new(FakeSecretRepo::new());
    repo.seed(SecretRow {
        id: Uuid::new_v4(),
        tenant_id: TenantId(tenant),
        reference: "stuck".into(),
        sharing: SharingMode::Tenant,
        owner_id: OwnerId(subject),
        status: SecretStatus::Provisioning,
        version: 1,
    });
    let svc = make_service_noop(
        repo,
        Arc::new(FakeDir::single(tenant)),
        mock_enforcer(),
        FakePlugin::new(),
    );

    let result = svc
        .put(
            &ctx,
            &k,
            SecretValue::from("v"),
            SharingMode::Tenant,
            false,
            None,
        )
        .await;
    assert!(
        matches!(result, Err(DomainError::Conflict)),
        "exhausted retry -> 409, got {result:?}"
    );
}

#[tokio::test]
async fn failed_create_does_not_wedge_the_reference() {
    let tenant = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let ctx = make_ctx(subject, tenant);
    let k = key("wedge");

    // Backend hiccup: the first plugin.put (mid create-saga) fails, then recovers.
    let plugin = FakePlugin::with_put_failures(1);
    let repo = Arc::new(FakeSecretRepo::new());
    let svc = make_service(
        repo.clone(),
        Arc::new(FakeDir::single(tenant)),
        mock_enforcer(),
        plugin,
        FakeMetrics::new(),
    );

    // Create: the provisioning row is inserted, then the backend put fails.
    let first = svc
        .put(
            &ctx,
            &k,
            SecretValue::from("v"),
            SharingMode::Tenant,
            true,
            None,
        )
        .await;
    assert!(
        first.is_err(),
        "backend put failure must surface, got {first:?}"
    );

    // The failed create must compensate its provisioning row rather than leave
    // it to wedge the reference until the reaper runs.
    assert_eq!(
        repo.inventory().await.expect("inventory").provisioning,
        0,
        "failed create must roll back the provisioning row"
    );

    // A retry now succeeds instead of hitting a misleading 409.
    svc.put(
        &ctx,
        &k,
        SecretValue::from("v"),
        SharingMode::Tenant,
        true,
        None,
    )
    .await
    .expect("retry after a failed create must succeed, not 409");
}

#[tokio::test]
async fn put_denied_when_tenant_out_of_scope_inserts_no_row() {
    let tenant = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let ctx = make_ctx(subject, tenant);
    let k = key("oos");

    // PDP allow-with-constraints scope that excludes the caller's own tenant.
    let repo = Arc::new(FakeSecretRepo::with_scope_allows(false));
    let metrics = FakeMetrics::new();
    let svc = make_service(
        repo.clone(),
        Arc::new(FakeDir::single(tenant)),
        mock_enforcer(),
        FakePlugin::new(),
        metrics.clone(),
    );

    let result = svc
        .put(
            &ctx,
            &k,
            SecretValue::from("v"),
            SharingMode::Tenant,
            false,
            None,
        )
        .await;

    assert!(
        matches!(result, Err(DomainError::AccessDenied { .. })),
        "out-of-scope write must be denied, got {result:?}"
    );
    // The scope_unchecked insert must never run: no orphan provisioning row.
    assert_eq!(
        repo.inventory().await.expect("inventory").provisioning,
        0,
        "denied write must not insert an orphan provisioning row"
    );
    assert_eq!(
        metrics.cross_tenant_denied_count(),
        1,
        "cross_tenant_denied metric must be recorded on the write path"
    );
}

#[tokio::test]
async fn failed_create_records_rollback_success_metric() {
    use crate::domain::ports::metrics::Outcome;
    let tenant = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let ctx = make_ctx(subject, tenant);
    let k = key("rollback-ok");

    let plugin = FakePlugin::with_put_failures(1);
    let metrics = FakeMetrics::new();
    let svc = make_service(
        Arc::new(FakeSecretRepo::new()),
        Arc::new(FakeDir::single(tenant)),
        mock_enforcer(),
        plugin,
        metrics.clone(),
    );

    _ = svc
        .put(
            &ctx,
            &k,
            SecretValue::from("v"),
            SharingMode::Tenant,
            true,
            None,
        )
        .await;

    assert_eq!(
        metrics.provisioning_rollbacks(),
        vec![Outcome::Success],
        "a compensated create must record one successful rollback"
    );
}

#[tokio::test]
async fn failed_rollback_records_error_metric() {
    use crate::domain::ports::metrics::Outcome;
    let tenant = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let ctx = make_ctx(subject, tenant);
    let k = key("rollback-fail");

    // Backend put fails, then the rollback delete also fails — the reference
    // stays wedged, the case worth alerting on.
    let plugin = FakePlugin::with_put_failures(1);
    let metrics = FakeMetrics::new();
    let svc = make_service(
        Arc::new(FakeSecretRepo::with_delete_failure()),
        Arc::new(FakeDir::single(tenant)),
        mock_enforcer(),
        plugin,
        metrics.clone(),
    );

    _ = svc
        .put(
            &ctx,
            &k,
            SecretValue::from("v"),
            SharingMode::Tenant,
            true,
            None,
        )
        .await;

    assert_eq!(
        metrics.provisioning_rollbacks(),
        vec![Outcome::Error],
        "a failed rollback must record an Error-outcome metric"
    );
}

#[tokio::test]
async fn post_create_only_race_returns_conflict_immediately() {
    let tenant = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let ctx = make_ctx(subject, tenant);
    let k = key("conly");

    // A Provisioning winner row; create_only POST must 409 without retrying.
    // (No promote_on_conflict needed: create_only returns before the retry loop.)
    let repo = Arc::new(FakeSecretRepo::new());
    repo.seed(SecretRow {
        id: Uuid::new_v4(),
        tenant_id: TenantId(tenant),
        reference: "conly".into(),
        sharing: SharingMode::Tenant,
        owner_id: OwnerId(subject),
        status: SecretStatus::Provisioning,
        version: 1,
    });
    let svc = make_service_noop(
        repo,
        Arc::new(FakeDir::single(tenant)),
        mock_enforcer(),
        FakePlugin::new(),
    );

    let result = svc
        .put(
            &ctx,
            &k,
            SecretValue::from("v"),
            SharingMode::Tenant,
            true,
            None,
        )
        .await;
    assert!(
        matches!(result, Err(DomainError::Conflict)),
        "create-only race -> 409, got {result:?}"
    );
}

// ── optimistic concurrency (If-Match precondition) ───────────────────────────

#[tokio::test]
async fn put_if_match_matching_version_overwrites_and_bumps() {
    use crate::domain::secret::model::WritePrecondition;
    let tenant = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let ctx = make_ctx(subject, tenant);
    let k = key("oc");
    let repo = Arc::new(FakeSecretRepo::new());
    let svc = make_service(
        repo.clone(),
        Arc::new(FakeDir::single(tenant)),
        mock_enforcer(),
        FakePlugin::new(),
        FakeMetrics::new(),
    );

    // Create at version 1, then overwrite with the matching If-Match.
    svc.put(
        &ctx,
        &k,
        SecretValue::from("v1"),
        SharingMode::Tenant,
        false,
        None,
    )
    .await
    .expect("create");
    svc.put(
        &ctx,
        &k,
        SecretValue::from("v2"),
        SharingMode::Tenant,
        false,
        Some(WritePrecondition::Version(1)),
    )
    .await
    .expect("matching If-Match must succeed");

    let got = svc.get(&ctx, &k).await.expect("get").expect("some");
    assert_eq!(got.version, 2, "matching If-Match bumps the version");
    assert_eq!(got.value.as_bytes(), b"v2");
}

#[tokio::test]
async fn put_if_match_stale_version_conflicts_without_writing() {
    use crate::domain::secret::model::WritePrecondition;
    let tenant = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let ctx = make_ctx(subject, tenant);
    let k = key("oc-stale");
    let svc = make_service(
        Arc::new(FakeSecretRepo::new()),
        Arc::new(FakeDir::single(tenant)),
        mock_enforcer(),
        FakePlugin::new(),
        FakeMetrics::new(),
    );

    svc.put(
        &ctx,
        &k,
        SecretValue::from("v1"),
        SharingMode::Tenant,
        false,
        None,
    )
    .await
    .expect("create");

    let result = svc
        .put(
            &ctx,
            &k,
            SecretValue::from("v2"),
            SharingMode::Tenant,
            false,
            Some(WritePrecondition::Version(99)),
        )
        .await;
    assert!(
        matches!(result, Err(DomainError::VersionConflict)),
        "stale If-Match must conflict, got {result:?}"
    );

    // The stale write must NOT have touched the backend value or the version.
    let got = svc.get(&ctx, &k).await.expect("get").expect("some");
    assert_eq!(got.version, 1, "stale write must not bump the version");
    assert_eq!(
        got.value.as_bytes(),
        b"v1",
        "stale write must not overwrite"
    );
}

#[tokio::test]
async fn put_if_match_on_missing_secret_conflicts() {
    use crate::domain::secret::model::WritePrecondition;
    let tenant = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let ctx = make_ctx(subject, tenant);
    let k = key("oc-missing");
    let repo = Arc::new(FakeSecretRepo::new());
    let svc = make_service(
        repo.clone(),
        Arc::new(FakeDir::single(tenant)),
        mock_enforcer(),
        FakePlugin::new(),
        FakeMetrics::new(),
    );

    let result = svc
        .put(
            &ctx,
            &k,
            SecretValue::from("v"),
            SharingMode::Tenant,
            false,
            Some(WritePrecondition::Version(1)),
        )
        .await;
    assert!(
        matches!(result, Err(DomainError::VersionConflict)),
        "If-Match on a non-existent secret must conflict, got {result:?}"
    );
    assert_eq!(
        repo.inventory().await.expect("inventory").provisioning,
        0,
        "conflicting create must not insert a row"
    );
}

#[tokio::test]
async fn delete_if_match_stale_version_conflicts() {
    use crate::domain::secret::model::WritePrecondition;
    let tenant = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let ctx = make_ctx(subject, tenant);
    let k = key("oc-del");
    let svc = make_service(
        Arc::new(FakeSecretRepo::new()),
        Arc::new(FakeDir::single(tenant)),
        mock_enforcer(),
        FakePlugin::new(),
        FakeMetrics::new(),
    );

    svc.put(
        &ctx,
        &k,
        SecretValue::from("v1"),
        SharingMode::Tenant,
        false,
        None,
    )
    .await
    .expect("create");

    let result = svc
        .delete(&ctx, &k, Some(WritePrecondition::Version(99)))
        .await;
    assert!(
        matches!(result, Err(DomainError::VersionConflict)),
        "stale If-Match delete must conflict, got {result:?}"
    );
    // The secret must still be there.
    assert!(
        svc.get(&ctx, &k).await.expect("get").is_some(),
        "stale delete must not remove the secret"
    );

    // Matching version deletes.
    svc.delete(&ctx, &k, Some(WritePrecondition::Version(1)))
        .await
        .expect("matching If-Match delete must succeed");
    assert!(svc.get(&ctx, &k).await.expect("get").is_none());
}

#[tokio::test]
async fn delete_if_match_race_maps_zero_rows_to_version_conflict() {
    use crate::domain::secret::model::WritePrecondition;
    let tenant = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let ctx = make_ctx(subject, tenant);
    let k = key("oc-del-race");

    // Row exists at v1 (find_own + pre-check pass), but the gated delete matches
    // 0 rows — the row moved/vanished between the pre-check and the commit.
    let repo = Arc::new(FakeSecretRepo::with_delete_not_found());
    repo.seed(SecretRow {
        id: Uuid::new_v4(),
        tenant_id: TenantId(tenant),
        reference: "oc-del-race".into(),
        sharing: SharingMode::Tenant,
        owner_id: OwnerId(subject),
        status: SecretStatus::Active,
        version: 1,
    });
    let svc = make_service_noop(
        repo,
        Arc::new(FakeDir::single(tenant)),
        mock_enforcer(),
        FakePlugin::new(),
    );

    // Under a version precondition, the 0-row delete is an optimistic-lock
    // conflict, not a misleading 404.
    let conflict = svc
        .delete(&ctx, &k, Some(WritePrecondition::Version(1)))
        .await;
    assert!(
        matches!(conflict, Err(DomainError::VersionConflict)),
        "0-row delete under precondition -> VersionConflict, got {conflict:?}"
    );

    // Without a precondition, the same 0-row delete stays a NotFound.
    let plain = svc.delete(&ctx, &k, None).await;
    assert!(
        matches!(plain, Err(DomainError::NotFound)),
        "0-row delete without precondition -> NotFound, got {plain:?}"
    );
}
