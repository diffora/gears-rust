//! `domain::read_model` — the visibility matrix cell by cell, and each of
//! §1.7's four names probed on what its own doc claims.

use bss_products_sdk::models::LifecycleState;
use chrono::{TimeZone, Utc};

use super::{
    BrowseProjection, POLLED_SURFACES, ReadProjector, ReadSurface, StalenessStamp,
    VisibilityFilter, scope_condition, serves,
};
use crate::infra::storage::entity::read_entity;

/// **Every cell of `dod-visibility`'s table.** Five states by the three
/// surfaces it names, plus P-D-70 arm 4's by-id read in both positions —
/// written out rather than looped, because a loop over a served-set is the
/// shape that loses `retired`'s single carve-out.
#[test]
fn the_visibility_matrix_is_the_dod_table_cell_for_cell() {
    let default = ReadSurface::DefaultBrowse;
    let keep = ReadSurface::FilteredBrowse {
        exclude_deprecated: false,
    };
    let drop_dep = ReadSurface::FilteredBrowse {
        exclude_deprecated: true,
    };
    let by_id = ReadSurface::ByIdRead {
        state_opt_in: false,
    };
    let by_id_opt_in = ReadSurface::ByIdRead { state_opt_in: true };
    let history = ReadSurface::History;

    // published — served on every surface.
    for s in [default, keep, drop_dep, by_id, by_id_opt_in, history] {
        assert!(serves(LifecycleState::Published, s), "published on {s:?}");
    }

    // deprecated — served with the flag by default, excludable by exactly
    // one filter, and served by the timeline.
    assert!(serves(LifecycleState::Deprecated, default));
    assert!(serves(LifecycleState::Deprecated, keep));
    assert!(
        !serves(LifecycleState::Deprecated, drop_dep),
        "excludeDeprecated is the one filter that changes a cell"
    );
    assert!(serves(LifecycleState::Deprecated, history));

    // retired — never on browse, served by the timeline (the one carve-out),
    // and by the by-id read ONLY under the explicit opt-in.
    assert!(!serves(LifecycleState::Retired, default));
    assert!(!serves(LifecycleState::Retired, keep));
    assert!(!serves(LifecycleState::Retired, drop_dep));
    assert!(
        serves(LifecycleState::Retired, history),
        "the history flow is retired's one carve-out"
    );
    assert!(
        !serves(LifecycleState::Retired, by_id),
        "never the default (P-D-70 arm 4)"
    );
    assert!(
        serves(LifecycleState::Retired, by_id_opt_in),
        "one explicit parameter, and only then"
    );

    // draft and discarded — never, on any surface.
    for state in [LifecycleState::Draft, LifecycleState::Discarded] {
        for s in [default, keep, drop_dep, by_id, by_id_opt_in, history] {
            assert!(!serves(state, s), "{state:?} must never be served on {s:?}");
        }
    }
}

/// The default browse serves exactly two states — the assertion a
/// "withhold what we know about" rule would pass while serving a sixth state
/// added later.
#[test]
fn the_default_browse_serves_exactly_published_and_deprecated() {
    let served = VisibilityFilter::for_surface(ReadSurface::DefaultBrowse).served_states();
    assert_eq!(
        served,
        vec![LifecycleState::Published, LifecycleState::Deprecated]
    );
}

/// The history surface serves three, and `retired` is the third.
#[test]
fn the_history_surface_adds_retired_and_nothing_else() {
    let served = VisibilityFilter::for_surface(ReadSurface::History).served_states();
    assert_eq!(
        served,
        vec![
            LifecycleState::Published,
            LifecycleState::Deprecated,
            LifecycleState::Retired
        ]
    );
}

/// **The contract renders as an `IN` over the served states**, so a row a
/// caller may not see is never fetched. The rendering is asserted rather
/// than the shape, because "applied at query build" is a claim about the SQL
/// and not about where the code sits.
#[test]
fn the_filter_renders_an_in_over_served_states_not_a_negation() {
    use sea_orm::{EntityTrait, QueryFilter, QueryTrait};
    let sql = read_entity::Entity::find()
        .filter(VisibilityFilter::for_surface(ReadSurface::DefaultBrowse).condition())
        .build(sea_orm::DatabaseBackend::Sqlite)
        .to_string();
    assert!(sql.contains("IN ("), "the predicate is an IN: {sql}");
    assert!(
        !sql.contains("NOT IN"),
        "a NOT IN over withheld states would serve any state added later: {sql}"
    );
    assert!(
        sql.contains("'published'") && sql.contains("'deprecated'"),
        "{sql}"
    );
    for withheld in ["'retired'", "'draft'", "'discarded'"] {
        assert!(
            !sql.contains(withheld),
            "{withheld} reached the query: {sql}"
        );
    }
}

/// **The scope predicate admits the unrestricted row** (P-D-39). Containment
/// alone hides the whole catalogue of a tenant that has set no scopes, which
/// is the inverted-obvious the `DoD` warns about.
#[test]
fn the_scope_predicate_admits_the_empty_set_as_unrestricted() {
    use sea_orm::{EntityTrait, QueryFilter, QueryTrait};
    let sql = read_entity::Entity::find()
        .filter(scope_condition(read_entity::Column::RegionScope, "eu"))
        .build(sea_orm::DatabaseBackend::Sqlite)
        .to_string();
    assert!(
        sql.contains(" OR "),
        "the predicate is a disjunction: {sql}"
    );
    assert!(sql.contains("= ''"), "the empty set is admitted: {sql}");
    assert!(sql.contains("eu"), "and so is the claim: {sql}");
}

/// The stamp's anchorless arm exists and carries no version — the case a
/// non-`Option` column could not answer.
#[test]
fn the_stamp_answers_a_tenant_with_no_catalog_version() {
    let at = Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, 0).unwrap();
    let stamp = StalenessStamp::anchorless(at);
    assert_eq!(stamp.as_of_catalog_version, None);
    assert_eq!(stamp.projected_at, at);
}

/// A polled surface's stamp is its own table's last apply and carries no
/// version — and it is a **different constructor** from the anchorless one,
/// so a dashboard's stamp is not read as a bootstrap.
#[test]
fn a_polled_surfaces_stamp_is_named_apart_from_a_bootstrap() {
    let at = Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, 0).unwrap();
    assert_eq!(StalenessStamp::polled(at), StalenessStamp::anchorless(at));
    // Equal by value and distinct by call site: the assertion above is the
    // measurement, and the two names are what a reader routes by.
    assert_eq!(StalenessStamp::polled(at).as_of_catalog_version, None);
}

/// **The projector's subject excludes every polled surface** (§1.7), and the
/// polled roster is exactly the three `inst-ps-dashboards` names.
#[test]
fn the_projector_claims_no_polled_surface() {
    assert_eq!(POLLED_SURFACES.len(), 3);
    for surface in POLLED_SURFACES {
        assert!(
            !ReadProjector::claims(surface),
            "{surface} is a polled projection, not the projector's subject"
        );
    }
    assert!(
        ReadProjector::claims("products_read_entity"),
        "the event-fed projection IS its subject"
    );
}

/// The Consumed roster is six entries and names 10 explicitly as producing
/// none — so a reader cannot mistake its absence for an omission.
#[test]
fn the_consumed_roster_is_six_and_names_the_silent_slice() {
    assert_eq!(ReadProjector::CONSUMED.len(), 6);
    let slices: Vec<&str> = ReadProjector::CONSUMED.iter().map(|(s, _)| *s).collect();
    assert_eq!(slices, ["01", "04", "02", "06", "03", "10"]);
    let (_, note) = ReadProjector::CONSUMED[5];
    assert!(
        note.contains("no events"),
        "10's entry states the absence: {note}"
    );
}

/// The locale-materialized fields are named, so "materialized rather than
/// computed at read" is a checkable claim.
#[test]
fn the_locale_materialized_fields_are_named() {
    assert_eq!(BrowseProjection::LOCALE_MATERIALIZED.len(), 2);
    assert!(BrowseProjection::LOCALE_MATERIALIZED.contains(&"display_attributes"));
}

/// **The ordering the projector assumes is the broker's**, which
/// `dod-read-seams` states as a correction to §1.7's outbox wording — the
/// broker's guarantee being the stronger of the two.
#[test]
fn the_projector_names_the_brokers_ordering_not_the_outboxs() {
    assert!(
        ReadProjector::ORDERING.contains("broker sequence"),
        "the consumer-visible order is the broker's: {}",
        ReadProjector::ORDERING
    );
    assert!(
        !ReadProjector::ORDERING.contains("outbox"),
        "the outbox key is a pipeline invariant P-D-47 supersedes for a consumer"
    );
}
