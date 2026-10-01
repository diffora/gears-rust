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
//! Every read renders a revision's state as it reads today (D-447), derived in memory from the
//! stored rows; no read writes. The three doors that can meet a due scheduled revision — the
//! copy, the clone and the unschedule door — persist its switch first (D-451), with the job's
//! event and audit row.
//!
//! @cpt-dod:cpt-cf-bss-pricing-dod-plan-clone:p1
use super::{
    AuthoringState, configuration,
    dto::{
        self, PlanReading, PricingPlanApprovalProgress, PricingPlanChecksDto, PricingPlanClone,
        PricingPlanCreate, PricingPlanDto, PricingPlanList, PricingPlanPatch,
        PricingPlanRevisionDto, PricingPlanRevisionPatch,
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
        events::TxOutbox,
        plan_revisions, reference_registry, reference_work,
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
use std::collections::{BTreeMap, BTreeSet};
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
/// Today, the UTC date of now: the day every read derives a revision's state on (D-447), as the
/// checks judge a sale date on it.
pub(super) fn today() -> time::Date {
    time::OffsetDateTime::now_utc().date()
}
/// What the plan DTO shows beside the rows of the plans whose revisions `revisions` yields (one
/// slice per plan): the item SKUs of each plan's current revision and of the revision in effect
/// (D-460, D-480) and the instants of every unit the revisions name (D-461), ONE grouped statement
/// each whatever the number of plans. It borrows the revisions: a single plan's read clones none
/// (the phase 9 review's R51).
async fn plan_reading<'a>(
    tx: &impl DBRunner,
    tenant: Uuid,
    revisions: impl IntoIterator<Item = &'a [plan_revision::Model]>,
    today: time::Date,
) -> Result<PlanReading, DoorError> {
    let children = AccessScope::for_tenant(tenant);
    let (mut wanted, mut units) = (Vec::new(), Vec::new());
    for own in revisions {
        wanted.extend(dto::current_revision(own, today)?);
        wanted.extend(dto::in_effect_revision(own, today)?);
        units.extend(dto::named_units(own));
    }
    wanted.sort_unstable();
    wanted.dedup();
    Ok(PlanReading {
        skus: plan_item_repo::skus_of_revisions(tx, &children, tenant, &wanted).await?,
        units: approval_repo::unit_instants(tx, &children, tenant, &units).await?,
    })
}
async fn plan_body(
    tx: &impl DBRunner,
    tenant: Uuid,
    m: plan_entity::Model,
) -> Result<PricingPlanDto, DoorError> {
    let own =
        plan_revision_repo::for_plan(tx, &AccessScope::for_tenant(tenant), tenant, m.id).await?;
    let today = today();
    let reading = plan_reading(tx, tenant, [own.as_slice()], today).await?;
    Ok(PricingPlanDto::of(m, &own, today, &reading)?)
}
/// A revision read (D-480): its items, its state among its plan's revisions as it reads today,
/// the instants of the unit it names (D-461), its vote progress while pending (D-462), then one
/// grouped read of the entries the items name, one admission of those entries' books under the
/// caller's `price_book` read (`books`, or none without that grant) and one grouped read of the
/// admitted entries' default-chain prices on the sale date. A draft or pending revision also
/// reads the in-effect revision's item SKUs, one statement whether or not one is in effect.
async fn revision_read(
    tx: &impl DBRunner,
    books: Option<&AccessScope>,
    tenant: Uuid,
    m: plan_revision::Model,
) -> Result<dto::PricingPlanRevisionReadDto, DoorError> {
    let children = AccessScope::for_tenant(tenant);
    let items = plan_item_repo::for_revision(tx, &children, tenant, m.id).await?;
    let mut entry_ids: Vec<Uuid> = items.iter().filter_map(|i| i.price_book_entry_id).collect();
    entry_ids.sort_unstable();
    entry_ids.dedup();
    let siblings = plan_revision_repo::for_plan(tx, &children, tenant, m.plan_id).await?;
    let today = today();
    let mut dto = PricingPlanRevisionDto::read(&m, &siblings, items, today)?;
    if let Some(named) = m.pending_unit_id.or(m.approved_by_unit_id)
        && let Some(unit) = approval_repo::find_unit(tx, &children, tenant, named)
            .await
            .map_err(support::approval_failure)?
    {
        let approval = progress(tx, tenant, &unit).await?;
        dto = dto.with_units(&instants_of(&unit), approval);
    }
    let sale = plan::sale_date(
        &plan::Revision {
            id: m.id,
            rev_no: m.rev_no,
            book_id: m.book_id,
            state: dto.state.into(),
            available_from: m.available_from,
        },
        today,
    );
    let entries = price_book_entry_repo::find_many(tx, &children, tenant, &entry_ids).await?;
    let mut book_ids: Vec<Uuid> = entries.iter().map(|e| e.book_id).collect();
    book_ids.sort_unstable();
    book_ids.dedup();
    let admitted: BTreeSet<Uuid> = match books {
        Some(scope) => book_repo::find_many(tx, scope, tenant, &book_ids)
            .await?
            .into_iter()
            .map(|b| b.id)
            .collect(),
        None => BTreeSet::new(),
    };
    let shown: Vec<&price_book_entry::Model> = entries
        .iter()
        .filter(|e| admitted.contains(&e.book_id))
        .collect();
    let mut headlines = if shown.is_empty() {
        BTreeMap::new()
    } else {
        super::price_book_entries::headline(tx, tenant, &shown, sale).await?
    };
    let summaries = entries
        .into_iter()
        .map(|e| {
            let price = admitted
                .contains(&e.book_id)
                .then(|| headlines.remove(&e.id).and_then(|h| h.current))
                .flatten();
            Ok(dto::PricingPlanEntrySummary::of(e, price)?)
        })
        .collect::<Result<Vec<_>, DoorError>>()?;
    let state: plan::RevisionState = dto.state.into();
    let carried_sku_ids = if matches!(
        state,
        plan::RevisionState::Draft | plan::RevisionState::Pending
    ) {
        let id = dto::in_effect_revision(&siblings, today)?;
        let ids: Vec<Uuid> = id.into_iter().collect();
        let map = plan_item_repo::skus_of_revisions(tx, &children, tenant, &ids).await?;
        Some(id.and_then(|i| map.get(&i).cloned()).unwrap_or_default())
    } else {
        None
    };
    Ok(dto::PricingPlanRevisionReadDto {
        revision: dto,
        sale_date: sale.to_string(),
        entries: summaries,
        carried_sku_ids,
    })
}
/// A unit's instants, keyed as the DTOs read them (D-461).
pub(super) fn instants_of(
    unit: &bss_approval::Unit,
) -> BTreeMap<Uuid, approval_repo::UnitInstants> {
    BTreeMap::from([(
        unit.id,
        approval_repo::UnitInstants {
            id: unit.id,
            submitted_at: unit.submitted_at,
            decided_at: unit.decided_at,
        },
    )])
}
/// A pending unit's vote progress (D-462, O-9a): the approve votes the quorum counts, by the
/// approval library's `counted_approvals` over the unit's decisions (the count the vote door
/// judges by: it reads no item and names no actor), and the quorum; `None` for a unit that is not
/// pending. One statement, read with the tenant's scope under the revision read's plan read:
/// counts only.
pub(super) async fn progress(
    tx: &impl DBRunner,
    tenant: Uuid,
    unit: &bss_approval::Unit,
) -> Result<Option<PricingPlanApprovalProgress>, DoorError> {
    if unit.state != bss_approval::UnitState::Pending {
        return Ok(None);
    }
    let decisions =
        approval_repo::decisions_of_units(tx, &AccessScope::for_tenant(tenant), tenant, &[unit.id])
            .await?
            .remove(&unit.id)
            .unwrap_or_default();
    Ok(progress_of(unit, &decisions))
}
/// [`progress`] over a unit's decisions already read.
pub(super) fn progress_of(
    unit: &bss_approval::Unit,
    decisions: &[bss_approval::Decision],
) -> Option<PricingPlanApprovalProgress> {
    (unit.state == bss_approval::UnitState::Pending).then(|| PricingPlanApprovalProgress {
        unit_id: unit.id,
        approvals: bss_approval::counted_approvals(unit, decisions),
        quorum_required: unit.quorum_required,
    })
}
fn etag(version: i64) -> Result<u64, CanonicalError> {
    Ok(
        crate::api::rest::preconditions::RowVersion::from_stored(version)
            .map_err(CanonicalError::from)?
            .get(),
    )
}

/// The book a plan names is one its author may read (D-456, extending D-440's money rule):
/// `books` is the caller's `price_book` read, `None` without that grant. A book of the tenant it
/// does not admit is 403 `PRICE_BOOK_READ_REQUIRED`.
/// # Errors
/// That refusal; storage failures.
async fn require_book_read(
    tx: &impl DBRunner,
    books: Option<&AccessScope>,
    tenant: Uuid,
    book: Uuid,
) -> Result<(), DoorError> {
    if super::price_book_entries::shows_money(tx, books, tenant, book).await? {
        Ok(())
    } else {
        Err(support::forbidden_because(
            "PRICE_BOOK_READ_REQUIRED",
            "a plan names a book: authoring it takes price_book read on that book",
        )
        .into())
    }
}
/// A new plan's code (D-468): blank is 400 `PLAN_CODE_REQUIRED`, as before; then a code that does
/// not follow the rule ([`plan::code_follows_the_rule`]) is 400 `PLAN_CODE_INVALID`, judged as
/// sent. The door's length cap (`FIELD_TOO_LONG`, 64, D-457) is judged before this, with the body.
fn judge_code(code: &str) -> Result<(), DoorError> {
    if code.trim().is_empty() {
        return Err(support::invalid("code", "PLAN_CODE_REQUIRED").into());
    }
    if !plan::code_follows_the_rule(code) {
        return Err(support::invalid_because(
            "code",
            "PLAN_CODE_INVALID",
            "a plan code is 1 to 32 characters of A-Z, 0-9, - and _, starting with a letter or \
             a digit",
        )
        .into());
    }
    Ok(())
}
/// `POST /plans`: the plan and its draft rev 1 on the named book, with the body's sale date if it
/// names one (D-463), in the key's transaction. The book is one the caller's `price_book` read
/// admits (`books`, D-456).
/// # Errors
/// 400 `PLAN_CODE_REQUIRED`, `PLAN_CODE_INVALID` (D-468) or `DATE_INVALID`; 404 for a book the tenant does not hold; 403
/// `PRICE_BOOK_READ_REQUIRED` for one the caller may not read; 409 `PLAN_CODE_TAKEN`; a replayed
/// or conflicting key.
pub(super) async fn create(
    tx: &impl DBRunner,
    (scope, books): (&AccessScope, Option<&AccessScope>),
    ctx: &SecurityContext,
    correlation: Uuid,
    (key, digest): (&str, &[u8]),
    input: PricingPlanCreate,
) -> Result<Response, DoorError> {
    let tenant = ctx.subject_tenant_id();
    let endpoint = "/bss-pricing/v1/plans";
    if let Some(replay) = support::claim(tx, tenant, endpoint, key, digest).await? {
        return Ok(replay);
    }
    judge_code(&input.code)?;
    // D-463: judged as the revision PATCH judges it, among the body's refusals (D-456's order).
    let available_from = support::date(input.available_from.clone(), "available_from")?;
    let children = AccessScope::for_tenant(tenant);
    if book_repo::find(tx, &children, tenant, input.book_id)
        .await?
        .is_none()
    {
        return Err(support::missing().into());
    }
    require_book_read(tx, books, tenant, input.book_id).await?;
    let now = crate::infra::storage::stored_now();
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
            available_from,
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
    // A write answers what it wrote (D-453): an empty draft that names no unit (D-460, D-461).
    let body = PricingPlanDto::of(p, &[r], today(), &PlanReading::default())?;
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
/// `GET /plans`: the tenant's plans by code, each with its revision headers as they read today
/// (D-447), its current revision and the one in effect (D-460) and each header's instants
/// (D-461); with `sku`, only the plans that have a draft, pending, scheduled or published revision
/// naming the SKU through an entry (D-434, the SKU usage's `plans`; the stored state counts,
/// D-446). Four set-based statements whatever the number of plans (one when there is none): the
/// plans, all their revisions, the current revisions' items and the units the revisions name;
/// the derivation is in memory.
/// # Errors
/// Storage failures.
pub(super) async fn list(
    tx: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    sku: Option<Uuid>,
) -> Result<PricingPlanList, DoorError> {
    let plans = match sku {
        Some(sku) => plan_repo::naming_sku(tx, scope, tenant, sku).await?,
        None => plan_repo::list(tx, scope, tenant).await?,
    };
    if plans.is_empty() {
        return Ok(PricingPlanList { items: Vec::new() });
    }
    let ids: Vec<Uuid> = plans.iter().map(|p| p.id).collect();
    let today = today();
    let mut revisions: BTreeMap<Uuid, Vec<plan_revision::Model>> = BTreeMap::new();
    for r in
        plan_revision_repo::for_plans(tx, &AccessScope::for_tenant(tenant), tenant, &ids).await?
    {
        revisions.entry(r.plan_id).or_default().push(r);
    }
    let reading = plan_reading(tx, tenant, revisions.values().map(Vec::as_slice), today).await?;
    Ok(PricingPlanList {
        items: plans
            .into_iter()
            .map(|p| {
                let own = revisions.remove(&p.id).unwrap_or_default();
                PricingPlanDto::of(p, &own, today, &reading)
            })
            .collect::<Result<_, _>>()?,
    })
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
    let now = crate::infra::storage::stored_now();
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
/// the attach ops of its items best-effort and answer 201 with the answer the key recorded. A due
/// scheduled revision is switched first (D-451), so the copy is of the revision in effect.
/// # Errors
/// 404 for an unknown plan; 409 `REVISION_DRAFT_EXISTS` while a draft or pending revision exists;
/// 409 `REVISION_SCHEDULED` while a revision waits for its sale date (D-451); 409
/// `PLAN_UNPUBLISHED` when there is no published revision to copy; a replayed or conflicting key.
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
    let db = state.db.db();
    let (response, ops) =
        support::transaction_with_events(&db, &state.outbox, move |tx, outbox| {
            let (scope, ctx, key, digest) =
                (scope.clone(), ctx.clone(), key.clone(), digest.clone());
            Box::pin(async move {
                copy_in(
                    tx,
                    &outbox,
                    &scope,
                    &ctx,
                    correlation,
                    plan_id,
                    (&key, &digest),
                )
                .await
            })
        })
        .await?;
    plan_items::drive_best_effort(&state, &original_ctx, &ops).await;
    Ok(response)
}
async fn copy_in(
    tx: &(impl DBRunner + Sync),
    outbox: &TxOutbox,
    scope: &AccessScope,
    ctx: &SecurityContext,
    correlation: Uuid,
    plan_id: Uuid,
    (key, digest): (&str, &[u8]),
) -> Result<(Response, Vec<Uuid>), DoorError> {
    let tenant = ctx.subject_tenant_id();
    let endpoint = format!("/bss-pricing/v1/plans/{plan_id}/revisions");
    if let Some(replay) = support::claim(tx, tenant, &endpoint, key, digest).await? {
        return Ok((replay, Vec::new()));
    }
    let children = AccessScope::for_tenant(tenant);
    let p = find_plan(tx, scope, tenant, plan_id).await?;
    let now = crate::infra::storage::stored_now();
    // @cpt-begin:cpt-cf-bss-pricing-algo-plans-revision-switch:p1:inst-plans-revision-switch-3
    plan_revisions::catch_up(tx, outbox, tenant, p.id, now, correlation).await?;
    let revisions = plan_revision_repo::for_plan(tx, &children, tenant, p.id).await?;
    let open = [
        RevisionState::Draft.as_str(),
        RevisionState::Pending.as_str(),
    ];
    if revisions.iter().any(|r| open.contains(&r.state.as_str())) {
        return Err(support::conflict("REVISION_DRAFT_EXISTS").into());
    }
    // D-451: one scheduled revision at a time, and nothing after it: withdraw it or wait.
    if revisions
        .iter()
        .any(|r| r.state == RevisionState::Scheduled.as_str())
    {
        return Err(support::conflict("REVISION_SCHEDULED").into());
    }
    // @cpt-end:cpt-cf-bss-pricing-algo-plans-revision-switch:p1:inst-plans-revision-switch-3
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
    let body = PricingPlanRevisionDto::of(&r, items)?;
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
        // D-467: a copy is a new row, which the repository writes `paid` with no quantity
        // whatever the source row carries; a legacy item without an entry stays one, so the
        // draft's checks show it ITEM_ENTRY_MISSING.
        let copy = plan_item_repo::insert(
            tx,
            children,
            plan_item::Model {
                id: Uuid::now_v7(),
                revision_id: target,
                reservation_id: None,
                reference_state: ReferenceState::Unreserved.as_str().into(),
                version: 1,
                created_by: ctx.subject_id(),
                created_at: now,
                updated_at: now,
                ..from
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
/// source plan's PUBLISHED revision — the one in effect: a due scheduled revision is switched
/// first (D-451) — book, sale date and items, under D-413, then drive the
/// attach ops of its items best-effort and answer 201 with the new plan. The body's
/// `available_from` overrides the copied sale date, and null clears it (D-463). Nothing of the
/// source's approval is copied: no decision, no `approved_by_unit_id` or `published_at`, no pin;
/// a deprecated SKU is carried, and the new plan's checks show it red (D-408).
/// # Errors
/// 400 `PLAN_CODE_REQUIRED`, `PLAN_CODE_INVALID` (D-468) or `DATE_INVALID`; 404 for a plan the
/// tenant does not hold; 409 `CLONE_SOURCE_UNPUBLISHED` when the source has no published revision; 403
/// `PRICE_BOOK_READ_REQUIRED` when the caller's `price_book` read (`books`) does not admit the
/// book the clone names, the source's (D-456); 409 `PLAN_CODE_TAKEN`; a replayed or conflicting
/// key.
#[expect(
    clippy::too_many_arguments,
    reason = "authorized context, replay identity and input belong to one transaction"
)]
pub(super) async fn clone(
    state: Arc<AuthoringState>,
    (scope, books): (AccessScope, Option<AccessScope>),
    ctx: SecurityContext,
    correlation: Uuid,
    source: Uuid,
    key: String,
    digest: Vec<u8>,
    input: PricingPlanClone,
) -> Result<Response, CanonicalError> {
    let original_ctx = ctx.clone();
    let db = state.db.db();
    let (response, ops) =
        support::transaction_with_events(&db, &state.outbox, move |tx, outbox| {
            let (scope, books, ctx, key, digest) = (
                scope.clone(),
                books.clone(),
                ctx.clone(),
                key.clone(),
                digest.clone(),
            );
            let input = input.clone();
            Box::pin(async move {
                clone_in(
                    tx,
                    &outbox,
                    (&scope, books.as_ref()),
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
#[expect(
    clippy::too_many_arguments,
    reason = "authorized context, replay identity and input belong to one transaction"
)]
async fn clone_in(
    tx: &(impl DBRunner + Sync),
    outbox: &TxOutbox,
    (scope, books): (&AccessScope, Option<&AccessScope>),
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
    judge_code(&input.code)?;
    // D-463: omitted keeps the source's sale date; a date overrides it; null clears it. Judged as
    // the revision PATCH judges it, among the body's refusals (D-456's order).
    let available_from = input
        .available_from
        .clone()
        .map(|from| support::date(from, "available_from"))
        .transpose()?;
    let children = AccessScope::for_tenant(tenant);
    let from = find_plan(tx, scope, tenant, source).await?;
    let now = crate::infra::storage::stored_now();
    // @cpt-begin:cpt-cf-bss-pricing-algo-plans-revision-switch:p1:inst-plans-revision-switch-3
    plan_revisions::catch_up(tx, outbox, tenant, from.id, now, correlation).await?;
    // @cpt-end:cpt-cf-bss-pricing-algo-plans-revision-switch:p1:inst-plans-revision-switch-3
    // @cpt-begin:cpt-cf-bss-pricing-algo-plans-clone-and-retire:p1:inst-plans-clone-and-retire-1
    let published = plan_revision_repo::for_plan(tx, &children, tenant, from.id)
        .await?
        .into_iter()
        .find(|r| r.state == RevisionState::Published.as_str())
        .ok_or_else(|| support::conflict("CLONE_SOURCE_UNPUBLISHED"))?;
    // The clone names the source's book: its author must be able to read it (D-456).
    require_book_read(tx, books, tenant, published.book_id).await?;
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
            available_from: available_from.unwrap_or(published.available_from),
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
    let (items, ops) = copy_items(tx, &children, ctx, correlation, published.id, r.id, now).await?;
    // @cpt-end:cpt-cf-bss-pricing-algo-plans-clone-and-retire:p1:inst-plans-clone-and-retire-1
    support::audit(tx, ctx, correlation, "plan.clone", p.id, 1).await?;
    support::audit(tx, ctx, correlation, "plan_revision.create", r.id, 1).await?;
    // A write answers what it wrote (D-453): the new draft with the items it copied (D-460).
    let mut skus: Vec<Uuid> = items.iter().map(|i| i.sku_id).collect();
    skus.sort_unstable();
    let reading = PlanReading {
        skus: BTreeMap::from([(r.id, skus)]),
        units: BTreeMap::new(),
    };
    let body = PricingPlanDto::of(p, &[r], today(), &reading)?;
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

/// `GET /plan-revisions/{id}`: the revision with its items and its version, the instants of the
/// unit it names (D-461), its vote progress while pending (D-462) and the read-only fields of
/// D-480 (sale date, entry summaries, carried SKUs). `books` is the caller's `price_book` read,
/// `None` without that grant: an entry of a book it does not admit has a null sale-date price.
/// The money's 503 is the handler's, before this read, so a missing revision is 404 only after
/// the policy can judge (D-440).
/// # Errors
/// 404 for a revision the tenant does not hold.
pub(super) async fn get_revision(
    tx: &impl DBRunner,
    scope: &AccessScope,
    books: Option<&AccessScope>,
    tenant: Uuid,
    id: Uuid,
) -> Result<Response, DoorError> {
    let m = find_revision(tx, scope, tenant, id).await?;
    let version = etag(m.version)?;
    Ok(support::response(
        StatusCode::OK,
        &revision_read(tx, books, tenant, m).await?,
        Some(version),
    )?)
}
/// `GET /plan-revisions/{id}/reservations` (D-480): each item's reference, under plan read. Two
/// statements: the revision's find (404 when the tenant does not hold it) and its items.
/// # Errors
/// 404 for a revision the tenant does not hold.
pub(super) async fn reservations(
    tx: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
) -> Result<Response, DoorError> {
    let m = find_revision(tx, scope, tenant, id).await?;
    let items =
        plan_item_repo::for_revision(tx, &AccessScope::for_tenant(tenant), tenant, m.id).await?;
    let items = items
        .iter()
        .map(dto::PricingPlanReservationItemDto::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    let settled = items.iter().all(|item| {
        !matches!(
            item.reference_state,
            crate::api::rest::closed_sets::PricingItemReferenceState::Unreserved
                | crate::api::rest::closed_sets::PricingItemReferenceState::ConfirmationPending
        )
    });
    Ok(support::response(
        StatusCode::OK,
        &dto::PricingPlanReservationsDto { items, settled },
        None,
    )?)
}
/// `POST /plan-revisions/{id}/unschedule` (D-452): a scheduled revision whose sale date has not
/// come returns to an unlocked draft — `approved_by_unit_id` cleared, its items and their
/// references kept — and the door answers the draft, its version the `ETag` its PATCH takes. The
/// order is the authorization (the handler's `plan:submit`), the key's claim, the plan's
/// catch-up (D-451), then the state: a due revision has just been switched and is in effect. The
/// applied unit stays applied; no event. A refusal rolls the catch-up back with it; the job
/// persists that switch.
/// # Errors
/// 404 for a revision the tenant does not hold; 409 `REVISION_IN_EFFECT` for a published revision;
/// 409 `REVISION_NOT_SCHEDULED` for a draft, pending or superseded one; a replayed or conflicting
/// key.
pub(super) async fn unschedule(
    state: Arc<AuthoringState>,
    scope: AccessScope,
    ctx: SecurityContext,
    correlation: Uuid,
    id: Uuid,
    key: String,
    digest: Vec<u8>,
) -> Result<Response, CanonicalError> {
    let db = state.db.db();
    support::transaction_with_events(&db, &state.outbox, move |tx, outbox| {
        let (scope, ctx, key, digest) = (scope.clone(), ctx.clone(), key.clone(), digest.clone());
        Box::pin(async move {
            unschedule_in(tx, &outbox, &scope, &ctx, correlation, id, (&key, &digest)).await
        })
    })
    .await
}
async fn unschedule_in(
    tx: &(impl DBRunner + Sync),
    outbox: &TxOutbox,
    scope: &AccessScope,
    ctx: &SecurityContext,
    correlation: Uuid,
    id: Uuid,
    (key, digest): (&str, &[u8]),
) -> Result<Response, DoorError> {
    let tenant = ctx.subject_tenant_id();
    let endpoint = format!("/bss-pricing/v1/plan-revisions/{id}/unschedule");
    if let Some(replay) = support::claim(tx, tenant, &endpoint, key, digest).await? {
        return Ok(replay);
    }
    let children = AccessScope::for_tenant(tenant);
    let r = find_revision(tx, scope, tenant, id).await?;
    let now = crate::infra::storage::stored_now();
    // @cpt-begin:cpt-cf-bss-pricing-algo-plans-revision-switch:p1:inst-plans-revision-switch-3
    plan_revisions::catch_up(tx, outbox, tenant, r.plan_id, now, correlation).await?;
    // @cpt-end:cpt-cf-bss-pricing-algo-plans-revision-switch:p1:inst-plans-revision-switch-3
    // @cpt-begin:cpt-cf-bss-pricing-algo-plans-revision-switch:p1:inst-plans-revision-switch-4
    let r = find_revision(tx, &children, tenant, id).await?;
    if r.state == RevisionState::Published.as_str() {
        return Err(support::conflict("REVISION_IN_EFFECT").into());
    }
    if r.state != RevisionState::Scheduled.as_str() {
        return Err(support::conflict("REVISION_NOT_SCHEDULED").into());
    }
    plan_revision_repo::unschedule(tx, &children, tenant, id, now).await?;
    let draft = plan_revision::Model {
        state: RevisionState::Draft.as_str().into(),
        approved_by_unit_id: None,
        version: r.version + 1,
        updated_at: now,
        ..r
    };
    support::audit(
        tx,
        ctx,
        correlation,
        "plan_revision.unschedule",
        id,
        draft.version,
    )
    .await?;
    // @cpt-end:cpt-cf-bss-pricing-algo-plans-revision-switch:p1:inst-plans-revision-switch-4
    let items = plan_item_repo::for_revision(tx, &children, tenant, id).await?;
    let body = PricingPlanRevisionDto::of(&draft, items)?;
    support::answer(
        tx,
        tenant,
        &endpoint,
        key,
        StatusCode::OK,
        &body,
        Some(etag(draft.version)?),
    )
    .await
}
/// `PATCH /plan-revisions/{id}`: the book and the sale date of an unlocked draft of the caller,
/// at the version the caller read. A book change remaps every item whose entry has a twin in the
/// new book (the same SKU, charge kind, period, model and policy digest, plus equal dimension key,
/// D-502); an unmatched item keeps its old
/// entry, which the checks then show foreign (`ITEM_BOOK_FOREIGN`).
/// A named book is one the caller's `price_book` read admits (`books`, D-456).
/// # Errors
/// 404; 409 `REVISION_NOT_DRAFT`; 403 `NOT_DRAFT_AUTHOR`; 409 `STALE_REVISION`; 400
/// `DATE_INVALID`; 404 for a book the tenant does not hold; 403 `PRICE_BOOK_READ_REQUIRED` for
/// one the caller may not read.
pub(super) async fn patch_revision(
    tx: &impl DBRunner,
    (scope, books): (&AccessScope, Option<&AccessScope>),
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
    let now = crate::infra::storage::stored_now();
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
        require_book_read(tx, books, tenant, book).await?;
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
        &PricingPlanRevisionDto::of(&next, items)?,
        Some(version + 1),
    )?)
}
/// Point each item at the new book's entry of the same (SKU, charge kind, period, model, policy
/// digest) and an equal dimension key (D-427, D-502). Without that full match it keeps its entry,
/// so the checks show `ITEM_BOOK_FOREIGN` instead of changing the policy. A moved item is written
/// in the shape of D-467 (`paid`, no quantity), as the item PATCH writes it, so a legacy row stops
/// being one.
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
            e.sku_id == old.sku_id
                && e.charge_kind == old.charge_kind
                && e.period == old.period
                && e.model == old.model
                && e.usage_policy_digest == old.usage_policy_digest
                && e.dimension_key == old.dimension_key
        }) else {
            continue;
        };
        item.price_book_entry_id = Some(twin.id);
        item.updated_at = now;
        // The repository rewrites the row in D-467's shape (`plan_item_repo::update_draft`).
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
/// answer 204. The last revision of a never-published plan takes the plan with it in the same
/// transaction, freeing its code (D-417); a plan with a published revision stays as it is.
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
            // D-417: a never-published plan left without a revision goes with it.
            let p = plan_repo::find(tx, &children, tenant, m.plan_id)
                .await?
                .ok_or_else(|| corrupt(format!("revision {id} has no plan")))?;
            if p.published_rev.is_none()
                && plan_revision_repo::for_plan(tx, &children, tenant, p.id)
                    .await?
                    .is_empty()
            {
                plan_repo::delete_unpublished(tx, &children, tenant, p.id, p.version).await?;
                support::audit(tx, &ctx, correlation, "plan.delete", p.id, p.version).await?;
            }
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
    let today = today();
    let mut context = support::transaction(&state.db.db(), move |tx| {
        let scope = scope.clone();
        Box::pin(async move { stored_context(tx, &scope, tenant, id, today).await })
    })
    .await?;
    // @cpt-begin:cpt-cf-bss-pricing-flow-plans:p1:inst-plans-flow-3
    context.skus = fresh_skus(&state.hub, &ctx, context.items.iter().map(|i| i.sku_id)).await?;
    let rows = plan::checks(&context, today);
    let ready = plan::ready(&rows);
    // @cpt-end:cpt-cf-bss-pricing-flow-plans:p1:inst-plans-flow-3
    let body = PricingPlanChecksDto {
        checks: rows.into_iter().map(Into::into).collect(),
        ready,
        sale_date: plan::sale_date(&context.revision, today).to_string(),
        quorum_required: context.quorum,
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
    let registry =
        reference_registry::resolve(hub).map_err(|e| support::registry_unavailable(&e))?;
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
            Err(error) => return Err(support::registry_unavailable(&error)),
        }
    }
    // @cpt-end:cpt-cf-bss-pricing-algo-plans-revision-checks:p1:inst-plans-revision-checks-1
    Ok(found)
}
fn corrupt(what: String) -> DoorError {
    RepoError::CorruptRow(what).into()
}
/// Everything the checks read from storage, with no SKU yet: the checks door and the
/// `plan_revision` subject build the checks' context through this one function. The plan's
/// revisions are read as they read on `today` (D-447), so the published revision whose SKUs a
/// deprecated SKU may be carried from is the one in effect, before the job persists a due switch
/// as after it.
/// # Errors
/// 404 for a revision the tenant does not hold; storage failures.
pub async fn stored_context(
    tx: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
    today: time::Date,
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
    let revisions = plan_revisions::effective_revisions(
        &plan_revision_repo::for_plan(tx, &children, tenant, p.id).await?,
        today,
    )?;
    let state = revisions
        .iter()
        .find(|x| x.id == r.id)
        .map(|x| x.state)
        .ok_or_else(|| corrupt(format!("revision {} is not among its plan's", r.id)))?;
    let published_sku_ids =
        plan_revisions::in_effect_skus(tx, &children, tenant, &revisions).await?;
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
            state,
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
    let model = price_book_entry_repo::model_of(&e)?;
    let prices = stored
        .iter()
        .map(|m| price_repo::to_domain(m, model))
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
