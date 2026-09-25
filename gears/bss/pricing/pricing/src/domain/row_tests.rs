#![allow(clippy::expect_used, clippy::unwrap_used)]
use super::*;
use crate::domain::{
    money::PriceData,
    price::{ChargeKind, Model},
    test_support::{date, dec},
};
use uuid::Uuid;
fn row(v: i32, from: &str, dim: Option<&str>, state: RowState) -> Row {
    Row {
        id: Uuid::from_u128(u128::try_from(v).unwrap()),
        price_id: Uuid::from_u128(100),
        version_no: v,
        dim_value: dim.map(str::to_owned),
        model: Model::PerUnit,
        price: Some(PriceData::PerUnit { rate: dec("1") }),
        min_fee: None,
        eligibility: Eligibility::All,
        effective_from: date(from),
        effective_to: None,
        temporary_until: None,
        paired_row_id: None,
        return_of_row_id: None,
        closed_explicitly: false,
        state,
    }
}
fn rows() -> Vec<Row> {
    let mut r = vec![
        row(1, "2026-01-01", None, RowState::Approved),
        row(2, "2026-06-01", None, RowState::Approved),
        row(3, "2026-06-01", Some("us"), RowState::Approved),
        row(4, "2026-12-01", Some("us"), RowState::Pending),
    ];
    normalize_windows(&mut r);
    r
}
fn rules(r: &Row, siblings: &[Row], values: Option<&[String]>) -> Vec<crate::domain::RuleError> {
    validate(
        r,
        ChargeKind::Usage,
        values,
        siblings,
        date("2026-09-23"),
        2,
    )
}
#[test]
fn prototype_l45_l47_version_at_ignores_pending() {
    let mut r = rows();
    r[3].dim_value = None;
    assert_eq!(
        version_at(&r, r[0].price_id, date("2026-03-01"), None)
            .unwrap()
            .version_no,
        1
    );
    assert_eq!(
        version_at(&r, r[0].price_id, date("2026-06-01"), None)
            .unwrap()
            .version_no,
        2
    );
    assert_eq!(
        version_at(&r, r[0].price_id, date("2026-12-15"), None)
            .unwrap()
            .version_no,
        2
    );
}
#[test]
fn prototype_l48_l50_status() {
    let r = rows();
    assert_eq!(status(&r[0], date("2026-09-23")), "superseded");
    assert_eq!(status(&r[1], date("2026-09-23")), "active");
    assert_eq!(status(&r[3], date("2026-09-23")), "pending");
    assert_eq!(
        status(
            &row(5, "2027-01-01", None, RowState::Approved),
            date("2026-09-23")
        ),
        "scheduled"
    );
}
#[test]
fn prototype_l70_l72_validation_collects_kind_past_overlap() {
    let mut r = row(9, "2026-06-01", None, RowState::Draft);
    r.model = Model::Flat;
    r.price = Some(PriceData::Flat { amount: dec("5") });
    let c = rules(&r, &rows(), None);
    for code in [
        "MODEL_KIND_CHARGEKIND_MISMATCH",
        "WINDOW_START_IN_PAST",
        "WINDOW_OVERLAP",
    ] {
        assert!(c.iter().any(|e| e.code == code), "{code}");
    }
}
#[test]
fn prototype_l73_price_missing() {
    let mut r = row(2, "2027-01-01", None, RowState::Draft);
    r.model = Model::Flat;
    r.price = None;
    assert_eq!(
        validate(&r, ChargeKind::Recurring, None, &[], date("2026-09-23"), 2)[0].code,
        "PRICE_MISSING"
    );
}
#[test]
fn prototype_l78_windows_normalize() {
    let r = rows();
    assert_eq!(r[0].effective_to, Some(date("2026-06-01")));
    assert_eq!(r[1].effective_to, None);
}
#[test]
fn prototype_l190_l191_return_copy_is_detached() {
    let mut r = row(1, "2026-01-01", None, RowState::Approved);
    r.model = Model::Flat;
    r.price = Some(PriceData::Flat { amount: dec("20") });
    let mut promo = r.clone();
    promo.id = Uuid::from_u128(2);
    promo.version_no = 2;
    promo.state = RowState::Draft;
    promo.effective_from = date("2026-10-01");
    promo.price = Some(PriceData::Flat { amount: dec("40") });
    let mut pair = temporary(&[r.clone()], promo, date("2026-10-11"), Uuid::from_u128(3)).unwrap();
    assert_eq!(pair[1].version_no, 3);
    assert_eq!(pair[1].effective_from, date("2026-10-11"));
    assert_eq!(pair[1].price, r.price);
    assert_eq!(pair[1].return_of_row_id, Some(r.id));
    assert_eq!(pair[1].paired_row_id, Some(pair[0].id));
    assert_eq!(pair[0].paired_row_id, Some(pair[1].id));
    pair[1].price = Some(PriceData::Flat { amount: dec("99") });
    assert_eq!(r.price, Some(PriceData::Flat { amount: dec("20") }));
}
#[test]
fn prototype_l192_l194_temporary_end() {
    let start = date("2026-10-01");
    assert!(validate_temporary(start, "2026-10-11").is_ok());
    for end in ["2026-10-01", "", "bogus", "2026-02-30"] {
        assert_eq!(
            validate_temporary(start, end).unwrap_err().code,
            "WINDOW_END_INVALID"
        );
    }
}
#[test]
fn prototype_l195_empty_chain_has_no_return() {
    let pair = temporary(
        &[],
        row(2, "2026-10-01", Some("us"), RowState::Draft),
        date("2026-10-11"),
        Uuid::from_u128(3),
    )
    .unwrap();
    assert_eq!(pair.len(), 1);
    assert!(pair[0].closed_explicitly);
    assert_eq!(pair[0].effective_to, Some(date("2026-10-11")));
    assert!(pair[0].paired_row_id.is_none());
}
#[test]
fn prototype_l200_l203_temporary_resolution_and_open_tail() {
    let mut base = row(1, "2026-01-01", None, RowState::Approved);
    base.price = Some(PriceData::PerUnit { rate: dec("20") });
    let mut promo = row(2, "2026-10-01", None, RowState::Approved);
    promo.price = Some(PriceData::PerUnit { rate: dec("40") });
    let mut r = temporary(
        &[base.clone()],
        promo,
        date("2026-10-11"),
        Uuid::from_u128(3),
    )
    .unwrap();
    r.push(base);
    normalize_windows(&mut r);
    for (day, v) in [("2026-09-30", 1), ("2026-10-05", 2), ("2026-10-11", 3)] {
        assert_eq!(
            version_at(&r, r[0].price_id, date(day), None)
                .unwrap()
                .version_no,
            v
        );
    }
    assert_eq!(open_tail(&r, r[0].price_id, None).unwrap().version_no, 3);
}
#[test]
fn prototype_l238_l241_shift_preserves_duration() {
    assert_eq!((date("2026-10-11") - date("2026-10-01")).whole_days(), 10);
    let mut r = row(3, "2026-10-01", None, RowState::Draft);
    r.temporary_until = Some(date("2026-10-11"));
    let shifted = shift(&r, date("2026-11-01")).unwrap();
    assert_eq!(shifted.effective_from, date("2026-11-01"));
    assert_eq!(shifted.temporary_until, Some(date("2026-11-11")));
    assert_eq!(shifted.price, r.price);
    assert_eq!(shift(&r, r.effective_from).unwrap(), r);
}
#[test]
fn prototype_l243_l244_proposed_rows() {
    let mut r = rows();
    let draft = row(5, "2026-11-01", None, RowState::Draft);
    r.push(draft.clone());
    let book = Uuid::from_u128(10);
    let mapping = [(draft.price_id, book)];
    assert_eq!(proposed_rows(book, &mapping, &r), vec![&r[4]]);
    assert!(proposed_rows(Uuid::nil(), &mapping, &r).is_empty());
}
#[test]
fn matrix_22_proposals_sort_by_start_price_version() {
    let book = Uuid::nil();
    let mut r = vec![
        row(3, "2026-11-01", None, RowState::Draft),
        row(2, "2026-11-01", None, RowState::Draft),
        row(1, "2026-10-01", None, RowState::Draft),
    ];
    r[2].price_id = Uuid::from_u128(200);
    assert_eq!(
        proposed_rows(book, &[(r[0].price_id, book), (r[2].price_id, book)], &r)
            .iter()
            .map(|x| x.version_no)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
}
#[test]
fn prototype_l252_l256_dimensional_windows_and_fallback() {
    let r = rows();
    assert_eq!(r[0].effective_to, Some(date("2026-06-01")));
    assert!(r[1].effective_to.is_none());
    assert!(r[2].effective_to.is_none());
    for (d, dim, v) in [
        ("2026-09-01", Some("us"), 3),
        ("2026-09-01", Some("eu"), 2),
        ("2026-09-01", None, 2),
        ("2026-03-01", Some("us"), 1),
    ] {
        assert_eq!(
            version_at(&r, r[0].price_id, date(d), dim)
                .unwrap()
                .version_no,
            v
        );
    }
}
#[test]
fn prototype_l257_l259_coverage_per_value() {
    let r = rows();
    let vals = vec!["eu".into(), "us".into()];
    let c = coverage_on(&r, r[0].price_id, date("2026-09-01"), &vals);
    assert!(c.missing.is_empty());
    assert_eq!(c.version.unwrap().version_no, 2);
    let c = coverage_on(&r[2..], r[0].price_id, date("2026-09-01"), &vals);
    assert_eq!(c.missing, vec!["eu"]);
}
#[test]
fn prototype_l260_unknown_dimension_value() {
    assert!(
        rules(
            &row(9, "2026-12-01", Some("mars"), RowState::Draft),
            &rows(),
            Some(&["eu".into(), "us".into()])
        )
        .iter()
        .any(|e| e.code == "DIM_VALUE_UNKNOWN")
    );
}
#[test]
fn prototype_l261_other_value_same_start_allowed() {
    assert!(
        !rules(
            &row(9, "2026-06-01", Some("ap"), RowState::Draft),
            &rows(),
            Some(&["ap".into(), "us".into()])
        )
        .iter()
        .any(|e| e.code == "WINDOW_OVERLAP")
    );
}
#[test]
fn prototype_l262_same_value_same_start_overlap() {
    assert!(
        rules(
            &row(9, "2026-06-01", Some("us"), RowState::Draft),
            &rows(),
            Some(&["eu".into(), "us".into()])
        )
        .iter()
        .any(|e| e.code == "WINDOW_OVERLAP")
    );
}
#[test]
fn prototype_l263_dimension_not_declared() {
    assert!(
        rules(
            &row(9, "2026-12-01", Some("us"), RowState::Draft),
            &[],
            None
        )
        .iter()
        .any(|e| e.code == "DIM_NOT_DECLARED")
    );
}
#[test]
fn prototype_l273_return_uses_own_chain() {
    let mut r = rows();
    r[2].price = Some(PriceData::PerUnit { rate: dec("5") });
    let pair = temporary(
        &r,
        row(7, "2026-10-01", Some("us"), RowState::Draft),
        date("2026-10-11"),
        Uuid::from_u128(8),
    )
    .unwrap();
    assert_eq!(pair[1].dim_value.as_deref(), Some("us"));
    assert_eq!(pair[1].price, r[2].price);
}
#[test]
fn matrix_12_19_closed_end_survives_twice_and_later_default() {
    let mut r = rows();
    let only = temporary(
        &r,
        row(7, "2026-10-01", Some("eu"), RowState::Approved),
        date("2026-10-11"),
        Uuid::from_u128(8),
    )
    .unwrap();
    assert_eq!(only.len(), 1);
    r.extend(only);
    normalize_windows(&mut r);
    normalize_windows(&mut r);
    assert_eq!(r[4].effective_to, Some(date("2026-10-11")));
    assert_eq!(
        version_at(&r, r[0].price_id, date("2026-10-11"), Some("eu"))
            .unwrap()
            .version_no,
        2
    );
    r.push(row(9, "2026-10-10", None, RowState::Approved));
    normalize_windows(&mut r);
    assert_eq!(r[4].effective_to, Some(date("2026-10-11")));
    assert_eq!(
        version_at(&r, r[0].price_id, date("2026-10-11"), Some("eu"))
            .unwrap()
            .version_no,
        9
    );
}
#[test]
fn matrix_18_min_fee_is_only_validated() {
    let mut r = row(9, "2026-12-01", None, RowState::Draft);
    for fee in ["-1", "1.001", "1.000"] {
        r.min_fee = Some(dec(fee));
        assert!(
            rules(&r, &[], None)
                .iter()
                .any(|e| e.code == "MIN_FEE_INVALID")
        );
    }
    r.min_fee = Some(dec("30"));
    assert!(rules(&r, &[], None).is_empty());
    assert_eq!(
        crate::domain::money::amount_for(r.model, r.price.as_ref().unwrap(), dec("10")).unwrap(),
        dec("10")
    );
}
#[test]
fn matrix_18_amount_package_and_start_invalid() {
    let mut r = row(9, "2026-12-01", None, RowState::Draft);
    r.price = Some(PriceData::PerUnit { rate: dec("-1") });
    assert_eq!(rules(&r, &[], None)[0].code, "AMOUNT_INVALID");
    r.model = Model::Package;
    r.price = Some(PriceData::Package {
        package_size: dec("0"),
        package_price: dec("5"),
    });
    assert_eq!(rules(&r, &[], None)[0].code, "PACKAGE_FIELDS_INVALID");
    assert_eq!(
        parse_start("2026-02-30").unwrap_err().code,
        "WINDOW_START_INVALID"
    );
}
#[test]
fn matrix_24_dated_metering_guard() {
    let a = row(1, "2026-01-01", None, RowState::Approved);
    let mut b = row(2, "2026-06-01", None, RowState::Draft);
    let old = SkuMetering {
        unit: Some("GB".into()),
        usage_type_ref: Some("storage".into()),
    };
    let mut new = old.clone();
    assert!(chain_guard(ChargeKind::Usage, &a, &old, &b, &new).is_ok());
    new.usage_type_ref = Some("other".into());
    assert_eq!(
        chain_guard(ChargeKind::Usage, &a, &old, &b, &new)
            .unwrap_err()
            .code,
        "CHAIN_MODEL_CHANGED"
    );
    new = old.clone();
    new.unit = Some("TB".into());
    assert!(chain_guard(ChargeKind::Usage, &a, &old, &b, &new).is_err());
    b.model = Model::Volume;
    assert!(chain_guard(ChargeKind::Recurring, &a, &old, &b, &new).is_ok());
    assert!(chain_guard(ChargeKind::Usage, &a, &old, &b, &old).is_err());
}
#[test]
fn a_closed_row_is_still_closed_by_a_successor_that_starts_inside_it() {
    // A temporary value row [10-01, 10-11) closed explicitly; a later value row
    // approved from 10-05 must cap it, or two rows are in force on 10-05..10-11.
    let mut r = vec![
        row(1, "2026-01-01", None, RowState::Approved),
        row(2, "2026-10-01", Some("us"), RowState::Approved),
        row(3, "2026-10-05", Some("us"), RowState::Approved),
    ];
    r[1].effective_to = Some(date("2026-10-11"));
    r[1].closed_explicitly = true;
    normalize_windows(&mut r);
    assert_eq!(r[1].effective_to, Some(date("2026-10-05")));
    assert!(r[1].closed_explicitly, "the closure itself is kept");
    assert_eq!(r[2].effective_to, None);
    // A successor after the explicit end leaves the explicit end alone.
    r[2].effective_from = date("2026-12-01");
    r[1].effective_to = Some(date("2026-10-11"));
    normalize_windows(&mut r);
    assert_eq!(r[1].effective_to, Some(date("2026-10-11")));
}
#[test]
fn a_temporary_row_returns_only_to_a_row_in_force_on_its_end() {
    let promo = |from: &str| {
        let mut p = row(9, from, Some("us"), RowState::Draft);
        p.price = Some(PriceData::PerUnit { rate: dec("4") });
        p
    };
    // The value's chain ended before the promo ends: no return row, one closed row.
    let mut ended = vec![
        row(1, "2026-01-01", None, RowState::Approved),
        row(2, "2026-03-01", Some("us"), RowState::Approved),
    ];
    ended[1].effective_to = Some(date("2026-05-01"));
    ended[1].closed_explicitly = true;
    let out = temporary(
        &ended,
        promo("2026-10-01"),
        date("2026-10-11"),
        Uuid::from_u128(77),
    )
    .unwrap();
    assert_eq!(out.len(), 1, "an ended value chain is not revived");
    assert!(out[0].closed_explicitly);
    assert_eq!(out[0].effective_to, Some(date("2026-10-11")));
    // The value's chain starts only after the promo ends: the value falls back
    // to the default between the two, so again no return row.
    let later = vec![
        row(1, "2026-01-01", None, RowState::Approved),
        row(2, "2027-01-01", Some("us"), RowState::Approved),
    ];
    let out = temporary(
        &later,
        promo("2026-10-01"),
        date("2026-10-11"),
        Uuid::from_u128(77),
    )
    .unwrap();
    assert_eq!(
        out.len(),
        1,
        "a chain that has not started yet is not copied backwards"
    );
}
#[test]
fn decision_7_a_common_date_moves_singles_to_it_and_a_pair_by_one_delta() {
    let back = row(1, "2026-01-01", None, RowState::Approved);
    let mut promo = row(2, "2026-10-01", None, RowState::Draft);
    promo.price = Some(PriceData::PerUnit { rate: dec("4") });
    let pair = temporary(&[back], promo, date("2026-10-11"), Uuid::from_u128(3)).unwrap();
    let single = row(5, "2026-11-01", None, RowState::Draft);
    let mut selection = pair;
    selection.push(single);
    let moved = shift_selection(&selection, Some(date("2026-10-05"))).unwrap();
    assert_eq!(moved[0].effective_from, date("2026-10-05"));
    assert_eq!(moved[0].temporary_until, Some(date("2026-10-15")));
    assert_eq!(moved[0].effective_to, Some(date("2026-10-15")));
    assert_eq!(
        moved[1].effective_from,
        date("2026-10-15"),
        "the return keeps the pair's length"
    );
    assert_eq!(moved[1].effective_to, None);
    assert_eq!(moved[2].effective_from, date("2026-10-05"));
    assert_eq!(
        shift_selection(&selection, None).unwrap(),
        selection,
        "no common date moves nothing"
    );
}
#[test]
fn the_predecessor_is_the_same_chain_row_in_force_the_day_before() {
    let mut chain = vec![
        row(1, "2026-01-01", None, RowState::Approved),
        row(2, "2026-02-01", Some("us"), RowState::Approved),
        row(3, "2026-06-01", Some("us"), RowState::Approved),
    ];
    normalize_windows(&mut chain);
    assert_eq!(in_force_before(&chain, &chain[2]).unwrap().version_no, 2);
    assert!(
        in_force_before(&chain, &chain[1]).is_none(),
        "the default chain is not a value's predecessor"
    );
    chain[1].effective_to = Some(date("2026-04-01"));
    chain[1].closed_explicitly = true;
    assert!(
        in_force_before(&chain, &chain[2]).is_none(),
        "an ended value row does not precede a later start"
    );
}
#[test]
fn every_refusal_code_names_its_input_field() {
    for (code, field) in [
        ("MODEL_KIND_CHARGEKIND_MISMATCH", "model"),
        ("CHAIN_MODEL_CHANGED", "model"),
        ("WINDOW_OVERLAP", "effective_from"),
        ("WINDOW_END_INVALID", "temporary_until"),
        ("DIM_VALUE_UNKNOWN", "dim_value"),
        ("MIN_FEE_INVALID", "min_fee"),
        ("PAIR_SPLIT", "row_ids"),
        ("AMOUNT_INVALID", "price"),
    ] {
        assert_eq!(field_of(code), field, "{code}");
    }
}
