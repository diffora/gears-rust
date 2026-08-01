//! Tests for [`crate::authz`] — the label set, the descriptors, and the stub
//! type-schemas registered at init.

use super::{SUPPORTED_PROPERTIES, actions, authz_label_type_schemas, labels, resource_types};

#[test]
fn every_label_lives_outside_the_platform_resource_family() {
    // The whole point of the label choice: `gts.cf.resources.*` is auto-covered
    // by the built-in Reader / Contributor / Owner roles, and pricing data must
    // require an explicit catalog role.
    for label in labels::ALL {
        assert!(
            !label.starts_with("gts.cf.resources."),
            "{label} is inside the auto-covered platform resource family"
        );
        assert!(
            label.starts_with("gts.cf.bss.pricing."),
            "{label} is not a pricing-owned label"
        );
        assert!(label.ends_with(".v1~"), "{label} is not a type-level id");
    }
}

#[test]
fn the_label_list_has_no_duplicates() {
    let distinct: std::collections::BTreeSet<&str> = labels::ALL.iter().copied().collect();

    assert_eq!(
        distinct.len(),
        labels::ALL.len(),
        "labels::ALL repeats a label"
    );
}

#[test]
fn approval_policy_is_a_separate_resource_from_config() {
    // Segregation of duties (D-10): a config admin must not be able to weaken
    // the approval thresholds it operates under. If these ever collapse into one
    // label the two-person rule becomes self-administered.
    assert_ne!(labels::APPROVAL_POLICY, labels::CONFIG);
}

#[test]
fn audit_and_customer_group_are_their_own_resources() {
    // An auditor role must carry no read of live pricing (D-12), and per-payer
    // membership must not ride a plan grant.
    assert_ne!(labels::AUDIT, labels::PLAN);
    assert_ne!(labels::CUSTOMER_GROUP, labels::PLAN);
}

#[test]
fn descriptors_carry_their_label_and_the_supported_properties() {
    for (rt, label) in [
        (&resource_types::PLAN, labels::PLAN),
        (&resource_types::BUNDLE, labels::BUNDLE),
        (&resource_types::PRICE_OVERLAY, labels::PRICE_OVERLAY),
        (&resource_types::CUSTOMER_GROUP, labels::CUSTOMER_GROUP),
        (&resource_types::APPROVAL, labels::APPROVAL),
        (&resource_types::APPROVAL_POLICY, labels::APPROVAL_POLICY),
        (&resource_types::CONFIG, labels::CONFIG),
        (
            &resource_types::HISTORICAL_IMPORT,
            labels::HISTORICAL_IMPORT,
        ),
        (&resource_types::AUDIT, labels::AUDIT),
    ] {
        assert_eq!(rt.name(), label);
        assert_eq!(rt.supported_properties(), SUPPORTED_PROPERTIES);
    }
}

#[test]
fn the_pep_advertises_no_subtree_property() {
    // The PDP pre-expands the caller's subtree to a flat `In`; advertising a
    // subtree property here would change what the PDP compiles.
    assert_eq!(SUPPORTED_PROPERTIES.len(), 2);
    assert!(
        !SUPPORTED_PROPERTIES
            .iter()
            .any(|p| p.contains("subtree") || p.contains("group")),
        "no subtree/group property is supported"
    );
}

#[test]
fn action_names_are_distinct() {
    let all = [
        actions::WRITE,
        actions::PUBLISH,
        actions::RETIRE,
        actions::MIGRATE,
        actions::READ,
        actions::PREVIEW,
        actions::APPROVE,
        actions::EXPORT,
    ];
    let distinct: std::collections::BTreeSet<&str> = all.iter().copied().collect();

    assert_eq!(distinct.len(), all.len(), "two actions share a name");
}

#[test]
fn a_stub_schema_is_produced_for_every_label() {
    let schemas = authz_label_type_schemas();

    assert_eq!(schemas.len(), labels::ALL.len());
    for (schema, label) in schemas.iter().zip(labels::ALL) {
        assert_eq!(
            schema["$id"].as_str(),
            Some(format!("gts://{label}").as_str())
        );
        assert_eq!(schema["type"].as_str(), Some("object"));
    }
}

#[test]
fn stub_schema_generation_is_byte_stable() {
    // Registration is re-run on every boot; the registry accepts an identical
    // duplicate, so the body must not vary between runs.
    assert_eq!(
        serde_json::to_string(&authz_label_type_schemas()).expect("schemas serialize"),
        serde_json::to_string(&authz_label_type_schemas()).expect("schemas serialize"),
    );
}
