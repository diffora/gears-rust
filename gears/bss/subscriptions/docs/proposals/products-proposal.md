---
description: "Product evidence and compatibility required by subscription acquisition, while trial policy remains in Pricing."
---

# Products proposal for Subscriptions

**Status:** Subscriptions integration proposal, 2026-10-15. This is an owner handoff, not a delivered API or external-owner approval. Examples are illustrative, not new tariff, duration or SLA defaults. See the [proposal index](README.md) for source precedence, scope exclusions and delivery order.

<!-- toc -->

- [Baseline and ownership](#baseline-and-ownership)
- [Required changes](#required-changes)
- [Examples](#examples)
- [Delivery and acceptance](#delivery-and-acceptance)

<!-- /toc -->

## Baseline and ownership

[Products](../../../products/docs/PRD.md) owns SKU identity, immutable versions, structure and applicable quantity/usage declarations. Pricing owns commercial trial terms. The latest [Rating input design](https://github.com/constructorfabric/gears-rust/blob/3a1cd8ac33bda1a72838a81435c7365a5d387d05/gears/bss/rating/docs/DESIGN.md#contract-11-4-2) (anchor valid at that commit; the 2026-10-16 re-inspection of #5175 is at `7afffbdd2b87188e67ecf0bcc06d306084c5be92`, see [rating-proposal.md](rating-proposal.md)) also requires typed historical reads of derived usage declarations; it describes existing SDK types/evaluators and a REST read, but no typed read trait at that source revision.

## Required changes

| Request | Required evidence or contract | Boundary |
| --- | --- | --- |
| PD-P01 Protected product evidence | Ratify how exact SKU/version/structure evidence participates in Pricing's future commitment and survives ordinary retirement | A catalog reference alone is not a customer future-service promise; Product evidence does not replace financial or capacity protection |
| PD-P02 Compatibility and limits | Expose versioned scope-key/quantity/structure policy evidence consumed through the agreed owner routes | Products does not count subscriptions; Orders cannot invent a replacement key from a current catalog lookup |
| PD-P03 Historical usage declarations | For enabled usage products, deliver the typed read and immutable declaration/digest requested by Rating; publish output magnitude and declared rounding limits | Rating evaluates derived usage; Subscriptions provides attribution and commercial scope, not the formula engine |
| PD-P04 Trial-free product model | Keep trial duration, grant amounts, repeat eligibility and paid-transition policy out of Products | Storefront joins Product descriptions with Pricing PlanRevision terms |

## Examples

**Product retirement after payment.** A future purchase references SKU version 7 under a durable commitment. Publishing version 8 or retiring ordinary new sales must not silently replace version 7 in the accepted purchase. The provider still needs actual fulfillment capability; catalog history alone does not promise resource availability.

**Two plans, one product.** The same SKU appears in a no-trial plan and a 14-day credit-trial plan. Their Pricing revisions differ; Products needs no `has_trial` flag and creates no grant.

**Derived usage replay.** Historical usage used a particular formula/output-rounding version. A later formula revision cannot change replay of that accepted period. Missing historical declaration leaves Rating pending instead of selecting today's formula.

## Delivery and acceptance

Agree PD-P01 with Pricing/AP and PD-P02 with Subscriptions/Orders; deliver PD-P03 with Rating only for the enabled usage profile. Test retirement after commitment, stale key/limit evidence and historical declaration recovery. Follow [the money ADR](https://github.com/constructorfabric/gears-rust/blob/510ce99f037a19f63d41b17acb3b0680e9331a44/gears/bss/docs/ADR/0001-cpt-cf-bss-adr-exact-decimal-boundaries.md) for published quantity forms and limits. No general catalog redesign, trial authoring or mandatory native resource provider is requested.
