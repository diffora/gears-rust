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
| P-D-213 | M | A SKU's history: every audit row on a SKU carries the lifecycle move its act made, and `GET /skus/{id}/history` reads them | DECIDED 2026-09-27 · Owner, 2026-09-27; phase 6 plan rev 2; amends P-D-189, P-D-200 |
| P-D-214 | L | SKU versions answer one shape each: the history an array, the version in force at `versions/as-of?date=` | DECIDED 2026-09-27 · Owner, 2026-09-27; phase 6 plan rev 2 |

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
- The 09-22 catalog design named the path `/bss-products/v1/catalog/usage-types` and gated it on
  `recognized_set × read`; that resource no longer exists, and picking a usage type is authoring a SKU, so the
  path is `/usage-types` and the gate the author grant.

Deploy note: SKU authors, submitters and approvers of usage SKUs need usage-collector read, granted with their
role; without it submit and approve answer 403 `USAGE_TYPE_FORBIDDEN`.

**Source:** Owner, 2026-09-27 (option b); phase 6 plan rev 2 (validation D5, asks 6 and 13; plan review H2, L9).

#### P-D-208 [M] A retired SKU no longer keeps its category in use

Amends P-D-186 (#9). Category retirement is refused (409 `CATEGORY_IN_USE`) only while a SKU in `draft`,
`published`, `deprecated` or `retiring` names the category. A `retired` SKU no longer counts: nothing moves a
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
  `gl_code`, the caller's `%`, `_` and `\` escaped. `lower()` folds ASCII only on SQLite and Unicode on
  Postgres, so a non-ASCII letter matches another case of itself on Postgres only; the two backends differ
  there (`nfr-two-backends`), and both are pinned by tests. An empty `q` is no search. The toolkit's
  `contains` in `$filter` follows the backend's `LIKE` (case-sensitive on Postgres); `q` is the
  case-insensitive search.
- **Refusals.** Any key besides `limit`, `cursor`, `q`, P-D-212's `priced` and `in_plan`, and the OData
  options is 400 `UNSUPPORTED_QUERY_PARAM`, every offender named (a products copy of ledger's
  `reject_non_odata_list_params_allowing`); a plain key given twice is 400. `$select` and `$count` are 400.
  The cursor carries a hash of `$filter`, `q`, `priced` and `in_plan`: a cursor replayed with other values is
  400 `FILTER_MISMATCH`. Authorization is judged first (`sku × read`).
- **The transaction.** As before, the list recovers the tenant's orphan fences and reads its page in one
  transaction (P-D-189).

The toolkit change this rests on is recorded in the toolkit's own docs
(`docs/web-docs/build-with-gears/add-pagination-odata.md`, `libs/toolkit-db/src/odata/README.md`): its typed
`$filter` path admits `null` with `eq` and `ne` on any field kind (and still refuses it inside `in`), and
`contains`, `startswith` and `endswith` emit `LIKE … ESCAPE '\'` with the pattern escaped. SQLite has no
default escape character, so the browse search (`search_skus`, a `startswith` on the name) matched nothing
for a name holding `%`, `_` or `\` until then. P-D-196's owed `category=none` browse is `category_id eq null`.

Every products query parameter is declared with its type in the served contract (`limit` integer,
`include_released` boolean, the rest string; validation D4).

Breaking: the list's parameters and its envelope (`next` is `page_info.next_cursor`). The gears-rust e2e
follows in this run; vhp-core's e2e in phase 6.6.

**Source:** Owner, 2026-09-27; phase 6 plan rev 2 (owner decision 1; asks 1, 2 and 4; validation D4, D7; plan
review H1, L1–L5, L10).

#### P-D-211 [M] The SKU list's tab counts: `GET /skus/counts`

`GET /skus/counts` answers `{ all, draft, published, deprecated, retiring, retired, in_review }` for the tabs
of the SKUs screen: every SKU the narrowing keeps, those in each lifecycle, and those a pending unit locks
(`pending_unit_id` set, in any lifecycle). It narrows as the list does, by `q`, P-D-212's `priced` and `in_plan`, and `$filter`, except that `$filter`'s `lifecycle` terms are dropped, because the counts count every
lifecycle. Only a term that is a top-level `and` conjunct is dropped; a `lifecycle` term under `or` or `not`
cannot go without changing what the rest means, so it is 400 `INVALID_FILTER`. The whole filter is checked as
the list reads it before the terms go. `$orderby`, `$top`/`limit`, `cursor`/`$skiptoken` and `$select` are
400 `UNSUPPORTED_QUERY_PARAM`. It recovers the tenant's orphan fences in its own transaction, as the list
does, so its `retiring` agrees with the list, and it counts in one grouped statement whatever the number of
SKUs. Authorization is `sku × read`.

**Source:** Owner, 2026-09-27; phase 6 plan rev 2 (ask 1; plan review M2).

#### P-D-212 [M] The SKU list filters on pricing's usage (`priced`, `in_plan`) through the port's sets

Amends P-D-197. The SKUs screen filters on what the usage shows, and a list that pages cannot filter a
page's `usage` on the client, so the list and the counts take `priced=true|false` and `in_plan=true|false`.

- **The definitions are the usage's own** (pricing D-428): `priced` keeps the SKUs whose `usage.entries` is
  above zero (an entry in any book of the tenant, in any reference state), `in_plan` those whose `usage.plans`
  is above zero (a draft, pending or published revision names one of the SKU's entries). A plan item that
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
| `approval.submit` | `sku_retire` | L, or `retiring` when an orphan fence is resumed | `retiring` |
| `approval.vote`, `approval.refreshed` | any | the lifecycle found | the same |
| `approval.applied` (the apply at submit, quorum 0), `approval.approved` | `sku_publish` | `draft` | `published` |
| `approval.applied`, `approval.approved` | `sku_change` | L | the proposed lifecycle, or L |
| `approval.applied`, `approval.approved` | `sku_retire` | `retiring` | `retired` |
| `approval.rejected`, `approval.withdrawn` | `sku_publish` | `draft` | `draft` |
| `approval.rejected`, `approval.withdrawn` | `sku_change` | L | L |
| `approval.rejected`, `approval.withdrawn` | `sku_retire` | `retiring` | the lifecycle before the fence |
| `sku.unfence` | none | `retiring` (retire fence) or L (type-change fence) | the lifecycle before the fence, or L |
| `sku.fence_expired` | none | as `sku.unfence` | as `sku.unfence` |

- **The orphan-fence expiry is audited** (amends P-D-189). Every SKU read runs the expiry: the list, the
  counts, the card, the versions, the references, the unit card, and the submit and reference doors. It
  lifted a fence without writing a row. It now writes `sku.fence_expired` on the SKU for each fence it
  lifts, with the move, the SKU's revision and the reason `fence_ttl_minutes=<n>`, in the read's own
  transaction. Its actor is the system: `actor_ref` is the nil uuid, the subject of the platform's system
  context, which no principal carries (`require_authenticated` refuses a nil subject). This is an audit
  actor, not an authorization subject; products still reads as the caller (P-D-207). The list's expiry
  reads the tenant's expired fences once and lifts each at the operation that holds it. A list with no
  expired fence makes one read, as before, and each fence it lifts adds its update and its row.
- **The read.** `GET /skus/{id}/history` answers `Page<ProductsSkuHistoryEntry>`: `{ items, page_info }`,
  each item `{ at, actor, action, from_lifecycle, to_lifecycle, unit_id, unit_kind, note }`. Its source is the
  audit rows whose subject is the SKU, and the rows whose subject is an approval unit whose `ref_id` is the
  SKU, in the caller's tenant. `at` is the row's `written_at`, `actor` its `actor_ref`, and `note` its
  `reason`: a decision's note, or the expiry's TTL. `unit_id` and `unit_kind` name the unit of a unit's row
  (one read of the page's units) and are null on a SKU's own row. The order is `(written_at, audit_id)`,
  oldest first. At quorum 0 the submit and its apply share one instant, so the id breaks the tie, and a
  page boundary inside a tie skips and repeats nothing. `audit_id` is a UUID v7, minted in write order. The
  toolkit's pager serves it: `$top` (alias `limit`) defaults to 50 and is clamped at 200, and `cursor`
  (alias `$skiptoken`) comes from `page_info`. Any other key is 400, and so are `$filter`, `$orderby`,
  `$select` and `$count`. The cursor carries a hash of the SKU id, so a cursor from another SKU's history is
  400 `FILTER_MISMATCH`. The read is authorized as the card is (`sku × read`), runs the SKU's orphan-fence
  expiry first, and answers 404 for a SKU the tenant does not hold and for a deleted draft. The
  `sku.delete` row stays in the log but is never read, because its SKU is gone. On SQLite `written_at` is
  RFC 3339 text and orders as text, as `updated_at` does in the SKU list (P-D-210). Postgres orders it as
  a timestamp.

**Source:** Owner, 2026-09-27; phase 6 plan rev 2 (ask 7; plan review H3).

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
