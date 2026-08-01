//! Tests for the frozen event-name set.

use std::collections::BTreeSet;

use super::CatalogEvent;

/// The frozen names, written out as literals.
///
/// Deliberately not derived from the variants: the whole point is that
/// renaming a Rust variant must not rename a wire event, so this list has to be
/// an independent statement of the contract that a rename breaks.
const FROZEN_WIRE_NAMES: &[&str] = &[
    "PlanCreated",
    "PlanUpdated",
    "PlanPublished",
    "PlanRetired",
    "PlanMigrationScheduled",
    "PlanPublishDegraded",
    "BundleUpdated",
    "PriceCreated",
    "PriceUpdated",
    "PriceWindowScheduled",
    "PriceWindowActivated",
    "PriceWindowExpired",
    "PriceWindowCancelled",
];

#[test]
fn the_wire_names_are_exactly_the_frozen_strings() {
    let emitted: Vec<&str> = CatalogEvent::ALL
        .iter()
        .map(|event| event.as_str())
        .collect();

    assert_eq!(emitted, FROZEN_WIRE_NAMES);
}

#[test]
fn the_set_is_thirteen_names_and_no_more() {
    // Adding a name is a contract change and has to be argued for, so the
    // count is asserted rather than left to follow the enum.
    assert_eq!(CatalogEvent::ALL.len(), 13);
    assert_eq!(FROZEN_WIRE_NAMES.len(), 13);
}

#[test]
fn all_lists_every_variant_once() {
    // A missed variant in `ALL` would be an event the outbox registration
    // never sees, and it would fail silently: nothing subscribes to it.
    let distinct: BTreeSet<CatalogEvent> = CatalogEvent::ALL.iter().copied().collect();

    assert_eq!(distinct.len(), CatalogEvent::ALL.len());
}

#[test]
fn no_two_events_share_a_wire_name() {
    let distinct: BTreeSet<&str> = CatalogEvent::ALL
        .iter()
        .map(|event| event.as_str())
        .collect();

    assert_eq!(distinct.len(), CatalogEvent::ALL.len());
}

#[test]
fn there_is_no_deletion_event() {
    // Published rows are never deleted: a row leaves service by supersession
    // or by its plan retiring, and stays readable as history — so no consumer
    // ever has to reconcile a disappearance.
    assert!(
        !CatalogEvent::ALL
            .iter()
            .any(|event| event.as_str().contains("Delete") || event.as_str().contains("Removed"))
    );
}

#[test]
fn display_and_as_str_agree() {
    for event in CatalogEvent::ALL {
        assert_eq!(event.to_string(), event.as_str());
    }
}
