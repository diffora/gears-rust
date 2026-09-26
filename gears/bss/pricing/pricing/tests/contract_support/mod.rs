//! The golden consumer contracts (run 4.4; D-419…D-422; spec §10): what Rating and Subscriptions
//! test their adapters against. ONE body for both tiers — `tests/contract.rs` runs it on `SQLite`
//! and alone re-records (`UPDATE_CONTRACT_GOLDEN=1`), `tests/postgres_contract.rs` runs it on
//! Postgres and only compares. The goldens live in `tests/contract/`, one file per contract.
//!
//! @cpt-dod:cpt-cf-bss-pricing-dod-consumer-golden-contracts:p1
//!
//! The fixture is a calendar at FIXED past dates written through the REPOSITORIES: every door
//! refuses a start before today (`WINDOW_START_IN_PAST`), so a door-built fixture cannot hold a
//! fixed calendar. Nothing here reads `now()`. Every response is read through the two consumer
//! doors, `GET /resolve` and `GET /prices/{id}`, and normalised by [`normalise`] alone: each id
//! becomes the fixture name it was minted under and each timestamp becomes `<timestamp>`.
//!
//! The calendar (tenant settings: timing `arrears`, rounding `half_even`, GL `9000`, tax `std`,
//! invoice-line templates for `recurring` and `usage`; dimension `region` = eu, us, apac, latam —
//! apac and latam registered after revision 2 was published, `mena` registered before and
//! removed, never priced):
//!
//! | price | chain | money | window | eligibility | approved by |
//! |---|---|---|---|---|---|
//! | `price:10` | pro, default | flat 10.00 | 2026-01-01 → 2026-11-01 | all | `unit:prices-1` |
//! | `price:12` | pro, default | flat 12.00 | 2026-11-01 → 2026-12-01, `keep_for_bound` | all | `unit:prices-3` |
//! | `price:15` | pro, default | flat 15.00 | 2026-12-01 → open | new | `unit:prices-3` |
//! | `price:storage-default` | storage, default | per unit 0.10, min fee 5.00 | 2026-01-01 → open | all | `unit:prices-1` |
//! | `price:storage-eu` | storage, eu | per unit 0.08 | 2026-01-01 → open | all | `unit:prices-1` |
//! | `price:storage-us` | storage, us | per unit 0.09 | 2026-10-01 → open | all | `unit:prices-3` |
//! | `price:storage-apac` | storage, apac | per unit 0.11 | 2026-10-01 → open | new | `unit:prices-3` |
//! | `price:storage-latam-temp` | storage, latam | per unit 0.05 | 2026-09-01 → 2026-09-20, temporary, no chain to return to | all | `unit:prices-2` |
//! | `price:egress-eu`, `price:egress-us` | egress, eu / us (no default price) | per unit 0.02 / 0.03 | 2026-01-01 → open | all | `unit:prices-1` |
//! | `price:egress-apac-temp` | egress, apac | per unit 0.01 | 2026-09-10 → 2026-09-20, temporary | all | `unit:prices-2` |
//! | `price:archive` | archive (an entry no revision names) | per unit 0.01 | 2026-01-01 → open | all | `unit:prices-1` |
//! | `price:pro-draft`, `-pending`, `-rejected` | pro, default | flat 18.00 / 19.00 / 20.00 | 2027 | all | — (`unit:prices-4` holds the pending one) |
//!
//! Plan `pro` on book `eur`: revision 1 (published 2026-05-20, superseded 2026-08-20) holds pro
//! (paid), storage (paid) and legacy (included, a SKU Products no longer knows); revision 2
//! (published 2026-08-20) holds pro (paid, `qty_min` 1), storage (paid), egress (optional) and
//! backup (included, 100 units, no entry); revision 3 is a draft. Plan `trial` revision 1 is
//! pending. Products holds pro v1 from 2026-01-01 (GL 4000) and v2 from 2026-10-01 (GL 4100),
//! and one version each of storage, egress and backup from 2026-01-01. Another tenant holds one
//! approved price, `price:other-tenant`.
#![allow(dead_code)]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
use crate::plan_support::{Catalog, Fixture, entry_support::user_of};
use bss_approval::{Store, Unit, UnitState};
use bss_pricing::infra::storage::{
    RepoError,
    entity::{
        dimension_key, plan, plan_item, plan_revision, price, price_book, price_book_entry,
        settings,
    },
    repo::{
        approval_repo::PricingApprovalStore, book_repo, dimension_repo, plan_item_repo, plan_repo,
        plan_revision_repo, price_book_entry_repo, price_repo, settings_repo,
    },
};
use bss_products_sdk::models::{BillingTiming, Lifecycle, SkuContent, SkuType, SkuVersion};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use time::{Date, Month, OffsetDateTime, format_description::well_known::Rfc3339};
use toolkit_db::secure::AccessScope;
use toolkit_security::SecurityContext;
use uuid::Uuid;

/// Every golden, one file each under `tests/contract/`.
pub const GOLDENS: [&str; 13] = [
    "resolve_signup",
    "resolve_renewal_walk",
    "resolve_ended_chain",
    "resolve_default_pin_moves",
    "resolve_matrix_uncovered",
    "resolve_sku_version_by_date",
    "resolve_invoice_inputs",
    "resolve_superseded_revision",
    "resolve_refusals",
    "price_approved_open",
    "price_closed",
    "price_keep_for_bound",
    "price_not_found",
];

// ------------------------------------------------------------------ names and the normaliser

/// The fixture's ids: each minted under one name, in minting order (a revision lists its items
/// in id order, so the order the fixture mints them in is the order the doors answer).
#[derive(Default)]
pub struct Names {
    by_id: BTreeMap<Uuid, String>,
    by_name: BTreeMap<String, Uuid>,
    minted: u128,
}
impl Names {
    /// A new id under `name`, shown as `<name>` wherever a response or a request carries it.
    pub fn mint(&mut self, name: &str) -> Uuid {
        self.minted += 1;
        let id = Uuid::from_u128(0x0192_6000_0000_7000_8000_0000_0000_0000 | self.minted);
        assert!(
            self.by_name.insert(name.to_owned(), id).is_none(),
            "{name} is minted twice"
        );
        self.by_id.insert(id, format!("<{name}>"));
        id
    }
    pub fn id(&self, name: &str) -> Uuid {
        *self
            .by_name
            .get(name)
            .unwrap_or_else(|| panic!("the fixture has no {name}"))
    }
}
/// THE normaliser, for every body and every request line: each id becomes its fixture name and
/// each timestamp becomes `<timestamp>`. An id the fixture never minted fails the contract: a
/// golden may not freeze an id it cannot name.
pub fn normalise(names: &Names, value: &Value) -> Value {
    match value {
        Value::String(text) => Value::String(normalise_text(names, text)),
        Value::Array(items) => Value::Array(items.iter().map(|v| normalise(names, v)).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(k, v)| (k.clone(), normalise(names, v)))
                .collect(),
        ),
        other => other.clone(),
    }
}
fn normalise_text(names: &Names, text: &str) -> String {
    if OffsetDateTime::parse(text, &Rfc3339).is_ok() {
        return "<timestamp>".to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some((at, id)) = rest
        .char_indices()
        .find_map(|(i, _)| Some((i, Uuid::try_parse(rest.get(i..i + 36)?).ok()?)))
    {
        out.push_str(&rest[..at]);
        out.push_str(
            names
                .by_id
                .get(&id)
                .unwrap_or_else(|| panic!("an id the fixture never minted: {id} in {text:?}")),
        );
        rest = &rest[at + 36..];
    }
    out.push_str(rest);
    out
}

// ------------------------------------------------------------------ the fixed calendar

fn at(year: i32, month: u8, day: u8) -> OffsetDateTime {
    Date::from_calendar_date(year, Month::try_from(month).unwrap(), day)
        .unwrap()
        .with_hms(9, 0, 0)
        .unwrap()
        .assume_utc()
}
fn date(text: &str) -> Date {
    Date::parse(text, &time::format_description::well_known::Iso8601::DATE).unwrap()
}
/// A calendar day `(year, month, day)`.
type When = (i32, u8, u8);
/// A unit by fixture name, with the day it was decided (or, pending, submitted).
type UnitAt = (&'static str, When);
/// When each unit was decided (or, pending, submitted).
const PRICES_1: When = (2025, 12, 15);
const PRICES_2: When = (2026, 8, 28);
const PRICES_3: When = (2026, 9, 5);
const PRICES_4: When = (2026, 9, 20);

/// The fixture, and the principals who read it.
pub struct World {
    pub f: Fixture,
    pub names: Names,
    /// A user of another tenant.
    pub stranger: SecurityContext,
}
struct Writer<'a> {
    f: &'a Fixture,
    names: &'a mut Names,
    tenant: Uuid,
    scope: AccessScope,
}
/// One stored price.
struct P {
    name: &'static str,
    entry: &'static str,
    version_no: i32,
    dim: Option<&'static str>,
    model: &'static str,
    price: Value,
    min_fee: Option<&'static str>,
    from: &'static str,
    to: Option<&'static str>,
    eligibility: &'static str,
    keep: bool,
    closed: bool,
    temporary_until: Option<&'static str>,
    state: &'static str,
    /// The approving unit of an approved price, the holding unit of a pending one.
    unit: Option<UnitAt>,
}
impl P {
    fn approved(
        name: &'static str,
        entry: &'static str,
        version_no: i32,
        model: &'static str,
        price: Value,
        from: &'static str,
        unit: UnitAt,
    ) -> Self {
        Self {
            name,
            entry,
            version_no,
            dim: None,
            model,
            price,
            min_fee: None,
            from,
            to: None,
            eligibility: "all",
            keep: false,
            closed: false,
            temporary_until: None,
            state: "approved",
            unit: Some(unit),
        }
    }
}
/// When the fixture's rows were created.
fn created() -> OffsetDateTime {
    at(2025, 12, 1)
}
impl Writer<'_> {
    async fn unit(&mut self, name: &str, kind: &str, state: UnitState, when: When) -> Uuid {
        let id = self.names.mint(name);
        let (tenant, scope, kind) = (self.tenant, self.scope.clone(), kind.to_owned());
        let submitted = at(when.0, when.1, when.2 - 1);
        let decided = (state != UnitState::Pending).then(|| at(when.0, when.1, when.2));
        price_repo::transaction(&self.f.db.db(), move |tx| {
            let (scope, kind) = (scope.clone(), kind.clone());
            Box::pin(async move {
                PricingApprovalStore {
                    scope,
                    tenant_id: tenant,
                }
                .insert_unit(
                    tx,
                    &Unit {
                        id,
                        tenant_id: tenant,
                        ref_type: kind.clone(),
                        kind,
                        ref_id: id,
                        state,
                        common_effective_date: None,
                        quorum_required: 1,
                        generation: 1,
                        submitted_by: Uuid::nil(),
                        submitted_at: submitted,
                        decided_at: decided,
                        decided_note: None,
                        snapshot: json!({}),
                        snapshot_hash: "fixture".into(),
                        version: 1,
                    },
                    &[],
                )
                .await
                .map_err(|e| RepoError::Db(e.to_string()))
            })
        })
        .await
        .unwrap();
        id
    }
    async fn book(&mut self, name: &str, code: &str) -> Uuid {
        let id = self.names.mint(name);
        book_repo::insert(
            &self.f.db.conn().unwrap(),
            &self.scope,
            price_book::Model {
                id,
                tenant_id: self.tenant,
                code: code.into(),
                name: format!("Default {}", code.to_uppercase()),
                currency: "EUR".into(),
                valid_from: None,
                valid_until: None,
                version: 1,
                created_at: created(),
                updated_at: created(),
            },
        )
        .await
        .unwrap();
        id
    }
    async fn entry(
        &mut self,
        name: &str,
        book: &str,
        sku: &str,
        kind: &str,
        shape: (Option<&str>, Option<&str>, Option<&str>),
    ) {
        let (period, key, line) = shape;
        let id = self.names.mint(name);
        price_book_entry_repo::insert(
            &self.f.db.conn().unwrap(),
            &self.scope,
            price_book_entry::Model {
                id,
                tenant_id: self.tenant,
                book_id: self.names.id(book),
                sku_id: self.names.id(sku),
                charge_kind: kind.into(),
                period: period.map(str::to_owned),
                dimension_key: key.map(str::to_owned),
                invoice_line_override: line.map(str::to_owned),
                reservation_id: Uuid::from_u128(id.as_u128() ^ 1),
                reference_state: "confirmed".into(),
                version: 1,
                created_at: created(),
                updated_at: created(),
            },
        )
        .await
        .unwrap();
    }
    async fn price(&mut self, p: P) {
        let id = self.names.mint(p.name);
        let unit = p.unit.map(|(unit, _)| self.names.id(unit));
        let approved_at = p.unit.map(|(_, when)| at(when.0, when.1, when.2));
        price_repo::insert(
            &self.f.db.conn().unwrap(),
            &self.scope,
            price::Model {
                id,
                tenant_id: self.tenant,
                price_book_entry_id: self.names.id(p.entry),
                version_no: p.version_no,
                dim_value: p.dim.map(str::to_owned),
                model: p.model.into(),
                price_json: p.price,
                min_fee: p.min_fee.map(str::to_owned),
                eligibility: p.eligibility.into(),
                effective_from: date(p.from),
                effective_to: p.to.map(date),
                keep_for_bound: p.keep,
                closed_explicitly: p.closed,
                temporary_until: p.temporary_until.map(date),
                paired_price_id: None,
                return_of_price_id: None,
                state: p.state.into(),
                pending_unit_id: unit.filter(|_| p.state == "pending"),
                approved_by_unit_id: unit.filter(|_| p.state == "approved"),
                note: Some("fixture".into()),
                created_by: Uuid::nil(),
                approved_at: approved_at.filter(|_| p.state == "approved"),
                version: 2,
                created_at: created(),
                updated_at: created(),
            },
        )
        .await
        .unwrap();
    }
    async fn plan(&mut self, name: &str, code: &str) -> Uuid {
        let id = self.names.mint(name);
        plan_repo::insert(
            &self.f.db.conn().unwrap(),
            &self.scope,
            plan::Model {
                id,
                tenant_id: self.tenant,
                code: code.into(),
                name: format!("Plan {code}"),
                published_rev: None,
                version: 1,
                created_by: Uuid::nil(),
                created_at: at(2026, 5, 1),
                updated_at: at(2026, 5, 1),
            },
        )
        .await
        .unwrap();
        id
    }
    /// A draft revision of `plan` on `book`.
    async fn revision(&mut self, name: &str, plan: &str, rev_no: i32, created: OffsetDateTime) {
        let id = self.names.mint(name);
        plan_revision_repo::insert(
            &self.f.db.conn().unwrap(),
            &self.scope,
            plan_revision::Model {
                id,
                tenant_id: self.tenant,
                plan_id: self.names.id(plan),
                rev_no,
                book_id: self.names.id("book:eur"),
                state: "draft".into(),
                available_from: None,
                pending_unit_id: None,
                approved_by_unit_id: None,
                published_at: None,
                version: 1,
                created_by: Uuid::nil(),
                created_at: created,
                updated_at: created,
            },
        )
        .await
        .unwrap();
    }
    async fn item(
        &mut self,
        name: &str,
        revision: &str,
        sku: &str,
        entry: Option<&str>,
        treatment: &str,
        quantities: (Option<&str>, Option<i32>),
    ) {
        let id = self.names.mint(name);
        plan_item_repo::insert(
            &self.f.db.conn().unwrap(),
            &self.scope,
            plan_item::Model {
                id,
                tenant_id: self.tenant,
                revision_id: self.names.id(revision),
                sku_id: self.names.id(sku),
                price_book_entry_id: entry.map(|e| self.names.id(e)),
                treatment: treatment.into(),
                included_qty: quantities.0.map(str::to_owned),
                qty_min: quantities.1,
                reservation_id: Some(Uuid::from_u128(id.as_u128() ^ 1)),
                reference_state: "confirmed".into(),
                version: 1,
                created_by: Uuid::nil(),
                created_at: at(2026, 5, 2),
                updated_at: at(2026, 5, 2),
            },
        )
        .await
        .unwrap();
    }
    /// Lock the draft under `unit`, as its submit does.
    async fn lock(&self, revision: &str, unit: &str) {
        let conn = self.f.db.conn().unwrap();
        let id = self.names.id(revision);
        let version = plan_revision_repo::find(&conn, &self.scope, self.tenant, id)
            .await
            .unwrap()
            .unwrap()
            .version;
        assert!(
            plan_revision_repo::try_lock(
                &conn,
                &self.scope,
                self.tenant,
                id,
                self.names.id(unit),
                version
            )
            .await
            .unwrap()
        );
    }
    /// Publish a locked revision as its applied unit does: supersede the published one, publish,
    /// project `published_rev`.
    async fn publish(&self, plan: &str, revision: &str, unit: &str, when: OffsetDateTime) {
        self.lock(revision, unit).await;
        let conn = self.f.db.conn().unwrap();
        let (plan, revision) = (self.names.id(plan), self.names.id(revision));
        if let Some(previous) = plan_revision_repo::for_plan(&conn, &self.scope, self.tenant, plan)
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.state == "published")
        {
            plan_revision_repo::supersede(
                &conn,
                &self.scope,
                self.tenant,
                previous.id,
                previous.version,
                when,
            )
            .await
            .unwrap();
        }
        plan_revision_repo::publish(
            &conn,
            &self.scope,
            self.tenant,
            revision,
            self.names.id(unit),
            when,
        )
        .await
        .unwrap();
        let rev_no = plan_revision_repo::find(&conn, &self.scope, self.tenant, revision)
            .await
            .unwrap()
            .unwrap()
            .rev_no;
        let version = plan_repo::find(&conn, &self.scope, self.tenant, plan)
            .await
            .unwrap()
            .unwrap()
            .version;
        plan_repo::set_published(&conn, &self.scope, self.tenant, plan, version, rev_no, when)
            .await
            .unwrap();
    }
}

/// One SKU version Products holds, published on a fixed date.
fn version(catalog: &Catalog, sku: Uuid, published_version: i64, from: &str, content: SkuContent) {
    let mut versions = catalog.versions.lock().unwrap();
    let chain = versions
        .get_or_insert_with(BTreeMap::new)
        .entry(sku)
        .or_default();
    chain.push(SkuVersion {
        sku_id: sku,
        published_version,
        effective_from: date(from),
        content,
        created_at: at(2025, 12, 1),
    });
    chain.sort_by_key(|v| v.effective_from);
}
#[allow(clippy::too_many_arguments)]
fn content(
    code: &str,
    r#type: SkuType,
    gl_code: Option<&str>,
    tax_category: Option<&str>,
    invoice_line_template: Option<&str>,
    billing_timing: Option<BillingTiming>,
    meter: Option<&str>,
) -> SkuContent {
    SkuContent {
        code: code.to_owned(),
        name: code.to_owned(),
        r#type,
        category_id: Uuid::nil(),
        description: String::new(),
        sellable: true,
        gl_code: gl_code.map(str::to_owned),
        tax_category: tax_category.map(str::to_owned),
        invoice_line_template: invoice_line_template.map(str::to_owned),
        billing_timing,
        usage_type_ref: meter.map(|m| format!("gts.cf.core.uc.usage_record.v1~cf.test.{m}.v1")),
        unit: meter.map(|_| "GB".to_owned()),
    }
}

fn flat(amount: &str) -> Value {
    json!({ "amount": amount })
}
fn rate(rate: &str) -> Value {
    json!({ "rate": rate })
}
const U1: UnitAt = ("unit:prices-1", PRICES_1);
const U2: UnitAt = ("unit:prices-2", PRICES_2);
const U3: UnitAt = ("unit:prices-3", PRICES_3);

/// Products: which SKUs it knows, and their dated versions.
fn products(names: &mut Names, catalog: &Catalog) {
    for (sku, r#type, meter) in [
        ("sku:pro", SkuType::Recurring, None),
        ("sku:storage", SkuType::Usage, Some("storage")),
        ("sku:egress", SkuType::Usage, Some("egress")),
        ("sku:backup", SkuType::Usage, Some("backup")),
    ] {
        let id = names.mint(sku);
        catalog.put(id, r#type, Lifecycle::Published, meter);
    }
    names.mint("sku:legacy");
    names.mint("sku:archive");
    let pro = content(
        "pro",
        SkuType::Recurring,
        Some("4000"),
        None,
        Some("Pro subscription"),
        Some(BillingTiming::Advance),
        None,
    );
    version(catalog, names.id("sku:pro"), 1, "2026-01-01", pro.clone());
    version(
        catalog,
        names.id("sku:pro"),
        2,
        "2026-10-01",
        SkuContent {
            gl_code: Some("4100".into()),
            ..pro
        },
    );
    version(
        catalog,
        names.id("sku:storage"),
        1,
        "2026-01-01",
        content(
            "storage",
            SkuType::Usage,
            None,
            Some("reduced"),
            None,
            None,
            Some("storage"),
        ),
    );
    version(
        catalog,
        names.id("sku:egress"),
        1,
        "2026-01-01",
        content(
            "egress",
            SkuType::Usage,
            Some("4300"),
            None,
            Some("Egress {dimension}, {unit}"),
            None,
            Some("egress"),
        ),
    );
    version(
        catalog,
        names.id("sku:backup"),
        1,
        "2026-01-01",
        content(
            "backup",
            SkuType::Usage,
            None,
            None,
            None,
            None,
            Some("backup"),
        ),
    );
}
/// The tenant's settings and dimension registry, its price units, its book and entries.
async fn book_and_entries(w: &mut Writer<'_>) {
    let conn = w.f.db.conn().unwrap();
    settings_repo::insert(
        &conn,
        &w.scope,
        settings::Model {
            tenant_id: w.tenant,
            default_timing: "arrears".into(),
            default_rounding: "half_even".into(),
            default_gl: Some("9000".into()),
            default_tax_category: Some("std".into()),
            invoice_line_templates: json!({
                "recurring": "{plan}: {sku} for {period}",
                "usage": "{sku} ({dimension}) in {unit}"
            }),
            version: 1,
            created_at: at(2025, 12, 1),
            updated_at: at(2025, 12, 1),
        },
    )
    .await
    .unwrap();
    dimension_repo::insert(
        &conn,
        &w.scope,
        dimension_key::Model {
            tenant_id: w.tenant,
            key: "region".into(),
            values: json!(["eu", "us", "apac", "latam"]),
            version: 4,
        },
    )
    .await
    .unwrap();

    w.unit("unit:prices-1", "prices", UnitState::Approved, PRICES_1)
        .await;
    w.unit("unit:prices-2", "prices", UnitState::Approved, PRICES_2)
        .await;
    w.unit("unit:prices-3", "prices", UnitState::Approved, PRICES_3)
        .await;
    w.unit("unit:prices-4", "prices", UnitState::Pending, PRICES_4)
        .await;
    w.book("book:eur", "eur").await;
    w.entry(
        "entry:pro",
        "book:eur",
        "sku:pro",
        "recurring",
        (Some("month"), None, Some("Pro plan, {period}")),
    )
    .await;
    w.entry(
        "entry:storage",
        "book:eur",
        "sku:storage",
        "usage",
        (None, Some("region"), None),
    )
    .await;
    w.entry(
        "entry:egress",
        "book:eur",
        "sku:egress",
        "usage",
        (None, Some("region"), None),
    )
    .await;
    w.entry(
        "entry:archive",
        "book:eur",
        "sku:archive",
        "usage",
        (None, None, None),
    )
    .await;
}
/// Pro: spec section 7.1's chain, 10 -> all 12 -> new 15 (12 is kept for the subscriptions bound
/// to it), and three prices no consumer may see.
async fn pro_prices(w: &mut Writer<'_>) {
    w.price(P {
        to: Some("2026-11-01"),
        ..P::approved(
            "price:10",
            "entry:pro",
            1,
            "flat",
            flat("10.00"),
            "2026-01-01",
            U1,
        )
    })
    .await;
    w.price(P {
        to: Some("2026-12-01"),
        keep: true,
        ..P::approved(
            "price:12",
            "entry:pro",
            2,
            "flat",
            flat("12.00"),
            "2026-11-01",
            U3,
        )
    })
    .await;
    w.price(P {
        eligibility: "new",
        ..P::approved(
            "price:15",
            "entry:pro",
            3,
            "flat",
            flat("15.00"),
            "2026-12-01",
            U3,
        )
    })
    .await;
    for (name, version_no, amount, from, state) in [
        ("price:pro-draft", 4, "18.00", "2027-01-01", "draft"),
        ("price:pro-pending", 5, "19.00", "2027-02-01", "pending"),
        ("price:pro-rejected", 6, "20.00", "2027-03-01", "rejected"),
    ] {
        w.price(P {
            state,
            unit: (state == "pending").then_some(("unit:prices-4", PRICES_4)),
            ..P::approved(
                name,
                "entry:pro",
                version_no,
                "flat",
                flat(amount),
                from,
                U1,
            )
        })
        .await;
    }
}
/// The usage entries' chains.
async fn usage_prices(w: &mut Writer<'_>) {
    // Storage: a default with a minimum fee; eu open; us `all` and apac `new` from October; latam
    // a temporary price on a value with no chain of its own (one closed price, no return).
    w.price(P {
        min_fee: Some("5.00"),
        ..P::approved(
            "price:storage-default",
            "entry:storage",
            1,
            "per_unit",
            rate("0.10"),
            "2026-01-01",
            U1,
        )
    })
    .await;
    w.price(P {
        dim: Some("eu"),
        ..P::approved(
            "price:storage-eu",
            "entry:storage",
            2,
            "per_unit",
            rate("0.08"),
            "2026-01-01",
            U1,
        )
    })
    .await;
    w.price(P {
        dim: Some("us"),
        ..P::approved(
            "price:storage-us",
            "entry:storage",
            3,
            "per_unit",
            rate("0.09"),
            "2026-10-01",
            U3,
        )
    })
    .await;
    w.price(P {
        dim: Some("apac"),
        eligibility: "new",
        ..P::approved(
            "price:storage-apac",
            "entry:storage",
            4,
            "per_unit",
            rate("0.11"),
            "2026-10-01",
            U3,
        )
    })
    .await;
    w.price(P {
        dim: Some("latam"),
        to: Some("2026-09-20"),
        closed: true,
        temporary_until: Some("2026-09-20"),
        ..P::approved(
            "price:storage-latam-temp",
            "entry:storage",
            5,
            "per_unit",
            rate("0.05"),
            "2026-09-01",
            U2,
        )
    })
    .await;
    // Egress: no default price; eu and us open; apac a temporary price that has ended.
    for (name, version_no, dim, money) in [
        ("price:egress-eu", 1, "eu", "0.02"),
        ("price:egress-us", 2, "us", "0.03"),
    ] {
        w.price(P {
            dim: Some(dim),
            ..P::approved(
                name,
                "entry:egress",
                version_no,
                "per_unit",
                rate(money),
                "2026-01-01",
                U1,
            )
        })
        .await;
    }
    w.price(P {
        dim: Some("apac"),
        to: Some("2026-09-20"),
        closed: true,
        temporary_until: Some("2026-09-20"),
        ..P::approved(
            "price:egress-apac-temp",
            "entry:egress",
            3,
            "per_unit",
            rate("0.01"),
            "2026-09-10",
            U2,
        )
    })
    .await;
    w.price(P::approved(
        "price:archive",
        "entry:archive",
        1,
        "per_unit",
        rate("0.01"),
        "2026-01-01",
        U1,
    ))
    .await;
}
/// The revisions and their items, published as their applied units publish them.
async fn plans(w: &mut Writer<'_>) {
    // Plan pro: revision 1 superseded by revision 2; revision 3 a draft. Plan trial: pending.
    w.unit(
        "unit:revision-pro-1",
        "plan_revision",
        UnitState::Approved,
        (2026, 5, 20),
    )
    .await;
    w.unit(
        "unit:revision-pro-2",
        "plan_revision",
        UnitState::Approved,
        (2026, 8, 20),
    )
    .await;
    w.unit(
        "unit:revision-trial-1",
        "plan_revision",
        UnitState::Pending,
        (2026, 9, 12),
    )
    .await;
    w.plan("plan:pro", "pro").await;
    w.revision("revision:pro-1", "plan:pro", 1, at(2026, 5, 2))
        .await;
    w.item(
        "item:pro-1/pro",
        "revision:pro-1",
        "sku:pro",
        Some("entry:pro"),
        "paid",
        (None, None),
    )
    .await;
    w.item(
        "item:pro-1/storage",
        "revision:pro-1",
        "sku:storage",
        Some("entry:storage"),
        "paid",
        (None, None),
    )
    .await;
    w.item(
        "item:pro-1/legacy",
        "revision:pro-1",
        "sku:legacy",
        None,
        "included",
        (None, None),
    )
    .await;
    w.publish(
        "plan:pro",
        "revision:pro-1",
        "unit:revision-pro-1",
        at(2026, 5, 20),
    )
    .await;
    w.revision("revision:pro-2", "plan:pro", 2, at(2026, 8, 1))
        .await;
    w.item(
        "item:pro-2/pro",
        "revision:pro-2",
        "sku:pro",
        Some("entry:pro"),
        "paid",
        (None, Some(1)),
    )
    .await;
    w.item(
        "item:pro-2/storage",
        "revision:pro-2",
        "sku:storage",
        Some("entry:storage"),
        "paid",
        (None, None),
    )
    .await;
    w.item(
        "item:pro-2/egress",
        "revision:pro-2",
        "sku:egress",
        Some("entry:egress"),
        "optional",
        (None, None),
    )
    .await;
    w.item(
        "item:pro-2/backup",
        "revision:pro-2",
        "sku:backup",
        None,
        "included",
        (Some("100"), None),
    )
    .await;
    w.publish(
        "plan:pro",
        "revision:pro-2",
        "unit:revision-pro-2",
        at(2026, 8, 20),
    )
    .await;
    w.revision("revision:pro-3", "plan:pro", 3, at(2026, 9, 10))
        .await;
    w.plan("plan:trial", "trial").await;
    w.revision("revision:trial-1", "plan:trial", 1, at(2026, 9, 11))
        .await;
    w.lock("revision:trial-1", "unit:revision-trial-1").await;
    // Ids no row holds, so a golden can name what a refusal was asked about.
    w.names.mint("revision:unknown");
    w.names.mint("item:unknown");
    w.names.mint("price:unknown");
}
/// Another tenant: one approved price of its own.
async fn other_tenant(f: &Fixture, names: &mut Names) -> Uuid {
    let other = names.mint("tenant:other");
    let mut o = Writer {
        f,
        names,
        tenant: other,
        scope: AccessScope::for_tenant(other),
    };
    o.unit("unit:other-tenant", "prices", UnitState::Approved, PRICES_1)
        .await;
    o.book("book:other-tenant", "other").await;
    o.entry(
        "entry:other-tenant",
        "book:other-tenant",
        "sku:pro",
        "recurring",
        (Some("month"), None, None),
    )
    .await;
    o.price(P::approved(
        "price:other-tenant",
        "entry:other-tenant",
        1,
        "flat",
        flat("10.00"),
        "2026-01-01",
        ("unit:other-tenant", PRICES_1),
    ))
    .await;
    other
}

/// Write the calendar the module doc tables.
pub async fn world(f: Fixture, catalog: &Catalog) -> World {
    let mut names = Names::default();
    products(&mut names, catalog);
    let tenant = f.ctx.subject_tenant_id();
    let mut w = Writer {
        f: &f,
        names: &mut names,
        tenant,
        scope: AccessScope::for_tenant(tenant),
    };
    book_and_entries(&mut w).await;
    pro_prices(&mut w).await;
    usage_prices(&mut w).await;
    plans(&mut w).await;
    let other = other_tenant(&f, &mut names).await;
    World {
        f,
        names,
        stranger: user_of(other),
    }
}

// ------------------------------------------------------------------ the exchanges

#[derive(Clone, Copy)]
enum Caller {
    Tenant,
    OtherTenant,
}
/// One request of a golden: `path` carries `{name}` placeholders for fixture ids.
struct Ask {
    case: &'static str,
    caller: Caller,
    path: String,
}
fn ask(case: &'static str, path: &str) -> Ask {
    Ask {
        case,
        caller: Caller::Tenant,
        path: path.to_owned(),
    }
}
fn ask_as_other_tenant(case: &'static str, path: &str) -> Ask {
    Ask {
        caller: Caller::OtherTenant,
        ..ask(case, path)
    }
}
/// Replace each `{name}` of a request template with the id minted under `name`.
fn fill(names: &Names, template: &str) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let close = open + rest[open..].find('}').unwrap();
        out.push_str(&rest[..open]);
        out.push_str(&names.id(&rest[open + 1..close]).to_string());
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

/// What each golden proves, and the requests it is made of.
#[allow(clippy::too_many_lines)]
fn contract(golden: &str) -> (&'static str, Vec<Ask>) {
    const REV2: &str = "/resolve?plan_revision_id={revision:pro-2}";
    let r2 = |rest: &str| format!("{REV2}{rest}");
    match golden {
        "resolve_signup" => (
            "D-419/D-420 rule 1, D-421: a signup (no pins) of the published revision on one date - every item, the default \
             chain then every registered value in registry order, each binding the price in force (own chain, else the \
             default, dim_used saying which), an item without an entry with no chains, the SKU version in force and the \
             resolved invoice inputs with their sources; no totals",
            vec![ask("signup on 2026-10-05", &r2("&date=2026-10-05"))],
        ),
        "resolve_renewal_walk" => (
            "D-420 rule 2, spec section 7.1: pinned 10 -> all 12 -> new 15 - the renewal walks to 12 and stops before the new 15 \
             (12 is keep_for_bound), a signup on the same date binds 15",
            vec![
                ask(
                    "renewal pinned to 10 on 2026-12-05",
                    &r2("&date=2026-12-05&item_id={item:pro-2/pro}&pins={price:10}"),
                ),
                ask(
                    "signup on 2026-12-05",
                    &r2("&date=2026-12-05&item_id={item:pro-2/pro}"),
                ),
            ],
        ),
        "resolve_ended_chain" => (
            "D-420 rule 3: a binding is always in force - a pin on a temporary price whose own end has passed binds as a \
             signup would for its value: the default chain where there is one (pinned_from still names the pin), \
             uncovered where there is none",
            vec![
                ask(
                    "latam pinned to its ended temporary price; the default carries on",
                    &r2(
                        "&date=2026-10-05&item_id={item:pro-2/storage}&pins={price:storage-latam-temp}",
                    ),
                ),
                ask(
                    "apac pinned to its ended temporary price; no default price, so uncovered",
                    &r2(
                        "&date=2026-10-05&item_id={item:pro-2/egress}&pins={price:egress-apac-temp}",
                    ),
                ),
            ],
        ),
        "resolve_default_pin_moves" => (
            "D-420 rule 4 (owner 2026-09-26): a value pinned to a default-chain price moves to its own chain once the own \
             price in force is `all` and started after the pin (us, from 2026-10-01); a `new` own price does not move it \
             (apac); before the own price starts both stay on the default",
            vec![
                ask(
                    "us and apac pinned to the default on 2026-09-15",
                    &r2(
                        "&date=2026-09-15&item_id={item:pro-2/storage}&pins={price:storage-default}:us,{price:storage-default}:apac",
                    ),
                ),
                ask(
                    "us and apac pinned to the default on 2026-10-05",
                    &r2(
                        "&date=2026-10-05&item_id={item:pro-2/storage}&pins={price:storage-default}:us,{price:storage-default}:apac",
                    ),
                ),
            ],
        ),
        "resolve_matrix_uncovered" => (
            "D-419/D-420, PRD AC #18: the whole matrix - a chain nothing covers is explicit `uncovered` with no binding, \
             never a refusal and never an invented price; a value the registry no longer holds still resolves for the pin \
             that names it, after the registered values",
            vec![
                ask(
                    "an entry with no default price on 2026-10-05",
                    &r2("&date=2026-10-05&item_id={item:pro-2/egress}"),
                ),
                ask(
                    "mena, no longer registered, pinned to the default on 2026-10-05",
                    &r2(
                        "&date=2026-10-05&item_id={item:pro-2/storage}&pins={price:storage-default}:mena",
                    ),
                ),
            ],
        ),
        "resolve_sku_version_by_date" => (
            "D-421, AC dod-binding-sku-version: the SKU version is read as of the date - September binds version 1 (GL 4000) \
             although version 2 (GL 4100, from 2026-10-01) is already published; October binds version 2 while the pinned \
             price does not change",
            vec![
                ask(
                    "signup on 2026-09-15",
                    &r2("&date=2026-09-15&item_id={item:pro-2/pro}"),
                ),
                ask(
                    "renewal pinned to 10 on 2026-10-15",
                    &r2("&date=2026-10-15&item_id={item:pro-2/pro}&pins={price:10}"),
                ),
            ],
        ),
        "resolve_invoice_inputs" => (
            "D-421: each invoice input carries its source - the invoice line from the entry's override, else the SKU \
             version's template, else the tenant template for the charge kind (an item without an entry: its SKU's type), \
             else null; GL code, tax category and billing timing from the SKU version, else the tenant (PRD AC #13: advance \
             on the SKU beats arrears as the tenant default); a SKU Products does not know has no version; the revision \
             carries the book's currency and scale and the tenant's rounding",
            vec![
                ask(
                    "revision 1 on 2026-09-15: the entry's line, the tenant's line, no line at all",
                    "/resolve?plan_revision_id={revision:pro-1}&date=2026-09-15",
                ),
                ask(
                    "the SKU version's line on 2026-09-15",
                    &r2("&date=2026-09-15&item_id={item:pro-2/egress}"),
                ),
                ask(
                    "an item without an entry takes its SKU type's tenant line on 2026-09-15",
                    &r2("&date=2026-09-15&item_id={item:pro-2/backup}"),
                ),
            ],
        ),
        "resolve_superseded_revision" => (
            "D-419: a superseded revision still resolves - its own items, bound on the date as a published revision's are",
            vec![ask(
                "revision 1 on 2026-10-05",
                "/resolve?plan_revision_id={revision:pro-1}&date=2026-10-05&item_id={item:pro-1/pro}",
            )],
        ),
        "resolve_refusals" => (
            "D-419 refusals, whole request: 400 QUERY_INVALID, DATE_INVALID, PIN_FOREIGN (a pin that does not parse, a \
             forged pin on another entry's chain, a draft price, a value pin on a value-chain price, another tenant's \
             price), PIN_DUPLICATE, PINS_TOO_MANY; 404 for an unknown revision, another tenant's revision and an item the \
             revision lacks; 409 REVISION_NOT_PUBLISHED for a draft and a pending revision; and the pinned price read's 404",
            vec![
                ask("no date", REV2),
                ask(
                    "a date that is not a calendar date",
                    &r2("&date=2026-02-30"),
                ),
                ask(
                    "an unknown parameter",
                    &r2("&date=2026-10-05&as_of=2026-10-05"),
                ),
                ask(
                    "a revision id that is not an id",
                    "/resolve?plan_revision_id=pro-2&date=2026-10-05",
                ),
                ask(
                    "a pin whose value is not spelled as a value",
                    &r2("&date=2026-10-05&pins={price:10}:EU"),
                ),
                ask(
                    "a forged pin on another entry's chain",
                    &r2("&date=2026-10-05&pins={price:archive}"),
                ),
                ask(
                    "a pin on a draft price",
                    &r2("&date=2026-10-05&pins={price:pro-draft}"),
                ),
                ask(
                    "a value pin on a price that is not a default-chain price",
                    &r2("&date=2026-10-05&pins={price:storage-eu}:us"),
                ),
                ask(
                    "a pin on another tenant's price",
                    &r2("&date=2026-10-05&pins={price:other-tenant}"),
                ),
                ask(
                    "two pins for one item and value",
                    &r2("&date=2026-10-05&pins={price:10},{price:12}"),
                ),
                ask(
                    "1 001 pins",
                    &r2(&format!(
                        "&date=2026-10-05&pins={}",
                        vec!["{price:10}"; 1_001].join(",")
                    )),
                ),
                ask(
                    "an unknown revision",
                    "/resolve?plan_revision_id={revision:unknown}&date=2026-10-05",
                ),
                ask_as_other_tenant("another tenant's revision", &r2("&date=2026-10-05")),
                ask(
                    "an item of another revision",
                    &r2("&date=2026-10-05&item_id={item:pro-1/legacy}"),
                ),
                ask(
                    "a draft revision",
                    "/resolve?plan_revision_id={revision:pro-3}&date=2026-10-05",
                ),
                ask(
                    "a pending revision",
                    "/resolve?plan_revision_id={revision:trial-1}&date=2026-10-05",
                ),
                ask("an unknown price", "/prices/{price:unknown}"),
            ],
        ),
        "price_approved_open" => (
            "D-422: an approved price with an open window, as stored - its entry's SKU, charge kind and period, its book \
             and currency, the approval that applied it; no status or other value computed from today, no authoring \
             internals",
            vec![
                ask("the new 15, open", "/prices/{price:15}"),
                ask(
                    "the storage default, open, with a minimum fee",
                    "/prices/{price:storage-default}",
                ),
            ],
        ),
        "price_closed" => (
            "D-422, AC dod-price-read-forever: a price whose window has closed is served forever with its original money - \
             closed by a later price's start, or by its own end (an ended temporary price)",
            vec![
                ask("10, followed by 12", "/prices/{price:10}"),
                ask(
                    "the ended temporary latam price",
                    "/prices/{price:storage-latam-temp}",
                ),
            ],
        ),
        "price_keep_for_bound" => (
            "D-422: the predecessor of a `new` price, kept for the subscriptions bound to it, is served with its original \
             money",
            vec![ask(
                "12, kept for bound subscriptions",
                "/prices/{price:12}",
            )],
        ),
        "price_not_found" => (
            "D-422: a draft, pending or rejected price, an unknown id and another tenant's price are one and the same 404 - \
             nothing about a price is revealed",
            vec![
                ask("a draft price", "/prices/{price:pro-draft}"),
                ask("a pending price", "/prices/{price:pro-pending}"),
                ask("a rejected price", "/prices/{price:pro-rejected}"),
                ask("an unknown id", "/prices/{price:unknown}"),
                ask("another tenant's price", "/prices/{price:other-tenant}"),
                ask_as_other_tenant(
                    "this tenant's price read by another tenant",
                    "/prices/{price:10}",
                ),
            ],
        ),
        other => panic!("no contract named {other}"),
    }
}

/// A request line as the golden shows it: a run of more than three pins is counted, not listed.
fn shown(names: &Names, path: &str) -> String {
    let text = normalise_text(names, path);
    let Some((head, pins)) = text.split_once("pins=") else {
        return format!("GET /bss-pricing/v1{text}");
    };
    let listed: Vec<&str> = pins.split(',').collect();
    if listed.len() <= 3 {
        return format!("GET /bss-pricing/v1{text}");
    }
    format!(
        "GET /bss-pricing/v1{head}pins={},... ({} pins)",
        listed[0],
        listed.len()
    )
}

/// The golden document of `golden`: every request of it through the doors, normalised.
pub async fn document(w: &World, golden: &str) -> Value {
    let (proves, asks) = contract(golden);
    let mut exchanges = Vec::with_capacity(asks.len());
    for a in asks {
        let path = fill(&w.names, &a.path);
        let (ctx, caller) = match a.caller {
            Caller::Tenant => (&w.f.ctx, "the tenant"),
            Caller::OtherTenant => (&w.stranger, "another tenant"),
        };
        let (status, body, _) = w.f.call_as(ctx, "GET", &path, json!({}), None, None).await;
        exchanges.push(json!({
            "case": a.case,
            "caller": caller,
            "request": shown(&w.names, &path),
            "status": status,
            "body": normalise(&w.names, &body),
        }));
    }
    json!({ "golden": golden, "proves": proves, "exchanges": exchanges })
}

fn golden_path(golden: &str) -> String {
    format!(
        "{}/tests/contract/{golden}.json",
        env!("CARGO_MANIFEST_DIR")
    )
}
/// The first place two documents differ, as a JSON pointer with both values.
fn difference(frozen: &Value, fresh: &Value, at: &str) -> Option<String> {
    match (frozen, fresh) {
        (Value::Object(a), Value::Object(b)) => {
            for key in a.keys().chain(b.keys()) {
                let here = format!("{at}/{key}");
                match (a.get(key), b.get(key)) {
                    (Some(x), Some(y)) => {
                        if let Some(d) = difference(x, y, &here) {
                            return Some(d);
                        }
                    }
                    (x, y) => return Some(format!("{here}: frozen {x:?}, now {y:?}")),
                }
            }
            None
        }
        (Value::Array(a), Value::Array(b)) => {
            for (i, (x, y)) in a.iter().zip(b).enumerate() {
                if let Some(d) = difference(x, y, &format!("{at}/{i}")) {
                    return Some(d);
                }
            }
            (a.len() != b.len())
                .then(|| format!("{at}: frozen {} entries, now {}", a.len(), b.len()))
        }
        (a, b) => (a != b).then(|| format!("{at}: frozen {a}, now {b}")),
    }
}

/// THE contract check both tiers run: build `golden`'s document through the doors and compare it
/// with the frozen file — or, with `record` (the `SQLite` tier under `UPDATE_CONTRACT_GOLDEN=1`),
/// write it. Re-recording is a claim that the contract was MEANT to change.
pub async fn verify(w: &World, golden: &str, record: bool) {
    let fresh = document(w, golden).await;
    let path = golden_path(golden);
    if record {
        std::fs::create_dir_all(std::path::Path::new(&path).parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_string_pretty(&fresh).unwrap() + "\n").unwrap();
        return;
    }
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("no frozen contract at {path} ({e}); record one with UPDATE_CONTRACT_GOLDEN=1 on the SQLite tier")
    });
    let frozen: Value = serde_json::from_str(&text).unwrap();
    if let Some(at) = difference(&frozen, &fresh, "") {
        panic!("the {golden} contract changed at {at}");
    }
    assert_eq!(frozen, fresh, "the {golden} contract changed");
}
