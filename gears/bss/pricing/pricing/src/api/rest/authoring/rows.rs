//! Draft rows: a single row, a temporary pair or one explicitly closed row; draft-only edits.
//!
//! @cpt-dod:cpt-cf-bss-pricing-dod-temporary-pair:p1
//! @cpt-dod:cpt-cf-bss-pricing-dod-row-pending-guard:p1
use super::{
    dto::{
        PricingPriceRowCreate, PricingPriceRowCreated, PricingPriceRowDto, PricingPriceRowPatch,
    },
    support::{self, DoorError},
};
use crate::{
    domain::{
        RuleError, money,
        price::Model,
        row::{self, Eligibility, Row, RowState},
    },
    infra::{
        price_rows::PriceContext,
        storage::{
            RepoError,
            entity::{price, price_row},
            repo::{price_repo, row_repo},
        },
    },
};
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use rust_decimal::Decimal;
use std::str::FromStr;
use time::OffsetDateTime;
use toolkit_canonical_errors::CanonicalError;
use toolkit_db::secure::{AccessScope, DBRunner};
use toolkit_security::SecurityContext;
use uuid::Uuid;

/// Attempts of the create transaction when a concurrent writer took the next `version_no`.
const VERSION_ATTEMPTS: u32 = 3;

fn refuse(error: RuleError) -> DoorError {
    support::invalid(row::field_of(error.code), error.code).into()
}
/// A price body refusal; a decimal sent as a JSON number is told to send a string.
fn refuse_price(error: RuleError) -> DoorError {
    if error.code == "AMOUNT_INVALID" {
        support::invalid_because("price", error.code, money::DECIMALS_ARE_STRINGS).into()
    } else {
        refuse(error)
    }
}

/// A draft belongs to its author (D-404): only its creator edits or deletes it, so every
/// number in a unit is its item author's and separation of duties excludes the right person.
fn own_draft(m: &price_row::Model, ctx: &SecurityContext) -> Result<(), DoorError> {
    if m.created_by == ctx.subject_id() {
        Ok(())
    } else {
        Err(support::forbidden("NOT_DRAFT_AUTHOR").into())
    }
}
fn parse_model(text: &str) -> Result<Model, DoorError> {
    text.parse()
        .map_err(|_| support::invalid("model", "MODEL_INVALID").into())
}
fn parse_eligibility(text: &str) -> Result<Eligibility, DoorError> {
    text.parse()
        .map_err(|_| support::invalid("eligibility", "ELIGIBILITY_INVALID").into())
}
fn parse_min_fee(text: Option<&str>) -> Result<Option<Decimal>, DoorError> {
    text.map(|s| {
        Decimal::from_str(s.trim())
            .map_err(|_| support::invalid("min_fee", "MIN_FEE_INVALID").into())
    })
    .transpose()
}
fn price_json(r: &Row) -> Result<serde_json::Value, DoorError> {
    Ok(support::value(&r.price)?)
}
fn next_version(rows: &[price_row::Model]) -> Result<i32, DoorError> {
    rows.iter()
        .map(|r| r.version_no)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| support::conflict("VERSION_EXHAUSTED").into())
}
fn stored(
    tenant: Uuid,
    r: &Row,
    note: Option<String>,
    author: Uuid,
    now: OffsetDateTime,
) -> Result<price_row::Model, DoorError> {
    Ok(price_row::Model {
        id: r.id,
        tenant_id: tenant,
        price_id: r.price_id,
        version_no: r.version_no,
        dim_value: r.dim_value.clone(),
        model: r.model.as_str().into(),
        price_json: price_json(r)?,
        min_fee: r.min_fee.map(|fee| fee.to_string()),
        eligibility: r.eligibility.as_str().into(),
        effective_from: r.effective_from,
        effective_to: r.effective_to,
        keep_for_bound: false,
        closed_explicitly: r.closed_explicitly,
        temporary_until: r.temporary_until,
        paired_row_id: None,
        return_of_row_id: r.return_of_row_id,
        state: RowState::Draft.as_str().into(),
        pending_unit_id: None,
        approved_by_unit_id: None,
        note,
        created_by: author,
        approved_at: None,
        version: 1,
        created_at: now,
        updated_at: now,
    })
}
async fn live_price(
    tx: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
) -> Result<price::Model, DoorError> {
    let price = price_repo::find(tx, scope, tenant, id)
        .await?
        .ok_or_else(|| support::missing_what("price"))?;
    if price.reference_state == "lost" {
        return Err(support::conflict("PRICE_REFERENCE_LOST").into());
    }
    Ok(price)
}

/// Create under a bounded retry: the `(price_id, version_no)` unique index arbitrates
/// two writers that read the same maximum; the loser recomputes in a fresh transaction.
/// # Errors
/// Returns the canonical refusal of the last attempt.
#[allow(
    clippy::too_many_arguments,
    reason = "authorized door identity, replay operands and input belong to one transaction"
)]
pub async fn create(
    db: &toolkit_db::Db,
    scope: AccessScope,
    ctx: SecurityContext,
    correlation: Uuid,
    price_id: Uuid,
    key: String,
    digest: Vec<u8>,
    input: PricingPriceRowCreate,
) -> Result<Response, CanonicalError> {
    let mut attempt = 1;
    loop {
        let (scope, ctx, key, digest, input) = (
            scope.clone(),
            ctx.clone(),
            key.clone(),
            digest.clone(),
            input.clone(),
        );
        let result = support::transaction_door(db, move |tx| {
            let (scope, ctx, key, digest, input) = (
                scope.clone(),
                ctx.clone(),
                key.clone(),
                digest.clone(),
                input.clone(),
            );
            Box::pin(async move {
                create_in(
                    tx,
                    &scope,
                    &ctx,
                    correlation,
                    price_id,
                    &key,
                    &digest,
                    input,
                )
                .await
            })
        })
        .await;
        match result {
            Err(DoorError::Repo(RepoError::Conflict {
                code: "ROW_VERSION_TAKEN",
            })) if attempt < VERSION_ATTEMPTS => attempt += 1,
            other => return other.map_err(Into::into),
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "authorized door identity, replay operands and input belong to one transaction"
)]
async fn create_in(
    tx: &impl DBRunner,
    scope: &AccessScope,
    ctx: &SecurityContext,
    correlation: Uuid,
    price_id: Uuid,
    key: &str,
    digest: &[u8],
    input: PricingPriceRowCreate,
) -> Result<Response, DoorError> {
    let tenant = ctx.subject_tenant_id();
    let endpoint = format!("/bss-pricing/v1/prices/{price_id}/rows");
    if let Some(replay) = support::claim(tx, tenant, &endpoint, key, digest).await? {
        return Ok(replay);
    }
    let pc =
        PriceContext::load(tx, tenant, &live_price(tx, scope, tenant, price_id).await?).await?;
    let now = OffsetDateTime::now_utc();
    let model = parse_model(&input.model)?;
    let promo = Row {
        id: Uuid::now_v7(),
        price_id,
        version_no: next_version(&pc.rows)?,
        dim_value: input.dim_value,
        model,
        price: Some(money::decode(model, input.price).map_err(refuse_price)?),
        min_fee: parse_min_fee(input.min_fee.as_deref())?,
        eligibility: parse_eligibility(&input.eligibility)?,
        effective_from: row::parse_start(&input.effective_from).map_err(refuse)?,
        effective_to: None,
        temporary_until: None,
        paired_row_id: None,
        return_of_row_id: None,
        closed_explicitly: false,
        state: RowState::Draft,
    };
    let siblings = pc.domain_rows()?;
    let rows = match input.temporary_until.as_deref() {
        Some(until) => {
            let end = row::validate_temporary(promo.effective_from, until).map_err(refuse)?;
            row::temporary(&siblings, promo, end, Uuid::now_v7()).map_err(refuse)?
        }
        None => vec![promo],
    };
    for r in &rows {
        if let Some(error) = pc.first_refusal(r, &siblings, now.date()) {
            return Err(refuse(error));
        }
    }
    let children = AccessScope::for_tenant(tenant);
    let mut items = Vec::with_capacity(rows.len());
    for r in &rows {
        let mut m = stored(tenant, r, input.note.clone(), ctx.subject_id(), now)?;
        // The pair's first half cannot name a partner that does not exist yet.
        if items.len() == 1 {
            m.paired_row_id = r.paired_row_id;
        }
        let m = row_repo::insert(tx, &children, m).await?;
        support::audit(tx, ctx, correlation, "price_row.create", m.id, m.version).await?;
        items.push(m);
    }
    if let [first, second] = items.as_mut_slice() {
        row_repo::link_pair(tx, &children, tenant, first.id, second.id).await?;
        first.paired_row_id = Some(second.id);
    }
    let body = PricingPriceRowCreated {
        items: items.into_iter().map(PricingPriceRowDto::from).collect(),
    };
    support::answer(
        tx,
        tenant,
        &endpoint,
        key,
        StatusCode::CREATED,
        &body,
        Some(1),
    )
    .await
}

/// Change business fields of an unlocked draft at the version the caller read.
/// A temporary row keeps its start and value: its window belongs to its pair or closure.
/// # Errors
/// Returns `ROW_NOT_DRAFT`, `NOT_DRAFT_AUTHOR`, `VERSION_CONFLICT`, `TEMPORARY_ROW_FIXED` or a
/// pure-rule refusal.
#[allow(
    clippy::too_many_arguments,
    reason = "Conditional resource identity and audit context are explicit"
)]
pub async fn patch(
    tx: &impl DBRunner,
    scope: &AccessScope,
    ctx: &SecurityContext,
    correlation: Uuid,
    id: Uuid,
    version: u64,
    input: PricingPriceRowPatch,
) -> Result<Response, DoorError> {
    let tenant = ctx.subject_tenant_id();
    let m = row_repo::find(tx, scope, tenant, id)
        .await?
        .ok_or_else(|| support::missing_what("price_row"))?;
    if m.state != RowState::Draft.as_str() || m.pending_unit_id.is_some() {
        return Err(support::conflict("ROW_NOT_DRAFT").into());
    }
    own_draft(&m, ctx)?;
    support::check_version(version, m.version)?;
    let children = AccessScope::for_tenant(tenant);
    let pc = PriceContext::load(
        tx,
        tenant,
        &live_price(tx, &children, tenant, m.price_id).await?,
    )
    .await?;
    let mut r = row_repo::to_domain(&m)?;
    let temporary = m.temporary_until.is_some() || m.paired_row_id.is_some();
    let fixed =
        || -> DoorError { support::invalid("effective_from", "TEMPORARY_ROW_FIXED").into() };
    if let Some(value) = input.dim_value {
        if temporary && value != r.dim_value {
            return Err(fixed());
        }
        r.dim_value = value;
    }
    if let Some(start) = input.effective_from.as_deref() {
        let start = row::parse_start(start).map_err(refuse)?;
        if temporary && start != r.effective_from {
            return Err(fixed());
        }
        r.effective_from = start;
    }
    if let Some(model) = input.model.as_deref() {
        r.model = parse_model(model)?;
    }
    if let Some(data) = input.price {
        r.price = Some(money::decode(r.model, data).map_err(refuse_price)?);
    }
    if let Some(fee) = input.min_fee {
        r.min_fee = parse_min_fee(fee.as_deref())?;
    }
    if let Some(eligibility) = input.eligibility.as_deref() {
        r.eligibility = parse_eligibility(eligibility)?;
    }
    let now = OffsetDateTime::now_utc();
    if let Some(error) = pc.first_refusal(&r, &pc.domain_rows()?, now.date()) {
        return Err(refuse(error));
    }
    let mut next = m.clone();
    next.dim_value = r.dim_value.clone();
    next.model = r.model.as_str().into();
    next.price_json = price_json(&r)?;
    next.min_fee = r.min_fee.map(|fee| fee.to_string());
    next.eligibility = r.eligibility.as_str().into();
    next.effective_from = r.effective_from;
    if let Some(note) = input.note {
        next.note = note;
    }
    next.updated_at = now;
    row_repo::update_draft(tx, &children, next.clone()).await?;
    next.version += 1;
    support::audit(tx, ctx, correlation, "price_row.patch", id, next.version).await?;
    Ok(support::response(
        StatusCode::OK,
        &PricingPriceRowDto::from(next),
        Some(version + 1),
    )?)
}

/// Delete an unlocked draft at its version; a pair half takes its partner with it.
/// # Errors
/// Returns `ROW_NOT_DRAFT`, `NOT_DRAFT_AUTHOR` or `VERSION_CONFLICT`.
pub async fn delete(
    tx: &impl DBRunner,
    scope: &AccessScope,
    ctx: &SecurityContext,
    correlation: Uuid,
    id: Uuid,
    version: u64,
) -> Result<Response, DoorError> {
    let tenant = ctx.subject_tenant_id();
    let m = row_repo::find(tx, scope, tenant, id)
        .await?
        .ok_or_else(|| support::missing_what("price_row"))?;
    let draft =
        |r: &price_row::Model| r.state == RowState::Draft.as_str() && r.pending_unit_id.is_none();
    if !draft(&m) {
        return Err(support::conflict("ROW_NOT_DRAFT").into());
    }
    own_draft(&m, ctx)?;
    support::check_version(version, m.version)?;
    let children = AccessScope::for_tenant(tenant);
    let mut targets = vec![(m.id, m.version)];
    if let Some(partner) = m.paired_row_id
        && let Some(p) = row_repo::find(tx, &children, tenant, partner).await?
    {
        if !draft(&p) {
            return Err(support::conflict("ROW_NOT_DRAFT").into());
        }
        own_draft(&p, ctx)?;
        targets.push((p.id, p.version));
    }
    row_repo::delete_drafts(tx, &children, tenant, &targets).await?;
    for (row, version) in targets {
        support::audit(tx, ctx, correlation, "price_row.delete", row, version).await?;
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}
