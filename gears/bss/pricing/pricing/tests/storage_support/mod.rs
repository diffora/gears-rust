//! File-backed `SQLite` for repository races.
#![allow(clippy::expect_used, clippy::unwrap_used)]
use toolkit::contracts::DatabaseCapability;
use toolkit_db::secure::AccessScope;
use toolkit_db::{ConnectOpts, DBProvider, DbError};
use uuid::Uuid;
pub async fn test_db() -> (DBProvider<DbError>, AccessScope, Uuid, String) {
    let dsn = format!(
        "sqlite://{}?mode=rwc",
        std::env::temp_dir()
            .join(format!("pricing-repos-{}.sqlite3", Uuid::new_v4()))
            .display()
    );
    let db = toolkit_db::connect_db(
        &dsn,
        ConnectOpts {
            max_conns: Some(1),
            min_conns: Some(1),
            ..ConnectOpts::default()
        },
    )
    .await
    .unwrap();
    toolkit_db::migration_runner::run_migrations_for_testing(
        &db,
        bss_pricing::module::BssPricingGear::default().migrations(),
    )
    .await
    .unwrap();
    let tenant = Uuid::new_v4();
    (
        DBProvider::new(db),
        AccessScope::for_tenant(tenant),
        tenant,
        dsn,
    )
}
pub fn at(hour: u8) -> time::OffsetDateTime {
    time::Date::from_calendar_date(2026, time::Month::September, 26)
        .unwrap()
        .with_hms(hour, 0, 0)
        .unwrap()
        .assume_utc()
}
