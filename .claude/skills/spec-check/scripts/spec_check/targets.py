"""Maps the register's propagation shorthand onto corpus-relative paths.

Port of `tools/spec-check/src/targets.rs`.
"""

import re
from pathlib import PurePosixPath

from .corpus import split_lines

# A shorthand — and a corpus-derived document stem — is a **citation token**: it
# is recognised where it stands on its own, never where it is a piece of a longer
# word or filename. `\b` alone was not that rule. `\b` holds on both sides of the
# `Foundation` in "the third-**Foundation**-refusal paragraph" (D-172, live) and
# on both sides of the `PRD` in `../../rating/docs/**PRD**.md`, because `-`, `/`
# and `.` are all non-word characters — so an English compound and a path segment
# both minted a phantom claim into the *citing* gear's own document.
#
# Rejected on each side: a word character (what `\b` already did), a `-` (a
# hyphenated compound, or a longer id whose head this is), a `.` before (a piece
# of a dotted filename) and a `.` followed by an alphanumeric after (an
# extension — `PRD.md`). A trailing sentence period is deliberately still fine:
# "…propagated to DESIGN." is a citation, and `.md` is not.
#
# `/` is deliberately **not** rejected on either side, and that is a measured
# choice rather than an oversight. `DESIGN/README` is a real, correct live
# citation of `DESIGN.md` (D-03), and `S7/S11` is the same shape waiting to be
# written; rejecting `/` would silently drop both — the exact defect class this
# file is being repaired for. Path segments are excluded by *claiming the span*
# of the whole path (`_CROSS_GEAR`, `_MD_PATH`, `_MD_LINK`), which is exact,
# rather than by guessing from punctuation, which is not.
_TOKEN_BEFORE = r"(?<![\w.-])"
_TOKEN_AFTER = r"(?![\w-]|\.[A-Za-z0-9])"

_TOKEN = re.compile(
    _TOKEN_BEFORE + r"(S(\d{1,2})|Foundation|PRD|DESIGN|SEAMS|ADR-(\d{4}))" + _TOKEN_AFTER
)

# `[label](destination)` — a markdown link, which is the register's house style
# for a cross-gear document reference: `[rating PRD](../../rating/docs/PRD.md)`
# and `` [`DESIGN.md`](../../rating/docs/DESIGN.md) `` both appear in it.
#
# When the destination is itself a document target, the **label is part of that
# target and never a second claim** — the same doctrine that already governs a
# shorthand inside a bare path, extended to the form an author actually writes.
# Without it, `[rating PRD](../../rating/docs/PRD.md)` resolved to the rating PRD
# *and* to the citing gear's own `PRD.md`, and the phantom is the one that fails:
# it is why D-313's prescribed fix — write the cross-gear claim in the resolvable
# form — could not be applied at all.
#
# A link whose destination is not a document target (an anchor, an external URL)
# claims nothing, so its label's tokens are read exactly as before.
_MD_LINK = re.compile(r"\[([^\]]*)\]\(([^)\s]+)\)")

# Explicit cross-gear file target: `../../<gear>/docs/<path>.md`, the exact form
# `text_at` already knows how to read across loaded corpora and the SEAMS branch
# already mints. Until 2026-07-31 no *authored* citation could name one — the
# only cross-gear channel was `SEAMS <id>` — so an honest cross-gear file claim
# (D-66's, whose every target lives in rating/subscriptions) was forced to choose
# between `propagation-uninterpretable` and false `propagation-missing` against
# the citing gear's own same-named documents. A path this shape resolves as
# written; whether the named gear is loaded is the *caller's* concern
# (`propagation-target-not-loaded`), exactly as for a seam target.
_CROSS_GEAR = re.compile(r"\.\./\.\./([a-z0-9_-]+)/docs/((?:[A-Za-z0-9_-]+/)*[A-Za-z0-9_.-]+\.md)\b")

# An **own-gear document named by path**: `STRIPE-GAP-ANALYSIS.md`,
# `design/05-governance.md`, `./design/03-price-structure.md`. Corpus-relative, as
# every `Corpus` key is; `_normalize` folds a leading `./` away before the lookup.
#
# This is half of the answer to a whole class of silently-dropped claim. Until
# 2026-08-16 a propagation target had to be one of six *shorthands* (`S<n>`,
# `Foundation`, `PRD`, `DESIGN`, `SEAMS <id>`, `ADR-NNNN`) or an explicit
# cross-gear path. A target naming any other document of the citing gear's own
# corpus — `STRIPE-GAP-ANALYSIS.md`, which pricing's register cites twice — was
# dropped, and *not* reported as `propagation-uninterpretable` either, because the
# same citation carried shorthands that did resolve. The claim therefore read as
# verified while nothing had checked it. D-43 and D-319 are the live instances;
# the class is "every document the shorthand table does not happen to name".
#
# The lookbehind keeps this from firing on the tail of a longer path (the
# `../../<gear>/docs/…` form is matched by `_CROSS_GEAR` first and its span is
# excluded, but a path shape this class does not model must not be half-matched
# either).
_MD_PATH = re.compile(r"(?<![\w./-])((?:[A-Za-z0-9_.-]+/)*[A-Za-z0-9_-]+\.md)\b")

#: Shorthands with a dedicated branch in `resolve`. They are recognised whether or
#: not the corpus holds the document, so a corpus missing its `PRD.md` still gets
#: `propagation-unresolvable` rather than silence — `_corpus_stems` must never
#: shadow them into a corpus-derived, silently-absent vocabulary.
_DEDICATED_STEMS = frozenset(("PRD", "DESIGN", "SEAMS"))

# `\**` tolerates markdown bold around the seam id — the real citation in
# DECISIONS.md (D-65) reads "subscriptions SEAMS **SUB-P7**.", and without it that
# seam id is invisible to this regex, so the token falls back to bare SEAMS and
# misreports a resolvable citation as unresolved.
#
# Anchored at the start of the slice that follows THIS `SEAMS` occurrence (see the
# call site below), so a clause citing two sibling gears resolves each `SEAMS` to
# its own id instead of both collapsing onto whichever id appears first.
#
# The id shape itself is gear-agnostic: it matches every id family either live
# seam map actually uses (`K1`, `ASC`, `M12`, `RG3`, `SUB-P7`, `UC6`, …), not just
# the two families a prefix-inferring version would recognize. *Which gear* owns a
# captured id is decided below by looking it up in `seams` — never by the shape.
_SEAM_GEAR = re.compile(r"^\s+(?:§\w+\s+)?\**([A-Z][A-Z0-9]*(?:-[A-Z0-9]+)*)\b")

_SEAM_ROW = re.compile(r"^\|\s*\*\*([A-Za-z0-9-]+)\*\*\s*\|")


class Resolved:
    """What a propagation citation resolved to. Every list is sorted."""

    __slots__ = ("paths", "unresolved", "seam_undefined", "seam_conflicts")

    def __init__(self, paths=None, unresolved=None, seam_undefined=None, seam_conflicts=None):
        self.paths = paths or []
        self.unresolved = unresolved or []
        #: `SEAMS <id>` citations whose `id` is shaped like a seam id but which no
        #: loaded gear's `SEAMS.md` defines a row for — a dangling seam reference,
        #: which is a defect in the citing document, not a syntax miss.
        self.seam_undefined = seam_undefined or []
        #: `(id, gears)` for ids more than one loaded gear defines — a genuine
        #: cross-gear conflict, not a resolvable target.
        self.seam_conflicts = seam_conflicts or []

    def is_empty(self):
        """True when the citation yielded *nothing at all*.

        `resolve` populates `unresolved` only for tokens it recognised but could
        not map, so a citation containing no recognised token whatsoever
        (`§15 rows ×5.`) comes back completely empty and every downstream loop
        pushes nothing — a silent skip of a real propagation claim. Callers use
        this to say so instead (`P1/propagation-uninterpretable`).
        """
        return not (self.paths or self.unresolved or self.seam_undefined or self.seam_conflicts)

    def __repr__(self):
        return "Resolved(paths={!r}, unresolved={!r}, seam_undefined={!r}, seam_conflicts={!r})".format(
            self.paths, self.unresolved, self.seam_undefined, self.seam_conflicts
        )


def _normalize(path):
    """Lexically normalizes `path`: drops `.` components, folds each `..` against
    the one before it. Purely textual — no filesystem access, no symlink
    resolution — because the only shape it has to undo is the `../../<gear>/docs/`
    form `resolve` itself mints.

    Note the `..`-against-`..` case: Rust's `PathBuf::pop()` removes whatever the
    last component is, including another `..`, so `../..` folds to the empty path.
    That is reproduced deliberately, not fixed — changing it would change which
    cross-gear targets `text_at` can find.

    A root component is a component here, as it is for Rust's `Path::components`,
    but it is not joined with a separator: Rust's `PathBuf` renders RootDir plus
    `a` as `/a`, and `pop()` fails on a root-only buffer exactly as it does on an
    empty one (so `/..` stays `/..`).
    """
    out = []
    for component in PurePosixPath(path).parts:
        if component == ".":
            continue
        if component == "..":
            if out and not out[0].startswith("/"):
                out.pop()
            elif len(out) > 1:
                out.pop()
            else:
                out.append("..")
        else:
            out.append(component)
    if out and out[0].startswith("/"):
        return "/" + "/".join(out[1:])
    return "/".join(out)


def _strip_prefix(target, base):
    """Component-wise `Path::strip_prefix`. `None` when `base` is not a prefix."""
    t = target.split("/") if target else []
    b = base.split("/") if base else []
    if t[: len(b)] != b:
        return None
    return "/".join(t[len(b):])


def text_at(corpus, rel, loaded):
    """Text of `rel` — which may escape `corpus`'s own root via `../`, the
    cross-gear `../../<gear>/docs/SEAMS.md` form `resolve` returns — looked up
    across every loaded corpus.

    In-corpus paths take the fast path and never consult `loaded`, so a
    single-gear run or a synthetic test that passes `[]` behaves exactly as a
    plain `corpus.text(rel)` would. `None` means no loaded corpus provides the
    document, which a caller must *report* rather than skip: an unverifiable
    propagation target is a Finding, never a silent skip. Before this existed,
    every cross-gear target was dropped and four of pricing's decisions had their
    only cross-gear claim silently unverified.
    """
    text = corpus.text(rel)
    if text is not None:
        return text
    target = _normalize(str(PurePosixPath(corpus.root()) / rel))
    for other in loaded:
        sub = _strip_prefix(target, _normalize(other.root()))
        if sub is None:
            continue
        text = other.text(sub)
        if text is not None:
            return text
    return None


def gear_name(corpus):
    """The gear name a corpus belongs to: its root's `docs`-parent directory name
    (`.../gears/bss/rating/docs` -> `rating`). `None` for a root shape this
    convention doesn't apply to (a bare, single-component synthetic test root) —
    callers treat that corpus as contributing nothing, never as an error.

    This is the one place gear identity is allowed to flow *out* of
    resolution-adjacent code: the CLI needs it to qualify each corpus's findings
    with the gear they came from before the known-debt decision. It stays data for
    that decision, never a branch in `resolve` or any invariant's matching logic.
    """
    name = PurePosixPath(corpus.root()).parent.name
    return name if name else None


class SeamIndex:
    """Where a `SEAMS.md` seam id (`M12`, `RG3`, `SUB-P7`, …) is actually defined,
    gathered once from every corpus the CLI loaded.

    `resolve` looks a citation's id up here instead of inferring its owning gear
    from the id's prefix. The id *shape* is a documentation convention this tool
    may reasonably know; *which gears exist*, and which of them owns a given id, is
    not — a new sibling gear needs no code change here, only another `--gear`
    corpus.
    """

    __slots__ = ("_owners",)

    def __init__(self, owners=None):
        #: Seam id -> set of gear names whose `SEAMS.md` defines a row for it.
        self._owners = owners if owners is not None else {}

    @classmethod
    def build(cls, corpora):
        """Scans every corpus's top-level `SEAMS.md` for table rows shaped
        `| **<ID>** | … |`. A corpus with no `SEAMS.md` (pricing has none; it only
        *cites* its neighbours' seam maps) simply contributes nothing — not an error.
        """
        owners = {}
        for corpus in corpora:
            text = corpus.text("SEAMS.md")
            if text is None:
                continue
            gear = gear_name(corpus)
            if gear is None:
                continue
            for line in split_lines(text):
                match = _SEAM_ROW.search(line)
                if match is not None:
                    owners.setdefault(match.group(1), set()).add(gear)
        return cls(owners)

    def owners(self, ident):
        """Gear names that define a row for `ident`, sorted; empty if none does."""
        return sorted(self._owners.get(ident, ()))


def _within(span, claimed):
    """True when `span` sits inside one of the already-claimed `(start, end)` spans."""
    start, end = span
    return any(low <= start and end <= high for low, high in claimed)


def _insert_if_present(corpus, rel, paths, unresolved, token):
    if corpus.has(rel):
        paths.add(rel)
    else:
        unresolved.add(token)


def _first_path_with_prefix(corpus, want):
    for path, _text in corpus.files():
        if path.startswith(want):
            return path
    return None


def _corpus_stems(corpus):
    """`(stem, path)` for every **top-level** document of `corpus`, minus the three
    with a dedicated branch — the other half of the answer to the dropped-target
    class, and the reason it is a class rather than three names.

    `PRD` and `DESIGN` were never really shorthands: they are the stems of
    `PRD.md` and `DESIGN.md`, hard-coded because those two happened to be the
    top-level documents when the tool was written. Pricing later grew
    `STRIPE-GAP-ANALYSIS.md` and subscriptions `REVIEW.md`, and every claim into
    them went unchecked — not for being written badly, but for being written about
    a document nobody had added to a list. Deriving the vocabulary from the corpus
    closes that permanently: a gear that adds a top-level document can cite it the
    next day, in either the stem form D-43 uses (`STRIPE-GAP-ANALYSIS G-2 marked
    actioned`) or the path form D-319 uses (`` `STRIPE-GAP-ANALYSIS.md` §4 ``),
    with no code change here.

    **Top-level only, and that is the boundary.** A `design/` slice and an `ADR/`
    are addressed by the shorthands built for them (`S<n>`, `Foundation`,
    `ADR-NNNN`) or by explicit path; minting a bare stem for every nested file
    would put `01-foundation` and `0002-cpt-cf-bss-pricing-adr-…` into the
    vocabulary, which no register writes and which cannot be told from prose.

    One consequence to know about: `Corpus.load` reads **every** `*.md` under the
    docs root, so a stray file dropped at the top level (a session's report, a
    scratch note) both joins the corpus and mints a stem. That is the same hazard
    the corpus has always had — a stray document once moved the live finding count
    7 → 0 — one surface wider. A stray *nested* file is inert here.
    """
    out = []
    for path, _text in corpus.files():
        if "/" in path or not path.endswith(".md"):
            continue
        stem = path[: -len(".md")]
        if stem in _DEDICATED_STEMS:
            continue
        out.append((stem, path))
    return out


def resolve(raw, corpus, seams):
    """Maps the register's propagation shorthand onto corpus-relative paths.

    Cross-gear targets are resolved two ways. `SEAMS <id>` is resolved against
    `seams` — the seam ids every loaded gear corpus actually defines a row for —
    rather than inferred from the id's prefix, and is returned as
    `../../<gear>/docs/SEAMS.md` so a finding names something a reader can open.
    An **explicit path** of that same shape (`../../<gear>/docs/<file>.md`,
    2026-07-31) resolves as written; one naming the citing gear itself folds to
    the in-corpus path, exactly as a same-gear seam citation does. A citation
    from within the defining gear's own corpus resolves to the in-corpus
    `SEAMS.md` instead of escaping and returning via `../../`. An id no loaded
    gear defines, or one two gears both define, is reported on `Resolved` rather
    than silently guessed. A shorthand token (`PRD`, `DESIGN`, …) occurring
    *inside* an explicit cross-gear path — `PRD` inside
    `../../subscriptions/docs/PRD.md` — is part of that path, never a second,
    own-gear claim.

    **Own-gear documents are nameable too** (2026-08-16), by corpus-relative path
    (`_MD_PATH`) or, for a top-level document, by stem (`_corpus_stems`). The
    vocabulary of nameable documents is therefore *the corpus*, not a list in this
    file, which is the honest boundary: P1 can verify a citation in exactly the
    documents it loaded and no others. A path of that shape naming a document the
    corpus does **not** hold is reported `unresolved` — never dropped — so a claim
    into `GONE.md` reads as a finding rather than as a verified claim, and it is
    reported even when the same citation carries five shorthands that do resolve.
    Things outside the corpus stay outside the vocabulary on purpose: D-314 cites
    `sqlite_window_service.rs` and `infra::window`'s module header, which are real
    propagation surfaces but not documents P1 can read, and pretending to
    interpret them would be a worse lie than saying nothing about them.

    **A markdown link is one target** (2026-08-16): when `[label](dest)`'s `dest`
    is a document target, the whole link — label included — is that target. The
    register writes cross-gear references that way, and without this rule
    `[rating PRD](../../rating/docs/PRD.md)` claimed the rating PRD *and* the
    citing gear's own `PRD.md`.
    """
    paths = set()
    unresolved = set()
    seam_undefined = set()
    seam_conflicts = {}
    citing_gear = gear_name(corpus)

    # Spans already claimed by a longer, more specific form. A token inside one of
    # them is part of that target, never a second claim of its own: `PRD` inside
    # `../../subscriptions/docs/PRD.md`, `DESIGN` inside `DESIGN.md`,
    # `STRIPE-GAP-ANALYSIS` inside `STRIPE-GAP-ANALYSIS.md`, and `PRD` inside the
    # *label* of `[rating PRD](../../rating/docs/PRD.md)`.
    claimed = []

    def claim_document_path(text, token):
        """Resolves `text` if it is a document-target path. True when it was one."""
        cross = _CROSS_GEAR.fullmatch(text)
        if cross is not None:
            gear, sub = cross.group(1), cross.group(2)
            if citing_gear == gear:
                _insert_if_present(corpus, sub, paths, unresolved, token)
            else:
                paths.add("../../{}/docs/{}".format(gear, sub))
            return True
        own = _MD_PATH.fullmatch(text)
        if own is not None:
            _insert_if_present(corpus, _normalize(own.group(1)), paths, unresolved, token)
            return True
        return False

    # The link pass runs first, and claims the *whole* `[label](dest)` span — so a
    # shorthand in the label is already inside a claimed span by the time the
    # shorthand pass runs, exactly as one inside a bare path is.
    for match in _MD_LINK.finditer(raw):
        if claim_document_path(match.group(2), match.group(2)):
            claimed.append(match.span())

    for match in _CROSS_GEAR.finditer(raw):
        if _within(match.span(), claimed):
            continue
        claimed.append(match.span())
        gear, sub = match.group(1), match.group(2)
        if citing_gear == gear:
            _insert_if_present(corpus, sub, paths, unresolved, match.group(0))
        else:
            paths.add("../../{}/docs/{}".format(gear, sub))

    for match in _MD_PATH.finditer(raw):
        if _within(match.span(1), claimed):
            continue
        claimed.append(match.span())
        rel = _normalize(match.group(1))
        _insert_if_present(corpus, rel, paths, unresolved, match.group(1))

    for stem, path in _corpus_stems(corpus):
        # Same citation-token boundary as `_TOKEN`: a stem is a stem, not the head
        # of a longer name (`REVIEW` is not `REVIEW-2026.md`, and
        # `STRIPE-GAP-ANALYSIS` is not `STRIPE-GAP-ANALYSIS-V2`).
        pattern = _TOKEN_BEFORE + re.escape(stem) + _TOKEN_AFTER
        for match in re.finditer(pattern, raw):
            if _within(match.span(), claimed):
                continue
            paths.add(path)

    for match in _TOKEN.finditer(raw):
        if _within(match.span(1), claimed):
            continue
        whole = match.group(1)
        if whole == "PRD":
            _insert_if_present(corpus, "PRD.md", paths, unresolved, whole)
        elif whole == "DESIGN":
            _insert_if_present(corpus, "DESIGN.md", paths, unresolved, whole)
        elif whole == "Foundation":
            _insert_if_present(corpus, "design/01-foundation.md", paths, unresolved, whole)
        elif whole == "SEAMS":
            # Anchored at the end of group 1, exactly as Rust anchors at
            # `whole_match.end()`.
            tail = _SEAM_GEAR.search(raw[match.end(1):])
            if tail is None:
                unresolved.add(whole)
            else:
                ident = tail.group(1)
                owners = seams.owners(ident)
                if len(owners) == 0:
                    seam_undefined.add(ident)
                elif len(owners) == 1:
                    gear = owners[0]
                    if citing_gear == gear:
                        paths.add("SEAMS.md")
                    else:
                        paths.add("../../{}/docs/SEAMS.md".format(gear))
                else:
                    seam_conflicts.setdefault(ident, set()).update(owners)
        elif match.group(2) is not None:
            want = "design/{:02d}-".format(int(match.group(2)))
            found = _first_path_with_prefix(corpus, want)
            if found is not None:
                paths.add(found)
            else:
                unresolved.add(whole)
        elif match.group(3) is not None:
            want = "ADR/{}-".format(match.group(3))
            found = _first_path_with_prefix(corpus, want)
            if found is not None:
                paths.add(found)
            else:
                unresolved.add(whole)

    return Resolved(
        paths=sorted(paths),
        unresolved=sorted(unresolved),
        seam_undefined=sorted(seam_undefined),
        seam_conflicts=[(ident, sorted(gears)) for ident, gears in sorted(seam_conflicts.items())],
    )
