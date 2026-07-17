use tower_http::cors::CorsLayer;

use crate::config::{ApiGatewayConfig, CorsConfig};

/// Validate the CORS configuration at config-load time, BEFORE any
/// `CorsLayer` is constructed. tower-http enforces the same rules with
/// an `assert!` inside `Layer::layer` — with axum's eager router
/// layering that would be a startup panic (crash-loop) with no pointer
/// at the offending config; failing `init` instead surfaces a clean,
/// actionable error.
///
/// # Errors
/// Returns a human-readable description of the invalid combination.
pub fn validate_cors_config(cfg: &ApiGatewayConfig) -> Result<(), String> {
    if !cfg.cors_enabled {
        return Ok(());
    }
    let cors_cfg: CorsConfig = cfg.cors.clone().unwrap_or_default();
    if !cors_cfg.allow_credentials {
        return Ok(());
    }
    // tower-http rejects every wildcard when credentials are allowed
    // (`ensure_usable_cors_rules`): origins, methods, request headers,
    // and exposed headers.
    let wildcard_lists = [
        ("cors.allowed_origins", &cors_cfg.allowed_origins),
        ("cors.allowed_methods", &cors_cfg.allowed_methods),
        ("cors.allowed_headers", &cors_cfg.allowed_headers),
        ("cors.exposed_headers", &cors_cfg.exposed_headers),
    ];
    for (name, list) in wildcard_lists {
        if list.iter().any(|v| v == "*") {
            return Err(format!(
                "invalid CORS configuration: `{name}` contains the wildcard \"*\" while \
                 `cors.allow_credentials` is true; browsers forbid this combination and \
                 tower-http would panic at startup — list the values explicitly or \
                 disable credentials"
            ));
        }
    }
    Ok(())
}

/// Parse a configured string list into typed values, WARNING about (and
/// skipping) every entry that does not parse instead of dropping it
/// silently. A silently-dropped entry is how a config typo turns into a
/// hard-to-diagnose behavioral hole — e.g. an unparseable
/// `exposed_headers` entry silently suppresses `Access-Control-Expose-Headers`
/// and breaks every ETag-guarded browser write.
fn parse_list<T: std::str::FromStr>(list_name: &'static str, items: &[String]) -> Vec<T> {
    items
        .iter()
        .filter_map(|s| {
            s.parse::<T>()
                .map_err(|_| {
                    tracing::warn!(
                        entry = %s,
                        list = list_name,
                        "api-gateway CORS config entry does not parse; ignoring it"
                    );
                })
                .ok()
        })
        .collect()
}

/// Build a CORS layer from config. `["*"]` in a list means "any";
/// otherwise entries are parsed into their typed forms with a warning
/// for every unparseable entry (see [`parse_list`]). Invalid
/// wildcard+credentials combinations are rejected earlier by
/// [`validate_cors_config`] at gear init.
pub fn build_cors_layer(cfg: &ApiGatewayConfig) -> CorsLayer {
    let cors_cfg: CorsConfig = cfg.cors.clone().unwrap_or_default();

    let mut layer = CorsLayer::new();

    if cors_cfg.allowed_origins.iter().any(|o| o == "*") {
        layer = layer.allow_origin(tower_http::cors::Any);
    } else {
        let origins: Vec<axum::http::HeaderValue> =
            parse_list("allowed_origins", &cors_cfg.allowed_origins);
        if !origins.is_empty() {
            layer = layer.allow_origin(origins);
        }
    }

    if cors_cfg.allowed_methods.iter().any(|m| m == "*") {
        layer = layer.allow_methods(tower_http::cors::Any);
    } else {
        let methods: Vec<axum::http::Method> =
            parse_list("allowed_methods", &cors_cfg.allowed_methods);
        if !methods.is_empty() {
            layer = layer.allow_methods(methods);
        }
    }

    if cors_cfg.allowed_headers.iter().any(|h| h == "*") {
        layer = layer.allow_headers(tower_http::cors::Any);
    } else {
        let headers: Vec<axum::http::HeaderName> =
            parse_list("allowed_headers", &cors_cfg.allowed_headers);
        if !headers.is_empty() {
            layer = layer.allow_headers(headers);
        }
    }

    if cors_cfg.exposed_headers.iter().any(|h| h == "*") {
        layer = layer.expose_headers(tower_http::cors::Any);
    } else {
        let headers: Vec<axum::http::HeaderName> =
            parse_list("exposed_headers", &cors_cfg.exposed_headers);
        if !headers.is_empty() {
            layer = layer.expose_headers(headers);
        }
    }

    if cors_cfg.allow_credentials {
        layer = layer.allow_credentials(true);
    }

    if cors_cfg.max_age_seconds > 0 {
        layer = layer.max_age(std::time::Duration::from_secs(cors_cfg.max_age_seconds));
    }

    layer
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ApiGatewayConfig;

    fn cfg_with(cors: CorsConfig) -> ApiGatewayConfig {
        ApiGatewayConfig {
            cors_enabled: true,
            cors: Some(cors),
            ..Default::default()
        }
    }

    // The exact combination tower-http panics on at layer construction:
    // any wildcard list + allow_credentials. Must be a config-load error,
    // not a startup crash-loop.
    #[test]
    fn wildcard_exposed_headers_with_credentials_rejected_at_validation() {
        // Explicit origins/methods/headers so ONLY exposed_headers carries
        // the wildcard — pins the newly-added list's own validation arm.
        let cfg = cfg_with(CorsConfig {
            allowed_origins: vec!["https://ui.example.com".to_owned()],
            allowed_methods: vec!["GET".to_owned()],
            allowed_headers: vec!["Content-Type".to_owned()],
            exposed_headers: vec!["*".to_owned()],
            allow_credentials: true,
            ..Default::default()
        });
        let err = validate_cors_config(&cfg).expect_err("wildcard+credentials must be rejected");
        assert!(err.contains("exposed_headers"), "got: {err}");
    }

    #[test]
    fn wildcard_origins_with_credentials_rejected_at_validation() {
        let cfg = cfg_with(CorsConfig {
            allow_credentials: true,
            ..Default::default() // default allowed_origins/headers are ["*"]
        });
        assert!(validate_cors_config(&cfg).is_err());
    }

    #[test]
    fn explicit_lists_with_credentials_pass_validation() {
        let cfg = cfg_with(CorsConfig {
            allowed_origins: vec!["https://ui.example.com".to_owned()],
            allowed_methods: vec!["GET".to_owned(), "POST".to_owned()],
            allowed_headers: vec!["Content-Type".to_owned()],
            exposed_headers: vec!["ETag".to_owned()],
            allow_credentials: true,
            ..Default::default()
        });
        assert!(validate_cors_config(&cfg).is_ok());
    }

    #[test]
    fn cors_disabled_skips_validation() {
        let mut cfg = cfg_with(CorsConfig {
            exposed_headers: vec!["*".to_owned()],
            allow_credentials: true,
            ..Default::default()
        });
        cfg.cors_enabled = false;
        assert!(validate_cors_config(&cfg).is_ok());
    }
}
