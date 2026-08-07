//! The four tenant taxonomies' read and write surface —
//! `design/04-currency-tax.md` §5's `GET/PUT /config/taxonomies/{…}`, §6's table,
//! and `inst-tx-mutation`'s retire guard.
//!
//! # This is the repository the four tables have been waiting for
//!
//! The tables landed on Slice 9's chain (`m20260802_000028`…`000031`) because
//! `inst-plv-scope` had to validate an overlay's scope value against *something*
//! and the alternative was shipping a rule against thin air — the D-211 defect
//! named in their own migration docs. What they got was one read,
//! [`overlay_repo::taxonomy_declares`](super::overlay_repo::OverlayRepo::taxonomy_declares),
//! and **no writer at all**. An operator could not author a brand-scoped overlay
//! end to end, because there was nowhere to put the brand.
//!
//! So the read here is deliberately *not* a second copy of that one. This module
//! owns the **set** reads an operator and the publish pipeline need — the whole
//! declared list, the active region universe, and D-01's readiness markers —
//! while the single-value membership probe stays where Slice 9 put it. Two
//! functions answering "is this value declared" would be two predicates to keep
//! in step, and the one that drifted would be the one nobody was looking at.
//!
//! # The `PUT` is the whole set, and absence is retirement rather than deletion
//!
//! §5 gives this resource an `ETag` and §6 gives its rows a `state` with a
//! guarded transition and a legal way back. Both point the same way: the resource
//! is *the tenant's taxonomy*, not a row, so the `PUT` carries the complete value
//! set exactly as the approval-threshold policy's does — *"the **whole** policy,
//! not a patch"*.
//!
//! What absence means is the one real decision, and it is **retire, never
//! delete**. Deleting would break the guard's whole purpose: a value a published
//! row names has to keep existing, because the row keeps naming it. Retiring is
//! the state §6 declares for exactly this, it is reversible (*"a `PUT` re-adding
//! an existing retired value re-activates it"*), and it is what makes the
//! resource a faithful representation — a value the operator left out of the
//! body is a value they are done with, and the response says so rather than
//! silently keeping it active.
//!
//! **A retirement the guard refuses fails the whole `PUT`.** One transaction, one
//! verdict: a partial application would leave the tenant holding a taxonomy that
//! is neither what they sent nor what they had, and the `ETag` they read it back
//! under would describe neither.
//!
//! # What counts as a reference, and the reading that is deliberate
//!
//! §3 step 3 refuses a retirement while the value is referenced by *"an active
//! published price row (`region`) **or** an active `PriceOverlay` scope of any
//! taxonomy-backed class"*. Both counts are over `lifecycle_state = 'published'`.
//!
//! `superseded` is deliberately **not** counted, and the argument is that
//! retiring a taxonomy value does not touch the rows that name it. A superseded
//! row keeps its `region` string, keeps its window, and keeps resolving for the
//! arrears periods `PROJECTED_ROW_STATES` exists to preserve; what retirement
//! withdraws is the value's ability to validate something *new*. Counting
//! history would refuse the retirement of every region a tenant ever
//! repriced — which is to say, of every region they actually use.
//!
//! `draft` is not counted for the mirror reason: a draft is not yet a
//! commitment, and a value blocked by somebody's unpublished experiment is a
//! value no operator can retire on a schedule.

use std::collections::{BTreeMap, BTreeSet};

use sea_orm::{ColumnTrait, Condition, EntityTrait, Set};
use toolkit_db::secure::{
    AccessScope, DBRunner, SecureEntityExt, SecureInsertExt, SecureUpdateExt,
};
use toolkit_db::{DBProvider, DbError};
use uuid::Uuid;

use crate::domain::audit::{AuditAction, AuditStamp, AuditSubjectKind};
use crate::domain::concurrency::PolicyTag;
use crate::domain::overlay::ScopeValue;
use crate::domain::scope_key::Region;
use crate::domain::taxonomy::{
    RegionTaxMarkers, TaxonomyClass, TaxonomyEntry, TaxonomyState, ValueReferences,
    check_retirable, tag_of,
};
use crate::domain::validation::ValidationReport;
use crate::infra::storage::entity::{
    brand_taxonomy, org_tier_taxonomy, partner_taxonomy, price, price_overlay, region_taxonomy,
};
use crate::infra::storage::{RepoError, contention_or_db};

use super::audit_repo::{self, NewAuditEntry};

/// The `lifecycle_state` a reference has to be in to block a retirement.
///
/// One constant rather than a literal at each of the two count sites: the two
/// counts are the same rule over two planes, and a spelling that drifted on one
/// of them would silently stop guarding that plane.
const PUBLISHED: &str = "published";

/// The `state` token an entry has to carry to declare anything.
const ACTIVE: &str = "active";

// ---------------------------------------------------------------------------
// The repository.
// ---------------------------------------------------------------------------

/// Reader and writer of the four tenant taxonomies.
#[derive(Clone)]
pub struct TaxonomyRepo {
    db: DBProvider<DbError>,
}

impl TaxonomyRepo {
    /// Build over one database provider.
    #[must_use]
    pub const fn new(db: DBProvider<DbError>) -> Self {
        Self { db }
    }

    /// Every declared value of one class, `active` and `retired` alike, ordered
    /// by value.
    ///
    /// **Retired values are in the list**, which is what makes the `PUT`'s
    /// round trip honest: an operator who reads, edits and writes back must be
    /// able to see the value they are about to re-activate, and a `GET` hiding
    /// retirements would make re-activation reachable only by guessing a string.
    ///
    /// The order is the repository's rather than the caller's so two reads of an
    /// unchanged taxonomy render one `ETag`; `BTreeMap`'s ordering is the same
    /// one, which is what keeps the digest stable.
    ///
    /// SQL-level BOLA: a foreign tenant's taxonomy reads empty.
    ///
    /// # Errors
    /// [`RepoError::Db`] on a scope or storage failure;
    /// [`RepoError::CorruptRow`] when a stored `state` or `value` is outside
    /// what its `CHECK` admits.
    pub async fn list(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        class: TaxonomyClass,
    ) -> Result<Vec<TaxonomyEntry>, RepoError> {
        let conn = self
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("taxonomy conn: {e}")))?;
        list_on(&conn, scope, tenant_id, class).await
    }

    /// Replace one class's whole value set (§5's `PUT`).
    ///
    /// Returns the taxonomy as it now stands, so the caller holds the
    /// representation its `ETag` covers without a second read.
    ///
    /// # Errors
    /// [`RepoError::Db`] on a scope or storage failure;
    /// [`RepoError::CorruptRow`] on an unreadable stored row. A refused
    /// retirement is **not** an error here — it comes back as a non-empty
    /// [`ValidationReport`] beside the unchanged taxonomy, because §5 types it
    /// `TAXONOMY_VALUE_IN_USE` (409) and a caller has to be able to render the
    /// code and the counts rather than a storage failure.
    pub async fn replace(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        class: TaxonomyClass,
        entries: Vec<TaxonomyEntry>,
        asserted: &PolicyTag,
        stamp: AuditStamp,
    ) -> Result<Replaced, RepoError> {
        let scope = scope.clone();
        let asserted = asserted.clone();
        let (_, outcome) = self
            .db
            .db()
            .in_transaction::<Replaced, RepoError, _>(move |txn| {
                Box::pin(async move {
                    apply_replace(txn, &scope, tenant_id, class, entries, &asserted, stamp).await
                })
            })
            .await;
        // `into_domain`, never a blanket `Db`: `in_transaction` answers
        // `TxError<RepoError>`, and flattening it turned a retriable
        // `ConcurrentMutation` into `Internal` — a **500** for a request whose
        // whole remedy is to retry. Every other repository here unwraps it this
        // way for exactly that reason.
        outcome
            .map_err(|e| e.into_domain(|infra| RepoError::Db(format!("taxonomy replace: {infra}"))))
    }
}

/// What a `PUT` did, or refused to do.
///
/// A pair rather than a `Result`, because a refused retirement is a **domain**
/// answer carrying §5's code and the two reference counts, not a storage fault —
/// and because the caller renders the taxonomy either way: on refusal it is the
/// unchanged one, which is what the operator has to re-author against.
#[derive(Clone, Debug)]
pub struct Replaced {
    /// The taxonomy as it stands after the call.
    pub entries: Vec<TaxonomyEntry>,
    /// Empty on success; `TAXONOMY_VALUE_IN_USE` violations otherwise.
    pub report: ValidationReport,
    /// The asserted `If-Match` tag no longer described the stored taxonomy, so
    /// **nothing was written**.
    ///
    /// A flag rather than a `RepoError`, for the same reason `report` is not one:
    /// §5 types this refusal 409 `STALE_VERSION` and the caller renders the
    /// taxonomy either way — on refusal it is the one the operator must
    /// re-author against.
    pub stale: bool,
}

// ---------------------------------------------------------------------------
// The publish-time reads.
// ---------------------------------------------------------------------------

/// The tenant's **active** region values — `inst-tx-region`'s universe.
///
/// Called by `infra::publish::rule_params` — `inst-tx-region`'s universe — and
/// by the price authoring route for the save-time half.
///
/// It takes a runner rather than a provider so `rule_params` can resolve it
/// **inside** the commit transaction: §4.2 runs the rule set twice, and a read
/// that could not join the transaction would answer the second run against a
/// world the commit is not holding.
///
/// `active` only, which is `overlay_repo::declares`' predicate one plane over: a
/// value that reached `retired` anyway must not validate a new row against
/// itself.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure; [`RepoError::CorruptRow`]
/// when a stored value is blank, which its `CHECK` refuses.
pub async fn active_regions(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
) -> Result<BTreeSet<Region>, RepoError> {
    let rows = region_taxonomy::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(region_taxonomy::Column::TenantId.eq(tenant_id))
                .add(region_taxonomy::Column::State.eq(ACTIVE)),
        )
        .all(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read pricing_region_taxonomy: {e}")))?;
    rows.into_iter()
        .map(|row| {
            Region::new(&row.value).map_err(|e| {
                RepoError::CorruptRow(format!(
                    "pricing_region_taxonomy.value `{}`: {e}",
                    row.value
                ))
            })
        })
        .collect()
}

/// C4's `RegionTaxReadiness` lookup: `(tenant, region) -> { taxCategory, ratePresent }`.
///
/// **`None` is an unknown region and C4 fails closed on it** — *"Readiness is
/// resolved per `(tenant, region)`; an unknown region fails closed"*. That is a
/// different fact from a *declared* region with no default category, which comes
/// back as `Some` with `tax_category: None` and which `inst-td-policy`'s coalesce
/// is entirely about. Collapsing the two would make a row whose own
/// `tax_category_ref` satisfies the check indistinguishable from a row in a
/// region nobody declared.
///
/// A **retired** region reads as `None` for [`active_regions`]' reason: it
/// declares nothing, so it has no readiness to report.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure.
pub async fn region_readiness(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    region: &Region,
) -> Result<Option<RegionTaxMarkers>, RepoError> {
    let found = region_taxonomy::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(region_taxonomy::Column::TenantId.eq(tenant_id))
                .add(region_taxonomy::Column::Value.eq(region.as_str()))
                .add(region_taxonomy::Column::State.eq(ACTIVE)),
        )
        .one(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read pricing_region_taxonomy: {e}")))?;
    Ok(found.map(|row| RegionTaxMarkers {
        tax_category: row.tax_category,
        tax_rate_present: row.tax_rate_present,
    }))
}

/// Every declared region's readiness in one read — the publish path's shape.
///
/// One statement rather than one per row, for `rule_params`' reason: a plan at
/// C1's 20-currency floor spans as many markets, and twenty round trips inside
/// the commit transaction is twenty chances to hold it open longer than it needs.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure.
pub async fn region_readiness_map(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
) -> Result<BTreeMap<String, RegionTaxMarkers>, RepoError> {
    let rows = region_taxonomy::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(region_taxonomy::Column::TenantId.eq(tenant_id))
                .add(region_taxonomy::Column::State.eq(ACTIVE)),
        )
        .all(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read pricing_region_taxonomy: {e}")))?;
    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.value,
                RegionTaxMarkers {
                    tax_category: row.tax_category,
                    tax_rate_present: row.tax_rate_present,
                },
            )
        })
        .collect())
}

// ---------------------------------------------------------------------------
// The reference counts behind `inst-tx-mutation`.
// ---------------------------------------------------------------------------

/// What still names `value`, across both planes §3 step 3 enumerates.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure.
pub async fn references_to(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    class: TaxonomyClass,
    value: &ScopeValue,
) -> Result<ValueReferences, RepoError> {
    // Only `region` is an axis of a price row: §3 step 2 is explicit that
    // `brand` is "**not** a price-row field (Foundation §4.1)", and the same is
    // true of the two D-120 classes. Counting the row plane for them would be a
    // query that can only ever answer zero.
    let published_price_rows = if class == TaxonomyClass::Region {
        price::Entity::find()
            .secure()
            .scope_with(scope)
            .filter(
                Condition::all()
                    .add(price::Column::TenantId.eq(tenant_id))
                    .add(price::Column::Region.eq(value.as_str()))
                    .add(price::Column::LifecycleState.eq(PUBLISHED)),
            )
            .count(runner)
            .await
            .map_err(|e| RepoError::Db(format!("count pricing_price: {e}")))?
    } else {
        0
    };

    let active_overlay_scopes = price_overlay::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(price_overlay::Column::TenantId.eq(tenant_id))
                .add(price_overlay::Column::ScopeClass.eq(class.scope_class().as_str()))
                .add(price_overlay::Column::ScopeValue.eq(value.as_str()))
                .add(price_overlay::Column::LifecycleState.eq(PUBLISHED)),
        )
        .count(runner)
        .await
        .map_err(|e| RepoError::Db(format!("count pricing_price_overlay: {e}")))?;

    Ok(ValueReferences {
        published_price_rows,
        active_overlay_scopes,
    })
}

// ---------------------------------------------------------------------------
// The write.
// ---------------------------------------------------------------------------

async fn apply_replace(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    class: TaxonomyClass,
    entries: Vec<TaxonomyEntry>,
    asserted: &PolicyTag,
    stamp: AuditStamp,
) -> Result<Replaced, RepoError> {
    let held = list_on(runner, scope, tenant_id, class).await?;

    // **The `If-Match` premise is tested here, and only here** — D-186's division
    // of labour, and `infra::threshold::propose`'s arrangement exactly: the
    // transport refuses a request it cannot *understand*, and the store refuses
    // one whose premise has *moved*.
    //
    // A comparison in the handler reads the world, decides, and then hands the
    // decision to a statement that races it. That is not hypothetical here: the
    // `PUT` is a whole-set replacement, so two callers whose reads both precede
    // either commit each pass a handler-side check, and the second one's write
    // **retires** whatever the first added — a value the retire guard cannot
    // protect, because a value just created has no published references by
    // construction. Both callers then see 200.
    //
    // Computed from the same `held` the write below works from, so there is no
    // second read to disagree with.
    if tag_of(class, &held) != *asserted {
        return Ok(Replaced {
            entries: held,
            report: ValidationReport::default(),
            stale: true,
        });
    }
    let submitted: BTreeMap<String, TaxonomyEntry> = entries
        .into_iter()
        .map(|entry| (entry.value.as_str().to_owned(), entry))
        .collect();

    // Every retirement this request implies, both spellings of it: a value the
    // body left out, and a value the body carries with `state: retired`. They
    // are the same act and are guarded identically — an operator must not be
    // able to slip past the guard by choosing the other spelling.
    let mut report = ValidationReport::default();
    for existing in &held {
        let key = existing.value.as_str();
        let retiring = submitted
            .get(key)
            .is_none_or(|entry| entry.state == TaxonomyState::Retired);
        if !retiring || existing.state == TaxonomyState::Retired {
            // Already retired is not a retirement: re-asserting a value's
            // current state is a no-op, and guarding it would make a taxonomy
            // with one guarded retirement permanently un-`PUT`-able.
            continue;
        }
        let references = references_to(runner, scope, tenant_id, class, &existing.value).await?;
        report.absorb(check_retirable(class, &existing.value, references));
    }

    if !report.is_publishable() {
        // One transaction, one verdict. Nothing has been written yet, so the
        // taxonomy handed back is the one the operator has to re-author against.
        return Ok(Replaced {
            entries: held,
            report,
            stale: false,
        });
    }

    write_set(runner, scope, tenant_id, class, &held, &submitted).await?;
    let now = list_on(runner, scope, tenant_id, class).await?;
    record_mutation(runner, scope, tenant_id, class, stamp).await?;
    Ok(Replaced {
        entries: now,
        report,
        stale: false,
    })
}

/// Apply the submitted set over the held one.
///
/// Three moves and no fourth: update a value both sets carry, insert one only
/// the body carries, and retire one only the store carries. There is no delete —
/// see the module doc.
async fn write_set(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    class: TaxonomyClass,
    held: &[TaxonomyEntry],
    submitted: &BTreeMap<String, TaxonomyEntry>,
) -> Result<(), RepoError> {
    let held_keys: BTreeSet<&str> = held.iter().map(|e| e.value.as_str()).collect();

    for entry in submitted.values() {
        if held_keys.contains(entry.value.as_str()) {
            update_entry(runner, scope, tenant_id, class, entry).await?;
        } else {
            insert_entry(runner, scope, tenant_id, class, entry).await?;
        }
    }
    for existing in held {
        if submitted.contains_key(existing.value.as_str()) {
            continue;
        }
        let retired = TaxonomyEntry {
            state: TaxonomyState::Retired,
            ..existing.clone()
        };
        update_entry(runner, scope, tenant_id, class, &retired).await?;
    }
    Ok(())
}

/// `inst-tx-mutation`'s audit half: taxonomy mutation is *"tenant-admin config,
/// audited"*.
///
/// Written **inside** the same transaction as the mutation, which is the whole of
/// what D-14 asks for and the arrangement every other mutating path here holds
/// to: a record that commits with its mutation cannot be lost by a crash between
/// the two, and a failure to write it rolls the mutation back rather than leaving
/// a trail that is silently incomplete.
///
/// One record per `PUT`, not one per value. The act an operator performed is
/// *"replaced the brand taxonomy"*; splitting it into a record per changed row
/// would make one decision look like several and would leave an auditor unable to
/// tell a re-authoring from a burst of unrelated edits.
///
/// # The subject kind is `policy`, and that is a decision rather than a default
///
/// S5 §6's aggregate list is *plan, overlay, payer, policy, bulk operation*, and
/// [`AuditSubjectKind`] carries exactly the tokens that list and
/// `chk_pricing_approval_subject_kind` admit. **There is no `taxonomy` token in
/// either**, and minting one here would be minting a design-set token this gear
/// does not own — the standing rule that kept `Window` and `Policy` out until
/// their writers existed cuts the other way too.
///
/// `policy` is the right one of the five rather than the least wrong. A taxonomy
/// is per-tenant configuration governed by the same `config × write` gate as the
/// tax-display policy, it is a per-tenant singleton like the policy object, and
/// [`audit_repo::policy_chain`] is already *"the per-tenant singleton segment"*.
/// The `subject_ref` discriminates within it — `taxonomy/brand` against the
/// policy object's own ref — so an auditor asking *what happened to this tenant's
/// brand list* walks one segment and filters, exactly as they do for the
/// threshold policy's versions.
///
/// Recorded as `T-5` in the owed register so the aggregate list is amended
/// deliberately if the set wants a sixth.
async fn record_mutation(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    class: TaxonomyClass,
    stamp: AuditStamp,
) -> Result<(), RepoError> {
    audit_repo::append(
        runner,
        scope,
        NewAuditEntry {
            tenant_id,
            chain_id: audit_repo::policy_chain(),
            recorded_at: stamp.recorded_at,
            actor_principal_id: stamp.actor_principal_id,
            action: AuditAction::Update,
            subject_kind: AuditSubjectKind::Policy,
            subject_ref: taxonomy_ref(class),
            // A taxonomy is a value **set** and the record names the set, not a
            // diff of it: the before/after columns hold version refs
            // (`inst-au-complete`), and a taxonomy has no version to refer to.
            // Rendering the whole list into them would put unbounded tenant
            // configuration on the hash chain for no question it answers.
            before_state: None,
            after_state: None,
            // No approval unit: taxonomy mutation is CatalogAdmin config (§10),
            // not one of the D-10 always-material acts.
            approval_ref: None,
            correlation_id: stamp.correlation_id,
        },
    )
    .await
    .map(|_| ())
}

/// The audited subject: one taxonomy of one tenant.
#[must_use]
pub fn taxonomy_ref(class: TaxonomyClass) -> String {
    format!("taxonomy/{}", class.path_segment())
}

// ---------------------------------------------------------------------------
// Per-table statements.
//
// One arm per table because each is a distinct entity, which is
// `overlay_repo::declares`' arrangement and its argument: `SeaORM` gives four
// generated types with no common trait to write these against, and a macro over
// them would hide which columns each carries — the `tax_*` pair being on exactly
// one of the four.
// ---------------------------------------------------------------------------

async fn list_on(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    class: TaxonomyClass,
) -> Result<Vec<TaxonomyEntry>, RepoError> {
    let rows: Vec<(String, String, String, Option<RegionTaxMarkers>)> = match class {
        TaxonomyClass::Region => region_taxonomy::Entity::find()
            .secure()
            .scope_with(scope)
            .filter(Condition::all().add(region_taxonomy::Column::TenantId.eq(tenant_id)))
            .order_by(region_taxonomy::Column::Value, sea_orm::Order::Asc)
            .all(runner)
            .await
            .map_err(|e| RepoError::Db(format!("read pricing_region_taxonomy: {e}")))?
            .into_iter()
            .map(|r| {
                let markers = RegionTaxMarkers {
                    tax_category: r.tax_category,
                    tax_rate_present: r.tax_rate_present,
                };
                (r.value, r.display_name, r.state, Some(markers))
            })
            .collect(),
        TaxonomyClass::Brand => brand_taxonomy::Entity::find()
            .secure()
            .scope_with(scope)
            .filter(Condition::all().add(brand_taxonomy::Column::TenantId.eq(tenant_id)))
            .order_by(brand_taxonomy::Column::Value, sea_orm::Order::Asc)
            .all(runner)
            .await
            .map_err(|e| RepoError::Db(format!("read pricing_brand_taxonomy: {e}")))?
            .into_iter()
            .map(|r| (r.value, r.display_name, r.state, None))
            .collect(),
        TaxonomyClass::Partner => partner_taxonomy::Entity::find()
            .secure()
            .scope_with(scope)
            .filter(Condition::all().add(partner_taxonomy::Column::TenantId.eq(tenant_id)))
            .order_by(partner_taxonomy::Column::Value, sea_orm::Order::Asc)
            .all(runner)
            .await
            .map_err(|e| RepoError::Db(format!("read pricing_partner_taxonomy: {e}")))?
            .into_iter()
            .map(|r| (r.value, r.display_name, r.state, None))
            .collect(),
        TaxonomyClass::OrgTier => org_tier_taxonomy::Entity::find()
            .secure()
            .scope_with(scope)
            .filter(Condition::all().add(org_tier_taxonomy::Column::TenantId.eq(tenant_id)))
            .order_by(org_tier_taxonomy::Column::Value, sea_orm::Order::Asc)
            .all(runner)
            .await
            .map_err(|e| RepoError::Db(format!("read pricing_org_tier_taxonomy: {e}")))?
            .into_iter()
            .map(|r| (r.value, r.display_name, r.state, None))
            .collect(),
    };

    rows.into_iter()
        .map(|(value, display_name, state, tax)| {
            Ok(TaxonomyEntry {
                value: ScopeValue::new(&value).ok_or_else(|| {
                    RepoError::CorruptRow(format!("{}.value is blank", class.table()))
                })?,
                display_name,
                state: TaxonomyState::parse(&state).ok_or_else(|| {
                    RepoError::CorruptRow(format!("{}.state `{state}`", class.table()))
                })?,
                tax,
            })
        })
        .collect()
}

async fn insert_entry(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    class: TaxonomyClass,
    entry: &TaxonomyEntry,
) -> Result<(), RepoError> {
    let value = entry.value.as_str().to_owned();
    let display_name = entry.display_name.clone();
    let state = entry.state.as_str().to_owned();
    match class {
        TaxonomyClass::Region => {
            let markers = entry.tax.clone().unwrap_or_default();
            let row = region_taxonomy::ActiveModel {
                tenant_id: Set(tenant_id),
                value: Set(value),
                display_name: Set(display_name),
                state: Set(state),
                tax_category: Set(markers.tax_category),
                tax_rate_present: Set(markers.tax_rate_present),
            };
            region_taxonomy::Entity::insert(row.clone())
                .secure()
                .scope_with_model(scope, &row)
                .map_err(|e| RepoError::Db(format!("scope pricing_region_taxonomy: {e}")))?
                .exec(runner)
                .await
                .map(|_| ())
                .map_err(|e| {
                    contention_or_db(
                        &e,
                        "pricing_region_taxonomy",
                        "insert pricing_region_taxonomy",
                    )
                })
        }
        TaxonomyClass::Brand => {
            let row = brand_taxonomy::ActiveModel {
                tenant_id: Set(tenant_id),
                value: Set(value),
                display_name: Set(display_name),
                state: Set(state),
            };
            brand_taxonomy::Entity::insert(row.clone())
                .secure()
                .scope_with_model(scope, &row)
                .map_err(|e| RepoError::Db(format!("scope pricing_brand_taxonomy: {e}")))?
                .exec(runner)
                .await
                .map(|_| ())
                .map_err(|e| {
                    contention_or_db(
                        &e,
                        "pricing_brand_taxonomy",
                        "insert pricing_brand_taxonomy",
                    )
                })
        }
        TaxonomyClass::Partner => {
            let row = partner_taxonomy::ActiveModel {
                tenant_id: Set(tenant_id),
                value: Set(value),
                display_name: Set(display_name),
                state: Set(state),
            };
            partner_taxonomy::Entity::insert(row.clone())
                .secure()
                .scope_with_model(scope, &row)
                .map_err(|e| RepoError::Db(format!("scope pricing_partner_taxonomy: {e}")))?
                .exec(runner)
                .await
                .map(|_| ())
                .map_err(|e| {
                    contention_or_db(
                        &e,
                        "pricing_partner_taxonomy",
                        "insert pricing_partner_taxonomy",
                    )
                })
        }
        TaxonomyClass::OrgTier => {
            let row = org_tier_taxonomy::ActiveModel {
                tenant_id: Set(tenant_id),
                value: Set(value),
                display_name: Set(display_name),
                state: Set(state),
            };
            org_tier_taxonomy::Entity::insert(row.clone())
                .secure()
                .scope_with_model(scope, &row)
                .map_err(|e| RepoError::Db(format!("scope pricing_org_tier_taxonomy: {e}")))?
                .exec(runner)
                .await
                .map(|_| ())
                .map_err(|e| {
                    contention_or_db(
                        &e,
                        "pricing_org_tier_taxonomy",
                        "insert pricing_org_tier_taxonomy",
                    )
                })
        }
    }
}

async fn update_entry(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    class: TaxonomyClass,
    entry: &TaxonomyEntry,
) -> Result<(), RepoError> {
    let value = entry.value.as_str().to_owned();
    let display_name = entry.display_name.clone();
    let state = entry.state.as_str().to_owned();
    match class {
        TaxonomyClass::Region => {
            let markers = entry.tax.clone().unwrap_or_default();
            region_taxonomy::Entity::update_many()
                .secure()
                .scope_with(scope)
                .col_expr(
                    region_taxonomy::Column::DisplayName,
                    sea_orm::sea_query::Expr::value(display_name),
                )
                .col_expr(
                    region_taxonomy::Column::State,
                    sea_orm::sea_query::Expr::value(state),
                )
                .col_expr(
                    region_taxonomy::Column::TaxCategory,
                    sea_orm::sea_query::Expr::value(markers.tax_category),
                )
                .col_expr(
                    region_taxonomy::Column::TaxRatePresent,
                    sea_orm::sea_query::Expr::value(markers.tax_rate_present),
                )
                .filter(
                    Condition::all()
                        .add(region_taxonomy::Column::TenantId.eq(tenant_id))
                        .add(region_taxonomy::Column::Value.eq(value)),
                )
                .exec(runner)
                .await
                .map(|_| ())
                .map_err(|e| RepoError::Db(format!("update pricing_region_taxonomy: {e}")))
        }
        TaxonomyClass::Brand => brand_taxonomy::Entity::update_many()
            .secure()
            .scope_with(scope)
            .col_expr(
                brand_taxonomy::Column::DisplayName,
                sea_orm::sea_query::Expr::value(display_name),
            )
            .col_expr(
                brand_taxonomy::Column::State,
                sea_orm::sea_query::Expr::value(state),
            )
            .filter(
                Condition::all()
                    .add(brand_taxonomy::Column::TenantId.eq(tenant_id))
                    .add(brand_taxonomy::Column::Value.eq(value)),
            )
            .exec(runner)
            .await
            .map(|_| ())
            .map_err(|e| RepoError::Db(format!("update pricing_brand_taxonomy: {e}"))),
        TaxonomyClass::Partner => partner_taxonomy::Entity::update_many()
            .secure()
            .scope_with(scope)
            .col_expr(
                partner_taxonomy::Column::DisplayName,
                sea_orm::sea_query::Expr::value(display_name),
            )
            .col_expr(
                partner_taxonomy::Column::State,
                sea_orm::sea_query::Expr::value(state),
            )
            .filter(
                Condition::all()
                    .add(partner_taxonomy::Column::TenantId.eq(tenant_id))
                    .add(partner_taxonomy::Column::Value.eq(value)),
            )
            .exec(runner)
            .await
            .map(|_| ())
            .map_err(|e| RepoError::Db(format!("update pricing_partner_taxonomy: {e}"))),
        TaxonomyClass::OrgTier => org_tier_taxonomy::Entity::update_many()
            .secure()
            .scope_with(scope)
            .col_expr(
                org_tier_taxonomy::Column::DisplayName,
                sea_orm::sea_query::Expr::value(display_name),
            )
            .col_expr(
                org_tier_taxonomy::Column::State,
                sea_orm::sea_query::Expr::value(state),
            )
            .filter(
                Condition::all()
                    .add(org_tier_taxonomy::Column::TenantId.eq(tenant_id))
                    .add(org_tier_taxonomy::Column::Value.eq(value)),
            )
            .exec(runner)
            .await
            .map(|_| ())
            .map_err(|e| RepoError::Db(format!("update pricing_org_tier_taxonomy: {e}"))),
    }
}
