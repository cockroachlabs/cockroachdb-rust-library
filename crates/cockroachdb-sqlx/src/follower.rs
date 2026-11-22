//! Follower reads via transaction-scoped `AS OF SYSTEM TIME`.
//!
//! The clause scopes the whole transaction (`SET TRANSACTION AS OF SYSTEM TIME
//! …`) rather than being appended to a query string — the latter is a syntax
//! error in CockroachDB and keeps bound parameters working.

use crate::pool::Pool;
use cockroachdb_core::follower_read_clause;
use sqlx::postgres::PgRow;
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

type FollowerBody<'c, T> = Pin<Box<dyn Future<Output = Result<T, sqlx::Error>> + Send + 'c>>;

impl Pool {
    /// Runs `f` inside a transaction scoped to a follower-read timestamp.
    ///
    /// `staleness == Duration::ZERO` uses `follower_read_timestamp()`; a positive
    /// value uses a fixed `-<staleness>` interval. Bound parameters work normally.
    pub async fn follower_read_tx<T, F>(&self, staleness: Duration, f: F) -> Result<T, sqlx::Error>
    where
        F: for<'c> Fn(&'c mut sqlx::PgConnection) -> FollowerBody<'c, T> + Send + Sync,
    {
        let clause =
            follower_read_clause(staleness).map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
        let start = Instant::now();
        let mut tx = self.pool.begin().await?;

        let set = format!("SET TRANSACTION {}", clause);
        if let Err(e) = sqlx::query(&set).execute(&mut *tx).await {
            let _ = tx.rollback().await;
            return Err(e);
        }

        match f(&mut tx).await {
            Ok(value) => {
                tx.commit().await?;
                if let Some(m) = &self.metrics {
                    m.record_follower_read(start.elapsed());
                }
                Ok(value)
            }
            Err(e) => {
                let _ = tx.rollback().await;
                Err(e)
            }
        }
    }

    /// Convenience: runs a parameterless `query` as a follower read using the
    /// pool's default staleness and returns all rows.
    pub async fn fetch_all_follower_read(&self, query: &str) -> Result<Vec<PgRow>, sqlx::Error> {
        self.fetch_all_follower_read_as(self.config.staleness, query)
            .await
    }

    /// Like [`Pool::fetch_all_follower_read`] but with an explicit staleness.
    pub async fn fetch_all_follower_read_as(
        &self,
        staleness: Duration,
        query: &str,
    ) -> Result<Vec<PgRow>, sqlx::Error> {
        let q = query.to_string();
        self.follower_read_tx(staleness, move |conn| {
            let q = q.clone();
            Box::pin(async move { sqlx::query(&q).fetch_all(&mut *conn).await })
        })
        .await
    }
}
