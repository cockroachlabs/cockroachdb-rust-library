//! Business logic for the ideas-board demo, built on the CockroachDB library.

use cockroachdb_sqlx::Pool;
use sqlx::Row;
use std::time::Duration;

/// A ranked idea.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Idea {
    /// Idea id (text).
    pub id: String,
    /// Idea title.
    pub title: String,
    /// Vote count.
    pub votes: i64,
}

/// Ideas-board service: all writes retry on serialization conflicts, reads can go
/// to a follower.
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

    /// Inserts an idea, returning its id. Retried on `40001`.
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

    /// Upvotes an idea. Retried on `40001` — the read-modify-write that most
    /// benefits from automatic retry under contention.
    pub async fn upvote(&self, id: &str) -> Result<(), sqlx::Error> {
        let id = id.to_string();
        self.pool
            .execute_tx(move |conn| {
                let id = id.clone();
                Box::pin(async move {
                    sqlx::query("UPDATE ideas SET votes = votes + 1 WHERE id = $1")
                        .bind(&id)
                        .execute(&mut *conn)
                        .await?;
                    Ok(())
                })
            })
            .await
    }

    /// Top ideas by votes, served from a follower read (slightly stale, lower
    /// latency).
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
