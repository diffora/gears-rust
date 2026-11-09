//! Test fixtures: a TLS server signed by a private CA and a minimal HTTP
//! forward proxy (CONNECT tunnels and absolute-form requests).
//!
//! Declared as `#[cfg(test)] mod common;` so clippy treats it as test code.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Once};

use rcgen::{BasicConstraints, CertificateParams, CertifiedIssuer, DnType, IsCa, KeyPair};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use toolkit_http::{HttpClient, HttpClientBuilder, HttpError};

pub fn install_crypto_provider() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        drop(toolkit_security::tls::configured_provider().install_default());
    });
}

pub struct Pki {
    pub ca_pem: String,
    leaf: CertificateDer<'static>,
    leaf_key: PrivateKeyDer<'static>,
}

pub fn private_pki(name: &str) -> Pki {
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, format!("{name} test CA"));
    let ca = CertifiedIssuer::self_signed(ca_params, KeyPair::generate().unwrap()).unwrap();

    let leaf_key = KeyPair::generate().unwrap();
    let leaf = CertificateParams::new(vec!["localhost".to_owned()])
        .unwrap()
        .signed_by(&leaf_key, &ca)
        .unwrap();

    Pki {
        ca_pem: ca.pem(),
        leaf: leaf.der().clone(),
        leaf_key: PrivatePkcs8KeyDer::from(leaf_key.serialize_der()).into(),
    }
}

impl Pki {
    /// Server side for `localhost`, HTTP/1.1 only.
    pub fn acceptor(&self) -> tokio_rustls::TlsAcceptor {
        self.acceptor_with(None, Vec::new())
    }

    /// Server side for `localhost` that asks for (but does not require) a
    /// client certificate issued by `client_ca`, and offers `alpn`.
    pub fn acceptor_with(
        &self,
        client_ca: Option<CertificateDer<'static>>,
        alpn: Vec<Vec<u8>>,
    ) -> tokio_rustls::TlsAcceptor {
        install_crypto_provider();
        let builder = rustls::ServerConfig::builder();
        let builder = match client_ca {
            None => builder.with_no_client_auth(),
            Some(ca) => {
                let mut roots = rustls::RootCertStore::empty();
                roots.add(ca).unwrap();
                let verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(roots))
                    .allow_unauthenticated()
                    .build()
                    .unwrap();
                builder.with_client_cert_verifier(verifier)
            }
        };
        let mut config = builder
            .with_single_cert(vec![self.leaf.clone()], self.leaf_key.clone_key())
            .unwrap();
        config.alpn_protocols = alpn;
        tokio_rustls::TlsAcceptor::from(Arc::new(config))
    }
}

/// TLS server for `localhost` that answers every request with `200 tls-ok`.
pub async fn spawn_tls_server(pki: &Pki) -> SocketAddr {
    spawn_tls_server_with(pki.acceptor()).await.0
}

/// [`spawn_tls_server`] with a given acceptor; also counts the handshakes in
/// which the client presented a certificate.
pub async fn spawn_tls_server_with(
    acceptor: tokio_rustls::TlsAcceptor,
) -> (SocketAddr, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let client_certs = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&client_certs);
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            let counter = Arc::clone(&counter);
            tokio::spawn(async move {
                // Handshake failures are the point of some tests.
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                if tls.get_ref().1.peer_certificates().is_some() {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
                if read_head(&mut tls).await.is_some() {
                    drop(tls
                        .write_all(
                            b"HTTP/1.1 200 OK\r\ncontent-length: 6\r\nconnection: close\r\n\r\ntls-ok",
                        )
                        .await);
                    drop(tls.shutdown().await);
                }
            });
        }
    });
    (addr, client_certs)
}

/// Read up to the end of the request head.
pub async fn read_head<S: AsyncReadExt + Unpin>(stream: &mut S) -> Option<String> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).await.ok()? == 0 || buf.len() > 16 * 1024 {
            return None;
        }
        buf.push(byte[0]);
    }
    String::from_utf8(buf).ok()
}

#[derive(Default)]
pub struct ProxyLog {
    pub tunnels: AtomicUsize,
    /// TLS handshakes (`https://` proxy) in which the client sent a certificate.
    pub client_certs: AtomicUsize,
    /// ALPN protocol of the last TLS handshake with the proxy.
    pub alpn: Mutex<Option<Vec<u8>>>,
    heads: Mutex<Vec<String>>,
}

impl ProxyLog {
    pub fn heads(&self) -> Vec<String> {
        self.heads.lock().unwrap().clone()
    }
}

/// How [`spawn_proxy`] answers.
#[derive(Clone, Default)]
pub struct ProxyBehavior {
    /// Require this `Proxy-Authorization` value; otherwise answer `407`.
    pub basic: Option<&'static str>,
    /// Refuse every `CONNECT` with this status line, e.g. `"403 Forbidden"`.
    pub refuse_connect: Option<&'static str>,
    /// Answer forwarded requests with a `302` to this URL instead of `200`.
    pub redirect_to: Option<String>,
    /// Serve the proxy over TLS (an `https://` proxy).
    pub tls: Option<tokio_rustls::TlsAcceptor>,
}

/// Minimal forward proxy. CONNECT is tunnelled to the target; any other
/// request is answered directly with `200 forwarded`.
pub async fn spawn_proxy(behavior: ProxyBehavior) -> (SocketAddr, Arc<ProxyLog>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let log = Arc::new(ProxyLog::default());
    let log_task = Arc::clone(&log);
    tokio::spawn(async move {
        while let Ok((client, _)) = listener.accept().await {
            let behavior = behavior.clone();
            let log = Arc::clone(&log_task);
            tokio::spawn(async move {
                match behavior.tls.clone() {
                    None => serve_proxy_conn(client, behavior, log).await,
                    Some(acceptor) => {
                        if let Ok(tls) = acceptor.accept(client).await {
                            let session = tls.get_ref().1;
                            if session.peer_certificates().is_some() {
                                log.client_certs.fetch_add(1, Ordering::SeqCst);
                            }
                            *log.alpn.lock().unwrap() = session.alpn_protocol().map(<[u8]>::to_vec);
                            serve_proxy_conn(tls, behavior, log).await;
                        }
                    }
                }
            });
        }
    });
    (addr, log)
}

async fn serve_proxy_conn<S>(mut client: S, behavior: ProxyBehavior, log: Arc<ProxyLog>)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let Some(head) = read_head(&mut client).await else {
        return;
    };
    log.heads.lock().unwrap().push(head.clone());
    if let Some(expected) = behavior.basic {
        let wanted = format!("proxy-authorization: {expected}").to_ascii_lowercase();
        if !head.to_ascii_lowercase().contains(&wanted) {
            reply(
                &mut client,
                "HTTP/1.1 407 Proxy Authentication Required\r\ncontent-length: 0\r\n\r\n",
            )
            .await;
            return;
        }
    }
    let request_line = head.lines().next().unwrap_or_default().to_owned();
    let Some(target) = request_line
        .strip_prefix("CONNECT ")
        .and_then(|rest| rest.split(' ').next())
    else {
        let response = match &behavior.redirect_to {
            Some(location) => format!(
                "HTTP/1.1 302 Found\r\nlocation: {location}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
            ),
            None => "HTTP/1.1 200 OK\r\ncontent-length: 9\r\nconnection: close\r\n\r\nforwarded"
                .to_owned(),
        };
        reply(&mut client, &response).await;
        return;
    };
    if let Some(status) = behavior.refuse_connect {
        reply(
            &mut client,
            &format!("HTTP/1.1 {status}\r\ncontent-length: 0\r\n\r\n"),
        )
        .await;
        return;
    }
    let Ok(mut upstream) = TcpStream::connect(target).await else {
        reply(&mut client, "HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
        return;
    };
    log.tunnels.fetch_add(1, Ordering::SeqCst);
    reply(&mut client, "HTTP/1.1 200 Connection established\r\n\r\n").await;
    drop(tokio::io::copy_bidirectional(&mut client, &mut upstream).await);
}

async fn reply<S: AsyncWrite + Unpin>(stream: &mut S, response: &str) {
    // The peer may already be gone; the test asserts on what it saw.
    drop(stream.write_all(response.as_bytes()).await);
}

pub fn https_client(builder: HttpClientBuilder) -> HttpClient {
    install_crypto_provider();
    builder.retry(None).deny_insecure_http().build().unwrap()
}

pub async fn get_text(client: &HttpClient, url: &str) -> Result<String, HttpError> {
    client.get(url).send().await?.text().await
}
