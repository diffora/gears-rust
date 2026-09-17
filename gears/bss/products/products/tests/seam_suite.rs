//! The seam suite (`design/12` §2.1 `inst-ss-home` / `inst-ss-pin` /
//! `inst-ss-fixtures`; P-D-130, P-D-132, P-D-151) — the products-side home of
//! the joint checks, **run on demand**:
//!
//! ```text
//! cargo test -p cf-gears-bss-products --test seam_suite
//! ```
//!
//! # Why this target, and not a crate or a CI job
//!
//! P-D-130 moved the suite out of `cf-gears-bss-fixtures` (pricing's closed
//! grammar) to the products side; P-D-132 is the owner's refusal of a CI job.
//! What is left is a test target that both gears' SDKs and the `SchemaPin`
//! reach: this file takes `bss-pricing-sdk` and `toml` as **dev**-dependencies
//! (the gear takes no production dependency on pricing), lives in a
//! traceability root the gate scans (`<crate>/tests`), and runs with the
//! gear's own `cargo test`. `dod-seam-suite-home` stays unticked by P-D-132's
//! reason — its CI clause is unsatisfiable by decision — and this is the home
//! the decision left.
//!
//! # The two-sided pin check (P-D-57)
//!
//! For every `field` member of `schema-pin.toml`:
//!
//! - a member flagged **comparable** must be present on **both** SDK types under
//!   the field names the pin records — a rename on either side fails here;
//! - a member flagged **not yet comparable** must **not** be present on both —
//!   the day the second side ships it, the flag must flip, and the change that
//!   shipped it fails until it does. That is what keeps the flag from rotting
//!   into a standing excuse.
//!
//! Presence is exhaustive by construction: the two field lists below are
//! destructured against the real types, so a field added to either SDK type
//! fails to compile here until the list carries it.
//!
//! # The joint fixtures (`inst-ss-fixtures`, C4)
//!
//! Six fixtures are named; **one is authorable** today and the other five are
//! OWED, each with its measured reason (`OWED_FIXTURES`). C4 admits a fixture
//! only when the counterpart raises the code the fixture asserts, and a
//! vacuously green fixture is worse than an absent one, so none of the five is
//! written as a green check. **Their registry-side halves are** (P-D-160): one
//! `#[ignore]`d test each, compiled against both SDKs, doing the registry's
//! setup and ending on the counterpart's absence with the ask in the ignore
//! reason — so the joint build starts from a half that already runs. Re-measured at `14344c110` against pricing's tree: no watermark
//! producer, no consumer of any registry event, no adoption-guard code raised
//! (`SKU_NOT_PUBLISHED` is named and not raised), no meter-binding rule.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-contract-seams:p1

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use bss_pricing_sdk::product_catalog::CatalogSku;
use bss_products_sdk::models::{LifecycleState, Sku};
use time::OffsetDateTime;

// ---------------------------------------------------------------------------
// The two SDK shapes, exhaustively
// ---------------------------------------------------------------------------

/// The registry side's read shape, field by field. Destructured against the
/// type so the list cannot fall behind it.
const REGISTRY_FIELDS: &[&str] = &[
    "sku_id",
    "tenant_id",
    "product_id",
    "sku_code",
    "lifecycle_state",
    "internal_revision",
    "published_version",
    "sku_type",
    "sellable",
    "composition_pending",
    "plan_tier",
    "metering_unit",
    "usage_type_ref",
];

/// The consumer side's read shape (pricing's `CatalogSku`), field by field.
const CONSUMER_FIELDS: &[&str] = &[
    "sku_id",
    "sku_code",
    "name",
    "metering_unit",
    "status",
    "plan_tier",
    "sku_type",
    "sellable",
    "usage_type_ref",
    "deprecated",
];

#[allow(clippy::no_effect_underscore_binding, unused_variables)]
fn the_lists_are_exhaustive(registry: Sku, consumer: CatalogSku) {
    // Without `..`: a field added to either type fails to compile here.
    let Sku {
        sku_id,
        tenant_id,
        product_id,
        sku_code,
        lifecycle_state,
        internal_revision,
        published_version,
        sku_type,
        sellable,
        composition_pending,
        plan_tier,
        metering_unit,
        usage_type_ref,
    } = registry;
    let CatalogSku {
        sku_id,
        sku_code,
        name,
        metering_unit,
        status,
        plan_tier,
        sku_type,
        sellable,
        usage_type_ref,
        deprecated,
    } = consumer;
}

#[test]
fn the_field_lists_mirror_the_two_types() {
    // Thirteen bindings above, thirteen names; ten and ten. It was fifteen
    // until P-D-169 took the two accounting codes off the registry's shape.
    assert_eq!(REGISTRY_FIELDS.len(), 13);
    assert_eq!(CONSUMER_FIELDS.len(), 10);
    let _ = the_lists_are_exhaustive;
}

// ---------------------------------------------------------------------------
// The pin
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Deserialize)]
struct Pin {
    version: u32,
    member: Vec<Member>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct Member {
    name: String,
    kind: String,
    #[serde(default)]
    comparable: bool,
    #[serde(rename = "registry-field")]
    registry_field: Option<String>,
    #[serde(rename = "consumer-field")]
    consumer_field: Option<String>,
    #[serde(default)]
    vocabulary: Vec<String>,
}

fn pin_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../products-sdk/schema-pin.toml")
}

fn read_pin() -> Pin {
    let raw = std::fs::read_to_string(pin_path()).expect("schema-pin.toml beside the SDK");
    toml::from_str(&raw).expect("the pin parses")
}

fn snake(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// The two-sided check, as findings.
fn pin_findings(pin: &Pin, registry: &[&str], consumer: &[&str]) -> Vec<String> {
    let mut f = Vec::new();
    for m in pin.member.iter().filter(|m| m.kind == "field") {
        let reg = m.registry_field.clone().unwrap_or_else(|| snake(&m.name));
        let con = m.consumer_field.clone().unwrap_or_else(|| snake(&m.name));
        let on_registry = registry.contains(&reg.as_str());
        let on_consumer = consumer.contains(&con.as_str());
        if m.comparable {
            if !on_registry {
                f.push(format!(
                    "{}: comparable, but the registry shape has no `{reg}`",
                    m.name
                ));
            }
            if !on_consumer {
                f.push(format!(
                    "{}: comparable, but the consumer shape has no `{con}`",
                    m.name
                ));
            }
        } else if on_registry && on_consumer {
            f.push(format!(
                "{}: flagged not-yet-comparable while both shapes carry it (`{reg}` / `{con}`) — flip the flag in the change that shipped the second side",
                m.name
            ));
        }
    }
    f
}

#[test]
fn every_comparable_member_is_on_both_shapes_and_no_shipped_member_hides_behind_the_flag() {
    let pin = read_pin();
    assert_eq!(pin.version, 1);
    let findings = pin_findings(&pin, REGISTRY_FIELDS, CONSUMER_FIELDS);
    assert!(
        findings.is_empty(),
        "the pin diverges from the shapes: {findings:#?}"
    );
    let comparable: BTreeSet<&str> = pin
        .member
        .iter()
        .filter(|m| m.kind == "field" && m.comparable)
        .map(|m| m.name.as_str())
        .collect();
    assert_eq!(
        comparable,
        BTreeSet::from([
            "skuId",
            "type",
            "unit",
            "usageTypeRef",
            "PlanTier",
            "status",
            "sellable"
        ]),
        "seven members compare today; compositionPending waits on pricing's side"
    );
}

#[test]
fn the_pin_divergence_red_fails_on_one_side_only() {
    let mut pin = read_pin();
    // Rename the consumer side of a comparable member: the registry still has
    // it, the consumer does not — the asymmetry is the enforcement.
    let m = pin
        .member
        .iter_mut()
        .find(|m| m.name == "sellable")
        .unwrap();
    m.consumer_field = Some("is_sellable".to_owned());
    let f = pin_findings(&pin, REGISTRY_FIELDS, CONSUMER_FIELDS);
    assert_eq!(f.len(), 1, "{f:?}");
    assert!(f[0].contains("consumer shape has no `is_sellable`"));
    // Flag a shipped member not-yet-comparable: the flag rotted.
    let mut pin = read_pin();
    pin.member
        .iter_mut()
        .find(|m| m.name == "type")
        .unwrap()
        .comparable = false;
    let f = pin_findings(&pin, REGISTRY_FIELDS, CONSUMER_FIELDS);
    assert_eq!(f.len(), 1, "{f:?}");
    assert!(f[0].contains("flip the flag"));
}

#[test]
fn the_status_vocabulary_is_the_registrys_wire_subset() {
    let pin = read_pin();
    let status = pin
        .member
        .iter()
        .find(|m| m.name == "status")
        .expect("status is pinned");
    assert_eq!(status.registry_field.as_deref(), Some("lifecycle_state"));
    assert_eq!(status.consumer_field.as_deref(), Some("status"));
    assert_eq!(status.vocabulary, ["published", "deprecated"]);
    for word in &status.vocabulary {
        let state = LifecycleState::parse(word).expect("a registry state");
        assert!(
            matches!(
                state,
                LifecycleState::Published | LifecycleState::Deprecated
            ),
            "the browse wire subset"
        );
    }
}

// ---------------------------------------------------------------------------
// The sixth fixture: the studio single-inbox envelope
// ---------------------------------------------------------------------------

fn struct_fields(source: &str, name: &str) -> Vec<String> {
    let start = source
        .find(&format!("pub struct {name} {{"))
        .unwrap_or_else(|| panic!("`{name}` is declared"));
    let body = &source[start..];
    let end = body.find("\n}").expect("the struct closes");
    body[..end]
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            l.strip_prefix("pub ")
                .and_then(|rest| rest.split(':').next())
                .map(str::to_owned)
        })
        .collect()
}

/// The common inbox envelope both gears render today — the intersection
/// measured at `14344c110`. Pricing spells the submitter `submitter_principal`
/// and carries no `quorum`; the registry spells `submitter` and carries the
/// quorum card `design/05` `inst-gv-queue` requires. That divergence is filed
/// (P-D-151, `features/consumer-contracts.md` §7), and this fixture pins what
/// the two shapes agree on so a further drift on either side fails here.
const COMMON_ENVELOPE: &[&str] = &[
    "approval_id",
    "subject_ref",
    "subject_kind",
    "state",
    "submitted_at",
];

#[test]
fn the_inbox_envelope_cross_check_pins_the_shared_fields() {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let registry =
        std::fs::read_to_string(here.join("src/api/rest/approvals.rs")).expect("own door");
    let pricing =
        std::fs::read_to_string(here.join("../../pricing/pricing/src/api/rest/approvals.rs"))
            .expect("pricing's door is in the same workspace");
    let card = struct_fields(&registry, "ApprovalInboxCard");
    let view = struct_fields(&pricing, "ApprovalView");
    for field in COMMON_ENVELOPE {
        assert!(
            card.iter().any(|f| f == field),
            "the registry card lost `{field}`: {card:?}"
        );
        assert!(
            view.iter().any(|f| f == field),
            "pricing's view lost `{field}`: {view:?}"
        );
    }
    // The measured divergence, held so a silent convergence is noticed too.
    assert!(card.iter().any(|f| f == "submitter") && card.iter().any(|f| f == "quorum"));
    assert!(view.iter().any(|f| f == "submitter_principal") && !view.iter().any(|f| f == "quorum"));
}

// ---------------------------------------------------------------------------
// The five OWED fixtures
// ---------------------------------------------------------------------------

/// `(fixture, register row, why it is not authorable)`, measured against
/// pricing's tree at `14344c110`.
const OWED_FIXTURES: &[(&str, &str, &str)] = &[
    (
        "watermark",
        "Produce the `SkuReferenceCount` watermark",
        "pricing has no watermark producer: no type or call site names SkuReferenceCount or WatermarkPost",
    ),
    (
        "adoption-block",
        "Refuse adoption of `deprecated` SKUs",
        "pricing's plan rules name SKU_NOT_PUBLISHED and its own module doc says it is not raised anywhere",
    ),
    (
        "usage-binding",
        "Usage-binding checks",
        "pricing raises neither METER_USAGE_TYPE_UNBOUND nor METER_DIMENSION_UNDECLARED; its foundation calls the binding deferred",
    ),
    (
        "grandfathered-resolution",
        "Resolve grandfathered refs against the frozen snapshot",
        "pricing's only CatalogVersionRegistryV1 implementors are local-dev and test stubs; no posted-use path resolves a frozen snapshot",
    ),
    (
        "correction",
        "Re-validate on `SkuImmutableFieldCorrected`",
        "pricing consumes no registry event: no type or call site names a bss-products payload",
    ),
];

#[test]
fn the_owed_fixtures_are_five_and_each_names_its_measured_reason() {
    assert_eq!(OWED_FIXTURES.len(), 5);
    let names: BTreeSet<&str> = OWED_FIXTURES.iter().map(|(n, _, _)| *n).collect();
    assert_eq!(names.len(), 5, "five distinct fixtures");
    for (_, row, why) in OWED_FIXTURES {
        assert!(!row.is_empty() && why.len() > 40, "a reason, not a shrug");
    }
}

// ---------------------------------------------------------------------------
// The products-side halves of the five OWED fixtures (P-D-160)
// ---------------------------------------------------------------------------
//
// Each half does what the registry side of the fixture will do — builds the
// shape it posts or reads, checks the pin and the roster the fixture leans
// on — and then stops at the counterpart's absence with the ask spelled out.
// `#[ignore]`d, with the ask in the reason, so `--ignored` runs them and shows
// exactly what pricing owes; the ignore comes off the day the counterpart
// raises its code (C4), and the `panic!` at the end becomes the assertion.

fn pin_member(pin: &Pin, name: &str) -> Option<bool> {
    pin.member
        .iter()
        .find(|m| m.name == name)
        .map(|m| m.comparable)
}

fn system_ctx() -> toolkit_security::SecurityContext {
    toolkit_security::SecurityContext::builder()
        .subject_id(uuid::Uuid::now_v7())
        .subject_type("bss-products.seam-suite")
        .subject_tenant_id(uuid::Uuid::nil())
        .build()
        .expect("both required builder fields are set")
}

/// The watermark fixture, registry half: the post pricing's producer will make
/// carries exactly `producer`, `watermark_at` and `sku_ids`, the port answers
/// an error while unconfigured (never a silent ack), and `skuId` is pinned
/// comparable — the operand the register names.
#[tokio::test]
#[ignore = "pricing ask (P-D-160, row 1): a SkuReferenceCount producer calling WatermarkPosts::post per watermark — pricing has no producer type or call site"]
async fn watermark_fixture_registry_half() {
    use bss_products_sdk::watermarks::{
        UnconfiguredWatermarkPosts, WatermarkPost, WatermarkPosts as _,
    };
    let post = WatermarkPost {
        producer: "pricing".to_owned(),
        watermark_at: OffsetDateTime::now_utc(),
        sku_ids: vec![uuid::Uuid::now_v7()],
    };
    let answer = UnconfiguredWatermarkPosts
        .post(&system_ctx(), uuid::Uuid::now_v7(), post)
        .await;
    assert!(
        answer.is_err(),
        "an unconfigured port refuses; it never acks"
    );
    assert_eq!(pin_member(&read_pin(), "skuId"), Some(true));
    panic!("counterpart absent: pricing produces no SkuReferenceCount watermark (register row 13)");
}

/// The adoption-block fixture, registry half: a `deprecated` SKU as pricing
/// reads it, under the pinned two-value `status` vocabulary; the fixture's
/// assertion will be pricing refusing the adoption at retirement or
/// unpublishing with `SKU_NOT_PUBLISHED`.
#[tokio::test]
#[ignore = "pricing ask (P-D-160, row 2): raise SKU_NOT_PUBLISHED on adopting a deprecated SKU at retirement or unpublishing (pricing AC #82) — named in pricing's plan rules, raised nowhere"]
async fn adoption_block_fixture_registry_half() {
    let deprecated = CatalogSku {
        sku_id: uuid::Uuid::now_v7(),
        sku_code: "FIBRE-500-1".to_owned(),
        name: "Fibre 500".to_owned(),
        metering_unit: None,
        status: LifecycleState::Deprecated.as_str().to_owned(),
        plan_tier: None,
        sku_type: "product".to_owned(),
        sellable: true,
        usage_type_ref: None,
        deprecated: false,
    };
    assert_eq!(
        deprecated.status, "deprecated",
        "the wire subset's second value"
    );
    assert_eq!(pin_member(&read_pin(), "status"), Some(true));
    panic!("counterpart absent: pricing raises SKU_NOT_PUBLISHED nowhere (register row 2)");
}

/// The usage-binding fixture, registry half: the meter pair as pricing reads
/// it, both members pinned comparable; the fixture's assertion will be
/// pricing's binding rule refusing an unbound meter and a `deprecated` bound
/// unit.
#[tokio::test]
#[ignore = "pricing ask (P-D-160, row 3): a meter-binding rule raising METER_USAGE_TYPE_UNBOUND / METER_DIMENSION_UNDECLARED and judging a deprecated bound unit — pricing's foundation calls the binding deferred"]
async fn usage_binding_fixture_registry_half() {
    let usage = CatalogSku {
        sku_id: uuid::Uuid::now_v7(),
        sku_code: "STORAGE-GIB".to_owned(),
        name: "Storage".to_owned(),
        metering_unit: Some("gib_month".to_owned()),
        status: "published".to_owned(),
        plan_tier: Some("standard".to_owned()),
        sku_type: "product".to_owned(),
        sellable: true,
        usage_type_ref: Some("usage:storage".to_owned()),
        deprecated: false,
    };
    assert!(
        usage.metering_unit.is_some() && usage.usage_type_ref.is_some(),
        "the pair travels together"
    );
    let pin = read_pin();
    assert_eq!(pin_member(&pin, "unit"), Some(true));
    assert_eq!(pin_member(&pin, "usageTypeRef"), Some(true));
    panic!("counterpart absent: pricing has no meter-binding rule (register rows 5 and 6)");
}

/// The grandfathered-resolution fixture, registry half: pricing's own port for
/// the frozen snapshot exists and refuses while unconfigured, and
/// `CatalogVersion` is pinned as a surface; the fixture's assertion will be a
/// byte-identical re-resolution after registry churn through a real
/// implementor.
#[tokio::test]
#[ignore = "pricing ask (P-D-160, row 4): a CatalogVersionRegistryV1 implementor on the posted-use path — pricing has only local-dev and test stubs"]
async fn grandfathered_resolution_fixture_registry_half() {
    use bss_pricing_sdk::catalog_version_registry::{
        CatalogVersionRegistryV1 as _, UnconfiguredCatalogVersionRegistryV1,
    };
    let answer = UnconfiguredCatalogVersionRegistryV1
        .committed_version(&system_ctx(), "pending:1")
        .await;
    assert!(answer.is_err(), "an unconfigured registry port refuses");
    let pin = read_pin();
    assert!(
        pin.member
            .iter()
            .any(|m| m.name == "CatalogVersion" && m.kind == "surface"),
        "the surface entry the fixture rides on"
    );
    panic!("counterpart absent: no posted-use path resolves a frozen snapshot (register row 7)");
}

/// The correction fixture, registry half: `SkuImmutableFieldCorrected` is on
/// the versioned event roster with its schema reference; the fixture's
/// assertion will be pricing re-validating on it.
#[tokio::test]
#[ignore = "pricing ask (P-D-160, row 5): consume SkuImmutableFieldCorrected and re-validate (07 inst-cr-republish) — pricing consumes no registry event"]
async fn correction_fixture_registry_half() {
    let roster = bss_products_sdk::events::SCHEMA_REFS;
    let entry = roster
        .iter()
        .find(|(name, _)| *name == "SkuImmutableFieldCorrected")
        .expect("the correction event is on the roster");
    assert!(
        entry.1.ends_with(".v1.0.0"),
        "a semver schema reference: {}",
        entry.1
    );
    panic!("counterpart absent: pricing consumes no bss-products event (register row 8)");
}

/// The five asks are filed where pricing's owner will read them: `design/12`
/// §2.2's counterpart table names each fixture, and pricing's own register
/// carries the section (P-D-160).
#[test]
fn the_counterpart_asks_are_filed_on_both_sides() {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let design = std::fs::read_to_string(here.join("../docs/design/12-consumer-contracts.md"))
        .expect("design/12");
    let asks = design
        .find("#### Counterpart asks (P-D-160)")
        .expect("the counterpart table is filed");
    let table = &design[asks..];
    let pricing = std::fs::read_to_string(here.join("../../pricing/docs/DECISIONS.md"))
        .expect("pricing's register is in the same workspace");
    let filed = pricing
        .find("Asks from the products gear")
        .expect("pricing's register carries the asks");
    for (name, _, _) in OWED_FIXTURES {
        assert!(
            table.contains(name),
            "design/12 names the `{name}` fixture's ask"
        );
        assert!(
            pricing[filed..].contains(name),
            "pricing's register names `{name}`"
        );
    }
}

// ---------------------------------------------------------------------------
// Task 8: in-process provide, no `client_wiring`
// ---------------------------------------------------------------------------

/// Config with no gear sections: `config_or_default` is `ProductsConfig::
/// default()`, and `read_wiring` sees no `client_wiring` → `Local`.
struct EmptyConfig;

impl toolkit::config::ConfigProvider for EmptyConfig {
    fn get_gear_config(&self, _gear_name: &str) -> Option<&serde_json::Value> {
        None
    }
}

/// Permissive PDP so `init` can build a `PolicyEnforcer`.
struct AllowResolver;

#[async_trait::async_trait]
impl authz_resolver_sdk::AuthZResolverApi for AllowResolver {
    async fn evaluate(
        &self,
        _ctx: toolkit_security::PlatformSecurityContext,
        _req: authz_resolver_sdk::models::EvaluationRequest,
    ) -> Result<
        authz_resolver_sdk::models::EvaluationResponse,
        toolkit_canonical_errors::CanonicalError,
    > {
        Ok(authz_resolver_sdk::models::EvaluationResponse {
            decision: true,
            context: authz_resolver_sdk::models::EvaluationResponseContext {
                constraints: Vec::new(),
                deny_reason: None,
            },
        })
    }
}

/// Accepts `register` (the only call `init` makes). Other methods are unused.
struct AcceptingTypesRegistry;

#[async_trait::async_trait]
impl types_registry_sdk::TypesRegistryClient for AcceptingTypesRegistry {
    async fn register(
        &self,
        entities: Vec<serde_json::Value>,
    ) -> Result<Vec<types_registry_sdk::RegisterResult>, toolkit_canonical_errors::CanonicalError>
    {
        Ok(entities
            .iter()
            .map(|_| types_registry_sdk::RegisterResult::Ok {
                gts_id: "ok".to_owned(),
            })
            .collect())
    }

    async fn register_type_schemas(
        &self,
        _type_schemas: Vec<serde_json::Value>,
    ) -> Result<Vec<types_registry_sdk::RegisterResult>, toolkit_canonical_errors::CanonicalError>
    {
        unimplemented!("seam boot only calls register")
    }

    async fn get_type_schema(
        &self,
        _type_id: &str,
    ) -> Result<types_registry_sdk::models::GtsTypeSchema, toolkit_canonical_errors::CanonicalError>
    {
        unimplemented!("seam boot only calls register")
    }

    async fn get_type_schema_by_uuid(
        &self,
        _type_uuid: uuid::Uuid,
    ) -> Result<types_registry_sdk::models::GtsTypeSchema, toolkit_canonical_errors::CanonicalError>
    {
        unimplemented!("seam boot only calls register")
    }

    async fn get_type_schemas(
        &self,
        _type_ids: Vec<String>,
    ) -> std::collections::HashMap<
        String,
        Result<types_registry_sdk::models::GtsTypeSchema, toolkit_canonical_errors::CanonicalError>,
    > {
        unimplemented!("seam boot only calls register")
    }

    async fn get_type_schemas_by_uuid(
        &self,
        _type_uuids: Vec<uuid::Uuid>,
    ) -> std::collections::HashMap<
        uuid::Uuid,
        Result<types_registry_sdk::models::GtsTypeSchema, toolkit_canonical_errors::CanonicalError>,
    > {
        unimplemented!("seam boot only calls register")
    }

    async fn list_type_schemas(
        &self,
        _query: types_registry_sdk::models::TypeSchemaQuery,
    ) -> Result<
        Vec<types_registry_sdk::models::GtsTypeSchema>,
        toolkit_canonical_errors::CanonicalError,
    > {
        unimplemented!("seam boot only calls register")
    }

    async fn register_instances(
        &self,
        _instances: Vec<serde_json::Value>,
    ) -> Result<Vec<types_registry_sdk::RegisterResult>, toolkit_canonical_errors::CanonicalError>
    {
        unimplemented!("seam boot only calls register")
    }

    async fn get_instance(
        &self,
        _id: &str,
    ) -> Result<types_registry_sdk::models::GtsInstance, toolkit_canonical_errors::CanonicalError>
    {
        unimplemented!("seam boot only calls register")
    }

    async fn get_instance_by_uuid(
        &self,
        _uuid: uuid::Uuid,
    ) -> Result<types_registry_sdk::models::GtsInstance, toolkit_canonical_errors::CanonicalError>
    {
        unimplemented!("seam boot only calls register")
    }

    async fn get_instances(
        &self,
        _ids: Vec<String>,
    ) -> std::collections::HashMap<
        String,
        Result<types_registry_sdk::models::GtsInstance, toolkit_canonical_errors::CanonicalError>,
    > {
        unimplemented!("seam boot only calls register")
    }

    async fn get_instances_by_uuid(
        &self,
        _uuids: Vec<uuid::Uuid>,
    ) -> std::collections::HashMap<
        uuid::Uuid,
        Result<types_registry_sdk::models::GtsInstance, toolkit_canonical_errors::CanonicalError>,
    > {
        unimplemented!("seam boot only calls register")
    }

    async fn list_instances(
        &self,
        _query: types_registry_sdk::models::InstanceQuery,
    ) -> Result<
        Vec<types_registry_sdk::models::GtsInstance>,
        toolkit_canonical_errors::CanonicalError,
    > {
        unimplemented!("seam boot only calls register")
    }
}

/// Boot products with no `client_wiring` section: the hub carries a local
/// `ProductCatalogClientV1` that answers a projected SKU.
#[tokio::test]
async fn products_boots_without_client_wiring_and_hub_answers_a_projected_sku() {
    use bss_pricing_sdk::product_catalog::ProductCatalogClientV1;
    use bss_products::gear::BssProductsGear;
    use bss_products::infra::storage::entity::read_entity;
    use sea_orm::EntityTrait;
    use std::sync::Arc;
    use toolkit::contracts::DatabaseCapability;
    use toolkit::{ClientHub, Gear, GearCtx};
    use toolkit_db::secure::{AccessScope, SecureInsertExt};
    use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
    use toolkit_gts::gts_id;
    use toolkit_security::SecurityContext;

    const TENANT: uuid::Uuid = uuid::Uuid::from_u128(0xca_7a_10_01);
    const SKU: uuid::Uuid = uuid::Uuid::from_u128(0xca_7a_10_91);

    let path = std::env::temp_dir().join(format!(
        "bss-products-seam-provide-{}.sqlite3",
        uuid::Uuid::new_v4()
    ));
    let dsn = format!("sqlite://{}?mode=rwc", path.display());
    let db = connect_db(
        &dsn,
        ConnectOpts {
            max_conns: Some(4),
            min_conns: Some(1),
            ..Default::default()
        },
    )
    .await
    .expect("connect the file-backed sqlite mirror");

    let gear = BssProductsGear::default();
    toolkit_db::migration_runner::run_migrations_for_testing(&db, gear.migrations())
        .await
        .expect("boot the gear's migration chain");

    let hub = Arc::new(ClientHub::new());
    hub.register::<dyn authz_resolver_sdk::AuthZResolverApi>(Arc::new(AllowResolver));
    hub.register::<dyn types_registry_sdk::TypesRegistryClient>(Arc::new(AcceptingTypesRegistry));

    let provider = DBProvider::<DbError>::new(db);
    let ctx = GearCtx::new(
        "bss-products",
        uuid::Uuid::nil(),
        Arc::new(EmptyConfig),
        Arc::clone(&hub),
        tokio_util::sync::CancellationToken::new(),
    )
    .with_db(provider.clone());

    gear.init(&ctx)
        .await
        .expect("init with no client_wiring must wire Local");

    let catalog = hub
        .get::<dyn ProductCatalogClientV1>()
        .expect("absent client_wiring still registers ProductCatalogClientV1");

    let row = read_entity::Model {
        tenant_id: TENANT,
        entity_kind: "sku".to_owned(),
        entity_id: SKU,
        entity_code: Some("COMP-VCPU-H".to_owned()),
        name: "COMP-VCPU-H".to_owned(),
        lifecycle_state: "published".to_owned(),
        deprecated: false,
        composition_pending: false,
        sellable: Some(true),
        deprecation_provenance: None,
        replaced_by_sku_id: None,
        region_scope: String::new(),
        brand_scope: String::new(),
        sku_type: Some("service".to_owned()),
        plan_tier_label: Some("Pro".to_owned()),
        metering_unit: Some("vCPU-hour".to_owned()),
        usage_type_ref: Some("cf.usage.vcpu-hour".to_owned()),
        display_attributes: None,
        category_paths: None,
        published_version: 3,
        projected_at: time::OffsetDateTime::UNIX_EPOCH,
        generation: 0,
    };
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let model: read_entity::ActiveModel = row.into();
    read_entity::Entity::insert(model.clone())
        .secure()
        .scope_with_model(&scope, &model)
        .expect("scope the insert")
        .exec(&conn)
        .await
        .expect("insert a projected SKU");

    let subject = SecurityContext::builder()
        .subject_id(uuid::Uuid::now_v7())
        .subject_tenant_id(TENANT)
        .subject_type(gts_id!("cf.core.security.subject_user.v1~"))
        .token_scopes(vec!["*".to_owned()])
        .build()
        .expect("authed SecurityContext");
    let found = catalog
        .get_skus(&subject, &[SKU])
        .await
        .expect("get_skus over the wired local provider");
    assert_eq!(found.len(), 1, "one projected SKU");
    assert_eq!(found[0].sku_id, SKU);
    assert_eq!(found[0].sku_code, "COMP-VCPU-H");
    assert_eq!(found[0].status, "published");

    drop(std::fs::remove_file(&path));
}
