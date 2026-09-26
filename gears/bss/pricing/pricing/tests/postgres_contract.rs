//! The golden consumer contracts on Postgres (run 4.4): the same body as `contract.rs` runs on
//! `SQLite`, against the same files under `tests/contract/`. This tier only compares — it never
//! re-records — so a contract that holds on one backend and drifts on the other is red here.
#![allow(clippy::expect_used, clippy::unwrap_used)]
mod contract_support;
mod pg_support;
mod plan_support;
use plan_support::{
    Catalog, Fixture,
    entry_support::{app_for, state_on, user_of},
};
use std::sync::Arc;
use toolkit_db::{DBProvider, DbError};
use uuid::Uuid;

async fn check(golden: &str) {
    let pg = pg_support::Pg::applied().await;
    let catalog = Arc::new(Catalog::default());
    let ctx = user_of(Uuid::new_v4());
    let db = DBProvider::<DbError>::new(pg.db().await);
    let state = state_on(db.clone(), catalog.clone()).await;
    let app = app_for(state.clone(), ctx.subject_tenant_id());
    let f = Fixture {
        dsn: pg.url(true),
        state,
        app: app.clone(),
        denied: app,
        ctx,
        db,
    };
    let world = contract_support::world(f, &catalog).await;
    contract_support::verify(&world, golden, false).await;
}

macro_rules! goldens {
    ($($golden:ident),* $(,)?) => {
        $(
            #[tokio::test]
            #[ignore = "needs the Postgres harness"]
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
