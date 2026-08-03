//! The publish unit's vocabulary: what is published, on whose authority, and
//! what the commit hands back.
//!
//! `design/01-foundation.md` §4.2 is one path with five steps, and this module
//! names the three values that path carries. It runs no rule and touches no
//! store — those are [`crate::domain::plan_rules`] and
//! [`crate::infra::publish`] respectively — because the words have to be
//! agreed before either can be written against them.
//!
//! ## One publish unit, and the ones that are not here
//!
//! [`PlanPublishUnit`] is a struct rather than a one-variant enum. §4.2 names
//! four kinds of publish unit that run the **same** engine path — the plan
//! publish (this one), plan retirement (D-128), window mutations (D-99), and
//! the `PriceOverlay` and customer-group membership units (D-06) — and three of
//! them need storage this gear has not built: there is no `PriceWindow` store,
//! no overlay revision table and no membership record. A single-variant enum
//! today would be a shape pretending to be a choice; when the second unit
//! arrives it gets its own type or this one grows a discriminator, and either
//! way the decision is made with the second unit in hand rather than guessed at
//! now.
//!
//! ## The approval gate is a value, not a call
//!
//! [`PublishAuthorization`] has no `Default` and no third arm, so
//! `PublishService::commit` cannot be called without a decision having been
//! made. That is the whole of what the commit genuinely needs from Slice 5:
//! **that it cannot proceed undecided**, and that whatever was decided reaches
//! the audit trail (`inst-au-complete`, `inst-tp-record`).
//!
//! What this deliberately is **not** is the approval workflow. Slice 5's
//! surface is the `MaterialityEvaluator` over a registered-trigger set owned by
//! eight slices (`inst-mat-registered` names D-10, D-13, D-50, D-62, D-104,
//! D-109 and D-115 in one paragraph), the `pricing_approval` store — which has
//! no migration and no entity in this crate — the content pin and its
//! `APPROVAL_CONTENT_MISMATCH`, the withdraw path and the approver scope check.
//! None of it is reachable from anywhere today, because this gear has no
//! authoring REST surface at all. Building it here would put that workflow in
//! front of the five moves the publish commit exists to make, and would shape it
//! with no caller to shape it against.
//!
//! The consequence is stated rather than hidden: **nothing in this crate can
//! mint a [`PublishAuthorization`] today except a test.** That costs exactly
//! nothing while no surface can reach the commit, and it becomes the approval
//! group's first obligation the moment one can.
//!
//! ## What is deliberately absent
//!
//! The two-person rule's `submitter != approver` check (`inst-tp-distinct`) is
//! **not** asserted by [`PublishAuthorization::approved`], even though the
//! comparison is one line. `inst-tp-selfaudit` binds the refusal to an audit
//! record of the *attempted violation*, written against the approval's own
//! subject — and there is no approval record and no approval `chain_id` for
//! that record to extend. Enforcing half the rule here would give it two
//! owners, free to disagree the day the other half lands, which is the defect
//! this crate's single-owner discipline exists to prevent. So the type
//! **carries** both principals, which is what makes the trail complete, and the
//! check plus its audited refusal stay whole in Slice 5's group.
//! `SELF_APPROVAL_FORBIDDEN` therefore gets no constant here: a code declared
//! where nothing raises it reads as enforcement to everyone who greps for it.

pub mod rules;

use toolkit_macros::domain_model;
use uuid::Uuid;

use crate::domain::evaluation_policy::EVALUATION_POLICY_GENERATION;
use crate::domain::scope_key::PlanId;
use crate::domain::snapshot::{PricingSnapshotRef, VersionRef};

/// The subject of a plan publish: one revision of one plan.
///
/// `(plan_id, revision)` is the durable name D-145 made permanent — the number
/// is minted once and never re-minted — so naming the unit by it is naming
/// exactly one row for the life of the plan.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlanPublishUnit {
    /// The plan being published.
    pub plan_id: PlanId,
    /// Which of its revisions.
    pub revision: u64,
}

impl PlanPublishUnit {
    /// Name a publish unit.
    #[must_use]
    pub const fn new(plan_id: PlanId, revision: u64) -> Self {
        Self { plan_id, revision }
    }
}

/// The approval decision a publish commit runs under (§4.2 step 3).
///
/// Two arms and no `Default`, so there is no way to reach the commit with the
/// question unanswered. The fail-safe direction is the design set's: the
/// two-person rule applies **unless** an explicit threshold is configured, the
/// change is below it, and it is not a first publish — so
/// [`PublishAuthorization::auto_publishable`] is the narrow case and
/// [`PublishAuthorization::approved`] is the ordinary one.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublishAuthorization {
    /// A material change that cleared the two-person rule.
    Approved {
        /// The `pricing_approval` record this publish runs under.
        approval_ref: Uuid,
        /// The principal that submitted the change.
        submitter_principal: Uuid,
        /// The independent principal that approved it.
        approver_principal: Uuid,
    },
    /// A below-threshold, non-first publish that needs no approver
    /// (`inst-ap-materiality`).
    AutoPublishable,
}

impl PublishAuthorization {
    /// A publish running under a closed approval record.
    ///
    /// **What the caller must already have established**, none of which this
    /// constructor re-checks: that `approval_ref` names a `pricing_approval`
    /// row in `approved` state whose `subject_ref` is the unit being published;
    /// that its pinned `content_hash` still matches the submitted content
    /// (S5 §6's TOCTOU guard); that the approver holds the scope
    /// `inst-rb-approve` requires; and that the two principals are distinct
    /// (`inst-tp-distinct` — see the module doc for why that one is not
    /// asserted here).
    #[must_use]
    pub const fn approved(
        approval_ref: Uuid,
        submitter_principal: Uuid,
        approver_principal: Uuid,
    ) -> Self {
        Self::Approved {
            approval_ref,
            submitter_principal,
            approver_principal,
        }
    }

    /// A publish that needs no approver.
    ///
    /// **What the caller must already have established**: that the tenant has
    /// an explicitly configured approval threshold, that the
    /// `MaterialityEvaluator` scored this change below it, and that it is not a
    /// first publish. The fail-safe reading is load-bearing — a tenant with no
    /// configured threshold makes *everything* material (S5 G1), so an
    /// evaluator that cannot answer must not reach for this arm.
    #[must_use]
    pub const fn auto_publishable() -> Self {
        Self::AutoPublishable
    }

    /// The approval record backing this publish, when there is one.
    ///
    /// It is what lands in `pricing_audit_log.approval_ref`, so an auditor can
    /// walk from the mutation to the decision that permitted it.
    #[must_use]
    pub const fn approval_ref(&self) -> Option<Uuid> {
        match self {
            Self::Approved { approval_ref, .. } => Some(*approval_ref),
            Self::AutoPublishable => None,
        }
    }

    /// The `(submitter, approver)` pair, when the publish was approved.
    ///
    /// `inst-au-complete` requires the approval trail on the audit record and
    /// `inst-tp-record` requires both identities on it. An auto-publishable
    /// change has no second principal to record, and `None` says exactly that
    /// rather than repeating the actor as its own approver.
    #[must_use]
    pub const fn principals(&self) -> Option<(Uuid, Uuid)> {
        match self {
            Self::Approved {
                submitter_principal,
                approver_principal,
                ..
            } => Some((*submitter_principal, *approver_principal)),
            Self::AutoPublishable => None,
        }
    }
}

/// What a successful publish commit hands back.
///
/// Enough for a surface to answer §4.2's 202 and for a test to assert the
/// commit's five artifacts, and no more — it is deliberately not a second copy
/// of the row set, which the caller can read from the store it just wrote.
///
/// The version ref is **structurally pending**: [`PublishReceipt::new`] takes
/// the registry's handle and builds the [`VersionRef`] itself, so a receipt
/// carrying a committed version is not expressible. That is the G5/G6 seam as a
/// type — the commit holds a handle, and only `CatalogVersionPublished` turns a
/// handle into a version.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishReceipt {
    plan_id: PlanId,
    revision: u64,
    version_ref: VersionRef,
    published_price_ids: Vec<Uuid>,
    audit_seq: u64,
}

impl PublishReceipt {
    /// Stamp a receipt from what the commit produced.
    #[must_use]
    pub fn new(
        unit: PlanPublishUnit,
        pending_ref: String,
        published_price_ids: Vec<Uuid>,
        audit_seq: u64,
    ) -> Self {
        Self {
            plan_id: unit.plan_id,
            revision: unit.revision,
            version_ref: VersionRef::Pending(pending_ref),
            published_price_ids,
            audit_seq,
        }
    }

    /// The plan that was published.
    #[must_use]
    pub const fn plan_id(&self) -> PlanId {
        self.plan_id
    }

    /// The revision that became current.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// The pending version ref the publish is addressable through until
    /// `CatalogVersionPublished` resolves it.
    #[must_use]
    pub const fn version_ref(&self) -> &VersionRef {
        &self.version_ref
    }

    /// The price rows this commit moved into `published`.
    #[must_use]
    pub fn published_price_ids(&self) -> &[Uuid] {
        &self.published_price_ids
    }

    /// The `seq` the plan's audit chain segment reached.
    #[must_use]
    pub const fn audit_seq(&self) -> u64 {
        self.audit_seq
    }

    /// The catalog-side `pricingSnapshotRef` this commit stamped — all three
    /// segments (D-162).
    ///
    /// The evaluation-policy generation is taken from the constant rather than
    /// carried in the receipt: it is the publishing gear's, not the publish's,
    /// so there is no call site that could stamp a period with a semantics its
    /// rows were never frozen under. The version ref is still structurally
    /// pending; `CatalogVersionPublished` finalizes it through
    /// [`PricingSnapshotRef::finalize`].
    #[must_use]
    pub fn snapshot_ref(&self) -> PricingSnapshotRef {
        PricingSnapshotRef::new(
            self.version_ref.clone(),
            self.published_price_ids.clone(),
            EVALUATION_POLICY_GENERATION.to_owned(),
        )
    }
}

#[cfg(test)]
#[path = "publish_tests.rs"]
mod publish_tests;
