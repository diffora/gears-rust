//! The mass-repricing run's two surfaces, over HTTP
//! (`design/12-operator-efficiency.md` §5, `inst-mr-api`, `inst-mr-journal`,
//! `inst-mr-return`, `inst-mp-grandfathered`; D-88, D-134, D-261, D-307).
//!
//! Every case asserts what the **store** holds and not only what the response
//! said. A `202` carrying a journal assembled from the caller's own request would
//! be a receipt for a freeze that may not have happened — the defect D-225 records
//! for the overlay submit — so the frozen row set is read back through the `GET`,
//! which is also `inst-mr-return`'s progress endpoint and therefore the thing
//! under test rather than a convenience. The two edges out of `validating`
//! (`inst-mr-coalesce`) are asserted the same way: on the **stored** run and the
//! **stored** approval unit, never on the response body alone — a route that
//! answered a materiality literal it had never computed would still pass a test
//! that only read the response, which is exactly the defect a 2026-08-10 review
//! records for the bundle publish's own materiality assertion.
//!
//! **What this suite still cannot assert**: that an *approved* run's apply
//! runs. `open_repricing_run`'s own hook (task 5) spends the non-material
//! edge's apply synchronously, in the same request — so a case here that
//! leaves `validating` for `committing` now also asserts the terminal state
//! the apply reaches, and the cases that do say so. An *approved* run's apply
//! is `api::rest::approvals`' arm instead, spent when a second principal
//! decides the batch unit, which is that surface's own suite's to cover.
//!
//! **No case in this file configures a threshold policy unless its own name says
//! so.** A fresh harness has none, so `inst-mat-failsafe` makes every ordinary run
//! here material and therefore `awaiting_approval` — which is what lets the cases
//! that predate the two edges keep asserting the row set and the audit record
//! without also standing up a policy fixture for a fact they are not about.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;
mod rest_support;

use axum::http::StatusCode;
use bss_pricing::api::rest::repricing_runs::REPRICING_RUNS;
use bss_pricing::domain::bulk::BulkState;
use bss_pricing::domain::lifecycle::LifecycleState;
use bss_pricing::domain::money::RateMinor;
use bss_pricing::domain::scope_key::{Cohort, PriceEligibility};
use chrono::{TimeZone, Utc};
use rest_support::{
    Harness, approval_rows, approve_threshold_policy, body_json, bulk_operation_row, price_rows,
    problem_code, seed_current_plan, seed_current_plan_with_phase, seed_per_unit_rate_row,
    seed_price, seed_price_keyed, seed_priced_row, with_headers,
};
use uuid::Uuid;

/// Far enough out that no wall clock reaches it, the fixtures' standing rule. It
/// matters more here than elsewhere: the changeover is judged against `Utc::now()`
/// at every submit, so a relatively-dated instant would make this suite go red on
/// its own one day.
const CHANGEOVER: &str = "2099-08-20T00:00:00Z";

fn run_path(run_id: Uuid) -> String {
    format!("{REPRICING_RUNS}/{run_id}")
}

/// A whole-currency run: `discount 5%`, one axis named.
fn a_run(run_id: Uuid, selector: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "run_id": run_id,
        "selector": selector,
        "adjustment": {
            "adjustment_kind": "discount",
            "magnitude_kind": "percent_bp",
            "adjustment_value": 500,
        },
        "changeover": CHANGEOVER,
    })
}

/// A published row on a named region of a named plan, which is what a run selects.
async fn a_published_row(harness: &Harness, plan: Uuid, region: &str) -> Uuid {
    let row = seed_price(harness, plan, region).await;
    harness.publish_price(plan, row.price_id).await;
    row.price_id
}

#[tokio::test]
async fn a_run_opens_over_the_published_rows_and_freezes_them_pending() {
    let harness = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_current_plan(&harness, plan).await;
    let eu = a_published_row(&harness, plan, "eu").await;
    let us = a_published_row(&harness, plan, "us").await;
    // A **draft** row on the same plan and currency. It must not be selected: the
    // run's domain is the published plane (D-118 gives the draft plane to the bulk
    // import), and a draft in the journal would be a row the apply would supersede
    // without it ever having been current on its key.
    let draft = seed_price(&harness, plan, "apac").await;

    let run_id = Uuid::now_v7();
    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            REPRICING_RUNS,
            Some(a_run(run_id, &serde_json::json!({ "currency": "USD" }))),
            &[],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::ACCEPTED, "inst-mr-return");
    let body = body_json(response).await;
    assert_eq!(body["run_id"], serde_json::json!(run_id.to_string()));
    assert_eq!(
        body["state"],
        serde_json::json!("awaiting_approval"),
        "a fresh harness configures no threshold policy, so `inst-mat-failsafe` makes this run \
         material and it leaves `validating` for `awaiting_approval` on the same request: {body}"
    );

    // The answer lives at the progress endpoint, and this is where the freeze is
    // actually observed: the `POST`'s own journal is read from the store too, so
    // both renderings come from the row set that landed.
    let read = harness
        .allowed()
        .send(with_headers("GET", &run_path(run_id), None, &[]))
        .await;
    assert_eq!(read.status(), StatusCode::OK);
    let run = body_json(read).await;
    let journal = run["journal"].as_array().expect("a journal");
    let selected: Vec<&str> = journal
        .iter()
        .map(|row| row["price_id"].as_str().expect("an id"))
        .collect();
    let mut expected = vec![eu.to_string(), us.to_string()];
    expected.sort();
    assert_eq!(selected, expected, "{run}");
    assert!(
        !selected.contains(&draft.price_id.to_string().as_str()),
        "a draft row is not a repricing run's business: {run}"
    );
    assert!(
        journal
            .iter()
            .all(|row| row["state"] == serde_json::json!("pending")),
        "every frozen row is born pending (D-261): {run}"
    );

    // The report is the run's frozen parameters, rendered from what was parsed —
    // the apply reads them from here, because `pricing_bulk_operation` has no
    // other column for them.
    assert_eq!(run["report"]["selected"], serde_json::json!(2));
    assert_eq!(
        run["report"]["selector"]["currency"],
        serde_json::json!("USD")
    );
    assert_eq!(
        run["report"]["adjustment"]["adjustment_kind"],
        serde_json::json!("discount")
    );
    assert_eq!(
        run["report"]["adjustment"]["adjustment_value"],
        serde_json::json!(500)
    );
}

#[tokio::test]
async fn a_selector_that_matches_nothing_is_refused_and_opens_no_run() {
    // §5's `RUN_SELECTOR_EMPTY`. The refusal exists because the *success* is
    // indistinguishable from it: a run is complete when no `pending` rows remain,
    // so a run over an empty set is complete at birth and would report a mass
    // adjustment that never happened.
    let harness = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_current_plan(&harness, plan).await;
    a_published_row(&harness, plan, "eu").await;

    let run_id = Uuid::now_v7();
    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            REPRICING_RUNS,
            Some(a_run(
                run_id,
                &serde_json::json!({ "region": "antarctica" }),
            )),
            &[],
        ))
        .await;

    // A 400 and not a 422: Foundation section 3.3 gives the platform no 422
    // category, and the code is the discriminator.
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        problem_code(response).await.contains("RUN_SELECTOR_EMPTY"),
        "the declared code names what happened"
    );

    // **And the `run_id` is not spent.** This is the half a status assertion
    // cannot see: the refusal happens before anything opens, so a corrected
    // selector may reuse the id — which is the whole reason for refusing ahead of
    // the open rather than opening a run and failing it.
    let read = harness
        .allowed()
        .send(with_headers("GET", &run_path(run_id), None, &[]))
        .await;
    assert_eq!(
        read.status(),
        StatusCode::NOT_FOUND,
        "a refused selector opened nothing under this run_id"
    );
}

#[tokio::test]
async fn a_changeover_that_is_not_in_the_future_is_refused_at_the_submit_floor() {
    // `inst-mr-api` gives the run's changeover `inst-su-instant`'s floors, and this
    // is the submit one: strictly future. The commit floor — a whole batching delay
    // ahead — belongs to an approval commit that does not exist yet, so a run is
    // held to the lenient bound here and the strict one is owed by the apply.
    let harness = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_current_plan(&harness, plan).await;
    a_published_row(&harness, plan, "eu").await;

    let run_id = Uuid::now_v7();
    let mut body = a_run(run_id, &serde_json::json!({ "currency": "USD" }));
    body["changeover"] = serde_json::json!(
        Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0)
            .unwrap()
            .to_rfc3339()
    );

    let response = harness
        .allowed()
        .send(with_headers("POST", REPRICING_RUNS, Some(body), &[]))
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        problem_code(response)
            .await
            .contains("SUPERSESSION_INSTANT_PASSED"),
        "the floor is the supersession's, and so is its code -- no second code over one bound"
    );

    let read = harness
        .allowed()
        .send(with_headers("GET", &run_path(run_id), None, &[]))
        .await;
    assert_eq!(
        read.status(),
        StatusCode::NOT_FOUND,
        "a run refused at the floor opened nothing"
    );
}

#[tokio::test]
async fn a_second_post_under_one_run_id_answers_the_run_it_opened() {
    // §5's Idempotency cell for this surface is `run_id`, so the replay is a read
    // of the run's own unique `(tenant, kind, client_key)` rather than a dedup
    // row — `bulk_imports`' arrangement, one path over.
    let harness = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_current_plan(&harness, plan).await;
    a_published_row(&harness, plan, "eu").await;
    a_published_row(&harness, plan, "us").await;

    let run_id = Uuid::now_v7();
    let first = harness
        .allowed()
        .send(with_headers(
            "POST",
            REPRICING_RUNS,
            Some(a_run(run_id, &serde_json::json!({ "currency": "USD" }))),
            &[],
        ))
        .await;
    assert_eq!(first.status(), StatusCode::ACCEPTED);
    let first = body_json(first).await;

    // The retry names a **different selector**, which is what makes this a replay
    // test rather than a repeat: the answer must be the run that was opened, not a
    // run over the set the second request described.
    let second = harness
        .allowed()
        .send(with_headers(
            "POST",
            REPRICING_RUNS,
            Some(a_run(run_id, &serde_json::json!({ "region": "eu" }))),
            &[],
        ))
        .await;
    assert_eq!(second.status(), StatusCode::ACCEPTED);
    let second = body_json(second).await;

    assert_eq!(
        second["operation_id"], first["operation_id"],
        "one run_id opens one run"
    );
    assert_eq!(
        second["journal"].as_array().expect("a journal").len(),
        2,
        "the replayed journal is the frozen one, not one re-expanded from the retry: {second}"
    );
    assert_eq!(
        second["report"]["selector"]["region"],
        serde_json::Value::Null,
        "and the frozen report is the first call's: {second}"
    );

    // The freeze happened once. Read back through the store's own ordering, a
    // second `open_rows` over the same set would have collided on the journal's
    // primary key and a *different* set would have doubled the row count.
    let read = harness
        .allowed()
        .send(with_headers("GET", &run_path(run_id), None, &[]))
        .await;
    let run = body_json(read).await;
    assert_eq!(run["journal"].as_array().expect("a journal").len(), 2);
}

#[tokio::test]
async fn the_grandfathered_class_is_excluded_unless_the_selector_names_it() {
    // `inst-mp-grandfathered` clause 1. A grandfathered row is immutable in price
    // (Foundation section 4.3), so an operator repricing "all USD rows" has not
    // asked to break a retention promise -- the class is excluded structurally.
    let harness = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_current_plan(&harness, plan).await;
    let ordinary = a_published_row(&harness, plan, "eu").await;

    let generation = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let retained = seed_price_keyed(
        &harness,
        plan,
        "eu-2025",
        PriceEligibility::ExistingGrandfathered,
        Cohort::Generation(generation),
    )
    .await;
    harness.publish_price(plan, retained.price_id).await;
    assert_eq!(
        retained.scope_key.price_eligibility(),
        PriceEligibility::ExistingGrandfathered,
        "the fixture really is on the retained class; `ScopeKey::new` is what pairs it with the \
         cohort, and a fixture on the wrong class would make both halves below vacuous"
    );

    let wide = Uuid::now_v7();
    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            REPRICING_RUNS,
            Some(a_run(wide, &serde_json::json!({ "currency": "USD" }))),
            &[],
        ))
        .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let body = body_json(response).await;
    let selected: Vec<&str> = body["journal"]
        .as_array()
        .expect("a journal")
        .iter()
        .map(|row| row["price_id"].as_str().expect("an id"))
        .collect();
    assert_eq!(
        selected,
        vec![ordinary.to_string()],
        "the retained generation is not in a run that did not name its class: {body}"
    );

    // Clause 2's other half: naming the class **selects** those rows rather than
    // dropping them, because dropping them is the silent skip the clause forbids.
    // The per-row refusal is owed by the apply -- a journal row cannot be born
    // `failed` (D-261) -- so what this asserts is that the rows reach the journal
    // where that refusal will be written, and not that they are refused today.
    let narrow = Uuid::now_v7();
    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            REPRICING_RUNS,
            Some(a_run(
                narrow,
                &serde_json::json!({ "price_eligibility": "existing_grandfathered" }),
            )),
            &[],
        ))
        .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let body = body_json(response).await;
    let selected: Vec<&str> = body["journal"]
        .as_array()
        .expect("a journal")
        .iter()
        .map(|row| row["price_id"].as_str().expect("an id"))
        .collect();
    assert_eq!(
        selected,
        vec![retained.price_id.to_string()],
        "an explicit inclusion is never a silent skip: {body}"
    );
}

#[tokio::test]
async fn a_cohort_without_its_class_is_refused_with_the_reason_the_axes_do_not_show() {
    // The one selector that is empty **by construction** rather than by what the
    // tenant published: a cohort exists only on the retained class, and the
    // retained class is excluded until `price_eligibility` names it. The two rules
    // live in different documents, so a bare "matched nothing" would send its
    // author hunting the catalog for rows that are sitting right there -- which is
    // the failure mode `RUN_SELECTOR_EMPTY` shares with every count-based refusal.
    let harness = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_current_plan(&harness, plan).await;
    let generation = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let retained = seed_price_keyed(
        &harness,
        plan,
        "eu-2025",
        PriceEligibility::ExistingGrandfathered,
        Cohort::Generation(generation),
    )
    .await;
    harness.publish_price(plan, retained.price_id).await;

    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            REPRICING_RUNS,
            Some(a_run(
                Uuid::now_v7(),
                &serde_json::json!({ "cohort": generation.to_rfc3339() }),
            )),
            &[],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let problem = body_json(response).await.to_string();
    assert!(
        problem.contains("RUN_SELECTOR_EMPTY") && problem.contains("existing_grandfathered"),
        "the refusal names the axis that would make this selector reachable: {problem}"
    );

    // And the same cohort **with** the class is the request they meant, which is
    // what makes the sentence above advice rather than an apology.
    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            REPRICING_RUNS,
            Some(a_run(
                Uuid::now_v7(),
                &serde_json::json!({
                    "cohort": generation.to_rfc3339(),
                    "price_eligibility": "existing_grandfathered",
                }),
            )),
            &[],
        ))
        .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let body = body_json(response).await;
    assert_eq!(
        body["journal"]
            .as_array()
            .expect("a journal")
            .first()
            .expect("one row")["price_id"],
        serde_json::json!(retained.price_id.to_string()),
        "{body}"
    );
}

#[tokio::test]
async fn an_unknown_axis_token_is_refused_before_the_selector_is_ever_expanded() {
    // A token outside the eligibility enumeration would otherwise match no row and
    // be reported `RUN_SELECTOR_EMPTY`, which sends an operator hunting for missing
    // rows when what they have is a typo. The axis is read through the same
    // `optional_token` the interactive plane uses, so the refusal enumerates the
    // admitted values.
    let harness = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_current_plan(&harness, plan).await;
    a_published_row(&harness, plan, "eu").await;

    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            REPRICING_RUNS,
            Some(a_run(
                Uuid::now_v7(),
                &serde_json::json!({ "price_eligibility": "everyone" }),
            )),
            &[],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    // The **whole** problem document, not one member of it: this asserts an
    // absence, and reading a single field would let the code it denies sit in a
    // sibling one -- which is where `RUN_SELECTOR_EMPTY` actually renders.
    let problem = body_json(response).await.to_string();
    assert!(
        !problem.contains("RUN_SELECTOR_EMPTY") && !problem.contains("matched no published"),
        "a malformed axis is a malformed request, not an empty result: {problem}"
    );
    assert!(
        problem.contains("existing_grandfathered"),
        "and the refusal enumerates the tokens the axis admits, from the same slice the store's \
         CHECK was written against: {problem}"
    );
}

#[tokio::test]
async fn a_fixed_adjustment_declared_percent_bp_is_refused_by_the_shared_parser() {
    // D-138, through `overlays::adjustment_of` -- the same function the overlay
    // line parser calls. It is asserted here and not only there because a run's
    // adjustment is **not** persisted as an overlay line, so none of the store
    // constraints that back the overlay plane's parse are behind this one: a copy
    // of the parse that drifted would drift silently.
    let harness = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_current_plan(&harness, plan).await;
    a_published_row(&harness, plan, "eu").await;

    let mut body = a_run(Uuid::now_v7(), &serde_json::json!({ "currency": "USD" }));
    body["adjustment"]["adjustment_kind"] = serde_json::json!("fixed");

    let response = harness
        .allowed()
        .send(with_headers("POST", REPRICING_RUNS, Some(body), &[]))
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn one_run_id_opens_one_repricing_run_and_one_bulk_import_alike() {
    // D-307's index half, over HTTP. The two flows have two different idempotency
    // columns (§5), so one token may name one of each -- and a `find_by_client_key`
    // missing its `kind` filter would answer this `GET` with the *import*, which
    // carries no `kind` member on either view to reveal the substitution.
    let harness = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_current_plan(&harness, plan).await;
    a_published_row(&harness, plan, "eu").await;

    let shared = Uuid::now_v7();
    let import = harness
        .allowed()
        .send(with_headers(
            "POST",
            bss_pricing::api::rest::bulk_imports::BULK_IMPORTS,
            Some(serde_json::json!({ "rows": [] })),
            &[("idempotency-key", shared.to_string().as_str())],
        ))
        .await;
    assert_eq!(import.status(), StatusCode::ACCEPTED);
    let import = body_json(import).await;

    let run = harness
        .allowed()
        .send(with_headers(
            "POST",
            REPRICING_RUNS,
            Some(a_run(shared, &serde_json::json!({ "currency": "USD" }))),
            &[],
        ))
        .await;
    assert_eq!(run.status(), StatusCode::ACCEPTED);
    let run = body_json(run).await;

    assert_ne!(
        run["operation_id"], import["operation_id"],
        "one client key, two flows, two runs"
    );

    let read = harness
        .allowed()
        .send(with_headers("GET", &run_path(shared), None, &[]))
        .await;
    assert_eq!(read.status(), StatusCode::OK);
    assert_eq!(
        body_json(read).await["operation_id"],
        run["operation_id"],
        "and the progress endpoint answers the repricing run, never the import"
    );
}

/// The debt `api::rest::repricing_runs`' own module doc named: until
/// `AuditSubjectKind` carried a `BulkOperation` member, opening a run wrote **no**
/// audit record at all. `subject_ref` is asserted equal to
/// `audit_repo::bulk_operation_ref(operation_id)` rather than merely "a row
/// exists" — a record naming the wrong subject would verify perfectly and tell an
/// auditor nothing true.
#[tokio::test]
async fn opening_a_run_writes_an_audit_record_naming_the_operation_id() {
    let harness = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_current_plan(&harness, plan).await;
    a_published_row(&harness, plan, "eu").await;

    let run_id = Uuid::now_v7();
    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            REPRICING_RUNS,
            Some(a_run(run_id, &serde_json::json!({ "currency": "USD" }))),
            &[],
        ))
        .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let body = body_json(response).await;
    let operation_id = body["operation_id"]
        .as_str()
        .expect("the view carries the minted id")
        .to_owned();

    // A fresh harness configures no threshold policy, so `inst-mat-failsafe`
    // makes this run material and it opens an approval unit too — which appends
    // its **own** `bulk_operation`-subject `submit` record on the same chain
    // (D-158). Filtered to `create` because that is the one this case is about;
    // the `submit` record is `a_material_run_...`'s own assertion.
    let bulk_records: Vec<_> = rest_support::audit_rows(&harness)
        .await
        .into_iter()
        .filter(|row| row.subject_kind == "bulk_operation" && row.action == "create")
        .collect();
    assert_eq!(
        bulk_records.len(),
        1,
        "one create record for the one run this test opened: {bulk_records:?}"
    );
    let record = &bulk_records[0];
    assert_eq!(record.action, "create");
    assert_eq!(
        record.subject_ref, operation_id,
        "the record must name the run it is about, not merely exist"
    );
    assert!(
        record.before_state.is_none(),
        "a run's open has no before-state"
    );
    assert_eq!(
        record
            .after_state
            .as_ref()
            .and_then(|state| state.get("kind")),
        Some(&serde_json::json!("repricing"))
    );
}

/// `inst-mr-coalesce`'s material edge: `validating -> awaiting_approval` under an
/// opened approval unit. A fresh harness configures no threshold policy, so
/// `inst-mat-failsafe` is what makes this run material — asserted on the
/// **stored** run (`bulk_operation_row`, a repository read, never the `POST`'s
/// own response) and the **stored** approval unit (`approval_rows`), per the
/// review finding this suite's own module doc names: a response-only assertion
/// cannot tell a handler that actually evaluated materiality from one that
/// answers a literal.
#[tokio::test]
async fn a_material_run_leaves_validating_for_awaiting_approval_with_an_approval_unit_open() {
    let harness = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_current_plan(&harness, plan).await;
    a_published_row(&harness, plan, "eu").await;

    let run_id = Uuid::now_v7();
    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            REPRICING_RUNS,
            Some(a_run(run_id, &serde_json::json!({ "currency": "USD" }))),
            &[],
        ))
        .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    // The response supplies only the id the store is then read back by — never
    // the fact under test.
    let operation_id = body_json(response).await["operation_id"]
        .as_str()
        .expect("the view carries the minted id")
        .to_owned();

    let stored = bulk_operation_row(&harness, operation_id.parse().expect("a uuid")).await;
    assert_eq!(
        stored.state,
        BulkState::AwaitingApproval,
        "no policy is configured, so the fail-safe trips: {stored:?}"
    );

    let units: Vec<_> = approval_rows(&harness)
        .await
        .into_iter()
        .filter(|row| row.subject_kind == "bulk_operation" && row.subject_ref == operation_id)
        .collect();
    assert_eq!(
        units.len(),
        1,
        "one approval unit for the one material run this test opened: {units:?}"
    );
    let unit = &units[0];
    assert_eq!(unit.state, "submitted");
    assert_eq!(
        unit.materiality.get("reason"),
        Some(&serde_json::json!("noConfiguredThreshold")),
        "the reason a reviewer would read: {unit:?}"
    );
}

/// `inst-mr-coalesce`'s non-material edge: `validating -> committing`, with
/// **no** approval unit opened, and — since the apply's own `open_repricing_run`
/// hook (`api::rest::repricing_runs`, task 5) spends that edge synchronously —
/// `committing -> completed` in the same request. A test that only proved the
/// material run above stops would pass against a handler that stops every run;
/// this one fails such a handler, because it asserts both that the run reaches
/// `committing` on its way through and that the approval store holds nothing
/// for it.
#[tokio::test]
async fn a_non_material_run_leaves_validating_for_committing_with_no_approval_unit_open() {
    let harness = Harness::new().await;
    // Any bar at all is below nothing for a zero-delta act (`rest_support`'s own
    // doc on `approve_threshold_policy`): what matters is only that USD has an
    // entry, not its size.
    approve_threshold_policy(&harness, &[("USD", 1_000_000)]).await;
    let plan = Uuid::now_v7();
    seed_current_plan_with_phase(&harness, plan).await;
    // `seed_price`/`a_published_row` leave `amount_minor` unset, which is a real
    // `NotComputable("amount_minor")` delta and would make this run material via
    // `alwaysMaterialTrigger` whatever the threshold says — the positive control
    // needs a row the per-currency comparison can actually compare.
    let priced = seed_priced_row(&harness, plan, "eu", 9_900).await;
    harness.publish_price(plan, priced.price_id).await;
    // `inst-wc-required`, via the apply's own aggregate pass: a supersession
    // presupposes current coverage, and `harness.publish_price` — the raw
    // `publish_rows` door, not the full pipeline — schedules none. Every
    // fixture in this crate that is meant to publish **cleanly** through the
    // real pipeline calls this (`tests/common/mod.rs`'s own module doc).
    common::schedule_coverage_window(
        &harness.db.conn().expect("conn"),
        &harness.scope(),
        harness.tenant,
        priced.price_id,
        rest_support::seed_stamp(),
    )
    .await;

    let run_id = Uuid::now_v7();
    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            REPRICING_RUNS,
            Some(a_run(run_id, &serde_json::json!({ "currency": "USD" }))),
            &[],
        ))
        .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let operation_id = body_json(response).await["operation_id"]
        .as_str()
        .expect("the view carries the minted id")
        .to_owned();

    let stored = bulk_operation_row(&harness, operation_id.parse().expect("a uuid")).await;
    assert_eq!(
        stored.state,
        BulkState::Completed,
        "USD has a configured entry and the zero-delta act never reaches it, so the run is \
         auto-publishable, and the apply that answers the same request has nothing to refuse \
         it on: {stored:?}"
    );

    let units: Vec<_> = approval_rows(&harness)
        .await
        .into_iter()
        .filter(|row| row.subject_kind == "bulk_operation" && row.subject_ref == operation_id)
        .collect();
    assert!(
        units.is_empty(),
        "an auto-publishable run opens no approval unit: {units:?}"
    );
}

/// `inst-mr-coalesce`'s own claim, armed directly: *"any row over its
/// own-currency threshold trips the run"* — the run's own **adjustment
/// magnitude**, not merely whether a policy exists. One fixture (one seeded
/// row, one configured threshold) carries two runs that differ only in the
/// `percent_bp` they submit: the smaller stays under the bar and the larger
/// crosses it. A change set built from zero-delta rows would answer
/// `noConfiguredThreshold` or `autoPublishable` identically for both, whatever
/// the adjustment said — which is exactly the gap a 2026-08-11 review found in
/// this suite's first draft, before either case here existed.
#[tokio::test]
async fn the_runs_own_adjustment_magnitude_against_the_configured_threshold_decides_materiality() {
    let harness = Harness::new().await;
    // $10.00 in USD's own currency: below the row's 5% move (500 minor) and
    // above nothing a discount could reach — the bar the pair straddles.
    approve_threshold_policy(&harness, &[("USD", 1_000)]).await;
    let plan = Uuid::now_v7();
    seed_current_plan_with_phase(&harness, plan).await;
    let priced = seed_priced_row(&harness, plan, "eu", 10_000).await;
    harness.publish_price(plan, priced.price_id).await;
    // `inst-wc-required`: the under-the-bar run below is non-material and its
    // apply runs synchronously in the same request, so the row needs real
    // coverage for that apply to have anything to supersede.
    common::schedule_coverage_window(
        &harness.db.conn().expect("conn"),
        &harness.scope(),
        harness.tenant,
        priced.price_id,
        rest_support::seed_stamp(),
    )
    .await;

    // Under the bar: 5% of 10_000 is 500 minor, and 500 < 1_000. Run first,
    // while nothing holds the row's key.
    let under_id = Uuid::now_v7();
    let mut under_body = a_run(under_id, &serde_json::json!({ "currency": "USD" }));
    under_body["adjustment"]["adjustment_kind"] = serde_json::json!("markup");
    under_body["adjustment"]["adjustment_value"] = serde_json::json!(500);
    let under_response = harness
        .allowed()
        .send(with_headers("POST", REPRICING_RUNS, Some(under_body), &[]))
        .await;
    assert_eq!(under_response.status(), StatusCode::ACCEPTED);
    let under_operation_id = body_json(under_response).await["operation_id"]
        .as_str()
        .expect("the view carries the minted id")
        .to_owned();

    let under_stored =
        bulk_operation_row(&harness, under_operation_id.parse().expect("a uuid")).await;
    assert_eq!(
        under_stored.state,
        BulkState::Completed,
        "500 minor is under the 1_000 bar, so the run is auto-publishable and the same \
         request's apply has nothing to refuse it on: {under_stored:?}"
    );
    let under_units: Vec<_> = approval_rows(&harness)
        .await
        .into_iter()
        .filter(|row| row.subject_kind == "bulk_operation" && row.subject_ref == under_operation_id)
        .collect();
    assert!(
        under_units.is_empty(),
        "under the bar opens no unit: {under_units:?}"
    );

    // Over the bar: 20% of 10_000 is 2_000 minor, and 2_000 >= 1_000. Same row,
    // same policy — only the adjustment's own size differs from the case above.
    let over_id = Uuid::now_v7();
    let mut over_body = a_run(over_id, &serde_json::json!({ "currency": "USD" }));
    over_body["adjustment"]["adjustment_kind"] = serde_json::json!("markup");
    over_body["adjustment"]["adjustment_value"] = serde_json::json!(2_000);
    let over_response = harness
        .allowed()
        .send(with_headers("POST", REPRICING_RUNS, Some(over_body), &[]))
        .await;
    assert_eq!(over_response.status(), StatusCode::ACCEPTED);
    let over_operation_id = body_json(over_response).await["operation_id"]
        .as_str()
        .expect("the view carries the minted id")
        .to_owned();

    let over_stored =
        bulk_operation_row(&harness, over_operation_id.parse().expect("a uuid")).await;
    assert_eq!(
        over_stored.state,
        BulkState::AwaitingApproval,
        "2_000 minor reaches the 1_000 bar: {over_stored:?}"
    );
    let over_units: Vec<_> = approval_rows(&harness)
        .await
        .into_iter()
        .filter(|row| row.subject_kind == "bulk_operation" && row.subject_ref == over_operation_id)
        .collect();
    assert_eq!(
        over_units.len(),
        1,
        "over the bar opens exactly one unit: {over_units:?}"
    );
    assert_eq!(
        over_units[0].materiality.get("reason"),
        Some(&serde_json::json!("thresholdReached")),
        "the real per-row comparison tripped it, not the fail-safe: {:?}",
        over_units[0]
    );
}

/// Pins `project_amount`'s rounding **rule**, not merely its arithmetic —
/// half-to-even (`round_half_even`), not truncation. `10_007` minor at
/// `1_000 bp` (10%) is `1_000.7` minor: the one case the pair above cannot
/// reach, since `500`/`10_000` and `2_000`/`10_000` both divide evenly and no
/// rounding rule contributes any error either way. Here the remainder (`7_000`
/// of `10_000`) is past the halfway point, so half-to-even rounds the move up
/// to `1_001` and it **reaches** the `1_001` bar; truncation over the
/// identical arithmetic would have given `1_000` and stayed **under** it. The
/// threshold sits exactly on the half-to-even answer and one short of the
/// truncated one, so reverting the rounding rule reddens this test.
#[tokio::test]
async fn the_projected_amount_rounds_half_to_even_not_toward_zero() {
    let harness = Harness::new().await;
    approve_threshold_policy(&harness, &[("USD", 1_001)]).await;
    let plan = Uuid::now_v7();
    seed_current_plan(&harness, plan).await;
    let priced = seed_priced_row(&harness, plan, "eu", 10_007).await;
    harness.publish_price(plan, priced.price_id).await;

    let run_id = Uuid::now_v7();
    let mut body = a_run(run_id, &serde_json::json!({ "currency": "USD" }));
    body["adjustment"]["adjustment_kind"] = serde_json::json!("markup");
    body["adjustment"]["adjustment_value"] = serde_json::json!(1_000);
    let response = harness
        .allowed()
        .send(with_headers("POST", REPRICING_RUNS, Some(body), &[]))
        .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let operation_id = body_json(response).await["operation_id"]
        .as_str()
        .expect("the view carries the minted id")
        .to_owned();

    let stored = bulk_operation_row(&harness, operation_id.parse().expect("a uuid")).await;
    assert_eq!(
        stored.state,
        BulkState::AwaitingApproval,
        "10_007 * 1_000 bp / 10_000 is exactly 1_000.7 minor: half-to-even rounds the move up to \
         1_001, which reaches the 1_001 bar, so this run is material. Truncation over the \
         identical arithmetic would have given 1_000 and stayed under it -- this assertion is the \
         rounding rule, not merely that some computation ran: {stored:?}"
    );

    let units: Vec<_> = approval_rows(&harness)
        .await
        .into_iter()
        .filter(|row| row.subject_kind == "bulk_operation" && row.subject_ref == operation_id)
        .collect();
    assert_eq!(
        units.len(),
        1,
        "the rounded-up move reaches the bar and opens exactly one unit: {units:?}"
    );
    assert_eq!(
        units[0].materiality.get("reason"),
        Some(&serde_json::json!("thresholdReached")),
        "the real per-row comparison tripped it: {:?}",
        units[0]
    );
}

// ---------------------------------------------------------------------------
// D-311's `per_unit` half: the money is a rate, and both the apply and the
// materiality verdict have to see it move.
// ---------------------------------------------------------------------------

/// The rate every case below seeds, in stored 10⁻⁹ minor units — `$0.230777165`
/// per unit.
///
/// Sub-minor-unit, the reason `RateMinor` exists at all, and deliberately
/// **not** a round count of nano-minor units: `23_077_701_650 x 700 bp` is
/// `1_615_439_115.5`, a remainder of exactly half over an **odd** quotient, so
/// half-to-even rounds the move up to `1_615_439_116` where truncation would
/// have stopped one nano-minor short. The pinned successor below therefore
/// separates four implementations — the no-op this defect was, a truncating
/// projection, one routed through `project_amount`'s coarse minor-unit scale,
/// and the correct one.
const PER_UNIT_RATE_NANO: i64 = 23_077_701_650;
/// `PER_UNIT_RATE_NANO` after `+7%`, half-to-even.
const PER_UNIT_RATE_AFTER_MARKUP_NANO: i64 = 24_693_140_766;
/// The `percent_bp` both cases submit. The move it produces is `1.615439116`
/// whole minor units, and the two bars below straddle it: `1` is crossed, `2`
/// is not.
const PER_UNIT_MARKUP_BP: i64 = 700;

/// A `+7%` run over one named currency.
fn a_markup_run(run_id: Uuid) -> serde_json::Value {
    let mut body = a_run(run_id, &serde_json::json!({ "currency": "USD" }));
    body["adjustment"]["adjustment_kind"] = serde_json::json!("markup");
    body["adjustment"]["adjustment_value"] = serde_json::json!(PER_UNIT_MARKUP_BP);
    body
}

/// **Consequence 1**: the apply writes a successor whose rate actually moved.
///
/// D-311 moved a `per_unit` row's money out of `amount_minor` into `unit_rate`
/// and made `amount_minor` NULL by rule on such a row
/// (`check_amount_placement`), while `project_row` went on repricing
/// `per_unit` through the shared `flat` arm — an `and_then` over a field that
/// is guaranteed absent, so it short-circuited and the row came back
/// unchanged. The apply then superseded a published row and published a
/// **byte-identical** successor in its place, writing the audit record, the
/// outbox events and a journal marked `applied` for a reprice that repriced
/// nothing.
///
/// The bar is `2` minor units and the move is `1.615439116`, so this run is
/// under it and non-material — which is what makes the apply run
/// **synchronously in this same request** (`open_repricing_run`'s own hook)
/// and gives this case a successor to read. The assertion is on the **stored**
/// successor's rate, never on the response: a run that reported `completed`
/// having written a copy of its predecessor passes every weaker check.
#[tokio::test]
async fn a_per_unit_rows_rate_moves_through_the_apply() {
    let harness = Harness::new().await;
    // 2 minor units, in USD's own currency. `AmountMove::reaches_absolute`
    // raises the bar into the move's nano-minor scale (D-311), so the
    // comparison is `1.615439116 < 2` and the run is auto-publishable.
    approve_threshold_policy(&harness, &[("USD", 2)]).await;
    let plan = Uuid::now_v7();
    seed_current_plan_with_phase(&harness, plan).await;
    let priced = seed_per_unit_rate_row(&harness, plan, "eu", PER_UNIT_RATE_NANO).await;
    harness.publish_price(plan, priced.price_id).await;
    // `inst-wc-required`: the apply supersedes, and a supersession presupposes
    // current coverage the raw publish door never schedules.
    common::schedule_coverage_window(
        &harness.db.conn().expect("conn"),
        &harness.scope(),
        harness.tenant,
        priced.price_id,
        rest_support::seed_stamp(),
    )
    .await;

    let run_id = Uuid::now_v7();
    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            REPRICING_RUNS,
            Some(a_markup_run(run_id)),
            &[],
        ))
        .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let operation_id = body_json(response).await["operation_id"]
        .as_str()
        .expect("the view carries the minted id")
        .to_owned();

    let stored = bulk_operation_row(&harness, operation_id.parse().expect("a uuid")).await;
    assert_eq!(
        stored.state,
        BulkState::Completed,
        "1.615439116 minor units is under the 2-minor bar, so the run auto-publishes and its \
         apply spends in the same request: {stored:?}"
    );

    let rows = price_rows(&harness, plan).await;
    let successor = rows
        .iter()
        .find(|row| row.supersedes_price_id == Some(priced.price_id))
        .unwrap_or_else(|| {
            panic!("the apply writes exactly one successor for the row it superseded: {rows:?}")
        });
    assert_eq!(
        successor.row.unit_rate.map(RateMinor::nano_minor),
        Some(PER_UNIT_RATE_AFTER_MARKUP_NANO),
        "+7% of {PER_UNIT_RATE_NANO} nano-minor is 1_615_439_115.5, half-to-even 1_615_439_116. A \
         successor still at {PER_UNIT_RATE_NANO} is the shared-`flat`-arm no-op this case exists \
         for -- an apply that superseded a published row and republished it unchanged while its \
         journal said `applied`: {successor:?}"
    );
    assert!(
        successor.row.amount_minor.is_none(),
        "the successor keeps its money in the one column its kind prices from: two priced columns \
         are AMOUNT_PLACEMENT_INVALID's two competing prices: {successor:?}"
    );
}

/// **Consequence 2**, and the one that reaches consumers approver-free: a
/// `per_unit` run whose rate move crosses the configured bar is judged
/// **material** and stops for a second principal.
///
/// `run_materiality` projects through the same `project_row`, so while that
/// function left `unit_rate` alone every `per_unit` row's delta was a rate
/// compared against itself — zero, under every bar — and the run skipped the
/// approval gate entirely on its way to `committing`. The fixture is the same
/// row and the same `+7%` as the case above; only the bar differs, `1` minor
/// unit instead of `2`, and `1.615439116 >= 1`. A projection that leaves the
/// rate published answers `autoPublishable` here whatever the bar says, so
/// this case is red against the defect and green only against a real
/// comparison.
#[tokio::test]
async fn a_per_unit_runs_rate_move_crosses_the_configured_threshold() {
    let harness = Harness::new().await;
    approve_threshold_policy(&harness, &[("USD", 1)]).await;
    let plan = Uuid::now_v7();
    seed_current_plan_with_phase(&harness, plan).await;
    let priced = seed_per_unit_rate_row(&harness, plan, "eu", PER_UNIT_RATE_NANO).await;
    harness.publish_price(plan, priced.price_id).await;

    let run_id = Uuid::now_v7();
    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            REPRICING_RUNS,
            Some(a_markup_run(run_id)),
            &[],
        ))
        .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let operation_id = body_json(response).await["operation_id"]
        .as_str()
        .expect("the view carries the minted id")
        .to_owned();

    let stored = bulk_operation_row(&harness, operation_id.parse().expect("a uuid")).await;
    assert_eq!(
        stored.state,
        BulkState::AwaitingApproval,
        "the rate moves 1.615439116 minor units and the bar is 1: a run that reached `committing` \
         here is one whose projection never moved the rate, so its delta was zero and it skipped \
         the approval gate on a change consumers do see: {stored:?}"
    );

    let units: Vec<_> = approval_rows(&harness)
        .await
        .into_iter()
        .filter(|row| row.subject_kind == "bulk_operation" && row.subject_ref == operation_id)
        .collect();
    assert_eq!(
        units.len(),
        1,
        "crossing the bar opens exactly one unit: {units:?}"
    );
    assert_eq!(
        units[0].materiality.get("reason"),
        Some(&serde_json::json!("thresholdReached")),
        "the real per-currency comparison tripped it, not the no-policy fail-safe -- a policy is \
         configured here precisely so the fail-safe cannot supply this verdict: {:?}",
        units[0]
    );
}

/// The refusal half, on `per_unit`'s field: an adjustment that computes **no**
/// rate mutation must fail the row rather than mark it `applied` over a
/// successor identical to its predecessor.
///
/// `project_rate` returns `None` for an `amount` markup/discount and for a
/// `fixed` line, and `project_row` reads that as "leave the rate published" —
/// which on the apply side is indistinguishable from the defect above unless
/// something refuses. `apply_rows_in` is that something, and its `matches!`
/// named `graduated`/`volume` only: the arm that would have caught `per_unit`
/// was never reached, because while `per_unit` shared `flat`'s arm no
/// adjustment of any kind moved such a row and the omission showed up as
/// nothing.
///
/// A `fixed` line is the sharpest case: it is the one adjustment that reads a
/// currency minor-unit literal as a price outright, and applied to a rate it
/// would silently reinterpret `$0.50` as a rate scale that can never express
/// the sub-minor-unit value the column exists for.
#[tokio::test]
async fn a_fixed_adjustment_fails_a_per_unit_row_rather_than_applying_nothing_to_it() {
    let harness = Harness::new().await;
    // A bar the zero delta this adjustment projects can never reach, so the run
    // is judged non-material and its apply spends in the same request — which
    // is what puts the refusal under test rather than the approval gate.
    approve_threshold_policy(&harness, &[("USD", 1_000_000)]).await;
    let plan = Uuid::now_v7();
    seed_current_plan_with_phase(&harness, plan).await;
    let priced = seed_per_unit_rate_row(&harness, plan, "eu", PER_UNIT_RATE_NANO).await;
    harness.publish_price(plan, priced.price_id).await;
    common::schedule_coverage_window(
        &harness.db.conn().expect("conn"),
        &harness.scope(),
        harness.tenant,
        priced.price_id,
        rest_support::seed_stamp(),
    )
    .await;

    let run_id = Uuid::now_v7();
    let mut body = a_run(run_id, &serde_json::json!({ "currency": "USD" }));
    body["adjustment"] = serde_json::json!({
        "adjustment_kind": "fixed",
        "magnitude_kind": "amount",
        "amounts": [{ "currency": "USD", "value_minor": 50 }],
    });
    let response = harness
        .allowed()
        .send(with_headers("POST", REPRICING_RUNS, Some(body), &[]))
        .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let operation_id = body_json(response).await["operation_id"]
        .as_str()
        .expect("the view carries the minted id")
        .to_owned();

    let stored = bulk_operation_row(&harness, operation_id.parse().expect("a uuid")).await;
    assert_eq!(
        stored.state,
        BulkState::CompletedWithConflicts,
        "a row this run cannot honestly reprice is a conflict, not a success: {stored:?}"
    );

    let read = harness
        .allowed()
        .send(with_headers("GET", &run_path(run_id), None, &[]))
        .await;
    assert_eq!(read.status(), StatusCode::OK);
    let run = body_json(read).await;
    let journal = run["journal"].as_array().expect("a journal");
    assert_eq!(journal.len(), 1, "one selected row: {run}");
    assert_eq!(
        journal[0]["state"],
        serde_json::json!("failed"),
        "the row is `failed`, and an `applied` here is the silent no-op this guard exists for -- \
         a successor byte-identical to its predecessor under a journal reporting success: {run}"
    );
    assert!(
        journal[0]["failure_reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("rate")),
        "the reason names the thing the operator has to reconsider: {run}"
    );

    let rows = price_rows(&harness, plan).await;
    assert!(
        rows.iter()
            .all(|row| row.supersedes_price_id != Some(priced.price_id)),
        "a refused row is superseded by nothing: {rows:?}"
    );
    let predecessor = rows
        .iter()
        .find(|row| row.price_id == priced.price_id)
        .expect("the seeded row is still there");
    assert_eq!(
        predecessor.lifecycle_state,
        LifecycleState::Published,
        "and it is still the published occupant of its key: {predecessor:?}"
    );
    assert_eq!(
        predecessor.row.unit_rate.map(RateMinor::nano_minor),
        Some(PER_UNIT_RATE_NANO),
        "at the rate it was published with: {predecessor:?}"
    );
}

// ---------------------------------------------------------------------------
// D-134's failure unit, over the widened rate refusal: the plan, not the row.
// ---------------------------------------------------------------------------

/// The `flat` amount both cases below seed, and the `fixed` line they submit.
///
/// `9_900 -> 50` is a move of `9_850` minor units, which is real, large, and
/// still far under the `1_000_000` bar these cases configure — so the run is
/// auto-publishable and its apply spends in the same request, which is what
/// puts the *apply's* failure unit under test rather than the approval gate.
const FLAT_AMOUNT_MINOR: i64 = 9_900;
/// The `fixed` line's literal. Not a divisor or multiple of
/// [`FLAT_AMOUNT_MINOR`] and not the projection of it under any percentage, so
/// the successor this value pins cannot also be produced by a `markup`,
/// a truncation, or a row left at its published amount.
const FIXED_LINE_MINOR: i64 = 50;

/// A `fixed $0.50` run over one named currency — the one adjustment kind
/// `project_rate` refuses outright on a rate.
fn a_fixed_run(run_id: Uuid) -> serde_json::Value {
    let mut body = a_run(run_id, &serde_json::json!({ "currency": "USD" }));
    body["adjustment"] = serde_json::json!({
        "adjustment_kind": "fixed",
        "magnitude_kind": "amount",
        "amounts": [{ "currency": "USD", "value_minor": FIXED_LINE_MINOR }],
    });
    body
}

/// **The positive control** for the mixed-plan case below, and the half that
/// makes it mean anything: on a plan holding **only** a `flat` row, this exact
/// `fixed` line applies.
///
/// Without this, "both rows failed" over the mixed plan would be satisfied by a
/// `flat` row that was never repriceable under a `fixed` adjustment in the
/// first place — the fixture-proves-nothing shape. Here the same row, the same
/// adjustment and the same bar produce `applied` and a successor at the
/// literal, so the *only* difference in the case below is the `per_unit` row
/// sitting beside it.
#[tokio::test]
async fn a_flat_row_alone_applies_the_same_fixed_line_the_mixed_plan_refuses() {
    let harness = Harness::new().await;
    approve_threshold_policy(&harness, &[("USD", 1_000_000)]).await;
    let plan = Uuid::now_v7();
    seed_current_plan_with_phase(&harness, plan).await;
    let flat = seed_priced_row(&harness, plan, "us", FLAT_AMOUNT_MINOR).await;
    harness.publish_price(plan, flat.price_id).await;
    common::schedule_coverage_window(
        &harness.db.conn().expect("conn"),
        &harness.scope(),
        harness.tenant,
        flat.price_id,
        rest_support::seed_stamp(),
    )
    .await;

    let run_id = Uuid::now_v7();
    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            REPRICING_RUNS,
            Some(a_fixed_run(run_id)),
            &[],
        ))
        .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let operation_id = body_json(response).await["operation_id"]
        .as_str()
        .expect("the view carries the minted id")
        .to_owned();

    let stored = bulk_operation_row(&harness, operation_id.parse().expect("a uuid")).await;
    assert_eq!(
        stored.state,
        BulkState::Completed,
        "a `fixed` line on a `flat` row is a well-defined reprice and nothing refuses it: {stored:?}"
    );

    let read = harness
        .allowed()
        .send(with_headers("GET", &run_path(run_id), None, &[]))
        .await;
    let run = body_json(read).await;
    let journal = run["journal"].as_array().expect("a journal");
    assert_eq!(journal.len(), 1, "one selected row: {run}");
    assert_eq!(
        journal[0]["state"],
        serde_json::json!("applied"),
        "the control is the row applying: {run}"
    );

    let rows = price_rows(&harness, plan).await;
    let successor = rows
        .iter()
        .find(|row| row.supersedes_price_id == Some(flat.price_id))
        .unwrap_or_else(|| {
            panic!("the apply writes a successor for the row it superseded: {rows:?}")
        });
    assert_eq!(
        successor
            .row
            .amount_minor
            .map(bss_pricing::domain::money::MinorAmount::get),
        Some(FIXED_LINE_MINOR),
        "a `fixed` line sets the amount to its literal, so the control's successor is a real \
         reprice and not the published amount carried forward: {successor:?}"
    );
}

/// **D-134, `inst-mr-apply`**: the transaction unit is the **plan**, not the
/// row — *"a per-row validation failure fails **every** row of that plan with
/// the shared reason — never a partial plan"*
/// (`design/12-operator-efficiency.md` §5 `inst-mr-apply`, and the Mass
/// Repricing `DoD`'s own **MUST** in §10).
///
/// So the widened D-311 rate refusal is deliberately **not** the per-row
/// refusal `inst-mp-grandfathered` owes: that clause names its own unit in as
/// many words (*"fails **that row**"*, `inst-mp-grandfathered` clause 1a) and
/// the general rule governs everything it does not name. A mixed plan — a
/// `per_unit` row the run cannot honestly reprice beside a `flat` row it could
/// — therefore fails **whole**, and the `flat` row's successor is never
/// written even though the projection for it is well-defined. D-134's own
/// reason is why: the plan-level aggregate pass runs inside the plan's commit
/// transaction over the row set *as it will stand post-commit*, and a partial
/// plan is a set that pass never evaluated.
///
/// The other half this pins is the **shared reason**. Before this case the
/// refusal's rendering opened `price <id>:` and named the offending row alone,
/// which is then stamped onto every row of the plan — so the `flat` row's
/// journal entry read as a statement about itself, asserting that a row whose
/// money is an `amount_minor` holds a rate. An operator reading that entry
/// cannot tell why a row the run could reprice did not apply, which is the
/// whole of what the journal is for.
#[tokio::test]
async fn a_per_unit_refusal_fails_its_whole_plan_with_a_reason_that_says_so() {
    let harness = Harness::new().await;
    approve_threshold_policy(&harness, &[("USD", 1_000_000)]).await;
    let plan = Uuid::now_v7();
    seed_current_plan_with_phase(&harness, plan).await;
    // One plan, two kinds, on two regions of the one currency the selector
    // names — the mixed plan none of `4817562f5`'s three single-row cases could
    // see.
    let per_unit = seed_per_unit_rate_row(&harness, plan, "eu", PER_UNIT_RATE_NANO).await;
    harness.publish_price(plan, per_unit.price_id).await;
    let flat = seed_priced_row(&harness, plan, "us", FLAT_AMOUNT_MINOR).await;
    harness.publish_price(plan, flat.price_id).await;
    for price_id in [per_unit.price_id, flat.price_id] {
        common::schedule_coverage_window(
            &harness.db.conn().expect("conn"),
            &harness.scope(),
            harness.tenant,
            price_id,
            rest_support::seed_stamp(),
        )
        .await;
    }

    let run_id = Uuid::now_v7();
    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            REPRICING_RUNS,
            Some(a_fixed_run(run_id)),
            &[],
        ))
        .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let operation_id = body_json(response).await["operation_id"]
        .as_str()
        .expect("the view carries the minted id")
        .to_owned();

    let stored = bulk_operation_row(&harness, operation_id.parse().expect("a uuid")).await;
    assert_eq!(
        stored.state,
        BulkState::CompletedWithConflicts,
        "every row of the one plan failed, so the run is a conflict rather than a success: \
         {stored:?}"
    );

    let read = harness
        .allowed()
        .send(with_headers("GET", &run_path(run_id), None, &[]))
        .await;
    assert_eq!(read.status(), StatusCode::OK);
    let run = body_json(read).await;
    let journal = run["journal"].as_array().expect("a journal");
    assert_eq!(journal.len(), 2, "both published rows were frozen: {run}");
    let entry = |price_id: Uuid| -> &serde_json::Value {
        journal
            .iter()
            .find(|row| row["price_id"] == serde_json::json!(price_id))
            .unwrap_or_else(|| panic!("the journal carries {price_id}: {run}"))
    };
    let per_unit_entry = entry(per_unit.price_id);
    let flat_entry = entry(flat.price_id);
    assert_eq!(
        (&per_unit_entry["state"], &flat_entry["state"]),
        (&serde_json::json!("failed"), &serde_json::json!("failed")),
        "D-134: the plan is the transaction unit, so the `flat` row the run could reprice fails \
         with the `per_unit` row it cannot -- never a partial plan: {run}"
    );

    let flat_reason = flat_entry["failure_reason"]
        .as_str()
        .expect("a `failed` row carries its reason");
    assert_eq!(
        Some(flat_reason),
        per_unit_entry["failure_reason"].as_str(),
        "`inst-mr-apply`'s words are `the shared reason`, one rendering of the rolled-back \
         transaction for every row of the plan: {run}"
    );
    assert!(
        flat_reason.contains(&per_unit.price_id.to_string()),
        "the reason names the row that actually refused, which is the one an operator has to \
         change or exclude -- and it is not this entry's own row: {flat_reason}"
    );
    assert!(
        !flat_reason.contains(&flat.price_id.to_string()),
        "and it does not name this row as a cause: a `flat` row's money is an amount, so a \
         reason claiming it holds a rate is false of it: {flat_reason}"
    );
    assert!(
        flat_reason.contains("D-134"),
        "and it states the failure unit, so an entry on a row the run could have repriced says \
         why it did not: {flat_reason}"
    );

    let rows = price_rows(&harness, plan).await;
    assert!(
        rows.iter().all(|row| row.supersedes_price_id.is_none()),
        "nothing was superseded: a partial plan is the state D-134's aggregate pass never \
         evaluated, and the `flat` row's successor is the one that would have made it partial: \
         {rows:?}"
    );
    for price_id in [per_unit.price_id, flat.price_id] {
        let predecessor = rows
            .iter()
            .find(|row| row.price_id == price_id)
            .expect("the seeded row is still there");
        assert_eq!(
            predecessor.lifecycle_state,
            LifecycleState::Published,
            "and both rows are still the published occupants of their keys: {predecessor:?}"
        );
    }
    assert_eq!(
        rows.iter()
            .find(|row| row.price_id == flat.price_id)
            .and_then(|row| row.row.amount_minor)
            .map(bss_pricing::domain::money::MinorAmount::get),
        Some(FLAT_AMOUNT_MINOR),
        "at the amount it was published with -- the control above proves this same row and this \
         same adjustment produce {FIXED_LINE_MINOR} when no `per_unit` row shares its plan"
    );
}
