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
//! # The refusal is **unconditional**, and that departs from the clause's wording
//!
//! `inst-bk-phase1` says "with changed content", and this rule refuses whatever
//! the content is. **Flagged for veto (D-287.)** Three reasons, in order of
//! weight:
//!
//! 1. **Three other places assert the refusal unconditionally, and one of them is
//!    a correctness argument in shipped code.** `infra::publish`'s
//!    `validated_draft_rows` is a no-op "on every world that predates the second
//!    door — nothing else can put a draft on an occupied key (`inst-pr-return`
//!    refuses it at save, `IMPORT_TARGETS_PUBLISHED` per row on the bulk plane)".
//!    `03-price-structure.md` §6's two-doors argument and
//!    `07-pricewindow-linkage.md`'s `inst-su-compose` say the same.
//! 2. **The carve-out made keys unsupersedable, silently.** An unchanged row
//!    authors a no-op draft beside the published row, which the draft plane's
//!    partial `UNIQUE` admits — and `price_repo`'s supersession door **refuses a
//!    draft occupant**. So running the same batch twice would block the repricing
//!    run this rule's own message names as the remedy.
//! 3. **Idempotency already serves the case the carve-out was for.**
//!    `inst-bk-idem` replays the client key to the original report, so re-running
//!    an unchanged file is not an error and never reaches these rules at all. The
//!    carve-out solved a problem something else solves and created one nothing
//!    does.
//!
//! A fourth, smaller: `PriceRecord::content` carries `supersedes_price_id` and
//! `grandfather_until`, which the **store** stamps. A row the operator authored
//! identically to a superseded-into published row compared unequal anyway, so the
//! message "whose content it changes" was false in exactly the case the carve-out
//! existed for.

use std::collections::{BTreeSet, HashMap};

use toolkit_db::secure::{AccessScope, DBRunner};
use uuid::Uuid;

use crate::domain::approval::PENDING_CHANGE_UNIT_EXISTS;
use crate::domain::import::{BatchReport, IMPORT_TARGETS_PUBLISHED, ImportRow, RowViolation};
use crate::domain::lifecycle::LifecycleState;
use crate::domain::price_record::PriceRecord;
use crate::domain::scope_key::{PlanId, ScopeKey};
use crate::infra::storage::RepoError;
use crate::infra::storage::repo::{approval_repo, price_repo};

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
    targets_published(runner, scope, tenant_id, rows, report).await?;
    key_holds_a_pending_unit(runner, scope, tenant_id, rows, report).await?;
    Ok(())
}

/// D-118: a row aimed at a **published** row's key, changing its content.
async fn targets_published(
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
        report.add(
            index,
            RowViolation {
                code: IMPORT_TARGETS_PUBLISHED.to_owned(),
                detail: format!(
                    "this row's scope key is held by published row {}; an import authors \
                     drafts, and a published price moves only through a repricing run, which \
                     carries the changeover instant this path has no way to state",
                    occupant.price_id
                ),
            },
        );
    }
    Ok(())
}

/// D-35: a row whose canonical scope key is held by a **pending** approval unit.
///
/// The PRD's one-pending-unit rule, applied per row. A held key is a poor target
/// even for a draft: the unit holding it is a supersession or a cutover on its way
/// to a decision, and an import authoring underneath it would author against a
/// state somebody is in the middle of changing.
///
/// **It inherits `PENDING_CHANGE_UNIT_EXISTS`** rather than minting an import
/// code — D-282's line: one rule, one code, however many surfaces notice it. The
/// interactive submit answers the same code on the same fact.
///
/// The register is read through `find_pending_key_holders`, the **plural** of the
/// read a submit uses, added for this caller. A submit stops at the first conflict
/// because one is enough to refuse; Phase 1 owes a verdict on every row and cannot.
async fn key_holds_a_pending_unit(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    rows: &[ImportRow],
    report: &mut BatchReport,
) -> Result<(), RepoError> {
    let keys: BTreeSet<String> = rows.iter().map(|row| row.scope_key.to_string()).collect();
    let holders: HashMap<String, _> =
        approval_repo::find_pending_key_holders(runner, scope, tenant_id, &keys)
            .await?
            .into_iter()
            .map(|(record, key)| (key, record))
            .collect();
    if holders.is_empty() {
        return Ok(());
    }

    for (index, row) in rows.iter().enumerate() {
        let Some(unit) = holders.get(&row.scope_key.to_string()) else {
            continue;
        };
        report.add(
            index,
            RowViolation {
                code: PENDING_CHANGE_UNIT_EXISTS.to_owned(),
                detail: format!(
                    "this row's scope key is held by pending approval unit {}; decide or \
                     withdraw it before importing onto that key, which the one-pending-unit \
                     rule reserves for the change already under way",
                    unit.approval_id
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
