//! Revision-owned draft window intentions — pure start resolution and schedule
//! composition (D-374).
//!
//! Draft entries and live baseline references are authoring inputs; [`ProposedWindow`]
//! is an ephemeral validation result projected at `evaluated_at`. Symbolic
//! [`DraftStart::AtPublish`] resolves to the evaluation instant at submit and commit
//! without persisting that resolution as an authored timestamp.
//!
//! ## `at_publish` adjacency hazard
//!
//! Two contiguous intentions `[AtPublish, T)` then `[T, open)` are valid at submit
//! when `evaluated_at < T`, but **must fail at commit** when `evaluated_at >= T`:
//! the symbolic start stamps to the commit instant, leaving an empty interval against
//! the exact neighbour at `T`. Operators who need a hand-off at `T` should author two
//! exact instants, not a symbol plus `T`. An open-ended `AtPublish` alone remains
//! valid at any evaluation instant.

use std::collections::{BTreeMap, HashMap, HashSet};

use toolkit_macros::domain_model;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::domain::error::DomainError;
use crate::domain::instant::{check_quantum, format_rfc3339};
use crate::domain::scope_key::ScopeKey;
use crate::domain::window::{
    WindowInterval, WindowState, check_cancellation, check_effective_to_adjustment,
    interval_is_non_empty, OCCUPYING_STATES,
};

/// How a draft create intention names its start before commit materialization.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraftStart {
    /// An exact UTC instant, validated at the millisecond quantum.
    At(OffsetDateTime),
    /// Symbolic: resolves to `evaluated_at` at each validation projection.
    AtPublish,
}

/// One explicit draft mutation of a window intention or live baseline reference.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DraftWindowAction {
    Create {
        window_id: Uuid,
        price_id: Uuid,
        start: DraftStart,
        effective_to: Option<OffsetDateTime>,
    },
    AdjustEnd {
        window_id: Uuid,
        effective_to: Option<OffsetDateTime>,
    },
    Cancel {
        window_id: Uuid,
    },
}

/// A persisted draft-window operation on a plan revision.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftWindowEntry {
    pub operation_id: Uuid,
    pub action: DraftWindowAction,
    pub reason_code: String,
}

/// Captured live window identity at draft open or baseline refresh.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowBaseline {
    pub window_id: Uuid,
    pub price_id: Uuid,
    pub mutation_seq: u64,
    pub effective_from: OffsetDateTime,
    pub effective_to: Option<OffsetDateTime>,
    pub cancelled: bool,
}

/// Ephemeral composed window after applying draft entries at `evaluated_at`.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposedWindow {
    pub window_id: Uuid,
    pub price_id: Uuid,
    pub key: ScopeKey,
    pub effective_from: OffsetDateTime,
    pub effective_to: Option<OffsetDateTime>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ComposedLiveWindow {
    window_id: Uuid,
    price_id: Uuid,
    effective_from: OffsetDateTime,
    effective_to: Option<OffsetDateTime>,
    cancelled: bool,
}

/// Resolve a draft start at `evaluated_at`.
///
/// # Errors
/// [`DomainError::WindowStartElapsed`] when an exact start is not strictly after
/// `evaluated_at`; [`DomainError::TimestampPrecisionExceeded`] when an exact start
/// carries sub-millisecond precision.
pub fn resolve_start(
    start: &DraftStart,
    evaluated_at: OffsetDateTime,
) -> Result<OffsetDateTime, DomainError> {
    match start {
        DraftStart::AtPublish => Ok(evaluated_at),
        DraftStart::At(at) => {
            check_quantum("start", *at)?;
            if *at > evaluated_at {
                return Ok(*at);
            }
            Err(DomainError::WindowStartElapsed(format!(
                "exact draft start {} is not strictly after {}; an elapsed exact start cannot \
                 commit",
                format_rfc3339(*at),
                format_rfc3339(evaluated_at)
            )))
        }
    }
}

/// Compose baseline references with explicit draft entries into proposed windows.
///
/// # Errors
/// Refuses missing targets, duplicate operations on one window, empty intervals,
/// overlap on a canonical scope key, historical immutability violations, illegal
/// cancellation, elapsed exact starts, and unknown price rows.
pub fn compose_windows(
    baseline: &[WindowBaseline],
    entries: &[DraftWindowEntry],
    keys: &BTreeMap<Uuid, ScopeKey>,
    evaluated_at: OffsetDateTime,
) -> Result<Vec<ProposedWindow>, DomainError> {
    let mut live: HashMap<Uuid, ComposedLiveWindow> = baseline
        .iter()
        .filter(|row| !row.cancelled)
        .map(|row| {
            (
                row.window_id,
                ComposedLiveWindow {
                    window_id: row.window_id,
                    price_id: row.price_id,
                    effective_from: row.effective_from,
                    effective_to: row.effective_to,
                    cancelled: false,
                },
            )
        })
        .collect();

    let mut operated = HashSet::new();

    for entry in entries {
        let window_id = match &entry.action {
            DraftWindowAction::Create { window_id, .. }
            | DraftWindowAction::AdjustEnd { window_id, .. }
            | DraftWindowAction::Cancel { window_id } => *window_id,
        };
        if !operated.insert(window_id) {
            return Err(DomainError::InvalidRequest(format!(
                "window {} is named by more than one draft operation",
                window_id
            )));
        }

        match &entry.action {
            DraftWindowAction::Create {
                window_id,
                price_id,
                start,
                effective_to,
            } => {
                if live.contains_key(window_id) {
                    return Err(DomainError::WindowHistoricalImmutable(format!(
                        "window {window_id} already exists on the baseline; the window↔price \
                         binding is immutable"
                    )));
                }
                if keys.get(price_id).is_none() {
                    return Err(DomainError::InvalidRequest(format!(
                        "price row {price_id} is not in the draft candidate set"
                    )));
                }
                let effective_from = resolve_start(start, evaluated_at)?;
                if matches!(start, DraftStart::At(_)) {
                    check_quantum("effectiveFrom", effective_from)?;
                }
                if !interval_is_non_empty(effective_from, *effective_to) {
                    return Err(empty_interval(effective_from, *effective_to));
                }
                live.insert(
                    *window_id,
                    ComposedLiveWindow {
                        window_id: *window_id,
                        price_id: *price_id,
                        effective_from,
                        effective_to: *effective_to,
                        cancelled: false,
                    },
                );
            }
            DraftWindowAction::AdjustEnd {
                window_id,
                effective_to,
            } => {
                let current = live.get(window_id).ok_or_else(|| {
                    DomainError::InvalidRequest(format!(
                        "window {window_id} is not on the captured baseline"
                    ))
                })?;
                if !keys.contains_key(&current.price_id) {
                    return Err(DomainError::InvalidRequest(format!(
                        "price row {} is not in the draft candidate set",
                        current.price_id
                    )));
                }
                let state = infer_state(current, evaluated_at);
                let interval = WindowInterval::new(
                    current.effective_from,
                    current.effective_to,
                    state,
                );
                check_effective_to_adjustment(&interval, *effective_to, evaluated_at)?;
                if let Some(row) = live.get_mut(window_id) {
                    row.effective_to = *effective_to;
                }
            }
            DraftWindowAction::Cancel { window_id } => {
                let current = live.get(window_id).ok_or_else(|| {
                    DomainError::InvalidRequest(format!(
                        "window {window_id} is not on the captured baseline"
                    ))
                })?;
                if !keys.contains_key(&current.price_id) {
                    return Err(DomainError::InvalidRequest(format!(
                        "price row {} is not in the draft candidate set",
                        current.price_id
                    )));
                }
                let state = infer_state(current, evaluated_at);
                check_cancellation(state)?;
                if let Some(row) = live.get_mut(window_id) {
                    row.cancelled = true;
                }
            }
        }
    }

    let mut proposed = Vec::new();
    for row in live.values().filter(|row| !row.cancelled) {
        let scope_key = keys.get(&row.price_id).ok_or_else(|| {
            DomainError::InvalidRequest(format!(
                "price row {} is not in the draft candidate set",
                row.price_id
            ))
        })?;
        proposed.push(ProposedWindow {
            window_id: row.window_id,
            price_id: row.price_id,
            key: scope_key.clone(),
            effective_from: row.effective_from,
            effective_to: row.effective_to,
        });
    }

    refuse_overlap(&proposed, evaluated_at)?;
    proposed.sort_by(proposed_window_order);
    Ok(proposed)
}

fn infer_state(row: &ComposedLiveWindow, evaluated_at: OffsetDateTime) -> WindowState {
    if row.cancelled {
        return WindowState::Cancelled;
    }
    if row.effective_to.is_some_and(|end| end <= evaluated_at) {
        return WindowState::Expired;
    }
    if row.effective_from <= evaluated_at {
        return WindowState::Active;
    }
    WindowState::Scheduled
}

fn refuse_overlap(proposed: &[ProposedWindow], evaluated_at: OffsetDateTime) -> Result<(), DomainError> {
    for (left_idx, left) in proposed.iter().enumerate() {
        let left_state = window_state(left, evaluated_at);
        if !OCCUPYING_STATES.contains(&left_state) {
            continue;
        }
        for right in proposed.iter().skip(left_idx + 1) {
            if left.key != right.key {
                continue;
            }
            let right_state = window_state(right, evaluated_at);
            if !OCCUPYING_STATES.contains(&right_state) {
                continue;
            }
            if intervals_overlap(
                left.effective_from,
                left.effective_to,
                right.effective_from,
                right.effective_to,
            ) {
                return Err(DomainError::WindowOverlap(format!(
                    "window {} [{}, {}) overlaps window {} [{}, {}) on canonical scope key {}",
                    left.window_id,
                    format_rfc3339(left.effective_from),
                    render_end(left.effective_to),
                    right.window_id,
                    format_rfc3339(right.effective_from),
                    render_end(right.effective_to),
                    left.key
                )));
            }
        }
    }
    Ok(())
}

fn window_state(window: &ProposedWindow, evaluated_at: OffsetDateTime) -> WindowState {
    if window.effective_to.is_some_and(|end| end <= evaluated_at) {
        return WindowState::Expired;
    }
    if window.effective_from <= evaluated_at {
        return WindowState::Active;
    }
    WindowState::Scheduled
}

fn intervals_overlap(
    a_from: OffsetDateTime,
    a_to: Option<OffsetDateTime>,
    b_from: OffsetDateTime,
    b_to: Option<OffsetDateTime>,
) -> bool {
    let a = WindowInterval::new(a_from, a_to, WindowState::Scheduled);
    let b = WindowInterval::new(b_from, b_to, WindowState::Scheduled);
    b.covers(a_from) || a.covers(b_from)
}

fn proposed_window_order(a: &ProposedWindow, b: &ProposedWindow) -> std::cmp::Ordering {
    a.key
        .to_string()
        .cmp(&b.key.to_string())
        .then(a.effective_from.cmp(&b.effective_from))
        .then(compare_effective_to(a.effective_to, b.effective_to))
        .then(a.window_id.cmp(&b.window_id))
}

fn compare_effective_to(
    a: Option<OffsetDateTime>,
    b: Option<OffsetDateTime>,
) -> std::cmp::Ordering {
    match (a, b) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (Some(_), None) => std::cmp::Ordering::Less,
        (Some(a), Some(b)) => a.cmp(&b),
    }
}

fn empty_interval(
    effective_from: OffsetDateTime,
    effective_to: Option<OffsetDateTime>,
) -> DomainError {
    DomainError::InvalidRequest(format!(
        "the window interval [{}, {}) is empty; effectiveTo must be strictly after \
         effectiveFrom",
        format_rfc3339(effective_from),
        render_end(effective_to)
    ))
}

fn render_end(effective_to: Option<OffsetDateTime>) -> String {
    effective_to.map_or_else(|| "open-ended".to_owned(), format_rfc3339)
}

#[cfg(test)]
#[path = "draft_window_tests.rs"]
mod draft_window_tests;
