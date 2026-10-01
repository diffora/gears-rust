//! HTTP surface of the inbox.

pub mod rest;

use std::sync::Arc;

use toolkit::ClientHub;

/// What a request needs: the configured source names and the hub they are registered in.
pub struct ApiState {
    /// Config order.
    pub sources: Vec<String>,
    /// Scoped `ApprovalSourceV1` clients, one scope per source name.
    pub hub: Arc<ClientHub>,
}
