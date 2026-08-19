//! Unit tests for the GTS permission catalog (`gts::permissions`): every
//! `(resource_type, action)` instance is registered in inventory, the
//! expected-id set matches exactly, and the catalog's distinct `resource_type`s
//! equal `crate::authz::labels::ALL` (anti-drift).

use toolkit_gts::{InventoryInstance, gts_id};

const PERMISSION_TYPE_ID: &str = gts_id!("cf.toolkit.authz.permission.v1~");
const INSTANCE_SUFFIX_PREFIX: &str = "cf.bss.pricing.";

/// Every pricing permission instance id — one per `(resource_type, action)`
/// pair the catalog surfaces enforce, per `design/05-governance.md`
/// `cpt-cf-bss-pricing-algo-authz-catalog`.
const EXPECTED_PERMISSION_IDS: &[&str] = &[
    gts_id!("cf.toolkit.authz.permission.v1~cf.bss.pricing.plan_write.v1"),
    gts_id!("cf.toolkit.authz.permission.v1~cf.bss.pricing.plan_publish.v1"),
    gts_id!("cf.toolkit.authz.permission.v1~cf.bss.pricing.plan_retire.v1"),
    gts_id!("cf.toolkit.authz.permission.v1~cf.bss.pricing.plan_migrate.v1"),
    gts_id!("cf.toolkit.authz.permission.v1~cf.bss.pricing.plan_read.v1"),
    gts_id!("cf.toolkit.authz.permission.v1~cf.bss.pricing.plan_preview.v1"),
    gts_id!("cf.toolkit.authz.permission.v1~cf.bss.pricing.bundle_write.v1"),
    gts_id!("cf.toolkit.authz.permission.v1~cf.bss.pricing.bundle_read.v1"),
    gts_id!("cf.toolkit.authz.permission.v1~cf.bss.pricing.price_overlay_write.v1"),
    gts_id!("cf.toolkit.authz.permission.v1~cf.bss.pricing.price_overlay_read.v1"),
    gts_id!("cf.toolkit.authz.permission.v1~cf.bss.pricing.customer_group_write.v1"),
    gts_id!("cf.toolkit.authz.permission.v1~cf.bss.pricing.customer_group_read.v1"),
    gts_id!("cf.toolkit.authz.permission.v1~cf.bss.pricing.approval_approve.v1"),
    gts_id!("cf.toolkit.authz.permission.v1~cf.bss.pricing.approval_read.v1"),
    gts_id!("cf.toolkit.authz.permission.v1~cf.bss.pricing.approval_policy_write.v1"),
    gts_id!("cf.toolkit.authz.permission.v1~cf.bss.pricing.approval_policy_read.v1"),
    gts_id!("cf.toolkit.authz.permission.v1~cf.bss.pricing.config_write.v1"),
    gts_id!("cf.toolkit.authz.permission.v1~cf.bss.pricing.config_read.v1"),
    gts_id!("cf.toolkit.authz.permission.v1~cf.bss.pricing.audit_read.v1"),
    gts_id!("cf.toolkit.authz.permission.v1~cf.bss.pricing.audit_export.v1"),
];

fn pricing_permission_instances() -> Vec<&'static InventoryInstance> {
    toolkit_gts::inventory::iter::<InventoryInstance>
        .into_iter()
        .filter(|e| {
            e.instance_id.starts_with(PERMISSION_TYPE_ID)
                && e.instance_id[PERMISSION_TYPE_ID.len()..].starts_with(INSTANCE_SUFFIX_PREFIX)
        })
        .collect()
}

#[test]
fn all_pricing_permissions_are_registered_in_inventory() {
    let entries = pricing_permission_instances();

    assert_eq!(
        entries.len(),
        EXPECTED_PERMISSION_IDS.len(),
        "expected {} pricing permission instances; found {}: {:?}",
        EXPECTED_PERMISSION_IDS.len(),
        entries.len(),
        entries.iter().map(|e| e.instance_id).collect::<Vec<_>>()
    );
    for entry in &entries {
        assert_eq!(
            entry.type_id, PERMISSION_TYPE_ID,
            "instance {} derived wrong type_id",
            entry.instance_id
        );
    }
}

#[test]
fn pricing_permission_inventory_covers_every_expected_id() {
    let actual: std::collections::BTreeSet<&str> = pricing_permission_instances()
        .iter()
        .map(|e| e.instance_id)
        .collect();

    for expected in EXPECTED_PERMISSION_IDS {
        assert!(
            actual.contains(expected),
            "missing expected permission id: {expected}; got {actual:?}"
        );
    }
    assert_eq!(
        actual.len(),
        EXPECTED_PERMISSION_IDS.len(),
        "inventory contains pricing permission ids not in the expected set"
    );
}

/// Anti-drift: the distinct `resource_type`s this catalog grants MUST equal
/// `crate::authz::labels::ALL` — the set the gear registers stub type-schemas
/// for so RBAC role-definitions can target them. Add a permission with a new
/// label (or a label to `ALL`) without the other and this fails.
#[test]
fn catalog_resource_types_match_authz_labels_all() {
    let catalog_types: std::collections::BTreeSet<String> = pricing_permission_instances()
        .iter()
        .map(|e| {
            (e.payload_fn)()["resource_type"]
                .as_str()
                .expect("AuthzPermissionV1 payload carries a resource_type string")
                .to_owned()
        })
        .collect();
    let labels_all: std::collections::BTreeSet<String> = crate::authz::labels::ALL
        .iter()
        .map(|s| (*s).to_owned())
        .collect();

    assert_eq!(
        catalog_types, labels_all,
        "permission-catalog resource_types must equal crate::authz::labels::ALL"
    );
}

/// The one action the catalog grants on `plan` that no other resource grants,
/// and the one no role may hold beside `plan × publish` by default. Both are
/// stated in the governance slice's role matrix; this pins that the actions
/// exist as separate permissions at all, so a role definition can express the
/// separation.
#[test]
fn publish_and_approve_are_separately_grantable() {
    let ids: std::collections::BTreeSet<&str> = pricing_permission_instances()
        .iter()
        .map(|e| e.instance_id)
        .collect();

    assert!(ids.contains(gts_id!(
        "cf.toolkit.authz.permission.v1~cf.bss.pricing.plan_publish.v1"
    )));
    assert!(ids.contains(gts_id!(
        "cf.toolkit.authz.permission.v1~cf.bss.pricing.approval_approve.v1"
    )));
}
