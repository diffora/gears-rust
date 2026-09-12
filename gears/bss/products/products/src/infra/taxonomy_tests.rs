//! `infra::taxonomy` — the event aggregates, and the lock key's shape.
//!
//! The writer lock itself is measured on Postgres by
//! `tests/postgres_taxonomy_race.rs`, which is where a two-writer race can
//! actually be run. What is measurable here is what the module decides rather
//! than what the engine does.

use uuid::Uuid;

use super::{ATTRIBUTE_VALUE_WRITE_ANNOUNCED_BY, TAXONOMY_TREE_AGGREGATE, metadata_aggregate};
use crate::infra::events::{
    PRODUCT_HEAD_SAVED_PAYLOAD_TYPE, SKU_HEAD_SAVED_PAYLOAD_TYPE, partition_for,
};

const TENANT: Uuid = Uuid::from_u128(0x7e_11);

/// **Every tree event of one tenant lands on one partition**, which is what
/// `dod-taxonomy-events`' *"`(tenant, category tree)` as one aggregate"* buys:
/// the broker orders per partition, so one aggregate is the only way five
/// event types can be ordered against each other.
///
/// The paired half is that the **tenant is an operand**, so a fixed aggregate
/// does not serialize every tenant's taxonomy behind every other's. That is
/// asserted over a spread of tenants rather than over one pair: two ids can
/// share a partition by the modulus, so a single `assert_ne!` would be a
/// claim about a hash collision and not about the formula.
#[test]
fn one_tenants_tree_events_share_a_partition_and_the_tenant_is_an_operand() {
    assert_eq!(
        partition_for(TENANT, TAXONOMY_TREE_AGGREGATE),
        partition_for(TENANT, TAXONOMY_TREE_AGGREGATE),
        "the key is a function of its operands and nothing else"
    );

    let spread: std::collections::HashSet<u32> = (0..64_u128)
        .map(|n| partition_for(Uuid::from_u128(0x7e_00 + n), TAXONOMY_TREE_AGGREGATE))
        .collect();
    assert!(
        spread.len() > 1,
        "a fixed aggregate whose formula ignored the tenant would put every \
         tenant's whole taxonomy on one partition: {spread:?}"
    );
}

/// **A metadata event orders on its entity**, so two entities of one tenant
/// are independent -- which is the ordering their door actually provides,
/// since a metadata write takes no taxonomy lock and rides the entity row's
/// own `If-Match`.
///
/// The assertion is on the **aggregate**, not on the partition. Two
/// aggregates may share a partition through the modulus, and `infra::events`'
/// own doc says what that costs: the local ordering is then *"**stronger**
/// than the `(tenant, aggregate)` key the envelope promises"* -- stricter
/// than required, never weaker. A first version of this case asserted two
/// entities land on different partitions and reddened on exactly that
/// collision, which was the test's premise being wrong and not the key's.
#[test]
fn metadata_orders_on_the_entity_and_not_on_the_tree() {
    let first = Uuid::from_u128(0xf0_01);
    let second = Uuid::from_u128(0xf0_02);
    assert_eq!(metadata_aggregate(first), first);
    assert_ne!(
        metadata_aggregate(first),
        metadata_aggregate(second),
        "two entities are two aggregates"
    );
    assert_ne!(
        metadata_aggregate(first),
        TAXONOMY_TREE_AGGREGATE,
        "metadata does not ride the tree's aggregate"
    );
}

/// **The tree aggregate is a sentinel no entity id can be.**
///
/// Every id this gear mints is a v7 UUID, whose version nibble is `7`. The
/// sentinel's is `0`, so it cannot collide with a category, a product or a
/// SKU however many are created -- which is what lets one fixed value stand
/// for "the tree" without a namespace scheme.
#[test]
fn the_tree_sentinel_cannot_collide_with_a_minted_id() {
    assert_eq!(
        TAXONOMY_TREE_AGGREGATE.get_version_num(),
        0,
        "the sentinel carries no UUID version; every minted id is v7"
    );
    assert_eq!(Uuid::now_v7().get_version_num(), 7, "the paired control");
}

/// **The no-event declaration names the types that DO announce the act**, and
/// those names are held against `infra::events`' own constants.
///
/// A declaration written as a bare `true` would assert nothing -- clippy says
/// so, and it is right. Written as the two substitute payload types it is a
/// real claim, and a real claim can go stale: if `ProductHeadSaved` were ever
/// renamed, a consumer following this declaration would subscribe to a type
/// nothing emits and see attribute-value changes silently stop arriving.
#[test]
fn the_no_event_declaration_names_the_types_that_do_announce() {
    assert_eq!(
        ATTRIBUTE_VALUE_WRITE_ANNOUNCED_BY,
        [PRODUCT_HEAD_SAVED_PAYLOAD_TYPE, SKU_HEAD_SAVED_PAYLOAD_TYPE],
        "the declaration must name the payload types the gear actually emits"
    );
}
