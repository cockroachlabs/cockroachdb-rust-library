//! A fake database error carrying a chosen SQLSTATE.

use cockroachdb_core::SqlState;

/// An error that reports a configurable SQLSTATE, for testing retry logic without
/// a real driver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeDbError {
    code: Option<String>,
    message: String,
}

impl FakeDbError {
    /// An error carrying `code` as its SQLSTATE.
    pub fn with_code(code: impl Into<String>) -> Self {
        let code = code.into();
        Self {
            message: format!("fake db error (SQLSTATE {code})"),
            code: Some(code),
        }
    }

    /// A serialization-failure error (`SQLSTATE 40001`).
    pub fn serialization_failure() -> Self {
        Self::with_code(cockroachdb_core::SQLSTATE_SERIALIZATION_FAILURE)
    }

    /// An error with no SQLSTATE (non-retryable), carrying `message`.
    pub fn message(message: impl Into<String>) -> Self {
        Self {
            code: None,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for FakeDbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for FakeDbError {}

impl SqlState for FakeDbError {
    fn sql_state(&self) -> Option<&str> {
        self.code.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialization_failure_is_retryable() {
        let e = FakeDbError::serialization_failure();
        assert_eq!(e.sql_state(), Some("40001"));
        assert!(cockroachdb_core::is_retryable(&e));
    }

    #[test]
    fn plain_message_is_not_retryable() {
        let e = FakeDbError::message("boom");
        assert_eq!(e.sql_state(), None);
        assert!(!cockroachdb_core::is_retryable(&e));
    }
}
