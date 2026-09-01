//! `SeaORM` entity for `bss.products_bulk_batch` — the batch head
//! (`design/09-bulk-promotion.md` §4, P-D-54, P-D-61, P-D-69). Working
//! state by design: the worker flips `state`, stamps `claimed_at`, bumps
//! `attempt` and writes `terminal_at`; the discipline is the CHECKs and the
//! machine's edges, the immutability living on the ledger rows instead.

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_bulk_batch")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "batch_key",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub batch_id: Uuid,
    /// The import door's idempotency operand, UNIQUE per tenant.
    pub batch_key: String,
    /// `import` or `promote` (P-D-69): only `promote` engages the
    /// `PromotionResolver`'s update-as-draft.
    pub mode: String,
    /// `import` or `lifecycle`.
    pub lane: String,
    /// The seven-state machine (P-D-54 plus P-D-69's `abandoned`).
    pub state: String,
    /// The idempotency key of the act that created the batch, where one
    /// was carried.
    pub operation_key: Option<String>,
    /// `05-governance`'s approval record, when the batch reports. No FK:
    /// that table is 05's and does not ship.
    pub approval_ref: Option<Uuid>,
    /// The worker's claim stamp.
    pub claimed_at: Option<ChronoDateTimeUtc>,
    /// The worker's attempt counter, against its budget.
    pub attempt: i64,
    pub created_at: ChronoDateTimeUtc,
    /// Stamped when the batch reaches a terminal state.
    pub terminal_at: Option<ChronoDateTimeUtc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
