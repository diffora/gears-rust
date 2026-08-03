//! Background jobs the gear's `stateful` `serve` loop drives.
//!
//! One today, and it is the only thing that turns a publish into something a
//! consumer can pin: without a ticker nothing ever resolves the pending handle
//! the commit left behind, and `pricing_read_model` stays empty whatever else
//! is built.
//!
//! - [`readmodel_warm`] — §3.8's read-model warm re-drive: resolve pending
//!   `CatalogVersion` handles against the registry, drive
//!   [`ReadModelProjector`](crate::infra::read_model::ReadModelProjector) over
//!   each version, raise the two Critical alarms §3.6 and §4.4 name by string,
//!   and enqueue `PlanPublishDegraded` for a publish whose subject is still
//!   not warm.
//!
//! These are **system-context, cross-tenant** jobs, exactly as the sibling
//! ledger's are: they read across tenants under the sanctioned
//! [`AccessScope::allow_all`](toolkit_db::secure::AccessScope::allow_all) system
//! scope with the actor
//! [`SecurityContext::anonymous`](toolkit_security::SecurityContext::anonymous),
//! and **narrow to `AccessScope::for_tenant` before any per-tenant write**.

pub mod readmodel_warm;
