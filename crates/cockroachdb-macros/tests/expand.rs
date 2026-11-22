//! The macros are validated by *compiling* their expansion: each annotated
//! function below must type-check against the real `cockroachdb-sqlx` API. They
//! are never called (no database needed), so this runs under a plain `cargo test`.

use cockroachdb_macros::{follower_read, transactional};
use cockroachdb_sqlx::Pool;
use std::time::Duration;

#[transactional]
async fn _transfer(pool: &Pool, from: i64, to: i64, cents: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE accounts SET balance = balance - $1 WHERE id = $2")
        .bind(cents)
        .bind(from)
        .execute(&mut *conn)
        .await?;
    sqlx::query("UPDATE accounts SET balance = balance + $1 WHERE id = $2")
        .bind(cents)
        .bind(to)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

#[transactional(pool = db)]
async fn _named_pool(db: &Pool, id: i64) -> Result<i64, sqlx::Error> {
    let (n,): (i64,) = sqlx::query_as("SELECT n FROM t WHERE id = $1")
        .bind(id)
        .fetch_one(&mut *conn)
        .await?;
    Ok(n)
}

#[follower_read(staleness = Duration::from_secs(5))]
async fn _recent_balance(pool: &Pool, id: i64) -> Result<i64, sqlx::Error> {
    let (bal,): (i64,) = sqlx::query_as("SELECT balance FROM accounts WHERE id = $1")
        .bind(id)
        .fetch_one(&mut *conn)
        .await?;
    Ok(bal)
}

#[follower_read]
async fn _any_balance(pool: &Pool, id: i64) -> Result<i64, sqlx::Error> {
    let (bal,): (i64,) = sqlx::query_as("SELECT balance FROM accounts WHERE id = $1")
        .bind(id)
        .fetch_one(&mut *conn)
        .await?;
    Ok(bal)
}

#[test]
fn macros_expand_and_typecheck() {
    // Referencing the items forces monomorphization/type-checking of the
    // generated bodies. The compile is the assertion.
    let _ = _transfer;
    let _ = _named_pool;
    let _ = _recent_balance;
    let _ = _any_balance;
}
