//! `ReadModelWarmJob` — the sweep, and the whole of §4.4's degraded path.
//!
//! The reconciliation **is** the re-drive. A publish commit leaves a pending
//! `pricing_catalog_version_ref` and no version; this pass asks the registry
//! what version each pending handle landed in, hands the answer to the
//! projector, and arrives again next tick for whatever did not complete. There
//! is no second mechanism: §4.4's "the warm re-drive continues past the SLO
//! with no bound" is this pass running again.
//!
//! # Why a pull, and why here
//!
//! `CatalogVersionPublished` is the registry's event (D-66) and there is no
//! broker in this repository, so an inbound port would be a trait with neither
//! producer nor caller. The pull is already contracted:
//! `CatalogVersionRegistryV1::committed_version`'s own doc hands the overdue
//! decision to the caller "since only it knows how long the ref has been
//! outstanding" — and the only thing that knows that is a sweep over
//! `requested_at`. [`crate::infra::read_model`] carries the argument and names
//! the entry point a registry event handler will hook to when the registry gear
//! lands.
//!
//! # Inert without a registry, and that is a requirement rather than a nicety
//!
//! With `UnconfiguredCatalogVersionRegistryV1` every `committed_version` call
//! answers [`CatalogVersionRegistryError::Unconfigured`]. The pass detects that
//! **once** and returns at **debug** — no alarm, no error, never a boot or
//! shutdown failure. The sibling ledger's `RateSyncJob` states exactly this
//! posture for its own unconfigured provider, and the e2e that boots this gear
//! without a registry depends on it: a pass logging an error every five seconds
//! would make that boot unreadable.
//!
//! # What "incomplete" means here, and why the re-drive needs no second query
//!
//! The projector finalizes a ref and writes its subject's delta **warm in one
//! transaction**, so a committed ref always has a warm delta and the state
//! "committed but unwarm" is not reachable. A version is therefore incomplete
//! exactly while it still has a **pending** ref — which is what
//! [`catalog_version_ref_repo::list_pending`] returns. The re-drive is
//! consequently the same read as the first warm, and this pass needs no
//! "committed but incomplete" scan at all. **Reported**, because the plan this
//! group was built from expected two reads and the finalize's position
//! collapses them into one.
//!
//! # The two alarms, and the sink that does not exist
//!
//! `pricing.catalogversion.commit_overdue` (§3.6) and
//! `pricing.readmodel.pin_eligibility_overdue` (§4.4) are the two Critical
//! alarms the design set names **by string**, and both names are taken from it
//! rather than invented. What is invented is nothing: **this gear has no
//! metrics or alarm facility at all** — the sibling ledger has
//! `infra/metrics.rs` and an event publisher with an alarm catalogue, and this
//! crate has neither — so a `tracing::error!` under the named string is the
//! whole of it. **Reported as a gap**, not stubbed behind a trait with no
//! implementation.
//!
//! # The degraded mark has no home, and none is invented
//!
//! §4.4 says completion "clears the degraded mark", and **no table in §3.7 has
//! a column for one**: `pricing_read_model` has none, and `pricing_operator_flag`
//! is forbidden from carrying version state by D-85. So the mark is **derived**
//! — a publish whose ref is still pending *is* the degraded state, and
//! completing it clears the state by making the predicate false. That is
//! precisely §4.4's own "no new event name is introduced; consumers observe
//! completion via the marker". No column, no flag. **Reported as a gap.**
//!
//! # The degraded threshold is the batching SLO, because the 5s one is not
//! measurable here
//!
//! §1.2 marks a publish degraded when post-commit warming misses the **5s**
//! SLO. In this gear the warm cannot begin until the registry commits the
//! batch, and D-47 budgets that at p95 <= 60s and **max 5 minutes** — so
//! measuring 5s from the only instant this store records (`requested_at`, the
//! moment the handle was requested inside the publish transaction) would mark
//! **every** publish degraded, including every one behaving exactly as
//! designed. The instant that would make the 5s rule measurable is when
//! `CatalogVersionPublished` fired, and nothing records it: `committed_at` on
//! the ref row is stamped by *this* pass's finalize, i.e. at the warm, so it
//! cannot bound the warm.
//!
//! The threshold used is therefore `config.jobs.catalog_version_overdue_after()`
//! — the ratified max batching delay, the same one `commit_overdue` uses.
//! **Reported.**

use std::collections::BTreeMap;
use std::sync::Arc;

use bss_pricing_sdk::CatalogVersion;
use chrono::{DateTime, Utc};
use toolkit_db::secure::{AccessScope, DBRunner};
use toolkit_db::{DBProvider, DbError};
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::config::JobsConfig;
use crate::domain::error::DomainError;
use crate::domain::ports::{CatalogVersionRegistryError, CatalogVersionRegistryV1};
use crate::domain::read_model::SubjectKind;
use crate::domain::scope_key::PlanId;
use crate::infra::read_model::ReadModelProjector;
use crate::infra::storage::repo::{
    NewOutboxEvent, PendingVersionRow, PlanPublishDegradedPayload, catalog_version_ref_repo,
    outbox_repo, pin_frontier_repo,
};
use crate::infra::storage::repo_failure;

/// The `pricing_catalog_version_ref` rows one pass will look at.
///
/// Bounded because the sweep runs every few seconds across every tenant of a
/// deployment, and an unbounded backlog read into memory would turn a transient
/// registry outage into a memory problem.
///
/// **Two costs of the bound, both real, neither fixed here.**
///
/// The read is **cross-tenant and oldest-first**, so it is FIFO by request
/// instant across the whole deployment: a tenant holding the oldest 500 refs
/// delays every other tenant's publishes until its backlog drains. That is fair
/// and it is slow, and it is a starvation an earlier version of this doc denied
/// ("nothing is starved by a steady arrival rate") on the strength of the
/// ordering alone — which says only that nothing is starved *within* a tenant.
///
/// A version whose refs **straddle the page boundary** is split across passes,
/// so the first pass sees a partial subject set and can declare the version
/// complete. That one is a correctness risk rather than a latency one, and it
/// is not left to a premise: `refuse_projection_below_frontier` in
/// [`crate::infra::read_model`] refuses the straggler loudly when it arrives at
/// a version the frontier has already passed.
///
/// Both are **reported and deferred**. Fixing the first needs a per-tenant
/// fairness policy (round-robin, or a per-tenant cap) that no document in the
/// set states; fixing the second needs the page to be closed under version,
/// which cannot be expressed while a pending ref carries no version. Neither
/// belongs in a guess made here.
const PENDING_SCAN_LIMIT: u64 = 500;

/// The `pricing_pin_frontier` rows one pass reads for the pin-eligibility
/// alarm. One row per tenant that has ever completed a version, so this bounds
/// an O(active tenants) read rather than an O(publishes) one.
const FRONTIER_SCAN_LIMIT: u64 = 1_000;

/// The Critical alarm §3.6 names for a ref that stays `pending` past the max
/// batching-delay SLO. The string is the design set's, not this module's.
const ALARM_COMMIT_OVERDUE: &str = "pricing.catalogversion.commit_overdue";

/// The Critical alarm §4.4 names for a version that has not become pin-eligible
/// within the same SLO. Likewise the design set's string.
const ALARM_PIN_ELIGIBILITY_OVERDUE: &str = "pricing.readmodel.pin_eligibility_overdue";

/// What one pass did. Returned rather than only logged so the suite can assert
/// the pass's behaviour instead of scraping its output.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SweepReport {
    /// The pass returned early because no registry is wired.
    pub inert: bool,
    /// Pending refs read this pass.
    pub pending_seen: u64,
    /// Distinct `(tenant, version)` pairs handed to the projector.
    pub versions_projected: u64,
    /// Subjects the projector actually wrote a delta for.
    pub subjects_projected: u64,
    /// Subjects whose own transaction refused. Their refs stay pending.
    pub subjects_failed: u64,
    /// Frontier advances this pass produced.
    pub frontiers_advanced: u64,
    /// `PlanPublishDegraded` rows enqueued.
    pub degraded_emitted: u64,
    /// Refs whose age tripped `pricing.catalogversion.commit_overdue`.
    pub commit_overdue: u64,
    /// Tenants whose blocked frontier tripped
    /// `pricing.readmodel.pin_eligibility_overdue`.
    pub pin_eligibility_overdue: u64,
}

/// The read-model warm re-drive: one pass, cross-tenant.
pub struct ReadModelWarmJob {
    db: DBProvider<DbError>,
    projector: ReadModelProjector,
    registry: Arc<dyn CatalogVersionRegistryV1>,
    jobs: JobsConfig,
}

impl ReadModelWarmJob {
    /// Build the job over one database provider, the resolved registry (the
    /// fail-closed default when none is wired) and the validated cadences.
    #[must_use]
    pub fn new(
        db: DBProvider<DbError>,
        registry: Arc<dyn CatalogVersionRegistryV1>,
        jobs: JobsConfig,
    ) -> Self {
        let projector = ReadModelProjector::new(db.clone());
        Self {
            db,
            projector,
            registry,
            jobs,
        }
    }

    /// Run one pass.
    ///
    /// Cross-tenant under [`AccessScope::allow_all`] with the system actor
    /// [`SecurityContext::anonymous`], narrowing to `AccessScope::for_tenant`
    /// before every per-tenant write — the sanctioned pattern the sibling
    /// ledger's jobs document.
    ///
    /// A per-version projection fault is **isolated**: it is logged and the
    /// pass continues, because one tenant's unprojectable plan must not stop
    /// every other tenant's publishes from becoming pinnable. The ref stays
    /// pending, so the next tick re-drives it and its age eventually trips
    /// `commit_overdue`.
    ///
    /// # Errors
    /// [`DomainError::Internal`] only when the pass cannot start — the pending
    /// read itself failing. Everything after that is isolated within the pass.
    pub async fn run(&self, now: DateTime<Utc>) -> Result<SweepReport, DomainError> {
        let conn = self
            .db
            .conn()
            .map_err(|e| DomainError::Internal(format!("bss-pricing: warm sweep: {e}")))?;
        let pending = catalog_version_ref_repo::list_pending(
            &conn,
            &AccessScope::allow_all(),
            PENDING_SCAN_LIMIT,
        )
        .await
        .map_err(|e| repo_failure(&e))?;

        let mut report = SweepReport {
            pending_seen: as_count(pending.len()),
            ..SweepReport::default()
        };
        if pending.is_empty() {
            return Ok(report);
        }
        let Some(resolved) = self.resolve(&pending, &mut report).await else {
            return Ok(report);
        };
        self.project_all(resolved, now, &mut report).await;

        // Re-read, because the predicate sec 3.6 states is "**still** pending
        // past the max batching delay" and the pass has just resolved some of
        // them. Evaluating it on the answer the registry gave, as an earlier
        // version did, left two whole classes silent: a registry that
        // **errors**, and a ref the registry **has** committed whose projection
        // keeps failing - which is exactly sec 4.4's "a stuck version now holds
        // the frontier, which is exactly what that alarm signals". Evaluating it
        // on the pre-pass snapshot instead would alarm for refs this very pass
        // fixed. One bounded query answers the predicate as written.
        let still_pending = catalog_version_ref_repo::list_pending(
            &conn,
            &AccessScope::allow_all(),
            PENDING_SCAN_LIMIT,
        )
        .await
        .map_err(|e| repo_failure(&e))?;
        for row in &still_pending {
            self.observe_overdue(row, now, &mut report).await;
        }
        self.observe_pin_eligibility(&conn, &still_pending, now, &mut report)
            .await;
        Ok(report)
    }

    /// Raise `pricing.readmodel.pin_eligibility_overdue` on the **frontier's
    /// own age**.
    ///
    /// D-136 and [`PinFrontier::advanced_at`](bss_pricing_sdk::PinFrontier)'s
    /// doc both name `advanced_at` as the referent: it is what the <= 5s pin-lag
    /// rule is measured against and what this alarm fires on. An earlier version
    /// measured a pending ref's age instead, which is `commit_overdue`'s
    /// referent and a different fact — a stuck frontier with nothing pending
    /// went unreported entirely.
    ///
    /// Two conditions, both necessary. **Stale**: the frontier has not moved
    /// within the SLO — a tenant with no frontier row at all counts as stale by
    /// construction, having never advanced. **Blocked**: something the tenant
    /// published is short of pin-eligibility, which is either a committed
    /// version standing above the frontier *or* a ref that has itself been
    /// pending past the SLO. The second arm is not redundant: a version
    /// **every** subject of which fails to project is never committed in
    /// storage at all, because the finalize and the projection share a
    /// transaction — so on the exact path §4.4 calls "a stuck version holds the
    /// frontier", the committed-version arm sees nothing.
    ///
    /// **The age condition on that second arm is what keeps this alarm off
    /// healthy publishes**, and it is the whole of the difference between a
    /// Critical that means something and one that fires twelve times per normal
    /// publish. Staleness alone does not discriminate: a tenant that has simply
    /// not published for five minutes has a stale frontier, and a tenant with
    /// no frontier row is stale by construction — so conjoining staleness with
    /// "has a pending ref" says nothing more than "has a pending ref", which is
    /// true of every publish for the whole of D-47's budget. The ref must be
    /// **overdue**, which is the same threshold `commit_overdue` uses and the
    /// same one `01-foundation.md` §4.4 names ("has not become pin-eligible
    /// within the max batching-delay SLO").
    ///
    /// The tenant set is every tenant with a frontier row, plus the tenants of
    /// this pass's still-pending refs. The first half is what finds a tenant
    /// whose frontier is stale precisely because nothing of it has moved.
    async fn observe_pin_eligibility(
        &self,
        conn: &impl DBRunner,
        pending: &[PendingVersionRow],
        now: DateTime<Utc>,
        report: &mut SweepReport,
    ) {
        let Ok(threshold) = chrono::Duration::from_std(self.jobs.catalog_version_overdue_after())
        else {
            return;
        };
        let mut tenants: BTreeMap<Uuid, Option<DateTime<Utc>>> = BTreeMap::new();
        if let Ok(frontiers) =
            pin_frontier_repo::list_all(conn, &AccessScope::allow_all(), FRONTIER_SCAN_LIMIT).await
        {
            for (tenant_id, frontier) in frontiers {
                tenants.insert(tenant_id, Some(frontier.advanced_at));
            }
        }
        for row in pending {
            tenants.entry(row.tenant_id).or_insert(None);
        }

        for (tenant_id, advanced_at) in tenants {
            let stale = advanced_at.is_none_or(|at| now.signed_duration_since(at) >= threshold);
            if !stale {
                continue;
            }
            // The ref must itself be **past the SLO**, not merely exist.
            // "This tenant has something pending" is true of every healthy
            // publish for the whole of D-47's batching budget, and conjoined
            // with a staleness test that a tenant simply not publishing for
            // five minutes satisfies, it degenerates into "this tenant has a
            // pending ref" - roughly twelve Critical alarms per normal publish
            // at a five-second tick.
            let waiting = pending.iter().any(|row| {
                row.tenant_id == tenant_id
                    && now.signed_duration_since(row.requested_at) >= threshold
            });
            if !waiting && !self.frontier_is_blocked(conn, tenant_id).await {
                continue;
            }
            tracing::error!(
                alarm = ALARM_PIN_ELIGIBILITY_OVERDUE,
                tenant_id = %tenant_id,
                advanced_at = ?advanced_at,
                "bss-pricing: a committed catalog version has stood short of pin-eligibility \
                 past the max batching delay; consumers keep pinning the previous edge"
            );
            report.pin_eligibility_overdue += 1;
        }
    }

    /// Ask the registry what version each pending handle landed in.
    ///
    /// `None` is the **inert** answer — no registry is wired — and it is
    /// returned rather than reported as an error for the reason the module doc
    /// gives: an unconfigured registry is a deployment state, not a catalog
    /// defect, and a pass alarming about it every five seconds would drown the
    /// boot that depends on it.
    ///
    /// Grouping into a `BTreeMap` keyed `(tenant, version)` is what makes a
    /// tenant's versions arrive in **ascending** order at the projector, so the
    /// frontier's next-version-in-order check never sees a gap this very pass
    /// is about to fill.
    async fn resolve(
        &self,
        pending: &[PendingVersionRow],
        report: &mut SweepReport,
    ) -> Option<BTreeMap<(Uuid, CatalogVersion), Vec<PendingVersionRow>>> {
        let ctx = SecurityContext::anonymous();
        let mut resolved: BTreeMap<(Uuid, CatalogVersion), Vec<PendingVersionRow>> =
            BTreeMap::new();
        for row in pending {
            match self
                .registry
                .committed_version(&ctx, &row.pending_ref)
                .await
            {
                Err(CatalogVersionRegistryError::Unconfigured) => {
                    tracing::debug!(
                        "bss-pricing: read-model warm sweep skipped (no CatalogVersion registry \
                         configured)"
                    );
                    report.inert = true;
                    return None;
                }
                Err(e) => {
                    // A configured registry that cannot answer is a transient
                    // outage, not a catalog defect: the ref stays pending and
                    // its age is what eventually alarms.
                    tracing::warn!(
                        error = %e,
                        pending_ref = %row.pending_ref,
                        "bss-pricing: registry could not resolve a pending version ref"
                    );
                }
                // Not committed yet. Not an error and not an alarm here - the
                // registry batches, and the wait is budgeted. Its age is
                // observed by the pass, uniformly with every other answer.
                Ok(None) => {}
                Ok(Some(version)) => resolved
                    .entry((row.tenant_id, version))
                    .or_default()
                    .push(row.clone()),
            }
        }
        Some(resolved)
    }

    /// Drive the projector over every resolved version, isolating faults.
    ///
    /// A per-version failure is logged and the pass continues: one tenant's
    /// unprojectable plan must not stop every other tenant's publishes from
    /// becoming pinnable. The ref stays pending, so the next tick re-drives it
    /// and its age eventually trips `commit_overdue`.
    async fn project_all(
        &self,
        resolved: BTreeMap<(Uuid, CatalogVersion), Vec<PendingVersionRow>>,
        now: DateTime<Utc>,
        report: &mut SweepReport,
    ) {
        for ((tenant_id, version), rows) in resolved {
            let scope = AccessScope::for_tenant(tenant_id);
            match self
                .projector
                .project_version(&scope, tenant_id, version, &rows, now)
                .await
            {
                Ok(outcome) => {
                    report.versions_projected += 1;
                    report.subjects_projected += as_count(outcome.projected);
                    report.subjects_failed += as_count(outcome.failed);
                    if outcome.frontier_advanced_to.is_some() {
                        report.frontiers_advanced += 1;
                    }
                }
                Err(e) => tracing::error!(
                    error = %e,
                    tenant_id = %tenant_id,
                    catalog_version = version.get(),
                    "bss-pricing: projecting a committed catalog version failed; its refs stay \
                     pending and the next tick re-drives them"
                ),
            }
        }
    }

    /// A ref that was still waiting when this pass read it: alarm on its age,
    /// and mark its publish degraded once.
    ///
    /// Evaluated for **every** pending ref of the pass, whatever the registry
    /// answered — see [`ReadModelWarmJob::run`] for the two classes an
    /// answer-conditioned version left silent.
    async fn observe_overdue(
        &self,
        row: &PendingVersionRow,
        now: DateTime<Utc>,
        report: &mut SweepReport,
    ) {
        let waited = now.signed_duration_since(row.requested_at);
        let Ok(threshold) = chrono::Duration::from_std(self.jobs.catalog_version_overdue_after())
        else {
            return;
        };
        if waited < threshold {
            return;
        }

        tracing::error!(
            alarm = ALARM_COMMIT_OVERDUE,
            tenant_id = %row.tenant_id,
            pending_ref = %row.pending_ref,
            subject_kind = %row.subject_kind,
            subject_ref = %row.subject_ref,
            waited_secs = waited.num_seconds(),
            "bss-pricing: a catalog version request has stood pending past the max batching \
             delay; remediation is a registry re-request, never a silent re-emit"
        );
        report.commit_overdue += 1;

        if self.mark_degraded(row, now).await {
            report.degraded_emitted += 1;
        }
    }

    /// Does a **committed** version stand above this tenant's frontier?
    ///
    /// One of the two arms of "blocked"; see
    /// [`ReadModelWarmJob::observe_pin_eligibility`] for why it is not enough on
    /// its own.
    ///
    /// Read under the tenant's own scope, not the system one: this is a
    /// per-tenant read on behalf of a per-tenant alarm, and narrowing here is
    /// the same discipline the writes below follow. A storage failure answers
    /// `false` — an alarm that cannot read must not become a second fault, and
    /// the pass has already alarmed on the ref itself.
    async fn frontier_is_blocked(&self, conn: &impl DBRunner, tenant_id: Uuid) -> bool {
        let scope = AccessScope::for_tenant(tenant_id);
        let Ok(standing) = pin_frontier_repo::read_at(conn, &scope, tenant_id).await else {
            return false;
        };
        let standing = standing.map(|frontier| frontier.catalog_version);
        matches!(
            catalog_version_ref_repo::next_committed_version_after(
                conn, &scope, tenant_id, standing
            )
            .await,
            Ok(Some(_))
        )
    }

    /// Enqueue one `PlanPublishDegraded` for a plan publish still waiting.
    ///
    /// **No check-then-enqueue.** `uq_pricing_outbox_dedup_key` is what makes a
    /// repeat of one degradation one event, and a pre-check would be a read the
    /// insert races with anyway — the posture `outbox_repo::enqueue`'s own doc
    /// takes for both refusals that index makes. A repeat therefore arrives as
    /// a storage refusal and is logged at debug, which is exactly what it is.
    ///
    /// The **correlation id is minted per emission** because a background pass
    /// is its own causal origin: the ref row carries none, the publish that
    /// requested the handle is long committed, and borrowing that publish's
    /// correlation id would attribute a sweep's observation to a request that
    /// did not make it.
    ///
    /// Only `plan` subjects are marked. The other three kinds have no store in
    /// this gear, so no publish unit here can have produced one, and the
    /// projector refuses them by name rather than this pass inventing an event
    /// for a subject that cannot exist.
    async fn mark_degraded(&self, row: &PendingVersionRow, now: DateTime<Utc>) -> bool {
        if row.subject_kind != SubjectKind::Plan {
            return false;
        }
        let Ok(plan_id) = Uuid::parse_str(&row.subject_ref) else {
            return false;
        };
        let payload = PlanPublishDegradedPayload {
            plan_id: PlanId::new(plan_id),
            pending_version_ref: row.pending_ref.clone(),
            requested_at: row.requested_at,
            correlation_id: Uuid::now_v7(),
        };
        let scope = AccessScope::for_tenant(row.tenant_id);
        let tenant_id = row.tenant_id;
        let (_, outcome) = self
            .db
            .db()
            .in_transaction::<u64, DomainError, _>(move |txn| {
                Box::pin(async move {
                    outbox_repo::enqueue(
                        txn,
                        &scope,
                        NewOutboxEvent::plan_publish_degraded(tenant_id, &payload, now),
                    )
                    .await
                    .map_err(|e| repo_failure(&e))
                })
            })
            .await;
        match outcome {
            Ok(_) => true,
            Err(e) => {
                // The expected case: the dedup index refusing a repeat of a
                // degradation an earlier pass already reported.
                tracing::debug!(
                    error = ?e,
                    pending_ref = %row.pending_ref,
                    "bss-pricing: PlanPublishDegraded not enqueued (already reported, or the \
                     store refused)"
                );
                false
            }
        }
    }
}

/// A count for the report, saturating rather than casting.
///
/// `usize -> u64` is infallible on every target this builds for, but `as` is
/// banned here and a `TryFrom` on a counter would put a `Result` in a report
/// that has nothing to say about one.
fn as_count(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
#[path = "readmodel_warm_tests.rs"]
mod readmodel_warm_tests;
