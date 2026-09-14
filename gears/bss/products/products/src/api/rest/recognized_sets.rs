//! The recognized-set membership doors — **P-D-90**'s one route family over
//! both sets (`design/03` §3.1 `inst-rs-shape`,
//! `dod-recognized-set-mechanics`, `dod-unit-delist`,
//! `dod-unit-immutable`).
//!
//! # One door family, two sets, the grant chosen by `setKind`
//!
//! `POST /bss-products/v1/recognized-sets/{setKind}/members` adds a member;
//! `POST …/members/{memberCode}/transitions` walks the state machine —
//! `active → deprecated → removed` and the two re-listing edges. The tier
//! set spends `plan_tier × write` and the unit set `recognized_set ×
//! write` (P-D-90 arm 2: the only reading under which both declared grants
//! have a spender). Behind the routes sits **one generic membership
//! implementation** (arm 3), and the kind decides exactly four things: the
//! grant, the event token, the blocked-removal code, and **which holder
//! population the removal counts** — `SetKind::carrier_column`, one
//! `products_sku` column per kind, uniform across both since 03's
//! columns landed (P-D-145, P-D-146; it spanned four until P-D-169). A third route, `POST
//! …/members/{memberCode}/label`, changes a member's display label and
//! nothing else — the rename `dod-plantier-governance` asks for, which by
//! `dod-unit-immutable` can never be a rename of the code.
//!
//! # Every mutation rides `GovernedLiveOp` under the stored host
//!
//! Add, transition and relabel each resolve the stored approval host before
//! their transaction (`api::rest::authorize_live_op`, subject
//! [`member_op_subject`]) and spend the record inside it (P-D-144's shape,
//! P-D-146): without a satisfied record for *this* set member the door
//! answers `APPROVAL_REQUIRED`, and the probes seed the record through the
//! same double the other live-op doors use.
//!
//! # And the record must have been submitted for *this* change (**P-D-172**)
//!
//! The subject names the member and not the act, so a matched record used to
//! answer *"there is a satisfied record for `gold`"* — which authorized a
//! **deprecate** on the strength of two principals agreeing to a **relabel**.
//! [`authorize_member_op`] now compares the record's `content_snapshot` with
//! the door's own declaration ([`add_declaration`],
//! [`transition_declaration`], [`relabel_declaration`]) under `07`'s
//! correction-door rendering, and refuses `APPROVAL_REQUIRED` on a mismatch.
//! A record declaring no op of this slice's — every record written before
//! P-D-172 — authorizes every op as it did, which is the compatibility arm
//! that entry names and owes.
//!
//! # Every mutation rides `GovernedLiveOp` and emits in the same transaction
//!
//! The transition body carries the op's **expected current state**, and the
//! write is pinned at it: a peer's flip between the caller's read and this
//! statement answers `STALE_LIVE_OP`, never a silent absorb. The membership
//! write and the set's event commit in **one transaction** (`inst-rs-shape`),
//! so a consumer never observes a set the events do not explain.
//!
//! # What has no door at all, deliberately
//!
//! There is no rename, no redefine, no DELETE and no `member_code` update —
//! `dod-unit-immutable`'s *"the absence of the door is the enforcement"*,
//! with the migration's guard as the floor: it refuses `member_code` by
//! name, along with `tenant_id`, `set_kind`, `seeded_by` and `created_at`.
//! That is a **complement enumeration**, not the whitelist §4 words —
//! `updated_at` is writable and a later column is admitted by default,
//! which `design/03` §6 carries as an open question. A correction is a new
//! member plus a deprecation of the old, tied through the `GovernedLiveOp`
//! payload.
//!
//! # The removal operand, uniform across the four kinds
//!
//! A removal is refused while a **non-terminal published head** references
//! the member (`inst-us-delist`, M2: frozen version content never blocks),
//! with the holders sampled into the refusal — `UNIT_DELIST_BLOCKED`,
//! `PLAN_TIER_RETIRE_BLOCKED` by kind —
//! and **never at all for a seeded member** (`inst-rs-seeded`). Today only
//! the metering-unit set has a shipped carrier column to hold it
//! (`products_sku.metering_unit`); the tier's carrier arrives
//! with their own columns, and until then their holder population is empty
//! by construction rather than by an exemption.
//!
//! # Why `dod-unit-immutable` is reached and not claimed
//!
//! Its absence half ships (no rename door, the trigger floor, and the
//! migrations suite's probe that no write path mutates a `member_code`) —
//! but the `DoD` also requires a correction to be *"a new unit plus a
//! deprecation of the old, tied through the `GovernedLiveOp` payload so the
//! audit trail carries the pair"*, and neither door carries a tie field yet.
//! A tick would claim the pairing; the bare marker below claims only the
//! reach.
//!
//! # Why `dod-recognized-set-mechanics` is reached and not claimed
//!
//! Its lookup half ships — one generic implementation, the tombstone outside
//! the set, the removal operand uniform across the four kinds — but the `DoD`
//! also obliges *"every mutation riding `GovernedLiveOp`"*, and what these
//! doors carry is the envelope's **staleness pin only**: the transitions
//! body names the expected current state, the add body names nothing, and
//! neither door reaches an approval. The feature's own §5 prices that
//! exactly — this `DoD` spends *"a `05-governance` approval that has no
//! runnable gate"* and therefore owes an **in-test approval double**,
//! *"without which its probe goes green against a gate that approves
//! nothing"*. No double ships here. The tick returns with the envelope and
//! that double, not before.
//!
//! @cpt-cf-bss-products-dod-recognized-set-mechanics
//! @cpt-dod:cpt-cf-bss-products-dod-unit-delist:p1
//!
//! **`dod-unit-delist` is claimed (P-D-121 row 21).** The `deprecated → removed` `UPDATE`
//! re-asserts the census in the same statement (`WHERE NOT EXISTS` a non-terminal published
//! head declaring the member), so a concurrent first publish and the flip cannot both
//! commit. The both-ways probe is
//! `a_removal_is_blocked_by_live_holders_and_admitted_after_them`.
//! @cpt-cf-bss-products-dod-unit-immutable

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Extension, Path};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::Value as JsonValue;
use time::OffsetDateTime;
use toolkit::api::OpenApiRegistry;
use toolkit::api::canonical_prelude::CanonicalError;
use toolkit::api::operation_builder::OperationBuilder;
use toolkit_db::secure::AccessScope;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use toolkit::api::canonical_prelude::resource_error;

use crate::api::rest::{ApiState, repo_error_to_canonical, require_authenticated};
use crate::domain::canonical;
use crate::domain::error::DomainError;
use crate::domain::governance::{GateAuthorization, GateSubject, SubjectPin};
use crate::domain::recognized::{MemberOp, MemberState, SetKind, member_edge};
use crate::domain::validation::ValidationReport;
use crate::infra::events;
use crate::infra::storage::repo::{self, RefusalSubject};

/// The `OpenAPI` tag both doors register under.
const TAG: &str = "BSS Products";

/// The canonical-error identity of this surface's refusals.
#[resource_error(gts_id!("cf.bss.products.recognized_set.v1~"))]
struct RecognizedSetResource;

/// One member as both doors answer it.
#[toolkit_macros::api_dto(response)]
pub struct RecognizedMemberView {
    /// The set the member belongs to.
    pub set_kind: String,
    /// The member's code — the identity that never changes.
    pub member_code: String,
    /// The tier set's operator-facing label; absent elsewhere.
    pub display_label: Option<String>,
    /// `active`, `deprecated` or `removed`.
    pub state: String,
    /// Who seeded it, or absent for an operator-added member.
    pub seeded_by: Option<String>,
}

impl RecognizedMemberView {
    fn from_member(set_kind: SetKind, member: repo::RecognizedMember) -> Self {
        Self {
            set_kind: set_kind.as_str().to_owned(),
            member_code: member.member_code,
            display_label: member.display_label,
            state: member.state.as_str().to_owned(),
            seeded_by: member.seeded_by,
        }
    }
}

/// One whole set as the list door answers it.
///
/// A wrapper object rather than a bare array, which is this corpus's shape for
/// a list that carries anything of its own (`bulk`'s manifest, the freeze
/// participants) — and here it carries `set_kind`, so a client that fanned out
/// over both kinds can tell the answers apart without keeping the request.
#[toolkit_macros::api_dto(response)]
pub struct RecognizedSetView {
    /// The set these members belong to.
    pub set_kind: String,
    /// Every member, `member_code`-ordered — tombstones included, carrying
    /// their state (see [`repo::recognized_members`] for why).
    pub members: Vec<RecognizedMemberView>,
}

/// `POST /recognized-sets/{setKind}/members` request body.
#[toolkit_macros::api_dto(request)]
pub struct AddMemberRequest {
    /// The member's code. Trimmed; must not be blank.
    pub member_code: String,
    /// The tier set's operator-facing label. Stored verbatim; the other
    /// three sets ignore it.
    pub display_label: Option<String>,
}

/// `POST …/members/{memberCode}/transitions` request body — the
/// `GovernedLiveOp` envelope's door shape: the target state and the state
/// the caller read.
#[toolkit_macros::api_dto(request)]
pub struct MemberTransitionRequest {
    /// The state to move to: `deprecated`, `removed` or `active`.
    pub to: String,
    /// The state the caller's read showed — the live-op staleness pin. A
    /// peer's flip in between answers `STALE_LIVE_OP`.
    pub expected_state: String,
}

/// The body of `POST …/members/{memberCode}/label` — the one rename a member
/// admits (`dod-plantier-governance`): the label, never the code.
#[toolkit_macros::api_dto(request)]
pub struct MemberRelabelRequest {
    /// The new display label; `null` clears it.
    pub display_label: Option<String>,
}

/// The grant a call on `set_kind` spends (P-D-90 arm 2), as the label and
/// resource type the authz layer needs.
/// The `GovernedLiveOp` subject one member op rides
/// (`dod-recognized-set-mechanics`; **P-D-146**): the set and the member,
/// unpinned — the door pins staleness through `expected_state`, not through
/// the approval (P-D-120's "no pin" sentinel; see `pin_for_kind`).
/// `pub(crate)` so the tests' seeding double builds the identical subject.
///
/// @cpt-dod:cpt-cf-bss-products-dod-recognized-set-mechanics:p1
pub(crate) fn member_op_subject(tenant_id: Uuid, kind: SetKind, member_code: &str) -> GateSubject {
    GateSubject::governed_live_op(
        tenant_id,
        &format!("recognized_set/{}/{member_code}", kind.as_str()),
        SubjectPin::Unpinned,
    )
}

/// The op declaration a door presents to the binding check (**P-D-172**), and
/// what a record must have been submitted for to authorize it.
///
/// # Explicit `null` is part of the proposal
///
/// The rendering runs under [`canonical::Absence::Omit`], which carries an
/// explicit `null` as `null` and drops nothing else — so *clear the label*
/// (`"display_label": null`) and *set it to "Gold tier"* are two different
/// declarations, and an approval for one does not authorize the other. That
/// is the mode's own reason for existing (**P-D-34**): an omitted field and
/// a cleared one are two different acts.
fn add_declaration(member_code: &str, display_label: Option<&str>) -> JsonValue {
    serde_json::json!({
        "op": MemberOp::Add.token(),
        "member_code": member_code,
        "display_label": display_label,
    })
}

/// [`add_declaration`] for the transitions door — the edge **and** the state
/// the submitter held, which is what makes the approval a proposal rather
/// than a standing permission.
fn transition_declaration(to: &str, expected_state: &str) -> JsonValue {
    serde_json::json!({
        "op": MemberOp::Transition.token(),
        "to": to,
        "expected_state": expected_state,
    })
}

/// [`add_declaration`] for the label door.
fn relabel_declaration(display_label: Option<&str>) -> JsonValue {
    serde_json::json!({
        "op": MemberOp::Relabel.token(),
        "display_label": display_label,
    })
}

/// Resolve the stored host for one member op before its transaction — the
/// taxonomy door's shape. A refusal is audited under the set's gate label
/// and answered `APPROVAL_REQUIRED`.
///
/// # The record must have been submitted for *this* change (**P-D-172**)
///
/// The subject is the member (`recognized_set/{set_kind}/{member_code}`,
/// **P-D-146**), so host matching alone answers *"is there a satisfied record
/// for `gold`"* and not *"did two principals agree to this change to
/// `gold`"*. Once matched, the record's `content_snapshot` — the op payload,
/// P-D-120 row 14 — is compared with `presented` under the same canonical
/// rendering `07`'s correction door uses (`skus::snapshot_matches`), and a
/// mismatch is `APPROVAL_REQUIRED`.
///
/// **A record whose snapshot declares no op of this slice's authorizes every
/// op, as it did before P-D-172.** That is the compatibility arm, not an
/// oversight: every live-op record written before this entry — `vhp-core`'s
/// e2e library sends `{"subject": …}` for all eight of its live-op subjects —
/// carries no declaration to compare against, and refusing them would break
/// every existing caller at the moment the binding landed. The arm closes
/// when the callers declare; it is named in P-D-172's *Owed* with the lines
/// that close it.
async fn authorize_member_op(
    state: &ApiState,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    kind: SetKind,
    member_code: &str,
    presented: &JsonValue,
) -> Result<GateAuthorization, CanonicalError> {
    let authorization = match crate::api::rest::authorize_live_op(
        state,
        scope,
        tenant_id,
        member_op_subject(tenant_id, kind, member_code),
    )
    .await
    {
        Ok(authorization) => authorization,
        Err(crate::api::rest::HostError::Refused(refusal)) => {
            return Err(refuse_set(
                state,
                scope,
                tenant_id,
                actor_ref,
                kind,
                member_code.to_owned(),
                refusal,
            )
            .await);
        }
        Err(crate::api::rest::HostError::Repo(error)) => {
            return Err(repo_error_to_canonical(&error));
        }
    };
    if let Err(refusal) =
        bound_to_the_change(state, scope, tenant_id, &authorization, presented).await?
    {
        return Err(refuse_set(
            state,
            scope,
            tenant_id,
            actor_ref,
            kind,
            member_code.to_owned(),
            refusal,
        )
        .await);
    }
    Ok(authorization)
}

/// Whether the matched record was submitted for `presented` (**P-D-172**).
///
/// The outer `Result` is the storage channel and the inner one the refusal
/// channel, `resolve_submission`'s shape: a driver failure is not a refusal.
///
/// The record is read **outside** the door's transaction, which is safe
/// because `content_snapshot` is written at submission and never re-derived
/// (`design/05` §4) — there is no window in which the bytes compared here
/// differ from the bytes the settle spends. The read has to happen before the
/// transaction anyway: the host itself runs there (P-D-144), and a refusal
/// that had to roll a transaction back would write an audit row the rollback
/// took with it.
async fn bound_to_the_change(
    state: &ApiState,
    scope: &AccessScope,
    tenant_id: Uuid,
    authorization: &GateAuthorization,
    presented: &JsonValue,
) -> Result<Result<(), DomainError>, CanonicalError> {
    let Some(approval_id) = authorization.approval_ref() else {
        // No record authorized the act at all — `NoRecord` is the
        // quorum-zero and pre-authorized arm, and there is nothing whose
        // content two principals agreed to.
        return Ok(Ok(()));
    };
    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&crate::infra::storage::RepoError::Db(e.to_string()))
    })?;
    let Some(record) = repo::read_approval(&conn, scope, tenant_id, approval_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?
    else {
        // The host matched it a moment ago; a record that has vanished since
        // is a store this gear wrote wrong, not a caller's refusal.
        return Err(repo_error_to_canonical(
            &crate::infra::storage::RepoError::Db(format!(
                "approval {approval_id} matched the host and then vanished"
            )),
        ));
    };
    if crate::api::rest::approvals::declared_member_op(&record.content_snapshot).is_none() {
        return Ok(Ok(()));
    }
    let agreed = serde_json::from_str::<JsonValue>(&record.content_snapshot)
        .map(|value| canonical::canonical_rendering(&value, canonical::Absence::Omit))
        .unwrap_or_default();
    if agreed == canonical::canonical_rendering(presented, canonical::Absence::Omit) {
        return Ok(Ok(()));
    }
    Ok(Err(DomainError::ApprovalRequired(format!(
        "approval {approval_id} was submitted for a different change to this member than the \
         one presented; submit this change for approval"
    ))))
}

/// Spend the authorization where the write commits (`inst-gv-one-shot`).
async fn settle_member_op(
    tx: &impl toolkit_db::secure::DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    authorization: &GateAuthorization,
    now: OffsetDateTime,
) -> Result<(), TxError> {
    repo::settle_authorization(tx, scope, tenant_id, authorization, now)
        .await
        .map(|_spent| ())
        .map_err(|error| match error {
            repo::SettleError::Refused(refusal) => TxError::Refused(refusal),
            repo::SettleError::Repo(error) => TxError::Repo(error),
        })
}

fn gate_for(kind: SetKind) -> (&'static str, authz_resolver_sdk::ResourceType) {
    match kind {
        SetKind::PlanTier => (
            crate::authz::labels::PLAN_TIER,
            crate::authz::resource_types::PLAN_TIER,
        ),
        SetKind::MeteringUnit => (
            crate::authz::labels::RECOGNIZED_SET,
            crate::authz::resource_types::RECOGNIZED_SET,
        ),
    }
}

/// The event token `set_kind`'s mutations emit (`design/03` §4's roster).
const fn event_token_for(kind: SetKind) -> &'static str {
    match kind {
        SetKind::MeteringUnit => events::RECOGNIZED_UNIT_UPDATED_PAYLOAD_TYPE,
        SetKind::PlanTier => events::PLAN_TIER_UPDATED_PAYLOAD_TYPE,
    }
}

/// Parse the path's `setKind`, refusing anything outside the two-kind
/// roster — fail-closed, like every roster parse in the gear.
fn parse_kind(raw: &str) -> Result<SetKind, CanonicalError> {
    SetKind::parse(raw).ok_or_else(|| {
        let mut report = ValidationReport::new();
        report.violate(
            "VALIDATION",
            "setKind",
            "setKind must be one of metering_unit, plan_tier",
        );
        CanonicalError::from(DomainError::Validation(report))
    })
}

/// Audit one refusal and answer it — `refuse_reference`'s twin, with the
/// label chosen by the kind whose grant the call spent.
async fn refuse_set(
    state: &ApiState,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    kind: SetKind,
    subject: String,
    refusal: DomainError,
) -> CanonicalError {
    let (label, _) = gate_for(kind);
    let code = refusal.code();
    crate::api::rest::audit_refusal_and_report(
        state,
        scope,
        crate::api::rest::RefusalAuditContext {
            tenant_id,
            actor_ref,
            subject_kind: label,
            error_code: code,
        },
        RefusalSubject::Attempted(subject),
        CanonicalError::from(refusal),
    )
    .await
}

/// Compile the scope the kind's **read** grant demands.
///
/// Unlike [`set_scope`] this audits nothing on a denial: a refused read wrote
/// nothing and attempted nothing, and the gear's audit trail is a record of
/// acts. The write helper audits because a refused write is an attempt on a
/// member, which an operator reviewing the trail has to be able to see.
async fn read_scope(
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    tenant_id: Uuid,
    kind: SetKind,
) -> Result<AccessScope, CanonicalError> {
    let (_, resource) = gate_for(kind);
    crate::authz::access_scope(
        enforcer,
        ctx,
        &resource,
        crate::authz::actions::READ,
        Some(tenant_id),
        None,
        true,
    )
    .await
    .map_err(|e| {
        crate::api::rest::authz_error_to_canonical(e, |reason| {
            RecognizedSetResource::permission_denied()
                .with_reason(reason)
                .create()
        })
    })
}

/// Compile the scope the kind's grant demands, auditing a denial.
async fn set_scope(
    state: &ApiState,
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    tenant_id: Uuid,
    actor_ref: Uuid,
    kind: SetKind,
    subject: String,
) -> Result<AccessScope, CanonicalError> {
    let (label, resource) = gate_for(kind);
    match crate::authz::access_scope(
        enforcer,
        ctx,
        &resource,
        crate::authz::actions::WRITE,
        Some(tenant_id),
        None,
        true,
    )
    .await
    {
        Ok(scope) => Ok(scope),
        Err(crate::authz::AuthzError::Denied(reason)) => {
            let self_scope = AccessScope::for_tenant(tenant_id);
            Err(crate::api::rest::audit_refusal_and_report(
                state,
                &self_scope,
                crate::api::rest::RefusalAuditContext {
                    tenant_id,
                    actor_ref,
                    subject_kind: label,
                    error_code: "PERMISSION_DENIED",
                },
                RefusalSubject::Attempted(subject),
                RecognizedSetResource::permission_denied()
                    .with_reason(reason)
                    .create(),
            )
            .await)
        }
        Err(err @ crate::authz::AuthzError::Unavailable(_)) => {
            Err(crate::api::rest::authz_error_to_canonical(err, |reason| {
                RecognizedSetResource::permission_denied()
                    .with_reason(reason)
                    .create()
            }))
        }
    }
}

/// Every door's registration.
pub(crate) fn router(state: Arc<ApiState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = Router::new();

    // The reads first, because they are what a caller meets first: a set has
    // to be enumerable before a member can be chosen or judged. Both gate on
    // the kind's `read` grant (P-D-170), and **neither answers an `ETag`** —
    // no door on this surface accepts `If-Match`, staleness being pinned by
    // the transition body's `expected_state`, so a tag would be a header with
    // nobody to assert it. It arrives with the per-value `PATCH`, not before.
    let router = OperationBuilder::get("/bss-products/v1/recognized-sets/{setKind}")
        .operation_id("bss_products.list_recognized_members")
        .summary("List a recognized set's members")
        .description(
            "Returns every member of the named set - `metering_unit` or `plan_tier` - in \
             `memberCode` order, each with its state and, for the tier set, its display \
             label. Gates on the kind's `read` grant: `plan_tier x read` for the tier set, \
             `recognized_set x read` for the unit set (P-D-90 arm 2's split, applied to the \
             read half). **Tombstones are included**, carrying `state: removed` - the add \
             door refuses a removed code `DUPLICATE_CODE` because its primary key never \
             frees, so a list that hid them would contradict the door a caller meets next; a \
             picker renders `active` and nothing else. The tenant's platform baseline is \
             seeded on this read if the set is still empty (P-D-104), so a set is never \
             answered emptier than the next write would find it. No paging: these are closed \
             vocabularies of tens.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("setKind", "Which recognized set to list.")
        .handler(list_members)
        .json_response_with_schema::<RecognizedSetView>(
            openapi,
            StatusCode::OK,
            "The set's members.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    let router = OperationBuilder::get(
        "/bss-products/v1/recognized-sets/{setKind}/members/{memberCode}",
    )
    .operation_id("bss_products.get_recognized_member")
    .summary("Read one member of a recognized set")
    .description(
        "Returns the member named by `memberCode`: its state, its `seededBy` provenance and, \
         for the tier set, its display label. Gates on the kind's `read` grant. A member \
         outside the caller's authorized scope reads exactly like an absent one (`404`, no \
         existence leak) - the same discipline every other read on this gear keeps.",
    )
    .tag(TAG)
    .authenticated()
    .no_license_required()
    .path_param("setKind", "Which recognized set the member belongs to.")
    .path_param("memberCode", "The member to read.")
    .handler(get_member)
    .json_response_with_schema::<RecognizedMemberView>(openapi, StatusCode::OK, "The member.")
    .error_400(openapi)
    .error_401(openapi)
    .error_403(openapi)
    .error_404(openapi)
    .error_500(openapi)
    .error_503(openapi)
    .register(router, openapi);

    let router = OperationBuilder::post("/bss-products/v1/recognized-sets/{setKind}/members")
        .operation_id("bss_products.add_recognized_member")
        .summary("Add a member to a recognized set")
        .description(
            "Adds an `active` member to the named set - `metering_unit` or `plan_tier` - and \
             enqueues the set's event in the same transaction. The grant is chosen by `setKind` \
             (P-D-90): the tier set spends `plan_tier x write`, the unit set \
             `recognized_set x write`. A code the set already carries \
             in any state is refused `DUPLICATE_CODE` - a removed member is a tombstone whose \
             primary key never frees, and the path back into the set is the transitions door's \
             re-listing, never a second add. There is no rename and no delete on any member, \
             in any state: a correction is a new member plus a deprecation of the old.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("setKind", "Which recognized set to add to.")
        .json_request::<AddMemberRequest>(openapi, "The member to add.")
        .handler(add_member)
        .json_response_with_schema::<RecognizedMemberView>(
            openapi,
            StatusCode::CREATED,
            "The member, active, as stored.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    let router = OperationBuilder::post(
        "/bss-products/v1/recognized-sets/{setKind}/members/{memberCode}/transitions",
    )
    .operation_id("bss_products.transition_recognized_member")
    .summary("Walk a recognized-set member's state machine")
    .description(
        "Applies one admitted edge to the member - `active -> deprecated`, `deprecated -> \
         removed`, or the re-listing edges `deprecated -> active` and `removed -> active`. \
         `active -> removed` is refused: de-listing deprecates first, so new declarations \
         stop before the member can leave the set. The body pins the state the caller read \
         (`expected_state`); a peer's flip in between is refused `STALE_LIVE_OP`. A removal \
         is refused while any non-terminal published head still references the member \
         (`UNIT_DELIST_BLOCKED` / `PLAN_TIER_RETIRE_BLOCKED`, holders sampled), and never \
         touches a seeded \
         member. The write and the set's event commit in one transaction.",
    )
    .tag(TAG)
    .authenticated()
    .no_license_required()
    .path_param("setKind", "Which recognized set the member belongs to.")
    .path_param("memberCode", "The member to transition.")
    .json_request::<MemberTransitionRequest>(
        openapi,
        "The edge to apply and the state the caller read.",
    )
    .handler(transition_member)
    .json_response_with_schema::<RecognizedMemberView>(
        openapi,
        StatusCode::OK,
        "The member, in its new state.",
    )
    .error_400(openapi)
    .error_401(openapi)
    .error_403(openapi)
    .error_404(openapi)
    .error_409(openapi)
    .error_500(openapi)
    .error_503(openapi)
    .register(router, openapi);
    let router = OperationBuilder::post(
        "/bss-products/v1/recognized-sets/{setKind}/members/{memberCode}/label",
    )
    .operation_id("bss_products.relabel_recognized_member")
    .summary("Change a recognized-set member's display label")
    .description(
        "Sets the member's `display_label` and nothing else - the rename a tier or unit \
         admits. The member's code is its identity and has no update path (the table's \
         trigger refuses one), so every SKU declaring the code is untouched. Rides \
         `GovernedLiveOp` under the stored approval host like the other two member ops, and \
         enqueues the set's event in the same transaction.",
    )
    .tag(TAG)
    .authenticated()
    .no_license_required()
    .path_param("setKind", "Which recognized set the member belongs to.")
    .path_param("memberCode", "The member to relabel.")
    .json_request::<MemberRelabelRequest>(openapi, "The new display label, or null to clear it.")
    .handler(relabel_member)
    .json_response_with_schema::<RecognizedMemberView>(
        openapi,
        StatusCode::OK,
        "The member, relabelled.",
    )
    .error_400(openapi)
    .error_401(openapi)
    .error_403(openapi)
    .error_404(openapi)
    .error_500(openapi)
    .error_503(openapi)
    .register(router, openapi);

    router.layer(Extension(state))
}

/// `GET /bss-products/v1/recognized-sets/{setKind}`.
///
/// The set an operator has to see before they can choose from it. Until this
/// door landed the gear could be written to and never enumerated: the
/// repository's only read was a single-member lookup the validators used, so
/// neither a picker nor a consumer could learn which codes exist (P-D-170).
async fn list_members(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(set_kind): Path<String>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let kind = parse_kind(&set_kind)?;
    let tenant_id = ctx.subject_tenant_id();
    let scope = read_scope(&enforcer, &ctx, tenant_id, kind).await?;

    let conn = state
        .db
        .conn()
        .map_err(|e| CanonicalError::internal(format!("bss-products: db conn: {e}")).create())?;

    // Seed before reading, exactly as the write doors do (P-D-104): a tenant
    // that has never written to this set still holds the platform baseline,
    // and a picker opened before the first write must not show an empty ladder
    // that the create door would then fill behind it.
    let now = canonical::write_instant(OffsetDateTime::now_utc());
    repo::ensure_recognized_seeds(&conn, &scope, tenant_id, kind, now)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;

    let members = repo::recognized_members(&conn, &scope, tenant_id, kind)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;

    Ok(Json(RecognizedSetView {
        set_kind: kind.as_str().to_owned(),
        members: members
            .into_iter()
            .map(|m| RecognizedMemberView::from_member(kind, m))
            .collect(),
    })
    .into_response())
}

/// `GET /bss-products/v1/recognized-sets/{setKind}/members/{memberCode}`.
///
/// A miss is a bare `404` carrying no registry code, the shape every other
/// read on this surface answers with: absent and out-of-scope must be
/// indistinguishable, or the door tells a caller the PDP did not grant which
/// codes the tenant holds.
async fn get_member(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path((set_kind, member_code)): Path<(String, String)>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let kind = parse_kind(&set_kind)?;
    let tenant_id = ctx.subject_tenant_id();
    let scope = read_scope(&enforcer, &ctx, tenant_id, kind).await?;

    let conn = state
        .db
        .conn()
        .map_err(|e| CanonicalError::internal(format!("bss-products: db conn: {e}")).create())?;

    let member = repo::recognized_member(&conn, &scope, tenant_id, kind, member_code.trim())
        .await
        .map_err(|e| repo_error_to_canonical(&e))?
        .ok_or_else(|| {
            RecognizedSetResource::not_found("no member matches this code in the caller's scope")
                .with_resource(format!("{}/{member_code}", kind.as_str()))
                .create()
        })?;

    Ok(Json(RecognizedMemberView::from_member(kind, member)).into_response())
}

/// `POST /recognized-sets/{setKind}/members/{memberCode}/label`.
///
/// @cpt-dod:cpt-cf-bss-products-dod-plantier-governance:p1
async fn relabel_member(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path((set_kind, member_code)): Path<(String, String)>,
    Json(body): Json<MemberRelabelRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let kind = parse_kind(&set_kind)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(OffsetDateTime::now_utc());
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let scope = set_scope(
        &state,
        &enforcer,
        &ctx,
        tenant_id,
        actor_ref,
        kind,
        member_code.clone(),
    )
    .await?;
    let authorization = authorize_member_op(
        &state,
        &scope,
        tenant_id,
        actor_ref,
        kind,
        &member_code,
        &relabel_declaration(body.display_label.as_deref()),
    )
    .await?;

    let outbox = state.sink.clone();
    let scope_tx = scope.clone();
    let code_tx = member_code.clone();
    let label = body.display_label.clone();
    let authorization_tx = authorization.clone();
    let result = state
        .db
        .db()
        .transaction_with_retry::<repo::RecognizedMember, TxError, _, _>(
            toolkit_db::secure::TxConfig::default(),
            member_contention_db_err,
            move |tx| {
                let outbox = outbox.clone();
                let scope = scope_tx.clone();
                let member_code = code_tx.clone();
                let display_label = label.clone();
                let authorization = authorization_tx.clone();
                Box::pin(async move {
                    settle_member_op(tx, &scope, tenant_id, &authorization, now).await?;
                    let relabelled = repo::relabel_recognized_member(
                        tx,
                        &scope,
                        tenant_id,
                        kind,
                        &member_code,
                        display_label,
                        now,
                    )
                    .await
                    .map_err(TxError::Repo)?;
                    if !relabelled {
                        return Err(TxError::NotFound);
                    }
                    let after = repo::recognized_member(tx, &scope, tenant_id, kind, &member_code)
                        .await
                        .map_err(TxError::Repo)?
                        .ok_or_else(|| {
                            TxError::Repo(crate::infra::storage::RepoError::Db(format!(
                                "recognized member {member_code} vanished under its own relabel"
                            )))
                        })?;
                    events::enqueue_set_event(
                        &outbox,
                        tx,
                        event_token_for(kind),
                        events::SetEventBody {
                            tenant_id,
                            set_kind: kind.as_str(),
                            member_code: &member_code,
                            state: after.state.as_str(),
                        },
                        actor_ref,
                    )
                    .await
                    .map_err(|e| {
                        TxError::Repo(crate::infra::storage::RepoError::Db(e.to_string()))
                    })?;
                    Ok(after)
                })
            },
        )
        .await;

    match result {
        Ok(member) => Ok((
            StatusCode::OK,
            Json(RecognizedMemberView::from_member(kind, member)),
        )
            .into_response()),
        Err(TxError::NotFound) => Err(member_not_found(kind, &member_code)),
        Err(TxError::Refused(refusal)) => Err(refuse_set(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            kind,
            member_code,
            refusal,
        )
        .await),
        Err(TxError::Repo(e)) => Err(repo_error_to_canonical(&e)),
    }
}

/// `POST /recognized-sets/{setKind}/members`.
async fn add_member(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(set_kind): Path<String>,
    Json(body): Json<AddMemberRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let kind = parse_kind(&set_kind)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(OffsetDateTime::now_utc());
    let member_code = body.member_code.trim().to_owned();
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let scope = set_scope(
        &state,
        &enforcer,
        &ctx,
        tenant_id,
        actor_ref,
        kind,
        member_code.clone(),
    )
    .await?;

    if member_code.is_empty() {
        let mut report = ValidationReport::new();
        report.violate("VALIDATION", "member_code", "member_code must not be blank");
        return Err(refuse_set(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            kind,
            member_code,
            DomainError::Validation(report),
        )
        .await);
    }

    let authorization = authorize_member_op(
        &state,
        &scope,
        tenant_id,
        actor_ref,
        kind,
        &member_code,
        &add_declaration(&member_code, body.display_label.as_deref()),
    )
    .await?;
    let outbox = state.sink.clone();
    let scope_tx = scope.clone();
    let code_tx = member_code.clone();
    let label = body.display_label.clone();
    let authorization_tx = authorization.clone();
    let result = state
        .db
        .db()
        .transaction_with_retry::<repo::RecognizedMember, TxError, _, _>(
            toolkit_db::secure::TxConfig::default(),
            member_contention_db_err,
            move |tx| {
                let outbox = outbox.clone();
                let scope = scope_tx.clone();
                let member_code = code_tx.clone();
                let display_label = label.clone();
                let authorization = authorization_tx.clone();
                Box::pin(async move {
                    settle_member_op(tx, &scope, tenant_id, &authorization, now).await?;
                    if let Some(standing) =
                        repo::recognized_member(tx, &scope, tenant_id, kind, &member_code)
                            .await
                            .map_err(TxError::Repo)?
                    {
                        return Err(TxError::Refused(DomainError::DuplicateCode(format!(
                            "the {} set already carries `{member_code}` in state `{}`: a removed \
                         member re-enters through the transitions door's re-listing, never a \
                         second add",
                            kind.as_str(),
                            standing.state.as_str()
                        ))));
                    }
                    let stored = repo::insert_recognized_member(
                        tx,
                        &scope,
                        tenant_id,
                        kind,
                        &member_code,
                        display_label,
                        None,
                        now,
                    )
                    .await
                    .map_err(|e| classify_member_insert(&member_code, kind, e))?;
                    events::enqueue_set_event(
                        &outbox,
                        tx,
                        event_token_for(kind),
                        events::SetEventBody {
                            tenant_id,
                            set_kind: kind.as_str(),
                            member_code: &member_code,
                            state: stored.state.as_str(),
                        },
                        actor_ref,
                    )
                    .await
                    .map_err(|e| {
                        TxError::Repo(crate::infra::storage::RepoError::Db(e.to_string()))
                    })?;
                    Ok(stored)
                })
            },
        )
        .await;

    match result {
        Ok(member) => Ok((
            StatusCode::CREATED,
            Json(RecognizedMemberView::from_member(kind, member)),
        )
            .into_response()),
        Err(TxError::Refused(refusal)) => Err(refuse_set(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            kind,
            member_code,
            refusal,
        )
        .await),
        Err(TxError::Repo(e)) => Err(repo_error_to_canonical(&e)),
        // The add path never reads a member it requires to exist.
        Err(TxError::NotFound) => Err(repo_error_to_canonical(
            &crate::infra::storage::RepoError::Db(
                "the add door raised NotFound, which no branch of it constructs".to_owned(),
            ),
        )),
    }
}

/// `POST /recognized-sets/{setKind}/members/{memberCode}/transitions`.
async fn transition_member(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path((set_kind, member_code)): Path<(String, String)>,
    Json(body): Json<MemberTransitionRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let kind = parse_kind(&set_kind)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(OffsetDateTime::now_utc());
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let scope = set_scope(
        &state,
        &enforcer,
        &ctx,
        tenant_id,
        actor_ref,
        kind,
        member_code.clone(),
    )
    .await?;

    let (Some(to), Some(expected)) = (
        MemberState::parse(body.to.trim()),
        MemberState::parse(body.expected_state.trim()),
    ) else {
        let mut report = ValidationReport::new();
        report.violate(
            "VALIDATION",
            "to",
            "to and expected_state must each be one of active, deprecated, removed",
        );
        return Err(refuse_set(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            kind,
            member_code,
            DomainError::Validation(report),
        )
        .await);
    };

    let authorization = authorize_member_op(
        &state,
        &scope,
        tenant_id,
        actor_ref,
        kind,
        &member_code,
        &transition_declaration(to.as_str(), expected.as_str()),
    )
    .await?;
    let outbox = state.sink.clone();
    let scope_tx = scope.clone();
    let code_tx = member_code.clone();
    let authorization_tx = authorization.clone();
    let result = state
        .db
        .db()
        .transaction_with_retry::<repo::RecognizedMember, TxError, _, _>(
            toolkit_db::secure::TxConfig::default(),
            member_contention_db_err,
            move |tx| {
                let outbox = outbox.clone();
                let scope = scope_tx.clone();
                let member_code = code_tx.clone();
                let authorization = authorization_tx.clone();
                Box::pin(async move {
                    settle_member_op(tx, &scope, tenant_id, &authorization, now).await?;
                    let Some(stored) =
                        repo::recognized_member(tx, &scope, tenant_id, kind, &member_code)
                            .await
                            .map_err(TxError::Repo)?
                    else {
                        return Err(TxError::NotFound);
                    };

                    // The live-op staleness pin, then the machine's own edge —
                    // in that order, so a stale caller is told the world moved
                    // rather than that its (stale) edge is illegal.
                    if stored.state != expected {
                        return Err(TxError::Refused(DomainError::StaleLiveOp(format!(
                            "the {} member `{member_code}` is `{}`, not the expected `{}`",
                            kind.as_str(),
                            stored.state.as_str(),
                            expected.as_str()
                        ))));
                    }
                    member_edge(stored.state, to).map_err(TxError::Refused)?;

                    if to == MemberState::Removed {
                        if let Some(seeder) = stored.seeded_by.as_deref() {
                            // NOT one of the three delist codes: §7 row 18
                            // asks which code refuses the removal of a
                            // seeded, unreferenced member and answers itself
                            // that "all three de-list codes are predicated on
                            // holders, so none fits". Picking one anyway would
                            // decide that row from the crate and hand
                            // consumers a wire contract its owner has not
                            // agreed. The generic validation channel carries
                            // the refusal until they do.
                            // P-D-131 row 18: the Foundation's variant, uniformly with
                            // 02's seeded definition — no sixteenth code.
                            return Err(TxError::Refused(DomainError::IllegalFieldMutation(
                                format!(
                                    "`{member_code}` is a seeded member (seeded by {seeder}): \
                                     seeded members are deprecatable and never removed"
                                ),
                            )));
                        }
                        // The removal operand, `inst-us-delist`'s exactly: only
                        // the metering-unit set has a shipped carrier column, so
                        // only it can have holders today — the other kinds'
                        // populations are empty by construction until their
                        // columns land.
                        // One read answers both halves: a count and a
                        // sample taken separately could disagree, and
                        // the message would name a total with no
                        // exemplar. Over the bound the count is
                        // honest about being a floor.
                        refuse_delist_if_held(tx, &scope, tenant_id, kind, &member_code).await?;
                    }

                    let flipped = repo::flip_recognized_member(
                        tx,
                        &scope,
                        tenant_id,
                        kind,
                        &member_code,
                        repo::StateFlip {
                            expected: stored.state,
                            to,
                        },
                        now,
                    )
                    .await
                    .map_err(TxError::Repo)?;
                    if !flipped {
                        // The UPDATE re-asserts the census (P-D-121 row 21).
                        // A concurrent first publish lands a holder and the
                        // flip matches nothing — that is UNIT_DELIST_BLOCKED,
                        // not a stale pin.
                        if to == MemberState::Removed {
                            refuse_delist_if_held(tx, &scope, tenant_id, kind, &member_code)
                                .await?;
                        }
                        return Err(TxError::Refused(DomainError::StaleLiveOp(format!(
                            "the {} member `{member_code}` moved between the read and the write",
                            kind.as_str()
                        ))));
                    }
                    events::enqueue_set_event(
                        &outbox,
                        tx,
                        event_token_for(kind),
                        events::SetEventBody {
                            tenant_id,
                            set_kind: kind.as_str(),
                            member_code: &member_code,
                            state: to.as_str(),
                        },
                        actor_ref,
                    )
                    .await
                    .map_err(|e| {
                        TxError::Repo(crate::infra::storage::RepoError::Db(e.to_string()))
                    })?;

                    let after = repo::recognized_member(tx, &scope, tenant_id, kind, &member_code)
                        .await
                        .map_err(TxError::Repo)?
                        .ok_or_else(|| {
                            TxError::Repo(crate::infra::storage::RepoError::Db(format!(
                                "recognized member {member_code} vanished under its own flip"
                            )))
                        })?;
                    Ok(after)
                })
            },
        )
        .await;

    match result {
        Ok(member) => Ok((
            StatusCode::OK,
            Json(RecognizedMemberView::from_member(kind, member)),
        )
            .into_response()),
        Err(TxError::NotFound) => Err(member_not_found(kind, &member_code)),
        Err(TxError::Refused(refusal)) => Err(refuse_set(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            kind,
            member_code,
            refusal,
        )
        .await),
        Err(TxError::Repo(e)) => Err(repo_error_to_canonical(&e)),
    }
}

/// The PK conflict two concurrent adds of one code produce, classified the
/// way the create doors classify theirs.
///
/// The pre-read inside the transaction closes only the sequential case: the
/// arbiter is `(tenant_id, set_kind, member_code)`, and a racer that
/// committed between that read and this insert reaches the driver. Left
/// unclassified it answered a `500` for an ordinary race the winner just
/// made true — while the door's own description promises `DUPLICATE_CODE`
/// in any state, the tombstone included, since the key never frees.
fn classify_member_insert(
    member_code: &str,
    kind: SetKind,
    error: crate::infra::storage::RepoError,
) -> TxError {
    let message = error.to_string().to_ascii_lowercase();
    if message.contains("unique constraint")
        || message.contains("duplicate key")
        || message.contains("primary key")
    {
        return TxError::Refused(DomainError::DuplicateCode(format!(
            "the {} set already carries `{member_code}`: a peer added it between this door's \
             read and its insert, and a removed member re-enters through the transitions door's \
             re-listing, never a second add",
            kind.as_str()
        )));
    }
    TxError::Repo(error)
}

/// The retryable-contention extractor both transactions pass, mirroring
/// `products::head_act_contention_db_err`.
///
/// Only [`TxError::Repo`] can carry a driver error, and only a driver error
/// can be contention: a refusal is a business answer and `NotFound` is a
/// read. Without this both doors passed `None` unconditionally, so a
/// `database is locked` on the interim engine — which every sibling door
/// retries — answered `500`.
fn member_contention_db_err(error: &TxError) -> Option<&sea_orm::DbErr> {
    match error {
        // `RepoError::Driver` carries `sea-orm`'s own error, which is what the
        // retry loop classifies — the sibling doors reach it through
        // `DbError::Sea`, and this one already holds the inner value.
        TxError::Repo(crate::infra::storage::RepoError::Driver { source, .. }) => Some(source),
        TxError::Repo(_) | TxError::Refused(_) | TxError::NotFound => None,
    }
}

/// Refuse a metering-unit removal while a non-terminal published head
/// still declares it. Other kinds have no carrier column yet, so their
/// holder population is empty by construction.
async fn refuse_delist_if_held(
    runner: &impl toolkit_db::secure::DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    kind: SetKind,
    member_code: &str,
) -> Result<(), TxError> {
    const SAMPLE: usize = 5;
    let holders = repo::member_holders(runner, scope, tenant_id, kind, member_code, SAMPLE as u64)
        .await
        .map_err(TxError::Repo)?;
    if holders.is_empty() {
        return Ok(());
    }
    let shown = holders.len().min(SAMPLE);
    let count = if holders.len() > SAMPLE {
        format!("at least {}", SAMPLE + 1)
    } else {
        holders.len().to_string()
    };
    Err(TxError::Refused(kind.delist_blocked(format!(
        "{count} non-terminal published head(s) still declare `{member_code}` ({}): \
         deprecate first, remove once unreferenced",
        holders[..shown].join(", ")
    ))))
}

/// The transactions' error channel: a business refusal (audited outside the
/// transaction, after rollback), a repository failure, or a member the set
/// never carried.
enum TxError {
    Refused(DomainError),
    Repo(crate::infra::storage::RepoError),
    NotFound,
}

impl From<toolkit_db::DbError> for TxError {
    fn from(error: toolkit_db::DbError) -> Self {
        Self::Repo(crate::infra::storage::RepoError::Db(error.to_string()))
    }
}

/// The bare 404 for a member the set never carried — no code channel, the
/// products read door's own shape.
fn member_not_found(kind: SetKind, member_code: &str) -> CanonicalError {
    RecognizedSetResource::not_found("no member matches this code in this set")
        .with_resource(format!("{}/{member_code}", kind.as_str()))
        .create()
}

#[cfg(test)]
#[path = "recognized_sets_tests.rs"]
mod recognized_sets_tests;
