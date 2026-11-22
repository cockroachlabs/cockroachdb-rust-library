//! Composable asynchronous middleware around a unit of work.
//!
//! Rust has no AOP, and its async closures don't compose as cleanly as Go's
//! `func(next TxFn) TxFn`. This module provides the minimal boxed-future
//! equivalent: a [`TxFn`] is a runnable unit of work and a [`Middleware`] wraps
//! one into another (for timing, logging, circuit breaking, ...). The circuit
//! breaker exposes a [`Middleware`] via this plumbing.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// A boxed, `Send` future.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A boxed error carried through the stack.
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// A runnable, re-runnable unit of work.
pub type TxFn = Arc<dyn Fn() -> BoxFuture<'static, Result<(), BoxError>> + Send + Sync>;

/// Wraps a [`TxFn`] into another [`TxFn`].
pub type Middleware = Arc<dyn Fn(TxFn) -> TxFn + Send + Sync>;

/// Composes middleware so the first element runs outermost:
/// `chain([a, b])(inner)` invokes `a`, then `b`, then `inner`.
pub fn chain(middlewares: Vec<Middleware>) -> Middleware {
    Arc::new(move |inner: TxFn| {
        let mut handler = inner;
        for mw in middlewares.iter().rev() {
            handler = mw(handler);
        }
        handler
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn empty_chain_runs_inner() {
        let hits = Arc::new(AtomicUsize::new(0));
        let h2 = hits.clone();
        let inner: TxFn = Arc::new(move || {
            let h = h2.clone();
            Box::pin(async move {
                h.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        });
        let composed = chain(vec![])(inner);
        composed().await.unwrap();
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn middleware_wraps_inner() {
        let wrapped = Arc::new(AtomicUsize::new(0));
        let inner_hits = Arc::new(AtomicUsize::new(0));

        let ih = inner_hits.clone();
        let inner: TxFn = Arc::new(move || {
            let ih = ih.clone();
            Box::pin(async move {
                ih.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        });

        let w = wrapped.clone();
        let mw: Middleware = Arc::new(move |next: TxFn| {
            let w = w.clone();
            Arc::new(move || {
                let w = w.clone();
                let next = next.clone();
                Box::pin(async move {
                    w.fetch_add(1, Ordering::SeqCst);
                    next().await
                }) as BoxFuture<'static, Result<(), BoxError>>
            }) as TxFn
        });

        let composed = chain(vec![mw])(inner);
        composed().await.unwrap();
        assert_eq!(wrapped.load(Ordering::SeqCst), 1);
        assert_eq!(inner_hits.load(Ordering::SeqCst), 1);
    }
}
