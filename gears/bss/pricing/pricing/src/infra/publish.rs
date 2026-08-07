//! The publish engine's service half: assembling the subject the rules judge,
//! §4.2 step 2 — the pre-check — and §4.2 step 4, the commit.
//!
//! The two share an assembler and a rule set on purpose. §4.2 runs the set
//! **twice**, and the second run is not a formality: approval approves
//! *content*, the commit re-validates *state*, and a commit-time failure voids
//! the approval and returns the subject to draft with the report. Two
//! assemblers or two rule sets would make the two runs incomparable, which is
//! the one thing that clause cannot survive.
//!
//! It lives in `infra` rather than in `domain` because it holds an
//! `AccessScope` and repositories, which dylint DE0301 forbids inside
//! `src/domain/**`. The sibling ledger puts `infra::posting::service` here for
//! the same reason. The publish **vocabulary** and the rules stay pure, in
//! [`crate::domain::publish`].
//!
//! # Assembling the subject: the one predicate that matters
//!
//! [`PlanShape`] is what the rules judge, and building it from five tables is
//! the half of §4.2 step 2 nobody had written. The consequential line is
//! **which rows are in it**.
//!
//! `inst-ph-coverage`, `inst-cs-hybrid`, `inst-cs-usage` and
//! `USAGE_MARKET_INCOMPLETE` all range over the plan's *published* rows —
//! meaning the shape **as it will be after this commit**. So the candidate set
//! is the plan's currently-`published` rows **plus** the `draft` rows this unit
//! publishes. Neither half alone is right:
//!
//! - **draft rows alone** would fail every completeness rule on a revision that
//!   changes only a descriptor or a phase duration, since such a revision
//!   authors no price rows at all and the plan would read as having none;
//! - **published rows alone** would let a newly authored market go unjudged,
//!   which is exactly the "sold but unrateable" state D-15/D-17 declare
//!   impossible.
//!
//! `superseded` and `abandoned` rows are **out**. A superseded row has had its
//! key taken by a successor and is history; an `abandoned` state does not occur
//! on a price row at all (it is a plan-revision state — §4.3,
//! `inst-ps-nodelete`), and naming it here would be a second reading of the
//! price row's state set.
//!
//! # The baseline, and why `None` is a correct answer
//!
//! [`PublishedBaseline`] exists for the three rules that are *comparisons*
//! against what is already published — `TerminalPhaseStable` (D-64) and
//! `AvailableFromNotBackdated`. On a **first** publish there is no current
//! revision, the baseline is `None`, and those rules decline to judge. That is
//! the `inst-cs-declared` posture (D-149): a rule with no referent has only two
//! behaviours available and both are wrong — pass vacuously, which is the guard
//! silently absent, or treat every value as changed, which blocks every first
//! publish outright.
//!
//! # Two assembly decisions the design set leaves implicit
//!
//! Both are decided here, argued here, and **reported** as divergences:
//!
//! 1. The candidate row set above. No section states it; four instructions
//!    presuppose it.
//! 2. `phase_ids_in_use` on the baseline is taken from the plan's currently
//!    **published** price rows, not from the candidate set. `PHASE_IN_USE`
//!    (D-64) refuses a revision that drops a phase "still referenced by a
//!    current published price row", so the referent is the published plane
//!    alone — reading it off the candidate set would let a draft row that this
//!    same revision authored keep a phase alive that nothing published uses.
//!
//! # The pre-check mutates nothing
//!
//! That is what makes it a pre-check, and it is what `POST …/plans/{id}/publish`
//! would call before routing to approval — **an endpoint that does not exist**,
//! because it cannot construct a `PublishAuthorization` without Slice 5's
//! approval record (see [`PublishService::commit`]). A non-empty report comes back as a
//! **report**; turning it into `DomainError::ValidationFailed` is the surface's
//! choice, because the submit path wants to *show* a report while the commit
//! path wants to *fail* on one.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use toolkit_db::secure::{AccessScope, DBRunner};
use toolkit_db::{DBProvider, DbError};
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::domain::audit::{AuditAction, AuditSubjectKind, subject_state};
use crate::domain::concurrency::RowVersion;
use crate::domain::error::DomainError;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::plan::PlanRevision;
use crate::domain::plan_shape::{PhaseGraph, PlanShape, PublishedBaseline};
use crate::domain::ports::{CatalogVersionRegistryV1, registry_failure};
use crate::domain::price_record::PriceRecord;
use crate::domain::publish::rules::{PublishRuleParams, SoftSizeCaps, run_publish_rules};
use crate::domain::publish::{PlanPublishUnit, PublishAuthorization, PublishReceipt};
use crate::domain::read_model::SubjectRef;
use crate::domain::scope_key::{PhaseId, PlanId};
use crate::domain::tax_display::{RegionReadiness, RegionTaxReadiness};
use crate::domain::validation::ValidationReport;
use crate::domain::window::{self, KeyWindows, WindowInterval};
use crate::infra::fixture_gate::{FixtureGate, Reservation};
use crate::infra::storage::repo::{
    NewAuditEntry, NewOutboxEvent, PendingVersionRow, PlanPublishedPayload, PolicyObjectRepo,
    approval_repo, audit_repo, catalog_version_ref_repo, outbox_repo, plan_repo, plan_shape_repo,
    price_repo, taxonomy_repo, window_repo,
};
use crate::infra::storage::repo_failure;

/// The lifecycle states a publish subject's candidate row set is drawn from.
///
/// Written down once, here, because it is the single most consequential line in
/// the assembler — see the module doc. `superseded` and `abandoned` are absent
/// deliberately and their absence is the rule.
///
/// `pub(crate)` for one reader: `api::rest::windows`' coverage report ranges over
/// the same set, because a remediation surface that answered about a different row
/// set from the check whose failure sent the operator there would tell them to fix
/// something else.
pub(crate) const CANDIDATE_ROW_STATES: &[LifecycleState] =
    &[LifecycleState::Published, LifecycleState::Draft];

/// The publish engine, as the surfaces and the commit path see it.
///
/// Holds the repositories and the joint-conformance gate, built from what
/// [`PricingRuntime`](crate::module) already carries. The `DBProvider` is here
/// because the pre-check reads through a plain connection while the commit
/// reads through its own transaction, and both go through the same assembler.
#[derive(Clone)]
pub struct PublishService {
    db: DBProvider<DbError>,
    policies: PolicyObjectRepo,
    fixture_gate: FixtureGate,
    /// The sole incrementer of `CatalogVersion`, resolved at init with the
    /// fail-closed default.
    ///
    /// The engine is the only place a version is **requested** from, and that
    /// is the invariant this field carries: two requesters would mean two
    /// publishes believing they own one assignment. It is deliberately narrower
    /// than "one holder", which is what this doc used to say — the read-model
    /// warm sweep holds the same `Arc` and calls only `committed_version`,
    /// asking what a handle **this** engine already obtained resolved to. One
    /// requester, two readers.
    registry: Arc<dyn CatalogVersionRegistryV1>,
    /// What this path reports about itself (`T-17`); a no-op unless the
    /// gear lifecycle attached one.
    metrics: Arc<dyn crate::domain::ports::metrics::PricingMetricsPort>,
}

impl PublishService {
    /// Build the engine over one database provider and the loaded gate.
    #[must_use]
    pub fn new(
        db: DBProvider<DbError>,
        limits: &crate::config::LimitsConfig,
        gate: FixtureGate,
        registry: Arc<dyn CatalogVersionRegistryV1>,
    ) -> Self {
        let policies = PolicyObjectRepo::new(db.clone(), limits);
        Self {
            db,
            policies,
            fixture_gate: gate,
            registry,
            // The safe default: a service built without one reports nothing
            // rather than failing to build. `with_metrics` is what production
            // hands it.
            metrics: Arc::new(crate::domain::ports::metrics::NoopPricingMetrics),
        }
    }

    /// Attach the metrics port (`T-17`).
    ///
    /// A **second call** rather than a fifth parameter on [`Self::new`], for
    /// `PublishRuleParams::with_referencing_markets`' reason: every existing
    /// caller has nothing to say to it, and the no-op [`Self::new`] already
    /// installs is exactly what they mean.
    #[must_use]
    pub fn with_metrics(
        mut self,
        metrics: Arc<dyn crate::domain::ports::metrics::PricingMetricsPort>,
    ) -> Self {
        self.metrics = metrics;
        self
    }

    /// §4.2 step 2 — validate a plan's open draft revision without touching it.
    ///
    /// Resolves the tenant's authoring policy, assembles the publish subject,
    /// runs the aggregate rule set, then asks the joint-conformance gate. It
    /// **mutates nothing**.
    ///
    /// The gate is asked **after** the rules and its refusal supersedes the
    /// report, which is a decision worth stating: a `FIXTURE_MISSING` is not an
    /// item on the author's remediation list at all — no edit to the plan
    /// changes the gate's answer, because the gate is about what this
    /// deployment's joint corpus has agreed to evaluate. So when both hold, the
    /// caller is told the one it can act on, and acting on it means waiting for
    /// a fixture rather than editing a row.
    ///
    /// # Errors
    /// [`DomainError::NotFound`] when the plan has no open draft revision —
    /// there is nothing to pre-check, and answering with an empty report would
    /// say a plan that cannot publish is publishable.
    /// [`DomainError::FixtureMissing`] when a candidate row's shape has no green
    /// joint conformance fixture.
    /// [`DomainError::Internal`] on a storage failure, or when the plan's
    /// current revision holds no unique terminal phase.
    pub async fn precheck(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        plan_id: PlanId,
        now: DateTime<Utc>,
    ) -> Result<ValidationReport, DomainError> {
        let conn = self
            .db
            .conn()
            .map_err(|e| DomainError::Internal(format!("bss-pricing: publish precheck: {e}")))?;
        // The shape is assembled **first**, because `inst-cb-addon`'s domain is
        // the add-on set this revision composes — the parameters are now a
        // function of the subject, not only of the tenant.
        let shape = assemble(&conn, scope, tenant_id, plan_id, now).await?;
        let params = rule_params(&self.policies, &conn, scope, tenant_id, &shape).await?;
        let report = run_publish_rules(&shape, &params);
        crate::infra::metrics::report_market_metrics(&*self.metrics, &shape, &params);
        check_fixtures(&self.fixture_gate, &shape)?;
        Ok(report)
    }

    /// §4.2 step 4 — the publish commit. One transaction, or none of it.
    ///
    /// The signature is the contract: no publish without a unit, a version to
    /// swap on, an authorization, and an actor for the trail.
    ///
    /// # The sequence, and why each move sits where it does
    ///
    /// 1. **Re-assemble the subject and re-run the rules and the gate, inside
    ///    the transaction.** This is the clause the whole design turns on.
    ///    Approval approved *content*; the commit re-validates *state*, because
    ///    the world moved between the two — a referenced phase retired, a
    ///    market's rows changed, a cap lowered. A non-empty report returns
    ///    [`DomainError::ValidationFailed`] and the transaction rolls back, so
    ///    the subject is exactly where it was: still `draft`, no ref, no event,
    ///    no audit row. §4.2's "voids the approval and returns the subject to
    ///    draft" has two halves, and only one of them is here: the return to
    ///    draft **is** the rollback, since the subject never left `draft`, while
    ///    voiding the approval **record** is Slice 5's and is owed with the
    ///    record store this gear does not have.
    /// 2. **Request the `CatalogVersion`, fail closed.** After the
    ///    re-validation passes and before the writes. Earlier, a commit-time
    ///    validation failure would leave a pending ref at the registry that
    ///    nothing ever commits, tripping
    ///    `pricing.catalogversion.commit_overdue` for a publish that never
    ///    happened; later, the outbox payload would have nothing to carry.
    ///    The cost is real and named: a network round-trip inside an open
    ///    database transaction. It is bounded by the registry's own idempotency
    ///    — `request_id` is documented as exactly that — so
    ///    [`publish_request_id`] derives it from `(tenant, plan, revision)` and
    ///    a rolled-back-then-retried commit re-requests the **same** handle
    ///    instead of orphaning one.
    /// 3. **Flip the row set** — the revision, then its price rows.
    /// 4. **Record the pending ref**, with the subject, the revision **and the
    ///    lifecycle state** this commit judged — the three facts the projector
    ///    needs and cannot re-derive later without reading a world that moved.
    /// 5. **Enqueue `PlanPublished`**, carrying the pending handle.
    /// 6. **Extend the audit chain.** Last among the writes, because the record
    ///    describes what the transaction did and every fact it hashes is
    ///    settled by then; and inside the transaction because D-14 says a record
    ///    that could commit separately is a record that can be lost on a crash.
    ///
    /// # CONTRACT — isolation, and the two invariants that are not the same
    ///
    /// This transaction is opened with the **engine default**, which on Postgres
    /// is READ COMMITTED. Two different invariants stand behind that, and an
    /// earlier version of this paragraph conflated them; the conflation is what
    /// hid a live defect, so both are spelled out.
    ///
    /// **The counter invariants need no SSI, and that argument is sound.** The
    /// audit chain's primary key `(tenant_id, chain_id, seq)` makes a fork
    /// unrepresentable at any isolation level; `uq_pricing_outbox_sequence` does
    /// the same for the event counter and `uq_pricing_plan_current` for the
    /// current revision. Two writers that read one head cannot both insert after
    /// it — the loser takes a unique violation and its whole transaction rolls
    /// back. The sibling ledger's posting path opens `SERIALIZABLE` because its
    /// chain tip lives in a separate row read locklessly, a premise this design
    /// does not have.
    ///
    /// **The predicate invariant is a different thing, and no key covers it.**
    /// The commit also depends on "every row that publishes was judged by the
    /// rule set", which ranges over rows a concurrent writer can *insert* and
    /// *mutate*. A key can only refuse a second row at a position it already
    /// knows about; it says nothing about a row that did not exist when the
    /// subject was assembled. Under READ COMMITTED every statement takes a fresh
    /// snapshot, so the window between the assembly at step 1 and the flips at
    /// step 3 — which contains the registry's network round-trip — is wide, and
    /// a fork argument cannot close it. **A conclusion about predicates does not
    /// follow from a premise about keys.**
    ///
    /// **It is closed by the row-version guard, not by isolation.** Step 3
    /// publishes exactly the `(price_id, row_version)` set the rule set judged,
    /// so a row whose content moved is refused naming the row and a row that did
    /// not exist is simply not in the set. That is what D-141 gave every price
    /// row its own entity tag for.
    ///
    /// **The rest of the subject, input by input, with the mechanism that holds
    /// each.** The enumeration is written out because an earlier version of it
    /// said "those are all the inputs" over a list that was not complete, and an
    /// unchecked "all" is the same shape of claim that hid the defect above.
    ///
    /// | Input | Read by | Held by |
    /// |---|---|---|
    /// | The **draft** revision row | [`assemble`] | its own compare-and-swap in `publish_revision` |
    /// | Its Slice-2 children — phases, add-on rules, descriptor set | [`assemble`] | that same revision tag; `PlanShapeRepo` bumps it on every child edit |
    /// | The plan's **draft** price rows | [`assemble`] | the per-row `row_version` guard above (D-141) |
    /// | The plan's **published** price rows — *content* | [`assemble`] | `pricing_price`'s append-only trigger: a published row's price, scope, model and tag columns are immutable |
    /// | The **current published** revision row (`available_from`/`available_to`) | [`published_baseline`] | `pricing_plan`'s frozen-column whitelist, which fires on any UPDATE of a non-draft row |
    /// | That revision's **phase set** (the terminal phase id) | [`terminal_phase`] | `pricing_plan_phase`'s append-only trigger, which refuses DML under a parent revision that is not `draft` |
    /// | The tenant's policy row | [`rule_params`] | nothing — see the isolation paragraph below |
    ///
    /// **One premise in that table is not a mechanism, and it is stated as a
    /// premise.** The append-only trigger on `pricing_price` guarantees a
    /// published row's *content*; it does **not** guarantee the *membership* of
    /// the published set, because the same trigger sanctions
    /// `published -> superseded`. Four completeness instructions range over that
    /// set (`inst-ph-coverage`, `inst-cs-hybrid`, `inst-cs-usage`,
    /// `USAGE_MARKET_INCOMPLETE`) and `PublishedBaseline::phase_ids_in_use` is
    /// derived from it, so a row leaving it between assembly and commit would
    /// change what the rule set judged. What holds it today is that **this gear
    /// has no producer for that flip at all**: its two sanctioned producers are
    /// the D-88 supersession unit and the D-100 grandfathering cutover, both
    /// Slice 7's, and neither exists.
    ///
    /// **The premise has narrowed twice, and it is now one step from gone.**
    /// This paragraph first closed with "the `PriceWindow` store is unbuilt",
    /// offered as why the two units could not exist; the store and the window state
    /// machine arrived on 2026-08-04 without either unit being built on them.
    ///
    /// As of **2026-08-05 the flip has a producer in this crate**:
    /// [`price_repo::commit_supersession_rows`] flips a predecessor
    /// `published -> superseded` and publishes the successor beside it (D-88's row
    /// half, ordered per D-195). What holds the premise today is therefore no longer
    /// "there is no producer" but the strictly weaker **"no surface reaches it"** —
    /// the unit around those two functions does not exist, so nothing a client can
    /// call flips a published row out of this candidate set. The premise is stated at
    /// its real strength rather than at the comfortable one.
    ///
    /// # What the next group owes, decided rather than left open (D-195)
    ///
    /// The route that reaches the supersession unit deletes the premise, and the
    /// published half of the candidate set then needs its own pin. **The decision is
    /// taken: extend the judged set.** Published rows carry `row_version` frozen with
    /// their content (D-141), so the same `(price_id, row_version)` shape reaches
    /// them, and the commit verifies before the flips that every published row the
    /// rule set judged is *still* `published` at *that* version. It is a membership
    /// check wearing a version check's clothes, which is exactly what a frozen tag
    /// makes it.
    ///
    /// **Departures are the hazard and arrivals are covered — by an argument, so it
    /// is written down.** A pin over the judged rows catches a published row
    /// *leaving* the set. It cannot catch one *arriving*, and an arrival matters:
    /// the market-uniformity rules (`TAX_BASIS_MIXED_MARKET`,
    /// `PRORATION_CONTRACT_MIXED_MARKET`) read content across the whole set, and the
    /// D-82 unit guard preserves the counter fields while leaving `tax_inclusive`
    /// and the proration contract free to move. Arrivals are nevertheless covered:
    ///
    /// - A **supersession** arrival is paired with a departure on the same key, by
    ///   `inst-su-commit`'s ordering — the flip precedes the publish, in one
    ///   transaction — so the pin fires on the departure.
    /// - A **cutover** (D-100) arrival on the `all_subscriptions` key is likewise
    ///   paired with that key's flip (`inst-co-supersede`).
    /// - The cutover's **grandfathered copy** is the one unpaired arrival: it lands
    ///   on a new generation key with nothing departing. It is covered by D-132 —
    ///   the market-uniformity rules exclude immutable grandfathered generations —
    ///   so the rules that could care do not read it.
    ///
    /// If a third producer of a published row is ever added that is *not* paired
    /// with a departure and *not* a grandfathered generation, this argument is what
    /// it has to be checked against.
    ///
    /// **`SERIALIZABLE` is still not the answer**, for the reason below: it would
    /// hold predicate locks across the registry round-trip, which is the one cost
    /// step 2's ordering exists to bound.
    ///
    /// **`SERIALIZABLE` is therefore not raised, and that is a judgement rather
    /// than an impossibility.** `Db::transaction_ref_mapped_with_config` takes no
    /// `extract_db_err` and is the platform's building block for exactly this
    /// case — a service entrypoint returning domain errors that needs non-default
    /// isolation — so the option is available and is declined on its merits: SSI
    /// would hold predicate locks across the registry's network round-trip, which
    /// is the one cost the ordering decision at step 2 was chosen to bound, and
    /// it would buy nothing the tags and the two triggers above do not already
    /// buy. The one input covered by neither is the tenant's policy row, read at
    /// the start of the transaction; a policy edit committing afterwards means
    /// the publish was judged against the snapshot it read rather than the newest
    /// one, which is staleness and not a bypass — the next publish enforces the
    /// new cap — and this gear has no writer for that row at all.
    ///
    /// **Contention is the remainder, and it is a retry question.**
    /// `transaction_with_retry` is the platform's mechanism for that and is
    /// **not usable here**: it classifies retryability by extracting a
    /// `sea_orm::DbErr` out of the error type, and [`DomainError`] is a
    /// `Clone + Eq` value type carried into responses that cannot hold one, so
    /// every failure would classify as non-retryable and the loop would be
    /// decoration. The retry decision therefore sits with the caller — the
    /// publish endpoint and its client.
    ///
    /// **Half of that debt is discharged and half is not.** The wire code
    /// arrived: `CONCURRENT_MUTATION` (D-159, 409, naming the aggregate) is
    /// declared for exactly this class, over the gear's three per-aggregate
    /// serialization points. What is still missing is any *path* that raises it
    /// — this method flattens a lost race into `RepoError::Db`, i.e. a 500 — and
    /// the mapping is not added here because the refusal has to be recognized
    /// from a backend-specific constraint violation, which is the coupling
    /// `RepoError`'s own doc refuses and which only a layer that knows the
    /// backend can do. So: the **code** is no longer owed, the **recognition**
    /// is, and it belongs with the publish endpoint that would have a client to
    /// tell.
    ///
    /// **One refusal after the registry request is permanent, and it is moved
    /// ahead of it.** Every other post-request refusal self-heals, because the
    /// `request_id` is deterministic and a retry of the same unit is handed the
    /// same pending handle. A retired plan's is not: no retry of that unit can
    /// ever succeed, so the handle would stand pending forever and trip
    /// `pricing.catalogversion.commit_overdue` for a publish that never happened.
    /// [`refuse_unpublishable_predecessor`] asks that question **before** step 2.
    /// What remains orphanable is the race in which a concurrent commit publishes
    /// this very revision between the two runs; §3.6's remediation — "a registry
    /// re-request, never a silent re-emit" — is what covers it, and the handle is
    /// identifiable to its unit because the id is derived rather than random.
    ///
    /// **What this crate's tests can and cannot see.** `sqlite::memory:`
    /// serializes writers, so no test here distinguishes one isolation level from
    /// another. The row-version guard, by contrast, **is** provable here and is
    /// proved — it is a predicate on a statement, not a property of a schedule.
    ///
    /// # Idempotency is deliberately not wired here
    ///
    /// `pricing_idempotency_dedup` and the gate over it exist and take a
    /// transaction handle, so wiring them in would be easy — and wrong for this
    /// path. S2 §5 scopes this endpoint's idempotency "per plan revision", and
    /// the revision already publishes at most once: `LifecycleState` gives
    /// `draft` one edge out and
    /// [`publish_revision`](crate::infra::storage::repo::plan_repo::publish_revision)'s
    /// compare-and-swap refuses the second attempt. The client-key gate answers
    /// a different question — "is this the same HTTP request" — and it needs a
    /// header only a surface receives. It belongs with the endpoint, which is
    /// unbuildable until Slice 5; the machinery it would use exists now
    /// (`infra::idempotent`), so what is owed is the endpoint and not the
    /// plumbing.
    ///
    /// # Errors
    /// [`DomainError::ValidationFailed`] carrying the whole report when the
    /// commit-time run finds anything;
    /// [`DomainError::FixtureMissing`] when a row's shape has no green joint
    /// fixture; [`DomainError::CatalogVersionUnavailable`] when the registry
    /// cannot be reached or has none configured — publish stops rather than
    /// inventing a version; [`DomainError::NotFound`] when the plan has no open
    /// draft revision; [`DomainError::StaleVersion`] when `expected` is not the
    /// revision's current row version;
    /// [`DomainError::PlanRetiredNoSuccessor`] when the plan's current revision
    /// can never be superseded; [`DomainError::LifecycleForbidden`] when the
    /// named revision is frozen; [`DomainError::Internal`] on a storage failure.
    #[allow(
        clippy::too_many_arguments,
        reason = "every parameter is a distinct precondition of a publish - the unit, the \
                  version to swap on, the approval decision, the actor and the correlation \
                  id are what the audit record and the outbox row are made of, and bundling \
                  them into a request struct would hide which of them a caller may omit \
                  (none)"
    )]
    pub async fn commit(
        &self,
        ctx: &SecurityContext,
        scope: &AccessScope,
        tenant_id: Uuid,
        unit: PlanPublishUnit,
        expected: RowVersion,
        authorization: PublishAuthorization,
        actor_principal_id: Uuid,
        correlation_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<PublishReceipt, DomainError> {
        let ctx = ctx.clone();
        let scope = scope.clone();
        let policies = self.policies.clone();
        let gate = self.fixture_gate.clone();
        let registry = Arc::clone(&self.registry);
        let request_id = publish_request_id(tenant_id, unit);

        let (_, outcome) = self
            .db
            .db()
            .in_transaction::<PublishReceipt, DomainError, _>(move |txn| {
                Box::pin(async move {
                    // 1. The second run. Same assembler, same rule set, same
                    // gate - against the world as it now stands.
                    let shape = assemble(txn, &scope, tenant_id, unit.plan_id, now).await?;
                    let params = rule_params(&policies, txn, &scope, tenant_id, &shape).await?;
                    if shape.revision != unit.revision {
                        return Err(DomainError::NotFound {
                            subject: "open plan draft revision".to_owned(),
                            id: format!("{}/{}", unit.plan_id, unit.revision),
                        });
                    }

                    // 1a. The approve→commit window, closed **here** and not at
                    // the surface. `inst-ap-pin` scopes the TOCTOU void to a
                    // `submitted` record, so an approved unit is not voided when
                    // its subject moves, and a check made before this
                    // transaction opened would be a check the world can move
                    // behind. The row-version swap below does not cover it: it
                    // swaps on the *revision's* version, and a price row edited
                    // after the approve moves the row's version and not the
                    // revision's.
                    //
                    // Ahead of the rules, because the answer is about the
                    // reviewer's decision rather than about the plan: telling an
                    // operator to fix a violation in content the second person
                    // never agreed to is the wrong next action.
                    if let Some(pinned) = authorization.pinned_content_hash()
                        && crate::domain::approval::content_hash(&shape) != pinned
                    {
                        return Err(DomainError::ApprovalContentMismatch(format!(
                            "plan {}/{}: the content approved under {} is not the content this \
                             commit would freeze; re-submit and have the change reviewed again",
                            unit.plan_id,
                            unit.revision,
                            authorization
                                .approval_ref()
                                .map_or_else(|| "-".to_owned(), |id| id.to_string()),
                        )));
                    }

                    let report = run_publish_rules(&shape, &params);
                    if !report.is_publishable() {
                        return Err(DomainError::ValidationFailed(report));
                    }
                    let validated = validated_draft_rows(&shape);
                    check_fixtures(&gate, &shape)?;

                    // The one **permanent** refusal that would otherwise land
                    // after the registry request, moved ahead of it: a retired
                    // plan takes no successor, so its handle would stand pending
                    // forever and trip `commit_overdue` for a publish that can
                    // never happen. Asked of the state machine rather than
                    // matched on `Retired`, and it is **not** the guard -
                    // `publish_revision` still is, and answers the same way.
                    refuse_unpublishable_predecessor(txn, &scope, tenant_id, unit.plan_id).await?;

                    // 2. Addressability, fail closed. No locally invented
                    // version, ever: two incrementers make `CatalogVersion`
                    // unordered.
                    let pending = registry
                        .request_version(&ctx, &request_id)
                        .await
                        .map_err(|e| registry_failure(&e))?;

                    // 3. The row set.
                    let published_revision = plan_repo::publish_revision(
                        txn,
                        &scope,
                        tenant_id,
                        unit.plan_id,
                        unit.revision,
                        expected,
                    )
                    .await
                    .map_err(|e| repo_failure(&e))?;
                    // Exactly the rows the rule set just judged, at the
                    // versions it judged them at. See `publish_rows`: a
                    // re-derived set would publish rows validated by nothing.
                    let price_ids = price_repo::publish_rows(
                        txn,
                        &scope,
                        tenant_id,
                        unit.plan_id,
                        &validated,
                        // **The readiness the rule set just judged these
                        // rows with**, not a fresh read: D-154 freezes the
                        // result of the check that passed, so resolving
                        // again — even here — would be a second answer to
                        // one question.
                        params.region_readiness(),
                    )
                    .await
                    .map_err(|e| repo_failure(&e))?;

                    // 4. The ref, carrying the subject G6 will project.
                    catalog_version_ref_repo::record_pending(
                        txn,
                        &scope,
                        PendingVersionRow::for_subject(
                            tenant_id,
                            pending.pending_ref.clone(),
                            &SubjectRef::Plan(unit.plan_id.get()),
                            // The revision this commit's rule set judged. The
                            // projector reads it rather than whatever revision
                            // is current when the sweep arrives, which may be a
                            // later one: the registry batches at up to five
                            // minutes and a second publish inside that window
                            // would otherwise freeze content this publish never
                            // judged into this publish's version.
                            Some(unit.revision),
                            // And the state that flip produced. Read back into
                            // the delta rather than the row's state as it then
                            // stands: that keeps moving (`published ->
                            // superseded` at the next revision's commit), and
                            // `superseded` is a value D-128 does not
                            // contemplate for a projected subject.
                            Some(published_revision.lifecycle_state),
                            now,
                        ),
                    )
                    .await
                    .map_err(|e| repo_failure(&e))?;

                    // 5. The event, carrying the pending handle.
                    let payload = PlanPublishedPayload {
                        plan_id: unit.plan_id,
                        revision: unit.revision,
                        pending_version_ref: pending.pending_ref.clone(),
                        price_ids: price_ids.clone(),
                        correlation_id,
                    };
                    outbox_repo::enqueue(
                        txn,
                        &scope,
                        NewOutboxEvent::plan_published(tenant_id, &payload, now),
                    )
                    .await
                    .map_err(|e| repo_failure(&e))?;

                    // 6. The record of what the transaction did.
                    let seq = audit_repo::append(
                        txn,
                        &scope,
                        NewAuditEntry {
                            tenant_id,
                            chain_id: audit_repo::plan_chain(unit.plan_id),
                            recorded_at: now,
                            actor_principal_id,
                            action: AuditAction::Publish,
                            subject_kind: AuditSubjectKind::PlanRevision,
                            subject_ref: audit_repo::plan_revision_ref(unit.plan_id, unit.revision),
                            before_state: Some(subject_state(
                                LifecycleState::Draft,
                                expected.get(),
                                None,
                            )),
                            after_state: Some(subject_state(
                                published_revision.lifecycle_state,
                                published_revision.row_version.get(),
                                Some(&pending.pending_ref),
                            )),
                            approval_ref: authorization.approval_ref(),
                            correlation_id,
                        },
                    )
                    .await
                    .map_err(|e| repo_failure(&e))?;

                    // 7. The units this publish did **not** consume.
                    //
                    // A unit still `submitted` over the revision this
                    // transaction just froze is an orphan: its subject is now
                    // immutable, so no approve of it can ever lead to a publish,
                    // and re-deriving it answers `APPROVAL_CONTENT_MISMATCH`
                    // forever. Left standing it sits in the reviewer's queue
                    // asking for a decision that cannot mean anything.
                    //
                    // Only `submitted` rows match, so this does **not** touch
                    // the `approved` record that authorized this very commit —
                    // which is the correction this line carries: the argument
                    // for not voiding here used to be that a publish would "undo
                    // its own authorization", and the authorizing unit is not
                    // pending. And it matches the **exact** subject rather than
                    // the plan prefix the TOCTOU guard uses, so a later slice's
                    // unit over another revision, a window or an overlay of the
                    // same plan is not swept up by a publish that had nothing to
                    // do with it.
                    approval_repo::void_pending_for_subject(
                        txn,
                        &scope,
                        tenant_id,
                        &audit_repo::plan_revision_ref(unit.plan_id, unit.revision),
                        crate::infra::approval::ORPHANED_BY_PUBLISH_REASON,
                        now,
                    )
                    .await
                    .map_err(|e| repo_failure(&e))?;

                    Ok(PublishReceipt::new(
                        unit,
                        pending.pending_ref,
                        price_ids,
                        seq,
                    ))
                })
            })
            .await;
        outcome.map_err(|err| {
            err.into_domain(|infra| {
                DomainError::Internal(format!("bss-pricing: publish transaction: {infra}"))
            })
        })
    }
}

/// The `(price_id, row_version)` set the rule set actually judged **and this publish
/// unit is entitled to move**.
///
/// The **draft** half of the candidate row set: the published half is already
/// published and is physically immutable, so nothing about it can move between
/// validation and the flip. What this returns is what
/// [`publish_rows`](crate::infra::storage::repo::price_repo::publish_rows)
/// publishes, at the versions it was validated at, and a row whose content moved
/// in between no longer matches. See that function's doc for the defect this
/// closes.
///
/// # A draft on an occupied key belongs to another unit, and is excluded
///
/// Price rows hang off `plan_id`, not off a revision, so this set is every draft row
/// of the plan. Since D-195 that includes one it must not touch: a **supersession
/// successor**, staged by `price_repo::insert_successor_draft_on` beside the published
/// row it will supersede.
///
/// Sweeping it up is not a near-miss. `publish_rows` would flip it to `published`
/// while its predecessor still reads `published`, the published-plane partial `UNIQUE`
/// would refuse the statement, and the refusal arrives as a raw driver error —
/// `RepoError::Db` → `DomainError::Internal` → **500**, taking an entirely unrelated
/// revision publish down with it for as long as a supersession is pending on any key
/// of the plan. No publish rule catches it first; there is no scope-key-duplication
/// rule in the set (found by review, 2026-08-05, after `publish_rows`' amended doc had
/// named the wrong caller for this collision).
///
/// So the rule is: **a draft row whose canonical scope key already carries a published
/// row is not this unit's to publish.** It publishes through the unit that staged it,
/// which is the one that also flips the predecessor and moves the windows. This is the
/// same reasoning `publish_rows` gives for a *newly inserted* row staying `draft` —
/// nothing validated it as part of this act — one step further: here something did
/// validate it, and it still is not this act's row.
///
/// The exclusion is **decided here rather than in the rule set** because it is about
/// entitlement rather than about validity: the successor's content is perfectly
/// publishable, and a rule reporting a violation would tell an operator to fix a row
/// that is not wrong. `shape.rows` keeps carrying it, so every completeness and
/// uniformity rule still sees the key covered.
///
/// **It is a no-op on every world that predates the second door** — nothing else can
/// put a draft on an occupied key (`inst-pr-return`/D-21 refuses it at save,
/// `IMPORT_TARGETS_PUBLISHED` per row on the bulk plane) — which is why no existing
/// case moves.
fn validated_draft_rows(shape: &PlanShape) -> Vec<(Uuid, RowVersion)> {
    // Compared by canonical **rendering**, the way `infra::window`'s pending-key check
    // does: `ScopeKey` is not `Ord`, and the rendering is the eight axes in a fixed
    // order — the same string a `DUPLICATE_SCOPE_KEY` refusal names, so two equal
    // renderings are one key by construction.
    let occupied: std::collections::BTreeSet<String> = shape
        .rows
        .iter()
        .filter(|record| record.lifecycle_state == LifecycleState::Published)
        .map(|record| record.scope_key.to_string())
        .collect();
    shape
        .rows
        .iter()
        .filter(|record| record.lifecycle_state == LifecycleState::Draft)
        .filter(|record| !occupied.contains(&record.scope_key.to_string()))
        .map(|record| (record.price_id, record.row_version))
        .collect()
}

/// Refuse a publish onto a predecessor that can never be superseded, **before**
/// the registry is asked for a handle.
///
/// It is not a second owner of D-90's rule:
/// [`publish_revision`](crate::infra::storage::repo::plan_repo::publish_revision)
/// is still the authority and answers identically. What this buys is position.
/// Every other refusal reachable after the registry request self-heals — the
/// `request_id` is deterministic, so a retry of the same unit is handed the same
/// pending handle rather than a second one. A retired plan's refusal is
/// **permanent**: no retry of that unit can ever succeed, so the handle stands
/// pending forever and trips `pricing.catalogversion.commit_overdue` for a
/// publish that never happened — which is the outcome the request's position was
/// chosen to avoid in the first place.
///
/// **`pub(crate)` for a second caller**, `infra::supersession`: a plan whose current
/// revision can never be superseded takes no *successor row* either, and the refusal
/// is permanent, so that unit hoists it ahead of its own registry request for exactly
/// the reason this one does.
///
/// # Errors
/// [`DomainError::PlanRetiredNoSuccessor`] when the plan's current revision
/// cannot flip `superseded`; [`DomainError::Internal`] on a storage failure.
pub(crate) async fn refuse_unpublishable_predecessor(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: PlanId,
) -> Result<(), DomainError> {
    let Some(current) = plan_repo::load_current(runner, scope, tenant_id, plan_id)
        .await
        .map_err(|e| repo_failure(&e))?
    else {
        return Ok(());
    };
    if current
        .lifecycle_state
        .can_transition(LifecycleState::Superseded)
    {
        return Ok(());
    }
    Err(DomainError::PlanRetiredNoSuccessor(format!(
        "plan {plan_id} stands at a {} revision, which can never be superseded; \
         it takes no successor",
        current.lifecycle_state
    )))
}

/// Resolve the tenant's authoring policy into the rule set's parameters.
///
/// One read per run, never memoized: a cached cap keeps rejecting plans after an
/// operator has raised it, and the two runs of one publish would be free to
/// disagree about the limit they enforced. The commit re-reads it **inside its
/// transaction**, because the caps are part of the state §4.2's second run
/// re-validates.
///
/// # Errors
/// [`DomainError::Internal`] on a storage failure or a policy row this gear
/// cannot read.
async fn rule_params(
    policies: &PolicyObjectRepo,
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    shape: &PlanShape,
) -> Result<PublishRuleParams, DomainError> {
    let plan_id = shape.plan_id;
    let policy = policies
        .authoring_policy_on(runner, scope, tenant_id)
        .await
        .map_err(|e| repo_failure(&e))?;
    // D-212: the bundle markets this plan's rows are judged against
    // (`inst-bc-taxbasis`'s reverse half). Resolved **here**, in the same pass as
    // the caps, for the reason this function exists at all — the rule set runs
    // twice on one publish and a rule that reached for storage itself could
    // answer differently in the two runs for no authored reason. A plan that is a
    // component of nothing costs one index probe returning no rows.
    let referencing = crate::infra::bundle::referencing_markets(runner, scope, tenant_id, plan_id)
        .await
        .map_err(|e| repo_failure(&e))?;
    // `inst-tx-region`: the tenant's **active** region universe. Resolved here,
    // in the same pass as everything else, for the reason this function exists —
    // the rule set runs twice on one publish and a rule reaching for storage
    // itself could answer differently in the two runs for no authored reason.
    //
    // It is the `active` set, which is `overlay_repo::declares`' predicate one
    // plane over: a value that reached `retired` must not validate a new row
    // against itself. An empty answer is C2's fail-closed reading and not an
    // error — a tenant who has declared no region publishes no row.
    let declared_regions = taxonomy_repo::active_regions(runner, scope, tenant_id)
        .await
        .map_err(|e| repo_failure(&e))?;
    // C4's `RegionTaxReadiness`, resolved in the same pass and for the same
    // reason. One statement over the tenant's declared regions rather than one
    // per market: a plan at C1's 20-currency floor spans as many, and twenty
    // round trips inside the commit transaction is twenty chances to hold it
    // open longer than it needs.
    let readiness = RegionTaxReadiness::new(
        taxonomy_repo::region_readiness_map(runner, scope, tenant_id)
            .await
            .map_err(|e| repo_failure(&e))?
            .into_iter()
            .map(|(region, markers)| {
                (
                    region,
                    RegionReadiness {
                        tax_category: markers.tax_category,
                        tax_rate_present: markers.tax_rate_present,
                    },
                )
            })
            .collect(),
    );
    let tax_display_policy = policy.tax_display_policy().map_err(|e| repo_failure(&e))?;
    // `inst-cb-addon`'s domain: what each of this plan's add-ons covers. Resolved
    // here for the reason the markets above are — the rule is a property of other
    // plans' published rows and the set runs twice on one publish.
    let addon_coverage =
        crate::infra::currency_binding::addon_coverage(runner, scope, tenant_id, shape)
            .await
            .map_err(|e| repo_failure(&e))?;
    Ok(PublishRuleParams::new(
        policy.interval_bounds(),
        policy.descriptor_rule(),
        policy.default_rounding_policy_ref().map(ToOwned::to_owned),
        // The two soft caps, which had no consumer at all until D-160 named the
        // advisory code. Read in the same pass as everything else, so the
        // commit's second run judges the caps the state then holds.
        SoftSizeCaps::new(
            policy.max_tier_bands_per_row(),
            policy.max_price_rows_per_plan(),
        ),
    )
    .with_referencing_markets(referencing)
    .with_declared_regions(declared_regions)
    .with_tax_display(tax_display_policy, readiness)
    .with_addon_coverage(addon_coverage))
}

/// The registry request id of one publish unit.
///
/// **Deterministic, and that is the whole point.** `request_id` is documented as
/// the catalog's own idempotency handle — "a retry after a crash returns the
/// same pending ref rather than queueing a second assignment" — so a commit that
/// rolled back and is retried must present the id its first attempt did. Derived
/// from `(tenant, plan, revision)` because that is what identifies the publish:
/// a revision publishes exactly once, so the id names one assignment for the
/// life of the plan. A random id per attempt would orphan a pending ref at the
/// registry on every rollback, and each orphan trips
/// `pricing.catalogversion.commit_overdue` for a publish that never happened.
///
/// The tenant is in it, unlike the outbox dedup key: that key is unique **per
/// tenant** by its index, while the registry is a cross-tenant service and has
/// no such scope to lean on.
///
/// **The unit's kind is in the preimage (D-99).** A revision publishes exactly
/// once, but a revision is not the whole of what identifies a unit any more: a
/// window mutation is a publish unit over the plan's *current* revision, so a
/// content publish and a window mutation of one revision are two units that would
/// otherwise present the same id. The registry is idempotent on that id, so the
/// second would be answered with the first's pending ref — and the window mutation
/// would ride a version that never re-projected it, which is precisely the
/// last-warmed-delta-disagrees-with-the-truth-side exposure D-99 exists to close.
/// Two window mutations of one revision are likewise two units, which is why the
/// window's own id is in the token rather than a bare `window-mutation` marker.
///
/// The plan-content token is unchanged from before there was a kind, for
/// [`PublishUnitKind::request_token`](crate::domain::publish::PublishUnitKind::request_token)'s reason: re-spelling it would orphan every
/// pending ref a retry is meant to find.
#[must_use]
fn publish_request_id(tenant_id: Uuid, unit: PlanPublishUnit) -> String {
    format!(
        "{}/{tenant_id}/{}/{}",
        unit.kind.request_token(),
        unit.plan_id,
        unit.revision
    )
}

/// Build the publish subject from storage, through whichever runner the
/// caller holds.
///
/// Generic over the runner precisely so the pre-check and the commit's
/// re-validation are the **same** assembly: two assemblers would be two
/// answers to "what is being published", and §4.2's whole point is that the
/// second run judges the same subject against a world that has moved.
///
/// # Errors
/// [`DomainError::NotFound`] when the plan has no open draft revision;
/// [`DomainError::Internal`] on a storage failure or a current revision with
/// no unique terminal phase.
pub(crate) async fn assemble(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: PlanId,
    now: DateTime<Utc>,
) -> Result<PlanShape, DomainError> {
    let draft = plan_repo::load_open_draft(runner, scope, tenant_id, plan_id)
        .await
        .map_err(|e| repo_failure(&e))?
        .ok_or_else(|| DomainError::NotFound {
            subject: "open plan draft revision".to_owned(),
            id: plan_id.to_string(),
        })?;
    assemble_from(runner, scope, tenant_id, plan_id, draft, now).await
}

/// The assembly itself, over a revision the caller has already resolved.
///
/// **One body, two entry points**, which is the reason [`assemble`]'s own doc
/// gives for being generic over the runner, applied one level up: two assemblies
/// would be two answers to "what is this plan", and the window path validates
/// against the same subject the publish path judges.
///
/// [`assemble`] resolves the plan's **open draft**; `infra::window` resolves its
/// **current** revision and calls this directly, because
/// [`PlanPublishUnit::window_mutation`] names the current revision by decision —
/// a window mutation authors no plan content and opens none. Passing the record in
/// rather than resolving it here is what lets that caller take the revision's
/// `lifecycle_state` off the same read it assembles from, instead of reading the
/// plan row twice and risking two answers about which revision is current.
pub(crate) async fn assemble_from(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: PlanId,
    draft: crate::domain::plan::PlanRevision,
    now: DateTime<Utc>,
) -> Result<PlanShape, DomainError> {
    let mut shape = PlanShape::new(plan_id, draft.revision, now);
    shape.sku_id = draft.sku_id;
    shape.billing_cycle = draft.billing_cycle;
    shape.frequency = draft.frequency;
    shape.plan_tier = draft.plan_tier.clone();
    shape.plan_tier_override = draft.plan_tier_override;
    shape.available_from = draft.available_from;
    shape.available_to = draft.available_to;
    shape.purchase_min_qty = draft.purchase_min_qty;
    shape.purchase_max_qty = draft.purchase_max_qty;
    shape.invoice_grouping_key = draft.invoice_grouping_key.clone();
    shape.phases = PhaseGraph::new(
        plan_shape_repo::load_phase_set(runner, scope, tenant_id, plan_id, draft.revision)
            .await
            .map_err(|e| repo_failure(&e))?,
    );
    shape.addon_rules =
        plan_shape_repo::load_addon_rule_set(runner, scope, tenant_id, plan_id, draft.revision)
            .await
            .map_err(|e| repo_failure(&e))?;
    shape.descriptor_set =
        plan_shape_repo::load_descriptor(runner, scope, tenant_id, plan_id, draft.revision)
            .await
            .map_err(|e| repo_failure(&e))?;
    // The candidate set: the shape as it will be after the commit.
    shape.rows = price_repo::load_for_plan(runner, scope, tenant_id, plan_id, CANDIDATE_ROW_STATES)
        .await
        .map_err(|e| repo_failure(&e))?;
    shape.windows = window_plane(runner, scope, tenant_id, plan_id, &shape.rows).await?;
    shape.baseline = published_baseline(runner, scope, tenant_id, plan_id).await?;
    Ok(shape)
}

/// The plan's window plane, grouped per canonical scope key, seeded with the
/// candidate rows' keys.
///
/// **Read here rather than only at `GET …/coverage`** so the coverage report
/// appears in [`PublishService::precheck`] too. The pre-check is the operator's
/// remediation path: a plan that cannot publish for want of a window has to say
/// so before the two-person unit opens, or the first news of it is a refused
/// commit.
///
/// # Two filters this deliberately does not apply
///
/// `window_repo::list_for_plan` is taken whole — every window state, over every
/// price row of the plan whatever its lifecycle state — because the coverage
/// rules are **validation over a hypothetical**: "if this row were current on
/// this key, would the interval set be sound?", asked of a change set made of
/// `draft` rows about to publish. That is the same posture
/// `price_repo::load_scope_keys_for_plan` states for itself and the same posture
/// `window_repo::refuse_overlap` runs under.
///
/// The two readers that *assert facts to a consumer* — `read_model::project_windows`
/// and the activation sweep — restrict to
/// [`PROJECTED_ROW_STATES`](crate::domain::projection::PROJECTED_ROW_STATES)
/// instead, and that asymmetry is the point rather than an inconsistency to
/// resolve. A later group narrowing this read to match theirs would move the
/// publish check across that line, and a `draft` row's own window would stop
/// counting as the coverage of the key it is about to publish onto.
///
/// Which of the intervals then *contribute* coverage is each reader's own
/// statement: [`KeyWindows::coverage_end`] drops `cancelled`, and
/// [`crate::domain::coverage`] names the state set `inst-wc-required` and
/// `inst-fg-detect` read.
///
/// # Errors
/// [`DomainError::Internal`] on a storage failure, or when a window names a price
/// row that is not on the plan.
async fn window_plane(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: PlanId,
    candidates: &[PriceRecord],
) -> Result<Vec<KeyWindows>, DomainError> {
    let records = window_repo::list_for_plan(runner, scope, tenant_id, plan_id)
        .await
        .map_err(|e| repo_failure(&e))?;
    Ok(window::group_by_key_seeded(
        candidates.iter().map(|record| record.scope_key.clone()),
        records.into_iter().map(|w| {
            (
                w.scope_key,
                WindowInterval::new(w.effective_from, w.effective_to, w.state),
            )
        }),
    ))
}

/// What the plan's current revision holds, for the rules that compare
/// against it. `None` on a first publish.
async fn published_baseline(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: PlanId,
) -> Result<Option<PublishedBaseline>, DomainError> {
    let Some(current) = plan_repo::load_current(runner, scope, tenant_id, plan_id)
        .await
        .map_err(|e| repo_failure(&e))?
    else {
        return Ok(None);
    };
    let terminal_phase_id = terminal_phase(runner, scope, tenant_id, plan_id, &current).await?;
    // The **published** plane alone. `PHASE_IN_USE` refuses dropping a phase
    // a current published row still references; taken off the candidate set
    // instead, a draft row this very revision authored could keep a phase
    // alive that nothing published uses.
    let published = price_repo::load_for_plan(
        runner,
        scope,
        tenant_id,
        plan_id,
        &[LifecycleState::Published],
    )
    .await
    .map_err(|e| repo_failure(&e))?;
    Ok(Some(PublishedBaseline {
        terminal_phase_id,
        phase_ids_in_use: published
            .iter()
            .map(|record| record.scope_key.phase())
            .collect(),
        available_from: current.available_from,
        available_to: current.available_to,
    }))
}

/// The current revision's terminal phase.
///
/// A published revision has **exactly one**: `PHASE_GRAPH_INVALID` refused
/// anything else at its own publish, and `uq_pricing_plan_phase_terminal`
/// refuses a second one physically. So zero or two here is an invariant breach
/// rather than a caller mistake, and it surfaces as an internal fault instead of
/// silently yielding no baseline — a baseline quietly withheld is D-64's guard
/// switched off, which is precisely the "pass vacuously" behaviour
/// [`PublishedBaseline`] exists to prevent.
async fn terminal_phase(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: PlanId,
    current: &PlanRevision,
) -> Result<PhaseId, DomainError> {
    let phases = PhaseGraph::new(
        plan_shape_repo::load_phase_set(runner, scope, tenant_id, plan_id, current.revision)
            .await
            .map_err(|e| repo_failure(&e))?,
    );
    let terminals = phases.terminals();
    match terminals.as_slice() {
        [only] => Ok(only.phase_id),
        _ => Err(DomainError::Internal(format!(
            "plan {plan_id} revision {} is current and holds {} terminal phases; \
             PHASE_GRAPH_INVALID permits exactly one",
            current.revision,
            terminals.len()
        ))),
    }
}

/// Ask the joint-conformance gate about **every** row of the publish unit.
///
/// The gate is a fail-closed **step** beside the report rather than a rule
/// inside it, and the doc on [`PublishService::precheck`] carries why. Its
/// refusal is the whole publish's refusal: one row whose shape nobody has agreed
/// how to evaluate makes the plan unpublishable, because the plan is what
/// becomes consumer-visible.
///
/// [`Reservation::of_slice3_row`] is asked at the call site rather than assumed,
/// which is the named hole that type exists to keep visible: a Slice-3 row
/// cannot say whether it reserves, and when Slice 10 lands that constructor
/// starts reading the row and this call site does not move.
///
/// # Errors
/// [`DomainError::FixtureMissing`] naming the kind and the variant that has no
/// green fixture.
fn check_fixtures(gate: &FixtureGate, shape: &PlanShape) -> Result<(), DomainError> {
    for record in &shape.rows {
        gate.check(&record.row, Reservation::of_slice3_row(&record.row))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "publish_tests.rs"]
mod publish_tests;
