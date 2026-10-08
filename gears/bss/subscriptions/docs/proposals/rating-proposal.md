---
description: "Remaining Rating integration deltas against current PR 5175 and the BSS money ADR, with valuation and correction examples."
---

# Rating proposal for the Subscriptions contract

**Status:** Subscriptions integration proposal, 2026-10-15. This is an owner handoff, not a delivered API or external-owner approval. Examples are illustrative, not new tariff, duration or SLA defaults. See the [proposal index](README.md) for source precedence, scope exclusions and delivery order.

<!-- toc -->

- [Sources and already aligned design](#sources-and-already-aligned-design)
- [Remaining integration changes](#remaining-integration-changes)
- [Examples](#examples)
- [Acceptance and source adoption](#acceptance-and-source-adoption)
- [Shared contract adoption checks](#shared-contract-adoption-checks)

<!-- /toc -->

## Sources and already aligned design

[PR #5270](https://github.com/constructorfabric/gears-rust/pull/5270) is the cross-BSS money ADR, not the Rating design. Read it at `510ce99f037a19f63d41b17acb3b0680e9331a44`. The current [Rating PR #5175](https://github.com/constructorfabric/gears-rust/pull/5175) was separately inspected at `3a1cd8ac33bda1a72838a81435c7365a5d387d05`; its consolidated [DESIGN](https://github.com/constructorfabric/gears-rust/blob/3a1cd8ac33bda1a72838a81435c7365a5d387d05/gears/bss/rating/docs/DESIGN.md) and feature files replace the old `docs/design/` layout present in this branch.

Already specified in that Rating revision: absolute full result revisions, no separate delta lane, Billing-owned invoice state, exact major-unit fractions with currency/scale, one Billing rounding step, recurring proration and per-price minimum fees, retained calculation evidence, expected-child completeness and pull recovery. These are **retain and verify**, not requests to invent them again. The Subscriptions [RB proposal](rating-billing-contract.md#subscriptions-rating-and-billing-replacement-contract) must be adopted by contract meaning in a separate Rating/Billing change, preserving behavior already present in the newer design.

## Remaining integration changes

| Request | Current source evidence | Required delta |
| --- | --- | --- |
| RT-P01 Current Subscriptions input | [Contract 11 §4.3](https://github.com/constructorfabric/gears-rust/blob/3a1cd8ac33bda1a72838a81435c7365a5d387d05/gears/bss/rating/docs/DESIGN.md#contract-11-4-3) still describes old phase-entry identity and missing Subscriptions accepted-term storage | Adopt immutable fact/version/digest, durable purchased-action occurrence, complete accepted term slices and lot-aware quantity provenance. Replace stale source-status statements; do not retain phase identity as the new one-time key. |
| RT-P02 One-time valuation | R-19 still gates rating one-time facts; the current design stores them without valuation until owner agreement | Record the Subscriptions-side accepted direction and ratify it with Rating/Billing. Enable one-time scheduling/valuation only with a committed qualifying action and agreed occurrence schema. No fee from retry, resume or conversion alone. |
| RT-P03 Accepted bindings versus calculation evidence | Initial comparison and later slice re-resolution remain based on the old pin contract | Consume the complete accepted binding for every frozen slice/lot. Historical read/replay must not choose a live catalog successor. Preserve accepted-evidence identity separately from Rating's calculation snapshot. Any new renewal binding must originate in accepted commercial policy. |
| RT-P04 Simultaneous price lots | The old per-item pin/timeline cannot by itself represent two simultaneous accepted prices for one item | Version fact intake, calculation inputs, result lineage and fixtures for concurrent lots. Refuse unsupported pooled usage allocation instead of collapsing two tariffs into one. |
| RT-P05 Strict completeness and recovery | The source retains migration `derived` groups and `delay_only` evidence modes | For this new Subscriptions profile require published parents/groups, independent ownership/inventory and usage coverage, and complete child results. No migration implementation is required here; do not enable an evidence-less fallback for this profile. Preserve the existing complete-result delivery lane and use event-independent recovery for old corrections. |
| RT-P06 Trial and offboarding valuation | General rating does not define the full Subscriptions benefit/liability boundary | Free-period terms include explicit zero covered fees; credit trials retain gross normal tariffs. Carry original served-time/boundary evidence so Billing can apply grants/platform liability. Apply the agreed nonchargeable cancellation tail without discarding truthful runtime. |
| RT-P07 Billing contract closure | [Contract 16 §4.1](https://github.com/constructorfabric/gears-rust/blob/3a1cd8ac33bda1a72838a81435c7365a5d387d05/gears/bss/rating/docs/DESIGN.md#contract-16-4-1) leaves multi-fact Ledger snapshot mapping open (R-26); R-32 requires Billing line-key agreement | Ratify grouping, full-source lineage, invoice-version fencing and uncertain-predecessor recovery with Billing. Do not split already-rounded contributions merely to fit one snapshot field. Rating continues publishing full exact results, never posting differences. |

## Examples

**Concurrent purchases.** Ten units use the accepted EUR 2 monthly tariff. Two added units use a later EUR 3 tariff. A full-month example totals EUR 26; an increase part-way through a month uses the accepted served intervals and proration. Rating must never reprice all twelve units at EUR 3. Replaying the purchase adds no second lot.

**One-time charge.** An accepted EUR 50 setup action completes. Rating emits the full EUR 50 result once for its stable occurrence. Billing applies an existing EUR 50 advance, if applicable, without collecting twice. A resumed subscription or rebuilt technical attempt creates no new setup occurrence.

**Exact rounding.** Two exact contributions of EUR 0.005 to the same invoice line stay exact in Rating and sum to EUR 0.01 for Billing. If they belong to two distinct invoice-line keys, each rounds to EUR 0 under HALF_EVEN. Grouping is contractual; it cannot be changed by transport convenience.

**Correction after posting.** An exact EUR 0.006 line was posted as EUR 0.01. Rating replaces it with exact EUR 0.005. Billing rounds the corrected total to EUR 0, then issues a EUR 0.01 credit note. Rating does not round a negative difference or post an invoice adjustment.

**Credit-trial correction.** Normal rated value changes from EUR 30 to EUR 25 under the same eligible EUR 100 grant. Rating publishes a full replacement of EUR 25. Billing restores EUR 5 once if still eligible; after original expiry, history changes but spend permission remains zero.

## Acceptance and source adoption

Update the current consolidated DESIGN contracts 08, 11 and 14–16, the corresponding current features, PRD/SEAMS, upstream S-requests and decisions R-19/R-26/R-29/R-32 as applicable. Keep already adopted T-D-50/T-D-80 behavior. Exact current feature destinations are listed in the [source tree](https://github.com/constructorfabric/gears-rust/tree/3a1cd8ac33bda1a72838a81435c7365a5d387d05/gears/bss/rating/docs/features).

Run [RB-C01 through RB-C16](rating-billing-contract.md#subscriptions-rating-and-billing-replacement-contract), applicable [TP cases](../TRIAL_PRICING_PROPOSAL.md#7-joint-conformance-cases) and the money ADR's numeric fixtures. A usage-only full October UTC period expects 744 hourly windows; one missing proof keeps it incomplete. Missing prices, unknown ownership and pending financial execution never become zero. Source inspection is not external-owner ratification or runtime proof.

## Shared contract adoption checks

The older local slices contain `periodState` prerequisites and independent `Adjustment` dedup rows that conflict with this proposed profile. They remain baseline evidence in this PR; do not copy them into the newer design during adoption.

- Preserve the current #5175 rule: Billing period state is an observation only. Complete frozen commercial/usage inputs can be valued and a full result can be persisted/published while the Billing projection is absent. Never substitute an unknown period state with `open` or `closed_posted`.
- Apply that rule consistently to evaluation-context assembly, unit synthesis, correction routing and delivery. In the old layout those are foundation 01, corrections 08, consumer contract 11, synthesis 14 and handoff 16; use the equivalent contracts/features in the current consolidated design.
- Deduplicate the RB monetary lane by stable full-result identity, revision and digest. A replay is a no-op, lower versions cannot replace the accepted head, and equal revision with different digest conflicts. Legacy `Adjustment` keys in old contracts 11 and 15 are not a second RB financial lane.
- Billing recovers its authoritative invoice revision and predecessor financial outcomes before applying the new target. Rating publication does not imply invoice execution, and missing Billing history keeps financial work pending.

**Example.** The full frozen inputs yield EUR 25 while the Billing period-state projection is unavailable. Rating still publishes the full EUR 25 result. Billing stages it and recovers whether EUR 30 was already posted before choosing a draft replacement or an idempotent EUR 5 credit. The missing projection neither blocks tariff math nor authorizes an assumed financial correction.
