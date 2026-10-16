---
description: "Independent historical inventory, attribution and usage-completeness contracts for subscription usage billing."
---

# Usage and inventory evidence proposal

**Status:** Subscriptions integration proposal, 2026-10-15. This is an owner handoff, not a delivered API or external-owner approval. Examples are illustrative, not new tariff, duration or SLA defaults. See the [proposal index](README.md) for source precedence, scope exclusions and delivery order.

<!-- toc -->

- [Baseline and separate owners](#baseline-and-separate-owners)
- [Required contracts](#required-contracts)
- [Examples](#examples)
- [Delivery and acceptance](#delivery-and-acceptance)

<!-- /toc -->

## Baseline and separate owners

[Subscriptions recovery](../features/09-consumer-contracts.md) supplies commercial attribution and authorized scope. A deployment-selected inventory owner supplies historical resource membership/lifecycle barriers. Usage emitters supply coverage/finalization declarations; a collector supplies recorded usage and its recovery position. [Rating](https://github.com/constructorfabric/gears-rust/blob/3a1cd8ac33bda1a72838a81435c7365a5d387d05/gears/bss/rating/docs/DESIGN.md) owns expected hourly geometry and verifies that these proofs are compatible. These are separate capabilities, not a requirement for our native IRM or OSS.

## Required contracts

| Request | Owner | Required evidence |
| --- | --- | --- |
| UE-P01 Historical inventory | Inventory/resource evidence provider, S-U02 | Immutable authorized snapshots including deleted resources and chargeable intervals, with lifecycle barrier and explicit gap outcomes |
| UE-P02 Source coverage | Usage emitter and Rating, S-U03 | Coverage for the exact interval/source scope and finalization policy; elapsed time or no rows is not completeness |
| UE-P03 Recorded usage recovery | Selected collector | Bounded exact quantities, durable identities/versions, recorded-through evidence and recoverable history with explicit loss |
| UE-P04 Attribution compatibility | Subscriptions and Rating | Scope proofs tied to fact version, inventory barrier and attribution sequence; consumer waits until its projection reaches that sequence |
| UE-P05 Adapter precision | Producer/collector/Rating | Published numeric limits and exact wire/storage behavior under [the money ADR](https://github.com/constructorfabric/gears-rust/blob/510ce99f037a19f63d41b17acb3b0680e9331a44/gears/bss/docs/ADR/0001-cpt-cf-bss-adr-exact-decimal-boundaries.md); refuse unsupported precision rather than round silently |

## Examples

**Empty hour versus lost hour.** An inventory snapshot and completed emitter coverage prove no billable resource usage from 10:00 to 11:00: zero is valid. If emitter coverage is missing, an empty collector response means pending evidence, not zero.

**Deleted resource, late usage.** A resource was deleted at noon and usage arrives later. Historical inventory and attribution still resolve the payer and accepted terms for the served interval. Current absence cannot erase the charge history or certify a deletion operation's outcome.

**Month completeness.** A full October UTC usage parent has 744 expected hourly windows. One missing ownership or source-coverage proof prevents complete finalization even if all other windows have rated results. A later correction is discovered through recovery feeds, independently of whether the original billing period is still due.

## Delivery and acceptance

Execute RB usage cases and V07/V09/V10 with the selected adapters. Keep source loss, unknown ownership, no resources and proven zero distinct. Only usage-enabled profiles require these proofs; a non-usage product does not acquire an hourly-data dependency. Legacy import, administrative historical editing and unsupported pooled allocation across differently priced purchase lots remain excluded.
