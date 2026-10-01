//! The derived usage type's create rules, its digest and its catalog resolution (P-D-231).
#![allow(clippy::expect_used, clippy::unwrap_used)]
use super::*;
use crate::test_support::{StubUsageTypes, authed_ctx, probe_binding};
use bss_products_sdk::derived::{
    DerivedInput, Expr, Granularity, GranuleFold, RoundMode, UnitSite,
};
use std::sync::atomic::Ordering;

const RAM_REF: &str = "gts.cf.core.uc.usage_record.v1~cf.test.usage.ram_mb.v1";
const CPU_REF: &str = "gts.cf.core.uc.usage_record.v1~cf.test.usage.cpu_mhz.v1";
/// The SHA-256 of the SDK's pinned cloudlet bytes (`derived_tests::the_cloudlet_canonical_bytes_are_
/// pinned`), taken outside the code: `printf '%s' '<the pinned bytes>' | shasum -a 256`.
const CLOUDLET_DIGEST: &str = "9af0a695c790874e75592010b587407a3d90878dab200005bc8697626a11c421";

fn input(name: &str, reference: &str, unit: &str) -> DerivedInput {
    DerivedInput {
        name: name.to_owned(),
        usage_type_ref: reference.to_owned(),
        granule_fold: GranuleFold::Peak,
        max_hold_seconds: None,
        unit: unit.to_owned(),
    }
}
fn share(name: &str, divisor: &str) -> Expr {
    Expr::Ceil(Box::new(Expr::DivConst(
        Box::new(Expr::Input(name.to_owned())),
        divisor.parse().unwrap(),
    )))
}
fn cloudlet_with(ram: &str, cpu: &str) -> DerivedUsageDeclaration {
    DerivedUsageDeclaration {
        output_unit: "cloudlet\u{b7}hour".to_owned(),
        granularity: Granularity::Hour,
        inputs: vec![
            input("ram_mb", RAM_REF, "MB"),
            input("cpu_mhz", CPU_REF, "MHz"),
        ],
        formula: Expr::Max(vec![share("ram_mb", ram), share("cpu_mhz", cpu)]),
        output_scale: 0,
        output_round: RoundMode::HalfEven,
    }
}

/// Decision 3: the digest is the SHA-256 of the canonical bytes, as lowercase hex; `128` and
/// `128.0` are one declaration.
#[test]
fn the_digest_is_the_sha256_of_the_canonical_bytes() {
    assert_eq!(digest_hex(&cloudlet_with("128", "400")), CLOUDLET_DIGEST);
    assert_eq!(
        digest_hex(&cloudlet_with("128.0", "400.00")),
        CLOUDLET_DIGEST
    );
    assert_ne!(digest_hex(&cloudlet_with("129", "400")), CLOUDLET_DIGEST);
}

/// Every SDK refusal names its own rule: one token per variant, none shared.
#[test]
fn every_declaration_error_names_a_rule_of_its_own() {
    let name = || "x".to_owned();
    let all = [
        DeclarationError::EmptyUnit {
            site: UnitSite::Output,
        },
        DeclarationError::UnitTooLong {
            site: UnitSite::Input(name()),
        },
        DeclarationError::ScaleTooLarge { scale: 13 },
        DeclarationError::TooFewInputs { count: 1 },
        DeclarationError::InvalidInputName { name: name() },
        DeclarationError::DuplicateInput { name: name() },
        DeclarationError::EmptyInputRef { name: name() },
        DeclarationError::InputRefTooLong { name: name() },
        DeclarationError::DerivedInput { name: name() },
        DeclarationError::HoldMissing { name: name() },
        DeclarationError::HoldNotAllowed { name: name() },
        DeclarationError::HoldOutOfRange {
            name: name(),
            seconds: 0,
        },
        DeclarationError::UnknownInput { name: name() },
        DeclarationError::UnusedInput { name: name() },
        DeclarationError::DivisionByZero,
        DeclarationError::TooFewOperands { found: 1 },
        DeclarationError::TooDeep,
        DeclarationError::TooManyNodes,
    ];
    let rules: std::collections::BTreeSet<&str> = all.iter().map(rule).collect();
    assert_eq!(rules.len(), all.len(), "{rules:?}");
    assert!(
        rules
            .iter()
            .all(|r| r.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
    );
}

/// The SDK's refusal is the door's 400, its detail led by the rule.
#[test]
fn an_sdk_refusal_is_derived_declaration_invalid_naming_its_rule() {
    let mut bad = cloudlet_with("128", "400");
    bad.inputs[1].usage_type_ref = "products.derived/other@1".to_owned();
    let Err(DomainError::Validation(report)) = validate(&bad) else {
        panic!("a derived input is refused");
    };
    let [v] = report.violations() else {
        panic!("{report:?}");
    };
    assert_eq!(v.code, DECLARATION_INVALID);
    assert_eq!(v.subject, "declaration");
    assert!(v.detail.starts_with("derived_input: "), "{}", v.detail);
    validate(&cloudlet_with("128", "400")).unwrap();
}

/// The identity of a create: the code the meter id carries (`FIELD_TOO_LONG` over 64 characters,
/// P-D-225), and a name that is not blank and at most 200 characters.
#[test]
fn the_identity_rules_of_a_create() {
    let check = |code: &str, name: &str| {
        check_identity(&NewDerivedType {
            code: code.to_owned(),
            name: name.to_owned(),
        })
        .violations()
        .iter()
        .map(|v| (v.code, v.subject.clone()))
        .collect::<Vec<_>>()
    };
    assert_eq!(check("cloudlets", "Cloudlets"), []);
    assert_eq!(check(&"a".repeat(64), &"n".repeat(200)), []);
    for code in ["", "Cloudlets", "-x", "a b", "a/b"] {
        assert_eq!(
            check(code, "N"),
            [("VALIDATION", "code".to_owned())],
            "{code:?}"
        );
    }
    assert_eq!(
        check(&"a".repeat(65), "N"),
        [("FIELD_TOO_LONG", "code".to_owned())]
    );
    assert_eq!(check("ok", " "), [("VALIDATION", "name".to_owned())]);
    assert_eq!(
        check("ok", &"n".repeat(201)),
        [("FIELD_TOO_LONG", "name".to_owned())]
    );
}

/// Each input is asked once, as the caller. A refusal of the caller outranks an outage, and an
/// outage an unresolved input; unresolved inputs are each named on their ref.
#[tokio::test]
async fn each_input_resolves_through_the_catalog_once() {
    let decl = cloudlet_with("128", "400");
    let ctx = authed_ctx(Uuid::new_v4());
    let resolved = UsageTypeAnswer::Resolved(probe_binding());
    let ask = |answers: Vec<UsageTypeAnswer>| {
        let decl = decl.clone();
        let ctx = ctx.clone();
        async move {
            let stub = StubUsageTypes::scripted(answers);
            let result = resolve_inputs(&stub, &ctx, &decl).await;
            (result, stub.asked.load(Ordering::SeqCst))
        }
    };
    let (ok, asked) = ask(vec![resolved.clone()]).await;
    assert_eq!(ok, Ok(()));
    assert_eq!(asked, 2, "one question per input");
    let (unresolved, _) = ask(vec![UsageTypeAnswer::Unresolved]).await;
    let Err(DomainError::Validation(report)) = unresolved else {
        panic!("{unresolved:?}");
    };
    let subjects: Vec<_> = report
        .violations()
        .iter()
        .map(|v| (v.code, v.subject.as_str()))
        .collect();
    assert_eq!(
        subjects,
        [
            (
                "USAGE_TYPE_UNRESOLVED",
                "declaration.inputs.ram_mb.usage_type_ref"
            ),
            (
                "USAGE_TYPE_UNRESOLVED",
                "declaration.inputs.cpu_mhz.usage_type_ref"
            )
        ]
    );
    let (one, _) = ask(vec![resolved.clone(), UsageTypeAnswer::Unresolved]).await;
    let Err(DomainError::Validation(report)) = one else {
        panic!("{one:?}");
    };
    assert_eq!(report.violations().len(), 1);
    assert_eq!(
        ask(vec![
            UsageTypeAnswer::Unavailable,
            UsageTypeAnswer::Unresolved
        ])
        .await
        .0,
        Err(DomainError::UsageTypeUnavailable(RAM_REF.to_owned()))
    );
    assert_eq!(
        ask(vec![
            UsageTypeAnswer::Unavailable,
            UsageTypeAnswer::Forbidden
        ])
        .await
        .0,
        Err(DomainError::UsageTypeForbidden(CPU_REF.to_owned()))
    );
}

/// Decision 5's accrual string carries the stored digest.
#[test]
fn the_accrual_policy_version_carries_the_stored_digest() {
    let v = DerivedUsageTypeVersion {
        tenant_id: Uuid::new_v4(),
        type_id: Uuid::new_v4(),
        version: 1,
        declaration_json: serde_json::json!({}),
        digest: "ab".repeat(32),
        created_by: Uuid::new_v4(),
        created_at: OffsetDateTime::now_utc(),
    };
    assert_eq!(
        v.accrual_policy_version(),
        format!("derived-v1:{}", "ab".repeat(32))
    );
}
