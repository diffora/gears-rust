//! Atomic settings and dimension registry operations.
//!
//! @cpt-dod:cpt-cf-bss-pricing-dod-dimension-registry:p1
use super::{
    dto::{PricingDimensionEntry, PricingDimensions, PricingSettingsDto, PricingSettingsPut},
    support::{DoorError, audit, check_version, conflict, invalid, response, value},
};
use crate::{
    domain::{dimension, price::validate_template},
    infra::storage::{
        entity::{dimension_key, settings},
        repo::{dimension_repo, price_repo, row_repo, settings_repo},
    },
};
use axum::{http::StatusCode, response::Response};
use toolkit_canonical_errors::CanonicalError;
use toolkit_db::secure::{AccessScope, DBRunner};
use toolkit_security::SecurityContext;
use uuid::Uuid;
pub async fn settings(
    tx: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
) -> Result<PricingSettingsDto, DoorError> {
    let m = settings_repo::find(tx, scope, tenant, tenant).await?;
    Ok(m.map_or_else(
        || PricingSettingsDto {
            default_timing: "advance".into(),
            default_rounding: "half_up".into(),
            default_gl: None,
            default_tax_category: None,
            invoice_line_templates: serde_json::json!({}),
            version: 0,
        },
        |m| PricingSettingsDto {
            default_timing: m.default_timing,
            default_rounding: m.default_rounding,
            default_gl: m.default_gl,
            default_tax_category: m.default_tax_category,
            invoice_line_templates: m.invoice_line_templates,
            version: m.version,
        },
    ))
}
pub async fn put_settings(
    tx: &impl DBRunner,
    scope: &AccessScope,
    ctx: &SecurityContext,
    correlation: Uuid,
    version: u64,
    body: PricingSettingsPut,
) -> Result<Response, DoorError> {
    let tenant = ctx.subject_tenant_id();
    let before = settings(tx, scope, tenant).await?;
    check_version(version, before.version)?;
    if !matches!(body.default_timing.as_str(), "advance" | "arrears") {
        return Err(invalid("default_timing", "TIMING_INVALID").into());
    }
    if body.default_rounding.trim().is_empty() {
        return Err(invalid("default_rounding", "ROUNDING_REQUIRED").into());
    }
    for (kind, template) in &body.invoice_line_templates {
        if !matches!(kind.as_str(), "recurring" | "usage" | "one_time" | "bundle") {
            return Err(invalid("invoice_line_templates", "SKU_TYPE_INVALID").into());
        }
        validate_template(template).map_err(|e| invalid("invoice_line_templates", e.code))?;
    }
    let now = time::OffsetDateTime::now_utc();
    let m = settings::Model {
        tenant_id: tenant,
        default_timing: body.default_timing,
        default_rounding: body.default_rounding,
        default_gl: body.default_gl,
        default_tax_category: body.default_tax_category,
        invoice_line_templates: value(&body.invoice_line_templates)?,
        version: before.version,
        created_at: now,
        updated_at: now,
    };
    if before.version == 0 {
        settings_repo::insert(tx, scope, settings::Model { version: 1, ..m }).await?;
    } else {
        settings_repo::update(tx, scope, m).await?;
    }
    audit(
        tx,
        ctx,
        correlation,
        "settings.update",
        tenant,
        before.version + 1,
    )
    .await?;
    Ok(response(
        StatusCode::OK,
        &settings(tx, scope, tenant).await?,
        Some(version + 1),
    )?)
}
pub async fn dimensions(
    tx: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
) -> Result<(PricingDimensions, u64), DoorError> {
    let rows = dimension_repo::list(tx, scope, tenant).await?;
    // A content validator covers the complete sorted collection, including each row version.
    // The decimal tag uses the first 64 SHA-256 bits to retain the door's strong decimal-tag grammar.
    let content: Vec<_> = rows
        .iter()
        .map(|r| (&r.key, &r.values, r.version))
        .collect();
    let hash =
        crate::api::rest::preconditions::request_digest(&content).map_err(CanonicalError::from)?;
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&hash[..8]);
    let tag = u64::from_be_bytes(bytes);
    let mut items = Vec::new();
    for r in rows {
        items.push(PricingDimensionEntry {
            key: r.key,
            values: serde_json::from_value(r.values)
                .map_err(|_| CanonicalError::internal("invalid stored dimension").create())?,
        });
    }
    Ok((PricingDimensions { items }, tag))
}
pub async fn put_dimensions(
    tx: &impl DBRunner,
    scope: &AccessScope,
    ctx: &SecurityContext,
    correlation: Uuid,
    version: u64,
    mut body: PricingDimensions,
) -> Result<Response, DoorError> {
    let tenant = ctx.subject_tenant_id();
    let (_, tag) = dimensions(tx, scope, tenant).await?;
    if tag != version {
        return Err(conflict("VERSION_CONFLICT").into());
    }
    let mut keys = std::collections::BTreeSet::new();
    for item in &mut body.items {
        item.key = item.key.trim().into();
        item.values = item
            .values
            .iter()
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
            .collect();
        if !keys.insert(item.key.clone()) {
            return Err(invalid("items", "DIM_KEY_DUPLICATE").into());
        }
        if let Some(e) = dimension::validate(&item.key, &item.values).first() {
            return Err(invalid("items", e.code).into());
        }
    }
    // Inspect all states. A rejected or pending row still carries its value.
    let dependency_scope = AccessScope::for_tenant(tenant);
    for p in price_repo::list(tx, &dependency_scope, tenant).await? {
        if let Some(key) = p.dimension_key.as_deref() {
            let next = body.items.iter().find(|i| i.key == key);
            for row in row_repo::for_price(tx, &dependency_scope, tenant, p.id).await? {
                if let Some(value) = row.dim_value
                    && next.is_none_or(|i| !i.values.contains(&value))
                {
                    return Err(conflict("DIM_VALUE_IN_USE").into());
                }
            }
            // A price names the key whatever its rows hold (the price's key is a foreign key
            // to the registry): removing it is the same refusal PATCH /prices answers.
            if next.is_none() {
                return Err(conflict("DIMENSION_KEY_IN_USE").into());
            }
        }
    }
    for old in dimension_repo::list(tx, scope, tenant).await? {
        if !keys.contains(&old.key) {
            dimension_repo::delete(tx, scope, tenant, &old.key, old.version).await?;
        }
    }
    for item in body.items {
        let prior = dimension_repo::find(tx, scope, tenant, &item.key).await?;
        let model = dimension_key::Model {
            tenant_id: tenant,
            key: item.key,
            values: value(&item.values)?,
            version: prior.as_ref().map_or(1, |r| r.version),
        };
        if prior.is_some() {
            dimension_repo::update(tx, scope, model).await?;
        } else {
            dimension_repo::insert(tx, scope, model).await?;
        }
    }
    audit(tx, ctx, correlation, "dimension_keys.update", tenant, 0).await?;
    let (body, tag) = dimensions(tx, scope, tenant).await?;
    Ok(response(StatusCode::OK, &body, Some(tag))?)
}
