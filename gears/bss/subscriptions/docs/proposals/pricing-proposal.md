---
description: "Pricing extensions for durable acquisition terms and immutable trial terms, without changing existing V1 semantics."
---

# Pricing proposal for Subscriptions

**Status:** Subscriptions integration proposal, 2026-10-15. This is an owner handoff, not a delivered API or external-owner approval. Examples are illustrative, not new tariff, duration or SLA defaults. See the [proposal index](README.md) for source precedence, scope exclusions and delivery order.

<!-- toc -->

- [Baseline and retained behavior](#baseline-and-retained-behavior)
- [Required changes](#required-changes)
- [Examples](#examples)
- [Acceptance and boundaries](#acceptance-and-boundaries)
- [Owner document adoption](#owner-document-adoption)

<!-- /toc -->

## Baseline and retained behavior

The local [Pricing design](../../../pricing/docs/DESIGN.md) supplies PriceBook reads and existing V1 acceptance/sellability methods. [AP](../ACTIVATION_PROPOSAL.md#13-orders-and-pricing-reciprocal-activation-contract) records their exact limitations: a future date outside current hold coverage cannot be accepted by silently extending an old receipt. Existing Plan/PlanRevision remains the storefront identity; no separate TrialOffer catalog is required.

## Required changes

| Request | Required owner work | Consumer consequence |
| --- | --- | --- |
| PC-P01 Future acceptance | Add a versioned initial acceptance and durable commitment for exact original terms and an accepted future window, coordinated with Products evidence | Orders can protect the purchase before capture, including a date outside V1 hold coverage |
| PC-P02 Final permission | Provide separately bounded activation authority with operation/attempt/consumer binding and agreed clock/withdrawal fencing | Subscriptions consumes it at actual local application; a cached sellability check is not permission |
| PC-P03 Trial authoring | Add an optional immutable free-period OR credit block to PlanRevision, including duration, applicability and already accepted paid target | The storefront displays the plan's terms; customer acceptance and historical reads preserve the selected revision |
| PC-P04 Free-period coverage | Define explicit zero bindings for covered charges, including minimum-fee behavior | A zero unit rate cannot accidentally produce a nonzero minimum fee |
| PC-P05 Credit terms | Preserve ordinary tariffs; publish grant amount/currency/scope/expiry terms and paid-transition restrictions | Billing supplies one promotional grant; Pricing neither owns the balance nor creates a funded wallet |
| PC-P06 Exact numeric evidence | Apply [the money ADR](https://github.com/constructorfabric/gears-rust/blob/510ce99f037a19f63d41b17acb3b0680e9331a44/gears/bss/docs/ADR/0001-cpt-cf-bss-adr-exact-decimal-boundaries.md) to prices, fees, quantities and trial-credit terms; publish field limits/currency/scale | Consumers retain exact values and reject unsupported precision; no minor-unit reinterpretation or implicit rounding |

## Examples

**Catalog changes before start.** A protected EUR 20 purchase remains EUR 20 despite a later EUR 25 successor. The final permit refers to the existing commitment, rather than choosing the current offer. True permission expiry still prevents new application.

**Free trial with a minimum fee.** A plan normally has a EUR 10 minimum. The accepted free-period block must make covered trial charges zero, including that floor. Setting only the per-unit rate to zero would leave an unintended EUR 10 charge.

**Credit trial.** A plan grants EUR 100 usable for eligible items for 14 days. Ordinary prices remain intact. Starting at T fixes the original expiry from those terms; retries, delayed finalization and early conversion do not issue a new grant or restart 14 days. These figures are examples only.

## Acceptance and boundaries

Version the authoring/read/receipt/commitment/permit schemas and SDKs; specify supported dates, immutable references, release/revocation rules and provider failure outcomes. Execute AP and TP producer/consumer fixtures. Free-period enablement has no promotional-grant provider dependency; credit does. Extension/repeat trials, stacking, regrant, plan replacement and a Products trial flag remain excluded. Missing capability refuses the affected sale before protected capture.

## Owner document adoption

Adopt AP03/AP04 and TP in a separate Pricing PR across authoring/read/publication, accepted receipts, authorized historical replay, DESIGN/PRD and decisions. The current seven-method D-510 V1 and its existing trial exclusions remain unchanged until a separately versioned extension is adopted. Do not make this proposal look like a delivered V1 capability.

AP requires initial future acceptance as well as durable commitment and final permit: requiring V1 acceptance for a date V1 rejects would make the future flow impossible. Coordinate Products retirement and price closure before reporting commitment; partial preparation cannot authorize capture. Historical replay preserves original identity/expiry, and revocation requires acknowledged consumer fencing or expiry. TP additionally freezes trial duration, complete free-period fee behavior or credit applicability, and the accepted paid target; unsupported data must refuse rather than silently become an ordinary paid sale. Conversion to unchanged accepted terms reuses assent with fresh operation authority. Detailed evidence, recovery and conformance remain in AP and TP.
