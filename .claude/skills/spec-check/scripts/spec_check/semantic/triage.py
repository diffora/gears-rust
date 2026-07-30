"""Which requirements are worth a judge call, and which are answered already.

Every requirement lands in exactly one class. Several conditions can hold at
once, so the *order* is what makes the outcome determinate.
"""

import re

from ..regions import STRONG_SCORE, is_substantive

#: In ladder order. The first condition that holds decides.
CLASSES = (
    "unbuildable:no-prose",
    "no-region",
    "anchored:no-account",
    "suspicious:multi-region",
    "suspicious:not-normative",
    "suspicious:weak-coverage",
    "covered:strong",
)

#: The three classes that spend a judge call. The other four are answered
#: deterministically — and are reported *with their reason*, never skipped:
#: zero neighbourhoods and zero findings must never look alike.
JUDGED = frozenset({
    "suspicious:multi-region",
    "suspicious:not-normative",
    "suspicious:weak-coverage",
})

#: `anchored:no-account` is the class the design did not have and the corpus
#: demanded. 47 of the 116 live requirements are named by an id anchor while no
#: region carries enough of their vocabulary to be an account of them — measured
#: 2026-07-30, 24 in pricing and 23 in ledger.
#:
#: It is deterministic, not judged, for the same reason `no-region` is: the search
#: found no statement of the rule, and a judge handed a neighbourhood of bare
#: citations can only invent. Unlike `no-region` it has citations to show, so the
#: report names them. What it must NOT say is `claim-only` — that is a judgment
#: about the documents, and this is a fact about the search.

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
    accounts = [r for r in regions if is_substantive(r)]
    if not accounts:
        # Regions exist — the id is named somewhere — but none of them states the
        # rule. Reported with its citations, not judged and not called claim-only.
        return "anchored:no-account"
    if len(accounts) >= 2:
        # Two accounts, so divergence is possible: it wins the ladder. Counting
        # citations here instead of accounts is what made this class swallow 95 %
        # of the corpus.
        return "suspicious:multi-region"
    if not is_normative(requirement.prose):
        return "suspicious:not-normative"
    region = accounts[0]
    if region.selected_by == "id-anchor" and region.score >= STRONG_SCORE:
        return "covered:strong"
    # A single account that fails either half of `covered:strong`: the rule is
    # general, so an anchored region scoring 0.65 and an unanchored one scoring
    # 0.95 both land here.
    return "suspicious:weak-coverage"
