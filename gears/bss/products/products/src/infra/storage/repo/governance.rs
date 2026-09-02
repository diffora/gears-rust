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
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use toolkit_db::secure::{
    AccessScope, DBRunner, SecureEntityExt, SecureInsertExt, SecureUpdateExt,
};
use uuid::Uuid;

use super::{driver_failure, supersede_open_approval};
use crate::domain::approval::{
    AckPlacement, ApprovalState, CandidateApproval, QuorumDescriptor, ack_placement,
    decision_admitted, describe_quorum, descriptor_from_stored,
};
use crate::domain::error::DomainError;
use crate::domain::governance::{ApprovalId, GateSubject};
use crate::domain::materiality::{
    MaterialAct, Materiality, MaterialityEvaluator, MaterialityRefusal,
};
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::{approval, approval_decision, breakglass_session};

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
/// # A rejection finalizes the record, in the same transaction as its row
///
/// `design/05` §2 rule 4: *"A rejection finalizes the record `rejected` with
/// the reason; the subject stays as it was"*, and §4 row 3
/// (`inst-ap-edge-reject`) gives the edge as `pending -> rejected`. Both
/// writes ride the caller's transaction, so a rejection whose finalization
/// fails appends no row either — the alternative leaves a rejection on file
/// against a record still open for a publish.
///
/// **The subject is untouched**, which is the rule and not an omission: there
/// is no `published -> draft` edge in this gear, so a first-publish draft
/// stays `draft` and a published head keeps its pending edits unpublished.
///
/// # A rejection is refused on a `satisfied` record rather than silently not
/// finalizing
///
/// §4 row 5 closes the machine — *"No transition other than those above is
/// admitted"* — and there is no `satisfied -> rejected` edge. Appending the
/// row without the flip would leave a recorded rejection against a record the
/// gate would still authorize, so the fail-closed arm is to refuse the
/// verdict. An **approval** on a `satisfied` record is admitted, because it
/// adds a signature and moves no state. No writer produces `satisfied` at
/// this commit (§7 row 11), so this arm is reachable only from a hand-written
/// setup today — which is why the probe writes one rather than waiting for a
/// door.
///
/// # Errors
///
/// [`ApprovalStoreError::Refused`] with `SELF_APPROVAL_FORBIDDEN` (403) when
/// the deciding principal submitted the record, and with
/// `APPROVAL_SUPERSEDED` (409) when the record is no longer open.
/// [`ApprovalStoreError::Repo`] on a storage or scope failure, on a
/// `quorum_descriptor` this gear wrote wrong ([`RepoError::CorruptRow`]),
/// and on the four refusals `design/05` §3.3 declares no code for — a
/// principal mismatch, a decision on a record admitting none, a second
/// verdict from one principal, and a rejection of an already-`satisfied`
/// record.
pub async fn record_decision(
    runner: &impl DBRunner,
    scope: &AccessScope,
    new: NewDecision<'_>,
    acting_principal: Uuid,
    decided_at: DateTime<Utc>,
) -> Result<DecisionOutcome, ApprovalStoreError> {
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
    // §4 row 5 admits no `satisfied -> rejected` edge, and a rejection row
    // that finalized nothing would sit against a record the gate still
    // authorizes. Refusing is the fail-closed arm; an approval here is fine.
    if new.verdict == DecisionVerdict::Rejected && record.state == "satisfied" {
        return Err(ApprovalStoreError::Repo(RepoError::Db(format!(
            "approval {} is satisfied: design/05 section 4 admits no satisfied -> rejected edge,              so a rejection here would append a row that finalizes nothing and leave the record              authorizable",
            new.approval_id
        ))));
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

    if new.verdict == DecisionVerdict::Rejected {
        finalize_rejected(runner, scope, new.tenant_id, new.approval_id, decided_at).await?;
        return Ok(DecisionOutcome::Finalized);
    }
    Ok(DecisionOutcome::Appended)
}

/// What a recorded decision did to the record.
///
/// Returned rather than left for the caller to read back, because the door
/// that emits `ApprovalDecided` needs to know which verdict it is announcing
/// and a second read could see a peer's supersession instead.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DecisionOutcome {
    /// The row landed and the record stays open for further verdicts.
    Appended,
    /// The row landed and the record finalized `rejected` in the same
    /// transaction (`inst-ap-edge-reject`).
    Finalized,
}

/// Flip a `pending` record to `rejected`, stamping `finalized_at`.
///
/// # The open-state predicate is on the `UPDATE`, not only on the read above
///
/// This is `supersede_open_approval`'s lesson applied rather than
/// rediscovered. That function's first build filtered by id alone, with the
/// predicate sitting on the preceding read: two concurrent writes both saw
/// the open record, the winner finalized it, and the loser met the
/// append-only trigger — *a legal act answering 500*, found by three
/// independent review lenses. Here the racer is a frozen-content write
/// superseding the record between this function's caller reading it and this
/// statement running. With the predicate on the write, the loser matches zero
/// rows and says so.
///
/// **Zero rows is a refusal here, not a no-op**, which is where this differs
/// from the supersede: there the write is legal whether or not a ceremony was
/// open, so nothing-matched means "nothing was open". Here a decision row has
/// **already been appended** in this transaction, so a record that moved out
/// from under the flip would leave that row against an unfinalized record.
/// The caller's transaction must roll both back.
///
/// `finalized_at` is written with the state because
/// `chk_products_approval_finalized` pins the pair —
/// `(state IN ('pending','satisfied')) = (finalized_at IS NULL)` — so a flip
/// that set one without the other is refused by the engine on both dialects.
///
/// # Errors
///
/// [`ApprovalStoreError::Repo`] on a storage or scope failure, and when the
/// record was no longer `pending` by the time the flip ran.
async fn finalize_rejected(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    approval_id: ApprovalId,
    now: DateTime<Utc>,
) -> Result<(), ApprovalStoreError> {
    let outcome = approval::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(approval::Column::State, Expr::value("rejected".to_owned()))
        .col_expr(approval::Column::FinalizedAt, Expr::value(now))
        .filter(
            Condition::all()
                .add(approval::Column::TenantId.eq(tenant_id))
                .add(approval::Column::ApprovalId.eq(approval_id.get()))
                // The predicate belongs HERE as well as on the read.
                .add(approval::Column::State.eq("pending")),
        )
        .exec(runner)
        .await
        .map_err(|e| {
            ApprovalStoreError::Repo(driver_failure(
                format!("finalize rejection of {approval_id}"),
                e,
            ))
        })?;
    if outcome.rows_affected == 0 {
        return Err(ApprovalStoreError::Repo(RepoError::Db(format!(
            "approval {approval_id} left the pending state before its rejection could finalize:              the decision row appended in this transaction must roll back with it"
        ))));
    }
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

/// Spend a `satisfied` record: flip it `consumed`, once
/// (`inst-gv-one-shot`, `inst-fd-publish-consume`; `dod-one-shot-consumption`).
///
/// # This function opens no transaction — `runner` MUST be the act's own
///
/// `inst-fd-publish-consume` requires the flip *"in the same transaction as
/// the authorized act"*, and `inst-gv-one-shot` adds that *"a failed attempt
/// consumes nothing"*. Neither is expressible here: this function writes one
/// row, and it is the caller's transaction that makes the pair atomic. On a
/// plain connection a committed consume followed by a failed act leaves a
/// record spent for an act that never happened, which is the one-shot rule
/// inverted.
///
/// # The one-shot is the `UPDATE`'s own predicate
///
/// `state = 'satisfied'` sits on the write, not merely on a preceding read,
/// so two acts racing off one record produce exactly one
/// [`Consumption::Spent`]. That is what makes the `DoD`'s *"two publishes off
/// one satisfied approval, the second fails"* a property of the statement
/// rather than of the order two callers happened to run in — and it is
/// `supersede_open_approval`'s own lesson, whose first build put the
/// predicate on the read alone and answered a 500 for a legal act.
///
/// Zero rows matched is [`Consumption::AlreadySpentOrClosed`] rather than an
/// error, because the caller is the one that knows what to do with it: a
/// second publish must refuse, while a `PreAuthorized` stage that raced a
/// peer is looking at the answer it wanted. Reporting it as a driver failure
/// would send both to a 500.
///
/// `finalized_at` is written with the state because
/// `chk_products_approval_finalized` pins the pair on both dialects.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn consume_approval(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    approval_id: ApprovalId,
    now: DateTime<Utc>,
) -> Result<Consumption, RepoError> {
    let outcome = approval::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(approval::Column::State, Expr::value("consumed".to_owned()))
        .col_expr(approval::Column::FinalizedAt, Expr::value(now))
        .filter(
            Condition::all()
                .add(approval::Column::TenantId.eq(tenant_id))
                .add(approval::Column::ApprovalId.eq(approval_id.get()))
                // The one-shot, and it belongs on the write.
                .add(approval::Column::State.eq("satisfied")),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("consume approval {approval_id}"), e))?;
    if outcome.rows_affected == 0 {
        return Ok(Consumption::AlreadySpentOrClosed);
    }
    Ok(Consumption::Spent)
}

/// What a consume attempt found.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Consumption {
    /// This call spent the record. Exactly one caller ever sees this for a
    /// given record.
    Spent,
    /// The record was not `satisfied` when the statement ran — already
    /// `consumed`, or finalized `rejected`/`superseded` under the act.
    AlreadySpentOrClosed,
}

/// Read the gate's candidate records for one subject
/// (`domain::approval::StoredApprovalGate`'s operand, §7 row 28's first arm).
///
/// # Why every state, and not just `satisfied`
///
/// [`crate::domain::governance::GateMode::PreAuthorized`] names a **`consumed`**
/// record, so a reader
/// scoped to `satisfied` would make that mode unanswerable — which is how the
/// mode came to have no call path at all. The host filters by state itself,
/// and it is an exhaustive match over [`ApprovalState`], so a state added to
/// the `CHECK` forces an arm there rather than being silently dropped here.
///
/// The rows are ordered newest-submission-first so a subject with a history
/// of consumed records presents its most recent one first; the host matches
/// on id and revision, so the order is a courtesy to a reader rather than
/// part of any rule.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure, and [`RepoError::CorruptRow`]
/// for a `state` outside the `CHECK`'s roster — a row this gear wrote wrong,
/// never a request-borne value.
pub async fn gate_candidates(
    runner: &impl DBRunner,
    scope: &AccessScope,
    subject: &GateSubject,
) -> Result<Vec<CandidateApproval>, RepoError> {
    let rows = approval::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(approval::Column::TenantId.eq(subject.tenant_id))
                .add(approval::Column::SubjectKind.eq(subject.kind.as_str()))
                .add(approval::Column::SubjectRef.eq(subject.reference.clone())),
        )
        .order_by(approval::Column::SubmittedAt, sea_orm::Order::Desc)
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("gate candidates for {}", subject.reference), e))?;

    let mut candidates = Vec::with_capacity(rows.len());
    for row in rows {
        let state = ApprovalState::parse(&row.state).map_err(|token| {
            RepoError::CorruptRow(format!(
                "approval {} carries state {token}, which is outside \
                 chk_products_approval_state's roster",
                row.approval_id
            ))
        })?;
        candidates.push(CandidateApproval {
            approval_id: ApprovalId::new(row.approval_id),
            subject: subject.clone(),
            internal_revision: row.internal_revision,
            state,
            // "An acknowledgment was stored", on either of its two homes —
            // the author's column at effective quorum zero, or any approver's
            // decision row above it. The by-name half has no operand; see
            // `CandidateApproval::override_acknowledged`.
            override_acknowledged: row.author_override_ack.is_some()
                || decision_ack_exists(runner, scope, subject.tenant_id, row.approval_id).await?,
        });
    }
    Ok(candidates)
}

/// Whether any decision row on this record stored an acknowledgment.
async fn decision_ack_exists(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    approval_id: Uuid,
) -> Result<bool, RepoError> {
    let found = approval_decision::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(approval_decision::Column::TenantId.eq(tenant_id))
                .add(approval_decision::Column::ApprovalId.eq(approval_id))
                .add(approval_decision::Column::OverrideAcknowledgments.is_not_null()),
        )
        .one(runner)
        .await
        .map_err(|e| driver_failure(format!("override acknowledgments of {approval_id}"), e))?;
    Ok(found.is_some())
}

// ---------------------------------------------------------------------------
// Break-glass: the elevation session's open, and the expiry that emits once
// (`design/05` `inst-bg-open`, `inst-bg-expiry`; **P-D-68** arms 2 and 3).
// ---------------------------------------------------------------------------

/// Which of `inst-bg-open`'s two approval paths an elevation took.
///
/// **One ceremony, two timings** (**P-D-68** arm 3): rule 1's
/// *"two-person-approved **or** post-hoc-reviewed"* is not two ceremonies but
/// one whose second principal may arrive late. So the two arms are exclusive
/// — `chk_products_breakglass_path` enforces that with
/// `(two_person_approval_ref IS NULL) <> (posthoc_state IS NULL)` — and the
/// post-hoc arm is discharged by that second principal's decision rather than
/// by a new door.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ApprovalPath {
    /// The second platform principal approved **before** the session opened.
    ///
    /// The reference carries **no FK**: whether its referent is an
    /// `ApprovalRecord` is `features/governance.md` §7 row 9, which P-D-68
    /// arm 3 deliberately did not presuppose, and the precedent is this
    /// gear's own `products_bulk_batch.approval_ref`.
    TwoPerson(Uuid),
    /// The obligation is recorded `pending` and the second principal reviews
    /// after the fact.
    PostHoc,
}

/// One elevation, as the store needs it.
///
/// A struct rather than six arguments because two pairs are mutually
/// assignable and a transposition would compile: the `Uuid`s `session_id`,
/// `principal` and `target_tenant`, and the two instants bounding the window.
#[derive(Copy, Clone, Debug)]
pub struct NewElevation {
    /// The session's own id.
    pub session_id: Uuid,
    /// The acting platform principal, pseudonymous from birth.
    pub principal: Uuid,
    /// The tenant whose data the session reaches.
    pub target_tenant: Uuid,
    /// The window's start, inclusive.
    pub valid_from: DateTime<Utc>,
    /// The window's end, **exclusive** — the interval is half-open, because
    /// expiry gates admission and an act admitted inside it finishes
    /// (P-D-68 arm 2).
    pub valid_until: DateTime<Utc>,
    /// Which approval path was taken.
    pub path: ApprovalPath,
    /// When the session opened.
    pub opened_at: DateTime<Utc>,
}

/// Open an elevation session (`inst-bg-open`).
///
/// # What the engine refuses, so this function does not restate it
///
/// The reason's presence is `chk_products_breakglass_reason` (`reason <> ''`),
/// the window's ordering is `chk_products_breakglass_window`
/// (`valid_until > valid_from`), the two paths' exclusivity is
/// `chk_products_breakglass_path`, and the reviewed triple is
/// `chk_products_breakglass_review` — all on **both** dialects. A guard here
/// would be a second answer to a question the schema already answers, and the
/// two could drift; what this function does is make the paths unrepresentable
/// wrongly in the first place, via [`ApprovalPath`].
///
/// # What is deliberately absent
///
/// **The alert.** `dod-breakglass-open` requires `BreakGlassElevated` *"and a
/// distinct alert channel"*, and *"a failed alert emission MUST NOT leave a
/// silent session"* — either the elevation is refused or it opens carrying a
/// recorded undelivered-alert obligation. Neither the event type nor an alert
/// channel exists in the gear, and no column holds an undelivered-alert
/// obligation, so this function opens the session and the obligation is
/// `dod-governance-events`' patch plus a missing artifact. **The window's
/// value is the caller's**: its interim 4 hours and the no-renewal rule live
/// only in the PRD's §17.1 interim-policy table (§7 row 22), and
/// `inst-bg-open` states neither, so nothing is defaulted here.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure, including a `CHECK` this
/// function does not pre-empt.
pub async fn open_breakglass_session(
    runner: &impl DBRunner,
    scope: &AccessScope,
    new: NewElevation,
    reason: &str,
) -> Result<(), RepoError> {
    let (two_person_approval_ref, posthoc_state) = match new.path {
        ApprovalPath::TwoPerson(reference) => (Some(reference), None),
        ApprovalPath::PostHoc => (None, Some("pending".to_owned())),
    };
    let model = breakglass_session::ActiveModel {
        session_id: Set(new.session_id),
        principal: Set(new.principal),
        target_tenant: Set(new.target_tenant),
        reason: Set(reason.to_owned()),
        valid_from: Set(new.valid_from),
        valid_until: Set(new.valid_until),
        two_person_approval_ref: Set(two_person_approval_ref),
        posthoc_state: Set(posthoc_state),
        reviewed_by: Set(None),
        reviewed_at: Set(None),
        expired_emitted: Set(false),
        opened_at: Set(new.opened_at),
    };
    breakglass_session::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| driver_failure(format!("elevation scope of {}", new.target_tenant), e))?
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("open elevation {}", new.session_id), e))?;
    Ok(())
}

/// Whether an elevated call is admitted, and who emits `BreakGlassExpired`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Elevation {
    /// Inside the window. **Expiry gates admission, not completion**
    /// (P-D-68 arm 2), so a call admitted here finishes even if the window
    /// closes under it.
    Admitted,
    /// Before `valid_from`.
    ///
    /// Its own arm rather than folded into [`Self::Expired`], which would
    /// emit `BreakGlassExpired` for a session that has not begun. Unreachable
    /// while every caller opens with `valid_from = opened_at`, but the column
    /// admits a future instant and a **total** function over the interval is
    /// what stops a `<` silently becoming a `<=` at the other boundary.
    NotYetValid,
    /// Past the window: the call is refused `BREAKGLASS_EXPIRED`.
    Expired {
        /// `true` for **exactly one** caller per session — the winner of the
        /// CAS on `expired_emitted`. A replay emits nothing, and a session
        /// never called after expiry emits no event at all, its expiry being
        /// a stored fact a gauge observes (P-D-68 arm 2, on P-D-54's and
        /// P-D-59's mechanisms).
        emit_expired: bool,
    },
}

/// Judge an elevated call against its session's window, flipping the
/// `expired_emitted` stamp for the one caller that emits (`inst-bg-expiry`,
/// **P-D-68** arm 2).
///
/// # This function opens no transaction — `runner` MUST be the refusal's own
///
/// P-D-68 puts the CAS *"in the same transaction as that refusal"*. The flip
/// here is one statement; only the caller's transaction makes it atomic with
/// the refusal it accompanies, and a committed flip beside a rolled-back
/// refusal is the exactly-once guarantee inverted — the event announced and
/// the refusal never delivered.
///
/// # The CAS is the `UPDATE`'s own predicate
///
/// `expired_emitted = false` sits on the write. Ten calls after expiry
/// therefore produce one `emit_expired: true` and nine `false`, whatever
/// order they arrive in — which is the whole of what item 19 asked and P-D-68
/// answered. Reading the column and then writing it would give ten emissions
/// under contention, the defect the item names in its own words.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure, and [`RepoError::Db`] when no
/// session with that id is visible under `scope` — a caller naming a session
/// of another tenant sees the same answer as one naming a session that does
/// not exist, which is the tenant-scoping boundary and not an accident.
pub async fn admit_elevated_call(
    runner: &impl DBRunner,
    scope: &AccessScope,
    session_id: Uuid,
    now: DateTime<Utc>,
) -> Result<Elevation, RepoError> {
    let session = breakglass_session::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(breakglass_session::Column::SessionId.eq(session_id)))
        .one(runner)
        .await
        .map_err(|e| driver_failure(format!("read elevation {session_id}"), e))?
        .ok_or_else(|| RepoError::Db(format!("no elevation session {session_id} in scope")))?;

    if now < session.valid_from {
        return Ok(Elevation::NotYetValid);
    }
    // Half-open `[from, until)`: the instant `valid_until` is already outside.
    if now < session.valid_until {
        return Ok(Elevation::Admitted);
    }

    let flipped = breakglass_session::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            breakglass_session::Column::ExpiredEmitted,
            Expr::value(true),
        )
        .filter(
            Condition::all()
                .add(breakglass_session::Column::SessionId.eq(session_id))
                // The CAS, and it belongs on the write.
                .add(breakglass_session::Column::ExpiredEmitted.eq(false)),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("expiry stamp of {session_id}"), e))?;
    Ok(Elevation::Expired {
        emit_expired: flipped.rows_affected == 1,
    })
}

/// Discharge a `pending` post-hoc obligation with the second platform
/// principal's late decision (**P-D-68** arm 3).
///
/// No new door and no new grant: this is the second principal of the *same*
/// ceremony, arriving after the fact. The `pending` predicate is on the
/// `UPDATE`, so two reviewers racing produce one discharge; zero rows means
/// the obligation was already discharged or the session took the two-person
/// path, and both are the caller's to interpret rather than errors.
///
/// `reviewed_by` and `reviewed_at` are written with the state because
/// `chk_products_breakglass_review` pins the triple on both dialects.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn discharge_posthoc_review(
    runner: &impl DBRunner,
    scope: &AccessScope,
    session_id: Uuid,
    reviewed_by: Uuid,
    reviewed_at: DateTime<Utc>,
) -> Result<bool, RepoError> {
    let outcome = breakglass_session::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            breakglass_session::Column::PosthocState,
            Expr::value("reviewed".to_owned()),
        )
        .col_expr(
            breakglass_session::Column::ReviewedBy,
            Expr::value(reviewed_by),
        )
        .col_expr(
            breakglass_session::Column::ReviewedAt,
            Expr::value(reviewed_at),
        )
        .filter(
            Condition::all()
                .add(breakglass_session::Column::SessionId.eq(session_id))
                .add(breakglass_session::Column::PosthocState.eq("pending")),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("discharge review of {session_id}"), e))?;
    Ok(outcome.rows_affected == 1)
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
