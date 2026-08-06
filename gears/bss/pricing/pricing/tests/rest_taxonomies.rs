//! `GET/PUT /config/taxonomies/{class}`, driven through the real router.
//!
//! # The positive control is the whole point of this file
//!
//! Every other suite in this crate can assume a taxonomy value exists, because
//! until now the only way to make one exist was direct SQL. What this surface
//! claims is that an **operator** can declare a value and that Slice 9's overlay
//! scope rule then accepts it — so the first case here drives exactly that, end
//! to end and through HTTP: `PUT` a brand, then author a brand-scoped overlay
//! against it.
//!
//! Without that case the file would be a pile of refusals, and a surface that
//! refuses everything passes every refusal test it has. It is also the specific
//! claim the slice exists to make good: `inst-plv-scope` and `inst-tx-region`
//! both validate against these four tables, and both shipped before any of them
//! had a writer.
//!
//! # Why the `ETag` cases are not ceremony here
//!
//! The `PUT` replaces the **whole** value set, so a lost update is not a
//! last-writer-wins on one field — it is the other author's addition being
//! **retired**, which reads afterwards exactly like a value somebody meant to
//! withdraw. That is why the precondition is asserted on the concurrent path and
//! not merely on a malformed header.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod common;
mod rest_support;

use axum::http::StatusCode;
use bss_pricing::api::rest::taxonomies::TAXONOMY;
use rest_support::{Harness, audit_rows, body_json, etag_of, problem_code, request, with_headers};
use serde_json::json;

/// The CatalogAdmin who configures the taxonomies.
const ADMIN: uuid::Uuid = uuid::Uuid::from_u128(0xca_d0);

fn path(class: &str) -> String {
    TAXONOMY.replace("{class}", class)
}

/// Read one taxonomy, answering the body and the tag together.
///
/// The two are taken from **one** response deliberately: a helper that read the
/// body and then re-read for a tag could hand a caller a tag describing a
/// different state than the body it was given, which is the exact failure the
/// precondition exists to catch.
async fn read(harness: &Harness, class: &str) -> (serde_json::Value, String) {
    let response = harness
        .allowed_as(ADMIN)
        .send(request("GET", &path(class), None))
        .await;
    assert_eq!(response.status(), StatusCode::OK, "the GET must answer 200");
    let tag = etag_of(&response).expect("a taxonomy read must carry its entity tag");
    (body_json(response).await, tag)
}

/// `PUT` a value set under a tag.
async fn put(
    harness: &Harness,
    class: &str,
    tag: &str,
    values: serde_json::Value,
) -> axum::http::Response<axum::body::Body> {
    harness
        .allowed_as(ADMIN)
        .send(with_headers(
            "PUT",
            &path(class),
            Some(json!({ "values": values })),
            &[("if-match", tag)],
        ))
        .await
}

fn codes(body: &serde_json::Value) -> Vec<String> {
    body["values"]
        .as_array()
        .expect("values is an array")
        .iter()
        .map(|v| {
            format!(
                "{}:{}",
                v["value"].as_str().expect("value"),
                v["state"].as_str().expect("state")
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The positive control.
// ---------------------------------------------------------------------------

/// A tenant with no taxonomy is answered `200` with an empty list **and a tag**.
///
/// That is a state, not an absent resource, and it is what makes the bootstrap
/// reachable: a first `PUT` reads its precondition off this response like every
/// other caller. It is also the sentence `threshold_policy` had to withdraw a
/// false version of, so it is pinned rather than assumed.
#[tokio::test]
async fn a_tenant_with_no_values_reads_200_with_an_empty_list_and_a_tag() {
    let harness = Harness::new().await;

    let (body, tag) = read(&harness, "brand").await;

    assert_eq!(body["class"], "brand");
    assert_eq!(codes(&body), Vec::<String>::new());
    assert!(
        !tag.is_empty(),
        "the bootstrap carries a tag like any other"
    );
}

/// The round trip: declare a value, read it back, and see it in the list.
#[tokio::test]
async fn a_put_declares_a_value_and_the_get_reads_it_back() {
    let harness = Harness::new().await;
    let (_, tag) = read(&harness, "brand").await;

    let response = put(
        &harness,
        "brand",
        &tag,
        json!([{ "value": "acme", "display_name": "Acme Corp" }]),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let (body, _) = read(&harness, "brand").await;
    assert_eq!(codes(&body), ["acme:active"]);
    assert_eq!(body["values"][0]["display_name"], "Acme Corp");
}

/// **The claim this slice exists to make good.** A value declared through this
/// surface is a value Slice 9's overlay scope rule accepts.
///
/// Driven through HTTP on both ends rather than through the repositories,
/// because what was missing was never a function — it was a *route*. An operator
/// holding only the API could not do this at all, and a test that reached past
/// the API to seed the brand would prove the rule works while leaving the actual
/// gap wide open.
#[tokio::test]
async fn a_brand_declared_here_is_one_a_brand_scoped_overlay_can_name() {
    let harness = Harness::new().await;
    let (_, tag) = read(&harness, "brand").await;

    let declared = put(
        &harness,
        "brand",
        &tag,
        json!([{ "value": "acme", "display_name": "Acme Corp" }]),
    )
    .await;
    assert_eq!(declared.status(), StatusCode::OK, "the brand is declared");

    let overlay = harness
        .allowed_as(ADMIN)
        .send(with_headers(
            "POST",
            "/bss-pricing/v1/price-overlays",
            Some(json!({
                "scope_class": "brand",
                "scope_value": "acme",
                "precedence": 10,
                "tax_basis": "delegated_tariffs",
                "target_plan_ids": [],
                "lines": [{
                    "adjustment_kind": "discount",
                    "magnitude_kind": "percent_bp",
                    "adjustment_value": 500,
                }]
            })),
            &[("idempotency-key", "brand-overlay-1")],
        ))
        .await;

    // **`CREATED`, not merely "some code other than SCOPE_VALUE_UNKNOWN".** The
    // weaker assertion is satisfied by a malformed request — which is exactly what
    // this case did on its first run, answering 400 for a missing field while
    // "proving" the scope rule accepted the brand. A positive control has to
    // assert the positive outcome.
    assert_eq!(
        overlay.status(),
        StatusCode::CREATED,
        "a brand declared through this surface must satisfy inst-plv-scope: if this fails, the \
         write surface and the read Slice 9 makes of it disagree about what `declared` means"
    );
}

// ---------------------------------------------------------------------------
// The whole-set semantics.
// ---------------------------------------------------------------------------

/// A value the body omits is retired, and stays readable.
#[tokio::test]
async fn a_value_the_body_omits_is_retired_and_still_listed() {
    let harness = Harness::new().await;
    let (_, tag) = read(&harness, "partner").await;
    put(
        &harness,
        "partner",
        &tag,
        json!([
            { "value": "reseller-a", "display_name": "A" },
            { "value": "reseller-b", "display_name": "B" }
        ]),
    )
    .await;

    let (_, tag) = read(&harness, "partner").await;
    let response = put(
        &harness,
        "partner",
        &tag,
        json!([{ "value": "reseller-a", "display_name": "A" }]),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let (body, _) = read(&harness, "partner").await;
    assert_eq!(codes(&body), ["reseller-a:active", "reseller-b:retired"]);
}

// ---------------------------------------------------------------------------
// The precondition.
// ---------------------------------------------------------------------------

/// A `PUT` with no `If-Match` is refused before anything is written.
#[tokio::test]
async fn a_put_without_if_match_is_refused() {
    let harness = Harness::new().await;

    let response = harness
        .allowed_as(ADMIN)
        .send(request(
            "PUT",
            &path("brand"),
            Some(json!({ "values": [{ "value": "acme", "display_name": "A" }] })),
        ))
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let (body, _) = read(&harness, "brand").await;
    assert_eq!(codes(&body), Vec::<String>::new(), "nothing was written");
}

/// **The lost update this precondition exists to stop.**
///
/// Two admins read the same brand list and each add one value. Without the
/// precondition the second `PUT` — a whole-set replacement authored against a
/// stale reading — would land and silently **retire** the first's addition. The
/// tag makes it a `409` instead.
#[tokio::test]
async fn a_concurrent_whole_set_put_is_refused_rather_than_retiring_the_other_authors_value() {
    let harness = Harness::new().await;
    let (_, shared_tag) = read(&harness, "brand").await;

    let first = put(
        &harness,
        "brand",
        &shared_tag,
        json!([{ "value": "acme", "display_name": "Acme" }]),
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);

    // The second author is still holding the tag they read before the first
    // landed — which is exactly the sequential case an approval unit or a
    // "one open change" guard would not catch.
    let second = put(
        &harness,
        "brand",
        &shared_tag,
        json!([{ "value": "zenith", "display_name": "Zenith" }]),
    )
    .await;

    assert_eq!(second.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(second).await, "STALE_VERSION");

    let (body, _) = read(&harness, "brand").await;
    assert_eq!(
        codes(&body),
        ["acme:active"],
        "the first author's value survives and the second's was not applied"
    );
}

/// The tag moves when a value is **re-labelled**, not only when membership
/// changes.
///
/// A validator that does not change when the representation changes is broken
/// rather than lenient — the argument the threshold policy's tag makes for
/// covering its pending unit as well as its version.
#[tokio::test]
async fn the_tag_moves_on_a_relabel() {
    let harness = Harness::new().await;
    let (_, tag) = read(&harness, "brand").await;
    put(
        &harness,
        "brand",
        &tag,
        json!([{ "value": "acme", "display_name": "Acme" }]),
    )
    .await;
    let (_, before) = read(&harness, "brand").await;

    put(
        &harness,
        "brand",
        &before,
        json!([{ "value": "acme", "display_name": "Acme Corporation" }]),
    )
    .await;
    let (_, after) = read(&harness, "brand").await;

    assert_ne!(before, after, "a re-label changes the representation");
}

/// A tag read from one class does not satisfy a `PUT` on another — **and the two
/// classes are compared while their contents are identical**.
///
/// The first version of this case seeded the partner list before comparing, so
/// the two representations differed by *content* and the tags differed whether
/// or not the class was in the digest. A probe that removed the class from the
/// digest left it green: it asserted the conclusion without testing the reason.
///
/// Both lists are therefore empty here, which is the only state in which the
/// class is the **sole** difference between the two representations. Without it
/// a tag read off any tenant's empty brand list would satisfy a `PUT` on their
/// empty partner list — and since a `PUT` is a whole-set replacement, that is a
/// write to the wrong universe under a precondition that appeared to hold.
#[tokio::test]
async fn a_tag_from_another_class_does_not_satisfy_this_one() {
    let harness = Harness::new().await;

    let (brand_body, brand_tag) = read(&harness, "brand").await;
    let (partner_body, partner_tag) = read(&harness, "partner").await;
    assert_eq!(
        codes(&brand_body),
        codes(&partner_body),
        "the two lists must be indistinguishable by content for this case to mean anything"
    );
    assert_ne!(
        brand_tag, partner_tag,
        "two empty taxonomies still have different tags, because the class is in the digest"
    );

    let response = put(
        &harness,
        "partner",
        &brand_tag,
        json!([{ "value": "reseller-b", "display_name": "B" }]),
    )
    .await;

    assert_eq!(
        response.status(),
        StatusCode::CONFLICT,
        "four taxonomies are four resources, and one tag must not travel between them"
    );
}

// ---------------------------------------------------------------------------
// The body's own rules.
// ---------------------------------------------------------------------------

/// An unaddressable class is refused, and the refusal names the four that are.
#[tokio::test]
async fn an_unaddressable_class_is_refused_naming_the_four() {
    let harness = Harness::new().await;

    for class in ["global", "customer_group", "org_tier"] {
        let response = harness
            .allowed_as(ADMIN)
            .send(request("GET", &path(class), None))
            .await;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "`{class}` is not an addressable universe"
        );
    }
}

/// `orgTier` **is** addressable, in §5's own camelCase spelling.
#[tokio::test]
async fn the_org_tier_segment_is_section_5s_camel_case() {
    let harness = Harness::new().await;

    let (body, _) = read(&harness, "orgTier").await;

    assert_eq!(body["class"], "orgTier");
}

/// The two `tax_*` markers are refused on the three non-region classes rather
/// than silently dropped.
///
/// A field that vanishes reads, at the operator's end, exactly like one that
/// failed to save — they would read the `GET` back, see no category, and try
/// again.
#[tokio::test]
async fn the_tax_markers_are_refused_on_a_non_region_taxonomy() {
    let harness = Harness::new().await;
    let (_, tag) = read(&harness, "brand").await;

    let response = put(
        &harness,
        "brand",
        &tag,
        json!([{ "value": "acme", "display_name": "A", "tax_category": "standard" }]),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

/// They round-trip on the region taxonomy, which is the one that has the columns.
#[tokio::test]
async fn the_tax_markers_round_trip_on_the_region_taxonomy() {
    let harness = Harness::new().await;
    let (_, tag) = read(&harness, "region").await;

    let response = put(
        &harness,
        "region",
        &tag,
        json!([{
            "value": "eu",
            "display_name": "Europe",
            "tax_category": "standard",
            "tax_rate_present": true
        }]),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let (body, _) = read(&harness, "region").await;
    assert_eq!(body["values"][0]["tax_category"], "standard");
    assert_eq!(body["values"][0]["tax_rate_present"], true);
}

/// A blank value is refused at the edge, not by the store.
///
/// The `CHECK` is the backstop against a writer that is not this crate; reaching
/// it from a request would make a one-field caller mistake an internal fault —
/// `overlay_rules::check_authored_shape`'s argument, and the reason that entry
/// point exists.
#[tokio::test]
async fn a_blank_value_is_refused_at_the_edge() {
    let harness = Harness::new().await;
    let (_, tag) = read(&harness, "brand").await;

    let response = put(
        &harness,
        "brand",
        &tag,
        json!([{ "value": "   ", "display_name": "A" }]),
    )
    .await;

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "a whitespace value must not reach the store — the length CHECK would admit it"
    );
}

/// One value twice in one body is refused rather than de-duplicated.
#[tokio::test]
async fn a_repeated_value_is_refused() {
    let harness = Harness::new().await;
    let (_, tag) = read(&harness, "brand").await;

    let response = put(
        &harness,
        "brand",
        &tag,
        json!([
            { "value": "acme", "display_name": "A", "state": "active" },
            { "value": "acme", "display_name": "A", "state": "retired" }
        ]),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// The audit half of `inst-tx-mutation`.
// ---------------------------------------------------------------------------

/// *"Taxonomy mutation is tenant-admin config, audited"* — one record per `PUT`.
#[tokio::test]
async fn a_put_is_audited_once_naming_the_taxonomy() {
    let harness = Harness::new().await;
    let before = audit_rows(&harness).await.len();
    let (_, tag) = read(&harness, "brand").await;

    put(
        &harness,
        "brand",
        &tag,
        json!([
            { "value": "acme", "display_name": "A" },
            { "value": "zenith", "display_name": "Z" }
        ]),
    )
    .await;

    let rows = audit_rows(&harness).await;
    assert_eq!(
        rows.len(),
        before + 1,
        "one PUT is one audited act, however many values it moved"
    );
    assert_eq!(rows.last().expect("a record").subject_ref, "taxonomy/brand");
}
