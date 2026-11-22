//! Ergonomic macros that remove the `Box::pin(async move { … })` boilerplate
//! around [`Pool::execute_tx`](crate::Pool::execute_tx) and the follower-read
//! helpers.
//!
//! These are the lightweight, dependency-free half of the "AOP in Rust" answer.
//! A proc-macro attribute (`#[cockroachdb::transactional]`) lives in the separate
//! `cockroachdb-macros` crate for teams that want the Spring-annotation feel; this
//! `macro_rules!` form covers the common case with no extra dependency.

/// Runs a block in a retrying transaction, hiding the boxed-future plumbing.
///
/// ```no_run
/// # use cockroachdb_sqlx::{Pool, transactional};
/// # async fn demo(pool: Pool) -> Result<(), sqlx::Error> {
/// let n: i64 = transactional!(pool, |conn| {
///     sqlx::query("UPSERT INTO accounts (id, balance) VALUES ($1, $2)")
///         .bind(1_i64).bind(100_i64)
///         .execute(&mut *conn)
///         .await?;
///     Ok(1)
/// }).await?;
/// # let _ = n; Ok(())
/// # }
/// ```
#[macro_export]
macro_rules! transactional {
    ($pool:expr, |$conn:ident| $body:block) => {
        $pool.execute_tx(move |$conn| ::std::boxed::Box::pin(async move { $body }))
    };
}

/// Runs a block as a follower read with a given staleness, hiding the plumbing.
///
/// ```no_run
/// # use cockroachdb_sqlx::{Pool, follower_read};
/// # use std::time::Duration;
/// # async fn demo(pool: Pool) -> Result<i64, sqlx::Error> {
/// follower_read!(pool, Duration::from_secs(5), |conn| {
///     let (bal,): (i64,) = sqlx::query_as("SELECT balance FROM accounts WHERE id = $1")
///         .bind(1_i64)
///         .fetch_one(&mut *conn)
///         .await?;
///     Ok(bal)
/// }).await
/// # }
/// ```
#[macro_export]
macro_rules! follower_read {
    ($pool:expr, $staleness:expr, |$conn:ident| $body:block) => {
        $pool.follower_read_tx($staleness, move |$conn| {
            ::std::boxed::Box::pin(async move { $body })
        })
    };
}
