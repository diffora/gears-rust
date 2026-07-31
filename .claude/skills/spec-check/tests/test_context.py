import subprocess
import sys

from conftest import REPO_ROOT, SCRIPTS

sys.path.insert(0, str(SCRIPTS))

from spec_check import context  # noqa: E402

CHECK_PY = SCRIPTS / "check.py"


def run(*args):
    return subprocess.run(
        [sys.executable, str(CHECK_PY)] + list(args),
        cwd=str(REPO_ROOT), stdout=subprocess.PIPE, stderr=subprocess.PIPE, encoding="utf-8",
    )


def find(gear):
    """`discover` on an absolute gear path, answers made repo-relative again.

    The unit tests must not depend on pytest's working directory, which is the skill
    root rather than the repository root.
    """
    paths, unresolved = context.discover(str(REPO_ROOT / "gears" / "bss" / gear / "docs"))
    return [p.replace(str(REPO_ROOT) + "/", "") for p in paths], unresolved


def test_pricing_discovers_exactly_the_two_gears_it_needs():
    # The case the feature exists for, and the one id discovery alone cannot solve:
    # pricing cites no foreign id at all, only `SEAMS M10`-style bare seam ids.
    paths, unresolved = find("pricing")
    assert paths == ["gears/bss/rating/docs", "gears/bss/subscriptions/docs"]
    assert unresolved == []


def test_rating_is_discovered_through_ids_not_links():
    # The mirror case: rating has no outbound relative links, only foreign ids.
    paths, unresolved = find("rating")
    assert "gears/bss/pricing/docs" in paths
    assert "gears/bss/products/docs" in paths


def test_a_cited_gear_that_does_not_exist_is_returned_not_dropped():
    # rating cites `tariffs`, consolidated away. A citation pointing at a gear that
    # is not there is a finding about the documents, not a lookup miss to swallow.
    _paths, unresolved = find("rating")
    assert unresolved == ["tariffs"]


def test_ledger_is_genuinely_standalone():
    # No links out, no foreign ids, no SEAMS citations — and the docs agree.
    paths, unresolved = find("ledger")
    assert paths == []
    assert unresolved == []


def test_discovery_never_returns_the_gear_itself():
    for gear in ("pricing", "rating", "subscriptions", "ledger"):
        paths, _ = find(gear)
        assert "gears/bss/{}/docs".format(gear) not in paths


def test_auto_context_clears_the_seams_pricing_alone_reports():
    # The documented symptom: pricing alone reports 6 P1/seam-undefined, every one a
    # row that does exist in a sibling gear. Auto-context must clear them without the
    # caller knowing the graph. (4 until 2026-07-31; D-79 `SEAMS SUB-P8` and D-80
    # `SEAMS SUB-P5` joined the honest cross-gear citations that round.)
    alone = run("--gear", "gears/bss/pricing/docs")
    auto = run("--gear", "gears/bss/pricing/docs", "--auto-context")
    assert alone.stdout.count("seam-undefined") == 6
    assert auto.stdout.count("seam-undefined") == 0


def test_auto_context_names_what_it_loaded_and_what_it_could_not():
    # A run that silently widens its own corpus is unreproducible from its output.
    auto = run("--gear", "gears/bss/rating/docs", "--auto-context")
    assert "gears/bss/pricing/docs" in auto.stdout
    assert "tariffs" in auto.stdout


def test_auto_context_is_opt_in():
    # Default-on would pull `products` into the documented three-gear invocation via
    # rating's citations and redden the byte-frozen oracles.
    default = run("--gear", "gears/bss/pricing/docs")
    assert "auto-context" not in default.stdout
