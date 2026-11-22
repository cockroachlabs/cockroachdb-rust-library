//! MCP (Model Context Protocol) client.
//!
//! Lets Continuum tooling expose/consume MCP tools. Defaults to a simulated
//! client; networked JSON/SSE transport is reserved for a future `http` feature.

use crate::BoxError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// An MCP tool descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tool {
    /// Tool name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
}

/// A piece of MCP result content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Content {
    /// Text content.
    pub text: String,
}

/// The result of calling an MCP tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallResult {
    /// Returned content items.
    pub content: Vec<Content>,
}

impl CallResult {
    /// Concatenates all content text.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join("")
    }
}

/// MCP configuration. No endpoint ⇒ simulated client.
#[derive(Debug, Clone, Default)]
pub struct McpConfig {
    /// Server endpoint. When `None`, the simulated client is used.
    pub endpoint: Option<String>,
}

impl McpConfig {
    fn use_simulated(&self) -> bool {
        self.endpoint.is_none()
    }
}

/// The MCP client surface.
#[async_trait]
pub trait Client: Send + Sync {
    /// Lists the available tools.
    async fn list_tools(&self) -> Result<Vec<Tool>, BoxError>;
    /// Calls a tool by name with JSON `args`.
    async fn call_tool(&self, name: &str, args: serde_json::Value) -> Result<CallResult, BoxError>;
}

/// Offline MCP client exposing one echo tool.
#[derive(Debug, Default)]
pub struct Simulated;

#[async_trait]
impl Client for Simulated {
    async fn list_tools(&self) -> Result<Vec<Tool>, BoxError> {
        Ok(vec![Tool {
            name: "echo".into(),
            description: "Echoes its arguments back as text.".into(),
        }])
    }

    async fn call_tool(&self, name: &str, args: serde_json::Value) -> Result<CallResult, BoxError> {
        if name != "echo" {
            return Err(format!("unknown tool: {name}").into());
        }
        Ok(CallResult {
            content: vec![Content {
                text: args.to_string(),
            }],
        })
    }
}

struct NotEnabled;

#[async_trait]
impl Client for NotEnabled {
    async fn list_tools(&self) -> Result<Vec<Tool>, BoxError> {
        Err(http_disabled())
    }
    async fn call_tool(
        &self,
        _name: &str,
        _args: serde_json::Value,
    ) -> Result<CallResult, BoxError> {
        Err(http_disabled())
    }
}

fn http_disabled() -> BoxError {
    "continuum HTTP transport is not enabled in this build".into()
}

/// Builds an MCP client: simulated when no endpoint is configured.
pub fn new(config: McpConfig) -> Box<dyn Client> {
    if config.use_simulated() {
        Box::new(Simulated)
    } else {
        Box::new(NotEnabled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn simulated_lists_and_calls() {
        let client = new(McpConfig::default());
        let tools = client.list_tools().await.unwrap();
        assert_eq!(tools[0].name, "echo");

        let result = client
            .call_tool("echo", serde_json::json!({"hi": 1}))
            .await
            .unwrap();
        assert!(result.text().contains("\"hi\""));

        assert!(client
            .call_tool("nope", serde_json::json!({}))
            .await
            .is_err());
    }
}
