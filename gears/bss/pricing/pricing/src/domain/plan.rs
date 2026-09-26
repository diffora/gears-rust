//! Plans: revisions bound to one book and their items, and the checks that decide whether a
//! revision may be submitted (D-394, D-407, D-408, D-413).
//!
//! A port of the prototype's `validatePlan`, `itemCoverage`, `planFrom` and `planBilling`
//! (`ui-prototype/pricebook/src/js/50-rules.js`). Everything here is plain data: the door reads each
//! item's SKU fresh through `sku_for_write` (D-408) and every entry with its prices, and hands the
//! lot in. The sold-as bundle and grants (D-411) and retirement (D-410) are deferred by the owner,
//! so neither `BUNDLE_SKU` nor `PLAN_RETIRING` is a check yet.
//!
//! @cpt-dod:cpt-cf-bss-pricing-dod-plan-item-rules:p1
//! @cpt-dod:cpt-cf-bss-pricing-dod-plan-coverage:p1
//! @cpt-dod:cpt-cf-bss-pricing-dod-plan-blocked-by:p1
use super::{
    book::{self, Book},
    price::{self, Price},
    price_book_entry::{ChargeKind, ReferenceState as EntryReferenceState, validate_entry_kind},
};
use bss_products_sdk::models::{Lifecycle, Sku, SkuType};
use rust_decimal::Decimal;
use std::collections::BTreeSet;
use time::Date;
use uuid::Uuid;

string_enum!(RevisionState {Draft=>"draft", Pending=>"pending", Published=>"published", Superseded=>"superseded"});
string_enum!(Treatment {Paid=>"paid", Optional=>"optional", Included=>"included"});
// A copied item starts `unreserved` and attaches after its write (D-413).
string_enum!(ReferenceState {Unreserved=>"unreserved", ConfirmationPending=>"confirmation_pending", Confirmed=>"confirmed", Lost=>"lost"});

/// The approval kind of a plan revision (spec §6); its quorum is the APPROVAL row's.
pub const KIND_PLAN_REVISION: &str = "plan_revision";
/// The most items one revision holds (`REVISION_ITEMS_TOO_MANY`).
pub const MAX_ITEMS: usize = 200;

/// The plan's identity.
#[toolkit_macros::domain_model]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub id: Uuid,
    pub code: String,
    pub name: String,
}
/// The revision under check. `available_from` null means "at publish": the sale date is today.
#[toolkit_macros::domain_model]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    pub id: Uuid,
    pub rev_no: i32,
    pub book_id: Uuid,
    pub state: RevisionState,
    pub available_from: Option<Date>,
}
/// An item's Products reference: its state and the receipt a reserve answered, if any.
#[toolkit_macros::domain_model]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub state: ReferenceState,
    pub reservation_id: Option<Uuid>,
}
/// One plan item. A null entry is an included item with no charge.
#[toolkit_macros::domain_model]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub id: Uuid,
    pub sku_id: Uuid,
    pub price_book_entry_id: Option<Uuid>,
    pub treatment: Treatment,
    pub included_qty: Option<Decimal>,
    pub qty_min: Option<i32>,
    pub reference: Reference,
}
/// A pending price and the approval unit that holds it: a candidate for `blocked_by`.
#[toolkit_macros::domain_model]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingPrice {
    pub price_id: Uuid,
    pub unit_id: Uuid,
}
/// An entry an item names, with its approved and pending prices and its reference state.
#[toolkit_macros::domain_model]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: Uuid,
    pub book_id: Uuid,
    pub sku_id: Uuid,
    pub charge_kind: ChargeKind,
    pub period: Option<String>,
    pub dimension_key: Option<String>,
    pub reference_state: EntryReferenceState,
    pub prices: Vec<Price>,
    pub pending: Vec<PendingPrice>,
}
/// A book the check reads: the revision's own, and any other book an item's entry lives in.
#[toolkit_macros::domain_model]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanBook {
    pub id: Uuid,
    pub book: Book,
}
/// The tenant's descriptor defaults, shown for information only (D-408).
#[toolkit_macros::domain_model]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Defaults {
    pub gl: Option<String>,
    pub rounding: String,
    pub tax_category: Option<String>,
}
/// Everything the checks read, as plain data.
#[toolkit_macros::domain_model]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanContext {
    pub plan: Plan,
    pub revision: Revision,
    pub items: Vec<Item>,
    /// Each item's SKU, read fresh (D-408); a SKU missing here is unavailable.
    pub skus: Vec<Sku>,
    pub entries: Vec<Entry>,
    pub books: Vec<PlanBook>,
    /// Each registered dimension key with its values.
    pub dimension_values: Vec<(String, Vec<String>)>,
    /// The item SKUs of this plan's published revision: a deprecated SKU may be carried over from
    /// there, never added (D-408). Empty for a clone, which is a new plan.
    pub published_sku_ids: Vec<Uuid>,
    pub quorum: u32,
    pub defaults: Defaults,
}
/// One check row. An `info` row is always ok and never blocks.
#[toolkit_macros::domain_model]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub code: &'static str,
    pub ok: bool,
    pub label: String,
    pub detail: String,
    pub info: bool,
    /// The approval units whose pending prices would cover what is uncovered; computed, never
    /// stored (spec §6).
    pub blocked_by: Vec<Uuid>,
}
/// Whether one item is priced on the sale date, in the plan's book.
#[toolkit_macros::domain_model]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemCoverage {
    pub ok: bool,
    pub detail: String,
    pub version_no: Option<i32>,
    pub blocked_by: Vec<Uuid>,
}

/// The sale date: `available_from`, or today for "at publish" (the prototype's `planFrom`).
#[must_use]
pub fn sale_date(revision: &Revision, today: Date) -> Date {
    revision.available_from.unwrap_or(today)
}
/// The revision's book, if the context carries it.
#[must_use]
pub fn book(ctx: &PlanContext) -> Option<&PlanBook> {
    book_by_id(ctx, ctx.revision.book_id)
}
/// The currency the plan sells in: its book's.
#[must_use]
pub fn currency(ctx: &PlanContext) -> Option<&str> {
    book(ctx).map(|b| b.book.currency.as_str())
}
/// The recurring periods the plan's items bill in (the prototype's `planBilling`).
#[must_use]
pub fn billing(ctx: &PlanContext) -> Vec<String> {
    ctx.items
        .iter()
        .filter_map(|it| entry(ctx, it.price_book_entry_id))
        .filter(|e| e.charge_kind == ChargeKind::Recurring)
        .map(|e| e.period.clone().unwrap_or_else(|| "-".to_owned()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
/// Whether every check is ok.
#[must_use]
pub fn ready(checks: &[Check]) -> bool {
    checks.iter().all(|c| c.ok)
}

fn book_by_id(ctx: &PlanContext, id: Uuid) -> Option<&PlanBook> {
    ctx.books.iter().find(|b| b.id == id)
}
fn sku_of(ctx: &PlanContext, id: Uuid) -> Option<&Sku> {
    ctx.skus.iter().find(|s| s.id == id)
}
fn entry(ctx: &PlanContext, id: Option<Uuid>) -> Option<&Entry> {
    id.and_then(|id| ctx.entries.iter().find(|e| e.id == id))
}
fn name_of(ctx: &PlanContext, item: &Item) -> String {
    sku_of(ctx, item.sku_id).map_or_else(|| item.sku_id.to_string(), |s| s.name.clone())
}
fn values_of<'a>(ctx: &'a PlanContext, e: &Entry) -> &'a [String] {
    e.dimension_key
        .as_ref()
        .and_then(|key| ctx.dimension_values.iter().find(|(k, _)| k == key))
        .map_or(&[], |(_, values)| values.as_slice())
}
/// Every approval unit holding a pending price of the entry: the default chain can cover a value,
/// so a pending default price blocks it as much as the value's own.
fn pending_units(e: &Entry) -> Vec<Uuid> {
    // @cpt-begin:cpt-cf-bss-pricing-algo-plans-revision-checks:p1:inst-plans-revision-checks-4
    e.pending
        .iter()
        .map(|p| p.unit_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
    // @cpt-end:cpt-cf-bss-pricing-algo-plans-revision-checks:p1:inst-plans-revision-checks-4
}
fn uncovered(detail: String, blocked_by: Vec<Uuid>) -> ItemCoverage {
    ItemCoverage {
        ok: false,
        detail,
        version_no: None,
        blocked_by,
    }
}

/// Coverage of one item on the sale date: per dimension value, through its own chain or the
/// default, with an open tail (the prototype's `itemCoverage`).
#[must_use]
pub fn item_coverage(ctx: &PlanContext, item: &Item, today: Date) -> ItemCoverage {
    let Some(e) = entry(ctx, item.price_book_entry_id) else {
        return if item.treatment == Treatment::Included {
            ItemCoverage {
                ok: true,
                detail: "no charge".into(),
                version_no: None,
                blocked_by: vec![],
            }
        } else {
            uncovered("no price".into(), vec![])
        };
    };
    let Some(b) = book(ctx) else {
        return uncovered("attach a price book".into(), vec![]);
    };
    if e.book_id != b.id {
        let other = book_by_id(ctx, e.book_id).map_or("another book", |o| o.book.name.as_str());
        return uncovered(format!("priced in {other}, not {}", b.book.name), vec![]);
    }
    let date = sale_date(&ctx.revision, today);
    let values = values_of(ctx, e);
    let key = e.dimension_key.as_deref().unwrap_or_default();
    // @cpt-begin:cpt-cf-bss-pricing-algo-plans-revision-checks:p1:inst-plans-revision-checks-3
    let cov = price::coverage_on(&e.prices, e.id, date, values);
    if !cov.missing.is_empty() {
        let detail = if values.is_empty() {
            format!("no approved price on {date}")
        } else {
            format!(
                "{key} {}: no price on {date} and no default price",
                cov.missing.join(", ")
            )
        };
        return uncovered(detail, pending_units(e));
    }
    if !cov.closing.is_empty() {
        let detail = if values.is_empty() {
            let end = price::approved_prices(&e.prices, e.id, None)
                .last()
                .and_then(|p| p.effective_to)
                .map_or_else(|| "?".to_owned(), |d| d.to_string());
            format!("last window closes {end}")
        } else {
            format!(
                "{key} {}: last window closes and no default carries on",
                cov.closing.join(", ")
            )
        };
        return uncovered(detail, pending_units(e));
    }
    // @cpt-end:cpt-cf-bss-pricing-algo-plans-revision-checks:p1:inst-plans-revision-checks-3
    let version_no = cov.version.map(|p| p.version_no);
    let dims = if values.is_empty() {
        String::new()
    } else {
        format!(" \u{b7} {} \u{d7} {key}", values.len())
    };
    ItemCoverage {
        ok: true,
        detail: format!(
            "{} \u{2713} v{}{dims}",
            b.book.currency,
            version_no.unwrap_or_default()
        ),
        version_no,
        blocked_by: vec![],
    }
}

/// What the item walk found, one list per check.
#[derive(Default)]
struct Tally {
    no_entry: Vec<String>,
    mismatch: Vec<String>,
    entry_lost: Vec<String>,
    bundle: Vec<String>,
    kind_clash: Vec<String>,
    foreign: Vec<String>,
    uncovered: Vec<String>,
    blocked: BTreeSet<Uuid>,
    periods: BTreeSet<String>,
    meters: Vec<(String, String)>,
    meter_dup: Vec<String>,
    included_qty: Vec<String>,
    deprecated: Vec<String>,
    unavailable: Vec<String>,
    pending: Vec<String>,
    lost: Vec<String>,
}

/// Lifecycle and reference, for every item: a fresh SKU read and a receipt (D-408, D-413).
fn tally_sku_and_reference(ctx: &PlanContext, item: &Item, name: &str, t: &mut Tally) {
    // @cpt-begin:cpt-cf-bss-pricing-algo-plans-revision-checks:p1:inst-plans-revision-checks-1
    match sku_of(ctx, item.sku_id).map(|s| s.lifecycle) {
        None => t.unavailable.push(format!("{name} - not found")),
        Some(Lifecycle::Draft | Lifecycle::Retiring | Lifecycle::Retired) => {
            t.unavailable.push(name.to_owned());
        }
        Some(Lifecycle::Deprecated) if !ctx.published_sku_ids.contains(&item.sku_id) => {
            t.deprecated.push(name.to_owned());
        }
        Some(Lifecycle::Published | Lifecycle::Deprecated) => {}
    }
    match item.reference.state {
        ReferenceState::Confirmed => {}
        ReferenceState::ConfirmationPending if item.reference.reservation_id.is_some() => {}
        ReferenceState::Lost => t.lost.push(name.to_owned()),
        ReferenceState::Unreserved | ReferenceState::ConfirmationPending => {
            t.pending.push(name.to_owned());
        }
    }
    // @cpt-end:cpt-cf-bss-pricing-algo-plans-revision-checks:p1:inst-plans-revision-checks-1
}
/// Structure (the prototype's walk up to its entry): charge kind, meter and included quantity.
fn tally_structure(sku: &Sku, e: Option<&Entry>, item: &Item, name: &str, t: &mut Tally) {
    // @cpt-begin:cpt-cf-bss-pricing-algo-plans-revision-checks:p1:inst-plans-revision-checks-1
    if let Some(e) = e
        && validate_entry_kind(e.charge_kind, sku.r#type).is_err()
    {
        t.kind_clash.push(format!(
            "{name} - entry is {}, SKU is {}",
            e.charge_kind.as_str(),
            sku.r#type.as_str()
        ));
    }
    // @cpt-end:cpt-cf-bss-pricing-algo-plans-revision-checks:p1:inst-plans-revision-checks-1
    // @cpt-begin:cpt-cf-bss-pricing-algo-plans-revision-checks:p1:inst-plans-revision-checks-2
    if let Some(meter) = &sku.usage_type_ref
        && (e.is_some() || item.treatment == Treatment::Included)
    {
        if let Some((_, first)) = t.meters.iter().find(|(m, _)| m == meter) {
            t.meter_dup.push(format!("{name} <-> {first}"));
        } else {
            t.meters.push((meter.clone(), name.to_owned()));
        }
    }
    let usage = sku.r#type == SkuType::Usage;
    let needs_quantity = item.treatment == Treatment::Included && usage;
    if needs_quantity && item.included_qty.is_none_or(|q| q < Decimal::ZERO) {
        t.included_qty
            .push(format!("{name}: set the included quantity"));
    }
    if item.included_qty.is_some() && !usage {
        t.included_qty
            .push(format!("{name}: an included quantity is for usage only"));
    }
    // @cpt-end:cpt-cf-bss-pricing-algo-plans-revision-checks:p1:inst-plans-revision-checks-2
}
/// Pricing: the entry, its book and its coverage on the sale date.
fn tally_pricing(
    ctx: &PlanContext,
    e: &Entry,
    item: &Item,
    name: &str,
    t: &mut Tally,
    today: Date,
) {
    if e.sku_id != item.sku_id {
        t.mismatch
            .push(format!("{name} - the entry prices another SKU"));
    }
    if e.reference_state == EntryReferenceState::Lost {
        t.entry_lost.push(name.to_owned());
    }
    // @cpt-begin:cpt-cf-bss-pricing-algo-plans-revision-checks:p1:inst-plans-revision-checks-2
    if e.book_id != ctx.revision.book_id {
        let describe = |id: Uuid| {
            book_by_id(ctx, id).map_or_else(
                || "? (?)".to_owned(),
                |b| format!("{} ({})", b.book.name, b.book.currency),
            )
        };
        t.foreign.push(format!(
            "{name} - priced in {}, the plan reads {}",
            describe(e.book_id),
            describe(ctx.revision.book_id)
        ));
        return;
    }
    if e.charge_kind == ChargeKind::Recurring {
        t.periods
            .insert(e.period.clone().unwrap_or_else(|| "-".to_owned()));
    }
    // @cpt-end:cpt-cf-bss-pricing-algo-plans-revision-checks:p1:inst-plans-revision-checks-2
    let cov = item_coverage(ctx, item, today);
    if !cov.ok {
        t.uncovered.push(format!("{name} - {}", cov.detail));
        t.blocked.extend(cov.blocked_by);
    }
}
fn tally(ctx: &PlanContext, today: Date) -> Tally {
    let mut t = Tally::default();
    for item in &ctx.items {
        let name = name_of(ctx, item);
        tally_sku_and_reference(ctx, item, &name, &mut t);
        let e = entry(ctx, item.price_book_entry_id);
        if let Some(sku) = sku_of(ctx, item.sku_id) {
            // @cpt-begin:cpt-cf-bss-pricing-algo-plans-revision-checks:p1:inst-plans-revision-checks-1
            if sku.r#type == SkuType::Bundle {
                t.bundle.push(name);
                continue;
            }
            // @cpt-end:cpt-cf-bss-pricing-algo-plans-revision-checks:p1:inst-plans-revision-checks-1
            tally_structure(sku, e, item, &name, &mut t);
        }
        if item.treatment == Treatment::Included && item.price_book_entry_id.is_none() {
            continue;
        }
        match e {
            Some(e) => tally_pricing(ctx, e, item, &name, &mut t, today),
            None => t.no_entry.push(name),
        }
    }
    t
}

fn row(code: &'static str, ok: bool, label: impl Into<String>, detail: impl Into<String>) -> Check {
    Check {
        code,
        ok,
        label: label.into(),
        detail: detail.into(),
        info: false,
        blocked_by: vec![],
    }
}
fn listed(found: &[String], otherwise: &str) -> String {
    if found.is_empty() {
        otherwise.to_owned()
    } else {
        found.join("; ")
    }
}
fn plan_rows(ctx: &PlanContext, sale: Date) -> Vec<Check> {
    let name = ctx.plan.name.trim();
    let mut rows = vec![row(
        "PLAN_NAME",
        !name.is_empty(),
        "Plan has a name",
        if name.is_empty() {
            "name is required"
        } else {
            name
        },
    )];
    let b = book(ctx);
    rows.push(row(
        "PLAN_BOOK",
        b.is_some(),
        "Exactly one price book attached",
        b.map_or_else(
            || {
                "a plan reads one book and sells in its currency; another currency is another plan"
                    .to_owned()
            },
            |b| format!("{} -> sells in {}", b.book.name, b.book.currency),
        ),
    ));
    if let Some(b) = b
        && (b.book.valid_from.is_some() || b.book.valid_until.is_some())
    {
        // @cpt-begin:cpt-cf-bss-pricing-algo-plans-revision-checks:p1:inst-plans-revision-checks-3
        let valid = book::valid_on(&b.book, sale);
        // @cpt-end:cpt-cf-bss-pricing-algo-plans-revision-checks:p1:inst-plans-revision-checks-3
        let window = format!(
            "{} -> {}",
            b.book
                .valid_from
                .map_or_else(|| "...".to_owned(), |d| d.to_string()),
            b.book
                .valid_until
                .map_or_else(|| "open".to_owned(), |d| d.to_string())
        );
        let detail = if valid {
            format!("{} valid {window}", b.book.name)
        } else {
            format!("{} is NOT valid on {sale} - {window}", b.book.name)
        };
        rows.push(row(
            "PLAN_BOOK_VALIDITY",
            valid,
            "The book is valid on the sale date",
            detail,
        ));
    }
    rows.push(row(
        "PLAN_ITEMS",
        !ctx.items.is_empty(),
        "At least one item",
        format!("{} item(s)", ctx.items.len()),
    ));
    rows
}
fn entry_rows(ctx: &PlanContext, t: &Tally, sale: Date) -> Vec<Check> {
    let currency = currency(ctx).unwrap_or("no book");
    let mut uncovered = row(
        "ITEM_UNCOVERED",
        t.uncovered.is_empty(),
        format!("Every item has an approved, open-ended price from {sale}"),
        listed(&t.uncovered, &format!("covered in {currency}")),
    );
    uncovered.blocked_by = t.blocked.iter().copied().collect();
    vec![
        row(
            "ITEM_ENTRY_MISSING",
            t.no_entry.is_empty(),
            "Every paid / optional item points at a price",
            listed(&t.no_entry, "all items priced"),
        ),
        row(
            "ITEM_ENTRY_SKU_MISMATCH",
            t.mismatch.is_empty(),
            "Every item's entry prices that item's SKU",
            listed(&t.mismatch, "ok"),
        ),
        row(
            "ITEM_ENTRY_LOST",
            t.entry_lost.is_empty(),
            "No item's entry has lost its Products reference",
            listed(&t.entry_lost, "ok"),
        ),
        row(
            "ITEM_BUNDLE_SKU",
            t.bundle.is_empty(),
            "No bundle SKU sits inside the plan as an item",
            listed(&t.bundle, "ok"),
        ),
        row(
            "CHARGE_KIND_SKU_TYPE",
            t.kind_clash.is_empty(),
            "Every item charges the way its SKU is typed",
            listed(&t.kind_clash, "ok"),
        ),
        row(
            "ITEM_BOOK_FOREIGN",
            t.foreign.is_empty(),
            "Every item is priced in the plan's book",
            listed(&t.foreign, "all from the plan's book"),
        ),
        uncovered,
    ]
}
fn structure_rows(t: &Tally) -> Vec<Check> {
    let periods: Vec<_> = t.periods.iter().cloned().collect();
    let frequency = match periods.as_slice() {
        [] => "no recurring items".to_owned(),
        [one] => format!("billed every {one}"),
        many => format!("found {} - one period per plan", many.join(" and ")),
    };
    vec![
        row(
            "FREQUENCY_MIXED",
            periods.len() <= 1,
            "All recurring items share one billing period",
            frequency,
        ),
        row(
            "METER_DUPLICATE",
            t.meter_dup.is_empty(),
            "No two items meter the same usage type",
            listed(&t.meter_dup, "meters unambiguous"),
        ),
        row(
            "INCLUDED_QTY",
            t.included_qty.is_empty(),
            "Included usage names a quantity",
            listed(&t.included_qty, "ok"),
        ),
    ]
}
fn sku_rows(t: &Tally) -> Vec<Check> {
    vec![
        row(
            "ITEM_SKU_DEPRECATED",
            t.deprecated.is_empty(),
            "No deprecated SKU enters the plan",
            listed(
                &t.deprecated,
                "a deprecated SKU only stays when carried over within the same plan",
            ),
        ),
        row(
            "ITEM_SKU_UNAVAILABLE",
            t.unavailable.is_empty(),
            "Every item SKU is published or deprecated",
            listed(&t.unavailable, "ok"),
        ),
        row(
            "ITEM_REFERENCE_PENDING",
            t.pending.is_empty(),
            "Every item reference holds a Products receipt",
            listed(&t.pending, "ok"),
        ),
        row(
            "ITEM_REFERENCE_LOST",
            t.lost.is_empty(),
            "No item reference is lost",
            listed(&t.lost, "ok"),
        ),
    ]
}
fn info_rows(ctx: &PlanContext) -> Vec<Check> {
    let d = &ctx.defaults;
    let descriptors = format!(
        "invoice line: entry override -> SKU -> tenant default; GL: SKU -> {}; rounding {}; tax {}",
        d.gl.as_deref().unwrap_or("none"),
        d.rounding,
        d.tax_category.as_deref().unwrap_or("none")
    );
    let approval = if ctx.quorum == 0 {
        "publishes directly - no second person".to_owned()
    } else {
        format!("needs {} independent approver(s)", ctx.quorum)
    };
    [
        row(
            "DESCRIPTORS",
            true,
            "Invoice line, GL code, tax, rounding resolved",
            descriptors,
        ),
        row(
            "APPROVAL",
            true,
            format!("Approval quorum {}", ctx.quorum),
            approval,
        ),
    ]
    .into_iter()
    .map(|c| Check { info: true, ..c })
    .collect()
}

/// Every check of a revision on its sale date (the prototype's `validatePlan`), in a fixed order.
#[must_use]
pub fn checks(ctx: &PlanContext, today: Date) -> Vec<Check> {
    let sale = sale_date(&ctx.revision, today);
    let t = tally(ctx, today);
    let mut out = plan_rows(ctx, sale);
    out.extend(entry_rows(ctx, &t, sale));
    out.extend(structure_rows(&t));
    out.extend(sku_rows(&t));
    out.extend(info_rows(ctx));
    out
}
#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
