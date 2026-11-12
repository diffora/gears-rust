# toolkit-http

HTTP client library for ToolKit, built on hyper and tower.

## What it does

- HTTPS client with TLS via rustls (WebPki roots by default, no OS dependency)
- Connection pooling via hyper
- Configurable request timeouts
- Automatic retries with exponential backoff (429, 5xx, transport errors, timeouts)
- User-Agent header injection
- Concurrency limiting with fail-fast load shedding
- Response body size limits (applied after decompression)
- **Transparent response decompression** (gzip, brotli, deflate)
- **Secure redirect following** with SSRF protection and credential leakage prevention

## What it does NOT do

- Cookie management
- Request body compression
- Caching
- WebSocket support
- Streaming uploads

## Usage

### Basic GET request

```rust
use toolkit_http::HttpClient;
use std::time::Duration;

let client = HttpClient::builder()
    .timeout(Duration::from_secs(10))
    .user_agent("my-app/1.0")
    .build()?;

// RequestBuilder API: chain methods then send
let data: MyResponse = client
    .get("https://api.example.com/data")
    .send()
    .await?
    .json()
    .await?;
```

### POST with form body

```rust
use toolkit_http::HttpClient;
use std::time::Duration;

let client = HttpClient::builder()
    .timeout(Duration::from_secs(30))
    .build()?;

let token: TokenResponse = client
    .post("https://auth.example.com/token")
    .header("authorization", "Basic dXNlcjpwYXNz")
    .form(&[
        ("grant_type", "client_credentials"),
        ("client_id", "my-client"),
    ])?
    .send()
    .await?
    .json()
    .await?;
```

## Transparent Decompression

The client automatically handles compressed responses:

- **Sends `Accept-Encoding: gzip, br, deflate`** on all requests
- **Decompresses response bodies** based on `Content-Encoding` header
- **Body size limit applies to decompressed bytes**, protecting against "zip bombs"

This is enabled by default with no configuration required. Example:

```rust
// Server sends gzip-compressed JSON with Content-Encoding: gzip
// Client automatically decompresses before parsing
let data: MyResponse = client
    .get("https://api.example.com/data")
    .send()
    .await?
    .json()
    .await?;
```

## Configuration

| Setting | Default | Description |
|---------|---------|-------------|
| `timeout` | 30s | Request timeout |
| `max_body_size` | 10 MB | Maximum response body size (after decompression) |
| `user_agent` | `toolkit-http/1.0` | User-Agent header value |
| `retry` | Enabled (3 retries) | Retry on 429, 5xx (for idempotent methods), transport errors |
| `rate_limit` | 100 concurrent | Maximum concurrent requests |
| `redirect` | Same-origin, 10 max | Redirect policy with security controls (see below) |
| `pool_idle_timeout` | 90s | Idle connection timeout (None to disable) |
| `pool_max_idle_per_host` | 32 | Max idle connections per host |
| `transport` | `TlsOnly` with `fips`, `AllowInsecureHttp` otherwise | Transport security mode; `deny_insecure_http()` enforces TLS. Under `fips`, selecting `AllowInsecureHttp` is an explicit opt-in and `build()` logs a warning |
| `tls_roots` | WebPki | TLS root certificate source |
| `extra_root_certs` | none | Extra trust anchors added to `tls_roots` |
| `connect_timeout` | none | Bound on DNS + TCP connect + proxy `CONNECT` + TLS handshake |
| `outbound_proxy` | `None` | Forward proxy: `None`, `FromEnv` or `Explicit` |

### Configuration presets

```rust
use toolkit_http::{HttpClientBuilder, HttpClientConfig};

// Minimal: no retry, no rate limit, 10s timeout
let client = HttpClientBuilder::with_config(HttpClientConfig::minimal()).build()?;

// Infrastructure: aggressive retry, 60s timeout, 50MB body limit
let client = HttpClientBuilder::with_config(HttpClientConfig::infra_default()).build()?;

// Token endpoint: conservative rate limit, no retry on POST
let client = HttpClientBuilder::with_config(HttpClientConfig::token_endpoint()).build()?;

// Testing: allows plain HTTP for mock servers
let client = HttpClientBuilder::with_config(HttpClientConfig::for_testing()).build()?;

// SSE streaming: 24h timeout, no retry, no rate limit
let client = HttpClientBuilder::with_config(HttpClientConfig::sse()).build()?;
```

## Private CA

Extra trust anchors are added to the store chosen by `tls_roots`; nothing in
the OS store changes. `add_root_certificates_pem` fails on a bundle that is
not PEM or holds no certificate; a certificate that is not valid DER fails
`build()`. With `TlsRootConfig::Native`, an empty or unreadable OS store is
an error only when no extra roots are given; otherwise the extra roots alone
are trusted.

```rust
use toolkit_http::HttpClient;

let pem = std::fs::read("/etc/ssl/private-ca.pem")?;
let client = HttpClient::builder()
    .add_root_certificates_pem(&pem)?
    .build()?;
```

`WebPki` (the default) ships Mozilla's roots and ignores the OS store. Code
moving from `reqwest`, which verifies through the OS, should use the OS store
to keep trusting OS-installed roots. `reqwest` also follows the proxy
environment variables by default; set `OutboundProxy::FromEnv` to keep that
(OS proxy settings on macOS and Windows are not read):

```rust
use toolkit_http::{HttpClient, OutboundProxy, TlsRootConfig};

let client = HttpClient::builder()
    .tls_roots(TlsRootConfig::Native)
    .outbound_proxy(OutboundProxy::FromEnv)
    .build()?;
```

## Outbound proxy

```rust
use toolkit_http::{ExplicitProxy, HttpClient, OutboundProxy};

// HTTPS_PROXY / HTTP_PROXY / ALL_PROXY / NO_PROXY, upper or lower case
let client = HttpClient::builder()
    .outbound_proxy(OutboundProxy::FromEnv)
    .build()?;

let client = HttpClient::builder()
    .outbound_proxy(
        ExplicitProxy::all("http://user:pass@proxy.internal:3128")
            .with_no_proxy("localhost, .svc.cluster.local, 10.0.0.0/8"),
    )
    .build()?;
```

- HTTPS goes through a `CONNECT` tunnel. TLS runs end to end with the
  destination, so `tls_roots`, extra roots and FIPS hardening still apply.
- `https://` proxies get TLS on the proxy hop too, verified with the same
  roots (add the proxy's CA with `add_root_certificates_pem` if private). The
  client certificate from `client_auth` is never sent to the proxy.
- `CONNECT` carries the client's `User-Agent`.
- Plain HTTP (only with `AllowInsecureHttp`) is forwarded in absolute form.
  The proxy's own replies to it (a `407`, a `502`) come back as ordinary
  responses, not as `HttpError::Proxy`.
- Credentials in the proxy URL (percent-encode reserved characters, `@`
  included) are sent
  as `Basic` `Proxy-Authorization` to the proxy only. With an `http://` proxy
  they travel in cleartext, even under `TlsOnly`/FIPS; use an `https://`
  proxy to avoid that. They are redacted in `Debug` output and errors.
- A `407` on `CONNECT` (HTTPS destinations) fails with `HttpError::Proxy`
  (`AuthRequired`) and is not retried:
  repeated bad logins can lock the proxy account. Any other refusal of
  `CONNECT` (`TunnelRejected`, e.g. a 403 or a 503) is retried like a
  transport error, since a 5xx from a proxy is often temporary.
- `FromEnv` reads the environment once, at `build()`:
  - `HTTPS_PROXY` for HTTPS destinations, `HTTP_PROXY` for plain HTTP, and
    `ALL_PROXY` for either when its own variable is unset or empty;
    `NO_PROXY` lists hosts to reach directly.
  - Of each pair, the upper case variable wins when it is set, even to an
    empty value. (curl does the opposite: it prefers lower case.)
  - Only the variables that will be used are checked. `HTTP_PROXY` is not
    used under `TlsOnly`. A SOCKS `ALL_PROXY` does not fail the build when
    every scheme in use has its own variable: `HTTPS_PROXY` under `TlsOnly`,
    both `HTTPS_PROXY` and `HTTP_PROXY` when plain HTTP is allowed.
  - In a CGI process (`REQUEST_METHOD` set) the variables are ignored
    (httpoxy).
- Not supported: SOCKS, OS proxy settings, auth schemes other than `Basic`.
  A SOCKS proxy or a malformed proxy URL (bad host or port, unencoded
  reserved characters in credentials) fails `build()`, also when it comes
  from the environment.

## Redirect Security

By default, `toolkit-http` follows redirects with security protections:

| Protection | Default | Description                                                 |
|------------|---------|-------------------------------------------------------------|
| Same-origin only | Yes | Follows redirects to the same host                          |
| Header stripping | Yes | Removes `Authorization`, `Cookie` on cross-origin redirects |
| HTTPS downgrade | Blocked | Blocks redirects from HTTPS to HTTP                         |
| Max redirects | 10 | Stops after 10 redirects                                    |

### Redirect policies

```rust
use toolkit_http::{HttpClient, RedirectConfig};

// Default: same-origin only (most secure)
let client = HttpClient::builder().build()?;

// Permissive: follow all redirects, but strip credentials on cross-origin
let client = HttpClient::builder()
    .redirect(RedirectConfig::permissive())
    .build()?;

// Disable redirects entirely
let client = HttpClient::builder()
    .redirect(RedirectConfig::disabled())
    .build()?;

// Custom: allow specific trusted hosts
use std::collections::HashSet;
let config = RedirectConfig {
    same_origin_only: true,
    allowed_redirect_hosts: HashSet::from(["cdn.example.com".to_string()]),
    ..Default::default()
};
let client = HttpClient::builder().redirect(config).build()?;
```

## Retry Behavior

The default retry policy:

| Trigger | Retried for | Notes |
|---------|-------------|-------|
| 429 Too Many Requests | All methods | Server explicitly requests retry |
| 408, 500, 502, 503, 504 | Idempotent methods (GET, HEAD, PUT, DELETE, OPTIONS, TRACE) | Or with `Idempotency-Key` header |
| Transport errors | Idempotent methods | Connection failures, resets |
| Timeouts | Idempotent methods | Per-attempt timeout exceeded |

Non-idempotent methods (POST, PATCH) are only retried on 429 by default to avoid duplicate side effects. To enable broader retry for these methods, include an `Idempotency-Key` header.

## Error handling

All operations return `HttpError`:

| Variant | Retryable | Description |
|---------|-----------|-------------|
| `TimeoutAttempt` | Yes | Single request attempt exceeded timeout |
| `DeadlineExceeded` | No | Total operation deadline exceeded (all retries) |
| `Transport` | Yes | Network/connection error |
| `HttpStatus` | No | Non-2xx HTTP response (from `error_for_status()` or `json()`) |
| `Json` | No | JSON parsing failed |
| `BodyTooLarge` | No | Response exceeded size limit |
| `Tls` | No | TLS certificate/setup error |
| `Overloaded` | No | Concurrency limit reached (fail-fast) |
| `ServiceClosed` | No | Internal service failure (buffer worker died) |
| `Proxy` | Depends on `kind` | `InvalidUrl`, `UnsupportedScheme`, `AuthRequired` (407): no. `TunnelRejected` (other `CONNECT` refusals): yes, like `Transport` |

## License

Apache-2.0
