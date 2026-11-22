//! Follower-read `AS OF SYSTEM TIME` clause builders.
//!
//! Follower reads let CockroachDB serve historical reads from the nearest
//! replica, trading a little staleness for lower latency. These helpers produce
//! the `AS OF SYSTEM TIME` clause to splice into a query; the SQLx adapter's
//! follower-read methods call them. SQL strings match the Go library exactly.

use std::time::Duration;

/// Errors from building a follower-read clause.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FollowerError {
    /// Staleness must be a positive duration.
    #[error("follower read staleness must be positive, got {0:?}")]
    NonPositiveStaleness(Duration),
}

/// `AS OF SYSTEM TIME follower_read_timestamp()` — let CockroachDB pick the
/// freshest safe timestamp for a follower read.
pub fn follower_read_timestamp() -> &'static str {
    "AS OF SYSTEM TIME follower_read_timestamp()"
}

/// `AS OF SYSTEM TIME '-<staleness>'` for a specific staleness interval, e.g.
/// 10 s produces `AS OF SYSTEM TIME '-10s'`.
pub fn follower_read_with_staleness(staleness: Duration) -> Result<String, FollowerError> {
    if staleness.is_zero() {
        return Err(FollowerError::NonPositiveStaleness(staleness));
    }
    Ok(format!(
        "AS OF SYSTEM TIME '-{}'",
        format_interval(staleness)
    ))
}

/// Returns the follower-read clause for a given staleness:
/// `0` uses `follower_read_timestamp()`, otherwise a fixed `-<staleness>` interval.
pub fn follower_read_clause(staleness: Duration) -> Result<String, FollowerError> {
    if staleness.is_zero() {
        Ok(follower_read_timestamp().to_string())
    } else {
        follower_read_with_staleness(staleness)
    }
}

/// `AS OF SYSTEM TIME '<timestamp>'` for an exact point in time. Pass an RFC 3339
/// timestamp string (e.g. `"2026-11-22T10:00:00Z"`).
pub fn as_of_system_time(timestamp: &str) -> String {
    format!("AS OF SYSTEM TIME '{}'", timestamp)
}

/// Formats a duration as a CockroachDB interval literal body (no quotes), e.g.
/// `10s` or `1500ms`. Whole seconds render as `Ns`; sub-second as `Nms`.
fn format_interval(d: Duration) -> String {
    if d.subsec_nanos() == 0 {
        format!("{}s", d.as_secs())
    } else {
        format!("{}ms", d.as_millis())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_clause() {
        assert_eq!(
            follower_read_timestamp(),
            "AS OF SYSTEM TIME follower_read_timestamp()"
        );
    }

    #[test]
    fn staleness_clause() {
        assert_eq!(
            follower_read_with_staleness(Duration::from_secs(10)).unwrap(),
            "AS OF SYSTEM TIME '-10s'"
        );
        assert_eq!(
            follower_read_with_staleness(Duration::from_millis(1500)).unwrap(),
            "AS OF SYSTEM TIME '-1500ms'"
        );
    }

    #[test]
    fn zero_staleness_is_rejected_but_clause_falls_back() {
        assert_eq!(
            follower_read_with_staleness(Duration::ZERO),
            Err(FollowerError::NonPositiveStaleness(Duration::ZERO))
        );
        assert_eq!(
            follower_read_clause(Duration::ZERO).unwrap(),
            "AS OF SYSTEM TIME follower_read_timestamp()"
        );
    }

    #[test]
    fn exact_timestamp() {
        assert_eq!(
            as_of_system_time("2026-11-22T10:00:00Z"),
            "AS OF SYSTEM TIME '2026-11-22T10:00:00Z'"
        );
    }
}
