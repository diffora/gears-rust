//! Plans and their revisions below their doors (D-404, D-407, D-413, D-414): a plan with its
//! draft rev 1, the rename, the copy of the published revision into a new draft, the clone of a
//! plan's published revision into a new plan, the draft revision's PATCH and delete, and the
//! revision's checks read over fresh SKUs (D-408).
//!
//! A copy writes the revision, every copied item (`unreserved`, no receipt) and one attach op per
//! item in ONE transaction (D-413); the door then drives the attach ops best-effort and answers
//! 201, and the ticker finishes what it could not. A draft revision belongs to its author (D-404):
//! only its `created_by` edits or deletes it and its items.
//!
//! @cpt-dod:cpt-cf-bss-pricing-dod-plan-clone:p1
use super::{
    AuthoringState, configuration,
    dto::{
        PricingPlanChecksDto, PricingPlanClone, PricingPlanCreate, PricingPlanDto, PricingPlanList,
        PricingPlanPatch, PricingPlanRevisionDto, PricingPlanRevisionPatch,
    },
    plan_items,
    support::{self, DoorError},
};
use crate::{
    domain::{
        book::Book,
        plan::{self, PlanContext, ReferenceState, RevisionState},
        price::PriceState,
    },
    infra::{
        reference_registry, reference_work,
        storage::{
            RepoError,
            entity::{plan as plan_entity, plan_item, plan_revision, price_book_entry},
            repo::{
                approval_repo, book_repo, dimension_repo, plan_item_repo, plan_repo,
                plan_revision_repo, price_book_entry_repo, price_repo, reference_op_repo,
            },
        },
    },
};
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use bss_products_sdk::models::Sku;
use std::collections::BTreeSet;
use std::sync::Arc;
use toolkit_canonical_errors::CanonicalError;
use toolkit_db::secure::{AccessScope, DBRunner};
use toolkit_security::SecurityContext;
use uuid::Uuid;

/// A plan of the caller's tenant, or 404.
pub(super) async fn find_plan(
    tx: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
) -> Result<plan_entity::Model, DoorError> {
    plan_repo::find(tx, scope, tenant, id)
        .await?
        .ok_or_else(|| support::missing_what("plan").into())
}
/// A revision of the caller's tenant, or 404.
pub(super) async fn find_revision(
    tx: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
) -> Result<plan_revision::Model, DoorError> {
    plan_revision_repo::find(tx, scope, tenant, id)
        .await?
        .ok_or_else(|| support::missing_what("plan_revision").into())
}
/// Whether the revision is a draft no pending unit holds.
pub(super) fn open_draft(r: &plan_revision::Model) -> bool {
    r.state == RevisionState::Draft.as_str() && r.pending_unit_id.is_none()
}
/// A revision the caller may edit, with its items: an unlocked draft (else 409
/// `REVISION_NOT_DRAFT`) that the caller created (else 403 `NOT_DRAFT_AUTHOR`, D-404).
/// # Errors
/// The two refusals above.
pub(super) fn editable(r: &plan_revision::Model, ctx: &SecurityContext) -> Result<(), DoorError> {
    if !open_draft(r) {
        return Err(support::conflict("REVISION_NOT_DRAFT").into());
    }
    if r.created_by != ctx.subject_id() {
        return Err(support::forbidden_because(
            "NOT_DRAFT_AUTHOR",
            format!("plan revision {} is a draft of another author", r.id),
        )
        .into());
    }
    Ok(())
}
async fn plan_body(
    tx: &impl DBRunner,
    tenant: Uuid,
    m: plan_entity::Model,
) -> Result<PricingPlanDto, DoorError> {
    let revisions =
        plan_revision_repo::for_plan(tx, &AccessScope::for_tenant(tenant), tenant, m.id).await?;
    Ok(PricingPlanDto::of(m, &revisions))
}
async fn revision_body(
    tx: &impl DBRunner,
    tenant: Uuid,
    m: plan_revision::Model,
) -> Result<PricingPlanRevisionDto, DoorError> {
    let items =
        plan_item_repo::for_revision(tx, &AccessScope::for_tenant(tenant), tenant, m.id).await?;
    Ok(PricingPlanRevisionDto::of(m, items))
}
fn etag(version: i64) -> Result<u64, CanonicalError> {
    Ok(
        crate::api::rest::preconditions::RowVersion::from_stored(version)
            .map_err(CanonicalError::from)?
            .get(),
    )
}

/// `POST /plans`: the plan and its draft rev 1 on the named book, in the key's transaction.
/// # Errors
/// 400 `PLAN_CODE_REQUIRED`; 404 for a book the tenant does not hold; 409 `PLAN_CODE_TAKEN`; a
/// replayed or conflicting key.
#[allow(
    clippy::too_many_arguments,
    reason = "authorized context, replay identity and input belong to one transaction"
)]
pub(super) async fn create(
    tx: &impl DBRunner,
    scope: &AccessScope,
    ctx: &SecurityContext,
    correlation: Uuid,
    key: &str,
    digest: &[u8],
    input: PricingPlanCreate,
) -> Result<Response, DoorError> {
    let tenant = ctx.subject_tenant_id();
    let endpoint = "/bss-pricing/v1/plans";
    if let Some(replay) = support::claim(tx, tenant, endpoint, key, digest).await? {
        return Ok(replay);
    }
    if input.code.trim().is_empty() {
        return Err(support::invalid("code", "PLAN_CODE_REQUIRED").into());
    }
    let children = AccessScope::for_tenant(tenant);
    if book_repo::find(tx, &children, tenant, input.book_id)
        .await?
        .is_none()
    {
        return Err(support::missing().into());
    }
    let now = time::OffsetDateTime::now_utc();
    // @cpt-begin:cpt-cf-bss-pricing-flow-plans:p1:inst-plans-flow-1
    let p = plan_repo::insert(
        tx,
        scope,
        plan_entity::Model {
            id: Uuid::now_v7(),
            tenant_id: tenant,
            code: input.code,
            name: input.name,
            published_rev: None,
            version: 1,
            created_by: ctx.subject_id(),
            created_at: now,
            updated_at: now,
        },
    )
    .await?;
    let r = plan_revision_repo::insert(
        tx,
        &children,
        plan_revision::Model {
            id: Uuid::now_v7(),
            tenant_id: tenant,
            plan_id: p.id,
            rev_no: 1,
            book_id: input.book_id,
            state: RevisionState::Draft.as_str().into(),
            available_from: None,
            pending_unit_id: None,
            approved_by_unit_id: None,
            published_at: None,
            version: 1,
            created_by: ctx.subject_id(),
            created_at: now,
            updated_at: now,
        },
    )
    .await?;
    // @cpt-end:cpt-cf-bss-pricing-flow-plans:p1:inst-plans-flow-1
    support::audit(tx, ctx, correlation, "plan.create", p.id, 1).await?;
    support::audit(tx, ctx, correlation, "plan_revision.create", r.id, 1).await?;
    let body = PricingPlanDto::of(p, &[r]);
    support::answer(
        tx,
        tenant,
        endpoint,
        key,
        StatusCode::CREATED,
        &body,
        Some(1),
    )
    .await
}
/// `GET /plans`: the tenant's plans by code, each with its revision headers.
/// # Errors
/// Storage failures.
pub(super) async fn list(
    tx: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
) -> Result<PricingPlanList, DoorError> {
    let mut items = Vec::new();
    for p in plan_repo::list(tx, scope, tenant).await? {
        items.push(plan_body(tx, tenant, p).await?);
    }
    Ok(PricingPlanList { items })
}
/// `GET /plans/{id}`: the plan and its version.
/// # Errors
/// 404 for a plan the tenant does not hold.
pub(super) async fn get(
    tx: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
) -> Result<Response, DoorError> {
    let m = find_plan(tx, scope, tenant, id).await?;
    let version = etag(m.version)?;
    Ok(support::response(
        StatusCode::OK,
        &plan_body(tx, tenant, m).await?,
        Some(version),
    )?)
}
/// `PATCH /plans/{id}`: rename at the version the caller read.
/// # Errors
/// 404; 409 `STALE_REVISION`.
pub(super) async fn patch(
    tx: &impl DBRunner,
    scope: &AccessScope,
    ctx: &SecurityContext,
    correlation: Uuid,
    id: Uuid,
    version: u64,
    input: PricingPlanPatch,
) -> Result<Response, DoorError> {
    let tenant = ctx.subject_tenant_id();
    let m = find_plan(tx, scope, tenant, id).await?;
    support::check_version(version, m.version)?;
    let now = time::OffsetDateTime::now_utc();
    plan_repo::rename(tx, scope, tenant, id, m.version, input.name.clone(), now).await?;
    let m = plan_entity::Model {
        name: input.name,
        version: m.version + 1,
        updated_at: now,
        ..m
    };
    support::audit(tx, ctx, correlation, "plan.patch", id, m.version).await?;
    Ok(support::response(
        StatusCode::OK,
        &plan_body(tx, tenant, m).await?,
        Some(version + 1),
    )?)
}

/// `POST /plans/{id}/revisions`: copy the published revision into a new draft (D-413), then drive
/// the attach ops of its items best-effort and answer 201 with the answer the key recorded.
/// # Errors
/// 404 for an unknown plan; 409 `REVISION_DRAFT_EXISTS` while a draft or pending revision exists;
/// 409 `PLAN_UNPUBLISHED` when there is no published revision to copy; a replayed or conflicting
/// key.
pub(super) async fn copy(
    state: Arc<AuthoringState>,
    scope: AccessScope,
    ctx: SecurityContext,
    correlation: Uuid,
    plan_id: Uuid,
    key: String,
    digest: Vec<u8>,
) -> Result<Response, CanonicalError> {
    let original_ctx = ctx.clone();
    let (response, ops) = support::transaction(&state.db.db(), move |tx| {
        let (scope, ctx, key, digest) = (scope.clone(), ctx.clone(), key.clone(), digest.clone());
        Box::pin(
            async move { copy_in(tx, &scope, &ctx, correlation, plan_id, &key, &digest).await },
        )
    })
    .await?;
    plan_items::drive_best_effort(&state, &original_ctx, &ops).await;
    Ok(response)
}
#[allow(
    clippy::too_many_arguments,
    reason = "authorized context, replay identity and the plan belong to one transaction"
)]
async fn copy_in(
    tx: &impl DBRunner,
    scope: &AccessScope,
    ctx: &SecurityContext,
    correlation: Uuid,
    plan_id: Uuid,
    key: &str,
    digest: &[u8],
) -> Result<(Response, Vec<Uuid>), DoorError> {
    let tenant = ctx.subject_tenant_id();
    let endpoint = format!("/bss-pricing/v1/plans/{plan_id}/revisions");
    if let Some(replay) = support::claim(tx, tenant, &endpoint, key, digest).await? {
        return Ok((replay, Vec::new()));
    }
    let children = AccessScope::for_tenant(tenant);
    let p = find_plan(tx, scope, tenant, plan_id).await?;
    let revisions = plan_revision_repo::for_plan(tx, &children, tenant, p.id).await?;
    let open = [
        RevisionState::Draft.as_str(),
        RevisionState::Pending.as_str(),
    ];
    if revisions.iter().any(|r| open.contains(&r.state.as_str())) {
        return Err(support::conflict("REVISION_DRAFT_EXISTS").into());
    }
    // @cpt-begin:cpt-cf-bss-pricing-flow-plans:p1:inst-plans-flow-1
    let source = revisions
        .iter()
        .find(|r| r.state == RevisionState::Published.as_str())
        .ok_or_else(|| support::conflict("PLAN_UNPUBLISHED"))?;
    let rev_no = revisions
        .iter()
        .map(|r| r.rev_no)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| support::conflict("REVISION_NO_TAKEN"))?;
    let now = time::OffsetDateTime::now_utc();
    let r = plan_revision_repo::insert(
        tx,
        &children,
        plan_revision::Model {
            id: Uuid::now_v7(),
            tenant_id: tenant,
            plan_id,
            rev_no,
            book_id: source.book_id,
            state: RevisionState::Draft.as_str().into(),
            available_from: source.available_from,
            pending_unit_id: None,
            approved_by_unit_id: None,
            published_at: None,
            version: 1,
            created_by: ctx.subject_id(),
            created_at: now,
            updated_at: now,
        },
    )
    .await?;
    let (items, ops) = copy_items(tx, &children, ctx, correlation, source.id, r.id, now).await?;
    // @cpt-end:cpt-cf-bss-pricing-flow-plans:p1:inst-plans-flow-1
    support::audit(tx, ctx, correlation, "plan_revision.copy", r.id, 1).await?;
    let body = PricingPlanRevisionDto::of(r, items);
    let response = support::answer(
        tx,
        tenant,
        &endpoint,
        key,
        StatusCode::CREATED,
        &body,
        Some(1),
    )
    .await?;
    Ok((response, ops))
}

/// Copy every item of the `source` revision into the new draft `target`, in the caller's
/// transaction (D-413): each copy is written `unreserved` with no receipt and authored by the
/// caller, with one attach op per copy. Answers the copies and their op ids, for the door to drive
/// after its commit.
#[allow(
    clippy::too_many_arguments,
    reason = "the copy's context, both revisions and its clock belong to one transaction"
)]
async fn copy_items(
    tx: &impl DBRunner,
    children: &AccessScope,
    ctx: &SecurityContext,
    correlation: Uuid,
    source: Uuid,
    target: Uuid,
    now: time::OffsetDateTime,
) -> Result<(Vec<plan_item::Model>, Vec<Uuid>), DoorError> {
    let tenant = ctx.subject_tenant_id();
    let mut items = Vec::new();
    let mut ops = Vec::new();
    for from in plan_item_repo::for_revision(tx, children, tenant, source).await? {
        let copy = plan_item_repo::insert(
            tx,
            children,
            plan_item::Model {
                id: Uuid::now_v7(),
                tenant_id: tenant,
                revision_id: target,
                sku_id: from.sku_id,
                price_book_entry_id: from.price_book_entry_id,
                treatment: from.treatment,
                included_qty: from.included_qty,
                qty_min: from.qty_min,
                reservation_id: None,
                reference_state: ReferenceState::Unreserved.as_str().into(),
                version: 1,
                created_by: ctx.subject_id(),
                created_at: now,
                updated_at: now,
            },
        )
        .await?;
        // @cpt-begin:cpt-cf-bss-pricing-flow-plans:p1:inst-plans-flow-2
        let op = reference_work::attach_op(ctx, &copy, correlation, now)?;
        ops.push(op.op_id);
        reference_op_repo::insert(tx, children, op).await?;
        // @cpt-end:cpt-cf-bss-pricing-flow-plans:p1:inst-plans-flow-2
        items.push(copy);
    }
    Ok((items, ops))
}

/// `POST /plans/{id}/clone`: a new plan (its own code and name) whose draft rev 1 copies the
/// source plan's PUBLISHED revision — book, sale date and items — under D-413, then drive the
/// attach ops of its items best-effort and answer 201 with the new plan. Nothing of the source's
/// approval is copied: no decision, no `approved_by_unit_id` or `published_at`, no pin; a
/// deprecated SKU is carried, and the new plan's checks show it red (D-408).
/// # Errors
/// 400 `PLAN_CODE_REQUIRED`; 404 for a plan the tenant does not hold; 409
/// `CLONE_SOURCE_UNPUBLISHED` when the source has no published revision; 409 `PLAN_CODE_TAKEN`;
/// a replayed or conflicting key.
#[allow(
    clippy::too_many_arguments,
    reason = "authorized context, replay identity and input belong to one transaction"
)]
pub(super) async fn clone(
    state: Arc<AuthoringState>,
    scope: AccessScope,
    ctx: SecurityContext,
    correlation: Uuid,
    source: Uuid,
    key: String,
    digest: Vec<u8>,
    input: PricingPlanClone,
) -> Result<Response, CanonicalError> {
    let original_ctx = ctx.clone();
    let (response, ops) = support::transaction(&state.db.db(), move |tx| {
        let (scope, ctx, key, digest) = (scope.clone(), ctx.clone(), key.clone(), digest.clone());
        let input = input.clone();
        Box::pin(async move {
            clone_in(
                tx,
                &scope,
                &ctx,
                correlation,
                source,
                (&key, &digest),
                input,
            )
            .await
        })
    })
    .await?;
    plan_items::drive_best_effort(&state, &original_ctx, &ops).await;
    Ok(response)
}
async fn clone_in(
    tx: &impl DBRunner,
    scope: &AccessScope,
    ctx: &SecurityContext,
    correlation: Uuid,
    source: Uuid,
    (key, digest): (&str, &[u8]),
    input: PricingPlanClone,
) -> Result<(Response, Vec<Uuid>), DoorError> {
    let tenant = ctx.subject_tenant_id();
    let endpoint = format!("/bss-pricing/v1/plans/{source}/clone");
    if let Some(replay) = support::claim(tx, tenant, &endpoint, key, digest).await? {
        return Ok((replay, Vec::new()));
    }
    if input.code.trim().is_empty() {
        return Err(support::invalid("code", "PLAN_CODE_REQUIRED").into());
    }
    let children = AccessScope::for_tenant(tenant);
    let from = find_plan(tx, scope, tenant, source).await?;
    // @cpt-begin:cpt-cf-bss-pricing-algo-plans-clone-and-retire:p1:inst-plans-clone-and-retire-1
    let published = plan_revision_repo::for_plan(tx, &children, tenant, from.id)
        .await?
        .into_iter()
        .find(|r| r.state == RevisionState::Published.as_str())
        .ok_or_else(|| support::conflict("CLONE_SOURCE_UNPUBLISHED"))?;
    let now = time::OffsetDateTime::now_utc();
    let p = plan_repo::insert(
        tx,
        scope,
        plan_entity::Model {
            id: Uuid::now_v7(),
            tenant_id: tenant,
            code: input.code,
            name: input.name,
            published_rev: None,
            version: 1,
            created_by: ctx.subject_id(),
            created_at: now,
            updated_at: now,
        },
    )
    .await?;
    let r = plan_revision_repo::insert(
        tx,
        &children,
        plan_revision::Model {
            id: Uuid::now_v7(),
            tenant_id: tenant,
            plan_id: p.id,
            rev_no: 1,
            book_id: published.book_id,
            state: RevisionState::Draft.as_str().into(),
            available_from: published.available_from,
            pending_unit_id: None,
            approved_by_unit_id: None,
            published_at: None,
            version: 1,
            created_by: ctx.subject_id(),
            created_at: now,
            updated_at: now,
        },
    )
    .await?;
    let (_, ops) = copy_items(tx, &children, ctx, correlation, published.id, r.id, now).await?;
    // @cpt-end:cpt-cf-bss-pricing-algo-plans-clone-and-retire:p1:inst-plans-clone-and-retire-1
    support::audit(tx, ctx, correlation, "plan.clone", p.id, 1).await?;
    support::audit(tx, ctx, correlation, "plan_revision.create", r.id, 1).await?;
    let body = PricingPlanDto::of(p, &[r]);
    let response = support::answer(
        tx,
        tenant,
        &endpoint,
        key,
        StatusCode::CREATED,
        &body,
        Some(1),
    )
    .await?;
    Ok((response, ops))
}

/// `GET /plan-revisions/{id}`: the revision with its items and its version.
/// # Errors
/// 404 for a revision the tenant does not hold.
pub(super) async fn get_revision(
    tx: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
) -> Result<Response, DoorError> {
    let m = find_revision(tx, scope, tenant, id).await?;
    let version = etag(m.version)?;
    Ok(support::response(
        StatusCode::OK,
        &revision_body(tx, tenant, m).await?,
        Some(version),
    )?)
}
/// `PATCH /plan-revisions/{id}`: the book and the sale date of an unlocked draft of the caller,
/// at the version the caller read. A book change remaps every item whose entry has a twin in the
/// new book (the same SKU, charge kind and period); an unmatched item keeps its old entry, which
/// the checks then show foreign (`ITEM_BOOK_FOREIGN`).
/// # Errors
/// 404; 409 `REVISION_NOT_DRAFT`; 403 `NOT_DRAFT_AUTHOR`; 409 `STALE_REVISION`; 400
/// `DATE_INVALID`; 404 for a book the tenant does not hold.
pub(super) async fn patch_revision(
    tx: &impl DBRunner,
    scope: &AccessScope,
    ctx: &SecurityContext,
    correlation: Uuid,
    id: Uuid,
    version: u64,
    input: PricingPlanRevisionPatch,
) -> Result<Response, DoorError> {
    let tenant = ctx.subject_tenant_id();
    let children = AccessScope::for_tenant(tenant);
    let m = find_revision(tx, scope, tenant, id).await?;
    editable(&m, ctx)?;
    support::check_version(version, m.version)?;
    let now = time::OffsetDateTime::now_utc();
    let mut next = m.clone();
    if let Some(from) = input.available_from {
        next.available_from = support::date(from, "available_from")?;
    }
    let mut items = plan_item_repo::for_revision(tx, &children, tenant, id).await?;
    if let Some(book) = input.book_id {
        if book_repo::find(tx, &children, tenant, book)
            .await?
            .is_none()
        {
            return Err(support::missing().into());
        }
        if book != m.book_id {
            remap(tx, &children, ctx, correlation, book, &mut items, now).await?;
        }
        next.book_id = book;
    }
    next.updated_at = now;
    plan_revision_repo::update_draft(tx, &children, next.clone()).await?;
    next.version += 1;
    support::audit(
        tx,
        ctx,
        correlation,
        "plan_revision.patch",
        id,
        next.version,
    )
    .await?;
    Ok(support::response(
        StatusCode::OK,
        &PricingPlanRevisionDto::of(next, items),
        Some(version + 1),
    )?)
}
/// Point every item at the new book's entry of the same (SKU, charge kind, period), where there
/// is one; the rest keep their entry.
async fn remap(
    tx: &impl DBRunner,
    scope: &AccessScope,
    ctx: &SecurityContext,
    correlation: Uuid,
    book: Uuid,
    items: &mut [plan_item::Model],
    now: time::OffsetDateTime,
) -> Result<(), DoorError> {
    let tenant = ctx.subject_tenant_id();
    let twins = price_book_entry_repo::for_book(tx, scope, tenant, book).await?;
    for item in items.iter_mut() {
        let Some(entry) = item.price_book_entry_id else {
            continue;
        };
        let Some(old) = price_book_entry_repo::find(tx, scope, tenant, entry).await? else {
            continue;
        };
        let Some(twin) = twins.iter().find(|e| {
            e.sku_id == old.sku_id && e.charge_kind == old.charge_kind && e.period == old.period
        }) else {
            continue;
        };
        item.price_book_entry_id = Some(twin.id);
        item.updated_at = now;
        plan_item_repo::update_draft(tx, scope, item.clone()).await?;
        item.version += 1;
        support::audit(
            tx,
            ctx,
            correlation,
            "plan_item.remap",
            item.id,
            item.version,
        )
        .await?;
    }
    Ok(())
}
/// `DELETE /plan-revisions/{id}`: remove an unlocked draft of the caller with every item, each
/// with its delete op, in one transaction (D-414); then drive the releases best-effort and
/// answer 204.
/// # Errors
/// 404; 409 `REVISION_NOT_DRAFT`; 403 `NOT_DRAFT_AUTHOR`; 409 `ITEM_CONFIRMATION_PENDING` while
/// an item's confirm is outstanding; 409 `STALE_REVISION` for a lost race.
pub(super) async fn delete_revision(
    state: Arc<AuthoringState>,
    scope: AccessScope,
    ctx: SecurityContext,
    correlation: Uuid,
    id: Uuid,
) -> Result<Response, CanonicalError> {
    let original_ctx = ctx.clone();
    let ops = support::transaction(&state.db.db(), move |tx| {
        let (scope, ctx) = (scope.clone(), ctx.clone());
        Box::pin(async move {
            let tenant = ctx.subject_tenant_id();
            let children = AccessScope::for_tenant(tenant);
            let m = find_revision(tx, &scope, tenant, id).await?;
            editable(&m, &ctx)?;
            let mut ops = Vec::new();
            for item in plan_item_repo::for_revision(tx, &children, tenant, id).await? {
                ops.push(plan_items::remove(tx, &children, &ctx, correlation, item.id).await?);
            }
            plan_revision_repo::delete_draft(tx, &children, tenant, id, m.version).await?;
            support::audit(tx, &ctx, correlation, "plan_revision.delete", id, m.version).await?;
            Ok(ops)
        })
    })
    .await?;
    plan_items::drive_best_effort(&state, &original_ctx, &ops).await;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `GET /plan-revisions/{id}/checks`: every check on the sale date (D-408). The stored state is
/// read in one transaction; then every item SKU is read fresh through `sku_for_write`, never from a
/// cache, so a SKU deprecated since the last read turns its check red at once. A SKU Products no
/// longer knows (404) is unavailable in the checks; any other definite refusal is answered as
/// Products gave it; a registry that cannot answer is 503 `REGISTRY_UNAVAILABLE`.
/// # Errors
/// 404 for a revision the tenant does not hold; the registry's refusal or unavailability.
pub(super) async fn checks(
    state: &AuthoringState,
    scope: AccessScope,
    ctx: SecurityContext,
    id: Uuid,
) -> Result<Response, CanonicalError> {
    let tenant = ctx.subject_tenant_id();
    let mut context = support::transaction(&state.db.db(), move |tx| {
        let scope = scope.clone();
        Box::pin(async move { stored_context(tx, &scope, tenant, id).await })
    })
    .await?;
    // @cpt-begin:cpt-cf-bss-pricing-flow-plans:p1:inst-plans-flow-3
    context.skus = fresh_skus(&state.hub, &ctx, context.items.iter().map(|i| i.sku_id)).await?;
    let today = time::OffsetDateTime::now_utc().date();
    let rows = plan::checks(&context, today);
    let ready = plan::ready(&rows);
    // @cpt-end:cpt-cf-bss-pricing-flow-plans:p1:inst-plans-flow-3
    let body = PricingPlanChecksDto {
        checks: rows.into_iter().map(Into::into).collect(),
        ready,
        sale_date: plan::sale_date(&context.revision, today).to_string(),
    };
    support::response(StatusCode::OK, &body, None)
}
/// Every SKU named, read fresh through `sku_for_write` and never from a cache (D-408): the checks
/// door, submit and apply all read the item SKUs this way. A SKU Products no longer knows (404)
/// is left out, so the checks show it unavailable.
/// # Errors
/// Any other definite refusal as Products gave it; a registry that cannot answer is 503
/// `REGISTRY_UNAVAILABLE`.
pub async fn fresh_skus(
    hub: &toolkit::ClientHub,
    ctx: &SecurityContext,
    skus: impl IntoIterator<Item = Uuid>,
) -> Result<Vec<Sku>, CanonicalError> {
    let registry = reference_registry::resolve(hub).map_err(|_| support::unavailable())?;
    let wanted: BTreeSet<Uuid> = skus.into_iter().collect();
    let mut found = Vec::with_capacity(wanted.len());
    // @cpt-begin:cpt-cf-bss-pricing-algo-plans-revision-checks:p1:inst-plans-revision-checks-1
    for sku in wanted {
        match registry
            .sku_for_write(ctx, ctx.subject_tenant_id(), sku)
            .await
        {
            Ok(read) => found.push(read),
            Err(error) if error.status_code() == 404 => {}
            Err(error) if reference_work::definite_refusal(&error) => return Err(error),
            Err(_) => return Err(support::unavailable()),
        }
    }
    // @cpt-end:cpt-cf-bss-pricing-algo-plans-revision-checks:p1:inst-plans-revision-checks-1
    Ok(found)
}
fn corrupt(what: String) -> DoorError {
    RepoError::CorruptRow(what).into()
}
/// Everything the checks read from storage, with no SKU yet: the checks door and the
/// `plan_revision` subject build the checks' context through this one function.
/// # Errors
/// 404 for a revision the tenant does not hold; storage failures.
pub async fn stored_context(
    tx: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
) -> Result<PlanContext, DoorError> {
    let children = AccessScope::for_tenant(tenant);
    let r = find_revision(tx, scope, tenant, id).await?;
    let p = plan_repo::find(tx, &children, tenant, r.plan_id)
        .await?
        .ok_or_else(|| corrupt(format!("revision {id} has no plan")))?;
    let rows = plan_item_repo::for_revision(tx, &children, tenant, r.id).await?;
    let mut items = Vec::with_capacity(rows.len());
    let mut entries: Vec<plan::Entry> = Vec::new();
    let mut book_ids = BTreeSet::from([r.book_id]);
    for row in &rows {
        items.push(item_of(row)?);
        let Some(entry_id) = row.price_book_entry_id else {
            continue;
        };
        if entries.iter().any(|e| e.id == entry_id) {
            continue;
        }
        if let Some(e) = price_book_entry_repo::find(tx, &children, tenant, entry_id).await? {
            book_ids.insert(e.book_id);
            entries.push(entry_of(tx, &children, tenant, e).await?);
        }
    }
    let mut books = Vec::with_capacity(book_ids.len());
    for book_id in book_ids {
        if let Some(b) = book_repo::find(tx, &children, tenant, book_id).await? {
            books.push(plan::PlanBook {
                id: b.id,
                book: Book {
                    name: b.name,
                    currency: b.currency,
                    valid_from: b.valid_from,
                    valid_until: b.valid_until,
                },
            });
        }
    }
    let mut dimension_values = Vec::new();
    for d in dimension_repo::list(tx, &children, tenant).await? {
        let values: Vec<String> = serde_json::from_value(d.values)
            .map_err(|_| corrupt(format!("dimension {} values", d.key)))?;
        dimension_values.push((d.key, values));
    }
    let published = plan_revision_repo::for_plan(tx, &children, tenant, p.id)
        .await?
        .into_iter()
        .find(|x| x.state == RevisionState::Published.as_str());
    let published_sku_ids = match published {
        Some(published) => plan_item_repo::for_revision(tx, &children, tenant, published.id)
            .await?
            .into_iter()
            .map(|i| i.sku_id)
            .collect(),
        None => Vec::new(),
    };
    let quorum = approval_repo::read_policy(tx, &children, tenant)
        .await?
        .quorum_for(plan::KIND_PLAN_REVISION);
    let settings = configuration::settings(tx, &children, tenant).await?;
    Ok(PlanContext {
        plan: plan::Plan {
            id: p.id,
            code: p.code,
            name: p.name,
        },
        revision: plan::Revision {
            id: r.id,
            rev_no: r.rev_no,
            book_id: r.book_id,
            state: r
                .state
                .parse()
                .map_err(|_| corrupt(format!("revision {} state", r.id)))?,
            available_from: r.available_from,
        },
        items,
        skus: Vec::new(),
        entries,
        books,
        dimension_values,
        published_sku_ids,
        quorum,
        defaults: plan::Defaults {
            gl: settings.default_gl,
            rounding: settings.default_rounding,
            tax_category: settings.default_tax_category,
        },
    })
}
fn item_of(m: &plan_item::Model) -> Result<plan::Item, DoorError> {
    let bad = |what: &str| corrupt(format!("plan item {} {what}", m.id));
    Ok(plan::Item {
        id: m.id,
        sku_id: m.sku_id,
        price_book_entry_id: m.price_book_entry_id,
        treatment: m.treatment.parse().map_err(|_| bad("treatment"))?,
        included_qty: m
            .included_qty
            .as_deref()
            .map(str::parse)
            .transpose()
            .map_err(|_| bad("included_qty"))?,
        qty_min: m.qty_min,
        reference: plan::Reference {
            state: m
                .reference_state
                .parse()
                .map_err(|_| bad("reference_state"))?,
            reservation_id: m.reservation_id,
        },
    })
}
async fn entry_of(
    tx: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    e: price_book_entry::Model,
) -> Result<plan::Entry, DoorError> {
    let bad = |what: &str| corrupt(format!("entry {} {what}", e.id));
    let stored = price_repo::for_entry(tx, scope, tenant, e.id).await?;
    let pending = stored
        .iter()
        .filter(|p| p.state == PriceState::Pending.as_str())
        .filter_map(|p| {
            p.pending_unit_id.map(|unit_id| plan::PendingPrice {
                price_id: p.id,
                unit_id,
            })
        })
        .collect();
    let prices = stored
        .iter()
        .map(price_repo::to_domain)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(plan::Entry {
        id: e.id,
        book_id: e.book_id,
        sku_id: e.sku_id,
        charge_kind: e.charge_kind.parse().map_err(|_| bad("charge_kind"))?,
        period: e.period.clone(),
        dimension_key: e.dimension_key.clone(),
        reference_state: e
            .reference_state
            .parse()
            .map_err(|_| bad("reference_state"))?,
        prices,
        pending,
    })
}
