<!-- CONFLUENCE_TITLE: [BSS]: Subscriptions — Decisions Log -->
<!-- Related: ./PRD.md, ./STRIPE-ZUORA-GAP-ANALYSIS.md | Owners: BSS Subscriptions team -->

# Subscriptions — Decisions Log

Decision IDs are `SUB-D-NN`. Autonomous decisions follow the pricing-gear pattern: adopted into
the docs immediately, **flagged for veto** until Product/Architecture confirms. Severity:
H (high — commercial/model shape), M (medium), L (low).

## Status board

| ID | Sev | Decision | Status |
|----|-----|----------|--------|
| SUB-D-01 | M | Scheduled lifecycle intents: `cancelMode { immediate, end_of_term, at(date) }` + `resumeAt`, pending intents on the aggregate | **DECIDED (autonomous) 2026-07-15 · flagged for veto** |
| SUB-D-02 | M | `updateQuantity` as a first-class transition with the plan-change envelope; increases immediate, decreases next-cycle by default | **DECIDED (autonomous) 2026-07-15 · flagged for veto** |
| SUB-D-03 | M | Billing-only pause = `collectionPaused` posture on `active` (subscription attribute, collection-scoped) | **DECIDED (autonomous) 2026-07-15 · flagged for veto** |
| SUB-D-04 | M | Ramps: Contracts authors the committed schedule; Subscriptions executes generated scheduled intents; no native schedule aggregate | **DECIDED (autonomous) 2026-07-15 · flagged for veto** |
| SUB-D-05 | M | Activation date trio (booking / service / acceptance) as attributes + events; no new statuses | **DECIDED (autonomous) 2026-07-15 · flagged for veto** |
| SUB-D-06 | H | Ordering tenant pinned at creation: transfer rebinds commercial axes, never the ordering/partition key | **DECIDED (autonomous) 2026-07-15 · flagged for veto** |
| SUB-D-07 | H | Recurring split: Subscriptions emits a money-free period fact; rating prices the recurring line; Billing posts | **DECIDED (autonomous) 2026-07-15 · flagged for veto** |
| SUB-D-08 | M | Mutation-type inventory completed: `renew`, `unschedule`, `pauseCollection`/`resumeCollection`, `confirmAcceptance`, `extendTrial` | **DECIDED (autonomous) 2026-07-15 · flagged for veto** |
| SUB-D-09 | M | Secondary producer-event inventory named in design (intents, renewal/grace, notices, pause, quantity, conversion, quota) | **DECIDED (autonomous) 2026-07-15 · flagged for veto** |
| SUB-D-10 | M | Entitlement check surface: bounded-staleness degraded mode (last-known-good ≤ budget, then fail-closed) | **DECIDED (autonomous) 2026-07-15 · flagged for veto** |
| SUB-D-11 | M | `draft → cancelled` (void) edge added; draft is exitable without activation | **DECIDED (autonomous) 2026-07-15 · flagged for veto** |
| SUB-D-12 | M | `collectionPaused` suspends renewal payment pre-check + grace entry for in-window renewals (term extension continues) | **DECIDED (autonomous) 2026-07-15 · flagged for veto** |

## Decisions

#### SUB-D-01 [M] Scheduled lifecycle intents (cancel at term end / at date; scheduled resume)

- **Where**: [PRD](./PRD.md) §6.1 `fr-scheduled-intents`, §6.5 interaction, AC 21–22; source: [gap analysis](./STRIPE-ZUORA-GAP-ANALYSIS.md) G-1.
- **Problem**: `changeMode` existed only for `changePlan`; "cancel at period end" (the most common self-service intent; Stripe `cancel_at_period_end`, Zuora cancellation policies, the predecessor's own glossary "Cancellation Policy") and "suspend until date" were unrepresentable — portal-side cron hacks would be invisible to the renewal job, Billing, and audit.
- **Options**: (a) pending-intent envelope on the aggregate reusing the §6.3 mode vocabulary; (b) status quo (portal automation).
- **Decision**: **(a), 2026-07-15** — `cancelMode ∈ {immediate, end_of_term, at(date)}`, `resumeAt` on suspend; pending intents stored on the aggregate, suppress renewal + next-term recurring, un-schedulable until effective, evented both ways; the effective-instant transition runs the full §6.1 guard set. *Flagged for veto.* **Propagated**: PRD §6.1 FR + §5.1 row + glossary (Cancellation policy) + AC 21–22 + §15 manifest-envelope row; gap analysis G-1 → actioned.

#### SUB-D-02 [M] `updateQuantity` as a first-class transition

- **Where**: [PRD](./PRD.md) §6.3 `fr-update-quantity`, §6.1 type list, AC 23; source: gap analysis G-3.
- **Problem**: pricing D-18 (`quantitySource = subscription_seat_count`) makes this gear the seat-count supplier, but the transition inventory had no quantity operation — no Policy gate, no proration boundary, no provenance for the number rating reads (Stripe `items.quantity`, Zuora `UpdateProduct` both first-class).
- **Options**: (a) full transition with the §6.3 envelope; (b) attribute-with-audit only (next-cycle effect).
- **Decision**: **(a), 2026-07-15**, with the up/down asymmetry defaulting to (b)'s conservatism: **increases MAY be immediate** (prorated by rating at the boundary), **decreases default `next-cycle`**. Seat counts consumed by rating MUST originate from committed transitions only. Policy-gated when quota-bearing; composition-changing event under AC 11 rules. *Flagged for veto.* **Propagated**: PRD §6.3 FR + §6.1 type list + §5.1 row + AC 23 + §15 manifest-type row; gap analysis G-3 → actioned.

#### SUB-D-03 [M] Billing-only pause = `collectionPaused` posture on `active`

- **Where**: [PRD](./PRD.md) §6.4 `fr-collection-pause`, glossary (Subscription pause), AC 24; source: gap analysis G-4.
- **Problem**: suspension always touches service (entitlement freeze, OSS pause); the inverse — service running, collection paused (hardship/dispute/goodwill; Stripe `pause_collection`; the predecessor's "Subscription Pause" P1 scope) — was unrepresentable without abusing `suspended`.
- **Options**: (a) subscription **attribute posture** on `active` (rides subscription events and renewal logic; reuses the §6.5 recurring-pause mechanism); (b) Billing-side AR hold (outside the aggregate).
- **Decision**: **(a), 2026-07-15** — `collectionPaused` auditable window (start/end/limit/reason) on `active`; collection-scoped only (service, entitlements, renewal evaluation untouched unless a separate intent says otherwise); Billing chooses the artifact treatment per policy. Pause-day limits and resume-proration mechanics stay open (§15) with Product/Billing. *Flagged for veto — Finance may still prefer (b); the aggregate-posture shape keeps the audit trail on the subscription either way.* **Propagated**: PRD §6.4 FR + §5.1 row + glossary + AC 24 + §15 mechanics row; gap analysis G-4 → decided.

#### SUB-D-04 [M] Ramps are Contract-authored; Subscriptions executes scheduled intents

- **Where**: [PRD](./PRD.md) §6.3 `fr-ramp-execution`; source: gap analysis G-5.
- **Problem**: committed multi-step growth (Zuora Ramps/Orders, Stripe schedules) had no representation; a native schedule aggregate would duplicate Contracts' role as the home of negotiated commitments (cf. rating T-D-14 pools).
- **Options**: (a) Contracts authors the committed ramp, materialized as a sequence of scheduled `changePlan`/`updateQuantity` intents executed here; (b) native `SubscriptionSchedule` aggregate; (c) status quo (operator re-keys).
- **Decision**: **(a), 2026-07-15** — depends on the SUB-D-01/02 envelopes; atomic multi-action submission is a Contracts/Design follow-up (§15). (b) revisited only if self-service ramps become a product goal. *Flagged for veto.* **Propagated**: PRD §6.3 FR + §15 Contracts-follow-up row; gap analysis G-5 → decided.

#### SUB-D-05 [M] Activation date trio as attributes; no new statuses

- **Where**: [PRD](./PRD.md) §6.1 `fr-activation-instants`, AC 25; source: gap analysis G-6.
- **Problem**: `activate` was a single instant; Zuora's `ContractEffectiveDate` / `ServiceActivationDate` / `CustomerAcceptanceDate` distinction — which the ASC 606 hooks need for enterprise acceptance clauses — was collapsed.
- **Options**: (a) three instants as attributes/evaluated fields + an optional acceptance-confirmation operation; (b) manifest enum extension (pending-activation / pending-acceptance interim statuses); (c) purely Contracts/Finance-side dates.
- **Decision**: **(a)+(c) split, 2026-07-15** — `contractEffectiveAt` referenced from Contract (booking semantics stay Contracts/Finance SoR), `serviceActivatedAt` stamped at the activate commit, `customerAcceptedAt` stamped by an acceptance confirmation where Contract clauses require (else = service activation); all three ride lifecycle events and ASC hooks. (b) rejected — the manifest enum stays closed, `draft` covers pre-activation. Confirmation-flow shape → Design (§15). *Flagged for veto.* **Propagated**: PRD §6.1 FR + AC 25 + §15 acceptance-flow row; gap analysis G-6 → decided.

#### SUB-D-06 [H] Ordering tenant pinned at creation (transfer never rebinds the ordering key)

- **Where**: [PRD](./PRD.md) §6.7 `fr-event-ordering`, AC 26; design slices 01 §4.2, 07 §4.4; seam SUB-R1.
- **Problem**: the ordering/partition key is `(resourceTenantId, subscriptionId)` and storage is partitioned by `resourceTenantId`; an ownership transfer that rebinds `resourceTenantId` would switch the key mid-stream — pre/post-transfer events land in different partitions, rating's ordered consumption breaks exactly at the most sensitive moment, and the aggregate row set (revisions, intervals, entitlements, audit) would need a physical partition migration. The design review (2026-07-15) flagged this as its top finding.
- **Options**: (a) pin an immutable `orderingTenantId` at creation (= `resourceTenantId` at creation) used for ordering + partitioning; transfer rebinds the commercial axes only; (b) ordering-epoch/barrier protocol that re-keys the stream transactionally; (c) forbid `resourceTenantId` transfer (cancel+new only).
- **Decision**: **(a), 2026-07-15** — `orderingTenantId` is stamped at creation and immutable; the manifest ordering invariant reads onto it; `OwnershipTransferCompleted` carries both old and new axes **on the same partition**, so consumers re-key their own projections without a stream barrier. (b) rejected as a heavy protocol with no consumer that needs it; (c) rejected — transfer is a manifest §4.11 flow. *Flagged for veto.* **Propagated**: PRD §6.7 FR + AC 26; slice 01 §4.2, slice 07 §4.1/§4.4; SEAMS SUB-R1 note + ownership matrix.

#### SUB-D-07 [H] Recurring split: Subscriptions emits the period fact, rating prices it

- **Where**: [PRD](./PRD.md) §6.8 `fr-recurring-idempotency`, §9.2 billing handoff + rating read-model, §4 diagram, AC 27; design slice 08 §4.3; seams SUB-B1, SUB-R1.
- **Problem**: slice 08's `RecurringEmitter` sends `BillableItemCreated(kind=recurring)` straight to Billing, but this gear computes no money — while the rating PRD evaluates recurring components itself (flat per-period, `per_unit = unitPrice × seats`, hybrid emits recurring + usage as two lines). Two candidate producers of the recurring line ⇒ double-emission or an amount-less item Billing cannot post; no seam pinned the split.
- **Options**: (a) Subscriptions emits a **money-free recurring period fact** (period identity + traceability tuple + `pricingSnapshotRef`); rating consumes it as the period trigger, evaluates the recurring amount from the frozen snapshot, and emits the priced line to Billing; (b) Subscriptions prices recurring itself (violates WHEN/MATH); (c) rating self-triggers recurring off its own calendar (duplicates the anchor/pause/intent logic that lives here).
- **Decision**: **(a), 2026-07-15** — the WHEN/MATH split extended to recurring: this gear owns the period cut (anchor, pauses, pending intents, idempotency key `(subscriptionId, billing period)`); rating owns the price. The fact carries no monetary column; the priced line inherits the fact's idempotency key. *Flagged for veto — needs the rating counterpart contract updated (joint fixture).* **Propagated**: PRD §6.8 FR + §9.2 contracts + §4 diagram + AC 27; slice 08 §4.3; SEAMS SUB-B1/SUB-R1 + ownership matrix.

#### SUB-D-08 [M] Mutation-type inventory completed

- **Where**: [PRD](./PRD.md) §6.1 `fr-transition-request`, §9.1 table; design slices 01 §3.1/§4.2, 09 §4.1; seam SUB-N1.
- **Problem**: the PRD's own FRs mandate mutations that had no `TransitionRequest.type`: manual **renew** (§6.5 "requires an explicit TransitionRequest"), **un-scheduling** a pending intent (SUB-D-01, AC 22), the `collectionPaused` set/clear (SUB-D-03), **acceptance confirmation** (SUB-D-05), **trial extension** (§6.10) — breaking the "single commit path, all mutations are TransitionRequests" principle; §9.1 also lagged behind `updateQuantity`/`convertTrial`/`transfer`.
- **Options**: (a) extend the type list with `renew`, `unschedule`, `pauseCollection`, `resumeCollection`, `confirmAcceptance`, `extendTrial` (same pending-manifest-alignment posture as `updateQuantity`/`convertTrial`); (b) model them as untyped admin endpoints outside the envelope.
- **Decision**: **(a), 2026-07-15** — every mutation gets a type; `extendTrial` is approval-gated (high-risk pattern), `unschedule` references the pending intent it voids. (b) rejected — it recreates the side door the Foundation exists to close. *Flagged for veto.* **Propagated**: PRD §6.1 type list + §9.1 table + §15 manifest row; slices 01/09; SEAMS SUB-N1.

#### SUB-D-09 [M] Secondary producer-event inventory named in design

- **Where**: design slice 08 §4.1 (naming registry); [PRD](./PRD.md) §6.7 pointer; ACs 7/14/17/19/22/23/24.
- **Problem**: the §6.7 producer inventory repeats the manifest baseline, but PRD FRs mandate auditable events with no names anywhere: intent schedule/un-schedule (AC 22), renewal outcome + grace entry/exit (AC 7), notice triggers (AC 19), pause window (AC 24), the quantity composition event (AC 23, "naming per Design"), the first-class conversion event (AC 17), quota warning/exhausted/restored (AC 14). Slice 08 deferred the field matrix to slice 09, slice 09 deferred to "Design" — a circular deferral with no owner.
- **Options**: (a) slice 08 owns the full naming registry + required-context groups; slice 09 owns only wire mappings; (b) keep deferring to implementation PRs.
- **Decision**: **(a), 2026-07-15** — the secondary set is named normatively in slice 08 §4.1 (`SubscriptionIntentScheduled`/`…Unscheduled`, `SubscriptionRenewalSucceeded`/`…Failed`, `SubscriptionGraceEntered`/`…Exited`, `SubscriptionRenewalNoticeDue`, `SubscriptionCollectionPaused`/`…Resumed`, `SubscriptionQuantityChanged`, `SubscriptionTrialConverted`/`…Extended`/`…Expired`, `SubscriptionAcceptanceConfirmed`, `EntitlementQuotaWarning`/`…Exhausted`/`…Restored`). *Flagged for veto.* **Propagated**: slice 08 §3.3/§4.1; slice 09 §3.3; PRD §6.7 pointer sentence.

#### SUB-D-10 [M] Check surface: bounded-staleness degraded mode

- **Where**: design slice 05 §3.8/§4.3; [PRD](./PRD.md) §15 row; seam SUB-E3.
- **Problem**: slice 05 declared the point-of-use check "fails closed on read-model outage (never stale-allow)" while also being cache-first with a < 5s propagation baseline — caching *is* bounded staleness, so the posture was self-contradictory; and a hard fail-closed read path means one projection outage blocks every entitlement check platform-wide (availability catastrophe for a runtime read).
- **Options**: (a) explicit staleness budget: serve last-known-good decisions up to a bounded age (default 60s) on projection outage, then fail closed; transitions stay strictly fail-closed; (b) keep binary fail-closed; (c) fail-open indefinitely.
- **Decision**: **(a), 2026-07-15** — the budget makes the normal-mode cache and the degraded mode one coherent rule; the 60s default is a Product/OSS knob (§15). (b) rejected as an availability landmine; (c) rejected as a silent-overrun door. *Flagged for veto.* **Propagated**: slice 05 §3.8/§4.3; slice 09 §3.8 scoping (only the check surface carries this posture); PRD §15 budget row.

#### SUB-D-11 [M] `draft → cancelled` (void) edge

- **Where**: [PRD](./PRD.md) §6.1 status machine + transitions table, AC 28; design slice 01 §4.1.
- **Problem**: the state machine had no exit from `draft` — abandoned drafts (incl. never-activated trials, whose expiry path is `cancel`) were immortal, and the single-commit-path principle forbids deleting them out-of-band.
- **Options**: (a) add `draft → cancelled` (void; immediate-only, not resource-affecting — nothing is provisioned, no OSS leg); (b) hard-delete drafts by retention job; (c) leave drafts forever.
- **Decision**: **(a), 2026-07-15** — void is a normal `cancel` from `draft` with the standard envelope and audit; an optional draft-retention TTL that *submits* the void is a Product knob (§15). Same manifest posture as `resume`: the manifest lists operations, not every arrow. *Flagged for veto.* **Propagated**: PRD §6.1 enum FR + table + diagram + AC 28 + §15 TTL row; slice 01 §4.1.

#### SUB-D-12 [M] `collectionPaused` suspends renewal collection, not renewal evaluation

- **Where**: [PRD](./PRD.md) §6.4 `fr-collection-pause`, AC 29; design slice 04 §4.2/§4.3; seam SUB-B2.
- **Problem**: SUB-D-03 left "renewal evaluation continues" during a pause — so an auto-renewal inside a hardship/dispute pause would run its payment pre-check, fail, enter grace, and suspend the customer: the collection-only pause would cause the exact service loss it exists to avoid.
- **Options**: (a) renewal evaluation and term extension continue, but the payment pre-check, grace entry, and dunning handoff are **suspended** for renewals whose collection falls inside the pause window — they run when the window ends (deferred collection per the Billing artifact treatment); (b) block renewal entirely during pause; (c) keep pre-check + grace running (status quo).
- **Decision**: **(a), 2026-07-15** — mirrors Stripe `pause_collection` semantics (service and term continuity, collection deferred); term extension stays deterministic for rating/Billing. (b) rejected — an expiring term during pause would kill service; (c) rejected as self-defeating. *Flagged for veto.* **Propagated**: PRD §6.4 FR + AC 29; slice 04 §4.2/§4.3; SEAMS SUB-B2 note.
