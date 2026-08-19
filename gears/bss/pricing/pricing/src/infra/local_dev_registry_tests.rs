//! Tests for the locally invented `CatalogVersion` source.
//!
//! Armed against the four things that would make it worse than the fail-closed
//! default it replaces: a version re-issued, a sequence that does not increase, a
//! retry that mints twice, and a ref that resolves to nothing after a restart.

use super::{DEV_LOCAL_REF_PREFIX, LocalDevCatalogVersionRegistryV1};
use crate::domain::ports::{CatalogVersionRegistryError, CatalogVersionRegistryV1};

/// The registry never reads the context, so any context serves.
fn ctx() -> toolkit_security::SecurityContext {
    toolkit_security::SecurityContext::anonymous()
}

#[tokio::test]
async fn a_request_mints_a_marked_ref_that_carries_its_version() {
    let registry = LocalDevCatalogVersionRegistryV1::new();

    let pending = registry
        .request_version(&ctx(), "req-1")
        .await
        .expect("a local source cannot be unavailable");

    assert_eq!(pending.request_id, "req-1");
    assert!(
        pending.pending_ref.starts_with(DEV_LOCAL_REF_PREFIX),
        "every artifact must be sweepable later: {}",
        pending.pending_ref
    );

    let committed = registry
        .committed_version(&ctx(), &pending.pending_ref)
        .await
        .expect("its own ref resolves")
        .expect("this source commits immediately");
    assert!(
        committed.get() > 0,
        "a version of zero would sort before every real one"
    );
}

#[tokio::test]
async fn a_retry_of_one_request_id_mints_no_second_version() {
    // The contract's stated purpose for `request_id`: "re-requesting with the same
    // id after a crash must not enqueue a second publish for versioning."
    let registry = LocalDevCatalogVersionRegistryV1::new();

    let first = registry.request_version(&ctx(), "req-same").await.unwrap();
    let again = registry.request_version(&ctx(), "req-same").await.unwrap();

    assert_eq!(
        first.pending_ref, again.pending_ref,
        "a retry must replay the first answer, not mint beside it"
    );
}

#[tokio::test]
async fn the_sequence_is_strictly_increasing_even_inside_one_millisecond() {
    // The clock alone does not give this: a hundred requests land well inside one
    // millisecond, and two publishes sharing a version are indistinguishable
    // rather than merely mis-sorted. This is the case the `max(now, last + 1)`
    // exists for, and a probe over a single call could not see it.
    let registry = LocalDevCatalogVersionRegistryV1::new();

    let mut versions = Vec::new();
    for n in 0..100 {
        let pending = registry
            .request_version(&ctx(), &format!("req-{n}"))
            .await
            .unwrap();
        let version = registry
            .committed_version(&ctx(), &pending.pending_ref)
            .await
            .unwrap()
            .unwrap();
        versions.push(version.get());
    }

    let mut sorted = versions.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        versions.len(),
        "a version was issued twice: {versions:?}"
    );
    assert_eq!(
        sorted, versions,
        "the sequence must be issued in increasing order, not merely be distinct"
    );
}

#[tokio::test]
async fn a_ref_from_a_previous_process_still_resolves() {
    // The reason the version travels inside the ref rather than in a map. The
    // projector resolves refs written by an earlier process; a lookup would answer
    // `None` for every one of them and strand each revision published before the
    // last boot — which is exactly the unaddressable state this type was brought
    // in to end.
    let fresh = LocalDevCatalogVersionRegistryV1::new();

    let committed = fresh
        .committed_version(&ctx(), &format!("{DEV_LOCAL_REF_PREFIX}1755200000000"))
        .await
        .expect("a ref this source recognises")
        .expect("and commits");

    assert_eq!(committed.get(), 1_755_200_000_000);
}

#[tokio::test]
async fn a_ref_this_source_did_not_mint_is_rejected_rather_than_left_pending() {
    // `None` means "not committed yet" and would have the caller wait forever for
    // a ref this source can never commit. Both halves are asserted because the
    // difference between them is a hang and an error.
    let registry = LocalDevCatalogVersionRegistryV1::new();

    for foreign in ["registry-issued-ref-42", "", "dev-local-vNOTANUMBER"] {
        let outcome = registry.committed_version(&ctx(), foreign).await;
        assert!(
            matches!(outcome, Err(CatalogVersionRegistryError::Rejected(_))),
            "expected a rejection for {foreign:?}, got {outcome:?}"
        );
    }
}
