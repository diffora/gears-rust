//! The event envelope's own guards (`design/01-foundation.md` §4.5, P-D-01,
//! P-D-47).
//!
//! The two `partition_for` cases stay inline in `events.rs` beside the
//! formula they judge; what lives here is everything about the **envelope** —
//! the schema-reference roster, the envelope's own four fields (three of
//! which are P-D-01 obligations — see the case that asserts them), and the
//! shape a consumer reads.

use serde_json::Value;
use uuid::Uuid;

use super::{
    EntityKind, EventBodyCore, EventEnvelope, PRODUCT_CREATED_PAYLOAD_TYPE,
    PRODUCT_DISCARDED_PAYLOAD_TYPE, PRODUCT_HEAD_SAVED_PAYLOAD_TYPE,
    PRODUCT_PUBLISHED_PAYLOAD_TYPE, PublishedEventBody, SCHEMA_REFS, SKU_CREATED_PAYLOAD_TYPE,
    SKU_DISCARDED_PAYLOAD_TYPE, SKU_HEAD_SAVED_PAYLOAD_TYPE, SKU_PUBLISHED_PAYLOAD_TYPE,
    schema_ref_for,
};

/// §4.5's roster, written out here rather than read from the code under test.
///
/// A test that built this list from [`SCHEMA_REFS`] could only prove the array
/// equals itself. These eight names are transcribed from the design's own
/// sentence, so a token renamed in the code without the design moving is a
/// red here.
const THE_EIGHT: &[&str] = &[
    "ProductCreated",
    "SkuCreated",
    "ProductHeadSaved",
    "SkuHeadSaved",
    "ProductPublished",
    "SkuPublished",
    "ProductDiscarded",
    "SkuDiscarded",
];

fn core() -> EventBodyCore {
    EventBodyCore {
        tenant_id: Uuid::from_u128(0x7e_42),
        entity_kind: EntityKind::Product.as_str(),
        entity_id: Uuid::from_u128(0x_1111),
        internal_revision: 3,
        lifecycle_state: "draft",
    }
}

/// The envelope as a consumer receives it, for the assertions below.
fn rendered<B: serde::Serialize>(body: &B, payload_type: &str) -> Value {
    let envelope = EventEnvelope {
        event_id: Uuid::from_u128(0x_e0e0),
        schema_ref: schema_ref_for(payload_type).expect("the roster names this event"),
        correlation_id: Some("4bf92f3577b34da6a3ce929d0e0e4736".to_owned()),
        causation_id: None,
        actor_ref: Uuid::from_u128(0x_ac70),
        data: body,
    };
    serde_json::to_value(&envelope).expect("the envelope renders as JSON")
}

/// **Every one of §4.5's eight has a versioned schema reference, and nothing
/// else does.**
///
/// Both directions matter. A missing entry is an event that would be refused
/// at its first enqueue; a *surplus* entry is a schema reference announced for
/// an event this gear does not emit, which a consumer contract would take for
/// a promise.
#[test]
fn the_schema_roster_names_exactly_the_eight_foundation_events() {
    let registered: Vec<&str> = SCHEMA_REFS.iter().map(|(token, _)| *token).collect();

    for event in THE_EIGHT {
        assert!(
            registered.contains(event),
            "{event} is one of the eight and carries no schema reference"
        );
    }
    for token in &registered {
        assert!(
            THE_EIGHT.contains(token),
            "{token} carries a schema reference but is not one of the eight"
        );
    }
    assert_eq!(
        registered.len(),
        THE_EIGHT.len(),
        "the roster must carry each of the eight exactly once"
    );
}

/// A schema reference is **versioned**, and the version belongs to the event
/// rather than to the gear (P-D-01: "versioned (semver) schema references").
///
/// The reference must also name its own event: a roster whose entries were
/// pasted from a neighbour would satisfy a bare "ends in a version" check and
/// would point every consumer at one schema.
#[test]
fn every_schema_reference_is_semver_and_names_its_own_event() {
    for (token, schema_ref) in SCHEMA_REFS {
        let (name, version) = schema_ref
            .rsplit_once(".v")
            .unwrap_or_else(|| panic!("{schema_ref} carries no .vN version segment"));
        assert!(
            name.ends_with(token),
            "{schema_ref} does not name its own event {token}"
        );
        let parts: Vec<&str> = version.split('.').collect();
        assert_eq!(parts.len(), 3, "{schema_ref} is not a three-part semver");
        for part in parts {
            assert!(
                part.parse::<u32>().is_ok(),
                "{schema_ref} has a non-numeric semver segment {part}"
            );
        }
    }
}

/// **The envelope's four fields are on the wire**, under the names a consumer
/// reads them by.
///
/// Not, as an earlier revision of this doc said, "the four obligations P-D-01
/// names". P-D-01 names **five**, and the mapping is not one-to-one:
/// `schemaRef`, `correlationId` and `actorRef` are its obligations;
/// `eventId` is **not** — that is the interim envelope's own handle, and
/// P-D-47's idempotency key will be the SDK's id (see `EventEnvelope::event_id`);
/// and P-D-01's ordering key is asserted **nowhere here**, because it is not an
/// envelope field at all — it is the partition, judged by `events.rs`'s own
/// inline cases and, for the guarantee a consumer actually gets, owned by the
/// broker under P-D-47 (§4.4). The `vN`->`vN+1` obligation is slice 12's.
///
/// This is `dod-outbox-eventing`'s envelope clause asserted on the rendering
/// rather than on the struct: a field renamed by a stray `#[serde(rename)]`,
/// or dropped by a `skip_serializing_if` that fires when it should not, is
/// invisible to a test that reads the Rust value.
#[test]
fn the_envelope_carries_the_four_obligations_and_the_body_beneath_them() {
    let core = core();
    let json = rendered(&core, PRODUCT_CREATED_PAYLOAD_TYPE);

    assert_eq!(
        json["eventId"], "00000000-0000-0000-0000-00000000e0e0",
        "the interim envelope carries its own event id"
    );
    assert_eq!(json["schemaRef"], "bss-products.ProductCreated.v1.0.0");
    assert_eq!(json["correlationId"], "4bf92f3577b34da6a3ce929d0e0e4736");
    assert_eq!(
        json["actorRef"], "00000000-0000-0000-0000-00000000ac70",
        "the acting principal rides the envelope pseudonymously"
    );

    // The body is a nested object, not flattened beside the envelope: §4.5's
    // five fields are read from one place whatever the envelope grows next.
    assert_eq!(
        json["data"]["tenantId"],
        "00000000-0000-0000-0000-000000007e42"
    );
    assert_eq!(json["data"]["entityKind"], "product");
    assert_eq!(json["data"]["internalRevision"], 3);
    assert_eq!(json["data"]["lifecycleState"], "draft");
    assert!(
        json.get("tenantId").is_none(),
        "the body must not also appear at the envelope's own level"
    );
}

/// **An absent causation id is absent, not null and not the correlation id.**
///
/// The pair exists to distinguish "caused by a request" from "caused by
/// another event". Rendering `causationId` equal to `correlationId` — the
/// tempting shortcut — would make every Foundation event look like it was
/// caused by an event, and no consumer could tell the two situations apart
/// again.
#[test]
fn a_foundation_events_causation_id_is_omitted_rather_than_echoing_the_correlation() {
    let core = core();
    let json = rendered(&core, PRODUCT_DISCARDED_PAYLOAD_TYPE);

    assert!(
        json.get("causationId").is_none(),
        "no operator-caused event may name a causing event"
    );
    assert!(
        json.get("correlationId").is_some(),
        "the correlation half must still be there, or this test proves nothing"
    );
}

/// An untraced request yields **no** correlation id rather than a minted one.
///
/// `repo::AuditCommon::correlation_id` records the judgement this asserts:
/// a value that correlates nothing while reading as though it does is worse
/// than its absence.
#[test]
fn an_untraced_request_omits_the_correlation_id_entirely() {
    let core = core();
    let envelope = EventEnvelope {
        event_id: Uuid::from_u128(0x_e0e0),
        schema_ref: schema_ref_for(SKU_CREATED_PAYLOAD_TYPE).expect("registered"),
        correlation_id: None,
        causation_id: None,
        actor_ref: Uuid::from_u128(0x_ac70),
        data: &core,
    };
    let json = serde_json::to_value(&envelope).expect("renders");

    assert!(
        json.get("correlationId").is_none(),
        "an absent trace must leave the field off the wire, never fill it"
    );
    assert!(
        json.get("actorRef").is_some(),
        "the rest of the envelope must survive an untraced request"
    );
}

/// The two publish events carry `publishedVersion` **inside `data`**, flat
/// beside the core's five (§4.5's "additionally carry").
///
/// The envelope must not have moved it: a consumer reading
/// `data.publishedVersion` is reading where §4.5 puts it.
#[test]
fn a_publish_events_version_stays_flat_inside_the_body() {
    let core = core();
    let body = PublishedEventBody {
        core: &core,
        published_version: 7,
    };
    let json = rendered(&body, PRODUCT_PUBLISHED_PAYLOAD_TYPE);

    assert_eq!(json["data"]["publishedVersion"], 7);
    assert_eq!(
        json["data"]["internalRevision"], 3,
        "the core's five must still be flat beside it"
    );
    assert_eq!(json["schemaRef"], "bss-products.ProductPublished.v1.0.0");
}

/// **`correlation_id` reads a real trace id back off the ambient span.**
///
/// The one positive control this obligation has. Measured across the crate:
/// three assertions read the field **absent** (one here, one in each door
/// suite) and two read it present — but both of those are reading back a value
/// [`rendered`] supplied by hand, not one this function produced. So a `correlation_id` that
/// answered `None` unconditionally (the wrong span-extension trait, a
/// subscriber layer never consulted, the `TraceId::INVALID` comparison
/// inverted) would leave the whole suite green while P-D-01's correlation
/// obligation shipped inert on all eight events.
///
/// The layer is installed here rather than assumed: see `correlation_id`'s own
/// doc for the three host conditions under which the production answer is a
/// permanent `None`. `set_default` is thread-local and leaves the process-wide
/// default untouched, so this case cannot disturb one running beside it.
#[test]
fn correlation_id_reads_the_trace_id_of_the_ambient_span() {
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_sdk::trace::SdkTracerProvider;
    use tracing_opentelemetry::OpenTelemetryLayer;
    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::util::SubscriberInitExt as _;

    let provider = SdkTracerProvider::builder().build();
    let tracer = provider.tracer("bss-products-events-tests");
    let _guard = tracing_subscriber::registry()
        .with(OpenTelemetryLayer::new(tracer))
        .set_default();

    let span = tracing::info_span!("a-traced-request");
    let _enter = span.enter();

    let id = super::correlation_id().expect(
        "a span entered under an OpenTelemetry layer must carry a trace id; a None here is the \
         inert-correlation defect this case exists to catch",
    );

    assert_eq!(
        id.len(),
        32,
        "the W3C trace id is 32 hex characters, and it is that rendering which keeps this value \
         grep-equal to the access log and the error envelope: got {id}"
    );
    assert!(
        id.chars().all(|c| c.is_ascii_hexdigit()),
        "a trace id rendered with hyphens joins to nothing by string equality: got {id}"
    );
    assert_ne!(
        id, "00000000000000000000000000000000",
        "the all-zero id is TraceId::INVALID rendered, which the guard must have refused"
    );
}

/// An unregistered payload type resolves to `None`, which is what makes
/// `enqueue_body` refuse it.
///
/// Asserted on a token that is *close* to a real one: a lookup written with
/// `starts_with` or `contains` would pass a bare-nonsense probe and fail here.
#[test]
fn a_payload_type_outside_the_roster_has_no_schema_reference() {
    assert_eq!(schema_ref_for("ProductCreatedV2"), None);
    assert_eq!(schema_ref_for("Product"), None);
    assert_eq!(schema_ref_for(""), None);
    assert!(schema_ref_for(PRODUCT_HEAD_SAVED_PAYLOAD_TYPE).is_some());
}

/// The four tokens the door files used to name resolve here, from the module
/// that now owns all eight.
///
/// **Two** of them actually moved — `ProductHeadSaved` from `products.rs` and
/// `SkuHeadSaved` from `skus.rs`, the only two `*_PAYLOAD_TYPE` consts those
/// files ever declared. The other two in this loop were born in `events.rs`
/// and are here as controls, so the case does not read as evidence about a
/// move that never happened. An earlier revision of this doc called all four
/// relocated and put "seven siblings" beside them; both counts were wrong.
///
/// This is the guard on the move itself: a token left behind in a door would
/// still compile there and would reach `schema_ref_for` as an unregistered
/// string at runtime.
#[test]
fn the_relocated_tokens_resolve_from_the_events_module() {
    for token in [
        PRODUCT_HEAD_SAVED_PAYLOAD_TYPE,
        SKU_HEAD_SAVED_PAYLOAD_TYPE,
        SKU_PUBLISHED_PAYLOAD_TYPE,
        SKU_DISCARDED_PAYLOAD_TYPE,
    ] {
        assert!(
            schema_ref_for(token).is_some(),
            "{token} must carry a schema reference from this module"
        );
    }
}
