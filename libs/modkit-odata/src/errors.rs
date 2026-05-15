//! GTS-typed error scope for the `OData` query / pagination / cursor layer.

use modkit_canonical_errors::resource_error;

#[resource_error("gts.cf.core.odata.query.v1~")]
pub struct OdataError;
