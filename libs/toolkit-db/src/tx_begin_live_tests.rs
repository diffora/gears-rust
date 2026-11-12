#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a failed database setup or assertion is a test failure"
)]
//! `tx_begin::begin` on live servers.
//!
//! A top-level transaction dropped while its `BEGIN` is in flight: a proxy between the pool
//! and the server holds back the server's answer to one `BEGIN`. The test drops the caller
//! while that answer is held, so the drop always lands while the `BEGIN` is in flight, then
//! lets the answer through and checks the connection the pool gets back. The tests live in
//! the crate because they run statements on the `Db`'s own pool and transactions.
//!
//! The begin's task runs under its caller's subscriber: a subscriber set for one future sees
//! the statements of its transaction's begin.

use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
use testcontainers::{ImageExt, runners::AsyncRunner};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

use crate::secure::{TxAccessMode, TxConfig, TxIsolationLevel};
use crate::{ConnectOpts, DbError, DbTx, connect_db};

/// Upper bound on each wait, so that a hang fails the test instead of stalling the run.
const PATIENCE: Duration = Duration::from_mins(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gate {
    /// Everything passes.
    Open,
    /// The server's answer to the next `BEGIN` a client sends is held.
    Armed,
    /// A `BEGIN` has gone to the server, and its answer is held.
    Holding,
}

/// A TCP proxy in front of a database server that can hold back the answer to one `BEGIN`.
struct BeginGate {
    addr: SocketAddr,
    gate: Arc<watch::Sender<Gate>>,
}

impl BeginGate {
    async fn start(server: (String, u16)) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let gate = Arc::new(watch::Sender::new(Gate::Open));
        let shared = Arc::clone(&gate);
        tokio::spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                let upstream = TcpStream::connect(server.clone()).await.unwrap();
                let (client_read, client_write) = client.into_split();
                let (server_read, server_write) = upstream.into_split();
                tokio::spawn(client_to_server(
                    client_read,
                    server_write,
                    Arc::clone(&shared),
                ));
                tokio::spawn(server_to_client(
                    server_read,
                    client_write,
                    shared.subscribe(),
                ));
            }
        });
        Self { addr, gate }
    }

    fn arm(&self) {
        self.gate.send_replace(Gate::Armed);
    }

    /// Returns once a `BEGIN` has gone to the server with its answer held.
    async fn begin_held(&self) {
        self.gate
            .subscribe()
            .wait_for(|gate| *gate == Gate::Holding)
            .await
            .unwrap();
    }

    fn release(&self) {
        self.gate.send_replace(Gate::Open);
    }
}

async fn client_to_server(
    mut client: OwnedReadHalf,
    mut server: OwnedWriteHalf,
    gate: Arc<watch::Sender<Gate>>,
) {
    let mut buf = vec![0; 64 * 1024];
    while let Ok(read @ 1..) = client.read(&mut buf).await {
        let bytes = &buf[..read];
        // Close the gate before the BEGIN reaches the server, so its answer is always held.
        gate.send_if_modified(|gate| {
            let hold = *gate == Gate::Armed && bytes.windows(5).any(|word| word == b"BEGIN");
            if hold {
                *gate = Gate::Holding;
            }
            hold
        });
        if server.write_all(bytes).await.is_err() {
            break;
        }
    }
}

async fn server_to_client(
    mut server: OwnedReadHalf,
    mut client: OwnedWriteHalf,
    mut gate: watch::Receiver<Gate>,
) {
    let mut buf = vec![0; 64 * 1024];
    while let Ok(read @ 1..) = server.read(&mut buf).await {
        if gate.wait_for(|gate| *gate != Gate::Holding).await.is_err()
            || client.write_all(&buf[..read]).await.is_err()
        {
            break;
        }
    }
}

fn noop<'a>(_: &'a DbTx<'a>) -> Pin<Box<dyn Future<Output = Result<(), DbError>> + Send + 'a>> {
    Box::pin(async { Ok(()) })
}

async fn count(conn: &DatabaseConnection, sql: &str) -> i64 {
    conn.query_one_raw(Statement::from_string(conn.get_database_backend(), sql))
        .await
        .unwrap()
        .unwrap()
        .try_get_by_index(0)
        .unwrap()
}

/// What the pool's one connection shows after a transaction was dropped in its `BEGIN`.
#[derive(Debug, PartialEq, Eq)]
struct Aftermath {
    /// Transactions other sessions see open.
    open_transactions: i64,
    /// An autocommit `INSERT` on the pool, as another session sees it.
    insert_seen: i64,
    /// A `REPEATABLE READ READ ONLY` transaction on the pool.
    snapshot_begin: Result<(), String>,
    /// The `INSERT`, as another session sees it after that transaction.
    insert_kept: i64,
}

const CLEAN: Aftermath = Aftermath {
    open_transactions: 0,
    insert_seen: 1,
    snapshot_begin: Ok(()),
    insert_kept: 1,
};

/// Drop a transaction on a one-connection pool while the server's answer to its `BEGIN` is
/// held, then let the answer through and see what the connection shows.
///
/// `watch` reaches the server directly; `proxied_dsn` names the proxy's address for the pool.
/// `open_transactions` counts the transactions open in other sessions.
async fn drop_a_transaction_in_its_begin(
    watch: &DatabaseConnection,
    server: (String, u16),
    proxied_dsn: impl FnOnce(SocketAddr) -> String,
    open_transactions: &str,
) -> Aftermath {
    watch
        .execute_unprepared("CREATE TABLE t (x int)")
        .await
        .unwrap();
    let gate = BeginGate::start(server).await;
    let db = connect_db(
        &proxied_dsn(gate.addr),
        ConnectOpts {
            max_conns: Some(1),
            ..ConnectOpts::default()
        },
    )
    .await
    .unwrap();
    // Open the pool's one connection, so the next transaction only has to send BEGIN.
    db.transaction_ref_mapped_with_config(TxConfig::default(), noop)
        .await
        .unwrap();

    gate.arm();
    let mut tx = Box::pin(db.transaction_ref_mapped_with_config(TxConfig::default(), noop));
    tokio::select! {
        () = gate.begin_held() => {}
        ended = &mut tx => panic!("the transaction ended while its BEGIN answer was held: {ended:?}"),
    }
    drop(tx);
    gate.release();

    // The pool has one connection, so this INSERT runs once the dropped transaction's
    // connection is back in the pool.
    db.sea_internal()
        .execute_unprepared("INSERT INTO t VALUES (1)")
        .await
        .unwrap();
    let open_transactions = count(watch, open_transactions).await;
    let insert_seen = count(watch, "SELECT count(*) FROM t").await;
    let snapshot_begin = db
        .transaction_ref_mapped_with_config(
            TxConfig {
                isolation: Some(TxIsolationLevel::RepeatableRead),
                access_mode: Some(TxAccessMode::ReadOnly),
            },
            noop,
        )
        .await
        .map_err(|e| e.to_string());
    let insert_kept = count(watch, "SELECT count(*) FROM t").await;
    Aftermath {
        open_transactions,
        insert_seen,
        snapshot_begin,
        insert_kept,
    }
}

/// The connection of a transaction dropped during its `BEGIN` comes back to the pool outside
/// any transaction: an autocommit statement on it commits at once, and a `REPEATABLE READ`
/// transaction can begin on it.
#[cfg(feature = "pg")]
#[tokio::test]
async fn postgres_a_transaction_dropped_in_its_begin_leaves_its_connection_clean() {
    let container = test_containers::postgres()
        .with_env_var("POSTGRES_PASSWORD", "pass")
        .with_env_var("POSTGRES_USER", "user")
        .with_env_var("POSTGRES_DB", "app")
        .start()
        .await
        .expect("start pinned PostgreSQL");
    let host = container.get_host().await.unwrap().to_string();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let watch = sea_orm::Database::connect(format!("postgres://user:pass@{host}:{port}/app"))
        .await
        .unwrap();
    let aftermath = tokio::time::timeout(
        PATIENCE,
        drop_a_transaction_in_its_begin(
            &watch,
            (host, port),
            |proxy| format!("postgres://user:pass@{proxy}/app?sslmode=disable"),
            "SELECT count(*) FROM pg_stat_activity WHERE state LIKE 'idle in transaction%'",
        ),
    )
    .await
    .expect("the test did not finish in time");
    assert_eq!(aftermath, CLEAN);
}

/// As on `PostgreSQL`: sqlx's `MySQL` driver also counts a transaction only once its `BEGIN`
/// has answered.
#[cfg(feature = "mysql")]
#[tokio::test]
async fn mysql_a_transaction_dropped_in_its_begin_leaves_its_connection_clean() {
    let container = test_containers::mysql()
        .with_env_var("MYSQL_ROOT_PASSWORD", "root")
        .with_env_var("MYSQL_USER", "user")
        .with_env_var("MYSQL_PASSWORD", "pass")
        .with_env_var("MYSQL_DATABASE", "app")
        .start()
        .await
        .expect("start pinned MySQL");
    let host = container.get_host().await.unwrap().to_string();
    let port = container.get_host_port_ipv4(3306).await.unwrap();
    // Over TLS. Root may read `innodb_trx`. The user's first login lets the server cache its
    // password, so the pool can then log in through the proxy without TLS, which the proxy has
    // to read.
    let watch = mysql_login(&format!("mysql://root:root@{host}:{port}/app")).await;
    drop(mysql_login(&format!("mysql://user:pass@{host}:{port}/app")).await);
    let aftermath = tokio::time::timeout(
        PATIENCE,
        drop_a_transaction_in_its_begin(
            &watch,
            (host, port),
            |proxy| format!("mysql://user:pass@{proxy}/app?ssl-mode=disabled"),
            "SELECT count(*) FROM information_schema.innodb_trx \
             WHERE trx_mysql_thread_id <> CONNECTION_ID()",
        ),
    )
    .await
    .expect("the test did not finish in time");
    assert_eq!(aftermath, CLEAN);
}

/// Log in to a `MySQL` container that may still be starting.
#[cfg(feature = "mysql")]
async fn mysql_login(dsn: &str) -> DatabaseConnection {
    let deadline = tokio::time::Instant::now() + PATIENCE;
    loop {
        match sea_orm::Database::connect(dsn).await {
            Ok(conn) => return conn,
            Err(error) => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "MySQL did not accept a login: {error}"
                );
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
}

/// Records the `summary` of each `sqlx::query` event, the first words of each statement.
#[cfg(feature = "pg")]
#[derive(Clone, Default)]
struct Statements(Arc<std::sync::Mutex<Vec<String>>>);

#[cfg(feature = "pg")]
impl tracing::Subscriber for Statements {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        metadata.target() == "sqlx::query"
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        struct Summary(String);
        impl tracing::field::Visit for Summary {
            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                if field.name() == "summary" {
                    value.clone_into(&mut self.0);
                }
            }

            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "summary" {
                    format!("{value:?}")
                        .trim_matches('"')
                        .clone_into(&mut self.0);
                }
            }
        }
        let mut summary = Summary(String::new());
        event.record(&mut summary);
        self.0.lock().unwrap().push(summary.0);
    }

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}

#[cfg(feature = "pg")]
fn select_one<'a>(
    tx: &'a DbTx<'a>,
) -> Pin<Box<dyn Future<Output = Result<(), DbError>> + Send + 'a>> {
    Box::pin(async move {
        tx.tx.execute_unprepared("SELECT 1").await?;
        Ok(())
    })
}

/// A subscriber set for one future, as a test that counts statements sets it, sees the
/// statements of the transaction's begin (here its `SET TRANSACTION`), which run in the begin's
/// own task, and the statements in the transaction.
#[cfg(feature = "pg")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn postgres_a_subscriber_set_for_the_caller_sees_the_begins_statements() {
    use tracing::instrument::WithSubscriber as _;

    let container = test_containers::postgres()
        .with_env_var("POSTGRES_PASSWORD", "pass")
        .with_env_var("POSTGRES_USER", "user")
        .with_env_var("POSTGRES_DB", "app")
        .start()
        .await
        .expect("start pinned PostgreSQL");
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let db = connect_db(
        &format!("postgres://user:pass@{host}:{port}/app"),
        ConnectOpts::default(),
    )
    .await
    .unwrap();
    let statements = Statements::default();
    db.transaction_ref_mapped_with_config(
        TxConfig {
            isolation: Some(TxIsolationLevel::RepeatableRead),
            access_mode: Some(TxAccessMode::ReadOnly),
        },
        select_one,
    )
    .with_subscriber(tracing::Dispatch::new(statements.clone()))
    .await
    .unwrap();
    let seen = statements.0.lock().unwrap().clone();
    assert!(
        seen.iter()
            .any(|summary| summary.starts_with("SET TRANSACTION")),
        "{seen:?}"
    );
    assert!(seen.iter().any(|summary| summary == "SELECT 1"), "{seen:?}");
}
