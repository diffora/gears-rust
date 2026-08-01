"""P3 — declared-and-referenced closure for instruction ids and error codes.

Port of `tools/spec-check/src/invariants/closure.rs`.
"""

import re

from ..corpus import split_lines
from ..finding import Finding, Severity

_DECLARED_INST = re.compile(r"- `(inst-[a-z0-9-]+)`\s*\Z")
_ANY_INST = re.compile(r"`(inst-[a-z0-9-]+)`")
_CODE = re.compile(r"`([A-Z][A-Z0-9_]{4,})`")

_PROBLEM_RESPONSES = "**Problem responses (RFC 9457):**"

_UNREFERENCED_SHAPE = re.compile(
    r"\A`([A-Z][A-Z0-9_]+)` is declared in a Problem-responses block "
    r"but referenced by no rule\Z"
)


class DeclaredInstructions:
    """The union of every `inst-*` id declared (via a `` - `inst-id` `` bullet
    line) across every corpus the CLI loaded.

    `check`'s dangling-instruction rule verifies references against this union
    rather than the referencing corpus's own declarations alone: an id pricing
    declares and rating cites without a local re-declaration is a legitimate
    cross-gear reference, not a dangling one — P3 must not conflate "declared in a
    sibling gear" with "declared nowhere".
    """

    __slots__ = ("_ids",)

    def __init__(self, ids=None):
        self._ids = ids if ids is not None else set()

    @classmethod
    def build(cls, corpora):
        ids = set()
        for corpus in corpora:
            for _path, text in corpus.files():
                for line in split_lines(text):
                    match = _DECLARED_INST.search(line.rstrip())
                    if match is not None:
                        ids.add(match.group(1))
        return cls(ids)

    def contains(self, ident):
        return ident in self._ids


def declared_codes_union(corpora):
    """Every error code declared inside a Problem-responses block, across every
    corpus the CLI loaded — the code analogue of `DeclaredInstructions`, and for
    the same reason: a rating slice that references a pricing-declared code
    without a local block is a legitimate cross-gear reference, not a slice
    declaring codes in prose (PR-review fix, 2026-07-31)."""
    codes = set()
    for corpus in corpora:
        for _path, text in corpus.files():
            in_block = False
            for line in split_lines(text):
                if _PROBLEM_RESPONSES in line:
                    in_block = True
                elif in_block and line.strip() == "":
                    in_block = False
                if in_block:
                    for match in _CODE.finditer(line):
                        codes.add(match.group(1))
    return codes


def check(corpus, declared, codes_declared=None):
    """P3 over one corpus.

    `declared` is the cross-corpus instruction-id union built once from every
    loaded gear; error-code closure stays scoped to `corpus` alone — unlike
    instruction ids, error codes are not cited across gears by convention.
    """
    findings = []

    referenced = {}
    for path, text in corpus.files():
        for line in split_lines(text):
            for match in _ANY_INST.finditer(line):
                referenced.setdefault(match.group(1), path)

    for ident in sorted(referenced):
        if not declared.contains(ident):
            findings.append(Finding(
                "P3/inst-dangling",
                Severity.MEDIUM,
                referenced[ident],
                None,
                "`{}` is referenced but declared by no instruction line".format(ident),
            ))

    findings.extend(_check_error_codes(corpus, codes_declared))
    return findings


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


def _check_error_codes(corpus, codes_declared=None):
    """Error codes declared inside a `**Problem responses (RFC 9457):**` block
    (which runs until the first blank line) and never mentioned again anywhere
    else in the corpus, plus design slices that declare codes without ever opening
    such a block at all.

    The latter mirrors P2's `directly addresses` handling:
    design/01-foundation.md names its Foundation-owned codes in prose rather than
    a Problem-responses block, so without this second check those codes — and any
    future slice doing the same — would be invisible to the "declared" side of the
    closure rule, silently narrowing what P3 covers rather than surfacing the gap.
    One finding per document, not per code, since the defect is "this document
    uses a different convention", not "this code is unreachable". Scoped to design
    slices only: non-slice documents legitimately reference codes they don't own.
    """
    declared = {}
    referenced = set()
    findings = []
    # Blockless design slices are judged only after the whole corpus'
    # declarations are known (PR-review fix, 2026-07-31; inherited verbatim from
    # the Rust port source): the old per-file `saw_code` fired on *any* code, so
    # a slice that merely referenced sibling-owned codes without opening a block
    # of its own was misread as declaring them in prose — measured live, rating
    # `design/04-overlays-precedence.md` drew exactly that false positive. The
    # finding now fires only when a blockless slice carries at least one code no
    # loaded document declares — i.e. the slice really is the closest thing the
    # corpus has to that code's declarer, which is the convention divergence the
    # check exists for.
    blockless = []

    for path, text in corpus.files():
        in_block = False
        saw_block = False
        codes_outside = set()
        for line in split_lines(text):
            if _PROBLEM_RESPONSES in line:
                in_block = True
                saw_block = True
            elif in_block and line.strip() == "":
                in_block = False
            for match in _CODE.finditer(line):
                if in_block:
                    declared.setdefault(match.group(1), path)
                else:
                    referenced.add(match.group(1))
                    codes_outside.add(match.group(1))
        if codes_outside and not saw_block and is_design_slice(path):
            blockless.append((path, codes_outside))

    known_declarations = codes_declared if codes_declared is not None else set(declared)
    for path, codes_outside in blockless:
        if any(code not in declared and code not in known_declarations
               for code in codes_outside):
            findings.append(Finding(
                "P3/code-convention-divergent",
                Severity.LOW,
                path,
                None,
                "{} declares error codes without a `**Problem responses (RFC 9457):**` "
                "block; the rest of the design set uses that convention".format(path),
            ))

    for code in sorted(declared):
        if code not in referenced:
            findings.append(Finding(
                "P3/code-unreferenced",
                Severity.LOW,
                declared[code],
                None,
                "`{}` is declared in a Problem-responses block but referenced by "
                "no rule".format(code),
            ))

    return findings


#: Pinned baseline of `P3/code-unreferenced` findings against the live pricing
#: corpus, hand-derived on 2026-07-29 from the failure output of the drift test —
#: not by running the checker and trusting whatever it produces. These 51
#: `(gear, code, declaring file)` triples are **debt, not correctness**: confirmed
#: real — each rule describes its failure mode in prose ("any overlap fails")
#: without naming the specific code its slice's catalogue defines for that
#: failure. Fixing them is a separate docs round, owed alongside **D-69**.
#:
#: Pinned as an exact set so a *new* unreferenced code fails immediately, and so a
#: *fixed* one fails too — the list must be updated deliberately when the docs
#: improve, never left to quietly become a floor.
#:
#: Every entry names "pricing": an error-code token plus a corpus-relative
#: filename is not a unique key across gears — `design/03-...`-shaped filenames in
#: particular are just as likely in a sibling gear's own design set. The gear name
#: here is baseline *data*; it must never leak into resolution or matching logic.
#:
#: Deliberate removals, hand-checked per the rule above:
#:
#: - `PACKAGE_FIELDS_INVALID` / design/03 (removed 2026-07-31): D-70's 2026-07-30
#:   propagation wrote the code into PRD AC #89 ("publish MUST fail with a
#:   field-level error (`PACKAGE_FIELDS_INVALID`; …)"), and this check counts a
#:   corpus-wide mention outside a Problem-responses block as a reference. The
#:   code is now referenced by the AC that tests it — a genuine fix, not a
#:   technicality.
#:
#: - `METER_AMBIGUOUS` / design/02 (removed 2026-07-31, the c-wave pin sweep):
#:   D-103's restated `inst-cmp-injective` (2026-07-31b fix round) names the code
#:   in the rule body ("a **duplicate line** within one slice is the ambiguity
#:   that fails publish (`METER_AMBIGUOUS`)") — a rule reference, not a
#:   Problem-responses row. Hand-checked at HEAD before removal.
#:
#: - `TAXONOMY_VALUE_IN_USE` / design/04 (removed 2026-07-31, the c-wave pin
#:   sweep): D-120's `inst-plv-scope` (S9) now references it from the rule body
#:   ("a referenced value cannot retire, `TAXONOMY_VALUE_IN_USE`"). A genuine
#:   fix by the wave that also gave the retire guard its missing overlay scopes.
#:
#: - `REGION_SCOPE_DENIED` / design/05 (removed 2026-08-01, the d-wave
#:   billing-domain review): the new `inst-rb-preview-scope` (N-1 — the preview
#:   grant's explicit pricing-region set) names the code in the rule body
#:   ("`REGION_SCOPE_DENIED` (403) otherwise"), so the 403 finally has the rule
#:   that fires it. Hand-checked at the working tree before removal.
PINNED_UNREFERENCED_CODES_2026_07_29 = (
    ("pricing", "ADDON_CYCLE", "design/02-plan-definition.md"),
    ("pricing", "ADDON_INCOMPATIBLE", "design/02-plan-definition.md"),
    ("pricing", "ADDON_OVERRIDE_UNRESOLVED", "design/02-plan-definition.md"),
    ("pricing", "APPROVAL_ROLE_REQUIRED", "design/05-governance.md"),
    ("pricing", "AVAILABILITY_OUTSIDE_COVERAGE", "design/07-pricewindow-linkage.md"),
    ("pricing", "BACKDATE_GRANT_REQUIRED", "design/05-governance.md"),
    ("pricing", "BASIS_MISSING", "design/08-bundles.md"),
    ("pricing", "BILLING_TIMING_MISSING", "design/06-consumer-contracts.md"),
    ("pricing", "BRAND_UNKNOWN", "design/04-currency-tax.md"),
    ("pricing", "BULK_ROW_CONFLICT", "design/12-operator-efficiency.md"),
    ("pricing", "CHANGE_TARGET_UNPUBLISHED", "design/06-consumer-contracts.md"),
    ("pricing", "CLONE_SOURCE_NOT_FOUND", "design/12-operator-efficiency.md"),
    ("pricing", "COMPONENT_UNPUBLISHED", "design/08-bundles.md"),
    ("pricing", "COMPOSITE_CONSTITUENT_UNPUBLISHED", "design/10-advanced-primitives.md"),
    ("pricing", "COMPOSITE_SELF_REFERENCE", "design/10-advanced-primitives.md"),
    ("pricing", "COMPOSITE_TOO_FEW_CONSTITUENTS", "design/10-advanced-primitives.md"),
    ("pricing", "CREDIT_UNIT_UNPUBLISHED", "design/10-advanced-primitives.md"),
    ("pricing", "DESCRIPTOR_INCOMPLETE", "design/02-plan-definition.md"),
    ("pricing", "EVAL_POLICY_MISPLACED", "design/03-price-structure.md"),
    ("pricing", "FLOOR_FALLBACK_MISSING", "design/10-advanced-primitives.md"),
    ("pricing", "FLOOR_INSIDE_PRICED_BAND", "design/10-advanced-primitives.md"),
    ("pricing", "FLOOR_TYPE_MISSING", "design/10-advanced-primitives.md"),
    ("pricing", "GRANDFATHERED_ROW_IMMUTABLE", "design/07-pricewindow-linkage.md"),
    ("pricing", "GRANDFATHER_LOOSEN_FORBIDDEN", "design/07-pricewindow-linkage.md"),
    ("pricing", "GRANT_APPLICABILITY_INELIGIBLE", "design/10-advanced-primitives.md"),
    ("pricing", "GRANT_APPLICABILITY_UNIT_MISMATCH", "design/10-advanced-primitives.md"),
    ("pricing", "GRANT_APPLICABILITY_UNPUBLISHED", "design/10-advanced-primitives.md"),
    ("pricing", "GRANT_EXPIRY_MISSING", "design/10-advanced-primitives.md"),
    ("pricing", "GRANT_PRICE_UNSCOPED", "design/10-advanced-primitives.md"),
    ("pricing", "GRANT_REF_UNDEFINED", "design/06-consumer-contracts.md"),
    ("pricing", "GROUP_UNKNOWN", "design/09-price-overlays.md"),
    ("pricing", "HYBRID_INCOMPLETE", "design/02-plan-definition.md"),
    ("pricing", "PHASE_DURATION_INVALID", "design/02-plan-definition.md"),
    ("pricing", "PHASE_GRAPH_INVALID", "design/02-plan-definition.md"),
    ("pricing", "PLANTIER_DIVERGENT", "design/02-plan-definition.md"),
    ("pricing", "PLANTIER_MISSING", "design/02-plan-definition.md"),
    ("pricing", "PRORATION_INPUTS_MISSING", "design/06-consumer-contracts.md"),
    ("pricing", "PURCHASE_QTY_RANGE_INVALID", "design/02-plan-definition.md"),
    ("pricing", "QUANTITY_SOURCE_MISSING", "design/03-price-structure.md"),
    ("pricing", "REASON_REQUIRED", "design/05-governance.md"),
    ("pricing", "RESERVATION_ON_NON_USAGE", "design/10-advanced-primitives.md"),
    ("pricing", "RUN_SELECTOR_EMPTY", "design/12-operator-efficiency.md"),
    ("pricing", "SETUP_ROW_INVALID", "design/02-plan-definition.md"),
    ("pricing", "TAX_BASIS_INCOMPLETE", "design/04-currency-tax.md"),
    ("pricing", "TIER_BANDS_GAP", "design/03-price-structure.md"),
    ("pricing", "TIER_BANDS_OVERLAP", "design/03-price-structure.md"),
    ("pricing", "WINDOW_GAP", "design/07-pricewindow-linkage.md"),
)

_PINNED_CODE_SET = frozenset(PINNED_UNREFERENCED_CODES_2026_07_29)


def unreferenced_pair(finding):
    """Parses a `P3/code-unreferenced` finding's `(code, declaring file)` from
    `_check_error_codes`'s own fixed message template. `None` for any other
    invariant tag or a message that doesn't match.

    Deliberately does not, and cannot, recover a gear from the `Finding` alone.
    """
    if finding.invariant != "P3/code-unreferenced":
        return None
    match = _UNREFERENCED_SHAPE.search(finding.message)
    if match is None:
        return None
    return (match.group(1), finding.file)


def is_pinned_baseline(finding, gear):
    """True if `finding`, attributed to `gear`, is exactly one of the pinned,
    accepted-debt unreferenced-code findings rather than newly appeared drift.
    """
    pair = unreferenced_pair(finding)
    if pair is None:
        return False
    return (gear, pair[0], pair[1]) in _PINNED_CODE_SET
