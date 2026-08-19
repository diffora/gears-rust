<!-- Related: ../../reviews/2026-08-10-eye-review-findings.md, ../../DECISIONS.md -->

# Handoff — bss-pricing wired into vhp-core, deployed to benidorm, e2e at 66/68

**Read this whole file before touching anything.** It restates the working method in full
rather than pointing at it, because the method is what produced the results and a summary of
the results without it is not reproducible.

---

## 1. Where things stand

### Two repositories, and only one of them is committed

**`gears-rust`** — `/Users/alexey/Projects/diffora/gears-rust`, branch `bss/pricing-impl`,
HEAD `d61d1ce88`, **126 commits ahead of `origin/bss/pricing-impl` and unpushed**. Five landed
this session:

| Commit | What |
|---|---|
| `17b0f3478` | merge: caught the branch up with `origin/main` (it was 121 behind on `toolkit` + `gears/system`) |
| `9e89758c4` | fix(coord): the lease migration had one owner in its head and two in the cluster |
| `6ae81d5ec` | fix(pricing): the overlay list served an unbounded array against D-125 |
| `a644a5d6b` | fix(pricing): a price row could be filed under a plan the tenant does not have |
| `d61d1ce88` | fix(pricing): two declared permissions no route asked for — the audit trail was readable by anyone with catalog read |

A backup ref `backup/pricing-impl-premerge-2026-08-10` → `16fc2c5b7` marks the pre-merge state.

**`vhp-core`** — `/Users/alexey/Projects/vhp/vhp-core`, branch `bss/pricing-wiring` (cut from
`origin/main`). **Nothing is committed.** Everything below lives in the working tree, so this
checkout is the only machine that can build it:

- `Cargo.toml` — `bss-pricing` + `bss-pricing-sdk` path-deps beside `bss-ledger`
- `crates/server/Cargo.toml` — `bss-pricing.workspace = true`
- `crates/server/src/registered_modules.rs` — `use bss_pricing as _;`
- `config/server.yaml` — the `bss-pricing` gear block (`pg_main`, `search_path: "public,bss"`,
  absolute `fixtures.registry_path`)
- `config/server-dev.yaml` — the same under sqlite, with a **repo-relative** registry path
- `docker/core-server/Dockerfile` — `COPY` of the corpus `registry.toml` into the runtime stage
- `crates/gears/monitoring/.../dto.rs` — `#[schema(as = MonitoringGroupDto)]`, **someone
  else's code**, see §4
- `external/gears-rust` — submodule **detached** at `d61d1ce88`, pointed there by
  `git fetch /Users/alexey/Projects/diffora/gears-rust bss/pricing-impl` then
  `git checkout --detach <sha>`. `.gitmodules` still names `constructorfabric`; it was left
  alone deliberately (local-only work).
- untracked: `tests/e2e/plans/bss-pricing-{smoke,functional}.yaml`,
  `tests/e2e/tests/bss-pricing/`, `tests/e2e/tests/lib/pricing.py`

`vhp-architecture` shows modified; it was already so and is not ours.

### Deployed

benidorm, helm release `core-server`, image built from the working tree by rsync. Pod
`core-server-86c959f75f-dzsgv`, 1/1 Running. The gear boots, migrates 64 migrations into `bss`,
mounts `/bss-pricing/v1/*`, and loads the fixture gate
(`registry_path=/bss-fixtures-registry.toml`, five open kinds).

**Publish does not work end to end** and says so at boot: `no CatalogVersionRegistryV1
registered; publish will fail closed until the registry gear is wired`. That is a separate
piece of work, not a defect.

### E2E

`tests/e2e/tests/bss-pricing/` — 68 scenarios, **66 passing** against the live cluster. Two
plans: `bss-pricing-smoke.yaml` (the redeploy gate) and `bss-pricing-functional.yaml`.

---

## 2. The two remaining failures, both diagnosed, both mine

### `test_audit_read_is_a_permission_of_its_own` — and the suite-wide gap behind it

`GET /history` answers **200** to an actor granted only `read`, even though `d61d1ce88` moved
that route onto `audit × read`.

The gear is right; the suite cannot see it. `pricing_actor_factory` grants
`target_type = "gts.cf.bss.pricing.*"`, and RBAC's `matches_target_type` is a **pure prefix
match** — so the family wildcard covers `gts.cf.bss.pricing.audit.v1~` as well. The actor
genuinely holds `audit read`.

**The consequence is bigger than one test.** Every actor in this suite holds the whole label
family, so the thirteen green `X does not imply Y` scenarios test **action** separation only.
Nothing here tests **label** separation, and nothing could. Closing this needs a factory that
grants one concrete label rather than the family — and then the `audit`, `config`,
`approval_policy` and `historical_import` splits become testable for the first time.

### `test_config_write_is_not_carried_by_plan_write`

`PUT /config/tax-display-policy` answers 400: *"If-Match is required on this verb: a policy
proposal asserts the policy it was authored against (D-186)"*. The body was corrected to
`{"mode": "fail_closed"}` but carries no tag. Read the policy first, pass its ETag. Same shape
as the threshold policy, which already does this.

---

## 3. The method — use it, it is what produced everything above

**Author expectations from the design set, never from the code under test.** The gear's own
tier has ~2440 green tests and could not find any of the three defects fixed this session,
because its tests were written from the handlers and therefore assert what the handlers do. A
test derived from its subject cannot disagree with it. Every fix below came from a scenario
written out of `DECISIONS.md` / `design/` instead.

**A disagreement is decided by a human, not by editing the expectation until it is green.**
When the suite and the gear disagreed, the question was always *which side is wrong*. D-125
had normative text, so the code was wrong and was changed. `retire`-on-a-draft had none — the
404 expectation was extrapolated by the author, so the scenario was corrected and the
alternative reading recorded in its docstring as an open question. Both docstrings say which
happened and why.

**Red first, and prove it red against the actual defect.** Every gear fix here has a test that
fails on the old code with the message the cluster produced, plus a positive control that
already passed, so the case pins the missing behaviour rather than general breakage.

**Compare sets of failures, never the headline count.** One round went 19 → 20 failures and
looked like standing still; the set diff showed four fixed and five newly broken by a change
of mine. Totals hide that. `comm -23`/`comm -13` over sorted `FAILED` lines is the whole
technique.

**Base + added = printed.** Fast tier went 2437 → 2439 → 2441 as two tests were added each
round. When the arithmetic does not close, a test was dropped.

**A probe proves only what it was armed against.** Twice this session a probe passed for the
wrong reason: a census scan that undercounted, and a `grep -c '^### F-'` that reported 18
findings in a document containing 107 because part II uses a different heading style. Silence
means *the probe missed*, not *the thing is absent*.

**When a symptom reads as a security incident, ask the discriminating question before saying
so.** A neighbour tenant POSTing onto another tenant's plan answered 201 with the foreign id
echoed back. Two probes settled it: the owner's catalog was unchanged (no boundary crossed),
and a real foreign id drew the same status as an invented one (no existence oracle). It was a
referential gap, not a breach. Both probes are now permanent scenarios, because the difference
between "missing validation" and "a neighbour can sweep ids" is invisible in a single response.

---

## 4. Gotchas that cost time here

- **`TaskStop` on a Monitor kills the whole pipeline**, including a `pytest` running inside it.
  Run long jobs detached (`nohup … >> log &`) and let the Monitor only `tail -f` the log.
- **A background command's "exit code 0" notification lies** when the command was piped through
  `tail`. Write `echo "EXIT=$?"` into the log and read that.
- **Renaming a test breaks the run-to-run set diff** — it looks like one fixed and one newly
  broken. Rename deliberately or not at all mid-investigation.
- **Per-test Monitor notifications flood.** Filter to `FAILED|ERROR|^=+ .*(passed|failed)`.
- **The monitoring `GroupDto` alias is someone else's code.** `resource-group` (vendored) and
  `monitoring` (vhp-core's own) both declare `GroupDto`; the toolkit's OpenAPI registry panics
  at boot on the collision. The alias is a workaround. The proper fix is renaming monitoring's
  type to `SourceGroupDto` (they are groups of monitoring *sources*) — **attempted and rolled
  back**: `cargo check -p monitoring` produced 8 errors the 21-reference count did not predict,
  and `pr/monitoring-source-liveness-VHP-2586` is editing that directory now. Start from those
  errors, not from a reference count, and coordinate with the branch owner. The collision
  pre-exists on `main` and is not ours.
- **`.cf-studio` / spec-check**: an untracked review doc under `docs/reviews/` breaks local
  cfs. The findings file below is untracked.

---

## 5. Running things

```bash
# deploy (≈5 min warm; rsyncs the working tree, so uncommitted work ships)
cd /Users/alexey/Projects/vhp/vhp-core
nohup bash .claude/skills/vp-core-redeploy/scripts/core-redeploy.sh >> /tmp/redeploy.log 2>&1 &

# kubeconfig (the e2e cache at .agents/test-e2e-cluster.json names this path)
ssh root@benidorm.jele.io 'cat /etc/rancher/k3s/k3s.yaml' \
  | sed 's#https://127.0.0.1:6443#https://185.231.240.176:6443#' > /tmp/k3s-benidorm-vpn.yaml

# the suite (≈7 min)
cd /Users/alexey/Projects/vhp/vhp-core/tests/e2e/tests
KUBECONFIG=/tmp/k3s-benidorm-vpn.yaml nohup ../.venv/bin/python -m pytest bss-pricing/ -v \
  --e2e-k8s-namespace=virtuozzo >> /tmp/e2e.log 2>&1 &

# the gear's own tier (≈4 min)
cd /Users/alexey/Projects/diffora/gears-rust && cargo test -p cf-gears-bss-pricing
```

The e2e run needs no Keycloak or URL flags — the suite discovers them from the HTTPRoutes.

---

## 6. The findings document — the main body of work waiting

`gears/bss/pricing/docs/reviews/2026-08-10-eye-review-findings.md`, **untracked, 2127 lines,
and being edited by something else**: line 2116 already references `6ae81d5ec`, a commit made
during this session. Check whether it has moved before planning against it.

**It carries 107 findings, not 18.** Part I is `F-1…F-18`, by layer, under `### F-NN` headings.
Part II (from *"What was verified by hand rather than accepted"*, ~line 631) is
`Z6-1…Z13-15` — **89 findings across eight zones** — structured as `**Findings** / **Verdict**
/ **Refutations** / **Not covered**` blocks with no `###` headings. A grep for `^### F-` misses
all of them. **Part II has not been read.** Read it first; the triage below covers Part I only
and its proportions are therefore unreliable.

### Part I, by why a test can or cannot reach it

- **Prose vs code — no test will ever catch these** (F-6, F-8, F-15, F-17, F-18): stale rule
  counts, a doc naming a producer that does not produce, refusal strings that lost their line
  continuations. This is spec-check work.
- **The defect is *in* a test** (F-3, F-7). F-7 is the sharp one and is **high**:
  `assert_eq!(named, 22)` in `projection_tests.rs:1078` passes when a new `PlanSubjectDelta`
  member is named in the destructure and left out of both classification lists, and **fails
  when someone classifies it correctly**. It fires on the fix and is silent on the omission,
  while reporting D-303 as covered. Not yet fixed. Close it by deriving the expectation from
  the struct — every name the destructure produces appears in exactly one list — not by
  bumping the literal.
- **Missing feature on a surface this suite does not cover** (F-1 high, F-2, F-4): `POST
  /bundles/{id}/publish` evaluates no materiality, so D-104's always-material
  `bundleComposition` / `revenueShareChange` triggers are unenforced on the one surface where
  the money being split belongs to third parties. The change sets exist and have no caller.
  **This is the flagship next job**: it is exactly the shape of the three already fixed —
  write the scenario from D-104 (a bundle publish opens an approval unit), watch it fail, fix
  the gear, watch it pass. The approvals machinery in the suite is green and reusable. The
  precedent is in-crate: `priceOverlayMutation` had the same shape and was closed by
  `overlays::overlay_submit_materiality`.
- **Latent** (F-12, F-13): nothing to catch until a new axis or the custom-frequency feature
  arrives.
- **Reachable, unit-level** (F-11 high): `NoticePeriod::earliest_effective` does
  `announced_at + Duration::days(self.days)` with a clamp only from below, so a large
  configured period overflows chrono and **panics** — a 500 where everything around it refuses
  with a named code. The period comes from a tenant policy row, not a request, so e2e cannot
  reach it. `checked_add_signed` plus a refusal, with a unit test.
- **Reachable, message-level** (F-9, F-10, F-14, F-16): duplicate or misleading violation
  entries, an empty rendered list. Catchable by asserting refusal text.

### Suggested order

1. Read Part II (89 findings). Everything below may be reordered by what is there.
2. F-1 → F-2 → F-4 (bundle materiality), full red-to-green through the cluster.
3. F-7 (false green, high) and F-11 (panic, high) in the gear's own tier.
4. The label-separation gap in §2 — it unlocks testing the `audit` / `config` /
   `approval_policy` / `historical_import` splits that were just introduced.
5. The error-code ledger: `tests/bss-pricing/test_error_catalogue.py` extracts 136 wire codes
   from the gear and 8 are claimed. It fails in **both** directions — a code that gains a
   scenario must leave `UNCLAIMED` in the same commit, and one that loses its scenario must
   return. It caught a coverage *regression* this session that the headline count hid.

---

## 7. Open questions for a human, recorded rather than decided

Three scenarios assert a corrected expectation and say in their docstring that the alternative
reading is defensible and unsettled:

- `GET /plans/{foreign}/prices` answers **200 empty** where the point read answers 404. Not a
  disclosure (probe confirms an invented id answers identically) — but should a collection
  under a plan the caller cannot see 404 like its parent?
- `retire` on a never-published plan answers **404**, not `LIFECYCLE_FORBIDDEN`. Retirement
  acts on the published plan, so a draft is arguably not a premature retirement but an absent
  subject.
- `clone` of a draft answers `CLONE_SOURCE_NOT_FOUND`, so the suite has no positive control
  for the clone path and cannot get one until publish works end to end.

Also unresolved: the `GroupDto` collision belongs to the `monitoring` owner and should be
reported upstream rather than carried as a local alias forever.
