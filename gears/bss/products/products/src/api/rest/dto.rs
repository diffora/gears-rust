//! The wire DTOs of the catalog-version, bulk and reference surfaces —
//! the canonical gear layout's `api/rest/dto.rs`, so request/response
//! shapes stay separated from handler logic and cannot silently drift
//! into domain or SDK surfaces.
//!
//! The Foundation's own views (`ProductView`, `SkuView` and their read
//! shapes) predate this file and still live beside their doors; moving
//! them is its own churn and follows separately if the layout rule is
//! extended to them.

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// One row of an import request.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request)]
pub struct ImportRowRequest {
    /// The caller's own key for this row, unique **within the batch**.
    pub row_key: String,
    /// `product` or `sku`.
    pub entity_kind: String,
    /// The entity this row targets, for an update-as-draft row.
    pub entity_id: Option<Uuid>,
    /// The revision this row pins, for an update-as-draft row.
    pub pinned_revision: Option<i64>,
    /// The row's content — what the worker parses and stages
    /// (**P-D-86**). A `product` row carries `{name, brand_id,
    /// product_code?, region_scope?, brand_scope?}`; a `sku` row
    /// `{product_id, sku_code, region_scope?, brand_scope?}`. The door
    /// records it canonically serialized and judges only that it is an
    /// object: **the field names are the worker's to parse**, through the
    /// same shape rules interactive authoring runs, which is what keeps
    /// bulk from becoming a second validator.
    pub content: serde_json::Value,
}

/// The import door's body.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request)]
pub struct ImportBatchRequest {
    /// The batch's idempotency key, unique per tenant.
    pub batch_key: String,
    /// `import` (default) or `promote`.
    pub mode: Option<String>,
    /// The rows, in dependency order.
    pub rows: Vec<ImportRowRequest>,
}

/// What the import door answers.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct BatchAcceptedView {
    /// The batch's server-minted id.
    pub batch_id: Uuid,
    /// The caller's key, echoed.
    pub batch_key: String,
    /// The mode the batch runs under.
    pub mode: String,
    /// The batch's state — `staging` on a fresh batch, whatever the worker
    /// has made of it on a replay.
    pub state: String,
    /// How many rows the ledger holds.
    pub row_count: usize,
    /// Whether this answer replayed an existing batch rather than minting
    /// one.
    pub replayed: bool,
}

/// One ledger entry, as the reader reports it.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct RowLedgerEntryView {
    /// The caller's own key.
    pub row_key: String,
    /// The lane's client key.
    pub row_id: Uuid,
    /// `product` or `sku`.
    pub entity_kind: String,
    /// The entity, once minted.
    pub entity_id: Option<Uuid>,
    /// NULL while the row is in flight.
    pub disposition: Option<String>,
    /// The owning feature's code on a failure.
    pub code: Option<String>,
    /// A closed-set literal, never operator text.
    pub reason: Option<String>,
}

/// The ledger reader's answer.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct BatchLedgerView {
    /// The batch.
    pub batch_id: Uuid,
    /// The caller's key.
    pub batch_key: String,
    /// The mode.
    pub mode: String,
    /// The lane.
    pub lane: String,
    /// The state machine's current value.
    pub state: String,
    /// `05`'s record, once the report edge submitted it.
    pub approval_ref: Option<Uuid>,
    /// One entry per row — the no-hidden-partial-failure surface.
    pub rows: Vec<RowLedgerEntryView>,
}

/// The request body: the entity minus `requested_at`, which is the door's.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request)]
pub struct CreateIncrementRequestBody {
    /// The registered requester this demand belongs to.
    pub source: String,
    /// `interactive` or `bulk`.
    pub lane: String,
    /// The caller's idempotency handle.
    pub request_key: String,
    /// The bulk batch this request coalesces under; required exactly when
    /// `lane` is `bulk`.
    pub operation_key: Option<String>,
}

/// The acknowledgement body.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct IncrementAckView {
    /// `true` once the request's version has committed.
    pub coalesced: bool,
    /// The committed version, present exactly when `coalesced`.
    pub catalog_version_id: Option<i64>,
}

/// The ack and release doors' body: the participant, and nothing else.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request)]
pub struct FreezeParticipantRequest {
    /// The participant whose ledger row the door flips. Membership is the
    /// row's existence (P-D-67); the identity-binding half is owed (see
    /// the module doc).
    pub participant: String,
}

/// What the two participant doors answer.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct FreezeEdgeView {
    /// The participant acted on.
    pub participant: String,
    /// The participant's ledger state after the act.
    pub state: String,
    /// The version's refreshed derived cache.
    pub freeze_state: String,
}

/// One manifest entry, as the resolver serves it.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct ManifestEntryView {
    /// `product` or `sku`.
    pub entity_kind: String,
    /// The entity.
    pub entity_id: Uuid,
    /// The frozen version the manifest pins.
    pub published_version: i64,
}

/// One stored capture, as the resolver serves it.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct ManifestCaptureView {
    /// The capture kind.
    pub capture_kind: String,
    /// The stored canonical copy.
    pub content: String,
}

/// The resolver's answer: metadata, the stored manifest, the verifiable
/// checksum, and — when the caller named a differing `bound_version` — the
/// re-binding triple (`dod-version-binding`).
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct ResolvedVersionView {
    /// The resolved version.
    pub catalog_version_id: i64,
    /// Hex digest over the canonical manifest rendering — returned so the
    /// caller can re-verify (`inst-rv-bytes`).
    pub checksum: String,
    /// The digest rule the checksum was computed under.
    pub digest_version: i32,
    /// The commit instant.
    pub published_at: chrono::DateTime<Utc>,
    /// The strict flag (P-D-84 arm 3): `freeze_state = 'complete'` and
    /// nothing else.
    pub freeze_complete: bool,
    /// The storage truth behind the flag.
    pub freeze_state: String,
    /// The manifest's entry half.
    pub entries: Vec<ManifestEntryView>,
    /// The manifest's capture half.
    pub captures: Vec<ManifestCaptureView>,
    /// The participant snapshot, parsed from its own capture.
    pub participant_set: Vec<String>,
    /// The caller's bound version, echoed when it differed.
    pub bound_version: Option<i64>,
    /// The resolved version, repeated beside the bound one when they
    /// differed.
    pub resolved_version: Option<i64>,
    /// The diff door's ref grammar for the span, when the two differed.
    pub diff_ref: Option<String>,
}

/// The watermark door's body.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request)]
pub struct PostWatermarkRequest {
    /// The posting producer.
    pub producer: String,
    /// The instant the set is complete as of.
    pub watermark_at: DateTime<Utc>,
    /// The complete SKU set — never a delta.
    pub sku_ids: Vec<Uuid>,
}

/// What the watermark door answers.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct WatermarkAckView {
    /// The stored instant after the post.
    pub watermark_at: DateTime<Utc>,
    /// How many SKUs the stored set holds.
    pub member_count: usize,
    /// Whether this was the admitted idempotent replay.
    pub replayed: bool,
}

/// The registration door's body.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request)]
pub struct RegisterProducerRequest {
    /// The producer's name.
    pub producer: String,
}

/// What both membership ops answer.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct ProducerView {
    /// The producer.
    pub producer: String,
    /// `registered` or `retired`.
    pub state: String,
}
