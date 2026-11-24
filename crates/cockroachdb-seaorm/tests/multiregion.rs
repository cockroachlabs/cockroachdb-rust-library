//! Multi-region check tests.
//!
//! The `describe` test exercises the `#[derive(CrdbLocality)]` macro + metamodel
//! read offline (no database), under a plain `cargo test`. The `#[ignore]`d tests
//! need a live CockroachDB cluster:
//!
//! ```bash
//! CRDB_URL=postgres://... cargo test -p cockroachdb-seaorm -- --ignored
//! ```

use cockroachdb_seaorm::{describe, describe_plain, verify_multiregion, Kind, Mode};

// A GLOBAL entity on table `ref_data`.
mod ref_global {
    use cockroachdb_macros::CrdbLocality;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, CrdbLocality)]
    #[sea_orm(table_name = "ref_data")]
    #[crdb(global)]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

// A REGIONAL BY ROW entity on table `ideas`, homed by the `region` column.
mod ideas_rbr {
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

// A REGIONAL BY ROW entity on table `ref_data` (no region column): used to force a
// GLOBAL-table-vs-RBR-entity contradiction.
mod ref_rbr {
    use cockroachdb_macros::CrdbLocality;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, CrdbLocality)]
    #[sea_orm(table_name = "ref_data")]
    #[crdb(regional_by_row)]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

#[test]
fn describe_reads_declared_locality_from_the_derive() {
    let g = describe::<ref_global::Entity>();
    assert_eq!(g.table, "ref_data");
    assert_eq!(g.kind, Kind::Global);
    assert!(g.region_column.is_none());

    let r = describe::<ideas_rbr::Entity>();
    assert_eq!(r.table, "ideas");
    assert_eq!(r.kind, Kind::RegionalByRow);
    assert_eq!(r.region_column.as_deref(), Some("region"));

    let plain = describe_plain::<ideas_rbr::Entity>();
    assert_eq!(plain.kind, Kind::None);
    assert_eq!(plain.table, "ideas");
}

fn dsn() -> String {
    std::env::var("CRDB_URL")
        .unwrap_or_else(|_| "postgres://root@localhost:26257/defaultdb?sslmode=disable".into())
}

fn is_multiregion_unavailable(e: &sea_orm::DbErr) -> bool {
    // A single-node cluster started without --locality has no regions, so declaring
    // one fails (42xxx / 42602); the message mentions "region". Mirrors the SQLx
    // example's skip.
    e.to_string().to_lowercase().contains("region")
}

#[tokio::test]
#[ignore = "requires a live CockroachDB cluster (set CRDB_URL)"]
async fn warn_reports_non_multiregion_database() {
    use sea_orm::ConnectionTrait;
    let db = sea_orm::Database::connect(dsn()).await.expect("connect");
    db.execute_unprepared("CREATE TABLE IF NOT EXISTS ref_data (id INT PRIMARY KEY, name STRING)")
        .await
        .expect("create table");

    // On a non-multi-region database the GLOBAL marker has no effect: a plain
    // problem, not a contradiction, so warn returns Ok with a non-empty report.
    let report = verify_multiregion(&db, Mode::Warn, vec![describe::<ref_global::Entity>()])
        .await
        .expect("warn mode returns Ok on a plain (non-contradiction) problem");
    assert!(!report.is_empty());
    assert!(!report.has_contradictions());
    assert!(report
        .problems()
        .iter()
        .any(|p| p.contains("not multi-region")));
}

#[tokio::test]
#[ignore = "requires a MULTI-REGION CockroachDB cluster (set CRDB_URL)"]
async fn contradiction_fails_even_in_warn() {
    use sea_orm::ConnectionTrait;
    let db = sea_orm::Database::connect(dsn()).await.expect("connect");
    db.execute_unprepared("CREATE TABLE IF NOT EXISTS ref_data (id INT PRIMARY KEY, name STRING)")
        .await
        .expect("create table");

    let dbname: String = db
        .query_one(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT current_database()",
        ))
        .await
        .expect("db name")
        .expect("row")
        .try_get_by::<String, _>(0usize)
        .expect("name");

    // Make the database multi-region and the table GLOBAL. On a single-node cluster
    // with no regions this fails — skip, there is nothing to prove.
    let setup = format!("ALTER DATABASE \"{dbname}\" SET PRIMARY REGION 'us-east1'");
    if let Err(e) = db.execute_unprepared(&setup).await {
        if is_multiregion_unavailable(&e) {
            eprintln!("skipping: cluster has no regions configured (single-node)");
            return;
        }
        panic!("set primary region: {e}");
    }
    db.execute_unprepared("ALTER TABLE ref_data SET LOCALITY GLOBAL")
        .await
        .expect("set global");

    // The table is GLOBAL but the entity declares REGIONAL BY ROW: a contradiction,
    // which must fail even in warn mode.
    let result = verify_multiregion(&db, Mode::Warn, vec![describe::<ref_rbr::Entity>()]).await;
    assert!(
        result.is_err(),
        "a contradiction must fail startup even in warn mode"
    );
}
