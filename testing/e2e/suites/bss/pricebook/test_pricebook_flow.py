"""The PriceBook flow over HTTP, across bss-products and bss-pricing.

Products publishes a SKU; pricing prices it in a EUR book through the reference
reservation (reserve, write, confirm in products' in-process registry), approves
one draft row with quorum 0 and serves it in the book export; products then
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
        "price": {},
        "row": {"model": "per_unit", "price": {"rate": "0.10"}},
    },
    "recurring": {
        "sku": {"type": "recurring"},
        "price": {"period": "month"},
        "row": {"model": "flat", "price": {"amount": "30.00"}},
    },
}


@pytest.mark.timeout(120)
@pytest.mark.parametrize("variant", ["usage", "recurring"])
def test_a_priced_sku_publishes_its_row_and_blocks_retirement(api, variant):
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

    # A price for the SKU: reserved, written and confirmed in one call; the key replays.
    key = _key()
    body = {"sku_id": sku, **shape["price"]}
    r = api.post(f"{PRICING}/price-books/{book}/prices", json=body, headers=key)
    assert r.status_code == 201, r.text
    price = r.json()
    assert price["sku_id"] == sku
    assert price["charge_kind"] == variant
    assert price["reference_state"] == "confirmed", price
    replay = api.post(f"{PRICING}/price-books/{book}/prices", json=body, headers=key)
    assert replay.status_code == 201, replay.text
    assert replay.json() == price, "the same Idempotency-Key replays the receipt"

    # One draft row, published with quorum 0.
    start = (datetime.date.today() + datetime.timedelta(days=30)).isoformat()
    r = api.post(
        f"{PRICING}/prices/{price['id']}/rows",
        json={**shape["row"], "eligibility": "all", "effective_from": start},
        headers=_key(),
    )
    assert r.status_code == 201, r.text
    row = r.json()["items"][0]
    assert row["state"] == "draft", row
    r = api.post(f"{PRICING}/price-books/{book}/publish-changes", json={}, headers=_key())
    assert r.status_code == 201, r.text
    receipt = r.json()
    assert receipt["applied"] is True, receipt
    assert receipt["unit"]["state"] == "approved", receipt

    # The export serves the approved row.
    r = api.get(f"{PRICING}/price-books/{book}/export")
    assert r.status_code == 200, r.text
    export = r.json()
    assert export["book"]["id"] == book
    assert [p["price"]["id"] for p in export["prices"]] == [price["id"]]
    rows = export["prices"][0]["rows"]
    assert [(x["id"], x["state"], x["effective_from"], x["price_json"]) for x in rows] == [
        (row["id"], "approved", start, shape["row"]["price"])
    ], rows

    # Products refuses to retire a SKU a live price references.
    r = api.post(f"{PRODUCTS}/skus/{sku}/retire", json={})
    assert r.status_code == 409, r.text
    assert "SKU_REFERENCED" in r.text, r.text
    r = api.get(f"{PRODUCTS}/skus/{sku}")
    assert r.status_code == 200, r.text
    assert r.json()["sku"]["lifecycle"] == "published", r.text
