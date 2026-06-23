//! Client-facing **delta streaming** wire model and the projector that turns a
//! backend plugin's [`StreamingEvent`] stream into it (FR-024).
//!
//! The plugin contract stays chunk-based (`Start` → `Chunk*` → `Complete`/`Error`);
//! Chat Engine *projects* that into the SSE delta protocol the client consumes:
//! a `start` opens an (empty) message document, `delta` events mutate it by
//! `(op, path, value)`, and `complete`/`error` terminate it. Every wire event
//! carries a per-message monotonic `seq` (mirrored in the SSE `id:` line) for
//! ordering, de-duplication, and resume.
//!
//! See DESIGN `cpt-cf-chat-engine-design-streaming-protocol` and
//! `cpt-cf-chat-engine-adr-sse-delta-streaming`.
//
// @cpt-cf-chat-engine-design-streaming-protocol:p1
// @cpt-cf-chat-engine-adr-sse-delta-streaming:p1

use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};
use uuid::Uuid;

use crate::domain::message::StreamingEvent;

/// Mutation operation carried by a [`WireStreamEvent::Delta`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeltaOp {
    /// Set the value at `path` (create a part, set a field).
    Add,
    /// Append `value` to the existing value at `path` (text fragment onto a
    /// text body, element onto a citation array).
    Append,
    /// Replace a scalar/field at `path`.
    Patch,
    /// Remove the value at `path`.
    Remove,
}

/// One event of the client-facing delta stream. Serialized with a `"type"`
/// discriminator (`start` / `delta` / `complete` / `error`); `seq` mirrors the
/// SSE `id:` line.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireStreamEvent {
    /// Opens the assistant message document (empty; no parts yet).
    Start {
        message_id: Uuid,
        seq: u64,
    },
    /// Mutates the message document by `(op, path, value)`.
    Delta {
        message_id: Uuid,
        seq: u64,
        op: DeltaOp,
        path: String,
        value: JsonValue,
    },
    /// Successful end; carries optional plugin metadata. Terminal.
    Complete {
        message_id: Uuid,
        seq: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<JsonValue>,
    },
    /// Terminal error.
    Error {
        message_id: Uuid,
        seq: u64,
        error: String,
    },
}

impl WireStreamEvent {
    /// The per-message sequence number (also the SSE `id:`).
    #[must_use]
    pub fn seq(&self) -> u64 {
        match self {
            WireStreamEvent::Start { seq, .. }
            | WireStreamEvent::Delta { seq, .. }
            | WireStreamEvent::Complete { seq, .. }
            | WireStreamEvent::Error { seq, .. } => *seq,
        }
    }

    /// The SSE `event:` name for this event.
    #[must_use]
    pub fn event_name(&self) -> &'static str {
        match self {
            WireStreamEvent::Start { .. } => "start",
            WireStreamEvent::Delta { .. } => "delta",
            WireStreamEvent::Complete { .. } => "complete",
            WireStreamEvent::Error { .. } => "error",
        }
    }
}

/// Path of the assistant's primary text part body (tokens append here).
const TEXT_BODY_PATH: &str = "parts/0/content/text";
/// Path of the assistant's primary text part (opened with `add`).
const TEXT_PART_PATH: &str = "parts/0";

/// Stateful projector: feed it the plugin's [`StreamingEvent`]s in order and it
/// yields the client-facing [`WireStreamEvent`]s, assigning a monotonic `seq`.
///
/// The assistant answer accumulates into a single `text` part at `parts/0`:
/// the first chunk opens the part (`add parts/0`), subsequent chunks append to
/// `parts/0/content/text`. Citations/references on `Complete` are appended to
/// the part's arrays as `delta`s before the terminal `complete`.
pub struct DeltaProjector {
    message_id: Uuid,
    next_seq: u64,
    text_opened: bool,
}

impl DeltaProjector {
    /// Create a projector for `message_id` (the assistant message id Chat Engine
    /// assigned — wire events are always stamped with it, regardless of the id
    /// the plugin echoes).
    #[must_use]
    pub fn new(message_id: Uuid) -> Self {
        Self {
            message_id,
            next_seq: 0,
            text_opened: false,
        }
    }

    fn take_seq(&mut self) -> u64 {
        let s = self.next_seq;
        self.next_seq += 1;
        s
    }

    fn delta(&mut self, op: DeltaOp, path: impl Into<String>, value: JsonValue) -> WireStreamEvent {
        WireStreamEvent::Delta {
            message_id: self.message_id,
            seq: self.take_seq(),
            op,
            path: path.into(),
            value,
        }
    }

    /// Project one plugin event into zero or more wire events.
    pub fn project(&mut self, event: StreamingEvent) -> Vec<WireStreamEvent> {
        match event {
            StreamingEvent::Start(_) => {
                vec![WireStreamEvent::Start {
                    message_id: self.message_id,
                    seq: self.take_seq(),
                }]
            }
            StreamingEvent::Chunk(c) => {
                let mut out = Vec::new();
                if !self.text_opened {
                    self.text_opened = true;
                    out.push(self.delta(
                        DeltaOp::Add,
                        TEXT_PART_PATH,
                        json!({ "type": "text", "content": { "text": "" }, "number": 0 }),
                    ));
                }
                out.push(self.delta(DeltaOp::Append, TEXT_BODY_PATH, JsonValue::String(c.chunk)));
                out
            }
            StreamingEvent::Complete(c) => {
                let mut out = Vec::new();
                if !c.file_citations.is_empty() {
                    let v = serde_json::to_value(&c.file_citations).unwrap_or(JsonValue::Null);
                    out.push(self.delta(DeltaOp::Append, "parts/0/file_citations", v));
                }
                if !c.link_citations.is_empty() {
                    let v = serde_json::to_value(&c.link_citations).unwrap_or(JsonValue::Null);
                    out.push(self.delta(DeltaOp::Append, "parts/0/link_citations", v));
                }
                if !c.references.is_empty() {
                    let v = serde_json::to_value(&c.references).unwrap_or(JsonValue::Null);
                    out.push(self.delta(DeltaOp::Append, "parts/0/references", v));
                }
                out.push(WireStreamEvent::Complete {
                    message_id: self.message_id,
                    seq: self.take_seq(),
                    metadata: c.metadata,
                });
                out
            }
            StreamingEvent::Error(e) => {
                vec![WireStreamEvent::Error {
                    message_id: self.message_id,
                    seq: self.take_seq(),
                    error: e.error,
                }]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::message::{
        StreamingChunkEvent, StreamingCompleteEvent, StreamingErrorEvent, StreamingStartEvent,
    };

    fn mid() -> Uuid {
        Uuid::parse_str("00000000-0000-0000-0000-0000000000aa").unwrap()
    }

    fn complete(file_citations: Vec<chat_engine_sdk::models::FileCitation>) -> StreamingEvent {
        StreamingEvent::Complete(StreamingCompleteEvent {
            message_id: Uuid::nil(),
            metadata: Some(json!({ "finish_reason": "stop" })),
            file_citations,
            link_citations: vec![],
            references: vec![],
        })
    }

    #[test]
    fn happy_path_projects_start_text_deltas_and_complete() {
        let mut p = DeltaProjector::new(mid());
        let mut events = Vec::new();
        events.extend(p.project(StreamingEvent::Start(StreamingStartEvent {
            message_id: Uuid::nil(),
        })));
        events.extend(p.project(StreamingEvent::Chunk(StreamingChunkEvent {
            message_id: Uuid::nil(),
            chunk: "Hel".into(),
        })));
        events.extend(p.project(StreamingEvent::Chunk(StreamingChunkEvent {
            message_id: Uuid::nil(),
            chunk: "lo".into(),
        })));
        events.extend(p.project(complete(vec![])));

        // start, (add parts/0 + append), append, complete = 5 events
        assert_eq!(events.len(), 5);
        // seq is contiguous from 0 and every event carries our message_id.
        for (i, e) in events.iter().enumerate() {
            assert_eq!(e.seq(), i as u64);
        }
        assert_eq!(events[0].event_name(), "start");
        // First chunk opens the text part then appends.
        assert!(matches!(
            &events[1],
            WireStreamEvent::Delta { op: DeltaOp::Add, path, .. } if path == "parts/0"
        ));
        assert!(matches!(
            &events[2],
            WireStreamEvent::Delta { op: DeltaOp::Append, path, value: JsonValue::String(s), .. }
                if path == "parts/0/content/text" && s == "Hel"
        ));
        // Second chunk only appends (part already open).
        assert!(matches!(
            &events[3],
            WireStreamEvent::Delta { op: DeltaOp::Append, path, .. } if path == "parts/0/content/text"
        ));
        assert_eq!(events[4].event_name(), "complete");
    }

    #[test]
    fn citations_on_complete_become_append_deltas_before_complete() {
        let cite: chat_engine_sdk::models::FileCitation = serde_json::from_value(json!({
            "document_id": "doc-1", "document_name": "Doc", "index": 1
        }))
        .unwrap();
        let mut p = DeltaProjector::new(mid());
        let _ = p.project(StreamingEvent::Start(StreamingStartEvent {
            message_id: Uuid::nil(),
        }));
        let _ = p.project(StreamingEvent::Chunk(StreamingChunkEvent {
            message_id: Uuid::nil(),
            chunk: "x".into(),
        }));
        let tail = p.project(complete(vec![cite]));
        // append parts/0/file_citations, then complete
        assert_eq!(tail.len(), 2);
        assert!(matches!(
            &tail[0],
            WireStreamEvent::Delta { op: DeltaOp::Append, path, .. } if path == "parts/0/file_citations"
        ));
        assert_eq!(tail[1].event_name(), "complete");
    }

    #[test]
    fn error_projects_single_terminal_error() {
        let mut p = DeltaProjector::new(mid());
        let out = p.project(StreamingEvent::Error(StreamingErrorEvent {
            message_id: Uuid::nil(),
            error: "boom".into(),
        }));
        assert_eq!(out.len(), 1);
        assert!(matches!(&out[0], WireStreamEvent::Error { error, .. } if error == "boom"));
    }

    #[test]
    fn wire_event_serializes_with_type_and_seq() {
        let ev = WireStreamEvent::Delta {
            message_id: mid(),
            seq: 7,
            op: DeltaOp::Append,
            path: "parts/0/content/text".into(),
            value: json!("hi"),
        };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["type"], "delta");
        assert_eq!(v["seq"], 7);
        assert_eq!(v["op"], "append");
        assert_eq!(v["path"], "parts/0/content/text");
        assert_eq!(v["value"], "hi");
    }
}
