<!-- Related: ../DESIGN.md, ../DECISIONS.md, ../SEAMS.md, ../REVIEW.md, ../design/ | Owners: BSS Subscriptions team -->

# Subscriptions — design-set review wave 3: billing-domain + cross-gear + feature interactions (2026-08-01)

**Scope**: [`gears/bss/subscriptions/docs/`](../) (PRD, DESIGN, DECISIONS, SEAMS, REVIEW, ADR,
`design/01…09`) after the 2026-07-28 billing pass and the 2026-07-29/30/31 fix rounds, checked
against the current neighbour gears (`gears/bss/pricing/docs/`, `gears/bss/rating/docs/`) —
pricing moved through decision waves D-57…D-122 since this gear's seam map was last touched, so
the **neighbour documents are the oracle** rather than this gear's own register.

Findings from the 2026-07-23 and 2026-07-28 waves are seeded in [`../REVIEW.md`](../REVIEW.md)
and not re-reported. Emphasis: **SUB-D-13…19** and the SUB-D-07/12/14 amendments.

Two prerequisites were run before the hunt. The repo's own design-set checker
(`python3 .claude/skills/spec-check/scripts/check.py --gear gears/bss/subscriptions/docs
--gear gears/bss/pricing/docs --gear gears/bss/rating/docs`) — results in item 24. The repo
guideline set (`guidelines/README.md` → DNA/languages/RUST.md, DNA/REST/*, SECURITY.md,
DEPENDENCIES.md) and the Rust/REST/approved-crates skills are code-scoped, so no prose-doc
guideline applies and there is no separate Guideline-compliance group.

Every citation below was opened and quoted first-hand. Two candidate findings arrived with
supporting quotes that do not exist in the documents (a fabricated slice-06 "no prior term"
sentence and a pricing `inst-el-msw` phrase); both survive here only because the underlying gap
was re-grounded on lines that do exist — see items 11 and 12.

## Summary

**24 items. No Blocking item**: both candidates that looked Blocking during the hunt were cut
down in the Challenge pass — the recurring period-coordinate collision is already tracked at
CRIT on the rating side (rating `SEAMS.md` SB1) and disclosed here as REVIEW.md F-08-3, and the
"one-time charge forces this gear to compute money" reading is refuted by the gear's own
wording, which permits resolving published price facts. Five candidates were retracted or
downgraded; they are listed at the end with the lines that killed them.

The dominant cluster is the 2026-07-28 fixes interacting with each other. §4.3b (the
post-suspension backdated revival) and SUB-D-16 (the 90-day dwell) were written independently,
and together they push a period cut, a payer snapshot, an eligibility re-resolution and a quota
reset up to 90 days past the period they describe — none of which has an as-of rule.
Separately, SUB-D-19's per-component key re-opened three rules that were written when one fact
per period existed, and SUB-D-18's ETF path is blind to the cancellation reason it is handed.

---

## High

1. **Pin an as-of instant for the §4.3b revival cut — composition and payer both drift**
   (design/04:239, design/04:246, design/08:136, design/08:243). §4.3b re-runs the
   never-succeeded renewal on the payment-resolved signal and backdates the term to the old
   term end; §4.4 then emits the once-blocked fact "with its original (subscriptionId, billing
   period, lineKey) key". But `lineKey` is "the billable component within the subscription's
   effective composition **at the cut**", and `payerTenantId` is "snapshotted **at cut time**"
   — and with SUB-D-16 the cut can now run up to 90 days after the period it describes. Two
   revenue consequences, one missing rule. (a) Composition: an `AddOn` interval that ended
   inside the blocked period is gone from the composition at the revival cut, so that
   component's fact is never cut, never priced, never posted — read the other way (as-of the
   backdated instant) the same code bills add-ons that had already ended. (b) Payer: transfer
   is allowed from `suspended` once the ladder is no longer running (design/01:327), so a
   transfer during the suspension makes the revival snapshot the **new** payer for a period
   consumed under the old one — defeating the exact purpose the field was added for ("Billing
   posts the period to the fact's frozen payer, never to a payer re-resolved from the aggregate
   at posting time") and contradicting slice 07's in-flight-period-stays-with-the-old-payer
   default. State that a revival cut resolves composition and payer **as of the period**, not
   as of the commit.

2. **Give an immediate mid-period composition change a recurring trigger** (design/03:236,
   design/08:242). SUB-D-02 allows increases to be immediate and slice 03 states the billing
   effect as "Immediate ⇒ delta recurring / one-time true-up (Billing)". After SUB-D-07/19 the
   period fact is the **only** recurring trigger rating has, the emitter is a daily cut keyed
   per `(subscriptionId, billing period, lineKey)`, and Billing computes no recurring price. So
   for an add-on added — or a seat count raised — on day 10 of a monthly period, no rule cuts a
   partial-period fact for the new or changed component and no gear is positioned to produce
   the delta: the two rules never meet. Name the trigger (a mid-period cut for the affected
   `lineKey`, or an explicit statement that the delta rides the next boundary and the
   intervening days are not charged).

3. **Define the dwell deadline's lifecycle — nothing clears it, re-resolves it, or offers
   relief** (design/04:247, design/04:217, design/01:327-329). SUB-D-16 stores the dwell as an
   evaluated deadline that "never moves", with "payment inside it revives per §4.3b" as the
   only counter-rule. Three holes follow. (a) No clearing condition: if the customer pays but
   the resume fails closed on an overlap acquired during the suspension (§4.1, "remediation is
   operator-driven"), or an operator uses the audited override to resume early, the deadline
   still stands — so the system fires `cancel (nonpayment_exhausted)` against a **paid or
   already-active** subscription, and SUB-D-18 then hands Billing an ETF trigger for it.
   (b) Not re-resolved on a payer rebind: transfer is allowed from `suspended` once the ladder
   has exhausted, and the deadline resolved from the old payer's ladder travels unchanged to a
   payer whose own Contract ladder differs — while the rule forbids moving it. (c) No relief
   path: `pauseCollection`'s allowed source status is `active`, so a dispute raised after a
   nonpayment suspension — the normal order, since the dispute follows the service loss —
   cannot open a window, and §4.2's ladder freeze needs a ladder that is still running. Say
   what clears the deadline, what happens to it on transfer, and whether a post-suspension
   dispute can hold it.

4. **Make the dwell's terminal cancel able to commit** (design/04:247, design/01:250,
   design/04:217). The dwell fires "an end-of-term-style cancel … through the commit path".
   `cancel` with a deprovision leg is an OSS-blocking edge whose status edge commits only on
   OSS confirmation, and at suspension OSS already deprovisioned or paused the resources.
   Nothing makes the deprovision leg a no-op when there is nothing to deprovision, so the
   firing can time out `oss_unconfirmed`, go retryable, exhaust its deadline and land terminal
   — committing no status change and no revision. The subscription stays `suspended`, and since
   the deadline "never moves" nothing re-arms the step: the immortal-suspended state SUB-D-16
   exists to kill returns, now with an alarm instead of a fix.

5. **Resolve the "parks until resume" promise against the firing-failure taxonomy's deadline**
   (design/04:228, design/01:348-355, DECISIONS.md:127). SUB-D-12's second amendment says the
   window-end `resumeCollection` firing against a `suspended` aggregate "parks until resume …
   or a terminal state", and the convertTrial matrix row says the same (design/01:326). The
   taxonomy it delegates to has two classes, and retryable is **bounded** — retries run "until
   a firing deadline (`effectiveAt` + a configurable grace horizon…)" and "On deadline
   exhaustion the failure escalates to terminal". There is no state-precondition exemption, no
   third class, and nothing re-arms a parked intent on resume; the only re-arm text is the
   opposite (design/04:235). A nonpayment suspension may last the 90-day dwell, so for any
   suspension outlasting the horizon the promise cannot hold. Not blocking: exhaustion always
   raises a dead-letter alarm, and §4.2a's terminal branch still closes the window and runs the
   deferred collection. Either add the parking class (deadline suspended while a named state
   precondition blocks the firing, re-armed on resume) or state that the window closes only by
   operator action or the terminal path. Also give the grace-horizon knob a PRD §15 row — it
   cites "§15" and no row exists.

6. **Pin who resolves the one-time / setup amount, then give the charge its missing surfaces**
   (design/08:247, PRD.md:1052). The T-D-18 adoption states a p1 MUST — "this gear MUST emit
   the one-time billable at the qualifying instant … valued from the frozen pricingSnapshotRef
   amount" — with a "once-per-subscription-lifetime dedup … keyed per (subscriptionId,
   priceId)". None of the surfaces that make it implementable exist: (a) the §4.1 registry
   names only `BillableItemCreated(kind=recurring)` (design/08:209) and the secondary set has
   no one-time row, so there is no field matrix; (b) §3.2 has only `RecurringEmitter`
   (design/08:145); (c) §3.7 declares the recurring unique index and then "No separate owned
   store" (design/08:188), so the lifetime dedup key has no table; (d) the Billing handoff
   contract is recurring-only (design/09:219) while slice 08 assigns the wire payload to slice
   09; (e) SEAMS.md has no one-time/setup row at all. Also unpinned: rating hands the charge to
   "Subscriptions/Billing" as a pair (rating DECISIONS:43) and neither gear splits it, so it is
   undecided whether this gear emits an amount or an amount-less trigger Billing values from
   the ref. The adoption has no SUB-D-xx record either — T-D-18 appears only at the two sites
   above, both "flagged for veto". The trials slice already assumes the charge exists ("the
   failed conversion charge enters the §6.5 grace ladder", design/06:189) without naming an
   emitter.

7. **Re-scope AC 5 and the traceability rule to the per-component key** (PRD.md:1204,
   PRD.md:848, design/08:255, PRD.md:379, STRIPE-ZUORA-GAP-ANALYSIS.md:116). SUB-D-19's body
   says "Idempotency, the §3.7 unique index, and AC 5 all re-scope to the three-part key" and
   its Propagated list names AC 5 and the traceability row (DECISIONS.md:176). Neither
   happened. AC 5 still reads "at most one recurring BillableItem per (subscriptionId, period)
   MUST be posted" while AC 27 carries the three-part key (PRD.md:1338) — two contradictory
   acceptance criteria for the same invariant, and AC 5 is the testable one: enforced as
   written, the uniqueness constraint rejects or collapses the add-on component's fact, which
   is exactly the F-08-1 defect SUB-D-19 exists to fix. Both normative traceability statements
   still describe a single tuple ("Items MUST trace to subscriptionId, skuId/planId/priceId,
   and pricingSnapshotRef") with no per-component dimension, so an implementer stamps the
   plan's catalog keys on add-on lines. The §5.1 scope row and the vendor-gap row carry the
   same stale two-part key.

8. **Define `lineKey` — value, occurrence dimension, and stability** (design/08:136,
   design/02:182, rating SEAMS:165). This gear defines the coordinate descriptively — "the
   billable component within the subscription's effective composition at the cut: the plan
   line and each AddOn line" — then asserts it "is the same coordinate rating carries". Rating
   never defines `lineKey` anywhere: it appears only inside the T-D-15 tuple and the SB1
   conflict notes, and SB1 treats the field as settled, scoping the open work to the other
   coordinate ("the two keys now differ only in the period coordinate"). So the joint fixture
   both gears wait on would be written against an unspecified field. It also has no occurrence
   dimension: slice 02's exclusion key forbids only the same add-on self-overlapping, so
   removing an add-on on day 3 and re-adding it on day 20 of one period is legal and both
   intervals map to one `(subscriptionId, billing period, lineKey)` — the second fact hits the
   unique index and the re-subscribed stretch is never billed. Name the identifier (the
   plan-line sentinel plus `addOnId`), add the occurrence/interval dimension, and state the
   behaviour across an in-place `changePlan`, an `updateQuantity`, and a cancel+new handover.

9. **Make the quota-cycle reset idempotent per period** (design/05:238, design/08:179,
   design/08:246). "Counters reset at the billing-anchor cycle boundary — triggered by the
   recurring period cut (slice 08), not by a separate clock" was written when one fact per
   period existed. SUB-D-19 makes the emitter cut "one recurring BillableItem per billable
   component", and §4.3 adds a targeted re-cut after a post-cut unschedule. Idempotency is
   defined on the fact key, not on the reset, so a second component's cut inside the same
   period — or the re-cut — fires the reset again and hands the customer a fresh cycle
   allowance mid-period. Key the reset to the period, not to a cut.

10. **Make SUB-D-18's ETF path reason-aware and give Billing a join key** (design/08:249,
    design/08:209, DECISIONS.md:168). `SubscriptionCancelled` carries "the cancellation
    instant, cancelMode, reason, and the contract ref", and Billing derives the ETF/credit
    artifacts from the Contract terms. The reason enum includes `term_expired`,
    `nonpayment_exhausted` and `saga_superseded` — a term that ran to its natural end has no
    early termination to charge, involuntary nonpayment churn is a collections/write-off case
    in normal practice rather than an ETF, and a saga-superseded predecessor cancel is a plan
    change. No site says which reasons are ETF-eligible, so the platform can terminate a
    customer for nonpayment and then bill them a termination fee on top of the deferred
    collection §4.2a still runs. Separately the payload carries no join key: no period
    identity, no `lineKey` set, no term window — so for "credit the unused portion" Billing
    must re-derive the period from the aggregate at ETF time, the exact posting-time
    re-resolution the design eliminated for the payer axis, against a term a §4.3b extension
    may since have moved.

11. **Define the revival path for a suspension caused by a failed trial conversion**
    (design/06:189, design/04:239). A failed conversion charge "enters the §6.5 grace ladder
    as its blocked collection; grace failure exits to suspended/cancelled", so a trial
    customer can reach nonpayment `suspended` without ever having had a renewal. §4.3b's
    revival is defined only as re-running "the never-succeeded renewal — the same idempotent
    term extension keyed (subscriptionId, currentTermSequence)", with the ordering "payment
    resolution → backdated term extension → resume" and "a resume never precedes the term that
    covers it". For a first conversion there is no prior term to extend, so the mandated
    middle step has no object and the resume precondition cannot be satisfied — the customer
    pays and service does not return, then the dwell cancels them. *(This finding arrived with
    a supporting quote that does not exist in slice 06; it is re-grounded on the two lines
    above.)*

12. **State the grandfathering consequence of cancel+new** (design/03:252, design/04:236).
    SUB-D-14 went to length to carry priceEligibility + cohort forward across renewals because
    "the pinned price id is the cohort's only carrier". Cross-currency, cross-region and
    cross-frequency changes are executed as cancel+new, which mints a new subscription with a
    fresh purchase gate and a re-frozen `(currency, region)` segment — and pricing selects the
    grandfathered row from "the cohort of the subscription's pinned price id" within the
    `existing_grandfathered` class, ahead of `new_subscriptions_only` and `all_subscriptions`
    (pricing design/07:236). A protected customer accepting a monthly→annual upsell therefore
    loses price protection, silently. Slice 05 records the quota-continuity consequence of the
    same handover and slice 02 the segment re-freeze; nothing records this one. Say whether
    the cohort is intended to carry across a `supersedesSubscriptionId` pair, and if not, that
    the loss must be disclosed before execution (the slice already requires that for credit
    forfeiture). *(The supporting `inst-el-msw` quote this arrived with does not exist;
    re-grounded on the eligibility-class ordering above.)*

13. **Bound the §4.3b revival against pricing's grandfathering margin** (design/04:239,
    design/04:236, design/04:247). The revival "re-runs the never-succeeded renewal", and
    every renewal re-resolves the snapshot refs eligibility-first — with no as-of instant
    stated for the re-resolution (item 1's problem, on the pricing axis). §4.3 justifies the
    design by pricing's window sizing: "grandfatherUntil + one full cycle margin, because
    re-bind happens at the next renewal after expiry". A dwell of up to 90 days lets the
    revival's renewal run far outside that margin, so a backdated period can be re-resolved to
    a successor row (a retroactive increase on an already-contracted period) or to no covered
    row at all. Either resolve the revival's eligibility as of the backdated term start, or
    reconcile the dwell length with pricing's margin.

14. **Void the future composition intervals a terminal commit leaves behind** (design/01:343,
    design/03:181, design/02:201). A scheduled next-cycle or end-of-term change persists a
    `ScheduledIntent` "and write[s] the future interval". The terminal sweep "atomically voids
    every pending ScheduledIntent of the aggregate" — and says nothing about those intervals,
    while slice 02 permits voiding an unreached interval only "by the owning
    unschedule/superseding change", which a terminal sweep is neither. So after an immediate
    cancel the un-reached `PlanLink` interval survives and opens on its `effectiveFrom` inside
    the composition read model that is rating's input contract — a new plan interval on a
    cancelled subscription. SUB-D-15 explicitly pairs step-voiding with the slice-02
    future-interval rule; the sweep does not.

15. **Give the period cut a status precondition** (design/08:246, design/08:249). The design
    pins the disposition for a cancel landing inside an already-cut period (SUB-D-18: the fact
    stands) and for an unschedule committed after the cut (a targeted re-cut). It never pins
    the reverse order: a terminal state reached **before** that period's daily cut runs.
    Nothing says the emitter skips a now-cancelled subscription and nothing says it cuts a
    clipped partial-period fact, so the cut either emits a full-period fact for a period never
    served — which Billing posts, and then derives ETF/credit on top — or silently skips it
    and the served days are never billed. The same ambiguity governs §4.2a's deferred
    collection of a window closed by a terminal commit.

16. **State whether period facts are cut during a trial phase, and the first paid period key
    after conversion** (design/08:243, design/08:245, design/06:239). Period identity comes
    "from the billing anchor" and the daily cut finds the plan component in force during a
    trial phase as much as after it. If facts are emitted, rating prices a phase the customer
    was promised free (or finds no row); if they are suppressed, a mid-period `convertTrial` —
    which advances the phase boundary to now — leaves the current period with no fact, so the
    first paid stretch is never rated. Period-key stability names only a cycle-length change
    and cancel+new as new-sequence triggers, so whether conversion opens a period is
    undefined. The same slice pins the qualifying instants for the one-time charge
    ("subscription activation, or trial conversion") and never for the recurring fact.

17. **Carry pricing D-80 and D-94 into the PRD and the slices** (SEAMS.md:64). SUB-P5 records
    two additions to the adopted sellability gate: D-80 extends predicate (1) with a coverage
    horizon ("the key's active-plus-scheduled coverage must reach now + the longest billing
    cycle sold on the key") and D-94 pins the granularity — "→ Obligation here: the
    create/changePlan guard evaluates the full conjunction over every scope key the purchase
    binds, not a single key". That line is the only carrier in the doc set: coverage horizon,
    longest billing cycle, conjunction and `not_sellable_ga` return zero hits in PRD.md and in
    all nine slices, and the §4.5 gate section covers only Policy and OSS
    (design/01:374-380). Unlike SUB-P7/P8 this obligation carries no "before design freeze"
    qualifier and SUB-P5 is absent from the joint-obligations checklist, so nothing tracks it;
    an implementer writes the single-key check. *(The base gate's absence from the slices is
    already tracked as REVIEW.md F-02-1, open — this is the D-80/D-94 increment on top.)*

18. **Give SUB-P6/P7/P8/B7 a slice owner and a PRD surface** (DESIGN.md:126-134,
    SEAMS.md:66-67, SEAMS.md:171, SEAMS.md:221). SEAMS states the contract: "each slice
    implements the Subscriptions side of the seams listed for it in DESIGN.md §1.3" — and the
    §1.3 slice map (mirrored in design/README.md) lists none of the four seams added in the
    2026-07-28/29/30 rounds, so formally nobody implements them. SUB-P7 (the migration
    :start/:complete executor handshake) and SUB-P8 (the in-flight-subscriber presence read,
    an inbound surface this gear must expose) each state "→ Obligation here: author … before
    design freeze" and appear in no slice body and in no PRD §9.1 operation or §9.2 contract.
    The ownership matrix has no row for any of them either — while slice 08 cites one
    explicitly: "ownership row SEAMS SUB-B7" (design/08:249) points at a row that does not
    exist.

19. **Fix the two decision surfaces that still state the pre-fix rule** (PRD.md:735,
    DECISIONS.md:136). (a) PRD §6.5 grace policy rule 2 still reads "MUST NOT be emitted until
    renewal succeeds **or grace resolves to failure**" — the exact wording slice 04 §4.4
    replaced on 2026-07-28 because it "literally licensed emitting a fact for a term that
    never started". The PRD is the requirement source, so the two documents differ by a full
    period of revenue. (b) SUB-D-14's body heading still announces the pre-amendment,
    veto-flagged rule — "Every successful renewal refreshes pricing-side snapshot refs" — the
    rule the wave-2 Blocking finding showed strips price protection from every grandfathered
    cohort. The board row, the body text and every propagation site say eligibility-first;
    only the heading a reader scans still says the opposite.

## Cleanup

20. **Adopt `billingAnchorPolicy` and the D-20 month-end clamp, and record the joint anchor
    fixture** (PRD.md:114, design/08:245, SEAMS.md:64-67). Pricing requires every recurring
    row to publish `billingAnchorPolicy` (pricing design/06:317) with the K2 enum and the
    no-drift clamp, and D-20 assigns execution here: "confirm with Subscriptions (they execute
    the math)", "rides the joint proration/anchor fixture with Subscriptions" (pricing
    DECISIONS:329); K5 adds that the joint fixture "exists before code". This gear adopts the
    sibling field in full (`prorationBasis` has a glossary row, a verbatim-adoption statement
    and a CI drift gate, PRD.md:130) but has no anchor counterpart: `billingAnchorPolicy`
    appears only in an ADR-0002 driver bullet and one vendor-gap row, no clamp semantics
    appear anywhere, and neither SEAMS nor PRD §15 carries an anchor seam or fixture row —
    while `billingAnchor` is attributed to Contract terms, not the catalog (design/01:184).
    Rating also states a rule with no counterpart here: "a plan change that alters
    billingAnchorPolicy takes effect from the next period boundary — asserted in the joint
    anchor fixture; period identity itself stays Billing/Subscriptions'" (rating
    design/09:298); slice 03 says nothing about an anchor moving. Ranked Cleanup because the
    consequential half — who owns the period coordinate — is already tracked at CRIT in SB1;
    it escalates to High if SB1 resolves in this gear's favour, since this gear would then
    implement a clamp it has never written down.

21. **Stop presenting the recurring seam as aligned, and name an owner for rating's period
    tick** (SEAMS.md:93, SEAMS.md:198, DECISIONS.md:91). SUB-B1 is marked "ALIGNED
    (SUB-authors)" and repeated in the "Aligned (counterpart written; no action beyond
    citing)" list, for the same key the counterpart gear files as CRIT | Joint — OPEN: "two
    owners of the recurring WHEN, two idempotency keys, no consumption contract between them …
    resolve before either gear implements billingAnchorPolicy" (rating SEAMS:165). SUB-R6 does
    carry it as Joint/HIGH and REVIEW.md F-08-3 discloses it, so the substance is not hidden —
    but the severity sits a tier below the neighbour's and the ALIGNED label points a veto
    reviewer the wrong way. The same list also names SUB-P3, whose own verdict two pages up is
    "OPEN (2026-07-28 cross-gear review — was ALIGNED)". Separately, SUB-D-07 rejected option
    (c) — "rating self-triggers recurring off its own calendar" — which is rating's live
    normative design (rating design/14:264); no register on either side records who retires or
    subordinates that tick.

22. **Update slice 03 §4.3 to the amended SUB-P1** (design/03:251 vs SEAMS.md:60). The slice
    still says "The pricing consumer contract publishes allowedChangeTargets /
    comparabilityRank / the boundary class", while SUB-P1 was amended 2026-07-31 for pricing
    D-93: "the boundary class … is no longer a published stamp — this gear computes it at
    change time from both plans' published market/frequency facts at its pinned version". The
    implementation surface points at a field pricing no longer publishes, and the read-time
    rule it must implement instead appears in no slice.

23. **Give the SUB-D-17 price-change flag a carrier, and cite the row instead of the signal**
    (design/04:255, design/08:218, design/09:213, SEAMS.md:65, PRD.md:1418). §4.5 has the
    notice job read the flag "straight from the pricing read model (scheduled PriceWindows +
    EligibilityExpirySignal, SUB-P6)" and arm the 30-day notice. Pricing derives that signal
    at read time as `now ≥ grandfatherUntil`, "never stored, no job/event" (pricing DECISIONS
    R-14:1079, pricing design/07:239) — true only at or after expiry, so it can never arm a
    notice 30 days ahead; the computable input is the bound generation's `grandfatherUntil`
    date against the renewal instant. The flag also has no carrier anywhere: the
    `SubscriptionRenewalNoticeDue` registry row lists only the trigger, slice 09's read-model
    contract names the expiry signal but not the scheduled-supersession lookahead, the SUB-P6
    seam text the decision cites as propagated covers only the re-bind gate and the outbound
    feedback, and the PRD §16 risk row the decision also claims still mitigates with "Contract
    templates MUST define notice/opt-out behavior" alone.

24. **Close the register and mechanism gaps left by the 2026-07-28 round.** Grouped because
    each is a short edit. **(a)** SEAMS: the preamble says the decisions are "SUB-D-01…18" and
    the register ends at SUB-D-18 — SUB-D-19 has no row and is one of only three [H] decisions
    — while the propagation-status paragraph already claims "SUB-D-01…19 … propagated"
    (SEAMS.md:28, SEAMS.md:195, SEAMS.md:217); the joint-obligations checklist omits
    SUB-P6/P7/P8/B7 (SEAMS.md:200-212). **(b)** DESIGN.md still describes the set as
    "SUB-D-01…12" in both the §4 summary and §5 traceability (DESIGN.md:446), lists the
    adopted pricing contracts as SUB-P1/P2/P3 only, and its renewal sequence carries the
    SUB-D-14 amendment but no term-expiry branch, revival, dwell or notice arming.
    **(c)** Slice §5 traceability: slice 04 lists only SUB-D-03 among decisions although it
    now implements SUB-D-12/13/14/16/17; slice 08 has no Decisions bullet at all despite being
    the propagation target of SUB-D-07/09/18/19. **(d)** PRD: the §6.8 fact-field list and
    §9.2 handoff omit the two SUB-D-07 amendment fields (cut-time payerTenantId, suspended
    intervals + posture), §6.4 fr-suspension-billing-posture still stops at "attributes +
    contract clauses", and no acceptance criterion exists for SUB-D-13/16/17/18 although every
    comparable earlier decision got one. **(e)** Slice 04 mechanism: §3.1's domain model lists
    no dwell deadline field and the §3.6 sequence has no dwell branch, unlike §4.2a which
    derives an explicit timer. **(f)** Slice 03 §4.5 still emits "an auditable failure event"
    anonymously although slice 08 claims to have named it `SubscriptionRampHalted`.
    **(g)** `SeatBound`/`SeatReleased` exist only in the slice-08 registry — slice 05, which
    owns bindSeat/releaseSeat, never names them, and PRD §6.7 does not list them.
    **(h)** spec-check: SUB-D-15's propagation citation "names nothing the resolver
    recognises, so the claim was not verified at all" and uses a §4.x placeholder; SUB-D-16's
    target "SEAMS" "names no document the resolver can map"; and no slice uses a recognised
    traceability convention, so "47 requirement(s) went unchecked" — the P2 invariant silently
    does not cover this gear. **(i)** Pointers: the renew-from-suspended matrix row cites
    slice 04 §4.3a (the autoRenew = false term-expiry cancel) where it means §4.3b, and
    SUB-D-13's Propagated list repeats the mis-pointer; SUB-D-18's "Where" cites slice 01 §4.3
    for the `SubscriptionCancelled` payload, which is the scheduled-intents section.
    **(j)** REVIEW.md: the tally claims 15 fixed where only 13 carry `Status: fixed`, and
    F-05-2 (quota reset under a pause) is still open although the 2026-07-28
    emission-vs-collection precedence answered it — the fact is emitted during a pause, so the
    cut and the reset fire and only collection defers.

## Minor / hardening (latent, not live)

- **Bound `draft` the way SUB-D-13 and SUB-D-16 bound `active` and `suspended`**
  (DECISIONS.md:120). Both decisions ship a platform default on the argument that an immortal
  state is "the exact term ambiguity the slice exists to kill". `draft` keeps only "an optional
  draft-retention TTL that submits the void … a Product knob (§15)", so with no knob configured
  an abandoned draft — including a never-activated trial and a voided cancel+new successor —
  never terminates, and the archive retention job only touches `cancelled`. Latent: a draft
  bills nothing and provisions nothing; the cost is inventory, not revenue.
- **Cap the pause window's total deferral** (design/04:226, design/04:228, design/01:329).
  §4.2 relies on a bound that does not exist yet — "the pause-day limit (§15, Product/Billing)
  bounds indefinite deferral so a pause cannot be used to escape suspension forever" — while
  the same section lists that limit as open. An open-ended window "has no derived intent and
  clears only by an explicit resumeCollection", and the matrix rejects a re-pause only "while a
  window is open", so consecutive windows chain with no cumulative cap: service runs, the
  ladder stays frozen, collection never happens. Latent because the pause is
  operator-initiated and the knob is openly tracked, but the cumulative cap is the part the
  knob alone does not give.

## Retracted / downgraded during the Challenge pass

Each was carried in as a High or Blocking candidate and cut down by the lines quoted below.

- **"The one-time charge forces a gear that computes no money to compute money" — retracted.**
  The constraint permits exactly what the charge needs: "Subscriptions resolves published
  catalog (Plan/Price/PriceWindow) … facts; it authors no catalog entity, evaluates no overlay,
  and computes no charge" (DESIGN.md:213-215) — copying a frozen flat amount is none of the
  three, and pricing forbids tier machinery on one-time rows. "No monetary column" is
  recurring-scoped in two of its three occurrences. The missing-surfaces half survives as
  item 6.
- **"The recurring period coordinate has two owners, so SUB-D-07's inheritance rule is
  unimplementable" — downgraded to item 20.** Rating concedes identity ("period identity
  itself stays Billing/Subscriptions'", rating design/09:298), pricing's clamp is unambiguous
  enough that there is one legal answer rather than two rival rules, and the residue is
  already tracked at CRIT with an implementation gate in SB1. ADR-0002's sentence is a
  Decision Driver about the proration basis, not a claim about period identity, so the "its
  own ADR contradicts SEAMS" half was unfair.
- **"The resumeCollection source-status row contradicts the amendment that cites it" —
  retracted as a defect, kept only as the formatting note in item 24.** The row's Notes column
  carries the pointer the finding called absent ("also fired system-side at window end (slice
  04 §4.2a)", design/01:330), and §4.2a cites the rule from the convertTrial row, which does
  state it. A second pass re-raised this as High; it does not survive the same check twice.
- **"The setup-charge dedup key breaks on cancel+new, double-charging the fee" — retracted.**
  In all three cancel+new triggers the successor's setup row has a different `priceId`
  (currency and region sit inside pricing's scope key; frequency is a plan column, so a
  cross-frequency target is a different `planId`), so the key could never have deduped it and
  extending it across the `supersedesSubscriptionId` pair would fix nothing. Pricing's rule is
  per subscription — "the setup row charges once per subscription lifetime — at activation" —
  and D-49's GTM line pins the framing ("changing country/currency = a new subscription"). One
  unwritten sentence remains, moot until item 6 gives the charge an emission lane.
- **"The SUB-D-16 dwell is not actually closed" — split.** The propagation claim was refuted:
  the rule, default, resolution order, replay discipline and `nonpayment_exhausted` reason
  code are pinned and every surface the decision names does cite it. What survived is
  behavioural, not documentary — items 3 and 4 — plus the §3.1/§3.6 write-up gap in item
  24(e).

## Method note

The four finder subagents dispatched at the start were cancelled when the session was
interrupted, and their two replacements hit a session limit; the hunt was therefore carried
out directly and the replacements were resumed from transcript afterwards. Three adversarial
skeptics were run against the strongest candidates — one finding was killed outright and four
dropped a tier, which is the Retracted section above. Two candidates from the resumed
interaction pass arrived with fabricated supporting quotes; the underlying gaps were
re-grounded on verified lines (items 11 and 12) rather than dropped or taken on trust. Every
file:line in this artifact was opened and quoted first-hand.
