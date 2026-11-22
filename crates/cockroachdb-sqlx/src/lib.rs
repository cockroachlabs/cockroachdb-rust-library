//! SQLx adapter for CockroachDB — the primary adapter for this library.
//!
//! Wraps a [`sqlx::PgPool`] with CockroachDB-aware behaviour:
//!
//! * [`Pool::execute_tx`] runs a closure in a transaction and retries it on
//!   serialization conflicts (`SQLSTATE 40001`) with exponential backoff.
//! * [`Pool::follower_read_tx`] / [`Pool::fetch_all_follower_read`] run reads
//!   against a transaction scoped with `AS OF SYSTEM TIME`.
//! * Isolation level and read-only mode are applied per transaction.
//! * The pool implements [`cockroachdb_core::HealthChecker`].
//!
//! Re-runnable closures take `&mut sqlx::PgConnection` and return a boxed future;
//! this works around Rust's lending-closure limits. A typical call:
//!
//! ```no_run
//! # use cockroachdb_sqlx::Pool;
//! # use cockroachdb_core::Config;
//! # async fn demo(pool: Pool) -> Result<(), sqlx::Error> {
//! pool.execute_tx(|conn| Box::pin(async move {
//!     sqlx::query("UPDATE accounts SET balance = balance - 100 WHERE id = $1")
//!         .bind(1_i64)
//!         .execute(&mut *conn)
//!         .await?;
//!     Ok(())
//! })).await
//! # }
//! ```

mod error;
mod follower;
#[macro_use]
mod macros;
mod pool;
mod tx;

pub use error::{is_retryable, sql_state};
pub use pool::Pool;
pub use tx::ExecuteTxOptions;

/// Re-export of the driver-agnostic core for convenience.
pub use cockroachdb_core as core;
