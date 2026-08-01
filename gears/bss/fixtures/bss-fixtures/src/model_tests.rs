use super::*;

const GRADUATED: &str = r#"
family     = "tier-boundary"
id         = "graduated-band-edge"
kind       = "evaluation"
provenance = ["AC#60", "PRD 17.2", "D-17"]

[snapshot]
model_kind              = "graduated"
currency                = "USD"
tier_aggregation_window = "invoice_period"
billing_granularity     = "whole_unit"

  [[snapshot.bands]]
  from_qty = 0
  to_qty   = 1000
  unit_amount_minor = 5

  [[snapshot.bands]]
  from_qty = 1000
  to_qty   = "open"
  unit_amount_minor = 3

[[assert]]
given  = { q = 1000 }
expect = { charge_minor = 5000 }
why    = "half-open [from,to): 1000 opens band 2, but graduated is marginal"
"#;

#[test]
fn parses_a_graduated_evaluation_case() {
    let case: EvaluationCase = toml::from_str(GRADUATED).expect("corpus case must parse");

    assert_eq!(case.family, Family::TierBoundary);
    assert_eq!(case.id, "graduated-band-edge");
    assert_eq!(case.kind, CaseKind::Evaluation);
    assert_eq!(case.snapshot.model_kind, ModelKind::Graduated);
    assert_eq!(case.snapshot.bands.len(), 2);
    assert_eq!(case.snapshot.bands[0].to_qty, BandTop::Closed(1000));
    assert_eq!(case.snapshot.bands[1].to_qty, BandTop::Open);
    assert_eq!(case.assert.len(), 1);
    assert_eq!(case.assert[0].given.q, 1000);
    assert_eq!(
        case.assert[0].expect,
        Expect::Charge(ChargeExpect { charge_minor: 5000 })
    );
}

#[test]
fn rejects_an_unknown_snapshot_field() {
    let bad = GRADUATED.replace(
        "currency                = \"USD\"",
        "currency                = \"USD\"\nlocked_unit_price_minor = 7",
    );

    let err = toml::from_str::<EvaluationCase>(&bad)
        .expect_err("unknown snapshot field must be rejected");

    // D-60: the trailing-tier lock is per subscription per period, so it is not
    // snapshot-frozen and must not be expressible in `[snapshot]`.
    assert!(
        err.to_string().contains("locked_unit_price_minor"),
        "error should name the offending field, got: {err}"
    );
}

#[test]
fn rejects_a_band_top_that_is_neither_integer_nor_open() {
    let bad = GRADUATED.replace("to_qty   = \"open\"", "to_qty   = \"unbounded\"");

    let err = toml::from_str::<EvaluationCase>(&bad).expect_err("bad band top must be rejected");

    assert!(
        err.to_string().contains("unbounded"),
        "error should quote the bad value, got: {err}"
    );
}

#[test]
fn rejects_a_snapshot_field_smuggled_into_runtime() {
    // The boundary is checked in both directions. A band or a model kind under
    // `[runtime]` is as wrong as a runtime value under `[snapshot]`: the split
    // is the gear ownership boundary, not a convenience.
    let bad = format!("{GRADUATED}\n[runtime]\nmodel_kind = \"volume\"\n");

    let err = toml::from_str::<EvaluationCase>(&bad)
        .expect_err("a snapshot field under runtime is rejected");

    assert!(
        err.to_string().contains("model_kind"),
        "error should name the offending field, got: {err}"
    );
}

#[test]
fn an_empty_runtime_section_is_fine() {
    // Phase-1 families need no consumer-side context beyond the quantity, so an
    // empty (or absent) `[runtime]` must stay legal.
    let ok = format!("{GRADUATED}\n[runtime]\n");

    toml::from_str::<EvaluationCase>(&ok).expect("an empty runtime section must parse");
}

#[test]
fn rejects_a_tier_aggregation_window_no_document_defines() {
    // `billing_period` is the value four cases carried: a plausible synonym of
    // `invoice_period` that reads as obviously correct and appears in no
    // enumeration. While the field was an `Option<String>` it loaded silently and
    // was only ever noticed by a consumer that could not map it.
    let bad = GRADUATED.replace("\"invoice_period\"", "\"billing_period\"");

    let err = toml::from_str::<EvaluationCase>(&bad)
        .expect_err("a window outside inst-tb-window's enumeration must not load");

    assert!(
        err.to_string().contains("billing_period"),
        "error should quote the bad value, got: {err}"
    );
}

#[test]
fn rejects_a_billing_granularity_no_document_defines() {
    // `per_unit` is a `modelKind`, not a granularity - which is exactly why it
    // was an easy thing to write in this field and a hard thing to see.
    let bad = GRADUATED.replace("\"whole_unit\"", "\"per_unit\"");

    let err = toml::from_str::<EvaluationCase>(&bad)
        .expect_err("a granularity outside inst-tb-window's enumeration must not load");

    assert!(
        err.to_string().contains("per_unit"),
        "error should quote the bad value, got: {err}"
    );
}

#[test]
fn provenance_is_required() {
    let bad = GRADUATED.replace(r#"provenance = ["AC#60", "PRD 17.2", "D-17"]"#, "");

    toml::from_str::<EvaluationCase>(&bad).expect_err("a case without provenance must not parse");
}
