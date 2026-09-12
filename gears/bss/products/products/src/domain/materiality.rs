//! Materiality — the four declared inputs, judged **once** at submission
//! (`design/05-governance.md` §3.1: `inst-mt-inputs`,
//! `inst-mt-policy-material`, `inst-mt-once`).
//!
//! # The inputs arrive as resolutions, so no call site can default its way in
//!
//! `dod-materiality-evaluator` requires every input to **fail closed**: an
//! unresolvable policy or bucket registry refuses the act rather than falling
//! back to a default. An `Option` argument invites `unwrap_or_default()`, and
//! the `DoD` names the exact damage that would do — *"a policy resolving to
//! absent-implies-default at floor 0 would publish a finance-material change
//! on one signature"*. So the one looked-up input arrives as [`Resolution`],
//! whose only escape is `?` on a refusal, and the bucket registry already
//! refuses inside [`crate::domain::bucket::classify`], which answers
//! `IllegalFieldMutation` for a column carrying no tag rather than routing it
//! to a default bucket.
//!
//! # The claim set is not an input, and the clause guarding it is withdrawn
//!
//! **P-D-119 row 36.** `verdict` required a claim set to resolve and then
//! discarded it — `let _claims = …` — so the fail-closed clause on it guarded
//! an input nothing read, and a guard on nothing is not safety. C8 says why it
//! never mattered: a role predicate narrows *who may approve*, which is
//! decided at **decide** time against the approver's claims, not at submission
//! against the submitter's. The evaluator takes the policy alone.
//!
//! # The policy is an argument, never a lookup
//!
//! `inst-mt-once` fixes the instant: *"Evaluated once at submission against
//! the policy in force at the submission instant (never the reader's
//! clock)"*. A module that resolved the policy itself would read whatever is
//! current when it happens to run, so [`MaterialityEvaluator::verdict`] takes the policy the
//! submitting transaction read and holds no resolver of its own. The
//! *once* half is the submit path's:
//! [`crate::infra::storage::repo::submit_approval`] is the only caller, and
//! it stores the verdict's **effect** — the descriptor's counts — so no
//! later reader re-evaluates. It does not store the verdict itself: the
//! descriptor's five names carry no `Materiality`, and at `N <= 1` both
//! verdicts render identically. A caller needing the verdict takes it from
//! that call's answer.
//!
//! # Refusal is a domain type here, because the code taxonomy is closed
//!
//! The fail-closed arm has **no declared code**: the gear's 503 set is closed
//! at three by name — `AUDIT_UNAVAILABLE`, 08's `READ_MODEL_OVERLOADED` and
//! 03's `USAGE_TYPE_UNAVAILABLE` (`design/01-foundation.md` §3.3, whose own
//! line reads *"one of the gear's **three** 503s"*; the `§4.4` in that
//! sentence points at where the audit-row rule lives, not at the census,
//! `design/12-consumer-contracts.md` `inst-cc-errors`) — and 05 §3.3 names
//! none for an unresolvable governance input. Minting a fourth would make a
//! closed roster consistent and wrong, so the refusal is
//! [`MaterialityUnresolved`], a domain value that reaches no wire, and the
//! missing code is registered as an open item rather than invented.
//!
//! # The policy's store, and where the default now comes from
//!
//! `inst-mt-policy-material` makes the policy object a `GovernedLiveOp`
//! subject on its **own** pair `materiality_policy × write`, and that pair now
//! has both a table and a route: `products_materiality_policy` (P-D-112 arm 1)
//! and `PUT /bss-products/v1/materiality-policy`. An earlier revision of this
//! doc said no table held it and no route was declared; both were true when
//! written and neither is now.
//!
//! **An absent row is not an absent policy** (P-D-112 arm 2): it resolves to
//! [`MaterialityPolicy::default`], and only a *failed read* is
//! [`Resolution::Unresolvable`]. P-D-135 reads
//! `dod-materiality-policy`'s *"initial value from tenant provisioning"* as
//! exactly that default — P-D-104 withdrew the tenant registry the
//! provisioning would have run from, and a tenant's initial `N` **is** the
//! default until the tenant configures one.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-materiality-evaluator:p1
//! @cpt-cf-bss-products-dod-materiality-policy

use bss_products_sdk::models::{EntityKind, LifecycleState};
use toolkit_macros::domain_model;

use crate::domain::bucket::{self, FieldBucket};
use crate::domain::error::DomainError;

/// `N`'s default — two, the retained name behind `quorumReduced`
/// (`design/05` C1, P-D-11).
pub const DEFAULT_APPROVER_COUNT: u32 = 2;

/// `N`'s floor. **Zero is reachable** (P-D-11) and only by explicit
/// configuration; it is safe by reason, evidence and tripwire rather than by
/// a count, which is why nothing here clamps it upward.
///
/// It is deliberately **not** asserted in [`MaterialityPolicy::new`]: at
/// zero, `approver_count >= APPROVER_COUNT_FLOOR` is a tautology for a
/// `u32`, and an always-true guard reads as a constraint while enforcing
/// nothing. The constant's job is to give the number one name, and
/// `n_defaults_to_two_and_zero_is_reachable` is what reads it — a probe
/// spelling the literal `0` would let the floor drift in prose.
pub const APPROVER_COUNT_FLOOR: u32 = 0;

/// The affected-entity trigger's interim value for batch acts
/// (`inst-mt-inputs` input (c), 09's unit).
pub const DEFAULT_AFFECTED_ENTITY_TRIGGER: u32 = 10;

/// The verdict. Two values, because `inst-gv-materiality` makes materiality
/// decide the quorum *count* and never whether a ceremony happens.
#[domain_model]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Materiality {
    /// The quorum descriptor follows C1's material rules.
    Material,
    /// `min(N, 1)` — one approver at the default.
    NonMaterial,
}

/// Which of the evaluator's looked-up inputs could not be resolved.
///
/// **One member, and it stays an enum.** P-D-119 row 36 withdrew the claim
/// set, which was the second; a bare marker would say the same thing and would
/// stop naming *which* input failed the moment a second lookup arrives. The
/// registry's own failure is not a member — an untagged column is a
/// registration bug in the slice that owns it, and it arrives as
/// [`MaterialityRefusal::Registry`].
#[domain_model]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MaterialityInput {
    /// The tenant's materiality policy — field set, trigger and `N`. Absent
    /// is **not** unresolved: an absent row is the default (P-D-112 arm 2),
    /// and only a failed read reaches here.
    Policy,
}

impl MaterialityInput {
    /// The input's stable spelling, for a refusal's detail and for audit.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Policy => "materiality_policy",
        }
    }
}

/// A looked-up input, resolved or not.
///
/// Deliberately not `Option`: an `Option` has `unwrap_or_default`, and the
/// whole point of the fail-closed clause is that no call site may reach a
/// default. The only way out of an [`Self::Unresolvable`] is a refusal.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Resolution<T> {
    /// The lookup answered.
    Resolved(T),
    /// The lookup did not answer. The act is refused.
    Unresolvable,
}

impl<T> Resolution<T> {
    /// The resolved value, or the refusal naming which input was missing.
    pub fn require(self, input: MaterialityInput) -> Result<T, MaterialityUnresolved> {
        match self {
            Self::Resolved(value) => Ok(value),
            Self::Unresolvable => Err(MaterialityUnresolved { input }),
        }
    }
}

/// The fail-closed refusal: an input the evaluator needs did not resolve, so
/// the act is refused rather than judged against a default.
///
/// This carries no error code on purpose — see the module doc.
#[domain_model]
#[derive(Copy, Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("materiality input {} did not resolve: the act is refused rather than judged against a default", input.as_str())]
pub struct MaterialityUnresolved {
    /// Which input was missing.
    pub input: MaterialityInput,
}

/// The policy object — **field set + trigger + the approver count `N`**
/// (`inst-mt-policy-material`; `N` is part of the governed object because C1
/// and P-D-11 both require every later change to it to be material under the
/// then-current quorum, which only holds if `N` is inside).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterialityPolicy {
    field_set: Vec<String>,
    affected_entity_trigger: u32,
    approver_count: u32,
}

impl MaterialityPolicy {
    /// A policy from its three declared parts.
    ///
    /// `approver_count` is taken as given: the floor is zero and reaching it
    /// is explicit configuration, so clamping here would silently restore
    /// the fixed count P-D-11 retired.
    #[must_use]
    pub fn new(field_set: Vec<String>, affected_entity_trigger: u32, approver_count: u32) -> Self {
        Self {
            field_set,
            affected_entity_trigger,
            approver_count,
        }
    }

    /// The tenant's `N`.
    #[must_use]
    pub const fn approver_count(&self) -> u32 {
        self.approver_count
    }

    /// The batch-act trigger (input (c)).
    #[must_use]
    pub const fn affected_entity_trigger(&self) -> u32 {
        self.affected_entity_trigger
    }

    /// The tenant's own extra field set, for the store that persists it
    /// (**P-D-112**).
    ///
    /// A read-only view: [`Self::names_field`] is how the evaluator asks about
    /// a column, and this exists so the row can be written and read back
    /// without a second copy of the set living beside the policy.
    #[must_use]
    pub fn field_set(&self) -> &[String] {
        &self.field_set
    }

    /// Whether the policy's own extra field set names this column.
    ///
    /// The bucket registry is the primary operand; this set is the policy's
    /// tenant-configured addition to it, so a column in either is material.
    #[must_use]
    pub fn names_field(&self, column: &str) -> bool {
        self.field_set.iter().any(|f| f == column)
    }
}

impl Default for MaterialityPolicy {
    /// The provisioning default: no extra fields, the interim trigger, `N` at
    /// two. `Default` is the shape tenant provisioning writes, **never** a
    /// fallback the evaluator reaches — [`MaterialityEvaluator::verdict`] refuses an unresolved
    /// policy instead of constructing this.
    fn default() -> Self {
        Self {
            field_set: Vec::new(),
            affected_entity_trigger: DEFAULT_AFFECTED_ENTITY_TRIGGER,
            approver_count: DEFAULT_APPROVER_COUNT,
        }
    }
}

/// The PRD-enumerated ops (input (b)) — the FR's exact enumeration.
#[domain_model]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EnumeratedOp {
    /// A lifecycle transition **to** `published`, `deprecated` or `retired`.
    /// `draft → discarded` is outside the enumeration and stays ungated
    /// beyond its own authz (M-1), which is why this arm carries the target
    /// state rather than a bare marker.
    LifecycleTransition(LifecycleState),
    /// Category create, rename, re-parent, retire or delete.
    CategoryOp,
    /// A material attribute-definition change.
    AttributeDefinitionChange,
}

/// The `GovernedLiveOp` kinds their owning slice registered material
/// (input (d), the H-1 fix).
///
/// A closed roster of six, one per owning slice — and the roster, not the
/// type, is what closes it: [`MaterialAct::LiveOp`] accepts any variant of
/// this enum, so "an unregistered kind cannot reach the evaluator" holds
/// only because every variant *is* registered. That is asserted rather than
/// assumed; see [`Self::ALL`].
#[domain_model]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MaterialLiveOp {
    /// 02's taxonomy ops (the enumeration).
    TaxonomyOp,
    /// 03's recognized-set add/deprecate/remove and `PlanTier` taxonomy ops.
    RecognizedSetOp,
    /// 04's `ScheduledTransition` **cancel** ops — the governed retirement
    /// abort (`inst-lc-undeprecate`). Without this line the evaluator judged
    /// it non-material and `inst-gv-materiality` would set
    /// `required = min(N, 1)` for the only act that unwinds a cascade.
    ScheduledTransitionCancel,
    /// 06's freeze-participant membership ops.
    FreezeParticipantOp,
    /// 07's reference-producer registration ops.
    ReferenceProducerOp,
    /// 10's PII-allow-list ops.
    PiiAllowListOp,
}

impl MaterialLiveOp {
    /// The whole roster, in slice order.
    ///
    /// **A seventh variant cannot land silently**: [`Self::owning_slice`] is
    /// an exhaustive match, so a new kind forces an arm there, and
    /// `every_variant_is_in_the_roster` matches every variant and asserts
    /// `ALL` contains it. A `len()` assertion alone proves nothing — the
    /// array's own type gives the length at compile time.
    pub const ALL: [Self; 6] = [
        Self::TaxonomyOp,
        Self::RecognizedSetOp,
        Self::ScheduledTransitionCancel,
        Self::FreezeParticipantOp,
        Self::ReferenceProducerOp,
        Self::PiiAllowListOp,
    ];

    /// Whether this kind has a display label to rename (**P-D-121** row 17).
    ///
    /// Two of the six do. A `FreezeParticipantOp` names a participant, a
    /// `ReferenceProducerOp` a producer, a `ScheduledTransitionCancel` a
    /// scheduled row and a `PiiAllowListOp` an allow-list entry; none of them
    /// carries an operator-facing label, so the exception has nothing to apply
    /// to and [`live_op_verdict`] leaves the registration standing. Exhaustive
    /// rather than a two-name `matches!`, so a seventh kind must say which
    /// side it is on.
    #[must_use]
    pub const fn bears_display_label(self) -> bool {
        match self {
            // `02`'s attribute definitions (P-D-108 arm 2) and `03`'s
            // `PlanTier` members (P-D-121 row 17).
            Self::TaxonomyOp | Self::RecognizedSetOp => true,
            Self::ScheduledTransitionCancel
            | Self::FreezeParticipantOp
            | Self::ReferenceProducerOp
            | Self::PiiAllowListOp => false,
        }
    }

    /// The slice that registered the kind, for audit and for the census.
    #[must_use]
    pub const fn owning_slice(self) -> &'static str {
        match self {
            Self::TaxonomyOp => "02",
            Self::RecognizedSetOp => "03",
            Self::ScheduledTransitionCancel => "04",
            Self::FreezeParticipantOp => "06",
            Self::ReferenceProducerOp => "07",
            Self::PiiAllowListOp => "10",
        }
    }
}

/// Which edit a registered `GovernedLiveOp` kind carries — input (d)'s one
/// exception (**P-D-121** row 17, on **P-D-108** arm 2's operand).
///
/// # Why the exception is here and not in the two slices that raise it
///
/// The decision's own words: *"one exception stated once, not two slices
/// reading one sentence two ways"*. `02` renames an attribute definition's
/// display label and `03` renames a `PlanTier` member's, and both arrive as
/// the same kind of envelope; a carve-out written in each slice is a carve-out
/// that can drift. `05` registers what is material, so `05` registers the
/// exception.
///
/// # Why a label rename is not the thing it labels
///
/// P-D-108 arm 2 measured it: a definition has **no label column**. The label
/// is an attribute *value* on the definition (`entity_kind =
/// 'attribute_definition'`, the seeded `displayName` key), resolved through
/// the same localized chain every other display name uses. Renaming it changes
/// what an operator reads and changes nothing a consumer contracts on — so it
/// takes `min(N, 1)` rather than `N`. **Non-material is not ungated**: it is
/// still a ceremony, still an `ApprovalRecord`, still one approver from the
/// base role set (§7 row 16).
#[domain_model]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LiveOpEdit {
    /// Anything the kind's own slice governs. The registration stands.
    Registered,
    /// A display-label rename. Non-material **where the kind bears a label**
    /// — see [`MaterialLiveOp::bears_display_label`].
    DisplayLabelRename,
}

/// The act under judgement, in the shape of the input that decides it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MaterialAct<'a> {
    /// An entity re-publish, judged by the buckets of the columns it touched
    /// (input (a)).
    EntityPublish {
        /// Which head the columns belong to.
        kind: EntityKind,
        /// The touched column names.
        touched: &'a [&'a str],
    },
    /// One of the PRD-enumerated ops (input (b)).
    Enumerated(EnumeratedOp),
    /// A batch act, judged against the configured trigger (input (c)).
    BatchAct {
        /// How many entities the batch affects.
        affected: u32,
    },
    /// A registered material `GovernedLiveOp` kind (input (d)), and which
    /// edit inside that kind the envelope carries — the one exception the
    /// registration admits (**P-D-121** row 17).
    LiveOp {
        /// The registered kind.
        kind: MaterialLiveOp,
        /// Which edit the envelope carries.
        edit: LiveOpEdit,
    },
    /// The policy object's own mutation — **always** material, in either
    /// direction (C4, `inst-mt-policy-material`).
    PolicyMutation,
}

/// The evaluator, over `inst-mt-inputs`' four declared inputs.
///
/// `inst-gv-materiality` is what names the act: *"Submission runs
/// `MaterialityEvaluator` over the change set"*. `inst-mt-inputs` enumerates
/// the inputs it runs over.
///
/// It holds the two looked-up inputs as resolutions and nothing else: no
/// resolver, no clock, no store handle. That is what makes
/// [`Self::verdict`] a function of what its caller read at the submission
/// instant rather than of what is current when it runs.
#[derive(Copy, Clone, Debug)]
pub struct MaterialityEvaluator<'a> {
    policy: Resolution<&'a MaterialityPolicy>,
}

impl<'a> MaterialityEvaluator<'a> {
    /// The evaluator over the input a submission resolved.
    ///
    /// **One argument, not two.** P-D-119 row 36 withdrew the claim set; it is
    /// not a defaulted second parameter, because a parameter the evaluator
    /// ignores is exactly what the decision found.
    #[must_use]
    pub const fn new(policy: Resolution<&'a MaterialityPolicy>) -> Self {
        Self { policy }
    }

    /// Decide materiality from the four declared inputs.
    ///
    /// Every looked-up input fails closed. The bucket registry's failure arm
    /// is [`bucket::classify`]'s own: a touched column carrying no bucket
    /// tag is refused rather than treated as non-material, and that refusal
    /// surfaces here as a [`DomainError`] because the registry is code
    /// (P-D-28) and an untagged column is a registration bug rather than a
    /// missing lookup.
    ///
    /// # Errors
    ///
    /// [`MaterialityRefusal::Unresolved`] when the policy read failed;
    /// [`MaterialityRefusal::Registry`] when a touched column carries no
    /// bucket tag; [`MaterialityRefusal::CorrectableTouch`] when a bucket-ii
    /// column arrives as an ordinary touch.
    /// Takes `self` by value: with the claim set withdrawn the evaluator is
    /// one 8-byte pointer, and `clippy::trivially_copy_pass_by_ref` is right
    /// that a reference to it costs more than the copy.
    pub fn verdict(self, act: &MaterialAct<'_>) -> Result<Materiality, MaterialityRefusal> {
        // The policy is required before any arm is judged: an act that would
        // answer `Material` on its shape alone must still refuse when the read
        // failed, since the *count* the verdict feeds comes from the policy
        // and a verdict without one cannot be spent. An **absent row** never
        // reaches here as `Unresolvable` — it resolved, to the default
        // (P-D-112 arm 2).
        let policy = self.policy.require(MaterialityInput::Policy)?;
        Self::judge(act, policy)
    }

    /// The verdict's arms, once both inputs have resolved.
    fn judge(
        act: &MaterialAct<'_>,
        policy: &MaterialityPolicy,
    ) -> Result<Materiality, MaterialityRefusal> {
        match act {
            MaterialAct::PolicyMutation => Ok(Materiality::Material),
            MaterialAct::LiveOp { kind, edit } => Ok(live_op_verdict(*kind, *edit)),
            MaterialAct::Enumerated(op) => enumerated_verdict(*op),
            MaterialAct::BatchAct { affected } => {
                if *affected >= policy.affected_entity_trigger() {
                    Ok(Materiality::Material)
                } else {
                    Ok(Materiality::NonMaterial)
                }
            }
            MaterialAct::EntityPublish { kind, touched } => touched_verdict(*kind, touched, policy),
        }
    }
}

/// Input (d)'s verdict, with the one exception the registration admits
/// (**P-D-121** row 17).
///
/// Not a `match` over the pair: two of its four arms would answer `Material`
/// and read as duplicates of each other. The exception is a single condition
/// — *a display-label rename on a kind that has one* — and everything else is
/// the registration, which is what the decision says.
const fn live_op_verdict(kind: MaterialLiveOp, edit: LiveOpEdit) -> Materiality {
    if matches!(edit, LiveOpEdit::DisplayLabelRename) && kind.bears_display_label() {
        Materiality::NonMaterial
    } else {
        Materiality::Material
    }
}

/// Input (b)'s verdict. The enumeration is the FR's exact one, so a
/// transition to `draft` or `discarded` is outside it (M-1) — and outside
/// means **refused, not non-material**.
///
/// `NonMaterial` is not "ungated": it feeds `required = min(N, 1)`, which is
/// **one** approver at the default. Answering it for `draft → discarded`
/// would mint a one-approver ceremony for the one transition M-1 leaves
/// *"ungated beyond its own authz"*. The module refuses the same way it
/// refuses a bucket-ii touch: an act that cannot arrive here does not get a
/// verdict.
fn enumerated_verdict(op: EnumeratedOp) -> Result<Materiality, MaterialityRefusal> {
    match op {
        EnumeratedOp::LifecycleTransition(to) => match to {
            LifecycleState::Published | LifecycleState::Deprecated | LifecycleState::Retired => {
                Ok(Materiality::Material)
            }
            LifecycleState::Draft | LifecycleState::Discarded => {
                Err(MaterialityRefusal::OutsideTheEnumeration(to))
            }
        },
        EnumeratedOp::CategoryOp | EnumeratedOp::AttributeDefinitionChange => {
            Ok(Materiality::Material)
        }
    }
}

/// How each bucket bears on materiality — a **total** function over the
/// registry's four tags.
///
/// It is a function of the tag rather than a branch inside the loop because
/// the `DoD`'s bucket-iv clause has no column to probe: `Descriptive` carries
/// **no registered member today** (`domain::bucket`'s own roster), so a
/// probe written over columns could only assert the buckets that happen to
/// be populated. Over the tag it is exhaustive, and it stays exhaustive when
/// bucket iv gains its first member.
#[must_use]
pub const fn bucket_bearing(bucket: FieldBucket) -> BucketBearing {
    match bucket {
        // iii is the governed-content bucket: any touch is material.
        FieldBucket::MaterialMutable => BucketBearing::Material,
        // Two tags, one bearing, for two different reasons: iv differs from
        // iii *only* in the materiality read off it, and that read is this
        // line; i is identity, refused by the head door before materiality
        // is ever asked, so reaching here it moves no verdict either.
        FieldBucket::Descriptive | FieldBucket::Structural => BucketBearing::Immaterial,
        // ii never arrives as an ordinary touch (L-1).
        FieldBucket::Correctable => BucketBearing::NotAnOrdinaryTouch,
    }
}

/// What a touched column's bucket does to the verdict.
#[domain_model]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BucketBearing {
    /// The touch makes the act material.
    Material,
    /// The touch leaves the verdict where it was.
    Immaterial,
    /// The column cannot be an ordinary touch at all — the act is refused.
    NotAnOrdinaryTouch,
}

/// Input (a)'s verdict, over the bucket registry plus the policy's own field
/// set.
///
/// A bucket-iii touch is material. A bucket-iv-only re-publish is
/// non-material. `Correctable` (bucket ii) reaching here is a **defect
/// upstream, not a verdict**: L-1 says the evaluator never sees the
/// metering-unit field as an ordinary touch, because before first publish it
/// rides 01's save door and after it only the slice-07 correction door, which
/// is `N`-governed in its own right. So the arm refuses rather than judging.
fn touched_verdict(
    kind: EntityKind,
    touched: &[&str],
    policy: &MaterialityPolicy,
) -> Result<Materiality, MaterialityRefusal> {
    let mut verdict = Materiality::NonMaterial;
    for column in touched {
        // **The registry runs first, always.** Checking `names_field`
        // before `classify` would let a tenant switch both fail-closed
        // refusals off for any column it lists: an untagged (misspelt)
        // column would stop raising `Registry`, and `metering_unit` would
        // stop raising `CorrectableTouch` — making L-1's
        // correction-door-only guarantee tenant-configurable. The policy's
        // set may raise a verdict; it may never skip a refusal.
        let class = bucket::classify(kind, column).map_err(MaterialityRefusal::Registry)?;

        // Bucket ii refuses before anything promotes: a tenant's field set
        // must not be able to make it an ordinary touch.
        if class.bucket() == Some(FieldBucket::Correctable) {
            return Err(MaterialityRefusal::CorrectableTouch((*column).to_owned()));
        }

        // **The union, and it is a union.** `names_field`'s contract is that
        // a column in *either* the registry or the tenant's set is material,
        // so the promotion runs for every non-refusing class — including the
        // bucket-less ones (`CreateOnly`, the two outside-the-scheme
        // classes). Scoping it to the `Immaterial` arm silently halved the
        // quorum for a tenant that named, say, `deprecation_provenance`:
        // `NonMaterial` gives `min(N, 1)` = one approver where the tenant
        // asked for `N`. The registry still runs FIRST, which is the
        // ordering the fail-closed arms need.
        if policy.names_field(column) {
            verdict = Materiality::Material;
            continue;
        }

        let Some(bucket) = class.bucket() else {
            // `CreateOnly` and the two outside-the-scheme classes: refused by
            // the head door's own guards, and no materiality verdict of
            // their own.
            continue;
        };
        match bucket_bearing(bucket) {
            BucketBearing::Material => verdict = Materiality::Material,
            BucketBearing::Immaterial => {}
            BucketBearing::NotAnOrdinaryTouch => {
                // Unreachable: the guard above returns first. Kept so the
                // match stays exhaustive over the bearing, not over today's
                // reachability.
                return Err(MaterialityRefusal::CorrectableTouch((*column).to_owned()));
            }
        }
    }
    Ok(verdict)
}

/// Why the evaluator refused. Three arms, because the remedies differ and a
/// single message would send every one of them to the wrong owner.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MaterialityRefusal {
    /// A looked-up input did not resolve.
    #[error(transparent)]
    Unresolved(#[from] MaterialityUnresolved),
    /// A touched column carries no bucket tag — the registry's own refusal.
    #[error("bucket registry: {0}")]
    Registry(DomainError),
    /// A bucket-ii column reached the evaluator, which L-1 says cannot
    /// happen through an ordinary touch.
    #[error(
        "column {0} is bucket ii: it reaches publish through the save door before first publish \
         and the correction door after, never as an ordinary touch (L-1)"
    )]
    CorrectableTouch(String),
    /// A lifecycle transition to a target the FR's enumeration excludes.
    /// `draft → discarded` is ungated beyond its own authz (M-1), so it has
    /// no verdict here — and `NonMaterial` would have given it a ceremony.
    #[error(
        "a transition to {0:?} is outside the FR's enumeration and ungated beyond its own authz \
         (M-1): it has no materiality verdict"
    )]
    OutsideTheEnumeration(LifecycleState),
}

#[cfg(test)]
#[path = "materiality_tests.rs"]
mod materiality_tests;
