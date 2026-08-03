"""What the deployed bss-pricing gear can honestly be asked.

These run against a real binary, behind a real gateway, with a real authn
resolver and a real PDP. That is the half no in-process test can reach: the
routes are mounted, the header contracts hold on the wire, and the PEP is
genuinely in the request path.

What this module **cannot** prove, and why — three independent reasons, any one
of them sufficient:

* **There is no publish route.** ``PublishService::commit`` needs a
  ``PublishAuthorization`` and Slice 5 — the approval store, its state machine
  and the threshold policy — has no code at all, so the endpoint that would open
  the publication path cannot be built. Nothing here can produce a
  ``CatalogVersion``.
* **There is no registry gear in this repository.** The gear boots with
  ``UnconfiguredCatalogVersionRegistryV1``, which fails every version request
  closed by design.
* **The joint conformance fixture gate is CLOSED for every model kind** in this
  deployment, so even with an approval store every publish would refuse per kind
  with ``FIXTURE_MISSING``.

``tests/sqlite_read_model.rs`` already proves the publish → sweep → pin-eligible
path in process. Asserting a weaker version of it over HTTP would prove nothing
new and would fail for reasons having nothing to do with the surface.
"""

import uuid

import pytest


def _problem_codes(payload: dict) -> set:
    """Every machine-readable code in an RFC 9457 problem document.

    The code rides either the ``reason`` of an aborted/denied error or a
    precondition violation's ``type``; both spellings are the platform's, so both
    are read rather than one being assumed. §3.3 makes the **code** the
    discriminator a consumer matches on, not the status, which is why every
    refusal below is asserted by code.
    """
    found = set()

    def walk(node):
        if isinstance(node, dict):
            for key in ("reason", "type", "code"):
                value = node.get(key)
                if isinstance(value, str) and value.isupper() and len(value) > 3:
                    found.add(value)
            for value in node.values():
                walk(value)
        elif isinstance(node, list):
            for item in node:
                walk(item)

    walk(payload)
    return found


def _create_plan(client, headers, key, tier="gold"):
    return client.post(
        "/bss-pricing/v1/plans",
        headers={**headers, "Idempotency-Key": key},
        json={"plan_tier": tier, "billing_cycle": "recurring"},
    )


# ---------------------------------------------------------------------------
# Mounted, and the frontier's empty reading on the wire.
# ---------------------------------------------------------------------------


def test_the_frontier_reads_empty_and_says_so_explicitly(client, auth_headers, api_base):
    """200 with ``pin_eligible: false`` and its two explicit nulls.

    The 404-vs-200 discrimination, through the whole middleware stack. A consumer
    must be able to tell "no publish has ever completed" from "the frontier
    stands at version 0", and this gear's 404 deliberately conflates absent with
    out-of-scope — so the empty reading has to be a 200 carrying its own
    discriminator.
    """
    response = client.get(f"{api_base}/catalog-version/frontier", headers=auth_headers)

    assert response.status_code == 200, response.text
    body = response.json()
    assert body["pin_eligible"] is False
    assert body["catalog_version"] is None
    assert body["advanced_at"] is None


@pytest.mark.parametrize(
    "method,path",
    [
        ("GET", "/bss-pricing/v1/catalog-version/frontier"),
        ("POST", "/bss-pricing/v1/plans"),
    ],
)
def test_an_unauthenticated_request_is_401_on_a_read_and_on_a_write(client, method, path):
    response = client.request(method, path, json={})

    assert response.status_code == 401, response.text


def test_the_nil_tenant_token_is_refused_before_it_reaches_the_pdp(
    client, auth_headers_nil_tenant, api_base
):
    """401, not 403 — and the difference is a property of this gear.

    ``static-authz`` denies the nil-UUID tenant, so on most gears this token is
    the standard way to prove the PEP is in the deployed request path. Here it
    cannot be: ``api::rest::auth_context::require_authenticated`` refuses a
    context whose ``subject_tenant_id`` is nil **before** the gate, because every
    surface of this gear is tenant-scoped and an all-zero tenant is not an
    identity the catalog can serve. So the only token this deployment has that
    the PDP would deny never reaches the PDP.

    That the PEP really is in the path is established instead by
    ``test_tenant_b_cannot_see_tenant_as_plan_…``: tenant B's 404 for a plan
    tenant A can read is only possible if the PDP-compiled ``owner_tenant_id``
    constraint became the SQL filter.
    """
    response = client.get(
        f"{api_base}/catalog-version/frontier", headers=auth_headers_nil_tenant
    )

    assert response.status_code == 401, response.text


# ---------------------------------------------------------------------------
# The authoring round-trip, as one narrative.
# ---------------------------------------------------------------------------


def test_the_authoring_round_trip_holds_on_the_wire(client, auth_headers, idempotency_key):
    """create → read → stale patch → patch → add a price → list → delete → abandon.

    One test rather than eight, because the value is the **sequence**: every step
    consumes a header the previous one emitted, and a broken `ETag` or `Location`
    shows up as the next step failing rather than as an assertion about a string.
    """
    created = _create_plan(client, auth_headers, idempotency_key)
    assert created.status_code == 201, created.text
    plan = created.json()
    plan_id = plan["plan_id"]
    assert created.headers["location"] == f"/bss-pricing/v1/plans/{plan_id}"
    assert created.headers["etag"] == '"0-0"'
    assert plan["lifecycle_state"] == "draft"
    assert plan["revision"] == 0

    # Read it back: the same tag, and the three child sets a PATCH can replace.
    read = client.get(f"/bss-pricing/v1/plans/{plan_id}", headers=auth_headers)
    assert read.status_code == 200, read.text
    assert read.headers["etag"] == '"0-0"'
    assert read.json()["phases"] == []
    assert read.json()["descriptor_set"] is None

    # A stale precondition is refused by its code, and the row does not move.
    # The tag names its revision as well as its version: `/plans/{planId}` names
    # no revision, so a version alone would be applied to whichever revision the
    # PATCH resolves.
    stale = client.patch(
        f"/bss-pricing/v1/plans/{plan_id}",
        headers={**auth_headers, "If-Match": '"0-9"'},
        json={"shape": {"plan_tier": "platinum"}},
    )
    assert stale.status_code == 409, stale.text
    assert "STALE_VERSION" in _problem_codes(stale.json())

    # The right one advances the tag.
    patched = client.patch(
        f"/bss-pricing/v1/plans/{plan_id}",
        headers={**auth_headers, "If-Match": '"0-0"'},
        json={"shape": {"plan_tier": "platinum"}},
    )
    assert patched.status_code == 200, patched.text
    assert patched.headers["etag"] == '"0-1"'
    assert patched.json()["plan_tier"] == "platinum"

    # A phase, so the price row below has a `phase` axis to sit on.
    phase_id = str(uuid.uuid4())
    phased = client.patch(
        f"/bss-pricing/v1/plans/{plan_id}",
        headers={**auth_headers, "If-Match": '"0-1"'},
        json={
            "phases": [
                {
                    "phase_id": phase_id,
                    "kind": "evergreen",
                    "ordinal": 0,
                    "converts_to_phase_id": None,
                    "phase_duration_days": None,
                    "display_trial_days": None,
                }
            ]
        },
    )
    assert phased.status_code == 200, phased.text
    plan_tag = phased.headers["etag"]

    # A price row on the plan.
    price = client.post(
        f"/bss-pricing/v1/plans/{plan_id}/prices",
        headers={**auth_headers, "Idempotency-Key": str(uuid.uuid4())},
        json={
            "scope_key": {
                "currency": "USD",
                "region": "EU",
                "phase": phase_id,
                "price_eligibility": "all_subscriptions",
                "charge_kind": "recurring",
                "cohort": None,
            },
            "content": {"model_kind": "flat", "amount_minor": 1500},
        },
    )
    assert price.status_code == 201, price.text
    price_id = price.json()["price_id"]
    assert (
        price.headers["location"]
        == f"/bss-pricing/v1/plans/{plan_id}/prices/{price_id}"
    )
    assert price.headers["etag"] == '"0"'

    # The list, in the platform's `Page` envelope.
    listed = client.get(
        f"/bss-pricing/v1/plans/{plan_id}/prices", headers=auth_headers
    )
    assert listed.status_code == 200, listed.text
    page = listed.json()
    assert [row["price_id"] for row in page["items"]] == [price_id]
    assert page["page_info"]["next_cursor"] is None
    assert page["page_info"]["prev_cursor"] is None
    assert page["page_info"]["limit"] == 100

    # Delete under its own tag - the price row's, never the plan's (D-141).
    deleted = client.delete(
        f"/bss-pricing/v1/plans/{plan_id}/prices/{price_id}",
        headers={**auth_headers, "If-Match": '"0"'},
    )
    assert deleted.status_code == 204, deleted.text

    # Abandon the plan's draft: it flips, it is never deleted.
    abandoned = client.post(
        f"/bss-pricing/v1/plans/{plan_id}/abandon",
        headers={**auth_headers, "If-Match": plan_tag},
    )
    assert abandoned.status_code == 200, abandoned.text
    assert abandoned.json()["lifecycle_state"] == "abandoned"


def test_a_blind_delete_is_refused_and_the_row_survives(client, auth_headers):
    """D-141's defect, on the wire.

    Before it, this verb's idempotency cell was empty and a draft row could be
    destroyed under an unknown version. What a blind delete destroys is a
    concurrent editor's uncommitted work, not the row.
    """
    plan = _create_plan(client, auth_headers, str(uuid.uuid4())).json()
    plan_id = plan["plan_id"]
    phase_id = str(uuid.uuid4())
    client.patch(
        f"/bss-pricing/v1/plans/{plan_id}",
        headers={**auth_headers, "If-Match": '"0-0"'},
        json={
            "phases": [
                {
                    "phase_id": phase_id,
                    "kind": "evergreen",
                    "ordinal": 0,
                    "converts_to_phase_id": None,
                    "phase_duration_days": None,
                    "display_trial_days": None,
                }
            ]
        },
    )
    price = client.post(
        f"/bss-pricing/v1/plans/{plan_id}/prices",
        headers={**auth_headers, "Idempotency-Key": str(uuid.uuid4())},
        json={
            "scope_key": {
                "currency": "USD",
                "region": "EU",
                "phase": phase_id,
                "price_eligibility": "all_subscriptions",
                "charge_kind": "recurring",
                "cohort": None,
            },
            "content": {"model_kind": "flat", "amount_minor": 900},
        },
    ).json()

    blind = client.delete(
        f"/bss-pricing/v1/plans/{plan_id}/prices/{price['price_id']}",
        headers=auth_headers,
    )
    assert blind.status_code == 400, blind.text

    survivor = client.get(
        f"/bss-pricing/v1/plans/{plan_id}/prices", headers=auth_headers
    )
    assert [row["price_id"] for row in survivor.json()["items"]] == [price["price_id"]]


# ---------------------------------------------------------------------------
# Idempotency: a header contract, so only provable end to end.
# ---------------------------------------------------------------------------


def test_the_same_key_and_body_replays_the_same_plan_id(
    client, auth_headers, idempotency_key
):
    """The property that is *only* provable over HTTP, because it is a header
    contract: the caller sends one header twice and must be handed the same id."""
    first = _create_plan(client, auth_headers, idempotency_key)
    assert first.status_code == 201, first.text

    replay = _create_plan(client, auth_headers, idempotency_key)

    assert replay.status_code == 201, replay.text
    assert replay.json()["plan_id"] == first.json()["plan_id"]


def test_the_same_key_with_a_different_body_is_refused_by_its_code(
    client, auth_headers, idempotency_key
):
    _create_plan(client, auth_headers, idempotency_key, tier="gold")

    clash = _create_plan(client, auth_headers, idempotency_key, tier="platinum")

    assert clash.status_code == 409, clash.text
    assert "IDEMPOTENCY_PAYLOAD_MISMATCH" in _problem_codes(clash.json())


def test_a_create_without_the_header_is_refused(client, auth_headers):
    response = client.post(
        "/bss-pricing/v1/plans", headers=auth_headers, json={"plan_tier": "gold"}
    )

    assert response.status_code == 400, response.text


# ---------------------------------------------------------------------------
# Cross-tenant isolation, genuinely.
# ---------------------------------------------------------------------------


def test_tenant_b_cannot_see_tenant_as_plan_and_gets_the_same_answer_as_for_a_random_id(
    client, auth_headers, auth_headers_tenant_b, idempotency_key
):
    """The owner's 200 is the baseline.

    Without it the 404 below would be consistent with the plan simply not
    existing, and the test would prove nothing about isolation.
    """
    created = _create_plan(client, auth_headers, idempotency_key)
    assert created.status_code == 201, created.text
    plan_id = created.json()["plan_id"]

    owner = client.get(f"/bss-pricing/v1/plans/{plan_id}", headers=auth_headers)
    assert owner.status_code == 200, owner.text
    assert owner.json()["plan_id"] == plan_id

    foreign = client.get(
        f"/bss-pricing/v1/plans/{plan_id}", headers=auth_headers_tenant_b
    )
    absent = client.get(
        f"/bss-pricing/v1/plans/{uuid.uuid4()}", headers=auth_headers_tenant_b
    )

    assert foreign.status_code == 404, foreign.text
    assert absent.status_code == 404, absent.text
    assert _problem_codes(foreign.json()) == _problem_codes(absent.json())


# ---------------------------------------------------------------------------
# The status-rendering rule, where a consumer would discover it.
# ---------------------------------------------------------------------------


def test_a_design_set_422_arrives_as_a_400_carrying_its_code(
    client, auth_headers, idempotency_key
):
    """§3.3 states it once for the whole design set; this is where a consumer
    would find out.

    ``LIFECYCLE_FORBIDDEN`` is annotated 422 in the documents. The platform's
    canonical family has no 422 category at all, so it reaches the wire as a
    **400 carrying its code** — and the code, not the status, is what a consumer
    matches on. A plan holding no open draft revision is the cheapest way to
    provoke it: abandon it once, then abandon it again.
    """
    plan_id = _create_plan(client, auth_headers, idempotency_key).json()["plan_id"]
    first = client.post(
        f"/bss-pricing/v1/plans/{plan_id}/abandon",
        headers={**auth_headers, "If-Match": '"0-0"'},
    )
    assert first.status_code == 200, first.text

    again = client.post(
        f"/bss-pricing/v1/plans/{plan_id}/abandon",
        headers={**auth_headers, "If-Match": '"0-1"'},
    )

    assert again.status_code == 400, again.text
    assert "PLAN_ABANDONED_NO_SUCCESSOR" in _problem_codes(again.json()), again.text
