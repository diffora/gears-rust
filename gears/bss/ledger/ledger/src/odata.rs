//! `OData` filter-field enums for the ledger's collection GETs.
//!
//! Schemas live in `bss_ledger_sdk::odata`. This module re-exports them so
//! existing `use crate::odata::…` sites keep compiling.

pub use bss_ledger_sdk::odata::{
    AccountInfoFilterField, BalanceFilterField, CreditNoteFilterField, DebitNoteFilterField,
    DisputeFilterField, ExceptionFilterField, JournalEntryFilterField, JournalLineFilterField,
    RecognitionRunFilterField, RefundFilterField,
};

#[cfg(test)]
#[path = "odata_tests.rs"]
mod odata_tests;
