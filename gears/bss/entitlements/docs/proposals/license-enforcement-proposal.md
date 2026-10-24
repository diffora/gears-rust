---
description: "Provider-independent grant application and recovery that Entitlements requires from the selected license adapter (LE, native LM or third party)."
---

# License enforcement proposal

**Status:** Entitlements integration proposal, 2026-10-23. This is an owner handoff to License Enforcement and to any other selected license adapter, not a delivered API or an external-owner approval. Examples are illustrative, not new tariff, duration or SLA defaults. See the [proposal index](README.md) for source precedence, scope exclusions and delivery order. The asks close gate E-U02 in [UPSTREAM_REQS.md](../UPSTREAM_REQS.md).

<!-- toc -->

- [Baseline and deployment choice](#baseline-and-deployment-choice)
- [Required contract](#required-contract)
- [Cases](#cases)
- [Acceptance and boundaries](#acceptance-and-boundaries)

<!-- /toc -->

## Baseline and deployment choice

The [adapter capability profile](../DESIGN.md#333-adapter-capability-profile) and the [license grants feature](../features/04-license-grants.md) define desired grant state separately from actual access. License Enforcement, a native License Manager, a third-party licensing system or a customer adapter may implement this role. No specific License Manager API or native PRD is required to complete the core design. The [License Enforcement PRD](../../../../system/license-enforcement/docs/PRD.md) names grants, holders, subject bindings and the lifecycle verbs accept, bind, delegate, return, recall, suspend, revoke and renew; the [License Resolver](../../../../system/license-resolver/docs/DESIGN.md) is a read and check boundary and owns no grant store. The asks below are written against those shapes; the concrete mapping is the adapter's work.

## Required contract

| Request | Adapter guarantee |
| --- | --- |
| LE-P01 Correlated application | Accept desired grant state by scope, assignment identity, desired revision and generation; return applied, pending, refused or unknown with authoritative effective time where known. The adapter neither receives nor reports a subscription identifier. Initial readiness observations use the separate readiness envelope of [feature 02](../features/02-initial-readiness.md) (schema version, service profile and target references, producer authority, status sequence; states `pending`, `ready`, `failed`, `withdrawn`). |
| LE-P02 Stale-action suppression | A delayed issue or resume must not undo a newer revoke or suspend; enforce ordering at the effect boundary, not only when reading a callback. |
| LE-P03 Idempotent recovery | Read or replay the same application operation after a lost response; do not create another grant on retry. |
| LE-P04 Preserve posture | Preparation grants are issued while the subscription is draft under ENT-D-02; `active` is not the trigger. They are reported as readiness observations, and Subscriptions commits active only after every assignment in the persisted manifest reports `ready` for the exact desired revision. Increases and additions while suspended are refused by Subscriptions before any posture reaches the adapter; a separate guarded resume comes first. Cancellation and trial outcomes materialize their accepted posture; delayed effects of an earlier committed change cannot undo a newer restriction. |
| LE-P05 Grant evidence dependency | Apply credit-dependent grants only after the posture carries the required grant evidence; the adapter holds no trial, expiry or benefit policy. |

## Cases

| Case | Situation | Required behavior |
| --- | --- | --- |
| LE-C01 Old issue after revoke | Generation 8 of an assignment revokes access. A delayed generation 7 issue request arrives afterwards. | The adapter rejects or reconciles the generation 7 request without restoring access. Ignoring its late success callback is insufficient if the adapter already granted access; the fence is at the effect boundary (LE-P02). |
| LE-C02 Ongoing change commits, adapter times out | An active subscription applies a composition change; Entitlements derives the new set and the adapter call times out. This is not an initial activation. | Entitlements recovers the same application operation and shows pending access; it does not fabricate readiness and Subscriptions does not repeat activation. The adapter holds one grant for the assignment after recovery (LE-P03). |
| LE-C03 Early paid conversion | The customer converts to the already accepted paid target while trial credit remains. | The paid grant set is applied; the retained trial grant keeps its original expiry. Later expiry removes the bonus coverage only; paid access follows its own financial and lifecycle rules (LE-P05). |
| LE-C04 Lost ready event | The adapter prepared an assignment for an initial activation and emitted `ready`, but the event never reached Entitlements. | The assignment status read returns the same `ready` observation with its `statusSequence`; Entitlements recovers it without a second grant, and the manifest becomes ready for the exact desired revision (LE-P01, LE-P03). |
| LE-C05 Ready racing withdrawal | Subscriptions withdraws the activation while the adapter's `ready` for the same assignment is in flight. | The withdrawal fences the operation first. The late `ready` is stored as evidence for cleanup and cannot activate; the adapter revokes or supersedes the preparation grant on the withdrawal command and does not recreate it from the in-flight work (LE-P02, LE-P04). |
| LE-C06 Wrong-tenant observation | An observation arrives with a `scopeId` that does not match the manifest's authorized scope for the assignment, or from a producer not named in the manifest. | Entitlements refuses or quarantines it with a structured reason and no readiness effect; the adapter must report observations only under the scope and producer authority agreed for the assignment (LE-P01). |

## Acceptance and boundaries

Select the deployment capability profile and map E-U02 to a concrete adapter. Execute the six cases above with lost replies and reordered actions, and the readiness fixtures of the [initial readiness feature](../features/02-initial-readiness.md). A genuinely inapplicable license capability differs from an unavailable required adapter. Seat binding, counting, leases and point-of-use checks remain with the enforcement owner; no local license engine, customer notification feature or native License Manager mandate is added. Transport of the rights feed, native identity mapping, freshness versus withdrawal arbitration, cleanup policy and preactivation exposure stay open under ENT-Q09 to ENT-Q13 in [DECISIONS.md](../DECISIONS.md#open-questions).
