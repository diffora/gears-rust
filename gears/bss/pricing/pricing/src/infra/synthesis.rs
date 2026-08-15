//! The `SnapshotSynthesizer` of §1.7 — the reads behind D-76's two tiers and
//! D-87's self-contained payload (`inst-sy-freeze`, `inst-sy-select`,
//! `inst-sy-payload`, `inst-sy-provenance`, `inst-sy-backdate`, `inst-ms-synth`).
//!
//! [`crate::domain::synthesis`] holds the rule; this module holds the queries and
//! the freeze. The split is the usual one, and here it earns itself twice over:
//! tier 2's query has no store to run against, so keeping the *rule* free of that
//! fact is what lets the rule stay tested while the read is absent.
//!
//! # Tier 1, and the one reader it needs
//!
//! "The `pricing_price` row, **current or superseded**, whose `PriceWindow`
//! covered `t` on that key". Every part of that is on one record:
//! `window_repo::list_for_plan` resolves each window's ten-axis
//! [`ScopeKey`](crate::domain::scope_key::ScopeKey) from `pricing_price` on read,
//! so the key match, the interval test and the row id all come from one query and
//! there is no join to get wrong.
//!
//! **Half-open, `[effective_from, effective_to)`.** An instant exactly at a
//! window's end belongs to the *next* window, which is the same rule coverage
//! uses, and `effective_to = None` is open-ended rather than missing.
//!
//! **Cancelled windows are excluded.** A cancelled window never took effect, so a
//! row it scheduled was never what rating resolved — including it would let
//! synthesis freeze a price the subscriber demonstrably never paid. `scheduled`,
//! `active` and `expired` are all admitted: what makes a window evidence here is
//! that its interval covered `t`, not what state the clock has since moved it to.
//!
//! **And the row behind the window must be `published` or `superseded`**, which is
//! clause 1 read literally. This sentence said "and nothing else is" until
//! 2026-08-11, and it was a claim about *windows* standing in for a filter that had
//! to be about *rows*: `list_for_plan` is taken whole, over every price row of the
//! plan whatever its lifecycle, so a `draft` row's window was admissible evidence
//! and a row that never published could be frozen — labelled `live_history` —
//! as what the subscriber was paying.
//!
//! # Tier 2 has no store, so it is a parameter rather than a query
//!
//! `pricing_historical_price` is Slice 5's `inst-bd-store` and is unbuilt (§1.7
//! records it normatively). [`select_for_key`] therefore passes an **empty**
//! reference candidate set into the domain rule rather than not calling it: the
//! call site is the seam, it is exercised on every synthesis today, and the day
//! the store lands only the query behind it changes.
//!
//! `inst-sy-backdate` names synthesis as the sanctioned **consumer** of that
//! backdating path — a consumer with nothing to consume is still the consumer,
//! and this is where it will read.
//!
//! # The payload materializes what nothing can look up
//!
//! D-87 plus C-5. Two halves, and the second is the one that was missing when the
//! rule was first written:
//!
//! * **per resolved row** — the evaluable content, read off `pricing_price`;
//! * **plan-level** — the billing descriptor set and the resolved entitlement
//!   grant set, without which the payload is row-complete and invoice-incomplete,
//!   because Billing has no `CatalogVersion` to fall back to on a
//!   `migrated-origin` ref by construction.
//!
//! **The grant set is empty and that is not a shortcut.** There is no entitlement
//! grant store in this gear — no table, no column, no domain type — so the
//! plan-level half lands descriptor-complete and grant-absent. It is reported
//! rather than silently omitted: the payload carries `grantSetUnavailable` so a
//! consumer cannot read the absence as "this plan grants nothing".

use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, Condition, EntityTrait, Order};
use serde_json::{Value as JsonValue, json};
use toolkit_db::secure::{AccessScope, DBRunner, SecureEntityExt};
use uuid::Uuid;

use crate::domain::error::DomainError;
use crate::domain::scope_key::PlanId;
use crate::domain::synthesis::{
    LiveCandidate, SelectedRow, SynthesisOutcome, UnresolvedKey, select_rows,
};
use crate::domain::window::WindowState;
use crate::infra::storage::entity::{plan_descriptor_set, plan_period_floor_cap, price};
use crate::infra::storage::repo::{plan_repo, price_repo, window_repo};
use crate::infra::storage::{RepoError, repo_failure};

/// A scope key synthesis must resolve a row for — the subscription's frozen
/// `(currency, region)` pair (D-76).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrozenKey {
    /// ISO currency.
    pub currency: String,
    /// The region axis.
    pub region: String,
}

/// Resolve one scope key against both tiers at `t` (`inst-sy-select`).
///
/// # Errors
/// [`DomainError::Internal`] on a storage failure.
pub async fn select_for_key(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: PlanId,
    key: &FrozenKey,
    at: DateTime<Utc>,
) -> Result<Vec<SelectedRow>, DomainError> {
    let windows = window_repo::list_for_plan(runner, scope, tenant_id, plan_id)
        .await
        .map_err(|e| repo_failure(&e))?;

    // **D-76 clause 1 is "the `pricing_price` row, *current or superseded*", and
    // that half was missing.** `list_for_plan` is taken whole — every window state,
    // over every price row of the plan whatever its lifecycle — so a `draft` row's
    // window was admissible evidence and synthesis could freeze, as "what the
    // subscriber was paying", a row that never published, never passed the publish
    // rules and was never approved. It labelled it `live_history` while doing so.
    //
    // `read_model::project_windows` is the other reader of this same list and the
    // other one that asserts fact to a consumer; it restricts on exactly this axis.
    // The intersection is done the same way, against rows read in
    // `PROJECTED_ROW_STATES` rather than by adding a lifecycle column to
    // `WindowRecord` — that type is shared by every window caller, and none of the
    // others is asking this question.
    let admissible = price_repo::load_for_plan(
        runner,
        scope,
        tenant_id,
        plan_id,
        crate::domain::projection::PROJECTED_ROW_STATES,
    )
    .await
    .map_err(|e| repo_failure(&e))?;

    let mut live: Vec<LiveCandidate> = windows
        .iter()
        .filter(|window| {
            // A cancelled window never took effect; see the module doc.
            window.state != WindowState::Cancelled
                && admissible
                    .iter()
                    .any(|row| row.price_id == window.price_id)
                && window.scope_key.currency().as_str() == key.currency
                && window.scope_key.region().as_str() == key.region
                // Half-open: `[from, to)`.
                && window.effective_from <= at
                && window.effective_to.is_none_or(|to| at < to)
        })
        .map(|window| LiveCandidate {
            price_id: window.price_id,
            plan_revision: None,
        })
        .collect();
    // Deterministic, so two runs of one synthesis cannot resolve different rows
    // where a key is covered by more than one admitted window.
    live.sort_by_key(|candidate| candidate.price_id);

    // Tier 2's query has no store. See the module doc: the seam is the call, not
    // a branch around it.
    let reference = Vec::new();

    Ok(select_rows(&live, &reference))
}

/// Resolve every frozen key of one subscription (`inst-sy-select`).
///
/// # Errors
/// [`DomainError::Internal`] on a storage failure.
pub async fn resolve(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: PlanId,
    keys: &[FrozenKey],
    at: DateTime<Utc>,
) -> Result<SynthesisOutcome, DomainError> {
    let mut selected = Vec::new();
    let mut unresolved = Vec::new();
    for key in keys {
        // **Every live line on the market, not the first.** A `FrozenKey` is
        // D-76's `(currency, region)` pair, and a market legitimately carries
        // more than one — `inst-cs-hybrid` sanctions a recurring row beside a
        // usage row. An empty set is what fails the key closed.
        let rows = select_for_key(runner, scope, tenant_id, plan_id, key, at).await?;
        if rows.is_empty() {
            unresolved.push(UnresolvedKey {
                currency: key.currency.clone(),
                region: key.region.clone(),
            });
        } else {
            selected.extend(rows);
        }
    }
    Ok(SynthesisOutcome {
        selected,
        unresolved,
    })
}

/// Materialize D-87's self-contained payload for a resolved set.
///
/// # Errors
/// [`DomainError::Internal`] on a storage failure; [`DomainError::NotFound`] when
/// a resolved tier-1 row has vanished between selection and materialization,
/// which cannot happen inside one transaction and is reported rather than
/// unwrapped.
pub async fn materialize(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: PlanId,
    selected: &[SelectedRow],
) -> Result<JsonValue, DomainError> {
    let mut rows = Vec::new();
    for row in selected {
        let stored = price::Entity::find()
            .secure()
            .scope_with(scope)
            .filter(
                Condition::all()
                    .add(price::Column::TenantId.eq(tenant_id))
                    .add(price::Column::PriceId.eq(row.row_id)),
            )
            .one(runner)
            .await
            .map_err(|e| {
                repo_failure(&RepoError::Db(format!(
                    "materialize the migrated-origin payload of price row {}: {e}",
                    row.row_id
                )))
            })?
            .ok_or_else(|| DomainError::NotFound {
                subject: "price row".to_owned(),
                id: row.row_id.to_string(),
            })?;

        rows.push(json!({
            "rowId": row.row_id,
            "source": row.tier.as_str(),
            "currency": stored.currency,
            "region": stored.region,
            "phase": stored.phase,
            "chargeKind": stored.charge_kind,
            "modelKind": stored.model_kind,
            "amountMinor": stored.amount_minor,
            "packageSize": stored.package_size,
            "packagePriceMinor": stored.package_price_minor,
            "meter": stored.meter,
            "dimensionKey": stored.dimension_key,
            // The evaluation-policy and S6 consumer-contract fields: a
            // `migrated-origin` line is evaluated from this and nothing else.
            "billingTiming": stored.billing_timing,
            "quantitySource": stored.quantity_source,
            "billingGranularity": stored.billing_granularity,
            "aggregationFunction": stored.aggregation_function,
            "aggregationGranularity": stored.aggregation_granularity,
            "tierAggregationWindow": stored.tier_aggregation_window,
            "tierQualificationWindow": stored.tier_qualification_window,
            "includedAllowance": stored.included_allowance,
            // Tax basis and the resolved rounding policy.
            "taxInclusive": stored.tax_inclusive,
            "taxCategoryRef": stored.tax_category_ref,
            "resolvedTaxCategory": stored.resolved_tax_category,
            "roundingPolicyRef": stored.rounding_policy_ref,
        }));
    }

    Ok(json!({
        "rows": rows,
        "planLevel": plan_level(runner, scope, tenant_id, plan_id).await?,
        // Foundation §4.4 names a `migrated-origin` ref the one deliberately
        // non-version-pinned reference. Stated in the payload so a consumer that
        // looks for a version learns why there is none rather than failing.
        "catalogVersion": JsonValue::Null,
        "catalogVersionDeliberatelyAbsent": true,
    }))
}

/// C-5's plan-level half of the payload: the descriptor set and the grant set.
async fn plan_level(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: PlanId,
) -> Result<JsonValue, DomainError> {
    let current = plan_repo::load_current(runner, scope, tenant_id, plan_id)
        .await
        .map_err(|e| repo_failure(&e))?;
    let Some(current) = current else {
        // A fully-legacy key may have no plan revision at all (D-87). The payload
        // says so rather than omitting the half, because "absent" and "empty" are
        // different things to a party posting an invoice from it.
        return Ok(json!({
            "descriptorSetUnavailable": true,
            "grantSetUnavailable": true,
            "periodFloorCapsUnavailable": true,
            "reason": "the source plan has no current revision; a fully legacy key belongs to none",
        }));
    };

    let revision = i64::try_from(current.revision).map_err(|_| {
        DomainError::Internal(format!(
            "plan {plan_id} stands at revision {}, which no column can address",
            current.revision
        ))
    })?;
    // D-319's period floor/cap set, read here by hand for the reason the
    // descriptor set is: this payload resolves through **no** `CatalogVersion`
    // by construction, so a bound outside it is a bound Billing cannot apply and
    // cannot look up. Frozen, and therefore permanently — which is why the
    // omission would have been worse here than in the read model. The `false`
    // marker beside it is `grantSetUnavailable`'s discipline: an empty list must
    // not be readable as "this plan has no minimum" unless it is one.
    let bounds = plan_period_floor_cap::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(plan_period_floor_cap::Column::TenantId.eq(tenant_id))
                .add(plan_period_floor_cap::Column::PlanId.eq(plan_id.get()))
                .add(plan_period_floor_cap::Column::PlanRevision.eq(revision)),
        )
        .order_by(plan_period_floor_cap::Column::Currency, Order::Asc)
        .order_by(plan_period_floor_cap::Column::Region, Order::Asc)
        .all(runner)
        .await
        .map_err(|e| {
            repo_failure(&RepoError::Db(format!(
                "read the period floor/cap set of plan {plan_id}: {e}"
            )))
        })?;

    let descriptors = plan_descriptor_set::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(plan_descriptor_set::Column::TenantId.eq(tenant_id))
                .add(plan_descriptor_set::Column::PlanId.eq(plan_id.get()))
                .add(plan_descriptor_set::Column::PlanRevision.eq(revision)),
        )
        .one(runner)
        .await
        .map_err(|e| {
            repo_failure(&RepoError::Db(format!(
                "read the descriptor set of plan {plan_id}: {e}"
            )))
        })?;

    Ok(json!({
        "planRevision": current.revision,
        // D-48's three v1 descriptor-set fields. Billing posts the line from
        // these, having no `CatalogVersion` to fetch them from.
        "invoiceLineTemplate": descriptors.as_ref().and_then(|d| d.invoice_line_template.clone()),
        "glCode": descriptors.as_ref().and_then(|d| d.gl_code.clone()),
        "itemizationRule": descriptors.as_ref().and_then(|d| d.itemization_rule.clone()),
        "descriptorSetUnavailable": descriptors.is_none(),
        // **There is no entitlement grant store in this gear.** Reported rather
        // than rendered as an empty set, because a consumer must not read the
        // absence as "this plan grants nothing".
        "grantSet": JsonValue::Null,
        "grantSetUnavailable": true,
        // D-319. Rendered with the same members the read-model delta uses, so a
        // consumer reads one shape whichever door the snapshot came through.
        "periodFloorCaps": bounds
            .iter()
            .map(|bound| {
                json!({
                    "currency": bound.currency,
                    "region": bound.region,
                    "floorMinor": bound.floor_minor,
                    "capMinor": bound.cap_minor,
                })
            })
            .collect::<Vec<_>>(),
        "periodFloorCapsUnavailable": false,
    }))
}

/// The resolved set, rendered for the provenance record (`inst-sy-provenance`).
///
/// **The tier rides each id.** An auditor reconstructing a disputed legacy charge
/// must be able to tell a real published price from a governed backdated
/// reconstruction without re-running the lookup, and a per-snapshot tier could
/// not say that about a subscription whose keys resolved differently.
#[must_use]
pub fn resolved_json(selected: &[SelectedRow]) -> JsonValue {
    JsonValue::Array(
        selected
            .iter()
            .map(|row| {
                json!({
                    "rowId": row.row_id,
                    "source": row.tier.as_str(),
                    "planRevision": row.plan_revision,
                })
            })
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// The service (`inst-sy-surface`, `inst-sy-freeze`, `inst-ms-synth`).
// ---------------------------------------------------------------------------

/// The `SnapshotSynthesizer` of §1.7.
///
/// `Clone` for [`crate::infra::retirement::RetirementService`]'s reason: the one
/// field is a handle rather than state. It requests **no** `CatalogVersion`, and
/// that is not an omission — D-87 makes a `migrated-origin` ref the one
/// deliberately non-version-pinned reference in the system, so there is no
/// version for it to request.
#[derive(Clone)]
pub struct SynthesisService {
    db: toolkit_db::DBProvider<toolkit_db::DbError>,
}

impl SynthesisService {
    /// Build the synthesizer over one provider.
    #[must_use]
    pub const fn new(db: toolkit_db::DBProvider<toolkit_db::DbError>) -> Self {
        Self { db }
    }

    /// Read one subscription's frozen snapshot (`inst-sy-surface`, D-102).
    ///
    /// # Errors
    /// [`DomainError::Internal`] on a storage failure.
    pub async fn load(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        subscription_ref: Uuid,
    ) -> Result<Option<crate::infra::storage::repo::ProvenanceRecord>, DomainError> {
        let conn = self.db.conn().map_err(|e| {
            DomainError::Internal(format!("bss-pricing: synthesis read connection: {e}"))
        })?;
        crate::infra::storage::repo::synthesis_repo::load(&conn, scope, tenant_id, subscription_ref)
            .await
            .map_err(|e| repo_failure(&e))
    }

    /// Synthesize and **freeze** one subscription's snapshot (`inst-sy-freeze`,
    /// `inst-sy-select`, `inst-sy-payload`, `inst-ms-synth`).
    ///
    /// Idempotent by §9: a subscription that already holds a snapshot is handed
    /// **that** one back rather than freezing a second at a second instant. The
    /// check is the store's unique index rather than a read here, because the two
    /// calls would otherwise race and D-81 gives the two triggers different `t`.
    ///
    /// # Errors
    /// [`DomainError::PriceRowAbsent`] when any frozen key resolves through
    /// neither tier — clause (3)'s fail-closed refusal, which is what puts the
    /// subscription on the exception list; [`DomainError::Internal`] on a storage
    /// failure.
    pub async fn synthesize(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        request: SynthesisRequest,
    ) -> Result<crate::infra::storage::repo::synthesis_repo::Frozen, DomainError> {
        let scope = scope.clone();
        let (_, outcome) = self
            .db
            .db()
            .in_transaction::<crate::infra::storage::repo::synthesis_repo::Frozen, DomainError, _>(
                move |txn| {
                    Box::pin(async move { synthesize_in(txn, &scope, tenant_id, request).await })
                },
            )
            .await;
        outcome.map_err(|err| {
            err.into_domain(|infra| {
                DomainError::Internal(format!("bss-pricing: synthesis transaction: {infra}"))
            })
        })
    }
}

/// What a caller must supply to synthesize a snapshot.
#[derive(Clone, Debug)]
pub struct SynthesisRequest {
    /// The subscription with no `pricingSnapshotRef`.
    pub subscription_ref: Uuid,
    /// The plan it is on.
    pub source_plan_id: PlanId,
    /// Its frozen `(currency, region)` keys.
    pub keys: Vec<FrozenKey>,
    /// D-81's instant `t`, chosen by the trigger.
    pub at: DateTime<Utc>,
    /// Which trigger this is.
    pub trigger: crate::domain::synthesis::SynthesisTrigger,
    /// Who is acting — recorded on the provenance.
    pub acting_principal: Uuid,
}

/// One synthesis, in the caller's transaction.
///
/// The order is the rule's own: resolve every key, **refuse if any is
/// unresolved** (clause 3 — a partial snapshot is the one outcome that must not
/// exist), materialize, then freeze.
///
/// # Errors
/// See [`SynthesisService::synthesize`].
pub async fn synthesize_in(
    txn: &toolkit_db::secure::DbTx<'_>,
    scope: &AccessScope,
    tenant_id: Uuid,
    request: SynthesisRequest,
) -> Result<crate::infra::storage::repo::synthesis_repo::Frozen, DomainError> {
    let outcome = resolve(
        txn,
        scope,
        tenant_id,
        request.source_plan_id,
        &request.keys,
        request.at,
    )
    .await?;
    outcome.ensure_complete(request.subscription_ref)?;

    let payload = materialize(
        txn,
        scope,
        tenant_id,
        request.source_plan_id,
        &outcome.selected,
    )
    .await?;

    // The revision the resolved rows belonged to, where they had one. Tier 2 has
    // none by construction, and `None` there is D-87's fact rather than a gap.
    let source_revision = outcome.selected.iter().find_map(|row| row.plan_revision);

    crate::infra::storage::repo::synthesis_repo::freeze_or_load(
        txn,
        scope,
        crate::infra::storage::repo::NewProvenance {
            provenance_id: Uuid::now_v7(),
            tenant_id,
            subscription_ref: request.subscription_ref,
            source_plan_id: request.source_plan_id,
            source_revision,
            snapshot_instant: request.at,
            trigger: request.trigger,
            acting_principal: request.acting_principal,
            resolved: resolved_json(&outcome.selected),
            payload,
            created_at: request.at,
        },
    )
    .await
    .map_err(|e| repo_failure(&e))
}
