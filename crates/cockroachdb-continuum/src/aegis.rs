//! Aegis: security findings, recommendations, and app-event emission.
//!
//! Defaults to an offline simulated client; a networked client is gated behind a
//! future `http` feature. When an endpoint is configured but HTTP is not
//! compiled in, the returned client reports a clear error rather than silently
//! running simulated.

use crate::BoxError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

/// A security finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    /// Stable finding id.
    pub id: String,
    /// Severity (e.g. `low`, `medium`, `high`, `critical`).
    pub severity: String,
    /// Short title.
    pub title: String,
    /// Human-readable detail.
    pub detail: String,
}

/// A remediation recommendation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recommendation {
    /// Stable recommendation id.
    pub id: String,
    /// Short title.
    pub title: String,
    /// Suggested action.
    pub action: String,
}

/// An application event reported to Aegis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppEvent {
    /// Event kind (e.g. `retry_exhausted`, `circuit_open`).
    pub kind: String,
    /// Event detail.
    pub detail: String,
}

/// Aegis configuration. No endpoint ⇒ simulated client.
#[derive(Debug, Clone, Default)]
pub struct AegisConfig {
    /// HTTP endpoint. When `None`, the simulated client is used.
    pub endpoint: Option<String>,
    /// API key for the endpoint.
    pub api_key: Option<String>,
}

impl AegisConfig {
    fn use_simulated(&self) -> bool {
        self.endpoint.is_none()
    }
}

/// The Aegis client surface.
#[async_trait]
pub trait Aegis: Send + Sync {
    /// Current security findings.
    async fn findings(&self) -> Result<Vec<Finding>, BoxError>;
    /// Current recommendations.
    async fn recommendations(&self) -> Result<Vec<Recommendation>, BoxError>;
    /// Reports an application event.
    async fn emit(&self, event: AppEvent) -> Result<(), BoxError>;
}

/// Offline Aegis client with canned data; records emitted events.
#[derive(Debug, Default)]
pub struct Simulated {
    emitted: Mutex<Vec<AppEvent>>,
}

impl Simulated {
    /// Events emitted so far (for tests/inspection).
    pub fn emitted(&self) -> Vec<AppEvent> {
        self.emitted.lock().unwrap().clone()
    }
}

#[async_trait]
impl Aegis for Simulated {
    async fn findings(&self) -> Result<Vec<Finding>, BoxError> {
        Ok(vec![Finding {
            id: "AEGIS-001".into(),
            severity: "medium".into(),
            title: "Insecure connection".into(),
            detail: "A connection is using sslmode=disable.".into(),
        }])
    }

    async fn recommendations(&self) -> Result<Vec<Recommendation>, BoxError> {
        Ok(vec![Recommendation {
            id: "REC-001".into(),
            title: "Require TLS".into(),
            action: "Set sslmode=verify-full on all client connections.".into(),
        }])
    }

    async fn emit(&self, event: AppEvent) -> Result<(), BoxError> {
        self.emitted.lock().unwrap().push(event);
        Ok(())
    }
}

/// Client returned when an endpoint is set but the `http` feature is off.
struct NotEnabled;

#[async_trait]
impl Aegis for NotEnabled {
    async fn findings(&self) -> Result<Vec<Finding>, BoxError> {
        Err(http_disabled())
    }
    async fn recommendations(&self) -> Result<Vec<Recommendation>, BoxError> {
        Err(http_disabled())
    }
    async fn emit(&self, _event: AppEvent) -> Result<(), BoxError> {
        Err(http_disabled())
    }
}

fn http_disabled() -> BoxError {
    "continuum HTTP transport is not enabled in this build".into()
}

/// Builds an Aegis client: simulated when no endpoint is configured.
pub fn new(config: AegisConfig) -> Box<dyn Aegis> {
    if config.use_simulated() {
        Box::new(Simulated::default())
    } else {
        Box::new(NotEnabled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn simulated_returns_findings_and_records_events() {
        let client = new(AegisConfig::default());
        assert!(!client.findings().await.unwrap().is_empty());
        assert!(!client.recommendations().await.unwrap().is_empty());
        client
            .emit(AppEvent {
                kind: "circuit_open".into(),
                detail: "payments".into(),
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn endpoint_without_http_errors() {
        let client = new(AegisConfig {
            endpoint: Some("https://aegis.example".into()),
            api_key: Some("k".into()),
        });
        assert!(client.findings().await.is_err());
    }
}
