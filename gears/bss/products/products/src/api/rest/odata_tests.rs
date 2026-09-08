//! Probes for the query seam.
//!
//! The guard exists because the defect it prevents is invisible: before it,
//! `?status=approved` on a list door answered **200 with the unfiltered
//! set**, and no assertion anywhere could tell that response from a correct
//! one. So the probes here are written against the refusal's *content* — the
//! violation subject naming the key the caller invented — and not merely
//! against the fact that something was refused.

use std::collections::HashMap;

use toolkit::api::canonical_prelude::CanonicalError;

use super::{
    INVALID_CURSOR, INVALID_FILTER, INVALID_LIMIT, INVALID_ORDERBY, LISTING_LIMIT_CFG,
    UNDECLARED_QUERY_PARAM, odata_error_to_canonical, reject_undeclared_query_params,
};
use crate::domain::error::DomainError;

/// Build the raw query map an Axum `Query<HashMap<String, String>>` would
/// hand a door.
fn raw(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

/// The `(type, subject)` pairs a refusal carries, in the order the response
/// renders them — read the way a consumer reads them off the envelope.
fn violations(err: &DomainError) -> Vec<(String, String)> {
    let canonical = CanonicalError::from(err.clone());
    match canonical {
        CanonicalError::FailedPrecondition { ctx, .. } => ctx
            .violations
            .iter()
            .map(|v| (v.type_.clone(), v.subject.clone()))
            .collect(),
        other => panic!("expected a precondition refusal, got {other:?}"),
    }
}

#[test]
fn an_undeclared_key_is_refused_and_names_itself() {
    let err = reject_undeclared_query_params(&raw(&[("status", "approved")]), &[])
        .expect_err("a key no door declared must not be admitted");
    assert_eq!(
        violations(&err),
        vec![(UNDECLARED_QUERY_PARAM.to_owned(), "status".to_owned())],
        "the refusal must name the invented key in `subject`, not only in prose"
    );
}

/// The mis-cased spelling of a real parameter is the realistic accident, and
/// the one that used to read as "no filter asked for".
#[test]
fn a_miscased_declared_key_is_still_undeclared() {
    let err = reject_undeclared_query_params(
        &raw(&[("excludedeprecated", "true")]),
        &["excludeDeprecated"],
    )
    .expect_err("case is part of the parameter's name on the wire");
    assert_eq!(
        violations(&err),
        vec![(
            UNDECLARED_QUERY_PARAM.to_owned(),
            "excludedeprecated".to_owned()
        )]
    );
}

#[test]
fn a_declared_operand_is_admitted() {
    reject_undeclared_query_params(&raw(&[("includeFacets", "true")]), &["includeFacets"])
        .expect("a door's own custom query option is not an accident");
}

/// The two aliases the platform's extractor folds onto `$top` and
/// `$skiptoken` reach every door whatever it declares, so the guard permits
/// them without being told. This is also what kept the `limit` these doors
/// already shipped working when the seam landed.
#[test]
fn the_pagination_aliases_need_no_declaration() {
    reject_undeclared_query_params(&raw(&[("limit", "10"), ("cursor", "abc")]), &[])
        .expect("`limit` and `cursor` are the platform's own spellings");
}

/// The `$` family belongs to the extractor, which can distinguish an
/// unsupported option from a typo. A second refusal here would replace that
/// message with a worse one.
#[test]
fn a_dollar_key_is_left_to_the_extractor() {
    reject_undeclared_query_params(
        &raw(&[
            ("$filter", "name eq 'a'"),
            ("$orderby", "name asc"),
            ("$skip", "10"),
            ("$filtre", "typo"),
        ]),
        &[],
    )
    .expect("this guard is the seam for non-OData accidents only");
}

/// Two mistakes cost one round trip, and the refusal is a function of the
/// request rather than of the hash map's iteration order — which is what
/// makes this assertion on the exact sequence meaningful instead of flaky.
#[test]
fn every_offender_is_named_in_a_stable_order() {
    let err = reject_undeclared_query_params(
        &raw(&[("zeta", "1"), ("alpha", "2"), ("includeFacets", "true")]),
        &["includeFacets"],
    )
    .expect_err("two undeclared keys are two violations");
    assert_eq!(
        violations(&err)
            .into_iter()
            .map(|(_, subject)| subject)
            .collect::<Vec<_>>(),
        vec!["alpha".to_owned(), "zeta".to_owned()]
    );
}

/// The detail must tell a caller what the door *does* accept, including its
/// own operands — otherwise the refusal is correct and useless.
#[test]
fn the_refusal_names_the_doors_own_parameters() {
    let err = reject_undeclared_query_params(&raw(&[("facets", "true")]), &["includeFacets"])
        .expect_err("`facets` is not `includeFacets`");
    let CanonicalError::FailedPrecondition { ctx, .. } = CanonicalError::from(err) else {
        panic!("expected a precondition refusal");
    };
    let detail = &ctx.violations[0].description;
    assert!(
        detail.contains("$filter") && detail.contains("includeFacets"),
        "the detail must name both the OData family and this door's operands, got: {detail}"
    );
}

#[test]
fn nothing_undeclared_is_nothing_to_refuse() {
    reject_undeclared_query_params(&raw(&[]), &[]).expect("an empty query is a valid query");
}

/// Each caller-side failure names the parameter the caller has to fix. A
/// rejected `$filter` blaming `cursor` would send a client looking in the
/// wrong place.
#[test]
fn each_caller_failure_names_its_own_parameter() {
    use toolkit_odata::Error as E;

    let cases: Vec<(E, &str, &str)> = vec![
        (
            E::InvalidFilter("bad".to_owned()),
            INVALID_FILTER,
            "$filter",
        ),
        (
            E::InvalidOrderByField("nope".to_owned()),
            INVALID_ORDERBY,
            "$orderby",
        ),
        (E::InvalidLimit, INVALID_LIMIT, "$top"),
        (E::OrderMismatch, INVALID_CURSOR, "cursor"),
        (E::FilterMismatch, INVALID_CURSOR, "cursor"),
        (E::InvalidCursor, INVALID_CURSOR, "cursor"),
        (E::OrderWithCursor, INVALID_CURSOR, "cursor"),
        (E::CursorInvalidBase64, INVALID_CURSOR, "cursor"),
        (E::CursorInvalidJson, INVALID_CURSOR, "cursor"),
        (E::CursorInvalidVersion, INVALID_CURSOR, "cursor"),
        (E::CursorInvalidKeys, INVALID_CURSOR, "cursor"),
        (E::CursorInvalidFields, INVALID_CURSOR, "cursor"),
        (E::CursorInvalidDirection, INVALID_CURSOR, "cursor"),
    ];
    for (err, code, subject) in cases {
        let rendered = format!("{err:?}");
        let canonical = odata_error_to_canonical("browse", &err);
        let CanonicalError::FailedPrecondition { ctx, .. } = canonical else {
            panic!("{rendered} must be the caller's mistake, not the service's");
        };
        assert_eq!(
            (
                ctx.violations[0].type_.as_str(),
                ctx.violations[0].subject.as_str()
            ),
            (code, subject),
            "{rendered} was classified wrongly"
        );
    }
}

/// A driver failure mid-walk is not the caller's doing, and rendering it as a
/// 400 would both mislead the client and hide the outage from the operator.
#[test]
fn a_service_failure_is_not_the_callers_fault() {
    use toolkit_odata::Error as E;

    for err in [
        E::Db("connection reset".to_owned()),
        E::ParsingUnavailable("built without the parser"),
    ] {
        let rendered = format!("{err:?}");
        let canonical = odata_error_to_canonical("browse", &err);
        assert!(
            matches!(canonical, CanonicalError::Internal { .. }),
            "{rendered} must render 500, got {canonical:?}"
        );
    }
}

/// The page contract this gear answers, pinned as a number rather than left
/// to whatever the last edit set: `account-management`'s own, on all three
/// of its listing repositories.
#[test]
fn the_page_contract_is_the_platforms() {
    assert_eq!(
        (LISTING_LIMIT_CFG.default, LISTING_LIMIT_CFG.max),
        (50, 200)
    );
}
