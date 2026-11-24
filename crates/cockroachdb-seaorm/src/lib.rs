//! SeaORM adapter for CockroachDB.
//!
//! SeaORM runs on SQLx, so this adapter adds CockroachDB's serialization-conflict
//! retry (`SQLSTATE 40001`) around SeaORM transactions and classifies [`DbErr`]
//! by reaching through to the underlying SQLSTATE (reusing the SQLx adapter's
//! classifier). Mirrors the Go library's GORM adapter.
//!
//! ```no_run
//! # use cockroachdb_seaorm::execute_tx;
//! # use cockroachdb_core::RetryPolicy;
//! # use sea_orm::{DatabaseConnection, DbErr};
//! # async fn demo(db: DatabaseConnection) -> Result<(), DbErr> {
//! let policy = RetryPolicy::default();
//! execute_tx(&db, &policy, |txn| Box::pin(async move {
//!     // ... SeaORM entity operations on `txn`, retried on 40001 ...
//!     let _ = txn;
//!     Ok(())
//! })).await
//! # }
//! ```

use cockroachdb_core::{Config, RetryPolicy};
use sea_orm::{
    Database, DatabaseConnection, DatabaseTransaction, DbErr, RuntimeErr, TransactionError,
    TransactionTrait,
};
use std::future::Future;
use std::pin::Pin;

pub mod multiregion;

pub use cockroachdb_core as core;
pub use multiregion::{
    describe, describe_plain, verify_multiregion, CrdbLocality, DeclaredLocality, Kind, Mode,
    MultiregionReport, MultiregionTable,
};

/// Extracts the Postgres `SQLSTATE` from a SeaORM error, if present.
fn sql_state(err: &DbErr) -> Option<String> {
    let runtime = match err {
        DbErr::Conn(e) | DbErr::Exec(e) | DbErr::Query(e) => e,
        _ => return None,
    };
    match runtime {
        RuntimeErr::SqlxError(e) => cockroachdb_sqlx::sql_state(e),
        RuntimeErr::Internal(_) => None,
    }
}

/// Returns `true` if the SeaORM error is a retryable CockroachDB conflict.
pub fn is_retryable(err: &DbErr) -> bool {
    sql_state(err)
        .as_deref()
        .map(cockroachdb_core::is_retryable_code)
        .unwrap_or(false)
}

/// Connects to CockroachDB and returns a SeaORM [`DatabaseConnection`].
///
/// The [`Config`]'s retry policy is applied by [`execute_tx`]; connection-pool
/// tuning beyond the URL should be set via SeaORM's `ConnectOptions` using
/// [`sea_orm::Database::connect`] directly.
pub async fn connect(url: &str, _config: &Config) -> Result<DatabaseConnection, DbErr> {
    Database::connect(url).await
}

/// Runs a SeaORM transaction, retrying the whole closure on serialization
/// conflicts (`40001`) per `policy`.
pub async fn execute_tx<T, F>(
    db: &DatabaseConnection,
    policy: &RetryPolicy,
    f: F,
) -> Result<T, DbErr>
where
    F: for<'c> Fn(
            &'c DatabaseTransaction,
        ) -> Pin<Box<dyn Future<Output = Result<T, DbErr>> + Send + 'c>>
        + Send
        + Sync,
    T: Send,
{
    let f = &f;
    retry_loop(policy, is_retryable, || async move {
        db.transaction::<_, T, DbErr>(|txn| f(txn))
            .await
            .map_err(|e| match e {
                TransactionError::Connection(db) => db,
                TransactionError::Transaction(db) => db,
            })
    })
    .await
}

/// Generic exponential-backoff retry loop.
///
/// Runs `run` until it succeeds, a non-retryable error occurs, or the policy's
/// retry budget is exhausted. Reused by [`execute_tx`] and independently testable.
pub async fn retry_loop<T, E, Fut, R>(
    policy: &RetryPolicy,
    is_retryable: impl Fn(&E) -> bool,
    run: R,
) -> Result<T, E>
where
    R: Fn() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let mut attempt: u32 = 0;
    loop {
        match run().await {
            Ok(value) => return Ok(value),
            Err(e) => {
                if is_retryable(&e) && attempt < policy.max_retries {
                    let delay = policy.backoff_delay(attempt);
                    attempt += 1;
                    tokio::time::sleep(delay).await;
                } else {
                    return Err(e);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    fn fast_policy(max: u32) -> RetryPolicy {
        RetryPolicy {
            max_retries: max,
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(2),
            multiplier: 2.0,
            jitter: false,
        }
    }

    #[tokio::test]
    async fn retries_then_succeeds() {
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();
        let res: Result<i32, &str> = retry_loop(
            &fast_policy(5),
            |e| *e == "retry",
            move || {
                let c = c.clone();
                async move {
                    let n = c.fetch_add(1, Ordering::SeqCst);
                    if n < 2 {
                        Err("retry")
                    } else {
                        Ok(42)
                    }
                }
            },
        )
        .await;
        assert_eq!(res, Ok(42));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn gives_up_after_budget() {
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();
        let res: Result<i32, &str> = retry_loop(
            &fast_policy(2),
            |e| *e == "retry",
            move || {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Err("retry")
                }
            },
        )
        .await;
        assert_eq!(res, Err("retry"));
        // 1 initial + 2 retries.
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn non_retryable_stops_immediately() {
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();
        let res: Result<i32, &str> = retry_loop(
            &fast_policy(5),
            |e| *e == "retry",
            move || {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Err("fatal")
                }
            },
        )
        .await;
        assert_eq!(res, Err("fatal"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn non_sqlx_errors_are_not_retryable() {
        assert!(!is_retryable(&DbErr::Custom("boom".into())));
        assert!(!is_retryable(&DbErr::RecordNotFound("x".into())));
    }
}
