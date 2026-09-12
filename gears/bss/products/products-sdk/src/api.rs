//! The client trait a consumer resolves from `ClientHub`.

use async_trait::async_trait;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::models::{Product, Sku};

/// The in-process contract for reading registry entities — `inst-sdk-surface`'s
/// second row, the read-model client. Its in-process binding spends the
/// `product × read` / `sku × read` grant the REST doors spend, which is why
/// every method takes the caller's [`SecurityContext`] like the SDK's other
/// traits (P-D-81 arm 4; the context was added when the binding was, P-D-151,
/// the trait having had no implementor before it).
///
/// Every method returns [`CanonicalError`] rather than a gear-local error type:
/// the gear's single `From<DomainError>` ladder is the authoritative
/// classification, and a second one on this port would be a second place for a
/// rejection to be categorised.
///
/// The browse and history surfaces are **not** on this port: a machine
/// consumer resolves one entity by id; browsing is the REST read model's
/// (`GET /bss-products/v1/browse`, `design/08`).
#[async_trait]
pub trait ProductsClient: Send + Sync {
    /// Read a Product's head row in the caller's tenant scope.
    ///
    /// # Errors
    /// A canonical `NotFound` when no such Product is visible in scope, or the
    /// canonical projection of whatever the authorization gate refused.
    async fn get_product(
        &self,
        ctx: &SecurityContext,
        tenant_id: Uuid,
        product_id: Uuid,
    ) -> Result<Product, CanonicalError>;

    /// Read a SKU's head row in the caller's tenant scope.
    ///
    /// # Errors
    /// A canonical `NotFound` when no such SKU is visible in scope, or the
    /// canonical projection of whatever the authorization gate refused.
    async fn get_sku(
        &self,
        ctx: &SecurityContext,
        tenant_id: Uuid,
        sku_id: Uuid,
    ) -> Result<Sku, CanonicalError>;
}
