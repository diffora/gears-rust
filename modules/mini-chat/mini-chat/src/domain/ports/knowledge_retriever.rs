//! Domain port for pluggable knowledge retrieval (RAG).
//!
//! The [`KnowledgeRetriever`] trait decouples the agentic `search_knowledge`
//! loop from any specific vector-store backend. The Azure Foundry
//! implementation lives in `infra/llm/providers/azure_knowledge_retriever.rs`.

use async_trait::async_trait;
use modkit_macros::domain_model;

/// A single retrieved chunk returned by a knowledge search call.
#[domain_model]
#[derive(Debug, Clone)]
pub struct RetrievedChunk {
    /// Stable URI for citation attribution.
    ///
    /// Format: `kb://chat/{chat_id}/doc/{file_id}` (file-level granularity).
    /// Implementations MAY append `#chunk/{i}` when they retain per-chunk
    /// addressing; the current Azure implementation emits the file-level
    /// form. Downstream citation handling treats the suffix as optional.
    pub source_uri: String,
    /// Display title (usually the filename).
    pub title: String,
    /// Text content of the chunk.
    pub text: String,
    /// Relevance score from the vector store (higher = more relevant).
    pub score: f32,
}

/// Parameters for a single knowledge retrieval call.
#[domain_model]
#[derive(Debug, Clone)]
pub struct RetrievalRequest {
    /// Natural-language query to search for.
    pub query: String,
    /// Maximum number of chunks to return (top-k).
    pub top_k: usize,
    /// Chat ID — used to build stable `kb://chat/{chat_id}/...` source URIs.
    pub chat_id: String,
    /// Vector store ID to search within.
    pub vector_store_id: String,
    /// OAGW upstream alias for the provider holding the vector store.
    pub upstream_alias: String,
    /// Azure API version query parameter (e.g. `"2025-03-01-preview"`).
    pub api_version: String,
}

/// Errors from knowledge retrieval operations.
#[domain_model]
#[derive(Debug, thiserror::Error)]
pub enum RetrievalError {
    /// Provider explicitly rejected the request (4xx).
    #[error("retrieval provider rejected: {0}")]
    Rejected(String),
    /// Provider unavailable or transient failure (5xx, timeout).
    #[error("retrieval provider unavailable: {0}")]
    Unavailable(String),
    /// Configuration error (missing alias, bad credentials, etc.).
    #[error("retrieval configuration error: {0}")]
    Configuration(String),
}

/// Port for pluggable knowledge search / RAG backends.
///
/// Implementations return chunks sorted by relevance score (descending).
/// Domain services depend only on this trait — no knowledge of HTTP details.
#[async_trait]
pub trait KnowledgeRetriever: Send + Sync {
    async fn retrieve(
        &self,
        ctx: modkit_security::SecurityContext,
        req: RetrievalRequest,
    ) -> Result<Vec<RetrievedChunk>, RetrievalError>;
}
