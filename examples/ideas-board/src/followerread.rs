//! Follower-reads track: serve the leaderboard from the nearest replica by
//! scoping the read to a slightly-stale timestamp.

// tag::follower-read[]
use crate::idea::Idea;
use cockroachdb_sqlx::Pool;
use sqlx::Row;
use std::time::Duration;

/// Returns the top ideas by votes, served from a **follower read**.
/// `follower_read_tx` scopes the query with `AS OF SYSTEM TIME`, so CockroachDB
/// can answer it from the nearest replica instead of the leaseholder — lower
/// latency, at the cost of reading data up to `staleness` old. Pass
/// `Duration::ZERO` to let CockroachDB pick the freshest safe timestamp.
pub async fn top_ideas_follower(
    pool: &Pool,
    staleness: Duration,
    limit: i64,
) -> Result<Vec<Idea>, sqlx::Error> {
    pool.follower_read_tx(staleness, move |conn| {
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
// end::follower-read[]

#[cfg(test)]
mod tests {
    // tag::follower-read-it[]
    // Follower reads are ~4.8s stale, so freshly-seeded rows are invisible until
    // the staleness window passes. Seed, then poll until the snapshot catches up
    // (tolerating the transient "as of a time before the table existed" error).
    // `#[ignore]`d — needs a live cluster.
    use super::top_ideas_follower;
    use crate::service::IdeaService;
    use cockroachdb_core::Config;
    use cockroachdb_sqlx::Pool;
    use std::time::{Duration, Instant};

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "requires a live CockroachDB cluster (set CRDB_URL)"]
    async fn follower_read_catches_up_to_seeded_ideas() {
        let url = std::env::var("CRDB_URL")
            .unwrap_or_else(|_| "postgres://root@localhost:26257/defaultdb?sslmode=disable".into());
        let pool = Pool::connect(&url, Config::default())
            .await
            .expect("connect");
        let svc = IdeaService::new(pool.clone());
        svc.ensure_schema().await.expect("schema");
        for title in ["Dark mode", "CSV export", "Full-text search"] {
            svc.create_idea(title).await.expect("seed");
        }

        let deadline = Instant::now() + Duration::from_secs(30);
        let mut seen = 0;
        while Instant::now() < deadline {
            if let Ok(top) = top_ideas_follower(&pool, Duration::from_secs(5), 10).await {
                seen = top.len();
                if seen >= 3 {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        assert!(
            seen >= 3,
            "follower read should catch up to the 3 seeded ideas within 30s"
        );
        pool.close().await;
    }
    // end::follower-read-it[]
}
