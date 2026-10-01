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
| P-D-196 | M | A SKU's category is optional; an omitted category stays null, with no default fallback | DECIDED 2026-09-26 · Owner, 2026-09-26; phase 5 plan rev 2 |
| P-D-197 | M | SKU reads carry pricing's usage through a port that pricing fills; the usage is information and never a fence input | DECIDED 2026-09-26 · Owner, 2026-09-26; phase 5 plan rev 2; pricing D-428 |
| P-D-198 | M | The replay store's mechanics (twin of pricing D-429) | DECIDED 2026-09-27 · Carried from P-D-29, P-D-30, P-D-38, P-D-42, P-D-49 (backup `3a38f0b28`); decisions cleanup, owner 2026-09-27; amends P-D-193 |
| P-D-199 | M | Events ride the toolkit outbox and the broker SDK producer | DECIDED 2026-09-27 · Carried from P-D-01, P-D-22, P-D-47 (backup `3a38f0b28`); decisions cleanup, owner 2026-09-27 |
| P-D-200 | M | The audit log is append-only with a reserved sealing seam (twin of pricing D-433) | DECIDED 2026-09-27 · Carried from P-D-08, P-D-28, P-D-46, P-D-118 (backup `3a38f0b28`); decisions cleanup, owner 2026-09-27 |
| P-D-201 | M | The request digest | DECIDED 2026-09-27 · Carried from P-D-29, P-D-34 (backup `3a38f0b28`); decisions cleanup, owner 2026-09-27 |
| P-D-202 | L | A validation refusal lists every violation of its stage | DECIDED 2026-09-27 · Carried from P-D-33, P-D-37 (backup `3a38f0b28`); decisions cleanup, owner 2026-09-27 |
| P-D-203 | M | The usage-type resolve is bounded and runs outside the transaction | DECIDED 2026-09-27 · Carried from P-D-121 (backup `3a38f0b28`); decisions cleanup, owner 2026-09-27; amends P-D-184 |
| P-D-204 | L | Authz label schemas are registered at boot | DECIDED 2026-09-27 · Carried from P-D-134 (backup `3a38f0b28`); decisions cleanup, owner 2026-09-27 |
| P-D-205 | M | The approval policy is read with a content `ETag` and written under `If-Match` | DECIDED 2026-09-27 · Owner, 2026-09-27; phase 6 plan rev 2 |
| P-D-206 | M | A never-published draft is deleted by its author, never retired | DECIDED 2026-09-27 · Owner, 2026-09-27; phase 6 plan rev 2; amends P-D-190 |
| P-D-207 | H | Usage types are read as the caller: a denial is 403 `USAGE_TYPE_FORBIDDEN`, and products serves the picker | DECIDED 2026-09-27 · Owner, 2026-09-27; phase 6 plan rev 2 (owner option b); amends P-D-184, P-D-203 |
| P-D-208 | M | A retired SKU no longer keeps its category in use; retiring a retired category is `CATEGORY_RETIRED` | DECIDED 2026-09-27 · Owner, 2026-09-27; phase 6 plan rev 2; amends P-D-186 |
| P-D-209 | L | No tenant settings door: the fence TTL is the deployment setting `fence_ttl_minutes` | DECIDED 2026-09-27 · Owner, 2026-09-27; phase 6 plan rev 2; amends P-D-189 |
| P-D-210 | M | The SKU list pages on the toolkit's OData, with a literal case-insensitive `q` | DECIDED 2026-09-27 · Owner, 2026-09-27; phase 6 plan rev 2 |
| P-D-211 | M | The SKU list's tab counts: `GET /skus/counts` | DECIDED 2026-09-27 · Owner, 2026-09-27; phase 6 plan rev 2 |
| P-D-212 | M | The SKU list filters on pricing's usage (`priced`, `in_plan`) through the port's sets; a filter pricing cannot answer fails the read | DECIDED 2026-09-27 · Owner, 2026-09-27; phase 6 plan rev 2; amends P-D-197 |
| P-D-213 | M | A SKU's history: every audit row on a SKU carries the lifecycle move its act made, and `GET /skus/{id}/history` reads them | DECIDED 2026-09-27 · Owner, 2026-09-27; phase 6 plan rev 2; amends P-D-189, P-D-200; amended by P-D-219 |
| P-D-214 | L | SKU versions answer one shape each: the history an array, the version in force at `versions/as-of?date=` | DECIDED 2026-09-27 · Owner, 2026-09-27; phase 6 plan rev 2 |
| P-D-215 | M | Category reads: `GET /categories/{id}`, a `sku_count` on every read from one grouped count, and the list on the toolkit's OData | DECIDED 2026-09-27 · Owner, 2026-09-27; phase 6 plan rev 2 |
| P-D-216 | M | An approval-policy override can be reset; the default cannot be deleted (twin of pricing D-435) | DECIDED 2026-09-27 · Owner, 2026-09-27; phase 6 plan rev 2 |
| P-D-217 | M | Closed sets are enums on the responses; requests keep strings and their codes (twin of pricing D-439) | DECIDED 2026-09-27 · Owner, 2026-09-27; phase 6 plan rev 2; amended by the phase 9 review (C, fix run 9.5d-1) |
| P-D-218 | M | Making a category the default moves the default in one write; a lost race is 409 `CATEGORY_DEFAULT_TAKEN` | DECIDED 2026-09-28 · Owner, 2026-09-28; amended by P-D-220 |
| P-D-219 | M | The submitter's note travels with the approval unit (twin of pricing D-445) | DECIDED 2026-09-28 · Owner, 2026-09-28; phase 7 plan rev 2; amends P-D-213; its Pricing bullet amended by pricing D-464 |
| P-D-220 | M | A retired category is never the default; retiring the default clears it | DECIDED 2026-09-28 · Owner, 2026-09-28; amends P-D-218 |
| P-D-221 | M | The outbox wakes its sequencer after the commit (twin of pricing D-455) | DECIDED 2026-09-29 · Main sync of 2026-09-29 (toolkit-db 2bfc76aec); pricing phase 8 plan rev 2 (run 8.2b) |
| P-D-222 | H | The registry trusts pricing's system actor in-process only; no REST door serves that actor | DECIDED 2026-09-29 · Owner, 2026-09-29 (dispositions O1, "ok"); whole-branch review RS-02 (fix run W1b); second review of W1b M1 (fix run W1c); keeps pricing D-424 |
| P-D-223 | M | A refusal keeps its class and names its resource | DECIDED 2026-09-29 · Whole-branch review RS-06, RS-07, RS-09, RS-25, RS-32 and W1a's `UnitNotFound` note (fix run W1b) |
| P-D-224 | M | The approval-unit list pages and reads its page set-based (twin of pricing D-458) | DECIDED 2026-09-29 · Owner, 2026-09-29 (dispositions O2, "ok"); whole-branch review RS-03 (fix run W1b); amended by P-D-227, P-D-228 |
| P-D-225 | M | Every text a request writes has an explicit length cap (twin of pricing D-457) | DECIDED 2026-09-29 · Whole-branch review RS-10, RS-11, RS-37, RS-38 (fix run W1b); the dispositions' "Length caps" |
| P-D-226 | M | The SDK's SKU types serialize as the wire carries them | DECIDED 2026-09-30 · Whole-branch review RS-22, RS-23, RS-24 (fix run W1b) |
| P-D-227 | M | The approval units are counted by state and kind and list newest first on request (twin of pricing D-470) | DECIDED 2026-09-30 · Owner, 2026-09-30 (the approvals option 1, "ok"); pricing phase 9 plan rev 2 (decision 10; plan review M4, L11); amends P-D-224; amended by the phase 9 review (C, R32; fix run 9.5d-1; I, fix run 9.5d-2); amended by P-D-250 |
| P-D-228 | M | A unit says whether its reader may approve it (twin of pricing D-471) | DECIDED 2026-09-30 · Owner, 2026-09-30 (validation 3 item 4, "ok"); pricing phase 9 plan rev 2 (decision 11; W2; plan review H1, M2); amends P-D-224; amended by the phase 9 review (E, fix run 9.5d-1) |
| P-D-229 | H | A derived usage meter is a catalog declaration that Rating evaluates | DECIDED 2026-10-01 · Owner, 2026-10-01 (who computes a cloudlet from RAM and CPU); supersedes the PriceBook spec §3 item 11 disposition for derived meters; rating T-D-39 |
| P-D-248 | H | A retire under review keeps the SKU's lifecycle; `retire_pending` is the fence | DECIDED 2026-10-01 · Owner, 2026-10-01; phase 9 plan rev 4 run 9.8d; amends P-D-189, P-D-208, P-D-211, P-D-213 |
| P-D-249 | H | A lifecycle change honours its date | DECIDED 2026-10-01 · Owner, 2026-10-01; phase 9 plan rev 4 run 9.8d; amends P-D-191 |
| P-D-250 | M | The approval units answer the approvals inbox through this gear's own doors (twin of pricing D-490) | DECIDED 2026-10-01 · Owner, 2026-10-01 (option A, "yes, A, agreed", then "write the plan"; Run 2 started before 9.5d-2); approvals inbox plan rev 2 (Run 2; design 1 and 3; plan review H1, H3, H4, M1, M2, M5, L1); amends P-D-227 |

## Entries

#### P-D-184 [H] Usage-type catalog stays a pluggable port

Carried from P-D-183 (backup): `UsageTypeCatalog` in `products-sdk` retains resolution and listing,
resolution order and provenance. A registered catalog wins, then the usage-collector adapter, then the
configured local-development catalog or unconfigured mode. Resolution remains resolvability-only.

On draft save, a changed ref is checked when a catalog is configured: a definitive unresolved answer is
400 `USAGE_TYPE_UNRESOLVED`; a catalog non-answer does not block the save. This is the carried save posture,
not a blanket 503 on authoring. Submit validates the proposed metering and `apply` revalidates before
publication or change. Publication requires both `usage_type_ref` and `unit` and fails closed: an
unresolvable ref is `USAGE_TYPE_UNRESOLVED`, an unreachable configured catalog is 503. P-D-207 amends this
entry: a catalog that refuses the caller is 403 `USAGE_TYPE_FORBIDDEN`, and the picker is `GET /usage-types`.

**Traceability:** [PRD `fr-sku-metering`](PRD.md#fr-sku-metering); spec §4, §6 and §15
(the usage-type catalog design remains in force, with the picker's path and gate changed by P-D-207).

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
column is a possible future addition, not part of this model. P-D-196 amends this entry: the category of a
SKU is optional, so a SKU has at most one category. P-D-208 amends it again: only a SKU that is not retired
keeps a category in use.

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
reverted by the next request on the SKU or `POST /skus/{id}/unfence` (the TTL is a deployment setting, P-D-209); recovery cannot clear a pending
unit's fence. Withdrawal or rejection clears the fence and pending lock in one statement guarded by
the unit id and fence operation id, restoring the pre-fence state. P-D-213 amends this entry: an orphan fence
the maintenance reverts is the system's act, with its own audit row `sku.fence_expired`.

Apply revalidates the reference environment. A failed retirement check is `APPLY_REFUSED` with reason
`SKU_REFERENCED`; the apply transaction rolls back and the SKU stays `retiring` until withdrawal or
rejection. Pricing also refuses a new price book entry or plan item on a retiring SKU (`SKU_RETIRING`).

P-D-248 amends this entry: the fence is `retire_pending`, not a `retiring` lifecycle. The SKU keeps
`published` or `deprecated` until apply sets `retired` and clears the flag. Reject, withdraw, unfence
and the orphan recovery clear the flag and restore nothing. A new reservation on it is still
`SKU_FENCED`.

**Traceability:** [PRD `fr-sku-retire-fenced`](PRD.md#fr-sku-retire-fenced),
[`fr-sku-type-frozen`](PRD.md#fr-sku-type-frozen), [`fr-sku-lifecycle`](PRD.md#fr-sku-lifecycle),
[`fr-reference-registry`](PRD.md#fr-reference-registry); spec §2 decision 17, §2.2, §4, §6.

#### P-D-190 [H] Approval kinds of this gear

The shared `bss-approval` shape serves `sku_publish`, `sku_change` (with `effective_from`) and
`sku_retire`. Quorum comes from tenant `approval_policy`, with an optional per-kind override, and is
copied into the unit on submit; a missing `'*'` row means quorum 1, fail-safe. There is no materiality
threshold. The submitter and every item's author are excluded from approving (403 `SOD_VIOLATION`),
even with both permissions; a reviewer need not have submit permission. A draft belongs to its author: only its creator edits or deletes it (403 `NOT_DRAFT_AUTHOR` for anyone else), so every item's author is the one who wrote its content (pricing D-404). The delete is P-D-206's.

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
defaulting to today; publication is effective immediately. P-D-249 amends the lifecycle half: a change
whose `effective_from` is after today stores `lifecycle_next` and leaves `lifecycle` until that date. Pricing reads
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

#### P-D-196 [M] A SKU's category is optional

Amends P-D-186: a SKU has at most one category. `category_id` is optional on `POST /skus`; an omitted or
null `category_id` is stored as null, and there is no fallback to the tenant's `is_default` category. The
draft `PATCH` clears it with an explicit `null` and leaves it unchanged when the field is omitted. A
`sku_change` sets or clears it: the unit item's `before` and `after`, the applied `sku_version` snapshot
and `SkuChanged.changed` show it. A set category is still resolved in tenant scope and must be active (a
missing one is 404, a retired one 409 `CATEGORY_RETIRED`). In `products-sdk`, `Sku.category_id` and
`SkuContent.category_id` are `Option<Uuid>`, and the wire carries `null`. No event payload carries the
category itself.

Browse by a category (`GET /skus?category=`) matches only the SKUs in that category, so a SKU without a
category never matches it; an unfiltered list includes it. The retirement of a category and its in-use
check count only the SKUs that point at it; a SKU without a category never blocks a retirement.

The chain is deployed, so the change is the forward migration `m20260925_000007_sku_category_optional`:

- Postgres: `ALTER TABLE bss.products_sku ALTER COLUMN category_id DROP NOT NULL`.
- SQLite cannot drop a NOT NULL in place. The toolkit runner runs each `up()` in a transaction, where
  `PRAGMA foreign_keys=OFF` has no effect, so the migration rebuilds the family and uses no PRAGMA: a new
  `products_sku` (only `category_id` is nullable), new `products_sku_version` and `products_sku_reference`
  against it, all rows copied, the old children dropped first, then the old parent, then the renames. Then
  every index (with the partial unique `uq_products_sku_reference_live`) and the two append-only triggers
  are recreated with their original text.
- `down()` is an explicit irreversible error.

Products has no schema golden. The proof is a structural comparison on both dialects, through the real
runner, from a database migrated by the chain before this migration, with SKUs, versions and references
seeded: only `category_id`'s NOT NULL changes, and every row survives. The `category=none` browse filter
this entry owed is the list's `$filter=category_id eq null` (P-D-210).

**Traceability:** [PRD `fr-category-flat`](PRD.md#fr-category-flat), [`fr-sku-define`](PRD.md#fr-sku-define);
DESIGN §3.1, §3.7; slice 01 §5, slice 02; phase 5 plan rev 2 (Run 5.1); plan review H1, M8, L9 and L13.

#### P-D-197 [M] SKU reads carry pricing's usage through a port that pricing fills

`products-sdk` gains the port `SkuUsageV1` (module `sku_usage`): `usage(ctx, tenant, sku_ids) ->
Result<Vec<SkuUsage>, CanonicalError>`, with `SkuUsage { sku_id, entries, currencies, prices { approved,
pending, draft }, plans }`. Pricing D-428 defines each count: `plans` is distinct across the SKU's entries,
never a sum of their counts; `currencies` are sorted; each requested id is answered once; an unknown SKU, a SKU
of another tenant and a bundle SKU answer zeros. Pricing implements the port and registers it in the
`ClientHub` at its init as `dyn SkuUsageV1`. Products resolves it at each read and not at its own init, because
the two gears boot in either order (pricing resolves its reference registry key the same way). The port types
carry no serde; the REST DTOs of this gear map them.

`GET /skus` and `GET /skus/{id}` carry `usage`: each list item has it next to the SKU's fields, and the card has
it beside `sku` and `references`. The list asks the port once per page, with the ids of the page; the card asks
for its one id. `usage` is `null` when no port is registered, when the port refuses the caller (403: the caller
has no pricing `price_book_entry:read`), and when it cannot answer (any error, or a call that does not finish).
The SKU read never fails because of the port: it calls the port on a task of its own, after its own reads, and
outside any transaction of this gear. It waits two seconds at most; a call still running then has not finished,
and it is aborted, as is a call whose read ends first (its client went away).

The usage is information for the SKUs screen: "N prices" with the currency chips, "unpriced", "N plans"; a
bundle shows "by plan" from its type. It never takes part in a fence, a retirement or a type change: those stay
on the local registry (P-D-188, P-D-194), and no remote count sits on a fence. The card's `references` stay
the local registry's counts.

P-D-212 amends this entry for one case: the list filters on the same facts (`priced`, `in_plan`) through the
port's `usage_sets`, and there a port that refuses or cannot answer fails the read instead of leaving `null`.

**Source:** Owner, 2026-09-26 (option 1: the counts on the entry and on the SKU); phase 5 plan rev 2; pricing
D-428.

#### P-D-198 [M] The replay store's mechanics

Amends P-D-193: the retention is configured, no longer a fixed 24 hours. A key's row in `products_idempotency`
is `claimed` or `answered`, and a CHECK ties the response pair to the state. The claim INSERT, on the
transaction that writes the act, is the at-most-once gate; `endpoint` is the concrete resource path, never the
route template; no in-flight deadline exists. A door authorizes before it looks up or claims a key, so a
denied caller consumes none. The answered row stores the status and body the caller was told, so a replay
reads no other row. An answer is stored only when its transaction commits: a refusal that rolls back takes the
claim with it and frees the key, and a committed refusal (the 400 `UNIT_STALE` after a refresh, P-D-192) is
stored and replays. Expiry is judged at claim time: an expired row is taken over by a compare-and-swap on the
`expires_at` that was read, and the loser answers `IDEMPOTENCY_KEY_IN_FLIGHT` having executed nothing. The
loser may even carry a different payload from the winner, and is still refused in-flight rather than for the
mismatch, since its transaction never compared the two: it read the expired holder's digest, never the
winner's. Apart from that loser, a matching live `claimed` row is `IDEMPOTENCY_KEY_IN_FLIGHT`, and a digest
mismatch is `IDEMPOTENCY_CONFLICT` in either state. The retention is `idempotency_retention_hours` (default
24), clamped to at least 24 hours and at most ten years. `entity_ref` is carried in the DDL and always NULL:
no products door binds an op. Pricing runs the same store (pricing D-429), with two differences: pricing binds
POST entry's and POST plan item's claim to its durable reference op, never takes over a bound claim and keeps
the op's late answer 24 hours from the answer; and pricing's retention is a fixed 24 hours.

**Source:** Carried from P-D-29, P-D-30, P-D-38, P-D-42, P-D-49 (backup `3a38f0b28`); decisions cleanup, owner 2026-09-27.

#### P-D-199 [M] Events ride the toolkit outbox and the broker SDK producer

Products' events are broker `TypedEvent`s in the broker-native envelope, not CloudEvents. They are enqueued
into the toolkit outbox (prefix `bss_products_outbox`, queue `bss_products_events`, 8 partitions) on the
transaction of the state change, and the facility's own migrations create its tables. `Gear::init` binds the
event-broker SDK's outbox producer when an `EventBrokerApi` is registered; a broker that is present but refuses
fails the boot. Without one, a holding processor keeps every message queued, and `require_broker = true` turns
that fallback into a boot failure. Each event carries the ambient W3C traceparent when a span has one.

**Source:** Carried from P-D-01, P-D-22, P-D-47 (backup `3a38f0b28`); decisions cleanup, owner 2026-09-27.

#### P-D-200 [M] The audit log is append-only with a reserved sealing seam

`products_audit_log` refuses every DELETE by trigger and admits one UPDATE: `unsealed` to `sealed`, supplying
`chain_id`, `seq` and `row_hash` (`prev_hash` NULL only on a segment head) with every record column unchanged.
The gear writes `seal_state = unsealed` with the four seal columns NULL on every row and never seals, chains
or verifies: sealing is a platform capability the columns are reserved for. The key is a surrogate `audit_id`,
because `seq` is NULL until a row is sealed. No `REVOKE UPDATE, DELETE` is issued (a deployment role the
migration does not own; SQLite has none). `correlation_id` is `text`: products writes NULL on every row,
because this gear establishes no request correlation; pricing writes its edge id, or on a rereserve op's rows
the id the op minted, and never NULL (pricing D-431). `error_code`, `attempted_key`, `session_id` and
`ceremony_ref` are carried in the DDL and written NULL. Pricing has the same table shape (pricing D-433).
P-D-213 amends this entry: the rows carry `from_lifecycle` and `to_lifecycle`, and the seal keeps both unchanged
as well.

**Source:** Carried from P-D-08, P-D-28, P-D-46, P-D-118 (backup `3a38f0b28`); decisions cleanup, owner 2026-09-27.

#### P-D-201 [M] The request digest

A key's payload hash is SHA-256 (`aws-lc-rs`) over the canonical rendering of the parsed request body: object
keys sorted at every depth, no insignificant whitespace, numbers without trailing zeroes (`1` and `1.0` hash
alike), strings carried verbatim, arrays in the order received. A member the request omits is omitted, so an
omitted field and an explicit `null` hash differently. Headers, `If-Match` included, are outside the hash.
Pricing's hash is pricing D-396's.

**Source:** Carried from P-D-29, P-D-34 (backup `3a38f0b28`); decisions cleanup, owner 2026-09-27.

#### P-D-202 [L] A validation refusal lists every violation of its stage

Validation is staged, and the first stage that fails answers; a later stage does not run. The shape parse
refuses first: the body's deserialization, and on `POST /skus` `NewSku::try_from`, which stops at the first
unknown token (`type`, then `billing_timing`). Then the door's checks (`validate_new` on `POST /skus`) collect
every violation of their stage into one report, and the refusal is one 400 that carries all of them, each with
its subject, detail and code, the first collected first. On the SKU draft doors the usage-type resolve
(P-D-184) runs last. So `POST /skus` with an unknown `type` and a blank `code` is told only of the `type`. A
report that holds `USAGE_TYPE_UNAVAILABLE` answers 503 instead, because an outage is retryable. Refusals write
no audit row.

**Source:** Carried from P-D-33 (backup `3a38f0b28`: the pipeline stops at the first failing phase and
collects violations within it) and P-D-37 (backup `3a38f0b28`); decisions cleanup, owner 2026-09-27.

#### P-D-203 [M] The usage-type resolve is bounded and runs outside the transaction

Amends P-D-184. The usage-collector adapter bounds each resolve by `usage_type_resolver_timeout_ms`
(configuration, default 2000); a call that outlives it is `Unavailable`, and a zero value is refused at
boot. Submit and apply resolve the SKU's `usage_type_ref` before their transaction opens, so their 503 holds
no lock and claims no key; a draft save asks too, and only a definite unresolved answer refuses it
(P-D-184). The bound is read from configuration, never inlined.

**Source:** Carried from P-D-121 (backup `3a38f0b28`); decisions cleanup, owner 2026-09-27.

#### P-D-204 [L] Authz label schemas are registered at boot

`Gear::init` registers a stub type-schema for every authz label (`authz_label_type_schemas`) with the
types-registry, so RBAC role definitions can target the gear's labels. A `TypesRegistryClient` missing from
the `ClientHub`, or any refused registration, fails the boot.

**Source:** Carried from P-D-134 (backup `3a38f0b28`); decisions cleanup, owner 2026-09-27.

#### P-D-205 [M] The approval policy is read with a content `ETag` and written under `If-Match`

DESIGN §3.3 already said "Policy PUT remains If-Match only"; the doors did not (validation D1: last write
won). `GET /approval-policy` answers a strong `ETag`: the first eight bytes of the SHA-256 of the policy's
canonical rendering (P-D-201's rendering of `{ default_quorum, overrides }`), as a quoted decimal. The policy
is a set of `(kind, quorum)` rows with no revision column, so its tag is its content, as pricing's policy tag
is (`policy_tag`); a write to any kind moves it. `PUT /approval-policy` requires that tag as `If-Match`: a
missing or malformed header (the wildcard, a weak tag, a list, a non-decimal) is 400 `VALIDATION` on
`If-Match`; a tag that no longer matches is 409 `STALE_REVISION`. The comparison reads the policy inside the
write's transaction (serializable on Postgres), so of two writers holding one tag exactly one wins. The PUT
answers the new policy with its new tag. Authorization is judged first: a caller without `products:settings`
is 403 before any 400. A refused write writes no audit row.

Breaking: every caller of the policy PUT sends `If-Match` (the gears-rust e2e in this run; vhp-core's
`set_products_quorum` fixture in phase 6.6; the deploy note).

**Source:** Owner, 2026-09-27; phase 6 plan rev 2 (validation D1, plan review M4).

#### P-D-206 [M] A never-published draft is deleted by its author, never retired

A draft cannot be retired (the fence takes only `published` or `deprecated`, P-D-189), and slice 03 said it
could (validation D3). A never-published draft is deleted instead: `DELETE /skus/{id}` when the SKU is
`draft` with `published_version = 0` and no pending unit, by its author only, like the draft PATCH (403
`NOT_DRAFT_AUTHOR`, P-D-190, pricing D-404), under `If-Match` with the SKU's revision. It answers 204 and
writes an audit row `sku.delete` whose subject is the SKU. Refusals, in the draft PATCH's order: 409
`SKU_NOT_DRAFT` (published once, or not a draft), 409 `ROW_LOCKED_PENDING`, 403 `NOT_DRAFT_AUTHOR`, 409
`STALE_REVISION`; then 409 `SKU_REFERENCED` if the registry holds any row naming the SKU. None can today: a
draft admits no reservation (`reservation_allowed`), and the guard is asserted by a test that seeds one past
the door. The delete is one conditional statement guarded by the same predicate, so a concurrent submit or
edit loses to it or wins against it, never both.

Only the head row goes. Its audit rows stay (append-only, P-D-200). A draft owns no version rows and no
references. A rejected or withdrawn unit that named it stays: `GET /approval-units/{id}` answers it with
`impact_live: null` instead of 404, and the queue still lists it. The code and name are free again (P-D-187).
A replay of the create's `Idempotency-Key` still answers the stored 201 of the deleted id (P-D-198's replay
reads no other row); documented, not changed.

**Source:** Owner, 2026-09-27; phase 6 plan rev 2 (validation D3 and ask 5, plan review M1).

#### P-D-207 [H] Usage types are read as the caller: a denial is 403 `USAGE_TYPE_FORBIDDEN`, and products serves the picker

Amends P-D-184 and P-D-203. Owner option b: products has no system actor for the usage-type catalog; it resolves
and lists usage types with the caller's security context, as it did. A system actor would not be authorized
on the stand (vhp-core's PDP trusts only `am.system` and `rms.system`; plan review H2).

- `UsageTypeAnswer` gains `Forbidden`. The collector adapter maps the collector's `PermissionDenied` to it,
  where it answered `Unavailable` before (validation D5); the PDP's reason stays in the operator log.
- Submit and approve answer `Forbidden` with 403 `USAGE_TYPE_FORBIDDEN` before their transaction opens, so
  nothing is recorded and no key is claimed; `Unavailable` stays 503 `USAGE_TYPE_UNAVAILABLE`. A publish
  report that carries `USAGE_TYPE_FORBIDDEN` answers 403 too. A draft save keeps P-D-184's posture: only a
  definite unresolved answer refuses it, so a denial does not block the save.
- `GET /bss-products/v1/usage-types?q&kind&limit&cursor` mounts `UsageTypeCatalog::list` under the products
  SKU-author grant (`sku × author`): `{ source, items [{ gts_id, kind, metadata_fields }], page_info {
  next_cursor, prev_cursor, limit } }`, `source` being the catalog's provenance. `limit` defaults to 50 and is
  clamped at 200; 0 or a non-integer is 400. A catalog that refuses the caller is 403, an unconfigured one 501,
  an unreachable one 503, an empty configured one 200 with no items (the 09-22 design's §4).
- `q` is a case-insensitive substring of `gts_id`. Over the usage collector products applies it itself, because
  the collector's storage plugin translates comparison operators only and refuses `contains` as an internal
  error (a 503 for the caller). The collector is asked with `kind eq` at most. Without `q` the page and its
  cursor are the collector's, uncapped. With `q` products walks the collector's pages for the asked `kind` up
  to 1000 usage types (`USAGE_TYPE_SEARCH_CAP`), keeps the ids that hold `q`, orders them by id and pages them
  with a cursor of its own, bound to `q` and `kind`: a cursor replayed with other values, or without `q`, is
  400. Past 1000 the search is 503 with `USAGE_TYPE_CATALOG_TOO_LARGE` in its detail, never a page of the part
  it read; the whole walk runs under the one `usage_type_resolver_timeout_ms` deadline.
- The 09-22 catalog design named the path `/bss-products/v1/catalog/usage-types` and gated it on
  `recognized_set × read`; that resource no longer exists, and picking a usage type is authoring a SKU, so the
  path is `/usage-types` and the gate the author grant.

Deploy note: SKU authors, submitters and approvers of usage SKUs need usage-collector read, granted with their
role; without it submit and approve answer 403 `USAGE_TYPE_FORBIDDEN`.

**Source:** Owner, 2026-09-27 (option b); phase 6 plan rev 2 (validation D5, asks 6 and 13; plan review H2, L9);
the `q` search, 2026-09-28 (the collector's plugin takes no `contains`).

#### P-D-208 [M] A retired SKU no longer keeps its category in use

Amends P-D-186 (#9). Category retirement is refused (409 `CATEGORY_IN_USE`) only while a SKU in `draft`,
`published` or `deprecated` names the category. P-D-248: a retire under review keeps one of those, so it still holds the category. The old `retiring` lifecycle no longer exists. A `retired` SKU no longer counts: nothing moves a
retired SKU (`sku_change` takes only published or deprecated), so under P-D-186 a category that ever held one
could never retire. `retiring` still counts, because a rejected or withdrawn retirement returns the SKU to its
prior lifecycle. The check stays one conditional write with a `NOT EXISTS` over those four lifecycles, in the
serializable transaction category assignment also runs in. A SKU without a category never counts (P-D-196).

Retiring a category that is already retired is 409 `CATEGORY_RETIRED` (validation D6), the code an assignment
to a retired category already answers; `CATEGORY_IN_USE` no longer covers it.

**Source:** Owner, 2026-09-27; phase 6 plan rev 2 (validation D6, ask 9).

#### P-D-209 [L] No tenant settings door: the fence TTL is the deployment setting `fence_ttl_minutes`

DESIGN §3.3, PRD §7.1 and slice 03 §5 described `GET/PUT /settings` with a tenant `fence_ttl_minutes`
(validation D2). No such door or table was built: the orphan-fence TTL is the gear's configuration
`fence_ttl_minutes` (default 30), one value per deployment. The docs now say so, and the route leaves them;
the approval policy stays on its own doors (P-D-190, P-D-205).

**Source:** Owner, 2026-09-27; phase 6 plan rev 2 (validation D2).

#### P-D-210 [M] The SKU list pages on the toolkit's OData, with a literal case-insensitive `q`

Owner decision 1 of the phase 6 plan: `GET /skus` moves to the toolkit's OData, as ledger's lists do. Its
own `type`, `category`, `lifecycle` and `after` parameters go.

- **`$filter`** names `id`, `code`, `name` (the toolkit's operators for their kinds), `lifecycle` and `type`
  (`eq`, `ne` or `in` with one of their values, another value being 400 `INVALID_FILTER`; the text functions
  as on any text field, since the served contract publishes them for every text field), `category_id` and
  `pending_unit_id` (`eq`, `ne`, `in`, and `eq null` / `ne null`: no category, in review).
  Only those two compare with `null`. `updated_at` orders and never filters: on SQLite a `$filter` would bind
  chrono's `+00:00` against the stored RFC 3339 `Z`, and a text comparison lies at the boundary.
- **`$orderby`** names `code`, `name` or `updated_at` (a narrower order vocabulary, ledger's
  `ExceptionOrderField` pattern); every order ends with the tie-break `id`, and the default is `code`. A
  nullable or filter-only field never keys an order: the cursor's comparison has no answer for a null key.
- **Paging** is the toolkit's: `$top` (alias `limit`) defaults to 50 and is clamped at 200; `cursor` (alias
  `$skiptoken`) comes from `page_info`. The answer is `Page<SkuListItem>`: `{ items, page_info { next_cursor,
  prev_cursor, limit } }`, each item the SKU's fields and its `usage` from one port call per page (P-D-197).
- **`q`** is `lower(column) LIKE lower(?) ESCAPE '\'` over `code`, `name`, `unit`, `usage_type_ref` and
  `gl_code`, the caller's `%`, `_` and `\` escaped. On Postgres both sides fold through the ICU root
  collation, `lower(column COLLATE "und-x-icu") LIKE lower(? COLLATE "und-x-icu") ESCAPE '\'`, so Unicode
  case folds whatever the database's locale: a database's own `lower()` folds ASCII only when its
  `LC_CTYPE` is `C` (`initdb --locale=C`, CloudNativePG's default; Benidorm's `app` database). SQLite's
  `lower()` folds ASCII only, so a non-ASCII letter matches another case of itself on Postgres only; the
  two backends differ there (`nfr-two-backends`), and both are pinned by tests, Postgres on a `C`-locale
  database too. An empty `q` is no search. The toolkit's `contains` in `$filter` follows the backend's
  `LIKE` (case-sensitive on Postgres); `q` is the case-insensitive search.
  Deployment note: the Postgres server must be built with ICU (the `und-x-icu` collation exists; the
  official images and CloudNativePG's have it); without it the list and the counts fail on any `q`.
- **Refusals.** Any key besides `limit`, `cursor`, `q`, P-D-212's `priced` and `in_plan`, and the OData
  options is 400 `UNSUPPORTED_QUERY_PARAM`, every offender named (a products copy of ledger's
  `reject_non_odata_list_params_allowing`); a plain key given twice is 400. `$select` and `$count` are 400.
  The cursor carries a hash of `$filter`, `q`, `priced` and `in_plan`: a cursor replayed with other values is
  400 `FILTER_MISMATCH`. Authorization is judged first (`sku × read`).
- **The transaction.** As before, the list recovers the tenant's orphan fences and reads its page in one
  transaction (P-D-189).

The toolkit change this rests on is recorded in the toolkit's own docs
(`docs/web-docs/build-with-gears/add-pagination-odata.md`, `libs/toolkit-db/src/odata/README.md`): its typed
`$filter` path admits `null` with `eq` and `ne` on a field that declares itself nullable
(`FilterField::nullable`, opt-in; here `category_id` and `pending_unit_id`), of any kind, and still refuses it
inside `in` and with an ordering; on every other field, in every gear, `null` stays a type mismatch (400), as
before. `contains`, `startswith` and `endswith` emit `LIKE … ESCAPE '\'` with the pattern escaped. SQLite has no
default escape character, so the browse search (`search_skus`, a `startswith` on the name) matched nothing
for a name holding `%`, `_` or `\` until then. P-D-196's owed `category=none` browse is `category_id eq null`.

Every products query parameter is declared with its type in the served contract (`limit` integer,
`include_released` boolean, the rest string; validation D4).

Breaking: the list's parameters and its envelope (`next` is `page_info.next_cursor`). The gears-rust e2e
follows in this run; vhp-core's e2e in phase 6.6.

**Source:** Owner, 2026-09-27; phase 6 plan rev 2 (owner decision 1; asks 1, 2 and 4; validation D4, D7; plan
review H1, L1–L5, L10); phase 6 review (queries F1: null equality opt-in per field; F3: the ICU fold).

#### P-D-211 [M] The SKU list's tab counts: `GET /skus/counts`

`GET /skus/counts` answers `{ all, draft, published, deprecated, retired, in_review }` for the tabs
of the SKUs screen (P-D-248 drops `retiring`; `$filter` gains `retire_pending`, and `lifecycle eq 'retiring'` is 400): every SKU the narrowing keeps, those in each lifecycle, and those a pending unit locks
(`pending_unit_id` set, in any lifecycle). It narrows as the list does, by `q`, P-D-212's `priced` and `in_plan`, and `$filter`, except that `$filter`'s `lifecycle` terms are dropped, because the counts count every
lifecycle. Only a term that is a top-level `and` conjunct is dropped; a `lifecycle` term under `or` or `not`
cannot go without changing what the rest means, so it is 400 `INVALID_FILTER`. The whole filter is checked as
the list reads it before the terms go. `$orderby`, `$top`/`limit`, `cursor`/`$skiptoken` and `$select` are
400 `UNSUPPORTED_QUERY_PARAM`. It recovers the tenant's orphan fences in its own transaction, as the list
does, so its `retiring` agrees with the list, and it counts in one grouped statement whatever the number of
SKUs. Authorization is `sku × read`.

The fence recovery the list and the counts run first is set-based too: one read of the tenant's expired
fences (the lifecycle each had while fenced, which the lift overwrites), one `UPDATE … RETURNING` lifting them
all on the same predicate in the same transaction, and one multi-row INSERT of their audit rows (P-D-213; one
INSERT per 1000 rows, under both backends' bind limits). A read that finds one expired fence makes the same
statements as a read that finds fifty; a read that finds none makes the one read.

**Source:** Owner, 2026-09-27; phase 6 plan rev 2 (ask 1; plan review M2); phase 6 review (queries F2: the
set-based fence recovery).

#### P-D-212 [M] The SKU list filters on pricing's usage (`priced`, `in_plan`) through the port's sets

Amends P-D-197. The SKUs screen filters on what the usage shows, and a list that pages cannot filter a
page's `usage` on the client, so the list and the counts take `priced=true|false` and `in_plan=true|false`.

- **The definitions are the usage's own** (pricing D-428): `priced` keeps the SKUs whose `usage.entries` is
  above zero (an entry in any book of the tenant, in any reference state), `in_plan` those whose `usage.plans`
  is above zero (a draft, pending, scheduled or published revision names one of the SKU's entries; pricing D-453
  counts a revision by its stored state). A plan item that
  names a SKU without an entry (an `included` item) does not count, as it does not in `plans`: the owner's open
  question from phase 5 stays open. `false` keeps the other SKUs. Tests pin `priced` ⇔ `entries > 0` and
  `in_plan` ⇔ `plans > 0` on the same data.
- **The port gains a method.** `SkuUsageV1::usage_sets(ctx, tenant) -> SkuUsageSets { priced, in_plan }`,
  each sorted and distinct. Pricing reads them under the same rule as `usage` (`price_book_entry:read`, the
  entries under that scope, the items and revisions tenant-scoped) in two set-based statements, the same
  whatever the number of SKUs.
- **One bind.** Products filters by a set with ONE bound value whatever its size: a JSON array read by
  `json_each` on SQLite (`unhex`, because a UUID is 16 bytes there), a `uuid[]` with `= ANY` on Postgres.
- **One call of each method per request.** A read with a usage filter asks `usage_sets` once, after the query
  is found valid and before its transaction; the list still asks `usage` once for its page (none for an empty
  page). The call is bounded like `usage`: two seconds on a task of its own, aborted when the read ends first.
- **Never an unfiltered page.** A port that refuses the caller is 403 `USAGE_FORBIDDEN`; no registered port,
  an error, a broken call and a call past the bound are 503 `USAGE_UNAVAILABLE`. A read without a usage filter
  is unchanged: its `usage` is `null` in those cases (P-D-197).
- The cursor's hash covers both filters (P-D-210); the counts take them as the list does (P-D-211).

**Source:** Owner, 2026-09-27; phase 6 plan rev 2 (ask 3; plan review M3).

#### P-D-213 [M] A SKU's history: every audit row on a SKU carries the lifecycle move its act made, and `GET /skus/{id}/history` reads them

The SKUs screen shows a SKU's history (ask 7): who did what, when, and the lifecycle the act moved the SKU
from and to. The audit log carried the act and its actor but no lifecycle, the approval rows are keyed on
the unit rather than the SKU, the retire's prior lifecycle lives only in `fence_prior_lifecycle` (cleared
when the fence goes), and the orphan-fence expiry wrote no row at all (plan review H3).

- **Two columns.** The migration `m20260927_000008_audit_lifecycle_move` adds `from_lifecycle` and
  `to_lifecycle` to `products_audit_log`: nullable `text`, each held to the five lifecycles by a named CHECK
  (`chk_products_audit_log_from_lifecycle`, `chk_products_audit_log_to_lifecycle`). It redefines the
  append-only guard, so the platform's one-way seal (P-D-200) still requires every record column unchanged,
  the two new ones included: the SQLite trigger `trg_products_audit_log_seal_unchanged`, and the Postgres
  function `bss.products_audit_log_append_only()` (the trigger that calls it is unchanged). Every other
  UPDATE and every DELETE stays refused. It is a forward migration, because the chain is deployed. On
  SQLite, `up` adds a column only when the table lacks it, so it replays. `down()` is irreversible: dropping
  the columns would erase recorded moves from an append-only record. A row written before the migration
  reads null for both, and nothing is backfilled.
- **Who stamps.** Every row whose act concerns a SKU carries the lifecycle the act found and the one it
  left, whether the row's subject is the SKU (`subject_kind` `sku`) or one of its approval units
  (`approval_unit`, whose `ref_id` is the SKU). Both values come from the act's own transaction, so a row
  says what its act did, not what a later act made of it. An act that moves nothing stamps the same
  lifecycle twice. A create has no `from` (the SKU did not exist), and a draft delete has no `to` (it no
  longer does). A row on no SKU (a category, reference or policy act) carries null for both.
- **The derivation table.** L is `published` or `deprecated`: the SKU's lifecycle when the act found it.
  At quorum 0, the submit and its apply are two rows at one instant, and the submit's `to` is the apply's
  `from`. Tests drive every row of the table through the doors, and they check that each row's `to` is the
  next row's `from` wherever no other writer came between them.

| action | unit kind | from | to |
|---|---|---|---|
| `sku.create` | none | null | `draft` |
| `sku.draft_update` | none | `draft` | `draft` |
| `sku.delete` | none | `draft` | null |
| `approval.submit` | `sku_publish` | `draft` | `draft` |
| `approval.submit` | `sku_change` | L | L (a type-change fence moves no lifecycle) |
| `approval.submit` | `sku_retire` | L | L (P-D-248: a retire submit moves no lifecycle) |
| `approval.vote`, `approval.refreshed` | any | the lifecycle found | the same |
| `approval.applied` (the apply at submit, quorum 0), `approval.approved` | `sku_publish` | `draft` | `published` |
| `approval.applied`, `approval.approved` | `sku_change` | L | the proposed lifecycle, or L |
| `approval.applied`, `approval.approved` | `sku_retire` | L | `retired` |
| `approval.rejected`, `approval.withdrawn` | `sku_publish` | `draft` | `draft` |
| `approval.rejected`, `approval.withdrawn` | `sku_change` | L | L |
| `approval.rejected`, `approval.withdrawn` | `sku_retire` | L | L |
| `sku.unfence` | none | L | L (a retire fence clears `retire_pending` and moves no lifecycle) |
| `sku.fence_expired` | none | as `sku.unfence` | as `sku.unfence` |

P-D-248: rows already stored with `retiring` stay in the log (the audit CHECK still allows the token). `GET /skus/{id}/history` maps them at read, on the raw strings before `Lifecycle::parse`, so the tab never shows `retiring` and a legacy row is never a 500. A retire submit is no move, an apply is `L → retired`, and a reject, withdraw, unfence or expiry is no move.

- **The orphan-fence expiry is audited** (amends P-D-189). Every SKU read runs the expiry: the list, the
  counts, the card, the versions, the references, the unit card, and the submit and reference doors. It
  lifted a fence without writing a row. It now writes `sku.fence_expired` on the SKU for each fence it
  lifts, with the move, the SKU's revision and the reason `fence_ttl_minutes=<n>`, in the read's own
  transaction. Its actor is the system: `actor_ref` is the nil uuid, the subject of the platform's system
  context, which no principal carries (`require_authenticated` refuses a nil subject). This is an audit
  actor, not an authorization subject; products still reads as the caller (P-D-207). The list's and the
  counts' expiry is set-based (P-D-211): it reads the tenant's expired fences once, lifts them all in one
  UPDATE, and writes their rows in one INSERT per 1000 rows (P-D-211); with no expired fence it is the one read.
- **The read.** `GET /skus/{id}/history` answers `Page<ProductsSkuHistoryEntry>`: `{ items, page_info }`,
  each item `{ at, actor, action, from_lifecycle, to_lifecycle, unit_id, unit_kind, note }`. Its source is the
  audit rows whose subject is the SKU, and the rows whose subject is an approval unit whose `ref_id` is the
  SKU, in the caller's tenant. `at` is the row's `written_at`: the submit, change and draft doors take it before
  their transaction and keep it across a retry; decisions, unfence, the fence expiry and a forced release take
  it inside the attempt. It is never the commit. `actor` is its `actor_ref`, and `note` its
  `reason`: a change's `note` (the change request's own field, on its `approval.submit` row), a decision's
  note, or the expiry's TTL. `unit_id` and `unit_kind` name the unit of a unit's row (one read of the page's
  units) and are null on a SKU's own row. The order is `audit_id` alone, the order the acts wrote: every
  writer mints it as a UUID v7 inside the act's transaction (per attempt), ordered within the process, and
  both backends compare it in time order (16 bytes on SQLite, `uuid` on Postgres); across replicas it is
  accurate to the millisecond. `written_at` does not order it: an act that began first can commit second (a
  slow usage-type resolution, a lost serialization race), and on SQLite its RFC 3339 text does not sort as
  time within one second (`…21.41868Z` after `…21.418681Z`). At quorum 0 the submit and its apply share one
  instant and read in the order they were written. The toolkit's pager serves it: `$top` (alias `limit`) defaults to 50 and is clamped at 200, and `cursor`
  (alias `$skiptoken`) comes from `page_info`. Any other key is 400, and so are `$filter`, `$orderby`,
  `$select` and `$count`. The cursor carries a hash of the SKU id, so a cursor from another SKU's history is
  400 `FILTER_MISMATCH`, and a cursor minted when the history ordered by `written_at` is 400. The read is
  authorized as the card is (`sku × read`), runs the SKU's orphan-fence expiry first, and answers 404 for a
  SKU the tenant does not hold and for a deleted draft. The `sku.delete` row stays in the log but is never
  read, because its SKU is gone.

**Source:** Owner, 2026-09-27; phase 6 plan rev 2 (ask 7; plan review H3); phase 6 review (behaviour B-1: the
change's note; B-2: the order by `audit_id`).

**Amended by P-D-219 (2026-09-28).** The `approval.submit` row's `reason` is the note of any of the three submit
doors: a publish's or a retire's `note` as well as a change's. The same note is stored on the unit as `submit_note`.

#### P-D-214 [L] SKU versions answer one shape each: the history an array, the version in force at `versions/as-of?date=`

`GET /skus/{id}/versions` answered an array, or one object when `as_of` was given: one path, two schemas, and a
client could not know the shape without reading the query (validation L10). Breaking changes are allowed in this
phase, so the shapes split.

- `GET /skus/{id}/versions` always answers an array of `SkuVersionDto`, oldest first. The array is empty before
  the first publication. Any query key is 400 `UNSUPPORTED_QUERY_PARAM`. The old `as_of` is refused, never
  answered with the history, and the detail names the new path.
- `GET /skus/{id}/versions/as-of?date=YYYY-MM-DD` answers one `SkuVersionDto`: the greatest `effective_from`
  not after the date, then the greatest `published_version` (P-D-191). Before the first version it is 404 with
  reason `NO_VERSION_IN_FORCE`, as before. A missing, repeated or malformed `date` is 400
  `INVALID_QUERY_PARAMS`, and any other key is 400 `UNSUPPORTED_QUERY_PARAM`, every offender named. The
  parameter is `date`, as pricing's `/resolve` spells it. This settles the PRD's `asOf`/`as_of` question
  (PRD §13).
- Both reads are authorized as the card is (`sku × read`), run the SKU's orphan-fence expiry first, and answer
  404 for a SKU of another tenant. `VersionsResponse` (untagged, array or object) is gone from the contract.

Pricing reads versions through the in-process registry port (`sku_version_as_of`, pricing D-424), not over REST,
so it is untouched. The gears-rust e2e reads no versions. vhp-core's e2e reads `versions?as_of=`
(`test_products_skus.py`) and follows in phase 6.6.

**Source:** Owner, 2026-09-27; phase 6 plan rev 2 (validation L10; plan review L10).

#### P-D-215 [M] Category reads: `GET /categories/{id}`, a `sku_count` on every read from one grouped count, and the list on the toolkit's OData

The Settings screen lists the categories with how many SKUs each holds, and opens one to edit it (ask 9). The
gear served the whole list only, with no count and no single read.

- **`GET /categories/{id}`** answers one category as `ProductsCategoryItem`: the category's fields and
  `sku_count`. The `ETag` is its version, the value the category PATCH takes as `If-Match`. A category the
  tenant does not hold is 404.
- **`sku_count`** counts the SKUs that are not retired and name the category: `draft`, `published`,
  `deprecated` and `retiring`. These are the SKUs that keep a category in use (P-D-208), so an active category
  with `sku_count` 0 may be retired. Both reads take it from ONE grouped read (`COUNT` grouped by
  `category_id`, over the tenant's SKUs), never one count per category. Tests pin the list and the single
  read at two statements each for 10 and for 100 categories. A category that no counted SKU names reads 0. The
  count is an aggregate of the tenant's SKUs, served under the category read grant (`category × read`),
  without the SKU read grant. It is the same fact the retire refusal `CATEGORY_IN_USE` already discloses to a
  category author. The write doors (create, PATCH, retire) answer the category without the count, as before.
- **The list** (`GET /categories`) pages on the toolkit's OData, the SKU list's pattern (P-D-210).
  `$filter` names `id`, `code`, `name`, `status` (`active` or `retired` with `eq`, `ne` and `in`, another
  value being 400), `is_default` and `sort_order`. `null` is refused, because no field is nullable.
  `$orderby` names `sort_order`, `code` or `name`, and every order ends with the tie-break `id`. The default
  order is kept: `sort_order`, then `code`. `status` and `is_default` filter only. `$top` (alias `limit`)
  defaults to 200, a generous page for a flat list (plan review M4), and is clamped at 200. `cursor` (alias
  `$skiptoken`) comes from `page_info`, and it carries a hash of the `$filter` it was cut under, none
  included: a cursor replayed under another `$filter` is 400 `FILTER_MISMATCH`. Any other key, `$select`
  and `$count` are 400. The answer is `Page<ProductsCategoryItem>`, and `CategoryList` is gone. Authorization
  is judged first (`category × read`).

Breaking: the list's envelope (`page_info`) and its page size. A tenant with more than 200 categories now
pages, so a client that reads the list whole must follow `next_cursor`. The gears-rust e2e reads no category
list. vhp-core's e2e reads it whole (`test_products_skus.py`, `test_products_isolation.py`) and follows in
phase 6.6.

**Source:** Owner, 2026-09-27; phase 6 plan rev 2 (ask 9; plan review M4).

#### P-D-216 [M] An approval-policy override can be reset; the default cannot be deleted (twin of pricing D-435)

The PUT sets an override for `sku_publish`, `sku_change` or `sku_retire`, but nothing removed one, so a kind
once overridden never followed the default again (ask 11b). `DELETE /approval-policy/{kind}` (`products:settings`)
removes the kind's override at the policy the caller read: `If-Match` carries the policy's content tag
(P-D-205). The kind then follows the default quorum again, and the answer is the policy with its new `ETag`, as
the PUT answers. The path names the default as `*` (percent-encoded or not), as the PUT's body does. The
default is never deleted (400 `POLICY_DEFAULT_REQUIRED`): a tenant always has a quorum to fall back to, and a
tenant that never stored one follows quorum one. The PUT changes the default and nothing removes it. The
refusals are judged in this order: 403 without `products:settings`, before any precondition; 400 for a missing
or malformed `If-Match`; 400 `POLICY_DEFAULT_REQUIRED`, or `VALIDATION` for an unknown kind, as the PUT
refuses one; 409 `STALE_REVISION`; 404 when the kind has no override. The comparison and the removal run in
one transaction. A reset writes one audit row, `approval_policy.reset`. A unit already submitted keeps the
quorum it copied (P-D-190). Pricing has the same door for its kinds (D-435).

**Source:** Owner, 2026-09-27; phase 6 plan rev 2 (ask 11b; plan review L8).

#### P-D-217 [M] Closed sets are enums on the responses; requests keep strings and their codes (twin of pricing D-439)

Every closed set a response schema carries is an enum in the served OpenAPI (ask 12). It holds exactly the
tokens that the column stores and that the wire always carried, so the wire does not change. The sets: a SKU's
`type` (`recurring`, `usage`, `one_time`, `bundle`), `lifecycle` (`draft`, `published`, `deprecated`,
`retiring`, `retired`) and `billing_timing` (`advance`, `arrears`), on the SKU and on a version's content; a
category's `status` (`active`, `retired`); the history's `from_lifecycle` and `to_lifecycle` (P-D-213); a unit's
`state`, a decision (`approve`, `reject`) and a vote's `outcome` (`pending`, `applied`, `rejected`,
`withdrawn`); a reference's `kind` (`price_book_entry`, `plan_item`, `sold_as`) and `state` (`reserved`,
`confirmed`, `released`), on the reference list and on the reservation receipt. Each set is one schema component
with the `Products` prefix (`api/rest/closed_sets.rs`). A set that the SDK or the approval engine already types
maps to and from that enum, so a value added on one side only does not compile.

A database CHECK holds each stored set on both dialects: the SKU's `type`, `lifecycle` and `billing_timing`,
the category's `status`, the reference's `ref_kind` and `state`, the audit log's lifecycle columns, and the
unit's state and the decision (`bss_approval::ddl`). The SKU's columns and the lifecycle columns already read
back fallibly in the repositories. A category's status and a reference's kind and state are read here. Only a
writer that goes around the CHECK can store a token outside its set, and the read answers it with `CorruptRow`,
which names the row: a 500, never a panic and never a value that the enum does not hold.

Requests keep `string`: `type`, `lifecycle` and `billing_timing` on the SKU writes, a reservation's `kind`, and
the policy's `kind`. Each door keeps its own refusal, 400 `VALIDATION` on the field. A request enum would fail
at deserialization, before the door, with a 400 that has no field and no code.

Response fields that stay `string`: the history's `action` and `unit_kind`, a unit's `kind` and `ref_type`,
and a reference's `owner`. No CHECK holds those columns (`m20260925_000004` records the audit vocabulary as an
owed debt), and a CHECK on an existing SQLite column needs a table rebuild, which the phase's ADD COLUMN
migrations do not allow. A usage type's `kind` is the collector's vocabulary, not this gear's. The picker's
`source` names the catalog that the deployment wired (P-D-207), not a stored value. The kept `/browse` envelope
carries the catalog port's vocabulary verbatim.

The census test in `gear_tests.rs` reads the served spec: every listed field has its enum with the exact values
and its nullability, no request body reaches an enum, and the fields above stay plain strings. A category row
poisoned on SQLite (the CHECK refuses the write; the test then bypasses it) reads 500.

**Source:** Owner, 2026-09-27; phase 6 plan rev 2 (ask 12; plan review M5).

**Amended by the phase 9 review, theme C (fix run 9.5d-1, 2026-10-01).** A unit's `kind` is an enum on every
response, `ProductsApprovalKind` (`sku_publish`, `sku_change`, `sku_retire`). No CHECK holds the column still; the
repository reads it through the set (`domain::approvals::ApprovalKind`), so a unit of another kind is a corrupt row
(500), never served (P-D-227). A unit's `ref_type` and the history's `unit_kind` stay strings; the history reads the
unit's kind through the same set, so a SKU's history refuses a unit of another kind too.

#### P-D-218 [M] Making a category the default moves the default in one write; a lost race is 409 `CATEGORY_DEFAULT_TAKEN`

A tenant has at most one default category: the partial unique index `uq_products_category_default` on
`(tenant_id) WHERE is_default`, on both dialects since `m20260925_000001`. A second default used to reach it
unmapped, and PATCH `{is_default: true}` and POST with `is_default: true` answered 500.

- `is_default: true` on PATCH `/categories/{id}` or on POST `/categories` moves the default. In the door's
  one transaction, the tenant's previous default (any category but the target) is cleared first, then the
  target is written. The cleared row is a category write like any other: `version` + 1, `updated_at`, and its
  own `category.update` audit row. The target keeps its own audit row (`category.update` or `category.create`).
  Setting the default on the category that already holds it clears nothing.
- A clash that remains is a concurrent move that took the default between the clear and the set. It is 409
  `CATEGORY_DEFAULT_TAKEN` on both doors, and the refused act writes nothing. The index maps by name on
  Postgres and by its column (`products_category.tenant_id`, after the code index's pair) on SQLite. The doors
  run read committed on Postgres: the second of two concurrent moves waits on the first's lock on the old
  default, finds it cleared, and its set meets the winner's default.
- `is_default: false` clears only the target; a tenant may have no default (P-D-196: nothing falls back to it).

**Source:** Owner, 2026-09-28 (backend asks, 9b).

**Amended by P-D-220 (2026-09-28).** A retired category is never the default: `is_default: true` on one is 409
`CATEGORY_RETIRED`, and retiring the default clears it first.

#### P-D-219 [M] The submitter's note travels with the approval unit (twin of pricing D-445)

The Approvals screen shows why a unit was submitted (ask 4b). Until now a change's note (P-D-213) stood only on
the submit's audit row, and `POST /skus/{id}/submit` and `/retire` took no body at all.

- **The doors.** `POST /skus/{id}/submit` and `POST /skus/{id}/retire` take an optional body `{ note }`
  (`ProductsSkuSubmitRequest`). No body, `{}` and `note: null` carry no note, and any other field is 400, as the
  empty body was. `POST /skus/{id}/changes` keeps its `note` (P-D-213). The three doors share one limit: at most
  2000 characters, counted as Unicode scalar values, as pricing counts a book's description (D-444). Before this
  decision the change's note had no limit. The door judges the limit with the body's other violations (P-D-202):
  400 `NOTE_TOO_LONG` on `note`, and nothing is written. The note is stored as sent: it is not trimmed, and a
  blank note is kept.
- **Stored on the unit.** The note is `submit_note` on `products_approval_unit`. The submit writes it once and
  nothing rewrites it: a stale refresh keeps it, and a decision leaves it. It also stays on the submit's
  `approval.submit` audit row as its `reason`, which the history reads (P-D-213). That row now carries the note
  of each of the three doors, not only a change's.
- **Not content.** The note is not part of the unit's snapshot or its fingerprint (`snapshot_hash`). Two submits
  of the same content that differ only by their note have the same hash, so a note never makes a unit stale.
- **The reads.** `GET /approval-units`, `GET /approval-units/{id}` and every receipt that carries a unit (submit,
  changes, retire, the votes) carry `submit_note`: the note as sent, or null.
- **The column.** The shared approval DDL `bss_approval::ddl::up()` is the body of deployed migrations (products
  `000003`, pricing `000002`), so it is not edited. If it were, a fresh chain would create the column at `000003`
  and an upgraded one at the new migration: the column order would differ, and the upgrade would never be tested
  against the stand's shape (plan review H3). The library has a separate step instead, `ddl::add_submit_note`:
  `ALTER TABLE … ADD COLUMN submit_note text`, nullable, no default, no CHECK. On Postgres it says `IF NOT EXISTS`;
  on SQLite `ddl::apply_add_submit_note` reads the catalog first. The forward migration
  `m20260928_000009_unit_submit_note` runs it, so it replays. Its `down` drops the column the same way; the audit
  rows keep every note for the history.
- **Units submitted before the migration** read `submit_note: null`. This includes a change unit whose history
  row shows its note: nothing is backfilled from the audit log (plan review L6).
- **Pricing.** The column belongs to the one unit shape the gears share. Pricing adds it by its own migration and
  carries it on its unit reads; its submit doors took no note (D-445). Pricing D-464 amends this bullet: its plan
  revision submit takes an optional `{ note }` under this entry's body rule, and its publish-changes an optional
  `note` beside its selection, both capped at 2000 characters; its single price's submit takes none.

**Source:** Owner, 2026-09-28; phase 7 plan rev 2 (ask 4b; plan review H3, L6). Its Pricing bullet is amended by
pricing D-464.

#### P-D-220 [M] A retired category is never the default; retiring the default clears it

P-D-218 said nothing about a retired category. PATCH `{is_default: true}` on a retired category moved the default
to it, and retiring the default left it the default. A tenant could then hold a retired default, which a SKU
cannot be given (409 `CATEGORY_RETIRED`, P-D-196). The phase 6 fix run 2 review (LOW-2) found the gap, and the
owner chose to close it.

- **PATCH on a retired category.** `is_default: true` is 409 `CATEGORY_RETIRED`, and nothing moves: the tenant's
  default keeps its flag and version, and no row is written. The door judges If-Match first, as on every PATCH, so
  a stale tag is 409 `STALE_REVISION`. The other edits of a retired category still pass, `is_default: false`
  included. POST always creates an active category, so the rule has no POST case.
- **Retiring the default.** `POST /categories/{id}/retire` on the tenant's default clears the default first. The
  clear is a category write of its own: `version` + 1, `updated_at`, and a `category.update` audit row, as a
  cleared holder has under P-D-218. Then the retirement writes (`version` + 1 and its `category.retire` row), and
  it answers the category retired and not default. Both writes are in the retirement's one transaction, so a
  refused retirement (`CATEGORY_IN_USE`, `CATEGORY_RETIRED`) clears nothing. The tenant has no default after,
  which P-D-196 allows: nothing falls back to a default. Retiring a category that is not the default writes the
  one retirement, as before.
- **Races.** A concurrent retirement bumps the category's version, so a default move that read the category
  active fails its version-conditional write (409 `STALE_REVISION`) and its clear of the old default rolls back.
  The retirement runs serializable on Postgres: a move that commits between its reads and its write is a
  serialization failure, and the retry finds the new default and clears it.
- **A retired default stored before this decision** is cleared by the forward migration
  `m20260928_000010_clear_retired_defaults`: `UPDATE products_category SET is_default = false, version =
  version + 1, updated_at = <the migration's instant> WHERE is_default AND status = 'retired'`, on both
  dialects. The clear is written as a category write writes it: a new version (a client that holds the old ETag
  gets `STALE_REVISION`) and `updated_at` bound in UTC from the process clock, as the doors bind theirs. It
  writes no audit row: a migration is not an act of a user, and the audit log records acts. The tenant then has
  no default, as after the retirement of its default. The schema does not change, a replay matches no row, and
  `down` changes nothing (a retired default is the state this decision forbids). So the rule holds for every
  stored row, and the deploy notes' query for such rows is informational.

**Source:** Owner, 2026-09-28 (a yes; phase 7 plan rev 2, added to run 7.3); phase 6 fix run 2 review (LOW-2);
phase 7 review (queries, migrations and docs lens, LOW-1: the migration for stored rows).

#### P-D-221 [M] The outbox wakes its sequencer after the commit (twin of pricing D-455)

Since the main sync, toolkit-db's `Outbox::enqueue` does not mark its partition dirty. It returns a `Wake`,
which marks the partition and wakes the sequencers when it is fired, after the commit (toolkit-db `2bfc76aec`).
`enqueue_typed` fired that `Wake` at once, inside the caller's transaction. A sequencer woken then read the
partition before the commit, found nothing and cleared the flag. The committed row then waited for the cold
reconciler, a minute at the default profile.

- **The handle.** `events::TxOutbox` is the event sink as one transaction sees it. `enqueue_typed` takes it in
  place of the `EventSink`. It adds each event's `Wake` to the handle and fires nothing. The clones of a handle
  share it.
- **The transaction.** `events::transaction` runs the work in `Db::transaction_with_retry` with the door's
  isolation and retry classifier, and with a new handle over `ApiState::sink`. A retried attempt first discards
  the wakes of the attempt before it, which rolled back. When the transaction commits, the handle fires once.
  When it fails, the handle is discarded.
- **The writers.** Every door that enqueues runs in it: the SKU submit (publish, change and retire, applied at
  once at quorum 0), the vote door (approve, reject and withdraw) and the force release of a reference. The
  unit's review read before a vote runs in it too: it enqueues nothing, but the subject it reads through takes
  the handle.
- **The approval subjects.** The apply of `SkuPublish`, `SkuChange` and `SkuRetire` enqueues `SkuPublished`,
  `SkuChanged` and `SkuRetired` inside the engine. `SkuPublish` holds the attempt's handle (its field `outbox`,
  which was `sink`), so the wake leaves the engine with the subject, which the gear builds for its
  transaction. `bss-approval` changes no signature, and the `ApprovalSubject` doc says where such an effect
  stays. `governance::decided` takes the handle for `ApprovalUnitDecided`.
- **The census.** A test pins that `TxOutbox::new` occurs in `src/` only in `events::transaction`, and that no
  other file fires or discards a wake. The door test alone cannot see a subject that holds a handle of its own:
  the test broker puts every event of a tenant in one partition, so the door's `ApprovalUnitDecided` wake
  delivers the SKU event too.
- **The tests.** `infra/broker_wake_tests.rs` drives the real in-process broker (the event-broker gear's
  `test_support` harness, a new dev-dependency) over a database with four connections. With one connection
  the sequencer queues behind the transaction, and the race does not show. A committed enqueue is delivered at
  once, although its transaction goes on for 300 ms after it. A rolled-back one wakes nothing: a row committed
  before it with its wake discarded stays undelivered in the same partition. A retried attempt's wake is
  dropped. The two events of a quorum-0 SKU submit are delivered at once. A probe that fires the wake inside
  the transaction again turns the first test red. Products had no real-broker test before: the mock of the
  broker SDK had hidden the subject-type contract that the sync found.
- *Rejected alternative:* each enqueue returns its `Wake`, and every function on the path returns it to the
  transaction (the toolkit's `outbox::in_transaction` and main's gears). The engine's `apply` returns nothing,
  so the subjects would need a second mechanism. An error after an enqueue would also drop an unfired `Wake`,
  which the toolkit logs as a leak.

**Source:** Main sync of 2026-09-29 (sync report, port 3; toolkit-db `2bfc76aec`); pricing phase 8 plan rev 2
(run 8.2b).

#### P-D-222 [H] The registry trusts pricing's system actor in-process only; no REST door serves that actor

**Status:** DECIDED 2026-09-29. Amended 2026-09-30 (fix run W1c): both gears refuse the actor at the REST edge.

The in-process reference registry (`infra::reference_registry::LocalReferenceRegistry`, the `ReferenceRegistryV1` that pricing
reaches as `PricingReferenceRegistry`) gives one principal a tenant-wide scope without the PDP: pricing's system actor
(`subject_type` `bss-pricing.system`, `subject_id` `PRICING_SYSTEM_ACTOR`), on the registry bound to the `pricing` owner, in the
caller's own tenant. Pricing's resolve and its reference ticker read and reserve as that actor (pricing D-424). The whole-branch
review asked whether a caller could assert it (RS-02). The owner kept the trust (O1).

- **What was measured.** A REST caller's `SecurityContext` comes from its token, and it can carry that subject type. The OIDC
  authn plugin maps `subject_type` from a claim (`user_type` by default, and vhp-core's `config/server.yaml` configures the same)
  and `subject_id` from `sub`. The static-authn plugin takes both from its configured identities. The gateway does not remove the
  value. A caller cannot forge a signed token, but an IdP that issues `user_type: bss-pricing.system` with that `sub` gives a REST
  caller exactly the context the registry trusts. So the trust is safe only if no REST path honours it.
- **The threat model.** In-process code of the same binary is trusted, as it is trusted with the database. The registry is reached
  only through the `ClientHub`: products registers it once at init, pricing is its one consumer, and no REST door calls it.
  The registry's system branch is in `LocalReferenceRegistry::scope` alone, and its doc says so.
- **The relay (second review of W1b, M1).** No products door calls the registry, but pricing's doors do, with their caller's
  context: the entry create reads the SKU (`sku_for_write`) and drives its reserve and confirm as the caller
  (`reference_work::drive`), and so do the entry delete, the plan item doors, the plan copy, clone and revision delete (their
  reference ops), and the plan checks' SKU reads (`plans::fresh_skus`). The entry PATCH makes no registry call. So a REST caller whose token carried the actor got the registry's tenant-wide trust through a pricing
  door, without products' own `read` or `reference` check, and the audit row said `actor_kind=system`. A pricing test showed
  it: under a policy that grants the caller every pricing action and a products that grants nobody, the entry create answered
  201. The first version of this decision said that nothing on a REST path read the subject type, which left pricing's doors out.
- **The edge refusal (fix run W1c).** Both gears refuse pricing's system actor on every REST door, in either half: a context
  whose subject type is `bss-pricing.system` or whose id is `PRICING_SYSTEM_ACTOR` is 403 `SYSTEM_ACTOR_RESERVED`, after the
  authentication check (401) and before the PDP. The one test is `bss_products_sdk::is_pricing_system_actor`, and each gear's `require_authenticated`,
  which every door calls first, applies it (products `api::rest`, pricing `authoring::support`). The refusal is logged on the
  gear's authz deny target. Only pricing's own actor is refused: another system subject (Rating's and Subscriptions', which call
  pricing's resolve, D-424) passes the edge, and the PDP judges it by its roles, as every other caller.
- **On the stand.** The relay predates phase 8. At `01f670fa4`, the build the Benidorm stand runs (its gears pin `a9cf1a605`
  merges it), the registry's trusted branch, pricing's doors that pass their caller's context and a `require_authenticated` that
  accepts any subject type are all present, and vhp-core maps `user_type` to the subject type. So it was live for a token whose
  `sub` is `PRICING_SYSTEM_ACTOR` and whose `user_type` is `bss-pricing.system`, with a pricing grant for the door. Only the IdP
  can issue such a token. The stand itself was not probed.
- **The audit label.** The reference doors wrote `actor_kind=system` on a reserve, confirm or release audit row when the subject
  type ended in `.system`, so a REST caller's token set the label. Now `references::Acting` carries the context and whether the
  registry's trusted branch admitted it. A REST door always acts as a subject, and only the registry records the system's act.
- **The tests.** `a_rest_caller_asserting_the_pricing_system_actor_gets_no_bypass` calls every served door (the operations the
  gear's own `register_rest` serves, RT-01 and fix run W1c's L1) as the actor three ways (both halves, the subject type alone, the id alone): each is 403
  `SYSTEM_ACTOR_RESERVED` and the PDP is never asked. Rating's system subject at the same doors, under a PDP that allows
  nothing, is 403 after the PDP was asked the door's own action.
  `a_rest_reservation_is_a_subjects_act_whatever_the_token_asserts` pins the audit label, for another gear's system subject.
  Pricing has the twins: `rest_authz::no_rest_door_serves_pricings_system_actor` (every door pricing serves) and
  `plan_doors::a_rest_caller_asserting_pricings_system_actor_never_reaches_the_registry` (the entry create above is 403,
  nothing is written and the registry is never called).
- *Rejected alternative:* a PDP role for the system actor (the way account-management's `am.system` goes through its PDP). vhp-core's
  PDP does not know `bss-pricing.system` (D-424's note), so every resolve and every ticker call would be refused until it did.

**Source:** Owner, 2026-09-29 (the dispositions' O1, answered "ok"); whole-branch review RS-02 (fix run W1b); the second review of
W1b, M1 (fix run W1c). Keeps pricing D-424.

#### P-D-223 [M] A refusal keeps its class and names its resource

The whole-branch review found refusals that left with another class than the one decided, or named a resource the gear does not
register.

- **The resource.** Every `DomainError` refusal named `cf.bss.products.product.v1~`, a type this gear neither defines nor
  registers, so a SKU's 404 said `product.v1~` and its 403 `sku.v1~` (RS-25). The ladder (`infra::error_mapping`) now picks one of
  the three registered labels from the refusal: a missing row by its kind (a reference is a SKU's), an approval refusal and a
  stale unit the approval unit, a code starting `CATEGORY_` the category, and every other refusal the SKU. The approval policy's
  404 names the approval unit, the label its grant is asked on, and `governance::scope`'s 403 names the resource it asked.
- **The class.** The browse REST client turned every non-2xx answer into a retryable 503. A 4xx from the browse door now passes
  its Problem through with its class (a caller without `sku` read stays 403, a refused `$filter` 400). A 4xx without a Problem is a
  500, an answer the client cannot use. Any other status stays the 503 of a catalog that did not answer, with a fixed detail
  (RS-06). Behind the catalog, a repository failure was a 503 whose detail was the driver's text. It is now the repository's
  logged 500 with the text off the wire, and only a pool that cannot hand out a connection stays a 503 (RS-07, RS-09).
- **The engine's refusals.** The engine's `UnitNotFound` answered 409 through the `other` arm, and it is now the unit's 404. A
  duplicate decision that loses the unique index is `DUPLICATE_VOTE` 409, not a store failure's 500 (RS-32). A reservation that
  loses the live-reference index twice is `REFERENCE_EXISTS` 409, not a 500 (RS-04, RS-05).
- **Consumers.** No consumer reads `resource_type`: the gears-rust tests, the e2e suites and vhp-core's e2e do not. Pricing no
  longer calls `ProductCatalogClientV1`. Products' own browse door and the REST client's test are its only callers.

**Source:** Whole-branch review of 2026-09-29, RS-04, RS-05, RS-06, RS-07, RS-09, RS-25 and RS-32, and fix run W1a's note on
`UnitNotFound` (fix run W1b).

#### P-D-224 [M] The approval-unit list pages and reads its page set-based (twin of pricing D-458)

**Status:** DECIDED 2026-09-29.

- **The defect.** `GET /bss-products/v1/approval-units` answered every unit of the tenant that its filters kept, decided ones
  included, and read each unit's decisions with a statement of its own, all in one serializable transaction on Postgres (RS-03).
- **The page.** The list takes `limit` (200 by default, clamped at 500, pricing's rule, D-458) and `cursor`, the opaque
  continuation of a page's `page_info.next_cursor`, the toolkit pager's cursor. The order stays submission order
  (`submitted_at`), with the unit id breaking a tie. The answer is `{ items, page_info }`: `items` keeps its shape, and
  `page_info` (`next_cursor`, `prev_cursor`, `limit`) is added. The cursor carries a hash of the narrowing (`state`, `kind`,
  `ref_id`), so a cursor replayed under another narrowing is 400 `FILTER_MISMATCH`. A cursor that does not read is 400, and a
  `limit` that is not a number 400 `VALIDATION` on `query`.
- **The reads.** A page reads its units in one statement and all their decisions in one more
  (`approval_repo::page_units`, `decisions_of_units`). The QueryRecorder shows the same statements for 10 and for 100 units,
  each with a vote.
- **Breaking for a caller** that reads the list whole: a tenant with more than 200 units matching its filters gets them over
  several pages and must follow `next_cursor`. The in-gear tests that read the list whole follow it
  (`sku_governance_tests::Fixture::all_units`); the gears-rust e2e reads no products list. The vhp-core e2e reads page 1
  in `tests/bss-products/test_products_approvals.py` (the pending queue, twice), `test_products_isolation.py` (another tenant's
  list), `test_products_usage_types.py` (an empty list) and `test_products_authz.py` (a reader's list), and through the helper
  `tests/lib/products.py` `units_of` (`GET /approval-units?ref_id=`, its `items`: one SKU's units), which
  `test_products_approvals.py` calls three times. Each runs in a fresh tenant with a few units, so it passes unchanged; a
  vhp-core change would make them follow `next_cursor`.

**Source:** Owner, 2026-09-29 (the dispositions' O2, answered "ok"); whole-branch review RS-03 (fix run W1b).

**Amended by P-D-227 (2026-09-30).** The list also takes `$orderby=submitted_at desc`, newest first, while submission
order stays the default. `$orderby=submitted_at asc` names that default. Any `$orderby` but `submitted_at` (asc or desc) is
400 `INVALID_ORDERBY_FIELD`, and `$orderby` beside a `cursor` is 400 `ORDER_WITH_CURSOR`.

**Amended by P-D-228 (2026-09-30).** A page also reads all its units' items in one statement more
(`approval_repo::items_of_units`; since the phase 9 review, `item_authors_of_units`, their authors alone), so it
makes three statements whatever its size.

#### P-D-225 [M] Every text a request writes has an explicit length cap (twin of pricing D-457)

**Status:** DECIDED 2026-09-29.

- **The defect.** Only a SKU's code had a cap (64). A SKU's name, description, GL code, tax category, invoice line template,
  usage-type reference and unit, a category's code and name, and an operator's release reason had a blank check at most, on
  create, draft PATCH and change alike (RS-10, RS-11, RS-38). They landed in unbounded `text` columns and were copied into the
  approval snapshots, and a long category code could fail its unique index on Postgres as a 500.
- **The caps**, counted in characters (Unicode scalar values), as a submitter's note always was (P-D-219): a code 64 (a SKU's,
  a category's); a name 200 (a SKU's, a category's); a description, a note or a reason 2000 (a SKU's description, a forced
  release's reason, a submitter's note, a vote's note); a GL code, a tax category and a unit 64; an invoice line template 2000;
  a usage-type reference 512. The values live in `domain::caps`, and const assertions tie the note cap to the approval
  engine's `NOTE_MAX_CHARS` and the submit doors' own.
- **The refusal.** 400 `FIELD_TOO_LONG` with the field named, a violation of the body's validation stage (P-D-202), the shape
  `NOTE_TOO_LONG` already has. A SKU code over 64 characters was a `VALIDATION` violation and is now `FIELD_TOO_LONG` too. A
  note keeps `NOTE_TOO_LONG`: the submit, change and retire doors judge it (P-D-219) and the approval engine judges a vote's
  (fix run W1a, X-01; RS-37 measured already closed). Each door judges the texts with its body's other rules, before its
  transaction opens, so a text too long is refused before a 404 or a 409.
- **Stored rows.** Only a write is judged, and only on the texts its body carries (a cleared field carries none). A stored row
  over a cap stays readable, and a PATCH that does not carry the field leaves it as it is.
- **The tests.** `api/rest/caps_tests.rs` sends one text over each cap to each door and reads that nothing was written; texts
  at the caps in two-byte characters pass; a vote's note over 2000 characters is `NOTE_TOO_LONG` on approve and reject.
- **Pricing** applies the same caps in D-457. The vhp-core e2e would add one refusal: a SKU created with a 65-character code is
  400 `FIELD_TOO_LONG` on `code`.

**Source:** Whole-branch review of 2026-09-29, RS-10, RS-11, RS-37 and RS-38 (fix run W1b; the dispositions' "Length caps").

#### P-D-226 [M] The SDK's SKU types serialize as the wire carries them

**Status:** DECIDED 2026-09-30.

- **The defect.** `bss_products_sdk::models::Sku` says it is the SKU "as the doors return it", but its derived serde wrote
  `created_at` and `updated_at` as `time`'s tuples, where `SkuDto` sends RFC 3339 strings; `SkuVersion` wrote `effective_from` as
  `[year, ordinal]` where `SkuVersionDto` sends `YYYY-MM-DD` (RS-23). `SkuChangedPayload` was `snake_case` with a tuple date and no
  actor, while the `SkuChanged` event the broker emits is `camelCase` (`skuId`, `effectiveFrom` as `YYYY-MM-DD`, `actorRef`), the
  PRD's shape (RS-24). A consumer that read a door's JSON or an event into these types failed on every one.
- **The serde.** `Sku`'s instants and `SkuVersion`'s `created_at` use `time::serde::rfc3339`, and `SkuVersion.effective_from` a
  `YYYY-MM-DD` date, as the DTOs write them. `SkuChangedPayload` is `camelCase`, carries `actorRef`, and writes its date as
  `YYYY-MM-DD`, field for field the emitted event. `BillingTiming` gains `as_str` and `parse` (RS-48), which the repository and the
  PATCH DTO now use in place of three copies of its tokens.
- **Consumers.** Nothing deserializes these types from JSON today: pricing receives `Sku` and `SkuVersion` typed, through the
  in-process registry, and nothing reads `SkuChangedPayload` (measured over gears-rust and vhp-core's crates). So no reader breaks,
  and the first one reads what the wire carries.
- **`SkuContent` is a storage format** (RS-22). Its derive writes the append-only `content` of every stored version and the
  proposal of every unit, so its serde stays compatible forever: a field added is an `Option` or carries `#[serde(default)]`, a
  field is never renamed without `#[serde(alias)]`, and there is no `deny_unknown_fields`. Its doc says so, and
  `sku_repo_tests::stored_content_fixtures_keep_reading` reads fixture rows through the repository: one as this build writes it,
  one without the optional fields and one with a field this build does not know.
- **The tests.** `the_sdk_sku_types_read_the_doors_json` reads a SKU card and a version history into the SDK types and writes
  them back unchanged; `the_sdk_payload_reads_the_emitted_sku_changed_event` does the same for the event.

**Source:** Whole-branch review of 2026-09-29, RS-22, RS-23, RS-24 and RS-48 (fix run W1b).

#### P-D-227 [M] The approval units are counted by state and kind and list newest first on request (twin of pricing D-470)

**Status:** DECIDED 2026-09-30.

The approvals screen of the pricing-mfe merges pricing's and products' units (the owner's option 1: two per-gear methods that
the UI merges). It shows a badge per state and per kind and the newest units first (ask 42).

- **The counts.** `GET /bss-products/v1/approval-units/counts` answers `ProductsApprovalUnitCounts { by_state { pending,
  approved, rejected, withdrawn }, by_kind { sku_publish, sku_change, sku_retire }, total }`: every state and every kind is
  named, 0 when none, and `total` is the number of units the list pages through under the same narrowing.
  - It takes the list's whole narrowing (pricing plan review L11): `state`, `kind` and `ref_id`. The list and the counts read
    one condition (`approval_repo::UnitListFilter`), so the badge and the list count the same set.
  - A narrowing the list refuses is refused the same way: 400 `VALIDATION` on `state` for an unknown state, 400 `VALIDATION`
    on `kind` for a kind products does not record (below), 400 `VALIDATION` on `query` for a query that does not parse.
    It takes nothing but the narrowing: `limit`, `cursor`, `$orderby` and any other key are 400 on `query`.
  - It counts in ONE grouped statement (`approval_repo::count_units`), whatever the number of units. A stored kind products
    does not record is a corrupt row (500).
  - It is authorized as the list is (products read on approval units) and declares 503 as every products op does.
- **The descending order.** The list takes `$orderby=submitted_at desc`, newest first, and `submitted_at asc` (or
  `submitted_at` alone), the submission order of P-D-224 and still the default. The unit id breaks a tie in the same
  direction. Any other `$orderby` is the toolkit's 400 `INVALID_ORDERBY_FIELD`. Before this decision the list ignored an
  `$orderby`. Its violation names the key it refuses: the other field, or "only one key, submitted_at, is accepted"
  for a second `submitted_at` key; it named the whole order before, which called `submitted_at` unsupported (the phase 9
  review's R67, fix run 9.5d-1; `sku_governance_tests::a_refused_order_names_the_key_it_refuses`).
  - The order is not part of the narrowing's hash (pricing plan review M4). The cursor carries its order (`CursorV1.s`), and a
    continuation follows it, so every cursor minted before this decision still continues ascending, never 400
    `FILTER_MISMATCH`.
  - `$orderby` beside a cursor is the toolkit's 400 `ORDER_WITH_CURSOR`, judged first: a continuation sends only its cursor.
- **The merge contract** (shared with pricing D-470). A client merging the two gears' pages compares `submitted_at` as an
  instant, never as text: the RFC 3339 rendering trims trailing zeros of the fraction and writes UTC as `Z`, so as strings
  `…:00Z` sorts after `…:00.5Z` (the P-D-213 trap). It then compares the unit id as lower-case hex, in the same direction.
- **The tests.** `api/rest/sku_governance_tests.rs`: the counts against the list's own pages under seven narrowings, and the
  list's refusals refused alike by the counts; the counts in one grouped statement for 10 and 100 units (in the list's
  statement test); the newest-first order and every page size over a three-way tie; a cursor minted before this decision (its
  narrowing hash `f71fffbdfa52de1f`, pinned as a literal), each order's cursor with the same hash and its own order,
  `ORDER_WITH_CURSOR` and `INVALID_ORDERBY_FIELD`. `gear_tests.rs`: the counts op's 503, parameters, schema and text, and the
  list's; the operation census and the door census (`DOOR_ACTIONS`) name `bss_products.count_approval_units`.

**Source:** Owner, 2026-09-30 (the approvals option 1, "ok"); pricing phase 9 plan rev 2 (decision 10; plan review M4, L11).
Amends P-D-224 (the order).

**Amended by the phase 9 review, theme C and R32 (fix run 9.5d-1, 2026-10-01).**
- **The kind is a closed set.** The list and the counts take a kind products records, `sku_publish`, `sku_change` or
  `sku_retire`: any other kind, an empty one or another case included, is 400 `VALIDATION` on `kind`, judged with the
  narrowing, before the filter or the cursor's hash is built. Before, any text was taken, of any length, and counted
  zero. The repository reads a stored unit's kind through the same set (`domain::approvals::ApprovalKind`: every unit
  read, `count_units`' rows, and the SKU history's unit kind), and `UnitDto.kind` is the enum `ProductsApprovalKind`
  (P-D-217). So the list, the card, the receipts, the votes and the counts refuse a unit of another kind alike, with
  a 500 that does not name it; before, the counts answered 500 while the list and the card served it. The cursor's
  hash is unchanged: it hashes the kind's stored name, as before (`f71fffbdfa52de1f` stays pinned).
- **The counts read outside any transaction.** The one grouped statement runs on the plain connection, never under
  `category_tx_config`'s serializable transaction: one statement is its own snapshot, and SSI read locks over the
  scanned units could push concurrent submits and votes into serialization failures. The list's statement test pins
  the counts' statement outside any transaction.
- **The tests.** `api/rest/sku_governance_tests.rs`: `kind=promotion`, an empty kind, `SKU_PUBLISH` and a kind of 5000
  characters are refused alike by the list and the counts, on `kind`; a unit of an unknown kind is 500 on the list
  (bare, by state, by SKU), the counts (bare, by state) and the card, while a narrowing that does not keep it serves;
  a state written around its CHECK is 500 on the list, the counts and the card. `gear_tests.rs`: the kind is an enum,
  and the counts' text names the refusal.

**Amended by the phase 9 review, theme I (fix run 9.5d-2, 2026-10-01; R66, R36; the twin of pricing D-470's).**
- **The order is declared through the toolkit.** The list declares it with `.with_odata_orderby::<UnitOrderField>()`, a
  one-field enum (`submitted_at`), so the served contract lists `submitted_at asc` and `submitted_at desc` under
  `x-odata-orderby`, as the SKU and category lists publish theirs, and `$orderby` is that one parameter (its text is the
  toolkit's; the op's text keeps the default, the tie-break and the refusals). The door accepts exactly the declared field.
- **The parse stays the list's own.** The toolkit's `OData` extractor keeps the rules above but answers other requests
  differently: it refuses `limit=0`, of which the list reads one unit as the house pager does, with 400 `INVALID_LIMIT`;
  its refusal of a cursor that does not read drops the cause; and, since the list's query ignores a key it does not
  know, it would bind `$top` and `$skiptoken` and parse `$filter` and `$select` that the list ignores today, and refuse
  `$count` and `$skip`. The served behaviour is unchanged.
- **One source for the order.** Without a cursor the door puts the order on the page's query
  (`approval_repo::submission_order`: `submitted_at`, then the id, in one direction), and `approval_repo::page_units`
  reads it from there alone, ascending when the query names none; a continuation follows its cursor's order. The page
  takes no separate direction, which a cursor silently overrode.
- **The tests.** `gear_tests.rs`: the declared fields and the one `$orderby`, and no order on the counts.
  `api/rest/sku_governance_tests.rs`: `limit=0` reads one unit in both orders.

**Amended by P-D-250 (2026-10-01).** The shared order is also the merge key of the approvals inbox (`bss-approvals`,
AP-D-2): the inbox merges the two gears' pages by `(submitted_at, id)` itself, so the merge contract above is now the
inbox's as well as a client's. This gear's source reads its page through this list's own read and pager, with a cursor
it builds from the inbox's key.

#### P-D-228 [M] A unit says whether its reader may approve it (twin of pricing D-471)

**Status:** DECIDED 2026-09-30.

The approvals screen shows the Approve action only to a reviewer the vote door would take (ask 28). Separation of duties
excludes the submitter and every author of the unit's items, and a products item's author is the SKU's creator
(`domain/approvals/publish.rs`, `change.rs`), who is often not the submitter of a change or a retire (pricing plan review
H1). The list and the card read no unit items, so the screen could not judge it.

- **The field.** Every `UnitDto` carries `caller_can_approve`, a boolean: on `GET /approval-units`, `GET /approval-units/{id}`
  and every receipt that carries a unit (the submit, change and retire receipts, and the vote receipts). It is true when the
  caller may approve the unit now.
- **One rule (pricing W2, D-459).** It is `bss_approval::approve_eligibility(unit, items, decisions, caller).refusal.is_none()`:
  the predicate `Engine::approve` judges through, over the unit's STORED items (the current generation's, as
  `Engine::approve` reads them) and its decisions. So it is false for a decided unit, for the submitter and for the SKU's
  creator (`SOD_VIOLATION`), and for a caller who voted in the current generation (`DUPLICATE_VOTE`); a vote that a refresh
  made stale does not stop its voter.
- **Approve only (pricing plan review M2).** `Engine::reject` judges no separation of duties, so the submitter and the SKU's
  creator may reject a unit whose flag is false. The reject door's served text claimed 403 `SOD_VIOLATION`; it now says that a
  reject judges no separation of duties, and names 409 `UNIT_ALREADY_DECIDED`.
- **Not the grant.** The flag does not judge products approve on approval units: without it the vote door still answers 403.
  The field's text says so.
- **The reads (amends P-D-224).** The list's page adds ONE grouped read of its units' items
  (`approval_repo::items_of_units`, the twin of pricing's): a page reads its units, all their decisions and all their items,
  three statements whatever its size; the QueryRecorder pins three for 10 and for 100 units. The card reads the unit's stored
  items (`store.items`) beside its decisions. A receipt reads the unit's items and decisions.
- **The tests.** `api/rest/sku_governance_tests.rs`: a publish unit at quorum 3 of a SKU created by one user and submitted by
  another, a vote in generation 1, a content drift that refreshes the unit to generation 2 (400 `UNIT_STALE`), a vote in
  generation 2; for the SKU's creator, the submitter, the voter of this generation, a fresh reviewer and the voter of the
  earlier generation, the flag on the card and in the list (which agree) is exactly whether the vote door answers 200; on the
  decided unit it is false for everyone and the door answers 409 `UNIT_ALREADY_DECIDED`; the submit and vote receipts answer
  false for their caller. The submitter's reject answers 200 while the submitter's and the creator's flags are false. The
  list's statement test pins three statements with the items read. `gear_tests.rs`: the field is a required boolean whose
  text says Approve only and 403, the list's text names it, and the reject's text claims no `SOD_VIOLATION`.

**Source:** Owner, 2026-09-30 (validation 3 item 4, "ok"); pricing phase 9 plan rev 2 (decision 11; W2, binding; plan review
H1, M2). Amends P-D-224 (the page's statements).

**Amended by the phase 9 review, theme E (fix run 9.5d-1, 2026-10-01; R70, R72, R73, R74).** The flag reads only who
authored a unit's items: `bss_approval::approve_eligibility` takes the authors (pricing D-459), and the page reads its
units' item authors alone (`approval_repo::item_authors_of_units`: each item's unit and author, one statement), not the
items with their before and after content; still three statements per page. The card and every receipt (the submit,
change and retire receipts and the vote receipts) read their unit's item authors the same way, beside its decisions;
the engine's own approve still reads its items. `api/rest/sku_governance_tests.rs`: the list's statement test pins the
projection, and the card, the submit receipt and the vote receipt read the authors alone.

#### P-D-229 [H] A derived usage meter is a catalog declaration that Rating evaluates

**Status:** DECIDED 2026-10-01.

A derived (composite) usage meter computes one quantity from other usage. A cloudlet, for example, is 128 MB of RAM and
400 MHz of CPU, and a cloudlet-hour is computed from the RAM and CPU usage of the hour. The old implementation had such meters.
The owner decided who does what:
- **Products declares it.** A derived usage type, with an immutable version, names:
  - its input usage types, each at an exact version (at least two);
  - the formula as data: how the input quantities combine (for example the larger of the two shares), and the rounding;
  - the granularity the formula applies at (per rating window, for example per UTC hour);
  - its output unit.

  A new formula is a new version; a published version never changes. What a cloudlet is, is a product decision, versioned
  with the catalog.
- **Rating evaluates it** (rating T-D-39), per subscription line and rating window. It folds each input over the window as
  that input's own meter declares, applies the formula, then prices the output. The order matters: the larger share in each
  hour is not the larger of the hourly sums.
- **The usage collector reports raw meters only.** It is not asked to compute a derived quantity, and the raw levels stay
  available for audit and for re-rating a past period.
- **Pricing references a derived usage type exactly as a raw one.** A usage entry's rating policy names the usage type and
  its version. The meter-semantics provider of the pricing seam plan (its external dependency E1) answers:
  - a derived type from this declaration: the canonical unit, the inputs and their versions, the formula version;
  - a raw type from the collector and the types registry.
- **Not built yet.** Today a usage SKU declares exactly one metering unit, and Products has no formula store
  (`gears/bss/rating/docs/SEAMS.md` RG2). Building the declaration (storage, authoring, and the read the provider serves) is a
  separate run. Until then no derived meter can be sold.
- **Supersedes:**
  - the PriceBook spec's §3 item 11 disposition ("D — metering's concern") for derived meters; level aggregation stays
    Rating's (rating T-D-17);
  - Rating's "declared and delivered by the pricing gear (Slice 10)": that legacy pricing gear is gone (pricing D-423).

**Source:** Owner, 2026-10-01 (asked who should compute a cloudlet from RAM and CPU usage; chose "Products declares, Rating
evaluates, the usage collector stays raw").

**Amendment (2026-10-01, the implementation plan review; approved by the owner).**
- **Exact inputs.** An input is a GTS usage type id, and its version segment is the exact version.
- **The input folds.** The declaration states each input's granule fold (`Sum`, `Peak`, `TimeWeighted`), because no raw meter
  declaration exists to read it from.
- **Granules.** The formula applies per granule (an hour), and a window's output is the sum of its granule outputs.
- **The pricing reference.** In pricing, the meter is named by the SKU's own ref word for word:
  `products.derived/<code>@<n>`, version `<n>`.
- **The pin.** A usage SKU's derived ref is fixed at its first publish. A new formula version is sold through a new SKU, as a
  usage chain's metering is fixed (pricing D-402).

#### P-D-248 [H] A retire under review keeps the SKU's lifecycle

**Status:** DECIDED 2026-10-01.

`retiring` is not a SKU lifecycle. While a `sku_retire` unit is in review the SKU keeps `published` or `deprecated` and `retire_pending` is true, the twin of `type_change_pending`. Apply sets `retired` and clears the flag. Reject, withdraw, `POST /skus/{id}/unfence` and the orphan-fence recovery clear the flag and do not change the lifecycle. Migration `m20261001_000011_sku_lifecycle_honesty` adds the flag, converts a stored `retiring` row to `lifecycle = fence_prior_lifecycle` with the flag set, drops `fence_prior_lifecycle`, and tightens the lifecycle CHECK to draft, published, deprecated and retired. On SQLite it rebuilds the whole `m000007` family. A new reservation on a retire-pending SKU is `SKU_FENCED`. The counts drop `retiring`. `$filter` gains `retire_pending`. `lifecycle eq 'retiring'` is 400. The history maps a legacy `retiring` token at read time, on the raw strings before `Lifecycle::parse`, so the tab never shows it and the read is never a 500. New acts record the move they serve: a retire submit, reject, withdraw, unfence or expiry is no move, and an apply is `L → retired`.

Pricing reads `retire_pending` instead of a `retiring` lifecycle. An entry or item create answers `SKU_RETIRING`. The revision checks answer `ITEM_SKU_UNAVAILABLE`. A lost reference is not re-reserved.

**Source:** Owner, 2026-10-01 ("давай уберем этот статус и сделаем что он еще не retired пока не согласовали а сохраняется старый статус"). Phase 9 plan rev 4, run 9.8d. Amends P-D-189, P-D-208, P-D-211 and P-D-213.

#### P-D-249 [H] A lifecycle change honours its date

**Status:** DECIDED 2026-10-01.

A `sku_change` whose `effective_from` is after today stores `lifecycle_next` and `lifecycle_next_from` and leaves `lifecycle` as it is. A change dated today or earlier sets `lifecycle` now. The lifecycle in force on a day is `lifecycle_next` when `lifecycle_next_from` has arrived, otherwise `lifecycle`. One Rust function, `effective_lifecycle`, serves the SDK `Sku.lifecycle` and `SkuDto.lifecycle`. One SQL `CASE`, bound to the gear clock's today, serves the list filter, the counts and the other lifecycle predicates. `SkuDto.lifecycle_next` is `{ lifecycle, from }`, null when none is pending. A later change replaces a pending next, or clears it when the target is the lifecycle in force. A retire apply clears it. The first statement of a head write folds a due next into `lifecycle`, so `set_lifecycle` sees the lifecycle in force. A read does not depend on that fold. The same migration as P-D-248 adds the two columns. They are both null or both set, and a next lifecycle is never `retired`.

**Source:** Owner, 2026-10-01. Phase 9 plan rev 4, run 9.8d. Amends P-D-191.

#### P-D-250 [M] The approval units answer the approvals inbox through this gear's own doors (twin of pricing D-490)

**Status:** DECIDED 2026-10-01.

The approvals inbox (`bss-approvals`, AP-D-1 to AP-D-4) serves ONE paged list, ONE count, ONE card and ONE vote door
over the approval units of every BSS gear. The units stay in their gears. Each gear implements the inbox's source port,
`bss_approvals_sdk::ApprovalSourceV1`, over its own door functions, and the inbox asks it AS THE CALLER. This entry is
this gear's side; pricing D-490 is its twin.

- **The source.** `api::rest::approval_units::inbox_source::ProductsApprovalSource`. `gear.rs` registers it at init in
  the ClientHub as `dyn ApprovalSourceV1`, scoped `ClientScope::new("products")`, over the gear's own `ApiState` and
  `PolicyEnforcer`. It copies no rule of a door: it calls the doors.
- **The page.** It is the list door's own read, `approval_units::page_of` (the list handler's transaction, taken out of
  the handler so both call it), under the list's grant (products read on approval units) and its narrowing
  (`approval_units::narrowing`). So the page has the list's refusals, decisions, item authors and statements.
  - **The keyset (pricing plan review H1, L1).** The inbox asks for up to `limit` units strictly after its key
    `(submitted_at, id)` for this source, or from the start. The source builds the pager's own `CursorV1` for that key:
    `s` is `+submitted_at,+id` or `-submitted_at,-id`, each value of `k` is encoded by the pager's codec
    (`encode_cursor_value`) under the list mapping's cursor kind, `f` is empty and `d` is `fwd`. `page_units` then
    compares the columns as it does for its own cursors. There is no second SQL predicate and no compare of text.
    `has_more` is whether the pager minted a next cursor.
  - The order is P-D-227's, which this entry makes the inbox's merge key. Postgres orders it exactly; `SQLite` keeps
    `submitted_at` as text, so the exact order of a walk is proved on Postgres. The source never parses `$orderby`
    and never reads the gear's cursor token. The phase 9 review's theme I (9.5d-2) changes how the list declares its
    `$orderby`, not this read.
- **The counts (pricing plan review M5).** The counts door's handler: one grouped statement on the plain connection,
  never in the list's serializable transaction (R32).
- **The card.** The card door's handler. Its 404 is the source's `None`, a unit this tenant does not hold. The facade's
  owner resolution asks every source (AP-D-3).
- **`subject_live` is the card's `impact_live` (pricing plan review M1)**: the live SKU head, null once a rejected or
  withdrawn draft was deleted (P-D-206). The list door serves no `impact_live`, so a list item's `subject_live` is null.
- **The impact (pricing plan review M2).** A `sku_change` or `sku_retire` unit's `impact` is pricing's usage of its SKU
  (`SkuUsage { entries, currencies, prices, plans }`). The source asks `SkuUsageV1::usage` ONCE per page, through the
  SKU read's own helper (`api::rest::usage::of`): on a task of its own, bounded, outside any transaction. A refusal (a
  caller without pricing's `price_book_entry` read), an outage, a late answer or a missing port is `impact: null` on
  those units, never a failed read (P-D-197). A `sku_publish` unit's impact is null. `usage_sets` is never called. The
  inbox's `impact=false` skips the call in the source; the list door takes no such parameter, so nothing is forwarded.
- **A vote (pricing plan review H3).** The source sends the vote to the vote door itself, through this gear's
  approval-unit router under its enforcer and the platform's error layer, as the gateway serves the door. The door
  therefore judges its own grant (products approve, or submit for a withdraw), separation of duties, quorum, generation
  and staleness, and keys the replay row under its own endpoint, `/bss-products/v1/approval-units/{id}/<action>`
  (P-D-198). The request body is the caller's exact bytes, with the caller's `Idempotency-Key`. The answer is the door's
  status, headers and body, unchanged: the receipt `{ have, need, outcome, unit }`, or the refusal with its `instance`
  (the door's path) and, on `GENERATION_MISMATCH` and `UNIT_STALE`, its `generation`. So a vote through the inbox and
  one through the door with the same key and body are ONE vote; the same key with another body is 409
  `IDEMPOTENCY_CONFLICT`.
- **What products holds nothing under (pricing plan review H4).** A kind products does not record, and any `book_id`
  (products holds no book), are an empty page and zero counts, decided in the source before the grant and before any
  door. The door itself refuses the kind, ignores the list's `book_id` (its query does not deny unknown keys) and refuses
  the counts' one. A state the door refuses is that 400 for the inbox's whole read.
- **`ApiState`'s fields are public.** `fence_ttl_minutes` and `reference_principals` were crate-visible; the inbox's
  Postgres walk builds an `ApiState` from outside the crate. `gear.rs` stays the only production writer.
- **The tests.** `api/rest/approval_units/inbox_source_tests.rs` holds the census: the same request through the door
  (the gear's router under the platform's error layer) and through the source answers equal status, code and body
  bytes. It covers the list in both orders after any key under six narrowings; the list's and the counts' refusals (an
  unknown state, the grant), rendered at the door's path through the same layer; the counts under five narrowings; the
  card, its miss and its grant; every vote refusal (an unreadable body, a missing `generation`, `GENERATION_MISMATCH`,
  `NOTE_REQUIRED`, `NOTE_TOO_LONG`, `SOD_VIOLATION`, a withdraw by another, the grant, 404, `DUPLICATE_VOTE`,
  `IDEMPOTENCY_CONFLICT`, `UNIT_ALREADY_DECIDED`); and `UNIT_STALE` with its `generation`. A source vote and a door vote
  with one key replay once. The QueryRecorder pins the page at three statements on products' tables for 10 and for 100
  units, every one in the list's transaction, and the counts at one statement outside any transaction.
  `api/rest/approval_units/inbox_e2e_tests.rs` runs the facade gear over BOTH real gears in one process (the
  cross-gear precedent is `sku_governance_tests`' real pricing entry): a walk over prices, plan revisions and the three
  SKU kinds in both orders at every page size, each unit once; a pricing-only approver sees products forbidden; a facade
  vote and a direct vote with one key replay once in both gears, and another body is the door's
  `IDEMPOTENCY_CONFLICT`; separation of duties is the door's refusal, byte for byte; `impact: null` for a caller
  without pricing's entry read; a foreign kind and products' `book_id` are empty, not 400; and the facade adds no
  statement beyond its sources' (the list, the counts off any transaction, the card). `tests/postgres_approvals_inbox.rs`
  walks both gears on Postgres in the exact `(submitted_at, id)` order both ways, with sub-second instants and ties
  within one gear and across the two.

**Source:** Owner, 2026-10-01 (asked how to merge the two approval-unit methods into one, then "yes, A, agreed" for
the read-and-route facade, then "write the plan"; Run 2 started before 9.5d-2 on the owner's word). Approvals inbox
plan rev 2 (Run 2; design 1 and 3; plan review H1, H3, H4, M1, M2, M5, L1). Amends P-D-227: its order is the inbox's
merge key.
