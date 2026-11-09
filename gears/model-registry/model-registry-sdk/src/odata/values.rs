// Created: 2026-08-06 by Constructor Tech
// Updated: 2026-10-26 by Constructor Tech
//! [`IntoODataValue`] impls for the SDK's wire enums.
//!
//! The filter columns behind these fields are plain strings, so without these
//! impls a caller would write `MODEL_APPROVAL_STATE.eq("approved")` and a
//! typo would survive to a runtime empty page. With them the enum itself is
//! the literal, and the wire spelling comes from the same `as_str` the
//! storage layer writes.
//!
//! `gts_type` has no impl here: [`gts::GtsTypeId`] is a foreign type and
//! [`IntoODataValue`] a foreign trait, so the orphan rule forbids it. Pass
//! `GtsTypeId::to_string()` (or a `&str`) to `MODEL_GTS_TYPE`.

use toolkit_odata::ast::Value;
use toolkit_odata::schema::IntoODataValue;

use crate::models::{ApprovalState, SharingMode};

impl IntoODataValue for SharingMode {
    fn into_odata_value(self) -> Value {
        Value::String(self.as_str().to_owned())
    }
}

impl IntoODataValue for ApprovalState {
    fn into_odata_value(self) -> Value {
        Value::String(self.as_str().to_owned())
    }
}
