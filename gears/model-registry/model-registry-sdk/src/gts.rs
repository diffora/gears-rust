// Created: 2026-10-26 by Constructor Tech
//! GTS identifiers of the Model Registry (DESIGN §3.3 Provider Base Type and
//! §4 Authorization).

use toolkit_gts::gts_id;

/// Provider base type. Every provider `gts_type` is derived from it; see
/// [`ProviderTypeV1`](crate::models::ProviderTypeV1).
pub const PROVIDER_BASE_TYPE: &str = gts_id!("cf.genai.model.provider.v1~");

/// PDP resource type of providers, and the `resource_type` of
/// provider-scoped `NotFound` / `AlreadyExists` errors.
pub const PROVIDER_RESOURCE_TYPE: &str = gts_id!("cf.genai.model_registry.provider.v1~");

/// PDP resource type of models, and the `resource_type` of model-scoped
/// `NotFound` / `AlreadyExists` errors.
pub const MODEL_RESOURCE_TYPE: &str = gts_id!("cf.genai.model_registry.model.v1~");

/// Typed view of the two resource types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Resource {
    Provider,
    Model,
}

impl Resource {
    /// `None` for a resource type the registry does not own.
    #[must_use]
    pub fn from_wire(resource_type: &str) -> Option<Self> {
        match resource_type {
            PROVIDER_RESOURCE_TYPE => Some(Self::Provider),
            MODEL_RESOURCE_TYPE => Some(Self::Model),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::Provider => PROVIDER_RESOURCE_TYPE,
            Self::Model => MODEL_RESOURCE_TYPE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_round_trips_through_wire() {
        for resource in [Resource::Provider, Resource::Model] {
            assert_eq!(Resource::from_wire(resource.as_wire()), Some(resource));
        }
        assert_eq!(Resource::from_wire("gts.cf.other.thing.v1~"), None);
    }
}
