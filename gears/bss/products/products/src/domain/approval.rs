//! The approval ceremony's rules — the quorum descriptor stored at
//! submission, the diff rendered from the stored copy, and the decision's
//! distinctness-by-principal refusal (`design/05-governance.md`
//! `inst-gv-materiality`, `inst-gv-stored-snapshot`, C2; P-D-11, P-D-13,
//! P-D-68).
//!
//! # Both stored values are computed once and never re-derived
//!
//! Both are stored-at-submission for one measured reason each, and they are
//! the same reason one field over. The snapshot's rule is **§2's
//! `inst-gv-stored-snapshot`** (§4's own entry only restates the column):
//! *"the diff shown to approvers is rendered from the STORED snapshot
//! against the last published version, never re-derived from the live
//! head"* — a re-derived diff shows the draft against itself, the pricing
//! defect this rule was designed out of. The descriptor's is **§4's**:
//! `configured_quorum` is the `N` in force at submission, so deriving it
//! from current policy would change a **pending** record when the tenant
//! edits `N`. A record that changes after the fact is not a record.
//!
//! So [`describe_quorum`] takes `N` as an argument and [`render_diff`] takes
//! the stored snapshot; neither reads a head, and neither holds a resolver.
//!
//! # `required` is the effective count, never the raw `N`
//!
//! `inst-gv-materiality` fixes it: **material ⇒ `required = N`**;
//! **non-material ⇒ `min(N, 1)`**. `inst-gv-queue` then forbids the raw
//! value on the wire — *"never the raw configured `N`, so a card cannot show
//! '2 required' for a record that closes on one"* — which is why
//! [`QuorumDescriptor`] carries both and names them apart.
//!
//! # The self-approval refusal is by principal, and that is a physical floor
//!
//! C2 reads *"Distinctness is by **principal**, never by role: one human
//! holding both roles is one approver"*, and §4 calls
//! `products_approval_decision`'s key *"C2's physical floor: one principal,
//! one decision"*. [`decision_admitted`] adds the half a
//! UNIQUE cannot express: the **author** is refused, at every `N >= 1`, by
//! principal and never by role — a human holding both `CatalogAdmin` and
//! `FinanceReviewer` is still one principal.
//!
//! # A fixed floor exists for exactly one ceremony, and it is not `N`
//!
//! P-D-13 enumerated the quorum shorthand's six sites and put **one** outside
//! the tenant's `N` entirely — cross-tenant break-glass elevation, at a fixed
//! floor of two distinct platform principals, *"because the acting principal
//! is not the tenant's"*. The other five follow `N` and record the reduction.
//! [`describe_platform_quorum`] is that floor's writer and
//! [`describe_quorum`] is the other five's; they are two functions rather
//! than a flag because the difference is **whose principal acts**, not how
//! many, and a flag on one function would let a caller pass the tenant's `N`
//! into a ceremony no tenant configures. See [`PLATFORM_QUORUM_FLOOR`] for
//! why it is a separate constant from [`DEFAULT_APPROVER_COUNT`] at the same
//! value.
//!
//! # What is deliberately absent
//!
//! **The finance predicate's operand.** `inst-gv-finance-predicate` names
//! `taxCategory`, `glCode` and `PlanTier`, and **none of the three is a
//! registered column** in `domain::bucket`'s roster — they are 03's, and 03
//! has not registered them. So `finance_material` arrives as an explicit
//! argument rather than being read off the registry, and
//! `dod-finance-predicate` stays unticked and blocked by its own §7 row. A
//! registry lookup here would answer "not finance-material" for every one of
//! the three columns the instruction names.
//!
//! **A principal's roles and scope claims.** [`ApproverRole`] and
//! [`CastDecision::roles`] are operands for the same reason and a sharper
//! one: §7 row 25 measures that no surface in the gear carries a role at all,
//! and that the decision row stores neither the roles nor the scope claims
//! that were true when the decision was made. So [`evaluate_quorum`] and
//! [`approver_covers_subject`] are total functions of what their caller
//! supplies, and the caller that would supply it does not exist. Both stay
//! unticked on that account, not on their own arithmetic.
//!
//! **Two of the six declared codes.** `design/05` §3.3 declares
//! `APPROVER_SCOPE_EXCEEDED` and `APPROVER_ROLE_REQUIRED`, and neither is in
//! `domain::error` at this commit — so [`ApproverScopeVerdict`] and
//! [`QuorumOutcome::RolePredicateUnmet`] carry their refusals as values and
//! the taxonomy registration is `dod-governance-errors`'.
//!
//! **The descriptor's sixth name.** See [`DESCRIPTOR_ROSTER`].
//!
//! @cpt-dod:cpt-cf-bss-products-dod-self-approval:p1
//! @cpt-cf-bss-products-dod-override-ceremony
//! @cpt-cf-bss-products-dod-quorum-descriptor
//! @cpt-cf-bss-products-dod-finance-predicate
//! @cpt-cf-bss-products-dod-quorum-evaluator
//! @cpt-cf-bss-products-dod-approver-scope

use std::collections::BTreeSet;

use serde_json::{Map, Value as JsonValue};
use toolkit_macros::domain_model;
use uuid::Uuid;

use crate::domain::canonical;
use crate::domain::containment::{
    ResolvedScope, ScopeContainment, ScopeDimension, ScopePair, contains,
};
use crate::domain::error::DomainError;
use crate::domain::materiality::{DEFAULT_APPROVER_COUNT, Materiality};

/// A mandatory predicate the descriptor could not carry at its own effective
/// count.
///
/// One value today, and it is an enum rather than a `bool` because the
/// descriptor records *which* control is absent: the marker's purpose is
/// that "the control's absence is a stored fact, not something a later
/// reader infers from a config value".
#[domain_model]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum UnsatisfiablePredicate {
    /// Finance-material at `N = 0`: there are no approvers to hold the role,
    /// so the predicate has no subject (`inst-gv-finance-predicate`).
    FinanceReviewer,
}

impl UnsatisfiablePredicate {
    /// The stored spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FinanceReviewer => "finance_reviewer",
        }
    }
}

/// The five §4 names the descriptor stores, in the order §4 gives them.
///
/// §4's set is **six** wide: `configuredQuorum`, the required count, the
/// finance predicate, `predicateUnsatisfiable`, **override conditions** and
/// `quorumReduced`. The sixth waits on `dod-override-ceremony`'s missing
/// operand — no artifact says where a subject's lint findings are read from
/// — so `dod-quorum-descriptor` stays unticked and this roster is five.
const DESCRIPTOR_ROSTER: [&str; 5] = [
    "configuredQuorum",
    "required",
    "financeRequired",
    "predicateUnsatisfiable",
    "quorumReduced",
];

/// The stored quorum descriptor — five of §4's six names; see
/// [`DESCRIPTOR_ROSTER`] for the sixth.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuorumDescriptor {
    configured_quorum: u32,
    required: u32,
    finance_required: bool,
    predicate_unsatisfiable: Option<UnsatisfiablePredicate>,
    quorum_reduced: bool,
}

impl QuorumDescriptor {
    /// The effective count this record closes on.
    #[must_use]
    pub const fn required(&self) -> u32 {
        self.required
    }

    /// The raw `N` in force at submission.
    #[must_use]
    pub const fn configured_quorum(&self) -> u32 {
        self.configured_quorum
    }

    /// Whether a `FinanceReviewer` must be among the approvers.
    #[must_use]
    pub const fn finance_required(&self) -> bool {
        self.finance_required
    }

    /// Which mandatory predicate could not be carried, if any.
    #[must_use]
    pub const fn predicate_unsatisfiable(&self) -> Option<UnsatisfiablePredicate> {
        self.predicate_unsatisfiable
    }

    /// P-D-13's marker: the effective count is below the retained-name
    /// default of 2, so a one-person act is never read off a trail that says
    /// "two-person".
    #[must_use]
    pub const fn quorum_reduced(&self) -> bool {
        self.quorum_reduced
    }

    /// The canonical rendering stored in `products_approval.quorum_descriptor`.
    ///
    /// Rendered through [`canonical::canonical_rendering`] rather than
    /// `serde_json::to_string` so the stored bytes are stable under key
    /// order — the column is compared byte-for-byte by the idempotency lane
    /// and by any later reader diffing two records.
    #[must_use]
    pub fn stored(&self) -> String {
        let mut map = Map::new();
        map.insert("configuredQuorum".to_owned(), self.configured_quorum.into());
        map.insert("required".to_owned(), self.required.into());
        map.insert("financeRequired".to_owned(), self.finance_required.into());
        map.insert(
            "predicateUnsatisfiable".to_owned(),
            match self.predicate_unsatisfiable {
                Some(p) => JsonValue::String(p.as_str().to_owned()),
                None => JsonValue::Null,
            },
        );
        map.insert("quorumReduced".to_owned(), self.quorum_reduced.into());
        // `Absence::Null` with the roster, not `Omit`: this is stored
        // content with a **required** field set, and the reader errors on any
        // missing member. Under `Omit` a sixth field added to the struct and
        // forgotten here would render fine and fail at decode — every record
        // written after the deploy unreadable while the older ones still
        // work, which is the hardest shape to diagnose. Under the roster the
        // forgotten member renders `null` and the reader names it at once.
        canonical::canonical_rendering(
            &JsonValue::Object(map),
            canonical::Absence::Null {
                roster: &DESCRIPTOR_ROSTER,
            },
        )
    }
}

/// Read a stored descriptor back off the row.
///
/// The inverse of [`QuorumDescriptor::stored`], and beside it for P-D-77's
/// reason: a parse written at a consumer would be the second serialization
/// rule the canonical module exists to prevent. Every field is required —
/// a descriptor missing one is a row this gear wrote wrong, and defaulting
/// the missing member is how a record silently loses its `quorumReduced`
/// marker.
///
/// # Errors
///
/// The decode's own text, or the name of the first field missing or
/// mistyped.
pub fn descriptor_from_stored(stored: &str) -> Result<QuorumDescriptor, String> {
    let map = canonical::decode_rendering(stored)?;
    let count = |name: &str| -> Result<u32, String> {
        map.get(name)
            .and_then(JsonValue::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| format!("{name} is missing or not a count"))
    };
    let flag = |name: &str| -> Result<bool, String> {
        map.get(name)
            .and_then(JsonValue::as_bool)
            .ok_or_else(|| format!("{name} is missing or not a boolean"))
    };
    let predicate_unsatisfiable = match map.get("predicateUnsatisfiable") {
        Some(JsonValue::Null) => None,
        Some(JsonValue::String(name))
            if name == UnsatisfiablePredicate::FinanceReviewer.as_str() =>
        {
            Some(UnsatisfiablePredicate::FinanceReviewer)
        }
        Some(other) => {
            return Err(format!(
                "predicateUnsatisfiable carries {other}, not a known predicate"
            ));
        }
        None => return Err("predicateUnsatisfiable is missing".to_owned()),
    };
    Ok(QuorumDescriptor {
        configured_quorum: count("configuredQuorum")?,
        required: count("required")?,
        finance_required: flag("financeRequired")?,
        predicate_unsatisfiable,
        quorum_reduced: flag("quorumReduced")?,
    })
}

/// Build the descriptor from the verdict and the `N` in force.
///
/// `finance_material` is the caller's, not a registry lookup — see the
/// module doc for why the three columns the instruction names cannot be
/// looked up here.
#[must_use]
pub fn describe_quorum(
    materiality: Materiality,
    configured_quorum: u32,
    finance_material: bool,
) -> QuorumDescriptor {
    let required = match materiality {
        // `required = N` (`inst-gv-materiality`).
        Materiality::Material => configured_quorum,
        // `min(N, 1)` — so a tenant at `N = 0` publishes approver-less by
        // policy and the record says exactly that (P-D-11).
        Materiality::NonMaterial => configured_quorum.min(1),
    };
    // At `N = 0` the finance predicate has no subject: there are no
    // approvers to hold the role, so it is not set and the descriptor
    // records the absence instead. Keying this on `required` rather than on
    // `configured_quorum` is deliberate — a non-material change at `N = 3`
    // closes on one approver, and that one approver must still be a
    // FinanceReviewer.
    let (finance_required, predicate_unsatisfiable) = match (finance_material, required) {
        (true, 0) => (false, Some(UnsatisfiablePredicate::FinanceReviewer)),
        (true, _) => (true, None),
        (false, _) => (false, None),
    };
    QuorumDescriptor {
        configured_quorum,
        required,
        finance_required,
        predicate_unsatisfiable,
        quorum_reduced: required < DEFAULT_APPROVER_COUNT,
    }
}

/// Whether one principal's decision is admitted on this record.
///
/// # Errors
///
/// [`DomainError::SelfApprovalForbidden`] when the approver is the record's
/// own submitter and the record closes on at least one approver. The
/// refusal is by principal: no role, claim or grant admits an author onto
/// their own record.
pub fn decision_admitted(
    submitter: Uuid,
    approver: Uuid,
    descriptor: &QuorumDescriptor,
) -> Result<(), DomainError> {
    if descriptor.required() >= 1 && approver == submitter {
        return Err(DomainError::SelfApprovalForbidden(format!(
            "principal {approver} submitted this record and cannot decide it \
             (required {})",
            descriptor.required()
        )));
    }
    Ok(())
}

/// Where the override acknowledgment is stored for a given effective count.
#[domain_model]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AckPlacement {
    /// `N >= 1`: each approver acknowledges on their own decision row.
    OnDecision,
    /// `N = 0`: the **author** acknowledges, in `author_override_ack` on the
    /// record itself, written by the submit door (P-D-68 arm 1). A synthetic
    /// decision row naming the author would break C2's
    /// one-principal-one-decision UNIQUE and the two-person invariant it
    /// enforces, so a fact gets a column.
    OnRecord,
}

/// Which home the acknowledgment takes at this effective count.
#[must_use]
pub const fn ack_placement(descriptor: &QuorumDescriptor) -> AckPlacement {
    if descriptor.required() == 0 {
        AckPlacement::OnRecord
    } else {
        AckPlacement::OnDecision
    }
}

/// The `diff_basis` a submission pins.
///
/// `None` on a **first publish**: there is no last published version, so the
/// diff renders as a whole-content addition against no basis. The arm is
/// explicit because the slice makes a first publish material, so a
/// first-publish record exists and must carry a basis — and filling the gap
/// by convention would most plausibly diff the draft against the head, which
/// is the re-derivation the rule forbids.
#[must_use]
pub const fn diff_basis_for(last_published_version: Option<i64>) -> Option<i64> {
    last_published_version
}

/// What an approver is shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApproverDiff {
    /// The stored submission against the named published version.
    Against {
        /// The published version the diff renders against.
        basis: i64,
        /// The content stored at submission — never the live head.
        submitted: String,
        /// The basis version's frozen content.
        basis_content: String,
    },
    /// A first publish: the whole stored submission, against no basis.
    WholeContentAddition {
        /// The content stored at submission.
        submitted: String,
    },
    /// A record that **has** a published basis whose frozen content could
    /// not be read.
    ///
    /// Its own arm, because collapsing it onto
    /// [`Self::WholeContentAddition`] shows the approver a first publish for
    /// a change that has a predecessor — they approve a diff they were never
    /// shown, which is the class `dod-stored-snapshot` exists to prevent,
    /// arriving through the unreadable-basis door rather than the
    /// re-derivation one. A caller renders this as a refusal, never as a
    /// diff.
    BasisUnreadable {
        /// The published version the diff should have rendered against.
        basis: i64,
        /// The content stored at submission, carried so the refusal can name
        /// what was being approved.
        submitted: String,
    },
}

/// Render the approver's diff **from the stored copy**.
///
/// The stored snapshot is an argument and no head is reachable from here, so
/// the re-derivation the rule forbids is not expressible at this call site
/// rather than merely discouraged.
#[must_use]
pub fn render_diff(
    stored_snapshot: &str,
    basis: Option<i64>,
    basis_content: Option<&str>,
) -> ApproverDiff {
    match (basis, basis_content) {
        (Some(basis), Some(basis_content)) => ApproverDiff::Against {
            basis,
            submitted: stored_snapshot.to_owned(),
            basis_content: basis_content.to_owned(),
        },
        // A pinned basis whose content could not be read is **not** a first
        // publish, and saying so is the point of the third arm.
        (Some(basis), None) => ApproverDiff::BasisUnreadable {
            basis,
            submitted: stored_snapshot.to_owned(),
        },
        // No basis at all: the first publish. Inventing one from the head is
        // the one thing this function must not do.
        (None, _) => ApproverDiff::WholeContentAddition {
            submitted: stored_snapshot.to_owned(),
        },
    }
}

// ---------------------------------------------------------------------------
// The quorum's three remaining operands: the platform floor, the evaluator,
// and the approver-scope rule.
// ---------------------------------------------------------------------------

/// The fixed floor for a ceremony whose acting principal is **not the
/// tenant's** — two distinct platform principals (**P-D-13**).
///
/// P-D-13 enumerated the quorum shorthand's six sites and dispositioned
/// exactly one of them outside `N`: *"Cross-tenant break-glass elevation
/// (AC #30, `inst-bg-open`) is **not `N`-governed at all**. Its principal is
/// a platform owner acting across tenants; no tenant's configured `N` has
/// standing over an act whose subject is another tenant's data. Fixed floor:
/// **two distinct platform principals**, or the AC's already-stated
/// post-hoc-review arm."*
///
/// The other five follow `N` and record the reduction, and the entry says
/// why a floor on them would be wrong: *"floor 2 on force-completion leaves a
/// solo tenant with a `CatalogVersion` permanently past its freeze timeout
/// and un-resolvable — the exact class of block P-D-11 exists to remove …
/// A fixed floor is right only where the acting principal is not the
/// tenant's, which is break-glass and nothing else in v1."*
///
/// It is a separate constant from [`DEFAULT_APPROVER_COUNT`] even though both
/// are 2, because they are two different facts that happen to share a value:
/// one is a **tenant policy default** a tenant may configure to 0, the other
/// a **platform floor** no tenant configuration reaches. Sharing the constant
/// would make a change to either silently move the other.
pub const PLATFORM_QUORUM_FLOOR: u32 = 2;

/// Which authority fixed a ceremony's approver count (**P-D-13**).
///
/// The distinction is the decision's own: it is not about *how many* but about
/// **whose** principal acts, and that is what decides whether a tenant's
/// configuration has standing at all.
#[domain_model]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum QuorumAuthority {
    /// The tenant's configured `N`, per `inst-gv-materiality`. P-D-13's five
    /// `N`-governed sites: freeze force-completion, the uncomposed-bundle
    /// override, un-deprecation, the slice-07 correction door and the
    /// slice-07 break-glass correction.
    TenantPolicy,
    /// A floor outside every tenant's `N`, because the acting principal is
    /// not the tenant's. Cross-tenant break-glass elevation, and nothing else
    /// in v1.
    PlatformFloor,
}

/// The descriptor for a ceremony held to the platform floor
/// ([`PLATFORM_QUORUM_FLOOR`], **P-D-13**).
///
/// # This is the writer §7 row 9 says does not exist
///
/// That row reads: *"the elevation demands two distinct platform principals
/// outside the tenant's `N`, while `required` is defined only as `N` or
/// `min(N, 1)` — **no writer can produce a fixed 2**"*. True at `HEAD`:
/// every path into a descriptor ran through [`describe_quorum`], whose
/// `required` is a function of `configured_quorum`. This function is the
/// missing writer, and it supplies it **without** answering the rest of row
/// 9 — whether a break-glass two-person approval is an `ApprovalRecord`, and
/// which row stores it, are untouched here, because a [`QuorumDescriptor`] is
/// a value that renders the same whichever row holds it.
///
/// # `quorumReduced` is false here, and that is the marker working
///
/// P-D-13 sets the marker *"when the effective count is below the
/// retained-name default of 2"*. The floor **is** 2, so a platform ceremony
/// at the floor is never reduced and the audit trail that says "two-person"
/// is telling the truth. That is the one case where the marker's own wording
/// and the ceremony agree without any tenant configuration in between.
///
/// # `configuredQuorum` carries the floor, not the tenant's `N`
///
/// `inst-gv-queue` puts `configuredQuorum` on the wire as *"the raw `N` when
/// a surface needs it"*, and the tenant's `N` has **no standing** over this
/// act. A card rendering the target tenant's `N` beside a platform ceremony
/// would assert exactly the standing P-D-13 denies, so the field carries the
/// count actually in force. **No artifact settles this field's meaning for a
/// non-tenant ceremony**, so the reading is registered as an owed item rather
/// than presented as the design's.
#[must_use]
pub fn describe_platform_quorum() -> QuorumDescriptor {
    QuorumDescriptor {
        configured_quorum: PLATFORM_QUORUM_FLOOR,
        required: PLATFORM_QUORUM_FLOOR,
        // The finance predicate is about finance-material *fields*
        // (`taxCategory`, `glCode`, `PlanTier`). An elevation touches none,
        // so the predicate is neither set nor unsatisfiable — its absence
        // here is "not applicable", not "could not be carried".
        finance_required: false,
        predicate_unsatisfiable: None,
        quorum_reduced: PLATFORM_QUORUM_FLOOR < DEFAULT_APPROVER_COUNT,
    }
}

/// The C1 base role set, by name.
///
/// # No surface supplies this, and that is §7 row 25
///
/// C1 demands each approver hold *"`CatalogAdmin` or `FinanceReviewer`"* and
/// `inst-gv-finance-predicate` demands one of them **be** a `FinanceReviewer`.
/// Neither is askable of the gear today: `SecurityContext` exposes
/// `subject_id`, `subject_type`, `subject_tenant_id`, `token_scopes` and
/// `bearer_token` and no role, and [`crate::authz`] asks the policy point
/// `(resource, action)` about the **current** caller. There is no way to ask
/// whether principal X holds role R, still less to ask it of a **past**
/// approver at gate time — which is what a quorum evaluation needs.
///
/// **The donor gear does not have this type either, and answers the question
/// a different way.** `gears/bss/pricing` resolves its `FinanceReviewer`
/// through the *grant* — whoever holds the approve permission is one — which
/// works there because its check is about the caller in front of it. It
/// cannot answer C1's question, which is about the roles a set of **already
/// recorded** approvers held when they decided. So this is not the donor's
/// shape declined; it is an operand neither gear has, and row 25 names the
/// cheapest place to hold it: the decision row, which today stores neither
/// the roles nor the scope claims that were true at the decision.
///
/// It is a closed enum rather than the `&[String]` claim set
/// [`crate::domain::materiality::MaterialityEvaluator`] takes, because the
/// two roles are named by C1 and a typo in a string would silently fail the
/// finance predicate open.
#[domain_model]
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ApproverRole {
    /// C1's ordinary approver role.
    CatalogAdmin,
    /// The mandatory second lens on finance-material fields.
    FinanceReviewer,
}

impl ApproverRole {
    /// The stable spelling, matching
    /// [`UnsatisfiablePredicate::FinanceReviewer`]'s for the one role the two
    /// types share.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CatalogAdmin => "catalog_admin",
            Self::FinanceReviewer => "finance_reviewer",
        }
    }
}

/// One recorded verdict, as the evaluator counts it.
///
/// `roles` are the roles that were true **when the decision was made**, not
/// the ones the principal holds now: an approval is evaluated at gate time,
/// possibly long after, and a role revoked in between must not retroactively
/// unmake a decision that was valid when cast. Nothing stores them today
/// (§7 row 25), which is why they arrive as an operand — the same reason
/// `finance_material` does in [`describe_quorum`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CastDecision {
    /// The deciding principal, pseudonymous.
    pub principal: Uuid,
    /// Whether the verdict was an approval. A rejection finalizes the record
    /// and never counts toward satisfaction.
    pub approved: bool,
    /// The roles held at the instant of the decision.
    pub roles: Vec<ApproverRole>,
}

/// What the evaluator answered, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuorumOutcome {
    /// The stored descriptor is met by distinct principals holding the
    /// required roles.
    Satisfied,
    /// The count is not yet met.
    CountUnmet {
        /// How many distinct eligible principals approved.
        counted: u32,
        /// How many the stored descriptor requires.
        required: u32,
    },
    /// The count is met **numerically** and the role predicate is not — the
    /// `APPROVER_ROLE_REQUIRED` case, which `design/05` §3.3 puts on the
    /// **gate**.
    ///
    /// Its own arm rather than a second `CountUnmet`, because the two carry
    /// different codes and the distinction is the whole of L-2: a caller who
    /// gathered enough signatures and the wrong ones must be told which.
    RolePredicateUnmet {
        /// How many distinct eligible principals approved.
        counted: u32,
    },
}

/// Count distinct approving principals against the **stored** descriptor.
///
/// # Distinctness is by principal, and it is deduplicated here as well as in
/// the index
///
/// `UNIQUE (tenant_id, approval_id, approver_principal)` already makes a
/// second row from one principal impossible, so this deduplication is not
/// what enforces C2 — it is what keeps the evaluator honest against a caller
/// that assembled the list itself (a gate reading a join, a test, a future
/// batch read). A human holding both `CatalogAdmin` and `FinanceReviewer`
/// counts **once** either way, which is C2's whole point.
///
/// # `base_roles` is a call operand, because §7 row 16 is open
///
/// C1 scopes its CatalogAdmin-or-FinanceReviewer floor to **material**
/// changes; a non-material change gets `min(N, 1)` and the descriptor carries
/// no base role set — and no `Materiality` either, so this function cannot
/// recover which applies. Row 16 asks exactly that: *"Does the base role set
/// bind the single approver of a non-material change?"* Defaulting it either
/// way here would answer an open item from a function signature, so the
/// caller names the set that binds and an empty slice means "any holder of
/// `approval x decide`".
///
/// # `predicateUnsatisfiable` counts as met, and only that way
///
/// `inst-gv-quorum`: a recorded `predicateUnsatisfiable` *"counts as met for
/// the evaluator while remaining visible as unmet-by-policy in the record and
/// the inbox envelope; that is the only way it may be discharged, and it is
/// never how a predicate is discharged at `N >= 1`"*. Since
/// [`describe_quorum`] records the marker only at effective count 0, and this
/// function reads the marker off the stored descriptor rather than
/// recomputing it, the `N >= 1` half holds by construction.
///
/// # What this function does **not** decide
///
/// Whether a [`QuorumOutcome::Satisfied`] answer flips the record's `state`
/// column, and in which transaction, is §7 row 11 — *"which transaction
/// writes `state = satisfied`?"* — and row 31, which observes that at
/// `required = 0` no decision is ever recorded so §4's only human arm never
/// fires. This function answers the **arithmetic**: at `required = 0` a
/// descriptor is met by zero decisions, which is what P-D-11's approver-less
/// tenant means. It takes no position on which writer acts on that.
#[must_use]
pub fn evaluate_quorum(
    descriptor: &QuorumDescriptor,
    decisions: &[CastDecision],
    base_roles: &[ApproverRole],
) -> QuorumOutcome {
    let mut counted_principals: BTreeSet<Uuid> = BTreeSet::new();
    let mut finance_lens_held = false;
    for decision in decisions {
        if !decision.approved {
            continue;
        }
        // C1's base set, where the caller says it binds. An approver holding
        // none of the named roles is not an eligible approver, so they do not
        // count toward the number either.
        let eligible =
            base_roles.is_empty() || decision.roles.iter().any(|role| base_roles.contains(role));
        if !eligible {
            continue;
        }
        // Newly counted or not, the finance lens is a property of the SET of
        // approvers, so it is read from every eligible approving principal —
        // including the one already counted, whose second role is exactly
        // what C2 says does not buy a second signature but does satisfy the
        // predicate "as one of the two".
        if decision.roles.contains(&ApproverRole::FinanceReviewer) {
            finance_lens_held = true;
        }
        counted_principals.insert(decision.principal);
    }
    let counted = u32::try_from(counted_principals.len()).unwrap_or(u32::MAX);

    let finance_met = if descriptor.finance_required() {
        finance_lens_held
    } else {
        // Either the predicate was never set, or it was recorded
        // unsatisfiable — `inst-gv-quorum`'s only discharge. Both leave
        // nothing for the evaluator to demand, and the record keeps saying
        // which of the two it was.
        true
    };

    if counted < descriptor.required() {
        return QuorumOutcome::CountUnmet {
            counted,
            required: descriptor.required(),
        };
    }
    if finance_met {
        QuorumOutcome::Satisfied
    } else {
        QuorumOutcome::RolePredicateUnmet { counted }
    }
}

/// Whether an approver's claims cover the subject's scope
/// (`inst-gv-scope`, on 01 **P-D-39**'s two boundaries).
///
/// # The claim set is the parent and the subject is the child
///
/// This is the whole of the rule and the one place it can be inverted.
/// `inst-gv-scope` reads: *"an unrestricted claim set covers every subject,
/// an unrestricted subject scope is covered only by an unrestricted claim
/// set, and between two non-empty sets it is ordinary subset"*. Laid against
/// [`contains`]'s three clauses — an unrestricted **parent** contains every
/// child; an unrestricted **child** is contained only by an unrestricted
/// parent; otherwise subset — the mapping is forced: **parent = the
/// approver's claims, child = the subject's scope**. Transposed, a
/// region-restricted approver would cover a tenant-wide subject, which is the
/// scope rule deleted rather than applied.
///
/// So this function does not re-implement containment; it names the mapping,
/// applies P-D-39's one rule to each of the two dimensions, and reports which
/// failed.
#[must_use]
pub fn approver_covers_subject(claims: &ScopePair, subject: &ScopePair) -> ApproverScopeVerdict {
    for (dimension, claim, subject_scope) in [
        (ScopeDimension::Region, &claims.region, &subject.region),
        (ScopeDimension::Brand, &claims.brand, &subject.brand),
    ] {
        // parent = claims, child = subject. See this function's doc.
        if let ScopeContainment::NotContained {
            dimension,
            parent,
            child,
        } = contains(dimension, claim, subject_scope)
        {
            return ApproverScopeVerdict::Exceeded {
                dimension,
                claimed: parent,
                subject: child,
            };
        }
    }
    ApproverScopeVerdict::Covered
}

/// The approver-scope verdict.
///
/// # Why this is a domain value and not a [`DomainError`]
///
/// `design/05` §3.3 declares `APPROVER_SCOPE_EXCEEDED` at **403** and the
/// roster is closed at six — but the code does not exist in
/// [`crate::domain::error`] at this commit, and neither does
/// `APPROVER_ROLE_REQUIRED`. Two of the slice's six ship
/// (`SELF_APPROVAL_FORBIDDEN`, `APPROVAL_SUPERSEDED`) and four do not. So the
/// refusal is carried as a value naming its operands, and the taxonomy
/// registration is `dod-governance-errors`' — which is the same treatment
/// `MaterialityUnresolved` takes, for a different reason: there the code was
/// never declared, here it is declared and not yet wired.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApproverScopeVerdict {
    /// The claims cover the subject on both dimensions.
    Covered,
    /// They do not, on the named dimension. Both scopes are carried so the
    /// refusal can say what escaped without re-running the check — and so the
    /// audit row `inst-gv-scope` requires can name it.
    Exceeded {
        /// Which dimension failed.
        dimension: ScopeDimension,
        /// The approver's claims on that dimension.
        claimed: ResolvedScope,
        /// The subject's scope on that dimension.
        subject: ResolvedScope,
    },
}

#[cfg(test)]
#[path = "approval_tests.rs"]
mod approval_tests;
