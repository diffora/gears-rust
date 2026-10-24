---
description: "Inbound requirements from Subscriptions and the provider prerequisites of the Entitlements application layer, with ownership and acceptance evidence."
---

# UPSTREAM_REQS — Entitlements

<!-- toc -->

- [1. Overview](#1-overview)
  - [1.1 Purpose](#11-purpose)
  - [1.2 Requesting gears](#12-requesting-gears)
- [2. Requirements](#2-requirements)
  - [2.1 Subscriptions](#21-subscriptions)
  - [2.2 Pricing and Products](#22-pricing-and-products)
  - [2.3 Enforcement providers](#23-enforcement-providers)
  - [2.4 Resource provider and operators](#24-resource-provider-and-operators)
  - [2.5 Provider prerequisites and bilateral acceptance](#25-provider-prerequisites-and-bilateral-acceptance)
- [3. Priorities](#3-priorities)
- [4. Traceability](#4-traceability)

<!-- /toc -->

## 1. Overview

### 1.1 Purpose

This register names the requirements consumers place on Entitlements and the provider work those requirements depend on. Every incoming requirement below is covered by the [PRD](PRD.md); implementation belongs to [DESIGN](DESIGN.md) and [features](DECOMPOSITION.md). TARGET means proposed documentation, not delivered code or another owner's approval. Owner handoffs for each provider are in the [proposals](proposals/README.md).

### 1.2 Requesting gears

| Consumer | Needs from Entitlements |
| --- | --- |
| Subscriptions | Fenced intake of `CommercialPostureV1`; application of every committed revision to the enabled providers; `ReadinessObservationV1` for every required assignment of an activation revision; materialization state and findings readable per posture revision |
| Pricing | The versioned semantic schema of quota and grant rules it stores on PlanRevision and freezes in acceptance |
| Products | The metric, unit and subject semantics the schema needs and the requirement for a versioned mapping to enforcement metrics |
| Quota Enforcement, License Enforcement or the selected license adapter, resource provider | A trusted, authorized projector of committed accepted state with stable source keys, monotonic revisions, digests and recovery by original identity |
| Platform operator | Desired versus applied state, findings and authorized repair without direct edits of provider records |

## 2. Requirements

### 2.1 Subscriptions

#### Commercial posture

- [ ] `p1` - **ID**: `cpt-cf-bss-entitlements-upreq-commercial-posture`

Entitlements MUST accept `CommercialPostureV1` as published by Subscriptions from the same transaction as its lifecycle change, through the Subscriptions outbox and through its recovery reads. The payload carries identity `(orderingTenantId, subscriptionId, revision)`, a payload digest and an operation id; the resource, payer and seller tenant axes; one member per logical component with capability/product references, quantity as the sum of effective lots, validity interval, posture `Granted | Restricted | Revoked` and holder scope; and provenance as accepted snapshot references (acceptance id, rule/binding snapshot digests) and contributing lot ids. Entitlements MUST authenticate the producer and the tenant scope before acceptance.

Entitlements MUST fence every delivery against its intake journal: a lower revision is stale and refused; equal revision and equal digest is a replay and returns the existing result; equal revision and different digest is a conflict, refused and visible; a higher revision supersedes the target while earlier outcomes are retained. Entitlements MUST apply each accepted revision to every enabled capability as one complete desired state per provider source key, with explicit tombstones, under the fenced apply contract of [DESIGN §3.3.4](DESIGN.md#334-quota-apply-and-read). Lots are provenance only; Entitlements MUST NOT re-derive quantity from lots, and MUST read rules from the accepted snapshot the provenance names, never from a live plan. Entitlements MUST report applied, pending, refused or unknown materialization per revision and MUST NOT report success because an adapter accepted a dispatch.

- **Source**: Subscriptions [UPSTREAM_REQS S-U04](../../subscriptions/docs/UPSTREAM_REQS.md#25-provider-prerequisites-and-bilateral-acceptance) and [DESIGN §3.3.5](../../subscriptions/docs/DESIGN.md#335-entitlement-and-retention-contracts); the split design of 2026-10-23.
- **Status / owner**: TARGET, Entitlements. Contract version JOINT OPEN with Subscriptions under E-U06.
- **Acceptance**: A replayed posture is a no-op; a conflicting digest is refused and visible; a posture lost in transit is recovered by revision from the Subscriptions recovery read; a revision 7 apply arriving after revision 8 revoked cannot restore access at any provider.

#### Readiness gate

- [ ] `p1` - **ID**: `cpt-cf-bss-entitlements-upreq-readiness-gate`

For an activation revision Entitlements MUST build and persist the readiness manifest of required assignments and producer authorities before dispatch, dispatch the preparation assignments while the subscription is still draft, and publish `ReadinessObservationV1` observations with the envelope of [DESIGN §3.3.2](DESIGN.md#332-readiness-observation-contract): `schemaVersion`, `scopeId`, `assignmentId`, `desiredRevision`, `serviceProfileRef`, `targetRef`, `eventId`, `producerId`, `statusSequence`, `status` (`pending | ready | failed | withdrawn`), `observedAt`, `evidenceRef`, `reasonCode`. No subscription identifier crosses this boundary; Subscriptions keeps the association from its operation to the assignment. Entitlements MUST authenticate every producer against the saved manifest, accept observations in sequence order with a durable high-water mark per producer stream, and report `ready` for the manifest only when every required assignment is ready for the exact `desiredRevision`. This is the provider side of ENT-D-02.

Subscriptions waits: its activation operation stays pending or approved and the subscription stays draft until every assignment in the manifest reports `ready` for the exact desired revision; the commit then rechecks fences and writes active, T, receipt and outbox atomically. Entitlements MUST NOT treat receipts, broker acknowledgement or license existence as readiness, MUST NOT let one producer's ready satisfy another required producer or assignment, and MUST honor withdrawal: a delayed `ready` for a withdrawn revision cannot activate and still contributes historical evidence for cleanup.

- **Source**: Subscriptions [initial activation readiness contract](../../subscriptions/docs/features/05-entitlements.md#initial-activation-readiness-contract) (IA-2026-10-18, SUB-D-55); the split design of 2026-10-23.
- **Status / owner**: TARGET, Entitlements. Envelope JOINT OPEN with Subscriptions under E-U06; freshness/withdrawal arbitration, cleanup policy and preactivation exposure are `ENT-Q11` to `ENT-Q13` in [DECISIONS](DECISIONS.md).
- **Acceptance**: A lost ready event is recovered from the producer's authoritative read; a ready racing a withdrawal cannot activate; a wrong-tenant observation is refused without information leakage; an older sequence cannot overwrite a newer ready, withdrawn or failed result.

### 2.2 Pricing and Products

Pricing stores the rule block Entitlements defines and freezes it in the acceptance receipt the posture provenance names; Products supplies the metric, unit and subject semantics that block references. Neither places a MUST on Entitlements beyond publishing a versioned semantic schema and keeping its meaning stable per version; their own obligations are the E-U03 and E-U04 gates below and the pricing rule and products metric proposals in the [proposals index](proposals/README.md).

### 2.3 Enforcement providers

Quota Enforcement and the selected license adapter require Entitlements to be a trusted, authorized projector of committed accepted state: commands bound to the resource tenant, subject membership proved through the resource authority, source ownership enforced by the provider, credentials that do not authorize uncontrolled customer writes, and authorization on recovery reads as well as writes. Entitlements never adopts or deactivates an operator record or another source's record by matching subject and metric. Their reciprocal obligations are E-U01 and E-U02.

### 2.4 Resource provider and operators

The resource provider requires work orders with operation identity and immutable resource generation, and retry by original identity. Platform operators require desired versus applied state, findings and an authorized repair route that uses only conditional provider operations; Entitlements never edits a license, counter or resource directly.

### 2.5 Provider prerequisites and bilateral acceptance

These are requests to other owners, not assertions of their implementation. The incoming requirements above remain the traceable product obligations. For each owner integration record the accepted version, producer and consumer, adapter mapping, refusal for unsupported capabilities and the executed fixture reference.

| ID | Owner | Required contract / status | Closure signal | Blocks |
| --- | --- | --- | --- | --- |
| E-U01 | Quota Enforcement | Versioned `apply_source_state(source_key, source_revision, digest, complete_target, effective_boundary, authority)` and authorized `read_source_state(source_key)` with durable operation recovery: durable authenticated source key `(namespace, resource tenant, assignmentId)`, uniqueness/ownership constraints, revision/digest compare-and-apply, terminal tombstones and recoverable results (existing create has no idempotency key, update has no expected version); recovery of applied outcomes by the stable key; demonstrated hard enforcement, suspension/revocation with fallback, counter preservation and isolation from operator and other-source records; explicit exclusive-end windows, immutable continuing counter identities, cap-below-used behavior and no reset on replay/update/renewal; the required storage/client/outbox runtime with end-to-end tests. EXTERNAL OPEN; quota enforcement proposal in the [proposals index](proposals/README.md). | A delayed revision 7 apply is rejected against the applied revision 8 head after timeout, restart or lease loss; a lost-response apply is recovered by source key without a second record; readback exposes the applied head. | Quota projection; the hard-limit profile |
| E-U02 | License Enforcement, native License Manager or third-party adapter | Accept desired grant state by scope, assignment, desired revision and generation without a subscription identifier; stale-action suppression at the effect boundary; idempotent recovery of the same operation after a lost response; preparation grants in draft reported as readiness observations. EXTERNAL OPEN; license enforcement proposal in the [proposals index](proposals/README.md). | An old issue cannot undo a newer revoke; pending versus actual application is recoverable; the readiness envelope is produced for every required assignment. | License grants; license-dependent initial readiness profiles |
| E-U03 | Pricing | Optional typed rule block on PlanRevision beside the trial block, validated at publication against Products evidence, frozen in `AcceptanceReceipt` with the bindings; replay reads the receipt, never the live plan. EXTERNAL OPEN; pricing rule proposal in the [proposals index](proposals/README.md). | A changed quota term requires a new accepted revision; the accepted snapshot resolves every rule and binding by digest. | Any sale carrying quota or grant rules |
| E-U04 | Products | Metric, unit and subject-kind semantics; registration of enforcement metrics as GTS instances; versioned mapping from Products references to QE `MetricId` and `projection_type`. A `usage_type_ref` is not automatically a `MetricId`. EXTERNAL OPEN; products metric proposal in the [proposals index](proposals/README.md). | Every rule in an accepted snapshot resolves to a registered enforcement metric with unit and subject semantics. | Quota members for the affected metrics |
| E-U05 | Resource provider | Physical outcomes correlated to work order and resource generation with actual effective time and explicit pending/unknown/partial; non-destructive suspension; retention/export under frozen disposition terms; hold/delete arbitration at the execution boundary; retry by original identity, per-resource partial completion, no action on a recycled resource name. EXTERNAL OPEN; resource provider proposal in the [proposals index](proposals/README.md). | Adapter mapping ratified and the late stop, hold racing deletion and reused name cases pass; automatic deletion stays disabled without the proven hold/execution fence. | Resource execution |
| E-U06 | Subscriptions | Subscriptions publishes `CommercialPostureV1` from its outbox and recovery reads and consumes `ReadinessObservationV1`; versions agreed bilaterally. JOINT OPEN. | Both gears cite the same contract versions; the replay, conflict, lost-delivery, lost-ready and ready-racing-withdrawal fixtures pass on both sides. | Posture intake; readiness reporting; every activation profile that depends on initial readiness |

An alternative provider may satisfy E-U01, E-U02 or E-U05 without the native system, but it must demonstrate the same selected-profile guarantees. No direct database writes to any provider are allowed. Broker transport is optional for recovery correctness; recovery reads are not.

## 3. Priorities

Both incoming requirements are `p1`: they gate the first activation profile that depends on initial readiness and every posture application. An implementation can develop against contract fixtures, but mocks never close an external production gate. [DECOMPOSITION](DECOMPOSITION.md) allocates each active PRD requirement to one primary feature and names its prerequisites.

## 4. Traceability

- [PRD](PRD.md): `Covers` links map the two incoming requirement ids to product requirements.
- [DESIGN](DESIGN.md): architecture and contract authority; [§4.1](DESIGN.md#41-capability-gates) lists what each open gate blocks.
- [DECISIONS](DECISIONS.md): `ENT-D-01` to `ENT-D-03` and open questions `ENT-Q01` to `ENT-Q13`.
- [proposals](proposals/README.md): owner handoffs for E-U01 to E-U05.
