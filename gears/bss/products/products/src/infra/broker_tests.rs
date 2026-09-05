//! The typed events' own guards: the three GTS constants, the two overrides
//! that carry P-D-47's ordering, and the shape a consumer deserializes.

use event_broker_sdk::TypedEvent;
use serde_json::Value;
use uuid::Uuid;

use super::{
    ATTRIBUTE_DEFINITION_SUBJECT_TYPE, ActorErased, AttributeDefinitionUpdated,
    CATEGORY_SUBJECT_TYPE, CatalogBulkOperationCompleted, CatalogEventCore, CategoryCreated,
    CategoryDeleted, CategoryDisplayUpdated, CategoryRenamed, CategoryReparented, CategoryRetired,
    METADATA_SUBJECT_TYPE, MetadataUpdated, PRODUCT_SUBJECT_TYPE, PiiAllowlistChanged,
    PlanTierUpdated, ProductCreated, ProductDeprecated, ProductDiscarded, ProductHeadSaved,
    ProductPublished, ProductRetired, ProductRetirementEffective, ProductUndeprecated,
    REFERENCE_PRODUCER_SUBJECT_TYPE, RecognizedCodeUpdated, RecognizedUnitUpdated,
    ReferenceProducerSetChanged, SKU_SUBJECT_TYPE, SOURCE, SkuCorrectionOverride, SkuCreated,
    SkuDeprecated, SkuDiscarded, SkuHeadSaved, SkuImmutableFieldCorrected, SkuPublished,
    SkuRetired, SkuRetirementEffective, SkuUndeprecated, TOPIC,
};

const TENANT: Uuid = Uuid::from_u128(0x7e_42);
const ENTITY: Uuid = Uuid::from_u128(0x_1111);
const ACTOR: Uuid = Uuid::from_u128(0x_ac70);

fn core(kind: &str) -> CatalogEventCore {
    CatalogEventCore {
        tenant_id: TENANT,
        entity_kind: kind.to_owned(),
        entity_id: ENTITY,
        internal_revision: 3,
        lifecycle_state: "draft".to_owned(),
        causation_id: None,
        actor_ref: ACTOR,
    }
}

/// The topic all eight publish onto, **transcribed**.
///
/// The constant under test is `super::TOPIC`; this is the literal it must equal.
/// An earlier revision of this file asserted `topic == TOPIC` — a constant
/// against itself — so changing `TOPIC` by one character left every case green
/// while the gear pointed at a topic no broker-side registration carries. These
/// three literals are the half of the agreement a test can freeze.
const TRANSCRIBED_TOPIC: &str = "gts.cf.core.events.topic.v1~cf.bss.products.catalog.v1";
/// [`TRANSCRIBED_TOPIC`]'s reason, for the Product subject type.
const TRANSCRIBED_PRODUCT_SUBJECT: &str =
    "gts.cf.core.events.subject.v1~cf.bss.products.product.v1";
/// [`TRANSCRIBED_TOPIC`]'s reason, for the SKU subject type.
const TRANSCRIBED_SKU_SUBJECT: &str = "gts.cf.core.events.subject.v1~cf.bss.products.sku.v1";

/// One row per event: the `events` payload-type token a door passes, the
/// `TYPE_ID` it must map to, and the `SUBJECT_TYPE` that id must carry.
///
/// Transcribed rather than read off the types, for `events_tests`' reason: a
/// list built from the code under test could only prove the code equals itself.
const THE_EIGHT: &[(&str, &str, &str)] = &[
    (
        "ProductCreated",
        "gts.cf.core.events.event_type.v1~cf.bss.products.product_created.v1",
        TRANSCRIBED_PRODUCT_SUBJECT,
    ),
    (
        "SkuCreated",
        "gts.cf.core.events.event_type.v1~cf.bss.products.sku_created.v1",
        TRANSCRIBED_SKU_SUBJECT,
    ),
    (
        "ProductHeadSaved",
        "gts.cf.core.events.event_type.v1~cf.bss.products.product_head_saved.v1",
        TRANSCRIBED_PRODUCT_SUBJECT,
    ),
    (
        "SkuHeadSaved",
        "gts.cf.core.events.event_type.v1~cf.bss.products.sku_head_saved.v1",
        TRANSCRIBED_SKU_SUBJECT,
    ),
    (
        "ProductPublished",
        "gts.cf.core.events.event_type.v1~cf.bss.products.product_published.v1",
        TRANSCRIBED_PRODUCT_SUBJECT,
    ),
    (
        "SkuPublished",
        "gts.cf.core.events.event_type.v1~cf.bss.products.sku_published.v1",
        TRANSCRIBED_SKU_SUBJECT,
    ),
    (
        "ProductDiscarded",
        "gts.cf.core.events.event_type.v1~cf.bss.products.product_discarded.v1",
        TRANSCRIBED_PRODUCT_SUBJECT,
    ),
    (
        "SkuDiscarded",
        "gts.cf.core.events.event_type.v1~cf.bss.products.sku_discarded.v1",
        TRANSCRIBED_SKU_SUBJECT,
    ),
];

/// `04-lifecycle`'s announced pair — the names from its Events roster, the
/// ids and subject types derived by this module's own naming rule — a second
/// list beside [`THE_EIGHT`], for `events_tests`' reason exactly.
///
/// `01` §4.5 leaves the `published → deprecated` edge eventless and records
/// that 04 announces it, so folding these two into `THE_EIGHT` would stop
/// that list being a transcription of one sentence: a Foundation event
/// dropped from §4.5 could then be replaced by a lifecycle one with the count
/// unchanged.
const THE_LIFECYCLE_PAIR: &[(&str, &str, &str)] = &[
    (
        "ProductDeprecated",
        "gts.cf.core.events.event_type.v1~cf.bss.products.product_deprecated.v1",
        TRANSCRIBED_PRODUCT_SUBJECT,
    ),
    (
        "SkuDeprecated",
        "gts.cf.core.events.event_type.v1~cf.bss.products.sku_deprecated.v1",
        TRANSCRIBED_SKU_SUBJECT,
    ),
];

/// 04's remaining six — un-deprecation, retirement initiation, both flips.
const THE_LIFECYCLE_REST: &[(&str, &str, &str)] = &[
    (
        "ProductUndeprecated",
        "gts.cf.core.events.event_type.v1~cf.bss.products.product_undeprecated.v1",
        TRANSCRIBED_PRODUCT_SUBJECT,
    ),
    (
        "SkuUndeprecated",
        "gts.cf.core.events.event_type.v1~cf.bss.products.sku_undeprecated.v1",
        TRANSCRIBED_SKU_SUBJECT,
    ),
    (
        "ProductRetired",
        "gts.cf.core.events.event_type.v1~cf.bss.products.product_retired.v1",
        TRANSCRIBED_PRODUCT_SUBJECT,
    ),
    (
        "SkuRetired",
        "gts.cf.core.events.event_type.v1~cf.bss.products.sku_retired.v1",
        TRANSCRIBED_SKU_SUBJECT,
    ),
    (
        "SkuRetirementEffective",
        "gts.cf.core.events.event_type.v1~cf.bss.products.sku_retirement_effective.v1",
        TRANSCRIBED_SKU_SUBJECT,
    ),
    (
        "ProductRetirementEffective",
        "gts.cf.core.events.event_type.v1~cf.bss.products.product_retirement_effective.v1",
        TRANSCRIBED_PRODUCT_SUBJECT,
    ),
];

// One branch per entry point the roster declares: the branching is the
// roster's width, not logic.
#[allow(clippy::cognitive_complexity)]
/// Enqueue one declared event through **the entry point that owns its body
/// shape**, and record the subject the read-back will find it by.
///
/// Lifted out of the round-trip case because that case crossed clippy's
/// `too_many_lines` floor when the fifth entry point landed — and because
/// the five-way routing is the thing under test, so it reads better named
/// than inline.
async fn enqueue_one(
    sink: &crate::infra::broker::EventSink,
    conn: &(impl toolkit_db::secure::DBRunner + Sync),
    token: &str,
    entity_id: Uuid,
    expected: &mut Vec<(String, &'static str)>,
    type_id: &'static str,
) {
    let core = crate::infra::events::EventBodyCore {
        tenant_id: TENANT,
        entity_kind: if token.starts_with("Sku") {
            "sku"
        } else {
            "product"
        },
        entity_id,
        internal_revision: 1,
        lifecycle_state: "draft",
    };
    if token == "CatalogBulkOperationCompleted" {
        crate::infra::events::enqueue_bulk_completed(
            sink,
            conn,
            token,
            crate::infra::events::BulkCompletedEventBody {
                tenant_id: TENANT,
                batch_id: entity_id,
                batch_key: "batch-1",
                ledger_digest: "sha256:0",
                rows: crate::infra::events::BulkCompletedRows {
                    published: 1,
                    applied: 0,
                    no_op: 0,
                    failed: 0,
                },
            },
            ACTOR,
        )
        .await
        .unwrap_or_else(|e| panic!("{token} must enqueue through enqueue_bulk_completed: {e}"));
        expected.push((entity_id.to_string(), type_id));
        return;
    }
    if THE_TAXONOMY_EIGHT.iter().any(|(name, _, _)| *name == token) {
        // Before the `Updated` suffix check below: three of these end in it
        // too, and are not set events.
        let entity_kind = match token {
            "MetadataUpdated" => "product",
            "AttributeDefinitionUpdated" => "attribute_definition",
            _ => "category",
        };
        crate::infra::events::enqueue_taxonomy(
            sink,
            conn,
            entity_id,
            token,
            &crate::infra::events::TaxonomyEventBody {
                tenant_id: TENANT,
                entity_kind,
                entity_id,
                act: "created",
                state: "active",
                mutation_seq: None,
                operation_kind: None,
            },
            ACTOR,
        )
        .await
        .unwrap_or_else(|e| panic!("{token} must enqueue through enqueue_taxonomy: {e}"));
        expected.push((entity_id.to_string(), type_id));
        return;
    }
    if THE_RETENTION_PAIR.iter().any(|(name, _, _)| *name == token) {
        // Before the `Updated` suffix check below, like the taxonomy arm, and
        // before the fall-through: neither of these carries an entity core.
        let subject_ref = entity_id.to_string();
        crate::infra::events::enqueue_retention(
            sink,
            conn,
            entity_id,
            token,
            &crate::infra::events::RetentionEventBody {
                tenant_id: TENANT,
                subject_ref: &subject_ref,
                act: if token == "ActorErased" {
                    "erased"
                } else {
                    "signed_off"
                },
                erased_actor_ref: (token == "ActorErased").then_some(entity_id),
            },
            ACTOR,
        )
        .await
        .unwrap_or_else(|e| panic!("{token} must enqueue through enqueue_retention: {e}"));
        expected.push((subject_ref, type_id));
        return;
    }
    if token.ends_with("Updated") {
        // One distinct kind per member, because the subject IS the kind.
        let set_kind = match token {
            "RecognizedUnitUpdated" => "metering_unit",
            "RecognizedCodeUpdated" => "tax_category",
            _ => "plan_tier",
        };
        crate::infra::events::enqueue_set_event(
            sink,
            conn,
            token,
            crate::infra::events::SetEventBody {
                tenant_id: TENANT,
                set_kind,
                member_code: "gib_month",
                state: "active",
            },
            ACTOR,
        )
        .await
        .unwrap_or_else(|e| panic!("{token} must enqueue through enqueue_set_event: {e}"));
        expected.push((set_kind.to_owned(), type_id));
        return;
    }
    if token == crate::infra::events::REFERENCE_PRODUCER_SET_CHANGED_PAYLOAD_TYPE {
        crate::infra::events::enqueue_producer_set_event(
            sink,
            conn,
            crate::infra::events::ProducerSetEventBody {
                tenant_id: TENANT,
                producer: "pricing",
                state: "registered",
            },
            ACTOR,
        )
        .await
        .unwrap_or_else(|e| panic!("{token} must enqueue through enqueue_producer_set_event: {e}"));
        expected.push((TENANT.to_string(), type_id));
        return;
    }
    if token == crate::infra::events::SKU_IMMUTABLE_FIELD_CORRECTED_PAYLOAD_TYPE
        || token == crate::infra::events::SKU_CORRECTION_OVERRIDE_PAYLOAD_TYPE
    {
        let ceremony = Uuid::from_u128(0xce_01);
        crate::infra::events::enqueue_correction_event(
            sink,
            conn,
            token,
            crate::infra::events::CorrectionEventBody {
                core: &core,
                field: "sku_type",
                value: Some("service"),
                lane: "producer_unavailable",
                quorum_reduced: false,
                correction_ref: ceremony,
            },
            Some(crate::infra::events::OverrideEventBody {
                core: &core,
                arm: "producer_unavailable",
                field: "sku_type",
                ceremony_ref: ceremony,
            }),
            ACTOR,
        )
        .await
        .unwrap_or_else(|e| panic!("{token} must enqueue through enqueue_correction_event: {e}"));
        expected.push((entity_id.to_string(), type_id));
        return;
    }
    if token.ends_with("Published") {
        crate::infra::events::enqueue_published(sink, conn, entity_id, token, &core, 7, ACTOR)
            .await
            .unwrap_or_else(|e| panic!("{token} must enqueue through enqueue_published: {e}"));
    } else if token.ends_with("Undeprecated") {
        crate::infra::events::enqueue(sink, conn, entity_id, token, &core, ACTOR)
            .await
            .unwrap_or_else(|e| panic!("{token} must enqueue through enqueue: {e}"));
    } else if token.ends_with("Retired") || token.ends_with("RetirementEffective") {
        crate::infra::events::enqueue_retired(
            sink,
            conn,
            entity_id,
            token,
            crate::infra::events::RetiredEventBody {
                core: &core,
                from_version: 1,
                reason: "fixture".to_owned(),
                replaced_by: None,
                effective_at: "2026-12-01T00:00:00Z".to_owned(),
                must_migrate_by: None,
            },
            ACTOR,
        )
        .await
        .unwrap_or_else(|e| panic!("{token} must enqueue through enqueue_retired: {e}"));
    } else if token.ends_with("Deprecated") {
        crate::infra::events::enqueue_deprecated(
            sink, conn, entity_id, token, &core, "direct", ACTOR,
        )
        .await
        .unwrap_or_else(|e| panic!("{token} must enqueue through enqueue_deprecated: {e}"));
    } else {
        crate::infra::events::enqueue(sink, conn, entity_id, token, &core, ACTOR)
            .await
            .unwrap_or_else(|e| panic!("{token} must enqueue through enqueue: {e}"));
    }
    expected.push((entity_id.to_string(), type_id));
}

/// The subject type every set event carries, transcribed.
const TRANSCRIBED_SET_SUBJECT: &str =
    "gts.cf.core.events.subject.v1~cf.bss.products.recognized_set.v1";

/// `03`'s set events — names from its §4 roster, ids and subject derived by
/// this module's naming rule (P-D-94), a third list for the second's reason.
const THE_SET_TRIO: &[(&str, &str, &str)] = &[
    (
        "RecognizedUnitUpdated",
        "gts.cf.core.events.event_type.v1~cf.bss.products.recognized_unit_updated.v1",
        TRANSCRIBED_SET_SUBJECT,
    ),
    (
        "RecognizedCodeUpdated",
        "gts.cf.core.events.event_type.v1~cf.bss.products.recognized_code_updated.v1",
        TRANSCRIBED_SET_SUBJECT,
    ),
    (
        "PlanTierUpdated",
        "gts.cf.core.events.event_type.v1~cf.bss.products.plan_tier_updated.v1",
        TRANSCRIBED_SET_SUBJECT,
    ),
];

/// `07`'s three — two SKU-subjected, one on the producer set — names from its
/// §1.8 roster, ids and subjects derived by this module's naming rule.
const THE_REFERENCE_TRIO: &[(&str, &str, &str)] = &[
    (
        "SkuImmutableFieldCorrected",
        "gts.cf.core.events.event_type.v1~cf.bss.products.sku_immutable_field_corrected.v1",
        TRANSCRIBED_SKU_SUBJECT,
    ),
    (
        "SkuCorrectionOverride",
        "gts.cf.core.events.event_type.v1~cf.bss.products.sku_correction_override.v1",
        TRANSCRIBED_SKU_SUBJECT,
    ),
    (
        "ReferenceProducerSetChanged",
        "gts.cf.core.events.event_type.v1~cf.bss.products.reference_producer_set_changed.v1",
        "gts.cf.core.events.subject.v1~cf.bss.products.reference_producer.v1",
    ),
];

/// The subject type the batch summary carries, transcribed.
const TRANSCRIBED_BULK_SUBJECT: &str = "gts.cf.core.events.subject.v1~cf.bss.products.bulk.v1";

/// `09`'s single event — the fourth list, for the third's reason.
const THE_BULK_SUMMARY: &[(&str, &str, &str)] = &[(
    "CatalogBulkOperationCompleted",
    "gts.cf.core.events.event_type.v1~cf.bss.products.catalog_bulk_operation_completed.v1",
    TRANSCRIBED_BULK_SUBJECT,
)];

/// The three subject types `02`'s events carry, transcribed.
const TRANSCRIBED_CATEGORY_SUBJECT: &str =
    "gts.cf.core.events.subject.v1~cf.bss.products.category.v1";
/// [`TRANSCRIBED_CATEGORY_SUBJECT`]'s reason, for definitions.
const TRANSCRIBED_DEFINITION_SUBJECT: &str =
    "gts.cf.core.events.subject.v1~cf.bss.products.attribute_definition.v1";
/// [`TRANSCRIBED_CATEGORY_SUBJECT`]'s reason, for the metadata map.
const TRANSCRIBED_METADATA_SUBJECT: &str =
    "gts.cf.core.events.subject.v1~cf.bss.products.metadata.v1";

/// `02`'s eight — names from its §4.3 roster, ids and subject types derived by
/// this module's naming rule (P-D-94, P-D-122) — a fifth list, for the
/// others' reason.
const THE_TAXONOMY_EIGHT: &[(&str, &str, &str)] = &[
    (
        "CategoryCreated",
        "gts.cf.core.events.event_type.v1~cf.bss.products.category_created.v1",
        TRANSCRIBED_CATEGORY_SUBJECT,
    ),
    (
        "CategoryRenamed",
        "gts.cf.core.events.event_type.v1~cf.bss.products.category_renamed.v1",
        TRANSCRIBED_CATEGORY_SUBJECT,
    ),
    (
        "CategoryReparented",
        "gts.cf.core.events.event_type.v1~cf.bss.products.category_reparented.v1",
        TRANSCRIBED_CATEGORY_SUBJECT,
    ),
    (
        "CategoryRetired",
        "gts.cf.core.events.event_type.v1~cf.bss.products.category_retired.v1",
        TRANSCRIBED_CATEGORY_SUBJECT,
    ),
    (
        "CategoryDeleted",
        "gts.cf.core.events.event_type.v1~cf.bss.products.category_deleted.v1",
        TRANSCRIBED_CATEGORY_SUBJECT,
    ),
    (
        "CategoryDisplayUpdated",
        "gts.cf.core.events.event_type.v1~cf.bss.products.category_display_updated.v1",
        TRANSCRIBED_CATEGORY_SUBJECT,
    ),
    (
        "AttributeDefinitionUpdated",
        "gts.cf.core.events.event_type.v1~cf.bss.products.attribute_definition_updated.v1",
        TRANSCRIBED_DEFINITION_SUBJECT,
    ),
    (
        "MetadataUpdated",
        "gts.cf.core.events.event_type.v1~cf.bss.products.metadata_updated.v1",
        TRANSCRIBED_METADATA_SUBJECT,
    ),
];

/// Transcribed, not imported, for [`TRANSCRIBED_CATEGORY_SUBJECT`]'s reason.
const TRANSCRIBED_ERASURE_SUBJECT: &str =
    "gts.cf.core.events.subject.v1~cf.bss.products.erasure.v1";

/// Transcribed, not imported, for [`TRANSCRIBED_CATEGORY_SUBJECT`]'s reason.
const TRANSCRIBED_ALLOWLIST_SUBJECT: &str =
    "gts.cf.core.events.subject.v1~cf.bss.products.pii_allowlist.v1";

/// `10`'s two (`dod-retention-events`) — names from its own §4 roster, ids
/// and subject types derived by this module's naming rule (P-D-94) — a sixth
/// list, for the others' reason: folding them into any sibling would make
/// that sibling's own completeness claim uncountable.
const THE_RETENTION_PAIR: &[(&str, &str, &str)] = &[
    (
        "ActorErased",
        "gts.cf.core.events.event_type.v1~cf.bss.products.actor_erased.v1",
        TRANSCRIBED_ERASURE_SUBJECT,
    ),
    (
        "PiiAllowlistChanged",
        "gts.cf.core.events.event_type.v1~cf.bss.products.pii_allowlist_changed.v1",
        TRANSCRIBED_ALLOWLIST_SUBJECT,
    ),
];

/// Every event this gear declares: §4.5's eight, 04's pair and rest, 03's
/// trio, 09's summary, 02's eight, 10's pair.
fn every_declared_event() -> Vec<(&'static str, &'static str, &'static str)> {
    THE_EIGHT
        .iter()
        .chain(THE_LIFECYCLE_PAIR)
        .chain(THE_LIFECYCLE_REST)
        .chain(THE_SET_TRIO)
        .chain(THE_BULK_SUMMARY)
        .chain(THE_TAXONOMY_EIGHT)
        .chain(THE_RETENTION_PAIR)
        .chain(THE_REFERENCE_TRIO)
        .copied()
        .collect()
}

/// The `TYPE_ID` each of the eight types actually declares, in the same order.
fn declared() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        (
            ProductCreated::TYPE_ID,
            ProductCreated::SUBJECT_TYPE,
            ProductCreated::TOPIC,
        ),
        (
            SkuCreated::TYPE_ID,
            SkuCreated::SUBJECT_TYPE,
            SkuCreated::TOPIC,
        ),
        (
            ProductHeadSaved::TYPE_ID,
            ProductHeadSaved::SUBJECT_TYPE,
            ProductHeadSaved::TOPIC,
        ),
        (
            SkuHeadSaved::TYPE_ID,
            SkuHeadSaved::SUBJECT_TYPE,
            SkuHeadSaved::TOPIC,
        ),
        (
            ProductPublished::TYPE_ID,
            ProductPublished::SUBJECT_TYPE,
            ProductPublished::TOPIC,
        ),
        (
            SkuPublished::TYPE_ID,
            SkuPublished::SUBJECT_TYPE,
            SkuPublished::TOPIC,
        ),
        (
            ProductDiscarded::TYPE_ID,
            ProductDiscarded::SUBJECT_TYPE,
            ProductDiscarded::TOPIC,
        ),
        (
            SkuDiscarded::TYPE_ID,
            SkuDiscarded::SUBJECT_TYPE,
            SkuDiscarded::TOPIC,
        ),
        (
            ProductDeprecated::TYPE_ID,
            ProductDeprecated::SUBJECT_TYPE,
            ProductDeprecated::TOPIC,
        ),
        (
            SkuDeprecated::TYPE_ID,
            SkuDeprecated::SUBJECT_TYPE,
            SkuDeprecated::TOPIC,
        ),
        (
            ProductUndeprecated::TYPE_ID,
            ProductUndeprecated::SUBJECT_TYPE,
            ProductUndeprecated::TOPIC,
        ),
        (
            SkuUndeprecated::TYPE_ID,
            SkuUndeprecated::SUBJECT_TYPE,
            SkuUndeprecated::TOPIC,
        ),
        (
            ProductRetired::TYPE_ID,
            ProductRetired::SUBJECT_TYPE,
            ProductRetired::TOPIC,
        ),
        (
            SkuRetired::TYPE_ID,
            SkuRetired::SUBJECT_TYPE,
            SkuRetired::TOPIC,
        ),
        (
            SkuRetirementEffective::TYPE_ID,
            SkuRetirementEffective::SUBJECT_TYPE,
            SkuRetirementEffective::TOPIC,
        ),
        (
            ProductRetirementEffective::TYPE_ID,
            ProductRetirementEffective::SUBJECT_TYPE,
            ProductRetirementEffective::TOPIC,
        ),
        (
            RecognizedUnitUpdated::TYPE_ID,
            RecognizedUnitUpdated::SUBJECT_TYPE,
            RecognizedUnitUpdated::TOPIC,
        ),
        (
            RecognizedCodeUpdated::TYPE_ID,
            RecognizedCodeUpdated::SUBJECT_TYPE,
            RecognizedCodeUpdated::TOPIC,
        ),
        (
            PlanTierUpdated::TYPE_ID,
            PlanTierUpdated::SUBJECT_TYPE,
            PlanTierUpdated::TOPIC,
        ),
        (
            CatalogBulkOperationCompleted::TYPE_ID,
            CatalogBulkOperationCompleted::SUBJECT_TYPE,
            CatalogBulkOperationCompleted::TOPIC,
        ),
        (
            CategoryCreated::TYPE_ID,
            CategoryCreated::SUBJECT_TYPE,
            CategoryCreated::TOPIC,
        ),
        (
            CategoryRenamed::TYPE_ID,
            CategoryRenamed::SUBJECT_TYPE,
            CategoryRenamed::TOPIC,
        ),
        (
            CategoryReparented::TYPE_ID,
            CategoryReparented::SUBJECT_TYPE,
            CategoryReparented::TOPIC,
        ),
        (
            CategoryRetired::TYPE_ID,
            CategoryRetired::SUBJECT_TYPE,
            CategoryRetired::TOPIC,
        ),
        (
            CategoryDeleted::TYPE_ID,
            CategoryDeleted::SUBJECT_TYPE,
            CategoryDeleted::TOPIC,
        ),
        (
            CategoryDisplayUpdated::TYPE_ID,
            CategoryDisplayUpdated::SUBJECT_TYPE,
            CategoryDisplayUpdated::TOPIC,
        ),
        (
            AttributeDefinitionUpdated::TYPE_ID,
            AttributeDefinitionUpdated::SUBJECT_TYPE,
            AttributeDefinitionUpdated::TOPIC,
        ),
        (
            MetadataUpdated::TYPE_ID,
            MetadataUpdated::SUBJECT_TYPE,
            MetadataUpdated::TOPIC,
        ),
        (
            ActorErased::TYPE_ID,
            ActorErased::SUBJECT_TYPE,
            ActorErased::TOPIC,
        ),
        (
            PiiAllowlistChanged::TYPE_ID,
            PiiAllowlistChanged::SUBJECT_TYPE,
            PiiAllowlistChanged::TOPIC,
        ),
        (
            SkuImmutableFieldCorrected::TYPE_ID,
            SkuImmutableFieldCorrected::SUBJECT_TYPE,
            SkuImmutableFieldCorrected::TOPIC,
        ),
        (
            SkuCorrectionOverride::TYPE_ID,
            SkuCorrectionOverride::SUBJECT_TYPE,
            SkuCorrectionOverride::TOPIC,
        ),
        (
            ReferenceProducerSetChanged::TYPE_ID,
            ReferenceProducerSetChanged::SUBJECT_TYPE,
            ReferenceProducerSetChanged::TOPIC,
        ),
    ]
}

/// `07`'s producer-set event names the tenant as its subject (P-D-71): the
/// registered set is a per-tenant singleton, so per-aggregate ordering
/// serializes set changes per tenant.
#[test]
fn the_producer_set_event_is_subjected_by_tenant() {
    let event = ReferenceProducerSetChanged {
        tenant_id: TENANT,
        producer: "pricing".to_owned(),
        state: "registered".to_owned(),
        actor_ref: ACTOR,
    };
    assert_eq!(event.subject(), TENANT.to_string());
    assert_eq!(
        ReferenceProducerSetChanged::SUBJECT_TYPE,
        REFERENCE_PRODUCER_SUBJECT_TYPE
    );
}

/// **Each of the eight declares the id this module's doc derived for it, and
/// each id names its own event.**
///
/// The ids are half of an agreement whose other half is a broker-side
/// event-type registration this gear does not own, so a rename here is a broken
/// subscription rather than a refactor — which is why every expected value in
/// [`THE_EIGHT`] is a **literal**, including the subject types. An earlier
/// revision compared `X::SUBJECT_TYPE` against the constant the macro had
/// assigned it, which is the code proving it equals itself.
#[test]
fn each_event_declares_its_derived_type_id_and_subject_type() {
    let declared = declared();
    let transcribed = every_declared_event();
    assert_eq!(
        declared.len(),
        transcribed.len(),
        "one row per declared event: 4.5's eight, 04's pair, 03's trio, 09's summary, 02's eight"
    );

    for ((type_id, subject_type, _), (token, want_type, want_subject)) in
        declared.iter().zip(&transcribed)
    {
        assert_eq!(type_id, want_type, "{token}'s type id moved");
        assert_eq!(subject_type, want_subject, "{token}'s subject type moved");
        assert!(
            type_id.starts_with("gts.cf.core.events.event_type.v1~"),
            "{type_id} must name an event **type**; `event.v1~` is the record namespace"
        );
    }
}

/// **All eight publish onto one topic, and it is the transcribed one.**
///
/// P-D-27's ordering key is `(tenant, aggregate)`, not
/// `(tenant, aggregate, entity_kind)`: a topic per entity kind would change
/// what a consumer subscribes to and nothing about the ordering.
#[test]
fn all_eight_share_one_topic() {
    assert_eq!(
        TOPIC, TRANSCRIBED_TOPIC,
        "the topic this gear publishes onto is broker-side state; a silent rename here is a \
         subscription nobody is serving"
    );
    for (type_id, _, topic) in declared() {
        assert_eq!(topic, TOPIC, "{type_id} publishes onto a second topic");
    }
    assert_eq!(
        ProductCreated::SOURCE,
        SOURCE,
        "the source is the gear's own registered name"
    );
    assert_eq!(
        (PRODUCT_SUBJECT_TYPE, SKU_SUBJECT_TYPE),
        (TRANSCRIBED_PRODUCT_SUBJECT, TRANSCRIBED_SKU_SUBJECT),
        "both subject types are broker-side state too, and are frozen here for the same reason"
    );
}

/// **A subject type follows the entity, not the verb.**
///
/// Four events are about a Product and four about a SKU; a paste that gave a
/// SKU event the Product subject type would satisfy "every constant is set"
/// and would misfile the event at ingest, where `allowed_subject_types` is
/// checked against the **event type's** registration.
#[test]
fn the_subject_type_follows_the_entity_the_event_is_about() {
    let product_events = declared()
        .iter()
        .filter(|(_, subject, _)| *subject == PRODUCT_SUBJECT_TYPE)
        .count();
    let sku_events = declared()
        .iter()
        .filter(|(_, subject, _)| *subject == SKU_SUBJECT_TYPE)
        .count();
    // Eight and eight: §4.5's four-and-four, plus 04's deprecation,
    // un-deprecation, retirement initiation and flip, one per entity kind.
    assert_eq!(
        product_events, 8,
        "eight of the catalog entity events are about a Product"
    );
    assert_eq!(
        sku_events, 10,
        "eight of the catalog entity events plus 07's two correction events are about a SKU"
    );
    let set_events = declared()
        .iter()
        .filter(|(_, subject, _)| *subject == TRANSCRIBED_SET_SUBJECT)
        .count();
    assert_eq!(
        set_events, 3,
        "three are about a recognized set, and about no entity at all"
    );
    let taxonomy_events = declared()
        .iter()
        .filter(|(_, subject, _)| {
            *subject == CATEGORY_SUBJECT_TYPE
                || *subject == ATTRIBUTE_DEFINITION_SUBJECT_TYPE
                || *subject == METADATA_SUBJECT_TYPE
        })
        .count();
    assert_eq!(
        taxonomy_events, 8,
        "02's eight carry the three taxonomy subject types and none of the entity ones"
    );
    assert_eq!(
        (
            CATEGORY_SUBJECT_TYPE,
            ATTRIBUTE_DEFINITION_SUBJECT_TYPE,
            METADATA_SUBJECT_TYPE
        ),
        (
            TRANSCRIBED_CATEGORY_SUBJECT,
            TRANSCRIBED_DEFINITION_SUBJECT,
            TRANSCRIBED_METADATA_SUBJECT
        ),
        "the three taxonomy subject types are broker-side state and are frozen here"
    );

    for (type_id, subject, _) in declared() {
        let names_sku = type_id.contains("sku_");
        assert_eq!(
            subject == SKU_SUBJECT_TYPE,
            names_sku,
            "{type_id} carries the other entity's subject type"
        );
    }
}

/// **`tenant_id` is overridden to the body's tenant, and `partition_key` is
/// not overridden at all.** Both halves are P-D-47's ordering.
///
/// The override is load-bearing: left `None`, the SDK partitions on the
/// authenticated security-context tenant — this gear's own service identity —
/// and every event of every tenant lands on one partition, which is the
/// opposite of *"every event of one tenant lands on one partition in publish
/// order"*. The absent `partition_key` is the other half: P-D-47 says the gear
/// sets none, so ADR-0002's default applies, and an override here would be an
/// amendment to the decision rather than a tuning knob.
#[test]
fn the_partition_inputs_are_the_bodys_tenant_and_no_partition_key() {
    let event = ProductCreated {
        core: core("product"),
    };
    assert_eq!(
        event.tenant_id(),
        Some(TENANT),
        "the partition input must be the entity's tenant, not the producer's"
    );
    assert!(
        event.partition_key().is_none(),
        "P-D-47: the gear sets no partition_key"
    );
    assert_eq!(
        event.subject().as_ref(),
        ENTITY.to_string(),
        "the subject is the entity the event is about"
    );

    let published = SkuPublished {
        core: core("sku"),
        published_version: 7,
    };
    assert_eq!(published.tenant_id(), Some(TENANT));
    assert!(
        published.partition_key().is_none(),
        "the publish twin must not have grown one either"
    );
}

/// **§4.5's five fields are flat on the payload, beside P-D-01's two.**
///
/// Asserted on the rendering rather than the struct: `#[serde(flatten)]` on the
/// core is what puts them at the top level, and a `flatten` dropped in a
/// refactor would nest them under `core` where no consumer looks.
#[test]
fn the_payload_is_the_body_core_flat_beside_the_two_obligations() {
    let json: Value = serde_json::to_value(ProductCreated {
        core: core("product"),
    })
    .expect("the payload renders as JSON");

    assert_eq!(json["tenantId"], "00000000-0000-0000-0000-000000007e42");
    assert_eq!(json["entityKind"], "product");
    assert_eq!(json["internalRevision"], 3);
    assert_eq!(json["lifecycleState"], "draft");
    assert_eq!(
        json["actorRef"], "00000000-0000-0000-0000-00000000ac70",
        "the acting principal rides the payload, the broker Event having no field for one"
    );
    assert!(
        json.get("causationId").is_none(),
        "an absent causation id is absent, not null"
    );
    assert!(
        json.get("core").is_none(),
        "the core must be flattened, not nested under its field name"
    );
    assert!(
        json.get("schemaRef").is_none() && json.get("eventId").is_none(),
        "the schema reference is TYPE_ID and the id is the SDK's; neither belongs in the payload"
    );
}

/// **Only the two publish events carry `publishedVersion`, and it is flat.**
#[test]
fn a_publish_events_version_is_flat_and_the_others_have_none() {
    let published: Value = serde_json::to_value(ProductPublished {
        core: core("product"),
        published_version: 7,
    })
    .expect("renders");
    assert_eq!(published["publishedVersion"], 7);
    assert_eq!(
        published["internalRevision"], 3,
        "the core's five stay flat beside it"
    );

    let discarded: Value = serde_json::to_value(ProductDiscarded {
        core: core("product"),
    })
    .expect("renders");
    assert!(
        discarded.get("publishedVersion").is_none(),
        "a core-only event must not carry a version field at all"
    );
}

/// **A consumer round-trips the payload.**
///
/// [`TypedEvent`] requires `DeserializeOwned`, and the borrowed
/// `events::EventBodyCore` cannot satisfy it. This is the case that would go
/// red if the owned core were ever "simplified" back to `&'static str`.
#[test]
fn the_payload_round_trips_through_a_consumers_deserializer() {
    let sent = SkuPublished {
        core: core("sku"),
        published_version: 2,
    };
    let wire = serde_json::to_string(&sent).expect("renders");
    let received: SkuPublished = serde_json::from_str(&wire).expect("a consumer deserializes it");
    assert_eq!(received, sent);
}

/// **`trace_parent` is the W3C header form, not a bare trace id.**
///
/// The broker's field is named `trace_parent`, so what goes in it must be a
/// `traceparent`. `events::correlation_id` answers the bare 32-hex id — the
/// value that stays grep-equal to the access log — and putting that in this
/// field would be a claim about the value that is not true of it. This case is
/// the one that would notice the two being swapped.
#[test]
fn trace_parent_is_a_w3c_traceparent_and_not_the_bare_trace_id() {
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_sdk::trace::SdkTracerProvider;
    use tracing_opentelemetry::OpenTelemetryLayer;
    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::util::SubscriberInitExt as _;

    let provider = SdkTracerProvider::builder().build();
    let tracer = provider.tracer("bss-products-broker-tests");
    let _guard = tracing_subscriber::registry()
        .with(OpenTelemetryLayer::new(tracer))
        .set_default();

    let span = tracing::info_span!("a-traced-act");
    let _enter = span.enter();

    let event = ProductCreated {
        core: core("product"),
    };
    let parent = event
        .trace_parent()
        .expect("a span under an OpenTelemetry layer must yield a traceparent");

    let parts: Vec<&str> = parent.split('-').collect();
    assert_eq!(
        parts.len(),
        4,
        "a traceparent has four hyphen-separated parts: {parent}"
    );
    assert_eq!(
        parts[0], "00",
        "version 00 is the only one defined: {parent}"
    );
    assert_eq!(
        parts[1].len(),
        32,
        "the trace id is 32 hex characters: {parent}"
    );
    assert_eq!(
        parts[2].len(),
        16,
        "the span id is 16 hex characters: {parent}"
    );
    assert_eq!(parts[3].len(), 2, "the flags are one hex octet: {parent}");
    assert!(
        parent.chars().all(|c| c.is_ascii_hexdigit() || c == '-'),
        "every part is hex: {parent}"
    );
    assert_eq!(
        parts[1],
        crate::infra::events::correlation_id().expect("the same span carries a correlation id"),
        "the traceparent's trace-id segment is exactly what correlation_id answers, which is what \
         keeps the two joinable"
    );
}

/// **Every one of the eight reaches the broker under the id this gear maps it
/// to.**
///
/// The only case that *executes* [`super::bind_producer`] and the eight-row
/// token-to-type dispatch in `infra::events`. That dispatch is a hand-written
/// string switch: `SKU_HEAD_SAVED_PAYLOAD_TYPE => broker::ProductHeadSaved`
/// compiles, and before this case seven of the eight rows — and **both**
/// `enqueue_published` rows — were executed by nothing at all. The commit that
/// introduced them claimed the typed events made the mapping a compile-time
/// concern; at the seam the doors actually call, it is a runtime string switch,
/// and this is what measures it.
///
/// It is also the check on the three derived GTS ids that an earlier revision
/// only appeared to be: the mock's registration is built from
/// [`THE_EIGHT`]'s **transcribed literals**, not from the constants under test,
/// so a topic or subject type renamed in `broker.rs` fails here.
#[tokio::test]
async fn every_event_reaches_the_broker_under_its_own_type_id() {
    use std::sync::Arc;

    use event_broker_sdk::EventBrokerApi;
    use event_broker_sdk::mock::MockBroker;
    use toolkit_db::outbox::Partitions;
    use toolkit_db::{ConnectOpts, connect_db};

    use crate::infra::broker::{EventSink, bind_producer};
    use crate::infra::events;

    // A guard rather than a trailing `remove_file`: the cleanup has to survive a
    // panic, and every assertion below is one. `-wal`/`-shm` go with it, which a
    // single-file removal never took.
    struct TempDb(std::path::PathBuf);
    impl Drop for TempDb {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let mut p = self.0.clone().into_os_string();
                p.push(suffix);
                std::fs::remove_file(std::path::PathBuf::from(p)).ok();
            }
        }
    }

    // The broker, carrying the topic and **all eight** event types — registered
    // from the transcribed literals, so this is an agreement between two
    // independent transcriptions rather than the gear agreeing with itself.
    let broker = Arc::new(MockBroker::new());
    let control = broker.handle();
    control
        .register_topic(TRANSCRIBED_TOPIC, u32::from(events::PARTITIONS))
        .await;
    for (_, type_id, subject_type) in every_declared_event() {
        control
            .register_event_type(
                TRANSCRIBED_TOPIC,
                type_id,
                serde_json::json!({}),
                &[subject_type],
            )
            .await;
    }

    let hub = toolkit::client_hub::ClientHub::new();
    hub.register::<dyn EventBrokerApi>(broker);

    // A database with the outbox facility's tables and the producer's own
    // registration tables — the two `Gear::init` appends, and nothing else:
    // this path never touches a Foundation table.
    let temp = TempDb(
        std::env::temp_dir().join(format!("bss-products-broker-{}.sqlite3", Uuid::new_v4())),
    );
    let dsn = format!("sqlite://{}?mode=rwc", temp.0.display());
    let db = connect_db(
        &dsn,
        ConnectOpts {
            max_conns: Some(1),
            min_conns: Some(1),
            ..Default::default()
        },
    )
    .await
    .expect("connect the file-backed sqlite mirror");
    toolkit_db::migration_runner::run_migrations_for_testing(
        &db,
        toolkit_db::outbox::outbox_migrations_with_prefix(events::OUTBOX_TABLE_PREFIX)
            .expect("a fixed identifier"),
    )
    .await
    .expect("run the outbox facility's migrator");
    toolkit_db::migration_runner::run_migrations_for_testing(
        &db,
        event_broker_sdk::producer_registration_migrations(),
    )
    .await
    .expect("run the producer registration migrator");

    let (sink, _handle) = bind_producer(
        &hub,
        db.clone(),
        events::OUTBOX_TABLE_PREFIX,
        Partitions::of(events::PARTITIONS),
    )
    .await
    .expect("the producer must bind against a broker that carries all eight ids")
    .expect("a ClientHub carrying an EventBrokerApi must not answer None");
    assert!(
        matches!(sink, EventSink::Broker(_)),
        "a reachable broker must select the SDK producer, never the interim queue"
    );

    let provider = toolkit_db::DBProvider::<toolkit_db::DbError>::new(db);
    let conn = provider.conn().expect("checkout a connection");

    // Each entry is (the subject the event will carry, its type id): an
    // entity event's subject is the minted entity id, a set event's is its
    // set kind — one distinct kind per trio member, so the read-back below
    // resolves each event unambiguously.
    let mut expected: Vec<(String, &str)> = Vec::new();
    let roster = every_declared_event();
    for (token, type_id, _) in &roster {
        let entity_id = Uuid::now_v7();
        enqueue_one(&sink, &conn, token, entity_id, &mut expected, type_id).await;
    }

    // The leased processor delivers asynchronously, so the read-back polls
    // rather than assuming. A bounded wait, and a failure here is "nothing was
    // ever published", not a slow machine: the budget is two orders of
    // magnitude above the in-process mock's cost.
    let mut delivered = Vec::new();
    for _ in 0..200_u32 {
        delivered.clear();
        for partition in 0..u32::from(events::PARTITIONS) {
            delivered.extend(control.stored(TRANSCRIBED_TOPIC, partition).await);
        }
        if delivered.len() >= roster.len() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    assert_eq!(
        delivered.len(),
        roster.len(),
        "every declared event must have reached the broker; a token mapped to the wrong type \\
         would be refused at ingest and never arrive"
    );

    // Each event is found by the subject its enqueue carried — a minted
    // entity id, or the set kind for the trio — so a dispatch
    // row wired to the wrong type shows up as a type-id mismatch on **that**
    // token rather than as a count that happens to add up.
    for (subject, want_type_id) in expected {
        let got = delivered
            .iter()
            .map(|stored| &stored.event)
            .find(|event| event.subject == subject)
            .unwrap_or_else(|| panic!("no event arrived for subject {subject}"));
        assert_eq!(
            got.type_id, want_type_id,
            "the token that minted subject {subject} was dispatched to the wrong typed event"
        );
        assert_eq!(got.topic, TRANSCRIBED_TOPIC);
        assert_eq!(got.source, SOURCE);
        assert_eq!(
            got.tenant_id, TENANT,
            "the partition input is the entity's tenant, not the producer's service identity"
        );
        assert!(
            got.partition_key.is_none(),
            "P-D-47: the gear sets no partition_key, so ADR-0002's default applies"
        );
        let data = got.data.as_ref().expect("the payload rides the event");
        assert_eq!(
            data["actorRef"],
            ACTOR.to_string(),
            "P-D-01's actor obligation stays in the payload, the broker Event having no field \\
             for one"
        );
        assert!(
            data.get("eventId").is_none() && data.get("schemaRef").is_none(),
            "the id is the SDK's and the schema reference is the type id; neither is in the \\
             payload"
        );
        let is_publish = want_type_id.contains("_published.");
        assert_eq!(
            data.get("publishedVersion").is_some(),
            is_publish,
            "only the two publish events carry a publishedVersion ({want_type_id})"
        );
    }
}

/// **The outbox half of the publication-propagation budget, measured rather
/// than asserted.**
///
/// `dod-outbox-eventing` says *"The outbox half of the sub-3-second
/// publication-propagation budget belongs here. The probe for it is owed and
/// the 01/06 split of that budget is open at the PRD owner; no measurement in
/// this document establishes it."* This is the measurement. It deliberately
/// **asserts no budget**, because the number the budget splits into is the
/// owner's to set and a threshold invented here would be a guard against
/// nothing.
///
/// # What it does and does not measure
///
/// It times the gear's own path: the instant before `events::enqueue` returns
/// its transaction to the instant the event is readable at the broker. Both
/// ends are in-process — `MockBroker` accepts with no network, no disk beyond
/// the local `SQLite` outbox, and no ingest work — so the number is a **floor**
/// on the outbox half and not a prediction of production. What it does bound is
/// the part this gear owns: enqueue, the sequencer, the leased processor's
/// pickup, and the SDK's publish call. Anything a real broker adds is on the
/// other side of that boundary and belongs to whoever owns the `01/06` split.
///
/// The one thing it **does** assert is arrival, because a timing measurement
/// over an event that never arrived is not a measurement at all.
#[tokio::test]
async fn the_outbox_half_of_the_propagation_budget_is_measured() {
    use std::sync::Arc;
    use std::time::Instant;

    use event_broker_sdk::EventBrokerApi;
    use event_broker_sdk::mock::MockBroker;
    use toolkit_db::outbox::Partitions;
    use toolkit_db::{ConnectOpts, connect_db};

    use crate::infra::broker::bind_producer;
    use crate::infra::events;

    struct TempDb(std::path::PathBuf);
    impl Drop for TempDb {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let mut p = self.0.clone().into_os_string();
                p.push(suffix);
                std::fs::remove_file(std::path::PathBuf::from(p)).ok();
            }
        }
    }

    let broker = Arc::new(MockBroker::new());
    let control = broker.handle();
    control
        .register_topic(TRANSCRIBED_TOPIC, u32::from(events::PARTITIONS))
        .await;
    for (_, type_id, subject_type) in every_declared_event() {
        control
            .register_event_type(
                TRANSCRIBED_TOPIC,
                type_id,
                serde_json::json!({}),
                &[subject_type],
            )
            .await;
    }
    let hub = toolkit::client_hub::ClientHub::new();
    hub.register::<dyn EventBrokerApi>(broker);

    let temp = TempDb(
        std::env::temp_dir().join(format!("bss-products-budget-{}.sqlite3", Uuid::new_v4())),
    );
    let dsn = format!("sqlite://{}?mode=rwc", temp.0.display());
    let db = connect_db(
        &dsn,
        ConnectOpts {
            max_conns: Some(1),
            min_conns: Some(1),
            ..Default::default()
        },
    )
    .await
    .expect("connect the file-backed sqlite mirror");
    toolkit_db::migration_runner::run_migrations_for_testing(
        &db,
        toolkit_db::outbox::outbox_migrations_with_prefix(events::OUTBOX_TABLE_PREFIX)
            .expect("a fixed identifier"),
    )
    .await
    .expect("run the outbox facility's migrator");
    toolkit_db::migration_runner::run_migrations_for_testing(
        &db,
        event_broker_sdk::producer_registration_migrations(),
    )
    .await
    .expect("run the producer registration migrator");

    let (sink, _handle) = bind_producer(
        &hub,
        db.clone(),
        events::OUTBOX_TABLE_PREFIX,
        Partitions::of(events::PARTITIONS),
    )
    .await
    .expect("the producer must bind")
    .expect("a ClientHub carrying an EventBrokerApi must not answer None");

    let provider = toolkit_db::DBProvider::<toolkit_db::DbError>::new(db);
    let conn = provider.conn().expect("checkout a connection");
    let entity_id = Uuid::now_v7();
    let core = crate::infra::events::EventBodyCore {
        tenant_id: TENANT,
        entity_kind: "product",
        entity_id,
        internal_revision: 1,
        lifecycle_state: "draft",
    };

    let started = Instant::now();
    events::enqueue(
        &sink,
        &conn,
        entity_id,
        events::PRODUCT_CREATED_PAYLOAD_TYPE,
        &core,
        ACTOR,
    )
    .await
    .expect("the enqueue must be accepted");

    // Polled at 1ms so the poll interval does not dominate the number the way a
    // 25ms one would; the ceiling is generous because this asserts arrival, not
    // a budget.
    let mut elapsed = None;
    for _ in 0..30_000_u32 {
        let mut arrived = false;
        for partition in 0..u32::from(events::PARTITIONS) {
            if !control
                .stored(TRANSCRIBED_TOPIC, partition)
                .await
                .is_empty()
            {
                arrived = true;
                break;
            }
        }
        if arrived {
            elapsed = Some(started.elapsed());
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }

    let elapsed = elapsed.expect(
        "the event must reach the broker; a propagation measurement over an event that never \
         arrived measures nothing",
    );
    println!(
        "bss-products outbox-half propagation floor: {:.1} ms (enqueue -> readable at an \
         in-process broker; no network, no ingest work). The sub-3-second budget's 01/06 split \
         is the PRD owner's and is not asserted here.",
        elapsed.as_secs_f64() * 1000.0
    );
}

/// The redelivery fixture's own harness: a `MockBroker` carrying the topic
/// and all eight event types, the outbox mirror, and the bound SDK producer.
/// Extracted so the fixture body stays within the line bar; the two sibling
/// cases above keep their inline copies deliberately (each documents a
/// different seam of the same setup).
async fn redelivery_harness(
    tag: &str,
) -> (
    std::sync::Arc<event_broker_sdk::mock::MockBroker>,
    event_broker_sdk::mock::MockBrokerHandle,
    crate::infra::broker::EventSink,
    toolkit_db::DBProvider<toolkit_db::DbError>,
    event_broker_sdk::ProducerOutboxHandle,
    TempDbGuard,
) {
    use std::sync::Arc;

    use event_broker_sdk::EventBrokerApi;
    use event_broker_sdk::mock::MockBroker;
    use toolkit_db::outbox::Partitions;
    use toolkit_db::{ConnectOpts, connect_db};

    use crate::infra::broker::bind_producer;
    use crate::infra::events;

    let broker = Arc::new(MockBroker::new());
    let control = broker.handle();
    control
        .register_topic(TRANSCRIBED_TOPIC, u32::from(events::PARTITIONS))
        .await;
    for (_, type_id, subject_type) in every_declared_event() {
        control
            .register_event_type(
                TRANSCRIBED_TOPIC,
                type_id,
                serde_json::json!({}),
                &[subject_type],
            )
            .await;
    }
    let hub = toolkit::client_hub::ClientHub::new();
    hub.register::<dyn EventBrokerApi>(Arc::clone(&broker) as Arc<_>);

    let temp = TempDbGuard(
        std::env::temp_dir().join(format!("bss-products-{tag}-{}.sqlite3", Uuid::new_v4())),
    );
    let dsn = format!("sqlite://{}?mode=rwc", temp.0.display());
    let db = connect_db(
        &dsn,
        ConnectOpts {
            max_conns: Some(1),
            min_conns: Some(1),
            ..Default::default()
        },
    )
    .await
    .expect("connect the file-backed sqlite mirror");
    toolkit_db::migration_runner::run_migrations_for_testing(
        &db,
        toolkit_db::outbox::outbox_migrations_with_prefix(events::OUTBOX_TABLE_PREFIX)
            .expect("a fixed identifier"),
    )
    .await
    .expect("run the outbox facility's migrator");
    toolkit_db::migration_runner::run_migrations_for_testing(
        &db,
        event_broker_sdk::producer_registration_migrations(),
    )
    .await
    .expect("run the producer registration migrator");

    let (sink, handle) = bind_producer(
        &hub,
        db.clone(),
        events::OUTBOX_TABLE_PREFIX,
        Partitions::of(events::PARTITIONS),
    )
    .await
    .expect("the producer must bind")
    .expect("a ClientHub carrying an EventBrokerApi must not answer None");

    let provider = toolkit_db::DBProvider::<toolkit_db::DbError>::new(db);
    (broker, control, sink, provider, handle, temp)
}

/// [`redelivery_harness`]'s cleanup guard - survives a panic, and takes the
/// `-wal`/`-shm` companions with the file.
struct TempDbGuard(std::path::PathBuf);
impl Drop for TempDbGuard {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let mut p = self.0.clone().into_os_string();
            p.push(suffix);
            std::fs::remove_file(std::path::PathBuf::from(p)).ok();
        }
    }
}

/// `dod-dedup-ordering`'s two fixtures — a duplicate delivery and an
/// out-of-order one — on the surface the contract names (**P-D-58**): the
/// stored log and the per-partition cursors `MockBroker` exports.
///
/// # The two keys, both asserted against a REAL redelivery
///
/// The duplicate delivery is produced by the mechanism that produces it in
/// production — the consumer's cursor rewound (SEEK) and the stream read
/// again — never by copying a list, which could only prove a list equals
/// itself. The second delivery of every event must repeat the **event `id`**
/// (the within-window key, minted once at enqueue) and the
/// **`(tenant, aggregate, sequence)`** triple (the beyond-window key, whose
/// `sequence` is the broker's server-assigned per-`(topic, partition)` value
/// — P-D-47, never a gear-assigned one).
///
/// # Monotonicity, not density
///
/// The gear sets no `partition_key`, so ADR-0002's default puts every event
/// of one tenant on ONE partition and `sequence` is monotonic across them.
/// The fixture interleaves a second aggregate between one aggregate's two
/// acts, so the first aggregate's neighbouring events carry a **gap** — and
/// the out-of-order detector (last-seen comparison) passes the gapped
/// in-order delivery while firing on any reversed pair. A consumer treating
/// the gap as loss would re-bootstrap on healthy traffic; this is the case
/// that proves the contract's sentence.
///
/// @cpt-dod:cpt-cf-bss-products-dod-dedup-ordering:p1
#[tokio::test]
async fn the_dedup_and_ordering_keys_hold_across_a_real_redelivery() {
    use std::sync::Arc;
    use std::time::Duration;

    use event_broker_sdk::api::{
        BarrierMode, JoinRequest, SeekPosition, SubscriptionInterest, TenantTraversalDepth,
        WireFrame,
    };
    use event_broker_sdk::mock::stubs::test_ctx_for_tenant;
    use event_broker_sdk::models::CreateConsumerGroupRequest;
    use event_broker_sdk::{EventBrokerApi, ResolvedPosition};

    use crate::infra::events;

    let (broker, control, sink, provider, _processor, _temp) = redelivery_harness("dedup").await;
    let conn = provider.conn().expect("checkout a connection");

    // Three acts, two aggregates, ONE tenant — each awaited to the broker
    // before the next enqueues, so the stored order is [A1, B1, A2] by
    // construction and aggregate A's neighbours are provably gapped.
    let aggregate_a = Uuid::now_v7();
    let aggregate_b = Uuid::now_v7();
    let acts: &[(Uuid, &str, i64)] = &[
        (aggregate_a, "ProductCreated", 1),
        (aggregate_b, "ProductCreated", 1),
        (aggregate_a, "ProductHeadSaved", 2),
    ];
    for (arrived, (entity_id, token, revision)) in (1..=acts.len()).zip(acts) {
        let core = crate::infra::events::EventBodyCore {
            tenant_id: TENANT,
            entity_kind: "product",
            entity_id: *entity_id,
            internal_revision: *revision,
            lifecycle_state: "draft",
        };
        crate::infra::events::enqueue(&sink, &conn, *entity_id, token, &core, ACTOR)
            .await
            .unwrap_or_else(|e| panic!("{token} must enqueue: {e}"));
        let mut seen = 0;
        for _ in 0..200_u32 {
            seen = 0;
            for partition in 0..u32::from(events::PARTITIONS) {
                seen += control.stored(TRANSCRIBED_TOPIC, partition).await.len();
            }
            if seen >= arrived {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert_eq!(seen, arrived, "act {arrived} must reach the broker");
    }

    // --- The ordering half, on the stored log. ---
    let mut tenant_partition = None;
    let mut stored = Vec::new();
    for partition in 0..u32::from(events::PARTITIONS) {
        let log = control.stored(TRANSCRIBED_TOPIC, partition).await;
        if !log.is_empty() {
            assert!(
                tenant_partition.is_none(),
                "no partition_key is set, so ADR-0002's default must put every event of one \
                 tenant on ONE partition; a second non-empty partition breaks the ordering key"
            );
            tenant_partition = Some(partition);
            stored = log;
        }
    }
    let tenant_partition = tenant_partition.expect("one partition holds the tenant's events");
    let sequences: Vec<i64> = stored
        .iter()
        .map(|s| {
            s.event
                .sequence
                .expect("the broker stamps sequence at ingest")
        })
        .collect();
    assert!(
        sequences.windows(2).all(|w| w[0] < w[1]),
        "sequence must be strictly monotonic across ONE tenant's events, whatever the \
         aggregate: {sequences:?}"
    );
    let a_seqs: Vec<i64> = stored
        .iter()
        .filter(|s| s.event.subject == aggregate_a.to_string())
        .map(|s| s.event.sequence.expect("stamped"))
        .collect();
    assert_eq!(a_seqs.len(), 2, "aggregate A carries two acts");
    assert!(
        a_seqs[1] - a_seqs[0] > 1,
        "aggregate A's neighbouring events must carry a GAP (aggregate B sits between them): \
         detection needs monotonicity, not density, and a consumer treating this gap as loss \
         would re-bootstrap on healthy traffic"
    );

    // --- The consumer, and the REAL redelivery. ---
    let ctx = test_ctx_for_tenant(TENANT);
    let group = broker
        .create_consumer_group(
            &ctx,
            CreateConsumerGroupRequest {
                client_agent: "bss-products-dedup-fixture/1.0".to_owned(),
                description: None,
            },
        )
        .await
        .expect("create the consumer group")
        .id;
    // One consumer session: join the group, seek, stream until three
    // events, leave. The duplicate below is produced by the mechanism that
    // produces it in production — a consumer session ends (a crash, a
    // deploy) and its successor re-reads from the rewound cursor. The
    // mock's per-subscription scan frontier only moves forward, exactly so
    // that a duplicate can only come from a session boundary, never from
    // one session re-reading itself.
    let join = || async {
        broker
            .join(
                &ctx,
                JoinRequest {
                    group,
                    client_agent: "bss-products-dedup-fixture/1.0".to_owned(),
                    interests: vec![
                        SubscriptionInterest::builder()
                            .topic(TRANSCRIBED_TOPIC)
                            .tenant_id(Uuid::nil())
                            .tenant_depth(TenantTraversalDepth::CurrentTenant)
                            .barrier_mode(BarrierMode::Respect)
                            .types(["*"])
                            .build()
                            .expect("a well-formed interest"),
                    ],
                    session_timeout: Some(Duration::from_secs(30)),
                },
            )
            .await
            .expect("join the group")
            .subscription_id
    };

    // One delivery attempt: seek to Earliest, stream until three events.
    let deliver = |label: &'static str, sub_id| {
        let broker = Arc::clone(&broker);
        let ctx = ctx.clone();
        async move {
            // The mock refuses a stream over any unseeded assigned
            // partition, so every one is seeked — the rewind that matters
            // is the tenant partition's.
            let positions: Vec<SeekPosition> = (0..u32::from(events::PARTITIONS))
                .map(|partition| SeekPosition {
                    topic: TRANSCRIBED_TOPIC.to_owned(),
                    partition,
                    value: ResolvedPosition::Earliest,
                })
                .collect();
            broker
                .seek(&ctx, sub_id, &positions)
                .await
                .expect("seek every assigned partition to the earliest offset");
            let mut stream = broker.stream(&ctx, sub_id).await.expect("open the stream");
            let mut events = Vec::new();
            let budget = tokio::time::Instant::now() + Duration::from_secs(5);
            while events.len() < 3 {
                let frame = tokio::time::timeout_at(
                    budget,
                    std::future::poll_fn(|cx| stream.as_mut().poll_next(cx)),
                )
                .await
                .unwrap_or_else(|_| panic!("{label}: the stream must deliver three events"))
                .unwrap_or_else(|| panic!("{label}: the stream must not end early"))
                .unwrap_or_else(|e| panic!("{label}: the stream must not fail: {e}"));
                if let WireFrame::Event(event) = frame {
                    events.push(event);
                }
            }
            events
        }
    };

    let first_session = join().await;
    let first = deliver("first delivery", first_session).await;
    // The cursor surface the contract names, both halves readable: the
    // session `offset` is where SEEK anchored it (0 — the broker emits from
    // offset+1), and `last_examined` has scanned through the third stored
    // event. The rewind that produces the duplicate below is exactly this
    // anchor being set again.
    assert_eq!(
        control
            .cursor(&group, TRANSCRIBED_TOPIC, tenant_partition)
            .await,
        Some(0),
        "the session cursor must be readable and sit where SEEK anchored it"
    );
    assert_eq!(
        control
            .last_examined(&group, TRANSCRIBED_TOPIC, tenant_partition)
            .await,
        Some(3),
        "last_examined must be readable and have scanned through the third delivery"
    );
    // The session boundary: the first consumer leaves, its successor joins
    // the same group and rewinds — the at-least-once duplicate's own shape.
    broker
        .leave(&ctx, first_session)
        .await
        .expect("the first session leaves");
    let second_session = join().await;
    let second = deliver("second delivery (the duplicate)", second_session).await;

    for (a, b) in first.iter().zip(&second) {
        // Within the idempotency window: the event id, minted once at
        // enqueue, repeated by every delivery attempt.
        assert_eq!(a.id, b.id, "a redelivery must repeat the event id");
        // Beyond the window: (tenant, aggregate, sequence) — the broker's
        // server-assigned sequence, identical on every attempt.
        assert_eq!(
            (a.tenant_id, &a.subject, a.sequence),
            (b.tenant_id, &b.subject, b.sequence),
            "a redelivery must repeat the (tenant, aggregate, sequence) triple"
        );
    }

    // The out-of-order detector: last-seen comparison per partition. The
    // gapped in-order delivery passes; ANY reversed pair fires it.
    let in_order = first.windows(2).all(|w| w[0].sequence < w[1].sequence);
    assert!(
        in_order,
        "the delivered order must already satisfy the monotonic detector"
    );
    let reversed_fires = first[2].sequence >= first[1].sequence;
    assert!(
        reversed_fires,
        "delivering event 3 before event 2 must fail the last-seen comparison; the \
         out-of-order case is detected on the sequence key alone"
    );
}
