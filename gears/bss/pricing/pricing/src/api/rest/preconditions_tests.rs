//! Strong validators and idempotency parsing retained by demolition.
#![allow(clippy::expect_used, clippy::unwrap_used)]
use super::*;
use axum::http::HeaderValue;

#[test]
fn strong_row_tags_round_trip() {
    for version in [0, 1, 42, u64::MAX] {
        let row = RowVersion::new(version);
        assert_eq!(RowVersion::from_etag(&etag(row)).unwrap(), row);
    }
    assert_eq!(
        RowVersion::from_stored(42).unwrap().to_stored().unwrap(),
        42
    );
    assert!(RowVersion::from_stored(-1).is_err());
    assert!(RowVersion::new(u64::MAX).to_stored().is_err());
}

#[test]
fn if_match_is_required_and_never_accepts_an_unconditional_write() {
    let mut headers = HeaderMap::new();
    assert!(if_match(&headers).is_err());
    for bad in [
        "*",
        "W/\"1\"",
        "1",
        "\"1\", \"2\"",
        "\"-1\"",
        "\"\"",
        "\"18446744073709551616\"",
    ] {
        headers.insert(IF_MATCH, HeaderValue::from_str(bad).unwrap());
        assert!(if_match(&headers).is_err(), "{bad}");
    }
    headers.insert(IF_MATCH, HeaderValue::from_static("\"12\""));
    assert_eq!(if_match(&headers).unwrap().get(), 12);
}

#[test]
fn keys_are_required_bounded_and_printable() {
    let mut headers = HeaderMap::new();
    assert!(idempotency_key(&headers).is_err());
    for bad in [String::new(), "a".repeat(256), "a\tb".to_owned()] {
        headers.insert(IDEMPOTENCY_KEY, HeaderValue::from_str(&bad).unwrap());
        assert!(idempotency_key(&headers).is_err());
    }
    headers.insert(
        IDEMPOTENCY_KEY,
        HeaderValue::from_str(&"a".repeat(255)).unwrap(),
    );
    assert_eq!(idempotency_key(&headers).unwrap().len(), 255);
}

#[test]
fn malformed_bodies_use_the_canonical_error_ladder() {
    use axum::response::IntoResponse as _;
    for bad in [b"".as_slice(), b"{"] {
        let error = parse_body::<serde_json::Value>(bad).unwrap_err();
        let response = toolkit_canonical_errors::CanonicalError::from(error).into_response();
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }
}

#[test]
fn payload_hash_keeps_the_sha256_contract() {
    assert_eq!(
        payload_hash("abc"),
        vec![
            0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
            0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
            0xf2, 0x00, 0x15, 0xad
        ]
    );
    let a: std::collections::BTreeMap<String, u64> = parse_body(br#"{"b":2,"a":1}"#).unwrap();
    let b: std::collections::BTreeMap<String, u64> = parse_body(br#"{ "a": 1, "b": 2 }"#).unwrap();
    assert_eq!(request_digest(&a).unwrap(), request_digest(&b).unwrap());
    assert_ne!(
        request_digest(&a).unwrap(),
        request_digest(&serde_json::json!({"a":3,"b":2})).unwrap()
    );
}

/// Surface F3: where `serde_json` keeps insertion order (`preserve_order`, on in the e2e server
/// build through file-parser), a parsed `Value` keeps the client's key order. Two renderings
/// of the same members in different orders must hash alike; these two structs serialise the
/// same members in opposite orders on every feature set.
#[test]
fn the_digest_is_independent_of_key_order_at_every_depth() {
    #[derive(serde::Serialize)]
    struct Inner {
        z: u8,
        a: u8,
    }
    #[derive(serde::Serialize)]
    struct InnerSorted {
        a: u8,
        z: u8,
    }
    #[derive(serde::Serialize)]
    struct Unsorted {
        name: &'static str,
        code: &'static str,
        items: Vec<Inner>,
    }
    #[derive(serde::Serialize)]
    struct Sorted {
        code: &'static str,
        items: Vec<InnerSorted>,
        name: &'static str,
    }
    let unsorted = Unsorted {
        name: "Standard",
        code: "standard",
        items: vec![Inner { z: 2, a: 1 }],
    };
    let sorted = Sorted {
        code: "standard",
        items: vec![InnerSorted { a: 1, z: 2 }],
        name: "Standard",
    };
    assert_ne!(
        serde_json::to_string(&unsorted).unwrap(),
        serde_json::to_string(&sorted).unwrap(),
        "the renderings really differ in order"
    );
    assert_eq!(
        request_digest(&unsorted).unwrap(),
        request_digest(&sorted).unwrap()
    );
    assert_eq!(
        canonical(&serde_json::to_value(&unsorted).unwrap()),
        r#"{"code":"standard","items":[{"a":1,"z":2}],"name":"Standard"}"#
    );
    let other = Sorted {
        code: "standard",
        items: vec![InnerSorted { a: 1, z: 3 }],
        name: "Standard",
    };
    assert_ne!(
        request_digest(&unsorted).unwrap(),
        request_digest(&other).unwrap()
    );
}

/// Surface F5: `SQLite` stored a NUL and Postgres refused it with a 500; the body is refused
/// before any database work, in a value or a key, at any depth.
#[test]
fn a_nul_character_anywhere_in_the_body_is_refused() {
    use axum::response::IntoResponse as _;
    for bad in [
        br#"{"name":"a\u0000b"}"#.as_slice(),
        br#"{"items":[{"key":"region","values":["e\u0000u"]}]}"#,
        br#"{"templates":{"us\u0000":"x"}}"#,
    ] {
        let error = parse_body::<serde_json::Value>(bad).unwrap_err();
        let response = toolkit_canonical_errors::CanonicalError::from(error).into_response();
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }
    let fine: serde_json::Value = parse_body(br#"{"name":"a\\u0000b"}"#).unwrap();
    assert_eq!(
        fine["name"], "a\\u0000b",
        "an escaped backslash is not a NUL"
    );
}
