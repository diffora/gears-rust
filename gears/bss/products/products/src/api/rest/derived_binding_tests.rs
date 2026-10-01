//! A usage SKU pins a derived usage type at its first publish (P-D-232), through the real doors over a
//! migrated database.
//!
//! Every case runs twice: with the usage-type catalog CONFIGURED (it resolves every GTS ref) and
//! UNCONFIGURED (it answers every ref `Unavailable`, as `gear.rs` installs it when nothing is wired).
//! The catalog of the doors under test counts its calls: a derived ref is this gear's own data, so
//! the catalog is never asked for one, at draft save, at submit or at approve.
//!
//! The derived types are seeded through the repository: with the catalog unconfigured, the derived
//! type doors refuse a write 503 (Run 2), since they resolve each input through the catalog.
//!
//! A SKU on a GTS ref is created and published through a second router over the same database, whose
//! catalog resolves every ref: with no catalog, a GTS usage SKU never publishes (P-D-184).
#![allow(clippy::expect_used, clippy::unwrap_used)]
use crate::api::rest::{ApiState, dto::ProductsDerivedDeclaration};
use crate::domain::derived::{self as rules, NewDerivedType, NewDerivedVersion};
use crate::domain::recognized::UsageTypeAnswer;
use crate::infra::storage::repo::{self, derived_usage_type_repo as store};
use crate::test_support::{
    StubUsageTypes, TestDsn, authed_ctx, body_json, flat_in_enforcer, probe_binding, problem_code,
    raw_i64, repo_connection, resolved_usage_types, rest_app_on_db, test_db, violation_for,
};
use async_trait::async_trait;
use axum::{
    Router,
    body::Body,
    http::{Method, Request},
};
use bss_products_sdk::derived::DerivedUsageDeclaration;
use bss_products_sdk::usage_types::{UsageTypeCatalog, UsageTypePage};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use time::OffsetDateTime;
use toolkit::api::canonical_prelude::CanonicalError;
use toolkit_db::secure::AccessScope;
use toolkit_security::SecurityContext;
use tower::ServiceExt;
use uuid::Uuid;

const CLOUDLET_UNIT: &str = "cloudlet\u{b7}hour";
const AT_1: &str = "products.derived/cloudlets@1";
const AT_2: &str = "products.derived/cloudlets@2";
/// A GTS ref; the configured catalog resolves every ref.
const GTS: &str = "usage:storage";
const RAM_REF: &str = "gts.cf.core.uc.usage_record.v1~cf.test.usage.ram_mb.v1";
const CPU_REF: &str = "gts.cf.core.uc.usage_record.v1~cf.test.usage.cpu_mhz.v1";

/// A catalog that counts every call and answers as `inner` does.
struct Counting {
    inner: Arc<dyn UsageTypeCatalog>,
    asked: AtomicUsize,
}
impl Counting {
    fn asked(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }
}
#[async_trait]
impl UsageTypeCatalog for Counting {
    async fn resolve(&self, ctx: &SecurityContext, usage_type_ref: &str) -> UsageTypeAnswer {
        self.asked.fetch_add(1, Ordering::SeqCst);
        self.inner.resolve(ctx, usage_type_ref).await
    }
    async fn list(
        &self,
        ctx: &SecurityContext,
        q: Option<&str>,
        kind: Option<&str>,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<UsageTypePage, CanonicalError> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        self.inner.list(ctx, q, kind, limit, cursor).await
    }
}

/// The two catalogs every case runs with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Leg {
    Configured,
    Unconfigured,
}
const LEGS: [Leg; 2] = [Leg::Configured, Leg::Unconfigured];
impl Leg {
    fn catalog(self) -> Arc<dyn UsageTypeCatalog> {
        match self {
            Self::Configured => Arc::new(StubUsageTypes::always(UsageTypeAnswer::Resolved(
                probe_binding(),
            ))),
            Self::Unconfigured => Arc::new(crate::infra::usage_types::UnconfiguredUsageTypes),
        }
    }
    fn source(self) -> &'static str {
        match self {
            Self::Configured => "test",
            Self::Unconfigured => crate::gear::USAGE_TYPE_SOURCE_UNCONFIGURED,
        }
    }
}

fn routes(s: Arc<ApiState>, o: &dyn toolkit::api::OpenApiRegistry) -> Router {
    crate::api::rest::skus::router(s.clone(), o)
        .merge(crate::api::rest::sku_governance::router(s.clone(), o))
        .merge(crate::api::rest::approval_units::router(s.clone(), o))
        .merge(crate::api::rest::approval_policy::router(s, o))
}

fn share(name: &str, divisor: &str) -> Value {
    json!({"op":"ceil","arg":{"op":"div_const","arg":{"op":"input","name":name},"divisor":divisor}})
}
/// The cloudlet of decision 1 on the wire, its CPU share's divisor given.
fn cloudlet(cpu_divisor: &str) -> Value {
    json!({
        "output_unit": CLOUDLET_UNIT,
        "granularity": "hour",
        "inputs": [
            {"name":"ram_mb","usage_type_ref":RAM_REF,"granule_fold":"peak","unit":"MB"},
            {"name":"cpu_mhz","usage_type_ref":CPU_REF,"granule_fold":"peak","unit":"MHz"}
        ],
        "formula": {"op":"max","args":[share("ram_mb","128"), share("cpu_mhz",cpu_divisor)]},
        "output_scale": 0,
        "output_round": "half_even"
    })
}

/// A derived type `code` of `tenant` with one version per declaration, through the repository.
async fn seed(dsn: &str, tenant: Uuid, code: &str, declarations: &[Value]) {
    let (db, _) = repo_connection(dsn, tenant).await;
    let conn = db.conn().unwrap();
    let scope = AccessScope::for_tenant(tenant);
    let now = OffsetDateTime::now_utc();
    let t = store::create_type(
        &conn,
        &scope,
        tenant,
        NewDerivedType {
            code: code.to_owned(),
            name: format!("{code} name"),
        },
        Uuid::from_u128(7),
        now,
    )
    .await
    .unwrap();
    for (n, wire) in (1..).zip(declarations) {
        let dto: ProductsDerivedDeclaration = serde_json::from_value(wire.clone()).unwrap();
        let declaration = DerivedUsageDeclaration::try_from(&dto).unwrap();
        store::insert_version(
            &conn,
            &scope,
            tenant,
            NewDerivedVersion {
                type_id: t.id,
                version: n,
                declaration_json: serde_json::to_value(ProductsDerivedDeclaration::from(
                    &declaration,
                ))
                .unwrap(),
                digest: rules::digest_hex(&declaration),
                created_by: Uuid::from_u128(7),
                created_at: now,
            },
        )
        .await
        .unwrap();
    }
}

struct F {
    leg: Leg,
    /// The doors under test, with the leg's counting catalog.
    app: Router,
    catalog: Arc<Counting>,
    /// The same doors over the same database with a catalog that resolves every ref: the GTS
    /// SKUs' setup. Held for the test's life: its outbox serves both routers.
    setup: Router,
    /// Held for the test's life: its temporary directory holds the database.
    dsn: TestDsn,
    tenant: Uuid,
    author: SecurityContext,
    reviewer: SecurityContext,
}

impl F {
    /// The tenant holds `cloudlets` with versions 1 and 2, both selling `cloudlet·hour`; another
    /// tenant holds `foreign` with version 1.
    async fn new(leg: Leg) -> Self {
        let (db, _, _, dsn) = test_db().await;
        let tenant = Uuid::new_v4();
        let (setup, state) =
            rest_app_on_db(tenant, routes, resolved_usage_types(), "test", db).await;
        let (db, _) = repo_connection(&dsn, tenant).await;
        let catalog = Arc::new(Counting {
            inner: leg.catalog(),
            asked: AtomicUsize::new(0),
        });
        let leg_state = Arc::new(ApiState {
            db,
            sink: state.sink.clone(),
            usage_type_catalog: catalog.clone(),
            usage_type_catalog_source: leg.source(),
            idempotency_retention_hours: 24,
            fence_ttl_minutes: 30,
            reference_principals: state.reference_principals.clone(),
            hub: state.hub.clone(),
        });
        let app = routes(leg_state, &toolkit::api::OpenApiRegistryImpl::new())
            .layer(axum::Extension(flat_in_enforcer(tenant)));
        seed(
            &dsn,
            tenant,
            "cloudlets",
            &[cloudlet("400"), cloudlet("500")],
        )
        .await;
        seed(&dsn, Uuid::new_v4(), "foreign", &[cloudlet("400")]).await;
        Self {
            leg,
            app,
            catalog,
            setup,
            dsn,
            tenant,
            author: authed_ctx(tenant),
            reviewer: authed_ctx(tenant),
        }
    }

    async fn call(
        &self,
        app: &Router,
        ctx: &SecurityContext,
        method: Method,
        path: &str,
        body: Value,
        if_match: Option<&str>,
    ) -> (u16, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(format!("/bss-products/v1{path}"))
            .extension(ctx.clone())
            .header("Content-Type", "application/json");
        if let Some(tag) = if_match {
            request = request.header("If-Match", tag);
        }
        let r = app
            .clone()
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        (r.status().as_u16(), body_json(r).await)
    }

    /// P-D-205: the policy is written at the tag its read answered.
    async fn policy(&self, quorum: u32) {
        let r = self
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/bss-products/v1/approval-policy")
                    .extension(self.author.clone())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let tag = r.headers()["etag"].to_str().unwrap().to_owned();
        let (status, b) = self
            .call(
                &self.app,
                &self.author,
                Method::PUT,
                "/approval-policy",
                json!({ "quorum": quorum }),
                Some(&tag),
            )
            .await;
        assert_eq!(status, 200, "{b}");
    }

    /// `POST /skus` through `app` as the author: a usage SKU on `reference` with `unit`.
    async fn create(
        &self,
        app: &Router,
        code: &str,
        reference: Option<&str>,
        unit: Option<&str>,
    ) -> (u16, Value) {
        let body = json!({
            "code": code, "name": code, "type": "usage",
            "usage_type_ref": reference, "unit": unit,
        });
        self.call(app, &self.author, Method::POST, "/skus", body, None)
            .await
    }

    /// [`F::create`] that must succeed; the SKU's id.
    async fn draft(&self, app: &Router, code: &str, reference: &str, unit: &str) -> Uuid {
        let (status, s) = self.create(app, code, Some(reference), Some(unit)).await;
        assert_eq!(status, 201, "{:?}: {s}", self.leg);
        Uuid::parse_str(s["id"].as_str().unwrap()).unwrap()
    }

    /// `PATCH /skus/{id}` through the doors under test, at the draft's current revision.
    async fn patch(&self, id: Uuid, body: Value) -> (u16, Value) {
        let revision = self.card(id).await["revision"].as_i64().unwrap();
        self.call(
            &self.app,
            &self.author,
            Method::PATCH,
            &format!("/skus/{id}"),
            body,
            Some(&format!("\"{revision}\"")),
        )
        .await
    }

    /// `POST /skus/{id}{suffix}` through `app` as the author.
    async fn post(&self, app: &Router, id: Uuid, suffix: &str, body: Value) -> (u16, Value) {
        self.call(
            app,
            &self.author,
            Method::POST,
            &format!("/skus/{id}{suffix}"),
            body,
            None,
        )
        .await
    }

    /// Submit the draft through `app` at quorum 0: the submit is the publish.
    async fn publish(&self, app: &Router, id: Uuid) {
        self.policy(0).await;
        let (status, b) = self.post(app, id, "/submit", json!({})).await;
        assert_eq!(status, 200, "{:?}: {b}", self.leg);
        assert_eq!(b["applied"], true, "{b}");
    }

    /// The reviewer's approve of `unit` at `generation`, through the doors under test.
    async fn approve(&self, unit: &Value, generation: i32) -> (u16, Value) {
        self.call(
            &self.app,
            &self.reviewer,
            Method::POST,
            &format!(
                "/approval-units/{}/approve",
                unit["unit"]["id"].as_str().unwrap()
            ),
            json!({ "generation": generation }),
            None,
        )
        .await
    }

    async fn card(&self, id: Uuid) -> Value {
        let (status, b) = self
            .call(
                &self.setup,
                &self.author,
                Method::GET,
                &format!("/skus/{id}"),
                json!({}),
                None,
            )
            .await;
        assert_eq!(status, 200, "{b}");
        b["sku"].clone()
    }

    async fn skus(&self) -> i64 {
        raw_i64(&self.dsn, "SELECT COUNT(*) AS v FROM products_sku").await
    }

    async fn units(&self) -> i64 {
        raw_i64(
            &self.dsn,
            "SELECT COUNT(*) AS v FROM products_approval_unit",
        )
        .await
    }

    /// A concurrent writer's content change on the head, outside every door.
    async fn drift(&self, id: Uuid, reference: &str, unit: &str) {
        let (db, scope) = repo_connection(&self.dsn, self.tenant).await;
        let conn = db.conn().unwrap();
        let mut c = bss_products_sdk::models::SkuContent::from(
            &repo::find_sku(&conn, &scope, self.tenant, id)
                .await
                .unwrap()
                .unwrap(),
        );
        c.usage_type_ref = Some(reference.to_owned());
        c.unit = Some(unit.to_owned());
        repo::write_sku_content(
            &conn,
            &scope,
            self.tenant,
            id,
            &c,
            OffsetDateTime::now_utc(),
        )
        .await
        .unwrap();
    }

    fn assert_never_asked(&self) {
        assert_eq!(
            self.catalog.asked(),
            0,
            "{:?}: the catalog is never asked for a derived ref",
            self.leg
        );
    }
}

/// The 400's code and the subject its violation names.
fn refused(status: u16, body: &Value, code: &str, subject: &str, leg: Leg) {
    assert_eq!(status, 400, "{leg:?}: {body}");
    assert_eq!(problem_code(body), code, "{leg:?}: {body}");
    assert!(
        violation_for(body, subject).is_some(),
        "{leg:?}: the violation names {subject}: {body}"
    );
}

/// A draft on `products.derived/cloudlets@1` selling its output unit is created, submitted and
/// approved: the SKU is published on that version. The catalog is asked nothing on the way.
#[tokio::test]
async fn a_usage_sku_on_a_derived_version_is_created_submitted_and_approved_without_the_catalog() {
    for leg in LEGS {
        let f = F::new(leg).await;
        f.policy(1).await;
        let id = f.draft(&f.app, "CL", AT_1, CLOUDLET_UNIT).await;
        let (status, unit) = f.post(&f.app, id, "/submit", json!({})).await;
        assert_eq!(status, 200, "{leg:?}: {unit}");
        assert_eq!(unit["applied"], false, "{unit}");
        let (status, b) = f.approve(&unit, 1).await;
        assert_eq!(status, 200, "{leg:?}: {b}");
        let s = f.card(id).await;
        assert_eq!(s["lifecycle"], "published", "{s}");
        assert_eq!(s["usage_type_ref"], AT_1);
        assert_eq!(s["unit"], CLOUDLET_UNIT);
        assert_eq!(s["published_version"], 1);
        f.assert_never_asked();
    }
}

/// A derived ref the tenant does not hold is 400 `DERIVED_USAGE_TYPE_UNKNOWN` at draft save, with
/// the catalog unconfigured too: an unknown version, an unknown code, another tenant's type, a
/// version that is not canonical and a ref with no version. A draft PATCH is judged the same way.
#[tokio::test]
async fn an_unknown_derived_version_is_refused_at_draft_save() {
    for leg in LEGS {
        let f = F::new(leg).await;
        for reference in [
            "products.derived/cloudlets@3",
            "products.derived/nothing@1",
            "products.derived/foreign@1",
            "products.derived/cloudlets@01",
            "products.derived/cloudlets@0",
            "products.derived/cloudlets",
            "products.derived/",
        ] {
            let (status, b) = f
                .create(&f.app, "U", Some(reference), Some(CLOUDLET_UNIT))
                .await;
            refused(
                status,
                &b,
                "DERIVED_USAGE_TYPE_UNKNOWN",
                "usage_type_ref",
                leg,
            );
        }
        assert_eq!(f.skus().await, 0, "{leg:?}: no draft was written");
        let (status, s) = f.create(&f.app, "P", None, None).await;
        assert_eq!(status, 201, "{leg:?}: {s}");
        let id = Uuid::parse_str(s["id"].as_str().unwrap()).unwrap();
        let (status, b) = f
            .patch(
                id,
                json!({"usage_type_ref":"products.derived/cloudlets@3","unit":CLOUDLET_UNIT}),
            )
            .await;
        refused(
            status,
            &b,
            "DERIVED_USAGE_TYPE_UNKNOWN",
            "usage_type_ref",
            leg,
        );
        assert!(f.card(id).await["usage_type_ref"].is_null());
        f.assert_never_asked();
    }
}

/// A unit other than the version's output unit is 400 `DERIVED_UNIT_MISMATCH`, at the create and
/// at a draft PATCH that changes the unit or the ref. A draft may leave its unit for later; its
/// publish then needs one (`USAGE_NEEDS_METER`).
#[tokio::test]
async fn a_unit_other_than_the_output_unit_is_refused() {
    for leg in LEGS {
        let f = F::new(leg).await;
        let (status, b) = f.create(&f.app, "M", Some(AT_1), Some("GB")).await;
        refused(status, &b, "DERIVED_UNIT_MISMATCH", "unit", leg);
        assert_eq!(f.skus().await, 0);
        let (status, s) = f.create(&f.app, "M", Some(AT_1), None).await;
        assert_eq!(status, 201, "{leg:?}: a draft names its unit later: {s}");
        let id = Uuid::parse_str(s["id"].as_str().unwrap()).unwrap();
        let (status, b) = f.patch(id, json!({"unit":"GB"})).await;
        refused(status, &b, "DERIVED_UNIT_MISMATCH", "unit", leg);
        f.policy(0).await;
        let (status, b) = f.post(&f.app, id, "/submit", json!({})).await;
        refused(status, &b, "USAGE_NEEDS_METER", "unit", leg);
        let (status, b) = f.patch(id, json!({"unit":CLOUDLET_UNIT})).await;
        assert_eq!(status, 200, "{leg:?}: {b}");
        // A GTS draft selling `GB` cannot move onto the derived version with its unit.
        let (status, s) = f.create(&f.setup, "G", Some(GTS), Some("GB")).await;
        assert_eq!(status, 201, "{s}");
        let gts = Uuid::parse_str(s["id"].as_str().unwrap()).unwrap();
        let (status, b) = f.patch(gts, json!({ "usage_type_ref": AT_1 })).await;
        refused(status, &b, "DERIVED_UNIT_MISMATCH", "unit", leg);
        f.publish(&f.app, id).await;
        assert_eq!(f.card(id).await["lifecycle"], "published");
        f.assert_never_asked();
    }
}

/// A draft that was never published may move its pin: `@1` → `@2` is a 200, and the SKU publishes
/// on `@2`.
#[tokio::test]
async fn a_draft_moves_its_derived_pin_until_its_first_publish() {
    for leg in LEGS {
        let f = F::new(leg).await;
        let id = f.draft(&f.app, "D", AT_1, CLOUDLET_UNIT).await;
        let (status, b) = f.patch(id, json!({ "usage_type_ref": AT_2 })).await;
        assert_eq!(status, 200, "{leg:?}: {b}");
        assert_eq!(b["usage_type_ref"], AT_2, "{b}");
        f.publish(&f.app, id).await;
        let s = f.card(id).await;
        assert_eq!(s["lifecycle"], "published");
        assert_eq!(s["usage_type_ref"], AT_2);
        f.assert_never_asked();
    }
}

/// After its first publish a usage SKU keeps its derived pin: a change to `@2`, to a GTS ref, or
/// one that drops the ref (with a type change, or alone) is 400 `DERIVED_PIN_IMMUTABLE` at submit;
/// no unit is recorded and no fence is left. A unit other than the pinned version's output unit is
/// 400 `DERIVED_UNIT_MISMATCH`. A change that leaves the ref alone still applies.
#[tokio::test]
async fn after_its_first_publish_a_usage_sku_keeps_its_derived_pin() {
    for leg in LEGS {
        let f = F::new(leg).await;
        let id = f.draft(&f.app, "K", AT_1, CLOUDLET_UNIT).await;
        f.publish(&f.app, id).await;
        let units = f.units().await;
        for patch in [
            json!({ "usage_type_ref": AT_2 }),
            json!({ "usage_type_ref": GTS, "unit": "GB" }),
            json!({ "type": "recurring", "usage_type_ref": null, "unit": null }),
            json!({ "usage_type_ref": null }),
        ] {
            let (status, b) = f.post(&f.app, id, "/changes", patch.clone()).await;
            refused(status, &b, "DERIVED_PIN_IMMUTABLE", "usage_type_ref", leg);
            let s = f.card(id).await;
            assert_eq!(s["usage_type_ref"], AT_1, "{patch}");
            assert_eq!(s["type"], "usage", "{patch}");
            assert_eq!(s["type_change_pending"], false, "{patch}: no fence is left");
            assert!(s["pending_unit_id"].is_null(), "{patch}");
            assert_eq!(s["published_version"], 1, "{patch}");
        }
        let (status, b) = f.post(&f.app, id, "/changes", json!({"unit":"GB"})).await;
        refused(status, &b, "DERIVED_UNIT_MISMATCH", "unit", leg);
        assert_eq!(f.card(id).await["unit"], CLOUDLET_UNIT);
        assert_eq!(f.units().await, units, "{leg:?}: no unit was recorded");
        let (status, b) = f
            .post(&f.app, id, "/changes", json!({"gl_code":"4012"}))
            .await;
        assert_eq!(status, 200, "{leg:?}: {b}");
        let s = f.card(id).await;
        assert_eq!(s["published_version"], 2);
        assert_eq!(s["usage_type_ref"], AT_1);
        f.assert_never_asked();
    }
}

/// A published GTS usage SKU cannot take a derived pin by a change: 400 `DERIVED_PIN_IMMUTABLE`,
/// with the unit made to match. A GTS → GTS change still applies, as before.
#[tokio::test]
async fn a_published_gts_usage_sku_cannot_take_a_derived_pin() {
    for leg in LEGS {
        let f = F::new(leg).await;
        let id = f.draft(&f.setup, "G", GTS, "GB").await;
        f.publish(&f.setup, id).await;
        let (status, b) = f
            .post(
                &f.app,
                id,
                "/changes",
                json!({ "usage_type_ref": AT_1, "unit": CLOUDLET_UNIT }),
            )
            .await;
        refused(status, &b, "DERIVED_PIN_IMMUTABLE", "usage_type_ref", leg);
        let s = f.card(id).await;
        assert_eq!(s["usage_type_ref"], GTS);
        assert_eq!(s["published_version"], 1);
        f.assert_never_asked();
        let (status, b) = f
            .post(
                &f.setup,
                id,
                "/changes",
                json!({"usage_type_ref":"usage:other"}),
            )
            .await;
        assert_eq!(status, 200, "{leg:?}: GTS to GTS: {b}");
        assert_eq!(f.card(id).await["usage_type_ref"], "usage:other");
    }
}

/// A change that was legal when submitted is judged again at apply. A GTS usage SKU's type change
/// that drops its ref waits for review; a concurrent writer then pins the head to
/// `products.derived/cloudlets@1`. The unit's fingerprint covers only what it proposes, which the
/// writer did not change, so the approve is not refreshed: it applies, and the apply refuses it,
/// 409 `DERIVED_PIN_IMMUTABLE`, the head as the writer left it and the unit still pending. The
/// catalog is asked nothing on the way.
///
/// With the catalog configured, a GTS → GTS change met by the same writer (its unit moved too, so
/// the proposal changes) is refreshed first, and the approve of the refreshed generation is refused
/// at apply too. With the catalog unconfigured, that approve is 503, since the proposed GTS ref is
/// the catalog's (P-D-184).
#[tokio::test]
async fn a_stale_change_is_refused_at_apply_when_a_concurrent_write_pinned_a_derived_type() {
    for leg in LEGS {
        let f = F::new(leg).await;
        let id = f.draft(&f.setup, "S", GTS, "GB").await;
        f.publish(&f.setup, id).await;
        f.policy(1).await;
        let (status, unit) = f
            .post(
                &f.app,
                id,
                "/changes",
                json!({ "type": "recurring", "usage_type_ref": null, "unit": null }),
            )
            .await;
        assert_eq!(status, 200, "{leg:?}: {unit}");
        assert_eq!(unit["applied"], false, "{unit}");
        f.drift(id, AT_1, CLOUDLET_UNIT).await;
        let left = f.card(id).await;
        let (status, b) = f.approve(&unit, 1).await;
        assert_eq!(status, 409, "{leg:?}: {b}");
        assert_eq!(problem_code(&b), "DERIVED_PIN_IMMUTABLE", "{leg:?}: {b}");
        let s = f.card(id).await;
        assert_eq!(s, left, "the head as the writer left it");
        assert_eq!(s["type"], "usage", "{s}");
        assert_eq!(s["usage_type_ref"], AT_1, "{s}");
        assert_eq!(s["pending_unit_id"], unit["unit"]["id"], "still pending");
        f.assert_never_asked();

        let other = f.draft(&f.setup, "T", GTS, "GB").await;
        f.publish(&f.setup, other).await;
        f.policy(1).await;
        let (status, unit) = f
            .post(
                &f.setup,
                other,
                "/changes",
                json!({"usage_type_ref":"usage:other"}),
            )
            .await;
        assert_eq!(status, 200, "{unit}");
        f.drift(other, AT_1, CLOUDLET_UNIT).await;
        let (status, b) = f.approve(&unit, 1).await;
        if leg == Leg::Unconfigured {
            assert_eq!(status, 503, "the GTS proposal is the catalog's: {b}");
            continue;
        }
        assert_eq!(status, 400, "the refresh: {b}");
        assert_eq!(problem_code(&b), "UNIT_STALE", "{b}");
        let (status, b) = f.approve(&unit, 2).await;
        assert_eq!(status, 409, "{b}");
        assert_eq!(problem_code(&b), "DERIVED_PIN_IMMUTABLE", "{b}");
        assert_eq!(f.card(other).await["usage_type_ref"], AT_1);
    }
}
