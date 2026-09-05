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
//! written. Re-measured at `14344c110` against pricing's tree: no watermark
//! producer, no consumer of any registry event, no adoption-guard code raised
//! (`SKU_NOT_PUBLISHED` is named and not raised), no meter-binding rule.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-contract-seams:p1

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use bss_pricing_sdk::product_catalog::CatalogSku;
use bss_products_sdk::models::{LifecycleState, Sku};

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
    "tax_category_ref",
    "gl_code_ref",
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
        tax_category_ref,
        gl_code_ref,
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
    } = consumer;
}

#[test]
fn the_field_lists_mirror_the_two_types() {
    // Fifteen bindings above, fifteen names; nine and nine.
    assert_eq!(REGISTRY_FIELDS.len(), 15);
    assert_eq!(CONSUMER_FIELDS.len(), 9);
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
