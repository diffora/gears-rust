"""`Finding` and `Severity` — the one output shape text and JSON both render.

Port of `tools/spec-check/src/finding.rs`.
"""

from typing import Optional


class Severity:
    """Severity as a plain string, so both renderings stay derivable from one value.

    The text report prints Rust's `{:?}` (Debug) form — `High` / `Medium` / `Low`
    — while the JSON envelope prints serde's `rename_all = "lowercase"` form. A
    single canonical value with two accessors is what keeps them from drifting.
    """

    HIGH = "High"
    MEDIUM = "Medium"
    LOW = "Low"

    # Gate ordering, from clap's `Gate` enum in `main.rs`: derived `Ord` follows
    # declaration order, so Low < Medium < High.
    _RANK = {"Low": 0, "Medium": 1, "High": 2}

    @classmethod
    def rank(cls, severity):
        return cls._RANK[severity]

    @staticmethod
    def json(severity):
        return severity.lower()


class Finding:
    """One reported defect. `line` is `None` for findings about a whole document."""

    __slots__ = ("invariant", "severity", "file", "line", "message")

    def __init__(self, invariant, severity, file, line, message):
        #: Stable id, e.g. `P1/propagation-missing`. Grep-able and safe to pin in tests.
        self.invariant = invariant
        self.severity = severity
        #: Corpus-relative path of the document that must change.
        self.file = file
        self.line = line  # Optional[int]
        self.message = message

    def render(self):
        loc = "{}:{}".format(self.file, self.line) if self.line is not None else self.file
        return "[{}] {} — {} ({})".format(self.severity, loc, self.message, self.invariant)

    def to_json(self):
        # Insertion order is serde's struct-field order, and `json.dumps` preserves
        # it — that is what makes the frozen JSON oracle reproduce.
        return {
            "invariant": self.invariant,
            "severity": Severity.json(self.severity),
            "file": self.file,
            "line": self.line,
            "message": self.message,
        }

    def __eq__(self, other):
        if not isinstance(other, Finding):
            return NotImplemented
        return (
            self.invariant == other.invariant
            and self.severity == other.severity
            and self.file == other.file
            and self.line == other.line
            and self.message == other.message
        )

    def __hash__(self):
        return hash((self.invariant, self.severity, self.file, self.line, self.message))

    def __repr__(self):
        return "Finding({!r}, {!r}, {!r}, {!r}, {!r})".format(
            self.invariant, self.severity, self.file, self.line, self.message
        )
