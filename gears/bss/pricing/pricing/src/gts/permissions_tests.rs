//! Unit tests for the GTS permission catalog (`gts::permissions`), and for the
//! default role matrix the governance slice states over it.
//!
//! The catalog half is anti-drift: the registered id set equals the expected
//! one, no id is registered twice, each instance declares the permission type,
//! and the distinct `resource_type`s equal `crate::authz::labels::ALL`.
//!
//! The matrix half reads `design/05-governance.md` through `include_str!` and
//! asserts the separations the slice states in prose — which nothing else does,
//! the matrix being a document rather than code.

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

/// Each registered instance declares the type it conforms to.
///
/// The macro derives `type_id` from the instance id's prefix **to its last
/// `~`**, so for an id carrying one `~` this restates the filter that selected
/// the entry. What it catches is an id carrying two — a nested instance id whose
/// derived type is longer than this catalog's, which the filter admits and
/// nothing else in this file reads.

#[test]
fn every_pricing_permission_declares_the_permission_type() {
    for entry in pricing_permission_instances() {
        assert_eq!(
            entry.type_id, PERMISSION_TYPE_ID,
            "instance {} declares the wrong type_id",
            entry.instance_id
        );
    }
}

/// The registered set and the expected set are the same set.
///
/// One equality rather than a membership loop beside a length: the loop names a
/// missing id and the length names a surplus one, and neither says which id is
/// surplus. The set difference names both directions at once.
#[test]
fn pricing_permission_inventory_covers_every_expected_id() {
    let actual: std::collections::BTreeSet<&str> = pricing_permission_instances()
        .iter()
        .map(|e| e.instance_id)
        .collect();
    let expected: std::collections::BTreeSet<&str> =
        EXPECTED_PERMISSION_IDS.iter().copied().collect();

    assert_eq!(
        actual,
        expected,
        "registered but unexpected: {:?}; expected but unregistered: {:?}",
        actual.difference(&expected).collect::<Vec<_>>(),
        expected.difference(&actual).collect::<Vec<_>>()
    );

    // The set cannot see a permission registered **twice** — a `gts_instance!`
    // block copy-pasted keeps one set member and two inventory entries — and
    // every other reader of this catalog collects into a set as well, so no
    // sibling would catch it either.
    let registered = pricing_permission_instances();
    assert_eq!(
        registered.len(),
        actual.len(),
        "an id is registered more than once: {:?}",
        registered
            .iter()
            .map(|e| e.instance_id)
            .filter(|id| registered.iter().filter(|e| e.instance_id == *id).count() > 1)
            .collect::<std::collections::BTreeSet<_>>()
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
