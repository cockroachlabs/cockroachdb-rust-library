//! SQLSTATE-based error classification.
//!
//! CockroachDB reports contention with `SQLSTATE 40001` (serialization failure).
//! Adapters implement [`SqlState`] for their driver's error type so the core can
//! decide whether an operation is retryable, mirroring the Go library's
//! `sqlStateError` interface and its source-chain unwrapping.

/// `SQLSTATE 40001` — serialization failure (the retryable CockroachDB conflict).
pub const SQLSTATE_SERIALIZATION_FAILURE: &str = "40001";

/// `SQLSTATE 40P01` — deadlock detected (also safe to retry).
pub const SQLSTATE_DEADLOCK: &str = "40P01";

/// Exposes a Postgres `SQLSTATE` code from a driver error.
///
/// The SQLx adapter implements this for `sqlx::Error`; the SeaORM adapter reuses
/// that implementation.
pub trait SqlState {
    /// Returns the five-character SQLSTATE code if this error carries one.
    fn sql_state(&self) -> Option<&str>;
}

/// Returns `true` for SQLSTATE codes that should be retried.
pub fn is_retryable_code(code: &str) -> bool {
    code == SQLSTATE_SERIALIZATION_FAILURE || code == SQLSTATE_DEADLOCK
}

/// Returns `true` if `err` carries a retryable SQLSTATE.
pub fn is_retryable<E: SqlState + ?Sized>(err: &E) -> bool {
    err.sql_state().map(is_retryable_code).unwrap_or(false)
}

/// Returns `true` if `err` is specifically a serialization failure (`40001`).
pub fn is_serialization_failure<E: SqlState + ?Sized>(err: &E) -> bool {
    err.sql_state()
        .map(|c| c == SQLSTATE_SERIALIZATION_FAILURE)
        .unwrap_or(false)
}

/// Walks a [`std::error::Error`] source chain, applying `extract` to each error
/// to recover a SQLSTATE code, and returns `true` if any is retryable.
///
/// This mirrors the Go library's behaviour of unwrapping wrapped errors before
/// classifying them. Adapters pass an `extract` closure that knows how to read a
/// code from their driver's concrete error type.
pub fn retryable_in_chain(
    err: &(dyn std::error::Error + 'static),
    extract: impl Fn(&(dyn std::error::Error + 'static)) -> Option<String>,
) -> bool {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(e) = current {
        if let Some(code) = extract(e) {
            if is_retryable_code(&code) {
                return true;
            }
        }
        current = e.source();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Coded(Option<&'static str>);
    impl SqlState for Coded {
        fn sql_state(&self) -> Option<&str> {
            self.0
        }
    }

    #[test]
    fn classifies_known_codes() {
        assert!(is_retryable_code("40001"));
        assert!(is_retryable_code("40P01"));
        assert!(!is_retryable_code("23505")); // unique_violation
        assert!(!is_retryable_code(""));
    }

    #[test]
    fn classifies_errors() {
        assert!(is_retryable(&Coded(Some("40001"))));
        assert!(is_serialization_failure(&Coded(Some("40001"))));
        assert!(!is_serialization_failure(&Coded(Some("40P01"))));
        assert!(!is_retryable(&Coded(None)));
    }

    #[derive(Debug)]
    struct Wrapper(Box<dyn std::error::Error + 'static>);
    impl std::fmt::Display for Wrapper {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "wrapper")
        }
    }
    impl std::error::Error for Wrapper {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(self.0.as_ref())
        }
    }

    #[derive(Debug)]
    struct Leaf(&'static str);
    impl std::fmt::Display for Leaf {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "leaf {}", self.0)
        }
    }
    impl std::error::Error for Leaf {}

    #[test]
    fn walks_source_chain() {
        let wrapped = Wrapper(Box::new(Leaf("40001")));
        let found = retryable_in_chain(&wrapped, |e| {
            e.downcast_ref::<Leaf>().map(|l| l.0.to_string())
        });
        assert!(found);

        let wrapped_ok = Wrapper(Box::new(Leaf("23505")));
        let found_ok = retryable_in_chain(&wrapped_ok, |e| {
            e.downcast_ref::<Leaf>().map(|l| l.0.to_string())
        });
        assert!(!found_ok);
    }
}
