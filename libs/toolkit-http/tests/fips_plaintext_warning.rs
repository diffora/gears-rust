#![cfg(feature = "fips")]

//! Under `--features fips`, the first `AllowInsecureHttp` build in the process
//! logs a warning on target `toolkit_http::security`.
//!
//! The warning is guarded by a process-wide `Once`, so this binary holds a
//! single test: another test building an `AllowInsecureHttp` client in the same
//! process could consume the warning first.

use std::sync::{Arc, Mutex, PoisonError};

use toolkit_http::{HttpClient, TransportSecurity};
use tracing::{Level, Subscriber};
use tracing_subscriber::Registry;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};

/// Collects `(level, target)` of every event.
#[derive(Clone, Default)]
struct EventLog(Arc<Mutex<Vec<(Level, String)>>>);

impl<S: Subscriber> Layer<S> for EventLog {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((*meta.level(), meta.target().to_owned()));
    }
}

impl EventLog {
    fn security_warnings(&self) -> usize {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(level, target)| *level == Level::WARN && target == "toolkit_http::security")
            .count()
    }
}

/// A `TlsOnly` build logs nothing; the first `AllowInsecureHttp` build warns;
/// later ones (a client rebuilt per operation) do not.
#[tokio::test]
async fn plaintext_opt_in_warns_once_per_process() {
    drop(toolkit_security::tls::configured_provider().install_default());
    let log = EventLog::default();
    let _guard = tracing::subscriber::set_default(Registry::default().with(log.clone()));

    let build = |transport| {
        HttpClient::builder()
            .transport(transport)
            .build()
            .expect("client must build under --features fips")
    };

    build(TransportSecurity::TlsOnly);
    assert_eq!(log.security_warnings(), 0, "TlsOnly must not warn");

    build(TransportSecurity::AllowInsecureHttp);
    assert_eq!(
        log.security_warnings(),
        1,
        "first plaintext build must warn"
    );

    build(TransportSecurity::AllowInsecureHttp);
    assert_eq!(
        log.security_warnings(),
        1,
        "later plaintext builds must not warn"
    );
}
