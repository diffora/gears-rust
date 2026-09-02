//! The taxonomy's domain values: the ancestor chain a re-parent is judged
//! against, and the two stored rosters its content tables key on
//! (`design/02-taxonomy-attributes.md` §3.1 `inst-tx-walk`, §3.4
//! `inst-tc-writer-lock`; `inst-tx-name-in-parent`; §4.1's `role` and
//! `state` columns).
//!
//! # The walk is a verdict over a chain the caller read, not a query
//!
//! `inst-tx-walk` puts the walk *"inside the write transaction, under the
//! per-tenant taxonomy writer lock"*, and gives the reason plainly:
//! *"single-writer is what makes `TaxonomyWalk`'s verdict trustworthy"*. So
//! this module holds no store handle. It judges a chain its caller read
//! **under the lock**, which is the only reading under which the verdict is
//! still true when the write lands — a walk that fetched its own rows would
//! be judging a tree that could move between the fetch and the update.
//!
//! # The cycle rule, and the two shapes it has to catch
//!
//! *"a re-parent whose new ancestor chain contains the node itself fails
//! `TAXONOMY_CYCLE`"*. Two shapes, and only the first is obvious:
//!
//! - the node is its own new parent — which `chk_products_category_not_own_parent`
//!   also refuses at the physical layer, so this arm is defence in depth;
//! - the node appears **anywhere** up the new parent's chain, at any depth.
//!   Nothing physical can catch this one: a `CHECK` sees one row.
//!
//! [`cycle_verdict`] therefore takes the whole chain, root-last, and the
//! probe walks a three-deep tree — a two-deep fixture passes a guard that
//! only compares the immediate parent.
//!
//! # `TAXONOMY_LIMIT` is deliberately absent
//!
//! `inst-tx-walk` fails a create or re-parent *"exceeding configured max
//! depth or max children"* with `TAXONOMY_LIMIT`, and `design/02` §6 records
//! that *"The taxonomy and metadata limits have no interim default
//! anywhere"* — their owner being the §17.1 policy owner. A guard with no
//! threshold is a guard that either refuses everything or nothing, so the
//! code is declared and unraised here, and `dod-taxonomy-walk` stays
//! unticked on its own §7 row rather than on an invented number.
//!
//! # Two rosters are closed here and a third is deliberately not
//!
//! [`AssignmentRole`] and [`DefinitionState`] are parsed types because their
//! DDL closes them — `chk_products_product_category_role` and
//! `chk_products_attribute_definition_state` each carry an `IN (…)` list, and
//! `dod-category-assignment-table` and `dod-attribute-definition-table` state
//! the same two sets in prose. A column the engine constrains is a column the
//! reader may parse fail-closed.
//!
//! **`entity_kind` and `value_type` get no type here, and that is the point.**
//! Both ship pinned to non-emptiness only (P-D-74's shape), because §7 rows 20
//! and 13 are the live questions *"what `entity_kind` values does each table
//! admit"* and what a definition's type roster is. An enum here would answer
//! them in code, which is the same authoring a `CHECK` would have been — the
//! migrations refused it for that reason and so does this module. They stay
//! `&str` through the repository until an owner closes the sets.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-name-in-parent:p1
//! @cpt-cf-bss-products-dod-taxonomy-walk

use toolkit_macros::domain_model;
use uuid::Uuid;

use crate::domain::error::DomainError;

/// Which taxonomy mutation is being judged.
///
/// The two the uniqueness rule is re-checked on, named apart because
/// `inst-tx-name-in-parent` requires **both** — *"re-checked on rename **and**
/// re-parent"* — and a guard wired to one of them passes every test written
/// for that one.
#[domain_model]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TaxonomyMutation {
    /// The name changes; the parent does not.
    Rename,
    /// The parent changes; the name does not.
    Reparent,
    /// A new node, which is a name check with no prior row.
    Create,
}

impl TaxonomyMutation {
    /// Whether the name-in-parent uniqueness rule is re-evaluated.
    ///
    /// All three, and that is the point: a re-parent carries the node's
    /// existing name into a **new** sibling set, so it can collide without
    /// the name changing at all — the case a rename-only guard misses.
    #[must_use]
    pub const fn rechecks_name(self) -> bool {
        match self {
            Self::Rename | Self::Reparent | Self::Create => true,
        }
    }
}

/// The verdict on a re-parent's new ancestor chain.
///
/// # Errors
///
/// [`DomainError::TaxonomyCycle`] when `node` appears in `new_ancestors`,
/// naming the depth at which it was found — the operand an operator needs to
/// see which edge closed the loop.
pub fn cycle_verdict(node: Uuid, new_ancestors: &[Uuid]) -> Result<(), DomainError> {
    if let Some(depth) = new_ancestors.iter().position(|a| *a == node) {
        return Err(DomainError::TaxonomyCycle(format!(
            "category {node} is its own ancestor at depth {depth} of the requested parent's \
             chain: the re-parent would close a cycle"
        )));
    }
    Ok(())
}

/// The chain a caller must read under the lock, root-last, for
/// [`cycle_verdict`].
///
/// A helper rather than a query: it walks a map the caller assembled from
/// rows it holds, so the walk cannot silently read outside the transaction.
/// Terminates on a `None` parent (the root) **or** on a repeat — a tree that
/// already contains a cycle would otherwise loop forever, and a store that
/// was corrupted or a lock that was skipped are exactly when this runs.
#[must_use]
pub fn ancestors_of(start: Uuid, parent_of: &impl Fn(Uuid) -> Option<Uuid>) -> Vec<Uuid> {
    let mut chain = Vec::new();
    let mut at = start;
    loop {
        chain.push(at);
        match parent_of(at) {
            Some(parent) if !chain.contains(&parent) => at = parent,
            _ => return chain,
        }
    }
}

/// Which role a category assignment carries for one Product.
///
/// The roster is `chk_products_product_category_role`'s, and
/// `dod-category-assignment-table` states it in prose. Parsed rather than
/// carried as a string because the *at-most-one-primary* guarantee is a
/// partial index keyed on this literal — a caller that spelled it
/// `"Primary"` would write a second row the index does not see, which is the
/// one failure the `DoD` says must be an index rather than a convention.
#[domain_model]
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AssignmentRole {
    /// The single primary assignment. Optional at draft and required at
    /// publish (`inst-tx-primary-at-publish`); at most one per Product, by
    /// `uq_products_product_category_primary`.
    Primary,
    /// Any additional assignment. Unbounded in number.
    Secondary,
}

impl AssignmentRole {
    /// The stored spelling — the migration's `CHECK` roster, verbatim.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Secondary => "secondary",
        }
    }

    /// Parse a stored value, `None` outside the roster.
    ///
    /// Fail-closed: a row outside the two is a row the `CHECK` should have
    /// refused, so the reader reports a corrupt row rather than guessing a
    /// role. It is never a caller's mistake — a request-borne value is the
    /// door's to validate.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "primary" => Some(Self::Primary),
            "secondary" => Some(Self::Secondary),
            _ => None,
        }
    }
}

/// One attribute definition's stored state
/// (`cpt-cf-bss-products-state-attribute-definition`).
///
/// # Why this is not `domain::recognized::MemberState`
///
/// The two rosters spell the same three tokens today, and reusing 03's would
/// still be wrong: `MemberState` carries `SetKind::delist_blocked` and
/// `member_edge`, which bind it to 03's four sets, 03's three refusal codes
/// and 03's `state-recognized-set` machine. This roster answers to §4's
/// **attribute-definition** machine, whose edges differ — it declares both
/// re-listings, and §7 row 10 asks whether its removal is a material op at
/// all. Sharing the type would put 02's machine inside 03's, which is the
/// redefinition [`crate::domain::live_op::GovernedLiveOp`] keeps its `kind`
/// an open string to avoid, in the other direction.
///
/// The edges themselves are **not** here: this type is the stored roster the
/// table reads back. `inst-de-edge-*` is `dod-definition-lifecycle`'s.
#[domain_model]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DefinitionState {
    /// In the roster; admits new values.
    Active,
    /// In the roster; refuses new values, existing ones keep resolving.
    Deprecated,
    /// The tombstone. Reachable only as a flip — the table's own trigger
    /// refuses every `DELETE` (P-D-47, `inst-de-no-delete`) — so a value on a
    /// terminal head keeps resolving and no `products_attribute_value` row is
    /// ever orphaned.
    Removed,
}

impl DefinitionState {
    /// The stored spelling — `chk_products_attribute_definition_state`'s
    /// roster, verbatim.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Deprecated => "deprecated",
            Self::Removed => "removed",
        }
    }

    /// Parse a stored value, `None` outside the roster. Fail-closed for the
    /// reason [`AssignmentRole::parse`] gives.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "deprecated" => Some(Self::Deprecated),
            "removed" => Some(Self::Removed),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "taxonomy_tests.rs"]
mod taxonomy_tests;
