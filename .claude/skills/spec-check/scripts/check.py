#!/usr/bin/env python3
"""spec-check CLI — cross-document invariants over BSS gear specs.

Port of `tools/spec-check/src/main.rs`. Run from the repository root:

    python3 .claude/skills/spec-check/scripts/check.py \\
      --gear gears/bss/pricing/docs \\
      --gear gears/bss/rating/docs \\
      --gear gears/bss/subscriptions/docs
"""

import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from spec_check import report  # noqa: E402
from spec_check.corpus import Corpus, CorpusError  # noqa: E402
from spec_check.finding import Severity  # noqa: E402
from spec_check.invariants import closure, fr_coverage, propagation  # noqa: E402
from spec_check.targets import SeamIndex, gear_name  # noqa: E402

#: Gate ordering, mirroring clap's `Gate` enum in `main.rs`: derived `Ord` follows
#: declaration order, so Low < Medium < High.
_GATE_RANK = {"low": 0, "medium": 1, "high": 2}


def is_failing(findings, gear, max_severity):
    """The run fails when at least one finding that is *not* pinned, accepted debt
    is at or above `max_severity`.

    A pinned baseline finding never fails the run on its own, no matter its
    severity or how high the gate is set — that is the entire point of pinning it:
    known, tracked debt should not block every run forever.

    `findings` must all be from the same corpus, named by `gear`: both baselines
    are gear-qualified, so this is called once per loaded corpus rather than once
    over every corpus's findings flattened together.
    """
    threshold = _GATE_RANK[max_severity]
    for finding in findings:
        if report.is_known_debt(finding, gear):
            continue
        if Severity.rank(finding.severity) >= threshold:
            return True
    return False


def _parse_args(argv):
    parser = argparse.ArgumentParser(
        prog="spec-check", description="Cross-document invariants over BSS gear specs"
    )
    # `extend` + `nargs="+"`, not `append`: clap declares this flag as
    # `num_args = 1..`, which takes one *or more* values per occurrence, so
    # `--gear a b c` is as valid as `--gear a --gear b --gear c`. `append` would
    # accept only the second form and reject an invocation the Rust binary took.
    parser.add_argument(
        "--gear", dest="gears", action="extend", nargs="+", required=True,
        help="One or more gear `docs/` directories.",
    )
    parser.add_argument("--format", choices=["text", "json"], default="text")
    parser.add_argument(
        "--max-severity", choices=["low", "medium", "high"], default="medium",
        help=(
            "Lowest severity that fails the run. Applies only to findings that are "
            "not pinned, accepted debt — a pinned finding never fails the run, "
            "regardless of its severity."
        ),
    )
    parser.add_argument(
        "--show-known-debt", action="store_true",
        help=(
            "Also print the pinned, accepted-debt findings (tracked as D-69) that "
            "the default output only summarizes the count of. Never changes the "
            "exit code."
        ),
    )
    return parser.parse_args(argv)


def main(argv=None):
    args = _parse_args(sys.argv[1:] if argv is None else argv)

    # Load every `--gear` corpus up front, before checking any of them: P1's seam
    # ownership check and P3's cross-gear instruction-id closure both need to know
    # what every loaded gear declares, not just the one currently being checked.
    corpora = [Corpus.load(gear) for gear in args.gears]
    seams = SeamIndex.build(corpora)
    declared = closure.DeclaredInstructions.build(corpora)

    # Findings are partitioned into known-debt vs. live *per corpus*, before being
    # accumulated across the whole run — not by flattening every corpus's findings
    # first and partitioning once. Both baselines are gear-qualified: the
    # known-debt decision needs the gear each finding actually came from, and that
    # context only still exists here, one corpus at a time.
    live = []
    known_debt = []
    failing = False
    for corpus in corpora:
        corpus_findings = []
        corpus_findings.extend(propagation.check(corpus, seams, corpora))
        corpus_findings.extend(fr_coverage.check(corpus))
        corpus_findings.extend(closure.check(corpus, declared))

        gear = gear_name(corpus) or ""
        failing = failing or is_failing(corpus_findings, gear, args.max_severity)
        corpus_live, corpus_known_debt = report.partition_known_debt(corpus_findings, gear)
        live.extend(corpus_live)
        known_debt.extend(corpus_known_debt)

    if args.format == "json":
        payload = report.json_report(live, known_debt, args.show_known_debt)
        # `ensure_ascii=False`: serde_json does not escape non-ASCII, and findings
        # carry §, × and ….
        print(json.dumps(payload, indent=2, ensure_ascii=False))
    else:
        print(report.render_text(live, known_debt, args.show_known_debt))

    return 1 if failing else 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except CorpusError as exc:
        # `Error: ` and exit 1, both copied from what the Rust binary actually did:
        # `main() -> Result<ExitCode>` hands an `Err` to std's `Termination for
        # Result`, which prints `Error: {err:?}` and returns `ExitCode::FAILURE`.
        # Exit 2 is the argument parser's (clap's, and argparse's) usage-error code,
        # so a docs tree that will not load must not report the same code as a
        # misspelled flag.
        print("Error: {}".format(exc), file=sys.stderr)
        sys.exit(1)
