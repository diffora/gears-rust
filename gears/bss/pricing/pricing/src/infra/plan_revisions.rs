//! The `plan_revision` approval subject (spec §6): one draft revision of a plan, published on
//! approval with its predecessor superseded and the plan's `published_rev` advanced, in the
//! door's serializable transaction.
//!
//! The fingerprinted `after` is the revision's business content only: its book, its sale date
//! and its items (SKU, entry, treatment, included quantity, minimum quantity), in SKU order —
//! never a version, a lock or an item's reference columns, which the reference machine moves
//! while the unit is pending. The item SKUs' current descriptors are read fresh for the reviewer
//! and kept beside `after`, never in it: a GL change must not refresh a pending unit (D-408).
//! Submit and apply judge the revision with the checks of `GET /plan-revisions/{id}/checks`,
//! built by the same function from the same fresh reads.
//!
//! @cpt-dod:cpt-cf-bss-pricing-dod-plan-revision-unit:p1
use crate::{
    api::rest::authoring::{
        dto::PricingPlanCheckDto,
        plans,
        support::{self, DoorError},
    },
    domain::plan::{self, RevisionState},
    infra::storage::{
        RepoError,
        entity::{plan_item, plan_revision},
        repo::{plan_item_repo, plan_repo, plan_revision_repo},
    },
};
use bss_approval::{ApprovalError, ApprovalSubject, ItemRef, Unit};
use bss_products_sdk::models::Sku;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
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

/// The approval kind of one plan revision.
pub const KIND_PLAN_REVISION: &str = crate::domain::plan::KIND_PLAN_REVISION;
/// A `plan_revision` unit references its revision.
pub const REF_TYPE: &str = "plan_revision";
/// The one item of a `plan_revision` unit is the revision.
pub const ITEM_TYPE: &str = "plan_revision";
/// The honest answer for the subscriptions a unit touches: pricing cannot count them before
/// the Subscriptions integration reports its pins.
pub const SUBSCRIPTIONS_UNAVAILABLE: &str = "unavailable until the Subscriptions integration";

/// What a revision's publication touches that pricing can measure: nothing yet, since
/// publishing a revision moves no existing pin (D-394).
#[must_use]
pub fn impact() -> Value {
    json!({ "subscriptions": SUBSCRIPTIONS_UNAVAILABLE })
}

/// A revision's business content, the unit's `after` (and `before` for the published one).
#[must_use]
pub fn content(revision: &plan_revision::Model, items: &[plan_item::Model]) -> Value {
    let mut items: Vec<&plan_item::Model> = items.iter().collect();
    items.sort_by_key(|i| i.sku_id);
    json!({
        "book_id": revision.book_id,
        "available_from": revision.available_from.map(|d| d.to_string()),
        "items": items
            .iter()
            .map(|i| json!({
                "sku_id": i.sku_id,
                "price_book_entry_id": i.price_book_entry_id,
                "treatment": i.treatment,
                "included_qty": i.included_qty,
                "qty_min": i.qty_min,
            }))
            .collect::<Vec<_>>(),
    })
}

/// One SKU's current descriptors, shown to the reviewer for information only (D-408).
#[must_use]
pub fn descriptors(sku: &Sku) -> Value {
    json!({
        "sku_id": sku.id,
        "gl_code": sku.gl_code,
        "tax_category": sku.tax_category,
        "invoice_line_template": sku.invoice_line_template,
        "billing_timing": sku.billing_timing,
    })
}

/// The snapshot's `descriptors` when Products could not answer their read (D-416).
pub const DESCRIPTORS_UNAVAILABLE: &str = "unavailable";

/// The snapshot's `descriptors` from a best-effort SKU read (D-416): each SKU's descriptors, or
/// `"unavailable"` when the registry could not answer or refused the caller. Descriptors are
/// information, never content: their read never refuses a submit, a vote or a reject.
#[must_use]
pub fn descriptors_or_unavailable(read: Result<Vec<Sku>, CanonicalError>) -> Value {
    read.map_or_else(
        |_| Value::from(DESCRIPTORS_UNAVAILABLE),
        |skus| Value::Array(skus.iter().map(descriptors).collect()),
    )
}

/// What changes against the published revision: the book, the sale date, and the items added,
/// removed or changed, by SKU.
fn diff(before: Option<&Value>, after: &Value) -> Value {
    let by_sku = |content: Option<&Value>| -> BTreeMap<String, Value> {
        content
            .and_then(|c| c["items"].as_array())
            .map(|items| {
                items
                    .iter()
                    .map(|i| (i["sku_id"].to_string(), i.clone()))
                    .collect()
            })
            .unwrap_or_default()
    };
    let (old, new) = (by_sku(before), by_sku(Some(after)));
    let field = |name: &str| {
        let was = before.map_or(Value::Null, |b| b[name].clone());
        if was == after[name] {
            Value::Null
        } else {
            json!({"before": was, "after": after[name]})
        }
    };
    json!({
        "book_id": field("book_id"),
        "available_from": field("available_from"),
        "added": new.iter().filter(|(k, _)| !old.contains_key(*k)).map(|(_, v)| v).collect::<Vec<_>>(),
        "removed": old.iter().filter(|(k, _)| !new.contains_key(*k)).map(|(_, v)| v).collect::<Vec<_>>(),
        "changed": new
            .iter()
            .filter_map(|(k, v)| old.get(k).filter(|was| *was != v).map(|was| json!({"before": was, "after": v})))
            .collect::<Vec<_>>(),
    })
}

/// What `collect` and `validate_submit` gather for the synchronous `snapshot`, and what `apply`
/// leaves for the door's `PlanRevisionPublished`.
#[derive(Default)]
struct Review {
    plan_id: Option<Uuid>,
    plan_code: Option<String>,
    rev_no: Option<i32>,
    diff: Value,
    /// The item SKUs' descriptors, or `"unavailable"` when Products did not answer the read.
    descriptors: Value,
    superseded: Option<Uuid>,
}

/// The subject of one `plan_revision` unit.
#[derive(Clone)]
pub struct PlanRevisionSubject {
    /// The caller; the fresh SKU reads are made on its behalf.
    pub ctx: SecurityContext,
    pub hub: Arc<toolkit::ClientHub>,
    pub tenant_id: Uuid,
    pub revision_id: Uuid,
    pub now: OffsetDateTime,
    /// A refusal whose body the approval error cannot carry (a Products refusal kept whole, or
    /// the red checks), for the door to answer as is.
    refused: Arc<Mutex<Option<CanonicalError>>>,
    review: Arc<Mutex<Review>>,
}

fn invalid(code: &'static str, detail: impl Into<String>) -> ApprovalError {
    ApprovalError::InvalidSubmit {
        code,
        field: ITEM_TYPE.into(),
        detail: detail.into(),
    }
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
fn red(checks: Vec<plan::Check>) -> Vec<PricingPlanCheckDto> {
    checks
        .into_iter()
        .filter(|c| !c.ok)
        .map(Into::into)
        .collect()
}
fn codes(red: &[PricingPlanCheckDto]) -> String {
    red.iter()
        .map(|c| c.code.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

impl PlanRevisionSubject {
    /// A subject for the caller's tenant.
    #[must_use]
    pub fn new(
        ctx: SecurityContext,
        hub: Arc<toolkit::ClientHub>,
        revision_id: Uuid,
        now: OffsetDateTime,
    ) -> Self {
        let tenant_id = ctx.subject_tenant_id();
        Self {
            ctx,
            hub,
            tenant_id,
            revision_id,
            now,
            refused: Arc::default(),
            review: Arc::default(),
        }
    }
    /// The refusal that ended the last judgement with a body of its own, if any.
    #[must_use]
    pub fn take_refusal(&self) -> Option<CanonicalError> {
        self.refused.lock().ok().and_then(|mut slot| slot.take())
    }
    /// The revision the last apply superseded, if any.
    #[must_use]
    pub fn superseded(&self) -> Option<Uuid> {
        self.review.lock().ok().and_then(|r| r.superseded)
    }
    fn refuse(&self, error: CanonicalError) {
        if let Ok(mut slot) = self.refused.lock() {
            *slot = Some(error);
        }
    }
    fn scope(&self) -> AccessScope {
        AccessScope::for_tenant(self.tenant_id)
    }
    async fn revision(&self, tx: &impl DBRunner) -> Result<plan_revision::Model, ApprovalError> {
        plan_revision_repo::find(tx, &self.scope(), self.tenant_id, self.revision_id)
            .await
            .map_err(storage)?
            .ok_or_else(|| {
                invalid(
                    "REVISION_NOT_FOUND",
                    format!("revision {}", self.revision_id),
                )
            })
    }
    /// The item SKUs, read fresh for the checks (D-408), a rule: the read is hard and made as the
    /// caller, and the descriptors are kept for the snapshot. Only unavailability is
    /// `REGISTRY_UNAVAILABLE`; a definite refusal is answered as Products gave it.
    async fn skus(&self, ids: impl IntoIterator<Item = Uuid>) -> Result<Vec<Sku>, ApprovalError> {
        match plans::fresh_skus(&self.hub, &self.ctx, ids).await {
            Ok(skus) => {
                if let Ok(mut review) = self.review.lock() {
                    review.descriptors = Value::Array(skus.iter().map(descriptors).collect());
                }
                Ok(skus)
            }
            Err(error) if error.status_code() == 503 => Err(invalid(
                "REGISTRY_UNAVAILABLE",
                "Products reference registry",
            )),
            Err(error) => {
                self.refuse(error);
                Err(invalid("REGISTRY_REFUSED", "Products refused a SKU read"))
            }
        }
    }
    /// The checks of `GET /plan-revisions/{id}/checks` on the stored state of this transaction
    /// and fresh SKU reads, on the subject's day.
    async fn judge(&self, tx: &DbTx<'_>) -> Result<Vec<plan::Check>, ApprovalError> {
        let mut context =
            plans::stored_context(tx, &self.scope(), self.tenant_id, self.revision_id)
                .await
                .map_err(|error| match error {
                    DoorError::Repo(e) => storage(e),
                    DoorError::Api(e) if e.status_code() == 404 => invalid(
                        "REVISION_NOT_FOUND",
                        format!("revision {}", self.revision_id),
                    ),
                    other => ApprovalError::Store(other.to_string()),
                })?;
        context.skus = self.skus(context.items.iter().map(|i| i.sku_id)).await?;
        Ok(plan::checks(&context, self.now.date()))
    }
}

#[async_trait::async_trait]
impl<'a> ApprovalSubject<DbTx<'a>> for PlanRevisionSubject {
    fn kind(&self) -> &'static str {
        KIND_PLAN_REVISION
    }
    fn ref_type(&self) -> &'static str {
        REF_TYPE
    }
    /// The revision's business content, the published revision's as `before`, the diff and the
    /// item SKUs' current descriptors (best-effort, D-416); its author is the item author
    /// separation of duties excludes.
    async fn collect(&self, tx: &DbTx<'a>, _ids: &[Uuid]) -> Result<Vec<ItemRef>, ApprovalError> {
        let r = self.revision(tx).await?;
        let scope = self.scope();
        let p = plan_repo::find(tx, &scope, self.tenant_id, r.plan_id)
            .await
            .map_err(storage)?
            .ok_or_else(|| ApprovalError::Store(format!("revision {} has no plan", r.id)))?;
        let items = plan_item_repo::for_revision(tx, &scope, self.tenant_id, r.id)
            .await
            .map_err(storage)?;
        let published = plan_revision_repo::for_plan(tx, &scope, self.tenant_id, p.id)
            .await
            .map_err(storage)?
            .into_iter()
            .find(|x| x.id != r.id && x.state == RevisionState::Published.as_str());
        let before = match published {
            Some(published) => {
                let old = plan_item_repo::for_revision(tx, &scope, self.tenant_id, published.id)
                    .await
                    .map_err(storage)?;
                Some(content(&published, &old))
            }
            None => None,
        };
        let after = content(&r, &items);
        // The descriptors are information (D-416): best-effort, never a refusal. The checks'
        // hard reads are `validate_submit`'s and `apply`'s.
        let described = descriptors_or_unavailable(
            plans::fresh_skus(&self.hub, &self.ctx, items.iter().map(|i| i.sku_id)).await,
        );
        if let Ok(mut review) = self.review.lock() {
            review.descriptors = described;
            review.plan_id = Some(p.id);
            review.plan_code = Some(p.code);
            review.rev_no = Some(r.rev_no);
            review.diff = diff(before.as_ref(), &after);
        }
        Ok(vec![ItemRef {
            item_type: ITEM_TYPE.into(),
            item_id: r.id,
            created_by: r.created_by,
            before,
            after,
        }])
    }
    /// An unlocked draft whose checks are all green; a red check refuses with the red checks and
    /// no unit.
    async fn validate_submit(
        &self,
        tx: &DbTx<'a>,
        _items: &[ItemRef],
    ) -> Result<(), ApprovalError> {
        let r = self.revision(tx).await?;
        if r.state != RevisionState::Draft.as_str() || r.pending_unit_id.is_some() {
            return Err(invalid("REVISION_NOT_DRAFT", format!("revision {}", r.id)));
        }
        // @cpt-begin:cpt-cf-bss-pricing-algo-plans-revision-checks:p1:inst-plans-revision-checks-4
        let red = red(self.judge(tx).await?);
        if red.is_empty() {
            return Ok(());
        }
        self.refuse(support::checks_red(&red));
        Err(invalid("REVISION_CHECKS_RED", codes(&red)))
        // @cpt-end:cpt-cf-bss-pricing-algo-plans-revision-checks:p1:inst-plans-revision-checks-4
    }
    /// `pending_unit_id` on an unlocked draft at the version read; a lost write is
    /// `ROW_LOCKED_PENDING` and the whole submit rolls back.
    async fn lock(
        &self,
        tx: &DbTx<'a>,
        unit_id: Uuid,
        _items: &[ItemRef],
    ) -> Result<(), ApprovalError> {
        let r = self.revision(tx).await?;
        if plan_revision_repo::try_lock(tx, &self.scope(), self.tenant_id, r.id, unit_id, r.version)
            .await
            .map_err(storage)?
        {
            Ok(())
        } else {
            Err(ApprovalError::Locked {
                item_type: ITEM_TYPE.into(),
                item_id: r.id,
            })
        }
    }
    fn snapshot(&self, items: &[ItemRef], _common_effective_date: Option<Date>) -> Value {
        let item = items.first();
        let (plan_id, plan_code, rev_no, diff, descriptors) = self.review.lock().map_or_else(
            |_| (None, None, None, Value::Null, Value::Null),
            |r| {
                (
                    r.plan_id,
                    r.plan_code.clone(),
                    r.rev_no,
                    r.diff.clone(),
                    r.descriptors.clone(),
                )
            },
        );
        json!({
            "plan_id": plan_id,
            "plan_code": plan_code,
            "revision_id": self.revision_id,
            "rev_no": rev_no,
            "before": item.and_then(|i| i.before.clone()),
            "after": item.map(|i| i.after.clone()),
            "diff": diff,
            "descriptors": descriptors,
            "impact": impact(),
            "computed_at": self
                .now
                .format(&time::format_description::well_known::Rfc3339)
                .ok(),
        })
    }
    /// The checks again with fresh reads (red: `APPLY_REFUSED`, the whole unit rolls back); then
    /// the published revision is superseded FIRST, this one published, and the plan's
    /// `published_rev` advanced.
    async fn apply(
        &self,
        tx: &DbTx<'a>,
        unit: &Unit,
        _items: &[ItemRef],
    ) -> Result<(), ApprovalError> {
        let r = self.revision(tx).await.map_err(applied)?;
        if r.state != RevisionState::Pending.as_str() || r.pending_unit_id != Some(unit.id) {
            return Err(ApprovalError::ApplyRefused {
                code: "REVISION_NOT_PENDING",
                detail: format!("revision {}", r.id),
            });
        }
        // @cpt-begin:cpt-cf-bss-pricing-algo-plans-revision-apply:p1:inst-plans-revision-apply-2
        let red = red(self.judge(tx).await.map_err(applied)?);
        if !red.is_empty() {
            return Err(ApprovalError::ApplyRefused {
                code: "REVISION_CHECKS_RED",
                detail: codes(&red),
            });
        }
        // @cpt-end:cpt-cf-bss-pricing-algo-plans-revision-apply:p1:inst-plans-revision-apply-2
        let scope = self.scope();
        let previous = plan_revision_repo::for_plan(tx, &scope, self.tenant_id, r.plan_id)
            .await
            .map_err(storage)?
            .into_iter()
            .find(|x| x.id != r.id && x.state == RevisionState::Published.as_str());
        let contended = |e: RepoError| match e {
            RepoError::Conflict { .. } => ApprovalError::Contended,
            other => storage(other),
        };
        // @cpt-begin:cpt-cf-bss-pricing-algo-plans-revision-apply:p1:inst-plans-revision-apply-3
        // @cpt-begin:cpt-cf-bss-pricing-flow-plans:p1:inst-plans-flow-5
        if let Some(previous) = &previous {
            plan_revision_repo::supersede(
                tx,
                &scope,
                self.tenant_id,
                previous.id,
                previous.version,
                self.now,
            )
            .await
            .map_err(contended)?;
        }
        plan_revision_repo::publish(tx, &scope, self.tenant_id, r.id, unit.id, self.now)
            .await
            .map_err(storage)?;
        let p = plan_repo::find(tx, &scope, self.tenant_id, r.plan_id)
            .await
            .map_err(storage)?
            .ok_or_else(|| ApprovalError::Store(format!("revision {} has no plan", r.id)))?;
        plan_repo::set_published(
            tx,
            &scope,
            self.tenant_id,
            p.id,
            p.version,
            r.rev_no,
            self.now,
        )
        .await
        .map_err(contended)?;
        // @cpt-end:cpt-cf-bss-pricing-flow-plans:p1:inst-plans-flow-5
        // @cpt-end:cpt-cf-bss-pricing-algo-plans-revision-apply:p1:inst-plans-revision-apply-3
        if let Ok(mut review) = self.review.lock() {
            review.superseded = previous.map(|x| x.id);
        }
        Ok(())
    }
    /// Reject and withdraw return the revision to an editable draft; an approval's publish
    /// already turned the lock into `approved_by_unit_id`.
    async fn unlock(
        &self,
        tx: &DbTx<'a>,
        unit: &Unit,
        _items: &[ItemRef],
        approved: bool,
    ) -> Result<(), ApprovalError> {
        if approved {
            return Ok(());
        }
        plan_revision_repo::unlock(tx, &self.scope(), self.tenant_id, self.revision_id, unit.id)
            .await
            .map_err(|e| match e {
                RepoError::Conflict { .. } => {
                    ApprovalError::Store("revision lock is not owned by this unit".into())
                }
                other => storage(other),
            })
    }
}
