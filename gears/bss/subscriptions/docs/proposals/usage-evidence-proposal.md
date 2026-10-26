---
description: "Independent historical inventory, attribution and usage-completeness contracts for subscription usage billing."
---

# Usage and inventory evidence proposal

**Status:** Required integration contract; owner agreement and executed conformance are required before enablement. See the [proposal index](README.md) for scope and common rules.

<!-- toc -->

- [Baseline and separate owners](#baseline-and-separate-owners)
- [Required contracts](#required-contracts)
- [Examples](#examples)
- [Delivery and acceptance](#delivery-and-acceptance)

<!-- /toc -->

## Baseline and separate owners

[Subscriptions recovery](../features/09-consumer-contracts.md) supplies commercial attribution and authorized scope. A deployment-selected inventory owner supplies historical resource membership/lifecycle barriers. Usage emitters supply coverage/finalization declarations; a collector supplies recorded usage and its recovery position. [Rating](rating-proposal.md#valuation-and-completeness) owns expected hourly geometry and verifies that these proofs are compatible. These capabilities are separate. They do not require our native IRM or OSS.

## Required contracts

| Request | Owner | Required evidence |
| --- | --- | --- |
| UE-P01 Historical inventory | Inventory/resource evidence provider, S-U02 | Immutable authorized snapshots including deleted resources and chargeable intervals, with lifecycle barrier and explicit gap outcomes |
| UE-P02 Source coverage | Usage emitter and Rating, S-U03 | Coverage for the exact interval/source scope and finalization policy; elapsed time or no rows is not completeness |
| UE-P03 Recorded usage recovery | Selected collector | Bounded exact quantities, durable identities/versions, recorded-through evidence and recoverable history with explicit loss |
| UE-P04 Attribution compatibility | Subscriptions and Rating | Scope proofs tied to fact version, inventory barrier and attribution sequence; consumer waits until its projection reaches that sequence |
| UE-P05 Adapter precision | Producer/collector/Rating | Published numeric limits and exact wire/storage behavior under [the money ADR](../../../docs/ADR/0001-cpt-cf-bss-adr-exact-decimal-boundaries.md); refuse unsupported precision rather than round silently |

## Examples

**Empty hour versus lost hour.** An inventory snapshot, completed emitter coverage and collector recorded-through evidence with no gap together prove zero usage quantity from 10:00 to 11:00: zero quantity is valid. Rating still evaluates applicable minimum fees before the result counts as zero-valued. If emitter coverage or the collector closure is missing, an empty collector response means pending evidence, not zero.

**Deleted resource, late usage.** A resource was deleted at noon and usage arrives later. Historical inventory and attribution still resolve the payer and accepted terms for the served interval. Current absence cannot erase the charge history or certify a deletion operation's outcome.

**Month completeness.** A full October UTC usage parent has 744 expected hourly windows. One missing ownership or source-coverage proof prevents complete finalization even if all other windows have rated results. A later correction is discovered through recovery feeds, independently of whether the original billing period is still due.

## Delivery and acceptance

Execute RB usage cases and V07/V09/V10 with the selected adapters. Keep source loss, unknown ownership, no resources and proven zero distinct. Only usage-enabled profiles require these proofs; a non-usage product does not acquire an hourly-data dependency. Legacy import, administrative historical editing and unsupported pooled allocation across differently priced purchase lots remain excluded.
