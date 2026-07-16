# PRD — Usage Collector Phase 2: Usage Event Feed & Ingestion Watermarks

> **Status**: proposal (2026-07-16). Adoption is decided **in this repo** (upstream-detach
> decision 2026-07-15/16: this branch is the canonical home; the collector's evolution for our
> platform lives here) — pending implementation scheduling.
> **Relationship to [PRD.md](./PRD.md)**: a self-contained phase-2 increment. The v1 PRD remains
> authoritative for every phase-1 surface; nothing here modifies a v1 requirement. Every phase-2
> capability is **additive within REST v1 / SDK v1** under
> `cpt-cf-usage-collector-adr-contract-stability` (ADR-0006) and executes reserved hooks the v1
> documents already name (DESIGN §4 deferral table: "Push / subscribe surface for downstream
> readers", "Rate limiting & watermarks"; PRD §4.2: watermark metadata "covered in a later
> phase"). On adoption, the sections below fold into the corresponding PRD.md sections per the
> one-PRD-per-gear convention.

<!-- toc -->

- [1. Overview](#1-overview)
  - [1.1 Purpose](#11-purpose)
  - [1.2 Background / Problem Statement](#12-background--problem-statement)
  - [1.3 Goals (Business Outcomes)](#13-goals-business-outcomes)
  - [1.4 Glossary](#14-glossary)
- [2. Actors](#2-actors)
  - [2.1 New System Actor](#21-new-system-actor)
  - [2.2 Actor Permissions (delta)](#22-actor-permissions-delta)
- [3. Operational Concept & Environment](#3-operational-concept--environment)
- [4. Scope](#4-scope)
  - [4.1 In Scope](#41-in-scope)
  - [4.2 Out of Scope](#42-out-of-scope)
- [5. Functional Requirements](#5-functional-requirements)
  - [5.1 Usage Event Feed](#51-usage-event-feed)
  - [5.2 Ingestion Watermarks](#52-ingestion-watermarks)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Surface Additions](#71-surface-additions)
  - [7.2 Usage Event Feed Contract](#72-usage-event-feed-contract)
- [8. Use Cases](#8-use-cases)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Risks](#12-risks)
- [13. Open Questions](#13-open-questions)
- [14. Traceability](#14-traceability)

<!-- /toc -->

## 1. Overview

### 1.1 Purpose

Phase 2 adds the **outbound half** of the metering substrate: a durable, resumable, per-tenant-
ordered **Usage Event Feed** of accepted entries (usage records, compensation entries, status
transitions) plus per-`(tenant, UsageType)` **ingestion watermarks**. Phase 1 deliberately shipped
pull-only; phase 2 executes the additive push hook the v1 DESIGN reserved, triggered by the first
concrete consumer requirement: the BSS rating pipeline's usage ingestion.

Terminology note: *emission* in the v1 PRD means the **inbound** submission of records by usage
sources; that meaning is unchanged. The outbound surface is named the **Usage Event Feed**.

### 1.2 Background / Problem Statement

The v1 Usage Collector is intentionally synchronous request/response on every public surface;
near-real-time consumers poll a Query SPI that is **eventually consistent with no upper bound** at
the gear floor, and read-after-write is guaranteed only via the ingestion ack (DESIGN §3.10,
ADR-0011). The v1 DESIGN reserved a push surface as "reserved-not-built — added once a concrete
consumer requirement and fan-out design land together" (DESIGN §4).

That consumer requirement has landed. The BSS rating pipeline
(`gears/bss/rating/docs/design/12-usage-ingestion-normalization.md`) is contractually built on a
durable, at-least-once, per-partition-ordered usage transport with offsets and source replay — the
pattern billing-critical consumers require to guarantee "one accepted record contributes to a
rated charge at most once". The polled Query SPI cannot serve it:

- **No freshness bound** — a cursor window over an eventually-consistent read can never be safely
  closed.
- **Event-time cursors miss late-accepted records** — the raw query filters on `created_at`; a
  record accepted late (with an older `created_at`) inserts *behind* an already-consumed cursor
  position and is silently missed.
- **Retractions are invisible** — event deactivation is an operator write outside the ingestion
  path; a poll of accepted records never observes it, so a downstream biller would keep charging
  for a retracted record.

The full seam analysis is recorded in the rating seam register,
`gears/bss/rating/docs/SEAMS.md` §J (UC1–UC6); UC1 gates the implementation of the rating
ingestion slice.

### 1.3 Goals (Business Outcomes)

| Goal | Success Measure | Baseline (v1) |
|------|-----------------|---------------|
| Unblock streaming consumption for billing-critical downstreams | The BSS rating pipeline ingests usage exclusively from the Usage Event Feed (no poll bridge, no dual-emit at sources); rating seam UC1 closed | No compliant transport exists; rating slice 12 blocked |
| End-to-end correction visibility | Every compensation and every deactivation (incl. depth-1 cascade) affecting an authorized scope is observable on the feed; zero silent retractions downstream | Deactivations invisible to any ingestion-path consumer |
| Reduced invoice adjustment churn | Downstream period closes gate on the ingestion watermark grace window; correction/adjustment volume attributable to ingestion lag drops | No watermark; every period close maximally provisional |

### 1.4 Glossary

Terms below extend the v1 PRD §1.4 glossary; v1 terms keep their meaning.

| Term | Definition |
|------|------------|
| Usage Event Feed | The phase-2 **outbound** surface: a durable, resumable, per-tenant-ordered feed of accepted entries (usage records, compensation entries, status transitions) for downstream consumers ([§5.1](#51-usage-event-feed)). |
| Accepted Sequence | `acceptedSeq` — a per-tenant, strictly-increasing (gaps permitted) sequence number assigned to every accepted entry at acceptance time; the ordering coordinate of the feed. Distinct from the caller-supplied `created_at` event time. |
| Feed Cursor | A consumer-held position in the feed (per tenant partition, in `acceptedSeq` order) from which consumption resumes deterministically after restart or redelivery. |
| Status Transition Entry | A feed entry produced by an event deactivation (including each depth-1 cascaded compensation flip), carrying the affected record id and the prior/new `status`, with its own `acceptedSeq`. |
| Ingestion Watermark | A per-`(tenant, UsageType)` acceptance-progress marker: every entry with `acceptedSeq` at or below the watermark is feed-visible. Explicitly **not** an event-time completeness promise ([§5.2](#52-ingestion-watermarks)). |
| Feed Consumer | A downstream system consuming the Usage Event Feed ([§2.1](#21-new-system-actor)). |

## 2. Actors

### 2.1 New System Actor

#### Feed Consumer

**ID**: `cpt-cf-usage-collector-actor-feed-consumer`

- **Role**: A downstream system consuming the ordered Usage Event Feed rather than (or in addition
  to) the polled query surfaces. First concrete consumer: the BSS rating pipeline (usage
  ingestion, `gears/bss/rating` design slice 12; seam register `gears/bss/rating/docs/SEAMS.md`
  §J).
- **Needs**: Durable at-least-once delivery in accepted order, deterministic cursor replay after
  restart, visibility of corrections and retractions (compensations and deactivations), and
  per-`(tenant, UsageType)` ingestion watermarks.

All v1 actors (PRD §2) are unchanged; the platform operator additionally operates the feed
surfaces, and existing usage consumers may adopt the feed without a contract change to their query
access.

### 2.2 Actor Permissions (delta)

| Actor | Permitted Operations | Denied by Default |
|-------|----------------------|-------------------|
| `cpt-cf-usage-collector-actor-feed-consumer` | Subscribe to and replay the Usage Event Feed for PDP-authorized tenant scopes; read ingestion watermarks for the same scopes | Consuming feed entries for tenants outside the PDP-authorized scope; mutating usage records or feed state; influencing ingestion admission or ordering |

Authorization follows the v1 posture verbatim: enforced via the platform PDP on every operation,
fail-closed, no anonymous bypass, no cached decisions (v1 PRD §2.2,
`cpt-cf-usage-collector-contract-authz-resolver`).

## 3. Operational Concept & Environment

Three constraints shape every requirement in this document:

- **Additive within v1.** All phase-2 surfaces are additive within REST v1 / SDK v1 under
  `cpt-cf-usage-collector-adr-contract-stability` (ADR-0006); no v1 endpoint, trait method, or
  schema changes incompatibly. "Phase 2" is a capability increment, not a major version.
- **The feed never taxes ingestion.** A downstream consumer's outage or slowness must not couple
  into the ingestion path (the v1 DESIGN §4 reservation: "a push surface must not couple
  downstream outages into ingestion"). Feed derivation and delivery are asynchronous to
  acceptance; the phase-1 ingestion NFRs hold with the feed enabled.
- **Backend-agnostic by construction.** `cpt-cf-usage-collector-adr-pluggable-storage` admits
  backends with no native change-feed (e.g. ClickHouse). Accepted order therefore derives from the
  ingestion path (an accepted-sequence assigned at acceptance), never from a plugin-specific
  change stream; the Plugin SPI gains an accepted-order scan capability, versioned per
  `cpt-cf-usage-collector-nfr-plugin-contract-stability`.

## 4. Scope

### 4.1 In Scope

- The Usage Event Feed: durable, resumable, per-tenant-ordered, at-least-once delivery of accepted
  entries (usage records, compensation entries) and status transition entries
- Deterministic feed-cursor replay over the active plugin's retention window, including
  new-consumer bootstrap
- Status transition entries for event deactivation, including the depth-1 cascade flips
- Ingestion watermarks per `(tenant, UsageType)` with a query surface and feed heartbeats
- PDP-authorized, tenant-scoped feed access
- The Plugin SPI accepted-order scan extension required for feed derivation

### 4.2 Out of Scope

- **Business logic** — pricing, rating, billing rules, invoice generation, quota decisions remain
  downstream responsibilities (v1 PRD §4.2, unchanged)
- **Dimension-value carriage as a new capability** — none is needed: dimension values ride the
  UsageType's declared closed-shape `metadata_fields` (a v1 capability) and are delivered on the
  feed verbatim by `cpt-cf-usage-collector-fr-usage-event-feed`. The meter ↔ UsageType binding and
  the dimension-set cross-validation are registry/pricing-side obligations (rating seam UC3), not
  collector capabilities
- **A general business event bus** — the feed carries this gear's accepted entries only; platform
  eventing topology is out of scope
- **Guaranteed event-time completeness** — the watermark is an acceptance-progress signal only;
  records with arbitrarily old `created_at` remain acceptable (v1 posture, `domain-model.md` §2.1)
- **External reconciliation workflows** — remain out of scope for the gear entirely (v1 PRD §4.2)
- **Per-consumer delivery guarantees** — the gear guarantees feed availability and ordering; a
  consumer's own consumption rate, offset management, and downstream processing are the
  consumer's

## 5. Functional Requirements

### 5.1 Usage Event Feed

#### Usage Event Feed

- [ ] `p1` - **ID**: `cpt-cf-usage-collector-fr-usage-event-feed`

The system **MUST** expose a durable, resumable Usage Event Feed of accepted entries: usage
records, compensation entries, and status transition entries. The system **MUST** assign every
accepted entry a per-tenant, strictly-increasing accepted sequence (`acceptedSeq`; gaps permitted)
at acceptance time, and the feed **MUST** deliver entries in `acceptedSeq` order within a tenant
partition. Delivery is **at-least-once**: a redelivered entry carries the same `acceptedSeq` and
canonical content. Every feed entry **MUST** carry the full canonical record — value, `created_at`,
attribution (tenant, resource, optional subject), `metadata` verbatim, `status`, `corrects_id`
where set — plus the identity tuple `(tenant_id, gts_id, idempotency_key)` and its `acceptedSeq`,
so a consumer needs no follow-up query to process an entry. The feed **MUST NOT** couple a
downstream consumer's outage or slowness into the ingestion path (feed derivation and delivery are
asynchronous to acceptance), and it **MUST** be available regardless of the active storage
backend's native change-feed capability (accepted order derives from the ingestion path, not from
a plugin-specific change stream). Feed access is PDP-authorized per tenant scope, consistent with
the v1 tenant-isolation posture on read surfaces (`cpt-cf-usage-collector-fr-tenant-isolation`).

- **Rationale**: The first concrete downstream consumer (the BSS rating pipeline, rating design slice 12) is contractually built on a durable at-least-once ordered transport with replay — the pattern billing-critical consumers require to guarantee "one accepted record contributes to a rated charge at most once". Deriving order from acceptance (not event time) is what makes the cursor safe: a late-arriving `created_at` never inserts behind an already-consumed position. Keeping ingestion decoupled preserves the phase-1 ingestion NFRs and the fail-closed ingestion posture; keeping the feed backend-agnostic preserves `cpt-cf-usage-collector-fr-pluggable-storage`.
- **Actors**: `cpt-cf-usage-collector-actor-feed-consumer`, `cpt-cf-usage-collector-actor-usage-consumer`

#### Feed Cursor Replay

- [ ] `p1` - **ID**: `cpt-cf-usage-collector-fr-feed-cursor-replay`

A feed consumer **MUST** be able to resume consumption from any retained cursor position, and
replay from a given cursor **MUST** be deterministic: the same cursor yields the same entries with
the same `acceptedSeq` and byte-identical canonical content, in the same order. The replay horizon
**MUST** equal the active storage plugin's record-retention window (the feed is derived from
retained records, not from a separate ephemeral buffer); a new consumer bootstraps by replaying
from the horizon start. A cursor before the horizon **MUST** be rejected with an actionable error
naming the earliest available position — never silently skipped forward.

- **Rationale**: Restart-safety and new-consumer bootstrap without a parallel export path. Deterministic replay is the property downstream determinism contracts build on (the rating pipeline re-derives byte-identical normalized records from a source-stream replay); silent skip-forward would be invisible data loss for a billing consumer, which must instead surface the gap and decide explicitly.
- **Actors**: `cpt-cf-usage-collector-actor-feed-consumer`

#### Status Transition Feed Entries

- [ ] `p1` - **ID**: `cpt-cf-usage-collector-fr-feed-status-entries`

Every event deactivation — including each depth-1 cascaded compensation flip
(`cpt-cf-usage-collector-fr-event-deactivation`) — **MUST** produce a status transition entry on
the Usage Event Feed with its own `acceptedSeq`, carrying the affected record id, the prior and
new `status`, and a reference to the operator action. A retraction **MUST NOT** be observable only
through the query surfaces: a feed consumer that never polls **MUST** still observe every
deactivation affecting its authorized scope. Compensation entries need no special treatment — they
ride the ingestion path and appear on the feed as accepted entries under
`cpt-cf-usage-collector-fr-usage-event-feed`.

- **Rationale**: Deactivation is an operator-initiated write outside the ingestion path; without a feed entry it is invisible to a stream consumer, and a downstream biller would keep charging for a retracted record (rating seam UC5). Compensations already enter through ingestion, so the accepted-entry stream covers them by construction.
- **Actors**: `cpt-cf-usage-collector-actor-feed-consumer`, `cpt-cf-usage-collector-actor-platform-operator`

### 5.2 Ingestion Watermarks

#### Ingestion Watermark

- [ ] `p2` - **ID**: `cpt-cf-usage-collector-fr-ingestion-watermark`

The system **MUST** expose a per-`(tenant, UsageType)` ingestion watermark: a monotonic
acceptance-progress marker guaranteeing that every entry with `acceptedSeq` at or below the
watermark is feed-visible. The watermark **MUST** be exposed on a query surface and **SHOULD** be
carried periodically on the Usage Event Feed as a heartbeat entry (cadence and idle-tenant
behavior are DESIGN-level). The watermark describes **acceptance progress only**: it is explicitly
**not** an event-time completeness promise — records with older `created_at` values remain
acceptable at any later time (consistent with the v1 posture that old event timestamps are
accepted without wall-clock validation), and consumers **MUST** treat the watermark as a
lateness-reduction signal, never as a correctness gate.

- **Rationale**: The rating consumer closes billing periods on time anchors and absorbs late usage through its correction machinery; a watermark lets it hold a short grace window before finalizing usage lines, cutting adjustment churn on invoices without introducing a completeness dependency the collector cannot honestly promise (rating seam UC2). Scoping the promise to acceptance progress keeps the contract truthful under unbounded event-time lateness.
- **Actors**: `cpt-cf-usage-collector-actor-feed-consumer`, `cpt-cf-usage-collector-actor-usage-consumer`

## 6. Non-Functional Requirements

#### Feed Lag and Ingestion Non-Regression

- [ ] `p2` - **ID**: `cpt-cf-usage-collector-nfr-feed-lag`

Under the `cpt-cf-usage-collector-nfr-throughput-profile` load envelope, the lag from record
acceptance to feed visibility **MUST** be ≤ 5 seconds at p95 (**provisional pending the phase-2
NFR review**; the bound is a freshness target for the feed, not a delivery guarantee to any given
consumer, whose own consumption rate is outside the gear's control). Enabling the Usage Event Feed
**MUST NOT** regress the phase-1 ingestion NFRs: `cpt-cf-usage-collector-nfr-ingestion-latency`
(p95 ≤ 200ms) and `cpt-cf-usage-collector-nfr-ingestion-throughput` (≥ 10,000 records/sec) hold
with the feed enabled and consumers attached.

- **Rationale**: The feed's value to a billing-critical consumer is near-real-time correction visibility; an unbounded lag degrades it to a batch export. The non-regression clause preserves the phase-1 contract for every existing usage source: the outbound surface is paid for by the feed path, never by the ingestion path (the decoupling constraint of `cpt-cf-usage-collector-fr-usage-event-feed`).

The v1 NFR exclusions (PRD §6.2) apply to phase 2 unchanged.

## 7. Public Library Interfaces

### 7.1 Surface Additions

All additions follow the wire shape the v1 DESIGN §4 reserved and are additive within REST v1 /
SDK v1 (ADR-0006):

- **REST**: SSE `GET /usage-collector/v1/records/stream` with cursor resume; a cursor-paged
  catch-up endpoint for deterministic replay; a watermark read endpoint. Authoritative wire
  schemas land in `usage-collector-v1.yaml` at design time.
- **SDK trait**: a `Stream`-returning feed-subscription method and a watermark read method, added
  with default implementations per the v1 SDK breaking-change policy.
- **Plugin SPI**: an accepted-order scan capability (read retained entries in `acceptedSeq` order
  from a cursor) — the feed-derivation primitive; versioned and coordinated per
  `cpt-cf-usage-collector-nfr-plugin-contract-stability`.

### 7.2 Usage Event Feed Contract

- [ ] `p1` - **ID**: `cpt-cf-usage-collector-contract-usage-event-feed`

<!-- cpt-cf-id-content -->

**Direction**: provided by library (ordered outbound feed consumed by downstream feed consumers; first named consumer: the BSS rating pipeline — `gears/bss/rating` design slice 12, seam register `gears/bss/rating/docs/SEAMS.md` §J)
**Protocol/Format**: the Usage Event Feed surfaces per [§7.1](#71-surface-additions) — SSE with cursor resume, a cursor-paged catch-up endpoint for deterministic replay, and a `Stream`-returning SDK method. Entries are delivered at-least-once, ordered by `acceptedSeq` within a tenant partition, each carrying the full canonical record + identity tuple `(tenant_id, gts_id, idempotency_key)` + `acceptedSeq` (`cpt-cf-usage-collector-fr-usage-event-feed`).
**Consumed / Provided Data**: consumers supply a PDP-authorized subscription scope and a feed cursor; the collector provides accepted entries (usage, compensation), status transition entries, and watermark heartbeats. Consumer obligations: consumption is idempotent on the identity tuple (a redelivery is absorbed, never double-processed); any downstream deduplication key **MUST** embed the `gts_id` scope (an idempotency key legitimately recurs across UsageTypes — rating seam UC4); `metadata` is passed through verbatim and interpreted downstream. Business logic (pricing, rating, invoice generation, quota decisions) remains outside the collector.
**Availability / Fallback**: a feed outage or slow consumer never blocks or degrades ingestion (`cpt-cf-usage-collector-nfr-feed-lag` non-regression clause); on reconnect a consumer resumes from its cursor via the catch-up endpoint (`cpt-cf-usage-collector-fr-feed-cursor-replay`). Consumers **MUST NOT** invent entries for gaps; a pre-horizon cursor is an explicit error.
**Compatibility**: additive within REST v1 / SDK v1 per `cpt-cf-usage-collector-adr-contract-stability` (ADR-0006) and `cpt-cf-usage-collector-nfr-plugin-contract-stability`. The Plugin SPI extension required for feed derivation (accepted-order scan) is versioned and coordinated per the same stability NFR.

<!-- cpt-cf-id-content -->

## 8. Use Cases

#### Consume the Usage Event Feed

- [ ] `p1` - **ID**: `cpt-cf-usage-collector-usecase-consume-event-feed`

**Actor**: `cpt-cf-usage-collector-actor-feed-consumer`

**Preconditions**:

- Actor is authenticated with a valid SecurityContext and PDP-authorized for the subscribed tenant
  scope
- The active storage plugin supports the accepted-order scan capability

**Main Flow**:

1. Consumer subscribes to the feed for its authorized scope, presenting its last committed cursor
   (or none, for bootstrap)
2. System authorizes the subscription via PDP and streams entries from the cursor in `acceptedSeq`
   order: accepted usage and compensation entries, status transition entries, and watermark
   heartbeats
3. Consumer processes each entry idempotently on the identity tuple and periodically commits its
   cursor
4. On restart or reconnect, the consumer resumes from its committed cursor; redelivered entries
   are absorbed by idempotent consumption

**Postconditions**:

- The consumer has observed every accepted entry and every status transition in its authorized
  scope, exactly once by identity tuple
- The consumer's cursor advances monotonically; no entry in scope is silently skipped

**Alternative Flows**:

- **Authorization denied / scope narrowed**: subscription rejected or constrained per PDP; no
  entries outside the authorized scope are delivered
- **Pre-horizon cursor**: system rejects with an actionable error naming the earliest available
  position; the consumer decides explicitly (bootstrap replay or operational escalation)
- **Slow consumer / feed backlog**: ingestion is unaffected; the consumer catches up via the
  cursor-paged endpoint

#### Gate a Downstream Period Close on the Ingestion Watermark

- [ ] `p2` - **ID**: `cpt-cf-usage-collector-usecase-watermark-close-gate`

**Actor**: `cpt-cf-usage-collector-actor-feed-consumer`

**Preconditions**:

- The consumer operates a time-anchored close process (e.g. a billing period close) downstream

**Main Flow**:

1. At its close anchor, the consumer reads the ingestion watermark for the relevant
   `(tenant, UsageType)` pairs
2. The consumer holds a bounded grace window until the watermark passes its close-relevant
   position (or the window expires)
3. The consumer finalizes its close; any usage accepted later is handled by the consumer's own
   correction machinery

**Postconditions**:

- The close consumed all feed-visible usage up to the watermark; residual lateness is bounded to
  genuinely late acceptance, not to feed lag

**Alternative Flows**:

- **Watermark unavailable**: the consumer proceeds on its own schedule — the watermark is a
  lateness-reduction signal, never a correctness gate (`cpt-cf-usage-collector-fr-ingestion-watermark`)

## 9. Acceptance Criteria

The §9.0 load-envelope and measurement definitions of the v1 PRD apply verbatim to every numeric
criterion below.

- [ ] Every accepted usage record and compensation entry appears on the Usage Event Feed exactly once per `acceptedSeq`, in per-tenant `acceptedSeq` order, carrying the full canonical record, the identity tuple `(tenant_id, gts_id, idempotency_key)`, and its `acceptedSeq` (cross-reference `cpt-cf-usage-collector-fr-usage-event-feed`)
- [ ] Feed delivery is at-least-once and redeliveries are byte-identical: a consumer that dedups on the identity tuple observes each entry's effect exactly once (cross-reference `cpt-cf-usage-collector-fr-usage-event-feed`)
- [ ] `acceptedSeq` is strictly increasing per tenant (gaps permitted) and is assigned at acceptance: a record accepted later with an older `created_at` receives a higher `acceptedSeq` and is delivered ahead of no already-delivered position (cross-reference `cpt-cf-usage-collector-fr-usage-event-feed`)
- [ ] Resuming from a committed cursor is deterministic: the same cursor yields the same entries, same order, byte-identical content; a pre-horizon cursor is rejected with an actionable error naming the earliest available position, and the replay horizon equals the active plugin's record-retention window (cross-reference `cpt-cf-usage-collector-fr-feed-cursor-replay`)
- [ ] Every deactivation — including each depth-1 cascaded compensation flip — produces a status transition entry with its own `acceptedSeq`; a feed consumer that never polls the query surfaces still observes every retraction in its authorized scope (cross-reference `cpt-cf-usage-collector-fr-feed-status-entries`)
- [ ] The ingestion watermark per `(tenant, UsageType)` is monotonic and guarantees feed visibility of every entry at or below it; documentation and wire contract state that it is not an event-time completeness promise (cross-reference `cpt-cf-usage-collector-fr-ingestion-watermark`)
- [ ] Feed subscription and watermark reads are PDP-authorized and tenant-scoped; entries outside the authorized scope are never delivered (cross-reference `cpt-cf-usage-collector-fr-usage-event-feed`)
- [ ] Acceptance-to-feed-visibility lag is ≤ 5s at p95 under the load envelope (provisional pending the phase-2 NFR review), and the phase-1 ingestion NFRs (p95 ≤ 200ms, ≥ 10,000 records/sec) hold with the feed enabled and consumers attached (cross-reference `cpt-cf-usage-collector-nfr-feed-lag`)
- [ ] The feed operates unchanged over any active storage plugin implementing the accepted-order scan capability, including backends without a native change-feed (cross-reference `cpt-cf-usage-collector-fr-usage-event-feed`)

## 10. Dependencies

| Dependency | Description | Criticality |
|------------|-------------|-------------|
| authz-resolver | Platform PDP; authorizes feed subscription, replay, and watermark reads (same fail-closed posture as v1) | p1 |
| Storage plugins | Accepted-order scan capability (Plugin SPI extension) from the active plugin; coordinated per the stability NFR | p1 |
| BSS rating pipeline | Driving consumer; co-reviews the feed contract and the dedup-key derivation (rating SEAMS §J UC1–UC6) | p1 |
| Platform gateway | Long-lived SSE connections through the gateway; the cursor-paged catch-up endpoint is the fallback lane if streaming is constrained | p2 |

## 11. Assumptions

| Assumption | Owner | Validation |
|------------|-------|------------|
| Phase-2 surfaces are additive within REST v1 / SDK v1; no v1 consumer or plugin breaks (ADR-0006) | Usage Collector Maintainers | Contract-diff check against `usage-collector-v1.yaml`; plugin compatibility suite |
| The active storage plugins on the v1 roadmap can implement an accepted-order scan (per-tenant monotonic sequence readable from a cursor) | Plugin Authors / Usage Collector Maintainers | SPI design review with each plugin; capability probe at gear readiness |
| The BSS rating pipeline is the first feed consumer and validates the contract end-to-end before general availability | BSS Rating Team | Joint integration fixture: feed → rating slice-12 ingestion → dedup/Q assertions |
| The platform gateway sustains long-lived streaming connections or the deployment falls back to cursor-paged catch-up without contract change | Platform Edge Team | Gateway streaming test; fallback drill |

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Per-tenant monotonic `acceptedSeq` needs a serialization point under multi-instance ingestion; a naive global sequence would throttle the ingestion hot path | Ingestion throughput regression — a direct violation of the non-regression clause | Sequence scope is per tenant (not global) with gaps permitted; assignment strategy is per-backend DESIGN work (e.g. transactional outbox on PG-class backends, insert-ordered scan on CH-class); load-test the envelope with the feed enabled (`cpt-cf-usage-collector-nfr-feed-lag`) |
| The Plugin SPI extension lands across independently-released plugins; a deployment may run a plugin without the accepted-order scan | Feed unavailable on that deployment while ingestion continues; consumer onboarding blocked | Capability-gated readiness (the feed advertises unavailability explicitly rather than degrading silently); coordinated SPI release per the stability NFR with a migration window |
| Replay horizon varies per deployment (plugin retention profile) | A new/recovering consumer may not reconstruct history older than the horizon | Deployment guide publishes the horizon; the first consumer (rating) persists normalized records immediately on consumption (mediation posture), so its correction horizon does not depend on the collector's retention |
| Watermark misread as a completeness promise by future consumers | Downstream closes treat late events as errors instead of corrections | Contract wording is explicit ("acceptance progress only"); the watermark FR obliges consumers to treat it as a lateness-reduction signal; rating-side handling is already correction-based |

## 13. Open Questions

- **Feed partition granularity** — per `tenant_id` (proposed) vs per `(tenant_id, gts_id)`; and
  correspondingly whether `acceptedSeq` is scoped per tenant or per partition. Per-tenant matches
  the first consumer's partitioning (the rating pipeline partitions on the ordering tenant and
  re-orders internally); finer partitions raise fan-out and watermark cardinality.
- **Canonical delivery lane** — SSE-with-resume as canonical with cursor-paged GET derived, or the
  inverse. The deterministic-replay obligation binds whichever lane is canonical.
- **`acceptedSeq` assignment mechanics per backend** — transactional outbox vs insert-ordered scan
  per plugin family; where the sequence is minted when ingestion runs multi-instance. DESIGN
  phase-2 work with plugin authors.
- **Watermark heartbeat cadence and idle-tenant behavior** — fixed cadence vs on-progress-only;
  how an idle `(tenant, UsageType)` communicates "no progress, nothing pending".
- **Feed fan-out limits** — whether feed subscriptions need per-consumer rate/fan-out controls,
  and how that composes with the (still-deferred) v1 rate-limiting item.

## 14. Traceability

- **Driving consumer requirement**: `gears/bss/rating/docs/SEAMS.md` §J — UC1 (transport; gates
  rating slice-12 implementation), UC2 (watermark), UC4 (dedup-key derivation), UC5 (correction
  visibility); rating design slice
  `gears/bss/rating/docs/design/12-usage-ingestion-normalization.md`.
- **Reserved hooks executed**: v1 [DESIGN.md](./DESIGN.md) §4 deferral table — "Push / subscribe
  surface for downstream readers" (SSE + SDK `Stream` hook; "a push surface must not couple
  downstream outages into ingestion"; pluggable storage admits backends with no native
  change-feed) and "Rate limiting & watermarks"; v1 [PRD.md](./PRD.md) §4.2 "Watermark and
  Reconciliation Metadata" ("covered in a later phase").
- **Stability envelope**: `cpt-cf-usage-collector-adr-contract-stability` (ADR-0006),
  `cpt-cf-usage-collector-nfr-plugin-contract-stability`.
- **Consistency baseline being complemented (not changed)**:
  `cpt-cf-usage-collector-adr-consistency-contract` (ADR-0011) — the polled Query SPI remains
  eventually consistent with no upper bound; the feed adds the ordered surface next to it.
- **v1 correction primitives carried onto the feed**:
  `cpt-cf-usage-collector-fr-usage-compensation` (ADR-0008),
  `cpt-cf-usage-collector-fr-event-deactivation` (ADR-0005).
