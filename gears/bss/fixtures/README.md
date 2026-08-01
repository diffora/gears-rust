# BSS joint golden conformance fixtures

The shared, version-controlled corpus that pins the arithmetic on the seam between the
pricing catalog — which publishes structure and never computes a charge — and
Rating/Tariffs, which compute. Pricing's design set requires it to exist **before**
implementation: `PRD.md` §13, restated as assumption Q4 of
[`design/03-price-structure.md`](../pricing/docs/design/03-price-structure.md).

Design: `docs/superpowers/specs/2026-08-01-bss-joint-fixture-corpus-design.md`.

## Why numbers and not prose

Prose admits two readings and the readings differ by money. Over the bands
`[0, 1000) @ 5` and `[1000, ∞) @ 3`:

| `Q` | `graduated` (marginal) | `volume` Variant A (one rate on the whole `Q`) |
|---:|---:|---:|
| 999 | 4995 | 4995 |
| 1000 | 5000 | 3000 |
| 1500 | 6500 | 4500 |

At `Q = 1000` the volume bill *drops* by 2000, because the half-open interval puts 1000 in
the second band. Read that interval as closed and every boundary customer is billed wrong.
The corpus fixes where the cliff sits.

## Layout

- `corpus/` — the truth. Hand-authored TOML, reviewed as normative text, readable from any
  language. One directory per family.
- `corpus/registry.toml` — **generated, committed.** What pricing's `FixtureGate` reads at
  publish time.
- `bss-fixtures/` — the registry, and behind the `corpus` feature the loader and case
  model. No arithmetic. The only crate a gear may take as a **production** dependency, and
  it takes it with `default-features = false`.
- `bss-fixtures-conformance/` — the `CorpusEvaluator` trait, the runner, and the reference
  oracle. **dev-dependency only**, so no evaluator reaches a gear even transitively.

## What a gear actually compiles

The publish gate asks one question — *is this kind green* — and the answer is a small
generated file. So a gear takes this crate narrow:

```toml
bss-fixtures = { workspace = true, default-features = false }
```

That surface is `ModelKind` + `Registry` + `gate_open_for`, over `serde`/`toml`/`thiserror`.
The loader, the case model and `chrono` sit behind the `corpus` feature — a gear never reads
the twenty-odd case files at runtime and has no business carrying the ability to.

Every other build in the workspace turns `corpus` on, so the narrow build is easy to break
without noticing. `tests/production_surface.rs` compiles it on purpose:

```sh
cargo test -p bss-fixtures --no-default-features --test production_surface
```

## What "green" means

Three flags per variant, none set by hand:

| Flag | Earned by |
|---|---|
| `oracle` | the reference oracle reproduces every evaluation case |
| `publish` | pricing's validator reproduces every publish case |
| `rating` | rating's evaluator reproduces the same evaluation cases |

`FixtureGate` reads `oracle AND publish`. The oracle is the executable §17.2 and stands in
for Tariffs until Tariffs exists; requiring the `rating` flag would block every publish at
launch, since the rating gear has no code. When rating's evaluator lands it must **agree**
with the oracle — disagreement reddens the corpus rather than overriding either side.

### Who writes the file, and why it is a `--example`

The two halves are earned in **different places**, and neither can see the other. `oracle`
needs the reference oracle, which lives in `bss-fixtures-conformance`; `publish` needs
pricing's `PublishValidator`, which lives in the gear — and the harness is a
**dev-dependency** of the gear, because no evaluator may reach a gear even transitively.

So `registry_gen::build` takes the earned publish set as an **argument** and stays a pure
function of `(corpus, earned set)`, knowing nothing about who ran what. The one build in
which both halves are visible is an *example* target of the gear, which compiles with
dev-dependencies:

```
gears/bss/pricing/pricing/examples/regen_registry/   # writes registry.toml
gears/bss/pricing/pricing/tests/corpus_publish.rs    # asserts it is fresh
```

The freshness assertion lives beside the writer, and **only** there. Two crates each
asserting a different expected content is how a generated file starts flapping.

## Adding a case

1. Put the file in its family directory. The `family` field must match the directory; the
   loader rejects drift, because the directory is the index.
2. `[snapshot]` may carry **only** fields the design set marks frozen in
   `pricingSnapshotRef`. Anything the consumer supplies at evaluation time belongs in
   `[runtime]`. Both deny unknown fields, so the gear ownership boundary is checked when the
   corpus loads — D-60's per-subscription trailing lock cannot be written into `[snapshot]`
   at all.
3. Cite the clause the case encodes in `provenance`. It is mandatory.
4. Say **why** the expected number is what it is, in `why`. A number without a reason cannot
   be reviewed, and a green run over unreviewed numbers proves nothing.
5. Author the **whole row**, not the delta. A publish case reads as "the only difference is
   X" and that is what it asserts — but publish is asked of a row, and a row that would not
   publish on an empty key does not publish on an occupied one either. Every usage row
   carries `billingGranularity`; every tiered (`inst-tb-window`) and `package`
   (`inst-pk-window`) usage row carries `tierAggregationWindow`. Authored short, a
   supersession pair stops at `EVAL_POLICY_MISSING` and the guard under test is never
   reached.
6. Run `cargo test -p bss-fixtures -p bss-fixtures-conformance`.
7. Regenerate the registry and commit that regeneration **on its own**:
   `cargo run -p bss-pricing --example regen_registry`

## Coverage today

`tier-boundary` (gates `graduated`, `volume`), `package`, `per-unit`, and `flat` — together
they gate **every** catalog `modelKind`, which is what `inst-fx-gate` requires: it blocks
publish of *any* kind without a green fixture. Plus `proration` (AC #61), which gates no
kind at all, plus `supersession-continuity`, `level-aggregation` and `reserved`.

Gated is not the same as **answerable**, and the two are checked separately.
`check_kind_coverage` asks whether a kind is gated by a family; `check_publish_case_coverage`
asks whether its gate can ever *open*, because `publish` is earned per kind by a passing run
and a kind the corpus asks no publish question of earns nothing — for ever, and
indistinguishably from a run that failed. `flat` and `per_unit` sat in exactly that state.
Both checks run inside the registry generator, where absent coverage becomes a named build
failure rather than a `false` in a file.

**`trailing-tier` is deliberately unbuilt.** SEAMS M12 is open: rating has no counterpart
for `tierQualificationWindow` at all, so a fixture would pin one side of a contract the
other has not accepted — the opposite of what "joint" means. It reads as *declined*, never
as green, and a test asserts exactly that.

### Declining one case rather than a whole family

`trailing-tier` can be declined by not existing. A case cannot: omitting it would delete an
authored rule the design set already states. So a publish case may carry
`declined_until = "<slice>"`, which says the same thing at case granularity — nothing has
built the slice this case is authored against, so a subject's refusal is the **anticipated**
answer rather than a disagreement.

`reserved/consumption-on-level-rejected` is the one that carries it: it expects Slice 10's
`LEVEL_RESERVATION_CONSUMPTION_FORBIDDEN`, and the only price-row shape that exists is Slice
3's, which has nowhere to put `reservedRate` / `reservationFlavor`.

It suspends **evidence**, never the assertion:

- the verdict stays authored and stays checked — a subject that answers with the wrong
  verdict is red exactly as before, so the marker cannot make a case unfalsifiable;
- the case earns nothing and blocks nothing: it is not counted as coverage, and it does not
  hold back the kinds its successor shares with cases that *are* answerable;
- answering it **correctly** makes the declaration stale, which the runner reports and the
  regeneration prints. "Nothing can answer this yet" has to stop being written down the
  moment it stops being true.

## Two case kinds

`kind = "evaluation"` asks what something costs, or what share of a period is chargeable.
`kind = "publish"` asks what publish does with an authored change: it carries a
`predecessor` and a `successor` row and expects a verdict.

The two are separate Rust types, and the loader reads `kind` before parsing the rest. A
serde-tagged enum would have had to give up `deny_unknown_fields`, and rejecting stray keys
is what keeps the snapshot/runtime boundary honest.

A rejection must name its error code. "Publish fails" without saying how is not reviewable,
and the codes are part of the published contract — the integrity check enforces it.

Publish cases are answered by a `PublishValidator`, which **only the pricing gear
implements**. The reference oracle deliberately does not: reproducing the gear's validation
surface would mean checking the gear against a copy of itself.

The validator runs the successor's **row-shape** rules first and the **supersession unit
guard** second, reporting the first violation in that order — a malformed row is malformed
regardless of what it supersedes, and the pair guard is a comparison that only means
something between two rows the catalog would have accepted on their own. It returns an
`EvalError` **only** when a snapshot carries something the gear's row shape cannot hold; a
rejection by the rules under test is a verdict, which is the case doing its job.

Where the two readings differ, `tests/corpus_publish.rs` pins the open disagreements by case
id with the reasoning for each. That list is meant to be loud: it fails the moment either
side moves, which is when a human is due to decide which one was wrong. It is currently
empty, and the reasoning for each entry that was closed is kept there — the corpus was under-
authored in five cases and right in the sixth, and both outcomes are recorded because "who
was wrong" is the part a future reader needs.

## Two family roles

Not every joint fixture is a publish gate, so `_family.toml` states which it is:

- **`role = "publish"`** — blocks publish of the `modelKind`s it lists.
- **`role = "conformance"`** — a joint regression that blocks nothing. `proration` is this:
  AC #61 is a field-consumption contract shared with Subscriptions and Tariffs.

Stating the role is what separates "gates nothing deliberately" from "someone forgot the
list" — a publish family with no kinds and a conformance family *with* kinds are both
violations.

## What proration asserts, and why it is not money

`proration` reports a **unit ratio**, never an amount. Rating emits prorated components at
full intermediate precision and never rounds — Billing rounds — so a prorated minor amount
does not exist at the pricing↔rating seam, and a fixture must not invent one.

The unit follows the basis: days for the calendar bases, seconds for `by_second`, the whole
period for `whole_unit` and `none`. Keeping it integral is what makes rating's T-D-26 rule —
"a period's slice fractions sum to exactly 1" — an exact equality: the split case asserts
15 + 16 = 31 = the basis, so no day is billed twice or lost at the cut.

The canonical `prorationBasis` enum is pinned in code here, spelling included. `serde`'s
`snake_case` rule would render `calendar_days_30` as `calendar_days30`; the enum is adopted
verbatim across three gears under the CI gate `pricing.contracts.enum_drift`, so the wire
name is part of the contract and is set explicitly.

## Enumerated fields are enums, not strings

`tierAggregationWindow` and `billingGranularity` are pinned in code for the same reason and
in the same way. While they were `Option<String>` the corpus carried
`tier_aggregation_window = "billing_period"` in four case files and
`billing_granularity = "per_unit"` in three — a plausible synonym of `invoice_period`, and
the name of a `modelKind` written into a granularity field. Neither value appears in any
design document, both read as obviously correct, and nothing failed: they were only ever
noticed by a consumer that could not map them, which reported *itself* as unable to answer.

An undefined value is now a **load** failure. The corpus carries its own vocabulary, so a
value added or renamed on one side cannot ride into a case file on the other.

Families and kinds are **different axes** — nine families against five kinds, with
`tier-boundary` gating two — so a kind can quietly belong to no family at all. That is how
`flat` was missed. `check_kind_coverage` now asserts every kind is gated by some family, and
`check_publish_case_coverage` asserts every kind is asked at least one answerable publish
question; both run inside the registry generator, so a sixth kind cannot be added without a
family to cover it or a publish case to open its gate.

The unbuilt families are reported **declined**, never green. An empty family is not green
either: without that rule absent coverage would read as success, which is the exact failure
this corpus exists to prevent.

## The rule that matters

Expected values are never adjusted to make a run pass. If the oracle disagrees with the
corpus, both go red and a human decides which is wrong. A corpus that heals itself has
stopped being a specification and become a transcript of whatever the code happens to do.
