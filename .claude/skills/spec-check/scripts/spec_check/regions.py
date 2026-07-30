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

#: Below this many distinct shared terms, a window is not a region at all.
SCORE_THRESHOLD = 4

#: `covered:strong` needs this score *and* an id anchor. A single anchored region
#: scoring 6 is `weak-coverage`: the design set names the id there and may still
#: say nothing about it, which is exactly the shape that passes P2.
STRONG_SCORE = 8

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

    __slots__ = ("file", "start", "end", "text", "score", "selected_by")

    def __init__(self, file, start, end, text, score, selected_by):
        self.file = file
        self.start = start
        self.end = end
        self.text = text
        #: Distinct discriminating terms shared with the requirement's prose.
        #: Recorded for anchors too: "anchored" and "high-scoring" are independent
        #: facts and `covered:strong` requires both.
        self.score = score
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
            out.append(Region(
                file=window.file,
                start=window.start,
                end=window.end,
                text=window.text,
                score=len(terms & window.terms),
                selected_by="id-anchor",
            ))
            break  # one anchor per document: four documents beat four paragraphs
    return out[:MAX_ANCHORS]


def _overlap_regions(index, terms, excluded):
    scored = []
    for window in index.windows():
        if window.key() in excluded:
            continue
        score = len(terms & window.terms)
        if score < SCORE_THRESHOLD:
            continue
        scored.append((-score, window.file, window.start, window))
    scored.sort(key=lambda row: row[:3])
    return [
        Region(
            file=window.file, start=window.start, end=window.end, text=window.text,
            score=-negated, selected_by="term-overlap",
        )
        for negated, _path, _start, window in scored[:MAX_OVERLAP_REGIONS]
    ]


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
    overlap = _overlap_regions(index, terms, excluded)
    return (anchors + overlap)[:MAX_FRAGMENTS - 1]
