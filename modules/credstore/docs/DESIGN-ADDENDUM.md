# Design Addendum — CredStore

Status: descriptive (documents the design decisions of the shipped
implementation).
Scope: the **`credstore` module** and `credstore-sdk`.

This document amends the design at
[`DESIGN.md`](./DESIGN.md)
(referred to below as "the original design", with `§` section references to it).
It is a **design-to-design** comparison: for each area it states what the
original design decided, what the revised design decides instead or in addition,
and why. It does not restate the original; read that first. Where the original is
unchanged (sharing-mode taxonomy, 404 anti-enumeration, plugin-per-vendor
selection, tenant-from-`SecurityContext`), this addendum is silent.

---

## 1. The central design shift: a stateful gateway

The original design is built on a **stateless gateway** (§4.7):

> "The gateway module has no local database. Secrets are persisted in the
> external backend ... The gateway and the VendorA/OS plugins are stateless."

All per-secret metadata (`sharing`, `owner_id`, `owner_tenant_id`) is therefore
stored **in the backend** and returned on every `get` (§4.3 *SecretMetadata*,
§4.4), and secret identity is a **stateless deterministic ExternalID mapping**
`(tenant_id, key, owner_id) → base64url(...)` (§4.4 *ExternalID Mapping*,
principle `…-stateless-mapping`).

The revised design inverts this: the **gateway is stateful and owns a metadata table**;
the backend stores the **value only**. Almost every other delta below follows
from this one decision. The remaining sections are organised around it.

---

## 2. Database schema (new — original design had none)

The original design specifies **no database schema for the gateway** (§4.7); the
only schema in the design lives in the optional `credentials_storage` plugin,
not in the gateway. The revised design adds a gateway-owned table, `credstore_secrets`
(migration `m0001_initial_schema`):

```sql
CREATE TABLE credstore_secrets (
    id         UUID PRIMARY KEY,
    tenant_id  UUID NOT NULL,
    reference  TEXT NOT NULL CHECK (length(reference) BETWEEN 1 AND 255),
    sharing    SMALLINT NOT NULL CHECK (sharing IN (1,2,3)),  -- private/tenant/shared
    owner_id   UUID NOT NULL,
    status     SMALLINT NOT NULL CHECK (status IN (1,2)),     -- provisioning/active
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    version    BIGINT NOT NULL DEFAULT 1
);
-- coexistence of a private and a tenant/shared secret under one reference:
CREATE UNIQUE INDEX uq_credstore_nonprivate ON credstore_secrets (tenant_id, reference)            WHERE sharing <> 1;
CREATE UNIQUE INDEX uq_credstore_private    ON credstore_secrets (tenant_id, reference, owner_id)  WHERE sharing = 1;
-- walk-up resolution and reaper sweep:
CREATE INDEX idx_credstore_lookup       ON credstore_secrets (reference, tenant_id, status);
CREATE INDEX idx_credstore_provisioning ON credstore_secrets (created_at) WHERE status = 1;
```

What this schema realizes that the original left to the backend (or didn't
model at all):

- **The uniqueness rules of §4.1** — `(tenant_id, reference)` unique for
  tenant/shared, `(tenant_id, reference, owner_id)` unique for private — are
  enforced here as **partial unique indexes**, instead of being a backend
  constraint. The two partial indexes also let a private and a tenant/shared
  secret **coexist** under one reference (as §4.1 allows) without either
  shadowing the other in storage.
- **`status` (provisioning/active)** has no counterpart in the original design —
  it exists to make writes a recoverable saga (§5 below).
- **`version`** has no counterpart — versioning was an explicit non-goal in the
  original (§2.2); see §6.
- The table is a **`Scopable` SecureORM entity**, which is what lets
  authorization be enforced in SQL (§3).

Targets PostgreSQL and SQLite (raw per-backend SQL to preserve `CHECK` and
partial-index semantics); MySQL fails fast.

**Why a gateway table at all (vs the original's stateless model):** it removes
the need for the backend to carry a metadata schema and a `sharing`/`owner_id`
column (the original made the "VendorA backend schema update" a launch
prerequisite, §4.4 *Compatibility Note*), it removes the ExternalID-collision
risk the original had to mitigate (§5.2 *ExternalID Encoding Collision*), and it
makes hierarchical resolution and authorization a single transactional query
(§3, §4).

---

## 3. Authorization: PDP scope vs permission strings

**Original design:** authorization is a coarse permission-string check —
`Secrets:Read` / `Secrets:Write` — performed in the gateway (§3.1 principle
`…-authz-gateway`, error table §4.3.1).

**Revised design:** authorization is delegated to the platform PDP via
`PolicyEnforcer`. Each operation evaluates an `AccessScope`, and that scope is
**enforced in SQL** through SecureORM clamps on the `credstore_secrets` table.
Both read and write paths additionally gate on an explicit own-tenant invariant
(`scope_includes_tenant`) and emit a `cross_tenant_denied` metric; out-of-scope
access is fail-closed and returns the canonical 404 (anti-enumeration, §5.1, is
preserved).

The revised design also adds a degraded mode the original does not contemplate:
**config-driven PDP capability advertisement** (`hierarchy.tenant_closure_colocated`).
When the shared tenant-closure is co-located with this table, the PDP emits a
structured `InTenantSubtree` predicate resolved by a closure subquery; when not,
the PDP pre-expands the subtree into a flat `In` list the module enforces with
no local closure.

**Why:** real tenant isolation enforced at the data layer, consistent with the
rest of the platform (RBAC/RG/PDP), rather than a permission-string gate that
cannot express tenant-subtree scope.

---

## 4. Hierarchical resolution: one SQL query vs N×2 backend probes

**Original design (§4.6):** a two-phase walk-up — for each tenant in the
ancestor chain, call the backend `get` once with `Some(owner_id)` (private
probe) and once with `None` (tenant/shared probe), stopping at the first
accessible secret. Cost: up to **N×2 backend round-trips** for an N-deep
hierarchy, because metadata only exists in the backend. Caching the ancestor
chain is left as an open recommendation (§7 Q7: "Yes, 5-minute TTL, LRU").

**Revised design:** because metadata is local (§2), the entire chain resolves in a
**single indexed SQL query** over `credstore_secrets` (`idx_credstore_lookup`):
filter by reference + ancestor-tenant set + active status + sharing-class
visibility, then pick the winner in-process (closest tenant wins;
private beats non-private at the same level). The backend is read **once**, for
the winning row's value only.

The ancestor chain is fetched from the tenant-resolver and cached in-process
with **TTL + LRU eviction** (resolving the original's open Q7 as an
implementation, not a recommendation), with a documented argument for why the
chain is safe to share across security contexts.

**Why:** walk-up latency no longer scales with hierarchy depth on the metadata
side; the backend is touched at most once per resolution.

---

## 5. Tenant-isolation barriers (new — not modelled in the original)

**Original design:** the walk-up is a plain child→parent→root traversal (§4.6).
There is no concept of an isolation boundary between a tenant and its ancestors.

**Revised design:** the ancestor chain is computed with `BarrierMode::Respect`, so a
`shared` secret is **not** inherited across a `self_managed` isolation barrier
(the "upward" axis). The "downward" scope reach is bounded separately by the PDP
/ tenant-closure (and the closure `barrier` column on the structured path).

**Why:** secret inheritance must honour the platform's tenant-isolation
guarantees; an ancestor above a barrier must not leak `shared` secrets into a
self-managed subtree. The original design predates this requirement.

---

## 6. Versioning and optimistic concurrency (new — explicit non-goal in the original)

**Original design:** "Secret versioning / history" is an explicit **non-goal**
(§2.2); compare-and-swap is deferred (§7 Q5).

**Revised design:** each row carries a monotonic `version`; `GET` returns it as a
strong `ETag`. `PUT`/`DELETE` honour `If-Match` (`*` or a quoted version) as an
optimistic-lock precondition, enforced as a `version = ?` filter on the metadata
update/delete. A mismatch surfaces as the canonical `Aborted`/409
(`OPTIMISTIC_LOCK_FAILURE`) — the canonical error model has no 412, so 409 is the
deliberate platform-correct status.

**Why:** safe concurrent updates and lost-update detection — partially
addressing the original's deferred CAS question (§7 Q5) at the metadata layer.

---

## 7. Write semantics: a recoverable saga vs an undefined upsert

**Original design:** `PUT` is an idempotent upsert and `POST` is create-only
(§5.1 *PUT-as-Upsert*). Crucially, a private↔(tenant/shared) change is **not
atomic**: it would be a two-step create-in-new-scope + delete-old, and the design
states there is **"no implicit migration guarantee"** (§4.4 *Sharing Mode
Transitions*). With no gateway state, there is no notion of a half-written
secret to recover.

**Revised design:** because the gateway is stateful, a write is a **compensating
saga**: insert a `provisioning` row → write the value to the backend → mark the
row `active`. Failure handling is explicit:
- backend write fails → roll back the provisioning row, so a failed create does
  not wedge the reference behind the unique index;
- a stuck provisioning row is swept by a periodic **reaper** (configurable
  timeout, indexed by `idx_credstore_provisioning`);
- `POST` is create-only (409 on conflict); `PUT` resolves a lost create-race to
  an update via a bounded retry.

The private↔non-private hazard from the original disappears: private and
tenant/shared secrets **coexist** under one reference (the two partial unique
indexes, §2), so there is no cross-boundary "migrate" to make atomic — such a
transition is simply **rejected as unsupported** rather than performed
non-atomically.

**Why:** writes become crash-safe and self-healing, and the original's
undefined-migration edge case is designed out instead of documented as a hazard.

---

## 8. Confidentiality: hardened beyond the original NFR

The original confidentiality measure is `SecretValue` redaction plus
transport-layer log scrubbing (§3.2, §5.2, NFR `…-confidentiality`). The revised
design keeps that and adds three points the original does not state:

- redacted hand-written `Debug` on the request/response **DTOs** (a derived
  `Debug` would leak plaintext if a future layer logged a DTO);
- **`Cache-Control: no-store`** on the `GET` response, so secret material is
  never cached by intermediaries;
- **no lossy decode** — a non-UTF-8 value is rejected with a typed error rather
  than silently corrupted by `from_utf8_lossy` on the response path.

---

## 9. Observability (new)

The original treats no-logging as the sole confidentiality NFR and defines no
operational metrics. The revised design emits typed OpenTelemetry metrics: walk-up depth,
read outcome (own / inherited / miss), dependency timings (PDP, tenant-resolver,
backend plugin), provisioning rollback/reaped counters, and an inventory gauge
refreshed by the reaper.

---

## 10. Original open questions, resolved by the revised design

| Original (§7) | Revised design |
|---|---|
| Q5 — Compare-and-Swap | optimistic concurrency via `version` + `If-Match` (§6) |
| Q7 — cache tenant hierarchy (recommended 5-min TTL, LRU) | implemented: in-process ancestor-chain cache, TTL + LRU sweep (§4) |
| §5.2 — ExternalID encoding collision risk | eliminated: identity is a DB row, not an encoded backend key (§2) |
| §4.4 — backend schema update is a launch prerequisite | eliminated: backend stores value only; metadata is local (§1–§2) |

---

## 11. Retained from the original design

- Three-tier sharing model: `private` (owner-only), `tenant` (default),
  `shared` (hierarchical), and the §4.1 uniqueness rules (now as partial
  indexes, §2).
- 404 for inaccessible secrets (anti-enumeration), not 403.
- `SecretRef` format `[a-zA-Z0-9_-]+`, plugin-per-vendor GTS selection, tenant
  derived from `SecurityContext`.
- `POST` = create-only (409 on conflict), `PUT` = upsert.
- `GetSecretResponse` metadata shape (`owner_tenant_id`, `sharing`,
  `is_inherited`) — extended with `version`.
