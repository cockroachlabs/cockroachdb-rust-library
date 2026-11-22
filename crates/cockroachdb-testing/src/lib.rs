//! Test harness for CockroachDB client code — no live cluster required.
//!
//! Mirrors the Go library's `crdbtest` package:
//!
//! * [`FakeDbError`] — an error carrying a chosen SQLSTATE, implementing
//!   [`cockroachdb_core::SqlState`] so you can test retry classification.
//! * [`RetrySimulator`] — fails with a retryable error a set number of times,
//!   then succeeds, to exercise retry loops deterministically.
//! * [`ConcurrencyRunner`] — runs an async operation across many concurrent
//!   workers to stress-test serialization behaviour.

mod concurrency;
mod errors;
mod retry_sim;

pub use concurrency::{ConcurrencyRunner, RunReport};
pub use errors::FakeDbError;
pub use retry_sim::RetrySimulator;
