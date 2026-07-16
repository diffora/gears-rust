<!-- CONFLUENCE_TITLE: [BSS]: Pricing — Stripe Billing Gap Analysis -->
<!-- Related: ./PRD.md, ./DESIGN.md, ./DECISIONS.md, ./design/ | Owners: BSS Product Catalog team -->

# Pricing — Stripe Billing Gap Analysis

<!-- toc -->

- [1. Purpose & method](#1-purpose--method)
- [2. Scope boundary — what Stripe puts on the customer that is NOT ours](#2-scope-boundary--what-stripe-puts-on-the-customer-that-is-not-ours)
- [3. Coverage map — Stripe pricing model vs what we have](#3-coverage-map--stripe-pricing-model-vs-what-we-have)
- [4. Already deferred (§17.8) with Stripe equivalents](#4-already-deferred-178-with-stripe-equivalents)
- [5. Gaps worth acting on](#5-gaps-worth-acting-on)
  - [G-1. Coupons / promotion codes (Promotions gear absent)](#g-1-coupons--promotion-codes-promotions-gear-absent)
  - [G-2. Prepaid grant is thinner than a Stripe credit grant](#g-2-prepaid-grant-is-thinner-than-a-stripe-credit-grant)
  - [G-3. Minimum commitment / committed spend (monetary true-up)](#g-3-minimum-commitment--committed-spend-monetary-true-up)
  - [G-4. Grant / discount / tax application order (joint contract)](#g-4-grant--discount--tax-application-order-joint-contract)
- [6. Recommendation summary](#6-recommendation-summary)

<!-- /toc -->

## 1. Purpose & method

A comparison of our Plan & Price Modeling design against **Stripe Billing** (the
`docs.stripe.com/billing/customer` surface plus the products/prices, subscriptions, credit
grants, and coupons models it links to), to answer: *what pricing models / schemas are we
missing?*

Method: each Stripe concept is bucketed as **HAVE** (already modelled), **DEFERRED**
(consciously in our Future-scope registry, [PRD.md §17.8](./PRD.md) (summary list at DESIGN.md L504)), **OTHER
GEAR** (correctly not a pricing-catalog concern), or **GAP** (worth a decision). Only the
GAP items (§5) are proposed work; everything else is recorded so the boundary is explicit.

Sources checked: [PRD.md](./PRD.md), [DESIGN.md](./DESIGN.md), all 12 slice designs under
[design/](./design/), [DECISIONS.md](./DECISIONS.md); plus the rating-gear docs for the
evaluation-side boundary — [rating PRD](../../rating/docs/PRD.md) (§17.1 step order, §17.2
coupon contract stub) and [rating design/06-coupons.md](../../rating/docs/design/06-coupons.md).

## 2. Scope boundary — what Stripe puts on the customer that is NOT ours

The Stripe `/billing/customer` page is mostly about the **customer object**, which in our
gear decomposition lives outside the pricing catalog. These are correctly **OTHER GEAR** —
listed here so the boundary is on the record, not to be pulled in:

| Stripe customer feature | Owner in our architecture |
|---|---|
| Customer balance / invoice credit balance (ad-hoc credit/debit adjustments) | Billing / Subscriptions |
| Tax IDs (store & render on invoice) | Billing / CRM (tax display basis is ours; the ID registry is not) |
| Invoice settings — numbering schemes, custom fields, footer, receipts | Billing |
| Default payment method | Payments |
| Smart Retries / dunning (automatic collection) | Billing / Subscriptions |
| Customer portal | Subscriptions / self-service UI |
| Pending invoice items / one-off invoices | Billing |
| Preferred locales, shipping address | CRM |
| Currency lock per customer | our invariant exists as **single-currency-per-invoice binding** ([04-currency-tax.md](./design/04-currency-tax.md)) |

Also correctly OTHER GEAR (evaluation / runtime, not catalog structure): graduated-vs-volume
**math**, override/stacking evaluation, FX steps, coupon application (fully designed: [rating design/06-coupons.md](../../rating/docs/design/06-coupons.md)) → **Tariffs/PLAL**;
subscription state machine, proration **math**, trial **runtime**, entitlement
**enforcement**, `PlanLink` execution → **Subscriptions**; balance ledger, drawdown, zero
cut-off, auto-recharge **execution** → **Billing/Rating**; negotiated per-account terms →
**Contracts**; bundle fee **accrual** → **Marketplace**.

## 3. Coverage map — Stripe pricing model vs what we have

**HAVE** — modelled today:

| Stripe pricing concept | Where we model it |
|---|---|
| Products & Prices | `Plan` / `Price` ([01-foundation.md](./design/01-foundation.md)) |
| Pricing models: per-unit, tiered (graduated + volume), package | explicit `modelKind`, `[fromQty,toQty)` tier bands, `package` ([03-price-structure.md](./design/03-price-structure.md)) |
| Usage-based / metered / meters | meters + meter injectivity, `billingGranularity`, `tierAggregationWindow` ([02](./design/02-plan-definition.md)/[03](./design/03-price-structure.md)) |
| Multi-currency prices (per-currency amounts) | per-`(currency, region)` price rows ([04-currency-tax.md](./design/04-currency-tax.md)) |
| Setup fee + recurring + one-time | `chargeKind = one_time_setup` first-class row ([02-plan-definition.md](./design/02-plan-definition.md)) |
| Billing cycles / custom frequency | billing cycles, custom frequency ([02-plan-definition.md](./design/02-plan-definition.md)) |
| Subscription schedules / phases (trial → intro → evergreen) | phases + `convertsToPhaseId` + `phaseDurationDays` + `displayTrialDays` ([02-plan-definition.md](./design/02-plan-definition.md)) |
| Features / entitlements | entitlement grant set, phase-scoped (D-41) ([06-consumer-contracts.md](./design/06-consumer-contracts.md)) |
| Tax (inclusive/exclusive, per region), automatic tax basis | tax-display basis + `not_sellable_ga` gate ([04-currency-tax.md](./design/04-currency-tax.md)) |
| Proration | proration input contract, canonical `prorationBasis` ([06-consumer-contracts.md](./design/06-consumer-contracts.md)) |
| Add-ons | add-on rules ([02-plan-definition.md](./design/02-plan-definition.md)) |
| Reserved / committed-rate (RI-style) | reserved-capacity attributes on the usage row ([10-advanced-primitives.md](./design/10-advanced-primitives.md)) |
| Package / transform-quantity | `package` model kind ([03-price-structure.md](./design/03-price-structure.md)) |
| Composite metering (e.g. VM = vCPU + RAM) | derived (composite) meter formula-as-data ([10-advanced-primitives.md](./design/10-advanced-primitives.md)) |
| Segment / partner / brand price books | `PriceOverlay` (adjustment overlays, precedence) ([09-price-overlays.md](./design/09-price-overlays.md)) |
| Bundles / marketplace composition | bundle basis + rev-share + itemization ([08-bundles.md](./design/08-bundles.md)) |
| Coupon **application** (order, stacking, FX split, snapshot pinning) | rating step-7 evaluator ([rating design/06-coupons.md](../../rating/docs/design/06-coupons.md); [rating PRD](../../rating/docs/PRD.md) §17.1 step 7, §17.2) — authoring/lifecycle is G-1 |

## 4. Already deferred (§17.8) with Stripe equivalents

These are **DEFERRED** — consciously recorded in [PRD.md §17.8](./PRD.md) (summary list at DESIGN.md L504). Not
gaps; captured here only to map them to their Stripe analogue so we don't "rediscover" them.

| Deferred item (§17.8) | Stripe analogue | Note |
|---|---|---|
| `includedAllowance` / `rolloverPolicy` | included usage in a price + credit rollover | Common SaaS shape ("500 units included, unused roll over"); deferred |
| authorable `aggregationFunction` / peak model kind | Stripe meter aggregation (`sum` \| `last` \| `max`) | We fix aggregation downstream today; not author-selectable |
| two-dimensional (seats × usage) single-line pricing | Stripe: two prices on one item | Deferred; today = two rows |
| structural freemium flag | Stripe free price / $0 tier | Expressible today as a `$0` row/tier; no first-class flag |
| `currencyFallbackPolicy` (FX fallback) | Stripe multi-currency / presentment | We are deliberately **fail-closed, no implicit FX** ([PRD.md](./PRD.md) §L524) |
| typed credit/discount (negative-amount) rows | Stripe negative invoice items / credits | Deferred; discounts go through the `discountRef` hook instead |
| per-row `refundable` / `creditPolicy` | Stripe refunds / credit notes | Billing-side; deferred here |
| self-service term / auto-renew metadata | Stripe subscription settings | Subscriptions-side; deferred here |
| per-group different-tier structures | Stripe: separate price per segment | Committed to adjustment-only overlays (F-88); different structure = separate plan |
| plan-level minimum fee / cap per period; committed-usage / drawdown flags on plan | Stripe committed spend / spending minimums on subscriptions | §17.8 rows the original pass missed (it read the shorter DESIGN.md L504 summary). Floor machinery already reserved rating-side (`PeriodFloorCapObligation`, Billing executes); catalog authoring field = the deferred part. Negotiated committed spend = Contracts SoR pools (rating T-D-14) — see G-3 |

## 5. Gaps worth acting on

Ranked. Each is a real delta against Stripe that is **not** already deferred in §17.8 and is
either a catalog-side concern or a missing/unpinned **cross-gear contract**.

### G-1. Coupons / promotion codes (Promotions gear absent)

**State today — two sides; only one is missing.**

*Application side (rating gear) — designed, not a gap.* Coupons are the **step-7 evaluator**
in the compiled §17.1 chain ([rating design/06-coupons.md](../../rating/docs/design/06-coupons.md)).
The frozen `CouponSnapshot` contract ([rating PRD](../../rating/docs/PRD.md) §17.2, the
Tariffs-side stub) already carries the structure a coupon needs: `adjustmentType`
(`percent | fixed_amount`), `value`, `settlementCurrency` (`price | billing`, with the
compiled FX split), `applyScope` (`usage | recurring | line_total` + hybrid split-back),
`applyPerTierBand`, stacking (`exclusive_best | ordered_stack`), validity, applicability
filters, redemption eligibility. Reproducibility is solved on that side too: applied ids +
stacking policy are the Tariffs-written **S1 segment of `pricingSnapshotRef`**, and replay
uses the pinned snapshot (T-D-04) — never live campaign state. Catalog-side `discountRef`
stays a referential-integrity hook **by design** ([PRD.md §L412](./PRD.md): neither coupon
authoring nor evaluation is a catalog concern).

*Authoring/lifecycle side — the actual gap.* The **Promotions gear does not exist** (no PRD):
coupon creation, campaigns, customer-facing promotion codes, redemption limits/counting, and
fraud controls have no owner. This is a **known, tracked open**, not a discovery — rating PRD
§15/§16 + risk register ("align field names with §17.2 before production coupon rating");
pricing PRD §15 ("dedicated PRD TBD") + risk register.

**Stripe model** (two first-class objects) — kept as the field checklist for the future
Promotions PRD, mapped against the §17.2 stub:
- **Coupon**: `percent_off | amount_off` (per-currency) → `adjustmentType` + `value` ✓;
  `applies_to.products` → applicability filters ✓; `max_redemptions` / `redeem_by` →
  validity + redemption eligibility (the counting itself is Promotions lifecycle) ✓;
  **`duration`** (`once | repeating(duration_in_months=N) | forever`) → **no explicit §17.2
  counterpart** — the one real field-shape delta: "−20% for the first 3 billing cycles after
  redemption" must be provably expressible in the snapshot's validity/eligibility shape, or
  needs an explicit field.
- **Promotion code**: a customer-facing code wrapping a coupon (`first_time_transaction`,
  `minimum_amount` + currency, `expires_at`, per-customer restriction) → purely
  Promotions-side authoring; correctly absent from both catalog and rating.

**What actually matters**: (1) end-to-end, no coupon can exist until Promotions stands up —
but its consumption contract is already written and fail-closed (§17.2), so the Promotions
PRD starts from a fixed target, not a blank page; (2) the Stripe `duration` shape belongs on
the already-open rating §15 Promotions-alignment agenda, next to the equal-benefit tie-break
and mixed-settlement opens.

**Options**:
- (a) Stand up the **Promotions PRD/gear** (the durable owner) — the target; this checklist
  plus the §17.2 stub are its inputs.
- (b) Upgrade `discountRef` to a typed descriptor the catalog freezes — **rejected**: coupons
  flow Promotions → frozen snapshot → rating step 7, bypassing the catalog; a typed catalog
  descriptor would open a second authoring path across the settled boundary
  ([PRD.md §L412](./PRD.md)) and duplicate the §17.2 schema with guaranteed drift.
- (c) Status quo until Promotions lands — the acceptable interim, precisely because the
  evaluation side is snapshot-shaped and fail-closed; the interim work is contract alignment
  (§15), not catalog schema.

**Recommendation**: (a) when prioritized; meanwhile add Stripe `duration` and the promo-code
wrapper fields to the rating §15 alignment agenda. Do **not** type `discountRef`.

### G-2. Prepaid grant is thinner than a Stripe credit grant

**State today**: the prepaid grant ([10-advanced-primitives.md L174–182](./design/10-advanced-primitives.md))
carries `grantAmount`, `creditUnit` (currency **or** published `meteringUnit`), `expiryPolicy`,
`autoRechargeAllowed`, and a per-`(currency, region)` `price`. Balance/drawdown are correctly
GA-gated to Billing/Rating.

**Stripe credit grant** additionally has three **definitional** fields (i.e. authoring
metadata, **not** "execution" that we can defer to Billing):

| Stripe field | We have? | Why it's a catalog concern |
|---|---|---|
| **`applicability_config`** (which metered prices/meters the grant is spendable on) | ✗ | Can't express "this wallet applies to egress but not compute". This is *structure*, authored at definition time — not balance. |
| **`category`** (`prepaid` \| `promotional`) | ✗ (always paid — `price` required) | A **free promotional** grant is unrepresentable; our grant always has a purchase price. |
| **`priority`** (drawdown order across multiple grants) | ✗ | Stripe puts it on the grant object; a customer can hold multiple grants (Stripe caps at 100). Arguably catalog-authored default; possibly Billing-owned. |

Stripe also fixes ineligibility (credits never apply to licensed prices, one-time/setup rows,
one-off invoices) — a rule set our applicability scoping would need to state.

**Recommendation**: add `applicability` (scope set over meters/prices) and `category` to the
grant definition; decide `priority` ownership (catalog default vs Billing). Small, additive
schema change to `pricing_plan.prepaid_grant` + `GrantValidator`.

**Actioned — D-43 (2026-07-14, flagged for veto)**: `category` (`prepaid | promotional`), a
**materialized** usage-line `applicability`, and `drawdownPriority` as an authored default
added to the grant definition ([DECISIONS.md](./DECISIONS.md) D-43); the **effective**
cross-grant order is Billing-owned via a normative tie-break chain (`drawdownPriority` →
`promotional` first → earlier expiry → earlier issuance → `grantId`). Rating-side check
passed: the wallet grant is disjoint from step-6 `commitmentPools[]` (rating SEAMS M8), so
this stays a catalog+Billing concern; where drawdown sits relative to tax remains G-4.

### G-3. Minimum commitment / committed spend (monetary true-up)

**State today — resolved by inspection; the ownership question is already answered.** The
original pass worked from the DESIGN.md L504 summary list (which omitted the relevant rows)
and never opened the rating docs. The monetary constructs exist, have owners, and the rating
PRD **forbids conflating them** (hybrid rule, [rating PRD](../../rating/docs/PRD.md) §6.2);
the only thing we author catalog-side today is the quantity floor (`minQtyThreshold`), which
is unrelated:

- **Negotiated committed spend with monetary true-up** ("pay at least \$X/month, true up the
  shortfall") — owned end-to-end: commitment pools are **Contracts SoR**
  (`commitmentPools[]`, evaluated at step 6), each with a frozen
  `poolType ∈ {prepaid_drawdown, committed_rate}` (rating **T-D-14**), and the spend-basis
  shortfall is a **normative formula** — `max(0, committedSpend − inCommitBilledAmount)` —
  emitted as a structured `TrueUpObligation` for Billing
  ([rating design/05](../../rating/docs/design/05-commitments-reservations.md) §4.5). This
  **is** Stripe's committed spend on subscriptions; the amount is a Contracts term by
  design, not a catalog field.
- **Minimum invoice fee per period (period floor)** — the evaluation boundary is already
  reserved: Tariffs sets amount, comparison basis, and attachment scope and emits
  `PeriodFloorCapObligation` (amount, basis, period, **contract/plan ref**); **Billing
  executes** `max(total, floor)` after step 9 ([rating PRD](../../rating/docs/PRD.md)
  `fr-period-floor-cap-obligation`). The **catalog authoring field** is a conscious
  deferral — [PRD.md](./PRD.md) §17.8 "Minimum fee / cap per period on plan" (`p2`,
  Follow-on) — i.e. §4 territory, which the original pass misfiled as a gap.

**Resolution — no new decision to record**: ownership is split and settled — Contracts for
negotiated commitments; the plan-level floor is deferred with its machinery reserved (the
obligation already anticipates a **plan** ref). When a self-service plan needs "\$X/month
minimum", the §17.8 row activates as an additive plan-level floor amount per
`(currency, region)`, frozen in the snapshot, feeding the existing obligation — no new
evaluation machinery. Actioned here: the stale §17.8 note ("Tariffs Future scope") updated
to cite the designed rating boundary; the item added to §4 with its Stripe analogue; the
DESIGN.md L504 summary list extended with the two omitted money-shaped rows.

### G-4. Grant / discount / tax application order (joint contract)

**State today**: the **within-rating** order is compiled and normative —
[rating PRD](../../rating/docs/PRD.md) §17.1 pins base row → overlays (step 4) → contract (5)
→ commitment (6) → coupon in price currency (7) → FX + billing-currency coupons (8) → emit
(9), with the period floor/cap applied after step 9 by Billing. What no document pins is the
**Billing-side continuation**: where **prepaid-grant drawdown** (GA-gated Billing/Rating
execution, outside steps 1–9) and **tax** sit relative to the post-coupon amount. Rating
explicitly declines to own it — "Tariffs supplies lineage, not the gross-vs-net ordering
decision" ([rating design/06-coupons.md](../../rating/docs/design/06-coupons.md) §4.5): the
lineage keeps either answer computable, but nothing selects the answer.

**Stripe** fixes it explicitly: credits apply **after discounts, before taxes** (and before
the invoice credit balance). We need the same one-line normative statement on the
**Billing-facing joint contract** (no Billing gear doc exists yet to hold it — carry it next
to the grant definition in [10-advanced-primitives.md](./design/10-advanced-primitives.md)
and mirror it in the rating consumer contracts until Billing docs exist). Low effort, high
clarity.

## 6. Recommendation summary

| # | Gap | Effort | Recommended action |
|---|---|---|---|
| G-1 | Coupons / promotion codes | Gear PRD (known open) | Application side already designed (rating 06 / §17.2); stand up Promotions PRD from that contract; put Stripe `duration` on the §15 alignment agenda; typed-`discountRef` bridge rejected |
| G-2 | Prepaid grant `applicability` + `category` (+ `priority`) | Low–Med | **Actioned → D-43** (fields added + materialized at publish; effective order Billing-owned; veto-flagged) |
| G-3 | Monetary minimum commitment / true-up | — (resolved by inspection) | Already owned: negotiated committed spend = Contracts pools + `TrueUpObligation` (rating T-D-14, step 6); plan-level period floor = conscious §17.8 deferral with `PeriodFloorCapObligation` machinery reserved; no new decision needed |
| G-4 | Grant-drawdown / tax placement after coupons | Low | Within-rating order already pinned (§17.1); pin the Billing-side continuation (drawdown, tax) as a normative joint-contract line |

Everything in §2 (customer object), §3 (HAVE), and §4 (§17.8 deferred) needs **no action** —
recorded to make the boundary and the conscious deferrals explicit.
