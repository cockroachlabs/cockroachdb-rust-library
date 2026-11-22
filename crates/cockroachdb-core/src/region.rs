//! Multi-region row homing for REGIONAL BY ROW tables.
//!
//! CockroachDB homes a row to a region via its hidden `crdb_region` **column**.
//! Homing is done by writing a value to that column on insert — **not** by a
//! `SET crdb_region` session statement, which does not exist (a mistake that bit
//! several sibling libraries). These helpers only ever produce column values.

use async_trait::async_trait;

/// The hidden column CockroachDB uses for REGIONAL BY ROW homing.
pub const CRDB_REGION_COLUMN: &str = "crdb_region";

/// Carries the region a unit of work should home its rows to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegionContext {
    region: Option<String>,
}

impl RegionContext {
    /// Creates a context homed to `region`.
    pub fn new(region: impl Into<String>) -> Self {
        Self {
            region: Some(region.into()),
        }
    }

    /// An empty context (no explicit home region).
    pub fn empty() -> Self {
        Self::default()
    }

    /// The home region, if set.
    pub fn region(&self) -> Option<&str> {
        self.region.as_deref()
    }

    /// Builds the `(column, value)` pair to include in an `INSERT`/`UPSERT` for
    /// row homing, or `None` when no region is set.
    pub fn homing_column(&self) -> Option<(&'static str, &str)> {
        self.region().map(|r| (CRDB_REGION_COLUMN, r))
    }
}

/// Resolves the region for the current unit of work (e.g. from a request header,
/// tenant, or geo-IP lookup).
#[async_trait]
pub trait RegionResolver: Send + Sync {
    /// Returns the region to home rows to, or `None` to let the database default.
    async fn resolve(&self) -> Option<String>;
}

/// A [`RegionResolver`] that always returns a fixed region.
#[derive(Debug, Clone)]
pub struct StaticRegionResolver(pub String);

#[async_trait]
impl RegionResolver for StaticRegionResolver {
    async fn resolve(&self) -> Option<String> {
        Some(self.0.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn homing_column_pair() {
        let ctx = RegionContext::new("us-east-1");
        assert_eq!(ctx.region(), Some("us-east-1"));
        assert_eq!(ctx.homing_column(), Some(("crdb_region", "us-east-1")));
    }

    #[test]
    fn empty_has_no_homing() {
        assert_eq!(RegionContext::empty().homing_column(), None);
    }

    #[tokio::test]
    async fn static_resolver() {
        let r = StaticRegionResolver("eu-west-1".into());
        assert_eq!(r.resolve().await, Some("eu-west-1".to_string()));
    }
}
