"""Maps the register's propagation shorthand onto corpus-relative paths.

Port of `tools/spec-check/src/targets.rs`.
"""

import re
from pathlib import PurePosixPath

from .corpus import split_lines

_TOKEN = re.compile(r"\b(S(\d{1,2})|Foundation|PRD|DESIGN|SEAMS|ADR-(\d{4}))\b")

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


def resolve(raw, corpus, seams):
    """Maps the register's propagation shorthand onto corpus-relative paths.

    Cross-gear targets (`SEAMS <id>`) are resolved against `seams` — the seam ids
    every loaded gear corpus actually defines a row for — rather than inferred from
    the id's prefix, and are returned as `../../<gear>/docs/SEAMS.md` so a finding
    names something a reader can open. A citation from within the defining gear's
    own corpus resolves to the in-corpus `SEAMS.md` instead of escaping and
    returning via `../../`. An id no loaded gear defines, or one two gears both
    define, is reported on `Resolved` rather than silently guessed.
    """
    paths = set()
    unresolved = set()
    seam_undefined = set()
    seam_conflicts = {}
    citing_gear = gear_name(corpus)

    for match in _TOKEN.finditer(raw):
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
