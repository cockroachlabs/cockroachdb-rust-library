//! [`VectorStore`] — executes vector search and schema setup against a pool.

use crate::query;
use crate::{AiError, BoxError, DistanceMetric};
use async_trait::async_trait;
use cockroachdb_sqlx::Pool;
use sqlx::Row;

/// Schema/behaviour configuration for a [`VectorStore`].
#[derive(Debug, Clone)]
pub struct VectorConfig {
    /// Target table name.
    pub table: String,
    /// Primary-key column.
    pub id_col: String,
    /// Text content column.
    pub content_col: String,
    /// `VECTOR` embedding column.
    pub embedding_col: String,
    /// Embedding dimensionality.
    pub dims: u32,
    /// Default distance metric for searches.
    pub metric: DistanceMetric,
}

impl VectorConfig {
    /// Config with default column names (`id` / `content` / `embedding`) and L2.
    pub fn new(table: impl Into<String>, dims: u32) -> Self {
        Self {
            table: table.into(),
            id_col: "id".into(),
            content_col: "content".into(),
            embedding_col: "embedding".into(),
            dims,
            metric: DistanceMetric::L2,
        }
    }

    /// Overrides the distance metric.
    pub fn with_metric(mut self, metric: DistanceMetric) -> Self {
        self.metric = metric;
        self
    }

    /// Overrides the column names.
    pub fn with_columns(
        mut self,
        id: impl Into<String>,
        content: impl Into<String>,
        embedding: impl Into<String>,
    ) -> Self {
        self.id_col = id.into();
        self.content_col = content.into();
        self.embedding_col = embedding.into();
        self
    }

    fn index_name(&self) -> String {
        let sanitize = |s: &str| s.replace(|c: char| !c.is_alphanumeric(), "_");
        format!(
            "{}_{}_idx",
            sanitize(&self.table),
            sanitize(&self.embedding_col)
        )
    }
}

/// One search hit.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    /// Row id (as text).
    pub id: String,
    /// Stored content.
    pub content: String,
    /// Distance under the configured metric (smaller is closer).
    pub distance: f64,
}

/// Produces an embedding vector for a piece of text. Bring your own model.
#[async_trait]
pub trait Embedder: Send + Sync {
    /// Embeds `text` into a vector.
    async fn embed(&self, text: &str) -> Result<Vec<f32>, BoxError>;
}

/// A vector store over a [`Pool`].
#[derive(Debug, Clone)]
pub struct VectorStore {
    pool: Pool,
    config: VectorConfig,
}

impl VectorStore {
    /// Creates a store.
    pub fn new(pool: Pool, config: VectorConfig) -> Self {
        Self { pool, config }
    }

    /// The configuration.
    pub fn config(&self) -> &VectorConfig {
        &self.config
    }

    /// Creates the backing table if absent (autocommit — no enclosing transaction,
    /// which CockroachDB requires for this DDL).
    pub async fn ensure_table(&self) -> Result<(), sqlx::Error> {
        let sql = query::create_table(
            &self.config.table,
            &self.config.id_col,
            &self.config.content_col,
            &self.config.embedding_col,
            self.config.dims,
        );
        sqlx::query(&sql).execute(self.pool.raw_pool()).await?;
        Ok(())
    }

    /// Creates the C-SPANN vector index if absent. **Requires CockroachDB v26.2+.**
    pub async fn ensure_index(&self) -> Result<(), sqlx::Error> {
        let sql = query::create_vector_index(
            &self.config.index_name(),
            &self.config.table,
            &self.config.embedding_col,
        );
        sqlx::query(&sql).execute(self.pool.raw_pool()).await?;
        Ok(())
    }

    /// Creates the table and the vector index.
    pub async fn ensure_schema(&self) -> Result<(), sqlx::Error> {
        self.ensure_table().await?;
        self.ensure_index().await
    }

    /// Inserts content with its embedding, returning the generated id.
    pub async fn add(&self, content: &str, embedding: &[f32]) -> Result<String, sqlx::Error> {
        self.add_with_metadata(content, embedding, None).await
    }

    /// Inserts content with an embedding and optional JSON metadata.
    pub async fn add_with_metadata(
        &self,
        content: &str,
        embedding: &[f32],
        metadata: Option<serde_json::Value>,
    ) -> Result<String, sqlx::Error> {
        let sql = query::insert(
            &self.config.table,
            &self.config.id_col,
            &self.config.content_col,
            &self.config.embedding_col,
        );
        let meta = metadata.map(|v| v.to_string());
        let row = sqlx::query(&sql)
            .bind(content)
            .bind(query::vector_literal(embedding))
            .bind(meta)
            .fetch_one(self.pool.raw_pool())
            .await?;
        row.try_get::<String, _>(0)
    }

    /// Nearest-neighbour search returning the top `k` rows.
    pub async fn search(
        &self,
        query_embedding: &[f32],
        k: i64,
    ) -> Result<Vec<SearchResult>, sqlx::Error> {
        self.run_search(query_embedding, k, None, &[]).await
    }

    /// Hybrid search: nearest neighbours restricted by a SQL `filter_sql` clause.
    ///
    /// Placeholders in `filter_sql` start at `$3` and are bound as text from
    /// `params` (e.g. `filter_sql = "metadata->>'lang' = $3"`, `params = ["en"]`).
    pub async fn search_with_filter(
        &self,
        query_embedding: &[f32],
        k: i64,
        filter_sql: &str,
        params: &[String],
    ) -> Result<Vec<SearchResult>, sqlx::Error> {
        self.run_search(query_embedding, k, Some(filter_sql), params)
            .await
    }

    /// Embeds `query` with `embedder`, then searches.
    pub async fn semantic_search<E: Embedder + ?Sized>(
        &self,
        embedder: &E,
        query: &str,
        k: i64,
    ) -> Result<Vec<SearchResult>, AiError> {
        let embedding = embedder.embed(query).await.map_err(AiError::Embedding)?;
        Ok(self.search(&embedding, k).await?)
    }

    async fn run_search(
        &self,
        query_embedding: &[f32],
        k: i64,
        filter_sql: Option<&str>,
        params: &[String],
    ) -> Result<Vec<SearchResult>, sqlx::Error> {
        let sql = query::search(
            &self.config.table,
            &self.config.id_col,
            &self.config.content_col,
            &self.config.embedding_col,
            self.config.metric,
            filter_sql,
        );
        let mut q = sqlx::query(&sql)
            .bind(query::vector_literal(query_embedding))
            .bind(k);
        for p in params {
            q = q.bind(p.clone());
        }
        let rows = q.fetch_all(self.pool.raw_pool()).await?;
        rows.iter()
            .map(|r| {
                Ok(SearchResult {
                    id: r.try_get("id")?,
                    content: r.try_get("content")?,
                    distance: r.try_get("distance")?,
                })
            })
            .collect()
    }
}

/// Joins the content of search results into a single RAG context block.
pub fn rag_context(results: &[SearchResult], separator: &str) -> String {
    results
        .iter()
        .map(|r| r.content.as_str())
        .collect::<Vec<_>>()
        .join(separator)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_name_is_sanitized() {
        let cfg = VectorConfig::new("my.docs", 3);
        assert_eq!(cfg.index_name(), "my_docs_embedding_idx");
    }

    #[test]
    fn rag_context_joins_content() {
        let results = vec![
            SearchResult {
                id: "1".into(),
                content: "alpha".into(),
                distance: 0.1,
            },
            SearchResult {
                id: "2".into(),
                content: "beta".into(),
                distance: 0.2,
            },
        ];
        assert_eq!(rag_context(&results, "\n---\n"), "alpha\n---\nbeta");
    }
}
