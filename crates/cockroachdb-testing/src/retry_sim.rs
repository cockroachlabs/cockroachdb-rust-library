//! A deterministic retry simulator.

use crate::errors::FakeDbError;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Fails with a retryable error the first `fail_times` calls, then succeeds.
///
/// Use it to exercise a retry loop without contention or a live cluster:
///
/// ```
/// # use cockroachdb_testing::RetrySimulator;
/// # async fn demo() {
/// let sim = RetrySimulator::serialization_failures(2);
/// // First two calls fail with 40001, the third succeeds.
/// assert!(sim.run().await.is_err());
/// assert!(sim.run().await.is_err());
/// assert!(sim.run().await.is_ok());
/// assert_eq!(sim.calls(), 3);
/// # }
/// ```
#[derive(Debug)]
pub struct RetrySimulator {
    fail_times: usize,
    code: String,
    calls: AtomicUsize,
}

impl RetrySimulator {
    /// Fails `fail_times` times with `code`, then succeeds.
    pub fn new(fail_times: usize, code: impl Into<String>) -> Self {
        Self {
            fail_times,
            code: code.into(),
            calls: AtomicUsize::new(0),
        }
    }

    /// Fails `fail_times` times with `SQLSTATE 40001`, then succeeds.
    pub fn serialization_failures(fail_times: usize) -> Self {
        Self::new(fail_times, cockroachdb_core::SQLSTATE_SERIALIZATION_FAILURE)
    }

    /// Number of times [`RetrySimulator::run`] has been called.
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// Advances the simulator: an error for the first `fail_times` calls, then `Ok`.
    pub async fn run(&self) -> Result<(), FakeDbError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        if n < self.fail_times {
            Err(FakeDbError::with_code(&self.code))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fails_then_succeeds() {
        let sim = RetrySimulator::serialization_failures(2);
        assert!(sim.run().await.is_err());
        assert!(sim.run().await.is_err());
        assert!(sim.run().await.is_ok());
        assert_eq!(sim.calls(), 3);
    }

    #[tokio::test]
    async fn drives_a_retry_loop_to_success() {
        let sim = RetrySimulator::serialization_failures(3);
        let mut attempts = 0;
        loop {
            attempts += 1;
            match sim.run().await {
                Ok(()) => break,
                Err(e) if cockroachdb_core::is_retryable(&e) && attempts < 10 => continue,
                Err(e) => panic!("unexpected error: {e}"),
            }
        }
        assert_eq!(attempts, 4); // 3 failures + 1 success
    }
}
