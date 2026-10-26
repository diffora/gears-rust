---
description: "Owner-by-owner integration changes, current source baselines, examples and acceptance gates for the subscription design."
---

# Subscription integration proposals by component

**Status:** Required integration contracts. Owner adoption, versioned mappings and runtime conformance are delivery gates. Examples do not define tariff, duration or SLA defaults.

<!-- toc -->

- [Owner handoff](#owner-handoff)
- [Purpose and status](#purpose-and-status)
- [Source baseline and precedence](#source-baseline-and-precedence)
- [Component proposals](#component-proposals)
- [Common evidence and numeric rules](#common-evidence-and-numeric-rules)
- [Delivery order and scope limits](#delivery-order-and-scope-limits)
- [Acceptance record](#acceptance-record)

<!-- /toc -->

## Owner handoff

This branch groups the integration requests for distribution to provider owners. Take each owner's proposal into that owner's design work. Subscriptions and Entitlements define their consumer contracts independently of this package.

The authoritative Subscriptions contracts are [activation](../DESIGN.md#332-pricing-acceptance-and-accepted-time), [trial coverage](../features/06-trials.md), [Rating](../DESIGN.md#rating-result-contract) and [Rating/Billing](../DESIGN.md#financial-consumer-boundary), together with [upstream requirements](../UPSTREAM_REQS.md). Provider packets must conform to those contracts. The [Entitlements proposal index](../../../entitlements/docs/proposals/README.md) covers quota, licenses, Pricing rules, Products metrics and resource management.

## Purpose and status

These proposals define the contracts Subscriptions needs: exchanged evidence, failure and recovery behavior, enablement conditions and joint acceptance cases. Internal architecture, storage, UI, runtime prototypes and implementation plans belong in the provider owners' designs. Each owner adopts the agreed contract separately.

The [provider requirements](../UPSTREAM_REQS.md#25-provider-prerequisites-and-bilateral-acceptance) define the integration dependencies. [AP](../ACTIVATION_PROPOSAL.md#13-orders-and-pricing-reciprocal-activation-contract), [FA](../DESIGN.md#3311-financial-admission-integration-contract), [TP](../TRIAL_PRICING_PROPOSAL.md#10-reciprocal-trial-delivery-contract), [RB](rating-billing-contract.md#subscriptions-rating-and-billing-replacement-contract) and [RO](../features/04-renewal-retention.md#7-renewal-and-offboarding-integration-contract) retain the detailed behavioral contracts and their conformance scenarios. The examples here are not additional product defaults or executed tests.

## Source baseline and precedence

The [Subscriptions design](../DESIGN.md), its feature contracts and the proposals below define the required integration behavior. Repository-local owner documents describe the available provider contracts. A proposed extension is enabled only after the owners agree the versioned mapping and demonstrate conformance; a local design agreement does not prove provider delivery.

## Component proposals

| Component | Proposal | Main request |
| --- | --- | --- |
| Orders Lifecycle | [orders-lifecycle-proposal.md](orders-lifecycle-proposal.md) | Protected acquisition, capture/cancel arbitration, exact read/financial evidence, decimal order totals and suspended-purchase refusal in Orders Changes (OL-P08) |
| Orders Workflow | [orders-workflow-proposal.md](orders-workflow-proposal.md) | Fulfillment guards, actual-time and spawn ordering, operation recovery, approval evidence and financial waiting/cancellation contracts |
| Rating | [rating-proposal.md](rating-proposal.md) | Detailed proposed fact/result profile, accepted slice/lot authority, one-time valuation, strict proofs, conflicts and recovery; preserve full replacements and exact arithmetic |
| Pricing | [pricing-proposal.md](pricing-proposal.md) | Durable future purchase acceptance/recovery, immutable PlanRevision trial terms, per-resource/group minimum profile (PC-P07) and continuation evidence (PC-P08) |
| Products | [products-proposal.md](products-proposal.md) | Durable product evidence, compatibility and historical declarations; no trial-policy fields |
| Billing | [billing-proposal.md](billing-proposal.md) | Complete invoice targets, final rounding, recoverable corrections and shared promotional grants |
| Payments / financial authority | [payments-proposal.md](payments-proposal.md) | Exact prepay/postpay admission, funding exclusivity and capture recovery |
| Ledger | [ledger-proposal.md](ledger-proposal.md) | Map existing exact posting capabilities to Billing corrections, benefit accounting and complete provenance |
| Contracts | [contracts-proposal.md](contracts-proposal.md) | Immutable term evaluation and next-renewal routing |
| Inventory / usage evidence | [usage-evidence-proposal.md](usage-evidence-proposal.md) | Historical membership, independent coverage and exact recoverable usage |

Quota, license and resource adapter proposals belong to the [Entitlements proposals](../../../entitlements/docs/proposals/README.md).

The detailed [Rating/Billing replacement contract](rating-billing-contract.md) defines RB01–RB07 and RB-C01–RB-C16 within Subscriptions. AP, FA, TP and RO remain in their linked Subscriptions documents. Adoption in a provider document is a separate owner task.

The [Rating profile](rating-proposal.md#contract-profile-and-ownership) defines the complete boundary schema and RT-C01–RT-C16 conformance cases. Exact schema/codecs, owner adoption and executed conformance remain release gates under S-U08/S-U11/S-U13.

## Common evidence and numeric rules

Every integration identifies producer/consumer, accepted contract version, business/operation identity, source revision/digest, authorized scope, effective/validity boundaries, unknown/conflict outcomes and durable recovery. Owners must define the exact DTO names. The fields here describe requirements, not shipped SDKs. Read-only prechecks never replace effect-owner fencing. No network call runs inside a local state mutation transaction and no cross-gear database transaction is introduced.

Money follows [the supplied ADR](../../../docs/ADR/0001-cpt-cf-bss-adr-exact-decimal-boundaries.md): major units, exact textual numeric forms, currency and stored scale; exact fractions where required; published bounds; one invoice-line rounding by Billing and no second rounding by Ledger. Numerical business examples omit taxes and FX. Orders totals and Workflow approval values require versioned owner alignment with this representation. Historical data migration is outside scope.

## Delivery order and scope limits

Begin local Subscriptions foundation/facts/recovery as in [DECOMPOSITION](../DECOMPOSITION.md#3-dependency-order-and-delivery-gates). In parallel, ratify AP/FA across Orders, Workflow, Pricing/Products and the selected financial owner. Agree the common fact/result/numeric and invoice-line contracts across Subscriptions, Rating, Billing and Ledger before monetary enablement. Free-period trial requires Pricing trial terms; credit additionally requires Billing promotional coverage. Enable renewal/offboarding and usage profiles only with their applicable Contracts/execution/evidence providers and conformance tests.

Keep migration excluded and import, audit subsystem, confirmAcceptance, notifications, collection pause/resume, transfer, ramps/co-termination, administrative backdating, brand/PlanTier metadata, trial extension/repeat, standalone acquisition, base-plan changes, scheduled composition/resume and linked replacement deferred. Immediate increases/additions consume the separate Change Orders contract; its authoring is not part of this work. Required unavailable notice or unsupported allocation still blocks the affected action/profile.

## Acceptance record

For each component record the ratified version, concrete owner mapping, supported/refused profiles, executed AP/FA/TP/RB/RO cases and remaining integration gates. Every required behavior needs runtime evidence. Source review and document validation do not close provider conformance. Do not mark the whole design unimplemented merely because one optional profile is gated, or mark a gated profile ready because its prose is complete.
