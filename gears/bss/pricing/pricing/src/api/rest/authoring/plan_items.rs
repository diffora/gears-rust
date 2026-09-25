//! The plan item's doors and its operations on the durable reference machine (D-404, D-407,
//! D-413, D-414).
//!
//! `POST /plan-revisions/{id}/items` ([`add`]) judges its input, the revision (an unlocked draft
//! of the caller), the entry, the revision's items and the SKU read fresh, all before any claim or
//! reservation (a refusal is 400 and costs nothing, D-403); then [`create`] writes the create op
//! and drives it. `PATCH /plan-items/{id}` (`patch`) edits an item of an unlocked draft of the
//! caller, never its SKU. `DELETE /plan-items/{id}` ([`delete`]) removes it with its delete op;
//! a draft revision's delete removes every item through `remove`. A copy or a clone writes each
//! copied item `unreserved` with its attach op ([`reference_work::attach_op`]) in its own
//! transaction and drives them with [`drive_best_effort`].
use super::{
    AuthoringState,
    dto::{PricingPlanItemCreate, PricingPlanItemDto, PricingPlanItemPatch},
    plans,
    price_book_entries::{settled, stored},
    support::{self, DoorError},
};
use crate::{
    domain::{
        plan::{MAX_ITEMS, ReferenceState, RevisionState, Treatment},
        reference_op::{OpKind, RefKind},
    },
    infra::{
        reference_registry,
        reference_work::{self, Caller, Receipt, Ref, Target, WallClock, Work},
        storage::{
            entity::plan_revision,
            repo::{
                idempotency_repo as idem, plan_item_repo, plan_revision_repo,
                price_book_entry_repo, reference_op_repo,
            },
        },
    },
};
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use bss_products_sdk::models::Lifecycle;
use std::sync::Arc;
use toolkit_canonical_errors::CanonicalError;
use toolkit_db::secure::{AccessScope, DBRunner};
use toolkit_security::SecurityContext;
use uuid::Uuid;
/// The item door's input rules, judged before any claim or reservation (D-403: 400).
fn treatment(text: &str) -> Result<Treatment, DoorError> {
    text.parse()
        .map_err(|_| support::invalid("treatment", "TREATMENT_INVALID").into())
}
/// Canonical unsigned decimal text, the column's own CHECK: digits, then an optional fraction.
fn included_qty(text: Option<&str>) -> Result<(), DoorError> {
    let canonical = |q: &str| {
        let (whole, fraction) = q.split_once('.').map_or((q, None), |(w, f)| (w, Some(f)));
        !whole.is_empty()
            && whole.bytes().all(|b| b.is_ascii_digit())
            && fraction.is_none_or(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit()))
    };
    if text.is_some_and(|q| !canonical(q)) {
        return Err(support::invalid("included_qty", "INCLUDED_QTY_INVALID").into());
    }
    Ok(())
}
fn qty_min(value: Option<i32>) -> Result<(), DoorError> {
    if value.is_some_and(|q| q < 0) {
        return Err(support::invalid("qty_min", "QTY_MIN_INVALID").into());
    }
    Ok(())
}
/// A paid or optional item points at a price; only an included one may name none.
fn entry_needed(treatment: Treatment, entry: Option<Uuid>) -> Result<(), DoorError> {
    if treatment != Treatment::Included && entry.is_none() {
        return Err(support::invalid("price_book_entry_id", "ITEM_ENTRY_MISSING").into());
    }
    Ok(())
}
/// The entry must exist, belong to the revision's book and price the item's SKU.
async fn entry_fits(
    tx: &impl DBRunner,
    scope: &AccessScope,
    revision: &plan_revision::Model,
    sku: Uuid,
    entry: Uuid,
) -> Result<(), DoorError> {
    let e = price_book_entry_repo::find(tx, scope, revision.tenant_id, entry)
        .await?
        .ok_or_else(support::missing_entry)?;
    if e.book_id != revision.book_id {
        return Err(support::invalid("price_book_entry_id", "ITEM_BOOK_FOREIGN").into());
    }
    if e.sku_id != sku {
        return Err(support::invalid("price_book_entry_id", "ITEM_ENTRY_SKU_MISMATCH").into());
    }
    Ok(())
}

/// `POST /plan-revisions/{id}/items` below its door: a replay answers from the key's store; then
/// every refusal the door owns is judged before anything is claimed or reserved, and only then
/// does [`create`] write its op and drive it (D-401, D-407).
/// # Errors
/// 400 `TREATMENT_INVALID`, `INCLUDED_QTY_INVALID`, `QTY_MIN_INVALID`, `ITEM_ENTRY_MISSING`,
/// `ITEM_BOOK_FOREIGN`, `ITEM_ENTRY_SKU_MISMATCH`, `REVISION_ITEMS_TOO_MANY`,
/// `ITEM_SKU_DEPRECATED` or `ITEM_BUNDLE_SKU`; 404 for an unknown revision or entry; 409
/// `REVISION_NOT_DRAFT` or `ITEM_SKU_TAKEN`; 403 `NOT_DRAFT_AUTHOR` (D-404); 503 when Products
/// cannot answer; then [`create`]'s own.
#[allow(
    clippy::too_many_arguments,
    reason = "authorized door identity and replay operands"
)]
pub async fn add(
    state: Arc<AuthoringState>,
    scope: AccessScope,
    ctx: SecurityContext,
    revision: Uuid,
    correlation: Uuid,
    key: String,
    digest: Vec<u8>,
    input: PricingPlanItemCreate,
) -> Result<Response, CanonicalError> {
    let endpoint = format!("/bss-pricing/v1/plan-revisions/{revision}/items");
    if let Some(receipt) = stored(&state, ctx.subject_tenant_id(), &endpoint, &key, &digest).await?
    {
        return receipt.response();
    }
    let kind = treatment(&input.treatment)?;
    included_qty(input.included_qty.as_deref())?;
    qty_min(input.qty_min)?;
    entry_needed(kind, input.price_book_entry_id)?;
    let (judged_scope, judged_ctx, judged) = (scope.clone(), ctx.clone(), input.clone());
    support::transaction(&state.db.db(), move |tx| {
        let (scope, ctx, input) = (judged_scope.clone(), judged_ctx.clone(), judged.clone());
        Box::pin(async move { admissible(tx, &scope, &ctx, revision, &input).await })
    })
    .await?;
    fresh_sku(&state, &ctx, input.sku_id).await?;
    create(state, scope, ctx, revision, correlation, key, digest, input).await
}
/// The revision, the entry and the revision's items, judged in one read.
async fn admissible(
    tx: &impl DBRunner,
    scope: &AccessScope,
    ctx: &SecurityContext,
    revision: Uuid,
    input: &PricingPlanItemCreate,
) -> Result<(), DoorError> {
    let tenant = ctx.subject_tenant_id();
    let children = AccessScope::for_tenant(tenant);
    let r = plans::find_revision(tx, scope, tenant, revision).await?;
    plans::editable(&r, ctx)?;
    if let Some(entry) = input.price_book_entry_id {
        entry_fits(tx, &children, &r, input.sku_id, entry).await?;
    }
    let items = plan_item_repo::for_revision(tx, &children, tenant, revision).await?;
    if items.iter().any(|i| i.sku_id == input.sku_id) {
        return Err(support::conflict("ITEM_SKU_TAKEN").into());
    }
    if items.len() >= MAX_ITEMS {
        return Err(support::invalid("items", "REVISION_ITEMS_TOO_MANY").into());
    }
    Ok(())
}
/// The SKU read fresh (D-408): a deprecated SKU cannot be added, and a bundle SKU is never an
/// item. A registry that cannot answer is 503 with nothing written; a definite Products refusal
/// is answered as Products gave it.
async fn fresh_sku(
    state: &AuthoringState,
    ctx: &SecurityContext,
    sku: Uuid,
) -> Result<(), CanonicalError> {
    let registry = reference_registry::resolve(&state.hub).map_err(|_| support::unavailable())?;
    let sku = registry
        .sku_for_write(ctx, ctx.subject_tenant_id(), sku)
        .await
        .map_err(|error| {
            if reference_work::definite_refusal(&error) {
                error
            } else {
                support::unavailable()
            }
        })?;
    if sku.lifecycle == Lifecycle::Deprecated {
        return Err(support::invalid("sku_id", "ITEM_SKU_DEPRECATED"));
    }
    if let Some(code) = reference_work::plan_item::type_refusal(sku.r#type) {
        return Err(support::invalid("sku_id", code));
    }
    Ok(())
}
/// `PATCH /plan-items/{id}` below its door: treatment, quantities or entry of an item of an
/// unlocked draft of the caller, at the version the caller read; the SKU never changes.
/// # Errors
/// 404; 409 `REVISION_NOT_DRAFT`; 403 `NOT_DRAFT_AUTHOR`; 409 `STALE_REVISION`; the input and
/// entry refusals of [`add`].
pub(super) async fn patch(
    tx: &impl DBRunner,
    scope: &AccessScope,
    ctx: &SecurityContext,
    correlation: Uuid,
    id: Uuid,
    version: u64,
    input: PricingPlanItemPatch,
) -> Result<Response, DoorError> {
    let tenant = ctx.subject_tenant_id();
    let children = AccessScope::for_tenant(tenant);
    let mut m = plan_item_repo::find(tx, scope, tenant, id)
        .await?
        .ok_or_else(|| support::missing_what("plan_item"))?;
    let r = plans::find_revision(tx, &children, tenant, m.revision_id).await?;
    plans::editable(&r, ctx)?;
    support::check_version(version, m.version)?;
    if let Some(text) = input.treatment {
        treatment(&text)?;
        m.treatment = text;
    }
    if let Some(qty) = input.included_qty {
        included_qty(qty.as_deref())?;
        m.included_qty = qty;
    }
    if let Some(min) = input.qty_min {
        qty_min(min)?;
        m.qty_min = min;
    }
    if let Some(entry) = input.price_book_entry_id {
        if let Some(entry) = entry {
            entry_fits(tx, &children, &r, m.sku_id, entry).await?;
        }
        m.price_book_entry_id = entry;
    }
    entry_needed(treatment(&m.treatment)?, m.price_book_entry_id)?;
    m.updated_at = time::OffsetDateTime::now_utc();
    plan_item_repo::update_draft(tx, &children, m.clone()).await?;
    m.version += 1;
    support::audit(tx, ctx, correlation, "plan_item.patch", id, m.version).await?;
    Ok(support::response(
        StatusCode::OK,
        &PricingPlanItemDto::from(m),
        Some(version + 1),
    )?)
}
enum Begun {
    Replay(Receipt),
    Op(Uuid),
}
/// Tx A of an item create, then the drive (D-401): claim the key, mint the item id and write the
/// create op before any reserve; the drive reserves, re-reads the SKU, writes the item (Tx B,
/// which re-reads the revision and the entry) and confirms. A 503 writes nothing.
///
/// A replay or an in-flight duplicate is answered from the key's store alone. Tx A refuses a
/// revision that is missing (404) or no longer an unlocked draft (409 `REVISION_NOT_DRAFT`).
/// # Errors
/// The canonical refusal, conflict or unavailability of any step.
#[allow(
    clippy::too_many_arguments,
    reason = "authorized door identity and replay operands"
)]
pub async fn create(
    state: Arc<AuthoringState>,
    scope: AccessScope,
    ctx: SecurityContext,
    revision: Uuid,
    correlation: Uuid,
    key: String,
    digest: Vec<u8>,
    input: PricingPlanItemCreate,
) -> Result<Response, CanonicalError> {
    let original_ctx = ctx.clone();
    let endpoint = format!("/bss-pricing/v1/plan-revisions/{revision}/items");
    if let Some(receipt) = stored(&state, ctx.subject_tenant_id(), &endpoint, &key, &digest).await?
    {
        return receipt.response();
    }
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
            let now = time::OffsetDateTime::now_utc();
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
            let Some(parent) = plan_revision_repo::find(tx, &scope, tenant, revision).await? else {
                return Err(support::missing_what("plan_revision").into());
            };
            if parent.state != RevisionState::Draft.as_str() || parent.pending_unit_id.is_some() {
                return Err(support::conflict("REVISION_NOT_DRAFT").into());
            }
            let reference = Ref {
                kind: RefKind::PlanItem,
                id: Uuid::now_v7(),
                sku_id: input.sku_id,
            };
            let work = Work {
                target: Target::PlanItem {
                    revision_id: revision,
                    input: Some(input),
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
/// Remove one item of an unlocked draft revision of the caller and write its delete op, in the
/// caller's transaction; the caller drives the op once it commits. An item whose confirm is still
/// pending is kept until it completes (`ITEM_CONFIRMATION_PENDING`), or its confirm could lose
/// it; an item of a pending, published or superseded revision is never removed
/// (`REVISION_NOT_DRAFT`): its reference outlives the revision (D-414); an item of another
/// author's draft is not the caller's to remove (`NOT_DRAFT_AUTHOR`, D-404). A draft revision's
/// delete removes every item through this, before the revision itself.
/// # Errors
/// 404 for an unknown item, the refusals above, `STALE_REVISION` for a lost race.
pub(crate) async fn remove(
    tx: &impl DBRunner,
    scope: &AccessScope,
    ctx: &SecurityContext,
    correlation: Uuid,
    id: Uuid,
) -> Result<Uuid, DoorError> {
    let tenant = ctx.subject_tenant_id();
    let item = plan_item_repo::find(tx, scope, tenant, id)
        .await?
        .ok_or_else(|| support::missing_what("plan_item"))?;
    let revision = plans::find_revision(tx, scope, tenant, item.revision_id).await?;
    plans::editable(&revision, ctx)?;
    if item.reference_state == ReferenceState::ConfirmationPending.as_str() {
        return Err(support::conflict("ITEM_CONFIRMATION_PENDING").into());
    }
    let now = time::OffsetDateTime::now_utc();
    let op = reference_work::plan_item::delete_op(ctx, &item, correlation, now)?;
    let op_id = op.op_id;
    plan_item_repo::delete_draft(tx, scope, tenant, id, item.version).await?;
    reference_op_repo::insert(tx, &AccessScope::for_tenant(tenant), op).await?;
    support::audit(tx, ctx, correlation, "plan_item.delete", id, item.version).await?;
    Ok(op_id)
}
/// `DELETE /plan-items/{id}` below its door: the removal and its delete op commit together
/// (204); the release is durable work, and what this call does not finish, the ticker does.
/// # Errors
/// The removal's refusals; a failed release is never an error of the delete.
pub async fn delete(
    state: Arc<AuthoringState>,
    scope: AccessScope,
    ctx: SecurityContext,
    correlation: Uuid,
    id: Uuid,
) -> Result<Response, CanonicalError> {
    let original_ctx = ctx.clone();
    let op_id = support::transaction(&state.db.db(), move |tx| {
        let (scope, ctx) = (scope.clone(), ctx.clone());
        Box::pin(async move { remove(tx, &scope, &ctx, correlation, id).await })
    })
    .await?;
    drive_best_effort(&state, &original_ctx, &[op_id]).await;
    Ok(StatusCode::NO_CONTENT.into_response())
}
/// Drive committed ops once each under the caller; an op this does not finish stays durable and
/// the ticker finishes it after the in-flight grace.
pub async fn drive_best_effort(state: &Arc<AuthoringState>, ctx: &SecurityContext, ops: &[Uuid]) {
    for op_id in ops {
        if let Err(error) =
            reference_work::drive(state, ctx, *op_id, Arc::new(WallClock), Caller::Door).await
        {
            tracing::warn!(op_id=%op_id, error=%error, "pricing plan item reference work deferred to the ticker");
        }
    }
}
