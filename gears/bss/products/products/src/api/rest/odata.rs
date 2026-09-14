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
/// `cursor` as an alias of `$skiptoken`, so both spellings reach a door that
/// binds the extractor whatever else it declares. That is also why adopting
/// this module did not break the `limit` these doors already shipped.
const PAGINATION_ALIASES: [&str; 2] = ["limit", "cursor"];

/// Whether a door binds the platform's query family at all.
///
/// The distinction is load-bearing and its absence was a defect: the guard
/// used to permit [`PAGINATION_ALIASES`] unconditionally, so `?limit=10` on
/// a door that pages nothing was **dropped** and answered `200` with the
/// whole collection — the very shape this module exists to refuse, and one
/// the version-diff door's own comment claimed to have closed. A door that
/// serves no pagination has no more business accepting `limit` than
/// accepting `status`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum QueryFamily {
    /// The door binds [`toolkit::api::odata::OData`]: the `$` family is the
    /// extractor's to police, and `limit`/`cursor` are the platform's own
    /// spellings of two of its members.
    Odata,
    /// The door binds none of it — its only query keys are the operands it
    /// declares. Every `$` key and both pagination aliases are as
    /// undeclared here as an invented word, and are refused rather than
    /// ignored: the extractor is not there to refuse them either.
    OperandsOnly,
}

/// Refuse any query key that is neither served by this door's query family
/// nor declared by the door itself.
///
/// `declared` carries the door's own custom query options — the operands
/// that are not filters (`includeFacets`, `intent`, `principalRef`, …).
/// `family` says what else is admissible: on [`QueryFamily::Odata`] the `$`
/// keys are the extractor's to police and `limit`/`cursor` are two of its
/// members, and on [`QueryFamily::OperandsOnly`] neither is admissible at
/// all.
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
    family: QueryFamily,
    declared: &[&str],
) -> Result<(), DomainError> {
    let odata = family == QueryFamily::Odata;
    let mut offenders: Vec<&str> = raw
        .keys()
        .map(String::as_str)
        // On an `OData` door the extractor owns the `$` family, including
        // the refusal of what it does not bind, and it tells an unsupported
        // option from a typo better than a second check could. On a door
        // that binds no extractor there is no such owner, so a `$` key is
        // simply undeclared.
        .filter(|key| !(odata && key.starts_with('$')))
        .filter(|key| !(odata && PAGINATION_ALIASES.contains(key)))
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
                "unrecognized query parameter `{key}`. {}{}",
                if odata {
                    "This door accepts the OData family (`$filter`, `$orderby`, `$top`/`limit`, \
                     `$skiptoken`/`cursor`)"
                } else {
                    "This door serves no filtering, ordering or paging"
                },
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
        return " and no parameters of its own.".to_owned();
    }
    format!(" and this door's own parameters ({}).", declared.join(", "))
}

/// A `$` option this platform binds but **this door** does not serve.
pub(crate) const UNSUPPORTED_QUERY_OPTION: &str = "UNSUPPORTED_QUERY_OPTION";

/// Why no door in this gear serves `$select`.
///
/// The extractor parses and validates the field list, so without this every
/// paginated door accepted a projection, bound it to nothing, and answered
/// `200` with every field — the same silent-drop shape the rest of this
/// module refuses, one level down. P-D-165 recorded it as an open item; a
/// recorded defect is still a defect, and the fix is one call per door.
pub(crate) const NO_SELECT: &str = "this door does not project fields: every row carries its whole shape. Ask for the \
     fields you need by reading them off the response.";

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

/// Refuse a continuation token whose walk is not the one this door serves.
///
/// The order-option guard above tests `odata.order`, and the platform's
/// extractor **empties** that field whenever a cursor is present — the
/// effective order is then re-derived from the token's own signed field
/// list. A door that refuses `$orderby` by name therefore refuses only the
/// spelling: the token is unsigned base64url JSON, so a caller can put the
/// order it was denied inside one and be served it. On the version timeline
/// that is not merely a different order — each entry's `changed_keys` is the
/// diff against the version *before* it, so a descending walk diffs every
/// entry against the version above and the body is silently wrong.
///
/// `expected` is the order the door serves, as `paginate_odata` would see
/// it after `ensure_tiebreaker`.
///
/// # Errors
///
/// [`DomainError::Validation`] under [`INVALID_CURSOR`]: the token does not
/// describe a walk this door performs, which is the caller's to fix.
pub(crate) fn reject_cursor_reordering(
    odata: &toolkit_odata::ODataQuery,
    expected: &[(&str, toolkit_odata::SortDir)],
) -> Result<(), DomainError> {
    let Some(cursor) = odata.cursor.as_ref() else {
        return Ok(());
    };
    let carried = toolkit_odata::ODataOrderBy::from_signed_tokens(&cursor.s).map_err(|e| {
        let mut report = ValidationReport::new();
        report.violate(INVALID_CURSOR, "cursor", format!("unreadable cursor: {e}"));
        DomainError::Validation(report)
    })?;
    let matches = carried.0.len() == expected.len()
        && carried
            .0
            .iter()
            .zip(expected)
            .all(|(key, (field, dir))| key.field == *field && key.dir == *dir);
    if matches {
        return Ok(());
    }
    let mut report = ValidationReport::new();
    report.violate(
        INVALID_CURSOR,
        "cursor",
        format!(
            "this cursor describes an order this door does not serve; it is served in {} order \
             and no other",
            expected
                .iter()
                .map(|(field, dir)| format!("{field} {dir:?}").to_lowercase())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    );
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
