//! The new SQL projection and typed seek keys on `PostgreSQL`, not only `SQLite`.

use super::{
    ACTOR, AccessScope, DBProvider, DbError, NewPlanDraft, OffsetDateTime, Pg, PlanId, PlanRepo,
    TENANT, Uuid, at, scope, stamp,
};
use bss_pricing::domain::plan::PlanShapePatch;
use toolkit_odata::{CursorV1, ODataOrderBy, ODataQuery, OrderKey, SortDir, parse_filter_string};

/// One sort with an explicit one-row page; the repository appends `plan_id`.
fn query(field: &str, dir: SortDir) -> ODataQuery {
    ODataQuery::new()
        .with_limit(1)
        .with_order(ODataOrderBy(vec![OrderKey {
            field: field.to_owned(),
            dir,
        }]))
}

/// Collect every page and prove it terminates without duplicate/missing rows.
async fn walk(plans: &PlanRepo, initial: ODataQuery) -> Vec<Uuid> {
    let mut request = initial;
    let mut ids = Vec::new();
    for _ in 0..10 {
        let page = plans
            .list_authoring_odata(&scope(), TENANT, &request)
            .await
            .expect("PG projection");
        ids.extend(page.items.iter().map(|entry| entry.revision.plan_id.get()));
        let Some(cursor) = page.page_info.next_cursor else {
            return ids;
        };
        request = ODataQuery::new()
            .with_limit(1)
            .with_cursor(CursorV1::decode(&cursor).unwrap());
    }
    unreachable!("seek did not advance")
}

#[tokio::test]
#[ignore = "requires Postgres; run with --ignored"]
async fn multi_page_order_and_canonical_revision_filters_on_postgres() {
    let pg = Pg::applied().await;
    let provider = DBProvider::<DbError>::new(pg.db().await);
    let plans = PlanRepo::new(provider.clone());
    let ids: Vec<_> = (1..=4).map(Uuid::from_u128).collect();
    for (index, id) in ids.iter().enumerate() {
        let plan = PlanId::new(*id);
        // Named here rather than left to the seeder's default: this case
        // sorts by the name, and since D-382 there is no NULL half for the
        // sort to place — what is left is the **tie**, which is what a cursor
        // loses when the order is not total.
        super::seed_published_plan_named(&provider, plan, if index < 2 { "Seed" } else { "Zeta" })
            .await;
        if index < 2 {
            let draft = plans
                .open_revision(&scope(), TENANT, plan, stamp())
                .await
                .unwrap();
            plans
                .update_draft(
                    &scope(),
                    TENANT,
                    plan,
                    draft.revision,
                    draft.row_version,
                    PlanShapePatch {
                        plan_name: Some(if index == 0 { "Beta" } else { "Alpha" }.to_owned()),
                        ..PlanShapePatch::default()
                    },
                    stamp(),
                )
                .await
                .unwrap();
        }
    }
    assert_eq!(
        walk(&plans, query("plan_name", SortDir::Asc)).await,
        vec![ids[1], ids[0], ids[2], ids[3]]
    );
    assert_eq!(
        walk(&plans, query("plan_name", SortDir::Desc)).await,
        vec![ids[2], ids[3], ids[0], ids[1]],
        "descending on the name; each tie broken by the walk's stable order"
    );
    assert_eq!(
        walk(&plans, query("price_row_count", SortDir::Desc)).await,
        ids
    );
    assert_eq!(walk(&plans, query("created_at", SortDir::Desc)).await, ids);
    let filtered = ODataQuery::new().with_filter(
        parse_filter_string("lifecycle_state eq 'published'")
            .unwrap()
            .into_expr(),
    );
    let page = plans
        .list_authoring_odata(&scope(), TENANT, &filtered)
        .await
        .unwrap();
    assert_eq!(
        page.items
            .iter()
            .map(|r| r.revision.plan_id.get())
            .collect::<Vec<_>>(),
        ids[2..]
    );
    for entry in plans
        .list_authoring_odata(&scope(), TENANT, &ODataQuery::new())
        .await
        .unwrap()
        .items
    {
        assert_eq!(entry.created_at, at(10));
        if entry.revision.revision == 1 {
            assert_eq!(entry.revision.created_at_utc, at(11));
        }
    }
}

/// Different original timestamps force timestamp comparison, not just UUID ties.
async fn draft_at(plans: &PlanRepo, id: Uuid, created: OffsetDateTime) {
    plans
        .create_draft(
            &scope(),
            NewPlanDraft {
                plan_name: "Fixture Plan".to_owned(),
                plan_id: PlanId::new(id),
                tenant_id: TENANT,
                created_by: ACTOR,
                created_at_utc: created,
                // D-372: `pricing_plan.sku_id` is `NOT NULL` since
                // the fresh-install DDL, so a draft with no SKU is refused
                // by the store. `PlanShape.sku_id` stays `Option` until Task 8.
                sku_id: Uuid::from_u128(5),
                plan_tier: None,
                frequency: None,
                purchase_min_qty: None,
                purchase_max_qty: None,
                descriptor_ext: std::collections::BTreeMap::new(),
                available_from: None,
                available_to: None,
                cloned_from: None,
                correlation_id: super::CORRELATION,
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres; run with --ignored"]
async fn creation_date_seek_and_resource_scope_on_postgres() {
    use toolkit_security::{ScopeConstraint, ScopeFilter, pep_properties};
    let pg = Pg::applied().await;
    let provider = DBProvider::<DbError>::new(pg.db().await);
    let plans = PlanRepo::new(provider);
    let ids: Vec<_> = (1..=3).map(Uuid::from_u128).collect();
    draft_at(&plans, ids[0], at(12)).await;
    draft_at(&plans, ids[1], at(10)).await;
    draft_at(&plans, ids[2], at(11)).await;
    assert_eq!(
        walk(&plans, query("created_at", SortDir::Asc)).await,
        vec![ids[1], ids[2], ids[0]]
    );
    assert_eq!(
        walk(&plans, query("created_at", SortDir::Desc)).await,
        vec![ids[0], ids[2], ids[1]]
    );
    let exact = ODataQuery::new().with_filter(
        parse_filter_string("created_at eq 2026-08-11T11:00:00Z")
            .unwrap()
            .into_expr(),
    );
    let page = plans
        .list_authoring_odata(&scope(), TENANT, &exact)
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].revision.plan_id.get(), ids[2]);
    let restricted = AccessScope::single(ScopeConstraint::new(vec![
        ScopeFilter::in_uuids(pep_properties::OWNER_TENANT_ID, vec![TENANT]),
        ScopeFilter::in_uuids(pep_properties::RESOURCE_ID, vec![ids[1]]),
    ]));
    let page = plans
        .list_authoring_odata(&restricted, TENANT, &query("created_at", SortDir::Desc))
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].revision.plan_id.get(), ids[1]);
    assert_eq!(page.items[0].created_at, at(10));
    assert!(page.page_info.next_cursor.is_none());
}

/// `charge_kind` membership runs as a correlated `EXISTS` over
/// `pricing_charge_line`, and that statement has to be one Postgres accepts —
/// `plan_id` is `uuid` there and `text` on the mirror, which is exactly the kind
/// of difference a suite that only ran `SQLite` would not see.
///
/// Three plans: one recurring line, one one-time line, no line. Both polarities,
/// and `ne` includes the plan that holds no line at all.
#[tokio::test]
#[ignore = "requires Postgres; run with --ignored"]
async fn charge_kind_membership_runs_on_postgres() {
    let pg = Pg::applied().await;
    let provider = DBProvider::<DbError>::new(pg.db().await);
    let plans = PlanRepo::new(provider.clone());
    let ids: Vec<_> = (1..=3).map(Uuid::from_u128).collect();
    for id in &ids {
        draft_at(&plans, *id, at(10)).await;
    }
    let conn = provider.conn().expect("conn");
    for (plan, kind) in [(ids[0], "recurring"), (ids[1], "one_time")] {
        super::common::seed_charge_graph(
            &conn,
            &scope(),
            &super::common::ChargeGraphSeed {
                tenant_id: TENANT,
                plan_id: plan,
                plan_revision: 0,
                phase: Uuid::from_u128(0xf1),
                sku_id: Uuid::from_u128(5),
                charge_kind: kind.to_owned(),
                lifecycle_state: "draft".to_owned(),
                model_kind: Some("flat".to_owned()),
                created_by: ACTOR,
                created_at_utc: at(10),
                ..Default::default()
            },
        )
        .await;
    }

    let matching = |filter: &'static str| {
        let plans = &plans;
        async move {
            let query =
                ODataQuery::new().with_filter(parse_filter_string(filter).unwrap().into_expr());
            let mut found: Vec<Uuid> = plans
                .list_authoring_odata(&scope(), TENANT, &query)
                .await
                .expect("the membership statement runs on Postgres")
                .items
                .iter()
                .map(|entry| entry.revision.plan_id.get())
                .collect();
            found.sort_unstable();
            found
        }
    };
    assert_eq!(matching("charge_kind eq 'recurring'").await, vec![ids[0]]);
    assert_eq!(matching("charge_kind eq 'one_time'").await, vec![ids[1]]);
    assert_eq!(
        matching("charge_kind ne 'recurring'").await,
        vec![ids[1], ids[2]],
        "`ne` is \"holds no line of this kind\", so the lineless plan matches"
    );
}
