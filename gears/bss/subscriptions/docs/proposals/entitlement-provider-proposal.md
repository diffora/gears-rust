---
description: "Provider-independent entitlement application and recovery required by the retained subscription lifecycle."
---

# Entitlement provider proposal

**Status:** Subscriptions integration proposal, 2026-10-15. This is an owner handoff, not a delivered API or external-owner approval. Examples are illustrative, not new tariff, duration or SLA defaults. See the [proposal index](README.md) for source precedence, scope exclusions and delivery order.

<!-- toc -->

- [Baseline and deployment choice](#baseline-and-deployment-choice)
- [Required contract](#required-contract)
- [Examples](#examples)
- [Acceptance and boundaries](#acceptance-and-boundaries)

<!-- /toc -->

## Baseline and deployment choice

The [abstract provider boundary](../DESIGN.md#11-architectural-vision) and [entitlement feature](../features/05-entitlements.md) define commercial desired state separately from actual access. A native License Manager, third-party service or customer adapter may implement this role. No specific LM API or native PRD is required to complete the core design.

## Required contract

| Request | Provider guarantee |
| --- | --- |
| EN-P01 Correlated application | Accept desired posture with subscription/operation identity and generation; return applied, pending, refused or unknown evidence with authoritative effective time where known |
| EN-P02 Stale-action suppression | A delayed issue/resume must not undo a newer revoke/suspend; enforce ordering at the effect boundary, not just when reading a callback |
| EN-P03 Idempotent recovery | Read/replay the same materialization operation after a lost response; do not create another entitlement on retry |
| EN-P04 Preserve posture | Draft edits grant no service; suspended composition changes preserve restriction; cancellation and trial outcomes materialize their accepted posture |
| EN-P05 Trial boundaries | Credit-dependent access waits for required grant evidence. Retained-grant expiry after paid conversion ends the benefit, not the paid entitlement |

## Examples

**Old activation arrives after cancellation.** Desired state generation 8 revokes access. A delayed generation 7 issue request must be rejected or reconciled without restoring access. Ignoring its late success callback is insufficient if the provider already granted access.

**Commercial commit succeeds, provider times out.** The subscription has an applied local outcome and pending materialization. Recover the same entitlement operation and show pending access; do not fabricate immediate service readiness or repeat subscription activation.

**Early paid conversion.** The customer converts to the already accepted paid target while trial credit remains. Later grant expiry removes bonus coverage only. Paid access follows its own financial/lifecycle rules.

## Acceptance and boundaries

Select the deployment capability profile and map S-U04 to a concrete adapter. Execute V11 and relevant AP/TP/RO cases with lost replies and reordered actions. A genuinely inapplicable entitlement capability differs from an unavailable required provider. Quota/license enforcement remains external; no local quota engine, customer notification feature or native LM mandate is added.
