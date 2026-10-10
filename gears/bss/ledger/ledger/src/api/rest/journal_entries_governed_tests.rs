//! The dual-control comparand of a reverse: the exact debit total of the
//! original in its entry currency (not both sides, not the first line).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::reverse_governed_amount;
use crate::domain::error::DomainError;
use bss_ledger_sdk::{
    AccountClass, CurrencySpec, EntryView, LineView, MappingStatus, PostedMoney, Side,
    SourceDocType,
};
use uuid::Uuid;

fn money(text: &str, code: &str, scale: u8) -> PostedMoney {
    PostedMoney::try_new(
        bss_ledger_sdk::parse_decimal(text).unwrap(),
        CurrencySpec::try_new(code.to_owned(), scale).unwrap(),
    )
    .unwrap()
}

fn line(side: Side, money: PostedMoney, functional: Option<PostedMoney>) -> LineView {
    LineView {
        line_id: Uuid::now_v7(),
        entry_id: Uuid::nil(),
        payer_tenant_id: Uuid::nil(),
        account_id: Uuid::now_v7(),
        account_class: AccountClass::Ar,
        gl_code: None,
        side,
        money,
        invoice_id: None,
        due_date: None,
        revenue_stream: None,
        mapping_status: MappingStatus::Resolved,
        functional_money: functional,
        tax_jurisdiction: None,
        tax_filing_period: None,
        ar_status: None,
    }
}

fn entry(lines: Vec<LineView>) -> EntryView {
    EntryView {
        entry_id: Uuid::now_v7(),
        tenant_id: Uuid::now_v7(),
        period_id: "202610".to_owned(),
        entry_currency: "EUR".to_owned(),
        source_doc_type: SourceDocType::InvoicePost,
        source_business_id: "INV-1".to_owned(),
        reverses_entry_id: None,
        reverses_period_id: None,
        posted_at_utc: time::OffsetDateTime::UNIX_EPOCH,
        effective_at: chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
        posted_by_actor_id: Uuid::nil(),
        origin: "USER".to_owned(),
        correlation_id: Uuid::nil(),
        created_seq: 1,
        lines,
    }
}

#[test]
fn the_comparand_is_the_exact_debit_total_in_the_entry_currency() {
    let original = entry(vec![
        line(Side::Debit, money("700.25", "EUR", 2), None),
        line(Side::Debit, money("300.5", "EUR", 2), None),
        line(Side::Credit, money("900", "EUR", 2), None),
        line(Side::Credit, money("100.75", "EUR", 2), None),
        // A functional-only line in another currency carries no transaction value.
        line(
            Side::Debit,
            money("0", "USD", 2),
            Some(money("5", "USD", 2)),
        ),
    ]);
    assert_eq!(
        reverse_governed_amount(&original).unwrap(),
        Some(money("1000.75", "EUR", 2))
    );
}

#[test]
fn no_debit_in_the_entry_currency_has_no_comparand_and_mixed_scales_are_named() {
    let credits_only = entry(vec![line(Side::Credit, money("10", "EUR", 2), None)]);
    assert_eq!(reverse_governed_amount(&credits_only).unwrap(), None);
    let mixed = entry(vec![
        line(Side::Debit, money("10", "EUR", 2), None),
        line(Side::Debit, money("10", "EUR", 3), None),
    ]);
    assert!(matches!(
        reverse_governed_amount(&mixed),
        Err(DomainError::InconsistentScale(_))
    ));
}
