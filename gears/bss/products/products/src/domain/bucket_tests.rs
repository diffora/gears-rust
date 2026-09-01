//! Tests for the field-mutability bucket registry.
//!
//! # Why some cases name columns and others derive them
//!
//! Two different questions are being asked, and only one of them can be
//! answered from the registry itself.
//!
//! The first is *"does the registry say what `design/01-foundation.md` §4.1
//! says"*. That is a claim about a document, and a case that loops over
//! [`super::columns`] cannot measure it — such a loop passes against **any**
//! table, including a wrong one, exactly as
//! `transition_tests::the_five_admitted_edges_are_admitted` argues about
//! [`crate::domain::transition::ADMITTED_EDGES`]. So the tagged columns are
//! named one by one, each against the sentence of §4.1 that assigns it. These
//! literals are **not a second roster that can drift silently**: the class
//! counts and the physical-agreement case below both fail loudly the moment a
//! column is added, removed or re-tagged, so a stale literal here is a red
//! test rather than a passing one.
//!
//! The second is *"is the registry complete, and does it agree with the
//! physical layer"*. That one is derived — from `products_product` and
//! `products_sku`'s own generated `Column` enums — because the drift P-D-32
//! admits is precisely between the registry and the physical column set, and a
//! hand-written column list on this side would be one more copy to keep in
//! step rather than a measurement of the two.
//!
//! # The `sea_orm` import in a domain test
//!
//! The module under test imports nothing from `crate::infra`, and must not.
//! This test file does, for the reason above: §5 requires *"a test asserting
//! the `BucketRegistry`'s tag map and §4.2's trigger column classes name the
//! same columns in the same classes"*, and the entity models are the only
//! machine-readable statement of what those columns are. The half of §5's
//! test that reads the trigger's whitelist out of the migration SQL is not
//! here: it is
//! `crate::infra::storage::migrations_tests::bucket_agreement_tests`, built by
//! this slice, where the executed `SQLite` triggers and the Postgres
//! migration source are both in reach.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use bss_products_sdk::models::EntityKind;
use sea_orm::{IdenStatic as _, Iterable as _};

use super::{ColumnTag, FieldBucket, FieldClass, OutsideTheScheme, classify, columns};
use crate::domain::error::DomainError;
use crate::infra::storage::entity::{product, sku};

/// Both entity kinds, so a case can quantify over the registry rather than
/// name a sample of it.
const BOTH_KINDS: [EntityKind; 2] = [EntityKind::Product, EntityKind::Sku];

/// The class the registry answers, or a panic naming the column.
fn class_of(kind: EntityKind, column: &str) -> FieldClass {
    classify(kind, column)
        .unwrap_or_else(|_| panic!("{} column {column} must classify", kind.as_str()))
}

/// How many rows of an entity's registry carry a given class.
fn count_of(kind: EntityKind, class: FieldClass) -> usize {
    columns(kind)
        .iter()
        .filter(|tag| tag.class == class)
        .count()
}

/// The physical column names of `products_product`, read from the entity the
/// migration created rather than from a list written here.
fn physical_product_columns() -> Vec<&'static str> {
    product::Column::iter().map(|col| col.as_str()).collect()
}

/// The physical column names of `products_sku`.
fn physical_sku_columns() -> Vec<&'static str> {
    sku::Column::iter().map(|col| col.as_str()).collect()
}

/// §4.1: *"`name` and `name_normalized` are **bucket-iii**"*, *"`region_scope`
/// and `brand_scope` are **bucket-iii in both directions**"*, *"`sku_code`,
/// `product_code` and **`brand_id`** are **bucket-i**"*.
///
/// Would catch a Product column re-tagged in the registry against the
/// document — the rename that came out as bucket-i and forced retire-and-clone
/// on every published Product, or the `brand_id` that became editable after
/// publish and moved a row into a different uniqueness scope.
#[test]
fn the_products_tagged_columns_answer_the_buckets_section_4_1_assigns() {
    let kind = EntityKind::Product;

    assert_eq!(
        class_of(kind, "brand_id"),
        FieldClass::Bucket(FieldBucket::Structural),
    );
    assert_eq!(
        class_of(kind, "product_code"),
        FieldClass::Bucket(FieldBucket::Structural),
    );
    assert_eq!(
        class_of(kind, "name"),
        FieldClass::Bucket(FieldBucket::MaterialMutable),
    );
    assert_eq!(
        class_of(kind, "name_normalized"),
        FieldClass::Bucket(FieldBucket::MaterialMutable),
    );
    assert_eq!(
        class_of(kind, "region_scope"),
        FieldClass::Bucket(FieldBucket::MaterialMutable),
    );
    assert_eq!(
        class_of(kind, "brand_scope"),
        FieldClass::Bucket(FieldBucket::MaterialMutable),
    );
}

/// §4.1 again, for the SKU: `sku_code` and the parent link are bucket-i, the
/// two scope columns bucket-iii. A SKU carries no `name` column at all
/// (§4.2's roster), which the miss case below measures.
///
/// Would catch a `region_scope` narrowing routed as identity, which §4.1
/// forbids in as many words: bucket-iii *"in both directions, widening and
/// narrowing alike"*.
#[test]
fn the_skus_tagged_columns_answer_the_buckets_section_4_1_assigns() {
    let kind = EntityKind::Sku;

    assert_eq!(
        class_of(kind, "sku_code"),
        FieldClass::Bucket(FieldBucket::Structural),
    );
    assert_eq!(
        class_of(kind, "product_id"),
        FieldClass::Bucket(FieldBucket::Structural),
    );
    assert_eq!(
        class_of(kind, "region_scope"),
        FieldClass::Bucket(FieldBucket::MaterialMutable),
    );
    assert_eq!(
        class_of(kind, "brand_scope"),
        FieldClass::Bucket(FieldBucket::MaterialMutable),
    );
}

/// One column name, two classes, decided by the entity it sits on.
///
/// `product_id` is the SKU's *"parent link"* and **bucket-i** (§4.1, owner's
/// call 2026-08-27); on `products_product` the same name is the primary key
/// and therefore row identity, *"outside the bucket scheme entirely and
/// admitted in no UPDATE at all"* (§4.2, P-D-34). Would catch a registry keyed
/// by column name alone, which would have to pick one of the two answers and
/// would be wrong on the other entity every time.
#[test]
fn product_id_is_the_parent_link_on_a_sku_and_row_identity_on_a_product() {
    assert_eq!(
        class_of(EntityKind::Sku, "product_id"),
        FieldClass::Bucket(FieldBucket::Structural),
    );
    assert_eq!(
        class_of(EntityKind::Product, "product_id"),
        FieldClass::Outside(OutsideTheScheme::RowIdentity),
    );
}

/// §5: the mechanical columns *"carry no bucket tag and are outside the
/// comparison"*.
///
/// Asserted as **outside the scheme** *and* as **not a bucket** — the second
/// half is the one that matters, because a mechanical column dropped into
/// bucket iv is an ordinary operator save re-stamping `lifecycle_state` or
/// clearing `composition_pending`.
#[test]
fn a_mechanical_column_is_outside_the_scheme_and_is_not_a_bucket() {
    let mechanical = [
        (EntityKind::Product, "lifecycle_state"),
        (EntityKind::Product, "published_version"),
        (EntityKind::Product, "internal_revision"),
        (EntityKind::Product, "updated_at"),
        (EntityKind::Sku, "lifecycle_state"),
        (EntityKind::Sku, "published_version"),
        (EntityKind::Sku, "internal_revision"),
        (EntityKind::Sku, "composition_pending"),
        (EntityKind::Sku, "updated_at"),
    ];

    for (kind, column) in mechanical {
        let class = class_of(kind, column);
        assert_eq!(
            class,
            FieldClass::Outside(OutsideTheScheme::Mechanical),
            "{} {column} is mechanical",
            kind.as_str(),
        );
        assert_eq!(
            class.bucket(),
            None,
            "{} {column} has no bucket",
            kind.as_str()
        );
    }
}

/// §5: *"together with the row-identity columns `tenant_id`, the primary key
/// and `created_by`, carry no bucket tag and are outside the comparison"*.
///
/// `created_at` is judged here as row identity rather than mechanical: the
/// document's roster does not name it, and both head-row triggers refuse any
/// change to it in the same clause as `tenant_id`, the primary key and
/// `created_by`. Would catch either of them becoming a bucket, which is a
/// tenant boundary and a provenance record respectively.
#[test]
fn a_row_identity_column_is_outside_the_scheme_and_is_not_a_bucket() {
    let row_identity = [
        (EntityKind::Product, "product_id"),
        (EntityKind::Product, "tenant_id"),
        (EntityKind::Product, "created_by"),
        (EntityKind::Product, "created_at"),
        (EntityKind::Sku, "sku_id"),
        (EntityKind::Sku, "tenant_id"),
        (EntityKind::Sku, "created_by"),
        (EntityKind::Sku, "created_at"),
    ];

    for (kind, column) in row_identity {
        let class = class_of(kind, column);
        assert_eq!(
            class,
            FieldClass::Outside(OutsideTheScheme::RowIdentity),
            "{} {column} is row identity",
            kind.as_str(),
        );
        assert_eq!(
            class.bucket(),
            None,
            "{} {column} has no bucket",
            kind.as_str()
        );
    }
}

/// P-D-50: *"a published-state column carrying no tag means it was added
/// without registering one, and the head door refuses the write under the
/// pipeline's own posture rather than routing to a default bucket"*.
///
/// The samples are real future columns — §4.2's `sellable`, `plan_tier`,
/// `metering_unit` and `type`, none of which exists today, all four owed by
/// slice `03` — plus `name` on a SKU, which is a Product column and not a SKU
/// one. `deprecation_provenance` and `replaced_by_sku_id` were on this list
/// until slice `04`'s columns landed (`dod-lifecycle-columns`); they are
/// registered `Mechanical` now, and a test asserting their absence would be
/// asserting the opposite of what ships. Would catch a catch-all
/// arm that answered bucket-iv for anything it did not recognise: every one of
/// these would then be an operator-writable field the trigger goes on to
/// refuse with a database error.
#[test]
fn an_unregistered_column_fails_closed_rather_than_defaulting() {
    let unregistered = [
        (EntityKind::Sku, "sellable"),
        (EntityKind::Sku, "plan_tier"),
        (EntityKind::Sku, "metering_unit"),
        (EntityKind::Sku, "type"),
        (EntityKind::Sku, "name"),
        (EntityKind::Product, "sku_code"),
        (EntityKind::Product, "composition_pending"),
    ];

    for (kind, column) in unregistered {
        let refusal = classify(kind, column)
            .expect_err("an unregistered column must be refused, never classified");
        assert_eq!(refusal.code(), "ILLEGAL_FIELD_MUTATION");
        assert!(
            matches!(refusal, DomainError::IllegalFieldMutation(ref reason) if reason.contains(column) && reason.contains(kind.as_str())),
            "the refusal names the entity and the column: {refusal}",
        );
    }
}

/// The refusal is not a class, and no public entry point turns it into one.
///
/// [`classify`] is the only way in, its `Ok` arm is the only source of a
/// [`FieldClass`], and [`FieldClass::bucket`] is the only way from a class to
/// a [`FieldBucket`]. So the composition below is the whole of the public path
/// from a column name to a bucket, and on a miss it is `None` at the first
/// step. There is no `Default` on either type and no `From<DomainError>` for
/// either, so `unwrap_or` cannot be spelled against them.
///
/// The near-miss spellings are the point: a registry that trimmed or
/// lower-cased its key would answer a bucket for a field name the physical
/// layer does not have.
#[test]
fn a_miss_yields_no_bucket_through_the_public_path() {
    let misses = [
        (EntityKind::Product, "Name"),
        (EntityKind::Product, " name"),
        (EntityKind::Product, "name "),
        (EntityKind::Product, "NAME_NORMALIZED"),
        (EntityKind::Product, ""),
        (EntityKind::Sku, "products_sku.sku_code"),
        (EntityKind::Sku, "skuCode"),
    ];

    for (kind, column) in misses {
        let bucket = classify(kind, column).ok().and_then(FieldClass::bucket);
        assert_eq!(
            bucket,
            None,
            "{} {column:?} yields no bucket",
            kind.as_str()
        );
    }
}

/// §4.1: `cloned_from` is *"stricter than bucket-i — writable only in the
/// creating statement and never again"*. The class was built waiting for the
/// column, and this case asserted zero carriers until 2026-09-01, its own doc
/// saying slice 11's registration "makes this case red and forces the count
/// to be restated deliberately". P-D-76 landed the pair, so this is that
/// deliberate restatement: both entities carry exactly the two create-only
/// columns, and `classify` routes them to the class rather than to the
/// fail-closed miss.
#[test]
fn the_create_only_class_carries_exactly_the_cloned_from_pair() {
    for kind in BOTH_KINDS {
        assert_eq!(
            count_of(kind, FieldClass::CreateOnly),
            2,
            "{}: P-D-76's pair is the class's whole population",
            kind.as_str(),
        );
        for column in ["cloned_from", "cloned_from_version"] {
            let class = classify(kind, column)
                .unwrap_or_else(|_| panic!("{column} carries the registry tag now"));
            assert_eq!(
                class,
                FieldClass::CreateOnly,
                "{}: {column} is create-only, not a bucket",
                kind.as_str(),
            );
            assert_eq!(
                class.bucket(),
                None,
                "create-only routes to no bucket: the refusal is its own rule, \
                 not a bucket door's",
            );
        }
    }
}

/// `inst-fd-bucket-tags` names four buckets; today's columns populate two of
/// them. Buckets ii and iv are encoded and empty, and a member appearing in
/// either without a decision behind it fails here.
///
/// Bucket ii is slice 07's correctable set and bucket iv the descriptive
/// catch-all `fr-field-mutability-matrix` words as *"other descriptive
/// fields"*; §4.1 assigns no Foundation column to either.
#[test]
fn buckets_ii_and_iv_have_no_members_today() {
    for kind in BOTH_KINDS {
        assert_eq!(
            count_of(kind, FieldClass::Bucket(FieldBucket::Correctable)),
            0,
            "{}: bucket-ii columns arrive with slice 07",
            kind.as_str(),
        );
        assert_eq!(
            count_of(kind, FieldClass::Bucket(FieldBucket::Descriptive)),
            0,
            "{}: no Foundation column is bucket-iv",
            kind.as_str(),
        );
    }
}

/// The class counts, pinned per entity, so a column added to a table without a
/// tag - or tagged into the wrong class - is loud rather than silently
/// fail-closed at some later door.
///
/// The totals are derived from the registry and compared against the physical
/// table width in the agreement case below, so these are the only numbers that
/// have to be restated when a slice adds a column.
#[test]
fn the_class_counts_are_pinned_per_entity() {
    let product_counts = [
        (FieldClass::Bucket(FieldBucket::Structural), 2),
        (FieldClass::Bucket(FieldBucket::MaterialMutable), 4),
        (FieldClass::CreateOnly, 2),
        (FieldClass::Outside(OutsideTheScheme::Mechanical), 5),
        (FieldClass::Outside(OutsideTheScheme::RowIdentity), 4),
    ];
    for (class, expected) in product_counts {
        assert_eq!(count_of(EntityKind::Product, class), expected);
    }
    assert_eq!(columns(EntityKind::Product).len(), 17);

    let sku_counts = [
        (FieldClass::Bucket(FieldBucket::Structural), 2),
        (FieldClass::Bucket(FieldBucket::MaterialMutable), 2),
        (FieldClass::CreateOnly, 2),
        (FieldClass::Outside(OutsideTheScheme::Mechanical), 7),
        (FieldClass::Outside(OutsideTheScheme::RowIdentity), 4),
    ];
    for (class, expected) in sku_counts {
        assert_eq!(count_of(EntityKind::Sku, class), expected);
    }
    assert_eq!(columns(EntityKind::Sku).len(), 17);
}

/// No column is named twice in one entity's registry.
///
/// [`classify`] answers with the first match, so a duplicated name would give
/// one of two classes by table order - a routing decision made by accident.
#[test]
fn an_entitys_registry_names_each_column_once() {
    for kind in BOTH_KINDS {
        let mut names: Vec<&'static str> = columns(kind)
            .iter()
            .map(|tag: &ColumnTag| tag.column)
            .collect();
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(
            names.len(),
            total,
            "{} names each column once",
            kind.as_str()
        );
    }
}

/// §5's agreement test, in the half this slice can measure: the registry and
/// the physical tables name **the same columns**.
///
/// Both directions matter and they catch different faults. A physical column
/// the registry does not name is P-D-50's own case - the column added without
/// a tag, which the door would then refuse at runtime. A registry row naming
/// no physical column is the opposite drift: a tag whose column was renamed or
/// dropped, which routes a field the table does not have.
///
/// The trigger-whitelist half of §5's test - the same columns *in the same
/// classes*, with iii and iv asserted as one combined class - reads the
/// migration SQL and is
/// `crate::infra::storage::migrations_tests::bucket_agreement_tests`, which
/// this slice built alongside this one.
#[test]
fn the_registry_and_the_physical_tables_name_the_same_columns() {
    let cases = [
        (EntityKind::Product, physical_product_columns()),
        (EntityKind::Sku, physical_sku_columns()),
    ];

    for (kind, physical) in cases {
        for column in &physical {
            classify(kind, column).unwrap_or_else(|_| {
                panic!(
                    "{} column {column} exists physically and carries no registry tag",
                    kind.as_str(),
                )
            });
        }
        for tag in columns(kind) {
            assert!(
                physical.contains(&tag.column),
                "{} registry names {}, which the table does not have",
                kind.as_str(),
                tag.column,
            );
        }
        assert_eq!(
            columns(kind).len(),
            physical.len(),
            "{} registry and table are the same width",
            kind.as_str(),
        );
    }
}

/// The tags are the design's own roman numerals, so a reader comparing the
/// registry against §4.1 or `inst-fd-bucket-tags` is comparing like with like.
#[test]
fn each_bucket_carries_the_designs_own_tag() {
    assert_eq!(FieldBucket::Structural.tag(), "i");
    assert_eq!(FieldBucket::Correctable.tag(), "ii");
    assert_eq!(FieldBucket::MaterialMutable.tag(), "iii");
    assert_eq!(FieldBucket::Descriptive.tag(), "iv");
}
