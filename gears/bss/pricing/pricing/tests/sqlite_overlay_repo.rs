//! `OverlayRepo` against a real database — the revision discipline D-92 and
//! D-107 describe, driven through the typed path rather than through raw SQL.
//!
//! `sqlite_overlay_store` proves what the **schema** refuses; this suite proves
//! what the **repository** does with it: that a successor revision copies the
//! published one forward under the same line ids, that a draft edit cannot reach
//! published truth, that the publish flips both rows in one commit, and that the
//! compare-and-swap tells its three refusals apart.
//!
//! The two are deliberately separate. A repository that writes only valid values
//! catches a constraint that got *narrower* and never one that stopped refusing,
//! so the store's guarantees are asserted past this layer; and a schema suite
//! cannot see a repository that writes the right rows in the wrong order.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use bss_pricing::domain::audit::AuditStamp;
use bss_pricing::domain::money::CurrencyCode;
use bss_pricing::domain::overlay::{
    Adjustment, AmountSet, Disclosure, LineKey, Magnitude, OverlayInterval, OverlayLifecycle,
    OverlayLine, ScopeClass, ScopeSelector, ScopeValue, TargetRef, TaxBasis,
};
use bss_pricing::domain::scope_key::PlanId;
use bss_pricing::infra::storage::RepoError;
use bss_pricing::infra::storage::entity::{
    brand_taxonomy, plan, price_overlay, price_overlay_line,
};
use bss_pricing::infra::storage::migrations::Migrator;
use bss_pricing::infra::storage::repo::{NewOverlay, OverlayRepo};
use chrono::{TimeZone, Utc};
use sea_orm::ActiveValue::Set;
use sea_orm::EntityTrait;
use sea_orm::{ColumnTrait, Condition};
use sea_orm_migration::MigratorTrait;
use toolkit_db::migration_runner::run_migrations_for_testing;
use toolkit_db::secure::SecureDeleteExt;
use toolkit_db::secure::{AccessScope, SecureInsertExt};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

const TENANT: Uuid = Uuid::from_u128(0x1111_1111);
const OVERLAY: Uuid = Uuid::from_u128(0xAAAA_AAAA);
const LINE_A: Uuid = Uuid::from_u128(0xCCCC_0001);
const LINE_B: Uuid = Uuid::from_u128(0xCCCC_0002);

/// Declare the `acme` brand in one state.
///
/// Written through the entity rather than the repository because the taxonomy's
/// **authoring** surface is Slice 4's and is not built — this strand builds the
/// table and the read `inst-plv-scope` makes of it, and says so.
async fn declare_brand(provider: &DBProvider<DbError>, state: &str) {
    let conn = provider.conn().expect("a connection");
    let row = brand_taxonomy::ActiveModel {
        tenant_id: Set(TENANT),
        value: Set("acme".to_owned()),
        display_name: Set("Acme".to_owned()),
        state: Set(state.to_owned()),
    };
    brand_taxonomy::Entity::insert(row.clone())
        .secure()
        .scope_with_model(&AccessScope::allow_all(), &row)
        .expect("the scope admits the row")
        .exec(&conn)
        .await
        .expect("the brand is declared");
}

fn plan(n: u128) -> PlanId {
    PlanId::new(Uuid::from_u128(n))
}

fn stamp() -> AuditStamp {
    AuditStamp {
        actor_principal_id: Uuid::from_u128(0x4444),
        recorded_at: Utc
            .with_ymd_and_hms(2099, 1, 1, 0, 0, 0)
            .single()
            .expect("a valid instant"),
        correlation_id: Uuid::from_u128(0x5555),
    }
}

fn brand_scope() -> ScopeSelector {
    ScopeSelector::scoped(
        ScopeClass::Brand,
        ScopeValue::new("acme").expect("a non-blank value"),
    )
    .expect("brand is not global")
}

fn new_overlay(id: Uuid, precedence: i32) -> NewOverlay {
    NewOverlay {
        price_overlay_id: id,
        tenant_id: TENANT,
        scope: brand_scope(),
        precedence,
        interval: OverlayInterval::default(),
        tax_basis: TaxBasis::DelegatedTariffs,
        disclosure: Disclosure::Restricted,
        target_ref: TargetRef {
            plans: vec![plan(1), plan(2)],
        },
    }
}

fn percent_line(id: Uuid, key: LineKey, bp: i64) -> OverlayLine {
    OverlayLine {
        line_id: id,
        key,
        adjustment: Adjustment::Discount(Magnitude::PercentBp(bp)),
    }
}

fn fixed_line(id: Uuid, key: LineKey, minor: i64) -> OverlayLine {
    OverlayLine {
        line_id: id,
        key,
        adjustment: Adjustment::Fixed(AmountSet::new([(
            CurrencyCode::new("EUR").expect("a valid code"),
            minor,
        )])),
    }
}

/// A migrated in-memory database, as the provider the repository takes.
async fn provider() -> DBProvider<DbError> {
    let db = connect_db("sqlite::memory:", ConnectOpts::default())
        .await
        .expect("connect in-memory sqlite");
    run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("run migrator");
    DBProvider::<DbError>::new(db)
}

/// The repository over a migrated database, and the scope every call takes.
async fn repo() -> (OverlayRepo, AccessScope) {
    (OverlayRepo::new(provider().await), AccessScope::allow_all())
}

/// Discard one draft revision: its lines first, then the revision row — the
/// order the foreign key forces.
///
/// Takes the provider **as an argument**. It was briefly a process-wide static,
/// which is wrong for the same reason a shared scratchpad is: these cases run in
/// parallel, so one test read another's database and the failure looked like a
/// defect in the code under test rather than in the fixture.
async fn discard_revision(provider: &DBProvider<DbError>, overlay: Uuid, revision: i64) {
    let conn = provider.conn().expect("a connection");
    let scope = AccessScope::allow_all();
    price_overlay_line::Entity::delete_many()
        .secure()
        .scope_with(&scope)
        .filter(
            Condition::all()
                .add(price_overlay_line::Column::PriceOverlayId.eq(overlay))
                .add(price_overlay_line::Column::OverlayRevision.eq(revision)),
        )
        .exec(&conn)
        .await
        .expect("the draft's lines are discardable");
    price_overlay::Entity::delete_many()
        .secure()
        .scope_with(&scope)
        .filter(
            Condition::all()
                .add(price_overlay::Column::PriceOverlayId.eq(overlay))
                .add(price_overlay::Column::Revision.eq(revision)),
        )
        .exec(&conn)
        .await
        .expect("a draft revision is discardable");
}

/// The overlay with one per-plan line and one list-default line, at revision 0.
async fn seeded() -> (OverlayRepo, AccessScope) {
    let (repo, scope, _) = seeded_full().await;
    (repo, scope)
}

/// The same, handing back the provider for the cases that must reach past the
/// repository.
async fn seeded_full() -> (OverlayRepo, AccessScope, DBProvider<DbError>) {
    let provider = provider().await;
    let (repo, scope) = (OverlayRepo::new(provider.clone()), AccessScope::allow_all());
    repo.create(
        &scope,
        new_overlay(OVERLAY, 10),
        vec![
            percent_line(LINE_A, LineKey::for_plan(plan(1)), 1000),
            percent_line(LINE_B, LineKey::list_default(), 500),
        ],
        stamp(),
    )
    .await
    .expect("the overlay is created at revision 0");
    (repo, scope, provider)
}

// ---------------------------------------------------------------------------
// Create.
// ---------------------------------------------------------------------------

/// A created overlay is a **draft** at revision 0, carrying its whole line set.
#[tokio::test]
async fn a_created_overlay_is_a_draft_at_revision_zero() {
    let (repo, scope) = seeded().await;

    let record = repo
        .load(&scope, TENANT, OVERLAY, 0)
        .await
        .expect("the read succeeds")
        .expect("revision 0 exists");

    assert_eq!(record.lifecycle_state, OverlayLifecycle::Draft);
    assert_eq!(record.revision, 0);
    assert_eq!(record.precedence, 10);
    assert_eq!(record.scope, brand_scope());
    assert_eq!(record.tax_basis, TaxBasis::DelegatedTariffs);
    assert_eq!(
        record.disclosure,
        Disclosure::Restricted,
        "the fail-closed default must survive the round trip"
    );
    assert_eq!(record.target_ref.plans, vec![plan(1), plan(2)]);
    assert_eq!(record.lines.len(), 2);
    assert_eq!(record.row_version, 0);
}

/// An overlay with no published revision has no `current`.
#[tokio::test]
async fn a_draft_only_overlay_has_no_current_revision() {
    let (repo, scope) = seeded().await;
    assert!(
        repo.current(&scope, TENANT, OVERLAY)
            .await
            .expect("the read succeeds")
            .is_none()
    );
}

/// A `fixed` line's per-currency values round-trip through the amount table.
#[tokio::test]
async fn an_amount_lines_values_round_trip() {
    let (repo, scope) = repo().await;
    repo.create(
        &scope,
        new_overlay(OVERLAY, 10),
        vec![fixed_line(LINE_A, LineKey::for_plan(plan(1)), 5000)],
        stamp(),
    )
    .await
    .expect("created");

    let record = repo
        .load(&scope, TENANT, OVERLAY, 0)
        .await
        .expect("the read succeeds")
        .expect("revision 0 exists");
    let line = record.lines.first().expect("one line");
    let amounts = line.adjustment.amounts().expect("an amount line");
    assert_eq!(
        amounts.get(&CurrencyCode::new("EUR").expect("a valid code")),
        Some(5000)
    );
    assert!(
        line.adjustment.replaces(),
        "a `fixed` line replaces (D-138)"
    );
}

/// **The nil plan id is refused**, which is what keeps the store's line-key
/// sentinel unforgeable.
///
/// `uq_pricing_price_overlay_line_key` coalesces an absent plan to the nil uuid,
/// so a request naming it would key as the list-default line and collide with
/// it. Unlike the other two sentinels — a blank SKU and a `-infinity` cohort —
/// this one is spellable from the wire, so the repository is what closes it.
#[tokio::test]
async fn a_line_naming_the_nil_plan_id_is_refused() {
    let (repo, scope) = repo().await;

    let refusal = repo
        .create(
            &scope,
            new_overlay(OVERLAY, 10),
            vec![percent_line(
                LINE_A,
                LineKey::for_plan(PlanId::new(Uuid::nil())),
                1000,
            )],
            stamp(),
        )
        .await
        .expect_err("the nil plan id must be refused");
    assert!(
        matches!(refusal, RepoError::ValueOutOfRange { ref field, .. } if field == "plan_id"),
        "got {refusal:?}"
    );
}

// ---------------------------------------------------------------------------
// The revision discipline (D-92).
// ---------------------------------------------------------------------------

/// **D-92's copy-on-new-revision, with stable line identity.**
///
/// The successor carries the *same* `line_id`s and its own copies of the rows.
/// That is the clause §6's literal `PK line_id` cannot express — see
/// `m20260802_000033`.
#[tokio::test]
async fn opening_a_revision_copies_the_line_set_forward_under_the_same_ids() {
    let (repo, scope) = seeded().await;
    repo.publish_revision(&scope, TENANT, OVERLAY, 0, stamp())
        .await
        .expect("revision 0 publishes");

    let successor = repo
        .open_revision(&scope, TENANT, OVERLAY, stamp())
        .await
        .expect("a successor opens");
    assert_eq!(successor, 1);

    let draft = repo
        .load(&scope, TENANT, OVERLAY, 1)
        .await
        .expect("the read succeeds")
        .expect("revision 1 exists");
    assert_eq!(draft.lifecycle_state, OverlayLifecycle::Draft);

    let mut copied: Vec<Uuid> = draft.lines.iter().map(|l| l.line_id).collect();
    copied.sort_unstable();
    let mut expected = vec![LINE_A, LINE_B];
    expected.sort_unstable();
    assert_eq!(
        copied, expected,
        "a line keeps its id across the revision that did not change it"
    );
    assert_eq!(
        draft.precedence, 10,
        "the header is copied forward too, precedence and all"
    );
    assert_eq!(
        draft.row_version, 0,
        "the successor starts its own tag sequence"
    );
}

/// An overlay holds at most one open draft revision.
#[tokio::test]
async fn a_second_open_revision_is_refused() {
    let (repo, scope) = seeded().await;
    repo.publish_revision(&scope, TENANT, OVERLAY, 0, stamp())
        .await
        .expect("revision 0 publishes");
    repo.open_revision(&scope, TENANT, OVERLAY, stamp())
        .await
        .expect("the first successor opens");

    let refusal = repo
        .open_revision(&scope, TENANT, OVERLAY, stamp())
        .await
        .expect_err("a second open draft must be refused");
    assert!(
        matches!(refusal, RepoError::OverlayOpenDraftExists { ref revision, .. } if *revision == 1),
        "the refusal must name the revision holding the slot, got {refusal:?}"
    );
}

/// **The acceptance criterion, end to end: a draft edit cannot reach published
/// truth.**
///
/// The successor's line edits land on **its own copies**; the published revision
/// keeps serving unchanged, which is what a re-warm of its version resolves.
#[tokio::test]
async fn a_draft_edit_leaves_the_published_revision_untouched() {
    let (repo, scope) = seeded().await;
    repo.publish_revision(&scope, TENANT, OVERLAY, 0, stamp())
        .await
        .expect("revision 0 publishes");
    repo.open_revision(&scope, TENANT, OVERLAY, stamp())
        .await
        .expect("a successor opens");

    repo.replace_lines(
        &scope,
        TENANT,
        OVERLAY,
        1,
        0,
        vec![percent_line(LINE_A, LineKey::for_plan(plan(1)), 9000)],
        stamp(),
    )
    .await
    .expect("the draft's lines are replaced");

    let published = repo
        .load(&scope, TENANT, OVERLAY, 0)
        .await
        .expect("the read succeeds")
        .expect("revision 0 exists");
    assert_eq!(
        published.lines.len(),
        2,
        "the published revision keeps both of its lines"
    );
    let untouched = published
        .lines
        .iter()
        .find(|l| l.line_id == LINE_A)
        .expect("its per-plan line");
    assert_eq!(
        untouched.adjustment,
        Adjustment::Discount(Magnitude::PercentBp(1000)),
        "the published magnitude must not have moved"
    );

    let draft = repo
        .load(&scope, TENANT, OVERLAY, 1)
        .await
        .expect("the read succeeds")
        .expect("revision 1 exists");
    assert_eq!(draft.lines.len(), 1, "the draft holds the replacement set");
    assert_eq!(
        draft.lines[0].adjustment,
        Adjustment::Discount(Magnitude::PercentBp(9000))
    );
}

/// The line set is replaced **wholesale**, never merged — every D-42 rule is a
/// property of the set.
#[tokio::test]
async fn replacing_the_line_set_removes_the_lines_not_resubmitted() {
    let (repo, scope) = seeded().await;

    repo.replace_lines(
        &scope,
        TENANT,
        OVERLAY,
        0,
        0,
        vec![percent_line(LINE_B, LineKey::list_default(), 700)],
        stamp(),
    )
    .await
    .expect("the lines are replaced");

    let record = repo
        .load(&scope, TENANT, OVERLAY, 0)
        .await
        .expect("the read succeeds")
        .expect("revision 0 exists");
    assert_eq!(record.lines.len(), 1);
    assert_eq!(record.lines[0].line_id, LINE_B);
}

// ---------------------------------------------------------------------------
// The compare-and-swap, and its three refusals.
// ---------------------------------------------------------------------------

/// A stale row version is refused, and the refusal carries **both** versions —
/// the difference is the diagnosis.
#[tokio::test]
async fn a_stale_row_version_is_refused() {
    let (repo, scope) = seeded().await;
    repo.replace_lines(
        &scope,
        TENANT,
        OVERLAY,
        0,
        0,
        vec![percent_line(LINE_A, LineKey::for_plan(plan(1)), 1200)],
        stamp(),
    )
    .await
    .expect("the first edit lands and bumps the tag to 1");

    let refusal = repo
        .replace_lines(
            &scope,
            TENANT,
            OVERLAY,
            0,
            0,
            vec![percent_line(LINE_A, LineKey::for_plan(plan(1)), 1300)],
            stamp(),
        )
        .await
        .expect_err("a second edit against the stale tag must be refused");
    assert!(
        matches!(
            refusal,
            RepoError::StaleRowVersion {
                current: 1,
                submitted: 0,
                ..
            }
        ),
        "got {refusal:?}"
    );
}

/// An edit aimed at a **published** revision is refused as not-a-draft, which is
/// a different remedy from a stale tag: no refresh makes it editable.
#[tokio::test]
async fn an_edit_of_a_published_revision_is_refused_as_not_draft() {
    let (repo, scope) = seeded().await;
    repo.publish_revision(&scope, TENANT, OVERLAY, 0, stamp())
        .await
        .expect("revision 0 publishes");

    let refusal = repo
        .replace_lines(
            &scope,
            TENANT,
            OVERLAY,
            0,
            0,
            vec![percent_line(LINE_A, LineKey::for_plan(plan(1)), 1300)],
            stamp(),
        )
        .await
        .expect_err("a published revision is frozen");
    assert!(
        matches!(refusal, RepoError::NotDraft { ref state, .. } if state == "published"),
        "got {refusal:?}"
    );
}

/// An edit of a revision that does not exist is a 404 and not a conflict.
#[tokio::test]
async fn an_edit_of_an_absent_revision_is_not_found() {
    let (repo, scope) = seeded().await;

    let refusal = repo
        .replace_lines(&scope, TENANT, OVERLAY, 7, 0, Vec::new(), stamp())
        .await
        .expect_err("revision 7 does not exist");
    assert!(
        matches!(refusal, RepoError::NotFound { .. }),
        "got {refusal:?}"
    );
}

// ---------------------------------------------------------------------------
// The publish, in one commit.
// ---------------------------------------------------------------------------

/// **§6's atomic pair**: the successor publishes and its predecessor flips
/// `superseded` in the same commit.
#[tokio::test]
async fn publishing_a_successor_supersedes_its_predecessor_in_one_commit() {
    let (repo, scope) = seeded().await;
    repo.publish_revision(&scope, TENANT, OVERLAY, 0, stamp())
        .await
        .expect("revision 0 publishes");
    repo.open_revision(&scope, TENANT, OVERLAY, stamp())
        .await
        .expect("a successor opens");

    repo.publish_revision(&scope, TENANT, OVERLAY, 1, stamp())
        .await
        .expect("revision 1 publishes");

    let predecessor = repo
        .load(&scope, TENANT, OVERLAY, 0)
        .await
        .expect("the read succeeds")
        .expect("revision 0 exists");
    assert_eq!(predecessor.lifecycle_state, OverlayLifecycle::Superseded);

    let current = repo
        .current(&scope, TENANT, OVERLAY)
        .await
        .expect("the read succeeds")
        .expect("the overlay has a published revision");
    assert_eq!(current.revision, 1);
}

/// **D-107 through the typed path.** Opening and publishing a successor of a
/// live overlay reuses its own `(class, precedence)` and is not refused — the
/// case an unqualified index would make impossible.
#[tokio::test]
async fn a_successor_reuses_its_own_precedence_without_a_conflict() {
    let (repo, scope) = seeded().await;
    repo.publish_revision(&scope, TENANT, OVERLAY, 0, stamp())
        .await
        .expect("revision 0 publishes");
    repo.open_revision(&scope, TENANT, OVERLAY, stamp())
        .await
        .expect("a successor opens at the same precedence");

    repo.publish_revision(&scope, TENANT, OVERLAY, 1, stamp())
        .await
        .expect("and it publishes at the same precedence");
}

/// ...while a **different** overlay claiming the slot loses, on the index.
#[tokio::test]
async fn a_second_overlay_claiming_a_published_precedence_is_refused() {
    let (repo, scope) = seeded().await;
    repo.publish_revision(&scope, TENANT, OVERLAY, 0, stamp())
        .await
        .expect("revision 0 publishes at precedence 10");

    let rival = Uuid::from_u128(0xBBBB_BBBB);
    repo.create(
        &scope,
        new_overlay(rival, 10),
        vec![percent_line(
            Uuid::from_u128(0xCCCC_0009),
            LineKey::list_default(),
            100,
        )],
        stamp(),
    )
    .await
    .expect("a draft at the same precedence is legal: the index is partial");

    let refusal = repo
        .publish_revision(&scope, TENANT, rival, 0, stamp())
        .await
        .expect_err("two published overlays may not share the slot");
    assert!(
        matches!(refusal, RepoError::OverlayPrecedenceHeld),
        "got {refusal:?}"
    );
}

// ---------------------------------------------------------------------------
// The taxonomy lookup (`inst-plv-scope`).
// ---------------------------------------------------------------------------

/// A declared, **active** value validates; an undeclared one does not.
#[tokio::test]
async fn the_taxonomy_lookup_answers_for_a_declared_value() {
    let provider = provider().await;
    declare_brand(&provider, "active").await;
    let repo = OverlayRepo::new(provider);
    let scope = AccessScope::allow_all();

    assert!(
        repo.taxonomy_declares(&scope, TENANT, &brand_scope())
            .await
            .expect("the lookup succeeds")
    );

    let unknown = ScopeSelector::scoped(
        ScopeClass::Brand,
        ScopeValue::new("nobody").expect("a non-blank value"),
    )
    .expect("brand is not global");
    assert!(
        !repo
            .taxonomy_declares(&scope, TENANT, &unknown)
            .await
            .expect("the lookup succeeds")
    );
}

/// A **retired** value declares nothing — a value that reached `retired` must not
/// validate a new overlay against itself.
#[tokio::test]
async fn a_retired_taxonomy_value_declares_nothing() {
    let provider = provider().await;
    declare_brand(&provider, "retired").await;
    let repo = OverlayRepo::new(provider);

    assert!(
        !repo
            .taxonomy_declares(&AccessScope::allow_all(), TENANT, &brand_scope())
            .await
            .expect("the lookup succeeds")
    );
}

/// The classless scope consults nothing and always answers `true` — it has no
/// value, so there is nothing to be undeclared.
#[tokio::test]
async fn the_classless_scope_consults_no_taxonomy() {
    let (repo, scope) = repo().await;
    assert!(
        repo.taxonomy_declares(&scope, TENANT, &ScopeSelector::Global)
            .await
            .expect("the lookup succeeds")
    );
}

/// **`customer_group` answers `false`, and that is fail-closed rather than a
/// gap.**
///
/// `pricing_customer_group_taxonomy` belongs to the membership half, which this
/// strand does not build. Answering `true` would let a `customerGroup` overlay
/// publish against a universe that does not exist — exactly the D-211 shape the
/// four taxonomy tables were built to avoid.
#[tokio::test]
async fn the_customer_group_class_has_no_universe_and_is_refused() {
    let (repo, scope) = repo().await;
    let selector = ScopeSelector::scoped(
        ScopeClass::CustomerGroup,
        ScopeValue::new("vip").expect("a non-blank value"),
    )
    .expect("customer_group is not global");

    assert!(
        !repo
            .taxonomy_declares(&scope, TENANT, &selector)
            .await
            .expect("the lookup succeeds"),
        "the class must fail closed while its taxonomy does not exist"
    );
}

// ---------------------------------------------------------------------------
// The list read.
// ---------------------------------------------------------------------------

/// The list is the admin/Tariffs read: it returns **every** revision, and it
/// does **not** filter on `disclosure`.
///
/// L6 governs consumer-facing exposure and §3 step 7 is explicit that operator
/// and service reads are unaffected. A `restricted` overlay hidden from this
/// surface would hide it from the operator who authored it.
#[tokio::test]
async fn the_list_returns_every_revision_and_hides_no_restricted_overlay() {
    let (repo, scope) = seeded().await;
    repo.publish_revision(&scope, TENANT, OVERLAY, 0, stamp())
        .await
        .expect("revision 0 publishes");
    repo.open_revision(&scope, TENANT, OVERLAY, stamp())
        .await
        .expect("a successor opens");

    let all = repo
        .list(&scope, TENANT, None)
        .await
        .expect("the list succeeds");
    assert_eq!(all.len(), 2, "both revisions are listed");
    assert!(
        all.iter().all(|o| o.disclosure == Disclosure::Restricted),
        "a restricted overlay is still an operator's to read"
    );

    let narrowed = repo
        .list(&scope, TENANT, Some(ScopeClass::Brand))
        .await
        .expect("the list succeeds");
    assert_eq!(narrowed.len(), 2);
    let none = repo
        .list(&scope, TENANT, Some(ScopeClass::Partner))
        .await
        .expect("the list succeeds");
    assert!(
        none.is_empty(),
        "a class nothing is authored under is empty"
    );
}

// ---------------------------------------------------------------------------
// `world_for` — the seam `PriceOverlayValidator` is judged against.
// ---------------------------------------------------------------------------

/// Seed one plan revision in a named lifecycle state.
///
/// Inserted directly rather than published through `PlanRepo`, because
/// `pricing_plan`'s append-only trigger guards UPDATE and DELETE and leaves
/// INSERT free — so this reaches the exact state the world assembly has to read,
/// which is the point of driving it from here.
async fn seed_plan(provider: &DBProvider<DbError>, plan_id: PlanId, revision: i64, state: &str) {
    let conn = provider.conn().expect("a connection");
    let row = plan::ActiveModel {
        plan_id: Set(plan_id.get()),
        revision: Set(revision),
        tenant_id: Set(TENANT),
        lifecycle_state: Set(state.to_owned()),
        created_by: Set(Uuid::from_u128(0x4444)),
        created_at_utc: Set(Utc
            .with_ymd_and_hms(2099, 1, 1, 0, 0, 0)
            .single()
            .expect("a valid instant")),
        ..Default::default()
    };
    plan::Entity::insert(row.clone())
        .secure()
        .scope_with_model(&AccessScope::allow_all(), &row)
        .expect("the scope admits the row")
        .exec(&conn)
        .await
        .expect("the plan revision is seeded");
}

/// **D-31 through the seam: a retired target is still a published target.**
///
/// `uq_pricing_plan_current` is `UNIQUE (plan_id) WHERE lifecycle_state IN
/// ('published','retired')`, so retirement flips the plan's **one current**
/// revision in place — a retired plan has no `published` row left. Reading that
/// as "not published" makes `TARGET_UNPUBLISHED` block every overlay on it,
/// which is the rule D-31 forbids, and it blocks the remediation too: ending or
/// retargeting the overlay is itself a submit.
///
/// This is the case that was missing. The domain test hand-built a world with the
/// plan in **both** sets — a state the schema cannot produce — so it stayed green
/// while `plan_facts` put a retired plan in neither.
#[tokio::test]
async fn a_retired_target_is_still_a_published_target_and_is_flagged() {
    let provider = provider().await;
    seed_plan(&provider, plan(1), 0, "retired").await;
    let repo = OverlayRepo::new(provider);
    let scope = AccessScope::allow_all();

    repo.create(
        &scope,
        new_overlay(OVERLAY, 10),
        vec![percent_line(LINE_A, LineKey::for_plan(plan(1)), 1000)],
        stamp(),
    )
    .await
    .expect("the overlay is created");
    let record = repo
        .load(&scope, TENANT, OVERLAY, 0)
        .await
        .expect("the read succeeds")
        .expect("revision 0 exists");

    let world = repo
        .world_for(&scope, TENANT, &record)
        .await
        .expect("the world assembles");

    assert!(
        world.published_plans.contains(&plan(1)),
        "a retired plan is still the plan's current revision (D-128), so the referential \
         rule must not refuse it"
    );
    assert!(
        world.retired_plans.contains(&plan(1)),
        "and it must still be flagged, which is what D-31's warning is"
    );
}

/// An **unpublished** plan is genuinely not a target — the other side of the same
/// rule, so the fix above cannot have made the referential check vacuous.
#[tokio::test]
async fn a_draft_only_target_is_not_a_published_target() {
    let provider = provider().await;
    seed_plan(&provider, plan(1), 0, "draft").await;
    let repo = OverlayRepo::new(provider);
    let scope = AccessScope::allow_all();

    repo.create(
        &scope,
        new_overlay(OVERLAY, 10),
        vec![percent_line(LINE_A, LineKey::for_plan(plan(1)), 1000)],
        stamp(),
    )
    .await
    .expect("the overlay is created");
    let record = repo
        .load(&scope, TENANT, OVERLAY, 0)
        .await
        .expect("the read succeeds")
        .expect("revision 0 exists");

    let world = repo
        .world_for(&scope, TENANT, &record)
        .await
        .expect("the world assembles");
    assert!(!world.published_plans.contains(&plan(1)));
    assert!(!world.retired_plans.contains(&plan(1)));
}

/// A caller-supplied `line_id` already taken at that revision is a **typed
/// refusal**, not a storage fault.
///
/// The line's primary key is `(line_id, overlay_revision)` and is **not** scoped
/// by overlay, while `line_id` is optional on the wire and echoed by every read —
/// so pasting a line id read off one overlay into another at the same revision is
/// a plausible "clone this line" act, and it used to answer 500.
#[tokio::test]
async fn a_line_id_already_taken_at_that_revision_is_a_typed_refusal() {
    let (repo, scope) = seeded().await;

    let refusal = repo
        .create(
            &scope,
            new_overlay(Uuid::from_u128(0xBBBB_BBBB), 20),
            // `LINE_A` is already `(LINE_A, 0)` under the seeded overlay.
            vec![percent_line(LINE_A, LineKey::list_default(), 700)],
            stamp(),
        )
        .await
        .expect_err("the line identity is taken at revision 0");
    assert!(
        matches!(refusal, RepoError::ValueOutOfRange { ref field, .. } if field == "line_id"),
        "got {refusal:?}"
    );
}

/// **A discarded revision number IS re-minted, and no repository change closes
/// it.**
///
/// This case began life asserting the opposite. `open_revision` was changed from
/// `published.revision + 1` to `max(revision) + 1` to close it — and the test
/// then failed, because a discarded draft is **deleted** (§6 gives the overlay
/// three states and no `abandoned` tombstone, so DELETE is the only exit a draft
/// has), and `max` cannot see a row that is gone.
///
/// So the hazard is real and its cause is the schema, not the repository: a
/// client still holding the discarded draft's entity tag `"1-3"` will find a
/// **different** revision 1 under the same identity, and a stale `If-Match:
/// "1-0"` would match the fresh row's compare-and-swap. Closing it needs a
/// tombstone the design set does not declare — owed-register entry **O-13**.
///
/// The assertion is what actually happens, so the day a tombstone lands this test
/// fails and points at the entry.
#[tokio::test]
async fn a_discarded_revision_number_is_re_minted() {
    let (repo, scope, provider) = seeded_full().await;
    repo.publish_revision(&scope, TENANT, OVERLAY, 0, stamp())
        .await
        .expect("revision 0 publishes");
    repo.open_revision(&scope, TENANT, OVERLAY, stamp())
        .await
        .expect("revision 1 opens");

    // Discard revision 1 the way the store permits.
    discard_revision(&provider, OVERLAY, 1).await;

    let successor = repo
        .open_revision(&scope, TENANT, OVERLAY, stamp())
        .await
        .expect("a fresh successor opens");
    assert_eq!(
        successor, 1,
        "the discarded draft's number is re-minted, because the row it consumed is gone \
         and no tombstone records it (O-13)"
    );
}

/// A re-entrant publish of an already-published revision refuses and **writes
/// nothing** — in particular it does not demote the predecessor on the way to
/// saying no.
#[tokio::test]
async fn a_re_entrant_publish_refuses_without_demoting_anything() {
    let (repo, scope) = seeded().await;
    repo.publish_revision(&scope, TENANT, OVERLAY, 0, stamp())
        .await
        .expect("revision 0 publishes");

    let refusal = repo
        .publish_revision(&scope, TENANT, OVERLAY, 0, stamp())
        .await
        .expect_err("revision 0 is no longer an open draft");
    assert!(
        matches!(refusal, RepoError::NotFound { .. }),
        "got {refusal:?}"
    );

    let current = repo
        .current(&scope, TENANT, OVERLAY)
        .await
        .expect("the read succeeds")
        .expect("the overlay still has a published revision");
    assert_eq!(
        current.revision, 0,
        "the published revision must not have been demoted by the refused call"
    );
}
