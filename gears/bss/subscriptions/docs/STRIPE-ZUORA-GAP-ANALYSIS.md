<!-- CONFLUENCE_TITLE: [BSS]: Subscriptions — Stripe & Zuora Subscriptions Gap Analysis -->
<!-- Related: ./PRD.md | Sibling analysis: ../../pricing/docs/STRIPE-GAP-ANALYSIS.md | Owners: BSS Subscriptions team -->

# Subscriptions — Stripe & Zuora Gap Analysis

<!-- toc -->

- [1. Purpose & method](#1-purpose--method)
- [2. Scope boundary — vendor features that are NOT ours](#2-scope-boundary--vendor-features-that-are-not-ours)
- [3. Coverage map — vendor model vs what we have](#3-coverage-map--vendor-model-vs-what-we-have)
  - [3.1 Status mapping](#31-status-mapping)
  - [3.2 Feature coverage (HAVE)](#32-feature-coverage-have)
- [4. Already deferred or consciously split, with vendor equivalents](#4-already-deferred-or-consciously-split-with-vendor-equivalents)
- [5. Gaps worth acting on](#5-gaps-worth-acting-on)
  - [G-1. Scheduled lifecycle intents (cancel at period end / at date; scheduled resume)](#g-1-scheduled-lifecycle-intents-cancel-at-period-end--at-date-scheduled-resume)
  - [G-2. First-class early trial conversion](#g-2-first-class-early-trial-conversion)
  - [G-3. Seat / quantity change as a lifecycle transition](#g-3-seat--quantity-change-as-a-lifecycle-transition)
  - [G-4. Billing-only pause (collection paused, service running)](#g-4-billing-only-pause-collection-paused-service-running)
  - [G-5. Committed multi-step schedules (ramps) and atomic multi-action orders](#g-5-committed-multi-step-schedules-ramps-and-atomic-multi-action-orders)
  - [G-6. Activation date trio (booking / service / acceptance)](#g-6-activation-date-trio-booking--service--acceptance)
- [6. Recommendation summary](#6-recommendation-summary)

<!-- /toc -->

## 1. Purpose & method

A comparison of the vendored **Subscriptions lifecycle PRD** ([PRD.md](./PRD.md), from vhp-architecture
PR #154) against **Stripe Billing subscriptions** (statuses, schedules, trials, pause, proration
behavior, cancellation semantics) and **Zuora** (subscription versions/amendments/orders, terms,
effective-date trio, suspend/resume, owner transfer, ramps), to answer: *what lifecycle
capabilities are we missing?*

Method (same buckets as the [pricing analysis](../../pricing/docs/STRIPE-GAP-ANALYSIS.md)): each
vendor concept is **HAVE** (already modelled), **DEFERRED/SPLIT** (consciously out of this PRD with
a recorded home), **OTHER GEAR** (correctly not a lifecycle concern), or **GAP** (worth a
decision). Only §5 items are proposed work.

Sources checked (the pricing analysis initially failed by reading one gear only — not repeated
here): [PRD.md](./PRD.md) (the lifecycle PRD incl. its §15/§16 open items); the
absorbed predecessor `PRD-subscriptions-entitlements-202601120119` scope split (§2.2);
the BSS manifest §4.3 **status-enum constraint** (no new statuses without a manifest change);
the **pricing gear** ([PRD](../../pricing/docs/PRD.md) — phases D-19/D-41, `quantitySource`
D-18, `allowedChangeTargets`/`comparabilityRank`, §17.8 deferrals, cross-currency cancel+new)
and the **rating gear** ([PRD](../../rating/docs/PRD.md) — §6.11/§17.2 plan-change proration,
carry-vs-reset, ordering key).

> **Local reconciliation finding (actioned with this analysis):** pricing PRD carried two stale
> phrases ("proration math is owned by Subscriptions", "Subscriptions executes proration") that
> contradict the normative three-way split (Subscriptions = change boundary/mode; rating =
> proration math; Billing = artifacts) stated by rating §6.11/§17.2 and [PRD.md](./PRD.md) §6.3 —
> the same defect the upstream reviewer flagged on PR #154 against the older upstream PRDs. Fixed
> in the pricing PRD alongside this document.

> **Consolidation note (2026-07-15):** after this analysis was written, the predecessor
> `PRD-subscriptions-entitlements` was **merged into [PRD.md](./PRD.md)** (§2.2 section map
> there). Its own competitor matrix (Chargebee / Stripe / Zuora / CloudBlue) was reviewed and is
> consistent with this analysis: pause/resume popularity → G-4; renewal price lock → covered by
> pricing grandfathering (§4); trial conversion without re-entering payment details → G-2 (now
> actioned).

## 2. Scope boundary — vendor features that are NOT ours

Stripe/Zuora bundle payment, invoicing, and portal behavior into "subscriptions". In our gear
decomposition these are **OTHER GEAR** — recorded so the boundary is explicit:

| Vendor feature | Owner in our architecture |
|---|---|
| `collection_method` (charge automatically vs send invoice + `days_until_due`) | Billing / Payments |
| Smart retries / dunning execution, retry schedules | Billing / Payments (Subscriptions consumes pre-check + retry-exhaustion signals — PRD §6.5) |
| Billing thresholds (invoice mid-cycle when usage amount crosses X) | Billing (post-aggregation spend caps; rating PRD §5.2 exclusion) |
| Invoice preview / upcoming invoice endpoint | Preview **owner** named in Design; calculation authority = rating gear (PRD §11) |
| Credit memos / credit notes, refunds execution | Billing / Payments |
| Tax on subscription invoices | Billing / Tax Engine |
| Customer portal (self-service UI shell) | Subscriptions/Frontend Design, not lifecycle PRD scope |
| Payment method management, `payment_behavior` at create | Payments (PSP) |
| Booking / revenue metrics (Zuora MRR, TCB/TCV deltas) | Analytics/DWH over lifecycle facts (PRD §6.7) |
| Proration **math** (`proration_behavior` arithmetic, day counts) | Rating gear (`prorationBasis` frozen in snapshot; rating §6.11/§17.2) |
| Coupons/discounts on subscription | Promotions (absent) + rating step 7 — see the [pricing analysis](../../pricing/docs/STRIPE-GAP-ANALYSIS.md) G-1 |

## 3. Coverage map — vendor model vs what we have

### 3.1 Status mapping

The manifest enum is closed (`draft | active | suspended | cancelled | archived`); vendor
statuses map onto **status + attributes/phase**, per the PRD's trials decision:

| Stripe status | Zuora status | Ours |
|---|---|---|
| `incomplete` / `incomplete_expired` | Draft | `draft` (+ payment-gated activation ordering — Design/Payments; see also G-6) |
| `trialing` | — (trial on rate plan) | `active` (or `draft`) + **trial phase / attributes** (PRD §6.1 — no `trial` status by design) |
| `active` | Active | `active` |
| `past_due` | — | `active` + **grace** (evaluated fields, 7-day default ladder — PRD §6.5) |
| `unpaid` | Suspended | `suspended` (grace ladder exit) |
| `paused` (`pause_collection`) | Suspended (with resume date) | `active` + **`collectionPaused`** posture (SUB-D-03, PRD §6.4); scheduled resume via `resumeAt` (SUB-D-01, PRD §6.1) |
| `canceled` | Cancelled | `cancelled` |
| — | Expired (term end) | `cancelled` via renewal ladder / non-renewal (Contract term) |
| — | — | `archived` (retention terminal — ours is richer) |

### 3.2 Feature coverage (HAVE)

| Vendor concept | Where we model it |
|---|---|
| Subscription as versioned aggregate (Zuora versions/amendments) | Monotonic `version` + immutable revisions + effective-dated `PlanLink`/`AddOn` (PRD §6.2) |
| Amendment types (NewProduct/RemoveProduct/UpdateProduct/Renewal/Cancellation/OwnerTransfer) | `TransitionRequest.type` inventory + composition ops (PRD §6.1) — see G-3 for the quantity residue |
| Plan changes with proration (Stripe `proration_behavior`, Zuora amendments) | Subscriptions owns `(changeEffectiveAt, changeMode)`; rating owns math on frozen `prorationBasis`; Billing materializes artifacts (PRD §6.3; rating §17.2) |
| Change timing modes (Stripe schedule at period end; Zuora effective dates) | `changeMode ∈ immediate / next-cycle / end-of-term` **on plan change** (PRD §6.3) — cancel/resume timing is G-1 |
| Allowed-change governance (no vendor equivalent) | `allowedChangeTargets` + `comparabilityRank` published by pricing, enforced by Subscriptions (pricing design/06) — **stronger than both vendors** |
| Tier counter behavior across a change (no vendor equivalent) | Tier-`Q` / commitment carry-vs-reset frozen in snapshot (rating §17.2) |
| Trials (Stripe `trial_period_days`, trial price) | Catalog-first trial offers + leading **trial phase** (pricing D-19/D-41) + subscription attributes (PRD §6.1); conversion depth in predecessor PRD |
| Backdating (Stripe `backdate_start_date`, Zuora backdated orders) | §6.3 backdated rules + AC 6 (reject when a posted invoice would be contradicted → adjustment path) |
| Renewal & auto-renew (Zuora terms; Stripe implicit evergreen) | Contract §4.6 `Renewal.autoRenew` + term windows; renewal job + evaluated fields; idempotent attempts (PRD §6.5) |
| Failed payment ladder (Stripe `past_due→unpaid/canceled` per settings) | Grace policy: 7-day default, paused next-term recurring, Contract SoR, hybrid exit (PRD §6.5) — Contract-configurable, i.e. stronger |
| Suspend / resume (Zuora suspend/resume) | `suspended` state + Policy-gated `resume`, entitlement freeze/re-issue (PRD §6.4) |
| Owner transfer (Zuora invoice-owner vs subscription-owner) | `payerTenantId` vs `resourceTenantId` vs `sellerTenantId` axes + `transfer` with Approval + `OwnershipTransfer*` events + delegation proofs (PRD §6.6) — stronger |
| Duplicate-subscription guard (no first-class vendor equivalent) | `overlapScopeKey` + `maxConcurrentActive` fail-closed on activate (PRD §6.3) |
| Recurring charge generation (Stripe invoice cycle; Zuora bill runs) | `BillableItem(kind=recurring)` idempotent per `(subscriptionId, period)`; `billingAnchor` + pricing `billingAnchorPolicy` (PRD §6.8) |
| Cancellation reasons / audit (Stripe `cancellation_details`) | TransitionRequest + audit lineage + events (PRD §6.1/§6.7); free-form reason fields are Design |
| Webhooks / event feed | CloudEvents 1.0 producer inventory + `(tenantId, subscriptionId)` ordering (PRD §6.7) |
| Quantity-based pricing (Stripe `items.quantity`) | `quantitySource = subscription_seat_count` — Subscriptions supplies the per-period count (pricing D-18); the **update verb** is G-3 |

## 4. Already deferred or consciously split, with vendor equivalents

Not gaps — each has a recorded home; mapped so nobody rediscovers them:

| Deferred / split item | Vendor analogue | Where recorded |
|---|---|---|
| Self-service `termLength` / `autoRenew` on the Plan | Zuora termed subscriptions (initial + renewal terms) | pricing §17.8 (Follow-on, decided 2026-07-04); negotiated terms = Contracts §4.6 — split is deliberate |
| Cross-currency / region / frequency mid-cycle change | Stripe allows currency-crossing schedule phases | pricing PRD: **cancel + new**, in-place rejected at launch |
| Trial runtime, conversion, notices | Stripe trial emails / portal | **merged** into [PRD.md](./PRD.md) §6.10 (runtime/conversion) + §6.5 (notice triggers); campaign content & delivery = Notifications (PRD §5.2). G-2 operation actioned |
| Entitlement enforcement at point of use (quotas, flags, exhaustion) | — (vendors don't own entitlements) | **merged** PRD §6.9 — state + check contract here (p95 < 100ms), enforcement execution = OSS |
| Committed usage / prepaid commercial variants | Zuora prepaid drawdown | Contracts pools + rating true-up (T-D-14) + pricing prepaid grant (D-43); subscription-side hooks in the merged PRD (§2.2) |
| Renewal price lock at renewal | Zuora renewal price lock (enterprise churn guard) | pricing `priceEligibility` + `cohort` grandfathering generations (pricing ADR-0002); the predecessor's P1 scope row is superseded there |
| Rule-based change targets | — | pricing D-23: explicit `allowedChangeTargets` lists at launch; rules = Future |
| Payment-gated plan change (Stripe `pending_updates` — apply only if payment succeeds) | Stripe `pending_updates` | Payments/Billing integration detail — Design scope per PRD §2 (manifest silence list); revisit if self-service upgrades require payment-first commit |
| Event attribute matrices, REST/header bindings | Stripe API schemas | `DESIGN-subscriptions-*` (PRD content boundary) |

## 5. Gaps worth acting on

Ranked. Each is a real delta against Stripe/Zuora that is **not** already deferred with a
recorded home and is a lifecycle (this-gear) concern.

### G-1. Scheduled lifecycle intents (cancel at period end / at date; scheduled resume)

**State today**: `changeMode` (immediate / next-cycle / end-of-term) exists **only for
`changePlan`** (PRD §6.3). `cancel` is an immediate transition; `resume` is an operator action.
There is no way to record the single most common self-service intent — **"cancel at period
end"** — nor "suspend until date / resume at date".

**Vendor model**: Stripe `cancel_at_period_end` + `cancel_at` (and un-scheduling by clearing
them), `cancellation_details`; Zuora cancellation policies (end of term / end of last invoice
period / specific date) and **suspend with resume date**. The predecessor PRD's own glossary
already names the concept — **Cancellation Policy** "(immediate, end-of-term, prorated refund
eligibility)" — but the lifecycle PRD carried no operation for it.

**Why it matters**: without a first-class scheduled intent, portals implement "cancel at period
end" as an external cron + immediate cancel — invisible to renewal evaluation (§6.5 would still
attempt renewal), to Billing (next-term recurring must not be emitted for a lapsing term), and
to audit. The intent must live **on the aggregate** so the renewal job, grace ladder, and event
consumers see it.

**Options**: (a) extend `TransitionRequest` with the same effective-timing envelope plan changes
already have — `cancelMode ∈ {immediate, end_of_term, at(date)}` (+ `resumeAt` on suspend), a
stored **pending intent** on the subscription, cancellable until effective, evaluated by the
renewal job; (b) leave scheduling to portal-side automation (status quo).

**Recommendation**: (a) — it reuses the §6.3 mode vocabulary, closes the renewal-interaction
hole, and is AC-testable ("a subscription with a pending end-of-term cancel MUST NOT renew and
MUST NOT emit next-term recurring"). Smallest high-value addition in this analysis.

**Actioned — SUB-D-01 (2026-07-15):** merged PRD §6.1 `fr-scheduled-intents` (+ AC 21–22):
`cancelMode { immediate, end_of_term, at(date) }`, `resumeAt`, pending intents on the aggregate
with renewal suppression and un-scheduling; manifest envelope alignment tracked in PRD §15.

### G-2. First-class early trial conversion

**State today**: trials are a leading **plan phase** with `convertsToPhaseId` +
`phaseDurationDays` (pricing D-19); conversion happens when the phase clock runs out. The
lifecycle PRD treats early conversion as a generic composition/attribute change and defers
conversion **workflows** to the predecessor. There is no `convertTrial` / phase-advance
**operation** — a long-standing open item (also visible in the studio prototype's
"Convert to paid now" button, which today maps to nothing normative).

**Vendor model**: Stripe — set `trial_end = now` on the subscription (first-class, prorates and
starts the paid cycle immediately); Zuora — amendment moving the charge segment.

**Why it matters**: "skip the trial, start paying now" is a conversion-funnel operation product
teams run constantly; modeling it as an untyped attribute edit loses the Policy gate, the
entitlement re-issue for the new phase (trial caps → evergreen caps, pricing D-41), the event
(`SubscriptionConverted`-class), and idempotency.

**Options**: (a) add a `convertTrial` (phase-advance) `TransitionRequest` type: sets the phase
boundary at `now` (a `changeEffectiveAt` for the phase axis), re-issues entitlements per the
phase grant map, emits a first-class event; (b) express it as `changePlan` onto the same plan
with a phase override (abuses plan-change semantics); (c) keep it predecessor/Design territory.

**Recommendation**: (a) — it is the phase-axis twin of `changePlan` and everything it needs
(phase map, per-phase grants, proration boundary consumption) already exists in the pricing and
rating gears.

**Actioned — merged PRD §6.10 (2026-07-15):** `convertTrial` is now first-class
(`fr-trial-early-conversion` + AC 17): explicit `TransitionRequest`, phase boundary at `now`
consumed by the rating gear, entitlement re-issue per the target phase, first-class event; the
manifest `TransitionRequest.type` extension is tracked in PRD §15.

### G-3. Seat / quantity change as a lifecycle transition

**State today**: per-seat pricing resolves quantity from `quantitySource =
subscription_seat_count` — "Subscriptions supplies the per-period seat count at rating time"
(pricing D-18). But the lifecycle PRD's `TransitionRequest.type` inventory has **no quantity
operation**: seat-count changes are not a modeled transition, have no `changeMode`/proration
trigger, no event, no idempotency envelope.

**Vendor model**: Stripe `items.quantity` update with `proration_behavior`; Zuora
`UpdateProduct` amendment (quantity) with effective date.

**Why it matters**: mid-period seat growth is the most frequent commercial mutation on B2B
subscriptions. Without a transition it cannot be Policy-gated (seats often carry entitlement
quotas), cannot prorate deterministically (rating needs a boundary), and the seat count Rating
reads has no auditable provenance.

**Options**: (a) add `updateQuantity` to the `TransitionRequest` inventory with the §6.3
envelope (`changeEffectiveAt`, `changeMode`), emitting a composition-changing event consumed
like `SubscriptionPlanChanged`; (b) treat seat count as a plain attribute with audit only (no
proration boundary — mid-period seat changes bill from next cycle only).

**Recommendation**: (a), with (b)'s "next-cycle only" as the launch-default `changeMode` if
Product wants to avoid mid-period seat proration initially — the envelope still makes that an
explicit, auditable choice.

**Actioned — SUB-D-02 (2026-07-15):** merged PRD §6.3 `fr-update-quantity` (+ AC 23, §6.1 type
list): full envelope; increases MAY be immediate (prorated by rating), decreases default
next-cycle; seat counts consumed by rating originate from committed transitions only.

### G-4. Billing-only pause (collection paused, service running)

**State today**: §6.4 makes **suspension** billing posture explicit (pause recurring vs continue
to charge) — but suspension always changes the **service** posture (entitlement freeze, OSS
pause). The inverse — **keep service running, pause collection** (hardship pauses, disputes,
goodwill) — has no representation.

**Vendor model**: Stripe `pause_collection` (subscription stays serviced; invoices
`keep_as_draft` / `mark_uncollectible` / `void`); Zuora handles it contractually. The
predecessor PRD's glossary likewise defines **Subscription Pause** ("temporary suspension
preserving subscription state; no charges during pause period") — a pause concept the
lifecycle PRD never modeled (it has only service-affecting suspension).

**Why it matters**: partners use collection pauses to keep customers alive during disputes
without service interruption; forcing a `suspended` state for it revokes entitlements — the
opposite intent.

**Options**: (a) a subscription **attribute posture** on `active` (mirror of the §6.4 pattern):
`collectionPaused` with an auditable window, consumed by Billing's recurring generation (skip /
draft per policy) — no new status (manifest enum untouched); (b) declare it a Billing-side
account-level hold, out of the lifecycle aggregate.

**Recommendation**: decide the owner first — (a) if the pause must ride subscription events and
renewal logic (likely, since §6.5 already pauses blocked next-term recurring — same mechanism),
(b) if Finance wants it as an AR-side hold. Either way, record it; today it is silently
unrepresentable.

**Decided — SUB-D-03 (2026-07-15):** option (a) — `collectionPaused` posture on `active`
(merged PRD §6.4 `fr-collection-pause` + AC 24), collection-scoped, auditable window;
pause-day limits and resume proration remain open (PRD §15). Veto-flagged — Finance may still
prefer the AR-hold shape.

### G-5. Committed multi-step schedules (ramps) and atomic multi-action orders

**State today**: future intent is expressible only as **one** pending boundary per concern
(scheduled `PlanLink` change, scheduled window on the catalog side). A committed multi-step
plan — "100 seats in Q1 → 200 in Q2 → 300 in Q3", or "phase into planB at date X then planC at
Y" — has no aggregate-level representation, and there is no **atomic multi-action** change (Zuora
Orders bundle several actions/subscriptions into one audited order).

**Vendor model**: Stripe **subscription schedules** (ordered phases with prices/quantities +
`end_behavior`); Zuora **Ramps** (committed ramp intervals with deal metrics) and **Orders**
(atomic multi-action).

**Why it matters**: negotiated growth deals (typical for ACP/enterprise channel) need the whole
ramp committed at signature for revenue planning (and ASC inputs), not re-keyed quarterly by an
operator.

**Options**: (a) defer to **Contracts** — a ramp is a contract term; Subscriptions executes each
step as a scheduled `changePlan`/`updateQuantity` generated from the contract (needs only the
G-1/G-3 envelopes + a Contracts follow-up); (b) native `SubscriptionSchedule` aggregate
(Stripe-style) inside this gear; (c) status quo (operator re-keys each step).

**Recommendation**: (a) — matches the SoR split (negotiated commitments already live in
Contracts, cf. rating T-D-14 pools) and keeps this gear's aggregate simple; record it as a
cross-PRD decision, not silence. (b) only if self-service ramps become a product goal.

**Decided — SUB-D-04 (2026-07-15):** option (a) — merged PRD §6.3 `fr-ramp-execution`
(Contracts authors; Subscriptions executes generated scheduled intents, riding SUB-D-01/02);
atomic multi-action submission = Contracts/Design follow-up (PRD §15).

### G-6. Activation date trio (booking / service / acceptance)

**State today**: `activate` is a single instant (Policy + OSS + entitlements). ASC 606 hooks are
"tags/snapshot refs" (PRD §5.1). There is no distinction between **booking** (contract
effective), **service activation**, and **customer acceptance** instants.

**Vendor model**: Zuora's triple — `ContractEffectiveDate` (booking/TCB), `ServiceActivationDate`
(service starts), `CustomerAcceptanceDate` (revenue-recognition trigger where acceptance
clauses apply); pending-activation / pending-acceptance interim states. Stripe has no analog
(SMB bias).

**Why it matters**: enterprise/channel deals with acceptance clauses need the three instants for
correct revenue timing downstream — exactly the "ASC inputs" this PRD promises to carry. A
single `activatedAt` collapses them.

**Options**: (a) record the trio as **subscription attributes/evaluated fields** stamped by the
existing flow (created/contract-signed → activate → optional acceptance confirmation), emitted
on events for Finance/Billing — no new statuses (manifest enum untouched); (b) extend the
manifest with interim statuses (pending-acceptance) — heavy, enum change; (c) declare booking
and acceptance dates a Contracts/Finance concern referenced by the subscription.

**Recommendation**: decision-first, default (a)+(c): attributes on the aggregate, SoR for
booking/acceptance semantics in Contracts/Finance, no enum change. Low effort once decided;
without it the ASC-hook promise is under-specified.

**Decided — SUB-D-05 (2026-07-15):** (a)+(c) — merged PRD §6.1 `fr-activation-instants`
(+ AC 25): `contractEffectiveAt` (Contract-referenced) / `serviceActivatedAt` (stamped at
activate) / `customerAcceptedAt` (acceptance confirmation where clauses apply); interim
statuses rejected; confirmation-flow shape → Design (PRD §15).

## 6. Recommendation summary

| # | Gap | Effort | Outcome |
|---|---|---|---|
| G-1 | Scheduled lifecycle intents (cancel at term end / at date; resume at date) | **Actioned** | SUB-D-01 → merged PRD §6.1 `fr-scheduled-intents` + AC 21–22 (2026-07-15) |
| G-2 | First-class early trial conversion (`convertTrial` phase-advance) | **Actioned** | Merged PRD §6.10 `fr-trial-early-conversion` + AC 17 (2026-07-15); manifest type-enum alignment tracked in PRD §15 |
| G-3 | Seat/quantity change transition (`updateQuantity`) | **Actioned** | SUB-D-02 → merged PRD §6.3 `fr-update-quantity` + AC 23; increases immediate / decreases next-cycle by default |
| G-4 | Billing-only pause (collection paused, service running) | **Decided** | SUB-D-03 → `collectionPaused` posture on `active` (PRD §6.4 + AC 24); mechanics open in PRD §15 |
| G-5 | Ramps / atomic multi-action orders | **Decided** (cross-PRD) | SUB-D-04 → Contracts authors, Subscriptions executes scheduled intents (PRD §6.3); Contracts follow-up in PRD §15 |
| G-6 | Activation date trio (booking/service/acceptance) | **Decided** | SUB-D-05 → attribute trio + acceptance confirmation (PRD §6.1 + AC 25); no enum change |

All six gaps are processed — G-1/G-2/G-3 actioned normatively, G-4/G-5/G-6 decided with
veto-flagged defaults ([DECISIONS.md](./DECISIONS.md) SUB-D-01…05). Everything in §2
(payment/collection/portal/metrics), §3 (HAVE), and §4 (recorded deferrals/splits) needs **no
action**. The local pricing-PRD proration-ownership wording was fixed alongside this analysis
(§1 note).
