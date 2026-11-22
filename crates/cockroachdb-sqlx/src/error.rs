//! SQLSTATE classification for sqlx errors.
//!
//! The orphan rule prevents implementing [`cockroachdb_core::SqlState`] for the
//! foreign `sqlx::Error`, so classification is exposed as free functions —
//! matching the Go library's `pgxadapter.IsRetryable`.

/// Extracts the Postgres `SQLSTATE` code from a sqlx error, if it carries one.
///
/// Only `sqlx::Error::Database` errors have a code; everything else returns
/// `None`.
pub fn sql_state(err: &sqlx::Error) -> Option<String> {
    match err {
        sqlx::Error::Database(db) => db.code().map(|c| c.into_owned()),
        _ => None,
    }
}

/// Returns `true` if the error is a retryable CockroachDB conflict
/// (`40001` serialization failure or `40P01` deadlock).
pub fn is_retryable(err: &sqlx::Error) -> bool {
    sql_state(err)
        .as_deref()
        .map(cockroachdb_core::is_retryable_code)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_database_errors_have_no_code() {
        assert_eq!(sql_state(&sqlx::Error::PoolClosed), None);
        assert!(!is_retryable(&sqlx::Error::PoolClosed));
    }
}
