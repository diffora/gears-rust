//! Gear-declaration smoke tests: the capability wiring is real, not decorative.

use std::collections::HashSet;

use bss_pricing::module::BssPricingGear;
use toolkit::contracts::DatabaseCapability;

#[test]
fn the_gear_declares_the_database_capability() {
    // The `db` capability must resolve to the Foundation chain, not to an empty
    // vec: a gear that declares `db` and hands the platform nothing has tables
    // no migration ever creates.
    let gear = BssPricingGear::default();

    assert!(
        !gear.migrations().is_empty(),
        "the Foundation migration chain must be wired into the db capability"
    );
}

#[test]
fn every_migration_name_is_unique() {
    // The toolkit runner applies migrations in NAME order and rejects a
    // duplicate name outright — so a copy-pasted `DeriveMigrationName` does not
    // merely sort oddly, it aborts the whole chain at boot. Asserting it here
    // means the mistake is caught by `cargo test` rather than by a crash loop
    // in a cluster.
    let gear = BssPricingGear::default();
    let migrations = gear.migrations();
    let names: Vec<String> = migrations.iter().map(|m| m.name().to_owned()).collect();
    let unique: HashSet<&String> = names.iter().collect();

    assert_eq!(
        unique.len(),
        names.len(),
        "duplicate migration name in the chain: {names:?}"
    );
}
