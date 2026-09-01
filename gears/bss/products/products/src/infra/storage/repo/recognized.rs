//! The recognized-set membership surface — the generic lookup, the member
//! writes the two P-D-90 doors perform, and the holders sample the delist
//! refusal names (`design/03` §3.1, `dod-recognized-set-mechanics`,
//! `dod-unit-delist`).
//!
//! # One implementation, four sets
//!
//! The three **membership** functions key on `(tenant_id, set_kind,
//! member_code)` and none branches on the kind — [`metering_unit_holders`]
//! is the module's one kind-specific surface, and deliberately so: a holder
//! sample reads the carrier column, and only the metering-unit set has one
//! on `products_sku` today. The other three kinds' samples arrive with their
//! own columns. So: P-D-90 arm 3 makes the membership machinery one
//! generic implementation, the kind deciding only the grant at the door and
//! the refusal code the delist raises. **The set is the `active` and
//! `deprecated` rows; a `removed` row is a tombstone outside it** — which is
//! why [`member_state`] answers the stored state and leaves set-membership
//! judgements to `domain::recognized`, one rule with every caller.

use chrono::{DateTime, Utc};
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use toolkit_db::secure::{
    AccessScope, DBRunner, SecureEntityExt, SecureInsertExt, SecureUpdateExt,
};
use uuid::Uuid;

use super::super::entity::{recognized_set, sku};
use super::{RepoError, driver_failure};
use crate::domain::recognized::{MemberState, SetKind};

/// One state flip: the state the caller's `GovernedLiveOp` read, and the
/// state to move to. Bundled because the two travel together and a call site
/// could transpose two loose `MemberState` arguments without the compiler
/// noticing.
#[derive(Debug, Clone, Copy)]
pub struct StateFlip {
    /// The state the caller read — the staleness pin.
    pub expected: MemberState,
    /// The admitted edge's target.
    pub to: MemberState,
}

/// One member row as the doors and the validators read it.
#[derive(Debug, Clone)]
pub struct RecognizedMember {
    /// The member's code — the identity that never changes.
    pub member_code: String,
    /// The tier set's operator-facing label; ignored by the other three.
    pub display_label: Option<String>,
    /// `active`, `deprecated` or `removed`, parsed fail-closed.
    pub state: MemberState,
    /// Who seeded it, or `None` for an operator-added member. A seeded
    /// member is deprecatable and never removed (`inst-rs-seeded`).
    pub seeded_by: Option<String>,
}

fn into_member(row: recognized_set::Model) -> Result<RecognizedMember, RepoError> {
    let state = MemberState::parse(&row.state).ok_or_else(|| {
        RepoError::CorruptRow(format!(
            "products_recognized_set.state `{}` on member {}/{}",
            row.state, row.set_kind, row.member_code
        ))
    })?;
    Ok(RecognizedMember {
        member_code: row.member_code,
        display_label: row.display_label,
        state,
        seeded_by: row.seeded_by,
    })
}

/// Read one member, or `None` where the set never carried the code.
///
/// # Errors
///
/// [`RepoError::Driver`] on a storage failure, [`RepoError::CorruptRow`] on
/// a state outside the roster.
pub async fn recognized_member(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    set_kind: SetKind,
    member_code: &str,
) -> Result<Option<RecognizedMember>, RepoError> {
    let row = recognized_set::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(recognized_set::Column::TenantId.eq(tenant_id))
                .add(recognized_set::Column::SetKind.eq(set_kind.as_str()))
                .add(recognized_set::Column::MemberCode.eq(member_code)),
        )
        .one(runner)
        .await
        .map_err(|e| {
            driver_failure(
                format!("read recognized member {}/{member_code}", set_kind.as_str()),
                e,
            )
        })?;
    row.map(into_member).transpose()
}

/// Insert one `active` member — the add door's write.
///
/// The PK `(tenant_id, set_kind, member_code)` is the arbiter: a duplicate
/// add — including one naming a `removed` tombstone, whose PK never frees —
/// surfaces as a driver conflict for the door to classify, never as a second
/// row.
///
/// # Errors
///
/// [`RepoError::Driver`] on a storage failure or the PK conflict.
pub async fn insert_recognized_member(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    set_kind: SetKind,
    member_code: &str,
    display_label: Option<String>,
    now: DateTime<Utc>,
) -> Result<RecognizedMember, RepoError> {
    let row = recognized_set::ActiveModel {
        tenant_id: Set(tenant_id),
        set_kind: Set(set_kind.as_str().to_owned()),
        member_code: Set(member_code.to_owned()),
        display_label: Set(display_label),
        state: Set(MemberState::Active.as_str().to_owned()),
        seeded_by: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };
    recognized_set::Entity::insert(row.clone())
        .secure()
        .scope_with_model(scope, &row)
        .map_err(|e| driver_failure(format!("member scope of {tenant_id}"), e))?
        .exec(runner)
        .await
        .map_err(|e| {
            driver_failure(
                format!("add recognized member {}/{member_code}", set_kind.as_str()),
                e,
            )
        })?;
    let stored = recognized_member(runner, scope, tenant_id, set_kind, member_code)
        .await?
        .ok_or_else(|| {
            RepoError::Db(format!(
                "recognized member {}/{member_code} vanished between its insert and its read-back",
                set_kind.as_str()
            ))
        })?;
    Ok(stored)
}

/// Flip one member's state, pinned at the state the caller's
/// `GovernedLiveOp` expected — the transitions door's write.
///
/// The pin is the live-op's own staleness rule made physical: a peer's flip
/// between the door's read and this statement leaves `rows_affected = 0`,
/// and the door answers `STALE_LIVE_OP` rather than absorbing the race.
///
/// **The guard is a complement enumeration, not the whitelist §4 words.**
/// The shipped trigger refuses five named columns — `tenant_id`, `set_kind`,
/// `member_code`, `seeded_by`, `created_at` — and admits everything else, so
/// `updated_at` is writable (this statement writes it) and a column a later
/// migration adds is admitted by default. `design/03` §6 carries that
/// difference as an open question; the migration's own doc states it. What
/// keeps this statement to `state` and the timestamp is therefore the
/// statement, not the guard.
///
/// # Errors
///
/// [`RepoError::Driver`] on a storage failure. A vanished or moved member is
/// `Ok(false)`, the caller's to classify.
pub async fn flip_recognized_member(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    set_kind: SetKind,
    member_code: &str,
    flip: StateFlip,
    now: DateTime<Utc>,
) -> Result<bool, RepoError> {
    let StateFlip { expected, to } = flip;
    let result = recognized_set::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(recognized_set::Column::State, Expr::value(to.as_str()))
        .col_expr(recognized_set::Column::UpdatedAt, Expr::value(now))
        .filter(
            Condition::all()
                .add(recognized_set::Column::TenantId.eq(tenant_id))
                .add(recognized_set::Column::SetKind.eq(set_kind.as_str()))
                .add(recognized_set::Column::MemberCode.eq(member_code))
                .add(recognized_set::Column::State.eq(expected.as_str())),
        )
        .exec(runner)
        .await
        .map_err(|e| {
            driver_failure(
                format!("flip recognized member {}/{member_code}", set_kind.as_str()),
                e,
            )
        })?;
    Ok(result.rows_affected == 1)
}

/// The non-terminal published heads still declaring `member_code` as their
/// metering unit — the delist refusal's operand and its sample
/// (`inst-us-delist`: "referenced" means **non-terminal published heads**, and
/// frozen version content never blocks a removal).
///
/// Answers up to `sample + 1` codes, ordered, so the refusal can name
/// holders without shipping a tenant's whole catalog in an error message —
/// and so the caller can say *"at least N"* honestly when the bound is hit,
/// rather than quoting a total read by a second statement that may disagree
/// with the sample.
///
/// # Errors
///
/// [`RepoError::Driver`] on a storage failure.
pub async fn metering_unit_holders(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    member_code: &str,
    sample: u64,
) -> Result<Vec<String>, RepoError> {
    let filter = Condition::all()
        .add(sku::Column::TenantId.eq(tenant_id))
        .add(sku::Column::MeteringUnit.eq(member_code))
        .add(sku::Column::LifecycleState.is_in(["published", "deprecated"]));

    // ONE statement, not a count plus a sample: two reads under READ
    // COMMITTED can disagree — a holder retiring between them left the
    // refusal rendering a non-zero total with an empty exemplar, on the one
    // message whose whole point is *"with the holders sampled"*. The sample
    // bound is `sample + 1` so the caller can tell "this many" from "at
    // least this many" without a second query.
    let rows: Vec<String> = sku::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(filter)
        .order_by(sku::Column::SkuCode, sea_orm::Order::Asc)
        .limit(sample + 1)
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("sample holders of unit {member_code}"), e))?
        .into_iter()
        .map(|row| row.sku_code)
        .collect();
    Ok(rows)
}
