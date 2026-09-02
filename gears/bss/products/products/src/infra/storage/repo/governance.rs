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
//! force at that instant, and [`submit_approval`] is that instant: it
//! **evaluates** the act with the [`MaterialityEvaluator`] whose inputs its
//! caller resolved ([`MaterialityEvaluator::verdict`]), renders the
//! descriptor, and stores both the descriptor and the content snapshot. It
//! does **not** take a verdict — that would be the second verdict site
//! `inst-mt-once` exists to forbid.
//!
//! What the row carries is the verdict's **effect**, not the verdict: the
//! descriptor's five names include no `Materiality`, and at `N <= 1` both
//! verdicts render byte-identical descriptors. A reader that needs the
//! verdict itself takes [`Submitted::materiality`] from this call;
//! `features/governance.md` §7 row 15 carries the question of whether the
//! stored `quorumReduced` should distinguish reduced-by-configuration from
//! reduced-by-non-materiality, and until it does the descriptor cannot.
//!
//! # Two UNIQUEs, two different meanings, both classified
//!
//! The partial `UNIQUE (tenant_id, subject_kind, subject_ref) WHERE state IN
//! ('pending','satisfied')` is *"one open approval per subject"*, and L-4
//! makes a new submission **supersede** the open one rather than be refused
//! — so [`submit_approval`] supersedes first and lets the index be the floor
//! under a lost race. `UNIQUE (tenant_id, approval_id, approver_principal)`
//! is C2's floor as §4 words it — *"one principal, one decision"* — and it
//! **is** a refusal: a second
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
use crate::domain::materiality::{
    MaterialAct, Materiality, MaterialityEvaluator, MaterialityRefusal,
};
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::{approval, approval_decision};

/// What a governance store write can answer.
///
/// Two arms, because the two have different wire meanings and flattening
/// them was measured: every domain refusal these functions raise used to be
/// stringified into [`RepoError::Db`], whose one production reader
/// (`api::rest::repo_error_to_canonical`) renders a bare **500** with no
/// registry code. So `SELF_APPROVAL_FORBIDDEN`'s declared 403 had no
/// reachable producer, and `ILLEGAL_FIELD_MUTATION`'s 409 was buried in a
/// message string. The shape here is `HeadActError`'s, which the head doors
/// already use for the same reason.
#[derive(Debug)]
pub enum ApprovalStoreError {
    /// A domain refusal. The caller renders it through
    /// `infra::error_mapping`, which carries its declared status and code.
    Refused(DomainError),
    /// A storage or scope failure, and the refusals for which `design/05`
    /// §3.3 declares **no code** — a decision on a record that admits none,
    /// and a second verdict from one principal. Both are registered in
    /// `features/governance.md` §7 rather than given an invented code, the
    /// roster being closed at six.
    Repo(RepoError),
}

impl From<RepoError> for ApprovalStoreError {
    fn from(error: RepoError) -> Self {
        Self::Repo(error)
    }
}

impl core::fmt::Display for ApprovalStoreError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Refused(e) => write!(f, "{}: {e}", e.code()),
            Self::Repo(e) => write!(f, "{e}"),
        }
    }
}

/// One submission, as the store needs it.
///
/// A struct rather than eleven arguments: two pairs are mutually assignable
/// and a call site could transpose either without the compiler noticing —
/// the `Uuid`s `approval_id` and `submitter`, and the numbers
/// `internal_revision` and `approver_count`.
///
/// **The tenant is read off `subject`**, not carried a second time: a
/// [`GateSubject`] already scopes itself, and a second copy is exactly the
/// transposition hazard this grouping exists to remove — a door building the
/// subject from an entity and taking the tenant from elsewhere would write a
/// record under one tenant naming another's subject.
#[derive(Clone, Debug)]
pub struct NewApproval<'a> {
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
/// # This function opens no transaction — `runner` MUST be the door's own
///
/// It writes twice: the supersession, then the insert. On a plain connection
/// those are two autocommits, and a lost race then leaves the previous open
/// record **permanently** `superseded` — the frozen trigger refuses any
/// UPDATE whose `OLD.state` is terminal — while this insert dies on
/// `uq_products_approval_open`. That is L-4's own act answered as a storage
/// failure. [`classify_submit_insert`] turns the collision into a refusal a
/// caller can act on, but only one transaction makes the pair atomic.
///
/// # Errors
///
/// [`ApprovalStoreError::Refused`] when the materiality evaluator refuses —
/// its `Registry` and bucket-ii arms carry `ILLEGAL_FIELD_MUTATION`'s
/// declared 409 — or when the insert loses the open-approval race.
/// [`ApprovalStoreError::Repo`] on a storage or scope failure, when
/// `author_override_ack` is supplied at an effective quorum above zero, and
/// on the codeless unresolvable-input arm.
pub async fn submit_approval(
    runner: &impl DBRunner,
    scope: &AccessScope,
    new: NewApproval<'_>,
    submitted_at: DateTime<Utc>,
) -> Result<Submitted, ApprovalStoreError> {
    // The tenant is the subject's, never a second argument: see
    // `NewApproval`'s own doc.
    let tenant_id = new.subject.tenant_id;
    // `inst-mt-once`: the judgement happens here, at submission, against
    // the inputs the caller resolved — and exactly once, because the
    // descriptor it produces is stored and every later reader takes the
    // stored value.
    let materiality = new.evaluator.verdict(new.act).map_err(|e| match e {
        // The registry's refusal and the bucket-ii arm both carry a declared
        // code (409 `ILLEGAL_FIELD_MUTATION`) and must reach the wire as one.
        MaterialityRefusal::Registry(domain) => ApprovalStoreError::Refused(domain),
        MaterialityRefusal::CorrectableTouch(column) => {
            ApprovalStoreError::Refused(DomainError::IllegalFieldMutation(format!(
                "{column} is bucket ii: after first publish it is writable only through the \
                 correction door, never as an ordinary touch (L-1)"
            )))
        }
        // An act outside the FR's enumeration reached the evaluator, which
        // means a door built one it may not submit: `draft -> discarded` is
        // ungated beyond authz (M-1). `ILLEGAL_TRANSITION` is the declared
        // code for an edge outside the admitted list.
        MaterialityRefusal::OutsideTheEnumeration(to) => {
            ApprovalStoreError::Refused(DomainError::IllegalTransition {
                from: "any".to_owned(),
                to: to.as_str().to_owned(),
            })
        }
        // The codeless arm — `domain::materiality`'s module doc and
        // `features/governance.md` §7 row 34.
        MaterialityRefusal::Unresolved(unresolved) => ApprovalStoreError::Repo(RepoError::Db(
            format!("materiality of {}: {unresolved}", new.approval_id),
        )),
    })?;
    let descriptor = describe_quorum(materiality, new.approver_count, new.finance_material);
    // The author's acknowledgment has exactly one admitted home, and it is
    // this count. Refusing here rather than silently dropping the value is
    // what keeps `author_override_ack` from becoming a second, unpoliced
    // acknowledgment channel above zero.
    if new.author_override_ack.is_some() && ack_placement(&descriptor) != AckPlacement::OnRecord {
        return Err(ApprovalStoreError::Repo(RepoError::Db(format!(
            "author override acknowledgment supplied at required {}: it is admitted only at \
             effective quorum zero (P-D-68 arm 1); above zero it rides the approver's decision row",
            descriptor.required()
        ))));
    }

    // L-4: a new submission explicitly supersedes the open one. Doing it
    // before the insert is what turns the partial UNIQUE from a refusal a
    // door must handle into a floor under a lost race.
    supersede_open_approval(runner, scope, tenant_id, new.subject, submitted_at).await?;

    let model = approval::ActiveModel {
        tenant_id: Set(tenant_id),
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
        .map_err(|e| driver_failure(format!("approval scope of {tenant_id}"), e))?
        .exec(runner)
        .await
        .map_err(|e| {
            classify_submit_insert(
                new.approval_id,
                driver_failure(format!("submit approval {}", new.approval_id), e),
            )
        })?;
    Ok(Submitted {
        approval_id: new.approval_id,
        materiality,
        descriptor,
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
    ///
    /// **Must equal `acting_principal`**, which [`record_decision`] asserts:
    /// the field exists so the row's own column is explicit at the call
    /// site, not so a caller can attribute a verdict to somebody else.
    /// Without the assertion an author holding `approval x decide` could
    /// name a second principal and satisfy C2's two-person invariant alone,
    /// and the row is append-only.
    pub approver_principal: Uuid,
    /// `approved` or `rejected`.
    pub verdict: DecisionVerdict,
    /// The operator's free-text reason. **Mandatory on a rejection**
    /// (`design/05` §2: *"mandatory reason on reject"*), which
    /// [`record_decision`] refuses without — no `CHECK` constrains the
    /// column, so the rule has to live here.
    ///
    /// Passes 02's write-block hook before it reaches here — this module
    /// does not run it, and `dod-pii-on-reasons` stays unticked because the
    /// hook does not ship.
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
/// `acting_principal` is the symmetric move on the other side — the
/// authenticated principal, asserted equal to the one the row will name.
///
/// # Errors
///
/// [`ApprovalStoreError::Refused`] with `SELF_APPROVAL_FORBIDDEN` (403) when
/// the deciding principal submitted the record, and with
/// `APPROVAL_SUPERSEDED` (409) when the record is no longer open.
/// [`ApprovalStoreError::Repo`] on a storage or scope failure, on a
/// `quorum_descriptor` this gear wrote wrong ([`RepoError::CorruptRow`]),
/// and on the three refusals `design/05` §3.3 declares no code for — a
/// principal mismatch, a decision on a record admitting none, and a second
/// verdict from one principal.
pub async fn record_decision(
    runner: &impl DBRunner,
    scope: &AccessScope,
    new: NewDecision<'_>,
    acting_principal: Uuid,
    decided_at: DateTime<Utc>,
) -> Result<(), ApprovalStoreError> {
    if acting_principal != new.approver_principal {
        return Err(ApprovalStoreError::Repo(RepoError::Db(format!(
            "principal {acting_principal} may not cast a verdict attributed to {}: one human, \
             one decision, and the row is append-only (design/05 C2)",
            new.approver_principal
        ))));
    }
    // `design/05` §2: a rejection finalizes the record "with the reason", and
    // no `CHECK` constrains the column — so an unreasoned rejection is
    // refused here or nowhere.
    if new.verdict == DecisionVerdict::Rejected && new.reason.is_none() {
        return Err(ApprovalStoreError::Repo(RepoError::Db(
            "a rejection carries a mandatory reason (design/05 section 2)".to_owned(),
        )));
    }

    let record = read_approval(runner, scope, new.tenant_id, new.approval_id)
        .await?
        .ok_or_else(|| {
            ApprovalStoreError::Repo(RepoError::Db(format!(
                "no approval {} in tenant {}",
                new.approval_id, new.tenant_id
            )))
        })?;
    // A verdict on a closed ceremony cannot be taken back — the decision
    // table is append-only outright — so the state is checked before the
    // insert rather than left to a trigger that does not exist.
    if !matches!(record.state.as_str(), "pending" | "satisfied") {
        return Err(ApprovalStoreError::Refused(
            DomainError::ApprovalSuperseded(format!(
                "approval {} is {}: a decision is admitted only while the record is open",
                new.approval_id, record.state
            )),
        ));
    }
    // A row this gear wrote wrong, not a statement that failed: the one
    // channel that separates the two is `CorruptRow`, and every other
    // `decode_rendering` caller uses it.
    let descriptor = descriptor_from_stored(&record.quorum_descriptor).map_err(|e| {
        ApprovalStoreError::Repo(RepoError::CorruptRow(format!(
            "quorum_descriptor of approval {}: {e}",
            new.approval_id
        )))
    })?;

    // At `required = 0` the record closes with no approver, so it admits no
    // verdict at all — P-D-68 arm 1's whole reason for putting the author's
    // acknowledgment in a column. `decision_admitted`'s `>= 1` guard is
    // silent there by construction, which is why this refusal is separate.
    if descriptor.required() == 0 {
        return Err(ApprovalStoreError::Repo(RepoError::Db(format!(
            "approval {} closes on no approver: it admits no decision row (P-D-68 arm 1)",
            new.approval_id
        ))));
    }
    decision_admitted(record.submitter, new.approver_principal, &descriptor)
        .map_err(ApprovalStoreError::Refused)?;

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
        .map_err(|e| {
            ApprovalStoreError::Repo(driver_failure(
                format!("decision scope of {}", new.tenant_id),
                e,
            ))
        })?
        .exec(runner)
        .await
        .map_err(|e| {
            ApprovalStoreError::Repo(classify_decision_insert(
                new.approver_principal,
                driver_failure(format!("record decision of {}", new.approval_id), e),
            ))
        })?;
    Ok(())
}

/// The open-approval partial UNIQUE, read back as the refusal it is.
///
/// L-4 makes a new submission **supersede** the open one, so a collision
/// here is never "this subject already has an approval" — it is a lost race
/// against a peer submission that superseded the same record first. The
/// caller's remedy is to retry, and a 500 would tell it the opposite.
fn classify_submit_insert(approval_id: ApprovalId, error: RepoError) -> ApprovalStoreError {
    let message = error.to_string().to_ascii_lowercase();
    if message.contains("unique constraint")
        || message.contains("duplicate key")
        || message.contains("uq_products_approval_open")
    {
        return ApprovalStoreError::Refused(DomainError::ApprovalSuperseded(format!(
            "a peer submission on this subject superseded the open record first, so {approval_id} \
             could not open one: re-submit against the current head (design/05 L-4)"
        )));
    }
    ApprovalStoreError::Repo(error)
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
            "principal {principal} has already decided this record: one principal, one \
             decision, and C2 makes distinctness by principal rather than by role, so holding \
             two roles does not buy a second verdict (design/05 §4)"
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

#[cfg(test)]
#[path = "governance_tests.rs"]
mod governance_tests;
