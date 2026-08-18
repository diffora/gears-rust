<!-- Related: ../DESIGN.md, ../DECISIONS.md, ../design/ | Owners: BSS Product Catalog team -->

# Pricing — implementation handoff (code-readiness state, 2026-08-03)

**What this is.** The branch-durable half of a session handoff: the state of the
implementation against the design set, what is owed and to whom, and the process and review
rules the program has paid for by getting them wrong. It carries **no new decisions** — every
normative claim below cites a `D-NN` already in [`DECISIONS.md`](../DECISIONS.md), and this
document is deliberately **not** linked from the register narrative in
[`DESIGN.md`](../DESIGN.md), which lists review waves because each produced decisions.

> **Phase 3 landed on 2026-08-04 and moved several statements below from true to false.** Rather
> than rewrite the document into a state that hides what changed, each superseded claim is struck
> where it stands and answered beside it. §1 and §4 carry most of them; §5, §6 and §7 are
> unchanged and were the point of writing this down. The one-line summary: **publish has an
> entrance and takes two people**, the SQLite mirror is no longer the only evidence, and the
> phase's own reviews found more defects in it than its tests did.

**What is deliberately not here.** Session-scoped state — commit and test counts, the open PR,
the roadmap board address, the current slice queue position — and the `spec-check` N1
measurement analysis, which belongs to that skill's own notes rather than to this gear. Those
live in the local (untracked) session notes.

---

## 1. Where the implementation stands

**Phase 2 — the publication path — is complete, and it has no entrance.** Eight groups:

| | |
|---|---|
| G1 | Slice-3 row-shape rules + the supersession unit guard (D-82) |
| G2 | `PublishValidator` + the joint-fixture gate |
| G3 | the draft-authoring plane: revisions, `RowVersion`/ETag, plan + price repositories, idempotency gate |
| G4 | the Slice-2 plan shape: 20 rules, three revision-scoped child tables, D-83 paid |
| G5 | the publish commit: pipeline re-run inside the transaction, lifecycle flips, outbox, `CatalogVersion`, segmented audit chain |
| G6 | the read side: projection, warm completion, pin frontier, degraded path |
| G7 | the REST surface: eight authoring routes, plus e2e |
| G8 | close-the-gap: paid what six docs waves had recorded as owed |

**What Phase 2 ends without** — the most useful thing for the next implementer to know, and it
holds **by rule rather than by omission**:

- ~~**Nothing can publish.**~~ **Superseded 2026-08-04 by phase 3.** The `pricing_approval` table
  exists (`m20260802_000015`), `POST /bss-pricing/v1/plans/{planId}/publish` is mounted, and a
  plan is published by **two distinct principals** through HTTP: submit opens a pinned unit, an
  independent reviewer approves, the second publish takes the commit arm. `AutoPublishable`
  remains unreachable *by rule*, exactly as stated — no threshold-policy surface exists, and
  configuring one is itself a material act (D-10). So **every** publish takes two people.
  Demonstrated end to end against a running gear behind the real gateway and PDP
  (`testing/e2e/gears/bss/pricing/test_pricing_seams.py`).

  **One inch is still missing, and it is not this gear's.** No `CatalogVersionRegistryV1`
  implementation exists anywhere in this repository; the module boots with
  `UnconfiguredCatalogVersionRegistryV1`, so the commit's version request answers 503 and rolls
  back. A locally invented version would make this gear a second incrementer, which
  `pricing-sdk/src/catalog_version_registry.rs` refuses outright. The e2e therefore asserts the
  commit arm was **reached** — by four traces including a re-asked gate that proves the 503 is
  the registry's and not the PEP's — rather than asserting a receipt. When the registry gear
  lands, one assertion changes and nothing else in the module does.
- **Nothing can be resolved.** A consumer pins the frontier and resolves nothing at it: no
  resolution route, no resolve method, `PricingCatalogClientV1` has no impl.
- **Zero of six sellability predicates are evaluated.** Three have their *inputs* frozen; there
  is no gate function and no `SellabilitySurface` type. Slice 7's window facts do not exist.
- ~~**35 validation rules … not one is reachable over HTTP.** Nine routes are mounted of 45.~~
  **Superseded.** **Fifteen** routes are mounted, the publish pipeline is reachable, and the
  route census (`tests/module_test.rs`) pins the set by name. Nine of twelve slices still have no
  code.
- ~~Everything is proved against `sqlite::memory:` … There is no testcontainers suite; every
  Postgres trigger, CHECK and partial index is verified by statement-by-statement comparison
  against the executed SQLite mirror, **never by execution**.~~
  **Superseded, and this was the phase's largest single change.** A testcontainers suite exists
  and the chain now runs on the backend it targets. **172 Postgres tests** prove **68 CHECK
  constraints, 10 PL/pgSQL trigger functions with 10 triggers, and 8 partial indexes** by
  *executing the statement each must refuse* and asserting the error names that object — each one
  additionally proved by removal, one object at a time. Two design claims SQLite could neither
  confirm nor refute now hold by execution: **D-159** (the loser of a same-segment race takes
  `CONCURRENT_MUTATION`/409 with its whole transaction rolled back, proved by a witness segment
  holding zero rows) and **D-135** (two aggregates of one tenant commit without contending;
  collapsing the key to the pre-segmentation shape makes that test time out).

  **The mechanism behind the Phase-2 finding is now demonstrated rather than inferred.**
  `tests/sqlite_price_checks.rs` names all twenty `pricing_price` CHECKs and executes only the
  SQLite mirror — a separate `const` array — so it is *structurally incapable* of seeing a
  Postgres-branch guard stop refusing. That is why fourteen of them could each be replaced with
  `CHECK (1 = 1)` with the suite green.

  What has **not** changed: `sqlite::memory:` still serializes writers, so the mirror suites
  remain evidence about shape and connectedness and not about concurrency.

---

## 2. Decisions waiting on the owner

**a. The projector's pending-scan budget and its order.** The order is ascending tenant id and
does **not** rotate, so past the bound the tail is never swept — not swept late. Either ratify
the 250 bound and accept the non-rotating order (lag is loud: `pin_eligibility_overdue` fires),
or rotate, which needs a cursor no §3.7 table carries. The wave recommends the first now and the
second as a named Future gate. The row is in `DECISIONS.md` §F.1.

**b. The overlay stack's sort direction.** `precedence → class order → overlay id` is stated
without ascending or descending. D-138 made it consequential: a `fixed` line *replaces* the
running amount and discards every layer beneath it, so the direction decides which line lands
last and therefore what price a consumer gets. Either declare it in `inst-plv-class-tiebreak`
or route it to Rating/Tariffs as a countersigned seam item. Row in §F.1.

---

## 3. Cross-gear debts, and one deadlock

Owed **out** of pricing — verified 2026-08-03 as genuinely uncited on the counterparty's own
side, not merely unacknowledged here:

- **Rating**: D-126 (cohort bootstrap), D-138 (`fixed` semantics), D-162 (the `ep-1` quantum
  clause on the already-owed D-126 adoption).
- **Subscriptions**: D-131 (lane response shape), D-169 (the warning-copy countersign),
  SUB-P7/P8, D-93/D-94.
- **The registry gear** (`gears/bss/products`, vendored): D-163's batch atomicity.
- **Billing**: D-48's countersign, still pending at that gear's own PRD.

**The deadlock.** The `trailing_tier` joint fixture has been owed with Rating since **D-60**.
Rating's SEAMS M12 carries it open *while asserting that pricing carries the variant*; the
design set here does declare the variant, but `bss_fixtures::Variant` has no member and
`required_variants` never asks for one, so no registry row exists. **Both sides have read the
other as done, and the block can fire on neither.** It cannot be closed inside one gear — the
fixture registry row is joint, so closing it requires a change on both sides in one change set.

---

## 4. Code debts still open

- ~~**The latent freeze.**~~ **PAID 2026-08-04, and the obligation as this document stated it was
  wrong.** §8.1 below told the next phase to *remove* the freeze before mounting publish; D-177's
  own clauses (2) and (3) forbid exactly that — the members stay modelled and rostered, and the
  authoring refusal may be deleted only in the change that lands Slice 10's ten rules and the
  allowance compile. What was actually owed was a **new fail-closed gate on publish itself**,
  because the authoring refusal is safe only while every authoring path exists and refuses, and
  the bulk-import arm it binds is still unbuilt. That gate is **D-179**
  (`PRIMITIVE_RULES_UNBUILT`, Foundation §3.3), registered in the Foundation publish rule set and
  proved by removal. The refusal is untouched and its removal keeps D-177(3)'s order.
- Ten Slice-10 refusals unbuilt. `ALLOWANCE_WITH_RESERVATION` is *unbuildable* — its columns do
  not exist. `FIXTURE_MISSING` needs the joint fixture in §3.
- `ADDON_INCOMPATIBLE`'s cycle arm, and six codes in S2, need a Product & SKU registry read
  model whose client is not in this repository. `ports.rs` holds only the `CatalogVersion`
  registry.
- `correlation_id` is `None` on every authoring audit record (D-178 names the producer; the
  plumbing is owed).
- `open_revision` takes no `expected`, so the `PATCH` successor arm is a comparison, not a swap.
  The group that gives it one should also fuse the handler's two transactions — divergence 16
  and D-176 carry the sequencing, and fusing *alone* would relocate three reads and a
  `CanonicalError` into an infra transaction while leaving the read-to-insert window where it is.
- `OPEN_DRAFT_REVISION_EXISTS` is reachable only through a TOCTOU race.
- The publish-path `PLAN_ABANDONED_NO_SUCCESSOR` arm has no surface (D-172's owed-back clause
  claimed otherwise; corrected in the D-175 wave).
- No test walks an audit segment containing **two** publish records link by link — a combination
  gap, not a property gap.

---

## 5. Process rules that are not optional

**One agent per GROUP, not per task.** The dispatch unit is the group; one implementer carries
all its tasks and keeps context across them. Ten agents for a ten-task group is ten paid entries
into context and ten re-readings of the design set. (This was gotten wrong in G4 and it cost
real time.)

**The controller reviews at group end, in the main session** — and one *independent* review per
group, on a fresh agent, is what earns its keep. What independent review found across Phase 2: a
Critical in G5 (the commit validated one row set and published another), a Critical in G6 (the
frontier advanced over an unresolved publish), an Important lost-update in G7 (the ETag did not
name its revision), the unstorable `new_subscriptions_only` class in G4, and three holes in the
audit trail G8 had just built. **None of these was found by tests.**

**The review method, to be given to every reviewer verbatim:** read the specification first and
write down what the code is *obliged* to do, **then** open the diff. A test written by the
rule's author encodes the author's reading rather than checking it.

**Docs waves run ONE ritual per group**, at group exit — not per edit. The ritual is
`spec-check`, the skill's pytest, a per-id mover diff against a detached worktree, and an oracle
re-capture. It costs the same for one line as for three hundred.

**Prove every guard by removing it and watching exactly one test fail.** Report which test.

**Before filing anything as "owed to a Postgres suite" or "unprovable here", check whether it is
owed to an argument you have not finished.** G5 filed its isolation claim that way while it was
answerable by reading two lines of one file together — and the claim was false, which is how
that Critical stayed hidden for a whole group.

**Never mint a wire code the design set does not name.** Report the gap instead. Where a code
*is* declared in the register, implementing it is not minting. Count claims of the form "~N
codeless gaps" are unsourced unless they cite ids.

**Divergence is the product.** Where code and design set disagree, report it; never reconcile by
quietly editing the document from a code group. Two register entries were caught overstating
their own discharge precisely because that rule held.

---

## 6. The recurring defect class — seven instances

Every one of these: **the test passed for a reason other than the property it named**, because a
fixture put the world in a state where an earlier guard answered first.

1. **G4** — fourteen `pricing_price` CHECKs that a test file *asserted* were covered could be
   replaced with `CHECK (1 = 1)` with the suite green. A repository that writes only valid values
   catches a constraint that got *narrower*, never one that stopped refusing.
2. **G5** — three atomicity tests refused before the first write, so their "holds none of the
   artifacts" assertions had nothing to roll back.
3. **G6** — the test helper committed every publish at one literal instant, so the request-order
   guard matched everything. Parameterising the instant turned three tests red before a source
   line moved.
4. **G7** — the ETag test staged two revisions at *different* versions, so the plain
   compare-and-swap answered 409 whether or not the revision was bound. The defect needs the
   versions to **coincide**.
5. **G7 again** — a guard could be deleted at three call sites with the whole suite green.
6. **G8** — blanking `before_state`/`after_state` in either of two audit writers left the suite
   fully green.
7. **G8's third writer**, the subtlest — the chain tests rebuild each preimage **from the stored
   columns**, so a test deriving its expectation from the data it checks cannot see that data
   replaced.

**The standing lesson**: a test must first assert that the world is the one in which the property
is *observable*, and **connectedness and content need different tests** — the second never
substitutes for the first.

Four further standing review heuristics: name the sibling surface for every decision; name the
publish unit *and* the reading surface for every consumer-visible fact; open the consuming gear's
docs for every snapshot-frozen field; ask what a mechanism's **second** run reads.

---

## 7. Gotchas

- `cargo-hack` and `cargo-nextest` are **not installed**: `make clippy` and `make test` die on
  the tool check. Use `cargo test -p bss-pricing --no-fail-fast` and
  `cargo clippy --workspace --all-targets --all-features -- -D warnings -D clippy::perf`.
- **Never** `make shear` / `cargo shear` in this repository.
- `make dylint` **works** and catches what clippy does not (DE0301 domain purity, DE0309
  `#[domain_model]`, DE1302 `.to_string()` in `From`, DE0801 route shape, the `api/rest` DTO
  rules).
- E2E: `rm -rf ~/.cf-gears/bss-pricing` **first** — the sqlite file survives between runs and the
  migration chain is amended in place, so a stale file fails with `no such column`. Then
  `python3 tools/scripts/ci.py e2e-local -- --ignore=testing/e2e/gears/oagw/test_websocket.py`;
  plain `make e2e-local` dies collecting an unrelated module.
- **CI on the implementation branch is light** — CRLF and `code-ranker` only, no tests and no
  clippy, and CodeRabbit reports reviews disabled for that base branch. **The local gates are the
  only gates.**
- `gh` defaults to the upstream remote: always pass `-R diffora/gears-rust`.
- Docs waves invalidate the `spec-check` live-corpus pins. After any docs edit run the skill's
  pytest; if a pin moves, hand-check the movers per id against a detached worktree and record the
  note **beside the pin**. Never reword a document to move a score back — that is tuning on the
  sample being measured.
- One error code in the register is a **deliberately pinned unreferenced code**. It is named in
  the local session notes and deliberately **not** repeated here: this directory is inside the
  checked corpus, and a mention would pay the pin down without any rule raising it. A pin paid by
  a prose mention is a pin paid by nothing.
- `--out` belongs to the skill's `neighbourhoods.py`, not `check.py`. Pass `--gear` flags as
  literals — zsh does not word-split an unquoted variable.

---

## 8. Next step

**Phase 3 = Slice 5** ([`../design/05-governance.md`](../design/05-governance.md)): the
`pricing_approval` store, the two-person rule, the materiality evaluator with its registered
always-material triggers, the content pin and its TOCTOU void, the withdraw path. It is what
gives publish an entrance, and it is the last thing keeping 35 registered rules unreachable.

Two obligations it **inherits**, both recorded rather than left to be rediscovered:

1. **Remove the latent freeze before mounting the publish route** (§4, D-177). Mounting the route
   without dealing with the two rostered Slice-10 primitives is the one action that turns a
   contained hazard into permanent bad data.
2. **The audit trail already exists** — six authoring routes write records inside their
   mutation's transactions, on a chain segmented per `(tenant_id, chain_id)`. Slice 5 *adds the
   approval trail* to records that already carry actor, timestamp and before/after, rather than
   building a trail from nothing. Note the three `action` tokens D-175 declared and the
   `correlation_id` producer D-178 named.

After Slice 5 the dependency order is **S7** (windows — three sellability predicates, coverage,
and the whole supersession machinery hang off it), then **S4** and **S6** (tax and consumer
contracts), which Billing needs for D-48.

---

## 9. What phase 3 added to §5 and §6, having paid for it

Recorded here rather than in the register, because these are facts about how this program works
rather than decisions about the product.

**An independent review found defects in every group that passed one — four for four**,
including groups whose implementer had caught a live defect in their own suite and reported it.
Three of those findings were not "a case was missed" but **a false claim the suite confirmed**:
a roster pinned as a count where all 62 members were uniquely named; four wire mappings whose
status *and* code could be corrupted with 1014 tests green; a correlation id replaceable by a
fresh mint at four sites with the whole suite green. **None was found by tests.** All were found
by reading the specification and then trying to break the thing the test named.

**Twice a review overturned not the code but the argument.** One group declined to void orphaned
approval units because "the commit would undo its own authorization" — every void selects
`submitted` and the authorizing record is `approved`. Another kept an audit record in a second
transaction because "a refusal rolls the judgement transaction back" — `judge` returns
`Ok(Refused)` and `in_transaction` commits on `Ok`, so it had always committed, and the fourth
alternative (keep the record in the transaction that already commits) was never considered. Both
arguments were internally coherent and rested on one unexecuted fact. **A coherent argument is
not evidence, and only a run tells them apart.**

**"Exactly one test reddens" is necessary and not sufficient.** A `pricing_plan` trigger arm was
almost wholly shadowed by a later arm: deleting it did not let the illegal statements through, it
only changed which sentence came back, and the test reddened on the message. The statement only
that arm can refuse is a frozen column moving *inside* a sanctioned flip — which is the shape a
real defect takes, since supersession is a write the gear performs. Look for the statement only
the guard under test can refuse.

**A test can be named for a property it does not check.** `a_revision_nobody_approved_never_publishes`
stayed green while a rejected unit authorized a publish, because an undecided record fails
earlier for a different reason. The property needed its own test.

**Improvements taken from measurement must themselves be measured.** Container-per-test was
replaced by a shared server after measuring the flake rate; the replacement leaked a container
per run and produced fifteen spurious reds — the same class it was adopted to remove; its fix
could then drop a live run's database. Three rounds, each found by executing rather than
reasoning. Under guard-by-removal a spurious red is indistinguishable from a second reddened
test, which is what makes harness flakiness a correctness problem here and not a nuisance.
