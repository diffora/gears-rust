---
description: "Ledger integration requirements for exact postings, correction lineage and promotional benefit accounting."
---

# Ledger proposal for the subscription billing chain

**Status:** Subscriptions integration proposal, 2026-10-15. This is an owner handoff, not a delivered API or external-owner approval. Examples are illustrative, not new tariff, duration or SLA defaults. See the [proposal index](README.md) for source precedence, scope exclusions and delivery order.

<!-- toc -->

- [Baseline and already delivered capability](#baseline-and-already-delivered-capability)
- [Required integration work](#required-integration-work)
- [Examples](#examples)
- [Acceptance and limits](#acceptance-and-limits)
- [Owner document adoption](#owner-document-adoption)

<!-- /toc -->

## Baseline and already delivered capability

The local [Ledger documents](../../../ledger/docs/DESIGN.md) are from the older branch baseline. The newer source tree inspected with Rating `3a1cd8ac33bda1a72838a81435c7365a5d387d05` contains [PostLine.money as PostedMoney](https://github.com/constructorfabric/gears-rust/blob/3a1cd8ac33bda1a72838a81435c7365a5d387d05/gears/bss/ledger/ledger-sdk/src/posting.rs) and the shared [money library](https://github.com/constructorfabric/gears-rust/blob/3a1cd8ac33bda1a72838a81435c7365a5d387d05/gears/bss/libs/money/README.md). Rating records the major-unit invoice DTO adoption from #5291. Thus exact validated posting is an existing capability in that inspected source, not a request to invent a new minor-unit API. Integration into this older branch is a separate dependency; no runtime test was run here.

## Required integration work

| Request | Required mapping | Retained owner boundary |
| --- | --- | --- |
| LG-P01 Posted invoice lines | Accept Billing's validated major-unit decimal with currency and stored scale; preserve exact storage and range errors | Billing rounds and groups; Ledger rejects invalid increments rather than silently rounding |
| LG-P02 Financial execution identity | Map Billing target/correction operations onto idempotent posting identities and authoritative result recovery | Ledger posting success is distinct from Subscriptions activation and resource completion |
| LG-P03 Complete provenance | Preserve invoice/group/result and calculation/accepted-source lineage, directly or through an immutable aggregate reference | One optional snapshot field cannot justify dropping sources or changing invoice grouping |
| LG-P04 Benefit postings | Agree Billing's promotional allocation/correction accounting path and platform-borne amounts | Existing customer-funded credit application is not a free promotional grant; Ledger does not own trial eligibility/expiry |
| LG-P05 Immutable corrections | Use supported credit/debit/reversal documents and retain prior references across retries | No inline rewrite of a posted invoice and no negative invoice item as a shortcut |

## Examples

**Valid posting versus rate.** EUR 0.047 is valid as an accepted rate but not as a scale-2 PostedMoney amount. Billing first aggregates and rounds its invoice line. Ledger accepts the resulting EUR 0.05 and rejects an unrounded 0.047 posting without rounding it itself.

**Correction reply lost.** Billing submitted a credit note for EUR 5 and lost the response. The same business identity must recover the original posting. A later debit of EUR 2 may proceed only after the preceding outcome is reconciled; it cannot generate a duplicate EUR 5 credit.

**Promotional credit.** A EUR 100 trial grant has no corresponding customer deposit. Its accounting mapping must be agreed with Billing; do not call the cash-funded credit API with invented customer funds.

## Acceptance and limits

Apply [the money ADR](https://github.com/constructorfabric/gears-rust/blob/510ce99f037a19f63d41b17acb3b0680e9331a44/gears/bss/docs/ADR/0001-cpt-cf-bss-adr-exact-decimal-boundaries.md), [RB](rating-billing-contract.md#subscriptions-rating-and-billing-replacement-contract) and applicable TP scenarios to the selected Ledger version. Verify exact round-trip, increment/currency/scale validation, duplicate posting recovery and complete historical references. Ledger supplies accounting evidence, not financial service admission or resource entitlement. No ledger migration project or new chart of accounts is included.

## Owner document adoption

In a separate Ledger/Billing integration PR, align the PRD/DESIGN boundary and payments/allocation documentation with RB, TP and FA. Billing validates complete financial targets, resolves invoice-version races and recovers predecessor work; Ledger receives the selected idempotent posting/allocation/correction operation. A posting receipt alone cannot certify commercial/rating completeness.

The existing cash-funded Grant must not fabricate a promotional settlement. Billing owns staged grant preparation, finalization, allocation and correction; any required Ledger accounting mapping is separately agreed, with original expiry and posting identity retained. This proposal chooses no journal, tax, refund or recognition policy and adds no generic invoice orchestrator.

Settlement, balances, allocations and disputes are accounting evidence. None supplies commit-valid service admission, a PSP capture instruction or a postpay credit reservation. The selected financial authority must bind the exact obligation/coverage and prevent concurrent reuse; raw Ledger events do not command activate/suspend/resume.
