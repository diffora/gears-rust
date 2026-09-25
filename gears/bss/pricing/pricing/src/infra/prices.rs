//! The `prices` approval subject (spec §6): draft prices of one book, optionally moved to a
//! common effective date, approved together inside the caller's transaction.
//!
//! No price is locked by the database: ownership is the conditional `pending_unit_id` write,
//! and `apply` re-reads every touched chain in the door's serializable transaction.
//!
//! @cpt-dod:cpt-cf-bss-pricing-dod-chain-windows:p1
//! @cpt-dod:cpt-cf-bss-pricing-dod-pair-guard:p1
//! @cpt-dod:cpt-cf-bss-pricing-dod-prices-unit:p1
//! @cpt-dod:cpt-cf-bss-pricing-dod-sod-excludes-authors:p1
use crate::{
    domain::{
        RuleError, book,
        price::{self, Eligibility, Price, PriceState, SkuMetering},
        price_book_entry::ChargeKind,
    },
    infra::{
        plan_revisions::{self, SUBSCRIPTIONS_UNAVAILABLE},
        reference_registry, reference_work,
        storage::{
            RepoError,
            entity::{self, price_book_entry},
            repo::{
                book_repo, dimension_repo, plan_item_repo, plan_repo, plan_revision_repo,
                price_book_entry_repo, price_repo,
            },
        },
    },
};
use bss_approval::{ApprovalError, ApprovalSubject, ItemRef, Unit};
use bss_products_sdk::{ReferenceRegistryV1, models::SkuVersion};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};
use time::{Date, OffsetDateTime};
use toolkit_canonical_errors::CanonicalError;
use toolkit_db::{
    DbTx,
    secure::{AccessScope, DBRunner},
};
use toolkit_security::SecurityContext;
use uuid::Uuid;

/// The approval kind of a batch of book prices.
pub const KIND_PRICES: &str = "prices";
/// A `prices` unit references its book.
pub const REF_TYPE: &str = "price_book";
/// Every item of a `prices` unit is one price.
pub const ITEM_TYPE: &str = "price";

/// What the pure rules need to judge a price of one entry.
pub struct PriceBookEntryContext {
    pub kind: ChargeKind,
    pub values: Option<Vec<String>>,
    pub digits: u32,
    pub prices: Vec<entity::price::Model>,
}
impl PriceBookEntryContext {
    /// Read the book currency, the declared dimension values and every price of the entry.
    /// # Errors
    /// Returns storage failures or a corrupt stored vocabulary.
    pub async fn load(
        tx: &impl DBRunner,
        tenant: Uuid,
        entry: &price_book_entry::Model,
    ) -> Result<Self, RepoError> {
        let children = AccessScope::for_tenant(tenant);
        let kind = entry
            .charge_kind
            .parse()
            .map_err(|_| RepoError::CorruptRow(format!("entry {} charge_kind", entry.id)))?;
        let book = book_repo::find(tx, &children, tenant, entry.book_id)
            .await?
            .ok_or_else(|| RepoError::CorruptRow(format!("entry {} has no book", entry.id)))?;
        let values = match &entry.dimension_key {
            Some(key) => dimension_repo::find(tx, &children, tenant, key)
                .await?
                .map(|d| {
                    serde_json::from_value::<Vec<String>>(d.values)
                        .map_err(|_| RepoError::CorruptRow(format!("dimension {key} values")))
                })
                .transpose()?,
            None => None,
        };
        let prices = price_repo::for_entry(tx, &children, tenant, entry.id).await?;
        Ok(Self {
            kind,
            values,
            digits: book::minor_digits(&book.currency),
            prices,
        })
    }
    /// Every price of the entry in the pure model.
    /// # Errors
    /// Returns a corrupt stored price.
    pub fn domain_prices(&self) -> Result<Vec<Price>, RepoError> {
        self.prices.iter().map(price_repo::to_domain).collect()
    }
    /// The first pure refusal of a candidate against the entry's approved prices.
    #[must_use]
    pub fn first_refusal(
        &self,
        candidate: &Price,
        siblings: &[Price],
        today: Date,
    ) -> Option<RuleError> {
        price::validate(
            candidate,
            self.kind,
            self.values.as_deref(),
            siblings,
            today,
            self.digits,
        )
        .first()
        .copied()
    }
}

/// What rejecting or withdrawing leaves on the unit's prices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Release {
    /// Withdraw: the prices are editable drafts again.
    Draft,
    /// Reject: the prices keep their review history and stay rejected.
    Rejected,
}

/// The subject of one `prices` unit of one book.
#[derive(Clone)]
pub struct PricesSubject {
    /// The caller; dated SKU reads are made on its behalf.
    pub ctx: SecurityContext,
    pub hub: Arc<toolkit::ClientHub>,
    pub tenant_id: Uuid,
    pub book_id: Uuid,
    pub now: OffsetDateTime,
    /// The unit's shared start; the door copies it from the unit when voting.
    pub common_effective_date: Option<Date>,
    /// Partners that publish-changes pulled in; recorded in the snapshot.
    pub added_partner: Vec<Uuid>,
    pub release: Release,
    /// A definite Products refusal met while judging, kept whole for the door: the approval
    /// error carries only static codes, and the caller must see Products' own status and code.
    refused: Arc<Mutex<Option<CanonicalError>>>,
    /// What `collect` gathers for the synchronous `snapshot` (D-408): the plans reading the
    /// unit's entries and each entry SKU's current descriptors, both outside `after`.
    review: Arc<Mutex<Review>>,
}

/// The reviewer's information about a unit that is never fingerprinted content.
#[derive(Default)]
struct Review {
    plans: Vec<Value>,
    /// The entry SKUs' descriptors, or `"unavailable"` when Products did not answer the read.
    descriptors: Value,
}

/// One entry's part of a unit, judged against the entry's current approved prices.
struct Judged {
    stored: Vec<entity::price::Model>,
    /// The unit prices as they will be approved: shifted, not yet normalised.
    proposed: Vec<Price>,
    /// Every approved price of the entry once the unit applies, normalised per chain.
    chain: Vec<Price>,
}

fn invalid(code: &'static str, detail: impl Into<String>) -> ApprovalError {
    ApprovalError::InvalidSubmit {
        code,
        field: price::field_of(code).into(),
        detail: detail.into(),
    }
}
fn rule(error: RuleError, id: Uuid) -> ApprovalError {
    invalid(error.code, format!("price {id}"))
}
/// Keep contention typed for the transaction's retry; a unique arbiter refuses the apply.
fn storage(error: RepoError) -> ApprovalError {
    match error {
        RepoError::Driver { source, .. } => ApprovalError::Db(source),
        RepoError::Conflict { code } => ApprovalError::ApplyRefused {
            code,
            detail: code.into(),
        },
        other => ApprovalError::Store(other.to_string()),
    }
}
/// A submit-time refusal met again at apply is an environment change.
fn applied(error: ApprovalError) -> ApprovalError {
    match error {
        ApprovalError::InvalidSubmit { code, detail, .. } => {
            ApprovalError::ApplyRefused { code, detail }
        }
        other => other,
    }
}
fn date(d: Date) -> String {
    d.to_string()
}
/// Proposed business content only: never state, lock, version or recomputed columns.
fn after(r: &Price, note: Option<&str>) -> Value {
    json!({
        "price_book_entry_id": r.price_book_entry_id,
        "version_no": r.version_no,
        "dim_value": r.dim_value,
        "model": r.model.as_str(),
        "price": r.price,
        "min_fee": r.min_fee.map(|v| v.to_string()),
        "eligibility": r.eligibility.as_str(),
        "effective_from": date(r.effective_from),
        "effective_to": r.effective_to.map(date),
        "temporary_until": r.temporary_until.map(date),
        "closed_explicitly": r.closed_explicitly,
        "paired_price_id": r.paired_price_id,
        "return_of_price_id": r.return_of_price_id,
        "note": note,
    })
}
fn before(r: &Price) -> Value {
    json!({
        "price_id": r.id,
        "version_no": r.version_no,
        "model": r.model.as_str(),
        "price": r.price,
        "min_fee": r.min_fee.map(|v| v.to_string()),
        "eligibility": r.eligibility.as_str(),
        "effective_from": date(r.effective_from),
        "effective_to": r.effective_to.map(date),
    })
}
/// The entries a unit's prices belong to, from their proposed content.
fn entries_of(items: &[ItemRef]) -> BTreeSet<Uuid> {
    items
        .iter()
        .filter_map(|i| i.after["price_book_entry_id"].as_str())
        .filter_map(|id| id.parse().ok())
        .collect()
}
/// The impact object every read shows (D-392): the queue card and list, the publish-changes
/// listing and the stored snapshot. Plans are the revisions whose items name one of the entries
/// (`plans_reading`); subscriptions wait for the Subscriptions integration.
#[must_use]
pub fn impact_of(prices: usize, entries: usize, plans: &[Value]) -> Value {
    json!({
        "prices": prices,
        "entries": entries,
        "plans": plans,
        "subscriptions": SUBSCRIPTIONS_UNAVAILABLE,
    })
}
/// Every plan revision, in any state, whose items name one of the entries, as
/// `{ plan_id, code, revision_id, rev_no, state }`, by plan code and revision number (D-408).
/// # Errors
/// Storage failures; a revision or plan an item points at that is gone is a corrupt row.
pub async fn plans_reading(
    tx: &impl DBRunner,
    tenant: Uuid,
    entries: &BTreeSet<Uuid>,
) -> Result<Vec<Value>, RepoError> {
    let scope = AccessScope::for_tenant(tenant);
    let ids: Vec<Uuid> = entries.iter().copied().collect();
    let revisions: BTreeSet<Uuid> = plan_item_repo::naming_entries(tx, &scope, tenant, &ids)
        .await?
        .into_iter()
        .map(|i| i.revision_id)
        .collect();
    let mut rows = Vec::with_capacity(revisions.len());
    for id in revisions {
        let r = plan_revision_repo::find(tx, &scope, tenant, id)
            .await?
            .ok_or_else(|| RepoError::CorruptRow(format!("plan item names lost revision {id}")))?;
        let p = plan_repo::find(tx, &scope, tenant, r.plan_id)
            .await?
            .ok_or_else(|| RepoError::CorruptRow(format!("revision {id} has no plan")))?;
        rows.push((p.code, r.rev_no, p.id, r.id, r.state));
    }
    rows.sort();
    Ok(rows
        .into_iter()
        .map(|(code, rev_no, plan_id, revision_id, state)| {
            json!({
                "plan_id": plan_id,
                "code": code,
                "revision_id": revision_id,
                "rev_no": rev_no,
                "state": state,
            })
        })
        .collect())
}
/// The live impact of a stored `prices` unit, recomputed on every read.
/// # Errors
/// Storage failures.
pub async fn live_impact(
    tx: &impl DBRunner,
    tenant: Uuid,
    items: &[ItemRef],
) -> Result<Value, RepoError> {
    let entries = entries_of(items);
    let plans = plans_reading(tx, tenant, &entries).await?;
    Ok(impact_of(items.len(), entries.len(), &plans))
}
fn by_entry(models: Vec<entity::price::Model>) -> BTreeMap<Uuid, Vec<entity::price::Model>> {
    let mut groups: BTreeMap<Uuid, Vec<entity::price::Model>> = BTreeMap::new();
    for m in models {
        groups.entry(m.price_book_entry_id).or_default().push(m);
    }
    groups
}

impl PricesSubject {
    /// A subject for the caller's tenant; no shift, no pulled-in partner, withdraw semantics.
    #[must_use]
    pub fn new(
        ctx: SecurityContext,
        hub: Arc<toolkit::ClientHub>,
        book_id: Uuid,
        now: OffsetDateTime,
    ) -> Self {
        let tenant_id = ctx.subject_tenant_id();
        Self {
            ctx,
            hub,
            tenant_id,
            book_id,
            now,
            common_effective_date: None,
            added_partner: Vec::new(),
            release: Release::Draft,
            refused: Arc::default(),
            review: Arc::default(),
        }
    }
    /// The Products refusal that ended the last judgement, if any; the door answers it as is.
    #[must_use]
    pub fn take_refusal(&self) -> Option<CanonicalError> {
        self.refused.lock().ok().and_then(|mut slot| slot.take())
    }
    /// Only unavailability (5xx, timeouts, rate limits, lost races) is `REGISTRY_UNAVAILABLE`;
    /// a definite refusal (403, 404, …) passes through with its code.
    fn registry_failure(&self, error: CanonicalError) -> ApprovalError {
        if !reference_work::definite_refusal(&error) {
            return invalid("REGISTRY_UNAVAILABLE", "Products reference registry");
        }
        if let Ok(mut slot) = self.refused.lock() {
            *slot = Some(error);
        }
        invalid("REGISTRY_REFUSED", "Products refused the dated SKU read")
    }
    async fn version_on(
        &self,
        registry: &dyn ReferenceRegistryV1,
        sku: Uuid,
        on: Date,
    ) -> Result<Option<SkuVersion>, ApprovalError> {
        registry
            .sku_version_as_of(&self.ctx, self.tenant_id, sku, on)
            .await
            .map_err(|e| self.registry_failure(e))
    }
    /// The SKU's first version: the latest, then each predecessor until none is older.
    async fn earliest(
        &self,
        registry: &dyn ReferenceRegistryV1,
        sku: Uuid,
    ) -> Result<Option<SkuVersion>, ApprovalError> {
        let Some(mut first) = self.version_on(registry, sku, Date::MAX).await? else {
            return Ok(None);
        };
        while let Some(eve) = first.effective_from.previous_day() {
            match self.version_on(registry, sku, eve).await? {
                Some(older) if older.effective_from < first.effective_from => first = older,
                _ => break,
            }
        }
        Ok(Some(first))
    }
    fn scope(&self) -> AccessScope {
        AccessScope::for_tenant(self.tenant_id)
    }
    /// The plans reading the unit's entries and each entry SKU's current descriptors, read fresh
    /// for the reviewer (D-408) and kept for `snapshot`, never in `after`. The descriptors are
    /// information, so their read is best-effort (D-416): a registry that cannot answer or refuses
    /// the caller records them `"unavailable"` and never refuses the submit, vote or reject. The
    /// reads a rule needs (a usage chain's dated metering, D-402) stay hard in `judge`.
    async fn review(&self, tx: &DbTx<'_>, entries: &BTreeSet<Uuid>) -> Result<(), ApprovalError> {
        let plans = plans_reading(tx, self.tenant_id, entries)
            .await
            .map_err(storage)?;
        let mut skus = BTreeSet::new();
        for id in entries {
            if let Some(entry) = price_book_entry_repo::find(tx, &self.scope(), self.tenant_id, *id)
                .await
                .map_err(storage)?
            {
                skus.insert(entry.sku_id);
            }
        }
        let descriptors = plan_revisions::descriptors_or_unavailable(
            crate::api::rest::authoring::plans::fresh_skus(&self.hub, &self.ctx, skus).await,
        );
        if let Ok(mut review) = self.review.lock() {
            review.plans = plans;
            review.descriptors = descriptors;
        }
        Ok(())
    }
    async fn load(&self, tx: &DbTx<'_>, id: Uuid) -> Result<entity::price::Model, ApprovalError> {
        price_repo::find(tx, &self.scope(), self.tenant_id, id)
            .await
            .map_err(storage)?
            .ok_or_else(|| invalid("PRICE_NOT_FOUND", format!("price {id}")))
    }
    async fn load_all(
        &self,
        tx: &DbTx<'_>,
        ids: impl IntoIterator<Item = Uuid>,
    ) -> Result<Vec<entity::price::Model>, ApprovalError> {
        let mut ids: Vec<Uuid> = ids.into_iter().collect();
        ids.sort_unstable();
        ids.dedup();
        let mut models = Vec::with_capacity(ids.len());
        for id in ids {
            models.push(self.load(tx, id).await?);
        }
        Ok(models)
    }
    /// The SKU's metering in force on a date (D-402). A date before the SKU's first version
    /// reads that first version's metering, never "none".
    async fn metering(&self, sku: Uuid, on: Date) -> Result<SkuMetering, ApprovalError> {
        let registry = reference_registry::resolve(&self.hub)
            .map_err(|_| invalid("REGISTRY_UNAVAILABLE", "Products reference registry"))?;
        let version = match self.version_on(registry.as_ref(), sku, on).await? {
            Some(version) => Some(version),
            None => self.earliest(registry.as_ref(), sku).await?,
        };
        Ok(version.map_or(
            SkuMetering {
                unit: None,
                usage_type_ref: None,
            },
            |v| SkuMetering {
                unit: v.content.unit,
                usage_type_ref: v.content.usage_type_ref,
            },
        ))
    }
    /// The pair guard over every link of a usage chain that touches the unit.
    async fn guard(
        &self,
        sku: Uuid,
        chain: &[Price],
        unit: &BTreeSet<Uuid>,
    ) -> Result<(), ApprovalError> {
        for successor in chain {
            let Some(predecessor) = price::in_force_before(chain, successor) else {
                continue;
            };
            if !unit.contains(&successor.id) && !unit.contains(&predecessor.id) {
                continue;
            }
            let was = self.metering(sku, predecessor.effective_from).await?;
            let is = self.metering(sku, successor.effective_from).await?;
            price::chain_guard(ChargeKind::Usage, predecessor, &was, successor, &is)
                .map_err(|e| rule(e, successor.id))?;
        }
        Ok(())
    }
    /// Shift, validate against the CURRENT approved prices, normalise and guard one entry's prices.
    async fn judge(
        &self,
        tx: &DbTx<'_>,
        price_book_entry_id: Uuid,
        prices: &[entity::price::Model],
        shift: Option<Date>,
    ) -> Result<Judged, ApprovalError> {
        let entry =
            price_book_entry_repo::find(tx, &self.scope(), self.tenant_id, price_book_entry_id)
                .await
                .map_err(storage)?
                .ok_or_else(|| {
                    invalid("ENTRY_NOT_FOUND", format!("entry {price_book_entry_id}"))
                })?;
        if entry.book_id != self.book_id {
            return Err(invalid(
                "PRICE_NOT_IN_BOOK",
                format!("entry {price_book_entry_id}"),
            ));
        }
        if entry.reference_state == "lost" {
            return Err(invalid(
                "ENTRY_REFERENCE_LOST",
                format!("entry {price_book_entry_id}"),
            ));
        }
        let pc = PriceBookEntryContext::load(tx, self.tenant_id, &entry)
            .await
            .map_err(storage)?;
        let unit: BTreeSet<Uuid> = prices.iter().map(|m| m.id).collect();
        let siblings: Vec<Price> = pc
            .domain_prices()
            .map_err(storage)?
            .into_iter()
            .filter(|r| r.state == PriceState::Approved && !unit.contains(&r.id))
            .collect();
        let drafts = prices
            .iter()
            .map(price_repo::to_domain)
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        let proposed =
            price::shift_selection(&drafts, shift).map_err(|e| rule(e, price_book_entry_id))?;
        let today = self.now.date();
        let mut starts = BTreeSet::new();
        for r in &proposed {
            if let Some(error) = pc.first_refusal(r, &siblings, today) {
                return Err(rule(error, r.id));
            }
            if !starts.insert((r.dim_value.clone(), r.effective_from)) {
                return Err(invalid("WINDOW_OVERLAP", format!("price {}", r.id)));
            }
        }
        // D-391: a return restores what the approved chain applies on the shifted end NOW; the
        // copy made at drafting is stale once another unit (or the common date) changed that.
        // The author re-drafts the pair.
        if let Some(stale) = proposed
            .iter()
            .find(|r| !price::temporary_is_current(&siblings, r, &proposed))
        {
            return Err(invalid("PAIR_RETURN_STALE", format!("price {}", stale.id)));
        }
        // D-406: a temporary window is not crossed, by the approved chain or by the unit itself.
        let mut around = siblings.clone();
        around.extend(proposed.iter().cloned());
        if let Some((r, promo)) = proposed
            .iter()
            .find_map(|r| price::temporary_holding(r, &around).map(|t| (r, t)))
        {
            return Err(invalid(
                "PRICE_INSIDE_TEMPORARY",
                format!("price {} starts inside temporary price {}", r.id, promo.id),
            ));
        }
        if let Some((r, start)) = proposed
            .iter()
            .find_map(|r| price::start_spanned(r, &around).map(|s| (r, s)))
        {
            return Err(invalid(
                "TEMPORARY_SPANS_A_CHANGE",
                format!(
                    "temporary price {} spans the start of price {}",
                    r.id, start.id
                ),
            ));
        }
        let mut chain = siblings;
        chain.extend(proposed.iter().cloned().map(|mut r| {
            r.state = PriceState::Approved;
            r
        }));
        price::normalize_windows(&mut chain);
        if pc.kind == ChargeKind::Usage {
            self.guard(entry.sku_id, &chain, &unit).await?;
        }
        Ok(Judged {
            stored: pc.prices,
            proposed,
            chain,
        })
    }
}

#[async_trait::async_trait]
impl<'a> ApprovalSubject<DbTx<'a>> for PricesSubject {
    fn kind(&self) -> &'static str {
        KIND_PRICES
    }
    fn ref_type(&self) -> &'static str {
        REF_TYPE
    }
    /// The prices, their pair partners, and each price's chain predecessor on its new start.
    async fn collect(&self, tx: &DbTx<'a>, ids: &[Uuid]) -> Result<Vec<ItemRef>, ApprovalError> {
        let mut wanted: BTreeSet<Uuid> = ids.iter().copied().collect();
        for id in ids {
            if let Some(partner) = self.load(tx, *id).await?.paired_price_id {
                wanted.insert(partner);
            }
        }
        let mut items = Vec::new();
        let grouped = by_entry(self.load_all(tx, wanted).await?);
        self.review(tx, &grouped.keys().copied().collect()).await?;
        for (price_book_entry_id, prices) in grouped {
            let unit: BTreeSet<Uuid> = prices.iter().map(|m| m.id).collect();
            let mut chain: Vec<Price> =
                price_repo::for_entry(tx, &self.scope(), self.tenant_id, price_book_entry_id)
                    .await
                    .map_err(storage)?
                    .iter()
                    .filter(|m| m.state == PriceState::Approved.as_str() && !unit.contains(&m.id))
                    .map(price_repo::to_domain)
                    .collect::<Result<_, _>>()
                    .map_err(storage)?;
            let drafts = prices
                .iter()
                .map(price_repo::to_domain)
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage)?;
            let proposed = price::shift_selection(&drafts, self.common_effective_date)
                .map_err(|e| rule(e, price_book_entry_id))?;
            chain.extend(proposed.iter().cloned().map(|mut r| {
                r.state = PriceState::Approved;
                r
            }));
            price::normalize_windows(&mut chain);
            for (m, r) in prices.iter().zip(&proposed) {
                let predecessor = chain
                    .iter()
                    .find(|c| c.id == r.id)
                    .and_then(|c| price::in_force_before(&chain, c));
                items.push(ItemRef {
                    item_type: ITEM_TYPE.into(),
                    item_id: m.id,
                    created_by: m.created_by,
                    before: predecessor.map(before),
                    after: after(r, m.note.as_deref()),
                });
            }
        }
        Ok(items)
    }
    /// Drafts only, whole pairs only, one book, and the pure rules against the current chains.
    async fn validate_submit(&self, tx: &DbTx<'a>, items: &[ItemRef]) -> Result<(), ApprovalError> {
        let ids: BTreeSet<Uuid> = items.iter().map(|i| i.item_id).collect();
        let models = self.load_all(tx, ids.iter().copied()).await?;
        for m in &models {
            if m.state != PriceState::Draft.as_str() || m.pending_unit_id.is_some() {
                return Err(invalid("PRICE_NOT_DRAFT", format!("price {}", m.id)));
            }
            if m.paired_price_id.is_some_and(|p| !ids.contains(&p)) {
                return Err(invalid("PAIR_SPLIT", format!("price {}", m.id)));
            }
        }
        for (price_book_entry_id, prices) in by_entry(models) {
            self.judge(tx, price_book_entry_id, &prices, self.common_effective_date)
                .await?;
        }
        Ok(())
    }
    /// Conditional ownership in ascending price id; a lost write refuses the whole submit.
    async fn lock(
        &self,
        tx: &DbTx<'a>,
        unit_id: Uuid,
        items: &[ItemRef],
    ) -> Result<(), ApprovalError> {
        for m in self.load_all(tx, items.iter().map(|i| i.item_id)).await? {
            if !price_repo::try_lock(tx, &self.scope(), self.tenant_id, m.id, unit_id, m.version)
                .await
                .map_err(storage)?
            {
                return Err(ApprovalError::Locked {
                    item_type: ITEM_TYPE.into(),
                    item_id: m.id,
                });
            }
        }
        Ok(())
    }
    fn snapshot(&self, items: &[ItemRef], common_effective_date: Option<Date>) -> Value {
        let (plans, descriptors) = self.review.lock().map_or_else(
            |_| (Vec::new(), Value::Null),
            |r| (r.plans.clone(), r.descriptors.clone()),
        );
        json!({
            "book_id": self.book_id,
            "common_effective_date": common_effective_date.map(date),
            "prices": items
                .iter()
                .map(|i| json!({"price_id": i.item_id, "before": i.before, "after": i.after}))
                .collect::<Vec<_>>(),
            "added_partner": self.added_partner,
            "impact": impact_of(items.len(), entries_of(items).len(), &plans),
            "descriptors": descriptors,
            "computed_at": self
                .now
                .format(&time::format_description::well_known::Rfc3339)
                .ok(),
        })
    }
    /// Re-judge every touched chain, then approve, re-close predecessors and mark
    /// `keep_for_bound`; any refusal rolls the whole unit back as `APPLY_REFUSED`.
    async fn apply(
        &self,
        tx: &DbTx<'a>,
        unit: &Unit,
        items: &[ItemRef],
    ) -> Result<(), ApprovalError> {
        let models = self.load_all(tx, items.iter().map(|i| i.item_id)).await?;
        if let Some(m) = models
            .iter()
            .find(|m| m.pending_unit_id != Some(unit.id) || m.state != "pending")
        {
            return Err(ApprovalError::ApplyRefused {
                code: "PRICE_NOT_PENDING",
                detail: format!("price {}", m.id),
            });
        }
        // Entries in ascending id: two batches over the same entries meet in one order.
        for (price_book_entry_id, prices) in by_entry(models) {
            let judged = self
                .judge(tx, price_book_entry_id, &prices, unit.common_effective_date)
                .await
                .map_err(applied)?;
            let normalised = |id: Uuid| judged.chain.iter().find(|c| c.id == id);
            // The current predecessor of EVERY `new` price of a touched chain binds renewals,
            // whether the `new` price is in this unit or was approved earlier and a price of this
            // unit now sits in front of it. A mark is never cleared.
            let touched: BTreeSet<Option<&str>> = judged
                .proposed
                .iter()
                .map(|r| r.dim_value.as_deref())
                .collect();
            let mut keep = BTreeSet::new();
            for c in judged.chain.iter().filter(|c| {
                c.eligibility == Eligibility::New && touched.contains(&c.dim_value.as_deref())
            }) {
                if let Some(predecessor) = price::in_force_before(&judged.chain, c) {
                    keep.insert(predecessor.id);
                }
            }
            for r in &judged.proposed {
                price_repo::approve(
                    tx,
                    &self.scope(),
                    self.tenant_id,
                    r.id,
                    unit.id,
                    price_repo::Approval {
                        effective_from: r.effective_from,
                        effective_to: normalised(r.id).and_then(|c| c.effective_to),
                        temporary_until: r.temporary_until,
                        keep_for_bound: keep.contains(&r.id),
                    },
                    self.now,
                )
                .await
                .map_err(storage)?;
            }
            for stored in judged
                .stored
                .iter()
                .filter(|m| m.state == PriceState::Approved.as_str())
            {
                let Some(chain) = normalised(stored.id) else {
                    continue;
                };
                let keep_for_bound = stored.keep_for_bound || keep.contains(&stored.id);
                if chain.effective_to != stored.effective_to
                    || keep_for_bound != stored.keep_for_bound
                {
                    price_repo::set_window(
                        tx,
                        &self.scope(),
                        self.tenant_id,
                        stored.id,
                        stored.version,
                        chain.effective_to,
                        keep_for_bound,
                        self.now,
                    )
                    .await
                    .map_err(|e| match e {
                        RepoError::Conflict { .. } => ApprovalError::Contended,
                        other => storage(other),
                    })?;
                }
            }
        }
        Ok(())
    }
    async fn unlock(
        &self,
        tx: &DbTx<'a>,
        unit: &Unit,
        items: &[ItemRef],
        approved: bool,
    ) -> Result<(), ApprovalError> {
        let outcome = match (approved, self.release) {
            (true, _) => price_repo::Unlock::Approved,
            (false, Release::Draft) => price_repo::Unlock::Draft,
            (false, Release::Rejected) => price_repo::Unlock::Rejected,
        };
        for item in items {
            price_repo::unlock(
                tx,
                &self.scope(),
                self.tenant_id,
                item.item_id,
                unit.id,
                outcome,
            )
            .await
            .map_err(|e| match e {
                RepoError::Conflict { .. } => {
                    ApprovalError::Store("price lock is not owned by this unit".into())
                }
                other => storage(other),
            })?;
        }
        Ok(())
    }
}
