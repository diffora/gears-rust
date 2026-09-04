//! Scope containment (`region_scope` / `brand_scope`) between a parent and a
//! child, and the inheritance a child's payload may ask for instead of naming
//! its own set.
//!
//! Containment is defined **over restrictions**, not over raw set membership
//! (P-D-39, `cpt-cf-bss-products-dod-containment`):
//!
//! 1. an **unrestricted parent contains every child**;
//! 2. an **unrestricted child is contained only by an unrestricted parent** —
//!    the clause that reads backwards: a child that reaches everywhere cannot
//!    be contained by a parent that does not;
//! 3. between two **non-empty** sets it is ordinary subset.
//!
//! # Two types, not one
//!
//! A SKU's payload carries **three** possible states per dimension — the
//! field was omitted, the field was sent as an explicit empty set (meaning
//! "no restriction"), or the field names a non-empty set — while a value that
//! has already been resolved (or a value read back off a stored row) carries
//! only **two**: unrestricted, or a non-empty set. Collapsing "omitted" into
//! "explicitly empty" would silently turn every inheriting child into an
//! unrestricted one, which clause 2 then refuses against any restricted
//! parent — a create that should have succeeded, refused instead. Collapsing
//! the other way lets a genuinely unrestricted child inherit a restriction it
//! never asked for.
//!
//! [`ScopeInput`] carries the three payload states and cannot be compared for
//! containment directly — it has to be resolved first. [`ResolvedScope`]
//! carries the two states containment is actually decided over, and is the
//! type that crosses the storage boundary. Keeping them as two types rather
//! than one enum with an "unreachable" third state means a caller cannot
//! accidentally hand an unresolved `Omitted` scope to the containment check —
//! the compiler refuses the call.
//!
//! # The stored form cannot carry this distinction
//!
//! `products_product.region_scope` and `products_sku.region_scope` (and their
//! `brand_scope` siblings) are `NOT NULL`, default the empty string, and the
//! empty string means unrestricted. That column can express exactly the two
//! [`ResolvedScope`] states — never "omitted", because by the time a row is
//! written the inheritance in this module has already run. Resolution happens
//! **here, before persistence**; the column only ever holds the resolved
//! value.
//!
//! The design set (`design/01-foundation.md` §4.1/§4.2) calls the column "a
//! flat value set" without pinning a separator. This module renders and
//! parses it as a comma-separated list (see [`SCOPE_VALUE_SEPARATOR`]),
//! chosen because a flat list of short tokens (region or brand identifiers)
//! is the conventional reading of "flat value set" and commas are not a
//! character either kind of identifier is expected to contain; a later slice
//! that needs a different separator should change it in this one place.

//! @cpt-dod:cpt-cf-bss-products-dod-scope-containment-final:p1

use std::collections::BTreeSet;

/// The character `region_scope` and `brand_scope` use to join a
/// [`ResolvedScope::Restricted`] set into the column's stored string, and to
/// split it back out again.
///
/// Not fixed by the design set — see the module doc for why a comma was
/// chosen here rather than invented independently at each call site.
pub const SCOPE_VALUE_SEPARATOR: char = ',';

/// Which of a SKU's two scope dimensions a containment check was run over.
///
/// Both dimensions are decided by the identical rule (see [`contains`]); this
/// type is what lets a caller apply that one rule twice and still say, in a
/// `SCOPE_NOT_CONTAINED` message, which of the two failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeDimension {
    /// `region_scope`.
    Region,
    /// `brand_scope`.
    Brand,
}

impl ScopeDimension {
    /// The column name this dimension corresponds to, for a rejection
    /// message.
    #[must_use]
    pub const fn column_name(self) -> &'static str {
        match self {
            Self::Region => "region_scope",
            Self::Brand => "brand_scope",
        }
    }
}

/// [`ResolvedScope::parse`] read a value containing an empty token — `","`,
/// `"eu,,us"` or `",eu"` — a malformed value the
/// [`SCOPE_VALUE_SEPARATOR`]-joined column format never needs to express: an
/// unrestricted value is written as the empty string itself, never as a set
/// containing an empty string. See [`ResolvedScope::parse`]'s own doc for why
/// this is a rejection rather than a silent filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmptyScopeToken;

/// A scope set once inheritance has already been decided — the only shape the
/// stored column, and the containment rule itself, ever operate on.
///
/// See the module doc for why this is a distinct type from [`ScopeInput`]
/// rather than the same enum with a third, invalid-here variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedScope {
    /// No restriction: contains, and (per clause 2) is contained by, nothing
    /// less than another unrestricted scope.
    Unrestricted,
    /// A non-empty named set. Never empty — an empty set is
    /// [`ResolvedScope::Unrestricted`] by construction; see [`Self::parse`].
    Restricted(BTreeSet<String>),
}

impl ResolvedScope {
    /// Parse a stored column value: the empty string is unrestricted,
    /// anything else is split on [`SCOPE_VALUE_SEPARATOR`] into a restricted
    /// set.
    ///
    /// # Errors
    ///
    /// [`EmptyScopeToken`] when the value contains an empty token —
    /// `","`, `"eu,,us"` or `",eu"` all split into a set with an empty-string
    /// member, which is not a value any caller meant to name (the caller
    /// asking for "no restriction" sends the empty string itself, the case
    /// already handled above, never a non-empty string that merely contains
    /// one). This is a fail-closed choice over the lenient alternative of
    /// silently dropping the empty token and keeping the rest: filtering
    /// would rewrite the caller's input without telling them, while
    /// rejecting costs a refusal the caller must understand but never lets a
    /// malformed value reach the very column P-D-39 defines meaning over.
    pub fn parse(stored: &str) -> Result<Self, EmptyScopeToken> {
        if stored.is_empty() {
            Ok(Self::Unrestricted)
        } else {
            let mut values = BTreeSet::new();
            for token in stored.split(SCOPE_VALUE_SEPARATOR) {
                if token.is_empty() {
                    return Err(EmptyScopeToken);
                }
                values.insert(token.to_owned());
            }
            Ok(Self::Restricted(values))
        }
    }

    /// Render this scope the way the stored column expects it: the empty
    /// string for [`Self::Unrestricted`], a [`SCOPE_VALUE_SEPARATOR`]-joined
    /// list otherwise.
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::Unrestricted => String::new(),
            Self::Restricted(values) => {
                let mut rendered = String::new();
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        rendered.push(SCOPE_VALUE_SEPARATOR);
                    }
                    rendered.push_str(value);
                }
                rendered
            }
        }
    }
}

/// A SKU payload's scope value for one dimension, before inheritance runs.
///
/// The three states a payload can actually express — see the module doc for
/// why "omitted" and "explicitly unrestricted" must not collapse into one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeInput {
    /// The payload named no value for this dimension: the child takes the
    /// parent's resolved scope verbatim.
    Omitted,
    /// The payload explicitly named an empty set: the child is unrestricted,
    /// regardless of what the parent is.
    Unrestricted,
    /// The payload named a non-empty set: the child's resolved scope is
    /// exactly this set, subject to clause 3's containment check.
    Restricted(BTreeSet<String>),
}

impl ScopeInput {
    /// Resolve this payload state against the parent's already-resolved
    /// scope for the same dimension, producing the child's resolved scope.
    ///
    /// This is the one inheritance rule, applied identically to
    /// `region_scope` and `brand_scope` by calling it once per dimension
    /// rather than duplicating it.
    #[must_use]
    pub fn resolve(self, parent: &ResolvedScope) -> ResolvedScope {
        match self {
            Self::Omitted => parent.clone(),
            Self::Unrestricted => ResolvedScope::Unrestricted,
            Self::Restricted(values) => ResolvedScope::Restricted(values),
        }
    }
}

/// The containment verdict for a single dimension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeContainment {
    /// The child's resolved scope is contained in the parent's.
    Contained,
    /// The child's resolved scope is not contained in the parent's for the
    /// named dimension. Both resolved scopes are carried so a door can
    /// compose the `SCOPE_NOT_CONTAINED` message without re-deriving which
    /// values escaped or re-running the check.
    NotContained {
        /// Which of the two dimensions failed.
        dimension: ScopeDimension,
        /// The parent's resolved scope for this dimension.
        parent: ResolvedScope,
        /// The child's resolved scope for this dimension.
        child: ResolvedScope,
    },
}

/// The containment rule (P-D-39), written once and applied to whichever
/// dimension the caller names.
///
/// - an unrestricted parent contains every child (clause 1);
/// - an unrestricted child is contained only by an unrestricted parent
///   (clause 2 — a restricted parent refuses an unrestricted child even
///   though every non-empty set is, set-theoretically, a subset of nothing
///   the raw-set reading would allow);
/// - between two non-empty sets, containment is ordinary subset (clause 3).
#[must_use]
pub fn contains(
    dimension: ScopeDimension,
    parent: &ResolvedScope,
    child: &ResolvedScope,
) -> ScopeContainment {
    let is_contained = match (parent, child) {
        (ResolvedScope::Unrestricted, _) => true,
        (ResolvedScope::Restricted(_), ResolvedScope::Unrestricted) => false,
        (ResolvedScope::Restricted(parent_values), ResolvedScope::Restricted(child_values)) => {
            child_values.is_subset(parent_values)
        }
    };
    if is_contained {
        ScopeContainment::Contained
    } else {
        ScopeContainment::NotContained {
            dimension,
            parent: parent.clone(),
            child: child.clone(),
        }
    }
}

/// The resolved pair a Product or SKU carries: `region_scope` and
/// `brand_scope` together.
///
/// A SKU's two dimensions are decided independently — a child may be
/// contained on one and refused on the other — which is why containment and
/// inheritance both operate per-dimension via [`contains`] and
/// [`ScopeInput::resolve`] rather than on this pair as a single unit. This
/// type exists only to carry "both, already resolved" through the door that
/// will call it; it adds no rule of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopePair {
    /// The resolved `region_scope`.
    pub region: ResolvedScope,
    /// The resolved `brand_scope`.
    pub brand: ResolvedScope,
}

impl ScopePair {
    /// Resolve a child's two payload inputs against this (the parent's)
    /// already-resolved pair.
    #[must_use]
    pub fn resolve_child(&self, region: ScopeInput, brand: ScopeInput) -> Self {
        Self {
            region: region.resolve(&self.region),
            brand: brand.resolve(&self.brand),
        }
    }

    /// Check the given child's resolved pair against this (the parent's)
    /// pair on both dimensions, refusing on the first dimension that fails.
    ///
    /// # Errors
    ///
    /// Returns the [`ScopeContainment::NotContained`] verdict for the first
    /// dimension (region, then brand) whose child scope is not contained in
    /// this parent's scope for that dimension.
    pub fn check_containment(&self, child: &Self) -> Result<(), ScopeContainment> {
        let checks = [
            (ScopeDimension::Region, &self.region, &child.region),
            (ScopeDimension::Brand, &self.brand, &child.brand),
        ];
        for (dimension, parent_scope, child_scope) in checks {
            match contains(dimension, parent_scope, child_scope) {
                ScopeContainment::Contained => {}
                failure @ ScopeContainment::NotContained { .. } => return Err(failure),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "containment_tests.rs"]
mod containment_tests;
