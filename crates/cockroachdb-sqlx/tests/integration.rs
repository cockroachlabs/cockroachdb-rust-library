//! Integration tests against a live CockroachDB cluster.
//!
//! These are `#[ignore]`d so `cargo test` passes with no database. Run them with
//! a cluster available:
//!
//! ```bash
//! CRDB_URL=postg+... cargo test -p cockroachdb-sqlx -- --ignored
//! ```

use cockroachdb_core::Config;
use cockroachdb_sqlx::Pool;
use std::sync::Arc;
use std::time::Duration;

fn dsn() -> String {
    std::env::var("CRDB_URL")
        .unwrap_or_else(|_| "postgres://root@localhost:26257/defaultdb?sslmode=disable".into())
}

#[tokio::test]
#[ignore = "requires a live CockroachDB cluster (set CRDB_URL)"]
async fn execute_tx_commits_and_metrics_record() {
    let recorder = Arc::new(cockroachdb_core::InMemoryRecorder::new());
    let pool = Pool::connect(&dsn(), Config::default())
        .await
        .expect("connect")
        .with_metrics(recorder.clone());

    pool.execute_tx(|conn| {
        Box::pin(async move {
            sqlx::query("CREATE TABLE IF NOT EXISTS rust_it (id INT PRIMARY KEY, n INT)")
                .execute(&mut *conn)
                .await?;
            sqlx::query("UPSERT INTO rust_it (id, n) VALUES (1, 100)")
                .execute(&mut *conn)
                .await?;
            Ok(())
        })
    })
    .await
    .expect("tx");

    use cockroachdb_core::Snapshotter;
    let snap = recorder.snapshot();
    assert_eq!(snap.transaction_count, 1);
    assert_eq!(snap.transaction_success_count, 1);

    pool.close().await;
}

#[tokio::test]
#[ignore = "requires a live CockroachDB cluster (set CRDB_URL)"]
async fn follower_read_returns_rows() {
    let pool = Pool::connect(&dsn(), Config::default())
        .await
        .expect("connect");
    pool.execute_tx(|conn| {
        Box::pin(async move {
            sqlx::query("CREATE TABLE IF NOT EXISTS rust_fr (id INT PRIMARY KEY)")
                .execute(&mut *conn)
                .await?;
            sqlx::query("UPSERT INTO rust_fr (id) VALUES (1)")
                .execute(&mut *conn)
                .await?;
            Ok(())
        })
    })
    .await
    .expect("seed");

    // A fixed staleness keeps the read in the safe follower window.
    let rows = pool
        .fetch_all_follower_read_as(Duration::from_secs(5), "SELECT id FROM rust_fr")
        .await
        .expect("follower read");
    assert!(!rows.is_empty());
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires a live CockroachDB cluster (set CRDB_URL)"]
async fn health_check_passes() {
    use cockroachdb_core::HealthChecker;
    let pool = Pool::connect(&dsn(), Config::default())
        .await
        .expect("connect");
    pool.check().await.expect("healthy");
    pool.close().await;
}
