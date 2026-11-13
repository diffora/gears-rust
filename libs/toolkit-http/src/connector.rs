//! Connector with optional outbound-proxy routing.
//!
//! Sits under the destination `hyper_rustls::HttpsConnector`: for HTTPS
//! through a proxy it returns a `CONNECT` tunnel, and the destination TLS
//! handshake runs on top of it, in the same place as for direct connections.
//! The hop to the proxy is plain TCP for an `http://` proxy and TLS for an
//! `https://` one.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use http::Uri;
use http::header::{HeaderMap, HeaderValue, USER_AGENT};
use hyper::rt::{Read, ReadBufCursor, Write};
use hyper_rustls::{HttpsConnector, MaybeHttpsStream};
use hyper_util::client::legacy::connect::proxy::Tunnel;
use hyper_util::client::legacy::connect::{Connected, Connection, HttpConnector};
use hyper_util::client::proxy::matcher::{Intercept, Matcher};
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;
use tower::{BoxError, Service, ServiceExt};

use crate::config::{ExplicitProxy, OutboundProxy, redact_userinfo};
use crate::error::{HttpError, ProxyErrorKind};

/// Build the proxy matcher for `proxy`. `None` means no request is proxied.
///
/// `Matcher` silently ignores a proxy URL it cannot use and connects directly,
/// so every URL it will use, explicit or from the environment, is checked here.
/// `allow_http` is whether plain-HTTP destinations can be reached at all; if
/// not, the environment's `http` proxy is never used and so not checked.
pub fn build_matcher(
    proxy: &OutboundProxy,
    allow_http: bool,
) -> Result<Option<Arc<Matcher>>, HttpError> {
    let matcher = match proxy {
        OutboundProxy::None => return Ok(None),
        OutboundProxy::FromEnv => {
            let cgi = std::env::var_os("REQUEST_METHOD").is_some();
            match env_matcher(|name| std::env::var(name).ok(), cgi, allow_http)? {
                Some(matcher) => matcher,
                None => return Ok(None),
            }
        }
        OutboundProxy::Explicit(ExplicitProxy {
            http,
            https,
            no_proxy,
        }) => {
            let mut builder = Matcher::builder();
            // Like the environment: an HTTP proxy is unused when plain HTTP is
            // not allowed, so it is neither checked nor set.
            if let Some(url) = http.as_ref().filter(|_| allow_http) {
                validate_proxy_url(url)?;
                builder = builder.http(url.as_str());
            }
            if let Some(url) = https {
                validate_proxy_url(url)?;
                builder = builder.https(url.as_str());
            }
            if let Some(list) = no_proxy {
                builder = builder.no(list.as_str());
            }
            builder.build()
        }
    };
    Ok(Some(Arc::new(matcher)))
}

/// The matcher for the proxy environment variables, read once, with the
/// precedence of `Matcher::from_env`:
///
/// - of each upper/lower case pair, the upper case variable wins when it is
///   set, even to an empty value;
/// - an empty or unset `HTTPS_PROXY` / `HTTP_PROXY` falls back to `ALL_PROXY`;
/// - in a CGI process (`cgi`) nothing is proxied: there `HTTP_PROXY` can come
///   from a request header (httpoxy).
///
/// Only the URLs that will be used are validated, so an unused variable meant
/// for other tools (say a SOCKS `ALL_PROXY` shadowed by `HTTPS_PROXY`) does not
/// fail the build.
fn env_matcher(
    var: impl Fn(&str) -> Option<String>,
    cgi: bool,
    allow_http: bool,
) -> Result<Option<Matcher>, HttpError> {
    if cgi {
        return Ok(None);
    }
    let first = |names: [&str; 2]| names.iter().find_map(|name| var(name)).unwrap_or_default();
    let all = first(["ALL_PROXY", "all_proxy"]);
    let or_all = |url: String| if url.is_empty() { all.clone() } else { url };
    let https = or_all(first(["HTTPS_PROXY", "https_proxy"]));
    let http = if allow_http {
        or_all(first(["HTTP_PROXY", "http_proxy"]))
    } else {
        String::new()
    };
    if https.is_empty() && http.is_empty() {
        return Ok(None);
    }
    for url in [&https, &http] {
        if !url.is_empty() {
            validate_proxy_url(url)?;
        }
    }
    Ok(Some(
        Matcher::builder()
            .https(https)
            .http(http)
            .no(first(["NO_PROXY", "no_proxy"]))
            .build(),
    ))
}

fn validate_proxy_url(url: &str) -> Result<(), HttpError> {
    let invalid = |kind| HttpError::Proxy {
        kind,
        proxy: redact_userinfo(url),
    };
    // An unencoded `/`, `?` or `#` in the credentials ends the authority early,
    // so the URL would parse with the password taken for the host. An
    // unencoded `@` splits it at the wrong place: hyper-util takes the first
    // `@`, so part of the password would become the proxy's userinfo.
    let after_scheme = url.find("://").map_or(0, |i| i + 3);
    if let Some(at) = url[after_scheme..].rfind('@')
        && url[after_scheme..after_scheme + at].contains(['/', '?', '#', '@'])
    {
        return Err(invalid(ProxyErrorKind::InvalidUrl));
    }
    let uri: Uri = url
        .parse()
        .map_err(|_| invalid(ProxyErrorKind::InvalidUrl))?;
    match uri.scheme_str() {
        None | Some("http" | "https") => {}
        Some(_) => return Err(invalid(ProxyErrorKind::UnsupportedScheme)),
    }
    // `Matcher` would accept an empty host, or drop a port that is not a
    // `u16` (`Authority::port` is then `None`) and use the scheme's default.
    let Some(authority) = uri.authority() else {
        return Err(invalid(ProxyErrorKind::InvalidUrl));
    };
    let host = authority.host();
    let after_host = authority
        .as_str()
        .rsplit('@')
        .next()
        .and_then(|host_port| host_port.strip_prefix(host));
    let port_ok = match after_host {
        Some("") => true,
        Some(port) => port
            .strip_prefix(':')
            .is_some_and(|p| p.parse::<u16>().is_ok()),
        None => false,
    };
    if host.is_empty() || !port_ok {
        return Err(invalid(ProxyErrorKind::InvalidUrl));
    }
    Ok(())
}

/// Connects to the proxy: plain TCP for `http://`, TLS for `https://`.
type ProxyHop = HttpsConnector<HttpConnector>;

/// Connector that routes through the proxy chosen by the matcher.
#[derive(Clone)]
pub struct ProxyConnector {
    direct: HttpConnector,
    proxy: Option<ProxyRoute>,
}

#[derive(Clone)]
struct ProxyRoute {
    matcher: Arc<Matcher>,
    hop: ProxyHop,
    /// Sent with `CONNECT`; some proxies filter on it.
    user_agent: HeaderValue,
}

impl ProxyConnector {
    /// Connects straight to every destination, without a proxy.
    pub fn direct(http: HttpConnector) -> Self {
        Self {
            direct: http,
            proxy: None,
        }
    }

    /// `proxy_tls` is used only for the hop to an `https://` proxy. It must not
    /// carry the destination's client certificate: that identity is for the
    /// destination, not for a middlebox.
    pub fn with_proxy(
        http: HttpConnector,
        matcher: Arc<Matcher>,
        proxy_tls: rustls::ClientConfig,
        user_agent: HeaderValue,
    ) -> Self {
        let hop = HttpsConnector::from((http.clone(), proxy_tls));
        Self {
            direct: http,
            proxy: Some(ProxyRoute {
                matcher,
                hop,
                user_agent,
            }),
        }
    }
}

impl Service<Uri> for ProxyConnector {
    type Response = ProxyStream;
    type Error = BoxError;
    type Future = Pin<Box<dyn Future<Output = Result<ProxyStream, BoxError>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.direct.poll_ready(cx).map_err(Into::into)
    }

    // Each connect drives its own clone of the inner connectors with
    // `oneshot`, so each is polled ready before it is called.
    fn call(&mut self, dst: Uri) -> Self::Future {
        let route = self
            .proxy
            .as_ref()
            .and_then(|p| p.matcher.intercept(&dst).map(|i| (p.clone(), i)));
        let direct = self.direct.clone();
        Box::pin(async move {
            let Some((route, intercept)) = route else {
                let io = direct.oneshot(dst).await?;
                return Ok(ProxyStream::direct(MaybeHttpsStream::Http(io)));
            };
            let proxy_error = |kind| HttpError::Proxy {
                kind,
                proxy: proxy_label(intercept.uri()),
            };
            // Unreachable after `build_matcher` validation; kept as a guard.
            if !matches!(intercept.uri().scheme_str(), Some("http" | "https")) {
                return Err(proxy_error(ProxyErrorKind::UnsupportedScheme).into());
            }
            if dst.scheme_str() == Some("https") {
                let io = tunnel(&route, &intercept).oneshot(dst).await.map_err(|e| {
                    match tunnel_rejection(&e) {
                        Some(kind) => proxy_error(kind).into(),
                        None => BoxError::from(e),
                    }
                })?;
                Ok(ProxyStream::direct(io))
            } else {
                let io = route.hop.oneshot(intercept.uri().clone()).await?;
                Ok(ProxyStream::forwarding(io))
            }
        })
    }
}

/// The proxy as `scheme://host:port` for an error message, without
/// credentials even if the URL was mangled.
fn proxy_label(uri: &Uri) -> String {
    let authority = uri.authority().map_or("", |a| a.as_str());
    redact_userinfo(&format!(
        "{}://{authority}",
        uri.scheme_str().unwrap_or("http")
    ))
}

fn tunnel(route: &ProxyRoute, intercept: &Intercept) -> Tunnel<ProxyHop> {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, route.user_agent.clone());
    let tunnel = Tunnel::new(intercept.uri().clone(), route.hop.clone()).with_headers(headers);
    match intercept.basic_auth() {
        Some(auth) => tunnel.with_auth(auth.clone()),
        None => tunnel,
    }
}

/// A `CONNECT` the proxy answered with a refusal, as opposed to an I/O
/// failure.
///
/// hyper-util does not export its `TunnelError`, so the variant is read from
/// its `Display` text; `tests/proxy_and_ca.rs` covers both cases.
fn tunnel_rejection(err: &dyn std::error::Error) -> Option<ProxyErrorKind> {
    match err.to_string().as_str() {
        "tunnel error: proxy authorization required" => Some(ProxyErrorKind::AuthRequired),
        "tunnel error: unsuccessful" => Some(ProxyErrorKind::TunnelRejected),
        _ => None,
    }
}

/// Bounds a whole connect: TCP (to the destination or the proxy), `CONNECT`
/// and the TLS handshake.
#[derive(Clone)]
pub struct ConnectTimeout<C> {
    inner: C,
    timeout: Option<Duration>,
}

impl<C> ConnectTimeout<C> {
    /// Wraps `inner`; `None` leaves connects unbounded.
    pub fn new(inner: C, timeout: Option<Duration>) -> Self {
        Self { inner, timeout }
    }
}

impl<C> Service<Uri> for ConnectTimeout<C>
where
    C: Service<Uri>,
    C::Error: Into<BoxError>,
    C::Future: Send + 'static,
{
    type Response = C::Response;
    type Error = BoxError;
    type Future = Pin<Box<dyn Future<Output = Result<C::Response, BoxError>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, dst: Uri) -> Self::Future {
        let connecting = self.inner.call(dst);
        let Some(timeout) = self.timeout else {
            return Box::pin(async move { connecting.await.map_err(Into::into) });
        };
        Box::pin(async move {
            match tokio::time::timeout(timeout, connecting).await {
                Ok(result) => result.map_err(Into::into),
                Err(_) => Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!("connect timed out after {timeout:?}"),
                )) as BoxError),
            }
        })
    }
}

/// `Proxy-Authorization` value for a plain-HTTP request forwarded via a proxy.
///
/// HTTPS requests carry the credentials in the `CONNECT` request instead, and
/// must not send them to the destination.
pub fn forward_proxy_auth(matcher: &Matcher, dst: &Uri) -> Option<http::HeaderValue> {
    if dst.scheme_str() != Some("http") {
        return None;
    }
    matcher.intercept(dst)?.basic_auth().cloned()
}

pin_project_lite::pin_project! {
    /// Connection that tells hyper whether it is talking to a forwarding proxy,
    /// so requests are written in absolute form.
    pub struct ProxyStream {
        #[pin]
        io: MaybeHttpsStream<TokioIo<TcpStream>>,
        forwarding: bool,
    }
}

impl ProxyStream {
    fn direct(io: MaybeHttpsStream<TokioIo<TcpStream>>) -> Self {
        Self {
            io,
            forwarding: false,
        }
    }

    fn forwarding(io: MaybeHttpsStream<TokioIo<TcpStream>>) -> Self {
        Self {
            io,
            forwarding: true,
        }
    }
}

impl Connection for ProxyStream {
    fn connected(&self) -> Connected {
        self.io.connected().proxy(self.forwarding)
    }
}

impl Read for ProxyStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: ReadBufCursor<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.project().io.poll_read(cx, buf)
    }
}

impl Write for ProxyStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        self.project().io.poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        self.project().io.poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        self.project().io.poll_shutdown(cx)
    }

    fn is_write_vectored(&self) -> bool {
        self.io.is_write_vectored()
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<std::io::Result<usize>> {
        self.project().io.poll_write_vectored(cx, bufs)
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    fn explicit(url: &str) -> OutboundProxy {
        ExplicitProxy::all(url).into()
    }

    fn kind(result: Result<Option<Arc<Matcher>>, HttpError>) -> ProxyErrorKind {
        match result {
            Err(HttpError::Proxy { kind, .. }) => kind,
            other => panic!("expected a proxy error, got {other:?}"),
        }
    }

    #[test]
    fn explicit_http_proxy_is_ignored_without_plain_http() {
        let proxy: OutboundProxy = ExplicitProxy::default()
            .with_https("http://proxy.local:3128")
            .with_http("socks5://proxy.local:1080")
            .into();
        assert!(build_matcher(&proxy, false).is_ok());
        assert_eq!(
            kind(build_matcher(&proxy, true)),
            ProxyErrorKind::UnsupportedScheme
        );
    }

    #[test]
    fn explicit_proxy_rejects_unsupported_schemes() {
        for url in ["socks5://proxy.local:1080", "socks4a://proxy.local:1080"] {
            assert_eq!(
                kind(build_matcher(&explicit(url), true)),
                ProxyErrorKind::UnsupportedScheme,
                "{url}"
            );
        }
    }

    #[test]
    fn unencoded_reserved_characters_in_credentials_are_rejected() {
        for url in [
            "http://user:p?ss@proxy.local:3128",
            "http://user:p#ss@proxy.local:3128",
            "http://user:p/ss@proxy.local:3128",
        ] {
            let err = build_matcher(&explicit(url), true).unwrap_err();
            assert!(!err.to_string().contains("ss@"), "{err}");
            assert_eq!(kind(Err(err)), ProxyErrorKind::InvalidUrl, "{url}");
        }
    }

    #[test]
    fn invalid_proxy_error_redacts_credentials() {
        let err =
            build_matcher(&explicit("socks5://user:secret@proxy.local:1080"), true).unwrap_err();
        let text = err.to_string();
        assert!(!text.contains("secret"), "{text}");
        assert!(!text.contains("user"), "{text}");
    }

    #[test]
    fn https_proxy_is_accepted() {
        let matcher = build_matcher(&explicit("https://proxy.local:3128"), true)
            .unwrap()
            .unwrap();
        let dst: Uri = "https://example.com/".parse().unwrap();
        assert_eq!(
            matcher.intercept(&dst).unwrap().uri().scheme_str(),
            Some("https")
        );
    }

    #[test]
    fn host_forms_are_accepted() {
        for url in [
            "http://proxy.local",
            "http://proxy.local:3128",
            "http://user:p%40ss@proxy.local:3128",
            "http://10.0.0.1:3128",
            "http://[::1]:3128",
            "http://[::1]",
        ] {
            assert!(build_matcher(&explicit(url), true).is_ok(), "{url}");
        }
    }

    #[test]
    fn explicit_proxy_accepts_missing_scheme() {
        let matcher = build_matcher(&explicit("proxy.local:3128"), true)
            .unwrap()
            .unwrap();
        let dst: Uri = "https://example.com/".parse().unwrap();
        let intercept = matcher.intercept(&dst).unwrap();
        assert_eq!(intercept.uri().scheme_str(), Some("http"));
    }

    #[test]
    fn no_proxy_bypasses_listed_hosts() {
        let proxy: OutboundProxy = ExplicitProxy::all("http://proxy.local:3128")
            .with_no_proxy("internal.example, 10.0.0.0/8")
            .into();
        let matcher = build_matcher(&proxy, true).unwrap().unwrap();
        for bypassed in [
            "https://internal.example/",
            "https://api.internal.example/",
            "http://10.1.2.3/",
        ] {
            assert!(
                matcher.intercept(&bypassed.parse().unwrap()).is_none(),
                "{bypassed}"
            );
        }
        assert!(
            matcher
                .intercept(&"https://example.com/".parse().unwrap())
                .is_some()
        );
    }

    #[test]
    fn forward_proxy_auth_only_for_plain_http() {
        let matcher = build_matcher(&explicit("http://u:p@proxy.local:3128"), true)
            .unwrap()
            .unwrap();
        let plain: Uri = "http://example.com/".parse().unwrap();
        let tls: Uri = "https://example.com/".parse().unwrap();
        assert_eq!(forward_proxy_auth(&matcher, &plain).unwrap(), "Basic dTpw");
        assert!(forward_proxy_auth(&matcher, &tls).is_none());
    }

    #[test]
    fn none_has_no_matcher() {
        assert!(build_matcher(&OutboundProxy::None, true).unwrap().is_none());
    }

    fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let vars: Vec<(String, String)> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |name| vars.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
    }

    fn proxy_for(matcher: &Matcher, dst: &str) -> Option<String> {
        matcher
            .intercept(&dst.parse().unwrap())
            .map(|i| i.uri().authority().unwrap().to_string())
    }

    #[test]
    fn env_proxy_follows_matcher_precedence() {
        let matcher = env_matcher(
            env(&[
                ("HTTPS_PROXY", "http://upper:1"),
                ("https_proxy", "http://lower:2"),
                ("all_proxy", "http://all:3"),
                ("HTTP_PROXY", ""),
                ("http_proxy", "http://shadowed:4"),
            ]),
            false,
            true,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            proxy_for(&matcher, "https://example.com/").unwrap(),
            "upper:1"
        );
        // An empty `HTTP_PROXY` hides `http_proxy`; `ALL_PROXY` fills in.
        assert_eq!(proxy_for(&matcher, "http://example.com/").unwrap(), "all:3");
    }

    #[test]
    fn env_proxy_validates_only_the_urls_it_uses() {
        // A SOCKS `ALL_PROXY` meant for other tools, shadowed for HTTPS.
        let vars = env(&[
            ("ALL_PROXY", "socks5://all:1080"),
            ("HTTPS_PROXY", "http://proxy:3128"),
            ("HTTP_PROXY", "htp://typo:1"),
        ]);
        let matcher = env_matcher(&vars, false, false).unwrap().unwrap();
        assert_eq!(
            proxy_for(&matcher, "https://example.com/").unwrap(),
            "proxy:3128"
        );
        // With plain HTTP allowed, the typo in `HTTP_PROXY` is used and fails.
        assert_eq!(
            kind(env_matcher(&vars, false, true).map(|m| m.map(Arc::new))),
            ProxyErrorKind::UnsupportedScheme
        );
        // With plain HTTP allowed and no `HTTP_PROXY`, `ALL_PROXY` serves HTTP.
        let https_only = env(&[
            ("ALL_PROXY", "socks5://all:1080"),
            ("HTTPS_PROXY", "http://proxy:3128"),
        ]);
        assert!(env_matcher(&https_only, false, false).is_ok());
        assert_eq!(
            kind(env_matcher(&https_only, false, true).map(|m| m.map(Arc::new))),
            ProxyErrorKind::UnsupportedScheme
        );
        // Without `HTTPS_PROXY` the SOCKS `ALL_PROXY` is the one used.
        let socks = env(&[("ALL_PROXY", "socks5://all:1080")]);
        assert_eq!(
            kind(env_matcher(socks, false, false).map(|m| m.map(Arc::new))),
            ProxyErrorKind::UnsupportedScheme
        );
    }

    #[test]
    fn env_proxy_is_none_when_unset_or_under_cgi() {
        assert!(env_matcher(env(&[]), false, true).unwrap().is_none());
        assert!(
            env_matcher(env(&[("HTTPS_PROXY", "htp://typo:1")]), true, true)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn malformed_authority_is_rejected() {
        for url in [
            "http://user:p@ss@proxy.local:3128",
            "http://:3128",
            "http://proxy.local:99999",
            "http://proxy.local:",
        ] {
            let err = build_matcher(&explicit(url), true).unwrap_err();
            assert!(!err.to_string().contains("ss@"), "{err}");
            assert_eq!(kind(Err(err)), ProxyErrorKind::InvalidUrl, "{url}");
        }
    }

    #[test]
    fn proxy_label_never_shows_credentials() {
        let uri: Uri = "http://ss@proxy.local:3128/".parse().unwrap();
        assert_eq!(proxy_label(&uri), "http://***@proxy.local:3128");
        let uri: Uri = "https://proxy.local:3128/".parse().unwrap();
        assert_eq!(proxy_label(&uri), "https://proxy.local:3128");
    }
}
