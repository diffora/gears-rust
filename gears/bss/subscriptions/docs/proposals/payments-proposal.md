---
description: "External financial admission and protected capture requirements for Subscriptions, including postpay and recovery."
---

# Payments and financial admission proposal

**Status:** Subscriptions integration proposal, 2026-10-15. This is an owner handoff, not a delivered API or external-owner approval. Examples are illustrative, not new tariff, duration or SLA defaults. See the [proposal index](README.md) for source precedence, scope exclusions and delivery order.

<!-- toc -->

- [Baseline and owner selection](#baseline-and-owner-selection)
- [Required changes](#required-changes)
- [Examples](#examples)
- [Delivery and acceptance](#delivery-and-acceptance)

<!-- /toc -->

## Baseline and owner selection

The local Orders authorization guard does not supply actual capture recovery or commit-valid paid-service permission. No canonical Payments/credit-admission provider is delivered here. Select the authoritative financial owner by deployment; Payments may execute collection while Billing/credit services own obligation and credit eligibility. Ledger balance and a policy boolean are not substitutes for that authority.

## Required changes

| Request | Required behavior |
| --- | --- |
| PM-P01 Exact admission evidence | Bind the decision to accepted terms, operation/acquisition/attempt, obligation, coverage and intended consumer; provide bounded validity or acknowledged withdrawal fencing compatible with local commit |
| PM-P02 Settlement schemes | Prepay proves the prescribed payment; postpay proves authorization under accepted credit terms. An available card or PSP authorization is not prescribed-payment satisfaction |
| PM-P03 Capture execution | Consume Lifecycle's durable capture-start decision and stable capture identity after capacity/terms protection; suppress a delayed debit when the cancellation fence wins |
| PM-P04 Funding exclusivity | Serialize finite funding/credit coverage at the financial owner so two operations cannot spend one authorization allocation |
| PM-P05 Unknown and later outcomes | Recover original execution after lost replies; expose ordered authoritative outcomes, including later delinquency or chargeback, without rewriting past subscription application |
| PM-P06 Numeric transport | Carry [exact major-unit amounts and currency/scale](https://github.com/constructorfabric/gears-rust/blob/510ce99f037a19f63d41b17acb3b0680e9331a44/gears/bss/docs/ADR/0001-cpt-cf-bss-adr-exact-decimal-boundaries.md) internally. Any PSP-specific representation belongs to an explicit validated adapter, not an implicit change of BSS units |

## Examples

**Postpay without a card.** A customer is authorized to receive service under accepted credit terms. Subscriptions can activate with valid financial admission; no card charge is required at that instant. Later due-and-unpaid evidence may trigger the agreed nonpayment lifecycle.

**One payment, two consumers.** An obligation has EUR 100 of applicable funding and two concurrent requests each seek that same coverage. The owner must prevent both from consuming it; a repeated positive balance read cannot enforce exclusivity.

**Capture response lost.** A debit may already have succeeded. Retry queries/replays the same capture identity; it neither starts another debit nor releases the order's place as unpaid. Cancellation records the applicable financial recovery/remedy and independently fences activation.

## Delivery and acceptance

Map [FA-C01 through FA-C12](../DESIGN.md#3311-financial-admission-integration-contract) and AP capture/release cases to concrete provider SDKs and recovery reads. Prove application-versus-withdrawal ordering, cancellation-versus-debit ordering, concurrent funding consumption and late stale refusals. Due-date calculation, collection strategy, credit limits and refund method/timing remain financial-owner policy, not new Subscriptions functionality.
