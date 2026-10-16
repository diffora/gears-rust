---
description: "Required Orders Lifecycle changes for protected acquisition, cancellation and financial admission, with examples."
---

# Orders Lifecycle proposal for Subscriptions

**Status:** Subscriptions integration proposal, 2026-10-15. This is an owner handoff, not a delivered API or external-owner approval. Examples are illustrative, not new tariff, duration or SLA defaults. See the [proposal index](README.md) for source precedence, scope exclusions and delivery order.

<!-- toc -->

- [Baseline and retained behavior](#baseline-and-retained-behavior)
- [Required changes](#required-changes)
- [Examples](#examples)
- [Owner touchpoints and acceptance](#owner-touchpoints-and-acceptance)
- [Detailed adoption checklist](#detailed-adoption-checklist)

<!-- /toc -->

## Baseline and retained behavior

Local owner documents: [PRD](../../../orders-lifecycle/docs/PRD.md), [DESIGN](../../../orders-lifecycle/docs/DESIGN.md), [Gate and Pin](../../../orders-lifecycle/docs/features/03-gate-and-pin.md) and [Workflow seam](../../../orders-lifecycle/docs/features/06-workflow-seam.md). Those documents are unchanged in this PR. Their baseline uses active-only counts and a renewal-price comparison at activation. This proposal specifies the AP/FA changes to adopt in a separate Lifecycle PR, including the detailed re-check instruction and provisioning request.

Keep Lifecycle as the order system of record, the existing order status enum, immutable order versions, the independent one-in-flight-order constraint, and the existing authoring/approval boundaries. Change Orders authoring remains in its separate PRD.

## Required changes

| Request | Previous assumption or gap | Required owner change |
| --- | --- | --- |
| OL-P01 Protected occupancy | Active count plus incoming lines can overlook a suspended right or confirmed reservation | Consume AP02 counts for the authoritative payer/resource/scope key, including active, retained-suspended and reserved rights. Deduplicate represented local claims. A read remains diagnostic; obtain a durable claim before protected capture. |
| OL-P02 Future commercial promise | The order pin and local deadline were treated as sufficient until a fresh pinned resolve | Preserve accepted evidence plus a versioned Pricing/Products future commitment. Ordinary catalog successors do not invalidate it. Distinguish assessment deadline, committed future window and final bounded permit. |
| OL-P03 Capture versus cancellation | Authorization before fulfillment does not order actual debit against cancellation | Serialize the capture-start/cancel decision for the acquisition/version, commit its durable dispatch identity and outbox, and let Workflow/Payments execute and recover it. Protect every funded line first. |
| OL-P04 Final admission | Tolerated authorization failure may be read as permission for paid service | Keep it a process decision. Subscriptions independently consumes FA evidence: prescribed payment for prepay, accepted credit-service authorization for postpay. |
| OL-P05 Outcome and release | Draft void, TTL or terminal order state may appear to finish compensation | Track claim/commitment/capture/remedy outcomes separately. Unknown financial or application outcomes retain protection; release needs authoritative fenced settlement. |
| OL-P06 Read and receipt contracts | One deadline and requested start do not represent actual application | Expose versioned protection evidence and immutable accepted-version references to authorized consumers. Requested dates are lower bounds; Subscriptions returns actual applied time. Preserve the committed spawn-signal boundary. |
| OL-P07 Exact order evaluation | Current local totals/TCV contracts still describe integer minor units | Adopt [the money ADR](https://github.com/constructorfabric/gears-rust/blob/510ce99f037a19f63d41b17acb3b0680e9331a44/gears/bss/docs/ADR/0001-cpt-cf-bss-adr-exact-decimal-boundaries.md): consume Rating's exact amount plus major-unit display estimate, currency and scale. Version the DTO/storage contract; display totals never become capture instructions. Propagate the selected TCV representation to Workflow/approval without recomputation. |

## Examples

**Future paid start.** A customer buys service for 1 November at EUR 20. Before capture, the line has one durable capacity claim and original-term commitment covering that start. The catalog changes to EUR 25 in October. Activation with a valid final permit uses the accepted EUR 20 terms. If future commitment is unavailable, do not capture on the promise that a later price comparison will pass.

**Reserved capacity is not counted twice.** Limit is one, active count is zero and this order already owns the one confirmed reservation. Its activation converts that right to active; it does not calculate reservation one plus incoming one and reject. A second unrelated acquisition refuses. Suspension keeps the first right.

**Cancel races capture.** If cancellation wins before capture-start, delayed debit must be fenced at Payments. If capture-start won and the response is lost, cancellation stops activation and opens financial recovery; deleting a draft proves nothing about the debit. The agreed remedy is recorded without inventing a refund amount or deadline.

**Money is carried, not recalculated.** A Rating display estimate of EUR 12.34 travels as a decimal amount in major units with scale 2 and its exact counterpart. Orders does not reinterpret a bare integer 1234, round again, or use the estimate as the funding obligation.

## Owner touchpoints and acceptance

Update PRD payment/date/occupancy requirements; DESIGN acquisition state, read DTOs and total storage; features 03, 05 and 06; upstream requests; and the decision register. Preserve historical decisions with explicit supersession. All AP/FA and numeric changes require owner adoption in a separate PR.

Acceptance uses [AP-C01 through AP-C12](../ACTIVATION_PROPOSAL.md#13-orders-and-pricing-reciprocal-activation-contract), [FA cases](../DESIGN.md#3311-financial-admission-integration-contract), and an exact/display amount round-trip through Rating → Lifecycle → Workflow. Prove no double claim, no late debit after a winning cancel, no loss of paid protection after TTL/rebuild, and original terms after catalog drift. Provider schemas, clock/fence mappings and runtime tests remain open.

## Detailed adoption checklist

Apply these changes together in the Lifecycle owner PR. A top-level AP notice alone is insufficient if the detailed algorithm or caller table still requires the legacy check.

| Document / contract | Required change |
| --- | --- |
| PRD dates, overlap and payment requirements; assumptions and risks | Use requested dates as lower bounds and the Subscriptions applied receipt as actual T. Count active, retained-suspended and confirmed-reserved rights once. Separate authorization/tolerance, capture and final financial admission. State unsupported AP capability as a release gate rather than assume legacy resolve can honor the promise. |
| DESIGN acquisition and payment ordering | Retain per-line protection, the funded roster, capture decision/generation and dispatch identity independently of order status. Serialize capture-start versus cancellation with a local outbox; owner reads and network work remain outside the transaction. |
| DESIGN accepted order evidence and composed read | Expose immutable order/version, admission profile/version, accepted-term digest, claim and commitment references. Distinguish legacy assessment deadline, accepted future window and final permit. Assessment totals remain frozen indicative evidence, not funding instructions. |
| DESIGN initial activation and reference protection | Replace D-162 renewal-price comparison for the AP profile with accepted evidence plus final permit. Preserve original price/SKU/policy/invoice evidence; later catalog descriptors cannot redefine acceptance. D-164 reference retention alone is not proven customer commitment. Pin subsequent renewal separately. |
| Feature 03 gate and detailed activation re-check | Apply AP authority in the actual numbered check, not only its summary. A catalog successor or expired assessment deadline cannot invalidate a protected future window. Recover an uncertain prior application before any new eligibility decision. |
| Feature 05 preconditions | Keep begin-fulfillment authorization/tolerated failure as process evidence. Prescribed prepayment or authorized postpay service evidence remains an independent final gate. |
| Feature 06 Workflow seam and UPSTREAM_REQS | Pass accepted-version/protection references and requested lower bound; obtain committed spawn signal before activation dispatch. Require applied/replayed receipt for success. Remove the legacy pinned-comparison request from the provisioning contract for this profile. |
| DECISIONS | Record scoped supersession of D-162 and the insufficient guarantee of D-164, preserving history and the unchanged order enum. Do not claim owner ratification from this proposal. |

The re-check needs explicit outcomes: missing/unknown dependency evidence defers with a versioned dependency reason; definitive unused final-authority expiry refuses new application while preserving paid recovery; an applied attempt replays its result. Superseded attempts stop dispatch and require fenced settlement or supported successor association, with no implicit payment transfer. Terminal order state stops forward work but still reconciles capture/application/remedy. Retry-budget exhaustion and draft void do not release a paid claim.

**Required regression example.** The future commitment covers 1 November but the assessment deadline was 16 October. The detailed step must validate the commitment and final permit on 1 November. Reusing the old `activation_deadline` check or running renewal resolve after an ordinary catalog successor would incorrectly refuse a protected paid purchase. A genuinely expired final permit instead stops new application and invokes AP recovery, without pretending payment or resource work never happened.
