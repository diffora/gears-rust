//! The single pure transition table used by both request and recovery execution.
use super::price::OpState;
use uuid::Uuid;
/// Durable protocol state; request metadata belongs to the persistence adapter.
#[toolkit_macros::domain_model]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Op {
    pub state: OpState,
    pub reservation_id: Option<Uuid>,
    pub refusal: Option<String>,
}
/// Observations from the registry or a committed local write.
#[toolkit_macros::domain_model]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Reserved { id: Uuid },
    ReserveRefused { code: String },
    RegistryUnavailable,
    SkuRefused { code: String },
    Written,
    Confirmed,
    ConfirmFailed,
    ReleasedOnConfirm,
    Released,
    ReleaseFailed,
}
/// Work authorized by a transition; effects are executed outside the pure model.
#[toolkit_macros::domain_model]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    ReadSku,
    Confirm,
    Release,
    Retry,
    Complete,
    MarkLost,
}
/// Illegal input is a typed failure, including all observations on terminal work.
#[toolkit_macros::domain_model]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("illegal reference event {event:?} in {state:?}")]
pub struct IllegalTransition {
    pub state: OpState,
    pub event: Event,
}
/// Advance one observation. A confirm timeout can only schedule another confirm.
/// # Errors
/// Returns `IllegalTransition` for every unlisted state/event pair.
pub fn next(mut op: Op, event: Event) -> Result<(Op, Vec<Effect>), IllegalTransition> {
    use OpState::{Cancelling, Done, Releasing, Reserving, Written};
    let (state, effect) = match (op.state, &event) {
        (Reserving, Event::Reserved { id }) => {
            op.reservation_id = Some(*id);
            (Reserving, Effect::ReadSku)
        }
        (Reserving, Event::ReserveRefused { code } | Event::SkuRefused { code }) => {
            op.refusal = Some(code.clone());
            (Cancelling, Effect::Release)
        }
        (Reserving, Event::RegistryUnavailable) => (Reserving, Effect::Retry),
        (Reserving, Event::Written) => (Written, Effect::Confirm),
        (Written, Event::Confirmed) | (Cancelling | Releasing, Event::Released) => {
            (Done, Effect::Complete)
        }
        (Written, Event::ConfirmFailed) => (Written, Effect::Retry),
        (Written, Event::ReleasedOnConfirm) => (Done, Effect::MarkLost),
        (Cancelling | Releasing, Event::ReleaseFailed) => (op.state, Effect::Retry),
        _ => {
            return Err(IllegalTransition {
                state: op.state,
                event,
            });
        }
    };
    op.state = state;
    Ok((op, vec![effect]))
}
