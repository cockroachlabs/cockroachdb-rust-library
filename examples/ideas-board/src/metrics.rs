//! Metrics track: attach an in-memory recorder to the pool and read the counters
//! that `execute_tx` bumps on every transaction and retry.

// tag::metrics-config[]
use cockroachdb_core::{Config, InMemoryRecorder, Snapshotter};
use cockroachdb_sqlx::Pool;
use std::sync::Arc;

/// Builds a pool with an in-memory metrics recorder attached. Once attached,
/// every `execute_tx` the pool runs records a transaction, and each `40001`
/// retry it absorbs records a retry — automatically, with no extra code in the
/// upvote path. The returned recorder's `snapshot()` exposes the running totals.
pub async fn metered_pool(
    url: &str,
    config: Config,
) -> Result<(Pool, Arc<InMemoryRecorder>), sqlx::Error> {
    let recorder = Arc::new(InMemoryRecorder::new());
    let pool = Pool::connect(url, config)
        .await?
        .with_metrics(recorder.clone());
    Ok((pool, recorder))
}

/// Prints the recorder's running totals.
pub fn print_metrics(recorder: &InMemoryRecorder) {
    let s = recorder.snapshot();
    println!(
        "metrics: {} transactions ({} ok, {} failed), {} retries, {} follower reads",
        s.transaction_count,
        s.transaction_success_count,
        s.transaction_failure_count,
        s.retry_count,
        s.follower_read_count,
    );
}
// end::metrics-config[]

#[cfg(test)]
mod tests {
    // tag::metrics-it[]
    // Integration test: fire contended upvotes through a metered pool, then assert
    // the recorder observed every transaction and at least one retry (retries are
    // only produced by genuine 40001 contention). `#[ignore]`d — needs a cluster.
    use super::metered_pool;
    use crate::service::IdeaService;
    use cockroachdb_core::{Config, Snapshotter};
    use cockroachdb_testing::ConcurrencyRunner;

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires a live CockroachDB cluster (set CRDB_URL)"]
    async fn recorder_counts_transactions_and_retries() {
        let url = std::env::var("CRDB_URL")
            .unwrap_or_else(|_| "postgres://root@localhost:26257/defaultdb?sslmode=disable".into());
        let (pool, recorder) = metered_pool(&url, Config::default())
            .await
            .expect("connect");
        let svc = IdeaService::new(pool.clone());
        svc.ensure_schema().await.expect("schema");
        let id = svc.create_idea("Metered idea").await.expect("seed");

        let svc2 = svc.clone();
        let id2 = id.clone();
        ConcurrencyRunner::new(3, 20)
            .run(move |_i| {
                let svc = svc2.clone();
                let id = id2.clone();
                async move { svc.upvote(&id).await.map_err(|_| ()) }
            })
            .await;

        let snap = recorder.snapshot();
        assert!(
            snap.transaction_count >= 60,
            "every upvote is a managed transaction"
        );
        assert!(snap.transaction_success_count >= 60);
        assert!(
            snap.retry_count > 0,
            "contended upvotes should produce retries"
        );
        pool.close().await;
    }
    // end::metrics-it[]
}
