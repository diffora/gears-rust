# DECISIONS — Product & SKU Registry (`products` gear)

Decision register for the products gear. Prefix **P-D-NN**. A decision lands here with its
date, rationale, and the propagation list of every document that restates it; the §15 rows of
[`PRD.md`](./PRD.md) that answered a gate cite these numbers rather than carrying the only copy.
Historical context: D-46 (`sellable`) and D-47 (`CatalogVersion` increment taxonomy) predate this
register and live in the **pricing** register (`gears/bss/pricing/docs/DECISIONS.md`) — they are
joint contracts, cited from here by their pricing numbers, never duplicated.

<!-- toc -->

- [Decision register](#decision-register)
  - [Entries](#entries)

<!-- /toc -->

## Decision register

### Entries

*Entries stay at `####` deliberately. `spec-check`'s propagation parser recognises a decision
only as `#### <id> …`; promoting them to `###` to satisfy MD001's heading-increment rule —
CodeRabbit's suggestion of 2026-08-26 — would make this register parse as zero decisions, which
is a regression this gear has already paid for once. This intermediate heading satisfies MD001
instead.*

*Consequence for the TOC gate. `cfs validate-toc` indexes to `--max-level 3` by default, so at the
default it does not see these `####` headings at all: **the default invocation is the one that
returns exit 0**, and the TOC lists only the two `##`/`###` headings above. Passing
`--max-level 4` makes every entry indexable and reports one `toc-heading-not-in-toc` per decision —
**54 errors on 2026-08-31, not a defect in this file**. Do not "fix" that by listing the entries;
run the gate at its default. (This paragraph said the opposite until 2026-08-31, measured against
both invocations at that date: it described a state in which the TOC still carried the fifty
per-decision anchors, and it was corrected by running the command it prescribed.)*

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
  `fr-event-versioning-replay`, §9.2, AC #28/#29; `design/01-foundation.md` §4 (event fan-out); `DESIGN.md` §1.2 Key decisions.

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
  `fr-bundle-adoption-guard`, `fr-prepublish-lint`, AC #7/#19/#25/#45; `design/01-foundation.md` §1.2, `design/03-sku-classification.md` (`inst-cl-bundle-override`), `design/05-governance.md`, `design/06-catalog-version.md`, `design/09-bulk-promotion.md` (`inst-bk-override`) — **§4.1 struck**: its bullets say nothing about mechanical increments or entity-publish governance (item 31 of the 2026-08-26 review); `DESIGN.md` §1.2 Key decisions.

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
  ../../rating/docs/SEAMS.md; `design/07-reference-signal.md`; `DESIGN.md` §1.2 Key decisions.

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
  AC #5, AC #33a (the promotion fallback identity), AC #38, §16; `design/01-foundation.md` (uniqueness index + `normalized(name)` pin), `design/02-taxonomy-attributes.md` (display-name coexistence), `design/04-lifecycle.md` (containment), `design/09-bulk-promotion.md` (C5 promotion identity); `DESIGN.md` §1.2 Key decisions.

#### P-D-05 — `usageTypeRef` validates resolvability only; UC3(c) lives at the pricing meter binding

- **Date**: 2026-08-25 (veto round over the UC3 adoption block of 2026-07-28)
- **Decision**: registry publish validates that a declared `usageTypeRef` **resolves** in the
  usage-collector's platform-global UsageType catalog — nothing more. "And is active" is dropped
  (a UsageType carries no lifecycle state: register/get/list/delete only, deletion FK-guarded
  against usage records — not against catalog meters, which rating's quarantine rule
  fail-safes). The UC3(c) dimension-set cross-validation is **not** performed here (the registry
  assigns dimension sets to plan-price and holds no operand): its home is pricing's
  meter-binding rule (confirmed 2026-07-31 — **specified, not built**, corrected 2026-08-26:
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
  retired); `design/03-sku-classification.md`; `DESIGN.md` §1.2 Key decisions.
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
- **Propagated**: `design/02-taxonomy-attributes.md` §2 (`inst-md-placement`) + §4.1/§5 + §6 (flag struck); slice
  `design/06-catalog-version.md` (the snapshot-capture step it owes, and where it cites this decision), `design/01-foundation.md` §4, `design/05-governance.md` §3.2 and `design/README.md` (the three further documents that restate it — added 2026-08-26 after a census of the class rather than of the one site lint 5 named); `DESIGN.md` slice row + status line.
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
- **Propagated**: `design/08-read-models.md` (`inst-rp-stamp`, §6 — flag struck); `DESIGN.md` slice row +
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
- **Propagated**: `design/01-foundation.md` §4.4 (the reserved seam + its CHECK); `design/05-governance.md` §1.6 C7 (the
  G4-shaped constraint row, deferral stated); `design/10-retention-erasure.md` §1.6 C1/C3 (sealing requirements seven
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
  list) + AC #40; `design/06-catalog-version.md` `inst-sn-revalidate` (flag struck) and §5's both-arm probes
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
- **Propagated**: `design/05-governance.md` §1.6 **C8** (the narrow-never-replace rule + the two guards
  any future replacing predicate would owe); `design/10-retention-erasure.md` `inst-pp-allowlist` + §5 probe (base
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
  bullets) + §17.1 materiality-threshold row; `design/05-governance.md` §1.6 C1, §1.7 `ApprovalRecord`,
  `inst-gv-materiality` (the "nothing publishes approver-less" interim retired),
  `inst-gv-queue` (envelope gains `predicateUnsatisfiable` **and `configuredQuorum`** — its
  `required` is the record's *effective* count, `N` for material and `min(N, 1)` for non-material,
  never the raw configured `N`, so a card cannot show "2 required" for a record closing on one),
  §6 (flag struck); **`design/03-sku-classification.md` `inst-ac-required`** — the rule this decision's own amendment
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
- **Propagated**: PRD `fr-plan-price-seam` (normative sentence); `design/12-consumer-contracts.md` §1.6 C1,
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
- **Propagated**: PRD glossary **Two-person rule** row; AC #17, #19, #22, #30; `design/05-governance.md`
  §1.6 C1, §1.7 `ApprovalRecord` + `OverrideCeremony`, `inst-bg-open`, `inst-gv-override`,
  `inst-gv-queue`, `inst-mt-inputs` (the metering-unit clause that routes to slice 07's correction door (a route, not a propagation target)),
  §4 `products_approval`/`products_breakglass_session`; `design/03-sku-classification.md` `inst-cl-bundle-override`;
  `design/04-lifecycle.md` `inst-lc-undeprecate`; `design/06-catalog-version.md` `inst-fz-force`; **`design/07-reference-signal.md` C4, C5,
  `inst-cr-republish`, `inst-bc-ceremony`**; `DESIGN.md` decision register summary.

#### P-D-14 — `system_signal` is an approval subject kind, not an exemption; the authorizing principal is the signal

- **Date**: 2026-08-26 (recorded 2026-08-26 by the branch review — **decided in prose on this
  branch and never registered**, which is the defect this entry closes)
- **Status**: **CONFIRMED as amended by P-D-48** — the subject kind stands; on a dirty head the
  clear is **deferred, never refused**, the owning slice's reading, which this entry had stated the
  other way. *(Was FLAGGED: the design is built on it; it was registered so the owner could veto a
  publish path that has no human approver.)*
- **Decision**: a publish whose **sole** content is a system-owned flag cleared by an inbound
  governed signal — in v1 exactly one: 06's `compositionPending` clearing — uses `ApprovalRecord`
  subject kind **`system_signal`**. The record is auto-satisfied with the **signal reference as
  the authorizing principal**, audited like any other decision. There is no human approver and
  **no exemption from the gate**: the act still produces a record, still lands in the audit
  trail, and is still refused if its preconditions fail.
- **Settled by the owner (P-D-48)**: on a dirty head the clear is **deferred, never refused** —
  the owning slice's reading (`design/06-catalog-version.md` `inst-cc-clear`), which §3.2 already
  carried by raising no error code: the caller is an inbound signal, not a request, so there is
  nobody to answer a refusal to, and a deferral cannot wedge a publish queue. This entry's
  *refused* reading is withdrawn; `fr-materiality-gated-publish`, AC #26 and 05 now say deferred.
- **The precondition that makes it safe**: the head must be **clean**. A `system_signal` publish
  carries the flag and nothing else; if the head holds unpublished bucket-iii/iv edits the
  publish is **deferred** rather than carrying them out under a record with no human approver — held until the head is clean, never refused (the owner's call, P-D-48). This
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
- **Propagated**: `design/05-governance.md` `inst-gv-one-shot`; `design/06-catalog-version.md` §2 composition-clear flow and its
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
- **Status**: **CONFIRMED as recorded (P-D-48)**. *(Was FLAGGED: it is a shape counterpart gears
  build against, so it is the one entry here a neighbouring team can be broken by.)*
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
- **Propagated**: PRD §9.2 (both inbound contract blocks); `design/06-catalog-version.md` increment-request
  intake; `design/12-consumer-contracts.md` `inst-sdk-surface` + the `ObligationRegister`; `DESIGN.md` §6 transport paragraph.

#### P-D-16 — A third correction-admission arm: an unresolvable meter target

- **Date**: 2026-08-26 (recorded by the branch review; the arm was authored into slice 07 as
  item 19 of that day's earlier review and stood against two `MUST`s until this entry)
- **Status**: **CONFIRMED (P-D-48), its open half closed — the arm carries no flag of its own.**
  *(Was FLAGGED: it amends a normative FR and an AC.)*
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
- **Settled by the owner (P-D-48): no flag of its own.** This arm is deliberately **not** behind
  `BREAKGLASS_CORRECTION_DISABLED`, because a default-OFF flag would withhold the exit the decision
  exists to provide — and it carries no flag of its own for the same reason: its admission
  predicate is a resolver fact (not-found), not operator discretion, and the arm already increments
  the same `TripwireCounter` the break-glass lane uses (`inst-bc-unresolvable`). What remains is a
  governed but permanently open write path onto a published, `fresh > 0` SKU's bucket-ii meter
  declaration; the ceremony, the reason and the `SkuCorrectionOverride` evidence row are required on
  every use.
- **Rejected alternative**: drop `inst-bc-unresolvable` and leave the wedged SKU to the §15
  negotiation. Rejected because the negotiation has no v1 landing and the state is reachable in
  v1 — but this is the arm to strike if the owner prefers the quarantine fail-safe §15 names.
- **Propagated**: PRD `fr-immutable-field-correction`, AC #4, §15 row (closed); `design/07-reference-signal.md`
  C5, `inst-bc-admission` (the "only" quantifier now names both arms), `inst-bc-unresolvable`,
  `inst-cr-republish` (the validator re-checks the admitting lane's own predicate).

#### P-D-17 — Promotion identity collision with different content is update-as-draft, not a per-row conflict

- **Date**: 2026-08-26 (recorded by the branch review; slice 09 was amended to this reading as
  item 15 of that day's earlier review, against three unamended PRD statements)
- **Status**: **CONFIRMED as recorded (P-D-48)**. *(Was FLAGGED: it amends an FR, an AC and a §10
  use case.)*
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
  (Alternative Flows); `design/09-bulk-promotion.md` C5 + `inst-pm-resolve`.

#### P-D-18 — Version liveness ends by an explicit release; the release is a fifth inbound contract

- **Date**: 2026-08-26 (recorded by the branch review; built into slices 06/10/12 as the
  slice-10 review's H1 fix, while the PRD question that authorises it was still open)
- **Status**: **CONFIRMED (P-D-48), with the v1 registered freeze-participant set narrowed to
  {plan-price}** — the duty is booked on one counterpart that exists, not three. *(Was FLAGGED: it
  closes an open §15 row and adds a duty on three counterpart gears.)*
- **Amended 2026-08-28 by P-D-49**: the pair below stands; its **domain** does not. The
  retention gate ranges over the version's **`participant_set_snapshot`** (06 §4), not over
  whatever registration rows happen to exist — a snapshot member with **no registration row holds
  the version**, because the fan-out has not reached it yet, while an **empty snapshot** (a tenant
  with no participant registered at publish) has nobody who ever owed an ack and is collectable.
  Quantifying over the registrations instead let an empty ledger satisfy the gate vacuously.
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
  (closed); `design/06-catalog-version.md` `inst-fz-liveness`; `design/05-governance.md` RBAC (`catalog_version × release`);
  `design/10-retention-erasure.md` `inst-rt-gc`; `design/12-consumer-contracts.md` `ObligationRegister` row.

#### P-D-19 — A force-completed version stays refused for posted use until opt-in; the pin is the registry's own door

- **Date**: 2026-08-26 (recorded by the branch review)
- **Status**: **CONFIRMED as amended by P-D-47** — the registry-side pin stands; the per-version
  auto-fallback opt-in, this entry's second disjunct, is withdrawn from v1 and stays the PRD's
  off-by-default later enhancement. *(Was FLAGGED for the owner: it moves an enforcement point back
  from an unbuilt consumer to the registry, and it makes a forced version unpostable by default.)*
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
- **Propagated**: PRD `fr-freeze-recovery`, AC #22; `design/06-catalog-version.md` `inst-fz-force` +
  `IntentfulResolver`, §5 error taxonomy (`VERSION_FORCED_INCOMPLETE`); `design/12-consumer-contracts.md`
  `ObligationRegister` row (the consumer duty becomes belt-and-braces, not the only enforcement).

#### P-D-20 — A publish during the retirement lead window re-announces `SkuRetired`; the door stays open

- **Date**: 2026-08-26 (recorded by the branch review; slice 04 introduced the publish freeze as
  item 16 of that day's earlier review)
- **Status**: **CONFIRMED (P-D-48), and completed** — the re-emission rule now has its door, 01's
  `inst-fd-publish-reannounce`. *(Was FLAGGED: it strikes a design-introduced normative refusal and
  adds a re-emission rule in its place.)*
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
  re-announcement rule); `design/04-lifecycle.md` `inst-rt-initiate` (`RETIREMENT_PENDING` struck from the
  publish door, re-emission added), §5 error taxonomy; `design/12-consumer-contracts.md` `ObligationRegister` (the
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

- **Date**: 2026-08-27 (product call, in the slice-01 review, prompted by "didn't we decide to
  defer audit?" and then "we are counting on the platform audit" — the owner spoke Russian; both
  quotations are translated)
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

- **Date**: 2026-08-27 (product call, in the slice-01 review — "take the toolkit"; the owner
  spoke Russian and the quotation is translated)
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
- **Owed**: `design/10-retention-erasure.md` §3 (the "outbox-delivered" retention class —
  **discharged 2026-08-28** by the slice-10 first lens pass, which found the class still standing),
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
- **Propagated** (2026-08-27, all three closed): `design/12-consumer-contracts.md`
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
  | **`AUDIT_UNAVAILABLE` (503)** — the refusal's audit row could not be written, so the door cannot report the domain refusal (§4.4). Names the condition rather than the mechanism, matching 08's `READ_MODEL_OVERLOADED` and 03's `USAGE_TYPE_UNAVAILABLE`, the gear's two other 503s | 12 (error map) |
  | **`products_audit_log` gains nullable `error_code` and `attempted_key`.** §3.1 makes the code the attribution channel ("never the rule name") and AC #38 maps by it, so it is a column rather than free text; `attempted_key` carries the natural key a pre-mint refusal has in place of an id, which `DUPLICATE_NAME` and `DUPLICATE_CODE` both need | 10 (the audit class its `RetentionClock` reads) |

- **Why `DUPLICATE_CODE` rather than a second, product-named code**: the alternative kept
  `DUPLICATE_SKU_CODE` and added `DUPLICATE_PRODUCT_CODE`, which avoids a breaking rename but
  writes the same rule twice in the enum and leaves a reader asking which applies to a clone that
  suggests both. The owner took the rename while the contract is still unbuilt.
- **What this entry does *not* settle**: what addresses a `products_audit_log` row, so the
  sealing seam's one-way UPDATE can target one — still open in slice 01 §6 with P-D-08's owner.
- **Propagated**: `design/01-foundation.md` (§2, §3.3, §4.4),
  `design/09-bulk-promotion.md` and `design/11-clone.md` (the renamed code).
- **Propagated** *(owed until 2026-08-27, all closed)*: `design/12-consumer-contracts.md` (`inst-cc-errors`' map gains two codes and a 503
  class), `design/10-retention-erasure.md` (the audit roster its retention class reads).

- **Amended 2026-08-28 by P-D-38**: the "a refusal answers the key" call below is withdrawn. A
  refusal stores nothing and releases the key; a retry runs.
- **Amended 2026-08-28 by P-D-42**: the "claim commits in its own transaction" call is withdrawn
  too, its stated reason having been measured and found not to hold — the gate is the insert, not
  a read, so a duplicate is stopped by the index conflict rather than by seeing the row. Two of
  this entry's four boundaries stand.

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
- **Propagated** *(owed until 2026-08-27, all closed)*: `design/04-lifecycle.md` and `design/09-bulk-promotion.md` (the lane names their
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
- **Propagated** *(owed until 2026-08-27, all closed)*: `design/06-catalog-version.md` and `design/08-read-models.md` (the body core their
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
  fields, not rows. **P-D-29** later settled the two collections named here (a JSON array sorted
  by the collection's own identifier); the manifest's entry and capture rows remain open, in
  slice 06 §6 (measured 2026-08-28: no item for this stood in 01 §6).
- **Propagated**: `design/01-foundation.md` (§1.7, §4.2, §4.3, §4.4).
- **Propagated** *(owed until 2026-08-27, all closed)*: `design/05-governance.md` (`BucketRegistry` by name where it reads the tags),
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
- **Propagated** *(owed until 2026-08-27, all closed)*: `design/12-consumer-contracts.md` (the replay contract and the committed-revision
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
- **Propagated** *(owed until 2026-08-27, all closed)*: `design/05-governance.md` (the gate phase's wider scope, and the pre-pipeline
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
- **Propagated**: `design/01-foundation.md` (§2 doors, §3.1, §4.2).
- **Not propagation, recorded so the field above is not read as understating itself**: the third
  row's slice-03 amendment and the two trimmed propagation fields are edits to **this register**,
  not documents this decision reaches. They sat inside the `Propagated` field until 2026-08-30,
  where a path token inside a disclaimer read as a target that never cites the decision — which
  is exactly what a propagation field is checked for.


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
  Product create surface (§10's operator flow, "Create/select a Product (name, category,
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


#### P-D-92 — `set_kind` pins no roster, so row 5 stops holding the recognized-set table

- **Date**: 2026-09-01 (owner call, autonomous under the standing instruction — a **scoping**
  decision in **P-D-74**'s form, not an answer to `features/sku-classification.md` §7 row 5)
- **Context**: row 5 asks whether `tax_category_ref` and `gl_code_ref` belong to this registry at
  all, and its answer *"may delete this feature's validators, its two `set_kind` values and its
  publish-blocking requirement together"*. Its owner is **the PRD owner** and it stays open. But it
  names `dod-recognized-set-table` among its Blocks, and it is that table's **only** live blocker —
  so a question about two of four row-value kinds is holding a table whose DDL need not know how
  many kinds there are.
- **Decision**: the hold on the table is released, exactly as P-D-74 released the capture table's.
  **`products_recognized_set`'s DDL pins no `set_kind` roster** — a non-empty text column — and the
  admitted set is the **membership door's** to enforce once row 5 resolves. This is the third
  application of that form in this chain (`capture_kind`, `entity_kind`, `value_type`), and the
  reason is the same each time: a `CHECK` enumerating the kinds would BE row 5's answer, written by
  a migration instead of by its owner.
- **The arguments against, stated**: a table that admits any kind admits a typo, and the first
  reader of the DDL learns nothing about the four kinds from it. Both are true and both are the
  price of not authoring; the door's roster and this feature's own §1.7 carry the four, and a later
  pin is an in-place edit rather than a redesign.
- **Not changed**: row 5 keeps its grip on `dod-classification-columns` (the two contingent
  columns are two of its seven `MUST`s), on `dod-accounting-validators`, `dod-finance-materiality`,
  `dod-sdk-read-shape`, `dod-recognized-set-events` and `dod-type-profile`. The question itself is
  untouched and stays with the PRD owner.
- **Propagated**: `features/sku-classification.md` §7 row 5 (its Blocks list loses the table),
  `design/03-sku-classification.md` §4 (the column's shape).

#### P-D-91 — None of the four code columns is a database foreign key, and two measurements say so

- **Date**: 2026-09-01 (owner call, autonomous under the standing instruction —
  `features/sku-classification.md` §7 row 7, `design/03` §4's own question)
- **Context**: row 7 asks whether `plan_tier` is a real FK. `dod-classification-columns` already
  carries the whole argument and applies it to four columns rather than one: `plan_tier`,
  `tax_category_ref`, `gl_code_ref` and `metering_unit` are all **single code columns** into
  `products_recognized_set`'s **three-column** primary key.
- **Decision**: **no FK on any of the four.** The answer is not a preference between two workable
  shapes — two independent measurements rule the FK out:
  | Measurement | What it rules out |
  |---|---|
  | A referencing side cannot supply `set_kind` as a **literal**. Neither engine admits a constant in a foreign key's column list, so the only real FK is one over a redundant `set_kind` column added per reference — four columns whose only value is to satisfy a constraint | the FK as written |
  | **Each of the four has a de-list code a raw violation would pre-empt**: `PLAN_TIER_RETIRE_BLOCKED`, `UNIT_DELIST_BLOCKED` and row 5's two. A real FK raises the driver's own error, and the design requires the coded refusal | the FK on any of them, even if the first were solved |
  The referential guarantee is the **membership door's**, which is where the codes live.
- **The arguments against, stated**: the columns can then hold a code no set member carries, and
  nothing physical stops it — a real cost, accepted because the alternative buys the guarantee at
  the price of the refusal the design specifies. The mitigation is the door plus the publish
  validators, and `dod-unit-recognition` requires exactly that refusal for a unit *"unknown or
  `removed`"*.
- **Not changed**: the atomic-pair `CHECK` on `metering_unit`/`usage_type_ref` (both null or both
  non-null) is a **shape** constraint, not referential, and stands; `dod-classification-columns`
  keeps its other blocker (row 5's two contingent columns).
- **Propagated**: `design/03-sku-classification.md` §4 and §6 (the question, struck),
  `features/sku-classification.md` §7 row 7 and `dod-classification-columns`.

#### P-D-90 — The recognized-set membership door: one route family, the grant chosen by set kind

- **Date**: 2026-09-01 (owner call, autonomous under the standing instruction —
  `features/sku-classification.md` §7 row 9)
- **Context**: the only stated write mechanism is `GovernedLiveOp` and this feature names no route,
  while `design/05` §3.2 already carries **`recognized_set × write`** and **`plan_tier × write`**
  with *"no route declared"* — two grants with nothing to spend them.
- **Decision**, three arms:
  1. **`POST /bss-products/v1/recognized-sets/{setKind}/members`** for a membership add, and
     **`POST /bss-products/v1/recognized-sets/{setKind}/members/{memberCode}/transitions`** for the
     `active → deprecated → removed` flips and the re-listing. The shape is this corpus's own,
     set twice by decision already: **P-D-67**'s `POST /bss-products/v1/freeze-participants` and
     **P-D-87**'s `POST /bss-products/v1/reference-producers` plus
     `…/reference-producers/{producer}/retirements`. A third application is uniformity rather than
     invention.
  2. **The grant is chosen by `setKind`, not by the route**: the tier set spends
     `plan_tier × write` and the other three spend `recognized_set × write`. That is the only
     reading under which **both** grants have a spender, and the design separates the tier set
     everywhere else too — its own `DoD`, its own event (`PlanTierUpdated`), its own refusal code
     (`PLAN_TIER_RETIRE_BLOCKED`).
  3. **One door, four sets, one generic membership implementation** — `dod-recognized-set-mechanics`
     requires *"one generic membership lookup"*, and a door per set kind would be four doors
     spending two grants with one rule set behind them.
- **The arguments against, stated**: arm 2 puts an authorization decision on a **path segment**,
  so a reader of the route table cannot see which grant a call spends without reading `setKind`'s
  roster — the alternative (two route families) was rejected because it duplicates one rule set
  across two doors and leaves the generic lookup with two callers to keep aligned. Arm 1's second
  route models a state flip as a subresource, which is `…/retirements`' shape rather than a `PATCH`
  on the member.
- **Not changed**: the mechanism (`GovernedLiveOp`), the removal operand (**P-D-89**), the four
  events, `05`'s catalog rows — the grants exist and this decision gives them a spender rather than
  minting anything.
- **Propagated**: `design/03-sku-classification.md` §2 (the flow's route) and §6,
  `features/sku-classification.md` §7 row 9, `design/05-governance.md` §3.2's `Doors` column (the
  row loses *"no route declared"*).

#### P-D-89 — The removal operand is the non-terminal published head, and the row's own DoDs say so three times

- **Date**: 2026-09-01 (owner call, autonomous under the standing instruction —
  `features/sku-classification.md` §7 row 15, `design/03` §6's own item)
- **Context**: the row asks whether `02` and `03` admit a **`draft`** head as a blocking reference,
  noting three texts that differ — `03` `inst-rs-removal-operand` says *"non-terminal **published**
  heads"*, `02` `inst-ad-deprecate-then-remove` says *"non-terminal head
  (`draft`/`published`/`deprecated` Product or SKU, active category)"*, and the PRD is narrower
  than both.
- **Decision**: **`03`'s operand is the non-terminal *published* head, uniform across its four
  sets, and a `draft` head does NOT block.** This is a measurement, not a choice between readings:
  the three `DoD`s the row itself blocks each state it independently and identically —
  `dod-recognized-set-mechanics` (*"the non-terminal published head, uniform across all four
  `set_kind` values"*), `dod-plantier-governance` (*"while a non-terminal published head carries the
  value"*) and `dod-unit-delist` (*"while a non-terminal published head declares it"*). A fourth
  document closes it: **`dod-unit-recognition` requires refusing a declaration on *"a draft whose
  unit was deprecated before its first publish"*** — a case that is **unreachable** if a draft
  blocks the deprecation, which would leave that `DoD` requiring a refusal no state could produce.
  A draft's protection is therefore the **publish-time** refusal, never the de-listing guard.
- **`02` keeps its own wider operand, and the divergence is already registered.** The row's premise
  — that the two must agree — was retired when `02`'s uniformity claim was struck: `02` §6 records
  the divergence in its own words, and the subjects differ (an attribute value on a draft head has
  no unit-style publish-time re-recognition to fall back on). So this is not the joint decision the
  row's Owner field expects; each slice's operand is stated in its own documents and both now say
  so.
- **The arguments against, stated**: an operator can remove a unit that a hundred drafts declare,
  and every one of those drafts then fails at publish — noisy, and discovered late. The mitigation
  is the design's own: the pre-publish lint (**P-D-02**, informational) *"surfaces `deprecated`-member
  usage so operators see debt before refusal teaches them"*, and deprecation precedes removal in
  the state machine.
- **Not changed**: `02`'s operand; the PRD's narrower wording, which is a subset of this reading and
  not in conflict with it; the tombstone mechanics (**P-D-47**).
- **Propagated**: `design/03-sku-classification.md` §6 (the item, struck),
  `features/sku-classification.md` §7 row 15, and a one-line pointer in
  `features/taxonomy-attributes.md` §7's mirror row.

#### P-D-88 — The nullable-UNIQUE gap: roots get a partial index, coordinates get P-D-39's stated absence

- **Date**: 2026-09-01 (owner call, autonomous under the standing instruction — `design/02` §6's
  *"Both uniqueness guarantees are UNIQUE over nullable columns"*, decided at the moment its first
  half became DDL)
- **Context**: both engines treat NULLs as distinct in a UNIQUE, so §4.1's declared
  `UNIQUE (tenant_id, parent_id, name_normalized)` does not constrain **root** categories, and the
  attribute-value coordinate tuple does not constrain the **global** coordinate — the one
  `inst-av-default-locale` makes mandatory. The item names three candidates: sentinels,
  `NULLS NOT DISTINCT`, extra partial indexes.
- **Decision**, two arms, one per half:
  1. **Root categories: a partial unique index**,
     `UNIQUE (tenant_id, name_normalized) WHERE parent_id IS NULL`, beside the declared UNIQUE —
     because for THIS column the other two candidates are measurably impossible, not merely worse.
     A sentinel cannot satisfy a **self-referencing FK** without minting a fake category row per
     tenant, which every tree walk would then have to skip; and `NULLS NOT DISTINCT` is Postgres 15
     syntax with **no `SQLite` equivalent**, so it cannot hold on both engines and cross-engine
     parity is one of this chain's gates. Partial indexes hold identically on both.
  2. **The attribute-value coordinates: P-D-39's own convention** — `locale`, `region` and `brand`
     ship `NOT NULL` with the empty string as the **stated absence value**, making the declared
     UNIQUE total with no index tricks. These are text columns with no FK, so the sentinel
     objection from arm 1 does not arise, and the gear already answers absence this way elsewhere
     (the item says so itself). The `global` coordinate is then spelled `('', '', '')` —
     **which deliberately answers only the SPELLING**: what "global" means to the resolver, which
     combinations a door admits, and where a brand-scoped default lives stay §6's open items,
     untouched.
- **The arguments against, stated**: arm 1 adds a second index whose predicate a reader must know
  to understand the guarantee (the alternative — no root constraint — ships the defect the item
  measured). Arm 2 makes `''` load-bearing in a UNIQUE, and a door that ever writes a real empty
  string would collide with absence — accepted because the coordinate values are identifiers a
  door validates non-empty anyway, and the third reader of a two-spelling absence is this gear's
  own recorded lesson.
- **Not changed**: §4.1's declared UNIQUE constraints (both ship as written); the resolver's
  coordinate semantics (three §6 items stay open); `inst-av-default-locale`'s wording.
- **Propagated**: `design/02-taxonomy-attributes.md` §4.1 (both table rows) and §6 (the item,
  struck); `features/taxonomy-attributes.md` `dod-category-table` (the root index rides it);
  the value-table arm lands with that table's migration.

#### P-D-87 — Reference-signal's five: four config knobs at home, a retired producer's rows cleared, and the three doors' routes

- **Date**: 2026-09-01 (owner call, autonomous under the standing instruction —
  `features/reference-signal.md` §7 rows 7, 12, 16, 19 and 32)

**1. The four knobs land on `ProductsConfig`, per-deployment and boot-time** (rows 7, 19, 32):
`reference_freshness_minutes` (interim 15), `watermark_skew_tolerance_minutes` (interim 5),
`tripwire_max_overrides_per_30_days` (interim 5) and `breakglass_correction_enabled: bool`
(default `false`). The posture is **P-D-84 arm 5's, now precedent rather than invention**: the gear's
config struct is where a per-deployment number lives, hours or minutes being the unit the knob's own
design states. The freshness threshold is **exported through a getter**, the shape
`resolved_idempotency_retention_hours` already has, because `04-lifecycle`'s `ActivationRunner`
polls on it. Row 32 dissolves rather than needing a ruling: **P-D-71 arm 1 already named the flag
enable-positive**, so "default OFF" and `false` are the same fact and the DoD pins what row 1
deferred, not against it.

**2. A retired producer's watermark and member rows are DELETED in the retirement transaction**
(row 12), and a re-registering producer starts `never-received`. That is what makes the DoD's own
*"a registering producer's first watermark MUST start `never-received`, so onboarding can only
tighten"* true: surviving rows let retire-then-re-register inside the freshness window read
**fresh** against a stale member set and free every SKU that has since gained a reference — the
exact inversion the row names. The producer row itself stays, its `state` moving to `retired`, so
the registration history is not lost.

**3. The three doors' routes and success responses** (row 16), each from the set's nearest
precedent rather than invented:

| door | route | success |
|---|---|---|
| watermark post | `POST /bss-products/v1/reference-watermarks` (already bound) | **200** — state, not a minted resource; an idempotent replay answers the same |
| producer registration | `POST /bss-products/v1/reference-producers` | **201** — a row is minted |
| producer retirement | `POST /bss-products/v1/reference-producers/{producer}/retirements` | **200** |
| correction | `POST /bss-products/v1/skus/{skuId}/corrections` | **202** — the door accepts, the write happens at approval |

The correction route **adopts the shape the shipped crate already announces** (row 20's
measurement): `correctable_after_publish` tells callers *"writable only through the correction door
(POST .../corrections, slice 07)"*, and of the two ways to stop that message being a lie —
adopt the shape, or change a shipped sentence — adopting costs nothing and changing costs the
sentence. The two membership routes take the freeze-participant door's own shape (P-D-67): a plural
collection for the act, a sub-collection for the act on one member.

- **The arguments against, stated**: arm 1 puts four policy numbers in a per-deployment struct where
  §17.1 may later want them per-tenant — the same argument P-D-84 arm 5 took and the same answer,
  nothing per-tenant exists to range over; arm 2 discards what a retired producer last claimed —
  accepted, a watermark is **state, not history** (the slice's own reason for it emitting no event),
  and the retirement's audit row records the act; arm 3 fixes routes ahead of the doors that serve
  them, which is exactly what `12-consumer-contracts`' lint needs to see them at all.
- **Not changed**: the refusal codes and their statuses, P-D-71's flag polarity, P-D-59's gauge, the
  correction door's own open rows (6, 10, 14, 20, 22, 23, 24) and producer registration's (2, 5) —
  neither door is freed by this entry.
- **Propagated**: `features/reference-signal.md` (rows 7, 12, 16, 19, 32 struck; §7 arithmetic;
  `dod-reference-config`, `dod-producer-table` and `dod-watermark-door` freed),
  `design/07-reference-signal.md` (§6 twins where the rows are carried), `ProductsConfig` and the
  producer/watermark doors when they build.

#### P-D-86 — The bulk row's staged payload is a column, appended to the ledger by an in-place edit

- **Date**: 2026-09-01 (owner call, autonomous under the standing instruction —
  `features/bulk-promotion.md` §7 row 30, raised by the group 13a build when it reached the wall)
- **Context**: `design/09` §4's ledger row carries `governed_live_op`, glossed as *"the pending
  payload a **live-entity** row stages"*, and nothing for a Product or SKU row's imported content —
  while `dod-stage-phase` has those rows *"parse, then the same registered validators"* and P-D-69
  arm 5 digests *"the bulk row's staged payload"*.

**`products_bulk_row` gains `staged_payload`, nullable, holding the canonical serialization of the
row's imported content**, written by the import door and read by the worker — appended by editing
`m20260901_000014_create_products_bulk.rs` **in place**, this chain's own convention. A shape CHECK
pairs it with the row class the way the table's other pairs are pinned: a `product` or `sku` row
carries a payload, since a row the worker cannot stage is a row that should never have been
recorded.

**The synchronous-door alternative is measurably wrong, not merely less tidy.** It would put a whole
batch's validation inside one HTTP call — against the 202 the import door answers, against the
ten-thousand-row sizing fixture, and against `dod-stage-phase` and **P-D-54**, which both name the
**worker** as the phase's executor. A design whose executor exists and whose payload does not is
missing the column, not the worker.

**`governed_live_op` keeps its stated meaning.** Folding both row classes onto one column would read
tidier and would require rewriting a gloss the design set states; two nullable payload columns, one
per row class, changes no existing sentence. A later slice may fold them, and that fold is then a
decision with its own entry.

- **The arguments against, stated**: two payload columns on one table is redundant shape, and the
  canonical serialization means the door must render the row's content before it can record it —
  which is what makes P-D-69 arm 5's digest rule computable at all, so the cost buys the answer that
  row already relies on.
- **Not changed**: P-D-69 arm 5 (this supplies its operand), P-D-54's executor, the ledger's
  append-only-after-terminal guard, the row keys' batch scope.
- **Propagated**: `features/bulk-promotion.md` (row 30 struck; `dod-stage-phase` freed),
  `m20260901_000014` and the import door when the worker builds.

#### P-D-85 — The revalidation guard's shipped shape: staged outside the transaction, committed under the fence's own retry loop

- **Date**: 2026-09-01 (owner call, autonomous under the standing instruction — recorded at the
  tick of `cpt-cf-bss-products-dod-stage-commit-revalidation`, whose isolation clause the shipped
  build does not match to the letter)
- **Context**: the DoD mandates *"the transaction opens at the engine default"* and argues that
  under `SERIALIZABLE` *"the transaction aborts rather than raising the code §6 requires"*, while
  `dod-coalescer` mandates `LeaseGuard::with_ack_in_tx` — whose internal loop runs
  `SERIALIZABLE` with transparent retries. Both clauses cannot hold in one build.

**The shipped guard stages OUTSIDE the transaction and commits under the fence, and that shape
delivers both of the DoD's arms.** The staged view predates the transaction, so the in-transaction
re-read compares across the whole collect-to-commit window — a moved `published_version` and a
moved `lifecycle_state` alike, appearance and disappearance included, which the suite asserts. A
serialization abort inside the fence is absorbed by its retry loop, which re-runs the compare on a
fresh snapshot and still surfaces the **refusal** — `Restaged` on the mechanical lanes, the
operator arm's `STAGED_ENTITY_CHANGED` when that door lands — so the abort-instead-of-refusal
failure the DoD's clause guarded against cannot reach a caller. The isolation clause was scoped to
a build that collected and committed in one transaction; the DoD is restated to record the shipped
shape rather than the hypothetical one.

- **The arguments against, stated**: the serializable retries spend transaction attempts a
  read-committed build would not — bounded by the fence's own retry policy, and the increment is a
  background drain with no caller waiting on the attempt count.
- **Not changed**: P-D-53 (its reason holds for a bare transaction: nothing here re-litigates the
  isolation of any door's own writes), the compare's two arms, the lane split (P-D-09), the
  request-never-lost posture.
- **Propagated**: `features/catalog-version.md` (`dod-stage-commit-revalidation`'s opening
  paragraph restated), `infra/increment.rs` (the module doc already records the reading).

#### P-D-84 — The freeze protocol's seven: settled-not-acked, the seeded ledger, the strict flag, the resolver's shape, and the timeout's field

- **Date**: 2026-09-01 (owner call, autonomous under the standing instruction —
  `features/catalog-version.md` §7 rows 6, 7, 12, 18, 36, 44 and 45, the set holding the freeze
  chain's doors)

**1. `freezeComplete` ranges over SETTLED, not acked** (row 6): complete ⇔ the ledger holds no
`pending` row and no `not_frozen(forced)` row — a release settles exactly as an ack does, so
completeness is **monotone** and the regression the row names cannot be expressed. The predicate
reads `state`, whose six admitted edges (P-D-60) leave `released` terminal; slice 10's
version-liveness pair already reads a released registration as released, so nothing there moves.

**2. The ledger's creation point is the increment transaction, and the empty-set vacuity is
deliberate** (row 7): P-D-67's seeding is built — one `pending` row per snapshotted participant,
written in the same transaction as the version row, so no resolution can see a version whose
ledger does not yet exist. An **empty registered set** has nobody to wait for: `freeze_state` is
seeded `complete` at insert (shipped), and C5's fail-closed default governs versions **with**
participants — a tenant that registers none has declined the ceremony, not evaded it.

**3. The exposed flag derives strictly** (row 18): wire `freezeComplete = (freeze_state =
'complete')`. `complete(forced)` reads **false** on the flag — the flag's PRD purpose is
posting-safety and a forced version is not posting-safe (`inst-rv-intent` refuses it
`VERSION_FORCED_INCOMPLETE`, which carries the why) — the column stays the storage truth, and
P-D-19's `freezeComplete = complete(forced)` phrasing stands under the state reading, unamended.

**4. The resolver takes the request door's dual shape** (row 12): an SDK client surface beside the
increment contract (P-D-15's rule for machine consumers) and
`GET /bss-products/v1/catalog-versions/{id}` with a **required** `intent` query as the
out-of-process binding, both passing `catalog_version x read`. The route is the slice's own
prefix; 01's unqualified contract claim is untouched because the surface is this slice's door, not
a new contract id.

**5. The timeout's field is `freeze_timeout_hours: u32` on `ProductsConfig`** (row 36) — hours,
the unit the retention resolution already speaks (*"retention **is** `max(24h,
max_freeze_timeout)`"*), per-deployment like every field of that struct; so `max_freeze_timeout`
IS the configured value — in v1 nothing per-tenant or per-lane exists to take a maximum over.

**6. The ceiling is a boot refusal** (row 45): config validation refuses `freeze_timeout_hours`
above the ten-year ceiling the clamp's upper bound encodes, so `u32::clamp`'s `min <= max`
precondition holds by construction and the resolution stays total.

**7. The export door resolves through the shared lookup** (row 44): when `09`'s export door
builds, it takes the `IntentfulResolver` component like resolve and diff do, keeping *"the single
raising door of `CATALOG_VERSION_UNKNOWN`"* true as written; raising its own refusal was declined
as falsifying a shipped clause to save a function call.

- **The arguments against, stated**: arm 1 lets a version report complete though every participant
  released without consuming — accepted, release is the participant's own declaration; arm 3 makes
  the flag blind to the forced-vs-open distinction — deliberate, the refusal codes carry it; arm 5
  fixes the maximum to one deployment value, which a future per-tenant config would have to
  revisit together with its own row.
- **Not changed**: P-D-60's edge list, P-D-67's seeding and snapshot columns, P-D-19, C5, the
  resolver's refusal codes, rows 13 and 48 (cv-authz's remaining holders).
- **Propagated**: `features/catalog-version.md` (rows 6, 7, 12, 18, 36, 44, 45 struck; §7
  arithmetic; `dod-ack-door`, `dod-intentful-resolver` and `dod-freeze-timeout` freed),
  `ProductsConfig` when `dod-freeze-timeout` builds.

#### P-D-83 — §4 governs the storage shape whole: columns and admitted row populations alike

- **Date**: 2026-09-01 (owner call, autonomous under the standing instruction —
  `features/catalog-version.md` §7 rows 40 and 49, the design-set owner's precedence pair)
- **Context**: the set states §4-over-§5 precedence explicitly and `features/catalog-version.md`
  applies §4 against a §2 step (`inst-fz-ack`'s key) and against a row-value set (the
  `capture_kind` roster) without a stated rule covering either.

**The precedence rule, stated once**: a slice's **§4 storage section governs every storage-shape
fact** — column sets, key shapes, and the admitted value population of a roster column — while §2's
instruction steps stay normative for **behavior**: who writes, when, in which transaction, refusing
what. A §2 step that names a storage fact is a shorthand reading of §4, correct while it agrees and
yielding where it does not. Consequences: `inst-fz-ack`'s `(version, participant)` needs no edit —
it is §4's `(tenant_id, catalog_version_id, participant)` read as the shorthand it is (row 40); the
capture-store roster is **§4's seven `capture_kind` values**, and the snapshot builder enforces
seven — `inst-sn-collect`'s and `inst-df-diff`'s six-value lists are behavioral enumerations of the
six whose source stores ship or are named today, not a competing roster (row 49). P-D-74's DDL
posture is unchanged: the builder is the enforcement site, and a later in-place DDL pin stays open
to whoever wants the CHECK.

- **The arguments against, stated**: reading a §2/§4 disagreement as shorthand can hide a real
  contradiction; the guard is that the shorthand ruling applies only where the §2 form is a
  projection of §4's (fewer axes, same members), never where the two name different members.
- **Not changed**: §2's normativity over behavior, P-D-74, the §5-versus-§4 precedence the siblings
  state.
- **Propagated**: `features/catalog-version.md` (rows 40 and 49 struck; §7 arithmetic — row 49's
  "blocks nothing" preamble line was stale against the row's own `Blocks` field and is corrected).

#### P-D-82 — Instants truncate to microseconds at every head-row write

- **Date**: 2026-09-01 (owner call, autonomous under the standing instruction —
  `features/catalog-version.md` §7 row 25, the fix `canonical::render_instant`'s own doc names,
  owed to `01-foundation`'s create doors)
- **Context**: `Utc::now()` carries nanoseconds; `SQLite` stores all nine digits while Postgres
  `timestamptz` **rounds** to six, so the same logical entity can freeze under two `content`
  strings and two digests across engines — under the byte-identity flagship.

**Every instant a head row stores is truncated to microseconds at the write** — the five creating
sites (`create_product`, `create_sku`, the product clone parent, the family child, the lone-SKU
clone) and any later site that mints a stored instant — through one named helper beside
`render_instant`, so neither engine holds a digit the other could round differently. Postgres's
rounding (versus truncation) can still differ on the half-microsecond boundary for a *rounded*
value; truncation at the write removes the digits before the engine sees them, which is why the
helper truncates rather than rounds — after it, both engines store the identical six digits.

- **The arguments against, stated**: sub-microsecond precision is lost — measured, nothing reads
  it; existing rows keep their stored values (dev data, no migration owed).
- **Not changed**: `render_instant`'s own truncation (defense in depth at the render), the golden
  vector (its fixture instants are whole seconds and its bytes do not move).
- **Propagated**: `features/catalog-version.md` (row 25 struck), the five write sites and the
  helper in `domain/canonical.rs`; the second open clause of `dod-version-history-table` (the
  cross-engine golden assertion) becomes buildable.

#### P-D-81 — The port stays the consumer's: adapter-supplied operands, a self-describing pending ref, the poll as its own surface, and no new trait

- **Date**: 2026-09-01 (owner call, autonomous under the standing instruction —
  `features/catalog-version.md` §7 rows 19, 20, 21 and 28, the increment port cluster)
- **Context**: `pricing-sdk` ships `CatalogVersionRegistryV1 { request_version(ctx, request_id) ->
  PendingVersionRef, committed_version(ctx, pending_ref) -> Option<CatalogVersion> }` with one
  consumer; this feature's request entity carries `(source, lane, request_key, operation_key?)`.

**1. The adapter supplies the operands; the port does not widen in v1** (row 19): `source` is the
port binding's registered producer name (`pricing`, the v1 set's one member — P-D-03), `lane` is
`interactive` (the SDK port *is* the interactive surface; the bulk lane's requester is this gear's
own bulk worker, in-crate, never crossing the SDK), `request_key = request_id`, `operation_key`
absent. Widening a shipped consumer trait for operands only the provider's own internal caller
needs would move the seam for nothing.

**2. `pending_ref` is the request's own coordinates, rendered** (row 20): the adapter answers
`pending_ref = "{source}/{request_key}"`. A consumer row keyed on the ref is thereby keyed on
exactly what `CatalogVersionPublished.satisfiedRequests` carries, so the event closes it with no
mapping table anywhere; and `request_version`'s stated idempotency — *"a retry after a crash
returns the same pending ref"* — holds by construction, the same key rendering the same ref.

**3. The poll's surface is the port method itself** (row 21): `committed_version` parses the ref
and reads the request row under the caller's scope — `pending` answers `None`, `coalesced` answers
the version `satisfied_by_version_id` names. No HTTP door is owed; the resolver door stays keyed on
`catalogVersionId`, and *"one of its two methods with no surface"* dissolves — an in-process port
method is a surface.

**4. A second trait beside `ProductsClient`, in `bss-products-sdk`** (row 28 — corrected in the
same session, before anything built on the first answer): the first draft of this arm reached for
the `rate-provider` precedent (the provider implements the consumer's trait) and had products
depend on `pricing-sdk`. **That measured the wrong donor**: the DoD's own normative text mandates
the contract *"as a client trait in `bss-products-sdk`"*, and `pricing-sdk`'s port doc had already
pre-agreed the opposite edge — *"when the registry publishes its own SDK this trait becomes an
adapter over it"*, the contract living in pricing *"only so the registry gear can implement it
without depending on `bss-pricing`"*. So: `bss-products-sdk` gains a **second trait** carrying the
whole `IncrementRequest` (typed `(source, lane, request_key, operation_key?)`) plus the poll, with
the not-wired / unreachable / unusable-answer error axis; `ProductsClient` is not widened (its own
doc scopes it to reading); the products crate ships the in-process binding; and pricing's port
becoming an adapter over it is **pricing's own pre-agreed work**, not this gear's — no dependency
edge from products to pricing-sdk exists or arrives.

- **The arguments against, stated**: arm 1 leaves the two-lane split unexpressed at the SDK seam —
  deliberately, until a second external producer exists to need it (the products-sdk trait carries
  the lane, so the seam is the pricing adapter's, not the contract's); arm 2 makes the ref
  parseable and a consumer may come to depend on its shape — the shape is therefore declared as
  the contract where the ref is minted; arm 4's first draft is kept struck-through in the register's
  history as the lesson: a §7 row can re-ask a question the DoD's own body already answered, and
  the row must be read against that body before a precedent is reached for.
- **Not changed**: the port's shipped signature, P-D-52's refusal discriminator, the request
  queue's key, `ProductsClient`.
- **Propagated**: `features/catalog-version.md` (rows 19, 20, 21, 28 struck; §7 arithmetic),
  the adapter when `dod-increment-request-port` builds.

#### P-D-80 — The manifest renders complete-set against its own pinned roster, and keyed collections sort by their key

- **Date**: 2026-09-01 (owner call, autonomous under the standing instruction —
  `features/catalog-version.md` §7 rows 15 (carried; `design/06` §6 answered first) and 43, the
  canonicalization pair, ours with 01's pin)
- **Context**: P-D-28 orders fields, not rows; P-D-29 names a row rule for two content sets only;
  `domain::canonical` requires the absence mode as an argument and a complete set requires a
  declared roster. The manifest's entry and capture rows are neither of P-D-29's sets, and no
  document said which `Absence` arm the manifest takes.

**1. The sort rule extends to every keyed row collection**: *"by the collection's own identifier"*
generalizes to **a keyed collection sorts by its own key rendering** — the manifest's entry rows by
`(entity_kind, entity_id)`, its capture rows by `capture_kind`, both being their stores' primary
keys under the fixed tenant and version. Two engines and two runs then hash one snapshot to one
digest, which is the flagship's own requirement.

**2. The manifest renders under `Absence::Null` against a pinned manifest roster** — the envelope's
own field names, declared as a `const` beside the snapshot builder, `DIGEST_VERSION` governing any
change. The parsed-request arm was declined: the checksum exists so slice 10's drill can re-verify
a stored manifest years later, and a drill needs the roster pinned in code rather than inferred
from the value — inference is a no-op exactly in the forgotten-field case the mode exists for,
`canonical`'s own words.

- **The arguments against, stated**: the builder constructs every field, so `Omit` could never
  actually omit one today — but "today" is the premise that rots, and the complete-set arm costs
  one const.
- **Not changed**: P-D-28, P-D-29's two named sets, `render_instant`, the checksum's coverage
  (both halves plus the participant snapshot — P-D-67).
- **Propagated**: `design/06-catalog-version.md` (§6's sort-key item answered; `inst-sn-checksum`'s
  parenthetical updated), `features/catalog-version.md` (rows 15 and 43 struck; §7 arithmetic),
  the roster const when `dod-snapshot-builder` builds.

#### P-D-79 — The product clone act is the family act, and the claim row carries its parent handle

- **Date**: 2026-09-01 (owner call, autonomous under the standing instruction — the three operands
  `dod-clone-children`'s build needs that P-D-72 and P-D-75 presupposed without naming)
- **Context**: no §7 row asks how a caller requests the product-with-SKUs act. P-D-75 closed the
  body as *"the overrides and nothing else"* with no flag in it, no document in the set names a
  lone-product clone, and the only two clone shapes named anywhere are the lone-SKU clone and the
  product-with-SKUs clone.

**1. Every product clone is the family act.** The closed body carries no selector, so there is
nothing for a caller to choose with; a childless product degenerates to a family of zero, the
per-child receipt present and empty. The remedy for an operator wanting the shell alone is
discarding the unwanted child drafts — the set's own *"drafts are cheap"*.

**2. The product door spends both grants unconditionally.** L4's *"a product-with-SKUs clone
requires **both**"* becomes the door's own gate: authorization is a pre-pipeline gate (**P-D-30**)
and cannot depend on a child count the door has not yet been authorized to read.

**3. The claim row gains `entity_ref` — the composite act's parent handle.** P-D-72's resume
*"scans the new parent's children"*, which presupposes the parent is findable; with several family
acts over one source, `cloned_from = source` selects several parents, and the claim row stores
nothing else. So `products_idempotency` gains one nullable id column, written in the parent's
transaction (claim `INSERT`, parent row, `entity_ref` stamp — one transaction), `NULL` for every
single-entity door; `IdempotencyClaim::InFlight` carries it out to the door, and the expired-claim
takeover resets it beside the response pair. **The family act's answer is not stored in the
parent's transaction**: the claim stays committed-and-unanswered — P-D-72's *"in progress"* —
until the children phase completes and the receipt is stored, which is what makes the crash window
resumable instead of replaying a parent-only answer.

**4. The family's children are the source's non-discarded SKUs**, each in any of C1's four states.
A `discarded` child is not attempted and not receipted — it is outside the family C1 admits —
where the lone door refuses it by name because there the caller addressed it by name.

**5. The concurrent same-key retry race is accepted and stated.** Two concurrent retries of one
unanswered key can both read committed-and-unanswered and both clone a remainder child, leaving
duplicate child drafts; the stored answer is the last completer's honest receipt. The fence was
declined: a per-parent uniqueness over `cloned_from` would refuse the legitimate second clone of
one source under one parent.

- **The arguments against, stated**: arm 1 makes cloning a fifty-SKU product unavoidable at this
  door; arm 2 demands `sku × write` of a caller cloning a childless product; arm 3 widens 01's
  P-D-42 table for one door's semantics — taken because the claim already joins the parent's
  transaction by P-D-72's own words, which made it the composite act's record.
- **Not changed**: P-D-42's single-entity semantics at every other door (the column stays `NULL`
  there), P-D-72's per-child transactions and receipt shape, the lone-SKU carve-out, P-D-75's body.
- **Propagated**: `design/11-clone.md` (§2 rules 1 and 6), `features/clone.md`
  (`dod-clone-door`, `dod-clone-children`, `dod-clone-authz`),
  `m20260829_000006_create_products_idempotency.rs` edited in place per the chain's convention.

#### P-D-78 — A frozen-state source reads its last frozen version; nothing bookmarks the version at deprecation

- **Date**: 2026-09-01 (owner call, autonomous under the standing instruction —
  `features/clone.md` §7 row 15)
- **Context**: §4.3's exclusions make a `deprecated` entity's last frozen version content-identical
  to a published one, but a `deprecated` head that moved *after* deprecation leaves two candidate
  reads, and the row asked which one the clone takes.

**The read is uniform — the last frozen version for `published`, `deprecated` and `retired`
alike, including a `deprecated` source whose head has moved since deprecation.** Two measurements
force it. First, the retirement design itself keeps the head open through the lead window and
re-announces every move — `design/04` `inst-rt-initiate`: a publish that moves the version
re-emits `SkuRetired` with the new `fromVersion` and *"consumers key on `(skuId, effectiveAt)` and
take the latest"* — so the latest frozen bytes are what consumers see under deprecation, and a
clone of the version current at deprecation would clone content consumers were already
re-announced away from. Second, the alternative has no operand: no store records which version was
current at deprecation — `deprecation_provenance` carries `direct|cascaded`, not a version, and
no other column or table holds the bookmark — so the "version at deprecation" read is unbuildable
without authoring a new column no document asks for.

- **The arguments against, stated**: an operator deprecating at version N may have meant "N is the
  last good one", and later frozen edits may be exactly what deprecation was meant to fence. But
  `cloned_from_version` records exactly the version read, the clone is a draft an operator reviews
  before publishing, and a version selector in the body was closed off by P-D-75's *"overrides and
  nothing else"*.
- **Not changed**: the head read for a `draft` source, §4.3's frozen-content exclusions, P-D-76's
  lineage pair.
- **Propagated**: `features/clone.md` (row 15 struck; one sentence in
  `cpt-cf-bss-products-dod-clone-read-surface`).

#### P-D-77 — The canonical decoder is `01-foundation`'s, beside the renderer

- **Date**: 2026-09-01 (owner call, autonomous under the standing instruction —
  `features/clone.md` §7 row 23)
- **Context**: `products_entity_version.content` is the canonical rendering as one string; the
  clone read surface needs the inverse, and the row asked which slice owns it.

**`domain::canonical` gains the decoder, beside `canonical_rendering`, owned by `01-foundation`.**
The row's own measurement decides it: building the parse at the clone door *"would create the
second serialization rule `domain/canonical.rs` exists to prevent"*, and
`dod-clone-read-surface` already binds the mechanism — *"MUST be the inverse of
`canonical_rendering` and live beside it"*. The decoder is 01's export like the renderer, the
clone read surface is its first consumer, and the round-trip test lives beside the renderer's own
tests. The interim parse the product clone door shipped with is replaced by the call.

- **The arguments against, stated**: the clone is today the decoder's only consumer, so placing it
  with the consumer would keep 01's surface one function smaller — declined for the row's own
  reason, and because a second consumer (the family act's child reads) arrives with the same
  build.
- **Not changed**: `canonical_rendering`, `content_digest` and `render_instant`; `repo.rs`'s
  "deliberately imports no canonicalizer" posture (the decoder's callers are the doors, not the
  repo).
- **Propagated**: `features/clone.md` (row 23 struck; the read-surface DoD's ownership sentence),
  `domain/canonical.rs`, `api/rest/products.rs`.

#### P-D-76 — `cloned_from` is two columns, immutable after create, and inside the content roster

- **Date**: 2026-09-01 (owner call, autonomous under the standing instruction — row 18 of
  `features/clone.md` §7, plus the roster placement its build forces)
- **Context**: `design/11` §4 says one nullable column while `inst-cn-lineage` records
  `(entity id, published_version | 'draft')` — a pair — and the head tables ship neither.
- **Decision**: **two columns**, the P-D-50 convention the set has now chosen three times —
  `cloned_from` (nullable uuid, the immediate source; for a SKU child its own source SKU, P-D-72) and
  `cloned_from_version` (nullable bigint; **NULL under a non-NULL `cloned_from` means the source was
  read at its head — a draft**, the `'draft'` sentinel made representable without a sentinel), with
  the shape CHECK `cloned_from IS NULL ⇒ cloned_from_version IS NULL`. Both join the head guards'
  **immutable set** — writable only in the creating statement, exactly `inst-cn-lineage`'s create-only
  rule — and both are added by editing `m20260829_000002`/`000003` **in place**. **They join the
  content roster**, by the roster's own membership rule: excluded is exactly what the publish act
  moves, and lineage is not moved by publish. No shipped data carries a digest, so the inclusion
  costs nothing today and never again; `digest_version` stays 1.
- **The argument against, stated**: content now differs between a clone and a hand-created twin — it
  already did, `product_id` being a roster member, so no byte-identity anyone relies on changes; and
  an encoded string was declined as the anti-pattern the set has struck twice.
- **Propagated**: `design/11-clone.md` (§4's one-column sentence corrected, §6 twin), `features/clone.md`
  (`dod-cloned-from-column`, §7 row 18 answered and its arithmetic).

#### P-D-75 — The clone door's five: its body, its side-table write, its discarded answer, C4's scope, its key

- **Date**: 2026-09-01 (owner call, autonomous under the standing instruction — the five rows holding
  `dod-clone-door`, taken together so the door can be built; rows 1, 2 and 5 carried, answered in
  `design/11` §6 first)

**1. The request body is the overrides and nothing else** (row 1):
`{code?, name?, newParentId?, and optional replacement values for the five re-validated classes}` —
anything absent copies or resets per the disposition table. The replacement slots exist because the
alternative dead-ends: a retired source with a deprecated attribute definition is refused naming the
field, the source is immutable, and without a re-select slot in the retry that lineage could never be
cloned — C4's own words are that the refusal names the field and the verdict *"so the operator
re-selects rather than guesses"*, and the only place a re-selection can land is the retry.

**2. The clone door writes the side tables in its own creating transaction** (row 2) — **P-D-46's
precedent extended to the second composite creator**: entity row, side rows, `internal_revision = 1`,
no side-door events, no second grant. The side tables do not ship yet (`02`/`03`), so the buildable
clone today copies the entity-row classes and this arm binds the side-table write the day they land.

**3. A `discarded` source is refused `CLONE_SOURCE_DISCARDED`, 409** (row 5) — minted on **P-D-52**'s
own test: the door owes a classified refusal and nothing existing fits. `ENTITY_TERMINAL` is
measured-wrong (it means a head **write**, and the clone writes nothing to the source, while a
`retired` source is explicitly admitted); the bare 404 is measured-wrong (the row is addressable and
the 404 convention carries no code channel). Declared by `design/11` §3.2; 409 by the
state-refuses-the-act mapping.

**4. C4's "every field class that failed" is scoped to the re-validated classes** (row 12) — the
row's own closing arm. Identity collisions are decided under the write (P-D-37) and surface per the
ordinary phase rules; an operator can learn of a name collision on the retry, exactly as on every
create. The pre-flight uniqueness probe was declined: it is a read racing the reservation it
predicts.

**5. The door is keyed, with the ordinary semantics** (row 14) — **P-D-72 already presupposed it**:
the family clone's resume is *"the same-key retry"* finding a committed-but-unanswered claim. Two
identical keyless clone requests are two legitimate clones (`Phase::Idempotency` is skipped, never
failed, on a keyless request); a keyed retry replays the first clone, which is what a crash-retrying
caller needs to not double-clone.

- **The arguments against, stated**: arm 1 widens the body with replacement slots for classes whose
  stores do not ship — declared now so the SDK shape (row 1's co-owner is `12`) is stable when they
  do; arm 3 is the day's only code mint, taken on precedent rather than taste; arm 5 gives a
  minted-id create replay semantics, which is unusual but exactly P-D-72's contract.
- **Not changed**: the disposition table, P-D-62's suggestion mechanics, P-D-72's family resume, and
  rows 3, 6, 8, 11, 18, 20, 25, 27 of `features/clone.md` §7 — the authz roster, lineage surface and
  test-scope questions stay open.
- **Propagated**: `design/11-clone.md` (§2 rule 1 the body and the key, §2 rule 2 the side-table
  write, §3.2 the minted code, §6 twins for rows 1, 2, 5), `features/clone.md` (`dod-clone-door`,
  `dod-disposition-rules`, §7's arithmetic and the five rows answered).

#### P-D-74 — The capture DDL pins no `capture_kind` roster, so row 49 stops holding the entry table

- **Date**: 2026-09-01 (owner call, autonomous under the standing instruction — a **scoping**
  decision, not an answer to row 49's question)
- **Context**: `features/catalog-version.md` §7 row 49 asks whether a §2 instruction or a §4 storage
  bullet governs a **row-value roster** — the capture-kind set, where §4 carries seven values and
  `inst-sn-collect`/`inst-df-diff` list six, omitting `category values`. Its owner is the design-set
  owner (row 40's), and it named `dod-version-entry-table` among its Blocks.
- **Decision**: the hold on the entry table rested on an assumption the DDL need not make. **The
  capture table's DDL pins no `capture_kind` roster** — `capture_kind` is a non-empty text column,
  and the admitted set is the **snapshot builder's** to enforce once rows 49/40 resolve, which is
  where the question actually lives. So row 49 keeps blocking `dod-snapshot-builder` and
  `dod-diff-door` and stops blocking the table. The freeze ledger's state roster *is* CHECK-pinned
  because its set is decided (P-D-60); this one is not, and pinning either count would author the
  answer — a later pin is an in-place migration edit, this chain's own convention.
- **The argument against, stated**: an unpinned roster admits a typo'd kind at the storage layer
  until the builder lands — accepted because the builder is the only writer the design admits, and
  the alternative authored a contested set into a CHECK.
- **Propagated**: `features/catalog-version.md` (row 49's Blocks narrowed, `dod-version-entry-table`
  noting the unpinned roster), `design/06-catalog-version.md` (the capture bullet's note).

#### P-D-73 — The version row unblocked: a digest companion, three cache writers, and a header that never existed

- **Date**: 2026-09-01 (owner call, autonomous under the standing instruction — the three rows
  holding `dod-catalog-version-table`, taken together so the table can be built)
- **Context**: `features/catalog-version.md` §7 rows 24, 38 and 42.

**1. `products_catalog_version` gains `digest_version`** (row 24) — written at publish beside the
checksum, mirroring `products_entity_version`'s convention exactly; `domain::canonical`'s own doc
states the reason and it applies identically to a manifest: without the column, corruption is
invisible to every checksum because the drill cannot re-verify against the rule the digest was
actually computed under.

**2. `freeze_state`'s cache is refreshed by the three acts that change the ledger** (row 38): the
ack door, the release door and the force-completion ceremony each recompute the P-D-49
snapshot-driven summary **in their own transaction** and write it. `complete` therefore lands with
the last snapshot member's ack, `complete(forced)` stays the ceremony's, and the
all-acked-while-cache-reads-`open` window is eliminated by construction. Recompute-on-read was
declined because the column's readers are the resolution refusals — C5 and `inst-rv-intent` branch
on it — and resolution is the hot path; a per-read ledger aggregate would tax every posted-intent
read for the benefit of avoiding one summary write on rare acts.

**3. "The manifest header" is struck** (row 42): it appears in exactly three places — §4's roster
item, the FEATURE's guard-enumeration mirror, and the row asking what it is — with **no field set,
no writer and no reader anywhere in the tree**. The third strike of the `superseded`/`staged_at`
class. The manifest's body is the two P-D-60 tables; the version row's summary columns are already
named individually.

- **The arguments against, stated**: arm 2 writes a derived value from three doors — three writers of
  one cache, ordered by their own transactions, and the ledger stays the authority a drill checks
  the cache against; arm 3 strikes a name a later reader might have wanted as an extension point —
  an extension point with no stated content is exactly what the class strike exists for.
- **Not changed**: rows 6, 7 and 18 (the formula's regression semantics, the naming, the predicate),
  the append-only posture, and `freeze_state`'s roster.
- **Propagated**: `design/06-catalog-version.md` (§4 the column roster), `features/catalog-version.md`
  (`dod-catalog-version-table`, `dod-ack-door`, `dod-snapshot-builder`, the guard enumeration, §7's
  arithmetic and rows 24, 38, 42 answered).

#### P-D-72 — The family clone resumes from its own data, and the identity-map remainder is a widening

- **Date**: 2026-09-01 (owner call, autonomous under the standing instruction;
  `features/retention-erasure.md` §7 row 8 — which surfaces may resolve an identity through the map —
  is **parked with `features/read-models.md`'s row 25**, the same privacy fork seen from the other
  side)
- **Context**: `features/clone.md` §7 rows 7, 19 and 26, jointly holding `dod-clone-children` and
  `dod-clone-lineage`, and `features/retention-erasure.md` §7 row 20.

**1. A child's `cloned_from` names its own source SKU** (clone row 19) — the column is uniform:
every clone's `cloned_from` names *its* source, same-kind, never the parent act. The batch stays
walkable anyway: the new parent's `cloned_from` names the source product, and the family
reconstructs from `parent_id` plus the children's own pointers — which arm 2 turns into the resume
operand.

**2. The durable ledger is the data itself, and the same-key retry resumes the family act**
(clone row 7). The decided posture stands — per-child transactions, an honestly-reported partial —
and no ledger table is built. A crash between children leaves the new parent's committed children
carrying their `cloned_from` pointers, so **resumption is a re-entry**: the retry with the same
idempotency key finds the claim committed and unanswered, scans the new parent's children, skips
sources already cloned, clones the rest, and stores the answer at completion. **This extends
P-D-42 for composite wire acts, and the extension is named**: the endpoint claim joins the
*parent's* transaction (the composite's first), and a committed-but-unanswered claim means
*in progress — resume*, never *replay* and never the refusal a conflicting concurrent claim gets.

**3. The family act answers `201` with a per-child receipt** (clone row 26): the parent was created
and *parent-plus-surviving-children is a valid, intended end state*, so the partial is not an error
status — the response carries one entry per attempted child,
`{source sku_id, disposition ∈ {created, failed}, new sku_id | code + violations}`, the codes being
the owning doors' verbatim (no parallel taxonomy, `09`'s own rule). A **failing parent** stays the
ordinary refusal of the whole act.

**4. The identity-map remainder is a widening of the ticked foundation DoD, not a second DoD**
(retention row 20): the tombstone-inclusive read belongs to `dod-actor-ref`, the function's owner —
a second DoD over the same code has two owners and no recorded precedence, the row's own argument.
`dod-identity-map` keeps the erasure-resolve and export halves, which their own DoDs already oblige.

- **The arguments against, stated**: arm 2 makes the clone door's claim semantics composite-aware —
  a committed unanswered claim is a third state P-D-42's single-entity reading did not have, and the
  resume scan costs a read of the new parent's children per retry; arm 3 reports a partial success
  as `201`, which a caller must read the receipt to see — the alternative (a 207-style multi-status)
  imports a vocabulary this API nowhere else uses.
- **Not changed**: per-child transactions and the honest partial (already decided), the lone-SKU
  carve-out, `retention` row 8 and `read-models` row 25 (parked together), and P-D-42's single-entity
  semantics everywhere else.
- **Propagated**: `design/11-clone.md` (§2 rule 6 the resume re-entry and the receipt, §6 twin for
  row 7 where carried), `features/clone.md` (`dod-clone-children`, `dod-clone-lineage`,
  `dod-clone-door`, §7's arithmetic and rows 7, 19, 26 answered), `features/retention-erasure.md`
  (`dod-identity-map`, §7's arithmetic and row 20 answered), `design/01-foundation.md` §3.2 —
  **owed**: the composite-claim extension's one-sentence home, recorded here and filed at the next
  01 edit.

#### P-D-71 — Reference-signal's seven: the flag named, the hash stored, absence means never-received

- **Date**: 2026-09-01 (owner call, autonomous under the standing instruction)
- **Context**: `features/reference-signal.md` §7 rows 1, 13, 25, 26, 28, 29 and 30, jointly holding
  `dod-breakglass-unavailable`, `dod-reference-events` and `dod-watermark-tables` (rows 1 and 13
  carried, answered in `design/07` §6 first).

**1. The flag is `breakglass_correction_enabled: bool`, default `false`** (row 1) — enable-positive,
so *"the flag is OFF"* and *"the arm is disabled"* stop being the same words for opposite polarities;
the refusal code stays `BREAKGLASS_CORRECTION_DISABLED` (403).

**2. It is per-deployment and boot-time** (row 29) — it lives in `ProductsConfig`, *"the gear's boot
configuration"*, exactly where `dod-reference-config` already put it. **The flag is a policy gate,
not an incident tool**: the emergency surface is `05`'s read elevation, which has no flag; this arm
is a deliberate organisational enablement of a write mechanism, and requiring a deploy to turn it on
is the point. A runtime or per-tenant toggle would need a reload mechanism or a store no slice
declares.

**3. The set hash is stored at ingestion** (row 13): a `set_hash` column on
`products_reference_watermark`, computed over the member `sku_id`s **sorted bytewise**, one
algorithm named (`SHA-256`) — the stored-checksum convention `06` already uses. Recomputing from 10K
member rows at every idempotence comparison is the declined arm.

**4. `ReferenceProducerSetChanged`'s aggregate is the tenant's producer set itself** (row 25):
`aggregate_id = tenant_id` — the set is a per-tenant singleton, so per-`(tenant, aggregate)` ordering
serializes set changes per tenant, which is exactly what a consumer of a *set* needs.
`FreezeParticipantSetChanged` is the same class and its subject question stays `06`'s row, noted as
parallel and not decided here.

**5. `never-received` is the absence of the watermark row** (row 26): registration writes **no** row
in `products_reference_watermark` — the registered set lives in `products_reference_producer`, and
the watermark table gains a row on first post. A sentinel timestamp is the poison-value class, and
row-absence is what P-D-59's *"deregistration removes the series"* already reads as.

**6. The two unnamed alarms are named on the named one's convention** (row 28):
**`reference_watermark_future`** (the future-watermark alert, aligned with `WATERMARK_FUTURE`) and
**`reference_breakglass_tripwire`** (the tripwire escalation). Prefix and case follow
`reference_watermark_stale`.

**7. Ingestion accepts unknown member ids and alarms** (row 30): the set is the producer's
authoritative claim, and an unknown `sku_id` can be **legitimate** — a producer's catalog lags
erasure, so `10`'s erasure of a SKU leaves the producer naming it until its next full-set post
replaces the set. Refusing a 10K post for one such id would wedge the producer on our lifecycle;
silence would hide a typo that silently frees a real SKU. So: accepted, counted per post, and alarmed
(**`reference_unknown_member`**, the fourth alarm, same convention) — visibility without refusal.
Erasure leaves member rows untouched; the next post replaces the set, as `inst-wm-tables` already
states.

- **The arguments against, stated**: arm 2 makes enabling the correction arm a deploy, deliberate but
  named; arm 4 fixes a partition key for a subject class whose `SUBJECT_TYPE` question (`06` row 47)
  is still open — the key stands whatever that answer is, but a reader could over-read it; arm 7
  accepts data that can be a typo, and the alarm is the only detector — a validation pass was
  declined on the erasure-lag measurement, not on cost alone.
- **Not changed**: the four watermark refusals, `inst-wm-tables`' set replacement, `10`'s erasure
  scope, and `06` §7 row 47.
- **Propagated**: `design/07-reference-signal.md` (§2/§4 the flag, the hash column, the absence rule,
  the aggregate, the alarm names; §6 twins for rows 1 and 13), `features/reference-signal.md`
  (`dod-reference-config`, `dod-breakglass-unavailable`, `dod-watermark-tables`,
  `dod-watermark-door`, `dod-reference-events`, `dod-tripwire`, §7's arithmetic and the seven rows
  answered).

#### P-D-70 — Read-models' six: the timeline's nature, the retirement signal, the stamp's home and its feed

- **Date**: 2026-09-01 (owner call, autonomous under the standing instruction; rows 10 — the
  dashboards' door and grant — and 25 — whether a timeline render may resolve an identity through
  `10`'s map — are **deferred to the owner**, the first minting an operator surface with several
  sub-choices, the second a privacy-adjacent three-way fork)
- **Context**: `features/read-models.md` §7 rows 6, 7, 13, 14, 20 and 21.

**1. The history timeline is a request-time read over frozen rows, and frozen rows are not
write-path for C1's purpose** (row 6). C1 exists to keep browse and search off the **head** tables —
contention and head-state dependence — and `products_entity_version` is append-only immutable
history; a read on it contends with nothing. §3.1 declaring no history table is the design's choice
already made: materializing would need a table the slice deliberately lacks.

**2. What tells the projector a Product retired is the Product analogue of `SkuRetirementEffective`,
and its mint is `04`'s already-registered item** (row 7). Nothing else can carry it: the effective
flip may trail `effectiveAt` (the D-47 guard), so no clock and no head read can substitute. Until
`04` mints it the projector has no signal and a retired Product stays browsable — the defect this row
measured, now pinned on the owning slice's own §6 entry rather than floating.

**3. `projectedAt` advances on every projector apply, version or none** (row 13) — the bootstrap of
a zero-version tenant is an apply and stamps it, so the sole freshness signal always has a writer —
**and every polled surface carries the stamp of its own table's last apply**, which is what C3's
every-response rule means for `products_read_delivery_state`, whose content bears no relation to a
catalog version.

**4. `retired` is retrievable at `p1` through the by-id read under an explicit state opt-in**
(row 14): the browse default stays exclusionary, the timeline stays `p2`, and the FR's `p1` promise
is met by the smallest surface that can carry it — no new route, one explicit parameter, never the
default.

**5. The stamp-advance step reads `products_catalog_version_entry`** (row 20): the event's
changed-entity list selects, the manifest supplies each entity's frozen version reference — the table
P-D-60 made exactly this shape. The head's `published_version` is refused as the source: it may be
ahead of the catalog version, and reading it breaches the three-column carve-out.

**6. The `StalenessStamp` persists as one per-tenant stamp row** (row 21), carrying the last
`catalog_version_id` and `projectedAt`. The alternatives fail a measured case: a column duplicated on
every projection row cannot answer an **empty** projection — the anchorless rebuild's own arm — and
derivation from the consumer checkpoint ties response metadata to broker internals.

- **The arguments against, stated**: arm 1 reads C1's clause purposively rather than literally —
  recorded so a stricter reading is a deliberate reopening; arm 2 names a mechanism whose event
  another slice must mint, so `dod-visibility` is determinate but not buildable until `04` moves;
  arm 4 widens the by-id read's parameter surface by one value.
- **Not changed**: rows 10 and 25 stay open (parked for the owner with the reasons above); C2's
  browse exclusion; the timeline's `p2` priority.
- **Propagated**: `design/08-read-models.md` (§2/§3 the six answers at their rules, §6 twins where
  carried), `features/read-models.md` (`dod-history-timeline`, `dod-projector`, `dod-visibility`,
  `dod-staleness-stamp`, `dod-dashboards` untouched, §7's arithmetic and the six rows answered);
  `design/04-lifecycle.md`'s §6 item on the missing Product analogue is now **load-bearing** and is
  annotated as such rather than answered for its owner.

#### P-D-69 — Bulk's remaining seven: the machine completed, the mode named, the lane's key and digest fixed

- **Date**: 2026-09-01 (owner call, autonomous under the standing instruction)
- **Context**: the seven §7 rows of `features/bulk-promotion.md` that jointly held
  `dod-resume-abandon`, `dod-import-door`, `dod-idempotency-lane` and `dod-bulk-lifecycle` (rows 5
  and 15 carried, answered in `design/09` §6 first).

**1. The machine gains `abandoned`, and `failed` gains its entry** (row 5). `reported → abandoned`
fires when the batch approval is **rejected or explicitly withdrawn**, executes `inst-bm-resume`'s
abandon procedure — created drafts discarded, update-drafts reverted, pending live-ops dropped — and
releases the tenant slot; `abandoned` is terminal. **`failed`'s entry is the worker's attempt-budget
exhaustion** — `staging → failed` or `committing → failed` when the P-D-54 claim's `attempt` budget
runs out, exactly `inst-ar-failure`'s arm on the activation runner — while row failures stay
row-local and never enter it. Rows 6 and 7 (the never-approved slot, edge 3's executor) are
untouched.

**2. Promotion mode is a batch-level request field** (row 15): `mode ∈ {import, promote}`, default
`import`. Under `import` a bound `skuCode` with different content is `DUPLICATE_CODE`; only `promote`
engages the `PromotionResolver`'s update-as-draft. Per-row mixing was declined — a mixed batch is two
batches, and a silent auto-update on collision would convert typos into overwrites, which is what
`DUPLICATE_CODE` exists to refuse.

**3. `PreAuthorized` names the batch through the widened subject, and needs no revision operand**
(row 19). **P-D-67 arm 4** made `(subject_kind = bulk_batch, subject_ref = batch_id)` expressible at
the gate. The revision half is not the gate's: **P-D-54 edge 2** already pins the approval's stored
snapshot as the report plus the ledger's per-row revisions, so each per-row publish is checked
against **its own ledger pin**, row-locally, and the gate in this mode verifies only that the named
record was consumed for this subject. `features/governance.md` §7 row 27's measured text stands —
the mode carries no membership operand — because membership is the ledger's, not the gate's.

**4. The `internal:bulk-row` outcome record is the ledger row's disposition** (row 20): the response
columns store the synthetic `200` (P-D-42's shape) with `{disposition, code, reason, entity_id,
published_version}` as the body — the same record the P-D-61 read door returns per row — so
crash-resume replays the stored outcome instead of re-executing a published row.

**5. One digest rule for all three internal lanes** (row 24): an internal lane's `payload_hash`
digests the **canonical serialization of the act's own input record** — the bulk row's staged
payload, the `ScheduledTransition` row for `internal:scheduled-activation`, the cascade leg for
`internal:cascade-leg` — which is what makes a replayed key with different content detectable, the
column's whole purpose, with no third shape for `chk_products_idempotency_response_group` to admit.

**6. The lane's `client_key` is the ledger row's surrogate id** (row 25) — P-D-26's *"its own id in
`client_key`"* read at its natural referent. A row re-listed in a **new** batch has a new ledger row
and therefore a new key: the new-act rule holds with **no batch column added** to the shipped
primary key.

**7. `05-governance`'s catalog DoD mints all four grant instances** (row 27): the shipped roster is
one closed set under a two-way set-equality assertion, and a closed set takes **one writer** — the
lesson this corpus has already paid for at four sites. This feature's doors consume the grants; the
catalog owns the roster.

- **The arguments against, stated**: arm 1 extends a slice-declared machine (two edges, one state) —
  the smallest completion that gives the rejection verdict somewhere to land; arm 2 adds a request
  field, the first on the import door; arm 5 states one rule over three lanes of which only one is
  this feature's — recorded here because the row asked for exactly that, with `01-foundation`'s
  storage owner named as the rule's keeper.
- **Not changed**: rows 6 and 7 stay open; the six-state roster keeps `failed`; `DUPLICATE_CODE`'s
  meaning; the shipped idempotency PK.
- **Propagated**: `design/09-bulk-promotion.md` (§1.7/§2 the machine and the mode, §4 the ledger
  outcome columns, §6 rows 5 and 15 answered), `features/bulk-promotion.md` (§4 the machine,
  `dod-resume-abandon`, `dod-import-door`, `dod-promotion-resolver`, `dod-idempotency-lane`,
  `dod-bulk-lifecycle`, `dod-batch-state-machine`, §7's arithmetic and the seven rows answered),
  `design/01-foundation.md` **owed**: the lane-digest rule's one-sentence home is 01 §3.2/§4.4, filed
  there rather than edited in this round.

#### P-D-68 — Governance's own queue: the override ack's column, the expiry event's one emitter, the review's discharger

- **Date**: 2026-09-01 (owner call, autonomous under the standing instruction; row 18 — what an
  elevation changes about the authorization decision — is **deferred to the owner**, its co-owner
  being the ToolKit and both candidate mechanisms living outside this gear)
- **Context**: `features/governance.md` §7 is a table the sole-blocker recipe never parsed, so its
  queue surfaced only when a table-aware pass ran: rows 10, 19, 20 and 23 jointly held six DoDs.

**1. The `N = 0` override acknowledgment gets its own nullable columns on `products_approval`**
(row 10). The decision row already carries *"reason · override acknowledgments · instant"*, but at
`N = 0` no decision row exists — the author is not an approver and has no verdict. So the approval
record gains nullable **`author_override_ack`** (the named findings acknowledged) and
**`author_override_ack_at`**, written by the **submit door** only when the effective quorum is zero —
the P-D-50 convention again: a fact gets a column instead of parameterizing someone else's row.
Decision rows keep theirs for `N ≥ 1`; audit carries both as already stated.

**2. `BreakGlassExpired` is emitted exactly once, by the first post-expiry act, via a CAS stamp**
(row 19). The measured defect: the only named producer is a refused call, so an untouched session
emits nothing and a session called ten times emits ten. The mechanism is assembled from the set's own
precedents: the session row gains **`expired_emitted`**, flipped by CAS in the same transaction as the
first post-expiry refusal — **the winner emits, a replay emits nothing** (P-D-54's mechanism) — and a
session never touched after expiry emits no event at all: its expiry is a stored fact (`until`
passed), observable as a **gauge** with the alerting rule on top (P-D-59's mechanism), which is also
what the post-hoc review alert keys on. **In-flight acts complete**: expiry gates **admission** — an
elevated read admitted inside the window finishes; the gate judges at admission, as every
claim-shaped mechanism in this set does.

**3. The post-hoc obligation's state set is `{pending, reviewed}`, and the discharger is the second
platform principal** (row 20). Rule 1 already says an elevation is *"two-person-approved **or**
post-hoc-reviewed"* with a fixed floor of two distinct platform principals — one ceremony, two
timings. So the review **is** the second principal's decision arriving after the fact: it writes
`reviewed_by (actor_ref)` / `reviewed_at` and flips the state, and **no new door or grant is
minted**. Whether that decision's record is an `ApprovalRecord` stays its own open §6 item,
deliberately not presupposed here — this arm names the discharger and the state set, nothing about
the record's shape.

**4. Row 23 is a filing, not a decision**: the row itself closed on re-measurement (C3 already
carries the widened exception), and the stale `design/05` §6 bullet it names is struck in the same
change.

- **The arguments against, stated**: arm 1 adds two columns for a ceremony variant (the alternative —
  a synthetic decision row with the author as approver — would break C2's *"one principal, one
  decision"* UNIQUE and the two-person invariant it enforces); arm 2's event is conditional on a
  post-expiry touch, which is deliberate — an event nobody's act produced would need a sweeper, and
  the quiet case is the gauge's; arm 3 leans on the one-ceremony reading of *"two-person-approved or
  post-hoc-reviewed"*, and a later decision that the post-hoc review is a different ceremony would
  reopen the discharger, not the state set.
- **Not changed**: C5's read-only boundary, the fixed floor of two, `BREAKGLASS_EXPIRED`'s refusal
  semantics, and row 18's question — the elevation-vs-authorization seam stays open with the ToolKit
  co-owner, **parked for the owner** rather than decided.
- **Propagated**: `design/05-governance.md` (§4 the approval columns and the session columns, §2's
  expiry and override rules, §6 items answered for rows 10, 19, 20 and the row-23 bullet struck),
  `features/governance.md` (`dod-override-ceremony`, `dod-approval-store`, `dod-breakglass-expiry`,
  `dod-governance-events`, `dod-breakglass-store`, `dod-supersede`, the table rows and the section
  arithmetic).

#### P-D-67 — The catalog-version sweep: nine rows, every answer forced by a measurement already in the set

- **Date**: 2026-08-31 (owner call, taken under the standing instruction to decide where the
  measurement is dominant; rows 8 and 16 are carried and answered in `design/06` §6 first)
- **Context**: the nine §7 rows of `features/catalog-version.md` that jointly held five DoDs —
  `dod-version-counter`, `dod-coalescer`, `dod-participant-set`, `dod-force-completion`,
  `dod-liveness-and-release` — plus `features/governance.md` row 29, which is the same seam defect as
  row 26 seen from the other side.

- **Decision, nine arms**:

  | # | row | call |
  |---|---|---|
  | 1 | §7.8 | **The capture-store copy of `participant_set_snapshot` is authoritative and inside the checksum; the version-row copy is a derived cache**, annotated exactly as `freeze_state` on the same row already is. One byte-identity, one convention |
  | 2 | §7.16 | **`staged_at` is struck.** It has no admitted writer — an insert at stage would burn gapless ids on every `STAGED_ENTITY_CHANGED` refusal — and **no reader**: the SLO measures from `requested_at`, and nothing else names it. A column with neither is the `superseded` pattern again |
  | 3 | §7.23 | **The counter's initial value is pinned: `1`.** Gapless, monotonic, per tenant — every fixture already assumes a low start, and no other value has an argument. That makes the dev-space ordering hazard real, so the second half routes: **the sweep is pricing's**, whose table (`pricing_plan_revision.pending_version_ref`), dev module and doc (*"nothing here should outlive one"*) it is — recorded as pricing-owed, not authored here |
  | 4 | §7.26 + governance §7.29 | **The gate's subject widens to the approval store's own pair, `(subject_kind, subject_ref)`**, with `EntityRef` remaining the constructor for the entity kinds. The store already fixed the vocabulary (`bulk_batch`, `governed_live_op`, `system_signal`, `sku_correction` beside the entities), so the seam expressing less than the store records was the defect — the store is the authority, the seam conforms |
  | 5 | §7.29 | **The per-tenant increment lease's cardinality is accepted.** `bss-ledger` already runs finer keys in production — `recognition-run:{tenant}:{period_id}` and `period-close:{tenant_id}:{legal_entity_id}:{period_id}` — so the objection dissolves against the precedent |
  | 6 | §7.31 | **The four routes are declared, on the increment door's own pattern** (a contract with an in-process default and an S2S/REST binding): `POST /bss-products/v1/catalog-versions/{catalogVersionId}/acks` and `…/releases` (S2S, participant identity — P-D-18's door), `…/force-completions` (the operator ceremony), and `POST /bss-products/v1/freeze-participants` (the governed set write). *"Admitting the grants are unspent"* was declined: it would retract P-D-18/P-D-19, closed records the lifting path rests on |
  | 7 | §7.32 | **The five-minute maximum is the interactive lane's.** For the bulk lane the same p95/max applies **from window close**, because a batch whose window closes at the five-minute hard max cannot also publish within five minutes of its earliest request — the SLO as written was unsatisfiable for every bulk batch that ran to its bound |
  | 8 | §7.33 | **The participant's own release door does not stamp `released_at`.** The column exists so the release fact *"cannot be read as … a release through the participant's own door"* — it is the force-completion ceremony's alone; a door-released row is `state = released`, `released_at` NULL, and the retention gate's two arms read exactly that |
  | 9 | §7.46 | **The ledger rows are seeded by the increment transaction**: one row per `participant_set_snapshot` member, `state = pending`. So `pending` is live, P-D-60's machine has its entry point, the *"empty ledger satisfies all acked"* hazard dies, and **the ack door becomes an UPDATE whose row-existence is the membership check** — a non-member's ack has no row to flip. P-D-49's snapshot rule stays as the defensive belt |

- **The arguments against, stated**: arm 4 widens a shipped seam type (code cost, deferred to the
  build); arm 6 mints four routes in one decision — the largest surface addition of the day, taken on
  the same forcing P-D-61 was (a contract with nothing to send to); arm 9 adds a seeding fan-out to
  the increment transaction — one row per participant per version, which at the v1 set of one
  participant is one row.
- **Not changed**: `freeze_state`'s roster and its derived-cache annotation, the coalescing windows,
  P-D-49, P-D-60's six edges, and pricing's dev module.
- **Propagated**: `design/06-catalog-version.md` (§2 SLO scoping and door routes, §4 `staged_at`
  struck / the derived-cache annotation / the seeding / the counter start, §6 items answered for the
  two carried rows), `design/05-governance.md` (four roster cells gain their routes),
  `features/catalog-version.md` (the five DoDs, §7's arithmetic and the nine rows answered),
  `features/governance.md` (row 29 answered; its DoDs carry the widened seam).

#### P-D-66 — The `status` pin entry: token `status`, spelled per side, vocabulary of two

- **Date**: 2026-08-31 (owner call, taken under the standing instruction to decide where the
  measurement is dominant — both halves were already answered by `inst-sdk-catalogsku` and the rows
  had not connected the texts)
- **Context**: `features/consumer-contracts.md` §7 rows 24 and 34, jointly the last blockers of
  `dod-schema-pin` and `dod-status-vocabulary`. Row 24: the shipped registry `Sku` carries
  `lifecycle_state` while the seam shape names the member `status`, and no document said which
  spelling the pin file uses. Row 34: this document pins a two-member wire vocabulary while pricing's
  shipped client doc reads *"`draft` | `published` | `deprecated`, verbatim. Not an enum:"* — three.

- **Decision, both halves from the seam contract already in force**:

  | Call | Propagation |
  |---|---|
  | **The pinned token is `status`** — the seam's name, fixed by `CatalogSku`-superset-compatibility, which `dod-catalogsku-shape` already decided and pricing's shipped `CatalogSku.status` already carries. **The entry records the registry-side source spelling as an annotation** (`registry-field = "lifecycle_state"`), so the job can resolve each side by its own name; pinning `lifecycle_state` instead would make the comparison against the consumer's shipped member impossible | `dod-schema-pin`; `dod-status-vocabulary` |
  | **The vocabulary is two members, `published` and `deprecated`** — `inst-sdk-catalogsku` M4 is already normative on it: *"browse serves `published\|deprecated` only (draft never served, retired history-only — 08 C2)"*, with the SDK enum documenting all five states and the wire subset named. Pricing's `draft` is display-tolerance prose — its own doc keeps the field a string so *"a fifth state must not become a parse failure in the gear that merely displays it"* — and the pin **replaces** that tolerance rather than adopting its list | `dod-status-vocabulary` |

- **The cost, recorded**: a registry-side `CatalogSku` read shape with a `status` member introduces a
  second spelling of `lifecycle_state` inside this gear's own SDK, against that crate's stated
  one-spelling rule. The seam contract wins because it is shipped on the consumer's side; the rule's
  purpose — no third spelling invented ad hoc — survives, the second spelling being the seam's, not a
  convenience.
- **Not changed**: `LifecycleState`'s five variants and its `parse`, pricing's `CatalogSku` type, and
  browse's visibility rules (08 C2).
- **Propagated**: `features/consumer-contracts.md` (`dod-schema-pin`, `dod-status-vocabulary`, §7's
  arithmetic and rows 24 and 34 answered). `design/12` §2's `inst-sdk-catalogsku` already carries both
  halves and is unchanged.

#### P-D-65 — `CatalogVersion` is a pin entry of kind `surface`, delegated to the port trait and compared by nothing

- **Date**: 2026-08-31 (owner call — the last sole-blocking row of the 2026-08-31 queue)
- **Context**: `features/consumer-contracts.md` §7 row 33 measured a conflict between two confirmed
  texts. **P-D-12** says `CatalogVersion` is *"pinned as a **surface**, not a field"*; lint 9's
  grammar makes a `(surface)` marker *"outside the pin by construction"*. Five of the register's
  fourteen rows carry `` `CatalogVersion` (surface) ``, and the pin file's schema for a surface-level
  member had to be settled before `dod-schema-pin` could be written.

  **What the surface concretely is**: the `CatalogVersion` type the counterpart port carries —
  `bss_pricing_sdk::CatalogVersionRegistryV1::committed_version` returns `Option<CatalogVersion>` —
  and its drift protection already exists structurally: *"when the registry publishes its own SDK
  this trait becomes an adapter over it"*, at which point the **compiler** checks the shape, which is
  strictly stronger than a TOML comparison. Before the adapter lands, `bss-products-sdk` carries no
  `CatalogVersion` type, so a pin comparison would have nothing to compare on this side either way.

- **Decision**: both sentences become literally true. **The pin carries a `CatalogVersion` entry of
  kind `surface`; the CI job neither compares it nor asserts its absence — its comparison is
  delegated to the port trait**; and lint 9's formulation narrows from *"outside the pin"* to
  *"outside the **field-comparison** population"*.

  | Call | Propagation |
  |---|---|
  | **A third entry kind, `surface`**: `kind = "surface"`, a `delegated-to` naming the port trait, **no comparability flag** — P-D-57's flag governs what the job compares, and this entry is compared by nothing, so carrying the flag would claim a comparison that never runs | `dod-schema-pin` |
  | **Lint 9 couples the five annotated markers to the surface entry**: a `` `CatalogVersion` (surface) `` token satisfies the operand→pin direction against the surface entry, and the surface entry's pin→operand direction is satisfied by those five rows. `payload` and `none in v1` couple to nothing, as before | `design/12` §3.2 lint 9; `dod-lint-pin-coupling` |
  | **P-D-12's sentence stands unreinterpreted** — the entry exists, as a surface and not a field — which is the point: re-reading a confirmed decision that five register rows and C1 cite is a retraction with a radius, and the entry costs one TOML kind instead | `design/12` §1 C1 |

- **The argument against, stated**: a third entry kind in a file that does not yet exist is schema
  growth for one row, and a `delegated-to` field is a claim the job never exercises — if the adapter
  promise is withdrawn, the entry silently protects nothing. That risk is accepted and recorded: the
  entry's honesty rests on the adapter sentence in the pricing-side port doc, which P-D-65 now cites
  from a second place.
- **Scope**: nothing pricing-side changes — the trait, its `CatalogVersion` type and the adapter
  promise stay as shipped. The five register cells stay exactly as P-D-63 normalized them. The event
  surface stays outside the pin entirely: `payload` rows gain no entry.
- **Not changed**: P-D-12's membership rule and its wording, P-D-57's comparability flag for field
  members, and the coupling rule's field direction.
- **Propagated**: `design/12-consumer-contracts.md` (§1 C1's pinned-as-a-surface clause, §3.2 lint
  9's narrowed formulation), `features/consumer-contracts.md` (`dod-schema-pin`,
  `dod-lint-pin-coupling`, §7's arithmetic and row 33 answered).

#### P-D-64 — The missing-sign-off refusal rides `VALIDATION`, and the owned roster stays at one

- **Date**: 2026-08-31 (owner call)
- **Context**: `design/10` §6 asked what code `inst-pp-allowlist`'s refusal carries — an allow-list
  entry *"offered without a mandatory Legal sign-off reference"* is refused, §5 asserts that refusal
  with a positive control, and §3.2 declares no code for it. The item's own arms: ride 01's
  `VALIDATION` or declare a slice code.
- **Decision**: it **rides `VALIDATION`**. A missing mandatory member of the offered entry is a
  shape-class refusal, and 01's convention for those is
  `violate("VALIDATION", <field>, <detail>)` — the caller's discriminator is the violation's
  **field**, exactly as for every other missing-field refusal in the gear, so the SDK error enum's
  `VALIDATION` member is the member this refusal uses. **P-D-52's counter-precedent does not
  transfer**: there the wire shape was forced by a consumer that discriminates on the specific code;
  nothing discriminates on this one. And `dod-retention-error-taxonomy` holds *"One code is the whole
  owned roster"* as a measurement — `ERASURE_UNKNOWN_ACTOR` — which a second minted code would break
  for a refusal the ordinary machinery already classifies.
- **The argument against, stated**: a caller automating allow-list submission cannot switch on a code
  to detect specifically this refusal; it must read the violation's field. That is the same position
  every shape refusal in the gear puts a caller in, and routine-ness does not create a taxonomy entry
  when the discriminator already exists.
- **Not changed**: `ERASURE_UNKNOWN_ACTOR`, its 422-architectural response, the alarms-not-errors
  posture of the GC and the drill, and `inst-pp-allowlist`'s refusal condition itself.
- **Propagated**: `design/10-retention-erasure.md` (§2 `inst-pp-allowlist`'s code, §6 answered),
  `features/retention-erasure.md` (`dod-pii-allowlist`, `dod-retention-error-taxonomy`, §7's
  arithmetic and row 13 answered).

#### P-D-63 — The `Operand` grammar gains marker annotations, and `+` is normalized out of the cells

- **Date**: 2026-08-31 (owner call — amends **P-D-43** arm 3)
- **Context**: `design/12` §6 measured that lint 9's cell grammar — *"one token per pin member,
  comma-separated, each either a catalog field name or one of three non-field markers"* — does not
  describe the cells it reads. Measured at HEAD: the register has **fourteen** rows (the item said
  thirteen, and the FEATURE's `dod-obligation-register` already says fourteen). Three cells fit the
  grammar (`compositionPending`, `sellable`, `skuId`); **six** lead a non-field marker with a
  backticked identifier (five `` `CatalogVersion` (surface) ``, one `` `SkuRetired` payload ``) —
  and a backticked identifier is not prose under any form the grammar states; **four** join with
  `+` rather than a comma (the item said three), and what `+` joins is a field with a **phrase**
  (*"its value vocabulary"*, *"the metering-unit declaration"*), not two field names; one is
  `none in v1` with a prose parenthetical, which the grammar already covers.
- **Decision**: one production is added and one separator is refused.

  | Call | Propagation |
  |---|---|
  | **A non-field marker may be preceded by exactly one backticked identifier, which the marker consumes as its annotation.** `` `CatalogVersion` (surface) `` and `` `SkuRetired` payload `` are each **one token**; the identifier names the surface or payload and the lint does not look it up in the pin | `design/12` §3.2 lint 9 (amends **P-D-43** arm 3) |
  | **`+` is not admitted to the grammar — it is normalized out of the four cells**, each rewritten to comma-separated pin tokens: `` `status` + its value vocabulary `` → `` `status` `` (the vocabulary is part of that pin member's own definition); `` `PlanTier` + the metering-unit declaration `` → `` `PlanTier`, `unit`, `usageTypeRef` ``; `the metering-unit declaration + `usageTypeRef`` → `` `unit`, `usageTypeRef` `` (the second half was redundant, the declaration being the pair); `` `type` + the metering-unit declaration (07 C4's bucket-ii set) `` → `` `type`, `unit`, `usageTypeRef` `` with the parenthetical staying as ignorable prose | `design/12` §2.2, the four cells |
  | **After both, all fourteen cells parse**: five surface-annotated, one payload-annotated, one `none in v1`, three already clean, four normalized | `dod-obligation-register`, `dod-lint-pin-coupling` |

- **The argument against, stated**: fitting the grammar to the cells risks the opposite failure —
  a grammar grown until everything parses checks nothing. That is why `+` was normalized out rather
  than admitted: the annotation production encodes one real distinction (a marker's referent), while
  admitting `+` would have encoded a typographic habit.
- **Two sub-questions the carry recorded land with it.** (i) **A backticked catalog field name is a
  token, not prose** — the cells' own convention writes every field token backticked, and the
  conforming class of the FEATURE's census is exactly "one backticked field token and nothing else";
  the amendment states it so *"prose beside the tokens is ignored"* can never be read as swallowing a
  backticked identifier. (ii) **A cell whose only token is `none in v1` is outside lint 9's coupling
  population by construction** — the rule the markers already carry: a marker token is outside the
  pin, and a row with no operand couples nothing in either direction.
- **Not changed**: the three markers, the comma, *"prose beside the tokens is ignored"*, lint 9's
  coupling rule itself, and the pin's membership.
- **Propagated**: `design/12-consumer-contracts.md` (§3.2 lint 9's grammar, §2.2's four cells, §6
  answered), `features/consumer-contracts.md` (`dod-obligation-register`, `dod-lint-pin-coupling`,
  §7's arithmetic and row 15 answered).

#### P-D-62 — Clone suffixes: the first free integer, decided by the index under the reservation

- **Date**: 2026-08-31 (owner call)
- **Context**: `design/11` §6 measured that `N` in `{name}-copy-N` / `{source}-copy-N` is never
  defined (per-source counter, global, first free integer), that `-revived` carries no counter while
  the uniqueness index admits **one holder per name in every non-`discarded` state** — so a second
  revival of one lineage produces a suggestion the registry must refuse — and that concurrent clones
  computing `N` by a read race each other.

  **The gear has already chosen this mechanism twice.** `inst-cn-identity` already says the suggested
  code is *"reserved atomically"*; **P-D-37** established that the `identity` phase is *"decided by
  the index under the write"*; and **P-D-42** adopted the donor's sentence that *"the gate is the
  insert, not a lookup"*. A read-then-suggest is exactly the arrangement those two decisions
  dismantled elsewhere.
- **Decision**: three arms.

  | Call | Propagation |
  |---|---|
  | **`N` is the first free integer for the suggested string, decided under the reservation.** The clone door reserves `{name}-copy-N` (name) and `{source}-copy-N` (code) starting at the lowest free integer; a reservation conflict moves to the next free one and retries. Two concurrent clones of one source get `-copy-2` and `-copy-3` — the index arbitrates, no lock and no counter column exists | `design/11` §2 `inst-cn-identity`, `inst-cn-rename` |
  | **A second revival of one lineage suggests `{name}-revived-N`**, the same first-free rule over the `-revived` family, so the flavor survives and the suggestion path never produces a refusal. The alternative — falling back to `-copy-N` — was declined because it silently drops the one signal `-revived` exists to carry | `inst-cn-rename`; `dod-rename-rule` |
  | **The operator path is untouched**: a collision on an operator-supplied name or code stays the ordinary `DUPLICATE_NAME`/`DUPLICATE_CODE`, exactly as both instructions already state | `dod-clone-identity` |

- **The argument against, stated**: `-revived-N` is a small invention — the slice names only
  `-revived` — and retry-under-reservation costs a loop at the door where a counter column would cost
  one read. The column was declined because it is a value that can drift from the thing it counts
  (**P-D-40** declined a reference counter on the same ground), and the loop's iterations are bounded
  by the lineage's own clone count.
- **Scope**: the suggestion for a clone of a clone (`X-copy-2-copy-1`) is left as the rule computes
  it; nothing here shortens or rewrites base names. Whether the reverse lineage lookup gets a surface
  stays its own §6 item.
- **Not changed**: the disposition table's rows, the null-code arm (a source Product with no
  `productCode` suggests none), and the index scopes.
- **Propagated**: `design/11-clone.md` (§2 `inst-cn-identity` and `inst-cn-rename`, §6 answered),
  `features/clone.md` (`dod-clone-identity`, `dod-rename-rule`, §7's arithmetic and row 4 answered).

#### P-D-61 — Three carried rows of `09-bulk-promotion`: a read door, an authored §4, and eight `no event` markers

- **Date**: 2026-08-31 (owner call — the second round over **carried** rows)
- **Context**: `design/09` §6's three sole-blocking items. Each was decided by measuring what the set
  already requires rather than by preference.

**1. The `RowLedger` gets a read door, because a PRD-level MUST demands a reader.**

C1 requires *"per-row success/failure reported — no hidden partial failure"* and `PRD.md` says the
same twice (§ *"report per-row success/failure"*, and *"track per-row success/failure"*). Measured
against the surface: the slice declares three routes — `POST bulk/imports`, `GET bulk/exports`,
`POST bulk/lifecycle` — and **none reads a batch**. The export door is 06's manifest under
`catalog_version × read`, deliberately decoupled. `05`'s RBAC roster mints only the two execute
pairs. `08` projects no bulk read model, and §4 says export artifacts are *"streamed, not stored"*.
So the door answers **202**, the caller holds a batch id, and nothing resolves it — the same shape
`design/06` §6 records as the doorless `committed_version` poll.

| Call | Propagation |
|---|---|
| **One read route**: `GET /bss-products/v1/bulk/batches/{batchId}` → the batch state (§4's six, P-D-54) plus its `RowLedger`, one entry per row with its disposition, code and reason. **One route for both lanes**, the key being the batch id and not the lane | `design/09` §2 new `inst-bk-read`; `dod-bulk-errors` |
| **Its own grant, `bulk × read`.** Not `bulk × execute` — a reader is not an executor, and the finance reviewer who signs batches must read without gaining the right to start one. Not `catalog_version × read`, which is the export's, auditor-shaped over a manifest and decoupled on purpose | `design/05` §RBAC roster; `dod-bulk-errors` |
| **The four per-row codes' statuses now have the surface their own clause was waiting for** — *"the status below applies only where a caller asks a single row's disposition"*. That caller exists | `design/09` §3.2 |

**The argument against, stated**: minting a route and a grant is authoring API surface, and the row's
co-owner is the contract owner. The price is 05's roster plus **three route censuses, not two**. The
cheaper arm — declaring the statuses dormant — was declined because it leaves a PRD-level **MUST**
unmet, which is worse than an owed census.

**2. §4 is authored from the operands already stated, and from nothing else.**

§4 is two sentences where every sibling slice carries a normative shape. Nothing needs inventing: the
row enumerates the values with a stated writer and no column — the per-row pinned revision, the batch
and row keys, the row disposition and `reason`, the pending `GovernedLiveOp` payload, the itemised
override set, and `operation_key` — and P-D-54 adds the six states with the worker's claim and lease.
Two constraints bound the authoring: **`reason` is a literal from a closed set, never operator text**
(**P-D-50** — `batch-abandoned` is a constant), and the **`ChangeReport` is derived**, carrying
*"the itemised override-carrying rows (`skuCode` per row)"*, so it needs no table.

Nothing beyond that list is added: no counters that duplicate the ledger, no report table, no free
text.

**The argument against, stated**: a schema in a design document commits migrations, and this §4 was
deliberately thin. But `cpt-cf-bss-products-dod-bulk-tables` cannot be met without it, and thinness
here is the outlier across twelve slices rather than a convention.

**3. The `no event` marker goes on eight instructions, and the row's premise was half wrong.**

Row 18 says *"01 states the rule over every slice and 12 lints it"*. **Lint 12 reads only the
`EventRegister` table** — *"The register is authored, never harvested"* (**P-D-45**), after five
harvest passes returned 31, 24, 32 and 35 events — so it lints the register, never the instructions.
What 01 supplies is the **convention**: an inline `**no event**` marker, in exactly that form on
`inst-fd-actor-ref-mint`, `inst-fd-actor-ref-seen` and `inst-fd-gate-rejection`.

Measured over `design/09`: **13 instructions, of which one names an event** — `inst-bk-complete` with
`CatalogBulkOperationCompleted`, and none records "no event". The marker goes on the eight that change
state: `inst-bk-keys`, `inst-bk-stage`, `inst-bk-report`, `inst-bk-commit`, `inst-bk-override`,
`inst-pm-resolve`, `inst-bl-lifecycle`, `inst-bm-resume`. The remaining four need none —
`inst-bk-export` is a read, `inst-bm-tables` and `inst-bm-limits` are declarative, and
`inst-pm-review` states a review step rather than a write.

**The reason is already written**, in `dod-coalesced-event`: row-level domain events are emitted by
01's doors that the rows drive, so **the acts are announced — just not by this slice's
instructions** — and the batch's own history is the ledger, which is audit-plane (**P-D-21**: the
audit table holds only what emits no event).

**The row's count is short by two, and the classification is named so it is checkable**: it says six,
the measurement gives eight. The two the row's count omits are a judgement about what counts as
state-changing, which is why all eight are enumerated rather than totalled.

- **Scope**: this decision does not author `design/09`'s `EventRegister` rows — that is owed per slice
  by `design/12` §6, and this slice's owing is exactly one row (`CatalogBulkOperationCompleted` →
  `inst-bk-complete`). It does not decide the job home of anything, does not touch the `ChangeReport`'s
  content, and does not answer §6's other items — the rejection edge, the abandon state, the `failed`
  entry edge, or what ends a never-approved batch.
- **A second grant was considered and declined**: a separate read grant for lifecycle batches, on the
  argument that `bulk_lifecycle × execute` is its own grant because that door is the gear's most
  destructive. Reading a ledger is not destructive and both lanes' rows carry the same shape, so one
  `bulk × read` covers both; the alternative is recorded here rather than left to be rediscovered.
- **Not changed**: the two execute grants, the export door's grant, `08`'s read models, and the
  streamed-not-stored export.
- **Propagated**: `design/09-bulk-promotion.md` (§2's new `inst-bk-read` and the eight markers, §3.2's
  status note, §4 authored, §6's three items answered), `design/05-governance.md` (the RBAC roster row),
  `features/bulk-promotion.md` (§2's read scenario, `dod-bulk-errors`, `dod-bulk-tables`,
  `dod-coalesced-event`, the grant census, §7's arithmetic and rows 8, 9 and 18 answered).

#### P-D-60 — Four carried rows of `06-catalog-version`: two events, two tables, a struck state value, and six edges

- **Date**: 2026-08-31 (owner call — the first round over **carried** rows, answered in the slice and
  then in the carry)
- **Context**: `design/06` §6's four sole-blocking open items, taken together because they are one
  document's and each turned out to be partly answered by text already in the set.

**1. The composition-clear re-publish emits both events.**

`inst-cc-clear` routes the clear through 01's publish door as a *"system save + re-publish of the head
(version N+1)"*, `inst-fd-publish-emit` fires `ProductPublished`/`SkuPublished` unconditionally, and
the crate's own event module says of the version field: *"`06` reads this as the content pointer and
`08`'s projector keys on it"*. So suppressing `SkuPublished` would leave the read model one version
behind on exactly the entity whose flag just changed. `SkuCompositionCleared` is **additive**, carrying
the semantic fact a bare publish does not distinguish. **Both name the same entity and the same
`publishedVersion`**, so a consumer keyed on version sees one version change — no consumer obligation
is created. 09's additivity rule is *not* widened; it stays scoped to its coalesced summary and this
act states its own.

**2. The capture store is its own table.**

§4's one bullet gave one name two disjoint keys — `(tenant_id, catalog_version_id, entity_kind,
entity_id)` and `(tenant_id, catalog_version_id, capture_kind)` — and two disjoint column sets, one
holding `published_version` as a reference into `products_entity_version`, the other a stored
canonical copy. One PK cannot express both, and on the one-table reading every column of both halves
becomes nullable, admitting a row that is neither a valid entry nor a valid capture — the class this
gear's CHECK constraints exist to refuse. So `products_catalog_version_entry` keeps the entity half
and **`products_catalog_version_capture`** takes the capture rows.

**P-D-40 needs no re-aiming, and the row's own owner clause was inverted on this point.** Its
predicate is written over `products_catalog_version_entry`, which the entity half keeps; two tables is
the arm under which the predicate and its index
`(tenant_id, entity_kind, entity_id, published_version)` are exactly right as written, with no
capture rows scanned and no dead index entries. Capture rows hold copies and reference nothing, which
is §4's own H3 fix — *"live content is copied, never referenced"* — so they never participated in that
predicate on either reading.

**3. `superseded` is struck; the increment transaction writes the other two.**

No instruction writes any of the three, and the roster's third value has no candidate writer at all:
`inst-sn-revalidate` says a failed mechanical run *"re-coalesces and retries fresh, the request never
lost"*, the PRD echoes *"A request is never dropped"*, an unregistered source is refused
`REQUEST_SOURCE_UNKNOWN` at the door before a row exists (**P-D-52**), and an idempotent replay is
caught by the `(tenant_id, source, request_key)` UNIQUE. Nothing supersedes a request. The roster
becomes **`(pending, coalesced)`**.

`coalesced` and `satisfied_by_version_id` are written by the **increment transaction** — the one that
allocates the id, builds the manifest, commits and emits `CatalogVersionPublished` carrying
`satisfiedRequests`. That set *is* the requests it satisfied, so the same transaction marks them and
stamps the FK; **P-D-50** gave the column its existence precisely so a replayed
`CatalogVersionPublished` can have that set rebuilt, which fixes its writer as whoever produces it.
`coalesced` is **terminal** — a satisfied request is history naming its satisfying version — which
answers *"and what leaves them"*.

**4. `products_freeze_ack.state`: six edges, and one of the three sub-questions was already answered.**

`dod-force-completion` already states the third: *"a forced participant that later recovers and acks
moves to `acked`, and `10-retention-erasure`'s gate reads the `(state, released_at)` **pair**, so the
stale stamp frees nothing."* So a later ack does **not** clear `released_at`; the state moving is what
makes the stamp inert, and `released_at` is write-once per registration. The other two follow from the
doors' own wording: force-completion records *"each **missing** participant"*, so it never overwrites a
row already `acked` or `released`; and the release door records that the participant *"holds no more
live references to that version"*, a precondition about references rather than about having acked, so
**`pending → released` is admitted** — a participant with nothing to freeze self-resolves without a
two-person ceremony.

| edge | door |
|---|---|
| `pending → acked` | the ack door |
| `pending → released` | the participant's own `catalog_version × release` door |
| `acked → released` | the same door (the `freezeComplete` regression this creates is §6's own separate item, unanswered here) |
| `pending → not_frozen(forced)` | force-completion, missing participants only, stamping `released_at` in the same transaction |
| `not_frozen(forced) → acked` | a recovered participant's ack; the stale stamp frees nothing |
| `not_frozen(forced) → released` | a recovered participant's own door — the other arm `VERSION_FORCED_INCOMPLETE` names |

**`released` is terminal**, and no transition other than the six is admitted. **The table has no entry
point, deliberately**: who writes `pending` at all is `features/catalog-version.md` §7 row **46**'s,
with §6's *"nothing creates the ledger rows"* item beside it, and both stay open.

- **The arguments against, stated.** (1) Two events per act oblige a consumer to de-duplicate; the
  shared `publishedVersion` is how, and that is a property of the payloads rather than a new duty.
  (2) Two tables mean two append-only guards and two migrations; the checksum still covers both
  halves, being computed over content rather than over a table. (3) Striking a roster value is a
  closed-set edit — measured at three sites in two files, and the same word in `04`'s
  `ScheduledTransition`, `05`'s `ApprovalRecord` and `01`'s approval rows is untouched. (4) A
  transition table without an initial state is incomplete on purpose, and a reader who misses that
  will look for the creation point in §4 rather than in row 46.
- **A carry-fidelity finding, recorded because it changed the scope of arm 3.** `design/06` §6 asks
  *"Who writes the request state `superseded`, and what leaves it?"* — one value. The FEATURE's
  carried row 10 widened it to *"the request states `superseded` and `coalesced`, and
  `satisfied_by_version_id`"* and added a P-D-50 sentence. The widening is correct on the measurement
  and is what makes arm 3 answer all three, but it is a **departure from verbatim that §7's preamble
  does not declare** — it lists three departures and question-widening is not among them.
- **Not changed**: the `capture_kind` value roster and its own count question, `freezeComplete`'s
  formula, `staged_at`'s missing writer, the resolution API's transport, and every other §6 item.
- **Propagated**: `design/06-catalog-version.md` (§2 `inst-cc-clear` and the increment rule; §4's
  entry/capture split, the request roster, the ack transition table; §6's four items answered),
  `features/catalog-version.md` (§4 mirror, `dod-cv-events`, `dod-composition-clear`,
  `dod-version-entry-table`, `dod-referential-delete-predicate`, `dod-request-queue`,
  `dod-freeze-ledger-tables`, §7's arithmetic and rows 1, 9, 10, 11 answered),
  `features/reference-signal.md` and `features/retention-erasure.md` — both cite row 9 as the
  unresolved capture-store question and must now read as resolved.

#### P-D-59 — `reference_watermark_stale` is an alerting rule over a gauge, so no fired-state is stored

- **Date**: 2026-08-31 (owner call)
- **Context**: `features/reference-signal.md` §7 row 27 measured that the alarm is described *"both as
  an output of a read and as a property of the registered set"*, that no verdict is stored so there is
  nowhere to record that it has already fired, and that `04-lifecycle`'s runner polls the predicate on
  a cadence — so a read-time emission *"alarms once per call"*.

  **The operand for a gauge already exists and is already stored.** `design/07` §4 declares
  `products_reference_watermark` — `(tenant_id, producer)` → `watermark_at`, `posted_at` — and
  `inst-wm-freshness` already says that *"the staleness alarm keys on the registered set so a retired
  producer stops alarming"*. So per-producer watermark age is derivable from a committed store over a
  set the gear maintains; nothing new is needed to observe it.

  **And the threshold is the gear's own exported config value, not a number in someone else's
  system.** `ProductsConfig` carries the freshness threshold (interim 15 min), and this feature
  already requires it exported *"because another feature already depends on reading it"* —
  `04-lifecycle`'s flip guard re-evaluating on the predicate's freshness cadence.

- **Decision**: `reference_watermark_stale` is an **alerting rule over a gauge**, not an emission from
  the predicate's evaluation.

  | Call | Propagation |
  |---|---|
  | **The gear exposes a gauge**: `now − watermark_at` per `(tenant_id, producer)`, over the **registered** producer set only. Deregistration removes the series rather than silencing an alarm, which is what `inst-wm-freshness` already promises | `design/07` §2 `inst-wm-freshness`; `dod-reference-predicate` |
  | **The alarm is the observability owner's rule over that gauge, and its condition references the gear's exported freshness threshold** rather than restating it. One number, one home | `dod-reference-events` |
  | **Nothing is raised per call and no fired-state is stored.** Repetition, for-duration and grouping belong to the alerting side, which is what dissolves the second half of the question: there is no verdict to persist because there is no per-call emission to suppress | `design/07` §2 `inst-rp-eval` |
  | **The predicate keeps its verdict unchanged.** `conservatively_referenced(stale, producer)` stays exactly as `inst-rp-eval` states, the per-producer detail already carrying `stale` — which is what `04`'s confirmation screen shows. What is corrected is only the reading of *"+ the `reference_watermark_stale` alarm"* as an emission the evaluation performs | `dod-reference-predicate`, and the §6 control that pairs `stale` with the alarm |

- **The argument against, stated**: the threshold lands in two places the moment an alerting rule
  restates it instead of reading it, and there is **no mechanical guard** against that — the
  protection is the requirement to reference the exported value. It is a smaller exposure than the
  alternative, which was to store a fired-state in this gear and own suppression, deduplication and
  re-arming for one alarm.
- **Scope — this does not answer §7 row 28.** Two of this feature's three alarms are unnamed (the
  future-watermark alert and the tripwire escalation) and naming them is that row's, owned by the
  observability owner with this feature. This entry decides the mechanism for the one alarm that
  **is** named, and the mechanism transfers to the other two only once they have names.
- **Not changed**: no store, column or config field is added; the freshness threshold's value and its
  config home are untouched, and `posted_at` remains read by nothing.
- **Propagated**: `design/07-reference-signal.md` (§2 `inst-rp-eval` and `inst-wm-freshness`),
  `features/reference-signal.md` (`dod-reference-predicate`, `dod-reference-events`, the §6 control,
  §7's arithmetic and row 27 answered).

#### P-D-58 — The replay fixtures are authorable now, against the SDK's own broker double

- **Date**: 2026-08-31 (owner call)
- **Context**: `features/consumer-contracts.md` §7 row 26 asked whether the replay contract is
  testable at all before `dyn EventBrokerApi` has a production registration, since every obligation of
  `cpt-cf-bss-products-flow-replay` — versioning, dedup, ordering, bootstrap — rides an arm this
  document's §1 calls *"inert in every real deployment"*. The unstated half was whether this feature's
  fixtures are authorable against the test registration or wait for the real one.

  **The double is not a local invention: it is public SDK API and this gear already depends on it.**
  `event-broker-sdk` exposes `pub mod mock` behind its `test-util` feature, `MockBroker` implements the
  trait (`src/mock/transport.rs:171`), and the module exports `MockBroker`, `MockBrokerHandle`,
  `StoredEvent` and `CursorEntry` — so the stored log and the cursors are inspectable, which is
  exactly the surface versioning, dedup, ordering and bootstrap assert over.
  `products/Cargo.toml` already takes `event-broker-sdk` with
  `features = ["outbox", "test-util"]` in its dev-dependencies.

  **And it is not a registration bypass, which is the part that decides the question.**
  `infra/broker_tests.rs` registers the topic and **all eight** event types through
  `MockBrokerHandle`, then performs `hub.register::<dyn EventBrokerApi>(broker)` into
  `toolkit::client_hub::ClientHub` — the same registration a production boot performs, with a
  different transport behind it. The event types are registered *"from the transcribed literals, so
  this is an agreement between two independent transcriptions rather than the gear agreeing with
  itself"*.

  **The boundary is already written by the gear, and this entry adopts it rather than drawing a new
  one.** `broker_tests.rs`: *"Both ends are in-process — `MockBroker` accepts with no network, no disk
  beyond the local `SQLite` outbox, and no ingest work"*, so what it bounds is *"enqueue, the
  sequencer, the leased processor's pickup, and the SDK's publish call"*, and *"Anything a real broker
  adds is on the other side of that boundary and belongs to whoever owns the `01/06` split."*

- **Decision**: the replay fixtures are **authorable now**, against `MockBroker` registered into
  `ClientHub` as `dyn EventBrokerApi`. They do not wait for a production registration.

  | Call | Propagation |
  |---|---|
  | **The suite's transport is the SDK's own double, under `test-util`** — not a fixture-local stub, so a change to the broker contract reaches the fixtures through the same crate the gear compiles against | `design/12` §2.1 joint fixtures; `dod-event-versioning`, `dod-dedup-ordering`, `dod-bootstrap` |
  | **The fixtures drive the gear's real registration path**, topic and all eight event types included, and the type registration is transcribed independently rather than read from the gear's constants. A fixture that injected a producer past `ClientHub` would assert the contract over wiring no boot performs | `dod-seam-suite-home` |
  | **The claim the green suite licenses is stated, and it is narrower than the obligation**: the contract holds over this gear's own path with a conforming transport. *"Events reach consumers in production"* is a different claim, it depends on the missing registration, and **no DoD in this gear owns it** | `features/consumer-contracts.md` §1 boundary |

- **A propagation item the round found**: `gears/bss/fixtures/bss-fixtures/Cargo.toml` declares **no
  dependency on `event-broker-sdk` at all**, with or without `test-util`. That is a second missing
  wire at the suite's home, beside the one `dod-seam-suite-home` already names — *"the dependency
  declared, the fixtures placed, and a job that runs them"* — and it is recorded there rather than
  filed as a new question, the DoD already owning the wiring.
- **The argument against, stated**: a suite green against a double proves the contract, not the
  deployment. The producer arm is inert in production, so every replay fixture can pass while no
  deployment runs the path. That is not removed by using a better double, and it is why the licensed
  claim above is written down: the suite's greenness is evidence about this gear's path, never about
  delivery.
- **Scope — `01-foundation`'s standing debt is untouched and is not closed by this.** Nothing in the
  workspace registers `dyn EventBrokerApi` outside this gear's tests; `features/catalog-version.md`
  records the same measurement independently. This entry decides only whether the fixtures wait for
  that, and they do not. It also does not touch the **event-log retention window**, whose value is a
  `PRD.md` §15 open and without which, as §1 says, the replay contract *"is words"*.
- **Not changed**: no fixture is authored here, no dependency edited, and no feature flag added to any
  manifest.
- **Propagated**: `features/consumer-contracts.md` (§1's boundary paragraph, `dod-event-versioning`,
  `dod-dedup-ordering`, `dod-bootstrap`, `dod-seam-suite-home`, §7's arithmetic and row 26 answered),
  `design/12-consumer-contracts.md` (§2.1's joint-fixture rule).

#### P-D-57 — The pin keeps every derived member and carries its comparability; the job is two-sided

- **Date**: 2026-08-31 (owner call)
- **Context**: `features/consumer-contracts.md` §7 row 25 asked which side of the schema pin moves
  first, given that members of the pinned set have no shipped column, and named two arms that *"give
  opposite CI colours for months"*: a job admitting a member as `owed`, or a pin listing only shipped
  members.

  **Half of it was already decided, and the row's own count came from conflating three sets.**
  `dod-catalogsku-shape` states the call: *"the superset lands on `products-sdk`'s read shape as those
  features land their columns, and until then the pin's membership is derived from the design set
  rather than compared against the type."* So the SDK side moves, additively, and the pin never
  shrinks to shipped-only. And the three sets are distinct, as this feature's own §1 says — *"The read
  shape is ten members; the **pin** is a different and smaller set"*:

  | set | size | measured against the crate |
  |---|---|---|
  | the catalog **read shape** | 10 members | 5 have no shipped column |
  | the **pin** (C1, P-D-12) | 8 fields, `skuCode` and `name` deliberately out — *"pick-list display, drift cosmetic"* | **6 of 8** have no shipped operand, counting the metering pair as its two tokens |
  | the SDK's `Sku` type | **7 members** — `sku_id`, `tenant_id`, `product_id`, `sku_code`, `lifecycle_state`, `internal_revision`, `published_version` | pin members present: `skuId`, and `status` under the name `lifecycle_state` |

  So `name`, one of the row's seven, **is not a pin member at all**.

  **And the SDK type calls its own absences deliberate**: *"The capability columns a SKU carries —
  typing, `sellable`, `PlanTier`, the accounting codes, the metering unit — are not here. They belong
  to the features that own their rules, and a consumer reads them from those."*

  **What actually forces the row is that the normative text today is the months-of-red arm.** C1 says
  the *"CI test **fails on divergence**"* and `inst-ss-home` that the job *"**fails on any
  divergence** in the C1 fields"*. Against a pin authored from the register and an SDK that carries
  two of its eight members, that is red from the day the pin lands until `02`, `03` and `06` finish.

- **Decision**: the pin lists **every** derived member and carries each member's **comparability**;
  the CI job is **two-sided**.

  | Call | Propagation |
  |---|---|
  | **The pin keeps its derived membership and gains a per-member comparability flag.** Membership stays P-D-12's rule; the flag says only whether the member is comparable against the SDK surface *yet*. Nothing is removed from the pin and no member is dropped for being unshipped | `design/12` §1 C1, §2.1 `inst-ss-pin`; `dod-schema-pin` |
  | **The job compares the comparable members and asserts the absence of the rest.** So a member that ships while still marked non-comparable **fails the job** — the flag cannot rot into a standing excuse, and the failure lands in the change that shipped the member | `design/12` §2.1 `inst-ss-home`; `dod-seam-suite-home` |
  | **The flag is authored conservatively: `comparable` only once both the column and the SDK member ship.** That makes the job green the day the pin lands and turns each landing into a deliberate pin edit reviewed by both gears, which is the asymmetry `inst-ss-pin` already relies on | `dod-schema-pin`, `dod-catalogsku-shape` |

- **The argument against, stated**: the two-sided check only catches the **late** direction. A flag
  reading `comparable` for a member that never ships is a plain red — exactly the months-long red the
  row wants to avoid — and nothing mechanical prevents it; the protection is conservative authoring,
  not the mechanism. Recorded rather than engineered around, because the alternative is a third state
  that means "expected soon", which is a schedule in a contract artifact.
- **Scope**: this decision does not touch **lint 9's grammar** — the register's `Operand` cell keeps
  P-D-43's one-token-per-member form and its three non-field markers, and the lint keeps reading only
  that cell. It does not decide the job's **home** (still a §15 open), does not reopen which side
  moves, and adds no member to the pin.
- **Not changed**: P-D-12's membership rule, C1's v1 set, the `skuCode`/`name` exclusions, and the
  runtime fail-closed on divergence (the dependent plan publish is rejected pricing-side), which is a
  different mechanism from the CI comparison and is untouched.
- **Propagated**: `design/12-consumer-contracts.md` (§1 C1 and §2.1 `inst-ss-home`),
  `features/consumer-contracts.md` (`dod-schema-pin`, `dod-seam-suite-home`, `dod-catalogsku-shape`,
  §7's arithmetic and row 25 answered).

#### P-D-56 — Two budgets, not one number: the door's acknowledgement and the lane's batching SLO

- **Date**: 2026-08-31 (owner call)
- **Context**: `features/catalog-version.md` §7 row 30 asked whether the increment door's answer time
  is this feature's to publish and whether five seconds is it, having measured that the only bound
  stated anywhere lives in the caller's crate as `DEFAULT_REGISTRY_CALL_TIMEOUT_SECS = 5`.

  **There are two independent fives in the picture and they never meet.** Pricing's is a
  **client-side, per-deployment configurable** await budget: `config.rs`'s
  `registry_call_timeout_secs` defaults to `DEFAULT_REGISTRY_CALL_TIMEOUT_SECS`, rejects `0`, and is
  bounded above by `MAX_REGISTRY_CALL_TIMEOUT_SECS = 60`. So five is one deployment's default, and
  adopting it as a published server promise would pin this gear's contract to a consumer's config
  value. The design's own five — `dod-coalescer`'s *"within ≤ 5 s of the earliest pending"* — is the
  **coalescing window**, which sits behind the acknowledgement rather than inside it.

  **The shipped consumer contract already separates the two objects.**
  `bss_pricing_sdk::CatalogVersionRegistryV1`'s `request_version` returns
  `PendingVersionRef { request_id, pending_ref }` — an acknowledgement, not a version — and
  `committed_version` returns `Option<CatalogVersion>`, `None` until commit, with the doc: *"A pending
  ref that stays unresolved past the batching SLO is an alarm, not an error here — the caller decides
  that, since only it knows how long the ref has been outstanding."* **That sentence presumes a
  published batching SLO**, or *"past the batching SLO"* has no referent.

  **And the caller's budget exists to protect the caller, not to describe us.**
  `infra/registry_deadline.rs`: *"an unanswering peer pins a transaction, its row locks and a pool
  connection on every mutating path at once"*, with ten of twelve awaits inside an open write
  transaction.

- **Decision**: this feature publishes **two** budgets, and neither is the number in the consumer's
  crate.

  | Call | Propagation |
  |---|---|
  | **The acknowledgement budget is a shape, not a copied number.** The door stamps `requested_at` at ingress, claims idempotently per `(tenant_id, source, request_key)`, enqueues and answers. It takes **no lease** and makes **no cross-gear call**, so it fits inside the *smallest* budget a consumer may configure — the config admits `1` and rejects `0` — rather than inside the default of five. The value stays the consumer's to set; what this gear owes is that the door's synchronous path has no unbounded step in it | `design/06` §2 rule 1; `dod-request-door`, `dod-increment-request-port` |
  | **The batching SLO is already published and is C1's**: `requested_at → published_at` **p95 ≤ 60 s, max 5 min**, instrumented by `inst-cv-slo` and alarmed as `catalog_version_overdue`. This decision **mints nothing** — it names those numbers as the referent the shipped consumer's *"batching SLO"* means, so the consumer's alarm and this gear's meter key on one thing | `dod-posting-safe-observability` |
  | **The ≤ 5 s interactive window and the five-minute bulk hard max are inputs to that SLO, not the SLO.** Reading either as the door's answer time is the conflation this entry exists to close | `dod-coalescer` |

- **A defect the round found, and it was load-bearing.**
  `features/catalog-version.md`'s `dod-increment-request-port` said the door *"MUST answer inside that
  budget, and anything it does synchronously — taking the per-tenant lease
  `cpt-cf-bss-products-dod-coalescer` obliges, or resolving a committed version — is inside it"*. The
  lease is **not** the door's: `design/06` §2 rule 2 gives it to *"the **coalescer** (one worker per
  tenant — C3 serialization)"* which *"drains the queue"*, and rule 3 puts the increment transaction
  there too. A door that waited on a per-tenant lease could not fit inside a one-second budget under
  contention, so the sentence and the budget could not both hold. Corrected: **the door enqueues, the
  coalescer leases.** That also settles row 30's own conditional clause about `dod-coalescer`.
- **The argument against, stated**: deriving the door's obligation from the *minimum* configurable
  consumer budget is stricter than any real deployment needs, and it is a bound this gear cannot yet
  measure — no door ships. The weaker alternative was to publish only the qualitative obligation and
  defer any number; it was declined because the qualitative obligation is exactly what arm 1 states,
  and naming the floor it must clear costs nothing while making the claim falsifiable the day the
  door ships.
- **Scope**: this decision does not answer §7 row 29, the cardinality cost of a per-tenant increment
  lease, which stays open with its own owner. It sets no timeout for `committed_version` polling and
  mints no code — a door that cannot answer is the consumer's `unreachable` arm, already distinct
  from `REQUEST_SOURCE_UNKNOWN`'s refusal by **P-D-52**.
- **Not changed**: pricing's constant, its config bounds, and C1's numbers. Nothing is edited in the
  consumer's crate or its register.
- **Propagated**: `design/06-catalog-version.md` (§2 rule 1's acknowledgement clause),
  `features/catalog-version.md` (`dod-request-door`, `dod-increment-request-port` — the corrected
  lease sentence, `dod-coalescer`, `dod-posting-safe-observability`, §7's arithmetic and row 30
  answered).

#### P-D-55 — The disposition rules register in the table's own row order, and the order is unobservable at this commit

- **Date**: 2026-08-31 (owner call)
- **Context**: `features/clone.md` §7 row 13 measured that within a phase rules run in **registration
  order** (`design/01` §3.1, and `design/01-foundation.md`'s own §3 states *"execution order is
  registration order within the phase"*), that `ValidationReport::audit_code` returns
  `self.violations.first().map(|v| v.code)`, and that **no document fixes the order** for the
  disposition set — while **P-D-37** fixed a precedence for the `state` phase's four codes for
  exactly this reason.

  **Collision is the expected case here, not a corner.** `design/11` §3.1 has five
  `Copy + re-validate` rows and says of them *"Every re-validation row below refuses on failure and
  the refusal collects across rows (C4); a clone either lands whole or lands not at all"*. A clone of
  an old `retired` SKU can fail its attribute definition, its `PlanTier` and its accounting code in
  one report. `ValidationRule`'s contract — it *"never short-circuits the run"* and *"never reads
  another rule's verdict"* — is what makes the collection fall out of registration.

  **But the question is a tie-break, not a correctness question, and P-D-37 already settled that
  framing**: the caller's rejection carries every violation the failing phase collected and the audit
  row records one code. What the one code buys is **attribution** — `design/12` §4.1's AC #38 map is
  `AC #38 row → code → declaring slice`, asserted by a lint — so with several classes failing there
  is no single slice to attribute to and no order can be *right*, only stable and recorded.

  **And two of the five rows name no code at all**: *Category assignments* says only
  *"retired category ⇒ re-select"*, and *Metering declaration* says *"fail per AC #38"*. A precedence
  over codes would have to mint two; a precedence over **table rows** does not.

- **Decision**: the disposition rules register in the **row order of `design/11` §3.1's table**, which
  is therefore its execution order and fixes which violation `audit_code()` would answer with.

  | Call | Propagation |
  |---|---|
  | **The table's row order is the registration order.** It is normative, already reviewed, and ordered; using it invents no code and changes no mechanism — `audit_code()` stays `violations.first()`, which `domain/rules_tests.rs:131` already pins as *"whichever runs first wins the audit row"* | `design/11` §3.1's caption; `features/clone.md`'s `dod-disposition-rules` |
  | **The precedence ranks rows, not codes**, so the two rows whose code is unminted take their place when it is minted, and nothing is invented to fill them | `design/11` §3.1 |

- **The order is unobservable at this commit, for two independent reasons, and neither is this
  feature's to change.** Measured in `products/src`: `ValidationReport::audit_code` has **zero
  production callers** (`domain/rules_tests.rs` and `domain/validation_tests.rs` only) — every door
  writes `error_code: domain_err.code()`, and `domain/error.rs:114` maps `Self::Validation(_)` to
  `"VALIDATION"`. **And every registered rule raises that same literal**: `domain/rules.rs:73` is
  `pub const CODE: &'static str = "VALIDATION"`, every `report.violate(…)` call site passes
  `"VALIDATION"`, and `domain/validation_tests.rs:70` asserts `audit_code()` answers `"VALIDATION"`
  for a two-violation report. So even a routed `audit_code()` would not discriminate today.
- **Scope — the observability half is already filed with its owner and this decision does not answer
  it.** `design/01-foundation.md` §6 item 2 asks *"Which code does the audit row store when a phase
  other than `state` collects two?"*, owned by that slice with the error-contract owner, and clone's
  row 13 is a specific instance of it. **No new item is filed here** — a duplicate would leave the
  specific one looking open after the general one closes. The consequence for this feature is
  determinate meanwhile: **a refused clone stores `VALIDATION`, like every other shipped door**, and
  the clone door does not diverge to route `audit_code()` on its own.
- **The argument against, stated**: `design/11` §3.1's row order was authored for readability —
  identity, codes, name, brand, `created_by`, structure, parent, then the re-validating rows — so
  *Display/localized attributes* leads and every multi-class failure that also failed on attributes
  will attribute to `02-taxonomy-attributes`. If attribution should ever prefer the class costliest
  to remedy, this reopens. And the decision **adds** a meaning to that table: its caption spoke to
  collection across rows, not to order.
- **Not changed**: `audit_code()`'s definition, the single `error_code` column, and AC #38's map. No
  code is minted and no door's behaviour changes.
- **Propagated**: `design/11-clone.md` §3.1 (the caption's order clause), `features/clone.md`
  (`dod-disposition-rules`, `dod-clone-audit`, §7's arithmetic and row 13 answered). Extends
  **P-D-37**'s precedence convention to a second rule set without amending it.

#### P-D-54 — The executor the batch machine never named: a gear-owned worker flips edges 1 and 4 inside its own claim

- **Date**: 2026-08-31 (owner call)
- **Context**: `features/bulk-promotion.md` §7 row 26 measured that edges 1 and 4 of the `BulkBatch`
  machine fire on a condition over every row — a stage outcome, a terminal ledger state — and name no
  door, actor or signal. The import door cannot be either: it answers **202**.

  **The design already bought the actor and did not name it.** `design/09` §3.1 `inst-bm-resume`
  states that a batch is resumable — *"a crash mid-commit resumes from the ledger (per-row publishes
  idempotent by row key)"*. Something has to re-enter a batch and re-read its ledger, and a door that
  answered 202 is gone.

  **The gear specifies this executor shape once already, and it was reviewed.** `design/04` §3.1's
  `algo-activation-runner`: due rows *"claimed atomically (state CAS `pending|deferred → running`
  with `claimed_at`"*, a `running` row past its **lease** reclaimed *"`running → pending` with
  `attempt += 1`"*, outcomes *"`applied|failed|deferred`"*, the runner *"its own raising door"*, and
  gauges for *"due-but-unclaimed and deferred counts"*.

  **The donor's mechanism does not transfer, though its conclusion is written down.**
  `gears/bss/pricing`'s `infra/bulk.rs` runs a bulk batch **inline in the caller's request** —
  *"Every row is its own transaction, and that is the whole shape"*, and the *"repository methods open
  their own transactions and that is why they are used rather than their runner-taking forms"*. Its
  module doc then records the price: *"`pricing_bulk_row_lock` has no sweeper, D-37's lease takeover
  is unbuilt"*, so a panic or a dropped future leaves the run in `committing` holding every row's
  lock — *"That run stays `committing`, which is where the remedy is"*. Pricing answers its caller
  when the work is done; **this door answers before the work starts**, and `inst-bm-resume` promises
  recovery, so neither half of the donor's posture is available here.

  **The platform ships the machinery, which is what makes this a naming decision rather than a new
  mechanism.** `toolkit_db::outbox::taskward` is framework-level and outbox-agnostic — its
  `PacingConfig` says so in as many words, *"Framework-level — no outbox-specific knowledge"* — and
  carries `WorkerBuilder`/`WorkerAction`/`Directive`, `PanicPolicy`, `WorkerListener` for
  observability, `ConcurrencyLimit::{Fixed,Tiered}` with `BackoffConfig`, and a caller-supplied wake
  source: *"Wake-up sources (notifiers, pokers) are the caller's responsibility via
  `WorkerBuilder::notifier()`"*, so a door can start the work without waiting a poll interval. It has
  four production consumers — `processor`, `sequencer`, `reconciler`, `vacuum` — **all inside the
  outbox and none in a gear**. And a gear may own such a task: `RunnableCapability::start(cancel)`
  (*"Start the gear's background task"*) with two-phase graceful shutdown, implemented by
  `gears/file-storage/file-storage/src/gear.rs:280`.

- **Decision**: edges 1 and 4 are flipped by a **gear-owned batch worker** that claims a batch the way
  `inst-ar-claim` claims a transition. The flip is a **CAS on the batch state inside the same
  transaction that finishes the last row**, so there is no separate detection pass to lag or race.

  | Call | Propagation |
  |---|---|
  | **Edge 1's executor is the claim transaction that stages the last row.** The `ChangeReport` is generated and submitted to the governance gate in that same transaction, so the report exists exactly when the ledger says staging is done | `features/bulk-promotion.md` §4 `inst-bb-edge-report`, `dod-stage-phase` |
  | **Edge 4's executor is the same worker at the other end** — the claim that lands the last row's terminal state — and `CatalogBulkOperationCompleted` is emitted **inside that CAS**. The winner emits; a re-claim after a lease expiry finds the state already flipped and emits nothing. That is where *"exactly one"* comes from | §4 `inst-bb-edge-complete`, `dod-coalesced-event` |
  | **Crash recovery is the claim's lease, not a sweeper.** A worker lost between the last row and the flip leaves a batch whose rows are all terminal and whose state is not; the lease reclaims it and the CAS makes the flip idempotent | `design/09` §3.1 `inst-bm-resume` (owed) |
  | **`inst-bm-limits`' per-tenant concurrent-batch ceiling is enforced at claim, not only at admission**, because a ceiling checked only by the door drifts as batches hang | `dod-stage-phase` |

- **The normative text names no framework, and that is deliberate.** `design/04`'s runner names none
  either. The platform measurement above is recorded as **evidence that a gear-owned worker with a
  claim, a lease and a wake source is available rather than aspirational** — not as a pin on
  `taskward`. **The argument against, stated**: products would be the first gear to run that
  framework, so the gear-side wiring is unproven and a build may find the abstraction cost real; the
  measurement is in this register so that finding arrives as a build note rather than a
  re-litigation of the executor.
- **Scope — this decision does NOT answer what performs edge 3.** `approved → committing` is §7 row
  **7**'s, carried from `design/09` §6 and owned by this slice with `05`. It has two live candidates
  — this worker, or `05`'s decide door flipping the state in the same transaction as the quorum
  verdict, which is also where the one-shot consumption would be enforced — and the carried row
  records that `05`'s decide door is itself unowned, so nothing here narrows it. Rows 5 and 6 are
  equally untouched: the missing rejection edge, the absent abandon state, the unstated `failed`
  entry edge, and the tenant slot a never-approved batch holds.
- **Not changed**: `products/src` carries none of this. `ActivationRunner`, `claimed_at`,
  `scheduled_transition`, `BulkBatch` and `bulk_batch` are **zero occurrences** across the crate, so
  nothing shipped constrains or contradicts the call.
- **Propagated**: `features/bulk-promotion.md` (§4 `inst-bb-edge-report`, `inst-bb-edge-complete` and
  the executor paragraph; `dod-stage-phase`, `dod-batch-state-machine`, `dod-coalesced-event`; §7's
  arithmetic and row 26 answered), `DECOMPOSITION.md` §2.9 (`BatchWorker`). **Owed and not edited
  here**: `design/09` §3.1's `inst-bm-resume`, which should name the claim and the lease — that is
  `design/09`'s edit.

#### P-D-53 — The increment transaction runs at the engine default, because the guard is what closes the race

- **Date**: 2026-08-31 (owner call)
- **Context**: `features/catalog-version.md` §7 row 37 measured that **no isolation level is stated
  anywhere in the design set or the crate**, while three levels give three different behaviours for
  the same recorded design. `inst-sn-collect` collects the snapshot *"inside the serialized
  transaction"* and `inst-sn-revalidate` re-reads the heads *"before commit"* in that same
  transaction, and §6 requires the detected race to surface as a **refusal**,
  `STAGED_ENTITY_CHANGED`.

  **The design already carries the mechanism, which is what makes the level a consequence rather
  than a choice.** `inst-sn-revalidate` records each collected entity's
  `(id, published_version, lifecycle_state)` and compares at re-read — a **row-version guard**. Of
  the three levels only one lets it work:

  | level | what happens to the guard |
  |---|---|
  | snapshot-isolating (`REPEATABLE READ`, SI) | the re-read returns the **collect-time** snapshot, so the guard **cannot fire** and a version publishes content the design says must be refused |
  | `SERIALIZABLE` | the transaction **aborts** with a serialization failure instead of raising the code, so §6's required refusal never reaches the caller |
  | **engine default — `READ COMMITTED` on Postgres** | every statement takes a fresh snapshot, the re-read sees the concurrent change, the guard fires and the door refuses `STAGED_ENTITY_CHANGED` |

  **The donor drew this distinction first, and it is cited for the distinction only.**
  `gears/bss/pricing`'s publish path opens the engine default and states the reason in a contract
  paragraph, separating two invariants an earlier revision had conflated — *"the conflation is what
  hid a live defect"*. Its counter invariants *"need no SSI"* because unique keys make a fork
  unrepresentable *"at any isolation level"*; its predicate invariant is *"a different thing, and no
  key covers it"*, and the conclusion is the sentence that transfers: **"It is closed by the
  row-version guard, not by isolation."**

  **What does not transfer is the donor's cost argument against `SERIALIZABLE`** — *"it would hold
  predicate locks across the registry round-trip"*. This increment holds **no** cross-gear call:
  `inst-sn-collect` reads `products_entity_version` and the heads, both local. So `SERIALIZABLE` is
  declined here for the refusal-versus-abort reason above, not for the donor's, and the borrowed
  reason is named as not applying so a later reader does not inherit it.

  **And it is a judgement rather than an impossibility.** `libs/toolkit-db`'s
  `Db::transaction_ref_mapped_with_config` takes a transaction config, so raising the level is
  available on the platform and is being declined deliberately.

- **Decision**: the increment transaction opens at the **engine default**, `READ COMMITTED` on
  Postgres, and the stage-vs-commit race is closed by `inst-sn-revalidate`'s **row-version guard**,
  never by isolation.

  | Call | Propagation |
  |---|---|
  | **The level is the engine default and is stated, not assumed.** The word *"serialized"* in `inst-sn-collect` describes the coalescer's **one-worker-per-tenant** serialization, not a database isolation level, and is not to be read as `SERIALIZABLE` | `design/06` §2; `features/catalog-version.md`'s `dod-snapshot-builder` |
  | **The guard is the correctness mechanism and its comparison is normative**: the collected `(id, published_version, lifecycle_state)` triple, re-read before commit, refusing `STAGED_ENTITY_CHANGED` on any difference. A build that relies on the snapshot instead has no detector | `features/catalog-version.md`'s `dod-stage-commit-revalidation` |
  | **`SERIALIZABLE` is refused for a stated reason**: it converts the design's refusal into an abort, and §6 requires the code | `design/06` §2 |

- **Scope — this decision does NOT set a gear-wide isolation posture, and the radius sweep found the
  one other site.** `features/sku-classification.md` §7 records *"The removal-vs-publish race is
  unguarded… No isolation level, no lock and no re-check-inside-the-transaction clause is stated"*,
  and that race is **not** of this shape: a publish adds the **first** reference between the holder
  scan and the state flip, so there is no row to version — it is precisely the donor's *"predicate
  invariant… no key covers it"*. It needs its own answer, and `02-taxonomy-attributes` registers the
  analogous class as its own item. This entry settles the increment door and nothing else.
- **Not changed**: `products/src` sets no isolation level anywhere and continues to take the engine
  default everywhere; no transaction config is introduced by this decision.
- **Propagated**: `features/catalog-version.md` (`dod-snapshot-builder`,
  `dod-stage-commit-revalidation`, §7 row 37 answered). **Owed and not edited here**:
  `design/06-catalog-version.md` §2's `inst-sn-collect` and `inst-sn-revalidate`, which should carry
  the level and the guard's normative comparison — that is `design/06`'s edit.

#### P-D-52 — The increment-request door gains a refusal code, and the counterparty's discriminator fixes its shape

- **Date**: 2026-08-31 (owner call)
- **Context**: `features/catalog-version.md` §7 row 22 measured a live asymmetry. The shipped
  `pricing-sdk` port `CatalogVersionRegistryError` carries a fourth arm, **`Rejected(String)`**,
  discriminated by the wire constant `CATALOG_VERSION_REJECTED`, and argues for its own existence:
  *"a refusal is a decision and will be made identically for as long as the request is unchanged; an
  outage is a deployment state a retry may find changed."* But **none of this feature's six codes is
  a refusal of an increment request.** `inst-cv-request` fixes the trigger set at exactly three —
  registered downstream addressability requests, this gear's own slice-09 bulk commits as a
  registered internal requester, and the operator catalog-publish act — and §3.2 declares no code for
  a request from a source outside it. So either the door owed a code or the port's arm was
  unreachable against this registry.

  **The refusal is not authorization-shaped, which is what makes the ladder position forced rather
  than chosen.** The door already gates on `catalog_version × request`, so a caller without the grant
  is refused by authz. What was missing is the refusal for a caller that *holds* the grant and whose
  `source` is not a registered requester — a precondition on the request's content, decided by the
  registry, identical for as long as the request is unchanged.

  **And the counterparty's discriminator fixes the wire shape, measured in its source.** The port
  reaches `Rejected` only on `CanonicalError::FailedPrecondition` **and** a precondition violation
  whose `type_` is `CATALOG_VERSION_REJECTED`, and it says why it matches on both: *"`FailedPrecondition`
  is a shape the registry could raise for something other than a refusal, and folding those onto
  `Rejected` would hand the gear a 400 for a fact it never decided."* It also takes its sentence from
  the **violation**, not the envelope detail. A 403 — the position `PARTICIPANT_UNKNOWN` holds for an
  analogous roster miss — would arrive as a different category and land on the port's `Other` arm,
  leaving the arm as unreachable as before.

- **Decision**: **`REQUEST_SOURCE_UNKNOWN` is minted**, declared by `06-catalog-version` in §3.2 and
  raised by `inst-cv-request` alone, when a request's `source` is outside the trigger set that
  instruction fixes.

  | Call | Propagation |
  |---|---|
  | **The code is `REQUEST_SOURCE_UNKNOWN`**, following the set's `*_UNKNOWN` idiom for a roster miss (`CATALOG_VERSION_UNKNOWN`, `PARTICIPANT_UNKNOWN`) | `design/06` §3.2 |
  | **Its class is `FailedPrecondition` — a 422 architecturally, reaching the wire as a 400 carrying its code** — and the refusal **MUST** carry a precondition violation of type `CATALOG_VERSION_REJECTED` with the registry's own sentence as the violation description. This is the first code in this gear whose wire shape is set by a consumer's discriminator rather than by the gear's own ladder, and it is recorded as such so a later status sweep does not "correct" it to 403 | `design/06` §3.2's problem-response block; `features/catalog-version.md`'s `dod-request-door` and `dod-cv-error-taxonomy` |
  | **It is NOT authorization-shaped and MUST NOT be 403**: the grant check has already passed when it is raised | `design/06` §3.2 |
  | **The code count moves from six to seven** wherever this feature states it — including §6's *"six codes, six lines"* positive-control criterion, which becomes seven | `features/catalog-version.md` §6, §5, §7 |
  | **It does not join AC #38's map.** That map's rows are the PRD's fifteen enumerated failure cases and this is not one of them; `design/12` §4.1 is unchanged | recorded, no edit |

- **Not changed**: the trigger set stays exactly three; the door's grant stays
  `catalog_version × request`; the composition clear still raises no code by design.
- **Propagated**: `design/06-catalog-version.md` (§3.2), `features/catalog-version.md` (§5's
  `dod-request-door` and `dod-cv-error-taxonomy`, §6's positive-control block, §7 row 22 struck).
  **Owed and not edited here**: `design/12-consumer-contracts.md`'s `inst-sdk-surface`, whose SDK
  error enum is built *"from every slice's registered codes"* and now has a seventh from this slice —
  that is 12's edit, not this one's.

#### P-D-51 — Where an envelope obligation lands when the transport has no slot, and the two subject types §6 asked for

- **Date**: 2026-08-30 (owner call — raised by the three-lens review of the broker producer)
- **Context**: P-D-47 put publishing on the broker SDK, and building it made two of the set's own
  statements unbuildable as written. Both were found by an independent reviewer, not by the author,
  and both had been shipped without being registered.
- **Decision**, two arms:
  1. **An envelope obligation binds to the envelope where the transport has a slot for it and to
     the payload where it does not**, and each obligation's landing place is now stated rather than
     implied. **P-D-01's own word is the authority**: it calls the five obligations
     *"envelope-agnostic"*, and §4.4 and `dod-outbox-eventing` are its restatements — so where the
     restatement says "envelope" and the transport has no field, the restatement moves, not the
     decision. Measured against `event-broker-sdk`'s `models::Event`, which carries `id`,
     `type_id`, `topic`, `tenant_id`, `source`, `subject`, `subject_type`, `partition_key`,
     `occurred_at`, `trace_parent` and `data`:

     | Obligation | Lands | Why |
     |---|---|---|
     | versioned schema reference | envelope, as `type_id` | the SDK's `TypedEvent::TYPE_ID` is that id |
     | correlation | envelope, as `trace_parent` | a slot exists, and the value is the W3C `traceparent` |
     | **causation** | **payload** | `Event` has no causation field |
     | per-aggregate ordering key | envelope, as the broker's partition selection | P-D-47: the gear sets no `partition_key`, so ADR-0002's default applies |
     | **pseudonymous actor** | **payload** | `Event` has no actor field |
     | `vN`→`vN+1` compatibility | neither — a discipline over schema versions | §4.5 defers it to slice 12 |

     The idempotency key stays the event `id`, which the SDK mints (P-D-47); the gear's interim
     envelope carries an id of its own that reaches no consumer.
  2. **`subject_type` is `gts.cf.core.events.subject.v1~cf.bss.products.product.v1` for a Product
     and `…sku.v1` for a SKU**, closing `design/01-foundation.md` §6 item 12. The **namespace** is
     the platform's — every other subject type in this workspace is a
     `gts.cf.core.events.subject.v1~` id — and the **name** is this set's own declared domain type
     (`DESIGN.md`: `gts.cf.bss.products.product.v1~`, `…sku.v1~`), so the broker-side id and the
     domain type are traceable to each other by inspection.
- **Measured, not argued**:
  - Arm 1 at the platform: `event-broker-sdk/src/models.rs`'s `Event` has neither field, and
    `producer/outbox.rs`'s `ProducerOutboxEnvelope` — which the SDK owns end to end — has neither
    either. There is no third place to put them short of amending a shared platform type.
  - Arm 2 at the platform: subject types in use are all
    `gts.cf.core.events.subject.v1~<name>`, and the mock's `assert_gts` checks only the `gts.`
    prefix and a `~`, so nothing but the registration itself constrains the name. The value is
    validated at ingest against the `allowed_subject_types` list registered **with the event
    type**, which is why it is one half of an agreement rather than a fact.
- **The costs, stated**:
  - Arm 1 amends a DoD to match what was built, which is the move that makes a DoD stop being a
    contract. The safeguard taken is that the amendment **enumerates where each obligation lands
    and why**, so the clause is more checkable after the change than before, not less. What is
    given up is the single-sentence form.
  - Arm 2 answers a question §6 assigned to *three* owners — this slice, slice 12 and the PRD
    owner. The two ids are broker-side resources, so whoever administers the broker may hold a
    naming convention neither party has seen; the answer is recorded as a derivation with its
    reasoning so it can be overridden by measurement rather than re-derived.
- **Propagated**: `design/01-foundation.md` (§4.4's Payloads bullet, §6 item 12);
  `features/foundation.md` (`dod-outbox-eventing`'s envelope clause).
- **Owed**: the event-type registrations at the broker under arm 2's ids and the eight
  `event_type.v1~` ids `infra::broker` derives — one half of an agreement whose other half this
  gear does not own.

#### P-D-50 — Seven taken ahead of the build: two columns, a minted code, a grant deliberately not minted, and three cells that denied a route the set declares

- **Date**: 2026-08-29 (owner call — the pre-implementation round)
- **Context**: the review programme was stopped by the owner on a measurement rather than a
  feeling. Across the set's twenty-four documented commits a lens pass adds **~13** open items and
  an owner round retires **~3**, so "no open items" is not a reachable exit; and the set has
  carried **zero** "cannot be built" statements in any slice since **P-D-47**, so the reachable
  exit — a buildable set — was passed ten commits earlier. What was asked for instead was one round
  over the questions that are cheap to answer in prose and expensive to discover in code: schema,
  authorization surface, error contract, cross-gear obligation, state machine. The lint layer, the
  register's own hygiene and every wording question were **deliberately excluded from the
  selection** — the in-repo gate was retired knowingly in `21a149fda`, so a defect in a lint
  grammar costs nothing today, while a missing column costs a migration.
- **Decision**, seven arms:
  1. **DSAR erasure is per-tenant in v1, and no platform-plane grant is minted.** A DSAR erasure
     enumerates and tombstones the principal's rows **in the requesting tenant only**; a principal
     appearing in several tenants needs one request per tenant. The alternative — a platform-plane
     `compliance × erase` grant — would create a write path outside tenant elevation, which
     `constraint-tenant-isolation` and 05 C5 (`any write under elevation is refused, full stop`)
     both forbid, and the gear will not build one on an assumption about what Legal requires.
     **The contingency is recorded rather than hidden**: should Legal rule per-tenant erasure
     incomplete, the platform grant becomes mandatory and is a post-v1 change, not a gap in the
     rule. This is the one arm whose recommendation was the engineering-cheapest and not
     necessarily the legally safest, and it was taken knowing that.
  2. **`products_category` gains `mutation_seq`, and `STALE_CATEGORY_TOKEN` is minted.**
     `inst-av-category-branch` put the live-value door behind an `If-Match` on a "category
     row-version token" that no column provided, and no code was declared for the mismatch. The
     row now carries a `mutation_seq` and the door refuses a mismatch `STALE_CATEGORY_TOKEN` (409),
     this slice's own — `STALE_REVISION` is 01's entity-head code and `STALE_LIVE_OP` the
     `GovernedLiveOp` envelope's, and neither is this door's precondition. **C2 is amended** so the
     counter is not read as a revision: categories still have no revisions and no versions, and
     nothing freezes, snapshots or treats `mutation_seq` as version content.
  3. **The satisfying version gets a column, and `coalesced-into(version)` becomes `coalesced`.**
     `products_catalog_version_request` gains `satisfied_by_version_id`. A state value cannot
     carry a parameter no column holds: after commit there was no queryable link from a version to
     the requests it satisfied, so a replayed `CatalogVersionPublished` could not have its
     `satisfiedRequests` rebuilt and pricing's stuck pending refs could not be reconciled.
  4. **The content-PII write block is wired at five doors, and 09 leaves the enumeration.** 05's
     `inst-gv-reject` and `inst-bg-open`, and 07's `inst-cr-door`, `inst-bc-ceremony` and
     `inst-pr-retirement`, now pass their free-text reason through 02's `inst-av-pii-block` before
     the row is written, a hit failing `CONTENT_PII_BLOCKED` — the form 01 already used at its
     audit-row door. Both slices list the code in their response map as declared elsewhere.
     **09 is struck from `inst-av-pii-reason`'s enumeration**: it has no free-text `reason` door of
     its own — its batch reason lives on 05's `ApprovalRecord`, its mass-retire reason on 04's
     `inst-rt-initiate`, both already enumerated, and its only other stored reason is the literal
     `batch-abandoned` constant. The four-slice class was a three-slice class.
  5. **The metadata PATCH is a per-key merge, a `null` value removes a key, and a write to a
     terminal entity is refused `ENTITY_TERMINAL`.** `inst-md-write` capped key count without
     stating remove semantics, so a map standing at the cap had no exit. The refusal code stays
     01's and is raised here: **P-D-06** puts the map outside the head's *version content*, which
     governs what a snapshot freezes and not what the terminal guard refuses, and **P-D-32**
     already widened `ENTITY_TERMINAL` to any head write on a `retired`/`discarded` row.
  6. **A `BucketRegistry` lookup miss is fail-closed, and §5's agreement test gains a third
     assertion.** The registry is a compile-time map, so a miss is a real runtime case: a
     published-state column carrying no tag means it was added without registering one, and the
     head door refuses the write under the pipeline's own posture rather than routing to a default
     bucket. The agreement test compared only columns *both* artifacts name; it now also asserts
     that no published-state column is named by **neither**, which is the exact column the door's
     miss would refuse.
  7. **A `Doors` cell is per action, and the three contradicted cells take their routes.** Where a
     row holds several actions and a declared route spends one, the cell names the route and the
     action, and says which actions still have none — a bare route in a multi-action row would
     otherwise read as if the whole row were doored. `approval × read` takes 05's own pending-queue
     door, `category × read` takes 08's browse door, `catalog_version × read` takes 09's export
     door.
- **Measured, not argued**, arm by arm where a measurement decided it:
  - Arm 2 at the donor: `pricing_price_window` hit exactly this problem and answered it with a
    column — "D-191's `If-Match` needs something to compare an entity tag against", in
    `gears/bss/pricing/pricing/src/infra/storage/migrations/m20260821_000039_create_pricing_price_window.rs`
    — while `pricing_price` already carried `row_version`. The donor's counter counts **acts, not row
    writes**, and its migration records why that is load-bearing: an approval subject is built from
    an act identity, and a retry after a refusal must render the same subject or the approval loop
    has no exit. The category door spends a `GovernedLiveOp`, so the same hazard is live here and
    the column inherits the act semantics.
  - Arm 3 inside this set: 06 §4 already spells the `FreezeLedger`'s `not_frozen(forced_at,
    ceremony_ref)` out as columns. One parameterized state in the slice had columns and the other
    did not; the precedent chose the arm.
  - Arm 4 at the donor, which argued **against** the rule and lost on a stated reason: pricing has
    no content-PII write block at all. All seventeen `pii` occurrences in its source are
    field-level — pricing's audit-PII rule (its **D-61**: the audit log stores a pseudonymous
    principal id, never a display name or an email; the donor's instruction id is not cited here,
    **P-D-43** having struck those from this set). `CONTENT_PII_BLOCKED` on free text is this set's invention. It is
    kept because slice 10 carries a DSAR erasure obligation pricing does not, and 02's stated
    consequence for an unwired door is that personal data typed into it is unreachable by erasure
    forever. Half-wired was the worst available state: it read as enforced and was not.
  - Arm 7 by census: the set declares seventeen routes as code spans; the `Doors` column held
    fourteen, and the three outside it named exactly the three grants whose cells read "no route
    declared".
- **Consequence, recorded rather than hidden**: **lint 3 is now green** — all seventeen declared
  routes appear in the `Doors` column. That is a property of the artifact, not of a gate: no job
  runs the lints, and 12 §6 still records that lint 3's population exists in two spellings, which
  this arm does not fix.
- **Not decided here**: the two duplicate open items the sweep left for an ownership call — the
  `commit → durable-acceptance` meter filed identically in 06 and 08 and declared by neither, and
  whether 02 owns the free-text class it enumerates. Both are cheap in code and were excluded on
  that ground.
- **Propagated**: `design/01-foundation.md` (§1.7's `BucketRegistry` row, §5's agreement test, §6 —
  item 6 struck and the list renumbered to twelve); `design/02-taxonomy-attributes.md` (C2, §3.3's
  two code lists, `inst-av-category-branch`, `inst-tc-etag`, `inst-md-write`, `inst-md-placement`,
  `inst-av-pii-reason`, §4.1's `products_category`, §6 — two items struck);
  `design/05-governance.md` (§3.2's column convention and three cells, §4's code declaration and
  response map, `inst-gv-reject`, `inst-bg-open`, §6 — the PII item struck and the grant-gap item
  re-measured); `design/06-catalog-version.md` (§4's request table, §6 — item struck);
  `design/07-reference-signal.md` (`inst-cr-door`, `inst-bc-ceremony`, `inst-pr-retirement`, the
  response map, §6 — item struck); `design/09-bulk-promotion.md` (§6 — item struck, the slice
  owing nothing); `design/10-retention-erasure.md` (`inst-er-export`'s L5 clause, `inst-er-erase`,
  §6 — item struck); `PRD.md` (§15's cross-tenant DSAR row, struck and answered).
- **Owed**: nothing in this set. Arm 1's contingency sits with Legal and is not a design debt; arm
  2's column and arm 3's column are implementation work, which is what the round exists to unblock.

#### P-D-49 — Six live contradictions: the takeover race, the vacuous GC gate, one clone vocabulary, a clearable successor, a principal column, and an entity-kind column

- **Date**: 2026-08-29 (owner call — the contradiction round)
- **Context**: the 211 open items were measured by what would settle each. 74 are risks with no
  question and ~125 need an owner call; the six below are the subset where the set **currently says
  two things that cannot both be built**, so answering them repairs a document rather than filling a
  gap. Every premise was opened at its source before the round, and one of mine did not survive that
  check — see the correction under arm 1.
- **Decision**, six arms:
  1. **The expired-key takeover is a compare-and-swap**, and `IDEMPOTENCY_KEY_IN_FLIGHT` has two
     documented paths. Nothing holds an expired row between a transaction's conflict check and its
     takeover UPDATE, so two duplicates on one expired key both clear the check, both read the same
     expired row, and — without a predicate on the row's own claim stamp — **both execute the
     guarded mutation under one key**. The UPDATE now carries that predicate; the loser is refused
     in-flight and executes nothing. The fresh-claim path stays unreachable and is recorded as such:
     reaching it means the one-transaction contract was violated, and refusing is how that becomes
     visible. **P-D-42's transaction contract and P-D-38's posture are untouched.**
  2. **Slice 10's `RetentionGate` ranges over the version's `participant_set_snapshot`**, not over
     the ledger rows that happen to exist. A snapshot member with no registration holds the version;
     an empty snapshot — nobody ever owed an ack — is collectable. The universal quantification let
     an empty ledger satisfy the gate vacuously and collect a version nobody had frozen, against C4.
  3. **The clone has one outcome vocabulary: it refuses, and the refusal collects.** "Forces
     re-selection" is the operator's next act on that answer, not a second wire outcome — the only
     reading under which §5's one fixture yields three named failures.
  4. **`replaced_by_sku_id` is write-once per retirement, not per row**: the governed cancel of a
     retirement's `ScheduledTransition` clears it in the same statement. Without that arm a
     cancelled, un-deprecated SKU stayed `published` naming a successor no admitted write could clear.
  5. **The identity map gains `principal_ref`** (pseudonymous, NOT NULL, indexed), because three
     rules read the map by principal and the key admitted no such read. A tombstone destroys the
     payload and leaves the pseudonym, which is what the slice already means by "pseudonym retained".
  6. **The clone disposition matrix gains an `Applies to` column.** One table served both entity
     kinds while the rename rule it delegates to is Product-only and `products_sku` carries no name
     column at all, so its "Canonical name" row was unbuildable for half its subjects. Every value
     in the new column is a fact 01 §4.1/§4.2 already states.
- **A premise of mine that did not survive its own check, recorded because the round was put to the
  owner on it.** Arm 1 was first brought as "restore the claim's own transaction, because the donor
  is built the other way". Opening `gears/bss/pricing/pricing/src/infra/storage/repo/idempotency_repo.rs`
  showed the opposite: the donor holds claim and answer in **one** transaction exactly as this gear
  does after P-D-42, and its own module doc says the fresh-claim refusal is *"Unreachable under the
  one-transaction contract"*. What keeps the code live there is the takeover race — *"Reachable in
  production, with no contract violation by anyone"* — and *"no tightening of the transaction
  contract closes it"*. So the recommendation was withdrawn and re-put; the arm that landed is
  cheaper, reverses nothing, and closes a **double-execution** defect the first framing would have
  left standing. Both quotations byte-verified.
- **The costs, stated**:
  - Arm 2: a tenant with no registered participant has an empty snapshot, so its versions are
    collectable with no ack at all — correct by the rule above, and worth knowing before the first
    participant registers.
  - Arm 5: the principal↔ref linkage survives erasure by construction. That is what makes a repeat
    DSAR answerable, and it is a posture Legal may wish to rule on — the `PRD` §15 rows on the
    allow-list and the cross-tenant DSAR reach are the place.
  - Arm 4: one more admitted write in the append-only whitelist, on a column whose whole point was
    that it never changed.
- **Propagated**: `design/01-foundation.md` (§3.2's expiry and in-flight rows, §4.2's whitelist, §6);
  `design/04-lifecycle.md` (§6); `design/06-catalog-version.md` (§6, and — added 2026-08-29 —
  `inst-fz-liveness`'s liveness formula); `design/10-retention-erasure.md` (`inst-rt-gc`, §4's
  identity map, §6); `design/11-clone.md` (C4, §3.1, §6); and — added 2026-08-29, the arm-2 domain
  correction having reached only `inst-rt-gc` until then — `DECISIONS.md` **P-D-18** (the entry
  that defines version liveness) and `PRD.md` (`fr-grandfathered-retention-coupling`, §9.2's
  protocol line, AC #44's `And` clause, §15's closed liveness-source row).
- **Owed**: nothing.

#### P-D-48 — The six flagged decisions, put to the owner: two amended, one completed, three confirmed as recorded

- **Date**: 2026-08-28 (owner call — the flagged-decision round)
- **Context**: P-D-14…P-D-20 were registered FLAGGED by the branch review on 2026-08-26 and never
  put to the owner; P-D-47 had confirmed P-D-19 as amended after measuring its premise against the
  PRD's pre-decision text. The other six were measured the same way — every claim that the PRD, the donor or the
  platform already said something was opened at its source, with the PRD read at `eb68b8515` — and put to the owner in one round. All six recommendations were taken as put.
- **Decision**, six calls:
  1. **P-D-14 confirmed as amended: on a dirty head the composition clear is deferred, never
     refused.** The owning slice's reading (06 `inst-cc-clear`) wins over the entry's *refused*:
     the caller is an inbound signal, not a request, so there is nobody to answer a refusal to; 06
     §3.2 raises no code for it by design; a deferral cannot wedge a publish queue. The signal is
     durable and idempotent, the flag stays set, `composition_clear_held` names the head, and the
     clear re-evaluates when the head next goes clean. 05, the PRD and AC #26 stop being neutral.
  2. **P-D-15 confirmed as recorded**: every §9.2 inbound machine contract is a `products-sdk`
     client resolved from `ClientHub`.
  3. **P-D-16 confirmed, and its open half closed: the unresolvable-target arm carries no flag of
     its own.** Its admission predicate is a resolver fact (not-found), not operator discretion; the
     arm already increments the break-glass `TripwireCounter`; a default-OFF flag would reinstate
     the wedge the arm exists to exit.
  4. **P-D-17 confirmed as recorded**: a same-identity promotion row with different content is
     update-as-draft.
  5. **P-D-18 confirmed, with the v1 registered freeze-participant set = {plan-price (pricing
     gear)}** — the P-D-03 pattern for the sibling signal: the ack and release clients are built
     jointly with this gear, and Contracts and Billing register at their own build time. No v1 duty
     is booked on a gear that does not exist; the registry-side half of `PRD` §15's row on the
     silent ack counterparts closes, and 12 §6's question whether the obligations are booked on
     gears that exist closes with it. Whether pricing's design accepts the ack and the release is
     the cross-gear half and stays open.
  6. **P-D-20 confirmed, and completed with the door it lacked**: the lead-window re-announcement
     of `SkuRetired`/`ProductRetired` is enqueued by 01's publish door in the publish's own
     transaction — a new row, `inst-fd-publish-reannounce`, beside `inst-fd-publish-emit`. The
     event, its payload and the retirement identity are 04's (`inst-rt-initiate`); the enqueue is
     the door's. 04 §6 had recorded that the re-emitter had no door.
- **Measured, not argued**:
  - Call 1: 06 §3.2 raises no error code for the clear because its caller is an inbound signal,
    not a request; 04's flip guard defers the same way; the producer's side of the signal is
    unregistered (`PRD` §15), so a refusal code would be a wire fact for a contract pricing has not
    adopted.
  - Call 2: `docs/ARCHITECTURE_MANIFEST.md` — *"in-process gears register local adapters in
    `ClientHub`"*; `docs/arch/toolkit-contract-binding/DESIGN.md` allows a remote-capable contract
    to be satisfied locally; and pricing already takes a `ProductCatalogClientV1` from the
    `ClientHub` (`gears/bss/pricing/pricing/src/module.rs`), consuming this gear in-process in the
    other direction.
  - Call 3: the amended FR says *MAY* under the same ceremony and names no flag; the donor has no
    break-glass lane at all, so there is no precedent either way.
  - Call 4: the parity citation (Stripe test/live, Zuora Deployment Manager) predates the decision
    — it is in the PRD at `eb68b8515`; the donor's bulk import edits an existing draft under its
    version and conflicts only on a concurrent edit (`BULK_ROW_CONFLICT`), so update is the donor's
    shape and conflict is reserved for a version mismatch.
  - Call 5: the PRD named three participants (the `freezeComplete` glossary row and §9.2's
    `Direction` line); Billing has no gear, Contracts' PRD never cites `CatalogVersion`, and
    pricing's design set contains no mention of producing an ack or a release. P-D-03 had already
    narrowed the sibling producer set to {plan-price} on the same facts.
  - Call 6: the pre-decision PRD named only adoption-block and browsable as initiation effects, so
    P-D-20's premise holds; 04 §6 recorded the missing door; 01's publish door already carries
    lane rows under the act unit (P-D-34). The donor is silent — pricing retires without a lead
    window, and D-146's *terminal for revisioning* is post-flip.
- **The costs, stated**:
  - Call 1: a deferred clear can wait indefinitely on a head that never goes clean; the alert is the
    only signal — and a refusal would not have cleaned the head either.
  - Call 5: with pricing silent, every version stays posting-unsafe until its ack lands — already
    the set's stated v1 posture, now on one participant instead of three. The §15 row's owner is
    Architecture with the participants; this is a product call on the registry's own governed set,
    taken as P-D-03 was.
  - Call 6: an event declared by 04 is enqueued by a 01 door. The alternative — 04 reacting to
    `SkuPublished` after commit — is a second transaction, at-least-once, with no ordering
    guarantee against the publish event it answers.
- **Propagated**: `design/01-foundation.md` (§1.4, the publish door's `inst-fd-publish-reannounce`
  row, §4.5, §6); `design/04-lifecycle.md` (`inst-rt-initiate`, §4 events, §5);
  `design/05-governance.md` (`inst-gv-one-shot`); `design/06-catalog-version.md` (`inst-cc-clear`,
  `inst-fz-timeout`, `inst-fz-liveness`, §6); `design/12-consumer-contracts.md` (`ObligationRegister`,
  §6); `PRD.md` (the branch-review note, `fr-materiality-gated-publish`, the `freezeComplete` glossary
  row, §9.2's freeze-ack and composition-signal blocks, AC #26, the §15 row); `DESIGN.md` (the
  status line, the cross-gear bullet, the flags paragraph).
- **Owed**: nothing. No decision in this register is flagged.

#### P-D-47 — The last four build-blockers: a tombstone state, a withdrawn opt-in, two codes, and the broker's own producer

- **Date**: 2026-08-28 (owner call — the second build-blocker round)
- **Context**: after P-D-46, four items across the set still said something could not be built:
  03's `RecognizedSet` removal, 06's P-D-19 opt-in, 11's accounting-code refusal, and the heaviest
  items behind 01 §6's `PRD` §15 pointer. Each was measured before it was put to the owner — three
  at the donor or the platform, one in this set's own git history — and all four recommendations
  were taken as put.
- **Decision**, four arms:
  1. **A `RecognizedSet` removal is a third state, never a DELETE.** The roster becomes
     `active|deprecated|removed`; the set is its `active` and `deprecated` rows and a `removed` row
     is a tombstone outside it, so a de-listed member fails `UNRECOGNIZED_UNIT` and the trigger
     whitelist stays as it was — `state` and `display_label`, no DELETE arm. Transitions:
     `active → deprecated → removed`, with `removed → active` (and `deprecated → active`) re-listing
     the same identity through the same `GovernedLiveOp`. Seeded members are still not removable.
     **The same arm closes 02's twin question** for `products_attribute_definition`: its roster gains
     `removed`, and a value on a terminal head keeps resolving because nothing is ever deleted.
  2. **P-D-19 is confirmed as amended: the per-version auto-fallback opt-in is withdrawn from v1.**
     The resolver's refusal at `complete(forced)` has one exit — every forced participant freezes or
     releases through its own door. A participant that never returns leaves the governed set
     (`inst-fz-membership`) and the next increment snapshots the reduced set; the forced version
     itself stays refused, which is the pinned default. The opt-in goes back to being what the PRD
     called it before P-D-19: an off-by-default later enhancement, with no column, door or ceremony
     in v1. P-D-19's status line records the amendment; its title keeps its historical wording.
  3. **Two codes are minted for the Finance sets**: `ACCOUNTING_CODE_DEPRECATED` (422 architectural —
     a `deprecated` code blocking new assignment) and `ACCOUNTING_CODE_DELIST_BLOCKED` (409 — removal
     refused while a non-terminal published head carries the code), one code per refusal for
     `taxCategory` and `glCode` alike, as `ACCOUNTING_CODE_UNKNOWN` already is. They are exactly as
     contingent as the two columns (`PRD` §15's ownership question) and go with them if it goes.
  4. **The gear publishes through the platform's `event-broker-sdk` outbox producer, and the
     envelope carries nothing of the toolkit outbox's.** `partition_id`/`seq` leave the envelope —
     the slot P-D-27 named is `readOnly` on the broker's schema and rejected on publish. The
     `(tenant, aggregate, sequence)` operand is the broker's read-side `sequence`; the gear sets no
     `partition_key`, so ADR-0002's default puts every event of one tenant on one partition in
     publish order; the toolkit's `seq` rides the producer chain's `meta.sequence` in managed
     monotonic mode, write-only, for ingest-side dedup. The envelope's idempotency key is the event
     `id`, which the SDK mints once at enqueue and every delivery attempt repeats. P-D-27's third
     row is re-taken; its other three stand. P-D-22 is refined, not reversed: the outbox is still
     the toolkit's, and its processor is now the SDK's producer rather than a handler of this gear's.
- **Measured, not argued**:
  - Arm 1 at the donor: `gears/bss/pricing`'s `TaxonomyState` is `Active | Retired`, and
    `pricing/src/infra/storage/repo/taxonomy_repo.rs` states why a value is never deleted — *"a
    value a published row names has to keep existing, because the row keeps naming it"* — and that
    *"a `PUT` re-adding an existing retired value re-activates it"*. Both byte-verified.
  - Arm 2 in this set's own history: `PRD.md` at `692c57989` (2026-08-24, before P-D-19 existed)
    read *"the default is **pinned fail-closed** for that participant's content (auto-fallback is an
    off-by-default later enhancement)"*. P-D-19, recorded two days later, made that enhancement the
    second disjunct of a v1 refusal predicate — the one disjunct no table, door or ceremony carried.
    No other gear has a per-version operator opt-in that relaxes a fail-closed pin.
  - Arm 3 at the donor: `TAXONOMY_VALUE_IN_USE` (409) is one code across every taxonomy class
    pricing governs, which is the shape arm 3 takes.
  - Arm 4 at the platform, the donor being silent (pricing runs a private `pricing_outbox` and
    takes no dependency on the SDK): `gears/system/event-broker/event-broker-sdk/README.md` —
    *"Outbox producers use toolkit-db `OutboxMessage.seq` as the durable local sequence and Event
    Broker cursors as the authoritative accepted sequence"*; `src/producer/outbox.rs` builds
    `meta.sequence` from that `seq` and re-uses the stored event `id` on every attempt;
    `src/producer/event_factory.rs` mints the `id` (`Uuid::now_v7()`) when the event is prepared.
    ADR-0002: the partition is MurmurHash3-32 over `partition_key`, else `tenant_id`, computed by
    the SDK for outbox routing and re-computed authoritatively at ingest.
- **The costs, stated**:
  - Arm 1: the PRD's word is "full removal", which a literal reader takes for a DELETE; the design
    now says in three places that it is a state.
  - Arm 2: a wedged version cannot be rescued in place — the only exit is a set-wide governance act
    and a new version. That is C3's roll-forward posture applied to the abnormal path, and P-D-19's
    own cost line had argued the other way.
  - Arm 4: one partition per tenant is a per-tenant throughput ceiling the bulk lane meets first;
    the named amendment path is `partition_key = tenant_id:aggregate_id`, which buys per-aggregate
    order back at the cost of cross-aggregate order. The publish path becomes the SDK's code, with a
    broker-issued producer registration this set had not priced. One residue is registered rather
    than decided: the `subject_type` the envelope requires (01 §6).
- **Propagated**: `design/01-foundation.md` (§1.4, §1.8, §4.4's outbox bullets, §6);
  `design/02-taxonomy-attributes.md` (`inst-ad-deprecate-then-remove`, §4.1, §6);
  `design/03-sku-classification.md` (§1.7, `inst-mt-recognized`, `inst-us-delist`,
  `inst-pt-governed`, `inst-ac-recognized`, §3.1, §3.2, §4, §5); `design/06-catalog-version.md`
  (C5, `inst-rv-intent`, `inst-fz-force`, §3.2, §5); `design/11-clone.md` (§3.1, §4);
  `design/12-consumer-contracts.md` (`inst-rc-dedup`, the §4.1 row-11 note); `design/README.md`;
  `PRD.md` (`fr-freeze-recovery`, AC #22, the branch-review note, three §15 rows); `DESIGN.md`
  (the flagged-decision status line).
- **Owed**: nothing. The set's build-blocker count is zero; what remains open is registered as
  questions, none of which says something cannot be built.

#### P-D-46 — Four write-path blockers, three of them settled by opening the donor

- **Date**: 2026-08-28 (owner call — the build-blocker round)
- **Context**: after the slice-12 rounds, eight items across the set still said something could not
  be built. Four are write-path questions — who writes what, where — and one of them held the
  **first migration**. Three were settled by measurement rather than by choosing between readings.
- **Decision**, four arms:
  1. **The `REVOKE` arm is withdrawn.** The trigger whitelist becomes the whole append-only guard on
     **both** engines. **P-D-35** had made `REVOKE` a Postgres-only arm; 01 §6 then measured that a
     blanket `REVOKE UPDATE, DELETE` from the writing role forbids every write the gear legitimately
     makes — head rows on save, the audit sealing UPDATE, the retention DELETE and §4.3's DELETE.
  2. **`inst-fd-save-txn` writes the entity's content rows** in the slices' own tables, in the same
     transaction. No third registration point: the door writes, the owning slice registers the
     validators, which is the mechanism already in place.
  3. **The retirement `reason` splits into two columns** — `retirement_reason` (the operator's,
     written once at `inst-rt-initiate`) and `outcome_reason` (the runner's, written on
     `applied|failed|deferred`).
  4. **`closed_at` is struck.** The bulk batch closes on the timer.
- **Measured at the donor, not argued** — three of the four:
  - Arm 1: `gears/bss/pricing` issues **no `REVOKE` anywhere**, deliberately, and says so in both
    engine tiers' tests: *"it names a deployment role the migration does not own and SQLite has no
    GRANT at all. The trigger is the portable half, and it is the half that has to work."* 01
    already names pricing "the pattern donor" **for append-only triggers with column whitelists** —
    the very pattern this arm duplicated. Quotation byte-verified against
    `pricing/tests/postgres_approval.rs`.
  - Arm 2: no registration mechanism for content writers exists in the donor's source at all, which
    priced the alternative — new machinery for two consumers, with a call-order decision attached.
  - Arm 4: pricing **D-47** states the contract as "**bulk** … coalesces into one version, hard max delay **5
    min**". Five minutes is the declared latency bound, not a fallback, so "every bulk batch
    waits the full five minutes" is conformance rather than the defect 06 read it as. An early-close
    signal would amend an inbound two-gear machine contract for an optimisation nobody requested.
- **The cost of arm 1, stated because it was argued**: the trigger defends against an application
  bug; `REVOKE` defended against someone at a psql prompt. That second ring is given up on the
  engine that holds production financial records. It is given up knowingly, on the donor's reasoning
  and because the arm as written was unimplementable in the first migration.
- **Arm 3's counter-argument, and why it lost**: a column per writer multiplies as authors are
  added, and `reason` + `reason_source` scales better. It lost because the protection would then be
  an application rule rather than the schema — the same "convention instead of a guarantee" this set
  had already recorded as lint 7's weakness one round earlier.
- **Arm 4 re-examined and confirmed (2026-08-29, owner's call)**: the cf semantic review found
  the timer call decided here but never carried into the operative rules — three of them still
  described the struck early-close signal, and after the first two were corrected `design/06`
  contradicted itself between its own rule 1 and rule 2. The owner was offered the alternative
  (restore the close marker as a sanctioned amendment of the inbound two-gear contract) and
  declined it. **The batch closes on the timer; there is no early-close signal.** The three rules
  now say so.
- **Propagated**: `design/01-foundation.md` (C5, §4.4's audit posture, `inst-fd-save-txn`);
  `design/04-lifecycle.md` (§4's transition table); `design/05-governance.md` (C7);
  `design/06-catalog-version.md` (§4's request table, and — added 2026-08-29 — `inst-cv-request`
  and `inst-cv-coalesce`); `design/09-bulk-promotion.md` (§1.5's scope statement and
  `inst-bk-commit`, never named until 2026-08-29); `PRD.md` (§15 and §16's interim control).
- **Owed**: nothing. Four of the set's eight remaining build-blockers close here; the other four are
  03's `RecognizedSet` removal, 06's P-D-19 opt-in, 11's accounting code, and 01's `PRD` §15 pointer.

#### P-D-45 — The last four lint grammars, and an event register that cannot be harvested

- **Date**: 2026-08-28 (owner call — the third slice-12 blocker round)
- **Context**: lints 3, 4, 7 and 8 were prose predicates over prose. Each is settled below, and one
  of them produced the sharpest measurement of the whole programme.
- **Decision**, four arms:
  1. **Lint 3 reads a `Doors` column** added to 05 §3.2, which becomes a table. The population is
     the **fourteen declared routes** — `` `METHOD /bss-products/v1/…` `` code spans, one
     machine-readable form. Doors named only in prose are outside it.
  2. **Lint 4 reads an authored `EventRegister`**, never a harvest.
  3. **Lint 7 reads column names**: an operator identity lives in a `*_actor_ref` column, the
     convention 10's `products_identity_ref` already follows, recorded in `DESIGN.md` §3.5.
  4. **Lint 8 needed a definition, not an artifact**: "registry schema surface" is the table and
     column declarations of the slices' §4 sections. The six §17.2 words are already a literal
     list, so the lint is executable as it stands.
- **The measurement behind arm 2, recorded because it is the evidence and not an opinion**: five
  harvest passes over one unchanged tree returned five different answers. Counting events by name
  gave **31**; a numbered-row pattern attributed 25 of them and a sub-bullet pattern 22, each
  finding rows the other missed; a literal `Emit \`X\`` pattern gave **24** and surfaced a
  **32nd** event no name census had seen (`PiiAllowlistChanged`); a widened suffix census gave
  **35**, two of them the donor gear's (`PlanPublished`, `BundleCompositionCompleted`) and two real
  ones dropped by a name-length filter (`SkuCreated`, `SkuRetired`). The emitting-instruction
  attribution disagreed in **28 of 31** rows. An emitting instruction is not recoverable from
  prose, so the register is written by each rule's owner and the lint reads only the table.
- **Two lints ship with their weakness stated rather than hidden**: lint 7's naming convention is
  enforced by the same reading it replaced — a column named otherwise passes silently, green over
  the defect it exists to catch. Lint 8 sees only §4, so a monetization marker arriving as an SDK
  field or event payload is invisible to it. Both were argued and accepted; the alternatives
  (an `identity:` field on all 34 tables; a surface spanning undeclared SDK shapes) cost more than
  they buy today.
- **Two gaps this round made countable for the first time**: fourteen of the twenty-three grant
  rows carry no route in the `Doors` column (05 §6), and the `EventRegister` is declared and empty
  (12 §6). Both are registered, neither is invented shut. *(This entry recorded the first gap as
  "sixteen of twenty-four" until 2026-08-29; the audit of this round's own propagation re-measured
  the table the round built — 23 grant rows, 9 of them routed — and found the figure wrong in the
  same commit that created the table, `5977aec64`.)*
- **Propagated**: `design/05-governance.md` (§3.2 as a table with `Doors`; §6's grant gap);
  `design/12-consumer-contracts.md` (lints 3, 4, 7, 8; §6's register item); `DESIGN.md` (§3.5's
  column convention).
- **Owed**: the `EventRegister`'s rows, per slice — the only thing now standing between the nine
  lints and a CI job that runs them.

#### P-D-44 — The AC #38 map, and the artifacts that turned out to already exist

- **Date**: 2026-08-28 (owner call — the second slice-12 blocker round)
- **Context**: lint 2's input set existed in no artifact. The code → declaring-slice half was
  settled by **P-D-35**; the row → code half lived as prose scattered across five slices, three of
  which claimed rows without listing codes. Assembling it forced three rows that do not reduce.
- **Decision**, four arms:
  1. **The post-v1 EOL row stays outside lint 2's universe.** `EOL_DISABLED` refuses *the feature
     being off in v1*, not "EOL without an acknowledged migration consumer" — a different condition,
     and lint 2 requires the code to answer the named one. `design/04-lifecycle.md`'s claim to have
     mapped the row is corrected.
  2. **The "indeterminate parent-child region-containment" row is withdrawn as unreachable.**
     **P-D-39** made both scope columns `NOT NULL` with the empty set meaning unrestricted, so every
     pair of scopes is comparable and no input produces indeterminacy. The row predates that
     decision, from the region-algebra gate that was answered a different way.
  3. **The "de-listed/deprecated unit" row splits in two.** The two conditions have different
     operands — recognition versus lifecycle — and the set already declares and raises a distinct
     code for each. One code would make one condition answer under a name that misdescribes it,
     which is arm 1's own objection.
  4. **The artifacts are named**: the `SchemaPin` is `products-sdk/schema-pin.toml`, TOML so a gate
     reads it without parsing prose. The fixture crate needed no naming — **it already exists**.
- **Measured, not chosen**: `cf-gears-bss-fixtures` ("the BSS joint golden conformance fixture
  corpus… the only fixture crate a gear may take as a production dependency") and
  `cf-gears-bss-fixtures-conformance` (runners and traits, dev-dependency only) are built, sit at
  `gears/bss/fixtures/`, and the donor gear already depends on both. Slice 12 wrote "a shared
  fixture crate" while it stood two directories away. Half of that open item closed by reading the
  tree rather than by deciding anything.
- **The count is the trap this entry wants on record**: the enumeration held at **fifteen** rows
  across arms 2 and 3 — one withdrawn, one split — while its membership changed. Every citation of
  "fifteen" was re-checked against the table and all still hold, but the number would not have
  revealed a mistake in either direction.
- **Carried, not closed**: row 11's code rests on `design/03-sku-classification.md`'s open question
  whether a `RecognizedSet` removal is a physical DELETE or a third state. Under the third-state
  reading the row has no code. The map states the dependency in the cell's own note.
- **Propagated**: `design/12-consumer-contracts.md` (§4.1, the map and the artifact table);
  `design/04-lifecycle.md` (the corrected rows-mapped claim); `PRD.md` (the enumeration, in both
  §6's FR and §12's AC #38).
- **Owed**: the five lints still without a harvest grammar (2 now has its input set, so 3, 4, 7, 8),
  and the CI job that runs any of them — both open in `design/12-consumer-contracts.md` §6.

#### P-D-43 — The checking layer's four grammars: a lint reads tokens, not prose

- **Date**: 2026-08-28 (owner call — the first of the slice-12 blocker rounds)
- **Context**: seven of the set's twenty-eight "cannot be built" items sit in the slice whose job is
  to check the other eleven, and four of those seven are the same defect wearing four faces: a lint
  whose input is prose. Nothing could be wired until they were settled, because wiring a lint that
  cannot be executed installs a red gate.
- **Decision**, four arms:
  1. **Donor-gear `inst-*` ids are struck from this set.** A citation of another gear's instruction
     id becomes prose naming the rule (`pricing's meter-binding rule`). Twelve sites in five files.
  2. **Lint 6's domain narrows to `inst-*`.** `cpt-*`/`flow` ids are declared on unnumbered bullets
     and an actor id is an Actors-table cell, so under the stated declaration grammar both kinds had
     **zero** declarations and the lint was red on a correct set by construction. An actor
     legitimately appears in every slice it acts in, and the set has no notion of an actor's owning
     slice that would make "exactly once" mean anything.
  3. **Lint 9's `Operand` cell is tokens**: one token per pin member, each a catalog field name or
     one of three non-field markers — `(surface)`, `none in v1`, `payload`. Prose beside the tokens
     is ignored.
  4. **The register carries one propagation field and one citation form**: `- **Propagated**`,
     naming documents by repo-relative path. A document **restates** a decision exactly when it
     **cites the decision id**.
- **The cost, recorded because it was argued and accepted**: arm 4's definition is the mechanical
  one, and a document can cite an id without carrying the claim — the blindness measured on
  **P-D-35**, where slice 10 cites the decision elsewhere for a different clause while the claim it
  was taken to settle never landed. The lint will not see that, and is not meant to; the claim-level
  check remains unowned. Arm 1 was taken **against the recommendation on the table**, which was to
  extend the existing scope-qualifier grammar to `inst-*` as it already runs for `AC #N`. Its stated
  price stands: three sites in `design/05-governance.md`, `design/12-consumer-contracts.md` and
  `PRD.md` no longer carry a checkable pointer into the donor gear, and the misattribution this
  programme caught twice by following such a pointer would now have to be caught by reading.
- **Propagated**: `design/12-consumer-contracts.md` (lints 5, 6 and 9, and the harvest-grammar count
  6 → 5); `design/05-governance.md`, `design/03-sku-classification.md`, `PRD.md` and this register
  (the struck donor ids); this register's own propagation fields (7 renamed, 46 citations reformed).
- **Owed**: the five lints still without a harvest grammar (2, 3, 4, 7, 8), and the job that runs
  any of them — both open in `design/12-consumer-contracts.md` §6.

#### P-D-42 — The idempotency store's last three operands

- **Date**: 2026-08-28 (owner call — the last of slice 01's own open items)
- **Context**: three operands the store named and never pinned: `in_flight_until`'s value, what the
  three `internal:` lanes write into the response columns, and what `endpoint` holds for a wire
  caller. The first had been filed as needing input this set does not hold, because no door timeout
  exists anywhere to derive a deadline from.

  **It turned out not to need one.** `in_flight_until` exists only because the claim committed in
  its own transaction, and that arrangement rests on **P-D-26**'s stated reason — that a claim
  inside the mutation's transaction would be "invisible to the concurrent duplicate the row exists
  to refuse". Measured against the donor, that reason does not hold: `gears/bss/pricing`'s
  `idempotency_repo` states in as many words that **"the gate is the insert, not a lookup"**, and a
  losing duplicate's own INSERT conflicts with the winner's *uncommitted* row and waits — then
  either finds the committed answer and replays it, or finds nothing left to conflict with, the
  winner having rolled back, and claims the key itself. Visibility is never the mechanism; the
  unique index is.

  | Call | Propagation |
  |---|---|
  | **The claim joins the mutation's transaction**, superseding P-D-26's arm. On SQLite the loser is answered `SQLITE_BUSY` rather than blocking, so the door carries a busy timeout and retries — the guarantee is identical, two are never admitted, and only the waiting differs | 01 §3.2 `inst-fd-idem-claim-txn` |
  | **`in_flight_until` is removed**, column and deadline alike. An unanswered claim was rolled back with its mutation, so nothing committed survives to expire and no row is ever left needing release. P-D-38's explicit delete-on-refusal becomes automatic for the same reason | 01 §3.2, §4.4 |
  | **An `internal:` lane stores a synthetic `200` and its own outcome record as the body.** One CHECK, one shape, no nullable-for-internal arm, and absence keeps a single meaning in these columns | 01 §4.4 |
  | **A wire caller's `endpoint` is the concrete resource path**, not the route template. Under the template two publishes of different entities under one client key share the whole key and an identical empty body hash, and the second replays the first's 200 without running — the path id being in neither the body nor, since P-D-34, the hash | 01 §3.2 `inst-fd-idem-key-scope` |

- **The arguments against, stated**: a synthetic status that never reached a wire is stored as
  though it had, and only an internal replay ever reads it; and the two lanes now name their
  subject in different components of the key — the wire lane in `endpoint`, the internal lanes in
  `client_key` — which §3.2 says once rather than leaving to be discovered.
- **Propagated**: `design/01-foundation.md` (§3.2, §4.4, §6). Amends **P-D-26** a second time (two
  of its four boundaries now stand) and simplifies **P-D-38**'s release step.
