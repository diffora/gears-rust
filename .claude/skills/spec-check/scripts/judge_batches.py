#!/usr/bin/env python3
"""Group judged neighbourhoods into batch prompts, one file per dispatch.

Step 2's cost control. Run from the repository root:

    python3 .claude/skills/spec-check/scripts/judge_batches.py \\
      --neighbourhoods .spec-check/neighbourhoods-ledger.json \\
      --out-dir .spec-check/batches-ledger

Every dispatch pays for its own context — system prompt, tool schemas, the agent's
instructions — before it reads a single fragment, and that fixed cost dominates a
neighbourhood's ~1.5k tokens. One dispatch per neighbourhood therefore pays the
overhead 69 times for the live corpora. Batching pays it once per batch.

**This is a deliberate deviation from the design**, which requires one
neighbourhood per dispatch so that verdicts stay independent. The mitigation is
mechanical rather than hopeful: **no two members of a batch may quote overlapping
lines of the same document**, so a judgment about one paragraph cannot be carried
into a verdict about another requirement that quotes that same paragraph.

Overlapping *spans*, not shared files. Comparing files alone sounds stricter and is
useless: every requirement of a gear declares itself in the same `PRD.md`, so every
pair conflicts and every batch holds exactly one member — measured, 17 judged ledger
neighbourhoods produced 17 batches. Two declarations 100 lines apart are two
different paragraphs, and treating them as one contaminating text buys nothing.

Nothing gates. Exit 0 on success, 1 when the input will not load, 2 on usage error.
"""

import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from spec_check.semantic import neighbourhood as nb  # noqa: E402

DEFAULT_BATCH_SIZE = 4

_SINGLE_INSTRUCTION = """\
Judge the one requirement below and respond with a single JSON object, exactly as
your instructions specify. No prose before or after, no code fence.

Copy the `id` verbatim from its `=== … ===` header, including the
`requirement/` prefix — the `Requirement:` line below does not carry it, and
`judge_report.py` matches verdicts to neighbourhoods on that exact string.
"""

_BATCH_INSTRUCTION = """\
Judge each of the {count} requirements below **independently** and respond with a
JSON **array** of {count} objects, in the order the requirements appear. Nothing
before or after the array, no code fence.

They are unrelated and deliberately quote no document in common: a conclusion about
one tells you nothing about another. Judge each one only from its own fragments, and
copy each `id` verbatim from its `=== … ===` header, including the
`requirement/` prefix — the `Requirement:` lines below do not carry it, and
`judge_report.py` matches verdicts to neighbourhoods on that exact string.
"""


class InputError(Exception):
    """Input that will not load. Exits 1, like the other two CLIs."""


def _spans(item):
    """Every `(file, start, end)` this neighbourhood quotes."""
    return [(f["file"], f["lines"][0], f["lines"][1]) for f in item["fragments"]]


def _overlaps(spans, others):
    for path, start, end in spans:
        for other_path, other_start, other_end in others:
            if path == other_path and start <= other_end and other_start <= end:
                return True
    return False


def group(neighbourhoods, size):
    """Batches of at most `size`, no two members quoting overlapping lines.

    Greedy and order-preserving, so the same input always yields the same batches:
    each neighbourhood goes into the first batch that has room and no overlapping
    span, otherwise it starts a new one. A neighbourhood that conflicts with
    everything simply ends up alone, which is the design's own default and never an
    error.
    """
    batches = []
    occupied = []
    for item in neighbourhoods:
        spans = _spans(item)
        for index, batch in enumerate(batches):
            if len(batch) < size and not _overlaps(spans, occupied[index]):
                batch.append(item)
                occupied[index].extend(spans)
                break
        else:
            batches.append([item])
            occupied.append(list(spans))
    return batches


def render_batch(batch):
    """One dispatch's entire prompt.

    Built from `render_for_judge`, so `selected_by`, `score` and the triage class
    stay hidden here too — the batching must not become a hole in the control.
    """
    out = []
    if len(batch) == 1:
        out.append(_SINGLE_INSTRUCTION)
    else:
        out.append(_BATCH_INSTRUCTION.format(count=len(batch)))
    for item in batch:
        out.append("")
        out.append("=== {} ===".format(item["id"]))
        out.append("")
        out.append(nb.render_for_judge(item))
    return "\n".join(out) + "\n"


def _load(path):
    try:
        with open(path, "r", encoding="utf-8") as handle:
            return json.load(handle)
    except (OSError, ValueError) as exc:
        raise InputError("{} could not be read as JSON: {}".format(path, exc))


def _parse_args(argv):
    parser = argparse.ArgumentParser(
        prog="judge-batches",
        description="Group judged neighbourhoods into batch prompts, one per dispatch",
    )
    parser.add_argument("--neighbourhoods", required=True)
    parser.add_argument("--out-dir", required=True)
    parser.add_argument(
        "--size", type=int, default=DEFAULT_BATCH_SIZE,
        help="Maximum neighbourhoods per dispatch (default {}). Larger batches cost "
             "less and make a schema slip more expensive, since a malformed array "
             "costs the whole batch a retry.".format(DEFAULT_BATCH_SIZE),
    )
    return parser.parse_args(argv)


def main(argv=None):
    args = _parse_args(sys.argv[1:] if argv is None else argv)
    if args.size < 1:
        print("Error: --size must be at least 1", file=sys.stderr)
        return 1

    envelope = _load(args.neighbourhoods)
    judged = [n for n in envelope["neighbourhoods"] if nb.judge_needed(n)]
    batches = group(judged, args.size)

    os.makedirs(args.out_dir, exist_ok=True)
    manifest = []
    for number, batch in enumerate(batches, start=1):
        name = "batch-{:02d}.md".format(number)
        with open(os.path.join(args.out_dir, name), "w", encoding="utf-8") as handle:
            handle.write(render_batch(batch))
        manifest.append({"batch": name, "ids": [item["id"] for item in batch]})
    with open(os.path.join(args.out_dir, "manifest.json"), "w", encoding="utf-8") as handle:
        json.dump({"batch_size": args.size, "batches": manifest}, handle,
                  indent=2, ensure_ascii=False)
        handle.write("\n")

    print("{} judged neighbourhood(s) → {} dispatch(es) of at most {}".format(
        len(judged), len(batches), args.size))
    for entry in manifest:
        print("  {}  {}".format(entry["batch"], " ".join(
            i.split("/", 1)[1] for i in entry["ids"])))
    print("\nnot judged: {} neighbourhood(s), reported deterministically".format(
        len(envelope["neighbourhoods"]) - len(judged)))
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except InputError as exc:
        print("Error: {}".format(exc), file=sys.stderr)
        sys.exit(1)
