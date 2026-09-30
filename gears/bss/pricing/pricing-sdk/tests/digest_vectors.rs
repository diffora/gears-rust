#![allow(clippy::expect_used, clippy::unwrap_used)]
// Compile the private encoder as a test module; no untyped production SDK door is exported.
#[allow(dead_code)]
#[path = "../src/digest.rs"]
pub mod digest;
use bss_pricing_sdk::{Digest, read, terms};
use digest::{CanonicalValue, canonical_json_bytes, hash_document};
use read::{ImmutablePrice, PriceModel};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn restricted(v: &Value) -> Result<CanonicalValue, &'static str> {
    Ok(match v {
        Value::Null => CanonicalValue::Null,
        Value::Bool(b) => CanonicalValue::Bool(*b),
        Value::String(s) => CanonicalValue::String(s.clone()),
        Value::Array(a) => {
            CanonicalValue::Array(a.iter().map(restricted).collect::<Result<_, _>>()?)
        }
        Value::Object(o) => CanonicalValue::Object(
            o.iter()
                .map(|(k, v)| Ok::<_, &'static str>((k.clone(), restricted(v)?)))
                .collect::<Result<_, _>>()?,
        ),
        Value::Number(_) => return Err("numbers are forbidden"),
    })
}
fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/digests.json")).unwrap()
}
fn hex(digest: Digest) -> String {
    use std::fmt::Write;
    digest.iter().fold(String::new(), |mut out, b| {
        write!(out, "{b:02x}").unwrap();
        out
    })
}
fn price(rate: &str) -> ImmutablePrice {
    ImmutablePrice {
        price_id: uuid::Uuid::from_u128(1),
        price_book_entry_id: uuid::Uuid::from_u128(2),
        money_digest: [0; 32],
        currency: "EUR".into(),
        model: PriceModel::PerUnit {
            unit_amount: Decimal::from_str_exact(rate).unwrap(),
        },
        minimum_fee: None,
        effective_from: time::Date::from_calendar_date(2026, time::Month::September, 1).unwrap(),
        ends_on: None,
    }
}
#[test]
fn frozen_canonical_bytes_and_sha256_match_for_every_vector() {
    for vector in fixture()["vectors"].as_array().unwrap() {
        let domain = vector["domain"].as_str().unwrap();
        let payload = restricted(&vector["payload"]).unwrap();
        let document = CanonicalValue::Object(BTreeMap::from([
            ("domain".into(), CanonicalValue::String(domain.into())),
            ("payload".into(), payload.clone()),
        ]));
        assert_eq!(
            String::from_utf8(canonical_json_bytes(&document)).unwrap(),
            vector["canonical_text"]
        );
        assert_eq!(hex(hash_document(domain, payload)), vector["sha256"]);
    }
}
#[test]
fn semantic_inputs_are_normalized_before_hashing() {
    let fixture = fixture();
    for case in fixture["semantic_projections"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        let vector =
            &fixture["vectors"][usize::try_from(case["vector"].as_u64().unwrap()).unwrap()];
        let payload = match case["kind"].as_str().unwrap() {
            "money" => {
                let p = price(input);
                assert_eq!(
                    hex(bss_pricing_sdk::digest::money_digest(&p)),
                    vector["sha256"]
                );
                json!({"currency":"EUR","model":{"kind":"per_unit","unit_amount":Decimal::from_str_exact(input).unwrap().normalize().to_string()},"minimum_fee":null})
            }
            "u64" => json!({"version":input.parse::<u64>().unwrap().to_string()}),
            "instant" => {
                let at = time::OffsetDateTime::parse(
                    input,
                    &time::format_description::well_known::Rfc3339,
                )
                .unwrap()
                .to_offset(time::UtcOffset::UTC);
                json!({"at":format!("{}T{:02}:{:02}:{:02}.{:09}Z",at.date(),at.hour(),at.minute(),at.second(),at.nanosecond())})
            }
            other => panic!("unknown projection {other}"),
        };
        assert_eq!(payload, vector["payload"]);
    }
}
#[test]
fn numeric_and_malformed_unicode_inputs_are_rejected() {
    assert!(restricted(&json!({"amount":0.047})).is_err());
    assert!(serde_json::from_str::<Value>(r#""\ud800""#).is_err());
    assert!(Decimal::from_str_exact("NaN").is_err());
    assert!(Decimal::from_str_exact("18446744073709551615.123456789012345678901").is_err());
}
#[test]
fn money_ignores_identity_and_closure_but_covers_every_operand() {
    let original = price("0.047");
    let baseline = bss_pricing_sdk::digest::money_digest(&original);
    let mut changed = original.clone();
    changed.price_id = uuid::Uuid::from_u128(3);
    changed.price_book_entry_id = uuid::Uuid::from_u128(4);
    changed.ends_on = Some(original.effective_from);
    changed.money_digest = [9; 32];
    assert_eq!(baseline, bss_pricing_sdk::digest::money_digest(&changed));
    changed.currency = "USD".into();
    assert_ne!(baseline, bss_pricing_sdk::digest::money_digest(&changed));
    changed = original;
    changed.minimum_fee = Some(Decimal::ONE);
    assert_ne!(baseline, bss_pricing_sdk::digest::money_digest(&changed));
    changed = price("0.048");
    assert_ne!(baseline, bss_pricing_sdk::digest::money_digest(&changed));
}

#[test]
fn read_contract_ir_is_valid_and_every_method_is_safe_read() {
    let ir = read::pricing_read_v1_ir();
    toolkit_contract::ir::validate_contract(&ir).unwrap();
    assert_eq!(ir.methods.len(), 3);
    assert!(
        ir.methods
            .iter()
            .all(|m| m.idempotency == toolkit_contract::ir::contract::Idempotency::SafeRead)
    );
}

fn policy_input() -> terms::UsageRatingPolicyInput {
    terms::UsageRatingPolicyInput {
        rating_window: terms::RatingWindow::CalendarHour {
            timezone: terms::Timezone::Utc,
        },
        aggregation_scope: terms::AggregationScope::SubscriptionLine,
        reset: terms::Reset::RatingWindowStart,
        quantity_semantics: terms::QuantitySemantics {
            meter: terms::MeterRef {
                usage_type_id: "cloudlets".into(),
                version: "1".into(),
            },
            unit: "cloudlet_hour".into(),
            fold: terms::Fold::Sum,
            accrual_policy_version: "integration-v1".into(),
        },
        partial_window: terms::PartialWindow::ActualQuantityFullThresholds,
    }
}
fn sample_binding() -> read::AcceptedBinding {
    use bss_pricing_sdk::digest::{money_digest, policy_digest, template_digest};
    let mut price = price("0.04700");
    price.money_digest = money_digest(&price);
    let content = policy_input();
    read::AcceptedBinding {
        item_id: uuid::Uuid::from_u128(3),
        price_book_entry_id: uuid::Uuid::from_u128(2),
        dimension_key: Some("region".into()),
        dimension_value: Some("eu".into()),
        sku_id: uuid::Uuid::from_u128(4),
        sku_version: 1,
        sku_code: "CLOUD".into(),
        sku_name: "Cloudlet".into(),
        unit: Some("cloudlet_hour".into()),
        price,
        kind: read::ChargeKind::Usage,
        recurring_period: None,
        via_default: true,
        usage_rating_policy: Some(terms::UsageRatingPolicy {
            policy_id: uuid::Uuid::from_u128(5),
            version: u64::MAX,
            digest: policy_digest(&content),
            content,
        }),
        invoice: terms::InvoiceInputs {
            template: "{sku}".into(),
            template_digest: template_digest("{sku}"),
            template_source: terms::InputSource::SkuVersion,
            gl_code: "usage".into(),
            tax_category: "standard".into(),
            timing: terms::BillingTiming::Arrears,
            currency_scale: 2,
            rounding: terms::Rounding::HalfEven,
        },
    }
}
#[test]
fn public_binding_and_policy_projections_match_frozen_vectors() {
    use bss_pricing_sdk::digest::{policy_digest, selected_bindings_digest, template_digest};
    let fixture = fixture();
    let policy_vector = usize::try_from(fixture["policy_vector"].as_u64().unwrap()).unwrap();
    assert_eq!(
        hex(policy_digest(&policy_input())),
        fixture["vectors"][policy_vector]["sha256"]
    );
    for case in fixture["binding_vectors"].as_array().unwrap() {
        let mut binding = sample_binding();
        match case["change"].as_str().unwrap() {
            "original" => {}
            "unit" => binding.unit = Some("cloudlet_second".into()),
            "policy_version" => binding.usage_rating_policy.as_mut().unwrap().version = 1,
            "template" => {
                binding.invoice.template = "{sku} changed".into();
                binding.invoice.template_digest = template_digest(&binding.invoice.template);
            }
            other => panic!("unknown variant {other}"),
        }
        let selection = read::BindingSelection {
            item_id: binding.item_id,
            dimension_value: binding.dimension_value.clone(),
        };
        let resolved = read::ResolvedBindings {
            plan_id: uuid::Uuid::from_u128(6),
            revision_id: uuid::Uuid::from_u128(7),
            cells: vec![read::ResolvedCell {
                selection: selection.clone(),
                binding: Some(binding),
            }],
        };
        let expected = &fixture["vectors"]
            [usize::try_from(case["vector"].as_u64().unwrap()).unwrap()]["sha256"];
        assert_eq!(
            hex(selected_bindings_digest(&resolved, &[selection]).unwrap()),
            *expected
        );
    }
}
#[test]
fn every_model_operand_is_in_the_public_money_projection() {
    use bss_pricing_sdk::digest::money_digest;
    let tiers = vec![
        read::Tier {
            up_to: Some(Decimal::TEN),
            rate: Decimal::from_str_exact("0.04700").unwrap(),
        },
        read::Tier {
            up_to: None,
            rate: Decimal::from_str_exact("0.030").unwrap(),
        },
    ];
    let models = [
        PriceModel::Flat {
            amount: Decimal::from(12),
        },
        PriceModel::Package {
            package_size: Decimal::TEN,
            package_price: Decimal::from_str_exact("4.70").unwrap(),
        },
        PriceModel::Volume {
            tiers: tiers.clone(),
        },
        PriceModel::Graduated { tiers },
    ];
    let fixture = fixture();
    for (index, model) in models.into_iter().enumerate() {
        let mut price = price("0.047");
        price.model = model;
        price.minimum_fee = Some(Decimal::from(2));
        assert_eq!(
            hex(money_digest(&price)),
            fixture["vectors"][index + 10]["sha256"]
        );
    }
}

#[test]
fn binding_sets_sort_by_selection_and_reject_identity_mismatch() {
    use bss_pricing_sdk::digest::selected_bindings_digest;
    let binding = sample_binding();
    let mut other = binding.clone();
    other.item_id = uuid::Uuid::from_u128(8);
    let selection = read::BindingSelection {
        item_id: binding.item_id,
        dimension_value: binding.dimension_value.clone(),
    };
    let other_selection = read::BindingSelection {
        item_id: other.item_id,
        dimension_value: other.dimension_value.clone(),
    };
    let mut resolved = read::ResolvedBindings {
        plan_id: uuid::Uuid::from_u128(6),
        revision_id: uuid::Uuid::from_u128(7),
        cells: vec![
            read::ResolvedCell {
                selection: selection.clone(),
                binding: Some(binding),
            },
            read::ResolvedCell {
                selection: other_selection.clone(),
                binding: Some(other),
            },
        ],
    };
    let expected =
        selected_bindings_digest(&resolved, &[selection.clone(), other_selection.clone()]).unwrap();
    resolved.cells.reverse();
    assert_eq!(
        expected,
        selected_bindings_digest(&resolved, &[other_selection.clone(), selection]).unwrap()
    );
    resolved.cells[0]
        .binding
        .as_mut()
        .unwrap()
        .price_book_entry_id = uuid::Uuid::from_u128(9);
    assert!(selected_bindings_digest(&resolved, &[other_selection]).is_err());
}
