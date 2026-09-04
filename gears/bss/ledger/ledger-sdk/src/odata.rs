//! `OData` filter-field schemas for the ledger's collection GETs.
//!
//! Dummy structs: the user-facing request shape is `ODataQuery`. The derive
//! generates `{Name}FilterField`, re-exported under the names the gear already
//! uses. `tenant_id` is a seller-scope filter on every in-scope list.

use toolkit_odata::filter::{FieldKind, FilterField};
use toolkit_odata_macros::ODataFilterable;
use uuid::Uuid;

#[derive(ODataFilterable)]
#[allow(dead_code)]
struct AccountInfoQuery {
    #[odata(filter(kind = "Uuid"))]
    pub tenant_id: Uuid,
    #[odata(filter(kind = "Uuid"))]
    pub account_id: Uuid,
    #[odata(filter(kind = "String"))]
    pub account_class: String,
    #[odata(filter(kind = "String"))]
    pub currency: String,
    #[odata(filter(kind = "String"))]
    pub revenue_stream: String,
    #[odata(filter(kind = "String"))]
    pub lifecycle_state: String,
}

#[derive(ODataFilterable)]
#[allow(dead_code)]
struct JournalLineQuery {
    #[odata(filter(kind = "Uuid"))]
    pub tenant_id: Uuid,
    #[odata(filter(kind = "Uuid"))]
    pub line_id: Uuid,
    #[odata(filter(kind = "Uuid"))]
    pub payer_tenant_id: Uuid,
    #[odata(filter(kind = "String"))]
    pub account_class: String,
    #[odata(filter(kind = "String"))]
    pub period_id: String,
    #[odata(filter(kind = "String"))]
    pub invoice_id: String,
}

#[derive(ODataFilterable)]
#[allow(dead_code)]
struct JournalEntryQuery {
    #[odata(filter(kind = "Uuid"))]
    pub tenant_id: Uuid,
    #[odata(filter(kind = "Uuid"))]
    pub entry_id: Uuid,
    #[odata(filter(kind = "String"))]
    pub source_doc_type: String,
    #[odata(filter(kind = "String"))]
    pub source_business_id: String,
    #[odata(filter(kind = "String"))]
    pub period_id: String,
}

#[derive(ODataFilterable)]
#[allow(dead_code)]
struct BalanceQuery {
    #[odata(filter(kind = "Uuid"))]
    pub tenant_id: Uuid,
    #[odata(filter(kind = "Uuid"))]
    pub account_id: Uuid,
    #[odata(filter(kind = "String"))]
    pub account_class: String,
    #[odata(filter(kind = "String"))]
    pub currency: String,
}

#[derive(ODataFilterable)]
#[allow(dead_code)]
struct RefundQuery {
    #[odata(filter(kind = "Uuid"))]
    pub tenant_id: Uuid,
    #[odata(filter(kind = "String"))]
    pub refund_id: String,
    #[odata(filter(kind = "String"))]
    pub payment_id: String,
    #[odata(filter(kind = "String"))]
    pub psp_refund_id: String,
    #[odata(filter(kind = "String"))]
    pub phase: String,
    #[odata(filter(kind = "String"))]
    pub pattern: String,
    #[odata(filter(kind = "String"))]
    pub clearing_state: String,
    #[odata(filter(kind = "String"))]
    pub invoice_id: String,
}

#[derive(ODataFilterable)]
#[allow(dead_code)]
struct CreditNoteQuery {
    #[odata(filter(kind = "Uuid"))]
    pub tenant_id: Uuid,
    #[odata(filter(kind = "String"))]
    pub credit_note_id: String,
    #[odata(filter(kind = "String"))]
    pub origin_invoice_id: String,
    #[odata(filter(kind = "String"))]
    pub revenue_stream: String,
    #[odata(filter(kind = "String"))]
    pub reason_code: String,
}

#[derive(ODataFilterable)]
#[allow(dead_code)]
struct DebitNoteQuery {
    #[odata(filter(kind = "Uuid"))]
    pub tenant_id: Uuid,
    #[odata(filter(kind = "String"))]
    pub debit_note_id: String,
    #[odata(filter(kind = "String"))]
    pub origin_invoice_id: String,
}

#[derive(ODataFilterable)]
#[allow(dead_code)]
struct DisputeQuery {
    #[odata(filter(kind = "Uuid"))]
    pub tenant_id: Uuid,
    #[odata(filter(kind = "String"))]
    pub dispute_id: String,
    #[odata(filter(kind = "String"))]
    pub payment_id: String,
    #[odata(filter(kind = "String"))]
    pub last_phase: String,
    #[odata(filter(kind = "String"))]
    pub variant: String,
}

#[derive(ODataFilterable)]
#[allow(dead_code)]
struct RecognitionRunQuery {
    #[odata(filter(kind = "Uuid"))]
    pub tenant_id: Uuid,
    #[odata(filter(kind = "Uuid"))]
    pub run_id: Uuid,
    #[odata(filter(kind = "String"))]
    pub period_id: String,
    #[odata(filter(kind = "String"))]
    pub status: String,
}

/// Hand-written: the wire field is `type`, a Rust keyword. The derive turns
/// `r#type` into variant `RType` and name `r#type`, which would break
/// `$filter=type eq '…'`.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum ExceptionFilterField {
    TenantId,
    ExceptionId,
    /// Wire name `type`.
    ExceptionType,
    Status,
    BusinessRef,
    PeriodId,
}

impl FilterField for ExceptionFilterField {
    const FIELDS: &'static [Self] = &[
        Self::TenantId,
        Self::ExceptionId,
        Self::ExceptionType,
        Self::Status,
        Self::BusinessRef,
        Self::PeriodId,
    ];

    fn name(&self) -> &'static str {
        match self {
            Self::TenantId => "tenant_id",
            Self::ExceptionId => "exception_id",
            Self::ExceptionType => "type",
            Self::Status => "status",
            Self::BusinessRef => "business_ref",
            Self::PeriodId => "period_id",
        }
    }

    fn kind(&self) -> FieldKind {
        match self {
            Self::TenantId | Self::ExceptionId => FieldKind::Uuid,
            Self::ExceptionType | Self::Status | Self::BusinessRef | Self::PeriodId => {
                FieldKind::String
            }
        }
    }
}

pub use AccountInfoQueryFilterField as AccountInfoFilterField;
pub use BalanceQueryFilterField as BalanceFilterField;
pub use CreditNoteQueryFilterField as CreditNoteFilterField;
pub use DebitNoteQueryFilterField as DebitNoteFilterField;
pub use DisputeQueryFilterField as DisputeFilterField;
pub use JournalEntryQueryFilterField as JournalEntryFilterField;
pub use JournalLineQueryFilterField as JournalLineFilterField;
pub use RecognitionRunQueryFilterField as RecognitionRunFilterField;
pub use RefundQueryFilterField as RefundFilterField;
