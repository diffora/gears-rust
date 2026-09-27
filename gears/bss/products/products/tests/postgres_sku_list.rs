#![allow(clippy::expect_used, clippy::unwrap_used)]
//! The SKU list and its counts on `PostgreSQL` (P-D-210, P-D-211): the text search folds case
//! with Postgres's Unicode `lower()` and takes wildcards literally, the null filters and the
//! cursor order hold on the engine production runs, and the counts group in one statement.
mod pg_support;

use bss_products::{
    domain::{category::NewCategory, sku::NewSku},
    infra::storage::repo::{self, SkuCounts, SkuListFilter},
};
use bss_products_sdk::models::{Lifecycle, SkuType};
use pg_support::Pg;
use sea_orm::DbBackend;
use time::OffsetDateTime;
use toolkit_db::{Db, secure::AccessScope};
use toolkit_odata::{ODataOrderBy, ODataQuery, OrderKey, SortDir};
use uuid::Uuid;

struct Fixture {
    db: Db,
    scope: AccessScope,
    tenant: Uuid,
}
impl Fixture {
    async fn new() -> (Pg, Self) {
        let pg = Pg::applied().await;
        let db = pg.db().await;
        let tenant = Uuid::new_v4();
        let f = Self {
            db,
            scope: AccessScope::for_tenant(tenant),
            tenant,
        };
        (pg, f)
    }
    async fn category(&self, code: &str) -> Uuid {
        repo::insert_category(
            &self.db.conn().unwrap(),
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
    async fn sku(
        &self,
        code: &str,
        name: &str,
        category: Option<Uuid>,
        lifecycle: Lifecycle,
        unit: Option<&str>,
    ) -> Uuid {
        let conn = self.db.conn().unwrap();
        let now = OffsetDateTime::now_utc();
        let s = repo::insert_sku(
            &conn,
            &self.scope,
            self.tenant,
            NewSku {
                code: code.into(),
                name: name.into(),
                r#type: SkuType::Usage,
                category_id: category,
                description: String::new(),
                sellable: true,
                gl_code: None,
                tax_category: None,
                invoice_line_template: None,
                billing_timing: None,
                usage_type_ref: None,
                unit: unit.map(Into::into),
            },
            self.tenant,
            now,
        )
        .await
        .unwrap();
        if lifecycle != Lifecycle::Draft {
            repo::set_lifecycle(
                &conn,
                &self.scope,
                self.tenant,
                s.id,
                &[Lifecycle::Draft],
                lifecycle,
                now,
            )
            .await
            .unwrap();
        }
        s.id
    }
    async fn page(&self, filter: SkuListFilter, query: &ODataQuery) -> Vec<String> {
        let page = repo::page_skus(
            &self.db.conn().unwrap(),
            &self.scope,
            self.tenant,
            DbBackend::Postgres,
            &filter,
            query,
        )
        .await
        .unwrap();
        page.items.into_iter().map(|s| s.code).collect()
    }
    async fn codes(&self, q: Option<&str>, filter: Option<&str>) -> Vec<String> {
        let mut query = ODataQuery::default();
        if let Some(raw) = filter {
            query = query.with_filter(toolkit_odata::parse_filter_string(raw).unwrap().into_expr());
        }
        let mut codes = self
            .page(
                SkuListFilter {
                    text: q.map(Into::into),
                },
                &query,
            )
            .await;
        codes.sort();
        codes
    }
}
fn sorted(mut v: Vec<&str>) -> Vec<String> {
    v.sort_unstable();
    v.into_iter().map(str::to_owned).collect()
}

#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn q_folds_unicode_case_and_takes_wildcards_literally_on_postgres() {
    let (_pg, f) = Fixture::new().await;
    f.sku("STOR-1", "Storage", None, Lifecycle::Draft, Some("GiB"))
        .await;
    f.sku("COMP", "Compute 50%_off", None, Lifecycle::Draft, None)
        .await;
    f.sku("DECOY", "Compute 50xyoff", None, Lifecycle::Draft, None)
        .await;
    f.sku("BACK", r"Back\slash", None, Lifecycle::Draft, None)
        .await;
    f.sku(
        "CYR",
        "\u{411}\u{435}\u{442}\u{430}",
        None,
        Lifecycle::Draft,
        None,
    ) // Beta, in Cyrillic
    .await;
    for (q, expected) in [
        ("stor", vec!["STOR-1"]),
        ("STORAGE", vec!["STOR-1"]),
        ("gib", vec!["STOR-1"]),
        ("%", vec!["COMP"]),
        ("_", vec!["COMP"]),
        ("50%_", vec!["COMP"]),
        (r"\", vec!["BACK"]),
        ("compute 50", vec!["COMP", "DECOY"]),
        ("\u{411}\u{435}\u{442}\u{430}", vec!["CYR"]),
        // Postgres folds Unicode: the other case of a Cyrillic letter matches here (SQLite's
        // `lower()` folds ASCII only; the SQLite door test pins the difference).
        ("\u{431}\u{435}\u{442}\u{430}", vec!["CYR"]),
        ("\u{411}\u{415}\u{422}\u{410}", vec!["CYR"]),
    ] {
        assert_eq!(f.codes(Some(q), None).await, sorted(expected), "q={q}");
    }
}

#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn null_filters_the_order_and_the_counts_hold_on_postgres() {
    let (_pg, f) = Fixture::new().await;
    let x = f.category("x").await;
    f.sku("A", "zeta", Some(x), Lifecycle::Published, None)
        .await;
    f.sku("B", "alpha", None, Lifecycle::Draft, None).await;
    f.sku("C", "mu", None, Lifecycle::Deprecated, None).await;
    let d = f.sku("D", "beta", Some(x), Lifecycle::Draft, None).await;
    // D is in review: a pending unit locks it.
    let conn = f.db.conn().unwrap();
    let revision = repo::find_sku(&conn, &f.scope, f.tenant, d)
        .await
        .unwrap()
        .unwrap()
        .revision;
    assert!(
        repo::try_lock_sku(&conn, &f.scope, f.tenant, d, Uuid::new_v4(), revision)
            .await
            .unwrap()
    );
    assert_eq!(f.codes(None, Some("category_id eq null")).await, ["B", "C"]);
    assert_eq!(f.codes(None, Some("category_id ne null")).await, ["A", "D"]);
    assert_eq!(f.codes(None, Some("pending_unit_id ne null")).await, ["D"]);
    assert_eq!(
        f.codes(
            None,
            Some(&format!("category_id eq {x} and lifecycle eq 'draft'"))
        )
        .await,
        ["D"]
    );
    // Name order walks with the cursor on Postgres too.
    let mut query = ODataQuery::default()
        .with_order(ODataOrderBy(vec![OrderKey {
            field: "name".into(),
            dir: SortDir::Desc,
        }]))
        .with_limit(3);
    let first = repo::page_skus(
        &conn,
        &f.scope,
        f.tenant,
        DbBackend::Postgres,
        &SkuListFilter::default(),
        &query,
    )
    .await
    .unwrap();
    let names: Vec<_> = first.items.iter().map(|s| s.name.clone()).collect();
    assert_eq!(names, ["zeta", "mu", "beta"]);
    let cursor = first.page_info.next_cursor.unwrap();
    query = ODataQuery::default()
        .with_cursor(toolkit_odata::CursorV1::decode(&cursor).unwrap())
        .with_limit(3);
    let rest = f.page(SkuListFilter::default(), &query).await;
    assert_eq!(rest, ["B"]);
    // The counts: one grouped statement, in review across lifecycles.
    let counts = repo::count_skus(
        &conn,
        &f.scope,
        f.tenant,
        DbBackend::Postgres,
        &SkuListFilter::default(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        counts,
        SkuCounts {
            all: 4,
            draft: 2,
            published: 1,
            deprecated: 1,
            retiring: 0,
            retired: 0,
            in_review: 1,
        }
    );
}
