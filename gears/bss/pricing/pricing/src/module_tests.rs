//! The background plane's supervision — what happens when a ticker dies.
//!
//! `serve` spawns three tickers and, until 2026-08-11, awaited their handles
//! **only after** cancellation. A panic on tick 1 therefore left the gear serving
//! traffic with `serve` still `Ok(())`, and the only trace was a `warn` emitted
//! whenever the process finally stopped.
//!
//! That is not cosmetic here. `serve`'s own doc says the warm re-drive is what
//! resolves a pending `CatalogVersion` handle — without it `pricing_read_model`
//! stays empty and no version ever becomes pin-eligible — and the two Criticals
//! `readmodel_warm` raises cannot fire, because the task that raises them is the
//! dead one.

use super::BssPricingGear;

/// A handle that has already panicked, for the arm under test.
async fn a_panicking_ticker() -> tokio::task::JoinHandle<()> {
    let handle = tokio::spawn(async {
        panic!("a sweep panicked");
    });
    // Let it land, so the arm sees a resolved `Err` rather than a pending future.
    tokio::task::yield_now().await;
    handle
}

/// A handle that ends cleanly, standing in for a surviving sibling.
fn a_surviving_ticker() -> tokio::task::JoinHandle<()> {
    tokio::spawn(async {})
}

/// **A panicking ticker fails `serve` rather than being logged at shutdown.**
///
/// The assertion is on the returned `Err` and on the ticker it names: a
/// supervision arm that reported *something* went wrong without saying which
/// plane died would leave an operator reading three tickers' logs to find out.
#[tokio::test]
async fn a_panicking_ticker_is_reported_through_serves_return() {
    let dead = a_panicking_ticker().await;

    let outcome = BssPricingGear::exited_first(
        "readmodel-warm",
        dead.await,
        a_surviving_ticker(),
        a_surviving_ticker(),
    )
    .await;

    let err = outcome.expect_err("a panicked ticker must not read as a clean stop");
    let rendered = err.to_string();
    assert!(
        rendered.contains("readmodel-warm"),
        "the failure must name the ticker that died: {rendered}"
    );
}

/// **A ticker that returns without panicking is also a failure**, and that is a
/// decision rather than an oversight.
///
/// The loop shape runs until the shared token is cancelled and catches every tick
/// failure itself, so a clean early return is a state its own code does not
/// produce. Treating it as a normal stop would put the background plane back
/// exactly where this arm found it: silently absent, with `serve` reporting
/// healthy.
#[tokio::test]
async fn a_ticker_that_stops_early_without_panicking_is_still_a_failure() {
    let quiet = a_surviving_ticker();

    let outcome = BssPricingGear::exited_first(
        "gated-markets",
        quiet.await,
        a_surviving_ticker(),
        a_surviving_ticker(),
    )
    .await;

    let err = outcome.expect_err("a ticker stopping before cancellation is not a clean stop");
    assert!(err.to_string().contains("gated-markets"), "{err}");
}

/// A sibling's panic is reported even when the ticker that woke the arm was fine.
///
/// The survivors are drained rather than abandoned — each holds a coordination
/// lease — and draining them is worth nothing if what the drain finds is
/// discarded.
#[tokio::test]
async fn a_siblings_panic_is_not_discarded_while_draining() {
    let quiet = a_surviving_ticker();
    let dead = a_panicking_ticker().await;

    let outcome =
        BssPricingGear::exited_first("window-activation", quiet.await, dead, a_surviving_ticker())
            .await;

    assert!(
        outcome.is_err(),
        "a panic found while draining the survivors must not be dropped"
    );
}
