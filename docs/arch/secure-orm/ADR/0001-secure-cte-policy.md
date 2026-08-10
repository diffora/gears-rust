---
status: proposed
date: 2026-07-24
decision-makers: Constructor Fabric steering committee
---

# ADR-0001: Safe CTE Support in the Secure ORM

**ID**: `cpt-cf-adr-secure-cte-policy`

## Context and Problem Statement

The Secure ORM (`libs/toolkit-db/src/secure/`) enforces multi-tenant isolation
through a single load-bearing invariant: **every access to a table passes through a
scope condition**. This is guaranteed mechanically, not by convention:

- A typestate transition `Unscoped → Scoped` — a query physically cannot be executed
  until `.scope_with()` is called
  ([select.rs:151-188](../../../../libs/toolkit-db/src/secure/select.rs#L151-L188)).
  The `Scoped` marker carries an `Arc<AccessScope>` so related-entity queries inherit
  the same scope
  ([select.rs:22-25](../../../../libs/toolkit-db/src/secure/select.rs#L22-L25)).
- `build_scope_condition::<E>()` attaches a `WHERE` for the concrete entity `E` via
  `E::resolve_property()`
  ([cond.rs:54-83](../../../../libs/toolkit-db/src/secure/cond.rs#L54-L83)).

A Common Table Expression (`WITH x AS (SELECT ... FROM sensitive_table) ...`) breaks
this invariant. The body of a CTE is an **independent `SELECT` over arbitrary tables**
to which the outer query's scope `WHERE` does **not** apply. If a gear could build a
CTE freely, the scope filter would land on the outer entity while the tables read
inside `WITH` stay unfiltered — a direct tenant-isolation hole.

The naive workaround — "expose `into_inner()` and assemble the CTE by hand on
sea_query"
([select.rs:415-418](../../../../libs/toolkit-db/src/secure/select.rs#L415-L418)) —
also violates the platform guardrails. The rule **"No plain SQL in
handlers/services/repos. Raw SQL is allowed only in migration infrastructure"** is
explicit and repeated in
[11_database_patterns.md:9](../../../toolkit_unified_system/11_database_patterns.md#L9)
(and again at lines 161 and 167). Raw SQL escapes both the typestate guarantee and
this rule.

There is real, tracked demand for capabilities that today would reach for a CTE:

- `account-management/.../tr_plugin/queries.rs:26-39` builds a parent map in memory and
  walks it pre-order on the client, explicitly deferring the SQL recursive CTE "as a
  follow-up once `toolkit-db` exposes a safe raw-SQL hook".
- `account-management/.../repo_impl/conversion.rs:1221` recomputes barriers per row and
  notes "Replace with self-join on `tenant_closure` once the recursive-CTE work lands".

**Scope of this ADR.** This policy governs **standard user gears** — the
handlers / services / repos that build queries against their own domain tables. It
does **not** govern `toolkit-db` itself. The one existing CTE in the crate is the
outbox writer
([outbox/store.rs:264-273](../../../../libs/toolkit-db/src/outbox/store.rs#L264-L273)):
that is raw dialect-specific SQL living **inside the system library that implements the
Secure ORM**, where sea_query and hand-written SQL are the implementation substrate.
That is correct and out of scope here — it is not user-gear code and is not a precedent
for user-gear code.

Today there is **no CTE support in the Secure ORM's user-facing API** and no policy
governing it for user gears. This ADR settles the policy and the API shape **before**
ad-hoc solutions accrete in gears.

## Decision Drivers

- **Preserve the typestate invariant** — a CTE must be as impossible to construct
  unscoped as an `Unscoped` select is impossible to execute.
- **No raw SQL in user-gear code** — honor
  [11_database_patterns.md:9](../../../toolkit_unified_system/11_database_patterns.md#L9);
  in gears everything goes through the Secure ORM / sea_query builder (the `toolkit-db`
  system library that implements it is a separate layer).
- **Reuse what exists** — `build_scope_condition` already emits nested subqueries via
  `sea_query::Query::select()`
  ([cond.rs:104-185](../../../../libs/toolkit-db/src/secure/cond.rs#L104-L185)); the
  "scope as a sea_query expression" mechanic is proven.
- **Do not regress hierarchy handling** — hierarchy traversal already works via
  materialized closure tables (`tenant_closure`, `resource_group_closure`); do not
  reintroduce recursion the isolation model cannot cover.
- **Reviewability** — any escape from the safe path must be visible at the API surface
  so a reviewer sees that isolation was considered.

## Considered Options

- **Option A** — `SecureCte`, constructible only from `SecureSelect<E, Scoped>`; scope
  is embedded inside each CTE body.
- **Option B** — Controlled escape-hatch: a CTE over a non-`Scopable` source with a
  mandatory `.scope_via_exists::<J>()` join predicate to a scoped entity.
- **Option C** — Raw `WITH` from a string in gear code.
- **Option D** — `WITH RECURSIVE` for hierarchy traversal.

## Decision Outcome

Chosen option: **Option A**, because it is the only option that keeps the
tenant-isolation invariant a **compile-time** guarantee while using the sea_query
builder exclusively. The principle: **do not apply scope outside the CTE — embed scope
inside the body of every CTE.** Then any table a CTE touches is already filtered.

### API shape

```rust
// 1. A CTE body can be built ONLY from an already-scoped query.
//    scope is already baked into the body via build_scope_condition.
//    The name is `&'static str` (see "CTE name"); injection safety comes from
//    sea_query identifier quoting, not from the lifetime.
impl<E: ScopableEntity + EntityTrait> SecureSelect<E, Scoped> {
    /// Turn a scoped query into a named CTE. Retains the query's `Arc<AccessScope>`
    /// so `with_ctes` can enforce that every CTE shares the outer query's scope.
    pub fn into_cte(self, name: &'static str) -> SecureCte { /* ... */ }
}

// 2. The outer query accepts CTEs and returns a DISTINCT type — NOT `Self`.
//    `SecureSelect<E, Scoped>` is backed by `sea_orm::Select<E>`, whose
//    `.all()/.one()/.count()` never see a `WithClause` (see "Feasibility
//    constraint"). Returning `Self` would let the `WITH` silently vanish at
//    execution. `SecureCteSelect` instead wraps a `sea_query::SelectStatement`
//    + `WithClause` and executes through `find_by_statement` / `FromQueryResult`.
impl<E: ScopableEntity + EntityTrait> SecureSelect<E, Scoped> {
    pub fn with_ctes(
        self,
        ctes: impl IntoIterator<Item = SecureCte>,
    ) -> SecureCteSelect<E, Scoped> { /* ... */ }
}

// 3. The CTE-aware result type: its own execution path, separate from `Select<E>`.
impl<E: EntityTrait> SecureCteSelect<E, Scoped> {
    pub async fn all(self, runner: &impl DBRunner) -> Result<Vec<E::Model>, ScopeError> { /* find_by_statement */ }
    pub async fn one(self, runner: &impl DBRunner) -> Result<Option<E::Model>, ScopeError> { /* find_by_statement */ }
}
```

The invariant this yields:

- A `SecureCte` **cannot** be constructed from an `Unscoped` select → the compiler
  forbids unwrapped table access inside `WITH`.
- The outer query is itself `Scoped` on its own root entity.
- The `Scopable` requirement is not a separate check: `SecureSelect<E, Scoped>` is only
  reachable through `scope_with`/`scope_with_arc`, which are bounded `E: ScopableEntity`
  ([select.rs:151-155](../../../../libs/toolkit-db/src/secure/select.rs#L151-L155)). So
  `into_cte`/`with_ctes` carry that bound (shown above) and a non-`Scopable` entity can
  never reach a CTE.
- `with_ctes` returns `SecureCteSelect`, a type that does **not** reuse `Select<E>`'s
  execution path, so the API shape and the "Feasibility constraint" section below agree
  — there is no way to build a CTE query that then executes as a plain `Select<E>` and
  drops the `WITH`.
- No raw SQL — everything flows through the sea_query builder, exactly as the scope
  subqueries in [cond.rs](../../../../libs/toolkit-db/src/secure/cond.rs) already do.

### Referencing a CTE

`with_ctes` only *attaches* the CTE definitions; the outer query must still reference
each one by name to use it. Level A's contract: the outer query reads a CTE as a named
source (`FROM`/`JOIN` on the CTE's `&'static str` name), and the predicate tying it to
the outer entity is the caller's responsibility — this is the correctness risk under
Risks, not an isolation one. The precise typed reference helper is an implementation
detail, out of scope for this ADR.

### CTE name

`into_cte` takes `&'static str`, not `&str` — an ergonomic nudge toward string literals,
not a security control (a `&'static str` can still be produced at runtime by leaking).
Injection safety does not depend on it: the name goes through sea_query's `Alias`/`Iden`,
which renders it as a quoted, escaped identifier. In the pinned **sea_query 0.32.7** the
quote char is `"` for Postgres and SQLite and `` ` `` for MySQL, and an embedded quote
char is escaped by doubling it (`types.rs:31-39`). So a hostile name becomes an inert
quoted identifier, never executable SQL — confirm with the hostile-name test (see
Confirmation). A gear that needs a truly dynamic name is out of
scope for Level A (escape-hatch, Level B); validate against an allowlist there.

### Same-scope constraint

The outer query and **every** supplied `SecureCte` must carry the **same**
`AccessScope`. Combining CTEs built from different scopes (e.g. two tenants) in one query
is disallowed: each CTE body is still individually safe (its own scope is embedded), but
a single query mixing scopes is semantically incoherent and exactly the kind of ad-hoc
pattern this ADR exists to prevent. Enforcement, in order of preference:

1. **Structural (preferred).** Seed each CTE from the outer query's own
   `Arc<AccessScope>` so a differently-scoped CTE is *unrepresentable* — no check needed,
   matching the compile-time ethos of the rest of the Secure ORM.
2. **Hard runtime check (fallback).** If the structural design proves too invasive for
   the first cut, `with_ctes` compares the outer `AccessScope` against each CTE's by
   value (`AccessScope: PartialEq`,
   [access_scope.rs](../../../../libs/toolkit-security/src/access_scope.rs)) and returns
   `Err` on mismatch — **in all build profiles**. Note: `debug_assert!` is **not**
   acceptable here; it is compiled out in release builds and would leave a
   tenant-isolation invariant unchecked in production.
3. **dylint (complementary, not a substitute).** A custom lint can enforce the *syntactic*
   Level-C prohibitions (no raw `WITH` string, no `into_inner`/`into_query` in
   handlers/services/repos), but it **cannot** verify that two runtime `AccessScope`
   *values* are equal — that is a value-equality/data-flow problem it cannot decide.
   Do not rely on dylint for the same-scope guarantee.

### Feasibility constraint (must be honored by the implementation)

`SecureSelect.inner` is a **`sea_orm::Select<E>`**, not a `sea_query::SelectStatement`
([select.rs:60-65](../../../../libs/toolkit-db/src/secure/select.rs#L60-L65)).
Execution goes through `self.inner.all()/one()/count()`
([select.rs:200-232](../../../../libs/toolkit-db/src/secure/select.rs#L200-L232)), and
the only public unwrap is `into_inner() -> Select<E>`
([select.rs:415-418](../../../../libs/toolkit-db/src/secure/select.rs#L415-L418)) —
there is no `into_query()`. Critically, **`sea_orm::Select<E>` has no `.with()`
method**; `WithClause`/`CommonTableExpression` live on `sea_query`. Therefore
`into_cte`/`with_ctes` cannot be a drop-in over `inner`:

- `into_cte` must call `QueryTrait::into_query()` on the `Select<E>` to obtain a
  `sea_query::SelectStatement` (scope `WHERE` already embedded) and wrap it in a
  `sea_query::CommonTableExpression`.
- `with_ctes` must build the outer query as a `sea_query` statement with
  `.with(WithClause)` and execute it through a **separate path** —
  `E::find_by_statement(backend.build(&stmt))` / `FromQueryResult` — **not** through
  `Select<E>::all()`.

This is a design requirement, not an optional detail: an implementer who assumes
`inner.with(...)` exists will hit a type wall.

### Levels of strictness

- **Level A (safe, this decision)** — `SecureCte` only from scoped selects over
  `Scopable` entities. Covers the great majority of real needs (aggregations, dedup,
  window functions over an intermediate scoped set).
- **Level B (future, controlled escape-hatch)** — CTE over a non-`Scopable` source but
  with a **mandatory** `.scope_via_exists::<J>()` predicate to a scoped entity.
  Out of scope here; recorded so it is not reinvented ad hoc.
- **Level C (forbidden in user gears)** — raw `WITH` from a string. Not allowed in
  standard user-gear code (handlers/services/repos). Migrations and the
  system libraries that implement the Secure ORM (`toolkit-db` internals, e.g. the
  outbox writer) are a different layer and are not governed by this policy.
  This prohibition is *syntactic* and can therefore be enforced mechanically rather
  than by reviewer vigilance: a custom [dylint](https://github.com/trailofbits/dylint)
  lint can flag raw `WITH` strings and any `into_inner()`/`into_query()` unwrap reaching
  a raw-SQL sink in gear crates, and fail CI. (A lint can enforce these syntactic rules;
  it cannot verify the runtime same-scope invariant — see "Same-scope constraint".)

### Consequences

**Positive:**

- Tenant isolation stays a compile-time guarantee; a CTE is as impossible to build
  unscoped as an unscoped select is to execute.
- sea_query-only; no new raw-SQL surface in gear code — the guardrail holds.
- Reuses the proven `build_scope_condition` subquery mechanic.

**Negative:**

- Requires a CTE-aware execution path that bypasses `Select<E>` (via
  `find_by_statement`/`FromQueryResult`), diverging from the existing `.all()` path.
- The CTE body is scoped, but the outer query cannot post-filter the CTE's contents —
  the scope must be correct at body-build time.

**Risks:**

- The correlation between a CTE and the outer query (e.g. joining the CTE to the outer
  entity on the right tenant key) is **not** compiler-verified. This is a **correctness**
  risk, not an isolation risk: because every `SecureCte` embeds its own scope in its own
  body via `build_scope_condition`, a wrong join predicate can only under- or over-select
  *rows* (missing/duplicate results) — it **cannot** leak another tenant's data, since the
  CTE body never contained that data to begin with. The isolation boundary is
  per-CTE-body and independent of outer-join correctness. Mitigate the correctness risk
  with tests and reviewer guidance. (If a future change ever lets CTEs from different
  scopes be combined in one query, that would be a separate, real isolation risk — see the
  "Same-scope constraint" above, which forbids it.)
- The CTE name cannot inject SQL regardless of contents: sea_query renders it as a
  quoted, escaped identifier (`Alias`/`Iden`). `&'static str` is only an ergonomic guard
  toward literals (see "CTE name"), not the injection defense.

### Confirmation

- Tests modeled on the existing scope-condition tests
  ([cond.rs:191+](../../../../libs/toolkit-db/src/secure/cond.rs#L191)) that assert the
  scope condition is present in the body of **every** CTE (assert against the built
  SQL, per backend).
- Add an execution-path test on `SecureCteSelect::all`/`one`: capture the emitted
  statement and assert it contains the `WITH` clause, so a regression that falls back to
  the plain `Select<E>` path (silently dropping the CTE) is caught — not just that the
  scope predicate is present inside the body.
- Add a hostile-name test for `into_cte`: pass a name containing the backend quote char
  and SQL metacharacters (e.g. `foo"; DROP TABLE x --`) and assert the built statement
  renders it as a single quoted, escaped identifier (doubled quote char), not as
  executable SQL — per backend.
- Codify the prohibition on raw and recursive CTEs in gear code in
  [11_database_patterns.md](../../../toolkit_unified_system/11_database_patterns.md),
  alongside the existing "No plain SQL" rule.
- Back the Level-C prohibition with a custom `dylint` lint in CI (flag raw `WITH`
  strings and `into_inner()`/`into_query()` reaching a raw-SQL sink in gear crates), so
  the guardrail is machine-checked rather than left to review. The same-scope invariant
  is *not* covered by the lint and must be enforced structurally / at runtime instead.

## Recursive CTE (`WITH RECURSIVE`)

**Rejected for hierarchy traversal.** A recursive member references the CTE itself, and
scope cannot be embedded into the recursive step statically — a direct conflict with
the guardrails. The platform already made the opposite architectural choice: it
**materializes transitive closure on write** rather than computing it on read.

- `tenant_closure` (`ancestor_id, descendant_id, barrier, descendant_status`) is
  maintained incrementally on tree changes
  ([tenant/closure.rs](../../../../gears/system/account-management/account-management/src/domain/tenant/closure.rs)),
  with self-row / barrier invariants enforced by check constraints
  (`ck_tenant_closure_self_row_barrier`) and covered by
  `idx_tenant_closure_ancestor_barrier_status`.
- `resource_group_closure` follows the same incremental pattern.
- `InTenantSubtree` / `InGroupSubtree` compile to a **flat** subquery over the closure
  table (`col IN (SELECT descendant_id FROM ... WHERE ancestor_id = ?)`), not recursion
  ([cond.rs:121-185](../../../../libs/toolkit-db/src/secure/cond.rs#L121-L185)).

The driver for this ADR is a **new domain hierarchy not yet in the closure model.**
The recommendation is therefore **not** to introduce `WITH RECURSIVE`, but to extend
the closure model:

1. Add a new closure table for the new hierarchy, maintained incrementally on write
   (self-row + one strict-ancestor hop per edge), with the same self-row / status /
   barrier-style invariants and a covering index.
2. Add a new `ScopeFilter` variant and a branch in `build_scope_condition`, modeled on
   `InTenantSubtree` / `InGroupSubtree`.

Then traversal of the new hierarchy becomes part of the scope condition and inherits
tenant isolation "for free", exactly as the existing hierarchies do.

A recursive CTE would be justified only for an **ad-hoc hierarchy not worth
materializing** (rare traversal, frequently changing tree). Even then it must be a
future Level B/C feature with an explicit constraint: the recursive seed is a
scoped select, and the recursive member is physically confined to the same tenant
column in its join predicate.

## Pros and Cons of the Options

### Option A: `SecureCte` from `SecureSelect<E, Scoped>`

- Good, because scope is embedded in the CTE body — isolation is a compile-time
  guarantee.
- Good, because it is sea_query-only; no raw SQL enters gear code.
- Good, because it reuses `build_scope_condition` and the existing subquery mechanic.
- Bad, because it needs a new execution path (`find_by_statement`/`FromQueryResult`)
  distinct from `Select<E>::all()`.
- Bad, because CTE↔outer correlation is not compiler-verified (test/review burden).

### Option B: Escape-hatch with mandatory `scope_via_exists`

- Good, because it covers CTEs over non-`Scopable` sources while keeping an explicit,
  reviewable isolation predicate.
- Bad, because correctness depends on the developer choosing the right join entity;
  weaker than Level A's structural guarantee.
- Deferred: recorded as a future level, not implemented now.

### Option C: Raw `WITH` from a string

- Good, because maximally flexible.
- Bad, because it discards the typestate guarantee entirely and violates the "no plain
  SQL outside migrations" rule
  ([11_database_patterns.md:9](../../../toolkit_unified_system/11_database_patterns.md#L9)).
- Rejected for user-gear code. (The outbox writer's raw CTE is unaffected: it is a
  `toolkit-db` internal — a system library, not user-gear code.)

### Option D: `WITH RECURSIVE` for hierarchies

- Good, because it materializes nothing (no closure table to maintain).
- Bad, because scope cannot be embedded into the recursive member statically → tenant
  isolation cannot be guaranteed.
- Bad, because barriers/status would have to be threaded into the recursion by hand.
- Bad, because it duplicates a problem already solved by closure tables.
- Rejected; extend the closure model instead.

## More Information

- Secure ORM select builder & execution:
  [libs/toolkit-db/src/secure/select.rs](../../../../libs/toolkit-db/src/secure/select.rs)
- Scope condition builder & closure subqueries:
  [libs/toolkit-db/src/secure/cond.rs](../../../../libs/toolkit-db/src/secure/cond.rs)
- Closure-table maintenance:
  [tenant/closure.rs](../../../../gears/system/account-management/account-management/src/domain/tenant/closure.rs)
- Raw-SQL policy:
  [11_database_patterns.md](../../../toolkit_unified_system/11_database_patterns.md)
- Closure vs recursive CTE rationale:
  [docs/arch/authorization/DESIGN.md](../../authorization/DESIGN.md)
- System-library CTE (out of scope — `toolkit-db` internal, not user-gear code):
  [outbox/store.rs](../../../../libs/toolkit-db/src/outbox/store.rs)
- ADR template & checklist:
  [docs/checklists/ADR.md](../../../checklists/ADR.md)
- sea_query `WithClause` / `CommonTableExpression`, sea_orm `find_by_statement` /
  `FromQueryResult`.
