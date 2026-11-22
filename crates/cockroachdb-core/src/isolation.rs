//! Transaction isolation levels.

/// A CockroachDB transaction isolation level.
///
/// [`Serializable`](IsolationLevel::Serializable) is the default and strongest
/// guarantee; [`ReadCommitted`](IsolationLevel::ReadCommitted) reduces retry
/// frequency at the cost of weaker isolation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IsolationLevel {
    /// `SERIALIZABLE` — the default.
    #[default]
    Serializable,
    /// `READ COMMITTED`.
    ReadCommitted,
}

impl IsolationLevel {
    /// The SQL spelling used in `SET TRANSACTION ISOLATION LEVEL ...`.
    pub fn as_sql(&self) -> &'static str {
        match self {
            IsolationLevel::Serializable => "SERIALIZABLE",
            IsolationLevel::ReadCommitted => "READ COMMITTED",
        }
    }
}

impl std::fmt::Display for IsolationLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_sql())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_serializable() {
        assert_eq!(IsolationLevel::default(), IsolationLevel::Serializable);
    }

    #[test]
    fn sql_spellings() {
        assert_eq!(IsolationLevel::Serializable.as_sql(), "SERIALIZABLE");
        assert_eq!(IsolationLevel::ReadCommitted.as_sql(), "READ COMMITTED");
        assert_eq!(IsolationLevel::ReadCommitted.to_string(), "READ COMMITTED");
    }
}
