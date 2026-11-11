// Created: 2026-11-11 by Constructor Tech
#![allow(clippy::expect_used, clippy::doc_markdown)]
//! Membership tenant isolation tests (issue 5585).
//!
//! The membership table has no scope columns, so the caller's tenant scope is
//! applied through the owning group. These tests run the `MembershipService`
//! with a PDP that returns `In(OWNER_TENANT_ID, [subject tenant])` and check
//! that a caller from tenant B cannot list, add into or remove from groups of
//! tenant A, and that a foreign group looks exactly like a missing one.

mod common;

use std::sync::Arc;

use common::{
    create_root_group, create_root_type, make_ctx, make_group_service, make_membership_service,
    make_membership_service_deny, make_membership_service_tenant_scoped, make_type_service,
    test_db,
};
use resource_group::domain::error::DomainError;
use resource_group::domain::membership_service::MembershipService;
use resource_group::infra::storage::group_repo::GroupRepository;
use resource_group::infra::storage::membership_repo::MembershipRepository;
use resource_group::infra::storage::type_repo::TypeRepository;
use resource_group_sdk::CreateTypeRequest;
use toolkit_db::{DBProvider, DbError};
use toolkit_gts::GTS_ID_PREFIX;
use toolkit_odata::ODataQuery;
use uuid::Uuid;

type MbrSvc = MembershipService<GroupRepository, TypeRepository, MembershipRepository>;

struct Fixture {
    db: Arc<DBProvider<DbError>>,
    member_type: String,
    tenant_a: Uuid,
    tenant_b: Uuid,
    group_a: Uuid,
    group_b: Uuid,
    /// Tenant-scoping service (the one under test).
    scoped: MbrSvc,
    /// Allow-all service used for seeding and for unscoped verification.
    seed: MbrSvc,
}

/// Two tenants, one group each, one seeded membership in each group.
async fn setup() -> Fixture {
    let db = test_db().await;
    let type_svc = make_type_service(db.clone());
    let group_svc = make_group_service(db.clone());
    let seed = make_membership_service(db.clone());
    let scoped = make_membership_service_tenant_scoped(db.clone());

    let member_type = create_root_type(&type_svc, "isombr").await;
    let grp_code = format!(
        "{GTS_ID_PREFIX}cf.core.rg.type.v1~x.test.isogrp{}.v1~",
        Uuid::now_v7().as_simple()
    );
    type_svc
        .create_type_unscoped(CreateTypeRequest {
            code: grp_code.clone(),
            can_be_root: true,
            allowed_parent_types: vec![],
            allowed_membership_types: vec![member_type.code.clone()],
            metadata_schema: None,
        })
        .await
        .expect("create group type");

    let tenant_a = Uuid::now_v7();
    let tenant_b = Uuid::now_v7();
    let group_a = create_root_group(&group_svc, &make_ctx(tenant_a), &grp_code, "GA", tenant_a)
        .await
        .id;
    let group_b = create_root_group(&group_svc, &make_ctx(tenant_b), &grp_code, "GB", tenant_b)
        .await
        .id;

    seed.add_membership_unscoped(group_a, &member_type.code, "res-a")
        .await
        .expect("seed membership in A");
    seed.add_membership_unscoped(group_b, &member_type.code, "res-b")
        .await
        .expect("seed membership in B");

    Fixture {
        db,
        member_type: member_type.code,
        tenant_a,
        tenant_b,
        group_a,
        group_b,
        scoped,
        seed,
    }
}

async fn list_all_unscoped(f: &Fixture) -> Vec<(Uuid, String)> {
    let page = f
        .seed
        .list_memberships_unscoped(&ODataQuery::default())
        .await
        .expect("unscoped list");
    page.items
        .into_iter()
        .map(|m| (m.group_id, m.resource_id))
        .collect()
}

#[tokio::test]
async fn list_is_limited_to_callers_tenant() {
    let f = setup().await;

    let page_b = f
        .scoped
        .list_memberships(&make_ctx(f.tenant_b), &ODataQuery::default())
        .await
        .expect("list as B");
    assert_eq!(page_b.items.len(), 1);
    assert_eq!(page_b.items[0].group_id, f.group_b);
    assert_eq!(page_b.items[0].resource_id, "res-b");

    let page_a = f
        .scoped
        .list_memberships(&make_ctx(f.tenant_a), &ODataQuery::default())
        .await
        .expect("list as A");
    assert_eq!(page_a.items.len(), 1);
    assert_eq!(page_a.items[0].group_id, f.group_a);
    assert_eq!(page_a.items[0].resource_id, "res-a");

    let all = list_all_unscoped(&f).await;
    assert_eq!(all.len(), 2, "unscoped list still sees both tenants");
}

#[tokio::test]
async fn list_with_filter_on_foreign_group_is_empty() {
    let f = setup().await;

    let filter = toolkit_odata::parse_filter_string(&format!("group_id eq {}", f.group_a))
        .expect("parse filter");
    let query = ODataQuery::new().with_filter(filter.into_expr());

    let page = f
        .scoped
        .list_memberships(&make_ctx(f.tenant_b), &query)
        .await
        .expect("filtered list as B");
    assert!(page.items.is_empty(), "foreign group must yield no rows");

    let page = f
        .scoped
        .list_memberships(&make_ctx(f.tenant_a), &query)
        .await
        .expect("filtered list as A");
    assert_eq!(page.items.len(), 1, "owner still sees its own row");
}

#[tokio::test]
async fn list_with_page_limit_stays_within_tenant() {
    let f = setup().await;
    for i in 0..3 {
        f.scoped
            .add_membership(
                &make_ctx(f.tenant_b),
                f.group_b,
                &f.member_type,
                &format!("extra-{i}"),
            )
            .await
            .expect("add own membership");
    }
    let first = f
        .scoped
        .list_memberships(&make_ctx(f.tenant_b), &ODataQuery::default().with_limit(2))
        .await
        .expect("first page");
    assert_eq!(first.items.len(), 2);
    assert!(first.items.iter().all(|m| m.group_id == f.group_b));
    assert!(
        first.page_info.next_cursor.is_some(),
        "tenant B has more rows than the page limit"
    );
}

#[tokio::test]
async fn add_into_foreign_group_looks_like_missing_group() {
    let f = setup().await;
    let ctx_b = make_ctx(f.tenant_b);

    let foreign = f
        .scoped
        .add_membership(&ctx_b, f.group_a, &f.member_type, "intruder")
        .await
        .expect_err("add into foreign group must fail");
    let missing_id = Uuid::now_v7();
    let missing = f
        .scoped
        .add_membership(&ctx_b, missing_id, &f.member_type, "intruder")
        .await
        .expect_err("add into missing group must fail");

    assert!(
        matches!(foreign, DomainError::GroupNotFound { id } if id == f.group_a),
        "got: {foreign:?}"
    );
    assert!(
        matches!(missing, DomainError::GroupNotFound { id } if id == missing_id),
        "got: {missing:?}"
    );
    // Same message shape once the requested id is masked out.
    let foreign_msg = foreign.to_string().replace(&f.group_a.to_string(), "<id>");
    let missing_msg = missing.to_string().replace(&missing_id.to_string(), "<id>");
    assert_eq!(foreign_msg, missing_msg);

    let all = list_all_unscoped(&f).await;
    assert!(
        !all.iter().any(|(_, r)| r == "intruder"),
        "no row may be inserted"
    );
}

#[tokio::test]
async fn remove_foreign_membership_looks_like_missing_membership() {
    let f = setup().await;
    let ctx_b = make_ctx(f.tenant_b);

    let foreign = f
        .scoped
        .remove_membership(&ctx_b, f.group_a, &f.member_type, "res-a")
        .await
        .expect_err("remove foreign membership must fail");
    let missing = f
        .scoped
        .remove_membership(&ctx_b, f.group_b, &f.member_type, "no-such-resource")
        .await
        .expect_err("remove missing membership must fail");

    assert!(
        matches!(foreign, DomainError::MembershipNotFound { .. }),
        "got: {foreign:?}"
    );
    assert!(
        matches!(missing, DomainError::MembershipNotFound { .. }),
        "got: {missing:?}"
    );

    let all = list_all_unscoped(&f).await;
    assert!(
        all.contains(&(f.group_a, "res-a".to_owned())),
        "A's membership must survive"
    );
}

#[tokio::test]
async fn owner_can_add_list_and_remove_in_own_group() {
    let f = setup().await;

    for (tenant, group) in [(f.tenant_a, f.group_a), (f.tenant_b, f.group_b)] {
        let ctx = make_ctx(tenant);
        let res_id = format!("own-{tenant}");
        f.scoped
            .add_membership(&ctx, group, &f.member_type, &res_id)
            .await
            .expect("add into own group");

        let page = f
            .scoped
            .list_memberships(&ctx, &ODataQuery::default())
            .await
            .expect("list own");
        assert!(
            page.items
                .iter()
                .any(|m| m.group_id == group && m.resource_id == res_id)
        );
        assert!(page.items.iter().all(|m| m.group_id == group));

        f.scoped
            .remove_membership(&ctx, group, &f.member_type, &res_id)
            .await
            .expect("remove from own group");
        let all = list_all_unscoped(&f).await;
        assert!(!all.contains(&(group, res_id)));
    }
}

#[tokio::test]
async fn deny_all_pdp_rejects_list_add_and_remove() {
    let f = setup().await;
    let deny = make_membership_service_deny(f.db.clone());
    let ctx = make_ctx(f.tenant_a);

    let err = deny
        .list_memberships(&ctx, &ODataQuery::default())
        .await
        .expect_err("list must be denied");
    assert!(
        matches!(err, DomainError::AccessDenied { .. }),
        "got: {err:?}"
    );

    let err = deny
        .add_membership(&ctx, f.group_a, &f.member_type, "denied")
        .await
        .expect_err("add must be denied");
    assert!(
        matches!(err, DomainError::AccessDenied { .. }),
        "got: {err:?}"
    );

    let err = deny
        .remove_membership(&ctx, f.group_a, &f.member_type, "res-a")
        .await
        .expect_err("remove must be denied");
    assert!(
        matches!(err, DomainError::AccessDenied { .. }),
        "got: {err:?}"
    );

    assert_eq!(list_all_unscoped(&f).await.len(), 2);
}

#[tokio::test]
async fn add_unscoped_works_for_any_tenants_group() {
    let f = setup().await;
    f.scoped
        .add_membership_unscoped(f.group_a, &f.member_type, "seed-a")
        .await
        .expect("unscoped add into A");
    f.scoped
        .add_membership_unscoped(f.group_b, &f.member_type, "seed-b")
        .await
        .expect("unscoped add into B");
    let all = list_all_unscoped(&f).await;
    assert!(all.contains(&(f.group_a, "seed-a".to_owned())));
    assert!(all.contains(&(f.group_b, "seed-b".to_owned())));
}
