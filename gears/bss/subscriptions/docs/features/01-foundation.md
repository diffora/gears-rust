---
description: "Subscription lifecycle commands, durable operation identities, protected occupancy and failure recovery."
---

# Feature: Lifecycle Foundation and Durable Operations

<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Submit a transition and recover its result](#submit-a-transition-and-recover-its-result)
  - [Create, activate and compensate an order](#create-activate-and-compensate-an-order)
  - [Withdraw or settle an uncertain attempt](#withdraw-or-settle-an-uncertain-attempt)
  - [Scheduled intents and competing obligations](#scheduled-intents-and-competing-obligations)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [External work and stale confirmations](#external-work-and-stale-confirmations)
  - [Atomic occupancy admission](#atomic-occupancy-admission)
- [4. States (CDSL)](#4-states-cdsl)
  - [Lifecycle and operation state](#lifecycle-and-operation-state)
- [5. Definitions of Done](#5-definitions-of-done)
  - [5.1 Recoverable lifecycle and measured admission](#51-recoverable-lifecycle-and-measured-admission)
- [6. Acceptance Criteria](#6-acceptance-criteria)

<!-- /toc -->

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-featstatus-foundation-implemented`
- [ ] `p1` - `cpt-cf-bss-subscriptions-feature-foundation`

**Status:** target design. Activation, retained suspended occupancy, confirmed-order reservations and preservation under limit decreases are specified. Versioned reservation/commercial-commitment/capture contracts, exact policy/key movement mapping and runtime evidence remain required before the capability is enabled.

**Execution scope:** Workflow dates are scheduling lower bounds. Pricing validates and freezes the purchase at acceptance; Subscriptions executes those accepted terms under current order, financial, security, occupancy and held-readiness guards and records actual T in its successful local transaction. Explicit purchased start restrictions apply; ordinary later offer/catalog changes do not reopen the purchase. Preparation stays on a draft operation under [the activation contract](../contracts/activation.md).
## 1. Feature Context

### 1.1 Overview

The Foundation serializes commercial state changes and provides a recoverable answer when a caller loses a response or an external side effect is still unresolved.

### 1.2 Purpose

**Requirements:** `cpt-cf-bss-subscriptions-fr-status-enum`, `cpt-cf-bss-subscriptions-fr-transitions-guards`, `cpt-cf-bss-subscriptions-fr-transition-request`, `cpt-cf-bss-subscriptions-fr-scheduled-intents`, `cpt-cf-bss-subscriptions-fr-activation-instants`.

**Primary NFRs:** `cpt-cf-bss-subscriptions-nfr-lifecycle-latency`, `cpt-cf-bss-subscriptions-nfr-operational-baselines`, `cpt-cf-bss-subscriptions-nfr-intent-seam-budget`.

**Principles:** `cpt-cf-bss-subscriptions-principle-idempotent-ordered`, `cpt-cf-bss-subscriptions-principle-fail-closed-gate`.

### 1.3 Actors

| Actor | Role |
| --- | --- |
| `cpt-cf-bss-subscriptions-actor-platform-operator` | Diagnoses and resolves a durable failed/pending operation. |
| `cpt-cf-bss-subscriptions-actor-partner-admin` | Requests authorized lifecycle changes. |
| Orders Workflow / system workers | Submit intent, retry, recover, withdraw or settle a business attempt. |

### 1.4 References

- **PRD:** [PRD.md](../PRD.md); **Design:** [DESIGN.md](../DESIGN.md#331-control-plane-and-operation-recovery).
- **Build prerequisites:** none; the Foundation defines the operation/commit contract first, as allocated in DECOMPOSITION.
- **Integration acceptance:** identity/Policy, accepted Pricing evidence and resource provider effects, with composition, posture and fact handlers supplying effects through that contract. Production activation requires the agreed contracts and runtime evidence for the selected S-Q01/S-Q02 core directions, plus applicable residual claim rules and S-U01/S-U04/S-U10 evidence; later handler integration is not a reverse build dependency.
- **Source decisions:** [DECISIONS.md](../DECISIONS.md), SUB-D-01/05/06/08/11/23. The [activation contract](../contracts/activation.md) specifies local activation time, purchase evidence and attempt recovery; S-U01/S-U04/S-U10 identify required owner integration.

## 2. Actor Flows (CDSL)

### Submit a transition and recover its result

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-flow-transition-commit-fnd`

**Actor:** authorized caller. **Success:** a stable applied outcome or an explicit durable in-progress request. **Errors:** unauthorized, payload conflict, stale revision, illegal status, dependency/capability unavailable, denied Policy, unresolved competing resource transition.

1. Authenticate and authorize the scoped subscription or creation identity. Validate normalized payload and operation envelope without remote effects.
2. Look up the scoped command key before evaluating current business guards. Identical replay returns its durable request and current outcome; changed digest conflicts. Replay never repeats completed side effects.
3. For Orders-origin create, lock the business fence for `(order_id, order_version, attempt_id, line_id)`. A same-attempt retry with another transport key attaches to the same outcome or conflicts; execution retry retains `attempt_id`. Reject standalone create without Orders before effects. Serialize Orders create and settlement under its durable business fence; a terminal non-creation tombstone prohibits a delayed create. Changed content conflicts, and a new transport key cannot allocate another subscription for that business operation. A new Orders attempt is not replay.

   Apply the accepted [successor-attempt protocol](../contracts/activation.md#7-boundary-failures-and-recovery): recover unknown outcomes, prove predecessor non-application with create/activation fences, authorize lineage, then atomically change the exclusive association while retaining the same claim and applicable financial references. An applied predecessor is recovered, not recreated; this profile does not reopen cancelled acquisitions. Provider association, accepted-purchase evidence and current execution guards gate activation; until the exact S-U01/S-U07/S-U10 contracts are supported, succession refuses explicitly.
4. Check capability gates, source status, expected revision and absence of incompatible in-flight work. Persist the operation and required approval/effect intentions. A create produces a `draft` only after all create guards; it creates no service or one-time occurrence.
5. Collect Policy, accepted Pricing and AMS evidence outside the aggregate transaction. Fulfillment preparation, execution and recovery go through Entitlements; record its correlated observations against the saved operation/posture. Direct historical-inventory reads supply attribution evidence only. Record each external request's stable identity and result. While awaiting confirmation return the durable pending/approved operation, never active success.
6. Once the evidence required for the specific operation is available, acquire aggregate and affected occupancy keys in deterministic order. Initial activation requires the held-readiness contract; suspend/resume follow their action-specific checks and observe provider completion separately. Inside that local decision, recheck the saved attempt deadline, local revision, status, ownership, intent precedence, approval/proof validity and local freshness fences. Current external observations are not reusable admission tokens. If their contract cannot fence this commit, the capability remains gated.
7. Commit aggregate state, one new commercial revision, applied receipt, immutable history, posture intent, attribution/fact changes and outbox atomically. Receipt-only persistence and operation progress leave the revision unchanged. A preparation, withdrawal or disposition change commits its own newer aggregate revision and posture/outbox even without a status transition. Initial activation then commits one further revision with active state and T; earlier preparation creates no served-service facts.
8. If commit guards fail after a remote effect, retain a compensation obligation and recoverable failure context. Do not declare the operation safely settled until required compensation or quarantine is durably accounted for.

### Create, activate and compensate an order

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-flow-order-lifecycle`

1. At explicit order confirmation, reserve the scoped order/version/line place and obtain durable Pricing/Products commitment to its original accepted future start and fulfillment conditions **before actual Billing/Payments capture or settlement**. Missing capacity or either contract refuses before capture or settlement. Payment authorization is not capture; an authorization-failure tolerance cannot bypass this guard. Plain unconfirmed draft creation creates no reservation.
2. Billing/Payments captures only against the agreed durable reservation/commercial evidence for the captured scope and its capture/cancellation fence. While payment is pending, the held reservation consumes capacity. Timeout or unknown capture retains the right until durable financial recovery establishes the outcome; it is not an unpaid-failure signal. Paid confirmation keeps the reservation through the future-date wait, holds and ordinary TTL.
3. Create each accepted line as a draft carrying its complete base/add-on composition and quantity, with immutable order/version/line, fulfillment attempt, wave, the caller binding reference and the external order reference when supplied, and receipt evidence; this provenance travels on operation outcomes, lifecycle events and commercial facts; attach its existing reservation without allocating again. Reservation may precede this draft. The durable order commitment determines its lifetime; a transient draft/attempt does not. Same-version successor attempts follow the accepted settlement/reassociation protocol.

   A commercial version amendment instead requires Orders reassessment and explicit claim/commitment mapping plus Billing/Payments applicability; it cannot automatically inherit or release the old right or funding. Both profiles require the exact supported S-U01/S-U07/S-U10 owner contracts and cannot steal or duplicate protection. Return authoritative draft receipts to Workflow.
4. Activation first persists the operation, authorized scope, preparation posture revision/digest and outbox intent; Entitlements allocates assignments, persists the required-assignment/producer manifest and authorizes preparation rights before active (SUB-D-55, SUB-D-56). Its aggregate readiness result binds the complete manifest to the saved operation under feature 05; Subscriptions never needs to discover assignments from uncorrelated callbacks. The adapter reconciles the purchased account/environment/access and reports readiness by assignment identity; it need not know subscription ID. While required readiness is pending, status remains draft and operation progress is visible.

   Snapshot the versioned preparation policy, authorized start and deadline on the attempt; retries and profile changes cannot move it. Expiry or definitive preparation failure closes the attempt through its activation fence and emits the non-activation reason. An applied winner is recovered with its T. Preserve purchase/payment/claim and send the attempt outcome to Orders remediation; neither a timeout nor unknown result permits automatic refund or an unfenced replacement. Apply the [initial readiness contract](05-entitlements.md#initial-activation-readiness-contract), including trusted ordered observations, recovery and readiness-withdrawal arbitration.

   SUB-D-57 requires a held readiness commitment and one serialized local decision between applied activation and irrevocable non-activation for the preparation generation. Persist its outcome with posture/outbox; timeout or failure of an execution guard alone never authorizes Entitlements cleanup. Activation consumes the immutable accepted acquisition and its retained payment allocation or future postpay commitment, checks current order/security and explicit financial-disposition fences, and requires usable readiness. Establish financial coverage during acquisition; recover missing evidence by the same identity without another debit or credit assessment.

   Technical proof expiry and later credit-limit/risk changes cannot revoke the accepted purchase. Future purchase coverage is durable before capture. Caller dates are scheduling lower bounds and any latest-start condition must be explicit in the purchase. No adapter substitutes current prices or an old quote deadline for accepted terms, fabricates earlier service or creates another purchase from a retry. Missing required implementation evidence gates this profile before capture.
5. Once jointly enabled, activation atomically converts its own reservation to active, without another place or competition, and stamps `serviceActivatedAt` with Subscriptions-owned T independently of `contractEffectiveAt` and `customerAcceptedAt`. An unsatisfied accepted start condition or current financial/security/order guard blocks application. Protected purchase obligations remain for recovery or agreed cancellation. All required initial usable access precedes this commit. T is sampled locally after fresh guards, never copied from a ready event; initial period/accrual starts at T. Post-commit reconciliation cannot first issue an essential initial right.

   Capture may precede T without proving service. Initial readiness does not guarantee continuing uptime. The separate confirmAcceptance operation is deferred; existing acquisition acceptance evidence remains required where applicable.
6. Workflow may compensate a partial order by withdrawing unapplied attempts and cancelling applied subscriptions with `order_compensation`. Void of an unactivated draft creates no service fact or service period; a paid order may still need financial settlement and staged external-effect cleanup. Cancellation must fence reservation consumption and capture, recover unknown payment, and record its agreed financial-remedy obligation. Refund policy and settlement/release ordering require owner agreement; neither automatic refund nor waiting for a posted refund is implied universally.
7. Applied-service compensation records stop, Entitlements/resource provider offboarding and corrected facts. No ETF or unused-portion credit is derived for `order_compensation`; prepayment remedies and posted setup reversals belong to their agreed Billing/Payments paths. Consumed usage is retained with real attribution and explicit correction disposition, never erased to simulate atomic rollback.

### Withdraw or settle an uncertain attempt

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-flow-withdraw-settle`

1. Authorize the same business/attempt scope used for submission, then serialize withdrawal/settle with create and final application.
2. If already applied, return the applied outcome and subscription reference; withdrawal cannot undo it. The caller may submit a separate compensation command.
3. If no create applied, store a terminal no-create tombstone under the business/attempt fence before acknowledging settle. A delayed request or external confirmation cannot create after it.
4. For an outstanding effect, first prevent commercial completion, cancel the work order idempotently and reconcile late success with compensating work. Report unsettled compensation while completion evidence is absent.
5. Once terminal non-application is established and outstanding external effects are safely resolved, retain the failed receipt with reason `withdrawn` or settled-no-create and full correlation. Until then, retain the non-application decision and report pending cleanup separately. Status reads by request and order/attempt return the same answer after restart.

Attempt settlement or draft void does not by itself release a confirmed order's durable reservation or commercial commitment. Its explicit cancellation/rebind path must fence capture and future consumption, recover payment uncertainty and retain the required financial-remedy evidence under the agreed owner contract.

### Scheduled intents and competing obligations

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-flow-scheduled-intent-fnd`

**Input:** committed future intent and current aggregate. **Output:** one applied outcome, explicit cancellation, retry or parked state.

1. Refuse scheduled composition and scheduled resume requests. Persist supported cancellation intents and ordinary lifecycle timers separately from applied intervals. Announce `SubscriptionIntentScheduled` with effective time; no committed billable boundary is invented.
2. Give each firing a derived identity `intent:{intentId}` and re-read current aggregate revision. For intents due at the same boundary, preserve deterministic kind priority: cancel/end-of-term opt-out → applicable lifecycle timers; break ties within a kind by ascending intent ID. Workers must respect that order rather than race for the aggregate lock. Each firing rechecks the preceding outcome and normal in-flight guards; a terminal transition sweeps lower-priority pending work. Cancellation/non-renewal at or before a trial or term boundary suppresses conversion/renewal before either can commit.

   Priority orders work; it does not backdate any successful application to the scheduled boundary.
3. Re-run all guards through the Foundation. An operator can unschedule before application; a customer-confirmed cancellation additionally requires an explicit versioned confirmed withdrawal/reschedule before its charging cutoff. Serialize against the intent and cutoff evidence; after cutoff, unapplied resource work does not permit retroactive restoration of charging. A successful firing publishes the actual committed boundary once.
4. Classify failure: transient dependency/resource provider uncertainty retries with bounded backoff and suppressions retained; denied/incompatible state becomes explicit failed intent plus operator finding. A state that is expected to clear, such as conversion while suspended, parks and pauses its firing deadline until resume.
5. If a terminal cancellation-intent failure occurs, renewal/conversion remains suppressed until explicit operator resolution. Failure cannot silently turn an opt-out into a new charge.
6. A terminal subscription sweeps incompatible pending intents, records their disposition, leaves already incurred debt intact; collection-pause windows are deferred.

## 3. Processes / Business Logic (CDSL)

### External work and stale confirmations

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-algo-effect-recovery`

See [RO-2026-10-15](04-renewal-retention.md#7-renewal-and-offboarding-integration-contract).

A resource-affecting commercial transition owns a local in-flight marker; Entitlements owns provider work orders, dispatch and physical-outcome recovery. Subscriptions correlates the observations Entitlements reports with its operation and posture. Competing resource-affecting commands are refused or explicitly supersede it through a journaled cancellation/compensation process. Non-resource commands may proceed if they do not invalidate staged assumptions; completion rechecks the changed aggregate version. A work-order timeout means unresolved effect, not proof of non-execution.

Late confirmations match the retained effect identity and either complete the still-current operation or start reconciliation/compensation. They cannot resurrect an applied withdrawal, terminal subscription or superseded operation.

Under accepted S-Q05, an explicit customer-confirmed cancellation charging cutoff is a separate financial commitment from the eventual guarded cancel/resource provider stop. A request receipt is not that confirmation. Customer cancellation is confirmed only after authorization, commercial eligibility and durable recording of its charging cutoff; confirmation is independent of resource provider completion. Withdrawal or rescheduling requires an explicit versioned confirmed action before the applicable cutoff, without retroactively restoring charges. After cutoff, new service needs a separate valid guarded route, not undo of the financial commitment.

Worker delay or unknown stop cannot extend ordinary customer tariff beyond a known cutoff; a future scheduled cutoff does not waive normal service before it. Recovery preserves truthful runtime and operation/state evidence without backdating stop or applied cancellation. Only applied cancellation releases normal occupancy; cutoff alone permits neither archive nor deletion. Versioned confirmation, capture/compensation and correction contracts remain S-U05/S-U13 gates; retention and paid-order remedy paths are unchanged.

### Atomic occupancy admission

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-algo-occupancy-admission`

The S-Q02 rule keys occupancy by `(payerTenantId, resourceTenantId, overlapScopeKey)` and requires explicit `maxConcurrentActive` from versioned policy, with retained source/version provenance. Admission counts actual active subscriptions + retained suspended places + explicit order reservations, each right exactly once and each count reported separately. Held payment-pending reservations count too. Missing policy refuses; there is no implicit limit of one. Confirmation, activation, suspend, resume, cancel and affected key changes serialize claim changes.

Confirmed-order activation converts its own reservation to active without a second place; exclude that same right once. Other supported unreserved paths use ordinary atomic admission against all claims and cannot bypass the pre-capture safeguards if collecting upfront. Capacity N > 1 must be safe under concurrency. Counting and then inserting outside the serialized commit is invalid.

**Suspended-place rule:** suspend converts the same subscription's active claim into a retained suspended place. Resume on its unchanged claim atomically converts that place back to active without competing for or consuming a second place. Thus active 0 plus retained 1 at limit 1 has no free capacity, and suspended-to-active preserves total claims. This guarantees only that the unchanged claim faces no competition. Applicable payment, Policy, term and resource guards still apply.

Entitlements materialization remains asynchronous and separately observable after the atomic state/claim/posture-intent commit; the place guarantees neither usable access nor a billing waiver. Applied cancellation releases the claim; a pending, scheduled or failed cancel does not. Mere TTL/dwell expiry cannot release it: an accepted timeout policy must use normal guarded cancellation. Cancellation's resource retention or legal hold does not retain normal occupancy.

For supported composition key changes on admitted active targets, lock source and target keys in deterministic order. Move the claim atomically with the commercial change. Refuse target conflicts while preserving the source claim; unchanged-key operations exclude self exactly once. A versioned limit decrease preserves all existing protected active, retained-suspended and explicit order-reserved claims, even when their total exceeds the new limit; it never evicts or releases a claim solely for that change. Reject new admissions that would exceed the current limit until room exists.

Unchanged-key resume and activation consuming an existing reservation convert the same protected right without another slot; their other guards remain. Applied cancellation releases its own claim. Policy/key/claim movement requires an established destination/version mapping and atomic move; unsupported or failed movement preserves the source claim and refuses the move. Exact provider contracts remain open; import mapping applies only to the separate import capability.

A plain draft is not active, retained suspended or automatically reserved. Explicit confirmation creates a durable order-bound reservation that may later attach to the intended draft/attempt. Paid confirmed reservations survive future-date waiting, holds, ordinary TTL and technical fulfillment failure; remediation or explicit cancellation under the financial contract resolves them. Unknown capture retains the reservation until the financial outcome is recovered. Cancellation must fence late capture/activation and record financial-remedy obligations before terminal release under the agreed contract.

Unpaid abandoned-draft cleanup is housekeeping with a configurable TTL and no financial effect; it cannot release paid/uncertain rights. A confirmed unpaid prepay order keeps its place until the versioned payment deadline from the accepted order terms; after that only the order's cancel route releases the claim.

Current Orders active-count-only observations and release-on-leaving-active semantics cannot represent all three claim states. Versioned producer/consumer evidence, future commercial commitment and capture/cancel fencing must align before enabling the prepayment profile, with refusal **before capture** when unavailable. A read preview is not a reservation, and reservation/receipt alone creates no service facts, access or service period. Linked subscription replacement and its predecessor occupancy exemption are deferred. Future-dated Orders bookings still count from confirmation; technical attempt succession remains distinct.

Exact policy/key movement mappings remain open under S-U10; S-U01 still governs attempt rebind/successors and capture association.

## 4. States (CDSL)

### Lifecycle and operation state

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-state-foundation`

| From | Operation / to | Mandatory guard |
| --- | --- | --- |
| draft | activate / active | Applicable accepted-terms/readiness/Policy/activation guards. Confirmed-order profile consumes its reservation once; other supported unreserved paths use ordinary atomic admission and cannot bypass pre-capture safeguards if collecting upfront. |
| draft | immediate cancel / cancelled | No supplied service; confirmed-order rights additionally need capture/consumption fencing and recorded financial remedy under the agreed cancellation contract. |
| active | suspend / suspended | Authorized Policy decision and supported non-destructive pause capability; atomically retain the same occupancy place and publish restriction intent. Observe provider pause completion after the commit; no resource deletion. |
| suspended | resume / active | Policy and applicable payment/term/resource guards; atomically convert its retained unchanged-key place to active without another slot. Entitlements access remains separately observable. |
| active or suspended | cancel / cancelled | Accepted reason/mode, service-stop and retention disposition; release normal occupancy only at applied cancel commit. |
| cancelled | archive / archived | Archive dwell, operational-data completion and evidence-retention constraints. |

Operations use `pending → approved → applied` or `pending/approved → failed`; approval means staged permission, not commercial success. Same-key replay reads the existing operation while it progresses. Applied receipts are immutable.

`changeComposition` follows [feature 03's source-state matrix](03-changes.md#change-operation-state) and declared operation profile. Under the purchase-state rule, new seat purchases, increases and additions require an active existing target; suspended targets refuse at submission and again under the completing transaction's state/revision fence (SUB-D-39). First perform a separate authorized guarded resume, then submit the normal active change; no purchase resumes or queues itself. A non-terminal state alone is not enough.

If suspension wins before commit, recover any staged external work without applying the purchase; if the purchase committed first, preserve its historical obligations and reconcile delivery under the newer suspension fence. Draft changes remain blocked until S-U01 accepts the immutable acquisition-plus-delta receipt, hold and withdrawal/recovery protocol; the selected S-Q01 direction and still-open versioned activation contract govern affected first activation. Direction approval does not settle these draft-change semantics.

Once enabled, a draft edit changes only draft composition and its revision, preserves acquisition/delta evidence and emits no activated service, usable grants or billable facts. A composition change never implicitly activates or resumes the target. Single-item wrappers apply only their explicitly supported source-state/profile combinations; they do not bypass this matrix.

Manual renew permits suspended only for a documented blocked-renewal recovery. Transfer and confirmAcceptance are unavailable. Unschedule requires a pending supported cancellation intent and, for confirmed cancellation, the additional pre-cutoff versioned-confirmation guard. Trial-specific guards are in feature 06.

## 5. Definitions of Done

### 5.1 Recoverable lifecycle and measured admission

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-dod-foundation`

The implementation must enforce one scoped replay/business fence and commercial commit path, persist every asynchronous effect and outcome across restart, reject closed capabilities before side effects, and expose status/settle recovery without mutation re-submission. Operation receipts and domain events carry actor, correlation, reason, input digest, expected/actual revision and denied/failed attempts for a later audit consumer; no audit store is built. Authorization precedes replay. Manifest and SDK operation names must match the agreed command inventory.

**NFR acceptance evidence:** use the authoritative proposed [PRD §7 profile](../PRD.md#7-non-functional-requirements): 100,000 active subscriptions in one large tenant, one million across ten tenants, one hour at 100 mutation submissions/s and 1,000 reads/s, then a five-minute twice-rate burst. Record hardware/storage, composition/resource distribution, hot-key contention, offered/completed rates and healthy-provider p99 latency (proposed maximum 250 ms). Report durable intent acceptance and local application without an external service leg separately against the proposed p95 < 1 s / p99 < 2 s targets.

Remote provisioning/materialization has its own elapsed-time metrics, never an implied successful activation.

Measure bounded authorized reads against the ratified p95 < 200 ms / p99 < 500 ms and commit-to-consumer event availability against the ratified p95 < 30 s / p99 < 60 s with a healthy broker. The report must cover every operation outcome, including pending and overload, and state the outstanding-operation bound and explicit back-pressure response. Dependency fault injection must demonstrate recoverable receipts and source-linked metrics/findings for oldest pending operation, external-leg age, outbox age, attribution gaps, overdue groups, unsealed scopes and retention/deletion backlog.

Capability handlers supply their metrics at integration acceptance. These thresholds are the ratified v1 targets (S-Q07); readiness requires measured evidence, not this document alone.

**Implements:** `cpt-cf-bss-subscriptions-flow-transition-commit-fnd`, `cpt-cf-bss-subscriptions-flow-order-lifecycle`, `cpt-cf-bss-subscriptions-flow-withdraw-settle`, `cpt-cf-bss-subscriptions-algo-occupancy-admission`.

## 6. Acceptance Criteria

- [ ] A versioned limit decrease preserves existing active, retained-suspended and order-reserved claims even above the new limit. New admissions above it refuse; own-reservation activation and unchanged-key resume retain their right and other guards. Applied cancel releases only its claim. Missing destination/version mapping or a failed atomic claim move leaves the source claim intact.

- [ ] A committed create with lost response, replayed under the same and a different transport key, produces one subscription and one successful business-attempt outcome.
- [ ] A changed payload under an existing key conflicts without issuing a remote command.
- [ ] Standalone create without an Orders acquisition refuses before draft creation, reservation, payment or provisioning; operator privilege does not enable it. Existing-subscription operations and original Orders-attempt recovery remain available under their guards.
- [ ] Cancellation and renewal/conversion due at the same boundary serialize with cancellation first despite reversed worker arrival. Applied cancellation suppresses conflicting work; a failed or pending cancellation cannot silently permit renewal. Same-kind intents use stable ordering and current guards.
- [ ] Create-vs-settle and activation-vs-withdraw races have one serialized winner; a late confirmation after the settle fence cannot activate.
- [ ] Competing confirmations and supported unreserved activations for the last place yield one admission; a refused order is not captured. Actual active, retained suspended and held reservations all count. Missing policy refuses; plain draft creation alone reserves nothing.
- [ ] A paid future-start reservation survives ordinary TTL, date waits and holds, blocks a rival and converts exactly once to active without another place. Retry preserves the same claim; technical failure retains it for remediation/cancellation without a service fact.
- [ ] Capture refuses a confirmed reservation that has no durable Pricing/Products commitment to its original accepted future start and fulfillment conditions. A later activation uses that commitment's versioned accepted-term evidence and does not read the catalog again.
- [ ] Unknown capture or payment timeout retains the claim. Cancellation racing late capture and activation reaches the agreed fenced outcome, records the required financial-remedy obligation and never silently frees a paid/uncertain right.
- [ ] Suspend racing a new activation at limit 1 never exposes a free place: active 1 becomes active 0 plus retained 1. Resume on that unchanged claim returns to active 1 without consuming another place or competing with the new activation; other resume guards still apply.
- [ ] Pending, scheduled or failed cancellation retains the active/suspended claim. Applied cancellation releases it atomically; later resource retention or legal hold does not keep normal occupancy claimed.
- [ ] Policy outage leaves commercial state unchanged and a recoverable request outcome; no database transaction spans the remote wait.
- [ ] External success followed by a stale completing revision creates a durable compensation finding; restart resumes it.
- [ ] Receipt-only persistence leaves the revision unchanged. Preparation and withdrawal each publish a new posture revision while status stays draft; successful activation commits one further revision with active state and T. Replays create no extra revisions, and preparation creates no served-service facts.
- [ ] Failed cancellation intent continues suppressing renewal and paid conversion until explicit resolution.
- [ ] Direction approval alone does not enable activation. Capabilities awaiting the S-Q01/S-Q02 owner contracts, applicable residual claim rules or runtime evidence return an explicit refusal with no resource, price-hold or financial side effect.
- [ ] The S-Q07 load report exercises the full ratified steady/burst profile, same-key retries, changed-payload conflicts and hot occupancy keys; durable receipt and local-application p95/p99 are reported separately and checked against the ratified profile before release.
- [ ] Saturating the declared outstanding-operation limit yields an explicit retryable back-pressure response or the existing receipt. No accepted operation is dropped; timed-out create can still be read/settled without another effect.
- [ ] Bounded-read and event-availability measurements include the proposed 200/500 ms and 30/60 s percentile targets under their declared healthy-provider/broker conditions. Outages instead produce visible pending/retry outcomes without granting access.
- [ ] Injected stuck work in every allocated observability class is discoverable through its metric and an authorized, source-linked operation/finding read; current version/checkpoint is visible rather than hidden as fresh state.
- [ ] New purchases/increases/additions refuse on suspended targets at submission and commit; a separate guarded resume precedes normal active change. A suspension that wins the commit race prevents purchase application; an already committed purchase retains its history and recovers under the later suspension fence. Draft changes refuse until the S-U01 protocol is accepted, then preserve both accepted receipts without creating service, usable grants or billable facts before separate activation.
