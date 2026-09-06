//! The elevated context is **marked** (review wave 1, P-D-163): the
//! substitution the break-glass gate performs keeps the caller's subject and
//! scopes, swaps the tenant, and adds one scope naming the session — so a
//! door, an audit row or a log line downstream can tell an elevated read from
//! a native principal of the target tenant.

use chrono::Utc;
use uuid::Uuid;

use super::{BREAK_GLASS_SCOPE_PREFIX, breakglass_session_of, elevated_context};
use crate::infra::storage::entity::breakglass_session;

const SESSION: Uuid = Uuid::from_u128(0x5e_55);

fn session(target_tenant: Uuid) -> breakglass_session::Model {
    let now = Utc::now();
    breakglass_session::Model {
        session_id: SESSION,
        principal: Uuid::from_u128(0x5a_b0),
        target_tenant,
        reason: "incident 4471".to_owned(),
        valid_from: now,
        valid_until: now,
        two_person_approval_ref: None,
        approver_a: None,
        approver_b: None,
        posthoc_state: Some("pending_review".to_owned()),
        reviewed_by: None,
        reviewed_at: None,
        posthoc_overdue_alerted_at: None,
        expired_emitted: false,
        opened_at: now,
    }
}

/// **The substituted context is the caller's, under the target tenant, plus
/// the marker.** Every original scope survives, the subject is unchanged,
/// the tenant is the session's target, and the marker names the session —
/// which an ordinary context never carries.
#[test]
fn an_elevated_context_keeps_its_scopes_and_carries_the_session_marker() {
    let home = Uuid::from_u128(0x7e_01);
    let target = Uuid::from_u128(0x7e_02);
    let ctx = crate::test_support::authed_ctx(home);

    let elevated = elevated_context(&ctx, &session(target)).expect("the substitution builds");

    assert_eq!(
        elevated.subject_tenant_id(),
        target,
        "the tenant is the target"
    );
    assert_eq!(
        elevated.subject_id(),
        ctx.subject_id(),
        "the subject is the caller"
    );
    for scope in ctx.token_scopes() {
        assert!(
            elevated.token_scopes().contains(scope),
            "the original scope {scope} survives the substitution"
        );
    }
    assert_eq!(
        elevated.token_scopes().len(),
        ctx.token_scopes().len() + 1,
        "exactly one scope is added"
    );
    assert!(
        elevated
            .token_scopes()
            .iter()
            .any(|s| s.starts_with(BREAK_GLASS_SCOPE_PREFIX)),
        "the marker is prefixed so nothing else can be read as one"
    );
    assert_eq!(breakglass_session_of(&elevated), Some(SESSION));
    assert_eq!(
        breakglass_session_of(&ctx),
        None,
        "an ordinary context carries no marker"
    );
}
