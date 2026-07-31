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


def json_report(live, known_debt, show_known_debt):
    """The `--format json` envelope, as a dict in serde's struct-field order.

    The suppressed count and the ticket it is tracked against are part of the
    reporting contract a consumer parses, so they are built here where a test can
    see them. `known_debt` is present only under `--show-known-debt`, so the
    default envelope stays the live set plus an honest count of what was withheld
    — absent, never null.
    """
    out = {
        "findings": [f.to_json() for f in live],
        # Zero when the debt is shown: nothing was withheld from this envelope
        # (PR-review fix, 2026-07-31 — the count previously claimed suppression
        # even when `known_debt` carried every row).
        "known_debt_suppressed": 0 if show_known_debt else len(known_debt),
        "known_debt_tracked_as": KNOWN_DEBT_TICKET,
    }
    if show_known_debt:
        out["known_debt"] = [f.to_json() for f in known_debt]
    return out
