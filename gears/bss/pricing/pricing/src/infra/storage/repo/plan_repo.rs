//! Repository for the plan aggregate's **revision chain** (`pricing_plan`,
//! D-56 / D-83 / D-90 / D-128).
//!
//! A plan is not a row, so this is not a CRUD repository over one. It is the
//! authoring half of `design/01-foundation.md` §4.2 step 1 — create a draft,
//! edit it, abandon it, or open the next revision — and every method exists
//! because the chain has an invariant that would otherwise be enforced nowhere
//! but in a caller's discipline.
//!
//! **The compare-and-swap is one statement, not a read then a write.**
//! [`update_draft`] and [`delete_draft`] match on the row version the caller
//! read *inside* the UPDATE/DELETE that acts on it, and the bump is
//! `row_version = row_version + 1` in that same statement. Computing the
//! successor in Rust would let two writers holding the same current version
//! compute the same next one and both write it — the silent overwrite
//! `cpt-cf-bss-pricing-fr-concurrent-edit` forbids, reintroduced by the helper
//! meant to prevent it. `domain::concurrency::RowVersion` has no increment for
//! exactly this reason.
//!
//! **A failed swap gets three answers, not one.** Zero rows affected is
//! ambiguous by construction — the predicate is a conjunction — so the row is
//! read back once and the refusal names which conjunct failed:
//! [`RepoError::NotFound`] (absent, or another tenant's),
//! [`RepoError::NotDraft`] (frozen), [`RepoError::StaleRowVersion`] (a read the
//! caller never refreshed). One undifferentiated conflict would tell an
//! operator to retry in the one case where retrying can never work.
//!
//! **The draft-only guard is enforced here as well as by the trigger.** §4.3 is
//! explicit that immutability is enforced twice, and the second enforcement is
//! not redundancy: the table trigger's answer is a raw database error carrying
//! no state and no subject, which reaches a caller as an internal fault. The
//! predicate on these statements is what turns "you are editing a frozen
//! revision" into something a surface can render.
//!
//! [`update_draft`]: PlanRepo::update_draft
//! [`delete_draft`]: PlanRepo::delete_draft

use chrono::{DateTime, Utc};
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use toolkit_db::secure::{
    AccessScope, SecureDeleteExt, SecureEntityExt, SecureInsertExt, SecureUpdateExt,
};
use toolkit_db::{DBProvider, DbError};
use uuid::Uuid;

use crate::domain::concurrency::RowVersion;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::plan::{PlanRevision, PlanShapePatch};
use crate::domain::scope_key::PlanId;
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::plan;

/// The noun every **compare-and-swap** refusal names, so a caller that failed
/// to edit revision 3 and a caller that failed to delete it are told about the
/// same kind of thing in the same word.
///
/// It is not the only subject this file emits, and the doc says so rather than
/// pretending otherwise: [`PlanRepo::open_revision`] reports a missing
/// `current plan revision`, because the referent it could not find is the
/// **current** revision and not the one the caller named — "plan revision not
/// found" would be false and would send an author looking for a revision
/// nobody asked about. Two nouns, each true of what it refuses; the rule this
/// const enforces is the narrower and more useful one, that no single refusal
/// gets spelled two ways.
const SUBJECT: &str = "plan revision";

/// Everything a plan's **first** revision needs.
///
/// The plan id is caller-supplied rather than minted here: the authoring
/// surface that mints it (G7) is also the one that has to return it in a
/// `Location` header before the row is durable, and a repository that generated
/// ids would make an idempotent retry create a second plan.
///
/// `created_at_utc` is caller-supplied for the same reason [`PlanRepo::open_revision`]
/// takes `now`: the catalog "mutates state only in response to explicit
/// authoring calls" (§2.2) and never self-originates a row, so the authoring
/// instant belongs to the request rather than to whichever database node
/// happened to evaluate `now()`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewPlanDraft {
    /// The plan being created.
    pub plan_id: PlanId,
    /// The owning tenant.
    pub tenant_id: Uuid,
    /// Pseudonymous principal id of the authoring actor.
    pub created_by: Uuid,
    /// When the request was authored, UTC.
    pub created_at_utc: DateTime<Utc>,
    /// The catalog SKU this plan realizes, when one is bound.
    pub sku_id: Option<Uuid>,
    /// The plan's tier.
    pub plan_tier: Option<String>,
    /// The plan's billing cycle.
    pub billing_cycle: Option<String>,
    /// Start of the availability window, UTC.
    pub available_from: Option<DateTime<Utc>>,
    /// End of the availability window, UTC.
    pub available_to: Option<DateTime<Utc>>,
}

/// `SeaORM`-backed repository over the plan revision chain.
#[derive(Clone)]
pub struct PlanRepo {
    db: DBProvider<DbError>,
}

impl PlanRepo {
    /// Build over one database provider.
    #[must_use]
    pub fn new(db: DBProvider<DbError>) -> Self {
        Self { db }
    }

    /// Create a plan by inserting its revision `0` in `draft`.
    ///
    /// # Errors
    /// [`RepoError::Db`] on a scope or storage failure — which **includes the
    /// collision case**: a plan id that already has a revision `0` is the
    /// table's `PRIMARY KEY` answer, and a second open draft is
    /// `uq_pricing_plan_open_draft`'s. Neither is pre-checked, because a
    /// pre-check would be a read the insert races with anyway.
    pub async fn create_draft(
        &self,
        scope: &AccessScope,
        draft: NewPlanDraft,
    ) -> Result<PlanRevision, RepoError> {
        let tenant_id = draft.tenant_id;
        let opened = PlanRevision {
            plan_id: draft.plan_id,
            revision: 0,
            sku_id: draft.sku_id,
            plan_tier: draft.plan_tier,
            billing_cycle: draft.billing_cycle,
            available_from: draft.available_from,
            available_to: draft.available_to,
            lifecycle_state: LifecycleState::Draft,
            created_by: draft.created_by,
            created_at_utc: draft.created_at_utc,
            row_version: RowVersion::new(0),
        };
        self.insert_revision(scope, tenant_id, &opened).await?;
        Ok(opened)
    }

    /// Read one revision by its composite identity.
    ///
    /// SQL-level BOLA: a foreign tenant's revision yields `None`.
    ///
    /// # Errors
    /// [`RepoError::Db`] on a scope or storage failure;
    /// [`RepoError::CorruptRow`] when the stored row cannot be read as the
    /// domain value its columns are `CHECK`-constrained to hold.
    pub async fn find_revision(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        plan_id: PlanId,
        revision: u64,
    ) -> Result<Option<PlanRevision>, RepoError> {
        let Some(number) = stored_revision(revision) else {
            return Ok(None);
        };
        let conn = self
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("conn: {e}")))?;
        let row = plan::Entity::find()
            .secure()
            .scope_with(scope)
            .filter(
                Condition::all()
                    .add(plan::Column::TenantId.eq(tenant_id))
                    .add(plan::Column::PlanId.eq(plan_id.get()))
                    .add(plan::Column::Revision.eq(number)),
            )
            .one(&conn)
            .await
            .map_err(|e| RepoError::Db(format!("read plan revision: {e}")))?;
        row.map(to_domain).transpose()
    }

    /// Read the plan's **current** revision.
    ///
    /// Current is `published` **or** `retired` (D-128), and the set is taken
    /// from [`LifecycleState::is_current_revision`] rather than restated here,
    /// so this query and the `uq_pricing_plan_current` partial index cannot
    /// drift apart. Retirement flips the only published revision; under the
    /// narrower predicate the projector, the sellability gate and every
    /// referential check would suddenly have no referent at all.
    ///
    /// # Errors
    /// [`RepoError::Db`] on a scope or storage failure;
    /// [`RepoError::CorruptRow`] when more than one revision answers — the
    /// partial `UNIQUE` index makes that impossible, so seeing it means the
    /// index is gone and "the current revision" has stopped being well defined.
    pub async fn find_current(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        plan_id: PlanId,
    ) -> Result<Option<PlanRevision>, RepoError> {
        let conn = self
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("conn: {e}")))?;
        let rows = plan::Entity::find()
            .secure()
            .scope_with(scope)
            .filter(
                Condition::all()
                    .add(plan::Column::TenantId.eq(tenant_id))
                    .add(plan::Column::PlanId.eq(plan_id.get()))
                    .add(plan::Column::LifecycleState.is_in(current_tokens())),
            )
            .all(&conn)
            .await
            .map_err(|e| RepoError::Db(format!("read current plan revision: {e}")))?;
        if rows.len() > 1 {
            return Err(RepoError::CorruptRow(format!(
                "plan {plan_id} holds {} current revisions; uq_pricing_plan_current permits one",
                rows.len()
            )));
        }
        rows.into_iter().next().map(to_domain).transpose()
    }

    /// Read the plan's open `draft` revision, when it has one.
    ///
    /// # Errors
    /// [`RepoError::Db`] on a scope or storage failure;
    /// [`RepoError::CorruptRow`] when the stored row cannot be read as the
    /// domain value its columns are `CHECK`-constrained to hold.
    pub async fn find_open_draft(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        plan_id: PlanId,
    ) -> Result<Option<PlanRevision>, RepoError> {
        let conn = self
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("conn: {e}")))?;
        let row = plan::Entity::find()
            .secure()
            .scope_with(scope)
            .filter(
                Condition::all()
                    .add(plan::Column::TenantId.eq(tenant_id))
                    .add(plan::Column::PlanId.eq(plan_id.get()))
                    .add(plan::Column::LifecycleState.eq(LifecycleState::Draft.as_str())),
            )
            .one(&conn)
            .await
            .map_err(|e| RepoError::Db(format!("read open plan draft: {e}")))?;
        row.map(to_domain).transpose()
    }

    /// Apply `patch` to an open draft revision, under the caller's row version.
    ///
    /// One statement does all of it: the patched columns, the
    /// `row_version + 1` bump, and the conjunction that makes it a
    /// compare-and-swap — tenant, plan, revision, the submitted version, and
    /// `lifecycle_state = 'draft'`. An **empty patch is a valid request**: it
    /// asserts the caller's `ETag` and advances it, which is what keeps a no-op
    /// edit distinguishable from a lost one.
    ///
    /// The returned value is a fresh read rather than a computed one, so under
    /// a concurrent second edit it may already carry a later version. The swap
    /// itself is still atomic — what the caller sees is the row as it stands,
    /// which is the only honest answer a non-`RETURNING` path can give.
    ///
    /// # Errors
    /// [`RepoError::NotFound`] when no such revision is visible to `scope`;
    /// [`RepoError::NotDraft`] when it is visible but frozen;
    /// [`RepoError::StaleRowVersion`] carrying both versions when the submitted
    /// one is not current; [`RepoError::Db`] on a scope or storage failure;
    /// [`RepoError::CorruptRow`] when the updated row reads back unusable.
    pub async fn update_draft(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        plan_id: PlanId,
        revision: u64,
        expected: RowVersion,
        patch: PlanShapePatch,
    ) -> Result<PlanRevision, RepoError> {
        let Some(guard) = swap_guard(tenant_id, plan_id, revision, expected) else {
            return Err(self
                .refuse(scope, tenant_id, plan_id, revision, expected)
                .await);
        };
        let conn = self
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("conn: {e}")))?;

        let mut update = plan::Entity::update_many().secure().scope_with(scope);
        if let Some(sku_id) = patch.sku_id {
            update = update.col_expr(plan::Column::SkuId, Expr::value(sku_id));
        }
        if let Some(plan_tier) = patch.plan_tier {
            update = update.col_expr(plan::Column::PlanTier, Expr::value(plan_tier));
        }
        if let Some(billing_cycle) = patch.billing_cycle {
            update = update.col_expr(plan::Column::BillingCycle, Expr::value(billing_cycle));
        }
        if let Some(available_from) = patch.available_from {
            update = update.col_expr(plan::Column::AvailableFrom, Expr::value(available_from));
        }
        if let Some(available_to) = patch.available_to {
            update = update.col_expr(plan::Column::AvailableTo, Expr::value(available_to));
        }

        let result = update
            .col_expr(
                plan::Column::RowVersion,
                Expr::col(plan::Column::RowVersion).add(1_i64),
            )
            .filter(guard)
            .exec(&conn)
            .await
            .map_err(|e| RepoError::Db(format!("update plan draft: {e}")))?;

        if result.rows_affected == 0 {
            return Err(self
                .refuse(scope, tenant_id, plan_id, revision, expected)
                .await);
        }
        self.find_revision(scope, tenant_id, plan_id, revision)
            .await?
            .ok_or_else(|| not_found(plan_id, revision))
    }

    /// Delete an open draft revision, under the caller's row version.
    ///
    /// Only a never-published `draft` is deletable (§4.3), and the same
    /// conjunction that guards [`PlanRepo::update_draft`] guards this. The
    /// table's DELETE trigger refuses a non-draft row too, and this method
    /// deliberately does not lean on it: the trigger's answer is a database
    /// error with no state in it, so a caller that abandons the wrong revision
    /// would be told the store is broken rather than that the revision is
    /// published.
    ///
    /// # Errors
    /// [`RepoError::NotFound`] when no such revision is visible to `scope`;
    /// [`RepoError::NotDraft`] when it is visible but frozen;
    /// [`RepoError::StaleRowVersion`] carrying both versions when the submitted
    /// one is not current; [`RepoError::Db`] on a scope or storage failure.
    pub async fn delete_draft(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        plan_id: PlanId,
        revision: u64,
        expected: RowVersion,
    ) -> Result<(), RepoError> {
        let Some(guard) = swap_guard(tenant_id, plan_id, revision, expected) else {
            return Err(self
                .refuse(scope, tenant_id, plan_id, revision, expected)
                .await);
        };
        let conn = self
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("conn: {e}")))?;
        let result = plan::Entity::delete_many()
            .secure()
            .scope_with(scope)
            .filter(guard)
            .exec(&conn)
            .await
            .map_err(|e| RepoError::Db(format!("delete plan draft: {e}")))?;

        if result.rows_affected == 0 {
            return Err(self
                .refuse(scope, tenant_id, plan_id, revision, expected)
                .await);
        }
        Ok(())
    }

    /// Open the next revision of a published plan: `revision + 1` in `draft`,
    /// `row_version = 0`, the current revision's shape copied forward.
    ///
    /// This is only the **opening** half of D-90. The new revision publishes
    /// through the standard §4.2 path and flips its predecessor `superseded` in
    /// the same commit; that flip belongs to the publish unit (G5) and nothing
    /// here anticipates it.
    ///
    /// # The child-copy gap (D-83) — open, and owned by G4
    ///
    /// D-83 requires a new revision to **copy its child shape tables** —
    /// `pricing_plan_phase`, `pricing_plan_addon_rule`,
    /// `pricing_plan_descriptor_set` — with stable `phase_id`s, so the `phase`
    /// scope-key axis and same-key supersession survive the revision. Those
    /// tables **do not exist yet**: they are Slice-2 storage and land in G4.
    /// This method therefore copies the plan's own columns and nothing else.
    ///
    /// Until G4 closes it, a revision opened on a plan that has phases, add-on
    /// rules or a descriptor set would carry **none of them** — the draft would
    /// look like a plan whose author had deleted its whole shape. There is
    /// deliberately no copier trait or registry standing in for the missing
    /// work: an abstraction with zero implementations hides the gap instead of
    /// stating it, and the gap is pinned by a test in
    /// `tests/sqlite_plan_repo.rs` that fails the moment the first child table
    /// appears.
    ///
    /// # Errors
    /// [`RepoError::NotFound`] when the plan has no current revision — the
    /// same answer a plan outside `scope` gets;
    /// [`RepoError::NoSuccessorRevision`] when the current revision is
    /// `retired`, since a retired revision can never flip `superseded` and a
    /// successor it could never yield to would be unpublishable by
    /// construction; [`RepoError::OpenDraftExists`] naming the
    /// revision that already holds the plan's one editable slot;
    /// [`RepoError::Db`] on a scope or storage failure — which **includes
    /// losing the race** for that slot to a concurrent `open_revision`, since
    /// the three checks above are reads and `uq_pricing_plan_open_draft` is
    /// what actually decides the winner; [`RepoError::CorruptRow`] when the
    /// current revision reads back unusable or has no representable successor.
    pub async fn open_revision(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        plan_id: PlanId,
        created_by: Uuid,
        now: DateTime<Utc>,
    ) -> Result<PlanRevision, RepoError> {
        let Some(current) = self.find_current(scope, tenant_id, plan_id).await? else {
            return Err(RepoError::NotFound {
                subject: "current plan revision".to_owned(),
                id: plan_id.to_string(),
            });
        };
        // Asked of the state machine rather than matched on `Retired`: what
        // actually blocks a successor is the predecessor's inability to be
        // superseded when that successor publishes.
        if !current
            .lifecycle_state
            .can_transition(LifecycleState::Superseded)
        {
            return Err(RepoError::NoSuccessorRevision {
                plan_id: plan_id.to_string(),
                state: current.lifecycle_state.to_string(),
            });
        }
        if let Some(open) = self.find_open_draft(scope, tenant_id, plan_id).await? {
            return Err(RepoError::OpenDraftExists {
                plan_id: plan_id.to_string(),
                revision: open.revision,
            });
        }

        let next = current.revision.checked_add(1).ok_or_else(|| {
            RepoError::CorruptRow(format!(
                "plan {plan_id} stands at revision {}, which has no successor",
                current.revision
            ))
        })?;
        let opened = PlanRevision {
            plan_id,
            revision: next,
            sku_id: current.sku_id,
            plan_tier: current.plan_tier,
            billing_cycle: current.billing_cycle,
            available_from: current.available_from,
            available_to: current.available_to,
            lifecycle_state: LifecycleState::Draft,
            created_by,
            created_at_utc: now,
            row_version: RowVersion::new(0),
        };
        self.insert_revision(scope, tenant_id, &opened).await?;
        Ok(opened)
    }

    /// Write `revision` as a new row, exactly as the value describes it.
    async fn insert_revision(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        revision: &PlanRevision,
    ) -> Result<(), RepoError> {
        let number = stored_revision(revision.revision).ok_or_else(|| {
            RepoError::CorruptRow(format!(
                "plan {} revision {} exceeds the storable range",
                revision.plan_id, revision.revision
            ))
        })?;
        let version = revision.row_version.to_stored().map_err(|e| {
            RepoError::CorruptRow(format!(
                "plan {} revision {}: {e}",
                revision.plan_id, revision.revision
            ))
        })?;
        let conn = self
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("conn: {e}")))?;
        let am = plan::ActiveModel {
            plan_id: Set(revision.plan_id.get()),
            revision: Set(number),
            tenant_id: Set(tenant_id),
            sku_id: Set(revision.sku_id),
            plan_tier: Set(revision.plan_tier.clone()),
            billing_cycle: Set(revision.billing_cycle.clone()),
            lifecycle_state: Set(revision.lifecycle_state.as_str().to_owned()),
            available_from: Set(revision.available_from),
            available_to: Set(revision.available_to),
            created_by: Set(revision.created_by),
            created_at_utc: Set(revision.created_at_utc),
            row_version: Set(version),
        };
        plan::Entity::insert(am.clone())
            .secure()
            .scope_with_model(scope, &am)
            .map_err(|e| RepoError::Db(format!("pricing_plan scope: {e}")))?
            .exec(&conn)
            .await
            .map_err(|e| RepoError::Db(format!("insert pricing_plan: {e}")))?;
        Ok(())
    }

    /// Name which conjunct of a failed compare-and-swap actually failed.
    ///
    /// One extra read, taken only on the refusal path. It costs nothing in the
    /// normal case and is the difference between an operator being told to
    /// retry and being told to stop retrying.
    async fn refuse(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        plan_id: PlanId,
        revision: u64,
        expected: RowVersion,
    ) -> RepoError {
        match self
            .find_revision(scope, tenant_id, plan_id, revision)
            .await
        {
            Err(err) => err,
            Ok(None) => not_found(plan_id, revision),
            Ok(Some(row)) if !row.lifecycle_state.is_content_mutable() => RepoError::NotDraft {
                subject: SUBJECT.to_owned(),
                id: revision_ref(plan_id, revision),
                state: row.lifecycle_state.to_string(),
            },
            Ok(Some(row)) => RepoError::StaleRowVersion {
                subject: SUBJECT.to_owned(),
                id: revision_ref(plan_id, revision),
                current: row.row_version.get(),
                submitted: expected.get(),
            },
        }
    }
}

/// The stored tokens the domain machine calls **current**.
///
/// Derived from [`LifecycleState::is_current_revision`] rather than written out,
/// so widening or narrowing that predicate moves this query with it. D-128
/// widened it once already, and the version of this list that did not move would
/// have been the one silently returning `None` for every retired plan.
fn current_tokens() -> Vec<&'static str> {
    LifecycleState::ALL
        .iter()
        .copied()
        .filter(|state| state.is_current_revision())
        .map(LifecycleState::as_str)
        .collect()
}

/// The conjunction that makes an UPDATE or DELETE a compare-and-swap on one
/// **draft** revision.
///
/// `None` when a number the caller supplied cannot be stored at all; every
/// caller then resolves it through [`PlanRepo::refuse`], because no row can hold
/// such a value and the truthful answer is the one an absent row already gets.
fn swap_guard(
    tenant_id: Uuid,
    plan_id: PlanId,
    revision: u64,
    expected: RowVersion,
) -> Option<Condition> {
    let number = stored_revision(revision)?;
    let version = expected.to_stored().ok()?;
    Some(
        Condition::all()
            .add(plan::Column::TenantId.eq(tenant_id))
            .add(plan::Column::PlanId.eq(plan_id.get()))
            .add(plan::Column::Revision.eq(number))
            .add(plan::Column::RowVersion.eq(version))
            .add(plan::Column::LifecycleState.eq(LifecycleState::Draft.as_str())),
    )
}

/// Render a revision number for its `bigint` column, `None` past the range the
/// column can hold. Checked rather than cast: a cast would turn an impossible
/// revision into a plausible one and quietly address a different row.
fn stored_revision(revision: u64) -> Option<i64> {
    i64::try_from(revision).ok()
}

/// The composite identity, as one reference a caller can read back in a URL.
fn revision_ref(plan_id: PlanId, revision: u64) -> String {
    format!("{plan_id}/{revision}")
}

/// The "absent, or not yours" refusal — deliberately one answer for both, so
/// the surface leaks no existence.
fn not_found(plan_id: PlanId, revision: u64) -> RepoError {
    RepoError::NotFound {
        subject: SUBJECT.to_owned(),
        id: revision_ref(plan_id, revision),
    }
}

/// Map a stored row to the domain value, at this boundary and nowhere else.
///
/// Both readings that can fail are **invariant breaches, not caller mistakes**:
/// `lifecycle_state` is `CHECK`-constrained to the four tokens the state machine
/// knows, and `revision` / `row_version` are `NOT NULL` columns that only ever
/// count up. A row that reads otherwise means something reached the table
/// outside this gear, which is why it surfaces as
/// [`RepoError::CorruptRow`] rather than as a not-found.
fn to_domain(row: plan::Model) -> Result<PlanRevision, RepoError> {
    let revision = u64::try_from(row.revision).map_err(|e| {
        RepoError::CorruptRow(format!(
            "pricing_plan row for plan {} holds revision {}: {e}",
            row.plan_id, row.revision
        ))
    })?;
    let lifecycle_state = LifecycleState::ALL
        .iter()
        .copied()
        .find(|state| state.as_str() == row.lifecycle_state)
        .ok_or_else(|| {
            RepoError::CorruptRow(format!(
                "pricing_plan revision {revision} of plan {} holds lifecycle_state {}",
                row.plan_id, row.lifecycle_state
            ))
        })?;
    let row_version = RowVersion::from_stored(row.row_version).map_err(|e| {
        RepoError::CorruptRow(format!(
            "pricing_plan revision {revision} of plan {}: {e}",
            row.plan_id
        ))
    })?;
    Ok(PlanRevision {
        plan_id: PlanId::new(row.plan_id),
        revision,
        sku_id: row.sku_id,
        plan_tier: row.plan_tier,
        billing_cycle: row.billing_cycle,
        available_from: row.available_from,
        available_to: row.available_to,
        lifecycle_state,
        created_by: row.created_by,
        created_at_utc: row.created_at_utc,
        row_version,
    })
}
