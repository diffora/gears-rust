<!-- CONFLUENCE_TITLE: [BSS]: Products — Design Decisions (PriceBook rewrite) -->
<!-- Related: ./DESIGN.md, ./PRD.md, ./design/ | Owners: BSS Product Catalog team -->

# Design Decisions — Products

**Numbering continues from the register on `bss/products-backup` (3a38f0b28), which ends at P-D-183.** Entries below P-D-184 are not in this tree; cite them as "P-D-NNN (backup)". This file is not a registered kit kind: its gates are toc and language.

<!-- toc -->

- [Status board](#status-board)
- [Entries](#entries)

<!-- /toc -->

## Status board

| id | sev | title | status |
|---|---|---|---|
| P-D-184 | H | Usage-type catalog stays a pluggable port; draft-save posture is carried, submit and apply validate the ref | CARRIED from P-D-183 (backup) · 2026-09-24 |
| P-D-185 | H | No Product entity | DECIDED 2026-09-24 · ADR-0001 |
| P-D-186 | M | Categories are a flat list, one per SKU | DECIDED 2026-09-24 · spec §2 decision 12 |
| P-D-187 | M | `sku.name` and `sku.code` unique per tenant | DECIDED 2026-09-24 · spec §4 |
| P-D-188 | H | Live registry references freeze SKU type; no remote count | DECIDED 2026-09-24 · spec §2 decision 17, §4 |
| P-D-189 | H | Retire and type change are fenced against the local registry; fenced SKUs refuse reservations; orphan fences are recoverable | DECIDED 2026-09-24 · spec §2.2, §4, decision 17 |
| P-D-190 | H | Approvals through `bss-approval`: `sku_publish`, `sku_change` (with `effective_from`), `sku_retire`; quorum from settings; author and submitter excluded | DECIDED 2026-09-24 · spec §6 |
| P-D-191 | M | Descriptors bind from dated `sku_version` snapshots; no per-book refreeze | DECIDED 2026-09-24 · spec §2 decision 14, §2.2 |
| P-D-192 | H | Stale units refresh with a new generation; votes name their generation; unit writes use a version, without row locks | DECIDED 2026-09-24 · spec §2.2, §6 |
| P-D-193 | M | Audit rows and the single idempotency store are kept on the new chain | DECIDED 2026-09-24 · spec §3 items 23, 27, §2.2 |
| P-D-194 | H | Products owns reference reservations; pricing reserves before writing and confirms with durable retries; live references and fences exclude each other | DECIDED 2026-09-24 · spec §2 decision 17, §4, §13 |
| P-D-195 | H | The chain refuses a legacy or stale products schema at boot | DECIDED 2026-09-26 · pricing D-423; phase 4 plan rev 2 (Run 4.1) |

## Entries

#### P-D-184 [H] Usage-type catalog stays a pluggable port

Carried from P-D-183 (backup): `UsageTypeCatalog` in `products-sdk` retains resolution and listing,
resolution order and provenance. A registered catalog wins, then the usage-collector adapter, then the
configured local-development catalog or unconfigured mode. Resolution remains resolvability-only.

On draft save, a changed ref is checked when a catalog is configured: a definitive unresolved answer is
400 `USAGE_TYPE_UNRESOLVED`; a catalog non-answer does not block the save. This is the carried save posture,
not a blanket 503 on authoring. Submit validates the proposed metering and `apply` revalidates before
publication or change. Publication requires both `usage_type_ref` and `unit` and fails closed: an
unresolvable ref is `USAGE_TYPE_UNRESOLVED`, an unreachable configured catalog is 503.

**Traceability:** [PRD `fr-sku-metering`](PRD.md#fr-sku-metering); spec §4, §6 and §15
(the usage-type catalog design remains in force).

#### P-D-185 [H] No Product entity

A SKU is the independent catalog definition; it has no Product parent or parent-child lifecycle cascade.
See [ADR-0001](ADR/0001-cpt-cf-bss-products-adr-no-product-entity.md). A bundle is a SKU sold as a Pricing
plan, never a price book entry or plan item; Products stores no bundle composition.

**Traceability:** [PRD `fr-sku-define`](PRD.md#fr-sku-define),
[`fr-sku-bundle`](PRD.md#fr-sku-bundle); spec §3 items 32 and 39, §4.

#### P-D-186 [M] Categories are a flat list

One category per SKU, with `code`, `name`, `is_default`, `sort_order` and `status` (`active | retired`);
category code is unique per tenant. Creation and edits, including rename, are direct operations without
approval. Retirement is refused while any SKU points at the category (`CATEGORY_IN_USE`). A `parent_id`
column is a possible future addition, not part of this model.

**Traceability:** [PRD `fr-category-flat`](PRD.md#fr-category-flat); spec §2 decision 12, §4;
ADR-0001 consequences.

#### P-D-187 [M] SKU name and code unique per tenant

Separate unique indexes enforce `(tenant_id, name)` and `(tenant_id, code)`, with 409 `SKU_NAME_TAKEN`
and `SKU_CODE_TAKEN` on collisions. The previous per-(tenant, brand) Product-name rule went with the
Product entity and brand axis.

**Traceability:** [PRD `fr-sku-define`](PRD.md#fr-sku-define); spec §3 items 5 and 32, §4.

#### P-D-188 [H] Live registry references freeze SKU type

A SKU with a price book entry cannot change type (`SKU_TYPE_FROZEN`, 409). The final reservation protocol makes the
guard broader: any `reserved` or `confirmed` reference, including `plan_item` and `sold_as`, refuses the
type-change fence with the same error. Products checks its registry in the transaction that sets the fence
(P-D-189 and P-D-194); pricing does not supply a remote count. Drafts cannot be priced or reserved and change type freely without a fence.
Published or deprecated type changes use a fence and `sku_change` in one transaction.

**Traceability:** [PRD `fr-sku-type-frozen`](PRD.md#fr-sku-type-frozen); spec §2 decision 17, §2.2, §4.
Decision 17 and the registry rules supersede the earlier remote-count wording in §4 and the Task 5 example.

#### P-D-189 [H] Fenced retire and type change

The fence (`lifecycle = retiring`, or `type_change_pending = true`) is one statement guarded by
`NOT EXISTS (live reference)` in the local registry (P-D-194), within the same transaction and under
serializable isolation on Postgres. Reserved and confirmed rows are live: they refuse retirement with
`SKU_REFERENCED` and type change with `SKU_TYPE_FROZEN`. A fenced SKU refuses new reservations with
409 `SKU_FENCED`; no cross-gear call participates in fence acquisition.

The fence records `fenced_at` and `fence_op_id`. A retried submit finding a fence without a pending unit
resumes by rechecking and submitting. An orphan fence older than configurable `fence_ttl_minutes` is
reverted by the next request on the SKU or `POST /skus/{id}/unfence`; recovery cannot clear a pending
unit's fence. Withdrawal or rejection clears the fence and pending lock in one statement guarded by
the unit id and fence operation id, restoring the pre-fence state.

Apply revalidates the reference environment. A failed retirement check is `APPLY_REFUSED` with reason
`SKU_REFERENCED`; the apply transaction rolls back and the SKU stays `retiring` until withdrawal or
rejection. Pricing also refuses a new price book entry or plan item on a retiring SKU (`SKU_RETIRING`).

**Traceability:** [PRD `fr-sku-retire-fenced`](PRD.md#fr-sku-retire-fenced),
[`fr-sku-type-frozen`](PRD.md#fr-sku-type-frozen), [`fr-sku-lifecycle`](PRD.md#fr-sku-lifecycle),
[`fr-reference-registry`](PRD.md#fr-reference-registry); spec §2 decision 17, §2.2, §4, §6.

#### P-D-190 [H] Approval kinds of this gear

The shared `bss-approval` shape serves `sku_publish`, `sku_change` (with `effective_from`) and
`sku_retire`. Quorum comes from tenant `approval_policy`, with an optional per-kind override, and is
copied into the unit on submit; a missing `'*'` row means quorum 1, fail-safe. There is no materiality
threshold. The submitter and every item's author are excluded from approving (403 `SOD_VIOLATION`),
even with both permissions; a reviewer need not have submit permission. A draft belongs to its author: only its creator edits or deletes it (403 `NOT_DRAFT_AUTHOR` for anyone else), so every item's author is the one who wrote its content (pricing D-404).

Submit validates and conditionally acquires `pending_unit_id` (`ROW_LOCKED_PENDING`, 409, on failure).
Quorum zero still records an approved unit with `decided_at = submitted_at`, no decisions, and the
ordinary audit and events. One reject closes the unit and needs a note; only the submitter may withdraw
a pending unit. Terminal transitions clear pending locks; successful approval keeps `approved_by_unit_id`.
Apply revalidates, refreshes content drift (P-D-192), and rolls back environment failures as `APPLY_REFUSED`.

**Traceability:** [PRD `fr-approval-units`](PRD.md#fr-approval-units),
[`fr-sku-descriptors`](PRD.md#fr-sku-descriptors), [`fr-events`](PRD.md#fr-events);
spec §2 decision 8, §6, §14; Task 5 specifies the missing-policy fail-safe.

#### P-D-191 [M] Descriptors bind from versions

Publish and every applied `sku_change` append a durable
`sku_version (sku_id, published_version, effective_from, snapshot)`. A change carries `effective_from`,
defaulting to today; publication is effective immediately. Pricing reads
`GET /skus/{id}/versions?as_of=<date>` for the version in force at a period's start. Earlier bindings keep
their descriptors; no per-book approval or refreeze action exists.

A change earlier than the latest version's date is refused with 409 `VERSION_ORDER`. Equal dates are
allowed and the higher `published_version` wins; `(sku_id, effective_from)` is not unique. A date before
the first version returns 404 `NO_VERSION_IN_FORCE`. The latest SKU row may be future-effective, so
consumers use the dated version read. The wire spelling is `as_of`, following the current spec and kit
constraints; the older `asOf` spelling in the Task 5 example and PRD is superseded here.

**Traceability:** [PRD `fr-sku-descriptors`](PRD.md#fr-sku-descriptors),
[`fr-sku-versions`](PRD.md#fr-sku-versions), [`fr-read-model`](PRD.md#fr-read-model);
spec §2 decision 14, §2.2, §4, §7.1–§7.2.

#### P-D-192 [H] Generations, not locks

A unit has `generation` and `version`. Approve and reject name the generation reviewed; another
generation is refused with 400 `GENERATION_MISMATCH` and the current generation. An actor votes at most
once per generation (409 `DUPLICATE_VOTE`). Before a vote counts, re-collection compares proposed business
content and effective date, excluding lock/version metadata. A fingerprint mismatch rewrites items,
snapshot and hash, increments generation, marks earlier decisions stale and commits the refresh, returning
400 `UNIT_STALE` with the new generation. Reviewers vote again on the refreshed content.

Every unit write is conditional on `version`; a lost race returns 409 `UNIT_CONTENDED` for client retry.
SecureORM offers no row locks, and none are used. An environment failure at apply is `APPLY_REFUSED` and
rolls back, rather than committing a content refresh.

**Traceability:** [PRD `fr-approval-units`](PRD.md#fr-approval-units),
[`fr-concurrency-idempotency`](PRD.md#fr-concurrency-idempotency); spec §2.2, §6.

#### P-D-193 [M] Audit and idempotency stay

Append-only audit rows and the tenant-scoped idempotency store are re-created on the new migration chain
from the backup migrations' DDL, not dropped. Submission is audited; every terminal unit transition,
including rejection, withdrawal and quorum-zero approval, writes its audit row and `ApprovalUnitDecided`
with the state change. Successful apply's domain events use the same transaction. Operator force-release
is audited and emits `ReferenceForceReleased`; rolled-back apply emits no successful terminal event.

The single replay store is keyed `(tenant, endpoint, client_key)`, retained for 24 hours and checked
before any fence or unit work. POST accepts an optional `Idempotency-Key`; the approval unit has no
separate idempotency key (spec §2.2 supersedes the older §6 schema). Reserve also has logical-reference
idempotency independently of client keys. PATCH requires `If-Match`; stale revisions return `STALE_REVISION`.

**Traceability:** [PRD `fr-concurrency-idempotency`](PRD.md#fr-concurrency-idempotency),
[`fr-events`](PRD.md#fr-events), [`fr-reference-registry`](PRD.md#fr-reference-registry),
[`nfr-audit`](PRD.md#nfr-audit); spec §3 items 23 and 27, §2.2, §4, §6.

#### P-D-194 [H] The reference registry replaces the remote count

Products owns `sku_reference`, with states `reserved | confirmed | released` and kinds
`price_book_entry | plan_item | sold_as`. Within tenant scope, uniqueness is over live rows only per
`(owner_gear, ref_kind, ref_id)`. A new attempt after release gets a fresh id; released rows remain and
are never reactivated. `POST /skus/{id}/references/reserve` creates with 201 or returns 200 and the
existing reservation for the same live logical reference. A fenced SKU refuses new reservations with
409 `SKU_FENCED`; any reserved or confirmed reference blocks the fence in the same database transaction.
A reservation left unconfirmed keeps counting until released, without expiry-based exemption.

Pricing follows reserve → re-read SKU → write object, reservation id and confirmation work in one
Pricing transaction → confirm with durable retry. This includes sold-as references. If Products is
unavailable before reserve, Pricing returns 503 `REGISTRY_UNAVAILABLE` and writes nothing. If confirmation
fails after commit, Pricing keeps `confirmation_pending = true` and retries; a confirmation timeout
never justifies release. `POST /references/{id}/confirm` returns 200 for an already-confirmed row;
confirming a released row returns 409 `REFERENCE_RELEASED`.

Release is the owner gear's act after durable cancellation or deletion: definite rollback leads to
durable cancellation then release, deletion to removal then release. Operator `DELETE /references/{id}`
requires `force: true` and a reason, records the actor/reason and audit, and emits `ReferenceForceReleased`
so the owner can verify and re-reserve if its object still exists. Products cannot detect release beneath
a live owner object; the protocol forbids that misuse, it does not detect it.

The `SkuReferences` port to pricing is gone. `GET /skus/{id}/references` reads this registry and returns
rows and live counts grouped by owner and kind; the SKU card exposes abandoned reservations for inspection
and explicit release. No remote count sits on a fence.

**Traceability:** [PRD `fr-reference-registry`](PRD.md#fr-reference-registry),
[`fr-read-model`](PRD.md#fr-read-model), [`fr-sku-bundle`](PRD.md#fr-sku-bundle),
[`fr-events`](PRD.md#fr-events); spec §2 decision 17, §2.2, §4, §13.

#### P-D-195 [H] The chain refuses a legacy or stale schema

The chain starts with one guard migration, `m0000_products_refuse_a_legacy_or_stale_schema`, named to sort
first under the toolkit runner's name sort: before the coordination, broker and outbox migrations
(`m0001_…`, `m001_…`) and before `m20260925_000001`. It is pending on every database that predates
phase 4, so it runs there once, before anything of the gear is created, and it creates nothing. It reads
the catalog only (`sqlite_master` and `table_info` on SQLite; `information_schema` and `pg_constraint` on Postgres, tables
in schema `bss`) and refuses, so that boot fails naming the gear, when it finds:

- a legacy table: one of the 35 `products_*` tables that the legacy chain creates (`bss/products-backup`,
  `m20260829_000001` to `m20260922_000031`, 40 tables) and today's chain does not. `products_sku`,
  `products_category`, `products_audit_log`, `products_idempotency` and `products_approval_decision`
  (which `bss_approval::ddl` creates with prefix `products_`) exist in both and are not evidence. The set
  is a constant in the guard; a test proves it disjoint from every table a fresh chain creates.
- a stale shape: `products_sku_reference` whose `ref_kind` CHECK does not admit `price_book_entry`. The
  phase 2 rename edited `m20260925_000006` in place, from `price` to `price_book_entry`, and
  `CREATE TABLE IF NOT EXISTS` keeps the old CHECK on a database migrated before it.
- the legacy shape of a table that both chains create: `products_category` or `products_sku` without the
  column `code`. A clean-up that drops only the tables a refusal names leaves them, and
  `m20260925_000001`/`000002`'s `CREATE TABLE IF NOT EXISTS` would keep them and then fail on their `code`
  indexes with a raw SQL error (pricing phase 4 review F2, fix run 8).

The refusal reads `bss-products: this database holds a <legacy|stale> bss-products schema (<what was
found>); PriceBook does not migrate it — start from an empty data root / empty bss-products tables`. A
fresh database passes, and so does a database migrated by today's chain: the guard is pending there
once and finds nothing. The pricing gear carries the same guard over its own tables (pricing D-423).

**Traceability:** [PRD `nfr-two-backends`](PRD.md#nfr-two-backends); pricing D-423; phase 4 plan rev 2,
Run 4.1; plan review H1, M1 and L6.
