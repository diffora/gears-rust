//! Product & SKU Registry authorization: PEP resource-type labels, action
//! names and the authz-label stub type-schemas that let RBAC role-definitions
//! target this gear's authz labels.
//!
//! The catalog is normative in `design/05-governance.md` §3.2 (the RBAC
//! catalog table) and `design/01-foundation.md` §2 (the door grants); this
//! module is their executable form for the Foundation's two entities. Only
//! `product` and `sku`, and only `read`/`write`/`publish`, are declared here —
//! the roster the Foundation's own doors name. The wider governance catalog
//! (`category`, `attribute_definition`, `approval`, `audit`, `breakglass` and
//! the rest) belongs to the slices that build those doors and is not declared
//! by this one.
//!
//! **`discard` is deliberately not a label action of its own.** `01
//! §2` narrates `POST /bss-products/v1/{products|skus}/{id}/discard` under
//! `… × discard`, but `05-governance.md` §3.2's own RBAC catalog rows the same
//! door under `product × write` / `sku × write`, and the document's own
//! open-items list records the contradiction as unresolved — "does the
//! discard door get its own grant, or inherit `product|sku × write`?" — with
//! the decision owned by that slice. Minting a `discard` permission here would
//! take one side of a question the design set has not settled; `write` is
//! what the normative catalog table currently grants the door, so that is what
//! this module declares.
//!
//! **Wiring is owed and not done here.** [`authz_label_type_schemas`] is
//! declared but not yet registered anywhere: the sibling pricing gear calls
//! its equivalent from `Gear::init` so the platform's RBAC role-definition
//! validator can resolve a rule's `target_type` against these labels. This
//! gear's `init` (`crate::gear::BssProductsGear::init`) does not call this
//! function yet; wiring it in is owed to a Phase 4 slice, alongside the
//! authoring doors that will gate through [`labels`] and [`actions`] via the
//! `PolicyEnforcer` the gear already builds.

/// Authz `resource_type` label strings (the PDP-visible glob targets).
///
/// Plain `&'static str` consts so the GTS permission catalog
/// (`crate::gts::permissions`) and, once Phase 4 wires the PEP calls, the
/// enforcement path share one source of truth.
pub mod labels {
    use toolkit_gts::gts_id;

    /// Products — the authoring data plane for the `Product` entity
    /// (`read`, `write`, `publish`).
    pub const PRODUCT: &str = gts_id!("cf.bss.products.product.v1~");
    /// SKUs — the authoring data plane for the `SKU` entity (`read`, `write`,
    /// `publish`).
    pub const SKU: &str = gts_id!("cf.bss.products.sku.v1~");

    /// Every authz label this module declares, stable order. The single
    /// canonical list driving [`super::authz_label_type_schemas`]'s stub
    /// registration. MUST match the permission catalog's distinct
    /// `resource_type`s (`crate::gts::permissions`); a drift test enforces it.
    pub const ALL: &[&str] = &[PRODUCT, SKU];
}

/// PEP action names for the labels above.
pub mod actions {
    /// Read action — authoring reads of a head row and its version history
    /// (`GET /bss-products/v1/{products|skus}/{id}`,
    /// `GET /bss-products/v1/{products|skus}/{id}/versions`).
    pub const READ: &str = "read";
    /// Write action — authoring mutations: create, update, clone and discard
    /// (`05-governance.md` §3.2 rows the discard door under this action; see
    /// this module's doc for why `discard` is not declared separately).
    pub const WRITE: &str = "write";
    /// Publish action — turning an approved draft into a published version
    /// (`POST /bss-products/v1/{products|skus}/{id}/publish`).
    pub const PUBLISH: &str = "publish";
}

fn authz_type_schema_json(gts_id: &str, title: &str) -> serde_json::Value {
    serde_json::json!({
        "$id": format!("gts://{gts_id}"),
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": title,
        "type": "object",
    })
}

/// Stub type-schemas for every authz label ([`labels::ALL`]). The platform
/// RBAC role-definition validator resolves a rule's `target_type` through the
/// types-registry, so registering these lets a custom catalog role target
/// this gear's authz labels.
///
/// **Not yet registered.** See this module's doc: the sibling pricing gear
/// registers its equivalent from `Gear::init`; this gear's `init` does not
/// call this function yet, and that wiring is owed to a later slice.
#[must_use]
pub fn authz_label_type_schemas() -> Vec<serde_json::Value> {
    labels::ALL
        .iter()
        .map(|label| {
            authz_type_schema_json(
                label,
                &format!("BSS Product & SKU Registry authz label {label}"),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{actions, authz_label_type_schemas, labels};

    /// `labels::ALL` names exactly the two labels this module declares, in
    /// the order the stub-schema registration and the drift test in
    /// `crate::gts::permissions` both read it in.
    #[test]
    fn labels_all_is_product_and_sku() {
        assert_eq!(labels::ALL, [labels::PRODUCT, labels::SKU]);
    }

    /// One stub schema per label, each addressed at the label's own `$id` and
    /// shaped as a bare JSON-Schema object — the shape the platform RBAC
    /// role-definition validator resolves a `target_type` against.
    #[test]
    fn authz_label_type_schemas_covers_every_label_exactly_once() {
        let schemas = authz_label_type_schemas();
        assert_eq!(schemas.len(), labels::ALL.len());

        let ids: std::collections::BTreeSet<String> = schemas
            .iter()
            .map(|schema| {
                schema["$id"]
                    .as_str()
                    .expect("each stub schema carries a $id")
                    .to_owned()
            })
            .collect();
        let expected: std::collections::BTreeSet<String> = labels::ALL
            .iter()
            .map(|label| format!("gts://{label}"))
            .collect();
        assert_eq!(ids, expected);

        for schema in &schemas {
            assert_eq!(schema["type"], "object");
        }
    }

    /// The three action names are distinct — a copy-paste that left two
    /// consts holding the same string would let two permissions in the
    /// catalog collide on `(resource_type, action)` without either the
    /// catalog's id-distinctness test or its resource-type drift test
    /// noticing, since neither reads the action names against each other.
    #[test]
    fn action_names_are_pairwise_distinct() {
        let names = [actions::READ, actions::WRITE, actions::PUBLISH];
        let distinct: std::collections::BTreeSet<&str> = names.iter().copied().collect();
        assert_eq!(distinct.len(), names.len(), "two action consts collide");
    }
}
