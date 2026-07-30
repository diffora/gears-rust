"""Which requirements are worth a judge call, and which are answered already.

Every requirement lands in exactly one class. Several conditions can hold at
once, so the *order* is what makes the outcome determinate.
"""

import re

from ..regions import STRONG_SCORE

#: In ladder order. The first condition that holds decides.
CLASSES = (
    "unbuildable:no-prose",
    "no-region",
    "suspicious:multi-region",
    "suspicious:not-normative",
    "suspicious:weak-coverage",
    "covered:strong",
)

#: The three classes that spend a judge call. The other three are answered
#: deterministically — and are reported *with their reason*, never skipped:
#: zero neighbourhoods and zero findings must never look alike.
JUDGED = frozenset({
    "suspicious:multi-region",
    "suspicious:not-normative",
    "suspicious:weak-coverage",
})

#: Uppercase only, and word-bounded. `MUST` in prose is RFC 2119; `must` is
#: English. `**MUST**` matches, because the search runs over the raw prose and
#: the asterisks are not word characters.
_NORMATIVE = re.compile(r"\b(?:MUST NOT|MUST|SHALL|SHOULD)\b")


def is_normative(prose):
    return _NORMATIVE.search(prose) is not None


def classify(requirement, regions):
    """The class of one requirement, given the regions selected for it."""
    if not requirement.prose:
        # Nothing to derive terms from, and nothing for a judge to compare — a
        # defect in the PRD itself.
        return "unbuildable:no-prose"
    if not regions:
        # Terms were derived and nothing matched. Not `claim-only` (a term
        # mismatch can be a vocabulary mismatch: "per-unit quantity" vs "seat
        # count") and not judgeable either (an empty neighbourhood leaves a judge
        # inventing). Its own finding, worded as what is actually known.
        return "no-region"
    if len(regions) >= 2:
        # Divergence is possible, so it wins the ladder.
        return "suspicious:multi-region"
    if not is_normative(requirement.prose):
        return "suspicious:not-normative"
    region = regions[0]
    if region.selected_by == "id-anchor" and region.score >= STRONG_SCORE:
        return "covered:strong"
    # A single region that fails either half of `covered:strong`. The class name
    # is from the common case (score 4–7); the rule is the general one, so an
    # anchored region scoring 6 and an unanchored one scoring 12 both land here.
    return "suspicious:weak-coverage"
