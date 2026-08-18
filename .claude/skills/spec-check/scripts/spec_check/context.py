"""Which other gears a gear's documents point at.

P1 and P3 resolve across gears: a `SEAMS <id>` propagation target is looked up in
whichever loaded gear defines that seam row, and instruction ids resolve across the
whole loaded set. Run a gear alone and its honest cross-gear citations become findings
against it — measured, pricing alone reports 4 `P1/seam-undefined`, rating alone 7
`P3/inst-dangling`. The documented fix is "pass every related gear", which means the
caller has to know the graph.

Two channels are needed, because neither covers the corpus alone (measured 2026-07-30):

    gear           links `../../<g>/docs`      ids `cpt-cf-bss-<g>-`
    pricing        rating, subscriptions       (none)
    ledger         (none)                      (none)
    rating         (none)                      pricing, products, tariffs
    subscriptions  ledger, pricing, rating     rating

`pricing` is the case the feature exists for and it cites **no** foreign id: its
cross-gear dependency runs through bare seam ids (`SEAMS M10`, `SEAMS M12`, `SEAMS O3`)
that carry no gear name at all. Id discovery alone would find nothing for it. Link
discovery alone would find nothing for rating. The union gets all four right, and
`ledger` correctly comes back standalone.

One hop, never transitive closure: pricing needs {rating, subscriptions}, and following
rating's own citations from there would pull in the world for no measured gain.
"""

import os
import re

#: `](../../<gear>/docs` — a markdown link leaving this gear's tree.
_LINK = re.compile(r"\]\(\.\./\.\./([a-z0-9]+)/docs")

#: `cpt-cf-bss-<gear>-…` — an id belonging to another gear. The gear is **one**
#: segment: allowing `-` inside it makes the class greedy past the separator and
#: yields `pricing-actor` for `cpt-cf-bss-pricing-actor-rating`. No gear in this
#: repository carries a hyphen, and a new one that did would need this reconsidered.
_FOREIGN_ID = re.compile(r"cpt-cf-bss-([a-z0-9]+)-")


def _names(text):
    return set(_LINK.findall(text)) | set(_FOREIGN_ID.findall(text))


def discover(gear):
    """`(paths, unresolved)` for the gears `gear`'s documents point at.

    `paths` are sibling `<parent>/<name>/docs` directories that exist, sorted and
    excluding `gear` itself. `unresolved` are cited names with no such directory —
    returned rather than dropped, because a citation pointing at a gear that is not
    there is a finding about the documents, not a lookup miss to swallow. `rating`
    cites `tariffs`, which was consolidated away.
    """
    gear = os.path.normpath(gear)
    root = os.path.dirname(os.path.dirname(gear))
    self_name = os.path.basename(os.path.dirname(gear))

    cited = set()
    for directory, _subdirs, filenames in os.walk(gear):
        for filename in filenames:
            if not filename.endswith(".md"):
                continue
            path = os.path.join(directory, filename)
            with open(path, "r", encoding="utf-8") as handle:
                cited |= _names(handle.read())
    cited.discard(self_name)

    paths, unresolved = [], []
    for name in sorted(cited):
        candidate = os.path.join(root, name, "docs")
        if os.path.isdir(candidate):
            paths.append(candidate)
        else:
            unresolved.append(name)
    return paths, unresolved
