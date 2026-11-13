//! `OutboundProxy::FromEnv`. The workspace forbids `unsafe`, so the process
//! environment cannot be changed in-process: each case re-runs one of this
//! binary's ignored `child_*` tests with the proxy variables set.

#[cfg(test)]
mod common;

use std::sync::atomic::Ordering;

use child::{CHILD_MARKER, run_child};
use common::{
    ProxyBehavior, get_text, https_client, install_crypto_provider, private_pki, spawn_proxy,
    spawn_tls_server,
};
use toolkit_http::{HttpClientBuilder, HttpError, OutboundProxy, ProxyErrorKind};

#[cfg(test)]
mod child {
    /// Proxy-related variables cleared before [`run_child`] sets its own.
    const PROXY_VARS: [&str; 9] = [
        "ALL_PROXY",
        "all_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "NO_PROXY",
        "no_proxy",
        "REQUEST_METHOD",
    ];

    /// Marker the child tests check, so a plain `--include-ignored` run skips them.
    pub const CHILD_MARKER: &str = "TOOLKIT_HTTP_TEST_CHILD";

    /// Run the ignored test `name` of the current test binary in a fresh process
    /// whose only proxy variables are `vars`; panics unless it ran and passed.
    pub async fn run_child(name: &str, vars: &[(&str, String)]) {
        let mut cmd = tokio::process::Command::new(std::env::current_exe().unwrap());
        cmd.args([name, "--exact", "--ignored", "--test-threads=1"]);
        for var in PROXY_VARS {
            cmd.env_remove(var);
        }
        cmd.env(CHILD_MARKER, "1");
        for (var, value) in vars {
            cmd.env(var, value);
        }
        let out = cmd.output().await.unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains("1 passed"),
            "child {name} failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

fn is_child() -> bool {
    // Without the marker this is a plain `--include-ignored` run: skip.
    std::env::var_os(CHILD_MARKER).is_some()
}

#[tokio::test]
#[ignore = "spawned by the other tests in this file with proxy variables set"]
async fn child_fetch_https() {
    if !is_child() {
        return;
    }
    let pki = private_pki("server");
    let server = spawn_tls_server(&pki).await;
    let client = https_client(
        HttpClientBuilder::new()
            .add_root_certificates_pem(pki.ca_pem.as_bytes())
            .unwrap()
            .outbound_proxy(OutboundProxy::FromEnv),
    );
    let body = get_text(&client, &format!("https://localhost:{}/", server.port()))
        .await
        .unwrap();
    assert_eq!(body, "tls-ok");
}

#[cfg(not(feature = "fips"))]
#[tokio::test]
#[ignore = "spawned by the other tests in this file with proxy variables set"]
async fn child_fetch_plain_http() {
    if !is_child() {
        return;
    }
    let client = HttpClientBuilder::with_config(toolkit_http::HttpClientConfig::for_testing())
        .outbound_proxy(OutboundProxy::FromEnv)
        .build()
        .unwrap();
    let body = get_text(&client, "http://upstream.invalid/").await.unwrap();
    assert_eq!(body, "forwarded");
}

#[test]
#[ignore = "spawned by the other tests in this file with proxy variables set"]
fn child_build_rejects_unsupported_proxy() {
    if !is_child() {
        return;
    }
    install_crypto_provider();
    let err = HttpClientBuilder::new()
        .outbound_proxy(OutboundProxy::FromEnv)
        .build()
        .err()
        .expect("unsupported env proxy must fail the build");
    assert!(
        matches!(
            err,
            HttpError::Proxy {
                kind: ProxyErrorKind::UnsupportedScheme,
                ..
            }
        ),
        "{err:?}"
    );
}

#[tokio::test]
async fn https_proxy_variable_is_used() {
    let (proxy, log) = spawn_proxy(ProxyBehavior::default()).await;
    run_child(
        "child_fetch_https",
        &[("HTTPS_PROXY", format!("http://127.0.0.1:{}", proxy.port()))],
    )
    .await;
    assert_eq!(log.tunnels.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lowercase_all_proxy_is_used() {
    let (proxy, log) = spawn_proxy(ProxyBehavior::default()).await;
    run_child(
        "child_fetch_https",
        &[("all_proxy", format!("127.0.0.1:{}", proxy.port()))],
    )
    .await;
    assert_eq!(log.tunnels.load(Ordering::SeqCst), 1);
}

#[cfg(not(feature = "fips"))]
#[tokio::test]
async fn http_proxy_variable_is_used_for_plain_http() {
    let (proxy, log) = spawn_proxy(ProxyBehavior::default()).await;
    run_child(
        "child_fetch_plain_http",
        &[("HTTP_PROXY", format!("http://127.0.0.1:{}", proxy.port()))],
    )
    .await;
    let heads = log.heads();
    assert!(
        heads[0].starts_with("GET http://upstream.invalid/ HTTP/1.1"),
        "{heads:?}"
    );
}

#[tokio::test]
async fn no_proxy_variable_bypasses_proxy() {
    let (proxy, log) = spawn_proxy(ProxyBehavior::default()).await;
    run_child(
        "child_fetch_https",
        &[
            ("HTTPS_PROXY", format!("http://127.0.0.1:{}", proxy.port())),
            ("NO_PROXY", "example.com, localhost".to_owned()),
        ],
    )
    .await;
    assert!(log.heads().is_empty(), "{:?}", log.heads());
}

/// httpoxy: in a CGI process `HTTP_PROXY` may come from a request header.
#[tokio::test]
async fn cgi_process_ignores_proxy_variables() {
    let (proxy, log) = spawn_proxy(ProxyBehavior::default()).await;
    let url = format!("http://127.0.0.1:{}", proxy.port());
    run_child(
        "child_fetch_https",
        &[
            ("HTTP_PROXY", url.clone()),
            ("HTTPS_PROXY", url.clone()),
            ("ALL_PROXY", url),
            ("REQUEST_METHOD", "GET".to_owned()),
        ],
    )
    .await;
    assert!(log.heads().is_empty(), "{:?}", log.heads());
}

/// Only the variables that are used are checked: under `TlsOnly`, a SOCKS
/// `ALL_PROXY` set for other tools and shadowed by `HTTPS_PROXY` does not fail
/// the build. (With plain HTTP allowed, it would also need `HTTP_PROXY`.)
#[tokio::test]
async fn shadowed_all_proxy_is_not_checked() {
    let (proxy, log) = spawn_proxy(ProxyBehavior::default()).await;
    run_child(
        "child_fetch_https",
        &[
            ("ALL_PROXY", "socks5://proxy.example:1080".to_owned()),
            ("HTTPS_PROXY", format!("http://127.0.0.1:{}", proxy.port())),
        ],
    )
    .await;
    assert_eq!(log.tunnels.load(Ordering::SeqCst), 1);
}

/// hyper-util would ignore these and connect directly; fail the build instead.
#[tokio::test]
async fn unsupported_or_malformed_env_proxy_fails_build() {
    for value in ["socks5://proxy.example:1080", "htp://typo.example:3128"] {
        run_child(
            "child_build_rejects_unsupported_proxy",
            &[("HTTPS_PROXY", value.to_owned())],
        )
        .await;
    }
}
