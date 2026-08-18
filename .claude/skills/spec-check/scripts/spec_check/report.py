"""Turns raw findings into what the CLI actually shows.

The pinned baselines (`invariants.propagation.PINNED_PROPAGATION_GAPS_2026_07_29`,
`invariants.closure.PINNED_UNREFERENCED_CODES_2026_07_29`) are accepted debt, not
new drift — tracked as **D-69** — so a run that reproduces exactly that debt and
nothing else should pass, not fail forever on the same known gaps.

Port of `tools/spec-check/src/report.rs`.
"""

from .invariants import closure, propagation

#: Docs-debt ticket the pinned baselines are owed against. Named once here so the
#: summary and the `--show-known-debt` output don't each carry their own copy.
KNOWN_DEBT_TICKET = "D-69"


def is_known_debt(finding, gear):
    """True if `finding`, attributed to `gear`, is exactly one of the pinned,
    accepted-debt findings rather than newly appeared drift.

    Each invariant module owns its own baseline and its own message-shape parsing;
    this just combines them, since a `Finding` only ever carries one invariant tag
    and so at most one of the two can ever match.

    `gear` is required, not read off `finding` (a `Finding` has no gear field):
    both baselines are snapshots of the *pricing* corpus specifically, and their
    keys are not unique across gears. Callers must supply the gear the finding
    actually came from, or a same-keyed finding from a different gear would be
    silently suppressed as pricing's pinned debt.
    """
    return propagation.is_pinned_baseline(finding, gear) or closure.is_pinned_baseline(
        finding, gear
    )


def unreproduced_pins(findings, gear):
    """The pinned entries for `gear` that this run did **not** reproduce.

    `is_known_debt` only ever subtracts: a pin whose document has since been
    fixed goes on suppressing nothing forever, and the run says the same
    `N known-debt finding(s) suppressed` it always did. The two set-equality
    tests (`test_propagation.py`, `test_closure.py`) are the only thing that has
    ever caught a dead pin — and they are pytest, which no CI job runs, so the
    signal exists exactly where it is not being read.

    Returned as `(invariant, first, second)` triples in the baselines' own key
    shape so the caller can print them without knowing either module's internals.
    """
    seen_propagation = set()
    seen_codes = set()
    for finding in findings:
        pair = propagation.missing_pair(finding)
        if pair is not None:
            seen_propagation.add(pair)
        pair = closure.unreferenced_pair(finding)
        if pair is not None:
            seen_codes.add(pair)

    out = []
    for pinned_gear, ident, path in propagation.PINNED_PROPAGATION_GAPS_2026_07_29:
        if pinned_gear == gear and (ident, path) not in seen_propagation:
            out.append(("P1/propagation-missing", ident, path))
    for pinned_gear, code, path in closure.PINNED_UNREFERENCED_CODES_2026_07_29:
        if pinned_gear == gear and (code, path) not in seen_codes:
            out.append(("P3/code-unreferenced", code, path))
    return out


def render_unreproduced_pins(rows):
    """The disclosure line for `unreproduced_pins`, or `""` when every pin fired.

    Deliberately not a `Finding`: a dead pin is a fact about this tool's own
    baseline, not about the documents, and routing it through the finding stream
    would make it eligible for the exit code and for suppression by the very
    mechanism it is reporting on.
    """
    if not rows:
        return ""
    out = [
        "\n{} pinned finding(s) did not reproduce — the documents were fixed and the "
        "pin is now dead; remove it deliberately rather than leaving it as a floor:"
        .format(len(rows))
    ]
    for invariant, first, second in rows:
        out.append("\n  {} — {} / {}".format(invariant, first, second))
    return "".join(out)


def partition_known_debt(findings, gear):
    """Splits `findings` into `(live, known_debt)`.

    `live` is what the exit-code decision and the default display are based on.
    Order within each group is preserved from the input. All of `findings` must
    come from the same gear — callers with more than one loaded corpus call this
    once per corpus and accumulate, rather than flattening across corpora before
    this decision is made.
    """
    live = []
    known_debt = []
    for finding in findings:
        (known_debt if is_known_debt(finding, gear) else live).append(finding)
    return live, known_debt


def render_text(live, known_debt, show_known_debt):
    """The whole text report, ready to print — no trailing newline, the caller's
    `print` supplies it.

    The suppression *policy* is this module's responsibility and so is disclosing
    it: the line `75 known-debt finding(s) suppressed, tracked as D-69` is the
    only thing standing between 75 suppressed findings and 75 invisible ones.
    """
    out = []
    for f in live:
        out.append(f.render())
        out.append("\n")
    if show_known_debt and known_debt:
        out.append(
            "\nKnown debt — accepted, tracked as {}, not new drift "
            "({} finding(s)):\n".format(KNOWN_DEBT_TICKET, len(known_debt))
        )
        for f in known_debt:
            out.append(f.render())
            out.append("\n")
    out.append("\n{} finding(s)".format(len(live)))
    if known_debt:
        out.append("\n")
        if show_known_debt:
            out.append(
                "{} known-debt finding(s) shown above, tracked as {} "
                "(accepted, not new drift)".format(len(known_debt), KNOWN_DEBT_TICKET)
            )
        else:
            out.append(
                "{} known-debt finding(s) suppressed, tracked as {} "
                "— pass --show-known-debt to see them".format(
                    len(known_debt), KNOWN_DEBT_TICKET
                )
            )
    return "".join(out)


def json_report(live, known_debt, show_known_debt, dead_pins=None):
    """The `--format json` envelope, as a dict in serde's struct-field order.

    The suppressed count and the ticket it is tracked against are part of the
    reporting contract a consumer parses, so they are built here where a test can
    see them. `known_debt` is present only under `--show-known-debt`, so the
    default envelope stays the live set plus an honest count of what was withheld
    — absent, never null.

    `pins_not_reproduced` is always present, empty list included: a consumer that
    had to distinguish "no dead pins" from "this build does not report them" is a
    consumer that will read the absence as the former.
    """
    out = {
        "findings": [f.to_json() for f in live],
        # Zero when the debt is shown: nothing was withheld from this envelope
        # (PR-review fix, 2026-07-31 — the count previously claimed suppression
        # even when `known_debt` carried every row).
        "known_debt_suppressed": 0 if show_known_debt else len(known_debt),
        "known_debt_tracked_as": KNOWN_DEBT_TICKET,
        "pins_not_reproduced": [
            {"invariant": invariant, "id": first, "path": second}
            for invariant, first, second in (dead_pins or ())
        ],
    }
    if show_known_debt:
        out["known_debt"] = [f.to_json() for f in known_debt]
    return out
