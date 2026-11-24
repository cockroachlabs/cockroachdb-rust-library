//! Multi-region schema check — the pure, driver-agnostic half.
//!
//! Compares an application's declared entity locality (`#[derive(CrdbLocality)]`,
//! see `cockroachdb-macros`) against the live multi-region configuration of its
//! database and tables, and suggests SQL for every difference. It only produces
//! strings; it never touches a connection — the SeaORM adapter loads the live
//! [`MultiregionState`] and calls [`evaluate`].
//!
//! Faithful port of the Spring Boot starter's `MultiregionCheck`: same SHOW /
//! `information_schema` reads (no `crdb_internal`), same both-directions diff, same
//! contradiction rules. The fix-up SQL is built from the shared [`crate::ddl`]
//! builders.

use crate::ddl;
use std::collections::{HashMap, HashSet};

const REGION_TYPE: &str = "crdb_internal_region";
const IMPLICIT_REGION_COLUMN: &str = "crdb_region";
const REGIONAL_BY_ROW: &str = "REGIONAL BY ROW";
const OVERVIEW: &str = "https://docs.cockroachlabs.com/docs/stable/multiregion-overview";
const CONTRADICTION: &str = "Contradiction: ";

/// How the check reacts to problems. [`Mode::Warn`] is the default.
///
/// A *contradiction* (an entity that declares one locality while its table has the
/// other, or entities that disagree about the same table) fails startup in every
/// mode but [`Mode::None`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// No check.
    None,
    /// Log any problems, with SQL that fixes them, and continue. A contradiction
    /// still fails.
    #[default]
    Warn,
    /// Fail on any problem.
    Validate,
}

impl Mode {
    /// Lower-case name, for the report header.
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::None => "none",
            Mode::Warn => "warn",
            Mode::Validate => "validate",
        }
    }
}

/// The locality an entity declares, via `#[derive(CrdbLocality)]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclaredLocality {
    /// `#[crdb(global)]` — the table should be `GLOBAL`.
    Global,
    /// `#[crdb(regional_by_row)]` — the table should be `REGIONAL BY ROW`, by the
    /// `#[crdb(region)]` field's column when one is given.
    RegionalByRow {
        /// DB column name of the `#[crdb(region)]` field, or `None` for the
        /// implicit `crdb_region`.
        region_column: Option<String>,
    },
}

/// Implemented by `#[derive(CrdbLocality)]` on a SeaORM entity's `Model`, for the
/// generated `Entity` type. Supplies the declared locality the check diffs against
/// the live table.
pub trait CrdbLocality {
    /// The declared locality of this entity.
    fn crdb_locality() -> DeclaredLocality;
}

/// What an entity's declaration expects of its table; [`Kind::None`] for an entity
/// with no locality marker. [`Kind::Both`] marks an entity (or set of entities on
/// one table) that declares conflicting localities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// No locality marker.
    None,
    /// Declared `GLOBAL`.
    Global,
    /// Declared `REGIONAL BY ROW`.
    RegionalByRow,
    /// Declares both — a contradiction.
    Both,
}

/// An entity's table and what its locality marker, if any, expects of it. Names
/// are as CockroachDB stores them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiregionTable {
    /// The entity type name, for messages.
    pub entity: String,
    /// The table's schema, or `None` for the connection's current schema.
    pub schema: Option<String>,
    /// The table name.
    pub table: String,
    /// What the marker expects; [`Kind::None`] for an unmarked entity.
    pub kind: Kind,
    /// The `#[crdb(region)]` column, or `None`.
    pub region_column: Option<String>,
}

/// A schema-qualified table name, as CockroachDB stores it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TableName {
    /// Schema.
    pub schema: String,
    /// Table.
    pub table: String,
}

/// The live facts about one table.
#[derive(Debug, Clone, Default)]
pub struct TableInfo {
    /// Locality as `SHOW TABLES` reports it; `None` outside a multi-region database.
    pub locality: Option<String>,
    /// Column types (`information_schema.columns.crdb_sql_type`), read only for the
    /// columns the check needs.
    pub column_types: HashMap<String, String>,
}

/// What the check reads from CockroachDB. Loaded by the SeaORM adapter.
#[derive(Debug, Clone)]
pub struct MultiregionState {
    /// The current database.
    pub database: String,
    /// The schema unqualified names resolve to.
    pub current_schema: String,
    /// The database's primary region, or `None` when it is not multi-region.
    pub primary_region: Option<String>,
    /// The cluster's node regions; empty when no node has a region locality.
    pub cluster_regions: Vec<String>,
    /// The checked tables that exist, by schema and name.
    pub tables: HashMap<TableName, TableInfo>,
    /// Keywords that must be quoted when used as an identifier.
    pub keywords: HashSet<String>,
}

/// The problems the check found, and SQL that fixes them.
#[derive(Debug, Clone, Default)]
pub struct MultiregionReport {
    problems: Vec<String>,
    statements: Vec<String>,
    contradictions: usize,
}

impl MultiregionReport {
    fn problem(&mut self, problem: impl Into<String>) {
        self.problems.push(problem.into());
    }

    fn contradiction(&mut self, problem: impl Into<String>) {
        self.problems
            .push(format!("{CONTRADICTION}{}", problem.into()));
        self.contradictions += 1;
    }

    fn statement(&mut self, statement: impl Into<String>) {
        self.statements.push(statement.into());
    }

    /// The problems found.
    pub fn problems(&self) -> &[String] {
        &self.problems
    }

    /// The suggested SQL.
    pub fn statements(&self) -> &[String] {
        &self.statements
    }

    /// Whether the check found nothing.
    pub fn is_empty(&self) -> bool {
        self.problems.is_empty()
    }

    /// Whether any problem is a contradiction.
    pub fn has_contradictions(&self) -> bool {
        self.contradictions > 0
    }

    /// The report as one message.
    pub fn format(&self, mode: Mode) -> String {
        let mut out = format!(
            "CockroachDB multiregion check (mode={}) found {} {}:",
            mode.as_str(),
            self.problems.len(),
            if self.problems.len() == 1 {
                "problem"
            } else {
                "problems"
            }
        );
        for problem in &self.problems {
            out.push_str("\n  - ");
            out.push_str(&problem.replace('\n', "\n    "));
        }
        if self.contradictions > 0 {
            out.push_str(
                "\n\nContradictions stop startup in every mode but none: the annotations and the \
                 schema disagree about where rows live, so one of them is wrong.",
            );
        }
        if !self.statements.is_empty() {
            out.push_str(
                "\n\nSuggested SQL (review, then add to a migration; or alter the annotations):",
            );
            for statement in &self.statements {
                out.push_str("\n  ");
                out.push_str(statement);
            }
        }
        out.push_str(&format!("\n\nSee also: {OVERVIEW}"));
        out
    }
}

/// Whether a non-empty `report` should fail startup under `mode`: validate fails on
/// any problem, and a contradiction fails in every mode but none. This is the
/// decision the SeaORM entry point makes — isolated here so it is unit-testable
/// without a database.
pub fn fails_startup(mode: Mode, report: &MultiregionReport) -> bool {
    !report.is_empty() && (mode == Mode::Validate || report.has_contradictions())
}

/// Lists every difference between `tables` and `state`, with SQL to fix the schema
/// or a description of the marker to fix. `tables` holds every entity, marked or
/// not: a `GLOBAL` or `REGIONAL BY ROW` table needs its marker too.
pub fn evaluate(state: &MultiregionState, tables: &[MultiregionTable]) -> MultiregionReport {
    let mut report = MultiregionReport::default();
    let keywords = &state.keywords;
    let multiregion = state.primary_region.is_some();
    let annotated = tables.iter().filter(|t| t.kind != Kind::None).count();

    if !multiregion && annotated > 0 {
        let database = quote(&state.database, keywords);
        let names = tables
            .iter()
            .filter(|t| t.kind != Kind::None)
            .map(|t| t.entity.clone())
            .collect::<Vec<_>>()
            .join(", ");
        let affected = format!(
            "#[crdb(global)]/#[crdb(regional_by_row)] on {} ({names}) have no effect.",
            count(annotated, "entity", "entities")
        );
        if state.cluster_regions.is_empty() {
            report.problem(format!(
                "Database {database} is not multi-region; {affected}\nThe cluster has no regions, \
                 so the database can't be made multi-region yet: start every node with \
                 --locality=region=<region>."
            ));
        } else {
            report.problem(format!(
                "Database {database} is not multi-region; {affected}\nThe cluster has regions: {}. \
                 Choose the primary region deliberately; the SQL below uses the first one.",
                state.cluster_regions.join(", ")
            ));
            report.statement(format!(
                "{};",
                ddl::set_primary_region(&state.database, &state.cluster_regions[0])
            ));
            for region in &state.cluster_regions[1..] {
                report.statement(format!("{};", ddl::add_region(&state.database, region)));
            }
        }
    }

    // Group entities by their resolved table, preserving first-seen order.
    let mut order: Vec<TableName> = Vec::new();
    let mut by_table: HashMap<TableName, Vec<&MultiregionTable>> = HashMap::new();
    for table in tables {
        let name = name_of(table, &state.current_schema);
        by_table
            .entry(name.clone())
            .or_insert_with(|| {
                order.push(name.clone());
                Vec::new()
            })
            .push(table);
    }

    for name in &order {
        let entities = &by_table[name];
        let qualified = format!(
            "{}.{}",
            quote(&name.schema, keywords),
            quote(&name.table, keywords)
        );
        let info = state.tables.get(name);

        let marked: Vec<&MultiregionTable> = entities
            .iter()
            .copied()
            .filter(|t| t.kind != Kind::None)
            .collect();
        let mut kinds: Vec<Kind> = Vec::new();
        for t in &marked {
            if !kinds.contains(&t.kind) {
                kinds.push(t.kind);
            }
        }

        if kinds.is_empty() {
            unannotated(&mut report, &qualified, info, entities, multiregion);
        } else if kinds.len() > 1 || kinds.contains(&Kind::Both) {
            conflicting(&mut report, &qualified, &marked);
        } else {
            let table = marked[0];
            match table.kind {
                Kind::Global => global(&mut report, name, &qualified, info, table, multiregion),
                _ => regional_by_row(
                    &mut report,
                    state,
                    name,
                    table,
                    &qualified,
                    info,
                    multiregion,
                ),
            }
        }
    }
    report
}

fn global(
    report: &mut MultiregionReport,
    name: &TableName,
    qualified: &str,
    info: Option<&TableInfo>,
    table: &MultiregionTable,
    multiregion: bool,
) {
    let Some(info) = info else {
        missing(report, qualified, "#[crdb(global)]", table);
        return;
    };
    let locality = info.locality.as_deref();
    if !multiregion || locality != Some("GLOBAL") {
        if multiregion {
            let loc = locality.unwrap_or("");
            let problem = format!(
                "Table {qualified} (#[crdb(global)] {}) is {loc}, not GLOBAL.",
                table.entity
            );
            if regional_by_row_column(loc).is_some() {
                report.contradiction(problem);
            } else {
                report.problem(problem);
            }
        }
        report.statement(format!("{};", ddl::set_locality_global(&name.table)));
    }
}

/// An entity with no locality marker whose table is `GLOBAL` or `REGIONAL BY ROW`:
/// the code, not the schema, needs fixing.
fn unannotated(
    report: &mut MultiregionReport,
    qualified: &str,
    info: Option<&TableInfo>,
    entities: &[&MultiregionTable],
    multiregion: bool,
) {
    if !multiregion {
        return;
    }
    let Some(info) = info else {
        return;
    };
    let names = entities
        .iter()
        .map(|t| t.entity.clone())
        .collect::<Vec<_>>()
        .join(", ");
    let locality = info.locality.as_deref().unwrap_or("");
    if locality == "GLOBAL" {
        report.problem(format!(
            "Table {qualified} is GLOBAL, but entity {names} has no #[crdb(global)]. Add \
             #[crdb(global)] to it."
        ));
    } else if let Some(column) = regional_by_row_column(locality) {
        report.problem(format!(
            "Table {qualified} is {locality}, but entity {names} has no #[crdb(regional_by_row)]. \
             Add #[crdb(regional_by_row)] to it, and #[crdb(region)] to the field mapped to \
             {column} so rows are homed in the transaction's region."
        ));
    }
}

fn conflicting(report: &mut MultiregionReport, qualified: &str, entities: &[&MultiregionTable]) {
    if entities.len() == 1 {
        report.contradiction(format!(
            "Entity {} is annotated with both #[crdb(global)] and #[crdb(regional_by_row)]; keep \
             one.",
            entities[0].entity
        ));
        return;
    }
    let described = entities
        .iter()
        .map(|t| {
            let marker = match t.kind {
                Kind::Global => "#[crdb(global)]",
                Kind::RegionalByRow => "#[crdb(regional_by_row)]",
                _ => "#[crdb(global)] and #[crdb(regional_by_row)]",
            };
            format!("{} ({marker})", t.entity)
        })
        .collect::<Vec<_>>()
        .join(", ");
    report.contradiction(format!(
        "Table {qualified} is mapped by entities with conflicting annotations: {described}. \
         Annotate them alike."
    ));
}

#[allow(clippy::too_many_arguments)]
fn regional_by_row(
    report: &mut MultiregionReport,
    state: &MultiregionState,
    name: &TableName,
    table: &MultiregionTable,
    qualified: &str,
    info: Option<&TableInfo>,
    multiregion: bool,
) {
    let Some(info) = info else {
        missing(report, qualified, "#[crdb(regional_by_row)]", table);
        return;
    };
    let locality = info.locality.as_deref().unwrap_or("");
    let wanted_column = table.region_column.as_deref();
    let current_column = regional_by_row_column(locality);
    if multiregion
        && current_column.is_some()
        && (wanted_column.is_none() || wanted_column == current_column.as_deref())
    {
        return;
    }

    let column = wanted_column.unwrap_or(IMPLICIT_REGION_COLUMN);
    let quoted_column = quote(column, &state.keywords);
    let col_type = info.column_types.get(column);
    let description = format!(
        "Table {qualified} (#[crdb(regional_by_row)] {})",
        table.entity
    );
    let (target, stmt) = if column == IMPLICIT_REGION_COLUMN {
        (
            REGIONAL_BY_ROW.to_string(),
            ddl::set_locality_regional_by_row(&name.table),
        )
    } else {
        (
            format!("{REGIONAL_BY_ROW} AS {quoted_column}"),
            ddl::set_locality_regional_by_row_as(&name.table, column),
        )
    };
    let is_global = multiregion && locality == "GLOBAL";
    if is_global {
        report.contradiction(format!("{description} is GLOBAL, not {target}."));
    }
    if let Some(ty) = col_type {
        if !is_region_type(ty) {
            report.problem(format!(
                "{description}: column {quoted_column} is {ty}, but REGIONAL BY ROW needs a \
                 {REGION_TYPE} column. Change it with your migrations."
            ));
            return;
        }
    } else if column != IMPLICIT_REGION_COLUMN {
        report.problem(format!(
            "{description}: column {quoted_column} of the #[crdb(region)] field does not exist. \
             Create it, as a {REGION_TYPE} column, with your migrations."
        ));
        return;
    }

    if multiregion && !is_global {
        report.problem(format!("{description} is {locality}, not {target}."));
    }
    report.statement(format!("{stmt};  -- rewrites the table"));
}

fn missing(
    report: &mut MultiregionReport,
    qualified: &str,
    marker: &str,
    table: &MultiregionTable,
) {
    report.problem(format!(
        "Table {qualified} ({marker} {}) does not exist. Create it with your migrations.",
        table.entity
    ));
}

/// The region column of a `REGIONAL BY ROW` locality, or `None` for any other
/// locality.
fn regional_by_row_column(locality: &str) -> Option<String> {
    let rest = locality.strip_prefix(REGIONAL_BY_ROW)?.trim();
    if rest.is_empty() {
        return Some(IMPLICIT_REGION_COLUMN.to_string());
    }
    let after = rest.strip_prefix("AS ")?;
    parse(after.trim()).into_iter().next()
}

fn is_region_type(ty: &str) -> bool {
    ty == REGION_TYPE || ty.ends_with(&format!(".{REGION_TYPE}"))
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

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

// --- Identifier parsing/quoting, as CockroachDB treats names ---

/// Whether `name` can be written as a plain, unquoted identifier (`[a-z_][a-z0-9_$]*`).
fn is_plain(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '$')
}

/// Quotes `name` when it can't be written as a plain identifier or is a keyword.
fn quote(name: &str, keywords: &HashSet<String>) -> String {
    if is_plain(name) && !keywords.contains(name) {
        name.to_string()
    } else {
        format!("\"{}\"", name.replace('"', "\"\""))
    }
}

/// Splits a possibly qualified, possibly quoted name such as `app."Orders"` into
/// its parts, as CockroachDB stores them: unquoted names fold to lower case,
/// quoted names are kept verbatim.
fn parse(rendered: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut part = String::new();
    let mut in_quotes = false;
    let mut quoted = false;
    let chars: Vec<char> = rendered.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_quotes {
            if c != '"' {
                part.push(c);
            } else if i + 1 < chars.len() && chars[i + 1] == '"' {
                part.push('"');
                i += 1;
            } else {
                in_quotes = false;
            }
        } else if c == '"' {
            in_quotes = true;
            quoted = true;
        } else if c == '.' {
            parts.push(finish(&part, quoted));
            part.clear();
            quoted = false;
        } else {
            part.push(c);
        }
        i += 1;
    }
    parts.push(finish(&part, quoted));
    parts
}

fn finish(part: &str, quoted: bool) -> String {
    if quoted {
        part.to_string()
    } else {
        part.trim().to_lowercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kw() -> HashSet<String> {
        ["order", "select"].into_iter().map(String::from).collect()
    }

    fn state(
        primary: Option<&str>,
        regions: &[&str],
        tables: &[(&str, &str, Option<&str>)],
    ) -> MultiregionState {
        let mut map = HashMap::new();
        for (schema, table, locality) in tables {
            map.insert(
                TableName {
                    schema: schema.to_string(),
                    table: table.to_string(),
                },
                TableInfo {
                    locality: locality.map(String::from),
                    column_types: HashMap::new(),
                },
            );
        }
        MultiregionState {
            database: "app".into(),
            current_schema: "public".into(),
            primary_region: primary.map(String::from),
            cluster_regions: regions.iter().map(|s| s.to_string()).collect(),
            tables: map,
            keywords: kw(),
        }
    }

    fn entity(
        name: &str,
        table: &str,
        kind: Kind,
        region_column: Option<&str>,
    ) -> MultiregionTable {
        MultiregionTable {
            entity: name.into(),
            schema: None,
            table: table.into(),
            kind,
            region_column: region_column.map(String::from),
        }
    }

    // --- identifier quoting ---

    #[test]
    fn quote_plain_keyword_and_special() {
        let k = kw();
        assert_eq!(quote("accounts", &k), "accounts");
        assert_eq!(quote("order", &k), "\"order\""); // keyword
        assert_eq!(quote("Orders", &k), "\"Orders\""); // uppercase -> not plain
        assert_eq!(quote("we\"ird", &k), "\"we\"\"ird\"");
        assert_eq!(quote("", &k), "\"\"");
    }

    #[test]
    fn parse_qualified_quoted() {
        assert_eq!(parse("app.\"Orders\""), vec!["app", "Orders"]);
        assert_eq!(parse("PUBLIC"), vec!["public"]); // unquoted folds to lower
        assert_eq!(parse("\"Mixed\""), vec!["Mixed"]);
    }

    #[test]
    fn rbr_column_parsing() {
        assert_eq!(
            regional_by_row_column("REGIONAL BY ROW").as_deref(),
            Some("crdb_region")
        );
        assert_eq!(
            regional_by_row_column("REGIONAL BY ROW AS \"Region\"").as_deref(),
            Some("Region")
        );
        assert_eq!(
            regional_by_row_column("REGIONAL BY ROW AS region").as_deref(),
            Some("region")
        );
        assert_eq!(regional_by_row_column("GLOBAL"), None);
        assert_eq!(
            regional_by_row_column("REGIONAL BY TABLE IN \"us-east1\""),
            None
        );
    }

    // --- database-level ---

    #[test]
    fn not_multiregion_with_regions_suggests_primary_and_add() {
        let s = state(
            None,
            &["us-east1", "us-west1"],
            &[("public", "ideas", None)],
        );
        let tables = vec![entity("Idea", "ideas", Kind::RegionalByRow, None)];
        let r = evaluate(&s, &tables);
        assert!(!r.is_empty());
        assert!(!r.has_contradictions());
        assert!(r.problems()[0].contains("is not multi-region"));
        let sql = r.statements().join("\n");
        assert!(sql.contains("SET PRIMARY REGION 'us-east1';"));
        assert!(sql.contains("ADD REGION 'us-west1';"));
        assert!(sql.contains("REGIONAL BY ROW;  -- rewrites the table"));
    }

    #[test]
    fn not_multiregion_without_regions_explains() {
        let s = state(None, &[], &[("public", "ideas", None)]);
        let tables = vec![entity("Idea", "ideas", Kind::Global, None)];
        let r = evaluate(&s, &tables);
        assert!(r.problems()[0].contains("cluster has no regions"));
        // No ALTER DATABASE statements, but the table locality fix still appears.
        let sql = r.statements().join("\n");
        assert!(!sql.contains("PRIMARY REGION"));
        assert!(sql.contains("SET LOCALITY GLOBAL;"));
    }

    // --- global() ---

    #[test]
    fn global_ok_is_silent() {
        let s = state(
            Some("us-east1"),
            &["us-east1"],
            &[("public", "ref", Some("GLOBAL"))],
        );
        let r = evaluate(&s, &[entity("Ref", "ref", Kind::Global, None)]);
        assert!(r.is_empty());
    }

    #[test]
    fn global_on_rbr_table_is_contradiction() {
        let s = state(
            Some("us-east1"),
            &["us-east1"],
            &[("public", "ref", Some("REGIONAL BY ROW"))],
        );
        let r = evaluate(&s, &[entity("Ref", "ref", Kind::Global, None)]);
        assert!(r.has_contradictions());
        assert!(r.problems()[0].starts_with(CONTRADICTION));
        assert!(r
            .statements()
            .iter()
            .any(|s| s.contains("SET LOCALITY GLOBAL")));
    }

    #[test]
    fn global_missing_table() {
        let s = state(Some("us-east1"), &["us-east1"], &[]);
        let r = evaluate(&s, &[entity("Ref", "ref", Kind::Global, None)]);
        assert!(r.problems()[0].contains("does not exist"));
        assert!(!r.has_contradictions());
    }

    // --- regional_by_row() ---

    #[test]
    fn rbr_on_global_table_is_contradiction() {
        let s = state(
            Some("us-east1"),
            &["us-east1"],
            &[("public", "ideas", Some("GLOBAL"))],
        );
        let r = evaluate(&s, &[entity("Idea", "ideas", Kind::RegionalByRow, None)]);
        assert!(r.has_contradictions());
        assert!(r.problems()[0].contains("is GLOBAL, not REGIONAL BY ROW"));
    }

    #[test]
    fn rbr_matches_is_silent() {
        let s = state(
            Some("us-east1"),
            &["us-east1"],
            &[("public", "ideas", Some("REGIONAL BY ROW"))],
        );
        let r = evaluate(&s, &[entity("Idea", "ideas", Kind::RegionalByRow, None)]);
        assert!(r.is_empty());
    }

    #[test]
    fn rbr_wrong_column_type_reported_no_statement() {
        let mut s = state(
            Some("us-east1"),
            &["us-east1"],
            &[("public", "ideas", Some("REGIONAL BY ROW"))],
        );
        let info = s
            .tables
            .get_mut(&TableName {
                schema: "public".into(),
                table: "ideas".into(),
            })
            .unwrap();
        info.column_types.insert("region".into(), "STRING".into());
        let r = evaluate(
            &s,
            &[entity("Idea", "ideas", Kind::RegionalByRow, Some("region"))],
        );
        assert!(r.problems()[0].contains("needs a crdb_internal_region column"));
    }

    #[test]
    fn rbr_region_type_column_matches() {
        let mut s = state(
            Some("us-east1"),
            &["us-east1"],
            &[("public", "ideas", Some("REGIONAL BY ROW AS region"))],
        );
        let info = s
            .tables
            .get_mut(&TableName {
                schema: "public".into(),
                table: "ideas".into(),
            })
            .unwrap();
        info.column_types
            .insert("region".into(), "crdb_internal_region".into());
        let r = evaluate(
            &s,
            &[entity("Idea", "ideas", Kind::RegionalByRow, Some("region"))],
        );
        assert!(r.is_empty());
    }

    // --- unannotated (reverse direction) ---

    #[test]
    fn table_global_but_entity_unmarked() {
        let s = state(
            Some("us-east1"),
            &["us-east1"],
            &[("public", "ref", Some("GLOBAL"))],
        );
        let r = evaluate(&s, &[entity("Ref", "ref", Kind::None, None)]);
        assert!(r.problems()[0].contains("has no #[crdb(global)]"));
        assert!(r.statements().is_empty());
    }

    #[test]
    fn table_rbr_but_entity_unmarked() {
        let s = state(
            Some("us-east1"),
            &["us-east1"],
            &[("public", "ideas", Some("REGIONAL BY ROW"))],
        );
        let r = evaluate(&s, &[entity("Idea", "ideas", Kind::None, None)]);
        assert!(r.problems()[0].contains("has no #[crdb(regional_by_row)]"));
    }

    // --- conflicting ---

    #[test]
    fn single_entity_both_markers_is_contradiction() {
        let s = state(
            Some("us-east1"),
            &["us-east1"],
            &[("public", "ideas", Some("GLOBAL"))],
        );
        let r = evaluate(&s, &[entity("Idea", "ideas", Kind::Both, None)]);
        assert!(r.has_contradictions());
        assert!(r.problems()[0].contains("both #[crdb(global)] and #[crdb(regional_by_row)]"));
    }

    #[test]
    fn two_entities_same_table_conflict_is_contradiction() {
        let s = state(
            Some("us-east1"),
            &["us-east1"],
            &[("public", "ideas", Some("GLOBAL"))],
        );
        let r = evaluate(
            &s,
            &[
                entity("A", "ideas", Kind::Global, None),
                entity("B", "ideas", Kind::RegionalByRow, None),
            ],
        );
        assert!(r.has_contradictions());
        assert!(r.problems()[0].contains("conflicting annotations"));
    }

    // --- fails_startup (teeth-check) ---

    #[test]
    fn fails_startup_rules() {
        // A plain problem (not a contradiction) only fails in validate.
        let s = state(
            Some("us-east1"),
            &["us-east1"],
            &[("public", "ref", Some("REGIONAL BY TABLE IN \"us-east1\""))],
        );
        let warn_only = evaluate(&s, &[entity("Ref", "ref", Kind::Global, None)]);
        assert!(!warn_only.is_empty());
        assert!(!warn_only.has_contradictions());
        assert!(!fails_startup(Mode::Warn, &warn_only));
        assert!(fails_startup(Mode::Validate, &warn_only));

        // A contradiction fails even in warn. If this assert flips, the
        // contradiction-fails-in-warn guarantee has been reverted.
        let contradiction = evaluate(
            &state(
                Some("us-east1"),
                &["us-east1"],
                &[("public", "ref", Some("REGIONAL BY ROW"))],
            ),
            &[entity("Ref", "ref", Kind::Global, None)],
        );
        assert!(contradiction.has_contradictions());
        assert!(fails_startup(Mode::Warn, &contradiction));

        // Empty report never fails.
        assert!(!fails_startup(
            Mode::Validate,
            &MultiregionReport::default()
        ));
    }
}
