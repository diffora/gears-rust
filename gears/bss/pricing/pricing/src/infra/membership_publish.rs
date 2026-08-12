//! The membership plane's publish commit — `dod-customer-group`'s "every
//! committed membership mutation MUST be its own publish unit through the
//! Foundation engine" (`design/09-price-overlays.md:472-473`).
//!
//! # Why this is small where [`crate::infra::overlay_publish`] is not
//!
//! An overlay publish is seven steps because the thing being published is a
//! **draft revision**: it has to be re-read and re-validated against a world
//! that may have moved since it was authored, flipped `draft -> published`,
//! and its predecessor superseded, all inside the transaction that requests
//! addressability. A membership mutation has none of that shape. There is no
//! draft table (`inst-mm-pending`, §3: "memberships have no `draft` state"),
//! so there is nothing to re-validate against a moved world and nothing to
//! flip — [`group_membership_repo::enroll`] / [`group_membership_repo::end_membership`]
//! already validate and write in one call, and they already write their own
//! audit record (`inst-mm-audit`). What is left for this module is exactly
//! two of the overlay's seven steps: request a `CatalogVersion`, and record
//! one pending ref per subject.
//!
//! # Every method here takes a runner, and opens no transaction of its own
//!
//! Unlike [`crate::infra::overlay_publish::OverlayPublishService::commit`],
//! which owns its transaction, every method here is a `_in`-style function
//! over the caller's own runner —
//! [`crate::infra::window::WindowService::schedule_in`]'s shape and reason:
//! `POST …/members` and `POST …/move` carry a **required** `Idempotency-Key`
//! (§5), so their transaction has to be the one
//! [`crate::infra::idempotent::guarded`] opens around its claim — a service
//! that opened a second transaction of its own could not be called from
//! inside that closure at all (`toolkit_db` refuses `Db::conn()` inside an
//! open transaction, `infra::idempotent`'s own module doc). `PATCH
//! …/members/{id}` carries no idempotency key (it is guarded by `If-Match`
//! instead) and opens its own transaction directly at the route.
//!
//! # Ordering: the mutation runs before the registry is asked
//!
//! [`crate::infra::overlay_publish`]'s module doc states the registry-request
//! rule: ask only **after** every check that can still refuse, because a
//! refusal after the handle is issued leaves a pending version nothing ever
//! commits (`commit_overdue` for a publish that never happened). For an
//! overlay the checks are separable from the write; for a membership row they
//! are not — `enroll`/`end_membership` validate and write atomically — so the
//! rule is applied at that granularity instead: the repository call runs
//! first, and the registry is asked only once it has returned `Ok`.
//!
//! # This is the audit-only path only (`inst-mm-renewal`)
//!
//! Every method here commits directly and opens no approval unit. The
//! material path — `inst-mm-immediate`'s explicit re-resolution and
//! `inst-mm-bulk`'s always-material group move — needs the payload-in-
//! `pricing_approval` shape `inst-mm-pending` describes and is a separate,
//! not-yet-built task. Building it here would mean un-refusing
//! `AuditSubjectKind::Membership` in `approval_repo::subject_aggregate` and
//! `infra::approval::re_derive`, which this module deliberately leaves exactly
//! as they stand.
//!
//! # No outbox event, and that is the design set's own words
//!
//! §7 states the membership half of D-06 explicitly: a publish unit "requests
//! `CatalogVersion` addressability and warms the read model **without a
//! dedicated event**" — consumers observe `CatalogVersionPublished` plus the
//! warmed content, never a wire name of their own. The overlay plane later
//! gained one (`PriceOverlayPublished`, D-248, 2026-08-07) as an explicit,
//! separately-decided amendment to that same paragraph; no analogous decision
//! exists for membership, and `CatalogEvent`'s fourteen names are a **frozen**
//! set enforced by `chk_pricing_outbox_event_name` — widening it costs a
//! migration that rebuilds the table on `SQLite` (`m20260802_000060`'s own
//! account) and is exactly the "add a variant to a shared enum" this task was
//! told to stop and report rather than do. So this module never calls
//! `outbox_repo::enqueue`: not because the function is unavailable — it is
//! exactly as generic as `catalog_version_ref_repo::record_pending` — but
//! because there is no frozen name for it to carry.
//!
//! # No `void_pending_for_subject` either, and that is scope rather than oversight
//!
//! `overlay_repo`'s commit voids any approval unit still `submitted` over the
//! revision it just froze, because a draft revision can be re-submitted and
//! re-approved before it publishes, leaving an orphan behind. A membership row
//! has no draft-then-approve cycle in this task's scope — the audit-only path
//! opens no unit at all — so there is nothing an enrollment or an end could
//! orphan today. Task 6b, which opens membership approval units for the
//! material path, is where a void call would first have a subject shape to
//! target; inventing one now would risk guessing that shape wrong.

use chrono::{DateTime, Utc};
use toolkit_db::secure::{AccessScope, DBRunner};
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::domain::audit::AuditStamp;
use crate::domain::error::DomainError;
use crate::domain::ports::{CatalogVersionRegistryV1, registry_failure};
use crate::domain::read_model::SubjectRef;
use crate::infra::storage::repo::{
    NewMembership, PendingVersionRow, catalog_version_ref_repo, group_membership_repo,
};
use crate::infra::storage::repo_failure;

/// What one membership publish unit produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MembershipPublishReceipt {
    /// The membership row as it stands after the mutation.
    pub membership: group_membership_repo::MembershipRow,
    /// The registry's **pending** handle — not a `CatalogVersion`, a value
    /// standing in for one: the commit requests an assignment and
    /// `CatalogVersionPublished` resolves it.
    pub pending_ref: String,
}

/// What one atomic move produced — two membership rows moved on one handle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MembershipMoveReceipt {
    /// The membership that was ended, when the payer held an active one at
    /// `at`. `None` on a move that finds no active membership to transfer out
    /// of — the atomic move degrades to a plain enrollment rather than
    /// refusing, since D-09's own error scenarios name no such refusal.
    pub ended: Option<group_membership_repo::MembershipRow>,
    /// The membership that was created in the target group.
    pub enrolled: group_membership_repo::MembershipRow,
    /// The one registry handle both subjects were recorded against.
    pub pending_ref: String,
}

/// Enroll a payer, and give the enrollment its own publish unit, inside
/// `runner`'s transaction.
///
/// # Errors
/// [`DomainError::GroupUnknown`] when `new.group_value` is absent from the
/// tenant's customer-group taxonomy or is retired there (§5:257); whatever
/// [`group_membership_repo::enroll`] refuses (`RepoError::MembershipOverlap` /
/// `MembershipConflict` / `MembershipIntervalEmpty`, mapped through
/// [`repo_failure`]); [`DomainError::CatalogVersionUnavailable`] when the
/// registry cannot be reached; [`DomainError::Internal`] on a storage failure.
pub async fn enroll_in(
    runner: &impl DBRunner,
    registry: &dyn CatalogVersionRegistryV1,
    ctx: &SecurityContext,
    scope: &AccessScope,
    tenant_id: Uuid,
    new: NewMembership,
    stamp: AuditStamp,
) -> Result<MembershipPublishReceipt, DomainError> {
    let membership_id = new.membership_id;
    let request_id = enroll_request_id(tenant_id, membership_id);

    require_active_group(runner, scope, tenant_id, &new.group_value).await?;

    let membership = group_membership_repo::enroll(runner, scope, tenant_id, new, stamp)
        .await
        .map_err(|e| repo_failure(&e))?;

    let pending = registry
        .request_version(ctx, &request_id)
        .await
        .map_err(|e| registry_failure(&e))?;

    record_ref(
        runner,
        scope,
        tenant_id,
        membership_id,
        &pending.pending_ref,
        stamp.recorded_at,
    )
    .await?;

    Ok(MembershipPublishReceipt {
        membership,
        pending_ref: pending.pending_ref,
    })
}

/// End a membership, and give the end its own publish unit, inside `runner`'s
/// transaction.
///
/// # Errors
/// Whatever [`group_membership_repo::end_membership`] refuses, including
/// `RepoError::StaleRowVersion` on a stale `expected_version` (mapped through
/// [`repo_failure`]); the registry/storage errors [`enroll_in`] documents.
#[allow(
    clippy::too_many_arguments,
    reason = "every argument is a fact only the caller holds: the runner and registry, the \
              security context, the scope, the tenant, the membership being ended, the instant \
              and the precondition it is being ended under, and the stamp. `enroll_in` carries \
              the same note for the same reason."
)]
pub async fn end_in(
    runner: &impl DBRunner,
    registry: &dyn CatalogVersionRegistryV1,
    ctx: &SecurityContext,
    scope: &AccessScope,
    tenant_id: Uuid,
    membership_id: Uuid,
    at: DateTime<Utc>,
    expected_version: u64,
    stamp: AuditStamp,
) -> Result<MembershipPublishReceipt, DomainError> {
    let request_id = end_request_id(tenant_id, membership_id, at);

    let membership = group_membership_repo::end_membership(
        runner,
        scope,
        tenant_id,
        membership_id,
        at,
        expected_version,
        stamp,
    )
    .await
    .map_err(|e| repo_failure(&e))?;

    let pending = registry
        .request_version(ctx, &request_id)
        .await
        .map_err(|e| registry_failure(&e))?;

    record_ref(
        runner,
        scope,
        tenant_id,
        membership_id,
        &pending.pending_ref,
        stamp.recorded_at,
    )
    .await?;

    Ok(MembershipPublishReceipt {
        membership,
        pending_ref: pending.pending_ref,
    })
}

/// The atomic move (`inst-ms-move`, D-09): end the payer's active membership
/// (if any) and enroll the new one in `target_group`, both at `at`, on **one**
/// registry handle, inside `runner`'s transaction.
///
/// Two audit records land — one from the end, one from the enroll, each
/// [`group_membership_repo`]'s own — which is `inst-ms-move`'s "one atomic
/// audited mutation" read as *one transaction, fully audited* rather than as
/// *one row*: the two repository calls already audit themselves, and
/// duplicating that here would be a second description of one act.
///
/// # Errors
/// [`DomainError::GroupUnknown`] when `target_group` is absent from the
/// tenant's customer-group taxonomy or is retired there, checked before
/// either write; whatever the two repository calls refuse; the
/// registry/storage errors [`enroll_in`] documents.
#[allow(
    clippy::too_many_arguments,
    reason = "every argument is a fact only the caller holds: the runner and registry, the \
              security context, the scope, the tenant, the payer being moved, the new \
              membership's own minted id, the target group, the instant the move pivots on and \
              the stamp. Folding any pair of them into a struct would move the argument list \
              rather than shorten it."
)]
pub async fn move_payer_in(
    runner: &impl DBRunner,
    registry: &dyn CatalogVersionRegistryV1,
    ctx: &SecurityContext,
    scope: &AccessScope,
    tenant_id: Uuid,
    payer_tenant_id: Uuid,
    new_membership_id: Uuid,
    target_group: String,
    at: DateTime<Utc>,
    stamp: AuditStamp,
) -> Result<MembershipMoveReceipt, DomainError> {
    let request_id = move_request_id(tenant_id, new_membership_id);

    // Validated before either write: a move that ended the payer's prior
    // membership and only then discovered the target group unknown would
    // have to roll the whole transaction back anyway, and validating first
    // means it never has to.
    require_active_group(runner, scope, tenant_id, &target_group).await?;

    let intervals =
        group_membership_repo::intervals_for_payer(runner, scope, tenant_id, payer_tenant_id)
            .await
            .map_err(|e| repo_failure(&e))?;
    let active = group_membership_repo::resolve_active_membership(&intervals, at).cloned();

    let ended = if let Some(active) = active {
        Some(
            group_membership_repo::end_membership(
                runner,
                scope,
                tenant_id,
                active.membership_id,
                at,
                active.row_version,
                stamp,
            )
            .await
            .map_err(|e| repo_failure(&e))?,
        )
    } else {
        None
    };

    let enrolled = group_membership_repo::enroll(
        runner,
        scope,
        tenant_id,
        NewMembership {
            membership_id: new_membership_id,
            tenant_id,
            payer_tenant_id,
            group_value: target_group,
            effective_from: at,
            effective_to: None,
        },
        stamp,
    )
    .await
    .map_err(|e| repo_failure(&e))?;

    let pending = registry
        .request_version(ctx, &request_id)
        .await
        .map_err(|e| registry_failure(&e))?;

    if let Some(ended) = &ended {
        record_ref(
            runner,
            scope,
            tenant_id,
            ended.membership_id,
            &pending.pending_ref,
            stamp.recorded_at,
        )
        .await?;
    }
    record_ref(
        runner,
        scope,
        tenant_id,
        enrolled.membership_id,
        &pending.pending_ref,
        stamp.recorded_at,
    )
    .await?;

    Ok(MembershipMoveReceipt {
        ended,
        enrolled,
        pending_ref: pending.pending_ref,
    })
}

/// Refuse a `{group}` the tenant's customer-group taxonomy does not currently
/// declare — absent, or declared and then **retired** (`GROUP_UNKNOWN`, §5).
///
/// Both are `GroupUnknown`: the retired case is named explicitly because it
/// is the one `inst-cg-taxonomy`'s retire guard exists to catch, and folding
/// it into "not found" would read as an oversight rather than the guard
/// working — see [`DomainError::GroupUnknown`]'s own doc.
async fn require_active_group(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    group_value: &str,
) -> Result<(), DomainError> {
    let entry = crate::infra::storage::repo::taxonomy_repo::find_customer_group_on(
        runner,
        scope,
        tenant_id,
        group_value,
    )
    .await
    .map_err(|e| repo_failure(&e))?;
    match entry {
        Some(entry) if entry.state == crate::domain::taxonomy::TaxonomyState::Active => Ok(()),
        Some(_) => Err(DomainError::GroupUnknown(format!(
            "customer group `{group_value}` is retired; a membership cannot be created or moved \
             into a retired group"
        ))),
        None => Err(DomainError::GroupUnknown(format!(
            "customer group `{group_value}` is not declared in this tenant's customer-group \
             taxonomy"
        ))),
    }
}

/// Record one pending ref for one membership subject.
///
/// No revision and no lifecycle state — `ProjectedSubject::GroupMembership`'s
/// own reason (`crate::infra::read_model`): `pricing_group_membership` carries
/// no revision-scoped content table, so there is nothing for the ref row to
/// pin beyond the subject reference itself. The projector reads the row live
/// through [`group_membership_repo::find`].
async fn record_ref(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    membership_id: Uuid,
    pending_ref: &str,
    requested_at: DateTime<Utc>,
) -> Result<(), DomainError> {
    catalog_version_ref_repo::record_pending(
        runner,
        scope,
        PendingVersionRow::for_subject(
            tenant_id,
            pending_ref.to_owned(),
            &SubjectRef::GroupMembership(membership_id),
            None,
            None,
            requested_at,
        ),
    )
    .await
    .map_err(|e| repo_failure(&e))
}

/// The registry request id of one enrollment.
///
/// Deterministic on `(tenant, membership_id)`: the caller mints
/// `membership_id` once and [`group_membership_repo::enroll`] refuses a second
/// insert under it (`RepoError::ConcurrentMutation`), so a rolled-back retry
/// of the same attempt re-presents the same id and re-requests the same
/// handle rather than orphaning one — `overlay_publish::publish_request_id`'s
/// argument, applied to a row that is minted once rather than revisioned.
#[must_use]
fn enroll_request_id(tenant_id: Uuid, membership_id: Uuid) -> String {
    format!("membership-enroll/{tenant_id}/{membership_id}")
}

/// The registry request id of one end. Includes the instant, at the
/// millisecond quantum every act-naming request id in this crate uses
/// (`cutover_unit_ref`'s reason): two ends of one membership at two different
/// instants are two different acts, and a retry of one must not collide with
/// the other's handle.
#[must_use]
fn end_request_id(tenant_id: Uuid, membership_id: Uuid, at: DateTime<Utc>) -> String {
    format!(
        "membership-end/{tenant_id}/{membership_id}/{}",
        at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    )
}

/// The registry request id of one move. Keyed on the **new** membership's
/// minted id rather than on the payer or the old membership, for
/// `enroll_request_id`'s reason: the new id is minted once by the caller and
/// the enroll half refuses a second insert under it, so it is the one fact in
/// this act guaranteed not to repeat across two genuinely different moves.
#[must_use]
fn move_request_id(tenant_id: Uuid, new_membership_id: Uuid) -> String {
    format!("membership-move/{tenant_id}/{new_membership_id}")
}
