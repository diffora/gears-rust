//! OAGW upstream and route registration for configured LLM providers.
//!
//! Called once during `Gear::init()` to ensure every provider entry
//! has a corresponding OAGW upstream (with auth config) and route.
//!
//! After each successful `create_upstream`, the OAGW-assigned alias is
//! stamped onto [`ProviderEntry::upstream_alias`] (or
//! [`ProviderTenantOverride::upstream_alias`]) so the rest of mini-chat uses
//! the authoritative alias from OAGW rather than deriving one locally.

use std::collections::HashMap;
use std::sync::Arc;

use oagw_sdk::ServiceGatewayClientV1;
use toolkit_canonical_errors::CanonicalError;
use tracing::{info, warn};

use crate::config::ProviderEntry;

/// Register OAGW upstreams and routes for each configured provider.
///
/// On success the **OAGW-assigned alias** is written into
/// [`ProviderEntry::upstream_alias`] (root) and
/// [`ProviderTenantOverride::upstream_alias`] (per-tenant).
///
/// Returns the ids of providers whose upstream could **not** be registered
/// because their backend secret is not yet accessible (see
/// [`create_or_reuse_upstream`]). With the stateful credstore, provider
/// secrets are created at runtime via the credstore API and may not exist at
/// boot; such providers are deferred so the caller can retry once the secret
/// is provisioned (see [`reconcile_deferred_upstreams`]) rather than crashing
/// the gear or leaving the provider permanently unavailable.
///
/// The caller is responsible for obtaining a valid `SecurityContext`
/// (typically via S2S client credentials exchange).
pub async fn register_oagw_upstreams(
    gateway: &Arc<dyn ServiceGatewayClientV1>,
    ctx: &toolkit_security::SecurityContext,
    providers: &mut HashMap<String, ProviderEntry>,
) -> anyhow::Result<Vec<String>> {
    // Each provider is registered independently: a failure for one provider
    // (secret not ready, transient OAGW error, or misconfiguration) never
    // aborts registration of the others. A single persistently-failing
    // provider must not starve its healthy peers of registration.
    let mut deferred = Vec::new();
    for (provider_id, entry) in providers.iter_mut() {
        match register_provider(gateway, ctx, provider_id, entry).await {
            ProviderRegistration::Deferred => deferred.push(provider_id.clone()),
            // Registered: nothing to do. Misconfigured: already error-logged, and
            // retrying cannot fix a deterministic config error, so it is not
            // added to the retry set.
            ProviderRegistration::Registered | ProviderRegistration::Misconfigured => {}
        }
    }

    Ok(deferred)
}

/// Outcome of registering a single provider's OAGW upstream(s) + route.
enum ProviderRegistration {
    /// Root upstream, route, and every tenant-override upstream are registered.
    Registered,
    /// Could not complete — the backend secret is not accessible yet (with the
    /// stateful credstore it may be provisioned after boot) or a transient OAGW
    /// error occurred. The caller should retry (see [`reconcile_deferred_upstreams`]).
    Deferred,
    /// A deterministic misconfiguration that cannot self-heal (logged at error).
    /// The provider is left unavailable; retrying would not help.
    Misconfigured,
}

/// Register one provider's root upstream, route, and tenant-override upstreams
/// as an isolated unit. Never panics or propagates — the outcome is reported
/// so the batch loop can keep going for other providers.
///
/// `AlreadyExists` is treated as success throughout (the upstream/route survives
/// from a previous registration — restart / idempotent re-run / retry), so this
/// is safe to call repeatedly for the same provider.
// The per-step branches + tracing macros inflate the measured cognitive
// complexity; the control flow is a linear sequence of register-or-defer steps.
#[allow(clippy::cognitive_complexity)]
async fn register_provider(
    gateway: &Arc<dyn ServiceGatewayClientV1>,
    ctx: &toolkit_security::SecurityContext,
    provider_id: &str,
    entry: &mut ProviderEntry,
) -> ProviderRegistration {
    let Some(upstream) = create_or_reuse_upstream(gateway, ctx, provider_id, entry).await else {
        return ProviderRegistration::Deferred;
    };
    entry.upstream_alias = Some(upstream.alias.clone());

    if let Err(e) = register_route(gateway, ctx, provider_id, entry, &upstream).await {
        warn!(provider_id, error = %e, "OAGW route registration failed; deferring provider for retry");
        return ProviderRegistration::Deferred;
    }

    // Tenant-specific upstreams (share the same route/api_path as the root).
    let tenant_ids: Vec<String> = entry.tenant_overrides.keys().cloned().collect();
    for tenant_id in &tenant_ids {
        let tenant_override = &entry.tenant_overrides[tenant_id];
        if tenant_override.host.is_none() && tenant_override.upstream_alias.is_none() {
            // Deterministic config error — no host and no alias to derive a
            // distinct upstream from. Surface it (won't self-heal) and leave the
            // provider unavailable rather than crashing the whole gear at boot.
            tracing::error!(
                provider_id,
                tenant_id,
                "tenant override has no host and no upstream_alias; \
                 skipping provider (misconfiguration)"
            );
            return ProviderRegistration::Misconfigured;
        }

        match create_or_reuse_tenant_upstream(gateway, ctx, provider_id, entry, tenant_id).await {
            Ok(alias) => {
                if let Some(tenant_override) = entry.tenant_overrides.get_mut(tenant_id) {
                    tenant_override.upstream_alias = Some(alias);
                }
            }
            Err(e) => {
                warn!(provider_id, tenant_id, error = %e, "OAGW tenant upstream registration failed; deferring provider for retry");
                return ProviderRegistration::Deferred;
            }
        }
    }

    ProviderRegistration::Registered
}

/// Retry registration for the providers [`register_oagw_upstreams`] deferred.
///
/// Re-attempts registration for exactly `deferred` and returns the ids that
/// are **still** deferred (their secret is still not accessible). Registration
/// is idempotent — providers that succeeded on a previous attempt are reused
/// via `AlreadyExists` — so this is safe to call repeatedly.
pub async fn reconcile_deferred_upstreams(
    gateway: &Arc<dyn ServiceGatewayClientV1>,
    ctx: &toolkit_security::SecurityContext,
    providers: &HashMap<String, ProviderEntry>,
    deferred: &[String],
) -> anyhow::Result<Vec<String>> {
    let mut subset: HashMap<String, ProviderEntry> = deferred
        .iter()
        .filter_map(|id| providers.get(id).map(|e| (id.clone(), e.clone())))
        .collect();
    register_oagw_upstreams(gateway, ctx, &mut subset).await
}

/// Create the root upstream, or recover the existing one on `AlreadyExists`.
///
/// Returns `None` (with a warning log) when the provider must be skipped:
/// registration failed for a reason other than a survived registration
/// (typically its credstore secret is not provisioned yet).
async fn create_or_reuse_upstream(
    gateway: &Arc<dyn ServiceGatewayClientV1>,
    ctx: &toolkit_security::SecurityContext,
    provider_id: &str,
    entry: &ProviderEntry,
) -> Option<oagw_sdk::Upstream> {
    match create_upstream(gateway, ctx, provider_id, entry).await {
        Ok(u) => Some(u),
        Err(CanonicalError::AlreadyExists { resource_name, .. }) => {
            reuse_existing_upstream(gateway, ctx, provider_id, resource_name.as_deref()).await
        }
        Err(e) => {
            warn!(
                provider_id,
                error = %e,
                "skipping OAGW provisioning for provider: upstream registration failed \
                 (its credstore secret may not be accessible yet); the provider will be \
                 unavailable until its secret is provisioned"
            );
            None
        }
    }
}

/// Recover the upstream object behind an `AlreadyExists` conflict.
async fn reuse_existing_upstream(
    gateway: &Arc<dyn ServiceGatewayClientV1>,
    ctx: &toolkit_security::SecurityContext,
    provider_id: &str,
    alias: Option<&str>,
) -> Option<oagw_sdk::Upstream> {
    let Some(u) = find_upstream_by_alias(gateway, ctx, alias).await else {
        warn!(
            provider_id,
            "OAGW upstream already exists but could not be looked up by alias; \
             skipping provider"
        );
        return None;
    };
    info!(
        provider_id,
        alias = %u.alias,
        upstream_id = %u.id,
        "OAGW upstream already registered; reusing existing"
    );
    Some(u)
}

/// Create a tenant-override upstream, or reuse the taken alias on
/// `AlreadyExists` (a survived registration from a previous run).
async fn create_or_reuse_tenant_upstream(
    gateway: &Arc<dyn ServiceGatewayClientV1>,
    ctx: &toolkit_security::SecurityContext,
    provider_id: &str,
    entry: &ProviderEntry,
    tenant_id: &str,
) -> anyhow::Result<String> {
    let label = format!("{provider_id}[tenant={tenant_id}]");
    match create_tenant_upstream(gateway, ctx, &label, entry, tenant_id).await {
        Ok(alias) => Ok(alias),
        // The conflict carries the taken alias — exactly what we need to reuse.
        Err(CanonicalError::AlreadyExists {
            resource_name: Some(alias),
            ..
        }) => {
            info!(
                label,
                alias = %alias,
                "OAGW tenant upstream already registered; reusing existing alias"
            );
            Ok(alias)
        }
        Err(e) => anyhow::bail!(
            "OAGW tenant upstream registration failed for provider \
             '{provider_id}', tenant '{tenant_id}': {e}"
        ),
    }
}

/// Create an OAGW upstream for a single provider entry.
///
/// Only passes `upstream_alias` to OAGW when explicitly configured
/// (required for IP-based hosts). For hostname-based hosts OAGW
/// auto-derives the alias.
fn endpoint_for(entry: &ProviderEntry) -> oagw_sdk::Endpoint {
    use oagw_sdk::{Endpoint, Scheme};
    let scheme = if entry.use_http {
        Scheme::Http
    } else {
        Scheme::Https
    };
    let port = entry.port.unwrap_or(if entry.use_http { 80 } else { 443 });
    Endpoint {
        scheme,
        host: entry.host.clone(),
        port,
    }
}

/// Look up an existing upstream by alias (paging through `list_upstreams`).
///
/// Used to recover the upstream object when `create_upstream` reports
/// `AlreadyExists` — `alias` is the taken alias the conflict names.
async fn find_upstream_by_alias(
    gateway: &Arc<dyn ServiceGatewayClientV1>,
    ctx: &toolkit_security::SecurityContext,
    alias: Option<&str>,
) -> Option<oagw_sdk::Upstream> {
    let alias = alias?;
    let mut query = oagw_sdk::ListQuery::default();
    loop {
        let page = match gateway.list_upstreams(ctx.clone(), &query).await {
            Ok(page) => page,
            Err(e) => {
                warn!(alias, error = %e, "OAGW upstream lookup by alias failed");
                return None;
            }
        };
        let page_len = page.len();
        if let Some(u) = page.into_iter().find(|u| u.alias == alias) {
            return Some(u);
        }
        if page_len < query.top as usize {
            return None;
        }
        query.skip += query.top;
    }
}

async fn create_upstream(
    gateway: &Arc<dyn ServiceGatewayClientV1>,
    ctx: &toolkit_security::SecurityContext,
    provider_id: &str,
    entry: &ProviderEntry,
) -> Result<oagw_sdk::Upstream, CanonicalError> {
    use oagw_sdk::{AuthConfig, CreateUpstreamRequest, Server};

    let server = Server {
        endpoints: vec![endpoint_for(entry)],
    };

    let mut builder =
        CreateUpstreamRequest::builder(server, "gts.cf.core.oagw.protocol.v1~cf.core.oagw.http.v1")
            .enabled(true);

    // Only pass alias when explicitly configured (IP-based hosts).
    if let Some(alias) = &entry.upstream_alias {
        builder = builder.alias(alias);
    }

    if let (Some(plugin_type), Some(config)) = (&entry.auth_plugin_type, &entry.auth_config) {
        builder = builder.auth(AuthConfig {
            plugin_type: plugin_type.clone(),
            sharing: oagw_sdk::SharingMode::Inherit,
            config: Some(config.clone()),
        });
    }

    if let Some(headers) = crate::infra::llm::providers::upstream_headers_for_kind(entry.kind) {
        builder = builder.headers(headers);
    }

    let u = gateway
        .create_upstream(ctx.clone(), builder.build())
        .await?;
    info!(
        provider_id,
        alias = %u.alias,
        upstream_id = %u.id,
        "OAGW upstream registered"
    );
    Ok(u)
}

/// Create an OAGW upstream for a tenant-specific override.
///
/// Uses [`ProviderEntry::effective_host_for_tenant`] and the tenant's auth
/// config. Only passes `upstream_alias` when the tenant override explicitly
/// sets one (required for IP-based hosts). For hostname-based hosts OAGW
/// auto-derives the alias.
///
/// Returns the OAGW-assigned alias on success.
async fn create_tenant_upstream(
    gateway: &Arc<dyn ServiceGatewayClientV1>,
    ctx: &toolkit_security::SecurityContext,
    label: &str,
    entry: &ProviderEntry,
    tenant_id: &str,
) -> Result<String, CanonicalError> {
    use oagw_sdk::{AuthConfig, CreateUpstreamRequest, Server};

    let host = entry.effective_host_for_tenant(tenant_id);

    // Inherit scheme/port from root entry (tenant override only changes host/auth).
    let mut ep = endpoint_for(entry);
    host.clone_into(&mut ep.host);

    let server = Server {
        endpoints: vec![ep],
    };

    let mut builder =
        CreateUpstreamRequest::builder(server, "gts.cf.core.oagw.protocol.v1~cf.core.oagw.http.v1")
            .enabled(true);

    // Only pass alias when the tenant override explicitly sets one (IP-based hosts).
    if let Some(alias) = entry
        .tenant_overrides
        .get(tenant_id)
        .and_then(|o| o.upstream_alias.as_deref())
    {
        builder = builder.alias(alias);
    }

    if let (Some(plugin_type), Some(config)) = (
        entry.effective_auth_plugin_type_for_tenant(tenant_id),
        entry.effective_auth_config_for_tenant(tenant_id),
    ) {
        builder = builder.auth(AuthConfig {
            plugin_type: plugin_type.to_owned(),
            sharing: oagw_sdk::SharingMode::Inherit,
            config: Some(config.clone()),
        });
    }

    if let Some(headers) = crate::infra::llm::providers::upstream_headers_for_kind(entry.kind) {
        builder = builder.headers(headers);
    }

    let u = gateway
        .create_upstream(ctx.clone(), builder.build())
        .await?;
    info!(
        label,
        alias = %u.alias,
        upstream_id = %u.id,
        "OAGW tenant upstream registered"
    );
    Ok(u.alias)
}

/// Derive route match rules from `api_path` and register the OAGW route.
///
/// Tenant-specific upstreams do NOT need separate routes — OAGW's route
/// resolution falls back to ancestor upstream IDs, so tenant upstreams
/// inherit routes from the root upstream automatically.
async fn register_route(
    gateway: &Arc<dyn ServiceGatewayClientV1>,
    ctx: &toolkit_security::SecurityContext,
    provider_id: &str,
    entry: &ProviderEntry,
    upstream: &oagw_sdk::Upstream,
) -> anyhow::Result<()> {
    use oagw_sdk::{CreateRouteRequest, HttpMatch, HttpMethod, MatchRules};

    let (route_prefix, suffix_mode) = derive_route_match(&entry.api_path);
    let query_allowlist = extract_query_allowlist(&entry.api_path);

    let match_rules = MatchRules {
        http: Some(HttpMatch {
            methods: vec![HttpMethod::Post],
            path: route_prefix.clone(),
            query_allowlist,
            path_suffix_mode: suffix_mode,
        }),
        grpc: None,
    };

    match gateway
        .create_route(
            ctx.clone(),
            CreateRouteRequest::builder(upstream.id, match_rules)
                .enabled(true)
                .build(),
        )
        .await
    {
        Ok(route) => {
            info!(
                provider_id,
                route_id = %route.id,
                route_path = %route_prefix,
                "OAGW route registered"
            );
        }
        // Idempotent re-run against a reused upstream: the route is already there.
        Err(CanonicalError::AlreadyExists { .. }) => {
            info!(
                provider_id,
                route_path = %route_prefix,
                "OAGW route already registered; reusing existing"
            );
        }
        Err(e) => return Err(e.into()),
    }

    // Register RAG-related routes (Files API, Vector Stores API) on the same upstream.
    register_rag_routes(gateway, ctx, provider_id, entry, upstream).await?;

    Ok(())
}

/// RAG route definitions: method, path suffix (appended to RAG prefix), suffix mode.
///
/// Note: POST `vector_stores` uses suffix=true to cover both the create
/// endpoint (exact path) and the add-file-to-VS endpoint ({id}/files).
/// Having two routes with the same method+path but different suffix modes
/// causes OAGW to pick the first registered one, blocking the suffix path.
const RAG_ROUTES: &[(&str, &str, bool)] = &[
    // POST {prefix}/files — upload file to provider
    ("POST", "/files", false),
    // DELETE {prefix}/files/{file_id} — delete provider file
    ("DELETE", "/files", true),
    // POST {prefix}/vector_stores — create vector store (exact)
    // POST {prefix}/vector_stores/{id}/files — add file to vector store (suffix)
    // Single route with suffix=true handles both paths.
    ("POST", "/vector_stores", true),
    // DELETE {prefix}/vector_stores/{vs_id}/files/{file_id} — remove file from vector store
    ("DELETE", "/vector_stores", true),
];

/// Register OAGW routes for RAG operations (Files API, Vector Stores API).
#[allow(clippy::cognitive_complexity)]
async fn register_rag_routes(
    gateway: &Arc<dyn ServiceGatewayClientV1>,
    ctx: &toolkit_security::SecurityContext,
    provider_id: &str,
    entry: &ProviderEntry,
    upstream: &oagw_sdk::Upstream,
) -> anyhow::Result<()> {
    use oagw_sdk::{CreateRouteRequest, HttpMatch, HttpMethod, MatchRules, PathSuffixMode};

    // Derive RAG path prefix from storage_kind:
    // Azure → /openai (+ api-version query param), OpenAi → /v1
    let (prefix, query_allowlist) = match entry.storage_kind {
        crate::config::StorageKind::Azure => ("/openai", vec!["api-version".to_owned()]),
        crate::config::StorageKind::OpenAi => ("/v1", vec![]),
    };

    for &(method_str, path_suffix, append_suffix) in RAG_ROUTES {
        let method = match method_str {
            "POST" => HttpMethod::Post,
            "DELETE" => HttpMethod::Delete,
            _ => continue,
        };

        let suffix_mode = if append_suffix {
            PathSuffixMode::Append
        } else {
            PathSuffixMode::Disabled
        };

        let full_path = format!("{prefix}{path_suffix}");

        let match_rules = MatchRules {
            http: Some(HttpMatch {
                methods: vec![method],
                path: full_path.clone(),
                query_allowlist: query_allowlist.clone(),
                path_suffix_mode: suffix_mode,
            }),
            grpc: None,
        };

        match gateway
            .create_route(
                ctx.clone(),
                CreateRouteRequest::builder(upstream.id, match_rules)
                    .enabled(true)
                    .build(),
            )
            .await
        {
            Ok(route) => {
                info!(
                    provider_id,
                    route_id = %route.id,
                    route_path = %full_path,
                    method = method_str,
                    "OAGW RAG route registered"
                );
            }
            Err(e) => {
                warn!(
                    provider_id,
                    error = %e,
                    route_path = %full_path,
                    method = method_str,
                    "OAGW RAG route registration failed (may already exist)"
                );
            }
        }
    }

    Ok(())
}

/// Derive route prefix and suffix mode from an `api_path` template.
///
/// Strips query string, replaces `{model}` with `*`, and returns
/// `(prefix, suffix_mode)` for OAGW route matching.
fn derive_route_match(api_path: &str) -> (String, oagw_sdk::PathSuffixMode) {
    let route_path = api_path
        .split('?')
        .next()
        .unwrap_or(api_path)
        .replace("{model}", "*");

    let route_prefix = if let Some(pos) = route_path.find('*') {
        route_path[..pos].trim_end_matches('/').to_owned()
    } else {
        route_path.clone()
    };

    let suffix_mode = if route_path.contains('*') {
        oagw_sdk::PathSuffixMode::Append
    } else {
        oagw_sdk::PathSuffixMode::Disabled
    };

    (route_prefix, suffix_mode)
}

/// Extract query parameter names from an `api_path` template's query string.
fn extract_query_allowlist(api_path: &str) -> Vec<String> {
    api_path
        .split('?')
        .nth(1)
        .map(|qs| {
            qs.split('&')
                .filter_map(|pair| pair.split('=').next().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_simple_path() {
        let (prefix, mode) = derive_route_match("/v1/responses");
        assert_eq!(prefix, "/v1/responses");
        assert!(matches!(mode, oagw_sdk::PathSuffixMode::Disabled));
    }

    #[test]
    fn derive_path_with_model_placeholder() {
        let (prefix, mode) =
            derive_route_match("/openai/deployments/{model}/responses?api-version=2025-03-01");
        assert_eq!(prefix, "/openai/deployments");
        assert!(matches!(mode, oagw_sdk::PathSuffixMode::Append));
    }

    #[test]
    fn derive_azure_openai_path() {
        let (prefix, mode) = derive_route_match("/openai/v1/responses");
        assert_eq!(prefix, "/openai/v1/responses");
        assert!(matches!(mode, oagw_sdk::PathSuffixMode::Disabled));
    }

    #[test]
    fn extract_empty_query() {
        assert!(extract_query_allowlist("/v1/responses").is_empty());
    }

    #[test]
    fn extract_single_query_param() {
        let params =
            extract_query_allowlist("/openai/deployments/{model}/responses?api-version=2025-03-01");
        assert_eq!(params, vec!["api-version"]);
    }

    #[test]
    fn extract_multiple_query_params() {
        let params = extract_query_allowlist("/path?foo=1&bar=2&baz=3");
        assert_eq!(params, vec!["foo", "bar", "baz"]);
    }

    #[test]
    fn derive_trailing_wildcard_strips_trailing_slash() {
        let (prefix, mode) = derive_route_match("/v1/models/*/completions");
        assert_eq!(prefix, "/v1/models");
        assert!(matches!(mode, oagw_sdk::PathSuffixMode::Append));
    }

    #[test]
    fn derive_root_path() {
        let (prefix, mode) = derive_route_match("/");
        assert_eq!(prefix, "/");
        assert!(matches!(mode, oagw_sdk::PathSuffixMode::Disabled));
    }

    #[test]
    fn derive_query_string_stripped_before_matching() {
        // Query params should not affect route prefix or suffix mode.
        let (prefix, mode) = derive_route_match("/v1/responses?stream=true");
        assert_eq!(prefix, "/v1/responses");
        assert!(matches!(mode, oagw_sdk::PathSuffixMode::Disabled));
    }

    #[test]
    fn extract_query_params_with_empty_values() {
        let params = extract_query_allowlist("/path?key=&other=val");
        assert_eq!(params, vec!["key", "other"]);
    }

    // ── Provisioning logic: registration / deferral / reuse / reconcile ──────

    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    use toolkit_canonical_errors::{CanonicalError, resource_error};
    use toolkit_security::SecurityContext;

    #[resource_error("gts.cf.core.oagw.upstream.v1~")]
    struct TestUpstreamScope;

    /// Scripted `create_upstream` behavior for the mock gateway.
    #[derive(Clone)]
    enum CreateOutcome {
        /// Registration succeeds (fresh upstream).
        Ok,
        /// Backend secret not accessible / transient error → caller should defer.
        Defer,
        /// The upstream already exists under the given alias (restart / retry).
        AlreadyExists(String),
    }

    /// Configurable in-memory `ServiceGatewayClientV1` for provisioning tests.
    struct MockGw {
        outcome: Mutex<CreateOutcome>,
        existing: Mutex<Vec<oagw_sdk::Upstream>>,
        create_calls: AtomicU32,
        list_calls: AtomicU32,
        route_calls: AtomicU32,
        fail_route: AtomicBool,
    }

    impl MockGw {
        fn new(outcome: CreateOutcome, existing: Vec<oagw_sdk::Upstream>) -> Arc<Self> {
            Arc::new(Self {
                outcome: Mutex::new(outcome),
                existing: Mutex::new(existing),
                create_calls: AtomicU32::new(0),
                list_calls: AtomicU32::new(0),
                route_calls: AtomicU32::new(0),
                fail_route: AtomicBool::new(false),
            })
        }

        fn set_outcome(&self, outcome: CreateOutcome) {
            *self.outcome.lock().unwrap() = outcome;
        }

        /// Make every subsequent `create_route` fail (transient OAGW error).
        fn fail_routes(&self) {
            self.fail_route.store(true, Ordering::SeqCst);
        }
    }

    fn upstream_with_alias(alias: &str) -> oagw_sdk::Upstream {
        oagw_sdk::Upstream {
            id: uuid::Uuid::new_v4(),
            tenant_id: uuid::Uuid::nil(),
            alias: alias.to_owned(),
            server: oagw_sdk::Server {
                endpoints: vec![oagw_sdk::Endpoint {
                    scheme: oagw_sdk::Scheme::Https,
                    host: "example.com".to_owned(),
                    port: 443,
                }],
            },
            protocol: "gts.cf.core.oagw.protocol.v1~cf.core.oagw.http.v1".to_owned(),
            enabled: true,
            auth: None,
            headers: None,
            plugins: None,
            rate_limit: None,
            cors: None,
            tags: vec![],
        }
    }

    fn dummy_route() -> oagw_sdk::Route {
        oagw_sdk::Route {
            id: uuid::Uuid::new_v4(),
            tenant_id: uuid::Uuid::nil(),
            upstream_id: uuid::Uuid::nil(),
            match_rules: oagw_sdk::MatchRules {
                http: None,
                grpc: None,
            },
            plugins: None,
            rate_limit: None,
            cors: None,
            tags: vec![],
            priority: 0,
            enabled: true,
        }
    }

    #[async_trait::async_trait]
    impl ServiceGatewayClientV1 for MockGw {
        async fn create_upstream(
            &self,
            _: SecurityContext,
            _: oagw_sdk::CreateUpstreamRequest,
        ) -> Result<oagw_sdk::Upstream, CanonicalError> {
            self.create_calls.fetch_add(1, Ordering::SeqCst);
            match self.outcome.lock().unwrap().clone() {
                CreateOutcome::Ok => Ok(upstream_with_alias("mock-upstream")),
                // Any non-AlreadyExists error routes to the defer path.
                CreateOutcome::Defer => Err(CanonicalError::service_unavailable().create()),
                CreateOutcome::AlreadyExists(alias) => {
                    Err(TestUpstreamScope::already_exists("upstream already exists")
                        .with_resource(alias)
                        .create())
                }
            }
        }
        async fn list_upstreams(
            &self,
            _: SecurityContext,
            query: &oagw_sdk::ListQuery,
        ) -> Result<Vec<oagw_sdk::Upstream>, CanonicalError> {
            self.list_calls.fetch_add(1, Ordering::SeqCst);
            let existing = self.existing.lock().unwrap();
            let page = existing
                .iter()
                .skip(query.skip as usize)
                .take(query.top as usize)
                .cloned()
                .collect();
            Ok(page)
        }
        async fn create_route(
            &self,
            _: SecurityContext,
            _: oagw_sdk::CreateRouteRequest,
        ) -> Result<oagw_sdk::Route, CanonicalError> {
            self.route_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_route.load(Ordering::SeqCst) {
                return Err(CanonicalError::service_unavailable().create());
            }
            Ok(dummy_route())
        }
        async fn get_upstream(
            &self,
            _: SecurityContext,
            _: uuid::Uuid,
        ) -> Result<oagw_sdk::Upstream, CanonicalError> {
            unimplemented!()
        }
        async fn update_upstream(
            &self,
            _: SecurityContext,
            _: uuid::Uuid,
            _: oagw_sdk::UpdateUpstreamRequest,
        ) -> Result<oagw_sdk::Upstream, CanonicalError> {
            unimplemented!()
        }
        async fn delete_upstream(
            &self,
            _: SecurityContext,
            _: uuid::Uuid,
        ) -> Result<(), CanonicalError> {
            unimplemented!()
        }
        async fn get_route(
            &self,
            _: SecurityContext,
            _: uuid::Uuid,
        ) -> Result<oagw_sdk::Route, CanonicalError> {
            unimplemented!()
        }
        async fn list_routes(
            &self,
            _: SecurityContext,
            _: Option<uuid::Uuid>,
            _: &oagw_sdk::ListQuery,
        ) -> Result<Vec<oagw_sdk::Route>, CanonicalError> {
            unimplemented!()
        }
        async fn update_route(
            &self,
            _: SecurityContext,
            _: uuid::Uuid,
            _: oagw_sdk::UpdateRouteRequest,
        ) -> Result<oagw_sdk::Route, CanonicalError> {
            unimplemented!()
        }
        async fn delete_route(
            &self,
            _: SecurityContext,
            _: uuid::Uuid,
        ) -> Result<(), CanonicalError> {
            unimplemented!()
        }
        async fn resolve_proxy_target(
            &self,
            _: SecurityContext,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<(oagw_sdk::Upstream, oagw_sdk::Route), CanonicalError> {
            unimplemented!()
        }
        async fn proxy_request(
            &self,
            _: SecurityContext,
            _: http::Request<oagw_sdk::Body>,
        ) -> Result<http::Response<oagw_sdk::Body>, CanonicalError> {
            unimplemented!()
        }
    }

    fn ctx() -> SecurityContext {
        SecurityContext::anonymous()
    }

    fn provider(host: &str) -> ProviderEntry {
        ProviderEntry {
            kind: crate::infra::llm::providers::ProviderKind::OpenAiResponses,
            upstream_alias: None,
            host: host.to_owned(),
            port: None,
            use_http: false,
            api_path: "/v1/responses".to_owned(),
            auth_plugin_type: None,
            auth_config: None,
            storage_backend: None,
            supports_file_search_filters: true,
            storage_kind: crate::config::StorageKind::OpenAi,
            api_version: None,
            rag_provider: None,
            tenant_overrides: HashMap::new(),
        }
    }

    fn bad_tenant_override() -> crate::config::ProviderTenantOverride {
        // No host and no upstream_alias → nothing to derive a distinct upstream
        // from → deterministic misconfiguration.
        crate::config::ProviderTenantOverride {
            host: None,
            upstream_alias: None,
            auth_plugin_type: None,
            auth_config: None,
        }
    }

    fn good_tenant_override(host: &str) -> crate::config::ProviderTenantOverride {
        // A distinct host → a valid tenant-specific upstream can be derived.
        crate::config::ProviderTenantOverride {
            host: Some(host.to_owned()),
            upstream_alias: None,
            auth_plugin_type: None,
            auth_config: None,
        }
    }

    #[tokio::test]
    async fn register_provider_registers_on_success() {
        let gw = MockGw::new(CreateOutcome::Ok, vec![]);
        let dyn_gw: Arc<dyn ServiceGatewayClientV1> = gw.clone();
        let mut entry = provider("api.openai.com");
        let outcome = register_provider(&dyn_gw, &ctx(), "openai", &mut entry).await;
        assert!(matches!(outcome, ProviderRegistration::Registered));
        assert!(entry.upstream_alias.is_some());
        // The primary route plus the RAG routes are registered on success.
        assert!(gw.route_calls.load(Ordering::SeqCst) >= 1);
    }

    #[tokio::test]
    async fn register_provider_defers_when_secret_not_accessible() {
        let gw = MockGw::new(CreateOutcome::Defer, vec![]);
        let dyn_gw: Arc<dyn ServiceGatewayClientV1> = gw.clone();
        let mut entry = provider("api.openai.com");
        let outcome = register_provider(&dyn_gw, &ctx(), "openai", &mut entry).await;
        assert!(matches!(outcome, ProviderRegistration::Deferred));
        // No route registered when the upstream itself was deferred.
        assert_eq!(gw.route_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn register_provider_reuses_existing_on_already_exists() {
        let gw = MockGw::new(
            CreateOutcome::AlreadyExists("api.openai.com".to_owned()),
            vec![upstream_with_alias("api.openai.com")],
        );
        let dyn_gw: Arc<dyn ServiceGatewayClientV1> = gw.clone();
        let mut entry = provider("api.openai.com");
        let outcome = register_provider(&dyn_gw, &ctx(), "openai", &mut entry).await;
        assert!(matches!(outcome, ProviderRegistration::Registered));
        assert_eq!(entry.upstream_alias.as_deref(), Some("api.openai.com"));
    }

    #[tokio::test]
    async fn register_provider_defers_when_already_exists_lookup_fails() {
        // Conflict reported, but the alias is not found on lookup → cannot reuse.
        let gw = MockGw::new(
            CreateOutcome::AlreadyExists("api.openai.com".to_owned()),
            vec![],
        );
        let dyn_gw: Arc<dyn ServiceGatewayClientV1> = gw.clone();
        let mut entry = provider("api.openai.com");
        let outcome = register_provider(&dyn_gw, &ctx(), "openai", &mut entry).await;
        assert!(matches!(outcome, ProviderRegistration::Deferred));
    }

    #[tokio::test]
    async fn register_provider_misconfigured_tenant_without_host_or_alias() {
        let gw = MockGw::new(CreateOutcome::Ok, vec![]);
        let dyn_gw: Arc<dyn ServiceGatewayClientV1> = gw.clone();
        let mut entry = provider("api.openai.com");
        entry
            .tenant_overrides
            .insert("tenant-a".to_owned(), bad_tenant_override());
        let outcome = register_provider(&dyn_gw, &ctx(), "openai", &mut entry).await;
        assert!(matches!(outcome, ProviderRegistration::Misconfigured));
    }

    #[tokio::test]
    async fn register_batch_isolates_a_misconfigured_provider_from_healthy_peers() {
        // One clean provider + one with a misconfigured tenant override. The bad
        // one must not abort the batch: the clean one still registers regardless
        // of (nondeterministic) HashMap iteration order.
        let gw = MockGw::new(CreateOutcome::Ok, vec![]);
        let dyn_gw: Arc<dyn ServiceGatewayClientV1> = gw.clone();
        let mut providers = HashMap::new();
        providers.insert("good".to_owned(), provider("good.example.com"));
        let mut bad = provider("bad.example.com");
        bad.tenant_overrides
            .insert("tenant-a".to_owned(), bad_tenant_override());
        providers.insert("bad".to_owned(), bad);

        let deferred = register_oagw_upstreams(&dyn_gw, &ctx(), &mut providers)
            .await
            .unwrap();

        // The misconfigured provider is not deferred for retry (won't self-heal),
        // and the healthy provider was registered despite it.
        assert!(
            deferred.is_empty(),
            "misconfig must not be queued for retry"
        );
        assert!(
            providers["good"].upstream_alias.is_some(),
            "healthy provider must register even when a peer is misconfigured"
        );
    }

    #[tokio::test]
    async fn register_batch_defers_all_unavailable_providers_without_stopping() {
        let gw = MockGw::new(CreateOutcome::Defer, vec![]);
        let dyn_gw: Arc<dyn ServiceGatewayClientV1> = gw.clone();
        let mut providers = HashMap::new();
        providers.insert("a".to_owned(), provider("a.example.com"));
        providers.insert("b".to_owned(), provider("b.example.com"));

        let mut deferred = register_oagw_upstreams(&dyn_gw, &ctx(), &mut providers)
            .await
            .unwrap();
        deferred.sort();
        // Both deferred → the loop did not bail after the first failure.
        assert_eq!(deferred, vec!["a".to_owned(), "b".to_owned()]);
    }

    #[tokio::test]
    async fn reconcile_converges_once_secret_becomes_available() {
        let gw = MockGw::new(CreateOutcome::Defer, vec![]);
        let dyn_gw: Arc<dyn ServiceGatewayClientV1> = gw.clone();
        let mut providers = HashMap::new();
        providers.insert("openai".to_owned(), provider("api.openai.com"));

        let deferred = register_oagw_upstreams(&dyn_gw, &ctx(), &mut providers)
            .await
            .unwrap();
        assert_eq!(deferred, vec!["openai".to_owned()]);

        // Secret gets provisioned at runtime → the next reconcile registers it.
        gw.set_outcome(CreateOutcome::Ok);
        let still = reconcile_deferred_upstreams(&dyn_gw, &ctx(), &providers, &deferred)
            .await
            .unwrap();
        assert!(
            still.is_empty(),
            "provider should register once secret exists"
        );
    }

    #[tokio::test]
    async fn find_upstream_by_alias_pages_across_multiple_pages() {
        // 50 non-matching (a full first page) + the target on the second page.
        let mut ups: Vec<_> = (0..50)
            .map(|i| upstream_with_alias(&format!("other-{i}")))
            .collect();
        ups.push(upstream_with_alias("target"));
        let gw = MockGw::new(CreateOutcome::Ok, ups);
        let dyn_gw: Arc<dyn ServiceGatewayClientV1> = gw.clone();

        let found = find_upstream_by_alias(&dyn_gw, &ctx(), Some("target")).await;
        assert_eq!(found.map(|u| u.alias), Some("target".to_owned()));
        assert!(
            gw.list_calls.load(Ordering::SeqCst) >= 2,
            "must have paged past the first full page"
        );
    }

    #[tokio::test]
    async fn find_upstream_by_alias_terminates_when_absent() {
        let ups: Vec<_> = (0..10)
            .map(|i| upstream_with_alias(&format!("other-{i}")))
            .collect();
        let gw = MockGw::new(CreateOutcome::Ok, ups);
        let dyn_gw: Arc<dyn ServiceGatewayClientV1> = gw.clone();

        let found = find_upstream_by_alias(&dyn_gw, &ctx(), Some("absent")).await;
        assert!(found.is_none());
        // A short-than-full page ends paging (no infinite loop).
        assert_eq!(gw.list_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn find_upstream_by_alias_none_when_no_alias() {
        let gw = MockGw::new(CreateOutcome::Ok, vec![]);
        let dyn_gw: Arc<dyn ServiceGatewayClientV1> = gw.clone();
        assert!(
            find_upstream_by_alias(&dyn_gw, &ctx(), None)
                .await
                .is_none()
        );
        assert_eq!(gw.list_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn register_provider_registers_tenant_override_upstream() {
        // A provider with a valid tenant override (distinct host) registers the
        // root upstream *and* a tenant-specific one, stamping the OAGW-assigned
        // alias onto the override.
        let gw = MockGw::new(CreateOutcome::Ok, vec![]);
        let dyn_gw: Arc<dyn ServiceGatewayClientV1> = gw.clone();
        let mut entry = provider("api.openai.com");
        entry.tenant_overrides.insert(
            "tenant-a".to_owned(),
            good_tenant_override("tenant-a.openai.com"),
        );

        let outcome = register_provider(&dyn_gw, &ctx(), "openai", &mut entry).await;

        assert!(matches!(outcome, ProviderRegistration::Registered));
        assert_eq!(
            entry.tenant_overrides["tenant-a"].upstream_alias.as_deref(),
            Some("mock-upstream"),
            "the OAGW-assigned alias must be stamped onto the tenant override"
        );
        // Two upstreams created: the root and the tenant-specific one.
        assert_eq!(gw.create_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn register_provider_reuses_existing_tenant_upstream_on_already_exists() {
        // On restart the tenant upstream already exists: the `AlreadyExists`
        // conflict carries the taken alias, reused directly without a lookup.
        let gw = MockGw::new(
            CreateOutcome::AlreadyExists("api.openai.com".to_owned()),
            vec![upstream_with_alias("api.openai.com")],
        );
        let dyn_gw: Arc<dyn ServiceGatewayClientV1> = gw.clone();
        let mut entry = provider("api.openai.com");
        entry.tenant_overrides.insert(
            "tenant-a".to_owned(),
            good_tenant_override("tenant-a.openai.com"),
        );

        let outcome = register_provider(&dyn_gw, &ctx(), "openai", &mut entry).await;

        assert!(matches!(outcome, ProviderRegistration::Registered));
        assert_eq!(
            entry.tenant_overrides["tenant-a"].upstream_alias.as_deref(),
            Some("api.openai.com"),
            "tenant override reuses the alias named by the AlreadyExists conflict"
        );
    }

    #[tokio::test]
    async fn register_provider_defers_when_route_registration_fails() {
        // Upstream registers, but the route call hits a transient OAGW error →
        // the provider is deferred for retry (not crashed, not misconfigured).
        let gw = MockGw::new(CreateOutcome::Ok, vec![]);
        gw.fail_routes();
        let dyn_gw: Arc<dyn ServiceGatewayClientV1> = gw.clone();
        let mut entry = provider("api.openai.com");

        let outcome = register_provider(&dyn_gw, &ctx(), "openai", &mut entry).await;

        assert!(matches!(outcome, ProviderRegistration::Deferred));
        // The root upstream was created; the route attempt failed and aborted
        // the provider before any tenant work.
        assert_eq!(gw.create_calls.load(Ordering::SeqCst), 1);
        assert!(gw.route_calls.load(Ordering::SeqCst) >= 1);
    }
}
