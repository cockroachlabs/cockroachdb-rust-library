//! CockroachDB Continuum support surface.
//!
//! The managed-connectivity + observability layer shared by the client
//! libraries, mirroring the Go library's `continuum` package:
//!
//! * [`vcluster`] — virtual-cluster quality-of-service tagging for transactions.
//! * [`plenum`] — per-table storage summaries.
//! * [`aegis`] — security findings and recommendations.
//! * [`mcp`] — a Model Context Protocol client.
//! * [`cloud`] — a CockroachDB Cloud API client (clusters + metric export).
//! * [`agent`] — an ops-tool registry with destructive-action gating.
//!
//! The `aegis`, `mcp`, and `cloud` clients default to an offline **simulated**
//! implementation (selected when no endpoint/key is configured), matching the Go
//! library. A networked transport is reserved for a future `http` feature.

pub mod aegis;
pub mod agent;
pub mod cloud;
pub mod mcp;
pub mod plenum;
pub mod vcluster;

/// A boxed error used across Continuum clients.
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;
