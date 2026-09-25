//! The `price_rows` approval subject (spec §6): draft rows of one book, optionally moved to a
//! common effective date, approved together inside the caller's transaction.
//!
//! No row is locked by the database: ownership is the conditional `pending_unit_id` write,
//! and `apply` re-reads every touched chain in the door's serializable transaction.
//!
//! @cpt-dod:cpt-cf-bss-pricing-dod-chain-windows:p1
//! @cpt-dod:cpt-cf-bss-pricing-dod-pair-guard:p1
//! @cpt-dod:cpt-cf-bss-pricing-dod-price-rows-unit:p1
//! @cpt-dod:cpt-cf-bss-pricing-dod-sod-excludes-authors:p1
use crate::{
    domain::{
        RuleError, book,
        price::ChargeKind,
        row::{self, Eligibility, Row, RowState, SkuMetering},
    },
    infra::{
        reference_registry,
        storage::{
            RepoError,
            entity::{price, price_row},
            repo::{book_repo, dimension_repo, price_repo, row_repo},
        },
    },
};
use bss_approval::{ApprovalError, ApprovalSubject, ItemRef, Unit};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use time::{Date, OffsetDateTime};
use toolkit_db::{
    DbTx,
    secure::{AccessScope, DBRunner},
};
use toolkit_security::SecurityContext;
use uuid::Uuid;

/// The approval kind of a batch of book rows.
pub const KIND_PRICE_ROWS: &str = "price_rows";
/// A `price_rows` unit references its book.
pub const REF_TYPE: &str = "price_book";
/// Every item of a `price_rows` unit is one row.
pub const ITEM_TYPE: &str = "price_row";
/// The honest answer for impact a later phase measures.
pub const UNAVAILABLE: &str = "unavailable until phase 3";

/// What the pure rules need to judge a row of one price.
pub struct PriceContext {
    pub kind: ChargeKind,
    pub values: Option<Vec<String>>,
    pub digits: u32,
    pub rows: Vec<price_row::Model>,
}
impl PriceContext {
    /// Read the book currency, the declared dimension values and every row of the price.
    /// # Errors
    /// Returns storage failures or a corrupt stored vocabulary.
    pub async fn load(
        tx: &impl DBRunner,
        tenant: Uuid,
        price: &price::Model,
    ) -> Result<Self, RepoError> {
        let children = AccessScope::for_tenant(tenant);
        let kind = price
            .charge_kind
            .parse()
            .map_err(|_| RepoError::CorruptRow(format!("price {} charge_kind", price.id)))?;
        let book = book_repo::find(tx, &children, tenant, price.book_id)
            .await?
            .ok_or_else(|| RepoError::CorruptRow(format!("price {} has no book", price.id)))?;
        let values = match &price.dimension_key {
            Some(key) => dimension_repo::find(tx, &children, tenant, key)
                .await?
                .map(|d| {
                    serde_json::from_value::<Vec<String>>(d.values)
                        .map_err(|_| RepoError::CorruptRow(format!("dimension {key} values")))
                })
                .transpose()?,
            None => None,
        };
        let rows = row_repo::for_price(tx, &children, tenant, price.id).await?;
        Ok(Self {
            kind,
            values,
            digits: book::minor_digits(&book.currency),
            rows,
        })
    }
    /// Every row of the price in the pure model.
    /// # Errors
    /// Returns a corrupt stored row.
    pub fn domain_rows(&self) -> Result<Vec<Row>, RepoError> {
        self.rows.iter().map(row_repo::to_domain).collect()
    }
    /// The first pure refusal of a candidate against the price's approved rows.
    #[must_use]
    pub fn first_refusal(
        &self,
        candidate: &Row,
        siblings: &[Row],
        today: Date,
    ) -> Option<RuleError> {
        row::validate(
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

/// What rejecting or withdrawing leaves on the unit's rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Release {
    /// Withdraw: the rows are editable drafts again.
    Draft,
    /// Reject: the rows keep their review history and stay rejected.
    Rejected,
}

/// The subject of one `price_rows` unit of one book.
#[derive(Clone)]
pub struct PriceRowsSubject {
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
}

/// One price's part of a unit, judged against the price's current approved rows.
struct Judged {
    stored: Vec<price_row::Model>,
    /// The unit rows as they will be approved: shifted, not yet normalised.
    proposed: Vec<Row>,
    /// Every approved row of the price once the unit applies, normalised per chain.
    chain: Vec<Row>,
}

fn invalid(code: &'static str, detail: impl Into<String>) -> ApprovalError {
    ApprovalError::InvalidSubmit {
        code,
        field: row::field_of(code).into(),
        detail: detail.into(),
    }
}
fn rule(error: RuleError, id: Uuid) -> ApprovalError {
    invalid(error.code, format!("price_row {id}"))
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
fn after(r: &Row, note: Option<&str>) -> Value {
    json!({
        "price_id": r.price_id,
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
        "paired_row_id": r.paired_row_id,
        "return_of_row_id": r.return_of_row_id,
        "note": note,
    })
}
fn before(r: &Row) -> Value {
    json!({
        "row_id": r.id,
        "version_no": r.version_no,
        "model": r.model.as_str(),
        "price": r.price,
        "min_fee": r.min_fee.map(|v| v.to_string()),
        "eligibility": r.eligibility.as_str(),
        "effective_from": date(r.effective_from),
        "effective_to": r.effective_to.map(date),
    })
}
/// Rows and prices a unit touches; plans and subscriptions arrive in phase 3.
#[must_use]
pub fn impact(items: &[ItemRef]) -> Value {
    let prices: BTreeSet<String> = items
        .iter()
        .filter_map(|i| i.after["price_id"].as_str().map(str::to_owned))
        .collect();
    json!({
        "rows": items.len(),
        "prices": prices.len(),
        "plans": UNAVAILABLE,
        "subscriptions": UNAVAILABLE,
    })
}
fn by_price(models: Vec<price_row::Model>) -> BTreeMap<Uuid, Vec<price_row::Model>> {
    let mut groups: BTreeMap<Uuid, Vec<price_row::Model>> = BTreeMap::new();
    for m in models {
        groups.entry(m.price_id).or_default().push(m);
    }
    groups
}

impl PriceRowsSubject {
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
        }
    }
    fn scope(&self) -> AccessScope {
        AccessScope::for_tenant(self.tenant_id)
    }
    async fn load(&self, tx: &DbTx<'_>, id: Uuid) -> Result<price_row::Model, ApprovalError> {
        row_repo::find(tx, &self.scope(), self.tenant_id, id)
            .await
            .map_err(storage)?
            .ok_or_else(|| invalid("ROW_NOT_FOUND", format!("price_row {id}")))
    }
    async fn load_all(
        &self,
        tx: &DbTx<'_>,
        ids: impl IntoIterator<Item = Uuid>,
    ) -> Result<Vec<price_row::Model>, ApprovalError> {
        let mut ids: Vec<Uuid> = ids.into_iter().collect();
        ids.sort_unstable();
        ids.dedup();
        let mut models = Vec::with_capacity(ids.len());
        for id in ids {
            models.push(self.load(tx, id).await?);
        }
        Ok(models)
    }
    /// The SKU's metering in force on a date (D-402); a date before any version reads as none.
    async fn metering(&self, sku: Uuid, on: Date) -> Result<SkuMetering, ApprovalError> {
        let unavailable = |_| invalid("REGISTRY_UNAVAILABLE", "Products reference registry");
        let registry = reference_registry::resolve(&self.hub).map_err(unavailable)?;
        let version = registry
            .sku_version_as_of(&self.ctx, self.tenant_id, sku, on)
            .await
            .map_err(unavailable)?;
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
        chain: &[Row],
        unit: &BTreeSet<Uuid>,
    ) -> Result<(), ApprovalError> {
        for successor in chain {
            let Some(predecessor) = row::in_force_before(chain, successor) else {
                continue;
            };
            if !unit.contains(&successor.id) && !unit.contains(&predecessor.id) {
                continue;
            }
            let was = self.metering(sku, predecessor.effective_from).await?;
            let is = self.metering(sku, successor.effective_from).await?;
            row::chain_guard(ChargeKind::Usage, predecessor, &was, successor, &is)
                .map_err(|e| rule(e, successor.id))?;
        }
        Ok(())
    }
    /// Shift, validate against the CURRENT approved rows, normalise and guard one price's rows.
    async fn judge(
        &self,
        tx: &DbTx<'_>,
        price_id: Uuid,
        rows: &[price_row::Model],
        shift: Option<Date>,
    ) -> Result<Judged, ApprovalError> {
        let price = price_repo::find(tx, &self.scope(), self.tenant_id, price_id)
            .await
            .map_err(storage)?
            .ok_or_else(|| invalid("ROW_NOT_FOUND", format!("price {price_id}")))?;
        if price.book_id != self.book_id {
            return Err(invalid("ROW_NOT_IN_BOOK", format!("price {price_id}")));
        }
        if price.reference_state == "lost" {
            return Err(invalid("PRICE_REFERENCE_LOST", format!("price {price_id}")));
        }
        let pc = PriceContext::load(tx, self.tenant_id, &price)
            .await
            .map_err(storage)?;
        let unit: BTreeSet<Uuid> = rows.iter().map(|m| m.id).collect();
        let siblings: Vec<Row> = pc
            .domain_rows()
            .map_err(storage)?
            .into_iter()
            .filter(|r| r.state == RowState::Approved && !unit.contains(&r.id))
            .collect();
        let drafts = rows
            .iter()
            .map(row_repo::to_domain)
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        let proposed = row::shift_selection(&drafts, shift).map_err(|e| rule(e, price_id))?;
        let today = self.now.date();
        let mut starts = BTreeSet::new();
        for r in &proposed {
            if let Some(error) = pc.first_refusal(r, &siblings, today) {
                return Err(rule(error, r.id));
            }
            if !starts.insert((r.dim_value.clone(), r.effective_from)) {
                return Err(invalid("WINDOW_OVERLAP", format!("price_row {}", r.id)));
            }
        }
        // D-391: a return restores what the approved chain applies on the shifted end NOW; the
        // copy made at drafting is stale once another unit (or the common date) changed that.
        // The author re-drafts the pair.
        if let Some(stale) = proposed
            .iter()
            .find(|r| !row::temporary_is_current(&siblings, r, &proposed))
        {
            return Err(invalid(
                "PAIR_RETURN_STALE",
                format!("price_row {}", stale.id),
            ));
        }
        let mut chain = siblings;
        chain.extend(proposed.iter().cloned().map(|mut r| {
            r.state = RowState::Approved;
            r
        }));
        row::normalize_windows(&mut chain);
        if pc.kind == ChargeKind::Usage {
            self.guard(price.sku_id, &chain, &unit).await?;
        }
        Ok(Judged {
            stored: pc.rows,
            proposed,
            chain,
        })
    }
}

#[async_trait::async_trait]
impl<'a> ApprovalSubject<DbTx<'a>> for PriceRowsSubject {
    fn kind(&self) -> &'static str {
        KIND_PRICE_ROWS
    }
    fn ref_type(&self) -> &'static str {
        REF_TYPE
    }
    /// The rows, their pair partners, and each row's chain predecessor on its new start.
    async fn collect(&self, tx: &DbTx<'a>, ids: &[Uuid]) -> Result<Vec<ItemRef>, ApprovalError> {
        let mut wanted: BTreeSet<Uuid> = ids.iter().copied().collect();
        for id in ids {
            if let Some(partner) = self.load(tx, *id).await?.paired_row_id {
                wanted.insert(partner);
            }
        }
        let mut items = Vec::new();
        for (price_id, rows) in by_price(self.load_all(tx, wanted).await?) {
            let unit: BTreeSet<Uuid> = rows.iter().map(|m| m.id).collect();
            let mut chain: Vec<Row> =
                row_repo::for_price(tx, &self.scope(), self.tenant_id, price_id)
                    .await
                    .map_err(storage)?
                    .iter()
                    .filter(|m| m.state == RowState::Approved.as_str() && !unit.contains(&m.id))
                    .map(row_repo::to_domain)
                    .collect::<Result<_, _>>()
                    .map_err(storage)?;
            let drafts = rows
                .iter()
                .map(row_repo::to_domain)
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage)?;
            let proposed = row::shift_selection(&drafts, self.common_effective_date)
                .map_err(|e| rule(e, price_id))?;
            chain.extend(proposed.iter().cloned().map(|mut r| {
                r.state = RowState::Approved;
                r
            }));
            row::normalize_windows(&mut chain);
            for (m, r) in rows.iter().zip(&proposed) {
                let predecessor = chain
                    .iter()
                    .find(|c| c.id == r.id)
                    .and_then(|c| row::in_force_before(&chain, c));
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
            if m.state != RowState::Draft.as_str() || m.pending_unit_id.is_some() {
                return Err(invalid("ROW_NOT_DRAFT", format!("price_row {}", m.id)));
            }
            if m.paired_row_id.is_some_and(|p| !ids.contains(&p)) {
                return Err(invalid("PAIR_SPLIT", format!("price_row {}", m.id)));
            }
        }
        for (price_id, rows) in by_price(models) {
            self.judge(tx, price_id, &rows, self.common_effective_date)
                .await?;
        }
        Ok(())
    }
    /// Conditional ownership in ascending row id; a lost write refuses the whole submit.
    async fn lock(
        &self,
        tx: &DbTx<'a>,
        unit_id: Uuid,
        items: &[ItemRef],
    ) -> Result<(), ApprovalError> {
        for m in self.load_all(tx, items.iter().map(|i| i.item_id)).await? {
            if !row_repo::try_lock(tx, &self.scope(), self.tenant_id, m.id, unit_id, m.version)
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
        json!({
            "book_id": self.book_id,
            "common_effective_date": common_effective_date.map(date),
            "rows": items
                .iter()
                .map(|i| json!({"row_id": i.item_id, "before": i.before, "after": i.after}))
                .collect::<Vec<_>>(),
            "added_partner": self.added_partner,
            "impact": impact(items),
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
                code: "ROW_NOT_PENDING",
                detail: format!("price_row {}", m.id),
            });
        }
        // Prices in ascending id: two batches over the same prices meet in one order.
        for (price_id, rows) in by_price(models) {
            let judged = self
                .judge(tx, price_id, &rows, unit.common_effective_date)
                .await
                .map_err(applied)?;
            let normalised = |id: Uuid| judged.chain.iter().find(|c| c.id == id);
            // The current predecessor of EVERY `new` row of a touched chain binds renewals,
            // whether the `new` row is in this unit or was approved earlier and a row of this
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
                if let Some(predecessor) = row::in_force_before(&judged.chain, c) {
                    keep.insert(predecessor.id);
                }
            }
            for r in &judged.proposed {
                row_repo::approve(
                    tx,
                    &self.scope(),
                    self.tenant_id,
                    r.id,
                    unit.id,
                    row_repo::Approval {
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
                .filter(|m| m.state == RowState::Approved.as_str())
            {
                let Some(chain) = normalised(stored.id) else {
                    continue;
                };
                let keep_for_bound = stored.keep_for_bound || keep.contains(&stored.id);
                if chain.effective_to != stored.effective_to
                    || keep_for_bound != stored.keep_for_bound
                {
                    row_repo::set_window(
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
            (true, _) => row_repo::Unlock::Approved,
            (false, Release::Draft) => row_repo::Unlock::Draft,
            (false, Release::Rejected) => row_repo::Unlock::Rejected,
        };
        for item in items {
            row_repo::unlock(
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
                    ApprovalError::Store("price row lock is not owned by this unit".into())
                }
                other => storage(other),
            })?;
        }
        Ok(())
    }
}
