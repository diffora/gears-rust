# Decomposition: BSS Product & SKU Registry

**Overall implementation status:**
- [ ] `p1` - **ID**: `cpt-cf-bss-products-status-overall`

<!-- toc -->

- [1. Overview](#1-overview)
- [2. Entries](#2-entries)
  - [2.1 Registry Foundation - HIGH](#21-registry-foundation---high)
  - [2.2 Taxonomy & Attributes - HIGH](#22-taxonomy--attributes---high)
  - [2.3 SKU Classification - HIGH](#23-sku-classification---high)
  - [2.4 Lifecycle Policy - MEDIUM](#24-lifecycle-policy---medium)
  - [2.5 Governance & Approvals - MEDIUM](#25-governance--approvals---medium)
  - [2.6 Catalog Version & Freeze - MEDIUM](#26-catalog-version--freeze---medium)
  - [2.7 Reference Signal - MEDIUM](#27-reference-signal---medium)
  - [2.8 Read Models - MEDIUM](#28-read-models---medium)
  - [2.9 Bulk & Promotion - MEDIUM](#29-bulk--promotion---medium)
  - [2.10 Retention & Erasure - LOW](#210-retention--erasure---low)
  - [2.11 Clone - LOW](#211-clone---low)
  - [2.12 Consumer Contracts - MEDIUM](#212-consumer-contracts---medium)
- [3. Feature Dependencies](#3-feature-dependencies)

<!-- /toc -->

## 1. Overview

This decomposition is a **projection of the existing design set, not a new carve-up of it**. The
gear's design already nests into twelve slices under [`design/`](./design/), and
[`DESIGN.md`](./DESIGN.md) §1.3 states that their numeric prefix *is* implementation order. Each
slice already declares what it depends on, which PRD §6 subsection it answers, and its own
in/out scope. This document lifts those declarations into the kit's DECOMPOSITION shape so the
FEATURE artifacts and `@cpt-*` implementation traceability have an upstream to hang from; it
introduces no requirement, no architecture decision and no scope that the slices did not
already carry.

**One feature per slice, twelve in total.** The granularity was measured rather than chosen. A
slice's breadth — its §2 actor flows plus its §3 processes — runs from 6 to 10, against a
donor feature's observed range of 2 to 8, measured with the same metric over the seven features
in `gears/file-storage/docs/features/`. `01-foundation` and `06-catalog-version`, both at 10,
therefore sit **above** the donor's largest feature and the fact is stated rather than smoothed:
both are kept whole because splitting them would cut the Foundation's single write door and the
freeze protocol respectively, seams the design set does not have. Grouping slices would instead
merge dependency sets the set deliberately keeps apart. Depth within a slice is carried by the FEATURE's own
Definitions of Done, which is where per-increment progress is tracked.

Two entries deviate from that uniformity and are called out rather than smoothed over:

- **`11-clone` sits at the donor's minimum size** — 2 units of breadth and 6 declared
  instructions, against a donor minimum of 2 units and a typical 3. It stays its own feature
  because the design set gives it its own dependency set and its own slice, but it is the
  candidate to fold into `04-lifecycle` if implementation finds it has no independent surface.
- **`12-consumer-contracts` is half verification, not behavior.** Its §1.5 in-scope list mixes
  runtime behavior (event schema versioning, the replay/bootstrap contract, the §9 SDK
  surfaces) with verification artifacts (the seam-suite specification, the consumer-obligation
  register, the completeness checks, §17.2 traceability). Only the behavioral half is a
  feature; the verification half is a CI and review track that this document does not decompose.
  Its behavioral half is three scope items, the smallest entry in the set, and the excluded
  verification half is roughly its equal in size.

**Entry priority is derived, not judged.** HIGH, MEDIUM and LOW come from
[`DESIGN.md`](./DESIGN.md) §1.3's Phase column: phase 0/1 and 1 map to HIGH; 1/2, 2 and 2/3 map to
MEDIUM; 3 maps to LOW. The `p`-marker on a checkbox is a different thing — it carries the build
priority of the entry rather than the priority of the id being referenced, which is why every
entry is uniformly `p1` except `11-clone`. Where a referenced id's own declaration carries a
different marker, the declaration governs and this document defers to it.

**Each slice's §1.6 constraint rows are not projected here.** The "Design Constraints Covered"
field carries [`DESIGN.md`](./DESIGN.md) §2.2's five gear-level ids, which is the id-bearing
universe. A slice's own C-rows carry no `cpt-*` id and are carried into that slice's FEATURE
artifact instead, where the implementer needs them.

The `features/` links below name the FEATURE artifact each entry will own. Those artifacts are
authored downstream of this document, in the order §3 establishes.

## 2. Entries

### 2.1 [Registry Foundation](features/foundation.md) - HIGH

- [ ] `p1` - **ID**: `cpt-cf-bss-products-feature-foundation`

- **Type**: Core

- **Phases**: [`DESIGN.md`](./DESIGN.md) §1.3 phase 0/1 — the root of the dependency order; every
  other feature in this document depends on it.

- **Purpose**: The shared engine every capability of the gear publishes through. It owns the
  `Product`/`SKU` entity model and identity rules (server-minted UUIDs, atomically reserved
  `skuCode` and `productCode`), the two version counters (internal revision against published
  version), the lifecycle state-machine core, the fail-closed registered-validator pipeline,
  append-only published-version history, per-row optimistic concurrency, tenant-scoped
  idempotency, the broker-native event fan-out through the toolkit outbox, and the append-only
  audit trail. It deliberately owns **no capability policy**: it does not know what a `PlanTier`,
  a metering unit, a materiality threshold or a freeze participant is. Capability features author
  draft state through its write doors, register their validation rules into its pipeline, and
  call its publish path.

- **Depends On**: None

- **Scope**:
  - Entity model and storage shape for the `Product`/`SKU` core columns
  - Identity plus `skuCode`/`productCode` reservation
  - Revision and version mechanics
  - State-machine core: edge list, terminality, physical floor
  - Validation pipeline frame and the error taxonomy
  - The gear's five wire doors — create, save, publish, discard and the authoring head read —
    and the governance gate's host contract (`Gate` / `PreAuthorized(approvalId)`); the grants
    those doors spend belong to `05-governance`
  - Field-mutability enforcement frame (bucket routing)
  - Idempotency
  - ETag concurrency
  - The toolkit outbox's enqueue path, the broker SDK's producer on top of it, envelope
    discipline, and per-tenant ordering by the broker's partition selection
  - Audit of the acts that emit no event: refusals, reads under elevation, and committed acts
    declared to emit no broker event
  - The interim parent-child brand/region containment check, whose final rule is
    `04-lifecycle`'s
  - Resolution and first-appearance minting of the acting principal's `actor_ref` through
    `10-retention-erasure`'s identity-ref table
  - The reserved platform audit-sealing seam's columns, `CHECK` and one-way trigger — present
    from the first migration, never sealed here
  - The column **guards** riding this feature's first migration and publish door:
    `cloned_from`'s create-only immutability, the publish door's `composition_pending` write, the
    interim row-image predicates on `deprecation_provenance` and `replaced_by_sku_id`, the audit
    log's retention `DELETE` arm, and the version table's one admitted `DELETE`. The semantics
    behind each guard belong to the owning feature.

- **Out of scope**:
  - Category and attribute content rules (`02-taxonomy-attributes`)
  - Typing, classification and metering policy (`03-sku-classification`)
  - Deprecation and retirement policy, scheduling, cascades (`04-lifecycle`)
  - Materiality, approvals, RBAC grants, break-glass (`05-governance`)
  - `CatalogVersion` and freezes (`06-catalog-version`)
  - `SkuReferenceCount` and corrections (`07-reference-signal`)
  - Read models (`08-read-models`)
  - Bulk operations (`09-bulk-promotion`)
  - Retention and erasure execution (`10-retention-erasure`)
  - Clone (`11-clone`)
  - Seam suite, replay and bootstrap (`12-consumer-contracts`)

- **Requirements Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-fr-identifier-contract`
  - [ ] `p1` - `cpt-cf-bss-products-fr-idempotent-authoring`
  - [ ] `p1` - `cpt-cf-bss-products-fr-skucode-reservation-concurrency`
  - [ ] `p1` - `cpt-cf-bss-products-fr-registry-eventing-audit`
  - [ ] `p1` - `cpt-cf-bss-products-fr-create-product` — uniqueness clause only; the category and
    attribute content rules are `02-taxonomy-attributes`'
  - [ ] `p1` - `cpt-cf-bss-products-fr-define-sku` — identity clause only; typing and
    classification are `03-sku-classification`'
  - [ ] `p1` - `cpt-cf-bss-products-fr-revision-vs-version` — the two counters and the history;
    version binding at freeze is `06-catalog-version`'
  - [ ] `p1` - `cpt-cf-bss-products-fr-lifecycle-transitions` — the machine core; the scheduling
    clauses are `04-lifecycle`'
  - [ ] `p1` - `cpt-cf-bss-products-fr-field-mutability-matrix` — the enforcement frame
  - [ ] `p1` - `cpt-cf-bss-products-fr-expected-failure-behavior` — the taxonomy's home; the
    retention-orphan row is `10-retention-erasure`'
  - [ ] `p1` - `cpt-cf-bss-products-fr-parent-child-integrity` — the interim containment check;
    the final rule is `04-lifecycle`'
  - [ ] `p1` - `cpt-cf-bss-products-fr-event-delivery-resilience` — the durable-acceptance
    clause; the per-consumer delivery and dead-letter projection is `08-read-models`'
  - [ ] `p1` - `cpt-cf-bss-products-nfr-publication-propagation` — the outbox half of the budget
  - [ ] `p1` - `cpt-cf-bss-products-nfr-scale-extensibility` — the entity-count half: the
    head/version split and the index shape
  - [ ] `p1` - `cpt-cf-bss-products-nfr-determinism-integrity` — the frame only: the pipeline,
    the edge list and the trigger whitelist its registered validators run in
  - [ ] `p1` - `cpt-cf-bss-products-usecase-product-sku-editor`
  - [ ] `p1` - `cpt-cf-bss-products-interface-authoring-publish`
  - [ ] `p1` - `cpt-cf-bss-products-contract-registry-events`

- **Design Principles Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-principle-fail-closed`
  - [ ] `p1` - `cpt-cf-bss-products-principle-two-version-counters`
  - [ ] `p1` - `cpt-cf-bss-products-principle-registered-validators`
  - [ ] `p1` - `cpt-cf-bss-products-principle-publish-through-engine`
  - [ ] `p1` - `cpt-cf-bss-products-principle-forward-only`

- **Design Constraints Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-constraint-immutable-identity`
  - [ ] `p1` - `cpt-cf-bss-products-constraint-broker-native-events`
  - [ ] `p1` - `cpt-cf-bss-products-constraint-tenant-isolation`
  - [ ] `p1` - `cpt-cf-bss-products-constraint-no-commercial-concern`
  - [ ] `p1` - `cpt-cf-bss-products-constraint-gts-types-not-instances`

- **Domain Model Entities**:
  - Product
  - SKU
  - EntityVersion (the frozen published row)
  - IdempotencyRecord
  - AuditRecord

- **Design Components**:

  - [ ] `p1` - `cpt-cf-bss-products-component-registry-foundation`

- **API**:
  - `POST /bss-products/v1/products` — create a Product
  - `POST /bss-products/v1/skus` — define a SKU
  - `PATCH /bss-products/v1/{products|skus}/{id}` — save an edit to a draft, published or
    deprecated head
  - `POST /bss-products/v1/{products|skus}/{id}/publish` — publish an entity
  - `POST /bss-products/v1/{products|skus}/{id}/discard` — discard a never-published draft
  - `GET /bss-products/v1/{products|skus}/{id}` — the authoring head read

- **Sequences**:

  - [ ] `p1` - `cpt-cf-bss-products-seq-authoring-publish`

- **Data**:
  - `products_product`
  - `products_sku`
  - `products_entity_version`
  - `products_idempotency`
  - `products_audit_log`
  - `products_product_category` — the table is defined by `02-taxonomy-attributes` and written by
    this feature's **save** door, in the same transaction as the head-row update (**P-D-46**).
    Whether the create door should write content on the same terms is slice 01's open item 11 and
    is unresolved; `DESIGN.md` §3.5 annotates this table as written by the create door, which is
    the reading open item 11 exists to settle
  - The event outbox is the toolkit's (`toolkit_db::outbox`), not a gear table

### 2.2 [Taxonomy & Attributes](features/taxonomy-attributes.md) - HIGH

- [ ] `p1` - **ID**: `cpt-cf-bss-products-feature-taxonomy-attributes`

- **Type**: Capability

- **Phases**: [`DESIGN.md`](./DESIGN.md) §1.3 phase 1

- **Purpose**: The category tree and the attribute system the catalog is described with. It owns
  typed, localized attribute definitions with brand and region visibility and a
  deprecate-then-remove retirement path, attribute values with locale fallback resolution, the
  well-known seeds, the metadata map, and the governed-live-entity mutation pattern that
  `03-sku-classification` reuses for its own live taxonomies. It also places the content-PII
  write-block hook whose detector policy belongs to `10-retention-erasure`.

- **Depends On**: `cpt-cf-bss-products-feature-foundation`

- **Scope**:
  - Category tree, its invariants and its operations
  - Attribute definitions: typed, localized flag, brand and region visibility, deprecate-then-remove
  - Attribute values and locale fallback resolution
  - Well-known seeds
  - The metadata map
  - The governed-live-entity mutation pattern, whose op envelope is handed to `05-governance`
  - The content-PII write-block **hook**; the detector policy is `10-retention-erasure`'

- **Out of scope**:
  - The approval machinery itself (`05-governance`)
  - Read-model and search projections, faceting, category read-model warming (`08-read-models`)
  - Erasure execution (`10-retention-erasure`)
  - `PlanTier` and recognized sets — also governed live entities, but owned by
    `03-sku-classification`, which reuses this feature's pattern

- **Requirements Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-fr-manage-taxonomy`
  - [ ] `p1` - `cpt-cf-bss-products-fr-localized-attributes`
  - [ ] `p1` - `cpt-cf-bss-products-fr-create-product` — the category and attribute content rules;
    the uniqueness clause is `01-foundation`'
  - [ ] `p1` - `cpt-cf-bss-products-fr-retention-erasure` — the write-block hook placement only;
    the detector policy and the erasure act are `10-retention-erasure`'
  - [ ] `p1` - `cpt-cf-bss-products-nfr-scale-extensibility` — the extensibility-limits half: max
    taxonomy depth and max children per node

- **Design Principles Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-principle-registered-validators`

- **Design Constraints Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-constraint-tenant-isolation`
  - [ ] `p1` - `cpt-cf-bss-products-constraint-immutable-identity`
  - [ ] `p1` - `cpt-cf-bss-products-constraint-broker-native-events`

- **Domain Model Entities**:
  - Category
  - AttributeDefinition
  - AttributeValue
  - MetadataMap
  - GovernedLiveOp
  - WellKnownSeed

- **Design Components**:

  - [ ] `p1` - `cpt-cf-bss-products-component-capability-handlers`

- **API**:
  - `PATCH /bss-products/v1/{products|skus}/{id}/metadata` — the metadata map
  - Category and attribute administration ride the governed-live-op envelope rather than
    dedicated routes; the design set declares no separate paths for them

- **Sequences**: None — this feature contributes validators and content to
  `cpt-cf-bss-products-seq-authoring-publish` rather than owning a sequence of its own

- **Data**:
  - `products_category`
  - `products_product_category`
  - `products_attribute_definition`
  - `products_attribute_value`
  - `products_metadata`

### 2.3 [SKU Classification](features/sku-classification.md) - HIGH

- [ ] `p1` - **ID**: `cpt-cf-bss-products-feature-sku-classification`

- **Type**: Capability

- **Phases**: [`DESIGN.md`](./DESIGN.md) §1.3 phase 1

- **Purpose**: What a SKU *is* commercially, short of any price. It owns SKU typing and the
  per-type required-field validators, the `sellable` member, the `PlanTier` taxonomy and a SKU's
  value in it, the recognized code and unit sets with their validators, the metering-unit
  declaration and its `usageTypeRef` resolution, unit de-listing, and the mutability-bucket
  registration for every field it owns.

- **Depends On**: `cpt-cf-bss-products-feature-foundation`,
  `cpt-cf-bss-products-feature-taxonomy-attributes`

- **Scope**:
  - Type and the per-type required-field validators
  - `sellable`
  - The `PlanTier` taxonomy and the SKU's value in it
  - Recognized code sets and their code validators
  - The recognized unit set, the metering-unit declaration and `usageTypeRef` resolution
  - Unit de-listing
  - The uncomposed-bundle publish override registration
  - Mutability-bucket registration for every field this feature owns

- **Out of scope**:
  - Bundle composition, which is plan-price's
  - `compositionPending` clearing (`06-catalog-version`)
  - The fresh-zero correction door for bucket-ii fields (`07-reference-signal`)
  - Plan-side enforcement — tier presence, dimension subset — which is pricing's
  - Usage collection, which is the collector's
  - The approval machinery (`05-governance`)

- **Requirements Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-fr-define-sku` — typing and classification; the identity
    clause is `01-foundation`'
  - [ ] `p1` - `cpt-cf-bss-products-fr-sku-sellable`
  - [ ] `p1` - `cpt-cf-bss-products-fr-plantier-classification`
  - [ ] `p1` - `cpt-cf-bss-products-fr-metering-unit-declaration`
  - [ ] `p1` - `cpt-cf-bss-products-fr-metering-unit-delisting`
  - [ ] `p1` - `cpt-cf-bss-products-fr-accounting-codes`

- **Design Principles Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-principle-registered-validators`

- **Design Constraints Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-constraint-no-commercial-concern`
  - [ ] `p1` - `cpt-cf-bss-products-constraint-broker-native-events`

- **Domain Model Entities**:
  - RecognizedSet (code sets, unit sets and the `PlanTier` taxonomy, discriminated by `set_kind`)
  - MeterDeclaration
  - TypeProfile
  - UsageTypeResolver

- **Design Components**:

  - [ ] `p1` - `cpt-cf-bss-products-component-capability-handlers`

- **API**: None of its own — SKU classification fields are authored through `01-foundation`'s
  create and save doors, and this feature contributes the validators those doors run

- **Sequences**: None — contributes validators to
  `cpt-cf-bss-products-seq-authoring-publish`

- **Data**:
  - `products_recognized_set`

### 2.4 [Lifecycle Policy](features/lifecycle.md) - MEDIUM

- [ ] `p1` - **ID**: `cpt-cf-bss-products-feature-lifecycle`

- **Type**: Capability

- **Phases**: [`DESIGN.md`](./DESIGN.md) §1.3 phase 1/2

- **Purpose**: The policy on every lifecycle edge, over the state-machine core `01-foundation`
  owns. It holds deprecation provenance, the un-deprecation rules, scheduled publish and
  retirement with their activation mechanics, the retirement flip guard against
  `07-reference-signal`'s predicate, `replacedBy`, parent-child publish ordering and the final
  scope-containment rule, cascade-retire with its deferred intent, and the v1 EOL lockout.

- **Depends On**: `cpt-cf-bss-products-feature-foundation`; at integration also
  `cpt-cf-bss-products-feature-governance` and `cpt-cf-bss-products-feature-reference-signal`

- **Scope**:
  - Policy validators on every lifecycle edge
  - Deprecation provenance
  - Un-deprecation rules
  - Scheduled transitions — publish and retirement — and their activation mechanics
  - The retirement flip guard against the reference-signal predicate
  - `replacedBy`
  - Parent-child publish ordering, the final scope-containment rule, cascade-retire and deferred intent
  - The v1 EOL lockout

- **Out of scope**:
  - The edge list itself and terminality (`01-foundation`)
  - The reference predicate (`07-reference-signal`)
  - The approval ceremonies the edges invoke (`05-governance`)
  - Grandfathered-snapshot immutability (`06-catalog-version`)
  - Live-subscription migration, which is Subscriptions'
  - The consumer-side adoption block, verified by `12-consumer-contracts`' seam suite

- **Requirements Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-fr-deprecation` — the deprecation and un-deprecation machine
    and its cascades; the consumer-side adoption block is `12-consumer-contracts`'
  - [ ] `p1` - `cpt-cf-bss-products-fr-undeprecation`
  - [ ] `p1` - `cpt-cf-bss-products-fr-retirement-eol`
  - [ ] `p1` - `cpt-cf-bss-products-fr-lifecycle-transitions` — the scheduling clauses; the
    machine core is `01-foundation`'
  - [ ] `p1` - `cpt-cf-bss-products-fr-parent-child-integrity` — the final containment rule; the
    interim check is `01-foundation`'
  - [ ] `p1` - `cpt-cf-bss-products-usecase-lifecycle-deprecation`

- **Design Principles Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-principle-forward-only`

- **Design Constraints Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-constraint-immutable-identity`

- **Domain Model Entities**:
  - ScheduledTransition
  - ActivationRunner
  - DeferredRetireIntent
  - CascadePlan

- **Design Components**:

  - [ ] `p1` - `cpt-cf-bss-products-component-capability-handlers`

- **API**: None of its own — lifecycle edges are driven through `01-foundation`'s publish and
  transition doors, which run this feature's registered validators

- **Sequences**:

  - [ ] `p1` - `cpt-cf-bss-products-seq-retirement-cascade`

- **Data**:
  - `products_scheduled_transition`
  - `products_deferred_retirement`

### 2.5 [Governance & Approvals](features/governance.md) - MEDIUM

- [ ] `p1` - **ID**: `cpt-cf-bss-products-feature-governance`

- **Type**: Capability

- **Phases**: [`DESIGN.md`](./DESIGN.md) §1.3 phase 1/2

- **Purpose**: Who may change what, and who has to agree. It owns materiality evaluation driven
  off the bucket registry, the enumerated ops and the affected-entity count; the approval
  workflow — submit, quorum, publish or apply — over both entity publishes and governed live
  ops; the stored pinned snapshot and its diff rendering; the override ceremony; approver
  constraints for distinctness, roles and scope; the pending-approvals read surface the studio
  inbox consumes; the RBAC catalog; and break-glass elevation with its audit.

- **Depends On**: `cpt-cf-bss-products-feature-foundation`

- **Scope**:
  - Materiality evaluation: field-set driven off the bucket registry, plus enumerated ops and the affected-entity count
  - The approval workflow — submit, quorum, publish or apply — over entity publishes and governed live ops alike
  - Stored pinned snapshots and diff rendering
  - The override ceremony
  - Approver constraints: distinctness, roles, scope
  - The pending-approvals read surface, which is the studio-inbox contract
  - The RBAC catalog
  - Break-glass elevation and its audit

- **Out of scope**:
  - The doors themselves (`01-foundation`, `02-taxonomy-attributes`)
  - Scheduling (`04-lifecycle` pins approvals; this feature validates them at activation through the gate)
  - The break-glass **correction** door, a distinct feature-flag-gated write mechanism owned by
    `07-reference-signal` that only reuses this feature's elevation ceremony
  - Erasure of approver identities (`10-retention-erasure`)

- **Requirements Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-fr-materiality-gated-publish`
  - [ ] `p1` - `cpt-cf-bss-products-fr-breakglass-action-scope`
  - [ ] `p1` - `cpt-cf-bss-products-fr-tenant-isolation-breakglass`
  - [ ] `p1` - `cpt-cf-bss-products-usecase-approval-publish`

- **Design Principles Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-principle-governance-at-entity-publish`

- **Design Constraints Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-constraint-gts-types-not-instances` — the authz resource and
    action catalog declares GTS-typed resources

- **Domain Model Entities**:
  - ApprovalRecord
  - ApprovalDecision
  - BreakGlassSession
  - MaterialityEvaluator
  - QuorumEvaluator
  - OverrideCeremony

- **Design Components**:

  - [ ] `p1` - `cpt-cf-bss-products-component-capability-handlers`

- **API**:
  - `GET /bss-products/v1/approvals?state=pending` — the pending-approvals read surface

- **Sequences**: None — this feature's gate phase runs inside
  `cpt-cf-bss-products-seq-authoring-publish`

- **Data**:
  - `products_approval`
  - `products_approval_decision`
  - `products_breakglass_session`

### 2.6 [Catalog Version & Freeze](features/catalog-version.md) - MEDIUM

- [ ] `p1` - **ID**: `cpt-cf-bss-products-feature-catalog-version`

- **Type**: Capability

- **Phases**: [`DESIGN.md`](./DESIGN.md) §1.3 phase 2

- **Purpose**: The point-in-time catalog every downstream gear pins to. It owns the increment
  lanes with their coalescing, serialization and gapless ids; the snapshot builder — content
  manifest, canonical serialization, checksum, metadata capture, participant-set snapshot; the
  resolution API with declared intent; the freeze protocol end to end, including acks, timeout,
  recovery, force-completion and participant governance; the grandfathering invariant and the
  per-version freeze-registration records `10-retention-erasure` consumes; `compositionPending`
  clearing; the version diff; the pre-publish lint report door; and the posting-safe
  observability.

- **Depends On**: `cpt-cf-bss-products-feature-foundation`,
  `cpt-cf-bss-products-feature-taxonomy-attributes`,
  `cpt-cf-bss-products-feature-sku-classification`,
  `cpt-cf-bss-products-feature-lifecycle`, `cpt-cf-bss-products-feature-governance`

- **Scope**:
  - Increment lanes, coalescing, serialization, gapless ids
  - The snapshot builder: content manifest, canonical serialization, checksum, metadata capture, participant-set snapshot
  - The resolution API with declared intent
  - The freeze protocol end to end: acks, timeout, recovery, force-completion, participant governance
  - The grandfathering invariant and the per-version freeze-registration records
  - `compositionPending` clearing
  - The version diff
  - The pre-publish `validate(lint)` report door, whose report `09-bulk-promotion` consumes
  - The posting-safe observability

- **Out of scope**:
  - Entity publish and its governance (`01-foundation`, `05-governance`)
  - What participants do with the fan-out, which belongs to their gears
  - Retention and GC execution (`10-retention-erasure`); this feature only supplies the liveness records
  - `pricingSnapshotRef` composition, which is rating's
  - The pricing-side pending-ref table, which pricing owns

- **Requirements Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-fr-catalog-version-publish`
  - [ ] `p1` - `cpt-cf-bss-products-fr-catalog-publish-concurrency`
  - [ ] `p1` - `cpt-cf-bss-products-fr-catalog-version-diff`
  - [ ] `p1` - `cpt-cf-bss-products-fr-snapshot-reproducibility`
  - [ ] `p1` - `cpt-cf-bss-products-fr-freeze-atomicity` — the freeze protocol itself; the
    consumer-observable half is `12-consumer-contracts`'
  - [ ] `p1` - `cpt-cf-bss-products-fr-freeze-recovery`
  - [ ] `p1` - `cpt-cf-bss-products-fr-freeze-participant-governance`
  - [ ] `p1` - `cpt-cf-bss-products-fr-grandfathering-invariant`
  - [ ] `p1` - `cpt-cf-bss-products-fr-grandfathered-retention-coupling` — the liveness source;
    the retention gate is `10-retention-erasure`'
  - [ ] `p1` - `cpt-cf-bss-products-fr-bundle-adoption-guard`
  - [ ] `p1` - `cpt-cf-bss-products-fr-prepublish-lint`
  - [ ] `p1` - `cpt-cf-bss-products-fr-revision-vs-version` — version binding at freeze; the two
    counters and the history are `01-foundation`'
  - [ ] `p1` - `cpt-cf-bss-products-nfr-posting-safe-budget`
  - [ ] `p1` - `cpt-cf-bss-products-nfr-snapshot-archival-dr` — the archival and snapshot
    operand, shared with `10-retention-erasure`, which owns the restore-verification and DR half
  - [ ] `p1` - `cpt-cf-bss-products-nfr-publication-propagation` — the catalog-version half
  - [ ] `p1` - `cpt-cf-bss-products-nfr-scale-extensibility` — the catalog-version growth half
  - [ ] `p1` - `cpt-cf-bss-products-usecase-freeze-monitoring`
  - [ ] `p1` - `cpt-cf-bss-products-contract-increment-request`
  - [ ] `p1` - `cpt-cf-bss-products-contract-freeze-ack`
  - [ ] `p1` - `cpt-cf-bss-products-contract-bundle-composition-signal`

- **Design Principles Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-principle-two-version-counters`

- **Design Constraints Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-constraint-immutable-identity`

- **Domain Model Entities**:
  - CatalogVersion
  - CatalogVersionEntry
  - IncrementRequest
  - FreezeParticipant
  - FreezeAck
  - VersionManifest
  - FreezeLedger
  - SnapshotBuilder
  - IntentfulResolver

- **Design Components**:

  - [ ] `p1` - `cpt-cf-bss-products-component-capability-handlers`

- **API**:
  - `POST /bss-products/v1/catalog-version-requests` — the increment-request contract

- **Sequences**:

  - [ ] `p1` - `cpt-cf-bss-products-seq-catalog-version-freeze`

- **Data**:
  - `products_catalog_version`
  - `products_catalog_version_counter`
  - `products_catalog_version_entry`
  - `products_catalog_version_request`
  - `products_freeze_participant`
  - `products_freeze_ack`

### 2.7 [Reference Signal](features/reference-signal.md) - MEDIUM

- [ ] `p1` - **ID**: `cpt-cf-bss-products-feature-reference-signal`

- **Type**: Capability

- **Phases**: [`DESIGN.md`](./DESIGN.md) §1.3 phase 2

- **Purpose**: Whether anything downstream still points at a SKU, answered honestly enough to
  gate a retirement. It owns the watermark door, its storage and its freshness; the three-state
  predicate and the per-producer detail surface a retirement confirmation shows; producer
  registration and its symmetric snapshot ride; the correction door for immutable fields; the
  break-glass correction; and the fail-safe tripwire.

- **Depends On**: `cpt-cf-bss-products-feature-foundation`,
  `cpt-cf-bss-products-feature-lifecycle`

- **Scope**:
  - The watermark door, its storage and its freshness
  - The three-state predicate and its per-producer detail surface
  - Producer registration and its symmetric snapshot ride
  - The correction door
  - The break-glass correction, which reuses `05-governance`'s elevation ceremony
  - The tripwire

- **Out of scope**:
  - What producers count, which is each producer's own contract
  - Retirement policy (`04-lifecycle`)
  - The ceremony machinery (`05-governance`)
  - Erasure of watermark content (`10-retention-erasure`); the sets carry SKU ids only and no PII by construction

- **Requirements Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-fr-reference-signal`
  - [ ] `p1` - `cpt-cf-bss-products-fr-reference-producer-registration`
  - [ ] `p1` - `cpt-cf-bss-products-fr-immutable-field-correction`
  - [ ] `p1` - `cpt-cf-bss-products-fr-failsafe-tripwire`
  - [ ] `p1` - `cpt-cf-bss-products-contract-sku-reference-count`

- **Design Principles Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-principle-fail-closed`

- **Design Constraints Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-constraint-tenant-isolation`

- **Domain Model Entities**:
  - ReferenceWatermark
  - ReferenceProducer
  - ReferenceMember
  - CorrectionOverride
  - WatermarkDoor
  - ReferencePredicate
  - CorrectionDoor
  - TripwireCounter

- **Design Components**:

  - [ ] `p1` - `cpt-cf-bss-products-component-capability-handlers`

- **API**: None declared in the design set — the watermark and correction doors are
  service-to-service surfaces whose routes the design set does not pin

- **Sequences**:

  - [ ] `p1` - `cpt-cf-bss-products-seq-reference-signal`

- **Data**:
  - `products_reference_watermark`
  - `products_reference_producer`
  - `products_reference_member`
  - `products_correction_override`

### 2.8 [Read Models](features/read-models.md) - MEDIUM

- [ ] `p1` - **ID**: `cpt-cf-bss-products-feature-read-models`

- **Type**: Capability

- **Phases**: [`DESIGN.md`](./DESIGN.md) §1.3 phase 2

- **Purpose**: The cache-first browse and search surface, and the history timeline. It owns the
  event-driven projector over frozen content, the projection schemas, the per-state visibility
  contract, staleness signalling through `asOfCatalogVersion`, scoping enforcement on the read
  path, degradation behavior, facets and filters, and the convergence budget with its
  measurement.

- **Depends On**: `cpt-cf-bss-products-feature-foundation`,
  `cpt-cf-bss-products-feature-catalog-version`

- **Scope**:
  - The projector, event-driven over frozen content
  - The projection schemas
  - Per-state visibility
  - Staleness signalling
  - Scoping enforcement on the read path
  - Degradation behavior
  - Facets and filters
  - The history-timeline projection
  - The convergence budget and its measurement

- **Out of scope**:
  - The write path and its availability (`01-foundation`)
  - Frozen-snapshot resolution (`06-catalog-version`'s resolver, a different surface with different guarantees)
  - The approval queue (`05-governance`)
  - External search infrastructure choices, an implementation detail behind the projection contract

- **Requirements Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-fr-cache-first-browse`
  - [ ] `p1` - `cpt-cf-bss-products-fr-event-delivery-resilience` — the per-consumer delivery
    and dead-letter **projection** clause; durable acceptance is `01-foundation`'
  - [ ] `p1` - `cpt-cf-bss-products-nfr-read-latency`
  - [ ] `p1` - `cpt-cf-bss-products-nfr-read-throughput`
  - [ ] `p1` - `cpt-cf-bss-products-nfr-graceful-degradation`
  - [ ] `p1` - `cpt-cf-bss-products-nfr-availability-audit`
  - [ ] `p1` - `cpt-cf-bss-products-usecase-catalog-browser-history`
  - [ ] `p1` - `cpt-cf-bss-products-interface-read-model`

- **Design Principles Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-principle-publish-through-engine`

- **Design Constraints Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-constraint-tenant-isolation`

- **Domain Model Entities**:
  - ReadEntity (the projection row)
  - ReadDeliveryState
  - ReadDeferredIntent
  - ReadFreezeStatus
  - ReadProjector
  - BrowseProjection
  - StalenessStamp
  - VisibilityFilter

- **Design Components**:

  - [ ] `p1` - `cpt-cf-bss-products-component-capability-handlers`

- **API**:
  - `GET /bss-products/v1/browse` — scoped browse, search and filter
  - `GET /bss-products/v1/{products|skus}/{id}/versions` — version-history retrieval

- **Sequences**: None — the projector consumes the events
  `cpt-cf-bss-products-seq-authoring-publish` emits

- **Data**:
  - `products_read_entity`
  - `products_read_delivery_state`
  - `products_read_deferred_intent`
  - `products_read_freeze_status`

### 2.9 [Bulk & Promotion](features/bulk-promotion.md) - MEDIUM

- [ ] `p1` - **ID**: `cpt-cf-bss-products-feature-bulk-promotion`

- **Type**: Capability

- **Phases**: [`DESIGN.md`](./DESIGN.md) §1.3 phase 2/3

- **Purpose**: Catalog change at volume, and moving a catalog between environments. It owns the
  import pipeline — parse, per-row validate, stage as drafts, aggregated report, batch approval,
  per-row publish — plus export, promotion identity resolution, bulk lifecycle operations,
  batch and row idempotency, the coalesced event, and the wiring that tags catalog-version
  requests with the batch's operation key.

- **Depends On**: `cpt-cf-bss-products-feature-foundation`,
  `cpt-cf-bss-products-feature-governance`

- **Scope**:
  - The import pipeline: parse, per-row validate, stage as drafts, aggregated report, batch approval, per-row publish
  - Export
  - Promotion identity resolution
  - Bulk lifecycle operations
  - Batch and row idempotency
  - The coalesced event
  - The `operation_key` wiring into `06-catalog-version`'s requests

- **Out of scope**:
  - Row-level validation rules — each feature's registered validators; bulk runs the same
    pipeline per row and never a parallel one
  - The approval ceremony (`05-governance`)
  - The catalog-version increment itself (`06-catalog-version`); the bulk window closes on a
    hard timer, not on any close operation this feature issues

- **Requirements Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-fr-bulk-import-export`
  - [ ] `p1` - `cpt-cf-bss-products-usecase-bulk-operations`
  - [ ] `p1` - `cpt-cf-bss-products-usecase-environment-promotion`

- **Design Principles Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-principle-registered-validators`

- **Design Constraints Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-constraint-tenant-isolation`

- **Domain Model Entities**:
  - BulkBatch
  - RowLedger
  - ChangeReport

- **Design Components**:

  - [ ] `p1` - `cpt-cf-bss-products-component-capability-handlers`

- **API**:
  - `POST /bss-products/v1/bulk/imports` — start an import batch
  - `GET /bss-products/v1/bulk/exports?catalogVersionId=` — export at a catalog version
  - `POST /bss-products/v1/bulk/lifecycle` — bulk lifecycle operations

- **Sequences**:

  - [ ] `p1` - `cpt-cf-bss-products-seq-environment-promotion`

- **Data**:
  - `products_bulk_batch`
  - `products_bulk_row`

### 2.10 [Retention & Erasure](features/retention-erasure.md) - LOW

- [ ] `p1` - **ID**: `cpt-cf-bss-products-feature-retention-erasure`

- **Type**: Capability

- **Phases**: [`DESIGN.md`](./DESIGN.md) §1.3 phase 3

- **Purpose**: How long the registry keeps what it keeps, and how an identity leaves it. It owns
  the identity-reference map and the erasure act, the content-PII detector policy and its
  allow-list governance, retention classes with their clocks and the GC, the
  grandfathered-retention gate, the durability mechanics, and the compliance-export surface.

- **Depends On**: `cpt-cf-bss-products-feature-foundation`,
  `cpt-cf-bss-products-feature-catalog-version`

- **Scope**:
  - The identity-reference map and the erasure act
  - The content-PII detector policy and allow-list governance
  - Retention classes, clocks and the GC
  - The grandfathered-retention gate
  - The durability mechanics: checksum restore-verification cadence, DR posture as config plus probes
  - The compliance-export surface

- **Out of scope**:
  - The write-block **hook placement** (`02-taxonomy-attributes`)
  - The liveness records themselves (`06-catalog-version`)
  - Audit-row **editing** — every feature writes its own and this one never edits them
  - Break-glass reads (`05-governance`)

- **Requirements Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-fr-retention-erasure` — the clocks, the erasure act and the
    retention gate; the content write-block hook is `02-taxonomy-attributes`'
  - [ ] `p1` - `cpt-cf-bss-products-fr-grandfathered-retention-coupling` — the retention gate;
    the liveness source is `06-catalog-version`'
  - [ ] `p1` - `cpt-cf-bss-products-fr-expected-failure-behavior` — the retention-orphan row; the
    taxonomy's home is `01-foundation`'
  - [ ] `p1` - `cpt-cf-bss-products-nfr-snapshot-archival-dr` — the restore-verification and DR
    half, shared with `06-catalog-version`, which owns the archival and snapshot operand

- **Design Principles Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-principle-fail-closed`

- **Design Constraints Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-constraint-tenant-isolation`

- **Domain Model Entities**:
  - IdentityRefMap
  - PiiDetector
  - RetentionClock
  - RetentionGate

- **Design Components**:

  - [ ] `p1` - `cpt-cf-bss-products-component-capability-handlers`

- **API**:
  - `POST /bss-products/v1/erasure-requests` — the erasure act
  - `GET /bss-products/v1/compliance/identity-export` — the compliance-export surface

- **Sequences**: None — erasure and GC are background acts over the tables the other features write

- **Data**:
  - `products_identity_ref`
  - `products_pii_allowlist`

### 2.11 [Clone](features/clone.md) - LOW

- [ ] `p2` - **ID**: `cpt-cf-bss-products-feature-clone`

- **Type**: Capability

- **Phases**: [`DESIGN.md`](./DESIGN.md) §1.3 phase 3

- **Purpose**: Creating a new catalog entity from an existing one without copying what must not
  be copied. It owns the clone door for a Product and for a SKU, single or product-with-SKUs;
  the disposition table saying which fields carry, which reset and which are refused; live
  re-validation of the result against every current rule; the `clonedFrom` lineage; and the
  revival-rename rule.

- **Depends On**: `cpt-cf-bss-products-feature-foundation`,
  `cpt-cf-bss-products-feature-taxonomy-attributes`,
  `cpt-cf-bss-products-feature-sku-classification`,
  `cpt-cf-bss-products-feature-lifecycle`

- **Scope**:
  - The clone door for Product and SKU, single and product-with-SKUs
  - The disposition table
  - Live re-validation
  - `clonedFrom`
  - The revival-rename rule

- **Out of scope**:
  - Bulk cloning — `09-bulk-promotion`'s resolver produces no copies
  - Pricing and plan content, which is never copied
  - Approval: a clone lands as `draft` and its publish is the ordinary `05-governance`-gated act

- **Requirements Covered**:

  - [ ] `p3` - `cpt-cf-bss-products-fr-clone`

- **Design Principles Covered**:

  - [ ] `p2` - `cpt-cf-bss-products-principle-registered-validators`

- **Design Constraints Covered**:

  - [ ] `p2` - `cpt-cf-bss-products-constraint-immutable-identity`
  - [ ] `p2` - `cpt-cf-bss-products-constraint-tenant-isolation`
  - [ ] `p2` - `cpt-cf-bss-products-constraint-no-commercial-concern`

- **Domain Model Entities**:
  - DispositionTable
  - CloneDoor

  Neither is an aggregate of its own: a clone produces a `Product` or `SKU` row owned by
  `01-foundation`, distinguished only by its `cloned_from` column. Both are
  `11-clone` §1.7's design-introduced names and are listed here on the same rule as
  §2.7's and §2.10's.

- **Design Components**:

  - [ ] `p2` - `cpt-cf-bss-products-component-capability-handlers`

- **API**:
  - `POST /bss-products/v1/{products|skus}/{id}/clone` — the clone door

- **Sequences**: None — a clone lands as a draft and joins
  `cpt-cf-bss-products-seq-authoring-publish` from there

- **Data**: None — the clone writes `products_product` and `products_sku`, both owned by
  `01-foundation`; `cloned_from` is a column on those tables and not a table of its own

### 2.12 [Consumer Contracts](features/consumer-contracts.md) - MEDIUM

- [ ] `p1` - **ID**: `cpt-cf-bss-products-feature-consumer-contracts`

- **Type**: Contract

- **Phases**: [`DESIGN.md`](./DESIGN.md) §1.3 phase 2/3

- **Purpose**: What a downstream gear may rely on, and how it recovers when it falls behind. It
  owns event schema versioning, the replay and bootstrap contract, and the SDK and §9 surfaces
  including the studio-inbox envelope cross-check. Its slice also specifies a verification track
  — the seam suite, the consumer-obligation register, the completeness checks and §17.2
  traceability — which is CI and review work rather than gear behavior and is therefore not
  decomposed into this feature.

- **Depends On**: `cpt-cf-bss-products-feature-foundation`,
  `cpt-cf-bss-products-feature-sku-classification`,
  `cpt-cf-bss-products-feature-catalog-version`,
  `cpt-cf-bss-products-feature-reference-signal`

- **Scope**:
  - Event schema versioning
  - The replay and bootstrap contract
  - The SDK and §9 surfaces, including the studio-inbox envelope cross-check

- **Out of scope**:
  - The seam-suite specification, the consumer-obligation register, the completeness checks and
    §17.2 traceability — the verification track this document does not decompose
  - The counterparts' implementations; each obligation names its owing gear
  - The broker's transport, which is Common Core's
  - The cross-gear open questions this feature can only assert once they are closed

- **Requirements Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-fr-event-versioning-replay`
  - [ ] `p1` - `cpt-cf-bss-products-fr-plan-price-seam`
  - [ ] `p1` - `cpt-cf-bss-products-fr-monetization-traceability`
  - [ ] `p1` - `cpt-cf-bss-products-fr-deprecation` — the consumer-side adoption block only; the
    policy is `04-lifecycle`'
  - [ ] `p1` - `cpt-cf-bss-products-fr-freeze-atomicity` — the consumer-observable half; the
    protocol is `06-catalog-version`'
  - [ ] `p1` - `cpt-cf-bss-products-nfr-backward-compatible-evolution`

- **Design Principles Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-principle-forward-only`

- **Design Constraints Covered**:

  - [ ] `p1` - `cpt-cf-bss-products-constraint-broker-native-events`

- **Domain Model Entities**:
  - SeamSuite
  - SchemaPin
  - ObligationRegister
  - CoverageChecks

- **Design Components**:

  - [ ] `p1` - `cpt-cf-bss-products-component-consumer-contracts`

- **API**: None of its own — this feature versions and documents the surfaces
  `01-foundation`, `06-catalog-version` and `08-read-models` expose, and adds the SDK client over them

- **Sequences**: None — the seam is asserted over the sequences the other features own

- **Data**: None — the schema pin is a committed CI artifact, not a gear table

---

## 3. Feature Dependencies

```text
cpt-cf-bss-products-feature-foundation
    |
    +-- cpt-cf-bss-products-feature-taxonomy-attributes
    |       |
    |       +-- cpt-cf-bss-products-feature-sku-classification
    |
    +-- cpt-cf-bss-products-feature-lifecycle
    |       |
    |       +-- cpt-cf-bss-products-feature-reference-signal
    |
    +-- cpt-cf-bss-products-feature-governance
    |       |
    |       +-- cpt-cf-bss-products-feature-bulk-promotion
    |
    +-- cpt-cf-bss-products-feature-catalog-version
            |
            +-- cpt-cf-bss-products-feature-read-models
            +-- cpt-cf-bss-products-feature-retention-erasure

The tree above shows one parent per node. The relation is a DAG with fan-in, and these
edges exist but cannot be drawn in it:

cpt-cf-bss-products-feature-catalog-version
    also depends on taxonomy-attributes, sku-classification, lifecycle, governance

cpt-cf-bss-products-feature-lifecycle
    at integration also depends on governance, reference-signal

cpt-cf-bss-products-feature-clone
    depends on foundation, taxonomy-attributes, sku-classification, lifecycle

cpt-cf-bss-products-feature-consumer-contracts
    depends on foundation, sku-classification, catalog-version, reference-signal
```

**Dependency Rationale**:

- Every feature depends on `cpt-cf-bss-products-feature-foundation`: it owns the entity model,
  the write doors, the validation pipeline every capability registers rules into, the publish
  path, and the event and audit planes. No capability feature has a write surface of its own.
- `cpt-cf-bss-products-feature-sku-classification` depends on
  `cpt-cf-bss-products-feature-taxonomy-attributes`: SKU typing and classification read the
  attribute definitions and the category tree that feature owns.
- `cpt-cf-bss-products-feature-catalog-version` depends on `taxonomy-attributes`,
  `sku-classification`, `lifecycle` and `governance` together: a catalog version freezes the
  content those four define, and its freeze protocol consumes the governance gate.
- `cpt-cf-bss-products-feature-reference-signal` depends on
  `cpt-cf-bss-products-feature-lifecycle`: the reference watermarks and the three-state predicate
  are read against an entity's lifecycle state, and the de-listing rules are that feature's.
- `cpt-cf-bss-products-feature-read-models` depends on
  `cpt-cf-bss-products-feature-catalog-version`: the `asOfCatalogVersion` staleness signal and
  the per-state visibility contract are defined over the versions that feature produces.
- `cpt-cf-bss-products-feature-retention-erasure` depends on
  `cpt-cf-bss-products-feature-catalog-version`: retention is coupled to grandfathering, which is
  expressed over frozen catalog versions.
- `cpt-cf-bss-products-feature-bulk-promotion` depends on
  `cpt-cf-bss-products-feature-governance`: a bulk batch's reason and its approval record live on
  that feature's `ApprovalRecord`.
- `cpt-cf-bss-products-feature-clone` depends on `foundation`, `taxonomy-attributes`,
  `sku-classification` and `lifecycle`: a clone re-validates the cloned entity live against all
  four rule sets, and `cloned_from` is a foundation-guarded column.
- `cpt-cf-bss-products-feature-consumer-contracts` depends on `foundation`,
  `sku-classification`, `catalog-version` and `reference-signal`: the schema pin's membership is
  derived from the catalog fields those features define, and the replay and bootstrap contract is
  expressed over foundation's event envelope and catalog-version's snapshots.
- `cpt-cf-bss-products-feature-lifecycle` requires `cpt-cf-bss-products-feature-governance` and
  `cpt-cf-bss-products-feature-reference-signal` **at integration only**: governance because the
  lifecycle edges invoke approval ceremonies that are validated at activation through the gate,
  and reference-signal because of the retirement flip guard, which is evaluated against that
  feature's predicate.
- **`lifecycle` and `reference-signal` are mutually dependent by declaration, and the cycle is
  broken by phase.** `reference-signal` needs `lifecycle`'s state model from its first commit;
  `lifecycle` needs `reference-signal` only for the retirement flip guard, which closes at
  integration once that feature lands. So `lifecycle`'s edge policy, deprecation provenance and
  scheduling build against `foundation` alone in phase 1/2, and the guard is wired in phase 2. The
  build order is a DAG; the declared dependency relation, read without the phase qualifier, is not.
- `cpt-cf-bss-products-feature-taxonomy-attributes`, `cpt-cf-bss-products-feature-lifecycle` and
  `cpt-cf-bss-products-feature-governance` are independent of each other and can be developed in
  parallel once foundation is in place. ([`DESIGN.md`](./DESIGN.md) §1.3's "Dependency order"
  prose also lists `03` in this parallel set; the §1.3 table's `Depends on` column, which puts
  `03` on "01, 02", is taken as authoritative here, and the prose is the site owing a fix.)
