// Created: 2026-04-17 by Constructor Tech
// Updated: 2026-11-10 by Constructor Tech
//! Opt-in typed projection of [`CanonicalError`] for Model Registry consumers.
//!
//! Every [`ModelRegistryClientV1`](crate::ModelRegistryClientV1) method
//! returns `CanonicalError` ([ADR 0005][adr]). [`ModelRegistryError`] is a
//! flat view over that envelope for consumers that dispatch on the outcomes
//! the registry emits; it is not part of the trait contract. The conversion
//! is infallible: categories the registry does not model, and envelopes it
//! cannot have produced (a `NotFound` without one of the registry's resource
//! types, a `PermissionDenied` with an unknown reason), fall through to
//! [`ModelRegistryError::Other`] with the full canonical error.
//!
//! | Outcome | Variant | HTTP |
//! |---|---|---|
//! | model not in the effective catalog, malformed canonical id, unknown or invisible id | [`ModelRegistryError::ModelNotFound`] | 404 |
//! | provider unknown or not visible | [`ModelRegistryError::ProviderNotFound`] | 404 |
//! | `resolve_model`: provider disabled | [`ModelRegistryError::ProviderDisabled`] | 403 |
//! | `resolve_model`: model pending, rejected, or revoked | [`ModelRegistryError::ModelNotApproved`] | 403 |
//! | PDP denial | [`ModelRegistryError::NotPermitted`] | 403 |
//! | write against an inherited entry | [`ModelRegistryError::InheritedReadOnly`] | 403 |
//! | duplicate provider slug in the tenant, or duplicate model name under the provider | [`ModelRegistryError::AlreadyExists`] | 409 |
//! | stale `expected_version`, or a concurrent change the write raced (see [`crate::reason::aborted`]) | [`ModelRegistryError::OptimisticLockFailure`] | 409 |
//! | invalid field, GTS type, settings, secret reference, `OData` option, or cursor (see [`crate::field`]) | [`ModelRegistryError::Invalid`] | 400 |
//! | Tenant Resolver, PDP, Types Registry, `CredStore`, or database cannot answer | [`ModelRegistryError::DependencyUnavailable`] | 503 |
//! | logic or invariant failure | [`ModelRegistryError::Internal`] | 500 |
//! | anything else | [`ModelRegistryError::Other`] | — |
//!
//! Consumer patterns:
//!
//! ```ignore
//! // 1. Propagate the canonical error.
//! let model = registry.resolve_model(&ctx, "openai::gpt-4o").await?;
//!
//! // 2. Project at the call site.
//! match registry.resolve_model(&ctx, id).await.map_err(ModelRegistryError::from) {
//!     Err(ModelRegistryError::ModelNotApproved { .. }) => { /* … */ }
//!     other => { /* … */ }
//! }
//!
//! // 3. Chain through the consumer's own error type.
//! impl From<CanonicalError> for OwnError {
//!     fn from(err: CanonicalError) -> Self {
//!         ModelRegistryError::from(err).into()
//!     }
//! }
//! ```
//!
//! [adr]: https://github.com/constructorfabric/gears-rust/blob/main/docs/arch/errors/ADR/0005-cpt-cf-adr-sdk-canonical-projection.md

use thiserror::Error;
use toolkit_canonical_errors::{CanonicalError, FieldViolation, InvalidArgument};

use crate::gts::{MODEL_RESOURCE_TYPE, PROVIDER_RESOURCE_TYPE, Resource};
use crate::reason::{aborted, permission};

/// Typed projection of [`CanonicalError`]; see the [module docs](self).
#[derive(Error, Debug, Clone)]
#[non_exhaustive]
pub enum ModelRegistryError {
    /// `name` is the canonical id or the model id the caller supplied.
    #[error("model not found: {name}")]
    ModelNotFound { name: String, detail: String },

    /// `name` is the provider id the caller supplied.
    #[error("provider not found: {name}")]
    ProviderNotFound { name: String, detail: String },

    #[error("provider disabled: {detail}")]
    ProviderDisabled { detail: String },

    #[error("model not approved: {detail}")]
    ModelNotApproved { detail: String },

    #[error("not permitted: {detail}")]
    NotPermitted { detail: String },

    #[error("inherited entry is read-only: {detail}")]
    InheritedReadOnly { detail: String },

    /// `name` is the duplicate slug or canonical id.
    #[error("already exists [{}]: {name}", resource.as_wire())]
    AlreadyExists {
        resource: Resource,
        name: String,
        detail: String,
    },

    /// `reason` is [`aborted::OPTIMISTIC_LOCK_FAILURE`] or
    /// [`aborted::PROVIDER_TYPE_CHANGED`]. Retriable after a re-read.
    #[error("optimistic lock failure ({reason}): {detail}")]
    OptimisticLockFailure { reason: String, detail: String },

    /// `violations` is empty when the error carries no field violations
    /// (for example an invalid `OData` option). Match
    /// `violations[].reason` against [`crate::field`].
    #[error("invalid argument: {detail}")]
    Invalid {
        violations: Vec<FieldViolation>,
        detail: String,
    },

    #[error("dependency unavailable: {detail}")]
    DependencyUnavailable { detail: String },

    #[error("internal error: {detail}")]
    Internal { detail: String },

    /// Catch-all for canonical categories the registry does not model.
    #[error("[{}] {}", canonical.gts_type(), canonical.detail())]
    Other { canonical: CanonicalError },
}

impl From<CanonicalError> for ModelRegistryError {
    fn from(err: CanonicalError) -> Self {
        let detail = err.detail().to_owned();
        match err {
            CanonicalError::NotFound {
                resource_type: Some(resource_type),
                resource_name,
                ..
            } if resource_type == MODEL_RESOURCE_TYPE => Self::ModelNotFound {
                name: resource_name.unwrap_or_default(),
                detail,
            },
            CanonicalError::NotFound {
                resource_type: Some(resource_type),
                resource_name,
                ..
            } if resource_type == PROVIDER_RESOURCE_TYPE => Self::ProviderNotFound {
                name: resource_name.unwrap_or_default(),
                detail,
            },

            CanonicalError::PermissionDenied { ref ctx, .. }
                if ctx.reason == permission::PROVIDER_DISABLED =>
            {
                Self::ProviderDisabled { detail }
            }
            CanonicalError::PermissionDenied { ref ctx, .. }
                if ctx.reason == permission::MODEL_NOT_APPROVED =>
            {
                Self::ModelNotApproved { detail }
            }
            CanonicalError::PermissionDenied { ref ctx, .. }
                if ctx.reason == permission::NOT_PERMITTED =>
            {
                Self::NotPermitted { detail }
            }
            CanonicalError::PermissionDenied { ref ctx, .. }
                if ctx.reason == permission::INHERITED_READ_ONLY =>
            {
                Self::InheritedReadOnly { detail }
            }

            CanonicalError::AlreadyExists {
                resource_type: Some(resource_type),
                resource_name,
                ..
            } if resource_type == MODEL_RESOURCE_TYPE => Self::AlreadyExists {
                resource: Resource::Model,
                name: resource_name.unwrap_or_default(),
                detail,
            },
            CanonicalError::AlreadyExists {
                resource_type: Some(resource_type),
                resource_name,
                ..
            } if resource_type == PROVIDER_RESOURCE_TYPE => Self::AlreadyExists {
                resource: Resource::Provider,
                name: resource_name.unwrap_or_default(),
                detail,
            },

            CanonicalError::Aborted { ref ctx, .. }
                if ctx.reason == aborted::OPTIMISTIC_LOCK_FAILURE
                    || ctx.reason == aborted::PROVIDER_TYPE_CHANGED =>
            {
                Self::OptimisticLockFailure {
                    reason: ctx.reason.clone(),
                    detail,
                }
            }

            CanonicalError::InvalidArgument { ctx, .. } => Self::Invalid {
                violations: match ctx {
                    InvalidArgument::FieldViolations { field_violations } => field_violations,
                    InvalidArgument::Format { .. } | InvalidArgument::Constraint { .. } => {
                        Vec::new()
                    }
                },
                detail,
            },

            CanonicalError::ServiceUnavailable { .. } => Self::DependencyUnavailable { detail },

            CanonicalError::Internal { .. } => Self::Internal { detail },

            other => Self::Other { canonical: other },
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::ModelRegistryError;
    use crate::gts::{MODEL_RESOURCE_TYPE, PROVIDER_RESOURCE_TYPE, Resource};
    use crate::{
        field,
        reason::{aborted, permission},
    };
    use toolkit_canonical_errors::{CanonicalError, Problem, resource_error};

    // Scopes mirroring the gear's `#[resource_error]` markers. Their
    // literals must equal the `gts::*_RESOURCE_TYPE` constants; the
    // round-trip tests below assert that (a proc macro cannot reference a
    // const).
    #[resource_error(gts_id!("cf.genai.model_registry.model.v1~"))]
    struct ModelScope;

    #[resource_error(gts_id!("cf.genai.model_registry.provider.v1~"))]
    struct ProviderScope;

    #[resource_error(gts_id!("cf.other.pkg.thing.v1~"))]
    struct ForeignScope;

    fn problem(err: CanonicalError) -> serde_json::Value {
        serde_json::to_value(Problem::from(err)).expect("Problem serializes")
    }

    // ── Wire vocabulary round-trips ──────────────────────────────────

    #[test]
    fn resource_types_round_trip_to_context_resource_type() {
        let json = problem(ModelScope::not_found("x").with_resource("x").create());
        assert_eq!(json["context"]["resource_type"], MODEL_RESOURCE_TYPE);

        let json = problem(ProviderScope::not_found("x").with_resource("x").create());
        assert_eq!(json["context"]["resource_type"], PROVIDER_RESOURCE_TYPE);
    }

    #[test]
    fn permission_reasons_round_trip_to_context_reason() {
        for reason in [
            permission::PROVIDER_DISABLED,
            permission::MODEL_NOT_APPROVED,
            permission::NOT_PERMITTED,
            permission::INHERITED_READ_ONLY,
        ] {
            let json = problem(ModelScope::permission_denied().with_reason(reason).create());
            assert_eq!(json["context"]["reason"], reason);
        }
    }

    #[test]
    fn aborted_reasons_round_trip_to_context_reason() {
        for reason in [
            aborted::OPTIMISTIC_LOCK_FAILURE,
            aborted::PROVIDER_TYPE_CHANGED,
        ] {
            let json = problem(ModelScope::aborted("stale").with_reason(reason).create());
            assert_eq!(json["context"]["reason"], reason);
        }
    }

    #[test]
    fn field_reasons_round_trip_to_field_violations() {
        for reason in field::Reason::ALL {
            let json = problem(
                ModelScope::invalid_argument()
                    .with_field_violation("provider_settings", "bad", reason.as_wire())
                    .create(),
            );
            assert_eq!(
                json["context"]["field_violations"][0]["reason"],
                reason.as_wire()
            );
            assert_eq!(
                json["context"]["field_violations"][0]["field"],
                "provider_settings"
            );
        }
    }

    // ── Projection ───────────────────────────────────────────────────

    #[test]
    fn not_found_projects_by_resource_type() {
        let err = ModelRegistryError::from(
            ModelScope::not_found("missing")
                .with_resource("openai::gpt-4o")
                .create(),
        );
        assert!(
            matches!(&err, ModelRegistryError::ModelNotFound { name, detail }
                if name == "openai::gpt-4o" && detail == "missing"),
            "{err:?}"
        );

        let err = ModelRegistryError::from(
            ProviderScope::not_found("missing")
                .with_resource("p1")
                .create(),
        );
        assert!(
            matches!(&err, ModelRegistryError::ProviderNotFound { name, .. } if name == "p1"),
            "{err:?}"
        );
    }

    #[test]
    fn not_found_with_foreign_resource_type_falls_through_to_other() {
        let err = ModelRegistryError::from(
            ForeignScope::not_found("missing")
                .with_resource("x")
                .create(),
        );
        assert!(
            matches!(
                &err,
                ModelRegistryError::Other {
                    canonical: CanonicalError::NotFound { .. }
                }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn permission_denied_projects_by_reason() {
        let project = |reason: &str| {
            ModelRegistryError::from(ModelScope::permission_denied().with_reason(reason).create())
        };
        assert!(matches!(
            project(permission::PROVIDER_DISABLED),
            ModelRegistryError::ProviderDisabled { .. }
        ));
        assert!(matches!(
            project(permission::MODEL_NOT_APPROVED),
            ModelRegistryError::ModelNotApproved { .. }
        ));
        assert!(matches!(
            project(permission::NOT_PERMITTED),
            ModelRegistryError::NotPermitted { .. }
        ));
        assert!(matches!(
            project(permission::INHERITED_READ_ONLY),
            ModelRegistryError::InheritedReadOnly { .. }
        ));

        let err = project("SOMETHING_ELSE");
        assert!(
            matches!(
                &err,
                ModelRegistryError::Other {
                    canonical: CanonicalError::PermissionDenied { .. }
                }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn already_exists_projects_typed_resource() {
        let err = ModelRegistryError::from(
            ProviderScope::already_exists("dup")
                .with_resource("openai")
                .create(),
        );
        assert!(
            matches!(&err, ModelRegistryError::AlreadyExists { resource: Resource::Provider, name, .. }
                if name == "openai"),
            "{err:?}"
        );

        let err = ModelRegistryError::from(
            ModelScope::already_exists("dup")
                .with_resource("openai::gpt-4o")
                .create(),
        );
        assert!(
            matches!(
                &err,
                ModelRegistryError::AlreadyExists {
                    resource: Resource::Model,
                    ..
                }
            ),
            "{err:?}"
        );

        let err = ModelRegistryError::from(
            ForeignScope::already_exists("dup")
                .with_resource("x")
                .create(),
        );
        assert!(matches!(&err, ModelRegistryError::Other { .. }), "{err:?}");
    }

    #[test]
    fn aborted_projects_to_optimistic_lock_failure_by_reason() {
        for expected in [
            aborted::OPTIMISTIC_LOCK_FAILURE,
            aborted::PROVIDER_TYPE_CHANGED,
        ] {
            let err = ModelRegistryError::from(
                ModelScope::aborted("stale").with_reason(expected).create(),
            );
            assert!(
                matches!(&err, ModelRegistryError::OptimisticLockFailure { reason, detail }
                    if reason == expected && detail == "stale"),
                "{err:?}"
            );
        }
    }

    #[test]
    fn invalid_argument_projects_every_violation() {
        let err = ModelRegistryError::from(
            ModelScope::invalid_argument()
                .with_field_violation("provider_settings/cost", "too high", field::SCHEMA_MISMATCH)
                .with_field_violation("base_url", "not a URL", field::INVALID_FORMAT)
                .create(),
        );
        let ModelRegistryError::Invalid { violations, .. } = &err else {
            panic!("expected Invalid, got {err:?}");
        };
        assert_eq!(violations.len(), 2);
        assert_eq!(violations[0].field, "provider_settings/cost");
        assert_eq!(
            field::Reason::from_wire(&violations[0].reason),
            Some(field::Reason::SchemaMismatch)
        );
        assert_eq!(violations[1].reason, field::INVALID_FORMAT);
    }

    #[test]
    fn invalid_argument_without_violations_projects_empty_list() {
        let err = ModelRegistryError::from(
            ModelScope::invalid_argument()
                .with_format("bad cursor")
                .create(),
        );
        assert!(
            matches!(&err, ModelRegistryError::Invalid { violations, .. } if violations.is_empty()),
            "{err:?}"
        );
    }

    #[test]
    fn service_unavailable_projects_to_dependency_unavailable() {
        let err = ModelRegistryError::from(
            CanonicalError::service_unavailable()
                .with_detail("tenant resolver timed out")
                .create(),
        );
        assert!(
            matches!(&err, ModelRegistryError::DependencyUnavailable { detail }
                if detail == "tenant resolver timed out"),
            "{err:?}"
        );
    }

    #[test]
    fn internal_projects_to_internal() {
        let err = ModelRegistryError::from(CanonicalError::internal("invariant").create());
        assert!(
            matches!(&err, ModelRegistryError::Internal { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn unmodeled_categories_fall_through_to_other() {
        let err = ModelRegistryError::from(
            ModelScope::aborted("conflict")
                .with_reason("CONFLICT")
                .create(),
        );
        assert!(
            matches!(
                &err,
                ModelRegistryError::Other {
                    canonical: CanonicalError::Aborted { .. }
                }
            ),
            "{err:?}"
        );

        let err = ModelRegistryError::from(ModelScope::deadline_exceeded("slow").create());
        assert!(
            matches!(
                &err,
                ModelRegistryError::Other {
                    canonical: CanonicalError::DeadlineExceeded { .. }
                }
            ),
            "{err:?}"
        );
    }
}
