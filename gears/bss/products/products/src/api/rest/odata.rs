//! The gear's one query seam: how a list door binds filtering, ordering and
//! pagination (**P-D-165**).
//!
//! # Why the platform's contract and not this gear's own
//!
//! Every list door here used to bind a hand-rolled `Query<T>` struct of its
//! own — twelve bespoke keys on browse, `state`+`limit` on the approval
//! inbox, `state` on scheduled transitions, nothing at all on the two
//! timelines and the dashboards. Three consequences, none of them a matter
//! of taste:
//!
//! 1. **An unrecognized key was silently dropped.** `serde` ignores a field
//!    it does not know and Axum's `Query` extractor claims nothing it cannot
//!    bind, so `?status=approved` — or a mis-cased `?excludedeprecated=true`
//!    — answered **200 with the unfiltered set**. A caller cannot tell that
//!    answer from a correct one.
//! 2. **`limit` was a ceiling, not a page.** Past it there was no
//!    continuation of any kind: no cursor, no offset, no token. The inbox
//!    said `has_more: true` and gave nothing to continue with.
//! 3. **Five envelopes for one idea.** `{stamp, rows, facets}`,
//!    `{items, has_more}`, `{items}`, `{entries}`, and the export artifact.
//!
//! The `account-management` gear is the platform's canonical answer and this
//! module is products' adoption of it, part for part: the
//! [`toolkit::api::odata::OData`] extractor binds `$filter`, `$orderby`,
//! `$select`, `$top` and `$skiptoken` and **refuses** every other
//! `$`-prefixed key (OASIS `OData` 4.01 Part 1, §6.1: a service MUST fail a
//! request carrying an unsupported system query option); a typed
//! `FilterField` set per door makes an unknown field, a type mismatch or an
//! unsupported operator a 400 instead of a shrug;
//! `toolkit_db::odata::sea_orm_filter::paginate_odata` walks the keyset over
//! a `SecureSelect` that is already scoped; and every page answers with
//! [`toolkit_odata::PageInfo`] — `next_cursor`, `prev_cursor`, `limit`.
//!
//! # What this module adds on top, and why AM does not need it
//!
//! AM's guard is "any query key without a `$` is refused", because AM's list
//! doors have no operands beyond the `OData` family. Products' do:
//! `includeFacets` on browse, `intent`/`boundVersion` on the resolver,
//! `principalRef`/`justification` on the identity export, `catalogVersionId`
//! on the bulk export. Those are legal **custom query options** — `OData`
//! 4.01 Part 2, §11.2.1 reserves the `$` and `@` prefixes for the protocol
//! and leaves every other name to the service — so the guard here is the
//! same idea with a declared allow-list per door rather than an empty one:
//! [`reject_undeclared_query_params`].
//!
//! `$`-prefixed keys are deliberately out of this guard's scope. The
//! extractor already refuses the ones it does not bind, and it can tell an
//! *unsupported* option (`$skip` — offset paging, which the platform does
//! not serve) from an *unknown* one (`$filtre`), which is a better message
//! than anything a second check could produce.

use std::collections::HashMap;

use toolkit_canonical_errors::CanonicalError;
use toolkit_db::odata::sea_orm_filter::LimitCfg;

use crate::domain::error::DomainError;
use crate::domain::validation::ValidationReport;

/// A query key the door does not declare and the protocol does not reserve.
pub(crate) const UNDECLARED_QUERY_PARAM: &str = "UNDECLARED_QUERY_PARAM";

/// `$filter` names a field the door does not expose, compares it with an
/// operator its kind does not admit, or does not parse at all.
pub(crate) const INVALID_FILTER: &str = "INVALID_FILTER";

/// `$orderby` names a field the door does not expose or cannot order by.
pub(crate) const INVALID_ORDERBY: &str = "INVALID_ORDERBY";

/// The continuation token is not one this door minted, or the walk it
/// describes is no longer the walk being asked for.
pub(crate) const INVALID_CURSOR: &str = "INVALID_CURSOR";

/// The page size is not a number this door can serve.
pub(crate) const INVALID_LIMIT: &str = "INVALID_LIMIT";

/// The page every list door in this gear serves, and the largest one it will
/// serve on request.
///
/// The numbers are `account-management`'s, on all three of its listing
/// repositories, and they are taken rather than re-derived: a page size is a
/// property of what an operator console renders and what a body may weigh,
/// not of this gear's subject matter, and two gears answering different
/// numbers to the same `$top` is a difference a caller has to learn for no
/// return.
///
/// This **lowers** browse's shipped ceiling from 500 to 200. Before this
/// module the 500 was the only bound a caller had — there was no
/// continuation past it, so the ceiling had to double as the whole result
/// set. With `next_cursor` on every page the ceiling is a page size again
/// and the rest of the set is reachable, which is what makes the reduction
/// safe rather than a capability being taken away.
pub(crate) const LISTING_LIMIT_CFG: LimitCfg = LimitCfg {
    default: 50,
    max: 200,
};

/// The two non-`$` spellings the platform's extractor folds onto `$top` and
/// `$skiptoken`.
///
/// `toolkit`'s `ODataParams` binds `limit` as an alias of `$top` and
/// `cursor` as an alias of `$skiptoken`, so both spellings reach a door
/// whatever it declares. Every list door therefore permits them, which is
/// also why adopting this module did not break the `limit` these doors
/// already shipped.
const PAGINATION_ALIASES: [&str; 2] = ["limit", "cursor"];

/// Refuse any query key that is neither reserved by the protocol nor
/// declared by this door.
///
/// `declared` carries the door's own custom query options — the operands
/// that are not filters (`includeFacets`, `intent`, `principalRef`, …). The
/// pagination aliases are permitted everywhere and need not be listed.
///
/// Every offender is reported, not just the first: a caller who mis-spelled
/// two keys should learn both in one round trip, which is the same reason
/// [`ValidationReport`] carries the whole set (P-D-37). The offenders are
/// sorted so the refusal is a function of the request and not of the hash
/// map's iteration order.
///
/// # Errors
///
/// [`DomainError::Validation`] naming each undeclared key as its own
/// violation subject — rendered 400 with the key in `subject`, so a client
/// sees *which* parameter it invented without parsing prose.
pub(crate) fn reject_undeclared_query_params(
    raw: &HashMap<String, String>,
    declared: &[&str],
) -> Result<(), DomainError> {
    let mut offenders: Vec<&str> = raw
        .keys()
        .map(String::as_str)
        // The extractor owns the `$` family, including the refusal of what
        // it does not bind. See the module doc.
        .filter(|key| !key.starts_with('$'))
        .filter(|key| !PAGINATION_ALIASES.contains(key))
        .filter(|key| !declared.contains(key))
        .collect();
    if offenders.is_empty() {
        return Ok(());
    }
    offenders.sort_unstable();
    let mut report = ValidationReport::new();
    for key in offenders {
        report.violate(
            UNDECLARED_QUERY_PARAM,
            key,
            format!(
                "unrecognized query parameter `{key}`. This door accepts the OData family \
                 (`$filter`, `$orderby`, `$select`, `$top`/`limit`, `$skiptoken`/`cursor`){}",
                describe_declared(declared)
            ),
        );
    }
    Err(DomainError::Validation(report))
}

/// The tail of the refusal detail naming the door's own operands, or nothing
/// when it has none.
fn describe_declared(declared: &[&str]) -> String {
    if declared.is_empty() {
        return ". Filtering is `$filter`, not a bare field name.".to_owned();
    }
    format!(
        " and this door's own parameters ({}). Filtering is `$filter`, not a bare field name.",
        declared.join(", ")
    )
}

/// A `$` option this platform binds but **this door** does not serve.
pub(crate) const UNSUPPORTED_QUERY_OPTION: &str = "UNSUPPORTED_QUERY_OPTION";

/// Refuse the `OData` options a door binds nothing to.
///
/// The extractor's job ends at "is this an option the platform serves"; a
/// door that serves only a subset must say so itself, or it lands back in
/// the defect this whole module exists to stop — an option accepted, bound
/// to nothing, and answered `200` as though it had been applied.
///
/// `wanted` is a closure per option rather than a set of names because the
/// reason differs per door and the refusal has to carry it: "this timeline
/// is served in version order because each row's changed keys are computed
/// against its predecessor" is the kind of thing a caller cannot guess.
///
/// # Errors
///
/// [`DomainError::Validation`] naming each refused option in `subject`.
pub(crate) fn reject_unsupported_odata_options(
    odata: &toolkit_odata::ODataQuery,
    filter: Option<&str>,
    orderby: Option<&str>,
    select: Option<&str>,
) -> Result<(), DomainError> {
    let mut report = ValidationReport::new();
    if odata.filter.is_some()
        && let Some(reason) = filter
    {
        report.violate(UNSUPPORTED_QUERY_OPTION, "$filter", reason);
    }
    if !odata.order.is_empty()
        && let Some(reason) = orderby
    {
        report.violate(UNSUPPORTED_QUERY_OPTION, "$orderby", reason);
    }
    if odata.select.is_some()
        && let Some(reason) = select
    {
        report.violate(UNSUPPORTED_QUERY_OPTION, "$select", reason);
    }
    if report.violations().is_empty() {
        return Ok(());
    }
    Err(DomainError::Validation(report))
}

/// Classify a pagination failure by whose mistake it was.
///
/// The helper's error type mixes the caller's four ways of writing an
/// unservable query with two failures of the service, and they owe different
/// answers: a rejected `$filter` is a 400 that must name `$filter`, while a
/// driver failure mid-walk is a 500 that must not blame the caller for it
/// (the provenance rule the `CorruptRow` class earned).
pub(crate) fn odata_error_to_canonical(door: &str, err: &toolkit_odata::Error) -> CanonicalError {
    use toolkit_odata::Error as E;

    let (code, subject) = match err {
        E::InvalidFilter(_) => (INVALID_FILTER, "$filter"),
        E::InvalidOrderByField(_) => (INVALID_ORDERBY, "$orderby"),
        E::InvalidLimit => (INVALID_LIMIT, "$top"),
        // Every way the continuation token can fail to describe this walk.
        // `OrderMismatch`/`FilterMismatch` are the caller changing `$orderby`
        // or `$filter` half-way through a walk, and `OrderWithCursor` is
        // sending both at once: all of them are "this token is not for this
        // request", which is what the caller has to fix.
        E::OrderMismatch
        | E::FilterMismatch
        | E::InvalidCursor
        | E::OrderWithCursor
        | E::CursorInvalidBase64
        | E::CursorInvalidJson
        | E::CursorInvalidVersion
        | E::CursorInvalidKeys
        | E::CursorInvalidFields
        | E::CursorInvalidDirection => (INVALID_CURSOR, "cursor"),
        // Not the caller's: a driver failure, or a build without the parser.
        E::Db(_) | E::ParsingUnavailable(_) => {
            return CanonicalError::internal(format!("products: {door} pagination failed: {err}"))
                .create();
        }
    };
    let mut report = ValidationReport::new();
    report.violate(code, subject, format!("{door}: {err}"));
    DomainError::Validation(report).into()
}

#[cfg(test)]
#[path = "odata_tests.rs"]
mod tests;
