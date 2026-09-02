//! The taxonomy's domain values: the ancestor chain a re-parent is judged
//! against, the retire guard's verdict over a census its caller read, the
//! extensibility limits' judge, the two stored rosters its content tables key
//! on, and the well-known seed roster
//! (`design/02-taxonomy-attributes.md` §3.1 `inst-tx-walk`,
//! `inst-tx-retire-guard`, §3.4 `inst-tc-writer-lock`;
//! `inst-tx-name-in-parent`; §4.1's `role` and `state` columns; §4.2's seeds).
//!
//! # Nothing here reads a store
//!
//! Every judgement below takes a census the caller read **under the writer
//! lock** and answers over it. That is `inst-tx-walk`'s own reason, applied
//! past the walk: *"single-writer is what makes `TaxonomyWalk`'s verdict
//! trustworthy"*. A guard that fetched its own rows would be judging a tree
//! that can move between the fetch and the write, and no single-writer test
//! would see the difference.
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

/// How deep a node sits: `0` for a root, `1` for its child.
///
/// Derived from [`ancestors_of`] rather than counted separately, so the depth
/// a limit is judged against and the chain a cycle is judged against can
/// never disagree — including on the pre-existing-cycle case, where the walk
/// stops on a repeat and this therefore reports a finite depth rather than
/// looping.
#[must_use]
pub fn depth_of(start: Uuid, parent_of: &impl Fn(Uuid) -> Option<Uuid>) -> u32 {
    // `ancestors_of` always contains at least `start` itself, so the
    // subtraction cannot wrap; `saturating_sub` states that rather than
    // relying on the reader to check.
    u32::try_from(ancestors_of(start, parent_of).len())
        .unwrap_or(u32::MAX)
        .saturating_sub(1)
}

/// How many children one parent holds, `None` counting the roots.
///
/// Takes the same edge list [`ancestors_of`]'s `parent_of` is built from, so
/// one read under the lock serves the cycle rule, the depth rule and this.
#[must_use]
pub fn children_of(parent: Option<Uuid>, edges: &[(Uuid, Option<Uuid>)]) -> u32 {
    u32::try_from(edges.iter().filter(|(_, p)| *p == parent).count()).unwrap_or(u32::MAX)
}

/// The configured extensibility limits (`nfr-scale-extensibility`'s
/// extensibility-limits half).
///
/// # Both are `None` at this commit, and that is a measurement rather than a
/// default
///
/// `design/02` §6 records that *"The taxonomy and metadata limits have no
/// interim default anywhere"*, and the feature's §7 row 2 names their owner:
/// the PRD §17.1 policy owner, whose section carries neither a taxonomy-limits
/// row nor a metadata-caps row. So `None` here does not mean *unlimited as a
/// policy*; it means **no threshold has been stated**, and this type carries
/// no `Default` precisely so that nothing can acquire one by accident. A
/// number invented here would be a policy this slice has no standing to set,
/// and `ProductsConfig` grows no field for it until that owner acts.
///
/// The judgement is built and unreachable, which is the honest shape for a
/// rule whose operand is owed: [`limit_verdict`] is correct for whatever
/// thresholds arrive, and no caller can supply one today.
#[domain_model]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TaxonomyLimits {
    /// Maximum depth a create or re-parent may reach, `None` when unstated.
    pub max_depth: Option<u32>,
    /// Maximum children one node may hold, `None` when unstated.
    pub max_children: Option<u32>,
}

/// Which limit a mutation would exceed, and by what.
///
/// Carries the operands rather than a rendered string, because
/// `dod-taxonomy-walk` requires the refusal **name the limit** and a caller
/// that had only a message would have to parse it back out.
#[domain_model]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TaxonomyLimitExceeded {
    /// `max_depth` or `max_children`, in the design's own spelling.
    pub limit: &'static str,
    /// The configured threshold.
    pub allowed: u32,
    /// What the mutation would make it.
    pub measured: u32,
}

impl TaxonomyLimitExceeded {
    /// The refusal code this maps to (`design/02` §3.3).
    ///
    /// **Declared and unraised**, and its `DomainError` variant does not exist
    /// at this commit: the code is one of twelve of this feature's sixteen
    /// still absent from `domain::error`, which is `dod-taxonomy-errors`'
    /// work. The constant lives here so that when the variant lands there is
    /// one spelling and not two.
    pub const CODE: &'static str = "TAXONOMY_LIMIT";
}

/// Judge one mutation against the limits.
///
/// # Only the mutation path calls this, which is the third `MUST`
///
/// `dod-taxonomy-walk`: *"Limits **MUST** be validated on the mutation path
/// only, so a later limit decrease never invalidates existing structure"*.
/// This function takes what a mutation **would** make the tree and never the
/// tree as it stands, so there is no reading of it that judges existing rows
/// — an over-limit subtree left by an earlier, looser configuration answers
/// nothing here because nothing asks it.
///
/// # Errors
///
/// [`TaxonomyLimitExceeded`] naming the first limit exceeded, depth before
/// children — an order, not a precedence claim: `design/02` states none, and
/// the feature's own §7 records that this feature's sixteen codes carry no
/// precedence at all.
pub fn limit_verdict(
    depth: u32,
    children_in_parent: u32,
    limits: TaxonomyLimits,
) -> Result<(), TaxonomyLimitExceeded> {
    if let Some(allowed) = limits.max_depth
        && depth > allowed
    {
        return Err(TaxonomyLimitExceeded {
            limit: "max_depth",
            allowed,
            measured: depth,
        });
    }
    if let Some(allowed) = limits.max_children
        && children_in_parent > allowed
    {
        return Err(TaxonomyLimitExceeded {
            limit: "max_children",
            allowed,
            measured: children_in_parent,
        });
    }
    Ok(())
}

/// The census a retire or delete is judged against
/// (`inst-tx-retire-guard`: retired + empty + unreferenced).
///
/// # Both samples are the caller's read, and both are bounded
///
/// The caller reads them under the writer lock and bounds each at
/// `sample + 1`, so [`retire_verdict`] can say *"at least N"* honestly
/// without a second statement whose total could disagree with its own
/// exemplars — the failure `repo::metering_unit_holders` documents on the
/// sibling guard.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RetireCensus {
    /// **Non-terminal** Products filing under the node, in either role,
    /// rendered as the refusal should name them.
    ///
    /// The operand is the Product's own `lifecycle_state` and **never the
    /// presence of a `products_product_category` row** — `dod-retire-delete-guard`
    /// says so in as many words, and the difference is the whole point: a
    /// discarded draft keeps its link row, so a guard reading row presence
    /// would refuse forever on catalog nobody can transact against.
    pub referencing_products: Vec<String>,
    /// `active` child categories, rendered the same way. A `retired` child
    /// does not block: `inst-ce-terminal` makes deletion the retired node's
    /// own exit, so a retired child is on its way out rather than in use.
    pub active_children: Vec<String>,
    /// The bound each sample was read at, so the refusal can distinguish
    /// *this many* from *at least this many*.
    pub sample_bound: usize,
}

/// A retire or delete refused because the node is still in use.
///
/// Carries the rendered detail; the wire code is [`Self::CODE`]. **Not a
/// [`DomainError`]**, because `CATEGORY_REFERENCED` has no variant at this
/// commit — see [`TaxonomyLimitExceeded::CODE`] for the same situation and
/// the same reason. The door maps this to a refusal exactly as it maps
/// `repo::AssignmentWrite`'s two conflicts, and the mapping lands with
/// `dod-taxonomy-errors`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CategoryReferenced {
    /// What is still holding the node, for a human reading the response.
    pub detail: String,
}

impl CategoryReferenced {
    /// The refusal code (`design/02` §3.3).
    pub const CODE: &'static str = "CATEGORY_REFERENCED";
}

/// Judge a retire or delete against its census.
///
/// # Errors
///
/// [`CategoryReferenced`] naming a sample of whatever holds the node. Both
/// halves are reported in one refusal rather than the first found, because an
/// operator who clears the Products only to meet the children next has been
/// told half the truth twice.
pub fn retire_verdict(census: &RetireCensus) -> Result<(), CategoryReferenced> {
    if census.referencing_products.is_empty() && census.active_children.is_empty() {
        return Ok(());
    }
    let mut parts = Vec::new();
    if !census.referencing_products.is_empty() {
        parts.push(format!(
            "{} non-terminal product(s) file under it ({})",
            count_phrase(census.referencing_products.len(), census.sample_bound),
            census.referencing_products.join(", ")
        ));
    }
    if !census.active_children.is_empty() {
        parts.push(format!(
            "{} active child category(ies) ({})",
            count_phrase(census.active_children.len(), census.sample_bound),
            census.active_children.join(", ")
        ));
    }
    Err(CategoryReferenced {
        detail: format!(
            "the category is still in use and cannot be retired or deleted: {}",
            parts.join("; and ")
        ),
    })
}

/// *"3"* or *"at least 3"*, the second when the sample hit its bound.
///
/// The caller reads `bound + 1` rows, so a sample longer than the bound is
/// the signal that more exist — which is why this compares against `bound`
/// and not against `bound + 1`.
fn count_phrase(found: usize, bound: usize) -> String {
    if found > bound {
        format!("at least {bound}")
    } else {
        found.to_string()
    }
}

/// One well-known attribute definition the registry seeds (`design/02` §4.2).
///
/// # `value_type`'s spelling is this module's proposal, not the design's
///
/// `dod-well-known-seeds` names five keys, says which are localized, and
/// describes three **shapes** — *"a localized string"*, *"a URI string"*, *"a
/// localized string list"*. It names no **tokens**, and no document in the set
/// enumerates the admitted `value_type` values at all: the column ships pinned
/// to non-emptiness only, on P-D-74's shape, precisely so that the roster
/// stays its owner's. The three constants below are therefore a **proposal**
/// carried in the owed register, not a decided set — and nothing closes it:
/// `repo::NewAttributeDefinition::value_type` is still a `&str`, so an owner
/// who picks other spellings changes these three lines and no signature.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct WellKnownSeed {
    /// The tenant-unique key.
    pub key: &'static str,
    /// The declared shape. See the type doc: a proposal.
    pub value_type: &'static str,
    /// Whether values carry locale coordinates.
    pub localized: bool,
}

/// The `seeded_by` marker a registry seed carries.
///
/// `dod-well-known-seeds` pins the literal: *"marked `seeded_by =
/// 'registry'`"*.
pub const REGISTRY_SEEDED_BY: &str = "registry";

/// The five seeds `dod-well-known-seeds` enumerates, in the order it lists
/// them.
///
/// # This roster has no writer, and that is a blocker rather than an omission
///
/// The `DoD` requires them *"per tenant bootstrap, by migration"*. Those are
/// two code paths, which the feature's §7 row 23 already records — and
/// measured at this commit **neither is available**: `migrations/` is not this
/// strand's to write, and the gear has **no tenant-bootstrap hook of any
/// kind** for the other path, so there is nothing for a per-tenant seeder to
/// hang off. The roster is declared here so that whichever path its owner
/// chooses has one definition site rather than a second copy; no seeder is
/// invented, and the `DoD` stays unticked.
pub const WELL_KNOWN_SEEDS: [WellKnownSeed; 5] = [
    WellKnownSeed {
        key: "displayName",
        value_type: "localized_string",
        localized: true,
    },
    WellKnownSeed {
        key: "description",
        value_type: "localized_string",
        localized: true,
    },
    WellKnownSeed {
        key: "imageUri",
        value_type: "uri_string",
        localized: false,
    },
    WellKnownSeed {
        key: "unitDisplayLabel",
        value_type: "localized_string",
        localized: true,
    },
    WellKnownSeed {
        key: "marketingFeatures",
        value_type: "localized_string_list",
        localized: true,
    },
];

/// Whether a definition may reach [`DefinitionState::Removed`].
///
/// `dod-well-known-seeds`: *"A seeded definition **MUST** be deprecatable and
/// **MUST NOT** be removable."* So the operand is the `seeded_by` marker and
/// nothing else — not the key, which an operator may reuse in another tenant,
/// and not the state, which is what the edge machine judges separately.
///
/// The complement is the load-bearing direction: this returns `true` for an
/// operator-added definition, so a caller cannot satisfy the rule by refusing
/// every removal.
#[must_use]
pub const fn is_removable(seeded_by: Option<&str>) -> bool {
    seeded_by.is_none()
}

#[cfg(test)]
#[path = "taxonomy_tests.rs"]
mod taxonomy_tests;
