//! The typed events' own guards: the three GTS constants, the two overrides
//! that carry P-D-47's ordering, and the shape a consumer deserializes.

use event_broker_sdk::TypedEvent;
use serde_json::Value;
use uuid::Uuid;

use super::{
    CatalogEventCore, PRODUCT_SUBJECT_TYPE, ProductCreated, ProductDiscarded, ProductHeadSaved,
    ProductPublished, SKU_SUBJECT_TYPE, SOURCE, SkuCreated, SkuDiscarded, SkuHeadSaved,
    SkuPublished, TOPIC,
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

/// One row per event: its `TYPE_ID`, its `SUBJECT_TYPE`, and the §4.5 token the
/// id must name.
///
/// Transcribed rather than read off the types, for `events_tests`' reason: a
/// list built from the code under test could only prove the code equals itself.
const THE_EIGHT: &[(&str, &str, &str)] = &[
    (
        "gts.cf.core.events.event_type.v1~cf.bss.products.product_created.v1",
        PRODUCT_SUBJECT_TYPE,
        "product_created",
    ),
    (
        "gts.cf.core.events.event_type.v1~cf.bss.products.sku_created.v1",
        SKU_SUBJECT_TYPE,
        "sku_created",
    ),
    (
        "gts.cf.core.events.event_type.v1~cf.bss.products.product_head_saved.v1",
        PRODUCT_SUBJECT_TYPE,
        "product_head_saved",
    ),
    (
        "gts.cf.core.events.event_type.v1~cf.bss.products.sku_head_saved.v1",
        SKU_SUBJECT_TYPE,
        "sku_head_saved",
    ),
    (
        "gts.cf.core.events.event_type.v1~cf.bss.products.product_published.v1",
        PRODUCT_SUBJECT_TYPE,
        "product_published",
    ),
    (
        "gts.cf.core.events.event_type.v1~cf.bss.products.sku_published.v1",
        SKU_SUBJECT_TYPE,
        "sku_published",
    ),
    (
        "gts.cf.core.events.event_type.v1~cf.bss.products.product_discarded.v1",
        PRODUCT_SUBJECT_TYPE,
        "product_discarded",
    ),
    (
        "gts.cf.core.events.event_type.v1~cf.bss.products.sku_discarded.v1",
        SKU_SUBJECT_TYPE,
        "sku_discarded",
    ),
];

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
    ]
}

/// **Each of the eight declares the id this module's doc derived for it, and
/// each id names its own event.**
///
/// The ids are half of an agreement whose other half is a broker-side event-type
/// registration this gear does not own, so a rename here is a broken
/// subscription rather than a refactor — which is exactly why the expected
/// values are written out rather than read from the constants.
#[test]
fn each_event_declares_its_derived_type_id_and_subject_type() {
    let declared = declared();
    assert_eq!(declared.len(), THE_EIGHT.len(), "eight events, eight rows");

    for ((type_id, subject_type, _), (want_type, want_subject, token)) in
        declared.iter().zip(THE_EIGHT)
    {
        assert_eq!(type_id, want_type, "{token}'s type id moved");
        assert_eq!(subject_type, want_subject, "{token}'s subject type moved");
        assert!(
            type_id.ends_with(&format!("{token}.v1")),
            "{type_id} does not name its own event {token}"
        );
    }
}

/// **All eight publish onto one topic, and it is the one this module names.**
///
/// P-D-27's ordering key is `(tenant, aggregate)`, not
/// `(tenant, aggregate, entity_kind)`: a topic per entity kind would change
/// what a consumer subscribes to and nothing about the ordering.
#[test]
fn all_eight_share_one_topic() {
    for (type_id, _, topic) in declared() {
        assert_eq!(topic, TOPIC, "{type_id} publishes onto a second topic");
    }
    assert_eq!(
        ProductCreated::SOURCE,
        SOURCE,
        "the source is the gear's own registered name"
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
    assert_eq!(product_events, 4, "four of the eight are about a Product");
    assert_eq!(sku_events, 4, "four of the eight are about a SKU");

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

/// **The producer path actually publishes, and the broker accepts what this
/// gear declares.**
///
/// Everything above judges constants and renderings; this is the only case that
/// *executes* [`super::bind_producer`], and it is here for the reason the
/// Postgres tier's own history records: a path no test runs is a path that
/// compiles and does nothing. It is also the first real check on the three
/// derived GTS ids — the mock refuses an event type its topic does not carry
/// and a `subject_type` outside the registration's allow-list, so a derivation
/// that disagreed with the registration below fails here rather than in
/// production.
///
/// What it pins beyond "a row arrived": the `type_id` is the **type** id, the
/// subject is the entity, and the partition input is the **entity's** tenant,
/// which is what P-D-47's per-tenant ordering rests on.
#[tokio::test]
async fn the_producer_publishes_a_typed_event_the_broker_accepts() {
    use std::sync::Arc;

    use event_broker_sdk::EventBrokerApi;
    use event_broker_sdk::mock::MockBroker;
    use toolkit_db::outbox::Partitions;
    use toolkit_db::{ConnectOpts, connect_db};

    use crate::infra::broker::{EventSink, bind_producer};
    use crate::infra::events;

    const PREFIX: &str = "bss_products_outbox";
    const PARTITIONS: u16 = 8;

    // The broker, carrying exactly the topic and event type this gear derives.
    let broker = Arc::new(MockBroker::new());
    let control = broker.handle();
    control.register_topic(TOPIC, u32::from(PARTITIONS)).await;
    control
        .register_event_type(
            TOPIC,
            ProductCreated::TYPE_ID,
            serde_json::json!({}),
            &[PRODUCT_SUBJECT_TYPE],
        )
        .await;

    let hub = toolkit::client_hub::ClientHub::new();
    hub.register::<dyn EventBrokerApi>(broker);

    // A database with the outbox facility's tables and the producer's own
    // registration tables — the two `Gear::init` appends, and nothing else:
    // this path never touches a Foundation table.
    let path = std::env::temp_dir().join(format!("bss-products-broker-{}.sqlite3", Uuid::new_v4()));
    let dsn = format!("sqlite://{}?mode=rwc", path.display());
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
        toolkit_db::outbox::outbox_migrations_with_prefix(PREFIX).expect("a fixed identifier"),
    )
    .await
    .expect("run the outbox facility's migrator");
    toolkit_db::migration_runner::run_migrations_for_testing(
        &db,
        event_broker_sdk::producer_registration_migrations(),
    )
    .await
    .expect("run the producer registration migrator");

    let bound = bind_producer(&hub, db.clone(), PREFIX, Partitions::of(PARTITIONS))
        .await
        .expect("the producer must bind against a broker that carries our ids")
        .expect("a ClientHub carrying an EventBrokerApi must not answer None");
    let (sink, _handle) = bound;
    assert!(
        matches!(sink, EventSink::Broker(_)),
        "a reachable broker must select the SDK producer, never the interim queue"
    );

    let entity_id = Uuid::now_v7();
    let core = events::EventBodyCore {
        tenant_id: TENANT,
        entity_kind: "product",
        entity_id,
        internal_revision: 1,
        lifecycle_state: "draft",
    };
    let provider = toolkit_db::DBProvider::<toolkit_db::DbError>::new(db);
    let conn = provider.conn().expect("checkout a connection");
    events::enqueue(
        &sink,
        &conn,
        entity_id,
        events::PRODUCT_CREATED_PAYLOAD_TYPE,
        &core,
        ACTOR,
    )
    .await
    .expect("the producer must accept a registered payload type");

    // The leased processor delivers asynchronously, so the read-back polls
    // rather than assuming. A bounded wait, and a failure here is "nothing was
    // ever published", not a slow machine: the budget is two orders of
    // magnitude above the in-process mock's cost.
    let mut delivered = Vec::new();
    for _ in 0..200_u32 {
        for partition in 0..u32::from(PARTITIONS) {
            let stored = control.stored(TOPIC, partition).await;
            if !stored.is_empty() {
                delivered = stored;
                break;
            }
        }
        if !delivered.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    assert_eq!(
        delivered.len(),
        1,
        "exactly one event must have reached the broker"
    );
    let event = &delivered[0].event;
    assert_eq!(
        event.type_id,
        ProductCreated::TYPE_ID,
        "the broker must have accepted the event **type** id this gear declares"
    );
    assert_eq!(event.topic, TOPIC);
    assert_eq!(event.source, SOURCE);
    assert_eq!(
        event.subject,
        entity_id.to_string(),
        "the subject is the entity the event is about"
    );
    assert_eq!(
        event.subject_type, PRODUCT_SUBJECT_TYPE,
        "and it passed the registration's own allowed_subject_types check"
    );
    assert_eq!(
        event.tenant_id, TENANT,
        "the partition input is the entity's tenant, not the producer's service identity"
    );
    assert!(
        event.partition_key.is_none(),
        "P-D-47: the gear sets no partition_key, so ADR-0002's default applies"
    );

    let data = event.data.as_ref().expect("the payload rides the event");
    assert_eq!(data["entityKind"], "product");
    assert_eq!(data["internalRevision"], 1);
    assert_eq!(
        data["actorRef"],
        ACTOR.to_string(),
        "P-D-01's actor obligation stays in the payload, the broker Event having no field for one"
    );
    assert!(
        data.get("eventId").is_none() && data.get("schemaRef").is_none(),
        "the id is the SDK's and the schema reference is the type id; neither is in the payload"
    );

    std::fs::remove_file(&path).ok();
}
