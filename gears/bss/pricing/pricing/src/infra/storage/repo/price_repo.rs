//! Repository for **draft** price rows and the tier bands that belong to them
//! (`pricing_price` + `pricing_price_tier_band`, `design/03-price-structure.md`
//! §5, `design/01-foundation.md` §4.1 / §4.3).
//!
//! A price row is not one table. Everything single-valued about it — the
//! amount, the package block, the evaluation policy — sits on the row, and only
//! the band set, being many-per-row, has a table of its own. So every operation
//! here writes **both** tables or neither, and the reasons are physical rather
//! than tidy:
//!
//! **A row and its bands land together.** Bands inserted before their parent are
//! rejected outright by the band table's structural-exclusivity trigger, which
//! reads the parent's `model_kind`; bands landing after a committed parent leave
//! a window in which a `graduated` row has no geometry at all, and the Slice-3
//! rules would judge it as a tiered row that prices nothing. One transaction
//! removes both states.
//!
//! **The bands go first on the way out.** The foreign key declares the default
//! `NO ACTION` on both backends — nothing cascades and nothing nulls — and sqlx
//! turns `foreign_keys` ON for `SQLite`, so deleting a draft row that still has
//! bands fails with an opaque constraint error.
//! `ON DELETE CASCADE` is *not* the fix: Postgres fires the child's row triggers
//! on a cascade and `SQLite` fires them only under `recursive_triggers`, so
//! cascading would make the append-only guard mean two different things on the
//! two backends — the one divergence the mirrored schema exists to prevent.
//! Deleting the children explicitly, inside the transaction, is what keeps the
//! guard identical.
//!
//! **A band's `tenant_id` comes from its parent row, never from the request.**
//! The foreign key covers `price_id` alone, so nothing in the schema stops a
//! band carrying a different tenant from the row it points at — and under
//! `SecureORM` such a band is invisible to its true owner while still joined by
//! `price_id` to their price. The value is therefore copied from the row this
//! repository just wrote, and no caller is offered the choice.
//!
//! **There is deliberately no band-level mutation.** Bands carry no entity tag
//! of their own, so a band edit that did not move the parent's `row_version`
//! would let two authors editing different bands of one draft both satisfy
//! `If-Match` and silently interleave — the outcome
//! `cpt-cf-bss-pricing-fr-concurrent-edit` forbids. Bands are written only
//! through [`PriceRepo::create_draft`], [`PriceRepo::update_draft`] and
//! [`PriceRepo::delete_draft`], and the band set is replaced wholesale inside
//! the same transaction as the row's compare-and-swap.
//!
//! **The compare-and-swap is one statement, and a failed swap gets three
//! answers.** Both are exactly `PlanRepo`'s shape and for exactly its reasons:
//! the `row_version + 1` bump rides inside the UPDATE that matches on the
//! version the caller read, and `rows_affected == 0` is resolved by one extra
//! read into [`RepoError::NotFound`], [`RepoError::NotDraft`] or
//! [`RepoError::StaleRowVersion`], because only one of those three is worth
//! retrying.
//!
//! **The draft-only predicate is enforced here as well as by the triggers.**
//! `pricing_price_tier_band` rejects INSERT, UPDATE *and* DELETE against a
//! non-draft parent. This repository only ever touches bands of a draft parent,
//! and does not lean on the trigger for the caller-facing refusal: the trigger's
//! answer is a raw database error carrying no state, and a caller that edited a
//! published row deserves to be told that, not that the store is broken. What
//! guarantees it is one read taken inside the transaction, before the first band
//! statement — [`mutable_draft`] — because both mutating paths now reach the
//! band table *before* the compare-and-swap has answered. The swap is still the
//! authority; the read only decides which sentence a doomed call gets.
//!
//! **Bands come back ordered by `from_qty`, ascending.** That is a read-side
//! guarantee and it is load-bearing. The band table is keyed
//! `(price_id, from_qty)` and carries no ordinal, so authoring order does not
//! survive a round trip; `inst-tb-order` was amended for exactly this after a
//! rule judged band geometry by the author's in-memory ordering and therefore
//! reached one verdict at save and another at publish, from the identical rule.
//!
//! **`PriceRow::charge_kind` is the key's, not a second column.** The row type
//! carries the axis because half the shape rules are a function of it, and
//! `price_row.rs` says as much — it is "not authored on the row itself". So the
//! store keeps exactly one copy, the scope key's, and a record read back always
//! agrees with its own key; a caller that hands the two different answers is
//! given the key's rather than a refusal, because there is no second value for
//! them to have disagreed about.
//!
//! **Superseding a published scope key is a different path.** It is the D-88
//! supersession unit — successor row, predecessor-window shorten and
//! successor-window schedule composed gap-free in one commit, setting
//! `supersedes_price_id` — and it is not implemented here and must not be
//! reached for by calling [`PriceRepo::create_draft`] on an occupied key. This
//! repository authors draft rows; the key an occupied published row holds is
//! refused, not taken over.

use std::collections::HashMap;

use chrono::{DateTime, TimeZone, Utc};
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait, Order, Value};
use serde_json::{Value as JsonValue, json};
use toolkit_db::secure::{
    AccessScope, DBRunner, DbConn, DbTx, SecureDeleteExt, SecureEntityExt, SecureInsertExt,
    SecureUpdateExt, TxError,
};
use toolkit_db::{DBProvider, DbError};
use uuid::Uuid;

use crate::domain::audit::{AuditAction, AuditStamp, AuditSubjectKind, subject_state};
use crate::domain::concurrency::RowVersion;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::money::{CurrencyCode, MinorAmount};
use crate::domain::price_record::{PriceContent, PriceRecord};
use crate::domain::price_row::{
    AggregationFunction, AggregationGranularity, BandTop, BillingGranularity, IncludedAllowance,
    ModelKind, PriceRow, QuantitySource, RolloverPolicy, TierAggregationWindow, TierBand,
    TierQualificationWindow, model_kind_wire,
};
use crate::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, PriceOverlay, Region, ScopeKey,
};
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::{price, price_tier_band};
use crate::infra::storage::repo::check_authored_instant;
use crate::infra::storage::repo::{NewAuditEntry, audit_repo};

/// The noun the authoring refusals name, so one subject word reaches the wire
/// from every method here.
const SUBJECT: &str = "price row";

// ---------------------------------------------------------------------------
// The stored token sets.
//
// Each list is the inverse of an `as_str()` the domain already owns, written
// out here rather than added to the domain because parsing a column was a
// storage concern and nothing above this boundary had a token to read. A
// variant added to one of these enums without a line here reads back as
// `CorruptRow` — loud, and never as a silently different value.
//
// **They are `pub` now, and the REST surface reads through them rather than
// declaring a second set.** The authoring routes parse the same tokens off a
// request that these parse off a column, and two tables would be two answers to
// "is `partner` a legal overlay" — free to disagree the day a variant lands in
// one of them. The lists stay here rather than moving onto the domain enums
// because moving them would put a *parse* on a type whose job is to render, and
// because these are the tables whose completeness the `CorruptRow` path already
// depends on.
// ---------------------------------------------------------------------------

/// Axis 4. One value today; the list exists so a stored `partner` overlay is
/// refused rather than quietly read as `base`.
pub const PRICE_OVERLAYS: &[PriceOverlay] = &[PriceOverlay::Base];
/// Axis 6. All **three** normative classes: a list that carried only the two
/// the cutover machinery uses would read a stored `new_subscriptions_only` row
/// — the class PRD AC #59 and W3 both name — as an invariant breach.
pub const PRICE_ELIGIBILITIES: &[PriceEligibility] = &[
    PriceEligibility::AllSubscriptions,
    PriceEligibility::NewSubscriptionsOnly,
    PriceEligibility::ExistingGrandfathered,
];
/// Axis 7.
pub const CHARGE_KINDS: &[ChargeKind] = &[
    ChargeKind::Recurring,
    ChargeKind::Usage,
    ChargeKind::OneTime,
    ChargeKind::OneTimeSetup,
];
/// Where a non-usage `per_unit` row's quantity comes from.
pub const QUANTITY_SOURCES: &[QuantitySource] = &[
    QuantitySource::SubscriptionSeatCount,
    QuantitySource::Manual,
];
/// The billable-unit quantization.
pub const BILLING_GRANULARITIES: &[BillingGranularity] = &[
    BillingGranularity::PerSecond,
    BillingGranularity::PerMinute,
    BillingGranularity::PerHour,
    BillingGranularity::PerDay,
    BillingGranularity::WholeUnit,
];
/// The counter-reset window.
pub const TIER_AGGREGATION_WINDOWS: &[TierAggregationWindow] = &[
    TierAggregationWindow::CalendarMonth,
    TierAggregationWindow::InvoicePeriod,
    TierAggregationWindow::SubscriptionLifetime,
    TierAggregationWindow::PerEvent,
];
/// The D-40 tier-qualification window.
pub const TIER_QUALIFICATION_WINDOWS: &[TierQualificationWindow] = &[
    TierQualificationWindow::Current,
    TierQualificationWindow::TrailingPeriod,
];
/// How the in-window `Q` is derived (D-44).
pub const AGGREGATION_FUNCTIONS: &[AggregationFunction] = &[
    AggregationFunction::Sum,
    AggregationFunction::Peak,
    AggregationFunction::TimeWeighted,
];
/// The granule a non-`sum` window is cut into (D-44).
pub const AGGREGATION_GRANULARITIES: &[AggregationGranularity] =
    &[AggregationGranularity::Hour, AggregationGranularity::Day];
/// What happens to an unused allowance (D-45).
pub const ROLLOVER_POLICIES: &[RolloverPolicy] = &[RolloverPolicy::None, RolloverPolicy::Carry];

/// Everything a **new** draft price row needs.
///
/// `price_id` and `created_at_utc` are caller-supplied for the reasons
/// [`super::NewPlanDraft`] states: the surface that mints an id has to return it
/// before the row is durable, and the catalog never self-originates a row, so
/// the authoring instant belongs to the request rather than to whichever
/// database node evaluated `now()`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewPriceDraft {
    /// The row being created.
    pub price_id: Uuid,
    /// The eight axes it is filed under.
    pub scope_key: ScopeKey,
    /// What the row says.
    pub content: PriceContent,
    /// Pseudonymous principal id of the authoring actor.
    pub created_by: Uuid,
    /// When the request was authored, UTC.
    pub created_at_utc: DateTime<Utc>,
}

/// `SeaORM`-backed repository over draft price rows and their tier bands.
#[derive(Clone)]
pub struct PriceRepo {
    db: DBProvider<DbError>,
}

impl PriceRepo {
    /// Build over one database provider.
    #[must_use]
    pub fn new(db: DBProvider<DbError>) -> Self {
        Self { db }
    }

    /// Create a price row in `draft` at `row_version = 0`, with its band set,
    /// in one transaction.
    ///
    /// The canonical scope key is checked for an occupant first. `inst-pr-return`
    /// (D-21) puts scope-key duplication among the **row-local** checks that run
    /// at save *and* re-run at publish, and the check has to happen here because
    /// the database's own `uq_pricing_price_scope_key_current` is partial over
    /// `lifecycle_state = 'published'` — it cannot see a second **draft** on one
    /// key, which is the ambiguity publish would fail on, discovered a round trip
    /// earlier.
    ///
    /// This is not the way to reprice an occupied key. That is the D-88
    /// supersession unit; see the module doc.
    ///
    /// # The race, and why its loser is not told `DUPLICATE_SCOPE_KEY`
    ///
    /// The check above is a **read**, so two concurrent creators can both pass
    /// it. What decides the winner is `uq_pricing_price_scope_key_draft`, the
    /// partial `UNIQUE` over `draft` — added for exactly this, because the
    /// published index is partial over `published` and cannot see a draft at
    /// all. The loser's INSERT therefore comes back as a driver error and
    /// reaches the caller as [`RepoError::Db`], i.e. a 500, where the identical
    /// collision caught by the check is a 409.
    ///
    /// That narrowing is **owed to the surface layer and is not paid here**.
    /// Recognizing the violation means introspecting a backend-specific error
    /// class, which is precisely the coupling [`RepoError`]'s own doc refuses —
    /// a variant per SQL error class would put the database's vocabulary into
    /// the gear's error surface — and the constraint name has to be mapped once
    /// for both backends, which only a layer that knows the backend can do.
    /// [`super::PlanRepo::open_revision`] carries the identical shape and the
    /// identical debt against `uq_pricing_plan_open_draft`, so the two read the
    /// same way.
    ///
    /// # Errors
    /// [`RepoError::DuplicateScopeKey`] naming the key and the draft or
    /// published row already on it;
    /// [`RepoError::GrandfatherHorizonOffClass`] when a grandfathering horizon
    /// is authored on a key that is not an `existing_grandfathered` generation;
    /// [`RepoError::ValueOutOfRange`] when an
    /// authored quantity is outside the range its column holds;
    /// [`RepoError::Db`] on a scope or storage failure — which **includes
    /// losing the race above**, and the band table's own refusal of bands on a
    /// kind that may not carry them; [`RepoError::CorruptRow`] only for a
    /// `row_version` that cannot be stored, which a created draft's `0` never
    /// is.
    pub async fn create_draft(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        draft: NewPriceDraft,
    ) -> Result<PriceRecord, RepoError> {
        // Rendered and value-checked before the transaction opens: a quantity no
        // column can hold, or a horizon off its class, is the caller's mistake,
        // and there is no reason to hold a transaction open to find it. That is
        // also why the split below is `prepare_draft` + `insert_prepared` rather
        // than one runner-taking body — `create_draft_on` runs both, which the
        // seam needs, and this path keeps the refusal ahead of the BEGIN.
        let prepared = prepare_draft(tenant_id, draft)?;

        let scope = scope.clone();
        let (_, outcome) = self
            .db
            .db()
            .in_transaction::<PriceRecord, RepoError, _>(move |txn| {
                Box::pin(async move { insert_prepared(txn, &scope, tenant_id, prepared).await })
            })
            .await;
        outcome.map_err(tx_failure)
    }

    /// Read one price row and its bands.
    ///
    /// Bands come back ordered by `from_qty` ascending — see the module doc; the
    /// order is a read-side guarantee, not a stored fact. SQL-level BOLA: a
    /// foreign tenant's row yields `None`.
    ///
    /// # Errors
    /// [`RepoError::Db`] on a scope or storage failure;
    /// [`RepoError::CorruptRow`] when the stored row cannot be read as the
    /// domain value its columns are `CHECK`-constrained to hold.
    pub async fn find(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        price_id: Uuid,
    ) -> Result<Option<PriceRecord>, RepoError> {
        let conn = self.conn()?;
        load_record(&conn, scope, tenant_id, price_id).await
    }

    /// Read a plan's price rows in the given lifecycle states.
    ///
    /// Ordered by `price_id` ascending — stated here because the order is part
    /// of the contract and not an accident of the plan index. It is the same
    /// order on both backends: Postgres compares `uuid` byte-wise and `SQLite`
    /// compares the canonical lowercase hyphenated text, and hex digits sort in
    /// ASCII exactly as the bytes they spell do. D-125's cursor contract is a REST
    /// concern and it now **has** its list surface
    /// (`api::rest::prices::list_plan_prices`), which walks
    /// [`PriceRepo::list_for_plan_page`] over exactly this order. This method
    /// keeps its unbounded form for the callers that genuinely want the whole
    /// set - the publish assembler is one - and is not the paginated one.
    ///
    /// An **empty** `states` selects nothing. Reading it as "every state" would
    /// hand a caller whose filter computed to nothing the whole catalog, which
    /// is the one answer it certainly did not ask for.
    ///
    /// # Errors
    /// [`RepoError::Db`] on a scope or storage failure;
    /// [`RepoError::CorruptRow`] when a stored row cannot be read as the domain
    /// value its columns are `CHECK`-constrained to hold.
    pub async fn list_for_plan(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        plan_id: PlanId,
        states: &[LifecycleState],
    ) -> Result<Vec<PriceRecord>, RepoError> {
        load_for_plan(&self.conn()?, scope, tenant_id, plan_id, states).await
    }

    /// One **page** of a plan's price rows, resuming strictly after `after`.
    ///
    /// [`PriceRepo::list_for_plan`] returns the whole `Vec` with no bound, and
    /// paginating that in memory would be a lie about D-125's contract: the
    /// promise is a walk that never skips or duplicates a row at or before the
    /// cursor, and a walk that read everything first would hold the whole plan
    /// in memory to serve a hundred rows of it. This is the keyset form, on the
    /// `price_id ASC` total order [`PriceRepo::list_for_plan`]'s doc already
    /// declares as part of its contract — the same order on both backends,
    /// because Postgres compares `uuid` byte-wise and `SQLite` compares the
    /// canonical lowercase hyphenated text, and hex digits sort in ASCII exactly
    /// as the bytes they spell do.
    ///
    /// **The caller asks for `limit + 1`** to learn whether another page exists
    /// without a second query; deciding `next_cursor` is the surface's, since
    /// only it knows the page size it promised.
    ///
    /// # Errors
    /// [`RepoError::Db`] on a scope or storage failure;
    /// [`RepoError::CorruptRow`] when a stored row cannot be read as the domain
    /// value its columns are `CHECK`-constrained to hold.
    pub async fn list_for_plan_page(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        plan_id: PlanId,
        states: &[LifecycleState],
        after: Option<Uuid>,
        limit: u64,
    ) -> Result<Vec<PriceRecord>, RepoError> {
        load_page(
            &self.conn()?,
            scope,
            tenant_id,
            plan_id,
            states,
            after,
            Some(limit),
        )
        .await
    }

    /// Replace an open draft's content, and its band set, under the caller's
    /// row version.
    ///
    /// One statement does the row: the content columns, the `row_version + 1`
    /// bump, and the conjunction that makes it a compare-and-swap — tenant, row,
    /// the submitted version, and `lifecycle_state = 'draft'`. The band set is
    /// replaced **wholesale** in the same transaction. It is replaced and not
    /// merged because bands are a set with no addressable members, and a partial
    /// band update has no geometry the Slice-3 rules could evaluate.
    ///
    /// **The band set goes first, and the row's own guard is the reason.**
    /// `trg_pricing_price_tier_band_parent_kind` refuses a row that still
    /// carries bands becoming a kind that carries none, so an edit turning a
    /// banded `graduated` row into a bandless `flat` one — an ordinary
    /// authoring move — would be refused by the store if the row were moved
    /// while the old bands were still standing. Nothing is lost by the order:
    /// the compare-and-swap is still the authority and a failure at any point
    /// rolls the whole transaction back, band set included. What the order costs
    /// is one read, taken so the frozen and absent cases are still answered by
    /// name: reaching the band DELETE with a published parent would have the
    /// band table's own trigger answer with a raw database error, and a caller
    /// who edited a published row deserves to be told that rather than that the
    /// store is broken. [`PriceRepo::delete_draft`] carries the identical read
    /// for the identical reason.
    ///
    /// The scope key is **not** part of the content and cannot be moved here.
    /// A row's key decides which duplicate it is, which supersession chain it
    /// joins and which window covers it; moving it would need the create-time
    /// duplicate check re-run against a different key, which is exactly what
    /// deleting the draft and authoring another one is.
    ///
    /// `content.row.charge_kind` is likewise **ignored**, and for the same
    /// reason [`PriceRepo::create_draft`] states: the axis is the key's, the
    /// copy on `PriceRow` is a convenience the shape rules read, and the store
    /// keeps exactly one copy. A caller that submits a different one is given
    /// the key's rather than a refusal, because there is no second stored value
    /// for them to have disagreed about — and the update path could not honour
    /// a different one anyway without moving the key it may not move.
    ///
    /// # Errors
    /// [`RepoError::NotFound`] when no such row is visible to `scope`;
    /// [`RepoError::NotDraft`] when it is visible but frozen;
    /// [`RepoError::StaleRowVersion`] carrying both versions when the submitted
    /// one is not current;
    /// [`RepoError::GrandfatherHorizonOffClass`] when a grandfathering horizon
    /// is submitted for a row whose key is not an `existing_grandfathered`
    /// generation; [`RepoError::ValueOutOfRange`] when an authored
    /// quantity is outside the range its column holds; [`RepoError::Db`] on a
    /// scope or storage failure, including the band table's refusal of a band
    /// set on a kind that may not carry one; [`RepoError::CorruptRow`] when the
    /// updated row reads back unusable.
    pub async fn update_draft(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        price_id: Uuid,
        expected: RowVersion,
        content: PriceContent,
        stamp: AuditStamp,
    ) -> Result<PriceRecord, RepoError> {
        let horizon = content.grandfather_until;
        // Before the row is even read: an instant the catalog cannot compare is
        // refused wherever it arrives, and this path can reject it without a
        // round trip.
        check_authored_instant("grandfatherUntil", horizon)?;
        let assignments = content_assignments(&content_model(&content)?);
        let bands = band_models(tenant_id, price_id, &content.row.bands)?;
        let Some(guard) = swap_guard(tenant_id, price_id, expected) else {
            let conn = self.conn()?;
            return Err(refuse(&conn, scope, tenant_id, price_id, expected).await);
        };

        let scope = scope.clone();
        let (_, outcome) = self
            .db
            .db()
            .in_transaction::<PriceRecord, RepoError, _>(move |txn| {
                Box::pin(async move {
                    let Some(row) =
                        mutable_draft(txn, &scope, tenant_id, price_id, expected).await?
                    else {
                        return Err(refuse(txn, &scope, tenant_id, price_id, expected).await);
                    };
                    // The row's class cannot move on an update, so the stored
                    // one is the one the submitted horizon has to pair with.
                    check_grandfather_horizon(horizon, read_eligibility(&row)?)?;
                    delete_bands(txn, &scope, tenant_id, price_id).await?;
                    let mut update = price::Entity::update_many().secure().scope_with(&scope);
                    for (column, value) in assignments {
                        update = update.col_expr(column, Expr::value(value));
                    }
                    let result = update
                        .col_expr(
                            price::Column::RowVersion,
                            Expr::col(price::Column::RowVersion).add(1_i64),
                        )
                        .filter(guard)
                        .exec(txn)
                        .await
                        .map_err(|e| RepoError::Db(format!("update price draft: {e}")))?;
                    // The read above is not the guard — a concurrent publish can
                    // land between it and this statement — so a swap that
                    // matched nothing is still resolved, and the band delete it
                    // has already done is undone by the rollback.
                    if result.rows_affected == 0 {
                        return Err(refuse(txn, &scope, tenant_id, price_id, expected).await);
                    }
                    insert_bands(txn, &scope, bands).await?;
                    let updated = load_record(txn, &scope, tenant_id, price_id)
                        .await?
                        .ok_or_else(|| not_found(price_id))?;
                    record_price_mutation(
                        txn,
                        &scope,
                        tenant_id,
                        &updated,
                        AuditAction::Update,
                        Some(expected),
                        stamp,
                    )
                    .await?;
                    Ok(updated)
                })
            })
            .await;
        outcome.map_err(tx_failure)
    }

    /// Delete an open draft row and its bands, under the caller's row version.
    ///
    /// Only a never-published `draft` is deletable (§4.3). The bands go first —
    /// the foreign key neither cascades nor nulls — and the row's
    /// compare-and-swap is still
    /// the authority: the read that precedes the band delete exists so a caller
    /// aiming at a published row is told so *before* the band table's own
    /// trigger answers with a raw database error, and so that nothing is
    /// deleted on the way to a refusal. Losing the row to a concurrent writer in
    /// the window between that read and the swap rolls the whole transaction
    /// back, band set included.
    ///
    /// # Errors
    /// [`RepoError::NotFound`] when no such row is visible to `scope`;
    /// [`RepoError::NotDraft`] when it is visible but frozen;
    /// [`RepoError::StaleRowVersion`] carrying both versions when the submitted
    /// one is not current; [`RepoError::Db`] on a scope or storage failure;
    /// [`RepoError::CorruptRow`] when the row reads back unusable.
    pub async fn delete_draft(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        price_id: Uuid,
        expected: RowVersion,
        stamp: AuditStamp,
    ) -> Result<(), RepoError> {
        let Some(guard) = swap_guard(tenant_id, price_id, expected) else {
            let conn = self.conn()?;
            return Err(refuse(&conn, scope, tenant_id, price_id, expected).await);
        };

        let scope = scope.clone();
        let (_, outcome) = self
            .db
            .db()
            .in_transaction::<(), RepoError, _>(move |txn| {
                Box::pin(async move {
                    let Some(_) = mutable_draft(txn, &scope, tenant_id, price_id, expected).await?
                    else {
                        return Err(refuse(txn, &scope, tenant_id, price_id, expected).await);
                    };
                    // Read whole before the row goes, because the record names
                    // the plan whose chain it extends and that fact lives on the
                    // row being deleted.
                    let doomed = load_record(txn, &scope, tenant_id, price_id)
                        .await?
                        .ok_or_else(|| not_found(price_id))?;
                    delete_bands(txn, &scope, tenant_id, price_id).await?;
                    let result = price::Entity::delete_many()
                        .secure()
                        .scope_with(&scope)
                        .filter(guard)
                        .exec(txn)
                        .await
                        .map_err(|e| RepoError::Db(format!("delete price draft: {e}")))?;
                    if result.rows_affected == 0 {
                        return Err(refuse(txn, &scope, tenant_id, price_id, expected).await);
                    }
                    record_price_mutation(
                        txn,
                        &scope,
                        tenant_id,
                        &doomed,
                        AuditAction::Delete,
                        Some(expected),
                        stamp,
                    )
                    .await
                })
            })
            .await;
        outcome.map_err(tx_failure)
    }

    /// The non-transactional runner, named once so four read paths spell the
    /// failure the same way.
    fn conn(&self) -> Result<DbConn<'_>, RepoError> {
        self.db
            .conn()
            .map_err(|e| RepoError::Db(format!("conn: {e}")))
    }
}

// ---------------------------------------------------------------------------
// Transaction plumbing.
// ---------------------------------------------------------------------------

/// Flatten the transaction wrapper back into this repository's vocabulary.
///
/// The body's error type is [`RepoError`] rather than the driver's precisely so
/// a typed refusal survives the rollback: a `create_draft` that met an occupied
/// key has to reach the caller as `DUPLICATE_SCOPE_KEY`, not as "the store
/// failed". What arrives as infrastructure is only what happens outside the
/// body — beginning the transaction, and committing it.
fn tx_failure(err: TxError<RepoError>) -> RepoError {
    err.into_domain(|infra| RepoError::Db(format!("price draft transaction: {infra}")))
}

/// Publish **exactly the validated price rows**, inside the caller's
/// transaction, returning the ids that moved.
///
/// # The set is an argument, and that is the whole guard
///
/// `validated` is `(price_id, row_version)` for every `draft` row of the subject
/// the rule set actually judged. This function publishes those rows at those
/// versions and nothing else — it does **not** re-derive "the plan's draft rows"
/// for itself.
///
/// It used to, and that was a defect. The publish commit assembles its subject,
/// runs the rules, then makes a network round-trip to the `CatalogVersion`
/// registry before it flips anything. `in_transaction` opens the engine default,
/// which on Postgres is READ COMMITTED, so **every statement takes a fresh
/// snapshot**: a `create_draft` committing inside that window inserts a row the
/// rule set never saw, and a `update_draft` committing inside it changes the
/// content of a row the rule set passed. A re-derived set would publish both.
/// The outcome is a consumer-visible `published` row that never passed §4.2's
/// rule set — one that could, for instance, carry no `roundingPolicyRef` on a
/// tenant with no default, which is precisely what the Foundation's rounding
/// rule exists to block.
///
/// **D-141 is what closes it.** Every price row carries its own entity tag
/// exactly so a per-row conflict means something, and this is that meaning: a
/// row whose content moved between validation and commit no longer stands at
/// the version the subject was assembled from, so it does not match and the
/// publish is refused **naming the row**. A newly inserted row is not in the
/// set, so it stays `draft` and publishes with the next revision — correct by
/// construction, because nothing validated it.
///
/// The alternative was `SERIALIZABLE`, and it was rejected: it would hold
/// predicate locks across the registry's network round-trip, which is the one
/// cost the commit's ordering decision was chosen to bound. See
/// [`PublishService::commit`](crate::infra::publish::PublishService::commit)'s
/// CONTRACT paragraph, which now carries the full argument.
///
/// # Why the pre-read, and why it is not the guard
///
/// The version conjunction is checked **before** the UPDATE as well as by the
/// UPDATE's own row count, and the order is forced: after the UPDATE this
/// transaction reads its own writes, so a row that legitimately moved to
/// `published` is indistinguishable from one that never matched. Taken first,
/// the read names which conjunct failed — absent, frozen, or stale — the way
/// every other compare-and-swap in this file does. The **count** is still the
/// guard, and it covers the one window the read cannot: a concurrent commit
/// landing between two adjacent statements.
///
/// # The rest, unchanged
///
/// **One UPDATE, not a per-row negotiation**, and the two partial unique indexes
/// are why that is safe. The draft-plane index (D-148,
/// `WHERE lifecycle_state = 'draft'`) releases a key as its row leaves `draft`
/// and the published-plane index claims it as the row arrives, both inside this
/// statement. [`PriceRepo::create_draft`] already refuses a scope key occupied by
/// a `draft` **or** a `published` row, so a plan's draft rows sit on keys nothing
/// else holds; a concurrent publish of a *second* plan cannot collide because
/// every key carries `plan_id`.
///
/// **`draft -> published` is the row's only edge out of `draft`** and this is the
/// sanctioned producer of it. The predicate is restated here rather than left to
/// the table because §4.3 enforces immutability twice and the physical guard's
/// answer is a raw driver error carrying no state — and the physical half now
/// exists on this table too: D-153's draft-plane whitelist was missing from
/// `pricing_price` (the trigger returned early for a draft row) and was amended
/// into `m20260802_000002` on both backends while this guard was being written.
///
/// The `row_version` is deliberately **not** bumped. §3.7 says the tag freezes
/// with the content it names (D-141) and this flip changes no content; after it
/// the row is frozen and no caller-driven mutation exists for a precondition to
/// guard, so a bump would move an entity tag under a representation nobody can
/// write to.
///
/// # Errors
/// [`RepoError::NotFound`] when a validated row is no longer visible to `scope`;
/// [`RepoError::NotDraft`] when one has left `draft` since it was validated;
/// [`RepoError::StaleRowVersion`] carrying both versions when one's content
/// moved since it was validated — the refusal this whole shape exists to
/// produce; [`RepoError::Db`] on a scope or storage failure, **including** a
/// concurrent commit landing between the pre-read and the UPDATE, reported as
/// the row-count mismatch it is; [`RepoError::CorruptRow`] when a stored row
/// cannot be read as the domain value its columns are `CHECK`-constrained to
/// hold.
pub async fn publish_rows(
    txn: &DbTx<'_>,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: PlanId,
    validated: &[(Uuid, RowVersion)],
) -> Result<Vec<Uuid>, RepoError> {
    if validated.is_empty() {
        return Ok(Vec::new());
    }

    // Refused before anything flips. See the doc: this is not the guard, it is
    // the only place a precise refusal can still be produced.
    //
    // **One query for the whole set, not one per row.** §14's soft cap is 500
    // price rows per plan, and a per-row read would put five hundred sequential
    // round-trips inside a transaction that is already holding the registry's
    // network call open. The per-row *detail* comes from comparing what the read
    // returned against what was validated, not from having read them one at a
    // time — so the refusal below is exactly as precise, and is still taken
    // through [`refuse`] so its three answers are spelled in one place.
    let held: HashMap<Uuid, price::Model> =
        load_rows(txn, scope, tenant_id, validated.iter().map(|(id, _)| *id))
            .await?
            .into_iter()
            .map(|row| (row.price_id, row))
            .collect();
    for (price_id, expected) in validated {
        if !stands_at(held.get(price_id), *expected) {
            return Err(refuse(txn, scope, tenant_id, *price_id, *expected).await);
        }
    }

    let mut identities = Condition::any();
    for (price_id, expected) in validated {
        let stored = expected
            .to_stored()
            .map_err(|e| RepoError::CorruptRow(format!("price row {price_id} row version: {e}")))?;
        identities = identities.add(
            Condition::all()
                .add(price::Column::PriceId.eq(*price_id))
                .add(price::Column::RowVersion.eq(stored)),
        );
    }

    let result = price::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            price::Column::LifecycleState,
            Expr::value(LifecycleState::Published.as_str()),
        )
        .filter(plan_rows_in_state(tenant_id, plan_id, LifecycleState::Draft).add(identities))
        .exec(txn)
        .await
        .map_err(|e| RepoError::Db(format!("publish plan price rows: {e}")))?;

    // Checked rather than cast: a cast that wrapped would report a mismatched
    // set as a matching one, which is the one answer this comparison must never
    // be able to give.
    let expected_count = u64::try_from(validated.len()).map_err(|_| {
        RepoError::CorruptRow(format!(
            "plan {plan_id} carries {} validated price rows, more than a row count can hold",
            validated.len()
        ))
    })?;
    if result.rows_affected != expected_count {
        return Err(RepoError::Db(format!(
            "plan {plan_id} validated {expected_count} price rows and {} moved; \
             a concurrent commit changed one between the check and the flip",
            result.rows_affected
        )));
    }
    Ok(validated.iter().map(|(price_id, _)| *price_id).collect())
}

/// Read a set of rows by id, scoped, in one query.
///
/// Deliberately **not** filtered by `plan_id`: it mirrors [`mutable_draft`]'s
/// predicate exactly, which is tenant plus identity. A row of another plan of
/// the same tenant therefore passes this read and is excluded by the UPDATE's
/// own plan filter — a mismatch the count assertion catches, which is the one
/// arm of that assertion reachable without concurrency and is what
/// `a_validated_row_of_another_plan_is_caught_by_the_count` exercises.
async fn load_rows(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    ids: impl Iterator<Item = Uuid>,
) -> Result<Vec<price::Model>, RepoError> {
    price::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(price::Column::TenantId.eq(tenant_id))
                .add(price::Column::PriceId.is_in(ids)),
        )
        .all(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read validated price rows: {e}")))
}

/// Is this row an editable `draft` standing at exactly `expected`?
///
/// The batched half of [`mutable_draft`]'s question, over a row already in hand.
/// A reading that fails — a `lifecycle_state` or `row_version` the column's
/// constraints forbid — answers `false` rather than erroring, because the
/// caller's next move is [`refuse`], which re-reads the row and surfaces the
/// [`RepoError::CorruptRow`] with the subject attached.
fn stands_at(row: Option<&price::Model>, expected: RowVersion) -> bool {
    let Some(row) = row else {
        return false;
    };
    let Ok(state) = read_lifecycle(&row.lifecycle_state) else {
        return false;
    };
    let Ok(current) = read_row_version(row.price_id, row.row_version) else {
        return false;
    };
    state.is_content_mutable() && current == expected
}

/// A plan's rows in one lifecycle state, as a filter. One spelling, so the read
/// that names the ids and the UPDATE that moves them cannot address different
/// sets.
fn plan_rows_in_state(tenant_id: Uuid, plan_id: PlanId, state: LifecycleState) -> Condition {
    Condition::all()
        .add(price::Column::TenantId.eq(tenant_id))
        .add(price::Column::PlanId.eq(plan_id.get()))
        .add(price::Column::LifecycleState.eq(state.as_str()))
}

/// A plan's price rows in the given lifecycle states, through whichever runner
/// the caller holds.
///
/// Public and runner-taking because the publish path assembles its subject
/// **twice** — once as the §4.2 step-2 pre-check and again inside the commit
/// transaction, where `Db::conn()` is refused by the toolkit's
/// transaction-bypass guard. One implementation with two entry points: a second
/// query is a second thing the two runs could disagree about, and that
/// disagreement is what §4.2's re-validation exists to detect rather than to
/// manufacture.
///
/// Ordered by `price_id` ascending, and an **empty** `states` selects nothing —
/// both are [`PriceRepo::list_for_plan`]'s contract and its doc carries the
/// argument.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure;
/// [`RepoError::CorruptRow`] when a stored row cannot be read as the domain
/// value its columns are `CHECK`-constrained to hold.
pub async fn load_for_plan(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: PlanId,
    states: &[LifecycleState],
) -> Result<Vec<PriceRecord>, RepoError> {
    load_page(runner, scope, tenant_id, plan_id, states, None, None).await
}

/// The keyset walk under [`load_for_plan`] and [`PriceRepo::list_for_plan_page`].
///
/// `after` resumes **strictly after** a `price_id`, and `limit` bounds the page.
/// Both ride the `price_id ASC` total order [`PriceRepo::list_for_plan`]'s
/// contract already promises on both backends, which is exactly what makes a
/// keyset walk possible: an offset over an append-only store names a different
/// row every time somebody inserts ahead of it (D-125 forbids one for that
/// reason).
async fn load_page(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: PlanId,
    states: &[LifecycleState],
    after: Option<Uuid>,
    limit: Option<u64>,
) -> Result<Vec<PriceRecord>, RepoError> {
    if states.is_empty() {
        return Ok(Vec::new());
    }
    let tokens: Vec<&str> = states.iter().copied().map(LifecycleState::as_str).collect();
    let mut filter = Condition::all()
        .add(price::Column::TenantId.eq(tenant_id))
        .add(price::Column::PlanId.eq(plan_id.get()))
        .add(price::Column::LifecycleState.is_in(tokens));
    if let Some(cursor) = after {
        filter = filter.add(price::Column::PriceId.gt(cursor));
    }
    let mut query = price::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(filter)
        .order_by(price::Column::PriceId, Order::Asc);
    if let Some(limit) = limit {
        query = query.limit(limit);
    }
    let rows = query
        .all(runner)
        .await
        .map_err(|e| RepoError::Db(format!("list plan price rows: {e}")))?;

    // One band query for the whole page rather than one per row: the bands
    // arrive already sorted, so grouping preserves the `from_qty` order the
    // read-side guarantee promises.
    let ids: Vec<Uuid> = rows.iter().map(|row| row.price_id).collect();
    let mut grouped: HashMap<Uuid, Vec<price_tier_band::Model>> = HashMap::new();
    if !ids.is_empty() {
        let bands = price_tier_band::Entity::find()
            .secure()
            .scope_with(scope)
            .filter(
                Condition::all()
                    .add(price_tier_band::Column::TenantId.eq(tenant_id))
                    .add(price_tier_band::Column::PriceId.is_in(ids)),
            )
            .order_by(price_tier_band::Column::PriceId, Order::Asc)
            .order_by(price_tier_band::Column::FromQty, Order::Asc)
            .all(runner)
            .await
            .map_err(|e| RepoError::Db(format!("list plan price bands: {e}")))?;
        for band in bands {
            grouped.entry(band.price_id).or_default().push(band);
        }
    }

    rows.iter()
        .map(|row| {
            let bands = grouped.remove(&row.price_id).unwrap_or_default();
            to_record(row, &bands)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Statements.
// ---------------------------------------------------------------------------

/// Read one row by identity, scoped.
async fn load_row(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    price_id: Uuid,
) -> Result<Option<price::Model>, RepoError> {
    price::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(price::Column::TenantId.eq(tenant_id))
                .add(price::Column::PriceId.eq(price_id)),
        )
        .one(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read price row: {e}")))
}

/// Read one row's bands, ascending by lower bound. See the module doc for why
/// the order is here and not left to the caller.
async fn load_bands(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    price_id: Uuid,
) -> Result<Vec<price_tier_band::Model>, RepoError> {
    price_tier_band::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(price_tier_band::Column::TenantId.eq(tenant_id))
                .add(price_tier_band::Column::PriceId.eq(price_id)),
        )
        .order_by(price_tier_band::Column::FromQty, Order::Asc)
        .all(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read price tier bands: {e}")))
}

/// Read one whole record — the row and its bands — or nothing.
async fn load_record(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    price_id: Uuid,
) -> Result<Option<PriceRecord>, RepoError> {
    let Some(row) = load_row(runner, scope, tenant_id, price_id).await? else {
        return Ok(None);
    };
    let bands = load_bands(runner, scope, tenant_id, price_id).await?;
    to_record(&row, &bands).map(Some)
}

/// The draft or published row already on `key`, if there is one.
///
/// A `superseded` row is deliberately not an occupant: it is history and is not
/// the current row on its key (§4.3), so refusing a new draft because of one
/// would make a key unusable forever after its first reprice. It is also the
/// only non-occupant state there is — the price-row state machine
/// (`03-price-structure.md` §4) has three states and `retired` is not one of
/// them, which `chk_pricing_price_lifecycle_state` now says out loud.
///
/// A key can hold a draft **and** a published row at once — the partial `UNIQUE`
/// only forbids two published ones, and the D-88 supersession unit will reach
/// that state by another path. Both block a new draft equally, so the query
/// takes the lowest `price_id` rather than whichever row the plan index happened
/// to reach first: a refusal that named a different row on each attempt would
/// send an author looking in a different place each time.
async fn find_key_occupant(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    key: &ScopeKey,
) -> Result<Option<price::Model>, RepoError> {
    price::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            scope_key_filter(tenant_id, key).add(price::Column::LifecycleState.is_in([
                LifecycleState::Draft.as_str(),
                LifecycleState::Published.as_str(),
            ])),
        )
        .order_by(price::Column::PriceId, Order::Asc)
        .one(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read scope-key occupant: {e}")))
}

/// Write the price row itself.
async fn insert_price(
    runner: &impl DBRunner,
    scope: &AccessScope,
    row: price::ActiveModel,
) -> Result<(), RepoError> {
    price::Entity::insert(row.clone())
        .secure()
        .scope_with_model(scope, &row)
        .map_err(|e| RepoError::Db(format!("pricing_price scope: {e}")))?
        .exec(runner)
        .await
        .map_err(|e| RepoError::Db(format!("insert pricing_price: {e}")))?;
    Ok(())
}

/// Write a band set, one row at a time.
///
/// One statement per band rather than a multi-row insert, because
/// `scope_with_model` validates the tenant of the `ActiveModel` it is given and
/// that validation is the second half of the tenant rule the module doc states:
/// the value is copied from the parent, and then checked against the caller's
/// scope. A band set is a handful of rows, so the cost of being checked is a
/// handful of statements.
async fn insert_bands(
    runner: &impl DBRunner,
    scope: &AccessScope,
    bands: Vec<price_tier_band::ActiveModel>,
) -> Result<(), RepoError> {
    for band in bands {
        price_tier_band::Entity::insert(band.clone())
            .secure()
            .scope_with_model(scope, &band)
            .map_err(|e| RepoError::Db(format!("pricing_price_tier_band scope: {e}")))?
            .exec(runner)
            .await
            .map_err(|e| RepoError::Db(format!("insert pricing_price_tier_band: {e}")))?;
    }
    Ok(())
}

/// Drop a row's whole band set.
async fn delete_bands(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    price_id: Uuid,
) -> Result<(), RepoError> {
    price_tier_band::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(price_tier_band::Column::TenantId.eq(tenant_id))
                .add(price_tier_band::Column::PriceId.eq(price_id)),
        )
        .exec(runner)
        .await
        .map_err(|e| RepoError::Db(format!("delete price tier bands: {e}")))?;
    Ok(())
}

/// The row, when it is a draft standing at exactly `expected`; `None` when it
/// is anything else, including absent.
///
/// Asked so [`PriceRepo::update_draft`] and [`PriceRepo::delete_draft`] can
/// refuse *before* they touch a band: both replace or remove the band set, and
/// a band statement against a frozen parent is answered by the band table's own
/// trigger with a raw database error carrying no state. It is **not** the guard
/// — each statement's own conjunction is — and it is deliberately not trusted as
/// one: between this read and that statement a concurrent publish may land, and
/// what closes the window is the rollback, not this answer.
///
/// It hands the row back rather than a `bool` so the one caller that needs a
/// stored column — `update_draft`, comparing a submitted grandfathering horizon
/// against the row's eligibility class — does not read the row a second time.
async fn mutable_draft(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    price_id: Uuid,
    expected: RowVersion,
) -> Result<Option<price::Model>, RepoError> {
    let Some(row) = load_row(runner, scope, tenant_id, price_id).await? else {
        return Ok(None);
    };
    let state = read_lifecycle(&row.lifecycle_state)?;
    let current = read_row_version(price_id, row.row_version)?;
    Ok((state.is_content_mutable() && current == expected).then_some(row))
}

/// Refuse a grandfathering horizon on a class that may not carry one.
///
/// `chk_pricing_price_grandfather_until` pairs `grandfather_until` with
/// `price_eligibility = 'existing_grandfathered'`, and `grandfather_until` is
/// ordinary caller-supplied content on the draft plane — so without this check
/// the pairing is discovered by the driver, reaches the caller as
/// [`RepoError::Db`] and renders as a 500 for a request whose author only has to
/// clear one field.
///
/// The pairing was a physical CHECK with no rule behind it when this refusal was
/// written; D-147 states it normatively and gives it
/// `GRANDFATHER_UNTIL_FORBIDDEN`, so the refusal now carries a code of its own
/// instead of borrowing the generic bad-request answer. One direction only: a
/// grandfathered row with no horizon is indefinite, and this check says nothing
/// about it.
///
/// # Errors
/// [`RepoError::GrandfatherHorizonOffClass`] naming the class the key holds.
fn check_grandfather_horizon(
    horizon: Option<DateTime<Utc>>,
    eligibility: PriceEligibility,
) -> Result<(), RepoError> {
    if horizon.is_none() || matches!(eligibility, PriceEligibility::ExistingGrandfathered) {
        return Ok(());
    }
    Err(RepoError::GrandfatherHorizonOffClass {
        eligibility: eligibility.to_string(),
    })
}

/// Name which conjunct of a failed compare-and-swap actually failed.
///
/// One extra read, taken only on the refusal path. It costs nothing in the
/// normal case and is the difference between an operator being told to retry
/// and being told to stop retrying.
async fn refuse(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    price_id: Uuid,
    expected: RowVersion,
) -> RepoError {
    let row = match load_row(runner, scope, tenant_id, price_id).await {
        Err(err) => return err,
        Ok(None) => return not_found(price_id),
        Ok(Some(row)) => row,
    };
    let state = match read_lifecycle(&row.lifecycle_state) {
        Ok(state) => state,
        Err(err) => return err,
    };
    if !state.is_content_mutable() {
        return RepoError::NotDraft {
            subject: SUBJECT.to_owned(),
            id: price_id.to_string(),
            state: state.to_string(),
        };
    }
    match read_row_version(price_id, row.row_version) {
        Err(err) => err,
        Ok(current) => RepoError::StaleRowVersion {
            subject: SUBJECT.to_owned(),
            id: price_id.to_string(),
            current: current.get(),
            submitted: expected.get(),
        },
    }
}

/// The conjunction that makes an UPDATE or DELETE a compare-and-swap on one
/// **draft** row.
///
/// `None` when the submitted version cannot be stored at all; every caller then
/// resolves it through [`refuse`], because no row can hold such a value and the
/// truthful answer is whichever one the row itself gives.
fn swap_guard(tenant_id: Uuid, price_id: Uuid, expected: RowVersion) -> Option<Condition> {
    let version = expected.to_stored().ok()?;
    Some(
        Condition::all()
            .add(price::Column::TenantId.eq(tenant_id))
            .add(price::Column::PriceId.eq(price_id))
            .add(price::Column::RowVersion.eq(version))
            .add(price::Column::LifecycleState.eq(LifecycleState::Draft.as_str())),
    )
}

/// The eight axes as a filter, in normative order.
///
/// One spelling, so no statement here can decide "the same key" by fewer axes
/// than the key actually has — the mistake that would report a collision
/// between two rows that do not share a key at all.
fn scope_key_filter(tenant_id: Uuid, key: &ScopeKey) -> Condition {
    Condition::all()
        .add(price::Column::TenantId.eq(tenant_id))
        .add(price::Column::PlanId.eq(key.plan_id().get()))
        .add(price::Column::Currency.eq(key.currency().as_str()))
        .add(price::Column::Region.eq(key.region().as_str()))
        .add(price::Column::PriceOverlay.eq(key.price_overlay().as_str()))
        .add(price::Column::Phase.eq(key.phase().get()))
        .add(price::Column::PriceEligibility.eq(key.price_eligibility().as_str()))
        .add(price::Column::ChargeKind.eq(key.charge_kind().as_str()))
        .add(price::Column::Cohort.eq(key.cohort().to_string()))
}

/// The "absent, or not yours" refusal — deliberately one answer for both, so
/// the surface leaks no existence.
fn not_found(price_id: Uuid) -> RepoError {
    RepoError::NotFound {
        subject: SUBJECT.to_owned(),
        id: price_id.to_string(),
    }
}

/// The occupied-key refusal.
///
/// The key's own eight-axis rendering leads, verbatim, because that is what a
/// `DUPLICATE_SCOPE_KEY` response has to carry; the occupant's id and state
/// follow, because "this key is taken" without saying by what leaves the author
/// to go looking.
fn duplicate_key(key: &ScopeKey, occupant: &price::Model) -> RepoError {
    RepoError::DuplicateScopeKey(format!(
        "{key} is held by {} price {}",
        occupant.lifecycle_state, occupant.price_id
    ))
}

// ---------------------------------------------------------------------------
// Domain -> storage.
// ---------------------------------------------------------------------------

/// Render a draft's **content** into the columns that hold it.
///
/// Identity, the scope key and the provenance columns are left `NotSet`: they
/// are what [`insert_model`] adds on creation, and what an update may never
/// move.
fn content_model(content: &PriceContent) -> Result<price::ActiveModel, RepoError> {
    let row = &content.row;
    Ok(price::ActiveModel {
        amount_minor: Set(row.amount_minor.map(MinorAmount::get)),
        model_kind: Set(row.model_kind.map(model_kind_wire).map(str::to_owned)),
        tax_inclusive: Set(content.tax_inclusive),
        billing_timing: Set(content.billing_timing.clone()),
        quantity_source: Set(row.quantity_source.map(|s| s.as_str().to_owned())),
        manual_quantity: Set(stored_count("manual_quantity", row.manual_quantity)?),
        package_size: Set(stored_count("package_size", row.package_size)?),
        package_price_minor: Set(row.package_price_minor.map(MinorAmount::get)),
        meter: Set(row.meter.clone()),
        dimension_key: Set(row.dimension_key.clone()),
        billing_granularity: Set(row.billing_granularity.map(|g| g.as_str().to_owned())),
        aggregation_function: Set(row.aggregation_function.map(|f| f.as_str().to_owned())),
        aggregation_granularity: Set(row.aggregation_granularity.map(|g| g.as_str().to_owned())),
        tier_aggregation_window: Set(row.tier_aggregation_window.map(|w| w.as_str().to_owned())),
        tier_qualification_window: Set(row
            .tier_qualification_window
            .map(|w| w.as_str().to_owned())),
        max_hold_granules: Set(stored_count("max_hold_granules", row.max_hold_granules)?),
        included_allowance: Set(row.included_allowance.map(allowance_json)),
        rounding_policy_ref: Set(content.rounding_policy_ref.clone()),
        grandfather_until: Set(content.grandfather_until),
        supersedes_price_id: Set(content.supersedes_price_id),
        ..price::ActiveModel::default()
    })
}

/// A create, rendered and value-checked, with no statement issued yet.
///
/// It exists so [`PriceRepo::create_draft`] can keep refusing a caller mistake
/// **before** it opens a transaction while [`create_draft_on`] still has one
/// runner-taking entry point. Both go through this and through
/// [`insert_prepared`]; neither restates the other's body.
struct PreparedDraft {
    /// The record as it will be answered, bands already in read order.
    record: PriceRecord,
    /// The row, rendered.
    row: price::ActiveModel,
    /// Its band set, rendered.
    bands: Vec<price_tier_band::ActiveModel>,
}

/// Render a create and refuse every value the store cannot take, statement-free.
fn prepare_draft(tenant_id: Uuid, draft: NewPriceDraft) -> Result<PreparedDraft, RepoError> {
    // The row's own `charge_kind` is not stored: the axis is the key's, and the
    // copy on `PriceRow` is a convenience the shape rules read. So a record read
    // back always agrees with its own key, and a caller that handed the two of
    // them different answers gets the key's.
    let charge_kind = draft.scope_key.charge_kind();
    let mut record = PriceRecord {
        price_id: draft.price_id,
        scope_key: draft.scope_key,
        row: PriceRow {
            charge_kind,
            ..draft.content.row
        },
        tax_inclusive: draft.content.tax_inclusive,
        billing_timing: draft.content.billing_timing,
        rounding_policy_ref: draft.content.rounding_policy_ref,
        grandfather_until: draft.content.grandfather_until,
        supersedes_price_id: draft.content.supersedes_price_id,
        lifecycle_state: LifecycleState::Draft,
        created_by: draft.created_by,
        created_at_utc: draft.created_at_utc,
        row_version: RowVersion::new(0),
    };
    // Answered in the order a read gives back, not the order it was authored in.
    // `find` sorts because the table carries no ordinal, and a create that
    // answered in authoring order would hand the caller a record that stops
    // equalling itself after one round trip.
    record.row.bands.sort_by_key(|band| band.from_qty);
    // Before the CHECK that would otherwise refuse it: the key is in hand here,
    // so the pairing costs a comparison rather than a round trip and a 500.
    check_grandfather_horizon(
        record.grandfather_until,
        record.scope_key.price_eligibility(),
    )?;
    check_authored_instant("grandfatherUntil", record.grandfather_until)?;
    let row = insert_model(tenant_id, &record)?;
    let bands = band_models(tenant_id, record.price_id, &record.row.bands)?;
    Ok(PreparedDraft { record, row, bands })
}

/// The three statements a create issues, through whichever runner holds them.
///
/// The occupancy read, the row and the band set are one unit by rule rather than
/// by preference: a row whose geometry can land a moment late is a row that is
/// briefly wrong, and the duplicate check is a read the insert would otherwise
/// race with across a commit boundary.
async fn insert_prepared(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    prepared: PreparedDraft,
) -> Result<PriceRecord, RepoError> {
    let PreparedDraft { record, row, bands } = prepared;
    if let Some(occupant) = find_key_occupant(runner, scope, tenant_id, &record.scope_key).await? {
        return Err(duplicate_key(&record.scope_key, &occupant));
    }
    insert_price(runner, scope, row).await?;
    insert_bands(runner, scope, bands).await?;
    // The record, in the same transaction as the insert (D-135). The stamp is the
    // draft's own - `created_by` is the actor, `created_at_utc` the instant - so
    // this path needs nothing threaded that it did not already have.
    record_price_mutation(
        runner,
        scope,
        tenant_id,
        &record,
        AuditAction::Create,
        None,
        AuditStamp {
            actor_principal_id: record.created_by,
            recorded_at: record.created_at_utc,
            correlation_id: None,
        },
    )
    .await?;
    Ok(record)
}

/// Append the audit record of one price-row mutation, inside the mutation's own
/// transaction.
///
/// The chain segment is the **plan's** (D-135 keys it on the audited subject's
/// aggregate, and a price row's aggregate is the plan it prices), so a plan's
/// authoring history and its rows' edits are one walk.
///
/// `before` is the version the caller presented — `None` on a create, where
/// nothing existed, which is the difference between a create and an edit and is
/// expressible only as an absence.
///
/// # Errors
/// [`RepoError::Db`] or [`RepoError::CorruptRow`] from the chain append, both of
/// which roll the mutation back with them. That is the point: a mutation whose
/// record cannot be written must not commit.
async fn record_price_mutation(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    record: &PriceRecord,
    action: AuditAction,
    before: Option<RowVersion>,
    stamp: AuditStamp,
) -> Result<(), RepoError> {
    audit_repo::append(
        runner,
        scope,
        NewAuditEntry {
            tenant_id,
            chain_id: audit_repo::plan_chain(record.scope_key.plan_id()),
            recorded_at: stamp.recorded_at,
            actor_principal_id: stamp.actor_principal_id,
            action,
            subject_kind: AuditSubjectKind::PriceUnit,
            subject_ref: audit_repo::price_unit_ref(record.price_id),
            before_state: before
                .map(|version| subject_state(LifecycleState::Draft, version.get(), None)),
            after_state: match action {
                // A delete has no after-state, and an empty object would be a
                // claim that the row still stands in some shape.
                AuditAction::Delete => None,
                _ => Some(subject_state(
                    record.lifecycle_state,
                    record.row_version.get(),
                    None,
                )),
            },
            approval_ref: None,
            correlation_id: stamp.correlation_id,
        },
    )
    .await
    .map(|_| ())
}

/// Create a draft price row and its bands through whichever runner the caller
/// holds.
///
/// [`PriceRepo::create_draft`] supplies its own transaction; the other caller is
/// `infra::idempotent`, which must run the insert inside the **same**
/// transaction as the idempotency claim guarding it — see
/// [`create_draft_on`](super::plan_repo::create_draft_on)'s sibling note and
/// `idempotency_repo`'s module doc for why splitting the two inverts the
/// guarantee. The row-and-bands-in-one-transaction invariant this module's doc
/// names holds either way: the seam supplies the transaction, the method
/// supplies its own.
///
/// # Errors
/// [`RepoError::DuplicateScopeKey`] naming the key and its occupant;
/// [`RepoError::GrandfatherHorizonOffClass`] for a horizon off the grandfathered
/// class; [`RepoError::TimestampPrecisionExceeded`] for a horizon finer than the
/// millisecond quantum; [`RepoError::ValueOutOfRange`] for an authored quantity
/// past its column; [`RepoError::Db`] on a scope or storage failure, including
/// losing the `uq_pricing_price_scope_key_draft` race;
/// [`RepoError::CorruptRow`] only for a `row_version` that cannot be stored.
pub async fn create_draft_on(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    draft: NewPriceDraft,
) -> Result<PriceRecord, RepoError> {
    insert_prepared(runner, scope, tenant_id, prepare_draft(tenant_id, draft)?).await
}

/// The whole insert: content, plus the columns only a creation writes.
fn insert_model(tenant_id: Uuid, record: &PriceRecord) -> Result<price::ActiveModel, RepoError> {
    let key = &record.scope_key;
    let content = content_model(&record.content())?;
    Ok(price::ActiveModel {
        price_id: Set(record.price_id),
        tenant_id: Set(tenant_id),
        plan_id: Set(key.plan_id().get()),
        currency: Set(key.currency().as_str().to_owned()),
        region: Set(key.region().as_str().to_owned()),
        price_overlay: Set(key.price_overlay().as_str().to_owned()),
        phase: Set(key.phase().get()),
        price_eligibility: Set(key.price_eligibility().as_str().to_owned()),
        charge_kind: Set(key.charge_kind().as_str().to_owned()),
        // The axis is a `NOT NULL` text token — `none`, or the cutover instant
        // — rather than a nullable timestamp, because distinct `NULL`s do not
        // collide in the partial `UNIQUE` that decides row uniqueness.
        cohort: Set(key.cohort().to_string()),
        lifecycle_state: Set(record.lifecycle_state.as_str().to_owned()),
        created_by: Set(record.created_by),
        created_at_utc: Set(record.created_at_utc),
        row_version: Set(record.row_version.to_stored().map_err(|e| {
            RepoError::CorruptRow(format!("pricing_price {}: {e}", record.price_id))
        })?),
        ..content
    })
}

/// The columns an `update_draft` writes: the whole of the row's content, and
/// nothing of its identity, its scope key or its provenance.
///
/// The values are read off the very `ActiveModel` the insert path builds, so
/// the two writers cannot render one column two different ways. The **list**,
/// by contrast, is written out rather than derived from the entity: which
/// columns a draft edit may move is a decision, and a column added to the table
/// must not become editable merely by existing.
fn content_assignments(model: &price::ActiveModel) -> Vec<(price::Column, Value)> {
    [
        (
            price::Column::AmountMinor,
            model.amount_minor.clone().into_value(),
        ),
        (
            price::Column::ModelKind,
            model.model_kind.clone().into_value(),
        ),
        (
            price::Column::TaxInclusive,
            model.tax_inclusive.clone().into_value(),
        ),
        (
            price::Column::BillingTiming,
            model.billing_timing.clone().into_value(),
        ),
        (
            price::Column::QuantitySource,
            model.quantity_source.clone().into_value(),
        ),
        (
            price::Column::ManualQuantity,
            model.manual_quantity.clone().into_value(),
        ),
        (
            price::Column::PackageSize,
            model.package_size.clone().into_value(),
        ),
        (
            price::Column::PackagePriceMinor,
            model.package_price_minor.clone().into_value(),
        ),
        (price::Column::Meter, model.meter.clone().into_value()),
        (
            price::Column::DimensionKey,
            model.dimension_key.clone().into_value(),
        ),
        (
            price::Column::BillingGranularity,
            model.billing_granularity.clone().into_value(),
        ),
        (
            price::Column::AggregationFunction,
            model.aggregation_function.clone().into_value(),
        ),
        (
            price::Column::AggregationGranularity,
            model.aggregation_granularity.clone().into_value(),
        ),
        (
            price::Column::TierAggregationWindow,
            model.tier_aggregation_window.clone().into_value(),
        ),
        (
            price::Column::TierQualificationWindow,
            model.tier_qualification_window.clone().into_value(),
        ),
        (
            price::Column::MaxHoldGranules,
            model.max_hold_granules.clone().into_value(),
        ),
        (
            price::Column::IncludedAllowance,
            model.included_allowance.clone().into_value(),
        ),
        (
            price::Column::RoundingPolicyRef,
            model.rounding_policy_ref.clone().into_value(),
        ),
        (
            price::Column::GrandfatherUntil,
            model.grandfather_until.clone().into_value(),
        ),
        (
            price::Column::SupersedesPriceId,
            model.supersedes_price_id.clone().into_value(),
        ),
    ]
    .into_iter()
    .filter_map(|(column, value)| value.map(|value| (column, value)))
    .collect()
}

/// Render a band set for its table.
///
/// `tenant_id` is the parent row's, never a caller's: the foreign key covers
/// `price_id` alone, so a band carrying a different tenant would be invisible to
/// its owner under `SecureORM` while still joined to their price.
fn band_models(
    tenant_id: Uuid,
    price_id: Uuid,
    bands: &[TierBand],
) -> Result<Vec<price_tier_band::ActiveModel>, RepoError> {
    bands
        .iter()
        .map(|band| {
            let from_qty = stored_bound("band from_qty", band.from_qty)?;
            let to_qty = match band.to_qty {
                BandTop::Open => None,
                BandTop::Closed(top) => Some(stored_bound("band to_qty", top)?),
            };
            Ok(price_tier_band::ActiveModel {
                band_id: Set(band_id(price_id, from_qty)),
                tenant_id: Set(tenant_id),
                price_id: Set(price_id),
                from_qty: Set(from_qty),
                to_qty: Set(to_qty),
                unit_price_minor: Set(band.unit_price_minor.get()),
            })
        })
        .collect()
}

/// A band's surrogate key, derived from the identity the table actually states:
/// `UNIQUE (price_id, from_qty)`.
///
/// `band_id` is a `PRIMARY KEY` with no default and nothing outside this module
/// reads it, so it could have been random. Deriving it makes the surrogate agree
/// with the real identity: replacing a band set with an identical one writes the
/// same ids back, so no consumer can come to depend on an id that changes on
/// every save, and two bands sharing a lower bound collide on both keys rather
/// than on only the one that happened to be checked.
fn band_id(price_id: Uuid, from_qty: i64) -> Uuid {
    Uuid::new_v5(&price_id, &from_qty.to_be_bytes())
}

/// The D-45 declaration as the column carries it, in the `{quantity,
/// rolloverPolicy}` spelling the design set uses. Persisted exactly as authored
/// and never compiled here: the compile is Slice 10's, and the D-129
/// supersession guard reads this field, so a round trip that reshaped it would
/// make the guard report a change nobody made.
fn allowance_json(allowance: IncludedAllowance) -> JsonValue {
    json!({
        "quantity": allowance.quantity,
        "rolloverPolicy": allowance.rollover_policy.as_str(),
    })
}

/// Render an authored count for its `bigint` column.
///
/// # Errors
/// [`RepoError::ValueOutOfRange`] past [`i64::MAX`]. Checked rather than cast: a
/// cast would turn an impossible quantity into a plausible one and price
/// something nobody authored.
fn stored_count(field: &str, value: Option<u64>) -> Result<Option<i64>, RepoError> {
    let Some(value) = value else {
        return Ok(None);
    };
    i64::try_from(value)
        .map(Some)
        .map_err(|_| out_of_range(field, value))
}

/// Render a band bound for its `bigint` column.
fn stored_bound(field: &str, value: u64) -> Result<i64, RepoError> {
    i64::try_from(value).map_err(|_| out_of_range(field, value))
}

/// The caller-mistake refusal for a value no column can hold.
fn out_of_range(field: &str, value: u64) -> RepoError {
    RepoError::ValueOutOfRange {
        field: field.to_owned(),
        value: value.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Storage -> domain.
//
// Every failure below is an **invariant breach, not a caller mistake**: each
// token *column* is `CHECK`-constrained to the set its domain enum renders, and
// every count column is either `CHECK`-constrained non-negative or only ever
// written from a `u64`. A row that reads otherwise means something reached the
// table outside this gear, which is why it surfaces as `CorruptRow` rather than
// as a not-found or a bad request.
//
// One token is not a column and cannot be constrained that way: the D-45
// `rolloverPolicy` lives **inside** the `included_allowance` jsonb, where no
// row CHECK can see it. It reads back through the same inverse list and gets
// the same `CorruptRow`, resting on the weaker of the two grounds — this
// repository is the only writer of the column and it renders the token from
// the domain enum. The reading is the same; what differs is that here the
// schema is not standing behind it, and a Slice-10 writer that started editing
// the declaration in place is what would find that out.
// ---------------------------------------------------------------------------

/// Map a stored row and its bands to the domain value, at this boundary and
/// nowhere else.
fn to_record(
    row: &price::Model,
    bands: &[price_tier_band::Model],
) -> Result<PriceRecord, RepoError> {
    let scope_key = to_scope_key(row)?;
    let shape = to_price_row(row, scope_key.charge_kind(), bands)?;
    Ok(PriceRecord {
        price_id: row.price_id,
        scope_key,
        row: shape,
        tax_inclusive: row.tax_inclusive,
        billing_timing: row.billing_timing.clone(),
        rounding_policy_ref: row.rounding_policy_ref.clone(),
        grandfather_until: row.grandfather_until,
        supersedes_price_id: row.supersedes_price_id,
        lifecycle_state: read_lifecycle(&row.lifecycle_state)?,
        created_by: row.created_by,
        created_at_utc: row.created_at_utc,
        row_version: read_row_version(row.price_id, row.row_version)?,
    })
}

/// Rebuild the canonical key from its eight columns.
///
/// The cohort / eligibility biconditional is re-established here rather than
/// assumed: the two axes are read back as two independent columns, so the
/// pairing has to hold on every rehydration and not only at first construction.
fn to_scope_key(row: &price::Model) -> Result<ScopeKey, RepoError> {
    let currency = CurrencyCode::new(&row.currency)
        .map_err(|e| RepoError::CorruptRow(format!("pricing_price.currency: {e}")))?;
    let region = Region::new(&row.region)
        .map_err(|e| RepoError::CorruptRow(format!("pricing_price.region: {e}")))?;
    // Asked even though `ScopeKey` takes no overlay: the constructor would
    // silently answer `base` for a row stored on any other plane, and a row the
    // authoring path could not have written must not read back as one it could.
    read_token(
        "pricing_price.price_overlay",
        &row.price_overlay,
        PRICE_OVERLAYS,
        PriceOverlay::as_str,
    )?;
    ScopeKey::new(
        PlanId::new(row.plan_id),
        currency,
        region,
        PhaseId::new(row.phase),
        read_eligibility(row)?,
        read_token(
            "pricing_price.charge_kind",
            &row.charge_kind,
            CHARGE_KINDS,
            ChargeKind::as_str,
        )?,
        read_cohort(&row.cohort)?,
    )
    .map_err(|e| RepoError::CorruptRow(format!("pricing_price scope key: {e}")))
}

/// Read the eligibility axis back into the class the row is filed under.
///
/// One spelling, because two readers want it: the key rehydration below, and
/// the horizon pairing an update has to check against the *stored* class. Two
/// spellings would be two inverse lists, and the one that fell behind would read
/// a live class as a corrupt row.
fn read_eligibility(row: &price::Model) -> Result<PriceEligibility, RepoError> {
    read_token(
        "pricing_price.price_eligibility",
        &row.price_eligibility,
        PRICE_ELIGIBILITIES,
        PriceEligibility::as_str,
    )
}

/// Map the Slice-3 columns and the band set back into the shape the rules judge.
fn to_price_row(
    row: &price::Model,
    charge_kind: ChargeKind,
    bands: &[price_tier_band::Model],
) -> Result<PriceRow, RepoError> {
    Ok(PriceRow {
        charge_kind,
        model_kind: read_optional(
            "pricing_price.model_kind",
            row.model_kind.as_deref(),
            &ModelKind::ALL,
            model_kind_wire,
        )?,
        amount_minor: read_amount("pricing_price.amount_minor", row.amount_minor)?,
        bands: bands.iter().map(to_band).collect::<Result<_, _>>()?,
        package_size: read_count("pricing_price.package_size", row.package_size)?,
        package_price_minor: read_amount(
            "pricing_price.package_price_minor",
            row.package_price_minor,
        )?,
        quantity_source: read_optional(
            "pricing_price.quantity_source",
            row.quantity_source.as_deref(),
            QUANTITY_SOURCES,
            QuantitySource::as_str,
        )?,
        manual_quantity: read_count("pricing_price.manual_quantity", row.manual_quantity)?,
        meter: row.meter.clone(),
        dimension_key: row.dimension_key.clone(),
        billing_granularity: read_optional(
            "pricing_price.billing_granularity",
            row.billing_granularity.as_deref(),
            BILLING_GRANULARITIES,
            BillingGranularity::as_str,
        )?,
        tier_aggregation_window: read_optional(
            "pricing_price.tier_aggregation_window",
            row.tier_aggregation_window.as_deref(),
            TIER_AGGREGATION_WINDOWS,
            TierAggregationWindow::as_str,
        )?,
        tier_qualification_window: read_optional(
            "pricing_price.tier_qualification_window",
            row.tier_qualification_window.as_deref(),
            TIER_QUALIFICATION_WINDOWS,
            TierQualificationWindow::as_str,
        )?,
        aggregation_function: read_optional(
            "pricing_price.aggregation_function",
            row.aggregation_function.as_deref(),
            AGGREGATION_FUNCTIONS,
            AggregationFunction::as_str,
        )?,
        aggregation_granularity: read_optional(
            "pricing_price.aggregation_granularity",
            row.aggregation_granularity.as_deref(),
            AGGREGATION_GRANULARITIES,
            AggregationGranularity::as_str,
        )?,
        max_hold_granules: read_count("pricing_price.max_hold_granules", row.max_hold_granules)?,
        included_allowance: row
            .included_allowance
            .as_ref()
            .map(read_allowance)
            .transpose()?,
    })
}

/// Map one stored band. `NULL` `to_qty` is the **open top** — a state of the
/// band, not an absent value.
fn to_band(band: &price_tier_band::Model) -> Result<TierBand, RepoError> {
    let from_qty = read_bound("pricing_price_tier_band.from_qty", band.from_qty)?;
    let to_qty = match read_count("pricing_price_tier_band.to_qty", band.to_qty)? {
        None => BandTop::Open,
        Some(top) => BandTop::Closed(top),
    };
    let unit_price_minor = MinorAmount::new(band.unit_price_minor).map_err(|e| {
        RepoError::CorruptRow(format!("pricing_price_tier_band.unit_price_minor: {e}"))
    })?;
    Ok(TierBand {
        from_qty,
        to_qty,
        unit_price_minor,
    })
}

/// Read the D-45 declaration back out of its JSON column.
fn read_allowance(stored: &JsonValue) -> Result<IncludedAllowance, RepoError> {
    let malformed = || {
        RepoError::CorruptRow(format!(
            "pricing_price.included_allowance is not a {{quantity, rolloverPolicy}} object: {stored}"
        ))
    };
    let quantity = stored
        .get("quantity")
        .and_then(JsonValue::as_u64)
        .ok_or_else(malformed)?;
    let token = stored
        .get("rolloverPolicy")
        .and_then(JsonValue::as_str)
        .ok_or_else(malformed)?;
    Ok(IncludedAllowance {
        quantity,
        rollover_policy: read_token(
            "pricing_price.included_allowance.rolloverPolicy",
            token,
            ROLLOVER_POLICIES,
            RolloverPolicy::as_str,
        )?,
    })
}

/// Read the cohort axis back: `none`, or the UTC cutover instant its own
/// rendering carries as epoch milliseconds.
///
/// Millisecond resolution is what [`Cohort`]'s rendering has, so it is what the
/// column can hold; a cutover instant authored with finer precision does not
/// round-trip, and would put a row on a key nothing later matches.
fn read_cohort(token: &str) -> Result<Cohort, RepoError> {
    if token == Cohort::None.to_string() {
        return Ok(Cohort::None);
    }
    let malformed = || {
        RepoError::CorruptRow(format!(
            "pricing_price.cohort holds {token}, not an instant"
        ))
    };
    let millis: i64 = token.parse().map_err(|_| malformed())?;
    Utc.timestamp_millis_opt(millis)
        .single()
        .map(Cohort::Generation)
        .ok_or_else(malformed)
}

/// Read a lifecycle token back into the state machine's vocabulary.
fn read_lifecycle(token: &str) -> Result<LifecycleState, RepoError> {
    read_token(
        "pricing_price.lifecycle_state",
        token,
        LifecycleState::ALL,
        LifecycleState::as_str,
    )
}

/// Read the entity tag back out of its `bigint` column.
fn read_row_version(price_id: Uuid, stored: i64) -> Result<RowVersion, RepoError> {
    RowVersion::from_stored(stored)
        .map_err(|e| RepoError::CorruptRow(format!("pricing_price {price_id}: {e}")))
}

/// Read a non-negative money column.
fn read_amount(column: &str, stored: Option<i64>) -> Result<Option<MinorAmount>, RepoError> {
    let Some(units) = stored else {
        return Ok(None);
    };
    MinorAmount::new(units)
        .map(Some)
        .map_err(|e| RepoError::CorruptRow(format!("{column}: {e}")))
}

/// Read a count column back into the unsigned type the domain counts in.
fn read_count(column: &str, stored: Option<i64>) -> Result<Option<u64>, RepoError> {
    let Some(value) = stored else {
        return Ok(None);
    };
    u64::try_from(value)
        .map(Some)
        .map_err(|_| RepoError::CorruptRow(format!("{column} holds {value}, not a count")))
}

/// Read a required count column back into the unsigned type the domain counts
/// in. `from_qty` has no null state — the open top lives on `to_qty` — so it is
/// read on its own rather than through an `Option` that could only ever be
/// `Some`.
fn read_bound(column: &str, stored: i64) -> Result<u64, RepoError> {
    u64::try_from(stored)
        .map_err(|_| RepoError::CorruptRow(format!("{column} holds {stored}, not a count")))
}

/// Read a nullable token column.
fn read_optional<T: Copy>(
    column: &str,
    token: Option<&str>,
    candidates: &[T],
    render: fn(T) -> &'static str,
) -> Result<Option<T>, RepoError> {
    let Some(token) = token else {
        return Ok(None);
    };
    read_token(column, token, candidates, render).map(Some)
}

/// Read a stored token back into the domain value that renders it.
fn read_token<T: Copy>(
    column: &str,
    token: &str,
    candidates: &[T],
    render: fn(T) -> &'static str,
) -> Result<T, RepoError> {
    candidates
        .iter()
        .copied()
        .find(|candidate| render(*candidate) == token)
        .ok_or_else(|| RepoError::CorruptRow(format!("{column} holds {token}")))
}
