<!-- CONFLUENCE_TITLE: [BSS]: Products — Derived Usage Types (Feature) -->
<!-- Related: ../DECOMPOSITION.md, ../DESIGN.md, ../PRD.md, ../DECISIONS.md | Owners: BSS Product Catalog team -->

# Feature: Derived Usage Types

- [ ] `p1` - **ID**: `cpt-cf-bss-products-featstatus-derived-usage-types-implemented`

<!-- reference to DECOMPOSITION entry -->
- [ ] `p1` - `cpt-cf-bss-products-feature-derived-usage-types`

<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Author declares a derived usage type](#author-declares-a-derived-usage-type)
  - [Pricing author reads a version](#pricing-author-reads-a-version)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [declaration-judged](#declaration-judged)
  - [inputs-resolve](#inputs-resolve)
  - [version-append](#version-append)
- [4. States (CDSL)](#4-states-cdsl)
  - [Derived usage type states](#derived-usage-type-states)
- [5. Definitions of Done](#5-definitions-of-done)
  - [Append-only store on both backends](#append-only-store-on-both-backends)
  - [Declarations judged by the SDK's rules](#declarations-judged-by-the-sdks-rules)
  - [Five doors with their grants and audit](#five-doors-with-their-grants-and-audit)
- [6. Acceptance Criteria](#6-acceptance-criteria)

<!-- /toc -->

## 1. Feature Context

### 1.1 Overview

This feature stores and serves derived usage types: composite meters, such as a cloudlet-hour computed from RAM and CPU usage,
declared as versioned catalog data (P-D-229). The declaration, its grammar, its evaluator and its canonical bytes are the
SDK's (`bss_products_sdk::derived`, P-D-230); this feature adds the storage, the doors, the grants and the audit (P-D-231).
It has no design slice of its own: [DESIGN](../DESIGN.md) §3.1, §3.3 and §3.7 are its design, and
[DECOMPOSITION](../DECOMPOSITION.md) entry 2.5 places it. A usage SKU's pin and the meter-semantics answer to pricing are
later runs of the same plan.

### 1.2 Purpose

Let a catalog author declare a derived usage type once per formula, keep every version immutable, and give a pricing author
the exact meter reference, unit and accrual policy version a usage policy names.

Requirements: `cpt-cf-bss-products-fr-derived-usage-type`.

### 1.3 Actors

`cpt-cf-bss-products-actor-catalog-admin`, `cpt-cf-bss-products-actor-pricing`, `cpt-cf-bss-products-actor-auditor`. The
writes ask `author` on the resource `derived_usage_type`; the reads ask `sku:read` (O-3).

### 1.4 References

- [PRD](../PRD.md): `fr-derived-usage-type` and AC #30.
- [DESIGN](../DESIGN.md): §3.1 (the derived usage declaration and type), §3.3 (the doors), §3.7 (the tables).
- [DECISIONS](../DECISIONS.md): P-D-229, P-D-230, P-D-231.
- The plan: `docs/superpowers/plans/2026-10-01-products-derived-usage-types.md` in the main checkout, rev 3, run 2.

## 2. Actor Flows (CDSL)

### Author declares a derived usage type

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-derived-usage-types-author-declares`

1. [ ] - `p1` - Author posts `{code, name, declaration}`; authenticate, ask `author` on `derived_usage_type` anchored to the caller's tenant, and look up an optional `Idempotency-Key` before anything else is judged - `inst-derived-create-input`
2. [ ] - `p1` - Judge the code and the name, then the declaration (algorithm declaration-judged), then each input (algorithm inputs-resolve) - `inst-derived-create-judge`
3. [ ] - `p1` - In one transaction, insert the type and its version 1 with the stored digest, write the audit row and record the replay answer; a taken code is 409 DERIVED_CODE_TAKEN - `inst-derived-create-commit`
4. [ ] - `p1` - Author posts `{declaration}` to the type's versions; an unknown code is 404 before the catalog is asked; the version is appended by algorithm version-append - `inst-derived-version-input`

### Pricing author reads a version

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-derived-usage-types-pricing-reads`

1. [ ] - `p1` - Ask `sku:read`; the compiled scope and the caller's tenant filter every read - `inst-derived-read-scope`
2. [ ] - `p1` - List the tenant's types by code, 50 to a page and at most 200, with a cursor and each type's latest version; read one type with its versions' headers - `inst-derived-read-list`
3. [ ] - `p1` - Read one version: the declaration, the stored digest, `meter_ref` `{usage_type_id: "products.derived/<code>@<n>", version: "<n>"}`, `canonical_unit` and `accrual_policy_version` `derived-v1:<digest>`; a non-canonical `n`, an unknown code or version, and another tenant's type are 404 - `inst-derived-read-version`

## 3. Processes / Business Logic (CDSL)

### declaration-judged

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-derived-usage-types-declaration-judged`

1. [ ] - `p1` - Parse the string-typed wire declaration into the SDK's types; an unknown granularity, fold, round mode or operator, a decimal that does not parse, or a node whose fields are not its operator's is 400 DERIVED_DECLARATION_INVALID naming the shape's rule - `inst-derived-shape`
2. [ ] - `p1` - Run the SDK's `validate`; its refusal is 400 DERIVED_DECLARATION_INVALID, the detail led by the rule of its variant - `inst-derived-rules`
3. [ ] - `p1` - Store the declaration as the doors serve it, and its digest: the SHA-256 of the SDK's canonical bytes through `aws-lc-rs`, taken once - `inst-derived-digest`

### inputs-resolve

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-derived-usage-types-inputs-resolve`

1. [ ] - `p1` - Ask the `UsageTypeCatalog` port once per input, as the caller, in input order, outside any transaction - `inst-derived-inputs-ask`
2. [ ] - `p1` - A refusal of the caller is 403 USAGE_TYPE_FORBIDDEN, an outage or an unconfigured catalog 503 USAGE_TYPE_UNAVAILABLE, and otherwise every unresolved input one 400 USAGE_TYPE_UNRESOLVED on its ref - `inst-derived-inputs-answer`

### version-append

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-derived-usage-types-version-append`

1. [ ] - `p1` - In the write's transaction, find the type, take its latest version and insert n + 1; a lost race on the number is 409 CONTENDED - `inst-derived-append-number`
2. [ ] - `p1` - Write the audit row `derived_usage_type.version` with the type's id and n in the same transaction; an audit failure rolls the version back - `inst-derived-append-audit`

## 4. States (CDSL)

### Derived usage type states

- [ ] `p1` - **ID**: `cpt-cf-bss-products-state-derived-usage-types`

1. [ ] - `p1` - A type has no lifecycle: create gives it version 1, and no door renames, retires or deletes it (O-1).
2. [ ] - `p1` - A version is immutable from its insert: the storage refuses every update and delete; a new formula is a new version.

## 5. Definitions of Done

These definitions own this feature's 3 DoDs. Design constraints: `cpt-cf-bss-products-constraint-two-backends`.

### Append-only store on both backends

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-derived-usage-type-store`

Verified by the plan's run 2 (the tests P-D-231 lists); implementation markers in
`products/src/infra/storage/migrations/m20261001_000011_derived_usage_type.rs` and
`products/src/infra/storage/repo/derived_usage_type_repo.rs`.

Migration 000011 creates the type and version tables on SQLite and Postgres: the code unique per tenant, a tenant-qualified
foreign key from a version to its type, CHECKs on the code, the version and the digest, and a trigger refusing every update
and delete of a version on both engines. The repository reads and writes under the caller's `AccessScope` and tenant, and
maps the code index and the version key to typed refusals (DESIGN §3.7; P-D-231).

### Declarations judged by the SDK's rules

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-derived-usage-type-rules`

Verified by the plan's run 2 (the tests P-D-231 lists); implementation marker in `products/src/domain/derived.rs`.

A declaration is refused with 400 DERIVED_DECLARATION_INVALID naming its rule, one per SDK variant plus the wire shape's;
the digest is the SHA-256 of the canonical bytes, stored and never recomputed; the inputs resolve through the usage-type
catalog as the caller (403, 503, 400); the SDK's caps are tied to the gear's by const asserts (DESIGN §3.1; P-D-230, P-D-231).

### Five doors with their grants and audit

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-derived-usage-type-doors`

Verified by the plan's run 2 (the tests P-D-231 lists); implementation marker in
`products/src/api/rest/derived_usage_types.rs`.

Two writes under `author` on `derived_usage_type` and three reads under `sku:read`, registered through OperationBuilder with
503 declared on each and a served text naming every code; writes take an optional `Idempotency-Key` and write one audit row
each in their transaction (`subject_kind = derived_usage_type`, the type's id, the version); the list pages as the SKU list
does (DESIGN §3.3; P-D-231).

## 6. Acceptance Criteria

Each criterion below corresponds to exactly one DoD above and cites [PRD §9](../PRD.md#9-acceptance-criteria). The pin of a
usage SKU and the meter-semantics answer are not part of this feature yet.

| DoD | PRD trace | Given / When / Then |
| --- | --- | --- |
| `cpt-cf-bss-products-dod-derived-usage-type-store` | AC #29, #30; `cpt-cf-bss-products-fr-derived-usage-type` | Given a stored version, when anything updates or deletes it on either engine, then the storage refuses; a version naming another tenant's type is refused by its key, and a second type with the tenant's code by its index. |
| `cpt-cf-bss-products-dod-derived-usage-type-rules` | AC #30; `cpt-cf-bss-products-fr-derived-usage-type` | Given a declaration that breaks a rule, when it is created or added as a version, then it is 400 DERIVED_DECLARATION_INVALID naming the rule and nothing is written; given the cloudlet, its stored digest is the SHA-256 of its canonical bytes. |
| `cpt-cf-bss-products-dod-derived-usage-type-doors` | AC #28, #30; `cpt-cf-bss-products-fr-derived-usage-type` | Given an author with `author` on `derived_usage_type`, when the cloudlet is created and a second version added, then both read back with their meter ids and version 1 is unchanged, each write has its audit row, a replayed key answers the first receipt, another tenant reads nothing, and a caller with only `sku:read` reads and cannot write. |
