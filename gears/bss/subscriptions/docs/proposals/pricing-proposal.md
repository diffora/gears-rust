---
description: "Pricing extensions for durable acquisition terms, immutable trial terms and required per-resource minimums, without reinterpreting existing V1 terms."
---

# Pricing proposal for Subscriptions

**Status:** Required integration contract; owner agreement and executed conformance are required before enablement. See the [proposal index](README.md) for scope and common rules.

<!-- toc -->

- [Baseline and retained behavior](#baseline-and-retained-behavior)
- [Required changes](#required-changes)
- [Examples](#examples)
- [Acceptance and boundaries](#acceptance-and-boundaries)

<!-- /toc -->

## Baseline and retained behavior

The local [Pricing design](../../../pricing/docs/DESIGN.md) supplies PriceBook reads and existing V1 acceptance/sellability methods. [AP](../ACTIVATION_PROPOSAL.md#13-orders-and-pricing-reciprocal-activation-contract) records their exact limitations: a future date outside current hold coverage cannot be accepted by silently extending an old receipt. Existing Plan/PlanRevision remains the storefront identity; no separate TrialOffer catalog is required.

## Required changes

| Request | Required owner work | Consumer consequence |
| --- | --- | --- |
| PC-P01 Future acceptance | Add a versioned initial acceptance and durable commitment for exact original terms and an accepted future window, coordinated with Products evidence | Orders can protect the purchase before capture, including a date outside V1 hold coverage |
| PC-P02 Accepted purchase recovery | Expose immutable accepted acquisition, complete original terms, durable future coverage and any explicit execution restrictions through authorized read/replay | Subscriptions executes the accepted purchase with its current operational guards; catalog changes after acceptance cannot veto or reprice it |
| PC-P03 Trial authoring | Add an optional immutable free-period OR credit block to PlanRevision, including duration, applicability and already accepted paid target | The storefront displays the plan's terms; customer acceptance and historical reads preserve the selected revision |
| PC-P04 Free-period coverage | Define explicit zero bindings for covered charges, including minimum-fee behavior | A zero unit rate cannot accidentally produce a nonzero minimum fee |
| PC-P05 Credit terms | Preserve ordinary tariffs; publish grant amount/currency/scope/expiry terms and paid-transition restrictions | Billing supplies one promotional grant; Pricing neither owns the balance nor creates a funded wallet |
| PC-P06 Exact numeric evidence | Apply [the money ADR](../../../docs/ADR/0001-cpt-cf-bss-adr-exact-decimal-boundaries.md) to prices, fees, quantities and trial-credit terms; publish field limits/currency/scale | Consumers retain exact values and reject unsupported precision; no minor-unit reinterpretation or implicit rounding |
| PC-P07 Individual/group usage minimum | Support the required Jelastic cases: storage of one container, traffic of another, or storage pooled across containers in a node group. Distinguish eligible consumption from the individual/group target in immutable accepted policy. The minimum is monetary and applies after eligible usage valuation. Preserve that amount/currency in accepted policy. Support hourly and billing-period minimum windows as alternative tariff settings. Preserve the full hourly minimum for a partial eligible hour, once per selected target/window. Use calendar-hour windows; service crossing an hour boundary receives the full minimum in each affected window. Support explicit full or proportional-to-time minimum treatment for partial billing periods; freeze the choice with the accepted minimum amount/scope/window. Define timezone/SDK mapping, stable identities, historical membership and eligible-duration evidence, lifecycle rules and cross-price eligibility before selecting the SDK extension. Individual and group scope are alternative tariff settings. Preserve the selected scope in accepted terms; do not stack both minima on the same eligible usage. Amend D-504 validation/admission restrictions for supported combinations without reinterpreting old `minimum_fee` values (SUB-D-47, SUB-D-48, SUB-D-49, SUB-D-50, SUB-D-51, SUB-D-52, SUB-D-53, SUB-D-54) | Rating preserves target identity and historical membership through calculation; an individual floor cannot silently become a group floor. Exact types, membership validation and execution proof remain open |
| PC-P08 Continuation evidence | Expose a committed-read observation (snapshot/version) with `ResolvedBindings` so Subscriptions and Rating can prove which successor was in force at a binding's `ends_on`; report `PricesPublished` as a wake-up signal; accept a `report_terms_discrepancy` exchange. Automatic adoption of a successor still requires the explicitly accepted continuation policy (SUB-D-41) | Subscriptions publishes the authorized successor fact and exact seal from proven evidence; a closure or a returned successor alone changes no customer terms |

Current [`pricing/pricing/src/domain/usage_policy.rs::refuses_minimum_fee`](../../../pricing/pricing/src/domain/usage_policy.rs) rejects both `CalendarHour` and `AggregationScope::Resource`; `commercial_terms.rs` enforces that restriction during admission. PC-P07 is required provider work, not delivered capability or authorization to remove guards without the matching calculation/evidence contract. The existing pooled price/subscription/period profile remains valid for its original terms. The selected minimum is monetary and applies after eligible usage valuation; it does not impose a minimum billable quantity. Invoice-line grouping cannot select its scope.

Container storage/traffic and node-group storage use the tariff-selected hourly or billing-period monetary minimum. A partial eligible calendar hour retains the full hourly minimum. Partial billing-period treatment is a tariff choice between full and proportional minimum. Timezone/SDK mapping, eligible-duration evidence and identities remain open; this adds no default and preserves existing accepted terms. The current `AggregationScope` has no resource-group variant.

Historical membership and usage coverage need coordinated provider contracts; an inventory alone does not establish zero usage.

## Examples

**Catalog changes before start.** A protected EUR 20 purchase remains EUR 20 despite a later EUR 25 successor or expiry of the original offer. Subscriptions activates from its original accepted evidence under current order, financial, security and readiness guards. Any latest-start rule must have been part of the purchase.

**Free trial with a minimum fee.** A plan normally has a EUR 10 minimum. The accepted free-period block must make covered trial charges zero, including that floor. Setting only the per-unit rate to zero would leave an unintended EUR 10 charge.

**Credit trial.** A plan grants EUR 100 usable for eligible items for 14 days. Ordinary prices remain intact. Starting at T fixes the original expiry from those terms; retries, delayed finalization and early conversion do not issue a new grant or restart 14 days. These figures are examples only.

## Acceptance and boundaries

Version the authoring/read/accepted-purchase/commitment schemas and SDKs; specify supported dates, immutable references, release/revocation rules and provider failure outcomes. Execute AP and TP producer/consumer fixtures. Free-period enablement has no promotional-grant provider dependency; credit does. Extension/repeat trials, stacking, regrant, plan replacement and a Products trial flag remain excluded. Missing capability refuses the affected sale before protected capture.


AP03/AP04 and TP require versioned acceptance, commitment and historical-read contracts. Existing D-510 V1 does not supply those extensions.

AP requires durable initial acceptance covering the agreed future start; requiring V1 acceptance for a date V1 rejects would make that flow impossible. Coordinate initial Products and Pricing decisions before reporting accepted coverage; partial evidence cannot authorize capture. Historical recovery preserves original terms and acquisition identity. Ordinary catalog changes do not invalidate execution; explicit order cancellation and financial/security disposition retain their own fences.

TP freezes trial duration, complete fee behavior or credit applicability and the accepted paid target; unsupported data refuses before purchase. Unchanged accepted conversion terms reuse assent with current financial/security/lifecycle guards.
