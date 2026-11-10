//! The wire body a `DomainError` maps to. A 5xx body names no cause (no backend
//! kind, topic or driver text): the cause is logged where the domain error
//! becomes the canonical one, and the 5xx tests read that log line too. A size
//! refusal's body is the one the SDK reads back.

use std::cell::RefCell;

use serde_json::json;
use toolkit::api::canonical_prelude::CanonicalError;
use toolkit_canonical_errors::Problem;
use tracing_test::traced_test;

use event_broker_sdk::EventBrokerError;

use crate::domain::error::DomainError;
use crate::test_support::mapping_lines;

/// A driver-shaped error with its own cause, so the log must walk two levels.
#[derive(Debug, thiserror::Error)]
#[error("pool timed out")]
struct PoolTimedOut(#[source] std::io::Error);

/// Shaped like sea-orm's `DbErr::Query` and sqlx's `Error::Database`: the
/// outer `Display` already embeds the inner error it also returns as source.
#[derive(Debug, thiserror::Error)]
#[error("Query Error: {0}")]
struct QueryError(#[source] std::io::Error);

fn wire_body(err: DomainError) -> serde_json::Value {
    serde_json::to_value(Problem::from(CanonicalError::from(err))).expect("Problem serializes")
}

#[test]
#[traced_test]
fn a_storage_outage_answers_the_generic_503_body_and_logs_its_reason_and_source_chain() {
    let body = wire_body(DomainError::StorageUnavailable {
        reason: "database unavailable".to_owned(),
        source: Some(Box::new(PoolTimedOut(std::io::Error::other(
            "connection refused by db.internal:5432",
        )))),
    });

    assert_eq!(
        body,
        json!({
            "type": "gts://gts.cf.core.errors.err.v1~cf.core.err.service_unavailable.v1~",
            "title": "Service Unavailable",
            "status": 503,
            "detail": "Service temporarily unavailable",
            "context": {},
        })
    );
    let logged = RefCell::new(Vec::new());
    logs_assert(|lines| {
        *logged.borrow_mut() = mapping_lines(lines);
        Ok(())
    });
    assert_eq!(
        logged.into_inner(),
        vec![
            "WARN answering 503 with the generic body error=storage backend unavailable: \
             database unavailable: pool timed out: connection refused by db.internal:5432"
                .to_owned()
        ]
    );
}

/// A source whose text its parent already printed is not printed again, so a
/// driver error that embeds its cause appears once.
#[test]
#[traced_test]
fn a_source_already_embedded_in_its_parent_is_logged_once() {
    let body = wire_body(DomainError::StorageUnavailable {
        reason: "database unavailable".to_owned(),
        source: Some(Box::new(QueryError(std::io::Error::other(
            "connection reset by peer",
        )))),
    });

    assert_eq!(
        body,
        json!({
            "type": "gts://gts.cf.core.errors.err.v1~cf.core.err.service_unavailable.v1~",
            "title": "Service Unavailable",
            "status": 503,
            "detail": "Service temporarily unavailable",
            "context": {},
        })
    );
    let logged = RefCell::new(Vec::new());
    logs_assert(|lines| {
        *logged.borrow_mut() = mapping_lines(lines);
        Ok(())
    });
    assert_eq!(
        logged.into_inner(),
        vec![
            "WARN answering 503 with the generic body error=storage backend unavailable: \
             database unavailable: Query Error: connection reset by peer"
                .to_owned()
        ]
    );
}

/// Wraps a source it does not embed whole: its text holds the source's only as a
/// prefix, so the source is a distinct link of the chain.
#[derive(Debug, thiserror::Error)]
#[error("pool timed out after connection reset by peer twice")]
struct RetriedPool(#[source] std::io::Error);

/// A source's text that its parent holds only somewhere inside, not at its end, is
/// printed: only the `"<prefix>: {0}"` shape of sea-orm's and sqlx's errors embeds a
/// source.
#[test]
#[traced_test]
fn a_source_its_parent_quotes_only_mid_text_is_still_logged() {
    let body = wire_body(DomainError::StorageUnavailable {
        reason: "database unavailable".to_owned(),
        source: Some(Box::new(RetriedPool(std::io::Error::other(
            "connection reset by peer",
        )))),
    });

    assert_eq!(
        body,
        json!({
            "type": "gts://gts.cf.core.errors.err.v1~cf.core.err.service_unavailable.v1~",
            "title": "Service Unavailable",
            "status": 503,
            "detail": "Service temporarily unavailable",
            "context": {},
        })
    );
    let logged = RefCell::new(Vec::new());
    logs_assert(|lines| {
        *logged.borrow_mut() = mapping_lines(lines);
        Ok(())
    });
    assert_eq!(
        logged.into_inner(),
        vec![
            "WARN answering 503 with the generic body error=storage backend unavailable: \
             database unavailable: pool timed out after connection reset by peer twice: \
             connection reset by peer"
                .to_owned()
        ]
    );
}

/// A topic its backend does not hold yet is a retryable outage like any
/// other: the topic id goes to the log, not the body.
#[test]
#[traced_test]
fn a_topic_not_provisioned_answers_the_generic_503_body_and_logs_the_topic() {
    let body = wire_body(DomainError::StorageUnavailable {
        reason: "topic 'gts.cf.core.events.topic.v1~x.eb.t1.topic.v1' is not provisioned on \
                 its storage backend yet"
            .to_owned(),
        source: None,
    });

    assert_eq!(
        body,
        json!({
            "type": "gts://gts.cf.core.errors.err.v1~cf.core.err.service_unavailable.v1~",
            "title": "Service Unavailable",
            "status": 503,
            "detail": "Service temporarily unavailable",
            "context": {},
        })
    );
    let logged = RefCell::new(Vec::new());
    logs_assert(|lines| {
        *logged.borrow_mut() = mapping_lines(lines);
        Ok(())
    });
    assert_eq!(
        logged.into_inner(),
        vec![
            "WARN answering 503 with the generic body error=storage backend unavailable: \
             topic 'gts.cf.core.events.topic.v1~x.eb.t1.topic.v1' is not provisioned on \
             its storage backend yet"
                .to_owned()
        ]
    );
}

#[test]
#[traced_test]
fn an_internal_failure_answers_the_generic_500_body_and_logs_its_detail_at_error() {
    let body = wire_body(DomainError::Internal(
        "stored GTS id 'gts.bad' is malformed: missing tilde".to_owned(),
    ));

    assert_eq!(
        body,
        json!({
            "type": "gts://gts.cf.core.errors.err.v1~cf.core.err.internal.v1~",
            "title": "Internal",
            "status": 500,
            "detail": "An internal error occurred. Please retry later.",
            "context": {},
        })
    );
    let logged = RefCell::new(Vec::new());
    logs_assert(|lines| {
        *logged.borrow_mut() = mapping_lines(lines);
        Ok(())
    });
    assert_eq!(
        logged.into_inner(),
        vec![
            "ERROR answering 500 with the generic body error=stored GTS id 'gts.bad' is \
             malformed: missing tilde"
                .to_owned()
        ]
    );
}

/// A `501` names the capability that is not built yet (a named consumer group):
/// it tells the caller what the API does not do yet, not how the infrastructure
/// failed, so its detail stays and nothing is logged.
#[test]
#[traced_test]
fn an_unbuilt_capability_keeps_its_name_in_the_501_body() {
    let body = wire_body(DomainError::Unimplemented(
        "named consumer groups are not implemented".to_owned(),
    ));

    assert_eq!(
        body,
        json!({
            "type": "gts://gts.cf.core.errors.err.v1~cf.core.err.unimplemented.v1~",
            "title": "Unimplemented",
            "status": 501,
            "detail": "named consumer groups are not implemented",
            "context": {
                "resource_type": "gts.cf.core.events.consumer_group.v1~",
            },
        })
    );
    let logged = RefCell::new(Vec::new());
    logs_assert(|lines| {
        *logged.borrow_mut() = mapping_lines(lines);
        Ok(())
    });
    assert_eq!(logged.into_inner(), Vec::<String>::new());
}

/// Both batch bounds answer `413` with the field and the reason the SDK reads its
/// `BatchTooLarge` back by.
#[test]
fn a_batch_over_either_bound_answers_413_with_the_batch_too_large_reason() {
    let count = CanonicalError::from(DomainError::BatchTooLarge {
        count: 101,
        max: 100,
    });
    let bytes = CanonicalError::from(DomainError::BatchPayloadTooLarge {
        bytes: 1_048_577,
        max: 1_048_576,
    });

    assert_eq!(
        serde_json::to_value(Problem::from(count.clone())).expect("Problem serializes"),
        json!({
            "type": "gts://gts.cf.core.errors.err.v1~cf.core.err.invalid_argument.v1~",
            "title": "Invalid Argument",
            "status": 413,
            "detail": "Request validation failed",
            "context": {
                "field_violations": [{
                    "field": "batch.count",
                    "description": "batch too large: 101 events (max 100)",
                    "reason": "batch_too_large",
                }],
                "resource_type": "gts.cf.core.events.request.v1~",
            },
        })
    );
    assert_eq!(
        serde_json::to_value(Problem::from(bytes.clone())).expect("Problem serializes"),
        json!({
            "type": "gts://gts.cf.core.errors.err.v1~cf.core.err.invalid_argument.v1~",
            "title": "Invalid Argument",
            "status": 413,
            "detail": "Request validation failed",
            "context": {
                "field_violations": [{
                    "field": "batch.bytes",
                    "description": "batch payload too large: 1048577 bytes (max 1048576)",
                    "reason": "batch_too_large",
                }],
                "resource_type": "gts.cf.core.events.request.v1~",
            },
        })
    );
    // Read back by the SDK and mapped again, each gives the broker's body unchanged.
    for canonical in [count, bytes] {
        let read_back = EventBrokerError::from(canonical.clone());
        assert_eq!(read_back.kind(), "BatchTooLarge", "{read_back:?}");
        assert_eq!(
            serde_json::to_value(Problem::from(CanonicalError::from(read_back)))
                .expect("Problem serializes"),
            serde_json::to_value(Problem::from(canonical)).expect("Problem serializes")
        );
    }
}

/// An event over the per-event limit answers `413` naming its batch position, with
/// the size and the limit the SDK reads back.
#[test]
fn an_event_over_the_size_limit_answers_413_and_reads_back_with_its_sizes() {
    let canonical = CanonicalError::from(DomainError::EventTooLarge {
        index: 3,
        bytes: 61_441,
        max: 61_440,
    });

    assert_eq!(
        serde_json::to_value(Problem::from(canonical.clone())).expect("Problem serializes"),
        json!({
            "type": "gts://gts.cf.core.errors.err.v1~cf.core.err.invalid_argument.v1~",
            "title": "Invalid Argument",
            "status": 413,
            "detail": "Request validation failed",
            "context": {
                "field_violations": [{
                    "field": "events[3]",
                    "description": "61441 bytes encoded, over the limit of 61440",
                    "reason": "event_too_large",
                }],
                "resource_type": "gts.cf.core.events.request.v1~",
            },
        })
    );
    let read_back = EventBrokerError::from(canonical);
    let EventBrokerError::EventTooLarge { bytes, max } = read_back else {
        panic!("the SDK must read back EventTooLarge, got {read_back:?}");
    };
    assert_eq!((bytes, max), (61_441, 61_440));
}

/// A partition key over its cap answers `400` naming the event's batch position,
/// with the size and the cap the SDK reads back, and nothing of the key.
#[test]
fn a_partition_key_over_its_cap_answers_400_and_reads_back_with_its_sizes() {
    let canonical = CanonicalError::from(DomainError::PartitionKeyTooLong {
        index: 0,
        bytes: 1025,
        max: 1024,
    });

    assert_eq!(
        serde_json::to_value(Problem::from(canonical.clone())).expect("Problem serializes"),
        json!({
            "type": "gts://gts.cf.core.errors.err.v1~cf.core.err.invalid_argument.v1~",
            "title": "Invalid Argument",
            "status": 400,
            "detail": "Request validation failed",
            "context": {
                "field_violations": [{
                    "field": "events[0].partition_key",
                    "description": "1025 bytes, over the limit of 1024",
                    "reason": "partition_key_too_long",
                }],
                "resource_type": "gts.cf.core.events.request.v1~",
            },
        })
    );
    let read_back = EventBrokerError::from(canonical);
    let EventBrokerError::PartitionKeyTooLong { bytes, max } = read_back else {
        panic!("the SDK must read back PartitionKeyTooLong, got {read_back:?}");
    };
    assert_eq!((bytes, max), (1025, 1024));
}
