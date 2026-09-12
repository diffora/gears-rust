//! The field-mutability bucket registry: which bucket a Foundation-owned
//!
//! @cpt-dod:cpt-cf-bss-products-dod-bucket-registration:p1
//! @cpt-dod:cpt-cf-bss-products-dod-meter-bucket:p1
//! column sits in, and what the answer is when it sits in none
//! (`design/01-foundation.md` §4.1's *"Bucket assignment for the
//! Foundation-owned columns"*, §5's agreement test, `inst-fd-bucket-tags`,
//! `inst-fd-mutability-frame`).
//!
//! @cpt-cf-bss-products-dod-save-door
//!
//! This module answers exactly one question — *which class is this column in*
//! — and takes no action on the answer. Refusing a bucket-i write after first
//! publish (`inst-fd-bucket-i-refusal`), naming slice 07's correction door on a
//! bucket-ii write (`inst-fd-bucket-ii-refusal`) and re-publishing a bucket-iii
//! change as version N+1 (`inst-fd-bucket-iii-iv`) are the save door's, and the
//! door is the next slice's. What is here is the map that door routes by.
//!
//! # The registry is keyed by entity **and** column, not by column
//!
//! `product_id` is the reason. On `products_sku` it is the parent link and
//! **bucket-i** (§4.1, owner's call 2026-08-27: *"re-parenting changes whose
//! SKU it is, not how it is described"*). On `products_product` the identical
//! name is the primary key, and §4.2 puts row identity *"outside the bucket
//! scheme entirely and admitted in no UPDATE at all"* (P-D-34). A registry
//! keyed by column name alone would have to answer one of those for both
//! tables and would be wrong on one of them at every call.
//!
//! # Two kinds of untagged, and why the type has to tell them apart
//!
//! §5 names the columns that *"carry no bucket tag and are outside the
//! comparison"*: the mechanical ones the trigger whitelist names by hand
//! (`lifecycle_state`, `published_version`, `internal_revision`,
//! `deprecation_provenance`, `replaced_by_sku_id`, `composition_pending`,
//! `correction_ref`, `cloned_from`, the update timestamp) *"together with the row-identity
//! columns `tenant_id`, the primary key and `created_by`"*.
//!
//! Those are **deliberately** outside the scheme: they have no bucket because
//! no request field maps to them, so no door routes them at all. A
//! **published-state** column with no tag is the opposite — a column that
//! should have been tagged and was not. Collapsing the two into one "no
//! answer" makes P-D-50's fail-closed rule either refuse the mechanical
//! columns the gear writes on every save, or admit the untagged one it exists
//! to catch. So the two are different values here:
//! [`FieldClass::Outside`] with its reason, versus a refusal from
//! [`classify`].
//!
//! # Fail closed, and why it matters that the physical layer disagrees
//!
//! P-D-50: *"a published-state column carrying no tag means it was added
//! without registering one, and the head door refuses the write under the
//! pipeline's own posture rather than routing to a default bucket"*.
//!
//! The rule reads as belt-and-braces next to a physical trigger that already
//! whitelists columns, and it is not. P-D-32 makes this registry **advisory
//! for the physical layer**: *"a compile-time Rust map has no read path from a
//! migration-time trigger"*, so the trigger's column classes stay static DDL
//! and the two statements of the rule can drift. The fail-closed miss is what
//! decides *how* they drift. Route a miss to a default bucket and the door
//! admits a write the trigger then rejects with a database error — an
//! operator-facing 500 for what is a governed refusal. Refuse it at the door
//! and the drift surfaces as the gear's ordinary `ILLEGAL_FIELD_MUTATION`,
//! against the column that caused it.
//!
//! That is why [`classify`] returns a `Result` and not an `Option`-with-a-
//! default, why neither [`FieldClass`] nor [`FieldBucket`] implements
//! `Default`, and why there is no catch-all arm anywhere below: a bucket can
//! only be obtained by a column matching a row of [`columns`] exactly.
//!
//! # What this module ships less of than the design describes
//!
//! - **Bucket iv has no members; bucket ii's arrived with slice 03.** §4.1
//!   assigns no Foundation column to either, and the two absences were owed
//!   by different slices — 03 has since paid its half. Bucket ii's columns
//!   are **slice 03's**: `design/03-sku-classification.md` §C6 registers
//!   *"`type`, metering-unit declaration (incl. `usageTypeRef`) →
//!   bucket ii (immutable-but-correctable, slice 07)"*, and the declaration
//!   pair now ships on `products_sku`; 03 owns the columns and their
//!   registration while slice 07 still owns the correction door that
//!   writes them after first publish — which is the door
//!   `inst-fd-bucket-ii-refusal` has this gear name rather than forward to.
//!   Bucket iv is `fr-field-mutability-matrix`'s *"other descriptive fields"*
//!   catch-all, and **no document in the set puts a named column in it**: what
//!   is known is that Foundation assigns none, not which slice owes the first
//!   row. Both tags are encoded because the door routes by tag, and a tag
//!   that appears only when its first column lands is a second change to the
//!   door.
//! - **[`FieldClass::CreateOnly`] carries the `cloned_from` pair, on both
//!   kinds.** §4.1 makes them *"stricter than bucket-i — writable only in the
//!   creating statement and never again, not merely never after first publish,
//!   so the lineage stays evidence rather than a claim"*. The columns landed
//!   with slice 11 (**P-D-76**), so the class's membership is no longer empty
//!   and a `cloned_from` write is refused **by the create-only rule** rather
//!   than by the fail-closed miss — the debt this paragraph used to record,
//!   paid. The physical guard holds the same rule from the other side: the
//!   head tables' immutable-column arms name the pair.
//!
//!   @cpt-dod:cpt-cf-bss-products-dod-create-only-class:p3
//! - **The `created_at` call.** §5's row-identity roster names three columns
//!   and the update timestamp separately; it does not name `created_at`. Both
//!   head-row triggers refuse a change to it in the same clause as
//!   `tenant_id`, the primary key and `created_by`
//!   (`m20260829_000002_create_products_product.rs`,
//!   `m20260829_000003_create_products_sku.rs`), which is row-identity
//!   treatment and not the update timestamp's, so it is registered as row
//!   identity here. Nothing routes on the distinction between the two
//!   outside-the-scheme reasons today; if the owner reads `created_at` as
//!   mechanical instead, this is a one-row change.
//!
//!   The Phase 7 review confirmed the reading and judged the **design**
//!   incomplete rather than this row wrong: the trigger clause is the
//!   evidence, it puts `created_at` with `tenant_id`, the primary key and
//!   `created_by`, and `updated_at` — the column §5 does name separately — is
//!   guarded by no clause at all. **§5's row-identity roster is therefore
//!   owed the addition**, and until it carries `created_at` this module is
//!   the only place the classification is written down.
//! - **§5's agreement test is built, in two halves and two files.** The half
//!   asserting the registry and the physical tables name the same columns is
//!   in `bucket_tests.rs`, read off the entity models. The half asserting the
//!   same *classes* against the trigger whitelist - iii and iv combined, since
//!   the whitelist admits them together and cannot distinguish them - is
//!   `infra::storage::migrations_tests::bucket_agreement_tests`, which reads
//!   the executed `SQLite` triggers out of `sqlite_master` and the `PL/pgSQL`
//!   clauses out of the migration source. It carries **P-D-50**'s third
//!   assertion too: no published-state column is named by *neither* artifact,
//!   which is the case the first two are blind to by construction and which a
//!   perturbation confirmed - a column added to the table alone leaves the
//!   same-columns assertions green and reddens only that one.

use bss_products_sdk::models::EntityKind;
use toolkit_macros::domain_model;

use crate::domain::error::DomainError;

/// A bucket tag, i-iv (`inst-fd-bucket-tags`, PRD
/// `fr-field-mutability-matrix`).
///
/// Named rather than numbered because the numeral says nothing about the rule:
/// a reader at a call site should see *what the door may do with the field*,
/// not a position in a list. [`FieldBucket::tag`] gives the numeral back for
/// anyone comparing against the design.
#[domain_model]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldBucket {
    /// **i, structural.** Identity: writable only while
    /// `published_version = 0` on a non-terminal head, and never after first
    /// publish (`inst-fd-bucket-i-refusal`, §4.2's whitelist under P-D-28).
    /// A mis-set identity on a published entity is corrected by
    /// retire-and-clone, not by a write.
    Structural,
    /// **ii, correctable.** Admitted at the save door while
    /// `published_version = 0` (P-D-41) and, after first publish, only through
    /// slice 07's correction door — which the head door's refusal names rather
    /// than forwarding to (`inst-fd-bucket-ii-refusal`: one door, one effect).
    ///
    /// **Two members, both on `products_sku`** since 03's meter pair landed:
    /// `metering_unit` and `usage_type_ref`. The Product table still has
    /// none.
    Correctable,
    /// **iii, material-mutable.** Governed content: an ordinary head-row save
    /// while `lifecycle_state` is non-terminal, coming out as version N+1
    /// under the governance gate, with materiality judged by slice 05
    /// (`inst-fd-bucket-iii-iv`).
    MaterialMutable,
    /// **iv, descriptive.** The matrix's *"other descriptive fields"*
    /// catch-all: the same save-and-re-publish path as bucket iii, differing
    /// only in the materiality 05 reads off it.
    ///
    /// **No column today**; see the module doc. It is a tag, never a default —
    /// nothing in this module routes an unrecognised column here.
    Descriptive,
}

impl FieldBucket {
    /// The design's own roman numeral for the tag, so a reader comparing this
    /// registry against §4.1 or `inst-fd-bucket-tags` compares like with like.
    ///
    /// It is also the numeral the save doors' refusals are **worded** with:
    /// `api::rest::products::structural_after_publish` and
    /// `correctable_after_publish`, and their two SKU twins, take it from
    /// here rather than spelling `bucket-i` and `bucket-ii` at four call
    /// sites. That is the drift this accessor exists to prevent — a message
    /// naming one bucket while the registry refused the write under another —
    /// and it is why the numeral lives in the registry that decides rather
    /// than in the strings that report.
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::Structural => "i",
            Self::Correctable => "ii",
            Self::MaterialMutable => "iii",
            Self::Descriptive => "iv",
        }
    }
}

/// Why a column carries no bucket tag **on purpose** (§5).
///
/// The reason is carried rather than dropped because the two classes are
/// refused by different rules and a later reader — the §5 agreement test
/// above all — needs to know which one it is looking at. Neither is routable:
/// no request field maps to either class, so a door reaching one of these
/// values has been handed a column no payload should have produced.
#[domain_model]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutsideTheScheme {
    /// A **mechanical** column the head-row trigger whitelist names by hand:
    /// the lifecycle and version columns, `internal_revision`, the
    /// system-owned flags, and the update timestamp. The gear writes these on
    /// its own behalf, each under its own predicate in §4.2, and never from a
    /// payload field.
    Mechanical,
    /// A **row-identity** column — `tenant_id`, the primary key, `created_by`
    /// and (see the module doc) `created_at`. §4.2, P-D-34: *"outside the
    /// bucket scheme entirely and admitted in no UPDATE at all"*, which is
    /// `cloned_from`'s treatment rather than the matrix's bucket-iv catch-all.
    RowIdentity,
}

/// What the registry knows about one column.
///
/// Three arms, because "has a bucket", "is stricter than every bucket" and "is
/// outside the scheme" are three different routing outcomes at the door. There
/// is deliberately no fourth arm for *unknown*: an unknown column is not a
/// class, it is a refusal, and giving it a value here would let a caller
/// pattern-match its way past [`classify`]'s `Result`.
#[domain_model]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldClass {
    /// A tagged published-state column, with the tag the door routes by.
    Bucket(FieldBucket),
    /// **Stricter than bucket-i** (§4.1): writable in the creating statement
    /// and in no UPDATE at all, not merely in none after first publish.
    /// `cloned_from`'s class, which no column carries until slice 11.
    CreateOnly,
    /// Deliberately outside the scheme, with the reason.
    Outside(OutsideTheScheme),
}

impl FieldClass {
    /// The bucket this class routes by, where it has one.
    ///
    /// `None` for [`Self::CreateOnly`] and [`Self::Outside`] — both of which
    /// are refusals at any update, by rules of their own — and reachable only
    /// from a class [`classify`] already returned. It is the single narrowing
    /// from a class to a bucket, which is what makes the fail-closed posture
    /// checkable: `classify(..).ok().and_then(FieldClass::bucket)` is the
    /// whole public path from a column name to a [`FieldBucket`], and on a
    /// miss it is `None` at the first step.
    #[must_use]
    pub const fn bucket(self) -> Option<FieldBucket> {
        match self {
            Self::Bucket(bucket) => Some(bucket),
            Self::CreateOnly | Self::Outside(_) => None,
        }
    }
}

/// One registry row: a physical column and the class it is in.
///
/// A named pair rather than a tuple so the tables below read as the
/// assignment §4.1 states, and so a row cannot be built with its two halves
/// the wrong way round.
#[domain_model]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColumnTag {
    /// The physical column name, exactly as `products_product` /
    /// `products_sku` spell it. Matching is exact: the registry keys on the
    /// column, and mapping a request field to a column is the door's job, done
    /// before it asks.
    pub column: &'static str,
    /// The class §4.1 or §5 puts the column in.
    pub class: FieldClass,
}

/// `products_product`'s columns, all fourteen of them (§4.1).
///
/// Every column of the table is here, tagged or explicitly outside the scheme,
/// because a registry that listed only the tagged ones could not tell a
/// mechanical column from an unregistered one — which is the distinction
/// P-D-50's fail-closed rule is made of. `bucket_tests` asserts this against
/// the entity model, so a column added to the table without a row here is a
/// red test rather than a runtime refusal.
const PRODUCT_COLUMNS: [ColumnTag; 19] = [
    // Row identity (§4.2, P-D-34): admitted in no UPDATE at all.
    ColumnTag {
        column: "product_id",
        class: FieldClass::Outside(OutsideTheScheme::RowIdentity),
    },
    ColumnTag {
        column: "tenant_id",
        class: FieldClass::Outside(OutsideTheScheme::RowIdentity),
    },
    ColumnTag {
        column: "created_by",
        class: FieldClass::Outside(OutsideTheScheme::RowIdentity),
    },
    ColumnTag {
        column: "cloned_from",
        // The class built waiting for this column (its own doc named it):
        // writable in the creating statement and in no UPDATE at all
        // (P-D-76; the head guard enforces the same pair physically).
        class: FieldClass::CreateOnly,
    },
    ColumnTag {
        column: "cloned_from_version",
        class: FieldClass::CreateOnly,
    },
    ColumnTag {
        column: "created_at",
        class: FieldClass::Outside(OutsideTheScheme::RowIdentity),
    },
    // Bucket i (§4.1): `brand_id` because re-branding moves the row into a
    // different `(tenant_id, brand_id, name_normalized)` scope — the very key
    // the partial unique index enforces on — and `product_code` under AC #1's
    // "under the same rules" as `skuCode`.
    ColumnTag {
        column: "brand_id",
        class: FieldClass::Bucket(FieldBucket::Structural),
    },
    ColumnTag {
        column: "product_code",
        class: FieldClass::Bucket(FieldBucket::Structural),
    },
    // Bucket iii (§4.1): a published Product can be renamed, and the rename
    // comes out as version N+1 under governance rather than forcing
    // retire-and-clone. `name_normalized` is the same field's index operand
    // and moves with it.
    ColumnTag {
        column: "name",
        class: FieldClass::Bucket(FieldBucket::MaterialMutable),
    },
    ColumnTag {
        column: "name_normalized",
        class: FieldClass::Bucket(FieldBucket::MaterialMutable),
    },
    // Bucket iii **in both directions**, widening and narrowing alike (§4.1).
    // A narrowing is admitted here on purpose and judged elsewhere: the
    // orphaning case — a narrowing that would leave a live child outside its
    // parent — is refused `SCOPE_NOT_CONTAINED` by
    // `crate::api::rest::products::check_children_stay_contained`, which the
    // save door runs in the registered-validators phase, ahead of the
    // governance gate, exactly where §4.1 puts
    // `fr-parent-child-integrity`'s fail-closed check. A stricter tag here
    // would refuse the narrowing that orphans nothing along with it.
    ColumnTag {
        column: "region_scope",
        class: FieldClass::Bucket(FieldBucket::MaterialMutable),
    },
    ColumnTag {
        column: "brand_scope",
        class: FieldClass::Bucket(FieldBucket::MaterialMutable),
    },
    // Mechanical (§5): each written by the gear under its own §4.2 predicate,
    // never from a payload field.
    ColumnTag {
        column: "lifecycle_state",
        class: FieldClass::Outside(OutsideTheScheme::Mechanical),
    },
    ColumnTag {
        column: "internal_revision",
        class: FieldClass::Outside(OutsideTheScheme::Mechanical),
    },
    ColumnTag {
        column: "published_version",
        class: FieldClass::Outside(OutsideTheScheme::Mechanical),
    },
    ColumnTag {
        column: "updated_at",
        class: FieldClass::Outside(OutsideTheScheme::Mechanical),
    },
    ColumnTag {
        // Slice 04's stamp (`dod-lifecycle-columns`). `Mechanical` is
        // measured rather than chosen: `design/01` §4.3 groups
        // `deprecation_provenance` with `lifecycle_state`,
        // `replaced_by_sku_id` and `internal_revision` as the four that
        // *"move on transitions, which write no version row"* (P-D-24 as
        // P-D-35 extended it), and two of that four are registered
        // `Mechanical` right here. The gear stamps it on its own behalf under
        // the deprecate transition's predicate, and no save may name it.
        column: "deprecation_provenance",
        class: FieldClass::Outside(OutsideTheScheme::Mechanical),
    },
    // The two row collections frozen inside the content (P-D-29, P-D-153):
    // not head-row columns — the assignment and value tables are their own —
    // but content keys the save door writes and the next publish freezes, so
    // they classify as bucket iii, exactly like `name`. A publish's
    // re-validation and a submission's materiality walk the content keys and
    // ask this registry; an unregistered key is refused, not defaulted.
    ColumnTag {
        column: "categories",
        class: FieldClass::Bucket(FieldBucket::MaterialMutable),
    },
    ColumnTag {
        column: "attributes",
        class: FieldClass::Bucket(FieldBucket::MaterialMutable),
    },
];

/// `products_sku`'s columns (§4.2).
///
/// Two differences from [`PRODUCT_COLUMNS`] are the ones worth reading twice:
/// `product_id` is the **parent link** and bucket-i here, where on the Product
/// it is the primary key; and the table carries **no `name`**, so a `name`
/// field arriving for a SKU is a miss and is refused rather than routed to the
/// Product's tag.
pub(crate) const SKU_COLUMNS: [ColumnTag; 26] = [
    // Row identity (§4.2, P-D-34).
    ColumnTag {
        column: "sku_id",
        class: FieldClass::Outside(OutsideTheScheme::RowIdentity),
    },
    ColumnTag {
        column: "tenant_id",
        class: FieldClass::Outside(OutsideTheScheme::RowIdentity),
    },
    ColumnTag {
        column: "created_by",
        class: FieldClass::Outside(OutsideTheScheme::RowIdentity),
    },
    ColumnTag {
        column: "cloned_from",
        // The class built waiting for this column (its own doc named it):
        // writable in the creating statement and in no UPDATE at all
        // (P-D-76; the head guard enforces the same pair physically).
        class: FieldClass::CreateOnly,
    },
    ColumnTag {
        column: "cloned_from_version",
        class: FieldClass::CreateOnly,
    },
    ColumnTag {
        column: "created_at",
        class: FieldClass::Outside(OutsideTheScheme::RowIdentity),
    },
    // Bucket i (§4.1): the code under AC #1, and the parent link by the
    // owner's call of 2026-08-27 — re-parenting changes *whose* SKU it is,
    // which puts it with identity rather than with governed content, so a
    // mis-parented published SKU is corrected by retire-and-clone.
    ColumnTag {
        column: "sku_code",
        class: FieldClass::Bucket(FieldBucket::Structural),
    },
    ColumnTag {
        column: "product_id",
        class: FieldClass::Bucket(FieldBucket::Structural),
    },
    // Bucket iii in both directions (§4.1), the child's own scope sets.
    ColumnTag {
        column: "region_scope",
        class: FieldClass::Bucket(FieldBucket::MaterialMutable),
    },
    ColumnTag {
        column: "brand_scope",
        class: FieldClass::Bucket(FieldBucket::MaterialMutable),
    },
    // 03's classification columns (P-D-145). The type profile is bucket ii —
    // `dod-bucket-registration` puts `type` beside the metering declaration —
    // so after first publish only the correction door moves it; the other
    // four are ordinary governed content (a `sellable` flip is material by
    // P-D-131 row 16 — the materiality evaluator's business, not the tag's).
    ColumnTag {
        column: "sku_type",
        class: FieldClass::Bucket(FieldBucket::Correctable),
    },
    ColumnTag {
        column: "sellable",
        class: FieldClass::Bucket(FieldBucket::MaterialMutable),
    },
    ColumnTag {
        column: "plan_tier",
        class: FieldClass::Bucket(FieldBucket::MaterialMutable),
    },
    ColumnTag {
        column: "tax_category_ref",
        class: FieldClass::Bucket(FieldBucket::MaterialMutable),
    },
    ColumnTag {
        column: "gl_code_ref",
        class: FieldClass::Bucket(FieldBucket::MaterialMutable),
    },
    // Mechanical (§5). `composition_pending` is named there explicitly and is
    // system-owned besides: §4.2 admits a change to it only in the same
    // statement as a `published_version` bump, so bucket iii/iv would be the
    // wrong home for it (P-D-32).
    ColumnTag {
        column: "lifecycle_state",
        class: FieldClass::Outside(OutsideTheScheme::Mechanical),
    },
    ColumnTag {
        column: "internal_revision",
        class: FieldClass::Outside(OutsideTheScheme::Mechanical),
    },
    ColumnTag {
        column: "published_version",
        class: FieldClass::Outside(OutsideTheScheme::Mechanical),
    },
    ColumnTag {
        // Slice 04's stamp (`dod-lifecycle-columns`). `Mechanical` is
        // measured rather than chosen: `design/01` §4.3 groups
        // `deprecation_provenance` with `lifecycle_state`,
        // `replaced_by_sku_id` and `internal_revision` as the four that
        // *"move on transitions, which write no version row"* (P-D-24 as
        // P-D-35 extended it), and two of that four are registered
        // `Mechanical` right here. The gear stamps it on its own behalf under
        // the deprecate transition's predicate, and no save may name it.
        column: "deprecation_provenance",
        class: FieldClass::Outside(OutsideTheScheme::Mechanical),
    },
    ColumnTag {
        // The successor a retirement names — written by that act from its own
        // optional input, never by a save, and cleared by the governed cancel
        // (P-D-49). Same grouping argument as the column above.
        column: "replaced_by_sku_id",
        class: FieldClass::Outside(OutsideTheScheme::Mechanical),
    },
    ColumnTag {
        column: "composition_pending",
        class: FieldClass::Outside(OutsideTheScheme::Mechanical),
    },
    ColumnTag {
        column: "updated_at",
        class: FieldClass::Outside(OutsideTheScheme::Mechanical),
    },
    // Bucket ii's first members — 03's atomic `MeterDeclaration` pair
    // (`05` §3.1 tags the metering-unit field bucket ii; the pair travels
    // together, `inst-mt-atomic-pair`). The save door admits them while
    // `published_version = 0` (P-D-41); after first publish the write is
    // slice 07's correction act, and the head guard's interim row-image
    // predicate (P-D-34) enforces the same-statement-as-a-bump floor.
    ColumnTag {
        column: "metering_unit",
        class: FieldClass::Bucket(FieldBucket::Correctable),
    },
    ColumnTag {
        column: "usage_type_ref",
        class: FieldClass::Bucket(FieldBucket::Correctable),
    },
    // P-D-129's door identity. Mechanical, like `composition_pending` and for
    // the same reason: written only by the publish door's own head-row
    // UPDATE (07's correction re-publish), never by an operator save, and the
    // head guard pins it to the same statement as a `published_version` bump.
    ColumnTag {
        column: "correction_ref",
        class: FieldClass::Outside(OutsideTheScheme::Mechanical),
    },
    // The attribute-value collection frozen inside the content (P-D-29,
    // P-D-153): a content key, not a head-row column; bucket iii like `name`
    // on a Product. See `PRODUCT_COLUMNS`' note.
    ColumnTag {
        column: "attributes",
        class: FieldClass::Bucket(FieldBucket::MaterialMutable),
    },
];

/// Every registry row for an entity kind, in table order.
///
/// Exposed because it is the operand of §5's agreement test and of anything
/// else that has to reason about the registry as a set rather than about one
/// column. It is **not** a lookup: reading a class off this slice by index or
/// by a defaulted search is exactly the accident [`classify`] exists to
/// prevent.
#[must_use]
pub const fn columns(kind: EntityKind) -> &'static [ColumnTag] {
    match kind {
        EntityKind::Product => &PRODUCT_COLUMNS,
        EntityKind::Sku => &SKU_COLUMNS,
    }
}

/// The registry lookup: the class of `column` on `kind`, or a refusal.
///
/// The only way to obtain a [`FieldClass`], and therefore — through
/// [`FieldClass::bucket`] — the only way to obtain a [`FieldBucket`] from a
/// column name. The match is exact; normalizing the key here would let a field
/// spelling the physical table does not have answer with a bucket.
///
/// # Errors
///
/// [`DomainError::IllegalFieldMutation`] where the entity has no row for the
/// column — P-D-50's fail-closed miss. The code is the state phase's own
/// (§3.3: *"a write the head door may not take"*), because that is what a
/// column no one tagged is: the door cannot show it is admitted, and the
/// registry will not invent a bucket to make it so. The reason names the
/// entity and the column, so the operator answer says which field was refused
/// and an operator reading it can see it is an unregistered one rather than a
/// governed refusal.
/// The registry members that are **content collections, not head columns**
/// (P-D-29, P-D-153): `categories` on a Product, `attributes` on both kinds.
/// They carry a bucket tag like any content key the save door writes and the
/// publish freezes, and no physical column of the head table — the assignment
/// and value tables are their own. The agreement probes between this registry
/// and the executed schema skip them by this predicate.
pub const CONTENT_COLLECTIONS: [&str; 2] = ["attributes", "categories"];

/// Whether `column` is one of the [`CONTENT_COLLECTIONS`].
#[must_use]
pub fn is_content_collection(column: &str) -> bool {
    CONTENT_COLLECTIONS.contains(&column)
}

pub fn classify(kind: EntityKind, column: &str) -> Result<FieldClass, DomainError> {
    columns(kind)
        .iter()
        .find(|tag| tag.column == column)
        .map(|tag| tag.class)
        .ok_or_else(|| {
            DomainError::IllegalFieldMutation(format!(
                "{} column {column} carries no bucket tag: refused rather than routed to a default bucket",
                kind.as_str(),
            ))
        })
}

#[cfg(test)]
#[path = "bucket_tests.rs"]
mod bucket_tests;
