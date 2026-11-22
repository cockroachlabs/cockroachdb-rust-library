//! CockroachDB Cloud API client: list clusters and export metrics.
//!
//! Defaults to a simulated client; networked transport is reserved for a future
//! `http` feature. Selection is keyed on the API key, mirroring the Go library.

use crate::BoxError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// A CockroachDB Cloud cluster.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cluster {
    /// Cluster id.
    pub id: String,
    /// Cluster name.
    pub name: String,
    /// Cluster state (e.g. `CREATED`).
    pub state: String,
}

/// Cloud configuration. No API key ⇒ simulated client.
#[derive(Debug, Clone, Default)]
pub struct CloudConfig {
    /// API key. When `None`, the simulated client is used.
    pub api_key: Option<String>,
    /// Optional API endpoint override.
    pub endpoint: Option<String>,
}

impl CloudConfig {
    fn use_simulated(&self) -> bool {
        self.api_key.is_none()
    }
}

/// The Cloud client surface.
#[async_trait]
pub trait Cloud: Send + Sync {
    /// Lists clusters.
    async fn list_clusters(&self) -> Result<Vec<Cluster>, BoxError>;
    /// Fetches one cluster by id.
    async fn get_cluster(&self, id: &str) -> Result<Cluster, BoxError>;
    /// Exports a single metric sample to Cloud.
    async fn export_metric(&self, name: &str, value: f64) -> Result<(), BoxError>;
}

/// Offline Cloud client with one canned cluster; records exported metrics.
#[derive(Debug, Default)]
pub struct Simulated {
    exported: std::sync::Mutex<Vec<(String, f64)>>,
}

impl Simulated {
    /// Metrics exported so far (for tests/inspection).
    pub fn exported(&self) -> Vec<(String, f64)> {
        self.exported.lock().unwrap().clone()
    }

    fn sample() -> Cluster {
        Cluster {
            id: "sim-cluster-1".into(),
            name: "simulated".into(),
            state: "CREATED".into(),
        }
    }
}

#[async_trait]
impl Cloud for Simulated {
    async fn list_clusters(&self) -> Result<Vec<Cluster>, BoxError> {
        Ok(vec![Self::sample()])
    }

    async fn get_cluster(&self, id: &str) -> Result<Cluster, BoxError> {
        let mut c = Self::sample();
        c.id = id.to_string();
        Ok(c)
    }

    async fn export_metric(&self, name: &str, value: f64) -> Result<(), BoxError> {
        self.exported
            .lock()
            .unwrap()
            .push((name.to_string(), value));
        Ok(())
    }
}

struct NotEnabled;

#[async_trait]
impl Cloud for NotEnabled {
    async fn list_clusters(&self) -> Result<Vec<Cluster>, BoxError> {
        Err(http_disabled())
    }
    async fn get_cluster(&self, _id: &str) -> Result<Cluster, BoxError> {
        Err(http_disabled())
    }
    async fn export_metric(&self, _name: &str, _value: f64) -> Result<(), BoxError> {
        Err(http_disabled())
    }
}

fn http_disabled() -> BoxError {
    "continuum HTTP transport is not enabled in this build".into()
}

/// Builds a Cloud client: simulated when no API key is configured.
pub fn new(config: CloudConfig) -> Box<dyn Cloud> {
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
    async fn simulated_lists_and_exports() {
        let client = new(CloudConfig::default());
        assert_eq!(client.list_clusters().await.unwrap().len(), 1);
        assert_eq!(client.get_cluster("abc").await.unwrap().id, "abc");
        client.export_metric("tx_count", 42.0).await.unwrap();
    }

    #[tokio::test]
    async fn api_key_without_http_errors() {
        let client = new(CloudConfig {
            api_key: Some("k".into()),
            endpoint: None,
        });
        assert!(client.list_clusters().await.is_err());
    }
}
