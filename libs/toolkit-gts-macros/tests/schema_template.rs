//! Generation-only wrappers work with a renamed dependency and no crate collector.

use gts::GtsSchema;

#[derive(serde::Serialize, schemars::JsonSchema, gts_kit::GtsTraitsSchema)]
pub struct TemplateTraits {
    pub enabled: bool,
}

#[gts_kit::gts_type_schema_template(
    dir_path = "schemas",
    type_id = gts_id!("test.cf.template.envelope.v1~"),
    description = "Configured-only envelope",
    properties = "id,payload",
    base = true,
    traits_schema = inline(TemplateTraits),
    gts_abstract = true
)]
pub struct EnvelopeV1<P> {
    pub id: gts::GtsInstanceId,
    pub payload: P,
}

#[gts_kit::gts_type_schema_template(
    dir_path = "schemas",
    type_id = gts_id!("test.cf.template.envelope.v1~test.cf.template.leaf.v1~"),
    description = "Configured-only leaf",
    properties = "name",
    base = EnvelopeV1,
    traits = TemplateTraits { enabled: true }
)]
pub struct LeafV1 {
    pub name: String,
}

#[test]
fn template_forwards_schema_and_trait_generation() {
    let schema: serde_json::Value =
        serde_json::from_str(&LeafV1::gts_schema_with_refs_as_string()).expect("generated schema");
    assert_eq!(
        schema["$id"],
        gts_kit::gts_uri!("test.cf.template.envelope.v1~test.cf.template.leaf.v1~")
    );
    assert_eq!(schema["x-gts-traits"], serde_json::json!({"enabled": true}));
    let base = EnvelopeV1::<()>::gts_schema_with_refs();
    assert_eq!(base["x-gts-abstract"], true);
    assert!(base["x-gts-traits-schema"].is_object());
}

#[test]
fn template_adds_no_inventory_or_declaration_provenance() {
    for id in [<EnvelopeV1<()> as GtsSchema>::TYPE_ID, LeafV1::TYPE_ID] {
        assert!(
            gts_kit::inventory::iter::<gts_kit::InventoryTypeSchema>
                .into_iter()
                .all(|entry| entry.type_id != id),
            "{id} must not be collected"
        );
        assert!(
            gts_kit::inventory::iter::<gts_kit::local::GlobalOnlyDeclaration>
                .into_iter()
                .all(|entry| entry.gts_id != id),
            "{id} must not be a global-only declaration"
        );
    }
}
