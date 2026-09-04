//! Shared guards for pricing OData collection GETs.
//!
//! Extra query keys (including retired named filters) are 400. `limit` / `cursor`
//! stay allowed so OpenAPI pagination matches AM tenants.

use std::collections::HashMap;

use toolkit::api::canonical_prelude::CanonicalError;
use toolkit_odata::ODataQuery;

use crate::domain::error::DomainError;
use crate::infra::storage::odata_mapping::OdataPageError;

const LIST_PAGINATION_KEYS: &[&str] = &["limit", "cursor"];

/// Reject a named filter on an OData list (`?lifecycle_state=`, `?status=`).
pub(crate) fn reject_non_odata_list_params(
    query: &HashMap<String, String>,
) -> Result<(), CanonicalError> {
    if let Some(unknown) = query
        .keys()
        .find(|k| !k.starts_with('$') && !LIST_PAGINATION_KEYS.contains(&k.as_str()))
    {
        return Err(CanonicalError::from(DomainError::InvalidRequest(format!(
            "unrecognized query parameter `{unknown}`; pricing list endpoints \
             accept OData parameters only (e.g. `$filter=lifecycle_state eq \
             'draft'`) plus `limit` and `cursor`"
        ))));
    }
    Ok(())
}

/// D-125: `limit=0` is 400. Toolkit `paginate_odata` would clamp it to 1.
pub(crate) fn refuse_zero_limit(query: &ODataQuery) -> Result<(), CanonicalError> {
    if query.limit == Some(0) {
        return Err(CanonicalError::from(DomainError::InvalidRequest(
            "limit must be at least 1; a page of zero rows never advances".to_owned(),
        )));
    }
    Ok(())
}

/// Project an [`OdataPageError`] into a [`CanonicalError`].
pub(crate) fn map_odata_page_err(err: OdataPageError) -> CanonicalError {
    match err {
        OdataPageError::Odata(e) => CanonicalError::from(e),
        OdataPageError::Db(detail) => {
            tracing::error!(
                detail = %detail,
                "bss-pricing: list read: database error (driver text redacted on the wire)"
            );
            CanonicalError::internal(
                "bss-pricing: list read: database error (driver text redacted)",
            )
            .create()
        }
    }
}

#[cfg(test)]
#[path = "odata_list_tests.rs"]
mod tests;
