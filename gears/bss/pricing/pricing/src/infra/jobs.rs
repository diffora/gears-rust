//! Background jobs the gear's `stateful` `serve` loop drives.
//!
//! **Two, and they are independent**: each has its own coordination lease, its
//! own cadence and its own pass, and neither reads what the other writes. That
//! independence is worth stating because it is what makes them two tickers
//! rather than two phases of one — a window flipping does not wait on a warm,
//! and a warm does not wait on a window.
//!
//! - [`readmodel_warm`] — §3.8's read-model warm re-drive: resolve pending
//!   `CatalogVersion` handles against the registry, drive
//!   [`ReadModelProjector`](crate::infra::read_model::ReadModelProjector) over
//!   each version, raise the two Critical alarms §3.6 and §4.4 name by string,
//!   and enqueue `PlanPublishDegraded` for a publish whose subject is still
//!   not warm. **It is the only thing that turns a publish into something a
//!   consumer can pin**: without its ticker nothing ever resolves the pending
//!   handle the commit left behind, and `pricing_read_model` stays empty
//!   whatever else is built. (That sentence used to say "one today, and it is
//!   the only thing that…" — the exclusive half stopped being true the moment
//!   the second job landed, while the claim it was really making, about this
//!   job's own necessity, is unchanged. A count in prose beside a roster in
//!   code is the shape that goes stale, so the count is gone rather than
//!   corrected.)
//! - [`window_activation`] — `07-pricewindow-linkage.md` §4's two time-driven
//!   transitions: flip `scheduled → active` at `effectiveFrom` and
//!   `active → expired` at `effectiveTo`, emitting `PriceWindowActivated` /
//!   `PriceWindowExpired` from the outbox, ordered per `(tenant, plan)`, and
//!   raise the Warn alarm §7 names for a boundary that has stood uncrossed past
//!   the job SLO. It is **not** a publish unit and re-projects nothing
//!   (`inst-ws-publishunit`), which is what the read model carrying window
//!   *intervals* buys.
//!
//! These are **system-context, cross-tenant** jobs, exactly as the sibling
//! ledger's are: they read across tenants under the sanctioned
//! [`AccessScope::allow_all`](toolkit_db::secure::AccessScope::allow_all) system
//! scope with the actor
//! [`SecurityContext::anonymous`](toolkit_security::SecurityContext::anonymous),
//! and **narrow to `AccessScope::for_tenant` before any per-tenant write**.

pub mod gated_markets;
pub mod readmodel_warm;
pub mod window_activation;
