//! The governance gate's **host contract** — the seam the publish door calls
//! and slice 05 fills (`design/01-foundation.md` §2, `inst-fd-governance-gate`
//! and its five sub-instructions).
//!
//! §1.5 puts the *contract* — `Gate` / `PreAuthorized(approvalId)` — In for
//! this slice and puts *materiality, approvals, `RBAC` grants and break-glass*
//! Out, owned by 05. So this module declares what crosses the seam, ships the
//! only host that can exist before an approval store does, and implements no
//! ceremony whatsoever. There is no materiality evaluator here, no
//! `ApprovalRecord`, no record store and no grant check, and their absence is
//! the design's, not an omission.
//!
//! # The mode is an internal argument, never a wire parameter
//!
//! The door takes the mode explicitly (`inst-fd-gate-mode`), and the owner's
//! call of 2026-08-27 pins where it may come from: **the `REST` and `SDK`
//! publish surfaces always call in [`GateMode::Gate`]**. No request field, no
//! header and no query parameter selects [`GateMode::PreAuthorized`]. The
//! re-use that mode admits is therefore bounded by the set of in-process
//! callers — `04-lifecycle`'s scheduled-publish runner, a cascade leg, a bulk
//! row (05 `inst-gv-one-shot`) — rather than by a grant a caller could hold.
//! A `PreAuthorized` publish reachable from the wire would let any caller
//! naming an approval id skip the ceremony, which is why the mode's only
//! constructor is a Rust call site.
//!
//! # What crosses the seam, and nothing else
//!
//! `inst-fd-gate-verdict` fixes the payload exactly: *"the gate answered
//! yes/no + reason, and on yes the authorizing `ApprovalRecord`'s id, plus
//! whether that record carried the two-person uncomposed-bundle override"*.
//! [`GateVerdict`] carries that and no more. The Foundation learns nothing
//! about who approved, against which materiality rule, in how many steps or
//! when; those are the ceremony's and stay inside the host.
//!
//! # Consumption is in the type, not in a caller's memory
//!
//! `inst-fd-publish-consume` requires the `satisfied` record be flipped
//! `consumed` **in the same transaction as the authorized act**, and requires
//! that **nothing is consumed under `PreAuthorized`**. A door that had to
//! remember which of those two it was in would eventually forget, so the
//! answer is [`ApprovalDisposition`]: a verdict either names a record **to
//! consume**, names one **already verified** (spend nothing), or names none
//! at all. [`GateAuthorization::approval_to_consume`] is the only route to an
//! id for the consume flip and it answers `None` on the other two, so the
//! `PreAuthorized` path cannot spend a record even by accident.
//!
//! # Re-validation is not this module's, and is fail-closed in both modes
//!
//! `inst-fd-gate-revalidation`: the mode governs *who approved*, never
//! *whether the entity is still publishable*. The full pipeline re-run
//! (`inst-fd-publish-revalidate`) is the publish door's own step 3 and runs
//! identically under both modes. Nothing in this module can weaken it, and
//! nothing here should grow a "the host already checked" short-circuit — the
//! door's re-run is what stops a stale-but-approved entity publishing.
//!
//! # A rejection flips no state
//!
//! `inst-fd-gate-rejection`: a refusal leaves a first-publish entity in
//! `draft` and leaves a published head's pending edits unpublished. There is
//! no `published -> draft` edge and this module creates none; a refusal is a
//! [`DomainError::ApprovalRequired`] and writes nothing. **No event.**
//!
//! # The default host, and why it answers what it answers
//!
//! [`NoMaterialityPolicyGate`] is what the gear runs until slice 05 registers
//! a policy. Its two answers are not the same kind of answer: one is a
//! recorded deviation from an instruction this slice cannot satisfy, the other
//! is that instruction's own fail-closed behaviour.
//!
//! - Under [`GateMode::Gate`] it **authorizes, naming no record**, and this is
//!   a **deviation from `inst-fd-gate-mode-gate`, not compliance with it**.
//!   That instruction says: *"In `Gate` mode, a publish with no satisfied,
//!   non-superseded `ApprovalRecord` pinned to the door's expected revision
//!   fails `APPROVAL_REQUIRED`"*. This host has no record store, so every
//!   publish it sees is a publish with no satisfied record, and the compliant
//!   answer is a refusal. It authorizes anyway. See
//!   [`NoMaterialityPolicyGate`] for the full argument and what is owed to
//!   slice 05.
//!
//!   The reason is **not** that no ceremony applies. `05-governance.md`
//!   `inst-gv-materiality` makes materiality decide the quorum *count*, never
//!   whether a ceremony happens — *"material ⇒ quorum descriptor per C1
//!   (`required = N`); non-material ⇒ `min(N, 1)`"* — and it puts *"first
//!   publish and every lifecycle transition to
//!   `published`/`deprecated`/`retired`"* in the material set outright. A
//!   non-material publish is still a publish with an `ApprovalRecord`, at
//!   `required = min(N, 1)`. There is no reading of the set under which an
//!   unregistered policy means an act needs no record.
//!
//!   What protects the door meanwhile is the permission check that already
//!   runs before it ([`crate::authz::access_scope`], over the catalog in
//!   [`crate::gts::permissions`]); the gate is the ceremony layer *above* that
//!   check, and it has no rules yet. That is a mitigation of the deviation,
//!   not a discharge of it.
//! - Under [`GateMode::PreAuthorized`] it **refuses**. That mode's whole
//!   contract is to *verify* that the named record authorized this subject and
//!   that its pinned revision still matches (`inst-fd-gate-mode-preauthorized`).
//!   A host holding no record store can verify nothing, and accepting an
//!   unverifiable approval id would be fail-open — precisely the failure the
//!   mode exists to avoid. The mode becomes usable when 05 supplies a host with
//!   a record store; the caller it exists for, `04-lifecycle`'s
//!   scheduled-publish runner, does not exist at this commit either, so nothing
//!   is blocked by the refusal today.
//!
//! # Deliberately synchronous
//!
//! [`GovernanceGate::evaluate`] takes no runtime: the default host reads
//! nothing and the domain layer holds no connection. A store-backed host will
//! need its candidate records to reach it either as an operand the door
//! already loaded inside its transaction, or through an async widening of this
//! signature. That choice is slice 05's and is recorded here rather than
//! guessed at, because guessing it wrong costs a signature change either way.

use uuid::Uuid;

use bss_products_sdk::models::EntityKind;
use toolkit_macros::domain_model;

use crate::domain::concurrency::InternalRevision;
use crate::domain::error::DomainError;

/// The id of an `ApprovalRecord` — slice 05's row, named here only as an
/// opaque handle.
///
/// A newtype over [`Uuid`] rather than a bare `Uuid` because it travels beside
/// three other identifiers (tenant, entity, actor) that are also `Uuid`s, and
/// the storage column it lands in is `products_entity_version.approval_ref`.
/// This module knows nothing about the record behind it and deliberately
/// declares no such struct: the record's shape is 05's.
#[domain_model]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ApprovalId(Uuid);

impl ApprovalId {
    /// Wrap a record id.
    #[must_use]
    pub const fn new(id: Uuid) -> Self {
        Self(id)
    }

    /// The underlying id, as `approval_ref` stores it.
    #[must_use]
    pub const fn get(self) -> Uuid {
        self.0
    }
}

impl core::fmt::Display for ApprovalId {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// The subject a gate question, and an invalidation hook, are asked about:
/// one head row, identified the way every Foundation table identifies one.
///
/// Carried as a value rather than as three loose arguments so a caller cannot
/// transpose the two `Uuid`s, which the compiler would otherwise accept.
#[domain_model]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntityRef {
    /// The tenant the row is scoped to.
    pub tenant_id: Uuid,
    /// Which of the two catalog entities the row is.
    pub entity_kind: EntityKind,
    /// The head row's id.
    pub entity_id: Uuid,
}

/// The five subject kinds `products_approval` records, as the **seam** now
/// expresses them (**P-D-67** arm 4).
///
/// The store fixed this vocabulary first — `kind ∈ {entity_publish,
/// governed_live_op, system_signal, sku_correction, bulk_batch}` — and arm 4's
/// finding was that *"the seam expressing less than the store records was the
/// defect: the store is the authority, the seam conforms"*. Before it, four of
/// the five kinds had no representation to hand `evaluate`.
///
/// An enum rather than a bare string, so a host that grows a policy for one
/// kind is forced by the compiler to say what it does with the rest. That is
/// the property the alternative — a second trait method a host could leave
/// defaulted — would have thrown away: a subject nobody wrote policy for would
/// then be authorized silently.
#[domain_model]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubjectKind {
    /// A Product or SKU publish — the kind the Foundation's own doors carry.
    EntityPublish,
    /// `02`/`03`'s live-entity operation envelope.
    GovernedLiveOp,
    /// `06`'s inbound composition clear (**P-D-14**).
    SystemSignal,
    /// `07`'s immutable-field correction.
    SkuCorrection,
    /// `09`'s batch.
    BulkBatch,
}

impl SubjectKind {
    /// The token `products_approval.subject_kind` stores.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EntityPublish => "entity_publish",
            Self::GovernedLiveOp => "governed_live_op",
            Self::SystemSignal => "system_signal",
            Self::SkuCorrection => "sku_correction",
            Self::BulkBatch => "bulk_batch",
        }
    }
}

/// What a gate question is about: the approval store's own
/// `(subject_kind, subject_ref)` pair (**P-D-67** arm 4).
///
/// `subject_ref` is textual because the five kinds do not share an id type —
/// the same reason the column is — and [`EntityRef`] remains the **constructor**
/// for the entity kinds, which is arm 4's own wording rather than a
/// convenience: a door holding a head row must not have to render an id by
/// hand and risk transposing the two `Uuid`s.
///
/// **What this type deliberately does not carry is the pinned revision.**
/// `governance` §7 row 14 asks what the entity-shaped columns — the pinned
/// revision, the content snapshot, the diff basis — hold for a subject that is
/// not an entity, and that row is live. So the revision stays a separate
/// argument of [`GovernanceGate::evaluate`], supplied by the entity doors that
/// have one; folding it in here would answer row 14 from a type definition.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GateSubject {
    /// The tenant every subject is scoped to.
    pub tenant_id: Uuid,
    /// Which of the five the subject is.
    pub kind: SubjectKind,
    /// The subject's own identifier, rendered as the store stores it.
    pub reference: String,
}

impl GateSubject {
    /// The entity constructor arm 4 keeps.
    #[must_use]
    pub fn entity_publish(entity: EntityRef) -> Self {
        Self {
            tenant_id: entity.tenant_id,
            kind: SubjectKind::EntityPublish,
            reference: format!("{}/{}", entity.entity_kind.as_str(), entity.entity_id),
        }
    }

    /// A live-entity operation's subject — `02`'s envelope target.
    #[must_use]
    pub fn governed_live_op(tenant_id: Uuid, target: &str) -> Self {
        Self {
            tenant_id,
            kind: SubjectKind::GovernedLiveOp,
            reference: target.to_owned(),
        }
    }

    /// An inbound system signal's subject (**P-D-14**).
    #[must_use]
    pub fn system_signal(tenant_id: Uuid, signal_ref: &str) -> Self {
        Self {
            tenant_id,
            kind: SubjectKind::SystemSignal,
            reference: signal_ref.to_owned(),
        }
    }

    /// A SKU correction's subject.
    #[must_use]
    pub fn sku_correction(tenant_id: Uuid, sku_id: Uuid) -> Self {
        Self {
            tenant_id,
            kind: SubjectKind::SkuCorrection,
            reference: sku_id.to_string(),
        }
    }

    /// A bulk batch's subject.
    #[must_use]
    pub fn bulk_batch(tenant_id: Uuid, batch_id: Uuid) -> Self {
        Self {
            tenant_id,
            kind: SubjectKind::BulkBatch,
            reference: batch_id.to_string(),
        }
    }
}

/// The door's authorization mode (`inst-fd-gate-mode`).
///
/// **Never a wire-visible parameter.** See the module doc: the `REST` and
/// `SDK` publish surfaces always pass [`Self::Gate`], and [`Self::PreAuthorized`]
/// is reachable only from an in-process caller.
#[domain_model]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GateMode {
    /// The ordinary interactive publish. Needs a `satisfied`, non-superseded
    /// record pinned to the door's expected revision, and **consumes** it
    /// (`inst-fd-gate-mode-gate`, `inst-fd-publish-consume`).
    Gate,
    /// The mechanical stage of a composite act: a scheduled activation, a
    /// cascade leg, a bulk row. The host does not look for a `satisfied`
    /// record and **consumes nothing**; it verifies that the named record
    /// authorized *this* subject and that its pinned revision still matches
    /// (`inst-fd-gate-mode-preauthorized`).
    ///
    /// Without this mode a scheduled publish fails terminally: the runner
    /// drives the ordinary publish door, the gate inside it would find an
    /// already-`consumed` record, and 04 `inst-ar-failure` wraps that into a
    /// terminal `SCHEDULE_STALE_APPROVAL`.
    PreAuthorized(ApprovalId),
}

/// What the authorized act must do with the record behind a `yes`.
///
/// The three states are exactly the three the seam can produce, and they are
/// distinct **types of answer**, not a nullable id plus a flag: an id that may
/// be spent, an id that may not, and no id at all.
#[domain_model]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApprovalDisposition {
    /// No record authorized this act, because none was required. The only
    /// answer [`NoMaterialityPolicyGate`] can give: nothing to consume,
    /// nothing to store in `approval_ref`.
    NoRecord,
    /// A `satisfied` record authorized this act and **must be flipped
    /// `consumed` in the same transaction as the act**
    /// (`inst-fd-publish-consume`, 05 `inst-gv-one-shot`).
    Consume(ApprovalId),
    /// A record was **verified**, not spent: the [`GateMode::PreAuthorized`]
    /// answer. The id is still what `approval_ref` stores, and nothing is
    /// consumed.
    Verified(ApprovalId),
}

/// A `yes` from the gate, with everything `inst-fd-gate-verdict` says crosses
/// the seam and nothing more.
#[domain_model]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateAuthorization {
    /// Which record, if any, authorized the act, and whether the act's own
    /// transaction must spend it.
    pub disposition: ApprovalDisposition,
    /// Whether that record carried the two-person uncomposed-bundle override
    /// — §4.2's `composition_pending` operand, which the publish door writes
    /// in its one head-row `UPDATE`.
    ///
    /// `false` where no record authorized the act: an override nobody granted
    /// is not one the door may apply.
    pub uncomposed_bundle_override: bool,
    /// Why the host said yes, in the host's own words. Carried on a `yes` as
    /// well as on a `no` because the act's audit row records why it was
    /// authorized, and under the default host that reason is the whole story.
    pub reason: String,
}

impl GateAuthorization {
    /// The record id the authorized act's transaction must flip `consumed`,
    /// or `None` where there is nothing to spend.
    ///
    /// The **only** route to an id for the consume flip. `None` under
    /// [`ApprovalDisposition::Verified`] is what makes "nothing is consumed
    /// under `PreAuthorized`" a property of the type rather than a rule the
    /// door has to remember.
    ///
    /// **No production caller yet.** The only host in the gear is
    /// [`NoMaterialityPolicyGate`], which answers
    /// [`ApprovalDisposition::NoRecord`] on its single authorizing arm, so
    /// every call would answer `None` and the publish door does not ask. The
    /// first reader is expected to be slice 05, whose host is the first that
    /// can return an id at all — and the flip it names,
    /// `inst-fd-publish-consume`, is 05's ceremony too. It is kept rather than
    /// deleted because it is the contract 05's host is written against: the
    /// property that a `PreAuthorized` act cannot spend a record has to exist
    /// before the code that must not violate it.
    #[must_use]
    /// **No production caller yet, measured 2026-08-30.** Every call site in
    /// the crate is a test or another accessor beside this one. It is the
    /// contract seam slice 05 needs: `inst-fd-publish-consume` requires the
    /// `satisfied` record be flipped `consumed` in the same transaction as
    /// the authorized act, and this is the only route to the id that flip
    /// takes. The first caller is therefore 05's store-backed host together
    /// with the door step that consumes what it returns. Kept rather than
    /// deleted for that reason, and said out loud rather than left for a
    /// reader to discover the method is dead.
    pub const fn approval_to_consume(&self) -> Option<ApprovalId> {
        match self.disposition {
            ApprovalDisposition::Consume(id) => Some(id),
            ApprovalDisposition::NoRecord | ApprovalDisposition::Verified(_) => None,
        }
    }

    /// The record id `products_entity_version.approval_ref` stores for this
    /// act (§4.3), which is the authorizing record under **either** mode.
    ///
    /// Distinct from [`Self::approval_to_consume`] on purpose: a
    /// `PreAuthorized` act still records which approval stands behind the
    /// frozen version, even though it spends nothing. The column is nullable
    /// precisely because [`ApprovalDisposition::NoRecord`] is reachable.
    #[must_use]
    pub const fn approval_ref(&self) -> Option<ApprovalId> {
        match self.disposition {
            ApprovalDisposition::Consume(id) | ApprovalDisposition::Verified(id) => Some(id),
            ApprovalDisposition::NoRecord => None,
        }
    }
}

/// The gate's answer: yes with an authorization, or no with a reason.
///
/// A refusal is modelled as a *verdict*, not as an `Err`, so it stays
/// distinguishable from a host that could not reach an answer at all — see
/// [`GovernanceGate::evaluate`]'s error contract. Both eventually become
/// something, and [`Self::into_authorization`] is the one place a `no` turns
/// into `APPROVAL_REQUIRED`, so no door can invent a different code for it.
#[domain_model]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateVerdict {
    /// The ceremony authorized the act.
    Authorized(GateAuthorization),
    /// The ceremony refused it. A first-publish entity stays `draft`, a
    /// published head keeps its pending edits unpublished, no state flips and
    /// no event is emitted (`inst-fd-gate-rejection`).
    Refused {
        /// Why, in the host's own words. This becomes the
        /// `APPROVAL_REQUIRED` message.
        reason: String,
    },
}

impl GateVerdict {
    /// Build a `yes`.
    ///
    /// A constructor rather than a struct literal at each host, so every host
    /// states the three operands `inst-fd-gate-verdict` names and cannot add
    /// a fourth.
    #[must_use]
    pub const fn authorized(
        disposition: ApprovalDisposition,
        uncomposed_bundle_override: bool,
        reason: String,
    ) -> Self {
        Self::Authorized(GateAuthorization {
            disposition,
            uncomposed_bundle_override,
            reason,
        })
    }

    /// Collapse the verdict into the door's own control flow.
    ///
    /// # Errors
    ///
    /// [`DomainError::ApprovalRequired`] on a refusal, carrying the host's
    /// reason. This is the single mapping site for `inst-fd-gate-mode-gate`'s
    /// `APPROVAL_REQUIRED`; a door that matched on the verdict itself could
    /// choose another code, and this method exists so none does.
    pub fn into_authorization(self) -> Result<GateAuthorization, DomainError> {
        match self {
            Self::Authorized(authorization) => Ok(authorization),
            Self::Refused { reason } => Err(DomainError::ApprovalRequired(reason)),
        }
    }
}

/// The port the publish door calls, and the whole of what the Foundation
/// knows about governance.
///
/// Slice 05 ships the host that reads `ApprovalRecord`s; this slice ships
/// [`NoMaterialityPolicyGate`] and the contract both must honour.
pub trait GovernanceGate {
    /// Ask the ceremony layer whether this act may proceed.
    ///
    /// `expected_revision` is the door's `If-Match` (P-D-33) and is not
    /// advisory: an approval is only usable against the exact revision it
    /// pinned (`inst-fd-publish-pin`), so a host with a record store matches
    /// on it rather than merely reporting it.
    ///
    /// # Errors
    ///
    /// [`DomainError`] where the host could not **reach** an answer — a
    /// record-store read that failed, say. That is not the same thing as a
    /// `no`, which is [`GateVerdict::Refused`]: a refusal is the ceremony's
    /// judgement and belongs to the caller, while a host failure is
    /// infrastructure and must not be reported as `APPROVAL_REQUIRED`, which
    /// would tell an operator an approval was missing when none was ever
    /// looked at.
    fn evaluate(
        &self,
        subject: GateSubject,
        expected_revision: InternalRevision,
        mode: GateMode,
    ) -> Result<GateVerdict, DomainError>;
}

/// The host the gear runs until slice 05 registers a materiality policy.
///
/// # Under `Gate` this host deviates from `inst-fd-gate-mode-gate`
///
/// The instruction: *"In `Gate` mode, a publish with no satisfied,
/// non-superseded `ApprovalRecord` pinned to the door's expected revision fails
/// `APPROVAL_REQUIRED`"*. This host holds no record store, so **every** publish
/// it sees is one with no satisfied record. The compliant answer is a refusal
/// on every publish; this host authorizes instead. That is a deviation, and it
/// is recorded here as one rather than argued away.
///
/// **Why the deviation is taken.** The slice that owns approvals does not
/// exist. A fail-closed host would refuse every publish the gear can currently
/// take — every one of them, since no `ApprovalRecord` can be created at this
/// commit — against a store that will never answer. That is not enforcement of
/// the instruction; it is a dead surface. §1.5 puts *"materiality, approvals,
/// `RBAC` grants, break-glass"* Out of this slice, which is why shipping the
/// contract without the ceremony is admissible at all — but Out-of-scope is
/// why the deviation is **acceptable**, not a reason the instruction does not
/// reach this host. It does reach it.
///
/// **Why "no policy is registered" is not a compliance argument.** It reads as
/// though an unregistered materiality policy meant no ceremony applied. It does
/// not: 05 `inst-gv-materiality` makes materiality decide the quorum *count* —
/// *"material ⇒ quorum descriptor per C1 (`required = N`); non-material ⇒
/// `min(N, 1)`"* — and never whether a record exists. A non-material publish
/// still gets an `ApprovalRecord`. An earlier revision of this doc made that
/// claim; it was measured against the set and withdrawn.
///
/// **Owed to slice 05.** When 05 registers its policy and its record store,
/// this host is replaced, and the replacement answers `APPROVAL_REQUIRED`
/// where `inst-fd-gate-mode-gate` says it must. Until then the door is
/// protected only by the permission check that runs before it
/// ([`crate::authz::access_scope`]), which is a different control at a
/// different layer and does not stand in for the ceremony.
///
/// # Under `PreAuthorized` it refuses, and that arm is compliant
///
/// A host with no record store can verify nothing, and accepting an
/// unverifiable approval id would be fail-open — precisely what
/// `inst-fd-gate-mode-preauthorized` exists to prevent. No deviation there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NoMaterialityPolicyGate;

/// The reason [`NoMaterialityPolicyGate`] authorizes under [`GateMode::Gate`].
///
/// Named rather than inlined so the audit row, the test that pins it and this
/// module's argument all read the same sentence.
///
/// It states the deviation rather than a justification, because this string is
/// what an operator reading an audit row sees. A sentence claiming the act
/// needed no ceremony would tell that operator something
/// `inst-gv-materiality` contradicts; a sentence naming the missing store
/// tells them what is actually true and what will change under slice 05.
const NO_POLICY_REASON: &str = "no materiality policy or approval record store is registered at this commit, \
     so this host cannot evaluate the ceremony inst-fd-gate-mode-gate requires and authorizes without one; \
     this is a deviation owed to slice 05, not a finding that the act needed no approval";

impl GovernanceGate for NoMaterialityPolicyGate {
    /// # Errors
    ///
    /// Never. This host reads nothing and so cannot fail to reach an answer;
    /// its `no` under [`GateMode::PreAuthorized`] is a verdict, not a host
    /// failure.
    fn evaluate(
        &self,
        _subject: GateSubject,
        _expected_revision: InternalRevision,
        mode: GateMode,
    ) -> Result<GateVerdict, DomainError> {
        match mode {
            GateMode::Gate => Ok(GateVerdict::authorized(
                ApprovalDisposition::NoRecord,
                false,
                NO_POLICY_REASON.to_owned(),
            )),
            GateMode::PreAuthorized(id) => Ok(GateVerdict::Refused {
                reason: format!(
                    "approval {id} cannot be verified: no approval record store is registered, \
                     and accepting an unverifiable approval id would be fail-open; \
                     slice 05 supplies the host that can verify it"
                ),
            }),
        }
    }
}

#[cfg(test)]
#[path = "governance_tests.rs"]
mod governance_tests;
