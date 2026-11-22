//! Metrics for the webhook sink.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// A point-in-time view of [`CdcMetrics`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CdcSnapshot {
    /// Row-change events dispatched to the handler.
    pub events: u64,
    /// Resolved-timestamp heartbeats received.
    pub resolved: u64,
    /// Handler invocations that returned an error.
    pub errors: u64,
    /// Requests rejected for failing authentication.
    pub auth_failures: u64,
    /// Summed handler latency.
    pub total_latency: Duration,
}

/// Lock-free counters for webhook deliveries.
#[derive(Debug, Default)]
pub struct CdcMetrics {
    events: AtomicU64,
    resolved: AtomicU64,
    errors: AtomicU64,
    auth_failures: AtomicU64,
    total_latency_nanos: AtomicU64,
}

impl CdcMetrics {
    /// Creates an empty metrics recorder.
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn record_event(&self, latency: Duration, is_error: bool) {
        self.events.fetch_add(1, Ordering::Relaxed);
        if is_error {
            self.errors.fetch_add(1, Ordering::Relaxed);
        }
        self.total_latency_nanos
            .fetch_add(latency.as_nanos() as u64, Ordering::Relaxed);
    }

    pub(crate) fn record_resolved(&self) {
        self.resolved.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_auth_failure(&self) {
        self.auth_failures.fetch_add(1, Ordering::Relaxed);
    }

    /// Returns a snapshot of the current counters.
    pub fn snapshot(&self) -> CdcSnapshot {
        CdcSnapshot {
            events: self.events.load(Ordering::Relaxed),
            resolved: self.resolved.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
            auth_failures: self.auth_failures.load(Ordering::Relaxed),
            total_latency: Duration::from_nanos(self.total_latency_nanos.load(Ordering::Relaxed)),
        }
    }
}
