//! The `DispositionTable` — `design/11-clone.md` §3.1's per-field
//! copy/reset/re-validate matrix, carried as this named seam rather than as
//! prose (`dod-clone-seams`; DECOMPOSITION §2.11 says *"Neither is an
//! aggregate of its own"*, so this is a module, not a struct).
//!
//! # What lives here, and what stays at the doors
//!
//! The matrix's **data half**: the source-field structs the clone doors read
//! out of wherever the state says (a `draft` at its head, everything else at
//! the last frozen version — `dod-clone-read-surface`), the suggestion
//! strings of `inst-cn-identity`/`inst-cn-rename`, and the walk's
//! operational cap. The **transactional half** — the first-free walk itself,
//! arbitrated by the reservation index under the insert (**P-D-62**) — stays
//! at the doors, because the index is the arbiter and the insert is the
//! probe; a suggestion computed here is only a candidate until the store
//! admits it.
//!
//! The re-validate rows are registered over **almost nothing at this
//! commit**. Four of their five field classes still have no shipped store;
//! the exception is the meter pair (`metering_unit`/`usage_type_ref`,
//! landed with 03), and it is one the clone cannot yet exercise: the clone
//! materializes through the ordinary create door, the create door writes no
//! content (01 §6 open item 11), and [`SkuCloneSource`] carries no meter
//! fields — so **a cloned SKU starts with no declaration**, whatever its
//! source declared, and the re-validate row for the pair stays unregistered
//! until the clone can carry it. The matrix's executable content today is
//! the copy/reset half these structs carry and the identity/parent rows the
//! doors enforce through the ordinary create path.
//!
//! # The suggestion rules (P-D-62)
//!
//! `{name}-copy-N` and `{code}-copy-N`, `N` the first free integer for the
//! suggested string, decided by the index under the reservation; a `retired`
//! Product source flavors the **name** `-revived`, a second revival of one
//! lineage `-revived-N` by the same first-free rule. The flavor is the
//! rename rule's and the rename rule is Product-only (`products_sku`
//! carries no name), so [`suggested_sku_code`] has no flavored arm.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-clone-seams:p1
//! @cpt-dod:cpt-cf-bss-products-dod-clone-identity:p1

use uuid::Uuid;

/// The Product clone's source fields, read from wherever `inst-cn-door`
/// says the source's state is read: a `draft` at its head, everything else
/// at its **last frozen version** — never a head's pending edits.
pub struct ProductCloneSource {
    /// Copies verbatim: a clone never retargets brand (§3.1 — a cross-brand
    /// copy is a create, not a clone).
    pub brand_id: Uuid,
    /// The rename rule's base (`inst-cn-rename`).
    pub name: String,
    /// The code suggestion's base; a source with none suggests none and the
    /// clone's stays null (`inst-cn-identity`, L5).
    pub product_code: Option<String>,
    /// Copies verbatim.
    pub region_scope: String,
    /// Copies verbatim.
    pub brand_scope: String,
    /// `None` = read at the head (a draft source) — P-D-76's representable
    /// sentinel; `Some(v)` = read at frozen version `v`.
    pub read_at_version: Option<i64>,
    /// Whether the source is `retired`, which flavors the name suggestion
    /// `-revived` rather than `-copy-N` (`inst-cn-rename`).
    pub retired: bool,
    /// The content the copy rows carry over (`design/11` §3.1's copy rows;
    /// P-D-154): the assignment set and the attribute-value set as the source
    /// was read — frozen for a published source, live for a draft — and the
    /// metadata map from the beside-entity store in both cases (P-D-06).
    pub content: CloneContent,
}

/// The SKU clone's source fields, on [`ProductCloneSource`]'s read rule.
/// No name (the rename rule is Product-only) and no brand column —
/// `products_sku` carries scope strings instead.
pub struct SkuCloneSource {
    /// 03's classification (P-D-145): content, so the clone carries it —
    /// the type profile, the flag, the tier and the two Finance codes.
    pub sku_type: Option<String>,
    pub sellable: bool,
    pub plan_tier: Option<String>,
    pub tax_category_ref: Option<String>,
    pub gl_code_ref: Option<String>,
    /// The source's own parent. A lone-SKU clone **copies** this link
    /// unless the caller overrides it (§3.1's carve-out: the create door
    /// then refuses a terminal parent, so a lone clone of a retired
    /// parent's SKU must name a new one); a family clone **remaps** it to
    /// the new parent.
    pub product_id: Uuid,
    /// The code suggestion's base. Never null: `sku_code` is NOT NULL.
    pub sku_code: String,
    /// Copies verbatim (re-checked by the ordinary containment validator).
    pub region_scope: String,
    /// Copies verbatim (re-checked likewise).
    pub brand_scope: String,
    /// `None` = read at the head; `Some(v)` = read at frozen version `v`.
    pub read_at_version: Option<i64>,
    /// The meter pair (`design/11` §3.1 *Metering declaration*: **Copy +
    /// re-validate**, P-D-154) — copied as the source was read; the unit is
    /// re-judged against the live recognized set at clone, `usageTypeRef`'s
    /// re-resolution stays `03`'s at publish.
    pub metering_unit: Option<String>,
    /// The other half of the pair.
    pub usage_type_ref: Option<String>,
    /// The attribute values and the metadata map (P-D-154).
    pub content: CloneContent,
}

/// What a clone copies beyond the head row: `02`'s two collections and the
/// metadata map (`design/11` §3.1 rows *Display/localized attributes +
/// metadata map* and *Category assignments*, both **Copy + re-validate**).
/// The values are keyed by definition **id**, as the frozen content carries
/// them; the re-validation resolves the ids to their live definitions and
/// judges each against `02`'s rules before a row is written.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CloneContent {
    /// `(category_id, role)` — a Product's assignments; empty for a SKU.
    pub assignments: Vec<(Uuid, crate::domain::taxonomy::AssignmentRole)>,
    /// The attribute values at their coordinates.
    pub values: Vec<crate::domain::taxonomy::FrozenAttributeValue>,
    /// The metadata map, `(key, value)`.
    pub metadata: Vec<(String, String)>,
}

/// The classification a clone writes on the copy (P-D-145): the source's,
/// with the two values the doors require defaulted where an older source
/// carries none — a `product` on the `standard` tier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceClassification {
    pub sku_type: String,
    pub sellable: bool,
    pub plan_tier: String,
    pub tax_category_ref: Option<String>,
    pub gl_code_ref: Option<String>,
}

impl SkuCloneSource {
    /// The five classification fields the copy is created with.
    #[must_use]
    pub fn classification(&self) -> SourceClassification {
        SourceClassification {
            sku_type: self.sku_type.clone().unwrap_or_else(|| {
                crate::domain::recognized::SkuType::Product
                    .as_str()
                    .to_owned()
            }),
            sellable: self.sellable,
            plan_tier: self
                .plan_tier
                .clone()
                .unwrap_or_else(|| crate::domain::recognized::DEFAULT_PLAN_TIER.to_owned()),
            tax_category_ref: self.tax_category_ref.clone(),
            gl_code_ref: self.gl_code_ref.clone(),
        }
    }
}

/// The suggested name for attempt `n` (1-based), per `inst-cn-rename` and
/// **P-D-62**: `{name}-copy-N` for a live-lineage source, `-revived`
/// flavored for a retired one — and a second revival of the lineage
/// `-revived-N`, the same first-free rule over the flavored family.
#[must_use]
pub fn suggested_product_name(source: &ProductCloneSource, n: u32) -> String {
    if source.retired {
        if n == 1 {
            format!("{}-revived", source.name)
        } else {
            format!("{}-revived-{n}", source.name)
        }
    } else {
        format!("{}-copy-{n}", source.name)
    }
}

/// The suggested Product code for attempt `n`: `{source}-copy-N`, and none
/// where the source carries none (the clone's stays null —
/// `inst-cn-identity`, L5).
#[must_use]
pub fn suggested_product_code(source: &ProductCloneSource, n: u32) -> Option<String> {
    source
        .product_code
        .as_ref()
        .map(|code| format!("{code}-copy-{n}"))
}

/// The suggested SKU code for attempt `n`: `{source}-copy-N`, unflavored —
/// the `-revived` flavor is the rename rule's and SKUs have no name.
#[must_use]
pub fn suggested_sku_code(source: &SkuCloneSource, n: u32) -> String {
    format!("{}-copy-{n}", source.sku_code)
}

/// The operational cap on the first-free walk. Not a semantic bound — the
/// walk's length is the lineage's own clone count (P-D-62) — but an insert
/// loop with no ceiling would spin on a store that refuses every candidate
/// for a reason that is not a name collision at all. Past it, the last
/// conflict is surfaced as the ordinary refusal.
pub const CLONE_SUGGESTION_ATTEMPTS: u32 = 100;

#[cfg(test)]
#[path = "disposition_tests.rs"]
mod disposition_tests;
