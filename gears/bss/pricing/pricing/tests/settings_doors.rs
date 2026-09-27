//! The tenant settings (D-437, D-438): `default_rounding` is one of five modes; `currencies` is
//! required on every PUT (a full replace, `[]` = any currency) and restricts the currency of a
//! NEW book; the settings say who changed them and when.
#![allow(clippy::expect_used, clippy::unwrap_used)]
mod plan_support;
use plan_support::{Fixture, holding, setup};
use serde_json::{Value, json};

async fn read(f: &Fixture) -> (Value, String) {
    let (s, b, tag) = f.call("GET", "/settings", json!({}), None, None).await;
    assert_eq!(s, 200, "{b}");
    (b, tag)
}
fn body(currencies: Value) -> Value {
    let mut b = json!({
        "default_timing": "advance",
        "default_rounding": "half_even",
        "default_gl": null,
        "default_tax_category": null,
        "invoice_line_templates": {},
    });
    b["currencies"] = currencies;
    b
}
async fn put(f: &Fixture, body: Value) -> (u16, Value, String) {
    let (_, tag) = read(f).await;
    f.call("PUT", "/settings", body, Some(&tag), None).await
}
async fn create_book(f: &Fixture, code: &str, currency: &str) -> (u16, Value) {
    let (s, b, _) = f
        .call(
            "POST",
            "/price-books",
            json!({"code":code,"name":code,"currency":currency}),
            None,
            Some(&format!("book-{code}")),
        )
        .await;
    (s, b)
}

#[tokio::test]
async fn the_default_settings_say_nobody_changed_them() {
    let (f, _) = setup().await;
    let (b, tag) = read(&f).await;
    assert_eq!(tag, "\"0\"");
    assert_eq!(
        b,
        json!({
            "default_timing": "advance",
            "default_rounding": "half_up",
            "default_gl": null,
            "default_tax_category": null,
            "invoice_line_templates": {},
            "currencies": [],
            "version": 0,
            "updated_at": null,
            "updated_by": null,
        })
    );
}

#[tokio::test]
async fn a_put_requires_currencies_and_stamps_who_and_when() {
    let (f, _) = setup().await;
    // `currencies` is required: a body without it is refused.
    let mut without = body(json!([]));
    without.as_object_mut().unwrap().remove("currencies");
    let (s, b, _) = put(&f, without).await;
    assert_eq!(s, 400, "{b}");
    assert!(b.to_string().contains("currencies"), "{b}");
    for bad in [
        json!(["eur"]),
        json!(["EURO"]),
        json!(["EU"]),
        json!([""]),
        json!(["EUR", "USD", "EUR"]),
        json!(["E1R"]),
    ] {
        let (s, b, _) = put(&f, body(bad.clone())).await;
        assert_eq!(s, 400, "{bad}: {b}");
        assert!(b.to_string().contains("CURRENCY_INVALID"), "{bad}: {b}");
    }
    assert_eq!(read(&f).await.1, "\"0\"", "no refusal wrote anything");
    let before = time::OffsetDateTime::now_utc();
    let (s, saved, tag) = put(&f, body(json!(["USD", "EUR"]))).await;
    assert_eq!(s, 200, "{saved}");
    assert_eq!(tag, "\"1\"");
    assert_eq!(saved["currencies"], json!(["USD", "EUR"]), "kept as sent");
    assert_eq!(saved["updated_by"], f.ctx.subject_id().to_string());
    let at = time::OffsetDateTime::parse(
        saved["updated_at"].as_str().unwrap(),
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap();
    assert!(at >= before - time::Duration::seconds(1), "{saved}");
    assert_eq!(read(&f).await.0, saved);
    // Another principal writes: the stamp follows the writer; `[]` means any currency again.
    let other = f.user();
    let (_, tag2) = read(&f).await;
    let (s, again, _) = f
        .call_as(
            &other,
            "PUT",
            "/settings",
            body(json!([])),
            Some(&tag2),
            None,
        )
        .await;
    assert_eq!(s, 200, "{again}");
    assert_eq!(again["currencies"], json!([]));
    assert_eq!(again["updated_by"], other.subject_id().to_string());
    assert_eq!(again["version"], 2);
    let _ = tag;
}

#[tokio::test]
async fn the_rounding_is_one_of_five_modes() {
    let (f, _) = setup().await;
    for mode in ["half_up", "half_even", "half_down", "up", "down"] {
        let mut b = body(json!([]));
        b["default_rounding"] = json!(mode);
        let (s, answer, _) = put(&f, b).await;
        assert_eq!(s, 200, "{mode}: {answer}");
        assert_eq!(answer["default_rounding"], mode);
    }
    for (mode, code) in [
        ("bankers", "ROUNDING_INVALID"),
        ("HALF_UP", "ROUNDING_INVALID"),
        ("half-up", "ROUNDING_INVALID"),
        ("ceiling", "ROUNDING_INVALID"),
        (" ", "ROUNDING_REQUIRED"),
        ("", "ROUNDING_REQUIRED"),
    ] {
        let mut b = body(json!([]));
        b["default_rounding"] = json!(mode);
        let (s, answer, _) = put(&f, b).await;
        assert_eq!(s, 400, "{mode:?}: {answer}");
        assert!(answer.to_string().contains(code), "{mode:?}: {answer}");
    }
}

// Probed in run 6.4: a book outside the offered currencies is refused.
#[tokio::test]
async fn a_new_book_takes_an_offered_currency() {
    let (f, _) = setup().await;
    // No settings, or an empty list: any currency.
    assert_eq!(create_book(&f, "jpy", "JPY").await.0, 201);
    let (s, b, _) = put(&f, body(json!(["EUR", "USD"]))).await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(create_book(&f, "eur", "EUR").await.0, 201);
    let (s, b) = create_book(&f, "gbp", "GBP").await;
    assert_eq!(s, 409, "{b}");
    assert!(b.to_string().contains("CURRENCY_NOT_OFFERED"), "{b}");
    // A malformed code is still the book's own 400, judged first.
    let (s, b) = create_book(&f, "bad", "gbp").await;
    assert_eq!(s, 400, "{b}");
    assert!(b.to_string().contains("BOOK_CURRENCY_INVALID"), "{b}");
    // The book author needs no settings grant for the rule to hold.
    let (s, b, _) = f
        .call_as(
            &holding(&f, "price_book:author"),
            "POST",
            "/price-books",
            json!({"code":"chf","name":"chf","currency":"CHF"}),
            None,
            Some("book-chf"),
        )
        .await;
    assert_eq!(s, 409, "{b}");
    // Existing books are untouched, and `[]` opens every currency again.
    let (s, books, _) = f.call("GET", "/price-books", json!({}), None, None).await;
    assert_eq!(s, 200);
    assert_eq!(books["items"].as_array().unwrap().len(), 2);
    let (s, _, _) = put(&f, body(json!([]))).await;
    assert_eq!(s, 200);
    assert_eq!(create_book(&f, "gbp2", "GBP").await.0, 201);
}
