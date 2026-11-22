//! The [`Pool`] — a CockroachDB-aware wrapper over [`sqlx::PgPool`].

use async_trait::async_trait;
use cockroachdb_core::health::BoxError;
use cockroachdb_core::{Config, HealthChecker, MetricsRecorder};
use std::sync::Arc;

/// A connection pool with CockroachDB retry, follower-read, and metrics support.
#[derive(Clone)]
pub struct Pool {
    pub(crate) pool: sqlx::PgPool,
    pub(crate) config: Config,
    pub(crate) metrics: Option<Arc<dyn MetricsRecorder>>,
}

impl std::fmt::Debug for Pool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pool")
            .field("config", &self.config)
            .field("metrics", &self.metrics.is_some())
            .finish_non_exhaustive()
    }
}

impl Pool {
    /// Connects to CockroachDB using `conn_str` and the given [`Config`].
    pub async fn connect(conn_str: &str, config: Config) -> Result<Self, sqlx::Error> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect(conn_str)
            .await?;
        Ok(Self {
            pool,
            config,
            metrics: None,
        })
    }

    /// Wraps an existing [`sqlx::PgPool`].
    pub fn from_pool(pool: sqlx::PgPool, config: Config) -> Self {
        Self {
            pool,
            config,
            metrics: None,
        }
    }

    /// Attaches a metrics recorder (transactions, retries, follower reads).
    pub fn with_metrics(mut self, recorder: Arc<dyn MetricsRecorder>) -> Self {
        self.metrics = Some(recorder);
        self
    }

    /// The effective configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Borrows the underlying sqlx pool for queries outside this library's API.
    pub fn raw_pool(&self) -> &sqlx::PgPool {
        &self.pool
    }

    /// Closes the pool and all its connections.
    pub async fn close(&self) {
        self.pool.close().await;
    }

    /// Issues `SELECT 1` to confirm the cluster is reachable.
    pub async fn ping(&self) -> Result<(), sqlx::Error> {
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .map(|_| ())
    }
}

#[async_trait]
impl HealthChecker for Pool {
    async fn check(&self) -> Result<(), BoxError> {
        self.ping().await.map_err(|e| Box::new(e) as BoxError)
    }
}
