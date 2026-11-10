// Created: 2026-05-06 by Constructor Tech
// Updated: 2026-11-10 by Constructor Tech
//! Transport-agnostic request types of the Model Registry SDK (DESIGN §3.3
//! Request contents).
//!
//! These are not REST DTOs: they sit on [`crate::ModelRegistryClientV1`] and
//! the gear's REST layer converts its DTOs into them.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::models::{
    ApprovalState, ContextWindow, DefaultInferenceParametersV1, DisabledCapabilities,
    ModelCapabilities, ModelCostV1, ModelPerformance, SharingMode, SupportedApi,
};

// ---------------------------------------------------------------------------
// CreateProviderRequestV1 (builder pattern)
// ---------------------------------------------------------------------------

/// Request for creating a provider in the acting tenant. Construct via
/// [`CreateProviderRequestV1::builder`].
#[derive(Debug, Clone, PartialEq)]
pub struct CreateProviderRequestV1 {
    slug: String,
    name: String,
    gts_type: gts::GtsTypeId,
    enabled: bool,
    sharing_mode: SharingMode,
    managed: bool,
    metadata: Option<serde_json::Value>,
}

impl CreateProviderRequestV1 {
    /// Start building a request. The three arguments are the required
    /// fields; `enabled` defaults to true, `sharing_mode` to
    /// [`SharingMode::Tenant`], `managed` to false, `metadata` to absent.
    #[must_use]
    pub fn builder(
        slug: impl Into<String>,
        name: impl Into<String>,
        gts_type: gts::GtsTypeId,
    ) -> CreateProviderRequestV1Builder {
        CreateProviderRequestV1Builder {
            slug: slug.into(),
            name: name.into(),
            gts_type,
            enabled: true,
            sharing_mode: SharingMode::Tenant,
            managed: false,
            metadata: None,
        }
    }

    /// 1–64 characters: lowercase letters, digits, hyphen. Immutable.
    #[must_use]
    pub fn slug(&self) -> &str {
        &self.slug
    }

    /// Display name, 1–255 characters.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Provider GTS type, derived from
    /// [`PROVIDER_BASE_TYPE`](crate::gts::PROVIDER_BASE_TYPE). Changes later
    /// only through a type migration ([`UpdateProviderRequestV1::gts_type`]).
    #[must_use]
    pub fn gts_type(&self) -> &gts::GtsTypeId {
        &self.gts_type
    }

    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    #[must_use]
    pub fn sharing_mode(&self) -> SharingMode {
        self.sharing_mode
    }

    #[must_use]
    pub fn managed(&self) -> bool {
        self.managed
    }

    #[must_use]
    pub fn metadata(&self) -> Option<&serde_json::Value> {
        self.metadata.as_ref()
    }
}

#[derive(Debug, Clone)]
pub struct CreateProviderRequestV1Builder {
    slug: String,
    name: String,
    gts_type: gts::GtsTypeId,
    enabled: bool,
    sharing_mode: SharingMode,
    managed: bool,
    metadata: Option<serde_json::Value>,
}

impl CreateProviderRequestV1Builder {
    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    #[must_use]
    pub fn sharing_mode(mut self, sharing_mode: SharingMode) -> Self {
        self.sharing_mode = sharing_mode;
        self
    }

    #[must_use]
    pub fn managed(mut self, managed: bool) -> Self {
        self.managed = managed;
        self
    }

    #[must_use]
    pub fn metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }

    #[must_use]
    pub fn build(self) -> CreateProviderRequestV1 {
        CreateProviderRequestV1 {
            slug: self.slug,
            name: self.name,
            gts_type: self.gts_type,
            enabled: self.enabled,
            sharing_mode: self.sharing_mode,
            managed: self.managed,
            metadata: self.metadata,
        }
    }
}

// ---------------------------------------------------------------------------
// UpdateProviderRequestV1
// ---------------------------------------------------------------------------

/// Request for updating an own provider. A present field replaces the stored
/// value as a whole; absent fields stay unchanged.
///
/// `slug` and `tenant_id` are immutable and absent here. The nullable
/// `metadata` uses `Option<Option<T>>`: `None` leaves it unchanged,
/// `Some(None)` clears it, `Some(Some(v))` sets it.
#[derive(Debug, Clone, Default, PartialEq)]
#[allow(clippy::option_option)]
#[non_exhaustive]
pub struct UpdateProviderRequestV1 {
    /// When present, the update applies only if the stored `version` equals
    /// it; otherwise it fails with `OPTIMISTIC_LOCK_FAILURE` and nothing
    /// changes. Absent means an unconditional write.
    pub expected_version: Option<i64>,
    pub name: Option<String>,
    /// A value different from the stored one is a type migration: every model
    /// of the provider is validated against the new type's schema and the
    /// provider and its models switch type in one transaction, or nothing
    /// changes (DESIGN §3.6 provider write path).
    pub gts_type: Option<gts::GtsTypeId>,
    pub enabled: Option<bool>,
    pub sharing_mode: Option<SharingMode>,
    pub managed: Option<bool>,
    pub metadata: Option<Option<serde_json::Value>>,
}

// ---------------------------------------------------------------------------
// CreateModelRequestV1
// ---------------------------------------------------------------------------

/// Request for creating a model under an own provider.
///
/// Carries every [`ModelInfoV1`](crate::models::ModelInfoV1) field except
/// `gts_type`, which the gear copies from the provider. The gear also builds
/// `canonical_id` as `provider_slug::name`. Field semantics and limits are
/// those of `ModelInfoV1`.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct CreateModelRequestV1 {
    /// Provider that serves the model. Must be owned by the acting tenant.
    pub provider_id: Uuid,
    /// 1–128 characters: lowercase letters, digits, hyphen, dot, underscore.
    /// Unique among the models of the provider. Immutable; the right part of
    /// `canonical_id`.
    pub name: String,
    /// Default [`SharingMode::Tenant`].
    pub sharing_mode: Option<SharingMode>,
    /// Default [`ApprovalState::Approved`], decided by the acting subject at
    /// creation time.
    pub approval_state: Option<ApprovalState>,

    pub provider_model_id: String,
    pub base_url: Option<String>,
    pub secret_ref: Option<String>,
    pub display_name: String,
    pub description: Option<String>,
    pub family: Option<String>,
    pub vendor: Option<String>,
    pub managed: bool,
    pub architecture: Option<String>,
    pub size_bytes: Option<u64>,
    pub format: Option<String>,
    pub region: Option<String>,
    pub hosted_by: Option<String>,
    pub last_release_at: Option<DateTime<Utc>>,
    pub reasoning_level: Option<String>,
    pub model_version: Option<String>,
    pub sort_order: i32,
    pub icon: Option<String>,
    pub multiplier_display: Option<String>,
    pub cost: ModelCostV1,
    pub performance: ModelPerformance,
    pub additional_info: HashMap<String, serde_json::Value>,
    pub supported_api: HashSet<SupportedApi>,
    pub capabilities: ModelCapabilities,
    pub disabled_capabilities: DisabledCapabilities,
    pub context_window: ContextWindow,
    pub default_parameters: DefaultInferenceParametersV1,
    pub allow_parameter_override: bool,
    pub allow_extra_params: Vec<String>,
    /// Validated by the gear against the provider type schema.
    pub provider_settings: serde_json::Value,
}

impl CreateModelRequestV1 {
    /// A request with the required fields set and every other field at its
    /// default: optional fields absent, flags false, collections empty,
    /// `cost` all absent, `provider_settings` an empty object.
    #[must_use]
    pub fn new(
        provider_id: Uuid,
        name: impl Into<String>,
        provider_model_id: impl Into<String>,
        display_name: impl Into<String>,
        context_window: ContextWindow,
    ) -> Self {
        Self {
            provider_id,
            name: name.into(),
            sharing_mode: None,
            approval_state: None,
            provider_model_id: provider_model_id.into(),
            base_url: None,
            secret_ref: None,
            display_name: display_name.into(),
            description: None,
            family: None,
            vendor: None,
            managed: false,
            architecture: None,
            size_bytes: None,
            format: None,
            region: None,
            hosted_by: None,
            last_release_at: None,
            reasoning_level: None,
            model_version: None,
            sort_order: 0,
            icon: None,
            multiplier_display: None,
            cost: ModelCostV1::default(),
            performance: ModelPerformance::default(),
            additional_info: HashMap::new(),
            supported_api: HashSet::new(),
            capabilities: ModelCapabilities::default(),
            disabled_capabilities: DisabledCapabilities::none(),
            context_window,
            default_parameters: DefaultInferenceParametersV1::default(),
            allow_parameter_override: false,
            allow_extra_params: Vec::new(),
            provider_settings: serde_json::Value::Object(serde_json::Map::new()),
        }
    }
}

// ---------------------------------------------------------------------------
// UpdateModelRequestV1
// ---------------------------------------------------------------------------

/// Request for updating an own model. A present field replaces the stored
/// value as a whole, nested objects included; absent fields stay unchanged.
///
/// The immutable fields `provider_id`, `name`, `gts_type`, `canonical_id`,
/// `tenant_id`, and the row `version` are absent here. A present
/// `approval_state` records a decision by the acting subject even when the
/// state does not change. Nullable fields use `Option<Option<T>>`: `None`
/// leaves the field unchanged, `Some(None)` clears it, `Some(Some(v))` sets
/// it.
#[derive(Debug, Clone, Default, PartialEq)]
#[allow(clippy::option_option)]
#[non_exhaustive]
pub struct UpdateModelRequestV1 {
    /// When present, the update applies only if the stored `version` equals
    /// it; otherwise it fails with `OPTIMISTIC_LOCK_FAILURE` and nothing
    /// changes. Absent means an unconditional write.
    pub expected_version: Option<i64>,
    pub sharing_mode: Option<SharingMode>,
    pub approval_state: Option<ApprovalState>,

    pub provider_model_id: Option<String>,
    pub base_url: Option<Option<String>>,
    /// A non-null value different from the stored one is ownership-checked
    /// against `CredStore`; `Some(None)` clears it without a check.
    pub secret_ref: Option<Option<String>>,
    pub display_name: Option<String>,
    pub description: Option<Option<String>>,
    pub family: Option<Option<String>>,
    pub vendor: Option<Option<String>>,
    pub managed: Option<bool>,
    pub architecture: Option<Option<String>>,
    pub size_bytes: Option<Option<u64>>,
    pub format: Option<Option<String>>,
    pub region: Option<Option<String>>,
    pub hosted_by: Option<Option<String>>,
    pub last_release_at: Option<Option<DateTime<Utc>>>,
    pub reasoning_level: Option<Option<String>>,
    pub model_version: Option<Option<String>>,
    pub sort_order: Option<i32>,
    pub icon: Option<Option<String>>,
    pub multiplier_display: Option<Option<String>>,
    pub cost: Option<ModelCostV1>,
    pub performance: Option<ModelPerformance>,
    pub additional_info: Option<HashMap<String, serde_json::Value>>,
    pub supported_api: Option<HashSet<SupportedApi>>,
    pub capabilities: Option<ModelCapabilities>,
    pub disabled_capabilities: Option<DisabledCapabilities>,
    pub context_window: Option<ContextWindow>,
    pub default_parameters: Option<DefaultInferenceParametersV1>,
    pub allow_parameter_override: Option<bool>,
    pub allow_extra_params: Option<Vec<String>>,
    /// Validated by the gear against the model's provider type schema.
    pub provider_settings: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use toolkit_gts::gts_id;

    #[test]
    fn provider_builder_applies_defaults_and_overrides() {
        let gts = gts::GtsTypeId::new(gts_id!("cf.genai.model.provider.v1~cf.genai._.openai.v1~"));

        let full = CreateProviderRequestV1::builder("openai", "OpenAI", gts.clone())
            .enabled(false)
            .sharing_mode(SharingMode::Shared)
            .managed(true)
            .metadata(serde_json::json!({"k": "v"}))
            .build();
        assert_eq!(full.slug(), "openai");
        assert_eq!(full.name(), "OpenAI");
        assert_eq!(full.gts_type(), &gts);
        assert!(!full.enabled());
        assert_eq!(full.sharing_mode(), SharingMode::Shared);
        assert!(full.managed());
        assert_eq!(full.metadata(), Some(&serde_json::json!({"k": "v"})));

        let bare = CreateProviderRequestV1::builder("openai", "OpenAI", gts).build();
        assert!(bare.enabled());
        assert_eq!(bare.sharing_mode(), SharingMode::Tenant);
        assert!(!bare.managed());
        assert!(bare.metadata().is_none());
    }

    #[test]
    fn create_model_request_new_sets_required_fields_only() {
        let req = CreateModelRequestV1::new(
            Uuid::nil(),
            "gpt-4o-eu",
            "gpt-4o",
            "GPT-4o",
            ContextWindow {
                max_input_tokens: 128_000,
                max_output_tokens: Some(16_384),
                output_vector_size: None,
            },
        );
        assert_eq!(req.name, "gpt-4o-eu");
        assert_eq!(req.provider_model_id, "gpt-4o");
        assert_eq!(req.display_name, "GPT-4o");
        assert_eq!(req.cost, ModelCostV1::default());
        assert_eq!(req.context_window.max_input_tokens, 128_000);
        assert_eq!(req.sharing_mode, None);
        assert_eq!(req.approval_state, None);
        assert_eq!(req.base_url, None);
        assert_eq!(req.secret_ref, None);
        assert_eq!(req.provider_settings, serde_json::json!({}));
        assert!(req.supported_api.is_empty());
    }
}
