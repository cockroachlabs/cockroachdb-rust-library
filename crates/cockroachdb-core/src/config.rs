//! Connection/behaviour configuration shared by the adapters.

use crate::isolation::IsolationLevel;
use crate::retry::RetryPolicy;
use std::time::Duration;

/// CockroachDB-aware configuration: retry policy, home region, follower-read
/// behaviour, default isolation level, and read-only mode.
///
/// Build one with [`Config::builder`], the idiomatic-Rust equivalent of the Go
/// library's functional options (`WithMaxRetries`, `WithRegion`, ...).
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Retry policy for serialization conflicts.
    pub retry: RetryPolicy,
    /// Home region for REGIONAL BY ROW writes, if any.
    pub region: Option<String>,
    /// Whether follower reads are enabled by default.
    pub follower_reads: bool,
    /// Default follower-read staleness (`0` = `follower_read_timestamp()`).
    pub staleness: Duration,
    /// Default transaction isolation level.
    pub isolation: IsolationLevel,
    /// Whether transactions are read-only by default.
    pub read_only: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            retry: RetryPolicy::default(),
            region: None,
            follower_reads: false,
            staleness: Duration::ZERO,
            isolation: IsolationLevel::Serializable,
            read_only: false,
        }
    }
}

impl Config {
    /// Starts a fluent builder seeded with defaults.
    pub fn builder() -> ConfigBuilder {
        ConfigBuilder(Config::default())
    }
}

/// Fluent builder for [`Config`].
#[derive(Debug, Clone)]
pub struct ConfigBuilder(Config);

impl ConfigBuilder {
    /// Maximum retries after the first attempt.
    pub fn max_retries(mut self, n: u32) -> Self {
        self.0.retry.max_retries = n;
        self
    }
    /// Delay before the first retry.
    pub fn initial_delay(mut self, d: Duration) -> Self {
        self.0.retry.initial_delay = d;
        self
    }
    /// Upper bound on any single retry delay.
    pub fn max_delay(mut self, d: Duration) -> Self {
        self.0.retry.max_delay = d;
        self
    }
    /// Backoff growth factor.
    pub fn multiplier(mut self, m: f64) -> Self {
        self.0.retry.multiplier = m;
        self
    }
    /// Enables or disables backoff jitter.
    pub fn jitter(mut self, on: bool) -> Self {
        self.0.retry.jitter = on;
        self
    }
    /// Replaces the whole retry policy.
    pub fn retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.0.retry = policy;
        self
    }
    /// Sets the home region for REGIONAL BY ROW writes.
    pub fn region(mut self, region: impl Into<String>) -> Self {
        self.0.region = Some(region.into());
        self
    }
    /// Enables follower reads with the given default staleness
    /// (`Duration::ZERO` for `follower_read_timestamp()`).
    pub fn follower_reads(mut self, staleness: Duration) -> Self {
        self.0.follower_reads = true;
        self.0.staleness = staleness;
        self
    }
    /// Sets the default isolation level.
    pub fn isolation(mut self, level: IsolationLevel) -> Self {
        self.0.isolation = level;
        self
    }
    /// Sets the default read-only mode.
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.0.read_only = read_only;
        self
    }
    /// Finalises the configuration.
    pub fn build(self) -> Config {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let c = Config::default();
        assert_eq!(c.retry.max_retries, 5);
        assert!(c.region.is_none());
        assert!(!c.follower_reads);
        assert_eq!(c.isolation, IsolationLevel::Serializable);
        assert!(!c.read_only);
    }

    #[test]
    fn builder_overrides() {
        let c = Config::builder()
            .max_retries(10)
            .region("us-east-1")
            .follower_reads(Duration::from_secs(5))
            .isolation(IsolationLevel::ReadCommitted)
            .read_only(true)
            .build();
        assert_eq!(c.retry.max_retries, 10);
        assert_eq!(c.region.as_deref(), Some("us-east-1"));
        assert!(c.follower_reads);
        assert_eq!(c.staleness, Duration::from_secs(5));
        assert_eq!(c.isolation, IsolationLevel::ReadCommitted);
        assert!(c.read_only);
    }
}
