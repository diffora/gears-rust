//! `LeaseManager` — the acquire path for the distributed lease.
//!
//! `acquire` runs the acquire-or-steal inside a `SERIALIZABLE` retry
//! transaction so two workers cannot both observe a free slot and both insert;
//! the loser surfaces as [`CoordError::LeaseHeld`] (PK unique-violation on the
//! INSERT path, or zero-rows on the steal-UPDATE path). The retry helper
//! absorbs transient `40001` aborts transparently.
//!
//! Time anchors on the **DB clock**: the steal/renew filters use `WHERE
//! locked_until < NOW()` (DB-side), so a worker that misreads expiry under NTP
//! drift simply gets `rows_affected == 0` and returns `LeaseHeld` — a
//! false-negative on acquire, never a false-positive steal. The only worker
//! clock used is the `locked_until` written on the free-slot INSERT, and drift
//! there can only *shorten* our own lease versus the DB's view, never extend it
//! past it.

use std::time::Duration;

use chrono::{DateTime, Utc};
use sea_orm::sea_query::{Expr, SimpleExpr};
use sea_orm::{ActiveValue, ColumnTrait, EntityTrait, QueryFilter};
use toolkit_db::Db;
use toolkit_db::secure::{
    ScopeError, SecureEntityExt, SecureUpdateExt, TxConfig, is_unique_violation, secure_insert,
};
use toolkit_security::AccessScope;
use uuid::Uuid;

use super::entity as coord_leases;
use super::error::CoordError;
use super::guard::LeaseGuard;

/// Acquire-side entry point. Cheap to construct (clones an `Arc` inside `Db`)
/// and safe to share behind an `Arc` across job ticks.
#[derive(Clone)]
pub struct LeaseManager {
    db: Db,
}

impl LeaseManager {
    #[must_use]
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    /// Try to acquire the lease keyed by `key` with the given `ttl`.
    ///
    /// On success returns a [`LeaseGuard`] bound to a fresh `locked_by` UUID;
    /// when a peer already holds a live slot returns [`CoordError::LeaseHeld`].
    /// Transient `40001` failures are retried internally; persistent DB
    /// failures surface via [`CoordError::Db`].
    ///
    /// # Errors
    /// * [`CoordError::LeaseHeld`] — peer holds a live, non-expired slot.
    /// * [`CoordError::Db`] — a DB failure not absorbed by the retry helper, or
    ///   an unsupported backend (only Postgres / `SQLite` are supported).
    pub async fn acquire(&self, key: &str, ttl: Duration) -> Result<LeaseGuard, CoordError> {
        let my_uuid = Uuid::new_v4();
        let ttl_secs = ttl_secs_i64(ttl);
        let dialect = Dialect::from_engine(self.db.db_engine())?;
        let key_owned = key.to_owned();

        self.db
            .transaction_with_retry::<(), CoordError, _, _>(
                TxConfig::serializable(),
                CoordError::db_err,
                move |tx| {
                    // FnMut body — clone the captured key per attempt so a
                    // retried iteration owns a fresh `String`. `my_uuid` /
                    // `dialect` / `ttl_secs` are `Copy`.
                    let key = key_owned.clone();
                    Box::pin(async move {
                        let existing = coord_leases::Entity::find()
                            .filter(coord_leases::Column::Key.eq(key.as_str()))
                            .secure()
                            .scope_with(&AccessScope::allow_all())
                            .one(tx)
                            .await
                            .map_err(map_scope_err)?;

                        match existing {
                            None => {
                                // Free slot — INSERT. `locked_until` is written
                                // worker-clock; the steal-path filter is
                                // DB-clock, so drift only shortens our own
                                // lease, never extends it past the DB's view.
                                let row = coord_leases::ActiveModel {
                                    key: ActiveValue::Set(key.clone()),
                                    locked_by: ActiveValue::Set(Some(my_uuid)),
                                    // Checked: `chrono::Duration::seconds` and
                                    // `DateTime + Duration` both panic on overflow,
                                    // and `acquire(key, ttl)` is library-public. A
                                    // pathological `ttl` clamps to the representable
                                    // max instead of panicking; the DB-clock steal
                                    // filter stays authoritative regardless.
                                    locked_until: ActiveValue::Set(
                                        chrono::TimeDelta::try_seconds(ttl_secs)
                                            .and_then(|d| Utc::now().checked_add_signed(d))
                                            .unwrap_or(DateTime::<Utc>::MAX_UTC),
                                    ),
                                    attempts: ActiveValue::Set(1),
                                };
                                match secure_insert::<coord_leases::Entity>(
                                    row,
                                    &AccessScope::allow_all(),
                                    tx,
                                )
                                .await
                                {
                                    Ok(_) => Ok(()),
                                    Err(ScopeError::Db(db)) if is_unique_violation(&db) => {
                                        // A peer raced us between SELECT and
                                        // INSERT and committed first; their row
                                        // is live → `LeaseHeld`.
                                        Err(CoordError::LeaseHeld)
                                    }
                                    Err(err) => Err(map_scope_err(err)),
                                }
                            }
                            Some(row) if row.locked_until <= Utc::now() => {
                                // Read side says expired; the UPDATE re-checks
                                // DB-side via `locked_until < NOW()`. If the row
                                // is in fact still live (drift), zero rows →
                                // `LeaseHeld`.
                                let result = coord_leases::Entity::update_many()
                                    .col_expr(
                                        coord_leases::Column::LockedBy,
                                        Expr::value(my_uuid),
                                    )
                                    .col_expr(
                                        coord_leases::Column::LockedUntil,
                                        dialect.ttl_expr(ttl_secs),
                                    )
                                    .col_expr(
                                        coord_leases::Column::Attempts,
                                        Expr::col(coord_leases::Column::Attempts).add(1),
                                    )
                                    .filter(
                                        coord_leases::Column::Key
                                            .eq(key.as_str())
                                            .and(dialect.expired_filter()),
                                    )
                                    .secure()
                                    .scope_with(&AccessScope::allow_all())
                                    .exec(tx)
                                    .await
                                    .map_err(map_scope_err)?;
                                if result.rows_affected == 0 {
                                    // Belt: under PG SERIALIZABLE a concurrent
                                    // steal normally aborts as 40001 (and
                                    // retries); this arm is reached only when
                                    // `locked_until` advanced between SELECT and
                                    // UPDATE for unrelated reasons. Surface as
                                    // `LeaseHeld` so the caller backs off.
                                    tracing::warn!(
                                        target: "coord.lease",
                                        key = %key,
                                        "lease steal-UPDATE matched zero rows after read-side classified expired; treating as held"
                                    );
                                    return Err(CoordError::LeaseHeld);
                                }
                                Ok(())
                            }
                            Some(_) => Err(CoordError::LeaseHeld),
                        }
                    })
                },
            )
            .await?;

        Ok(LeaseGuard::new(
            self.db.clone(),
            key.to_owned(),
            my_uuid,
            ttl,
        ))
    }
}

/// Supported SQL dialects for the DB-clock lease arithmetic.
///
/// `MySQL` is filtered out once in [`Dialect::from_engine`] (the only fallible
/// step), so the per-expr matches below stay exhaustive over two variants
/// **without a panic** — this is the one structural change from the AM
/// original, whose `engine: &str` expr helpers `panic!`-ed on an unknown
/// dialect (disallowed by this workspace's lints).
#[derive(Clone, Copy)]
pub(super) enum Dialect {
    Postgres,
    Sqlite,
}

impl Dialect {
    /// Classify the `toolkit_db` engine string. Unsupported backends surface as
    /// [`CoordError::Db`] (a `Custom` `DbErr` the retry classifier treats as
    /// non-retryable) instead of panicking.
    pub(super) fn from_engine(engine: &str) -> Result<Self, CoordError> {
        match engine {
            "postgres" => Ok(Self::Postgres),
            "sqlite" => Ok(Self::Sqlite),
            other => Err(CoordError::Db(toolkit_db::DbError::Sea(
                sea_orm::DbErr::Custom(format!(
                    "coord.lease: unsupported db_engine {other:?} (only postgres/sqlite)"
                )),
            ))),
        }
    }

    /// `WHERE`-clause "DB-side now" — the steal / renew / fence filter anchor.
    pub(super) fn now_expr(self) -> SimpleExpr {
        match self {
            Self::Postgres => Expr::cust("NOW()"),
            Self::Sqlite => Expr::cust("datetime('now')"),
        }
    }

    /// The "lease expired" predicate (`locked_until < now`) for the steal filter,
    /// DB-clock-anchored. On `SQLite` the column is wrapped in `datetime()` so a value
    /// stored by the free-slot INSERT (a Rust `DateTime` → RFC-3339, `T`-separated)
    /// and one stored by the steal/renew `ttl_expr` (DB-native, space-separated) both
    /// parse to one canonical form before the compare — a raw TEXT `<` would
    /// mis-order the `T`-form lexicographically and leave a same-day-expired lease
    /// un-stolen. Postgres compares the native `TIMESTAMPTZ`.
    pub(super) fn expired_filter(self) -> SimpleExpr {
        match self {
            Self::Postgres => Expr::col(coord_leases::Column::LockedUntil).lt(self.now_expr()),
            Self::Sqlite => Expr::cust("datetime(locked_until) < datetime('now')"),
        }
    }

    /// The "lease still live" predicate (`locked_until > now`) for the fence + renew
    /// filters — the same `datetime()` normalization on `SQLite` as
    /// [`Self::expired_filter`].
    pub(super) fn live_filter(self) -> SimpleExpr {
        match self {
            Self::Postgres => Expr::col(coord_leases::Column::LockedUntil).gt(self.now_expr()),
            Self::Sqlite => Expr::cust("datetime(locked_until) > datetime('now')"),
        }
    }

    /// "DB-side now + `ttl_secs`" — bumps `locked_until` on acquire-steal/renew.
    pub(super) fn ttl_expr(self, ttl_secs: i64) -> SimpleExpr {
        match self {
            Self::Postgres => Expr::cust(format!("NOW() + INTERVAL '{ttl_secs} seconds'")),
            Self::Sqlite => Expr::cust(format!("datetime('now', '+{ttl_secs} seconds')")),
        }
    }

    /// Epoch sentinel — `release` marks a row free without deleting it, so the
    /// `attempts` forensic streak survives the row's lifetime.
    pub(super) fn epoch_expr(self) -> SimpleExpr {
        match self {
            Self::Postgres => Expr::cust("TIMESTAMP 'epoch'"),
            Self::Sqlite => Expr::cust("'1970-01-01 00:00:00+00:00'"),
        }
    }
}

/// Lift a [`ScopeError`] from a secure-extension call into [`CoordError::Db`].
/// `ScopeError::Db` carries the raw `DbErr` through; the scope-shape variants
/// are unexpected on the unscoped `coord_leases` table and surface as a
/// synthetic `Custom` `DbErr` so the retry classifier sees `None` and
/// propagates.
pub(super) fn map_scope_err(err: ScopeError) -> CoordError {
    match err {
        ScopeError::Db(db) => CoordError::Db(toolkit_db::DbError::Sea(db)),
        other => CoordError::Db(toolkit_db::DbError::Sea(sea_orm::DbErr::Custom(format!(
            "coord.lease: unexpected ScopeError on unscoped coord_leases table: {other:?}"
        )))),
    }
}

/// Saturating `Duration` → seconds. Lease TTLs are minutes-scale, well within
/// `i64::MAX`; `try_from` + `unwrap_or` saturates pathological inputs without a
/// lossy `as` cast (and thus no cast-lint `#[allow]`).
pub(super) fn ttl_secs_i64(ttl: Duration) -> i64 {
    i64::try_from(ttl.as_secs()).unwrap_or(i64::MAX)
}
