"""PRD requirement declarations, their normative prose, and the terms it yields.

New in milestone 3 — nothing here is a port; `tools/spec-check` never read a
requirement as prose, only as an id to count claims of.
"""

import re

from .corpus import split_lines
from .targets import gear_name

#: The shape of a requirement id: `cpt-cf-<gear>-(fr|nfr)-<name>`.
#:
#: The gear segment is **non-greedy** on purpose. Greedy,
#: `cpt-cf-[a-z0-9-]+-(fr|nfr)-` can consume the `n` of `-nfr-` into the gear
#: segment and match `fr`, which files every nfr in the corpus as an fr — 11 of
#: pricing's 76 requirements and 5 of ledger's 40. Non-greedy takes the first
#: `-fr-`/`-nfr-` boundary it finds, which is the intended reading.
REQUIREMENT_ID = r"cpt-cf-[a-z0-9-]+?-(?:fr|nfr)-[a-z0-9-]+"

#: Any `- [ ] \`pN\` - **ID**: \`…\`` declaration, whatever kind of thing it
#: declares. Used for *boundaries*: the prose block of one declaration ends where
#: the next begins, and the next may be a usecase or a contract.
_ANY_DECLARATION = re.compile(r"^- \[[ x]\] `(p[0-9])` - \*\*ID\*\*: `([a-z0-9-]+)`")

#: Which of those declarations are requirements.
#:
#: This filter is load-bearing, not decoration. Measured 2026-07-30: pricing's PRD
#: carries 92 declarations of which 76 are requirements (65 fr + 11 nfr, the rest
#: 8 usecase + 5 contract + 3 interface), and ledger's 48 of which 40 are (35 + 5).
#: Dropping the filter inflates every count and every pin in this layer by 21 %
#: and 20 % respectively.
_REQUIREMENT = re.compile(r"^cpt-cf-[a-z0-9-]+?-(fr|nfr)-[a-z0-9-]+$")


class Requirement:
    """One requirement declaration and the prose block that follows it."""

    __slots__ = ("id", "gear", "kind", "file", "line", "priority", "prose", "prose_lines")

    def __init__(self, id, gear, kind, file, line, priority, prose, prose_lines):
        self.id = id
        #: The gear the *corpus* belongs to (`pricing`), never the id's own gear
        #: segment (`bss-pricing`) — one notion of gear identity, shared with
        #: every other invariant via `targets.gear_name`.
        self.gear = gear
        #: `fr` or `nfr`. Both are requirements; the report distinguishes them.
        self.kind = kind
        self.file = file
        #: 1-based line of the declaration itself.
        self.line = line
        self.priority = priority
        #: The declaration's own normative prose — side A of every comparison.
        #: Structured sub-fields (`**Rationale**:`, `**Actors**:`) are deliberately
        #: *kept*: the judge should see the whole declaration. `derive_terms` drops
        #: them.
        self.prose = prose
        #: 1-based inclusive `(first, last)`, or `None` when `prose` is empty.
        self.prose_lines = prose_lines

    def __repr__(self):
        return "Requirement({!r}, {!r}, lines {!r})".format(self.id, self.file, self.prose_lines)


def parse(corpus):
    """Every requirement declared anywhere in `corpus`, in (file, line) order.

    The whole tree, not `PRD.md` alone. Measured on both live trees, every `fr`
    and `nfr` declaration lives in the PRD and nowhere else (pricing: 65 fr in the
    tree, 65 in the PRD; ledger: 35 and 35), so scanning the tree costs nothing
    today and a gear that declares requirements in `DESIGN.md` needs no special
    case tomorrow.
    """
    gear = gear_name(corpus) or ""
    out = []
    for path, text in corpus.files():
        out.extend(_parse_document(gear, path, text))
    return out


def _parse_document(gear, path, text):
    lines = split_lines(text)

    starts = []
    for index, line in enumerate(lines):
        match = _ANY_DECLARATION.match(line)
        if match is not None:
            starts.append((index, match.group(1), match.group(2)))

    out = []
    for n, (start, priority, ident) in enumerate(starts):
        kind_match = _REQUIREMENT.match(ident)
        if kind_match is None:
            continue  # a usecase / contract / interface: a boundary, not a requirement
        limit = starts[n + 1][0] if n + 1 < len(starts) else len(lines)
        end = limit
        for j in range(start + 1, limit):
            if lines[j].startswith("#"):
                end = j
                break
        prose, prose_lines = _prose_block(lines, start + 1, end)
        out.append(Requirement(
            id=ident,
            gear=gear,
            kind=kind_match.group(1),
            file=path,
            line=start + 1,
            priority=priority,
            prose=prose,
            prose_lines=prose_lines,
        ))
    return out


def _prose_block(lines, start, end):
    """`(text, (first, last))` for `lines[start:end]`, blank edges stripped.

    Returns `("", None)` for an all-blank block. That is a finding
    (`unbuildable:no-prose`), never a silent skip, so it must be representable.
    """
    first = start
    last = end - 1
    while first <= last and lines[first].strip() == "":
        first += 1
    while last >= first and lines[last].strip() == "":
        last -= 1
    if first > last:
        return "", None
    return "\n".join(lines[first:last + 1]), (first + 1, last + 1)


#: A closed list of English function words. Small and fixed on purpose: the
#: per-corpus job is done by the document-frequency cutoff in `regions.py`, not by
#: a hand-maintained stoplist that would rot per gear.
FUNCTION_WORDS = frozenset("""
must should with that this from when where each such they then than into over also only
""".split())

#: Minimum term length. Three-letter tokens (`row`, `per`, `bss`, `cpt`) carry no
#: discriminating power and would inflate every score equally.
MIN_TERM_LENGTH = 4

_BACKTICKED = re.compile(r"`[^`]*`")
_ID_STRING = re.compile(r"cpt-cf-[a-z0-9-]+")
_FIELD_LINE = re.compile(r"^\s*\*\*[A-Z][^*]*\*\*:")
_WORD = re.compile(r"[a-z0-9]+")


def tokenize(text):
    """The distinct terms `text` yields, as a frozenset.

    One pipeline, used for both a requirement's prose and a corpus window, so a
    score is a comparison between like and like. The order of the first three
    steps is not interchangeable:

    1. **Drop backticked spans.** Identifiers, field names and enum values are
       code, not prose; prose-vs-prose is what this layer compares.
    2. **Delete whole `cpt-cf-…` ids as strings.** The spec words this as
       "every token of the form `cpt-cf-…` is discarded", which cannot be done at
       token level: step 3 splits on `-`, so by then the id
       `cpt-cf-bss-pricing-fr-per-seat` has already become `pricing`, `seat`, …
       Gear names and id tails would enter the term set, and `pricing` matches
       nearly every window of the pricing corpus — one requirement would pull in
       the whole gear. Ids are anchors; terms are prose; mixing them loses both
       signals.
    3. **Tokenise, floor the length, drop function words.**
    """
    lowered = text.lower()
    lowered = _BACKTICKED.sub(" ", lowered)
    lowered = _ID_STRING.sub(" ", lowered)
    return frozenset(
        word for word in _WORD.findall(lowered)
        if len(word) >= MIN_TERM_LENGTH and word not in FUNCTION_WORDS
    )


def derive_terms(prose):
    """The search terms a requirement's own declaration yields.

    `tokenize`, with structured sub-field lines (`**Rationale**:`, `**Actors**:`,
    `**Owed**:`) removed first. The prose block deliberately keeps them — the
    judge should see the whole declaration — but a rationale states why a rule
    exists, not the rule, and its vocabulary pulls in regions that discuss the
    motivation rather than the mechanism.
    """
    kept = [line for line in split_lines(prose) if _FIELD_LINE.match(line) is None]
    return tokenize("\n".join(kept))
