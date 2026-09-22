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
//! [`UnconfiguredUsageTypes`] is what a deployment with no catalog at all gets:
//! `Unavailable` from `resolve`, always, and a **501** from `list` — never an
//! empty page, because "this deployment has no usage types" and "nobody could
//! be asked" are opposite facts. `gear.rs` says so once at boot. That keeps the
//! decided posture — a usage SKU cannot publish without a catalog — instead
//! of a `Resolved` nobody asked for.
//!
//! # The port itself lives in the SDK now
//!
//! [`UsageTypeCatalog`] is `bss_products_sdk::usage_types`'s, so a module that
//! is not this collector can register one and be preferred over the adapter
//! below. This module is the two implementations this crate ships plus the
//! fabricated one a stand may opt into.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use toolkit_security::SecurityContext;
use usage_collector_sdk::{UsageCollectorClientV1, UsageCollectorError, UsageTypeGtsId};

use bss_products_sdk::usage_types::{
    UsageTypeAnswer, UsageTypeBinding, UsageTypeCatalog, UsageTypePage, invalid_usage_type_cursor,
    unconfigured_usage_type_catalog, usage_type_catalog_denied,
    usage_type_catalog_rejected_the_query, usage_type_catalog_unreachable,
};
use toolkit_canonical_errors::CanonicalError;
use toolkit_odata::{CursorV1, ODataQuery, parse_filter_string};

/// No catalog is wired: `resolve` is `Unavailable`, fail-closed (P-D-131), and
/// `list` is a **501**. Installed by `gear.rs` when nothing answers, with a
/// boot-time warning naming this type.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnconfiguredUsageTypes;

#[async_trait]
impl UsageTypeCatalog for UnconfiguredUsageTypes {
    async fn resolve(&self, _ctx: &SecurityContext, _usage_type_ref: &str) -> UsageTypeAnswer {
        UsageTypeAnswer::Unavailable
    }

    async fn list(
        &self,
        _ctx: &SecurityContext,
        _q: Option<&str>,
        _kind: Option<&str>,
        _limit: u32,
        _cursor: Option<&str>,
    ) -> Result<UsageTypePage, CanonicalError> {
        // **Not an empty page.** A caller that cannot tell "no types" from
        // "no catalog" will render silence as a clean answer, which is the
        // failure this whole surface exists to avoid.
        Err(unconfigured_usage_type_catalog())
    }
}

/// The adapter over the usage collector's own client, bounded by the
/// configured timeout.
pub struct CollectorUsageTypes {
    client: Arc<dyn UsageCollectorClientV1>,
    timeout: Duration,
}

impl CollectorUsageTypes {
    /// `timeout` is `ProductsConfig::usage_type_resolver_timeout()` — read,
    /// never inlined (P-D-107, P-D-121 row 12).
    #[must_use]
    pub fn new(client: Arc<dyn UsageCollectorClientV1>, timeout: Duration) -> Self {
        Self { client, timeout }
    }
}

#[async_trait]
impl UsageTypeCatalog for CollectorUsageTypes {
    async fn resolve(&self, ctx: &SecurityContext, usage_type_ref: &str) -> UsageTypeAnswer {
        let Ok(gts_id) = UsageTypeGtsId::new(usage_type_ref) else {
            return UsageTypeAnswer::Unresolved;
        };
        match tokio::time::timeout(self.timeout, self.client.get_usage_type(ctx, gts_id)).await {
            Ok(Ok(usage_type)) => UsageTypeAnswer::Resolved(binding_of(&usage_type)),
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

    async fn list(
        &self,
        ctx: &SecurityContext,
        q: Option<&str>,
        kind: Option<&str>,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<UsageTypePage, CanonicalError> {
        let query = list_query(q, kind, limit, cursor)?;
        // The same deadline `resolve` runs under. A pick-list that hangs is a
        // screen that hangs, and the operator learns nothing either way.
        match tokio::time::timeout(self.timeout, self.client.list_usage_types(ctx, &query)).await {
            Ok(Ok(page)) => Ok(UsageTypePage {
                items: page.items.iter().map(binding_of).collect(),
                next_cursor: page.page_info.next_cursor,
                prev_cursor: page.page_info.prev_cursor,
                // The catalog's own, not the caller's ask: it may have a
                // ceiling this gear does not know.
                limit: u32::try_from(page.page_info.limit).unwrap_or(limit),
            }),
            // **Never an empty page on a failure**, and never one class for
            // every failure. The 503 is what lets a caller tell "this
            // deployment has no usage types" from "the catalog did not
            // answer" - but a denial and a malformed filter are neither, and
            // reporting them as an outage sends an operator to retry forever
            // against a permission problem.
            //
            // **The PDP detail never reaches the wire.** The collector SDK's
            // own `PermissionDenied` doc says it is "kept for operator logs;
            // the host lift drops it from the public wire body", so it is
            // logged here and the caller is told only that authorization was
            // refused.
            Ok(Err(error)) => {
                tracing::warn!(%error, "bss-products: usage-type catalog list failed");
                Err(match error {
                    UsageCollectorError::PermissionDenied { .. } => usage_type_catalog_denied(),
                    UsageCollectorError::InvalidArgument { .. } => {
                        usage_type_catalog_rejected_the_query()
                    }
                    other => usage_type_catalog_unreachable(other.to_string()),
                })
            }
            Err(_elapsed) => Err(usage_type_catalog_unreachable(format!(
                "the usage-type catalog did not answer within {}ms",
                u64::try_from(self.timeout.as_millis()).unwrap_or(u64::MAX)
            ))),
        }
    }
}

/// One collector [`UsageType`](usage_collector_sdk::models::UsageType) as the
/// port's binding.
///
/// **One spelling for both methods.** `resolve` used to build this inline; a
/// second copy for `list` is how a picker comes to disagree with the gate about
/// what a type's `kind` is.
fn binding_of(usage_type: &usage_collector_sdk::models::UsageType) -> UsageTypeBinding {
    UsageTypeBinding {
        gts_id: usage_type.gts_id.to_string(),
        kind: match usage_type.kind {
            usage_collector_sdk::models::UsageKind::Counter => "counter",
            usage_collector_sdk::models::UsageKind::Gauge => "gauge",
        }
        .to_owned(),
        metadata_fields: usage_type
            .metadata_fields
            .iter()
            .map(|key| key.as_str().to_owned())
            .collect(),
    }
}

/// The pick-list's `OData` paging, on `infra::catalog_provider::search_odata`'s
/// terms exactly — the sibling browse walk in this same crate.
///
/// `q` is `contains` rather than `startswith` because a usage type's id is a
/// long GTS path whose distinguishing part is at the **end**
/// (`…usage_type.vcpuhours.v1`), so a prefix search would match everything or
/// nothing. `kind` is equality over the collector's own closed two-value set.
///
/// # Errors
///
/// [`CanonicalError`] when the narrowed filter will not parse or the
/// continuation token is not a `CursorV1`.
fn list_query(
    q: Option<&str>,
    kind: Option<&str>,
    limit: u32,
    cursor: Option<&str>,
) -> Result<ODataQuery, CanonicalError> {
    let mut clauses: Vec<String> = Vec::new();
    if let Some(needle) = q.filter(|s| !s.is_empty()) {
        clauses.push(format!("contains(gts_id,'{}')", needle.replace('\'', "''")));
    }
    if let Some(wanted) = kind.filter(|s| !s.is_empty()) {
        clauses.push(format!("kind eq '{}'", wanted.replace('\'', "''")));
    }
    let mut odata = ODataQuery::new().with_limit(u64::from(limit));
    if !clauses.is_empty() {
        // **A caller's filter is a 400, not a 500.** The donor
        // `catalog_provider::search_odata` raises `internal` because its
        // cursor is one this gear minted for an in-process client; here both
        // operands arrive on a public query string, and paging an on-call for
        // somebody's typo is the wrong answer.
        let parsed = parse_filter_string(&clauses.join(" and "))
            .map_err(|_| usage_type_catalog_rejected_the_query())?;
        odata = odata.with_filter(parsed.into_expr());
    }
    if let Some(token) = cursor.filter(|s| !s.is_empty()) {
        let decoded = CursorV1::decode(token).map_err(|_| invalid_usage_type_cursor())?;
        odata = odata.with_cursor(decoded);
    }
    Ok(odata)
}

/// A **fabricated** usage-type catalog, for a stand with no supplier at all.
///
/// Selected only by an explicit config mode named at length, and warned about
/// at boot: a deployment running this is showing operators usage types no
/// collector issued, and a meter declared against one names a stream nothing
/// will ever report. Every id sits under a reserved prefix so the rows can be
/// found and swept when a real supplier arrives.
#[derive(Debug, Default, Clone, Copy)]
pub struct LocalDevStaticUsageTypes;

/// The reserved namespace every fabricated id sits under.
pub const DEV_LOCAL_USAGE_TYPE_PREFIX: &str =
    "gts.cf.core.uc.usage_record.v1~cf.dev.local.usage_type.";

/// The fabricated set, built once.
///
/// `resolve` is called per SKU on the publish gate, so rebuilding three
/// `String`s on every call was three allocations for a constant.
static FABRICATED: std::sync::LazyLock<Vec<UsageTypeBinding>> = std::sync::LazyLock::new(|| {
    ["cpu.v1", "storage.v1", "requests.v1"]
        .into_iter()
        .map(|leaf| UsageTypeBinding {
            gts_id: format!("{DEV_LOCAL_USAGE_TYPE_PREFIX}{leaf}"),
            kind: "counter".to_owned(),
            metadata_fields: Vec::new(),
        })
        .collect()
});

#[async_trait]
impl UsageTypeCatalog for LocalDevStaticUsageTypes {
    async fn resolve(&self, _ctx: &SecurityContext, usage_type_ref: &str) -> UsageTypeAnswer {
        FABRICATED
            .iter()
            .find(|binding| binding.gts_id == usage_type_ref)
            .cloned()
            .map_or(UsageTypeAnswer::Unresolved, UsageTypeAnswer::Resolved)
    }

    async fn list(
        &self,
        _ctx: &SecurityContext,
        q: Option<&str>,
        kind: Option<&str>,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<UsageTypePage, CanonicalError> {
        // **A cursor is refused, not ignored.** The whole fabricated set fits
        // one page, so this type never mints one - but only the minting half
        // is under its control, and the port's own `list` doc says an
        // implementation "must not answer a page it did not narrow as though
        // it had". Handing page one back to a caller that asked to continue is
        // exactly that.
        if cursor.is_some_and(|token| !token.is_empty()) {
            return Err(invalid_usage_type_cursor());
        }
        // Narrowing is applied so a screen driving this mode behaves as it
        // will against a real catalog.
        let items = FABRICATED
            .iter()
            .filter(|b| {
                q.filter(|s| !s.is_empty())
                    .is_none_or(|n| b.gts_id.contains(n))
            })
            .filter(|b| kind.filter(|s| !s.is_empty()).is_none_or(|k| b.kind == k))
            .take(limit as usize)
            .cloned()
            .collect();
        Ok(UsageTypePage {
            items,
            next_cursor: None,
            prev_cursor: None,
            limit,
        })
    }
}

#[cfg(test)]
#[path = "usage_types_tests.rs"]
mod usage_types_tests;
