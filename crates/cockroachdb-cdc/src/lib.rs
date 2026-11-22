//! Change data capture (CDC) for CockroachDB.
//!
//! Three driver-agnostic pieces mirroring the Go library's `cdc` package:
//!
//! * [`ChangefeedSpecBuilder`] builds a `CREATE CHANGEFEED` statement — run the
//!   resulting SQL with your own pool (this crate stays decoupled from any driver).
//! * [`Envelope`] / [`Payload`] parse the JSON a CockroachDB webhook sink posts.
//! * [`WebhookSink`] validates optional bearer auth, parses the envelope, and
//!   dispatches each row to your [`EventHandler`], recording [`CdcMetrics`].

mod changefeed;
mod event;
mod handler;
mod metrics;

pub use changefeed::{ChangefeedSpec, ChangefeedSpecBuilder};
pub use event::{Envelope, Operation, Payload};
pub use handler::{EventHandler, Outcome, WebhookSink};
pub use metrics::{CdcMetrics, CdcSnapshot};

/// A boxed error returned by an [`EventHandler`].
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Errors from building changefeeds or handling webhook deliveries.
#[derive(Debug, thiserror::Error)]
pub enum CdcError {
    /// A changefeed spec was missing its target table.
    #[error("changefeed requires a table")]
    MissingTable,
    /// A changefeed spec was missing its sink URI.
    #[error("changefeed requires a sink (webhook target)")]
    MissingSink,
    /// A table or column identifier was empty.
    #[error("invalid identifier: {0:?}")]
    InvalidIdentifier(String),
    /// The webhook request failed bearer-token authentication.
    #[error("unauthorized webhook request")]
    Unauthorized,
    /// The webhook body could not be parsed as a changefeed envelope.
    #[error("invalid changefeed payload: {0}")]
    InvalidPayload(#[from] serde_json::Error),
    /// The registered event handler returned an error.
    #[error("event handler failed: {0}")]
    Handler(#[source] BoxError),
}
