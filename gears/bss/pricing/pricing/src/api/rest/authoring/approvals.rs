//! Submission, publish changes, the approval queue, generation-bound votes and the policy.
//!
//! @cpt-dod:cpt-cf-bss-pricing-dod-publish-changes-selection:p1
//! @cpt-dod:cpt-cf-bss-pricing-dod-stale-refresh-generation:p1
//! @cpt-dod:cpt-cf-bss-pricing-dod-generation-and-duplicate-vote:p1
//! @cpt-dod:cpt-cf-bss-pricing-dod-unit-contended:p1
//! @cpt-dod:cpt-cf-bss-pricing-dod-terminal-audit-event:p1
use super::{
    dto::{
        PriceBookDto, PricingApprovalPolicyDto, PricingApprovalPolicyPut, PricingApprovalUnitDto,
        PricingApprovalUnitList, PricingPriceDto, PricingPriceRowDto, PricingProposedRow,
        PricingPublishChanges, PricingPublishChangesRequest, PricingSubmitReceipt,
        PricingVoteReceipt, PricingVoteRequest,
    },
    support::{self, DoorError, approval_failure},
};
use crate::{
    domain::row::{self, RowState},
    infra::{
        events::{self, ApprovalUnitDecided, PriceRowsPublished, PublishedRow},
        price_rows::{KIND_PRICE_ROWS, PriceRowsSubject, Release},
        storage::{
            RepoError,
            entity::price_row,
            repo::{
                approval_repo::{self, PricingApprovalStore},
                book_repo, price_repo, row_repo,
            },
        },
    },
};
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use bss_approval::{
    ApprovalSubject, ApproveOutcome, Engine, Store, SubmitRequest, Unit, UnitState,
};
use std::{collections::BTreeSet, sync::Arc};
use time::OffsetDateTime;
use toolkit_canonical_errors::CanonicalError;
use toolkit_db::{
    Db, DbTx,
    secure::{AccessScope, DBRunner},
};
use toolkit_security::SecurityContext;
use uuid::Uuid;

/// Everything one keyed approval command carries into its transaction.
#[derive(Clone)]
pub struct Command {
    pub scope: AccessScope,
    pub ctx: SecurityContext,
    pub hub: Arc<toolkit::ClientHub>,
    /// The toolkit outbox the decision's events are enqueued on, inside its transaction.
    pub outbox: Arc<toolkit_db::outbox::Outbox>,
    pub correlation: Uuid,
    pub key: String,
    pub digest: Vec<u8>,
}
impl Command {
    fn tenant(&self) -> Uuid {
        self.ctx.subject_tenant_id()
    }
    fn store(&self) -> PricingApprovalStore {
        PricingApprovalStore {
            scope: AccessScope::for_tenant(self.tenant()),
            tenant_id: self.tenant(),
        }
    }
}
/// The three decisions a unit can take from a person.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vote {
    Approve,
    Reject,
    Withdraw,
}
impl Vote {
    const fn path(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::Reject => "reject",
            Self::Withdraw => "withdraw",
        }
    }
}

async fn unit_dto(
    tx: &DbTx<'_>,
    store: &PricingApprovalStore,
    unit: Unit,
) -> Result<PricingApprovalUnitDto, DoorError> {
    let decisions = store
        .decisions(tx, unit.id)
        .await
        .map_err(approval_failure)?;
    let mut dto = PricingApprovalUnitDto::from(unit);
    dto.decisions = decisions.into_iter().map(Into::into).collect();
    Ok(dto)
}
async fn load_unit(
    tx: &DbTx<'_>,
    store: &PricingApprovalStore,
    id: Uuid,
) -> Result<Unit, DoorError> {
    store
        .unit(tx, id)
        .await
        .map_err(approval_failure)?
        .ok_or_else(|| support::missing_what("approval_unit").into())
}
async fn rows_of(
    tx: &DbTx<'_>,
    store: &PricingApprovalStore,
    unit: Uuid,
) -> Result<Vec<PricingPriceRowDto>, DoorError> {
    let scope = AccessScope::for_tenant(store.tenant_id);
    let mut rows = Vec::new();
    for item in store.items(tx, unit).await.map_err(approval_failure)? {
        if let Some(m) = row_repo::find(tx, &scope, store.tenant_id, item.item_id).await? {
            rows.push(m.into());
        }
    }
    Ok(rows)
}

/// `ApprovalUnitDecided` for a unit that just reached its terminal state, in the deciding
/// transaction: the current-generation voters and the acting principal.
async fn decided(
    tx: &DbTx<'_>,
    cmd: &Command,
    store: &PricingApprovalStore,
    id: Uuid,
    now: OffsetDateTime,
) -> Result<(), DoorError> {
    let unit = load_unit(tx, store, id).await?;
    let mut actors: Vec<Uuid> = store
        .decisions(tx, unit.id)
        .await
        .map_err(approval_failure)?
        .into_iter()
        .filter(|d| !d.stale)
        .map(|d| d.actor)
        .collect();
    actors.push(cmd.ctx.subject_id());
    actors.sort_unstable();
    actors.dedup();
    let event = ApprovalUnitDecided {
        tenant_id: unit.tenant_id,
        unit_id: unit.id,
        kind: unit.kind.clone(),
        state: unit.state.as_str().into(),
        generation: unit.generation,
        actors,
    };
    events::enqueue(&cmd.outbox, tx, &event, now).await?;
    Ok(())
}
/// `PriceRowsPublished` for an applied unit, in the apply transaction: every row with the
/// window its chain was approved with.
async fn published(
    tx: &DbTx<'_>,
    cmd: &Command,
    store: &PricingApprovalStore,
    id: Uuid,
    now: OffsetDateTime,
) -> Result<(), DoorError> {
    let unit = load_unit(tx, store, id).await?;
    let scope = AccessScope::for_tenant(store.tenant_id);
    let mut rows = Vec::new();
    for item in store.items(tx, unit.id).await.map_err(approval_failure)? {
        let m = row_repo::find(tx, &scope, store.tenant_id, item.item_id)
            .await?
            .ok_or_else(|| {
                RepoError::CorruptRow(format!("unit {} lost row {}", unit.id, item.item_id))
            })?;
        rows.push(PublishedRow {
            row_id: m.id,
            price_id: m.price_id,
            dim_value: m.dim_value,
            effective_from: m.effective_from.to_string(),
            effective_to: m.effective_to.map(|d| d.to_string()),
            eligibility: m.eligibility,
        });
    }
    rows.sort_by_key(|r| r.row_id);
    let event = PriceRowsPublished {
        tenant_id: unit.tenant_id,
        book_id: unit.ref_id,
        unit_id: unit.id,
        rows,
        actor_ref: cmd.ctx.subject_id(),
    };
    events::enqueue(&cmd.outbox, tx, &event, now).await?;
    Ok(())
}

/// Record one unit over the selected rows, applying it at once under quorum zero.
async fn record(
    tx: &DbTx<'_>,
    cmd: &Command,
    endpoint: &str,
    subject: &PriceRowsSubject,
    ids: &[Uuid],
) -> Result<Response, DoorError> {
    let store = cmd.store();
    let policy = approval_repo::read_policy(tx, &store.scope, cmd.tenant()).await?;
    let submitted = Engine::submit(
        &store,
        subject,
        tx,
        SubmitRequest {
            tenant_id: cmd.tenant(),
            ref_id: subject.book_id,
            item_ids: ids,
            actor: cmd.ctx.subject_id(),
            policy: &policy,
            common_effective_date: subject.common_effective_date,
            now: subject.now,
        },
    )
    .await
    .map_err(approval_failure)?;
    let unit = submitted.unit;
    support::audit(
        tx,
        &cmd.ctx,
        cmd.correlation,
        "approval.submitted",
        unit.id,
        unit.version,
    )
    .await?;
    if submitted.applied {
        support::audit(
            tx,
            &cmd.ctx,
            cmd.correlation,
            "approval.approved",
            unit.id,
            unit.version,
        )
        .await?;
        published(tx, cmd, &store, unit.id, subject.now).await?;
        decided(tx, cmd, &store, unit.id, subject.now).await?;
    }
    let rows = rows_of(tx, &store, unit.id).await?;
    let receipt = PricingSubmitReceipt {
        applied: submitted.applied,
        unit: unit_dto(tx, &store, unit).await?,
        rows,
    };
    support::answer(
        tx,
        cmd.tenant(),
        endpoint,
        &cmd.key,
        StatusCode::CREATED,
        &receipt,
        None,
    )
    .await
}

/// `POST /rows/{id}/submit`: one row alone; a pair half is refused, publish the pair instead.
/// # Errors
/// Returns the canonical refusal.
pub async fn submit_row(db: &Db, cmd: Command, id: Uuid) -> Result<Response, CanonicalError> {
    support::transaction(db, move |tx| {
        let cmd = cmd.clone();
        Box::pin(async move {
            let endpoint = format!("/bss-pricing/v1/rows/{id}/submit");
            if let Some(replay) =
                support::claim(tx, cmd.tenant(), &endpoint, &cmd.key, &cmd.digest).await?
            {
                return Ok(replay);
            }
            let row = row_repo::find(tx, &cmd.scope, cmd.tenant(), id)
                .await?
                .ok_or_else(|| support::missing_what("price_row"))?;
            if row.paired_row_id.is_some() {
                return Err(support::invalid("row_ids", "PAIR_SPLIT").into());
            }
            let price = price_repo::find(
                tx,
                &AccessScope::for_tenant(cmd.tenant()),
                cmd.tenant(),
                row.price_id,
            )
            .await?
            .ok_or_else(|| support::missing_what("price"))?;
            let subject = PriceRowsSubject::new(
                cmd.ctx.clone(),
                cmd.hub.clone(),
                price.book_id,
                OffsetDateTime::now_utc(),
            );
            record(tx, &cmd, &endpoint, &subject, &[id]).await
        })
    })
    .await
}

/// Every draft row of a book with its price, chain and predecessor, in proposal order.
async fn proposals(
    tx: &impl DBRunner,
    tenant: Uuid,
    book: Uuid,
) -> Result<Vec<PricingProposedRow>, DoorError> {
    let children = AccessScope::for_tenant(tenant);
    let prices = price_repo::for_book(tx, &children, tenant, book).await?;
    let mut stored: Vec<price_row::Model> = Vec::new();
    for p in &prices {
        stored.extend(row_repo::for_price(tx, &children, tenant, p.id).await?);
    }
    let rows = stored
        .iter()
        .map(row_repo::to_domain)
        .collect::<Result<Vec<_>, RepoError>>()?;
    let owners: Vec<(Uuid, Uuid)> = prices.iter().map(|p| (p.id, p.book_id)).collect();
    let mut out = Vec::new();
    for r in row::proposed_rows(book, &owners, &rows) {
        let Some(m) = stored.iter().find(|m| m.id == r.id) else {
            continue;
        };
        if m.pending_unit_id.is_some() {
            continue;
        }
        let price = prices
            .iter()
            .find(|p| p.id == r.price_id)
            .cloned()
            .ok_or_else(|| RepoError::CorruptRow(format!("price_row {} has no price", r.id)))?;
        let before = row::in_force_before(&rows, r)
            .and_then(|b| stored.iter().find(|m| m.id == b.id))
            .cloned()
            .map(PricingPriceRowDto::from);
        out.push(PricingProposedRow {
            row: m.clone().into(),
            price: PricingPriceDto::from(price),
            chain: r.dim_value.clone().unwrap_or_else(|| "default".into()),
            before,
            pair_partner_id: r.paired_row_id,
            selected: true,
        });
    }
    Ok(out)
}

/// `GET /price-books/{id}/publish-changes`.
/// # Errors
/// Returns a missing book or storage failure.
pub async fn publish_list(
    tx: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    book: Uuid,
) -> Result<Response, DoorError> {
    let model = book_repo::find(tx, scope, tenant, book)
        .await?
        .ok_or_else(support::missing)?;
    let rows = proposals(tx, tenant, book).await?;
    let body = PricingPublishChanges {
        book: PriceBookDto::from(model),
        rows,
    };
    Ok(support::response(StatusCode::OK, &body, None)?)
}

/// `POST /price-books/{id}/publish-changes`: the ticked drafts (all when omitted), their pair
/// partners pulled in and recorded as `added_partner`, and an optional common start.
/// # Errors
/// Returns the canonical refusal.
pub async fn publish(
    db: &Db,
    cmd: Command,
    book: Uuid,
    input: PricingPublishChangesRequest,
) -> Result<Response, CanonicalError> {
    let date = support::date(input.common_effective_date.clone(), "common_effective_date")?;
    support::transaction(db, move |tx| {
        let (cmd, input) = (cmd.clone(), input.clone());
        Box::pin(async move {
            let endpoint = format!("/bss-pricing/v1/price-books/{book}/publish-changes");
            if let Some(replay) =
                support::claim(tx, cmd.tenant(), &endpoint, &cmd.key, &cmd.digest).await?
            {
                return Ok(replay);
            }
            book_repo::find(tx, &cmd.scope, cmd.tenant(), book)
                .await?
                .ok_or_else(support::missing)?;
            let children = AccessScope::for_tenant(cmd.tenant());
            let mut owned: Vec<price_row::Model> = Vec::new();
            for p in price_repo::for_book(tx, &children, cmd.tenant(), book).await? {
                owned.extend(row_repo::for_price(tx, &children, cmd.tenant(), p.id).await?);
            }
            let draft = |m: &price_row::Model| {
                m.state == RowState::Draft.as_str() && m.pending_unit_id.is_none()
            };
            let selected: Vec<Uuid> = match input.row_ids {
                None => owned.iter().filter(|m| draft(m)).map(|m| m.id).collect(),
                Some(ids) => {
                    for id in &ids {
                        let Some(m) = owned.iter().find(|m| m.id == *id) else {
                            return Err(support::invalid("row_ids", "ROW_NOT_IN_BOOK").into());
                        };
                        if !draft(m) {
                            return Err(support::conflict("ROW_NOT_DRAFT").into());
                        }
                    }
                    ids
                }
            };
            if selected.is_empty() {
                return Err(support::invalid("row_ids", "NO_DRAFT_ROWS").into());
            }
            let chosen: BTreeSet<Uuid> = selected.iter().copied().collect();
            let mut added: Vec<Uuid> = owned
                .iter()
                .filter(|m| chosen.contains(&m.id))
                .filter_map(|m| m.paired_row_id)
                .filter(|partner| !chosen.contains(partner))
                .collect();
            added.sort_unstable();
            added.dedup();
            let mut subject = PriceRowsSubject::new(
                cmd.ctx.clone(),
                cmd.hub.clone(),
                book,
                OffsetDateTime::now_utc(),
            );
            subject.common_effective_date = date;
            subject.added_partner = added;
            record(tx, &cmd, &endpoint, &subject, &selected).await
        })
    })
    .await
}

/// A known unit state filter.
/// # Errors
/// Unknown states are refused with `UNIT_STATE_INVALID`.
pub fn state_filter(state: Option<&str>) -> Result<Option<UnitState>, CanonicalError> {
    state
        .map(|s| UnitState::parse(s).ok_or_else(|| support::invalid("state", "UNIT_STATE_INVALID")))
        .transpose()
}
/// `GET /approval-units` in submission order, with every generation's decisions.
/// # Errors
/// Returns storage failures.
pub async fn list_units(
    tx: &DbTx<'_>,
    scope: &AccessScope,
    tenant: Uuid,
    state: Option<UnitState>,
    kind: Option<&str>,
    reference: Option<Uuid>,
) -> Result<Response, DoorError> {
    let store = PricingApprovalStore {
        scope: scope.clone(),
        tenant_id: tenant,
    };
    let mut items = Vec::new();
    for unit in approval_repo::list_units(tx, scope, tenant, state, kind, reference).await? {
        items.push(unit_dto(tx, &store, unit).await?);
    }
    Ok(support::response(
        StatusCode::OK,
        &PricingApprovalUnitList { items },
        None,
    )?)
}
/// `GET /approval-units/{id}`: the stored snapshot, the decisions and the live impact.
/// # Errors
/// Returns a missing unit or storage failure.
pub async fn get_unit(
    tx: &DbTx<'_>,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
) -> Result<Response, DoorError> {
    let store = PricingApprovalStore {
        scope: scope.clone(),
        tenant_id: tenant,
    };
    let unit = load_unit(tx, &store, id).await?;
    let items = store.items(tx, id).await.map_err(approval_failure)?;
    let mut dto = unit_dto(tx, &store, unit).await?;
    dto.impact = Some(crate::infra::price_rows::impact(&items));
    Ok(support::response(StatusCode::OK, &dto, None)?)
}

/// The subject a pending unit was submitted with: its book, shift and pulled-in partners.
fn subject_of(cmd: &Command, unit: &Unit, action: Vote) -> PriceRowsSubject {
    let mut subject = PriceRowsSubject::new(
        cmd.ctx.clone(),
        cmd.hub.clone(),
        unit.ref_id,
        OffsetDateTime::now_utc(),
    );
    subject.common_effective_date = unit.common_effective_date;
    subject.added_partner =
        serde_json::from_value(unit.snapshot["added_partner"].clone()).unwrap_or_default();
    subject.release = if action == Vote::Reject {
        Release::Rejected
    } else {
        Release::Draft
    };
    subject
}
/// Rejects obey the same content-generation barrier without applying.
async fn refresh_reject(
    tx: &DbTx<'_>,
    store: &PricingApprovalStore,
    subject: &PriceRowsSubject,
    unit: &Unit,
    seen: i32,
) -> Result<Option<i32>, DoorError> {
    if unit.generation != seen {
        return Err(DoorError::Generation {
            current: unit.generation,
        });
    }
    let ids: Vec<Uuid> = store
        .items(tx, unit.id)
        .await
        .map_err(approval_failure)?
        .iter()
        .map(|i| i.item_id)
        .collect();
    let items = subject.collect(tx, &ids).await.map_err(approval_failure)?;
    let hash = bss_approval::hash::snapshot_hash(&items, unit.common_effective_date);
    if hash == unit.snapshot_hash {
        return Ok(None);
    }
    if !store
        .bump_version(tx, unit.id, unit.version)
        .await
        .map_err(approval_failure)?
    {
        return Err(approval_failure(bss_approval::ApprovalError::Contended));
    }
    let generation = unit.generation.saturating_add(1);
    store
        .refresh(
            tx,
            unit.id,
            &items,
            &subject.snapshot(&items, unit.common_effective_date),
            &hash,
            generation,
        )
        .await
        .map_err(approval_failure)?;
    Ok(Some(generation))
}
/// `POST /approval-units/{id}/approve|reject|withdraw`.
/// Content drift commits the refreshed unit and answers `UNIT_STALE` with the new generation.
/// # Errors
/// Returns the canonical refusal; a generation mismatch carries the current generation.
pub async fn vote(
    db: &Db,
    cmd: Command,
    id: Uuid,
    action: Vote,
    body: Option<PricingVoteRequest>,
) -> Result<Response, CanonicalError> {
    let result = support::transaction_door(db, move |tx| {
        let (cmd, body) = (cmd.clone(), body.clone());
        Box::pin(async move { vote_in(tx, &cmd, id, action, body).await })
    })
    .await;
    match result {
        Err(DoorError::Generation { current }) => {
            let problem = support::generation_problem("GENERATION_MISMATCH", current);
            Ok((StatusCode::BAD_REQUEST, axum::Json(problem)).into_response())
        }
        other => other.map_err(Into::into),
    }
}
async fn vote_in(
    tx: &DbTx<'_>,
    cmd: &Command,
    id: Uuid,
    action: Vote,
    body: Option<PricingVoteRequest>,
) -> Result<Response, DoorError> {
    let endpoint = format!("/bss-pricing/v1/approval-units/{id}/{}", action.path());
    let store = PricingApprovalStore {
        scope: cmd.scope.clone(),
        tenant_id: cmd.tenant(),
    };
    let unit = load_unit(tx, &store, id).await?;
    if let Some(replay) = support::claim(tx, cmd.tenant(), &endpoint, &cmd.key, &cmd.digest).await?
    {
        return Ok(replay);
    }
    if unit.state != UnitState::Pending {
        return Err(support::conflict("UNIT_ALREADY_DECIDED").into());
    }
    let subject = subject_of(cmd, &unit, action);
    let now = subject.now;
    let actor = cmd.ctx.subject_id();
    let seen = || {
        body.as_ref()
            .map(|b| b.generation)
            .ok_or_else(|| DoorError::from(support::invalid("generation", "GENERATION_REQUIRED")))
    };
    let note = body.as_ref().and_then(|b| b.note.clone());
    let outcome = match action {
        Vote::Approve => Engine::approve(
            &store,
            &subject,
            tx,
            id,
            actor,
            seen()?,
            note.as_deref(),
            now,
        )
        .await
        .map_err(approval_failure)?,
        Vote::Reject => {
            let note = note
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| support::invalid("note", "NOTE_REQUIRED"))?;
            if let Some(generation) = refresh_reject(tx, &store, &subject, &unit, seen()?).await? {
                ApproveOutcome::Refreshed { generation }
            } else {
                Engine::reject(&store, &subject, tx, id, actor, seen()?, note, now)
                    .await
                    .map_err(approval_failure)?;
                ApproveOutcome::Applied
            }
        }
        Vote::Withdraw => {
            Engine::withdraw(&store, &subject, tx, id, actor, now)
                .await
                .map_err(approval_failure)?;
            ApproveOutcome::Applied
        }
    };
    let (label, audit, have, need) = match outcome {
        ApproveOutcome::Refreshed { generation } => {
            support::audit(
                tx,
                &cmd.ctx,
                cmd.correlation,
                "approval.refreshed",
                id,
                unit.version,
            )
            .await?;
            let problem = support::generation_problem("UNIT_STALE", generation);
            return support::answer(
                tx,
                cmd.tenant(),
                &endpoint,
                &cmd.key,
                StatusCode::BAD_REQUEST,
                &problem,
                None,
            )
            .await;
        }
        ApproveOutcome::Pending { have, need } => {
            ("pending", "approval.vote", Some(have), Some(need))
        }
        ApproveOutcome::Applied => {
            if action == Vote::Approve {
                published(tx, cmd, &store, id, now).await?;
            }
            decided(tx, cmd, &store, id, now).await?;
            match action {
                Vote::Approve => ("applied", "approval.approved", None, None),
                Vote::Reject => ("rejected", "approval.rejected", None, None),
                Vote::Withdraw => ("withdrawn", "approval.withdrawn", None, None),
            }
        }
    };
    let unit = load_unit(tx, &store, id).await?;
    support::audit(tx, &cmd.ctx, cmd.correlation, audit, id, unit.version).await?;
    let receipt = PricingVoteReceipt {
        have,
        need,
        outcome: label.into(),
        unit: unit_dto(tx, &store, unit).await?,
    };
    support::answer(
        tx,
        cmd.tenant(),
        &endpoint,
        &cmd.key,
        StatusCode::OK,
        &receipt,
        None,
    )
    .await
}

/// A strong decimal validator over the whole policy, like the dimension registry's.
fn policy_tag(policy: &bss_approval::Policy) -> Result<u64, CanonicalError> {
    let hash = crate::api::rest::preconditions::request_digest(&(
        policy.default_quorum,
        &policy.overrides,
    ))
    .map_err(CanonicalError::from)?;
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&hash[..8]);
    Ok(u64::from_be_bytes(bytes))
}
/// `GET /approval-policy`: the tenant default (fail-safe one) and kind overrides.
/// # Errors
/// Returns storage failures.
pub async fn get_policy(
    tx: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
) -> Result<Response, DoorError> {
    let policy = approval_repo::read_policy(tx, scope, tenant).await?;
    let tag = policy_tag(&policy)?;
    Ok(support::response(
        StatusCode::OK,
        &PricingApprovalPolicyDto::from(policy),
        Some(tag),
    )?)
}
/// `PUT /approval-policy`: set the default (`*`) or the `price_rows` quorum under If-Match.
/// # Errors
/// Returns `POLICY_KIND_INVALID`, `QUORUM_INVALID` or `VERSION_CONFLICT`.
pub async fn put_policy(
    tx: &impl DBRunner,
    scope: &AccessScope,
    ctx: &SecurityContext,
    correlation: Uuid,
    version: u64,
    input: PricingApprovalPolicyPut,
) -> Result<Response, DoorError> {
    let tenant = ctx.subject_tenant_id();
    let kind = input.kind.unwrap_or_else(|| "*".into());
    if !matches!(kind.as_str(), "*" | KIND_PRICE_ROWS) {
        return Err(support::invalid("kind", "POLICY_KIND_INVALID").into());
    }
    if i32::try_from(input.quorum).is_err() {
        return Err(support::invalid("quorum", "QUORUM_INVALID").into());
    }
    if policy_tag(&approval_repo::read_policy(tx, scope, tenant).await?)? != version {
        return Err(support::conflict("VERSION_CONFLICT").into());
    }
    approval_repo::write_policy(tx, scope, tenant, &kind, input.quorum).await?;
    support::audit(tx, ctx, correlation, "approval_policy.write", tenant, 0).await?;
    let policy = approval_repo::read_policy(tx, scope, tenant).await?;
    let tag = policy_tag(&policy)?;
    Ok(support::response(
        StatusCode::OK,
        &PricingApprovalPolicyDto::from(policy),
        Some(tag),
    )?)
}
