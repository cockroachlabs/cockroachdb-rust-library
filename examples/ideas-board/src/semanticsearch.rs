//! Semantic-search track: embed each idea's title into a vector and find the
//! nearest ideas to a free-text query. Uses CockroachDB's native `VECTOR` type
//! (v25.2+) for brute-force search; the C-SPANN index (v26.2+) is best-effort.

// tag::vector-search[]
use crate::idea::Idea;
use async_trait::async_trait;
use cockroachdb_ai::{
    AiError, BoxError, DistanceMetric, Embedder, SearchResult, VectorConfig, VectorStore,
};
use cockroachdb_sqlx::Pool;
use std::hash::{Hash, Hasher};

const DIMS: u32 = 32;

/// A deterministic, dependency-free embedder: it hashes each word into one of
/// `DIMS` buckets (a bag-of-words sketch) and L2-normalizes the result. Real apps
/// swap in an embedding model by implementing `Embedder` — the rest is unchanged.
pub struct HashingEmbedder {
    dims: usize,
}

impl HashingEmbedder {
    /// Creates an embedder producing `dims`-dimensional vectors.
    pub fn new(dims: usize) -> Self {
        Self { dims }
    }

    fn bucket(&self, token: &str) -> usize {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        token.hash(&mut h);
        (h.finish() % self.dims as u64) as usize
    }
}

#[async_trait]
impl Embedder for HashingEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, BoxError> {
        let mut v = vec![0f32; self.dims];
        for token in text.split_whitespace() {
            let i = self.bucket(&token.to_lowercase());
            v[i] += 1.0;
        }
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut v {
                *x /= norm;
            }
        }
        Ok(v)
    }
}

/// Stores ideas as vectors and finds the ones most similar to a query. Pairs a
/// `VectorStore` (the CockroachDB-backed index) with an `Embedder` (how text
/// becomes a vector); cosine distance ranks the hits.
pub struct SemanticIdeaService {
    store: VectorStore,
    embedder: HashingEmbedder,
}

impl SemanticIdeaService {
    /// Creates the service over a pool, using cosine distance.
    pub fn new(pool: Pool) -> Self {
        let config = VectorConfig::new("idea_docs", DIMS).with_metric(DistanceMetric::Cosine);
        Self {
            store: VectorStore::new(pool, config),
            embedder: HashingEmbedder::new(DIMS as usize),
        }
    }

    /// Creates the vector table and (best-effort) the C-SPANN index. The index
    /// needs CockroachDB v26.2+; without it, search falls back to brute force, so
    /// an index failure is logged, not fatal.
    pub async fn ensure_schema(&self) -> Result<(), sqlx::Error> {
        self.store.ensure_table().await?;
        if let Err(e) = self.store.ensure_index().await {
            eprintln!("vector index not created (needs v26.2+): {e}");
        }
        Ok(())
    }

    /// Embeds and stores each idea's title.
    pub async fn store(&self, ideas: &[Idea]) -> Result<(), AiError> {
        for idea in ideas {
            let embedding = self
                .embedder
                .embed(&idea.title)
                .await
                .map_err(AiError::Embedding)?;
            self.store.add(&idea.title, &embedding).await?;
        }
        Ok(())
    }

    /// Returns the `k` ideas most similar to `query`, nearest first.
    pub async fn find_similar(&self, query: &str, k: i64) -> Result<Vec<SearchResult>, AiError> {
        self.store.semantic_search(&self.embedder, query, k).await
    }
}
// end::vector-search[]

#[cfg(test)]
mod tests {
    // Offline: the embedder is pure. Assert it is deterministic and unit-normed,
    // and that different text yields different vectors — the properties cosine
    // search relies on. The end-to-end ranking proof is the ignored IT below.
    use super::HashingEmbedder;
    use cockroachdb_ai::Embedder;

    #[tokio::test]
    async fn embedding_is_deterministic_and_normalized() {
        let e = HashingEmbedder::new(32);
        let a = e.embed("add dark mode").await.unwrap();
        let b = e.embed("add dark mode").await.unwrap();
        assert_eq!(a, b, "same text embeds identically");
        let norm: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "vector is L2-normalized");
    }

    #[tokio::test]
    async fn distinct_text_differs() {
        let e = HashingEmbedder::new(32);
        assert_ne!(
            e.embed("export to csv").await.unwrap(),
            e.embed("full text search").await.unwrap()
        );
    }

    // tag::vector-it[]
    // End-to-end ranking proof. Needs CockroachDB v25.2+ for the VECTOR type (set
    // CRDB_URL); `#[ignore]`d otherwise. Stores three unrelated ideas, then a
    // query sharing words with one must rank that idea first. Search is
    // brute-force; the C-SPANN index is best-effort, so this runs on v25.2.
    use super::SemanticIdeaService;
    use crate::idea::Idea;
    use cockroachdb_core::Config;
    use cockroachdb_sqlx::Pool;

    fn idea(title: &str) -> Idea {
        Idea {
            id: String::new(),
            title: title.into(),
            votes: 0,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "requires CockroachDB v25.2+ with the VECTOR type (set CRDB_URL)"]
    async fn find_similar_returns_nearest_first() {
        let url = std::env::var("CRDB_URL")
            .unwrap_or_else(|_| "postgres://root@localhost:26257/defaultdb?sslmode=disable".into());
        let pool = Pool::connect(&url, Config::default())
            .await
            .expect("connect");
        let svc = SemanticIdeaService::new(pool.clone());

        if let Err(e) = svc.ensure_schema().await {
            // No VECTOR type on this cluster — nothing to prove here.
            if is_vector_unsupported(&e) {
                eprintln!("skipping: VECTOR type unsupported on this cluster");
                pool.close().await;
                return;
            }
            panic!("ensure_schema failed: {e}");
        }

        svc.store(&[
            idea("Add a dark mode theme"),
            idea("Export reports to CSV"),
            idea("Full text search over ideas"),
        ])
        .await
        .expect("store");

        let hits = svc.find_similar("dark mode", 1).await.expect("search");
        assert!(!hits.is_empty(), "expected at least one hit");
        assert_eq!(
            hits[0].content, "Add a dark mode theme",
            "the idea sharing 'dark'/'mode' must rank first"
        );
        pool.close().await;
    }

    fn is_vector_unsupported(e: &sqlx::Error) -> bool {
        let msg = e.to_string().to_lowercase();
        msg.contains("vector") && (msg.contains("unsupported") || msg.contains("unknown"))
    }
    // end::vector-it[]
}
