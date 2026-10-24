---
description: "Pricing extensions for durable acquisition terms, immutable trial terms and required per-resource minimums, without reinterpreting existing V1 terms."
---

# Pricing proposal for Subscriptions

**Status:** Subscriptions integration proposal, updated 2026-10-23. This is an owner handoff, not a delivered API or external-owner approval. Examples are illustrative, not new tariff, duration or SLA defaults. See the [proposal index](README.md) for source precedence, scope exclusions and delivery order.

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
| PC-P07 Individual/group usage minimum | Support the required Jelastic cases: storage of one container, traffic of another, or storage pooled across containers in a node group. Distinguish eligible consumption from the individual/group target in immutable accepted policy. The user selected a monetary minimum applied after eligible usage valuation; preserve that amount/currency in accepted policy. Support hourly and billing-period minimum windows as alternative tariff settings, accepted on 2026-10-17. Preserve the full hourly minimum for a partial eligible hour, accepted on 2026-10-18, once per selected target/window. Use calendar-hour windows, accepted on 2026-10-18; service crossing an hour boundary receives the full minimum in each affected window. Support explicit full or proportional-to-time minimum treatment for partial billing periods, accepted on 2026-10-18; freeze the choice with the accepted minimum amount/scope/window. Define timezone/SDK mapping, stable identities, historical membership and eligible-duration evidence, lifecycle rules and cross-price eligibility before selecting the SDK extension. Individual/group scope is an accepted alternative tariff setting; preserve it in accepted terms and do not stack both minima on the same eligible usage. Amend D-504 validation/admission restrictions for supported combinations without reinterpreting old `minimum_fee` values (SUB-D-47, SUB-D-48, SUB-D-49, SUB-D-50, SUB-D-51, SUB-D-52, SUB-D-53, SUB-D-54) | Rating preserves target identity and historical membership through calculation; an individual floor cannot silently become a group floor. Exact types, membership validation and execution proof remain open |
| PC-P08 Continuation evidence | Expose a committed-read observation (snapshot/version) with `ResolvedBindings` so Subscriptions and Rating can prove which successor was in force at a binding's `ends_on`; report `PricesPublished` as a wake-up signal; accept a `report_terms_discrepancy` exchange. Automatic adoption of a successor still requires the explicitly accepted continuation policy (SUB-D-41) | Subscriptions publishes the authorized successor fact and exact seal from proven evidence; a closure or a returned successor alone changes no customer terms |

Current [`pricing/pricing/src/domain/usage_policy.rs::refuses_minimum_fee`](../../../pricing/pricing/src/domain/usage_policy.rs) rejects both `CalendarHour` and `AggregationScope::Resource`; `commercial_terms.rs` enforces that restriction during admission. PC-P07 is required provider work, not delivered capability or authorization to remove guards without the matching calculation/evidence contract. The existing pooled price/subscription/period profile remains valid for its original terms. The selected minimum is monetary and applies after eligible usage valuation; it does not impose a minimum billable quantity. Invoice-line grouping cannot select its scope. The user clarified container storage/traffic and node-group storage scopes on 2026-10-17; the monetary basis and tariff-selected hourly or billing-period windows are accepted, while the full minimum for a partial eligible hour was selected on 2026-10-18. Calendar-hour alignment was selected on 2026-10-18. Partial billing-period treatment is a tariff choice between full and proportional minimum, accepted on 2026-10-18. Timezone/SDK mapping, eligible-duration evidence and identities remain open; this adds no default and preserves existing accepted terms. The current `AggregationScope` has no resource-group variant. Historical membership and usage coverage need coordinated provider contracts; an inventory alone does not establish zero usage.

## Examples

**Catalog changes before start.** A protected EUR 20 purchase remains EUR 20 despite a later EUR 25 successor. The final permit refers to the existing commitment, rather than choosing the current offer. True permission expiry still prevents new application.

**Free trial with a minimum fee.** A plan normally has a EUR 10 minimum. The accepted free-period block must make covered trial charges zero, including that floor. Setting only the per-unit rate to zero would leave an unintended EUR 10 charge.

**Credit trial.** A plan grants EUR 100 usable for eligible items for 14 days. Ordinary prices remain intact. Starting at T fixes the original expiry from those terms; retries, delayed finalization and early conversion do not issue a new grant or restart 14 days. These figures are examples only.

## Acceptance and boundaries

Version the authoring/read/receipt/commitment/permit schemas and SDKs; specify supported dates, immutable references, release/revocation rules and provider failure outcomes. Execute AP and TP producer/consumer fixtures. Free-period enablement has no promotional-grant provider dependency; credit does. Extension/repeat trials, stacking, regrant, plan replacement and a Products trial flag remain excluded. Missing capability refuses the affected sale before protected capture.

## Owner document adoption

Adopt AP03/AP04 and TP in a separate Pricing PR across authoring/read/publication, accepted receipts, authorized historical replay, DESIGN/PRD and decisions. The current seven-method D-510 V1 and its existing trial exclusions remain unchanged until a separately versioned extension is adopted. Do not make this proposal look like a delivered V1 capability.

AP requires initial future acceptance as well as durable commitment and final permit: requiring V1 acceptance for a date V1 rejects would make the future flow impossible. Coordinate Products retirement and price closure before reporting commitment; partial preparation cannot authorize capture. Historical replay preserves original identity/expiry, and revocation requires acknowledged consumer fencing or expiry. TP additionally freezes trial duration, complete free-period fee behavior or credit applicability, and the accepted paid target; unsupported data must refuse rather than silently become an ordinary paid sale. Conversion to unchanged accepted terms reuses assent with fresh operation authority. Detailed evidence, recovery and conformance remain in AP and TP.
