//! The event envelope's own guards (`design/01-foundation.md` §4.5, P-D-01,
//! P-D-47).
//!
//! The two `partition_for` cases live here too, as of review wave D. They had
//! been inline in `events.rs`, which made that module the only one in the crate
//! carrying **both** a `#[path]` sibling test file and an `mod tests` block —
//! a second place a reader had to know to look. Beyond them, what lives here
//! is everything about the **envelope** —
//! the schema-reference roster, the envelope's own four fields (three of
//! which are P-D-01 obligations — see the case that asserts them), and the
//! shape a consumer reads.

use serde_json::Value;
use uuid::Uuid;

use super::{
    CATEGORY_DISPLAY_UPDATED_PAYLOAD_TYPE, EntityKind, EventBodyCore, EventEnvelope, PARTITIONS,
    PRODUCT_CREATED_PAYLOAD_TYPE, PRODUCT_DISCARDED_PAYLOAD_TYPE, PRODUCT_HEAD_SAVED_PAYLOAD_TYPE,
    PRODUCT_PUBLISHED_PAYLOAD_TYPE, PublishedEventBody, RetiredEventBody, SCHEMA_REFS,
    SKU_CREATED_PAYLOAD_TYPE, SKU_DISCARDED_PAYLOAD_TYPE, SKU_HEAD_SAVED_PAYLOAD_TYPE,
    SKU_PUBLISHED_PAYLOAD_TYPE, SKU_RETIRED_PAYLOAD_TYPE, TaxonomyEventBody, partition_for,
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

/// `04-lifecycle`'s announced pair, transcribed from **its** Events roster —
/// a second list, deliberately not folded into [`THE_EIGHT`].
///
/// `01` §4.5 says the `published→deprecated` edge carries *"no event here"*
/// and that 04 announces it. Adding these two names to `THE_EIGHT` would
/// make that document's own completeness claim untestable: the list would no
/// longer be a transcription of one sentence, and a Foundation event dropped
/// from §4.5 could be replaced by a lifecycle one with the count unchanged.
///
/// The deprecation pair already ships. The rest of 04's roster sits in
/// [`THE_LIFECYCLE_REST`], not here and not in [`THE_EIGHT`].
const THE_LIFECYCLE_PAIR: &[&str] = &["ProductDeprecated", "SkuDeprecated"];

/// 04's remaining five, transcribed from its Events roster. Deliberately
/// not folded into [`THE_EIGHT`] or [`THE_LIFECYCLE_PAIR`]. Scheduling
/// acts stay audit-plane: no `PublishScheduled` / `RetirementScheduled`.
/// `ProductRetired` is the initiation event; no Product flip token (row 5).
const THE_LIFECYCLE_REST: &[&str] = &[
    "ProductUndeprecated",
    "SkuUndeprecated",
    "SkuRetired",
    "ProductRetired",
    "SkuRetirementEffective",
];

/// `03-sku-classification`'s set events, transcribed from **its** §4 roster —
/// the third declared list, separate for the same reason as the second.
const THE_SET_TRIO: &[&str] = &[
    "RecognizedUnitUpdated",
    "RecognizedCodeUpdated",
    "PlanTierUpdated",
];

/// `09-bulk-promotion`'s single event, transcribed from **its** roster — a
/// fourth list, separate for the reason the second and third are: `design/09`
/// marks its other eight state-changing instructions *no event*, and folding
/// this one into any sibling roster would make that deliberate silence
/// uncountable.
const THE_BULK_SUMMARY: &[&str] = &["CatalogBulkOperationCompleted"];

/// Slice `02`'s eight (`dod-taxonomy-events`), transcribed from its §4.3
/// roster — its own list for the reason every list above is its own: folding
/// them into [`THE_EIGHT`] would claim §4.5 announces them, and §4.5
/// announces eight.
///
/// **Eight of eight since 2026-09-03.** Two were held out while §7 row 15
/// asked which aggregate orders `CategoryDisplayUpdated` and
/// `AttributeDefinitionUpdated`; **P-D-116** row 15 answered *their own
/// entity's id*, and both joined with that aggregate. All eight are emitted
/// through `enqueue_taxonomy`, inside their acts' transactions.
const THE_TAXONOMY_EIGHT: &[&str] = &[
    "CategoryCreated",
    "CategoryRenamed",
    "CategoryReparented",
    "CategoryRetired",
    "CategoryDeleted",
    "CategoryDisplayUpdated",
    "AttributeDefinitionUpdated",
    "MetadataUpdated",
];

/// Slice `10`'s two (`dod-retention-events`), transcribed from its §4
/// roster — a sixth list for every list above's reason. Folding them into
/// [`THE_EIGHT`] would claim §4.5 announces them, and §4.5 announces eight;
/// folding them into [`THE_TAXONOMY_EIGHT`] would claim `design/02` does.
///
/// Neither carries an entity core and neither subject is an entity: an
/// `actor_ref` and an allow-list entry have none of `EventBodyCore`'s five
/// fields, and `EntityKind` is exactly `Product | Sku`. Both go through
/// `enqueue_retention` with `RetentionEventBody`.
const THE_RETENTION_PAIR: &[&str] = &["ActorErased", "PiiAllowlistChanged"];

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

/// **Every declared token belongs to exactly one entry point** — the
/// partition the four `enqueue*` guards implement, asserted as a partition
/// rather than arm by arm.
///
/// Each entry point owns one body shape and refuses every token it does not
/// build. Checking that partition here catches the failure the guards were
/// each added for: a token no guard claims falls through to the entry point
/// it was passed to, and on the interim sink writes a body of the wrong
/// shape while the broker sink refuses it. Two of the four arms were added
/// by review passes after exactly that divergence shipped — the deprecation
/// pair's, then the set trio's — so the invariant is stated once, over the
/// whole roster, instead of once per arm.
#[test]
fn every_declared_token_belongs_to_exactly_one_entry_point() {
    let published = ["ProductPublished", "SkuPublished"];
    let deprecated = THE_LIFECYCLE_PAIR;
    let set_events = THE_SET_TRIO;
    let bulk = THE_BULK_SUMMARY;
    let taxonomy = THE_TAXONOMY_EIGHT;
    let retention = THE_RETENTION_PAIR;

    for (token, _) in SCHEMA_REFS {
        let owners = usize::from(published.contains(token))
            + usize::from(deprecated.contains(token))
            + usize::from(set_events.contains(token))
            + usize::from(bulk.contains(token))
            + usize::from(taxonomy.contains(token))
            + usize::from(retention.contains(token));
        assert!(
            owners <= 1,
            "{token} is claimed by more than one entry point's guard"
        );
        // Zero owners is the core-only default: `enqueue` builds that shape,
        // so a token no specialised guard claims is legitimately its own.
        let core_only = owners == 0;
        let rest = THE_LIFECYCLE_REST.contains(token);
        assert_eq!(
            core_only,
            (THE_EIGHT.contains(token) && !published.contains(token)) || rest,
            "{token}: the core-only set must be exactly §4.5's eight minus the publish pair — \
             a declared token outside that partition reaches an entry point whose body shape it \
             does not have; 04's remaining five are registered ahead of their enqueue"
        );
    }
}

/// **Every declared event has a versioned schema reference — §4.5's eight,
/// 04's announced pair and 03's set trio — and nothing else does.**
///
/// Both directions matter. A missing entry is an event that would be refused
/// at its first enqueue; a *surplus* entry is a schema reference announced for
/// an event this gear does not emit, which a consumer contract would take for
/// a promise.
#[test]
fn the_schema_roster_names_exactly_the_declared_events() {
    let registered: Vec<&str> = SCHEMA_REFS.iter().map(|(token, _)| *token).collect();

    for event in THE_EIGHT {
        assert!(
            registered.contains(event),
            "{event} is one of §4.5's eight and carries no schema reference"
        );
    }
    for event in THE_LIFECYCLE_PAIR {
        assert!(
            registered.contains(event),
            "{event} is announced by 04 and carries no schema reference"
        );
    }
    for event in THE_LIFECYCLE_REST {
        assert!(
            registered.contains(event),
            "{event} is announced by 04 and carries no schema reference"
        );
    }
    for event in THE_SET_TRIO {
        assert!(
            registered.contains(event),
            "{event} is 03's set event and carries no schema reference"
        );
    }
    for event in THE_BULK_SUMMARY {
        assert!(
            registered.contains(event),
            "{event} is 09's only event and carries no schema reference"
        );
    }
    for event in THE_TAXONOMY_EIGHT {
        assert!(
            registered.contains(event),
            "{event} is 02's taxonomy event and carries no schema reference"
        );
    }
    for event in THE_RETENTION_PAIR {
        assert!(
            registered.contains(event),
            "{event} is 10's retention event and carries no schema reference"
        );
    }
    for token in &registered {
        assert!(
            THE_EIGHT.contains(token)
                || THE_LIFECYCLE_PAIR.contains(token)
                || THE_LIFECYCLE_REST.contains(token)
                || THE_SET_TRIO.contains(token)
                || THE_BULK_SUMMARY.contains(token)
                || THE_TAXONOMY_EIGHT.contains(token)
                || THE_RETENTION_PAIR.contains(token),
            "{token} carries a schema reference and belongs to no declared roster: an \
             event no design document announces is a promise nothing backs"
        );
    }
    assert_eq!(
        registered.len(),
        THE_EIGHT.len()
            + THE_LIFECYCLE_PAIR.len()
            + THE_LIFECYCLE_REST.len()
            + THE_SET_TRIO.len()
            + THE_BULK_SUMMARY.len()
            + THE_TAXONOMY_EIGHT.len()
            + THE_RETENTION_PAIR.len(),
        "the roster must carry each declared event exactly once"
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
/// envelope field at all — it is the partition, judged by the two
/// `partition_for` cases at the foot of this file and, for the guarantee a
/// consumer actually gets, owned by the
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

/// **A taxonomy body renders flat under `data`, omits its absent optionals,
/// and carries its own versioned schema reference** (P-D-122).
///
/// `mutationSeq` present and `operationKind` absent is the display door's
/// shape exactly: it spends a token and rides no envelope.
#[test]
fn a_taxonomy_body_renders_flat_and_omits_its_absent_optionals() {
    let body = TaxonomyEventBody {
        tenant_id: Uuid::from_u128(0x7e_42),
        entity_kind: "category",
        entity_id: Uuid::from_u128(0x_1111),
        act: "display_updated",
        state: "active",
        mutation_seq: Some(4),
        operation_kind: None,
    };
    let json = rendered(&body, CATEGORY_DISPLAY_UPDATED_PAYLOAD_TYPE);
    assert_eq!(json["data"]["entityKind"], "category");
    assert_eq!(json["data"]["act"], "display_updated");
    assert_eq!(json["data"]["state"], "active");
    assert_eq!(json["data"]["mutationSeq"], 4);
    assert!(
        json["data"].get("operationKind").is_none(),
        "absence is omitted, not null"
    );
    assert_eq!(
        json["schemaRef"],
        "bss-products.CategoryDisplayUpdated.v1.0.0"
    );
}

/// v1 retirement omits `mustMigrateBy` and a Product-side `replacedBy`.
/// A null would not round-trip absence.
#[test]
fn a_retirement_body_omits_must_migrate_by_when_absent() {
    let core = core();
    let body = RetiredEventBody {
        core: &core,
        from_version: 3,
        reason: "end of sale".to_owned(),
        replaced_by: None,
        effective_at: "2026-10-01T00:00:00Z".to_owned(),
        must_migrate_by: None,
    };
    let json = rendered(&body, SKU_RETIRED_PAYLOAD_TYPE);
    assert_eq!(json["data"]["fromVersion"], 3);
    assert_eq!(json["data"]["reason"], "end of sale");
    assert_eq!(json["data"]["effectiveAt"], "2026-10-01T00:00:00Z");
    assert!(
        json["data"].get("mustMigrateBy").is_none(),
        "v1 must omit mustMigrateBy, not emit null"
    );
    assert!(
        json["data"].get("replacedBy").is_none(),
        "a Product initiation (and an unset SKU replacement) omit replacedBy"
    );
    assert_eq!(json["schemaRef"], "bss-products.SkuRetired.v1.0.0");
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

/// `partition_for` must never answer outside `[0, PARTITIONS)` — an
/// out-of-range answer would make `Outbox::enqueue`'s own
/// `resolve_partition` fail every call with `PartitionOutOfRange`,
/// turning every create into a `500` regardless of anything the door
/// itself does right.
#[test]
fn partition_for_answers_within_the_configured_partition_count() {
    for _ in 0..64 {
        let tenant_id = Uuid::new_v4();
        let aggregate_id = Uuid::new_v4();
        let partition = partition_for(tenant_id, aggregate_id);
        assert!(
            partition < u32::from(PARTITIONS),
            "partition {partition} must be < PARTITIONS ({PARTITIONS})"
        );
    }
}

/// **The formula's output is pinned, not merely its determinism.**
///
/// An earlier revision asserted `partition_for(t, a) == partition_for(t, a)` —
/// a pure function called twice with the same arguments — and called that "the
/// entire property P-D-22 asks the formula for". It is not: that assertion
/// cannot fail for any implementation that does not read the clock. Drop the
/// `.rotate_left(64)`, swap `^` for `+`, or reorder the operands, and it stays
/// green while **every aggregate in a running deployment moves partition**,
/// which is the ordering guarantee P-D-22 exists to give and what
/// [`PARTITIONS`]' own doc calls "silently reassigning aggregates to different
/// partitions".
///
/// So the expected values are a golden vector: transcribed once, from the
/// formula as it stands at `PARTITIONS = 8`, and never recomputed from the
/// function. A formula change reddens here, which is the point; if the change
/// is deliberate the vector is re-transcribed **and** the deployment needs a
/// migration story, because live aggregates change partition.
#[test]
fn partition_for_is_pinned_to_a_golden_vector() {
    const GOLDEN: &[(u128, u128, u32)] = &[
        (0x7e42, 0x1111, 2),
        (0x1, 0x2, 1),
        (0xdead_beef, 0xcafe_babe, 7),
    ];
    assert_eq!(PARTITIONS, 8, "the vector below was taken at N = 8");
    for (tenant, aggregate, want) in GOLDEN {
        let got = partition_for(Uuid::from_u128(*tenant), Uuid::from_u128(*aggregate));
        assert_eq!(
            got, *want,
            "partition_for({tenant:#x}, {aggregate:#x}) moved from {want} to {got}; every \
             aggregate of that tenant would change partition on deploy"
        );
    }
}

/// The same `(tenant_id, aggregate_id)` pair must always land on the/// The same `(tenant_id, aggregate_id)` pair must always land on the
/// same partition — this is the entire property P-D-22 asks the formula
/// for: every event of one aggregate keeps its relative order because
/// they all land in one partition.
#[test]
fn partition_for_is_deterministic_for_the_same_pair() {
    let tenant_id = Uuid::new_v4();
    let aggregate_id = Uuid::new_v4();
    assert_eq!(
        partition_for(tenant_id, aggregate_id),
        partition_for(tenant_id, aggregate_id),
        "the same (tenant_id, aggregate_id) pair must hash to the same partition every time"
    );
}
