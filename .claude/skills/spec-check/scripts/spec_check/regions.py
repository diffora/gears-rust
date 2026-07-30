"""Candidate regions for a requirement: term-overlap windows and id anchors.

Neither mechanism needs a traceability convention, which is why this layer runs
on the 23 gears nobody has looked at as well as on the two that are measured.

Every constant here is a **starting value**, labelled as such. They came from one
measured run (the D-15 premise run: 8-line windows, top-5, one decisive region
and two mostly noise) and are tuned from the judge's per-region `usefulness`,
which the report aggregates — not from taste.
"""

from .corpus import split_lines
from .requirements import derive_terms, tokenize

#: Window geometry. Overlapping, so a rule stated across a window boundary is
#: still wholly inside some window.
WINDOW_LINES = 12
WINDOW_STEP = 6

#: A region's score is the **fraction** of the requirement's discriminating terms
#: the window carries — not a count of them.
#:
#: The design specified an absolute count (threshold 4, strong 8), derived from a
#: run over one-line `**Decision**:` fields. Measured 2026-07-30, that does not
#: transfer to requirements: a requirement's prose yields a median 33 terms in
#: pricing (max 161), so ~374 of the corpus's 1619 windows clear a threshold of 4
#: for the median requirement, top-3 always fills, and every one of the 116 live
#: requirements came back with 3–5 regions. Three of triage's six classes require
#: *exactly one* region, so they were unreachable, and the class was in any case a
#: function of how long the requirement's paragraph happened to be.
#:
#: A fraction is scale-free: a requirement of 8 terms and one of 161 are scored the
#: same way. These two values are the knee of the measured distribution, over
#: design documents only:
#:
#:   threshold  windows/requirement (pricing, ledger)  requirements with none
#:      0.50            3.3   3.6                          10   12
#:      0.60            1.4   0.8                          24   23
#:      0.70            0.6   0.3                          45   32
#:
#: 0.6 is where 0, 1 and 2 regions all occur naturally; 0.5 still fills top-3 and
#: 0.7 leaves half the corpus unmatched.
SCORE_THRESHOLD = 0.6

#: `covered:strong` needs this score *and* an id anchor. A single anchored region
#: scoring 0.65 is `weak-coverage`: the design set names the id there and may
#: still say little about it, which is exactly the shape that passes P2. At 0.75
#: about 20 of pricing's 76 requirements have a qualifying window.
STRONG_SCORE = 0.75

#: A term present in more than this fraction of the corpus's windows is not
#: discriminating and is dropped. Computed per corpus, never curated: `pricing`,
#: `plan` and `price` are noise in the pricing corpus and load-bearing terms in
#: another gear's.
DF_CUTOFF = 0.25

MAX_ANCHORS = 4
MAX_OVERLAP_REGIONS = 3

#: Including the requirement declaration itself, so at most 5 regions.
MAX_FRAGMENTS = 6


class Window:
    """A fixed slice of one document, with its term set precomputed once."""

    __slots__ = ("file", "start", "end", "text", "terms")

    def __init__(self, file, start, end, text, terms):
        self.file = file
        #: 1-based inclusive.
        self.start = start
        self.end = end
        self.text = text
        self.terms = terms

    def key(self):
        return (self.file, self.start)


class Region:
    """A window selected for one requirement, with why it was selected."""

    __slots__ = ("file", "start", "end", "text", "score", "matched", "selected_by")

    def __init__(self, file, start, end, text, score, selected_by, matched=None):
        self.file = file
        self.start = start
        self.end = end
        self.text = text
        #: The fraction of the requirement's discriminating terms this window
        #: carries, rounded to three places so the JSON stays diffable. Recorded
        #: for anchors too: "anchored" and "high-scoring" are independent facts and
        #: `covered:strong` requires both.
        self.score = score
        #: How many terms that fraction was of. A 0.5 over 8 terms and a 0.5 over
        #: 120 are the same class and not the same evidence, and the reader of a
        #: neighbourhood should be able to tell them apart.
        self.matched = matched
        #: `"id-anchor"` or `"term-overlap"`. Written to `neighbourhoods.json` for
        #: the report and for evaluation, and **hidden from the judge** — see
        #: `semantic.neighbourhood.render_for_judge`.
        self.selected_by = selected_by

    def __eq__(self, other):
        if not isinstance(other, Region):
            return NotImplemented
        return (self.file, self.start, self.end, self.score, self.selected_by) == (
            other.file, other.start, other.end, other.score, other.selected_by
        )

    def __repr__(self):
        return "Region({!r}, {}-{}, score {}, {})".format(
            self.file, self.start, self.end, self.score, self.selected_by
        )


class WindowIndex:
    """Every window of one corpus, plus the document frequency of every term.

    Built once per corpus and reused across all of its requirements: 116
    requirements over two corpora would otherwise re-tokenise the same documents
    116 times, and the document-frequency cutoff is a property of the corpus, not
    of a requirement.
    """

    __slots__ = ("_windows", "_document_frequency", "_lines")

    def __init__(self, windows, lines):
        self._windows = windows
        self._lines = lines
        counts = {}
        for window in windows:
            for term in window.terms:
                counts[term] = counts.get(term, 0) + 1
        total = len(windows)
        self._document_frequency = (
            {term: count / float(total) for term, count in counts.items()} if total else {}
        )

    @classmethod
    def build(cls, corpus):
        windows = []
        lines = {}
        for path, text in corpus.files():
            document = split_lines(text)
            lines[path] = document
            for start in range(0, len(document), WINDOW_STEP):
                chunk = document[start:start + WINDOW_LINES]
                if not chunk:
                    continue
                body = "\n".join(chunk)
                windows.append(Window(
                    file=path,
                    start=start + 1,
                    end=start + len(chunk),
                    text=body,
                    terms=tokenize(body),
                ))
        return cls(windows, lines)

    def windows(self):
        return list(self._windows)

    def lines(self, path):
        return self._lines.get(path, [])

    def document_frequency(self, term):
        return self._document_frequency.get(term, 0.0)

    def discriminating(self, terms):
        """`terms` minus those the cutoff rejects as corpus-wide noise."""
        return frozenset(t for t in terms if self.document_frequency(t) <= DF_CUTOFF)

    def containing(self, path, line):
        """The first window of `path` that contains `line`, or `None`.

        First, not best: windows overlap, and "the earliest window containing this
        line" is a rule with one answer, which is what keeps a run reproducible.
        """
        for window in self._windows:
            if window.file == path and window.start <= line <= window.end:
                return window
        return None


def is_substantive(region):
    """Whether this region is an *account* of the requirement, not just a citation.

    An id anchor is admitted regardless of score, because naming the id is precise
    evidence of intent — but a window that names the id while sharing almost none of
    the requirement's vocabulary is a citation, not a statement of the rule.
    `fr-addon-rules` has exactly that: one anchored region carrying one term of 53,
    score 0.019.

    The distinction is load-bearing for triage. Divergence needs two *accounts*: a
    citation cannot contradict anything, so counting it towards multiplicity sent
    110 of 116 live requirements to a judge, which is the cost the ladder exists to
    avoid.
    """
    return region.score >= SCORE_THRESHOLD


def _self_windows(index, requirement):
    """Window keys covering the requirement's own declaration and prose.

    Excluded from selection: without this, the highest-scoring region for every
    requirement is the paragraph that declares it.
    """
    last = requirement.prose_lines[1] if requirement.prose_lines else requirement.line
    out = set()
    for window in index.windows():
        if window.file != requirement.file:
            continue
        if window.start <= last and window.end >= requirement.line:
            out.add(window.key())
    return out


def _score(terms, window):
    """The fraction of `terms` present in `window`, and how many that was.

    `terms` is never empty here: `select` returns early when the requirement
    yields no discriminating term at all.
    """
    matched = len(terms & window.terms)
    return round(matched / float(len(terms)), 3), matched


def _anchor_regions(index, requirement, terms, excluded):
    """One region per document that names the id literally, capped.

    Cheap and precise, and — unlike term overlap — indifferent to the wording a
    gear happens to use.
    """
    out = []
    seen = set()
    for path in sorted({w.file for w in index.windows()}):
        for number, line in enumerate(index.lines(path), start=1):
            if requirement.id not in line:
                continue
            if path == requirement.file and number == requirement.line:
                continue  # the declaration naming itself
            window = index.containing(path, number)
            if window is None or window.key() in excluded or window.key() in seen:
                continue
            seen.add(window.key())
            score, matched = _score(terms, window)
            out.append(Region(
                file=window.file,
                start=window.start,
                end=window.end,
                text=window.text,
                score=score,
                selected_by="id-anchor",
                matched=matched,
            ))
            break  # one anchor per document: four documents beat four paragraphs
    return out[:MAX_ANCHORS]


def _overlap_regions(index, terms, excluded, declaring_file):
    """Top-scoring windows outside the declaring document.

    `declaring_file` is excluded wholesale, not just the declaration's own window.
    N1 asks whether the *design set* specifies the requirement, and the PRD is
    side A of that comparison: measured 2026-07-30, 57 % of pricing's term-overlap
    regions (125 of 220) were windows of `PRD.md` itself — neighbouring
    requirements sharing vocabulary with the one being judged — and each of them
    spent one of the five region slots. Duplication *within* one document is a real
    defect and a different check; id anchors are unaffected and still reach any
    document.
    """
    scored = []
    for window in index.windows():
        if window.key() in excluded or window.file == declaring_file:
            continue
        score, matched = _score(terms, window)
        if score < SCORE_THRESHOLD:
            continue
        scored.append((-score, window.file, window.start, window, score, matched))
    scored.sort(key=lambda row: row[:3])

    # Windows step by half their length, so adjacent ones share six lines and both
    # can clear the threshold on the strength of the same paragraph. Taking both
    # spends two of five region slots showing the judge one text twice — measured on
    # `fr-manual-adjustment-governance`, whose regions 3 and 4 were
    # `design/05:415-426` and `design/05:409-420`, the same seven governance steps.
    # The higher-scoring window wins because `scored` is already in that order.
    out = []
    for _negated, _path, _start, window, score, matched in scored:
        if any(window.file == kept.file and window.start <= kept.end
               and kept.start <= window.end for kept in out):
            continue
        out.append(Region(
            file=window.file, start=window.start, end=window.end, text=window.text,
            score=score, selected_by="term-overlap", matched=matched,
        ))
        if len(out) == MAX_OVERLAP_REGIONS:
            break
    return out


def select(index, requirement):
    """The regions for one requirement: anchors first, then term overlap.

    Anchors first because they are the precise signal and must never be crowded
    out by the heuristic one. The declaration counts against `MAX_FRAGMENTS`, so
    at most five regions survive.
    """
    terms = index.discriminating(derive_terms(requirement.prose))
    if not terms:
        return []
    excluded = _self_windows(index, requirement)
    anchors = _anchor_regions(index, requirement, terms, excluded)
    excluded = excluded | {(r.file, r.start) for r in anchors}
    overlap = _overlap_regions(index, terms, excluded, requirement.file)
    return (anchors + overlap)[:MAX_FRAGMENTS - 1]
