//! Retry policy with exponential backoff and jitter.
//!
//! The policy controls how the SQLx adapter's `execute_tx` loop re-runs a
//! transaction after a retryable [`SQLSTATE`](crate::error). Defaults match the
//! Go library: 5 retries, 50 ms initial delay, 5 s cap, 2.0 multiplier, jitter on.

use std::time::Duration;

/// Exponential-backoff retry configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct RetryPolicy {
    /// Maximum number of retries *after* the first attempt.
    pub max_retries: u32,
    /// Delay before the first retry.
    pub initial_delay: Duration,
    /// Upper bound on any single delay.
    pub max_delay: Duration,
    /// Backoff growth factor per attempt.
    pub multiplier: f64,
    /// When `true`, apply full jitter in `[0, computed_delay]`.
    pub jitter: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 5,
            initial_delay: Duration::from_millis(50),
            max_delay: Duration::from_secs(5),
            multiplier: 2.0,
            jitter: true,
        }
    }
}

impl RetryPolicy {
    /// Computes the backoff delay before retry `attempt` (0-based: `attempt == 0`
    /// is the delay before the first retry).
    ///
    /// The exponential base is `initial_delay * multiplier^attempt`, capped at
    /// `max_delay`. With `jitter`, the result is uniformly sampled in
    /// `[0, capped]` (full jitter).
    pub fn backoff_delay(&self, attempt: u32) -> Duration {
        let base = self.initial_delay.as_secs_f64() * self.multiplier.powi(attempt as i32);
        let capped = base.min(self.max_delay.as_secs_f64());
        let secs = if self.jitter {
            capped * jitter_fraction()
        } else {
            capped
        };
        Duration::from_secs_f64(secs.max(0.0))
    }
}

/// Cheap, dependency-free jitter source in `[0, 1)` derived from the clock.
///
// ponytail: nanos-based pseudo-random fraction, fine for backoff jitter; swap for
// a real RNG only if a provable distribution is ever required.
fn jitter_fraction() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    (nanos % 1_000_000) as f64 / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_go_library() {
        let p = RetryPolicy::default();
        assert_eq!(p.max_retries, 5);
        assert_eq!(p.initial_delay, Duration::from_millis(50));
        assert_eq!(p.max_delay, Duration::from_secs(5));
        assert_eq!(p.multiplier, 2.0);
        assert!(p.jitter);
    }

    #[test]
    fn backoff_is_exponential_without_jitter() {
        let p = RetryPolicy {
            jitter: false,
            ..Default::default()
        };
        assert_eq!(p.backoff_delay(0), Duration::from_millis(50));
        assert_eq!(p.backoff_delay(1), Duration::from_millis(100));
        assert_eq!(p.backoff_delay(2), Duration::from_millis(200));
    }

    #[test]
    fn backoff_is_capped() {
        let p = RetryPolicy {
            jitter: false,
            ..Default::default()
        };
        // Attempt 10 would be 50ms * 2^10 = ~51s, capped to 5s.
        assert_eq!(p.backoff_delay(10), Duration::from_secs(5));
    }

    #[test]
    fn jitter_stays_within_cap() {
        let p = RetryPolicy::default();
        for attempt in 0..6 {
            let d = p.backoff_delay(attempt);
            let ceiling = (p.initial_delay.as_secs_f64() * p.multiplier.powi(attempt as i32))
                .min(p.max_delay.as_secs_f64());
            assert!(d.as_secs_f64() <= ceiling + 1e-9);
        }
    }
}
