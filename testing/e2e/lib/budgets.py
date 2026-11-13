"""Time budgets shared by the e2e harness and tools/scripts.

tools/scripts/ci.py and coverage.py load this file by path, so keep it
standard-library only: it runs before the e2e requirements are installed.
"""

from __future__ import annotations

import os

STARTUP_TIMEOUT_ENV = "E2E_STARTUP_TIMEOUT_SECS"
_DEFAULT_STARTUP_TIMEOUT_SECS = 60


def _startup_timeout_secs() -> int:
    raw = os.environ.get(STARTUP_TIMEOUT_ENV, "").strip()
    if not raw:
        return _DEFAULT_STARTUP_TIMEOUT_SECS
    try:
        value = int(raw)
    except ValueError:
        raise ValueError(
            f"{STARTUP_TIMEOUT_ENV}={raw!r}: expected a positive number of seconds"
        ) from None
    if value <= 0:
        raise ValueError(
            f"{STARTUP_TIMEOUT_ENV}={raw!r}: expected a positive number of seconds"
        )
    return value


# How long a server may take to answer its readiness probe. On gears-large
# runners the server needs up to ~20 s, most of it in REST route registration.
# A server that crashes is still reported at once: the readiness loops check
# whether the process has exited on every poll. Set E2E_STARTUP_TIMEOUT_SECS
# to give a slow runner more time. Refs #5798, #5837.
SERVER_STARTUP_TIMEOUT_SECS = _startup_timeout_secs()
