<!-- CONFLUENCE_TITLE: [BSS]: FX Rate Provider (Adapter Gear) — Technical Design -->
<!-- Related: ../../ledger/docs/DESIGN.md, ../../ledger/docs/design/06-fx-multicurrency.md, ../../ledger/docs/PRD.md | Owners: @vstudzinskyi (BSS Billing Platform team) -->

# Technical Design — FX Rate Provider (Adapter Gear)

<!-- toc -->

- [1. Architecture Overview](#1-architecture-overview)
  - [1.1 Architectural Vision](#11-architectural-vision)
  - [1.2 Architecture Drivers](#12-architecture-drivers)
  - [1.3 Architecture Layers](#13-architecture-layers)
- [2. Principles & Constraints](#2-principles--constraints)
  - [2.1 Design Principles](#21-design-principles)
  - [2.2 Constraints](#22-constraints)
- [3. Technical Architecture](#3-technical-architecture)
  - [3.1 Domain Model](#31-domain-model)
  - [3.2 Component Model](#32-component-model)
  - [3.3 API Contracts](#33-api-contracts)
  - [3.4 Internal Dependencies](#34-internal-dependencies)
  - [3.5 External Dependencies](#35-external-dependencies)
  - [3.6 Interactions & Sequences](#36-interactions--sequences)
  - [3.7 Database schemas & tables](#37-database-schemas--tables)
  - [3.8 Deployment Topology](#38-deployment-topology)
- [4. Additional context](#4-additional-context)
  - [Security & AuthZ](#security--authz)
  - [Feature metrics](#feature-metrics)
  - [Testing architecture](#testing-architecture)
  - [Decision register](#decision-register)
  - [Companion ledger change (hard dependency, from O-3)](#companion-ledger-change-hard-dependency-from-o-3)
- [5. Traceability](#5-traceability)

<!-- /toc -->

- [ ] `p1` - **ID**: `cpt-cf-bss-rate-provider-design-main`

> **Canonical design entry point.** This document is the FX rate-provider gear's technical
> design and the anchor for spec traceability. The gear is small enough that the design is
> **self-contained** — there is no slice set; component, contract, and sequence detail is
> normative here.
>
> **Status**: DRAFT — decisions recorded (O-1 & O-3 decided; all other defaults accepted
> 2026-07-08). Ready for implementation planning. The O-3 companion `bss-ledger` change
> (§4 "Companion ledger change") is a linked hard dependency.

## 1. Architecture Overview

### 1.1 Architectural Vision

The **FX rate-provider gear** is a **stateless adapter**: it implements the ledger's
`RateProviderV1` contract (`bss-ledger-sdk`, GTS `gts.cf.bss.ledger.rate-provider.v1`) and
registers an `Arc<dyn RateProviderV1>` into the platform `ClientHub`, so the ledger's
background `RateSyncJob` resolves a live adapter instead of the fail-safe
`UnconfiguredRateProviderV1` default. The registered instance is a
**`CompositeRateProvider`** that tries its ordered sources in the PRD-ratified provider
order (2026-06-10) and returns the **first whole document** that succeeds (all-or-nothing
per source, never a merge; O-1). The fallback **mechanism** ships in v1, but the **v1
configuration is ECB-only** (O-10): the bank / PSP feed is a *future* source, added later
as a `sources[]` config entry with no code change. It performs no persistence, no HTTP
surface, and no accounting logic.

**This gear only fetches rates.** The ledger declares the FX rate provider out of its own
scope ("The FX rate provider itself (integration, feeds) — external; the ledger consumes
rates and snapshots them",
[`../../ledger/docs/design/06-fx-multicurrency.md`](../../ledger/docs/design/06-fx-multicurrency.md))
and already ships the consuming seam: the `RateProviderV1` SDK trait, the `RateSyncJob`
that pulls it, the `ledger_fx_rate` local store, the lock-time `RateSource`, staleness /
provider-precedence resolution, and the immutable `rate_snapshot`. Everything ledger-owned
stays there and is NOT restated here:

- Functional-currency **translation** and the dual-column balance.
- **Triangulation** through EUR (X→EUR→Y) — ledger-owned (O-3); requires the companion
  ledger change (§4 "Companion ledger change").
- **Staleness** rules (G10 > 24 h; others ≤ 7 d) and `stale` marking.
- **Provider precedence / fallback-order** resolution over the local store.
- **`rate_snapshot`** freezing, `ledger_fx_rate` upsert, per-tenant fan-out.
- Realized / unrealized FX, revaluation runs.
- The `RateSyncJob` tick cadence and its `FX_SNAPSHOT_MISSING` alarm (ledger-side).

Also out of scope: **pricing-side FX / rate-lock governance** (Catalog module) and
**provider commercial contracts / credential procurement** (ops).

### 1.2 Architecture Drivers

Requirements from the ledger [`PRD.md`](../../ledger/docs/PRD.md) and the FX slice design
that significantly shape this gear.

#### Functional Drivers

| Requirement | Design Response |
|-------------|-----------------|
| `cpt-cf-bss-ledger-fr-multi-currency-fx` | The ledger needs a live rate feed to translate transaction currency into functional currency; this gear supplies it through the fixed `RateProviderV1` seam — implement `provider_id()`, `fetch_latest()`, `health()` (§3.3). |
| `cpt-cf-bss-ledger-fr-fx-rate-source-failure` | Provider outage must never produce a silent wrong rate. The composite returns the last `RateProviderError` when **all** sources fail; the ledger job then alarms and FX posts block (`FX_RATE_UNAVAILABLE`) — fail-safe by absence (§2.2). |
| Rate-source fallback ([ledger FX design](../../ledger/docs/design/06-fx-multicurrency.md)) | The ledger resolves precedence over its **local store**; cross-source fallback at fetch time is this gear's `CompositeRateProvider` — ordered sources, first whole successful document, true-source provenance (§3.2). |
| Provider onboarding without code change | Config-driven source assembly: a `kind`-keyed factory builds the active sources from config in fallback order; a plain REST feed is onboarded by config alone (`kind: http-json`), a new provider *family* costs one factory arm (§3.2). |

#### NFR Allocation

| NFR theme | Allocated to | Design Response |
|-----------|--------------|-----------------|
| Post-path isolation (hard) | Consumption model | `fetch_latest` is called only by the background `RateSyncJob`, never on the posting path; a provider outage fails the job (ledger alarms), never a post. |
| Feed freshness | Fetch path + ledger tick | A successful fetch SHOULD complete within one `rate_sync_tick` (ledger default 1 h) so G10 pairs never cross the 24 h staleness window under normal operation. |
| Fetch latency | Sources + HTTP client | `fx_provider_fetch_duration_seconds{provider}` p95 ≤ 2 s **per source** (draft; confirm against ECB response times). One bounded attempt per source per call, no unbounded retry. The composite's worst case is the **sum of the configured per-source `timeout_ms`** (every source down); this is acceptable because the fetch runs only inside the background `RateSyncJob`, never on the posting path — no shared total deadline is imposed in v1. |
| Availability | Ledger fail-safe | Best-effort; the ledger's fail-safe (block, not guess) absorbs adapter downtime. |

#### Key Decisions

The load-bearing decisions are recorded in the decision register (§4 "Decision register");
the two that shape the architecture:

| Decision | Summary |
|----------|---------|
| **O-1 — Composite adapter, no merge** | The ledger resolves ONE unscoped `get::<dyn RateProviderV1>()` and stamps every synced row with that single `provider_id()`, so per-provider registrations do not work today. This gear registers ONE `CompositeRateProvider` that returns the first whole successful document — a snapshot period stays single-source-coherent for audit. Source provenance is preserved via the last-served index (§3.2). |
| **O-3 — Ledger owns triangulation** | The adapter emits **only the source's native direct pairs** (ECB's EUR pairs) — no cross-rate synthesis here. Cross-base rates (X→EUR→Y) are computed ledger-side in `RateSource`; enabling the ledger's deferred triangulation is a hard companion dependency (§4). |

### 1.3 Architecture Layers

```text
Gear init()        builds the shared HTTP client · runs the kind-keyed factory over
(wiring)           config sources[] · assembles the composite · registers it in ClientHub
       │
       ▼
CompositeRateProvider   impl RateProviderV1 · ordered sources · first whole successful
(selection)             document · last-served provenance — source-agnostic
       │
       ▼
Sources            EcbRateProvider (kind=ecb: XML fetch/parse) ·
(impl per kind)    HttpJsonRateProvider (kind=http-json: generic GET-JSON + mapping)
       │
       ▼
HTTP client        reqwest + rustls, outbound HTTPS only
                   → ECB eurofxref-daily.xml (primary) · bank/PSP feed (fallback, post-v1 O-10)
```

The ledger-side `RateSyncJob` (outside this boundary) resolves the composite with
`client_hub().get::<dyn RateProviderV1>()`, calls `fetch_latest(ctx, &[], request_id)`
once per tick, then reads `provider_id()` for the row stamp.

## 2. Principles & Constraints

### 2.1 Design Principles

#### Fetch-only adapter

- [ ] `p1` - **ID**: `cpt-cf-bss-rate-provider-principle-fetch-only`

The gear fetches rates and nothing else: no persistence, no translation, no triangulation,
no staleness marking, no snapshotting — those are ledger-owned (§1.1). `fetch_latest` MUST
be side-effect-free and safe to call repeatedly (the ledger job is idempotent).

#### Config-driven source assembly

- [ ] `p1` - **ID**: `cpt-cf-bss-rate-provider-principle-config-driven-assembly`

The active sources and their fallback order are config — the composite is assembled at
`init()`, never hardcoded. Add / remove / reorder providers by editing config; a new
provider *family* costs one factory arm; a new *simple REST feed* costs zero code
(`kind = http-json`).

#### All-or-nothing per source

- [ ] `p1` - **ID**: `cpt-cf-bss-rate-provider-principle-all-or-nothing-source`

Fallback triggers only when a source returns `Err` for the whole fetch — never per missing
pair, never a cross-source merge. A pair absent from the chosen source's document is simply
absent (the ledger treats it as no rate). A snapshot period stays single-source-coherent
for audit (O-1).

#### Direct pairs only

- [ ] `p1` - **ID**: `cpt-cf-bss-rate-provider-principle-direct-pairs-only`

A source emits only its natively published pairs. A requested pair the source cannot serve
is **omitted** (not an error), never synthesized — cross-base derivation is the ledger's
triangulation concern (O-3).

#### Deterministic conversion

- [ ] `p1` - **ID**: `cpt-cf-bss-rate-provider-principle-deterministic-conversion`

`rate → rate_micro` conversion parses the published decimal into an **exact decimal
representation** (never binary floating point) and rounds with banker's rounding
(half-to-even), matching the platform ledger rounding default, so a re-fetch of the same
published rate yields the same integer (§3.2, O-4).

#### Provider time, not fetch time

- [ ] `p1` - **ID**: `cpt-cf-bss-rate-provider-principle-provider-as-of`

`as_of` MUST be the provider's publication timestamp normalized to UTC — never `now()`.
On non-publication days (weekends / TARGET holidays) the last published rate is returned
with its original `as_of`, so the ledger's staleness rule still applies.

### 2.2 Constraints

#### Fixed SDK contract

- [ ] `p1` - **ID**: `cpt-cf-bss-rate-provider-constraint-fixed-sdk-contract`

The `RateProviderV1` trait, `ProviderRate`, `CurrencyPair`, and `RateProviderError` are
**already defined** in `bss-ledger-sdk` and MUST NOT be changed without a ledger-side
change (GTS `gts.cf.bss.ledger.rate-provider.v1`).

#### Never on the posting path

- [ ] `p1` - **ID**: `cpt-cf-bss-rate-provider-constraint-off-posting-path`

`fetch_latest` is called only by the background `RateSyncJob`. A provider outage fails the
job (ledger alarms), never a post. The adapter MUST NOT retry indefinitely; one bounded
attempt per call — the ledger job schedules the next tick.

#### Stateless

- [ ] `p1` - **ID**: `cpt-cf-bss-rate-provider-constraint-stateless`

The adapter holds no DB and no per-tenant state; a provider publishes **global** rates and
the ledger fans them out per tenant. The only in-memory state is the composite's
last-served source index (interior mutability, no persistence).

#### Fail-safe by absence

- [ ] `p1` - **ID**: `cpt-cf-bss-rate-provider-constraint-fail-safe-by-absence`

If the adapter is not registered, the ledger uses `UnconfiguredRateProviderV1` → the local
store stays empty → FX posts block (`FX_RATE_UNAVAILABLE`), never a silent wrong rate.

#### Rate precision

- [ ] `p1` - **ID**: `cpt-cf-bss-rate-provider-constraint-rate-micro-precision`

`ProviderRate.rate_micro` is the functional-per-unit-transaction multiplier × 1e6, `i64`
(O-5: kept for v1; revisit for high-unit / crypto pairs — any change is an SDK change).
Overflow / non-finite values MUST map to `RateProviderError::Internal`, never a silent
truncation.

#### Secrets handling

- [ ] `p1` - **ID**: `cpt-cf-bss-rate-provider-constraint-secrets`

Any provider API key MUST come from config `${VAR}` expansion or the platform CredStore —
never hardcoded, never logged, never in this document.

## 3. Technical Architecture

### 3.1 Domain Model

The domain types are **inherited from `bss-ledger-sdk`** (`rate_provider.rs`) and NOT
redefined here (constraint `cpt-cf-bss-rate-provider-constraint-fixed-sdk-contract`).

#### Type: `CurrencyPair`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `base` | string (ISO 4217) | Yes | Transaction currency |
| `quote` | string (ISO 4217) | Yes | Functional currency |

#### Type: `ProviderRate`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `base` | string | Yes | Base (transaction) currency |
| `quote` | string | Yes | Quote (functional) currency |
| `rate_micro` | int64 | Yes | Functional-per-unit-base × 1e6 (fixed precision) |
| `as_of` | timestamp (UTC) | Yes | Provider publication time; drives ledger staleness |

#### Enum: `RateProviderError`

| Value | Description |
|-------|-------------|
| `PairUnavailable { base, quote }` | Provider does not publish this pair |
| `Unreachable(msg)` | Network / DNS / timeout |
| `UpstreamStatus(u16)` | Non-success HTTP status |
| `InvalidPair(msg)` | Malformed / unknown currency code |
| `Internal(msg)` | Parse / conversion fault |

### 3.2 Component Model

Each component carries a stable `cpt-cf-bss-rate-provider-component-{slug}` ID.

#### Source factory & configuration

- [ ] `p1` - **ID**: `cpt-cf-bss-rate-provider-component-source-factory`

`init()` maps each config `sources[]` entry through `build_source` in order, then wraps the
results in the composite. `build_source(SourceConfig, shared HTTP client) →
Box<dyn RateProviderV1>` matches on `kind`; an unknown `kind` MUST be **rejected at
`init()`** (fail loud, not at first fetch). An **empty `sources[]` MUST also fail
`init()`** — a composite with zero sources cannot serve `provider_id()` (`last_served`
would index a nonexistent source) and has no "last error" to return from `fetch_latest`.
The `sources` order MUST match the ledger `fx.provider_order`; a mismatch is a
**configuration error and MUST fail `init()`** (O-12) — a warn-only mismatch would let the
composite fetch from one provider while the ledger's precedence resolution later prefers a
different provider's stored rate.

**Module config:**

```yaml
gears:
  fx-rate-provider:
    config:
      sources:                 # order = fallback order (MUST align with ledger fx.provider_order)
        - id: ecb
          kind: ecb
          base_url: "https://www.ecb.europa.eu/stats/eurofxref/eurofxref-daily.xml"
          timeout_ms: 5000
        - id: bank-x            # ILLUSTRATIVE — not part of v1 (O-10: v1 ships ECB-only)
          kind: http-json
          base_url: "${BANK_X_URL}"
          api_key:  "${BANK_X_KEY}"
          auth: bearer
          mapping: { base: "USD", rates: "$.rates", rate: "value", as_of: "$.date" }
```

**`SourceConfig` (common fields, every `kind`):**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | string | Yes | Stable `provider_id` stamped on synced rows; MUST be a member of the ledger `fx.provider_order`. |
| `kind` | string | Yes | Selects the implementation via the factory (`ecb`, `http-json`, …). |
| `base_url` | string | Yes | Source endpoint. |
| `timeout_ms` | integer | No (5000) | Outbound HTTP timeout. |
| `api_key` | string (secret) | No | `${VAR}` / CredStore only; never logged. |
| *(kind-specific)* | — | — | Extra fields per implementation (ECB `format`; http-json `mapping`/`auth`). |

**Adding a provider:**

- *Simple REST feed* → add a `sources` entry with `kind: http-json` + `mapping`. **No code.**
- *New family (quirky format/auth)* → implement `RateProviderV1`, add one `build_source` arm, reference it by `kind`.

#### `CompositeRateProvider`

- [ ] `p1` - **ID**: `cpt-cf-bss-rate-provider-component-composite`

The single `RateProviderV1` instance registered in `ClientHub` (the ledger resolves exactly
one — O-1). Wraps the **ordered** `Vec<Box<dyn RateProviderV1>>` produced by the factory
and does the fallback the ledger cannot (the ledger stamps one `provider_id` per sync
pass). Source-agnostic — it never names a concrete source. Configuration: none of its own —
the try order **is** the config `sources[]` order.

**State:** `last_served: AtomicUsize` — index of the source that produced the most recent
successful document (default `0` = primary). Interior mutability only; no persistence.

| Operation | Input | Output | Key Behavior |
|-----------|-------|--------|--------------|
| `fetch_latest` | `ctx`, `pairs`, `request_id` | `Vec<ProviderRate>` | Try sources **in order**; return the **first** that yields `Ok(document)` **whole** (no merge). On success, record its index in `last_served`. If **all** sources fail, return the last `RateProviderError` (the ledger job then raises `FX_SNAPSHOT_MISSING`). |
| `provider_id` | — | `&str` | Return `sources[last_served].provider_id()` — the **real** source that served last (`"ecb"` / `"bank-x"`), so `ledger_fx_rate.provider` and `rate_snapshot.provider` record the true upstream. |
| `health` | `ctx`, `request_id` | `()` | `Ok(())` if **any** source is healthy (ordered probe). |

**Behavioral rules:**

- **Provenance correctness depends on call order.** `provider_id()` reflects
  `last_served`, which is set during `fetch_latest`. This is correct **because**
  `RateSyncJob` calls `fetch_latest` before `provider_id` in the same pass
  (rate_sync.rs:111 then :149). A single non-concurrent ticker + `AtomicUsize` makes this
  race-free. Flagged as a residual coupling (O-7a): if the ledger job is ever refactored or
  made concurrent, revisit — or push a ledger change so `ProviderRate` carries its own
  source id. Assumption MUST be noted in code + tests.
- **Startup default.** Before any successful fetch, `last_served = 0`; `provider_id()`
  returns the primary source's id.

#### `EcbRateProvider` (source `kind = ecb`)

- [ ] `p1` - **ID**: `cpt-cf-bss-rate-provider-component-ecb-source`

HTTP fetch + XML parse + `rate_micro` conversion + error mapping over the ECB daily feed.
Dependencies: the shared `reqwest::Client` (built once in `init()`), module config.
Configuration: the common `SourceConfig` fields — `id` default `"ecb"`, `base_url` = the
ECB daily feed, `timeout_ms` = `5000` — plus ECB-specific:

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `format` | enum `xml` \| `sdmx` | `xml` | ECB payload format (O-2: direct ECB daily XML for prod; Frankfurter allowed for dev; SDMX optional). |

| Operation | Input | Output | Key Behavior |
|-----------|-------|--------|--------------|
| `provider_id` | — | `&str` | Returns the configured stable id. |
| `fetch_latest` | `ctx`, `pairs: &[CurrencyPair]`, `request_id` | `Vec<ProviderRate>` | GET latest feed → parse → convert. `pairs = &[]` ⇒ return the **whole** published table. Requested pairs the source cannot serve are **omitted** (not an error). Map transport failures to `Unreachable` / `UpstreamStatus`. |
| `health` | `ctx`, `request_id` | `()` | Cheap reachability probe (HEAD or a minimal GET); default trait impl delegates to `fetch_latest(&[])`. |

**ECB payload handling:**

- **Direct pairs published by ECB are EUR-based** (EUR→X). A `CurrencyPair` whose
  `base`/`quote` is not directly published is **omitted** — never synthesized (O-3). In
  particular the **inverse leg X→EUR is NOT emitted here** (e.g. `USD→EUR` for a USD
  transaction under an EUR functional currency): deriving it by **deterministic inversion**
  of the stored EUR→X rate is part of the ledger's triangulation (O-3; §4 "Companion
  ledger change").
- **Non-publication days** (weekends / TARGET holidays): return the last published rate
  with its original `as_of` (staleness is the ledger's call).
- **Cadence assumption:** ECB publishes once per TARGET business day ~16:00 CET; on
  non-publication days the last published rate is returned (its `as_of` unchanged, so the
  ledger's staleness rule still applies).

#### `HttpJsonRateProvider` (generic, source `kind = http-json`)

- [ ] `p2` - **ID**: `cpt-cf-bss-rate-provider-component-http-json-source`

A configurable GET-JSON source so a plain REST rate feed is onboarded by **config alone**.
Covers the common "fetch a JSON document of rates, map fields" shape; NOT for quirky
sources (ECB XML above, or a PSP settlement feed with signed auth — those get their own
`kind`). Dependencies: the shared `reqwest::Client`; the source's `SourceConfig` incl.
`mapping` + `auth`.

**Configuration (kind-specific, added to `SourceConfig`):**

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `mapping.base` | JSON path or literal | — | Base currency: a literal (single-base feed, e.g. `"USD"`) or a path. |
| `mapping.rates` | JSON path | — | The collection/object of quote→rate entries. |
| `mapping.rate` | field | — | The numeric rate field within each entry. |
| `mapping.as_of` | JSON path | — | Publication timestamp; parsed to UTC. |
| `auth` | enum `none` \| `bearer` \| `header-key` | `none` | How `api_key` is presented. |

| Operation | Input | Output | Key Behavior |
|-----------|-------|--------|--------------|
| `fetch_latest` | `ctx`, `pairs`, `request_id` | `Vec<ProviderRate>` | GET `base_url` (with `auth`) → parse JSON → apply `mapping` → convert each to `ProviderRate`. `pairs = &[]` ⇒ whole document. An entry that fails mapping is skipped (counted), never fabricated; a document where **zero** entries map ⇒ `RateProviderError::Internal` (behavioral rules below). |
| `provider_id` | — | `&str` | The configured `id`. |
| `health` | `ctx`, `request_id` | `()` | Minimal GET; maps transport failure to `Unreachable`. |

**Behavioral rules:**

- **Base-currency shape.** Many free feeds are single-base. Config states the base; a
  requested pair whose base ≠ the feed base is **omitted**, never synthesized here (O-3).
- **Deterministic mapping.** An unresolvable field ⇒ skip that entry with a counted
  warning; a wholesale parse failure ⇒ `RateProviderError::Internal`. A syntactically
  valid document from which **zero entries map** MUST also return
  `RateProviderError::Internal` — returning `Ok([])` would read as success, suppress the
  composite fallback, and let the ledger mark the sync pass successful without refreshing
  a single rate.
- **Scope (O-11):** v1 = single-base JSON feeds, simple field paths,
  `none` / `bearer` / `header-key` auth; richer transforms (multi-base, JSON-path dialects,
  custom date/number formats) deferred.

#### `rate → rate_micro` conversion

- [ ] `p1` - **ID**: `cpt-cf-bss-rate-provider-component-rate-micro-conversion`

ECB quotes ~5 significant digits. Convert `rate` (decimal) to
`rate_micro = round(rate × 1_000_000)` using **banker's rounding (half-to-even)** to match
the platform ledger rounding default, so a re-fetch of the same published rate yields the
same integer (O-4: accepted; final sign-off with Finance/audit still to be obtained).
The published decimal string MUST be parsed into an **exact decimal representation** (an
integer-scaled or arbitrary-precision decimal type, e.g. `BigDecimal`) — **never a binary
`f64`**, whose nearest-representable value can mis-round exact half-way decimals under
half-to-even. Rounding and the ×1e6 scaling are applied in decimal space with an explicit
`i64` overflow check. Overflow / non-finite / non-numeric values MUST map to
`RateProviderError::Internal` (never a silent truncation).

#### Gear wiring

- [ ] `p1` - **ID**: `cpt-cf-bss-rate-provider-component-gear-init`

`init()` builds the shared HTTP client, runs the factory over config, assembles the
composite, and registers it in `ClientHub`; plus workspace + `registered_modules.rs`
registration. Bank / PSP settlement feed: the generic `http-json` source if it is a plain
REST feed, else a dedicated `kind` for signed/settlement auth (concrete feed is O-10:
v1 = ECB-only; bank/PSP added later as a `sources[]` entry).

### 3.3 API Contracts

This gear exposes **no external REST surface** and produces **no events**. It is consumed
in-process by the ledger via the `RateProviderV1` trait resolved from `ClientHub`
(GTS `gts.cf.bss.ledger.rate-provider.v1`).

| Surface | Direction | Contract | Notes |
|---------|-----------|----------|-------|
| `RateProviderV1::fetch_latest` | inbound (from ledger) | SDK trait | One round-trip per tick; `&[]` = whole table. |
| `RateProviderV1::health` | inbound (from ledger) | SDK trait | Reachability probe. |
| ECB / bank feed | outbound | HTTPS GET | External provider; see §4 "Security & AuthZ". |

**Relationship to the ledger's manual ingest.** The `RateProviderV1` pull driven by
`RateSyncJob` is the **PRIMARY** rate path. The ledger separately exposes a **SECONDARY**
manual/seed path — `POST /bss-ledger/v1/fx/rates` (ledger-owned, `(ledger, provision)` PEP
gate) — that upserts one rate directly into `ledger_fx_rate`. This gear does **not** own or
replace that endpoint; the two are complementary (automated feed vs manual break-glass /
bootstrap).

**Events.** Provider-outage signalling is the ledger's `RateSyncJob`, which emits
`billing.ledger.invariant.alarm` with `alarmCategory = fx-snapshot-missing` (Critical) when
a **configured** provider fails to fetch. The adapter only returns a `RateProviderError`;
the ledger decides the alarm.

An optional debug/liveness HTTP endpoint is deferred (O-6: metrics only for v1).

### 3.4 Internal Dependencies

- **`bss-ledger-sdk`** — the `RateProviderV1` trait and its types (`rate_provider.rs`); the fixed contract this gear implements.
- **ToolKit `ClientHub`** — cross-gear registry; `init()` registers the composite, the ledger resolves it.
- **`reqwest` + `rustls`** — outbound HTTPS client, built once in `init()` and shared by all sources.
- **Platform OTel meter** — the gear owns its own metrics handle wired at `init()` (§4 "Feature metrics").

### 3.5 External Dependencies

- **ECB reference rates** — primary source; free, published once per TARGET business day (`eurofxref-daily.xml`).
- **Bank / PSP feed** — fallback / settlement evidence; deferred to ops (O-10), onboarded via config when available.
- **Billing Ledger (`bss-ledger`)** — the sole consumer: `RateSyncJob` pulls `fetch_latest` and stamps rows with `provider_id`; `RateSource` and the FX stores consume the synced rates ([`../../ledger/docs/design/06-fx-multicurrency.md`](../../ledger/docs/design/06-fx-multicurrency.md)).

### 3.6 Interactions & Sequences

#### Sync tick → fetch → stamp

- [ ] `p1` - **ID**: `cpt-cf-bss-rate-provider-seq-sync-tick-fetch`

Once per tick the ledger `RateSyncJob` resolves the composite from `ClientHub`, calls
`fetch_latest(ctx, &[], request_id)` (whole table), then reads `provider_id()` for the row
stamp and upserts into `ledger_fx_rate`. The caller context is
`SecurityContext::anonymous()` (system context) — no PEP gate on this internal cross-gear
plugin call.

#### Source fallback with provenance

- [ ] `p1` - **ID**: `cpt-cf-bss-rate-provider-seq-source-fallback`

Primary source fails (`Err` for the whole fetch) → the composite tries the next source in
config order → the first `Ok(document)` is returned **whole** and its index recorded in
`last_served` → the subsequent `provider_id()` reports the serving source's real id, so the
synced rows record the true upstream.

#### All sources fail → ledger alarm

- [ ] `p1` - **ID**: `cpt-cf-bss-rate-provider-seq-all-sources-fail`

Every source returns `Err` → the composite returns the last `RateProviderError` → the
ledger job raises `FX_SNAPSHOT_MISSING` (`billing.ledger.invariant.alarm`,
`alarmCategory = fx-snapshot-missing`, Critical). The local store keeps its last synced
rates; staleness marking and post blocking are the ledger's call.

### 3.7 Database schemas & tables

**None.** The adapter is stateless — no tables, no migrations. The persisted FX state is
owned by the ledger:

- `ledger_fx_rate` — the local "latest known rates" store (`RateSyncJob` upsert target).
- `rate_snapshot` — the immutable per-lock frozen rate.

Both are defined in the ledger FX / Foundation designs
([`../../ledger/docs/design/06-fx-multicurrency.md`](../../ledger/docs/design/06-fx-multicurrency.md),
[`../../ledger/docs/design/01-repository-foundation.md`](../../ledger/docs/design/01-repository-foundation.md))
and are NOT part of this gear.

### 3.8 Deployment Topology

A stateless adapter gear at `gears/bss/rate-provider` (placement per O-8; `provider_id =
"ecb"` for the primary source), deployed in-process with the platform gear set — no
standalone service, no DB. Startup ordering (O-7): the adapter must register in `ClientHub`
**before** the ledger reads it; accepted mitigation is the fail-safe + next tick (a missed
first tick self-heals), with a ledger `deps` edge to be added if ordering proves unreliable
during implementation.

## 4. Additional context

### Security & AuthZ

- **Caller context:** the ledger calls `fetch_latest` with `SecurityContext::anonymous()`
  (system context, not a per-request user). No PEP gate on this trait — it is an internal
  cross-gear plugin call, not a tenant-scoped resource.
- **No tenant data:** rates are global reference data; the adapter never sees tenant PII
  and never writes tenant-scoped rows (the ledger does the RLS-scoped fan-out).
- **Outbound TLS:** HTTPS with `rustls`. ECB is public/unauthenticated; paid providers need
  an API key (see constraint `cpt-cf-bss-rate-provider-constraint-secrets`).
- **Outbound URL validation & redirects:** every `base_url` MUST be `https://` and is
  validated at `init()` (fail loud on plain `http`; loopback / private-network hosts are
  rejected unless explicitly allow-listed for dev). The shared HTTP client's redirect
  policy MUST NOT carry `api_key` credentials across a host or scheme change — a
  cross-host redirect is either refused or re-issued without auth headers. Config is
  operator-supplied (not tenant input), so this is defense-in-depth against
  misconfiguration, not a tenant-facing SSRF surface.
- **Provider authenticity:** trusting the provider feed's authenticity is upstream/ops.

### Feature metrics

All metrics exposed as Prometheus scrape targets. (Provider **fallback** selection is
measured ledger-side as `ledger_fx_provider_fallback_total{provider}`, emitted at lock time
by `RateSource`; the adapter measures the fetch itself.)

| Vector | Metric | Description | Target Threshold |
|--------|--------|-------------|------------------|
| **Efficiency** | `fx_provider_fetch_duration_seconds{provider}` | Outbound fetch+parse latency | p95 ≤ 2 s |
| **Performance** | `fx_provider_rates_returned{provider}` | Pairs returned per successful fetch | — |
| **Reliability** | `fx_provider_fetch_errors_total{provider,kind}` | Fetch failures by `RateProviderError` kind | — |
| **Reliability** | `fx_provider_last_success_timestamp{provider}` | Unix time of last successful fetch (feed-freshness gauge) | — |
| **Security** | `fx_provider_upstream_status_total{provider,status}` | Upstream HTTP status distribution | — |

**Instrumentation ownership.** These are the **adapter's own** instruments (the fetch
happens inside the source, out of the ledger's sight), so this gear MUST own a metrics
handle — it does not piggy-back on the ledger's meter. Wire it at `init()` from the
platform OTel meter; each source records under its own `provider_id` label. The
`{provider}` label is the source `provider_id` (`"ecb"` / `"bank-x"`); for the composite,
the source that actually served.

### Testing architecture

| Level | Database | Network | What is real | What is mocked |
|---|---|---|---|---|
| **Unit** | None | None | Parser, `rate_micro` conversion, error mapping, `provider_id` | HTTP client (fixture bytes) |
| **Integration** | None | In-process fake HTTP server | `EcbRateProvider` end-to-end over a local `ecb`-shaped server | The real ECB endpoint |
| **API** | N/A | In-process | Trait-level: `fetch_latest`/`health` contract behavior | — (no REST surface) |
| **E2E** | Real (ledger) | Real HTTPS | Adapter registered in ClientHub; ledger `RateSyncJob` populates `ledger_fx_rate`; an EUR-invoice-under-USD post locks a rate | Optionally the live ECB feed (gated) |

**Unit tests** (mock boundary: `FakeHttpTransport` feeding canned payload bytes / errors):

| What to test | What is mocked | Verification target |
|---|---|---|
| Parse ECB daily XML fixture → `Vec<ProviderRate>` | `FakeHttpTransport` | All EUR pairs decoded; `as_of` = publication date UTC |
| `rate_micro` conversion determinism | none | `round(rate×1e6)` half-to-even over exact decimal parsing (no `f64` path); half-way golden vectors; same input → same i64 |
| Requested pair not published | `FakeHttpTransport` | Pair omitted from result (not an error) |
| Upstream 5xx | `FakeHttpTransport` (503) | `RateProviderError::UpstreamStatus(503)` |
| Network failure | `FakeHttpTransport` (conn err) | `RateProviderError::Unreachable` |
| Malformed payload | `FakeHttpTransport` (garbage) | `RateProviderError::Internal` |
| Overflow / non-finite rate | crafted fixture | `RateProviderError::Internal`, no truncation |
| `provider_id` | none | Returns configured id (default `"ecb"`) |
| Factory rejects unknown `kind` | none | `build_source` on an unknown `kind` fails loud at `init()` (not at first fetch) |
| Factory rejects empty `sources[]` | none | `init()` fails loud — never a composite with zero sources |
| Factory rejects `sources[]` / `fx.provider_order` mismatch | none | `init()` fails loud (O-12) |
| Fully-unmappable `http-json` document | `FakeHttpTransport` (valid JSON, zero mappable entries) | `RateProviderError::Internal`, never `Ok([])` — composite fallback must trigger |
| Generic `http-json` mapping → `Vec<ProviderRate>` | `FakeHttpTransport` (JSON body) | `mapping` resolves base/quote/rate/as_of; an unmappable entry is skipped (counted), never fabricated |
| Composite fallback order + provenance | two fake sources (primary fails, secondary ok) | Secondary document returned whole; `provider_id()` reports the secondary's id |

**Integration tests** (a lightweight in-process HTTP server, e.g. `wiremock`/`axum`,
serving ECB-shaped payloads; no DB):

| What to test | Setup | Verification target |
|---|---|---|
| Full fetch over local server | Serve `eurofxref-daily.xml` fixture | `fetch_latest(&[])` returns the full table |
| Whole-table vs specific pairs | Serve full feed | `&[EUR→USD]` returns only that pair; unknown pair omitted |
| `health` probe | Serve 200 / 503 | `Ok(())` vs `Unreachable` |
| Timeout | Server delays > `request_timeout_ms` | `Unreachable` (not a hang) |

**API tests:** no REST surface — the "contract" tests are the trait-level behaviors covered
at Unit/Integration. If a debug endpoint is ever added (O-6), add RFC 9457 error tests then.

**E2E tests** (planned location: `testing/e2e/modules/bss-ledger/`, extends the FX suite):

| What to test | Marker | Verification target |
|---|---|---|
| Adapter registered → ledger sync populates store | `@pytest.mark.smoke` | After a `RateSyncJob` tick, `GET /fx/rate-snapshots` path is servable; a cross-currency post locks a rate (no `FX_RATE_UNAVAILABLE`) |
| Provider unreachable → post blocks | — | With the adapter down, an EUR-under-USD post returns `FX_RATE_UNAVAILABLE` (fail-safe), and `fx-snapshot-missing` alarm fires |
| Live ECB fetch (gated) | `@pytest.mark.external` | A real ECB fetch returns a non-empty EUR table |

**What must NOT be mocked:**

| Component | Why |
|---|---|
| `rate_micro` conversion | Money precision — must be exact and deterministic against real parsing |
| The `RateProviderV1` contract behavior (`&[]` semantics, omit-on-unavailable) | The ledger job relies on it verbatim |
| Ledger fail-safe (block on empty store) — E2E | Proves "block, not guess" end to end |

**NFR verification mapping:**

| NFR | Test level | How verified |
|---|---|---|
| Post-path isolation | E2E | Provider down → posts still fast; only FX posts block |
| Fetch latency p95 ≤ 2 s | Integration + load | Timed fetch against local server; sample live ECB |
| Deterministic conversion | Unit | Golden-vector tests over the conversion function |
| Feed freshness | E2E | Sync tick populates store within the tick window |

### Decision register

| Ref | Item | Resolution | Owner |
|-----|------|------------|-------|
| **O-1** | Multiple providers vs single `dyn RateProviderV1` | ✅ **DECIDED — composite adapter, no merge.** ONE `CompositeRateProvider` registered; ordered sources; first whole document; provenance via last-served index (§3.2). Variant (b) — a ledger-side scoped multi-provider loop — stays a future option if per-pair fallback is ever needed. Residual coupling → O-7a. | Architecture |
| **O-2** | ECB source & format | ✅ **Accepted (2026-07-08):** direct ECB daily XML for prod; Frankfurter allowed for dev; SDMX optional. | PM + Architecture |
| **O-3** | Triangulation ownership | ✅ **DECIDED (2026-07-08) — the ledger owns triangulation.** The adapter emits only native direct pairs; cross-base rates are computed ledger-side in `RateSource`. Companion ledger change required (below). | Architecture |
| **O-4** | Conversion rounding mode | ✅ **Accepted (2026-07-08):** banker's rounding (half-to-even), matching the ledger default; final sign-off with Finance/audit still to be obtained. | PM + Finance |
| **O-5** | `rate_micro` precision sufficiency | ✅ **Accepted (2026-07-08):** keep ×1e6 (6 dp) for v1; revisit for high-unit / crypto pairs (any change is an SDK change). | Architecture |
| **O-6** | Debug/observability endpoint | ✅ **Accepted (2026-07-08):** metrics only for v1 — no debug HTTP endpoint; ops rely on metrics + the trait `health`. | Team |
| **O-7** | Gear vs plugin & startup order | ✅ **Accepted (2026-07-08):** rely on the fail-safe + next tick; verify startup ordering during implementation (add a ledger `deps` edge if ordering proves unreliable). | Architecture |
| **O-7a** | Composite provenance coupling (from O-1) | ✅ **Accepted for v1;** assumption noted in code + tests. `provider_id()` reflects the last-served source, correct only while `RateSyncJob` calls `fetch_latest` before `provider_id` in one pass (true today: rate_sync.rs:111 then :149, single ticker). If the job is refactored or made concurrent, revisit — or push a ledger change so `ProviderRate` carries its own source id. | Architecture |
| **O-8** | Crate placement & naming | ✅ **Accepted (2026-07-08):** `gears/bss/rate-provider`, `provider_id = "ecb"` (confirm against gear conventions at implementation). | Team |
| **O-9** | Jira / slice linkage | ✅ **Accepted (2026-07-08):** create a Technical task under the Slice-5 FX epic (VHP-1853 / VHP-1986 family), linked to the O-3 companion ledger ticket — action pending. | PM |
| **O-10** | Bank / PSP fallback source | ✅ **Accepted (2026-07-08):** v1 = ECB-only; bank/PSP added later as a `sources[]` entry (generic `http-json` if a plain REST feed, else a dedicated `kind` for signed/settlement auth). Concrete feed + credentials deferred to ops. | PM + Ops |
| **O-11** | Generic `http-json` mapping grammar | ✅ **Accepted (2026-07-08):** v1 = single-base JSON feeds, simple field paths, `none` / `bearer` / `header-key` auth; richer transforms deferred. | Architecture |
| **O-12** | `init()` config-validation strictness | ✅ **DECIDED (2026-07-17):** fail `init()` loud on an unknown `kind`, an empty `sources[]`, or a `sources[]` order that does not match the ledger `fx.provider_order` — a mismatch would let the composite fetch one provider while the ledger's precedence resolution prefers another's stored rate. | Architecture |

### Companion ledger change (hard dependency, from O-3)

O-3 puts triangulation in the ledger, so this gear ships **direct pairs only**. Enabling
the ledger's deferred triangulation is therefore a **hard dependency**, tracked as a
separate `bss-ledger` work item — NOT part of this gear:

- **Where:** `bss-ledger` `infra/fx/rate_source.rs` — today `resolve()` reads direct pairs
  only (a documented TODO); it MUST compute `X → EUR → Y` (via the configured bridge
  currency) when no direct pair exists. This **includes deriving the `X → EUR` leg by
  deterministically inverting** the stored `EUR → X` rate — the adapter emits only ECB's
  native EUR-based pairs, so without ledger-side inversion no non-EUR-base pair (e.g.
  `USD→EUR`) can resolve at all.
- **Snapshot:** the resulting `rate_snapshot` MUST record `triangulated_via` (the bridge
  currency) — the column already exists on `ledger_fx_rate_snapshot`.
- **Determinism:** the bridge path + rounding MUST be deterministic and
  auditor-reproducible (banker's rounding per O-4).
- **Sequencing:** this adapter can ship first — EUR-functional / EUR-base tenants already
  work with direct pairs. Non-EUR-functional tenants are unblocked only once the ledger
  triangulation lands. Track the two as linked tickets (O-9).

## 5. Traceability

- **PRD (this gear)**: [`PRD.md`](./PRD.md) — the adapter's own product requirements
  (`cpt-cf-bss-rate-provider-fr-*` / `-nfr-*`), derived from the ledger PRD below.
- **Upstream PRD**: [`../../ledger/docs/PRD.md`](../../ledger/docs/PRD.md) — § Multi-currency and
  foreign exchange, § FX rate-source failure and staleness
  (`cpt-cf-bss-ledger-fr-multi-currency-fx`, `cpt-cf-bss-ledger-fr-fx-rate-source-failure`).
- **Consuming design**:
  [`../../ledger/docs/design/06-fx-multicurrency.md`](../../ledger/docs/design/06-fx-multicurrency.md)
  (the ledger side: `RateSource`, staleness, snapshots, the rate-source-fallback
  algorithm, and the frozen rate-snapshot state) and
  [`../../ledger/docs/design/01-repository-foundation.md`](../../ledger/docs/design/01-repository-foundation.md)
  (functional columns, currency-scale registry).
- **Code seam (existing)**: `bss-ledger-sdk` `rate_provider.rs` (`RateProviderV1` trait);
  `bss-ledger` `infra/jobs/rate_sync.rs`, `infra/fx/rate_source.rs`, `config.rs`
  (`FxConfig`), `module.rs` (ClientHub resolution).
- **Provenance**: authored from the architecture-repo draft
  `DESIGN-billing-fx-module-202607011613` (vhp-architecture, `docs/bss/design/`), which
  itself traces to `PRD-billing-ledger-balances-202604041200`,
  `DESIGN-billing-ledger-balances-202606091200` (slices 01 / 06), and
  `ADR-platform-persistence-layer-202601221200`.
