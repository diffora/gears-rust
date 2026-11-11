// Created: 2026-04-17 by Constructor Tech
// Updated: 2026-11-09 by Constructor Tech
//! Public API trait of the `model-registry` gear (DESIGN §3.3 SDK Client
//! Trait).

use async_trait::async_trait;
use toolkit_canonical_errors::CanonicalError;
use toolkit_odata::{ODataQuery, Page};
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::models::{
    CreateModelRequestV1, CreateProviderRequestV1, ModelEntryV1, ModelV1, ProviderEntryV1,
    UpdateModelRequestV1, UpdateProviderRequestV1,
};

/// Model Registry client, version 1. Registered in `ClientHub` by the gear:
///
/// ```ignore
/// let registry = hub.get::<dyn ModelRegistryClientV1>()?;
/// let model = registry.resolve_model(&ctx, "openai::gpt-4o").await?;
/// ```
///
/// Every method takes the caller's [`SecurityContext`]; the acting tenant is
/// its subject tenant. Every method returns [`CanonicalError`]; the opt-in
/// typed view is [`ModelRegistryError`](crate::ModelRegistryError), and the
/// reason and field codes are in [`crate::reason`] and [`crate::field`].
///
/// `resolve_model` and `list_available_models` return the full [`ModelV1`],
/// `provider_settings`, `base_url`, and `secret_ref` included; both require
/// the model `evaluate` permission. The management methods return
/// [`ProviderEntryV1`] / [`ModelEntryV1`], which add the `inherited`,
/// `shadowed`, and eval-availability flags. Management operations address
/// entities by id; writes succeed only on entries the acting tenant owns
/// (inherited entries fail with `INHERITED_READ_ONLY`).
///
/// Every record carries a row `version`. Updates and deletes take an
/// optional `expected_version`: when present, the write applies only if the
/// stored version equals it and otherwise fails with `Aborted` and reason
/// `OPTIMISTIC_LOCK_FAILURE` ([`crate::reason::aborted`]); when absent, the
/// write is unconditional. Every successful update increments `version`.
///
/// Lists take an [`ODataQuery`] whose `$filter` and `$orderby` fields are
/// the [`ModelFilterField`](crate::odata::ModelFilterField) and
/// [`ProviderFilterField`](crate::odata::ProviderFilterField) allowlists;
/// `$orderby` additionally rejects the filter-only model fields
/// ([`MODEL_FILTER_ONLY_FIELDS`](crate::odata::MODEL_FILTER_ONLY_FIELDS)).
/// The SDK always returns whole records; `$select` is applied by the REST
/// layer only. Pages are keyset pages; a caller filter only narrows the
/// mandatory visibility predicates.
#[async_trait]
pub trait ModelRegistryClientV1: Send + Sync {
    // ==================== Resolution ====================

    /// Resolve a canonical id (`provider_slug::name`) in the caller's
    /// effective catalog for evaluation.
    ///
    /// Succeeds only when the model is visible to the acting tenant, its
    /// provider is enabled, it is approved, and the PDP permits `evaluate`.
    /// Failures, in this order: model not found (`NotFound`),
    /// `PROVIDER_DISABLED`, `MODEL_NOT_APPROVED`, `NOT_PERMITTED` (all
    /// `PermissionDenied`), or dependency unavailable (`ServiceUnavailable`)
    /// when Tenant Resolver, the PDP, or the database cannot answer. Returns
    /// the full record, including `provider_settings`, `base_url`, and
    /// `secret_ref`.
    async fn resolve_model(
        &self,
        ctx: &SecurityContext,
        canonical_id: &str,
    ) -> Result<ModelV1, CanonicalError>;

    /// List exactly the models `resolve_model` would return for the caller,
    /// as the same full records.
    ///
    /// Takes one PDP `evaluate` decision; a denial is `NOT_PERMITTED`, and a
    /// permit whose row constraints match no row is an empty page. There is
    /// no separate list permission.
    async fn list_available_models(
        &self,
        ctx: &SecurityContext,
        query: &ODataQuery,
    ) -> Result<Page<ModelV1>, CanonicalError>;

    // ==================== Providers ====================

    /// Create a provider in the acting tenant. A duplicate slug yields
    /// `AlreadyExists`; a `gts_type` that is not a registered provider type
    /// yields `InvalidArgument` with `INVALID_GTS_TYPE`,
    /// `NOT_A_PROVIDER_TYPE`, or `GTS_TYPE_NOT_REGISTERED`.
    async fn create_provider(
        &self,
        ctx: &SecurityContext,
        req: CreateProviderRequestV1,
    ) -> Result<ProviderEntryV1, CanonicalError>;

    /// Get an own or inherited provider by id.
    async fn get_provider(
        &self,
        ctx: &SecurityContext,
        id: Uuid,
    ) -> Result<ProviderEntryV1, CanonicalError>;

    /// List own providers plus inherited candidates, shadowed ones included.
    async fn list_providers(
        &self,
        ctx: &SecurityContext,
        query: &ODataQuery,
    ) -> Result<Page<ProviderEntryV1>, CanonicalError>;

    /// Update an own provider. Present fields replace stored values; a
    /// change of `enabled` or `sharing_mode` applies to the next request. A
    /// `gts_type` different from the stored one is a type migration of the
    /// provider and all its models, atomic and validated first. Conditional
    /// on `expected_version` when present.
    async fn update_provider(
        &self,
        ctx: &SecurityContext,
        id: Uuid,
        req: UpdateProviderRequestV1,
    ) -> Result<ProviderEntryV1, CanonicalError>;

    /// Delete an own provider and all of its models in one transaction. A
    /// provider it shadowed becomes visible again on the next request.
    /// Conditional on `expected_version` when present.
    async fn delete_provider(
        &self,
        ctx: &SecurityContext,
        id: Uuid,
        expected_version: Option<i64>,
    ) -> Result<(), CanonicalError>;

    // ==================== Models ====================

    /// Create a model under an own provider. The gear builds `canonical_id`
    /// as `provider_slug::name`, copies `gts_type` from the provider,
    /// validates `provider_settings` against the provider type schema
    /// (`SCHEMA_MISMATCH`), checks that a `secret_ref` names a secret the
    /// acting tenant owns (`SECRET_REF_NOT_FOUND`, `SECRET_REF_NOT_OWNED`),
    /// and records the approval decision (default `approved`). A duplicate
    /// `name` under the provider yields `AlreadyExists`; a provider type that
    /// changed during the write yields `Aborted` with
    /// `PROVIDER_TYPE_CHANGED`.
    async fn create_model(
        &self,
        ctx: &SecurityContext,
        req: CreateModelRequestV1,
    ) -> Result<ModelEntryV1, CanonicalError>;

    /// Get an own or inherited model by id, in any state.
    async fn get_model(
        &self,
        ctx: &SecurityContext,
        id: Uuid,
    ) -> Result<ModelEntryV1, CanonicalError>;

    /// Admin list under the model `list` permission: own models plus
    /// inherited candidates in every state, including models of disabled
    /// providers and models that are not approved, with flags. For the
    /// evaluable set use [`Self::list_available_models`].
    async fn list_models(
        &self,
        ctx: &SecurityContext,
        query: &ODataQuery,
    ) -> Result<Page<ModelEntryV1>, CanonicalError>;

    /// Update an own model. Present fields replace stored values; a present
    /// `approval_state` records a new decision; a `provider_settings`
    /// different from the stored one is validated against the provider type
    /// schema, and a non-null `secret_ref` different from the stored one is
    /// ownership-checked (null clears it without a check). Conditional on
    /// `expected_version` when present.
    async fn update_model(
        &self,
        ctx: &SecurityContext,
        id: Uuid,
        req: UpdateModelRequestV1,
    ) -> Result<ModelEntryV1, CanonicalError>;

    /// Hard-delete an own model. Conditional on `expected_version` when
    /// present.
    async fn delete_model(
        &self,
        ctx: &SecurityContext,
        id: Uuid,
        expected_version: Option<i64>,
    ) -> Result<(), CanonicalError>;
}
