//! Concurrency runner for stress-testing serialization behaviour.

use std::future::Future;
use std::sync::Arc;

/// Aggregate outcome of a [`ConcurrencyRunner::run`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RunReport {
    /// Total operations attempted (`concurrency * iterations`).
    pub total: usize,
    /// Operations that returned `Ok`.
    pub successes: usize,
    /// Operations that returned `Err`.
    pub failures: usize,
}

/// Runs an async operation across many concurrent workers.
///
/// Spawns `concurrency` workers, each performing `iterations` operations, and
/// tallies successes and failures. Useful for provoking serialization conflicts
/// against a real cluster, or for exercising retry logic against a simulator.
#[derive(Debug, Clone, Copy)]
pub struct ConcurrencyRunner {
    /// Number of concurrent workers.
    pub concurrency: usize,
    /// Operations per worker.
    pub iterations: usize,
}

impl ConcurrencyRunner {
    /// A runner with `concurrency` workers each doing `iterations` operations.
    pub fn new(concurrency: usize, iterations: usize) -> Self {
        Self {
            concurrency,
            iterations,
        }
    }

    /// Runs `op` (receiving a globally-unique 0-based index per call) across all
    /// workers and returns the aggregate report.
    pub async fn run<F, Fut, E>(&self, op: F) -> RunReport
    where
        F: Fn(usize) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), E>> + Send + 'static,
        E: Send + 'static,
    {
        let op = Arc::new(op);
        let mut handles = Vec::with_capacity(self.concurrency);
        for worker in 0..self.concurrency {
            let op = op.clone();
            let iterations = self.iterations;
            handles.push(tokio::spawn(async move {
                let mut successes = 0usize;
                let mut failures = 0usize;
                for i in 0..iterations {
                    match op(worker * iterations + i).await {
                        Ok(()) => successes += 1,
                        Err(_) => failures += 1,
                    }
                }
                (successes, failures)
            }));
        }

        let mut report = RunReport::default();
        for handle in handles {
            if let Ok((s, f)) = handle.await {
                report.successes += s;
                report.failures += f;
            }
        }
        report.total = report.successes + report.failures;
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn all_success() {
        let runner = ConcurrencyRunner::new(4, 10);
        let report = runner.run(|_i| async { Ok::<(), ()>(()) }).await;
        assert_eq!(report.total, 40);
        assert_eq!(report.successes, 40);
        assert_eq!(report.failures, 0);
    }

    #[tokio::test]
    async fn counts_failures() {
        // Even indices succeed, odd fail.
        let runner = ConcurrencyRunner::new(2, 5);
        let report = runner
            .run(|i| async move {
                if i % 2 == 0 {
                    Ok::<(), ()>(())
                } else {
                    Err(())
                }
            })
            .await;
        assert_eq!(report.total, 10);
        assert_eq!(report.successes + report.failures, 10);
        assert_eq!(report.successes, 5);
        assert_eq!(report.failures, 5);
    }

    #[tokio::test]
    async fn every_index_is_unique() {
        let seen = Arc::new(AtomicUsize::new(0));
        let s = seen.clone();
        let runner = ConcurrencyRunner::new(3, 4);
        runner
            .run(move |i| {
                let s = s.clone();
                async move {
                    // XOR-accumulate indices; distinct indices 0..12.
                    s.fetch_xor(i + 1, Ordering::SeqCst);
                    Ok::<(), ()>(())
                }
            })
            .await;
        // XOR of 1..=12
        let expected = (1..=12).fold(0usize, |a, b| a ^ b);
        assert_eq!(seen.load(Ordering::SeqCst), expected);
    }
}
