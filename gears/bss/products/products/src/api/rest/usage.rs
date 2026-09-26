//! Pricing's usage of the SKUs a read returns, through the port pricing fills (P-D-197).
//!
//! Information for the SKUs screen, and never a reason for the read to fail: an absent port, a
//! refusal (the caller holds no pricing `price_book_entry:read`), any error, and a call that does
//! not finish all leave `usage: null`. The port is called once per read, on a task of its own,
//! after the read's own work and outside any transaction of this gear, so a port that opens its
//! own connection or breaks cannot disturb the read; a call still running after [`PORT_BOUND`] is
//! aborted. The usage never takes part in a fence, a retirement or a type change (P-D-188,
//! P-D-194).
use super::{ApiState, dto::SkuUsageDto};
use bss_products_sdk::sku_usage::{SkuUsage, SkuUsageV1};
use std::{collections::BTreeMap, time::Duration};
use toolkit::api::canonical_prelude::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

/// How long a SKU read waits for the port: past it the call has not finished (P-D-197), and the
/// read answers `usage: null`.
const PORT_BOUND: Duration = Duration::from_secs(2);
/// Pricing's usage of `ids`, by SKU, from one call of the port resolved now (the two gears boot
/// in either order).
pub async fn of(
    state: &ApiState,
    ctx: &SecurityContext,
    ids: &[Uuid],
) -> BTreeMap<Uuid, SkuUsageDto> {
    if ids.is_empty() {
        return BTreeMap::new();
    }
    let Ok(port) = state.hub.get::<dyn SkuUsageV1>() else {
        return BTreeMap::new();
    };
    let (caller, tenant, asked) = (ctx.clone(), ctx.subject_tenant_id(), ids.to_vec());
    let mut call = tokio::spawn(async move { port.usage(&caller, tenant, &asked).await });
    let Ok(answer) = tokio::time::timeout(PORT_BOUND, &mut call).await else {
        call.abort();
        tracing::warn!(
            bound = ?PORT_BOUND,
            "bss-products: the SKU usage port did not answer in time; usage is null"
        );
        return BTreeMap::new();
    };
    by_sku(ids, answer)
}
/// The answer by SKU: only the ids asked, the first answer of each; nothing when the port did not
/// answer.
fn by_sku(
    ids: &[Uuid],
    answer: Result<Result<Vec<SkuUsage>, CanonicalError>, tokio::task::JoinError>,
) -> BTreeMap<Uuid, SkuUsageDto> {
    let rows = match answer {
        Ok(Ok(rows)) => rows,
        Ok(Err(error)) => {
            unanswered(&error);
            return BTreeMap::new();
        }
        Err(error) => {
            tracing::warn!(%error, "bss-products: the SKU usage port did not finish; usage is null");
            return BTreeMap::new();
        }
    };
    let mut usage = BTreeMap::new();
    for row in rows {
        if ids.contains(&row.sku_id) {
            usage
                .entry(row.sku_id)
                .or_insert_with(|| SkuUsageDto::from(row));
        }
    }
    usage
}
/// A refusal is routine (a caller without pricing read); anything else is pricing failing.
fn unanswered(error: &CanonicalError) {
    if error.status_code() == 403 {
        tracing::debug!(%error, "bss-products: pricing refused the SKU usage; usage is null");
    } else {
        tracing::warn!(%error, "bss-products: pricing could not answer the SKU usage; usage is null");
    }
}
