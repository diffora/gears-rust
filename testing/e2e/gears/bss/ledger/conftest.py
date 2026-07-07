"""E2E fixtures for the bss-ledger gear.

The ledger REST surface (``/bss-ledger/v1``) is an opt-in gear that is NOT yet
wired into ``cf-gears-example-server`` (no dependency + no ``registered_gears``
entry). Until those routes are mounted, the whole module skips gracefully — we
do NOT want red failures for endpoints that intentionally are not served here.
Once the gear is wired, the probe below starts returning non-404 and every seam
test runs for real (mirrors the file-storage module's staging pattern).
"""

import os

import httpx
import pytest

REQUEST_TIMEOUT = 5.0

# The gear's REST base path (design §3.3 / the axum routers). Every ledger route
# is a literal full path under this prefix.
API_BASE = "/bss-ledger/v1"


@pytest.fixture(scope="session", autouse=True)
def require_ledger_mounted():
    """Skip the whole module unless the ledger control surface is reachable.

    Keys on ``GET /bss-ledger/v1/accounts`` (the read-only chart-of-accounts
    surface — the simplest authenticated GET). A 404 means the gear's routes are
    not mounted (it is an opt-in gear that may not be built into this server); a
    connection error means the server is down. Either way: skip, don't fail.

    The probe is **authenticated**: the API gateway returns 401 (not 404) for an
    unauthenticated request to any unknown path, so an auth-less probe could not
    tell "gear absent" (skip) from "auth required" (present). With a valid token
    an unknown route yields a clean 404.
    """
    base_url = os.getenv("E2E_BASE_URL", "http://localhost:8086")
    token = os.getenv("E2E_AUTH_TOKEN", "e2e-token-tenant-a")
    url = f"{base_url}{API_BASE}/accounts"
    try:
        with httpx.Client(timeout=REQUEST_TIMEOUT) as client:
            r = client.get(url, headers={"Authorization": f"Bearer {token}"})
    except httpx.HTTPError as exc:
        pytest.skip(f"cf-gears-server not reachable at {base_url}: {exc}")
    if r.status_code == 404:
        pytest.skip(
            "bss-ledger REST endpoints are not mounted — the gear is an opt-in "
            "gear not built into this server (no registered_gears entry)."
        )


@pytest.fixture
def api_base():
    return API_BASE
