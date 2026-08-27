# DECISIONS — Product & SKU Registry (`products` gear)

Decision register for the products gear. Prefix **P-D-NN**. A decision lands here with its
date, rationale, and the propagation list of every document that restates it; the §15 rows of
[`PRD.md`](./PRD.md) that answered a gate cite these numbers rather than carrying the only copy.
Historical context: D-46 (`sellable`) and D-47 (`CatalogVersion` increment taxonomy) predate this
register and live in the **pricing** register (`gears/bss/pricing/docs/DECISIONS.md`) — they are
joint contracts, cited from here by their pricing numbers, never duplicated.

<!-- toc -->

- [P-D-01 — Broker-native event envelope (not CloudEvents 1.0)](#p-d-01--broker-native-event-envelope-not-cloudevents-10)
- [P-D-02 — CatalogVersion increments are mechanical; governance at entity publish](#p-d-02--catalogversion-increments-are-mechanical-governance-at-entity-publish)
- [P-D-03 — SkuReferenceCount v1 producer set = {pricing}](#p-d-03--skureferencecount-v1-producer-set--pricing)
- [P-D-04 — Absolute product-name uniqueness (region-independent)](#p-d-04--absolute-product-name-uniqueness-region-independent)
- [P-D-05 — `usageTypeRef` validates resolvability only; UC3(c) lives at the pricing meter binding](#p-d-05--usagetyperef-validates-resolvability-only-uc3c-lives-at-the-pricing-meter-binding)
- [P-D-06 — Metadata map lives outside frozen version content](#p-d-06--metadata-map-lives-outside-frozen-version-content)
- [P-D-07 — The staleness stamp is a floor, advanced only when its content is present](#p-d-07--the-staleness-stamp-is-a-floor-advanced-only-when-its-content-is-present)
- [P-D-08 — Audit sealing is a platform capability: reserved seam + stated requirements](#p-d-08--audit-sealing-is-a-platform-capability-reserved-seam--stated-requirements)
- [P-D-09 — Stage-vs-commit fail-closed is delivered per lane; the requirement says so](#p-d-09--stage-vs-commit-fail-closed-is-delivered-per-lane-the-requirement-says-so)
- [P-D-10 — No gear-side Legal role: the allow-list records Legal's decision, it does not enact it](#p-d-10--no-gear-side-legal-role-the-allow-list-records-legals-decision-it-does-not-enact-it)
- [P-D-11 — The approver count is a policy value with floor 0; the predicates are not](#p-d-11--the-approver-count-is-a-policy-value-with-floor-0-the-predicates-are-not)
- [P-D-12 — The `SchemaPin`'s membership is a rule, not a list](#p-d-12--the-schemapins-membership-is-a-rule-not-a-list)
- [P-D-13 — The quorum shorthand's reach is enumerated; a floor only where the principal is not the tenant's](#p-d-13--the-quorum-shorthands-reach-is-enumerated-a-floor-only-where-the-principal-is-not-the-tenants)
- [P-D-14 — `system_signal` is an approval subject kind, not an exemption; the authorizing principal is the signal](#p-d-14--system_signal-is-an-approval-subject-kind-not-an-exemption-the-authorizing-principal-is-the-signal)
- [P-D-15 — The two inbound machine contracts are `products-sdk` clients from `ClientHub`, not out-of-process REST doors](#p-d-15--the-two-inbound-machine-contracts-are-products-sdk-clients-from-clienthub-not-out-of-process-rest-doors)
- [P-D-16 — A third correction-admission arm: an unresolvable meter target](#p-d-16--a-third-correction-admission-arm-an-unresolvable-meter-target)
- [P-D-17 — Promotion identity collision with different content is update-as-draft, not a per-row conflict](#p-d-17--promotion-identity-collision-with-different-content-is-update-as-draft-not-a-per-row-conflict)
- [P-D-18 — Version liveness ends by an explicit release; the release is a fifth inbound contract](#p-d-18--version-liveness-ends-by-an-explicit-release-the-release-is-a-fifth-inbound-contract)
- [P-D-19 — A force-completed version stays refused for posted use until opt-in; the pin is the registry's own door](#p-d-19--a-force-completed-version-stays-refused-for-posted-use-until-opt-in-the-pin-is-the-registrys-own-door)
- [P-D-20 — A publish during the retirement lead window re-announces `SkuRetired`; the door stays open](#p-d-20--a-publish-during-the-retirement-lead-window-re-announces-skuretired-the-door-stays-open)
- [P-D-21 — The local audit table holds only what emits no event; the event stream is the success-path record](#p-d-21--the-local-audit-table-holds-only-what-emits-no-event-the-event-stream-is-the-success-path-record)
- [P-D-22 — The registry uses the toolkit's transactional outbox, not a gear-local one](#p-d-22--the-registry-uses-the-toolkits-transactional-outbox-not-a-gear-local-one)
- [P-D-23 — The 2026-08-27 slice-01 owner round: eighteen calls on standing open items](#p-d-23--the-2026-08-27-slice-01-owner-round-eighteen-calls-on-standing-open-items)

<!-- /toc -->

## Decision register

### Entries

*Entries stay at `####` deliberately. `spec-check`'s propagation parser recognises a decision
only as `#### <id> …`; promoting them to `###` to satisfy MD001's heading-increment rule —
CodeRabbit's suggestion of 2026-08-26 — would make this register parse as zero decisions, which
is a regression this gear has already paid for once. This intermediate heading satisfies MD001
instead.*

#### P-D-01 — Broker-native event envelope (not CloudEvents 1.0)

- **Date**: 2026-08-25 (product call; PRD §15 gate "Event-envelope conformance")
- **Decision**: the registry publishes its events in the platform event-broker's **broker-native
  schema** (`gears/system/event-broker`, its ADR-0003) — **not** CloudEvents 1.0. The semantic
  obligations are envelope-agnostic and bind unchanged: versioned (semver) schema references
  (the broker-native equivalent of `dataschema`), `vN`→`vN+1` consumer compatibility,
  correlation/causation, per-aggregate ordering keys `(tenant, aggregate)`, pseudonymous actors.
- **Why**: the manifest §7.2 CloudEvents mandate predates the built broker, whose ADR-0003
  explicitly rejects CloudEvents wire conformance (no `dataschema` field). A mapping layer would
  be a second envelope to keep honest with zero consumers asking for it.
- **Residue owed**: manifest §7.2 amendment re-scoping the CloudEvents mandate (owner:
  Architecture / Common Core).
- **Propagated**: PRD §2, §4.1, §5.1, `fr-registry-eventing-audit`,
  `fr-event-versioning-replay`, §9.2, AC #28/#29; S1 §4 (event fan-out); `DESIGN.md` §1.2 Key decisions.

#### P-D-02 — CatalogVersion increments are mechanical; governance at entity publish

- **Date**: 2026-08-25 (product call; PRD §15 gate "D-47 demand-driven increments vs publish governance")
- **Decision**: a `CatalogVersion` increment — operator- or system-initiated (pricing D-47
  lanes: interactive ≤ 5s coalescing, bulk ≤ 5 min) — is **mechanical**: it snapshots only
  content whose governance already happened, and is never itself an approval gate. Every
  governance gate attaches to the **entity publish** that introduces the exception; specifically
  the uncomposed-`bundle` two-person override moves from `CatalogVersion` publish to the
  bundle's entity publish (the lint findings presented to its approvers). The
  `CatalogVersion`-publish lint is an informational report for operator publishes.
- **Why**: the same FR carried both "manual two-person publish with blocking-with-override" and
  D-47's "increment within 5 seconds of a downstream publish request" — a machine cannot wait
  for two humans, skipping them breaks governance, and refusing breaks the ratified SLO. Moving
  the override to the human act dissolves the contradiction without weakening either control.
- **Propagated**: PRD §5.1/§5.2, `fr-define-sku`, `fr-catalog-version-publish`,
  `fr-bundle-adoption-guard`, `fr-prepublish-lint`, AC #7/#19/#25/#45; S1 §1.2, S3 (`inst-cl-bundle-override`), S5, S6, S9 (`inst-bk-override`) — **§4.1 struck**: its bullets say nothing about mechanical increments or entity-publish governance (item 31 of the 2026-08-26 review); `DESIGN.md` §1.2 Key decisions.

#### P-D-03 — SkuReferenceCount v1 producer set = {pricing}

- **Date**: 2026-08-25 (product call; PRD §15 gate "SkuReferenceCount owner + delivery date")
- **Decision**: the v1 **registered producer set** of the `SkuReferenceCount` per-producer
  watermark is **{plan-price (pricing gear)}**, built jointly with this gear's development and
  delivered before products v1 GA. Subscriptions and Contracts register as producers at their
  own build time; their GA is gated on producing the signal. Until the pricing watermark ships,
  AC #2/#4/#18 run fail-safe (break-glass + tripwire).
- **Why**: pricing is the only coded counterpart and already holds the data (live plan→SKU
  references); `fr-reference-producer-registration` makes late onboarding safe by construction
  (unregistered silence pins nothing, registration never re-flips history). One-party
  commitment instead of a three-party negotiation with two docs-only gears.
- **Propagated**: PRD §9.2, §14, `fr-reference-producer-registration`, AC #43; pricing PRD §15
  (mirrored answered row); the rating gear's ownership matrix in
  ../../rating/docs/SEAMS.md; S7; `DESIGN.md` §1.2 Key decisions.

#### P-D-04 — Absolute product-name uniqueness (region-independent)

- **Date**: 2026-08-25 (product call; PRD §15 gate "Region-set algebra")
- **Decision**: product-name uniqueness on `(tenantId, brandId, normalized(name))` is
  **absolute** — two same-named Products under one tenant+brand are forbidden regardless of
  region scope. The canonical internal name is a quasi-code; localized display names are
  attributes and repeat freely (regional variants: distinct internal names, identical display
  names). Region-set semantics survive only for **parent-child scope containment**
  (`fr-parent-child-integrity`) — pinned in slice 01/04 Design, interim conservative
  subset-check fail-closed.
- **Why**: the overlap/disjointness algebra was a pre-approval gate with real false-reject and
  false-allow risk; absolute uniqueness deletes the whole question from the create door.
  Strict→loose later is a compatible widening; loose→strict would be a breaking migration.
- **Propagated**: PRD glossary (Region), `fr-create-product`, `fr-expected-failure-behavior`,
  AC #5, AC #33a (the promotion fallback identity), AC #38, §16; S1 (uniqueness index + `normalized(name)` pin), S2 (display-name coexistence), S4 (containment), S9 (C5 promotion identity); `DESIGN.md` §1.2 Key decisions.

#### P-D-05 — `usageTypeRef` validates resolvability only; UC3(c) lives at the pricing meter binding

- **Date**: 2026-08-25 (veto round over the UC3 adoption block of 2026-07-28)
- **Decision**: registry publish validates that a declared `usageTypeRef` **resolves** in the
  usage-collector's platform-global UsageType catalog — nothing more. "And is active" is dropped
  (a UsageType carries no lifecycle state: register/get/list/delete only, deletion FK-guarded
  against usage records — not against catalog meters, which rating's quarantine rule
  fail-safes). The UC3(c) dimension-set cross-validation is **not** performed here (the registry
  assigns dimension sets to plan-price and holds no operand): its home is pricing
  `inst-cmp-usagetype` (confirmed 2026-07-31 — **specified, not built**, corrected 2026-08-26:
  pricing's `design/01-foundation.md` calls the binding "registry-dependent and deferred",
  `design/03-price-structure.md` says "neither is enforced today and both codes are emitted
  nowhere", and `pricing/src/domain/plan_rules.rs` lists `METER_USAGE_TYPE_UNBOUND` and
  `METER_DIMENSION_UNDECLARED` under *What is deliberately absent*. **So no gate enforces
  UC3(c) today, on either side** — the registry validates resolvability only, and pricing's
  rule is authored and unbuilt. The invariant has a home and no enforcer; whoever builds
  pricing's registry client owes it) — priced `dimensionKey` **⊆** the
  UsageType's `metadata_fields` at plan publish.
- **Why**: both original clauses named operands that do not exist registry-side; subset (not
  equality) is the load-bearing invariant — pricing fewer dimensions than the source emits is
  harmless, pricing one it never emits is the hazard.
- **Propagated**: PRD `fr-metering-unit-declaration`, AC #8, §15 (answered row); rating SEAMS
  UC3 row + ownership matrix; pricing design/02 (stale "registry holds equality" premise
  retired); S3; `DESIGN.md` §1.2 Key decisions.
- **Residue (2026-08-25, PR #14 review)**: quarantine-on-deleted-UsageType is a fail-safe, not
  an operating mode — the deletion-guard/deletion-signal negotiation with usage-collector is a
  PRD §15 open ("UsageType deletion vs published declarations").

#### P-D-06 — Metadata map lives outside frozen version content

- **Date**: 2026-08-25 (introduced by design slice 02 — first slice-born decision, not a §15
  gate answer); **CONFIRMED by the product owner 2026-08-26**, flag struck
- **Decision**: the per-entity metadata map is stored **beside** the entity, outside the frozen
  `products_entity_version` content: mutable in place on any non-terminal entity **without a
  published-version bump** (audited, `MetadataUpdated`-evented per write); a `CatalogVersion`
  captures the map **as of its own snapshot instant**, and that copy freezes with the snapshot.
- **Why**: the PRD demands both "ungoverned free-form channel" and "captured in CatalogVersion
  snapshots" — putting the map inside version content would force a governed version bump per
  sync-marker write (contradiction one) or mutate frozen versions (contradiction two). Beside
  the entity, old snapshots stay byte-identical while the map moves freely.
- **Accepted cost (recorded at confirmation, 2026-08-26)**: the map carries **no history
  between snapshots**. `products_metadata`'s key is `(tenant_id, entity_kind, entity_id, key)`
  — no version dimension, so versioning it is not merely unimplemented but structurally absent.
  A value overwritten before the next `CatalogVersion` survives only as the audit row recording
  the write. That is the ungoverned channel's nature; a key that needs version history is an
  **Attribute**, which has governance, versioning and localization.
- **Propagated**: S2 §2 (`inst-md-placement`) + §4.1/§5 + §6 (flag struck); slice
  S6 (the snapshot-capture step it owes, and where it cites this decision), S1 §4, S5 §3.2 and `design/README.md` (the three further documents that restate it — added 2026-08-26 after a census of the class rather than of the one site lint 5 named); `DESIGN.md` slice row + status line.
- **No PRD edit owed**: the PRD glossary's "Metadata map" row is compatible as written. *(This
  sentence sat inside the `**Propagated**` field, where a propagation check reads "PRD" as a
  claimed target and then fails to find the citation — a claim of the exact opposite of what the
  sentence says. 2026-08-26 branch review.)*

#### P-D-07 — The staleness stamp is a floor, advanced only when its content is present

- **Date**: 2026-08-26 (design slice 08; premise corrected by its own review, H1/H2);
  **CONFIRMED by the product owner 2026-08-26 — conditionally**, flag struck (see Scope below)
- **Decision**: `asOfCatalogVersion` on every read response is a **floor**: every catalog
  version ≤ the stamp is fully reflected in the projection, and later entity events may add,
  change, or **remove** content relative to the stamped version (`projectedAt` is the
  fine-grained coordinate; `null` + `projectedAt` before a tenant's first version). The stamp
  advances only after the `CatalogVersionPublished` changed-entity list is projected from
  frozen rows in the same step — it never claims a version whose content the projection lacks.
- **Why**: the PRD's one-signal rule binds staleness to resolvable catalog versions, but entity
  events between versions are not additive (a retirement flip removes content without an
  increment) — an "as of CV N" claim over mutated content would lie in the dangerous direction.
- **Scope of the confirmation (2026-08-26)**: the floor is correct **given a separate serving
  store**. It is a property of a projection that lags, so it has no subject without one — and
  whether browse needs one at all is now an open PRD §15 question for the NFR workshop ("Does
  browse need a separate serving store at all?"). If that answers *no*, this decision is
  **deleted, not amended**: there is nothing to stamp. Recorded rather than left implicit so a
  later reader does not treat the floor as load-bearing independently of the projection.
- **Secondary, deliberately not decided**: the stamp's audience is a **UI** — `fr-cache-first-browse`
  names one actor, `Presentation / Portals`, and slice 08's actor list is entirely human-facing,
  while the machine-correctness surface is a different door with different guarantees (06's
  `IntentfulResolver`: `intent` mandatory, byte-identical re-resolution, verifiable checksum, no
  staleness by construction). A human asks "how fresh is this", which `projectedAt` answers;
  `asOfCatalogVersion` answers the machine question "which numbered snapshot do I fully
  contain", and NFR #7 nonetheless names *it* as the signal. `asOfCatalogVersion` currently has
  **no reader anywhere** — no slice-12 consumer obligation mentions staleness, no other gear's
  documents name it. Left as the PRD has it: changing which field is primary would edit NFR #7
  and two ACs for a field nobody reads yet, and pricing has already shipped the same two-field
  shape (`pricing_read_model` carries both `projected_at` and `catalog_version`, asserted in
  its tests). Revisit with the first real portal consumer.
- **Propagated**: S8 (`inst-rp-stamp`, §6 — flag struck); `DESIGN.md` slice row +
  status line; PRD `fr-cache-first-browse` rationale re-derived + §15 serving-store row; NFR #7
  unchanged.

#### P-D-08 — Audit sealing is a platform capability: reserved seam + stated requirements

- **Amended 2026-08-27 by P-D-21**: the v1 audit table it describes no longer holds a row per
  mutating door — only refusals and reads under elevation. The reserved seam and S1–S9 stand
  unchanged; what shrank is the table they guard.

- **Date**: 2026-08-26 (product call, prompted by the audit-posture comparison against the
  ledger and pricing registers — this gear had inherited pricing's G1/G2/G5 and silently
  dropped its G4)
- **Decision**: the registry does **not** build a gear-local tamper-evidence chain. Pricing's
  **G4 / D-14** construction — hash-chained audit rows in the gear's own database, adopted there
  as "ledger precedent" — is **deliberately not adopted here**. v1 ships the ordinary
  append-only `products_audit_log` (complete: `actor_ref`, action, subject `(kind, id,
  revision)`, reason, correlation id; exactly one row per mutating door inside that door's
  transaction, rejections included; physically guarded per 01 C5) **over a reserved sealing
  seam** — the columns a future **platform** audit capability needs, present from the first
  migration and never written by this gear, plus one **stub marker** (`seal_state`) recording
  per row that no seal was applied. Activation is then an activation, not a redesign. The
  requirements that capability must satisfy are **S1–S9** below; the obligation itself is a PRD
  §15 open owned by Architecture.
- **Why**: tamper-evident audit is cross-cutting. Per gear it multiplies the chain
  construction, the verification job, the WORM anchoring and the key handling by the number of
  gears, and still leaves an auditor N unrelated chains to walk for one operator action that
  crossed three of them. The ledger built its own because posted financial facts *are* its
  subject matter; pricing took the pattern before any platform owner existed. The registry is
  where the replication stops.
- **What activation costs, corrected 2026-08-26**: "zero-migration" holds only because the
  audit-table trigger whitelist admits a **one-way `unsealed → sealed` UPDATE** supplying the
  hash columns in the same statement (01 §4.4). The seam as first written put `seal_state`
  outside the whitelist entirely *and* required `row_hash` NOT NULL on `sealed`, so an
  asynchronous sealer could neither update an existing row nor insert one already sealed — it
  required exactly the migration the seam exists to avoid (item 7 of the 2026-08-26 review).
- **Consequence, recorded rather than hidden**: rows written before activation are **never
  retroactively provable** — hashing a stored row proves only that it hashes to what it now
  contains. The seam therefore buys exactly two things: zero-migration activation, and an
  **era marker queryable in the data** instead of inferred from a deployment date. Until
  activation, audit immutability rests on `REVOKE` + the trigger whitelist and on nothing
  cryptographic — and slice 10 C1's erasure-completeness argument plus NFR
  `…-nfr-availability-audit`'s "100% write-path audit" lean on precisely that. Audit
  *completeness* is delivered in v1; audit *tamper-evidence* is not.
- **Sealing requirements on the platform capability (normative; cited, never restated)**:
  - **S1 — Construction.** `row_hash = H(domain_sep ‖ canonical row fields ‖ prev_hash)`:
    byte-reproducible across engines and releases, NULL-safe, over a **pinned** field list with
    a golden vector committed at activation (01 C1's schema-oracle practice).
  - **S2 — Segmentation.** Chains segmented per `(tenant_id, chain_id)` where `chain_id` is the
    audited subject's aggregate, with a periodic per-tenant **roll-up** row chaining the segment
    heads. A single chain per tenant is **non-compliant**: writing row *N* needs row *N−1*'s
    hash, so one chain serializes every audited mutation of that tenant inside its own mutation
    transaction (pricing learned this as D-135, after shipping D-14).
  - **S3 — Never on the mutation path.** The audit *record* stays local and commits inside the
    guarded mutation's transaction, as v1 already does; only the *seal* is platform-side and
    asynchronous, computed over rows already immutable. A capability reachable only across the
    network must never be a precondition for a write — an audit store that can be unavailable
    independently of the database is fail-open by construction, which is the whole reason the
    ledger and pricing put the row in the mutation's ACID transaction.
  - **S4 — Verification cadence.** Incremental link check at seal time **plus** a periodic full
    re-walk of every segment and the roll-up **plus** on-read spot checks. Sampling alone is
    non-compliant as the sole production mechanism (ledger G2, normative there).
  - **S5 — Anchoring.** The roll-up head anchored to WORM / object-lock storage **outside** the
    audited gear's own database, so one compromised database cannot rewrite both the rows and
    the evidence about them.
  - **S6 — Residency.** Per-tenant chains only, never cross-tenant: residency-bound tenants
    live on different cells' databases, so a cross-tenant chain is physically impossible and a
    cross-tenant anchor would breach residency.
  - **S7 — Erasure compatibility.** The canonical field list **MUST exclude every field the
    erasure path mutates**. The registry's erasure is a pseudonym-map update precisely so no
    record is ever rewritten (slice 10 C1); a seal covering a resolvable identity would make
    GDPR erasure break the chain. Audit rows carry `actor_ref`, never names — the seal covers
    the ref.
  - **S8 — Retention.** Seal and anchors retained **≥** the audited rows' own retention (slice
    10 C3: statutory maximum for the audit class). A seal expiring before its rows proves
    nothing about the tail.
  - **S9 — Coverage.** The seal MUST cover **every** mutating door of the participating gear,
    **rejections included**. A seal over part of a trail is evidence about that part only; the
    registry writes rejection rows (01 §4.4) specifically so the part is the whole.
- **Propagated**: S1 §4.4 (the reserved seam + its CHECK); S5 §1.6 C7 (the
  G4-shaped constraint row, deferral stated); S10 §1.6 C1/C3 (sealing requirements seven
  and eight — the exclusion and the retention, spelled in words because the short label collides
  with the slice shorthand and was read as a claim into two slices that know nothing of this
  decision; 2026-08-26 branch review);
  PRD §15 open row (owner: Architecture / Common Core) + §16 risk row; PRD NFR
  `…-nfr-availability-audit` reads unchanged — it requires audit *completeness*, which v1
  delivers; `DESIGN.md` §1.2 Key decisions.

#### P-D-09 — Stage-vs-commit fail-closed is delivered per lane; the requirement says so

- **Date**: 2026-08-26 (product call, answering the flagged M2 reading in design slice 06)
- **Decision**: `fr-catalog-publish-concurrency` and **AC #40** are **amended** to state the
  lane split explicitly instead of leaving it as a design reading. Fail-closed re-validation
  is one mechanism with two dispositions: the **operator lane** rejects and names the changed
  entity (`STAGED_ENTITY_CHANGED`); the **mechanical lane** re-collects fresh content and
  retries within its lane SLO and **must not lose the request**. In neither lane may stale
  content be frozen. The FR's actor list gains
  `cpt-cf-bss-products-actor-plan-price` alongside the catalog admin.
- **Why**: the requirement's remedy — "rejected, naming the changed entity" — presumes an
  addressee. It was written when a `CatalogVersion` publish was an operator act, and D-47 +
  P-D-02 then made increments demand-driven and mechanical ("never waits on a human", slice 06
  §1.2), so the dominant caller is pricing over the increment-request contract and there is no
  operator to reject to. Rejecting a machine request either drops it or forces the caller to
  own a retry policy, putting the same logic in two gears. The invariant AC #40 protects is
  *nothing stale is ever frozen*; rejection is the operator-facing delivery of it, and a retry
  with fresh collection is the machine-facing delivery — which also preserves the request.
  Evidence the requirement was operator-shaped from the start: its **only** listed actor was
  the catalog admin.
- **Why amend rather than confirm the reading**: the behaviour was already right; the defect
  was that the normative text said the opposite of the design, in **two** places (the FR and
  the AC both carried "rejected"). Such a disagreement is always resolved eventually, and
  usually by whoever has least context — here by "fixing" the retry into a rejection, which
  would silently drop pricing's requests and stall its pending refs (`commit_overdue` on its
  side, `catalog_version_overdue` on ours). Related note: the lifecycle-state arm of the check
  is the review's own H2 fix; a version-only comparison misses a retirement, which is exactly
  the case the AC spells "retired".
- **Propagated**: PRD `fr-catalog-publish-concurrency` (normative sentence, rationale, actor
  list) + AC #40; S6 `inst-sn-revalidate` (flag struck) and §5's both-arm probes
  (already assert both lanes, unchanged); `DESIGN.md` slice row + status line.

#### P-D-10 — No gear-side Legal role: the allow-list records Legal's decision, it does not enact it

- **Date**: 2026-08-26 (product call, answering the flagged M6 reading in design slice 10)
- **Decision**: the PII allow-list is a `GovernedLiveOp` under the **base approver quorum**
  (05 C1: the tenant's configured `N` distinct approvers, each distinct from the author, each
  holding CatalogAdmin or FinanceReviewer — `N` default 2, floor 0 per **P-D-11**, which landed
  the same day and amended C1 after this entry first quoted it as a fixed `≥ 2`). **No Legal
  role is introduced into this gear** — no actor, no IdP claim, no
  config row. Legal's authority is exercised outside the system and enters it as a **record**:
  each allow-list entry carries a mandatory Legal sign-off reference beside its justification,
  and an entry offered without one is refused. Consequently **role predicates narrow within the
  base set and never replace it**, and v1 registers no extension point that could (05 C8).
- **Why**: PRD AC #35 already specifies this construction in its own words — "curated
  allow-list; **Legal sign-off recorded in the approval artifact**". It never gives Legal a
  role, and §15 keeps the sign-off as an external obligation owned by Legal. Enforcing it
  in-system would require Legal counsel to hold platform identities, which no requirement asks
  for; and the two alternatives were both worse. "Predicate **adds**" would make a Legal
  reviewer also hold **CatalogAdmin** — the role that publishes price-bearing catalog changes,
  retires SKUs and governs the freeze participant set — i.e. a privilege escalation for a
  staffing reason, against the separation of duties the gate exists for. "Predicate
  **replaces**" is a bypass surface unless guarded, since a kind whose predicate admits anyone
  passes a material change on one signature.
- **What this closes**: the replacing-predicate grant lived in slice 05 `inst-mt-inputs` clause
  (d) and was flagged there as "a design reading of the AC #35 'Legal sign-off' clause", with
  slice 10 `inst-pp-allowlist` as its only intended user and the sole flag record in slice 10's
  M6 note. Both are retired: clause (d) now states the narrowing rule and C8 carries it as a
  constraint. Worth keeping in view — the gear's *other* predicate,
  `inst-gv-finance-predicate`, was additive all along (C1 already demands
  CatalogAdmin-or-FinanceReviewer; it demands that one of the two *be* a FinanceReviewer), and
  `APPROVER_ROLE_REQUIRED` fires on an unmet *additional* constraint, so the replacing grant
  was the one shape in the slice that no built predicate needed. Note also that slice 05 §6
  never listed the question among its risks — only slice 10's parenthesis and clause (d)'s own
  aside carried it, which is why it read as settled from either side alone.
- **What the gear does not claim**: that Legal approved. It proves only that a reference was
  recorded. The control remains the §15 paper sign-off plus the per-tenant audited export.
  Stated explicitly so the recorded reference is not later mistaken for an enforced approval.
- **Propagated**: S5 §1.6 **C8** (the narrow-never-replace rule + the two guards
  any future replacing predicate would owe); S10 `inst-pp-allowlist` + §5 probe (base
  quorum, mandatory reference, positive control); `design/README.md` slice-10 summary;
  `DESIGN.md` slice row + status line.
- **No PRD edit owed** — `fr-materiality-gated-publish` and AC #26 keep their closed approver
  set, which this decision makes true again, and AC #35 is followed rather than amended. *(Moved
  out of the `**Propagated**` field, where it read as a claim into the PRD — 2026-08-26 branch
  review.)*

#### P-D-11 — The approver count is a policy value with floor 0; the predicates are not

- **Date**: 2026-08-26 (product call, answering the flagged quorum-strictness risk in design
  slice 05)
- **Decision**: the approver count `N` becomes part of the typed materiality policy —
  **default 2, floor 0**. What does **not** become configurable: the FinanceReviewer predicate
  on finance-material fields (it governs *who*, never *how many*), the refusal of self-approval
  at every `N ≥ 1`, the requirement that `N` be reached only by explicit configuration (absent
  ⇒ default, so `0` is never reached by omission), and the provisioning-time origin of the
  initial value with every later change to it material under the then-current quorum. At
  `N = 0` the `ApprovalRecord` still exists — author, pinned content snapshot, audit row,
  `quorum {required: 0, satisfied: 0}` — and when the configured `N` cannot carry a mandatory
  predicate the descriptor carries an explicit **`predicateUnsatisfiable`** marker.
- **Why**: the fixed `≥ 2` was measured against the tenants it governs and against the sibling
  gear, and lost on both counts. A two-person commercial team could publish no material change;
  a **one-person** tenant could publish **nothing at all** — first publish and every lifecycle
  transition to `published`/`deprecated`/`retired` are material, a non-material change still
  demanded one approver, and C2 forbids self-approval. Meanwhile plan-price — the gear that
  holds the money, which this one does not (01 C3) — ships `submitter + 1` in its **schema**
  (`pricing_approval` carries `submitter_principal`/`approver_principal` as two columns under
  `chk_pricing_approval_distinct_principals`, so more than one approver is structurally
  impossible there) **and** an approver-less path for below-threshold non-first publishes
  (`PublishAuthorization::AutoPublishable`). The strict-catalog/lenient-money asymmetry was
  inverted from what risk suggests, and nothing recorded a decision to create it.
- **Why floor 0 and not floor 1**: floor 1 does not unblock the case it exists for. First
  publish is material by unscoreability (no baseline — pricing's G1, adopted here), so a solo
  tenant at `N = 1` still cannot publish a first SKU, having no second principal. The count
  therefore replaces the number everywhere, and materiality governs which predicates apply and
  what is recorded rather than how many humans sign.
- **Why 0 rather than permitting self-approval**: they are not the same act. At `N = 0` the
  trail says "no approval required by policy". An author signing as their own approver produces
  a trail saying "approved by X" where X is the author — **indistinguishable from a bypassed
  control** to whoever reads it later. The honest mechanism for "the owner decides alone" is a
  configured zero, which is why `SELF_APPROVAL_FORBIDDEN` is untouched at every `N ≥ 1`.
- **Amended the same day (2026-08-26), closing a hole this entry had left**: "the FinanceReviewer
  predicate is vacuous at `N = 0`" was stated here and **not built into the mechanism**. Slice 05's
  `inst-gv-finance-predicate` set the predicate on any finance-material touch regardless of `N`,
  `inst-gv-quorum`'s evaluator answers "satisfied" only on distinct principals holding the required
  roles, and 01's `inst-fd-governance-gate` raises `APPROVAL_REQUIRED` otherwise — so at `N = 0`
  the descriptor demanded a role no principal could hold and the change was **unpublishable
  forever**, re-blocking the one-person tenant this decision exists for (`taxCategory` is required
  at publish for product/service types, so their first such SKU could never publish). The register
  said *vacuous*; the mechanism said *unsatisfiable*, and the mechanism is what gets built. Now
  normative in both: at `N = 0` the predicate is **not set** and the descriptor records
  `predicateUnsatisfiable = finance_reviewer`, which the evaluator treats as met while the record
  and the inbox envelope keep it visible as unmet-by-policy; at `N ≥ 1` nothing changes. Found by
  the CodeRabbit pass on the same branch, hours after the wave landed — the exact class this
  programme keeps meeting: a decision's prose outrunning the mechanism that has to carry it.
- **The reduction, stated rather than wrapped**: at `N = 0` a material catalog publish proceeds
  on one person with no second pair of eyes, and the FinanceReviewer predicate has no subject. What
  compensates is not another approver: the pinned content snapshot, the full audit row, the
  attribution, the governed-ness of the lowering itself, and the `predicateUnsatisfiable`
  marker that makes the missing control a stored fact rather than an inference from a config
  value nobody re-opens.
- **On divergence from pricing (asked explicitly)**: the default stays 2 and pricing's effective
  count stays 1, so the two gears differ **by default** — deliberately, and cheaply. The shared
  studio inbox envelope already carries `quorum {required, satisfied, …}` per row
  (`inst-gv-queue`), so heterogeneous quorums render per card with no adapter; parity is one
  configuration value away in either direction. The alternative parities were both worse:
  lowering this gear's *default* to 1 weakens a stated control as a side effect of a
  small-tenant concern, and raising pricing to 2 is a governance change in the money-holding
  gear (41 non-test sites, 15 test files, 22 files, plus its distinctness CHECK and the move
  from two columns to a decisions table) that belongs in the pricing register with its own
  number, not in a line of ours.
- **Terminology**: "two-person rule" meant author + 2 (three people) in this PRD and
  `submitter != approver` (two people) in pricing. One phrase, two meanings, and it is the
  likeliest origin of the floor being three without anyone deciding it. Normative text now
  states the number.
- **Propagated**: PRD `fr-materiality-gated-publish` (normative sentence) + AC #26 (two
  bullets) + §17.1 materiality-threshold row; S5 §1.6 C1, §1.7 `ApprovalRecord`,
  `inst-gv-materiality` (the "nothing publishes approver-less" interim retired),
  `inst-gv-queue` (envelope gains `predicateUnsatisfiable` **and `configuredQuorum`** — its
  `required` is the record's *effective* count, `N` for material and `min(N, 1)` for non-material,
  never the raw configured `N`, so a card cannot show "2 required" for a record closing on one),
  §6 (flag struck); **S3 `inst-ac-required`** — the rule this decision's own amendment
  names as the operand of the hole it closed (`taxCategory` required at publish for
  `product`/`service` types is what made the `N = 0` tenant unpublishable forever), which
  restated the substance without citing the decision until the 2026-08-26 branch review;
  `DESIGN.md` slice row + status line.

#### P-D-12 — The `SchemaPin`'s membership is a rule, not a list

- **Date**: 2026-08-26 (product call, answering the flagged L1 item in design slice 12)
- **Decision**: `fr-plan-price-seam` is **amended** so the pin covers exactly the **operands the
  consumer obligations are enforced on** — the §2.2 `ObligationRegister`'s guards. Adding an
  obligation therefore adds its operand, and the coupling is doc-linted in both directions by
  the new `inst-cc-pin` (CoverageChecks #9). Derived v1 membership: `skuId`, `type` (the
  `bundle` discriminator), the metering-unit declaration **and** `usageTypeRef`, `PlanTier`,
  `status` **together with its value vocabulary**, `sellable`, `compositionPending`.
  `CatalogVersion` is pinned as a **surface**, not a field. Explicitly **outside** with the
  reason recorded: `skuCode` and `name` — read only by a pick-list that validates nothing, so
  drift is cosmetic and its absence must not read as an oversight.
- **Why a rule instead of the two fields the flag asked about**: measuring the register against
  the FR's list found **four** operands outside the pin, not two — `status`,
  `compositionPending`, `sellable`, `usageTypeRef` — and the FR **states three of those
  obligations in its own sentence** ("reject adoption of `compositionPending`/`deprecated`
  SKUs; reject a usage binding when the target SKU has no declared unit …") while pinning none
  of their operands. A list that drifted from the obligations once will drift again; the rule
  plus the lint is the only form that cannot.
- **Second finding, which the list could not survive either way**: of the five items it named,
  only **three** are comparable fields of the consumer's shape. Pricing's shipped `CatalogSku`
  is `sku_id`, `sku_code`, `name`, `metering_unit`, `status`, `plan_tier` — `bundle` type is
  absent from it (slice 12 adds it as `type`), and `CatalogVersion` is not a SKU field at all.
  So the pin as written could not do its job even for what it listed.
- **Why `status` needed pinning most, in pricing's own words**: its field doc reads
  "`draft` | `published` | `deprecated`, **verbatim. Not an enum**: the registry owns this
  vocabulary and a fifth state must not become a parse failure in the gear that merely displays
  it." That is the right tolerance for display and exactly the wrong one for a guard — rename
  `deprecated` or add a blocking state and pricing's adoption guard stops matching **and does
  not error, it accepts**. Hence the vocabulary, not just the field name, is pinned.
- **The failure the pin exists to prevent, concretely**: `compositionPending` unpinned means
  pricing can adopt an uncomposed bundle — a bundle priced with no components, and a customer
  billed for a set that resolves to nothing.
- **Honest limit**: the seam suite does not exist yet (PRD §15 owns its home and owner —
  "proposed: `api-contracts` CI"). This widens a **specification**, not a running gate. Getting
  the membership right before the job is built is cheaper than after, and nobody should read
  this as drift becoming detectable tomorrow.
- **Propagated**: PRD `fr-plan-price-seam` (normative sentence); S12 §1.6 C1,
  `inst-sdk-catalogsku` (`status` + vocabulary now normative pin members, previously a flagged
  widening), §3.2 **`inst-cc-pin`** (CoverageChecks #9 — the count in prose moved with it),
  §6 (flag struck); `DESIGN.md` slice row + status line.

#### P-D-13 — The quorum shorthand's reach is enumerated; a floor only where the principal is not the tenant's

- **Date**: 2026-08-26 (answering Blocking 1 of the bad-mood review of this branch)
- **Decision**: P-D-11 made `N` a policy value with floor 0 and the glossary made the shorthand
  total ("wherever *two-person* appears as shorthand below, read it as this quorum"). That reach
  is now **enumerated and dispositioned**, six sites, rather than left to the reader:
  - **Cross-tenant break-glass elevation** (AC #30, `inst-bg-open`) is **not `N`-governed at
    all**. Its principal is a platform owner acting across tenants; no tenant's configured `N`
    has standing over an act whose subject is another tenant's data. Fixed floor: **two distinct
    platform principals**, or the AC's already-stated **post-hoc-review** arm.
  - **Freeze force-completion** (AC #22), the **uncomposed-bundle override** (AC #19,
    `inst-cl-bundle-override`), **un-deprecation** (AC #17, `inst-lc-undeprecate`) and the
    **slice-07 correction door** (`inst-cr-republish`) follow `N` — and each **records the
    reduction**: when the effective count is below the retained-name default of 2, the
    authorizing `ApprovalRecord` and the act's event carry **`quorumReduced`**, so no audit
    trail reads "two-person" for a one-person act. *(This entry first named `inst-mt-inputs`
    here, which is slice 05's materiality-inputs instruction and not a door at all — the
    correction door's own instruction is `inst-cr-republish`. The wrong id is why the sweep
    reached three of these four and not the fourth: 2026-08-26 review of this branch.)*
  - **The slice-07 break-glass correction** (`inst-bc-ceremony`) is the **sixth** site, and it
    follows `N` with the reduction recorded, on this entry's own principle: a fixed floor is
    right only where the acting principal is not the tenant's, and this principal **is** the
    tenant's. It is a separate lane from `inst-bg-open` — both slices say so explicitly
    (05 C5, 07 C5: it is **not** a §6.8 `BreakGlassSession`) — and a separate admission gate
    from the ordinary correction door (`inst-cr-door`: "one door, three admission gates (two when P-D-13 was written; P-D-16 added the third on 2026-08-26)"), so it
    inherits neither disposition and had to be dispositioned on its own. What makes the lane
    safe at `N = 0` is not a quorum floor but the three controls it already carries — the
    feature flag OFF by default, the mandatory reason with the `SkuCorrectionOverride` evidence
    snapshot, and the `TripwireCounter` escalation that raises the release blocker past
    5/30 days. A floor of 2 here would wedge the one-person tenant in the one state where the
    ordinary safety predicate cannot answer, which is the class of block P-D-11 exists to
    remove.
  - The `OverrideCeremony` keeps its **informed** property at every `N`: at `N = 0` the
    **author** performs the acknowledgment-of-findings-by-name, recorded identically. Multiple
    people was never what that ceremony bought; informedness was.
- **Why not a floor on all six**: floor 2 on force-completion leaves a solo tenant with a
  `CatalogVersion` permanently past its freeze timeout and un-resolvable — the exact class of
  block P-D-11 exists to remove. And floor 1 on un-deprecation would contradict P-D-11's own
  enumeration, which already makes every lifecycle transition **to `published`** material and
  therefore `N`-governed; un-deprecation is that transition. A fixed floor is right only where
  the acting principal is not the tenant's, which is break-glass and nothing else in v1.
- **Why `quorumReduced` rather than a second approver**: it is the `predicateUnsatisfiable`
  device from P-D-11, applied to the count instead of the role — the missing control becomes a
  **stored fact** on the record instead of an inference from a config value nobody re-opens. The
  inbox envelope already carries `configuredQuorum` alongside the effective `required`
  (`inst-gv-queue`), so the marker costs a column and no new surface.
- **How this was missed, recorded because the class repeats**: the sweep followed P-D-11's
  propagation list, and the four ceremonies are not on it — they are not materiality questions,
  which is exactly why the shorthand reached them unexamined. Commit `a282041f8`'s message
  asserted that break-glass and force-completion "carry their own fixed two-person rules"; **no
  document stated such a rule**. The claim was true of the intent and false of the text, and the
  text is what gets built (the same shape as P-D-11's own `predicateUnsatisfiable` finding).
- **And how the fix itself was missed, same day**: this entry's first draft enumerated the
  ceremonies correctly but named **the wrong instruction id** for one of them, and named a
  ceremony's *sibling lane* nowhere at all. Both cost a site: the sweep that followed this
  enumeration reached slice 03, 04 and 06 and never opened slice 07, so the correction door
  and the break-glass correction still read a bare "two-person" a full wave after the decision
  that governs them. **An enumeration is only as good as its ids** — an id that resolves to
  another slice's instruction is worse than no id, because it reads as swept.
- **Propagated**: PRD glossary **Two-person rule** row; AC #17, #19, #22, #30; S5
  §1.6 C1, §1.7 `ApprovalRecord` + `OverrideCeremony`, `inst-bg-open`, `inst-gv-override`,
  `inst-gv-queue`, `inst-mt-inputs` (the metering-unit clause that routes to slice 07's correction door (a route, not a propagation target)),
  §4 `products_approval`/`products_breakglass_session`; S3 `inst-cl-bundle-override`;
  S4 `inst-lc-undeprecate`; S6 `inst-fz-force`; **S7 C4, C5,
  `inst-cr-republish`, `inst-bc-ceremony`**; `DESIGN.md` decision register summary.

#### P-D-14 — `system_signal` is an approval subject kind, not an exemption; the authorizing principal is the signal

- **Date**: 2026-08-26 (recorded 2026-08-26 by the branch review — **decided in prose on this
  branch and never registered**, which is the defect this entry closes)
- **Status**: **FLAGGED for the owner.** The design is built on it; nothing here changes
  behaviour. It is registered so the owner can veto a publish path that has no human approver.
- **Decision**: a publish whose **sole** content is a system-owned flag cleared by an inbound
  governed signal — in v1 exactly one: 06's `compositionPending` clearing — uses `ApprovalRecord`
  subject kind **`system_signal`**. The record is auto-satisfied with the **signal reference as
  the authorizing principal**, audited like any other decision. There is no human approver and
  **no exemption from the gate**: the act still produces a record, still lands in the audit
  trail, and is still refused if its preconditions fail.
- **OPEN, and the one thing this entry does not settle (2026-08-26, third review pass)**: what a
  dirty head does. This entry says *refused*; the owning slice's `inst-cc-clear` says the clear is
  "**deferred, never refused**" (`design/06-catalog-version.md` §2) and §3.2 raises **no** error code
  for it by design. Both are defensible — a refusal is louder, a deferral cannot wedge a publish
  queue — and only one can be built. `fr-materiality-gated-publish` and AC #26 deliberately assert
  neither. **Owner picks.**
- **The precondition that makes it safe**: the head must be **clean**. A `system_signal` publish
  carries the flag and nothing else; if the head holds unpublished bucket-iii/iv edits the
  publish is refused rather than carrying them out under a record with no human approver — **this entry's reading, and the one the OPEN bullet above puts to the owner**; the owning slice defers instead. This
  is not decoration — `cc752aed4`'s own Blocking-5 note records the alternative concretely: a
  publish "whose sole content is a system-owned flag" could otherwise "carry `taxCategory` and
  `PlanTier` edits out under an `ApprovalRecord` with no human approver".
- **Why a subject kind rather than an exemption**: an exemption leaves no record and therefore
  no audit answer to "who cleared this". A subject kind spends one column and keeps the gate's
  shape — the same device P-D-11 used for `predicateUnsatisfiable` and P-D-13 for
  `quorumReduced`: turn the missing control into a **stored fact** rather than an absence.
  The composition-clear gate was on the flag list as an *exemption* until the 2026-08-26
  CodeRabbit pass forced its resolution; it left the list as a subject kind.
- **Why it is independent of `N`**: the tenant's configured quorum governs acts whose principal
  is a tenant principal. This act's principal is an inbound governed signal from a counterpart
  gear, so `N` has no standing over it — the same reasoning P-D-13 applies to `inst-bg-open`
  from the other direction. It follows that `N = 0` neither weakens nor strengthens this path.
- **What "governed" means for the signal**: it arrives through a registered inbound machine
  contract (PRD §9.2), S2S-authenticated as the producing gear, and is itself recorded. An
  ungoverned or unauthenticated signal cannot open this path.
- **Propagated**: S5 `inst-gv-one-shot`; S6 §2 composition-clear flow and its
  `compositionPending` clearing; `design/README.md` slice-06 bullet; `DESIGN.md` §6 status block
  + decision register summary; PRD `fr-materiality-gated-publish` (the subject-kind sentence)
  and AC #26 (its own `And` clause). *(Corrected 2026-08-26: this said AC #7, which is "Define a
  SKU"; the materiality AC is #26. Both targets carried the claim and neither carried the
  sentence until now — and the register's own gate read the claim as verified, because the
  field names `PRD` and the PRD had begun citing P-D-14 in an unrelated change-log line.)*

#### P-D-15 — The two inbound machine contracts are `products-sdk` clients from `ClientHub`, not out-of-process REST doors

- **Date**: 2026-08-26 (the transport wave — **named in `DESIGN.md` as a decision that "landed
  without having been flagged" and then never entered this register**; that asymmetry is what
  this entry closes)
- **Status**: **FLAGGED for the owner.** It is a shape counterpart gears build against, so it is
  the one entry here a neighbouring team can be broken by.
- **Decision**: **every** inbound machine contract of PRD §9.2 is consumed as a **`products-sdk`
  client resolved from `ClientHub`**, in-process, rather than as a REST door that binds the
  counterpart out-of-process. §9.2 declares four — the `CatalogVersion` **increment request**,
  the **`SkuReferenceCount`** watermark, the **freeze acknowledgment** (and its release half,
  P-D-18) and the **bundle composition-completed** signal — so the rule is stated over the set
  rather than over a pair. The increment request is registered in PRD §9.2 beside its sibling,
  which is where the asymmetry between them was first visible.
  *(Corrected 2026-08-26: this entry said "the two inbound machine contracts" and named the
  increment request and the composition signal, while slice 12's `inst-sdk-surface` said "the
  two" and named the increment request and the watermark. Two registers, two different pairs,
  and §9.2 holds four contracts — so neither reading was right and the count was the defect.)*
- **Why**: the platform's own composition model puts sibling BSS gears in one process behind
  `ClientHub`; a REST door for an in-process call buys a network hop, a second authz surface and
  a second failure mode for no isolation the deployment actually has. The SDK client keeps the
  contract typed and versioned at the seam that slice 12 already pins.
- **The cost, stated**: an out-of-process counterpart — a gear that later moves, or a
  non-Rust producer — needs the REST door built. Nothing in v1 has that shape, and slice 12's
  `ObligationRegister` is where such a consumer would be booked when it appears.
- **Rejected alternative**: REST doors for both. Rejected on the cost above, not on principle;
  if the owner expects an out-of-process producer inside v1's horizon, this decision flips and
  §9.2 gains two door definitions.
- **Propagated**: PRD §9.2 (both inbound contract blocks); S6 increment-request
  intake; S12 `inst-sdk-surface` + the `ObligationRegister`; `DESIGN.md` §6 transport paragraph.

#### P-D-16 — A third correction-admission arm: an unresolvable meter target

- **Date**: 2026-08-26 (recorded by the branch review; the arm was authored into slice 07 as
  item 19 of that day's earlier review and stood against two `MUST`s until this entry)
- **Status**: **FLAGGED for the owner** — it amends a normative FR and an AC.
- **Decision**: `fr-immutable-field-correction` and AC #4 are **amended** to carry a third
  admission arm. Besides (a) fresh-zero and (b) break-glass while the signal is entirely
  unavailable, the correction door admits a **meter-declaration** correction when the subject's
  declared `usageTypeRef` **no longer resolves** — the `UsageTypeResolver` answers not-found,
  never a timeout — **regardless of the reference predicate**. The ceremony is unchanged
  (`N`-governed + mandatory reason + `SkuCorrectionOverride`), and the override record's
  evidence field carries **`unresolvable-target`** rather than unavailability evidence.
- **Why the arm has to exist**: a sold SKU whose `UsageType` the collector deleted is wedged in
  every lane at once — `fresh > 0` refuses the normal door (`CORRECTION_REFERENCED`), the signal
  *being available* refuses break-glass (`CORRECTION_SIGNAL_AVAILABLE`), and retire-and-clone is
  refused because the flip guard defers on anything but fresh-zero with no force-retire door in
  v1 (04 C4). PRD §15 confirms the collector can delete a referenced usage type, so this is a
  reachable state and not a hypothetical. Left unamended, the requirement's fail-closed default
  has no exit and the SKU stays broken forever.
- **Why `fresh > 0` is the reason to admit, not to refuse**: the fail-closed default exists to
  stop a correction from silently changing what a live consumer already bound. Here the binding
  is *already* broken — the declared target does not exist — so refusing preserves nothing and
  repairs nothing. The reference being real is what makes the repair urgent.
- **What this closes, and what it does not**: it closes the **registry-side** half of the PRD §15
  row *"UsageType deletion vs published declarations"* — the wedged-SKU repair, whose answer
  column read **TBD** while slice 07 already implemented an answer. It does **not** close the
  cross-gear half: the deletion-guard / deletion-signal negotiation with usage-collector stays
  an open §15 item owned by that gear, exactly as P-D-05's residue records it. (Corrected
  2026-08-26: this entry read as closing the whole row, which contradicted P-D-05 on the same
  row — a local repair arm is not a deletion contract.)
- **OPEN, registered 2026-08-27**: this arm is deliberately **not** behind
  `BREAKGLASS_CORRECTION_DISABLED`, because a default-OFF flag would withhold the exit the decision
  exists to provide. That leaves a governed but permanently open write path onto a published,
  `fresh > 0` SKU's bucket-ii meter declaration. Whether it should carry a flag of its own (the arm already increments the same `TripwireCounter` the break-glass lane uses — `inst-bc-unresolvable`) is the owner's call; the ceremony, the reason and the
  `SkuCorrectionOverride` evidence row are required either way.
- **Rejected alternative**: drop `inst-bc-unresolvable` and leave the wedged SKU to the §15
  negotiation. Rejected because the negotiation has no v1 landing and the state is reachable in
  v1 — but this is the arm to strike if the owner prefers the quarantine fail-safe §15 names.
- **Propagated**: PRD `fr-immutable-field-correction`, AC #4, §15 row (closed); S7
  C5, `inst-bc-admission` (the "only" quantifier now names both arms), `inst-bc-unresolvable`,
  `inst-cr-republish` (the validator re-checks the admitting lane's own predicate).

#### P-D-17 — Promotion identity collision with different content is update-as-draft, not a per-row conflict

- **Date**: 2026-08-26 (recorded by the branch review; slice 09 was amended to this reading as
  item 15 of that day's earlier review, against three unamended PRD statements)
- **Status**: **FLAGGED for the owner** — it amends an FR, an AC and a §10 use case.
- **Decision**: `fr-bulk-import-export`, AC #33a and `usecase-environment-promotion` are
  **amended** to carry slice 09's exhaustive four-way classification: unknown identity ⇒
  **create**; identity bound to **matching** content ⇒ **no-op**; identity bound to **different**
  content ⇒ **update-as-draft** against the existing entity; identity bound to an **incompatible
  kind/type, a `retired` holder, or a dirty head** ⇒ **per-row conflict**.
- **Why**: "an identity collision is a per-row conflict" makes the **modal** promotion row fail.
  Promoting a changed SKU into an environment that already holds it *is* the workflow — the PRD
  cites Stripe test/live and Zuora Deployment Manager as the parity target, and in both the
  second promotion of an object is an update. Under the unamended sentence, every environment
  after the first can be populated exactly once and never updated again.
- **How AC #33a's "never a silent merge" survives**: it is satisfied, not dropped. The update
  lands in **`draft`**, publication stays gated behind the batch's own quorum, and the
  `ChangeReport` shows the row. Nothing merges silently because nothing publishes silently.
- **Why the dirty-head arm is the load-bearing half**: a target holding **unpublished** edits is
  a conflict, so promotion can never clobber work in progress. That is the data-loss path the
  PRD's blunt sentence was protecting against, and it is protected precisely here.
- **Rejected alternative**: restore `conflict` for content difference in C5 and
  `inst-pm-resolve`. Rejected on the modal-row argument above; it is the change to make if the
  owner reads promotion as a create-only channel.
- **Propagated**: PRD `fr-bulk-import-export`, AC #33a, `usecase-environment-promotion`
  (Alternative Flows); S9 C5 + `inst-pm-resolve`.

#### P-D-18 — Version liveness ends by an explicit release; the release is a fifth inbound contract

- **Date**: 2026-08-26 (recorded by the branch review; built into slices 06/10/12 as the
  slice-10 review's H1 fix, while the PRD question that authorises it was still open)
- **Status**: **FLAGGED for the owner** — it closes an open §15 row and adds a duty on three
  counterpart gears.
- **Decision**: version liveness is **acked-and-not-yet-released**. A freeze participant that
  holds no more live references to a `CatalogVersion` records that through a
  **`catalog_version × release`** door (S2S, the participant's own identity), and the release is
  added to PRD **§9.2** as an inbound machine contract beside the freeze acknowledgment — the
  two halves of one participant obligation, documented together. §15's *"Snapshot-GC
  version-liveness source"* row is **closed** on the freeze-registration option it offered.
- **Why an explicit release rather than a `(catalogVersionId, producer)` count**: the per-SKU
  reference signal has no version dimension (§15 says so), so a version-scoped count would be a
  second signal with its own freshness, its own staleness alarm and its own producer set. The
  release is one idempotent fact per participant per version, and it rides the acknowledgment
  contract that already exists.
- **Why it must be in §9.2 and not only in the design**: slice 10's `RetentionGate` refuses to
  collect a version until **every** freeze registration satisfies the pair — `state = released`, or `not_frozen(forced)` with `released_at` stamped by force-completion (second arm 2026-08-26: a forced participant never acked and cannot use the S2S release door; a later recovery moves `state`, so a stale stamp frees nothing). That makes the release
  a **precondition for garbage collection** — a participant that never releases pins storage
  forever. A duty with that consequence cannot live only in the registry's own design; the
  counterpart has to be told it owes it.
- **The asymmetry this repairs**: the same branch already fixed exactly this shape for the
  increment request (PRD §9.2: "the two inbound machine contracts from the same counterparty had
  been documented asymmetrically"). The release door was missed by that sweep.
- **Propagated**: PRD §9.2 (`contract-freeze-ack` gains the release half), AC #44, §15 row
  (closed); S6 `inst-fz-liveness`; S5 RBAC (`catalog_version × release`);
  S10 `inst-rt-gc`; S12 `ObligationRegister` row.

#### P-D-19 — A force-completed version stays refused for posted use until opt-in; the pin is the registry's own door

- **Date**: 2026-08-26 (recorded by the branch review)
- **Status**: **FLAGGED for the owner** — it moves an enforcement point back from an unbuilt
  consumer to the registry, and it makes a forced version unpostable by default.
- **Decision**: for a `CatalogVersion` at `freezeComplete = complete(forced)`, the registry's
  `IntentfulResolver` **refuses `posted` resolution** (`VERSION_FORCED_INCOMPLETE`, naming each
  `not_frozen(forced)` participant) until either every forced participant has since frozen or
  released, or an explicit per-version operator **auto-fallback opt-in** is recorded. Browse
  resolution is unaffected. The opt-in is off by default, which is the shape
  `fr-freeze-recovery` already names as "an off-by-default later enhancement".
- **Why the design's reading could not stand**: `fr-freeze-recovery` and AC #22 state
  **"the default is pinned fail-closed for that participant's content"**. Slice 06 converted
  that default into a **consumer** obligation, and slice 12 books that obligation as **owed** —
  against pricing *and Billing, which has no gear at all*. So in v1 the enforcement existed on
  neither side: the registry resolved `posted` against a partially-frozen version and nobody
  refused it downstream. A stated safe default had become an unowned promise.
- **Why the registry can hold the pin even though it cannot hold the content**: slice 06 is
  right that the snapshot holds only *references* to a participant's content (C4), so the
  registry cannot refuse that content. It does not follow that the **version** must resolve
  `posted` — the resolver is a door the registry owns outright, and refusing the version is the
  fail-closed behaviour the requirement asks for, expressed where it is enforceable.
- **The cost, stated**: an operator who force-completes to unblock a stuck version cannot post
  against it until they take the opt-in. That is one extra deliberate act on the abnormal path,
  and it is the act that makes the risk a recorded decision instead of a silent one.
- **Rejected alternative**: amend `fr-freeze-recovery` and AC #22 to move the pin to the
  consumer, and promote slice 12's register row from `owed` to a launch gate. Rejected because
  it makes v1 depend on a gear that does not exist — but it is the right change if the owner
  intends posted resolution to stay available on forced versions.
- **Propagated**: PRD `fr-freeze-recovery`, AC #22; S6 `inst-fz-force` +
  `IntentfulResolver`, §5 error taxonomy (`VERSION_FORCED_INCOMPLETE`); S12
  `ObligationRegister` row (the consumer duty becomes belt-and-braces, not the only enforcement).

#### P-D-20 — A publish during the retirement lead window re-announces `SkuRetired`; the door stays open

- **Date**: 2026-08-26 (recorded by the branch review; slice 04 introduced the publish freeze as
  item 16 of that day's earlier review)
- **Status**: **FLAGGED for the owner** — it strikes a design-introduced normative refusal and
  adds a re-emission rule in its place.
- **Decision**: `RETIREMENT_PENDING` is **struck** from the `PublishDoor`. A live retire intent
  does **not** close the head to publishes; new adoption is blocked from initiation, as
  `fr-retirement-eol` requires, and the entity stays publishable. `fromVersion` remains pinned at
  the **initiation** instant — where it must be, because `SkuRetired` is emitted at initiation
  and the ≥ 30-day lead time exists precisely so consumers hear about the retirement early.
  What changes is what happens next: **a publish during the lead window re-emits `SkuRetired`**
  with the new `fromVersion`, the same `effectiveAt` and the same retirement identity. The
  announcement is an announcement, and an announcement whose subject moved is re-issued.
- **Why the freeze was wrong even though the problem was real**: the problem slice 04 found is
  genuine — publishing versions 8, 9, 10 after announcing "retires from version 7" makes the
  emitted payload a lie, and consumers pin against it. But the fix chosen was a
  **product-visible refusal for at least 30 days** (§17.1 lead time) that no PRD requirement
  carries. `fr-retirement-eol`'s only stated effects of initiation are that adoption is blocked
  and the entity stays browsable; a month-long publish freeze on an entity that is still live is
  a much larger constraint than either, and it arrived through a review fix rather than a
  requirement.
- **Why re-emission and not a flip-time `fromVersion`**: the tempting fix — resolve `fromVersion`
  at the flip, where it is truthful by construction — **does not work here, and the reason is
  worth recording because it is the obvious wrong answer.** `SkuRetired` is emitted **at
  initiation** (slice 04 `inst-rt-initiate`, and `fr-retirement-eol`'s own sentence orders it
  before the flip), so a flip-time value would arrive a month after the only event that carries
  it. Re-emission keeps the early announcement the lead time exists for *and* keeps the payload
  truthful, at the cost of one extra event on a path that is already rare.
- **What consumers owe**: `SkuRetired` is no longer at-most-once per entity. Consumers key on
  `(skuId, effectiveAt)` and take the **latest** `fromVersion`; the retirement identity does not
  change, so a re-announcement is an update and never a second retirement. That duty is booked
  in slice 12's `ObligationRegister`.
- **The escape hatch the freeze offered was itself broken**: slice 04 told an operator who needs
  to publish to "cancel the retirement first through the governed cancel that already exists" —
  a ceremony that this same wave found registered material nowhere, so it would have run on one
  approver. An escape hatch that under-specified is not an escape hatch.
- **Rejected alternative**: keep the freeze and push `RETIREMENT_PENDING` into
  `fr-retirement-eol` + AC #18 with `fromVersion`'s definition, which the PRD also lacks. That
  is the change to make if the owner reads a retiring SKU as frozen by intent — but it should
  then be a requirement, decided, and not a refusal that appeared in a fix commit.
- **Propagated**: PRD `fr-retirement-eol` + AC #18 (`fromVersion`'s definition and the
  re-announcement rule); S4 `inst-rt-initiate` (`RETIREMENT_PENDING` struck from the
  publish door, re-emission added), §5 error taxonomy; S12 `ObligationRegister` (the
  latest-wins consumer duty). *(Slice 01's `inst-fd-containment` is deliberately not a target
  here — the clause did not move; the bullet below says why in words, because a target list is
  a list of documents this decision changed.)*
- **What the exemption leaves standing.** Corrected 2026-08-26: this entry first described
  `inst-fd-containment` as guarding *re-parenting*, a door slice 01 does not have. It guards
  **child creation** under a parent holding a live retire intent, and it is genuinely unaffected
  by P-D-20, whose subject is the publish door. The consequence is worth stating rather than
  leaving to be rediscovered: during the lead window the retiring parent's head stays
  publishable while **no new SKU may be created under it**, and building the successor the
  retirement's `replacedBy` must name is the window's most predictable use — slice 11's C1 says
  exactly that. If that is the wrong trade, the fix belongs in `inst-fd-containment`, not here.
  Named in words rather than as a propagation target, because this is a statement about a
  clause that did **not** move.

#### P-D-21 — The local audit table holds only what emits no event; the event stream is the success-path record

- **Date**: 2026-08-27 (product call, in the slice-01 review, prompted by "мы же решили отложить
  audit?" and then "рассчитываем на платформенный аудит")
- **Residue flagged, and only the residue**: the decision below is the owner's, taken in
  conversation; two things in this entry are **not** and are open to veto. (1) The owner chose
  "local row for refusals, success by events"; the **second and third classes — reads under
  elevation, and committed acts declared to emit no broker event** — were added here because
  measurement showed the boundary the owner named ("what the event stream cannot carry") includes
  them: a read writes no outbox row, v1 elevation is read-only, and two sibling slices already
  route committed acts to the audit plane by design. (2) That
  **P-D-08's seam survives** is a reading of S1–S9, not something the owner said. Everything else
  is either the decision as stated or a measured consequence of it.
- **Decision**: v1 no longer writes a `products_audit_log` row for every mutating door. **The
  event stream is the audit of record for everything that succeeds**, and the local table
  survives only for acts the event stream structurally cannot carry. **The set was re-measured
  2026-08-27 after the first measurement missed a class** — it searched the audit *write* sites and
  not the phrase `audit-plane`/"no broker event", which is how sibling slices spell the third one.
  Three classes:
  - **refusals.** A rejected mutation rolls back its transaction and the outbox row rolls back
    with it, so no event exists; the design set declares **no** rejection event anywhere, and
    `fr-expected-failure-behavior` names **fifteen** cases that MUST fail closed *with an audited
    reason*.
  - **reads under elevation.** A read writes no outbox row at all. Break-glass in v1 is
    **audit-export only** (05 — "any write under elevation is refused, full stop"), so every
    audited act under elevation is a read, and 05 requires that "every elevated read leaves an
    audit row with the session id (count asserted, not sampled)".
  - **committed acts the design declares emit no broker event.** Not a structural limit like the
    other two — a deliberate choice already made per act, and it lands in the same place: slice 04
    writes `PublishScheduled`/`RetirementScheduled` as "audit-plane records, explicit \"no broker
    event\" per 01 §4.5", and slice 10 records the erasure act itself "audit-plane, explicit **no
    broker event** carrying identity". Both are committed and neither leaves an event, so the
    event stream is not their record either. **Found by the third lens on the pass that followed
    this entry, not by the measurement that wrote it.**
- **Why**: the outbox row is written inside the mutation's transaction, so for a *successful*
  write the event is exactly as durable and as transactional as the audit row was — P-D-08 S3's
  objection is to a **network call in the write path**, which the outbox pattern does not make.
  Erasure survives untouched: `fr-retention-erasure` already reasons over event streams in the
  same breath as audit rows ("because events carry only pseudonymous actor references, updating
  the reference map completes erasure without touching immutable event streams"), and slice 10 C1
  names "audit/event records" as one class. So duplicating every successful act into a second
  local table bought retention cost and a second erasure surface, and no control the events did
  not already provide.
- **Consequences, recorded rather than hidden**:
  - **The event payload must carry what the audit row carried.** The audit row's tuple is
    `actor_ref`, action, subject `(kind, id, revision)`, reason, correlation id. The stated
    payload (01 §4.4) carries the envelope, a versioned schema ref, correlation/causation, the
    idempotency key and `actor_ref`; the event type supplies the action and `aggregate_id` the
    subject id. **`revision` is not in it** — and without it a consumer cannot say which revision
    an act applied to, which is the whole point of an audit trail over a versioned entity. Owed
    as a payload amendment, not assumed here.
  - **Slice 03's resolved-binding snapshot loses its home.** `inst-cd-stamp` stamps
    `(gts_id, kind, metadata_fields)` "into the audit row of the publish", and a publish is a
    success. §15's deletion negotiation and pricing's `meter_binding_divergent` remediation are
    both written to reference it. It must move to the publish event payload or to
    `products_entity_version`; **which one is slice 03's call and is registered there, not
    decided here.**
  - **Retention moves onto the event store.** `fr-retention-erasure` requires audit records kept
    "for the configured retention duration" alongside financial records. A broker does not retain
    on that horizon, so the durable sink must be the platform audit capability — which per PRD
    §15 **does not yet exist** and is owned by Architecture. Until it does, successful-act audit
    is retained only as long as the event store retains it, and that is a v1 gap this decision
    creates deliberately.
  - **`nfr-availability-audit`'s "100% write-path audit"** is now satisfied by two records of
    different kinds — event for the committed path, local row for the refused path. The threshold
    holds; the mechanism named in the NFR's prose does not, and the PRD sentence needs the
    amendment.
  - **P-D-08's sealing seam still applies**, now to a much smaller table. Nothing in P-D-08's
    S1–S9 depends on the table's volume, and the seam's whole value — migration-free activation
    plus an era marker in the data — is unchanged. The seam is **not** struck by this decision.
- **Propagated**: `DECISIONS.md` P-D-08 (amended, pointer added), `design/01-foundation.md`
  (§1.1/§1.5/§1.8 framing, every success-path flow row, §4.4's table scope, §6's three registered
  consequences), `design/02-taxonomy-attributes.md` (`inst-tx-event`, `inst-gl-atomic`, `inst-ad-event`),
  `design/11-clone.md` (`inst-cn-lineage`).
- **Owed, and deliberately not applied by the wave that recorded this** — each needs its own
  slice's judgment rather than a sweep, and none of them is a phrasing change:
  - `PRD` `fr-registry-eventing-audit` and `nfr-availability-audit` prose — the "100% write-path
    audit" threshold survives, the single-mechanism sentence under it does not.
  - `design/03-sku-classification.md` `inst-cd-stamp` — the resolved-binding snapshot's
    new home (publish event payload, or `products_entity_version`).
  - `design/07-reference-signal.md` `inst-pr-governed` — its trailing audit obligation covers a
    `GovernedLiveOp` that both succeeds and refuses; which half stays local needs the row read
    whole.
  - `design/10-retention-erasure.md` §4 — "retention/drill state is config + audit, no new record
    tables" puts a *successful* drill's state in a store this decision empties.
  - `design/05-governance.md` — its audit-row references are refusal- and elevation-side and look
    correct as they stand, but were not read one by one.
  - The event-payload amendment carrying `revision` (see Consequences).

#### P-D-22 — The registry uses the toolkit's transactional outbox, not a gear-local one

- **Date**: 2026-08-27 (product call, in the slice-01 review — "взять toolkit")
- **Decision**: `products_outbox` as a gear-authored table is **struck**. The registry enqueues
  through **`toolkit_db::outbox`** (`libs/toolkit-db/src/outbox`), which ships the whole pipeline:
  `enqueue` inside the caller's transaction → `sequencer` assigning per-partition sequence numbers
  → `processor` invoking the gear's handler → `vacuum` collecting delivered rows. Its tables
  (`_body`, `_partitions`, `_incoming`, `_outgoing`, `_dead_letters`) carry a configurable prefix,
  and it brings its own migrations.
- **Why**: the design set copied the outbox shape from pricing, which its §1.4 names "the pattern
  donor". Measured 2026-08-27: **pricing does not use the platform facility** — it has its own
  `pricing_outbox` — while **mini-chat, the reference gear, imports `toolkit_db::outbox::Outbox`
  directly**. So the gear had inherited a private re-invention from a sibling rather than the
  platform's own component, and inherited it without the dead-letter table, the lease handling,
  the vacuum, or the multi-backend migrations that come with it.
- **The PRD contract is untouched, and that was checked before deciding**: `fr-registry-eventing-audit`
  requires the *envelope* to stamp "per-aggregate ordering keys `(tenant, aggregate…)`" and AC #28
  repeats it — a property of the message, not of a storage column. The toolkit's `enqueue` takes
  the partition from the caller, so `partition = hash(tenant_id, aggregate_id) mod N` puts every
  event of one aggregate in one partition and preserves their relative order, which is exactly
  what the ordering key promises.
- **Consequences**:
  - **Delivery stops being a column.** The gear-local design had the dispatcher "mark delivered
    only on durable broker acceptance" and never named a column to mark; in this model the row
    leaves — the processor hands it to the handler and the vacuum reclaims it. Slice 10's
    `RetentionClock` class "outbox-delivered" is therefore the **vacuum's** horizon, not a
    retention rule this gear writes, and that slice owes the correction.
  - **The UNIQUE `(tenant_id, aggregate_id, sequence)` this slice added earlier the same day is
    superseded** by the toolkit's own unique index on `(partition, seq)`.
  - **C1's "one migration per table, guards defined once" does not reach these tables** — they are
    migrated by `outbox_migrations()`, and the schema oracle must therefore golden them as
    imported rather than as gear-authored.
  - **`products_outbox` disappears from §4.4**, and with it the only table in this gear whose
    append-only posture C5 never governed.
- **Open, and deliberately not decided here**: which processing mode the registry runs. The
  toolkit offers `transactional` (exactly-once) and `leased` (at-least-once with lease-based
  locking). Publishing to a broker is a network side effect, which argues for `leased`; the PRD
  already accommodates it ("out-of-order/duplicate delivery beyond the idempotency window"), but
  the failure behaviour differs and the choice is the owner's. Registered in slice 01 §6.
- **Pricing is out of scope of this call**: rewriting `pricing_outbox` onto the toolkit is a
  separate task, recorded here only so the divergence is not read as products' error.
- **Propagated**: `design/01-foundation.md` §1.5/§4.4/§4.5.
- **Owed**: `design/10-retention-erasure.md` §3 (the "outbox-delivered" retention class),
  `gears/bss/pricing` (its own rewrite, separate task).

#### P-D-23 — The 2026-08-27 slice-01 owner round: eighteen calls on standing open items

- **Date**: 2026-08-27 (product call, worked through in one sitting during the slice-01 review)
- **Decision**: the open items slice 01 had accumulated over three review passes were decided
  rather than routed. Each call is recorded **inline in the rule it changes** — this entry exists
  so the calls are discoverable from the register and so the ones that reach other slices can be
  checked, not to restate them.

  | Call | Reaches |
  |---|---|
  | The one-code-one-door rule counts pipeline **phases**, not doors or instruction rows | 12 (`inst-cc-errors`), and every slice that declares codes |
  | A refusal's audit row is written in its own transaction and is a **precondition of answering**; 503 if it cannot be written | 05, 10 |
  | **Every publish** bumps `internal_revision` | 05 (approval pinning) |
  | Buckets: `name`/scope columns **iii**, `product_code` **i**, `cloned_from` stricter than **i** | 05 (bucket registry), 03 |
  | The parent guard's arms split by nature: missing ⇒ `VALIDATION`, terminal ⇒ **`PARENT_TERMINAL`** (409) | 12 (error map) |
  | A refusal before the mint carries the attempted natural key; `id`/`revision` absent | — |
  | The idempotency store adopts the donor's **claimed/answered** model; **`IDEMPOTENCY_KEY_IN_FLIGHT`** (409); expiry at claim time | 12 (error map) |
  | `PreAuthorized` is an **internal door argument**, never a wire parameter | 04, 05 |
  | A stray caller-supplied id rides `VALIDATION` | — |
  | A bucket-ii write at the head door is **refused naming 07's door**, not forwarded | 07 |
  | **Engine-canonical serialization is pinned here** (field order, encoding, absent-value form) | 06, 10 |
  | The transition floor records **"no event here"**; the completeness rule widens to *every* slice | 04, 12 |
  | **Discard releases the name**, as it releases codes | — |
  | A **slice-04 validator** reads the retire intent at the create door | 04 |
  | The 422 `MUST NOT` stands as **this gear's choice**, not as an impossibility | 02–12 (the shared note) |
  | The outbox runs **`leased`** (at-least-once); the frame is **`p1`**; `revision` rides the **payload**; slice 03's resolved-binding snapshot is **frozen into the version** | 03, 12 |
  | A SKU's parent link `product_id` is **bucket-i** | — |
  | `payload_hash` is over a **canonical rendering of the parsed request**, not the received bytes | — |

- **Why one entry rather than sixteen**: the register's unit is a decision another document must
  follow, and these are one owner's single pass over one slice's backlog. Splitting them would
  bury the two that actually restructure a contract (`PARENT_TERMINAL` and
  `IDEMPOTENCY_KEY_IN_FLIGHT` enter the SDK's error enum) among fourteen that only settle prose.
- **What this round did *not* settle**: the six items whose owner is not this slice, now filed in
  `PRD` §15 with owners named. *(The last two rows above were taken later the same day, after this
  entry was first written; the entry said they were left open and was corrected 2026-08-28 when the
  fourth review pass caught the register trailing the slice.)*
- **Propagated**: `design/01-foundation.md` (§1.4 roster and every rule the calls change),
  `design/03-sku-classification.md` (`inst-cd-stamp`).
