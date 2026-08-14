<!-- Related: ../DESIGN.md, ../DECISIONS.md, ../design/ | Owners: BSS Product Catalog team -->

# Pricing — read-through review findings (2026-08-10, open)

**What this is.** The running finding list of a layer-by-layer read of the implementation —
contract, gear wiring, REST, domain, infra, tests — done by reading the code rather than by
running a gate. It is **open**: the walk is in progress and entries are appended as layers are
read. Nothing here is a decision. No entry cites a new `D-NN`, and none should be added to
[`DECISIONS.md`](../DECISIONS.md) until it is triaged.

**How to read an entry.** Each carries what was observed, the evidence that was actually opened
(file and line, not a recollection), why it matters, and what closing it would take. `Verified`
means the named lines were read in this session; `Inferred` means the conclusion follows from
what was read but the confirming path was not itself opened.

**Status vocabulary.** `open` — found, untriaged. `known` — the crate already records it in
prose at the named site; the finding is that it is still true, not that it is undiscovered.
`PAID` — closed in a named commit, marked inline at the entry as well as here.

---

## What has been paid, 2026-08-11

Seven commits on `bss/pricing-impl`, each gated at clippy 0 / fast tier green / pg tier green.
**Every High in this document is now closed except Z8-2**, which is latent by its own entry and
has been left alone deliberately: a concurrent session has been working that lane.

| Entry | Commit | What closed, and what did not |
|---|---|---|
| Z12-1 | `e2dad9203` | The pg tier now runs: `make test-pricing-pg` + a step in CI's `integration` job. It was **run before being wired** — 346/346 — rather than after. |
| F-1, F-2, F-3, Z12-4 | `8a77c74d9`, corrected by `eba42b6bf` | Bundle publish evaluates D-104 through `materiality::evaluate`, opens the always-material unit, and answers the two-call shape. **The first version pinned the plan shape and was an approval bypass** — see below. |
| F-4 (blocking half) | `eba42b6bf` | The approver can read the composition: `ApprovalDetailView.pinned_composition`. **The missing `GET` on bundles is still open** — that route is not in §5's endpoint map, so adding it is a divergence to be decided. |
| Z13-15 | `d67a401de` | **Partial, deliberately.** `ScopeKeyParts` + `parts()` compile-gate the **four** sites that *consume* a key. The three that build a key **from** a row or JSON (`to_scope_key`, `scope_key_columns`, `read_scope_key`) cannot be reached by an accessor and are **still open**. |
| Z7-1 | `4eebef3c4` | `tax_category_ref` added to `content_assignments` (30 → 31 columns), armed both directions. |
| Z9-1 | `7f3da53a5` | The cutover appends the sibling audit record and carries `authorization` on its receipt. |
| Z9-3 | `7f3da53a5` | `DecisionRefusal::ForeignWithdraw`. **Its second clause is a proxy** — see the caveat below. |
| Z12-2, Z12-3 | `1c7b22d16` | The denial census reads every mutable plane; the retirement confirm arm reads the store. |
| Z8-1 | `5180ae475` | `pricing_migration`'s key is `(tenant_id, migration_id)` (`m20260802_000065`). |
| Z9-2 | `b8a70a3de` | Both halves. The row-lifecycle filter is `PROJECTED_ROW_STATES`, and `FrozenKey` was **decided as a market**: `select_rows` freezes every live line on it. |

### Three corrections to this document, found by building against it

1. **Z13-15 is wrong on two counts.** Its claim that no site is compile-gated rests on
   `let ScopeKey {` matching nothing; **two sites already were** — they spell it `let Self {`,
   and `is_sibling_of`'s own doc says "an eleventh axis is a compile error here". And three of
   its nine sites cannot be fixed by an accessor at all. A probe armed against the wrong
   spelling, which is the defect this document files elsewhere.
2. **Z12-1's numbers are right; the tier total is not 346.** `cargo test -- --ignored` prints
   **347** — the extra is a doc-test, not a test binary.
3. **Z9-1's suggested probe passes without the fix.** It asks for an audit row whose
   `approval_ref` is the approved unit; `approval_repo::open` already writes a `submit` record
   carrying that same ref. The probe must match the **action** as well. It did exactly this on
   its first run.

### Two things that remain open inside "paid" entries

- **Z9-3's `CatalogAdmin` half is a proxy.** `inst-as-void` names a role; nothing at that layer
  can answer a question about roles, so it is asked as `plan × publish`. A `FinanceManager` can
  still withdraw a `CatalogAdmin`'s unit. Closing it needs the design set to reconcile
  `inst-as-void` with the `approval × approve` gate the endpoint map assigns the route — under
  the default matrix the only principal who can reach `withdraw` is a `FinanceReviewer`, who is
  neither of the two the instruction names. `WITHDRAW_FORBIDDEN` is minted by the crate; §5
  declares no code for a refusal it does not contemplate.
- **Z12-2's plane list is hand-maintained.** A schema-derived one is not reachable from a test:
  `toolkit-db` seals raw `SeaORM` access from downstream crates by design, and the harness
  database is `sqlite::memory:`, so a second connection is a different, empty database.

### The fix for F-1 was itself an approval bypass, and F-4 is why

Marking F-1 as paid is what surfaced it: F-4 sat one entry away, filed
*"latent until F-1 is paid, then blocking"*, and nothing had checked what paying
F-1 made true. Two defects in `8a77c74d9`, both proven by driving them:

1. **The unit pinned `content_hash(&shape)`**, on the reasoning that a composition
   normalizes onto its absorber inside the plan — true at publish, false at submit,
   and D-104 exists precisely because a `sum_of_parts` recomposition carries no
   price-row delta at all. Submit, approve, edit the composition, publish: the
   edited composition published on the old approval. The pin now folds the
   revision's `row_version`, which a composition edit does advance.
2. **The approver was shown a document the act was invisible in.** `GET
   /approvals/{id}` returned the plan shape and nothing else; the mutation proving
   the new assertion prints `"pinned_composition": null` beside `"rows": []` — for
   a `sum_of_parts` bundle, an empty document.

**A fix built from one entry can be refuted by the entry beside it.** Both were
inside the closed-and-reported work for a full working session.

### What the paying turned up that this document did not

**Four tests had pinned a defect as the intended behaviour, each with a comment that
misdescribed it.** `a_decided_unit_frees_every_key_it_held` named a withdrawer that was not the
submitter while calling it "the submitter"; `each_route_asks_the_pdp_for_the_pair_the_catalog_names`
read `last()` where a gate is `first()`; `a_clean_publish_is_accepted_and_is_always_material`
compared the handler's literal against itself; `one_migration_id_holds_one_row` demonstrated its
rejection with `tenant: OTHER_TENANT` while calling it "a client that timed out and rebuilt its
request". **Making a rule enforceable is what surfaces them** — none was visible while the rule
went unenforced.

**A SQLite table rebuild retyped from a reading of the original shed enforcement.** The first
attempt at `m20260802_000065` recreated three of five triggers and shortened a fourth's refusal
message. Extract such statements programmatically; never retype them. What caught it was
`sqlite_migration_repo` asserting refusal **text** and the D-236 roster censuses comparing whole
lists.

---

## Layer 3 — REST surface

### F-1 — `POST /bundles/{bundleId}/publish` evaluates no materiality; D-104 is not enforced
**PAID `8a77c74d9`.** See the status ledger at the top of this document.

**Severity: high. Status: PAID `8a77c74d9` (was: known — recorded in-crate, not paid).**

`publish_bundle` gates on `plan × publish`, validates the composition, and calls
`BundleService::publish_composition`, which commits the composition inside one transaction. No
approval unit is opened, no content is pinned, and no second principal is required — for a
route that sets a marketplace bundle's component set and its per-vendor revenue split.

D-104 registers `bundleComposition` and `revenueShareChange` as **always-material** triggers.
The change sets that carry them exist and **have no caller anywhere in the crate**.

Verified:

- [`src/api/rest/bundles.rs:440`](../../pricing/src/api/rest/bundles.rs) — handler body:
  authz → `validate_publish` → `publish_composition` → `202`. No materiality evaluation.
- [`src/infra/bundle.rs:264`](../../pricing/src/infra/bundle.rs) — `publish_composition`
  reconciles rev-share and writes in `in_transaction`; no unit is opened.
- [`src/infra/bundle.rs:88`](../../pricing/src/infra/bundle.rs),
  [`:100`](../../pricing/src/infra/bundle.rs) — `composition_change_set` /
  `rev_share_change_set`; a crate-wide search for callers returns only the two definitions and
  the two comments below.
- [`src/domain/materiality/triggers_tests.rs:155`](../../pricing/src/domain/materiality/triggers_tests.rs)
  — *"`publish_bundle` evaluates no verdict at all … what is missing is the evaluation, and
  with it D-104's rule."*
- [`src/api/rest/overlays.rs:944`](../../pricing/src/api/rest/overlays.rs) — *"Nothing
  evaluates them, so D-104's rule is not enforced on the bundle publish route today."*

**Why it matters.** This is the governance hole D-104 exists to close, on the one surface where
the money being divided belongs to third parties. It is not a latent defect: the route is
mounted, reachable, and answers `202`.

**The precedent is in the same note.** `priceOverlayMutation` had exactly this shape — a
mounted surface writing its materiality token as a literal while nothing constructed the change
set — and was closed by giving the route a real evaluation
(`api::rest::overlays::overlay_submit_materiality`). The bundle route is one stage behind the
same fix.

**To close:** evaluate the verdict on the publish path, open the always-material unit, and let
the route answer the two-call shape the window mutations already answer
(`submitted_for_approval` then `mutated`). F-2, F-3 and F-4 all fall out of it.

---

### F-2 — the bundle publish response asserts a materiality it never computed
**PAID `8a77c74d9`.** See the status ledger at the top of this document.

**Severity: medium (consequence of F-1). Status: PAID `8a77c74d9`, with F-1.**

The `202` body carries `materiality: "alwaysMaterialTrigger"` as an owned string literal built
in the handler. A client — a console rendering "this change required approval", an integration
branching on the token — is told a property of the request that nothing established.

Verified: [`src/api/rest/bundles.rs:510`](../../pricing/src/api/rest/bundles.rs), and the wire
type's own doc at [`:215`](../../pricing/src/api/rest/bundles.rs) — *"Always
`alwaysMaterialTrigger` for a …"* — which states the constancy as a property of the field
rather than as a result.

**Why it matters separately from F-1.** Even before F-1 is paid, a literal here is the exact
defect the overlay strand was fixed for, and the crate's own rule on it is explicit
([`src/api/rest/overlays.rs:965`](../../pricing/src/api/rest/overlays.rs)): one verdict,
rendered twice, never built twice. A false token is worse than an absent one — it cannot be
distinguished downstream from a real one.

**To close:** with F-1. If F-1 is deferred, the honest interim is to drop the field rather than
keep a value no evaluation produced.

---

### F-3 — `a_clean_publish_is_accepted_and_is_always_material` pins the literal, not the rule
**PAID `8a77c74d9`.** See the status ledger at the top of this document.

**Severity: medium (false green). Status: PAID `8a77c74d9` — re-aimed at the stored unit.**

The test asserts that the response body's `materiality` equals `"alwaysMaterialTrigger"`, with
the message *"D-104: a composition publish is material whatever a threshold says"*. Both sides
of the comparison are literals with the handler's hard-coded string between them. The test
would stay green if every materiality rule in the gear were deleted.

Verified: [`tests/rest_bundles.rs:424`](../../pricing/tests/rest_bundles.rs).

**Why it matters.** It is not merely weak — while it is green it reports that D-104 is covered
on this route, which is the reading a coverage sweep would take from its name. This is the
"probe armed against the wrong claim" shape: the assertion names a rule and tests a spelling.

**To close:** with F-1, re-aim it at the observable consequence — that a single principal's
publish does **not** commit, and that a second, independent principal's approval is what does.

---

### F-4 — a bundle's composition is readable through no surface in the gear

**Severity: medium. Status: PARTIALLY PAID `eba42b6bf` — the approver's read is closed (`pinned_composition`); the absent `GET` on bundles is STILL OPEN, and is a divergence from §5's endpoint map to be decided rather than slipped in.**

There is no `GET` on bundles — the slice mounts `POST /bundles`,
`PATCH /bundles/{bundleId}`, `POST /bundles/{bundleId}/publish` and nothing else, which matches
`design/08-bundles.md` §5's endpoint map exactly. The composition is not reachable anywhere
else either.

Verified:

- [`src/api/rest/bundles.rs:72-76`](../../pricing/src/api/rest/bundles.rs) — the three route
  consts.
- [`src/api/rest/approvals.rs:374`](../../pricing/src/api/rest/approvals.rs) —
  `PinnedContentView` carries plan shape, phases, add-on rules, descriptor set, candidate rows
  and the change contract. No composition member. Its `addon_rules` is the plan's add-on
  compatibility rules, a different entity.
- `ProposedActView` is documented as `null` on every unit that is not a window mutation
  ([`src/api/rest/approvals.rs:660`](../../pricing/src/api/rest/approvals.rs)).
- `bundle` does not appear in `src/api/rest/approvals.rs` at all, nor anywhere in
  `tests/rest_approvals.rs`.

**Why it matters.** Today it is an authoring inconvenience (see F-5). The moment F-1 is paid it
becomes a D-61 violation of exactly the kind the overlay slice mounted its collection `GET` to
avoid: *"a reviewer of an always-material overlay mutation who could not read overlays would be
approving a document they cannot see"*
([`tests/rest_authz.rs:515`](../../pricing/tests/rest_authz.rs)). The sentence transfers to
bundles unchanged.

**To close:** either mount a bundle read, or carry the composition into `PinnedContentView`
beside `proposed_act`, which is the route the window plane already took. The second discharges
D-61 without adding a route the design set does not declare; the first also fixes F-5.

---

### F-5 — wholesale `PATCH` with no read forces the client to be the source of truth

**Severity: low. Status: open.**

`PATCH /bundles/{bundleId}` replaces the open draft revision's composition **wholesale**
(D-215). With no read surface (F-4), a client adding one component must resend a set it cannot
fetch. The `If-Match` on the route carries the **plan revision's** entity tag rather than the
composition's ([`src/api/rest/bundles.rs:18`](../../pricing/src/api/rest/bundles.rs)), so a
concurrent edit is correctly refused — but the client has no way to re-read and retry, which is
the recovery a precondition normally implies.

**To close:** falls out of F-4's first option. No separate work.

---

## Layer 2 — gear wiring

### F-6 — `infra::jobs`' module doc still says two jobs; three tickers run

**Severity: low (documentation). Status: open.**

The module doc opens *"**Two, and they are independent**"* and carries two bullets, while the
module declares `pub mod gated_markets;` and `serve` spawns, leases and joins **three** tickers
(`readmodel-warm`, `window-activation`, `gated-markets`), each with its own key.

Verified:

- [`src/infra/jobs.rs:3`](../../pricing/src/infra/jobs.rs) — the count and the two bullets;
  `gated_markets` has no entry.
- [`src/module.rs:169-211`](../../pricing/src/module.rs) — `serve` spawns three and `stop`
  joins three.
- [`src/module.rs:77`](../../pricing/src/module.rs) — `GATED_MARKETS_LEASE_KEY`.

**Why it is worth an entry.** The same doc already carries a paragraph about a count in it
going stale, ending *"A count in prose beside a roster in code is the shape that goes stale, so
the count is gone rather than corrected"* — and the correction removed the count from one
sentence while the opening sentence carries a new one. `module.rs`'s own `serve` doc says
"Three tickers", so the two docs disagree.

**To close:** drop the leading count and add the `gated_markets` bullet, so the roster is the
list and nothing counts it.

---

## Layer 4 — domain, Foundation core

Found by an independent `/code-review high` pass over the nine Foundation modules
(`scope_key`, `validation`, `rules`, `publish`, `read_model`, `events`, `concurrency`, `error`,
`projection`), run after the read-through. The read-through itself produced no finding on this
layer.

### F-7 — the frozen-payload classification guard is armed backwards

**Severity: high (false green). Status: open.**

`every_member_of_the_frozen_payload_is_classified_exactly_once` asserts
`assert_eq!(named, 22)` where `named` is the total length of
`partition_delta_members`' two classification lists. The guard **passes** when a new
`PlanSubjectDelta` member is named in the destructure and left out of **both** lists — `named`
stays 22 — and **fails** only once the member is correctly classified, when `named` becomes 23.

It fires on the fix and is silent on the omission, which is the exact D-303 defect its own
comment claims to cover: *"naming it in the pattern and forgetting it in both lists compiles
fine … This case is what makes the omission fail."*

Verified: [`src/domain/projection_tests.rs:1049-1070`](../../pricing/src/domain/projection_tests.rs)
— the assertion and its message were read; `partition_delta_members` has no rest pattern, so
the compile-error half of the pair does hold and only the classification half is unguarded.

**Why it matters.** The no-rest destructure catches *adding* a member. This test is the only
thing standing between a named-but-unclassified member and a silently mis-rendered payload, and
it does not stand there. While green it reports D-303 as covered.

**To close:** derive the expected count from the struct rather than from a literal, or assert
the classification directly — that every name the destructure produces appears in exactly one
list. A literal total can only ever be a reminder to bump a number, and it bites the person who
did the right thing.

---

### F-8 — `UsageLineAxisMismatch`'s doc names a producer that does not produce it

**Severity: low (documentation, with a real inconsistency behind it). Status: open, re-check
needed.**

The variant's doc names two producers: `domain::scope_key::check_usage_line_axes` and
`price_repo::check_authored_usage_line`. The first returns
`Err(DomainError::ValidationFailed(report))` carrying `USAGE_LINE_AXIS_MISMATCH` inside the
report, not this variant — so one rule reaches the wire under two different violation shapes,
and only the second raises the variant the doc is attached to.

Verified: [`src/domain/error.rs:118`](../../pricing/src/domain/error.rs) — the doc;
[`src/domain/scope_key.rs:523-537`](../../pricing/src/domain/scope_key.rs) — the signature and
the `ValidationFailed` return.

**Re-check before acting.** `domain/error.rs` was uncommitted and under active edit by a
concurrent session at the time of this pass, so the doc read here may be in-flight rather than
landed.

**To close:** decide which shape the rule reports under and make both sites use it, then fix
the doc to match. A consumer matching on the code string is served either way; a consumer
matching on the canonical category is not.

---

## Layer 5 — capability slices

Read-through over `materiality`, `approval`, `lifecycle`, `window`, `sellability`,
`supersession`, `contracts`, `overlay_rules`; independent `/code-review high` over those plus
`coverage` and `bundle_rules` and their production callers (~7,100 lines). The read-through
produced no finding; both entries below are the independent pass's, verified afterwards.

### F-9 — the cross-class tie warning never intersects with the candidate's targets

**Severity: medium (false advisory, misleading subject list). Status: open.**

`check_cross_class_tie` walks `candidate.world.cross_class_ties` and renders `tie.plans`
without ever intersecting them with the candidate's own `target_ref`. The supplier,
`overlay_repo::collect_cross_class_tie`, fills `plans` from the **holder's** line key or the
**holder's** `target_ref.plans` — never the candidate's. So the warning fires for every
same-precedence overlay in a different scope class, including ones whose targets are disjoint
from the candidate's, and the message names plans the candidate does not reach.

Verified:

- [`src/domain/overlay_rules.rs:787-812`](../../pricing/src/domain/overlay_rules.rs) — the walk;
  no intersection anywhere in the body.
- [`src/infra/storage/repo/overlay_repo.rs:1845-1869`](../../pricing/src/infra/storage/repo/overlay_repo.rs)
  — `plans` sourced from `holder.target_ref.plans` / the holder's line key.

**Not blocking.** It is `report.warn`, so no publish is refused. The cost is an advisory that
cries on unrelated overlays and names the wrong plans in its text — which is the failure mode
`inst-plv-class-tiebreak` exists to avoid, since the warning's whole purpose is to tell an
author *which overlay ties with mine*.

**The shape is known one rule over.** The sibling `check_replacement` performs exactly this
intersection against `layers_beneath`.

**One sub-claim not confirmed.** The independent pass also called the
`if tie.plans.is_empty() { continue; }` guard dead. It is only dead if a holder with an empty
`target_ref.plans` and no line-key plan is unrepresentable; that was not established here.
Check it before deleting the guard as part of the fix.

**To close:** intersect `tie.plans` with the candidate's target set, warn only on a non-empty
intersection, and render that intersection rather than the holder's whole reach.

---

### F-10 — a market listed twice yields duplicate identical tax-basis violations

**Severity: low. Status: open.**

`check_tax_basis` iterates `composition.markets` — a `Vec` built by `markets_of` straight off
the request body with no dedup — so a caller who lists `(USD, EU)` twice gets the same
`BUNDLE_TAX_BASIS_MIXED` violation twice, against the same subject and with the same detail.

Verified:

- [`src/domain/bundle_rules.rs:363-370`](../../pricing/src/domain/bundle_rules.rs) — the loop
  over the raw `Vec`.
- [`src/api/rest/bundles.rs:276-282`](../../pricing/src/api/rest/bundles.rs) — `markets_of`
  maps and collects, nothing more.

**Why it is worth an entry at low severity.** The aggregate report is the slice's remediation
contract — one violation per failing rule, so a composition is fixable in one pass — and a
duplicated entry breaks the count an operator reads. The module doc records the same defect
being fixed for `check_coverage` by switching to a `BTreeSet` difference; the fix was not
carried to this rule.

**To close:** dedup the market set once, at the edge or at the walk. Doing it in `markets_of`
also fixes any later rule that iterates the same `Vec`.

---

### F-11 — a large configured notice period panics instead of refusing

**Severity: high (panic on a request path). Status: open.**

`NoticePeriod::earliest_effective` computes `announced_at + Duration::days(self.days)`.
`NoticePeriod::resolved` clamps only **from below** (`< 60 → 60`) and the backing column's only
constraint is `>= 60`, so an arbitrarily large configured period reaches the addition,
overflows chrono's representable range, and panics — a 500 where the surrounding code's whole
posture is to refuse with a named code.

Verified: [`src/domain/migration.rs:243-272`](../../pricing/src/domain/migration.rs) — the
one-sided clamp and the unchecked addition.

**To close:** clamp from above as well, or use `checked_add_signed` and refuse the
out-of-range period the way `MigrationNoticeTooShort` refuses the too-small one. The refusal
already names the configured period, so the message shape exists.

---

### F-12 — `generation_key` hand-enumerates the scope key's axes

**Severity: medium (latent; bites the next axis). Status: open.**

`cutover::generation_key` builds the grandfathered successor's key by reading the predecessor's
axes through accessors, one call at a time. An eleventh axis compiles unchanged here and is
silently dropped from the copy — while the function's own doc presents it as the guard against
exactly that.

Verified: [`src/domain/cutover.rs:227-245`](../../pricing/src/domain/cutover.rs).

**No loss today**, and the reason is worth recording so the entry is not over-read:
`PriceOverlay` has a single variant (`Base`), so the one axis the constructor defaults rather
than copies cannot currently carry information. The finding is about the next widening.

**Precedent, and it cost four defects.** `scope_key.rs`'s own module doc records the ninth and
tenth axes landing and four sites being found separately over two days, because each enumerated
the axes by hand. `evaluation_policy::partition_row_fields` is the shape that prevents it — a
destructure with no `..` arm, which does not compile until the new member is classified.

**To close:** rebuild the key from an exhaustive destructure of the predecessor, so a new axis
is a compile error here.

---

### F-13 — `Boundary.frequency` collapses every custom frequency to one token

**Severity: medium (latent, hatches with the feature). Status: open.**

`Boundary.frequency` is a string, and its only producer renders it through `Frequency`'s
`Display`, which spells every `CustomEveryN { n, unit }` as `custom_every_n`. K3's
"matching frequency" arm therefore passes a migration between two *different* custom
frequencies.

Verified: [`src/domain/migration_delta.rs:140`](../../pricing/src/domain/migration_delta.rs)
and `Frequency`'s `Display`.

**Why it matters despite being unreachable now.** The subject set is always empty today (the
Subscriptions lane is absent — see the pattern note below), so no input reaches the arm. It
becomes live on the day the lane lands, and it will land with the existing tests green: nothing
in the current suite can distinguish the two custom frequencies either.

**To close:** compare the structured frequency, not its `Display`. A `Display` is a rendering;
using one as an equality key is the same class of mistake as keying on a formatted date.

---

### F-14 — a dangling `depends_on` edge earns a second, misleading coverage violation

**Severity: low. Status: open.**

`currency_binding::mandatory_closure` walks `depends_on` edges that point outside the plan's own
add-on set, so a dangling edge produces an extra `CURRENCY_NOT_COVERED` naming a SKU the plan
never composed. The plan-authored membership rule (`AddonEdgeMembership`) already refuses that
edge under its own code, so the operator is told about one defect twice, once under a code that
misdescribes it.

Verified: [`src/domain/currency_binding.rs:235`](../../pricing/src/domain/currency_binding.rs).

**To close:** intersect the closure with the plan's declared add-on set before asking coverage
of it.

---

### F-15 — four operator-facing refusal strings lost their line continuations

**Severity: low (cosmetic, but ships to the caller). Status: open.**

All four refusal literals in `cutover.rs` carry 14–18 consecutive spaces mid-sentence, from
missing `\` continuations in the multi-line string literals. They ship as the RFC 9457 `detail`.
Isolated to this file.

Verified: [`src/domain/cutover.rs:135`](../../pricing/src/domain/cutover.rs) and its three
siblings.

---

### F-16 — the empty-resolution refusal renders an empty list

**Severity: low. Status: open.**

`synthesis::ensure_complete` refuses an empty resolution set but composes its message only from
`unresolved`, so the empty case reads `on 0 scope key(s) … : ` with nothing after the colon.

Verified: [`src/domain/synthesis.rs:304`](../../pricing/src/domain/synthesis.rs).

---

### F-17 — the supersession guard's field count is stale in two docs

**Severity: low (documentation). Status: open.**

`price_row.rs`'s doc says the supersession guard adds **three** fields to the shared list;
`mismatched_unit_fields` adds **four** (`reservationFlavor`, D-254). `supersession.rs`'s own
"Ten fields" header is stale in the same way.

Verified: [`src/domain/price_row.rs:748`](../../pricing/src/domain/price_row.rs).

---

### F-18 — the plan-shape pipeline's rule count is stale

**Severity: low (documentation). Status: open.**

`plan_shape_rules`' doc reads *"none of the **twenty** below holds mutable state"*; the builder
registers **26** (8 cycle-shape, 6 composition, 2 composite, 9 phase-graph, 1 descriptors).

Verified: [`src/domain/plan_rules.rs:386-430`](../../pricing/src/domain/plan_rules.rs) —
counted from the `with_rule` chain.

**Same class as F-6.** A count in prose beside a roster in code. The crate's own stated remedy
is to drop the count rather than correct it.

---

### Noted, not filed — `AvailabilityInsideCoverage` mixes window rosters

`coverage.rs:719` uses `covers()` / `has_live_window()` (over `COVERING_STATES`, which excludes
`expired`) for one half and `coverage_end()` (which counts `expired`) for the `availableTo`
half, contrary to the `COVERING_STATES` doc's own argument. The independent pass could not
construct a store-reachable input where the two rosters change the verdict — the overlap rules
plus the activation sweep force every live window's end past every expired window's end on a
key — so it is recorded here as a comment owed rather than as a finding.

---

## Out of scope — a concurrent session's in-flight work

The same review pass reached uncommitted files belonging to another session's repricing strand.
Recorded here so the findings are not lost, and **explicitly not** part of this walk's list —
they belong to whoever owns that branch of work, and the code may have moved since.

| Where | What |
|---|---|
| `api/rest/repricing_runs.rs:663` | `cohort: request.cohort.map(Cohort::Generation)` skips `instant::check_quantum` while the SQL predicate compares through `Cohort::Display` = `timestamp_millis()`. A sub-millisecond cohort is silently truncated and selects a **different generation's** rows. `ScopeKey::new` refuses the same input, so this path bypasses a validation the domain already owns. |
| `api/rest/repricing_runs.rs:641` | `selector_of` never applies `check_usage_line_axes`, so `{charge_kind: "recurring", meter: "api_calls"}` — or a `dimension_key` with no `meter` — is accepted, matches nothing, and returns `RUN_SELECTOR_EMPTY`: the outcome the function's own doc says it exists to prevent. |
| `infra/storage/repo/price_repo.rs:1908` | `load_published_for_selector` imposes no page bound, while the sibling `repricing_journal_repo::list_for_run` justifies itself on having one. An unconstrained run materializes the whole published catalog, writes it one INSERT per row inside a single transaction, and echoes the entire journal in the `202` body. |
| `domain/repricing_tests.rs:3` | `cargo fmt --check` fails on import ordering; CI's Fmt job rejects the tree as it stands. |

---

## Checked and found sound

Recorded so the same questions are not re-opened later. Each was read and the reasoning in the
code answers it.

| Question | Answer | Where the argument is |
|---|---|---|
| Why `POST /prices/{priceId}/windows` but `PATCH`/`DELETE /price-windows/{windowId}`? | Creation needs the parent; mutation does not. A window id is unique on its own, so nesting would put a second identifier in the path the server must check agrees with the first — the `require_same_revision` arm that exists on the price routes, bought for nothing here. §5's own shape. | [`src/api/rest/windows.rs:179`](../../pricing/src/api/rest/windows.rs) |
| Why does `GET /migrated-origin-snapshots/{subscriptionRef}` exist, and why is its authz object `plan`? | The payload resolves through **no** `CatalogVersion` by construction (D-87), so the version-keyed read model cannot carry it; before D-102 it had no reader-facing surface while the PRD required Rating to evaluate from it. The gate is `plan × read` because the content is the catalog's; asking the PDP about a subscription id as a `plan` resource would be a question about an object of the wrong type. Tenant binding is stated explicitly because it does not come free here (review fix N-3). | [`src/api/rest/migrated_origin_snapshots.rs:1-40`](../../pricing/src/api/rest/migrated_origin_snapshots.rs) |
| Does `plan × preview` need to be its own action? | Yes, and for an audience rather than an operation: the preview grant is an extra assignment the default role matrix does not carry, evaluated against the grant's explicit pricing-region set. Filing it under `read` hands a partner surface to every catalog reader; granting a partner `read` hands them an authoring read including drafts. Invisible to any allow/deny fixture, which is why `rest_authz`'s census asserts the pair. | [`src/api/rest/preview.rs:44`](../../pricing/src/api/rest/preview.rs), [`src/authz.rs:122`](../../pricing/src/authz.rs) |
| Is the absence of `GET /bundles` a spec deviation? | No — `design/08-bundles.md` §5 declares three routes and no read. The finding is F-4, about what that absence costs once F-1 is paid, not about the route roster diverging from the design. | `design/08-bundles.md` §5 |

---

## Layers not yet read

**All of it was read in Part II below** (2026-08-10 evening, pinned at `6ae81d5ec`): infra storage
and the migration chain (Z6), the repositories in two halves (Z7, Z8), the service layer (Z9), the
background plane and the projector (Z10), the three authoring-throughput slices this paragraph
named — `import` / `bulk`, `snapshot`, `money` / `instant` (Z11) — the test tiers as a review object
(Z12), and eight whole-crate consistency sweeps (Z13). What remains unread after both parts is
recorded at the end of each zone block under **Not covered**; the largest residues are the bodies of
the `postgres_*` suites (they run nowhere — Z12-1), the ~58 migration bodies Z6 sampled by roster
rather than reading whole, and the trigger whitelists checked for column names but not against the
current lifecycle machine.

**Read so far:** layer 1 (the SDK contract — no findings), layer 2 (gear wiring — F-6), layer 3
(REST — F-1…F-5), layer 4 (Foundation domain — F-7, F-8), layer 5 (22 slice modules — F-9…F-18).
From layer 4 on, each layer is read first and then given an independent `/code-review high`
pass; **every finding from layer 4 onward came from the independent pass**, not the
read-through, which is worth knowing when weighing what each half of the method is buying.

## A pattern worth carrying into triage

Two recurring shapes account for most of the list, and both are cheaper to fix as a class than
one at a time.

**A fix that was not carried to its sibling.** F-1 (the overlay strand's literal-materiality
defect, fixed there and not on bundles), F-9 (`check_replacement` intersects; the tie rule does
not), F-10 (`check_coverage` dedups via `BTreeSet`; `check_tax_basis` does not), F-12
(`partition_row_fields` destructures exhaustively; `generation_key` enumerates by hand). When
fixing any one of these, the useful question is *where else does this form appear* rather than
*is this site now correct*.

**A rule whose counterpart system does not exist, resolved fail-closed.** Retirement's D-79
presence lane, `migration_delta`'s contract-lock registry, `synthesis`' tier-2 reference set,
five of the six `plan_rules` §5 codes, `rules.rs`' two `inst-la-*` checks, `overlay_rules`'
unreachable publish-time arm. Each is a deliberate, documented safe branch — but it means a
non-trivial share of the built behaviour is the degraded half, and F-13 shows the hazard: a
latent defect sitting behind an absent lane hatches **with green tests** on the day the lane
lands. Anything filed as "latent" here deserves a test that arms against the live case, not a
note to remember later.

---

# Part II — infra, the test tiers, and the whole-crate sweeps (2026-08-10 evening)

**What this is.** The continuation of the walk above, over the surface Part I recorded as unread:
infra (storage, repositories, services, the background plane), the test tiers, and the class-based
consistency sweeps. Method per `vp-implementation-review`: zone the surface, one pass per zone,
read the code *and* its tests, verify every claim by opening the file or running the grep, run the
static smell catalog and the control-flow catalog, and finish with the sibling-consistency pass —
recording the refutations as well as the findings.

**Review baseline.** Pinned at `6ae81d5ec`, read-only. A concurrent session was committing to this
crate throughout the pass (it landed `6ae81d5ec` itself), so nothing here was built, run or edited,
and every claim is a claim about the text at that commit. `overlay_repo.rs`, `api/rest/overlays.rs`
and the two overlay suites were mid-flight at the start of the pass — Z8-12 in particular may
already have moved.

**Identifier scheme.** Part I numbers its findings `F-N`. This part keeps the zone-local ids
(`Z6-1`, `Z13-15`) rather than folding 89 entries into one sequence: the zone is the unit that was
verified as a whole, and a renumbering would break the trail back to the pass that produced each
entry. Nothing here is a decision, and no entry cites a new `D-NN`.

**Zones.**

| Zone | Surface | Result |
|---|---|---|
| Z6 | storage foundation — `storage.rs`, 36 entities, the 64-migration chain | 2 Med, 6 Low |
| Z7 | catalog/money repositories — `price_repo` (3679 lines), `plan_repo`, `plan_shape_repo`, `catalog_version_ref_repo`, `taxonomy_repo` | 1 High, 2 Med, 7 Low |
| Z8 | governance/lifecycle repositories — approval, overlay, window, bundle, outbox, bulk, idempotency, journal, +7 | 2 High, 6 Med, 6 Low |
| Z9 | infra service layer — approval, window, supersession, publish, cutover, clone, retirement, bundle, read_model, +13 | 3 High, 3 Med, 4 Low |
| Z10 | the background plane — three tickers, metrics, the outbox projector | 5 Med, 7 Low |
| Z11 | the domain slices Part I left unread — import, bulk, snapshot, money, instant | 6 Med, 4 Low |
| Z12 | the test tiers **as a review object** — what the suite fails to prove | 4 High, 3 Med, 2 Low |
| Z13 | whole-crate sweeps S1–S8 | 1 High, 7 Med, 7 Low |

**Eighty-nine entries: 11 High, 34 Medium, 43 Low, and no Critical.**

## Executive summary

**The verdict is that the crate is in better shape than the count suggests.** No entry in this part
crosses a tenant boundary at runtime and none breaks financial integrity on a live path. The
storage chain, the CAS discipline in the repositories, the gate ordering in the services, the
canonical-error ladder (53 variants, 53 arms, no wildcard) and the REST authz census are all
genuinely strong, and several of them are *proved* rather than asserted. What the pass found is
concentrated in three shapes, and all three are cheaper to pay as a class than one at a time.

**1. A fix carried to one site and not to its sibling.** This is the single most common shape in the
list, and it is the same one Part I closes on. Z9-1 (the supersession audits its commit; the
cutover, written from the same eleven-step skeleton, stops one step short), Z7-1 (`content_model`
writes `tax_category_ref`; `content_assignments` does not), Z10-2 (two tickers release their lease,
the third drops it), Z10-1 (D-238 wired the alarm port into two of three tickers), Z9-4 (the
supersession refuses a divergent successor; the cutover does not), Z13-2 (two surfaces import
`OUTCOME_SUBMITTED`, three re-spell it), Z8-6, Z8-7, Z11-2, Z11-3. When fixing any one of these,
the useful question is *where else does this form appear*.

**2. A hand-enumeration where the compiler could hold the rule.** Z13-15 is the important one and it
is filed High for a reason that is not hypothetical: the same class fired once already. D-196
widened the scope key from eight axes to ten; the sweep that widened it missed three sites, and the
code records what each miss cost — including a content pin that framed eight axes, so two window
plans on two meters of one market pinned identically and an approve could be satisfied by a
re-derivation over the other line's coverage. That is an approval bypass, and it shipped. There is
no `let ScopeKey { .. }` destructure anywhere in the crate: **not one of the eight remaining sites
is compile-gated.** Z6-2 (a 44-name frozen-column array cross-checked against the literal `44`, on
the table that holds money) and Z6-4 are the same shape on the schema side.

**3. A rule whose counterpart system does not exist, resolved fail-closed — and therefore untested
against the live case.** Z11-1 (the ISO-4217 scale rule has no caller at all, while three design
documents describe `PRECISION_EXCEEDED` as a live publish refusal), Z8-2 (`mark_applied` discards
`rows_affected` and has no production caller yet), Z7-10, Z9-2, Z13-6, Z13-12. Each is a deliberate
safe branch today; each hatches **with green tests** on the day its lane lands. Part I's closing
sentence applies unchanged: anything filed as latent deserves a test armed against the live case,
not a note to remember later.

**The one finding that is about the review apparatus rather than the code is Z12-1**, and it is the
one to act on first because it changes what every other green signal means. The Postgres tier is
**346 tests and 346 `#[ignore]`s**, and `--run-ignored` / `--ignored` / `include-ignored` appears
nowhere in the `Makefile` or in any workflow. Every proof this crate owns about two racing writers,
a lock held by a crashed pass, `FOR UPDATE` semantics, READ COMMITTED re-evaluation and the
PL/pgSQL half of every dual-spelled trigger lives in a tier that has never run automatically —
all five files in `tests/` that use `tokio::spawn`/`join!` are in it. `sqlite_append_only.rs:110`
already names what that costs, from experience: *"D-236 is the record of what that costs — a
premise living on one tier only means a run without Docker reports a clean change through a guard
that stopped guarding."*

**Highs, in the order I would pay them:**

| Id | Finding | Why first |
|---|---|---|
| Z12-1 | the Postgres tier runs in no CI job | changes the meaning of every other green |
| Z13-15 | eight hand-enumerated scope-key sites, none compile-gated | the class already produced an approval bypass |
| Z9-1 | the cutover commits four table changes and writes no audit record | the act that moves money has no trail and no `approval_ref` |
| Z7-1 | `PATCH` cannot write `tax_category_ref` | the correction is unexpressible, and publish freezes the wrong category for seven years |
| Z9-3 | `inst-as-void`'s withdrawer rule is enforced by neither layer | any approver may close another principal's review and release its held keys |
| Z8-1 | `migration_id` is a client key in a global uniqueness namespace | a permanent cross-tenant refusal with no remedy |
| Z9-2 | legacy-snapshot synthesis resolves a market on two of ten axes | latent; hatches when the migration executor gains a writer |
| Z8-2 | `mark_applied`/`mark_failed` discard `rows_affected` | latent; a double-apply on the repricing lane |
| Z12-2/3/4 | census readback covers 3 planes of 4; retirement and bundle publish assert response bodies only | Z12-4 is where one line would have caught F-1 |

## What was verified by hand rather than accepted

Every zone block below is the work of a separate pass. Before filing, the load-bearing claims were
re-opened directly:

- **Z7-1** — confirmed. `tax_category_ref` appears four times in `price_repo.rs` and not once
  between `content_assignments`' bounds. Refined: the `200` body renders the *stored* record
  (`api/rest/prices.rs:385`), so the dropped write is visible to a client who re-reads the body —
  it is an unrefused no-op, not a silent revert.
- **Z8-1** — confirmed. `migration_id uuid NOT NULL PRIMARY KEY`, `OnConflict::column(MigrationId)`
  (the only single-column conflict target in the crate; both siblings scope theirs by tenant), and
  the value arrives straight off the request body. Calibrated: exploiting it requires *knowing or
  predicting* the UUID, so with random v4 ids this amplifies any identifier leak rather than
  standing on its own. High, not Critical.
- **Z9-1** — confirmed. Seven infra services call `audit_repo::append`; `cutover.rs` contains no
  occurrence of `audit_repo`, `AuditAction` or `NewAuditEntry`.
- **Z9-3** — confirmed. `submitter_principal` is compared in exactly two places crate-wide:
  `decision.rs:333`, inside `if let Some(approver)` (and `approver()` is `None` on every void), and
  `independent_approver`, which judges an already-approved record. Nothing compares the withdrawer
  to the submitter.
- **Z10-2** — confirmed. `drop(guard)` at `module.rs:459` against `guard.release().await` at
  `:302` and `:478`.
- **Z10-4** — confirmed, and strengthened: `TAX_ENGINE_GA` has two real readers
  (`metrics.rs:303`, `read_model.rs:933`); `price_repo::gated_markets` is the third consumer of the
  same predicate and mentions the constant only in prose, so a grep by name cannot find it.
- **Z12-1** — confirmed with numbers: 346 tests, 346 `#[ignore]`s, no `--run-ignored` anywhere.
- **Z13-6** — confirmed. The sole writer of `pricing_policy_object` sets 4 of 11 columns and
  `..Default::default()`s the rest.
- **Z13-15** — confirmed by its absence: `let ScopeKey {` matches nothing in the crate.

Second round, closing the gap: the five remaining High entries were re-opened as well, so **every
High in this part has been verified twice — once by its zone pass and once directly.**

- **Z8-2** — confirmed. Both `mark_applied` and `mark_failed` end `.exec(runner).await.map_err(…)?;
  Ok(())`, discarding `rows_affected`. `grep` over `src/` returns **no caller at all** outside the
  repository file; the only callers anywhere are `tests/sqlite_repricing_journal_repo.rs:228` and
  `:231`, both marking rows that exist. Latent exactly as filed.
- **Z9-2** — confirmed. `FrozenKey` is `{ currency, region }` — two fields. The candidate filter
  (`synthesis.rs:101-115`) tests cancelled-ness, currency, region and the half-open interval, and
  names no other axis and no price-row `lifecycle_state`; `live.sort_by_key(price_id)` then a
  `first()` keeps the lowest id.
- **Z12-2** — confirmed. The denial loop takes exactly four readbacks — `plan_count`,
  `plan_row_version`, `price_rows`, `approval_row(...).state` — over a census filtered to *every*
  mutating route, and compares the price plane through `prices_after.first()`. The overlay, bundle,
  window, taxonomy, policy, migration and bulk planes are read by nothing in it. The test's own
  comment makes the argument that generalises to all seven.
- **Z12-3** — confirmed. `rest_retirement.rs:18` imports `Harness, body_json, request,
  seed_publishable_plan` and no store-readback helper; the confirm arm asserts status plus five
  fields of the response it just received. Neither half of its name is proven.
- **Z12-4** — confirmed, and it is a tautology in the strict sense: the test asserts
  `body["materiality"] == "alwaysMaterialTrigger"` while `api/rest/bundles.rs:510` writes
  `materiality: "alwaysMaterialTrigger".to_owned()` unconditionally. No input can redden it.

The Mediums and Lows below rest on their zone pass's evidence, which cites `file:line` throughout
and was read but not independently re-opened — except Z10-2, Z10-4 and Z13-6, listed above.

**Severity re-calibrations applied to the zone blocks as written below:** Z12-1 was filed Critical
by its own pass under a zone-local scale; it is restated as High here so one ladder governs the
whole document — Critical is reserved for a live cross-tenant reach or an active
financial-integrity break, and a gap in the proofs is neither, however consequential.

---
**Zone Z6 — storage foundation**

**Files:** `gears/bss/pricing/pricing/src/infra/storage.rs` (918 lines, read whole); `src/infra/storage/entity.rs` + all 36 entity modules (2 256 lines, read whole); `src/infra/storage/migrations.rs` (316 lines, read whole) + the 64-migration chain (14 170 lines — read whole: 000002, 000016, 000021, 000023, 000043, 000057, 000063, 000064; read in part: 000001, 000010, 000015, 000019, 000035, 000044, 000045, 000046, 000048, 000060, 000062; the rest sampled by targeted grep and by the test rosters). Evidence-only: `tests/postgres_schema_*.rs`, `tests/postgres_migrations.rs`, `tests/sqlite_migrations.rs`, `tests/sqlite_plan_guards.rs`, `src/infra/storage_tests.rs`.

**What's done:** A 64-migration greenfield chain, one table per migration, written as paired raw-SQL const arrays (Postgres canonical in schema `bss`, `SQLite` mirror) dispatched through one `exec_backend`. It builds 36 tables with 142 named CHECKs, 49 indexes (12 partial), 29 PL/pgSQL trigger functions and their `SQLite` `RAISE(ABORT)` equivalents — including the append-only frozen-column whitelists that are the physical half of catalog immutability. On top sits a `SeaORM` entity per table, each tenant-scoped through `Scopable`/`#[secure(...)]`, and a 28-variant `RepoError` whose `repo_failure` fold is the single translation point into `DomainError`.

**Verdict:** Sound. The schema itself is the strongest part of this crate I have read: the scope-key uniqueness, the frozen-column whitelists and the state machines are complete and identical across both engines as the chain **now stands**, and the census tests are unusually honest about what they do and do not prove. The findings below are all in the second ring — one unobserved arm of the error fold, and coverage/roster asymmetries where a discipline invented for one table was not carried to its siblings. Nothing here is a cross-tenant or financial-integrity break.

**Findings**

**Z6-1 [Medium] One arm of the `repo_failure` fold is observed by nothing, and its wire code appears in no test at all**

- `src/infra/storage.rs:872` — `RepoError::UsageLineDisagrees { .. } => DomainError::UsageLineAxisMismatch(err.to_string())`
- `src/infra/storage/repo/price_repo.rs:2491`, `:2504`, `:2528` — the three producers of `RepoError::UsageLineDisagrees`
- `src/infra/error_mapping.rs:173-177` — `D::UsageLineAxisMismatch(detail) => … "USAGE_LINE_AXIS_MISMATCH"`
- `grep -rn "UsageLineDisagrees" tests/` → two hits, both **doc comments** (`tests/sqlite_cutover_unit.rs:230`, `tests/sqlite_import.rs:133`); no assertion
- `grep -rln "USAGE_LINE_AXIS_MISMATCH" tests/` → **empty**. The in-`src` hits (`src/domain/scope_key_tests.rs:377/392/403`) assert the *domain* rule's violation code produced by `scope_key::check_usage_line_axes`, a different producer that never travels through this fold.

This is the exact class the fold's own comment records having been caught once: `src/infra/storage.rs:897-902` says `WindowOverlap` "was the seventh arm of this fold found with no observation of what it maps to: the store's own suite asserted the `RepoError` and stopped, so corrupting this line to `Internal` left the whole crate green while a colliding interval answered 500." Corrupting line 872 to `DomainError::Internal` today leaves the whole crate green in the same way, and an author whose row's meter disagrees with its key reads a 500 instead of the 422 §4.1 declares. `src/infra/storage_tests.rs` exercises 21 of the 28 variants through `repo_failure`; the seven it does not are `BulkRowLocked`, `ConcurrentMutation`, `DuplicateBundleOnPlan`, `OverlayPrecedenceHeld`, `OverlayOpenDraftExists`, `ApprovalNotPending` and `UsageLineDisagrees`. Five of the other six are reached indirectly by a test that asserts the resulting wire code end to end (`PRECEDENCE_DUPLICATE` in `tests/rest_overlays.rs`, `BUNDLE_EXISTS_ON_PLAN` in `tests/rest_bundles.rs`, `APPROVAL_NOT_PENDING` in `tests/rest_approvals.rs`, `OPEN_DRAFT_REVISION_EXISTS` in `tests/sqlite_plan_repo.rs`, `CONCURRENT_MUTATION` in six files); `UsageLineDisagrees` is the one with neither.

*Fix/Verify:* add a `storage_tests.rs` case asserting `repo_failure(&RepoError::UsageLineDisagrees{..})` is `DomainError::UsageLineAxisMismatch`, and one end-to-end case that reads `USAGE_LINE_AXIS_MISMATCH` off a `PATCH` whose row meter disagrees with its key.

**PAID `f3ecc2a93` — and the arm is NOT dead, which is the opposite of what a first reading suggests.** Tracing the writers rather than the registrations found three live producers reachable from the wire: `resolve_authored_usage_line` on create and on bulk import, and `check_update_keeps_the_line` on `PATCH`. The usage line is authored on the content view, so a caller really can state one the key disagrees with, on all three paths. So this was a coverage finding, not a dead branch, and deleting the arm — the tempting cheap answer — would have removed a live refusal. Two cases: the fold arm, and a `PATCH` naming no `scope_key` so only the store's guard can refuse. RED with the arm corrupted to `Internal`: the unit test panics on the mapping and the e2e gets the literal 500 this entry predicted.

**Z6-2 [Medium] The table-driven frozen-column census was written for `pricing_plan` only; `pricing_price` still rests on a hand-enumeration and a literal count**

- `tests/sqlite_plan_guards.rs:641` `the_frozen_whitelist_names_every_content_column_the_table_holds` — reads the columns off `pragma_table_info('pricing_plan')` (line 652) and the predicate off `sqlite_master`, so a column added and unguarded reddens
- `tests/postgres_schema_plan.rs:994` — the same case against `information_schema.columns` (line 1003)
- `tests/postgres_schema_price.rs:1010` `every_frozen_column_of_a_published_row_refuses_to_move` — a **hand-written** 44-element array, cross-checked at line 1077 against the **literal** `44`
- `src/infra/storage/migrations/m20260802_000062_…rs:10-16` claims the fix: "both engines' censuses enumerated the **guard**. They now read the column list off the **table**" — true of `pricing_plan`; `grep -rn "pragma_table_info\|information_schema.columns" tests/` returns only `sqlite_plan_guards.rs`, `postgres_schema_plan.rs` and an unrelated `sqlite_plan_repo.rs:2200`
- `tests/sqlite_plan_guards.rs:625-628` states the blindness itself: "That case pins an 18-entry array copied from the trigger, so it proves the enumerated columns are frozen and is **blind by construction to a column the enumeration omits**"

`pricing_price` is where this class has already cost four separate repair migrations (`000040`, `000051`, `000055`, `000057`), and it is the table that holds money. A 45th column added to `pricing_price` and forgotten in the whitelist changes neither the array nor the literal `44`, so both stay green while the column becomes mutable under a frozen `CatalogVersion`. The same gap stands for every other guarded table — `pricing_price_overlay`, `pricing_price_window`, `pricing_bulk_operation`, `pricing_migration`, `pricing_repricing_journal` — and the `SQLite` side of `pricing_price` has neither the table-driven census nor the per-column loop (`tests/sqlite_append_only.rs` picks two columns by hand).

I verified the whitelists are **currently complete**: `m20260802_000057` names 44 of `pricing_price`'s 46 columns on both engines (`grandfather_until` and `lifecycle_state` are the two sanctioned-mutable exemptions), and `m20260802_000062` names 23 of `pricing_plan`'s 24. So this is a durability finding, not a live hole.

*Fix/Verify:* lift `sqlite_plan_guards.rs:641` into a helper parameterised on `(table, guard_trigger, sanctioned_mutable)` and instantiate it for every table that carries a frozen-column arm, on both engines.

**PAID `7b80bef23`, lints in `04f9dfc8a`.** `pricing_price` now reads its frozen columns off the table on BOTH engines, and the `moves.len() == 45` literal is replaced by a by-name set comparison in both directions; `postgres_schema_plan` was re-pointed at the same helper. **The census was proved rather than asserted:** a scratch column was added to both engines, three reds named it, and the column was reverted — the old literal count would not have moved. Postgres was EXECUTED, not reasoned (Docker was up). Residual: five other guarded tables still rest on hand-enumerations, and the helper is now the place to add them.

**Z6-3 [Low] The overlay entity's `lifecycle_state` doc contradicts the schema it describes, and names an exit the store now refuses**

- `src/infra/storage/entity/price_overlay.rs:50-51` — "`draft` | `published` | `superseded`. Three states and no `abandoned` tombstone, which is why a discarded draft revision leaves by DELETE."
- `src/infra/storage/migrations/m20260802_000045_add_price_overlay_abandoned_state.rs:1-12` — "`abandoned` joins the overlay's lifecycle … a terminal `draft -> abandoned` flip, `DELETE` refused outright"
- same file, `:107-111` — the `DELETE` arm now raises unconditionally: "DELETE of revision % of overlay % is not permitted"
- the sibling got it right: `src/infra/storage/entity/plan.rs:72` reads "`draft` | `abandoned` | `published` | `superseded` | `retired`"

D-231 is the decision whose whole point is that a discarded overlay revision must **not** leave by `DELETE` — a re-minted revision number let a stale `If-Match` match a fresh row. The column doc still teaches the pre-D-231 model, on the type a reader of the overlay plane meets first. Same class as the already-filed F-6.

*Fix/Verify:* restate the four states and delete the `DELETE` sentence; `m20260802_000045`'s module doc is the source.

**PAID `f6d77588a`**, docs. The CONSTRAINT was re-checked as it now stands — `m20260802_000045` drops and re-adds four states on Postgres and rebuilds SQLite with the same four, and no later migration touches either — rather than reading the migration that first created it, which is how a wrong conclusion has been reached here before.

**Z6-4 [Low] The fast tier's entity↔migration agreement census stops at Slice 9 — ten entities are never read through**

- `tests/sqlite_migrations.rs:6-10` states the property: "every table can be read through its `SeaORM` entity … a migration and an entity that disagree about a column fail here rather than at the first production read"
- `tests/sqlite_migrations.rs:1251` `assert_readable!` lists **26** entities; the `use` at `:42-47` imports the same 26
- `EXPECTED_TABLES` at `tests/sqlite_migrations.rs:57-103` lists all 36 tables, so the roster knows about the ten the census skips

Never read through: `bulk_operation`, `bulk_row_lock`, `bundle`, `bundle_component`, `bundle_revshare`, `bundle_revshare_group`, `composite_meter`, `migration`, `repricing_journal`, `snapshot_provenance` — Slices 8, 10, 11 and 12, i.e. every table added after the census was written. The list is a hand-enumeration with no completeness check against `EXPECTED_TABLES`, which is the F-12 shape. Mitigated in practice: each of the ten has its own `sqlite_*_store.rs` / `*_repo.rs` suite that selects through the entity, so a column mismatch would still redden somewhere — which is why this is Low and not Medium.

*Fix/Verify:* derive the `assert_readable!` set from `EXPECTED_TABLES`, or add a case asserting the two are the same size.

**PAID `2a6cdf764`, and every number in this entry was stale.** 28 entities against 39 tables, not 26 against 36, and the unread set is ELEVEN rather than the ten named — the extra is `coord_leases`, for which no entity exists at all, recorded as a structural exemption rather than folded in. The census reddens both on a dropped entity and on a renamed column; both probes reverted.

**Z6-5 [Low] The Postgres roster covers partial indexes only, so 37 of 49 indexes — including the one `m20260802_000064` just changed — have no Postgres assertion**

- `tests/postgres_migrations.rs:259` `EXPECTED_PARTIAL_INDEXES` — 12 names; `:663` `every_declared_partial_index_reaches_the_server` is the only index assertion in the suite
- `tests/sqlite_migrations.rs:232-292` `EXPECTED_INDEXES` — all 49, asserted in both directions
- `uq_pricing_bulk_operation_client_key` is not partial, so it is in the `SQLite` roster (`tests/sqlite_migrations.rs:269`) and in **no** Postgres roster
- `tests/postgres_schema_bulk_operation.rs:309` `one_client_key_opens_one_run_per_tenant` seeds `import`/`import` and varies only the tenant — it would pass identically against the pre-D-307 `(tenant_id, client_key)` index

D-307's cross-kind admission is proven on `SQLite` (`tests/sqlite_repricing_journal_repo.rs:273` `a_second_run_under_one_client_key_is_refused_per_kind_and_not_across_kinds`) and over HTTP (`tests/rest_repricing_runs.rs:520` `one_run_id_opens_one_repricing_run_and_one_bulk_import_alike`), and both are real RED-first probes. It is proven nowhere on Postgres. That is the principle `m20260802_000023:27-29` states for itself and honours — "a measurement on one engine is not a fact about the other", for which it wrote `postgres_schema_price::two_meterless_usage_rows_on_one_key_still_collide` — applied to the sentinel index and not to this one.

*Fix/Verify:* extend `tests/postgres_migrations.rs` to roster all indexes (name + `indexdef`), and add the cross-kind admission to `postgres_schema_bulk_operation.rs`.

**PAID `e271c6b05`; the roster is 51, not 49, so the gap was 39 rather than 37.** Proved by renaming a **Postgres** `CREATE INDEX` and watching the roster redden — that engine's indexes previously had no assertion of any kind. Residual: the 51 names are duplicated per engine on the existing convention, and nothing asserts the two lists are equal.

**Z6-6 [Low] `pricing_audit_log.subject_kind` carries no CHECK while `pricing_approval.subject_kind` carries the D-158 roster**

- `src/infra/storage/migrations/m20260802_000010_create_pricing_audit_log.rs:51` / `:95` — `subject_kind text NOT NULL`, no constraint on either engine; the module doc at `:21-27` enumerates the table's guards ("Two physical guards") and is silent about it
- the three CHECKs this table does declare, per both rosters: `chk_pricing_audit_log_entry_kind`, `chk_pricing_audit_log_rollup`, `chk_pricing_audit_log_seq` (`tests/sqlite_migrations.rs:314-316`)
- the sibling: `chk_pricing_approval_subject_kind` (`m20260802_000015:150`, widened at `000019:98` and `000035:84` to the five tokens)
- `src/infra/storage/entity/approval.rs:49-50` states the rule as a rule: "`AuditSubjectKind`'s enumeration, which D-158 requires this store to spell identically and to extend in step"

Both columns hold the same vocabulary from the same Rust enum, and only one of them is held to it. `action` on the same table is likewise free-form. No live break — the writers are typed — but a hash-chained record retained seven years is the last place a token should be able to arrive unspelled, and it is the table where a wrong token is hardest to correct afterwards.

*Fix/Verify:* either add `chk_pricing_audit_log_subject_kind` over `AuditSubjectKind::ALL`, or record in `m20260802_000010`'s doc why the audit plane deliberately does not carry the constraint its approval sibling does.

**PAID `fd4fec47c`** — new migration `m20260802_000074` gives the log table's discriminator the CHECK its sibling already carried. Postgres EXECUTED, not reasoned.

**Z6-7 [Low] `pricing_migration` and `pricing_snapshot_provenance` type a plan revision `integer` where every other revision column in the chain is `bigint`**

- `m20260802_000043:143` / `:291` — `source_revision integer NOT NULL`; `m20260802_000044:97` — `source_revision integer`
- every other one: `m20260802_000001:101` `revision bigint`, `000004:201` `subject_revision bigint`, `000012:145`, `000013:156`, `000014:106`, `000025:101`, `000026:68`, `000027:60`, `000032:105`, `000033:139`, `000034:90`, `000046:69` — all `bigint`
- guarded at the boundary, which is why this is Low: `src/infra/storage/repo/migration_repo.rs:141` `let Ok(revision) = i32::try_from(new.source_revision) else { … }` and `src/infra/storage/repo/synthesis_repo.rs:107` do the same

The consequence is a fail-closed refusal rather than a truncation, so it is a modeling outlier and not a defect. Worth a line because the narrower type is invisible from the entity (`entity/migration.rs:40` `source_revision: i32`) unless a reader goes to the DDL.

*Fix/Verify:* widen both to `bigint` in a follow-up migration, or record the narrowing in `m20260802_000043`'s module doc beside the `chk_..._source_revision` it already explains.

**PAID `f65b15c0b`** — new migration `m20260802_000075`. Note this entry survived a rebuild: the migration that paid a neighbouring finding rebuilt one of these tables and restated the narrow type. Two probes, reddened with **all four sites reverted at once**. `m20260802_000075` carries the chain's only empty SQLite arm; the argument is measured and lives in its module doc.

**Z6-8 [Low] Three stale cross-references in the chain's own prose, on a chain whose numbering has already collided once**

- `m20260802_000045_add_price_overlay_abandoned_state.rs:44` — "the table is rebuilt on `m20260802_000060`'s shape", while the code comment at `:234` in the same file says "The table text is `m20260802_000032`'s". `m20260802_000060` sorts **after** `000045` and rebuilds `pricing_outbox`, a different table.
- `src/infra/storage/migrations.rs:184` — "D-158's enumeration gains `price_overlay`". The token `m20260802_000035:84` actually mints is `overlay`; `price_overlay` is a different vocabulary — `pricing_read_model` / `pricing_catalog_version_ref`'s `subject_kind` (`m20260802_000003:47`, `m20260802_000004:209`).
- `src/infra/storage/migrations.rs:254` and `:272` — two concurrent strands each declare an allotted number range "disjoint from the concurrent strand's": `000050`-`000059` and `000054`-`000059`. They overlap. `000059` is unoccupied and the chain skips it.

`migrations.rs:190-199` records that exactly this discipline failed once already — two strands both took `000036` from the same base and had to be renumbered at the merge — so the range prose is load-bearing rather than decorative.

*Fix/Verify:* point `000045:44` at `m20260802_000032`, spell the token `overlay` at `migrations.rs:184`, and reconcile the two range claims.

**Verified clean:**

- **Do the scope-key uniqueness indexes carry all ten axes of the canonical key, as they now stand?** Yes, on both engines and both planes. `m20260802_000002:344/352` created them over nine columns; `m20260802_000023:77-89` (PG) and `:119-131` (`SQLite`) drop and recreate both over eleven — the tenant plus all ten axes, with `COALESCE(meter, '')` as the null-safe sentinel. `m20260802_000023:17-29` carries the argument for the sentinel (a bare nullable column would have removed uniqueness on every non-usage key, since NULLs are distinct inside a `UNIQUE`) and it is measured on both engines rather than reasoned about. `uq_pricing_price_meter_line_current` deliberately excludes `charge_kind` and is not subsumed; `:44-52` says why.
- **Is `pricing_price`'s frozen-column whitelist complete after the whole migration sequence?** Yes. `m20260802_000057:90-133` (PG) and `:255-298` (`SQLite`) name 44 columns; the table holds 46 after `000037`/`000039`/`000050`/`000054`/`000056`; the two absentees are `grandfather_until` (monotonically tightenable by design) and `lifecycle_state` (the sanctioned flip). I counted both sides against `entity/price.rs`.
- **And `pricing_plan`'s?** Yes — `m20260802_000062` PG arm lines 33-55 name 23 columns against `entity/plan.rs`'s 24, `lifecycle_state` exempt.
- **Is the `grandfather_until` monotonicity guard armed the right way round?** Yes, at all four sites (`m20260802_000002:436-443` and `:631-641`, `m20260802_000040:124`/`:203`, `m20260802_000057:146-153`). It raises when the new value is NULL (clearing the horizon = loosening) or strictly greater than a non-NULL old one; setting a horizon where there was none passes because the `OLD … IS NOT NULL` conjunct fails. This is the F-7 shape and it is correct here.
- **Smell 9 — ordered compares on values stored as TEXT.** Every ordered compare in the zone was checked against the column's type on both backends. The only compare whose other side is the clock is `trg_pricing_price_window_future_end`, and the `SQLite` arm normalises with `datetime(...)` (`m20260802_000016:465-467`) where the PG arm uses `now()` (`:345-346`); the module doc at `:220-245` and the inline note at `:405-408` state the rule ("only arm 5, whose other side is the clock, needs `datetime(...)`"). Every other compare — `effective_to > effective_from`, `activated_at >= effective_from`, `expired_at >= effective_to`, and `pricing_migration`'s `announced_at <= effective_at` / `started_at >= created_at` / `completed_at >= started_at` / `cancelled_at >= created_at` — is column-against-column with one writer and one rendering. `m20260802_000043:280-284` asserts the premise explicitly: "All instant comparisons here are between two STORED instants, which this gear's writers render identically (RFC 3339, `+00:00`) … Nothing in this table compares against `CURRENT_TIMESTAMP`." I checked that claim against the DDL and it holds.
- **Do the two engines declare the same CHECK set?** Exactly. Extracting both rosters (`tests/postgres_migrations.rs:335` and `tests/sqlite_migrations.rs:300`) gives 142 names on each side with an empty symmetric difference. All 12 partial indexes in the Postgres roster are also in the `SQLite` index roster.
- **Does `m20260802_000063`'s `SQLite` rebuild lose the D-307 index?** No. It restates `uq_pricing_bulk_operation_client_key` with the pre-D-307 columns at `:274`, but `m20260802_000064` sorts after it under the runner's **name** ordering and corrects it (`:51-55`); the roster prose at `migrations.rs:31-34` and `tests/module_test.rs` are what keep name ordering true. The rebuild also restates both indexes, all four of the table's own triggers, and the three sibling triggers on `pricing_repricing_journal` / `pricing_bulk_row_lock` that sub-select it — dropped **before** the rename, because `ALTER TABLE … RENAME TO` re-parses the schema (`:54-63`).
- **Does the D-307 index have a query that uses it?** Yes — `bulk_repo::find_by_client_key` takes a `BulkKind` (`src/infra/storage/repo/bulk_repo.rs:194`) and both call sites pass one (`api/rest/bulk_imports.rs:276` Import, `api/rest/repricing_runs.rs:381` Repricing). The index and the query agree; the module doc's claim that either fix alone would be half of it is accurate.
- **Does `m20260802_000045`'s rebuild lose a column?** No. The `INSERT … SELECT` names all 13 columns of `pricing_price_overlay` and the rebuilt table declares the same 13, matching `entity/price_overlay.rs`; no later migration adds a column to that table. All three indexes and the four own triggers are recreated, and the six dangling child triggers are dropped first.
- **Does `chk_pricing_approval_subject_kind` still agree with `AuditSubjectKind` after two widenings?** Yes — five tokens (`plan_revision`, `price_unit`, `window`, `policy`, `overlay`) at `m20260802_000035:84`, exactly `AuditSubjectKind::ALL` (`src/domain/audit.rs:449`, `as_str` at `:460`). `pricing_read_model` and `pricing_catalog_version_ref` agree with each other on their own four-token vocabulary (`m20260802_000003:47`, `m20260802_000004:209`).
- **`price_unit` is a CHECK token with no writer for its `subject_ref` form** — documented as such at `entity/approval.rs:44-47` ("A `price_unit`'s form is **undecided**, because the kind has no writer … the slice that gives the kind a writer owes `subject_aggregate` an arm. Reported there as well"). [by-design], and the kind itself does have writers (`src/infra/supersession.rs:1004`, `src/infra/approval.rs:891`).
- **The `pricing_price` state vocabulary is narrower than the shared `LifecycleState` enum** — deliberate and argued at `m20260802_000002:202-214`: a `retired` or `abandoned` price row would fall outside both partial `UNIQUE` indexes, so the one-current-row-per-key guarantee would stop covering it. [by-design], and the right direction.
- **Are there CHECKs that fail open on NULL?** The one place it matters is handled and commented: `chk_pricing_price_package_fields_kind` carries an explicit `model_kind IS NOT NULL` conjunct because "without it a kindless row makes the whole CHECK NULL, which both engines count as satisfied" (`m20260802_000002:323-330`). `chk_pricing_price_window_expiry_order` is deliberately NULL-silent and `chk_pricing_price_window_open_ended` owns that case (`m20260802_000016:291-299`).
- **Do the index and CHECK rosters pin names rather than counts?** Yes, in both directions, and `tests/postgres_migrations.rs:22-34` explains why a count was a false economy — "a constraint replaced by `CHECK (1 = 1)` keeps a count green". This is the right discipline and it is the reason Z6-2 and Z6-5 are the only roster gaps left.
- **The zone's own unit suite.** `src/infra/storage_tests.rs` (580 lines, 21 cases) pins `repo_failure`'s classification per variant, including the two hardest pairs — `NoSuccessorRevision` vs `OpenDraftExists` (`:186`) and the mint guard vs the primary key on both backends' message renderings (`:538`, against `names_the_policy_guard` at `storage.rs:729`, which takes a `&str` precisely so both renderings are asserted without staging a race). Twenty-one of 28 variants; Z6-1 is the residue.

**Refutations:**

- *Suspected:* `uq_pricing_price_scope_key_current` enumerates only eight axes while `entity/price.rs:5` says the key has ten — the F-12 hand-enumeration shape on the load-bearing index. **Killed** by `m20260802_000023`, which widens both indexes on both engines; I had been reading `m20260802_000002` as the current state, which is exactly the "read the schema, not a migration" trap.
- *Suspected:* `pricing_repricing_journal` lacks the same-tenant-as-its-run trigger its sibling `pricing_bulk_row_lock` has (`trg_pricing_bulk_row_lock_same_tenant_as_its_run`), so a journal row can carry a foreign tenant. **Killed by sibling consistency:** every child table in this chain carries a tenant its foreign key does not verify, each says so on its entity in the same words (`entity/repricing_journal.rs:34-36`, `composite_meter.rs:37-39`, `plan_phase.rs:30-32`, `plan_addon_rule.rs:34-36`, `plan_descriptor_set.rs:34-36`, `bundle_component.rs:39-41`, `price_overlay_line_amount.rs:40-43`), and in every case the writer copies it from the parent. `bulk_row_lock` is the **outlier**, and it is the stricter one, because its parent's tenant is what the cross-tenant refusal is about. Filing the journal would have made one path stricter than the rest of the codebase.
- *Suspected:* the `SQLite` mirror's `text` timestamps make the window and migration ordering CHECKs lexicographic and therefore wrong on one engine only — the classic smell-9 trap. **Killed:** the one clock-side compare normalises with `datetime(...)`, the rest are one-writer column-against-column, and RFC 3339 with a fixed `+00:00` suffix orders lexicographically under the millisecond quantum D-144 imposes (`' '` < `'.'` < digits, so a whole-second instant sorts before a fractional one on the same second).
- *Suspected:* `m20260802_000064` (D-307) is a schema change nothing tests. **Killed:** `tests/sqlite_repricing_journal_repo.rs:273` proves both halves — one key opens one import *and* one run, and still only one of each — and `tests/rest_repricing_runs.rs:520` proves it over HTTP against the substitution the migration doc describes. Only the Postgres tier is uncovered, which is Z6-5.
- *Suspected:* `source_revision integer` silently truncates a plan revision. **Killed:** `migration_repo.rs:141` and `synthesis_repo.rs:107` both `try_from` and refuse, so the narrowing is fail-closed. Left as Z6-7 for the modeling outlier only.
- *Suspected:* `m20260802_000063`'s `SQLite` rebuild reverts the D-307 index. **Killed** by the runner's name ordering — `000063` sorts before `000064`, and the chain's ordering discipline is asserted in `tests/module_test.rs`.

**Not covered:** `src/infra/storage/repo/**` (28 files, another zone) beyond the four greps cited above as evidence; the domain and API layers; the bodies of the migrations I sampled rather than read whole — chiefly the bundle family (`000024`-`000027`), the taxonomies (`000028`-`000031`, `000042`), the approval family's `SQLite` rebuilds (`000019`, `000035`), the overlay line tables (`000033`, `000034`) and the guard restatements `000040`/`000051`/`000055`, for which I relied on the CHECK/trigger/index rosters and the two engines' roster parity rather than on line-by-line reading. Nothing was executed: I read the Postgres suites, I did not run them, so every claim about Postgres behaviour here is a claim about the DDL text and the roster, not about the server. Per instruction I ran no cargo command.


---

**Zone Z7 — catalog/money repositories**

**Files:** `gears/bss/pricing/pricing/src/infra/storage/repo/price_repo.rs` (3679, read whole), `plan_repo.rs` (2255, read whole), `plan_shape_repo.rs` (1767, read whole), `catalog_version_ref_repo.rs` (852, read whole), `taxonomy_repo.rs` (939, read whole). Supporting evidence read, not reviewed: `entity/price.rs`, `migrations/m20260802_000002_create_pricing_price.rs`, `m20260802_000023_widen_pricing_price_scope_key_indexes.rs`, `m20260802_000036_widen_pricing_catalog_version_ref_subject_key.rs`, `libs/toolkit-db/src/secure/{cond.rs,select.rs,db_ops.rs}`, `tests/{sqlite_price_repo.rs,sqlite_plan_repo.rs,sqlite_taxonomy_repo.rs,sqlite_plan_phase.rs,sqlite_plan_addon_rule.rs,sqlite_plan_descriptor_set.rs,sqlite_history.rs}` (case inventories + the three cases named below in full).

**What's done:** All five repositories are built out and mounted. Every read and every mutating statement in the zone goes through `SecureORM`'s `.secure().scope_with(scope)`, which I verified emits a SQL predicate rather than a Rust-side decision (`libs/toolkit-db/src/secure/select.rs:167-175`, `db_ops.rs:739-746`, `db_ops.rs:837-843`, `cond.rs:54-83`). The compare-and-swap shape is uniform across `price_repo`, `plan_repo` and `plan_shape_repo` — bump inside the matching UPDATE, `rows_affected == 0` resolved into one of three named refusals by a re-read. D-196's ten-axis key is carried consistently through `scope_key_filter`, `scope_key_columns`, `to_scope_key` and the two partial `UNIQUE`s (with the `COALESCE(meter,'')` NULL trap closed on both planes). Publish, supersession and cutover row halves are transaction-typed (`&DbTx`) rather than runner-typed, which is what makes their ordering arguments hold.

**Verdict:** One High, two Medium, seven Low. The High is a silently unwritable money-relevant content column on the only draft-edit path in the gear; it is not reachable across tenants and does not misprice today, which is why it is not Critical — it arms one the day the Tax Engine goes GA. Nothing in this zone leaks across a tenant boundary, and I found no unguarded ordered compare on a TEXT timestamp.

**Findings**

**PAID `48db2de8e`**, docs. All three cross-references verified against their current targets before rewriting.

**Z7-1 [HIGH] `update_draft` cannot write `tax_category_ref` — the column is absent from the content-assignment list, on both live edit paths**
**PAID `4eebef3c4`.** See the status ledger at the top of this document.

`gears/bss/pricing/pricing/src/infra/storage/repo/price_repo.rs:2661` — `content_model` renders it: `tax_category_ref: Set(content.tax_category_ref.clone())`
`gears/bss/pricing/pricing/src/infra/storage/repo/price_repo.rs:3054-3177` — `content_assignments` enumerates **30** columns; `price::Column::TaxCategoryRef` is not one of them (I diffed it component-by-component against `content_model`'s 31 `Set`s: `amount_minor, model_kind, tax_inclusive, billing_timing, billing_anchor_policy, anchor_day, proration_basis, credit_on_downgrade, quantity_source, manual_quantity, package_size, package_price_minor, meter, dimension_key, billing_granularity, aggregation_function, aggregation_granularity, tier_aggregation_window, tier_qualification_window, max_hold_granules, included_allowance, reserved_rate_minor, reservation_flavor, min_qty_purchase, min_qty_usage, min_qty_usage_fallback, discount_ref, rounding_policy_ref, grandfather_until, supersedes_price_id` — exactly one gap)
`grep -rn "TaxCategoryRef" src/` returns **one** hit in the whole crate, `taxonomy_repo.rs:416`, a read. No writer anywhere but `content_model` on the insert path.
`gears/bss/pricing/pricing/src/infra/storage/repo/price_repo.rs:639-647` — `update_draft` builds its UPDATE from exactly that list
`gears/bss/pricing/pricing/src/api/rest/prices.rs:1163-1177` — `content_of` parses `taxCategoryRef` off the wire and even refuses a blank one by name
`gears/bss/pricing/pricing/src/api/rest/prices.rs:771-788` — `PATCH /plans/{planId}/prices/{priceId}` hands that content to `update_draft`
`gears/bss/pricing/pricing/src/infra/bulk.rs:255-267` — the bulk import's edit arm does the same
`gears/bss/pricing/pricing/tests/sqlite_price_repo.rs:1422-1443` — the case named `an_update_rewrites_every_content_column_and_can_clear_one`, whose own comment says *"Every content column this kind can carry moves at once"*, never sets and never asserts `tax_category_ref`; `graduated_content()` leaves it `None` (`tests/sqlite_price_repo.rs:222`) and it stays `None`

`content_model` writes it, `to_record` reads it back (`price_repo.rs:3295`), `PriceContent` carries it, the wire carries it — and the one path that edits a draft drops it. A `PATCH` that sets or clears `taxCategoryRef` returns **200** with a body rendered from the stored record (`api/rest/prices.rs:385`), so the field silently reverts. D-110 makes this column the source of truth and the only place a category lives; D-154 freezes `coalesce(tax_category_ref, readiness.taxCategory)` inside the publish transaction (`price_repo.rs:1009-1019`) into a version that is INSERT-only over the seven-year horizon. So a correction made on a draft never lands, and the row publishes and freezes the category the author already knows is wrong. `04-currency-tax.md:198` names *"the operator … authors `taxCategoryRef` onto the dependent rows"* as D-245's remedy; on a draft row that remedy is unexpressible.

Two other columns in the list — `Meter` and `DimensionKey` — are write-no-ops because `check_update_keeps_the_line` (`price_repo.rs:2520-2536`) refuses any move first; they are in the list and documented. `tax_category_ref` is the only column that is neither written nor refused nor documented as frozen, which is what makes this an omission rather than a decision: `charge_kind` and the usage line each get a paragraph in `update_draft`'s doc explaining why they are not moved (`price_repo.rs:541-553`, `616-632`), and this one gets none.

*Fix/Verify:* add `(price::Column::TaxCategoryRef, model.tax_category_ref.clone().into_value())` to `content_assignments`, and arm `an_update_rewrites_every_content_column_and_can_clear_one` against it in both directions (set on a row created without one; clear on a row created with one). The general guard is worth more than the one line: `content_assignments` is a hand-written list over an `ActiveModel` whose fields the compiler will not check it against — an exhaustive destructure of the `price::ActiveModel` returned by `content_model` would make the next omission a compile error rather than a silent no-op.

**Z7-2 [MEDIUM] The taxonomy retire guard counts published rows through a hand-written `"published"` literal, so a rename silently retires a value in use**

`gears/bss/pricing/pricing/src/infra/storage/repo/taxonomy_repo.rs:92` — `const PUBLISHED: &str = "published";`
`gears/bss/pricing/pricing/src/infra/storage/repo/taxonomy_repo.rs:95` — `const ACTIVE: &str = "active";`
`gears/bss/pricing/pricing/src/infra/storage/repo/taxonomy_repo.rs:362` — `.add(price::Column::LifecycleState.eq(PUBLISHED))`
`gears/bss/pricing/pricing/src/infra/storage/repo/taxonomy_repo.rs:379` — `.add(price_overlay::Column::LifecycleState.eq(PUBLISHED))`
`gears/bss/pricing/pricing/src/infra/storage/repo/taxonomy_repo.rs:415` — same, in `rows_resolving_category_through`
`gears/bss/pricing/pricing/src/infra/storage/repo/taxonomy_repo.rs:241,286,316` — `.add(region_taxonomy::Column::State.eq(ACTIVE))`
`gears/bss/pricing/pricing/src/domain/lifecycle.rs:89-95` — `LifecycleState::as_str` is the single renderer, `Published => "published"`
`gears/bss/pricing/pricing/src/domain/taxonomy.rs:221-226` — `TaxonomyState::as_str`, `Active => "active"`
`grep -rn '"published"|"draft"|"active"|"retired"|"superseded"|"abandoned"' src/infra/storage/repo/*.rs` — outside test modules the only hits are `taxonomy_repo.rs:92,95` and `overlay_repo.rs:834,1706,1715,1749`. Every other repository in the layer — `price_repo.rs:1740,1916,2450,2590`, `plan_repo.rs:1169,1223,1299,1516,2146,2153`, `plan_shape_repo`, `approval_repo`, `window_repo`, `bulk_repo` — compares `LifecycleState::X.as_str()`.

This file imports the domain (`use crate::domain::taxonomy::{... TaxonomyState ...}` at line 75, and uses `TaxonomyState::Retired` at 474/475) yet writes the token by hand for the two comparisons that decide the guard. The failure direction is the bad one: `references_to` is the **refusal**, so a spelling that stopped matching returns 0 for both counts, `check_retirable` finds nothing to report, and a region a published price row names is retired cleanly — with `tests/sqlite_taxonomy_repo.rs::a_published_price_row_blocks_its_regions_retirement` and `::a_published_overlay_scope_blocks_the_retirement_in_every_class` both still green, because the fixtures write their rows through the same drifted renderer. The `ACTIVE` half fails loud instead (an empty region universe refuses every publish), so it is the weaker half of the same defect.

`overlay_repo.rs:1749` is another instance of this class in a neighbouring zone: a bare `"published"` string literal against `price::Column::LifecycleState`, not even behind a named constant.

*Fix/Verify:* `LifecycleState::Published.as_str()` and `TaxonomyState::Active.as_str()` at all six sites; delete both constants. A cheap standing probe: assert `PUBLISHED == LifecycleState::Published.as_str()` — but the honest fix is to remove the second spelling rather than to test it.

**PARTIAL → PAID `828784bcd`.** The `taxonomy_repo` module was paid by `8af192e10`; the surviving `overlay_repo` literals are now rendered through the enum. See Z13-1 for the full site list, which turned out to be twelve rather than the three this plan's ledger recorded.

**Z7-3 [MEDIUM] `overlay_revisions_at_or_below` folds "later row wins" with no tie-break, so two publishes of one overlay batched into one `CatalogVersion` freeze an arbitrary revision into the index shard**

`gears/bss/pricing/pricing/src/infra/storage/repo/catalog_version_ref_repo.rs:695-697` — `// Ascending, so the fold below keeps the **greatest** ref of each overlay by simply letting later rows win.` then `.order_by(catalog_version_ref::Column::CatalogVersion, Order::Asc)` and nothing else
`gears/bss/pricing/pricing/src/infra/storage/repo/catalog_version_ref_repo.rs:702-719` — `for row in rows { … latest.insert(price_overlay_id, revision); }`
`gears/bss/pricing/pricing/src/infra/storage/repo/catalog_version_ref_repo.rs:640-646` — the doc's own rule: *"**The greatest ref, not any ref.** An overlay that published revisions 0 and 1 has two refs, and at a `V` between them the shard must speak of revision 0"*
`gears/bss/pricing/pricing/src/infra/storage/repo/catalog_version_ref_repo.rs:634-637` — the premise that makes the tie reachable: the registry batches at D-47's five-minute maximum

The ordering is total on `catalog_version` only. Two publishes of the **same** overlay are two handles and two rows, and D-47 batching is exactly the mechanism that can assign them one version — at which point the two rows tie, SQL leaves their relative order undefined, and whichever the store returns last decides which `subject_revision` the frozen `overlay_index` shard names. The consequence is the one this function exists to prevent, stated in its own doc: a consumer pinned at `V` enumerates the overlay at a revision the version did not describe, permanently, because a delta is INSERT-only over the seven-year horizon. Compare the sibling `list_pending_for_tenant` (`catalog_version_ref_repo.rs:579-580`), which adds `.order_by(PendingRef, Asc)` precisely so its `requested_at` order is total — the tie-break discipline exists in this file and was not carried here.

*Fix/Verify:* add `.order_by(catalog_version_ref::Column::SubjectRevision, Order::Asc)` after the version, so the fold's "later wins" is the greatest revision by construction. A probe: two committed `price_overlay` refs of one overlay id at one `catalog_version` with revisions 0 and 1, asserting the map answers 1.

**PAID `4b130a440`.** The fold takes the greatest ref, following the tie-break discipline already present in the same file. The probe batches two publishes of ONE overlay into ONE catalog version in *both* insertion orders and asserts which wins — a probe that resolved a single overlay would have proved nothing. RED: `left: Some(0) / right: Some(1)`.

**Z7-4 [LOW] `replace_composites_on` takes its child rows' `tenant_id` and `plan_id` from the request, where its three siblings take them from the parent revision row**

`gears/bss/pricing/pricing/src/infra/storage/repo/plan_shape_repo.rs:31-38` — the module's stated rule: *"A child row's `tenant_id` comes from the parent revision, never from the request (Global Constraint 9). The foreign key covers `(plan_id, plan_revision)` alone, so nothing in the schema stops a phase row carrying a different tenant …"*
`gears/bss/pricing/pricing/src/infra/storage/repo/plan_shape_repo.rs:1233-1234` — `phase_models`: `tenant_id: Set(parent.tenant_id), plan_id: Set(parent.plan_id)`
`gears/bss/pricing/pricing/src/infra/storage/repo/plan_shape_repo.rs:1299-1302` — `addon_rule_models`: same
`gears/bss/pricing/pricing/src/infra/storage/repo/plan_shape_repo.rs:1441-1443` — `descriptor_model`: same
`gears/bss/pricing/pricing/src/infra/storage/repo/plan_shape_repo.rs:1742-1747` — `replace_composites_on` **discards** the row `mutable_draft` hands back (`if mutable_draft(…).await?.is_none()`), unlike the three siblings which bind it as `parent`
`gears/bss/pricing/pricing/src/infra/storage/repo/plan_shape_repo.rs:823-826` — `write_composites`: `tenant_id: Set(tenant_id), plan_id: Set(plan_id.get())`, both the caller's arguments

Not exploitable today, and I checked why rather than assuming: `mutable_draft` → `load_revision_row` (`plan_repo.rs:1056-1063`) filters `TenantId.eq(tenant_id)` and `PlanId.eq(plan_id)`, so the resolved parent necessarily carries the values the arguments named, and `scope_with_model` then validates the rendered tenant against the caller's scope. So this is the stated invariant being satisfied by accident on one of four paths. It is worth recording because the module doc treats the rule as load-bearing and because `write_composites` is `pub(super)` — a second caller that resolves the revision by some other predicate would make the divergence real, and no test could tell.

*Fix/Verify:* bind the parent (`let Some(parent) = mutable_draft(…)`) and give `write_composites` a `parent: &plan::Model` the way the three sibling model-builders take one.

**PAID `235642ebc` — TRUE, BUT NO PROBE CAN BE ARMED AT ITS CLAIM, which was established by measurement rather than by reading.** A request naming a different tenant is refused `NotFound` BEFORE any child row is rendered (`mutable_draft` → `load_revision_row` selects on the same two values), so the probe this finding implies passes identically with and without the fix. The fix is therefore structural and the gate is the compiler: proved with a scratch caller that fails `expected &Model, found Uuid`. Also this entry's premise about a diverging second call site is wrong — `write_composites` has exactly one caller.

**Z7-5 [LOW] `publish_rows`' entire doc block — including its `# Errors` — is attached to `resolved_tax_categories`; `publish_rows` carries none**

`gears/bss/pricing/pricing/src/infra/storage/repo/price_repo.rs:817-918` — the argument for why `publish_rows` takes `validated` as an argument (the READ COMMITTED window, D-141, the supersession-ordering debt, the `# Errors` list)
`gears/bss/pricing/pricing/src/infra/storage/repo/price_repo.rs:919-927` — continues, with no blank line, into `/// Each row's **frozen** resolved tax category, by `price_id` (D-154).` and a second `# Errors`
`gears/bss/pricing/pricing/src/infra/storage/repo/price_repo.rs:928` — `pub async fn resolved_tax_categories(` — the item the whole 817-927 block binds to
`gears/bss/pricing/pricing/src/infra/storage/repo/price_repo.rs:950` — `pub async fn publish_rows(` — no doc comment at all

Rendered docs put `publish_rows`' normative contract on a two-line read helper, and give that helper an `# Errors` list naming `NotFound`, `NotDraft` and `StaleRowVersion`, none of which it can produce. A reader greping for why the publish path may not re-derive its own row set lands on the wrong function; a reader of `publish_rows` finds nothing. Same class as F-6 (stale module doc) and F-8 (error doc names a non-producer), in the zone's most load-bearing function.

*Fix/Verify:* move `resolved_tax_categories` and its own doc out of the middle of the block. The general guard: this is what a `#![warn(missing_docs)]` on `infra` would have caught, but `lib.rs:27` marks the module `#[doc(hidden)]`.

**PAID `a29525ae2`**, docs — the doc block and its `# Errors` moved to the function they describe.

**Z7-6 [LOW] `resolved_tax_categories` has no lifecycle filter, and its doc claims the absence that filter would have produced**

`gears/bss/pricing/pricing/src/infra/storage/repo/price_repo.rs:923-924` — *"`None` in the map is a row that has one and it is null; a row absent from the map has not published."*
`gears/bss/pricing/pricing/src/infra/storage/repo/price_repo.rs:934-947` — the query filters `TenantId` and `PlanId` and nothing else, then maps every row to `(price_id, resolved_tax_category)`

Every draft row of the plan is therefore *in* the map with `None`, which the doc says means "published with a null category". Harmless at the one call site — `infra/read_model.rs:914-935` looks the map up only at `price_id`s drawn from `load_for_plan(… PROJECTED_ROW_STATES)` — so this is an audit gap rather than a live defect, and it is priced Low for exactly that reason. It becomes real the moment a second caller reads the map's key set as "the published rows".

*Fix/Verify:* either add `LifecycleState.is_in(PROJECTED_ROW_STATES)` to the query or correct the sentence. The filter is the better half — it also stops a draft's NULL being carried into a projection.

**PAID `d4f852f26`**, with a RED probe. The constraint was re-checked as it now stands (`draft|published|superseded`, unwidened), which decided the shape of the fix: the filter is `PROJECTED_ROW_STATES`, not `eq(Published)` — the probe fails against the narrower form, so the narrower form is not merely stricter, it is wrong.

**Z7-7 [LOW] `replace_composites`' `# Errors` names `RepoError::LifecycleForbidden`, a variant that does not exist**

`gears/bss/pricing/pricing/src/infra/storage/repo/plan_shape_repo.rs:218-220` — *"[`RepoError::StaleRowVersion`] / [`RepoError::LifecycleForbidden`] / [`RepoError::NotFound`] as the compare-and-swap resolves them"*
`gears/bss/pricing/pricing/src/infra/storage.rs:39-411` — the `RepoError` enum: `Db, CorruptRow, FrontierRegression, NotFound, StaleRowVersion, NotDraft, NotSupersedable, NoSuccessorRevision, OpenDraftExists, IdempotencyPayloadMismatch, DuplicateScopeKey, BulkRowLocked, OverlayOpenDraftExists, DuplicateBundleOnPlan, ConcurrentMutation, UsageLineDisagrees, GrandfatherHorizonOffClass, ValueOutOfRange, TimestampPrecisionExceeded, IdempotencyKeyInFlight, ApprovalNotPending`. No `LifecycleForbidden`.
`gears/bss/pricing/pricing/src/infra/storage/repo/plan_repo.rs:1397-1401` — what the shared `refuse` actually answers for a frozen revision: `RepoError::NotDraft`

`LifecycleForbidden` exists only on `DomainError` (`grep -rn "LifecycleForbidden" src/` — every other hit is `DomainError::`). This is a broken intra-doc link that survives because `infra` is `#[doc(hidden)]`, and the sibling `replace_phases`/`replace_addon_rules`/`set_descriptor_set` docs (lines 135-144, 296-307, 391-396) all name `NotDraft` correctly. Another instance of F-8's class.

*Fix/Verify:* `RepoError::NotDraft`; the three siblings are the reference text.

**PAID `a29525ae2`**, docs — the named `RepoError` variant does not exist and the contract now names what the function can actually return.

**Z7-8 [LOW] `record_pending`'s `# Errors` still describes the two-column primary key `m20260802_000036` widened**

`gears/bss/pricing/pricing/src/infra/storage/repo/catalog_version_ref_repo.rs:224-229` — *"which **includes** a second record of one `(tenant_id, pending_ref)`, refused by the primary key"*
`gears/bss/pricing/pricing/src/infra/storage/migrations/m20260802_000036_widen_pricing_catalog_version_ref_subject_key.rs:79` — `ADD PRIMARY KEY (tenant_id, pending_ref, subject_kind, subject_ref)`
`gears/bss/pricing/pricing/src/infra/storage/repo/catalog_version_ref_repo.rs:265-268` — the same file, elsewhere, states the corrected shape: *"a handle names one assignment and an overlay publish unit records two or three subjects against it"*

A reader of the error contract is told that a handle may be recorded once, when the whole point of D-234 is that one handle is recorded once **per subject**. The migration doc says the protection survives at the narrower granularity, so this is a doc-vs-schema drift, not a behaviour defect — but it is the shape "read the schema, not a migration": the sentence describes a constraint a later migration dropped and recreated.

*Fix/Verify:* restate as `(tenant_id, pending_ref, subject_kind, subject_ref)` and cite `m20260802_000036`.

**PAID `a29525ae2`**, docs — the `# Errors` described a two-column primary key a migration had widened.

**Z7-9 [LOW, forward-dependency] `observe_commit` identifies a ref by two of the four columns `RefIdentity` exists to keep together**

`gears/bss/pricing/pricing/src/infra/storage/repo/catalog_version_ref_repo.rs:292-299` — `RefIdentity`'s doc: *"One value rather than four parameters, and that is the point … A caller cannot supply three of the four."*
`gears/bss/pricing/pricing/src/infra/storage/repo/catalog_version_ref_repo.rs:386-391` — `observe_commit` filters `TenantId`, `PendingRef` and `CommitObservedAt IS NULL`, taking neither subject column and not using `RefIdentity` at all
`gears/bss/pricing/pricing/src/infra/storage/repo/catalog_version_ref_repo.rs:426-453` — `finalize`, the sibling, is per-subject through `id.condition()`

Arguably correct — the registry answers a *handle*, so stamping every subject row of that handle at one instant is the honest observation, and `update_many` with no `.exec` row-count assertion is consistent with the doc's "an absent row is not an error". What is missing is the sentence: the module's D-234 paragraph (`:405-416`) argues only why the *finalize* is per subject, so the next reader meets one function of three that spells the identity differently with nothing saying why. Recorded rather than left, because `RefIdentity` was introduced specifically to make partial identities unwritable and this call site is the exception it does not cover.

*Fix/Verify:* one paragraph on `observe_commit` stating that the observation is handle-scoped by design, or a `HandleIdentity` type so the two granularities are both named.

**PAID `a29525ae2` — and this entry's forward-dependency premise DOES NOT HOLD.** `m20260802_000071` is `ALTER TABLE … ADD COLUMN` and leaves the primary key alone, so the four columns still identify a row and the new pin columns are payload rather than identity. Recorded as prose rather than rebuilt.

**Z7-10 [LOW, forward-dependency] `delete_draft` hard-codes `on_behalf_of: None`, so a bulk run can never delete a row it holds itself**

`gears/bss/pricing/pricing/src/infra/storage/repo/price_repo.rs:721` — `refuse_if_locked_elsewhere(txn, &scope, tenant_id, price_id, None).await?;`
`gears/bss/pricing/pricing/src/infra/storage/repo/price_repo.rs:772-804` — the guard's own doc: *"**The distinction is load-bearing and easy to miss**: Phase 2 edits the very rows it locked, so a guard that refused every locked row would make the commit refuse its own batch. What the lock excludes is *somebody else*."*
`gears/bss/pricing/pricing/src/infra/storage/repo/price_repo.rs:606` — `update_draft` threads the caller's `on_behalf_of` through, correctly
`grep -n "delete_draft" src/infra/bulk.rs` — no hits; `commit_rows` (`infra/bulk.rs:230-290`) has an edit arm and a create arm and no delete arm

Fail-closed behind an absent lane, which is the shape worth naming: the import has no delete verb today, so the hard-coded `None` refuses nothing that exists. The day one lands, a run's own delete of its own locked row is refused `BULK_ROW_LOCKED` naming the run itself — with every current test green, because no test can hold a rule against a verb that does not exist.

*Fix/Verify:* give `delete_draft` the same `on_behalf_of: Option<Uuid>` parameter `update_draft` has, defaulted `None` at the two interactive call sites. That is a signature change across the storage suites and changes no behaviour today, which is the argument for doing it now rather than after the lane lands.

**Verified clean:**

- **Tenant predicate in the SQL, not in a caller's decision.** Every read/update/delete in the zone is `.secure().scope_with(scope)`, and I traced that to `inner.filter(build_scope_condition::<E>(scope))` in all three builders (`libs/toolkit-db/src/secure/select.rs:167-175`, `db_ops.rs:739-746`, `db_ops.rs:837-843`), with `build_scope_condition` failing closed to `deny_all()` on an unresolvable property (`cond.rs:72-74, 88-98`). Inserts validate the model's tenant in Rust instead (`db_ops.rs:396-406`), which is why `band_models`/`phase_models`/`addon_rule_models` copy the tenant off the parent — the one path where the SQL cannot carry it. The two deliberately tenant-less reads, `price_repo::load_plan_ids` (`price_repo.rs:1992-2008`) and `price_repo::gated_markets` (`price_repo.rs:1734-1766`), are cross-tenant by contract, documented as such, and rely on the scope alone — correct, since both are keyed on a primary key or aggregate the tenant back out.
- **The scope key is complete in every WHERE.** `scope_key_filter` (`price_repo.rs:2599-2622`), `scope_key_columns` (`price_repo.rs:1409-1422`) and `to_scope_key` (`price_repo.rs:3313-3357`) each enumerate all ten axes; `market_columns` (`price_repo.rs:1377-1388`) enumerates eight and names the two it drops, matching `inst-co-copy`. The `meter IS NULL` arm at `price_repo.rs:2617-2620` closes the `Column::Meter.eq(None)` trap that would have read an occupied key as free, and mirrors the index's `COALESCE(meter,'')` (`m20260802_000023:77-89`). This is the axis-enumeration class F-12 was filed for and it is discharged here.
- **Ordered compare on TEXT.** The only ordered datetime compare in the zone is the history keyset, `price_repo.rs:1631-1639` + `1645-1646`, over a column that is `timestamptz` on Postgres (`m20260802_000002:275`) and `text` on SQLite (`m20260802_000002:503`). I checked the encoder rather than assuming: sqlx 0.8.6 writes `DateTime<Tz>` as `to_rfc3339_opts(SecondsFormat::AutoSi, false)` (`sqlx-sqlite-0.8.6/src/types/chrono.rs:64-70`), i.e. fixed-width UTC with a `+00:00` offset and 0/3/6/9 fractional digits. Lexicographic order equals chronological order under that format at every width boundary, because `'+' (0x2B) < '.' (0x2E) < '0'`. The composite predicate is also written out correctly (`>` on the instant OR `=` plus `>` on the id), so a tie neither skips nor repeats — pinned by `tests/sqlite_history.rs::a_one_row_walk_visits_every_row_exactly_once_across_a_tie`. The residual is named under **Refutations**.
- **Cursor and page bounds.** `list_history_page` has no unbounded form and says why (`price_repo.rs:497-501`); `list_for_plan_page` is keyset over the `price_id ASC` total order the sibling's contract already promises (`price_repo.rs:415-455`, `1577-1611`); `pending_tenants` is a bounded loose index scan (`catalog_version_ref_repo.rs:510-539`); `list_pending_for_tenant` bounds per tenant and makes the bound a fact the caller must act on (`catalog_version_ref_repo.rs:550-553`). The two unbounded reads that remain both declare themselves: `list_for_plan` (bounded by §14's 500-row soft cap and used by the publish assembler) and `load_published_for_selector` (`price_repo.rs:1895-1901`, *"There is no page bound, and that is a real edge"*, reached live from `api/rest/repricing_runs.rs:402`). Declared, so [by-design]; the second is the one I would revisit first if §5 ever ratifies a cap.
- **Locks and transactions.** All five mutating entry points in `price_repo`/`plan_repo`/`plan_shape_repo` run their whole body inside `in_transaction`, so the three abnormal exits collapse to one: `Err`, panic and future-drop all abandon the transaction without committing, and there is no lock taken outside it and no `Err`-arm-only cleanup anywhere in the zone. The two orderings that are forced rather than chosen — bands before the row's swap (`price_repo.rs:634`), child deletes before the abandon flip (`plan_repo.rs:468-480`) — each carry the trigger argument that forces them, and `publish_revision` takes `&DbTx` rather than a runner precisely so the demote-then-publish pair cannot be split (`plan_repo.rs:1094-1103`).
- **Idempotency / dedup keys.** `band_id = Uuid::new_v5(price_id, from_qty)` (`price_repo.rs:3218-3220`) is derived from the table's real identity `UNIQUE (price_id, from_qty)`, so the surrogate and the natural key collide together. `finalize`'s zero-rows case is decomposed rather than swallowed — same version is an idempotent `Ok`, a different version is `CorruptRow` (`catalog_version_ref_repo.rs:454-481`) — which is the replay-without-lifecycle-filter trap answered correctly. `observe_commit` is write-once on `IS NULL` (`catalog_version_ref_repo.rs:390`) for the stated reason that a resetting stamp would zero the degraded clock forever.
- **Contract-richer-than-implementation.** `PRICE_ELIGIBILITIES` carries all three normative classes rather than the two the cutover machinery uses, with the reason written down (`price_repo.rs:161-167`) — the inverse of the defect. `PRICE_OVERLAYS` carries one value and exists so a stored `partner` is refused rather than silently read as `base` (`price_repo.rs:157-159`), and `to_scope_key` asks it even though `ScopeKey` takes no overlay (`price_repo.rs:3318-3326`).
- **Dead fields.** I grepped writers and readers for the columns that looked suspicious. `resolved_tax_category`: written only by `publish_rows` (`price_repo.rs:1035-1038`), read by `resolved_tax_categories` → `infra/read_model.rs:914` — live both ways. `supersedes_price_id`: written by `insert_successor_draft_on` and by ordinary content, read by `refuse_mispaired` (`price_repo.rs:1203`) and the D-127 guard — live. `commit_observed_at`: written by `observe_commit`, read into `PendingVersionRow` and consumed by the degraded signal — live. `cloned_from` on the plan: written at create, carried forward at `open_revision` (`plan_repo.rs:716`), asserted by `sqlite_plan_repo.rs::lineage_round_trips_and_survives_the_next_revision`.
- **Test coverage of the zone.** Strong and mostly adversarial: the three-way refusal is pinned per repo (`a_frozen_row_refuses_by_name_and_an_absent_one_is_not_found`, `frozen_beats_stale_when_a_write_is_both`), cross-tenant invisibility is pinned per table (`another_tenants_price_row_is_invisible_and_unwritable`, `a_price_row_may_not_be_created_into_another_tenant`, and a sibling in each of the four shape suites), rollback-on-audit-failure is pinned three ways in `sqlite_plan_repo.rs:2790-3056`, and the closed-set guard `the_revision_scoped_tables_are_a_closed_set_and_each_one_is_copied_and_dropped` (`sqlite_plan_repo.rs:2165`) is the right shape for a rule the compiler cannot hold. The one test-smell I found is Z7-1's: a case that names itself exhaustive over content columns and is not.

**Refutations:**

- **The taxonomy `PUBLISHED` literal is not a lone outlier.** I expected `taxonomy_repo` to be the only file spelling a lifecycle token by hand and checked before pricing it; `overlay_repo.rs:834,1706,1715,1749` does the same. That does not save either — the rest of the repo layer uses `LifecycleState::as_str()` and the guard direction is silent — but it moves the finding from "one file drifted" to "two files share one drift", and the overlay instance belongs to another zone, so I record it rather than re-file it.
- **The meter-line index is not an unnamed 500.** `uq_pricing_price_meter_line_current` (`m20260802_000002:362`) is not consulted by `find_key_occupant`, so I expected two draft rows sharing a `(meter, dimensionKey)` line across two charge kinds to reach `publish_rows` and die as a raw driver error. They do not: `MeterInjectivity` is a registered publish rule (`src/domain/plan_rules.rs:409`, `src/domain/plan_rules/composition.rs:158-176`) and reports `inst-cmp-injective` before the flip. Refuted.
- **`refuse_mispaired` returning `Ok` on a missing row is not a hole.** It looked like a guard that gives up, but the two moves it precedes each produce a precise refusal for their own row — `supersede_row` → `refuse_unsupersedable` (`price_repo.rs:1459`), `publish_rows` → `stands_at(None, …) == false` → `refuse` → `NotFound` (`price_repo.rs:978-982`, `1517-1528`) — and both run inside the caller's `&DbTx`, so a predecessor flipped before a successor is found missing rolls back. The doc at `price_repo.rs:1186-1191` states exactly this and it holds.
- **`patched_columns`' "absent means don't touch" is not a lost-clear.** `stored_qty(…, patch.purchase_min_qty)?` returning `Ok(None)` looked like it might swallow a deliberate clear, but every arm of `patched_columns` (`plan_repo.rs:1529-1603`) treats `None` as unstated, `PlanShapePatch` carries no `Option<Option<T>>` anywhere, and `PriceRepo::update_draft` takes whole content rather than a patch precisely so clearing is expressible (`tests/sqlite_price_repo.rs:1436-1441` carries the argument). Two doors, two deliberate encodings, not a drift.
- **The SQLite `created_at_utc` default is not reachable from this gear.** `text NOT NULL DEFAULT (CURRENT_TIMESTAMP)` (`m20260802_000002:503`) renders `YYYY-MM-DD HH:MM:SS` with a space separator, which sorts before every sqlx-written `…T…+00:00` value regardless of date — so a row written without the column would sit permanently at the head of the history walk. `insert_model` always `Set`s it (`price_repo.rs:3038`) and `NewPriceDraft` requires it, so no path in the gear can leave it to the default; the raw `INSERT INTO pricing_price` statements in the tree are all in schema-guard suites (`tests/sqlite_append_only.rs`, `sqlite_price_checks.rs`, `sqlite_window_guards.rs`, `sqlite_tier_band_guard.rs`) and none of them feed the history reader. Dormant, and worth remembering only if a future writer ever omits the column.

**Not covered:** the tests are read as evidence and are not reviewed as a zone — I inventoried every case name in the six zone suites and read `an_update_rewrites_every_content_column_and_can_clear_one`, `publish_freezes_the_rows_own_category_over_the_region_default` and the `sqlite_history.rs` cursor cases in full, and did not audit the rest line by line. I did not review `overlay_repo.rs`, `window_repo.rs`, `approval_repo.rs`, `bulk_repo.rs`, `audit_repo.rs`, `outbox_repo.rs`, `read_model_repo.rs` or the other repositories outside Z7, nor `infra::publish` / `infra::supersession` / `infra::cutover` / `infra::bulk` beyond the call sites cited above. The Postgres schema suites (`postgres_schema_price.rs`, `postgres_schema_plan*.rs`, `postgres_schema_taxonomy.rs`) were not read; I read the Postgres DDL directly instead. I ran no cargo command of any kind — the tree is read-only for this pass.


---

**Zone Z8 — governance/lifecycle repositories**

**Files:** all of `gears/bss/pricing/pricing/src/infra/storage/repo/` except the five owned elsewhere — `approval_repo.rs` (+`approval_repo_tests.rs`), `audit_repo.rs`, `bulk_repo.rs`, `bundle_repo.rs`, `idempotency_repo.rs`, `migration_repo.rs`, `outbox_repo.rs`, `overlay_repo.rs`, `pin_frontier_repo.rs`, `policy_repo.rs`, `read_model_repo.rs`, `repricing_journal_repo.rs`, `synthesis_repo.rs`, `threshold_repo.rs`, `window_repo.rs`. Read as sibling evidence, not reviewed: `plan_repo::read_token`, `infra/storage.rs`'s `contention_or_db`/`policy_guard_or_contention`, `infra/approval.rs`, `infra/window.rs`, `infra/bulk.rs`, `infra/overlay_publish.rs`, `infra/bundle.rs`, `api/rest/{bundles,bulk_imports,migrations}.rs`, migrations `000009/000010/000015/000017/000032/000043/000045/000047/000060`, `domain/events.rs`.

**What's done:** every file read whole (approval, outbox, overlay, window, bulk, idempotency, repricing-journal, pin-frontier, threshold, synthesis, migration, audit) or read at every write/read site plus its doc (bundle, policy, read_model). Each mutating path traced to its caller and, where the answer is observable, to its route. Sibling passes run across: the five optimistic-commit paths (`approval_repo::swap`, `overlay_repo::flip`/`replace_lines`, `window_repo::transition`/`adjust_effective_to`, `pin_frontier_repo::advance`, `idempotency_repo::take_over`), the two `INSERT … ON CONFLICT DO NOTHING` + load pairs (`migration_repo::insert_or_load`, `synthesis_repo::freeze_or_load`), the two append-only stores (`audit_repo`, `outbox_repo`), the three client-supplied-key namespaces, and the ten `*_dedup_key` producers.

**Verdict:** the zone is unusually strong — the CAS discipline, the transaction-boundary argument and the fail-closed readings are consistent and mostly *proved* rather than asserted. Two findings are real and load-bearing: a client-supplied identifier in a **global** uniqueness namespace on a live route (Z8-1), and a pair of journal writes that discard `rows_affected` on the exact path whose safety is that predicate (Z8-2). Everything else is Medium-and-below: three fix-carried-to-one-site-not-its-sibling instances, one uncovered-cancellation lock, and a cluster of stale docs where the debt has since been paid.

---

**Findings**

**PAID `c62603c2f` — and THIS ENTRY'S *Fix/Verify* IS INCOMPLETE.** Threading `on_behalf_of` is necessary and not sufficient: `fk_pricing_bulk_row_lock_price` has no cascade, so past the guard the delete meets the foreign key and returns a 500. The second RED printed `(code: 787) FOREIGN KEY constraint failed`. All three arms are pinned, and the implementer **declined** to let a price repository drop another aggregate's lock row — the lane that lands a delete verb owes `release_locks` in the same transaction. The entry's "two interactive call sites" is also wrong: there is one.

**Z8-1 [High] `migration_id` is a client-supplied key in a global uniqueness namespace — a cross-tenant existence oracle and a permanent cross-tenant denial**
**PAID `5180ae475`.** See the status ledger at the top of this document.

`gears/bss/pricing/pricing/src/infra/storage/migrations/m20260802_000043_create_pricing_migration.rs:140` — `migration_id uuid NOT NULL PRIMARY KEY`
`gears/bss/pricing/pricing/src/infra/storage/migrations/m20260802_000043_create_pricing_migration.rs:194` — `CREATE INDEX idx_pricing_migration_tenant ON bss.pricing_migration (tenant_id, migration_id)` (non-unique)
`gears/bss/pricing/pricing/src/infra/storage/repo/migration_repo.rs:168` — `OnConflict::column(migration::Column::MigrationId)`
`gears/bss/pricing/pricing/src/infra/storage/repo/migration_repo.rs:194` — `load(runner, scope, new.tenant_id, new.migration_id)`
`gears/bss/pricing/pricing/src/infra/storage/repo/migration_repo.rs:196` — `.ok_or_else(|| RepoError::ConcurrentMutation { … })`
`gears/bss/pricing/pricing/src/api/rest/migrations.rs:227` — `migration_id: request.migration_id` (straight off the request body)
`gears/bss/pricing/pricing/src/infra/storage/migrations/m20260802_000043_create_pricing_migration.rs:10` — `//! # migration_id is client-supplied, and that is the whole of M2`
`gears/bss/pricing/pricing/src/infra/storage.rs:855` — `RepoError::ConcurrentMutation { .. } => DomainError::ConcurrentMutation(…)` → `CONCURRENT_MUTATION` 409

The conflict target is the bare `migration_id`, so the namespace of a **client-chosen** identifier is the whole deployment rather than the tenant. Tenant B posting a `migration_id` that tenant A already holds takes the `DO NOTHING` branch (`created = false`), then `load` filters by `tenant_id` and finds nothing, so B is answered `ConcurrentMutation` → **409 `CONCURRENT_MUTATION`**; posting an unused id is answered 202. That difference is observable on a live authenticated route and is a fact about another tenant's rows. The second half is worse than the leak: the refusal says *retry*, and a retry collides identically, forever — so any tenant can permanently reserve arbitrary migration ids against every other tenant, and the victim has no remedy at all (the table's `DELETE` is banned; `migration_repo.rs:134` calls the vanished-row branch "unreachable in practice", which is exactly the case it is not reasoning about).

The sibling one file over is the refutation of any "this is how the crate does it" defence. `synthesis_repo::freeze_or_load` is the *same* shape — `ON CONFLICT DO NOTHING`, then a load, then `ok_or_else(ConcurrentMutation)` — and scopes its conflict target: `gears/bss/pricing/pricing/src/infra/storage/repo/synthesis_repo.rs:137` `OnConflict::columns([TenantId, SubscriptionRef])`, with a doc block at `:134` that reasons about *which* target is right. So does the crate's other client-key store: `gears/bss/pricing/pricing/src/infra/storage/repo/idempotency_repo.rs:195` `OnConflict::columns([TenantId, Operation, ClientKey])`, and `bulk_repo`'s key moved to `(tenant_id, kind, client_key)` under D-307 (`gears/bss/pricing/pricing/src/infra/storage/repo/bulk_repo.rs:188`). `pricing_migration` is the only one left global.

I priced this High rather than Critical because no other tenant's *content* is readable and there is no financial-integrity break; if an existence oracle on a live route counts as cross-tenant reach for this review's purposes, re-price it Critical.

*Fix/Verify:* make the key `(tenant_id, migration_id)` — a migration widening the primary key (or a `UNIQUE (tenant_id, migration_id)` with the PK demoted) plus `OnConflict::columns([TenantId, MigrationId])`. Verify by a Postgres/SQLite case that two tenants schedule the *same* `migration_id` and both get 202, and that a same-tenant repeat still returns `created = false` with 200.

**Z8-2 [High] `mark_applied` / `mark_failed` throw away `rows_affected`, on the one path whose whole safety is that the row moved**

`gears/bss/pricing/pricing/src/infra/storage/repo/repricing_journal_repo.rs:167` — `.exec(runner).await.map_err(…)?;` then `Ok(())` (no `rows_affected` inspection)
`gears/bss/pricing/pricing/src/infra/storage/repo/repricing_journal_repo.rs:201` — the same, in `mark_failed`
`gears/bss/pricing/pricing/src/infra/storage/repo/repricing_journal_repo.rs:264` — `fn row_of(run_id, price_id)` — the predicate carries **no** `tenant_id`, unlike `list_for_run`'s at `:226`
`gears/bss/pricing/pricing/src/infra/storage/repo/repricing_journal_repo.rs:243` — *"a crashed run resumed by asking for `pending` rows cannot apply anything twice — and it cannot silently skip a row either"*

Every other write in this zone reads the affected count and decides from it: `approval_repo.rs:1395`, `overlay_repo.rs:458` and `:621` and `:1065`, `window_repo.rs:898` and `:1059`, `bulk_repo.rs:260`, `pin_frontier_repo.rs:283`, `idempotency_repo.rs:313`. These two are the only ones that do not. A statement matching zero rows — the wrong `run_id`/`price_id` pair, a row the scope gate filtered out, a row already removed — returns `Ok(())`, the journal row stays `pending`, and `pending_for_run` (`:250`) hands it to the re-drive, which applies the row a **second time** and mints a second successor price row on the key. The trigger cannot cover this: a trigger only fires on a row the statement matched.

It is latent today rather than live, and that is the shape worth flagging: `mark_applied` and `mark_failed` have **no production caller**. Grep across `src/` and `tests/` returns only `tests/sqlite_repricing_journal_repo.rs:228` and `:231`, which mark rows that exist — so the suite is green and stays green. This is the "rule whose counterpart system does not exist" case: it hatches the day D-308/D-309's apply lane wires in, and the wave that wires it will read a green journal suite as coverage.

*Fix/Verify:* have both functions return `RepoError::NotFound` (or `ConcurrentMutation`, matching `bulk_repo::advance`'s choice) on `rows_affected == 0`, and add `tenant_id` to `row_of` for the belt-and-braces every sibling has. Arm it with a RED case that marks a `(run_id, price_id)` pair that is not in the journal and asserts the refusal — not with a happy-path mark.

**PAID `69add0d3b`** (found already paid during the 2026-08-13 reconciliation, never marked). `mark_applied`/`mark_failed` read `rows_affected` and answer `NotFound` on zero; `row_of` gained `tenant_id`. This entry was the file's only High and it had been closed for days — which is why the reconciliation was run before any further fixing.

**Z8-3 [Medium] A repeated bundle publish at one plan revision is answered `CONCURRENT_MUTATION` forever — the defect the same file records as found-and-fixed for windows**

`gears/bss/pricing/pricing/src/infra/bundle.rs:341` — `dedup_key: format!("BundleUpdated:{bundle_id}:{revision}")`
`gears/bss/pricing/pricing/src/api/rest/bundles.rs:492` — `publish_composition(&scope, tenant, plan_id, body.plan_revision, correlation, Utc::now())` — the revision comes off the request body, and nothing between `:483` and `:492` refuses a second call
`gears/bss/pricing/pricing/src/infra/storage/repo/outbox_repo.rs:1006` — the enqueue's only error map is `contention_or_db`, i.e. any unique violation is `ConcurrentMutation`
`gears/bss/pricing/pricing/src/infra/storage/repo/outbox_repo.rs:375-390` — the same defect, on windows, diagnosed and fixed: *"an adjustment of a window that was scheduled through the route deduped against its own schedule and was refused by `uq_pricing_outbox_dedup_key` — a 409 on a legal act, and the reason no window could be adjusted twice"*

`plan_published_dedup_key`'s "a revision publishes exactly once" argument (`outbox_repo.rs:24-32`) holds because the publish commit's compare-and-swap refuses the second attempt *before* the enqueue. `publish_composition` has no such swap: it re-reads the composition, re-normalises rev-share and enqueues. So the outbox's unique index is the only thing standing there, and it answers with the wrong code — a 409 telling the operator to retry, which collides identically on every retry. The write does roll back cleanly (the enqueue is inside the same transaction as `write_effective_share`), so this is a wrong answer to a legal-looking act rather than a corruption.

*Fix/Verify:* either give the bundle publish an act segment in its dedup key, as `price_window_mutation` took, or a compare-and-swap that refuses the second publish of one revision with a code that says so. Verify by publishing one bundle twice at one `plan_revision` and asserting the second answer is not `CONCURRENT_MUTATION`.

**PAID `0a3319ba8`.** The finding was real but its premise was not: the dedup key asserted "a revision publishes exactly once", which is false for a bundle — a composition edit voids the content pin, so a republish at one revision is a LEGAL act. Remedy is `price_window_mutation`'s act segment, the act being the correlation id (D-178 clause 2). The probe drives the SECOND publish and carries a positive control that one act enqueued twice is still refused, so "no longer 409s" cannot be satisfied by a key that dedups nothing.

**Z8-4 [Medium] The two newest outbox events bypass the named-constructor discipline the module doc argues for, and hand-spell the event name as a string literal beside the enum**

`gears/bss/pricing/pricing/src/infra/bundle.rs:327` — `NewOutboxEvent { … event: CatalogEvent::BundleUpdated, … dedup_key: format!("BundleUpdated:{bundle_id}:{revision}") }`
`gears/bss/pricing/pricing/src/infra/overlay_publish.rs:328` — `NewOutboxEvent { … event: CatalogEvent::PriceOverlayPublished, … dedup_key: format!("PriceOverlayPublished:{price_overlay_id}:{revision}") }`
`gears/bss/pricing/pricing/src/infra/storage/repo/outbox_repo.rs:173-176` — *"A named constructor rather than a struct literal at the call site … A call site free to choose them is a call site free to enqueue a `PlanPublished` under a dedup key that dedups against nothing."*
`gears/bss/pricing/pricing/src/infra/storage/repo/outbox_repo.rs:30` — *"Derived in [`plan_published_dedup_key`] so that no caller can spell it a second way"*

Eight events have a `NewOutboxEvent::*` constructor and a `*_dedup_key` free function (`outbox_repo.rs:178, 204, 229, 254, 283, 310, 355, 416, 433`). The two most recent do not: they are the only `NewOutboxEvent { … }` struct literals in the crate. Both consequences of that are visible in the two lines: the event name is written as a **bare literal** in the dedup key while the enum is passed in the field immediately above it — the "imports the enum but still passes a literal" tell — and both use `:` as the separator where all ten `*_dedup_key` functions use `/` (`outbox_repo.rs:457, 596, 610, 685, 809, 935, 945`). Neither is a correctness break on its own (each key is internally self-consistent), but the *reason* the discipline exists is Z8-3, which is exactly what the bundle site then walked into.

*Fix/Verify:* give both events a `NewOutboxEvent::` constructor and a `*_dedup_key` free function beside their siblings, built from `CatalogEvent::…​.as_str()`. Verify by grepping that `NewOutboxEvent {` appears only in `outbox_repo.rs`.

**PAID `72bdb26f9`** — and THIS ENTRY'S OWN *Fix/Verify* LINE WAS WRONG and has been replaced. Both events now have `NewOutboxEvent::` constructors and `*_dedup_key` functions rendered from `CatalogEvent::as_str` with `/`. The old recipe ("grep that `NewOutboxEvent {` appears only in `outbox_repo.rs`") cannot tell a struct LITERAL from a struct UPDATE, and produced a false positive against `infra/repricing.rs`, whose `..NewOutboxEvent::price_updated(…)` sites are correct and whose per-run dedup key is required by `12-operator-efficiency.md:172`. The replacement counts the functional-update base per file and requires the two counts to be equal, plus a grep for event names inside `format!`. **CORRECTION, 2026-08-14 audit: that replacement was written into `outbox_repo`'s module doc, NOT into this entry — the *Fix/Verify* line below still carries the unsound grep.** Read the module doc, not the line below.

**Z8-5 [Medium] `SUBJECT_KINDS_WITH_A_WRITER` and its test assert that the overlay has no approval-plane writer; `submit_overlay_on` is one**

`gears/bss/pricing/pricing/src/infra/storage/repo/approval_repo.rs:185-190` — the roster: `PlanRevision, PriceUnit, Window, Policy` — no `Overlay`
`gears/bss/pricing/pricing/src/infra/storage/repo/approval_repo.rs:176-184` — *"`price_overlay` has a writer on the first (`OverlayRepo`'s four mutations) and none on the second. So it is **absent here** … the unit that would open one is Slice 9's O-7, unwired."*
`gears/bss/pricing/pricing/src/infra/approval.rs:764` — `pub async fn submit_overlay_on(…)`
`gears/bss/pricing/pricing/src/infra/approval.rs:790` — `subject_kind: AuditSubjectKind::Overlay,`
`gears/bss/pricing/pricing/src/infra/approval.rs:795` — `approval_repo::open(runner, scope, new, stamp)`
`gears/bss/pricing/pricing/src/infra/storage/repo/approval_repo.rs:1206-1211` — the same file, 1000 lines down: *"This arm refused outright while the unit was unwired … D-225's `submit_overlay_on` is the writer"*
`gears/bss/pricing/pricing/src/infra/storage/repo/approval_repo_tests.rs:176-180` — `assert_eq!(without_a_writer, [AuditSubjectKind::Overlay], "exactly one declared kind has no approval-plane writer, and it is the overlay")`

The roster contradicts its own module's `subject_aggregate` arm, and the test asserts the false sentence. Its own comment at `approval_repo_tests.rs:146-151` names this exact failure mode for the *previous* member — *"a roster is a maintained list, so the day it is wrong is the day a reader takes it as normative and the writer as the mistake"* — so `price_unit`'s wave paid the roster and D-225's wave did not. The const is `pub` and has no consumer outside the test (grep: `approval_repo_tests.rs:35, 143, 174` only), so nothing dereferences it at runtime; the cost is that the one artefact that answers "which kinds can open a unit" now answers wrongly, and the test that exists to catch that is the thing asserting it.

*Fix/Verify:* add `AuditSubjectKind::Overlay` to the roster, correct the doc at `:176-184`, and change the test's tail so `without_a_writer` is empty (or names whatever the next unwired kind is).

**PAID `a79f73320` / `adbc1feb7`**, hardened by `0475a84c9` and `6ea05eb64` (found already paid during the reconciliation). The constant holds seven kinds including `Overlay`, and the self-copying test — which compared the constant against a hand-written copy of itself, so both operands moved together — was replaced by a scan of the crate's own production sources.

**Z8-6 [Medium] The audit plane and the approval plane spell an overlay subject two different ways — the divergence `window_ref` records as corrected for windows**

`gears/bss/pricing/pricing/src/infra/storage/repo/overlay_repo.rs:938` — `subject_ref: price_overlay_id.to_string()` (a bare uuid), written by all four overlay mutations
`gears/bss/pricing/pricing/src/infra/storage/repo/audit_repo.rs:277` — `pub fn overlay_revision_ref(price_overlay_id, revision) -> String { format!("{price_overlay_id}/{revision}") }`
`gears/bss/pricing/pricing/src/infra/storage/repo/audit_repo.rs:268` — its doc: *"One overlay revision's durable **audit and approval** name"*
`gears/bss/pricing/pricing/src/infra/approval.rs:774` — the approval plane uses it: `audit_repo::overlay_revision_ref(revision.price_overlay_id, revision.revision)`
`gears/bss/pricing/pricing/src/infra/storage/repo/audit_repo.rs:306-309` — the rule, stated for windows: *"it keeps both stores on one spelling: the audit record and the approval record of one act name it identically, which is the alignment D-158 is about, and it is why the helper moved rather than its three call sites"*

`overlay_revision_ref` is declared to serve both stores and reaches only one. The plan plane obeys the rule (`retirement.rs:679`, `migration.rs:380`/`:459` all call `audit_repo::plan_revision_ref`), and so does the window plane after its correction (`window.rs:1304` calls `audit_repo::window_ref`); the overlay plane is the one that hand-writes its ref. The revision is not lost — it rides in `after_state` (`overlay_repo.rs:940`) — but a walk that joined the two planes on `subject_ref` would find nothing.

I priced this Medium rather than higher because I grepped the readers: nothing in `src/` reads `pricing_audit_log.subject_ref` at runtime, and `audit_repo`'s own doc at `:103-105` says every read surface is unbuilt. It is an audit gap with a forward dependency on whoever builds the auditor read.

Beside it, `overlay_repo.rs:944` sets `approval_ref: None` unconditionally with the comment *"D-50 makes an overlay mutation an approval subject and the unit that would carry the id is Slice 9's O-7, unwired; the field goes `Some` in the same change that opens one"* — same stale premise as Z8-5, and the field it names is the join key an auditor would use.

*Fix/Verify:* call `audit_repo::overlay_revision_ref(price_overlay_id, revision)` in `record_overlay_mutation`, and decide whether the publish path can thread the unit id into `approval_ref`. Verify by asserting the audit record's `subject_ref` equals the approval record's for one overlay revision.

**STILL OPEN, and now routed.** The only divergent writer is `overlay_repo::record_overlay_mutation` (`subject_ref` as a bare uuid), with its test pinning that spelling; everything else already calls `audit_repo::overlay_revision_ref`. The four-roster obligation is NOT triggered — `AuditSubjectKind::Overlay` is in all four already. Note `revision` is `i64` there, so the fix needs a checked conversion rather than a one-liner.

**PAID `305f647ae`.** `subject_ref` renders through `audit_repo::overlay_revision_ref` with a CHECKED `i64→u64` conversion resolving to `CorruptRow`. The prior trace was verified rather than assumed, and the divergence turned out to be visible INSIDE one audit segment: the `create` record said `<uuid>` while the `submit` record one seq later said `<uuid>/0`. The probe exploits exactly that, with the `submit` record asserted unchanged as its positive control. No migration: the column is free text with no runtime reader, and rewriting a hash-chained append-only log to manufacture a backfill would dwarf the gap.

**Z8-7 [Medium] `bulk_repo::advance` is the zone's only state-moving write with no compare-and-set predicate, and the trigger admits the self-edge**

`gears/bss/pricing/pricing/src/infra/storage/repo/bulk_repo.rs:252-256` — the whole `WHERE`: `TenantId.eq(tenant_id)` and `OperationId.eq(operation_id)`; no expected-state conjunct
`gears/bss/pricing/pricing/src/infra/storage/repo/bulk_repo.rs:5-11` — the justification: *"The state machine is the trigger's, and this repository does not restate it"*
`gears/bss/pricing/pricing/src/api/rest/bulk_imports.rs:443-449` — *"**The trigger does not refuse this one, and D-293 claimed it did.** A move to the state a run is already in returns early on both engines, so an abort against a run already in `completed_with_conflicts` … would rewrite `completed_at` and stamp an abort note over a report where every row WAS attempted."*

The trigger's early-return on a self-edge is the hole the deferral to it does not cover: `advance` also rewrites `report` and `completed_at` wholesale, so any repeat lands silently and clobbers the run's stored answer. The remedy that exists is a guard **in the route** (`bulk_imports.rs:456`, `if run.state != BulkState::Committing`), not in the store — which is the arrangement the crate elsewhere argues against (`window_repo.rs:1030-1035`: *"a tag read, compared and then handed to a statement is a decision racing the write it authorizes"*). Every look-alike carries the premise into the statement: `approval_repo.rs:1389`, `overlay_repo.rs:1194`, `window_repo.rs:891`/`:1048`, `pin_frontier_repo.rs:277`, `policy_repo.rs:397`, `idempotency_repo.rs:307`.

*Fix/Verify:* add the expected-`from` state to `advance`'s filter and resolve `rows_affected == 0` into "no such run" vs "the run has moved", the way `approval_repo::swap` (`:1395-1403`) does. Verify with a case that calls `advance` twice to one state and asserts the second is refused rather than rewriting the report.

**PAID `8800f90c8`.** `advance` now carries the state it was decided on into the statement, as every sibling write in the zone does. It had gained EIGHT callers since this entry was filed. RED output prints the clobbered report.

**Z8-8 [Medium] Bulk row locks have no Drop guard: a panic or a cancelled future leaves durable locks on an autocommit connection**

`gears/bss/pricing/pricing/src/infra/storage/repo/bulk_repo.rs:286-291` — *"`runner` **must be an autocommit connection, not a transaction**"*
`gears/bss/pricing/pricing/src/infra/storage/repo/bulk_repo.rs:338` — the release, on `take_locks`' own `Err` arm only
`gears/bss/pricing/pricing/src/infra/bulk.rs:170-200` — the caller's shape: `take_locks` → `commit_rows` (many `.await`s) → `release_locks`, with the `?`s deliberately removed from both statements
`gears/bss/pricing/pricing/src/api/rest/bulk_imports.rs:478-481` — *"nothing else calls `release_locks`, the lock table has no sweeper, and D-37's lease takeover is designed and unbuilt"*

`infra::bulk::commit` covers the **Err** exit meticulously — the block comment at `bulk.rs:168-174` and the two removed `?`s are the whole of it. It does not cover the other two abnormal exits. A panic inside `commit_rows` unwinds past the release; a dropped future (client disconnect, shutdown, a losing `select!` arm) at any `.await` inside `commit_rows` simply stops running, and neither is rolled back because the locks were taken on an autocommit connection by design. The rows stay frozen against every interactive editor and the run stays `committing` with no timeout. Only a `Drop` guard covers panic and cancellation together; there is none in this crate.

Priced Medium, not High, because a remedy exists and is reachable: the `POST …/abort` route (`bulk_imports.rs:482`) releases the locks and is deliberately ordered release-first so a failed abort is retryable. The residue is that nothing *detects* the state — no timeout, no sweeper, no alarm on a run stuck in `committing` — so an operator has to notice.

*Fix/Verify:* a `Drop`-implementing guard that owns `(conn, scope, tenant, operation_id)` and issues the release, or a sweeper over `committing` runs past a horizon. Verify by dropping the commit future mid-`commit_rows` and asserting `lock_holder` answers `None` afterwards.

**PAID `a931f5e3f`.** The `Drop` guard that `3de9d0c73` built for the repricing apply is now carried to the bulk commit lane this entry filed against. Residual, stated rather than hidden: a cancelled commit lands terminal carrying only the entry report, so rows it did commit are in the store but not in the report — the same limitation the abort route has had since D-300.

**Z8-9 [Low] `window_repo`'s module doc reports an audit debt that was paid, in three claims that are now false**

`gears/bss/pricing/pricing/src/infra/storage/repo/window_repo.rs:149` — *"`pricing_audit_log` gets nothing yet."*
`gears/bss/pricing/pricing/src/infra/storage/repo/window_repo.rs:163-165` — *"`created_by` is frozen by the whitelist, this table has no `updated_by`, and **no audit row is written**. So the store holds who *scheduled* a window and cannot answer who *shortened* it"*
`gears/bss/pricing/pricing/src/infra/storage/repo/window_repo.rs:166-167` — *"That is a missing column, not a missing INSERT, and it is reported rather than patched"*
`gears/bss/pricing/pricing/src/infra/window.rs:1693, 1708, 1719` — all three repo mutations (`schedule`, `adjust_effective_to`, `transition`) run through one commit path
`gears/bss/pricing/pricing/src/infra/window.rs:1299-1304` — that path appends a `NewAuditEntry` with `actor_principal_id: stamp.actor_principal_id` and `subject_ref: audit_repo::window_ref(…)`

The record is written, and its actor is the stamp's — so "who shortened it" is answerable. The debt is a repository-scoped statement that reads as an absolute one, and the risk is a later wave paying it twice. (Adjacent, and outside this zone to fix: `window.rs:775` `const AUDIT_ACTION: AuditAction = AuditAction::Publish` is one token for all three acts, so the trail distinguishes them only through `before_state`/`after_state`.)

*Fix/Verify:* rewrite §"The `AuditStamp` is taken and the trail is **not** written here" to say the trail is written one layer up and name the site.

**PAID `eed356136`**, docs.

**CORRECTION, 2026-08-14 audit: PAID AND THEN RE-BROKEN IN THE SAME PARAGRAPH.** The rewrite carried forward the sentence "`pricing_audit_log.subject_kind` is free `text` with no CHECK at all" — which `m20260802_000074`, the migration that paid Z6-6 a few hours later the same morning, made false. It also cites the `postgres_migrations` roster as its proof, and that roster is exactly what the new CHECK joined. A doc corrected against the code of the hour is a doc that expires within the day; this one expired inside one wave.

**Z8-10 [Low] `outbox_repo` says "thirteen" three times against a fourteen-member frozen set**

`gears/bss/pricing/pricing/src/infra/storage/repo/outbox_repo.rs:47` — *"`chk_pricing_outbox_event_name` pins the same thirteen names"*
`gears/bss/pricing/pricing/src/infra/storage/repo/outbox_repo.rs:155` — *"Which of the thirteen frozen names."*
`gears/bss/pricing/pricing/src/infra/storage/repo/outbox_repo.rs:726` — *"pinned to thirteen values by `chk_pricing_outbox_event_name`"*
`gears/bss/pricing/pricing/src/domain/events.rs:70-85` — `CatalogEvent::ALL` has fourteen members
`gears/bss/pricing/pricing/src/infra/storage/migrations/m20260802_000060_add_price_overlay_published_event_name.rs:46-51` — the CHECK lists fourteen

Another instance of the F-6 class. Same wave as Z8-4: D-248 added the name and the migration and left the three counts.

*Fix/Verify:* say "the frozen set" rather than a number, the way `subject_kind`'s roster was corrected (`approval_repo_tests.rs:107-109`: *"The count is deliberately not in this sentence"*).

**PAID `f2705a307` — and the ENTRY WAS RIGHT while the controller's dispatch note was wrong.** The set is fourteen in the enum and fourteen in the CHECK, and no migration after `m20260802_000060` touches it; the note that claimed it was larger than either number was mistaken. What the entry missed was two further sites — `entity/outbox.rs`, and a migration that asserts a now-false **equality** with the enum. Count the set; do not trust a summary of it, including this controller's.

**Z8-11 [Low] `highest_revision`'s doc contradicts the D-231 correction its own caller carries**

`gears/bss/pricing/pricing/src/infra/storage/repo/overlay_repo.rs:1947-1949` — *"**It is the max of what the table still holds**, which is not the max of what the overlay has ever minted — a discarded draft is deleted outright. See [`OverlayRepo::open_revision`] and owed-register entry O-13."*
`gears/bss/pricing/pricing/src/infra/storage/repo/overlay_repo.rs:331-339` — the caller, correcting exactly that: *"A discarded draft used to leave by DELETE … `m20260802_000045` added `abandoned` and removed DELETE as an exit, so the discarded row is still here and still counted"*
`gears/bss/pricing/pricing/src/infra/storage/migrations/m20260802_000045_add_price_overlay_abandoned_state.rs:107-110` — the DELETE ban, for every state

The fix landed at the call site and not in the helper it points at, and the helper is the one a reader lands on from the register entry it cites.

*Fix/Verify:* delete the stale paragraph and the O-13 pointer.

**PAID `2e19ccdbe`**, docs — the stale paragraph and a dangling pointer to an id declared nowhere in the gear are gone.

**Z8-12 [Low] Two transplanted/duplicated doc blocks in `overlay_repo` — one of them the exact defect `outbox_repo` diagnoses by name**

`gears/bss/pricing/pricing/src/infra/storage/repo/overlay_repo.rs:491-512` — the `publish_revision` doc block is present twice, verbatim, inside one `///` run
`gears/bss/pricing/pricing/src/infra/storage/repo/overlay_repo.rs:1853-1863` — `collect_lower_layer`'s doc (*"The targets one published line at a lower precedence would have a `fixed` layer discard (D-138)"*) runs straight into `collect_cross_class_tie`'s (*"The tie one published line of a **different** class…"*), and the merged block attaches to `collect_cross_class_tie` at `:1864`
`gears/bss/pricing/pricing/src/infra/storage/repo/overlay_repo.rs:1890` — `fn collect_lower_layer(…)` now carries no doc at all
`gears/bss/pricing/pricing/src/infra/storage/repo/outbox_repo.rs:858-865` — the same defect, named: *"That commit inserted the sibling type between this doc and the type it describes … A doc block that has to explain which type it belongs to has already been transplanted."*

Note as possibly mid-flight — `overlay_repo.rs` is one of the files another session is committing to.

*Fix/Verify:* split the block at `:1858` and move the first half back onto `collect_lower_layer`; drop the duplicated half of `:502-512`.

**PAID `2e19ccdbe` — and UNDERCOUNTED.** A third instance of the same transplant sits in `tests/sqlite_overlay_repo.rs`, where one decision's prose block had migrated onto another's case with its severed tail sentence stranded on the original. All three restored. This is the fifth entry in this register whose enumeration was shorter than the code's.

**Z8-13 [Low] `overlay_facts` is an unbounded N+1 fan-out inside the publish transaction, where its own sibling one screen up is page-bounded by D-125**

`gears/bss/pricing/pricing/src/infra/storage/repo/overlay_repo.rs:1792-1804` — every published overlay of the tenant, no `limit`
`gears/bss/pricing/pricing/src/infra/storage/repo/overlay_repo.rs:1823` — `read_lines(…)` per overlay, which itself issues one amounts query per line (`:1422`)
`gears/bss/pricing/pricing/src/infra/storage/repo/overlay_repo.rs:735-777` — `OverlayRepo::list`, the sibling, takes `limit` and argues for it: *"that order cannot carry a keyset walk … exactly what D-125 forbids"*
`gears/bss/pricing/pricing/src/infra/storage/repo/overlay_repo.rs:1631-1634` — and this runs on the **transaction**: *"The commit needs it on the transaction, not on a fresh connection"*

The validation genuinely needs the whole set, so a page bound is the wrong fix; the cost is that a tenant's overlay count decides how long the publish transaction stays open, and §4.2 runs the world twice per publish.

*Fix/Verify:* fold the per-line amounts read into one query per revision (or one over the whole published set), and record the intended bound on the tenant's published-overlay count.

**PAID `fcb671e79`, and its probe COUNTS QUERIES, which is the only honest measurement of an N+1 claim.** `read_lines` now reads a revision's amounts in one statement, so `load`/`list`/`copy_lines`/`overlay_facts` all stop fanning out. A hand-rolled subscriber over the statement log read **12 before / 3 after** on a 3-overlay × 4-line seed — and the report states plainly that a one-line seed could not have distinguished the two shapes. Armed three ways so it cannot pass by measuring nothing or by reading less. **One bound deliberately NOT added:** the published-set read stays unpaged, because all three facts over it are absence claims and a page bound would let them pass by paging. The residual — still linear in the tenant's published-overlay count — is recorded on `overlay_facts`, and the per-tenant cap it would need does not exist in the design set.

**Z8-14 [Low] Three driver-message matches with no `is_unique_violation()` conjunct**

`gears/bss/pricing/pricing/src/infra/storage/repo/overlay_repo.rs:1200-1204` — `if e.to_string().contains("uq_pricing_price_overlay_precedence") || e.to_string().contains("pricing_price_overlay.precedence")`
`gears/bss/pricing/pricing/src/infra/storage/repo/overlay_repo.rs:1976-1980` — `is_line_identity_collision`, the same shape
`gears/bss/pricing/pricing/src/infra/storage/repo/bundle_repo.rs:702-704` — `uq_pricing_bundle_plan` / `pricing_bundle.plan_id`, the same shape
`gears/bss/pricing/pricing/src/infra/storage.rs:699-701` — the crate's stated narrow exception: `if err.is_unique_violation() && names_the_policy_guard(&err.to_string())`, with `:662-690` arguing at length why message matching is the narrow case and not a precedent

`policy_guard_or_contention` conjuncts the typed class before it reads text; these three read text alone, so any error whose message happens to carry the substring is reported as the specific refusal. Weak-form: the substrings are DDL these chains own, so a false positive needs a contrived message.

*Fix/Verify:* add the `is_unique_violation()` conjunct to all three, matching `storage.rs:699`.

---

**Verified clean**

- **`audit_repo` whole.** The chain's linearity is a property of the key `(tenant_id, chain_id, seq)` and not of an isolation level (`audit_repo.rs:23-30`), the head is `MAX(seq)` over the whole segment rather than over mutation rows (`:334-337`), `head_hash` refuses a short digest rather than padding it (`:484-492`), and the three previously-owed concurrency properties are paid by `tests/postgres_audit_chain.rs` (`:68-88`). `overlay_chain` (`:240-246`) rewrites the UUID version nibble to `8`, making overlay chains disjoint from v7 plan chains and from `policy_chain` **by construction**, with the three properties asserted rather than described.
- **`approval_repo::decide`/`swap`.** The state machine is consulted three times and none is redundant (`:35-47`); the `UPDATE` carries its own `state = 'submitted'` (`:1389`); zero rows is **re-read** so the refusal names the row's state now rather than the state the caller read (`:1395-1403`), and `NotFound` vs `ApprovalNotPending` are told apart. `open` distinguishes the primary-key loser from the D-192 mint guard via `policy_guard_or_contention` (`:343-356`) and the register insert's unique violation is provably the pending-key index (`:378-390`).
- **The approval-key register.** Order is parent-then-register with the reason stated (`:359-362`), `held_keys` is a `BTreeSet` so a plan with two rows on one key holds it once (`:225-232`), `find_pending_key_holder` has **no** `subject_kind` filter and the C-3 note says why it must not (`:606-612`), and `find_pending_key_holders` does one read per *distinct* unit rather than per key (`:678-696`). `find_approved_for_content` matches subject **and** content exactly, with the forever-unpublishable-plan failure mode spelled out (`:769-796`).
- **`idempotency_repo` whole.** The gate is the insert, recognised through the driver's typed `RecordNotInserted` rather than a message (`:203-215`); expiry is asked *before* the hash so one payload cannot poison a key past the TTL (`:219-225`); `record_response` is write-once by the `response_status IS NULL` conjunct (`:307`) with the `NotFound` conflation argued (`:268-275`); `take_over`'s compare-and-swap on `created_at_utc` decides the takeover race and the loser is refused in-flight rather than for a mismatch it never lost (`:392-405`). `key_of` (`:439`) spells the composite key once so no statement can address a row by fewer axes.
- **`pin_frontier_repo::advance`.** Forward-only twice — the pre-read refusal (`:262`) and the `catalog_version < target` conjunct (`:277`) — with a regression *reported* rather than swallowed and the argument for that at `:16-22`.
- **`policy_repo::set_tax_display_policy`.** The premise is in the `WHERE` (`:394-398`), and the zero-rows branch reads whether the row exists at all so a first write is told apart from a lost update (`:406-425`) — the T-7 defect named and avoided.
- **`window_repo`'s edge conditions.** §4's conditions live in the domain predicate, in the pre-check *and* in the `UPDATE`'s `WHERE` (`:1353-1361`), so a second sweep over one instant matches zero rows; `idempotent_arrival` (`:1329`) is `active|expired` only, and the doc traces which of the three callers reaches which branch. `intersects` (`:1437-1446`) gets half-open adjacency right and §9's false positive is named. `list_due`'s lifecycle half is a **subquery** rather than a post-page filter, with the head-of-page starvation it would otherwise cause spelled out (`:750-760`).
- **`threshold_repo`.** `latest_version` takes the max across the entry table **and** the tombstone table (`:164-168`) with the one-number-two-signatures hazard argued; `read_version` derives `effective_from` as a max and reports a disagreement as `CorruptRow` rather than picking (`:259-269`); a version that is both tombstone and entry set fails closed (`:245-253`); `versions_desc` re-sorts after the merge rather than trusting two descending lists (`:461-477`).
- **Migration `000060`'s SQLite rebuild.** `DROP TABLE` takes its indexes, and all three are recreated after the rename — including `uq_pricing_outbox_dedup_key` (`m20260802_000060…:102-108`), whose silent loss the module doc identifies as *"a duplicate-event bug that nothing in the fast tier would see"*.
- **Overlay lifecycle guards, against the schema as it now stands.** `uq_pricing_price_overlay_open_draft ON (price_overlay_id) WHERE lifecycle_state='draft'` and `uq_pricing_price_overlay_precedence ON (tenant_id, scope_class, precedence) WHERE lifecycle_state='published'` (`m20260802_000032…:141-148`, recreated identically by `000045`) are what make `revision_in_state(…).one()` unambiguous and `precedence_holder`'s last-write-wins loop (`overlay_repo.rs:1820`) single-valued. `publish_revision_on` proves the target publishable **before** writing anything and supersedes the predecessor first, with the by-state-lookup hazard measured rather than argued (`overlay_repo.rs:1008-1036`).
- **Transaction boundaries.** Every store in this zone that must join the caller's transaction takes `&impl DBRunner` and opens none: `audit_repo`, `outbox_repo`, `approval_repo`, `window_repo`, `threshold_repo`, `migration_repo`, `synthesis_repo`, `repricing_journal_repo`, `pin_frontier_repo::advance`. `idempotency_repo` takes `&DbTx` outright. The one deliberate exception, `bulk_repo::take_locks`, states the requirement and its measured consequence (`bulk_repo.rs:286-301`).
- **`published_at` is written-but-never-read [by-design], documented.** Grep for readers of `pricing_outbox.published_at` in `src/` returns only the writer (`outbox_repo.rs:991`); the relay is out of gear scope and `outbox_repo.rs:34-40` says so, with the reasoning for why `events_enabled` gates fan-out and not the row.
- **Overlay world's cross-plane cohort compare [by-design], documented.** `published_generation` (`overlay_repo.rs:1916-1939`) converts the price plane's epoch-millis text into a `DateTime` at one seam rather than comparing timestamps as text, and states the trap it avoids (*"`SeaORM` writes ISO 8601 with a `T`, and `'T'` beats `' '` at byte 11"*).
- **`refuse_overlap` has no serialization point [by-design], documented and *pinned*.** `window_repo.rs:1107-1168` withdraws the fix it used to prescribe with three measured reasons, names the two out-of-remit routes, and pins the hole with a Postgres test asserting the negative — *"Its reddening is good news and it says so"*.

**Refutations**

- **Ordered compare on TEXT.** I checked every `order_by` and range compare in the zone against the column's type on both engines. The timestamp orderings — `approval_repo.rs:549`/`:592`/`:817`/`:1245`, `window_repo.rs:1471`, `pin_frontier_repo.rs:181` — are `timestamptz` on Postgres and `text` on SQLite, but SeaORM's rendering is fixed-width UTC so the lexicographic order coincides, and the crate states that once (`window_repo.rs:435-438`, citing `m20260802_000002`'s caveat). The rest are numeric or uuid: `outbox_repo.rs:1030` (`seq bigint`), `audit_repo.rs:454` (`seq bigint`), `threshold_repo.rs:150`/`:159`/`:449`/`:457` (`version bigint`), `read_model_repo.rs:226` (`catalog_version bigint`), `overlay_repo.rs:1964` (`revision bigint`), `approval_repo.rs:488` (uuid keyset). No misfiring ordered compare found. The one genuine cross-rendering compare has a dedicated seam (`published_generation`).
- **`take_over` not comparing the winner's payload hash** looked like a hole; `idempotency_repo.rs:358-364` argues it, and correctly — this transaction never read the winner's digest, so refusing for a mismatch would be a refusal on evidence it does not have, and at-most-once holds either way because the loser executes nothing. Not a finding.
- **`void_pending_for_plan` filtering `subject_kind = 'plan_revision'`** looked like a narrowing that would leave window units unswept. It is deliberate and paid: `audit_repo.rs:311-315` states that the filter *"used to be belt-and-braces and is now load-bearing"* precisely because `window_ref` gained the plan prefix, and both readers carry it (`approval_repo.rs:546`, `:1313`). Exact match to the established sibling — recorded and dropped.
- **`bundle_repo::replace_composition_on` looked like a drop-then-write with no guard.** It is a compare-and-swap: `bundle_repo.rs:750-755` pre-reads, `:771` bumps under the guard, and `:776-778` resolves a zero-row swap into `refuse(…)`, with the rollback restoring the composition it had already replaced. Sound.
- **`approval_repo::void_pending_for_subject` not filtering `subject_kind`**, unlike its plan sibling, looked like an over-sweep. It matches `subject_ref` **exactly**, and the ref formats (`<plan_id>/<revision>` integer tail, `<plan_id>/<window_id>` uuid tail, `<overlay_id>/<revision>`) cannot collide without a uuid collision. Its doc argues the narrowness (`:824-838`). Not a finding.
- **`enqueue` folding the dedup index and the sequence index into one `ConcurrentMutation`** looked like a defect; `infra/storage.rs:620-626` names it as a known residue with its cost, and the two do remedy the same way for the eight events whose dedup key is the act's own identity. The live consequence is Z8-3, filed against the bundle key rather than against this fold.
- **`repricing_journal_repo::row_of` omitting `tenant_id`** is not a tenancy hole on its own — `.scope_with(scope)` is the RLS gate, and `run_id` is a uuid FK. It is folded into Z8-2 as the belt-and-braces every sibling has, not filed as cross-tenant reach.

**Not covered**

- `price_repo.rs`, `plan_repo.rs`, `plan_shape_repo.rs`, `catalog_version_ref_repo.rs`, `taxonomy_repo.rs` — another reviewer's; read only as sibling evidence (`plan_repo::read_token`, `plan_repo::tx_failure`, `price_repo::load_scope_key(s)`).
- `read_model_repo.rs` and `bundle_repo.rs` were read at every write/read site and at their module docs, not line-by-line end to end; the bundle rules themselves (`domain::bundle_rules`) are F-1…F-5's ground and I did not re-open them.
- The service layers are outside the zone: `infra/{approval,window,bulk,overlay_publish,bundle,cutover,supersession,retirement,migration}.rs` were read only where a repo claim had to be checked against its caller.
- The migration bodies were read for `pricing_outbox`, `pricing_price_overlay`, `pricing_approval`(+`_key`), `pricing_migration` and the abandoned-state rebuild; the other ~58 were not audited.
- No test was executed and nothing was built — the tree is being committed to by another session. `overlay_repo.rs`, `api/rest/overlays.rs`, `tests/rest_overlays.rs` and `tests/sqlite_overlay_repo.rs` were dirty at the start of this pass; Z8-12 in particular may be mid-flight.


---

**Zone Z9 — infra service layer**

**Files:** every `.rs` directly under `gears/bss/pricing/pricing/src/infra/` except `storage.rs`, `storage/**`, `jobs/**`, `metrics*`. Directory listed first, so the roster is complete: `approval.rs` (+`approval_tests.rs`), `bulk.rs`, `bundle.rs`, `change_graph.rs`, `clone.rs`, `currency_binding.rs`, `cutover.rs`, `error_mapping.rs` (+tests), `fixture_gate.rs` (+tests), `history.rs` (+tests), `idempotent.rs` (+tests), `import.rs`, `jobs.rs`, `migration.rs`, `overlay_publish.rs`, `publish.rs` (+tests), `read_model.rs` (+tests), `retirement.rs`, `supersession.rs` (+tests), `synthesis.rs`, `threshold.rs`, `window.rs` (+tests). Corroborating reads outside the zone: `domain/approval/decision.rs`, `domain/approval/content_pin.rs`, `domain/publish.rs`, `domain/synthesis.rs`, `domain/scope_key.rs`, `infra/storage/repo/{approval_repo,audit_repo,price_repo,window_repo}.rs`, `api/rest/{approvals,bulk_imports,cutovers,migrated_origin_snapshots,publish}.rs`, `tests/rest_cutovers.rs`, `tests/rest_migrated_origin_snapshots.rs`.

**What's done:** the control-flow/state-machine catalog run over every mutating or gated operation in the zone — the five approval submits + `judge`, the publish pre-check and commit, the three window mutations, the supersession, the cutover, the retirement, the overlay publish, the threshold propose/retire, the bulk commit, the migration schedule/cancel, the clone, the synthesis freeze, the read-model projector pass. For each: the order of refusals against the first durable write, the idempotency/dedup key against the operation's identity, the replay read's lifecycle predicate, the abnormal-exit paths (Err / panic / future-drop), the actor rules against the design set's declared ones, and the sign/threshold questions. Sibling consistency was run first and is where three of the four top findings came from. Static smells swept: stringly-typed enum tokens (grepped each `as_str()` arm value outside its definition file), doc-vs-code drift, dangling doc links, and the `From<DomainError>` ladder's exhaustiveness.

**Verdict:** the plane is in good shape and the hard parts are right — the gate ordering, the content pin, the approve→commit windows, the "consult the approved unit before the evaluator" fix, and the idempotency seam are all sound and argued. What is wrong is concentrated in the two acts that were built last and copied least carefully from their siblings: **the cutover commit is audited nowhere**, and **the legacy-snapshot synthesizer resolves a market to one row on two of ten key axes** with no lifecycle filter. One authz rule (`inst-as-void`'s withdrawer) is declared and enforced by neither layer.

**Findings**

**PAID `632ae8cdd` for two of three sites, and the third is ALSO PAID — `bundle_repo`'s `uq_pricing_bundle_plan`, closed later the same day. (This line said "still open" until the 2026-08-14 audit found otherwise; stale in the safe direction.)** Only two of the named sites are in `overlay_repo`; the third is `bundle_repo`'s `uq_pricing_bundle_plan` and was out of that lane's scope. Both fixed sites now conjunct `is_unique_violation()` with a split-out name predicate, mirroring the existing guard. The two "named but not unique" cases fail against BOTH conjuncts reverted at once, with the real-driver cases on both engines as positive controls.

**Z9-1 [High] the grandfathering cutover commits four table changes and writes no audit record at all**
**PAID `7f3da53a5`.** See the status ledger at the top of this document.
`gears/bss/pricing/pricing/src/infra/cutover.rs:594` — `cutover_in`, the whole body
`gears/bss/pricing/pricing/src/infra/cutover.rs:752` — `commit_cutover(...)`: `published → superseded` flip, two `publish_rows`, one `adjust_effective_to`, two `schedule`
`gears/bss/pricing/pricing/src/infra/cutover.rs:774` — `record_pending`; `:818` and `:843` — three outbox rows; then `:867` `Ok(CutoverOutcome::Committed(...))`
`grep -n "audit_repo\|AuditAction\|NewAuditEntry" infra/cutover.rs` → **zero hits**; `grep -rn "audit_repo::append" src/ | grep -v storage/repo` → `supersession.rs:995`, `window.rs:1282`, `migration.rs:370,449`, `retirement.rs:669`, `publish.rs:645`, `approval.rs:1313` — cutover is the only committing act on this plane that is absent
`gears/bss/pricing/pricing/src/infra/supersession.rs:995-1023` — the exact sibling: `chain_id: plan_chain`, `action: Publish`, `subject_kind: PriceUnit`, `subject_ref: subject_ref` (the act), `approval_ref: authorization.as_ref().map(|r| r.approval_id)`
`gears/bss/pricing/pricing/src/infra/storage/repo/price_repo.rs` — `publish_rows` and `supersede_row` contain no `record_price_mutation` and no `audit_repo::append` (checked by `awk` over each function body), so nothing beneath compensates
`gears/bss/pricing/pricing/src/infra/cutover.rs:426-445` — `CutoverReceipt` carries no `verdict` and no `authorization`, unlike `SupersessionReceipt` (`supersession.rs:479,482`)
`gears/bss/pricing/docs/design/05-governance.md:340` — `inst-au-complete`: "actor, timestamp, before/after version refs, approval trail (submitter/approver/decision/reason), correlation id"; `:471` — "§8's `dod-audit` (**every mutation MUST record**)"
`gears/bss/pricing/pricing/tests/rest_cutovers.rs` — six tests; `grep -n audit` returns only an `AuditStamp` construction. Nothing asserts a record, so this ships green.

The submit half *is* audited — `submit_cutover_on` → `approval_repo::open` appends (`approval_repo.rs:1008`) — and so is the staging of the two drafts (`create_draft_on`/`insert_successor_draft_on` → `write_prepared` → `record_price_mutation`, `price_repo.rs:2864`). What has no record is the act that actually moves money: the predecessor's supersession, the successor's and the copy's publish, and the three window moves. The consequence is not only a missing row. `approval_ref` is the only join between the approved unit and the act it authorized, and on this path it is written nowhere — not on an audit record (there is none) and not on the receipt (no field) — so an auditor holding a cutover cannot establish that a second principal approved it, and `inst-tp-record` is unsatisfied for this act. This is the "a fix carried to one site and not to its sibling" shape: `supersede_in` and `cutover_in` were written to the same eleven-step skeleton and the cutover's copy stops one step short.

*Fix/Verify:* append the sibling record at the end of `cutover_in`, `chain_id: audit_repo::plan_chain(context.plan_id)`, `action: Publish`, `subject_kind: PriceUnit`, `subject_ref: subject_ref` (the act, already built at `:637`), before/after naming the three rows' states, `approval_ref: authorization.as_ref().map(|r| r.approval_id)`, `correlation_id: stamp.correlation_id`; and carry `authorization` onto `CutoverReceipt` as the supersession does. Arm the probe against the claim: assert in `rest_cutovers.rs::the_call_after_an_independent_approve_commits_...` that `pricing_audit_log` gained a row whose `approval_ref` equals the approval the test approved — not merely that a row exists.

**Z9-2 [High] legacy-snapshot synthesis resolves a market on two of ten key axes, with no row-lifecycle filter, and keeps exactly one candidate**
**PAID `b8a70a3de`.** Both halves; `FrozenKey` decided as a **market** on D-76's own wording for the frozen pair. See the status ledger at the top of this document.
`gears/bss/pricing/pricing/src/infra/synthesis.rs:77-83` — `FrozenKey { currency, region }`
`gears/bss/pricing/pricing/src/infra/synthesis.rs:101-116` — the candidate filter: `state != Cancelled && scope_key.currency() == key.currency && scope_key.region() == key.region && [from, to)`. Nothing about phase, `charge_kind`, `price_eligibility`, `cohort`, `meter` or `dimension_key`; nothing about the *price row's* `lifecycle_state`
`gears/bss/pricing/pricing/src/infra/synthesis.rs:119` — `live.sort_by_key(|c| c.price_id)`; `domain/synthesis.rs:244` — `select_row` returns `live.first()`, i.e. the lowest `price_id`
`gears/bss/pricing/pricing/src/infra/publish.rs:1143` — "`window_repo::list_for_plan` is taken whole — every window state, over every price row of the plan **whatever its lifecycle state**"
`gears/bss/pricing/pricing/src/infra/read_model.rs:1146-1149` — the sibling reader that asserts fact to a consumer filters `PROJECTED_WINDOW_STATES.contains(&w.state) && projected.iter().any(|row| row.price_id == w.price_id)` where `projected` is `PROJECTED_ROW_STATES`
`gears/bss/pricing/pricing/src/domain/synthesis.rs` module doc — D-76 clause 1 is "the `pricing_price` row, **current or superseded**, whose `PriceWindow` covered `t` on that key"; a `draft` row is neither
`gears/bss/pricing/pricing/src/infra/synthesis.rs:436` — `outcome.ensure_complete(...)`; the outcome is one `SelectedRow` per `FrozenKey`, so a market cannot express more than one row
`gears/bss/pricing/pricing/tests/rest_migrated_origin_snapshots.rs:354` — `assert_eq!(rows.len(), 1)`; every fixture seeds one row per market, so neither the multi-line nor the draft case is exercised

Two distinct breaks in one filter. (a) A market that legitimately carries more than one line — the hybrid recurring+usage plan `inst-cs-hybrid` exists to sanction, or a plan with an `existing_grandfathered` generation beside its `all_subscriptions` row — presents several live candidates, and the frozen `migrated-origin` payload keeps the one with the lowest `price_id` and silently drops the rest. That payload is self-contained by construction (D-87): Rating evaluates from it and Billing posts from it, resolving nothing through the read model, so a dropped line is a charge that never happens. (b) A **draft** price row's window is admissible evidence, so synthesis can freeze as "what the subscriber was paying" a row that never published, never passed the publish rules and was never approved. `read_model::project_windows` — the other reader of the same `list_for_plan` that asserts fact to a consumer — restricts on exactly the axis this one omits.

It is **latent today**: `grep -rn "\.synthesize(" src/ tests/` finds callers only in `tests/rest_migrated_origin_snapshots.rs`; the mounted route (`api/rest/migrated_origin_snapshots.rs:158`) is the `GET` alone. That is the "rule whose counterpart system does not exist" shape — it hatches, with green tests, on the day the migration executor or the first-rating exception path gains a writer.

*Fix/Verify:* decide whether `FrozenKey` is a market or a key. If it is a market, `resolve` must return the *set* of rows live on it and `SynthesisOutcome::selected` must be able to hold them; if it is a key, `FrozenKey` needs the remaining axes. Either way add `&& PROJECTED_ROW_STATES-equivalent` (D-76's "current or superseded") on the row behind the window, resolved the way `read_model::project_windows` resolves it. Probe: seed one market with a recurring row and a usage row plus a draft row, and assert the frozen payload names all the sellable lines and none of the draft.

**Z9-3 [High] `inst-as-void`'s withdrawer is the submitter or a `CatalogAdmin`; the rule the service enforces is "anyone the gate let through"**
**PAID `7f3da53a5`.** See the status ledger at the top of this document.
`gears/bss/pricing/pricing/src/infra/approval.rs:1333-1339` — `judge` reads the record and therefore holds `record.submitter_principal`
`gears/bss/pricing/pricing/src/infra/approval.rs:1366-1375` — it is passed to `authorize_decision` as `submitter_principal`
`gears/bss/pricing/pricing/src/domain/approval/decision.rs:331-344` — the only use is inside `if let Some(approver) = request.decision.approver()`; `approval.rs:1392-1400` records that `approver()` is `None` on **every** void, so both rule 2 and rule 3 are skipped
`gears/bss/pricing/pricing/src/infra/approval.rs:1346-1350` — a `Void` also skips the re-derivation, so no pin is consulted either
`gears/bss/pricing/pricing/src/api/rest/approvals.rs:1044` — `DecisionBy::Void(Some(ctx.subject_id()))`: the caller's identity travels, and nothing compares it to anything
`gears/bss/pricing/pricing/src/api/rest/approvals.rs:53-68` — the route's own module doc reports the *gate* contradiction (`approval × approve`, which no default role that could be a submitter holds) and stops there

The identity half of `inst-as-void` — "the submitter (or a `CatalogAdmin`) explicitly withdraws it" — is enforced at neither layer. What the code actually implements is: any principal holding `approval × approve` may void any `submitted` unit of the tenant. A withdraw is not cosmetic: `approval_repo::void_pending_for_plan`-class state moves the unit out of `submitted`, which releases its held scope keys (`refuse_held_key` selects `submitted` only — `approval.rs:1515-1532`) and re-opens the key to whoever wants it. So a `FinanceReviewer` who cannot approve a change can nonetheless close another principal's review of it, unaudited as an authority act (a void's refusal is not one of `is_an_audited_violation`'s two, `approval.rs:1383`). `judge` is the one place that holds both the record's submitter and the acting stamp, so this is the layer the check belongs in.

*Fix/Verify:* in `judge`, on the `DecisionBy::Void` arm, refuse unless `request.stamp.actor_principal_id == record.submitter_principal` or the caller carries the `CatalogAdmin` warrant the design names — and if the gate contradiction the route reports makes the second half unexpressible, say so *there*, in the service, where the rule is. Probe: a second principal's withdraw of a pending unit is refused, and the held key stays held.

**Z9-4 [Medium] the cutover silently ignores a divergent successor body where the supersession refuses it**
`gears/bss/pricing/pricing/src/infra/supersession.rs:838-839` and `:901-903` and `:1128-1130` — `refuse_divergent_successor(staged, &successor_content)` on all three arms (pending replay, committing, submit)
`gears/bss/pricing/pricing/src/infra/supersession.rs:1212-1225` — the refusal: "a composed unit's content is what a reviewer agreed to, so re-send that content, or withdraw the unit"
`gears/bss/pricing/pricing/src/infra/cutover.rs:934-985` — `stage_both`: `if let Some(staged) = context.staged_successor { staged.clone() } else { insert … request.successor }`; the copy arm is the same shape. There is no comparison anywhere in `cutover.rs` (`grep -n divergent cutover.rs` → nothing)
`gears/bss/pricing/pricing/src/infra/cutover.rs:676-702` — the pending-replay arm returns the *stored* ids and never looks at the request's content either

Not a content-substitution hole: the approved unit's pin is over `context.shape`, whose `rows` include the staged drafts (`publish.rs:1124`), so editing the staged row moves the digest and `authorizing_unit` stops answering. What the cutover does instead is worse to read than a refusal — a caller who edits the successor and re-`POST`s after the approve is answered **202/200 for a commit of the content they no longer asked for**, with nothing in the response saying so. The supersession decided this question and answered it with a 409; the cutover, written from the same skeleton, dropped the guard.

*Fix/Verify:* call `refuse_divergent_successor` (it is already `fn`-shaped and content-generic) on both cutover arms, against `staged_successor` and against `staged_copy`. Probe: a second cutover `POST` with a changed `amountMinor` under a pending unit is a 409 and not a replay.

**PAID `738dc8afe`.** `refuse_divergent_successor` is spent on both cutover arms over both staged rows. RED was this entry's own words: the pending retry answered `SubmittedForApproval` and the approved retry answered `Committed`, both for a staged body that differed from the one the call carried.

**Z9-5 [Medium] the bulk run's terminal move and lock release are covered against `Err` and against nothing else**
`gears/bss/pricing/pricing/src/infra/bulk.rs:146-161` — `advance(… Committing …)` commits, durably
`gears/bss/pricing/pricing/src/infra/bulk.rs:170` — `take_locks(...)` commits, durably
`gears/bss/pricing/pricing/src/infra/bulk.rs:179-197` — `commit_rows(...).await`, then `release_locks(...).await`; each row is its own transaction, so there are `rows.len()+2` await points between the two durable writes
`gears/bss/pricing/pricing/src/infra/bulk.rs:163-168` — the module's own argument: "**Everything from here lands the run terminal and releases its locks, on every path.** §4 offers no failure edge out of `committing` … the `?` that used to sit on `commit_rows` made that false." The remedy taken was removing two `?`s; there is no `Drop` guard, so a panic or a dropped future still exits between the two
`gears/bss/pricing/pricing/src/api/rest/bulk_imports.rs:355-366` — `commit_batch` is awaited inline in the HTTP handler, so a client disconnect drops the future mid-loop

Third abnormal exit of the three, unclosed: the run stays `committing` with its row locks held against every interactive editor, and `bulk_repo::advance`'s state machine has no failure edge out of it. There *is* an operator door — `POST …/bulk-imports/{id}/abort` (`bulk_imports.rs:77`, handler at `abort_bulk_import`), and it is well built (release first, then advance, so a failed abort stays retryable). Which makes the second half of this finding a stale doc that hides the remedy: `bulk.rs:167` says "with no operator remedy until D-37's lease takeover exists, which it does not", while `bulk_imports.rs:32-37` says "§4 already has that edge, which is why abort needs no state of its own — and why a crashed import cannot freeze interactive authoring: the operator has a door." Two module docs in one crate, opposite claims about the same edge. (Another instance of F-6's class.)

*Fix/Verify:* either hold the lock release in a `Drop` guard / `scopeguard`-shaped finaliser, or state in `bulk.rs` that the drop path is deliberately delegated to the abort route — and correct `bulk.rs:167` either way. Probe: drop the `commit_batch` future at an await and assert the run is still recoverable through abort.

**PAID `ceaf1ff16` — and HALF OF THIS ENTRY WAS ALREADY PAID when the fix was dispatched.** `a931f5e3f` (Z8-8) had landed the `Drop` guard AND its cancellation probe two days earlier, in `3de9d0c73`'s shape with a positive control. What was genuinely unpaid was different from what the entry emphasises: a stale `bulk.rs` sentence contradicting `bulk_imports.rs`, and `abandon_committing_run`'s SUCCESS path, which had zero coverage anywhere in the crate. The residual is now pinned rather than papered over — the case waits for a row to actually commit before cancelling, then asserts the store holds it while `report.committed` does not. Still unexercised, deliberately: `commit_batch`'s panic exit, because no seam exists to inject one and adding one to production code for a test is the wrong trade.

**Z9-6 [Medium] the bundle's effective-share write is a blind `update_many` whose zero-row outcome is indistinguishable from success**
`gears/bss/pricing/pricing/src/infra/bundle.rs:687-718` — `write_effective_share`: `bundle_revshare::Entity::update_many() … .exec(runner).await.map_err(…)?; Ok(())`. The `UpdateResult`'s `rows_affected` is discarded
`gears/bss/pricing/pricing/src/infra/bundle.rs:284-297` — the values come from `reconcile(group)` over `draft.rev_share_groups`, read from the composition; a group whose `(bundle, revision, vendor_sku, party)` row is absent or was written under a different revision matches nothing
`gears/bss/pricing/pricing/src/infra/bundle.rs:324-345` — `BundleUpdated` is enqueued regardless, announcing a composition whose reconciled shares were never stored

`reconcile`'s whole job is to turn authored shares into the effective ones a downstream party is paid on; a filter that matches no row leaves the stored `effective_share_bp` at whatever it was, and the act reports success and announces itself. This is the "detector swallows its own signal" shape applied to a writer.

*Fix/Verify:* read `rows_affected` and refuse (`RepoError::CorruptRow` or `NotFound` naming the four-column key) when it is zero; the sibling writers in `price_repo`/`window_repo` all answer on a no-op swap.

**PAID `60ba54b29`.** A share that reconciled onto no row is refused instead of announced. This was the only site in these zones where a MONEY write could silently not happen: zero matched rows left `effective_share_bp` stale, the act returned `Ok(())`, `BundleUpdated` announced it anyway, and downstream parties were paid on that column.

**Z9-7 [Low] three modules spell a domain enum's token as a bare literal, and none of them imports the enum**
`gears/bss/pricing/pricing/src/infra/bundle.rs:75` — `const COVERAGE_ELIGIBILITY: &str = "all_subscriptions";` and `:77` — `const COVERAGE_COHORT: &str = "none";`, used at `:475-476`
`gears/bss/pricing/pricing/src/infra/change_graph.rs:128` — `.add(plan::Column::LifecycleState.eq("published"))`
`gears/bss/pricing/pricing/src/infra/currency_binding.rs:58,61,64` — `CURRENT_PLAN_STATES = &["published","retired"]`, `PUBLISHED = "published"`, `GRANDFATHERED = "existing_grandfathered"`
The owners: `domain/scope_key.rs:248-250` (`PriceEligibility::as_str`), `domain/scope_key.rs:350` (`Cohort::None` → `"none"`), `LifecycleState::as_str`
The siblings that do it right, including one in the *same file*: `bundle.rs:382,444,474` — `.eq(LifecycleState::Published.as_str())`; `price_repo.rs:2609` — `.eq(key.cohort().to_string())`; `price_repo.rs:3035` — `Set(key.cohort().to_string())`
`grep -rn '"all_subscriptions"\|"none"' src/ | grep -v domain/scope_key.rs | grep -v migrations | grep -v _tests` — `bundle.rs:75,77` are the only production sites

Not a live bug: `cohort` is a `String` column holding the token (`entity/price.rs:59`, writer `price_repo.rs:3035`), so the comparisons match today. It is the strongest form of the tell — a module that imports the *neighbouring* enum and still hard-codes this one — and the failure mode is silent: a rename matches zero rows, `component_rows` answers empty, and every bundle coverage rule passes vacuously with green tests.

*Fix/Verify:* `PriceEligibility::AllSubscriptions.as_str()`, `Cohort::None.to_string()`, `LifecycleState::Published.as_str()` at all six sites.

**PAID `b43b909ad` (with Z11-9 — the same two literals), but THE ENTRY'S FAILURE MODE DOES NOT HOLD.** The predicted silent failure is unreachable: two CHECK constraints write both tokens into the schema, so a rename is refused at every insert — proved by trying it, which killed seven cases in the seeder — and the coverage check would fail loudly rather than vacuously. The substitution is kept on one-owner grounds, not on the entry's stated risk.

**Z9-8 [Low] `window.rs`'s error contracts name `DomainError::ServiceUnavailable`, a variant that does not exist**
`gears/bss/pricing/pricing/src/infra/window.rs:380`, `:501`, `:580` — "[`DomainError::ServiceUnavailable`] when the registry cannot assign"
`gears/bss/pricing/pricing/src/domain/error.rs:706` — the variant is `CatalogVersionUnavailable(String)`; `grep -n "ServiceUnavailable" src/` returns only those three doc lines
`gears/bss/pricing/pricing/src/infra/error_mapping.rs:498-501` — `CatalogVersionUnavailable` is what maps to 503

Three dangling intra-doc links on the `# Errors` contract of the zone's three most-called public methods, and the sibling contracts get it right (`publish.rs:453` names `CatalogVersionUnavailable`). Another instance of F-8's class (error doc names a non-producer).

*Fix/Verify:* rename the three references; consider `#![deny(rustdoc::broken_intra_doc_links)]` for the crate, which would have caught all three.

**PAID `28475be90`**, docs — the named `DomainError` variant does not exist.

**Z9-9 [Low] two module docs assert a fact their own code contradicts**
`gears/bss/pricing/pricing/src/infra/window.rs:311-313` — `PendingApproval::verdict`: "Why a second principal is required — **always** [`MaterialityReason::AlwaysMaterialTrigger`] here"
`gears/bss/pricing/pricing/src/infra/window.rs:1445-1451` — the producer, 1100 lines down: "**The evaluator's verdict, carried rather than re-minted.** It used to be `material(AlwaysMaterialTrigger)`, written here on the ground that D-62 needs no threshold — true of a cancel and a shortening, and **false of the other two acts**"
`gears/bss/pricing/pricing/src/infra/jobs.rs:3` — "**Two, and they are independent**", enumerating `readmodel_warm` and `window_activation`
`gears/bss/pricing/pricing/src/infra/jobs.rs:38-40` — `pub mod gated_markets; pub mod readmodel_warm; pub mod window_activation;` — three, and `jobs/gated_markets.rs:1` is "The gated-market gauge's refresher — D-246's missing half, on D-250's tick"
The same `jobs.rs` doc block, at `:22-24`, makes exactly this argument about itself: "A count in prose beside a roster in code is the shape that goes stale, so the count is gone rather than corrected" — and the count survived at the top of the paragraph

F-6/F-18's class, two more instances. The first is the more consequential: an operator reading the field doc believes every window unit says `alwaysMaterialTrigger`, which is the one thing they cannot act on when the real reason is a currency with no threshold entry.

*Fix/Verify:* delete the "always" clause and the "Two"; the surviving sentences are already correct.

**PAID `28475be90`** for this module; the sibling half was paid by a concurrent lane as `66a455f10`. This shape — a module doc asserting a fact its own code contradicts — has now been recorded in **five** different modules in this register.

**Z9-10 [Low] the retirement's registry request id carries no act discriminator, alone among the five**
`gears/bss/pricing/pricing/src/infra/retirement.rs:705-707` — `format!("{tenant_id}/{subject_ref}")`
The siblings: `publish.rs:968` `"{kind_token}/{tenant}/{plan}/{revision}"`; `supersession.rs:1372` `"supersession/{tenant}/{subject}"`; `cutover.rs:992` `"cutover/{tenant}/{subject}"`; `window.rs:1766` `"{kind_token}/{tenant}/{plan}/{revision}/{act}"`

No collision exists today — the subject already carries `/retirement/` (`approval.rs:2055`) — so this is naming rather than a defect. It is the one id whose uniqueness rests on a segment of a string built elsewhere rather than on a prefix of its own, and the registry is a cross-tenant service whose idempotency is keyed on exactly this value.

*Fix/Verify:* `format!("retirement/{tenant_id}/{subject_ref}")`, and note that re-spelling strands any outstanding pending ref (the same re-freeze question `PublishUnitKind::request_token` carries).

**Verified clean:**

- **`infra/idempotent.rs` — the at-most-once seam.** Claim, mutation, render and `record_response` are all inside one `in_transaction` (`:166-209`); the `Replay` arm returns *before* the mutation and before `record_response` (`:186-191`), so neither runs twice. The key is `(tenant_id, operation, client_key)` with `operation` a per-route `&'static str` (`:113-118`) — decomposed component by component, it is not narrower than the operation's identity, and a client key reused across verbs cannot collide. `tx_failure` (`:220`) preserves a typed refusal through the rollback and reserves `Internal` for BEGIN/COMMIT alone, which `idempotent_tests.rs:20,27` locks in both directions. No hand-rolled dedup was found anywhere else in the zone.
- **`infra/error_mapping.rs` — the single ladder.** The `match` on `DomainError` (`:87-509`) has **no wildcard arm** (`grep -n "_ =>" ` → nothing) so a new variant is a compile error rather than a silent 500; 503 details are logged and dropped (`:498-505`); the two authority refusals drop their detail deliberately (`:426-431`); an empty `ValidationFailed` report is an internal fault rather than a contentless 400 (`:462-471`). `error_mapping_tests.rs` walks `WindowRefusal::ALL` (`:430`) rather than asserting one arm.
- **`infra/publish.rs::commit` — the ordering.** Re-assemble → pin compare (`:521-533`, inside the transaction, ahead of the rules, with the stated reason) → rules → fixtures → the one *permanent* refusal hoisted ahead of the registry (`:559`, `refuse_unpublishable_predecessor`) → registry → flips → pending ref → event → audit → void the orphan units (`:692`). No refuse-check is reachable only after a durable write. `validated_draft_rows` (`:764-782`) excludes a supersession successor staged on an occupied key, which is the one arrival that would otherwise take an unrelated publish down with a 500.
- **`infra/approval.rs::judge` — the refusal path.** The `deny` record is written inside the judging transaction (`:1383-1385`) and a failure to write it rolls the judgement back, which is the fail-closed direction; the pin is compared only on `Approve` and a re-derivation that answered `None` is a mismatch (`domain/approval/decision.rs:356-364`), so a vanished subject cannot be approved. `refusal_to_domain` (`:2120`) matches the enum exhaustively and `approval_tests.rs:240` walks `DecisionRefusal::ALL`.
- **`infra/window.rs::mutate_in` — every refusal before the first write.** Steps 2, 3, 3a (`:1127`, `:1132-1135`, `:1153-1219`) all precede the registry request at `:1223` and the row change at `:1230`; the controlled arm returns having written only the approval unit. The unit's subject is the **act** — `unit_subject_ref` (`:944-963`) carries the operation, the act sequence the window was read at, and the prior→new transition, so an approve taken for a lengthening cannot authorize a cancel, and a genuine retry renders the same string (`read_at_seq`, `:971`). `parse_unit_subject` (`:874`) is the exact inverse and lives beside it.
- **`infra/supersession.rs` and `infra/cutover.rs` — "the approved unit decides before the evaluator".** `supersession.rs:821-828` / `cutover.rs:644-651`, with the defect the order closes written down at both sites: consulting the evaluator first let a threshold raised between the two calls make an already-reviewed act auto-publishable at the second call. Both then take `refuse_held_key` on the **committing** arm too (`supersession.rs:892`, `cutover.rs:729`), which is the gap the 2026-08-06 review found and it is genuinely closed.
- **`infra/threshold.rs::effective_version_at`.** A version whose `effective_from` is ahead of `now` is a `continue`, not a `break` (`:233`), so a future-dated proposal cannot take away the policy a tenant already has; and the approved record is put through `independent_approver` (`:253`) rather than accepted on `is_some()`, which is the asymmetry that used to run the wrong way on the one reader that decides whether every *other* change needs a reviewer.
- **`infra/read_model.rs` — the projector.** `refuse_projection_below_frontier` runs before `finalize` inside each subject's own transaction (`:452-468`); a per-subject fault is counted and logged rather than abandoning the version (`:396-406`, D-91); and completeness is gated twice on `PassCoverage::may_decide_completion()` — once for the report (`:423`) and once at the place it has a consequence, ahead of `advance_frontier` (`:538-543`).
- **`infra/history.rs` — the cursor.** `decode` (`:227-248`) fixes the total length as well as each field (`split_first_chunk::<8>`, `::<4>`, then `try_into::<[u8;16]>`), so a longer token is refused rather than truncated, and an unrepresentable instant is refused rather than clamped; the refusal is `InvalidRequest` and mints no code (`history_tests.rs:68`).
- **`assemble_from` and `clone_plan_on` destructure `PlanRevision` by name** (`publish.rs:1043-1070`, `clone.rs:285-305`), so a field added to the revision and forgotten is a compile error — the D-259 remedy, applied at both sites that rebuild a revision from another.
- **`infra/overlay_publish.rs::commit`** runs the pin compare and §4.2's second validation run on the transaction runner and before the registry request (`:200-263`), resolves the shards before the flip moves the predecessor (`:267`), and audits through `overlay_repo::publish_revision_on` (`overlay_repo.rs:928`), so it is not a second instance of Z9-1.
- **`infra/fixture_gate.rs`** fails closed on an unreadable registry (`:196-208` → `Self::closed()`), and a row with no `model_kind` is `FIXTURE_MISSING` rather than an inferred kind (`:284-290`).
- **No sign/direction defect found.** `PriceUpdatedPayload`, `PriceWindowTransitionPayload` and `PlanPublishedPayload` carry no signed money; the only threshold comparison in the zone is `after_end >= until` on instants (`window.rs:1572`), where signedness is not a question, and the materiality bars are the domain's, not this layer's.

**Refutations:**

- *Suspected:* the window unit's plan-shape pin cannot see a sibling window moving between the approve and the re-issued `PATCH`, so an approved act could commit against a changed interval set. **Refuted:** `PlanShape::windows` is in the pin preimage since `v2` (`domain/approval/content_pin.rs:238-251`, framed by `put_key_windows` at `:826`), and `assemble_from` fills it (`publish.rs:1127`). Any window movement on the plan moves the digest and `authorizing_unit` stops answering.
- *Suspected:* `authorization.pinned_content_hash()` returning `None` (`publish.rs:521`, `overlay_publish.rs:200`) skips the approve→commit check. **Refuted:** `None` is returned only on `PublishAuthorization::AutoPublishable` (`domain/publish.rs:309-314`), where by construction no reviewer was shown anything; the `Approved` arm makes `content_hash` a non-optional member (`:263`).
- *Suspected:* `bundle.rs`'s `.eq("none")` on the cohort column would match nothing if `none` were stored as `NULL`, silently emptying every bundle coverage read. **Refuted:** `entity/price.rs:59` is `cohort: String` and the writer is `price_repo.rs:3035` `Set(key.cohort().to_string())` → `"none"` (`domain/scope_key.rs:350`). Fragile (Z9-7), not broken.
- *Suspected:* an overlay approval unit's `subject_ref` leads with a UUID, so `subject_plan` would parse it as a `PlanId` and file the overlay's `deny` record on some plan's audit chain. **Refuted:** `approval_repo::subject_aggregate` routes `AuditSubjectKind::Overlay` to `subject_overlay` → `SubjectAggregate::Overlay` (`approval_repo.rs:1211`), whose `chain_id()` is `overlay_chain` (`:1138`) and whose `plan()` is deliberately `None` (`:1157`, with the "subject with a plan-shaped id" argument written out).
- *Suspected:* bulk Phase 2 never re-asks `inst-co-single-pending`, so a unit opened between Phase 1's `key_holds_a_pending_unit` (`import.rs:147`) and the commit lets an import write onto a reviewed key. **Refuted as a hole:** every draft write goes through `price_repo::record_price_mutation`, which calls `approval::void_pending_units_of` first (`price_repo.rs:2921-2928`). The rail voids the unit rather than racing it, which is the same answer the interactive authoring surfaces give.
- *Suspected:* `migration::schedule_in`'s replay arm reads by id with no state predicate (`migration.rs:259`), so a re-`POST` after a cancel would be answered as though scheduled — catalog item 4. **Refuted as severe:** the id is client-supplied and the arm returns `created: false` with the *stored* record, whose `state` the view renders; the caller is told `cancelled` rather than told a lie. Worth a look if the view ever drops the state field.
- *Suspected:* `refuse_trailing_void` compares a magnitude where a signed value could slip the gate. **Refuted:** both sides are `DateTime<Utc>` and the `OpenEnded` case is handled before any comparison (`window.rs:1552-1567`); `CoverageEnd` is a three-valued type precisely so `Uncovered` cannot collapse into `Ends(min)`.

**Not covered:** `storage.rs`, `storage/**` (schema, entities, migrations, repositories) beyond the specific functions cited as evidence; `jobs/**` internals — `jobs.rs`'s roster was read, the three job bodies were not; `metrics.rs` / `metrics_tests.rs`; `api/rest/**` except the handlers cited (`bulk_imports`, `cutovers`, `approvals`, `migrated_origin_snapshots`, `publish`); `domain/**` except the rules a service claim depended on (`decision.rs`, `content_pin.rs`, `publish.rs`, `synthesis.rs`, `scope_key.rs`); the `tests/postgres_*` and most `tests/sqlite_*` suites were not read line by line — `rest_cutovers.rs` and `rest_migrated_origin_snapshots.rs` were read for the two findings that turn on what is *not* asserted. Nothing was built, run or edited: the tree was read at `6ae81d5ec` with `git`, `grep`, `awk` and `Read` only.


---

**Zone Z10 — jobs, metrics, outbox projector**

**Files:** `src/infra/jobs.rs`; `src/infra/jobs/{gated_markets.rs, gated_markets_tests.rs, readmodel_warm.rs, readmodel_warm_tests.rs, window_activation.rs, window_activation_tests.rs}`; `src/infra/metrics.rs` + `metrics_tests.rs`; `src/domain/ports/metrics.rs`; `src/domain/events.rs`; `src/infra/storage/repo/outbox_repo.rs`; the scheduling/wiring in `src/module.rs:60-542` and `:678-796`; `src/config.rs:100-336` (`JobsConfig`); the coordination primitive `gears/bss/libs/coord/src/lease/{manager.rs,guard.rs}`; the sibling `gears/bss/ledger/ledger/src/module.rs:460-545` and `src/infra/{period_close.rs,recognition/run_service.rs}`; the suites `tests/sqlite_window_activation.rs`, `tests/postgres_window_activation.rs`, `tests/module_test.rs`; the reads the jobs make (`price_repo::gated_markets`, `window_repo::list_due`, `catalog_version_ref_repo::pending_tenants`, `pin_frontier_repo::list_all`).

**What's done:** every ticker under `src/infra/jobs/` enumerated (three: `gated_markets`, `readmodel_warm`, `window_activation`) and read whole with its unit tests; each compared against the other two on the six axes the method names (leasing primitive, batch bound, error isolation, alarm shape, metric shape, cleanup placement); the whole `module.rs` scheduling plane read; the coord lease library read to settle fencing/renewal/release semantics; the ledger's tickers read as the named sibling; the outbox enqueue path read end to end and every `CatalogEvent` variant grepped for a producer; the metrics plane traced from `PricingMetricsPort` through the OTel adapter to the process-global provider the host installs.

**Verdict:** the correctness core is sound — no cross-tenant reach, no financial-integrity break, no Err-only cleanup, no mutate-then-await without rollback. What is wrong is the **observability and supervision plane around the tickers**, and it is wrong in exactly the two shapes the method predicts: a fix (D-238) carried to two alarm sites and not to the third, and a third ticker whose arrival left the plane it joined un-updated in more places than F-6 names. Twelve findings, four Medium, eight Low; no Critical, no High.

---

**Findings**

**PAID `8bc26ce94`**, three probes.

**Z10-1 [Medium] The window sweep's Warn alarm is the third instance of the defect D-238 was opened to close, and its two justifying sentences still assert the alarm plane does not exist**

`src/infra/jobs/window_activation.rs:49-58` — module doc: *"`pricing.window.activation_overdue` (Warn, §7) is the design set's string… **this gear has no metrics or alarm facility at all** — the sibling ledger has `infra/metrics.rs` and an event publisher with an alarm catalogue, and this crate has neither — so a `tracing::error!` under the named string is the whole of it. **Reported as a gap**"*
`src/infra/jobs/window_activation.rs:265-268` — `pub struct WindowActivationJob { db, jobs }`: no metrics field, and `module.rs:378` builds it with no `.with_metrics`.
`src/infra/jobs/window_activation.rs:490-501` — the alarm is a bare `tracing::error!(alarm = ALARM_WINDOW_ACTIVATION_OVERDUE, …)`; `report.overdue += 1` and nothing else.
`src/domain/ports/metrics.rs:216-221` — `PricingAlarm::ALL` holds four names; `pricing.window.activation_overdue` is not among them.
`src/infra/jobs/window_activation_tests.rs:48-50` — *"this gear has no alarm facility for a second spelling to fail against"*.
`src/infra/jobs/readmodel_warm.rs:562-563` and `:821-822` — the two Criticals go through `self.metrics.alarm(…)` beside their log line.
`docs/DECISIONS.md:2184-2190` — D-238's own reasoning: *"the wiring is now one call, and **a Critical that an operator can only find by grepping logs is the state the plane was opened to end**"*.

`infra/metrics.rs` and `domain/ports/metrics.rs` exist; `readmodel_warm` was corrected on 2026-08-07 (`b0516ea83`); `gated_markets` was born holding a `PricingMetricsPort`. `window_activation` is the one ticker of three that holds no port, and its module doc and its unit test both still carry the falsified premise D-238 records as the cause. Consequence: §7's only window-plane alarm — the one whose text is *"the lease singleton is stalled"*, i.e. the signal that the whole activation plane has stopped — reaches `pricing_alarm_total` on no label and therefore reaches no alerting rule. D-238 is filed as `BUILT`, which makes this a closed decision with a live third instance rather than owed work anybody is looking for.
*Fix/Verify:* add `WindowActivationOverdue` (Warn) to `PricingAlarm`, give `WindowActivationJob` the `with_metrics` seam its two siblings have, wire it at `module.rs:378`, and delete the two "no facility" sentences at `window_activation.rs:52-58` and `window_activation_tests.rs:48-50`. Verify by asserting `counter_value("pricing_alarm_total", &[("alarm","pricing.window.activation_overdue")])` in the `MetricsHarness` after a pass with an overdue boundary — the harness already reads the exported stream (`metrics.rs:335-341`).

**PAID `f370e8933` (2026-08-11) — this mark was MISSING until the 2026-08-14 audit.** The four Z10 Mediums were paid by a concurrent session before the annotated programme began, and its edit marking them was never committed, so this register carried them as open for three days. Residue the audit found: this entry's own verify recipe is unfulfilled — no test asserts `pricing_alarm_total` moves for `pricing.window.activation_overdue`, because every suite builds the job with the no-op metrics adapter. The attachment is structural in `module.rs`, so it is not a silent risk, but the emission is unobserved end to end.

**Z10-2 [Medium] The gated-markets ticker drops its lease guard where its two siblings release it, so the gauge refreshes on every other tick — half the cadence D-250 decided**

`src/module.rs:443-460` — `gated_markets_pass`: `let Some(guard) = Self::take_lease(lease, GATED_MARKETS_LEASE_KEY, ttl)`, run, then `drop(guard);`
`src/module.rs:302-306` — `warm_pass`: `if let Err(e) = guard.release().await { … }`
`src/module.rs:478-485` — `activation_pass`: the same `guard.release().await`.
`gears/bss/libs/coord/src/lease/guard.rs:38-41` — *"Always release explicitly — **there is no `Drop` impl** performing async DB I/O… so a guard dropped without a `release` / `release_with_retry` relies on the TTL fallback to free the slot."* (grep for `impl Drop` in that file returns only `RenewalHandle`.)
`src/module.rs:411` and `:425` — the TTL passed is `rt.config.jobs.gated_markets_interval()`, i.e. the tick itself (60s, `config.rs:217`).
`gears/bss/libs/coord/src/lease/manager.rs:127` and `:150` — a row whose `locked_until` has not yet passed takes the `Some(_) => Err(CoordError::LeaseHeld)` arm.

The slot is claimed at `T+δ` with `locked_until = NOW()+60`, and dropping leaves it standing. The next tick fires at `T+60 < T+δ+60`, so `acquire` returns `LeaseHeld`, `take_lease` logs *"sweep skipped (a peer holds its lease)"* at **debug** — naming a peer where the holder is this same task's previous pass — and the refresh is skipped. The pass after that succeeds. So `pricing_tax_not_sellable_ga` is refreshed at ~120s while `gated_markets.rs:31-38` states the trade as *"the value is up to one tick old"* and D-250 (`DECISIONS.md:2348-2356`) ratifies 60s. Not a correctness break — the quantity moves in months — but it is a decided cadence the code does not deliver, invisible behind a debug line that misattributes the cause, and an exact outlier among three look-alike passes.
*Fix/Verify:* `guard.release().await` with the siblings' warn-on-error arm. Verify with a two-pass test over one `LeaseManager` and a fake clock, asserting the second pass acquires (there is no such test today — see Z10-12).

**PAID `f370e8933` (2026-08-11) — mark missing until the 2026-08-14 audit; see Z10-1.** `release_lease` verified present and probed.

**Z10-3 [Medium] A panicking ticker is silently dead for the process lifetime; `serve` returns `Ok`, and the sibling whose shape this file claims to copy `select!`s on the join handles precisely to prevent it**

`src/module.rs:181-189` — `let warm = Self::spawn_warm_ticker(…); … cancel.cancelled().await; tasks.cancel(); Self::stop(warm, activation, gated).await; Ok(())`. The handles are awaited **only after** shutdown.
`src/module.rs:219-223` — `join_ticker`: `if let Err(e) = handle.await { tracing::warn!(error = %e, ticker, "a ticker did not join cleanly"); }` — a panic is demoted to a warn and discarded.
`src/module.rs:166-168` — the doc claims the opposite: *"Never returns `Err` today; the signature is the lifecycle contract's, and **a spawned ticker's join error will surface through it**."*
`src/module.rs:192-195` — *"A join error is a **panic** in a sweep, and it is **reported rather than swallowed**"*.
`gears/bss/ledger/ledger/src/module.rs:533-540` — the named sibling: *"`select!` on the join handles (not `join!`): **a panic in one ticker would otherwise stay invisible for up to a full tick.** Each arm cancels the shared token, awaits the survivors, and maps a join error (panic / abort) to `anyhow`."* — feeding `let serve_result: anyhow::Result<()>`.
`src/module.rs:228-234` — pricing's doc: *"The sibling ledger's `spawn_*_ticker` shape, and its three properties are all load-bearing here."*

The three properties were inherited; the supervision the sibling wraps them in was not. `module.rs:154-161` states the warm ticker is load-bearing for correctness (*"without the warm re-drive nothing ever resolves it, `pricing_read_model` stays empty and no version becomes pin-eligible"*), so a panic on tick 1 leaves the gear serving traffic, answering 200s, and never warming a read model — with the only trace a warn line emitted at shutdown, possibly days later, and `serve` still `Ok(())`. There is no supervision, no restart and no alarm; the two Criticals in `readmodel_warm` cannot fire because the task that raises them is the dead one.
*Fix/Verify:* adopt the ledger's `select!`-on-handles shape (cancel the token, drain the survivors, map the join error to `Err`), or at minimum raise a Critical alarm and log at `error` on a join failure. Verify with a ticker whose job panics once, asserting `serve` resolves `Err` (or that the alarm counter moved).

**PAID `f370e8933` (2026-08-11) — mark missing until the 2026-08-14 audit; see Z10-1.** A ticker resolving before cancellation fails `serve`, including the no-panic case.

**Z10-4 [Medium] The gated-market gauge inlines `TAX_ENGINE_GA == false` into SQL, so the flag's documented flip has no compile gate at the one site that decides the gauge**

`src/domain/tax_display.rs:217-221` — *"It flips **in code** when the engine ships"* — `pub const TAX_ENGINE_GA: bool = false;`
`src/domain/tax_display.rs:229-232` — `pub const fn is_not_sellable_ga(row, tax_engine_ga) -> bool { row.tax_inclusive && !tax_engine_ga }`
`src/infra/metrics.rs:298-311` — the publish-path alarm calls `is_not_sellable_ga(record, TAX_ENGINE_GA)`, i.e. reads the constant.
`src/infra/storage/repo/price_repo.rs:1705-1713` — the gauge's read instead *reasons about* the constant in prose: *"`TAX_ENGINE_GA` is a compile-time `false` — so a gated market is exactly a market carrying a **published, non-grandfathered, tax-inclusive** row"*.
`src/infra/storage/repo/price_repo.rs:1735-1746` — and the query carries only `lifecycle_state = published AND tax_inclusive = true AND price_eligibility <> existing_grandfathered`. `TAX_ENGINE_GA` appears nowhere in the file.

The two consumers of one predicate diverge the moment the constant moves. On the day GA lands, `report_market_metrics` correctly stops raising `TaxNotSellableGaActive` while `GatedMarketsJob` keeps publishing the full count of published tax-inclusive tenant-markets to `pricing_tax_not_sellable_ga` forever — §7's backlog gauge pinned at a number no action can clear, and the whole point of the D-246 rebuild was that this series must be honest. It compiles, and every test stays green, because the tests exercise the plumbing rather than the predicate (`gated_markets_tests.rs:45-52` deliberately seeds no rows). This is the "flag flip needs a compile gate" shape: whoever flips the constant greps for `TAX_ENGINE_GA`, finds `tax_display.rs` and `metrics.rs`, and does not find `price_repo.rs`.
*Fix/Verify:* make the read take `tax_engine_ga: bool` (or short-circuit to `Ok(0)` on `TAX_ENGINE_GA`), so the site is reachable from the constant. Verify by flipping the constant in a `#[cfg(test)]` shim and asserting the count falls to zero.

**PAID `f370e8933` (2026-08-11) — mark missing until the 2026-08-14 audit; see Z10-1.** The GA flag is a parameter, so the site is reachable from the constant.

**Z10-5 [Low→Medium] The pin-eligibility Critical swallows its own frontier read: an `if let Ok` with no log, and the whole stale-frontier arm silently disappears**

`src/infra/jobs/readmodel_warm.rs:525-531`
```rust
if let Ok(frontiers) =
    pin_frontier_repo::list_all(conn, &AccessScope::allow_all(), FRONTIER_SCAN_LIMIT).await
{
    for (tenant_id, frontier) in frontiers { tenants.insert(tenant_id, Some(frontier.advanced_at)); }
}
```
`src/infra/jobs/readmodel_warm.rs:511-512` — the doc for the same method: *"The first half is what finds a tenant whose frontier is stale **precisely because nothing of it has moved**."*
`src/infra/jobs/readmodel_warm.rs:430-438` — the sibling read in the same file, `read_pending`, logs at `tracing::error!` on failure and says the pass continues.

On a storage failure the `Err` is dropped without a log or a counter, and the tenant map degenerates to *only tenants that have a pending ref this pass*. That is exactly the population the method's own doc says the frontier read exists to go beyond, so the Critical's most important arm goes silent with no trace whatsoever — the pass reports `pin_eligibility_overdue: 0`, which reads as "healthy". `frontier_is_blocked` (`:878-891`) makes the same `Ok`-only choice but documents it (*"an alarm that cannot read must not become a second fault, and the pass has already alarmed on the ref itself"*) — an argument that does not transfer to this call, because here there is no other alarm covering the gap.
*Fix/Verify:* match the `Err` and log at `error` as `read_pending` does; consider surfacing it on `SweepReport`. Verify with an unmigrated store, asserting the log/report rather than the silence.

**PAID `5ce7feb1a`.** The `Err` is matched, logged at `error`, and surfaced on a new `SweepReport::frontier_scan_failed`. **The RED is the finding itself:** a pass whose frontier read blew up produced a report BYTE-IDENTICAL to a healthy one, every counter equal — which is also why a report field is part of the fix rather than a log alone, since this crate has no tracing capture and nothing else was assertable. Deliberately NOT done: no `alarm =` label (it would report the Critical as raised when it is merely unevaluable) and no new `PricingAlarm` variant, because roster names belong to the design set.

**Z10-6 [Low] The cadence-inside-threshold invariant is validated for one of the two alarm-bearing tickers**

`src/config.rs:328-333` — `if self.window_activation_tick_secs >= self.window_activation_overdue_secs { return Err(CadenceNotInsideThreshold { … }) }`
`src/config.rs:318-327` — its reasoning: *"The same pathology as the line above, **at every value rather than only at zero**… Refusing only the zero left every such pair accepted — 600s/300s among them."*
`src/config.rs:268-317` — the warm ticker's three knobs (`readmodel_warm_tick_secs`, `catalog_version_overdue_secs`, `readmodel_degraded_after_secs`) are each checked for `== 0` and against nothing else.
`src/config.rs:210-212` — the defaults are `readmodel_warm_tick_secs: 5` and `readmodel_degraded_after_secs: 5`, i.e. **equal** out of the box.

The paragraph at `:318-327` describes a general rule — an age-threshold read off a periodic pass is meaningless once the cadence reaches the threshold — and applies it to one pair. `readmodel_degraded_after` is measured from `commit_observed_at`, which a pass stamps, and is then re-evaluated by the *next* pass; at `tick == threshold` there is zero slack, so any transient projection failure emits `PlanPublishDegraded` on the very next tick. Defensible against §1.2's 5s SLO, but it is an undeclared relation where its sibling is a declared and enforced one.
*Fix/Verify:* extend `CadenceNotInsideThreshold` to `readmodel_warm_tick_secs` vs `readmodel_degraded_after_secs` (and state the intended relation for `catalog_version_overdue_secs`), or record in `config.rs` why the warm pair is exempt. Verify in `config_tests.rs` beside the existing window case.

**PAID `47812c58c` — THE ENTRY NAMED THE WRONG PAIR, and the invariant as written would have refused the shipped configuration.** The two values it pairs both default to the same number, so asserting one strictly inside the other rejects the default config while protecting nothing: that clock is stamped by the sweep in the very pass that then projects. The pathology transfers to `catalog_version_overdue_secs`, whose clock a **publish** starts — that arm is what was built, with the exemption recorded and a case pinning the equal defaults. Right class of defect, wrong object.

**Z10-7 [Low] All three leases set TTL = tick, never renew and never fence, where the platform's own primitive offers both and the named sibling uses them**

`src/module.rs:282`, `:387`, `:425` — the TTL argument is always `…_interval()`, the tick itself: 5s (warm), 60s (window), 60s (gated).
`src/module.rs:254-258` — the argument: *"the TTL only bounds how long a **crashed** holder blocks its peers, and holding a slot for longer than the cadence would stall the sweep."*
`gears/bss/libs/coord/src/lease/guard.rs:266-276` — `spawn_renewal(period)`, with the convention *"`period` should be `~ttl / 3`"* and an in-band `RenewalState::Lost` signal.
`gears/bss/libs/coord/src/lease/guard.rs:190-264` — `with_ack_in_tx`, the write-fence that turns a mid-flight steal into `AckError::LeaseLost`.
`gears/bss/ledger/ledger/src/infra/recognition/run_service.rs:55-57,154,175` — `RECOGNITION_LEASE_TTL = 1 min`, `RECOGNITION_LEASE_RENEW = 20 s`, `guard.spawn_renewal(RECOGNITION_LEASE_RENEW)`.
`gears/bss/ledger/ledger/src/infra/period_close.rs:115-117,246,262` — `2 min` / `40 s`, likewise renewed.
`src/infra/jobs/readmodel_warm.rs:326-353` + `catalog_version_ref_repo.rs:510-539` — one warm pass issues 1 + up to 250 tenant-discovery queries + 2 pending reads per tenant + one registry round trip per distinct handle, against a **5-second** TTL.

Neither of pricing's two seams is used, so a pass that outruns its TTL — routine for the warm sweep at these numbers — loses the lease with no signal, keeps writing, and its terminal `guard.release()` matches zero rows and logs coord's *"row was likely stolen before release"* warn. The module doc argues concurrency is harmless because every write is key- or predicate-guarded, and I could not falsify that argument for the writes on these paths; what is missing is that the argument is hand-maintained where the primitive offers a mechanical guarantee the sibling takes.
*Fix/Verify:* either set TTL to a multiple of the tick with `spawn_renewal(ttl/3)` as the ledger does, or record in the module doc that renewal and the fence are declined deliberately and that a lost lease is provably safe on each path.

**RESOLVED AS PROSE `93c042391` — OWED A DECISION, not closed.** Whether this gear must renew and fence its leases, or may keep TTL = tick, is not stated by the design set; both readings are written at the declaration with citations. Deliberately not invented, on the same ground that kept two other producers dormant this week.

**Z10-8 [Low] Further instances of F-6: the third ticker's arrival did not update the scheduling plane's prose or its startup log**

`src/infra/jobs.rs:3` — *"**Two, and they are independent**"*, and the bullet list at `:9-30` names two of three (`gated_markets` is absent). *(This is F-6 itself — listed only as the anchor.)*
`src/module.rs:154-161` — *"Three tickers… **Neither** is optional, for **two** different reasons"* — while `:399-403` says of the third *"unlike the other two it is **not** load-bearing for correctness"*.
`src/module.rs:175-179` — the startup `info!` logs `warm_tick_secs` and `window_activation_tick_secs`; `gated_markets_tick_secs` is not logged. (Contrast `gears/bss/ledger/ledger/src/module.rs:517-528`, which logs all nine.)
`src/module.rs:191` / `:198` — *"Wind **both** tickers down"*, *"**Both** are joined"* — over a three-argument `stop`.
`src/module.rs:214-218` / `:513-517` — *"One function for **both**"* (twice, `join_ticker` and `take_lease`) — over three callers each.

Same class as F-6, not re-filed; recorded because the startup log is the operator-facing half and is the one that cannot be read as prose drift: a deployment cannot see the gated-markets cadence it is running.

**PAID `66a455f10`, and the entry was short by two.** The startup log now carries the third tick interval — the operator-facing survivor — and seven prose sites were corrected rather than six: it missed the module doc's `stateful` capability clause and a sentence claiming the gear drives *two* independent leased tickers.

**Z10-9 [Low] "thirteen names" survives in three places after the fourteenth landed (stale-count class, F-18)**

`src/infra/storage/repo/outbox_repo.rs:46` — *"`chk_pricing_outbox_event_name` pins the same **thirteen** names"*
`src/infra/storage/repo/outbox_repo.rs:155` — *"Which of the **thirteen** frozen names."*
`src/infra/storage/repo/outbox_repo.rs:726` — *"pinned to **thirteen** values by `chk_pricing_outbox_event_name`"*
`src/infra/storage/migrations/m20260802_000009_create_pricing_outbox.rs:13` — *"the same **thirteen** names"*
`src/domain/events.rs:62-66` — `PriceOverlayPublished` … *"The **fourteenth** name"* (D-248)
`src/infra/storage/migrations/m20260802_000060_add_price_overlay_published_event_name.rs:47-51` — the CHECK now admits fourteen.

**PAID by a concurrent lane (see Z8-10). CORRECTION, 2026-08-14 audit: THE CLAIM THAT BOTH NUMBERS WERE STALE WAS ITSELF WRONG, and it contradicted this register's own Z8-10 note.** Re-derived independently: `CatalogEvent::ALL` is fourteen and the CHECK in force is the same fourteen — exactly what this entry said. What was stale was "thirteen" and "three places": the entry listed four sites and there were five. All five sites are closed, including one this entry did not name.

**Z10-10 [Low] `pending_tenants` is an N+1 keyset walk: up to 250 single-row queries per 5-second tick**

`src/infra/storage/repo/catalog_version_ref_repo.rs:510-539` — a `while` loop issuing one `find().limit(1)` per tenant, advancing a `TenantId.gt(above)` cursor.
`src/infra/storage/repo/pin_frontier_repo.rs:173-185` — the sibling catalog-wide enumeration in the same pass is one query with `.limit(limit)`.
`src/config.rs:210,214` — `readmodel_warm_tick_secs: 5`, `pending_tenants_per_pass: 250`.

At the defaults this is up to 250 round trips every 5 seconds to enumerate distinct tenant ids — before the two pending reads per tenant and the registry calls — and it is the largest contributor to the warm pass outrunning its 5-second lease TTL (Z10-7). The keyset walk exists because `SecureSelect` exposes no `distinct`; the same constraint applies to `pin_frontier_repo::list_all`, which solves it differently.
*Fix/Verify:* page the walk (fetch `limit` rows per query and dedup in memory, as `price_repo::gated_markets` does at `:1759-1762`) rather than one row per query.

**PAID `4fb8798d3`** — the keyset walk is paged, measured at **12 → 2 statements**. Counted, not estimated: a probe over a handful of tenants cannot distinguish an N+1 from a page.

**Z10-11 [Low] `gated_markets` is the only unbounded read on any ticker**

`src/infra/storage/repo/price_repo.rs:1735-1749` — `price::Entity::find()… .all(runner)` with **no `.limit()`**, loading every published non-grandfathered tax-inclusive `price::Model` in the catalog into memory before deduplicating.
`src/infra/storage/repo/price_repo.rs:1756-1758` — the stated bound: *"The cost is bounded by the thing being measured — the gated rows **are** the backlog."*
`src/infra/jobs/readmodel_warm.rs:160-163` (`FRONTIER_SCAN_LIMIT = 1_000`), `src/infra/jobs/window_activation.rs:202` (`DUE_WINDOWS_PER_PASS = 1_000`), `src/config.rs:213-214` (`pending_refs_per_tenant`, `pending_tenants_per_pass`) — every other job read carries an explicit bound and a documented degradation.
`src/domain/ports/metrics.rs:183-186` — the backlog is expected to stand *"an estimated eight months"* by the PRD risk table.

"Bounded by the backlog" is not a bound: the backlog is every tax-inclusive row every tenant has published, expected to persist for months, re-read whole every 60 seconds. It is an availability rather than a correctness concern, and it is the one ticker read with no cap and no saturation story.

**PAID `1c6cfbd2d`, `9feaed032`** — bounded pages with the answer still exact. Note this entry's suggested model was wrong: the sibling it points at dedups without a bound, so it was never the paging example to copy.

**Z10-12 [Low] Nothing tests the scheduling plane — the exact test-smell (c) the method names**

`tests/module_test.rs` — grepped for `serve`, `ticker`, `cancel`, `lease`: the only hits are route/OpenAPI declarations; the file tests capability declarations, path sets, headers and the unconfigured-boot 404 (`:27,40,407,431,448,476,610,639`).
`src/infra/jobs/*_tests.rs` and `tests/{sqlite,postgres}_window_activation.rs` all drive `job.run(now)` / `job.run_once()` directly.

Every job body is well covered; the code that decides *when and under what lease a body runs* has no coverage at all. Z10-2 (a dropped guard halving a cadence), Z10-3 (a panic invisible until shutdown) and the `with_metrics` attachment at `module.rs:276` all live entirely in that gap, and all three are the kind of defect a single ticker-level test would have reddened.

---

**Verified clean:**

- **Cleanup is not Err-only.** `warm_pass` (`module.rs:296-307`) and `activation_pass` (`:469-486`) call `guard.release()` **after** the `match`/`if let`, on every outcome. There is no durable in-progress marker in any of the three jobs — no RUNNING row, no cursor, no "swept" column — so the lease is the only in-flight state, and it self-frees at its TTL. `window_activation.rs:26-33` states this positively: *"There is no 'swept' column and no cursor; the state **is** the record of what has been done."*
- **No future-drop hazard on the tick path.** In `tokio::select!` a chosen branch's handler runs to completion; cancellation is observed only at the next loop head (`module.rs:277-286`, `:379-392`, `:417-430`). The only multi-write sequences — `window_activation::flip` (`:434-448`) and `readmodel_warm::mark_degraded` (`:966-980`) — are entirely inside `in_transaction` closures, so even an abort at `stop_timeout` rolls them back whole rather than half-applying them.
- **`release` cannot free a peer's lease.** `coord/src/lease/guard.rs:136-147` filters on `key AND locked_by = self.locked_by`; a release after a steal matches zero rows and logs a warn. `renew_once` (`:344-362`) adds `live_filter`, so an expired lease cannot be resurrected.
- **One flip, one event.** `outbox_repo::price_window_transition_dedup_key` (`:808-810`) covers `(event, window_id)`, and `outbox_id` (`:1054-1056`) derives the PK from `(tenant_id, dedup_key)` so a repeat collides on both constraints; the flip and the enqueue share one transaction (`window_activation.rs:434-448`); raced for real in `tests/postgres_window_activation.rs:251` (`two_sweeps_in_flight_flip_a_window_once_and_emit_one_event`). The lease is explicitly *not* the guarantee, which is the correct reading — a lease can be lost.
- **The batch bound is spent in the order that makes it a guarantee.** `window_activation.rs:311-330` reads expiries first and gives activations `activation_budget(due.len())` (`:545-547`, saturating), so a saturated page defers a changeover instead of splitting it; unit-tested at `window_activation_tests.rs:106-129` and end-to-end at `tests/sqlite_window_activation.rs:767`.
- **The TEXT-timestamp ordering hazard is armed against.** `tests/sqlite_window_activation.rs:23-31` and the case `a_boundary_earlier_the_same_day_is_due` (`:591`) exist specifically because *"the mirror stores `timestamptz` as `text`… the two shapes agree on the date and disagree at byte 11"*. coord's own `Dialect::expired_filter` (`manager.rs:250-255`) normalizes with `datetime()` for the same reason. The one place a same-day compare could misfire is covered on the backend where it misfires.
- **The frontier scan's truncation drops the right end.** `pin_frontier_repo::list_all` (`:178-185`) orders by `advanced_at ASC` before `.limit()`, so `FRONTIER_SCAN_LIMIT = 1_000` discards the **freshest** frontiers — the ones that cannot be stale. I went looking for a coverage hole here and there is none.
- **Metrics do reach an operator.** `libs/toolkit/src/bootstrap/run.rs:314` calls `init_metrics_provider` during bootstrap, before `Gear::init`; `libs/toolkit/src/telemetry/init.rs:406` does `global::set_meter_provider`. `PricingMetricsMeter::new()` at `module.rs:691` therefore binds instruments to the real OTLP pipeline, is built **once** and stored on the runtime (`module.rs:126-133`) so the observable gauge's callback registration is not duplicated, and the `_tax_not_sellable_ga` handle is deliberately retained (`metrics.rs:96-102`) because dropping an `ObservableGauge` unregisters it. All three of those are the failure modes I checked for.
- **No label-cardinality hazard.** Every label value is an `as_str()` on a closed `#[domain_model]` enum (`ports/metrics.rs:79-87`, `:118-126`, `:150-158`, `:229-237`); no tenant id, no free-form key, no caller-supplied `&str` anywhere on the port. The argument lives at `ports/metrics.rs:18-32`.
- **The metrics suite reads the exported stream, not a spy.** `metrics.rs:335-341` and `test_harness` (`:347-508`) go through a real `SdkMeterProvider` + `InMemoryMetricExporter`, take the *latest cumulative* snapshot (`:400-411`), and distinguish "recorded zero" from "never recorded" via `gauge_point` (`:426-433`) — the exact distinction a probe needs to redden when a write is deleted.
- **The gated-markets job publishes nothing on a failed read**, so a storage fault cannot clear §7's alarm (`gated_markets.rs:90-109`), and the test asserts by **call count** as well as value (`gated_markets_tests.rs:95-123`) because the two states are indistinguishable from the value alone. This is the one place in the zone where the probe is armed against the right claim without prompting.
- **Enqueue is in the same transaction as the state change it reports** at every producer I checked on this plane (`window_activation.rs:437-447`, `readmodel_warm.rs:969-979`), and `outbox_repo.rs:1-10` states the invariant. `published_at` is set to `None` and never written (`:990-991`).
- **[by-design] There is no relay draining `pricing_outbox`.** `outbox_repo.rs:34-40` (*"Draining is the relay's job and no part of the publish commit; `idx_pricing_outbox_undrained` is that relay's cursor"*), `:738-739` (*"Nothing is owed to a consumer either way, **no relay existing**"*), and the index exists (`m20260802_000009:54`, re-created at `m20260802_000060:106`). Every event this gear enqueues is at-least-once *in intent* and zero-times *in fact* today; documented, and the consumer contract (`domain/events.rs:10-18`) correctly says at-least-once rather than exactly-once. No doc in the zone claims exactly-once or a fence the code does not implement.
- **[by-design] `PricingAlarm::TaxReadinessDivergent` is declared and never raised** — `ports/metrics.rs:187-194` says so and gives the reason (the post-GA reconciliation is a contract with a Tax Engine that does not exist). It sits in mild tension with the same block's rule at `:174-178` (*"A variant with no emitter would be a roster entry claiming coverage that does not exist"*), but the exception is stated at the variant, which is where a reader will be.
- **[by-design] `CatalogEvent::PlanCreated` and `PlanUpdated` have no producer** (grepped: zero `CatalogEvent::PlanCreated|PlanUpdated` outside `domain/events.rs`). The enum is a *frozen contract set* declared by the design set, not a roster of what this gear emits, so an unemitted name is not the same defect as an unemitted alarm. Recorded rather than filed.

**Refutations:**

- *Suspected: the gated-markets ticker's lease is unfenced so a drop could free a peer's slot.* Killed — `release_impl` filters on `locked_by` (`guard.rs:136-147`). The defect turned out to be the opposite direction (Z10-2: it never releases at all).
- *Suspected: a shutdown drops a pass mid-`.await`, leaving a half-applied flip.* Killed — `select!` does not cancel a chosen branch's body, and both multi-write sequences are inside `in_transaction`.
- *Suspected: `FRONTIER_SCAN_LIMIT` silently drops tenants from a Critical alarm past 1000.* Killed by the `ORDER BY advanced_at ASC` at `pin_frontier_repo.rs:181`.
- *Suspected: instruments built at `Gear::init` bind to a no-op provider because telemetry initializes later.* Killed — `bootstrap/run.rs:314` runs first, and `usage-collector/src/infra/metrics.rs:411-414` documents the same ordering guarantee for the same reason.
- *Suspected: `window_repo::list_due`'s `ORDER BY effective_from LIMIT n` sorts lexicographically on SQLite and so is never proven on Postgres.* Killed — every write goes through one SeaORM formatter, and the same-day case at `tests/sqlite_window_activation.rs:591` is armed against exactly that byte-11 divergence.
- *Suspected: a permanently failing ref poisons a tenant's queue.* Killed — `list_pending_for_tenant` is oldest-first and bounded, saturation is detected and downgraded to `PassCoverage::ScanSaturated` (`readmodel_warm.rs:382-392`) rather than ignored, per-version and per-tenant faults are isolated (`:750-775`, `:415-440`), and a stuck ref ages into `commit_overdue` / `pin_eligibility_overdue`. There is no attempt counter or dead-letter state, but nothing starves silently.
- *Suspected: a stringly-typed status token where the enum exists.* Killed for this zone — `boundary.origin_state().as_str()` / `target_state()` (`window_repo.rs:699`, `window_activation.rs:425`) and `event.as_str()` (`outbox_repo.rs:984`) all route through the domain enums, and `boundary_rank` (`window_activation.rs:527-532`) is deliberately spelled out rather than derived from declaration order.
- *Suspected: the correlation id is borrowed from an unrelated operator call.* Killed — both background emitters mint per-emission with a stated D-178 clause-(2) argument (`window_activation.rs:406-412`, `readmodel_warm.rs:948-962`).

**Not covered:** `src/infra/read_model.rs` (the `ReadModelProjector` itself — `project_version`, the finalize CAS, the frontier advance and `PassCoverage`'s completeness gate; I relied on the module docs' claims about it and did not verify them); the internals of `window_repo::transition`, `catalog_version_ref_repo::{observe_commit, next_committed_version_after}` and `plan_shape_repo` beyond the call shapes the jobs use; every non-job `outbox_repo::enqueue` call site (`publish.rs`, `overlay_publish.rs`, `bundle.rs`, `supersession.rs`, `retirement.rs`, `migration.rs`, `cutover.rs`, `window.rs`) and their dedup keys — I read `outbox_repo` whole but did not audit its eleven producers; `coord`'s own migration and `sqlite_tests.rs`; the ledger beyond the two comparisons cited; `tests/sqlite_read_model.rs`, which is where most of the warm sweep's behavioural assertions actually live.


---

**Zone Z11 — import / bulk / snapshot / money / instant**

**Files:** `src/domain/{import,bulk,snapshot,money,instant}.rs` + their five `_tests.rs` (1 654 lines, all read whole), and the call sites that matter: `src/infra/import.rs`, `src/infra/bulk.rs`, `src/api/rest/bulk_imports.rs`, `src/infra/storage/repo.rs` (`check_authored_instant`) and its eleven callers, `src/infra/storage/repo/{bulk_repo,outbox_repo,overlay_repo,price_repo,catalog_version_ref_repo}.rs`, `src/domain/publish.rs`, `src/domain/overlay.rs` + `overlay_rules.rs`, `src/api/rest/{overlays,prices,threshold_policy,approvals,publish}.rs`, migrations `m20260802_000018/000047/000063/000064`, and `docs/design/{01-foundation,03-price-structure,12-operator-efficiency}.md`.

**What's done:** every export of the five modules traced to its callers by grep, and every caller opened. The two primitives were checked the other way round as well — for each rule, *which construction sites skip it*: all four `check_quantum` sites and all eleven `check_authored_instant` sites enumerated and compared against every `DateTime<Utc>` field declared in `src/` (one grep, 170 hits, read in full); all `MinorAmount` sites enumerated and compared against the money values that travel as bare `i64`. Tests read as a review object; the store-side tiers (`tests/sqlite_import.rs`, `tests/postgres_bulk_repo.rs`) were consulted for what the unit tests deliberately do not pin. Nothing was built, run, or edited.

**Verdict:** the four *authoring* modules (import, bulk, and their two phases) are the strongest code I have read in this crate — the report invariant is structural, the duplicate rule is judged on the schema as it now stands, and Phase 2's "every path lands terminal" discipline is real. The defects are all at the **edges** of the zone: two primitives whose rule is not applied at every site it claims (money's scale rule at *none*, instant's quantum on three planes), one whole domain type that no live path produces, and the request-level half of the bulk flow (Phase 1's failure path, the replay's payload axis, the unbounded body) which never got the care Phase 2 got. No Critical. Two of the Mediums are the "fix carried to one site and not its sibling" shape the brief names.

**Findings**

**PAID `fb478a34c`, both halves, both runtime-driven** — the lease half (two passes over one `LeaseManager` proving the release lets the next tick acquire) and the `with_metrics` attachment. The entry's headline had already gone false before this: `f370e8933` added three supervision cases. Two production seams were added for testability with no signature changes, and `SweepReport` is now destructured exhaustively, so a twelfth counter becomes a compile error rather than a silently dropped field.

**Z11-1 [Medium] The ISO-4217 scale rule has no operand and no caller — `PRECISION_EXCEEDED` is a declared publish refusal nothing can raise**
- `pricing/src/domain/money.rs:158` `is_expressible`, `:168` `check_scale`, `:191` `fraction_digits`, `:216` `check_decimal`
- grep for all four names across the whole gear (`gears/bss/pricing`, `*.rs`): the only hit outside `money.rs`/`money_tests.rs` is a **doc sentence**, `pricing/src/domain/publish/rules.rs:75` — "`check_scale` / `check_decimal` carry `PRECISION_EXCEEDED`"
- `pricing/src/infra/error_mapping.rs:98-99` maps `D::PrecisionExceeded` to the wire code `PRECISION_EXCEEDED`; no producer exists
- `docs/design/01-foundation.md:355`, `docs/design/03-price-structure.md:163` and `:302`, `docs/design/04-currency-tax.md:160` all declare it a publish rejection; `01-foundation.md:82` is the FR (`…precision = the currency's ISO 4217 minor unit`)
- `pricing/src/domain/money.rs:16` — "`PRECISION_EXCEEDED` is a first-class publish rejection rather than a rounding decision taken quietly at the boundary"

The wire carries integer minor units only (`PriceContentView.amount_minor: Option<i64>` → `prices.rs:1330` `amount()` → `MinorAmount::new`), so no request can *declare* a scale and the fault is genuinely unrepresentable today — which `api/rest/approvals.rs:588-598` argues explicitly for the sibling threshold field ("as a *rule* about an integer already declared to be in minor units it constrains nothing"). That makes this a Medium and not a High. What is wrong is that three documents and one module doc describe the family as **live enforcement**, and `publish/rules.rs:73-78` cites the two functions as the reason no pipeline rule is needed — a reader checking whether the FR is satisfied is told "yes, by these", and these are never called. It is also the exact seam that hatches with green tests: the first authoring surface that takes a decimal literal (a CSV bulk import is the obvious one, and §4's import already exists) will not automatically reach `check_decimal`.
*Fix/Verify:* either state in `money.rs` and `publish/rules.rs` that the family is **dormant until a decimal/major-unit authoring surface exists**, and name that surface as the one owing the call; or delete the four functions and the `PrecisionExceeded` variant plus its mapping. Verify by grepping for a producer of `DomainError::PrecisionExceeded` — today there is exactly one, `money.rs:172`, reachable only from `money_tests.rs`.

**PAID `b31fe2bd1` — as prose, deliberately NOT as enforcement, and the entry's premise has moved.** D-311 gave `PRECISION_EXCEEDED` an operand in `RateMinor::from_decimal`, but that function has no production caller (D-311's own wire spelling is the integer `unitRateNanoMinor`), and `check_scale`/`check_decimal`/`is_expressible` still have zero callers. The design set names a path but fixes amounts as integer minor units (`01-foundation.md` §2.2), so that path cannot carry the fault. Two clauses of D-311's landing note are false and checkable. The rule stays dormant and the surface that owes the call is now named in the code.

**Z11-2 [Medium] The overlay plane's `cohort` bypasses the millisecond quantum the price plane enforces on the same axis (F-12 shape)**
- `pricing/src/domain/instant.rs:6-11` — the module's own motivating case: "`cohort` **is** a cutover instant and is matched for **equality** across a gear boundary … the generation becomes unfindable"
- price plane, gated twice: `pricing/src/domain/scope_key.rs:604` `instant::check_quantum("cohort", generation)?` in `ScopeKey::new`, and `:748` in the cutover narrowing
- overlay plane, ungated: `pricing/src/api/rest/overlays.rs:399-407` takes `request.cohort: Option<DateTime<Utc>>` (`:165`) straight into `LineKey::for_cohort`, and `pricing/src/domain/overlay.rs:445-450` `for_cohort` performs one check — that `plan_id` is present
- the equality compare it feeds: `pricing/src/infra/storage/repo/overlay_repo.rs:1935` `published_generation` rebuilds the price plane's millis, and `:1916-1932` documents that the two planes render one instant two ways
- `pricing/src/domain/overlay_rules.rs:472-495` `check_eligibility` — membership test `generations.contains(&cohort)`
- `pricing/src/domain/overlay_rules.rs:854` `check_dating` — `if holder.scope != candidate.scope || holder.key != line.key { continue; }`
- `pricing/src/domain/overlay_rules.rs:584-597` `check_duplicate_keys` — `BTreeSet<&LineKey>`, so `LineKey`'s derived `Ord` (cohort included) decides what a duplicate is

Live behaviour is fail-closed, which is why this is Medium: `published_cohorts` is built from millisecond values, so an authored `2026-08-02T12:00:00.000123Z` can never be a member and `OVERLAY_LINE_COHORT_UNKNOWN` always fires. But the refusal **misattributes the fault** — it tells the operator the plan "carries no published `existing_grandfathered` row at that cutover instant" when the real fault is precision, and the value that provokes it is precisely what a client gets by copying a Postgres `timestamptz` rendering from another gear. Two rules are silently disarmed behind that single fail-closed gate: `check_dating` **skips** an unquantized line entirely (key inequality at `:854`, so no overlap is ever reported for it), and `check_duplicate_keys` counts two cohorts a microsecond apart as two keys. So the eligibility rule is the only thing standing between an unquantized cohort and two published overlays overlapping on one generation — the arming condition being any future relaxation of eligibility (e.g. a cohort authored before its generation publishes).
*Fix/Verify:* call `instant::check_quantum("cohort", cohort)` in `LineKey::for_cohort` (or at `overlays.rs:399`), which puts the refusal in the same place, with the same code, as the price plane's. Verify with a case authoring a `.000123Z` cohort and asserting `TIMESTAMP_PRECISION_EXCEEDED` rather than `OVERLAY_LINE_COHORT_UNKNOWN`.

**PAID `ab27be2fc`.** `cohort` now passes `check_authored_instant` in `overlay_repo::write_lines`, the one funnel all three writers share.

**Z11-3 [Medium] Three more authored-instant planes never meet the quantum gate that `repo.rs` calls "one rule"**
- `pricing/src/infra/storage/repo.rs:91-99` — "Here rather than in each repository because both of them store instants an operator authored … and the quantum is one rule"; `:110-122` `check_authored_instant`
- its complete caller set (grep): `threshold_repo.rs:314,371` (`effectiveFrom`), `window_repo.rs:309,310` (`effectiveFrom`/`effectiveTo`), `plan_repo.rs:985,986,2208,2209` (`availableFrom`/`availableTo`), `price_repo.rs:588,2806` (`grandfatherUntil`)
- ungated writers of authored instants: `overlay_repo.rs:245-246` `effective_from`/`effective_to` (authored at `api/rest/overlays.rs:132-134`, `:219-221`); `migration_repo.rs:154-155` `effective_at`/`announced_at` (authored at `api/rest/migrations.rs:98`); `synthesis_repo.rs:126` `snapshot_instant` (authored at `api/rest/migrated_origin_snapshots.rs:82`)
- two of the three are emitted on events: `outbox_repo.rs:537` `effective_at`, `:755-757` `effective_from`/`effective_to`

`instant.rs:19-22` scopes the rule to "instants the gear **authors, carries in a contract field, publishes or compares**", excluding only storage bookkeeping. All three of these are authored, all three are contract fields, two are published, and the overlay interval is compared (`overlay_rules.rs:857` `intersects`). The overlay interval is the clearest outlier: `window_repo` gates the identically-shaped `effectiveFrom`/`effectiveTo` pair one module over. The failure mode is weaker than Z11-2's — these are order-compared, not equality-matched — which is why it is Medium and not High, but the divergence between a gated price window and an ungated overlay window is not something either module's doc claims.
*Fix/Verify:* add `check_authored_instant` calls beside the three writers, or record in `instant.rs` which planes the rule deliberately does not reach and why. Verify by re-running the caller grep: the gate's callers should be the set of tables holding an authored instant column.

**PAID `cda5f24bd`, with one clause of this entry REFUTED.** Overlay `effectiveFrom`/`effectiveTo`, `pricing_migration.effective_at` and `snapshot_provenance.snapshot_instant` are gated. But `announced_at` is NOT an authored instant — it is on no request and comes from the act's stamp, and `window_repo::transition` already records that gating a machine timestamp refuses every write. Left ungated deliberately, with both halves pinned in one case. Consequence worth knowing: `POST /migrations` now refuses a sub-millisecond `effective_at` on the wire, which reddened six cases whose fixture authored `Utc::now()`; the fixture was quantized AND a REST case added for the refusal, so the fixture change cannot hide the new behaviour.

**Z11-4 [Medium] A Phase-1 store failure strands the run in `validating` forever, and the replay then answers `202` with an empty report**
- `pricing/src/api/rest/bulk_imports.rs:304-318` opens the run (born `validating` — `bulk_repo.rs:132`), then `:321-324` `classify_against_store(...).await.map_err(...)?` — a bare `?`
- the only two other writers of the run's state (grep `BulkState::` across `src/`, excluding `domain/bulk` and tests): `bulk_imports.rs:333` (ValidationFailed, on the *rule* path only) and `infra/bulk.rs:151` (Committing)
- `bulk_imports.rs:454-462` — abort refuses anything that is not `Committing`
- `bulk_imports.rs:290-298` — the replay arm: any state other than `ValidationFailed` returns `202` + `run_view`, i.e. `state: "validating"`, `report: {"rows": []}` (the placeholder written at `:312`)
- the two precedents this contradicts: `domain/bulk.rs:141-144` — `Rejected` exists because D-267 found a run "stranded there with no exit"; `infra/bulk.rs:163-168` — "**Everything from here lands the run terminal and releases its locks, on every path** … §4 offers no failure edge out of `committing`, so a run that enters it and stops has no exit"

Phase 2 was hardened three times (D-294, D-297, D-300) for exactly this hazard and Phase 1 was not, in the same handler. A transient repo error inside `classify_against_store` — or a crash between `open` and either terminal move — leaves the client key **spent** on a run that will never advance, with no operator remedy: abort refuses it, no sweeper exists, and the retry is answered `202`. That answer is worse than the failure: `:280-289` argues at length that a replay must not tell a client a resubmit succeeded where the original failed, and installs a refusal for the `ValidationFailed` case; the `validating` case reproduces the same harm with no refusal at all.
*Fix/Verify:* on a Phase-1 repo error, advance to `ValidationFailed` (or another terminal state) with the failure in the report before propagating; and/or admit `validating` to the abort edge. Verify with a fault-injected `classify_against_store` and a replay asserting a refusal rather than `202`.

**PAID `4c27955c5`.** A Phase-1 store fault now lands the run terminal on the existing `validating → validation_failed` edge instead of stranding it, so the client key is not spent on a run with no exit and the replay no longer answers `202` over an empty report. `validating` was deliberately NOT admitted to the abort edge: that edge exists on neither engine, so adding it would be a migration and a design decision rather than a fix. RED output prints the run stuck `Validating` with `{"rows": []}`.

**Z11-5 [Medium] The bulk replay has no payload guard, so a *different* batch under a spent key is answered `202` and imports nothing**
- `pricing/src/api/rest/bulk_imports.rs:275-299` — the replay is `find_by_client_key` and nothing else; the body (`:301` `rows_of`) is never compared with what the key first carried
- `pricing_bulk_operation` has no request-hash column: `m20260802_000047_create_pricing_bulk_operation.rs:72` (columns are `operation_id, tenant_id, kind, state, client_key, report, submitted_by, submitted_at, completed_at`, re-created identically at `m20260802_000063:268-270`)
- the crate's own sibling gate does carry one: `infra/idempotent.rs:120` `request_hash`, `:177`; `storage/repo/idempotency_repo.rs:227` `if held.request_hash != request_hash` → `IDEMPOTENCY_PAYLOAD_MISMATCH`; mapped at `infra/error_mapping.rs:273`; documented for the interactive price route at `api/rest/prices.rs:499`
- the same replay has been corrected twice already on other axes: D-295 (state — `bulk_imports.rs:280-289`) and D-307 (kind — `bulk_repo.rs:182-190` plus `m20260802_000064`, whose doc says the danger is "a document with no field that could reveal the substitution")

`bulk_imports.rs:22-30` justifies replacing the gate with the run's own row on TTL grounds — which is sound — but silently drops the payload-mismatch guard the gate provided. `BulkImportView` carries no digest and no row count, so a caller who resubmits a corrected batch under the same key gets `202`, a stale report, and zero rows imported, with nothing in the response that could reveal it. That is the same inversion, on the third axis.
*Fix/Verify:* store `IdempotencyGate::payload_hash` of the canonical body on the run and refuse a mismatch with `IDEMPOTENCY_PAYLOAD_MISMATCH`; failing that, state the omission in the module doc beside the TTL argument. Verify with two `POST`s under one key carrying different row sets.

**PAID `0750ea7ab`, and an existing test had been PINNING the defect.** A payload digest now guards the replay. `a_replayed_key_answers_the_run_it_opened…` used to submit a DIFFERENT batch under the same key and assert `202` with the first run's report — it asserted the inversion as correct. It is re-armed on the same batch and is now the positive control for the new probe. Open decision recorded elsewhere: `rest_repricing_runs` pins the same shape deliberately, so that route records its digest and does not compare it.

**Z11-6 [Low] Row-level content faults escape the per-row report and name no row**
- `pricing/src/api/rest/bulk_imports.rs:542-561` `rows_of` — `.collect::<Result<Vec<_>, DomainError>>()`, so the first bad row aborts the whole body with one `DomainError` carrying no index
- the faults that land here: `CurrencyCode::new` (`money.rs:70`), `MinorAmount::new` via `prices.rs:1330-1339` (which prefixes the *field* name, never the row), every token parse in `content_of`
- against the contract: `bulk_imports.rs:155-163` promises "Phase 1 validates the whole batch and refuses it if any row is invalid … the per-row report is on the run"; `domain/import.rs:8-12` states the posture — "a rule that can answer for a row **must** answer for every row rather than stopping at the first"
- and it happens **before** `bulk_repo::open` (`:301` vs `:304`), so no run holds a report for these at all

A thousand-row batch with a typo'd currency in row 700 is answered "currency invalid: US" with no index and no run — which is precisely the round-trip-per-refusal the all-or-nothing posture exists to spare the operator.
*Fix/Verify:* fold these into `BatchReport` (they are per-row facts, exactly like `PRIMITIVE_RULES_UNBUILT`), or at minimum prefix `rows[{i}]:`. Verify with a batch whose second row carries a bad currency and a negative amount, asserting both are reported against row 1.

**PAID `223c381c1`**, two probes. Two judgements recorded rather than buried: the wire code for an unreadable row becomes `BULK_VALIDATION_FAILED` and now spends the idempotency key (both per §5 and per this entry's own complaint; no test pinned the old codes), and **no per-row code was minted** — the faults ride an `unreadable` report member instead, because an undeclared code on a replayable shape is the worse of the two.

**Z11-7 [Low] The submitted batch is unbounded**
- `pricing/src/api/rest/bulk_imports.rs:107` `pub rows: Vec<BulkImportRowRequest>` — no cap anywhere on the path
- both phases run **inline in the request**: `bulk_imports.rs:5-13` ("the reason survives the current implementation running both inline"), `:321-366`
- each row is its own transaction (`infra/bulk.rs:5-12`), and every row also takes a row lock (`bulk_repo::take_locks`)
- the read side of the same gear does have a hard cap: `api/rest/overlays.rs:192` "server default 100, hard cap 1,000 (D-125)"

[Q] whether the design set states a request-side cap: `docs/design/12-operator-efficiency.md` names only export chunking (`:220`, `:404`) and D-125 is a *read* pagination decision (`DECISIONS.md:1125`). Absent one, a single `POST` can hold a connection open across an arbitrary number of transactions while holding row locks against every interactive editor.
*Fix/Verify:* name a cap (the 500-rows/plan soft cap of §1.2 is the nearest stated number) and refuse above it, or record the absence as deliberate.

**PAID `fef443d7e` AS AN ABSENCE RECORDED — the headline is half false, established by measurement.** The batch is not unbounded: the platform body limit refuses an oversize body before the handler ever runs, proved with a probe asserting the refusal AND that no run was opened. A row cap was **declined**: §1.2's 500-per-plan is D-160's advisory, never-blocking number, and turning an advisory figure into a hard refusal would change the contract under cover of a fix.

**Z11-8 [Medium] `PricingSnapshotRef` is a live-looking domain type that no path produces, and its doc names a finalizer that does not exist**
- `pricing/src/domain/publish.rs:434-446` `snapshot_ref()` is the only producer; grep for `snapshot_ref` across `src/` and `tests/`: the definition and `domain/publish_tests.rs:101,121`. Nothing else.
- `PricingSnapshotRef::finalize` (`snapshot.rs:146`) and `VersionRef::finalize` (`:82`) have no caller outside `snapshot_tests.rs`
- `pricing/src/domain/publish.rs:437-439` states "`CatalogVersionPublished` finalizes it through [`PricingSnapshotRef::finalize`]" — the actual one-way finalize is a row compare-and-swap, `infra/storage/repo/catalog_version_ref_repo.rs:426`, whose own doc (`:43-47`) calls the domain one a "storage-side sibling … not one mechanism and cannot be"
- the composition that actually reaches consumers is hand-built beside the type: `infra/storage/repo/outbox_repo.rs:130-142` `json!` with `pendingVersionRef` / `priceIds` / `evaluationPolicyVersion`
- the 202 body re-lists two of the three segments and omits the policy version: `api/rest/publish.rs:161-188` `PublishReceiptView`

`snapshot.rs:1-19` says this type is where two normative properties live — the aligned composition and the one-way pin. Both are therefore asserted in a place off every live path, while the wire's version of the same composition is a separate spelling guarded only by `infra/storage/repo/outbox_repo_tests.rs:130-160`. This is the dead-field class applied to a whole type, plus F-8's shape (a doc naming a producer that does not produce).
*Fix/Verify:* either route the payload build through `PricingSnapshotRef` (so the three-segment invariant has one home), or correct `publish.rs:437-439` and mark the type as the not-yet-consumed catalog-side model with the surface that owes its use. Verify by grepping `snapshot_ref` for a non-test caller.

**PAID `1ec151008`, documentation only, lints in `04f9dfc8a`.** Verified the claim: `snapshot_ref()` is produced only in tests and `PricingSnapshotRef::finalize` has no caller outside its own tests; the live finalize is `catalog_version_ref_repo::finalize`'s row CAS. Read D-30 and D-102 — **no producer built and the type not deleted**, because this gear is told it never stamps snapshots and another gear's contract may name the type. The doc now names the live mechanism, the dormancy, and the surface that owes adoption. No test added and the totals do not move, which is the correct evidence for a prose change.

**Z11-9 [Low] A cohort token is re-spelled as a SQL literal where the enum owns the rendering**
- `pricing/src/infra/bundle.rs:77` `const COVERAGE_COHORT: &str = "none";` and `:476` `.add(price::Column::Cohort.eq(COVERAGE_COHORT))`
- the reader on the same column goes through the enum: `infra/storage/repo/price_repo.rs:3512-3514` `if token == Cohort::None.to_string()`
- every other filter on this column derives the token from the value: `price_repo.rs:1945` `cohort.to_string()`, `:2609` `key.cohort().to_string()`

A change to `Cohort`'s rendering is caught by `read_cohort` and by the two derived filters, and silently turns the bundle coverage query into one that matches zero rows.
*Fix/Verify:* `Cohort::None.to_string()` at the call site. (Flagging across the zone line — the axis belongs to `scope_key` — rather than dropping it.)

**PAID `b43b909ad`** with Z9-7 — the same two literals. See Z9-7 for why the stated failure mode does not hold.

**Z11-10 [Low] The row that failed is reported as "not attempted"**
- `pricing/src/infra/bulk.rs:335-343` — on a run-level fault at row `index`, the loop runs `for unreached in index..rows.len()` and stamps "not attempted: the run failed at row {index}"

Row `index` **was** attempted; its own transaction failed. Including it in the receipt is right (nothing committed for it), the sentence is not — and `not_attempted`'s own doc one screen down (`:387-393`) records that a previous version of this same family was corrected for saying something untrue of a row.
*Fix/Verify:* give row `index` its own sentence naming its failure, and start the "not attempted" run at `index + 1`.

**Verified clean:**
- **`BatchReport`'s one-entry-per-row invariant is structural, and all three writers honour it.** Private field + `binary_search_by_key` insert + per-row code sort (`domain/import.rs:158`, `:173-188`); the store half joins through `add` (`infra/import.rs:117,169`) and so does Phase 2's normalisation (`infra/bulk.rs:429-435`). `Serialize`-only, with the reason stated (`import.rs:112-117`) — a `Deserialize` really would rebuild the state the private field prevents.
- **The in-batch duplicate is judged on the schema as it now stands.** Whole `ScopeKey` including the usage pair (`import.rs:262-269`), matching `uq_pricing_price_scope_key_draft` as `m20260802_000023` widened it; and the keys it groups on are the keys the store files under, because `rows_of` derives the usage line through the store's own resolver (`bulk_imports.rs:548` `price_repo::resolve_authored_usage_line`, D-306) rather than a second spelling. The "builder producing keys the store would never file rows under" class was live here and is fixed.
- **Both sides of a duplicate are named, order-independently** (`import.rs:271-298`), tested at two and three rows *and* against the rows that must pass (`import_tests.rs:103-137`, `:205-226`), with the companion assertion that keeps each case honest against a `classify` that reports nothing (`:170-173`, `:198-201`). Empty batch tested (`:311-316`). The primitive arm is tested for one field and for both (`:229-276`), and the two-fault case pins the *order* of codes (`:290-299`).
- **Phase 2's partial-failure semantics.** Receipt survives a run-level fault so committed rows stay visible (`bulk.rs:169-189`), locks released on every path including a failed commit (`:194-197`), terminal state reached on every path (`:210-220`), and the lock target list is deduped so a batch naming one draft twice cannot conflict with itself (`:140-144`). The per-row/run-level error partition is explicit and reasoned (`:320-331`).
- **The client key is namespaced by kind** — query (`bulk_repo.rs:194-214`) and index (`m20260802_000064`) both, which is the half-fix hazard closed.
- **`domain::bulk`'s tokens agree with the store.** Round trip both ways, unknown token refused rather than defaulted, case-sensitivity pinned (`bulk_tests.rs:27-64`), terminal set matched against `chk_pricing_bulk_operation_completed_at` (`:66-89`) with the "everything is terminal" degenerate case separately excluded (`:92-101`). No state token appears as a bare literal anywhere outside `domain/bulk.rs` (grep of all seven).
- **`money.rs` construction-time invariants.** `CurrencyCode::new` checks byte length **and** ASCII-alphabetic, so a 3-byte multibyte input is refused (`money.rs:72`); normalization is applied before storage so the scope-key axis has one spelling (`:75`, tested `money_tests.rs:14-19`). `MinorAmount::new` is the only constructor and every store read-back goes through it (`price_repo.rs:3626-3631`, `:3470`), so the non-negative invariant survives a corrupt row rather than being assumed. No float, no decimal type, no division anywhere near money (grep for `f64` / `/ 100` in `money.rs`, `materiality.rs`, `prices.rs`: nothing). The zero- and three-decimal tables are, checked against ISO 4217, the **complete** sets for circulating currencies — the "launch subset" caveat is more modest than the code, and the 2-decimal fallback catches only genuinely 2-decimal codes.
- **`instant.rs`'s leap-second reasoning is correct as written** — `chrono` reports `nanosecond() ≥ 1_000_000_000` during a leap second and `1_500_000_000 % 1_000_000 == 0`, so a leap second on the quantum is expressible exactly as the doc claims (`instant.rs:38-42`). `is_quantized`/`check_quantum` is one rule with one predicate, consulted rather than re-implemented at all four sites (`scope_key.rs:604,748`, `supersession.rs:428`, `repricing_runs.rs:398`) and once at the storage boundary (`repo.rs:116`).
- **The stored-cohort text column is never ordered.** Every SQL use is equality: `price_repo.rs:1945`, `:2609`, `bundle.rs:476`. The one cross-rendering comparison is done in Rust, once, with the text-ordering trap named explicitly (`overlay_repo.rs:1929-1932`, "`SeaORM` writes ISO 8601 with a `T`, and `'T'` beats `' '` at byte 11").
- **The two money values that travel outside `MinorAmount` each carry their own enforcement.** `absolute_minor` is refused negative at parse (`api/rest/threshold_policy.rs:592-599`) *and* by a `CHECK` (`m20260802_000018:154`), with the precision question argued inapplicable in place (`api/rest/approvals.rs:588-598`); overlay amount magnitudes are refused negative by `overlay_rules.rs:674-687`. Outliers, but justified ones — recorded here so a later pass does not re-open them.
- **The three snapshot segments do reach the wire**, whole and closed: `outbox_repo_tests.rs:115-160` asserts the payload by whole-value equality *and* asserts the absence of an invented `pricingSnapshotRef` envelope. The retirement payload's missing `priceIds` is deliberate and documented (`outbox_repo.rs:477-479`).
- **`VersionRef`'s two-state model and one-way finalize are correct and well tested** — consuming `self` so a stale handle cannot coexist with its version (`snapshot.rs:70-91`), re-finalize refused even to the *same* version, with the reason (`snapshot_tests.rs:60-70`), and finalize proven to change nothing but the version ref (`:84-99`). The defect is that nothing calls it (Z11-8), not the model.
- **`BulkState::AwaitingApproval` / `Rejected` are written by no code** (grep) — the repricing approval lane is unbuilt. **[by-design]**, documented at `domain/bulk.rs:17-22` and D-267, with the `CHECK` (`chk_pricing_bulk_operation_import_never_awaits`) proven on Postgres (`tests/postgres_bulk_repo.rs:365`). Recording it because it is the "rule whose counterpart system does not exist" shape: the day the repricing lane lands, the `validating → awaiting_approval` edge must arrive in the trigger, and nothing here would fail if it did not.

**Refutations:**
- *Suspected the bulk dedup key was narrower than the operation's identity.* Refuted — `kind` is in both the lookup (`bulk_repo.rs:207`) and the unique index (`m20260802_000064`), and the migration doc reasons about exactly the substitution I was looking for. D-307 got there first.
- *Suspected `duplicate_scope_keys` was judged on a stale key shape* (the classic here, per D-283). Refuted twice over: the rule takes the whole `ScopeKey` (`import.rs:263`), the module doc reads the schema rather than the creating migration (`import.rs:26-45`), and both usage axes have their own passing-and-failing test pair (`import_tests.rs:139-202`).
- *Suspected a lexicographic compare on the epoch-millis-in-text cohort column.* Refuted — `.eq()` only, at all three sites.
- *Suspected `bulk_tests.rs`'s hand-transcribed `STORED_STATES` was the only pin on the store's `CHECK` (test-smell (b), a literal standing in for the real producer).* Refuted — the `CHECK` itself is exercised on both engines (`tests/sqlite_bulk_repo.rs:195,255,331`, `tests/postgres_bulk_repo.rs:284,365,394,432`), and the Postgres tier asserts by constraint **name**. The literal list is a second, cheaper pin, not the only one.
- *Suspected money's "launch subset" tables would mis-scale a real currency into the default 2.* Refuted by checking both lists against ISO 4217 — they are complete for circulating currencies; the doc understates the code.
- *Suspected `report_of` could drop Phase 1's report when Phase 2 overwrites the column.* Refuted — Phase 2 only runs when `blocks_the_batch()` is false (`bulk_imports.rs:326`), so the report it overwrites is empty by construction.

**Not covered:** materiality's percent arithmetic and the delta/threshold comparison itself (another zone's, and `MinorAmount` only enters it as a value); `scope_key.rs` beyond the cohort axis and its two `check_quantum` sites; `window_repo`/`threshold_repo`/`plan_repo` beyond confirming they call the instant gate; `overlay_rules.rs` beyond the three rules the cohort feeds; the SQL trigger bodies of `pricing_bulk_operation` (read only through the migration files' statement strings and the tests that exercise them); `tests/sqlite_import.rs` and `tests/postgres_bulk_commit.rs` were consulted for coverage of specific claims, not reviewed as objects in their own right; nothing was compiled or executed.


---

**Zone Z12 — the test tiers as a review object**

**Files:** `gears/bss/pricing/pricing/tests/` — 97 top-level suite files plus three shared harnesses (`common/mod.rs` 388 lines, `rest_support/mod.rs` 1910, `pg_support/mod.rs` 540) and `pg_harness.rs` (the harness's own guard). Read in full or in the relevant part: `common/mod.rs`, `rest_support/mod.rs`, `pg_harness.rs`, `rest_authz.rs`, `corpus_publish.rs`, `corpus_snapshot_shape.rs`, `rest_retirement.rs`, `rest_bundles.rs` (§ publish, § composition), `rest_approvals.rs` (§ pin), `rest_preview.rs` (§ metrics), `sqlite_idempotency.rs` (head), `sqlite_append_only.rs` (§ D-236), the module docs of `postgres_approval_race.rs`, `postgres_supersession_race.rs`, `postgres_clone_atomicity.rs`, `postgres_bulk_commit.rs`, plus `src/module.rs:855-1003`, `src/api/rest/tax_display_policy.rs`, `src/infra/metrics.rs`, `Makefile:459-512`, `.github/workflows/ci.yml`.

**What's done:** Verified counts. The fast tier is **1152** `#[test]`/`#[tokio::test]` functions — 360 in `rest_*`, 771 in `sqlite_*`, the rest in `corpus_*`, `module_test.rs`, `pg_harness.rs`. The Postgres tier is **346** tests over 24 files. The REST census in `rest_authz.rs` catalogues **48** routes, 26 of them mutating, and asserts set-equality against what the routers actually register (`rest_authz.rs:1118`). Four set-level authz properties range over that whole census: the recorded `(resource_type, action)` pair plus `require_constraints` (`:1525`), denial-with-state-unchanged over the 26 mutating rows (`:1572`), PDP-outage fail-closed over all 48 (`:1635`), and scope-mismatch denial over the 26 (`:1730`). Three source scans back them: no handler may construct an `AccessScope` (`:1337`), every mutating router must apply the correlation edge (`:1464`), every `pub fn …router` must be merged into all four mount sites (`:1213`) — and two of the three carry their own falsifiability tests (`:1420`, `:1439`) and anti-vacuity floors (`:1481`, `:1498`).

**Verdict:** This is the strongest test suite I have reviewed in this repo. The harnesses turn no control off: `rest_support` mounts *every* production router (`:535-630`), holds the real PDP enforcer with five distinguishable doubles including a denying, an erroring and an unconstrained one (`:163-250`), uses the real metrics adapter over a private exporter rather than the no-op (`:405-406`), shares **one** registry `Arc` between both services exactly as production does (`:399`), and loads the real committed fixture registry (`:442`). Its docs repeatedly name the defect each helper exists to prevent. That said, three gaps are real and one of them is severe: **every concurrency, lock and isolation proof this crate owns is `#[ignore]`d and runs in no CI job**; the census's write-before-gate readback covers three planes while 15 of 26 mutating routes write a fourth; and two governance-adjacent surfaces (`POST …/retire`, `PUT /config/tax-display-policy`) are asserted from response bodies or not at all.

**Findings**

**PAID `d0e697604`**, one probe. Same class as the residual `ceaf1ff16` recorded on the abandon path, seen from the opposite side — that one under-reports what committed, this one over-reports what was attempted. The `Drop` path is untouched and that residual stands.

**Z12-1 [Critical] The whole Postgres tier — 346 tests, and every concurrency proof the crate has — is `#[ignore]`d and no CI job runs it**
**PAID `e2dad9203`.** See the status ledger at the top of this document.

- `tests/postgres_*.rs` — 24 files, `346` `^\s*#\[(tokio::)?test` attributes and `346` `^\s*#\[ignore` attributes: **every** test in the tier is ignored (`#[ignore = "requires Docker (testcontainers)"]`, `"needs Docker"`, `"requires Postgres; run with --ignored"`).
- `.github/workflows/ci.yml:247` — the only workspace test step is `run: make test-no-macros`.
- `Makefile:465-466` — `test-no-macros: cargo nextest run --workspace --exclude cf-gears-toolkit-macros-tests --exclude cf-gears-toolkit-db-macros`. No `--run-ignored`.
- `grep -rn "run-ignored|--ignored|include-ignored" .github/ Makefile` → **no matches anywhere in the repo**. The `integration` job (`ci.yml:251-338`) names five Postgres targets: `users-info`, `timescaledb-usage-collector`, `postgres-cluster-plugin`, and `toolkit-db` twice. `bss-pricing` is in none of them.
- `Cargo.toml:115` puts `gears/bss/pricing/pricing` in the workspace, so the **fast** tier does run. The pg tier is compiled and never executed.

What is lost is not redundancy. Each pg-only suite states in its own module doc that its property is *unprovable* on SQLite, and two of them say the SQLite twin passes either way:
- `postgres_approval_race.rs:4-11` — "`sqlite::memory:` **serializes writers**, so the interleaving under test can be neither confirmed nor refuted there… what it cannot show is what happens when the two statements are **in flight at the same time**, which is the whole hazard `inst-ap-pin` names." This is the TOCTOU guard on the approval pin.
- `postgres_supersession_race.rs:3-16` — the supersession commit's serialization point on the predecessor price row, "owed" because a 2026-08-05 review showed the stated ground for the write order to be false.
- `postgres_bulk_commit.rs:10-21` — "`SQLite` does not abort on a failed statement, so its version of [`a_row_another_run_holds_conflicts_the_whole_batch`] passes **whether or not that contract is honoured**." Its SQLite twin `sqlite_bulk_commit.rs:321` is therefore a test that cannot fail on the property it is named for.
- `postgres_clone_atomicity.rs:4-14` — eight of eleven tables carry append-only triggers and CHECKs "this gear spells **twice**, once in PL/pgSQL and once as `RAISE(ABORT, …)`, and two spellings are only ever as equal as the suites that hold them to it."
- `sqlite_append_only.rs:110-114` names the cost directly: "Postgres has the exhaustive thirty-four-column loop, but it is `#[ignore]`d behind Docker — and **D-236 is the record of what that costs: a premise living on one tier only means a run without Docker reports a clean change through a guard that stopped guarding**."

Every test in this crate for two racing writers, a lock held by a crashed pass, `FOR UPDATE` semantics, READ COMMITTED re-evaluation, and the PL/pgSQL half of every dual-spelled trigger lives in a tier that has never run automatically. `grep -c "tokio::spawn|join!|JoinSet"` over `tests/*.rs` matches exactly five files, all of them `postgres_*`.

*Fix/Verify:* add a `test-pricing-pg` Makefile target (`cargo nextest run -p bss-pricing --run-ignored ignored --test 'postgres_*'`) and a CI step in the `integration` job, which already has Docker. Until then, treat the pg tier as documentation, not coverage, and do not count its tests in any green total.

**Z12-2 [High] `every_mutating_route_is_denied_with_the_state_unchanged` reads back three planes; 15 of the 26 mutating routes write a fourth**
**PAID `1c7b22d16`.** See the status ledger at the top of this document.

- `tests/rest_authz.rs:1579-1586` — the readbacks taken before the denied call: `plan_count`, `plan_row_version(seeded.plan, 0)`, `price_rows(seeded.plan)`, `approval_row(seeded.approval).state`.
- `tests/rest_authz.rs:1582-1585` — the doc that justifies the fourth: "Without it the three decision routes would be in this loop asserting nothing about themselves: a handler that decided the unit and then checked the gate moves no plan row and no price row, **so every assertion below would hold**."
- `tests/rest_authz.rs:679-686` — `Seeded` also carries `window`, `bundle` and `overlay`, and `drive()` aims routes at all three (`:710-712`).

That argument applies verbatim to every plane the readbacks omit, and the omission is large. Of the 26 mutating census rows, the following write a table no assertion in this loop reads: `POST/PATCH …/bundles*` and `POST …/bundles/{id}/publish` (3), `POST/PATCH …/price-overlays*` and the submit (3), `POST/PATCH/DELETE …/price-windows*` (3), `PUT …/config/taxonomies/{class}` (1), `PUT …/config/tax-display-policy` (1), `PUT …/config/approval-threshold-policy` (1), `POST …/migrations` and `DELETE …/migrations/{id}` (2), `POST …/bulk-imports` and its abort (2), `POST …/repricing-runs` (1) — **15 of 26**. A handler on any of them that wrote first and gated second moves no plan row, no price row and no approval unit, and this loop stays green. `PATCH /price-overlays/{id}` is the sharpest: it is the only per-tenant discount surface, and its own suite never reads the overlay store after a denial either.

A second, smaller edge on the same test: `prices_after.first()` compares only the **first** price row's version (`:1613-1615`), so a denied write that moved the second of several rows would be caught only by the length check.

*Fix/Verify:* extend the `Seeded` readback to one row-version-or-state probe per plane the census can address (overlay, bundle, window, taxonomy, policy, migration, bulk run) and compare the whole `price_rows` vector rather than its head. Prove it by deleting the gate call from `overlays::patch` and watching this loop redden.

**CORRECTION, 2026-08-14 audit: OVERSTATED.** The seven named planes and the whole-price-vector read are genuinely delivered, but "reads every mutable plane" is not true: `POST /repricing-runs` writes the repricing journal and the threshold-policy `PUT` writes its proposal row, and neither plane is read back. Two planes owed, not zero.

**Z12-3 [High] `rest_retirement.rs` asserts response bodies only — the confirm arm proves neither half of its own name**
**PAID `1c7b22d16`.** See the status ledger at the top of this document.

- `tests/rest_retirement.rs:18` — the entire import list: `use rest_support::{Harness, body_json, request, seed_publishable_plan};`. No store readback helper of any kind is in scope in this file.
- `tests/rest_retirement.rs:107` — `async fn the_confirm_answers_202_and_opens_a_unit_rather_than_committing()`.
- `tests/rest_retirement.rs:119-134` — its assertions in full: status is 202, `view["outcome"] == "submitted_for_approval"`, `view["approval"].is_object()`, `view["pending_version_ref"].is_null()`, `view["preview"]["presence_unresolved"] == true`, `view["cancelled_window_ids"].len() == 0`.

Every one of those reads the response the handler just wrote. Nothing reads `pricing_approval`, so "opens a unit" is unproven — a handler that rendered an `approval` object and opened nothing passes. Nothing reads the plan's lifecycle state, so "rather than committing" is unproven too — a handler that retired the plan *and* rendered the same body passes. Retirement is an irreversible act over a published plan (D-128), which makes this the highest-stakes instance of the pattern.

The suite had the tool and did not reach for it: `rest_support/mod.rs:490-493` documents `Harness::read_approval` as existing for exactly this — *"the response can be made to say anything, and a test that only asserted the response would pass against a handler that minted an id and opened nothing. That is the exact defect D-225 records for the overlay submit's 202."* `rest_support::plan_state` (`:1208`) is likewise unused here. All five tests in the file are body-only.

*Fix/Verify:* add `assert_eq!(plan_state(&h, plan_id, 0).await.as_deref(), Some("published"))` and a `read_approval` on the id the body names. Prove both by making the handler retire directly and by making it skip the submit.

**Z12-4 [High] The bundle-publish materiality assertion compares the handler's literal against itself — and this is where the suite would have caught F-1/F-2/F-3**
**PAID `8a77c74d9`.** See the status ledger at the top of this document.

- `tests/rest_bundles.rs:451-457` — `assert_eq!(body["materiality"].as_str(), Some("alwaysMaterialTrigger"), "D-104: a composition publish is material whatever a threshold says")`.
- `src/api/rest/bundles.rs:510` — `materiality: "alwaysMaterialTrigger".to_owned(),` — an unconditional literal in the handler. The two sides of the assertion are the same constant; no input can make it fail.
- `src/infra/bundle.rs` — `grep -n "approval|submit|materiality"` returns only doc-comment lines (`:8`, `:18`, `:60`, `:61`, `:97`). There is **no** `approvals.submit` call: the publish opens no unit at all.
- `tests/rest_support/mod.rs:1755` — `approval_rows(harness)` exists and returns every unit the tenant holds.

One line — `assert!(!approval_rows(&harness).await.is_empty())` — in the test named `a_clean_publish_is_accepted_and_is_always_material` would have failed and surfaced F-1 (bundle publish evaluates no materiality). The helper was in scope; 200 lines later in the same file `a_composition_edit_voids_the_pending_unit_over_its_plan` (`:628`) uses `read_approval` twice (`:656`, `:684`) and even installs an explicit control assertion at `:654-663` against exactly this failure mode. The discipline exists in the file; it was not applied to the 202.

The contrast that proves the pattern was available: the *plan* publish route does it right — `rest_publish.rs:1053` and `:1101` assert `unit.materiality["reason"]` on the **stored** unit, not on the response. Bundles and retirement are where it lapsed.

*Fix/Verify:* assert the opened unit's stored `materiality` on the bundle publish, as `rest_publish.rs:1053` does; the response-body assertion may stay beside it but must not be the only one.

**Z12-5 [Medium] `GET/PUT /config/tax-display-policy` has no behavioural test at any tier, and C4's `warn` branch never executes outside a domain unit test**

- `src/api/rest/tax_display_policy.rs` — 351 lines, five functions (`get_policy:180`, `put_policy:191`, `held_mode:268`, `read_scope:319`, `write_scope:336`), and `grep -n "cfg(test)"` returns nothing: no in-module test file, and no `tax_display_policy_tests.rs` exists under `src/api/rest/`.
- `grep -rn "TAX_DISPLAY_POLICY|tax_display_policy" tests/*.rs` → the only hits are `module_test.rs:141-142` (the route is *declared*), `module_test.rs:355` and `rest_authz.rs:1046` (the router is *merged*), and `rest_authz.rs:439/446/941` (the two census rows and the request `drive()` builds). There is **no** `rest_tax_display_policy.rs`.
- `rest_authz.rs:941-947` drives the `PUT` with a deliberately stale `If-Match` (`"000…0"`), so on the one allowing pass (`every_route_asks_the_catalogued_pair:1534`) the handler is refused before it writes, and the test asserts only that the status is not 401/403.
- `grep -rn "TaxDisplayPolicy" tests/` → nothing. `tests/sqlite_policy_object.rs` covers the caps and the defaults (its seven tests are `:123`, `:170`, `:223`, `:261`, `:298`, `:334`) and never touches `tax_display_policy_mode`.
- `src/infra/publish.rs:896` — `let tax_display_policy = policy.tax_display_policy()…` then `:932` `.with_tax_display(tax_display_policy, readiness)`. Since no test at any integration tier writes `tax_display_policy_mode = 'warn'`, this chain is only ever exercised with `FailClosed`.

The surface governs whether a `taxInclusive` row in a region declaring no tax rate may publish (module doc `:9-11`), and the module's central claim — that `warn` relaxes the rate arm while the **category** arm stays blocked whatever it says (`:12-14`) — is asserted nowhere that includes the wiring. The `warn` branch will hatch with green tests the first time an operator sets it.

*Fix/Verify:* a `rest_tax_display_policy.rs` with the bootstrap `GET` (200 + `fail_closed` + a tag), a `PUT` that flips to `warn` and re-reads it, a stale-tag 409 `STALE_VERSION`, and one publish that succeeds under `warn` and is refused under `fail_closed` on the same rate-incomplete row — plus the negative control that the category arm refuses under both.

**PAID `9ee2f9ea4`.** The route has REST-tier coverage and the `warn` branch executes; proved by reverting `inst-td-policy`'s arm and restoring it.

**Z12-6 [Medium] There is no census-level property for the unauthenticated caller, and no source scan for `require_authenticated`**

- `tests/rest_authz.rs` mounts four set-level properties over all 48 routes (`:1525`, `:1572`, `:1635`, `:1730`) and none of them uses `Harness::anonymous()`.
- `Harness::anonymous()` (`rest_support/mod.rs:667`) is called from exactly four places: `rest_approvals.rs:815`, `rest_plans.rs:44`, `rest_prices.rs:875`, `rest_publish.rs:883`.
- `grep -c UNAUTHORIZED rest_*.rs` → one occurrence each in `rest_approvals`, `rest_history`, `rest_prices`, `rest_plans`, `rest_frontier`, `rest_publish`, and one in `rest_authz.rs:1538` which is a *negative* (`status != UNAUTHORIZED`).
- `grep -rn "require_authenticated" tests/` → **no matches**. All 22 REST source files call it (`grep -rc` over `src/api/rest/*.rs`), but nothing in `tests/**` scans for it the way `every_mutating_router_applies_the_correlation_edge` (`:1464`) scans for `correlation::establish`.

The file's own module doc (`:12-14`) states the principle this violates: *"a route added later with no gate at all — a per-route suite grows a hole the moment somebody adds a route and forgets its test; a census driven from the registered path set fails instead."* Authentication is the one gate still proven per-route, on 6 of 48.

*Fix/Verify:* add `an_unauthenticated_caller_is_refused_on_every_route` over `census()` using `harness.anonymous()`, and add `"require_authenticated"` to the `scannable()`-based scan beside `correlation::establish`.

**PAID `6c0fb5e02`, and it closed a second gap on the way.** An unauthenticated refusal is now asserted over all 58 census rows, each paired with a positive control that must be neither 401 nor 403 — without which the property could pass vacuously. It also generalised the sibling `every_route_asks_the_catalogued_pair`, which read only `asked.first()` and therefore bound NOTHING a route asks second: it now compares the whole sequence as `(resource, action, require_constraints)` triples, with `require_constraints` carried per row because the withdraw's second question is deliberately unconstrained and a flat `true` would be armed wider than the claim. Owed and recorded in the property's own doc: `approve`'s second question is asked only on the repricing-run branch, which this seed never reaches, so it is proven nowhere.

**Z12-7 [Medium] The at-most-once gate has no Postgres proof, and two keyed creates never replay a key**

- `tests/sqlite_idempotency.rs:1-8` — "The gate is a `PRIMARY KEY` and an `INSERT ... ON CONFLICT DO NOTHING`… every assertion below is about what the *statement* did." 644 lines, SQLite only.
- `grep -ln "idempotency|IdempotencyGate" postgres_*.rs` → `postgres_migrations.rs`, `postgres_schema_bulk_operation.rs`, `postgres_schema_snapshot_provenance.rs`, `postgres_schema_repricing_journal.rs`, `postgres_schema_stores.rs` — all *schema* suites. There is no `postgres_idempotency.rs`: the claim/replay/mismatch behaviour is never run against the engine that runs in production, and its own doc (`src/infra/storage/repo/idempotency_repo.rs:170-172`) names a `RepoError::Db` arm for "the conflicting row being unreadable inside the transaction that just collided with it" — an engine-specific hazard with no engine-specific test. (Compounded by Z12-1: even if written, it would not run.)
- Route-level replay is proven on `POST /plans` (`rest_plans.rs:249`), the clone (`:387`), `POST …/prices` (`rest_prices.rs:183`), `POST …/price-windows` (`rest_windows.rs:3243`, `:3315`), bulk imports (`rest_bulk_imports.rs:123`, `:383`) and migrations (`rest_migrations.rs:87`, `:181`).
- It is **not** proven on `POST /bundles` or `POST /price-overlays`: every `idempotency-key` header in those two suites is a fresh `Uuid::now_v7()` — `rest_bundles.rs:56,99,123` and `rest_overlays.rs:56,141,185,230,262,404,529,590`. No second call ever carries a key a first call used, so the gate's duplicate branch is unreached at those routes.
- `IDEMPOTENCY_PAYLOAD_MISMATCH` is asserted at two routes only (`rest_plans.rs:312`, `:456`; `rest_windows.rs:3300`).

*Fix/Verify:* one replay case and one payload-mismatch case each on the bundle and overlay creates; and a `postgres_idempotency.rs` twin for the claim path once Z12-1 is fixed.

**PARTIAL — `7b84496d4`.** The Postgres proof now EXISTS AND WAS RUN (`postgres_idempotency` 5/5 with `-- --ignored`, including the two-duplicates-in-flight race), and `POST /bundles` replays its key. **`POST /price-overlays` remains ungated, and is worse than this entry describes:** it parses an idempotency key and then discards it (`let _client_key = …`), and there is no uniqueness index, so a retry creates a SECOND draft overlay. A route that demands a key and ignores it is more dangerous than one that never asked, because the caller has every reason to think a retry is safe.

**REMAINDER NOW PAID `f48e1ddb9`.** `POST /price-overlays` is wrapped in `idempotent::guarded` with the id minted inside the guarded body, `overlay_repo::create_on` lifts the three statements onto a caller-owned runner on `bundle_repo::create_on`'s shape, and the `Idempotency-Key` parameter is declared. The replay deliberately answers NO `ETag`: the performed path's tag names revision 0 / version 0, which a `PATCH` between the calls has already moved, and handing back a stale precondition marker is worse than handing back none. **The RED printed the defect itself — two different overlay ids out of one key.** The mismatch case is armed on a genuinely different body, the same-body replay is its positive control, and both assert the store, so the refusal is proved to have refused a WRITE rather than merely returned a status.

**Z12-8 [Low] `every_mounted_router_is_merged_into_both_censuses` scans raw source where its siblings scan stripped source — a commented-out merge satisfies it**

- `tests/rest_authz.rs:1268-1274` — `let text = std::fs::read_to_string(root.join(mount))…; assert!(text.contains(&needle), …)` with `needle = format!("rest::{module}::{function}(")`.
- `tests/rest_authz.rs:1302-1319` — `scannable()` exists and strips `//` comments, *"because this file's own prose names the type it bans, and a scan that matched its own explanation would have to be weakened until it matched nothing."* The two sibling scans (`:1366`, `:1486`) both use it; this one does not.

Commenting out a `.merge(...)` line leaves the needle in the file, so the guard passes over the exact regression it was written for (the 2026-08-04 sixth-router incident its own doc recounts at `:1181-1191`). The test also self-limits — `routers.len() >= 5` (`:1252`) against a gear that has 22 — but that floor is only an anti-vacuity check, not the property.

*Fix/Verify:* route the four mount files through `scannable()` before `contains`.

**PAID `74e06f316`, proved by removal as asked.** With a merge commented out the raw-text census stayed GREEN; routed through the stripped-source helper its siblings use, it reddens naming both file and router. Merge restored.

**Z12-9 [Low] Two of the four metrics instruments are never asserted from a live path**

- `src/domain/ports/metrics.rs:267-307` — the port is four methods: `preview_failclosed`, `currency_binding_block`, `tax_not_sellable_ga`, `alarm`.
- Proven through a real path: `preview_failclosed` in `rest_preview.rs:907-1011` (four cases, each with `force_flush()` and an explicit negative control at `:955-980`), and `alarm` in `sqlite_publish_commit.rs:2694-2740` (a before/after pair around a real `commit`) and `sqlite_read_model.rs:1886-1897`.
- Never proven through a real path: `pricing_currency_binding_blocks_total` and `pricing_tax_not_sellable_ga` (`src/infra/metrics.rs:52`, `:57`). `grep -rn "currency_binding|not_sellable" tests/*.rs` returns no metric assertion — the only callers are `src/infra/metrics_tests.rs:126-128` and `:271-272`, which invoke the port directly. Their live emitters are `src/infra/publish.rs:223` and `:545` and `src/infra/bundle.rs:220`; deleting any of the three leaves the whole suite green.

`rest_preview.rs:890-891` states the standard the other two miss: *"Asserted **through the router**, so what is proven is that a real refusal reported a real series, not that the adapter can increment its own counter."*

*Fix/Verify:* one `force_flush` + `counter_value` pair after a publish that raises `CURRENCY_NOT_COVERED`, and one after a gated-market publish.

**PAID `e39b43210` — and the probe uncovered something far larger than this entry.** The `tax_not_sellable_ga` half was already false when filed (proven live since `5578460b8`) and was left alone. Fixing the survivor produced a first RED reading `0` for a refusal the router had just returned, because `rest_support` built its `BundleService` with **no `.with_metrics(...)`** and held `NoopPricingMetrics` — eight lines under a comment claiming the harness holds the real adapter. Production wires it, so the counter was live in production and **unobservable in every test by construction**, for as long as that plane has existed. The same class as the fixture whose wildcard grant made thirteen authz scenarios untestable and green. Two probes then: emission removed → both blocked cases red; label chosen on the wrong basis → only one case red, which is the defect the emitter's own doc warns about and which no count-only assertion could ever see.

**Z12-10 [by-design] The retirement commit lane is absent, so `published → retired` has no producer and every consumer of a retired plan is tested against a state the gear cannot reach**

- `tests/rest_support/mod.rs:800-806` — *"Retirement is Slice 11's publish unit (D-128) and **has no producer in this gear at all**, so a suite that needs a retired plan has to write the state itself."* `Harness::retire` then issues a direct `UPDATE` (`:809-824`).
- `tests/rest_retirement.rs:107` confirms the live surface stops at `submitted_for_approval`.

Documented and spec-sanctioned, so filed as by-design — but the standing risk is the smell-8 one: the day the commit lane lands, whatever consumes `retired` will already be green against a state produced by a different writer than the one that will produce it. Worth an armed case at that point, not before.

**Verified clean:**

- **The harness turns nothing off.** `rest_support::Harness::client_as` (`:529-638`) merges **all 22 production routers** — I diffed the list against `src/module.rs:893-997` and it is the same set in the same shape, including `overlays::governance_router` mounted on `GovernanceState`. `.layer(axum::Extension(PolicyEnforcer::new(resolver)))` (`:631`) installs the real enforcer; there is no `gate: None` knob, no stub authz layer, and no in-memory repository double anywhere in the three harnesses.
- **The PDP doubles make fail-closed observable.** `DenyingResolver` (`:163`), `UnavailableResolver` (`:186`), `UnconstrainedResolver` (`:203`) and `RecordingResolver` (`:225`) are four genuinely different decisions, and `scope_mismatch()` (`:694`) is the only shape that exercises the write-target membership assertion — its doc says so and is correct.
- **A route gated on the wrong pair is catchable.** `every_route_asks_the_catalogued_pair` (`:1525`) reads the recorded `EvaluationRequest` and asserts resource, action **and** `require_constraints` for all 48 routes. Nothing else in the crate could see a `plan × read` gate on a mutating route.
- **The census cannot check itself.** `registered_paths()` (`:993`) builds from the routers and `the_census_covers_every_route_the_routers_register` (`:1105`) asserts set-*equality* both ways, plus that the label set is exactly the six decided ones (`:1154-1165`).
- **The `AccessScope` scan is alias-proof and self-tested.** `no_handler_can_build_an_access_scope_of_its_own` (`:1337`) bans the *import* rather than a spelling, and `the_scan_would_catch_each_evasion_the_earlier_one_missed` (`:1420`) proves all three historical evasions are now caught.
- **Fixtures are dated to be facts, not to expire.** `common/mod.rs:122-162` — `COVERAGE_FROM_UTC = (2099, 8, 4)` with a recorded post-mortem of the `2026-08-04` version that would have aged into failure, and an explanation of why it is adjacent to and not overlapping the window suites' own scale. `common/mod.rs:216-220` — the coverage helper has **no opt-out flag**, deliberately.
- **The corpus is checked against the real producer.** `corpus_snapshot_shape.rs:69-71` and `corpus_publish.rs:13-14` both `#[path]`-include `../examples/regen_registry/validator.rs` rather than re-authoring the projection, and `corpus_publish.rs:24` asserts the committed `registry.toml` is byte-identical to a fresh render. `every_kind_has_earned_its_publish_half` (`:114`) asserts `earned == ModelKind::ALL`, which is what stops the corpus tests passing vacuously on an empty corpus.
- **Metrics are read after a flush, with negative controls.** `rest_preview.rs:907/940/974/1002` all call `force_flush()` first — the failure mode `rest_support/mod.rs:342-344` warns about — and `a_successful_preview_counts_no_refusal` (`:958`) is a real negative control.
- **The plan-publish materiality is proven on the stored unit.** `rest_publish.rs:1053`, `:1057`, `:1101` assert `unit.materiality["reason"]` off the store, not off the response.
- **The approval pin is read back as a document, not as a hash.** `rest_approvals.rs:355-421` walks `pinned_content` field by field including the whole `windows` array against `common::COVERAGE_*`, and `an_activation_under_a_pending_unit_does_not_void_the_approval` (`:468`) carries an explicit note (`:462-463`) that its `activated == 1` assertion is load-bearing precisely because `== 0` would let the test pass without the interleaving occurring.
- **The pg harness has its own executed guard.** `pg_harness.rs:57` spawns a real second process so the prune's "is that pid alive" decision is asked about someone else's pid, and it is deliberately **not** `#[ignore]`d (`:6-8`) — the one Postgres-adjacent file that does run.

**Refutations:**

- *Suspected: the harness's `IdempotencyGate` is an in-memory double.* It is not — `src/infra/storage/repo/idempotency_repo.rs:127-131` holds only a TTL and claims against the `pricing_idempotency_dedup` table. `rest_support/mod.rs:450-452` further binds it to `LimitsConfig::default().idempotency_key_ttl()` with a note that a different expiry "would make the replay tests pass or fail on a knob rather than on the guarantee". Correct.
- *Suspected: `FixtureGate::load` fails open when the registry is missing.* It does not — `src/infra/fixture_gate.rs:53-60`: "a registry that cannot be read produces a *closed* gate… every publish of every row then fails with `FIXTURE_MISSING`."
- *Suspected: `RegionTaxReadiness` is always `empty()` in tests, so C4's readiness arm is never exercised.* Partly wrong: `sqlite_price_repo.rs:2053-2054` builds a real `readiness_for(region, category)`. It is only the *policy mode* half that is never varied (Z12-5).
- *Suspected: no golden digest pins the content-pin preimage, so a framing change would be invisible.* `tests/**` holds none — but `src/domain/approval/content_pin_tests.rs` exists with 43 test functions and is another reviewer's zone. Not filed.
- *Suspected: `rest_bulk_imports.rs` asserts response bodies only (it imports no store helper).* Its module doc's claim holds by a different route: it re-reads the run through the real `GET …/bulk-imports/{id}` (`:82`, `:154`, `:247`, `:346`), which is a store readback through a second surface. Adequate.
- *Suspected: the `every_mounted_router_is_merged` scan's `routers.len() >= 5` floor is the property.* It is only the anti-vacuity guard; the real assertion is the per-`(module, function)` loop at `:1265-1275`, and the pair-distinctness check at `:1258` is what repaired the `overlays::governance_router` blind spot. Only the comment-stripping gap (Z12-8) survives.

**Not covered:** the `src/**/*_tests.rs` in-module tests (other reviewers' zone) except where cited as the *only* home of a property; the `sqlite_*` repository suites read beyond their module docs and the specific sites cited — `sqlite_price_repo.rs` (4222 lines), `sqlite_read_model.rs` (3973), `sqlite_plan_repo.rs` (3116), `sqlite_publish_commit.rs` (3101) and `rest_windows.rs` (3391) were sampled at named line ranges, not read end to end; the bodies of the 24 `postgres_*` suites beyond their module docs and their `#[ignore]`/test-attribute census (they do not run, so their internal quality is moot until Z12-1 is fixed); `bss-fixtures` and `bss-fixtures-conformance` themselves; and `examples/regen_registry/validator.rs`, which the corpus suites include but which is a source file rather than a test.


---

**Consistency sweeps (whole-crate)**

Zone: eight class sweeps over `gears/bss/pricing/pricing` (crate `bss-pricing`) + `gears/bss/pricing/pricing-sdk`, at `6ae81d5ec`. Read-only throughout; no build, no test, no edit. Route census taken from the 25 `OperationBuilder::{get,post,patch,put,delete}` call sites and cross-checked against `tests/module_test.rs::declared_paths()`.

**S1 — Stringly-typed token where the enum exists**

Greps run:
- `grep -rn 'fn as_str' --include='*.rs' pricing/src pricing-sdk/src` → 62 `as_str` bodies; a script extracted all 187 distinct arm literals and grepped each one across the crate **excluding its defining file** and excluding `*_tests.rs`.
- `grep -rnE 'Column::(LifecycleState|State|PriceEligibility|Status|Phase|Kind|Transition|Outcome)\.(eq|ne|is_in|not_in)'` → 57 state-column SQL predicates, triaged one by one.
- `grep -rnE 'const [A-Z_]*(STATUS|STATE|KIND|PHASE|TRANSITION)[A-Z_]*: *&'`
- `grep -rn '"submitted_for_approval"\|OUTCOME_SUBMITTED'`

What the class contains: the crate is overwhelmingly disciplined — 49 of the 57 state predicates read `Enum::Variant.as_str()`, and `plan_repo::current_tokens()` (`infra/storage/repo/plan_repo.rs:1481`) even derives its token set from `LifecycleState::is_current_revision()` rather than spelling it. Three residues remain, below. I checked and **cleared** the false-positive direction: `"active"`/`"retired"` in `domain/taxonomy.rs` (a taxonomy entry's state) and `"active"`/`"expired"`/`"cancelled"` in `domain/window.rs` (a window's state) are two different vocabularies that happen to share text, not one duplicated token; `"published"` as a *wire outcome* (`api/rest/publish.rs:152`) and `"published"` as a *row lifecycle state* are likewise two concepts.

**CORRECTION, 2026-08-14 audit: THE BY-DESIGN JUDGEMENT RESTED ON A FALSE PREMISE.** `published → retired` DOES have a producer — `retirement::retire_in` step 6 into `plan_repo::retire_revision`, rendered by a mounted route and covered by its own suite — and it landed on 2026-08-07, THREE DAYS BEFORE this review was written. The review trusted a stale comment in the test harness claiming the transition "has no producer in this gear at all", and that comment is still in the tree. So the only real defect this entry names is that harness comment; the plane it says is absent has existed the whole time. A finding sourced from a doc rather than from the code inherits the doc's errors.

**Z13-1 [Medium] The row-lifecycle / eligibility vocabulary is respelled as literals in four modules, and one of them spells it into a `.ne()`**

`domain/lifecycle.rs:89` owns `LifecycleState::as_str` and `domain/scope_key.rs` owns `PriceEligibility::as_str`. Four modules bypass both:

- `infra/currency_binding.rs:58` — `const CURRENT_PLAN_STATES: &[&str] = &["published", "retired"];`, `:61` `const PUBLISHED: &str = "published";`, `:64` `const GRANDFATHERED: &str = "existing_grandfathered";`. The module imports neither enum. Used at `:101`, `:125`, `:126`.
- `infra/change_graph.rs:128` — `.add(plan::Column::LifecycleState.eq("published"))`, a bare literal.
- `infra/storage/repo/overlay_repo.rs:1706` — `if !matches!(state, "published" | "retired")`, `:1715` `if state == "retired"`, `:1749` `.eq("published")`, `:1762` `row.price_eligibility == "existing_grandfathered"` — in a module that at `:1157`, `:1194` and `:1799` correctly uses `OverlayLifecycle::…as_str()`. **That is the strongest tell in the sweep: one file, both spellings, 600 lines apart.**
- `infra/storage/repo/taxonomy_repo.rs:92` — `const PUBLISHED: &str = "published";` used at `:362`, `:379`, `:415` against `price::Column::LifecycleState` and `price_overlay::Column::LifecycleState`, in a module that imports and correctly uses its own `TaxonomyState` (`:474`, `:563`).

Two consequences differ in direction, and the second is the one that matters. `overlay_repo.rs:1706` and `currency_binding.rs:58` are *third and fourth hand copies of the rule `LifecycleState::is_current_revision()` already owns* (`domain/lifecycle.rs:134`), which `plan_repo::current_tokens()` derives correctly — so a widening of "current" reaches two of the four sites and misses two. And `currency_binding.rs:126` is a **`.ne()`**: every other eligibility predicate in the crate is an `.eq()`, so a token rename elsewhere makes those match zero rows (loud, fail-closed) while this one starts matching *everything*, counting grandfathered rows as add-on coverage and letting an `inst-cb-addon` publish through that should have been blocked. `infra/storage/repo/price_repo.rs:1919` writes the identical predicate as `PriceEligibility::ExistingGrandfathered.as_str()`, which is the shape this one should have.

*Fix/Verify:* replace each literal with the enum's `as_str()`; replace `CURRENT_PLAN_STATES` and `overlay_repo.rs:1706` with `plan_repo::current_tokens()` / `LifecycleState::is_current_revision()`. Verify by renaming one `as_str` arm locally and confirming the compile/predicate surface moves everywhere.

Also in this class, milder: `const ACTIVE: &str = "active"` is declared **twice**, in `infra/storage/repo/taxonomy_repo.rs:95` and `infra/storage/repo/overlay_repo.rs:834`, both against taxonomy `state` columns whose enum `TaxonomyState::Active.as_str()` exists at `domain/taxonomy.rs:220`. And `api/rest/overlays.rs:96` `const OVERLAY_ACT_REASON: &str = "alwaysMaterialTrigger"` respells `MaterialityReason::AlwaysMaterialTrigger.as_str()` (`domain/materiality.rs:260`) — the same literal F-2 filed at `api/rest/bundles.rs:510`, here only as an `unwrap_or_else` fallback.

**PAID `b534436bd` + `828784bcd`, and the site count was wrong three times over.** The fail-OPEN member went first: `currency_binding`'s `.ne(GRANDFATHERED)` was the only one where a token rename makes the predicate match everything and lets a publish through — every other member fails closed. Then twelve sites across five modules, where this plan's ledger had recorded three and the dispatch brief five; SIX of them were in `preview.rs`, a module no entry ever filed. Deliberately left: `OUTCOME_PUBLISHED` (the outcome vocabulary — collapsing it into `LifecycleState` is the two-vocabularies trap that has cost a session here) and `adjustment_of`'s wire-token parse arms.

**Z13-2 [Medium] Three handlers hard-code `submitted_for_approval` against a `pub(crate)` const whose own doc exists to stop exactly that**

`api/rest/publish.rs:143-149` declares the token `pub(crate)` and states the rule in as many words: *"`pub(crate)` because the window surface answers the same word for the same act … Two spellings of one outcome would make a client's `match` depend on which route it called."* Two surfaces obey it — `api/rest/windows.rs:1190` (`crate::api::rest::publish::OUTCOME_SUBMITTED`, with `:399` recording "imported rather than re-spelled") and `api/rest/overlays.rs:89`. Three do not:

- `api/rest/cutovers.rs:147` — `outcome: "submitted_for_approval".to_owned()`
- `api/rest/retirement.rs:255` — same literal
- `api/rest/supersessions.rs:245` — same literal

The suites pin the literal on all five surfaces (`tests/rest_cutovers.rs:106`, `tests/rest_retirement.rs:121`, `tests/rest_supersessions.rs:120`), so a rename of the const passes CI green while three endpoints keep answering the old word — F-3's shape, one plane over. Compounding it, the field is `pub outcome: String` on all six DTOs (`publish.rs:199`, `overlays.rs:290`, `cutovers.rs:98`, `retirement.rs:183`, `supersessions.rs:157`, `windows.rs:416`): a bare string where an enum belongs, so nothing catches the drift at the type level either.

*Fix/Verify:* import `publish::OUTCOME_SUBMITTED` at the three sites; better, lift the outcome vocabulary to an enum with `as_str()` so the DTO field is typed. Verify by grepping the literal — it should appear once, in `publish.rs`.

**PAID `5242f28c8`.** The three handlers use the const. `OUTCOME_PUBLISHED` was deliberately not folded in — it belongs to a different vocabulary.

**Z13-3 [Low] `BillingAnchorPolicy` has no declared roster, and its inverse is hand-enumerated in two modules**

`domain/contracts.rs:123` gives `BillingAnchorPolicy::as_str` three tokens but **no `ALL`**, unlike its immediate neighbour `ProrationBasis` (`domain/contracts.rs:180`), whose `ALL` doc says spelling it out is precisely so "adding a member is a change this array records … the drift `pricing.contracts.enum_drift` alarms on". Because there is no roster there is no `wire_token(…)` helper for it, and the token→enum direction is written out by hand twice: `infra/storage/repo/price_repo.rs:3573-3598` and `api/rest/prices.rs:1235-1254`. Both modules import the enum. The `ProrationBasis` field two lines below in the very same function (`prices.rs:1261-1266`) goes through `wire_token(… ProrationBasis::ALL, ProrationBasis::as_str)`.

Currently correct, and both arms fail closed on an unknown token, which is why this is Low rather than Medium.

*Fix/Verify:* add `BillingAnchorPolicy::ALL` and a parse that consumes it, keeping the `fixed_day`→`AnchorDay` pairing at the two call sites.

**S2 — Non-canonical error crossing the gear boundary**

Greps run:
- `grep -rnE 'Result<.*, *(DomainError|anyhow::Error|RepoError|String)>' pricing/src/api pricing-sdk/src`
- `grep -rnE '(permission_denied|failed_precondition|aborted|not_found|invalid_argument|already_exists|CanonicalError::internal|service_unavailable)\(' api/ infra/ domain/` excluding `infra/error_mapping.rs`
- variant/arm count: `grep -cE '^    [A-Z][A-Za-z]+(\(|\s*\{|,)' domain/error.rs` = **53**; `grep -cE '^\s+D::[A-Z]' infra/error_mapping.rs` = **53**; `grep -n '_ =>' infra/error_mapping.rs` = none.
- traced `CanonicalError::internal` → `libs/toolkit-canonical-errors/src/builder.rs:602,678,689` → `src/error.rs:274` and `src/context.rs:330`.

What the class contains: one finding, and three refutations that are worth as much.

**Refuted — there is exactly one ladder and it is exhaustive.** `infra/error_mapping.rs` carries the only `From<DomainError> for CanonicalError`. 53 variants, 53 arms, **no wildcard**, so the compiler is the coverage proof; nothing falls through to a 500. `api/rest/error.rs:1-6` states the split and holds to it (authz gate only). No handler hand-rolls a domain rejection: the only `CanonicalError::*` constructors outside the ladder are `api/rest/error.rs:24,29` (PEP gate) and three `state.db.conn()` failures in `api/rest/windows.rs:921,1273,1383`, which are genuine infrastructure faults correctly answered 500.

**Refuted — no internal diagnostic reaches the wire.** `infra/storage.rs:788-793` folds `RepoError::Db`/`CorruptRow` (which carry `format!("{e}")` of the sea-orm error) into `DomainError::Internal`, and the ladder renders it `CanonicalError::internal(format!("pricing: {detail}"))` (`error_mapping.rs:508`). That string lands in `InternalV1.description`, which is `#[serde(skip)]` (`libs/toolkit-canonical-errors/src/context.rs:331`); the wire `detail` is the fixed sentence set at `src/error.rs:277`, and `builder.rs:689-695` deliberately skips `with_detail` for `Internal`/`Unknown`. `Problem::from_error_debug` is the only leaky path and is documented "MUST NOT be used in production"; nothing in this gear calls it.

**Refuted — the same domain rejection gets one status everywhere.** Because every producer routes through `repo_failure` → the single ladder, `APPROVAL_NOT_PENDING`, `WINDOW_OVERLAP`, `STALE_VERSION` etc. carry one status and one code regardless of which of the six governance handlers surfaced them. The two 403 arms drop their detail rather than folding it into `reason`, which `error_mapping.rs:23-32` argues correctly (a `"CODE: detail"` reason would never `==` the code).

**PAID `2fb9ca14e`.** The compile gate was proved the hard way: it reddens only after the new variant is spelled everywhere else so the lib still compiles, which is the actual old failure mode — a bare grep or a half-applied variant would have proved nothing.

**Z13-4 [Medium] A permanent registry refusal is rendered as a retriable 503**

`domain/ports.rs:33` collapses all four `CatalogVersionRegistryError` variants into one:

```rust
pub fn registry_failure(err: &CatalogVersionRegistryError) -> DomainError {
    DomainError::CatalogVersionUnavailable(err.to_string())
}
```

`Unconfigured` and `Unreachable` are genuinely transient/deployment states; `Rejected` is not — the SDK types it *"The registry refused the request (an unknown SKU, a closed version)"* (`pricing-sdk/src/catalog_version_registry.rs:50-51`). The ladder maps `CatalogVersionUnavailable` to `CanonicalError::service_unavailable()` and logs the detail server-side (`infra/error_mapping.rs:498-501`), so the caller receives a bare 503 "Service temporarily unavailable" with no code and no discriminator. A publish blocked because the plan names a SKU the registry does not know is indistinguishable on the wire from a registry outage, and 503 tells the client to retry a request that will be refused identically forever.

The function's own doc anticipates the objection — *"The distinction between 'unreachable' and 'rejected' is diagnostic, not behavioural — neither produces a version"* — which is true of the **gear's** behaviour and false of the **caller's**: retry policy is exactly the behaviour a status code selects.

*Fix/Verify:* split `Rejected` onto a failed-precondition arm with its own code (the design set's `FIXTURE_MISSING`/`PLAN_RETIRED_NO_SUCCESSOR` shape), leaving the other three on 503. Verify by asserting two different statuses from the two registry stubs.

**S3 — Dead fields, both mirror forms**

Greps run: a script read every `pub struct Model` under `infra/storage/entity/*.rs` (36 entities, ~290 columns) and grepped the rest of the crate for a writer of each field; every hit was then hand-checked for the `ActiveValue::Set` / `col_expr` / `..content` spellings the naive grep misses. Reader side: `grep -rn '<field>' --include='*.rs'` per candidate.

What the class contains: two never-populated tables (one whole), one written-but-unread report, and two refutations.

**Refuted (a) — `pricing_price.resolved_tax_category` is written.** It looked never-set, but `infra/storage/repo/price_repo.rs:1035` writes it by `col_expr` inside the publish transaction, grouped by resolved value. I confirmed it is the *only* path to `published`: `grep -rn 'Column::LifecycleState,' -A3 infra/` shows exactly one `Expr::value(LifecycleState::Published…)` for the price plane (`price_repo.rs:1032`), and the only price insert builder (`prepare_draft`, `price_repo.rs:2794`) always writes `Draft` — so no row can reach `published` with a NULL category. Similarly `approval_threshold.{version,absolute_minor,percent_bp}` are written via fully-qualified `sea_orm::ActiveValue::Set` (`threshold_repo.rs:318-321`).

**PAID `1cc351062`.** `Rejected` now maps to a new `DomainError::CatalogVersionRejected` → **400** carrying `CATALOG_VERSION_REJECTED` with the registry's detail on the wire; the three transient variants keep the 503 with the detail suppressed. Split on the variant rather than on a rendered string. RED: `left: 503 / right: 400`. Urgency came from outside the entry: the Python e2e client suite that landed in `21830f724` is exactly the consumer that would have encoded the wrong retry policy.

**Z13-5 [Low] `pricing_operator_flag` has no writer, no reader and no repository — and one module doc says operators read it**

`grep -rn 'operator_flag::' --include='*.rs'` returns exactly one non-test hit outside the entity file itself: the migration registration (`infra/storage/migrations.rs:149`). The table exists on both backends with a four-value `CHECK` on `flag` and a secondary index `idx_pricing_operator_flag_by_flag (tenant_id, flag, set_at)` (`m20260802_000007_create_pricing_operator_flag.rs:38,44`); the SeaORM entity exists (`infra/storage/entity/operator_flag.rs`). Nothing constructs an `ActiveModel`, nothing queries it. Per the severity rule I grepped the readers first: nothing dereferences it on a live path, so this is a forensic/forward-dependency gap, not a break — and it is the load-bearing form of the class, an index on a forever-empty table.

The gear is honest about it in one place and wrong about it in another. `domain/plan_rules.rs:66-71` states it exactly: *"That store **exists** … and it has no repository. The absence is therefore the *signal*, not the storage — worth stating, because a reader who finds the table would otherwise reasonably conclude the feature is wired."* But `domain/projection.rs:127-129` says the drift flags *"live in `pricing_operator_flag` and **operators read them through the authoring surfaces**"* — no authoring surface reads them, and no repository could serve one.

*Fix/Verify:* correct `projection.rs:127` to match `plan_rules.rs:66`, or file the reader as owed. Verify with the grep above.

**PAID `49d349e26`**, docs.

**Z13-6 [Medium] Seven of `pricing_policy_object`'s eight content columns have no writer anywhere in the crate**

The only writer of the tenant policy object is `infra/storage/repo/policy_repo.rs:428-436`, reached from `PUT /config/tax-display-policy`, and it inserts

```rust
let row = policy_object::ActiveModel {
    tenant_id: Set(tenant_id),
    tax_display_policy_mode: Set(mode.as_str().to_owned()),
    updated_at_utc: Set(stamp.recorded_at),
    updated_by: Set(stamp.actor_principal_id),
    ..Default::default()
};
```

So `default_rounding_policy_ref`, `enforced_migration_notice_days`, `max_tier_bands_per_row`, `max_price_rows_per_plan`, `max_custom_interval_days`, `max_custom_interval_months` and `additional_required_descriptors` are permanently NULL / at their column defaults, for every tenant, forever. All seven are **read** (`policy_repo.rs:165-231`, `infra/publish.rs:921`, `MigrationService`), so this is not dormant plumbing — it is a governance record whose every read resolves to the deployment fallback.

The behavioural residue per column, from the readers' own docs:
- `default_rounding_policy_ref` (`policy_repo.rs:193`) can never be non-`None`, and the doc says `None` means *"**every** published row must carry its own `rounding_policy_ref` or the publish fails with `ROUNDING_POLICY_UNRESOLVED`"*. The per-tenant default PRD §17.4 contemplates is unreachable.
- `additional_required_descriptors` (`policy_repo.rs:180`) is permanently `[]`, so `DescriptorSetComplete::extending_v1` — D-152's whole mechanism — can never extend.
- `enforced_migration_notice_days` is permanently the 60-day floor; a tenant cannot lengthen its own notice.
- the four caps fall back to `LimitsConfig`, which is benign, but it means the schema's per-tenant caps and their `CHECK`s guard a value nothing can set.

*Fix/Verify:* either mount the authoring surface for the policy object, or record the seven columns as owed with the readers' fallbacks named. Verify by grepping for a second `policy_object::ActiveModel` construction.

**PAID `3f1042246` AS PROSE — declined to build, with citations.** The design set names exactly one authoring surface over this table (`04-currency-tax.md` §5, `05-governance.md` §5) and it is built; it names NO surface for the other seven — every `/config/*` path in the set is one of four, and none is theirs. D-152 records the carrier as provisional pending a settings gear not in this repo; D-49 contemplates an audited policy change and names no route; D-10/D-13 put a policy change in the approval workflow rather than a repository write. **The declaration is load-bearing regardless** — six CHECK constraints and two migration roster tests bind these column names — so "nothing writes it" was never grounds to drop them, only grounds to say so. Five false statements corrected, the sharpest being `policy_repo`'s "Read-only, deliberately. There is no upsert here" sitting in the same module as a function that opens "Upsert, because…". **A design question falls out of this and is owed a decision:** `default_rounding_policy_ref` can never be non-`None`, so the PRD's ratified "publish MUST resolve the tenant default rounding policy" is unreachable for every tenant. Fail-closed and safe, but unsatisfiable, and nothing says who mounts the surface.

**Z13-7 [Low] The warm sweep's eleven-counter report is discarded by its only production caller**

`infra/jobs/readmodel_warm.rs:208-236` defines `SweepReport` with eleven counters (`versions_complete`, `subjects_failed`, `frontiers_advanced`, `degraded_emitted`, …). Its only non-test caller is `module.rs:299`:

```rust
if let Err(e) = job.run(Utc::now()).await { tracing::error!(…); }
```

— the `Ok(SweepReport)` is dropped whole. The sibling ticker forty lines below does the opposite: `module.rs:472-473` matches `Ok(report) => Self::log_activation(&report)`, and `log_activation` emits an `info!` naming `activated/expired/failed/overdue`. So the pass `module.rs:157-161` calls the one *"without which `pricing_read_model` stays empty"* produces no per-pass operational signal at all, while its less load-bearing sibling does. The asymmetry across siblings is itself the finding.

Mitigating, and why this is Low: the two Critical alarms go to the metrics port and to `tracing::error!` independently (`readmodel_warm.rs:520,802`), and each failed subject is logged individually inside the projector (`infra/read_model.rs:397-403`). What is lost is the aggregate — including `degraded_emitted` and `versions_complete`, whose only other channel is `tracing::debug!` (`:854`, `:986`).

*Fix/Verify:* give `warm_pass` a `log_sweep` mirroring `log_activation`. Verify by comparing the two `*_pass` bodies in `module.rs`.

**S4 — Incomplete CRUD surface**

Greps run: 36 entities under `infra/storage/entity/` minus the eight excluded classes (`*_dedup`, `*_lock`, `outbox`, `approval_key`, `*_tombstone`, `read_model`, `pin_frontier`, `idempotency_*`), each matched against the 25-route census. F-4 (bundle composition) is filed and not re-reported.

What the class contains: one write-only business record, one already covered by Z13-6, and a read-shape asymmetry. Plus one clean refutation.

**Refuted — the window plane is *not* a write-only state machine.** `POST /prices/{priceId}/windows`, `PATCH`/`DELETE /price-windows/{windowId}` have no `GET` of their own, which reads as the classic case; but `GET /plans/{planId}/coverage` returns `KeyCoverageView.intervals: Vec<WindowIntervalView>`, and `api/rest/windows.rs:228-231` says *"Every window of the key, ordered, **every state included** — a cancelled one among them"*, with `WindowIntervalView.state` carrying `scheduled|active|expired|cancelled` (`:272`). The lifecycle is readable. Recording this because it is the shape the sweep was built to catch and it is genuinely covered.

**PAID `f361f686e`** — re-checked after `5ce7feb1a` (which added a counter to this very report) and still true, so the caller now acts on what it is handed.

**Z13-8 [Medium] `pricing_audit_log` is append-only with no reader — and the error ladder's justification for dropping 403 details depends on it being readable**

`infra/storage/repo/audit_repo.rs` exposes exactly one mutating entry point, `append` (`:361`), plus five ref-builders and a private `read_head` (`:440`) used only to find the next `seq`. There is no list, no page, no export, and no route: `grep -rn 'audit_log::Entity::find'` returns the one private hit. Every mutating path in the gear appends to it (D-12's seven-year actor trail), and nothing can read it back.

That is normally a plain Med/Low forensic gap, and I priced it that way — but it has a live dependant. `infra/error_mapping.rs:29-32` argues that dropping the detail from `SELF_APPROVAL_FORBIDDEN` and `REGION_SCOPE_DENIED` costs an operator nothing, *"because … the attempt itself is already on `pricing_audit_log` as a `deny` record carrying that id (`inst-tp-selfaudit`, `inst-rb-audit`) — **a durable trail rather than a log line**"*. The trail is durable and unreachable, so the compensating control the ladder names does not exist yet. The authz catalog already declares the two permissions the reader would gate on — `audit × read` and `audit × export` (`gts/permissions.rs:221,229`) — and nothing gates on either (see S6).

*Fix/Verify:* file the audit read/export surface as owed, and either soften `error_mapping.rs:29-32` or land the reader. Verify: `grep -rn 'audit_log::Entity' pricing/src` should show a list/page function.

**PAID `700e6a4af` — the design set names the reader, so the reader landed** (`rest_audit`, a new suite). RED at two levels: `ORDER BY` made to disagree with the cursor reddens by SKIPPING rows, which is the D-125 violation itself; the `deny` writer stubbed reddens the dependant case. This entry's "nothing gates on either permission" is false for `audit × read` — `/history` gates on it — but its core claim held: no reader of the table, and an error-ladder justification leaning on one. **Deliberately unbuilt, and stated in the code where the next builder meets it:** filters and `audit × export`, because the design set names filters without enumerating them and inventing a set would be the mistake this register already records twice. Residual: the walk's tie-break collation is backend-dependent, and there is no Postgres suite for it.

**Z13-9 [Low] Read-shape asymmetry across the record families**

From the census, no family has both a list and a by-id read, and each family is missing a *different* half:

| family | list | by id |
|---|---|---|
| plans | ✗ (no `GET /plans`) | ✓ `GET /plans/{planId}` |
| price rows | ✓ `GET /plans/{planId}/prices` | ✗ |
| price overlays | ✓ `GET /price-overlays` | ✗ (`PRICE_OVERLAY_BY_ID` is `PATCH`-only, `api/rest/overlays.rs:1168`) |
| migrations | ✗ | ✓ `GET /migrations/{migrationId}` |
| repricing runs | ✗ | ✓ `GET /repricing-runs/{runId}` |
| bulk imports | ✗ | ✓ `GET /bulk-imports/{operationId}` |
| approvals | ✓ | ✓ — the only complete pair |

The inconsistency is the finding rather than any single absence. The sharpest instance is overlays: `GET /price-overlays` narrows only on `scope_class` (`api/rest/overlays.rs:1075-1078`), so reading one known overlay means paging the tenant's whole overlay set with D-125's 100-row default.

*Fix/Verify:* decide the pattern once (the approvals pair) and record the deviations. Not a defect on its own; it is the shape a reviewer should be handed before the next surface is mounted.

**S5 — Contract richer than implementation**

Greps run: `grep -n 'query_param' api/rest/*.rs`, `grep -rn 'page_info\|PageInfo' api/rest/`, `grep -n 'Query<' api/rest/*.rs`, plus a read of every `.description(` block against its handler, and `git show --stat 6ae81d5ec`.

What the class contains: two findings, both freshly created by the HEAD commit's pagination work, and one refutation.

**Refuted — the unbounded-list class the HEAD commit closed has no siblings left.** All four collection GETs now carry a page contract: `/approvals` (`api/rest/approvals.rs:938`), `/plans/{planId}/prices` (`prices.rs:876`), `/price-overlays` (`overlays.rs:1102`) and `/history` (its own `{entries, next_cursor}`). `GET /plans/{planId}/coverage` returns an array with no cursor but is bounded by `max_price_rows_per_plan`.

**PAID `a80378716`**, docs — and the table was stale in four cells: four of the reads it lists as absent exist. Five deviations, not seven.

**CORRECTION, 2026-08-14 audit: the "stale in four cells" claim is OVERSTATED.** Two of the six absent cells are filled, not four — and both were built during this wave on 2026-08-11 rather than being stale at filing. The docs fix itself is accurate.

**Z13-10 [Medium] `GET /price-overlays` still advertises the ordering the HEAD commit removed, and declares none of its query parameters**

The registration at `api/rest/overlays.rs:1145-1149` reads:

> "Returns **every** revision, draft included, **ordered by precedence then id then revision**, optionally narrowed to one `scope_class`."

The repository now orders by `price_overlay_id, revision` and says so at length — `infra/storage/repo/overlay_repo.rs:755-767`: *"**Ordered by the cursor's key, not by precedence.** This list answered `precedence ASC, id ASC, revision ASC` before D-125's contract reached it … What a caller cannot do any more is assume the *sequence* it receives is precedence order"* — and the response DTO repeats the correction (`api/rest/overlays.rs:256-259`, "**Not** precedence order"). So the OpenAPI description, which is what a generated client's docs carry, promises exactly the guarantee the producer withdrew. Confirmed against the producer, not the test.

The same registration declares **no** `query_param` at all, while the handler takes `Query<ListOverlaysQuery>` (`overlays.rs:1057`) reading `limit`, `cursor` and `scope_class`. Its two siblings do declare them — `api/rest/prices.rs:611,617` and `api/rest/approvals.rs:737,743` each register `limit` and `cursor`. A generated client therefore cannot page the endpoint the HEAD commit paginated.

`GET /history` has the same declaration gap: `Query<HistoryQuery>` at `api/rest/history.rs:233` with `limit`/`cursor` (`:70-75`), the description *narrates* the pagination (`:195`), and no `query_param` is registered.

*Fix/Verify:* delete the ordering clause from `overlays.rs:1146`; add `.query_param_typed("limit", …)` / `.query_param("cursor", …)` / `.query_param("scope_class", …)` to both registrations, matching `prices.rs:611`. Verify against the emitted OpenAPI document, not against the handler.

**PAID `9b8697a25` for the open half; BOTH of its other claims are false, and in opposite directions.** The `/history` clause was already false when filed — that module registers its `limit` and `cursor` params, and the entry's grep looked for `query_param` while the code writes `.param(ParamSpec)`. But its "the two siblings do declare them" clause is false the other way: those siblings declare nothing, and neither do four more. ~~Six collection reads still advertise no page query~~ — **PHANTOM DEBT, withdrawn by the 2026-08-14 audit, and it was this controller's own.** Those reads DO declare `limit`/`cursor`, via `query_param`/`query_param_typed`, which push the identical `ParamSpec` the document reads. The claim came from grepping for one spelling of the call — the same mistake this very entry records about `/history`, made again by the person recording it. Seventh instance in this programme of a syntactic search answering a semantic question, and always in the direction of "there is a defect". RED: `left: []` against `right: ["cursor","limit","scope_class"]` on the emitted OpenAPI document. Found on the way and separately paid (`e08ff16d2`, `955c03985`): `/history` advertised `plan × read` while asking `audit × read`.

**Z13-11 [Low] `module.rs`'s roster of what is *not* mounted is false on five counts**

`module.rs:833-836` — the doc on `register_rest`, the function it describes:

> "…there is no overlay, bundle, customer-group, import, migration, bulk or preview table, no audit or history read, and no read-model resolution query."

`pricing_price_overlay`, `pricing_bundle`, `pricing_bulk_operation` and `pricing_migration` are all present under `infra/storage/entity/`, and their routers are merged **in this same function** at `module.rs:939`, `:935`, `:910` and `:974`. `history::router` is merged at `:901`, forty lines below the sentence saying there is no history read. Only "customer-group", "audit read" and "read-model resolution query" survive. Same class as F-6, and the same mechanism: a narrative list beside a roster leaves only one of the two true.

The paragraph directly above it (`module.rs:821-829`) got this right and says why — *"It used to open 'Fifteen routes' and enumerate them; the number was wrong by four the moment G3 and G4 mounted the window plane … A count beside a roster leaves only one of the two true, and it is never the prose."* The very next paragraph is the enumeration that lesson forbids.

*Fix/Verify:* replace the enumeration with the property, as the paragraph above already does.

**S6 — Defined-but-unwired**

Greps run:
- events: `for e in <14 CatalogEvent variants>; grep -c "CatalogEvent::$e"` excluding the definition file and tests; then `grep -rn 'outbox_repo::enqueue'` for the 14 enqueue sites.
- authz: extracted all 22 `(resource_type, action)` pairs from `gts/permissions.rs` by regex, and all pairs reached by `access_scope(...)` per route file.
- config: each of the 15 knobs and each of the 8 `JobsConfig`/`LimitsConfig` accessors grepped for a non-doc reader.
- jobs: the three tickers in `module.rs::serve` against `infra/jobs/`.

**PAID `a0947ca73` — and it was worse than filed, not merely stale.** The enumeration was false on SEVEN counts rather than the five recorded here: `customer-group` joined when its router was mounted, and `read_model_repo::delta_at` is reached from the mounted preview and sellability reads, a clause the entry never counted. The enumeration is withdrawn whole; the criterion and the two surviving absences are kept.

**Z13-12 [Medium] Two names of the "frozen" event set have no producer, and their row-plane analogues do**

`domain/events.rs:1-8` opens *"The **frozen** event-name set. Frozen means what it says: the names below are the contract, and a consumer subscribing to `PriceWindowActivated` is entitled to keep receiving it under that name forever."* Counting references outside the definition file:

```
PlanCreated: 0        PriceCreated: 2
PlanUpdated: 0        PriceUpdated: 2
```

every other one of the fourteen: ≥ 1. `PlanCreated` and `PlanUpdated` appear in `CatalogEvent::ALL` (`:71-86`), in `as_str` (`:91-92`), and in the migration that inserts every name of `ALL` into the event-name table (`m20260802_000060_…:6`) — and nowhere else. `outbox_repo` has a constructor and a dedup key for `PriceCreated`/`PriceUpdated` (`:424`, `:441`, `:934`, `:945`) and none for the plan pair; none of the fourteen `outbox_repo::enqueue` call sites is on the plan create/update path (`POST /plans`, `PATCH /plans/{planId}`). Nothing in `events.rs` or `outbox_repo.rs` records the absence as deliberate — contrast `events.rs:20-24`, which explicitly argues the *absence of a deletion event* as a design property.

So a consumer subscribing to `PlanCreated` — a name the module calls a frozen contract — will never receive one, while the exactly-parallel `PriceCreated` fires. The asymmetry across the two planes is the tell.

*Fix/Verify:* either enqueue the pair on the plan authoring paths or document them as reserved-unemitted, the way the deletion event's absence is documented. Verify with the per-variant count above.

**PAID `60b2c20c1` — built, because here the design set is explicit rather than silent.** `inst-pa-return` is a `p1` step naming the emission, PRD §4 is an unconditional MUST distinguished in the same sentence from the conditional names, and S10 §7 ("primitives ride `PlanUpdated`/`PriceUpdated`") fixed the emission point, so per-revision granularity was never an option and documenting these as reserved-unemitted would have contradicted a `p1` instruction. Each producer sits at one choke point rather than at call sites: `PlanCreated` in `plan_repo::create_draft_on` (the only implementation of "a plan comes into existence", covering both the REST create and the clone), `PlanUpdated` in `plan_repo::record_revision_mutation` (the rail six edit callers already share; the seventh, the D-145 abandon, stays silent and the discrimination is on the act). **Building the producers exposed two suites reading the outbox by a proxy:** `rest_publish` asserted "exactly one row in the outbox" as a stand-in for its event — a lone `PlanRetired` would have satisfied it — and `sqlite_publish_commit` asserted `events[0].seq == 1`, true only while the seed emitted one event. Both now assert the property, and neither repair moves a number.

**Z13-13 [Low] Seven catalogued authz pairs are gated on by nothing**

22 pairs declared in `gts/permissions.rs`; 15 reached by a route. Never gated: `audit × read`, `audit × export`, `customer_group × read`, `customer_group × write`, `historical_import × read`, `historical_import × write`, `bundle × read`. Correspondingly `actions::EXPORT` (`authz.rs:132`) and the labels `AUDIT`, `CUSTOMER_GROUP`, `HISTORICAL_IMPORT` are declared, registered as type-schemas at init (`module.rs:602-613`), and never passed to `access_scope`.

**The converse is clean, and that is the half that matters**: every pair a route asks for is declared. I enumerated the 20 route files' `resource_types::*` × `actions::*` usage and found no route gating on an uncatalogued pair.

`bundle × read` is F-4's authz mirror and is not re-filed. `historical_import` and `customer_group` are named as unbuilt in `module.rs:857` and `overlay_repo.rs:794-801` respectively, so those four are [by-design] and documented. The two that are **not** documented as owed are `audit × read` / `audit × export` — see Z13-8, where the permission exists, the data exists, and the reader does not.

**PAID `cf20dab9a` — stale by six of seven.** Only `audit × export` survives; `audit × read` gained its consumer this week. The guard is now pair-level rather than label-level, because the label-level one could not see the survivor by construction, and the remaining debt is stated. Both of this entry's "documented as unbuilt" citations are dead links.

**Z13-14 [Low] [by-design] `config.events_enabled` is read by nothing**

`grep -rn 'events_enabled'` over the whole gear returns four hits: the field (`config.rs:31`), one test assertion (`config_tests.rs:11`), and two doc comments. No code reads it. It is [by-design] and stated so at the point it would matter — `infra/storage/repo/outbox_repo.rs:36-41`: *"`config.events_enabled` (default `false`) gates **fan-out, not the row** … so `enqueue` is unconditional and never reads the flag."* Recording it because a knob whose gated component (the relay) does not exist in this repository is the S6 shape, and the marker is worth carrying forward.

**Refuted — every other config knob has a live reader.** I checked all 15 fields and all 8 duration accessors individually. The two that first looked orphaned, `catalog_version_overdue_after()` and `window_activation_overdue_after()`, are read at `infra/jobs/readmodel_warm.rs:520,802` and `infra/jobs/window_activation.rs:482` — my first grep missed them on the accessor suffix.

**Refuted — no declared job is unscheduled.** `infra/jobs/` holds exactly three (`readmodel_warm`, `window_activation`, `gated_markets`) and `module.rs::serve` spawns all three, each on its own lease key (`module.rs:60,71,77`), each joined at shutdown (`:207-209`). No migration is for a table nothing queries **except** `pricing_operator_flag` (Z13-5).

**S7 — Hand-enumeration of a composite key's axes**

Greps run: a script split every non-test `.rs` into function blocks and counted, per block, how many of `ScopeKey`'s ten axes (`plan_id, currency, region, price_overlay, phase, price_eligibility, charge_kind, cohort, meter, dimension_key`, in snake / camel / SeaORM-column spelling, comment lines stripped) it names; every block naming ≥6 was read. `domain/scope_key.rs:559-570` is the key; F-12 (`domain/cutover.rs:231 generation_key`) is filed and not re-reported.

**PAID `96cc82536`, docs — and the design set is not merely silent, it never mentions this flag or a relay AT ALL.** So nothing was implemented and both readings are written at the declaration with citations. "By-design" was the right label for the wrong reason.

**Z13-15 [High] Eight further sites build, compare or serialize the ten-axis scope key by hand; all eight are currently correct, and the last widening of this key missed three of them**
**PARTIALLY PAID `d67a401de`.** The four sites that *consume* a key are compile-gated through `ScopeKeyParts`. The three that build a key **from** a stored row or JSON (`to_scope_key`, `scope_key_columns`, `read_scope_key`) are **still open** — an accessor cannot reach them. Two entries in the table below were **already gated** before this: `is_sibling_of` and `to_generation` spell it `let Self {`, which this finding's `let ScopeKey {` grep missed.

The comparator F-12 measures against — `domain/evaluation_policy.rs:123 partition_row_fields` — destructures exhaustively, so it breaks at compile time when a field is added. None of the following does; each names the axes through accessors or columns, so a new axis is a silent omission:

| site | shape | axes named | currently |
|---|---|---|---|
| `domain/scope_key.rs:819` `impl Display` | ten-segment canonical rendering | 10 | correct |
| `domain/scope_key.rs:783` `is_sibling_of` | field-by-field comparison | 10 | correct |
| `domain/projection.rs:1144` `scope_key_value` | JSON members into the read model | 10 | correct |
| `domain/approval/content_pin.rs:1054` `put_scope_key` | the approval content digest | 10 | correct |
| `infra/storage/repo/price_repo.rs:2599` `scope_key_filter` | SQL `Condition` | 10 | correct |
| `infra/storage/repo/price_repo.rs:1409` `scope_key_columns` | tuple comparator over a stored row | 10 | correct |
| `infra/storage/repo/price_repo.rs:3313` `to_scope_key` | rehydration | 10 | correct, and partly compile-gated (7 positional args to `ScopeKey::new` + `with_usage_line`) |
| `infra/storage/repo/read_model_repo.rs:462` `read_scope_key` | rehydration from JSON | 10 | correct, same partial gate |
| `infra/storage/repo/price_repo.rs:1377` `market_columns` | tuple comparator, **8 by design** | 8 | correct *by intent* — "all of them but `priceEligibility` and `cohort`" |

This is filed High rather than Medium because the class is not hypothetical here: **it fired once already and the code records what it cost.** D-196 widened the key from eight axes to ten, and the sweep that widened it reached `to_scope_key` and `scope_key_filter` and missed three sites, each of which then shipped a defect —

- `price_repo.rs:1398-1408`, verbatim: *"**It compared eight columns while the key had ten, from D-196 until 2026-08-06.** … so `refuse_mispaired` — whose whole sentence is about key *identity* — read a successor on a **different meter of the same market** as being on the predecessor's key."*
- `content_pin.rs:1040-1047`, verbatim: *"**It framed eight until 2026-08-06** … It was live for `put_key_windows`, where a key is framed with no row beside it — two window plans on two meters of one market pinned identically, so an approve could be satisfied by a re-derivation over the other line's coverage."* That is an approval-bypass: the Critical this class arms.
- `sellability::siblings` (now `scope_key.rs:783 is_sibling_of`), named in the same paragraph as the third miss.

Nothing structural stops a fourth. Two of the eight (`to_scope_key`, `read_scope_key`) get partial cover from `ScopeKey::new`'s positional signature, but that cover fails for exactly the widening D-196 performed — an axis pair added through a `with_*` builder rather than a constructor parameter, which is how `meter`/`dimension_key` arrived and how the next one plausibly will. `scope_key_filter`'s own doc states the invariant it cannot enforce: *"One spelling, so no statement here can decide 'the same key' by fewer axes than the key actually has"* (`price_repo.rs:2596-2598`) — it can, and it did.

*Fix/Verify:* give `ScopeKey` a `pub(crate)` exhaustive-destructure accessor (`let ScopeKey { plan_id, …, dimension_key } = key;`) and route all eight sites through it, so a new field is a compile error at every one — the shape `partition_row_fields` and `put_price_row` (`content_pin.rs:1073`) already use. Verify by adding an eleventh field locally and counting the compile errors: it must be eight, not two. A stale-count grep cannot find these sites, which is why the last sweep missed three.

**S8 — Tenant predicate in the SQL, not only in the decision**

Greps run: a script located every `Entity::{find,find_by_id,update_many,delete_many,insert}` in non-test code (≈150 sites) and checked each statement for `.secure()` and for a tenant term; `grep -rn 'allow_all'`; `grep -rnE 'Statement::from_(sql|string)|execute_unprepared|from_raw_sql|query_all\(|query_one\('`.

**This sweep found nothing, and the negative is a strong one.** Recording it in full:

- **Exactly one query in the crate omits `.secure()`**: `infra/storage/repo/window_repo.rs:772 projected_price_rows()`. It is a `SELECT price_id` sub-query, never executed on its own, and its own doc argues the case (`:762-766`): *"It yields `price_id`s only, `price_id` is a primary key, and a window's `price_id` names the row of its own tenant — so intersecting it with a scoped outer read cannot admit a window the outer scope did not already admit."* I verified the outer read is scoped (`window_repo.rs:693-701`, `.secure().scope_with(scope)` with the sub-query as an `in_subquery` term). Correct.
- **Every other query goes through SecureORM**, so the tenant predicate is compiled into the SQL from the PDP's `AccessScope` rather than being left to the handler's decision — which is the exact inverse of the failure this sweep hunts.
- **No `AccessScope::allow_all()` on a request path.** All five uses are in the three background sweeps (`readmodel_warm.rs:328,423,526`, `window_activation.rs:359`, `gated_markets.rs:101`), each a documented cross-tenant pass. I verified the narrowing claim rather than taking it: `window_activation.rs:399` takes `AccessScope::for_tenant(window.tenant_id)` and both the flip and the enqueue run under it (`:439`, `:442`); `readmodel_warm.rs:749` does the same per tenant.
- **No raw SQL outside migrations.** The single `Statement::from_string` is the migration runner (`infra/storage/migrations.rs:130`).
- The 57 flagged-by-script sites were all inserts whose `tenant_id` is on the `ActiveModel` (outside my 45-line window) or reads whose tenant term is built in a helper (`scope_key_filter`, `plan_rows_in_state`); each was checked by hand.

**Not covered:**

- **SQL inside migration bodies.** The trigger bodies in `m20260802_000040/51/55/57_guard_*` enforce the append-only column whitelists and I read them only for column names (S3); I did not verify each trigger's whitelist against the current `LifecycleState` machine. Per the standing rule that a rule mirroring a DB constraint must be checked against the constraint *as it now stands* and later migrations drop and recreate, that check is owed and is not in this block.
- **Declared vs. actual response codes.** I confirmed no route declares a 422 (`grep -n 'error_422\|UNPROCESSABLE'` → nothing), honouring `error_mapping.rs:42-43`, but I did not diff each `OperationBuilder`'s declared `error_4xx/5xx` set against the statuses its handler can actually produce. Z13-10 is the one instance I hit while reading descriptions; a systematic pass over the 25 registrations would likely find more.
- **The test suites.** Sweeps S1–S8 excluded `*_tests.rs` and `tests/` from the "who spells this" side by design (a test may legitimately pin a literal); I only read tests to establish whether a literal is *pinned* (Z13-2).
- **Cross-document consistency** between the code and `docs/DESIGN.md` / `DECISIONS.md` / `design/*.md`. I quoted design ids only where the code itself cites them; verifying that a cited decision says what the code claims is spec-check's job, not this block's.
- **Platform crates** beyond the `toolkit-canonical-errors` rendering path traced for S2, and the `toolkit-db` SecureORM internals relied on for S8 — I verified the call shape at every site but took SecureORM's own tenant binding as given.


---

## Part I's "out of scope" table, re-checked at `6ae81d5ec`

The four entries Part I recorded as another session's uncommitted work have since been committed
(D-306…D-309 and the merge above them), so they are in the tree this part reviewed and are no
longer anybody else's to triage. Re-opened one by one:

| Part I entry | Status now |
|---|---|
| `repricing_runs.rs:663` — `cohort: request.cohort.map(Cohort::Generation)` skips the millisecond quantum | **Stands.** Now `:692`, inside `selector_of` (`:670`). `check_quantum` has four callers and none is this one; `ScopeKey::new` (`scope_key.rs:604`) refuses the same input, so the selector bypasses a validation the domain owns. Same shape as Z11-2 on the overlay plane — three planes now share it. |
| `repricing_runs.rs:641` — `selector_of` never applies `check_usage_line_axes` | **Stands.** No call in the file; the only caller is `ScopeKey::with_usage_line` (`scope_key.rs:641`). Sharpened by what the function's own doc claims at `:662-669`: *"Each axis is validated by the **same** constructor the authoring plane uses, so a selector cannot name a value no row could have been filed under … refused here rather than silently matching nothing and being reported as `RUN_SELECTOR_EMPTY`."* Two of its axes are not, and `RUN_SELECTOR_EMPTY` is exactly what they produce — contract richer than implementation, in the paragraph a reader trusts most. |
| `price_repo.rs:1908` — `load_published_for_selector` imposes no page bound | **Stands, and is documented rather than accidental.** Z7 read the declaration at `price_repo.rs:1895-1901` — *"There is no page bound, and that is a real edge"* — and confirmed it is reached live from `api/rest/repricing_runs.rs:402`. Recorded there as by-design-with-a-caveat rather than re-filed here. |
| `domain/repricing_tests.rs:3` — `cargo fmt --check` fails | **Resolved.** `cargo fmt -p bss-pricing -- --check` exits 0 at this commit. |
