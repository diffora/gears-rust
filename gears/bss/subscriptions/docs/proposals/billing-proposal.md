---
description: "Required Billing contracts for financial admission, complete invoices, corrections and promotional trial grants."
---

# Billing proposal for Subscriptions

**Status:** Required integration contract; owner agreement and executed conformance are required before enablement. See the [proposal index](README.md) for scope and common rules.

<!-- toc -->

- [Baseline and ownership](#baseline-and-ownership)
- [Required contracts](#required-contracts)
- [Examples](#examples)
- [Delivery and acceptance](#delivery-and-acceptance)

<!-- /toc -->

## Baseline and ownership

Billing owns financial obligations, invoice grouping/state, advance and benefit allocation, final rounding and correction execution. Rating owns gross tariff valuation; Subscriptions owns money-free commercial facts and lifecycle boundaries.

## Required contracts

| Request | Required capability | Completion evidence |
| --- | --- | --- |
| BL-P01 Financial admission | Participate in the deployment-selected authoritative FA provider: exact obligation/terms/coverage, durable allocated prepay satisfaction or accepted future postpay commitment, exclusive consumption, explicit disposition fences and later ordered outcomes | Allowed/pending/refused are distinguishable; no stale or double-consumed funding |
| BL-P02 Compatible invoice target | Combine a commercial seal's exact `(fact_id, fact_version, payload_digest)` members with complete Rating results for those exact sources, expected usage manifests and their source proofs (SUB-D-40) | A newer fact waits for its successor seal; a new calculation result for the same exact fact needs no new seal. Missing evidence is pending; a zero replacement is explicit and preserves lineage. A successor fact with an explicit `Unresolved` interval advances the commercial head; Billing cannot satisfy its seal with an older result and does not reverse posted money automatically (SUB-D-42) |
| BL-P03 Rounding and posting | Publish invoice-line grouping and apply [the money ADR](../../../docs/ADR/0001-cpt-cf-bss-adr-exact-decimal-boundaries.md); sum exact contributions, round once HALF_EVEN at stored scale, validate PostedMoney | The posting receiver accepts major-unit decimals and never rounds again |
| BL-P04 Corrections | Replace drafts with expected-version fencing; issue immutable credit/debit corrections for posted invoices with durable operation identities | Unknown predecessor effects are recovered before a successor adjustment can execute |
| BL-P05 Promotional grant | Commit one reserved grant before readiness, guarantee its effectiveness at Subscriptions T, and record it later without new approval or initial-access delay; allocate/correct it by full Rating result lineage | No pre-T bonus drawdown, duplicate grant or allocation; only fenced non-activation releases commitment, unknown retains it; original expiry/scope preserved and late recording cannot revive cancelled/exhausted/expired access |
| BL-P06 Offboarding and remedies | Apply confirmed-cutoff/served-time evidence, settle erroneous charges and preserve legitimate earlier obligations | Physical cleanup completion and financial settlement remain separate |
| BL-P07 Minimum top-up allocation | Define and validate allocation across invoice keys together with base-line rounding and corrections. Rounding a total shortfall, allocating exact proportions and assigning a residual to a stable slot is a candidate, not an enabled rule. Preserve exact source references and aggregate bounds under the shared money ADR | Golden cases must prove the selected floor after final line rounding, with complete source lineage. The profile remains disabled until that contract is agreed |

**Empty versus zero-valued result.** A complete empty Rating replacement removes that fact's previous contributions, subject to the exact seal and normal target/invoice/execution guards. Billing preserves other facts and history. Active zero-valued lines retain classification, frozen terms and calculation evidence, even if Billing chooses not to display them. Pending with structured reasons supplies no complete monetary result and never implies zero or reversal (SUB-D-44, SUB-D-45). Neither a zero total nor an empty delivery proves refund completion.

See the [canonical result contract](rating-proposal.md#canonical-result-and-billing-handoff).

## Examples

**Successive corrections with a lost reply.** EUR 30 was posted. Replacement 25 requires credit 5, but the reply is lost. Replacement 27 arrives. If credit 5 applied, the next adjustment is debit 2. If it is proven unapplied and fenced, a coalesced credit 3 is possible. Never execute both credits 5 and 3. Pending obligations need durable accounting lineage; they cannot be mistaken for confirmed postings.

**Rounding before correction.** Exact 0.006 EUR posts as 0.01. Corrected exact 0.005 rounds HALF_EVEN to 0; issue a credit note of 0.01 EUR. Rounding the difference instead would be wrong. The negative economic adjustment is not a negative invoice item sent to Ledger.

**Shared trial credit.** A EUR 100 grant covers EUR 60 for item A and EUR 30 for B, leaving 10, not a separate 100 per item. A correction of B to 25 restores 5 once while eligibility lasts. After expiry the corrected history is retained with zero spend permission. Eligible pre-paid-boundary trial overrun beyond the grant is platform-borne under TP, not customer overage debt.

## Delivery and acceptance

Use [FA](../DESIGN.md#3311-financial-admission-integration-contract), [RB](rating-billing-contract.md#subscriptions-rating-and-billing-replacement-contract), [TP](../TRIAL_PRICING_PROPOSAL.md#10-reciprocal-trial-delivery-contract) and [RO](../features/04-renewal-retention.md#7-renewal-and-offboarding-integration-contract). Publish concrete grouping, invoice concurrency, grant and Ledger posting mappings with refusal/recovery outcomes. Test partial advances, draft/post races, lost posting replies, duplicate corrections and trial expiry. Exact accounting accounts/remedy mappings remain owner work; this proposal chooses no new refund policy, tax policy or credit limit.
