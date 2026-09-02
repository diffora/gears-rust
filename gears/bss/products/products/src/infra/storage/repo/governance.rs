//! The approval store — the submit write, the decision write and the
//! pending-queue read (`design/05-governance.md` §4; P-D-11, P-D-13,
//! P-D-68).
//!
//! Split out of the foundation repository the way the sibling modules were;
//! every item re-exports through `super`
//! (`crate::infra::storage::repo`) unchanged.
//!
//! # The submit write is where materiality is evaluated, and it happens once
//!
//! `inst-mt-once` requires the judgement at submission against the policy in
//! force at that instant, and [`submit_approval`] is that instant: it takes
//! the verdict its caller computed with
//! [`crate::domain::materiality::evaluate`], renders the descriptor, and
//! stores both the descriptor and the content snapshot. Nothing downstream
//! re-evaluates — a reader takes `quorum_descriptor` off the row, which is
//! what makes the record stable when the tenant later edits `N`.
//!
//! # Two UNIQUEs, two different meanings, both classified
//!
//! The partial `UNIQUE (tenant_id, subject_kind, subject_ref) WHERE state IN
//! ('pending','satisfied')` is *"one open approval per subject"*, and L-4
//! makes a new submission **supersede** the open one rather than be refused
//! — so [`submit_approval`] supersedes first and lets the index be the floor
//! under a lost race. `UNIQUE (tenant_id, approval_id, approver_principal)`
//! is C2's *"one principal, one decision"* and **is** a refusal: a second
//! verdict from the same principal is not a supersession, it is the
//! two-person invariant being probed.
//!
//! @cpt-cf-bss-products-dod-stored-snapshot
//! @cpt-cf-bss-products-dod-self-approval

use chrono::{DateTime, Utc};
use sea_orm::ActiveValue::Set;
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use toolkit_db::secure::{AccessScope, DBRunner, SecureEntityExt, SecureInsertExt};
use uuid::Uuid;

use super::{driver_failure, supersede_open_approval};
use crate::domain::approval::{
    AckPlacement, QuorumDescriptor, ack_placement, decision_admitted, describe_quorum,
    descriptor_from_stored,
};
use crate::domain::error::DomainError;
use crate::domain::governance::{ApprovalId, GateSubject};
use crate::domain::materiality::{MaterialAct, Materiality, MaterialityEvaluator};
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::{approval, approval_decision};

/// One submission, as the store needs it.
///
/// A struct rather than nine arguments: the two `Uuid`s (`tenant_id` and
/// `submitter`) and the two `Option<i64>`-adjacent numbers are what a call
/// site could transpose without the compiler noticing.
#[derive(Clone, Debug)]
pub struct NewApproval<'a> {
    /// The tenant the record belongs to.
    pub tenant_id: Uuid,
    /// The record's own id, minted by the caller so the door can answer it.
    pub approval_id: ApprovalId,
    /// What is being approved — kind and reference come from here, so a
    /// caller cannot pair a `sku_correction` kind with a batch's reference.
    pub subject: &'a GateSubject,
    /// The revision the submission pins.
    pub internal_revision: i64,
    /// The submitted content, stored and never re-derived.
    pub content_snapshot: &'a str,
    /// The last published version the approver's diff renders against, or
    /// `None` on a first publish.
    pub diff_basis: Option<i64>,
    /// The act under judgement. The store evaluates it rather than taking a
    /// verdict, which is what makes `inst-mt-once`'s *"evaluated once at
    /// submission"* a property of the code rather than a convention: this is
    /// the only caller of [`MaterialityEvaluator::verdict`] on a write path,
    /// so there is no second place a verdict could be formed.
    pub act: &'a MaterialAct<'a>,
    /// The evaluator over the two inputs this submission resolved.
    pub evaluator: MaterialityEvaluator<'a>,
    /// Whether the change touches a finance-material field. The caller's,
    /// because `inst-gv-finance-predicate` names three columns the bucket
    /// registry does not carry — see `domain::approval`'s module doc.
    pub finance_material: bool,
    /// The tenant's `N` **in force at this instant** — stored inside the
    /// descriptor so a later policy edit cannot change a pending record.
    pub approver_count: u32,
    /// The author, pseudonymous from birth.
    pub submitter: Uuid,
    /// The author's own override acknowledgment, admitted **only** at
    /// effective quorum zero (P-D-68 arm 1). Supplied at any other count it
    /// is refused, because above zero the acknowledgment belongs on the
    /// approver's decision row.
    pub author_override_ack: Option<&'a str>,
}

/// Store one submission, superseding whatever open record the subject held.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure. [`RepoError::Db`] carrying
/// the refusal's reason when `author_override_ack` is supplied at an
/// effective quorum above zero, which no door may do.
pub async fn submit_approval(
    runner: &impl DBRunner,
    scope: &AccessScope,
    new: NewApproval<'_>,
    submitted_at: DateTime<Utc>,
) -> Result<Submitted, RepoError> {
    // `inst-mt-once`: the judgement happens here, at submission, against
    // the inputs the caller resolved — and exactly once, because the
    // descriptor it produces is stored and every later reader takes the
    // stored value.
    let materiality = new
        .evaluator
        .verdict(new.act)
        .map_err(|e| RepoError::Db(format!("materiality of {}: {e}", new.approval_id)))?;
    let descriptor = describe_quorum(materiality, new.approver_count, new.finance_material);
    let descriptor = &descriptor;
    // The author's acknowledgment has exactly one admitted home, and it is
    // this count. Refusing here rather than silently dropping the value is
    // what keeps `author_override_ack` from becoming a second, unpoliced
    // acknowledgment channel above zero.
    if new.author_override_ack.is_some() && ack_placement(descriptor) != AckPlacement::OnRecord {
        return Err(RepoError::Db(format!(
            "author override acknowledgment supplied at required {}: it is admitted only at \
             effective quorum zero (P-D-68 arm 1); above zero it rides the approver's decision row",
            descriptor.required()
        )));
    }

    // L-4: a new submission explicitly supersedes the open one. Doing it
    // before the insert is what turns the partial UNIQUE from a refusal a
    // door must handle into a floor under a lost race.
    supersede_open_approval(runner, scope, new.tenant_id, new.subject, submitted_at).await?;

    let model = approval::ActiveModel {
        tenant_id: Set(new.tenant_id),
        approval_id: Set(new.approval_id.get()),
        subject_kind: Set(new.subject.kind.as_str().to_owned()),
        subject_ref: Set(new.subject.reference.clone()),
        internal_revision: Set(new.internal_revision),
        content_snapshot: Set(new.content_snapshot.to_owned()),
        diff_basis: Set(new.diff_basis),
        quorum_descriptor: Set(descriptor.stored()),
        state: Set("pending".to_owned()),
        submitter: Set(new.submitter),
        author_override_ack: Set(new.author_override_ack.map(str::to_owned)),
        author_override_ack_at: Set(new.author_override_ack.map(|_| submitted_at)),
        submitted_at: Set(submitted_at),
        finalized_at: Set(None),
    };
    approval::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| driver_failure(format!("approval scope of {}", new.tenant_id), e))?
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("submit approval {}", new.approval_id), e))?;
    Ok(Submitted {
        approval_id: new.approval_id,
        materiality,
        descriptor: descriptor.clone(),
    })
}

/// What a submission answers: the record's id plus the judgement it was
/// stored under, so a door can render the queue envelope without reading the
/// row back.
#[derive(Clone, Debug)]
pub struct Submitted {
    /// The record's id.
    pub approval_id: ApprovalId,
    /// The verdict evaluated at this submission.
    pub materiality: Materiality,
    /// The descriptor stored on the record.
    pub descriptor: QuorumDescriptor,
}

/// One principal's verdict, as the store needs it.
#[derive(Clone, Debug)]
pub struct NewDecision<'a> {
    /// The tenant.
    pub tenant_id: Uuid,
    /// The record being decided.
    pub approval_id: ApprovalId,
    /// The deciding principal, an `actor_ref` and pseudonymous from birth.
    pub approver_principal: Uuid,
    /// `approved` or `rejected`.
    pub verdict: DecisionVerdict,
    /// The operator's free-text reason. Passes 02's write-block hook before
    /// it reaches here — this module does not run it, and
    /// `dod-pii-on-reasons` stays unticked because the hook does not ship.
    pub reason: Option<&'a str>,
    /// The lint findings this approver acknowledged, by name.
    pub override_acknowledgments: Option<&'a str>,
}

/// The two verdicts the `CHECK` admits.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DecisionVerdict {
    /// The approver approved.
    Approved,
    /// The approver rejected.
    Rejected,
}

impl DecisionVerdict {
    /// The stored spelling, which is what the `CHECK` pins.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Rejected => "rejected",
        }
    }
}

/// Record one principal's decision, refusing the author's own.
///
/// The self-approval check reads the record's stored `submitter` rather than
/// taking it as an argument: a caller that passed the submitter in could
/// pass the wrong one, and the row is the only authority on who submitted.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure, and [`RepoError::Db`]
/// carrying `SELF_APPROVAL_FORBIDDEN`'s reason when the deciding principal
/// submitted the record, or the duplicate-decision reason when that
/// principal has already decided it.
pub async fn record_decision(
    runner: &impl DBRunner,
    scope: &AccessScope,
    new: NewDecision<'_>,
    decided_at: DateTime<Utc>,
) -> Result<(), RepoError> {
    let record = read_approval(runner, scope, new.tenant_id, new.approval_id)
        .await?
        .ok_or_else(|| {
            RepoError::Db(format!(
                "no approval {} in tenant {}",
                new.approval_id, new.tenant_id
            ))
        })?;
    let descriptor = descriptor_from_stored(&record.quorum_descriptor)
        .map_err(|e| RepoError::Db(format!("stored descriptor of {}: {e}", new.approval_id)))?;

    decision_admitted(record.submitter, new.approver_principal, &descriptor)
        .map_err(|e: DomainError| RepoError::Db(format!("{}: {e}", e.code())))?;

    let model = approval_decision::ActiveModel {
        tenant_id: Set(new.tenant_id),
        approval_id: Set(new.approval_id.get()),
        approver_principal: Set(new.approver_principal),
        verdict: Set(new.verdict.as_str().to_owned()),
        reason: Set(new.reason.map(str::to_owned)),
        override_acknowledgments: Set(new.override_acknowledgments.map(str::to_owned)),
        decided_at: Set(decided_at),
    };
    approval_decision::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| driver_failure(format!("decision scope of {}", new.tenant_id), e))?
        .exec(runner)
        .await
        .map_err(|e| {
            classify_decision_insert(
                new.approver_principal,
                driver_failure(format!("record decision of {}", new.approval_id), e),
            )
        })?;
    Ok(())
}

/// C2's UNIQUE, read back as the refusal it is.
///
/// A second verdict from the same principal is **not** a supersession: the
/// UNIQUE is the physical floor under "one principal, one decision, whatever
/// roles they hold", so a hit means a human holding two roles tried to
/// decide twice under them.
fn classify_decision_insert(principal: Uuid, error: RepoError) -> RepoError {
    let message = error.to_string().to_ascii_lowercase();
    if message.contains("unique constraint")
        || message.contains("duplicate key")
        || message.contains("primary key")
    {
        return RepoError::Db(format!(
            "principal {principal} has already decided this record: one principal, one decision, \
             whatever roles they hold (design/05 C2)"
        ));
    }
    error
}

/// Read one approval record.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn read_approval(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    approval_id: ApprovalId,
) -> Result<Option<approval::Model>, RepoError> {
    approval::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(approval::Column::TenantId.eq(tenant_id))
                .add(approval::Column::ApprovalId.eq(approval_id.get())),
        )
        .one(runner)
        .await
        .map_err(|e| driver_failure(format!("read approval {approval_id}"), e))
}

/// The pending queue, oldest first — the operand behind
/// `GET /bss-products/v1/approvals?state=pending` (`inst-gv-queue`), whose
/// door is 05's to declare.
///
/// Ordered by `submitted_at` because that is the column
/// `idx_products_approval_queue` leads with after `state`, so the read is an
/// index scan rather than a sort.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn pending_approvals(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
) -> Result<Vec<approval::Model>, RepoError> {
    approval::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(approval::Column::TenantId.eq(tenant_id))
                .add(approval::Column::State.eq("pending")),
        )
        .order_by(approval::Column::SubmittedAt, sea_orm::Order::Asc)
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("pending approvals of {tenant_id}"), e))
}
