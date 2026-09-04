//! What more than one test module in this crate needs, written once.
//!
//! # Why this module exists
//!
//! Three suites had built their own copy of the same flat-`In` PDP fake, and
//! the two door suites had built their own copy of ten database-introspection
//! helpers on top of that — **twelve** byte-identical functions across
//! `api::rest::products_tests` and `api::rest::skus_tests`, plus a third
//! `FlatInResolver` in `authz_tests`. `FlatInResolver`'s own doc named the
//! reason: *"`authz_tests` is a private `#[cfg(test)]` sibling module, not a
//! reusable test-support crate."* That was true, and this module is the thing
//! whose absence it recorded.
//!
//! It matters more here than duplication usually does. This gear's Product and
//! SKU doors have already drifted apart six times, and a helper copied into
//! both suites is one more surface on which a repair can land in one and not
//! the other — a fix to a `SELECT` here, a widened predicate there, and the two
//! halves are silently measuring different things while both stay green.
//!
//! # What belongs here, and what does not
//!
//! Only what is genuinely **suite-agnostic**: reading a value back out of a
//! test database, and standing up a permissive PDP. A seed, a harness or a
//! request builder stays with its own suite, because those encode what a
//! particular door is being asked and moving them would hide the thing a
//! reader of that suite most needs to see.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use async_trait::async_trait;
use authz_resolver_sdk::constraints::{Constraint, InPredicate, Predicate};
use authz_resolver_sdk::models::{
    EvaluationRequest, EvaluationResponse, EvaluationResponseContext,
};
use authz_resolver_sdk::{AuthZResolverClient, AuthZResolverError, PolicyEnforcer};
use sea_orm::{ConnectionTrait, Database, DbBackend, FromQueryResult, Statement};
use toolkit_gts::gts_id;
use toolkit_security::{SecurityContext, pep_properties};
use uuid::Uuid;

use crate::infra::events;

/// Degraded flat-`In` PDP fake: permits and emits a single flat
/// `In([allowed])` constraint over `OWNER_TENANT_ID` — **the shape the
/// production PDP returns for a PEP that advertises no tenant-subtree
/// capability** (this gear: [`PolicyEnforcer::new`] with no
/// `with_capabilities`). The request is ignored: the fake models a subject
/// authorized only for the single `allowed` tenant.
///
/// That first clause is what makes this a measurement rather than a
/// convenience, and review wave D's extraction dropped it — the wave verified
/// the *bodies* were byte-identical and did not compare the docs.
struct FlatInResolver {
    allowed: Uuid,
}

#[async_trait]
impl AuthZResolverClient for FlatInResolver {
    async fn evaluate(
        &self,
        _req: EvaluationRequest,
    ) -> Result<EvaluationResponse, AuthZResolverError> {
        Ok(EvaluationResponse {
            decision: true,
            context: EvaluationResponseContext {
                constraints: vec![Constraint {
                    predicates: vec![Predicate::In(InPredicate::new(
                        pep_properties::OWNER_TENANT_ID,
                        vec![self.allowed],
                    ))],
                }],
                deny_reason: None,
            },
        })
    }
}

/// A fixture instant: `2026-09-02` at `hour`, UTC.
///
/// # Four copies, three epochs, and `at(9)` meant three different things
///
/// **P-D-110** arm 1 hoisted this. `repo_tests` had it on `2026-08-29`,
/// `repo/governance_tests` and `repo/taxonomy_tests` on `2026-09-02`, and
/// `repo/retention_tests` arrived on `2026-09-03` — a new module bringing a
/// new epoch, which is how the drift was accelerating. Two other modules had
/// no `at()` at all.
///
/// **The count was never the trigger.** `harness()` is copied five times too
/// and stays copied: its forms differ only in an `.expect()` message, so
/// unifying it would edit five files to agree on a panic string. This one
/// differed in **meaning**, and that is what a hoist is for.
///
/// `.single()` rather than `.unwrap()`, which is the form one of the four
/// already used and the only one that says what it is asserting: that the
/// civil time names exactly one instant.
#[must_use]
pub fn at(hour: u32) -> chrono::DateTime<chrono::Utc> {
    use chrono::TimeZone as _;
    chrono::Utc
        .with_ymd_and_hms(2026, 9, 2, hour, 0, 0)
        .single()
        .expect("a real instant")
}

/// A [`PolicyEnforcer`] over [`FlatInResolver`], scoped to one tenant.
pub fn flat_in_enforcer(allowed: Uuid) -> PolicyEnforcer {
    PolicyEnforcer::new(Arc::new(FlatInResolver { allowed }))
}

/// An authenticated [`SecurityContext`] for `tenant`, with a fresh subject.
pub fn authed_ctx(tenant: Uuid) -> SecurityContext {
    SecurityContext::builder()
        .subject_id(Uuid::now_v7())
        .subject_tenant_id(tenant)
        .subject_type(gts_id!("cf.core.security.subject_user.v1~"))
        .token_scopes(vec!["*".to_owned()])
        .build()
        .expect("authed SecurityContext must build")
}

/// Run `sql` (a `SELECT ... AS v FROM ...`) on its own auxiliary connection
/// into `dsn` and return the single integer column it names `v`.
///
/// Its own connection, deliberately: the door harnesses pin `max_conns: 1` on
/// the production provider, so introspecting through it would contend with the
/// very statement under test.
pub async fn raw_i64(dsn: &str, sql: &str) -> i64 {
    #[derive(Debug, FromQueryResult)]
    struct Row {
        v: i64,
    }

    let conn = Database::connect(dsn)
        .await
        .expect("open an auxiliary connection for test introspection");
    let row = Row::find_by_statement(Statement::from_string(DbBackend::Sqlite, sql.to_owned()))
        .one(&conn)
        .await
        .expect("the introspection query runs")
        .expect("an aggregate SELECT always returns exactly one row");
    conn.close().await.ok();
    row.v
}

/// [`raw_i64`] for a single nullable text column named `v`.
pub async fn raw_string_opt(dsn: &str, sql: &str) -> Option<String> {
    #[derive(Debug, FromQueryResult)]
    struct Row {
        v: Option<String>,
    }

    let conn = Database::connect(dsn)
        .await
        .expect("open an auxiliary connection for test introspection");
    let row = Row::find_by_statement(Statement::from_string(DbBackend::Sqlite, sql.to_owned()))
        .one(&conn)
        .await
        .expect("the introspection query runs")
        .expect("the row this test just wrote must exist");
    conn.close().await.ok();
    row.v
}

/// Drop `table` from the database at `dsn`, for the seams that need one gone.
pub async fn drop_table(dsn: &str, table: &str) {
    let conn = Database::connect(dsn)
        .await
        .expect("open an auxiliary connection to drop a table");
    conn.execute_unprepared(&format!("DROP TABLE {table};"))
        .await
        .expect("drop the table this seam needs gone");
    conn.close().await.ok();
}

/// The column names `table` declares, as the executed schema holds them.
pub async fn table_columns(dsn: &str, table: &str) -> Vec<String> {
    let joined = raw_string_opt(
        dsn,
        &format!("SELECT group_concat(name, ',') AS v FROM pragma_table_info('{table}')"),
    )
    .await
    .expect("the migration chain created this table, so the pragma answers a non-empty list");
    joined.split(',').map(ToOwned::to_owned).collect()
}

/// How many outbox rows carry `payload_type`.
///
/// Counted on `_body` rather than `_incoming`: `_incoming` is a staging table
/// the running sequencer drains, so a count taken after the response has raced
/// the pipeline.
pub async fn enqueued_event_count(dsn: &str, payload_type: &str) -> i64 {
    let body_table = format!("{}_body", events::OUTBOX_TABLE_PREFIX);
    raw_i64(
        dsn,
        &format!("SELECT COUNT(*) AS v FROM {body_table} WHERE payload_type = '{payload_type}'"),
    )
    .await
}

/// The full envelope of the **newest** enqueued row carrying `payload_type`.
///
/// `ORDER BY id DESC LIMIT 1` rather than a bare filter, so a case that
/// enqueued the same token twice reads the one it just wrote. The `payload`
/// column is a `BLOB`; `CAST(.. AS TEXT)` is what lets [`raw_string_opt`]'s
/// single-text-column shape read it.
pub async fn enqueued_event_envelope(dsn: &str, payload_type: &str) -> serde_json::Value {
    let body_table = format!("{}_body", events::OUTBOX_TABLE_PREFIX);
    let payload = raw_string_opt(
        dsn,
        &format!(
            "SELECT CAST(payload AS TEXT) AS v FROM {body_table} \
             WHERE payload_type = '{payload_type}' ORDER BY id DESC LIMIT 1"
        ),
    )
    .await
    .expect("the enqueued row carries a payload");
    serde_json::from_str(&payload).expect("the door enqueues a JSON envelope")
}

/// How many idempotency rows carry `client_key`.
pub async fn idempotency_rows_for(dsn: &str, client_key: &str) -> i64 {
    raw_i64(
        dsn,
        &format!(
            "SELECT COUNT(*) AS v FROM products_idempotency WHERE client_key = '{client_key}'"
        ),
    )
    .await
}

/// A predicate matching `column` against `id` under **either** rendering.
///
/// `SQLite` stores a `UUID` as a 16-byte `BLOB`, so a bare `= '<hyphenated>'`
/// misses rows the driver wrote as bytes; `hex()` is the other side of that.
pub fn id_matches(column: &str, id: Uuid) -> String {
    let hex = id.simple().to_string().to_uppercase();
    format!("({column} = '{id}' OR hex({column}) = '{hex}')")
}

/// One column of **the** audit row, and a proof that there is exactly one.
///
/// Both readers below carried the precondition "where exactly one was written"
/// in their docs and nothing enforced it: an unqualified `SELECT` over the
/// table hands `raw_string_opt`'s `.one()` an arbitrary row, so a case that
/// wrote a second audit row would read whichever sorted first and keep passing.
/// That is the same defect review wave D fixed for the `hex(actor_ref)` read —
/// and the class sweep that wave declared clean did not catch these, because
/// the detector was keyed to `LIMIT 1` without a `WHERE` and these carry no
/// `LIMIT` at all.
async fn the_one_audit_row(dsn: &str, column: &str) -> Option<String> {
    let rows = raw_i64(dsn, "SELECT COUNT(*) AS v FROM products_audit_log").await;
    assert_eq!(
        rows, 1,
        "these readers name **the** audit row; {rows} were written, so the value read would be \
         whichever the engine returned first"
    );
    raw_string_opt(
        dsn,
        &format!("SELECT {column} AS v FROM products_audit_log"),
    )
    .await
}

/// The `action` of the audit row, where exactly one was written.
pub async fn audit_action(dsn: &str) -> Option<String> {
    the_one_audit_row(dsn, "action").await
}

/// The `error_code` of the audit row, where exactly one was written.
pub async fn audit_error_code(dsn: &str) -> Option<String> {
    the_one_audit_row(dsn, "error_code").await
}

// **Owed, and measured rather than guessed**: 24 sites in the door suites still
// spell `SELECT error_code AS v FROM products_audit_log` inline against 6 that
// call the reader above, and 4 against 4 for `action`. Two spellings of one read
// is the drift surface this module exists to remove — but the swap is not
// mechanical, because the reader now asserts the table holds exactly one row and
// some of those sites may legitimately have written more. Each has to be looked
// at, which is why they are recorded here rather than converted blind.

/// A usage-type resolver that answers `Resolved` for every ref — what a test
/// `ApiState` carries unless a probe injects [`StubUsageTypes`] to script the
/// other two answers. Production never sees it: `gear.rs` installs the
/// collector's client or `NoCollector` (P-D-141).
pub fn resolved_usage_types() -> Arc<dyn crate::infra::usage_types::UsageTypeResolver> {
    Arc::new(StubUsageTypes::always(
        crate::domain::recognized::UsageTypeAnswer::Resolved,
    ))
}

/// A scripted collector: answers in order, then repeats the last one.
pub struct StubUsageTypes {
    answers:
        std::sync::Mutex<std::collections::VecDeque<crate::domain::recognized::UsageTypeAnswer>>,
    last: crate::domain::recognized::UsageTypeAnswer,
    /// How many times the door asked — the *once per publish* clause's operand.
    pub asked: std::sync::atomic::AtomicUsize,
}

impl StubUsageTypes {
    /// One answer, forever.
    #[must_use]
    pub fn always(answer: crate::domain::recognized::UsageTypeAnswer) -> Self {
        Self::scripted([answer])
    }

    /// `answers` in the order the door will receive them; the last repeats.
    #[must_use]
    pub fn scripted(
        answers: impl IntoIterator<Item = crate::domain::recognized::UsageTypeAnswer>,
    ) -> Self {
        let mut queue: std::collections::VecDeque<_> = answers.into_iter().collect();
        let last = queue.pop_back().expect("a stub needs at least one answer");
        Self {
            answers: std::sync::Mutex::new(queue),
            last,
            asked: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl crate::infra::usage_types::UsageTypeResolver for StubUsageTypes {
    async fn resolve(
        &self,
        _ctx: &SecurityContext,
        _usage_type_ref: &str,
    ) -> crate::domain::recognized::UsageTypeAnswer {
        self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let next = self.answers.lock().expect("stub lock").pop_front();
        next.unwrap_or(self.last)
    }
}

/// Seed a **satisfied** approval record for `entity`'s publish subject at
/// `revision`, so a routed act under the real host (`GateHost::Real`,
/// P-D-142) finds the record the door's `governed` constructor demands.
///
/// The record is submitted through `repo::submit_approval` and then flipped
/// to `satisfied` directly — the decision doors are a different suite's
/// concern; this helper stands in for a quorum that has already spoken.
pub async fn seed_satisfied_publish_approval(
    db: &toolkit_db::DBProvider<toolkit_db::DbError>,
    tenant_id: Uuid,
    entity_kind: bss_products_sdk::models::EntityKind,
    entity_id: Uuid,
    revision: i64,
) -> crate::domain::governance::ApprovalId {
    let subject = crate::domain::governance::GateSubject::entity_publish(
        crate::domain::governance::EntityRef {
            tenant_id,
            entity_kind,
            entity_id,
        },
        crate::domain::concurrency::InternalRevision::new(revision),
    );
    seed_satisfied_approval(db, tenant_id, subject, revision).await
}

/// [`seed_satisfied_publish_approval`] for any subject — a live op, the
/// materiality policy, a bulk batch — since the routed live-op doors run the
/// stored host too (P-D-144). `revision` is the stored pin (`0` for a subject
/// with no counter, P-D-120 row 14).
pub async fn seed_satisfied_approval(
    db: &toolkit_db::DBProvider<toolkit_db::DbError>,
    tenant_id: Uuid,
    subject: crate::domain::governance::GateSubject,
    revision: i64,
) -> crate::domain::governance::ApprovalId {
    use crate::domain::governance::ApprovalId;
    use crate::domain::materiality::{
        MaterialAct, MaterialityEvaluator, MaterialityPolicy, Resolution,
    };
    use crate::infra::storage::entity::approval;
    use crate::infra::storage::repo;
    use sea_orm::sea_query::Expr;
    use sea_orm::{ColumnTrait as _, Condition, EntityTrait as _};
    use toolkit_db::secure::SecureUpdateExt as _;

    let conn = db.conn().expect("connection");
    let scope = toolkit_db::secure::AccessScope::for_tenant(tenant_id);
    let approval_id = ApprovalId::new(Uuid::now_v7());
    // A case that seeded its own record for this subject keeps it: a second
    // submission would supersede the open one (L-4) and change what the case
    // is measuring.
    let existing = repo::gate_candidates(&conn, &scope, &subject)
        .await
        .expect("read the subject's candidates");
    if let Some(open) = existing.iter().find(|candidate| {
        matches!(
            candidate.state,
            crate::domain::approval::ApprovalState::Pending
                | crate::domain::approval::ApprovalState::Satisfied
        )
    }) {
        return open.approval_id;
    }
    let policy = MaterialityPolicy::default();
    let evaluator = MaterialityEvaluator::new(Resolution::Resolved(&policy));
    let act = MaterialAct::PolicyMutation;
    repo::submit_approval(
        &conn,
        &scope,
        repo::NewApproval {
            approval_id,
            subject: &subject,
            internal_revision: revision,
            content_snapshot: "{}",
            diff_basis: None,
            act: &act,
            evaluator,
            finance_material: false,
            approver_count: 2,
            submitter: Uuid::from_u128(0xd1_77),
            author_override_ack: None,
        },
        chrono::Utc::now(),
    )
    .await
    .expect("submit the record");
    approval::Entity::update_many()
        .secure()
        .scope_with(&scope)
        .col_expr(approval::Column::State, Expr::value("satisfied".to_owned()))
        .filter(
            Condition::all()
                .add(approval::Column::TenantId.eq(tenant_id))
                .add(approval::Column::ApprovalId.eq(approval_id.get())),
        )
        .exec(&conn)
        .await
        .expect("satisfy the record");
    approval_id
}
