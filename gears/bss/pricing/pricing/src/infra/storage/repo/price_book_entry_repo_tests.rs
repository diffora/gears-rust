//! Rename L2: a storage failure of this repository names the entry table in its log line,
//! never the words the Price repository uses for `pricing_price`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

/// Every context string handed to `driver_failure` or `map_unique` in this repository.
fn contexts(source: &str) -> Vec<&str> {
    let mut found = Vec::new();
    for call in ["driver_failure(\"", "map_unique(\""] {
        for (at, _) in source.match_indices(call) {
            let rest = &source[at + call.len()..];
            found.push(&rest[..rest.find('"').unwrap()]);
        }
    }
    found
}
#[test]
fn every_storage_context_names_the_price_book_entry() {
    let found = contexts(include_str!("price_book_entry_repo.rs"));
    assert_eq!(found.len(), 9, "{found:?}");
    for context in found {
        assert!(context.contains("price book entr"), "{context}");
    }
}
