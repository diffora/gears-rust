//! The event envelope's own guards (`design/01-foundation.md` §4.5, P-D-01,
//! P-D-47).
//!
//! The two `partition_for` cases stay inline in `events.rs` beside the
//! formula they judge; what lives here is everything about the **envelope** —
//! the schema-reference roster, the four obligations P-D-01 names, and the
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

/// **The four obligations P-D-01 names are on the wire**, under the names a
/// consumer reads them by.
///
/// This is the whole of `dod-outbox-eventing`'s envelope clause, asserted on
/// the rendering rather than on the struct: a field renamed by a stray
/// `#[serde(rename)]`, or dropped by a `skip_serializing_if` that fires when
/// it should not, is invisible to a test that reads the Rust value.
#[test]
fn the_envelope_carries_the_four_obligations_and_the_body_beneath_them() {
    let core = core();
    let json = rendered(&core, PRODUCT_CREATED_PAYLOAD_TYPE);

    assert_eq!(
        json["eventId"], "00000000-0000-0000-0000-00000000e0e0",
        "P-D-47's idempotency key is the event's own id"
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

/// The four tokens that spent two phases in the door files resolve here, from
/// the module their seven siblings live in.
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
