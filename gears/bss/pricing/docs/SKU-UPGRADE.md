# D-372: preserve legacy data across migrations 44 and 45

Migration 44 requires an offer SKU on every plan revision and a SKU on every
price row. A usage row's SKU is an authoritative registry fact. Never infer it
from a meter spelling, the local development catalogue, or a similar unit name.
Migrations 1–43 are unchanged. Do not mark 44 applied manually or renumber 45.

Stop all old pricing writers and keep them stopped from census through migration
45. Back up the database and verify recovery before beginning. Inspect the
actual gear migration-history table: 43 must be applied and 44/45 absent. The
production runner uses `toolkit_migrations__*`, while SeaORM test databases use
`seaql_migrations`; do not substitute one ledger for the other. Preserve a census
of tenant, plan, revision, price, meter and lifecycle state and the existing DDL.

Obtain explicit assignments keyed by `(tenant_id, plan_id, revision)` for missing
plan SKUs and `(tenant_id, price_id)` for price SKUs. Verify each against the
registry: published offer for a plan; the same offer or a permitted unsellable
resource for its price; charge kind and declared meter must agree exactly. Reject
unknown, nil, duplicate, cross-tenant or unused assignments and new scope-key
collisions. These facts cannot be validated by this database migration. Do not
use the UUIDs in test fixtures as deployment mappings.

The following SQL is a procedure template: bind the named parameters from the
reviewed registry assignments, one row at a time. Check that every update affects
exactly one row. Never remove tenant predicates. Incomplete staging may be
committed intentionally: migration 44 will refuse and retain the column and all
explicit assignments for a later retry.

## PostgreSQL

Use `psql` with `ON_ERROR_STOP` enabled. Verify the column is absent before the
first expansion; on a retry inspect its type/nullability/default in
`information_schema.columns` and its identity/generated flags. Only an ordinary
nullable `uuid` without default or key role is supported. Add no constraints,
indexes or triggers during staging.

```sql
\set ON_ERROR_STOP on
BEGIN;
LOCK TABLE bss.pricing_plan, bss.pricing_price IN ACCESS EXCLUSIVE MODE;
-- First staging only; skip after verifying the existing column on a retry.
ALTER TABLE bss.pricing_price ADD COLUMN sku_id uuid;
-- Set psql variables tenant_id, price_id and sku_id to reviewed UUIDs first.
UPDATE bss.pricing_price SET sku_id = :'sku_id'::uuid
 WHERE tenant_id = :'tenant_id'::uuid AND price_id = :'price_id'::uuid;
-- For each missing draft plan assignment, also bind plan_id and revision.
UPDATE bss.pricing_plan SET sku_id = :'sku_id'::uuid
 WHERE tenant_id = :'tenant_id'::uuid AND plan_id = :'plan_id'::uuid
   AND revision = :'revision'::bigint AND lifecycle_state = 'draft'
   AND sku_id IS NULL;
-- Review UPDATE counts and assignments before committing; otherwise ROLLBACK.
COMMIT;
```

For a missing SKU on a **non-draft** plan, ordinary UPDATE is deliberately refused.
This additional maintenance prerequisite must be performed in one bounded
transaction while writers remain stopped. Preserve the exact trigger definition
(`pg_get_triggerdef`) and function definition (`pg_get_functiondef`) before the
operation and require the trigger's `tgenabled` to equal `O`. Acquire the same
locks above. Disable only `trg_pricing_plan_append_only`, apply the tenant/plan/
revision-scoped assignment with `sku_id IS NULL`, then enable that same trigger
before commit. Never change the lifecycle state or use `session_replication_role`.

```sql
-- Inside the locked transaction, after checking/capturing the existing guard:
ALTER TABLE bss.pricing_plan DISABLE TRIGGER trg_pricing_plan_append_only;
UPDATE bss.pricing_plan SET sku_id = :'sku_id'::uuid
 WHERE tenant_id = :'tenant_id'::uuid AND plan_id = :'plan_id'::uuid
   AND revision = :'revision'::bigint AND sku_id IS NULL;
ALTER TABLE bss.pricing_plan ENABLE TRIGGER trg_pricing_plan_append_only;
-- Compare definitions and tgenabled with the captured originals before COMMIT.
-- Any error or mismatch: ROLLBACK, which restores assignments and trigger state.
```

## SQLite

Use the `sqlite3` shell with `.bail on` and `PRAGMA foreign_keys = ON`. Start
`BEGIN IMMEDIATE` before census checks and staging. Inspect
`PRAGMA table_xinfo(pricing_price)`: a retained `sku_id` must be an ordinary,
nullable `TEXT` column with no default, primary-key role or generated/hidden flag.
Add no other schema objects. UUIDs bound by the ORM are 16-byte blobs; legacy raw
SQL fixtures can hold UUID text. Bind tenant/plan/price identifiers in their
existing storage representation; inspect `typeof(...)` first. For SKU assignments
use canonical non-nil UUID text or a 16-byte UUID blob. Migration 44 validates both
and normalizes price SKUs to blobs before applying uniqueness constraints.

```sql
.bail on
PRAGMA foreign_keys = ON;
BEGIN IMMEDIATE;
-- First staging only; skip after checking table_xinfo on retry.
ALTER TABLE pricing_price ADD COLUMN sku_id TEXT;
-- Bind @tenant_id, @price_id, @sku_id from the reviewed assignments with
-- .parameter set; use a quoted X'32 hex digits' SQL expression for a blob ID.
UPDATE pricing_price SET sku_id = @sku_id
 WHERE tenant_id = @tenant_id AND price_id = @price_id;
SELECT changes(); -- must be exactly 1 for each assignment
UPDATE pricing_plan SET sku_id = @sku_id
 WHERE tenant_id = @tenant_id AND plan_id = @plan_id
   AND revision = @revision AND lifecycle_state = 'draft' AND sku_id IS NULL;
SELECT changes(); -- must be exactly 1
-- Inspect coverage and exact values before COMMIT, otherwise ROLLBACK.
COMMIT;
```

For non-draft plan assignments, preserve and temporarily remove **both** the
frozen-column and lifecycle-flip guards. SQLite's flip guard also refuses an
UPDATE that leaves the non-draft state unchanged. Inside the same
`BEGIN IMMEDIATE` transaction, capture the two original statements:

```sql
.headers off
.mode list
.once /absolute/operator-controlled/path/restore-pricing-plan-guards.sql
SELECT sql || ';' FROM sqlite_master WHERE type = 'trigger' AND name IN
 ('trg_pricing_plan_frozen_columns', 'trg_pricing_plan_flip_whitelist')
 ORDER BY name;
```

Verify the file contains exactly the two expected original CREATE TRIGGER
statements, with no edits. Keep a copy of this original DDL for comparison. Then:

```sql
DROP TRIGGER trg_pricing_plan_frozen_columns;
DROP TRIGGER trg_pricing_plan_flip_whitelist;
UPDATE pricing_plan SET sku_id = @sku_id
 WHERE tenant_id = @tenant_id AND plan_id = @plan_id
   AND revision = @revision AND sku_id IS NULL;
SELECT changes(); -- exactly 1; repeat only for reviewed assignments
.read /absolute/operator-controlled/path/restore-pricing-plan-guards.sql
-- Read sqlite_master again and compare both definitions exactly before COMMIT.
-- Any mismatch or failed statement: ROLLBACK; closing the shell also rolls back.
```

Do not commit with either guard absent. Restoration failure must abort the whole
transaction. This is a prerequisite for explicit maintenance assignments, never
an application write path. Keep foreign keys enabled throughout.

## Apply and verify

With writers still stopped, run the normal migration runner from the new build.
On the ordinary absent-column path, refusal rolls back the new column and any
automatic fee assignments. Follow the staging procedure before assigning usage
SKUs. On the pre-staged path, refusal rolls back only that attempt: the column
and explicit assignments remain. Complete or correct them and retry. Automatic
fee backfill only fills NULL values and never overwrites an explicit fee mapping.
Malformed or nil UUIDs and incompatible column shapes refuse before backfill.

Migration 45 additionally requires consistent legacy descriptors across plan
revisions, with complete descriptors for non-draft price rows. Resolve any such
refusal through a separately reviewed data repair; do not invent descriptors.
After success, verify both migration-history entries, original row/child counts,
exact SKU assignments, descriptor values, NOT NULL constraints, scope-key indexes
and foreign keys (`PRAGMA foreign_key_check` on SQLite). Verify the original plan
and new price append-only guards refuse an attempted non-draft SKU update inside
a transaction that is rolled back. Resume writers only after these checks pass.
