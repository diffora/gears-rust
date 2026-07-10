//! Domain output ports for the usage-collector module.
//!
//! Ports are the domain-layer contracts that infra adapters implement,
//! keeping the domain free of transport / vendor types (`OTel`, HTTP, …).

pub mod metrics;

pub use metrics::{
    AuthzDecision, DeactivationErrorCategory, IngestRequestErrorCategory, IngestRequestOutcome,
    NoopMetrics, PdpFailureCause, PdpOp, PluginErrorCategory, PluginOp, QueryErrorCategory,
    QueryKind, RecordErrorCategory, RecordKind, RecordOutcome, RequestOutcome,
    UsageCollectorMetrics, UsageTypeErrorCategory, UsageTypeOp,
};
