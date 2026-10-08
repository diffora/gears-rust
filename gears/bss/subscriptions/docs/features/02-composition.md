---
description: "Immutable subscription composition, accepted commercial terms, quantity lots and effective intervals."
---

# Feature: Composition and Accepted-Term History


<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Bind accepted composition](#bind-accepted-composition)
  - [Read as of service time](#read-as-of-service-time)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [Interval and identity discipline](#interval-and-identity-discipline)
  - [Resolve continued bindings](#resolve-continued-bindings)
  - [Descriptor and market continuity](#descriptor-and-market-continuity)
- [4. States (CDSL)](#4-states-cdsl)
  - [Composition intent state](#composition-intent-state)
- [5. Definitions of Done](#5-definitions-of-done)
  - [Accepted composition history](#accepted-composition-history)
- [6. Acceptance Criteria](#6-acceptance-criteria)

<!-- /toc -->

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-featstatus-composition-implemented`
- [ ] `p1` - `cpt-cf-bss-subscriptions-feature-composition`

**Status:** proposed consumer design; current Pricing provider semantics are authoritative.
**Current-profile boundary, 2026-10-15:** apply the [current delivery scope](../PRD.md). Audit subsystem, confirmAcceptance, ramps, co-term, collection pause, notifications, tenant/payer transfer, administrative backdating/correction and optional brand/PlanTier metadata are deferred. Earlier references and scenarios involving those capabilities are future context only, not current acceptance or implementation tasks. Retain authorization, operation/event provenance, reliable effects and current facts/replay.

**Additional current-profile boundary, 2026-10-15:** the [additional scope reduction](../PRD.md#11-purpose) defers trial extension/repeat, acquisition without Orders, scheduled composition/resume, linked subscription replacement and base-plan changes. Retain supported immediate increases/additions, existing lifecycle, future order starts/cancellation and ordinary timers, both trial variants and early conversion to the accepted paid target. Deferred paths are refused even with explicit policy or operator authorization; their earlier examples are future context, not launch gates.

## 1. Feature Context

### 1.1 Overview

Preserve what was accepted and supplied, including quantities, tenant context and immutable Pricing evidence, so replay does not consult a current catalog to reconstruct a past charge.

### 1.2 Purpose

**Requirements:** `cpt-cf-bss-subscriptions-fr-monotonic-version`, `cpt-cf-bss-subscriptions-fr-effective-dated-composition`, `cpt-cf-bss-subscriptions-fr-snapshot-discipline`.

**Deferred references, not current requirements:** `cpt-cf-bss-subscriptions-fr-plantier-derivability`, `cpt-cf-bss-subscriptions-fr-sale-brand-attribution`.

**Principle:** `cpt-cf-bss-subscriptions-principle-effective-dated-composition`.

### 1.3 Actors

| Actor | Role |
| --- | --- |
| `cpt-cf-bss-subscriptions-actor-pricing` | Supplies immutable accepted binding and policy descriptors. |
| `cpt-cf-bss-subscriptions-actor-rating` | Consumes committed term and quantity slices. |
| `cpt-cf-bss-subscriptions-actor-platform-operator` | Reads history and proposed/applied differences. |

### 1.4 References

- **PRD:** [PRD.md](../PRD.md); **Design:** [domain model](../DESIGN.md#31-domain-model).
- **Build prerequisites:** [foundation](./01-foundation.md), as allocated in [DECOMPOSITION.md](../DECOMPOSITION.md).
- **Provider prerequisites / integration acceptance:** Pricing D-420/D-425/D-502/D-510 and accepted Products descriptors. Consumer conformance with commercial facts and recovery is an integration acceptance step, not an additional feature build dependency.
- **Migration:** SUB-D-29 replaces CatalogVersion/cohort/phase state; old names are compatibility references only.

## 2. Actor Flows (CDSL)

### Bind accepted composition

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-flow-open-interval-cmp`

1. Authorize the caller and validate requested line/item identity against the accepted order/contract context.
2. Resolve the immutable acceptance/hold evidence using the current Pricing capability owners. Validate receipt/digest, complete selections, tenant axes, market, BillingTerms and policy descriptors. Do not substitute a fresh preview when evidence is unavailable.
3. Freeze the accepted bindings with stable line/item IDs, service quantity source, dated SKU descriptors, currency and region/market, and immutable source provenance. Brand/PlanTier metadata is deferred. Pricing descriptor/money digest and UsageRatingPolicy identity/version/digest remain intact.
4. At application, close/open composition and quantity intervals at the one committed boundary. Increment aggregate revision once and produce downstream frozen term slices from that revision.
5. Retain predecessor evidence and correction lineage. Scheduled composition changes are deferred and refuse before effects. Future-start Orders remain in the separate Foundation acquisition/cancellation flow; they create no served interval before activation.

A confirmed future-start order additionally retains its durable Pricing/Products commercial commitment, order-bound reservation and financial correlation separately from applied service intervals. That commitment must exist before capture under the Foundation's versioned contract; current V1 acceptance/hold evidence alone does not establish it. Confirmation or prepayment does not start service coverage. Cancelling or amending a paid future-start order cannot silently discard that commitment, reprice the customer or release its place; use the agreed amendment/cancellation and financial-remedy path. Draft/version/attempt rebinding remains gated under S-U01.

### Read as of service time

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-flow-composition-asof`

1. Authorize the requested scope and locate the committed revision/interval at the requested instant or range.
2. Return applied composition, total quantity and complete per-interval lot allocations with stable identities, quantities, accepted terms and purchase provenance, plus effective tenant context and revision/continuation context. Do not expose scheduled composition as a supported operation; never collapse different accepted lot terms into a single current price.
3. Resolve every term boundary within the requested range, including quantity, payer, suspension and binding `ends_on` changes. Return explicit absence or inconsistency instead of filling a gap with today's plan.
4. For a corrected view, expose superseded and current version references so financial consumers can reproduce the original evidence and the correction independently.

## 3. Processes / Business Logic (CDSL)

### Interval and identity discipline

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-algo-composition-intervals`

Applied intervals are half-open `[effectiveFrom, effectiveTo)`. Adjacent intervals meet exactly; one logical item has at most one applied state at an instant. A quantity change preserves logical item identity and adds quantity history. Replace-price or renewal version changes do not mint another economic occurrence. A new optional item obtains its own stable item ID. Every correction retains its predecessor; no update/delete of consumed historical evidence is permitted. Future plans are replaceable through audited intent changes, not through destructive edits to applied history.

**Quantity-lot model accepted, 2026-10-14.** One logical component retains a separately identified lot for each purchase, with accepted binding, quantity, purchase provenance and effective-dated history. Its single applied state contains the complete lot allocation; service quantity is the sum of the effective lots. entitlement provider consumes the resulting capacity. Rating consumes the full allocation in a versioned commercial fact, and corrections retain lot identities and prior evidence. Replaying a purchase business operation does not add another lot. Each lot follows Pricing's accepted renewal/continuation rules; no permanent price protection is inferred. Latest-purchase-first reduction is retained for a future phase only; decreases/removals and transfers between purchase lots are outside the initial increase/addition profile. Metered-usage allocation remains a separate contract question, as recorded in [DECISIONS](../DECISIONS.md#open-cross-owner-decisions).

**Different-binding quantity increases — S-U08 delivery gate.** A new purchase of quantity on an existing logical item may accept a binding different from the existing quantity. One temporal binding plus a total quantity cannot represent both concurrently. The lot-aware temporal-slice/full-replacement semantics are accepted in [feature 08 §3.3](./08-commercial-facts.md#33-derive-stable-facts-and-complete-accepted-term-slices). Until its exact versioned provider/consumer contract and runtime evidence exist, refuse that increase before effects; same-binding increases and distinct logical add-on items remain supported under their existing guards. The required contract must preserve immutable lot identity, accepted binding and purchase provenance, lot quantities whose sum equals service quantity, and the original terms of existing lots. For the initial profile it must encode increases/additions and correction allocation with version lineage; the future reduction algorithm is not a launch prerequisite. Wire schema and owner integration remain open. No resource-to-lot reassignment API is introduced.

**Future-phase quantity reduction; rule accepted, scope deferred on 2026-10-14.** Initial-profile requests refuse before effects. If this capability is explicitly enabled in a later phase, in a supported authorized decrease, select the most recently acquired lots first, independent of their prices. Freeze the proposed per-lot reductions against the observed composition revision and check contractual minima, commitments and capacity guards before application. A prohibited selection refuses without partial change; expose the reason and a permitted alternative where established, but never silently skip a protected lot and reduce another. Reductions default to the next billing cycle unless accepted terms prescribe another supported boundary. Apply the selected quantities atomically with append-only history and full replacement facts; replay retains the same operation and allocation. A changed composition requires ordinary revision/revalidation handling, not silent reselection of different lots. Commercial lot selection does not select which users lose licenses; entitlement provider/enforcement retains occupied-capacity and revocation authority.

For ten original units at EUR 10 and two later units at EUR 12, a permitted reduction from twelve to nine removes the later two and one original unit. Nine original units remain, valued by Rating at EUR 90 for a full period under those unchanged terms. If a commitment prevents removing the later lot, refuse this selection rather than silently remove three original units. Prices are illustrative, not the sort key.

Component/item lifetime identity is distinct from S-Q04 charge-occurrence identity. Opening another interval, re-adding a component or replacing its price does not authorize a setup fee. Only a separately agreed chargeable business action can qualify under its accepted terms; retry or an attempt/draft rebuild preserves that same action. Existing order-change profile restrictions and S-U01 rebinding gates remain unchanged.

### Resolve continued bindings

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-algo-pricebook-bindings`

At renewal or an allowed usage binding boundary, request Pricing resolution using the retained pin and the described commercial date. Follow the provider's successor rules; stop before a successor marked new when the pin contract requires it. Slice at the binding's own `ends_on`; informational `effective_to` may reflect a successor the holder did not take and is not a service/price cutoff. Retain keep-for-bound predecessors and immutable policy descriptors. Absence of required historical evidence blocks the affected fact; it cannot trigger current-price fallback.

Usage can bind lazily per supported dimension value under the accepted policy, but value discovery must not change period identity or invent a new policy version. The resulting frozen slice must identify the actual accepted/resolved money and policy evidence. Unsupported or mismatched descriptors yield a typed failure before the slice is published.

### Descriptor and market continuity

- [ ] `p2` - **ID**: `cpt-cf-bss-subscriptions-algo-composition-descriptors`

**Deferred:** PlanTier derivation and per-sale brand retention are future metadata capabilities, not current provider prerequisites or acceptance criteria. Retain the artifact identity for traceability only. Immutable accepted price, policy and required invoice-descriptor provenance remains mandatory; missing required historical evidence blocks replay rather than synthesizing legacy CatalogVersion or substituting live data. Currency/market changes require the explicitly supported commercial transition path; legacy cancel+new constraints are retained until accepted compatibility rules permit otherwise.

## 4. States (CDSL)

### Composition intent state

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-state-composition`

The initial profile applies supported immediate composition changes under operation receipts; it does not expose a scheduled composition state. **Deferred historical model:** `scheduled`, `applied`, `cancelled` and `failed` are future composition-intent dispositions, not subscription statuses. Applied evidence remains immutable; a successor revision can supersede its interpretation for a correction without changing the original. A past interval never returns to proposed state.

## 5. Definitions of Done

### Accepted composition history

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-dod-composition`

The implementation must persist accepted evidence, monotonic revisions, stable component identity and half-open quantity/ownership history; provide repeatable as-of reads; prevent overlapping applied states; and freeze complete term slices for Rating. Every adapter field must trace to current provider evidence or an explicit open contract gate.

**Implements:** `cpt-cf-bss-subscriptions-flow-open-interval-cmp`, `cpt-cf-bss-subscriptions-flow-composition-asof`, `cpt-cf-bss-subscriptions-algo-pricebook-bindings`.

## 6. Acceptance Criteria

- [ ] Replaying a past period after price/product changes returns byte-equivalent frozen evidence and the original quantity intervals.
- [ ] A new successor that the retained pin does not take never causes a split at `effective_to`; the holder splits at its binding's `ends_on`.
- [ ] A scheduled composition request refuses before effects. Cancelling a future-start order uses Foundation recovery and preserves any protected claim, commitment and financial obligations until their authorized settlement; it creates no served interval.
- [ ] Adjacent quantity and optional-item boundaries have neither double coverage nor gaps in the served extent.
- [ ] A missing immutable price/policy descriptor blocks the fact with an actionable finding; live catalog data is not substituted.
- [ ] A corrected version remains linked to the superseded version, and both can reproduce their respective financial evidence.

- [ ] Existing quantity 10 at its old accepted binding plus a requested increase of 2 at a different new binding refuses under the S-U08 lot gate, rather than repricing all 12 or dropping the delta price. Once that contract is enabled, the required fixture preserves 10 at the old terms and 2 at the new terms, with total service quantity 12 and replayable lot provenance.

- [ ] The initial profile refuses a reduction from twelve to nine and transfers between commercial lots before effects. **Future-phase criterion, not a launch requirement:** reducing ten original units plus two later units to nine removes two later units and one original unit at the accepted boundary, regardless of relative prices. A commitment prohibiting that selection refuses without partial change or silent substitution; replay and historical reads preserve the selected lot lineage.
