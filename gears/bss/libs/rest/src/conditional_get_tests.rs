#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::{PRIVATE_REVALIDATE, PRIVATE_SHORT, matches_if_none_match, respond, weak_etag};
use http::{HeaderMap, HeaderValue, header};

fn header(value: &str) -> HeaderValue {
    HeaderValue::from_str(value).expect("visible ASCII header")
}

fn opaque(tag: &HeaderValue) -> String {
    tag.to_str()
        .expect("ascii tag")
        .trim_start_matches("W/\"")
        .trim_end_matches('"')
        .to_owned()
}

#[test]
fn weak_etag_is_stable_and_changes_when_one_byte_changes() {
    let same = weak_etag(br#"{"n":1}"#);
    let again = weak_etag(br#"{"n":1}"#);
    let changed = weak_etag(br#"{"n":2}"#);
    assert_eq!(same, again);
    assert_ne!(same, changed);
    let text = same.to_str().expect("ascii tag");
    assert!(text.starts_with("W/\""), "{text}");
    assert!(text.ends_with('"'), "{text}");
    assert_eq!(opaque(&same).len(), 22);
}

#[test]
fn matches_if_none_match_accepts_star_a_list_and_a_strong_tag() {
    let tag = header("W/\"x\"");
    assert!(matches_if_none_match(Some(&header("*")), &tag));
    assert!(matches_if_none_match(
        Some(&header("\"no\", W/\"x\", \"later\"")),
        &tag
    ));
    assert!(matches_if_none_match(Some(&header("W/\"x\"")), &tag));
    assert!(matches_if_none_match(Some(&header("\"x\"")), &tag));
    assert!(!matches_if_none_match(None, &tag));
    assert!(!matches_if_none_match(Some(&header("\"y\"")), &tag));
    assert!(!matches_if_none_match(
        Some(&header("W/\"other\"")),
        &weak_etag(b"body")
    ));
}

#[tokio::test]
async fn respond_answers_304_with_an_empty_body_or_200_with_the_json() {
    let value = serde_json::json!({"n": 1});
    let first = respond(&HeaderMap::new(), &value, PRIVATE_REVALIDATE);
    assert_eq!(first.status(), http::StatusCode::OK);
    assert_eq!(
        first.headers().get(header::CACHE_CONTROL).expect("cache"),
        "private, no-cache"
    );
    assert_eq!(
        first.headers().get(header::CONTENT_TYPE).expect("type"),
        "application/json"
    );
    let etag = first.headers().get(header::ETAG).expect("etag").clone();
    let body = axum::body::to_bytes(first.into_body(), 64 * 1024)
        .await
        .expect("body");
    assert_eq!(body.as_ref(), serde_json::to_vec(&value).expect("json"));
    assert_eq!(etag, weak_etag(&body));

    let mut matched = HeaderMap::new();
    matched.insert(header::IF_NONE_MATCH, etag.clone());
    let not_modified = respond(&matched, &value, PRIVATE_REVALIDATE);
    assert_eq!(not_modified.status(), http::StatusCode::NOT_MODIFIED);
    assert_eq!(not_modified.headers().get(header::ETAG), Some(&etag));
    assert_eq!(
        not_modified
            .headers()
            .get(header::CACHE_CONTROL)
            .expect("cache"),
        "private, no-cache"
    );
    let empty = axum::body::to_bytes(not_modified.into_body(), 64 * 1024)
        .await
        .expect("body");
    assert!(empty.is_empty());

    let mut listed = HeaderMap::new();
    let etag_text = etag.to_str().expect("ascii tag");
    listed.insert(
        header::IF_NONE_MATCH,
        header(&format!("\"stale\", {etag_text}")),
    );
    let from_list = respond(&listed, &value, PRIVATE_SHORT);
    assert_eq!(from_list.status(), http::StatusCode::NOT_MODIFIED);
    assert_eq!(
        from_list
            .headers()
            .get(header::CACHE_CONTROL)
            .expect("cache"),
        "private, max-age=60"
    );

    let mut other = HeaderMap::new();
    other.insert(header::IF_NONE_MATCH, header("\"unrelated\""));
    let fresh = respond(&other, &value, PRIVATE_REVALIDATE);
    assert_eq!(fresh.status(), http::StatusCode::OK);
    let fresh_body = axum::body::to_bytes(fresh.into_body(), 64 * 1024)
        .await
        .expect("body");
    assert_eq!(fresh_body.as_ref(), body.as_ref());
}
