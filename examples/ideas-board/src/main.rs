//! Ideas-board — the reference app for the CockroachDB Rust library.
//!
//! Runs offline demos always (changefeed SQL, vCluster tagging, a retry simulator
//! under concurrency). Set `CRDB_URL` to also run the live-database demo, which
//! fires concurrent upvotes and shows every one landing via `execute_tx`.
//!
//!   cargo run -p ideas-board
//!   CRDB_URL=postgres://root@localhost:26257/defaultdb?sslmode=disable cargo run -p ideas-board

mod idea;
mod service;

use cockroachdb_cdc::ChangefeedSpec;
use cockroachdb_continuum::vcluster::{Priority, VClusterContext};
use cockroachdb_core::{Config, InMemoryRecorder, Snapshotter};
use cockroachdb_sqlx::{core, Pool};
use cockroachdb_testing::{ConcurrencyRunner, RetrySimulator};
use service::IdeaService;
use std::sync::Arc;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("== CockroachDB Rust library — ideas-board demo ==\n");
    offline_demos().await;
    match std::env::var("CRDB_URL") {
        Ok(url) => live_demo(&url).await?,
        Err(_) => println!("\nCRDB_URL not set — skipped live database demo."),
    }
    Ok(())
}

/// Demos that need no database.
async fn offline_demos() {
    let spec = ChangefeedSpec::builder()
        .table("ideas")
        .webhook_target("https://example.com/cdc")
        .with_updated()
        .resolved("10s")
        .build()
        .expect("valid changefeed");
    println!("changefeed SQL:\n  {}\n", spec.to_sql());

    let ctx = VClusterContext::new("ideas-board").with_priority(Priority::Background);
    println!("vCluster tagging:");
    for stmt in ctx.apply_statements() {
        println!("  {stmt}");
    }

    let sim = Arc::new(RetrySimulator::serialization_failures(2));
    let report = ConcurrencyRunner::new(8, 5)
        .run(move |_i| {
            let sim = sim.clone();
            async move {
                for _ in 0..10 {
                    match sim.run().await {
                        Ok(()) => return Ok::<(), ()>(()),
                        Err(e) if core::is_retryable(&e) => continue,
                        Err(_) => return Err(()),
                    }
                }
                Err(())
            }
        })
        .await;
    println!(
        "\nconcurrency run: {} ops, {} ok, {} failed",
        report.total, report.successes, report.failures
    );
}

// tag::runner[]
/// Fires `workers × each` concurrent upvotes at one idea through the retrying
/// `IdeaService::upvote`, and returns how many succeeded. Because the upvote
/// runs inside `execute_tx`, every serialization conflict is retried, so the
/// final vote count equals the number of upvotes fired.
async fn run_concurrent_upvotes(svc: &IdeaService, id: &str, workers: usize, each: usize) -> usize {
    let svc = svc.clone();
    let id = id.to_string();
    let report = ConcurrencyRunner::new(workers, each)
        .run(move |_i| {
            let svc = svc.clone();
            let id = id.clone();
            async move { svc.upvote(&id).await.map_err(|_| ()) }
        })
        .await;
    report.successes
}
// end::runner[]

/// Demos that need a live CockroachDB cluster.
async fn live_demo(url: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use cockroachdb_core::HealthChecker;

    println!("\n-- live database demo --");
    let recorder = Arc::new(InMemoryRecorder::new());
    let pool = Pool::connect(url, Config::builder().region("us-east1").build())
        .await?
        .with_metrics(recorder.clone());

    let svc = IdeaService::new(pool.clone());
    svc.ensure_schema().await?;
    let id = svc.create_idea("Add a dark mode").await?;

    let landed = run_concurrent_upvotes(&svc, &id, 3, 20).await;
    println!("fired 60 concurrent upvotes, {landed} landed");

    let top = svc.top_ideas(5).await?;
    println!("top ideas (follower read):");
    for idea in &top {
        println!("  {:>3} votes — {}", idea.votes, idea.title);
    }

    pool.check().await?;
    let snap = recorder.snapshot();
    println!(
        "metrics: {} transactions ({} ok), {} retries, {} follower reads",
        snap.transaction_count,
        snap.transaction_success_count,
        snap.retry_count,
        snap.follower_read_count
    );
    pool.close().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use cockroachdb_cdc::ChangefeedSpec;

    #[test]
    fn changefeed_sql_is_well_formed() {
        let sql = ChangefeedSpec::builder()
            .table("ideas")
            .webhook_target("https://example.com/cdc")
            .with_updated()
            .build()
            .unwrap()
            .to_sql();
        assert!(sql.starts_with("CREATE CHANGEFEED FOR TABLE \"ideas\""));
        assert!(sql.contains("WITH updated"));
    }
}
