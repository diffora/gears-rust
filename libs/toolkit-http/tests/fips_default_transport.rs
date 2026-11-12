#![cfg(feature = "fips")]

//! FIPS transport policy (issue #1934, relaxed afterwards): under
//! `--features fips` every non-testing [`HttpClientConfig`] preset and
//! [`TransportSecurity::default`] are [`TransportSecurity::TlsOnly`], so
//! plaintext cannot be picked up by accident. Selecting
//! [`TransportSecurity::AllowInsecureHttp`] explicitly is an opt-in:
//! [`HttpClientBuilder::build`] accepts it and logs a warning (tested in
//! `fips_plaintext_warning.rs`).
//!
//! Lives in its own integration-test binary: the lib's `cfg(test)` provider
//! auto-install does not apply here, so tests that build a client install the
//! FIPS provider themselves via [`install_fips_provider`].

use httpmock::prelude::*;
use toolkit_http::{HttpClient, HttpClientConfig, HttpError, TransportSecurity};

/// Install the platform FIPS provider; a second install is a benign `Err`.
fn install_fips_provider() {
    drop(toolkit_security::tls::configured_provider().install_default());
}

/// Acceptance criterion 1:
/// `HttpClientConfig::default().transport == TransportSecurity::TlsOnly`
/// under `--features fips`.
#[test]
fn default_preset_is_tls_only() {
    assert_eq!(
        HttpClientConfig::default().transport,
        TransportSecurity::TlsOnly,
        "default() must default to TlsOnly under --features fips"
    );
}

/// `TransportSecurity::default()` follows the presets, so downstream crates
/// that derive their own default from it (e.g. `toolkit-contract`'s
/// `require_tls`) get `TlsOnly` too.
#[test]
fn transport_security_default_is_tls_only() {
    assert_eq!(TransportSecurity::default(), TransportSecurity::TlsOnly);
}

/// Every non-testing preset must flip to `TlsOnly` so a caller picking up any
/// preset under FIPS gets a secure default.
#[test]
fn all_non_testing_presets_are_tls_only() {
    let presets: [(&str, TransportSecurity); 5] = [
        ("default", HttpClientConfig::default().transport),
        ("minimal", HttpClientConfig::minimal().transport),
        ("infra_default", HttpClientConfig::infra_default().transport),
        (
            "token_endpoint",
            HttpClientConfig::token_endpoint().transport,
        ),
        ("sse", HttpClientConfig::sse().transport),
    ];

    for (name, transport) in presets {
        assert_eq!(
            transport,
            TransportSecurity::TlsOnly,
            "preset {name}() must default to TlsOnly under --features fips"
        );
    }
}

/// `for_testing()` is the only preset that carries `AllowInsecureHttp`.
#[test]
fn for_testing_preset_field_keeps_allow_insecure_http() {
    assert_eq!(
        HttpClientConfig::for_testing().transport,
        TransportSecurity::AllowInsecureHttp,
        "for_testing() must keep AllowInsecureHttp for plaintext mock servers"
    );
}

/// Explicitly selecting `AllowInsecureHttp` is an opt-in: under
/// `--features fips` the client builds and sends a request over `http://`.
#[tokio::test]
async fn explicit_allow_insecure_http_sends_plaintext_request() {
    install_fips_provider();
    let server = MockServer::start_async().await;
    let mock = server
        .mock_async(|when, then| {
            when.method(GET).path("/plain");
            then.status(200);
        })
        .await;

    let client = HttpClient::builder()
        .transport(TransportSecurity::AllowInsecureHttp)
        .retry(None)
        .build()
        .expect("explicit AllowInsecureHttp must build under --features fips");
    let response = client
        .get(&format!("{}/plain", server.base_url()))
        .send()
        .await
        .expect("plaintext request must go through after the opt-in");

    assert_eq!(response.status(), http::StatusCode::OK);
    mock.assert_async().await;
}

/// Without the opt-in, an `http://` request is still refused before any
/// connection is attempted.
#[tokio::test]
async fn default_client_rejects_http_scheme() {
    install_fips_provider();
    let client = HttpClient::builder()
        .retry(None)
        .build()
        .expect("default client must build under --features fips");

    let Err(err) = client.get("http://127.0.0.1:9/").send().await else {
        panic!("http:// must be rejected by the default TlsOnly client");
    };
    assert!(
        matches!(err, HttpError::InvalidScheme { ref scheme, .. } if scheme == "http"),
        "expected HttpError::InvalidScheme for http, got {err:?}"
    );
}
