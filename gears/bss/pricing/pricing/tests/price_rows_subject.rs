//! The `price_rows` approval subject driven by the shared engine on a real database.
#![allow(clippy::expect_used, clippy::unwrap_used)]
mod price_support;
use bss_approval::{ApprovalError, ApprovalSubject, Engine, Policy, SubmitRequest, Unit};
use bss_pricing::infra::{
    price_rows::{PriceRowsSubject, Release},
    storage::{
        entity::price_row,
        repo::{approval_repo::PricingApprovalStore, price_repo, row_repo},
    },
};
use price_support::{Fixture, Script};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};
use time::Date;
use toolkit_db::{DbTx, secure::AccessScope};
use uuid::Uuid;

#[derive(Debug)]
enum TestErr {
    Approval(ApprovalError),
    Db,
}
impl From<toolkit_db::DbError> for TestErr {
    fn from(_: toolkit_db::DbError) -> Self {
        Self::Db
    }
}
impl From<ApprovalError> for TestErr {
    fn from(e: ApprovalError) -> Self {
        Self::Approval(e)
    }
}
/// The subject's own code for a submit refusal; the engine category otherwise.
fn code(e: &TestErr) -> String {
    match e {
        TestErr::Approval(ApprovalError::InvalidSubmit { code, .. }) => (*code).to_owned(),
        TestErr::Approval(other) => other.code().to_owned(),
        TestErr::Db => "DB".into(),
    }
}
fn day(s: &str) -> Date {
    Date::parse(s, &time::format_description::well_known::Iso8601::DATE).unwrap()
}

struct Setup {
    f: Fixture,
    script: Arc<Script>,
    book: Uuid,
    price: Value,
}
async fn setup(mode: usize) -> Setup {
    setup_with(mode, false).await
}
async fn setup_with(mode: usize, dimension: bool) -> Setup {
    let script = Arc::new(Script::default());
    script.set(mode);
    let f = Fixture::new(script.clone()).await;
    let (book, _) = f.book().await;
    let book: Uuid = book["id"].as_str().unwrap().parse().unwrap();
    if dimension {
        let (_, _, tag) = f
            .call("GET", "/dimension-keys", json!({}), None, None)
            .await;
        let saved = f
            .call(
                "PUT",
                "/dimension-keys",
                json!({"items":[{"key":"region","values":["eu","us"]}]}),
                Some(&tag),
                None,
            )
            .await;
        assert_eq!(saved.0, 200, "{saved:?}");
    }
    let body = match (mode, dimension) {
        (11, _) => json!({"sku_id":Uuid::new_v4(),"period":"month"}),
        (_, true) => json!({"sku_id":Uuid::new_v4(),"dimension_key":"region"}),
        _ => json!({"sku_id":Uuid::new_v4()}),
    };
    let (status, price, _) = f
        .call(
            "POST",
            &format!("/price-books/{book}/prices"),
            body,
            None,
            Some("price"),
        )
        .await;
    assert_eq!(status, 201, "{price}");
    Setup {
        f,
        script,
        book,
        price,
    }
}
impl Setup {
    fn tenant(&self) -> Uuid {
        self.f.ctx.subject_tenant_id()
    }
    fn price_id(&self) -> Uuid {
        self.price["id"].as_str().unwrap().parse().unwrap()
    }
    fn subject(&self) -> PriceRowsSubject {
        PriceRowsSubject::new(
            self.f.ctx.clone(),
            self.f.state.hub.clone(),
            self.book,
            time::OffsetDateTime::now_utc(),
        )
    }
    fn store(&self) -> PricingApprovalStore {
        PricingApprovalStore {
            scope: AccessScope::for_tenant(self.tenant()),
            tenant_id: self.tenant(),
        }
    }
    async fn draft(&self, key: &str, body: Value) -> Vec<Uuid> {
        let (status, b, _) = self
            .f
            .call(
                "POST",
                &format!("/prices/{}/rows", self.price_id()),
                body,
                None,
                Some(key),
            )
            .await;
        assert_eq!(status, 201, "{b}");
        b["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap().parse().unwrap())
            .collect()
    }
    async fn approved(&self, version_no: i32, from: &str, model: &str, price: Value) -> Uuid {
        let conn = self.f.db.conn().unwrap();
        let scope = AccessScope::for_tenant(self.tenant());
        let p = price_repo::find(&conn, &scope, self.tenant(), self.price_id())
            .await
            .unwrap()
            .unwrap();
        let mut r = price_support::row(&p);
        r.version_no = version_no;
        r.state = "approved".into();
        r.model = model.into();
        r.price_json = price;
        r.effective_from = day(from);
        row_repo::insert(&conn, &scope, r).await.unwrap().id
    }
    async fn row(&self, id: Uuid) -> price_row::Model {
        row_repo::find(
            &self.f.db.conn().unwrap(),
            &AccessScope::for_tenant(self.tenant()),
            self.tenant(),
            id,
        )
        .await
        .unwrap()
        .unwrap()
    }
    /// Run engine work in one serializable transaction.
    async fn tx<T: Send + 'static>(
        &self,
        work: impl for<'a> Fn(
            &'a DbTx<'a>,
            PricingApprovalStore,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<T, TestErr>> + Send + 'a>,
        > + Send
        + Sync
        + 'static,
    ) -> Result<T, TestErr> {
        let store = self.store();
        let work = Arc::new(work);
        self.f
            .db
            .db()
            .transaction_with_retry(
                toolkit_db::secure::TxConfig::serializable(),
                |e: &TestErr| match e {
                    TestErr::Approval(a) => a.db_err(),
                    TestErr::Db => None,
                },
                move |tx| work(tx, store.clone()),
            )
            .await
    }
    async fn submit(
        &self,
        subject: PriceRowsSubject,
        ids: Vec<Uuid>,
        quorum: u32,
    ) -> Result<bss_approval::Submitted, TestErr> {
        let (tenant, book, actor) = (self.tenant(), self.book, self.f.ctx.subject_id());
        self.tx(move |tx, store| {
            let (subject, ids) = (subject.clone(), ids.clone());
            Box::pin(async move {
                let policy = Policy {
                    default_quorum: quorum,
                    overrides: BTreeMap::new(),
                };
                let date = subject.common_effective_date;
                Ok(Engine::submit(
                    &store,
                    &subject,
                    tx,
                    SubmitRequest {
                        tenant_id: tenant,
                        ref_id: book,
                        item_ids: &ids,
                        actor,
                        policy: &policy,
                        common_effective_date: date,
                        now: time::OffsetDateTime::now_utc(),
                    },
                )
                .await?)
            })
        })
        .await
    }
    async fn approve(&self, subject: PriceRowsSubject, unit: &Unit) -> Result<(), TestErr> {
        let (id, generation) = (unit.id, unit.generation);
        let reviewer = Uuid::new_v4();
        self.tx(move |tx, store| {
            let subject = subject.clone();
            Box::pin(async move {
                Engine::approve(
                    &store,
                    &subject,
                    tx,
                    id,
                    reviewer,
                    generation,
                    None,
                    time::OffsetDateTime::now_utc(),
                )
                .await?;
                Ok(())
            })
        })
        .await
    }
}
fn body(from: &str) -> Value {
    json!({"model":"per_unit","price":{"rate":"0.10"},"eligibility":"all","effective_from":from})
}

#[tokio::test]
async fn collect_is_business_content_with_the_pair_partner_and_the_chain_predecessor() {
    let s = setup(0).await;
    let back = s
        .approved(1, "2031-01-01", "per_unit", json!({"rate":"0.20"}))
        .await;
    let mut promo = body("2031-03-01");
    promo["temporary_until"] = json!("2031-03-11");
    let pair = s.draft("pair", promo).await;
    let subject = s.subject();
    let promo_id = pair[0];
    let items = s
        .tx(move |tx, _| {
            let subject = subject.clone();
            Box::pin(async move { Ok(subject.collect(tx, &[promo_id]).await?) })
        })
        .await
        .unwrap();
    let mut ids: Vec<_> = items.iter().map(|i| i.item_id).collect();
    ids.sort();
    let mut want = pair.clone();
    want.sort();
    assert_eq!(ids, want, "the partner is pulled in");
    for item in &items {
        assert_eq!(item.item_type, "price_row");
        assert_eq!(item.created_by, s.f.ctx.subject_id());
        for lock_or_version in ["state", "version", "pending_unit_id", "approved_by_unit_id"] {
            assert!(
                item.after.get(lock_or_version).is_none(),
                "{lock_or_version}"
            );
        }
    }
    let promo_item = items.iter().find(|i| i.item_id == promo_id).unwrap();
    assert_eq!(promo_item.after["price"], json!({"rate":"0.10"}));
    assert_eq!(promo_item.after["paired_row_id"], pair[1].to_string());
    assert_eq!(
        promo_item.before.as_ref().unwrap()["row_id"],
        back.to_string(),
        "before is the chain's predecessor on the new start"
    );
}

#[tokio::test]
async fn validate_submit_reruns_the_rules_the_pair_rule_and_book_ownership() {
    let s = setup(0).await;
    s.approved(1, "2031-01-01", "per_unit", json!({"rate":"0.20"}))
        .await;
    let single = s.draft("single", body("2031-05-01")).await;
    let mut promo = body("2031-03-01");
    promo["temporary_until"] = json!("2031-03-11");
    let pair = s.draft("pair", promo).await;
    // Only the promo half, bypassing collect's pull-in.
    let subject = s.subject();
    let promo_only = pair[0];
    let err = s
        .tx(move |tx, _| {
            let subject = subject.clone();
            Box::pin(async move {
                let items: Vec<_> = subject
                    .collect(tx, &[promo_only])
                    .await?
                    .into_iter()
                    .filter(|i| i.item_id == promo_only)
                    .collect();
                Ok(subject.validate_submit(tx, &items).await?)
            })
        })
        .await
        .unwrap_err();
    assert_eq!(code(&err), "PAIR_SPLIT");
    // Another unit approved the same start meanwhile.
    s.approved(9, "2031-05-01", "per_unit", json!({"rate":"0.30"}))
        .await;
    let err = s.submit(s.subject(), single.clone(), 1).await.unwrap_err();
    assert_eq!(code(&err), "WINDOW_OVERLAP");
    let mut foreign = s.subject();
    foreign.book_id = Uuid::new_v4();
    let err = s.submit(foreign, pair, 1).await.unwrap_err();
    assert_eq!(code(&err), "ROW_NOT_IN_BOOK");
    assert_eq!(s.row(single[0]).await.state, "draft", "no unit, no lock");
}

#[tokio::test]
async fn the_chain_guard_reads_sku_metering_as_of_each_start() {
    let s = setup(0).await;
    s.script.versions.lock().unwrap().extend([
        (day("2030-01-01"), Some("GB".into()), Some("storage".into())),
        (day("2031-06-01"), Some("TB".into()), Some("storage".into())),
    ]);
    s.approved(1, "2031-01-01", "per_unit", json!({"rate":"0.20"}))
        .await;
    let early = s.draft("early", body("2031-03-01")).await;
    let late = s.draft("late", body("2031-07-01")).await;
    let err = s.submit(s.subject(), late, 1).await.unwrap_err();
    assert_eq!(
        code(&err),
        "CHAIN_MODEL_CHANGED",
        "the unit differs as of each start"
    );
    assert!(s.submit(s.subject(), early.clone(), 1).await.is_ok());
    let mut graduated = body("2031-04-01");
    graduated["model"] = json!("graduated");
    graduated["price"] = json!({"tiers":[{"up_to":null,"rate":"1"}]});
    let changed = s.draft("graduated", graduated).await;
    let err = s.submit(s.subject(), changed, 1).await.unwrap_err();
    assert_eq!(code(&err), "CHAIN_MODEL_CHANGED", "the model kind is kept");
    s.script
        .versions_down
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let again = s.draft("again", body("2031-02-01")).await;
    let err = s.submit(s.subject(), again, 1).await.unwrap_err();
    assert_eq!(code(&err), "REGISTRY_UNAVAILABLE");
}

#[tokio::test]
async fn a_recurring_chain_is_not_guarded_and_reads_no_metering() {
    let s = setup(11).await;
    s.approved(1, "2031-01-01", "flat", json!({"amount":"10"}))
        .await;
    let mut per_seat = body("2031-03-01");
    per_seat["price"] = json!({"rate":"2"});
    let ids = s.draft("seat", per_seat).await;
    assert!(s.submit(s.subject(), ids, 1).await.is_ok());
    assert_eq!(Script::count(&s.script.version_reads), 0);
}

#[tokio::test]
async fn lock_is_a_conditional_write_and_a_second_unit_is_refused() {
    let s = setup(0).await;
    let ids = s.draft("one", body("2031-03-01")).await;
    let first = s.submit(s.subject(), ids.clone(), 1).await.unwrap();
    let locked = s.row(ids[0]).await;
    assert_eq!(locked.state, "pending");
    assert_eq!(locked.pending_unit_id, Some(first.unit.id));
    let err = s.submit(s.subject(), ids.clone(), 1).await.unwrap_err();
    assert_eq!(code(&err), "ROW_NOT_DRAFT");
    // A racing unit that validated before the first lock loses the conditional write.
    let subject = s.subject();
    let tenant = s.tenant();
    let row = ids[0];
    let err = s
        .tx(move |tx, store| {
            let subject = subject.clone();
            Box::pin(async move {
                let mut unit = first_unit_like(tenant);
                unit.id = Uuid::new_v4();
                bss_approval::Store::insert_unit(&store, tx, &unit, &[]).await?;
                let items = vec![bss_approval::ItemRef {
                    item_type: "price_row".into(),
                    item_id: row,
                    created_by: Uuid::new_v4(),
                    before: None,
                    after: json!({}),
                }];
                Ok(subject.lock(tx, unit.id, &items).await?)
            })
        })
        .await
        .unwrap_err();
    assert_eq!(code(&err), "ROW_LOCKED_PENDING");
}
fn first_unit_like(tenant: Uuid) -> Unit {
    Unit {
        id: Uuid::new_v4(),
        tenant_id: tenant,
        kind: "price_rows".into(),
        ref_type: "price_book".into(),
        ref_id: Uuid::new_v4(),
        state: bss_approval::UnitState::Pending,
        common_effective_date: None,
        quorum_required: 1,
        generation: 1,
        submitted_by: Uuid::new_v4(),
        submitted_at: time::OffsetDateTime::now_utc(),
        decided_at: None,
        decided_note: None,
        snapshot: json!({}),
        snapshot_hash: "h".into(),
        version: 1,
    }
}

#[tokio::test]
async fn apply_normalizes_the_chain_and_marks_the_new_rows_predecessor_keep_for_bound() {
    let s = setup(0).await;
    let old = s
        .approved(1, "2031-01-01", "per_unit", json!({"rate":"0.20"}))
        .await;
    let mut signup = body("2031-06-01");
    signup["eligibility"] = json!("new");
    let ids = s.draft("new", signup).await;
    let mut subject = s.subject();
    subject.common_effective_date = Some(day("2031-05-01"));
    let submitted = s.submit(subject, ids.clone(), 0).await.unwrap();
    assert!(submitted.applied, "quorum 0 applies at submit");
    let row = s.row(ids[0]).await;
    assert_eq!(row.state, "approved");
    assert_eq!(
        row.effective_from,
        day("2031-05-01"),
        "the common date moved it"
    );
    assert_eq!(row.approved_by_unit_id, Some(submitted.unit.id));
    assert!(row.pending_unit_id.is_none());
    assert!(row.approved_at.is_some());
    let old = s.row(old).await;
    assert_eq!(old.effective_to, Some(day("2031-05-01")));
    assert!(
        old.keep_for_bound,
        "a pinned subscription may still be rated on it"
    );
}

#[tokio::test]
async fn apply_shifts_a_pair_by_one_delta() {
    let s = setup(0).await;
    let base = s
        .approved(1, "2031-01-01", "per_unit", json!({"rate":"0.20"}))
        .await;
    let mut promo = body("2031-03-01");
    promo["temporary_until"] = json!("2031-03-11");
    let pair = s.draft("pair", promo).await;
    let mut subject = s.subject();
    subject.common_effective_date = Some(day("2031-04-01"));
    s.submit(subject, vec![pair[0]], 0).await.unwrap();
    let (p, r) = (s.row(pair[0]).await, s.row(pair[1]).await);
    assert_eq!(
        (p.effective_from, p.effective_to),
        (day("2031-04-01"), Some(day("2031-04-11")))
    );
    assert_eq!(p.temporary_until, Some(day("2031-04-11")));
    assert_eq!(
        (r.effective_from, r.effective_to),
        (day("2031-04-11"), None)
    );
    assert_eq!(s.row(base).await.effective_to, Some(day("2031-04-01")));
    assert!(
        !s.row(base).await.keep_for_bound,
        "eligibility all binds forward"
    );
}

#[tokio::test]
async fn approving_a_later_default_row_keeps_a_closed_value_row() {
    let s = setup_with(0, true).await;
    s.approved(1, "2031-01-01", "per_unit", json!({"rate":"0.20"}))
        .await;
    let mut eu = body("2031-03-01");
    eu["dim_value"] = json!("eu");
    eu["temporary_until"] = json!("2031-03-11");
    let closed = s.draft("eu", eu).await;
    assert_eq!(closed.len(), 1);
    s.submit(s.subject(), closed.clone(), 0).await.unwrap();
    let later = s.draft("default", body("2031-03-05")).await;
    s.submit(s.subject(), later, 0).await.unwrap();
    let eu = s.row(closed[0]).await;
    assert_eq!(eu.state, "approved");
    assert!(eu.closed_explicitly);
    assert_eq!(
        eu.effective_to,
        Some(day("2031-03-11")),
        "another chain's approval never reopens a closed value row"
    );
}

#[tokio::test]
async fn apply_refuses_the_whole_unit_when_the_environment_changed() {
    let s = setup(0).await;
    s.script.versions.lock().unwrap().push((
        day("2030-01-01"),
        Some("GB".into()),
        Some("storage".into()),
    ));
    s.approved(1, "2031-01-01", "per_unit", json!({"rate":"0.20"}))
        .await;
    let first = s.draft("first", body("2031-05-01")).await;
    let second = s.draft("second", body("2031-08-01")).await;
    let unit = s
        .submit(s.subject(), vec![first[0], second[0]], 1)
        .await
        .unwrap()
        .unit;
    // Another unit approved one of the starts meanwhile.
    s.approved(9, "2031-08-01", "per_unit", json!({"rate":"0.30"}))
        .await;
    let err = s.approve(s.subject(), &unit).await.unwrap_err();
    assert_eq!(code(&err), "APPLY_REFUSED");
    assert!(format!("{err:?}").contains("WINDOW_OVERLAP"), "{err:?}");
    for id in [first[0], second[0]] {
        assert_eq!(
            s.row(id).await.state,
            "pending",
            "the whole unit rolled back"
        );
    }
    // The metering changed under the pending unit.
    let s = setup(0).await;
    s.script.versions.lock().unwrap().push((
        day("2030-01-01"),
        Some("GB".into()),
        Some("storage".into()),
    ));
    s.approved(1, "2031-01-01", "per_unit", json!({"rate":"0.20"}))
        .await;
    let ids = s.draft("x", body("2031-09-01")).await;
    let unit = s.submit(s.subject(), ids.clone(), 1).await.unwrap().unit;
    s.script.versions.lock().unwrap().push((
        day("2031-08-01"),
        Some("TB".into()),
        Some("storage".into()),
    ));
    let err = s.approve(s.subject(), &unit).await.unwrap_err();
    assert!(
        format!("{err:?}").contains("CHAIN_MODEL_CHANGED"),
        "{err:?}"
    );
    assert_eq!(s.row(ids[0]).await.state, "pending");
}

#[tokio::test]
async fn rejected_rows_stay_rejected_and_withdrawn_rows_return_to_draft() {
    let s = setup(0).await;
    let ids = s.draft("a", body("2031-03-01")).await;
    let unit = s.submit(s.subject(), ids.clone(), 1).await.unwrap().unit;
    let mut subject = s.subject();
    subject.release = Release::Rejected;
    let (id, generation) = (unit.id, unit.generation);
    s.tx(move |tx, store| {
        let subject = subject.clone();
        Box::pin(async move {
            Engine::reject(
                &store,
                &subject,
                tx,
                id,
                Uuid::new_v4(),
                generation,
                "too cheap",
                time::OffsetDateTime::now_utc(),
            )
            .await?;
            Ok(())
        })
    })
    .await
    .unwrap();
    let rejected = s.row(ids[0]).await;
    assert_eq!(rejected.state, "rejected");
    assert!(rejected.pending_unit_id.is_none());
    let other = s.draft("b", body("2031-04-01")).await;
    let unit = s.submit(s.subject(), other.clone(), 1).await.unwrap().unit;
    let subject = s.subject();
    let (id, actor) = (unit.id, s.f.ctx.subject_id());
    s.tx(move |tx, store| {
        let subject = subject.clone();
        Box::pin(async move {
            Engine::withdraw(
                &store,
                &subject,
                tx,
                id,
                actor,
                time::OffsetDateTime::now_utc(),
            )
            .await?;
            Ok(())
        })
    })
    .await
    .unwrap();
    assert_eq!(s.row(other[0]).await.state, "draft");
}
