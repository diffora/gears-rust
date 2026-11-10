"""Load testing/e2e/lib/budgets.py for the scripts in tools/scripts.

A plain `import lib.budgets` does not work here: tools/scripts/lib and
testing/e2e/lib are both packages named `lib`, so the file is loaded by path.
"""

import importlib.util
from pathlib import Path

BUDGETS_PATH = Path(__file__).resolve().parents[3] / "testing" / "e2e" / "lib" / "budgets.py"


def load_budgets():
    spec = importlib.util.spec_from_file_location("e2e_budgets", BUDGETS_PATH)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


SERVER_STARTUP_TIMEOUT_SECS = load_budgets().SERVER_STARTUP_TIMEOUT_SECS
