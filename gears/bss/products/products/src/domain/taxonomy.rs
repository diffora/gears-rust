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

use crate::domain::containment::ResolvedScope;
use crate::domain::error::DomainError;
use crate::domain::validation::{Phase, ValidationPipeline, ValidationReport, ValidationRule};

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

/// One category's stored state
/// (`cpt-cf-bss-products-state-category`).
///
/// Two members and no edge back from [`Self::Retired`]: `inst-ce-terminal`
/// makes physical deletion the retired node's own exit, and §4 records the
/// absent `retired -> active` edge as an asymmetry rather than a gap. Parsed
/// because `chk_products_category_state` closes the roster.
#[domain_model]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CategoryState {
    /// Open to new assignment.
    Active,
    /// Closed to new assignment, awaiting deletion.
    Retired,
}

impl CategoryState {
    /// The stored spelling -- `chk_products_category_state`'s roster.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Retired => "retired",
        }
    }

    /// Parse a stored value, `None` outside the roster. Fail-closed for the
    /// reason [`AssignmentRole::parse`] gives.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "retired" => Some(Self::Retired),
            _ => None,
        }
    }
}

// -- The content-save subject and its validators (`inst-av-validate`,
//    `inst-tx-assign`) --
//
// `ValidationRule::evaluate` is **synchronous**, so a rule whose operand
// lives in another table cannot fetch it. The door reads the fact and the
// subject carries it -- exactly why `rules::PublishedTransitionSubject`
// exists beside `CreateEntityCandidate` rather than extending it, and the
// same reason applies here to a whole content payload.
//
// Every rule below is `Phase::RegisteredValidators`, following this slice's
// own precedent (`rules::PrimaryCategoryRequired`) and that phase's own doc,
// *"each feature's contributed rules, in registration order"*. One phase
// means one rejection carrying every content violation, which is what a save
// door wants: an operator fixing four fields should not need four round
// trips. It also means each rule must skip what it cannot judge -- an
// unresolved definition produces exactly one violation and not four, which
// `an_unresolved_definition_raises_one_violation_and_not_four` holds.

/// The definition state machine's four edges
/// (`cpt-cf-bss-products-state-attribute-definition`, `inst-de-edge-*`).
///
/// Both re-listings are declared -- `deprecated -> active` and
/// `removed -> active` -- because §4 declares them, and re-listing *"the same
/// identity, which never changed"* is what makes the tombstone a tombstone
/// rather than a grave.
///
/// `active -> removed` is **not** an edge: `inst-de-deprecate-then-remove`
/// puts deprecation between them, so the destructive step cannot be reached
/// in one act. Nor is any edge out of a state to itself: a no-op flip would
/// consume a `GovernedLiveOp` and emit an event for nothing.
///
/// # Errors
///
/// [`DomainError::IllegalTransition`] for every pair outside the four.
pub fn definition_edge(from: DefinitionState, to: DefinitionState) -> Result<(), DomainError> {
    let admitted = matches!(
        (from, to),
        (DefinitionState::Active, DefinitionState::Deprecated)
            | (
                DefinitionState::Deprecated,
                DefinitionState::Removed | DefinitionState::Active
            )
            | (DefinitionState::Removed, DefinitionState::Active)
    );
    if admitted {
        return Ok(());
    }
    Err(DomainError::IllegalTransition {
        from: from.as_str().to_owned(),
        to: to.as_str().to_owned(),
    })
}

/// A definition that cannot move because something still carries its values.
///
/// Carries the rendered detail; the wire code is [`Self::CODE`]. **Not a
/// [`DomainError`]**, for the reason [`CategoryReferenced`] gives -- the
/// variant does not exist at this commit and `dod-taxonomy-errors` is where
/// it lands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefinitionInUse {
    /// What still carries values, for a human reading the response.
    pub detail: String,
}

impl DefinitionInUse {
    /// The refusal code (`design/02` §3.3, 409).
    pub const CODE: &'static str = "DEFINITION_IN_USE";
}

/// Judge one definition act against a census of what still carries its
/// values.
///
/// # The census is the caller's, and which census is §7 row 11's question
///
/// `dod-definition-lifecycle` states **two** operands in one sentence:
/// *"undefined 'live values'"* for a type change and *"the defined
/// non-terminal head"* for removal -- which is exactly what row 11 asks
/// about. This function therefore takes whatever census it is handed and
/// judges it; it does not decide which one a given act should read. The
/// removal operand is defined and `repo::definition_value_holders` reads it;
/// the type change's is not, and pointing that function at a type change is
/// the caller's act, not this one's.
///
/// # Errors
///
/// [`DefinitionInUse`] naming a sample of the holders.
pub fn definition_in_use_verdict(holders: &[String], bound: usize) -> Result<(), DefinitionInUse> {
    if holders.is_empty() {
        return Ok(());
    }
    Err(DefinitionInUse {
        detail: format!(
            "the definition still has values on {} live carrier(s) ({})",
            count_phrase(holders.len(), bound),
            holders.join(", ")
        ),
    })
}

/// Whether a seeded definition may take the edge it is being asked to take.
///
/// `dod-well-known-seeds`: a seeded definition *"**MUST** be deprecatable and
/// **MUST NOT** be removable"*. Both halves, so a guard refusing every act on
/// a seed would fail this as surely as one refusing none.
///
/// # Errors
///
/// [`DomainError::IllegalFieldMutation`] on a seeded definition's removal.
/// The variant is the Foundation's *"this field may not move"* refusal and is
/// the nearest declared one: `design/02` §3.3 declares no code for a seeded
/// removal, and the feature's §7 row 17 lists it among the four refusals that
/// have none. Minting one would answer that row from a rule.
pub fn seeded_edge(seeded_by: Option<&str>, to: DefinitionState) -> Result<(), DomainError> {
    if to == DefinitionState::Removed && !is_removable(seeded_by) {
        return Err(DomainError::IllegalFieldMutation(format!(
            "a definition seeded by `{}` is deprecatable but never removable",
            seeded_by.unwrap_or_default()
        )));
    }
    Ok(())
}

/// One category assignment as the save door presents it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssignmentCandidate {
    /// The category the payload names.
    pub category_id: Uuid,
    /// The role it names it in.
    pub role: AssignmentRole,
    /// The category's state as the **door** read it; `None` where the tenant
    /// has no such category. A rule cannot read this for itself.
    pub resolved: Option<CategoryState>,
}

/// One attribute definition as the save door resolved it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedDefinition {
    /// The stored state -- the set is `active` and `deprecated`; `removed` is
    /// a tombstone **outside** it, which is `repo::recognized`'s own wording
    /// for the sibling roster.
    pub state: DefinitionState,
    /// The declared type token. Open: see [`ValueShape::of`].
    pub value_type: String,
    /// Whether the definition takes locale coordinates.
    pub localized: bool,
    /// P-D-39 rendering -- `""` is **unrestricted**, not empty.
    pub region_scope: String,
    /// Same reading.
    pub brand_scope: String,
}

/// One attribute value as the save door presents it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValueCandidate {
    /// The definition key the payload names -- what a refusal quotes back.
    pub definition_key: String,
    /// `""` is absent, not null (**P-D-88** arm 2).
    pub locale: String,
    /// `""` is absent.
    pub region: String,
    /// `""` is absent.
    pub brand: String,
    /// The value being written.
    pub value: String,
    /// The definition the door resolved, `None` where the tenant has none.
    pub resolved: Option<ResolvedDefinition>,
}

/// What a content save presents to the registered validators.
///
/// Carries the entity's own two scope columns beside the payload because
/// `inst-av-validate` judges a coordinate against *both* the definition's
/// visibility scope **and** the entity's own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContentSaveSubject {
    /// The whole assignment set the payload names. A replace, so this is the
    /// set as it will stand, not a delta.
    pub assignments: Vec<AssignmentCandidate>,
    /// The values the payload writes.
    pub values: Vec<ValueCandidate>,
    /// The entity's stored `region_scope`. `""` is unrestricted.
    pub entity_region_scope: String,
    /// The entity's stored `brand_scope`. Same reading.
    pub entity_brand_scope: String,
}

/// The three value shapes §4.2's seeds describe.
///
/// # The tokens are a proposal; the shapes are the design's words
///
/// `inst-av-validate` requires a value *"match the declared type"* and names
/// no roster; §4.2 describes three shapes -- *"a localized string"*, *"a URI
/// string"*, *"a localized string list"* -- and no tokens. So [`Self::of`]
/// maps the three constants
/// [`WELL_KNOWN_SEEDS`] proposes and answers `None` for everything else, and
/// an unmapped token is **not judged**: the gear cannot decide whether a value
/// matches a type whose meaning nothing states. That is the same posture
/// [`TaxonomyLimits`] takes toward an unstated threshold, and for the same
/// reason -- a rule with no operand must not invent one.
#[domain_model]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ValueShape {
    /// Any string. Constrains nothing on its own; the `localized` flag is
    /// what decides whether it takes coordinates.
    LocalizedString,
    /// An absolute URI: a scheme, a colon, and something after it.
    UriString,
    /// A JSON array of strings.
    LocalizedStringList,
}

impl ValueShape {
    /// Map a declared type token to a shape, `None` for a token the gear does
    /// not know. See the type doc: the three tokens are this feature's
    /// proposal and `design/02` §6 owes the roster.
    #[must_use]
    pub fn of(value_type: &str) -> Option<Self> {
        match value_type {
            "localized_string" => Some(Self::LocalizedString),
            "uri_string" => Some(Self::UriString),
            "localized_string_list" => Some(Self::LocalizedStringList),
            _ => None,
        }
    }

    /// Whether `value` has this shape.
    #[must_use]
    pub fn admits(self, value: &str) -> bool {
        match self {
            Self::LocalizedString => true,
            // Scheme, colon, and a non-empty remainder. Deliberately not a
            // URI crate: the check that matters here is that a caller did not
            // put a display name in an image field, and a full RFC 3986 parse
            // would refuse valid URIs this gear has no business judging.
            Self::UriString => value.split_once(':').is_some_and(|(scheme, rest)| {
                !scheme.is_empty()
                    && !rest.is_empty()
                    && scheme
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
                    && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            }),
            Self::LocalizedStringList => serde_json::from_str::<Vec<String>>(value).is_ok(),
        }
    }
}

/// The generic code a refusal takes while its own is unassigned.
///
/// `design/02` §3.3 declares sixteen codes and the feature's §7 row 17 records
/// that **four refusals in this feature have no code at all** -- among them
/// the unresolvable category and the primary/secondary duplicate, both of
/// which are rules below. Minting a seventeenth here would answer that row
/// from a rule; raising the Foundation's declared generic keeps the refusal
/// **reachable** and leaves the code to row 17's owner. The `subject` and
/// `detail` a violation carries are what tells the two apart meanwhile.
const UNASSIGNED_CODE: &str = "VALIDATION";

/// A payload naming a category the tenant does not have.
///
/// @cpt-cf-bss-products-dod-assignment-validators
///
/// Raises [`UNASSIGNED_CODE`]: §7 row 17 lists *"the unresolvable category"*
/// among the four refusals with no code of their own.
pub struct CategoryResolvableRule;

impl CategoryResolvableRule {
    /// The code this rule raises. See [`UNASSIGNED_CODE`].
    pub const CODE: &'static str = UNASSIGNED_CODE;
}

impl ValidationRule<ContentSaveSubject> for CategoryResolvableRule {
    fn name(&self) -> &'static str {
        "inst-tx-assign/resolvable"
    }

    fn phase(&self) -> Phase {
        Phase::RegisteredValidators
    }

    fn evaluate(&self, subject: &ContentSaveSubject, report: &mut ValidationReport) {
        for candidate in &subject.assignments {
            if candidate.resolved.is_none() {
                report.violate(
                    Self::CODE,
                    "categories",
                    format!(
                        "category {} does not exist in this tenant",
                        candidate.category_id
                    ),
                );
            }
        }
    }
}

/// A payload filing under a retired category.
///
/// @cpt-cf-bss-products-dod-assignment-validators
///
/// `inst-ce-edge-retire` closes a retired node *"to new assignment"*, so this
/// judges the **payload's** set rather than the stored one: a set already
/// containing the node keeps it, and only a save that names it again is
/// refused. A rule reading the stored set instead would make every later save
/// of an untouched Product fail the day one of its categories retired.
pub struct CategoryNotRetiredRule;

impl CategoryNotRetiredRule {
    /// The refusal code (`design/02` §3.3, 422 architectural).
    pub const CODE: &'static str = "CATEGORY_RETIRED";
}

impl ValidationRule<ContentSaveSubject> for CategoryNotRetiredRule {
    fn name(&self) -> &'static str {
        "inst-ce-edge-retire/no-new-assignment"
    }

    fn phase(&self) -> Phase {
        Phase::RegisteredValidators
    }

    fn evaluate(&self, subject: &ContentSaveSubject, report: &mut ValidationReport) {
        for candidate in &subject.assignments {
            if candidate.resolved == Some(CategoryState::Retired) {
                report.violate(
                    Self::CODE,
                    "categories",
                    format!(
                        "category {} is retired and is closed to new assignment",
                        candidate.category_id
                    ),
                );
            }
        }
    }
}

/// One category named twice in one payload.
///
/// @cpt-cf-bss-products-dod-assignment-validators
///
/// `uq_products_product_category` refuses this at the engine too, and the
/// rule is not redundant: the index answers a driver conflict the door must
/// translate, while this answers a per-field violation beside every other
/// content violation in the same rejection. The `DoD` asks for the validator by
/// name.
///
/// Raises [`UNASSIGNED_CODE`] -- §7 row 17's *"primary/secondary duplicate"*.
pub struct CategoryRoleConflictRule;

impl CategoryRoleConflictRule {
    /// The code this rule raises. See [`UNASSIGNED_CODE`].
    pub const CODE: &'static str = UNASSIGNED_CODE;
}

impl ValidationRule<ContentSaveSubject> for CategoryRoleConflictRule {
    fn name(&self) -> &'static str {
        "inst-tx-assign/one-role-per-category"
    }

    fn phase(&self) -> Phase {
        Phase::RegisteredValidators
    }

    fn evaluate(&self, subject: &ContentSaveSubject, report: &mut ValidationReport) {
        for (index, candidate) in subject.assignments.iter().enumerate() {
            // Compare forward only, so one duplicated pair raises one
            // violation rather than two mirror-image ones.
            let duplicated = subject.assignments[index + 1..]
                .iter()
                .any(|other| other.category_id == candidate.category_id);
            if duplicated {
                report.violate(
                    Self::CODE,
                    "categories",
                    format!(
                        "category {} is named more than once; one product holds one category in \
                         one role",
                        candidate.category_id
                    ),
                );
            }
        }
    }
}

/// A value against a definition outside the tenant's set.
///
/// @cpt-cf-bss-products-dod-value-validators
///
/// **A `removed` definition is refused here and not by
/// [`AttributeDefinitionActiveRule`].** The tombstone is a row that exists and
/// is *outside the set* -- `repo::recognized`'s own words for the sibling
/// roster: *"the set is the `active` and `deprecated` rows; a `removed` row is
/// a tombstone outside it"*. It survives so a value on a terminal head keeps
/// **resolving**; it never admits a new **write**.
pub struct AttributeDefinitionKnownRule;

impl AttributeDefinitionKnownRule {
    /// The refusal code (`design/02` §3.3).
    pub const CODE: &'static str = "ATTRIBUTE_DEFINITION_UNKNOWN";
}

impl ValidationRule<ContentSaveSubject> for AttributeDefinitionKnownRule {
    fn name(&self) -> &'static str {
        "inst-av-validate/definition-known"
    }

    fn phase(&self) -> Phase {
        Phase::RegisteredValidators
    }

    fn evaluate(&self, subject: &ContentSaveSubject, report: &mut ValidationReport) {
        for candidate in &subject.values {
            let outside = match candidate.resolved {
                None => true,
                Some(ref definition) => definition.state == DefinitionState::Removed,
            };
            if outside {
                report.violate(
                    Self::CODE,
                    format!("attributes.{}", candidate.definition_key),
                    format!(
                        "`{}` is not a definition in this tenant's set",
                        candidate.definition_key
                    ),
                );
            }
        }
    }
}

/// A value against a deprecated definition.
///
/// @cpt-cf-bss-products-dod-value-validators
///
/// `inst-de-edge-deprecate`: new values are refused and existing ones keep
/// resolving. So this judges the **write**, never the read.
pub struct AttributeDefinitionActiveRule;

impl AttributeDefinitionActiveRule {
    /// The refusal code (`design/02` §3.3).
    pub const CODE: &'static str = "ATTRIBUTE_DEFINITION_DEPRECATED";
}

impl ValidationRule<ContentSaveSubject> for AttributeDefinitionActiveRule {
    fn name(&self) -> &'static str {
        "inst-av-validate/definition-active"
    }

    fn phase(&self) -> Phase {
        Phase::RegisteredValidators
    }

    fn evaluate(&self, subject: &ContentSaveSubject, report: &mut ValidationReport) {
        for candidate in &subject.values {
            if candidate
                .resolved
                .as_ref()
                .is_some_and(|d| d.state == DefinitionState::Deprecated)
            {
                report.violate(
                    Self::CODE,
                    format!("attributes.{}", candidate.definition_key),
                    format!(
                        "`{}` is deprecated and admits no new values; existing ones keep resolving",
                        candidate.definition_key
                    ),
                );
            }
        }
    }
}

/// A value whose shape does not match its definition's declared type.
///
/// @cpt-cf-bss-products-dod-value-validators
///
/// **Inert for a type token the gear does not know**, which is every token
/// outside [`ValueShape::of`]'s three -- see that method on why the roster is
/// owed rather than closed here. A rule that refused an unmapped token would
/// close the feature to every operator-defined type; one that admitted it
/// silently would be indistinguishable from this, so the difference is
/// stated rather than left to be inferred.
pub struct AttributeValueTypeRule;

impl AttributeValueTypeRule {
    /// The refusal code (`design/02` §3.3).
    pub const CODE: &'static str = "ATTRIBUTE_TYPE_MISMATCH";
}

impl ValidationRule<ContentSaveSubject> for AttributeValueTypeRule {
    fn name(&self) -> &'static str {
        "inst-av-validate/type-match"
    }

    fn phase(&self) -> Phase {
        Phase::RegisteredValidators
    }

    fn evaluate(&self, subject: &ContentSaveSubject, report: &mut ValidationReport) {
        for candidate in &subject.values {
            let Some(ref definition) = candidate.resolved else {
                continue;
            };
            let Some(shape) = ValueShape::of(&definition.value_type) else {
                continue;
            };
            if !shape.admits(&candidate.value) {
                report.violate(
                    Self::CODE,
                    format!("attributes.{}", candidate.definition_key),
                    format!(
                        "the value does not match `{}`'s declared type `{}`",
                        candidate.definition_key, definition.value_type
                    ),
                );
            }
        }
    }
}

/// A coordinate outside the definition's visibility scope or the entity's own.
///
/// @cpt-cf-bss-products-dod-value-validators
///
/// # The empty set is unrestricted, on both sides
///
/// **P-D-39.** A definition whose `brand_scope` is `""` is visible to every
/// brand, not to none, and the same for the entity's columns. A predicate
/// written as set membership alone would refuse every coordinate under every
/// unrestricted row -- which is to say, under nearly all of them.
///
/// # An absent coordinate is not judged, and that is §6's open item, deferred
///
/// *"Does a brand-less global value survive the scope check on a
/// brand-scoped entity?"* -- `design/02` §6, whose own text records that under
/// a containment-only reading *"the write the publish validator demands is the
/// write the save validator refuses"*, so a brand-scoped entity could never
/// publish. This rule judges a coordinate **only where the payload names
/// one**: `brand: ""` is the absence P-D-88 arm 2 spells, not a brand called
/// empty-string, and there is nothing to contain. That is the one direction
/// in which both `dod-value-validators` and `dod-default-locale` are
/// satisfiable at once, so it is taken as forced rather than chosen -- and it
/// is registered, because the owner may yet want the global value scoped some
/// other way.
pub struct AttributeScopeRule;

impl AttributeScopeRule {
    /// The refusal code (`design/02` §3.3).
    pub const CODE: &'static str = "ATTRIBUTE_SCOPE_VIOLATION";

    /// Whether `named` is admitted by a stored scope column.
    ///
    /// `""` on the column is unrestricted (P-D-39); `""` for the named
    /// coordinate is *absent* and is not judged -- see the type doc.
    fn admitted(named: &str, stored: &str) -> bool {
        if named.is_empty() {
            return true;
        }
        match ResolvedScope::parse(stored) {
            Ok(ResolvedScope::Unrestricted) => true,
            Ok(ResolvedScope::Restricted(values)) => values.contains(named),
            // A column that will not parse is a corrupt row, not an
            // admission: `ResolvedScope::parse` refuses an empty token
            // between separators, and fail-closed is this gear's principle.
            Err(_) => false,
        }
    }
}

impl ValidationRule<ContentSaveSubject> for AttributeScopeRule {
    fn name(&self) -> &'static str {
        "inst-av-validate/coordinate-scope"
    }

    fn phase(&self) -> Phase {
        Phase::RegisteredValidators
    }

    fn evaluate(&self, subject: &ContentSaveSubject, report: &mut ValidationReport) {
        for candidate in &subject.values {
            let Some(ref definition) = candidate.resolved else {
                continue;
            };
            for (axis, named, definition_scope, entity_scope) in [
                (
                    "region",
                    candidate.region.as_str(),
                    definition.region_scope.as_str(),
                    subject.entity_region_scope.as_str(),
                ),
                (
                    "brand",
                    candidate.brand.as_str(),
                    definition.brand_scope.as_str(),
                    subject.entity_brand_scope.as_str(),
                ),
            ] {
                if !Self::admitted(named, definition_scope) {
                    report.violate(
                        Self::CODE,
                        format!("attributes.{}", candidate.definition_key),
                        format!(
                            "{axis} `{named}` is outside `{}`'s visibility scope",
                            candidate.definition_key
                        ),
                    );
                } else if !Self::admitted(named, entity_scope) {
                    report.violate(
                        Self::CODE,
                        format!("attributes.{}", candidate.definition_key),
                        format!("{axis} `{named}` is outside the entity's own scope"),
                    );
                }
            }
        }
    }
}

// -- The locale fallback chain (`inst-av-resolve`, `inst-av-default-locale`) --

/// One stored value with its coordinate, as the resolver reads it.
///
/// The three coordinates carry P-D-88 arm 2's spelling: `""` is **absent**,
/// not null and not a value named empty-string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalizedValue {
    /// `""` is absent.
    pub locale: String,
    /// `""` is absent.
    pub region: String,
    /// `""` is absent.
    pub brand: String,
    /// The value itself.
    pub value: String,
}

/// What a reader is asking for.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct LocaleRequest<'a> {
    /// The reader's locale.
    pub locale: &'a str,
    /// The reader's region.
    pub region: &'a str,
    /// The reader's brand.
    pub brand: &'a str,
    /// The tenant's configured default locale — `ProductsConfig::default_locale`
    /// (**P-D-101**).
    ///
    /// **A preference, not the anchor.** `inst-av-resolve`'s item-37 note is
    /// explicit: *"Totality is anchored on the resolution path, not on the
    /// config value ... the tenant default locale is ungoverned config with no
    /// re-validation, so anchoring on it would un-total the chain for every
    /// already-published entity the moment it changed."* So this is consulted
    /// at step 3 and the chain still ends at the global coordinate.
    ///
    /// **`""` skips step 3 rather than keying it on the empty string**, which
    /// the config field's own doc states: *"an unset default locale skips step
    /// 3 and resolution still succeeds."* Running the step with `""` would key
    /// it on `("", "", brand)` — a brand-scoped, locale-less value, which is a
    /// real coordinate a caller can write and **not** the tenant default for
    /// that brand. A reader with no configured default would then be handed
    /// that value ahead of the global one, which is a different answer, not a
    /// shorter path.
    pub tenant_default_locale: &'a str,
}

/// Which step of `inst-av-resolve`'s chain answered.
///
/// Reported so a matrix fixture can assert **which** step resolved rather
/// than only that something did — a resolver whose first step matched
/// everything would satisfy a value-only assertion at every row of the
/// matrix.
#[domain_model]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ResolutionStep {
    /// `(locale, region, brand)` — the reader's exact coordinate.
    Exact,
    /// `(locale, brand)` — region dropped.
    LocaleAndBrand,
    /// `(default-locale, brand)` — the tenant preference, brand kept.
    DefaultLocaleAndBrand,
    /// `("", "", "")` — the global coordinate, which is what makes the chain
    /// total for every brand.
    Global,
}

/// A resolved value and the step that produced it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Resolved<'v> {
    /// The value.
    pub value: &'v str,
    /// Which step answered.
    pub step: ResolutionStep,
}

/// Walk `inst-av-resolve`'s chain over one definition's stored values.
///
/// `(locale, region, brand) -> (locale, brand) -> (default-locale, brand) ->
/// global`, in that order, first hit wins.
///
/// # The chain drops `region` after the first step, and that is the design's
///
/// Steps 2 and 3 name only a locale and a brand, so both look for a value
/// whose `region` is **absent** — a region-specific value is reachable only by
/// a reader who names that region. Nothing in the chain widens a regional
/// value to a neighbouring region, which is what a `region`-insensitive step 2
/// would silently do.
///
/// # `""` in the request is a coordinate the reader did not name
///
/// A reader with no brand looks for `brand: ""` at every step, so steps 3 and
/// 4 coincide for it. That is not a bug and not a shortcut: the global
/// coordinate **is** `(locale-less, region-less, brand-less)`, and a
/// brand-less reader whose tenant default matches nothing simply arrives one
/// step early.
///
/// An empty **tenant default** is different in kind: it skips step 3 rather
/// than keying it on `""` — see [`LocaleRequest::tenant_default_locale`].
///
/// # `None` means the chain ran out
///
/// Which `inst-av-default-locale` exists to prevent, by requiring the global
/// value at publish. This function reports the gap rather than inventing a
/// value, so `dod-default-locale`'s validator is what keeps it unreachable.
#[must_use]
pub fn resolve_localized<'v>(
    request: &LocaleRequest<'_>,
    values: &'v [LocalizedValue],
) -> Option<Resolved<'v>> {
    let at = |locale: &str, region: &str, brand: &str| {
        values
            .iter()
            .find(|v| v.locale == locale && v.region == region && v.brand == brand)
            .map(|v| v.value.as_str())
    };
    let chain = [
        (
            ResolutionStep::Exact,
            at(request.locale, request.region, request.brand),
        ),
        (
            ResolutionStep::LocaleAndBrand,
            at(request.locale, "", request.brand),
        ),
        (
            ResolutionStep::DefaultLocaleAndBrand,
            // Skipped, not keyed on `""`. See the field's own doc: an unset
            // default has no key, and `("", "", brand)` is a coordinate that
            // means something else.
            if request.tenant_default_locale.is_empty() {
                None
            } else {
                at(request.tenant_default_locale, "", request.brand)
            },
        ),
        (ResolutionStep::Global, at("", "", "")),
    ];
    chain
        .into_iter()
        .find_map(|(step, hit)| hit.map(|value| Resolved { value, step }))
}

/// The global coordinate, spelled.
///
/// **P-D-88 arm 2** ships the three columns `NOT NULL` with `""` as the stated
/// absence, so the global coordinate is `("", "", "")` and the `UNIQUE` over
/// the tuple is total. That is the *spelling*; §6's row 8 asks what it
/// **means** — see [`DefaultLocaleRequired`].
pub const GLOBAL_COORDINATE: (&str, &str, &str) = ("", "", "");

/// Whether a stored coordinate is the global one.
#[must_use]
pub fn is_global(value: &LocalizedValue) -> bool {
    (
        value.locale.as_str(),
        value.region.as_str(),
        value.brand.as_str(),
    ) == GLOBAL_COORDINATE
}

/// One localized definition's values, as the publish door presents them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CarriedDefinition {
    /// The definition key, for the refusal to name.
    pub key: String,
    /// Whether the definition takes locale coordinates. A non-localized one
    /// is not judged: it has no locale chain to make total.
    pub localized: bool,
    /// Every coordinate the entity carries for it.
    pub values: Vec<LocalizedValue>,
}

/// What a `-> published` transition presents to the default-locale validator.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PublishedContentSubject {
    /// One entry per definition the entity carries values for.
    pub carried: Vec<CarriedDefinition>,
}

/// Every localized definition an entity carries values for must carry one at
/// the global coordinate (`inst-av-default-locale`).
///
/// @cpt-cf-bss-products-dod-default-locale
///
/// # At publish, never at draft save
///
/// Registered in the `-> published` pipeline and nowhere else, for the reason
/// `rules::PrimaryCategoryRequired`'s own doc gives about its sibling: a rule
/// in the shared pipeline would refuse a draft save that the design admits.
/// A partially-authored draft is legal; an entity reaching `published` with a
/// fallback chain that can run out is not.
///
/// # Per-brand defaults are overrides, and this rule is what makes them safe
///
/// `inst-av-default-locale` calls them *"optional overrides"* and gives the
/// reason: the global value is *"what makes the fallback chain total for
/// **every** brand"*. So a value at `(default-locale, brand A)` satisfies
/// nothing here — a brand-B reader never visits it, which is exactly the
/// matrix case `dod-locale-resolver` requires.
///
/// # §6 row 8, and why this rule is buildable anyway
///
/// Row 8 asks what the global coordinate's key is, and notes that if it means
/// all three coordinates absent then *"a default-locale value at the global
/// coordinate"* names a coordinate carrying no locale. That naming is indeed
/// self-contradictory. The **fork it implies is not live**, though:
/// `inst-av-resolve`'s item-37 note already refuses the other horn in as many
/// words — *"anchoring on [the tenant default locale] would un-total the chain
/// for every already-published entity the moment it changed"* — and P-D-88 arm
/// 2 ships the spelling `("", "", "")`. So this rule demands a value at the
/// shipped global coordinate, and what is left of row 8 is a naming defect
/// rather than a decision. That reading is registered, not asserted: the row
/// stays open and this `DoD` stays unticked.
pub struct DefaultLocaleRequired;

impl DefaultLocaleRequired {
    /// The refusal code (`design/02` §3.3, 422 architectural).
    pub const CODE: &'static str = "DEFAULT_LOCALE_MISSING";
}

impl ValidationRule<PublishedContentSubject> for DefaultLocaleRequired {
    fn name(&self) -> &'static str {
        "inst-av-default-locale"
    }

    fn phase(&self) -> Phase {
        Phase::RegisteredValidators
    }

    fn evaluate(&self, subject: &PublishedContentSubject, report: &mut ValidationReport) {
        for definition in &subject.carried {
            if !definition.localized || definition.values.is_empty() {
                continue;
            }
            if !definition.values.iter().any(is_global) {
                report.violate(
                    Self::CODE,
                    format!("attributes.{}", definition.key),
                    format!(
                        "`{}` carries localized values but none at the global coordinate, so the \
                         fallback chain runs out for at least one brand",
                        definition.key
                    ),
                );
            }
        }
    }
}

/// The category live-value door's precondition mismatch
/// (**P-D-50**, `inst-av-category-branch`).
///
/// Carries the two counters so a caller can re-fetch without a second read.
/// **Not a [`DomainError`]**: `STALE_CATEGORY_TOKEN` has no variant at this
/// commit, for the reason [`CategoryReferenced`] gives.
///
/// It is deliberately **neither** of its two neighbours: `STALE_REVISION` is
/// the Foundation's entity-head code, which a live row cannot be stale
/// against, and `STALE_LIVE_OP` is the envelope's own currency check. This is
/// the door's `If-Match`, a third thing, and `design/02` §3.5 says so while
/// introducing all three.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct StaleCategoryToken {
    /// What the caller sent in `If-Match`.
    pub expected: i64,
    /// What the row carries now.
    pub found: i64,
}

impl StaleCategoryToken {
    /// The refusal code (`design/02` §3.3, **409** — the one code of this
    /// feature's sixteen that §3.3 does not file at 422).
    pub const CODE: &'static str = "STALE_CATEGORY_TOKEN";
}

// -- Frozen version content (`dod-version-content-rendering`, **P-D-29**) --

/// One attribute value as a frozen version renders it.
///
/// The definition and the coordinate travel together because the coordinate
/// alone does not identify a row: `("", "", "")` is the global coordinate of
/// **every** definition the entity carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrozenAttributeValue {
    /// Which definition the value answers to.
    pub definition_id: Uuid,
    /// The coordinate and the value.
    pub coordinate: LocalizedValue,
}

/// Render the category-assignment set as a frozen version carries it.
///
/// # The sort key is the whole assignment, and that is more than the `DoD`
/// says
///
/// `dod-version-content-rendering` asks for *"JSON arrays sorted by the
/// collection's own identifier"*. For assignments the category id **is** an
/// identifier — `uq_products_product_category` admits one row per
/// `(product, category)` — so sorting by it is total and this collection
/// needs no amendment. [`value_collection`] is the one that does.
///
/// # The element ordering is the Foundation's and is not repeated here
///
/// `canonical::render_into` sorts every object's keys and preserves every
/// array's order, recursively. So this function owes the **array** order and
/// nothing else: hand the result to
/// [`crate::domain::canonical::canonical_rendering`] and the elements come
/// out field-ordered by the one rule the gear has, with no second
/// serialization rule minted here — which is what
/// `canonical`'s module doc exists to prevent.
#[must_use]
pub fn assignment_collection(assignments: &[(Uuid, AssignmentRole)]) -> serde_json::Value {
    let mut rows: Vec<&(Uuid, AssignmentRole)> = assignments.iter().collect();
    rows.sort_unstable_by_key(|row| row.0);
    serde_json::Value::Array(
        rows.into_iter()
            .map(|(category_id, role)| {
                serde_json::json!({
                    "categoryId": category_id.to_string(),
                    "role": role.as_str(),
                })
            })
            .collect(),
    )
}

/// Render the attribute-value set as a frozen version carries it.
///
/// # The sort key is the whole coordinate, and §7 row 9 is why
///
/// The `DoD` and **P-D-29** both say *"sorted by the collection's own
/// identifier"*. For this collection that identifier is the **definition
/// id**, and row 9 measured what follows: *"Sorting by the attribute id
/// orders groups, not rows, so two engines can serialize one content two ways
/// — the failure the rule exists to prevent."* One definition carries as many
/// rows as it has coordinates, so an identifier sort leaves their relative
/// order to whatever the driver returned.
///
/// **So the sort here is the full coordinate** — definition, then locale,
/// then region, then brand — which is the table's own primary key and is
/// therefore total by construction. That **exceeds the letter of the `DoD`'s
/// first sentence** and is exactly the amendment row 9 says is owed to
/// P-D-29's owner. It is taken rather than deferred because the `DoD`'s
/// *second* sentence requires a golden vector proving the rendering
/// byte-identical across both engines, and an identifier sort cannot satisfy
/// it: the two sentences contradict each other, and only one of them can be
/// built. Registered, and the `DoD` stays unticked.
#[must_use]
pub fn value_collection(values: &[FrozenAttributeValue]) -> serde_json::Value {
    let mut rows: Vec<&FrozenAttributeValue> = values.iter().collect();
    rows.sort_unstable_by(|left, right| {
        (
            left.definition_id,
            &left.coordinate.locale,
            &left.coordinate.region,
            &left.coordinate.brand,
        )
            .cmp(&(
                right.definition_id,
                &right.coordinate.locale,
                &right.coordinate.region,
                &right.coordinate.brand,
            ))
    });
    serde_json::Value::Array(
        rows.into_iter()
            .map(|row| {
                serde_json::json!({
                    "definitionId": row.definition_id.to_string(),
                    "locale": row.coordinate.locale,
                    "region": row.coordinate.region,
                    "brand": row.coordinate.brand,
                    "value": row.coordinate.value,
                })
            })
            .collect(),
    )
}

/// The seven content rules, registered — the **one** list both save doors run.
///
/// # Why the list lives here and not at each door
///
/// `domain::rules`' own doc gives the rule this follows: *"a feature ships its
/// validators with its handler, and there is no runtime registry to fall out
/// of step with the handler set."* The concern that sentence names is a
/// **runtime** registry, and this is not one: it is compile-time code in the
/// feature's own module, beside the seven rule types it registers, and a rule
/// added without a line here is a rule with no caller — which is exactly the
/// state `A-OWED-08` found sixteen of this feature's own in.
///
/// The alternative was a builder per door. Two doors need the identical seven,
/// and the SKU door's list would have been the one to go stale: it is the door
/// that runs **no** pipeline today, so nothing there would have reddened when
/// an eighth rule landed on the Product side alone. One list cannot drift from
/// itself.
///
/// **The counter-argument, stated:** registration is now one call away from
/// the handler rather than in it, so a reader at the door sees
/// `content_save_pipeline()` and not the seven names. That is the cost, and it
/// is paid to remove a drift a second list would have made silent.
///
/// Every rule is [`Phase::RegisteredValidators`] — **P-D-97** arm 2's first
/// form, a registered rule whose operands are facts the door prefetches, the
/// shipped `PrimaryCategoryRequired` + `has_primary_category` pattern. One
/// phase means one rejection carrying every content violation, so an operator
/// fixing four fields makes one round trip.
#[must_use]
pub fn content_save_pipeline() -> ValidationPipeline<ContentSaveSubject> {
    ValidationPipeline::new()
        .with_rule(Box::new(CategoryResolvableRule))
        .with_rule(Box::new(CategoryNotRetiredRule))
        .with_rule(Box::new(CategoryRoleConflictRule))
        .with_rule(Box::new(AttributeDefinitionKnownRule))
        .with_rule(Box::new(AttributeDefinitionActiveRule))
        .with_rule(Box::new(AttributeValueTypeRule))
        .with_rule(Box::new(AttributeScopeRule))
}

/// The `-> published` content pipeline: the global-coordinate demand.
///
/// Separate from [`content_save_pipeline`] and from
/// `products::published_transition_pipeline`, because its subject is neither
/// of theirs — `inst-av-default-locale` judges the entity's **stored** values,
/// which a save's payload does not carry and
/// `rules::PublishedTransitionSubject` has no field for.
///
/// At publish and never at draft save, for the reason
/// `rules::PrimaryCategoryRequired`'s doc gives about its sibling: a
/// partially-authored draft is legal.
#[must_use]
pub fn published_content_pipeline() -> ValidationPipeline<PublishedContentSubject> {
    ValidationPipeline::new().with_rule(Box::new(DefaultLocaleRequired))
}

/// The sixteen codes `design/02` §3.3 declares, as one roster.
///
/// # Why the roster exists rather than only the constants
///
/// `dod-taxonomy-errors` requires *"all sixteen"* be declared and registered.
/// A constant on each raising rule is the declaration; nothing on its own is
/// the **census**, and a code that is declared nowhere is invisible to every
/// per-rule test. `the_sixteen_codes_are_all_reachable` reads this array
/// against the constants and against `DomainError::code`, so a code named
/// here with no raiser and a raiser with no entry here both redden.
///
/// The array is the design's list verbatim and in its order; it is **not** a
/// claim that all sixteen are raiseable at this commit, which they are not.
pub const TAXONOMY_ERROR_CODES: [&str; 16] = [
    "DUPLICATE_CATEGORY_NAME",
    "TAXONOMY_CYCLE",
    "TAXONOMY_LIMIT",
    "CATEGORY_REFERENCED",
    "CATEGORY_RETIRED",
    "ATTRIBUTE_DEFINITION_UNKNOWN",
    "ATTRIBUTE_DEFINITION_DEPRECATED",
    "DEFINITION_IN_USE",
    "ATTRIBUTE_TYPE_MISMATCH",
    "ATTRIBUTE_SCOPE_VIOLATION",
    "DEFAULT_LOCALE_MISSING",
    "PRIMARY_CATEGORY_REQUIRED",
    "STALE_CATEGORY_TOKEN",
    "CONTENT_PII_BLOCKED",
    "METADATA_LIMIT",
    "STALE_LIVE_OP",
];

#[cfg(test)]
#[path = "taxonomy_tests.rs"]
mod taxonomy_tests;
