//! The golden consumer contracts on `SQLite` (run 4.4): the one body is `contract_support`, the
//! same one `postgres_contract.rs` runs. This tier alone re-records: `UPDATE_CONTRACT_GOLDEN=1`
//! rewrites `tests/contract/<golden>.json` from the doors' answers — a claim that the contract
//! was MEANT to change, which the diff has to justify.
#![allow(clippy::expect_used, clippy::unwrap_used)]
mod contract_support;
mod plan_support;

async fn check(golden: &str) {
    let (f, catalog) = plan_support::setup().await;
    let world = contract_support::world(f, &catalog).await;
    let record = std::env::var("UPDATE_CONTRACT_GOLDEN").is_ok();
    contract_support::verify(&world, golden, record).await;
}

macro_rules! goldens {
    ($($golden:ident),* $(,)?) => {
        /// The contracts this tier checks, one test each.
        const CHECKED: &[&str] = &[$(stringify!($golden)),*];
        $(
            #[tokio::test]
            async fn $golden() {
                check(stringify!($golden)).await;
            }
        )*
    };
}
goldens!(
    resolve_signup,
    resolve_renewal_walk,
    resolve_ended_chain,
    resolve_default_pin_moves,
    resolve_matrix_uncovered,
    resolve_sku_version_by_date,
    resolve_invoice_inputs,
    resolve_superseded_revision,
    resolve_refusals,
    price_approved_open,
    price_closed,
    price_keep_for_bound,
    price_not_found,
);

/// A golden no test reads is a contract nobody checks: `tests/contract/` holds exactly the
/// contracts this tier checks, and they are the ones the shared body knows.
#[test]
fn every_golden_file_is_a_checked_contract() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/contract");
    let mut files: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    files.sort();
    let mut checked: Vec<String> = CHECKED.iter().map(|g| format!("{g}.json")).collect();
    checked.sort();
    assert_eq!(
        files, checked,
        "every file in tests/contract/ is a checked golden"
    );
    let mut known: Vec<&str> = contract_support::GOLDENS.to_vec();
    known.sort_unstable();
    let mut tested: Vec<&str> = CHECKED.to_vec();
    tested.sort_unstable();
    assert_eq!(known, tested, "every contract of the shared body is tested");
}
