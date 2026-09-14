//! The per-value door family the two single-table vocabularies share —
//! `GET/POST/PATCH …/config/{rounding-policies|gl-codes}[/values[/{value}]]`.
//!
//! # Why this module exists rather than a fifth and sixth `TaxonomyClass`
//!
//! D-353 gave the four scope classes `POST …/values`, `GET/PATCH
//! …/values/{value}` and took their whole-set `PUT` away, on three arguments
//! that were never about those four tables: a whole-set write cannot say
//! *which* value moved in its audit record; two admins editing two different
//! values refuse each other on one set tag; and a client that saves a filtered
//! list retires every value it did not show. All three hold word for word on
//! `pricing_rounding_policy_taxonomy` (D-334) and `pricing_gl_code_taxonomy`
//! (D-356).
//!
//! What could **not** be shared is the class vocabulary.
//! [`TaxonomyClass::scope_class`](crate::domain::taxonomy::TaxonomyClass::scope_class)
//! is a total function into
//! [`ScopeClass`](crate::domain::overlay::ScopeClass), so every member of that
//! enum asserts *an overlay may be scoped by this* — false of a GL code, and
//! not merely documentary: the assertion reaches
//! `pricing_price_overlay.scope_class`'s `CHECK`, which has no such token.
//! [`VocabularyClass`] is the parallel enum, and this module is the door
//! shape over it.
//!
//! # The routes stay where they are
//!
//! Each vocabulary keeps its own path — `/config/rounding-policies`,
//! `/config/gl-codes` — and gains `/values` and `/values/{value}` under it.
//! They are **not** folded onto `/config/taxonomies/{class}`: that segment is
//! the overlay scope token the plane stores (D-241), and giving it two tokens
//! that no overlay may carry would make the segment mean two things. The
//! registrations therefore live in each vocabulary's own module, where
//! `OperationBuilder` sees the literal path DE0801 requires; only the
//! handlers' bodies are here.
//!
//! # No approval unit on any edge
//!
//! D-334 and D-356 both say it and both give the same reason: a vocabulary
//! *narrows* what may be authored, so every edit can only make publishing
//! harder. There is no `202` arm here, no content pin, and no unit to
//! re-derive — the shape that separates this door from
//! [`taxonomies`](crate::api::rest::taxonomies)'s, whose values a published
//! row may resolve *through* (D-355). What both doors do share is that a
//! retirement is **guarded**: a value something published still names is
//! `409` `TAXONOMY_VALUE_IN_USE`, and nothing is written.

use std::sync::Arc;

use axum::http::StatusCode;
use toolkit::api::canonical_prelude::CanonicalError;
use toolkit::api::operation_builder::{ParamLocation, ParamSpec};
use toolkit_db::secure::AccessScope;
use toolkit_security::SecurityContext;

use crate::api::rest::auth_context::audit_stamp;
use crate::api::rest::error::authz_error_to_canonical;
use crate::api::rest::preconditions;
use crate::api::rest::state::AuthoringState;
use crate::domain::concurrency::PolicyTag;
use crate::domain::error::DomainError;
use crate::domain::overlay::ScopeValue;
use crate::domain::taxonomy::{
    TAXONOMY_VALUE_IN_USE, TaxonomyEntry, TaxonomyState, TaxonomyValuePatch, VocabularyClass,
};
use crate::infra::storage::repo::taxonomy_repo::{self, Declared, ValuePatched};
use crate::infra::storage::repo_failure;
use time::OffsetDateTime;
use uuid::Uuid;

/// The one value a `POST …/values` declares — the authored fields and nothing
/// else.
///
/// Its own type rather than the class's set-`GET` view, for
/// `taxonomies::DeclareTaxonomyValueRequest`'s reason: a request type that
/// carries read-only members advertises settable properties the server
/// discards, and *"a field that vanishes silently reads to the operator
/// exactly like one that failed to save"*. One type for both vocabularies
/// because the authored columns are the same three — see [`VocabularyClass`],
/// whose whole premise is that these two tables have one shape.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request)]
#[serde(deny_unknown_fields)]
pub struct DeclareVocabularyValueRequest {
    /// The declared code. Never blank.
    pub value: String,
    /// The operator's label for it.
    pub display_name: String,
    /// `active` or `retired`, defaulting to `active`.
    pub state: Option<String>,
}

/// The body of a `PATCH …/values/{value}`: only the fields to change.
///
/// **No tax markers, and they are unsayable rather than refused.** D-01 gives
/// `taxCategory` and `taxRatePresent` to the region taxonomy alone and neither
/// of these tables has the columns; a member absent from the request type is
/// refused by the parse, which is the strongest form of the refusal
/// `taxonomies::authored_patch` has to write by hand for the three non-region
/// classes.
#[derive(Debug, Clone, Default)]
#[toolkit_macros::api_dto(request)]
#[serde(deny_unknown_fields)]
pub struct PatchVocabularyValueRequest {
    /// A new label. Absent leaves the held one.
    pub display_name: Option<String>,
    /// `active` or `retired`. A retirement is guarded
    /// (`TAXONOMY_VALUE_IN_USE`); `retired -> active` re-activates.
    pub state: Option<String>,
}

/// The `{value}` path parameter — [`taxonomies::value_param`]'s text over a
/// vocabulary.
///
/// [`taxonomies::value_param`]: crate::api::rest::taxonomies
#[must_use]
pub fn value_param() -> ParamSpec {
    ParamSpec {
        name: "value".to_owned(),
        location: ParamLocation::Path,
        required: true,
        description: Some(
            "The declared code, exactly as the vocabulary lists it. Retired values are still \
             addressable: retirement is a state, not a deletion, and `PATCH` with `state: \
             active` is the way back."
                .to_owned(),
        ),
        param_type: "string".to_owned(),
        // Scalar: every parameter this gear declares is single-valued.
        array: false,
    }
}

/// The `If-Match` header the per-value `PATCH` requires, asserting the
/// **value's own** tag rather than the set's.
#[must_use]
pub fn if_match_value_param() -> ParamSpec {
    ParamSpec {
        name: "If-Match".to_owned(),
        location: ParamLocation::Header,
        required: true,
        description: Some(
            "Mandatory precondition (RFC 9110). The value is the **opaque** tag `GET \
             .../values/{value}` returns in its `ETag` header - copy it back verbatim. It \
             digests this one value's code, state and label, so it moves when this value \
             changes and **not** when a sibling does: two admins editing two different values \
             do not refuse each other, which is the reason this route exists. The set's tag \
             from the collection `GET` does not satisfy it. A tag that no longer describes the \
             value is `409` `STALE_VERSION`; an absent or malformed one is `400`."
                .to_owned(),
        ),
        param_type: "string".to_owned(),
        array: false,
    }
}

/// One value's entity tag, rendered for the wire. **The only producer.**
#[must_use]
pub fn value_tag(class: VocabularyClass, entry: &TaxonomyEntry) -> String {
    preconditions::policy_etag(&taxonomy_repo::vocabulary_value_tag_of(class, entry))
}

/// Where a freshly declared value lives — the `Location` a `201` carries.
#[must_use]
pub fn value_location(class: VocabularyClass, entry: &TaxonomyEntry) -> String {
    format!(
        "/bss-pricing/v1/config/{}/values/{}",
        class.resource(),
        entry.value.as_str()
    )
}

/// `GET …/values/{value}` — one declared value, or the 404 for one the tenant
/// never declared.
///
/// # Errors
///
/// The `config × read` gate's refusal, a blank segment (`400`), a value the
/// tenant never declared (`404`), or a storage failure.
pub async fn read_value(
    state: &Arc<AuthoringState>,
    scope: &AccessScope,
    tenant: Uuid,
    class: VocabularyClass,
    segment: &str,
) -> Result<TaxonomyEntry, CanonicalError> {
    // The caller gated first — `taxonomies::get_taxonomy`'s ordering: a caller
    // who may not read this resource is told that, rather than being told
    // their path segment is unknown.
    let value = parse_value(segment)?;
    let conn = state.db.conn().map_err(|e| {
        CanonicalError::internal(format!("bss-pricing: {class} value lookup: {e}")).create()
    })?;
    taxonomy_repo::find_vocabulary_value_on(&conn, scope, tenant, class, &value)
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?
        .ok_or_else(|| value_not_found(class, &value))
}

/// `POST …/values` — declare one value.
///
/// Answers the value as stored and the status the outcome earns: `201` on a
/// create, `200` on the create's replay. A body naming a held value with other
/// content is `409` `TAXONOMY_VALUE_EXISTS` pointing at the `PATCH`.
///
/// # Errors
///
/// The `config × write` gate's refusal, an unparseable body (`400`), a value
/// the tenant already declares with other content (`409`), or a storage
/// failure.
pub async fn declare_value(
    state: &Arc<AuthoringState>,
    scope: &AccessScope,
    ctx: &SecurityContext,
    correlation: uuid::Uuid,
    class: VocabularyClass,
    body: &axum::body::Bytes,
) -> Result<(TaxonomyEntry, StatusCode), CanonicalError> {
    let request: DeclareVocabularyValueRequest = preconditions::parse_body(body)?;
    let entry = authored_entry(class, request)?;

    let declared = state
        .taxonomies
        .declare_vocabulary_value(
            scope,
            ctx.subject_tenant_id(),
            class,
            entry,
            audit_stamp(ctx, OffsetDateTime::now_utc(), correlation),
        )
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?;
    match declared {
        Declared::Created(entry) => Ok((entry, StatusCode::CREATED)),
        Declared::Replayed(entry) => Ok((entry, StatusCode::OK)),
        Declared::Exists(existing) => Err(CanonicalError::from(DomainError::TaxonomyValueExists(
            format!(
                "`{}` is already declared in the {class} vocabulary ({}); a value is its own \
                 key, so declare it once and edit it with PATCH {}",
                existing.value,
                existing.state,
                value_location(class, &existing)
            ),
        ))),
    }
}

/// `PATCH …/values/{value}` — edit one declared value, under its own tag.
///
/// Commits at once: neither vocabulary opens an approval unit on any edge
/// (D-334, D-356), so there is no `202` arm. A body that changes nothing is
/// the value as it stands and writes nothing.
///
/// # Errors
///
/// The `config × write` gate's refusal, an absent or malformed `If-Match`
/// (`400`), a value the tenant never declared (`404`), a tag that no longer
/// describes it (`409` `STALE_VERSION`), a guarded retirement (`409`
/// `TAXONOMY_VALUE_IN_USE`), or a storage failure.
#[allow(
    clippy::too_many_arguments,
    reason = "the request's own operands — the axum extractors the two calling handlers hold, \
              passed through rather than re-bundled into a struct that would name nothing the \
              handler signature does not already say"
)]
pub async fn patch_value(
    state: &Arc<AuthoringState>,
    scope: &AccessScope,
    ctx: &SecurityContext,
    correlation: uuid::Uuid,
    class: VocabularyClass,
    segment: &str,
    asserted: &PolicyTag,
    body: &axum::body::Bytes,
) -> Result<TaxonomyEntry, CanonicalError> {
    let tenant = ctx.subject_tenant_id();
    let value = parse_value(segment)?;
    let request: PatchVocabularyValueRequest = preconditions::parse_body(body)?;
    let patch = authored_patch(&value, request)?;

    let conn = state.db.conn().map_err(|e| {
        CanonicalError::internal(format!("bss-pricing: {class} value lookup: {e}")).create()
    })?;
    let held = taxonomy_repo::find_vocabulary_value_on(&conn, scope, tenant, class, &value)
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?
        .ok_or_else(|| value_not_found(class, &value))?;
    if taxonomy_repo::vocabulary_value_tag_of(class, &held) != *asserted {
        return Err(stale_value(class, &value));
    }
    let next = patch.apply(&held);
    if next == held {
        // Not an act: nothing written, no audit record — the value as it
        // stands, under its tag.
        return Ok(held);
    }

    // Judged here as well as inside the write transaction, for
    // `taxonomies::patch_taxonomy_value`'s reason: a refusal an operator can
    // act on is worth one read on the way in, and the transaction's own
    // re-judge is what makes it sound.
    let report =
        taxonomy_repo::judge_vocabulary_value_patch(&conn, scope, tenant, class, &held, &next)
            .await
            .map_err(|e| CanonicalError::from(repo_failure(&e)))?;
    if let Some(violation) = report.violations.first() {
        debug_assert_eq!(violation.code, TAXONOMY_VALUE_IN_USE);
        return Err(CanonicalError::from(DomainError::TaxonomyValueInUse(
            violation.detail.clone(),
        )));
    }

    let patched = state
        .taxonomies
        .patch_vocabulary_value(
            scope,
            tenant,
            class,
            held,
            next,
            audit_stamp(ctx, OffsetDateTime::now_utc(), correlation),
        )
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?;
    match patched {
        ValuePatched::Committed(entry) => Ok(*entry),
        ValuePatched::Stale => Err(stale_value(class, &value)),
        ValuePatched::Refused(report) => {
            let detail = report.violations.first().map_or_else(
                || {
                    format!(
                        "`{value}` cannot be retired from the {class} vocabulary while \
                         something published still names it"
                    )
                },
                |violation| violation.detail.clone(),
            );
            Err(CanonicalError::from(DomainError::TaxonomyValueInUse(
                detail,
            )))
        }
    }
}

/// The 409 a moved tag earns, at the door and again from the transaction.
fn stale_value(class: VocabularyClass, value: &ScopeValue) -> CanonicalError {
    CanonicalError::from(DomainError::StaleVersion(format!(
        "the If-Match tag no longer describes `{value}` in the {class} vocabulary: it changed \
         after you read it. Re-read GET {} and author against the tag it hands back",
        format_args!(
            "/bss-pricing/v1/config/{}/values/{}",
            class.resource(),
            value.as_str()
        )
    )))
}

/// The 404 for a value the tenant never declared.
fn value_not_found(class: VocabularyClass, value: &ScopeValue) -> CanonicalError {
    CanonicalError::from(DomainError::NotFound {
        subject: format!("{class} vocabulary value"),
        id: value.as_str().to_owned(),
    })
}

/// Resolve the `{value}` segment, refusing what [`authored_entry`] refuses.
fn parse_value(segment: &str) -> Result<ScopeValue, CanonicalError> {
    ScopeValue::new(segment).ok_or_else(|| {
        CanonicalError::from(DomainError::InvalidRequest(
            "a vocabulary value must not be blank or whitespace".to_owned(),
        ))
    })
}

/// One authored value as a domain entry — the `POST`'s body.
fn authored_entry(
    class: VocabularyClass,
    request: DeclareVocabularyValueRequest,
) -> Result<TaxonomyEntry, CanonicalError> {
    let declared = ScopeValue::new(&request.value).ok_or_else(|| {
        CanonicalError::from(DomainError::InvalidRequest(format!(
            "a {class} value is blank; a vocabulary entry names a code and the empty string is \
             not one"
        )))
    })?;
    // **A control character is refused here rather than at the `Location`
    // header** — `taxonomies::authored_entry`'s finding, one vocabulary over:
    // `ScopeValue::new` trims and refuses a blank and nothing else, so a value
    // carrying `\n` is written and then interpolated into this route's
    // `Location`, where the header conversion fails and a create that already
    // committed is answered `500`. Only control characters: a vocabulary value
    // is operator-authored text, so anything printable stays authorable.
    if let Some(bad) = declared.as_str().chars().find(|c| c.is_control()) {
        return Err(CanonicalError::from(DomainError::InvalidRequest(format!(
            "a {class} value must not carry a control character (found U+{:04X}): the value is \
             interpolated into this route's `Location` header, which cannot hold one",
            u32::from(bad)
        ))));
    }
    let state = parse_state_token(&declared, request.state.as_deref())?.unwrap_or_default();
    Ok(TaxonomyEntry {
        value: declared,
        display_name: request.display_name,
        state,
        // Neither table carries D-01's markers, and `None` here is what makes
        // `TaxonomyValuePatch::apply` leave them alone rather than invent them.
        tax: None,
    })
}

/// Turn a `PATCH` body into the patch.
fn authored_patch(
    value: &ScopeValue,
    request: PatchVocabularyValueRequest,
) -> Result<TaxonomyValuePatch, CanonicalError> {
    Ok(TaxonomyValuePatch {
        display_name: request.display_name,
        state: parse_state_token(value, request.state.as_deref())?,
        // Unsayable on this door's wire; see `PatchVocabularyValueRequest`.
        tax_category: crate::domain::taxonomy::TaxCategoryPatch::Keep,
        tax_rate_present: None,
    })
}

/// One `state` token, or the refusal naming what the machine admits.
///
/// One parser for both bodies, so a declare and a patch cannot come to admit
/// different tokens.
fn parse_state_token(
    value: &ScopeValue,
    token: Option<&str>,
) -> Result<Option<TaxonomyState>, CanonicalError> {
    let Some(token) = token else {
        return Ok(None);
    };
    TaxonomyState::parse(token).map(Some).ok_or_else(|| {
        CanonicalError::from(DomainError::InvalidRequest(format!(
            "value `{value}` carries state `{token}`; a vocabulary value is `{}`, and nothing \
             else",
            TaxonomyState::ALL
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("` or `")
        )))
    })
}

/// The `config × read` gate — `taxonomies::read_scope`'s, over one more pair
/// of routes.
///
/// **Called by each vocabulary's own handler, not from the bodies here**, and
/// deliberately: `module_test`'s precondition census reads a *handler's own*
/// text for `preconditions::if_match`, and a door that delegated its gate and
/// its precondition into this module would read to that census as a mutating
/// route asserting nothing. It would have been recorded as such — a false
/// statement in a roster whose whole job is to be true. The gate and the
/// header parse therefore stay at the door, in the order
/// `taxonomies::patch_taxonomy_value` sets: authz first, then the header.
///
/// # Errors
///
/// The gate's own refusal, mapped by `authz_error_to_canonical`.
pub async fn read_scope(
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

/// The `config × write` gate.
///
/// `owner_tenant_id = Some(caller's tenant)` because this is a write, for
/// `taxonomies::write_scope`'s reason: the membership assertion is what
/// refuses a target outside the compiled scope, the degraded flat-`In`
/// decision not re-checking the property.
///
/// At the door for [`read_scope`]'s reason.
///
/// # Errors
///
/// The gate's own refusal, mapped by `authz_error_to_canonical`.
pub async fn write_scope(
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
