//! The increment coalescer and its transaction — `design/06` §2 rules 2 and
//! 3, `flow-snapshot` rules 1-3 (`dod-coalescer`, `dod-snapshot-builder`;
//! P-D-53, P-D-56, P-D-60, P-D-67, P-D-80, P-D-83).
//!
//! # One worker per tenant, and the lease is the drain's
//!
//! [`drain_tenant`] is one pass over one tenant's demand: read the pending
//! queue, decide whether a window has closed, and — only then — take the
//! per-tenant lease and run the increment transaction inside
//! `LeaseGuard::with_ack_in_tx`, the shared BSS write fence the `DoD` names.
//! The request door never touches the lease (P-D-56); a peer already
//! holding it is a skipped pass, not an error. The coord primitive runs its
//! own serializable retry loop internally; P-D-53's refusal semantics are
//! untouched by that, because `STAGED_ENTITY_CHANGED` is decided by the
//! explicit stage-vs-commit compare below, never by a serialization
//! conflict.
//!
//! # The windows (D-47's lanes)
//!
//! Interactive demand drains when the earliest pending interactive request
//! is [`INTERACTIVE_WINDOW`] old — the batch lands as one version within
//! ≤ 5 s of its earliest member. A **bulk** batch is keyed by its
//! `operation_key` and stays open until [`BULK_WINDOW_MAX`] from its own
//! earliest request — there is no early-close signal (P-D-46) — and lands
//! as ONE version. **Bulk readiness is checked first**: a steady
//! interactive trickle must not defer a bulk window past its hard max, and
//! the probe for exactly that is in this module's tests. One pass commits
//! at most one version; the next tick takes the next ready window, which is
//! how interactive versions publish in between without shredding an open
//! bulk batch.
//!
//! # Stage vs commit (`inst-sn-revalidate`)
//!
//! The heads are collected **before** the lease is taken; inside the
//! transaction they are re-read and compared — any entity whose
//! `published_version` **or** `lifecycle_state` moved, appeared or vanished
//! between collect and commit fails the pass closed. Every requester on
//! these lanes today is mechanical, so the failure is
//! [`DrainOutcome::Restaged`]: the requests stay `pending`, the next tick
//! re-collects fresh, and the request is never lost. (The operator
//! catalog-publish door, when it lands, owns the wire-visible
//! `STAGED_ENTITY_CHANGED` arm of the same compare.)
//!
//! # The manifest (P-D-80, P-D-83)
//!
//! [`VersionManifest`] renders under `Absence::Null` against
//! [`MANIFEST_ROSTER`] — the envelope's own field names, pinned here so
//! slice 10's drill can re-verify a stored manifest against a rule in code.
//! Keyed collections sort by their own key rendering: entry rows by
//! `(entity_kind, entity_id)`, capture rows by `capture_kind`. The admitted
//! capture set is [`CAPTURE_KINDS`] — §4's seven, the builder being the
//! enforcement site P-D-74 left it to — and a kind is written exactly when
//! its source store ships: today that is the freeze-participant set alone,
//! the other six arriving with `02`/`03`/`07`'s stores. The checksum is
//! [`canonical::content_digest`] over the rendering, hex, with
//! [`canonical::DIGEST_VERSION`] stored beside it (P-D-73).
//!
//! `freeze_state` is seeded `complete` for an empty participant snapshot —
//! the ledger vacuously reads complete, and nothing would ever ack — and
//! `open` otherwise, with one `pending` ledger row seeded per participant
//! (P-D-67). No `CatalogVersionPublished` is emitted yet: the event's body
//! shape for a non-entity subject is `dod-cv-events`' own open question
//! (§7 rows 27, 35, 39, 47), and consumers poll through the port until it
//! resolves.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-producer-snapshot:p1
//! @cpt-dod:cpt-cf-bss-products-dod-coalescer:p1
//! @cpt-dod:cpt-cf-bss-products-dod-snapshot-builder:p1
//! @cpt-dod:cpt-cf-bss-products-dod-stage-commit-revalidation:p1

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::{Map as JsonMap, Value as JsonValue};
use toolkit_db::secure::AccessScope;
use toolkit_db::{DBProvider, DbError};
use uuid::Uuid;

use bss_products_sdk::increments::IncrementLane;

use crate::domain::canonical;
use crate::domain::states::{FreezeAckState, FreezeState, ProducerState};
use crate::infra::storage::RepoError;
use crate::infra::storage::repo::{
    self, NewCatalogVersion, PendingIncrementRequest, SnapshotEntityRef,
};

/// The interactive lane's coalescing window: the batch lands within this
/// long of its earliest pending member (`design/06` §2 rule 2's "≤ 5 s").
pub const INTERACTIVE_WINDOW: Duration = Duration::from_secs(5);

/// The bulk lane's hard maximum: a keyed batch stays open this long from
/// its earliest request and no signal closes it earlier (P-D-46).
pub const BULK_WINDOW_MAX: Duration = Duration::from_mins(5);

/// The manifest envelope's complete-set roster (P-D-80). `DIGEST_VERSION`
/// governs any change to this list or to the rendering rule.
pub const MANIFEST_ROSTER: [&str; 3] = ["captures", "entries", "participant_set"];

/// §4's seven admitted `capture_kind` values (P-D-83 — the builder is the
/// enforcement site). Slugs derived from §4's own list, sorted; a capture
/// is written exactly when its source store ships.
pub const CAPTURE_KINDS: [&str; 7] = [
    "attribute_definitions",
    "category_tree",
    "category_values",
    "freeze_participant_set",
    "metadata_maps",
    "recognized_sets",
    "reference_producer_set",
];

/// One pass's verdict over one tenant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DrainOutcome {
    /// No pending demand at all.
    NoDemand,
    /// Demand exists but no window has closed yet.
    WindowOpen,
    /// A peer holds the tenant's lease; this pass did nothing.
    LeaseHeld,
    /// The stage-vs-commit compare failed closed; the requests stay
    /// `pending` and the next tick re-collects fresh.
    Restaged,
    /// One version committed.
    Committed {
        /// The allocated gapless id.
        catalog_version_id: i64,
        /// How many requests it satisfied.
        satisfied: usize,
    },
}

/// The batch one pass drains: the request keys and the lane that closed.
struct ReadyBatch {
    keys: Vec<(String, String)>,
}

/// Pick the ready window, bulk first (the starvation rule).
fn ready_batch(pending: &[PendingIncrementRequest], now: DateTime<Utc>) -> Option<ReadyBatch> {
    // Bulk groups, keyed by operation_key, ready when their own earliest
    // request has aged past the hard max. `pending` arrives oldest-first,
    // so the first ready group found is the longest-waiting one.
    let mut seen: Vec<&str> = Vec::new();
    for request in pending.iter().filter(|r| r.lane == IncrementLane::Bulk) {
        let Some(op) = request.operation_key.as_deref() else {
            continue;
        };
        if seen.contains(&op) {
            continue;
        }
        seen.push(op);
        let group: Vec<&PendingIncrementRequest> = pending
            .iter()
            .filter(|r| r.lane == IncrementLane::Bulk && r.operation_key.as_deref() == Some(op))
            .collect();
        let earliest = group
            .iter()
            .map(|r| r.requested_at)
            .min()
            .unwrap_or(request.requested_at);
        if now
            >= earliest
                + chrono::Duration::from_std(BULK_WINDOW_MAX).unwrap_or(chrono::Duration::zero())
        {
            return Some(ReadyBatch {
                keys: group
                    .iter()
                    .map(|r| (r.source.clone(), r.request_key.clone()))
                    .collect(),
            });
        }
    }

    // The interactive batch: every pending interactive request, ready when
    // the earliest is INTERACTIVE_WINDOW old.
    let interactive: Vec<&PendingIncrementRequest> = pending
        .iter()
        .filter(|r| r.lane == IncrementLane::Interactive)
        .collect();
    let earliest = interactive.iter().map(|r| r.requested_at).min()?;
    if now
        >= earliest
            + chrono::Duration::from_std(INTERACTIVE_WINDOW).unwrap_or(chrono::Duration::zero())
    {
        return Some(ReadyBatch {
            keys: interactive
                .iter()
                .map(|r| (r.source.clone(), r.request_key.clone()))
                .collect(),
        });
    }
    None
}

/// The staged snapshot — everything collected before the lease, compared
/// and written inside the transaction.
pub struct VersionManifest {
    /// The entry half: references into immutable `products_entity_version`.
    pub entries: Vec<SnapshotEntityRef>,
    /// The capture half: `(capture_kind, canonical content)` — stored
    /// copies, never references.
    pub captures: Vec<(String, String)>,
    /// The freeze-participant snapshot (AC #23).
    pub participant_set: Vec<String>,
}

impl VersionManifest {
    /// The canonical rendering the checksum covers: the complete-set arm
    /// against [`MANIFEST_ROSTER`], keyed collections sorted by their own
    /// key rendering (P-D-80).
    #[must_use]
    pub fn render(&self) -> String {
        let mut entries = self.entries.clone();
        entries.sort_by(|a, b| {
            (a.entity_kind.as_str(), a.entity_id).cmp(&(b.entity_kind.as_str(), b.entity_id))
        });
        let mut captures = self.captures.clone();
        captures.sort_by(|a, b| a.0.cmp(&b.0));
        let mut participants = self.participant_set.clone();
        participants.sort();

        let entry_values: Vec<JsonValue> = entries
            .iter()
            .map(|entry| {
                let mut map = JsonMap::new();
                map.insert(
                    "entity_id".to_owned(),
                    JsonValue::String(entry.entity_id.to_string()),
                );
                map.insert(
                    "entity_kind".to_owned(),
                    JsonValue::String(entry.entity_kind.clone()),
                );
                map.insert(
                    "published_version".to_owned(),
                    JsonValue::Number(entry.published_version.into()),
                );
                JsonValue::Object(map)
            })
            .collect();
        let capture_values: Vec<JsonValue> = captures
            .iter()
            .map(|(kind, content)| {
                let mut map = JsonMap::new();
                map.insert("capture_kind".to_owned(), JsonValue::String(kind.clone()));
                map.insert("content".to_owned(), JsonValue::String(content.clone()));
                JsonValue::Object(map)
            })
            .collect();
        let participant_values: Vec<JsonValue> =
            participants.into_iter().map(JsonValue::String).collect();

        let mut envelope = JsonMap::new();
        envelope.insert("captures".to_owned(), JsonValue::Array(capture_values));
        envelope.insert("entries".to_owned(), JsonValue::Array(entry_values));
        envelope.insert(
            "participant_set".to_owned(),
            JsonValue::Array(participant_values),
        );
        canonical::canonical_rendering(
            &JsonValue::Object(envelope),
            canonical::Absence::Null {
                roster: &MANIFEST_ROSTER,
            },
        )
    }

    /// The hex checksum over [`Self::render`].
    #[must_use]
    pub fn checksum(&self) -> String {
        let digest = canonical::content_digest(&self.render());
        digest
            .iter()
            .fold(String::with_capacity(digest.len() * 2), |mut hex, byte| {
                hex.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
                hex.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
                hex
            })
    }
}

/// The snapshot builder — `design/06` §1.7's own name for the collect +
/// render engine. Stateless; the staged data rides [`VersionManifest`].
pub struct SnapshotBuilder;

impl SnapshotBuilder {
    /// Collect the manifest for one tenant: entity references, the
    /// participant snapshot, and every capture whose source store ships.
    ///
    /// # Errors
    ///
    /// [`RepoError`] as the reads raise it.
    pub async fn collect(
        runner: &impl toolkit_db::secure::DBRunner,
        scope: &AccessScope,
        tenant_id: Uuid,
    ) -> Result<VersionManifest, RepoError> {
        let entries = repo::snapshot_entity_refs(runner, scope, tenant_id).await?;
        let participant_set = repo::freeze_participants(runner, scope, tenant_id).await?;

        // The two captures with shipped sources, each rendered canonically
        // like every stored copy (H3): the freeze-participant set, and the
        // **registered reference-producer set** — `07`'s symmetric ride
        // (`inst-pr-snapshot`, `dod-producer-snapshot`), which is what makes
        // a historical reference verdict judgeable against the
        // then-registered set rather than today's. The other five of
        // CAPTURE_KINDS arrive with their stores.
        let participants_value = JsonValue::Array(
            participant_set
                .iter()
                .cloned()
                .map(JsonValue::String)
                .collect(),
        );
        let producers_value = JsonValue::Array(
            repo::reference_producers(runner, scope, tenant_id)
                .await?
                .into_iter()
                .filter(|row| row.state == ProducerState::Registered)
                .map(|row| JsonValue::String(row.producer))
                .collect(),
        );
        let captures = vec![
            (
                "freeze_participant_set".to_owned(),
                canonical::canonical_rendering(&participants_value, canonical::Absence::Omit),
            ),
            (
                "reference_producer_set".to_owned(),
                canonical::canonical_rendering(&producers_value, canonical::Absence::Omit),
            ),
        ];
        for (kind, _) in &captures {
            assert!(
                CAPTURE_KINDS.contains(&kind.as_str()),
                "capture kind {kind} is outside the admitted roster"
            );
        }

        Ok(VersionManifest {
            entries,
            captures,
            participant_set,
        })
    }
}

/// One coalescer pass over one tenant at `now`. See the module doc for the
/// window rules and the transaction's contents.
///
/// # Errors
///
/// [`RepoError`] as the reads and writes raise it; a lost lease surfaces as
/// [`DrainOutcome::LeaseHeld`], not an error.
pub async fn drain_tenant(
    db: &DBProvider<DbError>,
    tenant_id: Uuid,
    now: DateTime<Utc>,
) -> Result<DrainOutcome, RepoError> {
    let scope = AccessScope::for_tenant(tenant_id);
    // The read connection lives in its own block so it is returned before
    // the lease work below checks one out again.
    let (staged, batch) = {
        let conn = db
            .conn()
            .map_err(|e| RepoError::Db(format!("coalescer connection: {e}")))?;
        let pending = repo::pending_increment_requests(&conn, &scope, tenant_id).await?;
        if pending.is_empty() {
            return Ok(DrainOutcome::NoDemand);
        }
        let Some(batch) = ready_batch(&pending, now) else {
            return Ok(DrainOutcome::WindowOpen);
        };
        // Stage: collect outside the lease (inst-sn-collect's serialization
        // is the coalescer's own, P-D-53).
        let staged = SnapshotBuilder::collect(&conn, &scope, tenant_id).await?;
        (staged, batch)
    };

    commit_increment(db, tenant_id, staged, batch.keys, now).await
}

/// The staged half handed to the committing half — split from
/// [`drain_tenant`] so the stage-vs-commit compare is testable: a test
/// stages, moves a head, then commits, which is the exact window
/// `inst-sn-revalidate` guards.
///
/// # Errors
///
/// As [`drain_tenant`].
pub async fn commit_increment(
    db: &DBProvider<DbError>,
    tenant_id: Uuid,
    staged: VersionManifest,
    keys: Vec<(String, String)>,
    now: DateTime<Utc>,
) -> Result<DrainOutcome, RepoError> {
    let scope = AccessScope::for_tenant(tenant_id);
    // The per-tenant lease, then the transaction under its write fence.
    let lease = coord::LeaseManager::new(db.db());
    let guard = match lease
        .acquire(&format!("cv-increment:{tenant_id}"), LEASE_TTL)
        .await
    {
        Ok(guard) => guard,
        Err(coord::CoordError::LeaseHeld) => return Ok(DrainOutcome::LeaseHeld),
        Err(coord::CoordError::Db(e)) => {
            return Err(RepoError::Db(format!("increment lease: {e}")));
        }
        Err(other) => {
            return Err(RepoError::Db(format!("increment lease: {other}")));
        }
    };

    let staged_entries = staged.entries.clone();
    let staged_manifest = std::sync::Arc::new(staged);
    let batch_keys = std::sync::Arc::new(keys);
    let outcome = guard
        .with_ack_in_tx(
            |e: &RepoError| match e {
                RepoError::Driver { source, .. } => Some(source),
                _ => None,
            },
            |tx| {
                let staged_entries = staged_entries.clone();
                let manifest = std::sync::Arc::clone(&staged_manifest);
                let keys = std::sync::Arc::clone(&batch_keys);
                let scope = scope.clone();
                Box::pin(async move {
                    // Stage vs commit: re-read the heads inside the
                    // transaction and compare both ways.
                    let live = repo::snapshot_entity_refs(tx, &scope, tenant_id).await?;
                    if live != staged_entries {
                        return Ok(DrainOutcome::Restaged);
                    }

                    let catalog_version_id =
                        repo::allocate_catalog_version_id(tx, &scope, tenant_id).await?;
                    let checksum = manifest.checksum();
                    let participants_rendering = canonical::canonical_rendering(
                        &JsonValue::Array(
                            manifest
                                .participant_set
                                .iter()
                                .cloned()
                                .map(JsonValue::String)
                                .collect(),
                        ),
                        canonical::Absence::Omit,
                    );
                    let freeze_state = if manifest.participant_set.is_empty() {
                        FreezeState::Complete
                    } else {
                        FreezeState::Open
                    };
                    repo::insert_catalog_version(
                        tx,
                        &scope,
                        tenant_id,
                        NewCatalogVersion {
                            catalog_version_id,
                            checksum,
                            digest_version: canonical::DIGEST_VERSION,
                            published_at: now,
                            participant_set_snapshot: participants_rendering,
                            freeze_state,
                        },
                    )
                    .await?;
                    repo::insert_catalog_version_entries(
                        tx,
                        &scope,
                        tenant_id,
                        catalog_version_id,
                        &manifest.entries,
                    )
                    .await?;
                    for (kind, content) in &manifest.captures {
                        repo::insert_catalog_version_capture(
                            tx,
                            &scope,
                            tenant_id,
                            catalog_version_id,
                            kind,
                            content,
                        )
                        .await?;
                    }
                    repo::seed_freeze_acks(
                        tx,
                        &scope,
                        tenant_id,
                        catalog_version_id,
                        &manifest.participant_set,
                    )
                    .await?;
                    repo::mark_requests_coalesced(tx, &scope, tenant_id, &keys, catalog_version_id)
                        .await?;
                    Ok(DrainOutcome::Committed {
                        catalog_version_id,
                        satisfied: keys.len(),
                    })
                })
            },
        )
        .await;

    // Release on EVERY path before the outcome is interpreted: the guard
    // has no Drop-based release, so an early return on a failed transaction
    // would hold the tenant's cv-increment lease until the TTL and block
    // every coalescer pass for that tenant in the window. A failed release
    // is the same TTL wait — named in the log rather than swallowed.
    if let Err(release_err) = guard.release_with_retry().await {
        tracing::warn!(
            %tenant_id,
            error = %release_err,
            "bss-products: cv-increment lease release failed; the lease holds until its TTL"
        );
    }

    match outcome {
        Ok(outcome) => Ok(outcome),
        Err(coord::AckError::LeaseLost) => Ok(DrainOutcome::LeaseHeld),
        Err(coord::AckError::Work(e)) => Err(e),
        Err(coord::AckError::Db(e)) => Err(RepoError::Db(format!("increment transaction: {e}"))),
    }
}

/// One overdue `open` version and the participants still silent on it —
/// `freeze_overdue`'s operand (`dod-freeze-timeout`). The timeout fails
/// closed (the resolver keeps refusing `posted`), so this scan only names
/// the silence; in v1 the named set is pricing, the registered set's one
/// member, which is what makes the PRD §15 open visible in this gear's own
/// telemetry from day one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverdueFreeze {
    /// The version's tenant.
    pub tenant_id: Uuid,
    /// The overdue version.
    pub catalog_version_id: i64,
    /// The snapshot members whose ledger rows still read `pending`.
    pub silent_participants: Vec<String>,
}

/// Scan for `open` versions older than the configured freeze timeout.
///
/// # Errors
///
/// [`RepoError`] as the reads raise it.
pub async fn overdue_freezes(
    db: &DBProvider<DbError>,
    now: DateTime<Utc>,
    freeze_timeout_hours: u32,
) -> Result<Vec<OverdueFreeze>, RepoError> {
    let conn = db
        .conn()
        .map_err(|e| RepoError::Db(format!("overdue scan connection: {e}")))?;
    let cutoff = now - chrono::Duration::hours(i64::from(freeze_timeout_hours));
    let versions = repo::overdue_open_versions(&conn, &AccessScope::allow_all(), cutoff).await?;
    let mut overdue = Vec::with_capacity(versions.len());
    for (tenant_id, catalog_version_id) in versions {
        let scope = AccessScope::for_tenant(tenant_id);
        let silent: Vec<String> =
            repo::freeze_ack_rows(&conn, &scope, tenant_id, catalog_version_id)
                .await?
                .into_iter()
                .filter(|(_, state)| *state == FreezeAckState::Pending)
                .map(|(participant, _)| participant)
                .collect();
        overdue.push(OverdueFreeze {
            tenant_id,
            catalog_version_id,
            silent_participants: silent,
        });
    }
    Ok(overdue)
}

/// The lease TTL: generous against a slow transaction, far above the tick.
const LEASE_TTL: Duration = Duration::from_secs(30);

/// One sweep over every tenant with pending demand — the ticker's body.
///
/// One tenant's failure is logged and the loop continues: tenants iterate
/// in sorted order, so propagating the first error would let one tenant
/// with a persistent fault (a corrupt row, a bad counter) permanently
/// starve every tenant that sorts after it. The discovery read runs under
/// the system scope and every per-tenant pass narrows to `for_tenant`
/// (the sibling pricing jobs' documented pattern).
///
/// # Errors
///
/// The last [`RepoError`] raised — only when EVERY tenant's pass failed,
/// which is the whole-sweep fault (a dead database) rather than one
/// tenant's.
pub async fn sweep(
    db: &DBProvider<DbError>,
    now: DateTime<Utc>,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<(), RepoError> {
    let tenants = {
        let conn = db
            .conn()
            .map_err(|e| RepoError::Db(format!("coalescer sweep connection: {e}")))?;
        repo::tenants_with_pending_requests(&conn, &AccessScope::allow_all()).await?
    };
    let total = tenants.len();
    let mut failed = 0_usize;
    let mut last_err: Option<RepoError> = None;
    for tenant in tenants {
        // The shutdown seam: one pass commits at most one version and is
        // left to finish, but the sweep stops taking new tenants the
        // moment the gear is asked to stop.
        if cancel.is_cancelled() {
            return Ok(());
        }
        match drain_tenant(db, tenant, now).await {
            Ok(outcome) => log_drain_outcome(tenant, &outcome),
            Err(e) => {
                failed += 1;
                tracing::error!(
                    %tenant,
                    error = %e,
                    "bss-products: increment pass failed; later tenants continue"
                );
                last_err = Some(e);
            }
        }
    }
    match last_err {
        Some(e) if failed == total => Err(e),
        _ => Ok(()),
    }
}

/// The outcome telemetry: allocating gapless version ids is this gear's
/// most operationally significant act, so a committed pass, a fail-closed
/// restage and persistent lease contention must all be visible to an
/// operator, not discarded.
// The tracing macros inflate the metric; the function is one flat match
// (the api-gateway's `log_authn_error` carries the same allow for the
// same shape).
#[allow(clippy::cognitive_complexity)]
fn log_drain_outcome(tenant: Uuid, outcome: &DrainOutcome) {
    match outcome {
        DrainOutcome::Committed {
            catalog_version_id,
            satisfied,
        } => {
            tracing::info!(
                %tenant,
                catalog_version_id,
                satisfied,
                "bss-products: catalog version committed"
            );
        }
        DrainOutcome::Restaged => {
            tracing::warn!(
                %tenant,
                "bss-products: increment pass failed closed (stage-vs-commit moved); \
                 requests stay pending"
            );
        }
        DrainOutcome::LeaseHeld => {
            tracing::debug!(
                %tenant,
                "bss-products: cv-increment lease held by a peer; pass skipped"
            );
        }
        DrainOutcome::NoDemand | DrainOutcome::WindowOpen => {}
    }
}

#[cfg(test)]
#[path = "increment_tests.rs"]
mod increment_tests;
