"""`#### D-NN` register entries. Port of `tools/spec-check/src/decisions.rs`."""

import re

from .corpus import split_lines

#: The decision-id shape a `####` register entry heading may carry: `D-NN`,
#: optionally carrying one or more uppercase gear prefixes (`SUB-D-01`, and a
#: future `T-D-01`).
#:
#: A convention, so it belongs in code — but it must stay *shape*, never a list of
#: gear names: `(?:[A-Z][A-Z0-9]*-)*` accepts any prefix a sibling gear might mint
#: without this module learning which gears exist. Pricing's register writes the
#: bare `D-NN` form and subscriptions' writes `SUB-D-NN`; before this shape was
#: widened the parser matched only the bare form, so all 19 of subscriptions'
#: populated `**Propagated**:` claims were never checked and the run reported clean.
DECISION_ID = r"(?:[A-Z][A-Z0-9]*-)*D-\d+"

_HEADING = re.compile(r"^#### (" + DECISION_ID + r")\b")

# Stop at the next *field label*: 8 of 68 real entries continue with
# `**Amendment …**:` on the SAME physical line, and a greedy `(.+)` swallows those
# paragraphs whole. The boundary requires the closing `**` to be followed by a
# colon, because every genuine label in this corpus carries one (`**Where**:`,
# `**Decision**:`, `**Owed**:`) while citations contain colon-less inline bold
# (`**Formalized as ADR-0003**` in D-03, `**SUB-P7**` in D-65) that must NOT end
# the capture.
#
# The left anchor accepts an optional parenthetical qualifier inside the bold span
# itself — `**Propagated (normative, 2026-07-28)**:` as well as the plain
# `**Propagated**:` — because a qualified label is still the same field, only
# annotated. `[^*)]*` cannot cross a `*` or a `)`, so the qualifier can never run
# past the closing `**` of its own bold span. A qualifier written any other way
# (`**Propagated pending**:`) is deliberately left unmatched: `propagated` comes
# back `None` for it exactly as for "nothing to propagate", and P1's
# `unparsed_propagated_label` fallback is what turns that shape into a Finding
# instead of a silent skip.
#
# `\Z`, not `$`: Python's `$` also matches immediately before a trailing newline,
# where Rust's matches only at end of haystack.
_PROPAGATED = re.compile(
    r"\*\*Propagated(?:\s*\([^*)]*\))?\*\*:\s*(.+?)(?:\s*\*\*[A-Z][^*]*\*\*:|\Z)"
)


class Decision:
    """One `#### D-NN …` entry of a gear's `DECISIONS.md`."""

    __slots__ = ("id", "line", "propagated")

    def __init__(self, id, line, propagated):
        self.id = id
        #: 1-based line of the `####` heading.
        self.line = line
        #: The `**Propagated**:` line, without the label. `None` when absent.
        self.propagated = propagated

    def __repr__(self):
        return "Decision({!r}, {!r}, {!r})".format(self.id, self.line, self.propagated)


def parse(text):
    """Parses decision entries.

    Status-board table rows mentioning `D-NN` are not entries and are deliberately
    not matched — only `####` headings are.
    """
    lines = split_lines(text)
    starts = []
    for i, line in enumerate(lines):
        match = _HEADING.search(line)
        if match is not None:
            starts.append((i, match.group(1)))

    out = []
    for n, (start, ident) in enumerate(starts):
        end = starts[n + 1][0] if n + 1 < len(starts) else len(lines)
        # The label and its citation sit on one physical line in this corpus, but
        # later prose (`**Amendment**: …`) may follow on that same line — the
        # regex above cuts the capture at that boundary.
        propagated = None
        for line in lines[start:end]:
            match = _PROPAGATED.search(line)
            if match is not None:
                propagated = match.group(1).strip()
                break
        out.append(Decision(ident, start + 1, propagated))
    return out
