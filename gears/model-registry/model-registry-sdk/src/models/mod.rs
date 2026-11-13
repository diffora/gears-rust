// Created: 2026-05-06 by Constructor Tech
// Updated: 2026-11-05 by Constructor Tech
//! Public models of the `model-registry` gear (DESIGN §3.1).
//!
//! Layout:
//! - [`common`] — enums ([`SharingMode`], [`ApprovalState`], [`SupportedApi`],
//!   …), capabilities, context window, cost ([`ModelCostV1`]), performance.
//! - [`default_parameters`] — [`DefaultInferenceParametersV1`] and its
//!   supporting types.
//! - [`info`] — [`ModelInfoV1`].
//! - [`entity`] — [`ProviderV1`], [`ModelV1`], [`ApprovalV1`], and the
//!   management entries [`ProviderEntryV1`] / [`ModelEntryV1`].
//! - [`provider_type`] — [`ProviderTypeV1`], the provider base GTS type.
//! - [`request`] — create and update requests.

pub mod common;
pub mod default_parameters;
pub mod entity;
pub mod info;
pub mod provider_type;
pub mod request;

pub use common::{
    ApprovalState, ContextWindow, DisabledCapabilities, DisabledMediaCapability,
    DisabledOutputCapability, DisabledReasoningCapability, DisabledSamplingCapability,
    DisabledToolsCapability, DisabledWebSearchCapability, MediaCapability, ModelCapabilities,
    ModelCostV1, ModelPerformance, OutputCapability, ReasoningCapability, ReasoningEffort,
    SamplingCapability, ServiceTier, SharingMode, SupportedApi, ToolsCapability,
    WebSearchCapability,
};

pub use default_parameters::{
    DefaultInferenceParametersV1, NamedToolChoice, ReasoningConfig, ReasoningSummary, TextFormat,
    TextFormatKind, TextVerbosity, ToolChoice, ToolChoiceMode, TruncationStrategy,
};

pub use entity::{ApprovalV1, ModelEntryV1, ModelV1, ProviderEntryV1, ProviderV1};

pub use info::ModelInfoV1;

pub use provider_type::ProviderTypeV1;

pub use request::{
    CreateModelRequestV1, CreateProviderRequestV1, CreateProviderRequestV1Builder,
    UpdateModelRequestV1, UpdateProviderRequestV1,
};
