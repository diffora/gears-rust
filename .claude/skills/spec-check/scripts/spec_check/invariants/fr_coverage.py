"""P2 — every `…-fr-…` id defined in the PRD is claimed by exactly one slice's
`**Traces to**:` block, and no slice traces to an id the PRD never defines.

Port of `tools/spec-check/src/invariants/fr_coverage.rs`.
"""

import re

from ..corpus import split_lines
from ..finding import Finding, Severity
from .closure import is_design_slice

_DEFINED_ID = re.compile(r"\*\*ID\*\*:\s*`(cpt-cf-[a-z0-9-]*-fr-[a-z0-9-]+)`")
_ID = re.compile(r"`(cpt-cf-[a-z0-9-]*-fr-[a-z0-9-]+)`")

_DIRECTLY_ADDRESSES = "This slice directly addresses:"


def _defined_requirements(corpus):
    text = corpus.text("PRD.md")
    if text is None:
        return []
    return sorted({m.group(1) for m in _DEFINED_ID.finditer(text)})


def _collect_traces_to(path, text, out):
    """Ids inside a `**Traces to**:` block, which runs until the first blank line.

    A set per id, not a list: a copy/paste or line-wrap slip can repeat the same
    id in one block, and that is one dangling defect, not one per repetition.
    Returns whether the marker was seen at all — the caller needs both collectors'
    "did I see my marker" signal to tell a gear using neither convention from one
    using this convention exhaustively.
    """
    in_block = False
    seen = False
    for line in split_lines(text):
        if "**Traces to**:" in line:
            in_block = True
            seen = True
        elif in_block and line.strip() == "":
            in_block = False
        if in_block:
            for match in _ID.finditer(line):
                out.setdefault(match.group(1), set()).add(path)
    return seen


def _collect_directly_addresses(path, text, out):
    """Ids inside a `This slice directly addresses:` block.

    Unlike `**Traces to**:`, the marker is followed by a blank line before the
    bullets start (design/01-foundation.md's only occurrence: marker, blank line,
    six bullets, then end of file, no trailing heading) — so "stop at the first
    blank line" would collect nothing here. Instead: skip blank lines after the
    marker, collect ids from bullet lines, and stop at the next heading or the
    first non-blank, non-bullet line. Falling off the end closes the block too.
    """
    in_block = False
    seen = False
    for line in split_lines(text):
        if _DIRECTLY_ADDRESSES in line:
            in_block = True
            seen = True
            continue
        if not in_block:
            continue
        trimmed = line.lstrip()
        if trimmed == "":
            continue  # blank line between the marker and the bullets: stay open
        if line.startswith("#") or not trimmed.startswith("-"):
            in_block = False
            continue
        for match in _ID.finditer(line):
            out.setdefault(match.group(1), set()).add(path)
    return seen


def _claimed_requirements(corpus):
    """Collects ids claimed by slice documents, recognising the two conventions
    the design set actually uses, and returns `(claims, findings, convention_seen)`.

    Eleven of the twelve pricing slices use `**Traces to**:`; the twelfth
    (design/01-foundation.md) predates that convention and lists requirements
    under `This slice directly addresses:`. Both are equally valid ownership
    claims — the P2 question is "is this requirement claimed by a slice", not
    "claimed in a particular markdown shape" — so both are read. A document using
    the second shape still gets a divergence finding of its own: the checker must
    not silently absorb a shape split from the rest of the set.

    Scoped to `is_design_slice` documents only. Without this, a stray
    `**Traces to**:` in a non-slice document would count towards
    `convention_seen` (wrongly reviving the per-id sweep for a gear whose real
    slices use neither known convention) and could even wrongly "claim" a
    requirement no real slice claims.
    """
    out = {}
    findings = []
    convention_seen = False

    for path, text in corpus.files():
        if not is_design_slice(path):
            continue
        saw_traces_to = _collect_traces_to(path, text, out)
        saw_directly_addresses = _collect_directly_addresses(path, text, out)
        if saw_directly_addresses:
            findings.append(Finding(
                "P2/traceability-convention-divergent",
                Severity.LOW,
                path,
                None,
                "{} claims requirements via `This slice directly addresses:`; "
                "the rest of the design set uses `**Traces to**:`".format(path),
            ))
        convention_seen = convention_seen or saw_traces_to or saw_directly_addresses

    return out, findings, convention_seen


def check(corpus):
    """P2 over one corpus.

    Exactly one claiming slice, not at least one: the map has always been
    id -> *set of files*, but nothing ever reported `len() > 1`, so the ownership
    ambiguity this invariant exists to catch — two slices both asserting they
    implement one requirement, with no single owner to change when it moves — was
    live and undetected. Both halves are Low: they are the two directions of the
    same cardinality question, and neither proves a document states something
    false the way `fr-dangling` (Medium) does.

    A gear whose slices use *neither* recognised convention is not "fully
    uncovered" — it is unparsed. Emitting a `P2/fr-unclaimed` for every one of
    that gear's requirements would report confident-sounding gaps P2 has no basis
    for, which is worse than silence. So it emits exactly one
    `P2/traceability-convention-unknown` instead of the per-id sweep.
    `P2/fr-dangling` is unaffected either way.
    """
    defined = _defined_requirements(corpus)
    claimed, findings, convention_seen = _claimed_requirements(corpus)

    if convention_seen:
        for ident in defined:
            if ident not in claimed:
                findings.append(Finding(
                    "P2/fr-unclaimed",
                    Severity.LOW,
                    "PRD.md",
                    None,
                    "{} is claimed by no slice's Traces-to line".format(ident),
                ))
    else:
        findings.append(Finding(
            "P2/traceability-convention-unknown",
            Severity.LOW,
            "PRD.md",
            None,
            "P2 cannot verify requirement coverage for {}: no slice uses a recognised "
            "traceability convention (`**Traces to**:` or `This slice directly "
            "addresses:`) — {} requirement(s) went unchecked as a result; per-id claims "
            "are not reported for this gear rather than reporting every requirement as "
            "unclaimed".format(corpus.root(), len(defined)),
        ))

    defined_set = set(defined)
    for ident in sorted(claimed):
        files = sorted(claimed[ident])
        # Reported whether or not the PRD defines the id: two slices both claiming
        # an id is an ownership defect on its own, and an id that is *also*
        # dangling gets both findings because they are two different defects in
        # two different documents.
        if len(files) > 1:
            findings.append(Finding(
                "P2/fr-multiply-claimed",
                Severity.LOW,
                "PRD.md",
                None,
                "{} is claimed by {} slices, not one: {}".format(
                    ident, len(files), ", ".join(files)
                ),
            ))
        if ident not in defined_set:
            for path in files:
                findings.append(Finding(
                    "P2/fr-dangling",
                    Severity.MEDIUM,
                    path,
                    None,
                    "Traces to {}, which the PRD does not define".format(ident),
                ))

    return findings
