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
//! **Each slice's own rows, as its doors arrive.** `design/05-governance.md`
//! §3.2's catalog is **twenty-four** rows (it was twenty-three until
//! **P-D-61** added `bulk × read` and **P-D-67** the freeze routes), each
//! carrying the slice that owns it, and `dod-rbac-catalog` obliges this
//! feature to *"extend rather than replace"* what `01-foundation` shipped.
//! So this file grows one slice at a time: `01`'s `product`/`sku` triples,
//! then `06`'s catalog-version actions, `09`'s bulk pair and `07`'s reference
//! pairs, and now **`05`'s own four rows** — `approval × submit|read|decide`,
//! `materiality_policy × write`, `breakglass × elevate`,
//! `audit × read|export`.
//!
//! The rows owned by `02`, `03` and `04` (`category`,
//! `attribute_definition`, `recognized_set`, `plan_tier`,
//! `scheduled_transition`, `metadata`) are **deliberately absent**: they
//! belong to the slices that build those doors. `10`'s three — `erasure × execute`,
//! `compliance × export`, `pii_allowlist × write` — arrived with
//! `dod-retention-authz`, that feature's own `DoD` declaring them. See `crate::authz`'s module doc for why `discard`
//! is not a permission of its own either.
//!
//! **A declared pair with no route is intentional here, and is the point.**
//! Of governance's four rows only `approval × read` names a door in §3.2;
//! `× submit`, `× decide`, `materiality_policy × write` and
//! `breakglass × elevate` are among the nine routeless rows §6 records. The
//! `DoD` is explicit that it *"obliges the catalog, not the routes"*, and §6's
//! reason is the argument for declaring them anyway: *"an authorization
//! surface nobody can enumerate is one nobody can review"*. Declared, they
//! are countable; withheld, they are invisible until a door invents its own
//! pair.
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

// -- bulk -- the batch plane ---------------------------------------------------

gts_instance! {
    AuthzPermissionV1 {
        id: gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.bulk_execute.v1"),
        resource_type: labels::BULK.to_owned(),
        action: actions::EXECUTE.to_owned(),
        display_name: "Run a bulk import or promotion batch".to_owned(),
    }
}
gts_instance! {
    AuthzPermissionV1 {
        id: gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.bulk_read.v1"),
        resource_type: labels::BULK.to_owned(),
        action: actions::READ.to_owned(),
        display_name: "Read a batch and its row ledger".to_owned(),
    }
}

// -- reference signal -- the producers' plane ----------------------------------

gts_instance! {
    AuthzPermissionV1 {
        id: gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.reference_signal_post.v1"),
        resource_type: labels::REFERENCE_SIGNAL.to_owned(),
        action: actions::POST.to_owned(),
        display_name: "Post a reference watermark".to_owned(),
    }
}
gts_instance! {
    AuthzPermissionV1 {
        id: gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.reference_producer_write.v1"),
        resource_type: labels::REFERENCE_PRODUCER.to_owned(),
        action: actions::WRITE.to_owned(),
        display_name: "Register or retire a reference producer".to_owned(),
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

// -- governance -- the ceremony plane (`design/05` §3.2, the rows owned by 05)

gts_instance! {
    AuthzPermissionV1 {
        id: gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.approval_submit.v1"),
        resource_type: labels::APPROVAL.to_owned(),
        action: actions::SUBMIT.to_owned(),
        display_name: "Submit a change set for approval".to_owned(),
    }
}
gts_instance! {
    AuthzPermissionV1 {
        id: gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.approval_read.v1"),
        resource_type: labels::APPROVAL.to_owned(),
        action: actions::READ.to_owned(),
        display_name: "Read the pending-approval queue".to_owned(),
    }
}
gts_instance! {
    AuthzPermissionV1 {
        id: gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.approval_decide.v1"),
        resource_type: labels::APPROVAL.to_owned(),
        action: actions::DECIDE.to_owned(),
        display_name: "Approve or reject a submitted change set".to_owned(),
    }
}
gts_instance! {
    AuthzPermissionV1 {
        id: gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.materiality_policy_write.v1"),
        resource_type: labels::MATERIALITY_POLICY.to_owned(),
        action: actions::WRITE.to_owned(),
        display_name: "Change the materiality policy and its approver count".to_owned(),
    }
}
gts_instance! {
    AuthzPermissionV1 {
        id: gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.breakglass_elevate.v1"),
        resource_type: labels::BREAKGLASS.to_owned(),
        action: actions::ELEVATE.to_owned(),
        display_name: "Open a break-glass elevation session".to_owned(),
    }
}
gts_instance! {
    AuthzPermissionV1 {
        id: gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.bulk_lifecycle_execute.v1"),
        resource_type: labels::BULK_LIFECYCLE.to_owned(),
        action: actions::EXECUTE.to_owned(),
        display_name: "Run a bulk lifecycle batch".to_owned(),
    }
}
// -- retention & erasure -- `10`'s three grants (`dod-retention-authz`)

gts_instance! {
    AuthzPermissionV1 {
        id: gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.erasure_execute.v1"),
        resource_type: labels::ERASURE.to_owned(),
        action: actions::EXECUTE.to_owned(),
        display_name: "Execute an erasure request".to_owned(),
    }
}
gts_instance! {
    AuthzPermissionV1 {
        id: gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.compliance_export.v1"),
        resource_type: labels::COMPLIANCE.to_owned(),
        action: actions::EXPORT.to_owned(),
        display_name: "Export the identity map for a compliance request".to_owned(),
    }
}
gts_instance! {
    AuthzPermissionV1 {
        id: gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.pii_allowlist_write.v1"),
        resource_type: labels::PII_ALLOWLIST.to_owned(),
        action: actions::WRITE.to_owned(),
        display_name: "Change the PII allow-list".to_owned(),
    }
}
gts_instance! {
    AuthzPermissionV1 {
        id: gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.audit_read.v1"),
        resource_type: labels::AUDIT.to_owned(),
        action: actions::READ.to_owned(),
        display_name: "Read the registry's audit plane".to_owned(),
    }
}
gts_instance! {
    AuthzPermissionV1 {
        id: gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.audit_export.v1"),
        resource_type: labels::AUDIT.to_owned(),
        action: actions::EXPORT.to_owned(),
        display_name: "Export audit content out of the gear".to_owned(),
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
        gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.bulk_execute.v1"),
        gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.bulk_read.v1"),
        gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.reference_signal_post.v1"),
        gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.reference_producer_write.v1"),
        gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.approval_submit.v1"),
        gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.approval_read.v1"),
        gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.approval_decide.v1"),
        gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.materiality_policy_write.v1"),
        gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.breakglass_elevate.v1"),
        gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.audit_read.v1"),
        gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.audit_export.v1"),
        gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.bulk_lifecycle_execute.v1"),
        gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.erasure_execute.v1"),
        gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.compliance_export.v1"),
        gts_id!("cf.toolkit.authz.permission.v1~cf.bss.products.pii_allowlist_write.v1"),
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
            crate::authz::actions::EXECUTE,
            crate::authz::actions::POST,
            crate::authz::actions::SUBMIT,
            crate::authz::actions::DECIDE,
            crate::authz::actions::ELEVATE,
            crate::authz::actions::EXPORT,
        ];
        // `10`'s three grants reuse EXECUTE, EXPORT and WRITE on their own
        // resources, so the action vocabulary does not grow with them — the
        // resource is the discriminator.
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
