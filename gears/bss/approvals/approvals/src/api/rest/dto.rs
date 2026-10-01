//! Wire shapes. They follow the inbox unit; the vote answer is the owning door's bytes.

use bss_approvals_sdk::{
    DecisionKind, InboxDecision, InboxKind, InboxUnit, KindCounts, SourceCounts, StateCounts,
    UnitState,
};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::domain::read::{SourceHealth, SourceRow};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[toolkit_macros::api_dto(response)]
pub enum KindDto {
    #[serde(rename = "prices")]
    Prices,
    #[serde(rename = "plan_revision")]
    PlanRevision,
    #[serde(rename = "sku_publish")]
    SkuPublish,
    #[serde(rename = "sku_change")]
    SkuChange,
    #[serde(rename = "sku_retire")]
    SkuRetire,
}

impl From<InboxKind> for KindDto {
    fn from(kind: InboxKind) -> Self {
        match kind {
            InboxKind::Prices => Self::Prices,
            InboxKind::PlanRevision => Self::PlanRevision,
            InboxKind::SkuPublish => Self::SkuPublish,
            InboxKind::SkuChange => Self::SkuChange,
            InboxKind::SkuRetire => Self::SkuRetire,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[toolkit_macros::api_dto(response)]
pub enum StateDto {
    #[serde(rename = "pending")]
    Pending,
    #[serde(rename = "approved")]
    Approved,
    #[serde(rename = "rejected")]
    Rejected,
    #[serde(rename = "withdrawn")]
    Withdrawn,
}

impl From<UnitState> for StateDto {
    fn from(state: UnitState) -> Self {
        match state {
            UnitState::Pending => Self::Pending,
            UnitState::Approved => Self::Approved,
            UnitState::Rejected => Self::Rejected,
            UnitState::Withdrawn => Self::Withdrawn,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[toolkit_macros::api_dto(response)]
pub enum DecisionKindDto {
    #[serde(rename = "approve")]
    Approve,
    #[serde(rename = "reject")]
    Reject,
}

impl From<DecisionKind> for DecisionKindDto {
    fn from(kind: DecisionKind) -> Self {
        match kind {
            DecisionKind::Approve => Self::Approve,
            DecisionKind::Reject => Self::Reject,
        }
    }
}

#[toolkit_macros::api_dto(response)]
pub struct DecisionDto {
    pub actor: Uuid,
    pub generation: i32,
    pub decision: DecisionKindDto,
    pub note: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String)]
    pub at: OffsetDateTime,
    pub stale: bool,
}

impl From<InboxDecision> for DecisionDto {
    fn from(decision: InboxDecision) -> Self {
        Self {
            actor: decision.actor,
            generation: decision.generation,
            decision: decision.decision.into(),
            note: decision.note,
            at: decision.at,
            stale: decision.stale,
        }
    }
}

#[toolkit_macros::api_dto(response)]
pub struct UnitDto {
    pub id: Uuid,
    pub source: String,
    pub kind: KindDto,
    pub ref_type: String,
    pub ref_id: Uuid,
    pub state: StateDto,
    pub generation: i32,
    pub quorum_required: u32,
    pub common_effective_date: Option<String>,
    pub submitted_by: Uuid,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String)]
    pub submitted_at: OffsetDateTime,
    pub submit_note: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>)]
    pub decided_at: Option<OffsetDateTime>,
    pub decided_note: Option<String>,
    pub snapshot: serde_json::Value,
    pub decisions: Vec<DecisionDto>,
    pub caller_can_approve: bool,
    pub subject_live: Option<serde_json::Value>,
    pub impact: Option<serde_json::Value>,
}

impl From<InboxUnit> for UnitDto {
    fn from(unit: InboxUnit) -> Self {
        Self {
            id: unit.id,
            source: unit.source,
            kind: unit.kind.into(),
            ref_type: unit.ref_type,
            ref_id: unit.ref_id,
            state: unit.state.into(),
            generation: unit.generation,
            quorum_required: unit.quorum_required,
            common_effective_date: unit.common_effective_date,
            submitted_by: unit.submitted_by,
            submitted_at: unit.submitted_at,
            submit_note: unit.submit_note,
            decided_at: unit.decided_at,
            decided_note: unit.decided_note,
            snapshot: unit.snapshot,
            decisions: unit.decisions.into_iter().map(Into::into).collect(),
            caller_can_approve: unit.caller_can_approve,
            subject_live: unit.subject_live,
            impact: unit.impact,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[toolkit_macros::api_dto(response)]
pub enum SourceStatusDto {
    #[serde(rename = "ok")]
    Ok,
    #[serde(rename = "forbidden")]
    Forbidden,
}

impl From<SourceHealth> for SourceStatusDto {
    fn from(status: SourceHealth) -> Self {
        match status {
            SourceHealth::Ok => Self::Ok,
            SourceHealth::Forbidden => Self::Forbidden,
        }
    }
}

#[toolkit_macros::api_dto(response)]
pub struct SourceDto {
    pub name: String,
    pub status: SourceStatusDto,
}

impl From<SourceRow> for SourceDto {
    fn from(row: SourceRow) -> Self {
        Self {
            name: row.name,
            status: row.status.into(),
        }
    }
}

#[toolkit_macros::api_dto(response)]
pub struct UnitListDto {
    pub items: Vec<UnitDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub sources: Vec<SourceDto>,
}

#[toolkit_macros::api_dto(response)]
pub struct StateCountsDto {
    pub pending: u64,
    pub approved: u64,
    pub rejected: u64,
    pub withdrawn: u64,
}

impl From<StateCounts> for StateCountsDto {
    fn from(counts: StateCounts) -> Self {
        Self {
            pending: counts.pending,
            approved: counts.approved,
            rejected: counts.rejected,
            withdrawn: counts.withdrawn,
        }
    }
}

#[toolkit_macros::api_dto(response)]
pub struct KindCountsDto {
    pub prices: u64,
    pub plan_revision: u64,
    pub sku_publish: u64,
    pub sku_change: u64,
    pub sku_retire: u64,
}

impl From<KindCounts> for KindCountsDto {
    fn from(counts: KindCounts) -> Self {
        Self {
            prices: counts.prices,
            plan_revision: counts.plan_revision,
            sku_publish: counts.sku_publish,
            sku_change: counts.sku_change,
            sku_retire: counts.sku_retire,
        }
    }
}

#[toolkit_macros::api_dto(response)]
pub struct CountsDto {
    pub by_state: StateCountsDto,
    pub by_kind: KindCountsDto,
    pub total: u64,
    pub sources: Vec<SourceDto>,
}

impl CountsDto {
    #[must_use]
    pub fn from_counts(counts: SourceCounts, sources: Vec<SourceRow>) -> Self {
        Self {
            by_state: counts.by_state.into(),
            by_kind: counts.by_kind.into(),
            total: counts.total,
            sources: sources.into_iter().map(Into::into).collect(),
        }
    }
}
