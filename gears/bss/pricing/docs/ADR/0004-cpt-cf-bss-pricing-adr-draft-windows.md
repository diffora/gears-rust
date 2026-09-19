---
status: accepted
date: 2026-09-18
decision-makers: "BSS Product Catalog team"
---

Created:  2026-09-18 by Virtuozzo International GmbH
Updated:  2026-09-18 by Virtuozzo International GmbH

# ADR-0004: Revision-Owned Draft Windows

<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Public contract](#public-contract)
  - [Authoring and reading](#authoring-and-reading)
  - [Error and replay contract](#error-and-replay-contract)
  - [Acceptance examples](#acceptance-examples)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Keep D-332 implicit coverage](#keep-d-332-implicit-coverage)
  - [Accept a window in the publish request](#accept-a-window-in-the-publish-request)
  - [Author explicit draft windows, then approve and publish (chosen)](#author-explicit-draft-windows-then-approve-and-publish-chosen)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-cf-bss-pricing-adr-draft-windows`

## Context and Problem Statement

D-332 (2026-08-17, product-owner confirmed) made a priced first publish open an
**implicit** window at the commit instant so `inst-wc-required` would pass. That
removed D-314's empty-publish ceremony. It also removed the operator's choice of
when the price starts, and it rejected pinning a window to a draft on three
measured grounds that belonged to the **live** window machine (no `draft` in
`pricing_price_window`, no draft lifecycle on a CatalogVersion ref, projector
licence over immutable revisions only).

Pricing Studio authors a **Working** schedule on an open draft, reviews the
complete proposed set, then publishes. An implicit window at commit cannot be
that schedule: the reviewer never sees it, IDs are minted at publish rather than
preserved, and a finite authored tail is indistinguishable from a missing one.

Who owns window **intentions** on a draft revision, and what does publish write?

## Decision Drivers

* A published billable key must still have coverage; silent fallback stays forbidden.
* Publish stays bodyless. No `initialWindow` operand on a verb that has none.
* Draft content is mutable; committed windows remain the four-state live machine.
* Approval must pin the proposed schedule, including live references and deletions.
* Live mutations remain publish units (D-99). Draft edits must not request a
  CatalogVersion or emit `PriceWindowScheduled`.
* The D-79 subscriber lane is still absent; trailing-void stays fail-closed (D-182).

## Considered Options

* Keep D-332: publish writes open-ended coverage at the commit instant.
* Accept a window in the publish request body.
* **Author explicit draft-window intentions on the revision, approve the composed
  proposal, materialize only those operations at publish** (chosen).

## Decision Outcome

A plan revision owns explicit draft window intentions. Publish creates no implicit
coverage. Approval covers the proposed schedule, including referenced live windows
and every authored operation. Publication atomically freezes prices and
materializes approved window operations; activation and sellability remain subject
to the committed CatalogVersion and read-model gates.

This **supersedes product-owner-confirmed D-332** (2026-08-17). D-332's historical
rationale (empty published deltas, ceremony signatures) remains true of the world
it described and is not rewritten. The operator-visible reversal: a priced first
publish is **no longer in force from the commit instant** unless the author wrote
a window (`at_publish` or an exact future start).

Wire and persistence shape: [`../DECISIONS.md`](../DECISIONS.md) D-374 and
`docs/superpowers/plans/2026-09-18-pricing-draft-windows.md`.

### Consequences

* Incomplete draft saves are legal; submit and commit still require complete
  non-overlapping coverage of every required canonical key.
* `at_publish` is a publish-path exception to D-63's strictly-future live create.
  Live `POST …/windows` stays future-only.
* Draft authoring is **not** a D-99 publish unit. Live schedule/adjust/cancel still
  are. Publish of the revision materializes approved creates into `scheduled`.
* Content pin bumps `v18` → `v19`; open approval units must be resubmitted
  (`APPROVAL_CONTENT_MISMATCH`).
* Existing plans need a **guard-row seed** (`INSERT … SELECT DISTINCT` from
  `pricing_plan`), not an implicit-window backfill.

### Confirmation

* Design: D-332 marked superseded; `inst-wc-required` has no publish-written
  exemption; Working/Committed reads are specified; new recovery routes sit on
  `plan × write`.
* Implementation (Tasks 2–10): `open_initial_windows` / `opened_by_this_publish`
  gone; REST and dual-engine tests; local fmt / `--lib --tests` / clippy /
  `make test-pricing-pg` GREEN (2026-09-19). E2E authored in vhp-core `0ed17b10`,
  not executed in this programme.

## Public contract

### Authoring and reading

Create a draft intention on the existing endpoint. First draft of a new plan
(`plan_revision: 0`, D-145):

```http
POST /bss-pricing/v1/prices/{priceId}/windows
If-Match: "0-3"
Idempotency-Key: draft-window-01
Content-Type: application/json

{
  "context": {"kind": "draft", "plan_revision": 0},
  "start": {"kind": "at_publish"},
  "effective_to": null,
  "reason_code": "initial launch"
}
```

A successor draft uses the open revision (example `"3-7"` with `"plan_revision": 3`).
Alternative start: `"start": {"kind": "at", "at": "2027-01-01T00:00:00.000Z"}`.
Return `201`, `Location: /bss-pricing/v1/price-windows?view=working&plan_id={planId}&plan_revision=0`,
the saved `window_id`, `plan_id`, `price_id`, `plan_revision`, `state: draft`,
`start`, `effective_to`, and the updated plan-revision ETag. The revision number
in the body must agree with If-Match.

Live create uses `"context": {"kind": "live"}` and the existing `effective_from`,
`effective_to`, `reason_code` fields. Preserve live approval/202 semantics, no
If-Match on POST, and strictly future starts. Reject mixing `start` with live
fields and reject `at_publish` in live context. A body that omits `context` is
400, including today's three-field live clients until they are updated. Never
infer `live` from a published parent or `draft` from an open revision.

`PATCH /price-windows/{windowId}` takes the same context discriminator.
Draft-created intentions may change their start/end/reason, never price binding.
Targeting a baseline live ID in draft context stages an `adjust_end` operation;
it cannot change a past start or price binding. Its response includes
`operation_id` and the target `window_id`. Draft PATCH If-Match is the **plan**
ETag (`"<revision>-<row_version>"`, D-170). Live PATCH If-Match stays the window
`mutation_seq` (D-191).

`DELETE /price-windows/{windowId}?context=draft&plan_id={planId}&plan_revision=0`
removes a draft-created intention, or stages cancellation of a baseline scheduled
window. It requires the plan ETag and Idempotency-Key. Live delete uses
`?context=live` and **existing** cancellation guards (no If-Match, no idempotency
header). Omitting `context` is 400. Never cancel an active/expired live window
through draft context.

Two explicit recovery operations (both `plan × write`, `resource_id = planId`):

- `DELETE /plans/{planId}/draft-window-operations/{operationId}?plan_revision=0`:
  discard a staged live adjustment/cancellation; require plan ETag and
  Idempotency-Key; return 204 + updated ETag.
- `POST /plans/{planId}/draft-window-baseline/refresh`, body `{"plan_revision":0}`,
  same headers: replace baseline references atomically, retain new-window
  intentions, and remove staged operations whose target version
  changed/disappeared. Return 200 with updated ETag and
  `discarded_operation_ids`. Re-evaluate coverage; do not renew approval
  automatically.

Extend existing list and coverage readers with explicit `view=working|committed`.
Price windows have **no GET-by-id** (D-191: `{windowId}` is PATCH+DELETE only).
**`view` is a door discriminator, not an OData field.** Default remains
`committed` (today's live table; draft rows never appear). Working reads are
`plan × read` and require:

| Surface | Required query |
|---|---|
| `GET /price-windows` | `view=working&plan_id={planId}&plan_revision={n}` |
| `GET /plans/{planId}/coverage` | `view=working&plan_revision={n}` |

Unknown `view` → 400. `view=working` on a revision that is not an open draft →
409 `DRAFT_WINDOW_CONTEXT_CHANGED`. Named `plan_id` is legal **only** with
`view=working`; on the committed collection it stays 400 (the existing
OData-only contract). Working composes baseline plus intentions and returns
provenance so UI cannot confuse a live record with a draft intention. List and
coverage keep a legally saved proposal visible after the clock has moved;
submit and commit still refuse elapsed exact starts and time-dependent
cancel/adjust legality. Bind
`view`, parent and revision into the cursor fingerprint. Historical published
revisions are not mutable working views.

New window IDs survive draft → committed. Committed windows enter `scheduled`;
the existing scheduler determines activation. Unchanged live windows are
references — never copied or reactivated during publication of a successor
revision. A new-plan clone receives no windows. Abandoning a revision retains
its intentions for audit but excludes them from all working/committed
evaluations.

### Error and replay contract

| Condition | Wire result |
|---|---|
| Malformed context/start, unsupported field combination or precision | Existing canonical 400 validation response, field path included |
| Missing `context` on POST/PATCH or on DELETE query | Existing canonical 400; never inferred as live or draft |
| Missing required precondition | Existing pricing precondition-required response |
| Stale draft ETag | Existing `STALE_VERSION` mapping |
| Draft already published/abandoned or context revision mismatch | 409 `DRAFT_WINDOW_CONTEXT_CHANGED` |
| Live dependency operator mutation after capture/approval | 409 `WINDOW_BASELINE_CHANGED` |
| Exact start passed by commit | 409 `WINDOW_START_ELAPSED`; nothing commits |
| Coverage missing/gap/trailing void/overlap | Existing coverage/window reason codes and canonical HTTP mapping |
| Foreign tenant or foreign parent | 404, indistinguishable from absent |
| Same idempotency key, different normalized payload/context | Existing `IDEMPOTENCY_PAYLOAD_MISMATCH` |

Authenticate and authorize every replay. After that, find the saved idempotency
result **before** testing current revision/window lifecycle; replay returns the
original status/body/ETag even after publish. New attempts still require current
lifecycle/version checks. Persist result and authoring mutation in the same
transaction. Namespace the request hash with context, revision, parent, method,
route and payload.

### Acceptance examples

1. **Draft save without any window succeeds.** A priced revision with rows and no
   draft-window intention is a legal incomplete save. No CatalogVersion is
   requested; no live `pricing_price_window` row is inserted.
2. **Submit without coverage fails.** Submit/commit of that revision is refused
   (`WINDOW_COVERAGE_MISSING` / existing coverage mapping). Publish writes no
   implicit open-ended window.
3. **Two contiguous windows pass.** `[T0, T1)` then `[T1, ∞)` on one key
   (half-open adjacency) is complete non-overlapping coverage and publishes.
4. **`at_publish` remains symbolic until commit.** Working reads keep
   `start.kind = at_publish`. Materialization stamps `effective_from` to the
   commit instant; the committed window enters `scheduled`.
5. **Late exact start rolls back.** An authored `start.kind = at` whose instant
   has passed at commit is 409 `WINDOW_START_ELAPSED`; prices and windows are
   unchanged.
6. **Unchanged baseline is not duplicated.** A successor revision that does not
   stage an adjust/cancel on a live window materializes no second interval for
   that id.
7. **End-of-sales does not terminate subscriber obligations.** Closing
   `available_to`, retiring the plan, or assuming zero subscribers is not a
   trailing-void exemption. Finite individual windows followed by continuous
   open-ended coverage are supported; a final finite tail is refused until an
   authoritative subscriber-termination/migration contract exists (D-79/D-182).

## Pros and Cons of the Options

### Keep D-332 implicit coverage

* Good, because a priced publish is immediately in force and empty deltas stay gone.
* Bad, because Studio cannot review or version the schedule, and finite coverage
  cannot be authored before commit.
* Bad, because a window the operator did not write still exists after publish —
  the consequence the D-332 veto flag named.

### Accept a window in the publish request

* Good, because publish remains one call.
* Bad, because a bodyless verb gains an operand, and omitting it returns the
  coverage refusal D-332 existed to avoid.
* Bad, because approval cannot pin the schedule before the publish call.

### Author explicit draft windows, then approve and publish (chosen)

* Good, because Working → approval → Committed matches the Studio workflow and
  keeps live windows out of `draft`.
* Good, because authored IDs survive commit and unchanged live windows stay
  references, not copies.
* Bad, because live window POST/PATCH/DELETE become a breaking `context`
  discriminator, and every existing caller must send `kind=live`.
* Bad, because D-332's "in force at commit" default is withdrawn.

## More Information

Register: D-374 (supersedes D-332; amends D-63 and D-99). Implementation plan:
`docs/superpowers/plans/2026-09-18-pricing-draft-windows.md`. Prior consolidation:
[ADR-0003](./0003-cpt-cf-bss-pricing-adr-pricewindow-consolidation.md).

## Traceability

- **PRD**: §6.5 `fr-pricewindow-coverage`, AC #24, AC #78, AC #118
- **Design**: [`../design/07-pricewindow-linkage.md`](../design/07-pricewindow-linkage.md)
  (`inst-wc-required`, `inst-ws-future-start`, `inst-ws-publishunit`, §5);
  [`../design/01-foundation.md`](../design/01-foundation.md) publish / pin;
  [`../design/05-governance.md`](../design/05-governance.md) endpoint map;
  [`../design/ui-read-contracts.md`](../design/ui-read-contracts.md) Working/Committed
- **Related ADRs**: `cpt-cf-bss-pricing-adr-pricewindow-consolidation`
