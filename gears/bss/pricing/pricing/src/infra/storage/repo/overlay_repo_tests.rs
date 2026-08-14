//! The two driver-message classifiers, without a database.
//!
//! Both of them answer a **specific, caller-fixable refusal** — "that precedence
//! is taken", "that `line_id` is taken" — and both decide it from text the driver
//! prints. `storage::policy_guard_or_contention`'s doc argues at length why
//! message matching is the narrow case rather than a precedent, and the argument
//! rests on two things these cases pin: the typed unique class is asked **first**,
//! and the literals are DDL these chains own.
//!
//! The conjunct is the half no database suite can testify about. A real collision
//! is both unique *and* named, so the suites in `tests/sqlite_overlay_repo.rs`
//! (`a_second_published_overlay_on_one_precedence_is_refused`,
//! `a_line_id_already_taken_at_that_revision_is_a_typed_refusal`) stay green
//! whether the conjunct is there or not. What needs a case is the pair the
//! database cannot easily be made to produce: a failure that is **named but not
//! unique**, and one that is **unique but not named**.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use sea_orm::DbErr;
use toolkit_db::secure::ScopeError;

use super::{is_line_identity_collision, precedence_held_or_db};
use crate::infra::storage::RepoError;

/// A driver error carrying exactly this message and nothing the typed class can
/// be read from.
///
/// `DbErr::Custom` has no SQLSTATE, so `is_unique_violation` falls back to the
/// toolkit's own message test — the shared one, not a substring this crate
/// invented — which is precisely the seam these cases exercise.
fn driver_says(message: &str) -> ScopeError {
    ScopeError::Db(DbErr::Custom(message.to_owned()))
}

// ---------------------------------------------------------------------------
// The precedence slot.
// ---------------------------------------------------------------------------

/// Both renderings of the real refusal, one per backend.
///
/// Captured rather than invented: the `SQLite` form is what the mirror prints for
/// `uq_pricing_price_overlay_precedence` (which names the indexed columns, not the
/// index), the Postgres form is that server's documented `unique_violation`
/// message. A rename of the index that forgot this function reddens here.
#[test]
fn a_real_precedence_collision_is_the_precedence_refusal_on_both_backends() {
    for message in [
        "database error: Execution Error: error returned from database: duplicate key value \
         violates unique constraint \"uq_pricing_price_overlay_precedence\"",
        "database error: Execution Error: error returned from database: (code: 2067) UNIQUE \
         constraint failed: pricing_price_overlay.precedence",
    ] {
        assert!(
            matches!(
                precedence_held_or_db(&driver_says(message), "flip pricing_price_overlay"),
                RepoError::OverlayPrecedenceHeld
            ),
            "the real refusal must survive the conjunct: {message}"
        );
    }
}

/// **Named but not unique**: the conjunct's whole purpose.
///
/// A lock report naming the index is a storage fault the caller can do nothing
/// about, and answering it with "that precedence is held" sends them to change a
/// field that was never the problem. Without `is_unique_violation()` this was
/// classified by prose the driver is free to reword.
#[test]
fn a_failure_that_merely_names_the_precedence_index_is_not_the_precedence_refusal() {
    let err = driver_says(
        "database error: Execution Error: error returned from database: deadlock detected; \
         process 8123 waits for ShareLock on index \"uq_pricing_price_overlay_precedence\"",
    );
    assert!(
        matches!(
            precedence_held_or_db(&err, "flip pricing_price_overlay"),
            RepoError::Db(ref detail) if detail.starts_with("flip pricing_price_overlay: ")
        ),
        "a non-unique failure is a storage failure, whatever index its text names"
    );
}

/// **Unique but not named**: the other index on this table.
///
/// `uq_pricing_price_overlay_open_draft` is a second partial unique index on
/// `pricing_price_overlay`, so "a unique index refused" cannot on its own mean the
/// precedence slot. Its loser is not told to pick another precedence.
#[test]
fn a_unique_violation_on_another_overlay_index_is_not_the_precedence_refusal() {
    let err = driver_says(
        "database error: Execution Error: error returned from database: (code: 2067) UNIQUE \
         constraint failed: pricing_price_overlay.tenant_id, \
         pricing_price_overlay.price_overlay_id",
    );
    assert!(err.is_unique_violation(), "the premise of this case");
    assert!(
        matches!(
            precedence_held_or_db(&err, "flip pricing_price_overlay"),
            RepoError::Db(_)
        ),
        "the open-draft index is not the precedence slot"
    );
}

// ---------------------------------------------------------------------------
// The line table's identity.
// ---------------------------------------------------------------------------

/// The line primary key's two renderings.
///
/// `SQLite` reports extended code `1555` for a primary-key violation rather than
/// `2067`, and `sea_orm` folds both into `UniqueConstraintViolation` — which is
/// what makes the conjunct free here rather than a narrowing.
#[test]
fn a_real_line_identity_collision_is_recognised_on_both_backends() {
    for message in [
        "database error: Execution Error: error returned from database: duplicate key value \
         violates unique constraint \"pricing_price_overlay_line_pkey\"",
        "database error: Execution Error: error returned from database: (code: 1555) UNIQUE \
         constraint failed: pricing_price_overlay_line.line_id",
    ] {
        assert!(
            is_line_identity_collision(&driver_says(message)),
            "the real collision must survive the conjunct: {message}"
        );
    }
}

/// Named but not unique, on the line table this time.
///
/// The caller-facing consequence is the sharper of the two: this refusal is
/// `ValueOutOfRange { field: "line_id" }`, which tells an author to drop a field
/// they supplied. A storage fault reported that way is a wrong instruction rather
/// than a generic one.
#[test]
fn a_failure_that_merely_names_the_line_primary_key_is_not_a_line_identity_collision() {
    assert!(
        !is_line_identity_collision(&driver_says(
            "database error: Execution Error: error returned from database: could not serialize \
             access due to concurrent update on index \"pricing_price_overlay_line_pkey\""
        )),
        "a serialization failure is not a caller-fixable line id"
    );
}

/// Unique but not named: the amounts table's own key.
#[test]
fn a_unique_violation_on_the_amount_table_is_not_a_line_identity_collision() {
    let err = driver_says(
        "database error: Execution Error: error returned from database: (code: 1555) UNIQUE \
         constraint failed: pricing_price_overlay_line_amount.line_id, \
         pricing_price_overlay_line_amount.currency",
    );
    assert!(err.is_unique_violation(), "the premise of this case");
    assert!(
        !is_line_identity_collision(&err),
        "the amount row's key is a different constraint, and the qualified table name is \
         what tells them apart"
    );
}
