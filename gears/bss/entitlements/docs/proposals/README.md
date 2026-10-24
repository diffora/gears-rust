---
description: "Owner-by-owner handoffs from the Entitlements application layer to Quota Enforcement, License Enforcement, Pricing, Products and the resource provider."
---

# Entitlements integration proposals by component

**Status:** owner handoffs, 2026-10-23. Each proposal is a request to an adjacent owner, not a delivered API or an external-owner approval. Examples are illustrative, not catalog defaults, SLA values or shipped SDK fields. The proposals were carried into this set by the split of the application layer out of Subscriptions; their asks and cases keep their numbering families (`QE-`, `LE-`, `PR-`, `PD-`, `RS-`). Every proposal applies the provider-independent boundary of ENT-D-01: adapters receive scope, assignment and desired revision, never the subscription identifier.

<!-- toc -->

- [Purpose and status](#purpose-and-status)
- [Source baseline and precedence](#source-baseline-and-precedence)
- [Component proposals](#component-proposals)
- [Common evidence and numeric rules](#common-evidence-and-numeric-rules)
- [Delivery order and scope limits](#delivery-order-and-scope-limits)
- [Acceptance record](#acceptance-record)

<!-- /toc -->

## Purpose and status

These files state what each adjacent component must change so that Entitlements can apply committed commercial posture through it, what is already compatible, and how the change behaves in concrete examples. They derive from the [PRD](../PRD.md), the [DESIGN](../DESIGN.md) and the features of this set. Adoption belongs in separate owner PRs; this set changes no adjacent gear. The proposals do not reopen product policy, create owner approvals or claim delivered APIs. The gates they close are `E-U01` to `E-U05` in [UPSTREAM_REQS.md](../UPSTREAM_REQS.md); the open questions they depend on are `ENT-Q01` to `ENT-Q13` in [DECISIONS.md](../DECISIONS.md#open-questions).

## Source baseline and precedence

| Source | Inspected revision | Use |
| --- | --- | --- |
| Quota Enforcement source | `ec7619e182b5b78a5bc8e9614dc3e2efa1c500a8` | Pinned baseline for every statement about QE models, manager, storage, admission and PEP; a signature at this commit is not evidence of a deployed runtime. |
| [License Enforcement PRD](../../../../system/license-enforcement/docs/PRD.md) | repository baseline | Named type and instance grants, assignment and licensing checks; numerical limits stay with QE. |
| [License Resolver DESIGN](../../../../system/license-resolver/docs/DESIGN.md) | repository baseline | Read and check boundary; not the owner of issuing rights or a delivered change feed. |
| [Subscriptions design set](../../../subscriptions/docs/PRD.md) | current main | Publisher of `CommercialPostureV1` and consumer of `ReadinessObservationV1`; retired ids in its PRD §17.2. |
| [Pricing PRD](../../../pricing/docs/PRD.md), [Products PRD](../../../products/docs/PRD.md) | repository baseline | PlanRevision, acceptance receipt, SKU and metric evidence as they exist today; the typed rule block and the metric mapping are proposed here. |

Statements use the pinned file contents, not PR descriptions. Links to owner documents describe the baseline, not acceptance of these proposals. External owner acceptance and concrete adapter and version mapping remain open even when two documents agree.

## Component proposals

| Component | Proposal | Main request |
| --- | --- | --- |
| Quota Enforcement | [quota-enforcement-proposal.md](quota-enforcement-proposal.md) | Source identity and atomic fenced application, authorized recovery by source key, access posture and precedence, windows and validity, complete runtime, versioned commercial evidence (QE-P01 to QE-P06; cases QE-C01 to QE-C22). |
| License Enforcement / selected license adapter (LE, native LM or third party) | [license-enforcement-proposal.md](license-enforcement-proposal.md) | Correlated grant application by scope, assignment and generation, stale-action suppression, idempotent recovery, preparation grants in draft, trial boundaries (LE-P01 to LE-P05; cases LE-C01 to LE-C06). |
| Pricing | [pricing-rule-proposal.md](pricing-rule-proposal.md) | Typed rule block on PlanRevision beside the trial block, publication validation against Products evidence, freeze in the acceptance receipt (PR-P01 to PR-P03). |
| Products | [products-metric-proposal.md](products-metric-proposal.md) | Metric, unit and subject semantics; registered enforcement metric and versioned mapping (PD-P01 to PD-P02). |
| Resource provider | [resource-provider-proposal.md](resource-provider-proposal.md) | Physical outcomes per operation and resource generation, non-destructive suspension, retention and export, hold and delete arbitration, recovery by original identity (RS-P01 to RS-P05; cases RS-C01 to RS-C04). |

The names above are fixed.

## Common evidence and numeric rules

Every integration identifies producer and consumer, accepted contract version, business and operation identity, source revision and digest, authorized scope, effective and validity boundaries, unknown and conflict outcomes and durable recovery. Exact DTO names remain owner work; conceptual fields in these proposals are not shipped SDKs. Read-only prechecks never replace effect-owner fencing. No network call runs inside a local state mutation transaction and no cross-gear database transaction is introduced.

Adapter payloads carry scope, assignment identity and desired revision or generation; none carries the subscription identity. Quantities are exact: caps are integral, nonnegative and bounded, converted with exact decimal or rational arithmetic and refused on any fractional, negative, overflowing or unit-incompatible result. No money crosses these contracts.

## Delivery order and scope limits

Agree the `CommercialPostureV1` and `ReadinessObservationV1` versions with Subscriptions first (E-U06). In parallel, settle the Pricing rule block and the Products metric mapping (E-U03, E-U04), then the QE conditional-apply and recovery contract (E-U01). Select the license and resource adapters per deployment and run their fixtures (E-U02, E-U05). Enable the smallest supported allocation profile only after its failures are demonstrated; consumption and pooling must not delay a valid allocation profile, and an incomplete allocation runtime cannot be excused by deferring consumption.

Cross-subscription pooling, consumption profiles, rate limits, Products-owned reusable templates, rolling and anniversary periods, explicit unbounded plans and counter-preserving subject or metric migration remain later phases. Deferred Subscriptions capabilities are not reopened by any proposal here.

## Acceptance record

For each component record the ratified version, the concrete owner mapping, supported and refused profiles, executed cases and remaining gates. Behavior marked "already specified" still requires runtime evidence. Source review and document validation do not close provider conformance. A gated profile is not ready because its prose is complete, and the whole set is not unimplemented because one optional profile is gated.
