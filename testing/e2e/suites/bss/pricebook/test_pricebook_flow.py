"""The PriceBook flow over HTTP, across bss-products and bss-pricing.

Products publishes a SKU; pricing prices it in a EUR book through the reference
reservation (reserve, write, confirm in products' in-process registry), approves
one draft price with quorum 0 and serves it in the book export; products then
refuses to retire the SKU while pricing's reference lives.

Two variants. ``usage`` is the metered SKU the programme names: it needs a
usage-type catalog, and products' catalog is the usage collector it links, which
answers only with a storage plugin. On a binary without one it skips with the
collector's own answer. ``recurring`` needs no catalog and always runs the flow.
"""

import datetime
import uuid

import pytest

from .conftest import PRICING, PRODUCTS, USAGE_TYPE

USAGE_TYPES = "/usage-collector/v1/usage-types"


def _key() -> dict:
    return {"Idempotency-Key": str(uuid.uuid4())}


def _usage_type_or_skip(api) -> None:
    r = api.post(
        USAGE_TYPES,
        json={"gts_id": USAGE_TYPE, "kind": "counter", "metadata_fields": []},
    )
    if r.status_code not in (201, 409):
        pytest.skip(
            "no usage-type catalog on this binary, so no usage SKU can publish: "
            f"POST {USAGE_TYPES} answered {r.status_code}: {r.text}"
        )


VARIANTS = {
    "usage": {
        "sku": {"type": "usage", "usage_type_ref": USAGE_TYPE, "unit": "GB"},
        "entry": {},
        "price": {"model": "per_unit", "price": {"rate": "0.10"}},
    },
    "recurring": {
        "sku": {"type": "recurring"},
        "entry": {"period": "month"},
        "price": {"model": "flat", "price": {"amount": "30.00"}},
    },
}


@pytest.mark.timeout(120)
@pytest.mark.parametrize("variant", ["usage", "recurring"])
def test_a_priced_sku_publishes_its_price_and_blocks_retirement(api, variant):
    shape = VARIANTS[variant]
    if variant == "usage":
        _usage_type_or_skip(api)
    run = uuid.uuid4().hex[:8]

    # Products: quorum 0, a category and a SKU, published at submit.
    r = api.put(f"{PRODUCTS}/approval-policy", json={"quorum": 0})
    assert r.status_code == 200, r.text
    r = api.post(
        f"{PRODUCTS}/categories",
        json={"code": f"e2e-{variant}-{run}", "name": f"E2E {variant} {run}"},
    )
    assert r.status_code == 201, r.text
    category = r.json()["id"]
    r = api.post(
        f"{PRODUCTS}/skus",
        json={
            "code": f"E2E-{variant.upper()}-{run}",
            "name": f"E2E {variant} {run}",
            "category_id": category,
            **shape["sku"],
        },
    )
    assert r.status_code == 201, r.text
    sku = r.json()["id"]
    r = api.post(f"{PRODUCTS}/skus/{sku}/submit", json={})
    assert r.status_code == 200, r.text
    assert r.json()["applied"] is True, r.text

    # Pricing: a EUR book and quorum 0 under the policy's own ETag.
    r = api.post(
        f"{PRICING}/price-books",
        json={"code": f"eur-{variant}-{run}", "name": f"EUR {run}", "currency": "EUR"},
        headers=_key(),
    )
    assert r.status_code == 201, r.text
    book = r.json()["id"]
    r = api.get(f"{PRICING}/approval-policy")
    assert r.status_code == 200, r.text
    r = api.put(
        f"{PRICING}/approval-policy",
        json={"quorum": 0},
        headers={"If-Match": r.headers["etag"]},
    )
    assert r.status_code == 200, r.text

    # An entry for the SKU: reserved, written and confirmed in one call; the key replays.
    key = _key()
    body = {"sku_id": sku, **shape["entry"]}
    r = api.post(f"{PRICING}/price-books/{book}/entries", json=body, headers=key)
    assert r.status_code == 201, r.text
    entry = r.json()
    assert entry["sku_id"] == sku
    assert entry["charge_kind"] == variant
    assert entry["reference_state"] == "confirmed", entry
    replay = api.post(f"{PRICING}/price-books/{book}/entries", json=body, headers=key)
    assert replay.status_code == 201, replay.text
    assert replay.json() == entry, "the same Idempotency-Key replays the receipt"

    # One draft price, published with quorum 0.
    start = (datetime.date.today() + datetime.timedelta(days=30)).isoformat()
    r = api.post(
        f"{PRICING}/price-book-entries/{entry['id']}/prices",
        json={**shape["price"], "eligibility": "all", "effective_from": start},
        headers=_key(),
    )
    assert r.status_code == 201, r.text
    price = r.json()["items"][0]
    assert price["state"] == "draft", price
    r = api.post(f"{PRICING}/price-books/{book}/publish-changes", json={}, headers=_key())
    assert r.status_code == 201, r.text
    receipt = r.json()
    assert receipt["applied"] is True, receipt
    assert receipt["unit"]["state"] == "approved", receipt

    # The export serves the approved price.
    r = api.get(f"{PRICING}/price-books/{book}/export")
    assert r.status_code == 200, r.text
    export = r.json()
    assert export["book"]["id"] == book
    assert [e["entry"]["id"] for e in export["entries"]] == [entry["id"]]
    prices = export["entries"][0]["prices"]
    assert [(x["id"], x["state"], x["effective_from"], x["price_json"]) for x in prices] == [
        (price["id"], "approved", start, shape["price"]["price"])
    ], prices

    # Products refuses to retire a SKU a live entry references.
    r = api.post(f"{PRODUCTS}/skus/{sku}/retire", json={})
    assert r.status_code == 409, r.text
    assert "SKU_REFERENCED" in r.text, r.text
    r = api.get(f"{PRODUCTS}/skus/{sku}")
    assert r.status_code == 200, r.text
    assert r.json()["sku"]["lifecycle"] == "published", r.text


def _policy(api) -> tuple[dict, str]:
    r = api.get(f"{PRICING}/approval-policy")
    assert r.status_code == 200, r.text
    return r.json(), r.headers["etag"]


def _set_quorum(api, kind: str, quorum: int) -> None:
    _, tag = _policy(api)
    r = api.put(
        f"{PRICING}/approval-policy",
        json={"kind": kind, "quorum": quorum},
        headers={"If-Match": tag},
    )
    assert r.status_code == 200, r.text


def _checks(api, revision: str) -> dict:
    r = api.get(f"{PRICING}/plan-revisions/{revision}/checks")
    assert r.status_code == 200, r.text
    return r.json()


def _check(checks: dict, code: str) -> dict:
    return next(c for c in checks["checks"] if c["code"] == code)


def _revision(api, revision: str) -> dict:
    r = api.get(f"{PRICING}/plan-revisions/{revision}")
    assert r.status_code == 200, r.text
    return r.json()


@pytest.mark.timeout(120)
def test_a_plan_blocked_by_a_pending_price_publishes_copies_and_clones(api, reviewer):
    """Spec §8's worked example across the two gears, then the revision's life.

    A price waits in a ``prices`` unit (quorum 1): a plan on the EUR book naming its entry is
    red, ``ITEM_UNCOVERED`` naming that unit; the reviewer approves it and the checks turn green;
    the revision publishes at once (``plan_revision`` quorum 0); a copy is rev 2, its item
    attached, and publishing it supersedes rev 1; a clone is a new draft on the same book; the
    entry a plan names cannot be deleted. The pricing policy is restored at the end.
    """
    run = uuid.uuid4().hex[:8]
    before, _ = _policy(api)
    found = {
        kind: before["overrides"].get(kind, before["default_quorum"])
        for kind in ("prices", "plan_revision")
    }
    try:
        # Products: a published recurring SKU.
        r = api.put(f"{PRODUCTS}/approval-policy", json={"quorum": 0})
        assert r.status_code == 200, r.text
        r = api.post(
            f"{PRODUCTS}/categories",
            json={"code": f"e2e-plan-{run}", "name": f"E2E plan {run}"},
        )
        assert r.status_code == 201, r.text
        r = api.post(
            f"{PRODUCTS}/skus",
            json={
                "code": f"E2E-PLAN-{run}",
                "name": f"E2E plan {run}",
                "category_id": r.json()["id"],
                "type": "recurring",
            },
        )
        assert r.status_code == 201, r.text
        sku = r.json()["id"]
        r = api.post(f"{PRODUCTS}/skus/{sku}/submit", json={})
        assert r.status_code == 200, r.text
        assert r.json()["applied"] is True, r.text

        # Pricing: a EUR book with a monthly entry for the SKU.
        r = api.post(
            f"{PRICING}/price-books",
            json={"code": f"eur-plan-{run}", "name": f"EUR plan {run}", "currency": "EUR"},
            headers=_key(),
        )
        assert r.status_code == 201, r.text
        book = r.json()["id"]
        r = api.post(
            f"{PRICING}/price-books/{book}/entries",
            json={"sku_id": sku, "period": "month"},
            headers=_key(),
        )
        assert r.status_code == 201, r.text
        entry = r.json()["id"]

        # A draft price, submitted into a prices unit that waits for one reviewer.
        _set_quorum(api, "prices", 1)
        _set_quorum(api, "plan_revision", 0)
        start = (datetime.date.today() + datetime.timedelta(days=30)).isoformat()
        r = api.post(
            f"{PRICING}/price-book-entries/{entry}/prices",
            json={
                "model": "flat",
                "price": {"amount": "30.00"},
                "eligibility": "all",
                "effective_from": start,
            },
            headers=_key(),
        )
        assert r.status_code == 201, r.text
        price = r.json()["items"][0]["id"]
        r = api.post(f"{PRICING}/prices/{price}/submit", json={}, headers=_key())
        assert r.status_code == 201, r.text
        assert r.json()["applied"] is False, r.text
        unit = r.json()["unit"]["id"]

        # A plan on the book, sold from the price's start, with the entry as a paid item: red.
        r = api.post(
            f"{PRICING}/plans",
            json={"code": f"plan-{run}", "name": f"Plan {run}", "book_id": book},
            headers=_key(),
        )
        assert r.status_code == 201, r.text
        plan = r.json()["id"]
        rev1 = r.json()["revisions"][0]["id"]
        r = api.get(f"{PRICING}/plan-revisions/{rev1}")
        assert r.status_code == 200, r.text
        r = api.patch(
            f"{PRICING}/plan-revisions/{rev1}",
            json={"available_from": start},
            headers={"If-Match": r.headers["etag"]},
        )
        assert r.status_code == 200, r.text
        r = api.post(
            f"{PRICING}/plan-revisions/{rev1}/items",
            json={"sku_id": sku, "price_book_entry_id": entry, "treatment": "paid"},
            headers=_key(),
        )
        assert r.status_code == 201, r.text
        assert r.json()["reference_state"] == "confirmed", r.text
        checks = _checks(api, rev1)
        assert checks["ready"] is False, checks
        assert checks["sale_date"] == start, checks
        uncovered = _check(checks, "ITEM_UNCOVERED")
        assert uncovered["ok"] is False, checks
        assert uncovered["blocked_by"] == [unit], checks

        # The reviewer approves the price unit: the checks turn green.
        r = reviewer.post(
            f"{PRICING}/approval-units/{unit}/approve",
            json={"generation": 1},
            headers=_key(),
        )
        assert r.status_code == 200, r.text
        assert r.json()["outcome"] == "applied", r.text
        checks = _checks(api, rev1)
        assert checks["ready"] is True, checks
        assert _check(checks, "ITEM_UNCOVERED")["blocked_by"] == [], checks

        # Quorum 0 for plan_revision: the submit publishes rev 1 at once.
        r = api.post(f"{PRICING}/plan-revisions/{rev1}/submit", json={}, headers=_key())
        assert r.status_code == 201, r.text
        assert r.json()["applied"] is True, r.text
        assert r.json()["revision"]["state"] == "published", r.text
        r = api.get(f"{PRICING}/plans/{plan}")
        assert r.status_code == 200, r.text
        assert r.json()["published_rev"] == 1, r.text

        # A copy is rev 2 whose item attaches; publishing it supersedes rev 1.
        r = api.post(f"{PRICING}/plans/{plan}/revisions", json={}, headers=_key())
        assert r.status_code == 201, r.text
        assert r.json()["rev_no"] == 2, r.text
        rev2 = r.json()["id"]
        copied = _revision(api, rev2)["items"]
        assert [(i["sku_id"], i["reference_state"]) for i in copied] == [
            (sku, "confirmed")
        ], copied
        r = api.post(f"{PRICING}/plan-revisions/{rev2}/submit", json={}, headers=_key())
        assert r.status_code == 201, r.text
        assert r.json()["revision"]["state"] == "published", r.text
        assert _revision(api, rev1)["state"] == "superseded"
        r = api.get(f"{PRICING}/plans/{plan}")
        assert r.json()["published_rev"] == 2, r.text

        # A clone is a new plan whose draft rev 1 reads the same book.
        r = api.post(
            f"{PRICING}/plans/{plan}/clone",
            json={"code": f"plan-{run}-clone", "name": f"Plan {run} clone"},
            headers=_key(),
        )
        assert r.status_code == 201, r.text
        clone = r.json()
        assert clone["id"] != plan, clone
        assert clone["published_rev"] is None, clone
        assert [x["state"] for x in clone["revisions"]] == ["draft"], clone
        draft = _revision(api, clone["revisions"][0]["id"])
        assert draft["book_id"] == book, draft
        assert [i["sku_id"] for i in draft["items"]] == [sku], draft

        # The entry a plan names cannot be deleted.
        r = api.delete(f"{PRICING}/price-book-entries/{entry}")
        assert r.status_code == 409, r.text
        assert "ENTRY_IN_USE" in r.text, r.text
    finally:
        for kind, quorum in found.items():
            _set_quorum(api, kind, quorum)
