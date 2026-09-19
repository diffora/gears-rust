#![allow(clippy::expect_used)]
use super::{DefaultLineTemplates, LINE_TEMPLATE_INVALID, parse};
use crate::domain::price_row::PriceRow;
use crate::domain::scope_key::ChargeKind;

#[test]
fn accepts_exact_vocabulary_and_escaped_literal_braces() {
    for source in [
        "{sku} {sku_code} {unit} {plan} {phase} {dimension} {period}",
        "{{sku_nam}}",
        "{{{sku}}}",
        "",
        "literal",
        "\u{0422}\u{0440}\u{0430}\u{0444}\u{0456}\u{043a}, {unit}",
    ] {
        assert_eq!(parse(source).expect("valid source").as_str(), source);
    }
}
#[test]
fn rejects_unknown_and_malformed_placeholders() {
    for source in [
        "{sku_nam}",
        "{SKU}",
        "{ sku}",
        "{}",
        "{sku",
        "sku}",
        "{sku{unit}",
    ] {
        assert!(parse(source).is_err(), "{source}");
    }
    assert_eq!(
        parse("{bad} {worse}").expect_err("two unknown names").len(),
        2
    );
}
#[test]
fn shipped_defaults_and_exact_policy_keys_are_enforced() {
    let defaults = DefaultLineTemplates::default();
    for (kind, expected) in [
        (ChargeKind::Recurring, "{sku} - {period}"),
        (ChargeKind::Usage, "{sku}, {unit}"),
        (ChargeKind::OneTime, "{sku}"),
    ] {
        assert_eq!(defaults.get(kind), expected);
    }
    let mut values = defaults.to_map();
    values.insert("usage".to_owned(), String::new());
    assert!(DefaultLineTemplates::from_map(values.clone()).is_ok());
    values.insert("usage".to_owned(), "{bad}".to_owned());
    assert!(DefaultLineTemplates::from_map(values.clone()).is_err());
    values.remove("usage");
    assert!(DefaultLineTemplates::from_map(values).is_err());
    let mut extra = defaults.to_map();
    extra.insert("other".to_owned(), "{sku}".to_owned());
    assert!(DefaultLineTemplates::from_map(extra).is_err());
}
#[test]
fn malformed_row_is_a_write_violation_but_absence_is_not() {
    let mut row = PriceRow::new(ChargeKind::Recurring, None);
    row.invoice_line_template = Some("{sku_nam}".to_owned());
    let report = crate::domain::rules::row_local_rules().run(&row);
    let write = report.write_stage_only().expect("syntax refuses at write");
    assert!(
        write
            .violations
            .iter()
            .any(|finding| finding.code == LINE_TEMPLATE_INVALID)
    );
    row.invoice_line_template = None;
    let report = crate::domain::rules::row_local_rules().run(&row);
    assert!(
        !report
            .violations
            .iter()
            .any(|finding| finding.code == LINE_TEMPLATE_INVALID)
    );
}
