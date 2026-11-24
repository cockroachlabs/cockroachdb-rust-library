//! Multi-region schema check for SeaORM entities — the driver half.
//!
//! [`describe`] turns a SeaORM entity marked with `#[derive(CrdbLocality)]` into a
//! [`MultiregionTable`] descriptor; [`verify_multiregion`] loads the live schema and
//! runs the pure [`cockroachdb_core::multiregion::evaluate`] diff. This mirrors the
//! Spring Boot starter's startup check, on the SeaORM path.
//!
//! It is **opt-in**: call [`verify_multiregion`] after [`crate::connect`], passing the
//! entities to check. In [`Mode::Warn`] (the default) it logs problems and continues;
//! in [`Mode::Validate`], or on a contradiction in any mode but [`Mode::None`], it
//! returns `Err` so the caller can fail startup.
//!
//! ```no_run
//! # use sea_orm::DatabaseConnection;
//! # use cockroachdb_seaorm::{describe, verify_multiregion, Mode};
//! # mod ideas { use sea_orm::entity::prelude::*;
//! #   #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
//! #   #[sea_orm(table_name = "ideas")] pub struct Model { #[sea_orm(primary_key)] pub id: i64 }
//! #   #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)] pub enum Relation {}
//! #   impl ActiveModelBehavior for ActiveModel {}
//! #   impl ::cockroachdb_core::multiregion::CrdbLocality for Entity {
//! #     fn crdb_locality() -> ::cockroachdb_core::multiregion::DeclaredLocality {
//! #       ::cockroachdb_core::multiregion::DeclaredLocality::Global } } }
//! # async fn demo(db: DatabaseConnection) -> Result<(), sea_orm::DbErr> {
//! let report = verify_multiregion(&db, Mode::Warn, vec![describe::<ideas::Entity>()]).await?;
//! assert!(report.is_empty() || !report.has_contradictions());
//! # Ok(()) }
//! ```

use cockroachdb_core::multiregion::{
    evaluate, fails_startup, MultiregionState, TableInfo, TableName,
};
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, DbErr, EntityTrait, Statement, Value,
};
use std::collections::{HashMap, HashSet};

pub use cockroachdb_core::multiregion::{
    CrdbLocality, DeclaredLocality, Kind, Mode, MultiregionReport, MultiregionTable,
};

const PG: DatabaseBackend = DatabaseBackend::Postgres;

/// Describes a marked entity for the check: its table, schema, declared locality, and
/// region column. `E` must be a SeaORM entity with `#[derive(CrdbLocality)]`.
pub fn describe<E>() -> MultiregionTable
where
    E: EntityTrait + CrdbLocality,
{
    let entity = E::default();
    let (kind, region_column) = match E::crdb_locality() {
        DeclaredLocality::Global => (Kind::Global, None),
        DeclaredLocality::RegionalByRow { region_column } => (Kind::RegionalByRow, region_column),
    };
    MultiregionTable {
        entity: std::any::type_name::<E>().to_string(),
        schema: entity.schema_name().map(str::to_string),
        table: entity.table_name().to_string(),
        kind,
        region_column,
    }
}

/// Describes an unmarked entity, so the check can also report a `GLOBAL` or
/// `REGIONAL BY ROW` table whose entity is missing its marker (the reverse
/// direction). `E` only needs to be a SeaORM entity.
pub fn describe_plain<E>() -> MultiregionTable
where
    E: EntityTrait,
{
    let entity = E::default();
    MultiregionTable {
        entity: std::any::type_name::<E>().to_string(),
        schema: entity.schema_name().map(str::to_string),
        table: entity.table_name().to_string(),
        kind: Kind::None,
        region_column: None,
    }
}

/// Checks `tables` against the live multi-region schema of `db`.
///
/// Returns an empty report when `mode` is [`Mode::None`], when `tables` is empty, or
/// when the database is not CockroachDB (the check skips quietly). Otherwise loads the
/// schema, diffs it, and — if there are problems — either returns `Err` (validate, or
/// any contradiction) or logs to stderr and returns the report (warn).
pub async fn verify_multiregion(
    db: &DatabaseConnection,
    mode: Mode,
    tables: Vec<MultiregionTable>,
) -> Result<MultiregionReport, DbErr> {
    if mode == Mode::None || tables.is_empty() {
        return Ok(MultiregionReport::default());
    }
    if !is_cockroachdb(db).await? {
        eprintln!("CockroachDB multiregion check skipped: the database is not CockroachDB");
        return Ok(MultiregionReport::default());
    }
    let state = load(db, &tables).await?;
    let report = evaluate(&state, &tables);
    if report.is_empty() {
        return Ok(report);
    }
    if fails_startup(mode, &report) {
        return Err(DbErr::Custom(report.format(mode)));
    }
    eprintln!("{}", report.format(mode));
    Ok(report)
}

/// Whether `db` is CockroachDB. The check skips other databases quietly.
async fn is_cockroachdb(db: &DatabaseConnection) -> Result<bool, DbErr> {
    let row = db
        .query_one(Statement::from_string(PG, "SELECT version()"))
        .await?;
    Ok(row
        .and_then(|r| r.try_get_by::<String, _>(0usize).ok())
        .map(|v| v.contains("CockroachDB"))
        .unwrap_or(false))
}

fn name_of(table: &MultiregionTable, current_schema: &str) -> TableName {
    TableName {
        schema: table
            .schema
            .clone()
            .unwrap_or_else(|| current_schema.to_string()),
        table: table.table.clone(),
    }
}

/// Reads what [`evaluate`] needs. Uses `SHOW` statements and `information_schema`,
/// never `crdb_internal`, matching the Spring starter.
async fn load(
    db: &DatabaseConnection,
    tables: &[MultiregionTable],
) -> Result<MultiregionState, DbErr> {
    let row = db
        .query_one(Statement::from_string(
            PG,
            "SELECT current_database(), current_schema()",
        ))
        .await?
        .ok_or_else(|| {
            DbErr::Custom("multiregion check: current_database() returned no row".into())
        })?;
    let database: String = row.try_get_by::<String, _>(0usize)?;
    let current_schema: String = row
        .try_get_by::<Option<String>, _>(1usize)?
        .unwrap_or_else(|| "public".to_string());

    let primary_region = db
        .query_one(Statement::from_sql_and_values(
            PG,
            "SELECT primary_region FROM [SHOW DATABASES] WHERE database_name = $1",
            [Value::from(database.clone())],
        ))
        .await?
        .and_then(|r| r.try_get_by::<Option<String>, _>(0usize).ok().flatten());

    let mut cluster_regions = Vec::new();
    for r in db
        .query_all(Statement::from_string(
            PG,
            "SELECT region FROM [SHOW REGIONS FROM CLUSTER] ORDER BY region",
        ))
        .await?
    {
        cluster_regions.push(r.try_get_by::<String, _>(0usize)?);
    }

    let mut keywords = HashSet::new();
    for r in db
        .query_all(Statement::from_string(
            PG,
            "SELECT word FROM pg_get_keywords() WHERE catcode IN ('R', 'T')",
        ))
        .await?
    {
        keywords.insert(r.try_get_by::<String, _>(0usize)?);
    }

    let wanted: HashSet<TableName> = tables.iter().map(|t| name_of(t, &current_schema)).collect();

    let mut found: HashMap<TableName, TableInfo> = HashMap::new();
    for r in db
        .query_all(Statement::from_string(
            PG,
            "SELECT schema_name, table_name, locality FROM [SHOW TABLES] WHERE type = 'table'",
        ))
        .await?
    {
        let name = TableName {
            schema: r.try_get_by::<String, _>(0usize)?,
            table: r.try_get_by::<String, _>(1usize)?,
        };
        if wanted.contains(&name) {
            found.insert(
                name,
                TableInfo {
                    locality: r.try_get_by::<Option<String>, _>(2usize)?,
                    column_types: HashMap::new(),
                },
            );
        }
    }

    for table in tables {
        if table.kind != Kind::RegionalByRow {
            continue;
        }
        let name = name_of(table, &current_schema);
        if !found.contains_key(&name) {
            continue;
        }
        let mut types = HashMap::new();
        for r in db
            .query_all(Statement::from_sql_and_values(
                PG,
                "SELECT column_name, crdb_sql_type FROM information_schema.columns \
                 WHERE table_catalog = current_database() AND table_schema = $1 AND table_name = $2",
                [
                    Value::from(name.schema.clone()),
                    Value::from(name.table.clone()),
                ],
            ))
            .await?
        {
            types.insert(
                r.try_get_by::<String, _>(0usize)?,
                r.try_get_by::<String, _>(1usize)?,
            );
        }
        if let Some(info) = found.get_mut(&name) {
            info.column_types = types;
        }
    }

    Ok(MultiregionState {
        database,
        current_schema,
        primary_region,
        cluster_regions,
        tables: found,
        keywords,
    })
}
