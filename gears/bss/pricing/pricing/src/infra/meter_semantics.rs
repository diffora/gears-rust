//! Detached semantic observations and transaction-local checks of their entry identities.
use crate::{
    api::rest::authoring::support::{self, DoorError},
    domain::usage_policy::validate_meter_policy,
    infra::{
        reference_registry,
        storage::{entity::price_book_entry, repo::usage_policy_repo},
        usage_policy_wire::{MeterEvidence, UsageRatingPolicyInput},
    },
};
use bss_pricing_sdk::{meter_semantics::UsageMeterSemanticsV1, terms::UsageRatingPolicy};
use bss_products_sdk::models::Sku;
use std::collections::BTreeMap;
use toolkit_canonical_errors::CanonicalError;
use toolkit_db::DbConn;
use toolkit_security::SecurityContext;
use uuid::Uuid;

/// Resolve a provider or report E1 unconfigured. No production default is supplied.
/// # Errors
/// Unconfigured dependency, configured outage or the provider's definite refusal.
pub async fn resolve(
    hub: &toolkit::ClientHub,
    ctx: &SecurityContext,
    input: &UsageRatingPolicyInput,
    sku: &Sku,
) -> Result<MeterEvidence, CanonicalError> {
    let provider = hub.get::<dyn UsageMeterSemanticsV1>().map_err(|_| {
        CanonicalError::from(bss_pricing_sdk::meter_semantics::UnconfiguredMeterSemantics)
    })?;
    let content = input.into();
    let policy = UsageRatingPolicy {
        policy_id: Uuid::nil(),
        version: 1,
        digest: bss_pricing_sdk::digest::policy_digest(&content),
        content,
    };
    let evidence = provider
        .resolve(ctx, policy.content.quantity_semantics.meter.clone())
        .await
        .map_err(|e| {
            if e.status_code() >= 500 {
                CanonicalError::service_unavailable().create()
            } else {
                e
            }
        })?;
    validate(&policy, sku, &evidence)?;
    Ok(evidence.into())
}
/// Verify SKU identity and complete immutable evidence without dependency calls.
/// # Errors
/// A mismatching SKU or declaration is a typed input refusal.
pub fn validate(
    policy: &UsageRatingPolicy,
    sku: &Sku,
    evidence: &bss_pricing_sdk::meter_semantics::MeterSemantics,
) -> Result<(), CanonicalError> {
    if sku.usage_type_ref.as_deref()
        != Some(
            policy
                .content
                .quantity_semantics
                .meter
                .usage_type_id
                .as_str(),
        )
    {
        return Err(support::invalid(
            "usage_rating_policy",
            "METER_POLICY_MISMATCH",
        ));
    }
    validate_meter_policy(policy, sku.unit.as_deref().unwrap_or_default(), evidence)
        .map_err(|e| support::invalid("usage_rating_policy", e.code))
}
/// One entry observation. The transaction must still hold precisely this entry generation.
#[derive(Clone)]
pub struct ObservedEntry {
    entry: price_book_entry::Model,
    pub policy: super::usage_policy_wire::UsageRatingPolicy,
    pub evidence: MeterEvidence,
}
/// Observations are request-local and never used by historical reads.
#[derive(Clone, Default)]
pub struct Observations {
    entries: BTreeMap<Uuid, Result<ObservedEntry, CanonicalError>>,
    skus: BTreeMap<Uuid, Result<Sku, CanonicalError>>,
    versions: BTreeMap<Uuid, Result<Vec<bss_products_sdk::models::SkuVersion>, CanonicalError>>,
}
impl Observations {
    /// Read local policy rows, then capture dependency results outside a transaction. Errors
    /// remain typed observations until submit/apply; non-final votes and rejects need no gate.
    /// # Errors
    /// Local storage failures; dependency failures are retained for the subject's gate.
    pub async fn capture(
        conn: &DbConn<'_>,
        hub: &toolkit::ClientHub,
        ctx: &SecurityContext,
        entries: Vec<price_book_entry::Model>,
        extra_skus: Vec<Uuid>,
    ) -> Result<Self, DoorError> {
        let policies =
            usage_policy_repo::for_entries(conn, ctx.subject_tenant_id(), &entries).await?;
        let mut result = Self::default();
        let ids = entries
            .iter()
            .map(|e| e.sku_id)
            .chain(extra_skus)
            .collect::<std::collections::BTreeSet<_>>();
        for id in ids {
            let read = match reference_registry::resolve(hub) {
                Ok(registry) => registry
                    .sku_for_write(ctx, ctx.subject_tenant_id(), id)
                    .await
                    .map_err(|e| {
                        if crate::infra::reference_work::definite_refusal(&e) {
                            e
                        } else {
                            support::registry_unavailable(&e)
                        }
                    }),
                Err(e) => Err(support::registry_unavailable(&e)),
            };
            result.skus.insert(id, read);
        }
        for entry in entries.into_iter().filter(|e| e.charge_kind == "usage") {
            if let std::collections::btree_map::Entry::Vacant(slot) =
                result.versions.entry(entry.sku_id)
            {
                slot.insert(history(hub, ctx, entry.sku_id).await);
            }
            let observed = async {
                let policy = policies.get(&entry.id).ok_or_else(|| {
                    support::invalid("usage_rating_policy", "MISSING_RATING_POLICY")
                })?;
                let sku = result
                    .skus
                    .get(&entry.sku_id)
                    .ok_or_else(|| support::conflict("METER_EVIDENCE_CHANGED"))?
                    .as_ref()
                    .map_err(Clone::clone)?;
                let evidence = resolve(hub, ctx, &policy.content, sku).await?;
                Ok(ObservedEntry {
                    entry: entry.clone(),
                    policy: policy.clone(),
                    evidence,
                })
            }
            .await;
            result.entries.insert(entry.id, observed);
        }
        Ok(result)
    }
    /// Recheck local identity before using detached evidence in submit or apply.
    /// # Errors
    /// Missing policy, dependency failure or changed local identity refuses publication.
    pub fn check(&self, entry: &price_book_entry::Model) -> Result<(), CanonicalError> {
        if entry.charge_kind != "usage" {
            return Ok(());
        }
        if entry.usage_policy_id.is_none() {
            return Err(support::invalid(
                "usage_rating_policy",
                "MISSING_RATING_POLICY",
            ));
        }
        let observed = self
            .entries
            .get(&entry.id)
            .ok_or_else(|| support::conflict("METER_EVIDENCE_CHANGED"))?
            .as_ref()
            .map_err(Clone::clone)?;
        if &observed.entry != entry {
            return Err(support::conflict("METER_EVIDENCE_CHANGED"));
        }
        Ok(())
    }
    /// Fresh caller-authorized SKU observations, consumed without a dependency call.
    /// # Errors
    /// The original dependency refusal, or missing evidence after a local selection change.
    pub fn skus(&self, ids: impl IntoIterator<Item = Uuid>) -> Result<Vec<Sku>, CanonicalError> {
        ids.into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .map(|id| {
                self.skus
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| Err(support::conflict("METER_EVIDENCE_CHANGED")))
            })
            .collect()
    }
    /// Dated metering from immutable history captured before the transaction.
    /// # Errors
    /// Original provider refusal, or missing capture for a changed entry.
    pub fn metering(
        &self,
        sku: Uuid,
        on: time::Date,
    ) -> Result<crate::domain::price::SkuMetering, CanonicalError> {
        let versions = self
            .versions
            .get(&sku)
            .ok_or_else(|| support::conflict("METER_EVIDENCE_CHANGED"))?
            .as_ref()
            .map_err(Clone::clone)?;
        let version = versions
            .iter()
            .find(|v| v.effective_from <= on)
            .or_else(|| versions.last());
        Ok(crate::domain::price::SkuMetering {
            unit: version.and_then(|v| v.content.unit.clone()),
            usage_type_ref: version.and_then(|v| v.content.usage_type_ref.clone()),
        })
    }
    /// Provider evidence retained outside the fingerprinted business content.
    #[must_use]
    pub fn audit(&self) -> serde_json::Value {
        serde_json::Value::Array(
            self.entries
                .iter()
                .filter_map(|(id, e)| {
                    e.as_ref().ok().map(|e| {
                        serde_json::json!({
                            "price_book_entry_id":id, "policy_id":e.policy.policy_id,
                            "policy_version":e.policy.version, "policy_digest":e.policy.digest,
                            "meter_evidence":e.evidence
                        })
                    })
                })
                .collect(),
        )
    }
}
/// Capture exact dated history once; never ask the registry while judging a DB transaction.
async fn history(
    hub: &toolkit::ClientHub,
    ctx: &SecurityContext,
    sku: Uuid,
) -> Result<Vec<bss_products_sdk::models::SkuVersion>, CanonicalError> {
    let registry =
        reference_registry::resolve(hub).map_err(|e| support::registry_unavailable(&e))?;
    let mut result = Vec::new();
    let mut on = time::Date::MAX;
    while let Some(version) = registry
        .sku_version_as_of(ctx, ctx.subject_tenant_id(), sku, on)
        .await?
    {
        let previous = version.effective_from.previous_day();
        if version.effective_from > on {
            return Err(CanonicalError::internal("invalid SKU version history").create());
        }
        result.push(version);
        let Some(previous) = previous else {
            break;
        };
        on = previous;
    }
    Ok(result)
}
