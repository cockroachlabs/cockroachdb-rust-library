//! Plenum: per-table storage summaries.

use cockroachdb_sqlx::Pool;
use sqlx::Row;

/// Storage used by one table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableStorage {
    /// Schema the table belongs to.
    pub schema: String,
    /// Table name.
    pub table: String,
    /// Approximate size in bytes.
    pub bytes: i64,
}

impl TableStorage {
    /// `"schema"."table"` with quoted identifiers.
    pub fn qualified_name(&self) -> String {
        format!(
            "\"{}\".\"{}\"",
            self.schema.replace('"', "\"\""),
            self.table.replace('"', "\"\"")
        )
    }
}

/// The storage-summary query. Exposed so callers can inspect or override it for
/// their CockroachDB version.
///
// ponytail: targets crdb_internal; the range-size columns have shifted across
// CRDB versions — validate against your cluster, override via raw SQL if needed.
pub fn summary_query() -> &'static str {
    "SELECT t.schema_name AS schema, t.name AS table_name, \
            COALESCE(s.bytes, 0)::INT8 AS bytes \
     FROM crdb_internal.tables t \
     LEFT JOIN ( \
         SELECT table_id, sum(range_size)::INT8 AS bytes \
         FROM crdb_internal.ranges_no_leases GROUP BY table_id \
     ) s ON s.table_id = t.table_id \
     WHERE t.database_name = current_database() \
       AND t.schema_name NOT IN ('crdb_internal','information_schema','pg_catalog') \
     ORDER BY bytes DESC"
}

/// Returns a per-table storage summary for the current database.
pub async fn summary(pool: &Pool) -> Result<Vec<TableStorage>, sqlx::Error> {
    let rows = sqlx::query(summary_query())
        .fetch_all(pool.raw_pool())
        .await?;
    rows.iter()
        .map(|r| {
            Ok(TableStorage {
                schema: r.try_get("schema")?,
                table: r.try_get("table_name")?,
                bytes: r.try_get("bytes")?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualified_name_quotes() {
        let ts = TableStorage {
            schema: "public".into(),
            table: "accounts".into(),
            bytes: 1024,
        };
        assert_eq!(ts.qualified_name(), "\"public\".\"accounts\"");
    }

    #[test]
    fn summary_query_targets_current_database() {
        assert!(summary_query().contains("current_database()"));
    }
}
