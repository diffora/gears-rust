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
    extract::Query,
    http::{StatusCode, Uri},
    response::{IntoResponse, Response},
};
use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::sync::Arc;
use toolkit_canonical_errors::CanonicalError;
use toolkit_db::secure::{AccessScope, DBRunner};
use toolkit_odata::{
    CursorV1, ODataOrderBy, OrderKey, PageInfo, SortDir,
    filter::{FieldKind, FilterField},
};
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
) -> Result<Option<crate::infra::usage_policy_wire::MeterEvidence>, CanonicalError> {
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
    if let Ok(kind) = price_book_entry::charge_kind_for(sku.r#type) {
        match (&input.usage_rating_policy, kind) {
            (None, price_book_entry::ChargeKind::Usage) => {
                return Err(support::invalid(
                    "usage_rating_policy",
                    "MISSING_RATING_POLICY",
                ));
            }
            (
                Some(_),
                price_book_entry::ChargeKind::Recurring | price_book_entry::ChargeKind::OneTime,
            ) => {
                return Err(support::invalid(
                    "usage_rating_policy",
                    "UNEXPECTED_RATING_POLICY",
                ));
            }
            (Some(policy), _) => crate::domain::usage_policy::validate_policy_shape(&policy.into())
                .map_err(|e| support::invalid("usage_rating_policy", e.code))?,
            (None, _) => {}
        }
    }
    match price_book_entry::charge_kind_for(sku.r#type) {
        Ok(kind) if !price_book_entry::model_allowed(kind, model) => {
            Err(support::invalid("model", "MODEL_KIND_CHARGEKIND_MISMATCH"))
        }
        _ => match &input.usage_rating_policy {
            Some(policy) => crate::infra::meter_semantics::resolve(&state.hub, ctx, policy, &sku)
                .await
                .map(Some),
            None => Ok(None),
        },
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
    let evidence = check_sku_rules(&state, &ctx, &input).await?;
    let result = support::transaction(&state.db.db(), move |tx| {
        let (scope, ctx, key, digest, input, endpoint) = (
            scope.clone(),
            ctx.clone(),
            key.clone(),
            digest.clone(),
            input.clone(),
            endpoint.clone(),
        );
        let evidence = evidence.clone();
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
                    input: EntryInput {
                        meter_evidence: evidence.map(Box::new),
                        ..EntryInput::from(input)
                    },
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
        &PricingPriceBookEntryDto::load(tx, m).await?,
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
    /// The default chain's next price after the day (`domain::price::next_of`).
    pub next: Option<super::dto::PricingPriceDto>,
}
/// A stored default-chain price as the next-price rule reads it.
fn chain_price(p: &entity::price::Model) -> Result<crate::domain::price::ChainPrice, DoorError> {
    Ok(crate::domain::price::ChainPrice {
        id: p.id,
        state: p.state.parse::<PriceState>().map_err(|_| {
            crate::infra::storage::RepoError::CorruptRow(format!(
                "price {} state {}",
                p.id, p.state
            ))
        })?,
        effective_from: p.effective_from,
        version_no: p.version_no,
        created_at: p.created_at,
    })
}
/// The headline prices of each of `entries` on `day` (D-434, D-440, D-472), from ONE read of the
/// entries' default-chain approved, pending and draft prices (no dimension value, never a
/// rejected price): the price in force — started on or before the day and not ended, chosen as
/// resolve chooses a price in force (the latest start, then the latest version) — and the next
/// price (`domain::price::next_of`), each with its status on the day; an entry with neither has
/// no key. The door maps the rows; both choices are the domain's. The caller has judged whose
/// money it may show: every entry given is shown.
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
        let views = chain
            .iter()
            .map(|p| chain_price(p))
            .collect::<Result<Vec<_>, DoorError>>()?;
        let next = crate::domain::price::next_of(&views, day)
            .and_then(|found| chain.iter().find(|p| p.id == found.id));
        let found = Headline {
            current: shown(current)?,
            next: shown(next)?,
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
    let mut policies =
        crate::infra::storage::repo::usage_policy_repo::for_entries(tx, tenant, &entries).await?;
    entries
        .into_iter()
        .map(|m| {
            let id = m.id;
            let (counted, prices) = (
                usage.remove(&id).unwrap_or_default(),
                headlines.remove(&id).unwrap_or_default(),
            );
            let mut dto = super::dto::PricingPriceBookEntryReadDto::of(
                m,
                counted,
                prices.current,
                prices.next,
            )?;
            dto.entry.usage_rating_policy = policies.remove(&id);
            Ok(dto)
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
/// `GET /price-book-entries?sku_id=` (D-434, D-486): the tenant's entries of one SKU across its
/// books, each with its book's code, name and currency, its usage (D-428, dated on `today`,
/// D-440), its status and changing on that day, and the default chain's price in force today and
/// its next price (D-472) when `books` — the scope the caller's `price_book` read gives, or
/// `None` without that grant — admits the entry's book. The entries are read under the caller's
/// entry scope, the books' names and the usage tenant-scoped (facts of an entry the caller may
/// read). Narrowing, order and the page are applied in memory, so the read stays a fixed number
/// of set-based statements whatever the number of entries; an unknown SKU is an empty list, never
/// a 404 (pricing does not know which SKUs exist).
/// # Errors
/// Storage failures; an entry whose book is gone is a corrupt row. A cursor the page refuses is
/// 400, judged before this read.
pub(super) async fn for_sku(
    tx: &impl DBRunner,
    scope: &AccessScope,
    books: Option<&AccessScope>,
    tenant: Uuid,
    query: &SkuEntriesQuery,
    today: time::Date,
) -> Result<super::dto::PricingSkuEntryList, DoorError> {
    use crate::infra::storage::RepoError;
    use std::collections::{BTreeMap, BTreeSet};
    let entries = price_book_entry_repo::for_skus(tx, scope, tenant, &[query.sku]).await?;
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
    let mut policies =
        crate::infra::storage::repo::usage_policy_repo::for_entries(tx, tenant, &entries).await?;
    let mut items = Vec::with_capacity(entries.len());
    for e in entries {
        let book = named.get(&e.book_id).ok_or_else(|| {
            RepoError::CorruptRow(format!("entry {} names lost book {}", e.id, e.book_id))
        })?;
        let id = e.id;
        let prices = headlines.remove(&id).unwrap_or_default();
        let entry_usage: super::dto::PricingEntryUsage =
            usage.remove(&id).unwrap_or_default().into();
        let (status, changing) = standing(&entry_usage.prices);
        let mut entry = PricingPriceBookEntryDto::try_from(e)?;
        entry.usage_rating_policy = policies.remove(&id);
        items.push(super::dto::PricingSkuEntryDto {
            entry,
            book_code: book.code.clone(),
            book_name: book.name.clone(),
            currency: book.currency.clone(),
            usage: entry_usage,
            status,
            changing,
            current_price: prices.current,
            next_price: prices.next,
        });
    }
    page_entries(items, query)
}

/// The fields `$orderby` may name (D-486). `id` is the tie-break, not a client key, so it is not
/// declared here. `.with_odata_orderby` publishes the list; the door parses with `parse_orderby`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum SkuEntryOrderField {
    BookName,
    Status,
}
impl FilterField for SkuEntryOrderField {
    const FIELDS: &'static [Self] = &[Self::BookName, Self::Status];
    fn name(&self) -> &'static str {
        match self {
            Self::BookName => "book_name",
            Self::Status => "status",
        }
    }
    fn kind(&self) -> FieldKind {
        FieldKind::String
    }
    fn from_name(name: &str) -> Option<Self> {
        Self::FIELDS
            .iter()
            .copied()
            .find(|field| field.name() == name)
    }
}

/// A parsed `GET /price-book-entries` (D-486). Every refusal is already judged.
#[derive(Clone)]
pub(super) struct SkuEntriesQuery {
    sku: Uuid,
    book_ids: Option<BTreeSet<Uuid>>,
    currency: Option<String>,
    q: Option<String>,
    statuses: Option<Vec<crate::api::rest::closed_sets::PricingSkuEntryStatus>>,
    changing: Option<bool>,
    limit: u64,
    order: ODataOrderBy,
    cursor: Option<CursorV1>,
    hash: String,
}

const PAGE_DEFAULT: u64 = 500;
const PLAIN_KEYS: &[&str] = &[
    "sku_id", "book_id", "currency", "q", "status", "changing", "limit", "cursor",
];

/// The query of `GET /price-book-entries` (D-486), judged before any read. `$orderby` beside a
/// cursor is 400 `ORDER_WITH_CURSOR` before the cursor is decoded. A `$filter`, `$select`,
/// `$count` or any other key this read does not take is 400 `QUERY_INVALID`.
/// # Errors
/// 400 `QUERY_INVALID`, `INVALID_ORDERBY_FIELD`, `ORDER_WITH_CURSOR`, `FILTER_MISMATCH` or
/// `INVALID_CURSOR`.
pub(super) fn sku_entries_query(uri: &Uri) -> Result<SkuEntriesQuery, CanonicalError> {
    let Query(pairs) = Query::<Vec<(String, String)>>::try_from_uri(uri)
        .map_err(|_| support::invalid("query", "QUERY_INVALID"))?;
    let mut seen: Vec<&str> = Vec::new();
    for (key, _) in &pairs {
        let key = key.as_str();
        if key != "$orderby" && !PLAIN_KEYS.contains(&key) {
            return Err(support::invalid_because(
                key,
                "QUERY_INVALID",
                &format!("`{key}` is not a parameter of this read"),
            ));
        }
        if seen.contains(&key) {
            return Err(support::invalid_because(
                key,
                "QUERY_INVALID",
                &format!("`{key}` is given more than once"),
            ));
        }
        seen.push(key);
    }
    if seen.contains(&"cursor") && seen.contains(&"$orderby") {
        return Err(toolkit_odata::Error::OrderWithCursor.into());
    }
    let value = |name: &str| {
        pairs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, raw)| raw.as_str())
    };
    let sku = value("sku_id").ok_or_else(|| {
        support::invalid_because("sku_id", "QUERY_INVALID", "`sku_id` is required")
    })?;
    let sku = Uuid::parse_str(sku)
        .map_err(|_| support::invalid_because("sku_id", "QUERY_INVALID", "`sku_id` is a SKU id"))?;
    let book_ids = value("book_id").map(book_ids).transpose()?;
    let currency = value("currency").map(currency_key).transpose()?;
    let q = value("q")
        .filter(|text| !text.is_empty())
        .map(str::to_owned);
    let statuses = value("status").map(status_keys).transpose()?;
    let changing = value("changing").map(changing_key).transpose()?;
    let limit = value("limit")
        .map(|raw| {
            raw.parse::<u64>().map_err(|_| {
                support::invalid_because("limit", "QUERY_INVALID", "`limit` is a page size")
            })
        })
        .transpose()?;
    let hash = narrowing_hash(
        sku,
        book_ids.as_ref(),
        currency.as_deref(),
        q.as_deref(),
        statuses.as_deref(),
        changing,
    )?;
    let (order, cursor) = match value("cursor") {
        Some(token) => {
            let cursor = CursorV1::decode(token).map_err(CanonicalError::from)?;
            if cursor.f.as_deref() != Some(hash.as_str()) {
                return Err(toolkit_odata::Error::FilterMismatch.into());
            }
            (cursor_order(&cursor)?, Some(cursor))
        }
        None => (requested_order(value("$orderby"))?, None),
    };
    Ok(SkuEntriesQuery {
        sku,
        book_ids,
        currency,
        q,
        statuses,
        changing,
        limit: clamp_limit(limit),
        order,
        cursor,
        hash,
    })
}

fn book_ids(raw: &str) -> Result<BTreeSet<Uuid>, CanonicalError> {
    let mut ids = BTreeSet::new();
    if raw.is_empty() {
        return Err(support::invalid_because(
            "book_id",
            "QUERY_INVALID",
            "`book_id` is one to 50 price book ids",
        ));
    }
    for token in raw.split(',') {
        let token = token.trim();
        if token.is_empty() {
            return Err(support::invalid_because(
                "book_id",
                "QUERY_INVALID",
                "`book_id` is one to 50 price book ids",
            ));
        }
        ids.insert(Uuid::parse_str(token).map_err(|_| {
            support::invalid_because("book_id", "QUERY_INVALID", "`book_id` is a price book id")
        })?);
    }
    if ids.len() > 50 {
        return Err(support::invalid_because(
            "book_id",
            "QUERY_INVALID",
            "`book_id` lists at most 50 distinct price book ids",
        ));
    }
    Ok(ids)
}

fn currency_key(raw: &str) -> Result<String, CanonicalError> {
    if crate::domain::book::currency_code(raw) {
        Ok(raw.to_owned())
    } else {
        Err(support::invalid_because(
            "currency",
            "QUERY_INVALID",
            "`currency` is three uppercase letters",
        ))
    }
}

fn status_keys(
    raw: &str,
) -> Result<Vec<crate::api::rest::closed_sets::PricingSkuEntryStatus>, CanonicalError> {
    use crate::api::rest::closed_sets::PricingSkuEntryStatus;
    if raw.is_empty() {
        return Err(support::invalid_because(
            "status",
            "QUERY_INVALID",
            "`status` is priced, scheduled or unpriced",
        ));
    }
    raw.split(',')
        .map(|token| {
            let token = token.trim();
            PricingSkuEntryStatus::ALL
                .iter()
                .copied()
                .find(|status| status.as_str() == token)
                .ok_or_else(|| {
                    support::invalid_because(
                        "status",
                        "QUERY_INVALID",
                        "`status` is priced, scheduled or unpriced",
                    )
                })
        })
        .collect()
}

fn changing_key(raw: &str) -> Result<bool, CanonicalError> {
    match raw {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(support::invalid_because(
            "changing",
            "QUERY_INVALID",
            "`changing` is true or false",
        )),
    }
}

fn clamp_limit(limit: Option<u64>) -> u64 {
    let mut limit = limit.unwrap_or(PAGE_DEFAULT);
    if limit == 0 {
        limit = 1;
    }
    limit.min(PAGE_DEFAULT)
}

fn requested_order(raw: Option<&str>) -> Result<ODataOrderBy, CanonicalError> {
    let Some(raw) = raw.filter(|text| !text.trim().is_empty()) else {
        return Ok(order_of("book_name", SortDir::Asc));
    };
    let parsed = toolkit::api::odata::parse_orderby(raw).map_err(CanonicalError::from)?;
    match parsed.0.as_slice() {
        [] => Ok(order_of("book_name", SortDir::Asc)),
        [key] if SkuEntryOrderField::from_name(&key.field).is_some() => {
            Ok(order_of(&key.field, key.dir))
        }
        keys => Err(toolkit_odata::Error::InvalidOrderByField(
            keys.iter()
                .find(|key| SkuEntryOrderField::from_name(&key.field).is_none())
                .map_or_else(
                    || "only one key, book_name or status, is accepted".to_owned(),
                    |key| key.field.clone(),
                ),
        )
        .into()),
    }
}

fn order_of(field: &str, dir: SortDir) -> ODataOrderBy {
    ODataOrderBy(vec![
        OrderKey {
            field: field.to_owned(),
            dir,
        },
        OrderKey {
            field: "id".to_owned(),
            dir,
        },
    ])
}

fn cursor_order(cursor: &CursorV1) -> Result<ODataOrderBy, CanonicalError> {
    let order = ODataOrderBy::from_signed_tokens(&cursor.s)
        .map_err(|_| CanonicalError::from(toolkit_odata::Error::InvalidCursor))?;
    let fields_ok = matches!(
        order.0.as_slice(),
        [primary, id]
            if SkuEntryOrderField::from_name(&primary.field).is_some()
                && id.field == "id"
                && cursor.k.len() == 2
    );
    if fields_ok {
        Ok(order)
    } else {
        Err(toolkit_odata::Error::InvalidCursor.into())
    }
}

fn narrowing_hash(
    sku: Uuid,
    book_ids: Option<&BTreeSet<Uuid>>,
    currency: Option<&str>,
    q: Option<&str>,
    statuses: Option<&[crate::api::rest::closed_sets::PricingSkuEntryStatus]>,
    changing: Option<bool>,
) -> Result<String, CanonicalError> {
    let book_id = book_ids.map(|ids| {
        let mut listed: Vec<String> = ids.iter().map(ToString::to_string).collect();
        listed.sort();
        listed
    });
    let status = statuses.map(|wanted| {
        let mut listed: Vec<&str> = wanted.iter().map(|status| status.as_str()).collect();
        listed.sort_unstable();
        listed.dedup();
        listed
    });
    let digest = crate::api::rest::preconditions::request_digest(&serde_json::json!({
        "book_id": book_id,
        "changing": changing,
        "currency": currency,
        "q": q,
        "sku_id": sku,
        "status": status,
    }))
    .map_err(CanonicalError::from)?;
    Ok(digest
        .iter()
        .take(8)
        .fold(String::with_capacity(16), |mut hex, byte| {
            const DIGITS: &[u8; 16] = b"0123456789abcdef";
            hex.push(char::from(DIGITS[usize::from(byte >> 4)]));
            hex.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
            hex
        }))
}

fn standing(
    counts: &super::dto::PricingEntryPriceCounts,
) -> (crate::api::rest::closed_sets::PricingSkuEntryStatus, bool) {
    use crate::api::rest::closed_sets::PricingSkuEntryStatus;
    let status = if counts.active > 0 {
        PricingSkuEntryStatus::Priced
    } else if counts.scheduled > 0 {
        PricingSkuEntryStatus::Scheduled
    } else {
        PricingSkuEntryStatus::Unpriced
    };
    (status, counts.draft > 0 || counts.pending > 0)
}

fn page_entries(
    mut items: Vec<super::dto::PricingSkuEntryDto>,
    query: &SkuEntriesQuery,
) -> Result<super::dto::PricingSkuEntryList, DoorError> {
    items.retain(|item| keeps(item, query));
    items.sort_by(|left, right| cmp_entries(left, right, &query.order));
    let backward = query
        .cursor
        .as_ref()
        .is_some_and(|cursor| cursor.d == "bwd");
    if let Some(cursor) = &query.cursor {
        items.retain(|item| {
            let at = position(item, cursor, &query.order);
            if backward {
                at == Ordering::Less
            } else {
                at == Ordering::Greater
            }
        });
    }
    let limit = query.limit;
    let has_more = u64::try_from(items.len()).unwrap_or(u64::MAX) > limit;
    let keep = usize::try_from(limit).unwrap_or(usize::MAX);
    if has_more {
        if backward {
            items = items.split_off(items.len().saturating_sub(keep));
        } else {
            items.truncate(keep);
        }
    }
    let next_cursor = if backward || has_more {
        items
            .last()
            .map(|item| cursor_token(item, &query.order, &query.hash, "fwd"))
            .transpose()?
    } else {
        None
    };
    let prev_cursor = if query.cursor.is_some() && (!backward || has_more) {
        items
            .first()
            .map(|item| cursor_token(item, &query.order, &query.hash, "bwd"))
            .transpose()?
    } else {
        None
    };
    Ok(super::dto::PricingSkuEntryList {
        items,
        page_info: PageInfo {
            next_cursor,
            prev_cursor,
            limit,
        },
    })
}

fn keeps(item: &super::dto::PricingSkuEntryDto, query: &SkuEntriesQuery) -> bool {
    if query
        .book_ids
        .as_ref()
        .is_some_and(|ids| !ids.contains(&item.entry.book_id))
    {
        return false;
    }
    if query
        .currency
        .as_ref()
        .is_some_and(|currency| item.currency != *currency)
    {
        return false;
    }
    if let Some(text) = query.q.as_deref() {
        let needle = fold(text);
        if !fold(&item.book_code).contains(&needle) && !fold(&item.book_name).contains(&needle) {
            return false;
        }
    }
    if query
        .statuses
        .as_ref()
        .is_some_and(|wanted| !wanted.contains(&item.status))
    {
        return false;
    }
    query
        .changing
        .is_none_or(|changing| item.changing == changing)
}

fn fold(text: &str) -> String {
    text.to_lowercase()
}

fn cmp_entries(
    left: &super::dto::PricingSkuEntryDto,
    right: &super::dto::PricingSkuEntryDto,
    order: &ODataOrderBy,
) -> Ordering {
    for key in &order.0 {
        let cmp = encode_key(left, &key.field).cmp(&encode_key(right, &key.field));
        let cmp = match key.dir {
            SortDir::Asc => cmp,
            SortDir::Desc => cmp.reverse(),
        };
        if cmp != Ordering::Equal {
            return cmp;
        }
    }
    Ordering::Equal
}

fn position(
    item: &super::dto::PricingSkuEntryDto,
    cursor: &CursorV1,
    order: &ODataOrderBy,
) -> Ordering {
    for (index, key) in order.0.iter().enumerate() {
        let mine = encode_key(item, &key.field);
        let theirs = cursor.k.get(index).map_or("", String::as_str);
        let cmp = mine.as_str().cmp(theirs);
        if cmp == Ordering::Equal {
            continue;
        }
        return match key.dir {
            SortDir::Asc => cmp,
            SortDir::Desc => cmp.reverse(),
        };
    }
    Ordering::Equal
}

fn encode_key(item: &super::dto::PricingSkuEntryDto, field: &str) -> String {
    match field {
        "book_name" => item.book_name.clone(),
        "status" => item.status.as_str().to_owned(),
        _ => item.entry.id.to_string(),
    }
}

fn cursor_token(
    item: &super::dto::PricingSkuEntryDto,
    order: &ODataOrderBy,
    hash: &str,
    direction: &str,
) -> Result<String, CanonicalError> {
    CursorV1 {
        k: order
            .0
            .iter()
            .map(|key| encode_key(item, &key.field))
            .collect(),
        o: order.0.first().map_or(SortDir::Asc, |key| key.dir),
        s: order.to_signed_tokens(),
        f: Some(hash.to_owned()),
        d: direction.to_owned(),
    }
    .encode()
    .map_err(|_| CanonicalError::internal("the cursor does not encode").create())
}
