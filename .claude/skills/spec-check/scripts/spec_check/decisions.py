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

# A line that *ends* the `**Propagated**:` field's own markdown block rather than
# continuing it. The field is authored as one block — a list item, or a bare
# paragraph — and a citation long enough to wrap has its remainder on the next
# physical line(s) with no marker of any kind. Everything below starts a *new*
# block and must not be swallowed:
#
#   `- ` / `* ` / `+ ` / `1. `  a sibling or nested list item. The register's real
#                               shape: D-158's `- **Amended by D-175 …**`,
#                               D-319's indented `  - **One of those five targets …**`,
#                               D-267's nineteen following bullets.
#   `#`                         a heading (`##### The finding`).
#   `|`                         a table row.
#   `>`                         a block quote.
#
# A blank line ends the block too, checked separately. Nothing here is a
# *widening* of what counts as the field: a continuation line is only read when
# the exact `**Propagated…**` anchor already matched on the line above it.
_BLOCK_BREAK = re.compile(r"^\s*(?:[-*+]\s|\d+[.)]\s|[#|>])")


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
        # The label and its citation sit on one *markdown block*, which is usually
        # one physical line — but not always. Reading the field line by line
        # resolved only the first line of a wrapped citation and dropped every
        # target below the wrap, unreported: D-313 cites over four lines and D-314
        # over six, and each had one line checked. The block is rebuilt here
        # (`_block`) and the field regex run over that, so later prose
        # (`**Amendment**: …`) still cuts the capture at its own boundary whether
        # it follows on the same physical line or a later one.
        propagated = None
        for offset in range(start, end):
            block = _block(lines, offset, end)
            match = _PROPAGATED.search(block)
            # The label must open on *this* line: the scan runs top-down, so the
            # first line carrying the anchor wins, exactly as it did before.
            if match is not None and match.start() < len(lines[offset]):
                propagated = match.group(1).strip()
                break
        out.append(Decision(ident, start + 1, propagated))
    return out


def _block(lines, start, end):
    """`lines[start]` joined with the continuation lines of its markdown block.

    Continuation lines are stripped and joined with a single space: the result is
    quoted verbatim into `P1/propagation-uninterpretable` findings and compared in
    frozen oracles, so it must stay one line. A block of one line — every entry in
    this corpus but two — returns that line unchanged, byte for byte.
    """
    out = [lines[start]]
    index = start + 1
    while index < end and lines[index].strip() and not _BLOCK_BREAK.search(lines[index]):
        out.append(lines[index].strip())
        index += 1
    return " ".join(out)
