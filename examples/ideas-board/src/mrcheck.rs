//! Multi-region annotation check (SeaORM path): declare each entity's intended
//! CockroachDB locality with `#[derive(CrdbLocality)]`, then verify that intent
//! against the live schema at startup. This is the Rust twin of the Spring Boot
//! starter's startup multi-region check (0.2.0). It is opt-in: call [`verify`]
//! after connecting.

// tag::mr-check-entity[]
/// The `ideas` table modelled as a SeaORM entity, declared **REGIONAL BY ROW**
/// and homed by its `region` column. `#[derive(CrdbLocality)]` records that
/// intent so [`verify`] can diff it against the live table's locality.
pub mod regional {
    use cockroachdb_macros::CrdbLocality;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, CrdbLocality)]
    #[sea_orm(table_name = "ideas")]
    #[crdb(regional_by_row)]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub title: String,
        #[crdb(region)]
        pub region: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
// end::mr-check-entity[]

// tag::mr-check-usage[]
use cockroachdb_seaorm::{describe, verify_multiregion, Mode};
use sea_orm::DatabaseConnection;

/// Verifies every annotated entity's declared locality against the live schema.
/// In `Mode::Warn` (the default) a plain mismatch — e.g. the `ideas` table not
/// being multi-region on a single-node cluster — is logged and startup
/// continues; a direct contradiction fails startup even in warn mode.
pub async fn verify(db: &DatabaseConnection) -> Result<(), sea_orm::DbErr> {
    let report = verify_multiregion(db, Mode::Warn, vec![describe::<regional::Entity>()]).await?;
    for problem in report.problems() {
        eprintln!("multi-region check: {problem}");
    }
    Ok(())
}
// end::mr-check-usage[]

#[cfg(test)]
mod tests {
    use super::{describe, regional};
    use cockroachdb_seaorm::Kind;

    #[test]
    fn entity_declares_regional_by_row_homed_by_region() {
        let t = describe::<regional::Entity>();
        assert_eq!(t.table, "ideas");
        assert_eq!(t.kind, Kind::RegionalByRow);
        assert_eq!(t.region_column.as_deref(), Some("region"));
    }

    // tag::mr-check-report[]
    // End-to-end check against a live cluster. On the single-node CI cluster the
    // `ideas` table has no regions, so `verify` reports a plain "not multi-region"
    // problem and (in warn mode) returns Ok — nothing to fail on. `#[ignore]`d;
    // the repo runs integration tests with `-- --ignored`.
    use super::verify;
    use cockroachdb_core::Config;
    use cockroachdb_seaorm::connect;

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "requires a live CockroachDB cluster (set CRDB_URL)"]
    async fn verify_runs_against_live_schema() {
        let url = std::env::var("CRDB_URL")
            .unwrap_or_else(|_| "postgres://root@localhost:26257/defaultdb?sslmode=disable".into());
        let db = connect(&url, &Config::default()).await.expect("connect");
        // Warn mode tolerates a non-multi-region table, so this is Ok on a
        // single-node cluster with no regions configured.
        verify(&db).await.expect("warn-mode verify returns Ok");
    }
    // end::mr-check-report[]
}
