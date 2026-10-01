//! Price book entry writes and their durable registry operations.
//!
//! @cpt-dod:cpt-cf-bss-pricing-dod-entry-key-unique:p1
//! @cpt-dod:cpt-cf-bss-pricing-dod-entry-metadata:p1
//! @cpt-dod:cpt-cf-bss-pricing-dod-entry-reference-handoff:p1
use super::{
    AuthoringState,
    dto::{PricingPriceBookEntryCreate, PricingPriceBookEntryDto, PricingPriceBookEntryPatch},
    support::{self, DoorError},
};
use crate::{
    domain::{
        price::PriceState,
        price_book_entry,
        reference_op::{OpKind, RefKind},
    },
    infra::{
        reference_work::{self, Caller, EntryInput, Receipt, Ref, Target, WallClock, Work},
        storage::{
            entity,
            repo::{
                book_repo, dimension_repo, idempotency_repo as idem, plan_item_repo,
                price_book_entry_repo, price_repo, reference_op_repo,
            },
        },
    },
};
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use toolkit_canonical_errors::CanonicalError;
use toolkit_db::secure::{AccessScope, DBRunner};
use toolkit_security::SecurityContext;
use uuid::Uuid;
pub(super) async fn find(
    tx: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
) -> Result<entity::price_book_entry::Model, DoorError> {
    price_book_entry_repo::find(tx, scope, tenant, id)
        .await?
        .ok_or_else(|| support::missing_entry().into())
}
fn validate_template(input: Option<&str>) -> Result<(), CanonicalError> {
    if let Some(template) = input {
        price_book_entry::validate_template(template)
            .map_err(|e| support::invalid("invoice_line_override", e.code))?;
    }
    Ok(())
}
enum Begun {
    Replay(Receipt),
    Op(Uuid),
}
/// What a held Idempotency-Key answers: `None` when this call holds it (or may take it).
pub(super) fn settled(
    claim: idem::IdempotencyClaim,
    digest: &[u8],
) -> Result<Option<Receipt>, CanonicalError> {
    // The op's answer is stored as its whole receipt (D-429).
    support::held(claim, digest)?
        .map(|(_, receipt)| {
            serde_json::from_value(receipt)
                .map_err(|_| CanonicalError::internal("invalid entry receipt").create())
        })
        .transpose()
}
/// The key's stored answer, read without claiming it: a replay or an in-flight duplicate is
/// answered from the store alone, before any Products call.
pub(super) async fn stored(
    state: &AuthoringState,
    tenant: Uuid,
    endpoint: &str,
    key: &str,
    digest: &[u8],
) -> Result<Option<Receipt>, CanonicalError> {
    let conn = state.db.conn().map_err(DoorError::from)?;
    match idem::lookup_idempotency_key(
        &conn,
        &AccessScope::for_tenant(tenant),
        tenant,
        endpoint,
        key,
        crate::infra::storage::stored_now(),
    )
    .await
    .map_err(DoorError::from)?
    {
        Some(claim) => settled(claim, digest),
        None => Ok(None),
    }
}
/// The period and model rules need the SKU's type, read before anything is claimed or reserved:
/// an input refusal is 400 and costs no reservation (D-403). The model is required (D-427): an
/// unknown one is `MODEL_INVALID` before any read, one the charge kind does not allow is
/// `MODEL_KIND_CHARGEKIND_MISMATCH` (a bundle SKU is left to the reservation's own refusal, 409
/// `BUNDLE_SKU_NOT_PRICEABLE`). Tx B judges both again against the type the reservation froze. A
/// registry that cannot answer is 503 with nothing written; a definite Products refusal is
/// answered as Products gave it.
async fn check_sku_rules(
    state: &AuthoringState,
    ctx: &SecurityContext,
    input: &PricingPriceBookEntryCreate,
) -> Result<(), CanonicalError> {
    let model: price_book_entry::Model = input
        .model
        .parse()
        .map_err(|_| support::invalid("model", "MODEL_INVALID"))?;
    let registry = crate::infra::reference_registry::resolve(&state.hub)
        .map_err(|e| support::registry_unavailable(&e))?;
    let sku = registry
        .sku_for_write(ctx, ctx.subject_tenant_id(), input.sku_id)
        .await
        .map_err(|error| {
            if reference_work::definite_refusal(&error) {
                error
            } else {
                support::registry_unavailable(&error)
            }
        })?;
    if !price_book_entry::period_valid(sku.r#type, input.period.as_deref()) {
        return Err(support::invalid("period", "ENTRY_PERIOD_INVALID"));
    }
    match price_book_entry::charge_kind_for(sku.r#type) {
        Ok(kind) if !price_book_entry::model_allowed(kind, model) => {
            Err(support::invalid("model", "MODEL_KIND_CHARGEKIND_MISMATCH"))
        }
        _ => Ok(()),
    }
}
/// A named dimension key must be declared in the tenant's registry (the seed key counts while
/// the tenant stores none; the entry write stores it).
async fn check_dimension(
    tx: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    key: Option<&str>,
) -> Result<(), DoorError> {
    if let Some(key) = key
        && !dimension_repo::declared(tx, scope, tenant, key).await?
    {
        return Err(support::invalid("dimension_key", "DIM_NOT_DECLARED").into());
    }
    Ok(())
}
#[expect(
    clippy::too_many_arguments,
    reason = "authorized door identity and replay operands"
)]
pub(super) async fn create(
    state: Arc<AuthoringState>,
    scope: AccessScope,
    ctx: SecurityContext,
    book: Uuid,
    correlation: Uuid,
    key: String,
    digest: Vec<u8>,
    input: PricingPriceBookEntryCreate,
) -> Result<Response, CanonicalError> {
    let original_ctx = ctx.clone();
    let endpoint = format!("/bss-pricing/v1/price-books/{book}/entries");
    if let Some(receipt) = stored(&state, ctx.subject_tenant_id(), &endpoint, &key, &digest).await?
    {
        return receipt.response();
    }
    check_sku_rules(&state, &ctx, &input).await?;
    let result = support::transaction(&state.db.db(), move |tx| {
        let (scope, ctx, key, digest, input, endpoint) = (
            scope.clone(),
            ctx.clone(),
            key.clone(),
            digest.clone(),
            input.clone(),
            endpoint.clone(),
        );
        Box::pin(async move {
            let tenant = ctx.subject_tenant_id();
            let now = crate::infra::storage::stored_now();
            let receipt_scope = AccessScope::for_tenant(tenant);
            let claim = idem::claim_idempotency_key(
                tx,
                &receipt_scope,
                tenant,
                &endpoint,
                &key,
                &digest,
                now,
                now + time::Duration::hours(24),
            )
            .await?;
            if let Some(receipt) = settled(claim, &digest)? {
                return Ok(Begun::Replay(receipt));
            }
            validate_template(input.invoice_line_override.as_deref())?;
            check_dimension(tx, &receipt_scope, tenant, input.dimension_key.as_deref()).await?;
            if book_repo::find(tx, &scope, tenant, book).await?.is_none() {
                return Err(support::missing().into());
            }
            let reference = Ref {
                kind: RefKind::Entry,
                id: Uuid::now_v7(),
                sku_id: input.sku_id,
            };
            let work = Work {
                target: Target::PriceBookEntry {
                    book_id: book,
                    input: EntryInput::from(input),
                },
                correlation,
                refusal: None,
                receipt: None,
                outcome: None,
            };
            let op = reference_work::new_op(
                &ctx,
                reference,
                &work,
                OpKind::Create,
                None,
                Some(key.clone()),
                now,
            )?;
            let id = op.op_id;
            reference_op_repo::insert(tx, &receipt_scope, op).await?;
            idem::bind_op(tx, &receipt_scope, tenant, &endpoint, &key, id).await?;
            Ok(Begun::Op(id))
        })
    })
    .await?;
    match result {
        Begun::Replay(receipt) => receipt.response(),
        Begun::Op(id) => {
            reference_work::drive(&state, &original_ctx, id, Arc::new(WallClock), Caller::Door)
                .await?
                .ok_or_else(|| CanonicalError::internal("missing create receipt").create())?
                .response()
        }
    }
}
pub(super) async fn patch(
    tx: &impl DBRunner,
    scope: &AccessScope,
    ctx: &SecurityContext,
    correlation: Uuid,
    id: Uuid,
    version: u64,
    input: PricingPriceBookEntryPatch,
) -> Result<Response, DoorError> {
    let tenant = ctx.subject_tenant_id();
    let mut m = find(tx, scope, tenant, id).await?;
    support::check_version(version, m.version)?;
    // The entry is the authorized aggregate; its prices are read tenant-scoped, never through a
    // scope narrowed to the entry's id.
    let prices = price_repo::for_entry(tx, &AccessScope::for_tenant(tenant), tenant, id).await?;
    if let Some(dimension) = input.dimension_key {
        check_dimension(tx, scope, tenant, dimension.as_deref()).await?;
        if dimension != m.dimension_key && prices.iter().any(|r| r.dim_value.is_some()) {
            return Err(support::conflict("DIMENSION_KEY_IN_USE").into());
        }
        m.dimension_key = dimension;
    }
    if let Some(template) = input.invoice_line_override {
        validate_template(template.as_deref())?;
        // D-426: the override reaches consumers through resolve (D-421), so once the entry carries
        // money — an approved or a pending price — its invoice line no longer changes; another line
        // is another entry.
        let carries_money = prices.iter().any(|r| {
            r.state == PriceState::Approved.as_str() || r.state == PriceState::Pending.as_str()
        });
        if template != m.invoice_line_override && carries_money {
            return Err(support::conflict("INVOICE_LINE_LOCKED").into());
        }
        m.invoice_line_override = template;
    }
    m.updated_at = crate::infra::storage::stored_now();
    price_book_entry_repo::update(tx, scope, m.clone()).await?;
    m.version += 1;
    support::audit(
        tx,
        ctx,
        correlation,
        "price_book_entry.patch",
        id,
        m.version,
    )
    .await?;
    Ok(support::response(
        StatusCode::OK,
        &PricingPriceBookEntryDto::try_from(m)?,
        Some(version + 1),
    )?)
}
pub(super) async fn delete(
    state: Arc<AuthoringState>,
    scope: AccessScope,
    ctx: SecurityContext,
    correlation: Uuid,
    id: Uuid,
) -> Result<Response, CanonicalError> {
    let original_ctx = ctx.clone();
    let op_id = support::transaction(&state.db.db(), move |tx| {
        let (scope, ctx) = (scope.clone(), ctx.clone());
        Box::pin(async move {
            let tenant = ctx.subject_tenant_id();
            let m = find(tx, &scope, tenant, id).await?;
            // A pending create must complete before deletion, otherwise its confirm could lose its entry.
            if m.reference_state
                == crate::domain::price_book_entry::ReferenceState::ConfirmationPending.as_str()
            {
                return Err(support::conflict("ENTRY_CONFIRMATION_PENDING").into());
            }
            // D-408: an entry a plan item names is in use, whatever its revision's state: a draft
            // may still be submitted, and published and superseded revisions keep their items
            // (D-414). Judged here, in the delete's transaction.
            if plan_item_repo::names_entry(tx, &AccessScope::for_tenant(tenant), tenant, id).await?
            {
                return Err(support::conflict("ENTRY_IN_USE").into());
            }
            // Approved or pending money blocks deletion; drafts and rejected proposals go with
            // the entry (a rejected price's history stays in its unit's snapshot).
            let prices = price_repo::for_entry(tx, &scope, tenant, id).await?;
            if prices.iter().any(|price| {
                !matches!(
                    price.state.parse(),
                    Ok(PriceState::Draft | PriceState::Rejected)
                ) || price.pending_unit_id.is_some()
            }) {
                return Err(support::conflict("ENTRY_PRICES_IN_USE").into());
            }
            // D-404: a draft belongs to its author, and deleting the entry would delete it. A
            // rejected price is history (its unit's snapshot keeps it) and never blocks.
            if let Some(foreign) = prices.iter().find(|price| {
                price.state == PriceState::Draft.as_str() && price.created_by != ctx.subject_id()
            }) {
                return Err(support::forbidden_because(
                    "NOT_DRAFT_AUTHOR",
                    format!("price {} is a draft of another author", foreign.id),
                )
                .into());
            }
            let prices: Vec<_> = prices
                .iter()
                .map(|price| (price.id, price.version))
                .collect();
            price_repo::delete_unapproved(tx, &scope, tenant, &prices).await?;
            let reference = Ref {
                kind: RefKind::Entry,
                id,
                sku_id: m.sku_id,
            };
            let work = Work {
                target: Target::PriceBookEntry {
                    book_id: m.book_id,
                    input: EntryInput::of(&m),
                },
                correlation,
                refusal: None,
                receipt: None,
                outcome: None,
            };
            let op = reference_work::new_op(
                &ctx,
                reference,
                &work,
                OpKind::Delete,
                Some(m.reservation_id),
                None,
                crate::infra::storage::stored_now(),
            )?;
            let op_id = op.op_id;
            price_book_entry_repo::delete_empty(tx, &scope, tenant, id, m.version).await?;
            reference_op_repo::insert(tx, &AccessScope::for_tenant(tenant), op).await?;
            support::audit(
                tx,
                &ctx,
                correlation,
                "price_book_entry.delete",
                id,
                m.version,
            )
            .await?;
            Ok(op_id)
        })
    })
    .await?;
    // The entry is gone once the transaction commits: answer 204. The release is durable work;
    // what this door does not finish, the ticker does.
    if let Err(error) = reference_work::drive(
        &state,
        &original_ctx,
        op_id,
        Arc::new(WallClock),
        Caller::Door,
    )
    .await
    {
        tracing::warn!(op_id=%op_id, error=%error, diagnostic=error.diagnostic().unwrap_or_default(), "pricing entry release deferred to the ticker");
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}
/// The two prices an entry read headlines (D-440, D-472), each `None` when there is none.
#[derive(Default)]
pub(super) struct Headline {
    /// The default chain's approved price in force on the day.
    pub current: Option<super::dto::PricingPriceDto>,
    /// The default chain's next price after the day ([`next_of`]).
    pub next: Option<super::dto::PricingPriceDto>,
}
/// The default chain's next price after `day` (D-472), of one entry's default-chain prices: its
/// earliest approved price that starts after the day (a scheduled price; two approved prices of
/// a chain never share a start, and the highest `version_no` would win as it wins in force), else
/// its newest draft or pending price — the highest `version_no`, then the latest `created_at`,
/// then the highest id — else `None`. A rejected price is never one.
fn next_of<'a>(
    chain: &[&'a entity::price::Model],
    day: time::Date,
) -> Option<&'a entity::price::Model> {
    let is = |p: &entity::price::Model, state: PriceState| p.state == state.as_str();
    chain
        .iter()
        .copied()
        .filter(|p| is(p, PriceState::Approved) && p.effective_from > day)
        .min_by(|a, b| {
            a.effective_from
                .cmp(&b.effective_from)
                .then(b.version_no.cmp(&a.version_no))
        })
        .or_else(|| {
            chain
                .iter()
                .copied()
                .filter(|p| is(p, PriceState::Draft) || is(p, PriceState::Pending))
                .max_by_key(|p| (p.version_no, p.created_at, p.id))
        })
}
/// The headline prices of each of `entries` on `day` (D-434, D-440, D-472), from ONE read of the
/// entries' default-chain approved, pending and draft prices (no dimension value, never a
/// rejected price): the price in force — started on or before the day and not ended, chosen as
/// resolve chooses a price in force (the latest start, then the latest version) — and the next
/// price ([`next_of`]), each with its status on the day; an entry with neither has no key. The
/// caller has judged whose money it may show: every entry given is shown.
/// # Errors
/// Storage failures; a stored token outside its closed set is a corrupt row.
pub(super) async fn headline(
    tx: &impl DBRunner,
    tenant: Uuid,
    entries: &[&entity::price_book_entry::Model],
    day: time::Date,
) -> Result<std::collections::BTreeMap<Uuid, Headline>, DoorError> {
    use crate::infra::storage::RepoError;
    let ids: Vec<Uuid> = entries.iter().map(|e| e.id).collect();
    let stored =
        price_repo::default_chain(tx, &AccessScope::for_tenant(tenant), tenant, &ids).await?;
    // Each entry's chain by key, grouped once: linear in the book, not entries x prices (PS-38).
    let mut chains: std::collections::BTreeMap<Uuid, Vec<&entity::price::Model>> =
        std::collections::BTreeMap::new();
    for p in &stored {
        chains.entry(p.price_book_entry_id).or_default().push(p);
    }
    let mut out = std::collections::BTreeMap::new();
    for e in entries {
        let model = price_book_entry_repo::model_of(e)?;
        let chain: Vec<&entity::price::Model> = chains.get(&e.id).cloned().unwrap_or_default();
        // Only the approved prices are decoded: they alone can be in force.
        let approved = chain
            .iter()
            .filter(|p| p.state == PriceState::Approved.as_str())
            .map(|p| price_repo::to_domain(p, model))
            .collect::<Result<Vec<_>, RepoError>>()?;
        let current = crate::domain::price::own_version_at(&approved, e.id, day, None)
            .and_then(|found| chain.iter().find(|p| p.id == found.id));
        let shown = |p: Option<&&entity::price::Model>| {
            p.map(|p| super::dto::PricingPriceDto::at((*p).clone(), &e.model, day))
                .transpose()
        };
        let found = Headline {
            current: shown(current)?,
            next: shown(next_of(&chain, day).as_ref())?,
        };
        if found.current.is_some() || found.next.is_some() {
            out.insert(e.id, found);
        }
    }
    Ok(out)
}
/// Whether the caller's `price_book` read — `books`, or `None` without that grant — admits the
/// tenant's `book`: the money's second judgement (D-434), ONE read.
/// # Errors
/// Storage failures.
pub(super) async fn shows_money(
    tx: &impl DBRunner,
    books: Option<&AccessScope>,
    tenant: Uuid,
    book: Uuid,
) -> Result<bool, DoorError> {
    Ok(match books {
        Some(books) => book_repo::find(tx, books, tenant, book).await?.is_some(),
        None => false,
    })
}
/// `GET /price-book-entries/{id}` and each item of `GET /price-books/{id}/entries` (D-428,
/// D-440, D-472): the entries with their usage and, when `shown`, their price in force and their
/// next price — all dated on `day`, today or the list's `as_of` (D-473), in a fixed number of
/// statements whatever their number.
/// # Errors
/// Storage failures; a stored token outside its closed set is a corrupt row.
pub(super) async fn read(
    tx: &impl DBRunner,
    tenant: Uuid,
    entries: Vec<entity::price_book_entry::Model>,
    shown: bool,
    day: time::Date,
) -> Result<Vec<super::dto::PricingPriceBookEntryReadDto>, DoorError> {
    let ids: Vec<Uuid> = entries.iter().map(|m| m.id).collect();
    let mut usage = crate::infra::usage::entry_usage(tx, tenant, &ids, day).await?;
    let mut headlines = if shown {
        headline(tx, tenant, &entries.iter().collect::<Vec<_>>(), day).await?
    } else {
        std::collections::BTreeMap::new()
    };
    entries
        .into_iter()
        .map(|m| {
            let (counted, prices) = (
                usage.remove(&m.id).unwrap_or_default(),
                headlines.remove(&m.id).unwrap_or_default(),
            );
            Ok(super::dto::PricingPriceBookEntryReadDto::of(
                m,
                counted,
                prices.current,
                prices.next,
            )?)
        })
        .collect()
}
/// `GET /price-book-entries/{id}/prices` (D-440): every price of an entry the caller's `scope`
/// reaches (else 404), in every state, each with its display status on `today`, the default chain
/// first, then each dimension value's chain in ascending order, each chain by `effective_from`,
/// then `version_no` (then id), as the export orders them; only the statuses in `wanted` when
/// given. The prices are money: the caller's `price_book` read (`books`) must admit the entry's
/// book, else 403 `PRICE_BOOK_READ_REQUIRED`. Three statements: the entry, its book under the
/// grant, its prices.
/// # Errors
/// 404 `ENTRY_NOT_FOUND`, 403 `PRICE_BOOK_READ_REQUIRED`; storage failures and corrupt rows.
pub(super) async fn prices(
    tx: &impl DBRunner,
    scope: &AccessScope,
    books: Option<&AccessScope>,
    tenant: Uuid,
    id: Uuid,
    wanted: Option<&[crate::api::rest::closed_sets::PricingPriceStatus]>,
    today: time::Date,
) -> Result<super::dto::PricingEntryPriceList, DoorError> {
    let entry = find(tx, scope, tenant, id).await?;
    if !shows_money(tx, books, tenant, entry.book_id).await? {
        return Err(support::forbidden_because(
            "PRICE_BOOK_READ_REQUIRED",
            "an entry's prices are money: reading them takes price_book read on its book",
        )
        .into());
    }
    let mut rows = price_repo::for_entry(tx, &AccessScope::for_tenant(tenant), tenant, id).await?;
    rows.sort_by(|a, b| {
        (&a.dim_value, a.effective_from, a.version_no, a.id).cmp(&(
            &b.dim_value,
            b.effective_from,
            b.version_no,
            b.id,
        ))
    });
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let price = super::dto::PricingPriceDto::at(row, &entry.model, today)?;
        if wanted.is_none_or(|w| w.contains(&price.status)) {
            items.push(price);
        }
    }
    Ok(super::dto::PricingEntryPriceList { items })
}
/// `GET /price-book-entries?sku_id=` (D-434): the tenant's entries of one SKU across its books,
/// each with its book's code, name and currency, its usage (D-428, dated on `today`, D-440), and
/// the default chain's price in force today and its next price (D-472) when `books` — the scope
/// the caller's `price_book` read gives, or `None` without that grant — admits the entry's book. The entries are read under
/// the caller's entry scope, the books' names and the usage tenant-scoped (facts of an entry the
/// caller may read). A fixed number of set-based statements, whatever the number of entries; an
/// unknown SKU is an empty list, never a 404 (pricing does not know which SKUs exist).
/// # Errors
/// Storage failures; an entry whose book is gone is a corrupt row.
pub(super) async fn for_sku(
    tx: &impl DBRunner,
    scope: &AccessScope,
    books: Option<&AccessScope>,
    tenant: Uuid,
    sku: Uuid,
    today: time::Date,
) -> Result<super::dto::PricingSkuEntryList, DoorError> {
    use crate::infra::storage::RepoError;
    use std::collections::{BTreeMap, BTreeSet};
    let entries = price_book_entry_repo::for_skus(tx, scope, tenant, &[sku]).await?;
    let book_ids: Vec<Uuid> = entries
        .iter()
        .map(|e| e.book_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let tenant_scope = AccessScope::for_tenant(tenant);
    let named: BTreeMap<Uuid, entity::price_book::Model> =
        book_repo::find_many(tx, &tenant_scope, tenant, &book_ids)
            .await?
            .into_iter()
            .map(|b| (b.id, b))
            .collect();
    let ids: Vec<Uuid> = entries.iter().map(|e| e.id).collect();
    let mut usage = crate::infra::usage::entry_usage(tx, tenant, &ids, today).await?;
    // The money: only the entries whose book the caller's price_book read admits.
    let mut headlines = if let Some(books) = books {
        let readable: BTreeSet<Uuid> = book_repo::find_many(tx, books, tenant, &book_ids)
            .await?
            .into_iter()
            .map(|b| b.id)
            .collect();
        let shown: Vec<&entity::price_book_entry::Model> = entries
            .iter()
            .filter(|e| readable.contains(&e.book_id))
            .collect();
        headline(tx, tenant, &shown, today).await?
    } else {
        BTreeMap::new()
    };
    let mut items = Vec::with_capacity(entries.len());
    for e in entries {
        let book = named.get(&e.book_id).ok_or_else(|| {
            RepoError::CorruptRow(format!("entry {} names lost book {}", e.id, e.book_id))
        })?;
        let prices = headlines.remove(&e.id).unwrap_or_default();
        let entry_usage = usage.remove(&e.id).unwrap_or_default().into();
        items.push(super::dto::PricingSkuEntryDto {
            entry: e.try_into()?,
            book_code: book.code.clone(),
            book_name: book.name.clone(),
            currency: book.currency.clone(),
            usage: entry_usage,
            current_price: prices.current,
            next_price: prices.next,
        });
    }
    items.sort_by(|a, b| sku_entry_order(a).cmp(&sku_entry_order(b)));
    Ok(super::dto::PricingSkuEntryList { items })
}
/// The order of a SKU's entries (D-434): book code, then the stored tokens of charge kind, period
/// and model, then id.
fn sku_entry_order(
    i: &super::dto::PricingSkuEntryDto,
) -> (&str, &'static str, &'static str, &'static str, Uuid) {
    (
        &i.book_code,
        i.entry.charge_kind.as_str(),
        i.entry
            .period
            .map_or("", crate::api::rest::closed_sets::PricingPeriod::as_str),
        i.entry.model.as_str(),
        i.entry.id,
    )
}
