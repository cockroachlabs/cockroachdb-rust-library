//! Vector search and RAG retrieval for CockroachDB.
//!
//! Uses CockroachDB's native `VECTOR` type and C-SPANN vector index. Mirrors the
//! Go library's `ai` package:
//!
//! * [`DistanceMetric`] + [`query`] build the search/DDL SQL (pure, unit-tested).
//! * [`VectorStore`] runs it against a [`cockroachdb_sqlx::Pool`]: schema setup
//!   in autocommit, buffered `search`, hybrid [`VectorStore::search_with_filter`].
//! * [`Embedder`] + [`VectorStore::semantic_search`] + [`rag_context`] cover the
//!   retrieval half of RAG (bring your own embedding model / LLM).

mod distance;
pub mod query;
mod store;

pub use distance::DistanceMetric;
pub use store::{rag_context, Embedder, SearchResult, VectorConfig, VectorStore};

/// A boxed error (e.g. from an [`Embedder`]).
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Errors from semantic search (embedding + query).
#[derive(Debug, thiserror::Error)]
pub enum AiError {
    /// The embedding model failed.
    #[error("embedding failed: {0}")]
    Embedding(#[source] BoxError),
    /// The database query failed.
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}
