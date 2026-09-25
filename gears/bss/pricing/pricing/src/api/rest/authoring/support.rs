//! Canonical failures, transaction retries, and audit plumbing shared by the doors.
use crate::{
    authz,
    infra::storage::{RepoError, repo},
};
use axum::{
    Extension, Json,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use toolkit::api::canonical_prelude::{CanonicalError, resource_error};
use toolkit_db::{
    Db, DbTx,
    secure::{AccessScope, DBRunner},
};
use toolkit_security::SecurityContext;
use uuid::Uuid;
#[resource_error(gts_id!("cf.bss.pricing.price_book.v1~"))]
struct PricingResource;
pub fn require_authenticated(
    ctx: Option<Extension<SecurityContext>>,
) -> Result<SecurityContext, CanonicalError> {
    ctx.map(|Extension(c)| c)
        .filter(|c| {
            !c.subject_id().is_nil()
                && !c.subject_tenant_id().is_nil()
                && c.subject_type().is_some()
        })
        .ok_or_else(|| {
            CanonicalError::unauthenticated()
                .with_reason("AUTHENTICATION_REQUIRED")
                .create()
        })
}
pub fn authz_failure(error: authz::AuthzError) -> CanonicalError {
    match error {
        authz::AuthzError::Denied(d) => PricingResource::permission_denied()
            .with_reason(d.reason)
            .create(),
        authz::AuthzError::Unavailable(detail) => {
            tracing::error!(detail, "pricing authorization unavailable");
            CanonicalError::service_unavailable().create()
        }
    }
}
pub fn invalid(field: &str, code: &str) -> CanonicalError {
    PricingResource::invalid_argument()
        .with_field_violation(field, code, code)
        .create()
}
pub fn conflict(code: &str) -> CanonicalError {
    PricingResource::aborted(code).with_reason(code).create()
}
pub fn missing() -> CanonicalError {
    PricingResource::not_found("Price book not found")
        .with_resource("price_book")
        .create()
}
/// A named pricing resource that the caller's tenant does not hold.
pub fn missing_what(what: &str) -> CanonicalError {
    PricingResource::not_found(format!("{what} not found"))
        .with_resource(what)
        .create()
}
/// Claim a POST's key inside the mutation transaction, or replay its stored answer.
/// # Errors
/// A different payload under the key is `IDEMPOTENCY_CONFLICT`; a live claim is in flight.
pub async fn claim(
    tx: &impl DBRunner,
    tenant: Uuid,
    endpoint: &str,
    key: &str,
    digest: &[u8],
) -> Result<Option<Response>, DoorError> {
    let now = time::OffsetDateTime::now_utc();
    let scope = AccessScope::for_tenant(tenant);
    match repo::idempotency_repo::claim_idempotency_key(
        tx,
        &scope,
        tenant,
        endpoint,
        key,
        digest,
        now,
        now + time::Duration::hours(24),
    )
    .await?
    {
        repo::idempotency_repo::IdempotencyClaim::Claimed => Ok(None),
        repo::idempotency_repo::IdempotencyClaim::Answered {
            payload_hash,
            response_status,
            response_body,
        } => {
            if payload_hash != digest {
                return Err(conflict("IDEMPOTENCY_CONFLICT").into());
            }
            let status = u16::try_from(response_status)
                .ok()
                .and_then(|s| StatusCode::from_u16(s).ok())
                .ok_or_else(|| CanonicalError::internal("invalid stored status").create())?;
            Ok(Some(response(
                status,
                &response_body["body"],
                response_body["etag"].as_u64(),
            )?))
        }
        repo::idempotency_repo::IdempotencyClaim::InFlight { payload_hash, .. } => {
            Err(conflict(if payload_hash == digest {
                "IDEMPOTENCY_KEY_IN_FLIGHT"
            } else {
                "IDEMPOTENCY_CONFLICT"
            })
            .into())
        }
        repo::idempotency_repo::IdempotencyClaim::TakeoverRaceLost => {
            Err(conflict("IDEMPOTENCY_KEY_IN_FLIGHT").into())
        }
    }
}
/// Record the answer of a claimed key in the same transaction and render it.
/// # Errors
/// A lost claim is an internal failure; the transaction rolls back.
pub async fn answer<T: serde::Serialize>(
    tx: &impl DBRunner,
    tenant: Uuid,
    endpoint: &str,
    key: &str,
    status: StatusCode,
    body: &T,
    etag: Option<u64>,
) -> Result<Response, DoorError> {
    let body = value(body)?;
    if repo::idempotency_repo::answer_idempotency_key(
        tx,
        &AccessScope::for_tenant(tenant),
        tenant,
        endpoint,
        key,
        i32::from(status.as_u16()),
        serde_json::json!({ "etag": etag, "body": body }),
    )
    .await?
        != repo::idempotency_repo::IdempotencyAnswer::Recorded
    {
        return Err(CanonicalError::internal("idempotency claim lost")
            .create()
            .into());
    }
    Ok(response(status, &body, etag)?)
}
#[derive(Debug, thiserror::Error)]
pub enum DoorError {
    #[error(transparent)]
    Repo(#[from] RepoError),
    #[error(transparent)]
    Api(#[from] CanonicalError),
}
impl From<toolkit_db::DbError> for DoorError {
    fn from(e: toolkit_db::DbError) -> Self {
        Self::Repo(e.into())
    }
}
impl From<DoorError> for CanonicalError {
    fn from(e: DoorError) -> Self {
        match e {
            DoorError::Api(e) => e,
            DoorError::Repo(RepoError::Conflict { code }) => conflict(code),
            DoorError::Repo(e) => {
                tracing::error!(error=%e,"pricing storage failure");
                Self::internal("pricing storage failure").create()
            }
        }
    }
}
pub async fn transaction<T: Send + 'static>(
    db: &Db,
    work: impl for<'a> FnMut(
        &'a DbTx<'a>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<T, DoorError>> + Send + 'a>,
    > + Send,
) -> Result<T, CanonicalError> {
    transaction_door(db, work).await.map_err(Into::into)
}
/// The serializable retrying transaction, keeping the door's typed refusal.
/// # Errors
/// Returns the last attempt's refusal or storage failure.
pub async fn transaction_door<T: Send + 'static>(
    db: &Db,
    work: impl for<'a> FnMut(
        &'a DbTx<'a>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<T, DoorError>> + Send + 'a>,
    > + Send,
) -> Result<T, DoorError> {
    db.transaction_with_retry(
        toolkit_db::secure::TxConfig::serializable(),
        |e| match e {
            DoorError::Repo(RepoError::Driver { source, .. }) => Some(source),
            _ => None,
        },
        work,
    )
    .await
}
pub fn response<T: serde::Serialize>(
    status: StatusCode,
    body: &T,
    version: Option<u64>,
) -> Result<Response, CanonicalError> {
    let mut headers = HeaderMap::new();
    if let Some(version) = version {
        headers.insert(
            "etag",
            format!("\"{version}\"")
                .parse()
                .map_err(|_| CanonicalError::internal("invalid ETag").create())?,
        );
    }
    Ok((status, headers, Json(body)).into_response())
}
pub fn value<T: serde::Serialize>(body: &T) -> Result<serde_json::Value, CanonicalError> {
    serde_json::to_value(body)
        .map_err(|_| CanonicalError::internal("pricing serialization failed").create())
}
pub fn date(raw: Option<String>, field: &str) -> Result<Option<time::Date>, CanonicalError> {
    raw.map(|s| {
        time::Date::parse(
            &s,
            &time::macros::format_description!("[year]-[month]-[day]"),
        )
        .map_err(|_| invalid(field, "DATE_INVALID"))
    })
    .transpose()
}
pub fn check_version(seen: u64, actual: i64) -> Result<(), CanonicalError> {
    if u64::try_from(actual).ok() == Some(seen) {
        Ok(())
    } else {
        Err(conflict("VERSION_CONFLICT"))
    }
}
pub async fn audit(
    tx: &impl DBRunner,
    ctx: &SecurityContext,
    correlation: Uuid,
    action: &str,
    id: Uuid,
    version: i64,
) -> Result<(), DoorError> {
    repo::audit_repo::write_eventless_act_audit(
        tx,
        &AccessScope::for_tenant(ctx.subject_tenant_id()),
        repo::audit_repo::AuditCommon {
            audit_id: Uuid::now_v7(),
            tenant_id: ctx.subject_tenant_id(),
            actor_ref: ctx.subject_id(),
            action: action.into(),
            subject_kind: "pricing".into(),
            reason: None,
            correlation_id: Some(correlation.to_string()),
            written_at: time::OffsetDateTime::now_utc(),
        },
        id,
        Some(version),
    )
    .await?;
    Ok(())
}
pub fn header(name: &str) -> toolkit::api::operation_builder::ParamSpec {
    use toolkit::api::operation_builder::{ParamLocation, ParamSpec};
    ParamSpec {
        name: name.into(),
        location: ParamLocation::Header,
        required: true,
        description: Some("Required authoring precondition".into()),
        param_type: "string".into(),
        array: false,
    }
}
