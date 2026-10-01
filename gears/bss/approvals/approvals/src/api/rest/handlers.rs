//! The list, the counts, the card and the three vote doors.

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Extension, Path, Query};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use bss_approvals_sdk::{VoteAction, VoteRequest};
use serde::Deserialize;
use toolkit_canonical_errors::CanonicalError;
use toolkit_odata::Error as ODataError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use super::dto::{CountsDto, UnitDto, UnitListDto};
use crate::api::ApiState;
use crate::domain::error;
use crate::domain::query::{self, ListParams};
use crate::domain::read;

const IDEMPOTENCY_KEY: &str = "Idempotency-Key";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ListQuery {
    state: Option<String>,
    kind: Option<String>,
    ref_id: Option<Uuid>,
    book_id: Option<Uuid>,
    limit: Option<u64>,
    cursor: Option<String>,
    #[serde(rename = "$orderby")]
    orderby: Option<String>,
    impact: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CountsQuery {
    state: Option<String>,
    kind: Option<String>,
    ref_id: Option<Uuid>,
    book_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CardQuery {
    impact: Option<bool>,
}

pub(super) async fn list_units(
    Extension(state): Extension<Arc<ApiState>>,
    ctx: Option<Extension<SecurityContext>>,
    query: Result<Query<ListQuery>, QueryRejection>,
) -> Result<Json<UnitListDto>, CanonicalError> {
    let ctx = caller(ctx)?;
    let Query(query) = bad_query(query)?;
    let prepared = query::prepare_list(&ListParams {
        state: query.state,
        kind: query.kind,
        ref_id: query.ref_id,
        book_id: query.book_id,
        limit: query.limit,
        cursor: query.cursor,
        orderby: query.orderby,
        impact: query.impact,
    })?;
    let listed = read::list_page(&state.hub, &state.sources, &ctx, &prepared).await?;
    Ok(Json(UnitListDto {
        items: listed.units.into_iter().map(Into::into).collect(),
        next_cursor: listed.next_cursor,
        sources: listed.sources.into_iter().map(Into::into).collect(),
    }))
}

pub(super) async fn count_units(
    Extension(state): Extension<Arc<ApiState>>,
    ctx: Option<Extension<SecurityContext>>,
    query: Result<Query<CountsQuery>, QueryRejection>,
) -> Result<Json<CountsDto>, CanonicalError> {
    let ctx = caller(ctx)?;
    let Query(query) = bad_query(query)?;
    let params = ListParams {
        state: query.state,
        kind: query.kind,
        ref_id: query.ref_id,
        book_id: query.book_id,
        ..ListParams::default()
    };
    let counted = read::count_all(
        &state.hub,
        &state.sources,
        &ctx,
        &query::narrowing_of(&params),
    )
    .await?;
    Ok(Json(CountsDto::from_counts(
        counted.counts,
        counted.sources,
    )))
}

pub(super) async fn get_unit(
    Extension(state): Extension<Arc<ApiState>>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
    query: Result<Query<CardQuery>, QueryRejection>,
) -> Result<Json<UnitDto>, CanonicalError> {
    let ctx = caller(ctx)?;
    let Query(query) = bad_query(query)?;
    let unit = read::get_unit(
        &state.hub,
        &state.sources,
        &ctx,
        id,
        query.impact.unwrap_or(true),
    )
    .await?;
    Ok(Json(unit.into()))
}

pub(super) async fn approve(
    state: Extension<Arc<ApiState>>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    vote(state, ctx, id, VoteAction::Approve, headers, body).await
}

pub(super) async fn reject(
    state: Extension<Arc<ApiState>>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    vote(state, ctx, id, VoteAction::Reject, headers, body).await
}

pub(super) async fn withdraw(
    state: Extension<Arc<ApiState>>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    vote(state, ctx, id, VoteAction::Withdraw, headers, body).await
}

async fn vote(
    Extension(state): Extension<Arc<ApiState>>,
    ctx: Option<Extension<SecurityContext>>,
    id: Uuid,
    action: VoteAction,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let ctx = match caller(ctx) {
        Ok(ctx) => ctx,
        Err(err) => return err.into_response(),
    };
    let request = match vote_request(&headers, body) {
        Ok(request) => request,
        Err(err) => return err.into_response(),
    };
    match read::vote_unit(&state.hub, &state.sources, &ctx, id, action, request).await {
        Ok(answered) => pass_through(answered),
        Err(err) => err.into_response(),
    }
}

fn vote_request(headers: &HeaderMap, body: Bytes) -> Result<VoteRequest, CanonicalError> {
    let idempotency_key = match headers.get(IDEMPOTENCY_KEY) {
        None => None,
        Some(value) => Some(
            value
                .to_str()
                .map_err(|_| ODataError::InvalidFilter("Idempotency-Key is not valid text".into()))?
                .to_owned(),
        ),
    };
    Ok(VoteRequest {
        body: Vec::from(body),
        idempotency_key,
    })
}

fn pass_through(answered: bss_approvals_sdk::VoteResponse) -> Response {
    let Ok(status) = StatusCode::from_u16(answered.status) else {
        return CanonicalError::internal(
            "bss-approvals: a source vote returned a status that is not an HTTP status",
        )
        .create()
        .into_response();
    };
    let mut response = (status, answered.body).into_response();
    for (name, value) in answered.headers {
        let Ok(name) = HeaderName::try_from(name) else {
            return CanonicalError::internal(
                "bss-approvals: a source vote returned a header name that is not valid",
            )
            .create()
            .into_response();
        };
        let Ok(value) = HeaderValue::try_from(value) else {
            return CanonicalError::internal(
                "bss-approvals: a source vote returned a header value that is not valid",
            )
            .create()
            .into_response();
        };
        response.headers_mut().insert(name, value);
    }
    response
}

fn caller(ctx: Option<Extension<SecurityContext>>) -> Result<SecurityContext, CanonicalError> {
    let Some(Extension(ctx)) = ctx else {
        return Err(error::unauthenticated());
    };
    if ctx.subject_id().is_nil() || ctx.subject_tenant_id().is_nil() || ctx.subject_type().is_none()
    {
        return Err(error::unauthenticated());
    }
    Ok(ctx)
}

fn bad_query<T>(query: Result<Query<T>, QueryRejection>) -> Result<Query<T>, CanonicalError> {
    query.map_err(|_| ODataError::InvalidFilter("the query did not parse".into()).into())
}
