---
description: "Protected purchase acceptance, actual service activation and recoverable Orders integration."
---

# Proposal: Accepted Purchase and Subscription Activation

**Scope.** New paid and trial subscriptions originate through Orders. These operations are deferred:

- Trial extension and repeat trials.
- Standalone acquisition without Orders.
- Scheduled composition changes and scheduled resume.
- Linked predecessor/successor subscription replacement.
- Changes of an existing subscription's base plan.

Reject these operations before effects, including through operator/direct wrappers. Supported operations remain existing-subscription lifecycle actions, immediate quantity increases/component additions, future order starts, cancellation scheduling, renewal and normal trial timers. Technical retry/successor attempts recover the same acquisition. They do not replace an existing subscription.

Free-period and credit trials, normal expiry/conversion and early conversion to the already accepted paid target remain in scope with their Pricing/Billing/financial gates. Trial-to-paid conversion is not a general changePlan capability. Preserve trial history/eligibility evidence and refuse repeat requests for the same payer/plan, including via a new subscription; a transport retry is not a repeat trial. No extension or repeat becomes available from a Pricing policy alone.

Independent cancellation and new purchase have separate resource/data lifecycles: there is no automatic resource adoption, data transfer, place handover or continuity promise. Cancellation retains its existing retention/disposition rules; it does not mean immediate deletion. Unsupported plan changes cannot silently become cancel+new.


<!-- toc -->

- [1. Context and source evidence](#1-context-and-source-evidence)
- [2. Decision drivers and scope](#2-decision-drivers-and-scope)
- [3. Alternatives](#3-alternatives)
- [4. Selected commercial time semantics](#4-selected-commercial-time-semantics)
- [5. Accepted acquisition authority](#5-accepted-acquisition-authority)
- [6. Proposed sequence and local application](#6-proposed-sequence-and-local-application)
- [7. Boundary failures and recovery](#7-boundary-failures-and-recovery)
- [8. Two-line order and partial fulfillment](#8-two-line-order-and-partial-fulfillment)
- [9. Owner changes and consequences](#9-owner-changes-and-consequences)
- [10. Joint acceptance scenarios](#10-joint-acceptance-scenarios)
- [11. Confirmation and remaining decisions](#11-confirmation-and-remaining-decisions)
- [12. Source and traceability index](#12-source-and-traceability-index)
- [13. Orders and Pricing reciprocal activation contract](#13-orders-and-pricing-reciprocal-activation-contract)

<!-- /toc -->

**Current design.** Pricing validates and freezes the purchase terms when the purchase is accepted. A durable accepted acquisition, including its agreed future start and any explicit execution limits, is the commercial basis for fulfillment. Subscriptions owns actual activation T and checks the current order, retained financial coverage, authorization, occupancy and held readiness before its local commit. Ordinary price changes, offer expiry after purchase or catalog retirement for new sales do not invalidate the accepted purchase.

Exact owner SDKs, durable acceptance/capture fences and runtime evidence remain delivery work.

**Initial readiness.** Under SUB-D-55 and SUB-D-57, preparation stays on a draft subscription and Entitlements retains the required assignments under a durable readiness commitment until the exact activation outcome is recovered. Applied continues those assignments; a fenced terminal non-activation outcome permits authorized cleanup. Unknown results never authorize timeout cleanup. Emergency security denial remains immediate and observable. Authorized preparation use before T is platform-borne under bounded profiles. See the [consumer protocol](features/05-entitlements.md#initial-activation-readiness-contract).

## 1. Context and source evidence

The following contracts disagree on who chooses the activation time.

| Source | What it currently says | Consequence |
| --- | --- | --- |
| [Orders Lifecycle upstream §2.1](../../orders-lifecycle/docs/UPSTREAM_REQS.md#21-subscriptions) | An explicit activation start wins over an internally computed instant; the earlier quoted service date must not backdate billing or entitlement. | Baseline mismatch; AP01 proposes lower-bound semantics to the Lifecycle/Workflow owners; their documents are unchanged. |
| [Subscriptions PRD](./PRD.md), [DESIGN §3.3.2](./DESIGN.md#332-pricing-acceptance-and-accepted-time) | The S-Q01 contract uses the Subscriptions activation transition as the commercial start and retains distinct booking and acceptance instants; the versioned cross-owner contract remains open. | A caller timestamp chosen before asynchronous fulfillment cannot automatically stand for the eventual transition. |
| Current [Pricing SDK](../../pricing/pricing-sdk/src/acceptance.rs) and [fulfillment implementation](../../pricing/pricing/src/infra/commercial_terms/fulfilment.rs) | First hold freezes one `activation_at` and the original bindings. Both hold and fresh fulfillment eligibility must agree with that time. | A later Subscriptions-selected time cannot be substituted into the V1 contract. |

The current Pricing implementation behaves as follows:

- `PricingAcceptanceV1::hold` checks for an authorized exact command replay before fresh capture and again inside its transaction. Exact replay returns the historical held result even after its admission window has expired.
- A first hold, or a hold under another command key, performs fresh checks. Its transaction calls `eligible` and `same_activation`. `same_activation` rejects an activation timestamp different from the first stored hold with `AcceptanceMismatch`.
- `SellabilityV1::check_fulfilment` also calls `eligible` and **the same `same_activation` function**. A fresh read cannot authorize a newly selected timestamp after a different first hold.
- Fresh eligibility checks the original acceptance, tenant axes, terms digest, market, live SKU retirement and the current rows of the original accepted prices. It does not traverse successors or replace accepted bindings. The current price's own temporary or explicit end constrains eligibility; an informational successor cutoff alone is not that end.
- Current SKU state is read from Products during detached capture, before the Pricing transaction. `prices_unchanged` fences Pricing's local price rows only; it does not prove that a SKU could not retire between that Products read and a later grant decision.
- Current fresh admission requires server time before `hold_until` and `start_at <= activation_at < hold_until`. Price ends are exclusive too. Equality with an upper boundary is ineligible.
- `FulfilmentEligibility.checked_at` and `valid_before` describe an observation. The SDK explicitly excludes use as an admission token. Another hold key cannot move the first activation timestamp or extend its deadline.
- Pricing's acquisition identity is seller/catalog tenant plus order/version/line, without an execution attempt; the stored first hold is unique by acceptance. The target contract preserves acquisition-level uniqueness while binding the authorized execution attempt.
- The current acceptance deadline is acceptance-time server `now` plus the selected seller hold duration, not the requested service date plus that duration ([issuance implementation](../../pricing/pricing/src/infra/commercial_terms/check.rs)). A future purchase therefore requires a supported durable acceptance contract, not an extended quote hold.

Future purchases therefore need a durable acceptance contract. Replaying a quote hold or rechecking live eligibility cannot provide that guarantee. This proposal provides no production workaround using the current API.

[Orders Lifecycle's Workflow reconciliation](../../orders-lifecycle/docs/UPSTREAM_REQS.md#26-orders-workflow) additionally requires committed `report-spawn-signal` before activation dispatch, with ambiguous replies recovered; the cited Workflow PRD still describes the first intent as that signal. The activation contract preserves the common two-wave barrier and per-line outcomes; AP06 requires committed signal acknowledgement before dispatch. The committed signal fences direct order cancellation. Later Workflow holds and mediated cancellation keep their own guards.

## 2. Decision drivers and scope

The design must honor accepted purchases through future-date waits and ordinary catalog changes. It must set actual service start locally after readiness, protect capacity and money while outcomes are unknown, and prevent duplicate acquisition or activation. New acquisition originates in Orders. Requested dates are scheduling lower bounds; any latest-start condition must be an explicit accepted purchase term.

Renewal and supported composition changes retain their own accepted-term rules. Changed or unaccepted terms require a supported fresh acceptance. Import, standalone acquisition, linked subscription replacement, base-plan changes and trial extensions remain outside current scope. No fixed preparation duration, all-lines simultaneous start or refund amount/timing is introduced.

## 3. Alternatives

| Approach | Consequence |
| --- | --- |
| Caller fixes actual service start at order creation | Preparation can finish later, so the supplied date cannot prove actual service start. It remains a scheduling lower bound. |
| Re-evaluate current catalog eligibility at activation | A paid future purchase could be rejected or repriced because the offer changed. This does not implement the accepted-purchase contract. |
| Execute the accepted purchase with current operational guards | Selected: immutable commercial evidence supplies the terms; Subscriptions checks order/cancellation, retained financial coverage, authorization, capacity, readiness and accepted start restrictions, then records actual T. |

## 4. Selected commercial time semantics

| Instant | Authority and meaning |
| --- | --- |
| Requested activation date/time | Accepted scheduling lower bound; Workflow retains its common date barrier. It is not proof of service. |
| Purchase acceptance time | When the original purchase terms and required future coverage became durably accepted. An offer/assessment deadline applies before this boundary. |
| `contractEffectiveAt` | Contract booking/effectiveness evidence, independent of fulfillment delay. |
| Readiness observation time | Diagnostic provider evidence; the operation/manifest-bound readiness commitment supplies retention authority. |
| **T / `serviceActivatedAt`** | Fresh server time selected at the final guarded decision of Subscriptions' successful local activation transaction. It becomes authoritative only if that transaction commits. |

T is the commercial effective instant recorded with application, not the physical WAL/fsync instant. After acquiring the local fences and validating the accepted acquisition and current guards, sample T and check the requested lower bound and any explicit purchase start constraint. State, acquisition/claim application, readiness resolution, receipt, facts/posture and outbox commit together. There are no remote calls between that final decision and commit.

A rolled-back transaction has no activation. A new transaction samples a new T and reruns its guards; an unknown outcome is recovered first. Successful recovery returns the original T without executing again. Transaction duration and clock health follow the bounded runtime profile; an old sample cannot be carried across restart or an abandoned transaction. Exact clock margins are needed only for actual contractual or financial time bounds, not an invented commercial deadline.

The start restrictions come from the accepted purchase. If it includes only a lower bound, Subscriptions must not manufacture a latest-start deadline from an old quote, hold or current price end. If an explicit latest-start condition cannot be met, preserve the accepted obligation and apply its agreed delay/remediation/cancellation route. Offer validity is evaluated at purchase acceptance; a later offer expiry does not shorten execution rights already bought.

All initial usable-service prerequisites and the held readiness commitment precede T. The first service period/accrual starts at T. Authorized pre-T preparation use remains platform-borne under SUB-D-57; ordinary later provider failure does not backdate or erase the applied acquisition.

## 5. Accepted acquisition authority

**Purchase acceptance.** Pricing and Products validate the intended purchase and its requested future date when it is accepted, and retain complete immutable terms and durable fulfillment coverage. Orders records the accepted acquisition and its evidence. The versioned acceptance contract must distinguish an offer/assessment from an accepted purchase; copied terms or a V1 hold with only quote-lifetime semantics are insufficient evidence of a durable future promise.

**Protected capture.** Before Billing/Payments capture, the confirmed order has both a durable capacity claim and the accepted commercial commitment covering every funded line. The commitment may be part of the acceptance receipt or a correlated durable record; its guarantee, not the number of records, is required. Missing either protection prevents capture. Issuance orders acceptance against Products retirement and Pricing changes; after acceptance, ordinary publication/price changes and retirement for new sales cannot retract the purchase.

| Required evidence | Invariant |
| --- | --- |
| Acquisition identity | Authorized seller/payer/resource scope, order/version/line, immutable purchase identity and terms digest; exclusive attempt/subscription association is recorded separately. |
| Frozen terms | Original bindings, quantities, product/policy/invoice descriptors and BillingTerms remain recoverable at their accepted versions. Activation does not select successor prices. |
| Accepted scheduling conditions | Requested start, agreed future coverage and explicit latest-start/delay rules only where the purchase includes them. Quote/assessment expiry is not an execution deadline after acceptance. |
| Protection and financial evidence | Durable capacity claim, commercial acceptance/commitment and confirmed payment allocation or accepted postpay commitment for the exact acquisition and agreed future service. Establish this basis during acquisition and retain it until application or an explicit authorized disposition; activation does not repeat financial eligibility assessment. |
| Local application fence | One Subscriptions decision for the acquisition/attempt association and preparation generation, serialized against cancellation/settlement; replicas share the same durable fence. |
| Applied result | Actual T, subscription revision, accepted terms digest, consumed own claim and readiness resolution, committed with receipt/outbox. |
| Historical recovery | Authorized immutable acceptance and application outcomes survive response loss, catalog changes and restart. Missing evidence is recoverable uncertainty, never permission to reprice or duplicate a purchase. |

Subscriptions obtains the accepted evidence through its public SDK/read contract or saved immutable snapshot. A required missing historical read leaves recovery pending; an available complete snapshot needs no fresh live-catalog verdict. The execution decision belongs to Subscriptions and the relevant order, security and financial owners. A cancelled/refunded or otherwise invalidated purchase follows their explicit versioned disposition; current catalog state is not such a disposition.

The order claim survives future-date waiting, draft rebuild and unknown payment/application outcomes. Unconfirmed unpaid draft cleanup remains housekeeping; confirmed unpaid orders follow their accepted payment deadline and Orders cancellation route. Paid unfulfilled purchases retain a remediation obligation or explicit agreed cancellation with recorded financial remedy. Acceptance and prepayment create no activation-driven service facts. Changed terms require supported new acceptance, and a technical attempt retry never becomes a new sale.

## 6. Proposed sequence and local application

```mermaid
sequenceDiagram
    participant W as Orders Workflow
    participant O as Orders Lifecycle
    participant P as Pricing / Products
    participant B as Financial owner / Payments
    participant S as Subscriptions
    participant E as Entitlements
    participant D as Subscriptions DB
    W->>P: Accept purchase and freeze original future terms
    W->>S: Secure durable order/version/line capacity claim
    W->>O: Confirm accepted purchase and both protections
    O->>B: Protected capture authority through outbox, where prepay requires it
    Note over W,B: Acquisition retains confirmed allocated payment or accepted future postpay commitment
    W->>S: Create drafts, then activate after date/wave barrier and committed spawn signal
    S->>D: Authenticate; recover or save operation and exact acquisition association
    S->>B: Recover original financial evidence only if missing; no new approval
    S->>D: Save authorized preparation posture and outbox
    S->>E: CommercialPostureV1 prepare_activation
    E-->>S: Aggregate readiness with held commitment
    S->>D: Final local guards against accepted purchase and current fences; sample T
    S->>D: Atomic active + T + own-claim conversion + resolution + receipt + outbox
    S->>E: Applied posture and activationResolution
    S-->>W: Durable applied result or explicit pending/refused outcome
```

1. Confirmation obtains durable accepted future terms and capacity before protected capture. Acquisition establishes confirmed allocated payment or the accepted postpay commitment covering the agreed future launch. An unpaid confirmed order remains pending its prescribed payment. Financial uncertainty retains both protections. Workflow creates drafts first, preserves its common requested-date barrier and obtains/replays the committed spawn signal before activation dispatch.
2. Subscriptions authenticates before replay and recovers the existing acquisition/attempt outcome before new effects. It validates the exact accepted purchase, immutable terms and exclusive order/version/line/attempt association; current catalog price or offer validity is not re-evaluated for that purchase.
3. Before usable preparation, validate action-scoped Policy/delegation, order and applicable financial evidence. Persist the operation, preparation posture revision/digest and outbox before external work. Entitlements owns assignment/manifest preparation and held readiness; Subscriptions consumes the exact aggregate result. SUB-D-57 governs bounded exposure, platform-borne pre-T use and retained uncertain effects.
4. In the final serialized local transaction, recheck operation and association revision, cancellation/withdrawal, accepted composition and start restrictions, current authorization, the snapshotted preparation deadline, retained financial commitment/disposition fences and own reservation. Validate the saved readiness commitment and latest known evidence. Upstream order/capture decisions require the agreed local fence; an earlier detached read alone is insufficient.
5. Sample fresh T after those guards, apply the accepted lower bound and any explicit contractual start condition, and atomically write active, T, period start, claim conversion, activation resolution, commercial records, receipt and outbox. No remote call occurs under the DB lock.
6. Confirmed rollback leaves no activation and does not release protected rights by itself. Unknown commit outcome is recovered under the original fence before retry. A retry uses a new T; successful recovery returns the original T. Terminal non-activation is durably fenced before Entitlements cleanup; emergency security denial and later lifecycle restrictions retain their separate rules.
7. Orders consumes actual applied per-line outcomes, not transport acceptance. A lost publication is repaired by the authorized operation read. Paid obligations and preparation remain recoverable until their respective operational and financial settlements.

## 7. Boundary failures and recovery

**Preparation deadline and attempt outcome.** The service profile defines a versioned preparation-duration policy. At the durable authorized start of an activation attempt, Subscriptions snapshots the selected policy/version, start and evaluated deadline with the operation and preparation posture. Waiting for a requested future start is not preparation time. Dispatch retries, polling, restart and later profile edits cannot move this attempt's deadline.

This is a technical attempt bound, not expiry of the accepted purchase, payment, credit commitment or protected capacity claim; explicit purchased start conditions remain separate.

The local activation decision checks that deadline under the supported clock/runtime profile together with the other guards. After it expires, that attempt cannot newly activate even if the timeout worker has not run or a late ready observation reports an earlier timestamp. The deadline worker requests/executes closure at the same Subscriptions operation/preparation-generation fence as activation. The saved profile distinguishes retryable observations from definitive preparation failure.

Retryable/unknown results recover the same attempt within its deadline; only a definitive failure uses the same fenced closure path. If activation already won, recover its applied T and result; later polling or timeout cannot rewrite it as failed or unapplied. Otherwise Subscriptions durably records `closed_without_activation` with the timeout/failure reason and withdrawal outbox. Missing/unreachable outcome or an uncommitted closure request remains unknown, not a terminal receipt.

Entitlements may request closure and report evidence but cannot turn its timer or failed observation into non-activation proof. It retains held assignments and recovers the exact resolution. Only a proved closed generation permits authorized cleanup; quota occupancy still waits for confirmed resource release, and trial coverage is settled under its exact commitment. Late ready, allocation or cleanup results remain correlated recovery work. Cleanup and its outstanding effects stay visible independently of the terminal attempt outcome.

Orders handles the closed attempt through remediation while retaining the purchase, accepted terms, protected claim and applicable financial basis. Timeout alone does not cancel the order, release purchased rights, create a refund or incur service-period charges. A successor requires the existing authorized same-acquisition protocol: terminal predecessor non-application, fenced delayed commands, resolved financial applicability and settled or safely quarantined external effects, with generation protection against old cleanup.

It retains the original purchase/payment/benefit lineage, creates no second claim or bonus and does not extend an explicit purchased start limit. Unknown predecessor outcome cannot authorize it. If activation won, continue that applied subscription; stopping it requires its normal guarded lifecycle action. Commercial cancellation and financial remedy remain an explicit separate route. Numeric durations, provider mappings and executed race/recovery evidence remain delivery work.


Linked subscription replacement is deferred. This protocol applies to Orders-origin acquisitions and recovery of their attempts; it does not hand an existing subscription's resources, data or occupancy to a new sale. A successor execution attempt is not a successor active subscription.


| Boundary / race | Required result under the proposal |
| --- | --- |
| Requested date or required readiness has not been reached | Keep the operation pending and retain protected acquisition and capacity; no activation facts or timer-only release. |
| First V1 hold fixed A; a call asks for B ≠ A | Current `hold` and `check_fulfilment` both reject the mismatch. Do not retry a different key as a workaround; the proposed capability is separate. |
| Exact V1 hold replay after expiry | Recover the historical V1 record; establish accepted-purchase coverage through the supported contract before acquisition. A quote hold is not a durable future purchase. |
| Purchase not accepted before quote expiry, price closure or SKU retirement | Refuse new acceptance/capture under the offer rules. An already accepted purchase retains its original terms despite later ordinary catalog drift. |
| Capacity/accepted commitment response lost or capture outcome unknown | Recover the same business identity and owner evidence. Unknown capture is not unpaid and cannot release the claim or create a second debit. |
| Explicit accepted start bound is unmet, or its required clock evidence is unavailable | Do not invent an earlier T. Preserve the purchase and follow agreed delay/remediation policy. With no contractual latest-start bound, do not manufacture one from quote expiry. |
| Confirmed rollback, or uncertain transaction outcome | Rollback creates no activation; recover unknown outcome under the same local fence before a fresh decision. Missing receipt alone is not proof that a late commit cannot appear. Retry samples fresh T. |
| Local activation commits; response or event is lost | Recover the original applied receipt/T and replay the same outbox records. No second activation or reassigned start occurs. |
| Final transaction is delayed or times out | Follow the bounded runtime completion/recovery contract. Report success only from a committed result; recover uncertainty before retry and never carry an old T into another transaction. |
| Withdrawal/cancellation or settled-no-create wins the local activation fence | Reject subsequent activation consumption. A no-draft/void outcome alone does not release an order claim or settle unknown capture. Paid release requires agreed cancellation and recorded financial remedy, fenced against late capture/activation; late callbacks cannot resurrect a cancelled right. |
| Activation wins before withdrawal | Return the applied T and subscription. Withdrawal cannot rewrite it as never activated; an explicit cancellation/compensation operation is required. |
| Order version, attempt or frozen composition differs | Reject before consumption. An old authorization cannot be moved to a new order version, replacement draft or attempt. Concurrent upstream state changes need the agreed local fence, not a last-minute read alone. |
| Ordinary catalog closure/retirement occurs after purchase acceptance | Execute original accepted terms under current order, financial, security and readiness guards; new sales follow current catalog eligibility. |
| Emergency security denial races activation | Deny access immediately under its independent authority and publish incident evidence. Known denial prevents commit; a racing applied T remains historical truth for subsequent guarded handling. |
| Required initial access is pending, failed or has stale/withdrawn evidence | Keep initial activation unapplied and subscription draft; reconcile the same assignment/operation, record partial-effect cleanup when needed, and preserve protected purchase rights. Nonessential/ongoing post-commit delivery retains its separate asynchronous contract. |
| Same transport command or different key within the same business attempt | Recover/conflict under the same accepted acquisition and local activation fence; no new sale, claim or monetary effect. A new attempt requires the supported association/settlement protocol. |

**Proposed acquisition evidence and transition contract: S-U01/S-U07/S-U10**

This matrix defines the proposed commands and their required evidence. The names describe behavior; they are not shipped endpoints or ratified wire types. Each command carries authenticated seller/payer/resource scope, stable business identity, expected relevant revisions, payload digest and correlation. Each receipt identifies the durable operation, exact accepted evidence versions, current disposition, outstanding owner effects and replay/conflict result. `pending` proves receipt of work, not protected capacity, capture, activation or compensation.

An observation of another owner's state is not a fence against its concurrent mutation.

| Stage / proposed request | Evidence and owner boundary | Receipt, refusal and recovery |
| --- | --- | --- |
| Confirm protected acquisition | Orders-origin order/version/line, plus intended customer/tenant scope, overlap key and versioned capacity policy; Subscriptions claim receipt; Pricing/Products original-term commitment with accepted future coverage, binding/policy digests and validity. Either guarantee may be established first, but neither proves the other | Confirmation exposes both durable owner references before capture can proceed. A partial or unknown result remains pending and is recovered by the same acquisition identity; no combined remote transaction or guessed guarantee. |
| Capture under protected evidence | Billing/Payments command identifies the financial obligation/capture identity and current claim plus term-commitment fences. The accepted arbitration below orders capture start against cancellation in Orders Lifecycle and preserves owner protections through execution/recovery; detached reads do not suffice. Versioned provider/release fencing still needs ratification. Payment authorization is distinct from captured funds | Return the same durable capture outcome on retry; unknown remains recoverable and preserves protected rights. A refused/missing guarantee prevents debit. `tolerate_payment_failure` cannot bypass either guarantee. No retry changes the capture identity to collect again |
| Attach/create an authorized operation | For Orders, exact `(order_id, order_version, attempt_id, line_id)` create identity and protected-acquisition reference plus expected attachment version. Claim lifetime remains independent of draft/transport retry | Replay of the same business operation returns its retained draft/outcome or payload conflict. Orders `settle_create` uses its exact attempt identity; standalone create/settlement is deferred. A missing read is not proof of non-creation. New attempt/version/rebind requires the applicable supported predecessor/successor contract; unsupported mapping refuses |
| Admit and apply activation | Exact accepted purchase/terms and current order/attempt association, authorization, retained acquisition financial coverage, own-claim evidence and held initial readiness; any contractual start limits | Sample T and atomically record state, acquisition/claim application, readiness resolution, receipt and outbox. No Pricing or financial re-admission or remote call in this transaction. Unknown result recovers the original decision. |
| Settle unapplied work or cancel a paid unactivated acquisition | Same authorized acquisition/operation (and Orders attempt where applicable) with explicit application/capture suppression fences; authoritative financial outcome or visible uncertainty; resource-effect settlement and the required financial-remedy obligation | No-create/non-application evidence cannot erase an already applied result. Unknown capture/application prevents silent release; recover the original identities. Reserved-right release requires the agreed owner acknowledgements and release conditions. Apply the accepted arbitration/release conditions below. Recording a remedy is neither posted refund nor a universally sufficient release condition; exact owner acknowledgements and SDK contracts remain ratification work |
| Cancel after application / complete partial order | Applied subscription/line operation and accepted cancellation/compensation identity; normal applied cancellation releases its active/retained place under S-Q02. Preserve the source acquisition, financial-remedy and resource-effect lineage | Keep per-line applied/failed/pending/compensating outcomes. The default partial-order response is remediation; do not silently cancel a successful line. If the accepted saga chooses failure with compensation, recover each required applied-line compensation before claiming order failure. A financial obligation acknowledgement is distinct from a posted credit/refund |

The proposed local projection distinguishes evidence pending, protected, capture pending/known, attempt attached, application pending/applied and settlement pending/settled. These are operation/evidence dispositions, not new subscription states or a distributed transaction. A version conflict returns the winning receipt and required recovery path; an unsupported predecessor policy returns explicit refusal. A new transport command cannot advance a business stage without its source evidence or reset original accepted purchase conditions.

Ordinary paid-right protection, suspended occupancy and trial commitment/effective-grant/recording separation remain intact.

**Capture and cancellation arbitration (S-U01/S-U07/S-U10).** For an Orders-origin acquisition, Orders Lifecycle owns the durable decision authorizing capture to start and orders it against cancellation of that acquisition. Workflow delivers and recovers the work; Payments executes the money movement and supplies authoritative outcomes, while Billing owns the applicable financial remedy. Subscriptions owns the reservation and activation fence, and Pricing/Products own the durable original-term commitment.

This is an agreed target protocol, not an existing Payments API: [Orders' current Payments request](../../orders-lifecycle/docs/UPSTREAM_REQS.md#25-payments) covers authorization and explicitly excludes capture/refunds.

1. Establish both durable protections before requesting capture start. Bind the capture business identity to the exact acquisition/order version, covered lines, financial obligation, protected claim and term-commitment versions, authenticated scope and payload digest. A whole-order capture requires protection for every covered line. These owner receipts must represent protections that cannot independently expire or be released while capture can still execute or its outcome is unknown; detached successful reads do not suffice.

   Unsupported revocation/release fencing blocks this profile.
2. Orders Lifecycle atomically records either cancellation winning before capture start, or capture-start authorization plus its durable dispatch intent, using the same acquisition/version concurrency guard. Validate the applicable order state and evidence in that local transaction; make no remote calls inside it. This is the logical start boundary, not proof that a bank has received a request or taken money.

   Authorization is bound to this capture identity and cannot be reused for another order version, line set or amount. A crash before commit creates no dispatch authority; a lost response after commit recovers the same decision and outbox work.
3. Payments accepts capture only under that recorded authority and deduplicates execution by the original capture business identity, independently of transport retries. Same identity with changed content conflicts. Its durable execution record and provider recovery must prevent a duplicate debit after a crash or lost response; a caller-side read or expiring HTTP idempotency cache alone is insufficient. Unknown remains pending recovery of this capture, never a new charge identity or definitive failure.
4. If cancellation wins before capture start, retain the cancellation decision so delayed start/dispatch commands cannot obtain or use authority and cannot debit. If capture start wins, cancellation remains available through the applicable order cancellation route; financial settlement can remain pending. Ask Payments to suppress outstanding execution where supported. Only authoritative, durable non-execution fencing permits a no-debit conclusion; otherwise recover the started capture, including a late success, and record the required Billing remedy.

   Starting capture adds no new prohibition on cancellation and gives no promise that money will not be taken after cancellation was requested. Existing direct versus workflow-mediated cancellation eligibility still applies.
5. Cancellation must also reach the Subscriptions operation/activation fence. A late activation cannot resurrect a cancelled acquisition; if activation already won its local fence, use normal applied-subscription cancellation/compensation. Release the unactivated acquisition's protection only through the coordinated release operation with authoritative capture resolution, acknowledged non-execution/no-additional-capture and activation/association fences, and the required recorded financial-remedy obligation. Unknown capture or activation retains protection.

   A draft void, payment deadline, cancellation notification or recorded remedy alone is not sufficient release evidence. A posted refund is not a universal prerequisite, and no refund amount or timing is selected here. Owner-specific release acknowledgements and financial evidence remain versioned contract work.

Standalone acquisition without Orders is deferred. Postpay without an acquisition-time debit continues under accepted terms and authoritative service authorization; it acquires no immediate capture or card prerequisite from this protocol.

**Capture race fixtures:** cancellation wins before start and delayed dispatch refuses; start wins then cancellation precedes provider dispatch; capture succeeds but its response is lost; restart after the Orders decision before dispatch; duplicate delivery and conflicting payload under one capture identity; unknown capture at the payment deadline; late capture success during cancellation; attempted claim/commitment release while capture is unresolved; cancellation racing activation; and multi-line capture with one missing protection.

Assert one financial execution, retained protection while uncertain, durable winning receipts and separately visible cancellation/remedy completion. SDK encoding, mirrored owner acceptance, provider recovery and executed fixtures remain release gates.

**Execution retry and successor attempt (S-U01/S-U07/S-U10).** A technical retry keeps the original attempt and operation identities and recovers their durable outcomes. A new transport key does not create a new attempt, subscription, purchase, reservation, capture or charge occurrence. A new fulfillment attempt is a separately authorized successor operation, never an idempotent replay.

The initial recovery profile preserves the same accepted order version/line and commercial acquisition; amendment, a new purchase after commercial cancellation and changed terms remain separately guarded operations.

1. Workflow recovers the predecessor by its exact business identity and all associated create, activation, external-effect and capture references. A missing notification/read, elapsed timeout or expired worker lease is not settlement. Resolve unknown application and financial outcomes before admitting the successor. If the predecessor already applied, recover that subscription and continue its existing fulfillment/remediation; do not create another subscription as a technical retry.

   A later applied cancellation does not retroactively turn it into an unapplied attempt.
2. For a genuinely unapplied predecessor, obtain durable create settlement and activation suppression serialized with the predecessor's local business fence. `settle_create` may prove `NoDraft` with a retained tombstone or find an existing draft; finding a draft is not itself proof of non-activation. Settle/void that draft and outstanding activation work through the normal guarded path while preserving the separate acquisition claim and payment obligation.

   Retain the terminal non-application receipt and association generation so delayed create/activate commands cannot escape settlement. Settle or quarantine outstanding external work; resource reuse additionally requires effect-owner generation fencing so old cleanup cannot damage the successor.
3. Orders/Workflow authorize the successor for the same still-valid acquisition and expected predecessor/association revisions. Cancellation, a superseding order version or incompatible in-flight work refuses admission. Record predecessor-to-successor lineage and explicitly retain the original claim, original-term commitment and financial-obligation/capture references. Billing/Payments must confirm the existing financial disposition is applicable, including any pending cancellation/refund/credit remedy; a settled or remedy-bound payment cannot silently become available funding.

   Creating a successor is not permission to collect again or to invent a new setup occurrence.
4. Subscriptions atomically consumes the predecessor-settlement evidence and compare-and-swaps the acquisition association to the successor attempt, retaining the same claim identity and count, with the durable association receipt and outbox. Only one successor can win. Cancellation/withdrawal and reassociation must serialize at this same acquisition fence; upstream authority must be valid for that commit under the agreed protocol, not merely observed by a prior read.

   A failure before this local commit preserves the prior association and protected claim; a lost reply after commit recovers the same successor. This transaction changes local association and retained references, not remote money or Pricing records. Provider acknowledgements are coordinated outside it; partial/unknown provider association keeps successor activation pending without reopening the predecessor or releasing the claim.
5. Activation needs the same accepted acquisition and terms, current authorization/financial/order guards, held readiness and the designated successor association. The predecessor must be irreversibly fenced before the successor can apply. A new attempt cannot change accepted terms, duplicate acquisition or extend an explicit contractual start bound. Unsupported association remains pending/refused with paid-obligation recovery; rebinding never fabricates service during the failed attempt.

Standalone acquisition is deferred; this recovery protocol uses the original Orders acquisition identity. This decision closes the retry-versus-successor semantics; exact SDKs, provider mappings and mirrored owner acceptance remain S-U01/S-U07/S-U10 release gates. It neither enables amended-order association automatically nor allows terminal subscriptions or cancelled acquisitions to reopen.

**Successor-attempt fixtures:** preparation deadline expires while readiness is arriving; restart/profile edits do not extend the saved deadline; an applied result is recovered after a lost publication and late timeout; failed closure/recovery read keeps effects held; fenced timeout enters remediation without automatic cancellation/refund; old cleanup cannot affect a safely associated successor; lost create reply recovers the existing draft; late create loses to a no-create tombstone; unknown activation resolves to applied and prevents replacement; known unapplied activation permits fenced succession; two concurrent successor requests yield one association; crash before/after local rebind preserves or replays the correct association without a second claim; unknown capture blocks successor admission; late predecessor activation and cleanup cannot affect the successor; cancellation wins the admission fence; provider rebind response loss leaves successor pending; an explicit purchase start restriction is not extended by retry; and an original payment under a remedy cannot be silently reused.

Each case must expose the surviving acquisition, attempt, claim, payment and accepted-term lineage. These are required acceptance scenarios, not executed runtime evidence.

**Commercial order-version amendment (S-U01/S-U07/S-U10).** Changing accepted composition, price bindings or payer is a commercial amendment, not a technical retry or same-version successor attempt. [Orders' amendment contract](../../orders-lifecycle/docs/features/04-versioning.md#21-amend-a-committed-order) owns admissible states, authorized fields/actors, immutable version history and approval/acceptance handling. Its current PRD admits amendments in `submitted`, `pending_approval` and `approved`, not `in_fulfillment` or terminal states; this decision does not expand that window or authorize otherwise forbidden tenant-axis changes. Administrative corrections remain under Orders' existing non-commercial path.

1. Submit an explicit amendment against the expected current order version. Orders reruns the full commercial gate, captures the newly accepted bindings and evaluation evidence and obtains the required new-version approvals/acceptance; old approval or buyer acceptance is not silently inherited. A version commit alone is not proof of protected capacity, transferable payment or permission to activate. Preserve old versions, receipts and provenance rather than rewriting them with new content.
2. Before admitting capture or activation for the amended version, establish explicit predecessor/successor line mapping, scoped authorization and fresh applicability of occupancy policy, original-term commitment and financial evidence to that version. Retain, replace or move a claim only through the supported versioned mapping with exclusive ownership; an unchanged key must not double-count the same transferred right, while changed keys require destination admission.

   An old claim or Pricing commitment cannot be relabelled solely because the order ID is unchanged. New terms require the owner's new-version evidence, not silent repricing of the old paid promise. Unsupported mapping refuses/defers the affected protected-acquisition path before effects.
3. Billing/Payments resolves the old financial outcome and explicitly determines any applicable allocation, credit/refund obligation and separately authorized additional collection for the new version. No automatic reuse of the old capture, transfer of funds to a changed payer or debit of a computed order-total difference is authorized. Orders' indicative total is not a billing input.

   Preserve original payer/currency/capture lineage and recover unknown outcomes; a payment with an outstanding remedy cannot be silently counted as available funding. Each authorized financial effect retains its own business identity and correction lineage; a version increment alone earns no setup fee. Authorized postpay follows accepted new-version terms/credit authority without an invented immediate-payment requirement.
4. Supersession must fence old-version capture dispatch and activation at their effect owners as well as rejecting stale Orders callbacks. If old capture has started or its outcome is unknown, use the accepted capture/cancellation recovery protocol; a new version cannot erase the debt, paid promise or remedy obligation. A stale-version read or asynchronous event alone is not an execution fence. New-version capture/activation remains pending until the required owner evidence and association are established.

   This is coordinated durable work, not a transaction across Orders, Subscriptions, Pricing and Payments.
5. Failed preparation before Orders' version commit leaves the prior version and protected claims intact. After the new version commits, incomplete provider handover retains old protected obligations with explicit pending disposition, while the superseded version cannot resume fulfillment merely because its claim remains. Do not automatically roll back the order version or discard old protection on timeout.

   Release/replace the old claim only after acknowledged capture/activation fences, applicable financial resolution/remedy and the supported successor mapping or explicit cancellation conditions. No blanket requirement to wait for a posted refund is added. Exact provider sequencing, mappings and financial allocation policy remain owner contract work; unsupported cases stay gated.

Commercial reassessment and technical recovery are separate operations. This distinction does not enable amendments during fulfillment, reinterpret an existing active-subscription change as an order amendment, change the increases/additions-only initial quantity-change scope, or create a generic payer-funds transfer facility.

**Order-version fixtures:** unchanged-content replay of an amendment returns its original version/result; conflicting payload under the same identity refuses; failed new-version preparation preserves the old claim; old approval/acceptance and late capture/activation cannot authorize the new version; changed occupancy key without capacity does not discard the protected source; lost provider handover response remains pending and recoverable; unknown old capture prevents assumed funding; changed payer cannot inherit the old payer's payment automatically; amendment racing capture observes the capture-start boundary; and `in_fulfillment` amendment refuses under Orders' existing state guard. Required owner/provider fixtures are not claims of runtime support.

**Required fixtures:** V01–V04/V06 cover lost confirmation/capture/create replies, capture racing release, restart after local commit, an unmet explicit accepted start condition after payment, refused new-attempt rebind, failed claim move, and two lines with independent starts/partial remediation. Include standalone-create refusal before effects and lost-reply replay for the authorized Orders origin. For each crash point, record the surviving owner references, expected receipt and which effect is still pending.

Delivery, SDK encoding and owner acknowledgements must be ratified and exercised before this profile is enabled.

## 8. Two-line order and partial fulfillment

For a prepaid order, both line claims and their future-term commitments must already have been secured before capture; a line failure does not erase that paid obligation. Assume independent lines A and B have both reached `draft_created`, and Workflow's common date barrier has passed. Each line receives its own activation operation bound to its accepted purchase. They can apply at distinct instants **T_A** and **T_B**.

Waiting until the latest requested date prevents early dispatch; it does not create a shared database commit or guarantee equal starts.

If A applies and B fails before application, Workflow's documented default is **remediate**: preserve A's applied outcome, halt affected pending work, allow independent in-flight work to settle, create the required manual task and hold the order. The order is not `completed` while B is unactivated. A is not automatically cancelled solely because B failed.

If remediation succeeds within the already accepted protocol, B gets its actual successful T_B and the order can complete once every line is applied. Recovery cannot change A's T_A to T_B for cosmetic order-level alignment. If B cannot meet an explicit accepted start condition, preserve its obligation and follow agreed delay/remediation or cancellation under S-U01. Technical evidence expiry does not require a new purchase or financial decision.

If fail-fast was configured, or remediation is exhausted and Workflow decides failure, it reconciles every uncertain line and compensates the real outcomes: void unapplied drafts while separately resolving their protected order claims and payment obligations, cancel applied subscriptions through Subscriptions, and account for late capture/activation successes. A paid unfulfilled line requires explicit agreed cancellation with recorded financial remedy before its reservation is released; this does not prescribe refund amount or timing.

Commercial history and consumed usage remain intact. The financial correction policy remains with Rating/Billing and the existing one-time/retention gates. No ETF or unused-portion credit is derived merely from `order_compensation`; required reversal/correction evidence is a separate outcome.

Order completion still requires all lines. Each line keeps its actual service and financial evidence. A requirement for all-or-none simultaneous commercial service, or for free service during remediation, would be a new product/protocol decision rather than an interpretation of the existing two-wave saga.

## 9. Owner changes and consequences

| Owner | Required responsibility |
| --- | --- |
| Pricing / Products | Validate and durably accept original purchase terms and supported future coverage; preserve immutable reads and order initial acceptance against catalog changes. Ordinary later drift does not revoke an accepted sale. |
| Orders Lifecycle / Workflow | Preserve accepted acquisition evidence, protect capture, maintain order/attempt and cancellation fences, obtain committed spawn signal, schedule lower bounds and consume actual applied per-line T. |
| Subscriptions | Own local application, occupancy conversion, current guards, held-readiness resolution and durable result/recovery. |
| Entitlements | Prepare and retain required assignments; publish commitment-bound readiness; recover activation resolution and perform authorized cleanup. |
| Financial owner / Payments / Billing | Establish and retain acquisition-bound payment allocation or future postpay commitment, with explicit versioned dispositions and durable financial outcomes; deduplicate capture and execute agreed remedy/accounting. |

Exact SDK/provider mapping and conformance evidence remain required. Native provider availability is not inferred from this document.

## 10. Joint acceptance scenarios

These are design acceptance scenarios, not executed tests.

| Scenario | Required assertion |
| --- | --- |
| Purchase accepted for a future date; quote later expires or price/SKU closes for new sales | Original accepted terms remain executable; no fresh catalog verdict or successor repricing. |
| Future terms or capacity cannot be durably accepted/protected | Refuse/defer capture; copied preview or partial evidence is insufficient. |
| Initial acceptance races catalog closure/retirement | Owner ordering gives one durable result; uncertainty is recovered before capture. |
| Readiness completes after the requested start | Sample actual T after readiness and current guards. Honor only explicit accepted latest-start/delay terms; do not backdate to the request. |
| Complete accepted snapshot exists while Pricing is temporarily unavailable | No activation-time Pricing veto/read is required; other current guards still apply. If necessary historical evidence is missing, recover it without re-admission or reprice. |
| Explicit contractual latest-start bound cannot be met | Preserve the purchase obligation and invoke agreed delay/remediation handling; no hidden technical expiry or retroactive T. |
| Policy or financial admission denied/unknown before preparation | No unauthorized usable preparation. Recovery and protected financial obligations remain distinct from success. |
| Capture and cancellation race with lost replies | The capture-start fence and Payments identity select/recover the result; no duplicate debit or release of uncertain rights. |
| Activation and withdrawal race | One local operation/generation decision wins. Applied retains T; non-activation is irreversible for that generation before cleanup. |
| Active commits but reply/posture event is lost | Recover identical result/T; retain preparation and never provision or charge twice. |
| Restart during unknown commit | Settle/recover the old outcome before another decision; a confirmed retry samples a new T. |
| Emergency security block or physical fault races readiness/commit | Apply SUB-D-57 incident handling; no fictitious atomic uptime or erasure of applied history. |
| Stale order/attempt/terms or changed replay payload | Refuse or return the retained exact outcome; no copied acceptance creates a second subscription. |
| Two lines: A applied, B failed | Preserve A and B's actual evidence and paid obligations; default remediation, with explicit compensation if selected. No early all-lines completion. |
| Authorized pre-T or never-activated preparation usage arrives late | Platform-borne under the enabled profile; unknown boundary stays unresolved, post-T usage follows accepted terms. |
| Provider hard ceiling blocks a required purchased capability | No initial ready result from a partial allowance. Preserve accepted desired rights and authoritative restriction evidence for remediation; known offer incompatibility prevents sale/capture. Ordinary changes after held readiness follow the activation/withdrawal fence; emergency security remains immediate. |
| Quota/credit prerequisite depends on post-T issuance | Refuse the unsupported initial profile before sale/capture; no early active shortcut. |

## 11. Confirmation and remaining decisions

The commercial direction is fixed: Pricing validates and freezes the purchase; Subscriptions executes that accepted acquisition with current order, financial, security, occupancy and readiness guards and records actual T. Work remaining for the owners is:

1. Versioned accepted-purchase evidence, durable future coverage and immutable historical reads, with initial Pricing/Products acceptance ordered against catalog closure/retirement.
2. Acquisition/attempt association and local application/withdrawal fences, including protected capture, settlement and recovery through unknown outcomes.
3. Mapping durable acquisition financial coverage and explicit disposition fences to local application/recovery; clock bounds apply to actual accepted time conditions, and no technical proof or quote deadline may shorten purchased execution rights.
4. Held-readiness commitment/resolution, provider retention and emergency mapping, bounded preparation exposure and platform-cost evidence under SUB-D-57.
5. Per-line order outcomes, default remediation and explicitly selected compensation/financial-remedy behavior.
6. SDKs, provider implementations and joint acceptance fixtures. Documentation agreement does not prove runtime readiness.

Unsupported future coverage prevents acceptance/capture of that profile. Accepted paid obligations survive ordinary technical TTL and unknown outcomes; commercial cancellation or explicit agreed start constraints retain their own handling. Deferred capabilities remain excluded.

## 12. Source and traceability index

| Source | Relevance |
| --- | --- |
| [Cross-owner decisions](./DECISIONS.md#integration-delivery-gates) | S-Q01 direction accepted; versioned contract and runtime evidence open. Other decision boundaries remain independent. |
| [Subscriptions PRD](./PRD.md) | Activation, durable operation, commercial entitlement and financial-evidence outcomes. |
| [Subscriptions DESIGN](./DESIGN.md#332-pricing-acceptance-and-accepted-time) | Current Pricing provider and accepted-purchase execution contract; versioned integration remains gated. |
| [Foundation feature](./features/01-foundation.md) | Atomic applied transition, business-attempt identity, withdrawal, recovery and external effects. |
| [Entitlement feature](./features/05-entitlements.md) | Asynchronous Entitlements materialization and provider-side source-revision fence. |
| [Upstream requirements](./UPSTREAM_REQS.md) | S-U01 operation/attempt and draft-change gates; owner acceptance remains required. |
| [Orders Lifecycle upstream §2.1](../../orders-lifecycle/docs/UPSTREAM_REQS.md#21-subscriptions) | Explicit start authority, no quoted-date backdating, preserved draft/activate pair. |
| [Orders Workflow PRD §6.3](../../orders-workflow/docs/PRD.md#63-fulfillment-orchestration) | Two waves, common date barrier, per-line progress, default remediation and all-lines completion. |
| [Pricing acceptance SDK](../../pricing/pricing-sdk/src/acceptance.rs) | Current receipt/hold types and observation-only fulfillment eligibility contract. |
| [Pricing fulfillment implementation](../../pricing/pricing/src/infra/commercial_terms/fulfilment.rs) | Both `same_activation` calls, exact replay ordering, fresh expiry/original-price/SKU checks. |
| [Pricing DESIGN](../../pricing/docs/DESIGN.md) | Current D-510 provider surface and accepted-binding ownership. |

## 13. Orders and Pricing reciprocal activation contract

This section defines the shared semantic contract for Orders-origin subscription acquisition. The [component proposals](proposals/README.md) record the required Orders Lifecycle/Workflow and Pricing changes. Provider adoption, compatible SDK mapping and runtime conformance remain delivery gates. Existing V1 methods retain their versioned semantics. Section 7 remains the detailed capture/cancellation and attempt-recovery protocol; the normalized evidence below is required content, not final SDK names.

**Verified baseline and required deltas**

| Seam | Repository evidence | Required target and owner action |
| --- | --- | --- |
| AP01 commercial time | Current V1 hold fixes an exact activation time; the accepted future-purchase profile needs a separate mapping | Workflow supplies the requested lower bound/common barrier; Subscriptions records actual T against the accepted acquisition and current guards. Lifecycle/Workflow consume the durable applied result. |
| AP02 occupancy | The local Lifecycle baseline SUB-O5 and gate text use active-only counts; its proposed release on leaving active would lose a suspended right | Subscriptions supplies separate active, retained-suspended and confirmed-reserved counts, policy key/limit/version and own-claim evidence. Suspend retains; own-reservation activation/resume converts once; applied cancellation releases. Orders changes its read/predicate and deduplicates local versus provider claims |
| AP03 protected future terms | [Acceptance issuance](../../pricing/pricing/src/infra/commercial_terms/check.rs) derives hold_until from acceptance server time; V1 rejects a requested start outside it. Products reference reservations protect references but are not demonstrated as customer-specific future-sale commitments | Pricing must expose a separate durable commitment for accepted future terms/date/window, ordered against Products retirement and price closure. A V1 receipt, hold or copied terms cannot stand in for this new capability. Orders confirmation records both AP02/AP03 evidence before capture; missing either prevents capture |
| AP04 accepted purchase execution | Current check_fulfilment is a fresh eligibility observation; it cannot preserve a future sale against ordinary later catalog changes | Publish/read immutable accepted acquisition and any explicit execution restrictions. Subscriptions applies that purchase under its own order/attempt/readiness/financial fence; no new Pricing decision is required at activation. |
| AP05 capture ordering | Orders begin-fulfillment uses payment authorization and tolerate-failure; neither establishes protected capture | Lifecycle owns the durable capture-start/cancel ordering for the exact acquisition/version and financial scope. Workflow delivers the resulting outbox command; Payments executes/reconciles the same capture identity. Authorization is not capture. Tolerate-failure never bypasses AP02/AP03 or prescribed activation payment |
| AP06 dispatch ordering | Lifecycle requires committed report-spawn-signal; dispatch must depend on its durable acknowledgement | Workflow obtains/replays the committed signal before activation dispatch. A missing/ambiguous acknowledgement permits recovery only. Signal closes the documented direct-order-cancel boundary; it is not payment protection, actual activation or a fence against all later mediated hold/cancel actions |
| AP07 terminal/recovery boundaries | Draft void, expired hold, order TTL or fulfillment_failed can be confused with financial settlement | Keep protected claims/commitments until authoritative payment/application recovery and coordinated release fences/remedy. Operational order outcome and financial completion are separate. No automatic refund amount/timing or wait-for-posted-refund requirement. A technical successor attempt preserves the original acquisition, not a new sale |

**Normalized evidence and outcomes**

| Record / request | Required information and invariant | Durable authority / recovery |
| --- | --- | --- |
| Acquisition association | Seller/catalog scope; order ID, immutable version and line; accepted terms/digest and tenant axes; execution attempt, intended subscription and association generation when known. Acquisition identity is independent of a transport key or attempt | Lifecycle owns the order/version; Subscriptions owns exclusive local create/application association. Same business/payload recovers; conflict refuses. A successor requires section 7 settlement/fencing; it never creates a new purchase implicitly |
| Capacity protection | Stable claim ID/revision, acquisition/line, authoritative payer/resource/scope key, evaluated policy/version, held/consumed/released disposition and exclusive association | Subscriptions owns reserve/read/attach/consume/release outcomes. Unconfirmed draft is not a claim. Local confirmation/consumption must serialize on the authoritative key; TTL is not proof of safe release |
| Future-term commitment | Stable commitment ID/revision, acceptance and original complete bindings/policies/digests, acquisition/tenant scope, accepted future date/window, protected Products/price decision evidence and recoverable cancellation/revocation disposition | Pricing owns commercial commitment and coordinates Products protection. It reports committed only after all required producer fences are established. Partial/unknown preparation is recoverable and cannot authorize capture. Original acceptance must qualify at issuance; no retroactive resurrection of expired V1 authority |
| Capture-start evidence | Stable capture business identity and financial amount/obligation scope; exact applicable claim/commitment references for every funded line; authorized order/version; decision/generation and dispatch identity | Lifecycle persists winner and outbox atomically against cancellation. A single order payment binds the complete covered line set, not one convenient receipt. Payments must fence/deduplicate execution; Workflow cannot infer success from an authorization or retry with a new capture identity |
| Accepted execution evidence | Exact acquisition, original terms/digest, supported future coverage and explicit start restrictions; designated local order/attempt association and current cancellation/financial evidence | Pricing preserves accepted evidence; Subscriptions owns one local application decision and actual T. An offer deadline does not become an execution deadline after acceptance. |
| Applied activation | Original acquisition/attempt/line and subscription, operation/revision, actual T, consumed claim, accepted terms digest and held-readiness resolution | Subscriptions is authoritative. Workflow consumes the durable applied outcome; Lifecycle completes only from the exact current line roster after all required outcomes. |
| Release/non-application evidence | Original owner references, definitive applied/unapplied result, cancellation/association fences, capture disposition and required recorded financial remedy, plus per-owner release acknowledgements | Coordinated recovery uses the original identities. A missing read is not a tombstone. Unknown capture/application retains protection; an applied winner is cancelled through ordinary guarded lifecycle rather than erased. No shared cross-gear database transaction is assumed |

Every owner distinguishes applied/replayed, pending/unknown, refused, stale/conflicting and unavailable under authorized scope. Historical replay preserves the accepted acquisition and its outcomes. Changed terms require a supported new commercial acceptance; technical retry or catalog change does not create one. Any replacement association follows section 7 with predecessor fencing and original paid obligations preserved.

**Occupancy read and caller predicate**

The target batched read binds the exact payer/resource/scope key and policy version to separate active, retained-suspended and confirmed-reserved counts, an explicit limit, observation/recovery provenance and authorized evidence identifying which caller claims are already represented. Active count keeps its literal meaning. A plain draft_count is diagnostic only and cannot substitute for reserved count.

For a new unreserved purchase, assess existing protected occupancy plus this request's additional unrepresented rights and relevant Orders in-flight claims not already represented in the provider result. Count each business right once; unresolved identity/dedup evidence blocks new admission. Preserve Orders' own in-flight uniqueness rule. Never re-bucket a provider's key locally or infer limit one. An old active-only adapter is unsupported for this profile, not a partial successful answer.

A preflight pass is not reservation. Confirmation must acquire each required durable claim before capture. Later activation of its own valid reservation and unchanged-key resume are conversions of protected rights, not new competitions for capacity; a later reduced limit cannot revoke them. Workflow validates own-claim evidence instead of treating that reservation as a collision with itself. Invalid/missing/unknown protection requires recovery/remediation and cannot release already-paid rights. New conflicting admissions refuse.

Failed current key changes preserve the original claim; linked replacement and tenant transfer remain deferred.

**Pricing/Products protection and validity**

A future date beyond V1 `hold_until` also needs an authorized, versioned initial accepted-terms receipt under this extension: current V1 `check` refuses that date before any hold is possible. The extension validates and freezes the exact future purchase terms and binds that receipt to the commitment before capture. It must not require an impossible V1 acceptance as its only input, revive expired V1 authority, or treat copied assessment data as customer acceptance.

Exact schema/issuance mapping remains a provider delivery item.

Orders records the accepted acquisition/profile and original terms/commitment references. Offer/assessment validity applies before purchase acceptance; an explicit purchased execution bound has separate provenance. A future start is not rejected because a previous assessment expired. Capture scope comes from the financial owner, not indicative order totals.

Pricing preserves the accepted purchase and its original terms under AP03/AP04. Initial acceptance is ordered against relevant Products/Pricing changes using durable owner evidence; ordinary later price closure or retirement for new sales cannot revoke that purchase. Explicit order cancellation, security denial or financial disposition retain their own authorities and recovery. No cross-gear table access or shared database transaction is assumed.

The existing Products reference registry may be used only if its owner contract proves the required customer-commitment semantics; the current price_book_entry/plan_item reference kind alone is not such proof. Exact provider mapping and conformance remain a named Pricing/Products dependency. No unimplemented Products command is presented as available. If that mapping cannot provide the guarantee, the protected future-order profile stays disabled before capture.

Accepted-purchase execution follows section 5 and the local order/attempt/cancellation fence. Catalog drift is distinct from security or explicit commercial cancellation. Emergency security blocking follows SUB-D-57; commercial cancellation must recover any already applied winner. Missing supported purchase evidence or retained financial coverage or current security authority blocks the affected operation without rewriting its terms.

**Owner implementation checklist**

| Owner | Concrete change | Current delivery status |
| --- | --- | --- |
| Orders Lifecycle | Store/read versioned per-line protection references and capture decision/outbox; order capture-start against cancellation; expose/replay committed spawn signal; consume revised occupancy evidence | Required changes recorded in the Lifecycle proposal; owner PRD/design adoption, SDK/storage/provider integration and fixtures remain open |
| Orders Workflow | Deliver protected capture using the same identity; persist line evidence; retain two-wave/date barrier; obtain committed spawn signal; send requested lower bound and consume actual applied T; recover uncertain create/capture/activation/release | Required changes are specified in the Workflow proposal; owner adoption and implementation proof remain open |
| Pricing | Versioned purchase acceptance/commitment and immutable read/recovery; original terms and explicit future coverage; coordinated initial Products acceptance | Owner adoption and SDK/runtime evidence remain open; existing V1 quote/hold behavior is not reinterpreted. |
| Subscriptions | Durable claim and attempt association; current-scope occupancy read; final local consume and applied receipt; cancellation/release recovery under section 7 | Semantic contract specified; local SDK/storage/integration and runtime conformance not delivered by documentation |
| Products / Payments / Billing | Map retirement protection, fenced capture outcome and financial remedy respectively to the evidence above | External mappings remain explicit dependencies; this package does not design a new catalog, payment engine or accounting policy |

**Joint conformance cases: AP-C01 to AP-C12**

| Case | Trigger / fault | Required observable result |
| --- | --- | --- |
| AP-C01 | Requested future start beyond V1 hold_until; no new commitment provider | Refuse/defer protected capture, never extend V1, silently reprice or take money on a preview |
| AP-C02 | Capacity held but commercial commitment fails, or vice versa | No capture; recover/release only settled unused preparation under original identities. One successful owner step is not confirmed protection |
| AP-C03 | Two acquisitions race for the last place; suspend/resume and limit decrease occur | At most one new claim wins; suspend retains; own-reservation activation/resume converts once even after limit decrease; new excess admission refuses |
| AP-C04 | Products retirement or explicit price closure races initial commitment | Durable owner ordering determines one valid result; a partial/unknown reservation cannot authorize capture. After commitment, ordinary drift cannot defeat original paid terms |
| AP-C05 | Readiness is delayed past requested date and races the saved preparation deadline | Before deadline, actual T follows the guarded decision and explicit purchased start conditions. After deadline this attempt cannot newly activate, even with a late ready timestamp or delayed timer. Fenced closure preserves the purchase for remediation; retries/restart/profile edits cannot extend the deadline. |
| AP-C06 | Accepted purchase survives quote expiry; conflicting replay or changed attempt | Preserve accepted terms and original identity; conflicts refuse and attempt succession requires settled predecessor fences. Quote expiry does not re-admit or reprice the purchase. |
| AP-C07 | Capture start and cancellation race, including lost Payments response | Cancel wins before capture-start: no late debit. Capture-start wins: cancellation/remedy remains possible; unknown retains protection, no duplicate capture |
| AP-C08 | report-spawn-signal response is lost | Recover committed signal under the same identity before dispatch; no assumption from the first intent or missing read. Later mediated hold/cancel keeps its independent guards |
| AP-C09 | Activation commits but result publication is lost | Recover original applied receipt/T, acquisition/claim and readiness resolution, even if a later timeout worker requests closure. Applied wins are not rewritten; no duplicate subscription, charge or claim and no timeout cleanup. |
| AP-C10 | Two-line order: A applies, B fails an actual execution prerequisite | Preserve A's real T and B's paid obligation; remediate or record explicit compensation/remedy. Order/draft terminal state alone releases no protected money/right. |
| AP-C11 | Authorization failed but tolerate-failure is enabled; protected capture or prescribed activation payment unavailable | Tolerance cannot waive either pre-capture guarantee or required financial admission. Authorized postpay adds no immediate card/capture requirement |
| AP-C12 | TTL/draft rebuild/forced unreconciled terminal state with unknown capture or application; stale release arrives | Preserve original protection and recover owner outcomes. A preparation-timeout closure preserves purchase, payment and claim for remediation; it is not commercial cancellation/refund. Only fenced coordinated release changes purchase protections, and a successor additionally requires safe predecessor-effect/cleanup fencing; an old attempt cannot free its successor's association |

These cases refine V01-V04/V18 and preserve S-U01/S-U07/S-U10. They are specified acceptance scenarios, not executed tests. Release requires exact producer/consumer version mapping, the selected Products and financial implementations, and recorded execution evidence. AP01-AP07 specify the Subscriptions side. Owner adoption, implementation, external ratification and conformance results remain required deliverables.
