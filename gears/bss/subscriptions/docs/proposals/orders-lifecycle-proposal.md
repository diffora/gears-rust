---
description: "Required Orders Lifecycle changes for protected acquisition, cancellation and financial admission, with examples."
---

# Orders Lifecycle proposal for Subscriptions

**Status:** Required integration contract; owner agreement and executed conformance are required before enablement. See the [proposal index](README.md) for scope and common rules.

<!-- toc -->

- [Baseline and retained behavior](#baseline-and-retained-behavior)
- [Required changes](#required-changes)
- [Examples](#examples)
- [Owner touchpoints and acceptance](#owner-touchpoints-and-acceptance)

<!-- /toc -->

## Baseline and retained behavior

Local owner documents: [PRD](../../../orders-lifecycle/docs/PRD.md), [DESIGN](../../../orders-lifecycle/docs/DESIGN.md), [Gate and Pin](../../../orders-lifecycle/docs/features/03-gate-and-pin.md) and [Workflow seam](../../../orders-lifecycle/docs/features/06-workflow-seam.md). Their baseline uses active-only counts and a pinned-price comparison at activation (D-162: `resolve(revision, activation date, pins)`; D-152 makes no renewal resolve call). This proposal specifies the AP/FA changes to adopt in a separate Lifecycle PR, including the detailed re-check instruction and provisioning request.

Keep Lifecycle as the order system of record, the existing order status enum, immutable order versions, the independent one-in-flight-order constraint, and the existing authoring/approval boundaries. Change Orders authoring remains in its separate PRD.

**Orders Changes admission handoff.** The current [Orders Changes target-eligibility requirement](../../../orders-changes/docs/PRD.md#target-eligibility) says “At submit, the target subscription **MUST** be non-terminal”; its [CHG-S2 upstream ask](../../../orders-changes/docs/PRD.md#upstream-asks-on-subscriptions) repeats target non-terminality at commit. Non-terminality alone cannot authorize a suspended purchase in the current Subscriptions profile. The [accepted purchase-state rule](../features/03-changes.md#apply-one-atomic-changecomposition) already governs Subscriptions; it does not wait for external-owner adoption. OL-P08 below requests the corresponding Orders Changes amendment. Owner acceptance remains OPEN.

This handoff neither edits that owner's PRD nor claims its ratification.

## Required changes

| Request | Previous assumption or gap | Required owner change |
| --- | --- | --- |
| OL-P01 Protected occupancy | Active count plus incoming lines can overlook a suspended right or confirmed reservation | Consume AP02 counts for the authoritative payer/resource/scope key, including active, retained-suspended and reserved rights. Deduplicate represented local claims. A read remains diagnostic; obtain a durable claim before protected capture. |
| OL-P02 Future commercial promise | The order pin and local deadline were treated as sufficient until a fresh pinned resolve | Preserve accepted evidence plus a versioned Pricing/Products future commitment. Ordinary catalog successors do not invalidate it. Distinguish pre-purchase assessment validity from explicit accepted execution constraints. |
| OL-P03 Capture versus cancellation | Authorization before fulfillment does not order actual debit against cancellation | Serialize the capture-start/cancel decision for the acquisition/version, commit its durable dispatch identity and outbox, and let Workflow deliver and reconcile it and Payments execute it once per capture identity. Protect every funded line first. |
| OL-P04 Final admission | Tolerated authorization failure may be read as permission for paid service | Keep it a process decision. Subscriptions independently consumes FA evidence: prescribed payment for prepay, accepted credit-service authorization for postpay. |
| OL-P05 Outcome and release | Draft void, TTL or terminal order state may appear to finish compensation | Track claim/commitment/capture/remedy outcomes separately. Unknown financial or application outcomes retain protection; release needs authoritative fenced settlement. |
| OL-P06 Read and receipt contracts | One deadline and requested start do not represent actual application | Expose versioned protection evidence and immutable accepted-version references to authorized consumers. Requested dates are lower bounds; Subscriptions returns actual applied time. Preserve the committed spawn-signal boundary. |
| OL-P07 Exact order evaluation | Current local totals/TCV contracts still describe integer minor units | Adopt [the money ADR](../../../docs/ADR/0001-cpt-cf-bss-adr-exact-decimal-boundaries.md): consume Rating's exact amount plus major-unit display estimate, currency and scale. Version the DTO/storage contract; display totals never become capture instructions. Propagate the selected TCV representation to Workflow/approval without recomputation. |
| OL-P08 Orders Changes purchase admission | Orders Changes PRD §5.1, §6.1 Target Eligibility and §9.2 CHG-S2 use non-terminality without excluding suspended purchases | Orders Changes MUST require refusal of suspended seat purchases, quantity increases and component additions before effects at submission and again in the Subscriptions commit transaction (SUB-D-39). Require a separate authorized guarded resume, then a normal active change with the current expected revision; no automatic resume or queued/deferred purchase. Preserve the separate S-U01 draft gate. If suspension wins before commit, refuse the unapplied purchase and recover staged work; if purchase committed first, preserve its history and reconcile delivery under the newer suspension fence. Renewal and unrelated administrative recovery retain their own contracts. |

## Examples

**Future paid start.** A customer buys service for 1 November at EUR 20. Before capture, the line has one durable capacity claim and original-term commitment covering that start. The catalog changes to EUR 25 in October. Activation uses the accepted EUR 20 terms under current order, financial, security and readiness guards. If future commitment is unavailable, do not capture on the promise that a later price comparison will pass.

**Reserved capacity is not counted twice.** Limit is one, active count is zero and this order already owns the one confirmed reservation. Its activation converts that right to active; it does not calculate reservation one plus incoming one and reject. A second unrelated acquisition refuses. Suspension keeps the first right.

**Cancel races capture.** If cancellation wins before capture-start, delayed debit must be fenced at Payments. If capture-start won and the response is lost, cancellation stops activation and opens financial recovery; deleting a draft proves nothing about the debit. The agreed remedy is recorded without inventing a refund amount or deadline.

**Money is carried, not recalculated.** A Rating display estimate of EUR 12.34 travels as a decimal amount in major units with scale 2 and its exact counterpart. Orders does not reinterpret a bare integer 1234, round again, or use the estimate as the funding obligation.

## Owner touchpoints and acceptance

Agree versioned order evidence, read/receipt contracts and financial authority across Lifecycle, Workflow and Subscriptions. Owner document and implementation changes belong in the Lifecycle PR.

Acceptance uses [AP-C01 through AP-C12](../ACTIVATION_PROPOSAL.md#13-orders-and-pricing-reciprocal-activation-contract), [FA cases](../DESIGN.md#3311-financial-admission-integration-contract), and an exact/display amount round-trip through Rating → Lifecycle → Workflow. Prove no double claim, no late debit after a winning cancel, no loss of paid protection after TTL/rebuild, and original terms after catalog drift. Provider schemas, clock/fence mappings and runtime tests remain open.

For OL-P08, the Orders Changes owner must adopt the purchase-state rule and demonstrate [V06](../DECOMPOSITION.md#31-cross-contract-acceptance) and the [change acceptance cases](../features/03-changes.md#6-acceptance-criteria). External acceptance and executed evidence remain open; they do not suspend the already accepted local refusal rule or introduce a synchronous entitlement-readiness prerequisite.

Preparation timeout closes only the Subscriptions attempt through its activation fence. Preserve the order purchase, accepted terms, protected claim and financial basis for remediation; timeout alone cannot cancel the acquisition or create a refund. A successor requires the existing predecessor-settlement and association protocol with safe external-effect/cleanup fencing. Explicit order cancellation and financial remedy retain their separate eligibility and durable handoffs.

Known provider/profile restrictions incompatible with the offered scope must prevent acceptance/capture of that unsupported purchase. This uses the agreed capability/constraint evidence, not a universal synchronous provider-health check on every order. A restriction discovered after acceptance retains the original obligation and enters remediation; neither the provider's lower cap nor its observation silently changes accepted terms or triggers a refund.

Required re-check outcomes are pending for missing/unknown evidence, refusal for a definitive unmet accepted execution condition, and replay for an already applied attempt. Superseded attempts stop forward dispatch but retain application/capture/remedy recovery. Terminal order status, retry exhaustion or draft void cannot release a paid claim. The AP scenarios above cover these outcomes.
