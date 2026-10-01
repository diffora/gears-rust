//! `GET /skus` on the toolkit's `OData` pager, and its tab counts `GET /skus/counts` (P-D-210,
//! P-D-211).
//!
//! The list takes `$filter` over [`SkuFilterField`], `$orderby` over [`SkuOrderField`] (tie-break
//! `id`), `$top` (alias `limit`; default 50, clamped at 200) and `cursor` (alias `$skiptoken`),
//! plus `q` and the usage filters `priced` and `in_plan` (P-D-212). Any other key is 400;
//! `$select` and `$count` are refused. The cursor carries a hash of `$filter`, `q`, `priced` and
//! `in_plan`, so a cursor replayed with other values is 400. Each item carries pricing's `usage`
//! from one port call per page, as before (P-D-197); a usage filter adds one call of the port's
//! `usage_sets`, and a filter it cannot answer fails the read (403 or 503), never widening it.
//!
//! The counts take the list's narrowing — `q`, the usage filters and `$filter` with its top-level
//! `lifecycle` terms dropped — and nothing that pages or orders.
//! @cpt-dod:cpt-cf-bss-products-dod-list-search:p1
use super::{
    ApiState, TxError, authz_error_to_canonical, category_tx_config, contention_db_err,
    dto::{ProductsSkuCounts, SkuListItem},
    require_authenticated, tx_to_canonical, usage,
};
use crate::{
    authz::{access_scope, actions, resource_types},
    domain::canonical::{canonical_rendering, content_digest},
    infra::storage::repo::{
        self, SetFilter, SkuListError, SkuListField, SkuListFilter, SkuListMapping,
    },
};
use authz_resolver_sdk::PolicyEnforcer;
use axum::{
    Extension, Json, Router,
    extract::{Query, rejection::QueryRejection},
    http::StatusCode,
};
use bss_products_sdk::sku_usage::SkuUsageSets;
use std::sync::Arc;
use toolkit::api::{
    OpenApiRegistry,
    canonical_prelude::{CanonicalError, resource_error},
    odata::OData,
    operation_builder::{OperationBuilder, OperationBuilderODataExt},
};
use toolkit_db::odata::filter_node_to_condition;
use toolkit_db::secure::AccessScope;
use toolkit_odata::{
    Error as ODataError, ODataQuery, Page,
    ast::Expr,
    errors::OdataError,
    filter::{FieldKind, FilterField, convert_expr_to_filter_node},
};
use toolkit_security::SecurityContext;
use uuid::Uuid;

const SKUS: &str = "/bss-products/v1/skus";
const TAG: &str = "SKUs";
#[resource_error(gts_id!("cf.bss.products.sku.v1~"))]
struct SkuResource;

/// The reason every refused query key carries: the toolkit extractor's own for a `$` option.
pub(super) const UNSUPPORTED: &str = "UNSUPPORTED_QUERY_PARAM";

/// The query string as its raw pairs, in wire order (so every key is seen, repeats included).
pub(super) type RawQuery = Result<Query<Vec<(String, String)>>, QueryRejection>;

/// The fields a list or count `$filter` names: the pager's fields without `updated_at`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkuFilterField {
    Id,
    Code,
    Name,
    Lifecycle,
    Type,
    CategoryId,
    PendingUnitId,
    RetirePending,
}
impl SkuFilterField {
    const fn field(self) -> SkuListField {
        match self {
            Self::Id => SkuListField::Id,
            Self::Code => SkuListField::Code,
            Self::Name => SkuListField::Name,
            Self::Lifecycle => SkuListField::Lifecycle,
            Self::Type => SkuListField::Type,
            Self::CategoryId => SkuListField::CategoryId,
            Self::PendingUnitId => SkuListField::PendingUnitId,
            Self::RetirePending => SkuListField::RetirePending,
        }
    }
}
impl FilterField for SkuFilterField {
    const FIELDS: &'static [Self] = &[
        Self::Id,
        Self::Code,
        Self::Name,
        Self::Lifecycle,
        Self::Type,
        Self::CategoryId,
        Self::PendingUnitId,
        Self::RetirePending,
    ];
    fn name(&self) -> &'static str {
        self.field().name()
    }
    fn kind(&self) -> FieldKind {
        self.field().kind()
    }
    /// `category_id` and `pending_unit_id` (P-D-210).
    fn nullable(&self) -> bool {
        self.field().nullable()
    }
    fn from_name(name: &str) -> Option<Self> {
        Self::FIELDS.iter().copied().find(|f| f.name() == name)
    }
}
/// The fields a list `$orderby` names: never a nullable or a filter-only field (ledger's
/// `ExceptionOrderField` pattern). `id` is the tie-break every order ends with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkuOrderField {
    Code,
    Name,
    UpdatedAt,
    Id,
}
impl SkuOrderField {
    const fn field(self) -> SkuListField {
        match self {
            Self::Code => SkuListField::Code,
            Self::Name => SkuListField::Name,
            Self::UpdatedAt => SkuListField::UpdatedAt,
            Self::Id => SkuListField::Id,
        }
    }
}
impl FilterField for SkuOrderField {
    const FIELDS: &'static [Self] = &[Self::Code, Self::Name, Self::UpdatedAt, Self::Id];
    fn name(&self) -> &'static str {
        self.field().name()
    }
    fn kind(&self) -> FieldKind {
        self.field().kind()
    }
    fn from_name(name: &str) -> Option<Self> {
        Self::FIELDS.iter().copied().find(|f| f.name() == name)
    }
}

/// Register the list and the counts; both read under `sku × read`.
pub(crate) fn register(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::get(SKUS)
        .operation_id("bss_products.list_skus")
        .summary("List, filter and search SKUs")
        .description(
            "One page of the tenant's SKUs (P-D-210). OData `$filter` over id, code, name, \
             lifecycle, retire_pending, type, category_id (`eq null`: no category) and \
             pending_unit_id (`ne null`: in review); `$orderby` over code, name, updated_at (tie-break id; default \
             code); `$top` (alias `limit`; default 50, clamped at 200) and `cursor` (alias \
             `$skiptoken`) from `page_info`. `q` is a case-insensitive substring of the code, \
             name, unit, usage type or GL code, matched literally (ASCII case folding on SQLite). \
             `priced` and `in_plan` (true or false) keep the SKUs pricing prices or a plan \
             names, or the others (P-D-212): 403 USAGE_FORBIDDEN when pricing refuses the \
             caller, 503 USAGE_UNAVAILABLE when it cannot answer. Any other key, `$select` and \
             `$count` are 400; a cursor replayed with another `$filter`, `q`, `priced` or \
             `in_plan` is 400. Each item carries pricing's `usage`, or null (P-D-197).",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .query_param_typed(
            "limit",
            false,
            "Page size, alias of $top (default 50, clamped at 200)",
            "integer",
        )
        .query_param_typed(
            "cursor",
            false,
            "Continuation from page_info (alias $skiptoken)",
            "string",
        )
        .query_param_typed(
            "q",
            false,
            "Case-insensitive substring of the code, name, unit, usage type or GL code",
            "string",
        )
        .query_param_typed(
            "priced",
            false,
            "true: SKUs pricing has an entry for; false: the others",
            "boolean",
        )
        .query_param_typed(
            "in_plan",
            false,
            "true: SKUs a live plan names through an entry; false: the others",
            "boolean",
        )
        .handler(list_skus)
        .with_odata_filter::<SkuFilterField>()
        .with_odata_orderby::<SkuOrderField>()
        .json_response_with_schema::<Page<SkuListItem>>(
            openapi,
            StatusCode::OK,
            "One page of SKUs.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    OperationBuilder::get(format!("{SKUS}/counts"))
        .operation_id("bss_products.count_skus")
        .summary("Count SKUs by lifecycle and in review")
        .description(
            "The list's tab counts (P-D-211): every SKU, each lifecycle, and those in review \
             (`pending_unit_id` set), narrowed like the list by `q`, `priced`, `in_plan` and \
             `$filter`, whose top-level `lifecycle` terms are dropped (a `lifecycle` term under \
             `or` or `not` is 400). `$orderby`, `$top`/`limit`, `cursor`/`$skiptoken` and \
             `$select` are 400.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .query_param_typed(
            "q",
            false,
            "Case-insensitive substring of the code, name, unit, usage type or GL code",
            "string",
        )
        .query_param_typed(
            "priced",
            false,
            "true: SKUs pricing has an entry for; false: the others",
            "boolean",
        )
        .query_param_typed(
            "in_plan",
            false,
            "true: SKUs a live plan names through an entry; false: the others",
            "boolean",
        )
        .handler(count_skus)
        .with_odata_filter::<SkuFilterField>()
        .json_response_with_schema::<ProductsSkuCounts>(openapi, StatusCode::OK, "The SKU counts.")
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi)
}

/// Read under `sku × read`, as every SKU read.
async fn read_scope(
    enforcer: &PolicyEnforcer,
    ctx: &SecurityContext,
) -> Result<AccessScope, CanonicalError> {
    access_scope(enforcer, ctx, &resource_types::SKU, actions::READ, None)
        .await
        .map_err(|e| {
            authz_error_to_canonical(e, |reason| {
                SkuResource::permission_denied()
                    .with_reason(reason)
                    .create()
            })
        })
}

/// A 400 naming each offending key.
pub(super) fn refused(keys: &[(&str, String, &'static str)]) -> Result<(), CanonicalError> {
    let mut keys = keys.iter();
    let Some((key, detail, reason)) = keys.next() else {
        return Ok(());
    };
    let mut error = OdataError::invalid_argument().with_field_violation(*key, detail, *reason);
    for (key, detail, reason) in keys {
        error = error.with_field_violation(*key, detail, *reason);
    }
    Err(error.create())
}

/// What the list and the counts read besides the `OData` options.
#[derive(Debug, Clone, Default)]
pub(crate) struct ListParams {
    pub q: Option<String>,
    pub priced: Option<bool>,
    pub in_plan: Option<bool>,
}
impl ListParams {
    /// Whether a usage filter asks pricing's sets.
    const fn filters_usage(&self) -> bool {
        self.priced.is_some() || self.in_plan.is_some()
    }
}

/// The `$` options the list takes, as the extractor binds them (`limit` and `cursor` are their
/// aliases).
const LIST_OPTIONS: &[&str] = &["$filter", "$orderby", "$top", "$skiptoken"];

/// The query keys a door takes besides the extractor's: `plain` without a `$`, and — when
/// `dollar` is given — the only `$` options it takes (otherwise the extractor polices them).
/// The rest is 400, every offender at once (a products copy of ledger's
/// `reject_non_odata_list_params_allowing`). A plain key given twice is 400. An empty `q` is no
/// search. The SKU history and the category list reuse it (P-D-213, P-D-215).
pub(super) fn params(
    query: RawQuery,
    plain: &[&str],
    dollar: Option<&[&str]>,
) -> Result<ListParams, CanonicalError> {
    let Query(pairs) = query.map_err(|e| {
        OdataError::invalid_argument()
            .with_field_violation("query", e.body_text(), "INVALID_QUERY_PARAMS")
            .create()
    })?;
    let takes = plain
        .iter()
        .chain(dollar.unwrap_or(LIST_OPTIONS))
        .map(|k| format!("`{k}`"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut offenders: Vec<(&str, String, &'static str)> = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for (key, _) in &pairs {
        let key = key.as_str();
        let allowed = if key.starts_with('$') {
            dollar.is_none_or(|d| d.contains(&key))
        } else {
            plain.contains(&key)
        };
        if offenders.iter().any(|(k, ..)| *k == key) {
            continue;
        }
        if !allowed {
            offenders.push((
                key,
                format!("`{key}` is not a parameter of this read; it takes {takes}"),
                UNSUPPORTED,
            ));
        } else if !key.starts_with('$') && seen.contains(&key) {
            offenders.push((
                key,
                format!("`{key}` is given more than once"),
                "INVALID_QUERY_PARAMS",
            ));
        } else {
            seen.push(key);
        }
    }
    refused(&offenders)?;
    let value = |name: &str| {
        pairs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    };
    let mut malformed: Vec<(&str, String, &'static str)> = Vec::new();
    let mut flag = |name: &'static str| match value(name).as_deref() {
        None => None,
        Some("true") => Some(true),
        Some("false") => Some(false),
        Some(other) => {
            malformed.push((
                name,
                format!("`{name}` is `true` or `false`, not `{other}`"),
                "INVALID_QUERY_PARAMS",
            ));
            None
        }
    };
    let (priced, in_plan) = (flag("priced"), flag("in_plan"));
    refused(&malformed)?;
    Ok(ListParams {
        q: value("q").filter(|q| !q.is_empty()),
        priced,
        in_plan,
    })
}

/// The `$filter`, checked the way the pager will read it: only [`SkuFilterField`]s (`null` only
/// on a nullable one), and each value through the mapping (a closed value). The condition it
/// becomes, for a count.
fn checked_filter(filter: Option<&Expr>) -> Result<Option<sea_orm::Condition>, CanonicalError> {
    let Some(expr) = filter else {
        return Ok(None);
    };
    convert_expr_to_filter_node::<SkuFilterField>(expr)
        .map_err(|e| ODataError::InvalidFilter(e.to_string()))?;
    let node = convert_expr_to_filter_node::<SkuListField>(expr)
        .map_err(|e| ODataError::InvalidFilter(e.to_string()))?;
    let condition = filter_node_to_condition::<SkuListField, SkuListMapping>(&node)
        .map_err(ODataError::InvalidFilter)?;
    Ok(Some(condition))
}

/// The cursor's filter hash over everything that narrows the list: the extractor's hash of
/// `$filter`, `q`, `priced` and `in_plan`.
fn list_hash(odata: &ODataQuery, params: &ListParams) -> String {
    cursor_hash(&serde_json::json!({
        "filter": odata.filter_hash,
        "q": params.q,
        "priced": params.priced,
        "in_plan": params.in_plan,
    }))
}

/// A cursor's filter hash over `narrowing`: the first 8 bytes of the SHA-256 of its canonical
/// rendering, as hex. A cursor replayed under another narrowing is 400 `FILTER_MISMATCH`.
pub(super) fn cursor_hash(narrowing: &serde_json::Value) -> String {
    content_digest(&canonical_rendering(narrowing))
        .iter()
        .take(8)
        .fold(String::with_capacity(16), |mut hex, b| {
            const DIGITS: &[u8; 16] = b"0123456789abcdef";
            hex.push(char::from(DIGITS[usize::from(b >> 4)]));
            hex.push(char::from(DIGITS[usize::from(b & 0x0f)]));
            hex
        })
}

/// The narrowing the repository applies before `$filter`: `q`, and each usage filter against
/// pricing's sets (asked once, only when a usage filter is given).
async fn list_filter(
    state: &ApiState,
    ctx: &SecurityContext,
    params: &ListParams,
) -> Result<SkuListFilter, CanonicalError> {
    let sets = if params.filters_usage() {
        usage::sets(state, ctx).await?
    } else {
        SkuUsageSets::default()
    };
    let set = |member: Option<bool>, ids: &[Uuid]| {
        member.map(|member| SetFilter {
            member,
            ids: ids.to_vec(),
        })
    };
    Ok(SkuListFilter {
        text: params.q.clone(),
        priced: set(params.priced, &sets.priced),
        in_plan: set(params.in_plan, &sets.in_plan),
    })
}

/// @cpt-cf-bss-products-fr-read-model
async fn list_skus(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    query: RawQuery,
    odata: Result<OData, CanonicalError>,
) -> Result<Json<Page<SkuListItem>>, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    // Authorization first, then the query (a 403 before a 400).
    let scope = read_scope(&enforcer, &ctx).await?;
    let params = params(query, &["limit", "cursor", "q", "priced", "in_plan"], None)?;
    let OData(mut odata) = odata?;
    if odata.select.is_some() {
        refused(&[(
            "$select",
            "a SKU list item is not projected; drop `$select`".to_owned(),
            UNSUPPORTED,
        )])?;
    }
    checked_filter(odata.filter.as_deref())?;
    for key in &odata.order.0 {
        if SkuOrderField::from_name(&key.field).is_none() {
            return Err(ODataError::InvalidOrderByField(key.field.clone()).into());
        }
    }
    let hash = list_hash(&odata, &params);
    if let Some(cursor) = &odata.cursor
        && cursor.f.as_deref() != Some(hash.as_str())
    {
        return Err(ODataError::FilterMismatch.into());
    }
    odata.filter_hash = Some(hash);
    // The query is valid; now pricing's sets, if a usage filter needs them (P-D-212).
    let filter = list_filter(&state, &ctx, &params).await?;
    let tenant = ctx.subject_tenant_id();
    let ttl = state.fence_ttl_minutes;
    let backend = state.db.db().backend();
    let page = state
        .db
        .db()
        .transaction_with_retry(category_tx_config(&state), contention_db_err, move |tx| {
            let (scope, filter, odata) = (scope.clone(), filter.clone(), odata.clone());
            Box::pin(async move {
                expire(tx, &scope, tenant, ttl).await?;
                repo::page_skus(tx, &scope, tenant, backend, &filter, &odata)
                    .await
                    .map_err(|e| match e {
                        SkuListError::Query(e) => TxError::OData(e),
                        SkuListError::Repo(e) => TxError::Repo(e),
                    })
            })
        })
        .await
        .map_err(tx_to_canonical)?;
    // P-D-197: one call of pricing's usage port for the page, after the page's transaction.
    let ids: Vec<Uuid> = page.items.iter().map(|s| s.id).collect();
    let mut usage = super::usage::of(&state, &ctx, &ids).await;
    Ok(Json(Page {
        items: page
            .items
            .into_iter()
            .map(|s| {
                let counted = usage.remove(&s.id);
                SkuListItem {
                    sku: s.into(),
                    usage: counted,
                }
            })
            .collect(),
        page_info: page.page_info,
    }))
}

/// Recover the tenant's orphan fences before a read, in the read's transaction, so the list
/// and the counts agree on a pending retire (P-D-248: `in_review`, the lifecycle unchanged); each fence lifted is the system's act, with its
/// audit row (P-D-213). Set-based (P-D-211): the fences, one lift, one insert of their rows —
/// the same statements for one expired fence as for fifty; one read when there is none.
async fn expire(
    tx: &impl toolkit_db::secure::DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    ttl: u32,
) -> Result<(), TxError> {
    let now = crate::infra::storage::stored_now();
    let expired = repo::expire_orphan_fences(
        tx,
        scope,
        tenant,
        now - time::Duration::minutes(i64::from(ttl)),
    )
    .await
    .map_err(TxError::Repo)?;
    super::governance::expiry_audits(tx, tenant, &expired, ttl, now).await
}

/// Whether `expr` names `lifecycle` anywhere.
fn names_lifecycle(expr: &Expr) -> bool {
    match expr {
        Expr::Identifier(name) => name == SkuListField::Lifecycle.name(),
        Expr::Value(_) => false,
        Expr::And(a, b) | Expr::Or(a, b) | Expr::Compare(a, _, b) => {
            names_lifecycle(a) || names_lifecycle(b)
        }
        Expr::Not(inner) => names_lifecycle(inner),
        Expr::In(inner, list) => names_lifecycle(inner) || list.iter().any(names_lifecycle),
        Expr::Function(_, args) => args.iter().any(names_lifecycle),
    }
}

/// `expr` without its top-level `and` conjuncts that name `lifecycle`: the counts count every
/// lifecycle. A `lifecycle` term anywhere else (under `or` or `not`) cannot be dropped without
/// changing what the rest means, so it is refused.
fn without_lifecycle(expr: &Expr) -> Result<Option<Expr>, ODataError> {
    match expr {
        Expr::And(a, b) => Ok(match (without_lifecycle(a)?, without_lifecycle(b)?) {
            (Some(a), Some(b)) => Some(Expr::And(Box::new(a), Box::new(b))),
            (one, None) | (None, one) => one,
        }),
        term if !names_lifecycle(term) => Ok(Some(term.clone())),
        Expr::Or(..) | Expr::Not(..) => Err(ODataError::InvalidFilter(
            "the counts drop `lifecycle` only from top-level `and` terms; a `lifecycle` term \
             under `or` or `not` is not counted"
                .to_owned(),
        )),
        _ => Ok(None),
    }
}

/// @cpt-cf-bss-products-fr-read-model
async fn count_skus(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    query: RawQuery,
    odata: Result<OData, CanonicalError>,
) -> Result<Json<ProductsSkuCounts>, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let scope = read_scope(&enforcer, &ctx).await?;
    let params = params(query, &["q", "priced", "in_plan"], Some(&["$filter"]))?;
    let OData(odata) = odata?;
    // The whole filter is checked as the list would read it, then its lifecycle terms go.
    checked_filter(odata.filter.as_deref())?;
    let condition = match odata.filter.as_deref() {
        Some(expr) => match without_lifecycle(expr)? {
            Some(rest) => checked_filter(Some(&rest))?,
            None => None,
        },
        None => None,
    };
    let filter = list_filter(&state, &ctx, &params).await?;
    let tenant = ctx.subject_tenant_id();
    let ttl = state.fence_ttl_minutes;
    let backend = state.db.db().backend();
    let counts = state
        .db
        .db()
        .transaction_with_retry(category_tx_config(&state), contention_db_err, move |tx| {
            let (scope, filter, condition) = (scope.clone(), filter.clone(), condition.clone());
            Box::pin(async move {
                expire(tx, &scope, tenant, ttl).await?;
                repo::count_skus(tx, &scope, tenant, backend, &filter, condition)
                    .await
                    .map_err(TxError::Repo)
            })
        })
        .await
        .map_err(tx_to_canonical)?;
    Ok(Json(counts.into()))
}

#[cfg(test)]
#[path = "sku_list_tests.rs"]
mod sku_list_tests;
