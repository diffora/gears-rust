"""E2E fixtures for the bss-pricing gear.

The wiring is already paid: ``bss-pricing`` is in ``config/e2e-features.txt``,
``config/e2e-local.yaml`` carries a gear block (SQLite, defaults, the
in-repository fixture registry), and ``static-authz-plugin`` answers a valid
tenant with ``decision: true`` plus an ``in`` predicate on ``owner_tenant_id`` —
which is exactly the flat-``In`` shape the gear's PEP compiles and which
``require_constraints = true`` needs.

The reachability probe below is a **graceful guard**, not a wiring gap: a server
built WITHOUT the ``bss-pricing`` cargo feature does not mount these routes, so
the probe would 404 and the whole module skips rather than emitting red failures
for endpoints that binary simply does not serve.
"""

import os
import uuid

import httpx
import pytest

REQUEST_TIMEOUT = 5.0

# The gear's REST base path (design 3.3 / D-140's route shape).
API_BASE = "/bss-pricing/v1"

# ── Tenant identities (must match config/e2e-local.yaml static-authn-plugin) ──
#
# TENANT_A is the caller ``e2e-token-tenant-a`` authenticates as; TENANT_B is a
# different root outside A's subtree, and is the foreign caller the cross-tenant
# isolation test reads as.
TENANT_A_ID = "00000000-df51-5b42-9538-d2b56b7ee953"
TENANT_B_ID = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"


def base_url() -> str:
    return os.getenv("E2E_BASE_URL", "http://localhost:8086")


def token_a() -> str:
    return os.getenv("E2E_AUTH_TOKEN", "e2e-token-tenant-a")


@pytest.fixture(scope="session", autouse=True)
def require_pricing_mounted():
    """Skip the whole module unless the pricing routes are mounted.

    Keys on ``GET /bss-pricing/v1/catalog-version/frontier`` — the simplest
    authenticated GET, and the only route the gear served before this group.

    The probe is **authenticated**: the API gateway answers 401 (not 404) for an
    unauthenticated request to any unknown path, so an auth-less probe could not
    tell "gear absent" (skip) from "auth required" (present). With a valid token
    an unknown route yields a clean 404.
    """
    url = f"{base_url()}{API_BASE}/catalog-version/frontier"
    try:
        with httpx.Client(timeout=REQUEST_TIMEOUT) as client:
            response = client.get(url, headers={"Authorization": f"Bearer {token_a()}"})
    except httpx.HTTPError as exc:
        pytest.skip(f"cf-gears-server not reachable at {base_url()}: {exc}")
    if response.status_code == 404:
        pytest.skip(
            "bss-pricing REST endpoints are not mounted — this server was built "
            "without the `bss-pricing` cargo feature."
        )


@pytest.fixture
def api_base() -> str:
    return API_BASE


@pytest.fixture
def auth_headers() -> dict:
    """Tenant A's bearer token."""
    return {"Authorization": f"Bearer {token_a()}"}


@pytest.fixture
def auth_headers_tenant_b() -> dict:
    """Tenant B's bearer token — a root outside tenant A's subtree."""
    return {"Authorization": "Bearer e2e-token-tenant-b"}


@pytest.fixture
def auth_headers_nil_tenant() -> dict:
    """A token static-authz denies (the nil-UUID tenant), for the 403 probe."""
    return {"Authorization": "Bearer e2e-token-nil-tenant"}


@pytest.fixture
def client():
    with httpx.Client(base_url=base_url(), timeout=REQUEST_TIMEOUT) as http:
        yield http


@pytest.fixture
def idempotency_key() -> str:
    """A fresh client key per test, so a rerun is never answered a replay."""
    return str(uuid.uuid4())
