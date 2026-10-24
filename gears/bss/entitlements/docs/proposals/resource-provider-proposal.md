---
description: "Provider-independent stop, retention, hold and deletion evidence that Entitlements requires from the selected resource adapter for subscription offboarding."
---

# Resource provider proposal

**Status:** Entitlements integration proposal, 2026-10-23. This is an owner handoff to the selected resource adapter, not a delivered API or an external-owner approval. Examples are illustrative, not new tariff, duration or SLA defaults. See the [proposal index](README.md) for source precedence, scope exclusions and delivery order. The asks close gate E-U05 in [UPSTREAM_REQS.md](../UPSTREAM_REQS.md).

<!-- toc -->

- [Baseline and ownership](#baseline-and-ownership)
- [Required contract](#required-contract)
- [Cases](#cases)
- [Acceptance and exclusions](#acceptance-and-exclusions)

<!-- /toc -->

## Baseline and ownership

Use the [RO integration contract](../../../subscriptions/docs/features/04-renewal-retention.md#7-renewal-and-offboarding-integration-contract) of Subscriptions and the [resource execution feature](../features/05-resource-execution.md). The requester of every work order is Entitlements; the commercial posture that drives it comes from Subscriptions. Subscriptions owns commercial intent, the confirmed charging cutoff and disposition eligibility; the selected resource adapter owns execution and the authoritative physical outcomes. Inventory and evidence is a separate role even when the same deployment service supplies both. The adapter receives the authorized scope, the resource generation and the operation identity, never a subscription identifier (ENT-D-01).

## Required contract

| Request | Adapter guarantee |
| --- | --- |
| RS-P01 Physical outcome | Correlate readiness, stop and delete to the operation and the immutable resource generation, with the actual effective time and explicit pending, unknown and partial outcomes. |
| RS-P02 Non-destructive suspension | Preserve data and implement the accepted restricted posture; a suspend command is not deletion. |
| RS-P03 Retention and export | Apply the frozen accepted disposition terms, expose the required read-only export capability and return the retention state with effective timing evidence; whether retained data is charged is decided by the financial owner from that evidence, not by the provider. |
| RS-P04 Hold and delete arbitration | Confirm hold protection only with execution-boundary arbitration or acknowledged fencing that prevents a stale delete; a local receipt alone is insufficient. |
| RS-P05 Recovery and generations | Retry by original identity, report per-resource partial completion and never apply an old action to a replacement resource with a recycled name. |

## Cases

| Case | Situation | Required behavior |
| --- | --- | --- |
| RS-C01 Stop is late | The confirmed charging cutoff is 10:00, but the adapter stops service at 10:05. | Preserve the actual 10:05 stop; the ordinary customer tariff ends at the valid cutoff, decided by Subscriptions and Billing. The adapter must not backdate its outcome to hide the technical delay (RS-P01). |
| RS-C02 Hold races deletion | A hold and a stale delete reach the adapter in either order. | If the adapter acknowledges the hold before irreversible deletion wins, the old delete must not execute. If deletion already won, report its true in-progress, partial or applied state; do not claim the data is protected or restored. Preserve any still-existing scope and reconcile (RS-P04). |
| RS-C03 Resource name reused | A deletion task targets generation A of a resource name. A later independent purchase creates generation B with the same name. | The delayed task must not delete B. Cancel followed by a new purchase does not adopt A's data or promise continuity (RS-P05). |
| RS-C04 Delete response lost or partial | The delete reply is lost, or only some of the resources of one order are deleted. | Entitlements recovers the original operation by identity and the adapter reports each resource with its generation; no false completion, no second delete and no deletion of a replacement resource (RS-P01, RS-P05). |

## Acceptance and exclusions

Ratify the E-U05 adapter mapping and the relevant RO cases of Subscriptions. Automatic deletion stays disabled without the proven hold and execution fence. Hold release needs a fresh eligible schedule and the required notice evidence from Subscriptions; no old expired deadline is reused. Notification delivery, import or adoption and subscription replacement are deferred. Archive completion is not evidence that financial settlement or record purge has occurred.
