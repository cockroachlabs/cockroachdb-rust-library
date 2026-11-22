//! `CREATE CHANGEFEED` statement builder.

use crate::CdcError;
use std::collections::BTreeMap;

fn ident(name: &str) -> Result<String, CdcError> {
    if name.trim().is_empty() {
        return Err(CdcError::InvalidIdentifier(name.to_string()));
    }
    Ok(format!("\"{}\"", name.replace('"', "\"\"")))
}

fn lit(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// A validated changefeed specification. Produce the SQL with [`ChangefeedSpec::to_sql`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangefeedSpec {
    table: String,
    columns: Vec<String>,
    sink: String,
    options: BTreeMap<String, String>,
}

impl ChangefeedSpec {
    /// Starts a builder.
    pub fn builder() -> ChangefeedSpecBuilder {
        ChangefeedSpecBuilder::default()
    }

    /// The target table.
    pub fn table(&self) -> &str {
        &self.table
    }

    /// The sink URI (webhook target).
    pub fn sink(&self) -> &str {
        &self.sink
    }

    /// The `WITH` options, sorted by key.
    pub fn options(&self) -> &BTreeMap<String, String> {
        &self.options
    }

    /// Renders the `CREATE CHANGEFEED` statement. Options with an empty value are
    /// rendered as bare flags (e.g. `updated`); others as `key = 'value'`.
    pub fn to_sql(&self) -> String {
        let mut sql = String::from("CREATE CHANGEFEED FOR TABLE ");
        // Identifiers were validated at build time, so unwrap is safe here.
        sql.push_str(&ident(&self.table).unwrap());
        if !self.columns.is_empty() {
            let cols: Vec<String> = self.columns.iter().map(|c| ident(c).unwrap()).collect();
            sql.push_str(" (");
            sql.push_str(&cols.join(", "));
            sql.push(')');
        }
        sql.push_str(" INTO ");
        sql.push_str(&lit(&self.sink));
        if !self.options.is_empty() {
            let opts: Vec<String> = self
                .options
                .iter()
                .map(|(k, v)| {
                    if v.is_empty() {
                        k.clone()
                    } else {
                        format!("{} = {}", k, lit(v))
                    }
                })
                .collect();
            sql.push_str(" WITH ");
            sql.push_str(&opts.join(", "));
        }
        sql
    }
}

/// Fluent builder for [`ChangefeedSpec`].
#[derive(Debug, Clone, Default)]
pub struct ChangefeedSpecBuilder {
    table: Option<String>,
    columns: Vec<String>,
    sink: Option<String>,
    options: BTreeMap<String, String>,
}

impl ChangefeedSpecBuilder {
    /// Sets the target table.
    pub fn table(mut self, table: impl Into<String>) -> Self {
        self.table = Some(table.into());
        self
    }

    /// Restricts the changefeed to specific columns.
    pub fn columns<I, S>(mut self, columns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.columns = columns.into_iter().map(Into::into).collect();
        self
    }

    /// Sets the webhook sink URI.
    pub fn webhook_target(mut self, url: impl Into<String>) -> Self {
        self.sink = Some(url.into());
        self
    }

    /// Adds a raw `WITH` option (`key = 'value'`). An empty `value` renders the
    /// key as a bare flag.
    pub fn option(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.options.insert(key.into(), value.into());
        self
    }

    /// `WITH updated` — include the MVCC timestamp in each row.
    pub fn with_updated(self) -> Self {
        self.option("updated", "")
    }

    /// `WITH diff` — include the previous row image (`before`).
    pub fn with_diff(self) -> Self {
        self.option("diff", "")
    }

    /// `WITH resolved = '<interval>'` — emit resolved timestamps.
    pub fn resolved(self, interval: impl Into<String>) -> Self {
        self.option("resolved", interval)
    }

    /// `WITH initial_scan = 'yes'`.
    pub fn initial_scan(self) -> Self {
        self.option("initial_scan", "yes")
    }

    /// `WITH initial_scan = 'no'`.
    pub fn no_initial_scan(self) -> Self {
        self.option("initial_scan", "no")
    }

    /// Sets the webhook envelope format (e.g. `wrapped`, `bare`).
    pub fn envelope(self, envelope: impl Into<String>) -> Self {
        self.option("envelope", envelope)
    }

    /// Skips TLS verification on the sink (`WITH tls_skip_verify`).
    pub fn skip_tls_verify(self) -> Self {
        self.option("tls_skip_verify", "")
    }

    /// Validates and builds the spec.
    pub fn build(self) -> Result<ChangefeedSpec, CdcError> {
        let table = self.table.ok_or(CdcError::MissingTable)?;
        if table.trim().is_empty() {
            return Err(CdcError::InvalidIdentifier(table));
        }
        for c in &self.columns {
            if c.trim().is_empty() {
                return Err(CdcError::InvalidIdentifier(c.clone()));
            }
        }
        let sink = self.sink.ok_or(CdcError::MissingSink)?;
        if sink.trim().is_empty() {
            return Err(CdcError::MissingSink);
        }
        Ok(ChangefeedSpec {
            table,
            columns: self.columns,
            sink,
            options: self.options,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_to_sql() {
        let spec = ChangefeedSpec::builder()
            .table("accounts")
            .webhook_target("https://sink.example/cdc")
            .build()
            .unwrap();
        assert_eq!(
            spec.to_sql(),
            "CREATE CHANGEFEED FOR TABLE \"accounts\" INTO 'https://sink.example/cdc'"
        );
    }

    #[test]
    fn with_columns_and_options() {
        let spec = ChangefeedSpec::builder()
            .table("accounts")
            .columns(["id", "balance"])
            .webhook_target("https://sink.example/cdc")
            .with_updated()
            .resolved("10s")
            .build()
            .unwrap();
        // Options are sorted (BTreeMap): resolved, updated.
        assert_eq!(
            spec.to_sql(),
            "CREATE CHANGEFEED FOR TABLE \"accounts\" (\"id\", \"balance\") \
             INTO 'https://sink.example/cdc' WITH resolved = '10s', updated"
        );
    }

    #[test]
    fn missing_table_and_sink_are_errors() {
        assert!(matches!(
            ChangefeedSpec::builder().webhook_target("x").build(),
            Err(CdcError::MissingTable)
        ));
        assert!(matches!(
            ChangefeedSpec::builder().table("t").build(),
            Err(CdcError::MissingSink)
        ));
    }

    #[test]
    fn empty_identifiers_rejected() {
        assert!(matches!(
            ChangefeedSpec::builder()
                .table("t")
                .columns([""])
                .webhook_target("x")
                .build(),
            Err(CdcError::InvalidIdentifier(_))
        ));
    }

    #[test]
    fn identifiers_are_escaped() {
        let spec = ChangefeedSpec::builder()
            .table("we\"ird")
            .webhook_target("sink'y")
            .build()
            .unwrap();
        assert_eq!(
            spec.to_sql(),
            "CREATE CHANGEFEED FOR TABLE \"we\"\"ird\" INTO 'sink''y'"
        );
    }
}
