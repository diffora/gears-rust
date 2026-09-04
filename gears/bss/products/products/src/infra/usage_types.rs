//! The usage-type collector as the publish door sees it
//! (`dod-usage-type-resolution`; **P-D-131**, **P-D-141**).
//!
//! One question, three answers ([`UsageTypeAnswer`]), asked **once per
//! publish** for the SKU's one `usage_type_ref`, **before** the publish
//! transaction opens — so a `503` leaves no claimed idempotency key and the
//! retry is a fresh act. The judge is the domain's
//! ([`crate::domain::recognized::judge_usage_type`]); this module is the
//! seam that fetches the answer.
//!
//! # Why a trait on `ApiState`, and why not a `cfg(test)` fork
//!
//! The first cut of this seam was a function that answered `Resolved` in the
//! test binary and `Unavailable` in production. That is two programs: every
//! probe exercised a path production never ran, and the production path — a
//! constant refusal — was exercised by nothing. `PiiDetector` had already
//! taken the shape that fixes it (P-D-136): the door reads a trait object off
//! `ApiState`, `gear.rs` installs the real one, tests inject a stub per
//! outcome, and no `cfg(test)` sits in the path.
//!
//! # The three answers, and how the collector's errors become them
//!
//! - `Resolved` — the collector returned the type.
//! - `Unresolved` — the collector answered `NotFound`, **or the ref is not a
//!   valid GTS id** (an id that cannot name anything cannot resolve anywhere;
//!   asking the collector would only rephrase the same `400`).
//! - `Unavailable` — every other error, and a call that outlives
//!   `usage_type_resolver_timeout_ms`: fail-closed, the gear's `503` channel,
//!   for usage SKUs only (P-D-131 — a latency coupling, not a lock).
//!
//! [`NoCollector`] is what a deployment without the collector's client gets:
//! `Unavailable`, always, and `gear.rs` says so once at boot. That keeps the
//! decided posture — a usage SKU cannot publish without a collector — instead
//! of a `Resolved` nobody asked the collector for.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use toolkit_security::SecurityContext;
use usage_collector_sdk::{UsageCollectorClientV1, UsageCollectorError, UsageTypeGtsId};

use crate::domain::recognized::UsageTypeAnswer;

/// The publish door's view of the collector.
#[async_trait]
pub trait UsageTypeResolver: Send + Sync {
    /// Ask whether `usage_type_ref` names a usage type the collector knows.
    async fn resolve(&self, ctx: &SecurityContext, usage_type_ref: &str) -> UsageTypeAnswer;
}

/// No collector is wired: every answer is `Unavailable`, fail-closed
/// (P-D-131). Installed by `gear.rs` when `ClientHub` carries no
/// [`UsageCollectorClientV1`], with a boot-time warning naming this type.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoCollector;

#[async_trait]
impl UsageTypeResolver for NoCollector {
    async fn resolve(&self, _ctx: &SecurityContext, _usage_type_ref: &str) -> UsageTypeAnswer {
        UsageTypeAnswer::Unavailable
    }
}

/// The collector's own client, bounded by the configured timeout.
pub struct CollectorResolver {
    client: Arc<dyn UsageCollectorClientV1>,
    timeout: Duration,
}

impl CollectorResolver {
    /// `timeout` is `ProductsConfig::usage_type_resolver_timeout()` — read,
    /// never inlined (P-D-107, P-D-121 row 12).
    #[must_use]
    pub fn new(client: Arc<dyn UsageCollectorClientV1>, timeout: Duration) -> Self {
        Self { client, timeout }
    }
}

#[async_trait]
impl UsageTypeResolver for CollectorResolver {
    async fn resolve(&self, ctx: &SecurityContext, usage_type_ref: &str) -> UsageTypeAnswer {
        let Ok(gts_id) = UsageTypeGtsId::new(usage_type_ref) else {
            return UsageTypeAnswer::Unresolved;
        };
        match tokio::time::timeout(self.timeout, self.client.get_usage_type(ctx, gts_id)).await {
            Ok(Ok(_)) => UsageTypeAnswer::Resolved,
            Ok(Err(UsageCollectorError::NotFound { .. })) => UsageTypeAnswer::Unresolved,
            Ok(Err(error)) => {
                tracing::warn!(%error, usage_type_ref, "bss-products: usage-type collector failed");
                UsageTypeAnswer::Unavailable
            }
            Err(_elapsed) => {
                tracing::warn!(
                    usage_type_ref,
                    timeout_ms = u64::try_from(self.timeout.as_millis()).unwrap_or(u64::MAX),
                    "bss-products: usage-type collector timed out"
                );
                UsageTypeAnswer::Unavailable
            }
        }
    }
}
