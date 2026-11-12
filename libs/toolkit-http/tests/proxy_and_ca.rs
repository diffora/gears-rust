//! Outbound proxy and extra trust anchors against real sockets: a TLS server
//! with a private CA and a minimal HTTP proxy (CONNECT + absolute-form).

#[cfg(test)]
mod common;

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use common::{
    ProxyBehavior, get_text, https_client, install_crypto_provider, private_pki, spawn_proxy,
    spawn_tls_server, spawn_tls_server_with,
};
use mtls_h2::{client_identity, spawn_h2_server};
use stall::{caused_by_io, spawn_silent_server};
use toolkit_http::{ExplicitProxy, HttpClientBuilder, HttpError, ProxyErrorKind, RetryConfig};

#[cfg(test)]
mod mtls_h2 {
    use std::net::SocketAddr;

    use bytes::Bytes;
    use http_body_util::Full;
    use hyper_util::rt::{TokioExecutor, TokioIo};
    use rcgen::{BasicConstraints, CertificateParams, CertifiedIssuer, IsCa, KeyPair};
    use rustls_pki_types::CertificateDer;
    use tokio::net::TcpListener;
    use toolkit_http::ClientAuthConfig;

    use crate::common::Pki;

    /// A client certificate and the CA that issued it. The PEM files live as
    /// long as `_dir`.
    pub struct ClientIdentity {
        pub config: ClientAuthConfig,
        pub ca: CertificateDer<'static>,
        _dir: tempfile::TempDir,
    }

    pub fn client_identity() -> ClientIdentity {
        let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let ca = CertifiedIssuer::self_signed(ca_params, KeyPair::generate().unwrap()).unwrap();
        let key = KeyPair::generate().unwrap();
        let cert = CertificateParams::new(vec!["client".to_owned()])
            .unwrap()
            .signed_by(&key, &ca)
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let cert_path = dir.path().join("client.pem");
        let key_path = dir.path().join("client.key");
        std::fs::write(&cert_path, cert.pem()).unwrap();
        std::fs::write(&key_path, key.serialize_pem()).unwrap();
        ClientIdentity {
            config: ClientAuthConfig::new(cert_path, key_path),
            ca: ca.der().clone(),
            _dir: dir,
        }
    }

    /// HTTP/2-only TLS server for `localhost` answering `200 h2-ok`.
    pub async fn spawn_h2_server(pki: &Pki) -> SocketAddr {
        let acceptor = pki.acceptor_with(None, vec![b"h2".to_vec()]);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    let Ok(tls) = acceptor.accept(tcp).await else {
                        return;
                    };
                    let service = hyper::service::service_fn(|_req| async {
                        Ok::<_, std::convert::Infallible>(hyper::Response::new(Full::new(
                            Bytes::from_static(b"h2-ok"),
                        )))
                    });
                    drop(
                        hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                            .serve_connection(TokioIo::new(tls), service)
                            .await,
                    );
                });
            }
        });
        addr
    }
}

#[cfg(test)]
mod stall {
    use std::net::SocketAddr;

    use tokio::net::TcpListener;
    use toolkit_http::HttpError;

    /// Accepts TCP connections and never writes: stalls a TLS handshake or a
    /// `CONNECT`. Connections stay open until the test ends.
    pub async fn spawn_silent_server() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((conn, _)) = listener.accept().await {
                held.push(conn);
            }
        });
        addr
    }

    /// Whether `err` was caused by an I/O error of `kind`, looking inside
    /// `io::Error` too (its `source()` skips the wrapped error).
    pub fn caused_by_io(err: &HttpError, kind: std::io::ErrorKind) -> bool {
        let mut source: Option<&(dyn std::error::Error + 'static)> = Some(err);
        while let Some(e) = source {
            if let Some(io) = e.downcast_ref::<std::io::Error>() {
                if io.kind() == kind {
                    return true;
                }
                if let Some(inner) = io.get_ref() {
                    source = Some(inner as &(dyn std::error::Error + 'static));
                    continue;
                }
            }
            source = e.source();
        }
        false
    }
}

/// "user:secret"
const GOOD_BASIC: &str = "Basic dXNlcjpzZWNyZXQ=";

/// The failure is the server certificate, not some other transport error.
fn assert_unknown_issuer(err: &HttpError) {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(e) = source {
        if let Some(rustls::Error::InvalidCertificate(cert_err)) = e.downcast_ref::<rustls::Error>()
        {
            assert_eq!(
                *cert_err,
                rustls::CertificateError::UnknownIssuer,
                "{err:?}"
            );
            return;
        }
        // `io::Error::source()` skips the wrapped error itself.
        source = match e
            .downcast_ref::<std::io::Error>()
            .and_then(std::io::Error::get_ref)
        {
            Some(inner) => Some(inner as &(dyn std::error::Error + 'static)),
            None => e.source(),
        };
    }
    panic!("no rustls certificate error in the chain: {err:?}");
}

fn proxy_kind(err: &HttpError) -> ProxyErrorKind {
    match err {
        HttpError::Proxy { kind, .. } => *kind,
        other => panic!("expected HttpError::Proxy, got {other:?}"),
    }
}

#[tokio::test]
async fn private_ca_is_rejected_without_extra_root() {
    let pki = private_pki("server");
    let addr = spawn_tls_server(&pki).await;
    let client = https_client(HttpClientBuilder::new());

    let err = get_text(&client, &format!("https://localhost:{}/", addr.port()))
        .await
        .unwrap_err();
    assert_unknown_issuer(&err);
}

#[tokio::test]
async fn private_ca_is_trusted_with_extra_root() {
    let pki = private_pki("server");
    let addr = spawn_tls_server(&pki).await;
    let client = https_client(
        HttpClientBuilder::new()
            .add_root_certificates_pem(pki.ca_pem.as_bytes())
            .unwrap(),
    );

    let body = get_text(&client, &format!("https://localhost:{}/", addr.port()))
        .await
        .unwrap();
    assert_eq!(body, "tls-ok");
}

#[tokio::test]
async fn unrelated_extra_root_does_not_help() {
    let pki = private_pki("server");
    let other = private_pki("other");
    let addr = spawn_tls_server(&pki).await;
    let client = https_client(
        HttpClientBuilder::new()
            .add_root_certificates_pem(other.ca_pem.as_bytes())
            .unwrap(),
    );

    let err = get_text(&client, &format!("https://localhost:{}/", addr.port()))
        .await
        .unwrap_err();
    assert_unknown_issuer(&err);
}

#[test]
fn empty_or_invalid_pem_bundle_is_rejected() {
    for pem in [
        &b""[..],
        b"not a pem",
        b"-----BEGIN CERTIFICATE-----\n!!!\n-----END CERTIFICATE-----\n",
    ] {
        let err = HttpClientBuilder::new()
            .add_root_certificates_pem(pem)
            .err()
            .expect("bundle must be rejected");
        assert!(matches!(err, HttpError::Tls(_)), "{err:?}");
    }
}

#[tokio::test]
async fn https_goes_through_connect_tunnel_with_credentials() {
    let pki = private_pki("server");
    let server = spawn_tls_server(&pki).await;
    let (proxy, log) = spawn_proxy(ProxyBehavior {
        basic: Some(GOOD_BASIC),
        ..ProxyBehavior::default()
    })
    .await;
    let client = https_client(
        HttpClientBuilder::new()
            .add_root_certificates_pem(pki.ca_pem.as_bytes())
            .unwrap()
            .outbound_proxy(ExplicitProxy::all(format!(
                "http://user:secret@127.0.0.1:{}",
                proxy.port()
            ))),
    );

    let body = get_text(&client, &format!("https://localhost:{}/", server.port()))
        .await
        .unwrap();
    assert_eq!(body, "tls-ok");
    assert_eq!(log.tunnels.load(Ordering::SeqCst), 1);
    let heads = log.heads();
    assert!(
        heads[0].starts_with(&format!("CONNECT localhost:{} HTTP/1.1", server.port())),
        "{heads:?}"
    );
}

/// An `https://` proxy: TLS to the proxy, then TLS to the destination inside
/// the tunnel, each verified against its own CA.
#[tokio::test]
async fn https_proxy_tunnels_https_with_credentials() {
    let server_pki = private_pki("server");
    let proxy_pki = private_pki("proxy");
    let server = spawn_tls_server(&server_pki).await;
    let (proxy, log) = spawn_proxy(ProxyBehavior {
        basic: Some(GOOD_BASIC),
        tls: Some(proxy_pki.acceptor()),
        ..ProxyBehavior::default()
    })
    .await;
    let client = https_client(
        HttpClientBuilder::new()
            .user_agent("proxy-test/1")
            .add_root_certificates_pem(server_pki.ca_pem.as_bytes())
            .unwrap()
            .add_root_certificates_pem(proxy_pki.ca_pem.as_bytes())
            .unwrap()
            .outbound_proxy(ExplicitProxy::all(format!(
                "https://user:secret@localhost:{}",
                proxy.port()
            ))),
    );

    let body = get_text(&client, &format!("https://localhost:{}/", server.port()))
        .await
        .unwrap();
    assert_eq!(body, "tls-ok");
    assert_eq!(log.tunnels.load(Ordering::SeqCst), 1);
    let head = log.heads()[0].to_ascii_lowercase();
    assert!(head.starts_with("connect localhost:"), "{head}");
    assert!(head.contains("user-agent: proxy-test/1"), "{head}");
}

/// The proxy's certificate is verified like any other server's.
#[tokio::test]
async fn untrusted_https_proxy_is_rejected() {
    let server_pki = private_pki("server");
    let proxy_pki = private_pki("proxy");
    let server = spawn_tls_server(&server_pki).await;
    let (proxy, log) = spawn_proxy(ProxyBehavior {
        tls: Some(proxy_pki.acceptor()),
        ..ProxyBehavior::default()
    })
    .await;
    let client = https_client(
        HttpClientBuilder::new()
            .add_root_certificates_pem(server_pki.ca_pem.as_bytes())
            .unwrap()
            .outbound_proxy(ExplicitProxy::all(format!(
                "https://localhost:{}",
                proxy.port()
            ))),
    );

    let err = get_text(&client, &format!("https://localhost:{}/", server.port()))
        .await
        .unwrap_err();
    assert_unknown_issuer(&err);
    assert!(log.heads().is_empty(), "{:?}", log.heads());
}

/// A 407 must not be retried: repeated bad logins can lock the proxy account.
#[tokio::test]
async fn wrong_proxy_credentials_fail_once_with_typed_error() {
    let pki = private_pki("server");
    let server = spawn_tls_server(&pki).await;
    let (proxy, log) = spawn_proxy(ProxyBehavior {
        basic: Some(GOOD_BASIC),
        ..ProxyBehavior::default()
    })
    .await;
    install_crypto_provider();
    // Default retry policy on purpose.
    let client = HttpClientBuilder::new()
        .add_root_certificates_pem(pki.ca_pem.as_bytes())
        .unwrap()
        .outbound_proxy(ExplicitProxy::all(format!(
            "http://user:wrong@127.0.0.1:{}",
            proxy.port()
        )))
        .build()
        .unwrap();

    let err = get_text(&client, &format!("https://localhost:{}/", server.port()))
        .await
        .unwrap_err();
    assert_eq!(proxy_kind(&err), ProxyErrorKind::AuthRequired, "{err:?}");
    assert!(!err.to_string().contains("wrong"), "{err}");
    let heads = log.heads();
    assert_eq!(heads.len(), 1, "{heads:?}");
    assert!(
        heads[0]
            .to_ascii_lowercase()
            .contains("proxy-authorization: basic"),
        "{heads:?}"
    );
    assert_eq!(log.tunnels.load(Ordering::SeqCst), 0);
}

/// A refused `CONNECT` is often a temporary 5xx from the proxy, so it is
/// retried like a transport error and ends with a typed error.
#[tokio::test]
async fn refused_connect_is_retried_with_typed_error() {
    let (proxy, log) = spawn_proxy(ProxyBehavior {
        refuse_connect: Some("403 Forbidden"),
        ..ProxyBehavior::default()
    })
    .await;
    install_crypto_provider();
    let client = HttpClientBuilder::new()
        .outbound_proxy(ExplicitProxy::all(format!(
            "http://127.0.0.1:{}",
            proxy.port()
        )))
        .build()
        .unwrap();

    let err = get_text(&client, "https://blocked.example/")
        .await
        .unwrap_err();
    assert_eq!(proxy_kind(&err), ProxyErrorKind::TunnelRejected, "{err:?}");
    assert_eq!(log.heads().len(), 1 + RetryConfig::default().max_retries);
}

/// The client certificate is for the destination; an `https://` proxy that
/// asks for one gets none.
#[tokio::test]
async fn client_certificate_goes_to_destination_not_to_https_proxy() {
    let identity = client_identity();
    let server_pki = private_pki("server");
    let proxy_pki = private_pki("proxy");
    let (server, server_client_certs) =
        spawn_tls_server_with(server_pki.acceptor_with(Some(identity.ca.clone()), Vec::new()))
            .await;
    let (proxy, log) = spawn_proxy(ProxyBehavior {
        tls: Some(proxy_pki.acceptor_with(Some(identity.ca.clone()), Vec::new())),
        ..ProxyBehavior::default()
    })
    .await;
    let client = https_client(
        HttpClientBuilder::new()
            .client_auth(identity.config.clone())
            .add_root_certificates_pem(server_pki.ca_pem.as_bytes())
            .unwrap()
            .add_root_certificates_pem(proxy_pki.ca_pem.as_bytes())
            .unwrap()
            .outbound_proxy(ExplicitProxy::all(format!(
                "https://localhost:{}",
                proxy.port()
            ))),
    );

    let body = get_text(&client, &format!("https://localhost:{}/", server.port()))
        .await
        .unwrap();
    assert_eq!(body, "tls-ok");
    assert_eq!(log.tunnels.load(Ordering::SeqCst), 1);
    assert_eq!(
        log.client_certs.load(Ordering::SeqCst),
        0,
        "sent to the proxy"
    );
    assert_eq!(
        server_client_certs.load(Ordering::SeqCst),
        1,
        "not sent to the destination"
    );
}

/// ALPN is per hop: HTTP/1.1 with an `https://` proxy that would take h2
/// (`CONNECT` is HTTP/1), h2 with the destination inside the tunnel.
#[tokio::test]
async fn https_proxy_hop_is_http1_and_destination_negotiates_h2() {
    let server_pki = private_pki("server");
    let proxy_pki = private_pki("proxy");
    let server = spawn_h2_server(&server_pki).await;
    let (proxy, log) = spawn_proxy(ProxyBehavior {
        tls: Some(proxy_pki.acceptor_with(None, vec![b"h2".to_vec(), b"http/1.1".to_vec()])),
        ..ProxyBehavior::default()
    })
    .await;
    let client = https_client(
        HttpClientBuilder::new()
            .add_root_certificates_pem(server_pki.ca_pem.as_bytes())
            .unwrap()
            .add_root_certificates_pem(proxy_pki.ca_pem.as_bytes())
            .unwrap()
            .outbound_proxy(ExplicitProxy::all(format!(
                "https://localhost:{}",
                proxy.port()
            ))),
    );

    let response = client
        .get(&format!("https://localhost:{}/", server.port()))
        .send()
        .await
        .unwrap();
    assert_eq!(response.into_inner().version(), http::Version::HTTP_2);
    assert_eq!(
        log.alpn.lock().unwrap().as_deref(),
        Some(&b"http/1.1"[..]),
        "proxy hop"
    );
}

#[tokio::test]
async fn no_proxy_host_connects_directly() {
    let pki = private_pki("server");
    let server = spawn_tls_server(&pki).await;
    let (proxy, log) = spawn_proxy(ProxyBehavior::default()).await;
    let client = https_client(
        HttpClientBuilder::new()
            .add_root_certificates_pem(pki.ca_pem.as_bytes())
            .unwrap()
            .outbound_proxy(
                ExplicitProxy::all(format!("http://127.0.0.1:{}", proxy.port()))
                    .with_no_proxy("localhost"),
            ),
    );

    let body = get_text(&client, &format!("https://localhost:{}/", server.port()))
        .await
        .unwrap();
    assert_eq!(body, "tls-ok");
    assert!(log.heads().is_empty(), "{:?}", log.heads());
}

#[test]
fn unsupported_proxy_url_is_rejected_at_build() {
    install_crypto_provider();
    for (url, kind) in [
        (
            "socks5://proxy.example:1080",
            ProxyErrorKind::UnsupportedScheme,
        ),
        (
            "http://user:p?ss@proxy.example:3128",
            ProxyErrorKind::InvalidUrl,
        ),
    ] {
        let err = HttpClientBuilder::new()
            .outbound_proxy(ExplicitProxy::all(url))
            .build()
            .err()
            .expect("proxy must be rejected");
        assert_eq!(proxy_kind(&err), kind, "{url}");
    }
}

/// `connect_timeout` covers the TLS handshake, not only the TCP connect.
#[tokio::test]
async fn connect_timeout_covers_tls_handshake() {
    let silent = spawn_silent_server().await;
    let client = https_client(
        HttpClientBuilder::new()
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_millis(200)),
    );

    let started = Instant::now();
    let err = get_text(&client, &format!("https://localhost:{}/", silent.port()))
        .await
        .unwrap_err();
    assert!(caused_by_io(&err, std::io::ErrorKind::TimedOut), "{err:?}");
    assert!(started.elapsed() < Duration::from_secs(5));
}

/// ... and a proxy that accepts the connection but never answers `CONNECT`.
#[tokio::test]
async fn connect_timeout_covers_proxy_connect() {
    let silent = spawn_silent_server().await;
    let client = https_client(
        HttpClientBuilder::new()
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_millis(200))
            .outbound_proxy(ExplicitProxy::all(format!(
                "http://127.0.0.1:{}",
                silent.port()
            ))),
    );

    let started = Instant::now();
    let err = get_text(&client, "https://unreachable.example/")
        .await
        .unwrap_err();
    assert!(caused_by_io(&err, std::io::ErrorKind::TimedOut), "{err:?}");
    assert!(started.elapsed() < Duration::from_secs(5));
}

// Plain HTTP needs `AllowInsecureHttp`, which FIPS builds reject.
#[cfg(not(feature = "fips"))]
mod plain_http {
    use super::*;
    use toolkit_http::HttpClientConfig;

    #[tokio::test]
    async fn forwarded_in_absolute_form_with_credentials() {
        let (proxy, log) = spawn_proxy(ProxyBehavior {
            basic: Some(GOOD_BASIC),
            ..ProxyBehavior::default()
        })
        .await;
        let client = HttpClientBuilder::with_config(HttpClientConfig::for_testing())
            .outbound_proxy(ExplicitProxy::all(format!(
                "http://user:secret@127.0.0.1:{}",
                proxy.port()
            )))
            .build()
            .unwrap();

        let body = get_text(&client, "http://upstream.invalid/path?q=1")
            .await
            .unwrap();
        assert_eq!(body, "forwarded");
        let heads = log.heads();
        assert!(
            heads[0].starts_with("GET http://upstream.invalid/path?q=1 HTTP/1.1"),
            "{heads:?}"
        );
        assert_eq!(log.tunnels.load(Ordering::SeqCst), 0);
    }

    /// Plain HTTP through an `https://` proxy: the credentials and the
    /// request travel inside TLS to the proxy.
    #[tokio::test]
    async fn forwarded_through_https_proxy() {
        let proxy_pki = private_pki("proxy");
        let (proxy, log) = spawn_proxy(ProxyBehavior {
            basic: Some(GOOD_BASIC),
            tls: Some(proxy_pki.acceptor()),
            ..ProxyBehavior::default()
        })
        .await;
        let client = HttpClientBuilder::with_config(HttpClientConfig::for_testing())
            .add_root_certificates_pem(proxy_pki.ca_pem.as_bytes())
            .unwrap()
            .outbound_proxy(ExplicitProxy::all(format!(
                "https://user:secret@localhost:{}",
                proxy.port()
            )))
            .build()
            .unwrap();

        let body = get_text(&client, "http://upstream.invalid/x")
            .await
            .unwrap();
        assert_eq!(body, "forwarded");
        assert!(
            log.heads()[0].starts_with("GET http://upstream.invalid/x HTTP/1.1"),
            "{:?}",
            log.heads()
        );
    }

    /// The proxy credentials belong to the proxy hop only.
    #[tokio::test]
    async fn direct_request_carries_no_proxy_credentials() {
        let (target, target_log) = spawn_proxy(ProxyBehavior::default()).await;
        let (proxy, _) = spawn_proxy(ProxyBehavior::default()).await;
        let client = HttpClientBuilder::with_config(HttpClientConfig::for_testing())
            .outbound_proxy(
                ExplicitProxy::all(format!("http://user:secret@127.0.0.1:{}", proxy.port()))
                    .with_no_proxy("127.0.0.1"),
            )
            .build()
            .unwrap();

        get_text(&client, &format!("http://127.0.0.1:{}/", target.port()))
            .await
            .unwrap();
        let head = target_log.heads()[0].to_ascii_lowercase();
        assert!(head.starts_with("get / http/1.1"), "{head}");
        assert!(!head.contains("proxy-authorization"), "{head}");
    }

    /// `Proxy-Authorization` is decided per redirect hop: a redirect to a
    /// bypassed host must not carry the credentials of the first hop.
    #[tokio::test]
    async fn redirect_to_bypassed_host_drops_proxy_credentials() {
        let (target, target_log) = spawn_proxy(ProxyBehavior::default()).await;
        let (proxy, proxy_log) = spawn_proxy(ProxyBehavior {
            basic: Some(GOOD_BASIC),
            redirect_to: Some(format!("http://127.0.0.1:{}/landing", target.port())),
            ..ProxyBehavior::default()
        })
        .await;
        let client = HttpClientBuilder::with_config(HttpClientConfig::for_testing())
            .outbound_proxy(
                ExplicitProxy::all(format!("http://user:secret@127.0.0.1:{}", proxy.port()))
                    .with_no_proxy("127.0.0.1"),
            )
            .build()
            .unwrap();

        let body = get_text(&client, "http://upstream.invalid/start")
            .await
            .unwrap();
        assert_eq!(body, "forwarded");
        assert!(
            proxy_log.heads()[0]
                .to_ascii_lowercase()
                .contains("proxy-authorization"),
            "{:?}",
            proxy_log.heads()
        );
        let head = target_log.heads()[0].to_ascii_lowercase();
        assert!(head.starts_with("get /landing http/1.1"), "{head}");
        assert!(!head.contains("proxy-authorization"), "{head}");
    }

    /// The reverse: a redirect from a bypassed host to a proxied one must
    /// get the proxy credentials, which the first hop did not carry.
    #[tokio::test]
    async fn redirect_from_bypassed_host_gets_proxy_credentials() {
        let (target, target_log) = spawn_proxy(ProxyBehavior {
            redirect_to: Some("http://upstream.invalid/next".to_owned()),
            ..ProxyBehavior::default()
        })
        .await;
        let (proxy, proxy_log) = spawn_proxy(ProxyBehavior {
            basic: Some(GOOD_BASIC),
            ..ProxyBehavior::default()
        })
        .await;
        let client = HttpClientBuilder::with_config(HttpClientConfig::for_testing())
            .outbound_proxy(
                ExplicitProxy::all(format!("http://user:secret@127.0.0.1:{}", proxy.port()))
                    .with_no_proxy("127.0.0.1"),
            )
            .build()
            .unwrap();

        let body = get_text(
            &client,
            &format!("http://127.0.0.1:{}/start", target.port()),
        )
        .await
        .unwrap();
        assert_eq!(body, "forwarded");
        let head = target_log.heads()[0].to_ascii_lowercase();
        assert!(!head.contains("proxy-authorization"), "{head}");
        let head = proxy_log.heads()[0].to_ascii_lowercase();
        assert!(
            head.starts_with("get http://upstream.invalid/next http/1.1"),
            "{head}"
        );
        assert!(head.contains("proxy-authorization"), "{head}");
    }
}
