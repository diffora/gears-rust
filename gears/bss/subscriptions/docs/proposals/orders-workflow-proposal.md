---
description: "Workflow integration contracts for protected acquisition, compensation, financial evidence and money transport."
---

# Orders Workflow proposal for Subscriptions

**Status:** Required integration contract; owner agreement and executed conformance are required before enablement. See the [proposal index](README.md) for scope and common rules.

<!-- toc -->

- [Ownership and required behavior](#ownership-and-required-behavior)
- [Integration boundaries](#integration-boundaries)
- [Required integration contracts](#required-integration-contracts)
- [Examples](#examples)
- [Acceptance and implementation boundary](#acceptance-and-implementation-boundary)
- [Abort and recovery branches](#abort-and-recovery-branches)
- [Operation identity and recovery proposal](#operation-identity-and-recovery-proposal)
  - [Identity and lookup](#identity-and-lookup)
  - [Absence, expiry and settlement](#absence-expiry-and-settlement)
  - [Retention and late delivery](#retention-and-late-delivery)
  - [Examples and acceptance cases](#examples-and-acceptance-cases)
  - [Owner responsibilities](#owner-responsibilities)
- [Approval valuation basis proposal](#approval-valuation-basis-proposal)
- [BSS Approvals integration and assignee selection](#bss-approvals-integration-and-assignee-selection)
- [Payment interaction and workflow waiting proposal](#payment-interaction-and-workflow-waiting-proposal)

<!-- /toc -->

## Ownership and required behavior

[Orders Lifecycle](../../../orders-lifecycle/docs/DESIGN.md) owns order state and transition authority. [Orders Workflow](../../../orders-workflow/docs/PRD.md) orchestrates fulfillment, approval, waiting and recovery. The [activation design](../ACTIVATION_PROPOSAL.md#13-orders-and-pricing-reciprocal-activation-contract) and [financial contract](../DESIGN.md#3311-financial-admission-integration-contract) define the required Subscriptions integration.

Fulfillment MUST use two waves, a common requested-date barrier, a committed `report-spawn-signal` before activation dispatch, and durable per-line invocation recovery. Subscriptions owns actual activation time; Workflow completes a line only on the authoritative `applied` outcome. Lifecycle alone evaluates authorization-failure tolerance. Operational cancellation is distinct from Billing financial settlement.

## Integration boundaries

The owner integration must cover these boundaries before the corresponding profile is enabled:

| Topic | Required integration |
| --- | --- |
| Financial readiness and capture | Lifecycle supplies order-transition authority. Billing/Payments own payment initiation and execution; Workflow consumes financial-owner evidence and delivers protected capture authority. |
| Cancellation before fulfillment | Lifecycle orders cancellation from the approved wait against begin-fulfillment and the accepted expiry policy. |
| Preparation and timing | Workflow waits for Subscriptions' final applied result. Polling limits, process lifetime and SLA accounting must support the selected preparation policy; dispatch-call timeout is not preparation timeout. |
| Approvals | One initial stage uses a named-user library criterion, RBAC-managed assignment and the common inbox. Missing or ineligible assignees require explicit handling. Owner units and decision paths remain authoritative. |
| Draft rebuilding | Expired drafts retain their successful creation history. Successor attempts require predecessor settlement and exclusive reassociation of protected acquisition evidence. |
| Future starts | The common barrier is the latest requested line date; it promises neither each earlier date nor simultaneous activation. Future-start horizons, preparation limits and process lifetime need a supported versioned profile. |
| Recovery | Complete scoped identity, authoritative rereads and durable settlement survive lost replies and transport-key expiry. Absence does not establish nonapplication. |

## Required integration contracts

| Request | Required integration |
| --- | --- |
| OW-P01 Versioned commercial authority | Replace unconditional `activation_deadline_at` abort for the AP profile with pre-purchase assessment validity and durable accepted-purchase coverage. Enforce explicit purchased execution restrictions and current financial/order guards; an old assessment deadline cannot shorten protected coverage. Apply consistently at freeze, re-check and every dispatch. |
| OW-P02 Protection/capture execution | Add durable steps that deliver Lifecycle's capture decision only after the funded roster has capacity and future-term protection. Deliver order/protection authority to Billing/Payments and recover through those financial owners under the original capture identity; Workflow does not create or execute payment. Keep this independent of the existing spawn signal. |
| OW-P03 Retained purchase financial evidence | Retain conclusive failed outcomes for Lifecycle's tolerance decision. Establish during acquisition and carry/recover FA payment-allocation or future-postpay commitment references. Process eligibility is not financial coverage; activation consumes the retained commitment without a new credit assessment or payment. Technical expiry and current credit-limit changes do not revoke accepted coverage. Support postpay without a card/capture step. |
| OW-P04 Draft rebuild and successor attempt | Preserve acquisition, claim, commitment, capture and benefit identities across a rebuilt draft. A new technical attempt requires fenced predecessor non-application and exclusive association; recover an already applied predecessor instead of creating another subscription. |
| OW-P05 Compensation completeness | `absent/already-absent` can finish the draft leg only. Retain separate capacity/commercial settlement and durable handoff of payment/remedy obligations to their owners. Workflow may close operationally before financial settlement once responsibility is durably accepted; it does not wait for a Billing credit note. Preserve the existing forced-terminal subscription unwind and extend obligation retention to the protected purchase. |
| OW-P06 Process-definition coverage | Every supported process, including custom definitions, retries, holds and supersession, must preserve protection-before-capture, evidence-based completion and recovery of unknown outcomes. |
| OW-P07 Approval money | Replace `tcv_minor` transport/storage with the versioned major-unit representation from Lifecycle/Rating under [the money ADR](../../../docs/ADR/0001-cpt-cf-bss-adr-exact-decimal-boundaries.md). Apply the [explicit approval valuation basis](#approval-valuation-basis-proposal), preserving exact/display/posting-estimate distinctions. Workflow neither computes the amount nor silently chooses its basis. |
| OW-P08 Operation lookup and recovery | Adopt the [operation identity and recovery proposal](#operation-identity-and-recovery-proposal): reuse the complete scoped intent identity for authoritative lookup, separate durable recovery from transport-key expiry, and require fenced settlement before absence can authorize replacement. |
| OW-P09 Financial-readiness ownership | Replace the direct Payments authorization-read dependency for order readiness: the deployment-selected financial owner (Billing where selected) interprets order financial readiness; Workflow reads it from that owner, never from a raw Payments authorization, backed by Payments facts. Preserve Lifecycle's order-transition authority, distinguish process eligibility from durable acquisition financial coverage, and align activation with retained evidence and explicit disposition fences in the upstream requirements. Owner SDKs must preserve waiting and authoritative rereads. |

## Examples

**Future window survives assessment expiry.** An order's assessment deadline is 16 October, while its separate committed start window includes 1 November. On 1 November, Workflow verifies the AP evidence and final admission; it must not take the `bindingExpired` unwind solely because 16 October passed. An actual accepted start restriction or unsatisfied financial/order/security condition retains its own recovery obligations.

**Draft disappears after payment.** Payment succeeded but the original draft expired before fulfillment. The compensation table may report the draft as absent. The order still owns a paid acquisition and its protection. Workflow recovers/rebinds that acquisition under AP07 or performs the agreed cancellation/remedy; it does not report all obligations settled from draft absence.

**Lost spawn-signal response.** Lifecycle committed the signal, but Workflow did not receive the response. Recover that same signal before dispatch. Capture and financial recovery cannot be skipped on this path.

**Postpay.** Accepted credit-service terms authorize service and no immediate debit is required. Workflow follows the applicable process route and passes the retained acquisition financial commitment to Subscriptions. It does not wait for a card authorization that this settlement scheme does not require.

## Acceptance and implementation boundary

Ratify the Lifecycle/Subscriptions/financial SDK mappings across fulfillment, approval, recovery, manual control and authorization. Align owner PRDs, contracts and upstream requirements. Subscriptions owns actual time; activation dispatch requires committed spawn evidence.

Execute [AP](../ACTIVATION_PROPOSAL.md#13-orders-and-pricing-reciprocal-activation-contract) and [FA](../DESIGN.md#3311-financial-admission-integration-contract) scenarios through the real process-definition engine, including worker death, duplicate dispatch, held/resumed instances, supersession and forced terminal state. A proposed step name is not a new public API until its owner versions it. General process-engine redesign and Change Orders authoring are outside this proposal.

## Abort and recovery branches

Scope the complete overlap-abort branch to a definitively unprotected acquisition: the overlap decision, stop-further-dispatch action, draft void, `overlap-collision` cause and `in_fulfillment → fulfillment_failed` acknowledgement. Protected acquisitions use the recovery protocol below.

For protected acquisition, validate AP02 own-claim evidence and use AP07 recovery. An owned reservation is not a collision with itself; unknown occupancy or a missing claim read is not proof of an unprotected collision. Keep claim, commitment and financial obligations until authoritative application/capture recovery, effect-owner fencing and the required remedy allow coordinated release. Draft void may finish only its own leg. If the final operational status is failed, this does not erase remaining financial work.

**Example.** Limit one is occupied by this order's reservation, and the occupancy read times out after payment. Keep the result unknown; a retry limit may require escalation or operational failure, but cannot prove a collision or authorize release. Preserve the reservation, commercial commitment and financial recovery until settlement permits release. A successful read must recognize this acquisition's own claim instead of counting it as a competing purchase.

## Operation identity and recovery proposal

This contract defines recovery for OW-P04 and OW-P06. Owner SDK mapping and runtime conformance remain delivery gates.

Workflow MUST persist its intent and complete scoped lookup identity before sending. Subscriptions uses the same identity for authoritative recovery. An operation's recovery identity must outlive the temporary transport deduplication record whenever recovery or delayed delivery remains possible.

### Identity and lookup

1. The proposed scoped operation identity is `(resource_tenant_id, orderId, orderVersion, orderLineId, wave, intentKind, wave_attempt)`. Both owners must specify the same versioned canonical encoding, including an explicit initial attempt. Workflow persists it and the immutable normalized request/digest before sending. Subscriptions durably associates its request record with that identity before executing effects. Creation, activation, draft void and activated cancellation are distinct actions; the operation identity is neither the process correlation ID nor the engine's invocation/retry identity.
2. Subscriptions exposes authorized lookup by that complete identity, as well as by its returned `transition_request_id`. Both address the same durable operation and return its current authoritative outcome, target reference when known, and reason/evidence needed for recovery. SUB-O13 recovery includes retained terminal outcomes as well as pending operations; lookup is not limited to requests that are still non-terminal. A partial tuple must not silently select the latest or an arbitrary operation.

   Recovery still works if the server-generated request ID is lost.
3. Repeating the same identity and normalized payload returns the retained operation and outcome. A different payload under that identity, including a changed expected subscription revision, conflicts before effects. Changing a transport key must not bypass the existing business deduplication or creation fence. Conflict is not permission to mint another key and repeat the effect.
4. Keep the operation identity distinct from Subscriptions' acquisition/create fence `(order_id, order_version, attempt_id, line_id)`. The SDK must carry an explicit association between them. Do not equate every action's `wave_attempt` with the acquisition's `attempt_id`: activation and compensation must identify the acquisition/draft attempt they act on, while their own action attempts distinguish their commands. A platform task retry changes neither identity.
5. A new acquisition attempt uses the existing [successor-attempt protocol](../ACTIVATION_PROPOSAL.md#7-boundary-failures-and-recovery): authoritative predecessor settlement, suppression of stale create/activation effects, authorized lineage and exclusive reassociation preserving applicable claims, commercial commitments and financial references. A successful create remains successful even if its draft later expires. It must not be relabelled as never-created; rebuilding it requires the supported successor association, not merely a fresh transport key.
6. Confirmation events echo the operation identity and correlation information. They wake reconciliation; an authoritative status read determines the result. Lookups enforce caller and tenant authorization before disclosure. A denied or hidden lookup provides no evidence of non-application; preserve the platform's nondisclosure policy rather than exposing existence through a new lookup route.

### Absence, expiry and settlement

Within the supported deduplication lifetime, an authorized same-key resend may recover or create the draft before ordinary compensation. Cancellation and post-expiry recovery require the durable settlement path below; a missing record cannot establish that the request was never dispatched.

**Proposed realization: versioned acquisition-attempt fencing.** Bind creation and cancellation to the same scoped acquisition attempt and its expected generation, including the order version and line identity from the acquisition/create fence above. Order cancellation or supersession must cover every affected line/attempt; closing one line is not evidence that another is fenced. Subscriptions durably records that attempt as closed even when no draft exists. A delayed create for the closed attempt is rejected.

Checking the expected generation/closed state and applying creation must serialize with closing the attempt at the Subscriptions effect boundary; a caller-supplied version field or a read before an unguarded write is insufficient. If creation commits first, recover its result and use ordinary compensation; versioning does not erase an applied effect.

A closure record blocks future application but does not prove that already dispatched external work is absent or cleaned up; retain that work as pending until authoritative settlement. Service preparation follows the persisted draft/activation operation under the initial-readiness contract, not an assumed pre-draft adapter call.

Reuse the existing acquisition-attempt identity and any compatible generation mechanism; this does not mandate a new identifier or equate the attempt generation with the order version, subscription revision, transport key or each action's `wave_attempt`. A retry retains its original attempt/generation. A new attempt requires the successor protocol above, not an increment that bypasses unresolved work. Closure evidence or an equivalent durable obsolete-generation rejection must survive transport-cache expiry under the retention rules below.

Exact fields and storage mechanics remain implementation work.

An authorized `not_found` read means only that the lookup found no record at that instant. It neither prevents a delayed request from arriving nor proves that an old effect never occurred after recovery records have been retired. A timeout or unavailable dependency is an unknown outcome, never an absence observation.

Within the supported replay interval, and only while dispatch remains authorized, Workflow may resend the identical operation under the same identity and payload. Subscriptions' atomic deduplication and business fences must serialize this resend with any delayed original. A pending or externally uncertain operation is recovered by status, not replaced with a new attempt.

After the replay guarantee expires, or when cancellation requires a definitive outcome, stop blind resubmission. Use the original operation's durable settlement/withdrawal contract. This is a separately keyed command referring to the original action, not a changed payload under the original action key. Subscriptions serializes it against creation or final application and answers:

| Authoritative result | Workflow action |
| --- | --- |
| Already applied | Recover the original receipt and target. If cancellation is required, issue the separate applicable compensation; withdrawal cannot erase an applied action. |
| Confirmed non-application with a durable fence | Record the terminal outcome. Delayed original requests cannot apply. Any successor still requires the applicable authorization, association and financial guards. |
| Pending or uncertain, including outstanding external cleanup | Retain recovery work and escalate as required. Do not report safe non-application, release protected rights or create a replacement from uncertainty. |

For cancel-before-create, the [Foundation settlement flow](../features/01-foundation.md#withdraw-or-settle-an-uncertain-attempt) records the no-create tombstone before acknowledging settlement. If creation wins the race, settlement returns the existing applied result instead. These outcomes survive restart.

After key expiry, `not_found` requires recovery or fenced settlement; it cannot establish `failed/never-dispatched`. Even confirmed non-application does not prove the request was never sent: record the settled reason accurately. The existing `oss_unconfirmed` label, a timeout or a generic `failed` label must not authorize a fresh attempt unless Subscriptions supplies authoritative settlement of the relevant effect. A process may escalate or stop trying while that effect remains unresolved; its operational status is not settlement evidence.

### Retention and late delivery

Separate bounded transport-key replay from durable business recovery. Retain the compact identity, normalized payload digest, receipt/outcome, acquisition association and required tombstone or generation fence across the agreed retry, hold, recovery and delayed-delivery windows. Pending effects cannot be garbage-collected merely because a transport TTL elapsed. Expiry of a deduplication cache neither authorizes another effect nor changes a known applied or withdrawn outcome to clean absence.

Retiring recovery evidence requires an explicit protocol that rejects obsolete generations at the effect boundary and preserves authoritative archival/recovery evidence. A cancelled attempt must never become admissible again because its tombstone was removed. If historical evidence is unavailable, report the recovery limitation and retain uncertainty rather than infer non-application. This proposal chooses no numeric retention period or unlimited storage policy; the owners must version the retention and retirement contract before delivery.

### Examples and acceptance cases

The following are planned acceptance cases, not executed tests.

| Case | Required result |
| --- | --- |
| OW-R01: create succeeds, reply is lost, Workflow restarts | Lookup by the identity persisted before send returns the original request and draft; no second subscription. |
| OW-R02: transport key changes for the same business create | The business fence returns the same result or an explicit conflict; it cannot allocate another subscription. |
| OW-R03: same operation identity, changed payload or expected revision | Conflict before effects; recovery never silently turns it into another command. |
| OW-R04: settlement arrives before delayed create | Durable no-create fence is acknowledged; the delayed original is refused after restart as well. |
| OW-R05: create applies before settlement | Return the applied receipt; cancellation uses a distinct compensation operation. |
| OW-R06: transport deduplication record expires while the business receipt remains | Lookup still returns the authoritative result; expiry cannot make an applied operation disappear. |
| OW-R07: lookup finds nothing after the replay interval, original request remains delayed | Settle/fence or remain unresolved; do not infer never-dispatched or create an unfenced successor. |
| OW-R08: external outcome is unknown and success arrives late | Keep the original effect identity; recover or compensate the late success. No fresh attempt based solely on `oss_unconfirmed` or timeout. |
| OW-R09: successfully created draft expires and requires rebuilding | Preserve historical create success and protected acquisition references; admit a replacement only through the supported exclusive successor protocol. |
| OW-R10: wrong tenant, denied access or a partial lookup tuple | No unauthorized disclosure or arbitrary operation result; none of these responses authorizes replacement. |

### Owner responsibilities

Workflow owns the persisted complete intent identity and operation recovery, including operator retry and rebuild. Its compensation and process-definition paths must preserve the same evidence requirements. Subscriptions owns authoritative lookup, business deduplication, settlement and suppression of late effects in [Foundation](../features/01-foundation.md) and [consumer contracts](../features/09-consumer-contracts.md). Align SUB-O13 lookup and SUB-O16 event echo, plus the versioned SDK's action/attempt mapping and retention rules, across both owners. The owners must adopt these changes in their interfaces.

## Approval valuation basis proposal

OW-P07 requires a versioned approval policy to select the valuation basis. Rating supplies tariff valuation; Orders Lifecycle retains immutable evidence; Workflow passes it to the approval authority without calculating money, converting currency or evaluating thresholds.

For a fixed committed term, the proposed default basis is the full contractual commitment. Monthly billing does not imply a twelve-month term. Other bases require explicit policy selection. Retain the policy version, order version, threshold currency/comparison, term or horizon, covered components and tax/discount/one-time/variable-charge treatment.

Valuation evidence identifies its source/version, basis, currency and exact major-unit amount, and distinguishes a contractual amount, estimate, authorized budget or unavailable value. A usage forecast or budget creates no payment authority, contractual minimum or enforced spending cap. Currency conversion, if required, comes from an authorized financial owner with provenance.

Missing or mismatched evidence cannot become zero or an amount-based approval exemption. Recover the required evidence or use an explicitly configured manual/non-monetary route. A policy may independently require no approval. Changed order or approval context requires reassessment; an old verdict does not authorize changed content. Approval values never become capture instructions.

| Purchase and policy | Expected interpretation |
| --- | --- |
| PaaS at EUR 100 per month with a fixed twelve-month commitment; policy requires approval above EUR 1,000 using full-term net commitment; no additional charges or discounts | The valuation owner returns EUR 1,200 for the committed term. The approval authority requires approval; Workflow does not compare the EUR 100 instalment instead. |
| PaaS at EUR 100 per month with only one month committed under the same basis | The owner reports the actual one-month commitment. Do not invent EUR 1,200 merely because monthly billing could continue. |
| Usage-based PaaS with unknown future consumption and an explicitly authorized EUR 2,000 budget for a declared horizon; policy selects that budget | Forward the budget as budget evidence. It is neither a forecast of the bill nor a promise to spend EUR 2,000. |
| Usage-based PaaS with no value matching the required policy basis | Obtain evidence or follow the policy's explicit alternative route. Unknown is not zero and cannot silently bypass approval. |

Before enablement, agree the evidence/verdict versions and supported policy bases. Verify exact decimal transport, basis/currency mismatch, missing usage valuation and amended-order rejection with the approval owner.

## BSS Approvals integration and assignee selection

When policy requires approval, the initial profile uses one order-approval stage and one explicitly assigned user/principal in seller-tenant scope. Assignment management requires current RBAC authorization; it is separate from permission to approve. Multiple stages, role/group-derived assignment and multi-user decision rules are outside this profile.

The named-user criterion requires an agreed shared-library extension before enablement. The approval owner must enforce that criterion, current tenant-scoped authorization and existing separation-of-duties rules on its decision path. Another user's vote cannot satisfy the request merely because that user has general approval permission. Inbox filtering alone is insufficient. Assignment replacement must invalidate stale decisions through a versioned criterion/generation. Missing or ineligible assignees require explicit handling, never an unrestricted quorum.

The [shared approval library](../../../libs/approval/README.md) supplies decision criteria; the owning gear retains authoritative units and decisions. The [BSS Approvals Inbox](../../../approvals/docs/DESIGN.md) presents owner-held requests and forwards votes. An Orders source and versioned kind/schema mapping are required. Workflow consumes the authoritative verdict bound to order version, stage, assignment and policy/evidence versions.

Before enabling real approvals, agree eligible-user evidence, assignment permissions, replacement and vote-invalidation behavior, and authoritative decision recovery. Preserve existing numeric-quorum behavior for other consumers. An unconditional stand-in `not_required` result cannot satisfy a required approval.

Joint checks cover unauthorized assignment, an unassigned stage, a vote by another user, self-approval, a stale former-assignee vote, an amended order and a lost decision notification. Owner storage, permission names, library types and inbox implementation belong in the approval integration design.

## Payment interaction and workflow waiting proposal

Billing/Payments own payment initiation, provider sessions, execution and financial recovery. Workflow consumes the deployment-selected financial owner's order-readiness answer, Billing where selected. Raw authorization, a browser redirect or a client assertion is not financial readiness. The UI may open a financial-owner session; Workflow does not create payments or execute collection.

**Required evidence and lookup.** Retain order/version, financial purpose, request identity and authoritative recovery reference. Lifecycle exposes outstanding customer action through an authorized order read; notifications may wake rereads but cannot be the only recovery route. Closing a browser, losing a notification or restarting Workflow must not lose the action or create another payment. Superseded or cancelled work retains its financial recovery/remedy obligation.

**Billing timing and service admission.** Reuse Pricing's resolved per-item `billing_timing` and SKU/default provenance from the [Pricing read contract](../../../pricing/docs/design/07-read-contract-events.md). `advance`/`arrears` describes billing timing; it proves neither collection nor permission to serve on credit. Preserve accepted settlement conditions and applicable policy. Workflow does not resolve new defaults or invent a plan-level payment flag.

The financial owner returns one of four semantic results for the exact order version: no prerequisite before execution, prerequisite satisfied, pending/unknown, or definitive refusal with a reason. The first two permit continuation through remaining order guards. Unknown does not authorize fulfillment; refusal follows Lifecycle policy. Usage/postpay with no prescribed financial prerequisite requires no card or invented credit-approval step. An advance-billed item does not require confirmed prepayment solely because of billing timing. The prepaid balance path remains deferred.

Establish durable purchase coverage during acquisition under [FA](../DESIGN.md#3311-financial-admission-integration-contract). Activation consumes that original coverage and explicit applicable dispositions; it does not repeat collection or financial eligibility assessment. Technical session/authorization expiry cannot revoke established purchase coverage. Re-entry and reauthorization follow the financial owner's contract under the same recoverable association.

**Order-wide financial gate and future start.** While the required order-wide financial result is pending, do not independently start a usage-billed line because a different line awaits prepayment. Independent starts can use separate orders. Passing the common gate does not promise simultaneous activation.

The common requested-date barrier is the latest requested line date. It starts connection/preparation, not guaranteed usable service. Workflow does not predict preparation duration and start early. Subscriptions stays draft until its [initial-readiness contract](../features/05-entitlements.md#initial-activation-readiness-contract) permits activation at T. Credit-trial preparation also requires Billing's operation-bound commitment; later grant recording neither delays the applied result nor resets trial duration. No exact-time SLA follows from the scheduled date.

For "pay now, connect later", protect the purchase before capture, retain the payment and accepted terms while waiting, and start preparation when due. Prepayment in other scenarios follows its accepted due conditions; this example establishes no universal checkout deadline.

**Preparation waiting.** A dispatch-call timeout or Workflow waiting limit triggers recovery/escalation of the same operation. Only Subscriptions' durable applied or `closed_without_activation` decision resolves the attempt. Its saved preparation policy/deadline survives retry, restart and profile changes; it cannot extend an explicit purchased start restriction. A prior applied result keeps its T. A closed attempt retains the purchase, claim and financial basis for remediation, with no automatic order cancellation or refund. A successor needs the [AP settlement/association protocol](../ACTIVATION_PROPOSAL.md#7-boundary-failures-and-recovery), including protection against old cleanup. Unknown outcomes cannot authorize a new attempt.

The Workflow owner must demonstrate supported waiting duration, durable cancellation delivery, bounded recovery and escalation with the selected preparation profile. Process-engine timers, history management, polling mechanics and numeric budgets belong in its implementation design.

**Payment deadline and cancellation.** Lifecycle retains the order-version policy for payment waiting: its deadline and cancellation or review action. Workflow requests the selected Lifecycle action; deadline expiry alone proves neither nonpayment nor successful cancellation. While financial readiness is pending, the order remains `approved`. Lifecycle MUST support cancellation during this wait and serialize pre-fulfillment cancellation against begin-fulfillment and its order-expiry rules. If fulfillment won, use the authorized Workflow compensation route instead.

A winning cancellation durably hands the original financial request to Billing/Payments for stop/reconciliation. Preserve [AP capture/cancel arbitration](../ACTIVATION_PROPOSAL.md#7-boundary-failures-and-recovery). Repeated requests recover the same financial operation. A transient notification or stop acknowledgement is not settlement. Operational closure may precede final financial settlement only after the financial owner durably accepts remedy responsibility; unresolved handoff remains tracked work.

| Financial stop outcome | Required handling |
| --- | --- |
| Confirmed stopped with no collection | Settle this financial-stop leg; complete order cancellation only after its other required legs settle. |
| Collection occurred or won the race | Retain the required remedy with Billing/Payments. Do not report stopped payment or restart a cancelled order. |
| Pending or unknown | Recover the original operation; do not claim nonpayment, refund or completion. |

Financial owners retain late and duplicate outcomes after Workflow closure, including request-to-order association and remedy progress. A late payment for a cancelled order enters financial recovery. If policy selected review and cancellation has not applied, continuation requires Lifecycle's authorized decision. Session expiry, Workflow lifetime and payment deadline are distinct from the accepted purchase commitment.

**Joint acceptance.** Before enablement, agree financial readiness/action/lookup versions, initiation and association ownership, supported settlement schemes, payment deadlines, stop/late-outcome handling and re-entry. Execute these boundary cases with the selected owners:

| Situation | Required result |
| --- | --- |
| Lost create/payment/stop response; worker restart; changed transport key | Recover the same operation and financial identity without duplicate effects. |
| Cancellation races listener registration, activation or capture | Recover the durable winner and all late effects; no disappearance, false non-application or cancelled-order revival. |
| Retry or Workflow waiting budget expires during preparation | Retain uncertainty and recover the original attempt; no automatic purchase release, refund or successor. |
| Preparation deadline races ready; timeout worker is late; profile changes | Apply Subscriptions' saved fence/deadline; recover a prior applied T or the closed attempt, preserving purchase obligations. |
| Browser closes, session expires, or notification is lost | Recover the outstanding financial action/readiness without another payment or browser-based success. |
| Payment succeeds before its deadline but arrives late, or after Workflow closes | Account for the original effect once; resolve continuation or financial remedy under the committed order state. |
| Stop wins, collection wins, or stop result is unknown | Preserve the distinct outcomes above; retries cannot duplicate remedies. |
| SKU timing overrides defaults; usage/arrears needs no prerequisite | Retain accepted provenance and proceed without an invented card/approval requirement; advance timing alone proves no collection. |
| Long approval, payment or preparation wait | Demonstrate durable recovery/cancellation within the supported runtime duration and capacity limits. |

These are required conformance cases, not executed results. Concrete session APIs, invoice sequencing and runtime prototypes remain with their owners.
