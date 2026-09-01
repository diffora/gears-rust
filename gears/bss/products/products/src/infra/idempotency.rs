//! The idempotency phase — the claim input, the claim/answer walk and the
//! verdicts the create and composite doors branch on (`design/01` §3.2,
//! P-D-42). Infra-owned so the batch worker's shared create path
//! (`crate::infra::create`) reaches it without depending on `api::rest`;
//! the doors import it back through that module's re-exports.
//!
//! Reading the `Idempotency-Key` header and rendering a replayed response
//! stay in `api::rest` — they are wire concerns; this module owns the
//! store-facing walk.

use axum::http::StatusCode;
use chrono::{DateTime, TimeDelta, Utc};
use serde_json::Value as JsonValue;
use uuid::Uuid;

use toolkit_db::secure::{AccessScope, DBRunner};

use crate::domain::error::DomainError;
use crate::infra::storage::RepoError;
use crate::infra::storage::repo::{self, IdempotencyClaim};

/// The `expires_at` a claim taken at `now` is stamped with — the key's
/// **retention** window, not a deadline anything waits on
/// (`design/01-foundation.md` §3.2 `inst-fd-idem-retention`).
///
/// The floor the design pins is `max(24h, max_freeze_timeout)`, and
/// [`crate::config::ProductsConfig`]'s own field doc records that the second half has no
/// source until the catalog-version feature exists. `retention_hours` is the
/// **operator's** value as
/// [`crate::config::ProductsConfig::resolved_idempotency_retention_hours`]
/// resolved it, carried from `gear.rs`'s `ctx.config_or_default()` on
/// `ApiState::idempotency_retention_hours` — an earlier version read
/// `ProductsConfig::default()` right here, so an operator who raised the
/// window silently got the 24-hour minimum instead.
///
/// # The floor is enforced at boot, and the fallback here is the floor too
///
/// The clamp belongs at boot, where a bad value cannot get past it, so this
/// function is not where the floor is decided — see
/// `resolved_idempotency_retention_hours`. What it must still not do is
/// **degrade**: the arithmetic can fail only for a window `chrono` cannot
/// add to `now`, and the previous `unwrap_or(now)` answered that with
/// `expires_at == now`, i.e. a key that is already expired when it is
/// written. The next request on it takes it over and re-executes the guarded
/// mutation — the longest window an operator can ask for silently becoming
/// no window at all. The fallback is therefore the floor, and it is logged:
/// the boot-time ceiling makes this unreachable, and an unreachable arm that
/// is wrong is exactly the kind that stays wrong.
fn idempotency_expiry(now: DateTime<Utc>, retention_hours: u32) -> DateTime<Utc> {
    let stamp = |hours: u32| {
        TimeDelta::try_hours(i64::from(hours)).and_then(|window| now.checked_add_signed(window))
    };
    if let Some(expires_at) = stamp(retention_hours) {
        return expires_at;
    }
    tracing::error!(
        retention_hours,
        floor_hours = crate::config::IDEMPOTENCY_RETENTION_FLOOR_HOURS,
        "bss-products: idempotency retention window is not representable; stamping the \
         design's floor instead"
    );
    // The floor is hours away from `now`, so this cannot fail for any
    // instant a running process can observe; `now` is returned only for one
    // within a day of `chrono`'s own maximum, and the error above has
    // already been emitted by then.
    stamp(crate::config::IDEMPOTENCY_RETENTION_FLOOR_HOURS).unwrap_or(now)
}

/// Everything [`claim_idempotency`] needs beyond the runner, the scope and
/// the tenant, grouped because these five always travel together: a door
/// that has a key to claim has all of them, never a subset.
///
/// [`Clone`] because the mutation that carries it runs under
/// `Db::transaction_with_retry`, whose body is `FnMut` and may be re-entered
/// on a retryable contention failure: every attempt gets its own copy of the
/// inputs, and no attempt can consume what the next one needs. The values
/// are the same on every attempt by construction — `now` and `expires_at`
/// are stamped once, before the first attempt, so a retry claims the same
/// window rather than sliding it forward.
#[derive(Clone)]
pub(crate) struct IdempotencyClaimInput {
    /// The **concrete resource path**, never the route template (P-D-42):
    /// `/bss-products/v1/products`, not a pattern with a placeholder. Three
    /// reserved lane names — `internal:scheduled-activation`,
    /// `internal:cascade-leg`, `internal:bulk-row` — are held for non-HTTP
    /// callers; this phase has none, so both doors pass their own path.
    ///
    /// An owned [`String`], not a `&'static str`, and P-D-42 is the reason:
    /// a create's concrete path is a constant because there is no id yet to
    /// put in one, but **every id-bearing door's is not** — the publish and
    /// discard doors claim under
    /// `/bss-products/v1/products/{that id}/publish`, a value that exists
    /// only per request. The alternatives were claiming under the route
    /// template, which is exactly what P-D-42 forbids, and leaking a
    /// `String` per request to manufacture a `'static` lifetime, which
    /// trades a spec violation for an unbounded allocation. Both create
    /// doors still pass their `&'static str` constant unchanged:
    /// [`IdempotencyClaimInput::new`] takes `impl Into<String>`.
    pub(crate) endpoint: String,
    /// The caller's own `Idempotency-Key`, as [`idempotency_key`] read it.
    pub(crate) client_key: String,
    /// `crate::domain::idempotency::payload_digest` over the parsed body.
    pub(crate) payload_hash: Vec<u8>,
    /// The door's own request instant.
    pub(crate) now: DateTime<Utc>,
    /// [`idempotency_expiry`]'s answer for that instant.
    pub(crate) expires_at: DateTime<Utc>,
}

impl IdempotencyClaimInput {
    /// Build the input for `endpoint`, `client_key` and `payload_hash` at
    /// `now`, stamping the expiry from the retention window so no door
    /// spells that arithmetic itself.
    ///
    /// `retention_hours` is the operator's own window, read off
    /// `ApiState::idempotency_retention_hours` by the calling door — this
    /// type reaches no configuration of its own, exactly as it reasons about
    /// no clock of its own.
    pub(crate) fn new(
        endpoint: impl Into<String>,
        client_key: String,
        payload_hash: Vec<u8>,
        now: DateTime<Utc>,
        retention_hours: u32,
    ) -> Self {
        Self {
            endpoint: endpoint.into(),
            client_key,
            payload_hash,
            now,
            expires_at: idempotency_expiry(now, retention_hours),
        }
    }
}

/// What a door does next, having asked the store for the key.
///
/// Three outcomes, kept apart here rather than collapsed into a
/// `Result<bool, _>`, because each one is a different act: one proceeds, one
/// answers without executing anything, and one refuses.
pub(crate) enum ClaimVerdict {
    /// The key is this caller's; the guarded mutation runs.
    Proceed,
    /// A stored answer exists for this key **and** the payloads agree: serve
    /// it and execute nothing (§3.2 `inst-fd-idem-replay-outcome`).
    Replay {
        /// The status the original caller was told.
        status: i32,
        /// The body the original caller was told.
        body: JsonValue,
    },
    /// `IDEMPOTENCY_CONFLICT` (a different payload under a live key, in
    /// **either** of its states — stored answer or live claim) or
    /// `IDEMPOTENCY_KEY_IN_FLIGHT` (the same payload under a live claim, or
    /// a lost takeover race). Both execute nothing and both audit through
    /// `audit_refusal_and_report` like every other refusal on this
    /// surface.
    Refused(DomainError),
}

/// Take the claim for `input` **on the caller's own runner** and read the
/// outcome as a [`ClaimVerdict`].
///
/// # `runner` MUST be the guarded mutation's transaction
///
/// `repo::claim_idempotency_key`'s own doc states the obligation and why it
/// is stricter than `repo::resolve_actor_ref`'s: the claim `INSERT` **is**
/// the gate (P-D-42), and joining the mutation's transaction is what makes a
/// rollback free the key with no release step. A claim taken on a runner of
/// its own would survive a mutation that rolled back and lock the key
/// against an act that never happened — the one property this whole
/// mechanism exists to provide. Both doors therefore call this from inside
/// their `insert_*_with_event` closure, before the entity insert.
///
/// The payload comparison is made **here** and not in the repository: that
/// layer was never handed the incoming request to compare against the stored
/// digest (`IdempotencyClaim::Answered`'s own doc), and the comparison is
/// what separates a replay from `IDEMPOTENCY_CONFLICT`
/// (§3.2 `inst-fd-idem-conflict`).
///
/// # The comparison is owed against a live claim too, with one exception
///
/// §3.2 `inst-fd-idem-claim-inflight` reserves `IDEMPOTENCY_KEY_IN_FLIGHT`
/// for a duplicate "**whose payload hash matches the claimed key's**"; a
/// mismatch "stays `IDEMPOTENCY_CONFLICT` **in either state**". So the digest
/// is compared against an `answered` row and against a live `claimed` one
/// alike, and only the matching duplicate is told in flight. Answering
/// in-flight for a mismatch would tell a client that its *different* request
/// is merely racing itself, and invite the retry that keeps being refused.
///
/// The exception is `IdempotencyClaim::TakeoverRaceLost` and it is the
/// reason that outcome is a variant rather than an in-flight hit with no
/// digest: the loser of an expired-key takeover "may even carry a different
/// payload from the winner, and is still refused in-flight rather than for
/// the mismatch, since this transaction never compared the two"
/// (§3.2 `inst-fd-idem-retention`, P-D-49). It read the *expired* holder's
/// row; the payload now under the key is the winner's, which it never saw.
/// A conflict raised from a digest this transaction never read would be a
/// fabricated verdict, so the two paths stay apart in the type rather than
/// in a comment.
///
/// # Errors
///
/// [`RepoError`] exactly as `repo::claim_idempotency_key` raises it — a
/// storage or scope failure, or a stored row that contradicts its own
/// `CHECK` constraints.
pub(crate) async fn claim_idempotency(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    input: &IdempotencyClaimInput,
) -> Result<ClaimVerdict, RepoError> {
    let claim = repo::claim_idempotency_key(
        runner,
        scope,
        tenant_id,
        &input.endpoint,
        &input.client_key,
        &input.payload_hash,
        input.now,
        input.expires_at,
    )
    .await?;

    Ok(match claim {
        IdempotencyClaim::Claimed => ClaimVerdict::Proceed,
        IdempotencyClaim::Answered {
            payload_hash,
            response_status,
            response_body,
        } => {
            if payload_hash == input.payload_hash {
                ClaimVerdict::Replay {
                    status: response_status,
                    body: response_body,
                }
            } else {
                ClaimVerdict::Refused(DomainError::IdempotencyConflict(format!(
                    "{} was already answered for a different payload on {}",
                    input.client_key, input.endpoint
                )))
            }
        }
        IdempotencyClaim::InFlight { payload_hash, .. } if payload_hash != input.payload_hash => {
            ClaimVerdict::Refused(DomainError::IdempotencyConflict(format!(
                "{} is held by an act still in flight under a different payload on {}",
                input.client_key, input.endpoint
            )))
        }
        IdempotencyClaim::InFlight { .. } | IdempotencyClaim::TakeoverRaceLost => {
            ClaimVerdict::Refused(DomainError::IdempotencyKeyInFlight(format!(
                "{} is held by an act still in flight on {}",
                input.client_key, input.endpoint
            )))
        }
    })
}

/// [`ClaimVerdict`] for a **composite** door (P-D-79): identical in every
/// arm but one — a live `claimed` row whose digest matches and whose
/// `entity_ref` is stamped is not a refusal but the resume signal.
///
/// The single-entity doors keep [`claim_idempotency`]'s reading: there, a
/// visible `claimed` row can only be a concurrent duplicate, because claim
/// and mutation commit together and the answer is recorded in the same
/// transaction. A composite act commits its claim with the *first* entity
/// and answers only at completion (P-D-72), so its committed-and-unanswered
/// claim means *in progress* — and the matching retry re-enters instead of
/// being told in flight.
pub(crate) enum CompositeClaimVerdict {
    /// Fresh claim: proceed with the composite's first transaction.
    Proceed,
    /// The act completed earlier; this is its stored answer.
    Replay {
        /// The stored status.
        status: i32,
        /// The stored body.
        body: JsonValue,
    },
    /// The idempotency phase refused; nothing was written.
    Refused(DomainError),
    /// A committed-but-unanswered claim with a matching digest: the act is
    /// in progress or crashed mid-composite. Resume from its parent.
    Resume {
        /// The stamped parent handle the re-entry scans from.
        entity_ref: Uuid,
    },
}

/// Take or read the claim for a composite door, classifying a matching
/// live claim as [`CompositeClaimVerdict::Resume`] (P-D-79).
///
/// The same `runner`-is-the-mutation's-transaction obligation as
/// [`claim_idempotency`]; the digest comparisons are the same ones. A
/// matching live claim with **no** stamp is refused in flight rather than
/// resumed: this door stamps `entity_ref` in the claim's own transaction,
/// so a visible claim without one was written by something else and resuming
/// from it would scan a parent this act never created.
pub(crate) async fn claim_composite_idempotency(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    input: &IdempotencyClaimInput,
) -> Result<CompositeClaimVerdict, RepoError> {
    let claim = repo::claim_idempotency_key(
        runner,
        scope,
        tenant_id,
        &input.endpoint,
        &input.client_key,
        &input.payload_hash,
        input.now,
        input.expires_at,
    )
    .await?;

    Ok(match claim {
        IdempotencyClaim::Claimed => CompositeClaimVerdict::Proceed,
        IdempotencyClaim::Answered {
            payload_hash,
            response_status,
            response_body,
        } => {
            if payload_hash == input.payload_hash {
                CompositeClaimVerdict::Replay {
                    status: response_status,
                    body: response_body,
                }
            } else {
                CompositeClaimVerdict::Refused(DomainError::IdempotencyConflict(format!(
                    "{} was already answered for a different payload on {}",
                    input.client_key, input.endpoint
                )))
            }
        }
        IdempotencyClaim::InFlight { payload_hash, .. } if payload_hash != input.payload_hash => {
            CompositeClaimVerdict::Refused(DomainError::IdempotencyConflict(format!(
                "{} is held by an act still in flight under a different payload on {}",
                input.client_key, input.endpoint
            )))
        }
        IdempotencyClaim::InFlight {
            entity_ref: Some(entity_ref),
            ..
        } => CompositeClaimVerdict::Resume { entity_ref },
        IdempotencyClaim::InFlight { .. } | IdempotencyClaim::TakeoverRaceLost => {
            CompositeClaimVerdict::Refused(DomainError::IdempotencyKeyInFlight(format!(
                "{} is held by an act still in flight on {}",
                input.client_key, input.endpoint
            )))
        }
    })
}

/// Record the answer for `input`'s key **on the caller's own runner**: the
/// status and body the door is about to return, written into the claim the
/// same transaction took (§3.2 `inst-fd-idem-claim-write`, P-D-29).
///
/// # `runner` MUST be the runner the claim and the mutation ran on
///
/// `repo::answer_idempotency_key`'s own doc states the obligation:
/// claim, mutation and answer commit together or not at all. Both doors call
/// this from inside their `insert_*_with_event` closure, after the entity
/// insert and the outbox enqueue, on the same `tx`.
///
/// # A key that is not held fails the mutation
///
/// `repo::answer_idempotency_key` reports `IdempotencyAnswer::NotHeld`
/// rather than raising, because a lane answering a claim taken elsewhere may
/// legitimately meet it. **This caller is not that caller**: the claim was
/// taken on this very transaction moments earlier, so nothing outside it can
/// have moved the row, and a `NotHeld` here means the store contradicts
/// itself. Answering `201` anyway would commit an act whose answer was never
/// recorded — the state this whole write exists to remove — so it is
/// surfaced as an error, the mutation rolls back, and the key is left free
/// for the client's retry to claim honestly.
///
/// # Errors
///
/// [`RepoError::Db`] as the repository raises it, or one naming the
/// unheld key.
pub(crate) async fn record_idempotency_answer(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    input: &IdempotencyClaimInput,
    status: StatusCode,
    body: &JsonValue,
) -> Result<(), RepoError> {
    let recorded = repo::answer_idempotency_key(
        runner,
        scope,
        tenant_id,
        &input.endpoint,
        &input.client_key,
        i32::from(status.as_u16()),
        body.clone(),
    )
    .await?;

    if recorded == repo::IdempotencyAnswer::NotHeld {
        return Err(RepoError::Db(format!(
            "idempotency key {} on {} was claimed by this transaction but no claimed row \
             remained to answer",
            input.client_key, input.endpoint
        )));
    }
    Ok(())
}
