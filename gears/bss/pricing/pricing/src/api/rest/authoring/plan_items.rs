//! The plan item's operations on the durable reference machine (D-407, D-413, D-414): a create
//! op below `POST /plan-revisions/{id}/items`, and the removal below `DELETE /plan-items/{id}` and
//! a draft revision's delete. A copy or a clone writes each copied item `unreserved` with its
//! attach op ([`reference_work::attach_op`]) in its own transaction and drives them with
//! [`drive_best_effort`]. Their REST doors, with the input and ownership checks that run before
//! any claim or reservation, are run 3.3's; the ops here are what those doors drive.
use super::{
    AuthoringState,
    dto::PricingPlanItemCreate,
    price_book_entries::{settled, stored},
    support::{self, DoorError},
};
use crate::{
    domain::{
        plan::{ReferenceState, RevisionState},
        reference_op::{OpKind, RefKind},
    },
    infra::{
        reference_work::{self, Caller, Receipt, Ref, Target, WallClock, Work},
        storage::repo::{
            idempotency_repo as idem, plan_item_repo, plan_revision_repo, reference_op_repo,
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
/// Remove one item of an unlocked draft revision and write its delete op, in the caller's
/// transaction; the caller drives the op once it commits. An item whose confirm is still pending
/// is kept until it completes (`ITEM_CONFIRMATION_PENDING`), or its confirm could lose it; an
/// item of a pending, published or superseded revision is never removed (`REVISION_NOT_DRAFT`):
/// its reference outlives the revision (D-414). A draft revision's delete removes every item
/// through this, before the revision itself.
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
    let revision = plan_revision_repo::find(tx, scope, tenant, item.revision_id)
        .await?
        .ok_or_else(|| support::missing_what("plan_revision"))?;
    if revision.state != RevisionState::Draft.as_str() || revision.pending_unit_id.is_some() {
        return Err(support::conflict("REVISION_NOT_DRAFT").into());
    }
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
