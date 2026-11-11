// Created: 2026-08-06 by Constructor Tech
// Updated: 2026-11-05 by Constructor Tech
//! `OData` filter / order surface of the `model-registry` list methods
//! (DESIGN §3.3).
//!
//! The filterable wire surface is declared once per resource, as an annotated
//! query struct; `#[derive(ODataFilterable)]` generates the `FilterField` enum
//! with its `FIELDS` / `name()` / `kind()` impl. The enums are the public
//! contract shared by three consumers: the `$filter` / `$orderby` allowlist on
//! [`crate::ModelRegistryClientV1`]'s [`ODataQuery`] arguments, the `OpenAPI`
//! query-parameter documentation, and the gear's `FieldToColumn` binding to
//! real database columns. Both model lists (`list_available_models`,
//! `list_models`) share [`ModelSchema`]; the provider list uses
//! [`ProviderSchema`].
//!
//! Each filter field maps to a real column (the toolkit `OData` layer has no
//! JSON-path filtering and no joins); the column binding lives in the gear
//! crate. The nullable model fields in [`MODEL_FILTER_ONLY_FIELDS`] are
//! filterable but not orderable.
//!
//! [`ODataQuery`]: toolkit_odata::ODataQuery
//!
//! # Building a query
//!
//! ```rust,ignore
//! use model_registry_sdk::odata::{
//!     ModelSchema, MODEL_CANONICAL_ID, MODEL_STREAMING, MODEL_VENDOR, QueryBuilder, SortDir,
//! };
//!
//! let query = QueryBuilder::<ModelSchema>::new()
//!     .filter(MODEL_STREAMING.eq(true).and(MODEL_VENDOR.eq("OpenAI")))
//!     .order_by(MODEL_CANONICAL_ID, SortDir::Asc)
//!     .page_size(50)
//!     .build();
//! let page = client.list_available_models(&ctx, &query).await?;
//! ```
//!
//! `build()` computes the `filter_hash` cursor pagination validates, so a
//! builder-constructed query is safe to page with. Values become AST literals
//! directly — there is no `$filter` text to quote or escape.
//!
//! `QueryBuilder::select` has no effect on the SDK methods: they always
//! return whole records, and `$select` is applied by the REST layer only.

mod models;
mod providers;
mod values;

pub use models::{
    MODEL_API_BATCH, MODEL_API_COMPLETION, MODEL_API_EMBEDDING, MODEL_APPROVAL_DECIDED_AT,
    MODEL_APPROVAL_STATE, MODEL_ARCHITECTURE, MODEL_AUDIO_INPUT, MODEL_AUDIO_OUTPUT,
    MODEL_CANONICAL_ID, MODEL_CODE_INTERPRETER, MODEL_CREATED_AT, MODEL_DISPLAY_NAME, MODEL_FAMILY,
    MODEL_FILE_INPUT, MODEL_FILTER_ONLY_FIELDS, MODEL_FORMAT, MODEL_GTS_TYPE, MODEL_HOSTED_BY,
    MODEL_ID, MODEL_IMAGE_GENERATION, MODEL_JSON_SCHEMA, MODEL_LAST_RELEASE_AT, MODEL_MANAGED,
    MODEL_MAX_INPUT_TOKENS, MODEL_MAX_OUTPUT_TOKENS, MODEL_MODEL_VERSION, MODEL_NAME,
    MODEL_PROVIDER_ID, MODEL_PROVIDER_MODEL_ID, MODEL_REASONING_EFFORT, MODEL_REGION,
    MODEL_SHARING_MODE, MODEL_SORT_ORDER, MODEL_STREAMING, MODEL_TENANT_ID, MODEL_TOOLS,
    MODEL_UPDATED_AT, MODEL_VENDOR, MODEL_VISION, MODEL_WEB_SEARCH, ModelFilterField, ModelQuery,
    ModelSchema,
};
pub use providers::{
    PROVIDER_CREATED_AT, PROVIDER_ENABLED, PROVIDER_GTS_TYPE, PROVIDER_ID, PROVIDER_MANAGED,
    PROVIDER_NAME, PROVIDER_SHARING_MODE, PROVIDER_SLUG, PROVIDER_TENANT_ID, PROVIDER_UPDATED_AT,
    ProviderFilterField, ProviderQuery, ProviderSchema,
};

// Re-exported so a caller needs one import path to build a query.
pub use toolkit_odata::{FieldRef, QueryBuilder, Schema, SortDir};
