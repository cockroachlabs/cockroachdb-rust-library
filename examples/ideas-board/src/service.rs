//! Ideas-board service: the fundamentals story — a read-modify-write upvote that
//! loses votes under contention, and the one-method fix that absorbs the
//! resulting `40001` serialization conflicts.

use crate::idea::Idea;
use cockroachdb_sqlx::Pool;
use sqlx::Row;
use std::time::Duration;

/// Ideas-board service over a CockroachDB pool.
#[derive(Clone)]
pub struct IdeaService {
    pool: Pool,
}

impl IdeaService {
    /// Creates the service over a pool.
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// Creates the `ideas` table if absent (autocommit DDL).
    pub async fn ensure_schema(&self) -> Result<(), sqlx::Error> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS ideas (\
             id STRING PRIMARY KEY DEFAULT gen_random_uuid()::STRING, \
             title STRING NOT NULL, \
             votes INT8 NOT NULL DEFAULT 0)",
        )
        .execute(self.pool.raw_pool())
        .await?;
        Ok(())
    }

    /// Inserts an idea, returning its id.
    pub async fn create_idea(&self, title: &str) -> Result<String, sqlx::Error> {
        let title = title.to_string();
        self.pool
            .execute_tx(move |conn| {
                let title = title.clone();
                Box::pin(async move {
                    let row =
                        sqlx::query("INSERT INTO ideas (title) VALUES ($1) RETURNING id::text")
                            .bind(&title)
                            .fetch_one(&mut *conn)
                            .await?;
                    row.try_get::<String, _>(0)
                })
            })
            .await
    }

    // Shown in the tutorial's "the conflict" guide to demonstrate lost updates;
    // the runnable demo only calls the retrying `upvote`.
    #[allow(dead_code)]
    // tag::plain[]
    /// Upvote **without** retry: read the current count, then write `count + 1`
    /// as two separate autocommit statements. Under concurrency the reads
    /// overlap, every writer computes the same `count + 1`, and later writes
    /// clobber earlier ones — votes are silently lost.
    pub async fn upvote_without_retry(&self, id: &str) -> Result<(), sqlx::Error> {
        let votes: i64 = sqlx::query("SELECT votes FROM ideas WHERE id = $1")
            .bind(id)
            .fetch_one(self.pool.raw_pool())
            .await?
            .get("votes");
        sqlx::query("UPDATE ideas SET votes = $1 WHERE id = $2")
            .bind(votes + 1)
            .bind(id)
            .execute(self.pool.raw_pool())
            .await?;
        Ok(())
    }
    // end::plain[]

    // tag::retry[]
    /// Upvote **with** retry: the same read-modify-write, now inside one
    /// `execute_tx` transaction. Under `SERIALIZABLE` isolation a conflicting
    /// write makes the commit fail with `40001`; `execute_tx` re-runs the whole
    /// closure with exponential backoff until it commits — so every vote lands.
    pub async fn upvote(&self, id: &str) -> Result<(), sqlx::Error> {
        let id = id.to_string();
        self.pool
            .execute_tx(move |conn| {
                let id = id.clone();
                Box::pin(async move {
                    let votes: i64 = sqlx::query("SELECT votes FROM ideas WHERE id = $1")
                        .bind(&id)
                        .fetch_one(&mut *conn)
                        .await?
                        .get("votes");
                    sqlx::query("UPDATE ideas SET votes = $1 WHERE id = $2")
                        .bind(votes + 1)
                        .bind(&id)
                        .execute(&mut *conn)
                        .await?;
                    Ok(())
                })
            })
            .await
    }
    // end::retry[]

    /// Top ideas by votes, served from a follower read (slightly stale, lower
    /// latency). Used by the follower-reads guide.
    pub async fn top_ideas(&self, limit: i64) -> Result<Vec<Idea>, sqlx::Error> {
        self.pool
            .follower_read_tx(Duration::from_secs(5), move |conn| {
                Box::pin(async move {
                    let rows = sqlx::query(
                        "SELECT id::text AS id, title, votes FROM ideas ORDER BY votes DESC LIMIT $1",
                    )
                    .bind(limit)
                    .fetch_all(&mut *conn)
                    .await?;
                    Ok(rows
                        .iter()
                        .map(|r| Idea {
                            id: r.get("id"),
                            title: r.get("title"),
                            votes: r.get("votes"),
                        })
                        .collect())
                })
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    // tag::unit-test[]
    // A fast, no-database check of the retry *mechanism*: a simulated `40001`
    // that clears after two attempts is driven to success under concurrency.
    // A fake can't produce a real serialization conflict, so the end-to-end
    // contention proof lives in the integration test below.
    use cockroachdb_testing::{ConcurrencyRunner, RetrySimulator};
    use std::sync::Arc;

    #[tokio::test]
    async fn retry_loop_absorbs_simulated_conflicts() {
        let sim = Arc::new(RetrySimulator::serialization_failures(2));
        let report = ConcurrencyRunner::new(8, 5)
            .run(move |_i| {
                let sim = sim.clone();
                async move {
                    for _ in 0..10 {
                        match sim.run().await {
                            Ok(()) => return Ok::<(), ()>(()),
                            Err(e) if cockroachdb_core::is_retryable(&e) => continue,
                            Err(_) => return Err(()),
                        }
                    }
                    Err(())
                }
            })
            .await;
        assert_eq!(report.total, 40);
        assert_eq!(report.failures, 0);
    }
    // end::unit-test[]

    // tag::concurrency-test[]
    // End-to-end contention proof against a live cluster (set `CRDB_URL`). Fires
    // 3 × 20 concurrent upvotes through the retrying `upvote`; every vote must
    // land (final count 60). `#[ignore]`d so it only runs with a cluster.
    use super::IdeaService;
    use cockroachdb_core::Config;
    use cockroachdb_sqlx::Pool;
    use sqlx::Row as _;

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires a live CockroachDB cluster (set CRDB_URL)"]
    async fn retry_lands_every_vote_under_contention() {
        let url = std::env::var("CRDB_URL")
            .unwrap_or_else(|_| "postgres://root@localhost:26257/defaultdb?sslmode=disable".into());
        let pool = Pool::connect(&url, Config::default())
            .await
            .expect("connect");
        let svc = IdeaService::new(pool.clone());
        svc.ensure_schema().await.expect("schema");
        let id = svc.create_idea("Add dark mode").await.expect("seed");

        let svc2 = svc.clone();
        let id2 = id.clone();
        let report = ConcurrencyRunner::new(3, 20)
            .run(move |_i| {
                let svc = svc2.clone();
                let id = id2.clone();
                async move { svc.upvote(&id).await.map_err(|_| ()) }
            })
            .await;
        assert_eq!(report.failures, 0);

        let votes: i64 = sqlx::query("SELECT votes FROM ideas WHERE id = $1")
            .bind(&id)
            .fetch_one(pool.raw_pool())
            .await
            .expect("read votes")
            .get("votes");
        assert_eq!(
            votes, 60,
            "retrying upvote must land every one of the 60 votes"
        );
        pool.close().await;
    }
    // end::concurrency-test[]
}
