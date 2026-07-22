<!-- CONFLUENCE_TITLE: [BSS]: Subscriptions — Cross-Gear Seam Map -->
<!-- Related: ./PRD.md, ./DESIGN.md, ./DECISIONS.md, ./STRIPE-ZUORA-GAP-ANALYSIS.md | Owners: BSS Subscriptions team -->

# Subscriptions — Cross-Gear Seam Map

> Cross-gear seam analysis produced before the Subscriptions design, against the mature
> **pricing** and **rating** design sets (`gears/bss/pricing/docs/`, `gears/bss/rating/docs/`)
> and the subscription lifecycle PRD ([`PRD.md`](./PRD.md)). Subscriptions is the **System of
> Record** for the subscription commercial aggregate: the lifecycle state machine, effective-dated
> composition (`PlanLink`/`AddOn`), the change **boundary/mode**, renewal execution, entitlement
> assignment + point-of-use check state, and multi-tenant ownership. It is **not** an authoring
> catalog and it computes **no** money — pricing owns the catalog, rating owns evaluation and
> proration math, Billing owns posting.
>
> Unlike the rating⇄pricing map (a single 1:1 complementarity), the subscription aggregate sits at
> the centre of the BSS lifecycle and its seams fan out to **many** neighbours. This map is
> therefore organised **by neighbour** (A–H), each seam carrying a stable `SUB-<letter><n>` id.
>
> **Verdict legend:** `SUB-authors` = Subscriptions is the SoR the neighbour consumes (no
> Subscriptions change beyond exposing the contract); `SUB-adopts` = Subscriptions adopts a
> neighbour-side fact verbatim (no neighbour change); `Joint` = a shared contract needs a
> co-decision; `Neighbour-extends` = a neighbour must expose/extend something; `Product` = a
> launch-scope / commercial call; `ALIGNED` = the counterpart contract is already written on the
> other side, no action beyond citing it.
>
> Severity: `CRIT` (breaks lifecycle/billing correctness), `HIGH`, `MED`, `LOW`. Line refs: `S:` =
> this gear's [`PRD.md`](./PRD.md); neighbour refs carry their path. The twelve autonomous decisions
> `SUB-D-01…12` live in [`DECISIONS.md`](./DECISIONS.md); the vendor gaps `G-1…G-6` in
> [`STRIPE-ZUORA-GAP-ANALYSIS.md`](./STRIPE-ZUORA-GAP-ANALYSIS.md).

---

## A. Rating (evaluation core + pipeline)

> The rating gear consumes subscription composition and the change boundary, and owns all
> commercial math. The counterpart contract is **already written on the rating side**
> (`gears/bss/rating/docs/PRD.md` §9.2 "Subscriptions input contract"; proration split in rating
> §6.11 / design slice `09-period-plan-change`). Ordering shares one key
> `(resourceTenantId, subscriptionId)` (S:803, rating `11-consumer-contracts`).

| # | Sev | Verdict | Seam |
|---|-----|---------|------|
| **SUB-R1** | CRIT | **Joint (field alignment open; was ALIGNED)** | **Composition read-model + change boundary.** Subscriptions exposes effective `PlanLink`/`AddOn` intervals, `PlanTier` @ `t`, the **active plan phase** @ `t`, the plan-change `(changeEffectiveAt, changeMode)`, the **committed seat quantity @ `t`** (effective-dated, SUB-D-02), the **`priceEligibility` inputs** (`activatedAt`, bound `cohort`), and the per-sale `brandId` context; rating consumes them and slices usage + prorates at the same boundary (S:1034; rating PRD §9.2, rating `design/09`). **Downgraded from ALIGNED (2026-07-15 review):** the rating counterpart names the seat count and `priceEligibility` inputs that this side's contract omitted — the two field lists MUST be mirror-reconciled before Design lock. Ordering rides the **pinned `orderingTenantId`** (immutable across transfers — SUB-D-06). **WHEN vs MATH split is binding**: Subscriptions owns the boundary/mode, rating owns proration day-count and tier-`Q`/commitment carry-vs-reset (S:590). No Subscriptions-side math. Design implements the exposure surface (slice `09-consumer-contracts`). |
| **SUB-R2** | MED | **ALIGNED (SUB-authors)** | **Snapshot `(currency, region)` segment.** Subscriptions freezes the `(currency, region)` binding at activation into the composite `pricingSnapshotRef`; rating is the composition SoR that seals the ref (S:532; rating SEAMS **S1**). **Open to confirm at design:** after SUB-D-02/05, the **seat-count provenance** (SUB-R3) and the **activation date-trio** (SUB-C4) are **not** new snapshot segments — they ride events/read-models, not the pinned pricing ref. Design must state this explicitly so no fourth Subscriptions segment silently appears. |
| **SUB-R3** | HIGH | **Joint** | **Seat-count provenance + mid-period seat boundary.** Pricing `quantitySource = subscription_seat_count` (D-18) makes this gear the seat supplier; the count rating reads MUST originate **only from committed `updateQuantity` transitions** (SUB-D-02, S:620) — never an untyped attribute edit — and is stored **effective-dated** so `quantity @ t` resolves for replay (2026-07-15 review fix; a single mutable value cannot serve the replay contract). **Open (mirrors rating's "seat-change boundary transport", rating `design/09` §4.3):** a mid-period seat change is transported as a **Subscriptions-driven change boundary** (default) that rating prorates — *not* Subscriptions-side proration. Pin the default with rating at design; increases MAY be immediate, decreases default `next-cycle` (SUB-D-02). |
| **SUB-R4** | HIGH | **Joint** | **Phase boundary = change boundary.** `convertTrial` / scheduled trial conversion advance the plan **phase** boundary; rating consumes that instant like any `changeEffectiveAt` (S:919). Phase axis is a `phase_id` (pricing D-19); Subscriptions is the **phase-structure SoR** and resolves the active phase @ `t` for rating (S:460). Confirm with rating that the phase-boundary instant travels on the same `(changeEffectiveAt, changeMode)` channel as a plan change (no second boundary vocabulary). A **trial extension** moves the phase boundary and MUST emit the boundary-move on this same channel (slice 06). |
| **SUB-R5** | MED | **Joint (discrepancy)** | **Brand context source.** This PRD publishes the **per-sale `brandId`** as a Subscriptions attribute in the evaluation context (S: §6.2, AC 20); the rating PRD's step-4 overlay scope matches `brand` against **Plan/SKU `brandId` @ `t`** (rating PRD §6 step 4, §17.4) — two different sources. Pin with rating which one feeds brand-scoped overlay matching (per-sale storefront attribution only this gear can supply vs catalog-declared membership); AC 20 is not implementable while they disagree. Tracked in PRD §15. |
| **SUB-R6** | HIGH | **Joint (SUB-D-07)** | **Recurring pricing enrichment.** Subscriptions cuts the **money-free recurring period fact** (`(subscriptionId, billing period)` key, traceability tuple, `pricingSnapshotRef`, pause/intent posture); **rating prices** the recurring component from the frozen snapshot (flat / per-unit × quantity / hybrid recurring line) and the priced line **inherits the fact's key** before Billing posts (SUB-D-07, S: §6.8, AC 27). Removes the double-producer collision with rating's step-9 recurring lines. Needs the rating counterpart contract + joint fixture before Design lock. |

## B. Pricing (Product Catalog)

> Pricing is the authoring SoR for `Plan`/`Price`/`PriceWindow`/`PriceOverlay`/`CatalogVersion` and
> publish governance; Subscriptions resolves published catalog keys and adopts its consumer
> contracts. Most seams are **adopt-verbatim** against the frozen pricing consumer contract
> (`pricing/docs/design/06-consumer-contracts.md`).

| # | Sev | Verdict | Seam |
|---|-----|---------|------|
| **SUB-P1** | HIGH | **SUB-adopts** | **Plan-change classification contract.** Pricing publishes `allowedChangeTargets` / `comparabilityRank` / the boundary class in the frozen consumer contract (pricing `design/06`); **Subscriptions classifies** an upgrade/downgrade/cross and enforces the boundary — it does not re-derive comparability. **Cross-currency / cross-region / cross-frequency = cancel+new**, not an in-place change (pricing §15 cross-boundary sign-off; S:610 overlap). Adopt the pricing classification verbatim; Subscriptions owns only the WHEN. |
| **SUB-P2** | HIGH | **SUB-adopts** | **Phase → grant-set map.** Entitlement assignment reads the plan's **published grant set**, including the **per-phase map** where the plan is phased (pricing **D-41**; S:867). `convertsToPhaseId` drives end-of-trial conversion (S:909). Catalog authors the templates; this gear resolves + materialises per subscription. Adopt the phase/grant contract; author nothing catalog-side. |
| **SUB-P3** | MED | **ALIGNED (SUB-adopts)** | **Trial sellable definition.** Catalog is authoritative for the trial offer — trial plan/SKU, promotional `PriceWindow`, or a leading trial **phase** (S:470, S:899). Subscriptions persists **evaluated** trial state (attributes + `PlanLink`/snapshot pointers), never a `trial` status (S:458). Aligned; attribute/event naming closes at design. |
| **SUB-P4** | MED | **Product (deferred)** | **Prepaid credit grant.** The prepaid credit **definition** is pricing **D-43**; the balance/drawdown is **Billing/Rating**, GA-gated (S:182). Subscriptions keeps **subscription-side hooks only** — it neither defines nor draws down the wallet. See the Billing-facing drawdown line **SUB-B4**. No launch dependency for the core lifecycle. |
| **SUB-P5** | MED | **SUB-adopts** | **Sellability / publish gate.** `PlanLink`/`AddOn` resolve only against **published** plans that pass the pricing sellability gate (pricing `design/07`; S:150). A draft or `not_sellable_ga` plan MUST fail the `create`/`changePlan` guard fail-closed. Adopt the gate as a precondition; the overlap **key** itself is registry-owned (**SUB-G1**). |

## C. Contracts & Agreements

> Contracts is the SoR for signed terms, renewal, grace, regional templates, ramps, commitment
> pools, and booking dates. **Risk:** the upstream Contracts PRD does **not yet author** several of
> these (S:1325, §16) — until it does, the platform defaults in the Subscriptions PRD govern and the
> obligation is tracked as a cross-PRD follow-up.

| # | Sev | Verdict | Seam |
|---|-----|---------|------|
| **SUB-C1** | HIGH | **Joint (RISK — upstream unauthored)** | **Renewal / grace ladder / regional templates.** §6.5 assumes Contracts is the SoR for `Renewal` (`autoRenew`, term windows, notice), grace length/ladder, and regional templates (S:674, S:718). Upstream Contracts PRD has not authored them (S:1360). **Until authored:** the platform defaults govern — **7-day grace**, **30/14/7/1 notices**, hybrid exit trigger (S:722–725). Subscriptions stores **evaluated fields** at renewal-evaluation time for replay. Cross-PRD obligation on Contracts to author the SoR; Subscriptions consumes via events + read models. |
| **SUB-C2** | MED | **Joint** | **Ramps (committed multi-step schedules).** Contracts authors the committed ramp; Subscriptions **executes** it as a sequence of scheduled `changePlan`/`updateQuantity` intents (**SUB-D-04**, S:630). No native `SubscriptionSchedule` aggregate at launch. **Open:** atomic multi-action submission (Zuora-Orders-style) is a Contracts/Design follow-up (S:1342). Depends on the SUB-D-01/02 intent envelopes. |
| **SUB-C3** | MED | **SUB-adopts (owner = Contracts)** | **Commitment pools.** Committed-usage pools are **Contracts SoR**, true-up is **rating** (rating **T-D-14**, rating SEAMS **M8**; S:181). This gear keeps **subscription-side hooks only** — it neither owns the pool balance nor computes the true-up. Adopt the owner split; expose the subscription linkage. |
| **SUB-C4** | MED | **Joint** | **Activation date-trio + acceptance.** `contractEffectiveAt` (booking) is **referenced from the Contract** — booking semantics stay Contracts/Finance SoR; `serviceActivatedAt` is stamped at the `activate` commit here; `customerAcceptedAt` is stamped by an optional **acceptance confirmation** where Contract clauses require it, else = service activation (**SUB-D-05**, S:500). **No new statuses.** All three ride lifecycle events + ASC hooks. **Open:** the confirmation-flow shape (who confirms, evidence) is design (S:1343). |
| **SUB-C5** | MED | **SUB-adopts** | **`PriceOverride` windows.** Contracts supplies negotiated override windows consumed via events/read models; in rating these are the **step-5 contract overlay** (rating `04-overlays-precedence`, precedence Contract > Partner PriceOverlay > Catalog base). Subscriptions references the override binding for composition/renewal; it does not evaluate the override. |

## D. Billing & Invoicing

> Billing ingests recurring `BillableItem`s, posts immutable invoices, executes
> adjustments/credit/debit notes and dunning, and owns floor/cap + rounding. This section is the
> single **Billing-facing** surface: it folds in the still-open **pricing** gap **G-4** (prepaid
> drawdown / tax placement) as a joint line, since that too resolves at the Billing boundary.

| # | Sev | Verdict | Seam |
|---|-----|---------|------|
| **SUB-B1** | HIGH | **ALIGNED (SUB-authors)** | **Recurring idempotency + no-retro-edit + traceability.** `BillableItemCreated(kind=recurring)` idempotent per `(subscriptionId, billing period)` (S:815); posted invoice lines never rewritten — corrections flow as new billable/adjustment artifacts (S:825); every item traces to `{subscriptionId, skuId, planId, priceId}` + `pricingSnapshotRef` (S:835). What crosses this seam is the **money-free period fact**; the priced line arrives at Billing via the rating enrichment (**SUB-R6**, SUB-D-07). These are manifest §4.3/§4.4 invariants shared with Billing; design exposes the handoff payload (slice `08-events-billing`, `09-consumer-contracts`). |
| **SUB-B2** | MED | **Joint** | **`collectionPaused` artifact treatment.** The billing-only pause (service running, collection suppressed/deferred) is a Subscriptions **attribute posture on `active`** (**SUB-D-03**, S:662); **Billing chooses the artifact treatment** per policy (suppress vs defer) — the period fact is still emitted, marked `collectionPaused`, so Billing has the artifact to treat (AC 24 "not posted" holds). Renewal **collection** inside the window is deferred with it (payment pre-check/grace/dunning suspended; term extension continues — **SUB-D-12**, AC 29). **Open (§15):** pause-day limits and resume-proration mechanics — Product/Billing (S:1347). Finance may still prefer a Billing-side AR hold; the aggregate-posture shape keeps the audit trail on the subscription either way. |
| **SUB-B3** | MED | **SUB-adopts (owner = Billing)** | **Period floor/cap + rounding execution.** Floor/cap and rounding are **Billing-executed** (rating §17.1 post-step-9; rating `09-period-plan-change` `PeriodFloorCapObligation`). Subscriptions neither floors nor rounds; it coordinates the artifacts only. Adopt the owner. |
| **SUB-B4** | MED | **Joint (pricing G-4)** | **Prepaid drawdown + tax placement.** The pricing gap **G-4** (`STRIPE-GAP-ANALYSIS.md`, still open) places prepaid-credit **drawdown** and **tax** at the Billing boundary; the pricing D-43 grant (**SUB-P4**) is defined but Billing/Rating execute the balance. Subscriptions supplies the subscription-side hooks (which subscription, which grant reference) only. Resolve jointly with pricing + Billing before prepaid GA. |
| **SUB-B5** | MED | **SUB-authors** | **Dunning handoff.** Post-renewal billing failure hands off to **dunning** (Billing/Payments §4.4–4.5); the §6.5 grace rules and triggers apply (S:707). Subscriptions emits the failure/grace signals and the audit trail; **dunning execution + PSP webhook payloads are Billing/Payments + Design** (S:1058, S:1326). |
| **SUB-B6** | MED | **Neighbour-extends (Billing)** | **Posted-period watermark for the backdating guard.** The §6.3 backdating guard ("reject a boundary inside an already-posted invoice period", AC 6) needs to *know* what is posted — Billing MUST expose a per-subscription **`billedThroughAt`** watermark (read model/event) that Subscriptions consumes fail-closed (unknown watermark ⇒ treat as posted, reject). Identified by the 2026-07-15 design review: without this the guard has no data source (design slice 03 §4.6). |

## E. Policy Engine & OSS Provisioning

> Every resource-affecting transition is fail-closed gated by Policy before commit; OSS executes
> provisioning; entitlement **enforcement** executes in OSS while the **decision state** stays here.

| # | Sev | Verdict | Seam |
|---|-----|---------|------|
| **SUB-E1** | CRIT | **ALIGNED (SUB-adopts)** | **Fail-closed Policy gate.** Every resource-affecting transition passes a pre-commit allow/deny + `reasonCodes`; on deny **or unavailability** the state MUST NOT change (S:450, S:1046; AC 1). Manifest §6. Aligned; design maps the gate call + reason surfacing. |
| **SUB-E2** | HIGH | **SUB-authors** | **OSS provisioning confirmation.** `activate`/`suspend`/`resume`/`cancel` coordinate provision/deprovision/pause **work orders confirmed by events**; **BSS never mutates OSS resource topology directly** (S:272, S:1052). Subscriptions issues the intent + consumes the confirmation; it does not touch OSS state. |
| **SUB-E3** | HIGH | **Joint** | **Entitlement enforcement split.** Subscriptions serves the **point-of-use check decision state** (feature flag, quota remaining, limit state) at **p95 < 100ms** (S:877); **OSS enforces** (allow/block/degrade) — this gear never executes enforcement. **Open (§15):** mid-request behaviour at the exhaustion instant (graceful degradation vs hard block) is OSS/Design (S:887, S:1344). Pin the check-state ↔ enforcement contract with OSS at design. |

## F. Payments & Notifications

| # | Sev | Verdict | Seam |
|---|-----|---------|------|
| **SUB-F1** | MED | **SUB-adopts** | **Payments signals.** Payment **pre-check** outcomes + **retry-exhaustion** declarations feed the renewal/grace ladder (S:1056); authorization is requested at renewal / trial conversion. **PSP capture + webhook behaviour are out of scope** (S:278, §4.5) — this gear only triggers requests and consumes outcome signals. The grace exit trigger (ii) — "no further automated retries" — is a Payments declaration (S:725). |
| **SUB-F2** | LOW | **SUB-authors** | **Notification triggers.** Renewal-notice **triggers + intervals** (30/14/7/1) and the trial-expiry **win-back hook** are owned here (S:694, S:929); **delivery channels are Notifications/Comms** (out of scope §5.2). Subscriptions emits the trigger events; it never delivers. |

## G. Catalog Registry (Product & SKU)

> The registry carves Product/SKU/Category/Attribute/`PlanTier` taxonomy/`CatalogVersion` out of the
> manifest. It is now **vendored on this branch** as the **products** gear (`gears/bss/products/docs/PRD.md`, 2026-07-16, from upstream **PR #4177**; merged in
> vhp-architecture `main`, tracked in [product-sku-prd-location]). Seams are tracked **prospectively**.

| # | Sev | Verdict | Seam |
|---|-----|---------|------|
| **SUB-G1** | HIGH | **Joint (prospective, PR #4177)** | **`overlapScopeKey` binds to a registry key.** The overlap cardinality rule defaults to at most **one `active`** per `(payerTenantId, catalogSubscriptionProductKey)` (S:610). `catalogSubscriptionProductKey` is the **registry-owned** stable key for the sellable subscription product/family; **Design binds the stored field to a published SKU/product key**. Engage on PR #4177 before merge so the key shape is agreed; `maxConcurrentActive` override may come from Catalog **or** Contract. |
| **SUB-G2** | MED | **SUB-adopts** | **Published `skuId` / `PlanTier` / `CatalogVersion`.** Subscriptions reads **published** registry facts only (S:242); `PlanLink`/overlap key bind to published keys; effective `PlanTier` MUST be derivable @ event time (S:540). Read-only consumer; never re-authors taxonomy. |

## H. Naming / cross-cutting

| # | Sev | Verdict | Seam |
|---|-----|---------|------|
| **SUB-N1** | LOW | **Joint (manifest alignment)** | **New `TransitionRequest.type` values + scheduled-intent envelope.** `updateQuantity` (SUB-D-02), `convertTrial` (G-2), and the SUB-D-08 completion set (`renew`, `unschedule`, `pauseCollection`, `resumeCollection`, `confirmAcceptance`, `extendTrial`) extend the manifest §4.3 `type` list; the scheduled-intent envelope (`cancelMode`, `resumeAt`) extends the change vocabulary (S:480, S:490). Manifest alignment is tracked in §15; **not re-opened here** — the seam records that downstream consumers keying on `TransitionRequest.type` must not fail-closed on the new values before the manifest lands. |
| **SUB-N2** | LOW | **Naming** | **Gear-name pinning.** "Pricing (Product Catalog)", "Rating (evaluation core + pipeline)", "Catalog registry (Product & SKU)" are pinned in §2.1 (S:156). Downstream references MUST use these canonical names; no drift to legacy "Tariffs"/"PLAL"/"Price Book". |

---

## Ownership matrix (contested / adjacent responsibilities)

| Responsibility | Owner | Seam |
|---|---|---|
| Subscription lifecycle state machine + terminality | **Subscriptions** (SoR) | SUB-R1 |
| Change **boundary/mode** (`changeEffectiveAt`, `changeMode`) | **Subscriptions** | SUB-R1, SUB-R3 |
| Proration / plan-change **math** | **Rating** (evaluates); Subscriptions sets the boundary | SUB-R1 |
| Usage slicing at the change boundary | **Rating** (pipeline) | SUB-R1 |
| `(currency, region)` snapshot segment | **Subscriptions** @ activation; rating seals the ref | SUB-R2 |
| Seat-count provenance (effective-dated, `quantity @ t`) | **Subscriptions** (committed `updateQuantity` only); rating consumes frozen | SUB-R3 |
| Ordering/partition key (`orderingTenantId`, pinned at creation) | **Subscriptions** — immutable across transfers | SUB-R1 |
| Recurring period **cut** (anchor, pauses, intents, idempotency key) | **Subscriptions**; rating **prices** the fact; Billing posts | SUB-R6, SUB-B1 |
| Brand context source for overlay matching | **Contested** — per-sale (here) vs Plan/SKU (rating); pin at design | SUB-R5 |
| Posted-period watermark (`billedThroughAt`) | **Billing** exposes; Subscriptions enforces the backdating guard | SUB-B6 |
| Plan phase **structure** | **Subscriptions** (SoR); rating resolves @ `t` | SUB-R4 |
| Plan-change **classification** (comparability/targets) | **Pricing** (publishes); Subscriptions enforces | SUB-P1 |
| Entitlement grant-set **templates** (incl. per-phase) | **Pricing** (authors); Subscriptions assigns | SUB-P2 |
| Entitlement **assignment** per subscription | **Subscriptions** | SUB-P2 |
| Entitlement point-of-use **decision state** | **Subscriptions** (serves); OSS enforces | SUB-E3 |
| Trial sellable **definition** | **Pricing/Catalog**; Contract legal clauses | SUB-P3, SUB-C1 |
| Prepaid credit **definition** / **drawdown** | **Pricing** (D-43 def) / **Billing+Rating** (balance) | SUB-P4, SUB-B4 |
| Renewal / grace / regional templates SoR | **Contracts** (unauthored upstream → platform default) | SUB-C1 |
| Ramp authoring / execution | **Contracts** (authors) / **Subscriptions** (executes intents) | SUB-C2 |
| Commitment pools | **Contracts** SoR; true-up = Rating; Subscriptions hooks only | SUB-C3 |
| Booking date (`contractEffectiveAt`) | **Contracts/Finance**; Subscriptions references | SUB-C4 |
| Recurring idempotency + posted-invoice immutability | **Subscriptions** (keys/emits) + **Billing** (posts) | SUB-B1 |
| `collectionPaused` posture / artifact treatment | **Subscriptions** (posture) / **Billing** (artifact) | SUB-B2 |
| Period floor/cap + rounding | **Billing** | SUB-B3 |
| Dunning execution + PSP webhooks | **Billing/Payments** | SUB-B5, SUB-F1 |
| Fail-closed transition gate | **Policy Engine** | SUB-E1 |
| OSS resource topology | **OSS** (BSS never mutates directly) | SUB-E2 |
| Notice/win-back **delivery** | **Notifications/Comms** | SUB-F2 |
| `catalogSubscriptionProductKey` / published SKU/`CatalogVersion` | **Registry** (Product & SKU) | SUB-G1, SUB-G2 |

---

## Decisions register (to close before Design lock)

**Resolved on this gear (autonomous, flagged for veto — [`DECISIONS.md`](./DECISIONS.md)):**
- **SUB-D-01** — scheduled lifecycle intents (`cancelMode`, `resumeAt`; pending intents suppress renewal/next-term recurring). Seams SUB-C2, SUB-B2.
- **SUB-D-02** — `updateQuantity` first-class transition with the change envelope; up/down asymmetry. Seams SUB-R3, SUB-P1.
- **SUB-D-03** — `collectionPaused` posture on `active`. Seam SUB-B2.
- **SUB-D-04** — ramps: Contracts authors, Subscriptions executes scheduled intents. Seam SUB-C2.
- **SUB-D-05** — activation date-trio as attributes; no new statuses. Seam SUB-C4.
- **SUB-D-06** — ordering tenant pinned at creation; transfers never rebind the ordering/partition key. Seam SUB-R1.
- **SUB-D-07** — recurring split: money-free period fact here, rating prices, Billing posts. Seams SUB-R6, SUB-B1.
- **SUB-D-08** — mutation-type inventory completed (`renew`, `unschedule`, `pauseCollection`/`resumeCollection`, `confirmAcceptance`, `extendTrial`). Seam SUB-N1.
- **SUB-D-09** — secondary producer-event inventory named in design slice 08. Seams SUB-R1, SUB-B1.
- **SUB-D-10** — entitlement check surface: bounded-staleness degraded mode. Seam SUB-E3.
- **SUB-D-11** — `draft → cancelled` (void) edge. (Status machine; no cross-gear seam.)
- **SUB-D-12** — `collectionPaused` defers renewal collection (pre-check/grace/dunning), not term extension. Seam SUB-B2.

**Aligned (counterpart written; no action beyond citing):**
- SUB-R2 (rating SEAMS S1), SUB-P3, SUB-B1, SUB-E1.

**Joint / cross-PRD obligations still open (engage the owner before Design lock):**
- **SUB-C1** — Contracts must author the renewal/grace/regional-template SoR (upstream PRD unauthored; §16 risk). Highest cross-PRD risk.
- **SUB-R1** — mirror-reconcile the read-model field lists with rating PRD §9.2 (seat quantity @ `t`, `priceEligibility` inputs; downgraded from ALIGNED by the 2026-07-15 review).
- **SUB-R3** — mid-period seat-change boundary transport (default: change-boundary) — pin with rating.
- **SUB-R4** — phase-boundary instant on the shared `(changeEffectiveAt, changeMode)` channel (incl. trial extension moves) — confirm with rating.
- **SUB-R5** — brand context source for overlay matching (per-sale vs Plan/SKU) — pin with rating; AC 20 blocked until resolved.
- **SUB-R6** — recurring pricing enrichment (SUB-D-07): rating counterpart contract + joint fixture.
- **SUB-B6** — Billing to expose the `billedThroughAt` posted-period watermark for the backdating guard.
- **SUB-E3** — check-state ↔ OSS enforcement contract + quota mid-request instant — pin with OSS; staleness-budget default (SUB-D-10) to confirm.
- **SUB-B2 / SUB-B4** — pause-day limits + resume proration (Product/Billing); prepaid drawdown/tax placement (pricing G-4) — pin with Billing.
- **SUB-C2** — atomic multi-action ramp submission — Contracts/Design follow-up.
- **SUB-C4** — acceptance-confirmation flow shape — Design.
- **SUB-G1** — `catalogSubscriptionProductKey` shape — engage PR #4177 before merge.

**Manifest alignment (tracked §15, not a design blocker):**
- **SUB-N1** — `updateQuantity` / `convertTrial` `TransitionRequest.type` values + `cancelMode`/`resumeAt` envelope await manifest §4.3 alignment.

**Propagation status:** the SUB-D-01…12 decisions are propagated into [`PRD.md`](./PRD.md)
(§5.1/§6/§12/§15) and [`DECISIONS.md`](./DECISIONS.md); vendor gaps G-1…G-6 are all processed
([`STRIPE-ZUORA-GAP-ANALYSIS.md`](./STRIPE-ZUORA-GAP-ANALYSIS.md)). This seam map is the input to
the design set — each slice implements the Subscriptions side of the seams listed for it in
[`DESIGN.md`](./DESIGN.md) §1.3.
