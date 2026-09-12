//! Every roster is `CHECK`-pinned in a migration, so the tests assert the
//! exact spellings round-trip and that anything else parses to `None` —
//! the fail-closed arm a defaulting parse would silently lose.

use super::{BatchState, FreezeAckState, FreezeState, ProducerState, RequestState};

#[test]
fn every_spelling_round_trips_and_nothing_else_parses() {
    for state in [
        FreezeState::Open,
        FreezeState::Complete,
        FreezeState::CompleteForced,
    ] {
        assert_eq!(FreezeState::parse(state.as_str()), Some(state));
    }
    for state in [
        FreezeAckState::Pending,
        FreezeAckState::Acked,
        FreezeAckState::Released,
        FreezeAckState::NotFrozenForced,
    ] {
        assert_eq!(FreezeAckState::parse(state.as_str()), Some(state));
    }
    for state in [RequestState::Pending, RequestState::Coalesced] {
        assert_eq!(RequestState::parse(state.as_str()), Some(state));
    }
    for state in [
        BatchState::Staging,
        BatchState::Reported,
        BatchState::Approved,
        BatchState::Committing,
        BatchState::Completed,
        BatchState::Failed,
        BatchState::Abandoned,
    ] {
        assert_eq!(BatchState::parse(state.as_str()), Some(state));
    }
    for state in [ProducerState::Registered, ProducerState::Retired] {
        assert_eq!(ProducerState::parse(state.as_str()), Some(state));
    }

    assert_eq!(FreezeState::parse("OPEN"), None, "no case folding");
    assert_eq!(FreezeAckState::parse("not_frozen"), None);
    assert_eq!(RequestState::parse(""), None);
    assert_eq!(BatchState::parse("done"), None);
    assert_eq!(ProducerState::parse("unregistered"), None);
}

#[test]
fn the_terminal_roster_matches_the_ceiling_filter() {
    // count_live_batches counts everything OUTSIDE these three.
    assert_eq!(
        BatchState::TERMINAL.map(BatchState::as_str),
        ["completed", "failed", "abandoned"]
    );
}
