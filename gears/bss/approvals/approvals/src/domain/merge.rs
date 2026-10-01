//! K-way merge of source pages by `(submitted_at, id)`. No I/O.
//!
//! Every source is asked on every page, after its own key. A key becomes the last unit taken
//! from that source, or stays when none of its units were taken. There is no exhausted state.
//! `has_more` is set when any source had more, or returned a unit this page did not take.

use std::collections::BTreeMap;

use bss_approvals_sdk::{InboxUnit, Order, SortKey};

/// One source's answer for the page being merged.
pub struct SourceAnswer<'a> {
    /// The configured source name.
    pub source: &'a str,
    /// Units the source returned, already past its key.
    pub units: &'a [InboxUnit],
    /// The source has a further unit after `units`.
    pub has_more: bool,
}

/// The merged page and the key each source carries into the next page.
pub struct MergedPage {
    /// The first `limit` units in the asked order.
    pub units: Vec<InboxUnit>,
    /// Each asked source's next key: the last unit taken from it, or the key it arrived with.
    pub keys: BTreeMap<String, Option<SortKey>>,
    /// Another page exists: some source had more, or returned a unit that was not taken.
    pub has_more: bool,
}

/// Merges one round of source answers.
///
/// `incoming` holds the key each source was asked after. A source with no entry starts from
/// `None`. Sources that are not in `pages` are ignored.
#[must_use]
pub fn merge(
    order: Order,
    limit: u32,
    incoming: &BTreeMap<String, Option<SortKey>>,
    pages: &[SourceAnswer<'_>],
) -> MergedPage {
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    let mut index = vec![0_usize; pages.len()];
    let mut taken: Vec<Option<SortKey>> = vec![None; pages.len()];
    let mut units = Vec::new();

    while units.len() < limit {
        let Some(pick) = next_unit(order, pages, &index) else {
            break;
        };
        let unit = &pages[pick].units[index[pick]];
        taken[pick] = Some(SortKey::of(unit));
        units.push(unit.clone());
        index[pick] = index[pick].saturating_add(1);
    }

    let mut keys = BTreeMap::new();
    let mut has_more = false;
    for (slot, page) in pages.iter().enumerate() {
        let previous = incoming.get(page.source).copied().flatten();
        let next_key = taken[slot].or(previous);
        keys.insert(page.source.to_owned(), next_key);
        if page.has_more || index[slot] < page.units.len() {
            has_more = true;
        }
    }

    MergedPage {
        units,
        keys,
        has_more,
    }
}

/// The source whose next unit sorts first, if any source still has one.
fn next_unit(order: Order, pages: &[SourceAnswer<'_>], index: &[usize]) -> Option<usize> {
    let mut best: Option<usize> = None;
    for (slot, page) in pages.iter().enumerate() {
        let Some(unit) = page.units.get(index[slot]) else {
            continue;
        };
        let replace = match best {
            None => true,
            Some(current) => comes_first(unit, &pages[current].units[index[current]], order),
        };
        if replace {
            best = Some(slot);
        }
    }
    best
}

/// Whether `left` sorts before `right` in `order`. Equal keys keep the earlier source.
fn comes_first(left: &InboxUnit, right: &InboxUnit, order: Order) -> bool {
    let left_key = (left.submitted_at, left.id);
    let right_key = (right.submitted_at, right.id);
    match order {
        Order::Asc => left_key < right_key,
        Order::Desc => left_key > right_key,
    }
}
