//! Transaction execution with automatic retry on serialization conflicts.

use crate::error::is_retryable;
use crate::pool::Pool;
use cockroachdb_core::{IsolationLevel, RetryPolicy};
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

/// Per-transaction overrides for [`Pool::execute_tx_with_options`].
#[derive(Debug, Clone, Default)]
pub struct ExecuteTxOptions {
    /// Override the pool's default read-only mode.
    pub read_only: Option<bool>,
    /// Override the pool's default isolation level.
    pub isolation: Option<IsolationLevel>,
}

/// A re-runnable transaction body: given a connection, returns a boxed future.
///
/// The closure is [`Fn`] because it may run **more than once** (each retry), so
/// capture data by reference or clone it inside the async block rather than
/// moving owned values into the closure.
type TxBody<'c, T> = Pin<Box<dyn Future<Output = Result<T, sqlx::Error>> + Send + 'c>>;

impl Pool {
    /// Runs `f` in a transaction, retrying on serialization conflicts (`40001`)
    /// using the pool's [`RetryPolicy`]. Commits on success, rolls back and
    /// retries (with backoff) on a retryable error, and returns the last error
    /// once retries are exhausted.
    pub async fn execute_tx<T, F>(&self, f: F) -> Result<T, sqlx::Error>
    where
        F: for<'c> Fn(&'c mut sqlx::PgConnection) -> TxBody<'c, T> + Send + Sync,
    {
        self.execute_tx_with_options(f, ExecuteTxOptions::default())
            .await
    }

    /// Like [`Pool::execute_tx`] but with per-call isolation / read-only overrides.
    pub async fn execute_tx_with_options<T, F>(
        &self,
        f: F,
        opts: ExecuteTxOptions,
    ) -> Result<T, sqlx::Error>
    where
        F: for<'c> Fn(&'c mut sqlx::PgConnection) -> TxBody<'c, T> + Send + Sync,
    {
        let isolation = opts.isolation.unwrap_or(self.config.isolation);
        let read_only = opts.read_only.unwrap_or(self.config.read_only);
        let policy = self.config.retry.clone();
        let start = Instant::now();
        let mut attempt: u32 = 0;

        loop {
            let mut tx = self.pool.begin().await?;

            // Isolation level must be set before any other statement in the tx.
            let set_iso = format!("SET TRANSACTION ISOLATION LEVEL {}", isolation.as_sql());
            if let Err(e) = sqlx::query(&set_iso).execute(&mut *tx).await {
                let _ = tx.rollback().await;
                return Err(e);
            }
            if read_only {
                if let Err(e) = sqlx::query("SET TRANSACTION READ ONLY")
                    .execute(&mut *tx)
                    .await
                {
                    let _ = tx.rollback().await;
                    return Err(e);
                }
            }

            let result = f(&mut tx).await;

            match result {
                Ok(value) => match tx.commit().await {
                    Ok(()) => {
                        self.record_tx(start.elapsed(), attempt, true);
                        return Ok(value);
                    }
                    // Commit itself can surface a serialization conflict.
                    Err(e) if is_retryable(&e) && attempt < policy.max_retries => {
                        attempt += 1;
                        self.backoff(&policy, attempt).await;
                    }
                    Err(e) => {
                        self.record_tx(start.elapsed(), attempt, false);
                        return Err(e);
                    }
                },
                Err(e) => {
                    let _ = tx.rollback().await;
                    if is_retryable(&e) && attempt < policy.max_retries {
                        attempt += 1;
                        self.backoff(&policy, attempt).await;
                    } else {
                        self.record_tx(start.elapsed(), attempt, false);
                        return Err(e);
                    }
                }
            }
        }
    }

    async fn backoff(&self, policy: &RetryPolicy, attempt: u32) {
        let delay = policy.backoff_delay(attempt - 1);
        if let Some(m) = &self.metrics {
            m.record_retry(delay);
        }
        tokio::time::sleep(delay).await;
    }

    pub(crate) fn record_tx(&self, elapsed: Duration, retries: u32, success: bool) {
        if let Some(m) = &self.metrics {
            m.record_transaction(elapsed, retries, success);
        }
    }
}
