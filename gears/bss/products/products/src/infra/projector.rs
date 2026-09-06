//! The `ReadProjector` — `design/08` §2's write→read pipeline
//! (`inst-rp-consume`, `inst-rp-stamp`, `inst-rp-bootstrap`,
//! `inst-rp-reparent`) and the three polled dashboards
//! (`inst-ps-dashboards`), P-D-150.
//!
//! # The inbox is the source
//!
//! Every consumed family writes its event to `products_read_inbox` in the
//! transaction that wrote the outbox row (`infra::events::record_inbox`), so
//! the projector walks a gear-owned, tenant-ordered copy whose `created_at`
//! is the commit instant — P-D-124's origin for the convergence meter. The
//! checkpoint is per tenant (`design/08`'s "per partition", every inbox row
//! being one tenant's); a checkpoint the swept tail has run past rebuilds
//! from the latest catalog version into a shadow generation and swaps
//! (P-D-126 row 8), serving the old generation until cutover.
//!
//! # The three-source read path (`dod-frozen-read-path`)
//!
//! Product and SKU content is rendered from the **frozen** version row the
//! event names (`repo::entity_version_at`), never the head; the three
//! head-read columns of `01` §4.3's carve-out — `lifecycle_state`,
//! `deprecation_provenance`, `replaced_by_sku_id` — come from the head; the
//! governed live entities (categories, definitions, recognized sets) are
//! read live, having no draft to leak.
//!
//! # Poison (P-D-126 rows 9 and 12)
//!
//! A `*Published` whose frozen row is gone, or a row whose payload does not
//! decode, is **parked**: retried each pass up to the configured ceiling,
//! then skipped with `read_model_poison` raised, surfaced through the
//! delivery-state dashboard. A pass never silently halts a tenant.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-projector:p1
//! @cpt-dod:cpt-cf-bss-products-dod-frozen-read-path:p1

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde_json::Value as JsonValue;
use toolkit_db::secure::AccessScope;
use uuid::Uuid;

use crate::domain::read_model::{StampApply, StampCatalogTouch};
use crate::infra::events;
use crate::infra::storage::RepoError;
use crate::infra::storage::repo::{self, ReadEntityRow};

/// `08`'s knobs, read once from `ProductsConfig` at boot.
#[derive(Debug, Clone)]
pub struct ReadKnobs {
    /// The per-tenant read ceiling the limiter enforces.
    pub qps_ceiling: u32,
    /// How many passes retry a poison row before it is skipped and alarmed.
    pub poison_retry_ceiling: u32,
    /// The commit-to-projected budget, seconds.
    pub convergence_budget_secs: u32,
    /// The polled dashboards' cadence, seconds.
    pub dashboard_poll_secs: u32,
    /// Consumed inbox rows are kept this long for replay.
    pub inbox_retention_hours: u32,
    /// The locales `display_attributes` are materialised for, in order.
    pub active_locales: Vec<String>,
}

impl From<&crate::config::ProductsConfig> for ReadKnobs {
    fn from(cfg: &crate::config::ProductsConfig) -> Self {
        Self {
            qps_ceiling: cfg.read_path_qps_ceiling,
            poison_retry_ceiling: cfg.read_poison_retry_ceiling,
            convergence_budget_secs: cfg.read_convergence_budget_secs,
            dashboard_poll_secs: cfg.read_dashboard_poll_secs,
            inbox_retention_hours: cfg.read_inbox_retention_hours,
            active_locales: cfg.read_active_locales.clone(),
        }
    }
}

/// What one projector pass needs.
pub(crate) struct ProjectorContext {
    pub(crate) db: toolkit_db::DBProvider<toolkit_db::DbError>,
    pub(crate) knobs: ReadKnobs,
}

impl ProjectorContext {
    /// Where this pass reads its events from. Today the gear-owned inbox
    /// (P-D-150); the day the platform ships a broker consumer, the swap
    /// is a second [`InboxSource`] implementor returned here — the
    /// projection above it does not move (P-D-161).
    pub(crate) fn source(&self) -> RepoInbox {
        RepoInbox {
            db: self.db.clone(),
        }
    }
}

/// The seam between the projector and whatever delivers its events: the
/// bounds of a tenant's undelivered tail and a batch above a checkpoint. The
/// inbox is the first implementor; a broker consumer is the second, and
/// "replace the hook, not the projection" (P-D-150) is this trait rather than
/// a sentence.
#[async_trait::async_trait]
pub(crate) trait InboxSource: Send + Sync {
    /// The first and last event ids a tenant holds, `None` when it holds
    /// nothing.
    async fn bounds(&self, tenant_id: Uuid) -> Result<Option<(i64, i64)>, RepoError>;
    /// Up to `batch` events above `checkpoint`, in delivery order.
    async fn after(
        &self,
        tenant_id: Uuid,
        checkpoint: i64,
        batch: u64,
    ) -> Result<Vec<repo::InboxRow>, RepoError>;
}

/// The gear-owned inbox as the source: `products_read_inbox`, written in the
/// producing transaction (`infra::events::record_inbox`).
pub(crate) struct RepoInbox {
    db: toolkit_db::DBProvider<toolkit_db::DbError>,
}

#[async_trait::async_trait]
impl InboxSource for RepoInbox {
    async fn bounds(&self, tenant_id: Uuid) -> Result<Option<(i64, i64)>, RepoError> {
        let conn = self
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("inbox source connection: {e}")))?;
        repo::inbox_bounds(&conn, &AccessScope::for_tenant(tenant_id), tenant_id).await
    }

    async fn after(
        &self,
        tenant_id: Uuid,
        checkpoint: i64,
        batch: u64,
    ) -> Result<Vec<repo::InboxRow>, RepoError> {
        let conn = self
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("inbox source connection: {e}")))?;
        repo::inbox_after(
            &conn,
            &AccessScope::for_tenant(tenant_id),
            tenant_id,
            checkpoint,
            batch,
        )
        .await
    }
}

/// Rows read per tenant per pass.
const INBOX_BATCH: u64 = 200;

/// What one pass did to a tenant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PassOutcome {
    /// Nothing above the checkpoint.
    Idle,
    /// Events applied; poison rows parked (and skipped once past the ceiling).
    Projected { applied: usize, parked: usize },
    /// The checkpoint had fallen behind the swept tail: rebuilt into a new
    /// generation and swapped.
    Rebuilt { rows: usize, generation: i64 },
}

enum ApplyError {
    /// The row cannot be applied and will not become applicable: parked.
    Poison(String),
    Repo(RepoError),
}

impl From<RepoError> for ApplyError {
    fn from(error: RepoError) -> Self {
        Self::Repo(error)
    }
}

/// One projector pass over every tenant holding inbox rows.
///
/// # Errors
///
/// The last tenant's [`RepoError`] when every tenant's pass failed; a single
/// failing tenant is logged and the others continue.
pub(crate) async fn sweep(
    ctx: &ProjectorContext,
    now: DateTime<Utc>,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<(), RepoError> {
    let tenants = {
        let conn = ctx
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("projector discovery connection: {e}")))?;
        repo::tenants_with_inbox(&conn, &AccessScope::allow_all()).await?
    };
    let total = tenants.len();
    let mut failed = 0_usize;
    let mut last_err = None;
    for tenant in tenants {
        if cancel.is_cancelled() {
            return Ok(());
        }
        if let Err(e) = project_tenant(ctx, tenant, now).await {
            failed += 1;
            tracing::error!(%tenant, error = %e, "bss-products: read projection pass failed");
            last_err = Some(e);
        }
    }
    match last_err {
        Some(e) if failed == total => Err(e),
        _ => Ok(()),
    }
}

/// Project one tenant's inbox above its checkpoint (`inst-rp-consume`).
///
/// # Errors
///
/// [`RepoError`] on a storage failure below the domain.
#[allow(clippy::cognitive_complexity)] // one pass: the gap, the park, the apply, the checkpoint
pub(crate) async fn project_tenant(
    ctx: &ProjectorContext,
    tenant_id: Uuid,
    now: DateTime<Utc>,
) -> Result<PassOutcome, RepoError> {
    let scope = AccessScope::for_tenant(tenant_id);
    let conn = ctx
        .db
        .conn()
        .map_err(|e| RepoError::Db(format!("projector connection: {e}")))?;
    let (mut checkpoint, generation) = repo::load_read_checkpoint(&conn, &scope, tenant_id)
        .await?
        .unwrap_or((0, 0));
    // A checkpoint the swept tail has run past cannot resume: rebuild
    // (`inst-rp-bootstrap`), the old generation serving until the swap.
    let source = ctx.source();
    if let Some((first, last)) = source.bounds(tenant_id).await?
        && checkpoint > 0
        && first > checkpoint + 1
    {
        let rows = rebuild_tenant(ctx, &conn, &scope, tenant_id, generation + 1, last, now).await?;
        return Ok(PassOutcome::Rebuilt {
            rows,
            generation: generation + 1,
        });
    }
    let rows = source.after(tenant_id, checkpoint, INBOX_BATCH).await?;
    if rows.is_empty() {
        return Ok(PassOutcome::Idle);
    }
    let budget = ChronoDuration::seconds(i64::from(ctx.knobs.convergence_budget_secs));
    let mut applied = 0usize;
    let mut parked = 0usize;
    let mut touched_entities = false;
    for row in rows {
        match apply_event(ctx, &conn, &scope, tenant_id, &row, generation, now).await {
            Ok(outcome) => {
                touched_entities |= outcome.touched_entities;
                if let Some(version) = outcome.catalog_version {
                    advance_stamp(
                        &conn,
                        &scope,
                        tenant_id,
                        StampCatalogTouch::Set(version),
                        outcome.entities_projected,
                        now,
                    )
                    .await?;
                }
                let latency = now.signed_duration_since(row.created_at);
                tracing::info!(
                    event = "read_model_convergence",
                    %tenant_id,
                    payload_type = %row.payload_type,
                    latency_ms = latency.num_milliseconds(),
                    "bss-products: commit -> projected"
                );
                if latency > budget {
                    tracing::warn!(
                        event = "read_model_lag",
                        %tenant_id,
                        lag_secs = latency.num_seconds(),
                        budget_secs = ctx.knobs.convergence_budget_secs,
                        "bss-products: read model behind its convergence budget; serving continues"
                    );
                }
                applied += 1;
                checkpoint = row.inbox_id;
            }
            Err(ApplyError::Poison(reason)) => {
                let attempts = repo::park_poison(
                    &conn,
                    &scope,
                    tenant_id,
                    row.inbox_id,
                    &row.payload_type,
                    &reason,
                    now,
                )
                .await?;
                parked += 1;
                if u32::try_from(attempts).unwrap_or(u32::MAX) >= ctx.knobs.poison_retry_ceiling {
                    tracing::warn!(
                        event = "read_model_poison",
                        %tenant_id,
                        inbox_id = row.inbox_id,
                        payload_type = %row.payload_type,
                        attempts,
                        reason = %reason,
                        "bss-products: poison message parked past its retry ceiling; skipped"
                    );
                    checkpoint = row.inbox_id;
                    continue;
                }
                // Below the ceiling the pass stops here: the row is retried
                // next tick, ordering per tenant intact.
                break;
            }
            Err(ApplyError::Repo(error)) => return Err(error),
        }
    }
    if touched_entities {
        advance_stamp(
            &conn,
            &scope,
            tenant_id,
            StampCatalogTouch::Unchanged,
            true,
            now,
        )
        .await?;
    }
    repo::write_read_checkpoint(&conn, &scope, tenant_id, checkpoint, generation, now).await?;
    Ok(PassOutcome::Projected { applied, parked })
}

/// Advance the tenant's stamp; a refusal (`projected_at` not moving inside
/// one instant, or a version whose entities are not yet projected) is not a
/// failure of the pass — the next event advances it.
async fn advance_stamp(
    conn: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    catalog: StampCatalogTouch,
    entities_projected: bool,
    now: DateTime<Utc>,
) -> Result<(), RepoError> {
    match repo::apply_read_stamp(
        conn,
        scope,
        tenant_id,
        StampApply {
            catalog,
            projected_at: now,
            entities_projected,
        },
    )
    .await
    {
        Ok(_) => Ok(()),
        Err(RepoError::Db(detail)) if detail.contains("stamp") => {
            tracing::debug!(%tenant_id, detail, "bss-products: stamp not advanced this pass");
            Ok(())
        }
        Err(other) => Err(other),
    }
}

struct Applied {
    touched_entities: bool,
    catalog_version: Option<i64>,
    entities_projected: bool,
}

const NOTHING: Applied = Applied {
    touched_entities: false,
    catalog_version: None,
    entities_projected: true,
};

fn json_uuid(data: &JsonValue, key: &str) -> Option<Uuid> {
    data.get(key)?
        .as_str()
        .and_then(|raw| Uuid::parse_str(raw).ok())
}

fn json_i64(data: &JsonValue, key: &str) -> Option<i64> {
    data.get(key)?.as_i64()
}

fn json_str<'a>(data: &'a JsonValue, key: &str) -> Option<&'a str> {
    data.get(key)?.as_str()
}

/// Apply one inbox row.
#[allow(clippy::too_many_lines)] // one arm per consumed family, in the roster's order
async fn apply_event(
    ctx: &ProjectorContext,
    conn: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    row: &repo::InboxRow,
    generation: i64,
    now: DateTime<Utc>,
) -> Result<Applied, ApplyError> {
    let data: JsonValue = serde_json::from_str(&row.payload)
        .map_err(|e| ApplyError::Poison(format!("payload does not decode: {e}")))?;
    let kind_of = |data: &JsonValue| {
        json_str(data, "entityKind")
            .map(str::to_owned)
            .ok_or_else(|| ApplyError::Poison("entityKind missing".to_owned()))
    };
    match row.payload_type.as_str() {
        events::PRODUCT_PUBLISHED_PAYLOAD_TYPE | events::SKU_PUBLISHED_PAYLOAD_TYPE => {
            let kind = kind_of(&data)?;
            let entity_id = json_uuid(&data, "entityId")
                .ok_or_else(|| ApplyError::Poison("entityId missing".to_owned()))?;
            let version = json_i64(&data, "publishedVersion")
                .ok_or_else(|| ApplyError::Poison("publishedVersion missing".to_owned()))?;
            project_entity(
                ctx, conn, scope, tenant_id, &kind, entity_id, version, generation, now,
            )
            .await?;
            Ok(Applied {
                touched_entities: true,
                catalog_version: None,
                entities_projected: true,
            })
        }
        events::SKU_COMPOSITION_CLEARED_PAYLOAD_TYPE
        | events::SKU_IMMUTABLE_FIELD_CORRECTED_PAYLOAD_TYPE => {
            let entity_id = json_uuid(&data, "entityId")
                .ok_or_else(|| ApplyError::Poison("entityId missing".to_owned()))?;
            // The clear and the correction re-publish: the row follows the
            // newest frozen version, whatever the event carried.
            let latest = repo::latest_entity_version(
                conn,
                scope,
                tenant_id,
                repo::VersionedEntityKind::Sku,
                entity_id,
            )
            .await?;
            let Some((version, _)) = latest else {
                return Err(ApplyError::Poison(format!(
                    "sku {entity_id} has no frozen version"
                )));
            };
            project_entity(
                ctx, conn, scope, tenant_id, "sku", entity_id, version, generation, now,
            )
            .await?;
            Ok(Applied {
                touched_entities: true,
                catalog_version: None,
                entities_projected: true,
            })
        }
        events::PRODUCT_DISCARDED_PAYLOAD_TYPE | events::SKU_DISCARDED_PAYLOAD_TYPE => {
            let kind = kind_of(&data)?;
            if let Some(entity_id) = json_uuid(&data, "entityId") {
                repo::delete_read_entity(conn, scope, tenant_id, &kind, entity_id).await?;
            }
            Ok(Applied {
                touched_entities: true,
                catalog_version: None,
                entities_projected: true,
            })
        }
        events::PRODUCT_DEPRECATED_PAYLOAD_TYPE
        | events::SKU_DEPRECATED_PAYLOAD_TYPE
        | events::PRODUCT_UNDEPRECATED_PAYLOAD_TYPE
        | events::SKU_UNDEPRECATED_PAYLOAD_TYPE
        | events::PRODUCT_RETIRED_PAYLOAD_TYPE
        | events::SKU_RETIRED_PAYLOAD_TYPE
        | events::SKU_RETIREMENT_EFFECTIVE_PAYLOAD_TYPE
        | events::PRODUCT_RETIREMENT_EFFECTIVE_PAYLOAD_TYPE => {
            let kind = kind_of(&data)?;
            let entity_id = json_uuid(&data, "entityId")
                .ok_or_else(|| ApplyError::Poison("entityId missing".to_owned()))?;
            refresh_head_fields(conn, scope, tenant_id, &kind, entity_id, now).await?;
            Ok(Applied {
                touched_entities: true,
                catalog_version: None,
                entities_projected: true,
            })
        }
        events::CATALOG_VERSION_PUBLISHED_PAYLOAD_TYPE => {
            let Some(version) = json_i64(&data, "catalogVersionId") else {
                return Ok(NOTHING);
            };
            // The stamp is a floor (P-D-07): the version advances it only once
            // every changed entity it names is projected at or above the
            // version it froze.
            let mut all_projected = true;
            if let Some(changed) = data.get("changedEntities").and_then(JsonValue::as_array) {
                for entity in changed {
                    let (Some(kind), Some(id), Some(published)) = (
                        json_str(entity, "entityKind"),
                        json_uuid(entity, "entityId"),
                        json_i64(entity, "publishedVersion"),
                    ) else {
                        continue;
                    };
                    let projected = repo::find_read_entity(conn, scope, tenant_id, kind, id)
                        .await?
                        .is_some_and(|row| row.published_version >= published);
                    all_projected &= projected;
                }
            }
            Ok(Applied {
                touched_entities: false,
                catalog_version: Some(version),
                entities_projected: all_projected,
            })
        }
        events::CATEGORY_RENAMED_PAYLOAD_TYPE
        | events::CATEGORY_REPARENTED_PAYLOAD_TYPE
        | events::CATEGORY_RETIRED_PAYLOAD_TYPE
        | events::CATEGORY_DELETED_PAYLOAD_TYPE
        | events::CATEGORY_DISPLAY_UPDATED_PAYLOAD_TYPE => {
            // The affected subtree is every Product row whose paths carry the
            // category; recomputing all of the tenant's paths from the live
            // tree is bounded by 02's depth and children limits
            // (`inst-rp-reparent`) and needs no per-row diff.
            refresh_category_paths(conn, scope, tenant_id, generation, now).await?;
            Ok(Applied {
                touched_entities: true,
                catalog_version: None,
                entities_projected: true,
            })
        }
        events::ATTRIBUTE_DEFINITION_UPDATED_PAYLOAD_TYPE
        | events::PLAN_TIER_UPDATED_PAYLOAD_TYPE => {
            refresh_display_fields(ctx, conn, scope, tenant_id, generation, now).await?;
            Ok(Applied {
                touched_entities: true,
                catalog_version: None,
                entities_projected: true,
            })
        }
        _ => Ok(NOTHING),
    }
}

/// Render the browse paths of one Product from the live tree: every assigned
/// category, primary and secondary alike (`inst-rb-facets`), as
/// `Root > Child` strings, sorted, JSON-encoded.
fn category_paths_for(
    assignments: &[Uuid],
    nodes: &BTreeMap<Uuid, (Option<Uuid>, String)>,
) -> Option<String> {
    let mut paths = BTreeSet::new();
    for category in assignments {
        let mut segments = Vec::new();
        let mut cursor = Some(*category);
        let mut hops = 0;
        while let Some(id) = cursor {
            let Some((parent, name)) = nodes.get(&id) else {
                break;
            };
            segments.push(name.clone());
            cursor = *parent;
            hops += 1;
            if hops > 64 {
                break;
            }
        }
        if !segments.is_empty() {
            segments.reverse();
            paths.insert(segments.join(" > "));
        }
    }
    if paths.is_empty() {
        None
    } else {
        Some(serde_json::json!(paths.into_iter().collect::<Vec<_>>()).to_string())
    }
}

/// The display attributes of one entity for the active locales: per locale,
/// the definition key to its value, first match by the configured order.
async fn display_attributes_for(
    ctx: &ProjectorContext,
    conn: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    entity_kind: &str,
    entity_id: Uuid,
) -> Result<Option<String>, RepoError> {
    let definitions = repo::attribute_definitions(conn, scope, tenant_id).await?;
    let keys: BTreeMap<Uuid, String> = definitions
        .into_iter()
        .map(|d| (d.definition_id, d.key))
        .collect();
    let values = repo::attribute_values_of(conn, scope, tenant_id, entity_kind, entity_id).await?;
    let mut per_locale: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for locale in &ctx.knobs.active_locales {
        let mut map = BTreeMap::new();
        for value in values.iter().filter(|v| &v.locale == locale) {
            if let Some(key) = keys.get(&value.definition_id) {
                map.entry(key.clone())
                    .or_insert_with(|| value.value.clone());
            }
        }
        if !map.is_empty() {
            per_locale.insert(locale.clone(), map);
        }
    }
    if per_locale.is_empty() {
        Ok(None)
    } else {
        Ok(Some(serde_json::json!(per_locale).to_string()))
    }
}

/// Project one Product or SKU from the frozen row `version` names (the
/// three-source read path), into `generation`.
#[allow(clippy::too_many_arguments)] // the projection's operands
async fn project_entity(
    ctx: &ProjectorContext,
    conn: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    entity_kind: &str,
    entity_id: Uuid,
    version: i64,
    generation: i64,
    now: DateTime<Utc>,
) -> Result<(), ApplyError> {
    let versioned = match entity_kind {
        "product" => repo::VersionedEntityKind::Product,
        "sku" => repo::VersionedEntityKind::Sku,
        other => return Err(ApplyError::Poison(format!("unknown entity kind {other}"))),
    };
    let Some(frozen) =
        repo::entity_version_at(conn, scope, tenant_id, versioned, entity_id, version).await?
    else {
        return Err(ApplyError::Poison(format!(
            "frozen version {version} of {entity_kind} {entity_id} is not there (collected, or \
             never written)"
        )));
    };
    let content: JsonValue = serde_json::from_str(&frozen)
        .map_err(|e| ApplyError::Poison(format!("frozen content does not decode: {e}")))?;
    let text = |key: &str| json_str(&content, key).map(str::to_owned);
    let display_attributes =
        display_attributes_for(ctx, conn, scope, tenant_id, entity_kind, entity_id).await?;
    let mut row = ReadEntityRow {
        tenant_id,
        entity_kind: entity_kind.to_owned(),
        entity_id,
        entity_code: None,
        name: String::new(),
        lifecycle_state: "published".to_owned(),
        deprecated: false,
        composition_pending: false,
        sellable: None,
        deprecation_provenance: None,
        replaced_by_sku_id: None,
        region_scope: text("region_scope").unwrap_or_default(),
        brand_scope: text("brand_scope").unwrap_or_default(),
        sku_type: None,
        plan_tier_label: None,
        metering_unit: None,
        display_attributes,
        category_paths: None,
        published_version: version,
        projected_at: now,
        generation,
    };
    if entity_kind == "sku" {
        row.entity_code = text("sku_code");
        row.name = text("sku_code").unwrap_or_default();
        row.composition_pending = content
            .get("composition_pending")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
        row.sku_type = text("sku_type");
        row.metering_unit = text("metering_unit");
        if let Some(tier) = text("plan_tier") {
            row.plan_tier_label = repo::recognized_member(
                conn,
                scope,
                tenant_id,
                crate::domain::recognized::SetKind::PlanTier,
                &tier,
            )
            .await?
            .and_then(|member| member.display_label)
            .or(Some(tier));
        }
        if let Some(head) = repo::find_sku(conn, scope, tenant_id, entity_id).await? {
            head.lifecycle_state
                .as_str()
                .clone_into(&mut row.lifecycle_state);
            row.deprecated =
                head.lifecycle_state == bss_products_sdk::models::LifecycleState::Deprecated;
            row.deprecation_provenance = head.deprecation_provenance.map(|p| p.as_str().to_owned());
            row.replaced_by_sku_id = head.replaced_by_sku_id;
            // **The author's own flag, carried — not a derived one.** This line
            // read `published && !composition_pending` until the stand caught it
            // (2026-09-06): the row already carries `lifecycle_state` and
            // `composition_pending` as members of its own, so deriving `sellable`
            // from them said nothing new and **dropped** the one fact only the
            // head holds — `inst-cl-sellable`'s bucket-iii flag, which is
            // pricing's operand for predicate 6 (`03` §1.8). A SKU saved
            // `sellable = false` served `true` on browse, and the filter
            // `?sellable=false` could not find it.
            row.sellable = Some(head.sellable);
        }
    } else {
        row.entity_code = text("product_code");
        row.name = text("name").unwrap_or_default();
        if let Some(head) = repo::find_product(conn, scope, tenant_id, entity_id).await? {
            head.lifecycle_state
                .as_str()
                .clone_into(&mut row.lifecycle_state);
            row.deprecated =
                head.lifecycle_state == bss_products_sdk::models::LifecycleState::Deprecated;
            row.deprecation_provenance = head.deprecation_provenance.map(|p| p.as_str().to_owned());
        }
        let nodes: BTreeMap<Uuid, (Option<Uuid>, String)> =
            repo::category_nodes(conn, scope, tenant_id)
                .await?
                .into_iter()
                .map(|(id, parent, name)| (id, (parent, name)))
                .collect();
        // The assignment ids come from the frozen content (P-D-153): the
        // paths a consumer sees are the published assignment set's, with the
        // live tree supplying the path text. A version frozen before the
        // collections were content (scheme 2) has no array; it reads the live
        // assignments as before.
        let assigned = match frozen_assignment_ids(&content) {
            Some(ids) => ids,
            None => repo::category_assignments(conn, scope, tenant_id, entity_id)
                .await?
                .into_iter()
                .map(|a| a.category_id)
                .collect(),
        };
        row.category_paths = category_paths_for(&assigned, &nodes);
    }
    repo::upsert_read_entity(conn, scope, row).await?;
    Ok(())
}

/// The category ids a frozen version's `categories` collection names, or
/// `None` when the content predates the collections (digest scheme 2).
fn frozen_assignment_ids(content: &serde_json::Value) -> Option<Vec<Uuid>> {
    let rows = content.get("categories")?.as_array()?;
    Some(
        rows.iter()
            .filter_map(|row| row.get("categoryId")?.as_str()?.parse().ok())
            .collect(),
    )
}

/// The `04` flips' projection: the three head-read columns and the flags,
/// from the head row, no frozen content moving.
async fn refresh_head_fields(
    conn: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    entity_kind: &str,
    entity_id: Uuid,
    now: DateTime<Utc>,
) -> Result<(), RepoError> {
    let (state, provenance, replaced_by) = match entity_kind {
        "sku" => match repo::find_sku(conn, scope, tenant_id, entity_id).await? {
            Some(head) => (
                head.lifecycle_state,
                head.deprecation_provenance.map(|p| p.as_str().to_owned()),
                head.replaced_by_sku_id,
            ),
            None => return Ok(()),
        },
        _ => match repo::find_product(conn, scope, tenant_id, entity_id).await? {
            Some(head) => (
                head.lifecycle_state,
                head.deprecation_provenance.map(|p| p.as_str().to_owned()),
                None,
            ),
            None => return Ok(()),
        },
    };
    repo::set_read_entity_head_fields(
        conn,
        scope,
        tenant_id,
        entity_kind,
        entity_id,
        state.as_str(),
        state == bss_products_sdk::models::LifecycleState::Deprecated,
        provenance.as_deref(),
        replaced_by,
        now,
    )
    .await?;
    Ok(())
}

/// Re-file every Product row's browse paths from the live tree
/// (`inst-rp-reparent`).
///
/// @cpt-dod:cpt-cf-bss-products-dod-reparent:p2
async fn refresh_category_paths(
    conn: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    generation: i64,
    now: DateTime<Utc>,
) -> Result<(), RepoError> {
    let nodes: BTreeMap<Uuid, (Option<Uuid>, String)> =
        repo::category_nodes(conn, scope, tenant_id)
            .await?
            .into_iter()
            .map(|(id, parent, name)| (id, (parent, name)))
            .collect();
    for existing in repo::read_entities_of(conn, scope, tenant_id, generation).await? {
        if existing.entity_kind != "product" {
            continue;
        }
        // The published assignment set, from the row's own frozen version
        // (P-D-153); the live table only for a scheme-2 version.
        let frozen = repo::entity_version_at(
            conn,
            scope,
            tenant_id,
            repo::VersionedEntityKind::Product,
            existing.entity_id,
            existing.published_version,
        )
        .await?
        .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
        .and_then(|content| frozen_assignment_ids(&content));
        let assigned = match frozen {
            Some(ids) => ids,
            None => repo::category_assignments(conn, scope, tenant_id, existing.entity_id)
                .await?
                .into_iter()
                .map(|a| a.category_id)
                .collect(),
        };
        let paths = category_paths_for(&assigned, &nodes);
        if paths == existing.category_paths {
            continue;
        }
        repo::upsert_read_entity(
            conn,
            scope,
            row_from(existing, now, |row| row.category_paths.clone_from(&paths)),
        )
        .await?;
    }
    Ok(())
}

/// Re-render the locale-materialised columns (`display_attributes`,
/// `plan_tier_label`) of every row after a definition or tier-label change.
async fn refresh_display_fields(
    ctx: &ProjectorContext,
    conn: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    generation: i64,
    now: DateTime<Utc>,
) -> Result<(), RepoError> {
    for existing in repo::read_entities_of(conn, scope, tenant_id, generation).await? {
        let display = display_attributes_for(
            ctx,
            conn,
            scope,
            tenant_id,
            &existing.entity_kind,
            existing.entity_id,
        )
        .await?;
        let label = if existing.entity_kind == "sku"
            && let Some(head) = repo::find_sku(conn, scope, tenant_id, existing.entity_id).await?
            && let Some(tier) = head.plan_tier
        {
            repo::recognized_member(
                conn,
                scope,
                tenant_id,
                crate::domain::recognized::SetKind::PlanTier,
                &tier,
            )
            .await?
            .and_then(|member| member.display_label)
            .or(Some(tier))
        } else {
            existing.plan_tier_label.clone()
        };
        if display == existing.display_attributes && label == existing.plan_tier_label {
            continue;
        }
        repo::upsert_read_entity(
            conn,
            scope,
            row_from(existing, now, |row| {
                row.display_attributes.clone_from(&display);
                row.plan_tier_label.clone_from(&label);
            }),
        )
        .await?;
    }
    Ok(())
}

fn row_from(
    existing: crate::infra::storage::entity::read_entity::Model,
    now: DateTime<Utc>,
    patch: impl FnOnce(&mut ReadEntityRow),
) -> ReadEntityRow {
    let mut row = ReadEntityRow {
        tenant_id: existing.tenant_id,
        entity_kind: existing.entity_kind,
        entity_id: existing.entity_id,
        entity_code: existing.entity_code,
        name: existing.name,
        lifecycle_state: existing.lifecycle_state,
        deprecated: existing.deprecated,
        composition_pending: existing.composition_pending,
        sellable: existing.sellable,
        deprecation_provenance: existing.deprecation_provenance,
        replaced_by_sku_id: existing.replaced_by_sku_id,
        region_scope: existing.region_scope,
        brand_scope: existing.brand_scope,
        sku_type: existing.sku_type,
        plan_tier_label: existing.plan_tier_label,
        metering_unit: existing.metering_unit,
        display_attributes: existing.display_attributes,
        category_paths: existing.category_paths,
        published_version: existing.published_version,
        projected_at: now,
        generation: existing.generation,
    };
    patch(&mut row);
    row
}

/// Shadow-then-swap (`inst-rp-bootstrap`, P-D-126 row 8): project the latest
/// catalog version's manifest into `next_generation`, then swap by moving the
/// checkpoint's generation and dropping the old rows. A tenant with no
/// version starts from the empty catalog — the anchorless arm — and the tail
/// above `tail` is consumed by the ordinary passes that follow.
///
/// This is the bootstrap contract's first consumer (`design/12` §2.3 row 4,
/// `inst-rc-bootstrap`): the anchored arm from the latest version, the
/// anchorless arm from the empty catalog, and a checkpoint behind the retained
/// tail failing loudly (`read_model_rebuilt`) with the rebuild as the remedy.
///
/// @cpt-dod:cpt-cf-bss-products-dod-bootstrap:p1
#[allow(clippy::too_many_arguments)] // the rebuild's operands
#[allow(clippy::cognitive_complexity)] // the anchored arm's loop and the swap, one fn
async fn rebuild_tenant(
    ctx: &ProjectorContext,
    conn: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    next_generation: i64,
    tail: i64,
    now: DateTime<Utc>,
) -> Result<usize, RepoError> {
    let newest = repo::newest_catalog_versions(conn, scope, tenant_id, 1).await?;
    let mut rows = 0usize;
    let catalog = if let Some(version) = newest.first().copied() {
        let (entries, _captures) =
            repo::catalog_version_manifest_rows(conn, scope, tenant_id, version).await?;
        for entry in entries {
            match project_entity(
                ctx,
                conn,
                scope,
                tenant_id,
                &entry.entity_kind,
                entry.entity_id,
                entry.published_version,
                next_generation,
                now,
            )
            .await
            {
                Ok(()) => rows += 1,
                Err(ApplyError::Poison(reason)) => {
                    tracing::warn!(%tenant_id, entity = %entry.entity_id, reason, "bss-products: rebuild skipped an entry");
                }
                Err(ApplyError::Repo(error)) => return Err(error),
            }
        }
        StampCatalogTouch::Set(version)
    } else {
        StampCatalogTouch::Anchorless
    };
    tracing::warn!(
        event = "read_model_rebuilt",
        %tenant_id,
        rows,
        generation = next_generation,
        "bss-products: read projection checkpoint fell behind the swept tail; rebuilt from the \
         latest catalog version and swapped"
    );
    repo::write_read_checkpoint(conn, scope, tenant_id, tail, next_generation, now).await?;
    repo::delete_read_generation(conn, scope, tenant_id, next_generation - 1).await?;
    advance_stamp(conn, scope, tenant_id, catalog, true, now).await?;
    Ok(rows)
}

// ---------------------------------------------------------------------------
// The polled dashboards (`inst-ps-dashboards`, P-D-126 row 10)
// ---------------------------------------------------------------------------

/// How many children a deferred retirement's `children_snapshot` names, or
/// `None` for a snapshot that does not parse or is not a list — a corrupt
/// row, named on the log and **skipped**: showing `0` for it would read as
/// "safe to retire" on a row whose children are unknown.
fn deferred_children_count(
    tenant_id: Uuid,
    product_id: Uuid,
    children_snapshot: &str,
) -> Option<usize> {
    match serde_json::from_str::<JsonValue>(children_snapshot) {
        Ok(JsonValue::Array(children)) => Some(children.len()),
        Ok(_) => {
            tracing::warn!(
                event = "deferred_intent_snapshot_unreadable",
                %tenant_id,
                %product_id,
                "bss-products: a deferred retirement's children_snapshot is not a list; the \
                 dashboard row is skipped"
            );
            None
        }
        Err(error) => {
            tracing::warn!(
                event = "deferred_intent_snapshot_unreadable",
                %tenant_id,
                %product_id,
                %error,
                "bss-products: a deferred retirement's children_snapshot does not parse; the \
                 dashboard row is skipped"
            );
            None
        }
    }
}

/// One poll of the three dashboard tables over every tenant that has a
/// source row: 04's deferred table, 06's ledger, the inbox and the park.
///
/// # Errors
///
/// [`RepoError`] on a storage failure.
pub(crate) async fn poll_dashboards(
    ctx: &ProjectorContext,
    now: DateTime<Utc>,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<(), RepoError> {
    let conn = ctx
        .db
        .conn()
        .map_err(|e| RepoError::Db(format!("dashboard poll connection: {e}")))?;
    let all = AccessScope::allow_all();
    let mut tenants: BTreeSet<Uuid> = BTreeSet::new();
    tenants.extend(repo::tenants_with_inbox(&conn, &all).await?);
    tenants.extend(repo::tenants_with_catalog_versions(&conn, &all).await?);
    tenants.extend(repo::tenants_with_unresolved_deferred_retirements(&conn, &all).await?);
    for tenant_id in tenants {
        // Each tenant's poll is its own set of upserts; a stop between two
        // tenants leaves the rest one poll stale, which the next process's
        // first tick repairs.
        if cancel.is_cancelled() {
            return Ok(());
        }
        let scope = AccessScope::for_tenant(tenant_id);
        // Deferred intents, from 04's table.
        let intents = repo::unresolved_deferred_retirements(&conn, &scope, tenant_id).await?;
        let mut keep = Vec::with_capacity(intents.len());
        for intent in intents {
            let Some(children) =
                deferred_children_count(tenant_id, intent.product_id, &intent.children_snapshot)
            else {
                continue;
            };
            keep.push(intent.product_id);
            repo::upsert_read_deferred_intent(
                &conn,
                &scope,
                crate::infra::storage::entity::read_deferred_intent::Model {
                    tenant_id,
                    product_id: intent.product_id,
                    cascade_ref: intent.cascade_ref,
                    children_count: i32::try_from(children).unwrap_or(i32::MAX),
                    created_at: intent.created_at,
                    age_secs: now
                        .signed_duration_since(intent.created_at)
                        .num_seconds()
                        .max(0),
                    polled_at: now,
                },
            )
            .await?;
        }
        repo::prune_read_deferred_intents(&conn, &scope, tenant_id, &keep).await?;
        // Freeze status, from 06's ledger.
        for version in repo::newest_catalog_versions(&conn, &scope, tenant_id, 50).await? {
            let Some(record) =
                repo::find_catalog_version(&conn, &scope, tenant_id, version).await?
            else {
                continue;
            };
            let rows = repo::freeze_ack_rows(&conn, &scope, tenant_id, version).await?;
            let count = |state: crate::domain::states::FreezeAckState| {
                i32::try_from(rows.iter().filter(|(_, s)| *s == state).count()).unwrap_or(i32::MAX)
            };
            repo::upsert_read_freeze_status(
                &conn,
                &scope,
                crate::infra::storage::entity::read_freeze_status::Model {
                    tenant_id,
                    catalog_version_id: version,
                    freeze_state: record.freeze_state.as_str().to_owned(),
                    pending: count(crate::domain::states::FreezeAckState::Pending),
                    acked: count(crate::domain::states::FreezeAckState::Acked),
                    released: count(crate::domain::states::FreezeAckState::Released),
                    forced: count(crate::domain::states::FreezeAckState::NotFrozenForced),
                    published_at: record.published_at,
                    polled_at: now,
                },
            )
            .await?;
        }
        // Delivery state, from the inbox and the park.
        let (checkpoint, _) = repo::load_read_checkpoint(&conn, &scope, tenant_id)
            .await?
            .unwrap_or((0, 0));
        let (pending, oldest) = repo::inbox_pending(&conn, &scope, tenant_id, checkpoint).await?;
        let parked = repo::parked_poison(&conn, &scope, tenant_id).await?.len();
        repo::upsert_read_delivery_state(
            &conn,
            &scope,
            crate::infra::storage::entity::read_delivery_state::Model {
                tenant_id,
                inbox_pending: i64::try_from(pending).unwrap_or(i64::MAX),
                parked: i64::try_from(parked).unwrap_or(i64::MAX),
                oldest_pending_age_secs: oldest
                    .map_or(0, |at| now.signed_duration_since(at).num_seconds().max(0)),
                polled_at: now,
            },
        )
        .await?;
    }
    Ok(())
}

/// Sweep consumed inbox rows past the retention window, per tenant.
///
/// # Errors
///
/// [`RepoError`] on a storage failure.
pub(crate) async fn sweep_inbox(
    ctx: &ProjectorContext,
    now: DateTime<Utc>,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<u64, RepoError> {
    let conn = ctx
        .db
        .conn()
        .map_err(|e| RepoError::Db(format!("inbox sweep connection: {e}")))?;
    let before = now - ChronoDuration::hours(i64::from(ctx.knobs.inbox_retention_hours));
    let mut swept = 0;
    for tenant_id in repo::tenants_with_inbox(&conn, &AccessScope::allow_all()).await? {
        if cancel.is_cancelled() {
            break;
        }
        let scope = AccessScope::for_tenant(tenant_id);
        let Some((checkpoint, _)) = repo::load_read_checkpoint(&conn, &scope, tenant_id).await?
        else {
            continue;
        };
        swept += repo::sweep_inbox(&conn, &scope, tenant_id, checkpoint, before).await?;
    }
    Ok(swept)
}

#[cfg(test)]
#[path = "projector_tests.rs"]
mod projector_tests;
