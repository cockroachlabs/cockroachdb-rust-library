//! DDL string builders for multi-region schema changes.
//!
//! Small, dependency-free helpers that produce the SQL for the common
//! multi-region operations, mirroring the Go library's `ddl` package. Identifiers
//! are double-quoted and region names are single-quoted string literals.

/// Quotes a SQL identifier, escaping embedded double quotes.
pub fn ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Escapes a single-quoted SQL string literal body.
fn lit(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// `ALTER DATABASE <db> SET PRIMARY REGION '<region>'`.
pub fn set_primary_region(database: &str, region: &str) -> String {
    format!(
        "ALTER DATABASE {} SET PRIMARY REGION {}",
        ident(database),
        lit(region)
    )
}

/// `ALTER DATABASE <db> ADD REGION '<region>'`.
pub fn add_region(database: &str, region: &str) -> String {
    format!(
        "ALTER DATABASE {} ADD REGION {}",
        ident(database),
        lit(region)
    )
}

/// `ALTER TABLE <table> SET LOCALITY REGIONAL BY ROW`.
pub fn set_locality_regional_by_row(table: &str) -> String {
    format!("ALTER TABLE {} SET LOCALITY REGIONAL BY ROW", ident(table))
}

/// `ALTER TABLE <table> SET LOCALITY GLOBAL`.
pub fn set_locality_global(table: &str) -> String {
    format!("ALTER TABLE {} SET LOCALITY GLOBAL", ident(table))
}

/// `ALTER TABLE <table> SET LOCALITY REGIONAL BY TABLE IN '<region>'`.
pub fn set_locality_regional_by_table(table: &str, region: &str) -> String {
    format!(
        "ALTER TABLE {} SET LOCALITY REGIONAL BY TABLE IN {}",
        ident(table),
        lit(region)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_quoted_and_escaped() {
        assert_eq!(ident("accounts"), "\"accounts\"");
        assert_eq!(ident("we\"ird"), "\"we\"\"ird\"");
    }

    #[test]
    fn region_ddl() {
        assert_eq!(
            set_primary_region("bank", "us-east1"),
            "ALTER DATABASE \"bank\" SET PRIMARY REGION 'us-east1'"
        );
        assert_eq!(
            add_region("bank", "eu-west1"),
            "ALTER DATABASE \"bank\" ADD REGION 'eu-west1'"
        );
    }

    #[test]
    fn locality_ddl() {
        assert_eq!(
            set_locality_regional_by_row("accounts"),
            "ALTER TABLE \"accounts\" SET LOCALITY REGIONAL BY ROW"
        );
        assert_eq!(
            set_locality_global("ref_data"),
            "ALTER TABLE \"ref_data\" SET LOCALITY GLOBAL"
        );
        assert_eq!(
            set_locality_regional_by_table("orders", "us-east1"),
            "ALTER TABLE \"orders\" SET LOCALITY REGIONAL BY TABLE IN 'us-east1'"
        );
    }
}
