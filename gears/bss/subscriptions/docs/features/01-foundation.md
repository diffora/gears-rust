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

**Status:** S-Q01 activation and S-Q02 occupancy, including suspended-place retention and confirmed-order reservation before capture, accepted by the user on 2026-10-07. Limit-decrease preservation and the occupancy residual package were accepted on 2026-10-08. Versioned reservation/commercial-commitment/capture contracts, exact policy/key movement mapping and runtime evidence remain OPEN. The implementation is unchecked. No running implementation is claimed.

**Approval scope:** Workflow dates are scheduling lower bounds; Subscriptions completes required checks and resource readiness, obtains the proposed bounded Pricing permit for original terms, and assigns commercial T in its atomic transition to active. Expiry refuses activation; retries do not extend the interval. entitlement provider usable access may follow T. This approval establishes no fixed permit TTL, entitlement provider latency SLO or credit policy; see the dated [activation direction record](../ACTIVATION_PROPOSAL.md).
**Current-profile boundary, 2026-10-15:** apply the [current delivery scope](../PRD.md). Audit subsystem, confirmAcceptance, ramps, co-term, collection pause, notifications, tenant/payer transfer, administrative backdating/correction and optional brand/PlanTier metadata are deferred. Earlier references and scenarios involving those capabilities are future context only, not current acceptance or implementation tasks. Retain authorization, operation/event provenance, reliable effects and current facts/replay.

**Additional current-profile boundary, 2026-10-15:** the [additional scope reduction](../PRD.md#11-purpose) defers trial extension/repeat, acquisition without Orders, scheduled composition/resume, linked subscription replacement and base-plan changes. Retain supported immediate increases/additions, existing lifecycle, future order starts/cancellation and ordinary timers, both trial variants and early conversion to the accepted paid target. Deferred paths are refused even with explicit policy or operator authorization; their earlier examples are future context, not launch gates.

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
- **Source decisions:** SUB-D-01/05/06/08/11/23 and amended source precedence in [DECISIONS.md](../DECISIONS.md). S-Q01 and S-Q02 core directions are accepted as of 2026-10-07; their owner contracts/runtime evidence remain open. The S-Q02 residual product package was accepted on 2026-10-08; exact claim/key movement and replacement-protocol contracts remain gated. The [S-Q01 companion proposal](../ACTIVATION_PROPOSAL.md) records the selected clock direction, proposed technical contract and failure vectors; direction approval alone enables no runtime capability.

## 2. Actor Flows (CDSL)

### Submit a transition and recover its result

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-flow-transition-commit-fnd`

**Actor:** authorized caller. **Success:** a stable applied outcome or an explicit durable in-progress request. **Errors:** unauthorized, payload conflict, stale revision, illegal status, dependency/capability unavailable, denied Policy, unresolved competing resource transition.

1. Authenticate and authorize the scoped subscription or creation identity. Validate normalized payload and operation envelope without remote effects.
2. Look up the scoped command key before evaluating current business guards. Identical replay returns its durable request and current outcome; changed digest conflicts. Replay never repeats completed side effects.
3. For Orders-origin create, lock the business fence for `(order_id, order_version, attempt_id, line_id)`. A same-attempt retry with another transport key attaches to the same outcome or conflicts; execution retry retains `attempt_id`. Reject standalone create without Orders before effects. Serialize Orders create and settlement under its durable business fence; a terminal non-creation tombstone prohibits a delayed create. Changed content conflicts, and a new transport key cannot allocate another subscription for that business operation. A new Orders attempt is not replay. Apply the accepted [successor-attempt protocol](../ACTIVATION_PROPOSAL.md#7-boundary-failures-and-recovery): recover unknown outcomes, prove predecessor non-application with create/activation fences, authorize lineage, then atomically change the exclusive association while retaining the same claim and applicable financial references. An applied predecessor is recovered, not recreated; this profile does not reopen cancelled acquisitions. Provider association and final permit readiness gate activation; until the exact S-U01/S-U07/S-U10 contracts are supported, succession refuses explicitly.
4. Check capability gates, source status, expected revision and absence of incompatible in-flight work. Persist the operation and required approval/effect intentions. A create produces a `draft` only after all create guards; it creates no service or one-time occurrence.
5. Collect Policy, Pricing, AMS and resource provider observations outside the aggregate transaction. Record every external command's stable identity and result. While awaiting confirmation return the durable pending/approved operation, never active success.
6. On readiness, acquire aggregate and affected occupancy keys in deterministic order. Recheck local revision, status, ownership, intent precedence, approval/proof validity and local freshness fences. Current external observations are not reusable admission tokens; if their contract cannot fence this commit, the capability remains gated.
7. Commit aggregate state, one commercial revision, applied receipt, immutable history, posture intent, attribution/fact changes and outbox atomically. An async intent commit does not increment commercial revision; successful application increments it once.
8. If commit guards fail after a remote effect, retain a compensation obligation and recoverable failure context. Do not declare the operation safely settled until required compensation or quarantine is durably accounted for.

### Create, activate and compensate an order

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-flow-order-lifecycle`

1. At explicit order confirmation, reserve the scoped order/version/line place and obtain durable Pricing/Products commitment to its original accepted future start and fulfillment conditions **before actual Billing/Payments capture**. Missing capacity or either contract refuses before capture. Payment authorization is not capture; an authorization-failure tolerance cannot bypass this guard. Plain unconfirmed draft creation creates no reservation.
2. Billing/Payments captures only against the agreed durable reservation/commercial evidence for the captured scope and its capture/cancellation fence. While payment is pending, the held reservation consumes capacity. Timeout or unknown capture retains the right until durable financial recovery establishes the outcome; it is not an unpaid-failure signal. Paid confirmation keeps the reservation through the future-date wait, holds and ordinary TTL.
3. Create each accepted line as a draft with immutable order/version/line, attempt/wave and receipt evidence; attach its existing reservation without allocating again. Reservation may precede this draft. Its lifetime belongs to the durable order commitment, not a transient draft/attempt. Same-version successor attempts follow the accepted settlement/reassociation protocol. A commercial version amendment instead requires Orders reassessment and explicit claim/commitment mapping plus Billing/Payments applicability; it cannot automatically inherit or release the old right or funding. Both profiles require the exact supported S-U01/S-U07/S-U10 owner contracts and cannot steal or duplicate protection. Return authoritative draft receipts to Workflow.
4. Activation separately checks required readiness and the bounded S-Q01 Pricing permit. The earlier commercial commitment is a distinct new versioned capability, not a V1 hold or fresh sellability preview, and is not itself the final permit. Caller dates are scheduling lower bounds; no adapter extends expired V1/permit evidence, substitutes `now` into a held receipt, backdates service or silently reprices a paid order. Missing required implementation evidence gates this profile before capture.
5. Once jointly enabled, activation atomically converts its own reservation to active, without another place or competition, and stamps `serviceActivatedAt` with Subscriptions-owned T independently of `contractEffectiveAt` and `customerAcceptedAt`. Expired final authorization refuses that application; paid reservation and commitment remain for remediation or explicit cancellation. Usable entitlement provider access may follow T. The separate confirmAcceptance operation is deferred; existing acquisition acceptance evidence remains required where applicable.
6. Workflow may compensate a partial order by withdrawing unapplied attempts and cancelling applied subscriptions with `order_compensation`. Void of an unactivated draft creates no service fact or service period; a paid order may still need financial settlement and staged external-effect cleanup. Cancellation must fence reservation consumption and capture, recover unknown payment, and record its agreed financial-remedy obligation. Refund policy and settlement/release ordering require owner agreement; neither automatic refund nor waiting for a posted refund is implied universally.
7. Applied-service compensation records stop, entitlement provider/resource provider offboarding and corrected facts. No ETF or unused-portion credit is derived for `order_compensation`; prepayment remedies and posted setup reversals belong to their agreed Billing/Payments paths. Consumed usage is retained with real attribution and explicit correction disposition, never erased to simulate atomic rollback.

### Withdraw or settle an uncertain attempt

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-flow-withdraw-settle`

1. Authorize the same business/attempt scope used for submission, then serialize withdrawal/settle with create and final application.
2. If already applied, return the applied outcome and subscription reference; withdrawal cannot undo it. The caller may submit a separate compensation command.
3. If no create applied, store a terminal no-create tombstone under the business/attempt fence before acknowledging settle. A delayed request or external confirmation cannot create after it.
4. For an outstanding effect, first prevent commercial completion, cancel the work order idempotently and reconcile late success with compensating work. Report unsettled compensation while completion evidence is absent.
5. Once terminal non-application is established, retain the failed receipt with reason `withdrawn` or settled-no-create and full correlation. Status reads by request and order/attempt return the same answer after restart.

Attempt settlement or draft void does not by itself release a confirmed order's durable reservation or commercial commitment. Its explicit cancellation/rebind path must fence capture and future consumption, recover payment uncertainty and retain the required financial-remedy evidence under the agreed owner contract.

### Scheduled intents and competing obligations

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-flow-scheduled-intent-fnd`

**Input:** committed future intent and current aggregate. **Output:** one applied outcome, explicit cancellation, retry or parked state.

1. Refuse scheduled composition and scheduled resume requests. Persist supported cancellation intents and ordinary lifecycle timers separately from applied intervals. Announce `SubscriptionIntentScheduled` with effective time; no committed billable boundary is invented.
2. Give each firing a derived identity `intent:{intentId}` and re-read current aggregate revision. For intents due at the same boundary, preserve deterministic kind priority: cancel/end-of-term opt-out → applicable lifecycle timers; break ties within a kind by ascending intent ID. Workers must respect that order rather than race for the aggregate lock. Each firing rechecks the preceding outcome and normal in-flight guards; a terminal transition sweeps lower-priority pending work. Cancellation/non-renewal at or before a trial or term boundary suppresses conversion/renewal before either can commit. Priority orders work; it does not backdate any successful application to the scheduled boundary.
3. Re-run all guards through the Foundation. An operator can unschedule before application; a customer-confirmed cancellation additionally requires an explicit versioned confirmed withdrawal/reschedule before its charging cutoff. Serialize against the intent and cutoff evidence; after cutoff, unapplied resource work does not permit retroactive restoration of charging. A successful firing publishes the actual committed boundary once.
4. Classify failure: transient dependency/resource provider uncertainty retries with bounded backoff and suppressions retained; denied/incompatible state becomes explicit failed intent plus operator finding. A state that is expected to clear, such as conversion while suspended, parks and pauses its firing deadline until resume.
5. If a terminal cancellation-intent failure occurs, renewal/conversion remains suppressed until explicit operator resolution. Failure cannot silently turn an opt-out into a new charge.
6. A terminal subscription sweeps incompatible pending intents, records their disposition, leaves already incurred debt intact; collection-pause windows are deferred.

## 3. Processes / Business Logic (CDSL)

### External work and stale confirmations

**RO-2026-10-15 integration.** [RO-2026-10-15](04-renewal-retention.md#7-renewal-and-offboarding-integration-contract) specifies frozen term/default evaluation and Contract-end handling, one renewal/dwell episode, durable cutoff versus actual stop/cancel, and independent retention/hold/delete/archive generations. Terms or payment events do not mutate running terms or reopen terminal state. Resource/hold adapters must prove stale-action suppression and hold-versus-delete arbitration at the actual execution boundary. Deferred notification delivery is not assumed complete. Exact provider mapping and RO-C01-RO-C16 execution remain open; no native LM/OSS dependency is introduced.


- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-algo-effect-recovery`

A resource transition owns an in-flight marker. Competing resource-affecting commands are refused or explicitly supersede it through a journaled cancellation/compensation process. Non-resource commands may proceed if they do not invalidate staged assumptions; completion rechecks the changed aggregate version. A work-order timeout means unresolved effect, not proof of non-execution. Late confirmations match the retained effect identity and either complete the still-current operation or start reconciliation/compensation. They cannot resurrect an applied withdrawal, terminal subscription or superseded operation.

Under accepted S-Q05, an explicit customer-confirmed cancellation charging cutoff is a separate financial commitment from the eventual guarded cancel/resource provider stop. A request receipt is not that confirmation. Customer cancellation is confirmed only after authorization, commercial eligibility and durable recording of its charging cutoff; confirmation is independent of resource provider completion. Withdrawal or rescheduling requires an explicit versioned confirmed action before the applicable cutoff, without retroactively restoring charges. After cutoff, new service needs a separate valid guarded route, not undo of the financial commitment. Worker delay or unknown stop cannot extend ordinary customer tariff beyond a known cutoff; a future scheduled cutoff does not waive normal service before it. Recovery preserves truthful runtime and operation/state evidence without backdating stop or applied cancellation. Only applied cancellation releases normal occupancy; cutoff alone permits neither archive nor deletion. Versioned confirmation, capture/compensation and correction contracts remain S-U05/S-U13 gates; retention and paid-order remedy paths are unchanged.

### Atomic occupancy admission

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-algo-occupancy-admission`

The user-accepted S-Q02 rule keys occupancy by `(payerTenantId, resourceTenantId, overlapScopeKey)` and requires explicit `maxConcurrentActive` from versioned policy, with retained source/version provenance. Admission counts actual active subscriptions + retained suspended places + explicit order reservations, each right exactly once and each count reported separately. Held payment-pending reservations count too. Missing policy refuses; there is no implicit limit of one. Confirmation, activation, suspend, resume, cancel and affected key changes serialize claim changes. Confirmed-order activation converts its own reservation to active without a second place; exclude that same right once. Other supported unreserved paths use ordinary atomic admission against all claims and cannot bypass the pre-capture safeguards if collecting upfront. Capacity N > 1 must be safe under concurrency; count-then-insert outside the serialized commit is invalid.

**Suspended-place rule accepted, 2026-10-07:** suspend converts the same subscription's active claim into a retained suspended place. Resume on its unchanged claim atomically converts that place back to active without competing for or consuming a second place. Thus active 0 plus retained 1 at limit 1 has no free capacity, and suspended-to-active preserves total claims. The guarantee covers absence of competition for the unchanged claim only; applicable payment, Policy, term and resource guards still apply. entitlement provider materialization remains asynchronous and separately observable after the atomic state/claim/posture-intent commit; the place guarantees neither usable access nor a billing waiver. Applied cancellation releases the claim; a pending, scheduled or failed cancel does not. Mere TTL/dwell expiry cannot release it: an accepted timeout policy must use normal guarded cancellation. Cancellation's resource retention or legal hold does not retain normal occupancy.

Supported composition key changes lock source and target keys in deterministic order and atomically move the claim with the commercial change. Refuse target conflicts while preserving the source claim; unchanged-key operations exclude self exactly once. A versioned limit decrease preserves all existing protected active, retained-suspended and explicit order-reserved claims, even when their total exceeds the new limit; it never evicts or releases a claim solely for that change. Reject new admissions that would exceed the current limit until room exists. Unchanged-key resume and activation consuming an existing reservation convert the same protected right without another slot; their other guards remain. Applied cancellation releases its own claim. Policy/key/claim movement requires an established destination/version mapping and atomic move; unsupported or failed movement preserves the source claim and refuses the move. Exact provider contracts remain open; import mapping applies only to the separate import capability.

A plain draft is not active, retained suspended or automatically reserved. Explicit confirmation creates a durable order-bound reservation that may later attach to the intended draft/attempt. Paid confirmed reservations survive future-date waiting, holds, ordinary TTL and technical fulfillment failure; remediation or explicit cancellation under the financial contract resolves them. Unknown capture retains the reservation until the financial outcome is recovered. Cancellation must fence late capture/activation and record financial-remedy obligations before terminal release under the agreed contract. Unpaid abandoned-draft cleanup is housekeeping with a configurable TTL and no financial effect; it cannot release paid/uncertain rights. A confirmed unpaid prepay order keeps its place until the versioned payment deadline from the accepted order terms; after that only the order's cancel route releases the claim (accepted 2026-10-08).

Current Orders active-count-only observations and release-on-leaving-active semantics cannot represent all three claim states. Versioned producer/consumer evidence, future commercial commitment and capture/cancel fencing must align before enabling the prepayment profile, with refusal **before capture** when unavailable. A read preview is not a reservation, and reservation/receipt alone creates no service facts, access or service period. Linked subscription replacement and its predecessor occupancy exemption are deferred. Future-dated Orders bookings still count from confirmation; technical attempt succession remains distinct. Exact policy/key movement mappings remain open under S-U10; S-U01 still governs attempt rebind/successors and capture association.

## 4. States (CDSL)

### Lifecycle and operation state

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-state-foundation`

| From | Operation / to | Mandatory guard |
| --- | --- | --- |
| draft | activate / active | Applicable accepted-terms/readiness/Policy/activation guards. Confirmed-order profile consumes its reservation once; other supported unreserved paths use ordinary atomic admission and cannot bypass pre-capture safeguards if collecting upfront. |
| draft | immediate cancel / cancelled | No supplied service; confirmed-order rights additionally need capture/consumption fencing and recorded financial remedy under the agreed cancellation contract. |
| active | suspend / suspended | Policy and confirmed pause where required; atomically retain the same occupancy place; no resource deletion. |
| suspended | resume / active | Policy and applicable payment/term/resource guards; atomically convert its retained unchanged-key place to active without another slot. entitlement provider access remains separately observable. |
| active or suspended | cancel / cancelled | Accepted reason/mode, service-stop and retention disposition; release normal occupancy only at applied cancel commit. |
| cancelled | archive / archived | Archive dwell, operational-data completion and evidence-retention constraints. |

Operations use `pending → approved → applied` or `pending/approved → failed`; approval means staged permission, not commercial success. Same-key replay reads the existing operation while it progresses. Applied receipts are immutable.

`changeComposition` follows feature 03's non-terminal source-state matrix and declared operation profile: an active target keeps its ordinary service posture; a suspended target remains suspended with restricted grants and its accepted suspension-billing posture. Draft changes remain blocked until S-U01 accepts the immutable acquisition-plus-delta receipt, hold and withdrawal/recovery protocol; the selected S-Q01 direction and still-open versioned activation contract govern affected first activation. Direction approval does not settle these draft-change semantics. Once enabled, a draft edit changes only draft composition and its revision, preserves acquisition/delta evidence and emits no activated service, usable grants or billable facts. No composition change implicitly activates or resumes the target. Single-item wrappers apply only their explicitly supported source-state/profile combinations; they do not bypass this matrix.

Manual renew permits suspended only for a documented blocked-renewal recovery. Transfer and confirmAcceptance are unavailable. Unschedule requires a pending supported cancellation intent and, for confirmed cancellation, the additional pre-cutoff versioned-confirmation guard. Trial-specific guards are in feature 06.

## 5. Definitions of Done

### 5.1 Recoverable lifecycle and measured admission

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-dod-foundation`

The implementation must enforce one scoped replay/business fence and commercial commit path, persist every asynchronous effect and outcome across restart, reject closed capabilities before side effects, and expose status/settle recovery without mutation re-submission. Audit contains actor, correlation, reason, input digest, expected/actual revision and denied/failed attempts; authorization precedes replay. Manifest and SDK operation names must match the agreed command inventory.

**NFR acceptance evidence:** use the authoritative proposed [PRD §7 profile](../PRD.md#7-non-functional-requirements): 100,000 active subscriptions in one large tenant, one million across ten tenants, one hour at 100 mutation submissions/s and 1,000 reads/s, then a five-minute twice-rate burst. Record hardware/storage, composition/resource distribution, hot-key contention, offered/completed rates and healthy-provider p99 latency (proposed maximum 250 ms). Report durable intent acceptance and local application without an external service leg separately against the proposed p95 < 1 s / p99 < 2 s targets. Remote provisioning/materialization has its own elapsed-time metrics, never an implied successful activation.

Measure bounded authorized reads against the ratified p95 < 200 ms / p99 < 500 ms and commit-to-consumer event availability against the ratified p95 < 30 s / p99 < 60 s with a healthy broker. The report must cover every operation outcome, including pending and overload, and state the outstanding-operation bound and explicit back-pressure response. Dependency fault injection must demonstrate recoverable receipts and source-linked metrics/findings for oldest pending operation, external-leg age, outbox age, attribution gaps, overdue groups, unsealed scopes and retention/deletion backlog. Capability handlers supply their metrics at integration acceptance. These thresholds are the ratified v1 targets (S-Q07, 2026-10-08); readiness requires measured evidence, not this document alone.

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
- [ ] Unknown capture or payment timeout retains the claim. Cancellation racing late capture and activation reaches the agreed fenced outcome, records the required financial-remedy obligation and never silently frees a paid/uncertain right.
- [ ] Suspend racing a new activation at limit 1 never exposes a free place: active 1 becomes active 0 plus retained 1. Resume on that unchanged claim returns to active 1 without consuming another place or competing with the new activation; other resume guards still apply.
- [ ] Pending, scheduled or failed cancellation retains the active/suspended claim. Applied cancellation releases it atomically; subsequent resource retention or legal hold does not keep normal occupancy claimed.
- [ ] Policy outage leaves commercial state unchanged and a recoverable request outcome; no database transaction spans the remote wait.
- [ ] External success followed by a stale completing revision creates a durable compensation finding; restart resumes it.
- [ ] An resource provider-blocking command increments commercial revision exactly once at application; receipt acceptance is never returned as active service.
- [ ] Failed cancellation intent continues suppressing renewal and paid conversion until explicit resolution.
- [ ] Direction approval alone does not enable activation. Capabilities awaiting the S-Q01/S-Q02 owner contracts, applicable residual claim rules or runtime evidence return an explicit refusal with no resource, price-hold or financial side effect.
- [ ] The S-Q07 load report exercises the full ratified steady/burst profile, same-key retries, changed-payload conflicts and hot occupancy keys; durable receipt and local-application p95/p99 are reported separately and checked against the ratified profile before release.
- [ ] Saturating the declared outstanding-operation limit yields an explicit retryable back-pressure response or the existing receipt. No accepted operation is dropped; timed-out create can still be read/settled without another effect.
- [ ] Bounded-read and event-availability measurements include the proposed 200/500 ms and 30/60 s percentile targets under their declared healthy-provider/broker conditions. Outages instead produce visible pending/retry outcomes without granting access.
- [ ] Injected stuck work in every allocated observability class is discoverable through its metric and an authorized, source-linked operation/finding read; current version/checkpoint is visible rather than hidden as fresh state.
- [ ] Active and suspended composition changes retain their source posture; draft changes refuse until the S-U01 protocol is accepted, then preserve both accepted receipts without creating service, usable grants or billable facts before separate activation.
