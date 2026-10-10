//! Exact number tokens survive parsing.
#![allow(clippy::unwrap_used)]

use super::ExactJson;

#[test]
fn numbers_keep_their_exact_tokens_and_structure_is_preserved() {
    let doc = ExactJson::parse(
        br#"{"a": {"b": [1.123456789123456789123456789, -3, 1e-7, "x", true, null]}}"#,
    )
    .unwrap();
    let items = match doc.get("a").and_then(|a| a.get("b")).unwrap() {
        ExactJson::Array(items) => items.clone(),
        other => panic!("expected an array, got {other:?}"),
    };
    assert_eq!(
        items,
        vec![
            ExactJson::Number("1.123456789123456789123456789".into()),
            ExactJson::Number("-3".into()),
            ExactJson::Number("1e-7".into()),
            ExactJson::String("x".into()),
            ExactJson::Bool(true),
            ExactJson::Null,
        ]
    );
    assert!(ExactJson::parse(b"{\"a\": }").is_err());
}
