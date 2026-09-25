//! Shared REST plumbing; phase 1c adds the SKU registry routes.
use crate::domain::error::DomainError;
use crate::domain::validation::ValidationReport;
use crate::infra::storage::RepoError;
use axum::Router;
use axum::extract::Extension;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::Value as JsonValue;
use toolkit::api::canonical_prelude::CanonicalError;
use toolkit_security::SecurityContext;

pub mod approval_policy;
pub mod approval_units;
pub mod browse;
pub mod categories;
pub mod dto;
pub(crate) mod governance;
pub mod preconditions;
pub mod references;
mod replay;
pub mod sku_governance;
pub mod skus;

/// The reserved service prefix.
pub const PREFIX: &str = "/bss-products/v1";
/// Optional replay key header.
pub const IDEMPOTENCY_KEY_HEADER: &str = "Idempotency-Key";
/// Maximum replay key size in bytes.
pub const IDEMPOTENCY_KEY_MAX_BYTES: usize = 255;

/// Dependencies shared by the registry routes.
pub struct ApiState {
    pub db: toolkit_db::DBProvider<toolkit_db::DbError>,
    pub sink: crate::infra::broker::EventSink,
    pub usage_type_catalog: std::sync::Arc<dyn bss_products_sdk::usage_types::UsageTypeCatalog>,
    pub usage_type_catalog_source: &'static str,
    pub idempotency_retention_hours: u32,
    pub(crate) fence_ttl_minutes: u32,
    pub(crate) reference_principals: std::collections::BTreeMap<uuid::Uuid, String>,
}

/// Shared REST foundation helper.
pub fn router(host_router: Router) -> Router {
    host_router.nest(PREFIX, Router::new())
}

/// Shared REST foundation helper.
pub fn require_authenticated(
    extension_ctx: Option<Extension<SecurityContext>>,
) -> Result<SecurityContext, CanonicalError> {
    let Some(Extension(ctx)) = extension_ctx else {
        return Err(unauthenticated());
    };
    if ctx.subject_id().is_nil() || ctx.subject_tenant_id().is_nil() {
        return Err(unauthenticated());
    }
    if ctx.subject_type().is_none() {
        return Err(unauthenticated());
    }
    Ok(ctx)
}

/// Shared REST foundation helper.
#[must_use]
pub fn unauthenticated() -> CanonicalError {
    CanonicalError::unauthenticated()
        .with_reason("AUTHENTICATION_REQUIRED")
        .create()
}

/// Shared REST foundation helper.
pub fn repo_error_to_canonical(err: &RepoError) -> CanonicalError {
    tracing::error!(error = %err, "bss-products: repository failure");
    CanonicalError::internal(format!("bss-products: {err}")).create()
}

/// Shared REST foundation helper.
pub fn idempotency_key(headers: &HeaderMap) -> Result<Option<String>, DomainError> {
    let Some(raw) = headers.get(IDEMPOTENCY_KEY_HEADER) else {
        return Ok(None);
    };
    let value = raw
        .to_str()
        .map_err(|_| refuse_idempotency_key("the header value is not valid UTF-8"))?
        .trim();
    if value.is_empty() {
        return Err(refuse_idempotency_key(
            "the header is present but blank; send a stable, caller-chosen key or omit the \
             header entirely",
        ));
    }
    if value.len() > IDEMPOTENCY_KEY_MAX_BYTES {
        return Err(refuse_idempotency_key(&format!(
            "the header value is {} bytes long and a key is at most {IDEMPOTENCY_KEY_MAX_BYTES}",
            value.len()
        )));
    }
    Ok(Some(value.to_owned()))
}

/// Shared REST foundation helper.
fn refuse_idempotency_key(detail: &str) -> DomainError {
    let mut report = ValidationReport::new();
    report.violate("VALIDATION", IDEMPOTENCY_KEY_HEADER, detail);
    DomainError::Validation(report)
}

/// Shared REST foundation helper.
pub fn replay_response(status: i32, body: JsonValue) -> Response {
    let recorded = u16::try_from(status)
        .ok()
        .and_then(|code| StatusCode::from_u16(code).ok());
    if let Some(code) = recorded {
        return (code, axum::Json(body)).into_response();
    }
    tracing::error!(
        status,
        "bss-products: stored idempotency response_status is not a status code"
    );
    CanonicalError::internal(format!(
        "bss-products: stored idempotency response_status {status} is not a status code"
    ))
    .create()
    .into_response()
}

/// Transaction refusals preserve typed database errors for retry classification.
pub(crate) enum TxError {
    Refused(DomainError),
    Repo(RepoError),
    ApprovalDb(sea_orm::DbErr),
    GenerationMismatch {
        seen: i32,
        current: i32,
    },
    FencedReferences {
        code: &'static str,
        rows: serde_json::Value,
    },
}
impl From<toolkit_db::DbError> for TxError {
    fn from(e: toolkit_db::DbError) -> Self {
        match e {
            toolkit_db::DbError::Sea(source) => Self::Repo(RepoError::Driver {
                context: "transaction".into(),
                source,
            }),
            other => Self::Repo(RepoError::Db(other.to_string())),
        }
    }
}
impl From<bss_approval::ApprovalError> for TxError {
    fn from(e: bss_approval::ApprovalError) -> Self {
        match e {
            bss_approval::ApprovalError::Db(db) => Self::ApprovalDb(db),
            bss_approval::ApprovalError::GenerationMismatch { seen, current } => {
                Self::GenerationMismatch { seen, current }
            }
            other => Self::Refused(other.into()),
        }
    }
}
/// Driver errors reach the toolkit retry classifier unchanged.
pub(crate) fn contention_db_err(e: &TxError) -> Option<&sea_orm::DbErr> {
    match e {
        TxError::Repo(RepoError::Driver { source, .. }) | TxError::ApprovalDb(source) => {
            Some(source)
        }
        TxError::Repo(_)
        | TxError::Refused(_)
        | TxError::GenerationMismatch { .. }
        | TxError::FencedReferences { .. } => None,
    }
}
/// Convert only after the retry loop has finished.
pub(crate) fn tx_to_canonical(e: TxError) -> CanonicalError {
    match e {
        TxError::Refused(d) => d.into(),
        TxError::FencedReferences { code, rows } => DomainError::Conflict {
            code,
            detail: rows.to_string(),
        }
        .into(),
        TxError::GenerationMismatch { seen, current } => {
            DomainError::from(bss_approval::ApprovalError::GenerationMismatch { seen, current })
                .into()
        }
        TxError::Repo(r) => repo_error_to_canonical(&r),
        TxError::ApprovalDb(source) => repo_error_to_canonical(&RepoError::Driver {
            context: "approval".into(),
            source,
        }),
    }
}
/// Category assignment and retirement must not write-skew on `PostgreSQL`.
pub(crate) fn category_tx_config(state: &ApiState) -> toolkit_db::secure::TxConfig {
    if state.db.db().backend() == sea_orm::DbBackend::Postgres {
        toolkit_db::secure::TxConfig::serializable()
    } else {
        toolkit_db::secure::TxConfig::default()
    }
}

/// Map the PEP denial while keeping PDP outages fail-closed.
pub(crate) fn authz_error_to_canonical(
    err: crate::authz::AuthzError,
    denied: impl FnOnce(String) -> CanonicalError,
) -> CanonicalError {
    match err {
        crate::authz::AuthzError::Denied(reason) => denied(reason),
        crate::authz::AuthzError::Unavailable(detail) => {
            tracing::error!(detail, "bss-products: authorization service unavailable");
            CanonicalError::service_unavailable().create()
        }
    }
}

/// JSON shape errors use the same 400 validation envelope as field rules.
pub(crate) fn json_body<T>(
    body: Result<axum::Json<T>, axum::extract::rejection::JsonRejection>,
) -> Result<T, CanonicalError> {
    body.map(|axum::Json(body)| body).map_err(|error| {
        let mut report = ValidationReport::new();
        report.violate("VALIDATION", "body", error.body_text());
        DomainError::Validation(report).into()
    })
}

impl From<crate::infra::events::EventsError> for TxError {
    fn from(error: crate::infra::events::EventsError) -> Self {
        bss_approval::ApprovalError::from(error).into()
    }
}
