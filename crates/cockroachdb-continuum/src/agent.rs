//! An ops-tool registry with destructive-action gating.
//!
//! Lets an operator/agent register named tools (health checks, storage summaries,
//! maintenance actions). Tools marked `destructive` are refused by [`Registry::invoke`]
//! and only run via [`Registry::invoke_allowing_destructive`].

use crate::BoxError;
use std::collections::HashMap;

/// A registered tool implementation.
pub type ToolFunc =
    Box<dyn Fn(serde_json::Value) -> Result<serde_json::Value, BoxError> + Send + Sync>;

/// Metadata describing a registered tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDescriptor {
    /// Unique tool name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Whether invoking the tool mutates state.
    pub destructive: bool,
}

/// A registry of ops tools.
#[derive(Default)]
pub struct Registry {
    tools: HashMap<String, (ToolDescriptor, ToolFunc)>,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("tools", &self.tools.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Registry {
    /// A new, empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a tool (chainable).
    pub fn register(
        mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        destructive: bool,
        func: ToolFunc,
    ) -> Self {
        let name = name.into();
        let descriptor = ToolDescriptor {
            name: name.clone(),
            description: description.into(),
            destructive,
        };
        self.tools.insert(name, (descriptor, func));
        self
    }

    /// Lists registered tools, sorted by name.
    pub fn list(&self) -> Vec<ToolDescriptor> {
        let mut out: Vec<ToolDescriptor> = self.tools.values().map(|(d, _)| d.clone()).collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Invokes a non-destructive tool. Destructive tools are refused.
    pub fn invoke(
        &self,
        name: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, BoxError> {
        let (descriptor, func) = self
            .tools
            .get(name)
            .ok_or_else(|| -> BoxError { format!("unknown tool: {name}").into() })?;
        if descriptor.destructive {
            return Err(
                format!("tool `{name}` is destructive; use invoke_allowing_destructive").into(),
            );
        }
        func(args)
    }

    /// Invokes any tool, including destructive ones.
    pub fn invoke_allowing_destructive(
        &self,
        name: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, BoxError> {
        let (_, func) = self
            .tools
            .get(name)
            .ok_or_else(|| -> BoxError { format!("unknown tool: {name}").into() })?;
        func(args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Registry {
        Registry::new()
            .register(
                "health",
                "Check cluster health",
                false,
                Box::new(|_| Ok(serde_json::json!({"status": "up"}))),
            )
            .register(
                "drop_table",
                "Drops a table",
                true,
                Box::new(|_| Ok(serde_json::json!({"dropped": true}))),
            )
    }

    #[test]
    fn lists_sorted() {
        let names: Vec<String> = registry().list().into_iter().map(|d| d.name).collect();
        assert_eq!(names, vec!["drop_table", "health"]);
    }

    #[test]
    fn invokes_non_destructive() {
        let r = registry();
        let out = r.invoke("health", serde_json::json!({})).unwrap();
        assert_eq!(out["status"], "up");
    }

    #[test]
    fn refuses_destructive_via_invoke() {
        let r = registry();
        assert!(r.invoke("drop_table", serde_json::json!({})).is_err());
        // But the gated entry point runs it.
        let out = r
            .invoke_allowing_destructive("drop_table", serde_json::json!({}))
            .unwrap();
        assert_eq!(out["dropped"], true);
    }

    #[test]
    fn unknown_tool_errors() {
        assert!(registry().invoke("nope", serde_json::json!({})).is_err());
    }
}
