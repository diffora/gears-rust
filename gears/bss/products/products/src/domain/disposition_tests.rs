//! The suggestion rules are P-D-62's contract verbatim, so the tests assert
//! the exact strings: `{name}-copy-N` for a live lineage, `-revived` for a
//! retired source's first revival, `-revived-N` for a later one — the arm
//! whose N==1 special case would ship silently off-by-one without a case
//! naming it.

use uuid::Uuid;

use super::{
    CloneContent, ProductCloneSource, SkuCloneSource, suggested_product_code,
    suggested_product_name, suggested_sku_code,
};

/// A live-lineage source with `name` and an optional code.
fn live_source(name: &str, code: Option<&str>) -> ProductCloneSource {
    ProductCloneSource {
        brand_id: Uuid::new_v4(),
        name: name.to_owned(),
        product_code: code.map(str::to_owned),
        region_scope: "global".to_owned(),
        brand_scope: "all".to_owned(),
        read_at_version: Some(1),
        retired: false,
        content: CloneContent::default(),
    }
}

/// The same source, retired — the flavored arm's operand.
fn retired_source(name: &str) -> ProductCloneSource {
    ProductCloneSource {
        retired: true,
        ..live_source(name, None)
    }
}

#[test]
fn a_live_lineage_suggests_copy_n() {
    let source = live_source("Widget", None);
    assert_eq!(suggested_product_name(&source, 1), "Widget-copy-1");
    assert_eq!(suggested_product_name(&source, 2), "Widget-copy-2");
}

#[test]
fn a_retired_source_first_revival_is_revived_unnumbered() {
    // inst-cn-rename: the FIRST revival carries no number at all.
    let source = retired_source("Widget");
    assert_eq!(suggested_product_name(&source, 1), "Widget-revived");
}

#[test]
fn a_retired_source_later_revivals_are_revived_n() {
    // The same first-free rule over the flavored family: a second revival
    // of one lineage numbers itself, and the number is the attempt's.
    let source = retired_source("Widget");
    assert_eq!(suggested_product_name(&source, 2), "Widget-revived-2");
    assert_eq!(suggested_product_name(&source, 100), "Widget-revived-100");
}

#[test]
fn the_code_suggestion_is_unflavored_and_absent_where_the_source_has_none() {
    // The -revived flavor is the RENAME rule's; the code stays -copy-N even
    // for a retired source, and a source with no code suggests none.
    let with_code = ProductCloneSource {
        retired: true,
        ..live_source("Widget", Some("W-1"))
    };
    assert_eq!(
        suggested_product_code(&with_code, 1),
        Some("W-1-copy-1".to_owned())
    );
    assert_eq!(suggested_product_code(&retired_source("Widget"), 1), None);
}

#[test]
fn the_sku_code_suggestion_has_no_flavored_arm() {
    let source = SkuCloneSource {
        sku_type: Some("product".to_owned()),
        sellable: true,
        plan_tier: Some("standard".to_owned()),
        tax_category_ref: None,
        gl_code_ref: None,
        product_id: Uuid::new_v4(),
        sku_code: "SKU-1".to_owned(),
        region_scope: "global".to_owned(),
        brand_scope: "all".to_owned(),
        read_at_version: Some(1),
        metering_unit: None,
        usage_type_ref: None,
        content: CloneContent::default(),
    };
    assert_eq!(suggested_sku_code(&source, 3), "SKU-1-copy-3");
}
