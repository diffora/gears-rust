---
description: "Typed quota rule block on PlanRevision, its publication validation against Products evidence and its freeze in the acceptance receipt, requested from Pricing by Entitlements."
---

# Pricing rule proposal from Entitlements

**Status:** proposed owner handoff from Entitlements, 2026-10-23. This is a request to the Pricing owner, not an accepted amendment to the Pricing design, a delivered SDK or an external-owner approval. Examples are illustrative, not catalog defaults. The [proposal index](README.md) provides scope and source precedence; the consuming feature is [quota projection](../features/03-quota-projection.md).

<!-- toc -->

- [1. Baseline](#1-baseline)
- [2. Required changes](#2-required-changes)
- [3. Example](#3-example)
- [4. Acceptance](#4-acceptance)
- [5. Open points](#5-open-points)

<!-- /toc -->

## 1. Baseline

| Inspected source | What it establishes |
| --- | --- |
| [Pricing NewSaleQuery and AcceptanceReceipt](../../../pricing/pricing-sdk/src/acceptance.rs), [BindingSelection](../../../pricing/pricing-sdk/src/read.rs) | One positive exact Decimal quantity for the selected plan; selection contains only item ID and optional dimension value. Receipt retains the query and bindings. There is no per-component quantity in BindingSelection and no quota rule on the revision. |
| [Trial pricing proposal, TP01](../../../subscriptions/docs/TRIAL_PRICING_PROPOSAL.md#10-reciprocal-trial-delivery-contract) | A separately versioned optional trial block is already proposed on PlanRevision authoring, read and publication; an absent block means no trial. The rule block below follows the same placement. |
| [Products PRD](../../../products/docs/PRD.md) | Existing descriptors are billing descriptors (`gl_code`, `tax_category`, `invoice_line_template`). No general quota-definition catalog exists; the metric evidence a rule refers to is requested in the [products metric proposal](products-metric-proposal.md). |

Pricing's place in the ownership after the split (ENT-D-03): it owns PlanRevision quota rules, fixed and per-plan-unit coefficients, their binding to plan selections, publication validation and immutable acceptance and continuation evidence. It does not count usage, resolve mutable resource ownership or issue live quota. Changes to quota terms require a new accepted revision, even if the price is unchanged. The meaning of the rule (kinds, exact conversion, validity, suspend and revoke policy) is owned by Entitlements and published as a versioned semantic schema; Pricing stores and freezes it.

**Recommended option: Pricing owns typed rules; Products supplies semantics.** Two plans for the same SKU can sell different limits without duplicating SKU identity, and quota revisions follow commercial acceptance. A fixed cap and a coefficient are sufficient for the initial model. The cost is that Pricing must validate metric, unit and subject semantics against historical Products or provider evidence, and accepted snapshots must preserve these references and values. The alternative, Products-owned immutable definitions bound by Pricing, adds a second definition lifecycle and a precise split between invariant fields and plan overrides; it is kept for later if reuse justifies it, with the same accepted snapshot and provider boundary (ENT-Q01).

## 2. Required changes

| Request | Minimal change / release evidence |
| --- | --- |
| PR-P01 Typed rule block on PlanRevision | Add a separately versioned optional block of `PlanQuotaRule` and `QuotaBinding` entries to PlanRevision authoring, read and publication, beside the trial block of TP01. Preserve current V1; an absent block means no purchased quota declaration, never zero or unlimited. Each rule carries `rule_id`, immutable revision and digest, the Products SKU/version and metric semantics reference with digest, metric code and canonical unit, `kind`, `subject_kind`, `capacity = Fixed(exact_quantity) \| PerPlanUnit(exact_coefficient)`, applicability, period where applicable, hard-enforcement policy, `combination = ExclusiveSource`, validity, suspend/revoke policy and capability profile/version. Each binding names plan/revision, rule/revision, applicable item IDs and accepted dimension values, immutable Products references and quantity basis `PlanQuantity`. Define a stable logical quota binding ID and a versioned semantic-equivalence digest (unit/mapping, coefficient, applicability, subject, window and lifecycle policy); a matching `item_id` across revisions does not prove equivalence. |
| PR-P02 Acceptance freeze | Accept the quota binding and snapshot together with the existing query and bindings in the same `AcceptanceReceipt`, as an `AcceptedQuotaSnapshot`: acceptance ID, request/terms digests, order/version/line, plan/revision, the original exact plan quantity, the complete selected rule and binding snapshots or durable immutable resolvable references with digests, seller/payer/resource tenant axes, authorized subject-binding policy, terms/validity/continuation policy and required provider profile. Subsequent lot and delta acceptances remain separate immutable evidence. Replay and continuation read the frozen rule from the receipt, never the live plan. Acceptance fixes cap semantics and tenant axes; it is not permission to apply. |
| PR-P03 Publication validation | At publication, validate every rule against Products evidence: the metric exists with the stated unit and supported subject kinds, the period is permitted for the kind, the coefficient and fixed quantity are exact decimals with published bounds, and the unit conversion to the enforcement unit is accepted data. Refuse an unknown enum variant, an unsupported profile version or an unbounded capacity until an explicit unbounded profile is accepted. Refuse publication when the referenced Products evidence version is not resolvable by digest. |

Pricing does not evaluate `PerPlanUnit` and does not compute caps; the exact conversion runs in Entitlements from the frozen values. A rule change, a coefficient change or a unit change produces a new PlanRevision and a new acceptance; an accepted receipt is never edited.

## 3. Example

Assume Products evidence defines metric `storage.allocated_bytes` with unit `byte` and a supported `project` subject. Pricing publishes plan revision `P7` with rule `storage-cap/R1`: allocation, `PerPlanUnit(107374182400 bytes)`, exclusive source, hard limit, no consumption period. The coefficient is exactly 100 GiB per plan unit. These identifiers and values are examples, not catalog defaults or literal SDK values.

1. Authoring adds the rule block to `P7` with the binding `storage-cap` for the plan item; `BindingSelection` stays unchanged. Publication validation resolves `storage.allocated_bytes`, `byte` and `project` against the Products evidence version by digest and accepts the exact coefficient (PR-P01, PR-P03).
2. A new-sale line selects `P7` with exact plan quantity `"3"`. Acceptance writes the receipt with the existing query and bindings plus the `AcceptedQuotaSnapshot`: rule `storage-cap/R1` with its digest, binding `storage-cap`, quantity `"3"`, the tenant axes and the required provider profile (PR-P02).
3. Entitlements later reads the receipt, not `P7`, and derives `3 × 107374182400 = 322122547200 bytes`. If Pricing publishes `P8` with a 200 GiB coefficient the next day, the existing receipt still says 100 GiB; the subscription keeps 100 GiB per unit until a new acceptance.
4. A second fixed rule `Fixed(10 projects)` on the same revision freezes as ten; it is not multiplied by the plan quantity at acceptance or later.

## 4. Acceptance

The cases are defined in the [quota enforcement proposal](quota-enforcement-proposal.md); Pricing owns their evidence side.

| Case | Pricing obligation |
| --- | --- |
| QE-C01 Accepted evidence | After a new plan or metric revision is published, a replay still resolves the original rule, unit, quantity and digest from the receipt; no live-catalog fallback. Missing history is pending or refused, not unlimited. |
| QE-C03 Quantity basis | The receipt carries one exact plan quantity; a per-plan-unit rule is bound once per logical binding, a fixed rule once, and no per-component quantity is invented from several item selections. |
| QE-C22 Binding equivalence | A newer PlanRevision that reuses an item ID with changed quota semantics yields a different logical quota binding ID and digest; lots are combined only under the agreed semantic-equivalence contract and the distinct-binding gates. |

## 5. Open points

| Question | Owners | What this proposal needs |
| --- | --- | --- |
| ENT-Q01 | Pricing, Products, Entitlements | Whether Pricing-owned rules are accepted and which Products or provider descriptor supplies each allocation metric, unit and authorized subject kind. |
| ENT-Q02 | Pricing, Orders, Entitlements | The exact rule applicability and the change-order delta mapping; the single plan quantity of NewSaleQuery must remain unambiguous. |
| ENT-Q07 | Pricing, Quota Enforcement, Entitlements | Accepted continuation behavior for quota terms and the supported lot combinations. |

These questions are registered in [DECISIONS.md](../DECISIONS.md#open-questions). The gate this proposal closes is E-U03 in [UPSTREAM_REQS.md](../UPSTREAM_REQS.md).
