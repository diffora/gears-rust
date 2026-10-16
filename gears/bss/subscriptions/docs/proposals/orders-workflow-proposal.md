---
description: "Changes against Workflow PR 4787 for protected acquisition, compensation, financial evidence and money transport."
---

# Orders Workflow proposal for Subscriptions

**Status:** Subscriptions integration proposal, 2026-10-15. This is an owner handoff, not a delivered API or external-owner approval. Examples are illustrative, not new tariff, duration or SLA defaults. See the [proposal index](README.md) for source precedence, scope exclusions and delivery order.

<!-- toc -->

- [Source and what is already present](#source-and-what-is-already-present)
- [Required changes against the reviewed design](#required-changes-against-the-reviewed-design)
- [Examples](#examples)
- [Acceptance and implementation boundary](#acceptance-and-implementation-boundary)
- [Abort and recovery branches](#abort-and-recovery-branches)

<!-- /toc -->

## Source and what is already present

Reviewed [PR #4787](https://github.com/constructorfabric/gears-rust/pull/4787) at `ce16d8ff2f59e674ca5e36b091588781022348ea`. Its detailed design is not imported into this branch. Use the pinned sources below when applying this proposal to that branch, not the older local PRD alone.

Already designed: two-wave fulfillment, Subscriptions-owned actual activation time (D-209), committed `report-spawn-signal` before activation, durable invocation recovery, and Lifecycle as the sole evaluator of authorization-failure tolerance. Preserve those mechanisms. They are documented capabilities, not proof of a running provider.

## Required changes against the reviewed design

| Request | Source touchpoint | Proposed delta |
| --- | --- | --- |
| OW-P01 Versioned commercial authority | [04 §2.2 and §3.7](https://github.com/constructorfabric/gears-rust/blob/ce16d8ff2f59e674ca5e36b091588781022348ea/gears/bss/orders-workflow/docs/design/04-fulfillment-plan.md), [05 dispatch guard](https://github.com/constructorfabric/gears-rust/blob/ce16d8ff2f59e674ca5e36b091588781022348ea/gears/bss/orders-workflow/docs/design/05-provisioning-intents.md) | Replace unconditional `activation_deadline_at` abort for the AP profile with distinct assessment, commitment and final-permit evidence. Keep valid expiry checks, but never let an old assessment deadline shorten protected future coverage. Apply consistently at freeze, re-check and every dispatch. |
| OW-P02 Protection/capture execution | [04 payment gate](https://github.com/constructorfabric/gears-rust/blob/ce16d8ff2f59e674ca5e36b091588781022348ea/gears/bss/orders-workflow/docs/design/04-fulfillment-plan.md), [10 protected process ordering](https://github.com/constructorfabric/gears-rust/blob/ce16d8ff2f59e674ca5e36b091588781022348ea/gears/bss/orders-workflow/docs/design/10-process-definition.md) | Add durable steps that deliver Lifecycle's capture decision only after the funded roster has capacity and future-term protection. Recover Payments by the original capture identity. Keep this independent of the existing spawn signal. |
| OW-P03 Final paid-service evidence | [04 authorization path](https://github.com/constructorfabric/gears-rust/blob/ce16d8ff2f59e674ca5e36b091588781022348ea/gears/bss/orders-workflow/docs/design/04-fulfillment-plan.md) | Retain conclusive failed outcomes for Lifecycle's tolerance decision, as already specified. Separately carry/recover FA service-admission references; process eligibility and authorization freshness are not final paid permission. Support postpay without a card/capture step. |
| OW-P04 Draft rebuild and successor attempt | [05 rebuild and reconciliation](https://github.com/constructorfabric/gears-rust/blob/ce16d8ff2f59e674ca5e36b091588781022348ea/gears/bss/orders-workflow/docs/design/05-provisioning-intents.md) | Preserve acquisition, claim, commitment, capture and benefit identities across a rebuilt draft. A new technical attempt requires fenced predecessor non-application and exclusive association; recover an already applied predecessor instead of creating another subscription. |
| OW-P05 Compensation completeness | [06 compensation branch table](https://github.com/constructorfabric/gears-rust/blob/ce16d8ff2f59e674ca5e36b091588781022348ea/gears/bss/orders-workflow/docs/design/06-saga-and-compensation.md), [08 cancellation](https://github.com/constructorfabric/gears-rust/blob/ce16d8ff2f59e674ca5e36b091588781022348ea/gears/bss/orders-workflow/docs/design/08-hold-and-cancel.md) | `absent/already-absent` can finish the draft leg only. Retain separate capacity, commercial, payment and remedy recovery until authoritative settlement. Operator-forced terminal status must not erase this work. |
| OW-P06 Process-definition coverage | [10 routing and protected steps](https://github.com/constructorfabric/gears-rust/blob/ce16d8ff2f59e674ca5e36b091588781022348ea/gears/bss/orders-workflow/docs/design/10-process-definition.md) | Update the protected-step validator, retry/hold/supersede paths, cause mapping and completion predicates for the new evidence. A custom process definition cannot bypass protection-before-capture or turn unknown into settled. |
| OW-P07 Approval money | [03 approval request and storage](https://github.com/constructorfabric/gears-rust/blob/ce16d8ff2f59e674ca5e36b091588781022348ea/gears/bss/orders-workflow/docs/design/03-approval-execution.md) | Replace `tcv_minor` transport/storage with the versioned major-unit representation from Lifecycle/Rating under [the money ADR](https://github.com/constructorfabric/gears-rust/blob/510ce99f037a19f63d41b17acb3b0680e9331a44/gears/bss/docs/ADR/0001-cpt-cf-bss-adr-exact-decimal-boundaries.md). Preserve exact/display/posting-estimate distinctions; agree which declared value approval thresholds consume. Do not recompute or silently choose a different threshold basis. |

## Examples

**Future window survives a legacy deadline.** An order's assessment deadline is 16 October, while its separate committed start window includes 1 November. On 1 November, Workflow verifies the AP evidence and final admission; it must not take the legacy unconditional `bindingExpired` unwind solely because 16 October passed. Genuine final-permit expiry still stops new application and retains recovery obligations.

**Draft disappears after payment.** Payment succeeded but the original draft expired before fulfillment. The compensation table may report the draft as absent. The order still owns a paid acquisition and its protection. Workflow recovers/rebinds that acquisition under AP07 or performs the agreed cancellation/remedy; it does not report all obligations settled from draft absence.

**Lost spawn-signal response.** Lifecycle committed the signal, but Workflow did not receive the response. Recover that same signal before dispatch. This behavior is already in #4787; the proposal adds proof that capture/financial recovery cannot be skipped on the same path.

**Postpay.** Accepted credit-service terms authorize service and no immediate debit is required. Workflow follows the applicable process route and passes final financial evidence to Subscriptions. It does not wait for a card authorization that this settlement scheme does not require.

## Acceptance and implementation boundary

Ratify the concrete Lifecycle/Subscriptions/financial SDK mappings, then update slices 03–06, 08 and 10, their table/operation definitions, PRD, upstream requests and decisions together. Keep D-209 actual-time ownership and existing spawn ordering unchanged.

Execute [AP](../ACTIVATION_PROPOSAL.md#13-orders-and-pricing-reciprocal-activation-contract) and [FA](../DESIGN.md#3311-financial-admission-integration-contract) scenarios through the real process-definition engine, including worker death, duplicate dispatch, held/resumed instances, supersession and forced terminal state. A proposed step name is not a new public API until its owner versions it. General process-engine redesign and Change Orders authoring are outside this proposal.

## Abort and recovery branches

Scope the complete legacy overlap-abort branch to a definitively unprotected acquisition: the overlap decision, stop-further-dispatch action, draft void, `overlap-collision` cause and `in_fulfillment → fulfillment_failed` acknowledgement. A declaration near the top of the document must not leave later unconditional abort steps applying to protected acquisitions.

For protected acquisition, validate AP02 own-claim evidence and use AP07 recovery. An owned reservation is not a collision with itself; unknown occupancy or a missing claim read is not proof of an unprotected collision. Keep claim, commitment and financial obligations until authoritative application/capture recovery, effect-owner fencing and the required remedy allow coordinated release. Draft void may finish only its own leg. If the final operational status is failed, this does not erase remaining financial work.

**Example.** Limit one is fully occupied by this order's reservation. The occupancy service times out on re-check after payment. Workflow defers and recovers that exact reservation; it must not emit `overlap-collision`, void the draft and release the customer's paid place merely because the count could not be read.
