//! The SKU list on `OData` and its counts (P-D-210, P-D-211), through the door.
#![allow(clippy::expect_used, clippy::unwrap_used)]
use crate::api::rest::{ApiState, categories, skus};
use crate::domain::{category::NewCategory, sku::NewSku};
use crate::infra::storage::repo;
use crate::test_support::{body_json, get, problem_code, resolved_usage_types, rest_app_on_db};
use axum::{Router, http::StatusCode};
use bss_products_sdk::models::{Lifecycle, SkuType};
use serde_json::Value;
use std::sync::Arc;
use time::OffsetDateTime;
use toolkit::api::OpenApiRegistry;
use toolkit_db::secure::AccessScope;
use uuid::Uuid;

fn doors(s: Arc<ApiState>, o: &dyn OpenApiRegistry) -> Router {
    categories::router(Arc::clone(&s), o).merge(skus::router(s, o))
}

/// Percent-encode a query component (everything but the unreserved characters).
fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
fn uri(path: &str, params: &[(&str, &str)]) -> String {
    let query: Vec<String> = params
        .iter()
        .map(|(k, v)| format!("{}={}", enc(k), enc(v)))
        .collect();
    format!("/bss-products/v1/{path}?{}", query.join("&"))
}
fn list(params: &[(&str, &str)]) -> String {
    uri("skus", params)
}
fn counts(params: &[(&str, &str)]) -> String {
    uri("skus/counts", params)
}

/// A door over its own database, with the database's tenant and scope to seed through.
struct Door {
    app: Router,
    state: Arc<ApiState>,
    tenant: Uuid,
    scope: AccessScope,
}
impl Door {
    async fn new() -> Self {
        let (db, scope, tenant, _) = crate::test_support::test_db().await;
        let (app, state) = rest_app_on_db(tenant, doors, resolved_usage_types(), "test", db).await;
        Self {
            app,
            state,
            tenant,
            scope,
        }
    }
    async fn category(&self, code: &str) -> Uuid {
        repo::insert_category(
            &self.state.db.conn().unwrap(),
            &self.scope,
            self.tenant,
            NewCategory {
                code: code.into(),
                name: code.into(),
                is_default: false,
                sort_order: 0,
            },
            OffsetDateTime::now_utc(),
        )
        .await
        .unwrap()
        .id
    }
    /// A SKU in `lifecycle`, written at `at` (its `updated_at`).
    async fn sku(&self, s: Seed) -> Uuid {
        let conn = self.state.db.conn().unwrap();
        let row = repo::insert_sku(
            &conn,
            &self.scope,
            self.tenant,
            NewSku {
                code: s.code.into(),
                name: s.name.into(),
                r#type: s.ty,
                category_id: s.category,
                description: String::new(),
                sellable: true,
                gl_code: s.gl_code.map(Into::into),
                tax_category: None,
                invoice_line_template: None,
                billing_timing: None,
                usage_type_ref: s.usage_type_ref.map(Into::into),
                unit: s.unit.map(Into::into),
            },
            self.tenant,
            s.at,
        )
        .await
        .unwrap();
        if s.lifecycle != Lifecycle::Draft {
            repo::set_lifecycle(
                &conn,
                &self.scope,
                self.tenant,
                row.id,
                &[Lifecycle::Draft],
                s.lifecycle,
                s.at,
            )
            .await
            .unwrap();
        }
        if s.locked {
            let revision = repo::find_sku(&conn, &self.scope, self.tenant, row.id)
                .await
                .unwrap()
                .unwrap()
                .revision;
            assert!(
                repo::try_lock_sku(
                    &conn,
                    &self.scope,
                    self.tenant,
                    row.id,
                    Uuid::new_v4(),
                    revision
                )
                .await
                .unwrap()
            );
        }
        row.id
    }
    async fn get(&self, uri: &str) -> (StatusCode, Value) {
        let r = get(&self.app, self.tenant, uri).await;
        let status = r.status();
        (status, body_json(r).await)
    }
    /// The codes of a list page that must answer 200.
    async fn codes(&self, uri: &str) -> Vec<String> {
        let (status, body) = self.get(uri).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
        codes(&body)
    }
    async fn refused(&self, uri: &str) -> Value {
        let (status, body) = self.get(uri).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}: {body}");
        body
    }
}
fn codes(page: &Value) -> Vec<String> {
    page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["code"].as_str().unwrap().to_owned())
        .collect()
}
#[derive(Clone, Copy)]
struct Seed {
    code: &'static str,
    name: &'static str,
    ty: SkuType,
    category: Option<Uuid>,
    lifecycle: Lifecycle,
    locked: bool,
    unit: Option<&'static str>,
    usage_type_ref: Option<&'static str>,
    gl_code: Option<&'static str>,
    at: OffsetDateTime,
}
fn seed(code: &'static str) -> Seed {
    Seed {
        code,
        name: code,
        ty: SkuType::Recurring,
        category: None,
        lifecycle: Lifecycle::Draft,
        locked: false,
        unit: None,
        usage_type_ref: None,
        gl_code: None,
        at: OffsetDateTime::now_utc(),
    }
}
fn sorted(mut v: Vec<&str>) -> Vec<String> {
    v.sort_unstable();
    v.into_iter().map(str::to_owned).collect()
}

/// Every filter field narrows the list: the two closed ones by value, the two nullable ones by
/// `eq null` / `ne null` too, and the three text and id fields.
#[tokio::test]
async fn every_filter_field_narrows_the_list() {
    let d = Door::new().await;
    let x = d.category("x").await;
    let y = d.category("y").await;
    let a = d
        .sku(Seed {
            name: "Alpha",
            category: Some(x),
            lifecycle: Lifecycle::Published,
            ..seed("A")
        })
        .await;
    d.sku(Seed {
        name: "Beta",
        ty: SkuType::Usage,
        category: Some(x),
        locked: true,
        ..seed("B")
    })
    .await;
    d.sku(Seed {
        name: "Gamma",
        ty: SkuType::OneTime,
        lifecycle: Lifecycle::Deprecated,
        ..seed("C")
    })
    .await;
    let dd = d
        .sku(Seed {
            name: "Delta",
            ty: SkuType::Bundle,
            category: Some(y),
            lifecycle: Lifecycle::Retired,
            ..seed("D")
        })
        .await;
    d.sku(Seed {
        name: "Epsilon",
        ty: SkuType::Usage,
        ..seed("E")
    })
    .await;
    let (x, a, dd) = (x.to_string(), a.to_string(), dd.to_string());
    for (filter, expected) in [
        ("lifecycle eq 'draft'".to_owned(), vec!["B", "E"]),
        ("lifecycle ne 'draft'".to_owned(), vec!["A", "C", "D"]),
        (
            "lifecycle in ('published', 'deprecated')".to_owned(),
            vec!["A", "C"],
        ),
        ("type eq 'usage'".to_owned(), vec!["B", "E"]),
        ("type in ('bundle', 'one_time')".to_owned(), vec!["C", "D"]),
        (format!("category_id eq {x}"), vec!["A", "B"]),
        ("category_id eq null".to_owned(), vec!["C", "E"]),
        ("category_id ne null".to_owned(), vec!["A", "B", "D"]),
        ("pending_unit_id ne null".to_owned(), vec!["B"]),
        (
            "pending_unit_id eq null".to_owned(),
            vec!["A", "C", "D", "E"],
        ),
        ("code eq 'C'".to_owned(), vec!["C"]),
        ("code in ('A', 'E')".to_owned(), vec!["A", "E"]),
        ("contains(name, 'lph')".to_owned(), vec!["A"]),
        ("startswith(name, 'Ep')".to_owned(), vec!["E"]),
        (format!("id eq {a}"), vec!["A"]),
        (format!("id in ({a}, {dd})"), vec!["A", "D"]),
        (
            "category_id eq null and lifecycle eq 'draft'".to_owned(),
            vec!["E"],
        ),
        (
            "pending_unit_id ne null or type eq 'bundle'".to_owned(),
            vec!["B", "D"],
        ),
        ("not (lifecycle eq 'draft')".to_owned(), vec!["A", "C", "D"]),
    ] {
        assert_eq!(
            d.codes(&list(&[("$filter", &filter)])).await,
            sorted(expected),
            "{filter}"
        );
    }
    for filter in [
        "type eq 'bad'",
        "lifecycle eq 'bad'",
        "lifecycle in ('draft', 'bad')",
        "contains(lifecycle, 'pub')",
        "startswith(type, 'us')",
        "code eq null",
        "lifecycle ne null",
        "category_id in (null)",
        "updated_at gt 2026-01-01T00:00:00Z",
        "sellable eq true",
        "category_id gt null",
    ] {
        let body = d.refused(&list(&[("$filter", filter)])).await;
        assert_eq!(problem_code(&body), "INVALID_FILTER", "{filter}: {body}");
    }
}

/// Each published order walks the whole list page by page with its cursor, every SKU once, in
/// the order one page shows — ties on `updated_at` broken by `id` — and back again.
#[tokio::test]
async fn every_order_pages_with_a_stable_cursor_across_ties() {
    let d = Door::new().await;
    let t1 = OffsetDateTime::now_utc() - time::Duration::hours(2);
    let t2 = t1 + time::Duration::minutes(5);
    let mut rows: Vec<(String, String, OffsetDateTime, Uuid)> = Vec::new();
    for (code, name, at) in [
        ("K1", "zeta", t1),
        ("K7", "alpha", t2),
        ("K3", "mu", t1),
        ("K5", "beta", t1),
        ("K2", "omega", t2),
        ("K6", "kappa", t1),
        ("K4", "delta", t2),
    ] {
        let id = d
            .sku(Seed {
                name,
                at,
                ..seed(code)
            })
            .await;
        rows.push((code.to_owned(), name.to_owned(), at, id));
    }
    let expected = |key: &str, desc: bool| -> Vec<String> {
        let mut r = rows.clone();
        r.sort_by(|a, b| {
            let primary = match key {
                "code" => a.0.cmp(&b.0),
                "name" => a.1.cmp(&b.1),
                _ => a.2.cmp(&b.2),
            };
            let primary = if desc { primary.reverse() } else { primary };
            primary.then(a.3.cmp(&b.3))
        });
        r.into_iter().map(|r| r.0).collect()
    };
    for (orderby, key, desc) in [
        ("code", "code", false),
        ("code desc", "code", true),
        ("name asc", "name", false),
        ("name desc", "name", true),
        ("updated_at", "updated_at", false),
        ("updated_at desc", "updated_at", true),
        ("updated_at desc, code", "updated_at", true),
    ] {
        let one_page = d.codes(&list(&[("$orderby", orderby)])).await;
        if orderby != "updated_at desc, code" {
            assert_eq!(one_page, expected(key, desc), "{orderby}");
        }
        let (status, mut page) = d.get(&list(&[("$orderby", orderby), ("limit", "2")])).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        let mut walked = codes(&page);
        let mut pages = vec![page.clone()];
        while let Some(next) = page["page_info"]["next_cursor"].as_str().map(str::to_owned) {
            let (status, body) = d.get(&list(&[("cursor", &next), ("limit", "2")])).await;
            assert_eq!(status, StatusCode::OK, "{orderby}: {body}");
            walked.extend(codes(&body));
            pages.push(body.clone());
            page = body;
        }
        assert_eq!(
            walked, one_page,
            "{orderby}: the walk is the one page, once each"
        );
        assert_eq!(pages.len(), 4, "{orderby}");
        // Back from the last page: its previous page is the one before it.
        let prev = pages[3]["page_info"]["prev_cursor"].as_str().unwrap();
        let back = d.codes(&list(&[("cursor", prev), ("limit", "2")])).await;
        assert_eq!(back, codes(&pages[2]), "{orderby}: back one page");
    }
    // Every order ends with the id: the default order is the code.
    assert_eq!(d.codes(&list(&[])).await, expected("code", false));
}

/// What the list refuses: projections and totals, an order key that is not published, the old
/// parameters and any other key, and a cursor replayed with another narrowing.
#[tokio::test]
async fn the_list_refuses_what_it_does_not_publish() {
    let d = Door::new().await;
    for code in ["A", "B", "C"] {
        d.sku(seed(code)).await;
    }
    for (params, field) in [
        (vec![("$select", "code")], "$select"),
        (vec![("$count", "true")], "$count"),
        (vec![("$skip", "1")], "$skip"),
        (vec![("category", "x")], "category"),
        (vec![("after", "A")], "after"),
        (vec![("type", "usage")], "type"),
        (vec![("lifecycle", "draft")], "lifecycle"),
        (vec![("bogus", "1"), ("q", "a")], "bogus"),
        (vec![("q", "a"), ("q", "b")], "q"),
    ] {
        let body = d.refused(&list(&params)).await;
        assert!(body.to_string().contains(field), "{params:?}: {body}");
    }
    for orderby in [
        "lifecycle",
        "type",
        "category_id",
        "pending_unit_id desc",
        "sellable",
    ] {
        let body = d.refused(&list(&[("$orderby", orderby)])).await;
        assert_eq!(
            problem_code(&body),
            "INVALID_ORDERBY_FIELD",
            "{orderby}: {body}"
        );
    }
    let body = d.refused(&list(&[("limit", "0")])).await;
    assert_eq!(problem_code(&body), "INVALID_LIMIT", "{body}");
    // A cursor answers the narrowing it was issued for, and only that one.
    let (status, page) = d
        .get(&list(&[
            ("q", "a"),
            ("$filter", "code ne 'Z'"),
            ("limit", "1"),
        ]))
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let (status, page) = d.get(&list(&[("limit", "1")])).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let cursor = page["page_info"]["next_cursor"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        d.codes(&list(&[("cursor", &cursor), ("limit", "1")])).await,
        ["B"]
    );
    for params in [
        vec![("cursor", cursor.as_str()), ("q", "b")],
        vec![("cursor", cursor.as_str()), ("$filter", "code ne 'A'")],
    ] {
        let body = d.refused(&list(&params)).await;
        assert_eq!(problem_code(&body), "FILTER_MISMATCH", "{params:?}: {body}");
    }
    let body = d
        .refused(&list(&[("cursor", &cursor), ("$orderby", "name")]))
        .await;
    assert!(body.to_string().contains("cursor"), "{body}");
}

/// `q` is a case-insensitive substring of five columns, taken literally; `SQLite` folds ASCII
/// only (Postgres folds Unicode: `tests/postgres_sku_list.rs`).
#[tokio::test]
async fn q_is_a_case_insensitive_literal_substring_of_five_columns() {
    let d = Door::new().await;
    d.sku(Seed {
        name: "Storage",
        unit: Some("GiB"),
        usage_type_ref: Some("gts.cf.usage.disk~"),
        gl_code: Some("GL-4000"),
        ..seed("STOR-1")
    })
    .await;
    d.sku(Seed {
        name: "Compute 50%_off",
        ..seed("COMP")
    })
    .await;
    d.sku(Seed {
        name: r"Back\slash",
        ..seed("BACK")
    })
    .await;
    d.sku(Seed {
        name: "Compute 50xyoff",
        ..seed("DECOY")
    })
    .await;
    d.sku(Seed {
        name: "\u{411}\u{435}\u{442}\u{430}", // Beta, in Cyrillic
        ..seed("CYR")
    })
    .await;
    for (q, expected) in [
        ("stor", vec!["STOR-1"]),
        ("STORAGE", vec!["STOR-1"]),
        ("gib", vec!["STOR-1"]),
        ("USAGE.DISK", vec!["STOR-1"]),
        ("gl-4000", vec!["STOR-1"]),
        ("%", vec!["COMP"]),
        ("_", vec!["COMP"]),
        ("50%_", vec!["COMP"]),
        (r"\", vec!["BACK"]),
        ("compute 50", vec!["COMP", "DECOY"]),
        ("\u{411}\u{435}\u{442}\u{430}", vec!["CYR"]),
        ("nothing", vec![]),
    ] {
        assert_eq!(d.codes(&list(&[("q", q)])).await, sorted(expected), "q={q}");
    }
    // ASCII-only folding on SQLite, pinned: another case of a Cyrillic letter does not match.
    assert!(
        d.codes(&list(&[("q", "\u{431}\u{435}\u{442}\u{430}")]))
            .await
            .is_empty()
    );
    // An empty `q` is no search.
    assert_eq!(d.codes(&list(&[("q", "")])).await.len(), 5);
    // `q` and `$filter` intersect.
    assert_eq!(
        d.codes(&list(&[("q", "compute"), ("$filter", "code eq 'DECOY'")]))
            .await,
        ["DECOY"]
    );
}

/// `$top` (alias `limit`) defaults to 50 and is clamped at 200.
#[tokio::test]
async fn the_page_defaults_to_50_and_is_clamped_at_200() {
    let d = Door::new().await;
    for i in 0..205 {
        d.sku(seed(Box::leak(format!("S{i:03}").into_boxed_str())))
            .await;
    }
    for (params, size) in [
        (vec![], 50),
        (vec![("limit", "7")], 7),
        (vec![("$top", "7")], 7),
        (vec![("limit", "500")], 200),
        (vec![("$top", "201")], 200),
        (vec![("limit", "200")], 200),
    ] {
        let (status, page) = d.get(&list(&params)).await;
        assert_eq!(status, StatusCode::OK, "{params:?}: {page}");
        assert_eq!(page["items"].as_array().unwrap().len(), size, "{params:?}");
        assert_eq!(page["page_info"]["limit"], size, "{params:?}");
        assert!(page["page_info"]["next_cursor"].is_string(), "{params:?}");
    }
}

/// The counts: every SKU, each lifecycle and those in review, narrowed like the list; a
/// top-level `lifecycle` term is dropped, one under `or` or `not` refused; nothing that pages.
#[tokio::test]
async fn the_counts_follow_the_list_without_its_lifecycle_terms() {
    let d = Door::new().await;
    let x = d.category("x").await;
    for s in [
        Seed {
            category: Some(x),
            locked: true,
            ..seed("D1")
        },
        seed("D2"),
        Seed {
            category: Some(x),
            lifecycle: Lifecycle::Published,
            ..seed("P1")
        },
        Seed {
            name: "storage",
            lifecycle: Lifecycle::Published,
            locked: true,
            ..seed("P2")
        },
        Seed {
            lifecycle: Lifecycle::Deprecated,
            ..seed("X1")
        },
        Seed {
            category: Some(x),
            lifecycle: Lifecycle::Retiring,
            ..seed("R1")
        },
        Seed {
            name: "old storage",
            lifecycle: Lifecycle::Retired,
            ..seed("Z1")
        },
    ] {
        d.sku(s).await;
    }
    let count_of = |body: &Value| -> Vec<u64> {
        [
            "all",
            "draft",
            "published",
            "deprecated",
            "retiring",
            "retired",
            "in_review",
        ]
        .iter()
        .map(|k| body[k].as_u64().unwrap_or_else(|| panic!("{k}: {body}")))
        .collect()
    };
    let x = x.to_string();
    for (params, expected) in [
        (vec![], vec![7, 2, 2, 1, 1, 1, 2]),
        (
            vec![("$filter", "lifecycle eq 'draft'".to_owned())],
            vec![7, 2, 2, 1, 1, 1, 2],
        ),
        (
            vec![(
                "$filter",
                format!("lifecycle in ('draft', 'published') and category_id eq {x}"),
            )],
            vec![3, 1, 1, 0, 1, 0, 1],
        ),
        (
            vec![(
                "$filter",
                format!("category_id eq {x} and lifecycle eq 'retired'"),
            )],
            vec![3, 1, 1, 0, 1, 0, 1],
        ),
        (
            vec![("$filter", "category_id eq null".to_owned())],
            vec![4, 1, 1, 1, 0, 1, 1],
        ),
        (vec![("q", "storage".to_owned())], vec![2, 0, 1, 0, 0, 1, 1]),
        (
            vec![
                ("q", "storage".to_owned()),
                ("$filter", "pending_unit_id ne null".to_owned()),
            ],
            vec![1, 0, 1, 0, 0, 0, 1],
        ),
    ] {
        let params: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let (status, body) = d.get(&counts(&params)).await;
        assert_eq!(status, StatusCode::OK, "{params:?}: {body}");
        assert_eq!(count_of(&body), expected, "{params:?}: {body}");
    }
    // Each lifecycle count is the length of the list narrowed to it.
    let (_, all) = d.get(&counts(&[("q", "storage")])).await;
    for lifecycle in ["draft", "published", "deprecated", "retiring", "retired"] {
        let listed = d
            .codes(&list(&[
                ("q", "storage"),
                ("$filter", &format!("lifecycle eq '{lifecycle}'")),
            ]))
            .await
            .len();
        assert_eq!(
            all[lifecycle].as_u64().unwrap(),
            listed as u64,
            "{lifecycle}"
        );
    }
    for filter in [
        "lifecycle eq 'draft' or category_id eq null",
        "not (lifecycle eq 'draft')",
        "category_id eq null and (lifecycle eq 'draft' or code eq 'A')",
        "lifecycle eq 'bad'",
        "updated_at gt 2026-01-01T00:00:00Z",
    ] {
        let body = d.refused(&counts(&[("$filter", filter)])).await;
        assert_eq!(problem_code(&body), "INVALID_FILTER", "{filter}: {body}");
    }
    for (key, value) in [
        ("$orderby", "code"),
        ("$top", "5"),
        ("limit", "5"),
        ("cursor", "abc"),
        ("$skiptoken", "abc"),
        ("$select", "code"),
        ("bogus", "1"),
    ] {
        let body = d.refused(&counts(&[(key, value)])).await;
        assert_eq!(
            problem_code(&body),
            "UNSUPPORTED_QUERY_PARAM",
            "{key}: {body}"
        );
    }
}

/// The counts recover orphan fences in their transaction as the list does, so `retiring` agrees
/// with the list: an expired fence counts as the lifecycle it returns to, a live one as retiring.
#[tokio::test]
async fn the_counts_and_the_list_expire_orphan_fences_alike() {
    let d = Door::new().await;
    let mut fenced = Vec::new();
    for (code, age) in [
        ("OLD", time::Duration::hours(2)),
        ("NEW", time::Duration::ZERO),
    ] {
        let id = d
            .sku(Seed {
                lifecycle: Lifecycle::Published,
                ..seed(code)
            })
            .await;
        repo::fence_sku(
            &d.state.db.conn().unwrap(),
            &d.scope,
            d.tenant,
            id,
            repo::Fence::Retire,
            Uuid::new_v4(),
            OffsetDateTime::now_utc() - age,
        )
        .await
        .unwrap();
        fenced.push(id);
    }
    let (status, body) = d.get(&counts(&[])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        (body["retiring"].as_u64(), body["published"].as_u64()),
        (Some(1), Some(1)),
        "{body}"
    );
    assert_eq!(
        d.codes(&list(&[("$filter", "lifecycle eq 'retiring'")]))
            .await,
        ["NEW"]
    );
    assert_eq!(
        d.codes(&list(&[("$filter", "lifecycle eq 'published'")]))
            .await,
        ["OLD"]
    );
}

// ------------------------------------------------------------------ fixed statements

/// A door over a database whose statements are recorded.
async fn recorded_door(n: usize) -> (Door, toolkit_db::test_support::QueryRecorder) {
    use sea_orm_migration::MigratorTrait;
    let dsn = format!(
        "sqlite://{}?mode=rwc",
        std::env::temp_dir()
            .join(format!("products-recorded-{}.sqlite3", Uuid::new_v4()))
            .display()
    );
    let (db, recorder) = toolkit_db::test_support::connect_with_recorder(
        &dsn,
        toolkit_db::ConnectOpts {
            max_conns: Some(1),
            min_conns: Some(1),
            ..toolkit_db::ConnectOpts::default()
        },
    )
    .await
    .unwrap();
    toolkit_db::migration_runner::run_migrations_for_testing(
        &db,
        crate::infra::storage::migrations::Migrator::migrations(),
    )
    .await
    .unwrap();
    toolkit_db::migration_runner::run_migrations_for_testing(
        &db,
        toolkit_db::outbox::outbox_migrations_with_prefix(
            crate::infra::events::OUTBOX_TABLE_PREFIX,
        )
        .unwrap(),
    )
    .await
    .unwrap();
    let tenant = Uuid::new_v4();
    let scope = AccessScope::for_tenant(tenant);
    let (app, state) = rest_app_on_db(
        tenant,
        doors,
        resolved_usage_types(),
        "test",
        toolkit_db::DBProvider::new(db),
    )
    .await;
    let d = Door {
        app,
        state,
        tenant,
        scope,
    };
    for i in 0..n {
        d.sku(Seed {
            lifecycle: if i % 2 == 0 {
                Lifecycle::Published
            } else {
                Lifecycle::Draft
            },
            ..seed(Box::leak(format!("R{i:03}").into_boxed_str()))
        })
        .await;
    }
    recorder.clear();
    (d, recorder)
}
/// The statements on the gear's tables one read makes, with their bind counts.
async fn statements(
    d: &Door,
    recorder: &toolkit_db::test_support::QueryRecorder,
    uri: &str,
) -> Vec<(String, usize)> {
    recorder.clear();
    let (status, body) = d.get(uri).await;
    assert_eq!(status, StatusCode::OK, "{uri}: {body}");
    recorder
        .events()
        .into_iter()
        .filter(|q| {
            q.table
                .as_deref()
                .is_some_and(|t| t.starts_with("products_"))
        })
        .map(|q| (q.sql, q.param_count))
        .collect()
}

/// The list and the counts make the same statements, with the same binds, for 10 and for 100
/// SKUs: one fence expiry and one read each.
#[tokio::test]
async fn the_list_and_the_counts_read_in_fixed_statements_for_10_and_100_skus() {
    let (ten, ten_rec) = recorded_door(10).await;
    let (hundred, hundred_rec) = recorded_door(100).await;
    for uri in [
        list(&[
            ("$filter", "lifecycle eq 'published'"),
            ("q", "r0"),
            ("limit", "200"),
        ]),
        list(&[("$orderby", "updated_at desc")]),
        counts(&[
            ("$filter", "lifecycle eq 'draft' and category_id eq null"),
            ("q", "r"),
        ]),
    ] {
        let a = statements(&ten, &ten_rec, &uri).await;
        let b = statements(&hundred, &hundred_rec, &uri).await;
        for (i, (sql, binds)) in b.iter().enumerate() {
            eprintln!("{uri} statement {i} ({binds} binds): {sql}");
        }
        assert_eq!(a.len(), 2, "{uri}: one fence expiry and one read: {a:#?}");
        assert_eq!(a, b, "{uri}: the same statements whatever the size");
    }
}
