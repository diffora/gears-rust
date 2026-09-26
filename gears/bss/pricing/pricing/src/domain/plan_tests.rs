#![allow(clippy::expect_used, clippy::unwrap_used)]
//! The prototype's plan assertions (`test/rules.test.js`, "a plan reads one book …" onward) in the new
//! names, then one test per check code, then spec §8's blocked revision. The prototype's `BUNDLE_SKU`
//! (sold as a bundle) assertions wait with the sold-as bundle (D-411); `quote` and `upcoming` are not
//! built (D-415).
use super::*;
use crate::domain::{
    book::Book,
    money::PriceData,
    price::{Eligibility, Price, PriceState, normalize_windows},
    price_book_entry::{ChargeKind, Model, ReferenceState as EntryReference},
    test_support::{date, dec},
};
use bss_products_sdk::models::{Lifecycle, Sku, SkuType};
use uuid::Uuid;

const TODAY: &str = "2026-09-23";
fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
// books
const EUR: u128 = 1;
const USD: u128 = 2;
const PARTNER: u128 = 3;
const GBP: u128 = 4;
const CONTRACT: u128 = 5;
// SKUs
const WP_BASIC: u128 = 11;
const STORAGE: u128 = 12;
const STORAGE_COLD: u128 = 13;
const SUPPORT: u128 = 14;
const SUITE: u128 = 15;
const SUITE_CLOSED: u128 = 16;
// entries
const E_STORAGE: u128 = 21;
const E_USD_STORAGE: u128 = 22;
const E_WPB_M: u128 = 23;
const E_USD_WPB_M: u128 = 24;
const E_SUPPORT_Y: u128 = 25;
const E_P_WPB_M: u128 = 26;
const E_C1: u128 = 27;
const E_STORAGE_COLD: u128 = 28;
// the pending storage price's approval unit
const AP_STORAGE_V3: u128 = 91;

fn make_book(name: &str, currency: &str) -> Book {
    Book {
        name: name.into(),
        currency: currency.into(),
        valid_from: None,
        valid_until: None,
    }
}
fn books() -> Vec<PlanBook> {
    vec![
        PlanBook {
            id: id(EUR),
            book: make_book("Default EUR", "EUR"),
        },
        PlanBook {
            id: id(USD),
            book: make_book("Default USD", "USD"),
        },
        PlanBook {
            id: id(PARTNER),
            book: make_book("Partner EMEA", "EUR"),
        },
        PlanBook {
            id: id(GBP),
            book: make_book("UK list", "GBP"),
        },
        PlanBook {
            id: id(CONTRACT),
            book: Book {
                valid_from: Some(date("2026-01-01")),
                valid_until: Some(date("2026-12-31")),
                ..make_book("Acme contract", "EUR")
            },
        },
    ]
}
fn sku(n: u128, name: &str, code: &str, kind: SkuType, usage_type: Option<&str>) -> Sku {
    let at = time::OffsetDateTime::UNIX_EPOCH;
    Sku {
        id: id(n),
        tenant_id: id(0),
        code: code.into(),
        name: name.into(),
        r#type: kind,
        category_id: id(0),
        description: String::new(),
        sellable: true,
        lifecycle: Lifecycle::Published,
        revision: 1,
        published_version: 1,
        gl_code: None,
        tax_category: None,
        invoice_line_template: None,
        billing_timing: None,
        usage_type_ref: usage_type.map(str::to_owned),
        unit: usage_type.map(|_| "GB-month".to_owned()),
        type_change_pending: false,
        pending_unit_id: None,
        approved_by_unit_id: None,
        created_by: id(0),
        created_at: at,
        updated_at: at,
    }
}
fn skus() -> Vec<Sku> {
    vec![
        sku(
            WP_BASIC,
            "WordPress Basic",
            "WP-BASIC",
            SkuType::Recurring,
            None,
        ),
        sku(
            STORAGE,
            "Storage",
            "STORAGE",
            SkuType::Usage,
            Some("storage_gb_hours"),
        ),
        sku(
            STORAGE_COLD,
            "Storage (cold)",
            "STORAGE-COLD",
            SkuType::Usage,
            Some("storage_gb_hours"),
        ),
        sku(
            SUPPORT,
            "Premium Support",
            "SUPPORT-PREM",
            SkuType::Recurring,
            None,
        ),
        sku(SUITE, "Pro Suite", "PRO-SUITE", SkuType::Bundle, None),
        Sku {
            sellable: false,
            ..sku(
                SUITE_CLOSED,
                "Old Suite",
                "OLD-SUITE",
                SkuType::Bundle,
                None,
            )
        },
    ]
}
fn price(entry: u128, v: i32, state: PriceState, from: &str, dim: Option<&str>) -> Price {
    Price {
        id: Uuid::from_u128(entry * 1000 + u128::try_from(v).unwrap()),
        price_book_entry_id: id(entry),
        version_no: v,
        dim_value: dim.map(str::to_owned),
        model: Model::PerUnit,
        price: Some(PriceData::PerUnit { rate: dec("0.10") }),
        min_fee: None,
        eligibility: Eligibility::All,
        effective_from: date(from),
        effective_to: None,
        temporary_until: None,
        paired_price_id: None,
        return_of_price_id: None,
        closed_explicitly: false,
        state,
    }
}
fn entry(
    n: u128,
    book: u128,
    sku: u128,
    kind: ChargeKind,
    period: Option<&str>,
    mut prices: Vec<Price>,
    pending: Vec<PendingPrice>,
) -> Entry {
    normalize_windows(&mut prices);
    Entry {
        id: id(n),
        book_id: id(book),
        sku_id: id(sku),
        charge_kind: kind,
        period: period.map(str::to_owned),
        dimension_key: None,
        reference_state: EntryReference::Confirmed,
        prices,
        pending,
    }
}
fn open(n: u128) -> Vec<Price> {
    vec![price(n, 1, PriceState::Approved, "2026-01-01", None)]
}
fn entries() -> Vec<Entry> {
    use ChargeKind::{Recurring, Usage};
    vec![
        entry(
            E_STORAGE,
            EUR,
            STORAGE,
            Usage,
            None,
            vec![
                price(E_STORAGE, 1, PriceState::Approved, "2026-01-01", None),
                price(E_STORAGE, 2, PriceState::Approved, "2026-06-01", None),
                price(E_STORAGE, 3, PriceState::Pending, "2026-12-01", None),
            ],
            vec![PendingPrice {
                price_id: Uuid::from_u128(E_STORAGE * 1000 + 3),
                unit_id: id(AP_STORAGE_V3),
            }],
        ),
        entry(
            E_USD_STORAGE,
            USD,
            STORAGE,
            Usage,
            None,
            open(E_USD_STORAGE),
            vec![],
        ),
        entry(
            E_WPB_M,
            EUR,
            WP_BASIC,
            Recurring,
            Some("month"),
            open(E_WPB_M),
            vec![],
        ),
        entry(
            E_USD_WPB_M,
            USD,
            WP_BASIC,
            Recurring,
            Some("month"),
            open(E_USD_WPB_M),
            vec![],
        ),
        entry(
            E_SUPPORT_Y,
            EUR,
            SUPPORT,
            Recurring,
            Some("year"),
            open(E_SUPPORT_Y),
            vec![],
        ),
        entry(
            E_P_WPB_M,
            PARTNER,
            WP_BASIC,
            Recurring,
            Some("month"),
            open(E_P_WPB_M),
            vec![],
        ),
        entry(
            E_C1,
            CONTRACT,
            WP_BASIC,
            Recurring,
            Some("month"),
            open(E_C1),
            vec![],
        ),
        entry(
            E_STORAGE_COLD,
            EUR,
            STORAGE_COLD,
            Usage,
            None,
            open(E_STORAGE_COLD),
            vec![],
        ),
    ]
}
fn confirmed() -> Reference {
    Reference {
        state: ReferenceState::Confirmed,
        reservation_id: Some(id(99)),
    }
}
fn item(n: u128, sku: u128, entry: Option<u128>, treatment: Treatment) -> Item {
    Item {
        id: id(n),
        sku_id: id(sku),
        price_book_entry_id: entry.map(id),
        treatment,
        included_qty: None,
        qty_min: None,
        reference: confirmed(),
    }
}
fn included(n: u128, sku: u128, entry: Option<u128>, qty: Option<&str>) -> Item {
    Item {
        included_qty: qty.map(dec),
        ..item(n, sku, entry, Treatment::Included)
    }
}
/// A plan on `book` with `items`, in the prototype's whole world of SKUs, entries and books.
fn ctx(name: &str, book: u128, items: Vec<Item>) -> PlanContext {
    PlanContext {
        plan: Plan {
            id: id(100),
            code: name.to_lowercase(),
            name: name.into(),
        },
        revision: Revision {
            id: id(200),
            rev_no: 1,
            book_id: id(book),
            state: RevisionState::Draft,
            available_from: None,
        },
        items,
        skus: skus(),
        entries: entries(),
        books: books(),
        dimension_values: vec![],
        published_sku_ids: vec![],
        quorum: 0,
        defaults: Defaults {
            gl: Some("4000-GEN".into()),
            rounding: "half-up-2".into(),
            tax_category: Some("standard".into()),
        },
    }
}
fn basic() -> PlanContext {
    ctx(
        "Basic",
        EUR,
        vec![
            Item {
                qty_min: Some(1),
                ..item(1, WP_BASIC, Some(E_WPB_M), Treatment::Paid)
            },
            included(2, STORAGE, Some(E_STORAGE), Some("10")),
            Item {
                qty_min: Some(0),
                ..item(3, SUPPORT, Some(E_SUPPORT_Y), Treatment::Optional)
            },
        ],
    )
}
fn pro() -> PlanContext {
    ctx(
        "Pro",
        EUR,
        vec![
            Item {
                qty_min: Some(1),
                ..item(1, WP_BASIC, Some(E_WPB_M), Treatment::Paid)
            },
            included(2, STORAGE, Some(E_STORAGE), Some("100")),
            included(3, SUPPORT, None, None),
        ],
    )
}
fn pro_usd() -> PlanContext {
    ctx(
        "Pro (USD)",
        USD,
        vec![
            Item {
                qty_min: Some(1),
                ..item(1, WP_BASIC, Some(E_USD_WPB_M), Treatment::Paid)
            },
            included(2, STORAGE, Some(E_USD_STORAGE), Some("100")),
        ],
    )
}
fn today() -> time::Date {
    date(TODAY)
}
fn check(c: &PlanContext, code: &str) -> Option<Check> {
    checks(c, today()).into_iter().find(|k| k.code == code)
}
fn red(c: &PlanContext, code: &str) -> bool {
    !check(c, code)
        .unwrap_or_else(|| panic!("{code} is always evaluated here"))
        .ok
}
fn green(c: &PlanContext, code: &str) -> bool {
    check(c, code)
        .unwrap_or_else(|| panic!("{code} is always evaluated here"))
        .ok
}

// ---------------------------------------------------------------- the prototype's assertions

#[test]
fn prototype_a_plan_reads_one_book_and_sells_in_its_currency() {
    let c = basic();
    assert_eq!(currency(&c), Some("EUR"));
    assert_eq!(book(&c).unwrap().book.name, "Default EUR");
    assert!(green(&c, "PLAN_BOOK"));
    assert!(
        red(&c, "FREQUENCY_MIXED"),
        "monthly WordPress + yearly Support"
    );
    assert!(green(&c, "ITEM_UNCOVERED"));
    assert!(!ready(&checks(&c, today())));
}

#[test]
fn prototype_pro_is_ready_and_its_storage_is_covered_by_v2() {
    let c = pro();
    assert!(ready(&checks(&c, today())), "{:?}", checks(&c, today()));
    let cov = item_coverage(&c, &c.items[1], today());
    assert!(cov.ok);
    assert_eq!(cov.version_no, Some(2));
    assert_eq!(cov.detail, "EUR \u{2713} v2");
    let usd = pro_usd();
    assert!(ready(&checks(&usd, today())), "{:?}", checks(&usd, today()));
    assert_eq!(currency(&usd), Some("USD"));
}

#[test]
fn prototype_an_item_priced_in_another_book_is_refused() {
    let foreign = ctx(
        "P",
        USD,
        vec![item(1, WP_BASIC, Some(E_WPB_M), Treatment::Paid)],
    );
    let fc = check(&foreign, "ITEM_BOOK_FOREIGN").unwrap();
    assert!(!fc.ok);
    assert!(
        fc.detail
            .contains("priced in Default EUR (EUR), the plan reads Default USD (USD)"),
        "{}",
        fc.detail
    );
    assert!(!item_coverage(&foreign, &foreign.items[0], today()).ok);
    // No book, or an empty new book in another currency.
    let mut no_book = ctx("P", EUR, vec![]);
    no_book.books.clear();
    assert!(red(&no_book, "PLAN_BOOK"));
    let gbp = ctx(
        "P",
        GBP,
        vec![item(1, WP_BASIC, Some(E_WPB_M), Treatment::Paid)],
    );
    assert!(red(&gbp, "ITEM_BOOK_FOREIGN"));
}

#[test]
fn prototype_two_items_on_one_usage_type_is_a_double_charge() {
    let dup = ctx(
        "Dup",
        EUR,
        vec![
            item(1, STORAGE, Some(E_STORAGE), Treatment::Paid),
            item(2, STORAGE_COLD, Some(E_STORAGE_COLD), Treatment::Paid),
        ],
    );
    assert!(red(&dup, "METER_DUPLICATE"));
}

#[test]
fn prototype_a_bundle_sku_is_never_an_item() {
    let with_bundle = ctx("P", EUR, vec![included(1, SUITE, None, None)]);
    assert!(red(&with_bundle, "ITEM_BUNDLE_SKU"));
}

#[test]
fn prototype_an_item_charges_the_way_its_sku_is_typed() {
    let clash = ctx(
        "P",
        EUR,
        vec![item(1, SUPPORT, Some(E_STORAGE), Treatment::Paid)],
    );
    assert!(red(&clash, "CHARGE_KIND_SKU_TYPE"));
}

#[test]
fn prototype_a_contract_book_sells_only_inside_its_validity() {
    let contract = ctx(
        "P",
        CONTRACT,
        vec![item(1, WP_BASIC, Some(E_C1), Treatment::Paid)],
    );
    assert!(green(&contract, "PLAN_BOOK_VALIDITY"));
    let mut late = contract;
    late.revision.available_from = Some(date("2027-01-05"));
    assert!(red(&late, "PLAN_BOOK_VALIDITY"));
    assert!(
        check(&pro(), "PLAN_BOOK_VALIDITY").is_none(),
        "a book without validity adds no check"
    );
}

#[test]
fn prototype_plan_from_and_plan_billing() {
    let c = basic();
    assert_eq!(sale_date(&c.revision, today()), today(), "at publish");
    let mut dated = c.revision.clone();
    dated.available_from = Some(date("2026-10-01"));
    assert_eq!(sale_date(&dated, today()), date("2026-10-01"));
    assert_eq!(billing(&c), vec!["month".to_owned(), "year".to_owned()]);
    assert_eq!(billing(&pro()), vec!["month".to_owned()]);
}

// ---------------------------------------------------------------- the check list

#[test]
fn the_check_list_is_the_plans_codes_in_order() {
    let c = ctx(
        "P",
        CONTRACT,
        vec![item(1, WP_BASIC, Some(E_C1), Treatment::Paid)],
    );
    let codes: Vec<_> = checks(&c, today()).into_iter().map(|k| k.code).collect();
    assert_eq!(
        codes,
        vec![
            "PLAN_NAME",
            "PLAN_BOOK",
            "PLAN_BOOK_VALIDITY",
            "PLAN_ITEMS",
            "ITEM_ENTRY_MISSING",
            "ITEM_ENTRY_SKU_MISMATCH",
            "ITEM_ENTRY_LOST",
            "ITEM_BUNDLE_SKU",
            "CHARGE_KIND_SKU_TYPE",
            "ITEM_BOOK_FOREIGN",
            "ITEM_UNCOVERED",
            "FREQUENCY_MIXED",
            "METER_DUPLICATE",
            "INCLUDED_QTY",
            "ITEM_SKU_DEPRECATED",
            "ITEM_SKU_UNAVAILABLE",
            "ITEM_REFERENCE_PENDING",
            "ITEM_REFERENCE_LOST",
            "DESCRIPTORS",
            "APPROVAL",
        ]
    );
    assert!(ready(&checks(&c, today())), "{:?}", checks(&c, today()));
}

#[test]
fn plan_name_is_required() {
    let mut c = pro();
    c.plan.name = "  ".into();
    assert!(red(&c, "PLAN_NAME"));
    assert!(green(&pro(), "PLAN_NAME"));
}

#[test]
fn plan_book_is_the_revisions_book() {
    let mut c = pro();
    c.books.retain(|b| b.id != id(EUR));
    assert!(red(&c, "PLAN_BOOK"));
    assert!(!item_coverage(&c, &c.items[0], today()).ok);
}

#[test]
fn plan_book_validity_is_judged_on_the_sale_date() {
    let mut c = ctx(
        "P",
        CONTRACT,
        vec![item(1, WP_BASIC, Some(E_C1), Treatment::Paid)],
    );
    c.revision.available_from = Some(date("2026-12-31"));
    assert!(red(&c, "PLAN_BOOK_VALIDITY"), "the end is exclusive");
    c.revision.available_from = Some(date("2026-12-30"));
    assert!(green(&c, "PLAN_BOOK_VALIDITY"));
}

#[test]
fn plan_items_needs_at_least_one() {
    assert!(red(&ctx("P", EUR, vec![]), "PLAN_ITEMS"));
    assert!(green(&pro(), "PLAN_ITEMS"));
}

#[test]
fn item_entry_missing_is_a_priced_item_without_an_entry() {
    let c = ctx("P", EUR, vec![item(1, WP_BASIC, None, Treatment::Paid)]);
    assert!(red(&c, "ITEM_ENTRY_MISSING"));
    let c = ctx(
        "P",
        EUR,
        vec![item(1, WP_BASIC, Some(777), Treatment::Optional)],
    );
    assert!(red(&c, "ITEM_ENTRY_MISSING"), "an entry the context lacks");
    assert!(
        green(&pro(), "ITEM_ENTRY_MISSING"),
        "an included item may name none"
    );
    let cov = item_coverage(&pro(), &pro().items[2], today());
    assert!(cov.ok);
    assert_eq!(cov.detail, "no charge");
}

#[test]
fn item_entry_sku_mismatch_is_an_entry_of_another_sku() {
    let c = ctx(
        "P",
        EUR,
        vec![item(1, STORAGE_COLD, Some(E_STORAGE), Treatment::Paid)],
    );
    assert!(red(&c, "ITEM_ENTRY_SKU_MISMATCH"));
    assert!(green(&pro(), "ITEM_ENTRY_SKU_MISMATCH"));
}

#[test]
fn item_entry_lost_is_an_entry_whose_reference_is_lost() {
    let mut c = pro();
    c.entries
        .iter_mut()
        .find(|e| e.id == id(E_WPB_M))
        .unwrap()
        .reference_state = EntryReference::Lost;
    assert!(red(&c, "ITEM_ENTRY_LOST"));
    assert!(green(&pro(), "ITEM_ENTRY_LOST"));
}

#[test]
fn item_bundle_sku_is_red_for_a_bundle_item() {
    let c = ctx("P", EUR, vec![included(1, SUITE_CLOSED, None, None)]);
    assert!(red(&c, "ITEM_BUNDLE_SKU"));
    assert!(green(&pro(), "ITEM_BUNDLE_SKU"));
}

#[test]
fn charge_kind_sku_type_compares_the_entry_kind_with_the_fresh_sku_type() {
    let mut c = pro();
    // The SKU was retyped: its entry still charges recurring.
    c.skus
        .iter_mut()
        .find(|s| s.id == id(WP_BASIC))
        .unwrap()
        .r#type = SkuType::OneTime;
    assert!(red(&c, "CHARGE_KIND_SKU_TYPE"));
    assert!(green(&pro(), "CHARGE_KIND_SKU_TYPE"));
}

#[test]
fn item_book_foreign_is_an_entry_of_another_book_even_in_the_same_currency() {
    let c = ctx(
        "P",
        EUR,
        vec![item(1, WP_BASIC, Some(E_P_WPB_M), Treatment::Paid)],
    );
    let fc = check(&c, "ITEM_BOOK_FOREIGN").unwrap();
    assert!(!fc.ok);
    assert!(
        fc.detail.contains("priced in Partner EMEA (EUR)"),
        "{}",
        fc.detail
    );
    // The prototype reports a foreign item once, as foreign: it is not also uncovered.
    assert!(green(&c, "ITEM_UNCOVERED"));
}

#[test]
fn item_uncovered_names_every_pending_unit_of_the_uncovered_entry() {
    let mut c = pro();
    // Sold from 2027: storage's v2 still covers it, but move the item to a fresh entry whose
    // only prices are pending.
    let fresh = entry(
        31,
        EUR,
        STORAGE,
        ChargeKind::Usage,
        None,
        vec![
            price(31, 1, PriceState::Pending, "2026-10-01", None),
            price(31, 2, PriceState::Pending, "2026-11-01", None),
            price(31, 3, PriceState::Draft, "2026-12-01", None),
        ],
        vec![
            PendingPrice {
                price_id: Uuid::from_u128(31_001),
                unit_id: id(502),
            },
            PendingPrice {
                price_id: Uuid::from_u128(31_002),
                unit_id: id(501),
            },
            PendingPrice {
                price_id: Uuid::from_u128(31_009),
                unit_id: id(501),
            },
        ],
    );
    c.entries.push(fresh);
    c.items[1].price_book_entry_id = Some(id(31));
    let k = check(&c, "ITEM_UNCOVERED").unwrap();
    assert!(!k.ok);
    assert_eq!(k.blocked_by, vec![id(501), id(502)], "distinct and ordered");
    assert!(k.detail.contains("Storage"), "{}", k.detail);
    let green_check = check(&pro(), "ITEM_UNCOVERED").unwrap();
    assert!(green_check.ok);
    assert!(green_check.blocked_by.is_empty());
}

#[test]
fn item_uncovered_is_judged_per_dimension_value_with_the_default_as_fallback() {
    let region = || vec![("region".to_owned(), vec!["eu".to_owned(), "us".to_owned()])];
    let valued = |prices: Vec<Price>, pending: Vec<PendingPrice>| {
        let mut e = entry(32, EUR, STORAGE, ChargeKind::Usage, None, prices, pending);
        e.dimension_key = Some("region".into());
        e
    };
    let with = |e: Entry| {
        let mut c = ctx("P", EUR, vec![included(1, STORAGE, Some(32), Some("100"))]);
        c.dimension_values = region();
        c.entries.push(e);
        c
    };
    // EU covered by its own chain, US by nothing: US is named.
    let eu_only = with(valued(
        vec![price(32, 1, PriceState::Approved, "2026-01-01", Some("eu"))],
        vec![],
    ));
    let k = check(&eu_only, "ITEM_UNCOVERED").unwrap();
    assert!(!k.ok);
    assert!(k.detail.contains("region us"), "{}", k.detail);
    assert!(!k.detail.contains("eu,"), "{}", k.detail);
    // Complete own-value chains pass without a default.
    let both = with(valued(
        vec![
            price(32, 1, PriceState::Approved, "2026-01-01", Some("eu")),
            price(32, 2, PriceState::Approved, "2026-01-01", Some("us")),
        ],
        vec![],
    ));
    assert!(green(&both, "ITEM_UNCOVERED"));
    let cov = item_coverage(&both, &both.items[0], today());
    assert_eq!(cov.detail, "EUR \u{2713} v1 \u{b7} 2 \u{d7} region");
    // The default chain covers a value without its own, so a pending DEFAULT price blocks it too.
    let pending_default = with(valued(
        vec![
            price(32, 1, PriceState::Approved, "2026-01-01", Some("eu")),
            price(32, 2, PriceState::Pending, "2026-01-01", None),
        ],
        vec![PendingPrice {
            price_id: Uuid::from_u128(32_002),
            unit_id: id(601),
        }],
    ));
    assert_eq!(
        check(&pending_default, "ITEM_UNCOVERED")
            .unwrap()
            .blocked_by,
        vec![id(601)]
    );
    // A value chain that closes, with no default to carry on, is not an open tail.
    let mut closing = vec![
        price(32, 1, PriceState::Approved, "2026-01-01", Some("eu")),
        price(32, 2, PriceState::Approved, "2026-01-01", Some("us")),
    ];
    closing[1].effective_to = Some(date("2026-12-01"));
    closing[1].closed_explicitly = true;
    let k = check(&with(valued(closing, vec![])), "ITEM_UNCOVERED").unwrap();
    assert!(!k.ok);
    assert!(k.detail.contains("last window closes"), "{}", k.detail);
}

#[test]
fn item_uncovered_without_a_dimension_needs_a_price_on_the_sale_date_and_an_open_tail() {
    let mut early = pro();
    early.revision.available_from = Some(date("2025-12-01"));
    let k = check(&early, "ITEM_UNCOVERED").unwrap();
    assert!(!k.ok);
    assert!(
        k.detail.contains("no approved price on 2025-12-01"),
        "{}",
        k.detail
    );
    let mut closed = pro();
    let e = closed
        .entries
        .iter_mut()
        .find(|e| e.id == id(E_WPB_M))
        .unwrap();
    e.prices[0].effective_to = Some(date("2027-01-01"));
    e.prices[0].closed_explicitly = true;
    let k = check(&closed, "ITEM_UNCOVERED").unwrap();
    assert!(!k.ok);
    assert!(
        k.detail.contains("last window closes 2027-01-01"),
        "{}",
        k.detail
    );
}

#[test]
fn frequency_mixed_is_two_recurring_periods() {
    assert!(red(&basic(), "FREQUENCY_MIXED"));
    assert!(green(&pro(), "FREQUENCY_MIXED"));
}

#[test]
fn meter_duplicate_counts_an_included_item_without_an_entry() {
    let c = ctx(
        "P",
        EUR,
        vec![
            item(1, STORAGE, Some(E_STORAGE), Treatment::Paid),
            included(2, STORAGE_COLD, None, Some("5")),
        ],
    );
    assert!(red(&c, "METER_DUPLICATE"));
    assert!(green(&pro(), "METER_DUPLICATE"));
}

#[test]
fn included_qty_is_required_on_included_usage_and_refused_elsewhere() {
    let missing = ctx("P", EUR, vec![included(1, STORAGE, Some(E_STORAGE), None)]);
    assert!(red(&missing, "INCLUDED_QTY"));
    let on_recurring = ctx("P", EUR, vec![included(1, SUPPORT, None, Some("1"))]);
    assert!(red(&on_recurring, "INCLUDED_QTY"));
    let on_paid_recurring = ctx(
        "P",
        EUR,
        vec![Item {
            included_qty: Some(dec("2")),
            ..item(1, WP_BASIC, Some(E_WPB_M), Treatment::Paid)
        }],
    );
    assert!(red(&on_paid_recurring, "INCLUDED_QTY"));
    assert!(green(&pro(), "INCLUDED_QTY"));
    let zero = ctx(
        "P",
        EUR,
        vec![included(1, STORAGE, Some(E_STORAGE), Some("0"))],
    );
    assert!(green(&zero, "INCLUDED_QTY"), "zero is a quantity");
}

#[test]
fn item_sku_deprecated_may_only_be_carried_over_within_the_same_plan() {
    let deprecated = |published: Vec<Uuid>| {
        let mut c = pro();
        c.skus
            .iter_mut()
            .find(|s| s.id == id(WP_BASIC))
            .unwrap()
            .lifecycle = Lifecycle::Deprecated;
        c.published_sku_ids = published;
        c
    };
    assert!(
        green(&deprecated(vec![id(WP_BASIC)]), "ITEM_SKU_DEPRECATED"),
        "carried over from this plan's published revision"
    );
    assert!(
        red(&deprecated(vec![id(STORAGE)]), "ITEM_SKU_DEPRECATED"),
        "added now"
    );
    assert!(
        red(&deprecated(vec![]), "ITEM_SKU_DEPRECATED"),
        "a clone is a new plan with no published revision"
    );
    assert!(green(
        &deprecated(vec![id(WP_BASIC)]),
        "ITEM_SKU_UNAVAILABLE"
    ));
}

#[test]
fn item_sku_unavailable_is_a_draft_retiring_retired_or_unknown_sku() {
    for lifecycle in [Lifecycle::Draft, Lifecycle::Retiring, Lifecycle::Retired] {
        let mut c = pro();
        c.skus
            .iter_mut()
            .find(|s| s.id == id(STORAGE))
            .unwrap()
            .lifecycle = lifecycle;
        assert!(red(&c, "ITEM_SKU_UNAVAILABLE"), "{lifecycle:?}");
    }
    let mut unknown = pro();
    unknown.skus.retain(|s| s.id != id(SUPPORT));
    assert!(red(&unknown, "ITEM_SKU_UNAVAILABLE"));
    assert!(green(&pro(), "ITEM_SKU_UNAVAILABLE"));
}

#[test]
fn item_reference_pending_until_every_item_holds_a_receipt() {
    let with_item = |reference: Reference| {
        let mut c = pro();
        c.items[0].reference = reference;
        c
    };
    assert!(red(
        &with_item(Reference {
            state: ReferenceState::Unreserved,
            reservation_id: None,
        }),
        "ITEM_REFERENCE_PENDING"
    ));
    assert!(red(
        &with_item(Reference {
            state: ReferenceState::ConfirmationPending,
            reservation_id: None,
        }),
        "ITEM_REFERENCE_PENDING"
    ));
    assert!(
        green(
            &with_item(Reference {
                state: ReferenceState::ConfirmationPending,
                reservation_id: Some(id(98)),
            }),
            "ITEM_REFERENCE_PENDING"
        ),
        "confirmation_pending with a receipt is enough (D-413)"
    );
    assert!(green(&pro(), "ITEM_REFERENCE_PENDING"));
}

#[test]
fn item_reference_lost_is_an_item_whose_reference_is_lost() {
    let lost = Reference {
        state: ReferenceState::Lost,
        reservation_id: Some(id(97)),
    };
    let mut item_lost = pro();
    item_lost.items[1].reference = lost;
    assert!(red(&item_lost, "ITEM_REFERENCE_LOST"));
    assert!(green(&pro(), "ITEM_REFERENCE_LOST"));
}

#[test]
fn descriptors_and_approval_are_information_rows() {
    let c = pro();
    let d = check(&c, "DESCRIPTORS").unwrap();
    assert!(d.ok && d.info);
    assert!(d.detail.contains("4000-GEN"), "{}", d.detail);
    assert!(d.detail.contains("half-up-2"), "{}", d.detail);
    let a = check(&c, "APPROVAL").unwrap();
    assert!(a.ok && a.info);
    assert_eq!(a.label, "Approval quorum 0");
    assert!(a.detail.contains("publishes directly"), "{}", a.detail);
    let mut two = pro();
    two.quorum = 2;
    assert!(
        check(&two, "APPROVAL")
            .unwrap()
            .detail
            .contains("needs 2 independent approver(s)")
    );
    // Information never blocks: a plan red elsewhere is red for that reason alone.
    assert!(
        checks(&c, today())
            .iter()
            .filter(|k| k.info)
            .all(|k| k.ok && k.blocked_by.is_empty())
    );
}

// ---------------------------------------------------------------- spec §8

/// Spec §8: "Pro rev 5 vs rev 4 · +1 item · Submit disabled: `ITEM_UNCOVERED` · `blockedBy` ap-12";
/// the operator publishes the book's changes (ap-12), the reviewer approves, the revision's
/// coverage turns green.
#[test]
fn spec_8_a_revision_blocked_by_a_pending_price_unit_turns_green_once_it_is_approved() {
    let ap_12 = id(12);
    let mut c = pro();
    c.revision.rev_no = 5;
    c.revision.available_from = Some(date("2026-10-01"));
    let mut add_on = entry(
        40,
        EUR,
        STORAGE_COLD,
        ChargeKind::Usage,
        None,
        vec![price(40, 1, PriceState::Pending, "2026-10-01", None)],
        vec![PendingPrice {
            price_id: Uuid::from_u128(40_001),
            unit_id: ap_12,
        }],
    );
    c.entries.push(add_on.clone());
    c.items.push(Item {
        qty_min: Some(0),
        ..item(4, STORAGE_COLD, Some(40), Treatment::Optional)
    });
    // Storage and cold storage share a meter; this revision replaces storage with cold storage.
    c.items.remove(1);
    let blocked = checks(&c, today());
    let k = blocked.iter().find(|k| k.code == "ITEM_UNCOVERED").unwrap();
    assert!(!k.ok);
    assert_eq!(k.blocked_by, vec![ap_12]);
    assert!(!ready(&blocked));
    // ap-12 is approved: its price is approved and the unit no longer holds it.
    add_on.prices[0].state = PriceState::Approved;
    normalize_windows(&mut add_on.prices);
    add_on.pending.clear();
    c.entries.retain(|e| e.id != id(40));
    c.entries.push(add_on);
    let covered = checks(&c, today());
    assert!(ready(&covered), "{covered:?}");
    assert!(
        covered
            .iter()
            .find(|k| k.code == "ITEM_UNCOVERED")
            .unwrap()
            .blocked_by
            .is_empty()
    );
}
