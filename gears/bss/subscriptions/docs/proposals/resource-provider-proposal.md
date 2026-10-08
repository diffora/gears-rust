---
description: "Provider-independent stop, retention, hold and deletion evidence required for subscription offboarding."
---

# Resource provider proposal

**Status:** Subscriptions integration proposal, 2026-10-15. This is an owner handoff, not a delivered API or external-owner approval. Examples are illustrative, not new tariff, duration or SLA defaults. See the [proposal index](README.md) for source precedence, scope exclusions and delivery order.

<!-- toc -->

- [Baseline and ownership](#baseline-and-ownership)
- [Required contract](#required-contract)
- [Examples](#examples)
- [Acceptance and exclusions](#acceptance-and-exclusions)

<!-- /toc -->

## Baseline and ownership

Use the [RO integration contract](../features/04-renewal-retention.md#7-renewal-and-offboarding-integration-contract). Subscriptions owns commercial intent, confirmed charging cutoff and disposition eligibility; the selected resource provider owns execution and authoritative physical outcomes. Inventory/evidence is a separate role even when the same deployment service supplies both.

## Required contract

| Request | Provider guarantee |
| --- | --- |
| RS-P01 Physical outcome | Correlate readiness/stop/delete to operation and immutable resource generation, with actual effective time and explicit pending/unknown/partial outcomes |
| RS-P02 Non-destructive suspension | Preserve data and implement the accepted restricted posture; a suspend command is not deletion |
| RS-P03 Retention/export | Apply the frozen accepted disposition terms and expose required read-only/export capability; retained data does not itself create ordinary service charges |
| RS-P04 Hold/delete arbitration | Confirm hold protection only with execution-boundary arbitration or acknowledged fencing that prevents a stale delete; local receipt alone is insufficient |
| RS-P05 Recovery and generations | Retry by original identity, report per-resource partial completion and never apply an old action to a replacement resource with a recycled name |

## Examples

**Stop is late.** Confirmed charging cutoff is 10:00, but the provider stops service at 10:05. Preserve the actual 10:05 stop; ordinary customer tariff ends at the valid cutoff. The provider must not backdate its outcome to hide the technical delay.

**Hold races deletion.** If the provider acknowledges the hold before irreversible deletion wins, the old delete must not execute. If deletion already won, report its true in-progress/partial/applied state; do not claim the data is protected or restored. Preserve any still-existing scope and reconcile.

**Resource name reused.** A deletion task targets generation A of a resource name. A later independent purchase creates generation B with the same name. The delayed task must not delete B. Cancel/new does not automatically adopt A's data or promise continuity.

## Acceptance and exclusions

Ratify S-U05 adapter mapping and relevant RO cases. Automatic deletion stays disabled without the proven hold/execution fence. Hold release needs a fresh eligible schedule and required notice evidence; no old expired deadline is reused. Notification delivery, import/adoption and subscription replacement are deferred. Archive completion is not evidence that financial settlement or record purge has occurred.
