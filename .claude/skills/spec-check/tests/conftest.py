"""Test wiring: put `scripts/` on `sys.path` and name the live corpora once."""

import subprocess
import sys
from pathlib import Path

import pytest

SKILL_ROOT = Path(__file__).resolve().parent.parent
SCRIPTS = SKILL_ROOT / "scripts"
if str(SCRIPTS) not in sys.path:
    sys.path.insert(0, str(SCRIPTS))

#: Repository root — `.claude/skills/spec-check/tests` is four levels down.
REPO_ROOT = SKILL_ROOT.parent.parent.parent

#: The three gear docs trees, in the order the removed `make spec-check` target
#: passed them — which is what the frozen oracles were captured from. Pricing is
#: first, so `LIVE_GEARS[0]` is the corpus both pinned baselines were taken from.
#: These are repo-relative strings, not absolute paths: the corpus root is echoed
#: verbatim into two finding messages, so the oracles only reproduce when the
#: checker is invoked with exactly these arguments from the repository root.
LIVE_GEARS = [
    "gears/bss/pricing/docs",
    "gears/bss/rating/docs",
    "gears/bss/subscriptions/docs",
]

ORACLES = SKILL_ROOT / "tests" / "oracles"

CHECK_PY = SCRIPTS / "check.py"


def oracle(name):
    """Frozen stdout of the Rust binary, read as text."""
    return (ORACLES / name).read_text(encoding="utf-8")


def run_check(*args, **kwargs):
    """Runs the Python CLI from the repository root, returning (stdout, returncode).

    From the repository root specifically: the `--gear` arguments are relative and
    two finding messages echo them verbatim.

    `expect_stderr=True` inverts the stderr assertion for the runs that are meant
    to fail to load: the diagnostic must be there, and must carry the prefix Rust's
    `Termination for Result` produced (`Error: {err:?}`). The frozen oracles cover
    stdout only, so this is the one place that surface is pinned.
    """
    expect_stderr = kwargs.pop("expect_stderr", False)
    assert not kwargs, kwargs
    proc = subprocess.run(
        [sys.executable, str(CHECK_PY)] + list(args),
        cwd=str(REPO_ROOT),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        encoding="utf-8",
    )
    if expect_stderr:
        assert proc.stderr.startswith("Error: "), (
            "a corpus that cannot be loaded must report on stderr the way the Rust "
            "binary did: {!r}".format(proc.stderr)
        )
    else:
        assert proc.stderr == "", "the CLI must not write to stderr: {!r}".format(proc.stderr)
    return proc.stdout, proc.returncode


def live_args():
    """The `--gear a --gear b --gear c` argument list, as the Makefile passes it."""
    out = []
    for gear in LIVE_GEARS:
        out += ["--gear", gear]
    return out


@pytest.fixture
def repo_root():
    return REPO_ROOT
