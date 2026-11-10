## Detailed design

### Dependency policy

`toolkit-auth` uses an in-house token watcher (`src/oauth2/token_watcher.rs`) for
background refresh with jitter and exponential backoff. This replaces the former
`aliri_tokens` dependency, eliminating the transitive `aliri_tokens` → `aliri` →
`ring 0.17` chain that blocked FIPS Phase A promotion.

`toolkit-auth` depends on `toolkit-http` for token endpoint HTTP calls. This is acceptable because `toolkit-http` is hyper-based and does not pull `reqwest`.

### Public API

Gears use a long-lived token capability:

```rust
#[derive(Clone)]
pub struct TokenProvider {
    inner: std::sync::Arc<TokenWatcher>,
}

impl TokenProvider {
    pub fn new(config: OAuthClientConfig) -> Result<Self, TokenError>;
    pub async fn get(&self) -> Result<SecretString, TokenError>;
    pub async fn invalidate(&self) -> Result<(), TokenError>;
    pub async fn shutdown(&self);
}
```

Rules:

- gears never see client credentials
- gears never store the token string beyond request construction
- token is returned only as a transient `SecretString`

### Token configuration

**Required** (the arguments of `OAuthClientConfig::new`):

- `endpoint` (`TokenEndpoint`): `Direct(url)`, or `Issuer(url)` for OIDC
  discovery
- `client_id`
- `credentials` (`ClientCredentials`): the client authentication method
  together with its credential
    - `SecretBasic(secret)`: `client_secret_basic`, `Authorization: Basic`
    - `SecretPost(secret)`: `client_secret_post`, secret in the form body
    - `PrivateKeyJwt(key)`: `private_key_jwt` (RFC 7523 §2.2, OIDC Core §9),
      no shared secret on the wire

**Optional** (defaults from `new`, overridden with struct update syntax):

- `scopes` (normalized once, stable order; default none)
- extra headers for vendor quirks
- refresh policy:
    - refresh offset (default 30 minutes)
    - jitter max (default 5 minutes)
    - minimum refresh period (default 10 seconds)
- safe default TTL if `expires_in` is missing
- `HttpClientConfig` override for the token endpoint client (defaults to `HttpClientConfig::token_endpoint()`)

### `private_key_jwt`

Every token request carries a new assertion, signed through the
`toolkit-security` crypto facade (`toolkit_security::jwt::encode`). With
`toolkit-security/fips` enabled, signing runs in the platform FIPS module
(AWS-LC FIPS, corecrypto, CNG); jsonwebtoken's signer is never used.

- Keys: unencrypted PKCS#8 PEM, P-256 (`ES256`) or RSA 2048–8192 bits
  (`RS256`, `PS256`). Keys are parsed and checked when `PrivateKeyJwt` is
  built, so a bad key fails at construction rather than on the first request.
- Claims: `iss` = `sub` = `client_id`, `aud`, `jti` (random UUID from the
  facade CSPRNG), `iat`, `exp` = `iat` + 60 s. Optional `kid` header.
- `aud` defaults to the resolved token endpoint, so it is the exact URL the
  request is sent to. With `TokenEndpoint::Issuer`, it is computed after OIDC
  discovery. `with_audience` overrides it for IdPs that expect their issuer.
- The request body carries `client_id`, `client_assertion_type` and
  `client_assertion`. It has no `client_secret` and no `Authorization`
  header.
- Assertions are not cached. Token caching and refresh are unchanged.

### Token source (toolkit-http based)

The token source (`OAuthTokenSource`) performs the OAuth2 client credentials
exchange using `toolkit-http::HttpClient` and returns a `FetchedToken` consumed
by the in-house token watcher.

The token source holds an `HttpClient` instance built with `HttpClientConfig::token_endpoint()`. This client provides:

- retry with exponential backoff and jitter (transport errors, timeout, 429)
- concurrency limiting (10 concurrent requests)
- 30s request timeout
- 1 MB response body limit
- OTel tracing (when `otel` feature is enabled)
- TLS-only transport (no accidental plaintext token requests)

**Responsibilities:**

- resolve token endpoint:
    - use `TokenEndpoint::Direct(url)` as is, or
    - OIDC discovery from `TokenEndpoint::Issuer(url)` using `/.well-known/openid-configuration`
- do client credentials grant:
    - `grant_type=client_credentials`
    - optional `scope` (space-separated)
    - `Basic` auth or form-based auth
    - uses `HttpClient::post(url).form(&fields)` for the token request
- attach extra headers (if configured) via `RequestBuilder::header()`
- parse response via `HttpResponse::json::<TokenResponse>()`:
    - `access_token` (required)
    - `expires_in` (optional)
    - `token_type` (optional)

**Behavior rules:**

- missing `expires_in` uses a safe default TTL (prevents hot loops)
- if `token_type` exists and is not `Bearer`, fail token acquisition
- HTTP error handling: `HttpClient` already retries on transport errors, timeouts, and 429; the token source maps non-2xx responses to `TokenError` using `HttpResponse::error_for_status()`
- `Retry-After` header parsing uses `toolkit_http::response::parse_retry_after()` when available in error context
- sanitize errors: never log or include secrets (`client_secret`, token value) in error text

### Token watcher

In-house implementation (`src/oauth2/token_watcher.rs`) with:

- `ArcSwap`-backed lock-free reads (`CachedToken`)
- `tokio::spawn` background refresh loop
- Random early jitter (via `rand`, non-crypto)
- Exponential backoff on consecutive errors
- Explicit shutdown via `CancellationToken` and a joined task; final drop cancels as a fallback
- Three-state token lifecycle: `Fresh` → `Stale` → `Expired`

### Invalidate semantics

- clear the cached credential under the publication mutex
- share the current or next acquisition attempt across invalidations and gets
- never restore an invalidated token after a failed replacement
- preserve backoff when the cache is already empty
- reuse successful OIDC discovery and the same worker

Construction is synchronous and performs no network I/O. Async `get()` starts
acquisition lazily and respects backoff. See the [lifecycle guide](../README.md#acquisition-and-lifecycle)
and [migration notes](../../../CHANGELOG.md#toolkit-auth-080-migration).

### Outbound auth layer (tower)

Implement a tower layer that injects the `Authorization` header.

**Behavior:**

- on each request:
    - await `TokenProvider::get()`
    - set `Authorization: Bearer <token>`
    - forward request
- configurable header name if a vendor requires a non-standard header

**Integration with `HttpClient`:** The auth layer is composed into the `HttpClient` tower stack at build time using a decoupled architecture:

- `toolkit-http` provides a generic `HttpClientBuilder::with_auth_layer()` hook that accepts any tower layer. This keeps toolkit-http free of OAuth-specific knowledge and avoids circular dependencies.
- `toolkit-auth` provides an extension trait `HttpClientBuilderExt` with `.with_bearer_auth(token)` and `.with_bearer_auth_header(token, header_name)` methods. These wrap `BearerAuthLayer` and call `with_auth_layer()` internally.

The auth layer is inserted between retry and timeout in the tower stack, so each retry re-acquires the token.

### Integration rules

- Token endpoint calls use a dedicated `HttpClient` built with `HttpClientConfig::token_endpoint()`. This client is internal to the token source and is not shared with gear business logic.
- Gear outbound requests use `HttpClient` with the auth tower layer composed in. Tracing (OTel), retries, concurrency limits, and other middleware are already part of the `HttpClient` stack.
- No migration or coexistence concerns — `TracedClient` has been removed; all outbound HTTP uses `toolkit-http::HttpClient`.
