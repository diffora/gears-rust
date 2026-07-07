"""Black-box E2E seam tests for the bss-ledger gear.

Drives the running ``cf-gears-server`` over HTTP (no in-process access). The
whole module is skipped by ``conftest.require_ledger_mounted`` until the gear's
routes are mounted, so these assert reachability + the coarse REST contract
(auth gate, scoped not-found, route presence) rather than deep payloads — the
full posting/allocation behaviour is covered by the crate's Postgres
integration tests.
"""

import uuid

import httpx
import pytest

REQUEST_TIMEOUT = 10.0

# The tenant the standard E2E token (`e2e-token-tenant-a`) authenticates as —
# its `subject_tenant_id` in config/e2e-local.yaml's static-authn-plugin. Reads
# are tenant-scoped, and several take `tenant_id` as a required query param, so
# the seam tests pass this (in-scope) id: an absent resource then reads as a
# clean 404 (not a scope miss), and list/read surfaces resolve to the caller's
# own — possibly empty — data.
TENANT_A = "00000000-df51-5b42-9538-d2b56b7ee953"

# Write surfaces (POST) whose route presence + auth gate we probe. Full paths
# under the gear's `/bss-ledger/v1` base (design §3.3 / the axum routers).
WRITE_PATHS = [
    "/journal-entries",
    "/payments",
    "/credit-notes",
    "/debit-notes",
    "/manual-adjustments",
    "/refunds",
    "/recognition-runs",
]

# Read-by-id surfaces (GET) — an absent/foreign id must read as a scoped 404.
GET_BY_ID_PATHS = [
    "/journal-entries/{id}",
    "/recognition-schedules/{id}",
    "/refunds/{id}",
    "/credit-notes/{id}",
]


def test_accounts_read_is_reachable(base_url, auth_headers, api_base):
    """The chart-of-accounts read surface is mounted and returns JSON.

    (The session probe already proved non-404; here we assert the authenticated
    read succeeds and yields a JSON body — the paginated chart envelope.)
    """
    with httpx.Client(timeout=REQUEST_TIMEOUT) as client:
        r = client.get(f"{base_url}{api_base}/accounts?tenant_id={TENANT_A}", headers=auth_headers)
    assert r.status_code == 200, f"expected 200, got {r.status_code}: {r.text}"
    # Either the canonical page envelope or a bare list — both are valid JSON.
    assert isinstance(r.json(), (dict, list))


def test_unknown_journal_entry_is_404(base_url, auth_headers, api_base):
    """An absent (or foreign-scoped) journal entry reads as 404, not 500/leak.

    A random entry id the caller's tenant never posted must be indistinguishable
    from one outside its authorized subtree — the same 404 either way (no
    existence leak / BOLA).
    """
    entry_id = uuid.uuid4()
    with httpx.Client(timeout=REQUEST_TIMEOUT) as client:
        r = client.get(
            f"{base_url}{api_base}/journal-entries/{entry_id}?tenant_id={TENANT_A}",
            headers=auth_headers,
        )
    assert r.status_code == 404, f"expected 404, got {r.status_code}: {r.text}"


def test_post_journal_entry_requires_auth(base_url, api_base):
    """Posting without a bearer token is rejected by the gateway auth gate (401).

    No ``auth_headers`` here on purpose: the write surface must never accept an
    unauthenticated post.
    """
    with httpx.Client(timeout=REQUEST_TIMEOUT) as client:
        r = client.post(f"{base_url}{api_base}/journal-entries", json={})
    assert r.status_code == 401, f"expected 401, got {r.status_code}: {r.text}"


def test_provisioning_route_is_present(base_url, auth_headers, api_base):
    """The seller-provisioning route exists (a bad body is a 4xx, never a 404).

    An empty body fails request validation / the seller predicate, so we expect
    some client error — the point is only that the route is MOUNTED (not 404),
    the one Foundation endpoint the design marks externally exposed.
    """
    with httpx.Client(timeout=REQUEST_TIMEOUT) as client:
        r = client.post(
            f"{base_url}{api_base}/provisioning",
            headers=auth_headers,
            json={},
        )
    assert r.status_code != 404, f"provisioning route must be mounted: {r.text}"
    assert 400 <= r.status_code < 500, (
        f"an empty provisioning body should be a 4xx, got {r.status_code}: {r.text}"
    )


def test_balances_read_is_reachable(base_url, auth_headers, api_base):
    """The current-balances read surface is mounted (no server error).

    A bare read may 200 (empty) or 400 (a required filter is missing), but never
    404 (route absent) or 5xx (server fault).
    """
    with httpx.Client(timeout=REQUEST_TIMEOUT) as client:
        r = client.get(f"{base_url}{api_base}/balances?tenant_id={TENANT_A}", headers=auth_headers)
    assert r.status_code != 404, f"balances route must be mounted: {r.text}"
    assert r.status_code < 500, f"balances read must not 5xx: {r.status_code} {r.text}"


def test_journal_entries_list_is_reachable(base_url, auth_headers, api_base):
    """The journal-entries list surface returns a JSON page envelope."""
    with httpx.Client(timeout=REQUEST_TIMEOUT) as client:
        r = client.get(f"{base_url}{api_base}/journal-entries?tenant_id={TENANT_A}", headers=auth_headers)
    assert r.status_code == 200, f"expected 200, got {r.status_code}: {r.text}"
    assert isinstance(r.json(), (dict, list))


@pytest.mark.parametrize("path", WRITE_PATHS)
def test_write_routes_require_auth(base_url, api_base, path):
    """Every write surface is behind the gateway auth gate — no token ⇒ 401.

    No ``auth_headers`` on purpose: an unauthenticated POST to a write route
    must never be accepted.
    """
    with httpx.Client(timeout=REQUEST_TIMEOUT) as client:
        r = client.post(f"{base_url}{api_base}{path}", json={})
    assert r.status_code == 401, (
        f"{path}: expected 401 without a token, got {r.status_code}: {r.text}"
    )


@pytest.mark.parametrize("path", WRITE_PATHS)
def test_write_routes_are_mounted(base_url, auth_headers, api_base, path):
    """Every write surface is mounted: an empty body is a 4xx, never a 404/5xx.

    Proves the route exists (request validation / a domain guard rejects the
    empty body) without asserting the exact rejection code.
    """
    with httpx.Client(timeout=REQUEST_TIMEOUT) as client:
        r = client.post(f"{base_url}{api_base}{path}", headers=auth_headers, json={})
    assert r.status_code != 404, f"{path}: route must be mounted: {r.text}"
    assert 400 <= r.status_code < 500, (
        f"{path}: an empty body should be a 4xx, got {r.status_code}: {r.text}"
    )


@pytest.mark.parametrize("path", GET_BY_ID_PATHS)
def test_get_by_id_absent_is_404(base_url, auth_headers, api_base, path):
    """A random id on each read-by-id surface reads as a scoped 404.

    Absent and foreign-scoped are indistinguishable — the same 404 either way
    (no existence leak / BOLA).
    """
    url = f"{base_url}{api_base}{path.format(id=uuid.uuid4())}?tenant_id={TENANT_A}"
    with httpx.Client(timeout=REQUEST_TIMEOUT) as client:
        r = client.get(url, headers=auth_headers)
    assert r.status_code == 404, f"{path}: expected 404, got {r.status_code}: {r.text}"


def test_not_found_is_problem_json(base_url, auth_headers, api_base):
    """The gear renders a not-found as RFC 9457 ``application/problem+json``.

    Checks the error envelope the gear's canonical-error middleware produces
    (content type + a ``status`` of 404 in the body), the machine-readable shape
    consumers match on.
    """
    url = f"{base_url}{api_base}/journal-entries/{uuid.uuid4()}?tenant_id={TENANT_A}"
    with httpx.Client(timeout=REQUEST_TIMEOUT) as client:
        r = client.get(url, headers=auth_headers)
    assert r.status_code == 404
    assert "problem+json" in r.headers.get("content-type", ""), (
        f"expected application/problem+json, got {r.headers.get('content-type')!r}"
    )
    body = r.json()
    assert body.get("status") == 404, f"problem body should carry status 404: {body}"
