---
description: "Cross-gear specification of commercial quotas from provider capability onboarding to purchase, fulfillment and runtime enforcement."
---

# Quotas: commercial lifecycle and enforcement

**Status: draft specification.** This package defines the commercial quota lifecycle across BSS, platform Quota Enforcement (QE) and resource providers: catalog onboarding, plans, purchase, fulfillment, resource accounting and recovery.

| Document | Purpose |
| --- | --- |
| [PRD](PRD.md) | What customers buy, how plans contain quotas, tenant ownership, lifecycle and observable outcomes |
| [DESIGN](DESIGN.md) | Owners, data and control flow, contracts, consistent application, failures and operations |
| [Acceptance and delivery gates](ACCEPTANCE.md) | Verification scenarios, provider qualification and delivery criteria |

The short path is:

```text
Provider capabilities / product engineering
  → discovery adapter: GTS capability snapshots with metric/unit references
  → Products: validation, review and versioned resource/metric/enforcement mappings
  → Pricing: a plan revision defines allocation limits and separate charging terms
  → AcceptanceReceipt: exactly what the customer accepted
  → Orders → Subscriptions: committed purchase, quantity and commercial posture
  → Entitlements: desired quota set and one tracked adapter operation
  → selected quota application adapter: coordinated application in OSS
```

A provider limit is technical evidence, not a purchased allowance. A plan rule is an offer, not a live tenant quota. A receipt proves acceptance, not authority to grant access. A quota record proves materialization, not successful enforcement. These distinctions are maintained throughout the documents.

For the example purchase of **10 vCPU and 20 GB RAM**, both limits belong to the resource-owning tenant. Authorized users consume that shared allowance. The adapter can maintain corresponding limits in several OSS systems, but these remain one purchased allowance. BSS receives a result for the complete quota set rather than coordinating each internal update. The plan specifies the units and authorized scope.

**Quota and price:** A 20-vCPU quota can accompany a package fee, usage pricing or a mixed plan. It limits concurrent allocation; charging terms determine what is included and what is billed. Entitlements derives limits from quota rules and the effective subscription quantity; Rating/Billing owns charges.

**Ownership:** Products defines resource semantics, Pricing owns plan rules and accepted terms, Subscriptions owns commercial posture, and Entitlements applies and reconciles purchased quotas. QE and resource providers own runtime accounting and enforcement. These responsibilities do not require a separate Quota Manager gear. Quota Resolver is an optional runtime facade; it does not import catalog entries or provision purchased limits.

The specification spans several gears and is organized under `gears/bss/docs/quotas`. [DESIGN](DESIGN.md) defines the component boundaries and [ACCEPTANCE](ACCEPTANCE.md) defines the delivery criteria.

**Quota readiness:** The adapter confirms that the complete desired quota set is applied and available. Buying resource limits alone does not require creating a VM. Entitlements consumes this confirmation within its readiness contract; resource fulfillment and the general activation process belong to the Entitlements and Subscriptions designs. Quota limits and occupied allocations remain consistent across their lifecycle transitions.

**Runtime admission boundary:** OSS ensures that resource creation fits every required quota under concurrency and preserves accounting through failures. Reservation and recovery mechanisms belong to the OSS design; BSS requires conformance evidence.

**Resource lifecycle boundary:** Resource services and their adapters own stop, resize, delete and cleanup. Quota accounting returns only capacity supported by authorized release evidence for the original allocation and resource generation. Only the confirmed portion is returned; unresolved occupancy stays counted and repeated confirmation cannot return capacity twice.

**Catalog input:** Products receives capability snapshots through a versioned discovery adapter SDK. GTS identifies and validates metric/unit definitions and extensible capability records. Catalog review and profile qualification precede sale; a registered type is not a purchased quota. [DESIGN §3.3.1](DESIGN.md#331-catalog-onboarding-and-plan-publication) defines fields, examples, validation and versioning.

**Sale eligibility:** The adapter owner verifies the integration version and supported quota combinations. Products retains qualification evidence and controls availability. Pricing checks every required quota in the plan at publication and purchase acceptance. Physical capacity checks remain in OSS.

**Read boundary:** BSS exposes purchased limits, application status and adapter-reported fulfillment discrepancies. OSS owns effective limits, usage, reservations, remaining capacity and admission decisions. A portal can combine the two sources; Entitlements does not maintain or serve an operational usage projection.

**Application boundary:** Entitlements submits desired quota state through the BSS-selected adapter contract and reads its correlated result. The receiving OSS component and the mechanism used to set the limits are defined by the adapter design.

**Plan authoring:** [DESIGN §3.3.1.1–3.3.1.2](DESIGN.md#3311-catalog-reads-for-plan-authoring) specifies Products catalog listing/detail reads, `PlanRevision.quota_rules`, atomic draft replacement, existing Pricing checks/publication and frozen purchase evidence. [The purchase-to-adapter path](DESIGN.md#361-from-purchase-to-quota-readiness) names each handoff, payload and recovery boundary from Pricing through Orders, Subscriptions and Entitlements to the selected adapter.
