"""P1 — every decision that records a propagation surface has each named target
document cite the decision id.

Port of `tools/spec-check/src/invariants/propagation.rs`.
"""

import re

from ..corpus import split_lines
from ..decisions import DECISION_ID
from ..decisions import parse as parse_decisions
from ..finding import Finding, Severity
from ..targets import resolve, text_at

# Any label shaped like a `**Propagated…**` bold field, however punctuated.
# Deliberately loose: a backstop as strict as the parser it backstops is not a
# backstop. The parser's anchor and the old fallback *both* required the colon to
# sit outside the bold span, so `- **Propagated:** PRD §1.` matched neither — and
# colon-inside-bold is house style in these very documents (267 occurrences across
# the three gears). Dropping the colon requirement also catches
# `**Propagated** — PRD §1.` and `**Propagated**  : PRD §1.`
#
# Looseness is safe *because* this only runs when the primary parser already
# returned None. The literal `Propagated` keeps it from firing on the neighbouring
# `**Propagation status:**` label, which is a different field.
_PROPAGATED_LABEL = re.compile(r"\*\*Propagated[^*]*\*\*(?:\s*:)?")

_MISSING_SHAPE = re.compile(
    r"\A(" + DECISION_ID + r") claims propagation into (.+), "
    r"but that document never cites " + DECISION_ID + r"\Z"
)


def _propagated_label_count(register):
    """How many `**Propagated…**`-shaped labels the whole register carries. Used
    only to state the cost of a register P1 could not parse at all: this is the
    propagation surface that went unverified.
    """
    return sum(1 for _ in _PROPAGATED_LABEL.finditer(register))


def _unparsed_propagated_label(lines, all_decisions, i):
    """The literal label text (asterisks and any trailing colon included) when
    decision `all_decisions[i]`'s body still contains a `**Propagated…**`-shaped
    label that the exact anchor did not capture.
    """
    start = all_decisions[i].line - 1
    end = all_decisions[i + 1].line - 1 if i + 1 < len(all_decisions) else len(lines)
    body = "\n".join(lines[start:end])
    match = _PROPAGATED_LABEL.search(body)
    return match.group(0) if match is not None else None


def check(corpus, seams, loaded):
    """P1 over one corpus.

    `seams` is the cross-gear seam-ownership index built from every corpus the CLI
    loaded, so a `SEAMS <id>` target is checked against who actually owns `id`
    instead of a prefix guess. `loaded` is every corpus the CLI loaded, so a
    resolved *cross-gear* target is checked against the sibling document for real
    rather than dropped; a caller with no siblings passes `[]`.

    This check states its own coverage. A `DECISIONS.md` that exists but yields
    zero parsed entries produces one `P1/decision-register-unparsed` naming the
    gear, because a run that parsed nothing must never be reported the same way as
    a run that verified everything.
    """
    register = corpus.text("DECISIONS.md")
    if register is None:
        return []

    lines = split_lines(register)
    all_decisions = parse_decisions(register)
    findings = []

    if not all_decisions:
        # Before the id-shape widening, subscriptions' 19 populated claims went
        # unchecked and rating's table-shaped register went unchecked, and the CLI
        # printed a finding count that read exactly like a clean verdict for both.
        # A register with a genuinely empty propagation surface reports 0, which is
        # the correct, honest outcome — not a defect to engineer away.
        findings.append(Finding(
            "P1/decision-register-unparsed",
            Severity.LOW,
            "DECISIONS.md",
            None,
            "P1 cannot verify propagation for {}: DECISIONS.md yielded zero decision "
            "entries — no `#### <id> …` heading matched the recognised id shape (`D-NN`, "
            "optionally gear-prefixed, e.g. `SUB-D-01`) — {} `**Propagated`-shaped "
            "field(s) in that register went unchecked as a result".format(
                corpus.root(), _propagated_label_count(register)
            ),
        ))
        return findings

    for i, d in enumerate(all_decisions):
        raw = d.propagated
        if raw is None:
            # `propagated is None` conflates two shapes: legitimately nothing to
            # propagate, and a genuine propagation surface recorded under a label
            # the exact anchor does not recognize. Only the second is a defect —
            # widening the anchor to swallow it would hide the very gap this check
            # exists to find.
            label = _unparsed_propagated_label(lines, all_decisions, i)
            if label is not None:
                findings.append(Finding(
                    "P1/propagation-label-unparsed",
                    Severity.MEDIUM,
                    "DECISIONS.md",
                    d.line,
                    "{} carries a propagation label this parser could not read: `{}`".format(
                        d.id, label
                    ),
                ))
            continue

        resolved = resolve(raw, corpus, seams)

        # A citation the resolver understands *nothing* in used to yield zero
        # findings, because `resolve` populates `unresolved` only for tokens it
        # recognised. Guarded on the *whole* `Resolved` being empty, not just
        # `paths`: a citation whose tokens were recognised but unmappable already
        # reports `propagation-unresolvable` below, and reporting both would
        # double-count one defect.
        if resolved.is_empty() and raw != "":
            findings.append(Finding(
                "P1/propagation-uninterpretable",
                Severity.LOW,
                "DECISIONS.md",
                d.line,
                "{}: propagation citation `{}` names nothing the resolver recognises, "
                "so the claim was not verified at all".format(d.id, raw),
            ))

        for token in resolved.unresolved:
            findings.append(Finding(
                "P1/propagation-unresolvable",
                Severity.LOW,
                "DECISIONS.md",
                d.line,
                "{}: propagation target `{}` names no document the resolver can map".format(
                    d.id, token
                ),
            ))

        # Low, not Medium: the seam-id shape is deliberately gear-agnostic, so it
        # also matches any all-caps word written after `SEAMS` — `SEAMS TBD` yields
        # the id `TBD`. Tightening the shape is not available (`ASC` is a real seam
        # id with no digit). At Medium this single most likely false positive
        # failed the default `--max-severity medium` gate, and one that breaks the
        # build is qualitatively worse than one that prints a Low line.
        for ident in resolved.seam_undefined:
            findings.append(Finding(
                "P1/seam-undefined",
                Severity.LOW,
                "DECISIONS.md",
                d.line,
                "{}: propagation target `SEAMS {}` cites a seam id that no loaded "
                "gear's SEAMS.md defines".format(d.id, ident),
            ))

        for ident, owners in resolved.seam_conflicts:
            findings.append(Finding(
                "P1/seam-conflict",
                Severity.MEDIUM,
                "DECISIONS.md",
                d.line,
                "{}: propagation target `SEAMS {}` is defined in more than one "
                "loaded gear's SEAMS.md: {}".format(d.id, ident, ", ".join(owners)),
            ))

        cite = re.compile(r"\b" + re.escape(d.id) + r"\b")
        for path in resolved.paths:
            # Cross-gear targets used to be dropped by a bare `continue` — four of
            # pricing's decisions had their only cross-gear claim silently
            # unverified that way. A target no loaded corpus provides is
            # *reported*, never skipped. Low, because it is a statement about this
            # run's inputs rather than a defect in a document.
            text = text_at(corpus, path, loaded)
            if text is None:
                findings.append(Finding(
                    "P1/propagation-target-not-loaded",
                    Severity.LOW,
                    "DECISIONS.md",
                    d.line,
                    "{} claims propagation into {}, which no loaded gear corpus "
                    "provides, so the claim was not verified — pass that gear's docs "
                    "directory as another `--gear` to check it".format(d.id, path),
                ))
                continue
            if cite.search(text) is None:
                findings.append(Finding(
                    "P1/propagation-missing",
                    Severity.MEDIUM,
                    "DECISIONS.md",
                    d.line,
                    "{} claims propagation into {}, but that document never cites {}".format(
                        d.id, path, d.id
                    ),
                ))

    return findings


#: Pinned baseline of `P1/propagation-missing` findings against the live pricing
#: register, hand-derived on 2026-07-29 from the failure output of the drift test —
#: not by running the checker and trusting whatever it produces, which asserts
#: nothing. These 24 `(gear, decision id, target path)` triples are **debt, not
#: correctness**: pre-existing gaps left by the 2026-07-10 decision wave, confirmed
#: real by manual cross-check (PRD.md cites 34 *other* decision ids, so the citation
#: convention is genuine and broadly followed). Fixing them is a separate docs round
#: (tracked as **D-69**).
#:
#: Pinned as an exact set so a *new* gap fails immediately, and so a *fixed* gap
#: fails too — the list must be updated deliberately when the docs improve, never
#: left to quietly become a floor.
#:
#: Every entry names "pricing": this baseline is a snapshot of *one specific
#: corpus*, and `(id, path)` alone is not a unique key across gears. Without the
#: gear qualifier, a same-shaped finding from a different gear would be silently
#: swallowed as if it were this pinned pricing debt. The gear name here is baseline
#: *data* — it must never leak into `targets.py`'s resolution path or any
#: invariant's matching logic.
#:
#: Every entry is also an **in-corpus** target, and that is all this snapshot ever
#: could have covered: when it was taken, a resolved cross-gear target was dropped
#: without being verified. The one cross-gear gap that later surfaced — D-46 into
#: rating's SEAMS.md — was deliberately never listed here, and was closed by fixing
#: the document instead. This register holds *accepted* debt, whose contents are a
#: human decision; adding a brand-new finding would have buried it.
#:
#: Deliberate removals, hand-checked per the rule above:
#:
#: - `D-01 -> PRD.md` (removed 2026-07-31): the D-79 lane text added to PRD §9.2
#:   reads "…have no data source (the D-01 defect class)", and this invariant is
#:   *document*-granular — any `D-01` token in PRD.md satisfies it. The finding
#:   legitimately no longer reproduces, but note the residual: D-01's originally
#:   claimed sites (`fr-tax-display-basis`, §17.4) still carry no D-01 citation.
#:   That per-site gap is below P1's granularity and stays part of the docs round
#:   this debt tracks.
#:
#: - `D-25 -> PRD.md` (removed 2026-07-31, the c-wave pin sweep): D-93's fix round
#:   (the 2026-07-31a slice review) rewrote `fr-plan-change-contract` with
#:   "…(D-93, revising D-25's publish-time stamp…)" — a real D-25 citation at the
#:   exact site D-25 claimed. Same document-granular satisfaction as D-01 above,
#:   but here the citation sits in the claimed FR itself. Hand-checked at HEAD
#:   before removal (`git show HEAD:…PRD.md | grep -c D-25` = 1).
#:
#: - `D-40 -> design/10-advanced-primitives.md` (removed 2026-07-31, the c-wave
#:   pin sweep): the 2026-07-31b review's L-2 fix declared the
#:   `tier_qualification_window` column in S10 §6 with "(D-40; the lock itself is
#:   Rating-owned — D-60, `inst-tt-lock`)" — the design doc now cites D-40 twice.
#:   Hand-checked at HEAD before removal.
PINNED_PROPAGATION_GAPS_2026_07_29 = (
    ("pricing", "D-02", "ADR/0001-cpt-cf-bss-pricing-adr-canonical-scope-key.md"),
    ("pricing", "D-02", "DESIGN.md"),
    ("pricing", "D-02", "PRD.md"),
    ("pricing", "D-02", "design/01-foundation.md"),
    ("pricing", "D-02", "design/07-pricewindow-linkage.md"),
    ("pricing", "D-04", "PRD.md"),
    ("pricing", "D-05", "PRD.md"),
    ("pricing", "D-06", "PRD.md"),
    ("pricing", "D-07", "PRD.md"),
    ("pricing", "D-13", "PRD.md"),
    ("pricing", "D-15", "PRD.md"),
    ("pricing", "D-16", "PRD.md"),
    ("pricing", "D-19", "PRD.md"),
    ("pricing", "D-20", "PRD.md"),
    ("pricing", "D-24", "PRD.md"),
    ("pricing", "D-28", "PRD.md"),
    ("pricing", "D-32", "PRD.md"),
    ("pricing", "D-35", "PRD.md"),
    ("pricing", "D-39", "PRD.md"),
    ("pricing", "D-41", "DESIGN.md"),
    ("pricing", "D-60", "design/03-price-structure.md"),
)

_PINNED_PROPAGATION_SET = frozenset(PINNED_PROPAGATION_GAPS_2026_07_29)


def missing_pair(finding):
    """Parses a `P1/propagation-missing` finding's `(decision id, target path)`
    from `check`'s own fixed message template. `None` for any other invariant tag
    or a message that doesn't match — the single production-and-test definition of
    "how to read this finding back into pinned-baseline shape".

    Deliberately does not, and cannot, recover a gear from the `Finding` alone: a
    `Finding` carries only a corpus-relative path, never a gear qualifier. Callers
    matching against a gear-qualified baseline must supply the gear themselves.
    """
    if finding.invariant != "P1/propagation-missing":
        return None
    match = _MISSING_SHAPE.search(finding.message)
    if match is None:
        return None
    return (match.group(1), match.group(2))


def is_pinned_baseline(finding, gear):
    """True if `finding`, attributed to `gear`, is exactly one of the pinned,
    accepted-debt propagation gaps rather than newly appeared drift.
    """
    pair = missing_pair(finding)
    if pair is None:
        return False
    return (gear, pair[0], pair[1]) in _PINNED_PROPAGATION_SET
