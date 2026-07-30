#!/usr/bin/env python3
"""Turn judged neighbourhoods into the N1 report, enforcing the honesty rules.

Step 3 of three. Run from the repository root:

    python3 .claude/skills/spec-check/scripts/judge_report.py \\
      --neighbourhoods .spec-check/neighbourhoods-ledger.json \\
      --verdicts .spec-check/verdicts-ledger.json \\
      --out docs/spec-check/N1-ledger.md

Nothing gates: this exits 0 whatever the report says. It exits 1 when its inputs
will not load or contradict each other, and 2 on a usage error.
"""

import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

COVERAGE_VALUES = ("specified", "claim-only", "underspecified")
#: `contradicts-declaration` is not a comparison between accounts — it is the one
#: case the other three cannot express: the design set states a rule that contradicts
#: the requirement's own declaration. Measured on the 2026-07-30 evaluation sample it
#: applied to 2 of 12, both of which had to be recorded as `underspecified` instead,
#: which reads as "someone forgot to finish this" rather than "someone decided the
#: opposite". Side A is fragment 0 of the neighbourhood, so it is citable exactly like
#: any region; only this vocabulary and the agent's instructions were missing.
AGREEMENT_VALUES = ("consistent", "divergent", "not-applicable", "contradicts-declaration")
ROLE_VALUES = ("specifies", "mentions")
USEFULNESS_VALUES = ("decisive", "useful", "noise")

_REQUIRED_KEYS = (
    "id", "regions", "coverage", "agreement", "citations", "reasoning",
)


class InputError(Exception):
    """Inputs that will not load or contradict each other. Exits 1, like `check.py`."""


class VerdictError(Exception):
    """One verdict this report will not accept. Becomes a `judge-failed` row."""


def _fragment_spans(neighbourhood):
    return [
        (f["file"], f["lines"][0], f["lines"][1])
        for f in neighbourhood["fragments"]
    ]


def _check_schema(verdict):
    for key in _REQUIRED_KEYS:
        if key not in verdict:
            raise VerdictError("missing key `{}`".format(key))
    if verdict["coverage"] not in COVERAGE_VALUES:
        raise VerdictError("coverage `{}` is not one of {}".format(
            verdict["coverage"], ", ".join(COVERAGE_VALUES)))
    if verdict["agreement"] not in AGREEMENT_VALUES:
        raise VerdictError("agreement `{}` is not one of {}".format(
            verdict["agreement"], ", ".join(AGREEMENT_VALUES)))
    if not isinstance(verdict["regions"], list) or not isinstance(verdict["citations"], list):
        raise VerdictError("`regions` and `citations` must both be lists")
    for region in verdict["regions"]:
        for key in ("file", "lines", "role", "usefulness"):
            if key not in region:
                raise VerdictError("a region is missing key `{}`".format(key))
        if region["role"] not in ROLE_VALUES:
            raise VerdictError("region role `{}` is not one of {}".format(
                region["role"], ", ".join(ROLE_VALUES)))
        if region["usefulness"] not in USEFULNESS_VALUES:
            raise VerdictError("region usefulness `{}` is not one of {}".format(
                region["usefulness"], ", ".join(USEFULNESS_VALUES)))
    for citation in verdict["citations"]:
        for key in ("file", "line", "quote"):
            if key not in citation:
                raise VerdictError("a citation is missing key `{}`".format(key))
    # `proposed_fix` is deliberately absent from `_REQUIRED_KEYS`: whether it is
    # required depends on coverage and agreement *after* any downgrade, so the rule
    # lives in `normalise` and nowhere else. Requiring the key here as well made a
    # missing key fail differently from an empty one — measured on the ledger step-1
    # run, that cost `fr-idempotency-per-flow`, the requirement the run existed to
    # answer, a `judge-failed` row for a verdict its own agent contract calls valid.


def _check_citations_inside_fragments(verdict, neighbourhood):
    """Every citation must land inside a fragment the judge was actually given.

    Beyond the design, deliberately. "The judge has no repository access" is
    otherwise an untestable claim about the harness; this makes its violation
    visible, because a judge that answered from the repository cites a line that
    was never in its neighbourhood — the one failure mode nobody can audit later.
    """
    spans = _fragment_spans(neighbourhood)
    for citation in verdict["citations"]:
        inside = any(
            citation["file"] == path and start <= citation["line"] <= end
            for path, start, end in spans
        )
        if not inside:
            raise VerdictError(
                "citation {}:{} is outside every fragment of this neighbourhood".format(
                    citation["file"], citation["line"]
                )
            )


def _distinct_locations(citations):
    return {(c["file"], c["line"]) for c in citations}


def _declaration_span(neighbourhood):
    """Side A, identified by role rather than position — `None` if absent."""
    for fragment in neighbourhood["fragments"]:
        if fragment.get("role") == "requirement-declaration":
            return (fragment["file"], fragment["lines"][0], fragment["lines"][1])
    return None


def _cites_declaration(citations, span):
    if span is None:
        return False
    path, start, end = span
    return any(c["file"] == path and start <= c["line"] <= end for c in citations)


def normalise(verdict, neighbourhood):
    """`(verdict, notes)` — the verdict as it will be reported, and every downgrade.

    Raises `VerdictError` for anything that must be reported as `judge-failed`
    rather than downgraded.
    """
    _check_schema(verdict)
    _check_citations_inside_fragments(verdict, neighbourhood)

    out = dict(verdict)
    notes = []

    specifies = [r for r in out["regions"] if r["role"] == "specifies"]
    # `contradicts-declaration` is exempt: its two sides are one account and side A,
    # so requiring two accounts would erase exactly the verdict it exists to carry.
    if (len(specifies) < 2
            and out["agreement"] not in ("not-applicable", "contradicts-declaration")):
        notes.append(
            "{} → not-applicable: agreement is derived only from regions the judge "
            "marked `specifies`, and this verdict marks {} — there is nothing to "
            "compare".format(out["agreement"], len(specifies))
        )
        out["agreement"] = "not-applicable"

    if out["agreement"] == "contradicts-declaration":
        # The mirror of honesty rule 2, one side of which is fixed: a contradiction
        # of the declaration must show the declaration. Citing only design regions is
        # a claim about accounts disagreeing with each other, which is `divergent`;
        # citing only the declaration shows nothing contradicting it.
        span = _declaration_span(neighbourhood)
        outside = [c for c in out["citations"]
                   if not _cites_declaration([c], span)]
        if not (_cites_declaration(out["citations"], span) and outside):
            notes.append(
                "contradicts-declaration → not-applicable: this verdict must cite the "
                "declaration and, in a distinct location, the account that contradicts "
                "it; it cites {} inside the declaration and {} outside it".format(
                    len(out["citations"]) - len(outside), len(outside))
            )
            out["agreement"] = "not-applicable"

    if out["agreement"] == "divergent":
        locations = _distinct_locations(out["citations"])
        if len(locations) < 2:
            notes.append(
                "divergent → consistent: honesty rule 2 requires `file:line` for the "
                "assertion and `file:line` for what contradicts it, in distinct "
                "locations; this verdict cites {} distinct location(s)".format(len(locations))
            )
            out["agreement"] = "consistent"

    if not (out["coverage"] == "specified" and out["agreement"] == "consistent"):
        if not str(out.get("proposed_fix") or "").strip():
            raise VerdictError(
                "`proposed_fix` is required for every verdict other than specified + "
                "consistent — a row that says only that something is wrong costs the "
                "reader the whole investigation"
            )

    return out, notes


#: Report order: the outcomes a reader must act on first. Within a verdict, by id.
_VERDICT_ORDER = (
    "divergent", "contradicts-declaration", "judge-failed", "claim-only",
    "underspecified", "no-region", "no-prose", "specified", "consistent",
    "no-account", "not-judged",
)


def _short(requirement_id):
    """`cpt-cf-bss-ledger-fr-thing` → `fr-thing`. The prefix is stated once, in the
    report header, instead of 40 times in a table column."""
    for marker in ("-fr-", "-nfr-"):
        index = requirement_id.find(marker)
        if index != -1:
            return requirement_id[index + 1:]
    return requirement_id


def _assertion_and_contradiction(verdict):
    citations = verdict.get("citations") or []
    plain = ["{}:{}".format(c["file"], c["line"]) for c in citations]
    assertion = plain[0] if plain else "—"
    contradiction = "—"
    for reference in plain[1:]:
        if reference != assertion:
            contradiction = reference
            break
    return assertion, contradiction


def rows(envelope, verdicts):
    """`(rows, notes)` — one row per neighbourhood, and every recorded downgrade.

    Every neighbourhood produces a row, judged or not. Honesty rule 1: zero
    neighbourhoods and zero findings must never look alike.
    """
    by_id = {}
    for verdict in verdicts:
        ident = verdict.get("id")
        if ident is None:
            raise InputError(
                "a verdict carries no `id`, so it cannot be matched to a neighbourhood"
            )
        by_id.setdefault(ident, verdict)

    known = {n["id"] for n in envelope["neighbourhoods"]}
    for ident in by_id:
        if ident not in known:
            raise InputError(
                "verdict for {} names no neighbourhood in this run — a verdict that "
                "came from nowhere is a wiring bug, not a finding".format(ident)
            )

    out = []
    notes = []
    for neighbourhood in envelope["neighbourhoods"]:
        ident = neighbourhood["id"]
        short = _short(neighbourhood["requirement_id"])
        base = {
            "id": ident,
            "requirement_id": neighbourhood["requirement_id"],
            "short": short,
            "kind": neighbourhood.get("requirement_kind", "fr"),
            "triage": neighbourhood["triage"],
            "declared_at": "{}:{}".format(
                neighbourhood.get("declaration_file", "PRD.md"),
                neighbourhood.get("declaration_line", ""),
            ).rstrip(":"),
            "assertion": "—",
            "contradicted_by": "—",
            "proposed_fix": "—",
            "reasoning": "",
            "regions": [],
        }

        if not neighbourhood["judge"]:
            if neighbourhood["triage"] == "unbuildable:no-prose":
                base["verdict"] = "no-prose"
                base["reasoning"] = neighbourhood["unbuildable"][0]
            elif neighbourhood["triage"] == "no-region":
                base["verdict"] = "no-region"
                base["reasoning"] = neighbourhood["unbuildable"][0]
            elif neighbourhood["triage"] == "anchored:no-account":
                # Not `covered:strong` and emphatically not its opposite: the id is
                # named, and nothing named it carries enough of the requirement's
                # vocabulary to be an account of it. Reporting these under the
                # `covered:strong` reason asserted the reverse of the truth for 10 of
                # the 16 requirements in the ledger step-1 run.
                base["verdict"] = "no-account"
                base["reasoning"] = (
                    "the id is named in the design set, but no region carries enough of "
                    "the requirement's vocabulary to be an account of it — a fact about "
                    "the search, not a judgment that the requirement is unspecified"
                )
            else:
                base["verdict"] = "not-judged"
                base["reasoning"] = (
                    "single id-anchored region scoring at or above the strong threshold, "
                    "with normative prose — not judged, and this is the record of why"
                )
            out.append(base)
            continue

        verdict = by_id.get(ident)
        if verdict is None:
            base["verdict"] = "judge-failed"
            base["reasoning"] = "no verdict was returned for this neighbourhood"
            out.append(base)
            continue

        try:
            normalised, verdict_notes = normalise(verdict, neighbourhood)
        except VerdictError as exc:
            base["verdict"] = "judge-failed"
            base["reasoning"] = str(exc)
            out.append(base)
            continue

        assertion, contradiction = _assertion_and_contradiction(normalised)
        base.update({
            # The agreement axis names the row only when it carries a defect the
            # coverage axis would hide: `specified + contradicts-declaration` is not a
            # covered requirement, it is a decided-the-other-way one.
            "verdict": normalised["agreement"]
                       if normalised["agreement"] in ("divergent", "contradicts-declaration")
                       else normalised["coverage"],
            "coverage": normalised["coverage"],
            "agreement": normalised["agreement"],
            "assertion": assertion,
            "contradicted_by": contradiction,
            "proposed_fix": str(normalised.get("proposed_fix") or "").strip() or "—",
            "reasoning": normalised.get("reasoning", ""),
            "regions": normalised["regions"],
        })
        out.append(base)
        for note in verdict_notes:
            notes.append((short, note))

    rank = {name: index for index, name in enumerate(_VERDICT_ORDER)}
    out.sort(key=lambda row: (rank.get(row["verdict"], len(rank)), row["requirement_id"]))
    return out, notes


def _usefulness_table(envelope, report_rows):
    """Counts of `decisive`/`useful`/`noise` per selection mechanism.

    The tuning channel the design asked for: without it, a threshold change is
    taste. Joined back to the neighbourhood by `(file, lines)`, which is why
    fragments carry real line numbers.
    """
    provenance = {}
    for neighbourhood in envelope["neighbourhoods"]:
        for fragment in neighbourhood["fragments"]:
            if fragment["role"] != "candidate-region":
                continue
            key = (neighbourhood["id"], fragment["file"],
                   fragment["lines"][0], fragment["lines"][1])
            provenance[key] = fragment.get("selected_by", "unknown")

    counts = {}
    for row in report_rows:
        for region in row.get("regions") or []:
            key = (row["id"], region["file"], region["lines"][0], region["lines"][1])
            mechanism = provenance.get(key, "unknown")
            bucket = counts.setdefault(mechanism, {name: 0 for name in USEFULNESS_VALUES})
            bucket[region["usefulness"]] += 1
    return counts


def _cell(text):
    """Markdown table cells cannot contain a raw `|` or a newline."""
    return str(text).replace("|", "\\|").replace("\n", " ").strip()


def _batching_note(manifest):
    """One paragraph stating how judging was batched, or that it was not.

    The design requires one neighbourhood per dispatch so verdicts stay
    independent. Batching trades some of that for cost, and the trade has to be
    legible in the artifact, not only in the tool that made it.
    """
    if manifest is None:
        return (
            "Judging was **not batched**, or the batch manifest was not supplied: "
            "this report cannot say how many dispatches produced these verdicts."
        )
    batches = manifest.get("batches", [])
    sizes = [len(entry.get("ids", [])) for entry in batches]
    multi = [n for n in sizes if n > 1]
    if not multi:
        return (
            "Judged in {} dispatch(es), one neighbourhood each — verdicts are "
            "independent, as the design requires.".format(len(batches))
        )
    return (
        "Judged in **{} dispatch(es)** for {} neighbourhood(s), at most {} per "
        "dispatch. {} dispatch(es) carried more than one, so those verdicts were "
        "**not produced in isolation** — a deliberate deviation from the design, "
        "bounded by the batching rule that no two members of a dispatch quote "
        "overlapping lines of any document.".format(
            len(batches), sum(sizes), max(sizes), len(multi))
    )


def render(envelope, report_rows, notes, manifest=None):
    """The whole report. No timestamp, no run-dependent ordering: regenerating it
    with unchanged inputs must diff clean.

    References are plain `path:line` text, never markdown links — `make lychee`
    walks `docs`, and a line-anchor link would break it.
    """
    gears = ", ".join(envelope["gears"])
    thresholds = envelope["thresholds"]
    counts = {}
    for row in report_rows:
        counts[row["verdict"]] = counts.get(row["verdict"], 0) + 1

    out = []
    out.append("# N1 — requirement coverage and prose agreement")
    out.append("")
    out.append("Corpus: `{}`. {} requirement(s).".format(gears, len(report_rows)))
    out.append("")
    out.append(
        "Generated by `judge_report.py` from a `neighbourhoods.json` / `verdicts.json` "
        "pair. **Advisory: nothing gates on this report.** Requirement ids are "
        "abbreviated — a row reading `fr-thing` is `cpt-cf-<gear>-fr-thing`. References "
        "are plain `path:line` text on purpose, not links."
    )
    out.append("")
    out.append("## Run configuration")
    out.append("")
    out.append("| Setting | Value |")
    out.append("|---|---|")
    for key in sorted(thresholds):
        out.append("| {} | {} |".format(key.replace("_", " "), thresholds[key]))
    out.append("")
    out.append("These are starting values, tuned from the region usefulness table below.")
    out.append("")
    out.append("## How this was judged")
    out.append("")
    out.append(_batching_note(manifest))
    out.append("")

    out.append("## Outcomes")
    out.append("")
    out.append("| outcome | count |")
    out.append("|---|---|")
    for name in _VERDICT_ORDER:
        if name in counts:
            out.append("| {} | {} |".format(name, counts[name]))
    out.append("")

    judged_rows = [r for r in report_rows if r["verdict"] not in
                   ("no-prose", "no-region", "no-account", "not-judged")]
    out.append("## Judged")
    out.append("")
    if judged_rows:
        out.append("| requirement | verdict | assertion | contradicted by | proposed fix |")
        out.append("|---|---|---|---|---|")
        for row in judged_rows:
            out.append("| {} | {} | {} | {} | {} |".format(
                row["short"], row["verdict"], row["assertion"],
                row["contradicted_by"], _cell(row["proposed_fix"]),
            ))
    else:
        out.append("No neighbourhood in this run was judged.")
    out.append("")

    # Every row that carries a reason must show it. A row reading `judge-failed`
    # and nothing else tells a reader that something went wrong and withholds the
    # only thing that would let them fix it — the same defect honesty rule 1 exists
    # to prevent, one level down.
    for heading, verdict_name, blurb in (
        ("Judge failures", "judge-failed",
         "A verdict this report would not accept, or none returned at all. Recorded "
         "with the reason: a neighbourhood can never drop out silently."),
        ("Not judged — no prose", "no-prose",
         "The declaration exists and says nothing. A defect in the PRD itself, "
         "reported rather than skipped."),
        ("Not judged — no region", "no-region",
         "Reported as what is actually known: sending an empty neighbourhood to a "
         "judge would produce invention, and calling it `claim-only` would overclaim."),
        ("Not judged — anchored, no account", "no-account",
         "The design set names the id, and no location carrying it states enough of "
         "the rule to be an account of it. Answered deterministically: these were "
         "never sent to a judge, and they are not evidence that the requirement is "
         "unspecified — only that term overlap did not find an account."),
        ("Not judged — covered", "not-judged",
         "A single id-anchored region scoring at or above the strong threshold, with "
         "normative prose. Listed with the reason, so a silent skip and a deliberate "
         "one are distinguishable."),
    ):
        group = [r for r in report_rows if r["verdict"] == verdict_name]
        if not group:
            continue
        out.append("## {} ({})".format(heading, len(group)))
        out.append("")
        out.append(blurb)
        out.append("")
        out.append("| requirement | declared at | reason |")
        out.append("|---|---|---|")
        for row in group:
            out.append("| {} | {} | {} |".format(
                row["short"], row["declared_at"], _cell(row["reasoning"])
            ))
        out.append("")

    out.append("## Downgrades ({})".format(len(notes)))
    out.append("")
    if notes:
        out.append(
            "Applied by `judge_report.py`, not by the judge. Both honesty rules are "
            "machine-enforced, so they no longer depend on a prompt."
        )
        out.append("")
        out.append("| requirement | downgrade |")
        out.append("|---|---|")
        for short, note in notes:
            out.append("| {} | {} |".format(short, _cell(note)))
    else:
        out.append("None: every verdict was reported as the judge returned it.")
    out.append("")

    usefulness = _usefulness_table(envelope, report_rows)
    out.append("## Region usefulness by selection mechanism")
    out.append("")
    out.append(
        "The tuning channel. `id-anchor` regions that come back `noise` argue for a "
        "tighter anchor rule; `term-overlap` regions that come back `decisive` argue "
        "the heuristic is carrying the design, as the D-15 premise run suggested."
    )
    out.append("")
    out.append("| mechanism | decisive | useful | noise |")
    out.append("|---|---|---|---|")
    for mechanism in sorted(usefulness):
        bucket = usefulness[mechanism]
        out.append("| {} | {} | {} | {} |".format(
            mechanism, bucket["decisive"], bucket["useful"], bucket["noise"]
        ))
    if not usefulness:
        out.append("| — | 0 | 0 | 0 |")
    out.append("")
    return "\n".join(out) + "\n"


def _load_json(path):
    try:
        with open(path, "r", encoding="utf-8") as handle:
            return json.load(handle)
    except (OSError, ValueError) as exc:
        raise InputError("{} could not be read as JSON: {}".format(path, exc))


def _verdict_list(payload):
    """Accepts a bare list or `{"verdicts": [...]}` — the agent writes this file,
    and both shapes are natural enough that rejecting one would just cost a rerun.
    """
    if isinstance(payload, dict):
        payload = payload.get("verdicts")
    if not isinstance(payload, list):
        raise InputError("the verdicts file must be a JSON list, or an object with a "
                         "`verdicts` list")
    return payload


def _default_out(envelope):
    gears = envelope["gears"]
    name = os.path.basename(os.path.dirname(gears[0])) if len(gears) == 1 else "combined"
    return os.path.join("docs", "spec-check", "N1-{}.md".format(name))


def _parse_args(argv):
    parser = argparse.ArgumentParser(
        prog="judge-report", description="Render the N1 report from judged neighbourhoods"
    )
    parser.add_argument("--neighbourhoods", required=True)
    parser.add_argument("--verdicts", required=True)
    parser.add_argument(
        "--batches", default=None,
        help="The manifest.json written by judge_batches.py. When judging was "
             "batched, pass it: the report then states how many dispatches produced "
             "these verdicts and that members of a batch were not judged in "
             "isolation. A deviation the reader cannot see is a deviation nobody "
             "can weigh.",
    )
    parser.add_argument(
        "--out", default=None,
        help="Defaults to docs/spec-check/N1-<gear>.md. Never write this inside a "
             "gear's docs/ tree: a corpus loads every *.md under its root, so the "
             "report would become a document the next run parses.",
    )
    return parser.parse_args(argv)


def main(argv=None):
    args = _parse_args(sys.argv[1:] if argv is None else argv)
    envelope = _load_json(args.neighbourhoods)
    verdicts = _verdict_list(_load_json(args.verdicts))
    report_rows, notes = rows(envelope, verdicts)
    manifest = _load_json(args.batches) if args.batches else None
    text = render(envelope, report_rows, notes, manifest)

    out = args.out or _default_out(envelope)
    segments = out.replace(os.sep, "/").split("/")
    if "gears" in segments and "docs" in segments[segments.index("gears"):]:
        raise InputError(
            "{} is inside a gear docs tree; the report must go to docs/spec-check/ at "
            "the repository root, or the next run will parse it as a document".format(out)
        )
    directory = os.path.dirname(os.path.abspath(out))
    if directory:
        os.makedirs(directory, exist_ok=True)
    with open(out, "w", encoding="utf-8") as handle:
        handle.write(text)

    print("{} row(s), {} downgrade(s) → {}".format(len(report_rows), len(notes), out))
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except InputError as exc:
        print("Error: {}".format(exc), file=sys.stderr)
        sys.exit(1)
