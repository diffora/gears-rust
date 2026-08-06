//! `TaxonomyValidator` — the three `inst-tx-*` rules of
//! `design/04-currency-tax.md` §3, plus the vocabulary the four tenant
//! taxonomies are addressed by.
//!
//! §1.7 names one entity for all of it: *"Registered rules: `region` membership
//! on price rows; overlay **scope-value** membership per class — `brand`,
//! `region`, `partner`, `orgTier` against their tenant taxonomies (D-120; rule
//! shared with Slice 9, which owns the `customerGroup` analogue)"*. The second
//! half of that sentence is **already built**, in
//! [`overlay_rules::check_scope`](crate::domain::overlay_rules), because Slice 9
//! could not validate an overlay scope without it and the four tables were
//! carried onto its chain to make that possible. What was missing is everything
//! else: the price-row half, the retire guard, and any surface at all through
//! which a value reaches one of the four tables.
//!
//! # Why this module does not re-declare a scope class
//!
//! [`ScopeClass`] already enumerates the six overlay scope classes, ranks them
//! for `inst-plv-class-tiebreak`, and maps the four taxonomy-backed ones to their
//! physical table. A second enumeration here would be a second answer to *which
//! table declares this class's values*, and the two would drift the day a class
//! is added. [`TaxonomyClass`] is therefore a **narrowing** of it — the four
//! classes §5's route can address — and it converts into `ScopeClass` rather than
//! restating its table map.
//!
//! The narrowing is the route's, not an opinion: `global` has no value universe
//! at all, and `customerGroup`'s universe is `pricing_customer_group_taxonomy`,
//! which belongs to Slice 9's membership half and does not exist (D-223 refuses
//! that class outright for exactly this reason). A `PUT` addressing either would
//! be a write to a table this gear does not have.
//!
//! # The refusal codes, and the one this module deliberately does not raise
//!
//! §5 declares `REGION_UNKNOWN`, `BRAND_UNKNOWN`, `PARTNER_UNKNOWN`,
//! `ORG_TIER_UNKNOWN` and `TAXONOMY_VALUE_IN_USE`. Only two of them are minted
//! here.
//!
//! [`REGION_UNKNOWN`] is the **price-row** refusal and is genuinely this
//! module's: a row's `region` is a scope-key axis, §2's flow declares the code
//! against that flow, and the remedy is to fix the row. [`TAXONOMY_VALUE_IN_USE`]
//! is the retire guard's and has one owner by construction.
//!
//! The other three are **not raised anywhere in this crate**, and that is a
//! recorded finding rather than an omission. They name the overlay scope-value
//! failure, which `overlay_rules` already reports as `SCOPE_VALUE_UNKNOWN` —
//! declared for the same refusal by D-222 three days before this slice was built,
//! in `design/09-price-overlays.md` §5. Two codes for one rejection is a decision
//! the design set owes; raising both would make a consumer match on whichever
//! surface happened to answer, and raising the trio *instead* would re-spell a
//! code Slice 9 shipped. See `T-1` in the owed register. Note also that the trio
//! is three codes for **four** classes: a region-scoped overlay value, which
//! D-120 explicitly brought under the same rule, has no code in either list.

use std::collections::BTreeSet;
use std::fmt;

use toolkit_macros::domain_model;

use crate::domain::overlay::{ScopeClass, ScopeValue};
use crate::domain::plan_shape::PlanShape;
use crate::domain::scope_key::Region;
use crate::domain::validation::{ValidationReport, ValidationRule};

// ---------------------------------------------------------------------------
// The codes.
// ---------------------------------------------------------------------------

/// A price row naming a `region` the tenant's region taxonomy does not declare
/// as `active` (§2, §5, `inst-tx-region` / `inst-mc-region`).
///
/// The **price-row** refusal specifically. An overlay whose `region`-class scope
/// value is undeclared is `overlay_rules::SCOPE_VALUE_UNKNOWN`'s, not this — see
/// the module doc and register entry `T-1`.
pub const REGION_UNKNOWN: &str = "REGION_UNKNOWN";

/// A taxonomy value that cannot retire because something active still names it
/// (§5, 409; `inst-tx-mutation`).
pub const TAXONOMY_VALUE_IN_USE: &str = "TAXONOMY_VALUE_IN_USE";

// ---------------------------------------------------------------------------
// The classes.
// ---------------------------------------------------------------------------

/// The four taxonomy classes `GET/PUT /config/taxonomies/{…}` addresses.
///
/// Ordered as §5 and §6 list them. The order carries no ranking — the ranking is
/// [`ScopeClass`]'s derived `Ord` and there is exactly one of those.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TaxonomyClass {
    /// Pricing regions: the price-row axis **and** an overlay scope class.
    Region,
    /// Commercial brands: an overlay scope class only — §3 step 2 is explicit
    /// that `brand` is *"**not** a price-row field (Foundation §4.1)"*.
    Brand,
    /// Resellers / partners (D-120).
    Partner,
    /// Organisation tiers (D-120).
    OrgTier,
}

impl TaxonomyClass {
    /// Every addressable class, in §5's order.
    pub const ALL: &'static [Self] = &[Self::Region, Self::Brand, Self::Partner, Self::OrgTier];

    /// The path segment §5 spells for this class.
    ///
    /// **`orgTier` is camelCase and every other wire token for this class is
    /// `org_tier`** ([`ScopeClass::as_str`], and the `scope_class` column Slice 9
    /// stores). §5 is the normative statement of the route and a path segment is
    /// not a JSON field, so the two are not required to agree by any rule the set
    /// states — but an operator meets both in one sitting, so the divergence is
    /// recorded as `T-4` rather than left to be "fixed" later in the direction
    /// that breaks the route.
    #[must_use]
    pub const fn path_segment(self) -> &'static str {
        match self {
            Self::Region => "region",
            Self::Brand => "brand",
            Self::Partner => "partner",
            Self::OrgTier => "orgTier",
        }
    }

    /// Parse a path segment back into a class.
    #[must_use]
    pub fn parse_segment(segment: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|class| class.path_segment() == segment)
    }

    /// The overlay scope class this taxonomy declares the values of.
    ///
    /// The conversion is the point: it is what keeps one table map in the crate.
    #[must_use]
    pub const fn scope_class(self) -> ScopeClass {
        match self {
            Self::Region => ScopeClass::Region,
            Self::Brand => ScopeClass::Brand,
            Self::Partner => ScopeClass::Partner,
            Self::OrgTier => ScopeClass::OrgTier,
        }
    }

    /// The physical table, by way of [`ScopeClass::taxonomy_table`].
    ///
    /// Every one of the four classes has one, which is what makes the
    /// `expect`-free `unwrap_or` below unreachable in practice; it is written as
    /// a total function rather than an `Option` because *this* enum's whole
    /// domain is the classes that have a table.
    #[must_use]
    pub const fn table(self) -> &'static str {
        match self.scope_class().taxonomy_table() {
            Some(table) => table,
            // Unreachable: all four narrow classes map to a declared table. A
            // panic here would be a claim about `ScopeClass` that this enum
            // exists to guarantee.
            None => "a taxonomy this class does not declare",
        }
    }

    /// Does this class carry D-01's two `tax_*` markers?
    ///
    /// §6: *"the `tax_*` columns below are region-only"*, and
    /// `sqlite_taxonomy_store::the_other_three_taxonomies_carry_no_tax_columns`
    /// asserts that absence against the schema.
    #[must_use]
    pub const fn carries_tax_markers(self) -> bool {
        matches!(self, Self::Region)
    }
}

impl fmt::Display for TaxonomyClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.path_segment())
    }
}

// ---------------------------------------------------------------------------
// The entries.
// ---------------------------------------------------------------------------

/// `active | retired` — the whole state machine §6 gives these tables.
#[domain_model]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TaxonomyState {
    /// Declared and usable. The only state that validates anything.
    #[default]
    Active,
    /// Withdrawn from new use. Existing references survive — retirement is
    /// **guarded**, never cascading — but a retired value declares nothing, which
    /// is `overlay_repo::declares`' `state = 'active'` predicate.
    Retired,
}

impl TaxonomyState {
    /// Both states.
    pub const ALL: &'static [Self] = &[Self::Active, Self::Retired];

    /// The stored / wire token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Retired => "retired",
        }
    }

    /// Parse a stored token.
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|s| s.as_str() == token)
    }
}

impl fmt::Display for TaxonomyState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// D-01's two tenant-declared markers, which live on the region taxonomy alone.
///
/// The MVP `RegionTaxReadiness` source. Absence of the whole struct is an
/// **undeclared region**, which C4 fails closed on; absence of
/// [`RegionTaxMarkers::tax_category`] is a declared region with no default
/// category, which is a different fact and one `inst-td-policy`'s coalesce is
/// about.
#[domain_model]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RegionTaxMarkers {
    /// The region's default tax category. `None` is undeclared, not empty.
    pub tax_category: Option<String>,
    /// Tenant-declared *"a tax rate is configured for this region"*. Defaults to
    /// **false** — the fail-closed reading: a region nobody has declared a rate
    /// for is a region with no rate, not one with an unknown rate.
    pub tax_rate_present: bool,
}

/// One declared value of one taxonomy.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaxonomyEntry {
    /// The declared code. Never blank — [`ScopeValue`] refuses one, and the
    /// store spends a `CHECK` on the same rule because the empty string is
    /// `pricing_price_overlay`'s sentinel for the classless scope.
    pub value: ScopeValue,
    /// The operator's label.
    pub display_name: String,
    /// Where the value stands.
    pub state: TaxonomyState,
    /// D-01's markers, on the region taxonomy only. `None` on the other three,
    /// where the columns do not exist.
    pub tax: Option<RegionTaxMarkers>,
}

// ---------------------------------------------------------------------------
// `inst-tx-mutation` — the retire guard.
// ---------------------------------------------------------------------------

/// What still names a value the operator is retiring.
///
/// **Two counts and not one**, because §3 step 3 enumerates two referencing
/// shapes with different remedies: *"referenced by an active published price row
/// (`region`) **or** an active `PriceOverlay` scope of any taxonomy-backed
/// class"*. An operator told only that *something* references the value cannot
/// act; told which plane it is on, they can.
///
/// Resolved by the caller, never here — the same purity contract every rule in
/// this crate holds to.
#[domain_model]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ValueReferences {
    /// Published, non-superseded price rows carrying this value as their
    /// `region` axis. Always zero for the three non-`region` classes, whose value
    /// is not a price-row field at all (§3 step 2).
    pub published_price_rows: u64,
    /// Published `PriceOverlay` revisions scoped to this class and value.
    pub active_overlay_scopes: u64,
}

impl ValueReferences {
    /// Does anything name it?
    #[must_use]
    pub const fn any(self) -> bool {
        self.published_price_rows > 0 || self.active_overlay_scopes > 0
    }
}

/// `inst-tx-mutation`: refuse a retirement while the value is referenced.
///
/// D-120 widened this guard and the widening is the whole of why it is written
/// over a class rather than over `region`: *"the guard previously enumerated
/// price rows and brand overlays only, so a region value retired cleanly while
/// region-scoped overlays still named it"*. Every one of the four classes is
/// checked on the overlay plane; only `region` is checked on the row plane,
/// because only `region` is an axis of a price row.
///
/// Answers a [`ValidationReport`] rather than a `Result` so the refusal carries
/// §5's code and a message naming **both** planes' counts — an operator who
/// retires a value referenced by nine overlays and one row has two jobs, and a
/// refusal naming one of them sends them back a second time.
#[must_use]
pub fn check_retirable(
    class: TaxonomyClass,
    value: &ScopeValue,
    references: ValueReferences,
) -> ValidationReport {
    let mut report = ValidationReport::default();
    if !references.any() {
        return report;
    }
    report.violate(
        TAXONOMY_VALUE_IN_USE,
        value.as_str(),
        format!(
            "{class} value `{value}` cannot retire: {} published price row(s) carry it on their \
             region axis and {} published overlay scope(s) select on it; retirement is guarded \
             rather than cascading, so end or retarget every reference first (D-120)",
            references.published_price_rows, references.active_overlay_scopes
        ),
    );
    report
}

// ---------------------------------------------------------------------------
// `inst-tx-region` / `inst-mc-region` — the price-row membership rule.
// ---------------------------------------------------------------------------

/// Every candidate row's `region` is declared `active` in the tenant's region
/// taxonomy (§3 step 1, §2 step 2).
///
/// # Why the declared set is a field and not a lookup
///
/// The rule set runs **twice** on one publish — as a pre-check and again inside
/// the commit transaction (Foundation §4.2) — so a rule that fetched its own
/// inputs would be free to answer differently in the two runs for no authored
/// reason. This is `RoundingPolicyResolved`'s arrangement and `ReferencingMarket`'s,
/// and the set is resolved once in `infra::publish::rule_params`.
///
/// # `active`, and why an empty set is not a special case
///
/// The set the caller resolves carries **active** values only, which is
/// `overlay_repo::declares`' predicate one plane over: a value that reached
/// `retired` anyway must not validate a new row against itself. A tenant that has
/// declared no region at all therefore fails every row, and that is C2's
/// fail-closed reading rather than an edge case — *"membership is validated at
/// save/publish (unknown value fails before publish)"* does not carve out the
/// tenant who configured nothing.
#[derive(Debug, Clone)]
pub struct RegionsDeclared {
    /// The tenant's `active` region values, resolved by the caller.
    pub declared: BTreeSet<Region>,
}

impl ValidationRule<PlanShape> for RegionsDeclared {
    fn name(&self) -> &'static str {
        "inst-tx-region"
    }

    fn evaluate(&self, subject: &PlanShape, report: &mut ValidationReport) {
        let mut reported: BTreeSet<&Region> = BTreeSet::new();
        for record in &subject.rows {
            let region = record.scope_key.region();
            if self.declared.contains(region) {
                continue;
            }
            // One violation per undeclared **value**, not per row. A plan with
            // forty rows in one bad region is one authoring mistake, and forty
            // copies of it is a report an operator cannot read — and C1's
            // 20-currency floor makes forty a real number rather than a
            // hypothetical.
            if !reported.insert(region) {
                continue;
            }
            report.violate(
                REGION_UNKNOWN,
                region.as_str(),
                format!(
                    "region `{region}` is not an active value of this tenant's region taxonomy; a \
                     price row's region is validated at save and at publish, and an unknown value \
                     fails before publish (C2) — declare it at PUT \
                     /bss-pricing/v1/config/taxonomies/region first"
                ),
            );
        }
    }
}

#[cfg(test)]
#[path = "taxonomy_tests.rs"]
mod taxonomy_tests;
