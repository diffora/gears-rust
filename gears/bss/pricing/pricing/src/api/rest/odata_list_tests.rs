//! Unit tests for the pricing OData list extra-key guard.

use std::collections::HashMap;

use toolkit_odata::ODataQuery;

use super::{refuse_zero_limit, reject_non_odata_list_params};

#[test]
fn extra_key_guard_allows_odata_and_pagination() {
    let mut q = HashMap::new();
    q.insert(
        "$filter".to_owned(),
        "lifecycle_state eq 'draft'".to_owned(),
    );
    q.insert("$orderby".to_owned(), "plan_id".to_owned());
    q.insert("limit".to_owned(), "100".to_owned());
    q.insert("cursor".to_owned(), "abc".to_owned());
    reject_non_odata_list_params(&q).expect("allowed keys must pass");
}

#[test]
fn extra_key_guard_rejects_lifecycle_state() {
    let mut q = HashMap::new();
    q.insert("lifecycle_state".to_owned(), "draft".to_owned());
    let err = reject_non_odata_list_params(&q).expect_err("named lifecycle_state must reject");
    let detail = format!("{err}");
    assert!(detail.contains("lifecycle_state"), "{detail}");
    assert!(detail.contains("$filter"), "{detail}");
}

#[test]
fn extra_key_guard_rejects_status() {
    let mut q = HashMap::new();
    q.insert("status".to_owned(), "OPEN".to_owned());
    let err = reject_non_odata_list_params(&q).expect_err("plain status must reject");
    assert!(format!("{err}").contains("status"));
}

#[test]
fn refuse_zero_limit_rejects_zero() {
    let err =
        refuse_zero_limit(&ODataQuery::default().with_limit(0)).expect_err("limit=0 must be 400");
    let detail = format!("{err}");
    assert!(detail.contains("limit must be at least 1"), "{detail}");
}

#[test]
fn refuse_zero_limit_allows_unset_and_one() {
    refuse_zero_limit(&ODataQuery::default()).expect("unset limit must pass");
    refuse_zero_limit(&ODataQuery::default().with_limit(1)).expect("limit=1 must pass");
}
