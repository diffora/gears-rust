//! Product & SKU Registry authorization permissions catalog.
//!
//! Declares every grantable permission as an [`AuthzPermissionV1`] GTS instance
//! via [`gts_instance!`]. Each invocation submits an `InventoryInstance` entry;
//! `types-registry::init()` aggregates and validates them at startup — no
//! registration code in [`crate::gear`] or anywhere else in this crate.
//!
//! `resource_type` values are the authz labels from [`crate::authz`] — the same
//! strings the service paths will pass to `PolicyEnforcer` at enforce time
//! once Phase 4 wires the authoring doors through it, so the catalog and the
//! (future) enforcement path share one source of truth.
//!
//! **Only the Foundation's two entities.** `product` and `sku`, each with
//! `read`, `write` and `publish` — the roster `design/01-foundation.md` §2
//! names on the gear's doors and `design/05-governance.md` §3.2's RBAC catalog
//! rows for those two resources. The wider governance catalog (`category`,
//! `attribute_definition`, `approval`, `audit`, `breakglass`, and the rest of
//! §3.2's twenty-three rows) belongs to the slices that build those doors and
//! is not declared here; see `crate::authz`'s module doc for why `discard` is
//! not a permission of its own either.
//!
//! Instance id layout (instance suffix needs >= 5 dot-separated tokens):
//! `gts.cf.toolkit.authz.permission.v1~cf.bss.products.<entity>_<action>.v1`.

// The expected-id string literals (here and in the drift test) trip DE0901
// (`gts_string_pattern`, which hardcodes the allowed vendor set); they are
// legitimate catalog literals. Suppress file-wide, mirroring the sibling
// pricing gear's permission catalog.
#![allow(unknown_lints)]
#![allow(de0901_gts_string_pattern)]

use toolkit_gts::{AuthzPermissionV1, gts_instance};

use crate::authz::{actions, labels};

// -- product -- the authoring data plane -------------------------------------

gts_instance! {
    AuthzPermissionV1 {
        id: gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.product_write.v1"),
        resource_type: labels::PRODUCT.to_owned(),
        action: actions::WRITE.to_owned(),
        display_name: "Author, update, clone or discard a product".to_owned(),
    }
}
gts_instance! {
    AuthzPermissionV1 {
        id: gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.product_publish.v1"),
        resource_type: labels::PRODUCT.to_owned(),
        action: actions::PUBLISH.to_owned(),
        display_name: "Publish a product".to_owned(),
    }
}
gts_instance! {
    AuthzPermissionV1 {
        id: gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.product_read.v1"),
        resource_type: labels::PRODUCT.to_owned(),
        action: actions::READ.to_owned(),
        display_name: "Read a product's head row and version history".to_owned(),
    }
}

// -- catalog version -- the demand plane ---------------------------------------

gts_instance! {
    AuthzPermissionV1 {
        id: gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.catalog_version_request.v1"),
        resource_type: labels::CATALOG_VERSION.to_owned(),
        action: actions::REQUEST.to_owned(),
        display_name: "Request a catalog-version increment".to_owned(),
    }
}

gts_instance! {
    AuthzPermissionV1 {
        id: gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.catalog_version_ack.v1"),
        resource_type: labels::CATALOG_VERSION.to_owned(),
        action: actions::ACK.to_owned(),
        display_name: "Acknowledge a catalog version's freeze".to_owned(),
    }
}
gts_instance! {
    AuthzPermissionV1 {
        id: gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.catalog_version_release.v1"),
        resource_type: labels::CATALOG_VERSION.to_owned(),
        action: actions::RELEASE.to_owned(),
        display_name: "Release a catalog version's freeze liveness".to_owned(),
    }
}
gts_instance! {
    AuthzPermissionV1 {
        id: gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.catalog_version_read.v1"),
        resource_type: labels::CATALOG_VERSION.to_owned(),
        action: actions::READ.to_owned(),
        display_name: "Resolve or diff a catalog version".to_owned(),
    }
}

// -- sku -- the authoring data plane ------------------------------------------

gts_instance! {
    AuthzPermissionV1 {
        id: gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.sku_write.v1"),
        resource_type: labels::SKU.to_owned(),
        action: actions::WRITE.to_owned(),
        display_name: "Author, update, clone or discard a SKU".to_owned(),
    }
}
gts_instance! {
    AuthzPermissionV1 {
        id: gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.sku_publish.v1"),
        resource_type: labels::SKU.to_owned(),
        action: actions::PUBLISH.to_owned(),
        display_name: "Publish a SKU".to_owned(),
    }
}
gts_instance! {
    AuthzPermissionV1 {
        id: gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.sku_read.v1"),
        resource_type: labels::SKU.to_owned(),
        action: actions::READ.to_owned(),
        display_name: "Read a SKU's head row and version history".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use toolkit_gts::{InventoryInstance, gts_id};

    const PERMISSION_TYPE_ID: &str = gts_id!("cf.toolkit.authz.permission.v1~");
    const INSTANCE_SUFFIX_PREFIX: &str = "cf.bss.products.";

    /// Every products permission instance id — one per `(resource_type,
    /// action)` pair this catalog declares.
    const EXPECTED_PERMISSION_IDS: &[&str] = &[
        gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.product_write.v1"),
        gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.product_publish.v1"),
        gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.product_read.v1"),
        gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.sku_write.v1"),
        gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.sku_publish.v1"),
        gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.sku_read.v1"),
        gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.catalog_version_request.v1"),
        gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.catalog_version_ack.v1"),
        gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.catalog_version_release.v1"),
        gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.catalog_version_read.v1"),
    ];

    fn products_permission_instances() -> Vec<&'static InventoryInstance> {
        toolkit_gts::inventory::iter::<InventoryInstance>
            .into_iter()
            .filter(|e| {
                e.instance_id.starts_with(PERMISSION_TYPE_ID)
                    && e.instance_id[PERMISSION_TYPE_ID.len()..].starts_with(INSTANCE_SUFFIX_PREFIX)
            })
            .collect()
    }

    /// Each registered instance declares the permission type it conforms to.
    #[test]
    fn every_products_permission_declares_the_permission_type() {
        for entry in products_permission_instances() {
            assert_eq!(
                entry.type_id, PERMISSION_TYPE_ID,
                "instance {} declares the wrong type_id",
                entry.instance_id
            );
        }
    }

    /// The registered set and the expected set are the same set, and no id is
    /// registered twice — a set alone cannot see a duplicate registration, so
    /// the length is checked against the raw (unfiltered) count separately.
    #[test]
    fn products_permission_inventory_covers_every_expected_id() {
        let registered = products_permission_instances();
        let actual: std::collections::BTreeSet<&str> =
            registered.iter().map(|e| e.instance_id).collect();
        let expected: std::collections::BTreeSet<&str> =
            EXPECTED_PERMISSION_IDS.iter().copied().collect();

        assert_eq!(
            actual,
            expected,
            "registered but unexpected: {:?}; expected but unregistered: {:?}",
            actual.difference(&expected).collect::<Vec<_>>(),
            expected.difference(&actual).collect::<Vec<_>>()
        );

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

    /// Anti-drift: the distinct `resource_type`s this catalog grants MUST
    /// equal `crate::authz::labels::ALL` — the set `crate::authz` registers
    /// stub type-schemas for so RBAC role-definitions can target them. Add a
    /// permission with a new label (or a label to `ALL`) without the other
    /// and this fails.
    #[test]
    fn catalog_resource_types_match_authz_labels_all() {
        let catalog_types: std::collections::BTreeSet<String> = products_permission_instances()
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

    /// Anti-drift the other direction: every declared instance's `action`
    /// string is one this module's `crate::authz::actions` actually exports —
    /// catching a permission whose action was typo'd past the constant it was
    /// meant to copy.
    #[test]
    fn catalog_actions_are_declared_action_constants() {
        let known = [
            crate::authz::actions::READ,
            crate::authz::actions::WRITE,
            crate::authz::actions::PUBLISH,
            crate::authz::actions::REQUEST,
            crate::authz::actions::ACK,
            crate::authz::actions::RELEASE,
        ];
        for entry in products_permission_instances() {
            let action = (entry.payload_fn)()["action"]
                .as_str()
                .expect("AuthzPermissionV1 payload carries an action string")
                .to_owned();
            assert!(
                known.contains(&action.as_str()),
                "instance {} declares an action not in crate::authz::actions: {action}",
                entry.instance_id
            );
        }
    }
}
