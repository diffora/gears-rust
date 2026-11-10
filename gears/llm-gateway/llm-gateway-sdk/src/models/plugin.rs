//! Models for the provider-plugin interface.
//!
//! A provider plugin (see [`crate::plugin_api::LlmGatewayProviderPluginClientV1`])
//! isolates one LLM provider behind a common trait. These types carry the
//! per-call context the core Gateway hands to a plugin and the
//! integration-level capabilities a plugin reports. Per-model feature
//! capabilities (vision, function calling, streaming, …) are owned by Model
//! Registry (`ModelCapabilities`), and per-model parameter support is decided
//! by the Gateway service; both are validated by the core before dispatch and
//! are deliberately not part of the plugin contract.

use model_registry_sdk::ModelInfoV1;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

/// Per-call context the core Gateway passes to every provider-plugin method.
///
/// Carries what translation and transport need without the plugin reaching into
/// Model Registry or `CredStore` itself. The core builds it from the resolved
/// model, the provider API key, and the request being served. It is an
/// in-process value and is deliberately not serializable, so the key cannot
/// leak through serialization.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ProviderCallCtx {
    /// The resolved model info the core fetched from Model Registry, carried
    /// unchanged. Its `gts_type` is the provider type, the authoritative
    /// provider routing key, and equals the resolving plugin's declared
    /// `provider_type`. The plugin reads `provider_model_id`, `base_url`,
    /// generic settings, and the `cost` rates from it, and narrows the opaque
    /// `provider_settings` to its own settings type via
    /// [`ProviderCallCtx::provider_settings`].
    pub model_info: ModelInfoV1,

    /// Gateway request correlation id (the response `id`), propagated for
    /// tracing across usage, error, and audit events
    /// (`cpt-cf-llm-gateway-fr-request-traceability-v1`).
    pub request_id: String,

    /// Provider API key the core fetched from `CredStore` by the model's secret
    /// reference (`cpt-cf-llm-gateway-fr-provider-credentials-v1`). The plugin
    /// sets it on the outbound provider request, which goes only through OAGW,
    /// over HTTPS, to the model's destination (`base_url`, or the provider's
    /// default URL).
    /// `None` for models without a secret reference (authentication handled by
    /// OAGW, local provider). The
    /// key is redacted in `Debug`, zeroed on drop, and must never be logged or
    /// persisted.
    pub api_key: Option<SecretString>,

    /// Per-request cancellation token. The core cancels it on consumer
    /// disconnect, on a provider call or stream timeout, and at the shutdown
    /// drain deadline (`cpt-cf-llm-gateway-seq-stream-cancellation-v1`). The
    /// plugin checks it between provider reads, for sync and streaming calls
    /// alike, and on cancellation stops reading and drops the provider
    /// connection so OAGW closes it.
    ///
    /// Clones share the same cancellation state: calling `.cancel()` on any
    /// clone cancels the whole request. Derive child tokens with
    /// [`CancellationToken::child_token`] for sub-tasks that need independent
    /// cancellation.
    pub cancel: CancellationToken,
}

impl ProviderCallCtx {
    /// Call context for request `request_id` on `model_info`, with no
    /// provider API key.
    #[must_use]
    pub fn new(
        model_info: ModelInfoV1,
        request_id: impl Into<String>,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            model_info,
            request_id: request_id.into(),
            api_key: None,
            cancel,
        }
    }

    /// Set the provider API key fetched from `CredStore`.
    #[must_use]
    pub fn with_api_key(mut self, api_key: SecretString) -> Self {
        self.api_key = Some(api_key);
        self
    }

    /// Narrow `model_info.provider_settings` to the plugin's settings type `S`,
    /// validating `model_info.gts_type` against `S`'s GTS type id.
    ///
    /// # Errors
    ///
    /// - [`gts::NarrowError::SchemaId`] when `model_info.gts_type` does not match
    ///   `S`'s GTS type id (the plugin was handed a model of another provider).
    /// - [`gts::NarrowError::Deserialize`] when `provider_settings` can't be
    ///   deserialized into `S`.
    pub fn provider_settings<S>(&self) -> Result<S, gts::NarrowError>
    where
        S: gts::GtsSchema,
        for<'de> S: gts::GtsDeserialize<'de>,
    {
        gts::try_narrow::<S>(
            self.model_info.gts_type.as_ref(),
            self.model_info.provider_settings.clone(),
        )
    }
}

/// How a provider plugin consumes media referenced by input content parts
/// (`input_image`, `input_file`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MediaInputMode {
    /// Base64 data and external URLs are forwarded to the provider as-is; URLs
    /// stay URLs (`cpt-cf-llm-gateway-fr-vision-v1`).
    UrlForward,
    /// The provider accepts only bytes: the core fetches external URLs before
    /// dispatch and hands the plugin base64 content. `FileStorage` URLs are
    /// resolved the same way from P2 (`cpt-cf-llm-gateway-fr-filestorage-media-v1`).
    PrefetchRequired,
}

/// Integration-level capabilities a provider plugin reports.
///
/// These describe how the provider *integration* behaves — concerns Model
/// Registry does not track. Per-model feature capabilities live in
/// `ModelInfoV1.capabilities` / `supported_api` and are validated by the core
/// together with these (`cpt-cf-llm-gateway-fr-model-capability-check-v1`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[non_exhaustive]
pub struct ProviderPluginCapabilities {
    /// Whether the provider integration can stream responses at all
    /// (independent of any individual model's `streaming` flag).
    pub streaming_transport: bool,

    /// How the plugin expects media input to be delivered.
    pub media_input: MediaInputMode,

    /// Whether the plugin maps the Gateway web search tool (`cf_gears:web_search`)
    /// to a native provider web search tool (`cpt-cf-llm-gateway-fr-web-search-v1`).
    pub web_search: bool,
}

impl ProviderPluginCapabilities {
    /// Capabilities with the given media input mode, no streaming transport,
    /// and no web search.
    #[must_use]
    pub fn new(media_input: MediaInputMode) -> Self {
        Self {
            streaming_transport: false,
            media_input,
            web_search: false,
        }
    }

    /// The provider integration can stream responses.
    #[must_use]
    pub fn with_streaming_transport(mut self) -> Self {
        self.streaming_transport = true;
        self
    }

    /// The plugin maps `cf_gears:web_search` to a native provider tool.
    #[must_use]
    pub fn with_web_search(mut self) -> Self {
        self.web_search = true;
        self
    }
}

#[cfg(test)]
mod tests {
    use model_registry_sdk::models::{ContextWindow, ModelInfoV1, ProviderTypeV1};
    use tokio_util::sync::CancellationToken;
    use toolkit_gts::gts_id;

    use super::ProviderCallCtx;

    #[derive(Debug)]
    #[toolkit_gts::gts_type_schema(
        dir_path = "schemas",
        base = ProviderTypeV1,
        type_id = gts_id!("cf.genai.model.provider.v1~cf.genai._.test.v1~"),
        description = "Test provider settings",
        properties = "oagw_alias"
    )]
    pub struct TestProviderSettingsV1 {
        pub oagw_alias: String,
    }

    fn ctx(gts_type: &str) -> ProviderCallCtx {
        let mut info = ModelInfoV1::new(
            gts::GtsTypeId::new(gts_type),
            "test-model",
            "Test model",
            ContextWindow::new(8_192),
        );
        info.provider_settings = serde_json::json!({ "oagw_alias": "test-upstream" });
        ProviderCallCtx::new(info, "req-1", CancellationToken::new())
    }

    #[test]
    fn provider_settings_narrows_to_the_plugin_type() {
        let settings: TestProviderSettingsV1 =
            ctx(gts_id!("cf.genai.model.provider.v1~cf.genai._.test.v1~"))
                .provider_settings()
                .expect("narrows");
        assert_eq!(settings.oagw_alias, "test-upstream");
    }

    #[test]
    fn provider_settings_rejects_another_provider_type() {
        let err = ctx(gts_id!("cf.genai.model.provider.v1~cf.genai._.other.v1~"))
            .provider_settings::<TestProviderSettingsV1>()
            .expect_err("type mismatch");
        assert!(matches!(err, gts::NarrowError::SchemaId { .. }), "{err}");
    }
}
