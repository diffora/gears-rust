//! Bulk import, Phase 1 — the checks that need the store
//! (`design/12-operator-efficiency.md` §3 `algo-bulk-import` `inst-bk-phase1`,
//! §4 `inst-bi-validate`; D-118).
//!
//! # Why this half is here and the other half is in `domain`
//!
//! [`crate::domain::import`] holds the rules a batch can be judged by on its
//! own — the in-batch duplicate and the D-177 refusal. These need reads, so they
//! live on this side of `DE0202`'s line, and both halves write to **one**
//! [`BatchReport`] through [`BatchReport::add`], which is what keeps its
//! one-entry-per-row invariant true of a report two modules have written to.
//!
//! # The import's domain is the draft plane, and that is the rule (D-118)
//!
//! An import row lands as a **draft**: on a key nothing holds, or over an
//! existing draft under its version. A row aimed at a **published** row's key
//! with changed content is refused per-row, and the refusal names its remedy — a
//! **repricing run** — because the remedy exists and is the sibling flow.
//!
//! Published rows are append-only and move only through D-88's supersession
//! units at a bounded changeover instant. This path has neither: no instant in
//! its API, no window operations. Leaving the domain unstated is what invited an
//! import-as-bulk-supersession build, which would reopen the transient
//! fail-closed window D-88 closed.
//!
//! # Identical content is **not** a refusal, and that is deliberate
//!
//! `inst-bk-phase1` refuses a row aimed at a published key **with changed
//! content**. A row identical to what is published is the re-imported file — the
//! ordinary operator act of running the same batch twice — and refusing it would
//! make the second run of an unchanged file an error. It authors a draft that
//! changes nothing, which the draft plane's partial `UNIQUE` admits beside the
//! published row.
//!
//! The comparison is [`PriceRecord::content`]'s, not a field list written here,
//! for the reason that method's own doc gives: restating which columns are
//! content is the restatement that silently drops one the day a slice adds a
//! column.

use std::collections::HashMap;

use toolkit_db::secure::{AccessScope, DBRunner};
use uuid::Uuid;

use crate::domain::import::{BatchReport, IMPORT_TARGETS_PUBLISHED, ImportRow, RowViolation};
use crate::domain::lifecycle::LifecycleState;
use crate::domain::price_record::PriceRecord;
use crate::domain::scope_key::{PlanId, ScopeKey};
use crate::infra::storage::RepoError;
use crate::infra::storage::repo::price_repo;

/// The states a row aimed at one of them is refused for.
///
/// `published` only. A `superseded` or `retired` row is history and holds no
/// key — the draft plane's index does not see it, and neither does the operator.
const OCCUPIED_STATES: &[LifecycleState] = &[LifecycleState::Published];

/// Add Phase 1's **store-dependent** violations to a report the batch-only half
/// has already written.
///
/// One read per plan the batch touches, not one per row: a batch of a thousand
/// rows across three plans is three reads. Rows are grouped by the plan their
/// scope key names, which is also the only place this function needs to know
/// that a batch may span plans at all.
///
/// # Errors
/// Whatever the price repository refuses with.
pub async fn classify_against_store(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    rows: &[ImportRow],
    report: &mut BatchReport,
) -> Result<(), RepoError> {
    let published = published_rows(runner, scope, tenant_id, rows).await?;
    for (index, row) in rows.iter().enumerate() {
        let Some(occupant) = published.get(&row.scope_key) else {
            continue;
        };
        if occupant.content() == row.content {
            continue;
        }
        report.add(
            index,
            RowViolation {
                code: IMPORT_TARGETS_PUBLISHED.to_owned(),
                detail: format!(
                    "this row's scope key is held by a published row ({}) whose content it \
                     changes; an import authors drafts, and a published price moves only \
                     through a repricing run, which carries the changeover instant this path \
                     has no way to state",
                    occupant.price_id
                ),
            },
        );
    }
    Ok(())
}

/// The published rows occupying any key the batch aims at, by key.
///
/// The map is over the **whole** scope key, matching the draft plane's index as
/// `m20260802_000023` widened it — the usage pair included (D-196, D-283).
async fn published_rows(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    rows: &[ImportRow],
) -> Result<HashMap<ScopeKey, PriceRecord>, RepoError> {
    let mut plans: Vec<PlanId> = rows.iter().map(|row| row.scope_key.plan_id()).collect();
    plans.sort_by_key(|plan| plan.get());
    plans.dedup();

    let mut occupied = HashMap::new();
    for plan in plans {
        for record in
            price_repo::load_for_plan(runner, scope, tenant_id, plan, OCCUPIED_STATES).await?
        {
            occupied.insert(record.scope_key.clone(), record);
        }
    }
    Ok(occupied)
}
