//! Asks every configured source and merges what the caller can read.

use std::sync::Arc;

use bss_approvals_sdk::{
    ApprovalSourceV1, InboxUnit, SortKey, SourceCounts, SourceNarrowing, SourcePage,
    SourcePageQuery, VoteAction, VoteRequest, VoteResponse,
};
use toolkit::ClientHub;
use toolkit::client_hub::ClientScope;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use super::cursor;
use super::error;
use super::merge::{self, SourceAnswer};
use super::owner::{self, SourceGet};
use super::query::PreparedList;

/// Whether a source contributed to the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceHealth {
    /// The source answered.
    Ok,
    /// The source refused the caller. Its units are not in the answer.
    Forbidden,
}

/// One source in a list or counts answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRow {
    /// The configured name.
    pub name: String,
    /// `ok` or `forbidden`.
    pub status: SourceHealth,
}

/// One merged page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    /// The page, in the asked order.
    pub units: Vec<InboxUnit>,
    /// Present when any source had more, or returned a unit this page did not take.
    pub next_cursor: Option<String>,
    /// Every source that was readable or forbidden, in config order.
    pub sources: Vec<SourceRow>,
}

/// The counts of the readable sources, and which sources those are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Counted {
    /// Summed over the readable sources only.
    pub counts: SourceCounts,
    /// Every source that was readable or forbidden, in config order.
    pub sources: Vec<SourceRow>,
}

/// Reads one merged page.
///
/// # Errors
/// A source is down or unregistered, every source forbids the caller, or a source refuses the
/// narrowing. The cursor errors are the caller's, from [`PreparedList`].
pub async fn list_page(
    hub: &ClientHub,
    names: &[String],
    ctx: &SecurityContext,
    prepared: &PreparedList,
) -> Result<Listed, CanonicalError> {
    let mut pages = Vec::new();
    let mut sources = Vec::new();
    let mut down = Vec::new();
    for name in names {
        let after = prepared.keys.get(name).copied().flatten();
        match ask_page(hub, name, ctx, prepared, after).await? {
            Ask::Ready(page) => {
                sources.push(SourceRow {
                    name: name.clone(),
                    status: SourceHealth::Ok,
                });
                pages.push((name.clone(), page));
            }
            Ask::Forbidden => sources.push(SourceRow {
                name: name.clone(),
                status: SourceHealth::Forbidden,
            }),
            Ask::Unavailable => down.push(name.clone()),
        }
    }
    if !down.is_empty() {
        return Err(error::source_unavailable(&down));
    }
    if pages.is_empty()
        && sources
            .iter()
            .any(|row| row.status == SourceHealth::Forbidden)
    {
        return Err(error::forbidden());
    }
    let views: Vec<SourceAnswer<'_>> = pages
        .iter()
        .map(|(name, page)| SourceAnswer {
            source: name,
            units: &page.units,
            has_more: page.has_more,
        })
        .collect();
    let merged = merge::merge(prepared.order, prepared.limit, &prepared.keys, &views);
    let next_cursor = if merged.has_more {
        Some(cursor::encode(
            prepared.order,
            &prepared.hash,
            &merged.keys,
        )?)
    } else {
        None
    };
    Ok(Listed {
        units: merged.units,
        next_cursor,
        sources,
    })
}

/// Sums the counts of the readable sources.
///
/// # Errors
/// A source is down or unregistered, every source forbids the caller, or a source refuses the
/// narrowing.
pub async fn count_all(
    hub: &ClientHub,
    names: &[String],
    ctx: &SecurityContext,
    narrowing: &SourceNarrowing,
) -> Result<Counted, CanonicalError> {
    let mut counts = SourceCounts::default();
    let mut sources = Vec::new();
    let mut down = Vec::new();
    for name in names {
        match ask_counts(hub, name, ctx, narrowing).await? {
            AskCount::Ready(page) => {
                counts.add(&page);
                sources.push(SourceRow {
                    name: name.clone(),
                    status: SourceHealth::Ok,
                });
            }
            AskCount::Forbidden => sources.push(SourceRow {
                name: name.clone(),
                status: SourceHealth::Forbidden,
            }),
            AskCount::Unavailable => down.push(name.clone()),
        }
    }
    if !down.is_empty() {
        return Err(error::source_unavailable(&down));
    }
    if counts_unreadable(&sources) {
        return Err(error::forbidden());
    }
    Ok(Counted { counts, sources })
}

/// The card: one unit, or the owner-resolution refusal.
///
/// # Errors
/// See [`owner::require_one`].
pub async fn get_unit(
    hub: &ClientHub,
    names: &[String],
    ctx: &SecurityContext,
    id: Uuid,
    impact: bool,
) -> Result<InboxUnit, CanonicalError> {
    let resolved = resolve_owner(hub, names, ctx, id, impact).await?;
    owner::require_one(resolved).map(|(_, unit)| unit)
}

/// The owner, then that source's vote door. The answer is the door's own.
///
/// # Errors
/// The card's refusal, or the source's failure to call its door.
pub async fn vote_unit(
    hub: &ClientHub,
    names: &[String],
    ctx: &SecurityContext,
    id: Uuid,
    action: VoteAction,
    request: VoteRequest,
) -> Result<VoteResponse, CanonicalError> {
    let resolved = resolve_owner(hub, names, ctx, id, false).await?;
    let (owner, _) = owner::require_one(resolved)?;
    let Some(source) = lookup(hub, &owner) else {
        return Err(error::source_unavailable(std::slice::from_ref(&owner)));
    };
    source.vote(ctx, id, action, request).await
}

fn counts_unreadable(sources: &[SourceRow]) -> bool {
    !sources.is_empty()
        && sources
            .iter()
            .all(|row| row.status == SourceHealth::Forbidden)
}

async fn resolve_owner(
    hub: &ClientHub,
    names: &[String],
    ctx: &SecurityContext,
    id: Uuid,
    impact: bool,
) -> Result<owner::Resolved, CanonicalError> {
    let mut answers = Vec::with_capacity(names.len());
    for name in names {
        let answer = match lookup(hub, name) {
            None => SourceGet::Unavailable,
            Some(source) => classify_get(source.get(ctx, id, impact).await),
        };
        answers.push((name.clone(), answer));
    }
    Ok(owner::resolve(answers))
}

fn classify_get(result: Result<Option<InboxUnit>, CanonicalError>) -> SourceGet {
    match result {
        Ok(Some(unit)) => SourceGet::Found(Box::new(unit)),
        Ok(None) => SourceGet::Absent,
        Err(err) if err.status_code() == 403 => SourceGet::Forbidden,
        Err(err) if err.status_code() == 503 => SourceGet::Unavailable,
        Err(err) => SourceGet::Failed(Box::new(err)),
    }
}

enum Ask {
    Ready(SourcePage),
    Forbidden,
    Unavailable,
}

async fn ask_page(
    hub: &ClientHub,
    name: &str,
    ctx: &SecurityContext,
    prepared: &PreparedList,
    after: Option<SortKey>,
) -> Result<Ask, CanonicalError> {
    let Some(source) = lookup(hub, name) else {
        return Ok(Ask::Unavailable);
    };
    let query = SourcePageQuery {
        narrowing: prepared.narrowing.clone(),
        order: prepared.order,
        limit: prepared.limit,
        after,
        impact: prepared.impact,
    };
    match source.page(ctx, &query).await {
        Ok(page) => Ok(Ask::Ready(page)),
        Err(err) if err.status_code() == 403 => Ok(Ask::Forbidden),
        Err(err) if err.status_code() == 503 => Ok(Ask::Unavailable),
        Err(err) => Err(err),
    }
}

enum AskCount {
    Ready(SourceCounts),
    Forbidden,
    Unavailable,
}

async fn ask_counts(
    hub: &ClientHub,
    name: &str,
    ctx: &SecurityContext,
    narrowing: &SourceNarrowing,
) -> Result<AskCount, CanonicalError> {
    let Some(source) = lookup(hub, name) else {
        return Ok(AskCount::Unavailable);
    };
    match source.counts(ctx, narrowing).await {
        Ok(counts) => Ok(AskCount::Ready(counts)),
        Err(err) if err.status_code() == 403 => Ok(AskCount::Forbidden),
        Err(err) if err.status_code() == 503 => Ok(AskCount::Unavailable),
        Err(err) => Err(err),
    }
}

fn lookup(hub: &ClientHub, name: &str) -> Option<Arc<dyn ApprovalSourceV1>> {
    hub.try_get_scoped(&ClientScope::new(name))
}
