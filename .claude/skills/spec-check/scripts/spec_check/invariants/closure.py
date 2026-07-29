"""P3 — declared-and-referenced closure for instruction ids and error codes.

Port of `tools/spec-check/src/invariants/closure.rs`.
"""


def is_design_slice(path):
    """True for corpus-relative paths that are numbered design slices —
    `design/01-foundation.md`, `design/02-plan-definition.md`, and so on — the
    only documents expected to own an error catalogue or a traceability claim.

    Excludes `design/README.md` (an index, not a slice: no numeric prefix) and
    everything outside `design/` (PRD, DESIGN, DECISIONS, ADRs), which
    legitimately *reference* codes and ids a slice owns without ever declaring
    any. Path shape, not "does it mention one", is the discriminator: what makes
    a document own a catalogue is that it is a slice.

    Shared with `fr_coverage.py`, which scopes its traceability-convention
    detection to the same set for the same reason — a `**Traces to**:` convention
    lives on slices, and a non-slice document merely mentioning the shape in prose
    must not count as the gear "using" it.
    """
    if not path.startswith("design/"):
        return False
    rest = path[len("design/"):]
    # `c.is_ascii_digit()`, not Python's Unicode-aware `str.isdigit()`.
    return rest[:1] in "0123456789" and rest[:1] != ""
