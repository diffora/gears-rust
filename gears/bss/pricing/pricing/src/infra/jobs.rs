//! Background jobs the gear's `stateful` `serve` loop drives.
//!
//! **They are independent**, and the module list below is the roster: each job has
//! its own coordination lease, its own cadence and its own pass, and none reads what
//! another writes. That independence is worth stating because it is what makes them
//! separate tickers rather than phases of one — a window flipping does not wait on a
//! warm, a warm does not wait on a window, and neither waits on a gauge refresh.
//!
//! **No count.** This paragraph opened *"Two, and they are independent"* and the
//! bullets named two of three for the two days after [`gated_markets`] landed
//! (F-6/Z10-8). A count in prose beside a roster in code leaves only one of the two
//! true, and it is never the prose — which is the correction the first bullet below
//! already carries for its own earlier count.
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
//! - [`gated_markets`] — D-246's catalog-wide GA backlog on D-250's cadence: count
//!   the tenant-markets a published tax-inclusive row gates while `TAX_ENGINE_GA`
//!   stands false, and publish it to `pricing_tax_not_sellable_ga`. **The one job
//!   here that is not load-bearing for correctness** — the gear serves every request
//!   without it — and what its absence costs is §7's alarm never firing, because the
//!   gauge is an observable over a cached value and an unrefreshed cache reports `0`
//!   while markets are gated.
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
