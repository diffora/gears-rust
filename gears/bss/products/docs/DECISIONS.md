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
- [P-D-24 — The 2026-08-27 fifth-pass round: four calls closing slice-01 open items](#p-d-24--the-2026-08-27-fifth-pass-round-four-calls-closing-slice-01-open-items)
- [P-D-25 — The error contract completed: DUPLICATE_CODE, ENTITY_TERMINAL, AUDIT_UNAVAILABLE, and the audit row's two columns](#p-d-25--the-error-contract-completed-duplicate_code-entity_terminal-audit_unavailable-and-the-audit-rows-two-columns)
- [P-D-26 — Idempotency, identity and the publish bump: four transaction boundaries](#p-d-26--idempotency-identity-and-the-publish-bump-four-transaction-boundaries)
- [P-D-27 — The event contract: HeadSaved, a common body core, the toolkit's seq, and what the third audit class covers](#p-d-27--the-event-contract-headsaved-a-common-body-core-the-toolkits-seq-and-what-the-third-audit-class-covers)
- [P-D-28 — Four read paths the guard needed: the bucket-i writer, the BucketRegistry, the audit row's key, and one canonicalization rule](#p-d-28--four-read-paths-the-guard-needed-the-bucket-i-writer-the-bucketregistry-the-audit-rows-key-and-one-canonicalization-rule)
- [P-D-29 — What a replay, an envelope and a digest actually carry](#p-d-29--what-a-replay-an-envelope-and-a-digest-actually-carry)
- [P-D-30 — Where the gate hosts, where authorization sits, whose validator, and what the door can see](#p-d-30--where-the-gate-hosts-where-authorization-sits-whose-validator-and-what-the-door-can-see)
- [P-D-31 — The four the slice had routed outward, decided here](#p-d-31--the-four-the-slice-had-routed-outward-decided-here)
- [P-D-32 — Six calls closing the slice-01 second lens wave](#p-d-32--six-calls-closing-the-slice-01-second-lens-wave)
- [P-D-33 — Eight calls from weeding slice 01's open items](#p-d-33--eight-calls-from-weeding-slice-01s-open-items)
- [P-D-34 — The remaining slice-01 items, decided from the set](#p-d-34--the-remaining-slice-01-items-decided-from-the-set)
- [P-D-35 — The five slice-01 items the set already forced](#p-d-35--the-five-slice-01-items-the-set-already-forced)
- [P-D-36 — The phase unit is withdrawn; a code's unit is its declaring slice](#p-d-36--the-phase-unit-is-withdrawn-a-codes-unit-is-its-declaring-slice)
- [P-D-37 — One code per audit row, every violation in the answer](#p-d-37--one-code-per-audit-row-every-violation-in-the-answer)
- [P-D-38 — A refusal stores nothing and releases the key](#p-d-38--a-refusal-stores-nothing-and-releases-the-key)
- [P-D-39 — The scope columns, and what the empty set means](#p-d-39--the-scope-columns-and-what-the-empty-set-means)
- [P-D-40 — The entity-version retention DELETE, under a referential predicate](#p-d-40--the-entity-version-retention-delete-under-a-referential-predicate)
- [P-D-41 — The two doors that write bucket-ii](#p-d-41--the-two-doors-that-write-bucket-ii)

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
  - **S3 — Never on the mutation path.** *(Amended 2026-08-27 by P-D-31, closing the conflict
    flagged the same day: S1–S9 are **this gear's** stated requirements on a future platform
    capability, so the amendment is the gear's to make; Architecture / Common Core owns the
    capability's delivery, not this sentence.)* The audit *record* stays local and commits
    inside the guarded mutation's transaction, as v1 already does — **except a refusal's row,
    which commits in its own transaction** (P-D-23, P-D-26), the mutation's being precisely the
    one a refusal rolls back. **What S3 requires is unchanged in both cases**: no audit write
    depends on a network-reachable capability, which is the property "never on the mutation path"
    exists to protect. Only the *seal* is platform-side and
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
    broker event** carrying identity". 04's two are committed and leave no event, so the event
    stream is not their record either. **Found by the third lens on the pass that followed
    this entry, not by the measurement that wrote it.** **Slice 10's erasure act is the
    exception, corrected 2026-08-27**: `inst-er-event` declares that "a minimal
    `ActorErased(actor_ref)` broker event exists as a **defensive cache-buster**", so the act is
    eventless only for events *carrying identity* and sits outside this class — 10's GC deletes
    (`inst-rt-gc`) stay in it. 01 §4.4 already states the membership this way.
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
  (§1.1/§1.5/§1.8 framing, every success-path flow row, §4.4's table scope) *(§6's "three
  registered consequences" was listed until 2026-08-27 and trimmed by P-D-31: the 2026-08-27
  owner round merged that backlog, §6 restates one of them, and a propagation field describes
  what a document says rather than what was intended for it)*, `design/02-taxonomy-attributes.md` (`inst-tx-event`, `inst-gl-atomic`, `inst-ad-event`),
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
- **Left open here, closed by P-D-23 (2026-08-27): the registry runs `leased`.** The toolkit
  offers `transactional` (exactly-once) and `leased` (at-least-once with lease-based locking).
  Publishing to a broker is a network side effect, which argues for `leased`. The PRD does not
  merely tolerate the consequence: `fr-event-versioning-replay` requires that
  "out-of-order/duplicate delivery beyond the idempotency window **MUST** be detectable via
  `(tenant, aggregate, sequence)`" — and the `sequence` operand's home, after this decision
  superseded the `(tenant_id, aggregate_id, sequence)` index, is open in slice 01 §6.
- **Pricing is out of scope of this call**: rewriting `pricing_outbox` onto the toolkit is a
  separate task, recorded here only so the divergence is not read as products' error.
- **Propagated**: `design/01-foundation.md` §1.5/§4.4. *(§4.5 was listed until 2026-08-27 and
  trimmed by P-D-31: it names no outbox facility and restates nothing of this decision, which is
  what `12 inst-cc-register` lints for.)*
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

- **Why one entry rather than eighteen**: the register's unit is a decision another document must
  follow, and these are one owner's single pass over one slice's backlog. Splitting them would
  bury the two that actually restructure a contract (`PARENT_TERMINAL` and
  `IDEMPOTENCY_KEY_IN_FLIGHT` enter the SDK's error enum) among sixteen that only settle prose.
- **What this round did *not* settle**: the six items whose owner is not this slice, now filed in
  `PRD` §15 with owners named. *(The last two rows above were taken later the same day, after this
  entry was first written; the entry said they were left open and was corrected 2026-08-28 when the
  fourth review pass caught the register trailing the slice.)*
- **Propagated**: `design/01-foundation.md` (§1.4 roster and every rule the calls change),
  `design/03-sku-classification.md` (`inst-cd-stamp`).

#### P-D-24 — The 2026-08-27 fifth-pass round: four calls closing slice-01 open items

- **Amended 2026-08-28 by P-D-36**: the door→phase amendment below is withdrawn. A code's unit is
  its **declaring slice**, not a pipeline phase; §3.1's seven phases remain the execution order.
  The reason this entry gave for abandoning the *door* unit stands and is why the slice unit was
  chosen. Every other call here — the `state` phase itself, `shape`'s scoping, the status move and
  the frozen-content exclusions — is about execution order or status and is untouched.

- **Context**: the fifth review pass over `design/01-foundation.md` (three lenses, two passes)
  left four questions whose owner is this slice. Recorded as one entry for the same reason
  P-D-23 was: the register's unit is a decision another document must follow, and these four
  were taken together in one sitting.

  | Call | Propagation |
  |---|---|
  | A **`state` phase** joins the pipeline **after `shape`** — the §2 edge list, bucket routing for a published-state field write, and the parent's own lifecycle state. It raises `ILLEGAL_TRANSITION`, `ILLEGAL_FIELD_MUTATION` and `PARENT_TERMINAL`, which until now belonged to no phase while §3.3 required exactly one per code. It sits **after** `shape` because the parent's state can only be judged once the reference naming it has resolved | 12 (the door→phase amendment) |
  | **`shape` is scoped to include reference resolvability** — an unresolvable `productId` is a defect of the payload, which is why §2 already routes it to `VALIDATION`. This keeps `VALIDATION` single-phase and closes the second-raiser question the `state` phase would otherwise have opened | — |
  | **`PARENT_NOT_PUBLISHED` is 409**, not 422: it is a refusal by the parent's current state, which is what §3.3's own discriminator assigns to 409 — the same reading that already put `PARENT_TERMINAL` there | 04 |
  | **`brand_id` is bucket-i** structural identity — re-branding moves the row into a different `(tenant_id, brand_id, name_normalized)` scope, the key §4.1's partial unique index enforces on; it joins `sku_code`, `product_code` and the SKU→parent link | — |
  | **`lifecycle_state`, `deprecation_provenance` and `replaced_by_sku_id` are excluded from frozen version content.** They move on transitions, which write no version row, so freezing them would require the digest to change on a write that produces no row to digest. They are read from the head row, as §4.3 already routed them | 08, 10 (the digest's column set) |

- **Why the `state` phase rather than widening `identity` or adding carve-outs**: all three codes
  are judged from the row as it now stands rather than from the payload, which is what separates
  them from `identity`; and widening the carve-out list from one to four would have made the
  "exactly one raising phase" rule vacuous rather than satisfied.
- **What this round did *not* settle**: whose arm `RETIREMENT_PENDING`'s create-door check is —
  01 reads it as a slice-04 validator and 04 reads it as 01's own guard. Still open in slice 01 §6
  and in 04's, with owners named.
- **Propagated**: `design/01-foundation.md` (§1.7, §2 publish row, §3.1, §3.3, §4.1, §4.3),
  `design/04-lifecycle.md` (§3.2 problem-response map).
- **Restated by** (2026-08-27, all three closed): `design/12-consumer-contracts.md`
  (`inst-cc-errors`, now phase-shaped), `design/08-read-models.md` (C6) and
  `design/10-retention-erasure.md` (the drill).

#### P-D-25 — The error contract completed: DUPLICATE_CODE, ENTITY_TERMINAL, AUDIT_UNAVAILABLE, and the audit row's two columns

- **Context**: the fifth review pass left four gaps in the error contract — two refusals a door
  can take with no code to carry them, one status with no code at all, and an audit row that
  could not record either the code or the key it refused on. Taken together because they are one
  contract and one consumer reads all of it: the SDK's error enum and AC #38's map.

  | Call | Propagation |
  |---|---|
  | **`DUPLICATE_SKU_CODE` becomes `DUPLICATE_CODE`**, covering both reservations. §2 already says `productCode` reserves "under the same rules as `skuCode`", so one rule carries one code; the SKU-named form was declared before `productCode` had an index of its own. **This is a rename, and §3.3 states renames are breaking** — taken now precisely because nothing is built yet | 09, 11 |
  | **`ENTITY_TERMINAL` (409)** — a save on a `retired`/`discarded` head. The subject's own terminal state refusing the write, exactly as `PARENT_TERMINAL` is the parent's; both sit in the `state` phase (P-D-24). Without it an ordinary operator mistake reached the trigger and answered a bare 500 | 12 (error map) |
  | **`AUDIT_UNAVAILABLE` (503)** — the refusal's audit row could not be written, so the door cannot report the domain refusal (§4.4). Names the condition rather than the mechanism, matching 08's `READ_MODEL_OVERLOADED`, the gear's only other 503 | 12 (error map) |
  | **`products_audit_log` gains nullable `error_code` and `attempted_key`.** §3.1 makes the code the attribution channel ("never the rule name") and AC #38 maps by it, so it is a column rather than free text; `attempted_key` carries the natural key a pre-mint refusal has in place of an id, which `DUPLICATE_NAME` and `DUPLICATE_CODE` both need | 10 (the audit class its `RetentionClock` reads) |

- **Why `DUPLICATE_CODE` rather than a second, product-named code**: the alternative kept
  `DUPLICATE_SKU_CODE` and added `DUPLICATE_PRODUCT_CODE`, which avoids a breaking rename but
  writes the same rule twice in the enum and leaves a reader asking which applies to a clone that
  suggests both. The owner took the rename while the contract is still unbuilt.
- **What this entry does *not* settle**: what addresses a `products_audit_log` row, so the
  sealing seam's one-way UPDATE can target one — still open in slice 01 §6 with P-D-08's owner.
- **Propagated**: `design/01-foundation.md` (§2, §3.3, §4.4),
  `design/09-bulk-promotion.md` and `design/11-clone.md` (the renamed code).
- **Restated by** *(owed until 2026-08-27, all closed)*: `design/12-consumer-contracts.md` (`inst-cc-errors`' map gains two codes and a 503
  class), `design/10-retention-erasure.md` (the audit roster its retention class reads).

- **Amended 2026-08-28 by P-D-38**: the "a refusal answers the key" call below is withdrawn. A
  refusal stores nothing and releases the key; a retry runs. The other three boundaries stand.

#### P-D-26 — Idempotency, identity and the publish bump: four transaction boundaries

- **Context**: four fifth-pass items that all turned on the same unstated thing — which
  transaction a write commits in, and what happens when the process holding it dies. None could
  be built without an answer, and the donor (pricing) declares the `IDEMPOTENCY_KEY_IN_FLIGHT`
  code but not the boundary, so "adopting the donor's model whole" inherited no answer here.

  | Call | Propagation |
  |---|---|
  | **The `claimed` idempotency row commits in its own transaction**, ahead of the guarded operation — sharing the mutation's would make it invisible to the concurrent duplicate the row exists to refuse, and `IDEMPOTENCY_KEY_IN_FLIGHT` could never fire | — |
  | **A refusal answers the key.** A refused request is a finished request, so the door sets `answered` with the refusal as the stored outcome and a retry replays it rather than re-running a rule whose verdict cannot change. `claimed` therefore means exactly "in flight", and the §4.4 CHECK stays true | 12 (replay semantics) |
  | **A crashed claim is released by `in_flight_until`**, a new nullable column distinct from `expires_at`'s retention window. Retention is the answered key's; the in-flight deadline is short, and without it the only exit was `max(24h, max_freeze_timeout)` — a legitimate retry refused 409 for a day | — |
  | **A non-wire caller writes a reserved lane name in `endpoint`** — `internal:scheduled-activation`, `internal:cascade-leg`, `internal:bulk-row` — and its own id in `client_key`. Two internal lanes cannot collide on one key, and the `internal:` prefix cannot collide with a wire endpoint | 04, 09 |
  | **The first-appearance `actor_ref` mint commits in its own transaction, ahead of the guarded operation.** A refusal rolls the door's transaction back while the refusal's audit row commits independently and requires an `actor_ref`; a ref is a pseudonym rather than a domain record, so minting one for a principal that was then refused costs nothing | 10 |
  | **A first publish bumps `internal_revision` once and fires no invalidation hook.** The publish door is one act, not a transition plus a publish: it owns the `draft→published` edge, so the transition guard's "every transition" does not reach it, and a hook firing against the record the same transaction consumes has no defined ordering | 05 |

- **What this entry did *not* settle, both closed later the same day by P-D-29**: what the stored
  outcome dereferences to (answer: the donor's `response_status`/`response_body`, replacing the
  single `outcome_ref` this gear had imported), and whether the `internal_revision` on an event or
  audit row is the value before or after the act's own bump (answer: after — as committed).
- **Propagated**: `design/01-foundation.md` (§2 create and publish rows, §2 transition guard,
  §3.2, §4.4).
- **Restated by** *(owed until 2026-08-27, all closed)*: `design/04-lifecycle.md` and `design/09-bulk-promotion.md` (the lane names their
  runners write), `design/05-governance.md` (the hook's non-firing on the door's own edge),
  `design/10-retention-erasure.md` (the mint's transaction), `design/12-consumer-contracts.md`
  (a replayed refusal is part of the consumer contract).

#### P-D-27 — The event contract: HeadSaved, a common body core, the toolkit's seq, and what the third audit class covers

- **Context**: `design/08-read-models.md`'s projector and `design/12-consumer-contracts.md`'s SDK
  contract could not be built — §4.5 declared eight events whose bodies no document specified,
  under a name that had been false since the H1 fix. Taken with the two remaining event-plane
  questions from the same pass.

  | Call | Propagation |
  |---|---|
  | **`ProductDraftSaved`/`SkuDraftSaved` become `ProductHeadSaved`/`SkuHeadSaved`.** The H1 fix made the head the authoring surface in every non-terminal state, so the old name was false for `published` and `deprecated` — two of the three. A rename, taken while nothing is built, on the same reasoning as P-D-25's | 02 |
  | **Every one of the eight carries a common body core**: `{tenantId, entityKind, entityId, internalRevision, lifecycleState}`. `lifecycleState` is the discriminator a `*HeadSaved` consumer needs. `*Published` additionally carries **`publishedVersion`** — what 06 reads as content and 08's projector keys on. Anything beyond the core is named where the act is specified, as 04 already does for `SkuRetired` | 06, 08, 12 |
  | **The envelope carries the toolkit outbox's `partition_id` and `seq`**, which the processor already hands the handler (`libs/toolkit-db/src/outbox/handler.rs`'s `OutboxMessage`). Since `partition = hash(tenant_id, aggregate_id) mod N`, every event of one aggregate shares a partition and `seq` is monotonic within it — which satisfies `fr-event-versioning-replay`'s "detectable via `(tenant, aggregate, sequence)`" without restoring the index P-D-22 superseded. Detectability needs monotonicity, not density; gaps left by neighbouring aggregates are harmless | 12 |
  | **§4.4's third audit class covers *domain* acts** — one over a `Product`/`SKU` or a governed record. A door's own infrastructure writes are outside it, which is why `inst-fd-actor-ref` and `inst-fd-idem-claim` declare "no event" without also writing an audit row. Read literally the class would have put an audit row behind every ref resolution | 10 |

- **Why the rename rather than a payload field alone**: a `lifecycleState` field fixes what a
  consumer can *tell* but leaves the event's name asserting something false, and the name is what
  a reader of the §9.2 outbound contract meets first.
- **Propagated**: `design/01-foundation.md` (§2 save row, §4.4 payloads, §4.5),
  `design/02-taxonomy-attributes.md` (the renamed event its attribute writes ride).
- **Restated by** *(owed until 2026-08-27, all closed)*: `design/06-catalog-version.md` and `design/08-read-models.md` (the body core their
  consumers read), `design/12-consumer-contracts.md` (`inst-rc-dedup` re-based on the published
  `seq`, and the replay contract against the body core), `design/10-retention-erasure.md` (the
  audit class's scope).

#### P-D-28 — Four read paths the guard needed: the bucket-i writer, the BucketRegistry, the audit row's key, and one canonicalization rule

- **Context**: four fifth-pass items that each named something the design *used* without saying
  where it came from — a write with no admitted writer, a tag with no read path, a row with no
  address, and a rule claimed to be shared that was defined over only one of its two subjects.

  | Call | Propagation |
  |---|---|
  | **The save door writes bucket-i columns while `published_version = 0`**, and nothing writes them after. §4.2's whitelist named an admitting door for every other class and only a prohibition here, so the `skuCode`/`productCode` change §2 makes legal on an unpublished head had no admitted writer. No new door: `inst-fd-save-txn` already carries it | — |
  | **`BucketRegistry` is a Foundation artifact**, named in §1.7 beside `RegisteredValidator`: a slice registers its columns' bucket tags exactly as it registers validators — code, not config — and 05 reads the same registry to judge materiality. 05 already attributed the frame here, and a physical guard of the Foundation's cannot depend on a capability slice's artifact (§1.1) | 05 |
  | **`products_audit_log` gains `audit_id` (PK, uuid).** The sealing seam's one-way UPDATE must address a row that is *not yet* sealed, and `seq` is null until it is; the surrogate is independent of the chain's ordering, and matches the uuid-PK convention of every other §4 table | 10 |
  | **The canonical rendering is stated over any named field set**, not only a version row's columns — "sorted lexicographically by **field** name". That is what lets §3.2 hash a parsed request under the same rendering and makes its "one such rule and not two" true; mechanically the rule was already field-shaped | 06 |

- **What this entry does *not* settle**: how a **row collection** inside a frozen version (the
  category-assignment set, the attribute-value set) is ordered for the digest — the rule orders
  fields, not rows. Still open in slice 01 §6 with 02 and 10.
- **Propagated**: `design/01-foundation.md` (§1.7, §4.2, §4.3, §4.4).
- **Restated by** *(owed until 2026-08-27, all closed)*: `design/05-governance.md` (`BucketRegistry` by name where it reads the tags),
  `design/10-retention-erasure.md` (the audit row's key), `design/06-catalog-version.md`
  (the canonicalization rule its `inst-sn-checksum` points at).

#### P-D-29 — What a replay, an envelope and a digest actually carry

- **Context**: four fifth-pass items that each named a value the design relied on without saying
  what it contained. Two of them were left open by P-D-26 earlier the same day; the other two are
  what 10's restore drill compares byte-for-byte.

  | Call | Propagation |
  |---|---|
  | **`outcome_ref` is replaced by the donor's two columns, `response_status` and `response_body`.** A replay must reproduce the original response *including its status*, which a bare reference to an entity cannot do — and after P-D-26 made a refusal answer the key, a refusal has no entity to reference at all. §3.2 already said the model was adopted from the donor "whole"; the single reference was the divergence, and it was the part that did not work | 12 (replay contract) |
  | **The `internal_revision` on an envelope or audit row is the value *as committed by the act*** — N+1 where the act bumped it, the unchanged current value where it did not. The event is P-D-21's record of a *committed* act, so the number describes the state the act left behind and matches the caller's next ETag. "At the act" had admitted both readings | 12 |
  | **The digest is SHA-256**, with a **`digest_version`** column beside it on `products_entity_version`. `sha2` is already a workspace dependency (`Cargo.toml`), and §4.3's "adding a column is a digest-version bump, not a silent change" is only checkable if the version a row was computed under is stored on that row | 10 |
  | **A row collection inside frozen content is a JSON array sorted by the collection's own identifier** — the category id, the attribute id — each element rendered by the same field rule. P-D-28's rule orders *fields* and said nothing about *rows*, so two engines could have serialized one content in two orders | 02, 10 |

- **Why the donor's two columns rather than a reference to the audit row**: under P-D-21 a
  successful act writes no audit row at all, so that reference would have had nothing to point at
  on exactly the path replay matters most.
- **Propagated**: `design/01-foundation.md` (§3.2, §4.3, §4.4).
- **Restated by** *(owed until 2026-08-27, all closed)*: `design/12-consumer-contracts.md` (the replay contract and the committed-revision
  reading), `design/10-retention-erasure.md` (the digest's algorithm and version column its drill
  compares), `design/02-taxonomy-attributes.md` (the ordering of its two collections).

#### P-D-30 — Where the gate hosts, where authorization sits, whose validator, and what the door can see

- **Context**: four fifth-pass items about *who runs what, where*. Two were scoping errors in
  §3.1, one was a standing disagreement between 01 and 04 over the same code, and one asked the
  Foundation to read an operand it cannot have.

  | Call | Propagation |
  |---|---|
  | **The governance-gate phase runs on any gated act, not publish alone.** 05 words the obligation generically — "submit → quorum → publish/apply" over both entity publishes and `GovernedLiveOp`s — and 04's un-deprecation is two-person with a slice-05 gate registered on that edge. A transition door consumes the `satisfied` record exactly as the publish door does; scoping the phase to publish left that ceremony with a gate no phase hosted | 04, 05 |
  | **Authorization is not a phase — it is a pre-pipeline gate**, run before the pipeline opens. The only order in which a denied caller neither consumes an idempotency key nor writes a claim row, and the order §2's flows already use. Its refusal code is 05's, RBAC grants being that slice's (§1.5) | 05 |
  | **Both arms of `RETIREMENT_PENDING` are slice-04 validators** — the un-deprecation edge, and 04's validator registered on 01's create door. The operand is the live retire intent in `products_scheduled_transition`, a table 04 owns; reading it in the Foundation would put the floor in the business of lifecycle policy against §1.1. 04's contrary note is corrected. Both arms therefore sit in the **registered validators** phase and the code needs no carve-out | 04 |
  | **The `PublishDoor` sets `composition_pending` when the publish carried the two-person uncomposed-bundle override**, not when the bundle "is uncomposed". Whether plan-price has composed it is 03's validator's judgement (`BUNDLE_OVERRIDE_REQUIRED`); what the door sees is whether *this* publish carried the override. It also removes the re-raise for free: 06's clearing re-publish is a `system_signal` subject carrying no override, so the predicate is false and the flag stays cleared | 03, 06 |

- **Why the override rather than the composition itself**: §1.1's "the Foundation owns no
  capability policy" is load-bearing, and a validator refuses rather than writing a column (§3.1) —
  so neither side could own the write until the predicate was restated in terms the door can see.
- **Owed by slice 04**: the instruction row registering the create-door validator. Until it
  exists nobody builds the guard, and item 36's hole stays open.
- **Propagated**: `design/01-foundation.md` (§1.7, §2 create row, §3.1, §4.2),
  `design/04-lifecycle.md` (§3.2 code contract, open items),
  `design/03-sku-classification.md` and `design/06-catalog-version.md` (the override-shaped
  predicate, restated at `inst-cl-bundle-override` and `inst-cc-clear`).
- **Restated by** *(owed until 2026-08-27, all closed)*: `design/05-governance.md` (the gate phase's wider scope, and the pre-pipeline
  authorization gate its denial code answers to).

- **Amended 2026-08-28 by P-D-40**: "the guard judges the row image" is widened to "the guard
  judges the **data**". The objection recorded below is to a guard reading the *door* through a
  session variable that exists on Postgres and not SQLite; a predicate that reads another table
  judges data and both engines evaluate it. `products_entity_version`'s retention DELETE is
  admitted under exactly such a referential predicate.

#### P-D-31 — The four the slice had routed outward, decided here

- **Context**: the fifth pass filed four items with owners outside this slice. Two of those
  routings were wrong on inspection — **P-D-08's S1–S9 are this gear's own stated requirements**
  on a future platform capability, and the wire shape of this gear's primary surface is nobody
  else's — and the remaining two were decidable from constraints already in the set. Where an
  answer binds another team, it binds as a **stated requirement on them**, not as a fact about
  their artifact.

  | Call | Propagation |
  |---|---|
  | **P-D-08 S3 is amended**: the audit record commits inside the guarded mutation's transaction **except a refusal's row, which commits in its own** (P-D-23, P-D-26) — the mutation's being the one a refusal rolls back. What S3 requires is unchanged either way: no audit write depends on a network-reachable capability, which is the property "never on the mutation path" protects. Architecture / Common Core owns the capability's *delivery*, not this sentence | — |
  | **The head-row guard judges the row image, never the door.** A session variable exists on Postgres and not on SQLite, so a door-reading guard breaks C1 in both halves — dual-engine, and "guards defined once". Which door wrote is an **application** guarantee; the trigger enforces which column may move in which state. §4.2's per-door clauses are restated accordingly, and §3.1's claim that the physical guard enforces "one door, one effect" is corrected to name the application as the enforcer and the guard as the backstop | 07 |
  | **The gear's primary wire surface is declared** in §2: `POST …/products`/`…/skus` → 201; `PATCH …/{id}` (If-Match required) → 200; `POST …/{id}/publish` → 200; `POST …/{id}/discard` → 200; `Idempotency-Key` on every mutating door. Paths follow the form 02/05/06/08/09/10/11 already use; the transition floor has no wire door of its own | 12 (door×grant lint), 05 (RBAC catalog) |
  | **P-D-21's and P-D-22's propagation fields are trimmed to what the documents actually restate** — P-D-21 loses "§6's three registered consequences" (the 2026-08-27 round merged that backlog; §6 restates one), P-D-22 loses §4.5 (which names no outbox facility). A propagation field describes what a document says, not what was intended for it | — |

- **Owed**: the row-image predicates that would let the trigger tighten two clauses it now admits
  on the application's word — **04's** for `deprecation_provenance`/`replaced_by_sku_id`, and
  **07's** for bucket-ii columns. Until those exist the guard is a backstop there rather than a
  proof.
- **Propagated**: `design/01-foundation.md` (§2 doors, §3.1, §4.2). *(The S3 amendment and the
  two trimmed propagation fields are edits to this register itself, not propagation out of it.)*


#### P-D-32 — Six calls closing the slice-01 second lens wave

- **Amended 2026-08-28 by P-D-36**: the call scoping the "exactly one raising phase" rule to codes
  raised *inside* the pipeline is withdrawn with the rule itself. Its other five calls stand,
  including the target-state registration key, which is about *when* a validator runs rather than
  about code attribution.

- **Date**: 2026-08-27 (owner call, on the second three-lens pass over `design/01-foundation.md`)
- **Context**: the second pass raised twelve questions. Three merged into items the first pass had
  already registered, one was closed by the same pass's fix to the audit-seal predicate, and two
  are slice 12's. The six below were decidable from constraints already in the set, and are
  recorded here rather than inline because four of them bind another document.

  | Call | Propagation |
  |---|---|
  | **`composition_pending` is cleared by the publish door's own head-row UPDATE** — the one carrying `published_version += 1`. §4.2 admits the flag's change only in that statement, and `inst-fd-save-txn` never touches `published_version`, so a save cannot clear it. 06's "system save + re-publish" names the ceremony, not the writing statement | 01 §4.2; 06 `inst-cc-clear` |
  | **The `BucketRegistry` is advisory for the physical layer.** A compile-time Rust map has no read path from a migration-time trigger, so §4.2's column classes stay static DDL; generating them from the registry would break C1's "guards defined once" and the schema-oracle goldens. A test asserts the two name the same columns in the same classes | 01 §1.7, §5 |
  | **`→ published` in a validator's registration key names the target state, not the edge.** A re-publish takes no edge, so an edge-keyed reading runs no validator at all and empties the fail-closed re-run — while also pulling the `deprecated→published` two-person ceremony onto a content re-publish that changes no state | 01 §2 (`inst-fd-publish-revalidate`) |
  | **`ILLEGAL_TRANSITION` and `ILLEGAL_FIELD_MUTATION` move 422 → 409.** All four codes the `state` phase raises are refusals by the row's **current state**, which is §3.3's own 409 rule; splitting them left one phase straddling two status classes. Wire-visible — a 422 reaches the wire as 400 — and taken while nothing is built | 01 §3.3; 07 (status repeat) |
  | **`ENTITY_TERMINAL` covers any head write on a `retired`/`discarded` row** — save, publish or correction. The publish door's accepted set excludes a `retired` head and `ILLEGAL_TRANSITION` cannot cover it, a re-publish being no edge | 01 §3.3 |
  | **The "exactly one raising phase" rule ranges over codes raised *inside* the pipeline.** Authorization is a pre-pipeline gate (P-D-30), so 05's owed denial code and `BREAKGLASS_WRITE_FORBIDDEN` sit outside the rule instead of forcing a third carve-out; the carve-out list stays closed at two | 01 §3.3; 12 `inst-cc-errors` |

- **Left open, registered with their owners**: 12 `inst-cc-errors` is owed **both** carve-out
  members, not one — it names none today — and `inst-cc-ids`' enumeration of continued ids is a
  stale count against 01. Both are slice 12's.
- **Propagated**: `design/01-foundation.md` (§1.7, §2, §3.3, §4.2, §5, §6),
  `design/06-catalog-version.md` (`inst-cc-clear`), `design/07-reference-signal.md` (the
  `ILLEGAL_FIELD_MUTATION` status repeat), `design/12-consumer-contracts.md` (open items).


- **Amended 2026-08-28 by P-D-37**: the stop-at-first-phase call below stands. What it did not
  say is what happens when one phase collects more than one *code* — the caller's rejection now
  carries them all and the row records one, by a precedence §3.3 pins for the only phase that can.

#### P-D-33 — Eight calls from weeding slice 01's open items

- **Date**: 2026-08-27 (owner call, on weeding `design/01-foundation.md` §6 after four lens passes)
- **Context**: four passes registered questions and only one round closed any, so §6 had grown to
  22 items and 18% of the file against a sibling maximum of 14%. Weeding merged four thematic
  groups (22 → 16) and found eight items already decidable from constraints the set had fixed
  elsewhere. The eight below are those; the remaining ten need input this design set does not hold.

  | Call | Propagation |
  |---|---|
  | **A read door is declared**: `GET /bss-products/v1/{products\|skus}/{id}` (`… × read`) returns the head with its internal revision as `ETag`. `inst-fd-etag` requires a precondition that no surface returned — 08's projections serve frozen content and expose no head revision, so an author who had not just written could obtain none | 01 §2 |
  | **The publish door's pinned revision arrives as `If-Match`**, like every other head verb's, rather than as an unnamed door argument with no wire carrier | 01 §2 |
  | **The freeze captures the post-act image** — including the `composition_pending` value the same UPDATE is about to write. The version row's key already carries `published_version = N+1`, so freezing the pre-UPDATE image would store content the act never produced and put the digest and 10's byte-for-byte restore drill on different bytes | 01 §2 |
  | **`digest_version` starts at `1`**, pinned as a code constant by §5's golden vector rather than by config — the vector is already owed by that section, so no second carrier is introduced | 01 §4.3 |
  | **The pipeline stops at the first failing phase**, collecting violations per-field *within* that phase. §4.4's audit row carries a single `error_code`, so collecting across phases would produce more codes than the row can record | 01 §3.1 |
  | **"One phase, one status class" is a rationale, not an invariant.** P-D-32 used it to move two codes; read as a rule it would force `SCOPE_NOT_CONTAINED` to 409, contradicting §3.3's own "422 for content the door cannot process". The `identity` phase legitimately spans both | 01 §3.3 |
  | **An absent `If-Match` rides `VALIDATION`**, not the bare 400. The request parsed, which is `inst-fd-mint-id`'s own criterion for what the malformed-request 400 does not cover | 01 §2 |
  | **`inst-fd-save-txn` is the admitting door for every bucket-i column while `published_version = 0`** — `skuCode`/`productCode`, `brand_id` and a SKU's parent `product_id` — and **`brand_id` is a required payload field validated against the caller's brand claims**, refused `VALIDATION` when it names a brand the caller does not hold. §4.2 admitted the class with no door claiming it, and no step assigned the column at all; silent derivation breaks on a principal holding more than one brand | 01 §2, §4.1 |

- **Left open**: ten items, each needing input outside this design set — the retention DELETE arm
  (10 and the retention-duration owner), the interim trigger predicates owed by 04 and 07, the gate
  phase's reach (05), `SCOPE_NOT_CONTAINED`'s phase and both codes' declaring slice (04), the
  idempotency store's seven unpinned operands, the event-declaration criterion (12), and the
  identity columns' bucket (the PRD owner who holds the matrix).
- **Propagated**: `design/01-foundation.md` (§2, §3.1, §3.3, §4.1, §4.3, §6).


#### P-D-34 — The remaining slice-01 items, decided from the set

- **Date**: 2026-08-27 (owner call, after P-D-33's weeding left ten items)
- **Context**: the ten were filed as "needing input this design set does not hold". On inspection
  that was true of two — the envelope slot (the event-broker's contract) and the retention
  *durations* (Legal/Finance), both already in `PRD` §15. The rest named owners — "this slice",
  "the governance owner", "whoever took P-D-24", "the `nfr-availability-audit` owner" — who are
  this set's own. Nine are decided here, plus five of the idempotency store's seven operands.

  | Call | Propagation |
  |---|---|
  | **The no-hook exception reaches any transition that consumes an approval in the same transaction**, not `draft→published` alone — 05 C3's own reason (a hook firing against the record the act is consuming has no defined ordering) applies wherever P-D-30 put the gate. And **the gate phase passes trivially on an ungated act**: a head save invalidates approvals, never consumes one, so `Gate` mode imposes no approval requirement on create, save or discard | 01 §2, §3.1; 05 C3 |
  | **A read under elevation commits its audit row in its own transaction, as a precondition of serving the read** — P-D-08 S3 is scoped to "the guarded mutation's transaction" and a read has none | 01 §4.4 |
  | **A parsed request's named field set is the fields the request carries.** §4.3's "absent written `null`" addresses a *complete* set, so an omitted field and an explicit `null` hash differently — which is what they mean at the head door | 01 §4.3 |
  | **The event-declaration unit is the act, not the row**: a step inside a transaction whose event another row of that transaction names inherits the declaration | 01 §4.5; 12 (completeness check) |
  | **04's final containment rule replaces the operand inside 01's `identity` phase** rather than registering a slice-04 validator — the literal reading of C5's "the final form of 01's interim check", and the only one under which the code keeps one raising phase | 01 §3.3; 04 C5 |
  | **`→ published` names the publish act, not the row's `lifecycle_state` afterwards** — the door accepts a `deprecated` head for N+1 and leaves it `deprecated`, so a state-after reading selects nothing there | 01 §1.7, §2 |
  | **`AUDIT_UNAVAILABLE` is carved out of the audit class** (the class, not §3.3's phase list): its own row is the one that could not be written. Recorded out-of-band; `nfr-availability-audit`'s "100%" is scoped to domain refusals | 01 §4.4 |
  | **`RETIREMENT_PENDING` is declared by 04** and listed in 01 for the response map only — P-D-30 gave 04 both arms, so 01 raises neither. **`SCOPE_NOT_CONTAINED` stays 01's**, with 04 carrying the reciprocal "named in 01, registered here" | 01 §3.3; 04 |
  | **Four row-image predicates the first migration's trigger was missing**: `deprecation_provenance` only in the same statement as a `lifecycle_state` change; `replaced_by_sku_id` **write-once** (04: "Validated once, and the row is terminal at the flip"); bucket-ii only in the same statement as a `published_version` bump (07 defines its `CorrectionDoor` as ending in a re-publish); and **row identity — `tenant_id`, the PK, `created_by` — admitted in no UPDATE at all**, `cloned_from`'s treatment rather than the PRD matrix's bucket-iv catch-all, which that FR words as "other *descriptive* fields". Plus a **retention DELETE arm** predicated on `written_at` past the class's window, so 10 `inst-rt-gc` has an admitted path; the window's value stays Legal/Finance's | 01 §4.2, §4.4 |

- **Five of the idempotency store's seven operands**, in §3.2: a keyless request runs unguarded (the
  PRD scopes the guarantee to requests *with* a key); `AUDIT_UNAVAILABLE` is **not** stored as an
  answer, being the one refusal whose verdict can change; the payload hash covers the body and not
  the precondition; `expires_at` is stamped at the claim INSERT; and `answered` joins the
  mutation's transaction on success, its own on a refusal.
- **Left open**: `in_flight_until`'s value, which has no anchor until a door timeout is pinned, and
  what the three `internal:` lanes store in the response columns — three workable shapes, none
  following from what the set fixes.
- **Propagated**: `design/01-foundation.md` (§1.7, §2, §3.1, §3.2, §3.3, §4.2, §4.3, §4.4, §4.5,
  §6), `design/04-lifecycle.md` (both codes' reciprocal qualifiers).


#### P-D-35 — The five slice-01 items the set already forced

- **Date**: 2026-08-28 (owner call, after the sixth and seventh lens passes)
- **Context**: the seventh pass left twelve open items in `design/01-foundation.md` §6. Five of
  them were not open in the sense the other seven are: for each, this set already held a rule, a
  precedent or a reciprocal claim that made one answer the only consistent one, and the item was
  open only because nobody had said so out loud. Those five are decided here. The remaining seven
  are genuine forks — each reopens something if answered the other way — and stay in §6 with their
  owners named.

  | Call | Propagation |
  |---|---|
  | **`internal_revision` joins §4.3's frozen-content exclusions**, and the digest column is named **`content_digest`**. The exclusion criterion is a column that moves on a transition writing no version row; `inst-fd-transition-bump` bumps `internal_revision` on **every** transition, so it met the criterion and was simply missing from the enumeration. Without the name, §5's golden vector and 10's restore drill both address a column the schema never declares | 01 §4.3, §5; 10 `inst-rd-drill` |
  | **`composition_pending` is `NOT NULL` with default `false`.** The create flow writes it nowhere and the publish door on a `bundle` is its only raiser, so the default is the unraised state — the one value under which 11's **Reset** has a meaning and the first migration does not need a nullable third reading | 01 §4.2; 11 (the clone Reset row); 06 (semantics owner) |
  | **`REVOKE` is a Postgres-only arm; the trigger whitelist is the whole guard on SQLite.** SQLite has no `GRANT`/`REVOKE`, so one migration cannot carry that arm on both engines, and C1 requires both dual-engine and "guards defined once". This is **P-D-31**'s reasoning applied a second time: where a mechanism exists on one engine only, the guard is the row-image trigger and the rest is an application or deployment guarantee. The schema-oracle goldens differ by exactly this statement, and the difference is now stated rather than discovered | 01 §1.6 C5, §4.4; 05 C7 |
  | **A 404 is bare, carrying no registry code** — the reading §3.3 already applies to the bare 400. A path segment is judged before the pipeline opens, so no phase raises it; the governing `.cf-studio/config/rules/api-contracts.md` pins no code for it; and giving it one would require a raising phase this taxonomy cannot supply without reopening the one-phase rule | 01 §3.3; 12 (AC #38 map unchanged — nothing is added to it) |
  | **The declaration rule: the slice that names a code for its response map holds the declaration unless the register moves it.** P-D-34's "raises neither and cannot hold the declaration" was a call about `RETIREMENT_PENDING`, not a general test — read generally it also selects `PARENT_NOT_PUBLISHED`, which 01 declares and 04 twice records as declared in 01. Narrowing the wording leaves both codes exactly where the set already puts them | 01 §3.3; 04 (its two reciprocal lines already agree) |

- **Left open**: the other seven §6 items, none of which the set forces — what the "exactly one
  raising phase" rule ranges over (raised independently by all three lenses); the idempotency
  store's three unpinned operands (`in_flight_until`'s value, what the `internal:` lanes store in
  the response columns, and whether `endpoint` is a route template or a concrete path); whether a
  stored refusal replays or re-runs; one `error_code` against several codes in one phase, and the
  same item's write-side half; which door writes bucket-ii on either side of first publish; the
  scope columns no door writes; and `products_entity_version`'s missing DELETE arm against 10's
  retention GC.
- **Owed elsewhere**: `PRD` §15/§16 still word the interim audit control as "`REVOKE` + trigger
  whitelist" without the engine split — precision, not a contradiction, and the PRD owner's to
  take.
- **Propagated**: `design/01-foundation.md` (§1.4, §1.6 C5, §3.3, §4.2, §4.3, §4.4, §6),
  `design/05-governance.md` (C7), `design/10-retention-erasure.md` (`inst-rd-drill`),
  `design/11-clone.md` (the clone-disposition table).


#### P-D-36 — The phase unit is withdrawn; a code's unit is its declaring slice

- **Date**: 2026-08-28 (owner call, taken against the donor's code rather than against this set)
- **Context**: §3.3 required every code raised inside the pipeline to belong to exactly one of
  §3.1's seven phases, with a carve-out list for the ones that could not. All three lenses of the
  seventh pass raised the same contradiction independently: an absent `If-Match` rides
  `VALIDATION`, but header presence can only be judged in the `precondition` phase while
  `VALIDATION` is `shape`'s — and the carve-out list "closes at two" while both of its members are
  stated to be raised outside every phase, exactly as **P-D-32** reasons the authorization-gate
  codes are, on which reading it closes at zero.

  **The question was settled by measurement, not by picking an arm.** `gears/bss/pricing` is the
  gear whose validation pipeline this set copied, and it is built as well as designed. Its shared
  `ValidationPipeline<S>` registers rules and returns a `ValidationReport`; its rules "append and
  never short-circuit"; its codes are `const`s on the rules that raise them; everything that is not
  a rule — `StaleVersion`, `NotFound`, `ConcurrentMutation`, `LifecycleForbidden` — is an early
  `Err` at the point of detection. **It carries no notion of a validation stage at all**, and
  `phase` in that gear names a plan phase. The phrase "one raising phase" occurs nowhere in the
  repository outside this set's own four files. The phase taxonomy was this set's invention, and
  the contradiction was a property of the invention.

  | Call | Propagation |
  |---|---|
  | **The "exactly one raising phase" rule is withdrawn.** A code belongs to the rule that raises it, and the rule belongs to a slice. §3.1's seven phases remain the **execution order** — what runs before what, and therefore which refusal a caller meets first — and stop being a taxonomy | 01 §3.3; 12 `inst-cc-errors` |
  | **The AC #38 map keys on code → declaring slice.** The declaring slice is **P-D-35**'s rule. This buys what the phase unit was introduced to buy and the door unit could not: **P-D-24** abandoned the door unit because one code is raised at many doors, and a code has exactly one declaring slice by construction | 12 `inst-cc-errors` |
  | **There is no carve-out list**, because there is no longer a rule to carve out of. `CONTENT_PII_BLOCKED` (02), `AUDIT_UNAVAILABLE` (01), 05's owed denial code and `BREAKGLASS_WRITE_FORBIDDEN` (05) are codes their own slices declare, and nothing further is owed about them | 01 §3.3; 12; 05 |

- **What this supersedes**: the phase-unit half of **P-D-24** (the door→phase amendment) and the
  scoping half of **P-D-32** ("the rule ranges over codes raised inside the pipeline"). Both were
  correct repairs of a rule that should not have existed; their other calls stand untouched — the
  `state` phase, the code status moves, and the target-state registration key are unaffected,
  because they are about execution order and status, not about attribution. **P-D-30**'s and
  **P-D-34**'s rows likewise carry phase-worded justifications — "both arms sit in the registered
  validators phase and the code needs no carve-out", "the only reading under which the code keeps
  one raising phase" — whose *calls* are untouched: both arms of `RETIREMENT_PENDING` are still
  04's, and 04's final rule still replaces the operand inside 01's `identity` phase. Only the
  reason given has lapsed.
- **What it closes besides its own question**: 12's "`inst-cc-errors` admits one phase carve-out
  and the taxonomy now has two"; 05's "is `BREAKGLASS_WRITE_FORBIDDEN` a phase refusal?"; 01 §6's
  mirror owed to 12; and the phase clause in 01 §6's standing containment risk.
- **Settled since, by P-D-37 (2026-08-28)**: the within-phase half. Stop-at-first-*phase* stands
  as P-D-33 wrote it. The paragraph below reads more broadly than it should have — what was open
  was never the phase boundary but the code overflow inside one phase.
- **Deliberately not decided here**: whether the run still stops at the first failing phase. The
  donor appends and never short-circuits, but it renders a *report* into a response, while this
  gear's `products_audit_log` carries a single `error_code` column — the constraint that produced
  the stop-at-first rule. That is 01 §6's own open item and is unaffected by this call.
- **Propagated**: `design/01-foundation.md` (§3.3, §4.4, §6), `design/04-lifecycle.md` (§3.2's
  code block), `design/05-governance.md` (§6), `design/12-consumer-contracts.md`
  (`inst-cc-errors`, §6).


#### P-D-37 — One code per audit row, every violation in the answer

- **Date**: 2026-08-28 (owner call)
- **Context**: `inst-fd-fail-closed` stops the run at the first failing phase and collects
  violations per-field *within* it, and **P-D-33**'s stated reason is that §4.4's audit row carries
  a single `error_code`, so collecting *across* phases would overflow it. The seventh pass observed
  that the same overflow can happen *inside* one phase, since the `identity` phase names three
  codes and the `state` phase four.

  **Measured before deciding, and the question turned out to be narrower than stated.** The
  `identity` phase is decided by the index **under the write** (§3.4), and an insert violating two
  unique constraints returns one violation — whichever the engine checked first; the donor's
  storage layer folds every `is_unique_violation()` (Postgres `23505` and its SQLite equivalent)
  into a single error without distinguishing the index. So the pass's own example — a create
  colliding on both `name` and `productCode` — physically yields one code, not two. `shape` raises
  a single code with many per-field entries. **Only the `state` phase can genuinely collect two**,
  and it does: a save on a `retired` head that also moves a bucket-i column satisfies
  `ENTITY_TERMINAL` and `ILLEGAL_FIELD_MUTATION` alike.

  | Call | Propagation |
  |---|---|
  | **The caller's rejection carries every violation the failing phase collected; the audit row records one code.** This is the donor's split, and the reason the two differ here is structural: `gears/bss/pricing` renders a whole `ValidationReport` into one refusal and has no `error_code` in its audit record at all, because it does not audit validation refusals — this gear audits every one of them under `nfr-availability-audit`, and a row needs one code | 01 §3.1 `inst-fd-fail-closed`; 12 (the response envelope) |
  | **Precedence over the `state` phase's four codes**: `ENTITY_TERMINAL` → `PARENT_TERMINAL` → `ILLEGAL_TRANSITION` → `ILLEGAL_FIELD_MUTATION`, running from the refusal that admits no write to the row at all down to the one that refuses a single column. Derived, not picked: it is the same ordering that makes terminality the physical floor — if the subject is terminal nothing else about the write matters | 01 §3.3; 12 (AC #38 map reads the stored code) |
  | **The `identity` phase cannot collect a second code**, being decided under the write, and §3.1's per-field collection is therefore a property of the **read-decided** phases. Stated rather than left implicit, so the promise the rule makes is one the phase can keep | 01 §3.1, §3.4 |

- **Not changed**: the column stays a single nullable `error_code`; AC #38 still maps by it; the
  reserved sealing seam still hashes the row as it stands. That is what made the precedence the
  cheaper arm than widening the column, which has three readers.
- **Propagated**: `design/01-foundation.md` (§3.1, §3.3, §3.4, §6). Amends **P-D-33** (which said
  nothing about the within-phase case) and narrows **P-D-36**'s "deliberately not decided" note.


#### P-D-38 — A refusal stores nothing and releases the key

- **Date**: 2026-08-28 (owner call)
- **Context**: `inst-fd-idem-claim-refusal` stored a refusal on the idempotency key and replayed it
  "rather than re-running a rule whose verdict cannot change", carving out `AUDIT_UNAVAILABLE` as
  the one refusal whose verdict *can* change. `inst-fd-idem-hash` keeps the precondition out of the
  payload hash for the opposite reason: a client refused `STALE_REVISION` that re-read the head and
  retried "is making the same request" and must therefore **run**. The two rules of one section
  prescribed opposite things about the same retry.

  **Measured on both sides.** The donor settles it directly: `gears/bss/pricing`'s `idempotent.rs`
  runs claim and answer inside the mutation's transaction precisely so that "a failure anywhere
  rolls the claim back with the mutation" — it stores no refusal at all and needs no exception. And
  the alternative was measured rather than assumed: keyed on "the verdict can change on retry", the
  carve-out selects **ten of the taxonomy's fifteen codes** — `APPROVAL_REQUIRED`,
  `PARENT_NOT_PUBLISHED`, `RETIREMENT_PENDING`, `SCOPE_NOT_CONTAINED`, `INCOMPLETE_ENTITY`,
  `ILLEGAL_TRANSITION`, both `DUPLICATE_*`, `STALE_REVISION` and `AUDIT_UNAVAILABLE`. Only the two
  payload-determined codes (`VALIDATION`, `CONTENT_PII_BLOCKED`) and the two irreversible ones
  (`ENTITY_TERMINAL`, `PARENT_TERMINAL`) would stay. The exception would have become the rule.

  | Call | Propagation |
  |---|---|
  | **A refusal stores nothing and releases the key.** The answer write joins the mutation's transaction and rolls back with it; the door then deletes the claim row in its own transaction, so the key is free immediately and a retry runs. A key exists to prevent a duplicate *side effect*, and a refusal has none — the mutation rolled back — so storing one protects nothing while freezing a transient verdict for `expires_at`'s window, up to a day | 01 §3.2, §4.4; 12 (the consumer note) |
  | **The carve-out is withdrawn**, `AUDIT_UNAVAILABLE` having needed one only because refusals were stored | 01 §3.2 |
  | **`in_flight_until` now covers exactly one case — a door that died**, which is what `inst-fd-idem-claim-inflight-until` always said `claimed` means. The `AUDIT_UNAVAILABLE` collision §6 carried is gone: that 503 is retryable immediately | 01 §3.2, §6 |

- **The argument against, stated**: a retried refusal now writes one audit row per attempt instead
  of one per key. That is what already happens to every refused request carrying no key, and
  `nfr-availability-audit` asks for 100% of refusals rather than for their deduplication.
- **What it does not settle**: `in_flight_until`'s *value* still has no anchor, the set pinning no
  door timeout — §6's item (a), now narrowed to crash recovery alone. And what the three
  `internal:` lanes store in the response columns — item (b) — narrows without closing: the columns
  now only ever reproduce a **success**, and a lane with no wire surface still has none.
- **Propagated**: `design/01-foundation.md` (§3.2, §4.4, §6), `design/12-consumer-contracts.md`
  (the replay note). Amends **P-D-26** (whose "a refusal answers the key" arm is withdrawn) and
  narrows **P-D-34**'s claim-write boundary.


#### P-D-39 — The scope columns, and what the empty set means

- **Date**: 2026-08-28 (owner call)
- **Context**: `inst-fd-containment-scope` and 04 C5 both read a Product's `region_scope`/
  `brand_scope`, and **no door wrote them**. §4.1 listed the pair with neither default nor
  nullability, the create flow never reached them, and the PRD puts brand/region scope on the
  Product create surface (§4.1's operator flow, "Create/select a Product (name, category,
  description, brand/region scope)") without pinning requiredness or the empty-set reading. Under
  the fail-closed wording — "anything not provably a subset" — a Product whose scope was never set
  refuses **every** child SKU that names one, since nothing non-empty is a subset of the empty set.
  The literal set mathematics gives exactly the opposite of the business meaning: an unscoped
  Product sells everywhere, not nowhere.

  | Call | Propagation |
  |---|---|
  | **Both columns are `NOT NULL` with the empty set as default, and the empty set means *unrestricted*.** One spelling of absence, one meaning for it — the alternative, a nullable column where `NULL` means unrestricted and `[]` means nothing, gives absence two spellings with different meanings, which this corpus has been bitten by before | 01 §4.1, §4.2 |
  | **The create door writes them**, as an optional payload value set — `inst-fd-scope-write`. Unlike `brand_id` (P-D-33) they are **not** validated against the caller's claims: they say where the Product may be sold, not who owns it | 01 §2 |
  | **Containment is defined over restrictions, not over raw sets**: an unrestricted parent contains every child; an unrestricted child is contained only by an unrestricted parent; between two non-empty sets it is ordinary subset. A SKU whose payload omits either set **takes the parent's**, so an inherited scope is contained by construction | 01 §2 `inst-fd-containment-scope`; 04 C5 and `inst-pc-containment` |

- **The argument against, stated**: the subset rule gains two boundary cases instead of staying
  pure set mathematics, and they have to be carried in two places — 01's interim check and 04's
  final one. §6's standing risk already names that pair as a thing that must not silently diverge,
  and both sides are amended in the same commit for exactly that reason.
- **Not changed**: both columns stay **bucket-iii in both directions**, so widening and narrowing
  alike are material and meet the governance gate; `PRD` §545's "a SKU's brand/region scope MUST be
  contained within its parent's" holds unchanged under the restriction reading.
- **Propagated**: `design/01-foundation.md` (§2 create flow and containment row, §4.1, §4.2, §6),
  `design/04-lifecycle.md` (C5, `inst-pc-containment`).


#### P-D-40 — The entity-version retention DELETE, under a referential predicate

- **Date**: 2026-08-28 (owner call)
- **Context**: §4.3 admitted no UPDATE or DELETE on `products_entity_version` ever, while 10
  `inst-rt-gc` must collect those rows — "entity versions only after every referencing manifest".
  **P-D-34**'s repair one table over does not transfer: `products_audit_log`'s DELETE arm is a
  row-image predicate (`written_at` older than its class window), and a version row's
  collectability is not a property of the row at all but of what still points at it. So the GC had
  no admitted path and 10's ordering was a procedural promise with nothing enforcing it.

  | Call | Propagation |
  |---|---|
  | **One DELETE is admitted, under a referential predicate**: a `products_entity_version` row may be deleted only when **no `products_catalog_version_entry` references it**. UPDATE stays refused in every form | 01 §4.2 (shared guard), §4.3; 10 `inst-rt-gc` |
  | **A guard may read another table.** This is the first predicate here that does, and it is compatible with **P-D-31**, whose objection was to a guard reading the *door* through a session variable that exists on one engine only. A subquery judges data, and both engines evaluate it — so "the guard judges the row image" is widened to "the guard judges the **data**" | 01 §4.2; P-D-31 (amended) |
  | **06's manifest carries an index on `(tenant_id, entity_kind, entity_id, published_version)`** — not for a read of its own, but because the manifest's key leads with `catalog_version_id` and is useless for this lookup | 06 `products_catalog_version_entry` |

- **What this buys beyond an admitted path**: 10's deletion order stops being a promise. Under the
  predicate a GC *cannot* delete a referenced version row, whichever order it walks — strictly more
  than the audit table's window predicate buys, which still trusts the GC to compute the window.
  It also subsumes the freeze-registration arm transitively: a live registration holds the catalog
  version, which holds its manifest entries, which hold these rows, so **P-D-18**'s "a participant
  that never releases pins that version's storage" needs no second predicate here.
- **The argument against, stated**: a per-row index lookup on every collected version, and an index
  on a table that grows as catalog versions × entities. Both are the price of the guarantee, and
  the alternative that keeps predicates row-image — a denormalized reference counter on the version
  row — buys a column that can drift from the thing it counts.
- **Propagated**: `design/01-foundation.md` (§4.2, §4.3, §6), `design/06-catalog-version.md` (the
  manifest's index), `design/10-retention-erasure.md` (`inst-rt-gc`). Amends **P-D-31**.


#### P-D-41 — The two doors that write bucket-ii

- **Date**: 2026-08-28 (owner call)
- **Context**: §4.2 admits bucket-ii writes while `published_version = 0` and, after first publish,
  only in the same statement as a `published_version` bump. Neither side named a door. Below first
  publish the class was admitted with no writer at all — the same hole **P-D-28** had closed for
  bucket-i, whose own test is that an admitted class needs a named admitting door. Above it, 07's
  `CorrectionDoor` **already accepts** `(skuId, field, new value, expected revision)` and delegates
  its re-publish to 01's `PublishDoor`, whose signature `(entity, expected internal revision)` has
  no slot for the value — so the only statement permitted to write it could not receive it, while
  07's own "clean head" gate forbids staging it as an ordinary edit first.

  | Call | Propagation |
  |---|---|
  | **Below first publish, `inst-fd-save-txn` is the admitting door**, on the same terms as bucket-i. 03 `inst-mt-bucket` already says the draft plane "edits freely"; this names the door that lets it | 01 §2 `inst-fd-save-txn`, §4.2; 03 `inst-mt-bucket` |
  | **After first publish, `PublishDoor` gains an optional third argument** — the corrected bucket-ii field and value — supplied only by 07's `CorrectionDoor`, and written by the door's own head-row UPDATE beside the `published_version` bump | 01 §2 `inst-fd-publish-pin`, new `inst-fd-publish-correction`; 07 `inst-cr-republish` |
  | **This is the mechanism `composition_pending` already uses**, not a new one: the publish door already writes a value into that UPDATE that did not arrive as a head edit, and the freeze is already the **post-act image** "including the `composition_pending` value the same UPDATE is about to write" — so the corrected value is what freezes into version N+1, which is what a correction must do. §4.2's predicate is unchanged | 01 §2 freeze step, §4.2 |

- **The argument against, stated**: 01's door signature widens, and three slices drive that door.
  The argument is **optional and additive**, so 06's composition-clear and 09's per-row publishes
  pass nothing and are untouched — only 07 passes it. The residue is that a door whose job is
  otherwise "publish what the head says" now takes a field value. The alternative — 07 issuing its
  own head-row UPDATE — was rejected because publish mechanics (freeze, version row, event, bump)
  would then be spelled in two places and could diverge silently.
- **Propagated**: `design/01-foundation.md` (§2 publish rows and `inst-fd-save-txn`, §4.2, §6),
  `design/03-sku-classification.md` (`inst-mt-bucket`), `design/07-reference-signal.md`
  (the re-publish step).
