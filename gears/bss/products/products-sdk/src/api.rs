//! The client trait a consumer resolves from `ClientHub`.

use async_trait::async_trait;
use toolkit_canonical_errors::CanonicalError;
use uuid::Uuid;

use crate::models::{Product, Sku};

/// The in-process contract for reading registry entities.
///
/// Every method returns [`CanonicalError`] rather than a gear-local error type:
/// the gear's single `From<DomainError>` ladder is the authoritative
/// classification, and a second one on this port would be a second place for a
/// rejection to be categorised.
#[async_trait]
pub trait ProductsClient: Send + Sync {
    /// Read a Product's head row in the caller's tenant scope.
    ///
    /// # Errors
    /// A canonical `NotFound` when no such Product is visible in scope, or the
    /// canonical projection of whatever the authorization gate refused.
    async fn get_product(
        &self,
        tenant_id: Uuid,
        product_id: Uuid,
    ) -> Result<Product, CanonicalError>;

    /// Read a SKU's head row in the caller's tenant scope.
    ///
    /// # Errors
    /// A canonical `NotFound` when no such SKU is visible in scope, or the
    /// canonical projection of whatever the authorization gate refused.
    async fn get_sku(&self, tenant_id: Uuid, sku_id: Uuid) -> Result<Sku, CanonicalError>;
}
