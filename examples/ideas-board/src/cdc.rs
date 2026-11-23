//! CDC track: a changefeed handler that projects row changes into `Idea`s, and
//! the webhook sink that dispatches CockroachDB deliveries to it.

// tag::cdc-handler[]
use crate::idea::Idea;
use async_trait::async_trait;
use cockroachdb_cdc::{BoxError, EventHandler, Operation, Payload};
use std::sync::Mutex;

/// Projects changefeed row changes into `Idea`s. Inserts and updates are kept as
/// the new row image; deletes are counted. The library's `EventHandler` takes
/// `&self` and must be `Send + Sync`, so state lives behind a `Mutex` — the
/// handler is shared across concurrent webhook deliveries.
#[derive(Default)]
pub struct IdeaChangefeedHandler {
    handled: Mutex<Vec<Idea>>,
    deletes: Mutex<usize>,
}

#[async_trait]
impl EventHandler for IdeaChangefeedHandler {
    async fn handle(&self, payload: &Payload) -> Result<(), BoxError> {
        match payload.operation() {
            Operation::Delete => *self.deletes.lock().unwrap() += 1,
            Operation::Insert | Operation::Update => {
                self.handled.lock().unwrap().push(idea_from_after(payload)?);
            }
        }
        Ok(())
    }
}

impl IdeaChangefeedHandler {
    /// Ideas projected from insert/update events, in arrival order.
    pub fn handled(&self) -> Vec<Idea> {
        self.handled.lock().unwrap().clone()
    }

    /// How many delete events were seen.
    pub fn deletes(&self) -> usize {
        *self.deletes.lock().unwrap()
    }
}

/// Builds an `Idea` from a payload's `after` row image.
fn idea_from_after(payload: &Payload) -> Result<Idea, BoxError> {
    let after = payload
        .after
        .as_ref()
        .ok_or("changefeed row missing 'after' image")?;
    Ok(Idea {
        id: after["id"].as_str().ok_or("missing id")?.to_string(),
        title: after["title"].as_str().ok_or("missing title")?.to_string(),
        votes: after["votes"].as_i64().unwrap_or(0),
    })
}
// end::cdc-handler[]

#[cfg(test)]
mod tests {
    // tag::cdc-test[]
    // No cluster needed: the handler is pure projection. Dispatch the row images
    // a webhook would carry — an insert, an update, and a null-`after` delete —
    // straight to the handler and assert what it extracts. Then drive the sink
    // with the raw wire bytes to confirm it parses the batch and treats a
    // resolved-timestamp heartbeat as "nothing to dispatch".
    use super::IdeaChangefeedHandler;
    use cockroachdb_cdc::{EventHandler, Payload, WebhookSink};
    use serde_json::{json, Value};

    fn row(after: Value, before: Option<Value>) -> Payload {
        Payload {
            after: Some(after),
            before,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn projects_inserts_and_counts_deletes() {
        let handler = IdeaChangefeedHandler::default();
        handler
            .handle(&row(json!({"id":"a","title":"Dark mode","votes":3}), None))
            .await
            .unwrap();
        handler
            .handle(&row(
                json!({"id":"a","title":"Dark mode","votes":4}),
                Some(json!({"id":"a"})),
            ))
            .await
            .unwrap();
        handler
            .handle(&row(Value::Null, Some(json!({"id":"a"}))))
            .await
            .unwrap();

        let handled = handler.handled();
        assert_eq!(
            handled.len(),
            2,
            "insert + update projected, delete excluded"
        );
        assert_eq!(handled[1].votes, 4, "update carries the new row image");
        assert_eq!(handler.deletes(), 1);
    }

    #[tokio::test]
    async fn webhook_sink_parses_batch_and_skips_resolved() {
        let sink = WebhookSink::new(IdeaChangefeedHandler::default());
        let changes = br#"{"payload":[
            {"after":{"id":"a","title":"Dark mode","votes":3},"key":["a"]},
            {"after":{"id":"b","title":"CSV export","votes":1},"key":["b"]}
        ],"length":2}"#;

        let out = sink.handle(None, changes).await.expect("deliver batch");
        assert_eq!(out.processed, 2);
        assert!(!out.resolved);

        let beat = sink
            .handle(None, br#"{"resolved":"1700000000.0"}"#)
            .await
            .expect("deliver heartbeat");
        assert!(beat.resolved);
        assert_eq!(beat.processed, 0);
    }
    // end::cdc-test[]
}
