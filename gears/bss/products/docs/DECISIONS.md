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

<!-- /toc -->

## P-D-01 — Broker-native event envelope (not CloudEvents 1.0)

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
  `fr-event-versioning-replay`, §9.2, AC #28/#29; design slice 01 §4 (event fan-out).

## P-D-02 — CatalogVersion increments are mechanical; governance at entity publish

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
- **Propagated**: PRD §4.1/§5.1/§5.2, `fr-define-sku`, `fr-catalog-version-publish`,
  `fr-bundle-adoption-guard`, `fr-prepublish-lint`, AC #7/#19/#25/#45; design slices 03 (`inst-cl-bundle-override`), 05, 06.

## P-D-03 — SkuReferenceCount v1 producer set = {pricing}

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
  (mirrored answered row); rating SEAMS ownership matrix; design slice 07.

## P-D-04 — Absolute product-name uniqueness (region-independent)

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
  AC #5, AC #33a (the promotion fallback identity), AC #38, §16; design slices 01 (uniqueness index + `normalized(name)` pin), 02 (display-name coexistence), 04 (containment), 09 (C5 promotion identity).

## P-D-05 — `usageTypeRef` validates resolvability only; UC3(c) lives at the pricing meter binding

- **Date**: 2026-08-25 (veto round over the UC3 adoption block of 2026-07-28)
- **Decision**: registry publish validates that a declared `usageTypeRef` **resolves** in the
  usage-collector's platform-global UsageType catalog — nothing more. "And is active" is dropped
  (a UsageType carries no lifecycle state: register/get/list/delete only, deletion FK-guarded
  against usage records — not against catalog meters, which rating's quarantine rule
  fail-safes). The UC3(c) dimension-set cross-validation is **not** performed here (the registry
  assigns dimension sets to plan-price and holds no operand): its home is pricing
  `inst-cmp-usagetype` (confirmed 2026-07-31, built) — priced `dimensionKey` **⊆** the
  UsageType's `metadata_fields` at plan publish.
- **Why**: both original clauses named operands that do not exist registry-side; subset (not
  equality) is the load-bearing invariant — pricing fewer dimensions than the source emits is
  harmless, pricing one it never emits is the hazard.
- **Propagated**: PRD `fr-metering-unit-declaration`, AC #8, §15 (answered row); rating SEAMS
  UC3 row + ownership matrix; pricing design/02 (stale "registry holds equality" premise
  retired); design slice 03.
- **Residue (2026-08-25, PR #14 review)**: quarantine-on-deleted-UsageType is a fail-safe, not
  an operating mode — the deletion-guard/deletion-signal negotiation with usage-collector is a
  PRD §15 open ("UsageType deletion vs published declarations").

## P-D-06 — Metadata map lives outside frozen version content

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
- **Propagated**: design slice 02 §2 (`inst-md-placement`) + §4.1/§5 + §6 (flag struck); slice
  06 owes the snapshot-capture step; `DESIGN.md` slice row + status line; PRD glossary
  "Metadata map" is compatible as written (no PRD edit).

## P-D-07 — The staleness stamp is a floor, advanced only when its content is present

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
- **Propagated**: design slice 08 (`inst-rp-stamp`, §6 — flag struck); `DESIGN.md` slice row +
  status line; PRD `fr-cache-first-browse` rationale re-derived + §15 serving-store row; NFR #7
  unchanged.

## P-D-08 — Audit sealing is a platform capability: reserved seam + stated requirements

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
- **Propagated**: design slice 01 §4.4 (the reserved seam + its CHECK); slice 05 §1.6 C7 (the
  G4-shaped constraint row, deferral stated); slice 10 §1.6 C1/C3 (S7 exclusion, S8 retention);
  PRD §15 open row (owner: Architecture / Common Core) + §16 risk row; PRD NFR
  `…-nfr-availability-audit` reads unchanged — it requires audit *completeness*, which v1
  delivers.

## P-D-09 — Stage-vs-commit fail-closed is delivered per lane; the requirement says so

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
  list) + AC #40; design slice 06 `inst-sn-revalidate` (flag struck) and §5's both-arm probes
  (already assert both lanes, unchanged); `DESIGN.md` slice row + status line.

## P-D-10 — No gear-side Legal role: the allow-list records Legal's decision, it does not enact it

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
- **Propagated**: design slice 05 §1.6 **C8** (the narrow-never-replace rule + the two guards
  any future replacing predicate would owe); slice 10 `inst-pp-allowlist` + §5 probe (base
  quorum, mandatory reference, positive control); `design/README.md` slice-10 summary;
  `DESIGN.md` slice row + status line. **No PRD edit owed** — `fr-materiality-gated-publish`
  and AC #26 keep their closed approver set, which this decision makes true again, and AC #35
  is followed rather than amended.

## P-D-11 — The approver count is a policy value with floor 0; the predicates are not

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
- **The reduction, stated rather than wrapped**: at `N = 0` a material catalog publish proceeds
  on one person with no second pair of eyes, and the FinanceReviewer predicate is vacuous. What
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
  bullets) + §17.1 materiality-threshold row; design slice 05 §1.6 C1, §1.7 `ApprovalRecord`,
  `inst-gv-materiality` (the "nothing publishes approver-less" interim retired),
  `inst-gv-queue` (envelope gains `predicateUnsatisfiable`), §6 (flag struck); `DESIGN.md`
  slice row + status line.

## P-D-12 — The `SchemaPin`'s membership is a rule, not a list

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
- **Propagated**: PRD `fr-plan-price-seam` (normative sentence); design slice 12 §1.6 C1,
  `inst-sdk-catalogsku` (`status` + vocabulary now normative pin members, previously a flagged
  widening), §3.2 **`inst-cc-pin`** (CoverageChecks #9 — the count in prose moved with it),
  §6 (flag struck); `DESIGN.md` slice row + status line.
