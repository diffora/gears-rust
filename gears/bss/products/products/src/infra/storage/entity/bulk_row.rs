//! `SeaORM` entity for `bss.products_bulk_row` — the `RowLedger`
//! (`design/09-bulk-promotion.md` §4, P-D-69): the row idempotency store,
//! batch-scoped, and the no-hidden-partial-failure surface. A row with a
//! `disposition` is immutable; the migration's trigger holds it.

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_bulk_row")]
#[secure(tenant_col = "tenant_id", resource_col = "row_key", no_owner, no_type)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub batch_id: Uuid,
    /// The caller's own key for this row, **batch-scoped**.
    #[sea_orm(primary_key, auto_increment = false)]
    pub row_key: String,
    /// The ledger row's surrogate id — the `internal:bulk-row` lane's
    /// `client_key` (P-D-69), UNIQUE so the lane's key resolves one row.
    pub row_id: Uuid,
    /// `product` or `sku` today; live-entity kinds arrive with 02.
    pub entity_kind: String,
    /// The entity the row acted on, once minted.
    pub entity_id: Option<Uuid>,
    /// The revision the row pinned, for an update-as-draft row.
    pub pinned_revision: Option<i64>,
    /// The row's imported content, canonically serialized (**P-D-86**) —
    /// what the worker parses and stages. `NULL` only for a live-entity
    /// row, whose payload is `governed_live_op`'s.
    pub staged_payload: Option<String>,
    /// `published`, `applied`, `no_op` or `failed` — NULL while in flight.
    pub disposition: Option<String>,
    /// The owning feature's code verbatim on a failure (no parallel
    /// taxonomy).
    pub code: Option<String>,
    /// A literal from a closed set, never operator text (P-D-50).
    pub reason: Option<String>,
    /// The governed live operation this row carries, for a live-entity row.
    pub governed_live_op: Option<String>,
    /// Whether the batch override ceremony acknowledged this row.
    pub override_acknowledged: bool,
    /// Stamped with the disposition, the two moving together by CHECK.
    pub terminal_at: Option<ChronoDateTimeUtc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
