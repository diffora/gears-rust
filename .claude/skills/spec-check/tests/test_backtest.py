"""Backtest: the invariants against the pricing docs as of commit 10073c36, the
tree before the 2026-07-29 review fixes.

The design spec
(`docs/superpowers/specs/2026-07-29-bss-spec-ir-verification-design.md`) gates
promotion of this mechanism on rediscovery: >= 15 of the 24 mechanically-checkable
findings after migration steps 1-3. Step 1 ships three invariants, so the
rediscovery score here is a floor against that gate, not the final score — while
each per-invariant count below is an exact pin, for the reasons `PINNED_P1`
explains.

`test_records_the_step1_backtest_score` pins 28 P1 findings. **2 of those 28 are
not historical debt**: D-44 (`SEAMS M10`) and D-46 (`SEAMS RG3`) are real, defined
seam ids that read as `P1/seam-undefined` here only because this backtest loads the
pricing corpus alone, so no sibling SEAMS.md is loaded for the seam index to
resolve against. A real multi-gear run would not flag either. They are left in
the pin because the pin's job is "what this exact fixture, checked exactly this
way, produces" — but anyone reading 28 as "28 rediscovered defects" would
overcount by these 2.

One correction to the Rust original's doc comment, which describes these two as
"genuinely present in rating's and subscriptions' own `SEAMS.md`": both are rows
in *rating's* `SEAMS.md`, and `SeamIndex.build` over the live three-gear set
resolves each to `['rating']`. Subscriptions enters this class only through ids
that postdate this fixture: the live corpus raises four such findings, M10, RG3
and M12 owned by rating and SUB-P7 by subscriptions.

One deliberate deviation from the design spec, inherited from the Rust original:
its flagship example is D-53, and P1 raises no finding for it. D-53's own
`**Propagated**:` field names only S10 and S3 — both of which already cite D-53 —
and never names the PRD at all. P1 checks "every file this decision *names*
contains a reference to it"; a target that was never named is invisible to it by
construction. D-34 and D-36 are the mechanically-detectable version of the same
failure class.
"""

from conftest import REPO_ROOT
from spec_check.corpus import Corpus
from spec_check.invariants import closure, fr_coverage, propagation
from spec_check.targets import SeamIndex

#: Frozen copy of the pricing design set as of `10073c36`, byte-identical to
#: `git archive 10073c36 gears/bss/pricing/docs`. Never hand-edited: the pins
#: below fail in both directions, and the only reason they cannot flake is that
#: the input cannot change.
FIXTURE = (
    REPO_ROOT / ".claude/skills/spec-check/tests/fixtures/gears/bss/pricing/docs"
)

#: Pinned per-invariant counts, verified by hand against the corpus, not derived
#: from a first run. Exact, not a floor — the fixture cannot change, so these are
#: a stable pin that fails if the real count moves in *either* direction.
#:
#: A failure here is a real claim about the checker's effectiveness. Verify the
#: new count by hand before trusting it, then update these deliberately, in the
#: same commit as the change that moved them, with the reasoning in the commit
#: message. Never edit these to quietly re-baseline a failing run back to green,
#: and never edit the fixture to protect a pin.
#:
#: Both counts moved once, in the 2026-07-29 final-review fix wave:
#:   - P1 27 -> 28, by `P1/propagation-uninterpretable`. The fixture's register
#:     has exactly one citation with no recognised token, D-49's `§15 rows ×5.`
#:   - P2 3 -> 7, by `P2/fr-multiply-claimed`. Four fixture requirements are
#:     claimed by more than one slice — `fr-price-amount-validation` by 3 (slices
#:     01, 03, 04), and `fr-invoice-currency-binding`, `fr-per-seat`,
#:     `fr-mutation-idempotency` by 2 each.
#: P3 did not move.
PINNED_P1 = 28
PINNED_P2 = 7
PINNED_P3 = 55
PINNED_TOTAL = PINNED_P1 + PINNED_P2 + PINNED_P3


def historical():
    return Corpus.load(str(FIXTURE))


def test_the_fixture_is_present_and_has_its_twenty_documents():
    # A guard against the fixture move in Task 13 silently emptying this file's
    # subject: an absent fixture must fail loudly, not produce a zero score.
    assert FIXTURE.is_dir(), "the frozen 10073c36 fixture is missing: {}".format(FIXTURE)
    assert len(historical().files()) == 20


def test_rediscovers_d34_and_d36_never_reaching_the_prd():
    # D-34 and D-36 each declare `PRD fr-scheduled-migration` in their own
    # `**Propagated**:` field (unlike D-53, which never names the PRD at all), and
    # historically the PRD contained neither id anywhere. The 2026-07-29 review
    # closed both gaps with one new paragraph citing both. Neither id appears in
    # PINNED_PROPAGATION_GAPS_2026_07_29, confirming the gap was actually closed
    # rather than merely narrowed.
    corpus = historical()
    findings = propagation.check(corpus, SeamIndex.build([corpus]), [corpus])
    for ident in ["D-34", "D-36"]:
        assert any(
            f.invariant == "P1/propagation-missing"
            and ident in f.message
            and "PRD.md" in f.message
            for f in findings
        ), ident


def test_rediscovers_the_two_unclaimed_requirements():
    unclaimed = [f for f in fr_coverage.check(historical()) if f.invariant == "P2/fr-unclaimed"]
    assert len(unclaimed) == 2, (
        "expected fr-level-aggregation and fr-trailing-tier-qualification; got {!r}".format(
            [f.message for f in unclaimed]
        )
    )


def test_records_the_step1_backtest_score():
    # Fails in both directions: a regression collapsing the total to some other
    # nonzero number, or an unnoticed explosion, must be as loud as a drop to zero.
    corpus = historical()
    seams = SeamIndex.build([corpus])
    declared = closure.DeclaredInstructions.build([corpus])
    p1 = len(propagation.check(corpus, seams, [corpus]))
    p2 = len(fr_coverage.check(corpus))
    p3 = len(closure.check(corpus, declared))
    total = p1 + p2 + p3
    print("step-1 backtest: {} finding(s) against 10073c36 (P1 {}, P2 {}, P3 {})".format(
        total, p1, p2, p3
    ))

    drift = (
        "count drifted from the pin — this is a claim about the checker's "
        "effectiveness, not a number to quietly re-baseline: verify the new count "
        "by hand, then update it deliberately with the reasoning in the commit message"
    )
    assert p1 == PINNED_P1, "P1 {} (got {}): {}".format(PINNED_P1, p1, drift)
    assert p2 == PINNED_P2, "P2 {} (got {}): {}".format(PINNED_P2, p2, drift)
    assert p3 == PINNED_P3, "P3 {} (got {}): {}".format(PINNED_P3, p3, drift)
    assert total == PINNED_TOTAL, "total {} (got {}): {}".format(PINNED_TOTAL, total, drift)
