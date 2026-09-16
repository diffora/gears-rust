//! Tenant invoice template and GL defaults (D-373). Publishing freezes these on
//! each row; changing a default never changes an already published price.

use crate::api::rest::auth_context::{audit_stamp, require_authenticated};
use crate::api::rest::correlation::{CorrelationId, require_correlation};
use crate::api::rest::error::authz_error_to_canonical;
use crate::api::rest::{preconditions, state::AuthoringState};
use crate::domain::concurrency::{PolicyTag, TaxonomyTagEntry};
use crate::domain::error::DomainError;
use crate::domain::line_template::DefaultLineTemplates;
use crate::infra::storage::{repo::policy_repo, repo_failure};
use axum::extract::Extension;
use axum::http::{HeaderMap, StatusCode, header::ETAG};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use std::collections::BTreeMap;
use std::sync::Arc;
use time::OffsetDateTime;
use toolkit::api::canonical_prelude::CanonicalError;
use toolkit::api::{OpenApiRegistry, operation_builder::OperationBuilder};
use toolkit_db::secure::AccessScope;
use toolkit_security::SecurityContext;

/// Tenant descriptor defaults resource.
pub const BILLING_DESCRIPTORS: &str = "/bss-pricing/v1/config/billing-descriptors";

/// Whole replacement of the tenant's descriptor defaults.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct BillingDescriptorPolicyView {
    /// Null clears the fallback and requires GL overrides on published rows.
    pub default_gl_code_ref: Option<String>,
    /// Exactly `recurring`, `usage`, `one_time` and `one_time_setup` template sources.
    pub default_line_templates: BTreeMap<String, String>,
}

type Defaults = (Option<String>, DefaultLineTemplates);

/// Register the conditional read and compare-and-swap configuration write.
pub fn router(state: Arc<AuthoringState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::get("/bss-pricing/v1/config/billing-descriptors")
        .operation_id("bss_pricing.get_billing_descriptors")
        .summary("Read tenant billing descriptor defaults")
        .tag("BSS Pricing Configuration")
        .authenticated()
        .no_license_required()
        .param(crate::api::rest::plans::if_none_match_param())
        .handler(get_policy)
        .json_response_with_schema::<BillingDescriptorPolicyView>(
            openapi,
            StatusCode::OK,
            "Current defaults.",
        )
        .no_content_response(StatusCode::NOT_MODIFIED, "The representation is unchanged.")
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(Router::new(), openapi);
    let router = OperationBuilder::put("/bss-pricing/v1/config/billing-descriptors")
        .operation_id("bss_pricing.put_billing_descriptors")
        .summary("Replace tenant billing descriptor defaults")
        .description("Requires If-Match. Validates seven template placeholders and four charge-kind keys. Null clears the GL default. Existing published rows retain their frozen values.")
        .tag("BSS Pricing Configuration").authenticated().no_license_required()
        .param(crate::api::rest::rounding_policy::if_match_param())
        .json_request::<BillingDescriptorPolicyView>(openapi, "Complete replacement defaults.")
        .handler(put_policy)
        .json_response_with_schema::<BillingDescriptorPolicyView>(openapi, StatusCode::OK, "Stored defaults.")
        .error_400(openapi).error_401(openapi).error_403(openapi).error_409(openapi)
        .error_500(openapi).error_503(openapi)
        .register(router, openapi);
    router
        .layer(Extension(state))
        .layer(axum::middleware::from_fn(
            crate::api::rest::correlation::establish,
        ))
}

async fn get_policy(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    headers: HeaderMap,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let scope = read_scope(&enforcer, &ctx).await?;
    Ok(render(
        &held(&state, &scope, ctx.subject_tenant_id()).await?,
        Some(&headers),
    ))
}

async fn put_policy(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    extension_correlation: Option<Extension<CorrelationId>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let correlation = require_correlation(extension_correlation)?;
    let scope = write_scope(&enforcer, &ctx).await?;
    let asserted = preconditions::if_match_policy(&headers)?;
    let request: BillingDescriptorPolicyView = preconditions::parse_body(&body)?;
    if request
        .default_gl_code_ref
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err(DomainError::InvalidRequest(
            "default_gl_code_ref must not be blank; use null to clear it".to_owned(),
        )
        .into());
    }
    let mut report = crate::domain::validation::ValidationReport::default();
    for (kind, source) in &request.default_line_templates {
        if let Err(errors) = crate::domain::line_template::parse(source) {
            report.violate_at_write("LINE_TEMPLATE_INVALID", kind, format!("{errors:?}"));
        }
    }
    if !report.is_publishable() {
        return Err(DomainError::ValidationFailed(report).into());
    }
    let requested = (
        request.default_gl_code_ref,
        DefaultLineTemplates::from_map(request.default_line_templates)
            .map_err(DomainError::InvalidRequest)?,
    );
    let tenant = ctx.subject_tenant_id();
    let expected = held(&state, &scope, tenant).await?;
    if tag_of(&expected) != asserted {
        return Err(stale());
    }
    let conn = state
        .db
        .conn()
        .map_err(|e| DomainError::Internal(format!("policy conn: {e}")))?;
    if let Some(reference) = &requested.0 {
        let declared =
            crate::infra::storage::repo::taxonomy_repo::active_gl_codes(&conn, &scope, tenant)
                .await
                .map_err(|e| repo_failure(&e))?;
        let rule = crate::domain::taxonomy::GlCodeDeclared {
            tenant_default: None,
            declared,
        };
        if let Some(violation) = rule.violation_for("default_gl_code_ref", reference) {
            let mut report = crate::domain::validation::ValidationReport::default();
            report.violate_at_write(violation.code, violation.subject, violation.detail);
            return Err(DomainError::ValidationFailed(report).into());
        }
    }
    let applied = policy_repo::set_descriptor_defaults(
        &conn,
        &scope,
        tenant,
        &requested,
        &expected,
        &audit_stamp(&ctx, OffsetDateTime::now_utc(), correlation),
    )
    .await
    .map_err(|e| repo_failure(&e))?;
    if !applied {
        return Err(stale());
    }
    Ok(render(&requested, None))
}

fn stale() -> CanonicalError {
    DomainError::StaleVersion("billing descriptor defaults changed; re-read their ETag".to_owned())
        .into()
}

async fn held(
    state: &AuthoringState,
    scope: &AccessScope,
    tenant: uuid::Uuid,
) -> Result<Defaults, CanonicalError> {
    let conn = state
        .db
        .conn()
        .map_err(|e| DomainError::Internal(format!("policy conn: {e}")))?;
    policy_repo::descriptor_defaults_on(&conn, scope, tenant)
        .await
        .map_err(|e| repo_failure(&e).into())
}

fn tag_of(value: &Defaults) -> PolicyTag {
    let mut entries = value.1.to_map();
    entries.insert(
        "default_gl_code_ref".to_owned(),
        value.0.clone().unwrap_or_default(),
    );
    PolicyTag::of_taxonomy(
        "billing-descriptors",
        entries.iter().map(|(key, value)| TaxonomyTagEntry {
            value: key,
            state: "set",
            display_name: value,
            tax_category: None,
            tax_rate_present: false,
        }),
    )
}

fn render(value: &Defaults, conditional: Option<&HeaderMap>) -> Response {
    let tag = preconditions::policy_etag(&tag_of(value));
    if conditional.is_some_and(|headers| preconditions::if_none_match(headers, &tag)) {
        return preconditions::not_modified(&tag);
    }
    (
        [(ETAG, tag)],
        Json(BillingDescriptorPolicyView {
            default_gl_code_ref: value.0.clone(),
            default_line_templates: value.1.to_map(),
        }),
    )
        .into_response()
}

async fn read_scope(
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
) -> Result<AccessScope, CanonicalError> {
    crate::authz::access_scope(
        enforcer,
        ctx,
        &crate::authz::resource_types::CONFIG,
        crate::authz::actions::READ,
        /* owner_tenant_id */ None,
        /* resource_id */ None,
    )
    .await
    .map_err(authz_error_to_canonical)
}

async fn write_scope(
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
) -> Result<AccessScope, CanonicalError> {
    crate::authz::access_scope(
        enforcer,
        ctx,
        &crate::authz::resource_types::CONFIG,
        crate::authz::actions::WRITE,
        /* owner_tenant_id */ Some(crate::authz::OwnerTenant(ctx.subject_tenant_id())),
        /* resource_id */ None,
    )
    .await
    .map_err(authz_error_to_canonical)
}
