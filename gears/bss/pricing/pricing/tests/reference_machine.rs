//! Exhaustive state/event contract for durable reference work.
#![allow(clippy::expect_used, clippy::unwrap_used)]
use bss_pricing::domain::{
    price::OpState,
    reference_op::{Effect, Event, Op, next},
};
use uuid::Uuid;
#[test]
fn every_state_event_pair_is_typed_and_never_panics() {
    use Effect::{Complete, Confirm, MarkLost, ReadSku, Release, Retry};
    use OpState::{Cancelling, Done, Releasing, Reserving, Written};
    let id = Uuid::from_u128(1);
    let events = [
        Event::Reserved { id },
        Event::ReserveRefused {
            code: "SKU_FENCED".into(),
        },
        Event::RegistryUnavailable,
        Event::SkuRefused {
            code: "BUNDLE_SKU_NOT_PRICEABLE".into(),
        },
        Event::Written,
        Event::Confirmed,
        Event::ConfirmFailed,
        Event::ReleasedOnConfirm,
        Event::Released,
        Event::ReleaseFailed,
    ];
    let table = [
        (
            Reserving,
            vec![
                Some((Reserving, ReadSku)),
                Some((Cancelling, Release)),
                Some((Reserving, Retry)),
                Some((Cancelling, Release)),
                Some((Written, Confirm)),
                None,
                None,
                None,
                None,
                None,
            ],
        ),
        (
            Written,
            vec![
                None,
                None,
                None,
                None,
                None,
                Some((Done, Complete)),
                Some((Written, Retry)),
                Some((Done, MarkLost)),
                None,
                None,
            ],
        ),
        (
            Cancelling,
            vec![
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some((Done, Complete)),
                Some((Cancelling, Retry)),
            ],
        ),
        (
            Releasing,
            vec![
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some((Done, Complete)),
                Some((Releasing, Retry)),
            ],
        ),
        (Done, vec![None; 10]),
    ];
    let mut count = 0;
    for (state, expected) in table {
        for (event, expected) in events.iter().zip(expected) {
            let op = Op {
                state,
                reservation_id: Some(id),
                refusal: None,
            };
            let got = next(op, event.clone());
            match expected {
                Some((state, effect)) => {
                    let (op, effects) = got.unwrap();
                    assert_eq!(op.state, state);
                    assert_eq!(effects, vec![effect]);
                }
                None => {
                    let error = got.unwrap_err();
                    assert_eq!(error.state, state);
                }
            }
            count += 1;
        }
    }
    assert_eq!(count, 50);
}
#[test]
fn receipts_and_refusals_survive_until_completion() {
    let id = Uuid::new_v4();
    let op = Op {
        state: OpState::Reserving,
        reservation_id: None,
        refusal: None,
    };
    let (op, _) = next(op, Event::Reserved { id }).unwrap();
    assert_eq!(op.reservation_id, Some(id));
    let (op, _) = next(
        op,
        Event::SkuRefused {
            code: "SKU_DRAFT".into(),
        },
    )
    .unwrap();
    let (op, _) = next(op, Event::Released).unwrap();
    assert_eq!(op.reservation_id, Some(id));
    assert_eq!(op.refusal.as_deref(), Some("SKU_DRAFT"));
}
