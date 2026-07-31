<!-- CONFLUENCE_TITLE: [BSS]: Subscriptions — Design Review Findings -->
<!-- Related: ./DESIGN.md, ./PRD.md, ./DECISIONS.md, ./SEAMS.md, ./design/ | Owners: BSS Subscriptions team -->

# Subscriptions — Design Review Findings

Walkthrough review of the 9-slice design set (started 2026-07-19). Findings are
collected per slice, ranked by severity within each. Severity legend:

- **S1** — correctness/money/data-loss risk in a real (if rare) failure path.
- **S2** — under-specified leg a reviewer/implementer will trip on; behaviour undefined.
- **S3** — wording/precision/unvalidated-assumption; low blast radius.

Status per finding: `open` → `accepted` (fix agreed) → `fixed` (docs patched) → `wontfix` (rationale recorded).

## Summary (all 9 slices reviewed 2026-07-19)

Tally: **3 × S1, 21 × S2, 14 × S3** (38 findings). The design is architecturally sound — the closed
status machine, the single commit path, the WHEN/MATH split, and the many 2026-07-15 review fixes are
solid. The findings cluster into five themes; fix in this order:

1. **Async & transient-failure recovery (highest — all S1s live here). ✅ FIXED 2026-07-19.** The
   design handled the happy-path async edges and *permanent* failures but conflated *transient*
   failures with terminal ones and left recovery/ordering legs open: F-01-1, F-01-3, F-01-4, F-03-1,
   F-03-2, F-03-3, F-04-3, F-05-3, F-07-1 — all now fixed. Keystone: a **firing-failure taxonomy**
   (slice 01 §4.3) splitting retryable (retry with bounded backoff, suppressions held) from terminal
   (never silently dropped), plus an async in-flight single-writer rule, a one-revision-per-logical-
   transition version model, entitlement changes bound to the status-edge commit, and transfer
   guards reordered before OSS re-homing. A momentary Policy/OSS blip can no longer drop a scheduled
   cancel/change, strand a cancel+new saga, or leave revoked entitlements on an active subscription.
2. **Pause × grace × quota interactions. ✅ MOSTLY FIXED 2026-07-28.** F-04-1 (grace vs pause emission
   precedence) and F-04-2 (`resume` trapped by an overlap acquired during suspension) were both closed by
   the 2026-07-28 billing pass — grace governs emission, the pause defers only collection, and a resume
   collision now fails closed naming its blocker with an operator remediation path. **F-05-2** (quota-cycle
   reset under a pause) remains open.
3. **Brand source (contested seam):** F-08-2 (root), F-02-3, F-07-2 — SUB-R5 is unresolved (per-sale
   vs Plan/SKU brand) and slice 02 overstates it as settled; AC 20 is blocked until reconciled.
4. **Multi-component billing model. ✅ FIXED 2026-07-28 (SUB-D-19).** F-08-1 — the recurring key gains a
   component dimension: `(subscriptionId, billing period, lineKey)` with a per-component traceability
   tuple, matching the coordinate rating already carries in its period-driven unit key. This was the one
   finding whose effect was S1-grade (every add-on-bearing subscription mis-billed), though filed S2.
5. **Tenancy/RLS after transfer:** F-07-3 — partitioning by the immutable `orderingTenantId` vs
   post-transfer access by the new `resourceTenantId`.

The remaining S3s are wording/precision and unvalidated-NFR items. Two findings were closed on review
(F-02-5 cohort sourcing; the rating-stranding half of F-01-1, addressed by slice 03).

> **Status sync 2026-07-28.** A cross-gear review pass found this register lagging the design: F-04-1 and
> F-04-2 had been fixed by the 2026-07-28 billing pass while still marked `open` here, and F-08-1 was
> resolved the same day as SUB-D-19. Running tally after the sync: **15 fixed / 1 closed / 22 open** of 38.
> When a decision closes a finding, update both the finding's Status line and the theme list above.

---

## Slice 01 — Lifecycle Foundation

### F-01-1 (S1) — Failed scheduled-intent firing has no recovery leg
- **Where**: `design/01-foundation-lifecycle.md` §3.6 (scheduled-intent lifecycle), §4.3, §4.5.
- **Problem**: §3.6 states only *"a Policy deny at firing leaves state unchanged."* What
  happens to the intent afterwards is unspecified across slices 01/03/04: does it stay
  scheduled and retry, is it consumed and silently dropped, is there a dead-letter + alarm?
- **Failure scenario**: A customer schedules an end-of-term `cancel`. At `effectiveAt` the
  firing hits Policy unavailability → fail-closed → cancel does not commit. If the intent is
  dropped, the subscription **renews and is billed for the period the customer asked to
  cancel** — direct money + dispute.
- **Asymmetry**: `RenewalJob` double-extension is rigorously idempotent
  (`(subscriptionId, currentTermSequence)`, slice 04 §4.x); the symmetric *failed-firing*
  case has no recovery model.
- **Update (after slice 03)**: Slice 03 §3.6 partially addresses this — a firing failure emits
  `SubscriptionIntentUnscheduled(reason=firing_failed)` to void the announced boundary so rating
  is not stranded. But it treats the failure as **terminal** with no retry (see F-03-1), so the
  retry/dead-letter half of this finding remains open, and the renewal-suppression dangling half
  is now F-03-2.
- **Suggested fix**: Define firing-failure semantics — retryable failures (Policy unavailable,
  `oss_unconfirmed`) retry with bounded backoff before terminal abandonment; dead-letter + alarm
  on exhaustion; renewal/next-term-recurring suppression holds across the retry window.
- **Status**: **fixed** (2026-07-19) — firing-failure taxonomy added to slice 01 §4.3 (retryable
  retry with bounded backoff + held suppressions + dead-letter/alarm on exhaustion; terminal never
  silently dropped). Referenced from slice 01 §3.6.

### F-01-2 (S2) — Idempotency replay during an approval hold is under-specified
- **Where**: `design/01-foundation-lifecycle.md` §3.6 step 2 vs step 3; DECISIONS SUB-D-08.
- **Problem**: Step 2 says a replay returns *"the original outcome"*; step 3 parks
  approval-required types (`transfer`, `extendTrial`) at `pending → approved` with the
  idempotency key covering the whole hold. While `pending`, there **is no outcome yet**, so
  the registry's "return original outcome" contract has no defined answer.
- **Failure scenario**: `transfer` submitted, awaiting approval. Client retry-logic resends
  the same request. Does the registry return "still pending", or create a second `Approval`?
  Undefined.
- **Suggested fix**: Introduce an explicit non-terminal registry response
  (`in_progress`/`pending_approval`); state that a replay during the hold does NOT create a
  second `Approval` and re-attaches to the parked request.
- **Status**: open.

### F-01-3 (S1) — Async OSS edges: the "approved, awaiting OSS" window is unguarded against concurrent transitions
- **Where**: `design/01-foundation-lifecycle.md` §3.6 async note, §4.5.
- **Problem**: In the async model the intent commits (`approved`, work order issued) and the
  status edge commits later on the confirmation event. The design never says what happens to
  the subscription during that window (no `in-flight`/`while pending` handling found).
- **Failure scenario**: `activate` is in flight (approved, awaiting OSS provisioning). A
  `cancel` arrives. The guard evaluates against the *current* status — still `draft`. Which
  wins? Is the activate work order cancelled? Does the activation confirmation or the cancel
  commit land first? Currently a rules-free race.
- **Suggested fix**: Add an explicit in-flight lock/rule — while an unresolved OSS-leg
  request exists, competing resource-affecting transitions are rejected (or queued) with a
  typed rejection; define reconciliation if a late confirmation meets a superseding state.
- **Status**: **fixed** (2026-07-19) — slice 01 §3.6 "In-flight single-writer rule": competing
  resource transitions rejected `transition_in_flight`; operator supersede cancels the outstanding
  work order first.

### F-01-4 (S2) — One logical async transition produces two versions/revisions; the version invariant is written for one
- **Where**: `design/01-foundation-lifecycle.md` §4.2 (`fr-monotonic-version`) vs §3.6 async note.
- **Problem**: §4.2 says each committed commercial-meaning change increments `version` and
  appends a `SubscriptionRevision`. But an async `activate` commits twice (intent/`approved`,
  then status/`applied`). Is "work order issued" a commercial-meaning change (bumps version,
  emits a revision) or a service record?
- **Impact**: Ambiguity hits (a) optimistic concurrency — a client ETag goes stale after the
  first bump though the transition "hasn't happened"; (b) the revision log — auditors see two
  revisions for one activation.
- **Suggested fix**: Explicitly model "commit-of-intent" vs "commit-of-status" in the version
  scheme and state which bumps `version`/emits a `SubscriptionRevision`.
- **Status**: **fixed** (2026-07-19) — slice 01 §3.6 "Version model for async edges": the intent
  commit does not bump `version`/emit a revision; only the status-edge commit does — one revision
  per logical transition, client ETag stays valid until apply.

### F-01-5 (S3) — `draft→cancelled` void: gated-or-not stated by type, contradicting the edge-level rule
- **Where**: `design/01-foundation-lifecycle.md` §4.1 vs §4.5.
- **Problem**: §4.1 marks the void edge *"not resource-affecting, no OSS leg"* ⇒ by §4.5 it is
  NOT Policy-gated. But §4.5 lists gated transitions **by type** (*"`cancel`…"*), and the void
  is the same `cancel` type — so one type is gated on `active→cancelled` but not on
  `draft→cancelled`. The type-wise wording reads as "all cancels are gated".
- **Suggested fix**: Describe the Policy gate **per edge**, not per type; call out the void
  edge as the explicit non-gated `cancel`.
- **Status**: open.

### F-01-6 (S3) — Latency NFR: an unvalidated ~500ms Policy round-trip budget + an undeclared availability coupling
- **Where**: `design/01-foundation-lifecycle.md` §1.2 NFR table.
- **Problem**: Two NFRs — `<1s` including Policy, `<500ms` excluding Policy — implicitly budget
  ~500ms for a synchronous Policy round-trip on every resource-affecting transition; both are
  "workshop-pending/baseline" (unvalidated). Separately, Policy sits on both the latency
  critical path (step 6) and, via fail-closed, on availability (its downtime blocks *all*
  resource transitions).
- **Suggested fix**: Validate the Policy round-trip budget; explicitly record the "Policy is a
  single point for both latency and availability" risk, and either document a degradation mode
  (cached allow-decisions, if business permits) or state the coupling is accepted.
- **Status**: open.

---

## Slice 02 — Composition & Versioning

### F-02-1 (S2) — SUB-P5 sellability gate is adopted but not stated in the slice
- **Where**: `design/02-composition-versioning.md` §3.5, §2.2 (frozen-refs constraint); SEAMS SUB-P5.
- **Problem**: §3.5 cites SUB-P5 as a generic "linkage" dependency. In SEAMS, SUB-P5 is the
  **fail-closed sellability gate**: `PlanLink`/`AddOn` resolve only against *published* plans that
  pass the pricing sellability gate; a draft or `not_sellable_ga` plan MUST fail the
  `create`/`changePlan` guard fail-closed. The slice never states this — not in a guard, not in
  the "frozen catalog references only" constraint (which says "published catalog keys" but not the
  sellability precondition). The obligation silently dropped out of the slice design.
- **Suggested fix**: Add the sellability precondition as an explicit `create`/`changePlan` guard
  in §4.1/§2.2, referencing SUB-P5.
- **Status**: open.

### F-02-2 (S2) — Half-open interval convention not reconciled across the rating seam
- **Where**: `design/02-composition-versioning.md` §4.1; SEAMS SUB-R1.
- **Problem**: §4.1 declares `[from, to)` half-open intervals on the Subscriptions side. SUB-R1
  says rating "slices usage + prorates at the same boundary" and shares the boundary instant — but
  neither doc states that rating uses the **same half-open convention**. A convention mismatch
  double-counts or drops the boundary instant.
- **Failure scenario**: Subscriptions closes the prior plan interval at `boundary` (exclusive) and
  opens the new at `boundary` (inclusive). If rating treats the boundary as closed on both sides,
  the boundary instant is priced under both plans (double-count) or neither (drop).
- **Suggested fix**: Add the half-open convention to the SUB-R1 mirror-reconcile; assert both sides
  agree `[from, to)`.
- **Status**: open.

### F-02-3 (S2) — `brandId` behaviour under ownership transfer is undefined
- **Where**: `design/02-composition-versioning.md` §4.4; `design/07-tenancy-transfer.md` (no `brand` mention).
- **Problem**: `brandId` is a per-sale attribute recorded at creation and drives brand-scoped
  overlay matching in rating. Slice 07 (transfer) does not mention `brand` at all; §4.2 explicitly
  says the `(currency, region)` segment persists across transfers, but says nothing about `brandId`.
- **Failure scenario**: A subscription sold under brand A is transferred to a new owner (seller/payer
  tenants change). Does `brandId=A` keep driving overlay matching post-transfer, or move to the new
  seller's brand? Pricing outcome depends on the answer.
- **Suggested fix**: State in slice 07 that per-sale `brandId` is immutable across transfer (or
  define the re-attribution rule), consistent with the "per-sale" framing.
- **Status**: open — confirmed in slice 07 (§4.4 lists no `brandId` handling; see F-07-2).

### F-02-4 (S3) — "Append-only interval algebra" principle contradicts "future is replaceable"
- **Where**: `design/02-composition-versioning.md` §2.1 vs §4.1.
- **Problem**: §2.1 declares composition **append-only**; §4.1 permits voiding/replacing *future*
  intervals before `effectiveFrom` (a mutate/delete, not an append). Reconcilable (append-only
  applies to history) but the absolute wording misleads.
- **Suggested fix**: Reword to "append-only for history; future intervals are replaceable until
  effective."
- **Status**: open.

### F-02-5 (S3, tracks existing SUB-R1 flag) — `priceEligibility` inputs (`activatedAt`, `cohort`) not enumerated among published composition facts
- **Where**: `design/02-composition-versioning.md` §3.1/§1.2; SEAMS SUB-R1 (downgraded).
- **Problem**: SUB-R1 was downgraded from ALIGNED because rating names seat-count and
  `priceEligibility` inputs the Subscriptions side omitted. Seat-count is now covered
  (`quantity_interval`), but `activatedAt`/`cohort` appear in neither the domain model nor the
  drivers of slice 02 — the composition slice is where these facts belong.
- **Suggested fix**: Enumerate `activatedAt` and `cohort` binding in the composition read-model
  facts as part of the SUB-R1 mirror-reconcile.
- **Status**: closed — slice 09 §4.2 enumerates `activatedAt`/`cohort`; PRD §1054 sources `cohort`
  "via the pinned price id" (pricing ADR-0002). Not dangling. Residual wording noted as F-09-1.

---

## Slice 03 — Plan & Quantity Changes

### F-03-1 (S1) — Transient and permanent firing failures conflated; a momentary Policy outage permanently abandons a scheduled change/cancel/ramp
- **Where**: `design/03-plan-changes.md` §3.6 (firing-failure retraction), §4.5 (mid-ramp failure).
- **Problem**: Both treat any "terminal firing failure" — `Policy deny`, `guard_violation`,
  `oss_unconfirmed` — identically. But `oss_unconfirmed` and Policy *unavailability* are transient,
  while `deny`/`guard_violation` are permanent. Collapsing them into "terminal → void boundary /
  halt ramp" permanently drops the action (and a ramp additionally parks "suspended pending
  re-authoring") on a momentary blip.
- **Failure scenario**: An end-of-term `cancel` fires while Policy is briefly unavailable →
  fail-closed → §3.6 declares a terminal firing failure → `unschedule(firing_failed)` → the cancel
  is **abandoned**. The customer asked to cancel; it silently did not happen. For a ramp, the whole
  Contract-authored schedule halts pending human re-authoring.
- **Suggested fix**: Split retryable (Policy unavailable, `oss_unconfirmed`) from terminal
  (`deny`, `guard_violation`); retry with bounded backoff until a deadline before abandoning.
- **Status**: **fixed** (2026-07-19) — slice 03 §3.6 (retraction) and §4.5 (mid-ramp) now retract/
  halt only on terminal failure; retryable failures retry per the slice-01 taxonomy.

### F-03-2 (S2) — After a firing-failed cancel is voided, renewal-suppression state is left dangling
- **Where**: `design/03-plan-changes.md` §3.6; `design/01-foundation-lifecycle.md` §4.3; slice 04.
- **Problem**: Slice 01 §4.3 says a pending end-of-term cancel suppresses renewal + next-term
  recurring. Slice 03 §3.6 voids the boundary on firing failure via `unschedule(firing_failed)`.
  Neither states what happens to the suppression: does renewal re-arm (though the customer asked to
  cancel), or is the subscription left in limbo? And was the renewal window already missed while the
  cancel was pending? The `RenewalJob`-vs-firing-failed-cancel sequencing is unaddressed.
- **Suggested fix**: Define the post-void state explicitly — re-arm renewal + emit an operator
  signal, or hold in a defined state; specify ordering vs the renewal job.
- **Status**: **fixed** (2026-07-19) — slice 04 §4.3: on terminal opt-out/cancel firing failure the
  term is held un-renewed with an operator alarm (no silent renew); re-arming is an explicit audited
  action. Ordering vs `RenewalJob` fixed in F-04-3.

### F-03-3 (S2) — Asymmetric compensation in the cancel+new saga
- **Where**: `design/03-plan-changes.md` §4.3 (cancel+new), §4.4 (supersedes exemption).
- **Problem**: §4.3 schedules the predecessor cancel and successor activation on the **same boundary
  instant**; §4.4 exempts the successor from the overlap rule against its predecessor during the
  handover. Compensation in §4.3 step 4 covers only "successor failure → void successor." The
  reverse — the **predecessor-cancel firing fails** (F-03-1) after the successor has activated — is
  not covered: two `active` on one `overlapScopeKey` with the exemption no longer justified (the
  predecessor never ended).
- **Suggested fix**: Add the symmetric compensation branch — on predecessor-cancel firing failure
  after successor activation, either retry the cancel (F-03-1) or roll the successor back to a
  defined state; hold the exemption until the predecessor actually ends.
- **Status**: **fixed** (2026-07-19) — slice 03 §4.3 symmetric-compensation bullet + §4.4 exemption
  now bound to the predecessor **actually ending** (held open across the retry window); terminal
  failure alarms, never two unexempted `active`.

### F-03-4 (S3) — `at(date)` absent from `changeMode`
- **Where**: `design/03-plan-changes.md` §3.1; `design/01-foundation-lifecycle.md` §4.3.
- **Problem**: `cancelMode` includes `at(date)`; `changeMode ∈ {immediate, next-cycle, end-of-term}`
  does not. An arbitrary-future-date plan change appears possible only via a Contract ramp.
- **Suggested fix**: If intentional, state "arbitrary-date plan changes go through ramps"; otherwise
  add `at(date)` to `changeMode`.
- **Status**: open.

### F-03-5 (S3, verify in slice 07) — Overlap collision on `transfer` (payer change)
- **Where**: `design/03-plan-changes.md` §4.4; `design/07-tenancy-transfer.md`.
- **Problem**: §4.4 re-evaluates overlap on a `transfer` that alters `payerTenantId`, rejecting
  fail-closed on collision. Since `overlapScopeKey = (payerTenantId, productKey)`, the new payer may
  already hold an `active` of the same product. A mid-saga transfer rejection has its own
  consequences that must be reconciled with slice 07's transfer mechanics.
- **Suggested fix**: Verify slice 07 handles an overlap-rejected transfer without leaving the
  subscription in a half-transferred state.
- **Status**: confirmed problematic in slice 07 — the overlap re-check runs at the completing commit,
  after OSS re-homing side effects; see F-07-1.

---

## Slice 04 — Suspension, Renewal & Grace

### F-04-1 (S2) — Recurring-fact emission is ambiguous when a pause is layered on an in-flight grace ladder
- **Where**: `design/04-suspension-renewal-grace.md` §4.4 vs §4.2 (pause-applied-while-in-grace).
- **Problem**: §4.4 says during grace the blocked next-term recurring **MUST NOT be emitted**. §4.2
  says `collectionPaused` emits the recurring fact **marked `collectionPaused`**. §4.2's
  "pause applied while already in grace" merges the two: does the previously grace-blocked recurring
  now get emitted marked-paused (pause rule) or stay suppressed (grace rule)? Both norms apply and
  prescribe opposite emission behaviour.
- **Failure scenario**: A subscription in grace (recurring suppressed) receives a `pauseCollection`.
  If the implementer follows the pause rule, a recurring fact is emitted for a period the grace rule
  said must not be emitted — potentially colliding with the later grace-release re-emit on the same
  `(subscriptionId, billing period)` key (§4.3 late success).
- **Suggested fix**: State explicitly which rule wins in the merged state — recommend: while frozen,
  the recurring stays suppressed (grace semantics dominate), no paused-marked emission.
- **Status**: **fixed** (2026-07-28, recorded 2026-07-28 review sync) — precedence pinned as recommended:
  slice 08 §4.3 "grace governs **emission**, the pause defers **collection** of emitted facts only", so a
  grace-blocked next-term fact stays un-emitted through any pause window; slice 04 §4.4 carries the same
  rule. Recorded in SUB-D-07's 2026-07-28 amendment.

### F-04-2 (S2) — `resume` can be permanently blocked by an overlap acquired during suspension
- **Where**: `design/04-suspension-renewal-grace.md` §4.1 (resume re-runs overlap check); slice 03 §4.4.
- **Problem**: Resume re-runs the overlap check on entry into `active`. If, while subscription A is
  `suspended`, subscription B with the same `overlapScopeKey` is activated (legal — A is not active),
  a later `resume` of A collides with B and is rejected fail-closed. With the default
  `maxConcurrentActive = 1`, a paid, resolvable subscription can become permanently un-resumable.
- **Failure scenario**: Grace-suspended A resolves its payment; the customer resumes; resume is
  rejected because B took the overlap slot during suspension. No defined resolution.
- **Suggested fix**: Define precedence (e.g. a suspended subscription reserves its overlap slot, or
  an operator-mediated resolution path); do not silently trap the resume.
- **Status**: **fixed** (2026-07-28, recorded 2026-07-28 review sync) — slice 04 §4.1 "Collision on resume
  (explicit, 2026-07-28)": the resume fails **closed** (`guard_violation`) with the conflicting
  subscription(s) **enumerated in the problem response**, remediation is operator-driven (cancel/re-scope
  the conflicting acquisition, or cancel+new), and the resume retries cleanly afterwards — so the trap is
  never silent, which is what the finding required. No auto-rebind, no silent supersession of either side.

### F-04-3 (S2, confirms F-03-2) — Scheduled non-renewal (opt-out) vs `RenewalJob` sequencing at term end is undefined
- **Where**: `design/04-suspension-renewal-grace.md` §4.5 (opt-out) and §4.3 (`RenewalJob`).
- **Problem**: Opt-out is a scheduled non-renewal via `IntentScheduler` firing "at term end";
  `RenewalJob` also evaluates `endDate` at term end. Both target the same instant. If the
  non-renewal firing fails and its boundary is voided (slice 03 §3.6), does `RenewalJob` then renew?
  Which runs first? No sequencing is specified — the general form of F-03-2.
- **Suggested fix**: Define ordering between the scheduled non-renewal firing and `RenewalJob` at
  term end, and the post-void renewal decision.
- **Status**: **fixed** (2026-07-19) — slice 04 §4.3: `RenewalJob` reads the pending-intent set
  first and does not extend while a non-renewal intent is live; a failed non-renewal follows the
  taxonomy (retry with suppression held / terminal parks + alarm), never an implicit renewal.

### F-04-4 (S3, verify in slice 05) — Entitlement freeze vs OSS pause ordering in an async suspend is unspecified
- **Where**: `design/04-suspension-renewal-grace.md` §4.1; foundation §3.6 async note; slice 05.
- **Problem**: Suspend revokes/freezes entitlements and pauses OSS, and (OSS-leg edge) runs async:
  intent commits, status edge commits on OSS confirmation. Whether entitlements are frozen at
  intent-commit or at status-edge commit is unspecified — a window where the user has no
  entitlements but the resource still runs (or the reverse).
- **Suggested fix**: Specify the freeze point relative to the OSS pause confirmation; verify against
  slice 05.
- **Status**: open — slice 05 §4.1 does not resolve it (says "same commit as the transition" without
  disambiguating intent-commit vs status-edge-commit in the async case); see F-05-3.

---

## Slice 05 — Entitlement Lifecycle

### F-05-1 (S2) — Grant-set pinning is undefined: live-published vs snapshot-pinned
- **Where**: `design/05-entitlements.md` §4.2, §2.2; SEAMS SUB-P2; slice 02 frozen-refs constraint.
- **Problem**: §4.2/SUB-P2 assign entitlements from the "published grant set" (the live template),
  while slice 02 freezes catalog refs for reproduction. Nothing states whether entitlement
  assignment is pinned to the subscription's activation snapshot or re-resolved against the live
  template at each transition.
- **Failure scenario**: Pricing republishes a plan's grant set. The subscription's next committed
  change (§4.2 "on committed change") re-materialises entitlements against the **new** template —
  silently drifting what the customer has, with no explicit rule licensing it.
- **Suggested fix**: State the rule explicitly — either "assignment resolves the grant set published
  as-of the transition instant" (a deliberate forward-looking choice, distinct from billing freeze)
  or "pinned to the version resolved at activation."
- **Status**: open.

### F-05-2 (S2, extends F-04-1) — Quota-cycle reset during a long `collectionPaused` is undefined
- **Where**: `design/05-entitlements.md` §4.4 (cycle reset), §4.1; slice 04 §4.2.
- **Problem**: §4.4 resets counters on the recurring period cut (slice 08), "not by a separate
  clock." During `collectionPaused` the recurring cut is suppressed/deferred (slice 04 §4.2), so the
  cut never fires → the quota cycle never resets. The user either stays exhausted (no fresh quota) or
  keeps the old allowance indefinitely. §4.1 says counters persist across pause, but says nothing
  about reset during a long pause.
- **Suggested fix**: Define quota-cycle behaviour under a long pause — reset on a logical anchor even
  when the billing cut is deferred, or explicitly hold the cycle and document the consequence.
- **Status**: open.

### F-05-3 (S2, sharpens F-04-4) — "Issue/revoke in the same commit" is not reconciled with async edges
- **Where**: `design/05-entitlements.md` §4.1; foundation §3.6 async note; F-03-1.
- **Problem**: §4.1 says issue/revoke is part of "the same commit as the transition" and "posture
  never lags committed state." But suspend/cancel are async OSS-leg edges (intent commit, then
  status-edge commit on OSS confirmation). Which commit carries the entitlement change is unstated.
- **Failure scenario**: An async `cancel` is abandoned (F-03-1: OSS unconfirmed / Policy blip) after
  entitlements were revoked at intent-commit → an **active subscription with revoked entitlements**.
- **Suggested fix**: Bind the entitlement change to the status-edge commit (on OSS confirmation) for
  async edges, or define compensation if it rides the intent commit.
- **Status**: **fixed** (2026-07-19) — slice 05 §4.1: for async edges "the same commit" is the
  status-edge commit; an abandoned transition commits no entitlement change — no active-with-revoked
  window.

### F-05-4 (S3) — An immediate seat decrease can retroactively block the current cycle
- **Where**: `design/05-entitlements.md` §4.4; slice 03 §4.2.
- **Problem**: `updateQuantity` re-materialises per-seat quotas per `changeMode`. Decreases default
  next-cycle (safe), but a policy-forced immediate decrease drops the quota mid-cycle; if the cycle's
  usage already exceeds the new lower quota, the check state flips to `blocking` immediately.
- **Suggested fix**: If intended, state the consequence; otherwise clamp the mid-cycle quota to
  `max(new, already-consumed)` until the next cycle.
- **Status**: open.

---

## Slice 06 — Trial Runtime & Conversion

### F-06-1 (S2, revenue/abuse) — Free paid-access vector: no-method conversion + 7-day paid grace + unbounded serial re-trials
- **Where**: `design/06-trials.md` §4.2, §3.6, §4.5; slice 04 grace ladder.
- **Problem**: On conversion the boundary advances and **target-phase (paid) entitlements are issued
  even with no payment method on file** (§4.2), the failure entering the 7-day grace ladder. §4.5
  leaves repeat-trial eligibility "open" and notes the overlap rule blocks only *concurrent*
  duplicates — serial re-trials are not blocked.
- **Failure scenario**: trial → convert with no method → 7 days of full paid access via grace →
  cancel → new trial → repeat. The "never trade access for collection" principle, combined with
  no-method conversion and unbounded re-trials, is a free-tier exploit.
- **Suggested fix**: Add an anchor — serial-re-trial limiting (tenant/identity level), and/or
  no-method conversion grants reduced access rather than full paid entitlements for the grace window.
- **Status**: **open — escalated and now tracked** (2026-07-28). Deliberately *not* decided in design: each
  candidate anchor (re-trial limiting / reduced no-method access / payment method required before the
  boundary advances) trades conversion funnel against revenue leakage, which is a Product/Finance call. The
  composed exploit loop is now written up explicitly as its own PRD §15 row (owner Product / Finance, due
  before trial GA) rather than living only as three separately-innocuous open legs.

### F-06-2 (S2, cross-slice) — Grace late-success is undefined for a first-time trial→paid conversion
- **Where**: `design/06-trials.md` §4.2 vs slice 04 §4.3.
- **Problem**: §4.2 says grace start for a first trial→paid conversion is the conversion instant,
  "there is no prior term." Slice 04 §4.3's late-success rule is defined against the **old term end**
  ("new term starts at the old term end, backdated"). A first conversion has no prior term end, so
  the backdating rule is undefined.
- **Suggested fix**: State that for a trial conversion, grace late-success starts the term at the
  conversion instant, not a (non-existent) old term end.
- **Status**: open.

### F-06-3 (S2) — Double-conversion between `convertTrial` and the term-conversion job is not mechanically prevented
- **Where**: `design/06-trials.md` §3.6, §3.1 (`ConversionRecord` idempotency key), §4.3.
- **Problem**: The term-conversion job fires at trial end; `convertTrial` fires early. Both advance
  the phase boundary. `convertTrial` keys on client `(subscriptionId, idempotencyKey)`; the
  term-job's key derivation is unspecified and not coordinated with it. "Zero double" is claimed but
  the mechanism against "convertTrial, then the term-job fires" is not stated.
- **Suggested fix**: Specify that the term-conversion job guards on an active trial phase (no-op if
  already converted), rather than relying on an uncoordinated idempotency key.
- **Status**: **fixed** (2026-07-28) — slice 06 §3.8 adopts the suggested fix: a **state guard**, not a
  shared key. The job re-reads phase state inside its commit and no-ops when the trial phase is no longer
  active; `convertTrial` fails closed if the boundary already advanced. The two keys stay deliberately
  uncoordinated because they key different actors — only the phase state can arbitrate. *(Not to be
  confused with the separate 2026-07-28 pending-intent fix in §3.6, which addresses cancel-before-conversion.)*

### F-06-4 (S3) — Trial draft-vs-active status is ambiguous and couples to "entitlements issue on activate"
- **Where**: `design/06-trials.md` §1.1, §4.1; slice 05 §4.1.
- **Problem**: §1.1 says "draft before first paid activation, active under trial service"; §4.1 says
  feature access during trial follows the trial-phase grant set; slice 05 issues entitlements only on
  a resource-affecting transition (activate). A service-providing trial must therefore be `active`,
  making "draft before first paid activation" misleading.
- **Suggested fix**: Tighten the wording — a service-providing trial is `active`; `draft` is only a
  not-yet-started trial subscription.
- **Status**: open.

### F-06-5 (S3) — `extendTrial` approved after the trial already converted has no guard
- **Where**: `design/06-trials.md` §4.5.
- **Problem**: `extendTrial` is approval-gated (maker-checker delay). During the approval hold the
  trial can reach its end and convert via the term-job. An approval landing post-conversion conflicts
  with the completed conversion; no "cannot extend an already-converted trial" guard is stated.
- **Suggested fix**: Add the guard; on approval, re-read state and reject if the trial phase already
  converted.
- **Status**: open.

---

## Slice 07 — Multi-Tenant Ownership & Transfer

### F-07-1 (S2, sharpens F-03-5) — Commit-time transfer guards run after the OSS re-homing side effect
- **Where**: `design/07-tenancy-transfer.md` §4.4 (guards at commit).
- **Problem**: §4.4 lists the overlap re-check (new payer key) and delegation-proof re-validation as
  "guards at commit," while OSS re-homing work orders are issued "before the completing commit"
  (async). So resources re-home first; the guards run in the completing commit. A guard failure at
  commit leaves resources re-homed but ownership not transferred (the commit aborts).
- **Failure scenario**: A payer rebind whose new overlap key collides, or whose proof expired during
  the approval hold, fails at commit — after OSS already moved the resources to the new tenant.
  Inconsistent: resources at new tenant, subscription still owned by the old.
- **Suggested fix**: Run the overlap re-check and proof re-validation **before** issuing OSS
  re-homing; issue re-homing only once all guards pass.
- **Status**: **fixed** (2026-07-19) — slice 07 §4.4 reordered: overlap re-check + proof
  re-validation run before any OSS re-homing work order; a guard failure aborts with resources still
  homed to the old tenant.

### F-07-2 (S2, confirms F-02-3) — `brandId` not addressed on transfer while `sellerTenantId` rebinds
- **Where**: `design/07-tenancy-transfer.md` §4.4, §4.1; slice 02 §4.4.
- **Problem**: §4.4 enumerates the rebinding commercial axes (incl. `sellerTenantId` = channel/
  marketplace seller) and the re-keyed consumers; `brandId` appears nowhere. Post-transfer the
  subscription has a new seller but (if immutable) the old per-sale brand, and rating's brand-scoped
  overlay matching keys on a brand no longer matching the seller.
- **Suggested fix**: State the rule — per-sale `brandId` is immutable across transfer — and reconcile
  it with the `sellerTenantId` rebind (brand-scoped overlays follow the original sale, not the new
  seller), or define re-attribution.
- **Status**: open.

### F-07-3 (S2) — Partitioning/RLS by the immutable `orderingTenantId` vs the new `resourceTenantId` after transfer
- **Where**: `design/07-tenancy-transfer.md` §4.1; slice 01 §3.7.
- **Problem**: `orderingTenantId` = `resourceTenantId` at creation and is immutable; the aggregate is
  tenant-partitioned by it. A transfer can rebind `resourceTenantId`, so the subscription physically
  lives in the *old* tenant's partition/scope. Whether RLS keys on `orderingTenantId` or
  `resourceTenantId` is not reconciled: if RLS is on `orderingTenantId`, the new operational owner
  cannot see their own subscription under normal RLS. The check surface re-keys, but aggregate access
  via the new owner's RLS scope is not addressed.
- **Suggested fix**: State the RLS key explicitly and how a post-transfer owner's reads reach a
  subscription that remains in the original partition (e.g. RLS on current axes with partition-key
  decoupled from RLS scope).
- **Status**: open (verify RLS key).

### F-07-4 (S3) — Payer rebind defaults next-cycle but the overlap key changes at commit
- **Where**: `design/07-tenancy-transfer.md` §4.4.
- **Problem**: The payer rebind defaults next-cycle (the in-flight period stays with the old payer),
  yet the overlap re-check uses the new key at commit — so `overlapScopeKey` (which includes
  `payerTenantId`) changes at commit while billing still bills the old payer for the current period.
- **Suggested fix**: State whether the overlap key follows the commit instant or the next-cycle
  billing effect, and confirm the transient is intended.
- **Status**: open.

---

## Slice 08 — Event Model & Billing Alignment

### F-08-1 (S2) — Recurring idempotency key `(subscriptionId, billing period)` + singular traceability tuple cannot represent a multi-component subscription
- **Where**: `design/08-events-billing.md` §4.3, §3.1; SEAMS SUB-B1/SUB-R6.
- **Problem**: "At most one recurring item per `(subscriptionId, billing period)`" with a **singular**
  traceability tuple `{subscriptionId, skuId, planId, priceId}`. But a subscription is a plan plus N
  add-ons (slice 02 `AddOn`), each a recurring component with its own `priceId`. One fact per period
  with one tuple cannot represent them; either the key needs a component dimension (`+ lineKey`/
  `priceId`) or there are multiple facts per period, which breaks "at most one per key."
- **Suggested fix**: Add a component/line dimension to the recurring key and traceability, or state
  that add-ons are separately-keyed recurring facts.
- **Status**: **fixed** (2026-07-28) — **SUB-D-19** takes the first option: the fact is cut **per billable
  component**, key `(subscriptionId, billing period, lineKey)` with a per-component traceability tuple.
  `lineKey` is deliberately the coordinate rating already carries in its period-driven unit key
  `(subscription, priceId, chargeKind, lineKey, AnchorPeriod)`, so SUB-D-07's "the priced line inherits the
  fact's key" becomes well-defined for plan + add-ons. Propagated across slice 08 (§3.1/§3.6/§3.7/§4.3 +
  constraint), PRD §6.8/§9.2/AC 5, SEAMS SUB-B1/SUB-R6, and the rating-side SB1 note. Escalated to **S1** in
  effect: as written the contract mis-billed every add-on-bearing subscription.

### F-08-2 (S2, root of the brand cluster F-02-3/F-07-2) — Brand-source for overlay matching is contested (SUB-R5) but slice 02 presents it as settled
- **Where**: SEAMS SUB-R5; `design/02-composition-versioning.md` §4.4; also slices 06/07 brand refs.
- **Problem**: SUB-R5 states the brand-match source is contested — this PRD publishes the **per-sale
  `brandId`** as the overlay-match source, while rating matches `brand` against **Plan/SKU `brandId`
  @ `t`** — and "AC 20 is not implementable while they disagree." Slice 02 §4.4 asserts as fact that
  per-sale `brandId` "publishes it… **so rating matches** brand-scoped overlays," taking a side on an
  unresolved seam without flagging it open.
- **Impact**: Deepens F-02-3/F-07-2 — the issue is not only transfer behaviour but which brand source
  feeds matching at all; the entire brand path rests on an unresolved seam.
- **Suggested fix**: Reword slice 02 §4.4 to flag SUB-R5 as contested/open (not "so rating matches");
  resolve the source with rating before Design lock, then settle F-02-3/F-07-2 on top.
- **Status**: **partially fixed** (2026-07-28) — the doc half is done: slice 02 §4.4 (and the
  `fr-sale-brand-attribution` driver row) now present the per-sale `brandId` as **a candidate** and flag
  SUB-R5 as an open contested seam with AC 20 blocked, cross-referencing F-02-3/F-07-2. The seam itself is
  **still open** — pin the source with rating before Design lock, then settle F-02-3/F-07-2 on top.

### F-08-3 (S3, disclosed risk) — The slice's core recurring handoff (SUB-R6) is an unlocked HIGH Joint seam
- **Where**: `design/08-events-billing.md` §4.3; SEAMS SUB-R6.
- **Problem**: The money-free-fact → rating-prices → Billing-posts flow rests on SUB-R6, flagged in
  SEAMS as "Joint (SUB-D-07) … needs the rating counterpart contract + joint fixture before Design
  lock." The slice's central handoff contract is not yet agreed with rating.
- **Suggested fix**: Keep as an explicit Design-lock gating dependency; land the joint fixture.
- **Status**: open (disclosed).

---

## Slice 09 — Consumer & Integration Contracts

*(Assembly slice — projects the capability slices; consistent, and correctly flags SUB-R5 as open,
unlike slice 02. Two minor completeness/wording items.)*

### F-09-1 (S3, closes F-02-5) — `cohort` presented as a Subscriptions-"bound" field though it derives from the rating-sealed pinned priceId
- **Where**: `design/09-consumer-contracts.md` §4.2; PRD §1054, §190; pricing ADR-0002.
- **Problem**: §4.2 lists "bound cohort" among the facts this gear publishes. PRD §1054 sources
  `cohort` "via the pinned price id," and cohort generations are a pricing concept (ADR-0002) — so
  Subscriptions does not bind it; it is derived from the rating-sealed part of `pricingSnapshotRef`.
  The field is sourced (F-02-5 is not a dangling leg), but "bound cohort" misattributes ownership.
- **Suggested fix**: Reword to "cohort derived via the pinned priceId (pricing-owned)."
- **Status**: open (wording).

### F-09-2 (S3) — SUB-B6 (inbound `billedThroughAt`) referenced in §4.3 but absent from the §3.5 external-deps table
- **Where**: `design/09-consumer-contracts.md` §4.3 vs §3.5.
- **Problem**: §3.5 lists only SUB-B1 for Billing; the backdating guard consumes `billedThroughAt`
  (SUB-B6), an inbound Billing dependency missing from the table.
- **Suggested fix**: Add SUB-B6 (inbound) to the §3.5 dependency table.
- **Status**: open.
