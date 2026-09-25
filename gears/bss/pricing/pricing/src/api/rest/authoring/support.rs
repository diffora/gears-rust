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
#[cfg(test)]
#[path = "support_tests.rs"]
mod tests;
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
    invalid_because(field, code, code)
}
/// [`invalid`] whose violation tells the client what to send instead.
pub fn invalid_because(field: &str, code: &str, description: &str) -> CanonicalError {
    PricingResource::invalid_argument()
        .with_field_violation(field, description, code)
        .create()
}
/// A 403 with its own code: the caller may act on the resource type, not on this one.
pub fn forbidden(code: &str) -> CanonicalError {
    PricingResource::permission_denied()
        .with_reason(code)
        .create()
}
/// [`forbidden`] whose detail names what forbids the act (for example the other author's
/// draft). The toolkit fixes a permission problem's detail text, so it is set on the rendered
/// problem and read back.
pub fn forbidden_because(code: &str, detail: impl Into<String>) -> CanonicalError {
    let mut problem = toolkit::api::canonical_prelude::Problem::from(forbidden(code));
    problem.detail = detail.into();
    CanonicalError::try_from(problem).unwrap_or_else(|_| forbidden(code))
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
    /// A vote named another generation; the answer carries the current one.
    #[error("the vote names another generation; the unit is at {current}")]
    Generation { current: i32 },
}
/// Refusals of the shared approval engine and the `prices` subject, with their codes.
///
/// A pure-rule refusal is 400 with its code (D-403); a conflict is 409; separation of duties
/// and the submitter-only withdraw are 403. Database errors stay typed for the retry loop.
#[must_use]
pub fn approval_failure(error: bss_approval::ApprovalError) -> DoorError {
    use bss_approval::ApprovalError as A;
    match error {
        A::Db(source) => DoorError::Repo(RepoError::Driver {
            context: "approval".into(),
            source,
        }),
        A::InvalidSubmit { code, field, .. } => match code {
            "PRICE_NOT_DRAFT" | "ENTRY_REFERENCE_LOST" => conflict(code).into(),
            "REGISTRY_UNAVAILABLE" => unavailable().into(),
            "PRICE_NOT_FOUND" => missing_what("price").into(),
            _ => invalid(&field, code).into(),
        },
        A::ApplyRefused { code, detail } => {
            if code == "REGISTRY_UNAVAILABLE" {
                unavailable().into()
            } else {
                PricingResource::aborted(format!("{code}: {detail}"))
                    .with_reason("APPLY_REFUSED")
                    .create()
                    .into()
            }
        }
        A::SodViolation | A::NotSubmitter => PricingResource::permission_denied()
            .with_reason(error.code())
            .create()
            .into(),
        A::NoteRequired => invalid("note", "NOTE_REQUIRED").into(),
        A::Empty => invalid("price_ids", "NO_DRAFT_PRICES").into(),
        A::GenerationMismatch { current, .. } => DoorError::Generation { current },
        // The shared engine names its lock conflict for every gear (`ROW_LOCKED_PENDING`);
        // a pending unit holds a Price here, and the door says so.
        A::Locked { .. } => conflict("PRICE_LOCKED_PENDING").into(),
        A::AlreadyDecided | A::DuplicateVote | A::Contended => conflict(error.code()).into(),
        A::Store(detail) if detail.starts_with("DUPLICATE") => conflict("DUPLICATE_VOTE").into(),
        A::Store(detail) => {
            tracing::error!(detail, "pricing approval store failure");
            CanonicalError::internal("pricing approval store failure")
                .create()
                .into()
        }
    }
}
/// The Products registry is not reachable from this process.
pub fn unavailable() -> CanonicalError {
    CanonicalError::service_unavailable()
        .with_detail("REGISTRY_UNAVAILABLE: Products reference registry is unavailable")
        .create()
}
/// A 400 problem that names the unit's current generation for the reviewer's client.
#[must_use]
pub fn generation_problem(code: &str, generation: i32) -> toolkit::api::canonical_prelude::Problem {
    let mut problem = toolkit::api::canonical_prelude::Problem::from(invalid("generation", code));
    problem.context["generation"] = serde_json::json!(generation);
    problem
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
            DoorError::Generation { .. } => invalid("generation", "GENERATION_MISMATCH"),
            DoorError::Repo(RepoError::Conflict { code }) => conflict(code),
            DoorError::Repo(e) => {
                tracing::error!(error=%e,"pricing storage failure");
                Self::internal("pricing storage failure").create()
            }
        }
    }
}
/// A mutation door's refusal when its transaction still meets retryable contention after
/// the toolkit's retries: a lost race the client may retry, never a 500.
pub const CONTENDED: &str = "CONTENDED";
/// The same refusal at an approval-unit door (submit, publish-changes, approve, reject,
/// withdraw), where the design names it `UNIT_CONTENDED` (D-403).
pub const UNIT_CONTENDED: &str = "UNIT_CONTENDED";
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
/// [`transaction`] for an approval-unit door: exhausted contention is `UNIT_CONTENDED`.
/// # Errors
/// Returns the last attempt's refusal or storage failure.
pub async fn unit_transaction<T: Send + 'static>(
    db: &Db,
    work: impl for<'a> FnMut(
        &'a DbTx<'a>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<T, DoorError>> + Send + 'a>,
    > + Send,
) -> Result<T, CanonicalError> {
    transaction_coded(db, UNIT_CONTENDED, work)
        .await
        .map_err(Into::into)
}
/// The serializable retrying transaction, keeping the door's typed refusal.
/// # Errors
/// Returns the last attempt's refusal or storage failure; contention the retries could not
/// clear is `CONTENDED`.
pub async fn transaction_door<T: Send + 'static>(
    db: &Db,
    work: impl for<'a> FnMut(
        &'a DbTx<'a>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<T, DoorError>> + Send + 'a>,
    > + Send,
) -> Result<T, DoorError> {
    transaction_coded(db, CONTENDED, work).await
}
/// [`transaction_door`] for an approval-unit door: exhausted contention is `UNIT_CONTENDED`.
/// # Errors
/// Returns the last attempt's refusal or storage failure.
pub async fn unit_transaction_door<T: Send + 'static>(
    db: &Db,
    work: impl for<'a> FnMut(
        &'a DbTx<'a>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<T, DoorError>> + Send + 'a>,
    > + Send,
) -> Result<T, DoorError> {
    transaction_coded(db, UNIT_CONTENDED, work).await
}
/// Run `work` serializably with the toolkit's contention retries. A driver error the retry
/// classifier still calls contention after the last attempt becomes 409 `code`.
async fn transaction_coded<T: Send + 'static>(
    db: &Db,
    code: &'static str,
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
    .map_err(|error| exhausted_contention(db.backend(), code, error))
}
/// Classify a finished transaction's error: retryable contention is the door's 409 `code`.
#[must_use]
pub fn exhausted_contention(
    backend: sea_orm::DbBackend,
    code: &'static str,
    error: DoorError,
) -> DoorError {
    match &error {
        DoorError::Repo(RepoError::Driver { source, .. })
            if toolkit_db::contention::is_retryable_contention(backend, source) =>
        {
            tracing::warn!(error=%error, code, "pricing transaction contention outlasted its retries");
            conflict(code).into()
        }
        _ => error,
    }
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
/// A command POST that carries no fields: an empty body or `{}`.
/// # Errors
/// Any other body is refused with `BODY_UNEXPECTED`.
pub fn empty_body(body: &[u8]) -> Result<serde_json::Value, CanonicalError> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(serde_json::json!({}));
    }
    let value: serde_json::Value =
        crate::api::rest::preconditions::parse_body(body).map_err(CanonicalError::from)?;
    if value.as_object().is_some_and(serde_json::Map::is_empty) {
        Ok(value)
    } else {
        Err(invalid("body", "BODY_UNEXPECTED"))
    }
}
/// The If-Match token must name the stored version: a stale one is 409 `STALE_REVISION`
/// before anything is written (the code products answers for the same refusal).
///
/// @cpt-dod:cpt-cf-bss-pricing-dod-if-match-version:p1
pub fn check_version(seen: u64, actual: i64) -> Result<(), CanonicalError> {
    if u64::try_from(actual).ok() == Some(seen) {
        Ok(())
    } else {
        Err(conflict("STALE_REVISION"))
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
