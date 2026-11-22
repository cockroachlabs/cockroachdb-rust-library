//! The webhook sink: auth check, envelope parse, per-row dispatch, metrics.

use crate::event::{Envelope, Payload};
use crate::metrics::CdcMetrics;
use crate::{BoxError, CdcError};
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Instant;

/// Handles a single changefeed row change.
#[async_trait]
pub trait EventHandler: Send + Sync {
    /// Processes one row change. Returning an error stops the batch and signals
    /// the sink to retry the delivery.
    async fn handle(&self, payload: &Payload) -> Result<(), BoxError>;
}

/// The result of handling one webhook delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcome {
    /// Number of row changes dispatched to the handler.
    pub processed: usize,
    /// Whether the delivery was a resolved-timestamp heartbeat.
    pub resolved: bool,
}

/// A CockroachDB webhook sink.
///
/// Framework-agnostic: feed it the raw request body (and the `Authorization`
/// header, if auth is configured) from whatever HTTP server you use.
pub struct WebhookSink<H: EventHandler> {
    handler: H,
    auth_token: Option<String>,
    metrics: Option<Arc<CdcMetrics>>,
}

impl<H: EventHandler> WebhookSink<H> {
    /// Creates a sink that dispatches to `handler`.
    pub fn new(handler: H) -> Self {
        Self {
            handler,
            auth_token: None,
            metrics: None,
        }
    }

    /// Requires `Authorization: Bearer <token>` on every delivery.
    pub fn with_auth_token(mut self, token: impl Into<String>) -> Self {
        self.auth_token = Some(token.into());
        self
    }

    /// Records delivery metrics.
    pub fn with_metrics(mut self, metrics: Arc<CdcMetrics>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    /// Validates auth, parses the envelope, and dispatches each row change.
    ///
    /// Pass the raw `Authorization` header value in `auth_header` (or `None`).
    pub async fn handle(
        &self,
        auth_header: Option<&str>,
        body: &[u8],
    ) -> Result<Outcome, CdcError> {
        if let Some(expected) = &self.auth_token {
            let provided = auth_header
                .and_then(|h| h.strip_prefix("Bearer "))
                .map(str::trim);
            if provided != Some(expected.as_str()) {
                if let Some(m) = &self.metrics {
                    m.record_auth_failure();
                }
                return Err(CdcError::Unauthorized);
            }
        }

        let env: Envelope = serde_json::from_slice(body)?;

        if env.is_resolved() {
            if let Some(m) = &self.metrics {
                m.record_resolved();
            }
            return Ok(Outcome {
                processed: 0,
                resolved: true,
            });
        }

        let mut processed = 0;
        for payload in &env.payload {
            let start = Instant::now();
            let result = self.handler.handle(payload).await;
            if let Some(m) = &self.metrics {
                m.record_event(start.elapsed(), result.is_err());
            }
            result.map_err(CdcError::Handler)?;
            processed += 1;
        }

        Ok(Outcome {
            processed,
            resolved: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Counting(Arc<AtomicUsize>);
    #[async_trait]
    impl EventHandler for Counting {
        async fn handle(&self, _payload: &Payload) -> Result<(), BoxError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct Failing;
    #[async_trait]
    impl EventHandler for Failing {
        async fn handle(&self, _payload: &Payload) -> Result<(), BoxError> {
            Err("nope".into())
        }
    }

    const INSERT_BATCH: &[u8] =
        br#"{"payload":[{"after":{"id":1}},{"after":{"id":2}}],"length":2}"#;

    #[tokio::test]
    async fn dispatches_each_row() {
        let seen = Arc::new(AtomicUsize::new(0));
        let metrics = Arc::new(CdcMetrics::new());
        let sink = WebhookSink::new(Counting(seen.clone())).with_metrics(metrics.clone());

        let outcome = sink.handle(None, INSERT_BATCH).await.unwrap();
        assert_eq!(outcome.processed, 2);
        assert!(!outcome.resolved);
        assert_eq!(seen.load(Ordering::SeqCst), 2);
        assert_eq!(metrics.snapshot().events, 2);
    }

    #[tokio::test]
    async fn resolved_is_not_dispatched() {
        let seen = Arc::new(AtomicUsize::new(0));
        let metrics = Arc::new(CdcMetrics::new());
        let sink = WebhookSink::new(Counting(seen.clone())).with_metrics(metrics.clone());

        let outcome = sink
            .handle(None, br#"{"resolved":"1700000000.0"}"#)
            .await
            .unwrap();
        assert!(outcome.resolved);
        assert_eq!(outcome.processed, 0);
        assert_eq!(seen.load(Ordering::SeqCst), 0);
        assert_eq!(metrics.snapshot().resolved, 1);
    }

    #[tokio::test]
    async fn rejects_bad_auth() {
        let seen = Arc::new(AtomicUsize::new(0));
        let metrics = Arc::new(CdcMetrics::new());
        let sink = WebhookSink::new(Counting(seen.clone()))
            .with_auth_token("s3cret")
            .with_metrics(metrics.clone());

        let err = sink.handle(Some("Bearer wrong"), INSERT_BATCH).await;
        assert!(matches!(err, Err(CdcError::Unauthorized)));
        assert_eq!(seen.load(Ordering::SeqCst), 0);
        assert_eq!(metrics.snapshot().auth_failures, 1);

        // Correct token passes.
        let ok = sink
            .handle(Some("Bearer s3cret"), INSERT_BATCH)
            .await
            .unwrap();
        assert_eq!(ok.processed, 2);
    }

    #[tokio::test]
    async fn handler_error_propagates_and_counts() {
        let metrics = Arc::new(CdcMetrics::new());
        let sink = WebhookSink::new(Failing).with_metrics(metrics.clone());
        let err = sink.handle(None, INSERT_BATCH).await;
        assert!(matches!(err, Err(CdcError::Handler(_))));
        assert_eq!(metrics.snapshot().errors, 1);
    }
}
