---
description: "Product requirements for the complete quota lifecycle across BSS, Quota Enforcement and resource provider integrations."
---

# PRD — Commercial Quotas


<!-- toc -->

- [1. Overview](#1-overview)
  - [1.1 Purpose](#11-purpose)
  - [1.2 Background / Problem Statement](#12-background--problem-statement)
  - [1.3 Goals (Business Outcomes)](#13-goals-business-outcomes)
  - [1.4 Glossary](#14-glossary)
- [2. Actors](#2-actors)
  - [2.1 Human Actors](#21-human-actors)
  - [2.2 System Actors](#22-system-actors)
- [3. Operational Concept & Environment](#3-operational-concept--environment)
- [4. Scope](#4-scope)
  - [4.1 In Scope](#41-in-scope)
  - [4.2 Out of Scope](#42-out-of-scope)
  - [4.3 Future scope](#43-future-scope)
- [5. Functional Requirements](#5-functional-requirements)
  - [5.1 Catalog and plans](#51-catalog-and-plans)
  - [5.2 Purchase and attribution](#52-purchase-and-attribution)
  - [5.3 Delivery and enforcement](#53-delivery-and-enforcement)
  - [5.4 Changes and end of service](#54-changes-and-end-of-service)
  - [5.5 Visibility and recovery](#55-visibility-and-recovery)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 Capability-specific requirements](#61-capability-specific-requirements)
  - [6.2 NFR Exclusions](#62-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
  - [8.1 Buy and consume a tenant package](#81-buy-and-consume-a-tenant-package)
  - [8.2 Change quantity and recover a partial delivery](#82-change-quantity-and-recover-a-partial-delivery)
  - [8.3 Restrict and end service](#83-restrict-and-end-service)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Risks](#12-risks)
- [13. Open Questions](#13-open-questions)
- [14. Traceability](#14-traceability)

<!-- /toc -->

**Status: proposed.** Allocation quotas are the initial delivery profile. The requirements describe target behavior; [qualification gates](DESIGN.md#42-integration-qualification-gates) distinguish it from delivered capability. Numbers in examples are illustrative offers, not platform defaults.
## 1. Overview

### 1.1 Purpose

Define the complete path from a provider's resource capabilities to a sellable plan, a customer's accepted purchase, live tenant limits, resource consumption and eventual release. A customer and an operator must be able to explain every applied limit by its purchase or an explicitly separate administrative restriction.

This PRD defines the commercial outcomes and acceptance criteria. [DESIGN](DESIGN.md) specifies owners, contracts, the Pricing-to-adapter path and integration qualification. JSON contract examples are stored in `schemas/`.

### 1.2 Background / Problem Statement

A tenant buys a service plan with numerical resource limits, such as 10 vCPU and 20 GB RAM. BSS must preserve the purchased terms, attribute the allowance to the resource-owning tenant, and apply it through the selected adapter. Authorized users share that allowance while creating and changing resources. The allowance limits permitted allocation; it does not imply that usage is prepaid or included in a package. The accepted plan separately defines package charges, usage charges and any included usage.

Without a common contract, importing a technical limit could accidentally create a sellable entitlement; a plan change could change an old purchase; independent quota records could multiply allowance; and a successfully written limit could be reported as ready although the provider rejects the promised capacity.

### 1.3 Goals (Business Outcomes)

- Every purchased numerical allowance is traceable to immutable accepted terms and an authorized resource owner.
- A supported purchase becomes usable after its quota readiness conditions are met and, only when resource deployment is included in the purchase, the required resources are ready.
- Duplicate delivery, retry, activation and renewal create no extra allowance and erase no occupied capacity.
- BSS exposes purchased limits, their application status and adapter-reported fulfillment discrepancies. OSS owns operational quota state and usage reads.
- Unsupported resource/profile combinations are rejected before sale instead of becoming impossible fulfillment obligations.

### 1.4 Glossary

| Term | Definition |
| --- | --- |
| Metric descriptor | Versioned technical meaning: resource, unit, subject scope and supported enforcement behavior |
| Quota rule | Commercial limit semantics attached to a plan revision, such as 10 vCPU per plan unit |
| Binding | Selection of a rule for a plan item, dimension and subject-binding policy |
| Accepted snapshot | Immutable record of the selected rules, quantity and supporting evidence |
| Allocation quota | Limit on concurrently occupied or reserved resources; confirmed release can return capacity. The limit does not determine usage charges. |
| Plan quota / purchased limit | Allocation ceiling authorized by accepted plan terms, including a usage-priced plan; not proof of prepaid or included usage |
| Included usage | Usage covered by the accepted charging terms, measured under their billing meter and period; distinct from an allocation ceiling |
| Consumption quota | Limit on cumulative consumption in a defined window; separate future profile |
| Purchased cap | Amount promised by accepted terms for the relevant interval |
| Effective restriction | Actual independent provider/operator constraint, which can be lower than the purchased cap |
| Remaining | OSS-authoritative available amount after occupied/reserved capacity and applicable restrictions; read through OSS contracts |
| Resource tenant | Tenant that owns the resource scope; may differ from seller, payer and the acting user's home tenant |
| Provider scope | Logical boundary in which a provider enforces a quota and accounts for allocations, identified through an authorized adapter binding |
| Assignment | Stable fulfillment identity for one acquisition and its source-owned quota members |
| Quota application | One adapter operation that establishes the complete desired quota set across all required OSS systems |
| Profile | Versioned, tested set of supported metrics, scopes, lifecycle, admission and recovery guarantees |

## 2. Actors

### 2.1 Human Actors

#### Product engineer / catalog administrator

**ID**: `cpt-cf-bss-quotas-actor-catalog-owner`

- **Role**: Onboard resource semantics and approve provider mappings.
- **Needs**: Publish reviewed quota semantics and supported adapter capabilities.

#### Commercial plan author

**ID**: `cpt-cf-bss-quotas-actor-seller`

- **Role**: Package and price explicit limits without changing technical meanings.
- **Needs**: Configure understandable quota limits and separate charging terms on plans.

#### Authorized tenant purchaser

**ID**: `cpt-cf-bss-quotas-actor-buyer`

- **Role**: Buy and change an allowance and understand delivery status.
- **Needs**: Know the accepted limits, charging terms and fulfillment outcome.

#### Authorized resource user

**ID**: `cpt-cf-bss-quotas-actor-user`

- **Role**: Create, resize and delete resources against the owning tenant's allowance.
- **Needs**: Use shared tenant capacity with predictable admission and release behavior.

#### Platform/provider operator

**ID**: `cpt-cf-bss-quotas-actor-operator`

- **Role**: Inspect restrictions, discrepancies and recoverable outcomes.
- **Needs**: Recover incomplete fulfillment using correlated evidence and scoped authority.

### 2.2 System Actors

#### Products, Pricing, Orders, Subscriptions

**ID**: `cpt-cf-bss-quotas-actor-commerce`

- **Role**: Maintain catalog, accepted evidence and commercial lifecycle.

#### Entitlements and selected adapters

**ID**: `cpt-cf-bss-quotas-actor-fulfillment`

- **Role**: Apply and reconcile committed rights.

#### QE, resource providers and resource service

**ID**: `cpt-cf-bss-quotas-actor-enforcer`

- **Role**: Enforce admission, account for actual effects and report evidence.

## 3. Operational Concept & Environment

The platform follows the [ToolKit architecture](../../../../docs/toolkit_unified_system/README.md) and [project guidelines](../../../../guidelines/README.md). This specification allocates responsibilities across BSS and OSS owner contracts; it introduces no separate quota-management runtime unit. Named contract extensions are required target capabilities, with qualification prerequisites in [DESIGN §4.2](DESIGN.md#42-integration-qualification-gates).

A deployment can enforce the same allowance in several OSS systems. The selected adapter owns their coordinated application and exposes one application result to BSS. Its profile must cover every enabled resource-access path. Physical resource availability is separate from quota permission: buying 10 vCPU does not by itself reserve ten physical CPUs.

## 4. Scope

### 4.1 In Scope

Resource onboarding; plan authoring and versioning; acceptance and purchase; tenant attribution; quota application and readiness; compatible quantity changes; renewal, suspension, resume, expiry and cancellation; consistent quota application across OSS; quota-accounting and release guarantees at the OSS boundary; commercial/application visibility, repair and capability qualification.

### 4.2 Out of Scope

Initial delivery excludes shared pools across acquisitions/providers, tenant-to-child budget redistribution, per-user budget partitions, arbitrary formula languages, unlimited quotas, admission beyond a hard quota cap, voluntary quantity decreases/base-plan changes, automatic legacy subscription adoption, resource migration between providers, and general consumption/rate-limit profiles. These need additional commercial and enforcement contracts. The design explicitly describes the boundaries so that plan publication cannot silently enable them. Lower effective caps caused by lifecycle or provider restrictions still require safe behavior.

The general subscription activation process, readiness aggregation, activation deadlines, cancellation arbitration and preparation cleanup orchestration belong to the Entitlements and Subscriptions designs. This specification defines the quota contribution to their contracts. Resource stop, resize, delete, retention, export and cleanup orchestration belong to the resource lifecycle and adapter designs; this package defines the release evidence required for quota accounting.

Operational quota usage, reservations, effective limits, remaining capacity and admission decisions belong to OSS. Their storage, calculation and read APIs are outside BSS scope. A portal may combine BSS commercial/application data and OSS operational data without introducing an Entitlements usage projection.

The quota contract supports package-priced, usage-priced and mixed plans by keeping quota rules separate from accepted charging terms. Rating algorithms, usage ingestion, included-usage accounting and tariff execution remain with their financial/usage owners. Monetary credit, charging, taxes, refunds, named license grants and retention policy remain with their existing owners. Quota state supplies evidence and follows authorized posture; it does not decide those policies.

### 4.3 Future scope

Deferred extensions are before/after change previews with reasons, a dedicated explanation and historical exploration API/UI, and commercial composition of add-ons, promotions, negotiated overrides or multiple subscriptions. Broader profiles listed in §4.2 remain deferred. Existing accepted-terms preview, immutable history/audit, application status and discrepancy reasons remain v1 requirements. DESIGN §4.1 defines these boundaries; none of the deferred features is required to enable the initial profile.

## 5. Functional Requirements

### 5.1 Catalog and plans

#### Resource entry into BSS

- [ ] `p1` - **ID**: `cpt-cf-bss-quotas-fr-catalog`

The system MUST support authorized onboarding through a Products-owned, versioned GTS capability discovery contract. Metrics and units MUST be registered GTS instances; capability records MUST validate against their registered base or derived schema, with exact-version references, canonical unit conversion, supported resource/subject types, technical bounds and explicit operation capabilities. Products MUST validate and store candidates before catalog approval. Each published quota metric MUST provide a reviewed localized display name and description for plan authoring, with an English fallback. The description MUST reflect the metric's accounting semantics; relevant profile-specific conditions MUST be visible when selecting the adapter capability. The form MUST show the unit and validate the author's commercial amount against the selected capability using exact conversion. Manual authoring MUST use the same GTS identities, validation and approval rules. Provider discovery MUST produce reviewable candidates; it MUST NOT create a price, purchased entitlement or live customer quota. Monetary, feature, retention and numerical limits MUST be classified separately. Historical descriptor versions MUST remain resolvable for accepted purchases. Each qualified definition MUST pin registered accounting and scope policies with retained content digests: what allocated capacity counts, treatment of resource states and uncertain effects, release evidence, and the exact aggregation/dimensions. A description alone MUST NOT establish compatibility. Platform GTS identities MUST remain major-only; compatible schema revisions MUST retain historical content evidence and breaking changes MUST use a new major.

#### Quotas in plans

- [ ] `p1` - **ID**: `cpt-cf-bss-quotas-fr-plan-rules`

A plan revision MUST declare each purchased quota as a selected, versioned rule with `Fixed` or `PerPlanUnit` capacity and a precise applicability binding. The same product MUST support different caps in different plans. Publication MUST reject unknown metrics, unsupported scopes/profiles, inexact conversions, negative/overflowed amounts and incompatible combinations. Absent rules MUST mean no quota declaration, distinct from explicit zero and from unbounded capacity. A quota-semantic change MUST require a new plan revision and acceptance even when price is unchanged. The plan MUST specify charging terms separately from quota rules and associate both with the applicable service/resource scope. A quota cap MUST NOT imply prepaid usage, a billing inclusion, a tariff or a payment schedule. Package, usage and mixed charging MUST be distinguishable in the accepted terms and plan-authoring view; Entitlements MUST derive limits from quota rules without deciding usage charges.

Plan authoring MUST expose an authorized, paginated catalog of approved quota definitions applicable to the selected SKU/version, including reviewed descriptions, units, profile conditions and availability. Exact historical definition versions MUST remain readable. Pricing MUST store quota rules on the plan revision with item/logical-resource bindings and expose read and atomic draft replacement operations for adding, editing and removing them. Writes MUST reject stale revision versions and conflicting retries, and MUST preserve published and accepted terms. Existing plan checks, copy/clone, approval/publication and purchase acceptance MUST include quota content and its digest. Price objects MUST retain their charging responsibility; they MUST NOT become quota stores. Details of these owner contracts are defined together in DESIGN §3.3.1.1–3.3.1.2.

#### Exact quantity and combinations

- [ ] `p1` - **ID**: `cpt-cf-bss-quotas-fr-quantity`

Derivation MUST use the current effective committed plan quantity supplied by Subscriptions, with immutable provenance for the original purchase and compatible additions. A per-plan-unit rule is multiplied once by that effective quantity; a fixed rule is applied once per logical binding. The original receipt quantity MUST remain immutable and MUST NOT substitute for the effective quantity after additions. Billing-meter quantities MUST NOT substitute for subscription quantity or increase an allocation cap. Several selected items or purchase retries MUST NOT duplicate that rule. Compatible committed quantity changes MUST replace the cap while preserving usage. Unsupported independent component quantities, overlapping commercial sources or differing windows/terms MUST be refused before purchase/change. Initial installation and increase support MUST be qualified separately; an increase MUST preserve occupancy and the prior safe allowance while incomplete. The selected scope dimensions MUST survive from plan to applied assignment without duplicating an allowance across regions or resource pools. Before commercial commitment/capture, protected admission MUST exclude conflicting acquisitions on the canonical quota ownership domain across payers/products. Missing domain resolution or unknown claim outcomes MUST block commitment; claim recovery/retention MUST prevent delayed fulfillment or cancellation from releasing it prematurely.

### 5.2 Purchase and attribution

#### Frozen purchase evidence

- [ ] `p1` - **ID**: `cpt-cf-bss-quotas-fr-acceptance`

Acceptance MUST preserve price and quota terms together, including rule/binding versions, exact original quantity, metric mappings, provider profile, validity and seller/payer/resource tenant axes. Replay MUST recover the same receipt; changed content under the same purchase identity MUST conflict. Missing historical evidence MUST block fulfillment; it MUST NOT fall back to a live plan or unrestricted access.

#### Shared tenant ownership

- [ ] `p1` - **ID**: `cpt-cf-bss-quotas-fr-attribution`

The quota MUST bind to the authorized resource owner rather than automatically to the acting user. All supported users creating resources in that scope MUST consume the same tenant allowance. Subject/provider scope mappings MUST be verified and retained. The purchaser, seller or payer role alone MUST NOT authorize a foreign resource scope. Optional user limits MUST NOT replace or bypass the purchased aggregate tenant limit.

#### Committed lifecycle and readiness

- [ ] `p1` - **ID**: `cpt-cf-bss-quotas-fr-readiness`

Only committed, authorized commercial posture MUST drive quota fulfillment. A draft or an acceptance receipt alone MUST NOT grant usable quota. Quota readiness MUST require adapter confirmation that the complete desired quota set is applied and available for the authorized scope and revision. A purchase of resource limits only MUST NOT require creating resources to establish quota readiness. Entitlements consumes this confirmation within its readiness contract; resource readiness and the subscription activation process belong to Entitlements and Subscriptions. Any authorized preparation that consumes quota MUST stay within purchased capacity and count occupied allocations before activation. Activation MUST preserve those allocation identities and counters. An unknown lifecycle outcome MUST NOT release occupied quota or create another allowance.

### 5.3 Delivery and enforcement

#### Consistent quota application

- [ ] `p1` - **ID**: `cpt-cf-bss-quotas-fr-application-state`

Entitlements MUST submit the complete desired quota set as one operation to the selected adapter. The adapter MUST coordinate application across all required OSS systems and confirm success only for the complete change. The Entitlements-to-adapter result MUST use `pending`, `applied`, `refused` or `unknown`. Known partial application MUST remain `pending`; uncertain effects MUST be `unknown`. A definitive `refused` outcome MUST NOT leave unresolved effects or recovery obligations. BSS MUST keep incomplete application recoverable through the same adapter and operation identity. During an increase, the adapter MUST preserve the previously effective safe allowance until the complete increase is applied. Internal copies of a quota MUST NOT create additive entitlement.

#### Concurrent admission and occupancy

- [ ] `p1` - **ID**: `cpt-cf-bss-quotas-fr-accounting`

Concurrent resource operations MUST NOT exceed any required purchased metric cap. OSS MUST permit resource creation only when quota is sufficient for every required metric under concurrent operations. Failure, timeout or a delayed outcome MUST NOT cause unaccounted resources, duplicate accounting or reuse of capacity that may still be occupied. The reservation, concurrency and recovery mechanism belongs to the OSS design. Successful creation MUST account for actual occupancy once; failed or unknown operations MUST retain recoverable identities. Occupied capacity MUST be returned only against authoritative, authorized release evidence identifying the original allocation, immutable resource generation, metric, exact amount and stable release identity. Repeated evidence MUST settle the same release once; conflicting evidence MUST NOT alter counters. Partial release MUST return only the confirmed amount and leave unresolved occupancy counted. A stopped resource MUST follow the profile's per-metric occupancy semantics; a stop request or status alone MUST NOT imply release. Confirmed release MUST reduce occupancy without increasing the purchased cap or bypassing a current restriction.

#### Controlled provider access

- [ ] `p1` - **ID**: `cpt-cf-bss-quotas-fr-provider-paths`

Every enabled create/resize path, including provider portals and APIs, MUST satisfy the published profile's admission guarantees. A path that bypasses the qualified quota enforcement MUST be disabled or mediated by OSS. The selected OSS implementation MUST prove the same complete admission guarantee on every enabled path; delayed usage import alone MUST NOT be presented as protection against concurrent over-allocation. The product MUST not offer aggregate cross-scope limits without enforcement for that aggregate.

### 5.4 Changes and end of service

#### Lifecycle continuity

- [ ] `p1` - **ID**: `cpt-cf-bss-quotas-fr-lifecycle`

Supported increases, lifecycle cap reductions, renewal, suspend and resume MUST preserve existing allocation evidence and continuing counter identity. A lower cap below occupancy MUST NOT evict resources, clamp counters or invent refunds. Suspension/revocation MUST deny the prohibited actions even when a fallback/default quota exists. Cancel/expiry MUST close the old acquisition against delayed regrant. A terminally closed source MUST reject issuance, increases and resume even under a higher revision. Authorized reads, recovery of original operations and settlement of confirmed resource releases MUST remain possible without restoring access. A new purchase MUST use a distinct source identity and MUST NOT automatically inherit unused allowance, occupied allocations or unsettled effects; source replacement MUST NOT bypass scope-ownership checks. Release evidence MUST remain bound to the original allocation and resource generation; it MUST NOT reduce occupancy of a replacement resource or another acquisition. Numeric zero MUST NOT substitute for a restriction that the provider cannot enforce. Voluntary purchase decreases remain outside the initial scope.

#### Exact validity and financial separation

- [ ] `p1` - **ID**: `cpt-cf-bss-quotas-fr-validity`

Validity MUST have an explicit start and exclusive end with provider-supported boundary semantics. Renewal MUST NOT refill ongoing allocations or an unchanged consumption window. Billing cutoff, credit depletion and refunds MUST be decided by financial/lifecycle owners; they MUST NOT be inferred from quota counters or provider stop time. Preparation charging treatment MUST not waive real resource occupancy.

### 5.5 Visibility and recovery

#### Commercial limits and application status

- [ ] `p1` - **ID**: `cpt-cf-bss-quotas-fr-visibility`

Authorized BSS readers MUST see purchased/desired limits and their accepted-term provenance, the correlated adapter application status, its revision and confirmation time, and any adapter-reported fulfillment discrepancy with reason and evidence. Entitlements MUST expose application state and discrepancies; it MUST NOT collect, persist or calculate a projection of occupied, reserved or remaining quota. Operational quota reads and admission decisions belong to OSS contracts. An adapter-reported inability to fulfill a purchased cap of 100 because of a ceiling of 60 MUST remain a discrepancy without changing the purchase. Ordinary usage of 40 leaving 60 available MUST NOT create such a discrepancy. Unknown or stale application evidence MUST remain explicit and MUST NOT imply successful fulfillment.

#### Durable recovery and ownership

- [ ] `p1` - **ID**: `cpt-cf-bss-quotas-fr-recovery`

Embedded or referenced quota snapshots MUST declare complete-set identity, item count and digest; receivers MUST verify all pages before deriving limits or interpreting missing members as removal. Missing/partial evidence MUST NOT mean an empty quota set. Delivery MUST survive duplicate events, process death, lost responses and reordered revisions without duplicate grant or stale resurrection. Reconciliation MUST preserve commercial intent, identify actual provider outcomes and repair only source-owned records under authorization. An independent operator restriction MUST be reported rather than silently overwritten. An authorized revision durably accepted by the adapter MUST prevent older revisions from newly applying, even if the accepted revision later ends in refusal. Accepted and applied revisions MUST remain distinguishable. Replay of a refused operation MUST return the saved refusal; a new attempt after remediation MUST use a higher source revision and a new operation identity without inventing a purchase. A superseded revision MUST retain unresolved allocation and release evidence for accounting recovery through the responsible owner contract. Definitive application refusal or exhausted automatic retries MUST preserve the unfulfilled commercial obligation and a correlated reason. Entitlements MUST durably hand the problem to its recovery/escalation process. Exhaustion MUST NOT fabricate a definitive adapter outcome, release occupancy, cancel the subscription or trigger a refund. The recovery workflow and authorized commercial remedies belong to their owner designs.

#### Qualification before sale

- [ ] `p1` - **ID**: `cpt-cf-bss-quotas-fr-qualification`

A plan MUST be offered only for a qualified profile with supported identity, units, admission, lifecycle, authority, recovery and readiness behavior. The adapter owner MUST supply evidence for the identified integration version and supported quota combinations. Products MUST retain that evidence and control its availability for new sales. Pricing MUST validate the complete required quota set, including exact units, amounts, scopes and required operations, against the enabled qualification at publication and purchase/change acceptance. Discovery declarations alone MUST NOT authorize sale. Withdrawn qualification MUST prevent new sales while preserving accepted obligations. Physical capacity checks and reservations remain OSS responsibilities; quota compatibility MUST NOT imply reserved hardware. Known incompatibility MUST prevent acceptance/capture under the acquisition contract. Temporary failure of a qualified provider MUST preserve an already accepted obligation and expose pending fulfillment; it MUST NOT fabricate service, automatically cancel the purchase or create another one.

## 6. Non-Functional Requirements

### 6.1 Capability-specific requirements

- [ ] `p1` - **ID**: `cpt-cf-bss-quotas-nfr-isolation`

Writes and recovery reads MUST reject unauthorized tenant/source access on both local and remote SDK paths. Cross-tenant conformance cases MUST produce no mutation or protected state disclosure.

- [ ] `p1` - **ID**: `cpt-cf-bss-quotas-nfr-durability`

Every committed desired revision MUST have durable delivery intent and a recoverable result or an explicitly unresolved finding. Under the fault matrix, retries MUST cause zero extra grants, counter resets or stale regrants. Terminal fences MUST survive all permitted retry horizons.

- [ ] `p1` - **ID**: `cpt-cf-bss-quotas-nfr-operations`

Every enabled profile MUST declare finite request timeouts, concurrency/queue limits, retry budget, observation freshness threshold and escalation deadline. Exceeding a threshold MUST produce a durable actionable finding routed through the Entitlements recovery/escalation contract; overload MUST not change an unknown outcome to success. Qualification MUST record measured behavior against those profile values.

### 6.2 NFR Exclusions

No separate availability or latency promise is introduced for a new quota gear because this package introduces no such gear. Per-profile delivery SLOs require deployment evidence; this document does not invent vendor performance guarantees. Project security, audit and runtime baselines continue to apply.

## 7. Public Library Interfaces

### 7.1 Public API Surface

- [ ] `p1` - **ID**: `cpt-cf-bss-quotas-interface-management`

**Type:** versioned SDK capabilities of the responsible owner components, with their authorized UI/REST projections. **Stability:** proposed extensions. Catalog authoring/read belongs to Products, rules/acceptance to Pricing, purchase/change to Orders and Subscriptions, fulfillment status/reconcile to Entitlements, and runtime counters to QE/provider. Breaking changes require versioned contracts; no independent generic cap-write API is introduced for buyers.

### 7.2 External Integration Contracts

- [ ] `p1` - **ID**: `cpt-cf-bss-quotas-contract-provider`

**Catalog input:** Products reads capability snapshots through the selected discovery adapter SDK. Records use registered GTS metric/unit identities and typed capability schemas as specified in DESIGN §3.3.1. Discovery produces reviewable catalog candidates; it grants no tenant allowance.

**Application direction:** required from the BSS-selected quota application adapter. **Protocol:** versioned platform SDK boundary. Entitlements supplies authorized desired quotas and receives correlated application status, effective-limit evidence and recoverable operation outcomes. The adapter must preserve scope isolation, replay safety and ordering. Selection of the receiving OSS component and downstream management APIs belongs to the adapter design.

## 8. Use Cases

### 8.1 Buy and consume a tenant package

- [ ] `p1` - **ID**: `cpt-cf-bss-quotas-usecase-buy`

**Actors:** catalog owner, seller, buyer, resource user, fulfillment and enforcer actors above. **Precondition:** an empty verified provider scope and qualified profile exist.

1. Products describes vCPU and RAM with supported units and tenant/provider mappings.
2. Seller publishes a plan with 10 vCPU and 20 GB per plan unit; buyer accepts quantity one for the resource tenant.
3. Orders and Subscriptions commit the purchase/preparation operation; Entitlements submits the complete quota set to the selected adapter.
4. The adapter confirms that the complete quota set is applied and available. Entitlements reports fulfillment readiness; Subscriptions activates under its commercial guards without waiting for a VM to be created. A permitted user then requests a VM with 2 vCPU and 4 GB.
5. Admission reserves both dimensions; successful creation leaves 8 vCPU and 16 GB available under the matching measurement policy.
6. Other users in that tenant share the remainder. Another tenant's purchase is unaffected.

**Alternative flows:** rejection frees only safely releasable holds; an uncertain creation retains recovery. Confirmed deletion returns capacity once. Physical capacity shortage can reject creation even with sufficient quota.

### 8.2 Change quantity and recover a partial delivery

- [ ] `p1` - **ID**: `cpt-cf-bss-quotas-usecase-change`

**Actor:** buyer. **Precondition:** active quantity one, 2 vCPU occupied.

An accepted compatible increase changes effective quantity to two and desired cap to 20 vCPU, retaining 2 occupied. If the adapter reports partial application inside OSS, the increase remains pending and the previous safe allowance remains until the complete change is confirmed. A retry recovers the same increase. Publishing a different plan coefficient cannot change this purchase retrospectively. Voluntary downgrades are refused under the initial subscription profile even when occupancy would fit; externally mandated restrictions still preserve and report occupancy.

### 8.3 Restrict and end service

- [ ] `p1` - **ID**: `cpt-cf-bss-quotas-usecase-end`

**Actors:** commerce, fulfillment, operator. **Precondition:** an active purchase with occupied resources.

Subscriptions commits a restriction or terminal posture. The adapter applies the corresponding restrictions across the required OSS systems, including fallback paths. Existing resources are handled through their owner contracts. A resource operation without confirmed release keeps occupancy unresolved. Confirmed generation-specific release settles capacity once. Delayed old grant cannot reopen the acquisition.

## 9. Acceptance Criteria

These business criteria validate the requirements in §5–6. Their `Q-C` labels are local verification references, not additional product requirements. Technical contract guarantees and per-profile evidence are specified in [DESIGN §4.2–4.4](DESIGN.md#42-integration-qualification-gates).

Each enabled offer must satisfy the applicable criteria and have a complete trace from approved catalog terms through the accepted purchase to confirmed quota application. Unsupported offers are refused before commitment; transient delivery failure preserves the purchased obligation and a visible recovery state.

| Criterion | Business acceptance outcome |
| --- | --- |
| Q-C01 Accepted evidence | Purchased rules, units and original quantity remain those accepted by the customer after the catalog or plan changes. Recovery preserves that evidence; missing history never becomes unlimited access. |
| Q-C03 Quantity basis | A per-plan-unit allowance is multiplied once by the accepted/effective plan quantity; a fixed allowance applies once. Selecting several items does not create independent component quantities or duplicate the same allowance. |
| Q-C04 Quota readiness | A purchase of resource limits becomes quota-ready when the complete allowance is confirmed usable, without requiring a VM to be created. Partial or uncertain fulfillment is not readiness. Any authorized preparation usage remains counted through activation: a 100 GiB allowance with 20 GiB allocated has 80 GiB remaining. A draft grants no access; resource readiness and activation remain separate owner decisions. |
| Q-C05 Purchase replay | Repeating the same purchase recovers the same purchased quantity and accepted terms without another allowance, even through another transport attempt. A changed request cannot replace the original purchase silently. |
| Q-C06 Compatible increase | Adding a second compatible plan unit changes a 10-vCPU-per-unit allowance from 10 to 20 vCPU while retaining occupied capacity. Fixed(10 vCPU) remains 10. The original purchase still records one unit and both purchases remain traceable. Repetition adds no extra capacity; unsupported combinations are refused. |
| Q-C07 Suspended purchase and race | A suspended service cannot accept a new addition/increase or queue it for automatic execution. Suspension before purchase commitment prevents new quantity and fulfillment effects. Separately authorized resume is required; an already committed purchase remains in history but cannot override a newer restriction. |
| Q-C11 Tenant isolation | Purchased capacity belongs to the authorized resource owner. Being a seller, payer or acting user does not authorize another tenant's resources or disclose that tenant's quota state during recovery. |
| Q-C12 Allocation lifecycle continuity | Renewal and suspension/resume preserve occupied allocation capacity. A 20-vCPU allowance with 8 vCPU allocated still has 12 available after an authorized resume, absent another restriction; renewal does not refill or duplicate it. Consumption-window behavior is available only under a separately qualified profile. |
| Q-C13 Expiry geometry | An offer's validity is start-inclusive and end-exclusive: access is not extended by a delayed expiry process. A provider that cannot honor that boundary cannot sell the timed profile. |
| Q-C14 Mixed grants | Unsupported combinations of periods, units, expiry or commercial terms are refused before purchase/change. The system does not silently sum limits, choose a maximum or extend validity. |
| Q-C15 Below used | When an authorized restriction reduces capacity below existing allocations, those allocations remain accounted and new prohibited allocations stop. The quota change does not itself evict resources, issue refunds or erase usage. |
| Q-C20 Financial separation | Monetary minimums and exhausted trial credit do not directly change numerical quota amounts. Access restrictions follow separately authorized commercial posture and the provider's supported enforcement behavior. |
| Q-C22 Binding equivalence | Reusing a plan item identifier does not make changed quota terms equivalent. Combining purchased quantities requires compatible quota meaning and applicability, not only matching item names or IDs. |
| Q-C27 Purchased limit and fulfillment discrepancy | The customer retains the purchased limit when a provider restriction prevents full delivery, and sees an explicit fulfillment discrepancy. Ordinary consumption does not change that purchased limit or become a delivery discrepancy: 40 GiB used under a usable 100 GiB allowance is normal consumption, while an external 60 GiB ceiling prevents fulfillment of the purchased 100 GiB. BSS does not need its own usage ledger. |
| Q-C39 Plan packaging | The same product can be offered with different limits in different plans. The accepted plan determines the cap, with one quantity scaling for per-unit rules and one application for fixed rules. Duplicate selections add no allowance; unsupported CPU-only add-ons and voluntary decreases are refused. |
| Q-C44 Shared users | All authorized users in the same resource-owning tenant share the purchased allowance, including concurrent allocations. User-level policies cannot replace or bypass that aggregate tenant limit. |
| Q-C49 Reduction below use | A voluntary downgrade remains unavailable in the initial profile. An externally authorized reduction below allocated capacity preserves that occupancy and blocks prohibited growth without automatically evicting, refilling or refunding. |
| Q-C55 Quota and charging independence | Package, usage-priced and mixed offers can carry the same 20-vCPU allocation ceiling; allocating 4 vCPU occupies the same capacity in each. Accepted charging terms separately identify what is included and chargeable. No usage produces no usage charge for a purely usage-priced component. Exhausting a billing inclusion does not increase the cap or permit bypass; quantity scaling still follows Fixed/PerPlanUnit. Quota application carries no price or paid flag. |

## 10. Dependencies

| Dependency | Required contribution | Criticality |
| --- | --- | --- |
| Products / Pricing | Versioned metric/rule/acceptance extensions and historical retention | p1 |
| Types-registry and discovery adapters | Registered GTS schemas/instances, authenticated capability snapshots and reference validation | p1 |
| Orders / Subscriptions | Exact quantity changes, committed posture and activation resolution | p1 |
| Entitlements | Desired state, adapter operation tracking, delivery, readiness and reconciliation | p1 |
| Quota application adapter | Consistent application, operation recovery and effective-limit evidence across OSS | p1 |
| Resource service, provider adapters and resource authority | Provider occupancy semantics, scope binding and authoritative generation-specific allocation/release evidence | p1 |
| Authorization and platform infrastructure | Scoped SDK access, durable events/tasks, credentials and egress | p1 |

## 11. Assumptions

The initial offer uses one commercial source per resolved quota scope and a bounded allocation profile. OSS can retain several enforcement systems for the same allowance; the adapter coordinates them. Examples use new, empty provider scopes; existing allocations require a separately qualified adoption protocol. Accepted deployment profiles supply exact units, clocks and operating bounds before publication.

## 12. Risks

| Risk | Impact | Required treatment |
| --- | --- | --- |
| Provider API lacks fencing or safe multi-metric behavior | Stale grant or partial entitlement | Block profile until an effect-boundary protocol is demonstrated |
| Provider identity scope differs from platform tenant | Independent scopes receive duplicated caps | Prove aggregate mapping or restrict the offer to one provider scope |
| Admin/provider bypass | QE counters lag actual use | Mediate/disable bypass or qualify an explicit provider authority protocol |
| Unqualified enforcement capability | Selling unsupported limits | Require SDK and deployed-provider conformance before enabling a profile |

## 13. Open Questions

Scope resolution through the selected adapter is a working integration model. The responsible OSS component for resolving and creating a tenant scope remains to be agreed; BSS requires an authorized stable binding and replay safety regardless of that ownership.

The architecture is defined in [DESIGN](DESIGN.md). Deployment qualification must establish exact provider versions and API mappings, multi-metric admission and stale-write fencing, operational bounds, and provider readiness/activation continuity. Each has a qualification gate in [DESIGN §4.2](DESIGN.md#42-integration-qualification-gates); an unresolved capability prevents enabling the affected offer.

## 14. Traceability

[DESIGN §5](DESIGN.md#5-traceability) maps every functional and non-functional requirement to its architectural treatment and the business criteria in [§9](#9-acceptance-criteria) and technical guarantees in [DESIGN §4.3](DESIGN.md#43-contract-conformance-criteria).
