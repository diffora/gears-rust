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
5. Run `cargo test -p bss-fixtures -p bss-fixtures-conformance`.
6. Regenerate the registry and commit that regeneration **on its own**:
   `cargo run -p bss-fixtures-conformance --bin regen_registry`

## Coverage today

`tier-boundary` (gates `graduated`, `volume`), `package`, `per-unit`, and `flat` — together
they gate **every** catalog `modelKind`, which is what `inst-fx-gate` requires: it blocks
publish of *any* kind without a green fixture. Plus `proration` (AC #61), which gates no
kind at all, plus `supersession-continuity`, `level-aggregation` and `reserved`.

**`trailing-tier` is deliberately unbuilt.** SEAMS M12 is open: rating has no counterpart
for `tierQualificationWindow` at all, so a fixture would pin one side of a contract the
other has not accepted — the opposite of what "joint" means. It reads as *declined*, never
as green, and a test asserts exactly that.

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
surface would mean checking the gear against a copy of itself. Until pricing's validator
exists the registry's `publish` half stays unearned, which is exactly what it reports.

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

Families and kinds are **different axes** — nine families against five kinds, with
`tier-boundary` gating two — so a kind can quietly belong to no family at all. That is how
`flat` was missed. `check_kind_coverage` now asserts every kind is gated by some family, and
it runs inside the registry generator, so a sixth kind cannot be added without a family to
cover it.

The unbuilt families are reported **declined**, never green. An empty family is not green
either: without that rule absent coverage would read as success, which is the exact failure
this corpus exists to prevent.

## The rule that matters

Expected values are never adjusted to make a run pass. If the oracle disagrees with the
corpus, both go red and a human decides which is wrong. A corpus that heals itself has
stopped being a specification and become a transcript of whatever the code happens to do.
