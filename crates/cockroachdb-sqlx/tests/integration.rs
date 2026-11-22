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
async fn follower_read_catches_up_to_seeded_row() {
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

    // Follower reads are ~4.8s stale, so a freshly-seeded row is invisible until
    // the staleness window passes. Poll until it appears (mirrors the Go sibling's
    // awaitTopIdeas), tolerating transient "as of a time before the table existed"
    // errors while the window advances.
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let mut caught_up = false;
    while std::time::Instant::now() < deadline {
        match pool
            .fetch_all_follower_read_as(Duration::from_secs(5), "SELECT id FROM rust_fr")
            .await
        {
            Ok(rows) if !rows.is_empty() => {
                caught_up = true;
                break;
            }
            _ => tokio::time::sleep(Duration::from_millis(500)).await,
        }
    }
    assert!(
        caught_up,
        "follower read never caught up to the seeded row within 30s"
    );
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
