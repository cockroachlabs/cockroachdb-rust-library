//! Parsing of the JSON a CockroachDB webhook sink posts.

use serde::Deserialize;

/// The `wrapped` webhook envelope: a batch of row changes and/or a resolved
/// timestamp marker.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Envelope {
    /// Row changes in this delivery.
    #[serde(default)]
    pub payload: Vec<Payload>,
    /// Number of changes, when present.
    pub length: Option<i64>,
    /// A resolved-timestamp marker (no row changes accompany it).
    pub resolved: Option<String>,
}

impl Envelope {
    /// `true` when this is a resolved-timestamp heartbeat (no row changes).
    pub fn is_resolved(&self) -> bool {
        self.resolved.is_some() && self.payload.is_empty()
    }
}

/// A single row change.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Payload {
    /// The row image after the change; `null`/absent for a delete.
    pub after: Option<serde_json::Value>,
    /// The row image before the change (only with `WITH diff`).
    pub before: Option<serde_json::Value>,
    /// The primary key of the changed row.
    pub key: Option<serde_json::Value>,
    /// The source topic/table.
    pub topic: Option<String>,
    /// The MVCC timestamp (only with `WITH updated`).
    pub updated: Option<String>,
}

/// The kind of change a [`Payload`] represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// A new row.
    Insert,
    /// An existing row changed.
    Update,
    /// A row was removed.
    Delete,
}

impl Payload {
    /// Infers the operation from the row images.
    ///
    /// A missing/`null` `after` is a delete. Otherwise, a present `before`
    /// (available only with `WITH diff`) means update; its absence means insert.
    pub fn operation(&self) -> Operation {
        let after_present = matches!(&self.after, Some(v) if !v.is_null());
        if !after_present {
            Operation::Delete
        } else if matches!(&self.before, Some(v) if !v.is_null()) {
            Operation::Update
        } else {
            Operation::Insert
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wrapped_insert() {
        let json = r#"{"payload":[{"after":{"id":1,"balance":100},"key":[1],"topic":"accounts"}],"length":1}"#;
        let env: Envelope = serde_json::from_str(json).unwrap();
        assert!(!env.is_resolved());
        assert_eq!(env.payload.len(), 1);
        assert_eq!(env.payload[0].operation(), Operation::Insert);
        assert_eq!(env.payload[0].topic.as_deref(), Some("accounts"));
    }

    #[test]
    fn resolved_marker() {
        let json = r#"{"resolved":"1700000000.0000000000"}"#;
        let env: Envelope = serde_json::from_str(json).unwrap();
        assert!(env.is_resolved());
        assert!(env.payload.is_empty());
    }

    #[test]
    fn operation_resolution() {
        let insert = Payload {
            after: Some(serde_json::json!({"id": 1})),
            before: None,
            ..Default::default()
        };
        assert_eq!(insert.operation(), Operation::Insert);

        let update = Payload {
            after: Some(serde_json::json!({"id": 1})),
            before: Some(serde_json::json!({"id": 1})),
            ..Default::default()
        };
        assert_eq!(update.operation(), Operation::Update);

        let delete = Payload {
            after: Some(serde_json::Value::Null),
            before: Some(serde_json::json!({"id": 1})),
            ..Default::default()
        };
        assert_eq!(delete.operation(), Operation::Delete);

        let delete_absent = Payload {
            after: None,
            ..Default::default()
        };
        assert_eq!(delete_absent.operation(), Operation::Delete);
    }
}
