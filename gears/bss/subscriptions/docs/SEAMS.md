---
description: "Subscription ownership boundaries, external integration proposals and provider gates."
---

# Subscriptions — historical seam address index

**Additional scope reduction — user decision, 2026-10-15.** Defer trial extension and repeat trials, standalone acquisition without Orders, scheduled composition changes and scheduled resume, linked predecessor/successor subscription replacement, and changes of an existing subscription's base plan. Reject these operations before effects, including through operator/direct wrappers; earlier procedures and decisions for them are future traceability only. New paid and trial subscriptions originate through Orders. Existing-subscription lifecycle operations, supported immediate quantity increases/component additions, future order starts, cancellation scheduling, renewal and normal trial timers remain. Technical retry/successor attempts for the same acquisition remain recovery, not linked replacement of an existing subscription.

Free-period and credit trials, normal expiry/conversion and early conversion to the already accepted paid target remain in scope with their Pricing/Billing/financial gates. Trial-to-paid conversion is not a general changePlan capability. Preserve trial history/eligibility evidence and refuse repeat requests for the same payer/plan, including via a new subscription; a transport retry is not a repeat trial. No extension or repeat becomes available from a Pricing policy alone. Independent cancellation and new purchase have separate resource/data lifecycles: there is no automatic resource adoption, data transfer, place handover or continuity promise. Cancellation retains its existing retention/disposition rules; it does not mean immediate deletion. Unsupported plan changes cannot silently become cancel+new.


<!-- toc -->

- [A. Rating (evaluation core + pipeline)](#a-rating-evaluation-core--pipeline)
- [B. Pricing (Product Catalog)](#b-pricing-product-catalog)
- [C. Contracts & Agreements](#c-contracts--agreements)
- [D. Billing & Invoicing](#d-billing--invoicing)
- [E. Policy Engine & OSS Provisioning](#e-policy-engine--oss-provisioning)
- [F. Payments & Notifications](#f-payments--notifications)
- [G. Catalog Registry (Product & SKU)](#g-catalog-registry-product--sku)
- [H. Naming / cross-cutting](#h-naming--cross-cutting)
- [I. Orders (Lifecycle + Workflow)](#i-orders-lifecycle--workflow)
- [J. License Manager (issuance seam, added 2026-10-02)](#j-license-manager-issuance-seam-added-2026-10-02)
- [Ownership matrix (contested / adjacent responsibilities)](#ownership-matrix-contested--adjacent-responsibilities)
- [Decisions and provider gates](#decisions-and-provider-gates)

<!-- /toc -->

**Provider abstraction, 2026-10-14:** historical LM/OSS labels below identify the original integration sources. The current design uses replaceable entitlement fulfillment, resource execution and inventory/evidence adapters; native provider APIs are deferred to integration specifications. See [DESIGN](DESIGN.md#11-architectural-vision).

This file preserves the SUB-/CHG- addresses cited by other gears. Current obligations
and provider status live in [UPSTREAM_REQS](UPSTREAM_REQS.md), architecture in
[DESIGN](DESIGN.md), and decisions in [DECISIONS](DECISIONS.md). It is not a second
normative seam specification. [RECONCILIATION](RECONCILIATION.md) records source
versions, conflicts and proposed resolutions. A TARGET entry is not provider delivery
or bilateral acceptance. Historical subject labels below do not restore removed
Pricing phases/cohorts, direct entitlement checks or old one-time dedup rules.

## A. Rating (evaluation core + pipeline)

| Stable address | Historical subject | Current authority / disposition |
| --- | --- | --- |
| **SUB-R1** | Composition read-model + change boundary | [02-composition.md](features/02-composition.md); TARGET accepted composition and boundary; consumer adapter S-U08 |
| **SUB-R2** | Snapshot `(currency, region)` segment | [02-composition.md](features/02-composition.md); Old snapshot segment HISTORICAL; accepted bindings replace it, S-U08 |
| **SUB-R3** | Seat-count provenance + mid-period seat boundary | [03-changes.md](features/03-changes.md); TARGET committed quantity intervals; Rating owns arithmetic |
| **SUB-R4** | Phase boundary = change boundary | [06-trials.md](features/06-trials.md); Pricing phases HISTORICAL; S-Q06 settlement-scheme and prepay-failure default directions accepted, trial source and remaining contracts JOINT OPEN S-Q06/S-U12 |
| **SUB-R5** | Brand context source | [02-composition.md](features/02-composition.md); Per-sale brand metadata DEFERRED; accepted price/terms provenance remains |
| **SUB-R6** | Recurring pricing enrichment | [08-commercial-facts.md](features/08-commercial-facts.md); TARGET money-free parent facts; Rating monetary evaluation |
| **SUB-R7** | Usage attribution | [09-consumer-contracts.md](features/09-consumer-contracts.md); TARGET attribution projection and recovery; inventory S-U02 |
| **SUB-R8** | Co-termination proration | DEFERRED; no current delivery gate. Ordinary Rating proration for retained operations remains. |

## B. Pricing (Product Catalog)

| Stable address | Historical subject | Current authority / disposition |
| --- | --- | --- |
| **SUB-P1** | Plan-change classification contract | Base-plan changes DEFERRED. Immediate component additions/increases retain accepted Pricing compatibility and resulting-composition validation. |
| **SUB-P2** | Phase → grant-set map | [05-entitlements.md](features/05-entitlements.md); Old phase/grant map HISTORICAL; desired posture and License Manager source EXTERNAL OPEN S-U04 |
| **SUB-P3** | Trial sellable definition | [PlanRevision trial contract](TRIAL_PRICING_PROPOSAL.md); optional free-period OR credit direction ACCEPTED 2026-10-08. No separate offer catalog; exact revision/receipt/fulfilment S-U12 and shared credit S-U15 OPEN. Products has no trial-policy fields; V1 unchanged |
| **SUB-P4** | Prepaid credit grant | [05-entitlements.md](features/05-entitlements.md); existing paid-wallet/prepaid-grant scope remains separate and gated. Promotional trial-credit accounting/correction belongs to Billing by accepted direction; post-expiry correction preserves original expiration and zero spend permission without trial extension, and correction cannot automatically reopen an exhausted closed episode or reverse paid conversion; early guarded conversion retains eligible unused credit on the same grant under original expiry/restrictions; exact provider/Ledger integration and other disposition contracts OPEN S-U15; no current PriceBook grant capability asserted |
| **SUB-P5** | Sellability / publish gate | [09-consumer-contracts.md](features/09-consumer-contracts.md); SellabilityV1 provider CURRENT; Subscriptions adapter TARGET; activation S-Q01 |
| **SUB-P6** | Grandfathering eligibility expiry — inbound signal + outbound feedback | [02-composition.md](features/02-composition.md); Grandfathering signal HISTORICAL; PriceBook pin/ends_on replaces eligibility expiry |
| **SUB-P7** | Plan-migration execution handshake — this gear executes, and until now no seam recorded it | [03-changes.md](features/03-changes.md); Pricing migration delivery DEFERRED (D-410); no assumed producer |
| **SUB-P8** | Per-scope-key in-flight-subscription presence — pricing §9.2 inbound lane 3 | [09-consumer-contracts.md](features/09-consumer-contracts.md); Old per-scope retirement/presence protocol HISTORICAL; no D-510 method supplies it |
| **SUB-P9** | `billingAnchorPolicy` + the month-end clamp + the joint anchor fixture | [04-renewal-retention.md](features/04-renewal-retention.md); Anchor ownership CONFIRMED SUB-D-27; accepted BillingTerms adapter TARGET, S-U06 |

## C. Contracts & Agreements

| Stable address | Historical subject | Current authority / disposition |
| --- | --- | --- |
| **SUB-C1** | Renewal / grace ladder / regional templates | [04-renewal-retention.md](features/04-renewal-retention.md); TARGET renewal/grace terms; provider S-U06 |
| **SUB-C2** | Ramps (committed multi-step schedules) | [03-changes.md](features/03-changes.md); Contracts ramps and multi-date schedules DEFERRED; no current S-U06 ramp gate |
| **SUB-C3** | Commitment pools | [09-consumer-contracts.md](features/09-consumer-contracts.md); Commitment-pool application outside initial profile; explicit Contracts/Rating follow-up |
| **SUB-C4** | Activation date-trio + acceptance | [01-foundation.md](features/01-foundation.md); Date-trio preserved; S-Q01 direction accepted; activation contracts JOINT OPEN |
| **SUB-C5** | `PriceOverride` windows | [09-consumer-contracts.md](features/09-consumer-contracts.md); Legacy override model HISTORICAL; unsupported overlays DEFERRED |

## D. Billing & Invoicing

| Stable address | Historical subject | Current authority / disposition |
| --- | --- | --- |
| **SUB-B1** | Recurring idempotency + no-retro-edit + traceability — and the recurring WHEN owner | [08-commercial-facts.md](features/08-commercial-facts.md); TARGET immutable per-component facts; Billing consumer S-U13 |
| **SUB-B2** | `collectionPaused` artifact treatment | [04-renewal-retention.md](features/04-renewal-retention.md); Collection pause/resume DEFERRED; ordinary facts and financial admission remain |
| **SUB-B3** | Period floor/cap + rounding execution | [08-commercial-facts.md](features/08-commercial-facts.md); Rating minimum-fee/proration, Billing rounding; legacy Billing floor ownership HISTORICAL |
| **SUB-B4** | Prepaid drawdown + tax placement | [05-entitlements.md](features/05-entitlements.md); Prepaid balance path DEFERRED; no implied Billing or license provider |
| **SUB-B5** | Dunning handoff | [04-renewal-retention.md](features/04-renewal-retention.md); TARGET failure/grace handoff; Payments S-U07, Billing S-U13 |
| **SUB-B6** | Posted-period watermark for the backdating guard | [03-changes.md](features/03-changes.md); Administrative backdating DEFERRED; S-U14 is not a current gate |
| **SUB-B7** | Mid-term cancellation money path | [08-commercial-facts.md](features/08-commercial-facts.md); TARGET recurring and usage commercial facts, versions and seals; S-U13 |
| **SUB-B8** | One-time / setup charge valuation | [08-commercial-facts.md](features/08-commercial-facts.md); S-Q03 Rating tariff-valuation direction ACCEPTED; Billing accounts/posts, applies advances and owns final invoice rounding/corrections. S-Q04 charge-action identity direction ACCEPTED; exact target schema and S-U08/S-U13 owner/runtime evidence remain OPEN |

## E. Policy Engine & OSS Provisioning

| Stable address | Historical subject | Current authority / disposition |
| --- | --- | --- |
| **SUB-E1** | Fail-closed Policy gate | [01-foundation.md](features/01-foundation.md); Policy gate required; external observations never replace commit fences |
| **SUB-E2** | OSS provisioning confirmation | [04-renewal-retention.md](features/04-renewal-retention.md); Non-destructive pause TARGET; resource stop/purge provider S-U05 |
| **SUB-E3** | Entitlement enforcement split | [05-entitlements.md](features/05-entitlements.md); Old direct check/counter owner HISTORICAL; LM/platform handoff S-U04 |
| **SUB-E4** | Resource and data disposition (added 2026-10-01) | [04-renewal-retention.md](features/04-renewal-retention.md); Retention/purge TARGET; S-Q05 post-stop and confirmed cancellation charging-cutoff directions ACCEPTED, internal stops split by cause and v1 retention defaults ACCEPTED 2026-10-08; confirmation protocol and financial/stop/correction owner/runtime evidence OPEN S-U05/S-U13 |
| **SUB-E5** | Resource adoption on import (added 2026-10-01) | [07-tenancy-import.md](features/07-tenancy-import.md); Import/adoption DEFERRED; S-U09 is not a current gate |

## F. Payments & Notifications

| Stable address | Historical subject | Current authority / disposition |
| --- | --- | --- |
| **SUB-F1** | Financial admission and later outcomes | TARGET S-U07; authoritative operation/terms-bound evidence, pending/refused handling and recovery. No notification delivery dependency. |
| **SUB-F2** | Notification triggers | DEFERRED; no notification scheduling/delivery work. Existing domain events remain for future consumers. |

## G. Catalog Registry (Product & SKU)

| Stable address | Historical subject | Current authority / disposition |
| --- | --- | --- |
| **SUB-G1** | `overlapScopeKey` binds to a registry key | [03-changes.md](features/03-changes.md); S-Q02 core key/limit, retained places and limit-decrease preservation ACCEPTED: keep protected active/suspended/reserved claims, reject excess new admissions; failed/unsupported migration preserves the source. Owner provenance, reservation/count, current key/policy contract and runtime evidence JOINT OPEN S-U10 |
| **SUB-G2** | Published `skuId` / `PlanTier` / `CatalogVersion` | [02-composition.md](features/02-composition.md); SKU/version via accepted PriceBook binding; CatalogVersion/PlanTier assumptions HISTORICAL |

## H. Naming / cross-cutting

| Stable address | Historical subject | Current authority / disposition |
| --- | --- | --- |
| **SUB-N1** | New `TransitionRequest.type` values + scheduled-intent envelope | [DESIGN.md](DESIGN.md); TARGET; shared naming/manifest alignment remains explicit |
| **SUB-N2** | Gear-name pinning | [DESIGN.md](DESIGN.md); TARGET; shared naming/manifest alignment remains explicit |

## I. Orders (Lifecycle + Workflow)

<a id="i-orders-lifecycle--workflow--change-orders"></a>

| Stable address | Historical subject | Current authority / disposition |
| --- | --- | --- |
| **SUB-O1** | Cancellation reason for order-fulfilment compensation | [01-foundation.md](features/01-foundation.md); TARGET compensation reason/outcome; monetary disposition S-Q03/S-Q05/S-U13 |
| **SUB-O2** | External order reference on `create` + idempotency-key derivation | [01-foundation.md](features/01-foundation.md); TARGET order/attempt/line identity and create settlement |
| **SUB-O3** | `draft → activate` MUST remain an externally callable two-phase pair | [01-foundation.md](features/01-foundation.md); Two-phase pair preserved; shared activation protocol S-Q01 |
| **SUB-O4** | Sellability gate as a pre-purchase check for a caller that is not yet creating a subscription | [09-consumer-contracts.md](features/09-consumer-contracts.md); Orders consumes CURRENT SellabilityV1; dated preview is not acceptance |
| **SUB-O5** | `overlapScopeKey` presence read for order-time conflict detection | [01-foundation.md](features/01-foundation.md); core/suspension/prepayment directions ACCEPTED. Distinct active/retained/order-reserved counts share capacity; confirmation reserves before capture, activation consumes its own place. Plain unconfirmed draft gives no automatic claim. Counts, paid-right TTL/release/capture fencing and Pricing future commitment require versioned S-U01/S-U07/S-U10 alignment; technical-attempt settlement and supported policy mappings remain; linked replacement and legacy migration are excluded |
| **SUB-O6** | Atomic multi-subscription submission is cheaper than the deferral assumed | [03-changes.md](features/03-changes.md); Single-subscription changeComposition TARGET; distributed multi-subscription atomic commit DEFERRED |
| **SUB-O10** | Subscription start instant | [01-foundation.md](features/01-foundation.md); S-Q01 T and bounded final permit remain accepted; subsequent paid-future-order direction requires a separate durable Pricing/Products commitment before capture. Versioned authority/time/capture contract JOINT OPEN |
| **SUB-O11** | Machine-readable in-flight signal | [01-foundation.md](features/01-foundation.md); TARGET same-key replay and explicit competing-operation refusal |
| **SUB-O12** | Withdrawal of an accepted request | [01-foundation.md](features/01-foundation.md); TARGET withdrawal with durable external-effect settlement |
| **SUB-O13** | Status read of a non-terminal request | [01-foundation.md](features/01-foundation.md); TARGET recoverable operation status; absence alone is not settled create |
| **SUB-O14** | `correlationId` propagation | [01-foundation.md](features/01-foundation.md); TARGET propagation of correlation to all external legs |
| **SUB-O15** | Latency and throughput budget for the intent seam | [PRD](PRD.md); intent budget ratified as target v1 under S-Q07 (2026-10-08); measurement OPEN S-U11 |
| **SUB-O16** | Identity-envelope echo | [01-foundation.md](features/01-foundation.md); TARGET operation/order/attempt envelope echo |

SUB-O7, SUB-O8 and SUB-O9 were not allocated in the Subscriptions register. Historical Workflow references must be resolved by subject to correlation, request outcome and occupancy requirements; those numbers are not silently reassigned.

| Stable address | Subject | Current authority |
| --- | --- | --- |
| **CHG-S1** | Order reference on quantity and composition changes | [Composition changes](features/03-changes.md), `upreq-composite-change`; multi-date requests deferred |
| **CHG-S2** | Transactional multi-item change | [Composition changes](features/03-changes.md), `upreq-composite-change`; multi-date requests deferred |
| **CHG-S3** | Overlap self-exemption | [Composition changes](features/03-changes.md), `upreq-composite-change`; multi-date requests deferred |
| **CHG-S4** | Composition read with revision | [Composition changes](features/03-changes.md), `upreq-composite-change`; multi-date requests deferred |
| **CHG-S5** | Requested effective date | [Composition changes](features/03-changes.md), `upreq-composite-change`; multi-date requests deferred |

## J. License Manager (issuance seam, added 2026-10-02)

| Stable address | Historical subject | Current authority / disposition |
| --- | --- | --- |
| **SUB-L1** | Issuance request | [features/05-entitlements.md](features/05-entitlements.md); TARGET; external provider S-U04 |
| **SUB-L2** | License state and findings back | [features/05-entitlements.md](features/05-entitlements.md); TARGET; external provider S-U04 |
| **SUB-L3** | Trials, suspension and conversion map to License Manager verbs | [features/05-entitlements.md](features/05-entitlements.md); TARGET; external provider S-U04 |
| **SUB-L4** | `license-enforcement` actor naming | [features/05-entitlements.md](features/05-entitlements.md); TARGET; external provider S-U04 |

## Ownership matrix (contested / adjacent responsibilities)

| Concern | Target owner | Outstanding agreement |
| --- | --- | --- |
| Commercial state, boundaries, accepted terms and facts | Subscriptions | S-Q01 direction and S-Q04 charge-action identity accepted; detailed owner/target-schema evidence still required |
| Authoring and accepted Pricing inputs | Pricing; Products for SKU identity/structure only | Initial trial eligibility and immutable variant/paid-target terms remain S-U12. Extension/repeat/additional-bonus/transfer/stacking are unavailable; early-paid carryover is preserved. |
| Shared promotional trial-credit accounting and allocation | Billing ownership direction accepted | S-U15 provider/Ledger integration OPEN: full Rating replacements, prior allocation/version lineage, valid-grant restoration once, standard draft/posted correction. Original chronological expiration is preserved: corrected historical balance is unavailable to spend after expiry, with no trial extension. Correction cannot automatically reopen a trial already ended by exhaustion, even before original expiry, grant service/spend permission for that episode or reverse applied paid conversion. Early valid guarded conversion preserves unused credit on the same grant for eligible paid expenses under original expiry/currency/product/item/charge-kind restrictions, without new grant/term or assumed upfront-payment satisfaction. V1 has no reactivation route, rollover or regrant; stacking is refused and transfer is deferred (accepted 2026-10-08). Eligible accepted-trial value, including positive corrections and delay/error overrun, served before the applied paid boundary is platform-borne beyond bounded bonus; actual post-boundary residual follows accepted paid terms, regardless of arrival time. Exact boundary evidence/coverage/enforcement and other liability cases remain open; existing cash-funded Ledger wallet is not a bonus API |
| Tariff valuation, proration and hourly scheduling | Rating | S-Q03 one-time direction accepted; routing/binding/replay contracts S-U08/S-U13 open |
| Invoice accounting/posting, advance application and final invoice rounding/corrections | Billing | Replacement/seal and prepayment/replay contracts S-U13; taxes/payments retain their respective owners |
| Attribution and ownership scopes | Subscriptions | IRM history S-U02; usage coverage is separate S-U03 |
| Entitlement posture | Subscriptions | Materialization/enforcement handoff S-U04 |
| License/pack materialization | Selected entitlement fulfillment adapter | External owner contract S-U04 |
| Resource lifecycle and retained data execution | Selected resource execution adapter | S-Q05 post-stop and confirmed charging-cutoff directions accepted; separate financial confirmation/actual offboarding stop, correction and retention/purge contracts OPEN S-U05/S-U13; internal stops split by cause accepted 2026-10-08 |

<a id="decisions-register-to-close-before-design-lock"></a>

## Decisions and provider gates

See [DECISIONS](DECISIONS.md#open-cross-owner-decisions) for S-Q01–S-Q07 and
[UPSTREAM_REQS](UPSTREAM_REQS.md#25-provider-prerequisites-and-bilateral-acceptance)
for S-U01–S-U15. Historical SUB-D identifiers remain in DECISIONS with explicit
current dispositions. Provider and consumer readiness are never inferred from a
legacy ALIGNED label.
