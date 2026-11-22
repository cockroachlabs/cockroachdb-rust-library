//! A three-state circuit breaker (Closed / Open / HalfOpen).
//!
//! Ports the Go library's `circuitbreaker` package. A run of failures trips the
//! breaker Open; after `break_duration` it moves HalfOpen to probe recovery; one
//! success closes it again, one failure reopens it. It integrates with the
//! [`middleware`](crate::middleware) stack via [`CircuitBreaker::middleware`].

use crate::middleware::{BoxError, BoxFuture, Middleware, TxFn};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Breaker state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Requests pass through; failures are counted.
    Closed,
    /// Requests are rejected until `break_duration` elapses.
    Open,
    /// A single probe request is allowed through.
    HalfOpen,
}

impl std::fmt::Display for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            State::Closed => "closed",
            State::Open => "open",
            State::HalfOpen => "half-open",
        })
    }
}

/// Returned when the breaker is Open and rejects a call.
#[derive(Debug, Clone, thiserror::Error)]
#[error("circuit breaker is open, retry after {retry_after:?}")]
pub struct CircuitBreakerOpenError {
    /// Time until the breaker will allow a probe.
    pub retry_after: Duration,
}

/// Error from [`CircuitBreaker::execute`]: either the breaker was open, or the
/// wrapped operation itself failed.
#[derive(Debug, thiserror::Error)]
pub enum CbError<E> {
    /// The breaker rejected the call.
    #[error(transparent)]
    Open(#[from] CircuitBreakerOpenError),
    /// The wrapped operation returned an error.
    #[error(transparent)]
    Inner(E),
}

/// Callback fired on every state transition `(from, to)`.
pub type StateChangeFn = Arc<dyn Fn(State, State) + Send + Sync>;

#[derive(Debug)]
struct Inner {
    state: State,
    failure_count: u32,
    opened_at: Option<Instant>,
}

/// A three-state circuit breaker.
pub struct CircuitBreaker {
    failure_threshold: u32,
    break_duration: Duration,
    on_state_change: Option<StateChangeFn>,
    inner: Mutex<Inner>,
}

impl std::fmt::Debug for CircuitBreaker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CircuitBreaker")
            .field("failure_threshold", &self.failure_threshold)
            .field("break_duration", &self.break_duration)
            .field("inner", &self.inner)
            .finish_non_exhaustive()
    }
}

impl CircuitBreaker {
    /// Starts a builder with defaults (threshold 5, break 30 s).
    pub fn builder() -> CircuitBreakerBuilder {
        CircuitBreakerBuilder {
            failure_threshold: 5,
            break_duration: Duration::from_secs(30),
            on_state_change: None,
        }
    }

    /// The current state.
    pub fn state(&self) -> State {
        self.inner.lock().unwrap().state
    }

    /// The current consecutive-failure count.
    pub fn failure_count(&self) -> u32 {
        self.inner.lock().unwrap().failure_count
    }

    /// Forces the breaker Closed and clears the failure count.
    pub fn reset(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.failure_count = 0;
        inner.opened_at = None;
        self.set_state(&mut inner, State::Closed);
    }

    /// Runs `f` through the breaker.
    ///
    /// Rejects immediately with [`CbError::Open`] while Open; otherwise runs `f`
    /// and updates state from the outcome.
    pub async fn execute<F, Fut, T, E>(&self, f: F) -> Result<T, CbError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        // Pre-call: reject if open (unless it's time to probe).
        {
            let mut inner = self.inner.lock().unwrap();
            if inner.state == State::Open {
                let elapsed = inner.opened_at.map(|t| t.elapsed()).unwrap_or_default();
                if elapsed >= self.break_duration {
                    self.set_state(&mut inner, State::HalfOpen);
                } else {
                    return Err(CbError::Open(CircuitBreakerOpenError {
                        retry_after: self.break_duration - elapsed,
                    }));
                }
            }
        }

        let result = f().await; // lock released while running

        // Post-call: update state.
        let mut inner = self.inner.lock().unwrap();
        match &result {
            Ok(_) => {
                inner.failure_count = 0;
                if inner.state == State::HalfOpen {
                    inner.opened_at = None;
                    self.set_state(&mut inner, State::Closed);
                }
            }
            Err(_) => {
                inner.failure_count += 1;
                let trip = inner.state == State::HalfOpen
                    || (inner.state == State::Closed
                        && inner.failure_count >= self.failure_threshold);
                if trip {
                    inner.opened_at = Some(Instant::now());
                    self.set_state(&mut inner, State::Open);
                }
            }
        }
        result.map_err(CbError::Inner)
    }

    /// A [`Middleware`] that runs the wrapped work through this breaker.
    pub fn middleware(self: &Arc<Self>) -> Middleware {
        let cb = self.clone();
        Arc::new(move |next: TxFn| {
            let cb = cb.clone();
            Arc::new(move || {
                let cb = cb.clone();
                let next = next.clone();
                Box::pin(async move {
                    cb.execute(|| async move { next().await })
                        .await
                        .map_err(|e| -> BoxError {
                            match e {
                                CbError::Open(o) => Box::new(o),
                                CbError::Inner(i) => i,
                            }
                        })
                }) as BoxFuture<'static, Result<(), BoxError>>
            }) as TxFn
        })
    }

    fn set_state(&self, inner: &mut Inner, to: State) {
        let from = inner.state;
        if from == to {
            return;
        }
        inner.state = to;
        if let Some(cb) = &self.on_state_change {
            cb(from, to);
        }
    }
}

/// Builder for [`CircuitBreaker`].
pub struct CircuitBreakerBuilder {
    failure_threshold: u32,
    break_duration: Duration,
    on_state_change: Option<StateChangeFn>,
}

impl CircuitBreakerBuilder {
    /// Consecutive failures that trip the breaker Open.
    pub fn failure_threshold(mut self, n: u32) -> Self {
        if n > 0 {
            self.failure_threshold = n;
        }
        self
    }
    /// How long the breaker stays Open before probing.
    pub fn break_duration(mut self, d: Duration) -> Self {
        self.break_duration = d;
        self
    }
    /// Callback fired on every `(from, to)` transition.
    pub fn on_state_change(mut self, f: impl Fn(State, State) + Send + Sync + 'static) -> Self {
        self.on_state_change = Some(Arc::new(f));
        self
    }
    /// Builds the breaker.
    pub fn build(self) -> CircuitBreaker {
        CircuitBreaker {
            failure_threshold: self.failure_threshold,
            break_duration: self.break_duration,
            on_state_change: self.on_state_change,
            inner: Mutex::new(Inner {
                state: State::Closed,
                failure_count: 0,
                opened_at: None,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn ok() -> Result<(), &'static str> {
        Ok(())
    }
    fn fail() -> Result<(), &'static str> {
        Err("boom")
    }

    #[tokio::test]
    async fn success_stays_closed() {
        let cb = CircuitBreaker::builder().build();
        cb.execute(|| async { ok() }).await.unwrap();
        assert_eq!(cb.state(), State::Closed);
        assert_eq!(cb.failure_count(), 0);
    }

    #[tokio::test]
    async fn failures_reach_threshold_trips_open() {
        let cb = CircuitBreaker::builder().failure_threshold(3).build();
        for _ in 0..3 {
            let _ = cb.execute(|| async { fail() }).await;
        }
        assert_eq!(cb.state(), State::Open);
    }

    #[tokio::test]
    async fn success_resets_failure_count() {
        let cb = CircuitBreaker::builder().failure_threshold(3).build();
        let _ = cb.execute(|| async { fail() }).await;
        let _ = cb.execute(|| async { fail() }).await;
        cb.execute(|| async { ok() }).await.unwrap();
        assert_eq!(cb.failure_count(), 0);
        assert_eq!(cb.state(), State::Closed);
    }

    #[tokio::test]
    async fn open_blocks_execution() {
        let cb = CircuitBreaker::builder()
            .failure_threshold(1)
            .break_duration(Duration::from_secs(60))
            .build();
        let _ = cb.execute(|| async { fail() }).await;
        assert_eq!(cb.state(), State::Open);
        let err = cb.execute(|| async { ok() }).await.unwrap_err();
        assert!(matches!(err, CbError::Open(_)));
    }

    #[tokio::test]
    async fn open_transitions_to_half_open_then_closes() {
        let cb = CircuitBreaker::builder()
            .failure_threshold(1)
            .break_duration(Duration::from_millis(20))
            .build();
        let _ = cb.execute(|| async { fail() }).await;
        assert_eq!(cb.state(), State::Open);
        tokio::time::sleep(Duration::from_millis(30)).await;
        // Next call probes (HalfOpen) and, on success, closes.
        cb.execute(|| async { ok() }).await.unwrap();
        assert_eq!(cb.state(), State::Closed);
    }

    #[tokio::test]
    async fn half_open_failure_reopens() {
        let cb = CircuitBreaker::builder()
            .failure_threshold(1)
            .break_duration(Duration::from_millis(20))
            .build();
        let _ = cb.execute(|| async { fail() }).await;
        tokio::time::sleep(Duration::from_millis(30)).await;
        let _ = cb.execute(|| async { fail() }).await; // probe fails
        assert_eq!(cb.state(), State::Open);
    }

    #[tokio::test]
    async fn reset_closes() {
        let cb = CircuitBreaker::builder().failure_threshold(1).build();
        let _ = cb.execute(|| async { fail() }).await;
        assert_eq!(cb.state(), State::Open);
        cb.reset();
        assert_eq!(cb.state(), State::Closed);
        assert_eq!(cb.failure_count(), 0);
    }

    #[tokio::test]
    async fn tracks_state_changes() {
        let transitions = Arc::new(AtomicUsize::new(0));
        let t = transitions.clone();
        let cb = CircuitBreaker::builder()
            .failure_threshold(1)
            .on_state_change(move |_from, _to| {
                t.fetch_add(1, Ordering::SeqCst);
            })
            .build();
        let _ = cb.execute(|| async { fail() }).await; // Closed -> Open
        assert!(transitions.load(Ordering::SeqCst) >= 1);
    }

    #[tokio::test]
    async fn middleware_blocks_when_open() {
        let cb = Arc::new(
            CircuitBreaker::builder()
                .failure_threshold(1)
                .break_duration(Duration::from_secs(60))
                .build(),
        );
        let mw = cb.middleware();
        let failing: TxFn = Arc::new(|| Box::pin(async { Err("boom".into()) }));
        let _ = mw(failing.clone())().await; // trips open
        assert_eq!(cb.state(), State::Open);
        let err = mw(failing)().await;
        assert!(err.is_err());
    }
}
