#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Real toolkit-db transaction/runner plumbing with nontransactional in-memory state.
//!
//! These fakes deliberately do not roll back. SQL rollback and competing writers
//! are covered by the SQL-backed gear store in phase 1c.
use bss_approval::{
    ApprovalError, ApprovalSubject, ApproveOutcome, Decision, Engine, ItemRef, Policy, Store,
    SubmitRequest, Unit, UnitState,
};
use parking_lot::Mutex;
use std::{collections::BTreeMap, sync::Arc};
use time::{Date, OffsetDateTime, macros::datetime};
use toolkit_db::secure::{DbTx, TxConfig};
use toolkit_db::{ConnectOpts, Db, DbError, connect_db};
use uuid::Uuid;

// ---- a real Db; the closure's runner type is DbTx<'a> (conv §1) ----
async fn db() -> Db {
    connect_db(
        "sqlite::memory:",
        ConnectOpts {
            max_conns: Some(1),
            min_conns: Some(1),
            ..Default::default()
        },
    )
    .await
    .unwrap()
}
#[derive(Debug)]
enum TxErr {
    Approval(ApprovalError),
    Db(DbError),
}
impl From<DbError> for TxErr {
    fn from(e: DbError) -> Self {
        Self::Db(e)
    }
}
fn no_retry(_: &TxErr) -> Option<&sea_orm::DbErr> {
    None
}

/// Runs `f` inside one transaction; `f` gets `&DbTx` and returns the engine's result.
async fn in_tx<T: Send + 'static>(
    db: &Db,
    mut f: impl for<'a> FnMut(
        &'a DbTx<'a>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<T, ApprovalError>> + Send + 'a>,
    > + Send,
) -> Result<T, ApprovalError> {
    db.transaction_with_retry::<T, TxErr, _, _>(TxConfig::default(), no_retry, move |tx| {
        let future = f(tx);
        Box::pin(async move { future.await.map_err(TxErr::Approval) })
    })
    .await
    .map_err(|e| match e {
        TxErr::Approval(a) => a,
        TxErr::Db(d) => ApprovalError::Store(d.to_string()),
    })
}

async fn unit_of(db: &Db, store: &Mem, id: Uuid) -> Unit {
    let store = store.clone();
    in_tx(db, move |tx| {
        let store = store.clone();
        Box::pin(async move { store.unit(tx, id).await.map(|unit| unit.unwrap()) })
    })
    .await
    .unwrap()
}

// ---- in-memory store: a map behind a mutex, typed over DbTx so the Engine generics resolve to the real runner ----
type Units = BTreeMap<Uuid, (Unit, Vec<ItemRef>)>;

#[derive(Default, Clone)]
struct Mem {
    units: Arc<Mutex<Units>>,
    decisions: Arc<Mutex<Vec<Decision>>>,
    steal_next_bump: Arc<Mutex<bool>>,
}

#[async_trait::async_trait]
impl<'a> Store<DbTx<'a>> for Mem {
    async fn insert_unit(
        &self,
        _: &DbTx<'a>,
        u: &Unit,
        items: &[ItemRef],
    ) -> Result<(), ApprovalError> {
        self.units.lock().insert(u.id, (u.clone(), items.to_vec()));
        Ok(())
    }
    async fn unit(&self, _: &DbTx<'a>, id: Uuid) -> Result<Option<Unit>, ApprovalError> {
        Ok(self.units.lock().get(&id).map(|(u, _)| u.clone()))
    }
    async fn bump_version(
        &self,
        _: &DbTx<'a>,
        id: Uuid,
        expected: i64,
    ) -> Result<bool, ApprovalError> {
        let mut steal = self.steal_next_bump.lock();
        if *steal {
            *steal = false;
            if let Some((u, _)) = self.units.lock().get_mut(&id) {
                u.version += 1;
            }
            return Ok(false);
        } // the other writer's bump lands, ours misses
        let mut g = self.units.lock();
        let Some((u, _)) = g.get_mut(&id) else {
            return Ok(false);
        };
        if u.version != expected {
            return Ok(false);
        }
        u.version += 1;
        Ok(true)
    }
    async fn items(&self, _: &DbTx<'a>, id: Uuid) -> Result<Vec<ItemRef>, ApprovalError> {
        Ok(self
            .units
            .lock()
            .get(&id)
            .map(|(_, i)| i.clone())
            .unwrap_or_default())
    }
    async fn decisions(&self, _: &DbTx<'a>, id: Uuid) -> Result<Vec<Decision>, ApprovalError> {
        Ok(self
            .decisions
            .lock()
            .iter()
            .filter(|d| d.unit_id == id)
            .cloned()
            .collect())
    }
    async fn insert_decision(&self, _: &DbTx<'a>, d: &Decision) -> Result<(), ApprovalError> {
        let mut g = self.decisions.lock();
        if g.iter()
            .any(|x| x.unit_id == d.unit_id && x.actor == d.actor && x.generation == d.generation)
        {
            return Err(ApprovalError::Store(
                "PK (unit_id, actor, generation)".into(),
            ));
        }
        g.push(d.clone());
        Ok(())
    }
    async fn refresh(
        &self,
        _: &DbTx<'a>,
        id: Uuid,
        items: &[ItemRef],
        snapshot: &serde_json::Value,
        hash: &str,
        generation: i32,
    ) -> Result<(), ApprovalError> {
        let mut g = self.units.lock();
        let (u, i) = g.get_mut(&id).unwrap();
        u.generation = generation;
        u.snapshot = snapshot.clone();
        hash.clone_into(&mut u.snapshot_hash);
        *i = items.to_vec();
        for d in self.decisions.lock().iter_mut() {
            if d.unit_id == id && d.generation < generation {
                d.stale = true;
            }
        }
        Ok(())
    }
    async fn set_state(
        &self,
        _: &DbTx<'a>,
        id: Uuid,
        s: UnitState,
        at: Option<OffsetDateTime>,
        note: Option<&str>,
    ) -> Result<(), ApprovalError> {
        let mut g = self.units.lock();
        let (u, _) = g.get_mut(&id).unwrap();
        u.state = s;
        u.decided_at = at;
        u.decided_note = note.map(str::to_owned);
        Ok(())
    }
}

type LiveRows = BTreeMap<Uuid, (Uuid, i64)>;

/// Rows whose business content is one amount; the lock lives in a separate map so it is never part of `after`.
#[derive(Default, Clone)]
struct Rows {
    live: Arc<Mutex<LiveRows>>,
    locked: Arc<Mutex<BTreeMap<Uuid, Uuid>>>,
    applied: Arc<Mutex<Vec<Uuid>>>,
    refuse_apply: Arc<Mutex<bool>>,
}

#[async_trait::async_trait]
impl<'a> ApprovalSubject<DbTx<'a>> for Rows {
    fn kind(&self) -> &'static str {
        "price_rows"
    }
    fn ref_type(&self) -> &'static str {
        "book"
    }
    async fn collect(&self, _: &DbTx<'a>, ids: &[Uuid]) -> Result<Vec<ItemRef>, ApprovalError> {
        let live = self.live.lock();
        Ok(ids
            .iter()
            .map(|id| {
                let (author, amount) = live[id];
                ItemRef {
                    item_type: "price_row".into(),
                    item_id: *id,
                    created_by: author,
                    before: None,
                    after: serde_json::json!({ "amount": amount }),
                }
            })
            .collect())
    }
    async fn validate_submit(&self, _: &DbTx<'a>, items: &[ItemRef]) -> Result<(), ApprovalError> {
        if items.is_empty() {
            Err(ApprovalError::Empty)
        } else {
            Ok(())
        }
    }
    async fn lock(&self, _: &DbTx<'a>, unit: Uuid, items: &[ItemRef]) -> Result<(), ApprovalError> {
        let mut l = self.locked.lock();
        for i in items {
            if l.contains_key(&i.item_id) {
                return Err(ApprovalError::Locked {
                    item_type: i.item_type.clone(),
                    item_id: i.item_id,
                });
            }
        }
        for i in items {
            l.insert(i.item_id, unit);
        }
        Ok(())
    }
    fn snapshot(&self, items: &[ItemRef], _: Option<Date>) -> serde_json::Value {
        serde_json::json!({ "n": items.len() })
    }
    async fn apply(&self, _: &DbTx<'a>, _: &Unit, items: &[ItemRef]) -> Result<(), ApprovalError> {
        if *self.refuse_apply.lock() {
            return Err(ApprovalError::ApplyRefused {
                code: "SKU_NAME_TAKEN",
                detail: "taken meanwhile".into(),
            });
        }
        self.applied.lock().extend(items.iter().map(|i| i.item_id));
        Ok(())
    }
    async fn unlock(
        &self,
        _: &DbTx<'a>,
        _: &Unit,
        items: &[ItemRef],
        _: bool,
    ) -> Result<(), ApprovalError> {
        let mut l = self.locked.lock();
        for i in items {
            l.remove(&i.item_id);
        }
        Ok(())
    }
}

fn rows(author: Uuid, n: usize) -> (Rows, Vec<Uuid>) {
    let ids: Vec<Uuid> = (0..n).map(|_| Uuid::new_v4()).collect();
    let r = Rows::default();
    {
        let mut live = r.live.lock();
        for id in &ids {
            live.insert(*id, (author, 10));
        }
    }
    (r, ids)
}
const T0: OffsetDateTime = datetime!(2026-09-24 10:00 UTC);
const T1: OffsetDateTime = datetime!(2026-09-24 11:00 UTC);
fn policy(q: u32) -> Policy {
    Policy {
        default_quorum: q,
        overrides: BTreeMap::new(),
    }
}

async fn submit(
    db: &Db,
    store: &Mem,
    subject: &Rows,
    ids: Vec<Uuid>,
    actor: Uuid,
    q: u32,
) -> Result<bss_approval::Submitted, ApprovalError> {
    let (store, subject) = (store.clone(), subject.clone());
    in_tx(db, move |tx| {
        let (store, subject, ids) = (store.clone(), subject.clone(), ids.clone());
        Box::pin(async move {
            Engine::submit(
                &store,
                &subject,
                tx,
                SubmitRequest {
                    tenant_id: Uuid::new_v4(),
                    ref_id: Uuid::new_v4(),
                    item_ids: &ids,
                    actor,
                    policy: &policy(q),
                    common_effective_date: None,
                    now: T0,
                },
            )
            .await
        })
    })
    .await
}
async fn approve(
    db: &Db,
    store: &Mem,
    subject: &Rows,
    unit: Uuid,
    actor: Uuid,
) -> Result<ApproveOutcome, ApprovalError> {
    let (store, subject) = (store.clone(), subject.clone());
    in_tx(db, move |tx| {
        let (store, subject) = (store.clone(), subject.clone());
        Box::pin(async move {
            let g = store.unit(tx, unit).await?.map_or(1, |u| u.generation);
            Engine::approve(&store, &subject, tx, unit, actor, g, None, T1).await
        })
    })
    .await
}
#[tokio::test]
async fn a_vote_against_an_older_generation_is_refused() {
    let db = db().await;
    let author = Uuid::new_v4();
    let (subject, ids) = rows(author, 1);
    let store = Mem::default();
    let s = submit(&db, &store, &subject, ids.clone(), author, 2)
        .await
        .unwrap();
    subject.live.lock().get_mut(&ids[0]).unwrap().1 = 99;
    assert!(matches!(
        approve(&db, &store, &subject, s.unit.id, Uuid::new_v4())
            .await
            .unwrap(),
        ApproveOutcome::Refreshed { generation: 2 }
    ));
    let (st, su) = (store.clone(), subject.clone());
    let id = s.unit.id;
    let late = in_tx(&db, move |tx| {
        let (st, su) = (st.clone(), su.clone());
        Box::pin(
            async move { Engine::approve(&st, &su, tx, id, Uuid::new_v4(), 1, None, T1).await },
        )
    })
    .await;
    assert!(matches!(
        late,
        Err(ApprovalError::GenerationMismatch {
            seen: 1,
            current: 2
        })
    ));
}

#[tokio::test]
async fn quorum_zero_applies_at_submit_and_records_the_unit() {
    let db = db().await;
    let author = Uuid::new_v4();
    let (subject, ids) = rows(author, 2);
    let store = Mem::default();
    let s = submit(&db, &store, &subject, ids, author, 0).await.unwrap();
    assert_eq!(unit_of(&db, &store, s.unit.id).await, s.unit);
    assert!(store.decisions.lock().is_empty());
    assert!(s.applied);
    assert_eq!(s.unit.state, UnitState::Approved);
    assert_eq!(s.unit.decided_at, Some(T0));
    assert_eq!(subject.applied.lock().len(), 2);
    assert!(subject.locked.lock().is_empty());
}
#[tokio::test]
async fn quorum_one_pends_then_an_independent_approve_applies() {
    let db = db().await;
    let author = Uuid::new_v4();
    let (subject, ids) = rows(author, 1);
    let store = Mem::default();
    let s = submit(&db, &store, &subject, ids, author, 1).await.unwrap();
    assert!(!s.applied);
    assert!(matches!(
        approve(&db, &store, &subject, s.unit.id, author).await,
        Err(ApprovalError::SodViolation)
    ));
    assert!(matches!(
        approve(&db, &store, &subject, s.unit.id, Uuid::new_v4())
            .await
            .unwrap(),
        ApproveOutcome::Applied
    ));
    assert!(matches!(
        approve(&db, &store, &subject, s.unit.id, Uuid::new_v4()).await,
        Err(ApprovalError::AlreadyDecided)
    ));
}
#[tokio::test]
async fn a_locked_item_refuses_a_second_unit() {
    let db = db().await;
    let author = Uuid::new_v4();
    let (subject, ids) = rows(author, 1);
    let store = Mem::default();
    submit(&db, &store, &subject, ids.clone(), author, 1)
        .await
        .unwrap();
    assert!(matches!(
        submit(&db, &store, &subject, ids, author, 1).await,
        Err(ApprovalError::Locked { .. })
    ));
    // the second unit row was inserted by the fake and would be rolled back by a real store; the fake keeps it — that is the fake's limit, not the engine's
}
#[tokio::test]
async fn the_lock_itself_does_not_change_the_fingerprint() {
    let db = db().await;
    let author = Uuid::new_v4();
    let (subject, ids) = rows(author, 1);
    let store = Mem::default();
    let s = submit(&db, &store, &subject, ids, author, 1).await.unwrap();
    assert!(
        matches!(
            approve(&db, &store, &subject, s.unit.id, Uuid::new_v4())
                .await
                .unwrap(),
            ApproveOutcome::Applied
        ),
        "lock metadata is not business content"
    );
}
#[tokio::test]
async fn content_drift_refreshes_the_generation_and_the_same_reviewer_votes_again() {
    let db = db().await;
    let author = Uuid::new_v4();
    let (subject, ids) = rows(author, 1);
    let store = Mem::default();
    let s = submit(&db, &store, &subject, ids.clone(), author, 2)
        .await
        .unwrap();
    let first = Uuid::new_v4();
    assert!(matches!(
        approve(&db, &store, &subject, s.unit.id, first)
            .await
            .unwrap(),
        ApproveOutcome::Pending { have: 1, need: 2 }
    ));
    assert!(matches!(
        approve(&db, &store, &subject, s.unit.id, first).await,
        Err(ApprovalError::DuplicateVote)
    ));
    subject.live.lock().get_mut(&ids[0]).unwrap().1 = 99; // the proposed content changed under the reviewers
    assert!(matches!(
        approve(&db, &store, &subject, s.unit.id, author).await,
        Err(ApprovalError::SodViolation)
    ));
    assert!(matches!(
        approve(&db, &store, &subject, s.unit.id, first).await,
        Err(ApprovalError::DuplicateVote)
    ));
    assert_eq!(
        unit_of(&db, &store, s.unit.id).await.generation,
        1,
        "ineligible reviewers cannot refresh"
    );
    let second = Uuid::new_v4();
    assert!(matches!(
        approve(&db, &store, &subject, s.unit.id, second)
            .await
            .unwrap(),
        ApproveOutcome::Refreshed { generation: 2 }
    ));
    let u = unit_of(&db, &store, s.unit.id).await;
    assert_ne!(u.snapshot_hash, s.unit.snapshot_hash);
    assert_eq!(u.generation, 2);
    assert_eq!(u.state, UnitState::Pending);
    assert_eq!(store.decisions.lock().len(), 1, "refresh adds no vote");
    assert!(
        store.decisions.lock().iter().all(|d| d.stale),
        "the first vote is stale"
    );
    assert!(
        matches!(
            approve(&db, &store, &subject, s.unit.id, first)
                .await
                .unwrap(),
            ApproveOutcome::Pending { have: 1, need: 2 }
        ),
        "the first reviewer votes again on what they now see"
    );
    assert!(matches!(
        approve(&db, &store, &subject, s.unit.id, second)
            .await
            .unwrap(),
        ApproveOutcome::Applied
    ));
}
#[tokio::test]
async fn a_lost_version_race_is_contended_and_writes_nothing() {
    let db = db().await;
    let author = Uuid::new_v4();
    let (subject, ids) = rows(author, 1);
    let store = Mem::default();
    let s = submit(&db, &store, &subject, ids, author, 1).await.unwrap();
    *store.steal_next_bump.lock() = true; // the fake's bump_version answers false once: another writer won between our read and our CAS
    let before = store.decisions.lock().len();
    assert!(matches!(
        approve(&db, &store, &subject, s.unit.id, Uuid::new_v4()).await,
        Err(ApprovalError::Contended)
    ));
    assert_eq!(store.decisions.lock().len(), before);
    assert!(subject.applied.lock().is_empty());
    assert_eq!(
        unit_of(&db, &store, s.unit.id).await.state,
        UnitState::Pending
    );
}
#[tokio::test]
async fn an_apply_refusal_keeps_the_unit_pending() {
    let db = db().await;
    let author = Uuid::new_v4();
    let (subject, ids) = rows(author, 1);
    let store = Mem::default();
    let s = submit(&db, &store, &subject, ids, author, 1).await.unwrap();
    *subject.refuse_apply.lock() = true;
    assert!(matches!(
        approve(&db, &store, &subject, s.unit.id, Uuid::new_v4()).await,
        Err(ApprovalError::ApplyRefused { .. })
    ));
    assert!(subject.applied.lock().is_empty());
    assert!(!subject.locked.lock().is_empty());
    // the fake cannot roll back; a real store rolls the vote back with the transaction. The assertion that matters here:
    assert_eq!(
        unit_of(&db, &store, s.unit.id).await.state,
        UnitState::Pending
    );
}
#[tokio::test]
async fn reject_needs_a_note_and_unlocks_withdraw_is_the_submitters() {
    let db = db().await;
    let author = Uuid::new_v4();
    let (subject, ids) = rows(author, 1);
    let store = Mem::default();
    let s = submit(&db, &store, &subject, ids, author, 1).await.unwrap();
    let (st, su) = (store.clone(), subject.clone());
    let id = s.unit.id;
    let w = in_tx(&db, move |tx| {
        let (st, su) = (st.clone(), su.clone());
        Box::pin(async move { Engine::withdraw(&st, &su, tx, id, Uuid::new_v4(), T1).await })
    })
    .await;
    assert!(matches!(w, Err(ApprovalError::NotSubmitter)));
    let (st, su) = (store.clone(), subject.clone());
    let r = in_tx(&db, move |tx| {
        let (st, su) = (st.clone(), su.clone());
        Box::pin(async move { Engine::reject(&st, &su, tx, id, Uuid::new_v4(), 1, "  ", T1).await })
    })
    .await;
    assert!(matches!(r, Err(ApprovalError::NoteRequired)));
    let (st, su) = (store.clone(), subject.clone());
    in_tx(&db, move |tx| {
        let (st, su) = (st.clone(), su.clone());
        Box::pin(async move {
            Engine::reject(&st, &su, tx, id, Uuid::new_v4(), 1, "wrong amount", T1).await
        })
    })
    .await
    .unwrap();
    let rejected = unit_of(&db, &store, id).await;
    assert_eq!(rejected.state, UnitState::Rejected);
    assert_eq!(rejected.decided_note.as_deref(), Some("wrong amount"));
    assert!(subject.locked.lock().is_empty());
    let (subject, ids) = rows(author, 1);
    let s = submit(&db, &store, &subject, ids, author, 1).await.unwrap();
    let (st, su) = (store.clone(), subject.clone());
    let id = s.unit.id;
    in_tx(&db, move |tx| {
        let (st, su) = (st.clone(), su.clone());
        Box::pin(async move { Engine::withdraw(&st, &su, tx, id, author, T1).await })
    })
    .await
    .unwrap();
    let withdrawn = unit_of(&db, &store, id).await;
    assert_eq!(withdrawn.state, UnitState::Withdrawn);
    assert_eq!(withdrawn.decided_at, Some(T1));
    assert!(subject.locked.lock().is_empty());
}
