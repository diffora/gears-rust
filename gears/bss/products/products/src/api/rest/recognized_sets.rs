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
//! statement answers `STALE_LIVE_OP`, never a silent absorb.
//!
//! # And the two per-member doors pin the whole row (**P-D-174**)
//!
//! `GET …/members/{memberCode}` answers an `ETag` — the `SHA-256` of the
//! member's canonical rendering, a live row having no revision to name — and
//! the transitions and label doors **require** it back as `If-Match`,
//! compared under the write inside their own transaction. It is the stronger
//! pin: `expected_state` covers one column, and the label door had **no** pin
//! at all, so two operators renaming one member raced and the later write won
//! silently. Both are kept on the transitions door because they answer
//! different obligations — the tag is the caller's read-currency, and
//! `expected_state` is what two principals agreed to in the approval unit,
//! which cannot be re-derived at apply time. A stale tag is `STALE_LIVE_OP`:
//! the code roster is closed and a live row's staleness has one voice. The
//! **add** door takes no `If-Match` — a create names no member to have read —
//! and the **list** read answers no `ETag`, for P-D-170's own reason that a
//! tag nobody can assert is a header with no reader. The membership
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
    if let Err(refusal) = bound_to_the_change(state, tenant_id, &authorization, presented).await? {
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

/// The scope this gear consults **its own governance state** under.
///
/// **Not the door's.** `products_approval` declares `resource_col =
/// "approval_id"`, and this door's scope is compiled for `plan_tier` /
/// `recognized_set` — so a real PDP's scope filters that table by a column it
/// never constrained and matches nothing. `repo::gate_candidates` records the
/// measurement that found it (the benidorm stand, 2026-09-06: *"every
/// governed act answered `APPROVAL_REQUIRED` while its satisfied record sat
/// in the table"*) and takes exactly this posture, ignoring the door scope it
/// is handed. The reads and the submission below are the same access — the
/// caller has already passed this door's own authz, and what follows is the
/// gear consulting its ceremony inside that tenant.
fn governance_scope(tenant_id: Uuid) -> AccessScope {
    AccessScope::for_tenant(tenant_id)
}

/// The refusal a stale member tag earns (**P-D-174**).
///
/// **`STALE_LIVE_OP`, not a sixteenth code and not `01`'s
/// `STALE_REVISION`.** `02` §3.5 draws the line this sits on: a live entity's
/// staleness is its own code because a live row has no revision to be stale
/// at, and `STALE_REVISION` names one. The roster is closed and pinned in two
/// counters (`ErrorCode::ALL`, `DOMAIN_ERROR_VARIANTS`); minting a third
/// staleness code for a stronger operand on the same subject would say the
/// world moved in a new voice.
fn stale_member_tag(kind: SetKind, member_code: &str) -> DomainError {
    DomainError::StaleLiveOp(format!(
        "the {} member `{member_code}` has changed since the `ETag` this call asserts; re-read \
         `GET .../members/{member_code}` and send its tag",
        kind.as_str()
    ))
}

/// The entity tag one member's `GET` hands out and its two write doors assert
/// (**P-D-174**).
///
/// # A digest, because a member has no revision
///
/// `01`'s heads tag their `internal_revision` and `domain::concurrency`
/// parses the tag back into one. A recognized-set member is a **live row** —
/// `domain::live_op`'s own words, the reason `GovernedLiveOp` pins a state
/// rather than a revision — and has no counter to name. So the tag is the
/// `SHA-256` of the member's canonical rendering, which is the gear's own
/// rendering primitive (`inst-fd-canonical`, P-D-34) and is what makes two
/// equal members tag equally on both engines. `seeded_by` is in the rendering
/// as well as the three wire fields: it decides whether a removal is
/// admissible (`inst-rs-seeded`), so a caller who read a member before it was
/// adopted by the platform baseline has not read the member the write acts
/// on.
fn member_etag(member: &repo::RecognizedMember) -> String {
    let rendering = canonical::canonical_rendering(
        &serde_json::json!({
            "member_code": member.member_code,
            "display_label": member.display_label,
            "state": member.state.as_str(),
            "seeded_by": member.seeded_by,
        }),
        canonical::Absence::Omit,
    );
    let digest = canonical::content_digest(&rendering);
    let mut hex = String::with_capacity(digest.len() * 2 + 2);
    hex.push('"');
    for byte in digest {
        // Two lowercase hex digits per byte, built by hand rather than
        // through `write!`: the formatter's `Result` on a `String` cannot
        // fail, and discarding it is what `-D warnings` refuses.
        hex.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
        hex.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
    }
    hex.push('"');
    hex
}

/// The member tag an `If-Match` on a per-member write door asserts
/// (**P-D-174**).
///
/// The four syntactic refusals are `domain::concurrency::strong_tag_body`'s —
/// one contract for every tagged subject in the gear — and what this adds is
/// the body's own shape: 64 lowercase hex digits, because a tag this door
/// cannot have minted names no member it could compare against.
///
/// # Errors
///
/// [`DomainError::Validation`] naming `If-Match` when the header is absent,
/// is not valid UTF-8, or is not one strong tag quoting a `SHA-256` digest.
fn member_if_match(headers: &axum::http::HeaderMap) -> Result<String, DomainError> {
    let refuse = |detail: &str| {
        let mut report = ValidationReport::new();
        report.violate("VALIDATION", "If-Match", detail);
        DomainError::Validation(report)
    };
    let Some(raw) = headers.get(axum::http::header::IF_MATCH) else {
        return Err(refuse(
            "If-Match is required on this verb: a member op asserts the member it was authored \
             against, and an unconditional write would overwrite a concurrent editor's. Read the \
             `ETag` off `GET .../members/{memberCode}` and send it back verbatim",
        ));
    };
    let raw = raw
        .to_str()
        .map_err(|_| refuse("If-Match: the header value is not valid UTF-8"))?;
    let body = crate::domain::concurrency::strong_tag_body(raw)?;
    if body.len() == 64
        && body
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Ok(format!("\"{body}\""));
    }
    Err(refuse(
        "If-Match: a member's tag quotes the 64 lowercase hex digits of its canonical rendering's \
         SHA-256; this is not one this gear could have minted",
    ))
}

/// What the approval unit a member op rides looks like on the `202` arm
/// (**P-D-173**) — `SubmitApprovalReceipt`'s fields, so an operator's inbox
/// card renders a unit this door opened exactly as one the submit door did.
#[toolkit_macros::api_dto(response)]
pub struct MemberOpApprovalView {
    /// The unit the decide door addresses.
    pub approval_id: Uuid,
    /// `pending` — a unit born `satisfied` never reaches this arm, because
    /// the door proceeds with it in the same request.
    pub state: String,
    /// The **effective** count, never the raw configured `N`.
    pub required: u32,
    /// The raw `N` in force when the unit opened.
    pub configured_quorum: u32,
    /// Whether the finance lens is demanded of the approver set.
    pub finance_required: bool,
    /// Whether the effective count sits below the retained default of two.
    pub quorum_reduced: bool,
}

/// What the gate answered for one member op (**P-D-173**).
enum MemberOpGate {
    /// A record stands for this exact change: the door proceeds and spends
    /// it inside its own transaction.
    Authorized(GateAuthorization),
    /// No record stood, so the door opened one. Nothing was written to the
    /// member; the caller re-sends the identical request once the unit is
    /// approved.
    Opened(MemberOpApprovalView),
}

/// Resolve — or open — the approval one member op rides (**P-D-173**).
///
/// # The door was the only one on this surface that could not be used
///
/// Before this entry the three write doors answered `APPROVAL_REQUIRED` and
/// nothing else when no satisfied record stood, so **the only way to use
/// them was to submit through `POST /approvals` first**, with a
/// `content_snapshot` the caller hand-rendered to match what the door would
/// present (**P-D-172**). That is a contract no client can hold: the bytes
/// are the door's, and a caller who renders them differently is refused for
/// a reason they cannot see. Pricing solved the same problem with a two-arm
/// door (`api/rest/taxonomies.rs`' value `PATCH`, **D-355**), and this is
/// that shape: the first call opens the unit and answers `202`, the re-send
/// after approval answers the door's ordinary `200`/`201`.
///
/// # It never supersedes a unit somebody is already deciding
///
/// `design/05` §4's partial `UNIQUE` admits **one** open record per subject,
/// and `repo::submit_approval` supersedes whatever open record the subject
/// held (L-4). A door that submitted on every unauthorized call would
/// therefore discard the approvals already collected on a pending unit — two
/// principals' work, silently, on a caller's retry. So the unit is opened
/// **only** when the subject holds no open record at all; a standing unit
/// for this same change is answered `202` again (the idempotent re-send
/// before approval), and a standing unit for a *different* change keeps
/// P-D-172's refusal, which names it rather than replacing it.
///
/// # The pre-existing path is unchanged, which is why this is additive
///
/// A caller that submits through `POST /approvals` and then calls the door
/// takes the `Authorized` arm exactly as it did — that is the path
/// `vhp-core`'s e2e drives for all three doors, and no case of it reaches
/// the new arm.
#[allow(clippy::too_many_arguments)] // the gate's one sequence: subject, binding, unit
async fn gate_member_op(
    state: &ApiState,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    kind: SetKind,
    member_code: &str,
    op: MemberOp,
    presented: &JsonValue,
    now: OffsetDateTime,
) -> Result<MemberOpGate, CanonicalError> {
    match crate::api::rest::authorize_live_op(
        state,
        scope,
        tenant_id,
        member_op_subject(tenant_id, kind, member_code),
    )
    .await
    {
        Ok(authorization) => {
            if let Err(mismatch) =
                bound_to_the_change(state, tenant_id, &authorization, presented).await?
            {
                // A record stands and is not for this change. It is open, so
                // opening a second one would supersede it: report it.
                return Err(refuse_set(
                    state,
                    scope,
                    tenant_id,
                    actor_ref,
                    kind,
                    member_code.to_owned(),
                    mismatch,
                )
                .await);
            }
            Ok(MemberOpGate::Authorized(authorization))
        }
        // **Only** `APPROVAL_REQUIRED` opens a unit. The host's other
        // refusals — a corrupt stored subject, a gate that judged and said no
        // — are answers about the record that exists, and minting a second
        // one over them would turn a refusal into a proposal.
        Err(crate::api::rest::HostError::Refused(DomainError::ApprovalRequired(_))) => {
            open_the_unit(
                state,
                scope,
                tenant_id,
                actor_ref,
                kind,
                member_code,
                op,
                presented,
                now,
            )
            .await
        }
        Err(crate::api::rest::HostError::Refused(refusal)) => Err(refuse_set(
            state,
            scope,
            tenant_id,
            actor_ref,
            kind,
            member_code.to_owned(),
            refusal,
        )
        .await),
        Err(crate::api::rest::HostError::Repo(error)) => Err(repo_error_to_canonical(&error)),
    }
}

/// The `202` arm's body: answer a standing unit, or open one (**P-D-173**).
#[allow(clippy::too_many_arguments)] // the same sequence, continued
async fn open_the_unit(
    state: &ApiState,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    kind: SetKind,
    member_code: &str,
    op: MemberOp,
    presented: &JsonValue,
    now: OffsetDateTime,
) -> Result<MemberOpGate, CanonicalError> {
    let subject = member_op_subject(tenant_id, kind, member_code);
    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&crate::infra::storage::RepoError::Db(e.to_string()))
    })?;
    let governance = governance_scope(tenant_id);
    let candidates = repo::gate_candidates(&conn, &governance, &subject)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;
    if let Some(standing) = candidates.iter().find(|candidate| {
        matches!(
            candidate.state,
            crate::domain::approval::ApprovalState::Pending
                | crate::domain::approval::ApprovalState::Satisfied
        )
    }) {
        let record = repo::read_approval(&conn, &governance, tenant_id, standing.approval_id)
            .await
            .map_err(|e| repo_error_to_canonical(&e))?
            .ok_or_else(|| {
                repo_error_to_canonical(&crate::infra::storage::RepoError::Db(format!(
                    "approval {} is a candidate and has no row",
                    standing.approval_id
                )))
            })?;
        let agreed = serde_json::from_str::<JsonValue>(&record.content_snapshot)
            .map(|value| canonical::canonical_rendering(&value, canonical::Absence::Omit))
            .unwrap_or_default();
        if agreed == canonical::canonical_rendering(presented, canonical::Absence::Omit) {
            // The caller's own unit, still waiting. Answering `202` again is
            // what makes the re-send idempotent before approval as well as
            // after it.
            return Ok(MemberOpGate::Opened(unit_view(&record)?));
        }
        return Err(refuse_set(
            state,
            scope,
            tenant_id,
            actor_ref,
            kind,
            member_code.to_owned(),
            DomainError::ApprovalRequired(format!(
                "approval {} is open on this member for a different change; decide or reject it \
                 before proposing another — one open unit per subject (design/05 §4)",
                standing.approval_id
            )),
        )
        .await);
    }

    let policy = match repo::resolve_materiality_policy(&conn, &governance, tenant_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?
    {
        crate::domain::materiality::Resolution::Resolved(policy) => policy,
        // The policy read is fail-closed: a submission whose count cannot be
        // resolved is not one this door may mint at a guessed `N`.
        crate::domain::materiality::Resolution::Unresolvable => {
            return Err(repo_error_to_canonical(
                &crate::infra::storage::RepoError::Db(
                    "the tenant's materiality policy could not be resolved, so no approval unit \
                     can carry a count"
                        .to_owned(),
                ),
            ));
        }
    };
    let approval_id = crate::domain::governance::ApprovalId::new(Uuid::now_v7());
    let snapshot = canonical::canonical_rendering(presented, canonical::Absence::Omit);
    // The act is the **door's own**, not a declaration read off a payload:
    // this door knows which of its three routes it is, so P-D-171's exception
    // applies here by construction rather than by the caller's spelling.
    let act = crate::domain::materiality::MaterialAct::LiveOp {
        kind: crate::domain::materiality::MaterialLiveOp::RecognizedSetOp,
        edit: if op.is_display_label_rename() {
            crate::domain::materiality::LiveOpEdit::DisplayLabelRename
        } else {
            crate::domain::materiality::LiveOpEdit::Registered
        },
    };
    let scope_tx = governance.clone();
    let subject_tx = subject.clone();
    let snapshot_tx = snapshot.clone();
    let submitted = state
        .db
        .db()
        .transaction_with_retry::<repo::Submitted, TxError, _, _>(
            toolkit_db::secure::TxConfig::default(),
            member_contention_db_err,
            move |tx| {
                let scope = scope_tx.clone();
                let subject = subject_tx.clone();
                let snapshot = snapshot_tx.clone();
                let policy = policy.clone();
                let act = act.clone();
                Box::pin(async move {
                    // `submit_approval` writes twice — the supersession and
                    // the insert — and its own doc requires the door's
                    // transaction so the pair is atomic.
                    repo::submit_approval(
                        tx,
                        &scope,
                        repo::NewApproval {
                            approval_id,
                            subject: &subject,
                            internal_revision: subject.pin.stored_revision(),
                            content_snapshot: &snapshot,
                            diff_basis: None,
                            act: &act,
                            evaluator: crate::domain::materiality::MaterialityEvaluator::new(
                                crate::domain::materiality::Resolution::Resolved(&policy),
                            ),
                            // The caller declares nothing here: this door
                            // touches no column `inst-gv-finance-predicate`
                            // names, and P-D-169 left the gear no computed
                            // operand to OR in.
                            finance_material: false,
                            approver_count: policy.approver_count(),
                            submitter: actor_ref,
                            author_override_ack: None,
                            override_conditions: Vec::new(),
                        },
                        now,
                    )
                    .await
                    // **A refusal is a refusal, not a `500`.** The one a
                    // caller can actually provoke here is
                    // `uq_products_approval_open`: two of their own retries
                    // racing, which `classify_submit_insert` turns into a
                    // refusal precisely so a caller can act on it. Flattened
                    // into a driver failure it would answer `500` for a
                    // double-click.
                    .map_err(|e| match e {
                        repo::ApprovalStoreError::Refused(refusal) => TxError::Refused(refusal),
                        repo::ApprovalStoreError::Repo(error) => TxError::Repo(error),
                    })
                })
            },
        )
        .await;
    let submitted = match submitted {
        Ok(submitted) => submitted,
        Err(TxError::Refused(refusal)) => {
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
        Err(TxError::Repo(error)) => return Err(repo_error_to_canonical(&error)),
        Err(TxError::NotFound) => {
            return Err(repo_error_to_canonical(
                &crate::infra::storage::RepoError::Db(
                    "the submission raised NotFound, which no branch of it constructs".to_owned(),
                ),
            ));
        }
    };

    if matches!(
        submitted.state,
        crate::domain::approval::ApprovalState::Satisfied
    ) {
        // Born satisfied at `required = 0` (P-D-119 row 31). Nothing waits,
        // so `202 Accepted` would be a lie: the door proceeds in this
        // request, on the record it has just opened.
        let authorization = authorize_member_op(
            state,
            scope,
            tenant_id,
            actor_ref,
            kind,
            member_code,
            presented,
        )
        .await?;
        return Ok(MemberOpGate::Authorized(authorization));
    }
    Ok(MemberOpGate::Opened(MemberOpApprovalView {
        approval_id: approval_id.get(),
        state: submitted.state.as_str().to_owned(),
        required: submitted.descriptor.required(),
        configured_quorum: submitted.descriptor.configured_quorum(),
        finance_required: submitted.descriptor.finance_required(),
        quorum_reduced: submitted.descriptor.quorum_reduced(),
    }))
}

/// The `202` body for a unit that was already standing, read back off its
/// stored descriptor rather than recomputed (`design/05` §4: the descriptor
/// is stored at submission and never re-derived).
fn unit_view(
    record: &crate::infra::storage::entity::approval::Model,
) -> Result<MemberOpApprovalView, CanonicalError> {
    let descriptor = crate::domain::approval::descriptor_from_stored(&record.quorum_descriptor)
        .map_err(|e| {
            repo_error_to_canonical(&crate::infra::storage::RepoError::CorruptRow(format!(
                "approval {}'s stored descriptor: {e}",
                record.approval_id
            )))
        })?;
    Ok(MemberOpApprovalView {
        approval_id: record.approval_id,
        state: record.state.clone(),
        required: descriptor.required(),
        configured_quorum: descriptor.configured_quorum(),
        finance_required: descriptor.finance_required(),
        quorum_reduced: descriptor.quorum_reduced(),
    })
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
    let Some(record) =
        repo::read_approval(&conn, &governance_scope(tenant_id), tenant_id, approval_id)
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
    // the kind's `read` grant (P-D-170). **The by-code read answers an
    // `ETag`** (P-D-174), which the two per-member write doors assert back as
    // `If-Match`; **the list does not**, and that is P-D-170's own rule kept
    // rather than dropped — no door takes a set-level precondition, so a list
    // tag would be the header with nobody to assert it that entry refused.
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
         for the tier set, its display label, with the **`ETag`** its two per-member write \
         doors assert back as `If-Match` (P-D-174) - the `SHA-256` of the member's canonical \
         rendering, a live row having no revision to name. Gates on the kind's `read` grant. A member \
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
             enqueues the set's event in the same transaction. **No `If-Match`**: a create \
             names no member to have read, and the wildcard is refused everywhere in this \
             gear; the add's concurrency guard is the `DUPLICATE_CODE` refusal under the \
             primary key, which is stronger than a tag (P-D-174). **The door has two arms** (P-D-173): with no approval unit standing for this exact change it opens one, answers `202` naming it, and writes nothing; the identical request re-sent once that unit is approved answers below. A unit already open for a *different* change on the member is named in a `403 APPROVAL_REQUIRED` rather than superseded - `design/05` admits one open unit per subject. At `N = 0` the unit is born satisfied and the first call writes. \
             The grant is chosen by `setKind` \
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
        .json_response_with_schema::<MemberOpApprovalView>(
            openapi,
            StatusCode::ACCEPTED,
            "No approval unit stood for this change, so one was opened: the body names it and \
             its effective count. Nothing was written. Re-send the identical request once the \
             unit is approved.",
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
         **The door has two arms** (P-D-173): with no approval unit standing for this exact change it opens one, answers `202` naming it, and writes nothing; the identical request re-sent once that unit is approved answers below. A unit already open for a *different* change on the member is named in a `403 APPROVAL_REQUIRED` rather than superseded - `design/05` admits one open unit per subject. At `N = 0` the unit is born satisfied and the first call writes. \
         `active -> removed` is refused: de-listing deprecates first, so new declarations \
         stop before the member can leave the set. **`If-Match` is required** and asserts \
         the member's own tag from `GET .../members/{memberCode}`; a stale one is \
         `STALE_LIVE_OP`, the same code a stale `expected_state` earns, because a live row's \
         staleness has one voice (P-D-174). The tag pins the whole row where `expected_state` \
         pins one column, and both are kept: the state is what the approval unit was agreed \
         against and cannot be re-derived at apply time. The body pins the state the caller \
         read (`expected_state`); a peer's flip in between is refused `STALE_LIVE_OP`. A removal \
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
        .json_response_with_schema::<MemberOpApprovalView>(
        openapi,
        StatusCode::ACCEPTED,
        "No approval unit stood for this change, so one was opened: the body names it and \
         its effective count. Nothing was written. Re-send the identical request once the \
         unit is approved.",
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
         admits. **`If-Match` is required** and asserts the member's own tag from `GET \
         .../members/{memberCode}` - this door had no staleness pin at all before P-D-174, so \
         two operators renaming one member raced and the later write won silently; a stale \
         tag is `STALE_LIVE_OP`. **The door has two arms** (P-D-173): with no approval unit standing for this exact change it opens one, answers `202` naming it, and writes nothing; the identical request re-sent once that unit is approved answers below. A unit already open for a *different* change on the member is named in a `403 APPROVAL_REQUIRED` rather than superseded - `design/05` admits one open unit per subject. At `N = 0` the unit is born satisfied and the first call writes. \
         The member's code is its identity and has no update path (the table's \
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
        .json_response_with_schema::<MemberOpApprovalView>(
        openapi,
        StatusCode::ACCEPTED,
        "No approval unit stood for this change, so one was opened: the body names it and \
         its effective count. Nothing was written. Re-send the identical request once the \
         unit is approved.",
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

    // The tag before the move: `from_member` consumes the record, and the
    // header a caller sends back on the next verb has to be this read's.
    let tag = member_etag(&member);
    Ok((
        [(axum::http::header::ETAG, tag)],
        Json(RecognizedMemberView::from_member(kind, member)),
    )
        .into_response())
}

/// `POST /recognized-sets/{setKind}/members/{memberCode}/label`.
///
/// @cpt-dod:cpt-cf-bss-products-dod-plantier-governance:p1
async fn relabel_member(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path((set_kind, member_code)): Path<(String, String)>,
    headers: axum::http::HeaderMap,
    Json(body): Json<MemberRelabelRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let kind = parse_kind(&set_kind)?;
    let asserted = member_if_match(&headers).map_err(CanonicalError::from)?;
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
    let authorization = match gate_member_op(
        &state,
        &scope,
        tenant_id,
        actor_ref,
        kind,
        &member_code,
        MemberOp::Relabel,
        &relabel_declaration(body.display_label.as_deref()),
        now,
    )
    .await?
    {
        MemberOpGate::Authorized(authorization) => authorization,
        MemberOpGate::Opened(unit) => {
            return Ok((StatusCode::ACCEPTED, Json(unit)).into_response());
        }
    };

    let outbox = state.sink.clone();
    let scope_tx = scope.clone();
    let code_tx = member_code.clone();
    let label = body.display_label.clone();
    let authorization_tx = authorization.clone();
    let asserted_tx = asserted.clone();
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
                let asserted = asserted_tx.clone();
                Box::pin(async move {
                    settle_member_op(tx, &scope, tenant_id, &authorization, now).await?;
                    // The label door had **no** staleness pin at all before
                    // P-D-174 — `expected_state` is the transitions body's —
                    // so two operators renaming one member raced and the
                    // later write won silently. The tag is read under the
                    // write, inside the transaction.
                    let Some(held) =
                        repo::recognized_member(tx, &scope, tenant_id, kind, &member_code)
                            .await
                            .map_err(TxError::Repo)?
                    else {
                        return Err(TxError::NotFound);
                    };
                    if member_etag(&held) != asserted {
                        return Err(TxError::Refused(stale_member_tag(kind, &member_code)));
                    }
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

    let authorization = match gate_member_op(
        &state,
        &scope,
        tenant_id,
        actor_ref,
        kind,
        &member_code,
        MemberOp::Add,
        &add_declaration(&member_code, body.display_label.as_deref()),
        now,
    )
    .await?
    {
        MemberOpGate::Authorized(authorization) => authorization,
        MemberOpGate::Opened(unit) => {
            return Ok((StatusCode::ACCEPTED, Json(unit)).into_response());
        }
    };
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
    headers: axum::http::HeaderMap,
    Json(body): Json<MemberTransitionRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let kind = parse_kind(&set_kind)?;
    let asserted = member_if_match(&headers).map_err(CanonicalError::from)?;
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

    let authorization = match gate_member_op(
        &state,
        &scope,
        tenant_id,
        actor_ref,
        kind,
        &member_code,
        MemberOp::Transition,
        &transition_declaration(to.as_str(), expected.as_str()),
        now,
    )
    .await?
    {
        MemberOpGate::Authorized(authorization) => authorization,
        MemberOpGate::Opened(unit) => {
            return Ok((StatusCode::ACCEPTED, Json(unit)).into_response());
        }
    };
    let outbox = state.sink.clone();
    let scope_tx = scope.clone();
    let code_tx = member_code.clone();
    let authorization_tx = authorization.clone();
    let asserted_tx = asserted.clone();
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
                let asserted = asserted_tx.clone();
                Box::pin(async move {
                    settle_member_op(tx, &scope, tenant_id, &authorization, now).await?;
                    let Some(stored) =
                        repo::recognized_member(tx, &scope, tenant_id, kind, &member_code)
                            .await
                            .map_err(TxError::Repo)?
                    else {
                        return Err(TxError::NotFound);
                    };

                    // **The tag first** (P-D-174): it pins the whole row,
                    // where `expected_state` pins one column, so a caller
                    // whose read is stale in the label alone is told so here
                    // rather than writing over it. Inside the transaction
                    // that would otherwise race it, which is the placement
                    // `preconditions`' own doc gives for the entity heads.
                    if member_etag(&stored) != asserted {
                        return Err(TxError::Refused(stale_member_tag(kind, &member_code)));
                    }

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
