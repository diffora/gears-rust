//! `10-retention-erasure`'s three unattended acts: the retention sweep, the
//! age-triggered tombstone and the restore drill (`dod-retention-clock`,
//! `dod-retention-order`, `dod-erasure-age`, `dod-restore-drill`).
//!
//! Each is a function `gear.rs`'s loop calls; **the function owns its cadence
//! and the loop owns the tick**, which is the shape `report_overdue_freezes`
//! already uses for a scan whose answer changes on a scale of hours.
//!
//! # Every act here runs under the system principal, with no correlation id
//!
//! `gear::system_actor_ref()` (**P-D-113** arm 2), and `correlation_id` is
//! `None` because a background act **has** no request (**P-D-118** item 16) —
//! that is a fact about the act, not a hole in the row.
//!
//! # The sweep judges and deletes per candidate, each in its own transaction
//!
//! **P-D-136**, taking the shape this strand proposed and P-D-118 item 25
//! already required of catalog versions, generalised to every class. The
//! failure it guards is named: a collector reaching a flat-refusal row raises
//! `P0001`, which is **not** retryable contention, so a sweep that judged a
//! whole class in one transaction would abort and take every unrelated
//! candidate with it.
//!
//! # Two classes collect and one is held, and the shapes differ
//!
//! **Financial** collects under a release stamp (**P-D-137**): a catalog
//! version is a financial record with a statutory window, so the sweep stamps
//! `retention_released_at` and then deletes captures, entries and the version
//! in **one** transaction — item 25's *"whole"*, and the order the FK and
//! both `DELETE` arms require anyway. **Version** collects under P-D-40's
//! referential predicate, for rows no manifest references and no head names.
//! **Audit** — the log and the four evidential stores — is **held**, and that
//! is P-D-136's decided posture rather than a gap: evidence is not deletable
//! in v1, and at a ten-year window no collector reaches one of those rows
//! before 2036.
//!
//! The hold is **reported, not assumed**: the sweep offers the delete and
//! classifies the refusal, so the day a migration opens an arm the sweep
//! starts collecting with no edit here — which is exactly how the financial
//! class began collecting when P-D-137 opened its two.
//!
//! # One doomed statement per class per pass, not one per row
//!
//! A guard is a property of the **table**, not of the row, so the first
//! refusal in a class settles that class for the pass and the remaining
//! candidates are held under the same reason without a second statement. The
//! hold stays **measured** — the class is asked once, every pass — while the
//! cost stays bounded as the held population grows.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-retention-clock:p1
//! @cpt-dod:cpt-cf-bss-products-dod-retention-order:p1
//! @cpt-dod:cpt-cf-bss-products-dod-erasure-age:p1
//! @cpt-dod:cpt-cf-bss-products-dod-restore-drill:p2

use chrono::{DateTime, Utc};
use toolkit_db::secure::AccessScope;
use toolkit_db::{DBProvider, DbError};
use uuid::Uuid;

use crate::domain::retention::{ClassOutcome, HeldReason, RecordClass, RetentionCaps};
use crate::infra::storage::RepoError;
use crate::infra::storage::repo::{self, AuditClassCandidate, EntityVersionKey};

/// How many rows of one store one pass will look at.
///
/// A retention pass is telemetry-shaped work on a table that only grows, so
/// an unbounded read would load a decade of audit rows to decide that none of
/// them may be deleted. Five hundred is enough that a real backlog drains
/// over hours rather than never, and small enough that one pass is a short
/// transaction's worth of judging. Not a configuration knob: the number
/// changes how fast a backlog drains and never *what* is deleted, and
/// `ProductsConfig` is for operands a deployment must be able to set.
const PASS_BOUND: u64 = 500;

/// The subject kind a sweep's audit rows carry.
const SWEEP_SUBJECT_KIND: &str = "retention_sweep";

/// One retention sweep: every tenant, every class (`dod-retention-clock`).
///
/// Errors are logged per tenant and the sweep continues — one tenant's
/// storage failure is not a reason to stop collecting for the others, the
/// same posture `increment::sweep` takes.
///
/// @cpt-flow:cpt-cf-bss-products-flow-retention:p1
pub async fn sweep(
    db: &DBProvider<DbError>,
    caps: &RetentionCaps,
    actor_ref: Uuid,
    now: DateTime<Utc>,
    cancel: &tokio_util::sync::CancellationToken,
) {
    let Some(tenants) = discover_tenants(db, "retention sweep").await else {
        return;
    };
    for tenant_id in tenants {
        if cancel.is_cancelled() {
            return;
        }
        for class in RecordClass::ALL {
            if cancel.is_cancelled() {
                return;
            }
            match sweep_class(db, caps, tenant_id, class, actor_ref, now).await {
                Ok(outcome) => log_class_outcome(tenant_id, class, &outcome),
                Err(error) => tracing::warn!(
                    %tenant_id,
                    class = class.as_str(),
                    %error,
                    "bss-products: retention pass failed; later classes continue"
                ),
            }
        }
    }
}

/// Every tenant the three sweeps run over.
///
/// One function rather than three copies: all three entry points want the
/// same set and the same "log it and do nothing this pass" posture on a
/// failure, and three copies is how two of them come to differ.
///
/// `None` means the pass does not run — never an empty pass, which would log
/// as a tenant-less deployment rather than as a failed read.
async fn discover_tenants(db: &DBProvider<DbError>, act: &str) -> Option<Vec<Uuid>> {
    let Ok(conn) = db.conn() else {
        tracing::warn!(
            act,
            "bss-products: retention act could not open a connection"
        );
        return None;
    };
    match repo::tenants_with_retention_history(&conn, &AccessScope::allow_all()).await {
        Ok(tenants) => Some(tenants),
        Err(error) => {
            tracing::warn!(act, %error, "bss-products: retention act discovery failed");
            None
        }
    }
}

/// One pass over one class for one tenant.
///
/// # Errors
///
/// [`RepoError`] only from the **discovery** read or the outcome's own audit
/// row. A refused delete is not an error here — it is a `held` candidate,
/// which is the whole of P-D-136's shape.
pub(crate) async fn sweep_class(
    db: &DBProvider<DbError>,
    caps: &RetentionCaps,
    tenant_id: Uuid,
    class: RecordClass,
    actor_ref: Uuid,
    now: DateTime<Utc>,
) -> Result<ClassOutcome, RepoError> {
    let scope = AccessScope::for_tenant(tenant_id);
    let cutoff = caps.cutoff(class, now);
    let conn = db
        .conn()
        .map_err(|e| RepoError::Db(format!("retention sweep connection: {e}")))?;

    let mut outcome = ClassOutcome::default();
    // Once a class's storage refuses, the rest of its candidates are held
    // under the same reason without a second doomed statement — see this
    // module's doc.
    let mut refused: Option<HeldReason> = None;

    match class {
        RecordClass::Financial => {
            let candidates =
                repo::catalog_version_candidates(&conn, &scope, tenant_id, cutoff).await?;
            outcome.candidates = u32::try_from(candidates.len()).unwrap_or(u32::MAX);
            for catalog_version_id in candidates {
                if let Some(reason) = refused.as_ref() {
                    outcome.hold(reason);
                    continue;
                }
                match collect_catalog_version(db, &scope, tenant_id, catalog_version_id, now).await
                {
                    Ok(()) => outcome.collect(),
                    Err(reason) => {
                        outcome.hold(&reason);
                        // A freeze hold is a property of the VERSION, not of
                        // the table, so it must not settle the class: the
                        // next candidate may be perfectly collectable.
                        if matches!(reason, HeldReason::StorageRefused(_)) {
                            refused = Some(reason);
                        }
                    }
                }
            }
        }
        RecordClass::Version => {
            let candidates =
                repo::entity_version_candidates(&conn, &scope, tenant_id, cutoff).await?;
            outcome.candidates = u32::try_from(candidates.len()).unwrap_or(u32::MAX);
            for key in candidates {
                match collect_entity_version(db, &scope, tenant_id, &key).await {
                    Ok(()) => outcome.collect(),
                    // Never settles the class: this refusal IS the derive
                    // rule, and it is per row — the next candidate may have
                    // no referencing manifest at all.
                    Err(reason) => outcome.hold(&reason),
                }
            }
        }
        RecordClass::Audit => {
            let candidates =
                repo::audit_class_candidates(&conn, &scope, tenant_id, cutoff, PASS_BOUND).await?;
            outcome.candidates = u32::try_from(candidates.len()).unwrap_or(u32::MAX);
            for candidate in candidates {
                if let Some(reason) = refused.as_ref() {
                    outcome.hold(reason);
                    continue;
                }
                match collect_audit_class_row(db, &scope, tenant_id, &candidate).await {
                    Ok(()) => outcome.collect(),
                    Err(reason) => {
                        outcome.hold(&reason);
                        refused = Some(reason);
                    }
                }
            }
        }
    }

    write_pass_audit(
        db, &scope, tenant_id, class, cutoff, &outcome, actor_ref, now,
    )
    .await?;
    Ok(outcome)
}

/// One catalog version, whole, in one transaction (**P-D-118** item 25).
///
/// The gate runs **first and outside** the delete: `domain::retention`'s
/// predicate answers `Held` for a version whose freeze ledger still carries a
/// live registration (C4 — skipped, never forced), and a version held that
/// way must not have its captures deleted on the way to finding out.
async fn collect_catalog_version(
    db: &DBProvider<DbError>,
    scope: &AccessScope,
    tenant_id: Uuid,
    catalog_version_id: i64,
    now: DateTime<Utc>,
) -> Result<(), HeldReason> {
    let conn = db
        .conn()
        .map_err(|e| HeldReason::StorageRefused(format!("connection: {e}")))?;
    let record = repo::find_catalog_version(&conn, scope, tenant_id, catalog_version_id)
        .await
        .map_err(|e| HeldReason::StorageRefused(e.to_string()))?;
    let Some(record) = record else {
        // A racer collected it between the discovery read and now. Not a
        // hold and not an error: the row is gone, which is what the pass
        // wanted.
        return Ok(());
    };
    let registrations = repo::freeze_registrations(&conn, scope, tenant_id, catalog_version_id)
        .await
        .map_err(|e| HeldReason::StorageRefused(e.to_string()))?;
    let snapshot = decode_snapshot(&record.participant_set_snapshot);
    if let crate::domain::retention::RetentionVerdict::Held(holds) =
        crate::domain::retention::evaluate(&snapshot, &registrations)
    {
        // Every hold is reported by the predicate; the first names the
        // participant an operator would chase first.
        let first = holds.into_iter().next().unwrap_or(
            crate::domain::retention::RetentionHold::NoRegistration {
                participant: String::new(),
            },
        );
        return Err(HeldReason::FreezeLive(first));
    }

    // The release stamp and the three deletes, in **one** transaction
    // (P-D-118 item 25's "whole"). The stamp goes first because it is what
    // the arms read: `m20260901_000013`'s predicate admits an entry or a
    // capture whose parent carries it, and `m20260901_000010`'s admits the
    // parent itself — so a pass that deleted first would meet a refusal it
    // had the authority to lift.
    let scope_tx = scope.clone();
    db.db()
        .transaction_ref_mapped::<_, (), TxError>(move |tx| {
            let scope = scope_tx.clone();
            Box::pin(async move {
                repo::stamp_retention_release(tx, &scope, tenant_id, catalog_version_id, now)
                    .await
                    .map_err(TxError::Repo)?;
                repo::delete_catalog_version(tx, &scope, tenant_id, catalog_version_id)
                    .await
                    .map_err(TxError::Repo)
            })
        })
        .await
        .map_err(|e| HeldReason::StorageRefused(e.to_string()))
}

/// One entity-version row, offered to the engine's referential predicate.
async fn collect_entity_version(
    db: &DBProvider<DbError>,
    scope: &AccessScope,
    tenant_id: Uuid,
    key: &EntityVersionKey,
) -> Result<(), HeldReason> {
    let scope_tx = scope.clone();
    let key_tx = key.clone();
    db.db()
        .transaction_ref_mapped::<_, (), TxError>(move |tx| {
            let scope = scope_tx.clone();
            let key = key_tx.clone();
            Box::pin(async move {
                repo::delete_entity_version(tx, &scope, tenant_id, &key)
                    .await
                    .map_err(TxError::Repo)
            })
        })
        .await
        // **Error class follows provenance** (P-D-137): only P-D-40's
        // refusal is the derive rule, and its own message is the operand
        // that says so. A connection failure or a scope refusal mapped to
        // `ReferencedByRetainedManifest` would be audited as a *design*
        // hold — a row an operator would read as correctly retained when it
        // was never judged at all.
        .map_err(|error| classify_entity_version_failure(&error.to_string()))
}

/// Which hold a failed entity-version delete is.
///
/// **Error class follows provenance** (P-D-137 (ii)): only P-D-40's refusal
/// is the derive rule, and the migration names itself in its message, so the
/// message is the operand. Everything else — a connection failure, a scope
/// refusal, a table that is gone — is a storage refusal, and mapping it to
/// the derive rule would audit it as a **design** hold: a row an operator
/// reads as correctly retained when it was never judged at all.
///
/// Its own function so the classification can be probed without a live
/// engine: the failures that must NOT be the derive rule are exactly the ones
/// hard to provoke through a working database.
///
/// @cpt-algo:cpt-cf-bss-products-algo-retention-errors:p1
pub(crate) fn classify_entity_version_failure(rendered: &str) -> HeldReason {
    if rendered.contains("P-D-40") {
        HeldReason::ReferencedByRetainedManifest
    } else {
        HeldReason::StorageRefused(rendered.to_owned())
    }
}

/// One audit-class row, offered to its guard.
async fn collect_audit_class_row(
    db: &DBProvider<DbError>,
    scope: &AccessScope,
    tenant_id: Uuid,
    candidate: &AuditClassCandidate,
) -> Result<(), HeldReason> {
    let scope_tx = scope.clone();
    let candidate_tx = candidate.clone();
    db.db()
        .transaction_ref_mapped::<_, (), TxError>(move |tx| {
            let scope = scope_tx.clone();
            let candidate = candidate_tx.clone();
            Box::pin(async move {
                repo::delete_audit_class_row(tx, &scope, tenant_id, &candidate)
                    .await
                    .map_err(TxError::Repo)
            })
        })
        .await
        .map_err(|e| HeldReason::StorageRefused(e.to_string()))
}

/// The pass's own audit row: the class, the clock and the verdict.
///
/// **One row per class per pass, not one per candidate**, and the reason is
/// arithmetic: with the evidence class held by design for a decade, a row per
/// held candidate would write audit rows about audit rows every cadence, and
/// each of those becomes a candidate in its turn. The `DoD`'s *"every GC act
/// is audited with the class, the clock and the gate verdict"* is satisfied
/// by the act being **the pass over the class** — which is what has a class,
/// a clock and a verdict. A collected row is a different thing and is a
/// per-row act, but there is nothing to record it *in* once it is gone, so
/// the count is the record.
#[allow(
    clippy::too_many_arguments,
    reason = "an audit row's own field set: \
    the subject, the class, the clock, the counts and the principal all reach \
    the row, and grouping them into a struct used at one call site would hide \
    the shape rather than share it"
)]
async fn write_pass_audit(
    db: &DBProvider<DbError>,
    scope: &AccessScope,
    tenant_id: Uuid,
    class: RecordClass,
    cutoff: DateTime<Utc>,
    outcome: &ClassOutcome,
    actor_ref: Uuid,
    now: DateTime<Utc>,
) -> Result<(), RepoError> {
    let reason = format!(
        "class={} cutoff={} candidates={} collected={} held={} held_reason={}",
        class.as_str(),
        cutoff.to_rfc3339(),
        outcome.candidates,
        outcome.collected,
        outcome.held,
        outcome.held_reason.unwrap_or("none"),
    );
    let audit_id = Uuid::now_v7();
    let scope_tx = scope.clone();
    db.db()
        .transaction_ref_mapped::<_, (), TxError>(move |tx| {
            let scope = scope_tx.clone();
            let reason = reason.clone();
            Box::pin(async move {
                repo::write_eventless_act_audit(
                    tx,
                    &scope,
                    repo::AuditCommon {
                        audit_id,
                        tenant_id,
                        actor_ref,
                        action: "retention.sweep".to_owned(),
                        subject_kind: SWEEP_SUBJECT_KIND.to_owned(),
                        reason: Some(reason),
                        // A background act has no request (P-D-118 item 16).
                        correlation_id: None,
                        written_at: now,
                    },
                    // The subject IS the tenant: a pass has no other id, and
                    // minting one would put a value in the column that
                    // identifies nothing.
                    tenant_id,
                    None,
                )
                .await
                .map_err(TxError::Repo)
            })
        })
        .await
        .map_err(RepoError::from)
}

/// The participant snapshot, decoded.
///
/// `participant_set_snapshot` is the canonical rendering of a string array
/// (P-D-67). A row that does not parse yields an **empty** snapshot, which
/// `domain::retention::evaluate` reads as *collectable* — so the decode
/// failure is logged and the version is held here rather than silently
/// admitted by the vacuity the gate deliberately allows.
fn decode_snapshot(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_else(|error| {
        tracing::warn!(%error, "bss-products: unparseable participant snapshot");
        // One unnamed member: `evaluate` answers `Held(NoRegistration)` for a
        // snapshot member with no registration row, which is the fail-closed
        // answer a corrupt snapshot must get.
        vec![String::new()]
    })
}

fn log_class_outcome(tenant_id: Uuid, class: RecordClass, outcome: &ClassOutcome) {
    if outcome.candidates == 0 {
        return;
    }
    tracing::info!(
        %tenant_id,
        class = class.as_str(),
        candidates = outcome.candidates,
        collected = outcome.collected,
        held = outcome.held,
        held_reason = outcome.held_reason.unwrap_or("none"),
        "bss-products: retention_pass"
    );
}

/// This module's transaction error.
///
/// `transaction_ref_mapped` requires `E: From<DbError>`, and `RepoError` has
/// no such impl by design — its `Db` arm is a rendered string and its
/// `Driver` arm preserves `sea-orm`'s own error, so a blanket `From` would
/// have to choose one and would erase the other for every existing caller.
/// A local wrapper is the smaller change.
enum TxError {
    Repo(RepoError),
}

impl From<DbError> for TxError {
    fn from(error: DbError) -> Self {
        Self::Repo(RepoError::Db(error.to_string()))
    }
}

impl From<TxError> for RepoError {
    fn from(error: TxError) -> Self {
        match error {
            TxError::Repo(e) => e,
        }
    }
}

impl std::fmt::Display for TxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Repo(e) => write!(f, "{e}"),
        }
    }
}

// -- The age-triggered tombstone (`dod-erasure-age`) --

/// The subject kind an age-triggered tombstone's audit row carries — the same
/// one the erasure **door** writes, because it is the same act.
const IDENTITY_SUBJECT_KIND: &str = "identity_ref";

/// The reason an age-triggered tombstone records.
///
/// The requested path is *"audited with a reason"* supplied by a human; the
/// age path has no requester and no supplied reason, so its row carries the
/// **age rule's own name** (**P-D-117** item 14). A row with a system reason
/// is honest about its origin where a row without one would be a hole in the
/// very class this feature retains.
const AGE_REASON: &str = "pseudonymization_age_days elapsed since last_seen_at (inst-er-age)";

/// One age-triggered pseudonymization pass (`dod-erasure-age`).
///
/// **One mechanism, two triggers.** This calls the same store function the
/// erasure door calls and emits the same `ActorErased` — the `DoD` forbids a
/// second code path, and the way that is honoured is that
/// [`repo::tombstone_principal`] and `enqueue_retention` are reached from
/// here exactly as `api::rest::retention::execute_erasure` reaches them.
///
/// What the two paths leave **identical** is the map entry: tombstoned,
/// payload destroyed, `principal_ref` standing. What differs by construction
/// is the audit row — its actor is the system principal and its reason is
/// [`AGE_REASON`] (P-D-117 item 14).
pub async fn tombstone_aged_principals(
    db: &DBProvider<DbError>,
    sink: &crate::infra::broker::EventSink,
    caps: &RetentionCaps,
    actor_ref: Uuid,
    now: DateTime<Utc>,
    cancel: &tokio_util::sync::CancellationToken,
) {
    let cutoff = crate::domain::retention::cutoff_before(now, caps.pseudonymization_age_days);
    let Some(tenants) = discover_tenants(db, "age sweep").await else {
        return;
    };
    for tenant_id in tenants {
        if cancel.is_cancelled() {
            return;
        }
        if let Err(error) = tombstone_tenant(db, sink, tenant_id, cutoff, actor_ref, now).await {
            tracing::warn!(
                %tenant_id,
                %error,
                "bss-products: age pass failed; later tenants continue"
            );
        }
    }
}

/// One tenant's aged principals.
async fn tombstone_tenant(
    db: &DBProvider<DbError>,
    sink: &crate::infra::broker::EventSink,
    tenant_id: Uuid,
    cutoff: DateTime<Utc>,
    actor_ref: Uuid,
    now: DateTime<Utc>,
) -> Result<u32, RepoError> {
    let scope = AccessScope::for_tenant(tenant_id);
    let aged = {
        let conn = db
            .conn()
            .map_err(|e| RepoError::Db(format!("age sweep connection: {e}")))?;
        repo::principals_older_than(&conn, &scope, tenant_id, cutoff, PASS_BOUND).await?
    };
    let mut erased = 0_u32;
    for principal_ref in aged {
        // Per principal, its own transaction — the sweep's discipline
        // (P-D-136) applied to the act the door performs one at a time.
        let scope_tx = scope.clone();
        let sink_tx = sink.clone();
        let principal_tx = principal_ref.clone();
        let audit_id = Uuid::now_v7();
        let outcome = db
            .db()
            .transaction_ref_mapped::<_, Option<Uuid>, TxError>(move |tx| {
                let scope = scope_tx.clone();
                let sink = sink_tx.clone();
                let principal_ref = principal_tx.clone();
                Box::pin(async move {
                    let Some(retired) =
                        repo::tombstone_principal(tx, &scope, tenant_id, &principal_ref, now)
                            .await
                            .map_err(TxError::Repo)?
                    else {
                        return Ok(None);
                    };
                    repo::write_evidential_act_audit(
                        tx,
                        &scope,
                        repo::AuditCommon {
                            audit_id,
                            tenant_id,
                            actor_ref,
                            action: "erasure.execute".to_owned(),
                            subject_kind: IDENTITY_SUBJECT_KIND.to_owned(),
                            reason: Some(AGE_REASON.to_owned()),
                            correlation_id: None,
                            written_at: now,
                        },
                        retired,
                    )
                    .await
                    .map_err(TxError::Repo)?;
                    crate::infra::events::enqueue_retention(
                        &sink,
                        tx,
                        crate::infra::events::retention_aggregate_id(tenant_id, &principal_ref),
                        crate::infra::events::ACTOR_ERASED_PAYLOAD_TYPE,
                        &crate::infra::events::RetentionEventBody {
                            tenant_id,
                            subject_ref: &principal_ref,
                            act: "erased",
                            erased_actor_ref: Some(retired),
                        },
                        actor_ref,
                    )
                    .await
                    .map_err(|e| TxError::Repo(RepoError::Db(format!("retention event: {e}"))))?;
                    Ok(Some(retired))
                })
            })
            .await;
        match outcome {
            // `None` is a racer: the principal was tombstoned between the
            // discovery read and the act. Not an error and not a count.
            Ok(None) => {}
            Ok(Some(_)) => erased = erased.saturating_add(1),
            Err(error) => tracing::warn!(
                %tenant_id,
                %error,
                "bss-products: age tombstone failed; later principals continue"
            ),
        }
    }
    if erased > 0 {
        tracing::info!(%tenant_id, erased, "bss-products: pseudonymization_age_pass");
    }
    Ok(erased)
}

// -- The restore drill (`dod-restore-drill`; P-D-133 items 7/10/30,
//    P-D-134 item 6, P-D-135) --

/// The stable event name a `digest_version` with no recomputation code
/// raises. A **warning**: the row may be perfectly intact and this drill
/// cannot say so.
const DRILL_UNVERIFIABLE: &str = "products_restore_drill_unverifiable";

/// The stable event name a real mismatch raises. An **alarm**: a row whose
/// digest can be recomputed and does not match is a compliance incident, not
/// a log line (C5).
const DRILL_CORRUPTION: &str = "products_restore_drill_corruption";

/// The subject kind a drill run's audit row carries.
const DRILL_SUBJECT_KIND: &str = "restore_drill";

/// How many catalog versions one drill run samples per tenant.
///
/// **The sample is the newest N catalog versions, and their referenced entity
/// versions are all scanned** — not a random sample. Two reasons, both about
/// what a drill is for: corruption in a backup is found by reading a *recent*
/// restore, because that is the one an incident would restore from; and a
/// deterministic sample makes two consecutive runs comparable, where a random
/// one turns a real regression into a coin flip. Every row inside the sample
/// is scanned on every drill (P-D-133 item 7), so the sample bounds the
/// *versions* looked at and never the *rows* verified within them.
const DRILL_SAMPLE: u64 = 20;

/// What one drill run found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DrillOutcome {
    /// Catalog versions sampled.
    pub versions: u32,
    /// Rows whose digest was recomputed and matched.
    pub verified: u32,
    /// Rows whose `digest_version` has no recomputation code here.
    pub unverifiable: u32,
    /// Rows whose digest was recomputed and did **not** match.
    pub corrupt: u32,
    /// `ok`, `no_target`, or `unreachable`.
    pub status: &'static str,
}

/// One restore drill (`dod-restore-drill`).
///
/// The gear owns the **probe**; the platform owns the restore (**P-D-133**).
/// So this reads a restored copy through `drill_target_dsn` and re-verifies
/// **both** halves C5 names: the manifest checksum, rebuilt from the restored
/// rows through `VersionManifest::render`, and each referenced row's
/// `content_digest`, recomputed byte-for-byte through `domain::canonical`.
/// Manifest checksums alone are blind to version-history corruption, which is
/// why the `DoD` names both.
///
/// **With no target configured the run still happens**, writes its row with
/// outcome `no_target` and raises the `unverifiable` warning (**P-D-135**): a
/// drill that cannot run is not a passed drill, and silence is exactly what
/// P-D-133's *"report, never skip"* forbids.
///
/// @cpt-flow:cpt-cf-bss-products-flow-restore-drill:p2
pub async fn run_restore_drill(
    db: &DBProvider<DbError>,
    caps: &RetentionCaps,
    actor_ref: Uuid,
    now: DateTime<Utc>,
    cancel: &tokio_util::sync::CancellationToken,
) {
    let Some(tenants) = discover_tenants(db, "restore drill").await else {
        return;
    };

    let target = match open_drill_target(caps.drill_target_dsn.as_deref()).await {
        Ok(target) => target,
        Err(status) => {
            // No target, or a target that would not open. Every tenant still
            // gets its row and its warning: the watermark is the newest such
            // row per tenant (P-D-134 item 6), and a tenant with no row at
            // all reads as "never drilled" rather than as "could not run".
            for tenant_id in tenants {
                let outcome = DrillOutcome {
                    status,
                    ..DrillOutcome::default()
                };
                raise_drill_signals(tenant_id, &outcome);
                write_drill_audit(db, tenant_id, &outcome, actor_ref, now).await;
            }
            return;
        }
    };

    for tenant_id in tenants {
        if cancel.is_cancelled() {
            return;
        }
        let outcome = drill_tenant(&target, tenant_id)
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(%tenant_id, %error, "bss-products: restore drill read failed");
                DrillOutcome {
                    status: "unreachable",
                    ..DrillOutcome::default()
                }
            });
        raise_drill_signals(tenant_id, &outcome);
        write_drill_audit(db, tenant_id, &outcome, actor_ref, now).await;
    }
}

/// Open the restored copy, or say why not.
///
/// `Err(status)` carries the outcome the run records: `no_target` when none
/// is configured (P-D-135) and `unreachable` when one is and will not open.
/// The two are kept apart because they need different operators — the first
/// is a deployment that has not wired the drill, the second is a restore that
/// is not there.
async fn open_drill_target(dsn: Option<&str>) -> Result<DBProvider<DbError>, &'static str> {
    let Some(dsn) = dsn else {
        return Err("no_target");
    };
    let opts = toolkit_db::ConnectOpts {
        max_conns: Some(1),
        min_conns: Some(1),
        ..Default::default()
    };
    match toolkit_db::connect_db(dsn, opts).await {
        Ok(db) => Ok(DBProvider::<DbError>::new(db)),
        Err(error) => {
            tracing::warn!(%error, "bss-products: restore drill target would not open");
            Err("unreachable")
        }
    }
}

/// One tenant's sample, on the restored copy.
async fn drill_tenant(
    target: &DBProvider<DbError>,
    tenant_id: Uuid,
) -> Result<DrillOutcome, RepoError> {
    let scope = AccessScope::for_tenant(tenant_id);
    let conn = target
        .conn()
        .map_err(|e| RepoError::Db(format!("drill target connection: {e}")))?;
    let sampled = repo::newest_catalog_versions(&conn, &scope, tenant_id, DRILL_SAMPLE).await?;

    let mut outcome = DrillOutcome {
        status: "ok",
        ..DrillOutcome::default()
    };
    for catalog_version_id in sampled {
        outcome.versions = outcome.versions.saturating_add(1);
        let Some(record) =
            repo::find_catalog_version(&conn, &scope, tenant_id, catalog_version_id).await?
        else {
            continue;
        };
        let (entries, captures) =
            repo::catalog_version_manifest_rows(&conn, &scope, tenant_id, catalog_version_id)
                .await?;
        let participant_set = serde_json::from_str::<Vec<String>>(&record.participant_set_snapshot)
            .unwrap_or_default();

        verify_manifest(
            tenant_id,
            catalog_version_id,
            &record,
            &entries,
            captures,
            participant_set,
            &mut outcome,
        );
        verify_referenced_versions(
            &conn,
            &scope,
            tenant_id,
            catalog_version_id,
            &entries,
            &mut outcome,
        )
        .await?;
    }
    Ok(outcome)
}

/// C5's first half: the manifest checksum, rebuilt from the restored rows.
///
/// A version written under a digest rule this build cannot recompute is
/// `unverifiable` and **never** a corruption alarm (P-D-133 item 7): the
/// re-render would manufacture the mismatch.
#[allow(
    clippy::too_many_arguments,
    reason = "the manifest's own operands: \
    a checksum needs its record, both manifest halves and the participant \
    set, and the outcome it counts into. Grouping them into a struct used at \
    one call site would hide the shape rather than share it"
)]
fn verify_manifest(
    tenant_id: Uuid,
    catalog_version_id: i64,
    record: &repo::CatalogVersionRecord,
    entries: &[repo::SnapshotEntityRef],
    captures: Vec<(String, String)>,
    participant_set: Vec<String>,
    outcome: &mut DrillOutcome,
) {
    if record.digest_version != crate::domain::canonical::DIGEST_VERSION {
        outcome.unverifiable = outcome.unverifiable.saturating_add(1);
        return;
    }
    let manifest = crate::infra::increment::VersionManifest {
        entries: entries.to_vec(),
        captures,
        participant_set,
    };
    if manifest.checksum() == record.checksum {
        outcome.verified = outcome.verified.saturating_add(1);
    } else {
        outcome.corrupt = outcome.corrupt.saturating_add(1);
        tracing::error!(
            %tenant_id,
            catalog_version_id,
            "bss-products: {DRILL_CORRUPTION} manifest checksum mismatch"
        );
    }
}

/// C5's second half: **and their referenced entity versions**.
///
/// Manifest checksums alone are blind to version-history corruption, which is
/// the whole reason the `DoD` names both halves — a manifest can be perfectly
/// intact while a row it points at has rotted.
async fn verify_referenced_versions(
    conn: &impl toolkit_db::secure::DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    catalog_version_id: i64,
    entries: &[repo::SnapshotEntityRef],
    outcome: &mut DrillOutcome,
) -> Result<(), RepoError> {
    for entry in entries {
        let row = repo::entity_version_digest(
            conn,
            scope,
            tenant_id,
            &entry.entity_kind,
            entry.entity_id,
            entry.published_version,
        )
        .await?;
        let Some(row) = row else {
            // A manifest entry whose row is gone from the restore is
            // corruption of exactly the kind this drill exists to find.
            outcome.corrupt = outcome.corrupt.saturating_add(1);
            tracing::error!(
                %tenant_id,
                catalog_version_id,
                entity_id = %entry.entity_id,
                "bss-products: {DRILL_CORRUPTION} manifest entry has no version row"
            );
            continue;
        };
        if row.digest_version != crate::domain::canonical::DIGEST_VERSION {
            outcome.unverifiable = outcome.unverifiable.saturating_add(1);
            continue;
        }
        if crate::domain::canonical::content_digest(&row.content) == row.content_digest {
            outcome.verified = outcome.verified.saturating_add(1);
        } else {
            outcome.corrupt = outcome.corrupt.saturating_add(1);
            tracing::error!(
                %tenant_id,
                entity_id = %entry.entity_id,
                published_version = entry.published_version,
                "bss-products: {DRILL_CORRUPTION} content digest mismatch"
            );
        }
    }
    Ok(())
}

/// Raise the run's signals: the alarm on any mismatch, the warning on any
/// row this build cannot recompute — **both**, when both happened.
///
/// The channel is `tracing::warn!` / `error!` with a stable event name, as
/// `gear.rs`'s loops already do. There is no metrics facility in the toolkit
/// this gear links; adding a crate for one is a dependency decision and not
/// this `DoD`'s.
fn raise_drill_signals(tenant_id: Uuid, outcome: &DrillOutcome) {
    if outcome.corrupt > 0 {
        tracing::error!(
            %tenant_id,
            corrupt = outcome.corrupt,
            versions = outcome.versions,
            "bss-products: {DRILL_CORRUPTION}"
        );
    }
    if outcome.unverifiable > 0 || outcome.status != "ok" {
        tracing::warn!(
            %tenant_id,
            unverifiable = outcome.unverifiable,
            status = outcome.status,
            "bss-products: {DRILL_UNVERIFIABLE}"
        );
    }
}

/// The run's state: **an audit row per run** (P-D-134 item 6), P-D-21's own
/// class — an act that emits no event.
///
/// *"The last-verified watermark"* is the newest such row per tenant: a
/// query, not a table, which is what keeps §4's *"config + audit, no new
/// record tables"* true.
async fn write_drill_audit(
    db: &DBProvider<DbError>,
    tenant_id: Uuid,
    outcome: &DrillOutcome,
    actor_ref: Uuid,
    now: DateTime<Utc>,
) {
    let reason = format!(
        "status={} versions={} verified={} unverifiable={} corrupt={}",
        outcome.status, outcome.versions, outcome.verified, outcome.unverifiable, outcome.corrupt,
    );
    let audit_id = Uuid::now_v7();
    let scope = AccessScope::for_tenant(tenant_id);
    let result = db
        .db()
        .transaction_ref_mapped::<_, (), TxError>(move |tx| {
            let scope = scope.clone();
            let reason = reason.clone();
            Box::pin(async move {
                repo::write_eventless_act_audit(
                    tx,
                    &scope,
                    repo::AuditCommon {
                        audit_id,
                        tenant_id,
                        actor_ref,
                        action: "retention.restore_drill".to_owned(),
                        subject_kind: DRILL_SUBJECT_KIND.to_owned(),
                        reason: Some(reason),
                        correlation_id: None,
                        written_at: now,
                    },
                    tenant_id,
                    None,
                )
                .await
                .map_err(TxError::Repo)
            })
        })
        .await;
    if let Err(error) = result {
        // The row is the run's only record, so failing to write it is worth
        // its own line — but it must not stop the remaining tenants' drills.
        tracing::warn!(%tenant_id, %error, "bss-products: restore drill audit row failed");
    }
}

#[cfg(test)]
#[path = "retention_tests.rs"]
mod retention_tests;
