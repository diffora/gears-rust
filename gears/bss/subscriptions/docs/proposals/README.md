---
description: "Owner-by-owner integration changes, current source baselines, examples and acceptance gates for the subscription design."
---

# Subscription integration proposals by component

**Status:** Subscriptions integration proposal, 2026-10-15. This is an owner handoff, not a delivered API or external-owner approval. Examples are illustrative, not new tariff, duration or SLA defaults. The source precedence, scope exclusions and delivery order are recorded below.

<!-- toc -->

- [Purpose and status](#purpose-and-status)
- [Source baseline and precedence](#source-baseline-and-precedence)
- [Component proposals](#component-proposals)
- [Common evidence and numeric rules](#common-evidence-and-numeric-rules)
- [Delivery order and scope limits](#delivery-order-and-scope-limits)
- [Acceptance record](#acceptance-record)

<!-- /toc -->

## Purpose and status

These files explain what each adjacent component must change, what is already compatible, and how the change behaves in concrete examples. They are proposed owner changes derived from the retained Subscriptions design. This PR changes only Subscriptions documentation; the adjacent gears retain the PR base versions. Their PRD/design adoption belongs in separate owner PRs. They do not reopen product policy, create new owner approvals, or claim delivered APIs.

The [core closure register](../RECONCILIATION.md#61-remaining-work-and-closure-order) remains the status authority. [AP](../ACTIVATION_PROPOSAL.md#13-orders-and-pricing-reciprocal-activation-contract), [FA](../DESIGN.md#3311-financial-admission-integration-contract), [TP](../TRIAL_PRICING_PROPOSAL.md#10-reciprocal-trial-delivery-contract), [RB](rating-billing-contract.md#subscriptions-rating-and-billing-replacement-contract) and [RO](../features/04-renewal-retention.md#7-renewal-and-offboarding-integration-contract) retain the detailed behavioral contracts and 82 scenario identities. The examples here are not additional product defaults or executed tests.

## Source baseline and precedence

| Source | Inspected revision | Use |
| --- | --- | --- |
| This Subscriptions PR | `6b8a085914be5b3104507f39725cdd745653fb65` before this follow-up | Historical reconciliation evidence before scope isolation; adjacent-owner amendments have been withdrawn from this PR and retained as proposals |
| [Orders Workflow PR #4787](https://github.com/constructorfabric/gears-rust/pull/4787) | `ce16d8ff2f59e674ca5e36b091588781022348ea` | Actual current Workflow design, including slices 03–06, 08 and 10; inspected as source, not imported into this branch |
| [Rating PR #5175](https://github.com/constructorfabric/gears-rust/pull/5175) | `3a1cd8ac33bda1a72838a81435c7365a5d387d05` | Current consolidated Rating design and feature layout, including money-ADR adoption; supersedes earlier assumptions about what that design already contains |
| [BSS money PR #5270](https://github.com/constructorfabric/gears-rust/pull/5270) | `510ce99f037a19f63d41b17acb3b0680e9331a44` | Cross-BSS exact amounts, posting and correction rules; this is the money ADR, not a replacement Rating design |
| Ledger SDK and shared money library | Same inspected tree as Rating above | Verified newer `PostLine.money: PostedMoney` and shared exact-money support; no runtime verification claimed |

PR descriptions can lag their files. The statements here use the pinned file contents. Local owner documents retain the unchanged PR base; links to them describe the baseline, not acceptance of these proposals. The newer branches have not been imported. External owner acceptance and concrete adapter/version mapping remain open even when two documents agree. The money ADR is adopted by the inspected Rating design; its own frontmatter still says proposed despite the merged PR.

## Component proposals

| Component | Proposal | Main request |
| --- | --- | --- |
| Orders Lifecycle | [orders-lifecycle-proposal.md](orders-lifecycle-proposal.md) | Protected acquisition, capture/cancel arbitration, exact read/financial evidence and decimal order totals |
| Orders Workflow | [orders-workflow-proposal.md](orders-workflow-proposal.md) | Extend real process steps/guards/recovery without duplicating already specified actual-time and spawn ordering |
| Rating | [rating-proposal.md](rating-proposal.md) | Adopt current facts/lots/one-time policy and strict proofs; preserve existing full replacements and exact arithmetic |
| Pricing | [pricing-proposal.md](pricing-proposal.md) | Future commitment/final permit and immutable PlanRevision trial terms |
| Products | [products-proposal.md](products-proposal.md) | Durable product evidence, compatibility and historical declarations; no trial-policy fields |
| Billing | [billing-proposal.md](billing-proposal.md) | Complete invoice targets, final rounding, recoverable corrections and shared promotional grants |
| Payments / financial authority | [payments-proposal.md](payments-proposal.md) | Exact prepay/postpay admission, funding exclusivity and capture recovery |
| Ledger | [ledger-proposal.md](ledger-proposal.md) | Map existing exact posting capabilities to Billing corrections, benefit accounting and complete provenance |
| Contracts | [contracts-proposal.md](contracts-proposal.md) | Immutable term evaluation and next-renewal routing |
| Entitlement adapter | [entitlement-provider-proposal.md](entitlement-provider-proposal.md) | Correlated desired-state application and stale-command suppression |
| Resource adapter | [resource-provider-proposal.md](resource-provider-proposal.md) | Truthful stop/retention/delete outcomes and execution-boundary hold fencing |
| Inventory / usage evidence | [usage-evidence-proposal.md](usage-evidence-proposal.md) | Historical membership, independent coverage and exact recoverable usage |

The detailed [Rating/Billing replacement contract](rating-billing-contract.md) preserves RB01–RB07 and RB-C01–RB-C16 within Subscriptions. AP, FA, TP and RO remain in their linked Subscriptions documents. Adoption in a provider document is a separate owner task.

## Common evidence and numeric rules

Every integration identifies producer/consumer, accepted contract version, business/operation identity, source revision/digest, authorized scope, effective/validity boundaries, unknown/conflict outcomes and durable recovery. Exact DTO names remain owner work; conceptual fields in these proposals are not invented shipped SDKs. Read-only prechecks never replace effect-owner fencing. No network call runs inside a local state mutation transaction and no cross-gear database transaction is introduced.

Money follows [the supplied ADR](https://github.com/constructorfabric/gears-rust/blob/510ce99f037a19f63d41b17acb3b0680e9331a44/gears/bss/docs/ADR/0001-cpt-cf-bss-adr-exact-decimal-boundaries.md): major units, exact textual numeric forms, currency and stored scale; exact fractions where required; published bounds; one invoice-line rounding by Billing and no second rounding by Ledger. Numerical business examples omit taxes and FX. The existing Rating design already adopts these rules. Orders' old minor-unit DTO/storage and Workflow's approval TCV still need versioned owner alignment. That is a discovered integration dependency, not a request for historical data migration.

## Delivery order and scope limits

Begin local Subscriptions foundation/facts/recovery as in [DECOMPOSITION](../DECOMPOSITION.md#3-dependency-order-and-delivery-gates). In parallel, ratify AP/FA across Orders, Workflow, Pricing/Products and the selected financial owner. Agree the common fact/result/numeric and invoice-line contracts across Subscriptions, Rating, Billing and Ledger before monetary enablement. Free-period trial requires Pricing trial terms; credit additionally requires Billing promotional coverage. Enable renewal/offboarding and usage profiles only with their applicable Contracts/execution/evidence providers and conformance tests.

Keep migration excluded and import, audit subsystem, notifications, transfer, ramps/co-termination, administrative backdating, trial extension/repeat, standalone acquisition, base-plan changes, scheduled composition/resume and linked replacement deferred. Immediate increases/additions consume the separate Change Orders contract; its authoring is not part of this work. Native LM/OSS PRDs are not prerequisites. Required unavailable notice or unsupported allocation still blocks the affected action/profile.

## Acceptance record

For each component record the ratified version, concrete owner mapping, supported/refused profiles, executed AP/FA/TP/RB/RO cases and remaining integration gates. Existing behavior marked “already specified” still requires runtime evidence. Source review and document validation do not close provider conformance. Do not mark the whole design unimplemented merely because one optional profile is gated, or mark a gated profile ready because its prose is complete.
