---
description: "Decisions and open questions of the Entitlements application layer."
---

# Entitlements - Decisions Log

<!-- toc -->

- [Status board](#status-board)
- [Decisions](#decisions)
- [Open questions](#open-questions)

<!-- /toc -->

Decision ids are `ENT-D-NN`; open questions are `ENT-QNN`. A decision carries a **Propagated** line naming the documents that cite it. Two founding decisions were accepted in the Subscriptions design set and bind both gears; they are adopted here with their original dates. Open questions are gates: a profile that depends on an open question stays unavailable until its owners close it.

## Status board

| ID | Sev | Decision | Status |
| --- | --- | --- | --- |
| ENT-D-01 | H | Entitlements owns the provider-independent application boundary; native LM/OSS are adapters | ACCEPTED 2026-10-14 in Subscriptions, adopted here 2026-10-23 |
| ENT-D-02 | H | Initial usable readiness precedes Subscriptions' active commit (IA-2026-10-18) | ACCEPTED 2026-10-18, adopted here 2026-10-23 |
| ENT-D-03 | H | Rule semantics owned here; rules stored on PlanRevision and frozen in Pricing acceptance | ACCEPTED 2026-10-23 |

## Decisions

#### ENT-D-01 Provider-independent application boundary

- **Where**: [DESIGN.md](DESIGN.md) §1.1 and §3.3.3; [ADR 0001](ADR/0001-application-boundary.md).
- **Problem**: BSS must not require the platform's own License Manager or OSS to apply what was bought. A deployment may run a native system, a third-party licensing or service system or a customer integration. Before the split, Subscriptions carried both the commercial posture and the adapter boundary, and the boundary had no owner.
- **Decision (2026-10-14 in Subscriptions, adopted 2026-10-23)**: The application of commercial posture depends on abstract capabilities implemented by deployment-selected adapters: entitlement fulfillment, resource execution and inventory evidence. Entitlements owns this boundary. The native License Manager, a third-party system and a customer integration are alternative adapters of the same capability profile; none is mandatory because another deployment uses it. Provider-specific APIs, reply schemas and status mappings belong to adapter specifications. The core keeps desired state, correlated pending, applied and failure observations, and the end-to-end idempotency, stale-action and recovery guarantees; a vendor need not expose our revision field or state machine. Required capabilities are explicit per product profile; an explicitly inapplicable capability differs from an unavailable required adapter, and an absent required adapter is never fabricated success. A native provider PRD is not a completion gate of this design; runtime enablement still needs the selected adapter to satisfy its applicable guarantees. Subscriptions publishes the commercial posture and consumes readiness; it holds no adapter contract.
- **Propagated**: DESIGN.md §1.1; features/02-initial-readiness.md; features/03-quota-projection.md; features/04-license-grants.md; features/05-resource-execution.md; features/06-reconciliation-recovery.md; proposals/README.md; proposals/quota-enforcement-proposal.md; proposals/resource-provider-proposal.md; ADR/0001-application-boundary.md

#### ENT-D-02 Initial readiness before active (IA-2026-10-18)

- **Where**: [PRD.md](PRD.md) §6.2 and the documents listed under Propagated.
- **Problem**: The earlier Subscriptions rule allowed initial usable access to follow commercial T. A subscription could then be active, accruing and billed while the service it bought did not work yet.
- **Decision (2026-10-18 in Subscriptions, adopted 2026-10-23)**: Initial activation requires confirmed usable service before active and before the service period and accrual start. Entitlements is the provider side of that gate. It persists the complete required-assignment and producer manifest for the activation operation before dispatch, makes the preparation assignments available through the selected adapters while the subscription is still draft, accepts readiness observations by authorized scope, concrete assignment and desired revision, and reports ready only when every assignment of the manifest is ready for the exact desired revision. No `subscriptionId` crosses the adapter boundary; Subscriptions keeps the association from its operation to the assignments. Subscriptions is the consumer side: its activation operation stays pending and the subscription stays draft until that evidence arrives, and its commit then rechecks fences and writes active, fresh local T, receipt and outbox together. Entitlements never assigns T and never treats a ready event as completion. Required initial assignments cover creation or linking of the account, the deployment included in the purchase and usable permissions; first login or a later unrelated purchase is not required. Late or unknown results keep their recovery; no readiness is not failure; withdrawal fences local activation and late external work with separate cleanup and financial obligations. Renewals, resume, trial conversion and modifications of an active subscription keep their existing contracts.
- **Limits**: The readiness envelope fields are semantic candidate names, not external SDK acceptance. The License Resolver remains read and check only. Provider mappings, freshness versus withdrawal arbitration, preactivation exposure and its financial treatment, and runtime proof remain open under ENT-Q09 to ENT-Q13.
- **Propagated**: PRD.md §2; DESIGN.md §1.1, §3.3.5; features/02-initial-readiness.md; features/04-license-grants.md; proposals/license-enforcement-proposal.md; UPSTREAM_REQS.md §2.1

#### ENT-D-03 Rule semantics here, storage and acceptance in Pricing

- **Where**: [PRD.md](PRD.md) §6.3 and the documents listed under Propagated.
- **Problem**: Quota and grant rules had three owners and no owner of meaning. Pricing stored a rule, Products supplied a metric, Subscriptions projected a value. Two drafts disagreed on the identity carried to the provider, and replay could read a live plan.
- **Decision (2026-10-23)**: Entitlements owns the rule schema and its semantics: `PlanQuotaRule` kinds `Fixed` and `PerPlanUnit`, exact conversion, validity, suspend and revoke policy, and grant definition semantics. Pricing stores the rule as an optional typed block on the immutable PlanRevision, next to the trial block, validates it at publication against Products metric evidence, and freezes it in the same acceptance receipt as the price bindings. Products supplies metric, unit and subject kinds and the versioned mapping to the enforcement metric registry. One acceptance evidence source exists: the receipt. Acceptance fixes the cap semantics and the tenant axes; it is not permission to apply. Derivation reads the frozen rule from the receipt together with the committed posture and never the live plan; a rule change needs a new accepted revision even when the price is unchanged.
- **Propagated**: PRD.md §6; DESIGN.md §2; features/03-quota-projection.md; proposals/pricing-rule-proposal.md; proposals/products-metric-proposal.md; proposals/quota-enforcement-proposal.md

## Open questions

| ID | Question | Owners | Blocks |
| --- | --- | --- | --- |
| ENT-Q01 | Whether Pricing-owned rules are accepted, and which Products/provider descriptor supplies each allocation metric, unit and authorized subject kind. Existing usage semantics may be reused only when enforcement equivalence is proved. | Pricing, Products, Entitlements | quota profile |
| ENT-Q02 | The exact rule applicability and change-order delta mapping. NewSaleQuery's single plan quantity must remain unambiguous; any independent component-unit model requires a versioned extension. | Pricing, Orders, Entitlements | quota profile |
| ENT-Q03 | Whether exclusive source is sufficient for initial products; which operator safety ceilings coexist, with what explicit precedence. Do not promise pooling before its owner contract exists. | Quota Enforcement, Products, Entitlements | quota profile |
| ENT-Q04 | The selected enforcement/entitlement adapter, source namespace authority, full-set atomicity boundary and permission model for proposed apply/read operations. | Quota Enforcement, Entitlements | quota profile |
| ENT-Q05 | Hard suspension/revoke behavior, fallback denial, existing allocations, cap-below-used policy and any separately authorized export access. | Quota Enforcement, Products, Entitlements | quota profile |
| ENT-Q06 | Exact validity boundary/clock mapping, allowed calendars and counter identity, especially for subscription renewal that does not align with UTC QE windows. | Quota Enforcement, Entitlements | quota profile |
| ENT-Q07 | Accepted continuation behavior for quota terms, supported lot combinations, late expiry handling, tombstone retention and safe administrative recovery. | Pricing, Quota Enforcement, Entitlements | quota profile |
| ENT-Q08 | SDK/profile versions, executed conformance evidence and provider readiness. Separate accepted design semantics from implemented code and from deployed capability. | Quota Enforcement, Entitlements | quota profile |
| ENT-Q09 | Rights-feed transport and recovery: event delivery, polling or another reliable feed from the selected adapter, and how authoritative assignment state and sequence are recovered after missed events, restart or retention gaps. | Entitlements, selected adapter | initial readiness profile |
| ENT-Q10 | Native schema and identity mapping: how the adapter maps its native identities to `scopeId`, `assignmentId` and `desiredRevision`, so that several environments under one broad right stay distinct and retired identities are not reused. | Entitlements, selected adapter | initial readiness profile |
| ENT-Q11 | Freshness versus withdrawal arbitration at commit: the current-evidence and revocation contract under which Subscriptions revalidates readiness before its guarded commit; a timestamp or a last health check alone cannot prevent a concurrent remote revocation, and no cross-service atomicity is claimed. | Entitlements, Subscriptions, selected adapter | initial readiness profile |
| ENT-Q12 | Cleanup policy and timeouts: per-profile readiness evidence, timeouts and escalation, and which resources a withdrawn operation may clean up under the accepted policy. | Entitlements, Subscriptions, selected adapter | initial readiness profile |
| ENT-Q13 | Preactivation exposure and its financial treatment: permitted preparation timing, usable exposure or usage before T, and who bears it. This design neither backdates service accrual nor invents free credit, permission to serve before financial admission or a universal pre-T liability policy. | Entitlements, Subscriptions, Billing | initial readiness profile |

The quota questions ENT-Q01 to ENT-Q08 are closed by a joint Pricing, Products and Entitlements schema agreement followed by a Quota Enforcement conditional-apply and recovery contract. The readiness questions ENT-Q09 to ENT-Q13 are closed per adapter profile; a profile lacking that contract cannot claim a safe initial activation on a cached ready event.
