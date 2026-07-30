"""The neighbourhood: the only thing a judge ever sees, and its JSON contract.

Fragments are excerpts with real line numbers, never whole files. Two renderings
exist on purpose — the JSON keeps selection provenance for the report and for
evaluation, and `render_for_judge` withholds it.
"""

from .triage import JUDGED

#: The exact wording each deterministic non-judged outcome is reported with.
#: Named once so the JSON, the report and the tests cannot drift apart.
#:
#: `no-region` must not be reported as either extreme. Calling it `claim-only`
#: would overclaim — a term mismatch can be a vocabulary mismatch, where the PRD
#: says "per-unit quantity" and the design says "seat count". So it says what is
#: actually known.
UNBUILDABLE_REASONS = {
    "unbuildable:no-prose": (
        "the declaration carries no prose block, so there is nothing to specify "
        "and nothing to derive search terms from — a defect in the PRD itself"
    ),
    "no-region": (
        "no fragment of the corpus matched this requirement's vocabulary — either "
        "it is unaddressed, or the design states it in different words"
    ),
}


def build(requirement, regions, triage_class):
    """The neighbourhood for one requirement, ready to serialise."""
    fragments = []
    if requirement.prose_lines is not None:
        fragments.append({
            "role": "requirement-declaration",
            "file": requirement.file,
            "lines": [requirement.prose_lines[0], requirement.prose_lines[1]],
            "text": requirement.prose,
        })
    for region in regions:
        fragments.append({
            "role": "candidate-region",
            "file": region.file,
            "lines": [region.start, region.end],
            "text": region.text,
            "selected_by": region.selected_by,
            "score": region.score,
            "matched_terms": region.matched,
        })

    reason = UNBUILDABLE_REASONS.get(triage_class)
    return {
        "id": "requirement/{}".format(requirement.id),
        "kind": "requirement",
        "gear": requirement.gear,
        "requirement_id": requirement.id,
        "requirement_kind": requirement.kind,
        "priority": requirement.priority,
        #: Where the declaration itself is. A `no-prose` neighbourhood has no
        #: fragment at all, so this is the only location its report row can name —
        #: and naming one is the difference between a finding and a shrug.
        "declaration_file": requirement.file,
        "declaration_line": requirement.line,
        "triage": triage_class,
        "judge": triage_class in JUDGED,
        "fragments": fragments,
        "unbuildable": [reason] if reason is not None else [],
    }


def judge_needed(neighbourhood):
    return bool(neighbourhood["judge"])


def render_for_judge(neighbourhood):
    """The neighbourhood as inline text, with everything a judge must not see removed.

    Withheld: `selected_by`, `score` and `triage`. The first two are the D-15
    control — a judge told a region was id-anchored is biased toward accepting it,
    and D-15 was validated blind. `triage` would hand the judge the conclusion it
    is being asked for.

    Enforced here, in code covered by a test, rather than in a prompt: a prompt
    rule about not looking at a field the prompt itself contains is not a rule.
    """
    out = []
    declaration = None
    candidates = []
    for fragment in neighbourhood["fragments"]:
        if fragment["role"] == "requirement-declaration":
            declaration = fragment
        else:
            candidates.append(fragment)

    out.append("Requirement: {}".format(neighbourhood["requirement_id"]))
    if declaration is not None:
        out.append("")
        out.append("Declaration — {}:{}-{}".format(
            declaration["file"], declaration["lines"][0], declaration["lines"][1]
        ))
        out.append("")
        out.append(declaration["text"])
    for number, fragment in enumerate(candidates, start=1):
        out.append("")
        out.append("Region {} — {}:{}-{}".format(
            number, fragment["file"], fragment["lines"][0], fragment["lines"][1]
        ))
        out.append("")
        out.append(fragment["text"])
    return "\n".join(out)
