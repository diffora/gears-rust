---
description: "Immutable accepted-term evidence and renewal behavior required from Contracts, with no new ramp or notice implementation."
---

# Contracts proposal for Subscriptions

**Status:** Required integration contract; owner agreement and executed conformance are required before enablement. See the [proposal index](README.md) for scope and common rules.

<!-- toc -->

- [Baseline and retained behavior](#baseline-and-retained-behavior)
- [Required changes](#required-changes)
- [Examples](#examples)
- [Acceptance and exclusions](#acceptance-and-exclusions)

<!-- /toc -->

## Baseline and retained behavior

The local [Contracts PRD](../../../contracts/docs/PRD.md) defines the provider baseline. Subscriptions consumes an explicitly bound accepted version and evaluated commercial policy; a Contract is optional where accepted platform defaults apply. Contract authoring, legal acceptance and the source of terms remain with their owners.

## Required changes

| Request | Required provider evidence or rule |
| --- | --- |
| CT-P01 Versioned evaluation | Return the bound contract version, parties/applicability and effective policy evidence; Subscriptions freezes the evaluated term, grace, dwell and retention parameters |
| CT-P02 Running-term stability | Amendment, contract expiry or a new default cannot rewrite an already running term or its recorded deadlines |
| CT-P03 Next-renewal routing | Supply an explicitly bound successor or the evidence for the accepted versioned-default route, including required notice/assent prerequisites |
| CT-P04 Typed unavailability | Distinguish no applicable contract from provider outage, denial or incomplete evidence; unavailable required evidence is not permission to fall back |
| CT-P05 No implicit subscription command | A signed/changed Contract event does not create, bind, activate or change a subscription by itself |

## Examples

**Contract expires during a paid term.** A term runs through 31 October and its governing Contract expires on 15 October. The accepted running term finishes unchanged. At the next renewal, use an explicitly bound successor or the accepted default route with all required prerequisites; do not terminate or reprice the current term on 15 October.

**Defaults change during grace.** A grace episode already has its evaluated start and deadline. Publishing different platform defaults does not restart or shorten that episode. A later eligible episode evaluates its own version.

**Contract service is unavailable.** A renewal cannot read its required bound terms. It stays pending/blocked under the applicable policy. The outage does not make the customer uncontracted or permit a fallback to defaults.

## Acceptance and exclusions

Ratify S-U06 input versions and execute [RO-C01 through RO-C16 as applicable](../features/04-renewal-retention.md#7-renewal-and-offboarding-integration-contract), especially term-end/default routing and unavailable evidence. Ramps, co-termination, separate post-start acceptance confirmation and notification delivery remain deferred. An unavailable required notice still blocks the dependent action. No Contracts-driven base-plan replacement is added.


Required term evidence includes authorized scope, immutable version/digest, applicability/effective intervals and successor disposition. Initial uncontracted term shape comes from accepted Orders evidence; a Contract row or signed event alone creates no binding.

Events announce immutable source versions and recovery references. Consumers recover those exact terms and evaluate them at a guarded renewal boundary; replay cannot renew, cancel or rewrite running deadlines. Specify next-renewal successor/default routing explicitly. This proposal enables neither migration nor live rebinding, and does not require implementing deferred notification delivery.
