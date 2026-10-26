---
description: "Draft Billing architecture for exact financial targets, invoice corrections, execution recovery and trial coverage."
---

# Technical Design: Billing

**Status: draft.** This design covers the financial responsibilities required by the Subscriptions integration. Owner mapping, schemas, retention, tax treatment and unresolved allocation rules must be completed before implementation enablement.

- [ ] `p1` - **ID**: `cpt-cf-bss-billing-design-financial-execution`

<!-- toc -->

- [1. Architecture Overview](#1-architecture-overview)
  - [1.1 Architectural Vision](#11-architectural-vision)
  - [1.2 Architecture Drivers](#12-architecture-drivers)
  - [1.3 Architecture Layers](#13-architecture-layers)
- [2. Principles & Constraints](#2-principles--constraints)
  - [2.1 Design Principles](#21-design-principles)
  - [2.2 Constraints](#22-constraints)
- [3. Technical Architecture](#3-technical-architecture)
  - [3.1 Domain model](#31-domain-model)
  - [3.2 Component Model](#32-component-model)
  - [3.3 API Contracts](#33-api-contracts)
  - [3.4 Internal Dependencies](#34-internal-dependencies)
  - [3.5 External Dependencies](#35-external-dependencies)
  - [3.6 Interactions & Sequences](#36-interactions--sequences)
  - [3.7 Database schemas & tables](#37-database-schemas--tables)
- [4. Open Decisions](#4-open-decisions)
- [5. Traceability](#5-traceability)

<!-- /toc -->

## 1. Architecture Overview

### 1.1 Architectural Vision

Subscriptions owns economic occurrence, accepted commercial terms, actual service boundaries, facts and seals. Pricing owns tariff selection and publication. Rating values exact frozen inputs and publishes complete replacements. Billing owns invoice equivalence, financial targets, benefit/advance allocation, final rounding, draft/posting arbitration and financial corrections. Ledger and payment providers execute their own authorized operations.

Billing consumes the [Subscriptions fact/result contract](../../subscriptions/docs/DESIGN.md#rating-result-contract) and [commercial seals](../../subscriptions/docs/features/08-commercial-facts.md#36-seal-the-complete-commercial-set). Rating can publish a complete result while Billing's invoice-state projection is absent. That absence blocks unsafe financial execution, not tariff math.

### 1.2 Architecture Drivers

| PRD requirement | Design response |
| --- | --- |
| `cpt-cf-bss-billing-fr-complete-targets` | Exact input-cut validation and immutable target acceptance |
| `cpt-cf-bss-billing-fr-full-replacements` | Per-source contribution replacement and retained lineage |
| `cpt-cf-bss-billing-fr-invoice-rounding` | Versioned line equivalence, exact aggregation and one rounding boundary |
| `cpt-cf-bss-billing-fr-posted-corrections` | Expected invoice revision and immutable posted corrections |
| `cpt-cf-bss-billing-fr-financial-recovery` | Stable execution keys, ordered predecessor recovery and durable outcomes |
| `cpt-cf-bss-billing-fr-advance-allocation` | Versioned obligation-bound advance/coverage allocations |
| `cpt-cf-bss-billing-fr-trial-coverage` | One durable conditional grant and same-grant correction history |
| `cpt-cf-bss-billing-fr-service-boundaries` | Served-time classification independent of callback arrival |
| `cpt-cf-bss-billing-fr-recoverable-evidence` | Scoped snapshots, committed catch-up and historical repair |

| NFR | Design response | Verification |
| --- | --- | --- |
| `cpt-cf-bss-billing-nfr-tenant-isolation` | PolicyEnforcer scope on commands, reads and every recovery page | Denied reads, scope changes and cross-tenant references |
| `cpt-cf-bss-billing-nfr-evidence-retention` | Retained immutable financial input and execution history | Correction, outage, history-loss and restore cases under agreed bounds |

### 1.3 Architecture Layers

The SDK owns public contracts. Application services coordinate target assembly, invoice changes, allocations and recovery. Domain records hold immutable evidence and state transitions. Infrastructure uses ToolKit secure storage, outbox workers and deployment-selected financial adapters through ClientHub. Public HTTP adapters use canonical RFC-9457 errors.

Use the existing [ToolKit architecture](../../../../docs/toolkit_unified_system/README.md) and [ClientHub/plugin model](../../../../docs/toolkit_unified_system/03_clienthub_and_plugins.md). No runtime layout, crate, database migration or endpoint is implemented by this draft.

## 2. Principles & Constraints

### 2.1 Design Principles

- [ ] `p1` - **ID**: `cpt-cf-bss-billing-principle-exact-recoverable-targets`

- Full rated replacements are the monetary input. No independent Rating delta channel creates another financial effect.
- Commercial membership, usage completeness, rated completeness and financial completion are independent proofs.
- Source revisions are owner-local. A barrier vector is not a global transaction.
- Unknown financial outcomes retain their identities and recovery obligations.
- Posted documents are immutable. Corrections use new authorized financial operations.

### 2.2 Constraints

- [ ] `p1` - **ID**: `cpt-cf-bss-billing-constraint-local-financial-boundaries`

- Local transactions contain no remote calls. Stage external evidence before guarded local acceptance.
- Follow the [shared money ADR](../../docs/ADR/0001-cpt-cf-bss-adr-exact-decimal-boundaries.md). Tax, FX and payment execution retain their own owners.

## 3. Technical Architecture

### 3.1 Domain model

| Record | Required content |
| --- | --- |
| Source reference | Tenant-qualified identity, exact version/digest and predecessor for a fact, seal, rated result or manifest |
| Financial target | Stable ID/revision/digest, selected seal, full exact result/manifest set, financial basis, allocations and predecessor |
| Aggregate evidence | Immutable line-equivalence version, full source contribution references/digests, exact total and financial target |
| Invoice state | Invoice identity/revision and authoritative draft/posted disposition |
| Financial execution | Stable business key, target, expected financial basis, action, dispatch identity and pending/applied/refused/unknown outcome |
| Advance allocation | Funding identity, scope/currency, accepted obligation, allocation version and remaining applicable evidence |
| Trial grant | Grant identity, immutable accepted benefit terms, exact acquisition/activation operation/preparation generation, reserved/effective/released disposition and original expiry |
| Recovery checkpoint | Authorized scope/query/profile, snapshot identity, fixed partition barriers, continuation and installed generation |

IDs and field names describe semantic roles. Physical schemas, codecs, retention and indexes remain owner design work. Equal identity/version with changed content conflicts even when an older version is no longer the head.

### 3.2 Component Model

#### Input recovery

- [ ] `p1` - **ID**: `cpt-cf-bss-billing-component-input-recovery`

Discover exact seals/results/manifests, stage gaps and retain source provenance.

#### Target assembler

- [ ] `p1` - **ID**: `cpt-cf-bss-billing-component-target-assembler`

Validate completeness and accept one immutable financial cut.

#### Invoice coordinator

- [ ] `p1` - **ID**: `cpt-cf-bss-billing-component-invoice-coordinator`

Apply contribution replacements, group lines and fence draft/posting races.

#### Allocation service

- [ ] `p1` - **ID**: `cpt-cf-bss-billing-component-allocation-service`

Allocate accepted advances and trial benefits without changing gross tariffs.

#### Financial executor

- [ ] `p1` - **ID**: `cpt-cf-bss-billing-component-financial-executor`

Dispatch stable business operations and recover unknown outcomes.

#### Recovery/read service

- [ ] `p1` - **ID**: `cpt-cf-bss-billing-component-recovery-reads`

Expose exact target/execution state and authorized historical reconstruction.

### 3.3 API Contracts

The following semantics define the draft SDK and provider boundaries. Exact DTO names and financial adapter versions remain open.

#### 3.3.1 Complete target acceptance

1. Recover the selected commercial seal and every exact member fact. For usage, obtain the complete expected manifest, selected child results and inventory/attribution/coverage proofs. Missing input stays pending.
2. Match every rated result's source fact ID, version and digest to the seal exactly. A newer fact waits for its successor seal. A new calculation result for the same exact fact may create a new financial target under the same seal.
3. Verify result/manifest lineage and supported profile. A digest conflict, unsupported gap or observed newer commercial head prevents stale acceptance. Preserve the previous accepted target while the successor is incomplete.
4. Build the full financial target outside the mutation transaction. Serialize the group/head decision locally, recheck expected heads, and persist the exact reference set and digest.

The target is a declared immutable cut. It does not promise that no later correction can arrive. A later valid correction produces a successor target; finalization follows the accepted profile rather than an invented early cutoff.

A complete empty replacement removes only that fact's old contributions. Active zero-valued lines retain classification, accepted terms and provenance. Pending work emits neither monetary form. A zero total cannot erase source lines or establish refund completion.

#### 3.3.2 Invoice equivalence and rounding

The candidate invoice key includes billing group, subscription sub-line, meter/item, dimension key/value, line kind, currency, stored scale, unit, GL code, tax category and invoice template. Its final fields and codec require Billing acceptance. Fact/result revision, hour, price ID and lot ID are contribution provenance, not independent grouping dimensions.

Sum compatible exact major-unit contributions, including supported top-ups, then round once HALF_EVEN at stored scale. Reject unsupported scales, precision and aggregate posting overflow. Separate stored scales cannot share one rounded line. Quantity business rounding stays with the accepted rating model; Rating performs no invoice monetary rounding.

A full replacement may move a contribution between keys when classification or scale changes. Update both affected aggregates while preserving complete evidence. Ledger's optional single snapshot field may reference an immutable aggregate-evidence object under an agreed mapping; selecting one convenient fact or splitting one line to fit that field would lose evidence or change rounding.

**Examples.** Two EUR 0.005 contributions to one scale-2 line total EUR 0.01. Two distinct line keys each round to EUR 0 under HALF_EVEN. If a line previously posted EUR 0.01 from exact EUR 0.006, and the corrected total is EUR 0.005, the corrected posted total is EUR 0 and the credit is EUR 0.01. Do not round the exact difference.

**Cross-line minimum allocation remains open.** A floor spanning several invoice keys requires a complete rule for base contributions, top-up allocation, final line rounding and corrections. For example, exact 5.005 + 5.005 + 19.99 equals 30, while separately rounded lines total 29.99.

A candidate approach is to round a total shortfall once, allocate it by exact proportions and assign the residual to a stable slot. That alone does not prove the floor after base lines are rounded. Owners must resolve its interaction with one-rounding-per-line and immutable source provenance, with golden correction cases. This draft does not enable that combination or silently choose the candidate algorithm.

#### 3.3.3 Draft replacement and posted correction

Billing derives the new complete contribution set from the accepted target and actual financial basis. A mutable draft can be replaced only under its expected invoice revision. If it posted meanwhile, the stale draft write refuses; recover posted history and derive the authorized credit/debit correction. A detached `periodState` read cannot close this race.

Persist target, correction intent, stable execution key, expected invoice/financial basis and outbox atomically. Dispatch external work after commit. Recover a lost response by the same identity before another effect relies on an assumed balance. Order execution per affected group/invoice; coalescing requires proof of the predecessor's disposition.

**Example.** EUR 30 is posted. Target v2 is EUR 25 and its EUR 5 credit response is lost. Target v3 is EUR 27. If v2 applied, v3 needs a EUR 2 debit against EUR 25. If v2 is authoritatively unapplied and fenced, a supported coalesced correction may move EUR 30 to EUR 27. Never execute independent EUR 5 and EUR 3 credits or discard unknown v2.

A consumer may skip materializing an intermediate Rating result after recovering its immutable lineage. It cannot skip recovery of an already-dispatched financial operation. Advances, taxes and benefits remain part of the actual financial basis; these examples select no refund method or timing.

#### 3.3.4 Advances and financial admission

A full EUR 50 setup valuation with an eligible EUR 50 advance remains gross EUR 50. Billing applies that advance once and creates no second collection from receiving the valuation. Partial advances reduce the remaining payable only. Corrections preserve the allocation identity and previous financial basis.

When Billing is the selected financial-admission provider, follow the [Subscriptions financial-admission contract](../../subscriptions/docs/DESIGN.md#3311-financial-admission-integration-contract). Retain acquisition-bound allocated payment or an accepted future postpay commitment through technical waiting. Activation consumes that basis without a new eligibility assessment or duplicate collection. An explicit disposition must serialize against application; quote expiry or a changed live credit limit alone does not revoke already accepted coverage.

The exact designated provider, Payments/Ledger mapping and funding exclusivity protocol remain open. Promotional credit is neither settled customer cash nor an authorization for postpay service.

#### 3.3.5 Trial grants and service-time allocation

Follow the accepted [trial feature](../../subscriptions/docs/features/06-trials.md). One grant is bound to the purchase and exact activation operation/preparation generation. Before readiness, Billing returns a durable reserved commitment with immutable terms/scope/digest. Authoritative applied activation makes the same grant effective at actual T; delayed recording preserves T and original expiry without another approval or access gate.

Fenced terminal non-activation permits release. An unknown outcome retains the commitment and triggers recovery. Technical retry cannot create another grant. Authorized pre-T and never-activated preparation is platform-borne and consumes no bonus.

Rating publishes ordinary gross value for credit trials. Billing allocates it once among bonus, platform and customer using the full source version and actual served/paid boundaries. A free-period profile uses explicit zero tariff bindings and requires no promotional grant. Cash-funded Ledger credit is not this benefit provider.

Retain original restrictions, scope, currency and expiry. A 30-to-25 correction under a valid 100 grant restores eligible remaining coverage from 70 to 75 once. After original expiry, historical net consumption may become 25, but spendable credit remains zero. A correction cannot reopen an exhausted closed trial or undo an applied paid conversion. Early paid conversion preserves eligible unused credit until the same original expiry.

Eligible pre-paid-boundary trial expenses beyond remaining bonus, including positive corrections, are platform-borne. For eligible gross 90-to-110 under grant 100, the replacement is bonus 100, platform 10, customer 0. Post-paid-boundary residual follows accepted paid terms. Late arrival does not change served-time classification. Missing or unsupported boundary evidence remains pending; no negative bonus, automatic top-up or invented customer debt.

The grant must also satisfy the agreed access/enforcement and bounded-exposure contract. Financial allocation, posting and access status are separate. Provider mapping, accounting entries, retention and liability outside the accepted eligible trial scope require further design.

##### Grant allocation and recovery

Each grant retains accepted policy/digests, original scope/currency, issued amount, allocation revisions, historical consumed/remaining totals and eligibility reasons. Each allocation retains the canonical full Rating identity/version/digest, prior accepted allocation, frozen tariff, served extents and authoritative lifecycle boundaries. Store per-extent gross, bonus, platform and customer amounts with invoice/period and financial-work references. Local allocation and remote posting completion are separate statuses.

For every fully classified supported extent, allocate gross exactly once among bonus, platform and customer before separate invoice rounding/tax. Missing attribution or unsupported policy remains pending; preserve the prior applied result and allocation. Grant restrictions never broaden to cover another paid-plan SKU, charge kind or currency. A one-time charge still needs its separately accepted occurrence.

1. Authenticate the caller and authorize the seller/payer/beneficiary scope. Resolve the canonical full Rating result and immutable accepted policy, then obtain the required lifecycle/served-time evidence. Separate original grant applicability from the corrected fact's frozen tariff. Missing or ambiguous crossing-boundary evidence stays pending and cannot be allocated wholly by arrival time.
2. Serialize affected grant/allocation revisions within Billing and compare the expected prior accepted result and financial lineage. Concurrent eligible items cannot both consume the last available credit. Ordering among competing allocations and multiple benefits must follow the supported explicit Pricing policy/contract. Stacking is unavailable in this release; no priority value is silently selected.
3. Replay the same accepted result version and digest as a no-op with its stored outcome. Same identity/version with different content is a conflict. A stale version cannot overwrite the head or restore credit again; recover the authoritative head. A required predecessor/manifest gap triggers source recovery instead of applying an independent delta.
4. Derive a replacement allocation from the full result and prior accepted allocation under frozen grant restrictions and authoritative effective boundaries. Eligible delay-caused trial gross 5 with remaining bonus 2 yields bonus 2/platform 3/customer 0; actual post-paid-boundary residual follows accepted paid terms. Preserve chronological expiry, early-conversion grant retention and closed-episode rules. Eligible pre-boundary trial gross corrected from 90 to 110 against grant 100 yields bonus 100/platform 10/customer 0 under the same full-result lineage.

   Liability/correction cases outside that scope remain pending for policy resolution.
5. Atomically commit the accepted result/allocation revision, bounded grant totals, operation receipt and durable financial/outcome work in Billing. History retains the replaced versions; no negative grant or fictional top-up is allowed. An unresolved replacement records its pending reason without claiming a fully applied new allocation or erasing the prior applied version.
6. Execute/recover invoice work through the established full-result/replacement contract: replace eligible drafts; use the standard versioned correction path for posted invoices. Keep an invoice-version guard and stable financial-work identity bound to allocation/result lineage. A duplicate or crash after a remote effect recovers that same effect/acknowledgement; it does not post a second credit or rerun grant issuance. Local allocation acceptance is not proof that a required Ledger posting completed.

   No cross-gear database transaction or inline posted-invoice rewrite is assumed.
7. Publish ordered versioned outcomes from the committed record. Readers can recover a snapshot at an immutable barrier plus subsequent outcomes; cursors bind authorized scope and explicitly report a gap/retention loss or partial page. Never translate lost history or unavailable balance into empty success, zero balance, permission or customer debt. Retention budgets must satisfy the consumer recovery contract.
8. Consumers reconcile outcome versions with the current episode/paid state before any lifecycle intent. Corrected exhaustion never automatically reopens a closed trial; retained-grant exhaustion/expiry after paid conversion ends coverage only. Duplicate/reordered outcomes cannot duplicate or reverse conversion. Access enforcement and payment authorization remain separate contracts.

For an exhausted closed trial, a 100-to-95 correction restores 5 in accounting but grants no access/spend permission for that closed episode. Early valid paid conversion has different accepted behavior: a 100-credit/14-day grant with 60 remaining on day 5 retains that same eligible 60 until original day 14. Neither case creates a new bonus or duration. These values are examples, not defaults. A bonus satisfies prescribed upfront payment only if the accepted settlement contract explicitly permits it.

The promotional grant cannot be backed by fabricated settled cash. Ledger's current cash-funded reusable-credit API is not this provider. Select the financial posting mapping separately; this design adds no journal, tax or revenue-recognition rules. Positive corrections outside eligible accepted-trial service before the applied paid boundary remain a policy gate. The trial rule waives neither unrelated prior debt nor explicit retention/excluded charges.

#### 3.3.6 Offboarding and preparation accounting

Use the [Subscriptions offboarding contract](../../subscriptions/docs/features/04-renewal-retention.md). Confirmed charging cutoff, actual resource stop, applied cancellation and deletion are separate evidence. A confirmed 10:00 cutoff with a 10:05 stop retains truthful runtime but creates no ordinary customer tariff for the technical tail. An earlier qualifying offboarding stop ends accrual; suspension follows its explicit bill/waive policy.

Correct erroneous charges once using the full replacement path. Preserve legitimate earlier debt and separately accepted retention obligations. A retained resource, export access or delayed deletion alone creates no new billed service. Late pre-cutoff usage retains its original terms and coordinates. Financial cutoff alone releases neither resource occupancy nor deletion permission.

Preparation attribution needs authoritative served-time and attempt evidence, including cancellation and never-activated attempts. Exact exposure limits and settlement mapping remain required before enabling the service profile.

#### 3.3.7 Recovery and status

Recover complete seal/fact snapshots and old-period changes from Subscriptions, exact results/manifests and committed catch-up from Rating, and execution outcomes from the financial providers. Due-date scanning alone cannot find a correction to last month's invoice committed today.

Use fixed snapshot membership, committed partition barriers, stable pages and explicit completion. A claim sequence is not a committed-history barrier. Reauthorize every page; filtered empty intervals differ from missing history. Scope expansion requires a new bootstrap.

Persist validated inbox/projection and applied checkpoint atomically. Missing referenced versions, incomplete pages or sequence gaps hold that checkpoint. Broker acknowledgement may mean durable staging; it does not mean a complete target or applied posting. Install a bootstrap generation only after every page and reference validates.

Snapshot expiry with intact history requires rebootstrap. Actual history loss requires authoritative repair or an explicit repair-required pending outcome. Current active resources/subscriptions cannot reconstruct zero obligations or old financial effects. Financially referenced source and execution history survives operational deletion.

### 3.4 Internal Dependencies

Use ToolKit secure storage, PolicyEnforcer, ClientHub and durable worker lifecycle support. Exact crate and storage-provider selection is implementation design work.

### 3.5 External Dependencies

Subscriptions supplies facts/seals and applied service evidence. Rating supplies complete exact results and source proofs. The selected Ledger/Payments providers supply idempotent financial execution and authoritative recovery. Trial access requires the agreed Entitlements coverage mapping.

### 3.6 Interactions & Sequences

- [ ] `p1` - **ID**: `cpt-cf-bss-billing-seq-target-to-financial-outcome`

Stage source references, recover their immutable content, validate a complete cut, then accept a target under local head guards. Derive the draft or posted correction against authoritative financial state, persist execution identity and outbox, then dispatch. A lost response enters recovery under the same identity before successor effects are allowed.

### 3.7 Database schemas & tables

The eventual SDK must expose exact target/execution reads, financial-admission and advance evidence where selected, trial coverage prepare/read/resolve, structured pending/conflict states and scoped recovery. Stable business keys and canonical error meanings must be agreed with each adapter before dispatch is enabled.

Storage must durably enforce source-version conflicts, target/head transitions, invoice revisions, operation uniqueness and trial grant resolution. Exact tables, constraints, isolation and retention remain implementation design choices; demonstrate the specified races before selecting a provider profile.

## 4. Open Decisions

| ID | Required decision |
| --- | --- |
| BILL-Q01 | Final versioned invoice key, supported classification changes and aggregate-evidence mapping to Ledger |
| BILL-Q02 | Cross-line minimum top-up allocation consistent with final line rounding, correction lineage and the shared money rules |
| BILL-Q03 | Selected financial-admission provider, protected funding and Payments/Ledger execution/recovery mappings |
| BILL-Q04 | Promotional grant schema, access/enforcement coordination, accounting mapping and liability outside accepted trial coverage |
| BILL-Q05 | Tax/FX participation, financial posting types and applicable refund/remedy policies; no defaults inferred here |
| BILL-Q06 | Legal retention, supported correction horizon, throughput, outage and restore bounds |

## 5. Traceability

The [PRD](PRD.md) defines the requirements and acceptance cases. Subscriptions owns the [commercial source contract](../../subscriptions/docs/DESIGN.md#financial-consumer-boundary), [trial coverage](../../subscriptions/docs/features/06-trials.md) and [offboarding boundaries](../../subscriptions/docs/features/04-renewal-retention.md). The [money ADR](../../docs/ADR/0001-cpt-cf-bss-adr-exact-decimal-boundaries.md) owns shared arithmetic and posting representation.
