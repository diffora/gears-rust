# Feature: Clone & Templating

- [ ] `p2` - **ID**: `cpt-cf-bss-products-featstatus-clone-implemented`

<!-- reference to DECOMPOSITION entry -->
- [ ] `p2` - `cpt-cf-bss-products-feature-clone`

<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Clone an entity](#clone-an-entity)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [The disposition table](#the-disposition-table)
- [4. States (CDSL)](#4-states-cdsl)
- [5. Definitions of Done](#5-definitions-of-done)
  - [`cloned_from` lands on both entity tables, in the migrations that already name it](#cloned_from-lands-on-both-entity-tables-in-the-migrations-that-already-name-it)
  - [The create-only registry row, over a refusal that already ships](#the-create-only-registry-row-over-a-refusal-that-already-ships)
  - [The clone door](#the-clone-door)
  - [The source read surface, and the leak it closes](#the-source-read-surface-and-the-leak-it-closes)
  - [Identity: minted ids, a suggested code, and one reservation](#identity-minted-ids-a-suggested-code-and-one-reservation)
  - [The rename rule, and why it is not revival-only](#the-rename-rule-and-why-it-is-not-revival-only)
  - [The disposition matrix, registered as rules over one phase](#the-disposition-matrix-registered-as-rules-over-one-phase)
  - [The error vocabulary, twelve codes of which none has a variant](#the-error-vocabulary-twelve-codes-of-which-none-has-a-variant)
  - [Lineage, and the event that is deliberately absent](#lineage-and-the-event-that-is-deliberately-absent)
  - [The product-with-SKUs clone, and its honestly-reported partial](#the-product-with-skus-clone-and-its-honestly-reported-partial)
  - [The authz surface, and the roster it would redden](#the-authz-surface-and-the-roster-it-would-redden)
  - [The clone is audited as a refusal and as nothing else](#the-clone-is-audited-as-a-refusal-and-as-nothing-else)
  - [`design/11` §1.7's two design-introduced names exist as named seams](#design11-17s-two-design-introduced-names-exist-as-named-seams)
  - [The test posture, with a positive control per code](#the-test-posture-with-a-positive-control-per-code)
- [6. Acceptance Criteria](#6-acceptance-criteria)
- [7. Known unknowns](#7-known-unknowns)
  - [Carried verbatim from `design/11` §6](#carried-verbatim-from-design11-6)
  - [Raised here rather than carried](#raised-here-rather-than-carried)
  - [Owed to other documents, recorded and deliberately not edited](#owed-to-other-documents-recorded-and-deliberately-not-edited)

<!-- /toc -->

## 1. Feature Context

### 1.1 Overview

Clone creates a new `draft` Product or SKU from an existing one — draft, published, `deprecated` or
`retired` — with new identity, an explicit per-field-class copy/reset disposition, **live
re-validation** of every vocabulary reference the copy carries (unit, tier, category, attribute
definitions, codes — `usageTypeRef` excepted, §5), a `clonedFrom` lineage pointer, and
the rename that absolute name uniqueness forces on every same-brand Product clone.

It is the registry's **only sanctioned revival path** for a `retired` entity, and — because
`deprecated` is a governed sub-state of `published` occupying the whole retirement lead window — it
is also how an operator builds the successor a retirement's `replacedBy` names.

The act is a **create**, not a transition. It writes a new row through the ordinary Foundation create
door and never touches the source.

### 1.2 Purpose

Copying structure is cheap; copying **stale classifications** is how a defect from last year
republishes itself. The disposition table and the live re-validation are the whole design: what
copies, what resets, and what must be re-proven against today's vocabularies.

The second purpose is narrower and is the reason the feature is not merely a convenience: without a
clone path, a `retired` entity is unrecoverable, because `retired` is terminal and there is no
un-retire. `PRD.md` §1's glossary says so in as many words — *"`retired` is terminal (revival only
via clone)"* — and §6's lifecycle requirement repeats the parenthesis against the state machine. So
this feature is the named remedy for a state the lifecycle deliberately admits no exit from.

### 1.3 Actors

| Actor | Role |
|-------|------|
| `cpt-cf-bss-products-actor-product-manager` | Clones draft, published, `deprecated` and `retired` sources into new drafts |

`PRD.md` §6.10's `fr-clone` names this actor and `design/11` §1.3 names the same one. **A second
candidate is recorded and not adopted**: `PRD.md` §11's operational-console story assigns clone to an
*"Operator/Platform owner"*. The two are not reconciled by any document, and reconciling them here
would be a product decision — it is §7's, routed to the PRD owner with `05-governance`.

### 1.4 References

- [`../PRD.md`](../PRD.md) §6.10 (`cpt-cf-bss-products-fr-clone`), §12 AC #34, AC #38 (the two rows
  **P-D-44** split: authoring or cloning against a **de-listed** unit, and against a **deprecated**
  one); §1's glossary, which makes clone the only revival path out of `retired`, and
  `fr-field-mutability-matrix` in §6, which names retire-and-clone as the only remedy for a mis-set
  structural identity.
- [`../DECISIONS.md`](../DECISIONS.md) **P-D-04** (absolute product-name uniqueness — the interaction
  this feature's rename rule resolves), **P-D-06** (the metadata map lives outside frozen version
  content), **P-D-21** (the event stream is the success-path record), **P-D-25** (one
  `DUPLICATE_CODE` for both reservations), **P-D-37** (one code per audit row, every violation in the
  answer), **P-D-47** (`ACCOUNTING_CODE_DEPRECATED` minted), **P-D-49** arms 3 and 6 (one outcome
  vocabulary; the disposition matrix's `Applies to` column).
- [`../design/11-clone.md`](../design/11-clone.md) — the slice. Its §2 and §3.1 carry the **normative
  steps and the normative matrix**; this document declares their ids and carries the actor, the
  scenarios, the Input/Output and the boundary.
- Sibling slices: [`../design/01-foundation.md`](../design/01-foundation.md) (identity, the create
  and save doors, the bucket registry, the validation pipeline),
  [`../design/02-taxonomy-attributes.md`](../design/02-taxonomy-attributes.md) (attributes,
  categories, the metadata map, the PII hook),
  [`../design/03-sku-classification.md`](../design/03-sku-classification.md) (metering, `PlanTier`,
  accounting codes, `inst-mt-resolve`),
  [`../design/04-lifecycle.md`](../design/04-lifecycle.md) (deprecation, retirement, `replacedBy`).

**Requirements**: `cpt-cf-bss-products-fr-clone`

**Principles**: `cpt-cf-bss-products-principle-registered-validators`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`,
`cpt-cf-bss-products-constraint-tenant-isolation`,
`cpt-cf-bss-products-constraint-no-commercial-concern`

**Components**: `cpt-cf-bss-products-component-capability-handlers`

**Sequences**: **none** — DECOMPOSITION §2.11 states it: a clone lands as a draft and joins
`cpt-cf-bss-products-seq-authoring-publish` from there.

**The code surface this feature is written against, measured at `89d13fed5`.**

Every DoD in §5 names what exists. These are the measurements it rests on, so a reader can check them
rather than trust them.

- **The `CreateOnly` refusal already ships, written out, in both doors.**
  `api/rest/products.rs:3751` (in `route_product_field`) and `api/rest/skus.rs:3826` (in
  `route_sku_field`) each carry a complete `bucket::FieldClass::CreateOnly` arm returning
  `IllegalFieldMutation`. `products.rs`'s comment says why it is unreached: *"No column carries it
  until slice 11; the arm is built for the reason `correctable_after_publish` states for its own."*
- **`domain/bucket.rs:226`** declares the `FieldClass::CreateOnly` variant and documents it as *"`cloned_from`'s
  class, which no column carries until slice 11"*. Its module doc adds the debt in the same breath:
  *"A `cloned_from` write is still refused today, by the fail-closed miss rather than by the
  create-only rule — the same answer from a different rule, and slice 11 owes the row that makes it
  the right one."*
- **`cloned_from` is on neither table.** `infra/storage/entity/product.rs` ships fourteen columns
  and `sku.rs` thirteen; neither is `cloned_from`.
- **No `/clone` route ships.** The route census over `api/rest/` finds create, read, update, publish
  and discard for both kinds, and nothing matching `clone`.
- **The content stores three disposition rows read do not exist.** Seven migrations ship —
  `bss_schema`, `products_product`, `products_sku`, `products_audit_log`, `products_identity_ref`,
  `products_idempotency`, `products_entity_version`. `products_product_category`,
  `products_attribute_value` and any metadata-map table occur **zero** times under `infra/`.
- **`design/11` names seventeen error codes; five have a `DomainError` variant.** §4's per-field map
  carries **thirteen** of them, and §3.1 and §6 carry `PARENT_TERMINAL`, `RETIREMENT_PENDING`,
  `CONTENT_PII_BLOCKED` and `ENTITY_TERMINAL`. The five with a variant are `DUPLICATE_NAME`,
  `DUPLICATE_CODE`, `ILLEGAL_FIELD_MUTATION`, `PARENT_TERMINAL` and `ENTITY_TERMINAL`. Of the other
  twelve, ten are named nowhere in the crate; `RETIREMENT_PENDING` and `CONTENT_PII_BLOCKED` are
  named in `infra/error_mapping.rs` as **other slices'** — *"`PARENT_NOT_PUBLISHED` and
  `RETIREMENT_PENDING` are raised by slice `04-lifecycle`'s registered validators … and
  `CONTENT_PII_BLOCKED` is slice `02`'s content write-block. `DomainError` has no variant for any of
  the three"*. **`USAGE_TYPE_UNRESOLVED` and `USAGE_TYPE_UNAVAILABLE` are not in this population**:
  they occur zero times in `design/11` and are `design/03`'s, raised at publish.
- **The validation pipeline collects per violation and stops per phase.**
  `ValidationReport::violate(code, subject, detail)` takes a code **per violation**, `violations()`
  returns all of them, and `audit_code()` returns the first. `ValidationPipeline`'s doc: *"**The run
  stops at the first failing phase**, so a report is always one phase's findings and never a
  mixture."* `Phase::ordered()` is seven, in order: `Idempotency`, `Precondition`, `Shape`, `State`,
  `Identity`, `RegisteredValidators`, `GovernanceGate`.
- **The authz surface is closed at six.** `authz.rs:69`'s `labels::ALL` is exactly `[PRODUCT, SKU]`;
  `SUPPORTED_PROPERTIES` is `[OWNER_TENANT_ID, RESOURCE_ID]`; `gts/permissions.rs:99`'s
  `EXPECTED_PERMISSION_IDS` is exactly six under a **two-way** set-equality assertion plus a
  duplicate-registration check.

**One thing the crate says that no design document does, recorded for its owner and not repaired
here.** Two migration module docs attribute `cloned_from` to **slice 03** —
`migrations/m20260829_000002_create_products_product.rs:111-112` and
`m20260829_000003_create_products_sku.rs:101-102` — while **four** other files attribute it to slice 11
(`domain/bucket.rs`, `domain/bucket_tests.rs`, `api/rest/products.rs`, `api/rest/skus.rs`). Two more
name the column without naming a slice (`domain/error.rs`,
`infra/storage/migrations_tests.rs`), and `design/03-sku-classification.md` mentions `cloned_from`
zero times. §7 carries it.

## 2. Actor Flows (CDSL)

The flow below is **declared here and stepped in
[`../design/11-clone.md`](../design/11-clone.md) §2**, whose steps are the normative ones. What this
section carries is the triggering actor, the scenarios and the boundary.

### Clone an entity

- [ ] `p3` - **ID**: `cpt-cf-bss-products-flow-clone`

**Actor**: `cpt-cf-bss-products-actor-product-manager`

**Success Scenarios**:

- **The ordinary clone.** `POST /bss-products/v1/{products|skus}/{id}/clone` resolves the source
  in-tenant in any C1 state — `draft`, `published`, `deprecated` or `retired` — applies the
  disposition table field class by field class, and answers **201** with a new `draft`. The act is
  **one transaction per entity**: the create, its content rows and its metadata land together or not
  at all.
- **The read surface depends on the source's state.** A `retired`, `published` or `deprecated` source
  is read from its **last frozen version**, never from the head's pending edits, so in-flight
  unapproved content cannot leak into a clone; `clonedFrom` records exactly that version. A `draft`
  source is read from its head. The metadata map comes from the beside-entity store in both cases
  (**P-D-06**), which is what lets it survive retirement.
- **Identity is minted, never carried.** New `productId`/`skuId`; a suggested `{source}-copy-N` code,
  operator-overridable, reserved atomically. A source Product carrying no `productCode` suggests
  none and the clone's stays null.
- **Every same-brand Product clone renames.** The uniqueness index holds the source's name in every
  non-`discarded` state, so a clone of a draft, published, deprecated or retired Product collides
  alike. The suggestion is `{name}-copy-N`, flavoured `{name}-revived` for a retired source, and is
  operator-overridable. Display attributes copy verbatim as to their values — the quasi-code renames,
  the storefront does not.
- **Lineage is recorded and nothing new is emitted.** `clonedFrom = (entity id, published_version |
  'draft')` is written on the clone and is immutable thereafter. The clone rides the ordinary
  `ProductCreated`/`SkuCreated`; there is **no clone event**, because **P-D-21** makes the event
  stream the audit of record for what succeeds, so a committed act that emits one writes no audit row.
- **A product-with-SKUs clone is a batch of per-entity acts.** Children clone through the same table,
  each riding its own `SkuCreated`. **Parent-plus-surviving-children is a valid, intended end
  state**: drafts are cheap, and a child that fails re-validation is re-selectable and re-clonable.

**Error Scenarios**:

- **A failed re-validation refuses and collects.** The refusal names **every field class that
  failed**, with the live-registry verdict for each, so the operator re-selects rather than guesses.
  "Forces re-selection" is the operator's next act on that answer, **not a second wire outcome**
  (**P-D-49** arm 3) — which is the only reading under which `design/11` §5's single fixture yields
  three named failures.
- **A collision on an operator-supplied name or code** is the ordinary `DUPLICATE_NAME` /
  `DUPLICATE_CODE`, raised by the index under the write.
- **A lone-SKU clone whose copied parent is terminal or holds a live retire intent** is refused by
  the ordinary create door — `PARENT_TERMINAL` or `RETIREMENT_PENDING` — so cloning a retired
  parent's SKU alone requires naming a new parent.
- **A failing parent creates nothing** and its children are never attempted.

**Boundary, and what this flow deliberately does not claim.**

**It mints no parallel write path.** The clone materialises through the ordinary Foundation create
door — same validators, same codes, same guards. Every refusal above is a refusal that door already
makes or will make; this feature contributes the disposition and the re-validation, not a second
create.

**It does not carry commercial content.** Pricing and plan content is not copied and is not
represented here at all, per `cpt-cf-bss-products-constraint-no-commercial-concern`.

**It does not govern.** A clone lands as `draft`; its publish is the ordinary `05-governance`-gated
act, and no approval is required to clone.

**Bulk cloning is out**, and is out with a measured gap rather than a delegation:
`design/09-bulk-promotion`'s resolver is total over identity and produces no copies. Its C5 is
explicit — *"**Four classifications, exhaustive**"* — and none of the four is a copy: an unknown
identity creates, a matching one is a no-op, a differing one updates as a draft, and an incompatible
or `retired` holder conflicts, naming clone as the only path. So mass cloning is claimed by nobody.
§7 row 10 carries it.

## 3. Processes / Business Logic (CDSL)

The process below is **declared here and stepped in
[`../design/11-clone.md`](../design/11-clone.md) §3**.

### The disposition table

- [ ] `p3` - **ID**: `cpt-cf-bss-products-algo-disposition`

**Input**: a resolved source entity — its frozen version content for a non-`draft` source, its head
for a `draft` one — plus the beside-entity metadata map, the cloning actor's ref, and whatever the
operator supplied to override a suggested code, name or parent.

**Output**: either the field set for one `INSERT` through the create door — each class copied, reset,
re-validated or refused — or a **collected refusal** naming every field class that failed with its
live-registry verdict.

**Boundary**: the matrix is **normative in `design/11` §3.1** and is not restated here. It is
declared per **field class × entity kind** (**P-D-49** arm 6 added the `Applies to` column, because
one table served both kinds while the rename rule it delegates to is Product-only and
`products_sku` carries no name column at all). What this document owns is the obligation that every
row have a writer, a reader and a phase — which §5 discharges.

**The one property of this process that is a code constraint, not a design preference.**

`design/11` C4 requires that the refusal *"names every field class that failed"* and **P-D-49** arm 3
fixes that as the single outcome vocabulary. The shipped pipeline admits that **within one phase and
only there**, and the constraint is worth stating precisely because both halves are already decided:

- **Collection is representable.** `ValidationReport::violate` takes a code **per violation** and
  `violations()` returns them all, so **one report** carrying `UNIT_DEPRECATED`,
  `PLAN_TIER_DEPRECATED` and `CATEGORY_RETIRED` at once is legal. `infra::error_mapping` renders a
  `DomainError::Validation(report)` carrying each violation's own code, so all three reach the
  caller.
- **Collection does not cross a phase boundary — where a pipeline runs at all.** The pipeline stops
  at the first failing phase, and **P-D-37** states the reason structurally: `design/01` §4.4's audit
  row carries a single `error_code`.
- **The create path runs no pipeline today.** The only two `ValidationPipeline` constructions in the
  crate are `publish_revalidation_pipeline` in each door — **publish**, not create. `create_product`
  and `create_sku` build a `ValidationReport` inline, with `report.violate("VALIDATION", …)`
  literals. So the phase machinery binds the clone only once the create door gains a pipeline, which
  no document obliges.

So the obligation this document takes is **one report, not one phase**: every disposition
re-validation must land in a single `ValidationReport`, and — if and when the create door becomes
phased — in a single phase, because a set split across phases can never produce the collected
refusal C4 and **P-D-49** arm 3 require.

**Which phase, if any, is not settled here.** `design/11` contains the token `Phase` zero times.
`Phase::Shape` covers *"the resolvability of every reference the payload carries"*, but the flagship
fixture's three failures all **resolve** and fail on the referent's lifecycle; and `Phase::State` is
scoped to *"the parent's terminal state and the subject's own … the row as it now stands"*, which a
foreign vocabulary referent is not. **No phase's stated scope covers a live-lifecycle check on
another registry's row.** §7 row 11 carries it.

**What the rule does not settle, and is therefore §7's.** `DUPLICATE_NAME` and `DUPLICATE_CODE` are
`Identity`-phase by the enum's own text — *"Uniqueness, reservation, containment"* — and are decided
by the index under the write, which **P-D-37** measured as unable to collect a second code at all. A
clone that fails a vocabulary re-validation **and** collides on an operator-supplied name therefore
reports only the earlier phase, and the operator learns of the collision on the retry. Whether C4's
*"every field class"* is scoped to the re-validated vocabulary classes or to the whole act is an
owner's call, not this document's.

## 4. States (CDSL)

**No state machine is declared by this feature.**

`design/11` declares no `state-` id and no transition. A clone is a **create**: C3 resets the
lifecycle to `draft` with `published_version = 0` and `internal_revision = 1`, and the source is
never affected, so nothing moves. The lifecycle the clone's product then enters is
`04-lifecycle`'s, unchanged and unextended by this feature.

Because §4 declares nothing, **this feature mints no `inst-` id at all** — as on
`features/catalog-version.md`, `features/reference-signal.md` and `features/retention-erasure.md`.
The six instruction ids the act runs on (`inst-cn-door`, `inst-cn-identity`, `inst-cn-rename`,
`inst-cn-disposition`, `inst-cn-lineage`, `inst-cn-children`) are **`design/11` §2's** and are
stepped there.

## 5. Definitions of Done

Every DoD below names types, functions, tables and tests **that exist at `89d13fed5`** wherever one
exists. Where the shipped seam already carries an obligation, the DoD says so rather than restating
it as new work — which on this feature is unusually often, the refusal machinery being built ahead of
the column that reaches it.

### `cloned_from` lands on both entity tables, in the migrations that already name it

- [x] `p3` - **ID**: `cpt-cf-bss-products-dod-cloned-from-column`

Both entity tables gain `cloned_from`, **nullable**, holding the immediate source's identity and the
version it was read from. Neither table has it today.

**The landing site is fixed by the product migration, and extended here to the SKU one.**
`m20260829_000002_create_products_product.rs` states it — *"When those columns land, their clauses
join this same file's whitelist rather than a follow-up migration"* — and
`m20260829_000003_create_products_sku.rs` carries **no landing-site sentence at all**. This DoD
applies the same rule to both: the column and its guard clause are edited into those two files, and
**no tightening migration is added**. That the SKU migration does not say so itself is recorded in
§7's owed subsection.

The guard clause is the create-only one: `cloned_from` is admitted by the `INSERT` and by **no
`UPDATE` at all** — not merely by none after first publish. `design/01` §4.1 words it *"**stricter
than bucket-i** — writable only in the creating statement and never again, not merely never after
first publish, so the lineage stays evidence rather than a claim"*, and both triggers already refuse
`tenant_id`, the primary key and `created_by` in the same shape (**P-D-34**, which names those
three; `created_at` sits with them by the migrations' own extension, *"which the `DoD` does not
name"*), so the
clause joins that group rather than inventing a form.

**Implements**: `cpt-cf-bss-products-algo-disposition`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_product`, `products_sku`
- Migrations: `m20260829_000002_create_products_product`, `m20260829_000003_create_products_sku`

### The create-only registry row, over a refusal that already ships

- [x] `p3` - **ID**: `cpt-cf-bss-products-dod-create-only-class`

`cloned_from` is registered in the bucket registry as `FieldClass::CreateOnly` for **both** entity
kinds. **This DoD builds no refusal.** The arms exist: `route_product_field`
(`api/rest/products.rs:3751`) and `route_sku_field` (`api/rest/skus.rs:3826`) each return
`IllegalFieldMutation` with the message already written, and `unroutable_product_field` /
`unroutable_sku_field` already route the class per their own docs — *"`cloned_from`'s `CreateOnly`
when slice 11 lands it"*.

What changes is only that the arms become **reachable**. `domain/bucket.rs` stated the debt this
discharges — a `cloned_from` write refused *"by the fail-closed miss rather than by the
create-only rule — the same answer from a different rule"* — and **the debt is paid**: P-D-76's
pair carries the registry tag on both kinds, so the classifier answers the class and the door's
refusal follows from it, with a door-level case asserting the pair for both columns.

**A shipped green test goes red here, by design, and the DoD is not met until it is restated.**
`domain/bucket_tests.rs`'s `no_column_carries_the_create_only_class_today` asserts both
`count_of(kind, FieldClass::CreateOnly) == 0` and that `classify(kind, "cloned_from")` is an error.
Its own doc anticipates this feature: *"Derived from the registry, not from a literal list, so slice
11 registering the column makes this case red and forces the count to be restated deliberately."*
The test is rewritten to assert the count is **one** per kind and that `classify` now returns
`FieldClass::CreateOnly` — **not** deleted, and not weakened to a range.

**Implements**: `cpt-cf-bss-products-algo-disposition`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**A second census reddens with it, and it is hard-coded.**
`domain::bucket_tests::the_class_counts_are_pinned_per_entity` asserts
`columns(Product).len() == 14` and `columns(Sku).len() == 13` over a per-class table, and its own doc
calls those *"the only numbers that have to be restated when a slice adds a column"*. Landing
`cloned_from` moves both to 15 and 14 and adds a `CreateOnly` row per kind. Its sibling
`the_registry_and_the_physical_tables_name_the_same_columns` reads the physical side from
`product::Column::iter()`, so the **SeaORM models** must gain the field too or the registry names a
column no table has.

**Touches**:
- Modules: `domain::bucket`, `api::rest::products`, `api::rest::skus`
- Entities: `infra::storage::entity::product`, `infra::storage::entity::sku`
- Tests: `domain::bucket_tests::no_column_carries_the_create_only_class_today`,
  `domain::bucket_tests::the_class_counts_are_pinned_per_entity`,
  `domain::bucket_tests::the_registry_and_the_physical_tables_name_the_same_columns`

### The clone door

- [x] `p3` - **ID**: `cpt-cf-bss-products-dod-clone-door`

`POST /bss-products/v1/{products|skus}/{id}/clone` exists for both kinds, answering **201** with the
new draft. For a Product the act is always the family act — the body carries no selector and no
document names a lone-product clone (**P-D-79**) — the per-child receipt riding the 201, empty
for a childless source.

The door **MUST** drive the ordinary Foundation create door rather than a parallel insert path —
`design/11` §1.7 words it *"internally it drives the ordinary 01 create door"* — so the validators,
the codes, the reservation and the outbox row are the create door's, once.

The act is **one transaction per entity**: the entity row, its content rows and its metadata map land
together. For a product-with-SKUs clone that is one transaction **per child**, not one across the
batch, which is how it squares with `01-foundation`'s no-partial-application rule: each act is
itself complete.

**Authorization precedes everything, including idempotency.** `domain/validation.rs` states it:
*"Authorization is **not** a phase. It is a pre-pipeline gate, which is the only order in which a
denied caller neither consumes an idempotency key nor writes a claim row"* (**P-D-30**).

**Implements**: `cpt-cf-bss-products-flow-clone`

**Constraints**: `cpt-cf-bss-products-constraint-tenant-isolation`

**Touches**:
- API: `POST /bss-products/v1/products/{id}/clone`, `POST /bss-products/v1/skus/{id}/clone`
- Modules: `api::rest::products`, `api::rest::skus`
- Entities: `CloneDoor`

### The source read surface, and the leak it closes

- [ ] `p3` - **ID**: `cpt-cf-bss-products-dod-clone-read-surface`

A `retired`, `published` or `deprecated` source is read from its **last frozen version** in
`products_entity_version` — which ships — and **never** from the head row. A `draft` source is read
from its head. The metadata map is read from the beside-entity store in both cases (**P-D-06**),
because it sits outside frozen content and therefore survives retirement.

**The rule exists for a specific leak.** A published entity's head carries pending, unapproved
edits; reading it would copy them into a new draft and thereby publish them by a side door. The
frozen read is what makes a clone reproduce what consumers actually saw. A `deprecated`
source whose head has moved since deprecation reads the **newer** frozen version (**P-D-78** —
§7 row 15): the retirement design keeps the head open and re-announces consumers onto the latest
frozen bytes, and no store records which version was current at deprecation.

`clonedFrom` records **exactly the version read** — `(entity id, published_version)` for a frozen
read and `(entity id, 'draft')` for a head read — so the lineage names a byte-identical source
rather than an entity whose head has since moved.

**The frozen column set is not the whole row.** `01-foundation` §4.3 excludes `lifecycle_state`,
`deprecation_provenance`, `replaced_by_sku_id` and `internal_revision` from frozen content
(**P-D-24**, extended by **P-D-35**) — all four of which the disposition table resets anyway, so the
exclusion and the reset agree. A DoD that read the frozen row expecting them would find them absent.

**And the frozen row is not a column set at all.** `products_entity_version` ships `content` as a
single `String` holding *"the canonical rendering itself, exactly the bytes `content_digest` was
computed over, **rather than one column per content field**"*. Two consequences this DoD owns:

- **There is no reader.** `repo.rs` has `insert_entity_version` and no read function for the table.
- **There is no parser.** `domain::canonical` exports `content_digest`, `canonical_rendering` and
  `render_instant`, and `repo.rs` states it *"deliberately imports no canonicalizer"*.

So this DoD adds a **tombstone-free read of `products_entity_version` by
`(tenant, entity_kind, entity_id, published_version)` and a decoder for the canonical rendering** —
the decoder being the half nothing in the crate supplies. It **MUST** be the inverse of
`canonical_rendering` and live beside it, not a second serialization rule invented at the clone door;
`domain/canonical.rs` exists to keep that in one place. The decoder is `01-foundation`'s, beside
the renderer (**P-D-77** — §7 row 23), and this read surface is its first consumer.

**Implements**: `cpt-cf-bss-products-algo-disposition`

**Touches**:
- DB Table: `products_entity_version`
- Modules: `infra::storage::repo`

### Identity: minted ids, a suggested code, and one reservation

- [x] `p3` - **ID**: `cpt-cf-bss-products-dod-clone-identity`

New `productId`/`skuId` are minted. The code is **suggested** as `{source}-copy-N`,
**operator-overridable**, and reserved atomically through the same index the create door uses —
**`N` is the first free integer for the suggested string, decided by the index under the
reservation** (**P-D-62**): a reservation conflict moves to the next free integer and retries, so
concurrent clones of one source are arbitrated by the index rather than racing a read, and no counter
column exists to drift. A collision on an **operator-supplied** code is the ordinary
`DUPLICATE_CODE`, which **P-D-25** made one code covering both the
`skuCode` and `productCode` reservations.

**A source Product with no `productCode` suggests none**, and the clone's stays null. `product_code`
is nullable on `products_product` and its uniqueness is a partial index, so a null clone of a null
source is legal and must not be back-filled with a derived value.

**`name_normalized` is derived, not copied.** It ships as a column on `products_product` and
`domain/name.rs::normalize` is its only producer; the clone derives it from the **renamed** name,
application-side.

**What this DoD does not do.** It does not pin `N`. The suffix's generation rule — per-source
counter, global counter, or first free integer — is undefined, `-revived` carries no counter at all,
and concurrent clones computing `N` by a read race each other. That is **§7 row 4's, owned by this
feature**. A separate question — whether `{source}-copy-N` is even a legal code — is the PRD owner's
at `PRD.md` §15: §12 AC #1 requires the code be *"short, fixed-format"* while
`fr-identifier-contract` words it *"fixed-format"* without the first adjective, and no document pins
the format either way. **The DoD is met by a suggestion mechanism whose
rule is supplied**, not by inventing one here.

**Implements**: `cpt-cf-bss-products-flow-clone`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_product`, `products_sku`
- Modules: `domain::name`

### The rename rule, and why it is not revival-only

- [x] `p3` - **ID**: `cpt-cf-bss-products-dod-rename-rule`

**Every same-brand Product clone renames.** The suggestion is `{name}-copy-N` — `N` the first free
integer under the reservation (**P-D-62**) — flavoured `{name}-revived` for a `retired` source, **and
a second revival of one lineage suggests `{name}-revived-N`** by the same first-free rule, so the
flavor survives and the suggestion path never produces a refusal. It is operator-overridable; a
collision on an operator-supplied name is the ordinary `DUPLICATE_NAME`.

**The premise is measurable and must be tested as such.** **P-D-04** makes name uniqueness absolute
on `(tenantId, brandId, normalized(name))` and region-independent, and the index holds the source's
name in **every non-`discarded` state**. So a clone of a `draft`, `published`, `deprecated` or
`retired` Product collides alike — revival is *why* the rule is non-negotiable, not *when* it
applies. A test that only exercises the retired source proves the weaker rule.

**The rule is Product-only.** `products_sku` carries no name column at all; `design/11` §3.1's
`Applies to` column says so, and **P-D-49** arm 6 added that column precisely because the undivided
table made this row unbuildable for half its subjects.

**Display attributes are not renamed.** Their values copy verbatim — still re-validated per the
matrix — because the canonical name is a quasi-code and the storefront name is a localized display
attribute that repeats freely. That is **P-D-04**'s own rationale, and inverting it would rename
what customers see.

**The brand premise is load-bearing.** `brand_id` **copies**; a clone never retargets brand, and a
cross-brand copy is a create rather than a clone. If that ever changed, the collision premise —
same brand, same name — would no longer hold and the rename rule would lose its reason.

**Implements**: `cpt-cf-bss-products-flow-clone`

**Touches**:
- DB Table: `products_product`
- Modules: `domain::name`

### The disposition matrix, registered as rules over one phase

- [ ] `p3` - **ID**: `cpt-cf-bss-products-dod-disposition-rules`

Every row of `design/11` §3.1 is implemented, per entity kind, as the matrix's `Applies to` column
states. The **copy** and **reset** rows are field mapping in the clone's assembly step; the
**re-validate** rows are registered `ValidationRule` implementations.

**Every re-validating rule contributes to one `ValidationReport`**, so that a single refusal carries
all their codes. That is the DoD's substance, and §3's measurement is its reason. Where the door is
phased, one report means **one phase** — the pipeline stops at the first failing phase — but the
create path the clone delegates to runs no pipeline today, so the report is the obligation and the
phase is its consequence, not a separate requirement.

**The DoD does not choose the phase**, and neither does any document: `design/11` never names one,
and no phase's stated scope covers a live-lifecycle check on another registry's row. §7 row 11 is
where that is routed, together with §7 row 25's question of which pipeline would host it at all.

**They register in `design/11` §3.1's row order** (**P-D-55**), which is therefore their execution
order within the phase. The table is the normative list, so the order it already carries becomes the
order rather than being settled by whoever registers first; it ranks rows, so the two rows whose code
is unminted take their place when it is minted.

**The rules append and never short-circuit.** That is `ValidationRule`'s own contract — *"It never
short-circuits the run, never mutates the subject, and **never reads another rule's verdict**"* — and
it is what makes the collected refusal fall out of the registration rather than needing a
collector of its own.

**Not one of the five `Copy + re-validate` rows has an operand at this commit**, and the DoD says so
rather than specifying work over tables and columns that do not exist. `design/11` §3.1's
re-validating rows are *Display/localized attributes + metadata map*, *Category assignments*,
*`PlanTier`*, *Metering declaration* and *Accounting codes*. Measured:

- `products_product_category`, `products_attribute_value` and any metadata-map table occur **zero**
  times under `infra/`;
- `plan_tier`, the accounting-code refs, the metering unit and `sellable` are on **no** shipped
  table — `products_sku`'s thirteen columns carry none of them, and `infra/storage/entity/sku.rs`
  says so itself: the capability columns *"arrive with those features"*.

So the re-validating set is registered over **nothing** today, and this DoD is discharged when
`02-taxonomy-attributes` and `03-sku-classification` land their stores and columns. What is
buildable now is the copy/reset half and the identity and parent rows. §7 carries the writer
question, which is `01-foundation`'s and `02`'s jointly.

**`usageTypeRef` is not re-validated here.** Its re-resolution stays `03`'s `inst-mt-resolve` and
runs **at publish**, not at clone — so a clone carrying an unresolvable `usageTypeRef` is admitted
and fails later, deliberately.

**Implements**: `cpt-cf-bss-products-algo-disposition`

**Constraints**: `cpt-cf-bss-products-constraint-no-commercial-concern`

**Touches**:
- Modules: `domain::validation`, `domain::rules`
- Entities: `DispositionTable`

### The error vocabulary, twelve codes of which none has a variant

- [ ] `p3` - **ID**: `cpt-cf-bss-products-dod-revalidation-codes`

The refusal names, per failing field class, the code its owning slice declares. The roster is
`design/11`'s and is normative there: **§4's per-field map carries thirteen codes**, and §3.1 and §6
carry four more — `PARENT_TERMINAL`, `RETIREMENT_PENDING`, `CONTENT_PII_BLOCKED` and
`ENTITY_TERMINAL` — for **seventeen** in all.

**Measured at `89d13fed5`, five of the seventeen have a `DomainError` variant** —
`DUPLICATE_NAME`, `DUPLICATE_CODE`, `ILLEGAL_FIELD_MUTATION`, `PARENT_TERMINAL`, `ENTITY_TERMINAL` —
and the other **twelve** have none:

`UNRECOGNIZED_UNIT`, `UNIT_DEPRECATED`, `PLAN_TIER_UNKNOWN`, `PLAN_TIER_DEPRECATED`,
`CATEGORY_RETIRED`, `ACCOUNTING_CODE_UNKNOWN`, `ACCOUNTING_CODE_DEPRECATED`,
`ATTRIBUTE_DEFINITION_UNKNOWN`, `ATTRIBUTE_DEFINITION_DEPRECATED`, `ATTRIBUTE_SCOPE_VIOLATION`,
`RETIREMENT_PENDING`, `CONTENT_PII_BLOCKED`.

Ten of those twelve are named nowhere in the crate. The remaining two are named — in
`infra/error_mapping.rs`, as **other slices'** codes with their statuses already assigned by
`design/01` §3.3's ladder, and explicitly without a variant: *"`DomainError` has no variant for any
of the three — mapping a code this gear cannot raise would be a dead `match` arm"*.

**`USAGE_TYPE_UNRESOLVED` and `USAGE_TYPE_UNAVAILABLE` are not in this roster.** They occur zero
times in `design/11`; they are `design/03`'s, raised at publish, and the clone does not re-validate
`usageTypeRef` at all.

**This feature declares none of them.** Each belongs to the slice that owns its vocabulary — `02`
for the attribute and PII codes, `03` for unit, tier and accounting, `04` for `RETIREMENT_PENDING` —
and declaring a second home for any of them would break the one-declaration rule. **The DoD is that
the clone raises them, not that it mints them**, and it is blocked on their owners until then.

**The five that ship are reused unchanged**, being the create door's own identity and state codes.
No clone-specific code is minted at all: `design/11` §4 says *"Errors reuse the owning slices'
codes"*, and this document adds nothing to that.

**Implements**: `cpt-cf-bss-products-algo-disposition`

**Touches**:
- Modules: `domain::error`, `domain::validation`

### Lineage, and the event that is deliberately absent

- [ ] `p3` - **ID**: `cpt-cf-bss-products-dod-clone-lineage`

`cloned_from` is written in the creating statement to the **immediate** source — never copied from
the source's own `cloned_from`, so a clone of a clone points one step back and the chain stays
walkable. **In a product-with-SKUs clone a child's `cloned_from` names its own source SKU** (**P-D-72**
— uniform, same-kind, never the parent act; the family reconstructs from `parent_id` plus the
children's own pointers, and that walkability is what the resume re-entry reads). The create-only guard makes a copied value unrepairable, which is why the reset is a rule
and not a convention.

**No clone event is emitted.** The clone rides `ProductCreated` / `SkuCreated`, and each child of a
product-with-SKUs clone rides its own `SkuCreated`. **P-D-21** is the reason and is not a preference:
the event stream is the audit of record for what succeeds, so a committed act that emits an event
writes **no audit row**, and adding a clone event would either duplicate the create or split the
record.

**The justification for the absence carries a debt this DoD does not discharge.** `design/11`
§2 justifies having no clone event by the lineage field being *"queryable"* — and the field appears
in no read model and no SDK shape, while a clone is a `draft`, which `08-read-models`' browse
projection cannot see at all. So the reverse lookup — *what was cloned from this entity* — has no
surface. §7 routes it to `08`'s and `12`'s owners: expose it, or withdraw the justification.

**Implements**: `cpt-cf-bss-products-flow-clone`

**Touches**:
- DB Table: `products_product`, `products_sku`
- Modules: `infra::events`

### The product-with-SKUs clone, and its honestly-reported partial

- [x] `p3` - **ID**: `cpt-cf-bss-products-dod-clone-children`

A product-with-SKUs clone clones each child — the source's non-discarded SKUs, in any of C1's
four states (**P-D-79**: a `discarded` child is neither attempted nor receipted) — through the
same disposition table, **remapping the parent link to the new parent** rather than copying the
source's.

- A **failing parent** creates nothing and children are never attempted.
- A **failing child fails alone**; siblings land.
- **Parent-plus-surviving-children is a valid, intended end state.** Drafts are cheap and a failed
  child is re-selectable and re-clonable.

**Each act is complete, which is how this squares with no-partial-application.** `01-foundation`
forbids a partially applied act; here the unit is the entity, not the batch, so per-row atomic acts
honestly reported is the PRD's own §6.9 shape rather than a carve-out.

**The lone-SKU carve-out is the one the matrix discloses.** A lone-SKU clone **copies** the parent
link, and the create door then requires a parent that is neither `retired` nor `discarded` and holds
no live retire intent — `PARENT_TERMINAL` / `RETIREMENT_PENDING`. So a lone clone of a retired
parent's SKU must name a new parent, even though C1 admits a retired *source*. The two rules are
about different entities and both hold.

**The durability question is answered without a table** (**P-D-72**): the durable ledger is the data
itself — the new parent's children carry their own `cloned_from` pointers — and the **same-key retry
resumes the family act**, the door's claim joining the *parent's* transaction, a
committed-but-unanswered claim meaning *in progress*, the re-entry skipping already-cloned sources
and storing the answer at completion — the parent found by the claim row's own `entity_ref`
stamp, since several family acts over one source make `cloned_from` alone ambiguous (**P-D-79**). **The family act answers `201` with a per-child receipt** —
`{source sku_id, disposition, new sku_id | code + violations}`, codes the owning doors' verbatim — a
failing parent staying the ordinary refusal of the whole act.

**Implements**: `cpt-cf-bss-products-flow-clone`

**Touches**:
- DB Table: `products_product`, `products_sku`

### The authz surface, and the roster it would redden

- [ ] `p3` - **ID**: `cpt-cf-bss-products-dod-clone-authz`

The clone door spends `product × write` for a Product and `sku × write` for a SKU. A
product-with-SKUs clone requires **both** — and every product clone is the family act
(**P-D-79**), so the product door spends both unconditionally: authorization precedes the child
count it has not yet been authorized to read (P-D-30).

**Both permissions already ship.** `gts/permissions.rs`'s `EXPECTED_PERMISSION_IDS` is exactly
six — `{product,sku} × {read,write,publish}` — and the door passes a `ResourceType` from
`authz::resource_types`, never a bare label, as every authoring door does.

**A second grant is declared in the design and ships nowhere, and this DoD does not decide it.**
`design/05-governance` declares `metadata × write` against `02`'s map door. In the crate,
`labels::ALL` is exactly `[PRODUCT, SKU]`, `SUPPORTED_PROPERTIES` is
`[OWNER_TENANT_ID, RESOURCE_ID]`, and the permission roster is closed at six under a **two-way**
set-equality assertion plus a duplicate-registration check. So whichever way §7's question is
answered, the cost is measurable:

- **If the clone needs it**, the pair becomes a **seventh** permission id and
  `EXPECTED_PERMISSION_IDS`' set-equality test is the census site that must change with it.
- **If it does not**, `05`'s declared pair still has no code, and that is `05`'s and `02`'s to
  settle rather than a gap this door introduces.

**The DoD is met by the two write grants the design assigns**, with the metadata question registered
and not pre-empted.

**Implements**: `cpt-cf-bss-products-flow-clone`

**Constraints**: `cpt-cf-bss-products-constraint-tenant-isolation`

**Touches**:
- Modules: `authz`, `gts::permissions`

### The clone is audited as a refusal and as nothing else

- [x] `p3` - **ID**: `cpt-cf-bss-products-dod-clone-audit`

A **committed** clone writes no audit row — its `ProductCreated`/`SkuCreated` is the record
(**P-D-21**). A **refused** clone writes one, carrying a single `error_code`.

**The code the row actually stores today is `VALIDATION`, not the first violation's.** Measured:
`ValidationReport::audit_code` — which returns `self.violations.first().map(|v| v.code)` — has
**zero production callers**; its only references are in `domain/rules_tests.rs` and
`domain/validation_tests.rs`. Every door writes `error_code: domain_err.code()` instead, and
`DomainError::code()` maps `Self::Validation(_)` to `"VALIDATION"`. So a clone refused for three
field classes returns three violations to the caller and stores the single code `VALIDATION`.

**That is P-D-37's split, and it is also a gap.** *"The caller's rejection carries every violation the
failing phase collected; the audit row records one code"* holds; what does not hold is the assumption
that the recorded code discriminates. `AC #38`'s map reads the **stored** code, and a stored
`VALIDATION` tells it nothing about which field class failed.

**Answered (P-D-55): this door stores `VALIDATION` like every shipped door, and does not route
`audit_code()` on its own.** So the obligation is determinate — one audit row carrying `VALIDATION` —
and the discrimination gap is **`design/01-foundation.md` §6 item 2**'s, owned by that slice with the
error-contract owner, which asks the same question in its general form. The registration order that
would become observable if the code were ever routed is fixed by `design/11` §3.1's row order, also
P-D-55. **Two things make the order unobservable today, not one**: `audit_code()` has no production
caller, *and* every registered rule raises the literal `"VALIDATION"` (`domain/rules.rs:73`), so even
a routed call would not discriminate.

**Implements**: `cpt-cf-bss-products-algo-disposition`

**Touches**:
- DB Table: `products_audit_log`
- Modules: `domain::validation`

### `design/11` §1.7's two design-introduced names exist as named seams

- [x] `p3` - **ID**: `cpt-cf-bss-products-dod-clone-seams`

`design/11` §1.7 introduces exactly two names, and both are addressable in the implementation rather
than being prose:

- **`DispositionTable`** — the normative per-field copy/reset/re-validate matrix, whose
  implementation is the registered rule set of `dod-disposition-rules`.
- **`CloneDoor`** — the one endpoint, which *"internally drives the ordinary 01 create door"*.

**Neither is an aggregate.** DECOMPOSITION §2.11 lists both under **Domain Model Entities** — the
convention §2.7 and §2.10 follow for their own §1.7 names — and says in the same entry: *"Neither is
an aggregate of its own: a clone produces a `Product` or `SKU` row owned by `01-foundation`,
distinguished only by its `cloned_from` column"*. So this DoD obliges a named module seam, **not** a
new aggregate, and a reviewer finding no `DispositionTable` struct has found conformance rather than
a gap if the rule set carries the name. Whether that listing field is a naming convention or an
ontological claim is §7 row 24's.

**Implements**: `cpt-cf-bss-products-algo-disposition`

**Touches**:
- Entities: `DispositionTable`, `CloneDoor`

### The test posture, with a positive control per code

- [ ] `p3` - **ID**: `cpt-cf-bss-products-dod-clone-tests`

`design/11` §5's posture is implemented in full, and the criteria are §6's. Two obligations that are
this DoD's rather than §6's:

**A positive control per code the clone raises.** The clone **raises** seventeen codes and
**declares none** — twelve of them belong to `02`, `03` and `04`, five to `01-foundation`. Each gets
a paired case — the failure and a clean-source control proving the fixture could reach the admitting
branch — so no arm passes because the fixture could never have succeeded.

**That is sixteen paired cases today, not seventeen.** `ENTITY_TERMINAL`'s only candidate trigger at
this door is a `discarded` source, which §7 row 5 records as undecided, so its pair is blocked on
that answer rather than absent by oversight. The count is stated so a partial suite is visible as
partial.

**The collected-refusal fixture is the flagship and is asserted as a set.** One source carrying a
deprecated unit, a retired tier and a retired category yields **three named failures in one
response**, asserted as a set of three codes rather than as "at least one" or as the first — an
assertion on the first code passes on a build that short-circuits, which is the exact behaviour
**P-D-49** arm 3 struck.

**Implements**: `cpt-cf-bss-products-flow-clone`, `cpt-cf-bss-products-algo-disposition`

**Touches**:
- Tests: the clone door suite

## 6. Acceptance Criteria

**The revival flagship**

- [ ] Clone a `retired` Product: new ids, a **forced canonical rename**, an **identical display
      name**, the source **untouched**, and `clonedFrom` recording the version read. All five
      asserted in one probe — the source-untouched half alone passes on a build that cloned nothing.
- [ ] The cloned entity is `draft` with `published_version = 0` and `internal_revision = 1`.

**Rename is not revival-only**

- [ ] Clone a `published` Product, a `draft` Product and a `deprecated` Product: **each renames**.
      Three cases, because the rule's premise is the index over every non-`discarded` state and a
      retired-only test proves the weaker rule.
- [ ] A SKU clone does **not** rename, `products_sku` carrying no name column.
- [ ] An operator-supplied name that collides is `DUPLICATE_NAME`; an operator-supplied code that
      collides is `DUPLICATE_CODE`.

**The re-validation matrix**

- [ ] The flagship fixture — a source with a deprecated unit, a retired tier and a retired category
      — yields **three named failures in one response**, asserted as a set, **none silently copied**.
- [ ] Each of the seventeen codes `design/11` names — thirteen in §4's per-field map, four in §3.1
      and §6 — has a failing case **and** its clean-source positive control, **except**
      `ENTITY_TERMINAL`, whose trigger §7 row 5 leaves undecided. Sixteen pairs.
- [ ] No case exists for `USAGE_TYPE_UNRESOLVED` or `USAGE_TYPE_UNAVAILABLE`: they are `design/03`'s,
      raised at publish, and the criterion below asserts the clone admits an unresolvable ref.
- [ ] A PII value allow-listed when the source was created, since de-listed, **blocks on clone** —
      the policy of today governs, not the policy the source was written under.
- [ ] `usageTypeRef` is **not** re-validated at clone: a source carrying an unresolvable one clones
      successfully and fails at publish.

**The read surface**

- [ ] A `published` source with pending head edits clones its **frozen** content, and the probe
      asserts the pending edit is **absent** from the clone.
- [ ] A `draft` source clones its head.
- [ ] The metadata map is carried for a `retired` source, whose frozen content does not hold it.

**Lineage**

- [ ] `clonedFrom` names the immediate source and its version; a clone of a clone points one step
      back, **not** to the original.
- [ ] An `UPDATE` naming `cloned_from` is refused `ILLEGAL_FIELD_MUTATION` on both kinds, and the
      refusal comes from the **create-only** rule rather than the fail-closed registry miss — the
      distinction `domain/bucket.rs` names as owed to this feature.
- [ ] The clone emits `ProductCreated`/`SkuCreated` and **no clone event**, and a committed clone
      writes **no audit row**.
- [ ] A clone refused for three field classes returns **three** violations to the caller and writes
      **exactly one** audit row. The stored `error_code` is asserted against whichever answer §7
      row 13 receives — `VALIDATION` on today's shipped path — and the assertion names the answer it
      encodes, so a later change of that answer fails here rather than silently.

**Product-with-SKUs**

- [ ] One failing child: the parent and the surviving siblings land, and the response reports the
      failed child.
- [ ] A failing parent creates **nothing**, and no child was attempted.
- [ ] Children's parent links point at the **new** parent.

**The lone-SKU carve-out**

- [ ] A lone clone of a SKU whose parent is `retired` is refused `PARENT_TERMINAL`; naming a new,
      non-terminal parent admits it.
- [ ] A lone clone under a parent holding a live retire intent is refused `RETIREMENT_PENDING`.

**The registry row**

- [ ] `classify` returns `FieldClass::CreateOnly` for `cloned_from` on both kinds, and
      `count_of(kind, FieldClass::CreateOnly)` is **one** per kind — the restatement
      `no_column_carries_the_create_only_class_today` demands when this feature lands.

**Authorization**

- [ ] The clone door refuses without `product × write` (Product) or `sku × write` (SKU), and a
      product-with-SKUs clone refuses without **both**.
- [ ] The refusal happens **before** an idempotency key is consumed or a claim row written.

## 7. Known unknowns

**The arithmetic of this section.** Twenty-seven rows: **ten carried verbatim** from
[`../design/11-clone.md`](../design/11-clone.md) §6 — the slice's full count, not a selection — and
**seventeen raised here**: twelve while authoring and five by the three-lens review of this
document. Eight of the seventeen (rows 11, 12, 13, 14, 17, 20, 23 and 25) come from reading the
crate and nine from the design set. Of the twenty-seven, **seventeen block
no DoD in this document** (rows 9, 10, 21 and 24, plus rows 13 and 4, resolved by **P-D-55 and
P-D-62 on 2026-08-31**, and rows 7, 19, 26 — **P-D-72** — 1, 2, 5, 12, 14 — **P-D-75** — 18 — **P-D-76** —
15 — **P-D-78** — and 23 — **P-D-77** —
on 2026-09-01, all kept in place rather than struck); the other ten each name the DoD they block. A
third subsection carries defects owed to other documents, recorded and not repaired here; those are
not rows. The two register pointers are in this preamble, not there.

**Carried, not answered**, and registered against **its owner's** register. **Two departures from
verbatim, declared so the claim is checkable.** First, the slice's inline `Owner:` sentence and its
provenance marker are converted into this document's `**Owner**:` field. Second, every bare `§N`
inside a carried row is **`design/11`'s numbering, not this document's** — measured, exactly two
appear: **§4** in row 7 and **§1.3** in row 8. The two qualified references in the carried rows are
**not** the slice's: `01 §6` in row 2 is `design/01-foundation.md`'s, and *"the PRD's own §11"* in
row 8 is `PRD.md`'s. Apart from those, nothing is altered; the carried text was diffed against
`design/11` §6 sentence by sentence, mechanically, and every row matched.

**One question is deliberately NOT raised here because another register already owns it**, and is
cited instead:

- **Whether `{source}-copy-N` is a legal `skuCode`, and the code's format** — `PRD.md` §15's open
  register, filed there from `design/11` §6 and marked TBD, cited by
  `cpt-cf-bss-products-dod-clone-identity`.

`clonedFrom`'s physical storage is `design/01-foundation.md` §6's, and this document does **not**
restate it: row 18 asks the narrower question this feature's own DoDs turn on — whether the pair
`(entity id, version)` is one column or two — and cites `01` §6 for the storage half rather than
duplicating it.

### Carried verbatim from `design/11` §6

1. ~~**What is the clone door's request body?**~~
   **Answered in the slice (owner call, 2026-09-01 — P-D-75 arm 1): the overrides and nothing else** —
   `{code?, name?, newParentId?, optional replacement values for the five re-validated classes}`,
   absent meaning copy/reset per the table; the replacement slots exist because a refused
   re-validation on an immutable source must be answerable in the retry.
   Original text: Three rules require operator input — an overridable
   code, an overridable name, a replacement parent — and a fourth ("forces re-selection") may require
   re-selected values. No slice declares a clone payload, and whether those arrive in the clone
   request or in a follow-up save changes the door's shape, its validator order and whether it can
   refuse for a vocabulary reason at all.
   **Blocks**: no DoD — **resolved by P-D-75**; `cpt-cf-bss-products-dod-clone-door` carries the body.
   **Owner**: was this feature with `12-consumer-contracts`; **closed**.

2. ~~**What writes the clone's category assignments, attribute values and metadata map?**~~
   **Answered in the slice (owner call, 2026-09-01 — P-D-75 arm 2): the clone door itself, in its
   creating transaction** — P-D-46's precedent extended to the second composite creator: entity row,
   side rows, `internal_revision = 1`, no side-door events, no second grant. The tables do not ship
   yet; the rule binds when they land.
   Original text: All three
   live in side tables whose only stated writers are the save door and the metadata door — both of
   which bump `internal_revision` (defeating C3's `= 1`), emit their own events (defeating
   `inst-cn-lineage`'s "no new events") and spend a grant this door does not name. 01's create flow
   writes the entity row and its outbox row and nothing else. **P-D-46** answered the general question
   for the **save** door — `inst-fd-save-txn` now writes content in its own transaction — but the clone
   lands through the **create** door, which that arm did not reach, so this slice's atomicity claim
   still has no writer; 01 §6 carries the narrowed question.
   **Blocks**: no DoD — **resolved by P-D-75**; `dod-clone-door` and `dod-disposition-rules` carry the writer.
   **Owner**: was `01-foundation`'s door owner with `02-taxonomy-attributes`', plus `05-governance` for
   the grant; **closed**.

3. **Does the clone door need `metadata × write` beside `product|sku × write`?** 05 split that grant
   because the map is mutable in place on a **published** entity; the clone writes a new draft's map,
   which that reason does not reach — but the pair is declared per resource, not per lifecycle state,
   and 05 lists no exemption.
   **Blocks**: `cpt-cf-bss-products-dod-clone-authz`.
   **Owner**: `05-governance`'s owner with `02-taxonomy-attributes`'.

4. ~~**How are `-copy-N` and `-revived` generated?**~~
   **Answered in the slice (owner call, 2026-08-31 — P-D-62): the first free integer for the
   suggested string, decided by the index under the reservation** — P-D-37's and P-D-42's mechanism,
   no counter column and no read-then-suggest race — **and a second revival suggests
   `{name}-revived-N`** by the same rule, so the flavor survives and the suggestion path never
   refuses. The operator path is untouched: a collision on a supplied name or code stays the ordinary
   `DUPLICATE_NAME`/`DUPLICATE_CODE`.
   Original text: `N` is never defined (per-source counter, global,
   first free integer), `-revived` carries no counter at all, and the index admits one holder per name
   in every non-`discarded` state — so a second revival of one lineage produces a suggestion the
   registry must refuse, and concurrent clones computing `N` by a read race each other.
   **Blocks**: no DoD — **resolved by P-D-62**; `cpt-cf-bss-products-dod-clone-identity` and
   `cpt-cf-bss-products-dod-rename-rule` are both freed — the only row that freed two.
   **Owner**: was this feature; **closed**.

5. ~~**What does the door answer for a `discarded` source?**~~
   **Answered in the slice (owner call, 2026-09-01 — P-D-75 arm 3): refused
   `CLONE_SOURCE_DISCARDED`, 409, minted on P-D-52's test** — `ENTITY_TERMINAL` means a head *write*
   while the clone writes nothing to the source, and the bare 404 carries no code channel.
   Original text: C1 admits four states and `discarded` is
   the fifth, reachable and addressable; nothing says whether it is a 404-class miss, a state refusal
   or admitted, and `ENTITY_TERMINAL` cannot be reused as-is because the clone writes nothing to the
   source while a `retired` source is explicitly allowed.
   **Blocks**: no DoD — **resolved by P-D-75**; `cpt-cf-bss-products-dod-clone-door` carries the refusal.
   **Owner**: was the taxonomy owner with this feature; **closed**.

6. **Which surface answers the reverse lineage lookup — what was cloned from a given entity?** The
   absence of a clone event is justified by the lineage field being "queryable", and the field appears
   in no read model and no SDK shape; a clone is a draft, which the browse projection cannot see at
   all.
   **Blocks**: `cpt-cf-bss-products-dod-clone-lineage`.
   **Owner**: `08-read-models`' and `12-consumer-contracts`' owners — expose it, or withdraw the
   justification.

7. ~~**Where does the per-child ledger of a product-with-SKUs clone live?**~~
   **Answered (owner call, 2026-09-01 — P-D-72 arm 2): in the data itself.** The new parent's
   children carry their own `cloned_from` pointers, so the same-key retry **resumes** the family act
   — the claim joins the parent's transaction, committed-but-unanswered means *in progress*, the
   re-entry skips already-cloned sources and stores the answer at completion. No table is built; the
   response receipt stays a receipt. The P-D-42 extension is named on the decision.
   Original text: §4 declares no tables and no
   events, so the ledger is response-only and a crash between children leaves an unreported half-clone
   with no resumption path — 09, whose shape this cites, has both a table and a resume rule.
   **Blocks**: no DoD — **resolved by P-D-72**; `cpt-cf-bss-products-dod-clone-children` carries the mechanism.
   **Owner**: was this feature with `09-bulk-promotion`'s storage owner; **closed**.

8. **Which role holds the clone grant?** §1.3 gives it to the product manager and the PRD's own §11
   console gives clone to an Operator/Platform owner, while the door spends the authoring pair. The
   PRD disagrees with itself and no document maps roles to grants.
   **Blocks**: `cpt-cf-bss-products-dod-clone-authz`.
   **Owner**: the PRD owner with `05-governance`.

9. **May a slice restate a decision whose propagation field does not name it?** This slice's central
   rule leans on P-D-04, whose surface names `design/01-foundation.md`,
   `design/02-taxonomy-attributes.md`, `design/04-lifecycle.md` and `design/09-bulk-promotion.md` —
   not this file; the same holds for P-D-05 and P-D-06, while P-D-21, P-D-25 and P-D-35 name this
   file explicitly. The register's own standard is that a propagation field describes what a document
   says — which makes these register omissions rather than defects here, but nothing states whether
   the citing side owes an entry.
   **Blocks**: no DoD — it is a register-hygiene question about `DECISIONS.md`'s own convention.
   **Owner**: the register's owner.

10. **Who owns mass cloning?** This slice puts it Out and pointed at 09, whose resolver is total over
    identity and produces no copies — it classifies such a row as a no-op, an update to the source, or
    (for revival) a conflict naming clone as the only path. So the case is claimed by nobody.
    **Blocks**: no DoD — mass cloning is Out of this feature's scope.
    **Owner**: the design-set owner with `09-bulk-promotion`.

### Raised here rather than carried

11. **Which phase hosts the disposition re-validation set?** C4 and **P-D-49** arm 3 require the
    refusal to name every failing field class, and the shipped `ValidationPipeline` *"stops at the
    first failing phase"*, so a set split across phases cannot produce that refusal. `Phase::Shape`
    covers *"the resolvability of every reference the payload carries"* and is the natural host, but
    no document assigns the clone's rules to a phase, and `ATTRIBUTE_SCOPE_VIOLATION` and
    `CONTENT_PII_BLOCKED` are plausibly `RegisteredValidators` rather than `Shape`.
    **Blocks**: `cpt-cf-bss-products-dod-disposition-rules`.
    **Owner**: this feature with `01-foundation`'s pipeline owner.

12. ~~**Is C4's "every field class that failed" scoped to the vocabulary classes or to the whole act?**~~
   **Answered (owner call, 2026-09-01 — P-D-75 arm 4): C4 is scoped to the re-validated classes** —
    the row's own closing arm. Identity collisions stay P-D-37's phase rules; the pre-flight probe was
    declined as a read racing the reservation it predicts.
   Original text:
    `DUPLICATE_NAME` and `DUPLICATE_CODE` are `Identity`-phase and, per **P-D-37**, are decided under
    the write and cannot collect a second code at all. So a clone failing both a re-validation and an
    operator-supplied name collision reports only the earlier phase, and the operator learns of the
    collision on the retry. Either C4 is scoped to the re-validated classes — in which case saying so
    closes it — or the door owes a pre-flight uniqueness probe, which introduces a TOCTOU window the
    index exists to avoid.
   **Blocks**: no DoD — **resolved by P-D-75**.
   **Owner**: was this feature with `01-foundation`'s; **closed**.

13. ~~**Which of the disposition rules registers first?**~~
    **Answered (owner call, 2026-08-31 — P-D-55): in `design/11` §3.1's own row order.** The table is
    the normative list of the field classes and it is ordered, so the order it already carries becomes
    the registration order — nothing is invented, `audit_code()` keeps returning
    `violations.first()`, and the precedence ranks **rows**, so the two rows naming no code take their
    place when one is minted. It is a tie-break rather than a correctness question, which is
    **P-D-37**'s own framing: the caller's rejection carries every violation, the audit row records
    one code, and what that code buys is attribution.
    **And the order is unobservable at this commit for two independent reasons**, neither of them this
    feature's: `audit_code()` has zero production callers, and every registered rule raises the
    literal `"VALIDATION"` (`domain/rules.rs:73`), so a routed call would not discriminate either.
    That half is **`design/01-foundation.md` §6 item 2**'s — the same question in its general form,
    already filed with its owner — and no duplicate is filed here.
    Original text: Within a phase, rules run in registration
    order, `ValidationReport::audit_code` stores `violations.first()`, and `AC #38`'s map reads the
    **stored** code. So the audit code for a multi-class clone failure is a consequence of
    registration order, and no document fixes that order. **P-D-37** fixed a precedence for the
    `state` phase's four codes for exactly this reason; the disposition set has none.
    **Blocks**: no DoD — **resolved by P-D-55**; `cpt-cf-bss-products-dod-clone-audit` carries the
    answer and is freed.
    **Owner**: was this feature with `01-foundation`'s; **closed**, the observability half staying
    with `01`.

14. ~~**What is the clone's idempotency key, and does a retried clone return the first clone or make a
    second?**~~
   **Answered (owner call, 2026-09-01 — P-D-75 arm 5): keyed, ordinary semantics** — P-D-72's family
    resume already presupposed the key; keyless skips the phase, a keyed retry replays the first
    clone, which is what a crash-retrying caller needs to not double-clone.
   Original text: The pipeline's first phase is `Idempotency` and the door is a mutation, so a key is
    admitted; but a clone is a create with **minted** ids, so replaying one cannot be the same
    request in the sense the create door means — two identical clone requests are two legitimate
    clones. Nothing states whether the clone door is keyless (`Phase::Idempotency` being *"skipped,
    never failed, on a keyless request"*) or keyed with the ordinary semantics.
   **Blocks**: no DoD — **resolved by P-D-75**; `cpt-cf-bss-products-dod-clone-door` carries the key.
   **Owner**: was this feature with `01-foundation`'s; **closed**.

15. ~~**Does a `deprecated` source clone as `published` content or as its deprecated head?**~~
    **Answered (owner call, 2026-09-01 — P-D-78): the last frozen version, uniformly** — the
    retirement design keeps the head open and re-announces consumers onto every moved version
    (`design/04` `inst-rt-initiate`: *"consumers key on `(skuId, effectiveAt)` and take the
    latest"*), and nothing records which version was current at deprecation
    (`deprecation_provenance` is `direct|cascaded`, not a bookmark), so the other read has no
    operand. `cloned_from_version` records exactly the version read.
    Original text: `01-foundation` §4.3 excludes `lifecycle_state` and `deprecation_provenance` from frozen content
    (**P-D-24**, extended by **P-D-35**), so a `deprecated` entity's last frozen version is
    indistinguishable from its published one — which is correct for content and leaves the read
    surface unambiguous. What is unstated is whether a `deprecated` source whose head has moved
    **since** deprecation clones the newer frozen version or the one current at deprecation.
    **Blocks**: no DoD — **resolved by P-D-78**; `cpt-cf-bss-products-dod-clone-read-surface`
    keeps rows 16, 20 and 22.
    **Owner**: was `04-lifecycle`'s owner with this feature; **closed**.

16. **What does a clone of a source with no frozen version at all do?** C1 admits a `published`
    source, and `01-foundation`'s publish door writes the version row, so the case should not arise —
    but a `retired` entity whose versions were collected by `10-retention-erasure`'s GC has a
    lifecycle state implying frozen content and no row to read. `10`'s retention gate protects
    versions with live freeze registrations, not versions a future clone might want.
    **Blocks**: `cpt-cf-bss-products-dod-clone-read-surface`.
    **Owner**: `10-retention-erasure`'s owner with this feature.

17. **Do the twelve variantless codes ship with their owning slices or with this feature, and as
    what?** `UNRECOGNIZED_UNIT`, `UNIT_DEPRECATED`, `PLAN_TIER_UNKNOWN`, `PLAN_TIER_DEPRECATED`,
    `CATEGORY_RETIRED`, `ACCOUNTING_CODE_UNKNOWN`, `ACCOUNTING_CODE_DEPRECATED`,
    `ATTRIBUTE_DEFINITION_UNKNOWN`, `ATTRIBUTE_DEFINITION_DEPRECATED` and
    `ATTRIBUTE_SCOPE_VIOLATION` have no `DomainError` variant and are named nowhere in the crate.
    **Two of the twelve are already assigned** and need no ruling: `infra/error_mapping.rs` records
    `RETIREMENT_PENDING` as slice `04`'s and `CONTENT_PII_BLOCKED` as slice `02`'s, deliberately
    unmapped because *"`DomainError` has no variant for any of the three"*. What is open for the
    other ten is **the form**: a `DomainError` variant each, or `Violation` codes inside one
    `DomainError::Validation` report. The second is what the shipped collector supports and what
    §7 row 13 turns on; the first would mint variants this document says it does not mint.
    **Blocks**: `cpt-cf-bss-products-dod-revalidation-codes`.
    **Owner**: the design-set owner, with `02`, `03` and `04`.

18. ~~**Is `cloned_from` one column or two?**~~
   **Answered (owner call, 2026-09-01 — P-D-76): two columns**, the P-D-50 convention a third
    time — `cloned_from` (nullable uuid) and `cloned_from_version` (nullable bigint; NULL under a set
    source = read at the head, the `'draft'` sentinel made representable), shape-CHECKed, both in the
    head guards' immutable set and **landed the same day**: `m20260829_000002`/`000003` edited in
    place, the registry's waiting `FieldClass::CreateOnly` populated, and the pair joined the content
    rosters by their own membership rule.
   Original text: `design/11` §4 says *"One column (`cloned_from`,
    nullable ...) on both entity tables"*, while `inst-cn-lineage` records
    `(entity id, published_version | 'draft')` — a pair. Whether that is one composite column, a
    column plus a nullable version integer, or an encoded string is unstated, and it decides whether
    the `'draft'` sentinel is representable.
   **Blocks**: no DoD — **resolved by P-D-76**; `cpt-cf-bss-products-dod-cloned-from-column` is built and ticked.
   **Owner**: was `01-foundation`'s schema owner, whose §6 already carries the storage half; **closed**.

19. ~~**Does `cloned_from` point across tables?**~~
   **Answered (owner call, 2026-09-01 — P-D-72 arm 1): a child's `cloned_from` names its own source
    SKU** — uniform, same-kind, never the parent act; the family stays walkable through `parent_id`
    plus the children's own pointers, which is exactly the operand the resume re-entry reads.
   Original text: A SKU clone's source is a SKU and a Product clone's a
    Product, so the column is same-table today — but a product-with-SKUs clone creates children whose
    sources are the source product's children, and nothing says whether a child's `cloned_from` names
    its own source SKU or the parent act. The first makes the column uniform; the second makes the
    batch walkable.
   **Blocks**: no DoD — **resolved by P-D-72**; `cpt-cf-bss-products-dod-clone-lineage` and `dod-clone-children` carry it.
   **Owner**: was this feature; **closed**.

20. **Which store answers the metadata map read for a `retired` source?** **P-D-06** places the map
    outside frozen version content so it survives retirement, and `design/11` §2 relies on that —
    but no metadata-map table exists in the crate at this commit, and `02-taxonomy-attributes` owns
    it. The read is specified against a store whose shape is not yet settled.
    **Blocks**: `cpt-cf-bss-products-dod-clone-read-surface`,
    `cpt-cf-bss-products-dod-disposition-rules`.
    **Owner**: `02-taxonomy-attributes`' owner.

21. **Is a `p3` feature the right priority for the only exit from a terminal state?** Every id this
    document **declares** is `p3`, following `design/11` (six of six) and `PRD.md` §6.10's
    `fr-clone`; the two head lines carry `p2` because they mirror DECOMPOSITION §2.11's own token,
    which is a third value again. But two **`p1`** functional
    requirements name this `p3` feature as their only remedy: `cpt-cf-bss-products-fr-lifecycle-transitions`
    makes `retired` terminal with *"revival only via clone"*, and
    `cpt-cf-bss-products-fr-field-mutability-matrix` makes structural identity *"immutable and never
    correctable in place (remedied only by retire + clone)"*. §1's glossary repeats the first and
    carries no priority of its own. Raised as an observation about the set, not a request to re-prioritise.
    **Blocks**: no DoD — it is a priority question about the feature as a whole.
    **Owner**: the PRD owner.

22. **Does the clone door consume a version of the source at all, or a snapshot of it?** The read
    surface reads the last frozen version, and `10-retention-erasure`'s freeze registrations exist so
    that a participant can hold a version against collection. A clone reads a version and does not
    register — correctly, since it copies rather than references — but nothing states that a clone's
    read is exempt from the freeze protocol, and `06-catalog-version`'s participant model does not
    enumerate readers.
    **Blocks**: `cpt-cf-bss-products-dod-clone-read-surface`.
    **Owner**: `06-catalog-version`'s owner.

23. ~~**Who owns the decoder for a frozen version's canonical rendering?**~~
    **Answered (owner call, 2026-09-01 — P-D-77): `01-foundation`'s, beside the renderer** —
    `domain::canonical` gains the inverse of `canonical_rendering`, the round-trip test beside the
    renderer's own, and the clone read surface is its first consumer; the row's own closing arm
    (a parse at the door is the second serialization rule the module exists to prevent) decides
    the placement.
    Original text: `products_entity_version.content` is a single `String` — *"the canonical rendering itself …
    rather than one column per content field"* — `repo.rs` has no reader for the table, and
    `domain::canonical` renders without parsing while `repo.rs` *"deliberately imports no
    canonicalizer"*. The clone's read surface needs both halves. Whether the decoder is
    `01-foundation`'s (beside `canonical_rendering`, which is where a round-trip belongs) or this
    feature's is unstated, and building it at the clone door would create the second serialization
    rule `domain/canonical.rs` exists to prevent.
    **Blocks**: no DoD — **resolved by P-D-77**; `cpt-cf-bss-products-dod-clone-read-surface`
    keeps rows 16, 20 and 22.
    **Owner**: was `01-foundation`'s canonical-rendering owner, with this feature; **closed**.

24. **Is DECOMPOSITION's `Domain Model Entities` field a listing convention or an ontological
    claim?** This pass listed `DispositionTable` and `CloneDoor` there on §2.7's and §2.10's
    precedent — both list their slice's §1.7 names — while the same entry says *"Neither is an
    aggregate of its own"* and `cpt-cf-bss-products-dod-clone-seams` derives a reviewer rule from
    that. Nothing states which reading governs, so the derived rule rests on an unsettled basis.
    **Blocks**: no DoD — `dod-clone-seams` stands under either reading.
    **Owner**: the design-set owner.

25. **Which pipeline hosts the disposition rules, and over which subject type?**
    `cpt-cf-bss-products-dod-disposition-rules` calls the re-validating rows registered
    `ValidationRule` implementations, but the **create doors run no pipeline**: the only two
    `ValidationPipeline` constructions in the crate are `publish_revalidation_pipeline` in each door,
    and `create_product` builds its report inline with `report.violate("VALIDATION", …)` literals.
    `ValidationPipeline<S>` is generic over one subject type, and the shipped candidate
    `CreateEntityCandidate` carries `tenant_id`, `brand_id`, `name` and `code` — not one vocabulary
    reference. So either `01-foundation` widens the create door to a registered-rule pipeline over a
    candidate carrying the disposition operands, or the clone door runs its own pipeline before
    delegating — which contradicts `cpt-cf-bss-products-dod-clone-door`'s "same validators, once".
    **Blocks**: `cpt-cf-bss-products-dod-disposition-rules`.
    **Owner**: `01-foundation`'s door and pipeline owner, with this feature. Row 11 asks *which
    phase*; this row asks *which pipeline*, and the second must be answered first.

26. ~~**What status does a partial product-with-SKUs clone answer, and what is a ledger entry?**~~
   **Answered (owner call, 2026-09-01 — P-D-72 arm 3): `201` with a per-child receipt** —
    `{source sku_id, disposition ∈ {created, failed}, new sku_id | code + violations}`, codes the
    owning doors' verbatim, no parallel taxonomy. Parent-plus-surviving-children is the valid intended
    end state, so the partial is not an error status; a failing parent stays the ordinary refusal of
    the whole act.
   Original text:
    `design/11` §2 fixes the single-entity answer at **201** and names the per-child ledger without
    shaping it — no field, no entry contents, no status for the partial outcome. `09-bulk-promotion`,
    whose shape the slice cites, is explicitly Out. So `cpt-cf-bss-products-dod-clone-children`'s
    acceptance criterion — the response reports the failed child — cannot be turned into an
    assertion. What must be decided: the partial-clone status, the ledger field on the response, and
    whether a failed child carries its whole collected refusal or one code.
   **Blocks**: no DoD — **resolved by P-D-72**; `cpt-cf-bss-products-dod-clone-children` is freed.
   **Owner**: was this feature with `12-consumer-contracts`, which row 1 already holds the request half
    of; **closed**.

27. **Do the two accounting codes survive into the positive-control count?**
    `cpt-cf-bss-products-dod-clone-tests` states a count per code raised, and **P-D-47** arm 3 makes
    `ACCOUNTING_CODE_UNKNOWN` and `ACCOUNTING_CODE_DEPRECATED` *"exactly as contingent as the two
    columns (`PRD` §15's ownership question) and go with them if it goes"*. No document says whether
    the test count is stated against the codes as minted or against those that survive that call.
    **Blocks**: `cpt-cf-bss-products-dod-clone-tests`.
    **Owner**: the PRD owner with `03-sku-classification`'s.

### Owed to other documents, recorded and deliberately not edited

- **Two migration module docs attribute `cloned_from` to slice 03.**
  `products/src/infra/storage/migrations/m20260829_000002_create_products_product.rs:111-112` reads
  *"Slice 03 brings `cloned_from`, `deprecation_provenance` and `replaced_by_sku_id`"* and
  `m20260829_000003_create_products_sku.rs:101-102` reads *"`cloned_from`, `deprecation_provenance` and
  `replaced_by_sku_id` arrive with slice 03"*. **Four** other files in the same crate name **slice
  11** — `domain/bucket.rs`, `domain/bucket_tests.rs`, `api/rest/products.rs`, `api/rest/skus.rs` —
  two more name the column without a slice, and `design/03-sku-classification.md` mentions it zero
  times. Owner: `01-foundation`'s migration owner.
- **`design/11` §6 row 9's decision census is short by two.** The row says *"P-D-21, P-D-25 and
  P-D-35 name this file explicitly"*; measured at HEAD, **five** decisions name
  `design/11-clone.md` in a propagation field — the three it lists plus **P-D-47** and **P-D-49**,
  the second being the decision this feature leans on hardest. The row is carried verbatim in §7 and
  is **deliberately not repaired there**: repairing a carried row is authoring inside a quotation.
  Owner: `design/11`'s owner, whose row it is.
- **`design/11` §6 row 10 states three of `09`'s four classifications.** The row reads *"a no-op, an
  update to the source, or (for revival) a conflict"*; `design/09-bulk-promotion` C5 is *"**Four
  classifications, exhaustive**"*, the fourth being **create** on an unknown identity. §2's boundary
  above states the four; the carried row is left as the slice wrote it. Owner: `design/11`'s owner.
- **`m20260829_000003_create_products_sku.rs` carries no landing-site sentence.** Its sibling
  product migration says a later column's clauses *"join this same file's whitelist rather than a
  follow-up migration"*; the SKU migration says nothing about where its next column lands, and
  `cpt-cf-bss-products-dod-cloned-from-column` applies the product migration's rule to it. Owner:
  `01-foundation`'s migration owner.
- **`design/05-governance`'s `metadata × write` has no code.** The pair is declared in §3.2's RBAC
  catalog against
  `02`'s map door; `authz::labels::ALL` is `[PRODUCT, SKU]` and the permission roster is closed at
  six. Owner: `05-governance`'s owner. Recorded here because row 3 above turns on it.
