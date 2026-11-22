//! Driver-agnostic core for the CockroachDB Rust library.
//!
//! This crate mirrors the Go library's `crdbgo` package: it has no SQL-driver
//! dependency and defines the types, traits, and pure logic that the SQLx and
//! SeaORM adapters build on — retry policy, SQLSTATE classification, isolation
//! levels, follower-read `AS OF SYSTEM TIME` builders, multi-region row homing,
//! a circuit breaker, composable middleware, and metrics/health traits.
//!
//! See [`cockroachdb-sqlx`](https://docs.rs/cockroachdb-sqlx) for the primary
//! adapter that wires these into a connection pool.

pub mod circuit_breaker;
pub mod config;
pub mod ddl;
pub mod error;
pub mod follower;
pub mod health;
pub mod isolation;
pub mod metrics;
pub mod middleware;
pub mod region;
pub mod retry;

pub use circuit_breaker::{CbError, CircuitBreaker, CircuitBreakerOpenError, State};
pub use config::{Config, ConfigBuilder};
pub use error::{
    is_retryable, is_retryable_code, is_serialization_failure, retryable_in_chain, SqlState,
    SQLSTATE_DEADLOCK, SQLSTATE_SERIALIZATION_FAILURE,
};
pub use follower::{
    as_of_system_time, follower_read_clause, follower_read_timestamp, follower_read_with_staleness,
    FollowerError,
};
pub use health::{Health, HealthChecker, HealthStatus};
pub use isolation::IsolationLevel;
pub use metrics::{InMemoryRecorder, MetricsRecorder, Snapshot, Snapshotter};
pub use region::{RegionContext, RegionResolver, StaticRegionResolver, CRDB_REGION_COLUMN};
pub use retry::RetryPolicy;
