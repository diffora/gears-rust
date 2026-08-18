"""One gear's `docs/` tree, read once. Port of `tools/spec-check/src/corpus.rs`."""

import os


class CorpusError(Exception):
    """A docs tree that cannot be loaded. Never swallowed into an empty corpus."""


#: Directory names skipped wherever they appear under a docs root, and the file
#: name prefixes skipped at any depth.
#:
#: `Corpus.load` takes **every** `*.md` under the root, which is what makes a
#: stray document dangerous rather than merely untidy: it joins the corpus, its
#: prose satisfies P1 citation searches and P3 code references, and — since
#: `targets._corpus_stems` — a stray *top-level* file also mints a citation stem
#: into P1's vocabulary. That is not hypothetical: a stray report once moved the
#: live finding count 7 → 0, which is the incident `targets.py` records.
#:
#: The one thing the tool can name with certainty is **its own output**, so that
#: is what this excludes. `judge_report._default_out` writes
#: `docs/spec-check/N1-<gear>.md`, and `.spec-check/` is where a run's
#: `neighbourhoods.json` / `verdicts.json` artifacts land (gitignored). The
#: `judge_report` guard refuses to *write* a report into a gear docs tree; this
#: is the other half — a report that arrived by any other route (a copy, a
#: `--out` from an older revision, a session moving files) is now inert.
#:
#: **This is a denylist and it is honest about being one.** It cannot recognise
#: an arbitrary scratch note, and an allowlist is not available: the design set
#: grows top-level documents (`STRIPE-GAP-ANALYSIS.md`, `REVIEW.md`) that must be
#: readable the day they land, which is the whole reason `_corpus_stems` derives
#: its vocabulary from the corpus. What closes the residual is
#: `excluded_paths()`: whatever this skips is *reported* rather than silently
#: dropped, so an exclusion can never be mistaken for a file that was never there.
_EXCLUDED_DIR_NAMES = frozenset({"spec-check", ".spec-check"})
_EXCLUDED_NAME_PREFIXES = ("N1-",)


def is_excluded(rel):
    """True for a corpus-relative path this tool refuses to read as a document.

    Takes the relative path rather than a name so a directory exclusion applies
    at any depth, and so the decision is reproducible from a finding's own path.
    """
    parts = rel.split("/")
    if any(part in _EXCLUDED_DIR_NAMES for part in parts[:-1]):
        return True
    return parts[-1].startswith(_EXCLUDED_NAME_PREFIXES)


def split_lines(text):
    """Exact equivalent of Rust's `str::lines()`.

    Python's `str.splitlines()` is NOT equivalent: it also splits on \\v, \\f,
    \\x1c-\\x1e, \\u0085, \\u2028 and \\u2029. Every line number this tool reports
    and every decision-body slice boundary is computed from this, so a document
    containing a form feed would silently shift them all.

    Rust splits at "\\n", strips a "\\r" that immediately preceded that "\\n",
    treats the final line ending as optional, and yields nothing for "".
    """
    if not text:
        return []
    had_final_newline = text.endswith("\n")
    body = text[:-1] if had_final_newline else text
    parts = body.split("\n")
    # Every part but the last was terminated by a "\n", so a trailing "\r" there
    # is the "\r\n" ending and Rust drops it. The last part only had one if the
    # text itself ended with a newline.
    out = [p[:-1] if p.endswith("\r") else p for p in parts[:-1]]
    last = parts[-1]
    if had_final_newline and last.endswith("\r"):
        last = last[:-1]
    out.append(last)
    return out


class Corpus:
    """One gear's `docs/` tree. Keys are paths relative to the tree root, always
    with `/` separators so tests and findings read the same on every platform.
    """

    __slots__ = ("_root", "_files", "_excluded")

    def __init__(self, root, files, excluded=None):
        self._root = root
        # Sorted at construction: Rust holds these in a BTreeMap, and consumers
        # that take "the first path with this prefix" depend on the order.
        self._files = dict(sorted(files.items()))
        self._excluded = sorted(excluded or ())

    @classmethod
    def load(cls, root):
        """Reads every `*.md` under `root`.

        Raises — rather than returning an empty corpus — when `root` is not an
        existing directory. A typo in the invocation or a renamed `docs/`
        directory used to yield an empty corpus, so every invariant found nothing
        and the run exited 0 having checked nothing at all: the exact failure mode
        this tool exists to catch, in the tool itself. A gate must never claim
        coverage it does not have, and a silent run must never be
        indistinguishable from a clean one.
        """
        if not os.path.isdir(root):
            raise CorpusError(
                "{} is not an existing directory — a gear docs tree must exist to be "
                "checked (an empty corpus would silently pass every invariant)".format(root)
            )
        def reraise(error):
            # `os.walk` reports errors to this callback and, with the default
            # `onerror=None`, drops them — the walk just yields fewer files. Rust
            # propagates every `WalkDir` error instead of dropping it, for exactly
            # the reason `load` refuses a missing root: a directory it could not
            # read must never be indistinguishable from a directory with nothing
            # in it.
            raise CorpusError(
                "walking the docs tree under {}: {}".format(root, error)
            )

        files = {}
        excluded = []
        for dirpath, _dirnames, filenames in os.walk(root, onerror=reraise):
            for name in filenames:
                if not name.endswith(".md"):
                    continue
                path = os.path.join(dirpath, name)
                rel = os.path.relpath(path, root).replace(os.sep, "/")
                if is_excluded(rel):
                    excluded.append(rel)
                    continue
                with open(path, "r", encoding="utf-8") as handle:
                    files[rel] = handle.read()
        return cls(root, files, excluded)

    @classmethod
    def from_parts(cls, root, parts):
        """Builds an in-memory corpus.

        Applies the same exclusion `load` does, so a test cannot prove a
        behaviour over a document a real run would never have read.
        """
        kept = {}
        excluded = []
        for rel, text in dict(parts).items():
            (excluded.append(rel) if is_excluded(rel) else kept.__setitem__(rel, text))
        return cls(root, kept, excluded)

    def root(self):
        """The root exactly as it was passed in — never normalised.

        `P1/decision-register-unparsed` and `P2/traceability-convention-unknown`
        embed this string in their messages, so the frozen oracles only reproduce
        when the CLI is given the same relative paths the Makefile used.
        """
        return self._root

    def files(self):
        """Every `(path, text)` pair, in sorted path order."""
        return list(self._files.items())

    def excluded_paths(self):
        """The `*.md` paths under this root that `is_excluded` refused, sorted.

        Reported by the CLI rather than kept private. A checker that silently
        narrows its own input is the failure mode this module already refuses in
        two other places (a missing root, an unreadable directory); an exclusion
        list is the third, and the only difference is that this one is deliberate.
        """
        return list(self._excluded)

    def text(self, rel):
        return self._files.get(rel)

    def has(self, rel):
        return rel in self._files
