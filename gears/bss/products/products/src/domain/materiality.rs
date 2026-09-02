//! Materiality — the four declared inputs, judged **once** at submission
//! (`design/05-governance.md` §3.1: `inst-mt-inputs`,
//! `inst-mt-policy-material`, `inst-mt-once`).
//!
//! # The inputs arrive as resolutions, so no call site can default its way in
//!
//! `dod-materiality-evaluator` requires every input to **fail closed**: an
//! unresolvable policy, claim set or bucket registry refuses the act rather
//! than falling back to a default. An `Option` argument invites
//! `unwrap_or_default()`, and the `DoD` names the exact damage that would do —
//! *"a policy resolving to absent-implies-default at floor 0 would publish a
//! finance-material change on one signature"*. So the two looked-up inputs
//! arrive as [`Resolution`], whose only escape is `?` on a refusal, and the
//! third — the bucket registry — already refuses inside
//! [`crate::domain::bucket::classify`], which answers `IllegalFieldMutation`
//! for a column carrying no tag rather than routing it to a default bucket.
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
//! it stores the verdict in `quorum_descriptor` so no later reader
//! re-evaluates.
//!
//! # Refusal is a domain type here, because the code taxonomy is closed
//!
//! The fail-closed arm has **no declared code**: the gear's 503 set is closed
//! at three by name — `AUDIT_UNAVAILABLE`, 08's `READ_MODEL_OVERLOADED` and
//! 03's `USAGE_TYPE_UNAVAILABLE` (`design/01-foundation.md` §4.4,
//! `design/12-consumer-contracts.md` `inst-cc-errors`) — and 05 §3.3 names
//! none for an unresolvable governance input. Minting a fourth would make a
//! closed roster consistent and wrong, so the refusal is
//! [`MaterialityUnresolved`], a domain value that reaches no wire, and the
//! missing code is registered as an open item rather than invented.
//!
//! # What is deliberately absent: the policy's store
//!
//! `inst-mt-policy-material` makes the policy object a `GovernedLiveOp`
//! subject on its **own** pair `materiality_policy × write`, and
//! [`crate::authz`] mints that pair — but **no table holds it**.
//! `DESIGN.md` §3.5 gives slice 05 exactly `products_approval`,
//! `products_approval_decision` and `products_breakglass_session`, and 05
//! §3.2 records `materiality_policy × write` as having **no route
//! declared**. So [`MaterialityPolicy`] is a value with a default, a floor
//! and a material-on-any-mutation rule, and its store and door are
//! registered as open items rather than invented here. That is why
//! `dod-materiality-policy` carries a bare marker.
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
/// Named individually rather than collapsed into one message because the
/// remedies differ: an absent policy is a provisioning gap, an absent claim
/// set is an authentication one, and an untagged column is a registration
/// bug in the slice that owns it.
#[domain_model]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MaterialityInput {
    /// The tenant's materiality policy — field set, trigger and `N`.
    Policy,
    /// The caller's tenant-scoped claim set (C6, deny-by-default).
    ClaimSet,
}

impl MaterialityInput {
    /// The input's stable spelling, for a refusal's detail and for audit.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Policy => "materiality_policy",
            Self::ClaimSet => "claim_set",
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
/// A closed roster of six, one per owning slice. A kind absent from it is
/// **not** silently non-material: [`MaterialAct::LiveOp`] can only be built
/// from a member, so an unregistered kind has no way to reach the evaluator
/// at all.
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
    /// The whole roster, in slice order. Its length is asserted by a probe,
    /// so a seventh kind cannot arrive without the census moving.
    pub const ALL: [Self; 6] = [
        Self::TaxonomyOp,
        Self::RecognizedSetOp,
        Self::ScheduledTransitionCancel,
        Self::FreezeParticipantOp,
        Self::ReferenceProducerOp,
        Self::PiiAllowListOp,
    ];

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
    /// A registered material `GovernedLiveOp` kind (input (d)).
    LiveOp(MaterialLiveOp),
    /// The policy object's own mutation — **always** material, in either
    /// direction (C4, `inst-mt-policy-material`).
    PolicyMutation,
}

/// The evaluator `inst-mt-inputs` names — *"Submission runs
/// `MaterialityEvaluator` over the change set"*.
///
/// It holds the two looked-up inputs as resolutions and nothing else: no
/// resolver, no clock, no store handle. That is what makes
/// [`Self::verdict`] a function of what its caller read at the submission
/// instant rather than of what is current when it runs.
#[derive(Copy, Clone, Debug)]
pub struct MaterialityEvaluator<'a> {
    policy: Resolution<&'a MaterialityPolicy>,
    claims: Resolution<&'a [String]>,
}

impl<'a> MaterialityEvaluator<'a> {
    /// The evaluator over the inputs a submission resolved.
    #[must_use]
    pub const fn new(
        policy: Resolution<&'a MaterialityPolicy>,
        claims: Resolution<&'a [String]>,
    ) -> Self {
        Self { policy, claims }
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
    /// [`MaterialityRefusal::Unresolved`] when the policy or the claim set
    /// did not resolve; [`MaterialityRefusal::Registry`] when a touched
    /// column carries no bucket tag; [`MaterialityRefusal::CorrectableTouch`]
    /// when a bucket-ii column arrives as an ordinary touch.
    pub fn verdict(&self, act: &MaterialAct<'_>) -> Result<Materiality, MaterialityRefusal> {
        // Both looked-up inputs are required before any arm is judged: an
        // act that would answer `Material` on its shape alone must still
        // refuse when the policy is missing, since the *count* the verdict
        // feeds comes from the policy and a verdict without one cannot be
        // spent.
        let policy = self.policy.require(MaterialityInput::Policy)?;
        let _claims = self.claims.require(MaterialityInput::ClaimSet)?;
        Self::judge(act, policy)
    }

    /// The verdict's arms, once both inputs have resolved.
    fn judge(
        act: &MaterialAct<'_>,
        policy: &MaterialityPolicy,
    ) -> Result<Materiality, MaterialityRefusal> {
        match act {
            MaterialAct::PolicyMutation | MaterialAct::LiveOp(_) => Ok(Materiality::Material),
            MaterialAct::Enumerated(op) => Ok(enumerated_verdict(*op)),
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

/// Input (b)'s verdict. The enumeration is the FR's exact one, so a
/// transition to `draft` or `discarded` is outside it (M-1).
const fn enumerated_verdict(op: EnumeratedOp) -> Materiality {
    match op {
        EnumeratedOp::LifecycleTransition(to) => match to {
            LifecycleState::Published | LifecycleState::Deprecated | LifecycleState::Retired => {
                Materiality::Material
            }
            LifecycleState::Draft | LifecycleState::Discarded => Materiality::NonMaterial,
        },
        EnumeratedOp::CategoryOp | EnumeratedOp::AttributeDefinitionChange => Materiality::Material,
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
        if policy.names_field(column) {
            verdict = Materiality::Material;
            continue;
        }
        let class = bucket::classify(kind, column).map_err(MaterialityRefusal::Registry)?;
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
}

#[cfg(test)]
#[path = "materiality_tests.rs"]
mod materiality_tests;
