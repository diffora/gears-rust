#!/usr/bin/env python3
"""Build N1 neighbourhoods for one or more gear docs trees.

Step 1 of three. Run from the repository root:

    python3 .claude/skills/spec-check/scripts/neighbourhoods.py \\
      --gear gears/bss/ledger/docs \\
      --out .spec-check/neighbourhoods-ledger.json

Writes the neighbourhood file and prints a triage-class histogram. Nothing gates:
this exits 0 whatever the histogram says. It exits 1 only when a docs tree will
not load, and 2 on a usage error.
"""

import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from spec_check import regions, requirements  # noqa: E402
from spec_check.corpus import Corpus, CorpusError  # noqa: E402
from spec_check.semantic import neighbourhood as nb  # noqa: E402
from spec_check.semantic import triage  # noqa: E402

DEFAULT_OUT = os.path.join(".spec-check", "neighbourhoods.json")

#: Echoed into the envelope so the report can state what the run was configured
#: with. Starting values, tuned from the judge's per-region `usefulness`.
THRESHOLDS = {
    "window_lines": regions.WINDOW_LINES,
    "window_step": regions.WINDOW_STEP,
    "score_threshold": regions.SCORE_THRESHOLD,
    "strong_score": regions.STRONG_SCORE,
    "document_frequency_cutoff": regions.DF_CUTOFF,
    "max_anchors": regions.MAX_ANCHORS,
    "max_overlap_regions": regions.MAX_OVERLAP_REGIONS,
    "max_fragments": regions.MAX_FRAGMENTS,
}


def build_all(gears, also_judge=None):
    """Every requirement of every gear, as a neighbourhood, in a stable order."""
    out = []
    for gear in gears:
        corpus = Corpus.load(gear)
        index = regions.WindowIndex.build(corpus)
        for requirement in requirements.parse(corpus):
            picked = regions.select(index, requirement)
            out.append(nb.build(requirement, picked,
                                triage.classify(requirement, picked), also_judge))
    return out


def histogram(neighbourhoods):
    """Counts per class, every class present even at zero.

    A class that silently stops occurring must not be indistinguishable from a
    class that never existed.
    """
    counts = {name: 0 for name in triage.CLASSES}
    for item in neighbourhoods:
        counts[item["triage"]] += 1
    return counts


def render_histogram(counts, also_judge=None):
    """The histogram, and the dispatch count a reader budgets from.

    `also_judge` must reach both the per-class label and the total: a run that
    promotes a class and still prints it as `not judged` understates its own bill.
    """
    judged_classes = frozenset(triage.JUDGED) | frozenset(also_judge or ())
    width = max(len(name) for name in counts)
    lines = []
    for name in triage.CLASSES:
        judged = "judged" if name in judged_classes else "not judged"
        lines.append("{:<{w}}  {:>4}  ({})".format(name, counts[name], judged, w=width))
    lines.append("{:<{w}}  {:>4}".format("total", sum(counts.values()), w=width))
    lines.append("{:<{w}}  {:>4}".format(
        "judge calls", sum(counts[n] for n in triage.CLASSES if n in judged_classes), w=width
    ))
    return "\n".join(lines)


def _parse_args(argv):
    parser = argparse.ArgumentParser(
        prog="neighbourhoods", description="Build N1 neighbourhoods for a gear docs tree"
    )
    parser.add_argument(
        "--gear", dest="gears", action="extend", nargs="+", required=True,
        help="One or more gear `docs/` directories, repo-relative from the repository root.",
    )
    parser.add_argument("--out", default=DEFAULT_OUT)
    parser.add_argument(
        "--only-class", dest="only_classes", action="append", choices=list(triage.CLASSES),
        default=None, help="Keep only requirements in this triage class. Repeatable.",
    )
    parser.add_argument(
        "--only-id", dest="only_ids", action="append", default=None,
        help="Keep only this requirement id. Repeatable — this is how an evaluation "
             "run addresses a hand-picked sample.",
    )
    parser.add_argument(
        "--judge-anchored", dest="judge_anchored", action="store_true",
        help="Also judge `anchored:no-account`. That class is not an answer, it is an "
             "unasked question: the id is named and no window carried enough of the "
             "requirement's vocabulary to be an account, which says nothing about "
             "whether one of those places states the rule. Off by default because it "
             "takes ledger from 17 judge calls to 40 and pricing from 52 to 70.",
    )
    parser.add_argument(
        "--only-id-file", dest="only_id_file", default=None,
        help="A file of requirement ids, one per line, blank lines and `#` comments "
             "ignored. Use this rather than a shell loop building --only-id flags: "
             "zsh does not word-split an unquoted variable, so the loop silently "
             "passes one long string as a --gear path.",
    )
    return parser.parse_args(argv)


def main(argv=None):
    args = _parse_args(sys.argv[1:] if argv is None else argv)

    also_judge = frozenset({"anchored:no-account"}) if args.judge_anchored else frozenset()
    neighbourhoods = build_all(args.gears, also_judge)
    wanted = set(args.only_ids or [])
    if args.only_id_file is not None:
        with open(args.only_id_file, "r", encoding="utf-8") as handle:
            for line in handle:
                line = line.split("#", 1)[0].strip()
                if line:
                    wanted.add(line)
    if wanted:
        missing = wanted - {n["requirement_id"] for n in neighbourhoods}
        if missing:
            # A typo'd or renamed id must not quietly shrink the sample: an
            # evaluation run that judges 15 of 16 and says nothing is the exact
            # failure this tool exists to catch.
            raise CorpusError(
                "these requested id(s) are declared by no loaded gear: {}".format(
                    ", ".join(sorted(missing))))
        neighbourhoods = [n for n in neighbourhoods if n["requirement_id"] in wanted]
    if args.only_classes is not None:
        wanted = set(args.only_classes)
        neighbourhoods = [n for n in neighbourhoods if n["triage"] in wanted]

    counts = histogram(neighbourhoods)
    envelope = {
        "gears": list(args.gears),
        "thresholds": THRESHOLDS,
        "counts": counts,
        "neighbourhoods": neighbourhoods,
    }

    directory = os.path.dirname(os.path.abspath(args.out))
    if directory:
        os.makedirs(directory, exist_ok=True)
    with open(args.out, "w", encoding="utf-8") as handle:
        json.dump(envelope, handle, indent=2, ensure_ascii=False)
        handle.write("\n")

    print(render_histogram(counts, also_judge))
    print("\n{} neighbourhood(s) written to {}".format(len(neighbourhoods), args.out))
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except CorpusError as exc:
        print("Error: {}".format(exc), file=sys.stderr)
        sys.exit(1)
