//! Multi-region track: home each idea to a region via the hidden `crdb_region`
//! **column**. CockroachDB homes a REGIONAL BY ROW row by the value written to
//! that column on insert — there is no `SET crdb_region` session statement (a
//! mistake that bit several sibling libraries). This module only ever writes the
//! column value.

// tag::region-config[]
use cockroachdb_core::ddl;

/// DDL that turns the board into a multi-region, REGIONAL BY ROW deployment:
/// declare the database's regions, then set the table's locality so every row
/// carries a `crdb_region` homing column. Run these once per cluster (adding a
/// non-primary region needs nodes in that locality).
pub fn regional_setup_sql(database: &str) -> Vec<String> {
    vec![
        ddl::set_primary_region(database, "us-east1"),
        ddl::add_region(database, "us-west1"),
        ddl::set_locality_regional_by_row("ideas"),
    ]
}
// end::region-config[]

// tag::inregion-usage[]
use cockroachdb_core::region::{RegionContext, CRDB_REGION_COLUMN};
use cockroachdb_sqlx::Pool;
use sqlx::Row;

/// Writes ideas homed to a caller-chosen region. The region travels as the row's
/// `crdb_region` **column value** on insert, so CockroachDB stores and serves
/// that row from the matching region. Homing is per-row data — there is no
/// session-level region switch.
#[derive(Clone)]
pub struct RegionalIdeaService {
    pool: Pool,
}

impl RegionalIdeaService {
    /// Creates the service over a pool.
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// Inserts an idea homed to `region`, returning its id. The `(column, value)`
    /// pair comes from `RegionContext::homing_column()` — the column name is the
    /// fixed `crdb_region`, never caller input, so interpolating it is safe.
    pub async fn add_in_region(&self, region: &str, title: &str) -> Result<String, sqlx::Error> {
        let ctx = RegionContext::new(region);
        let (col, value) = ctx.homing_column().expect("region is set");
        let sql = format!("INSERT INTO ideas (title, {col}) VALUES ($1, $2) RETURNING id::text");
        let (title, value) = (title.to_string(), value.to_string());
        self.pool
            .execute_tx(move |conn| {
                let (sql, title, value) = (sql.clone(), title.clone(), value.clone());
                Box::pin(async move {
                    let row = sqlx::query(&sql)
                        .bind(&title)
                        .bind(&value)
                        .fetch_one(&mut *conn)
                        .await?;
                    row.try_get::<String, _>(0)
                })
            })
            .await
    }

    /// Reads back the region a row is homed to.
    pub async fn region_of(&self, id: &str) -> Result<String, sqlx::Error> {
        let sql = format!("SELECT {CRDB_REGION_COLUMN}::text FROM ideas WHERE id = $1");
        sqlx::query(&sql)
            .bind(id)
            .fetch_one(self.pool.raw_pool())
            .await?
            .try_get::<String, _>(0)
    }
}
// end::inregion-usage[]

#[cfg(test)]
mod tests {
    // Offline: the setup DDL and the homing pair are pure string construction,
    // so assert them without a cluster. The end-to-end homing proof is the
    // `#[ignore]`d integration test below (needs a multi-region cluster).
    use super::regional_setup_sql;
    use cockroachdb_core::region::RegionContext;

    #[test]
    fn setup_sql_declares_regions_and_row_locality() {
        let sql = regional_setup_sql("defaultdb").join("\n");
        assert!(sql.contains("PRIMARY REGION"));
        assert!(sql.contains("us-east1"));
        assert!(sql.contains("ADD REGION"));
        assert!(sql.contains("REGIONAL BY ROW"));
    }

    #[test]
    fn homing_writes_the_crdb_region_column() {
        let ctx = RegionContext::new("us-west1");
        assert_eq!(ctx.homing_column(), Some(("crdb_region", "us-west1")));
    }

    // tag::region-test[]
    // End-to-end homing proof. Needs a REGIONAL BY ROW cluster whose regions
    // include us-east1/us-west1 (set CRDB_URL); `#[ignore]`d otherwise. Writes an
    // idea in each region and reads the `crdb_region` column back to confirm the
    // row landed where it was homed.
    use super::RegionalIdeaService;
    use crate::service::IdeaService;
    use cockroachdb_core::Config;
    use cockroachdb_sqlx::Pool;

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "requires a multi-region CockroachDB cluster (set CRDB_URL)"]
    async fn writes_home_rows_in_bound_region() {
        let url = std::env::var("CRDB_URL")
            .unwrap_or_else(|_| "postgres://root@localhost:26257/defaultdb?sslmode=disable".into());
        let pool = Pool::connect(&url, Config::default())
            .await
            .expect("connect");
        IdeaService::new(pool.clone())
            .ensure_schema()
            .await
            .expect("schema");
        for stmt in regional_setup_sql("defaultdb") {
            sqlx::query(&stmt)
                .execute(pool.raw_pool())
                .await
                .expect("regional setup");
        }

        let svc = RegionalIdeaService::new(pool.clone());
        let east = svc
            .add_in_region("us-east1", "East idea")
            .await
            .expect("east");
        let west = svc
            .add_in_region("us-west1", "West idea")
            .await
            .expect("west");

        assert_eq!(svc.region_of(&east).await.expect("region"), "us-east1");
        assert_eq!(svc.region_of(&west).await.expect("region"), "us-west1");
        pool.close().await;
    }
    // end::region-test[]
}
