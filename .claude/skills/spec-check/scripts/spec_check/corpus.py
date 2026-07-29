"""One gear's `docs/` tree, read once. Port of `tools/spec-check/src/corpus.rs`."""

import os


class CorpusError(Exception):
    """A docs tree that cannot be loaded. Never swallowed into an empty corpus."""


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

    __slots__ = ("_root", "_files")

    def __init__(self, root, files):
        self._root = root
        # Sorted at construction: Rust holds these in a BTreeMap, and consumers
        # that take "the first path with this prefix" depend on the order.
        self._files = dict(sorted(files.items()))

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
        for dirpath, _dirnames, filenames in os.walk(root, onerror=reraise):
            for name in filenames:
                if not name.endswith(".md"):
                    continue
                path = os.path.join(dirpath, name)
                rel = os.path.relpath(path, root).replace(os.sep, "/")
                with open(path, "r", encoding="utf-8") as handle:
                    files[rel] = handle.read()
        return cls(root, files)

    @classmethod
    def from_parts(cls, root, parts):
        """Builds an in-memory corpus. Test-only in practice."""
        return cls(root, dict(parts))

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

    def text(self, rel):
        return self._files.get(rel)

    def has(self, rel):
        return rel in self._files
