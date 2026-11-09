// Created: 2026-05-06 by Constructor Tech
// Updated: 2026-11-06 by Constructor Tech
//! [`ModelInfoV1`] — the record LLM Gateway uses to evaluate a model
//! (DESIGN §3.1 Model Info).

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};

use crate::models::{
    ContextWindow, DefaultInferenceParametersV1, DisabledCapabilities, ModelCapabilities,
    ModelCostV1, ModelPerformance, SupportedApi,
};

/// Model info: general settings, user-facing information, cost, capabilities,
/// the context window, default parameters with their override policy, and the
/// opaque `provider_settings` document.
///
/// Not a GTS type. `gts_type` names the provider type, derived from
/// [`PROVIDER_BASE_TYPE`](crate::gts::PROVIDER_BASE_TYPE), whose schema fixes
/// the shape of `provider_settings`. The registry validates the document
/// against that schema on write and returns it unchanged; LLM Gateway selects
/// the plugin whose provider type equals `gts_type` and narrows
/// `provider_settings` to the plugin's typed view.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct ModelInfoV1 {
    /// Provider GTS type, copied from the model's provider on create.
    /// Read-only: never part of a create or update request.
    pub gts_type: gts::GtsTypeId,
    /// The provider's own model identifier, sent to the provider. 1–255
    /// characters without whitespace or control characters. Mutable; not
    /// part of the canonical id, and several models of one provider may
    /// share it.
    pub provider_model_id: String,
    /// Absolute `http` or `https` URL the plugin calls for this model, up to
    /// 2,048 characters, without user information. When absent, the plugin
    /// uses its provider's default URL; the registry stores no default.
    pub base_url: Option<String>,
    /// `CredStore` secret reference for the provider credential: 1–255
    /// characters from `[a-zA-Z0-9_-]`, the `SecretRef` format of
    /// `credstore-sdk`. Must name a secret the model's tenant owns; the
    /// registry checks this on create and on an update that changes it to a
    /// non-null value, and stores no secret material.
    pub secret_ref: Option<String>,
    /// Name shown to users, 1–255 characters.
    pub display_name: String,
    /// Free-text description, up to 4,096 characters.
    pub description: Option<String>,
    /// Model family label, for example `gpt-4` or `llama`.
    pub family: Option<String>,
    /// Organization that produced the model weights. Independent of the
    /// provider that serves the model.
    pub vendor: Option<String>,
    /// Whether the platform can load and unload this model on a managed
    /// runtime. Informational. Default false.
    pub managed: bool,
    /// Architecture class, for example `llama` or `mistral`.
    pub architecture: Option<String>,
    /// On-disk size of locally hosted weights, at most 2^63 − 1.
    pub size_bytes: Option<u64>,
    /// Weight or serving format, for example `gguf` or `safetensors`.
    pub format: Option<String>,
    /// Deployment region, for example `eu-west-1`.
    pub region: Option<String>,
    /// Infrastructure host, for example `Azure` or `self-hosted`.
    pub hosted_by: Option<String>,
    /// When the vendor last released this model version.
    pub last_release_at: Option<DateTime<Utc>>,
    /// Display-only reasoning level label.
    pub reasoning_level: Option<String>,
    /// The model's release version as published by the vendor. Not the row
    /// `version` of [`ModelV1`](crate::models::ModelV1).
    pub model_version: Option<String>,
    /// Position in model pickers; lower first. Default 0.
    pub sort_order: i32,
    /// Absolute `http` or `https` URL of the model icon, up to 2,048
    /// characters.
    pub icon: Option<String>,
    /// Display label for the relative cost, for example `3x`.
    pub multiplier_display: Option<String>,
    /// Cost in AI credits.
    pub cost: ModelCostV1,
    /// Expected performance.
    pub performance: ModelPerformance,
    /// Deployment-specific metadata. Default empty.
    pub additional_info: HashMap<String, serde_json::Value>,
    /// API kinds the model serves.
    pub supported_api: HashSet<SupportedApi>,
    /// What the model can do.
    pub capabilities: ModelCapabilities,
    /// Capabilities an administrator turned off for this model. The
    /// effective capability is the declared capability minus the disabled
    /// one.
    pub disabled_capabilities: DisabledCapabilities,
    /// Token limits.
    pub context_window: ContextWindow,
    /// Provider-independent inference defaults.
    pub default_parameters: DefaultInferenceParametersV1,
    /// Whether callers may override `default_parameters` per request.
    /// Default false.
    pub allow_parameter_override: bool,
    /// Names of extra request parameters callers may pass to the provider,
    /// up to 64 names of up to 128 characters. Default empty.
    pub allow_extra_params: Vec<String>,
    /// Provider-specific settings. Shape defined by the provider type
    /// schema; opaque to the registry beyond validation.
    pub provider_settings: serde_json::Value,
}
