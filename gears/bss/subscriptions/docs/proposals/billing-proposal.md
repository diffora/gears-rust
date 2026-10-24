---
description: "Required Billing contracts for financial admission, complete invoices, corrections and promotional trial grants."
---

# Billing proposal for Subscriptions

**Status:** Subscriptions integration proposal, 2026-10-15. This is an owner handoff, not a delivered API or external-owner approval. Examples are illustrative, not new tariff, duration or SLA defaults. See the [proposal index](README.md) for source precedence, scope exclusions and delivery order.

<!-- toc -->

- [Baseline and ownership](#baseline-and-ownership)
- [Required contracts](#required-contracts)
- [Examples](#examples)
- [Delivery and acceptance](#delivery-and-acceptance)

<!-- /toc -->

## Baseline and ownership

No canonical Billing implementation is present in this branch; this is a required provider contract, not a patch to a delivered Billing API. Billing owns financial obligations, invoice grouping/state, advance and benefit allocation, final rounding and correction execution. Rating owns gross tariff valuation; Subscriptions owns money-free commercial facts and lifecycle boundaries.

## Required contracts

| Request | Required capability | Completion evidence |
| --- | --- | --- |
| BL-P01 Financial admission | Participate in the deployment-selected authoritative FA provider: exact obligation/terms/coverage, prepay satisfaction or postpay authorization, bounded consumption and later ordered outcomes | Allowed/pending/refused are distinguishable; no stale or double-consumed funding |
| BL-P02 Compatible invoice target | Combine a commercial seal's exact `(fact_id, fact_version, payload_digest)` members with complete Rating results for those exact sources, expected usage manifests and their source proofs (SUB-D-40) | A newer fact waits for its successor seal; a new calculation result for the same exact fact needs no new seal. Missing evidence is pending; a zero replacement is explicit and preserves lineage. A successor fact with an explicit `Unresolved` interval advances the commercial head; Billing cannot satisfy its seal with an older result and does not reverse posted money automatically (SUB-D-42) |
| BL-P03 Rounding and posting | Publish invoice-line grouping and apply [the money ADR](https://github.com/constructorfabric/gears-rust/blob/510ce99f037a19f63d41b17acb3b0680e9331a44/gears/bss/docs/ADR/0001-cpt-cf-bss-adr-exact-decimal-boundaries.md); sum exact contributions, round once HALF_EVEN at stored scale, validate PostedMoney | Posting receiver receives major-unit decimals and never rounds again |
| BL-P04 Corrections | Replace drafts with expected-version fencing; issue immutable credit/debit corrections for posted invoices with durable operation identities | Unknown predecessor effects are recovered before a successor adjustment can execute |
| BL-P05 Promotional grant | Prepare durable coverage, finalize one shared grant from applied T, allocate/correct it by full Rating result lineage | No duplicate grant or allocation, original expiry/scope preserved; exhausted/expired trial is not revived |
| BL-P06 Offboarding and remedies | Apply confirmed-cutoff/served-time evidence, settle erroneous charges and preserve legitimate earlier obligations | Physical cleanup completion and financial settlement remain separate |
| BL-P07 Minimum top-up allocation | Take the exact shortfall Rating returns once per selected target and window, round it once `HALF_EVEN` at stored scale (BL-P03), and allocate the rounded amount across the invoice keys it spans by the shared money allocation: exact proportions, each share rounded once at stored scale, the exact residual to one pinned slot. Store an immutable aggregate-evidence object naming the exact source fact refs and digests and the allocation rule, and validate aggregate bounds across facts (SUB-D-47, SUB-D-48, SUB-D-49, SUB-D-50, SUB-D-51, SUB-D-52, SUB-D-53, SUB-D-54) | The shares sum exactly to the rounded shortfall and no share is rounded again; missing source evidence keeps the target pending; no fact is dropped from the aggregate evidence |

**Empty versus zero-valued result accepted, 2026-10-16.** Billing treats a complete empty Rating replacement as removal of that fact's previous contributions, subject to the exact seal and normal target/invoice/execution guards; other facts and history remain. Active zero-valued lines retain classification, frozen terms and calculation evidence, even if Billing chooses not to display them. Pending with structured reasons supplies no complete monetary result and never implies zero or reversal (SUB-D-44, SUB-D-45). Neither a zero total nor an empty delivery proves refund completion. See the [canonical result contract](rating-proposal.md#canonical-result-and-billing-handoff).

## Examples

**Successive corrections with a lost reply.** EUR 30 was posted. Replacement 25 requires credit 5, but the reply is lost. Replacement 27 arrives. If credit 5 applied, the next adjustment is debit 2. If it is proven unapplied and fenced, a coalesced credit 3 is possible. Never execute both credits 5 and 3. Pending obligations need durable accounting lineage; they cannot be mistaken for confirmed postings.

**Rounding before correction.** Exact 0.006 EUR posts as 0.01. Corrected exact 0.005 rounds HALF_EVEN to 0; issue a credit note of 0.01 EUR. Rounding the difference instead would be wrong. The negative economic adjustment is not a negative invoice item sent to Ledger.

**Shared trial credit.** A EUR 100 grant covers EUR 60 for item A and EUR 30 for B, leaving 10, not a separate 100 per item. A correction of B to 25 restores 5 once while eligibility lasts. After expiry the corrected history is retained with zero spend permission. Eligible pre-paid-boundary trial overrun beyond the grant is platform-borne under TP, not customer overage debt.

## Delivery and acceptance

Use [FA](../DESIGN.md#3311-financial-admission-integration-contract), [RB](rating-billing-contract.md#subscriptions-rating-and-billing-replacement-contract), [TP](../TRIAL_PRICING_PROPOSAL.md#10-reciprocal-trial-delivery-contract) and [RO](../features/04-renewal-retention.md#7-renewal-and-offboarding-integration-contract). Publish concrete grouping, invoice concurrency, grant and Ledger posting mappings with refusal/recovery outcomes. Test partial advances, draft/post races, lost posting replies, duplicate corrections and trial expiry. Exact accounting accounts/remedy mappings remain owner work; this proposal chooses no new refund policy, tax policy or credit limit.
