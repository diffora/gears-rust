---
description: "Source identity, atomic fenced application, authorized recovery, access posture, windows, runtime and evidence that Quota Enforcement must provide before Entitlements can project purchased quotas into it."
---

# Quota Enforcement proposal from Entitlements

**Status:** proposed owner handoff from Entitlements. This is a request to the Quota Enforcement (QE) owner, not an accepted amendment to the QE design, a delivered SDK or an external-owner approval. Examples are illustrative, not commercial defaults. The [proposal index](README.md) provides the surrounding scope and source precedence; the consuming feature is [quota projection](../features/03-quota-projection.md).

<!-- toc -->

- [1. Baseline](#1-baseline)
- [2. Required changes](#2-required-changes)
  - [2.1 Application contract](#21-application-contract)
  - [2.2 Authorization](#22-authorization)
  - [2.3 Local commit and durable recovery](#23-local-commit-and-durable-recovery)
- [3. Examples](#3-examples)
- [4. Acceptance](#4-acceptance)
- [5. Open points](#5-open-points)

<!-- /toc -->

## 1. Baseline

| Inspected source | What it establishes |
| --- | --- |
| [QE SDK models](../../../../system/quota-enforcement/quota-enforcement-sdk/src/models.rs), [client](../../../../system/quota-enforcement/quota-enforcement-sdk/src/client.rs), [manager service](../../../../system/quota-enforcement/quota-enforcement/src/domain/quotas/service.rs) | Repository quota manager create/read/update/deactivate interfaces and validations exist; their presence does not establish a production-capable provider. |
| [QE in-process API](../../../../system/quota-enforcement/quota-enforcement/src/api/in_process.rs), [storage quotas](../../../../system/quota-enforcement/plugins/quota-enforcement-storage-plugin/src/domain/quotas.rs), [storage gear](../../../../system/quota-enforcement/plugins/quota-enforcement-storage-plugin/src/gear.rs) | ClientHub/storage wiring is present as source work, but the scoped storage client is unpublished pending primitives, and an unbound outbox makes mutations roll back until its dispatcher is bound. Full consumption reserve/commit runtime is not established; deactivation's lease-cascade slot awaits the lease feature. Do not enable BSS on the strength of CRUD signatures. |
| [Entitlements DESIGN §3.3.4](../DESIGN.md#334-quota-apply-and-read) | The apply and read contract Entitlements consumes; this proposal asks QE to provide it. |

The [quota store](../../../../system/quota-enforcement/plugins/quota-enforcement-storage-plugin/src/infra/storage/quota_store.rs) adds a concrete limitation: `consumed_under_lock` reads the allocation in-flight counter but returns zero for non-allocation until consumption operations add period counters. A patch below that allocation value fails with `CapBelowConsumed`; lowering cap to zero cannot implement suspension of occupied capacity. The separate restriction behavior proposed here requires provider support. Internal row locking and record-version CAS protect storage updates; without a caller source-revision precondition they do not fence delayed Entitlements commands. Do not claim the current cap guard protects consumed usage.

The SDK's deactivation documentation promises atomic lease resolution, while the storage implementation leaves that cascade unimplemented. Treat the SDK statement as an intended contract, not evidence that the implementation fulfills it. The manager also accepts a quota for a `Direct` metric but records that it is inert. Successful materialization alone cannot satisfy a purchased hard-enforcement requirement.

Repository source does not establish that a provider is deployed or that the selected profile is enabled. Proposed model names below are conceptual owner contracts, not existing Rust types. Entitlements owns the rule semantics and the derived desired state (ENT-D-03); the selected adapter translates it and QE owns the final atomic effect (ENT-D-01).

## 2. Required changes

| Provider request | Minimal change / release evidence |
| --- | --- |
| QE-P01 Source identity and atomic application | Durable authenticated source/business key `(namespace, resource tenant, assignmentId)`, uniqueness/ownership constraints, revision/digest compare-and-apply, terminal tombstones and recoverable results. Existing create has no idempotency key; update has no expected version; record version on reads is not CAS. |
| QE-P02 Authorized recovery | Read current source state by the stable key and recover applied outcomes. Existing filters cover IDs/tenant/subject/metric/status, not an Entitlements business key. An adapter's UUID mapping cannot recover an unkeyed create whose response was lost. |
| QE-P03 Access posture and precedence | Demonstrate hard enforcement, suspension/revocation including fallback, preservation of counters and isolation from operator/other-source records. Keep purchased source state distinct from effective provider restrictions and report authoritative scope/ceiling/reason/policy version with ordered recovery. Lower restriction is not full fulfillment; ordinary held-readiness changes are fenced and emergency security remains immediate. Deactivation is not a universal deny. |
| QE-P04 Windows and validity | Explicit exclusive-end support/equivalence, immutable continuing counter identities, supported cap-below-used behavior and no reset on replay/update/renewal. Reject unsupported windows and differing-expiry combinations. |
| QE-P05 Complete runtime | Demonstrate end-to-end apply, enforcement and recovery for the selected profile, including allocation or separately enabled consumption reserve/commit/release behavior and deactivation lease handling. Public CRUD signatures or unit tests alone do not establish this capability. |
| QE-P06 Versioned commercial evidence | Pricing publication, accepted snapshot/receipt and continuation extensions ([pricing rule proposal](pricing-rule-proposal.md)); Products metric/unit/subject capability evidence ([products metric proposal](products-metric-proposal.md)); exact delta quantity mapping; Entitlements desired-state/recovery schema; negotiated adapter versions. |
| QE-P07 Allocation preparation | Support exact operation/generation-bound preparation authority for allowed resources/actions under the frozen purchased cap; count occupied/reserved capacity before T and continue the same source/member/allocation across activation without reset, duplicate allowance or access gap. Retain through unknown outcomes; fenced abort blocks new allocation and releases counted occupancy only after confirmed resource release/QE settlement. The deferred consumption profile uses marked readiness-check requests with provider-owned execution and quota treatment. |
| QE-P08 Recognizable check purpose | Accept/map readiness-check purpose in scoped adapter request context so the provider can recognize it. Execution method and quota/accounting handling remain provider-owned; the marker alone implies no dry-run, exemption, no-effect guarantee or success. Expose authoritative correlated readiness/effect evidence and preserve counter/window. Deferred consumption runtime gates remain. |

These changes belong in the existing provider/adapter owner projects. No direct database writes are allowed. Metadata may carry approved provenance, but cannot substitute for atomic provider state: the inspected metadata contract is validated, replaces the whole object, and includes server-supplied constraint-contract semantics. Do not smuggle a fencing protocol into arbitrary JSON or overwrite another owner's metadata. An alternative provider may satisfy gate E-U01 without native QE, but it must demonstrate the same selected-profile guarantees.

### 2.1 Application contract

The existing CRUD API is not enough for this protocol. Propose a versioned provider operation equivalent to `apply_source_state(source_key, source_revision, digest, complete_target, effective_boundary, authority)` plus an authorized `read_source_state(source_key)` and durable operation recovery. These are requirements for owner adoption, not names of shipped methods.

The provider must retain an authoritative revision, digest, terminal posture and recoverable result for each source, including revoke-before-create. Its reads must identify source-owned members and their provider record IDs. The provider owning the final effect enforces uniqueness and revision fencing; an adapter-local cache is not sufficient.

Expose these guarantees through a versioned managed contract. Existing `QuotaSpec` rejects unknown fields; arbitrary create metadata cannot implement the protocol. Physical tables, indexes and SDK types belong to the provider design. BSS never writes provider storage directly.

Use an aggregate source/business key `(authenticated namespace, resource tenant, assignment identity)` for the complete-set fence. A stable member key beneath it adds the logical quota binding and subject; metric, kind, unit and period/window identity are immutable attributes of that member. The separate resolved-tuple exclusivity constraint prevents competing source keys from claiming the same purchased quota scope. These are distinct identities, not interchangeable source-head domains. An aggregate revision is ordering; an operation ID is retry identity; a digest is content identity. None substitutes for the others.

The key must survive worker restarts, transport-key changes, renewal and cap updates. A complete-set revision covers every member that must change together; omissions do not implicitly delete unrelated source records. Terminal cancellation tombstones the aggregate before any further member creation can apply, including members the provider has never seen. A new rule/member ID cannot evade the revoked source fence.

At the enforcement effect boundary, atomically:

1. Authorize the caller's source namespace and target scope; prove exclusive ownership or refuse conflict. Bind accepted authority to the exact target and content.
2. Compare the incoming revision/digest with the durable applied source head. Lower revision is stale; equal revision/equal digest returns the existing result; equal revision/different digest conflicts; higher revision may replace only that source's state after validation. A terminal revoke also closes the acquisition/source identity: a later `Granted` request is refused even with a higher revision. Only an explicitly authorized new acquisition with a distinct identity can establish new rights; numeric monotonicity alone is not terminality.
3. Apply the full supported member set/restriction/tombstones and persist its new head, digest and recoverable result as one atomic effect. Counter preservation is part of that effect. Record definitive refusal separately from an unknown result.
4. Expose an authoritative readback of the source head and effective state. Do not acknowledge a queued command as already enforced.

If the provider cannot atomically apply a multi-member change, v1 must restrict the offered profile to a single member (or provide another proved all-or-safe-restricted protocol). Do not partially grant an atomic commercial bundle and report success. A restriction may be applied first for safety, but its durable pending completion and recovery must be explicit.

The fence must cover in-flight work: revision 7 can pass an adapter's precheck, wait remotely, and arrive after revision 8 revoked access. Filtering its late callback cannot undo the resulting grant. The provider's conditional application must reject revision 7 even after a timeout, worker restart or lease loss. A serialized adapter is acceptable only if its execution protocol proves that an earlier operation can no longer apply before a later one is finalized; dispatch serialization alone cannot prove that with the current CRUD boundary.

### 2.2 Authorization

The [admission implementation](../../../../system/quota-enforcement/quota-enforcement/src/domain/admission.rs) and [PEP](../../../../system/quota-enforcement/quota-enforcement/src/domain/pep.rs) authorize the actor/action/resource with supported tenant/resource constraints. Manager create admits `quota_enforcement.quota/create` using `owner_tenant_id`; it does not pass metric, cap, purchased rule or subject properties as purchase proof. The existence of a generic admission-with-properties helper does not make this manager path purchase-aware.

Update and deactivate have a different target construction: the current manager sends the quota ID and the caller's home tenant to admission, then uses the returned scope to read/mutate the row. It does not first substitute the stored quota's owner tenant. A service principal projecting into customer resource tenants therefore needs an explicitly tested delegation/scope contract for create, read, update and deactivate; permission to create in a target tenant alone does not prove later lifecycle calls are authorized.

Therefore an actor allowed to create a quota is not thereby entitled to choose any purchased cap. The adapter must be a trusted, authorized projector of committed accepted state, with source ownership enforced by the provider. Its credentials must not authorize uncontrolled customer writes. Bind commands to the resource tenant and prove subject membership through the resource authority. A tenant-scope QE subject must match that tenant; a nonempty project/resource identifier is not proof that the resource belongs to it. Seller/payer permissions cannot substitute for resource-tenant authority. Enforce authorization on recovery reads as well as writes.

`source = Licensing | Operator` is provenance, not a security boundary. It must not let a customer, another source or an operator-shaped request impersonate Entitlements ownership. Existing operator quotas and other sources' records are never adopted/deactivated by matching just subject and metric. Changes to source-owned records require explicit ownership and version fencing; maintenance overrides must have a declared conflict/precedence policy.

For the initial hard-limit profile, the adapter maps `fail_open_hint = false` and `notification_thresholds = []`, including for cap zero. QE's hint is informational: the consuming service must demonstrate that unavailable or unknown enforcement does not grant access under this profile. Thresholds, if enabled later, need explicit accepted semantics and must satisfy QE's bounded-cap validation. Metadata must satisfy the metric owner's constraint contract; an empty object is valid only when that contract permits it. These fields cannot carry an implicit purchase or authorization override.

### 2.3 Local commit and durable recovery

Entitlements persists desired state and delivery intent before dispatch under its [posture intake](../features/01-posture-intake.md) and [quota projection](../features/03-quota-projection.md) flows. Provider recovery must return the same source revision/digest and effect identity after a lost reply. Retries cannot change quantity or create another quota.

Reconciliation compares complete desired and applied state and repairs only records owned by this source. A stale success cannot mark a newer desired head complete.

| Recovery situation | Required action |
| --- | --- |
| Remote apply succeeds; local response/write times out | Mark unknown, read the same source key/operation, verify revision/digest and recover the receipt; replay only the same idempotent apply. Do not issue another unkeyed create. |
| Local commit succeeds; worker dies before dispatch | Recover the outbox and send the identical desired revision. |
| Delayed old request arrives after revoke | Provider rejects it against the retained tombstone/source head. Local callback filtering is an additional check, not the primary fence. |
| Provider reports a higher revision | Reconcile to known authorized history; mark an unexplained revision as conflict. Do not overwrite it with the local lower head. |
| Record missing / drift / operator edit | Use authoritative source ownership/readback. Repair only through authorized conditional operations; report conflict where another owner or unknown state exists. Never create a parallel quota or overwrite an operator record blindly. |
| Metric registration or enforcement mode drifts | Current update revalidates the metric and may return `MetricNotRegistered`; reads can retain the row with missing metric-kind evidence. Report a blocked update/unknown enforcement finding, retain the commercial obligation, and obtain owner remediation. A Direct/inert record is materialized but not enforced; never mark the purchased hard-limit profile ready or substitute another metric silently. |
| Adapter/profile changes | Keep in-flight work bound to the original adapter/profile. Require a separately supported transition and settlement; a new adapter must not replay old commands into a different provider. |

Keep tombstones and monotonic heads for at least the entire accepted delayed-message/retry horizon, with an explicit retention/namespace-reuse policy. If stale senders cannot be conclusively retired, retain the fence. Counter retention, tombstone retention and commercial history retention have distinct purposes; garbage collection cannot reopen a revoked source.

## 3. Examples

The values below are illustrative; identifiers and numbers are not catalog defaults or SDK literals.

**Fenced apply and readback.** Entitlements derives revision 7 for source key `(BSS entitlements, tenant-R, assignment-42)` with digest D7 and one member `(storage-cap binding, project-42)` mapped to QE tenant `tenant-R`, subject `(project, project-42)`, metric `storage.allocated_bytes`, allocation, no period, hard mode, bounded cap `322122547200`. The adapter calls the managed apply with that key, revision, digest and the complete member set.

QE admits ownership of the resolved tuple, finds no source head, creates the head and the member atomically and returns the applied revision 7, digest D7 and the quota record id. A replay of the same call returns the same result without a second record. A later revision 8 with cap `536870912000` replaces the member under the same head and leaves the allocation counter untouched.

**Stale in-flight request.** Revision 7 passes the adapter's precheck and waits in a remote queue. Meanwhile revision 9 arrives as a terminal `Revoked` and tombstones the head. When revision 7 reaches the effect boundary, QE compares it with the applied head (revision 9, terminal) and refuses it as stale; the refusal is recorded and recoverable. A later `Granted` request with revision 10 for the same source key is refused as well, because the tombstone closed the acquisition. Only an explicitly authorized new acquisition with a distinct assignment identity can create new rights.

**Lost reply.** The apply for revision 8 succeeds in QE but the response is lost and the worker restarts. Entitlements marks the operation unknown, calls `read_source_state` for the same key, finds applied revision 8 with digest D8 and the record ids, and records the receipt. No second create is issued. Had the head shown revision 8 with another digest, the result would be a conflict finding, not an overwrite.

## 4. Acceptance

These are proposed conformance cases, not executed test results. The identities below are local to this proposal and do not renumber scenarios of other owners.

| Case | Stimulus | Required observable result |
| --- | --- | --- |
| QE-C01 Accepted evidence | Publish a new plan/metric revision after purchase | Replay retains the original rule/unit/quantity/digest; no live-catalog fallback. Missing history is pending/refused, not unlimited. |
| QE-C02 Numeric limits | Fractional integer result, overflow, negative value or unit mismatch | Refuse before effects with a specific reason; exact valid result accepted. Test zero separately from absent/unbounded. |
| QE-C03 Quantity basis | Plan quantity three; several item selections | Per-plan-unit rule applied once with quantity three; fixed rule once. No per-component quantity is invented. |
| QE-C04 Draft and initial readiness | Authorized preparation under a 100 GiB allocation cap creates a 20 GiB disk before T; also try a draft without preparation authority and a profile with unsupported readiness/effect handling | QE counts 20 GiB occupied, 80 GiB available, and both quota and real resource readiness precede T. Only the scoped authorized preparation succeeds. Unsupported runtime or readiness/effect handling refuses before sale/capture; a check marker cannot bypass those gates. |
| QE-C05 Purchase replay | Same purchase with another transport key | Same lot and accepted snapshot recovered; no extra quantity or quota. Changed purchase content conflicts. |
| QE-C06 Compatible increase | Add a same-rule accepted delta lot | Complete desired cap replaces the same source member; original lot retained and usage unchanged. Unsupported delta mapping or different-rule combination refuses. |
| QE-C07 Suspended purchase and race | Request an increase/addition while suspended, or suspend before its commit; separately delay an already committed active increase | Refuse uncommitted purchases with no quantity/lot/fact/provider effects or queued work. Resume is a separate guarded operation. Preserve previously committed purchase history, and fence its delayed delivery behind the newer restriction. |
| QE-C08 Unknown apply | Provider succeeds, reply lost; worker restarts | Source read/replay returns same applied revision/digest and one effect; unknown is visible until resolved. |
| QE-C09 Stale in-flight race | Old issue already in flight when newer revoke applies | Old issue cannot grant; terminal tombstone also rejects a later higher-revision grant for the same closed acquisition. Tombstone survives restart/retry; equal revision with different digest conflicts. |
| QE-C10 Ownership conflict | Another source or operator record matches subject/metric | No overwrite, deletion, implicit merge or double allowance. Atomic ownership admission refuses or follows an explicitly adopted policy. |
| QE-C11 Tenant isolation | Seller/payer tenant or arbitrary foreign resource supplied as target | Refuse without a resource-owner binding proof; authorized recovery cannot expose another tenant's state. |
| QE-C12 Window continuity | Update cap, renew, replay, suspend/resume in one consumption window | Used/reserved values and window identity preserved; only provider-owned window transition resets as specified. |
| QE-C13 Expiry geometry | Request at exact exclusive expiry; delayed timer | Access follows `[start,end)` without endpoint overlap. Unsupported inclusive-only mapping prevents enabling timed profiles. |
| QE-C14 Mixed grants | Different windows, units, expiries or incompatible terms | Refuse the unsupported profile before sale/change; no accidental sum/max, new counter or silent extension. |
| QE-C15 Below used | Applicable cap falls below occupied/spent amount | Preserve evidence; block further positive admission according to the adopted policy; no silent eviction, refund or usage clamp. |
| QE-C16 Revocation fallback | Specific quota revoked while parent/default policy permits | Required access remains denied by the accepted restriction policy; mere deactivation is not enough. |
| QE-C17 Atomic set | Failure halfway through a multi-member desired update | Atomic apply or safe restricted pending state with recovery; unsupported multi-member profiles refused. No falsely complete partial grant. |
| QE-C18 Runtime capability | Required primitive absent, or an enabled provider temporarily unavailable | Absent capability blocks enabling the profile. A transient outage leaves delivery pending/unknown under the accepted acquisition contract; no fabricated applied result or direct DB workaround. |
| QE-C19 Counter identity change | Subject/metric/kind/period change presented as update | Refuse unsupported migration; do not mint a fresh allowance under a new record. |
| QE-C20 Financial separation | Change monetary minimum or exhaust trial credit | No inferred quota arithmetic. Financial/lifecycle owners determine posture; hard access behavior still needs its own provider evidence. |
| QE-C21 Metric drift | A metric becomes Direct/inert or loses registration after materialization | Read/report materialization separately from enforcement; failed updates retain the purchased obligation and surface owner remediation. No silent metric substitution or false readiness. |
| QE-C22 Binding equivalence | A newer PlanRevision reuses an item ID with changed quota semantics | Do not treat it as the same quota binding. Require the agreed semantic-equivalence contract and all distinct-binding gates before combining lots. |
| QE-C23 Preparation-to-service continuity | Activation applies but its publication is delayed/lost while allocations are in flight | Preserve source/member/resource/allocation identity and occupied/reserved amounts, recover exact applied T, and continue prepared usable access without a second allowance, reset or revoke/recreate gap. No unrelated preparation actions are admitted. |
| QE-C24 Abort and occupied cleanup | Fenced non-activation wins; cleanup/release reply is lost and an old allocation completes late | Deny new positive allocations and fence/recover in-flight effects. Retain counted capacity until authoritative resource release and QE settlement; a timer, tombstone or dispatched delete is not release proof. Shared/existing resources require their own cleanup authority. |
| QE-C25 Unknown activation | Outcome read is absent/unreachable, then returns applied or terminal non-activation | Retain held quota and occupancy through uncertainty. Applied continues under latest lifecycle posture; non-activation follows authorized cleanup with confirmed release. No fabricated T, counter deletion, duplicate allocation or historical regrant. |
| QE-C26 Check purpose and provider discretion | Send a marked readiness-check request, recover its outcome, and apply/replay activation | Provider can recognize the purpose under exact scope/assignment/revision/operation; its declared execution and quota/accounting behavior is preserved. The marker alone is not ready, dry-run, zero consumption or privilege. Recover real effects under the same identity and retain rule/counter/window without reset or another allowance. |
| QE-C27 Purchased versus effective cap | Purchased 100 GiB meets a same-scope hard ceiling of 60 GiB; separately observe cap 100 with 40 occupied | Preserve desired 100 and report restriction 60 with evidence; no required initial ready and no silent term change. Ordinary remaining 60 after occupancy is not a mismatch. Known incompatible offers refuse; post-purchase mismatch retains remediation. |
| QE-C28 Restriction drift and recovery | Provider policy changes without a posture revision; source-state apply succeeds but restriction persists; observation is lost/reordered | Distinguish delivery from fulfillment, retain terms/counters, recover ordered actual restriction and do not overwrite operator policy. Only authoritative enforcement recovery clears the finding. |
| QE-C29 Restriction after held ready | An ordinary ceiling reduction or emergency security block races activation | Ordinary withdrawal respects the shared activation fence; applied winner retains T and later restriction history. Authorized emergency block may act immediately with incident evidence. No implicit refund, refill, counter clamp or eviction. |

Run cases across the actual Entitlements/adapter/provider boundary, including process death, concurrent revisions, duplicate delivery, lost responses and authorization denial. Establish the supported profile and durable fulfillment obligations before offering it; do not sell an unsupported quota and discover the incompatibility only at activation. This does not require a synchronous QE health check on every order. Temporary provider unavailability follows the existing accepted acquisition, capture and remediation contract: a paid obligation is retained, delivery remains pending, and usable access is never invented.

Where that contract explicitly requires protected capacity or readiness evidence, obtain it at its specified boundary. A documentation check cannot close these release gates.

## 5. Open points

| Question | Owners | What this proposal needs |
| --- | --- | --- |
| ENT-Q03 | Quota Enforcement, Products, Entitlements | Accepted distinction between purchased desired caps and provider restrictions. Exact technical-control precedence and scope/version/recovery evidence must satisfy QE-P03 and QE-C27 to QE-C29; independent operator controls are neither overwritten nor additional pooled entitlement. |
| ENT-Q04 | Quota Enforcement, Entitlements | The source namespace authority, the full-set atomicity boundary and the permission model for the apply and read operations. |
| ENT-Q05 | Quota Enforcement, Products, Entitlements | Hard suspension and revoke behavior, fallback denial, existing allocations, cap-below-used policy and separately authorized export access. |
| ENT-Q06 | Quota Enforcement, Entitlements | Exact validity boundary and clock mapping, allowed calendars and counter identity for renewals that do not align with UTC windows. |
| ENT-Q07 | Pricing, Quota Enforcement, Entitlements | Late expiry handling, tombstone retention and safe administrative recovery. |
| ENT-Q08 | Quota Enforcement, Entitlements | SDK and profile versions, executed conformance evidence and provider readiness. |

The first enabled runtime profile is allocation with one atomic member, or a proven atomic set, including scoped preparation and same-source continuity under QE-P07/ENT-Q15 once the applicable gates pass. Quota permission is not a guarantee of physical provider capacity; required actual environment preparation supplies readiness. Required hard restriction and expiry behavior must be real, not inferred from a cap field. A separately gated consumption profile needs explicit UTC calendar or supported OneTime semantics, counter continuity and the complete reservation/commit/release runtime; no anniversary approximation.

Cross-subscription pooling, differing-rule contribution schedules, rolling periods, explicit unbounded plans, rate enforcement and counter-preserving migration come later, after separate decisions. These questions are registered in [DECISIONS.md](../DECISIONS.md#open-questions).
