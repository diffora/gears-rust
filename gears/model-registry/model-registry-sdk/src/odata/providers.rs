// Created: 2026-08-06 by Constructor Tech
// Updated: 2026-10-26 by Constructor Tech
//! `OData` filter / order surface of [`list_providers`] (DESIGN §3.3).
//!
//! [`list_providers`]: crate::ModelRegistryClientV1::list_providers

use chrono::{DateTime, Utc};
use toolkit_odata::filter::FilterField as _;
use toolkit_odata::{FieldRef, Schema};
use toolkit_odata_macros::ODataFilterable;
use uuid::Uuid;

/// Filterable wire surface of the provider list. Every field maps to a real
/// `providers` column, and every field is orderable.
///
/// Never constructed at runtime: the struct exists to carry the annotations;
/// the generated [`ProviderFilterField`] enum is the usable contract.
#[derive(ODataFilterable)]
pub struct ProviderQuery {
    #[odata(filter(kind = "Uuid"))]
    pub id: Uuid,
    #[odata(filter(kind = "Uuid"))]
    pub tenant_id: Uuid,
    #[odata(filter(kind = "String"))]
    pub slug: String,
    #[odata(filter(kind = "String"))]
    pub name: String,
    #[odata(filter(kind = "String"))]
    pub gts_type: String,
    #[odata(filter(kind = "Bool"))]
    pub enabled: bool,
    #[odata(filter(kind = "String"))]
    pub sharing_mode: String,
    #[odata(filter(kind = "Bool"))]
    pub managed: bool,
    #[odata(filter(kind = "DateTimeUtc"))]
    pub created_at: DateTime<Utc>,
    #[odata(filter(kind = "DateTimeUtc"))]
    pub updated_at: DateTime<Utc>,
}

/// `OData` filter / order field enum of the provider list, generated from
/// [`ProviderQuery`].
pub use ProviderQueryFilterField as ProviderFilterField;

/// Schema marker binding [`ProviderFilterField`] to the typed
/// [`QueryBuilder`](toolkit_odata::QueryBuilder).
#[derive(Debug, Clone, Copy)]
pub struct ProviderSchema;

impl Schema for ProviderSchema {
    type Field = ProviderFilterField;

    fn field_name(field: Self::Field) -> &'static str {
        field.name()
    }
}

// ---------------------------------------------------------------------------
// Typed field references
// ---------------------------------------------------------------------------

pub const PROVIDER_ID: FieldRef<ProviderSchema, Uuid> = FieldRef::new(ProviderFilterField::Id);
pub const PROVIDER_TENANT_ID: FieldRef<ProviderSchema, Uuid> =
    FieldRef::new(ProviderFilterField::TenantId);
pub const PROVIDER_SLUG: FieldRef<ProviderSchema, String> =
    FieldRef::new(ProviderFilterField::Slug);
pub const PROVIDER_NAME: FieldRef<ProviderSchema, String> =
    FieldRef::new(ProviderFilterField::Name);
pub const PROVIDER_GTS_TYPE: FieldRef<ProviderSchema, String> =
    FieldRef::new(ProviderFilterField::GtsType);
pub const PROVIDER_ENABLED: FieldRef<ProviderSchema, bool> =
    FieldRef::new(ProviderFilterField::Enabled);
pub const PROVIDER_SHARING_MODE: FieldRef<ProviderSchema, String> =
    FieldRef::new(ProviderFilterField::SharingMode);
pub const PROVIDER_MANAGED: FieldRef<ProviderSchema, bool> =
    FieldRef::new(ProviderFilterField::Managed);
pub const PROVIDER_CREATED_AT: FieldRef<ProviderSchema, DateTime<Utc>> =
    FieldRef::new(ProviderFilterField::CreatedAt);
pub const PROVIDER_UPDATED_AT: FieldRef<ProviderSchema, DateTime<Utc>> =
    FieldRef::new(ProviderFilterField::UpdatedAt);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_fields_match_the_design_list() {
        let names: Vec<&str> = ProviderFilterField::FIELDS
            .iter()
            .map(ProviderFilterField::name)
            .collect();
        assert_eq!(
            names,
            [
                "id",
                "tenant_id",
                "slug",
                "name",
                "gts_type",
                "enabled",
                "sharing_mode",
                "managed",
                "created_at",
                "updated_at",
            ]
        );
    }
}
