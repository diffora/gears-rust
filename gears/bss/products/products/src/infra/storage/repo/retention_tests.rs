//! `10-retention-erasure`'s store tests, against the executed `SQLite` mirror.
//!
//! # The payload cases seed the column directly, and that is the whole point
//!
//! Nothing in the crate writes `identity_payload`: `resolve_actor_ref`'s mint
//! writes `Set(None)` and no other statement touches it. So an erasure test
//! that minted a row the ordinary way and then asserted the payload was gone
//! would pass on an implementation that destroys nothing — the column was
//! already `NULL` before the act. Every case below that claims the
//! destruction happened seeds a payload first, through the same
//! `update_many().secure()` chain the repository itself uses.
//!
//! # Only the `SQLite` mirror is executed
//!
//! As in `repo_tests`, the suite runs in-memory. What that leaves resting on
//! `migrations_tests`' clause-for-clause reading is the Postgres half of
//! `chk_products_identity_ref_tombstone` and of the partial unique index —
//! both asserted there by reading the statements, and both exercised here
//! through their `SQLite` twins.
#![allow(clippy::expect_used)]

use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use sea_orm_migration::MigratorTrait;
use toolkit_db::secure::{AccessScope, DBRunner, SecureEntityExt, SecureUpdateExt};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

use super::{
    audit_refs_of_actors, identity_entries_of_principal, tombstone_principal,
    write_audited_read_audit, write_evidential_act_audit,
};
use crate::infra::storage::entity::{audit_log, identity_ref};
use crate::infra::storage::migrations::Migrator;
use crate::infra::storage::repo::{AuditCommon, resolve_actor_ref};
use crate::test_support::at;

const TENANT: Uuid = Uuid::from_u128(0x7e_11);
const OTHER_TENANT: Uuid = Uuid::from_u128(0x7e_22);
const ERASER: Uuid = Uuid::from_u128(0xe4_01);
const AUDIT: Uuid = Uuid::from_u128(0xa0_10);
const ALICE: &str = "principal:alice";

/// A pinned in-memory `SQLite` pool, one connection only -- `repo_tests`'
/// harness and for its reason: a multi-connection memory pool hands each
/// checkout its own empty database.
async fn harness() -> DBProvider<DbError> {
    let opts = ConnectOpts {
        max_conns: Some(1),
        min_conns: Some(1),
        ..Default::default()
    };
    let db = connect_db("sqlite::memory:", opts)
        .await
        .expect("connect in-memory sqlite");
    toolkit_db::migration_runner::run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("run migrator");
    DBProvider::<DbError>::new(db)
}

/// Put an identity payload on a live row.
///
/// No production writer does this, which is exactly why the tests need it:
/// see this module's own doc. Uses the repository's own secure chain, never a
/// raw connection -- `DBRunner` does not implement `ConnectionTrait`, so
/// there is no other route to the row.
async fn seed_payload(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    payload: &str,
) {
    identity_ref::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            identity_ref::Column::IdentityPayload,
            Expr::value(Some(payload.to_owned())),
        )
        .filter(
            Condition::all()
                .add(identity_ref::Column::TenantId.eq(tenant_id))
                .add(identity_ref::Column::ActorRef.eq(actor_ref)),
        )
        .exec(runner)
        .await
        .expect("seed an identity payload");
}

async fn row_of(
    runner: &impl DBRunner,
    scope: &AccessScope,
    actor_ref: Uuid,
) -> identity_ref::Model {
    identity_ref::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(identity_ref::Column::TenantId.eq(TENANT))
                .add(identity_ref::Column::ActorRef.eq(actor_ref)),
        )
        .one(runner)
        .await
        .expect("read the map row")
        .expect("the row exists")
}

/// **The erasure destroys the payload, stamps the tombstone, and leaves the
/// pseudonym standing** -- all three, because any two of them can pass on a
/// build that got the third wrong.
///
/// The payload is seeded first. Without that this case is vacuous: the column
/// is `NULL` on every row the crate can mint, so "the payload is gone" would
/// be true before the act ran.
#[tokio::test]
async fn an_erasure_destroys_a_seeded_payload_and_stamps_the_tombstone() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    let actor_ref = resolve_actor_ref(&conn, &scope, TENANT, ALICE, at(9))
        .await
        .expect("mint a ref");
    seed_payload(&conn, &scope, TENANT, actor_ref, "alice@example.test").await;
    assert!(
        row_of(&conn, &scope, actor_ref)
            .await
            .identity_payload
            .is_some(),
        "the probe is armed: the payload is present before the erasure"
    );

    let erased = tombstone_principal(&conn, &scope, TENANT, ALICE, at(12))
        .await
        .expect("erase");

    assert_eq!(erased, Some(actor_ref), "the act names the ref it retired");
    let row = row_of(&conn, &scope, actor_ref).await;
    assert_eq!(row.identity_payload, None, "the identity is destroyed");
    assert_eq!(row.tombstoned_at, Some(at(12)), "the tombstone is stamped");
    assert_eq!(
        row.principal_ref, ALICE,
        "the pseudonym stands, which is what lets a repeat DSAR resolve"
    );
}

/// **An unknown principal answers `None` and mints nothing.**
///
/// The mint half is the case that matters: `resolve_actor_ref` would have
/// created a live row for this principal, and a door built on it would report
/// a successful erasure of a principal it had just invented.
#[tokio::test]
async fn an_unknown_principal_answers_none_and_mints_nothing() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    let erased = tombstone_principal(&conn, &scope, TENANT, "principal:nobody", at(12))
        .await
        .expect("the query runs");

    assert_eq!(erased, None, "no live ref, no erasure");
    let rows = identity_entries_of_principal(&conn, &scope, TENANT, "principal:nobody")
        .await
        .expect("read back");
    assert!(rows.is_empty(), "and no row was minted: {rows:?}");
}

/// **A second erasure of the same principal answers `None`.**
///
/// `tombstoned_at` is *"set once, by erasure, and never cleared"*, so the
/// second call must not restamp it -- the assertion is on the instant, not
/// only on the answer.
#[tokio::test]
async fn a_second_erasure_answers_none_and_does_not_restamp() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    let actor_ref = resolve_actor_ref(&conn, &scope, TENANT, ALICE, at(9))
        .await
        .expect("mint a ref");
    tombstone_principal(&conn, &scope, TENANT, ALICE, at(12))
        .await
        .expect("erase");

    let again = tombstone_principal(&conn, &scope, TENANT, ALICE, at(15))
        .await
        .expect("the query runs");

    assert_eq!(again, None, "the principal has no live ref to erase");
    assert_eq!(
        row_of(&conn, &scope, actor_ref).await.tombstoned_at,
        Some(at(12)),
        "and the first erasure's instant is untouched"
    );
}

/// **The export read returns the tombstoned entry; the shipped resolve
/// cannot.**
///
/// This is the whole reason `dod-identity-map` obliges a second read. The
/// contrast is asserted rather than described: after the erasure the export
/// sees one entry, and a fresh resolve mints a **different** ref, after which
/// the export sees two -- the tombstoned one and the live one.
#[tokio::test]
async fn the_export_read_sees_the_tombstone_and_a_later_mint_is_a_second_entry() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    let first = resolve_actor_ref(&conn, &scope, TENANT, ALICE, at(9))
        .await
        .expect("mint a ref");
    tombstone_principal(&conn, &scope, TENANT, ALICE, at(12))
        .await
        .expect("erase");

    let after_erasure = identity_entries_of_principal(&conn, &scope, TENANT, ALICE)
        .await
        .expect("export read");
    assert_eq!(after_erasure.len(), 1, "{after_erasure:?}");
    assert_eq!(after_erasure[0].actor_ref, first);
    assert_eq!(after_erasure[0].tombstoned_at, Some(at(12)));

    let second = resolve_actor_ref(&conn, &scope, TENANT, ALICE, at(15))
        .await
        .expect("a principal acting after its erasure mints a fresh ref");
    assert_ne!(second, first, "the tombstoned ref is retired permanently");

    let both = identity_entries_of_principal(&conn, &scope, TENANT, ALICE)
        .await
        .expect("export read");
    assert_eq!(both.len(), 2, "{both:?}");
    assert_eq!(
        both.iter().map(|e| e.actor_ref).collect::<Vec<_>>(),
        vec![first, second],
        "ordered by first_seen_at, so a DSAR response is reproducible"
    );
    assert_eq!(both[1].tombstoned_at, None, "the fresh ref is live");
}

/// **The export read is tenant-scoped**, on a principal string that exists in
/// both tenants -- the case a scope bug passes when the two tenants hold
/// different principals.
#[tokio::test]
async fn the_export_read_does_not_cross_tenants() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let other = AccessScope::for_tenant(OTHER_TENANT);

    resolve_actor_ref(&conn, &scope, TENANT, ALICE, at(9))
        .await
        .expect("mint here");
    resolve_actor_ref(&conn, &other, OTHER_TENANT, ALICE, at(9))
        .await
        .expect("mint there");

    let here = identity_entries_of_principal(&conn, &scope, TENANT, ALICE)
        .await
        .expect("export read");

    assert_eq!(here.len(), 1, "one tenant's entry only: {here:?}");
}

/// **An erasure in one tenant does not reach the same principal in another**
/// -- P-D-50's per-tenant reach, asserted rather than assumed.
#[tokio::test]
async fn an_erasure_stops_at_the_tenant_boundary() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let other = AccessScope::for_tenant(OTHER_TENANT);

    resolve_actor_ref(&conn, &scope, TENANT, ALICE, at(9))
        .await
        .expect("mint here");
    let there = resolve_actor_ref(&conn, &other, OTHER_TENANT, ALICE, at(9))
        .await
        .expect("mint there");

    tombstone_principal(&conn, &scope, TENANT, ALICE, at(12))
        .await
        .expect("erase here");

    let survivor = identity_ref::Entity::find()
        .secure()
        .scope_with(&other)
        .filter(
            Condition::all()
                .add(identity_ref::Column::TenantId.eq(OTHER_TENANT))
                .add(identity_ref::Column::ActorRef.eq(there)),
        )
        .one(&conn)
        .await
        .expect("read the other tenant's row")
        .expect("it exists");

    assert_eq!(
        survivor.tombstoned_at, None,
        "a DSAR needs one request per tenant (P-D-50)"
    );
}

/// **The evidential row carries the eraser's own ref, the reason, and the
/// retired ref as its subject** -- and carries neither an error code nor a
/// session id, which is what distinguishes the class from a refusal and from
/// an elevated read.
#[tokio::test]
async fn the_evidential_row_carries_the_erasers_ref_and_the_reason() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    let subject = Uuid::from_u128(0x5b_01);
    write_evidential_act_audit(
        &conn,
        &scope,
        AuditCommon {
            audit_id: AUDIT,
            tenant_id: TENANT,
            actor_ref: ERASER,
            action: "erasure.execute".to_owned(),
            subject_kind: "identity_ref".to_owned(),
            reason: Some("dsar-2026-114".to_owned()),
            correlation_id: None,
            written_at: at(12),
        },
        subject,
    )
    .await
    .expect("write the evidential row");

    let row = audit_log::Entity::find()
        .secure()
        .scope_with(&scope)
        .filter(Condition::all().add(audit_log::Column::AuditId.eq(AUDIT)))
        .one(&conn)
        .await
        .expect("read it back")
        .expect("it exists");

    assert_eq!(row.actor_ref, ERASER, "the eraser's own pseudonymous ref");
    assert_eq!(row.subject_id, Some(subject), "the ref it retired");
    assert_eq!(row.reason.as_deref(), Some("dsar-2026-114"));
    assert_eq!(row.error_code, None, "not a refusal");
    assert_eq!(row.session_id, None, "not an elevated read");
}

/// **The compliance export's row names the principal and carries no session
/// id.**
///
/// The class it lands in was widened from *"reads under elevation"* to the
/// reason that justified it, and this door runs under no elevation at all --
/// its grant is `compliance × export`, its own pair. A session id here would
/// be an invented value, and so would a `subject_id`: the request names a
/// principal string, and an export is answerable for a principal that has
/// never held a ref.
#[tokio::test]
async fn the_audited_read_row_carries_no_session_id() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    write_audited_read_audit(
        &conn,
        &scope,
        AuditCommon {
            audit_id: AUDIT,
            tenant_id: TENANT,
            actor_ref: ERASER,
            action: "compliance.export".to_owned(),
            subject_kind: "identity_ref".to_owned(),
            reason: None,
            correlation_id: None,
            written_at: at(12),
        },
        ALICE.to_owned(),
    )
    .await
    .expect("write the audited-read row");

    let row = audit_log::Entity::find()
        .secure()
        .scope_with(&scope)
        .filter(Condition::all().add(audit_log::Column::AuditId.eq(AUDIT)))
        .one(&conn)
        .await
        .expect("read it back")
        .expect("it exists");

    assert_eq!(row.session_id, None, "not an elevated read");
    assert_eq!(
        row.subject_id, None,
        "an export's subject is a principal string, not a minted id"
    );
    assert_eq!(
        row.attempted_key.as_deref(),
        Some(ALICE),
        "and the principal the caller named is what the row carries"
    );
}

/// **An empty ref list answers empty**, and a populated one answers only the
/// named refs' rows -- the positive control, without which the empty case
/// passes on a function that always answers empty.
#[tokio::test]
async fn audit_refs_are_scoped_to_the_named_actors() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    let mine = Uuid::from_u128(0xac_11);
    let theirs = Uuid::from_u128(0xac_22);
    for (audit_id, actor_ref) in [(Uuid::from_u128(0x1), mine), (Uuid::from_u128(0x2), theirs)] {
        write_evidential_act_audit(
            &conn,
            &scope,
            AuditCommon {
                audit_id,
                tenant_id: TENANT,
                actor_ref,
                action: "erasure.execute".to_owned(),
                subject_kind: "identity_ref".to_owned(),
                reason: None,
                correlation_id: None,
                written_at: at(12),
            },
            actor_ref,
        )
        .await
        .expect("write a row");
    }

    assert!(
        audit_refs_of_actors(&conn, &scope, TENANT, &[])
            .await
            .expect("empty input")
            .is_empty(),
        "no refs, no references -- never the whole tenant's"
    );
    assert_eq!(
        audit_refs_of_actors(&conn, &scope, TENANT, &[mine])
            .await
            .expect("one ref"),
        vec![Uuid::from_u128(0x1)],
        "and the positive control: the named actor's row is found"
    );
}

/// **A ref is found in `subject_id` as well as in `actor_ref`.**
///
/// The erasure's own evidential row names the eraser as the actor and the
/// **retired** ref as the subject, so an erased principal never appears in the
/// actor column of the row recording its own erasure. An export matching only
/// that column answers a DSAR filed after an erasure without the erasure in
/// it. The negative control is in the same case: a third ref appearing in
/// neither column is not returned.
#[tokio::test]
async fn a_ref_in_the_subject_column_is_found_too() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    let erased = Uuid::from_u128(0xac_33);
    let stranger = Uuid::from_u128(0xac_44);
    write_evidential_act_audit(
        &conn,
        &scope,
        AuditCommon {
            audit_id: AUDIT,
            tenant_id: TENANT,
            actor_ref: ERASER,
            action: "erasure.execute".to_owned(),
            subject_kind: "identity_ref".to_owned(),
            reason: Some("dsar-2026-114".to_owned()),
            correlation_id: None,
            written_at: at(12),
        },
        erased,
    )
    .await
    .expect("write the erasure's own row");

    assert_eq!(
        audit_refs_of_actors(&conn, &scope, TENANT, &[erased])
            .await
            .expect("read"),
        vec![AUDIT],
        "the row recording the erasure is part of the erased principal's export"
    );
    assert!(
        audit_refs_of_actors(&conn, &scope, TENANT, &[stranger])
            .await
            .expect("read")
            .is_empty(),
        "and a ref in neither column is not"
    );
}
