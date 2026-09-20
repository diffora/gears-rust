//! The plan-SKU binding, probed on the shapes that reach publish without it.

use uuid::Uuid;

use super::{ROLE_BUNDLE, ROLE_OFFER, validate_plan_sku, validate_plan_sku_at_create};
use crate::domain::ports::CatalogSku;
use crate::domain::registry_view::SkuIndex;
use crate::domain::scope_key::SkuId;

fn only_sku(role: &str, sellable: bool, status: &str, deprecated: bool) -> (SkuId, SkuIndex) {
    let id = Uuid::from_u128(1);
    (
        SkuId::new(id),
        SkuIndex::from_listing(vec![CatalogSku {
            sku_id: id,
            sku_code: "TEST-PLAN".to_owned(),
            name: "Test plan".to_owned(),
            metering_unit: None,
            status: status.to_owned(),
            plan_tier: None,
            sku_type: role.to_owned(),
            sellable,
            usage_type_ref: None,
            deprecated,
        }]),
    )
}

fn codes(report: &crate::domain::validation::ValidationReport) -> Vec<&str> {
    report
        .violations
        .iter()
        .map(|violation| violation.code.as_str())
        .collect()
}

/// The role decides, and the sale flag is not consulted.
///
/// Both rows of the `offer` pair must be admitted. An offer closed for new
/// sales is a plan an operator is very often in the middle of reworking, and a
/// rule that refused to author against it would make closing sales and
/// maintaining the catalogue mutually exclusive.
#[test]
fn plan_role_is_independent_of_sale_permission() {
    for (role, sellable, expected) in [
        ("offer", false, None),
        ("offer", true, None),
        ("component", false, Some("PLAN_SKU_TYPE_INVALID")),
        ("component", true, Some("PLAN_SKU_TYPE_INVALID")),
        ("bundle", false, Some("PLAN_SKU_TYPE_INVALID")),
        ("bundle", true, Some("PLAN_SKU_TYPE_INVALID")),
    ] {
        let (id, index) = only_sku(role, sellable, "published", false);
        let report = validate_plan_sku(id, &index, ROLE_OFFER, true);
        assert_eq!(
            codes(&report),
            expected.into_iter().collect::<Vec<_>>(),
            "{role}/{sellable} in an ordinary plan context"
        );
    }
}

/// The context decides which role is right, and the context is the composition
/// the caller can see — never a field the author wrote.
#[test]
fn a_composition_context_wants_a_bundle_and_an_ordinary_one_wants_an_offer() {
    let (bundle_id, bundle_index) = only_sku("bundle", true, "published", false);
    assert!(
        codes(&validate_plan_sku(
            bundle_id,
            &bundle_index,
            ROLE_BUNDLE,
            true
        ))
        .is_empty(),
        "a bundle SKU under a composition is the admitted pairing"
    );
    let (offer_id, offer_index) = only_sku("offer", true, "published", false);
    assert_eq!(
        codes(&validate_plan_sku(
            offer_id,
            &offer_index,
            ROLE_BUNDLE,
            true
        )),
        vec!["PLAN_SKU_TYPE_INVALID"],
        "an offer cannot own a composition: attaching one to an offer-bound \
         draft is refused at the attach, and nothing is written"
    );
    assert_eq!(
        codes(&validate_plan_sku(
            bundle_id,
            &bundle_index,
            ROLE_OFFER,
            true
        )),
        vec!["PLAN_SKU_TYPE_INVALID"],
        "a bundle-bound draft may sit without a composition, but an ordinary \
         publish of one is refused"
    );
}

/// A SKU the registry cannot serve is reported once, as absent, and nothing
/// else is said about it.
#[test]
fn an_unreadable_sku_is_one_violation_and_not_a_role_complaint() {
    let (_, index) = only_sku("offer", true, "published", false);
    let absent = SkuId::new(Uuid::from_u128(0xdead));
    assert_eq!(
        codes(&validate_plan_sku(absent, &index, ROLE_OFFER, true)),
        vec!["SKU_NOT_PUBLISHED"]
    );

    let (draft_id, draft_index) = only_sku("component", true, "draft", false);
    assert_eq!(
        codes(&validate_plan_sku(draft_id, &draft_index, ROLE_OFFER, true)),
        vec!["SKU_NOT_PUBLISHED"],
        "an unpublished SKU is not additionally scolded for its role: the \
         author would be sent to correct a value that is not the problem"
    );
}

/// D-370 one level up: deprecation refuses the **introduction**, not the
/// binding that already exists.
#[test]
fn a_deprecated_sku_may_not_be_newly_bound_but_an_existing_binding_holds() {
    let (id, index) = only_sku("offer", true, "published", true);
    assert_eq!(
        codes(&validate_plan_sku(id, &index, ROLE_OFFER, true)),
        vec!["PLAN_SKU_DEPRECATED"]
    );
    assert!(
        codes(&validate_plan_sku(id, &index, ROLE_OFFER, false)).is_empty(),
        "a plan already published against this SKU keeps its binding"
    );
}

/// Two faults are two violations: a wrong role on a newly bound deprecated SKU
/// reports both, because fixing either alone leaves the plan unpublishable.
#[test]
fn a_wrong_role_and_a_deprecated_introduction_are_both_reported() {
    let (id, index) = only_sku("component", true, "published", true);
    assert_eq!(
        codes(&validate_plan_sku(id, &index, ROLE_OFFER, true)),
        vec!["PLAN_SKU_TYPE_INVALID", "PLAN_SKU_DEPRECATED"]
    );
}

/// Creation admits either plan-owning role and refuses the constituent.
///
/// The draft comes before the composition, so a create cannot know which of the
/// two it is staging. What it can know is that a `component` will never become
/// either, and refusing it here rather than at publish saves the author from
/// building a phase chain on a binding that was never going to hold.
#[test]
fn creation_admits_an_offer_or_a_bundle_and_refuses_a_component() {
    for role in ["offer", "bundle"] {
        let (id, index) = only_sku(role, true, "published", false);
        assert!(
            codes(&validate_plan_sku_at_create(id, &index, true)).is_empty(),
            "{role} may be staged as a plan's own SKU"
        );
    }
    let (id, index) = only_sku("component", true, "published", false);
    assert_eq!(
        codes(&validate_plan_sku_at_create(id, &index, true)),
        vec!["PLAN_SKU_TYPE_INVALID"]
    );

    // A fault that is not the role survives the two readings unchanged.
    let (absent_id, absent_index) = only_sku("offer", true, "draft", false);
    assert_eq!(
        codes(&validate_plan_sku_at_create(absent_id, &absent_index, true)),
        vec!["SKU_NOT_PUBLISHED"]
    );
}
