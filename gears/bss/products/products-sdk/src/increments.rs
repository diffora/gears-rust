//! The increment-request contract (`design/06-catalog-version.md` §2 rule 1,
//! P-D-15, P-D-81) — the SDK's first write surface, a **second trait**
//! beside [`ProductsClient`](crate::api::ProductsClient) rather than a
//! widening of it: that one calls itself the contract for *reading*
//! registry entities, and its own doc scopes it so.
//!
//! A consumer resolves `dyn IncrementRequests` from `ClientHub`; the default
//! deployment binds it in-process inside the registry gear, and
//! `POST /bss-products/v1/catalog-version-requests` is the same contract's
//! out-of-process binding. Both bindings pass the same
//! `catalog_version x request` authorization gate, which is why every method
//! takes the caller's [`SecurityContext`].
//!
//! # The error axis, not the performance axis
//!
//! The transport choice moves *which errors exist*, not how fast the door
//! answers (the lane SLO measures the **version**, not this
//! acknowledgement — P-D-56). [`IncrementRequestError`] therefore separates
//! **not wired** (no binding registered — [`UnconfiguredIncrementRequests`]'
//! arm) from **unreachable** (a transport failure an in-process binding can
//! never produce) from an **unusable answer**, mirroring the projection
//! `bss-pricing-sdk` ships for the opposite direction; a registry refusal
//! rides a `FailedPrecondition` carrying a precondition violation of type
//! [`CATALOG_VERSION_REJECTED`], which is the discriminator the consumer
//! side's `Rejected` arm matches on (P-D-52).
//!
//! @cpt-dod:cpt-cf-bss-products-dod-increment-request-port:p1

use async_trait::async_trait;
use toolkit_canonical_errors::{CanonicalError, resource_error};
use toolkit_security::SecurityContext;
use uuid::Uuid;

/// The canonical-error identity of this contract's own dispositions —
/// the sibling `pricing-sdk` port's pattern, under this registry's own
/// resource id rather than a borrowed one.
#[resource_error(gts_id!("cf.bss.products.catalog_version.v1~"))]
struct IncrementResource;

/// The precondition-violation type a registry refusal carries (P-D-52).
///
/// One spelling, shared with `bss-pricing-sdk`'s constant of the same name:
/// the consumer's `Rejected` arm matches on this string, and a refusal that
/// omits it lands on that projection's `Other` arm — the asymmetry the code
/// was minted to close.
pub const CATALOG_VERSION_REJECTED: &str = "CATALOG_VERSION_REJECTED";

/// The two demand lanes of `design/06`'s D-47 split.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IncrementLane {
    /// Coalesces within seconds of the earliest pending request.
    Interactive,
    /// Held open per `operation_key` until the five-minute hard max.
    Bulk,
}

impl IncrementLane {
    /// The wire and storage spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Interactive => "interactive",
            Self::Bulk => "bulk",
        }
    }
}

/// One increment request, exactly the entity `design/06` §1.7 declares minus
/// `requested_at` — that one is **the door's**, stamped at ingress and never
/// accepted from the caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncrementRequest {
    /// The registered requester this demand belongs to. A source outside the
    /// registered set is refused `REQUEST_SOURCE_UNKNOWN`.
    pub source: String,
    /// Which demand lane the request rides.
    pub lane: IncrementLane,
    /// The caller's idempotency handle: the contract is idempotent per
    /// `(tenant_id, source, request_key)` — the tenant axis because one
    /// source serves many tenants, and a cache keyed on the pair alone would
    /// hand one tenant's pending answer to another.
    pub request_key: String,
    /// The bulk batch this request belongs to; a `Bulk` request MUST name
    /// one so the whole operation coalesces into a single version.
    pub operation_key: Option<String>,
}

/// The door's acknowledgement — what a request or an idempotent replay of
/// one answers. Never the committed version's contents: the assignment is
/// asynchronous by design, and [`IncrementRequests::committed`] is the poll.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncrementAck {
    /// `true` once the request's version has committed; the replayed
    /// acknowledgement of an already-coalesced request.
    pub coalesced: bool,
    /// The committed version, present exactly when `coalesced`.
    pub catalog_version_id: Option<i64>,
}

/// A committed version, as the poll answers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommittedIncrement {
    /// The gapless per-tenant version id.
    pub catalog_version_id: i64,
}

/// The increment-request client — the typed contract a consumer resolves
/// from `ClientHub` rather than an implementation package.
#[async_trait]
pub trait IncrementRequests: Send + Sync {
    /// Enqueue one increment request, or replay the acknowledgement the key
    /// already earned. Synchronous work only: the door stamps
    /// `requested_at`, claims idempotently, enqueues and answers — it takes
    /// no lease and resolves no version (P-D-56).
    ///
    /// # Errors
    /// The canonical projection of the authorization gate's refusal; a
    /// `FailedPrecondition` carrying [`CATALOG_VERSION_REJECTED`] when
    /// `source` is outside the registered set; `VALIDATION` for a shape the
    /// door cannot use.
    async fn request(
        &self,
        ctx: &SecurityContext,
        tenant_id: Uuid,
        request: IncrementRequest,
    ) -> Result<IncrementAck, CanonicalError>;

    /// Resolve a request to its committed version, or `None` while the
    /// coalescer has not yet committed the batch it belongs to. The poll's
    /// surface is this method itself — no HTTP door exists for it (P-D-81).
    ///
    /// # Errors
    /// The canonical projection of the authorization gate's refusal, or a
    /// storage failure.
    async fn committed(
        &self,
        ctx: &SecurityContext,
        tenant_id: Uuid,
        source: &str,
        request_key: &str,
    ) -> Result<Option<CommittedIncrement>, CanonicalError>;
}

/// The projection a consumer classifies a [`CanonicalError`] into — the
/// not-wired / unreachable / unusable-answer axis the transport choice
/// actually moves.
#[derive(Clone, Debug)]
pub enum IncrementRequestError {
    /// No binding is registered (`Unimplemented`): the fail-closed
    /// [`UnconfiguredIncrementRequests`] arm.
    NotWired(String),
    /// The transport failed (`ServiceUnavailable`) — an arm an in-process
    /// binding can never produce.
    Unreachable(String),
    /// The registry itself refused: a `FailedPrecondition` whose violations
    /// carry [`CATALOG_VERSION_REJECTED`], the registry's own sentence as
    /// the description.
    Rejected(String),
    /// Everything else — an answer the caller cannot use, carried whole.
    Other(CanonicalError),
}

impl From<CanonicalError> for IncrementRequestError {
    fn from(err: CanonicalError) -> Self {
        let detail = err.to_string();
        match &err {
            CanonicalError::Unimplemented { .. } => Self::NotWired(detail),
            CanonicalError::ServiceUnavailable { .. } => Self::Unreachable(detail),
            // Matched on the violation type rather than the category alone,
            // for the sibling projection's stated reason: a
            // `FailedPrecondition` can carry something other than the
            // registry's refusal, and folding those onto `Rejected` would
            // hand the caller a refusal the registry never decided.
            CanonicalError::FailedPrecondition { ctx, .. } => ctx
                .violations
                .iter()
                .find(|violation| violation.type_ == CATALOG_VERSION_REJECTED)
                .map_or_else(
                    || Self::Other(err.clone()),
                    |violation| Self::Rejected(violation.description.clone()),
                ),
            _ => Self::Other(err),
        }
    }
}

/// Fail-closed default until a binding is wired: every call answers
/// `Unimplemented`, so a consumer that forgot the wiring stops rather than
/// silently minting demand nowhere.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnconfiguredIncrementRequests;

#[async_trait]
impl IncrementRequests for UnconfiguredIncrementRequests {
    async fn request(
        &self,
        _ctx: &SecurityContext,
        _tenant_id: Uuid,
        _request: IncrementRequest,
    ) -> Result<IncrementAck, CanonicalError> {
        Err(unconfigured())
    }

    async fn committed(
        &self,
        _ctx: &SecurityContext,
        _tenant_id: Uuid,
        _source: &str,
        _request_key: &str,
    ) -> Result<Option<CommittedIncrement>, CanonicalError> {
        Err(unconfigured())
    }
}

fn unconfigured() -> CanonicalError {
    IncrementResource::unimplemented(
        "bss-products: no IncrementRequests binding is registered in ClientHub",
    )
    .create()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The projection's four arms, each fed the canonical shape that owns
    /// it — the not-wired / unreachable / rejected / other axis the `DoD`
    /// requires the taxonomy to separate.
    #[test]
    fn the_projection_separates_the_transport_axis() {
        match IncrementRequestError::from(unconfigured()) {
            IncrementRequestError::NotWired(_) => {}
            other => panic!("Unimplemented must project NotWired, got {other:?}"),
        }

        let unreachable = CanonicalError::service_unavailable()
            .with_detail("transport down")
            .create();
        match IncrementRequestError::from(unreachable) {
            IncrementRequestError::Unreachable(_) => {}
            other => panic!("ServiceUnavailable must project Unreachable, got {other:?}"),
        }

        let rejected = IncrementResource::failed_precondition()
            .with_precondition_violation(
                "source",
                "source \"billing\" is not in the registered requester set",
                CATALOG_VERSION_REJECTED,
            )
            .create();
        match IncrementRequestError::from(rejected) {
            IncrementRequestError::Rejected(sentence) => {
                assert!(
                    sentence.contains("billing"),
                    "the registry's own sentence rides the violation description"
                );
            }
            other => panic!("the discriminator must project Rejected, got {other:?}"),
        }

        // A FailedPrecondition WITHOUT the discriminator is deliberately not
        // folded onto Rejected: the registry could raise the shape for
        // something it never decided as a refusal.
        let foreign = IncrementResource::failed_precondition()
            .with_precondition_violation("x", "unrelated", "SOMETHING_ELSE")
            .create();
        match IncrementRequestError::from(foreign) {
            IncrementRequestError::Other(_) => {}
            other => panic!("a foreign violation type must project Other, got {other:?}"),
        }
    }
}
