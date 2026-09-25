//! Actor, timestamp and request identity for future audited mutations.

use time::OffsetDateTime;
use toolkit_security::SecurityContext;
use uuid::Uuid;

/// The identity shared by every record produced by one mutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuditStamp {
    /// Pseudonymous authenticated actor.
    pub actor_principal_id: Uuid,
    /// Caller-supplied recording instant.
    pub recorded_at: OffsetDateTime,
    /// Identity established once at the request edge.
    pub correlation_id: Uuid,
}

/// Carry the authenticated actor and the request's time and correlation unchanged.
#[must_use]
pub fn audit_stamp(ctx: &SecurityContext, now: OffsetDateTime, correlation: Uuid) -> AuditStamp {
    AuditStamp {
        actor_principal_id: ctx.subject_id(),
        recorded_at: now,
        correlation_id: correlation,
    }
}
