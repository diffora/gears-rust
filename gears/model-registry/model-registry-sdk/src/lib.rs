// Created: 2026-04-17 by Constructor Tech
// Updated: 2026-11-05 by Constructor Tech
//! Model Registry SDK
//!
//! Public contract of the `model-registry` gear (DESIGN §3.2 SDK):
//! - [`ModelRegistryClientV1`] — the client trait, obtained from `ClientHub`;
//!   every method returns [`toolkit_canonical_errors::CanonicalError`]
//! - Models: [`ProviderV1`], [`ModelV1`], [`ModelInfoV1`], [`ApprovalV1`],
//!   the management entries [`ProviderEntryV1`] / [`ModelEntryV1`], and the
//!   create / update requests; records carry a row `version` and writes
//!   take an optional `expected_version`
//! - [`ProviderTypeV1`] — the provider base GTS type
//!   `gts.cf.genai.model.provider.v1~` that LLM Gateway provider plugins
//!   derive their provider types from
//! - [`ModelRegistryError`] — opt-in typed projection of the canonical error
//!   (see [`errors`]), with the wire vocabulary in [`reason`], [`field`],
//!   and [`gts`]
//! - `OData` filter / order allowlists of the list methods ([`odata`])
//!
//! ```ignore
//! use model_registry_sdk::{ModelRegistryClientV1, ModelRegistryError};
//!
//! let registry = hub.get::<dyn ModelRegistryClientV1>()?;
//! match registry.resolve_model(&ctx, "openai::gpt-4o").await.map_err(ModelRegistryError::from) {
//!     Ok(model) => {
//!         // `model.info.gts_type` selects the provider plugin; the plugin
//!         // narrows `model.info.provider_settings` to its own settings type.
//!         let _ = model.info.base_url.as_deref();
//!         let _ = model.info.secret_ref.as_deref();
//!     }
//!     Err(ModelRegistryError::ModelNotApproved { .. }) => { /* … */ }
//!     Err(other) => return Err(other.into()),
//! }
//! ```

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

pub mod api;
pub mod errors;
pub mod field;
pub mod gts;
pub mod models;
pub mod odata;
pub mod reason;

pub use api::ModelRegistryClientV1;
pub use errors::ModelRegistryError;
pub use gts::{MODEL_RESOURCE_TYPE, PROVIDER_BASE_TYPE, PROVIDER_RESOURCE_TYPE};
pub use models::{
    ApprovalState,
    ApprovalV1,
    ContextWindow,
    CreateModelRequestV1,
    CreateProviderRequestV1,
    CreateProviderRequestV1Builder,
    // default inference parameters (user-facing)
    DefaultInferenceParametersV1,
    DisabledCapabilities,
    DisabledMediaCapability,
    DisabledOutputCapability,
    DisabledReasoningCapability,
    DisabledSamplingCapability,
    DisabledToolsCapability,
    DisabledWebSearchCapability,
    // shared media-typed capability (vision, file_input, image_generation,
    // audio_input, audio_output)
    MediaCapability,
    ModelCapabilities,
    ModelCostV1,
    ModelEntryV1,
    ModelInfoV1,
    ModelPerformance,
    ModelV1,
    // tagged-object form of `ToolChoice` (`{"type":"function","name":"…"}`)
    NamedToolChoice,
    OutputCapability,
    ProviderEntryV1,
    ProviderTypeV1,
    ProviderV1,
    ReasoningCapability,
    ReasoningConfig,
    ReasoningEffort,
    ReasoningSummary,
    SamplingCapability,
    ServiceTier,
    SharingMode,
    SupportedApi,
    TextFormat,
    TextFormatKind,
    TextVerbosity,
    ToolChoice,
    // bare-string mode form of `ToolChoice` (`"auto"`/`"required"`/`"none"`)
    ToolChoiceMode,
    ToolsCapability,
    TruncationStrategy,
    UpdateModelRequestV1,
    UpdateProviderRequestV1,
    WebSearchCapability,
};
pub use odata::{MODEL_FILTER_ONLY_FIELDS, ModelFilterField, ProviderFilterField};
