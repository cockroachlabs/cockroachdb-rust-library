//! Metrics traits and an in-memory recorder.
//!
//! Mirrors the Go library's `MetricsRecorder` interface and `Snapshot`. The SQLx
//! adapter records transaction outcomes, retries, and follower reads here; the
//! [`InMemoryRecorder`] is ideal for tests. An OpenTelemetry recorder can be
//! added as a separate type implementing [`MetricsRecorder`].

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Records CockroachDB operation metrics.
pub trait MetricsRecorder: Send + Sync {
    /// Records a completed transaction: total `duration`, number of `retries`,
    /// and whether it ultimately succeeded.
    fn record_transaction(&self, duration: Duration, retries: u32, success: bool);
    /// Records a single retry attempt and its backoff `delay`.
    fn record_retry(&self, delay: Duration);
    /// Records a follower-read query and its `duration`.
    fn record_follower_read(&self, duration: Duration);
}

/// A point-in-time view of recorded metrics.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Total transactions recorded.
    pub transaction_count: u64,
    /// Transactions that succeeded.
    pub transaction_success_count: u64,
    /// Transactions that failed.
    pub transaction_failure_count: u64,
    /// Total retry attempts across all transactions.
    pub retry_count: u64,
    /// Total follower reads recorded.
    pub follower_read_count: u64,
    /// Summed transaction durations.
    pub total_transaction_duration: Duration,
    /// Summed retry backoff delays.
    pub total_retry_delay: Duration,
    /// Summed follower-read durations.
    pub total_follower_read_duration: Duration,
}

/// A recorder that can produce a [`Snapshot`].
pub trait Snapshotter {
    /// Returns a consistent-enough snapshot of current counters.
    fn snapshot(&self) -> Snapshot;
}

/// A lock-free in-memory [`MetricsRecorder`] backed by atomics.
#[derive(Debug, Default)]
pub struct InMemoryRecorder {
    transaction_count: AtomicU64,
    success_count: AtomicU64,
    failure_count: AtomicU64,
    retry_count: AtomicU64,
    follower_read_count: AtomicU64,
    total_tx_nanos: AtomicU64,
    total_retry_nanos: AtomicU64,
    total_follower_nanos: AtomicU64,
}

impl InMemoryRecorder {
    /// Creates an empty recorder.
    pub fn new() -> Self {
        Self::default()
    }
}

impl MetricsRecorder for InMemoryRecorder {
    fn record_transaction(&self, duration: Duration, retries: u32, success: bool) {
        self.transaction_count.fetch_add(1, Ordering::Relaxed);
        if success {
            self.success_count.fetch_add(1, Ordering::Relaxed);
        } else {
            self.failure_count.fetch_add(1, Ordering::Relaxed);
        }
        self.retry_count
            .fetch_add(retries as u64, Ordering::Relaxed);
        self.total_tx_nanos
            .fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
    }

    fn record_retry(&self, delay: Duration) {
        self.total_retry_nanos
            .fetch_add(delay.as_nanos() as u64, Ordering::Relaxed);
    }

    fn record_follower_read(&self, duration: Duration) {
        self.follower_read_count.fetch_add(1, Ordering::Relaxed);
        self.total_follower_nanos
            .fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
    }
}

impl Snapshotter for InMemoryRecorder {
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            transaction_count: self.transaction_count.load(Ordering::Relaxed),
            transaction_success_count: self.success_count.load(Ordering::Relaxed),
            transaction_failure_count: self.failure_count.load(Ordering::Relaxed),
            retry_count: self.retry_count.load(Ordering::Relaxed),
            follower_read_count: self.follower_read_count.load(Ordering::Relaxed),
            total_transaction_duration: Duration::from_nanos(
                self.total_tx_nanos.load(Ordering::Relaxed),
            ),
            total_retry_delay: Duration::from_nanos(self.total_retry_nanos.load(Ordering::Relaxed)),
            total_follower_read_duration: Duration::from_nanos(
                self.total_follower_nanos.load(Ordering::Relaxed),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_snapshots() {
        let r = InMemoryRecorder::new();
        r.record_transaction(Duration::from_millis(10), 2, true);
        r.record_transaction(Duration::from_millis(5), 0, false);
        r.record_retry(Duration::from_millis(50));
        r.record_follower_read(Duration::from_millis(3));

        let s = r.snapshot();
        assert_eq!(s.transaction_count, 2);
        assert_eq!(s.transaction_success_count, 1);
        assert_eq!(s.transaction_failure_count, 1);
        assert_eq!(s.retry_count, 2);
        assert_eq!(s.follower_read_count, 1);
        assert_eq!(s.total_transaction_duration, Duration::from_millis(15));
        assert_eq!(s.total_retry_delay, Duration::from_millis(50));
        assert_eq!(s.total_follower_read_duration, Duration::from_millis(3));
    }
}
