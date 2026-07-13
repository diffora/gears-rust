Created:  2026-07-08 by Constructor Tech
Updated:  2026-07-08 by Constructor Tech
# Feature: Audit Trail

- [ ] `p2` - **ID**: `cpt-cf-file-storage-featstatus-audit-trail-implemented`

> **Status: write-side shipped; drain/relay deferred.** Every write mutation this
> gear performs inserts an audit row transactionally into `audit_outbox`. What is
> **not** implemented is anything downstream of that insert: there is no consumer,
> exporter, or relay that ever reads a row back out and marks it `published_at`.
> See [§5 "Outbox Drain to a Downstream Sink (NOT IMPLEMENTED)"](#outbox-drain-to-a-downstream-sink-not-implemented)
> below for the explicit gap.



<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Write Operation Emits an Audit Row](#write-operation-emits-an-audit-row)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [Build and Persist an Audit Entry Transactionally](#build-and-persist-an-audit-entry-transactionally)
- [4. States (CDSL)](#4-states-cdsl)
  - [Audit Outbox Row Lifecycle](#audit-outbox-row-lifecycle)
- [5. Definitions of Done](#5-definitions-of-done)
  - [Transactional Audit-Entry Insertion on Every Write](#transactional-audit-entry-insertion-on-every-write)
  - [Schema: audit_outbox Table](#schema-audit_outbox-table)
  - [Outbox Drain to a Downstream Sink (NOT IMPLEMENTED)](#outbox-drain-to-a-downstream-sink-not-implemented)
- [6. Acceptance Criteria](#6-acceptance-criteria)

<!-- /toc -->

## 1. Feature Context

- [ ] `p2` - `cpt-cf-file-storage-feature-audit-trail`

### 1.1 Overview

A transactional-outbox audit trail: every write mutation this gear performs — file
create, finalize, bind, metadata patch, delete (file/version), multipart
complete/abort, ownership transfer, backend migration, and the background
cleanup engine's retention-delete/orphan-reconcile actions — inserts one
`AuditEntry` row into the `audit_outbox` table **in the same DB transaction**
as the mutation it describes. There is no separate "log after the fact" step
and no code path that mutates state without also building an audit row, so a
rolled-back mutation leaves zero audit rows (the same transaction covers
both).

This feature has **no REST endpoint of its own** — it is a pure side effect of
other features' write paths. The only way to read `audit_outbox` rows today is
a direct SQL query or (in tests) `Store::list_audit`; there is no
`GET /files/{id}/audit` route or equivalent exposed by this gear.

**Traces to**: `cpt-cf-file-storage-fr-audit-trail`, `cpt-cf-file-storage-nfr-audit-completeness`

### 1.2 Purpose

Give the platform a complete, tamper-evident-by-construction record of every
write this gear performs, for compliance and forensic purposes, with a
correctness guarantee stronger than "best-effort logging": because the audit
row is written in the *same* database transaction as the mutation, there is no
window in which a mutation commits without its audit row, or an audit row
exists for a mutation that was rolled back. This is the transactional-outbox
pattern applied to compliance logging rather than to event delivery (the
sibling `events_outbox` table applies the identical pattern to file lifecycle
events — see `cpt-cf-file-storage-fr-file-events`).

**Requirements**: `cpt-cf-file-storage-fr-audit-trail`, `cpt-cf-file-storage-nfr-audit-completeness`

**Principles**: `cpt-cf-file-storage-principle-control-no-content` (audit rows carry
only metadata/identifiers in `detail`, never content bytes)

> **Caveat (P2 — outbox drain/relay is NOT implemented).** `audit_outbox.published_at`
> is written as `NULL` on every insert (`AuditRepo::insert`) and nothing in this
> codebase ever sets it. Tier-4 item 4.1 of the P2 remediation plan ("EventBroker
> relay") — which would drain both `audit_outbox` and its sibling `events_outbox`
> to a downstream platform sink — is **not scheduled/implemented**. Concretely
> this means: (a) rows accumulate in `audit_outbox` indefinitely with no retention
> or archival process; (b) the background cleanup sweep's idempotency-key-expiry
> step (`cleanup.rs::run_sweep`, step 4) *deliberately* does **not** touch
> `audit_outbox`/`events_outbox` — the inline comment there explains that a
> row-age-based purge would silently drop rows that were never delivered, since
> `published_at` can never become non-`NULL` today; (c) there is no way for any
> downstream consumer to actually receive these audit events short of a direct
> database read. The write-side guarantee (100% coverage, same-transaction
> atomicity) is real and tested; the "and it reaches an audit sink" half of the
> feature does not exist yet. See `../DEFERRED_ITEMS_PLAN.md` Tier-4 item 4.1.

### 1.3 Actors

| Actor | Role in Feature |
|-------|-----------------|
| `cpt-cf-file-storage-actor-platform-user` | Performs a write operation (create/finalize/bind/patch/delete/transfer/…); the audit row is an automatic, non-optional side effect of their request, not something they explicitly request |
| `cpt-cf-file-storage-actor-cf-gears` | Peer gear / service acting as `actor_kind = "app"`; subject to the identical audit coverage as a human user |

The background cleanup engine (`cpt-cf-file-storage-fr-orphan-reconciliation`,
`cpt-cf-file-storage-fr-retention-policies`) also writes audit rows for its own
sweep-triggered deletions, using a synthetic `actor_kind = "system"`,
`actor_id = Uuid::nil()` identity rather than either actor above — there is no
human or peer-gear caller to attribute those rows to. One inconsistency worth
noting as observed, not corrected here: the `OrphanReconcile` audit rows built
in `cleanup.rs` (`delete_abandoned_pending_version`, `orphan_reconcile_audit`)
also set `tenant_id = Uuid::nil()`, whereas the `RetentionDelete` audit rows
(`expire_file`) correctly carry the real `file.tenant_id`. `audit_outbox` has no
tenant secure-column enforcement (`#[secure(no_tenant, ...)]`), so this does not
bypass any access control, but it does mean a tenant-scoped query over
orphan-reconcile rows by `tenant_id` would miss them.

### 1.4 References

- **PRD**: [PRD.md](../PRD.md)
- **Design**: [DESIGN.md](../DESIGN.md)
- **DECOMPOSITION**: [DECOMPOSITION.md](../DECOMPOSITION.md)
- **Dependencies**: none — the audit trail is a cross-cutting concern threaded
  through every other feature's write path (multipart coordinator, ownership
  transfer, backend migration, retention/cleanup, and the P1 single-shot
  upload/bind/metadata/delete foundation), rather than depending on any one of
  them
- **Related**: `cpt-cf-file-storage-fr-file-events` (the sibling `events_outbox`
  table, same transactional-outbox pattern, same undrained-relay caveat)

## 2. Actor Flows (CDSL)

This feature introduces no new endpoint and no actor-initiated journey of its
own (`ARCH-FDESIGN-NO-002`-style: it rides along inside other features'
flows). The one flow below documents the side effect common to all of them,
from the audited operation's point of view.

### Write Operation Emits an Audit Row

- [x] `p1` - **ID**: `cpt-cf-file-storage-flow-audit-trail-record-write`

**Actor**: `cpt-cf-file-storage-actor-platform-user` (or `cpt-cf-file-storage-actor-cf-gears`)

**Success Scenarios**:
- The actor's write request (create, finalize, bind, metadata patch, delete,
  multipart complete/abort, ownership transfer, backend migration) succeeds;
  exactly one audit row describing it is committed in the same transaction,
  with `outcome = success`

**Error Scenarios**:
- The mutation's own precondition fails (e.g. a stale `If-Match`/CAS version) —
  the entire transaction, including the would-be audit row, rolls back; **no**
  audit row is left behind for the failed attempt (proven by
  `tests/audit_test.rs::failed_metadata_cas_leaves_no_audit_row` and
  `::failed_bind_cas_leaves_no_audit_row`)
- The mutation's CAS predicate finds no matching row at all (e.g. ownership
  transfer racing a concurrent delete) — again no audit row, no event
  (`tests/ownership_test.rs::transfer_ownership_no_row_means_no_audit_and_no_event`)

**Steps**:
1. [x] - `p1` - Actor: issues a write request through any audited operation's normal API path - `inst-audit-actor-request`
2. [x] - `p1` - Service layer: builds an `AuditEntry` via `Self::audit_ok` (or, for the cleanup engine, a synthetic `system`-actor entry built inline) using `cpt-cf-file-storage-algo-audit-trail-build-entry` - `inst-audit-build`
3. [x] - `p1` - Service layer: passes the `AuditEntry` into the same `Store`/repo call that performs the mutation (e.g. `transfer_ownership_atomic`, `rebind_version_backend`, `finalize`, `bind_atomic`, `delete_version`, `delete_file`, `update_metadata`) - `inst-audit-pass-through`
4. [x] - `p1` - `Store`: opens (or reuses) one DB transaction; performs the mutation's own writes, then calls `AuditRepo::insert` inside that **same** transaction - `inst-audit-insert-same-tx`
5. [x] - `p1` - DB: commits both the mutation and the audit row together, or rolls back both together on any failure - `inst-audit-commit-or-rollback`
6. [x] - `p1` - **RETURN** the mutation's normal response to the actor; the audit row is invisible in that response (no audit-related fields in any success payload) - `inst-audit-return`

## 3. Processes / Business Logic (CDSL)

### Build and Persist an Audit Entry Transactionally

- [x] `p1` - **ID**: `cpt-cf-file-storage-algo-audit-trail-build-entry`

**Input**: `SecurityContext` (or a synthetic `system` identity for background
sweeps), an optional `file_id`, an `AuditOperation` variant, a JSON `detail`
payload

**Output**: an `AuditEntry` value (`src/domain/audit.rs`), later persisted by
`AuditRepo::insert` (`src/infra/storage/repo/audit_repo.rs`) as one row in
`audit_outbox`

**Steps**:
1. [x] - `p1` - Extract `tenant_id`/`actor_id` from the `SecurityContext` (`ctx.subject_tenant_id()`, `ctx.subject_id()`), or use `Uuid::nil()` for a background-sweep-originated entry - `inst-buildentry-identity`
2. [x] - `p1` - Compute `actor_kind`: `"app"` if `ctx.subject_type() == Some("app")`, else `"user"`; `"system"` for the cleanup engine's own entries - `inst-buildentry-actor-kind`
3. [x] - `p1` - Select the `AuditOperation` variant matching the mutation (`Create`, `PatchContent`, `PatchMetadata`, `DeleteFile`, `DeleteVersion`, `MultipartComplete`, `MultipartAbort`, `FinalizeVersion`, `RetentionDelete`, `BackendMigrate`, `OrphanReconcile`, `TransferOwnership`) - `inst-buildentry-operation`
4. [x] - `p1` - Build a `detail` JSON object with operation-specific identifiers (e.g. `version_id`, `from_backend`/`to_backend`, `from_owner_id`/`to_owner_id`) — never content bytes - `inst-buildentry-detail`
5. [x] - `p1` - Construct the `AuditEntry` via `AuditEntry::success(...)` (or `::failure(...)`, defined but not currently called by any production call site — every shipped call site only ever records successes, since a failed mutation's transaction rolls back before an audit row would matter) with `occurred_at = now_utc()` - `inst-buildentry-construct`
6. [x] - `p1` - `AuditRepo::insert` maps the entry to an `ActiveModel` (`event_id = Uuid::now_v7()`, `published_at = None`) and calls `secure_insert` under `AccessScope::allow_all()` (the table has no tenant secure-column; `tenant_id` is a plain data column set from the caller's context) - `inst-buildentry-insert`
7. [x] - `p1` - **RETURN** control to the caller once the surrounding transaction commits - `inst-buildentry-return`

## 4. States (CDSL)

### Audit Outbox Row Lifecycle

- [ ] `p2` - **ID**: `cpt-cf-file-storage-state-audit-outbox-row`

**States**: unpublished, published

**Initial State**: unpublished (`published_at IS NULL`)

**Transitions**:
1. [ ] - `p2` - **FROM** unpublished **TO** published **WHEN** a downstream drain/relay process reads the row and marks `published_at` — **this transition never fires in the current codebase; no drain process exists** (Tier-4 item 4.1, NOT IMPLEMENTED) - `inst-st-audit-never-published`

Every `audit_outbox` row shipped today is permanently `unpublished`. The
`published_at` column and its supporting index
(`audit_outbox_unpublished_idx ... WHERE published_at IS NULL`) were added in
anticipation of a drain process that does not exist yet; they are inert schema
today, not dead weight to be removed, since the intent is for item 4.1 to
implement the consumer against this exact shape.

## 5. Definitions of Done

### Transactional Audit-Entry Insertion on Every Write

- [x] `p1` - **ID**: `cpt-cf-file-storage-dod-audit-trail-transactional-write`

The system **MUST** insert exactly one `audit_outbox` row, in the same DB
transaction as the mutation, for every write operation: `create_file`,
`finalize_upload`/`finalize_upload_by_token`, `bind`, `update_metadata`,
`delete_file`, `delete_version`, `complete_multipart_upload`,
`abort_multipart_upload`, `transfer_ownership`, `migrate_backend`, and the
cleanup engine's `RetentionDelete`/`OrphanReconcile`/expired-multipart-session
`MultipartAbort` deletions. A rolled-back mutation (failed CAS/`If-Match`,
CAS predicate matching zero rows) **MUST** leave zero new audit rows.

**Implements**:
- `cpt-cf-file-storage-flow-audit-trail-record-write`
- `cpt-cf-file-storage-algo-audit-trail-build-entry`

**Touches**:
- Gears: `src/domain/audit.rs`, `src/domain/service/mod.rs` (`audit_ok` helper),
  `src/domain/service/write.rs`, `src/domain/service/create.rs`,
  `src/domain/service/read_ops.rs`, `src/domain/service/backend.rs`,
  `src/domain/multipart_service.rs`, `src/domain/cleanup.rs`
- DB Table: `audit_outbox`

### Schema: audit_outbox Table

- [x] `p2` - **ID**: `cpt-cf-file-storage-dod-audit-trail-schema`

The system **MUST** provide an `audit_outbox` table
(`event_id` uuid PK, `tenant_id`, `actor_kind`, `actor_id`, `file_id`
nullable, `operation`, `outcome`, `detail` json, `occurred_at`,
`published_at` nullable) via migration `m20260701_000001_p2_initial`, plus an
index `audit_outbox_unpublished_idx` scoped to `published_at IS NULL` for the
(not-yet-implemented) drain process's eventual query pattern. The table
carries **no tenant secure-column** (`#[secure(no_tenant, resource_col =
"event_id", no_owner, no_type)]`) — application code, not `SecureORM`, is
responsible for populating `tenant_id` correctly.

**Implements**:
- `cpt-cf-file-storage-algo-audit-trail-build-entry`

**Touches**:
- DB Table: `audit_outbox`

### Outbox Drain to a Downstream Sink (NOT IMPLEMENTED)

- [ ] `p2` - **ID**: `cpt-cf-file-storage-dod-audit-trail-relay`

**NOT IMPLEMENTED, no target date.**

The system **SHOULD** eventually drain unpublished `audit_outbox` rows to a
platform audit sink (the same Tier-4 item 4.1 "EventBroker relay" that would
also drain `events_outbox`), marking `published_at` on successful delivery.
**None of this exists today.** Nothing in `cf-gears-file-storage` reads
`audit_outbox` back out except test helpers (`Store::list_audit`,
`AuditRepo::list_for_file`) and ad hoc SQL. This DoD line is recorded
specifically so the gap is tracked as an explicit, acknowledged limitation
rather than silently assumed-done because the write side is fully tested.

**Implements**: (nothing yet — this is the open item)

**Touches**:
- DB Table: `audit_outbox` (read side, not yet built)
- Gears: a future relay/drain component (not yet designed beyond the Tier-4
  plan entry)

## 6. Acceptance Criteria

- [x] `create_file` leaves exactly one `create` audit row (`tests/audit_test.rs::create_file_leaves_one_audit_row`)
- [x] `finalize_upload` (via `put_content`) leaves exactly one `finalize_version` audit row (`::finalize_upload_leaves_audit_row`)
- [x] `bind` leaves exactly one `patch_content` audit row (`::bind_leaves_audit_row`)
- [x] `update_metadata` leaves exactly one `patch_metadata` audit row (`::update_metadata_leaves_audit_row`)
- [x] `delete_file` leaves exactly one `delete_file` audit row (`::delete_file_leaves_audit_row`)
- [x] `delete_version` leaves exactly one `delete_version` audit row (`::delete_version_leaves_audit_row`)
- [x] `complete_multipart_upload` leaves exactly one `multipart_complete` row and exactly one `finalize_version` row (`::multipart_complete_leaves_audit_rows`)
- [x] A failed metadata CAS (stale `expected_meta_version`) leaves **no** new audit row — proves the same-transaction atomicity guarantee (`::failed_metadata_cas_leaves_no_audit_row`)
- [x] A failed bind (stale `If-Match`) leaves **no** new audit row (`::failed_bind_cas_leaves_no_audit_row`)
- [x] `transfer_ownership` leaves exactly one `transfer_ownership` audit row (`tests/ownership_test.rs::transfer_ownership_leaves_audit_row`); a CAS-losing transfer (target row not found) leaves **no** audit row and **no** file event (`::transfer_ownership_no_row_means_no_audit_and_no_event`)
- [x] `migrate_backend` leaves at least one `backend_migrate` audit row on a real migration, and **zero** when the migration is a same-backend no-op (`tests/cleanup_test.rs::migrate_backend_moves_content_and_updates_version_row`, `::migrate_backend_to_same_backend_is_noop`)
- [x] The cleanup engine's retention-expiry sweep leaves a `retention_delete` audit row per expired file, and its abandoned-pending-version reclamation leaves an `orphan_reconcile` row (`tests/cleanup_test.rs`, retention/orphan sweep tests)
- [ ] `audit_outbox` rows are drained/relayed to a downstream platform audit sink — **NOT IMPLEMENTED**; `published_at` is written `NULL` on every insert and never updated by any code path in this repository (Tier-4 item 4.1, no target date; see the caveat in §1.2 and the DoD in §5)
- [ ] The audit trail is queryable through this gear's own REST API — **NOT IMPLEMENTED**; there is no `GET`-style audit endpoint, only direct SQL / test-only repo methods
