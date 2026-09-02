//! The approval ceremony's rules — the quorum descriptor stored at
//! submission, the diff rendered from the stored copy, and the decision's
//! distinctness-by-principal refusal (`design/05-governance.md`
//! `inst-gv-materiality`, `inst-gv-stored-snapshot`, C2; P-D-11, P-D-13,
//! P-D-68).
//!
//! # Both stored values are computed once and never re-derived
//!
//! §4 makes `content_snapshot` and `quorum_descriptor` stored-at-submission
//! for one measured reason each, and they are the same reason one field
//! over. The snapshot: *"the diff shown to approvers is rendered from the
//! STORED snapshot against the last published version, never re-derived from
//! the live head"* — a re-derived diff shows the draft against itself, the
//! pricing defect this rule was designed out of. The descriptor:
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
//! C2 is *"one principal, one decision, whatever roles they hold"*, and
//! `products_approval_decision`'s `UNIQUE (tenant_id, approval_id,
//! approver_principal)` is that floor. [`decision_admitted`] adds the half a
//! UNIQUE cannot express: the **author** is refused, at every `N >= 1`, by
//! principal and never by role — a human holding both `CatalogAdmin` and
//! `FinanceReviewer` is still one principal.
//!
//! # What is deliberately absent
//!
//! The finance predicate's operand: `inst-gv-finance-predicate` names
//! `taxCategory`, `glCode` and `PlanTier`, and **none of the three is a
//! registered column** in `domain::bucket`'s roster — they are 03's, and 03
//! has not registered them. So `finance_material` arrives as an explicit
//! argument rather than being read off the registry, and
//! `dod-finance-predicate` stays unticked and blocked by its own §7 row. A
//! registry lookup here would answer "not finance-material" for every one of
//! the three columns the instruction names.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-self-approval:p1
//! @cpt-cf-bss-products-dod-override-ceremony

use serde_json::{Map, Value as JsonValue};
use toolkit_macros::domain_model;
use uuid::Uuid;

use crate::domain::canonical;
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

/// The stored quorum descriptor — §4's own field set.
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
        canonical::canonical_rendering(&JsonValue::Object(map), canonical::Absence::Omit)
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
        // No basis, or a basis whose frozen content could not be read: both
        // render as the whole-content addition. Inventing a basis from the
        // head is the one thing this function must not do.
        _ => ApproverDiff::WholeContentAddition {
            submitted: stored_snapshot.to_owned(),
        },
    }
}

#[cfg(test)]
#[path = "approval_tests.rs"]
mod approval_tests;
