//! Health-check trait and status.
//!
//! Mirrors the Go library's `HealthChecker` (a pool implements it by pinging the
//! cluster) and `HealthStatus` (`"up"` / `"down"`). This crate is framework-free:
//! the trait and status live here; an HTTP handler can be built on top in an
//! adapter (e.g. with `axum`).

use async_trait::async_trait;

/// A boxed error for health checks.
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Up/down health.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    /// The dependency is reachable.
    Up,
    /// The dependency is unreachable.
    Down,
}

impl Health {
    /// The lowercase string form (`"up"` / `"down"`), matching the Go library.
    pub fn as_str(&self) -> &'static str {
        match self {
            Health::Up => "up",
            Health::Down => "down",
        }
    }
}

impl std::fmt::Display for Health {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The outcome of a health check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthStatus {
    /// Up or down.
    pub status: Health,
    /// Optional detail (e.g. the error message when down).
    pub detail: Option<String>,
}

impl HealthStatus {
    /// An `up` status with no detail.
    pub fn up() -> Self {
        Self {
            status: Health::Up,
            detail: None,
        }
    }
    /// A `down` status carrying `detail`.
    pub fn down(detail: impl Into<String>) -> Self {
        Self {
            status: Health::Down,
            detail: Some(detail.into()),
        }
    }
}

/// A component that can report whether its dependency is reachable.
#[async_trait]
pub trait HealthChecker: Send + Sync {
    /// Returns `Ok(())` when healthy, or an error describing the failure.
    async fn check(&self) -> Result<(), BoxError>;
}

/// Runs a checker and maps the result to a [`HealthStatus`].
pub async fn health_status(checker: &dyn HealthChecker) -> HealthStatus {
    match checker.check().await {
        Ok(()) => HealthStatus::up(),
        Err(e) => HealthStatus::down(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Mock(bool);
    #[async_trait]
    impl HealthChecker for Mock {
        async fn check(&self) -> Result<(), BoxError> {
            if self.0 {
                Ok(())
            } else {
                Err("connection refused".into())
            }
        }
    }

    #[tokio::test]
    async fn up_and_down() {
        assert_eq!(health_status(&Mock(true)).await, HealthStatus::up());
        let down = health_status(&Mock(false)).await;
        assert_eq!(down.status, Health::Down);
        assert_eq!(down.status.as_str(), "down");
        assert!(down.detail.unwrap().contains("connection refused"));
    }
}
