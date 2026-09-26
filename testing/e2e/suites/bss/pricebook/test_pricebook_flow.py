"""The PriceBook flow over HTTP, across bss-products and bss-pricing.

Products publishes a SKU; pricing prices it in a EUR book through the reference
reservation (reserve, write, confirm in products' in-process registry), approves
one draft price with quorum 0 and serves it in the book export; products then
refuses to retire the SKU while pricing's reference lives.

Two variants. ``usage`` is the metered SKU the programme names: it needs a
usage-type catalog, and products' catalog is the usage collector it links, which
answers only with a storage plugin. On a binary without one it skips with the
collector's own answer. ``recurring`` needs no catalog and always runs the flow.

Phase 5 on the same wire: the model is the entry's (D-427), so an entry is created with
``model`` and a price with money only; the entry reads carry ``usage`` (D-428); the SKU card and
the SKU list carry pricing's ``usage`` through the port pricing registers in the ClientHub
(P-D-197) — these suites are its only proof on the real binary; and a SKU may have no category
(P-D-196).
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


# D-427: the model is the entry's; a price carries only its money, in that model.
VARIANTS = {
    "usage": {
        "sku": {"type": "usage", "usage_type_ref": USAGE_TYPE, "unit": "GB"},
        "entry": {"model": "per_unit"},
        "price": {"price": {"rate": "0.10"}},
    },
    "recurring": {
        "sku": {"type": "recurring"},
        "entry": {"period": "month", "model": "flat"},
        "price": {"price": {"amount": "30.00"}},
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

    # D-428: the entry read counts its approved price, and no plan names it.
    assert _usage(api, entry["id"]) == _entry_usage(approved=1), entry

    # Products refuses to retire a SKU a live entry references.
    r = api.post(f"{PRODUCTS}/skus/{sku}/retire", json={})
    assert r.status_code == 409, r.text
    assert "SKU_REFERENCED" in r.text, r.text
    r = api.get(f"{PRODUCTS}/skus/{sku}")
    assert r.status_code == 200, r.text
    assert r.json()["sku"]["lifecycle"] == "published", r.text
    # P-D-197: the card carries pricing's usage through the port pricing registers.
    assert r.json()["usage"] == _sku_usage(1, ["EUR"], approved=1), r.text


def _entry_usage(
    approved: int = 0,
    pending: int = 0,
    draft: int = 0,
    plans: int = 0,
    superseded_only: int = 0,
) -> dict:
    """An entry read's ``usage`` (D-428)."""
    return {
        "prices": {"approved": approved, "pending": pending, "draft": draft},
        "plans": plans,
        "plans_superseded_only": superseded_only,
    }


def _usage(api, entry: str) -> dict:
    """``GET /price-book-entries/{id}``'s ``usage``."""
    r = api.get(f"{PRICING}/price-book-entries/{entry}")
    assert r.status_code == 200, r.text
    return r.json()["usage"]


def _sku_usage(
    entries: int,
    currencies: list[str],
    approved: int = 0,
    pending: int = 0,
    draft: int = 0,
    plans: int = 0,
) -> dict:
    """A SKU read's ``usage`` (P-D-197), as pricing's port fills it."""
    return {
        "entries": entries,
        "currencies": currencies,
        "prices": {"approved": approved, "pending": pending, "draft": draft},
        "plans": plans,
    }


def _sku_reads(api, sku: str, code: str) -> tuple[dict, dict]:
    """The SKU card and the SKU's one row of the list (``q`` = its unique code)."""
    r = api.get(f"{PRODUCTS}/skus/{sku}")
    assert r.status_code == 200, r.text
    card = r.json()
    r = api.get(f"{PRODUCTS}/skus", params={"q": code})
    assert r.status_code == 200, r.text
    rows = [row for row in r.json()["items"] if row["id"] == sku]
    assert len(rows) == 1, r.text
    return card, rows[0]


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


def _resolve(api, revision: str, date: str, pins: str | None = None) -> dict:
    params = {"plan_revision_id": revision, "date": date}
    if pins is not None:
        params["pins"] = pins
    r = api.get(f"{PRICING}/resolve", params=params)
    assert r.status_code == 200, r.text
    return r.json()


def _binding(resolved: dict) -> dict:
    """The default chain's binding of the one item."""
    [item] = resolved["items"]
    chain = item["chains"][0]
    assert chain["dim_value"] is None, resolved
    assert chain["uncovered"] is False, resolved
    return chain["binding"]


@pytest.mark.timeout(120)
def test_a_plan_blocked_by_a_pending_price_publishes_copies_and_clones(api, reviewer):
    """Spec §8's worked example across the two gears, then the revision's life.

    A price waits in a ``prices`` unit (quorum 1): a plan on the EUR book naming its entry is
    red, ``ITEM_UNCOVERED`` naming that unit; the reviewer approves it and the checks turn green;
    the revision publishes at once (``plan_revision`` quorum 0) and the consumer read contract
    serves it: ``GET /resolve`` binds the approved price, with the SKU version products holds on
    the date (read through the real in-process registry), and ``GET /prices/{id}`` serves that
    price; a copy is rev 2, its item attached, and publishing it supersedes rev 1, which still
    resolves, and a renewal pinned to the price binds it again; a clone is a new draft on the
    same book; the entry a plan names cannot be deleted. The pricing policy is restored at the
    end.

    Along the way the entry read's ``usage`` (D-428) follows the price through draft, pending
    and approved, and counts the plan once while its draft, published and copied revisions name
    the entry (the superseded rev 1 is history); the clone is a second plan. The SKU card and
    the SKU list carry the same counts through the port pricing fills (P-D-197).
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
            json={"sku_id": sku, "period": "month", "model": "flat"},
            headers=_key(),
        )
        assert r.status_code == 201, r.text
        entry = r.json()["id"]
        # D-427, D-428: the entry read carries its model and a usage of zeros.
        r = api.get(f"{PRICING}/price-book-entries/{entry}")
        assert r.status_code == 200, r.text
        assert r.json()["model"] == "flat", r.text
        assert r.json()["usage"] == _entry_usage(), r.text

        # A draft price, submitted into a prices unit that waits for one reviewer.
        _set_quorum(api, "prices", 1)
        _set_quorum(api, "plan_revision", 0)
        start = (datetime.date.today() + datetime.timedelta(days=30)).isoformat()
        r = api.post(
            f"{PRICING}/price-book-entries/{entry}/prices",
            json={
                "price": {"amount": "30.00"},
                "eligibility": "all",
                "effective_from": start,
            },
            headers=_key(),
        )
        assert r.status_code == 201, r.text
        price = r.json()["items"][0]["id"]
        assert _usage(api, entry) == _entry_usage(draft=1)
        r = api.post(f"{PRICING}/prices/{price}/submit", json={}, headers=_key())
        assert r.status_code == 201, r.text
        assert r.json()["applied"] is False, r.text
        unit = r.json()["unit"]["id"]
        assert _usage(api, entry) == _entry_usage(pending=1)

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
        # A draft revision naming the entry counts its plan.
        assert _usage(api, entry) == _entry_usage(pending=1, plans=1)
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
        assert _usage(api, entry) == _entry_usage(approved=1, plans=1)

        # Quorum 0 for plan_revision: the submit publishes rev 1 at once.
        r = api.post(f"{PRICING}/plan-revisions/{rev1}/submit", json={}, headers=_key())
        assert r.status_code == 201, r.text
        assert r.json()["applied"] is True, r.text
        assert r.json()["revision"]["state"] == "published", r.text
        r = api.get(f"{PRICING}/plans/{plan}")
        assert r.status_code == 200, r.text
        assert r.json()["published_rev"] == 1, r.text

        # The read contract: a signup on the sale date binds the approved price.
        resolved = _resolve(api, rev1, start)
        assert resolved["state"] == "published", resolved
        assert (resolved["plan_id"], resolved["rev_no"], resolved["book_id"]) == (plan, 1, book)
        assert (resolved["currency"], resolved["currency_minor_digits"]) == ("EUR", 2), resolved
        [item] = resolved["items"]
        assert (item["sku_id"], item["price_book_entry_id"]) == (sku, entry), item
        assert (item["charge_kind"], item["period"], item["treatment"]) == (
            "recurring",
            "month",
            "paid",
        ), item
        version = item["sku_version"]
        assert version is not None, "products answered the dated read: " + str(item)
        assert version["published_version"] >= 1, item
        assert version["effective_from"] <= start, item
        binding = _binding(resolved)
        assert binding["price_id"] == price, binding
        assert binding["dim_used"] is None, binding
        assert binding["pinned_from"] is None, binding
        # D-427: the model is the item's (its entry's); the binding carries the money.
        assert (item["model"], binding["price"]) == ("flat", {"amount": "30.00"}), binding
        assert (binding["eligibility"], binding["effective_from"]) == ("all", start), binding

        # The pinned price read serves that price, as stored.
        r = api.get(f"{PRICING}/prices/{price}")
        assert r.status_code == 200, r.text
        pinned = r.json()
        assert pinned["price_id"] == price, pinned
        assert (pinned["sku_id"], pinned["price_book_entry_id"], pinned["book_id"]) == (
            sku,
            entry,
            book,
        ), pinned
        assert (pinned["charge_kind"], pinned["period"], pinned["currency"]) == (
            "recurring",
            "month",
            "EUR",
        ), pinned
        assert (pinned["price"], pinned["effective_from"]) == ({"amount": "30.00"}, start), pinned
        assert "status" not in pinned and "version" not in pinned, pinned

        # A copy is rev 2 whose item attaches; publishing it supersedes rev 1.
        r = api.post(f"{PRICING}/plans/{plan}/revisions", json={}, headers=_key())
        assert r.status_code == 201, r.text
        assert r.json()["rev_no"] == 2, r.text
        rev2 = r.json()["id"]
        copied = _revision(api, rev2)["items"]
        assert [(i["sku_id"], i["reference_state"]) for i in copied] == [
            (sku, "confirmed")
        ], copied
        # The published rev 1 and its draft copy name the entry: one plan.
        assert _usage(api, entry) == _entry_usage(approved=1, plans=1)
        r = api.post(f"{PRICING}/plan-revisions/{rev2}/submit", json={}, headers=_key())
        assert r.status_code == 201, r.text
        assert r.json()["revision"]["state"] == "published", r.text
        assert _revision(api, rev1)["state"] == "superseded"
        r = api.get(f"{PRICING}/plans/{plan}")
        assert r.json()["published_rev"] == 2, r.text
        # Rev 1 superseded is history; rev 2 published still names the entry: still one plan.
        assert _usage(api, entry) == _entry_usage(approved=1, plans=1)

        # The superseded rev 1 still resolves, and a renewal pinned to the price binds it again.
        superseded = _resolve(api, rev1, start)
        assert superseded["state"] == "superseded", superseded
        assert _binding(superseded)["price_id"] == price, superseded
        renewal = _binding(_resolve(api, rev2, start, pins=price))
        assert (renewal["price_id"], renewal["pinned_from"]) == (price, price), renewal

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

        # The clone's draft names the entry too: two plans, on the entry read and the book list.
        counted = _entry_usage(approved=1, plans=2)
        assert _usage(api, entry) == counted
        r = api.get(f"{PRICING}/price-books/{book}/entries")
        assert r.status_code == 200, r.text
        assert [(e["id"], e["model"], e["usage"]) for e in r.json()["items"]] == [
            (entry, "flat", counted)
        ], r.text

        # The SKU card and the SKU list carry the same counts from pricing's port.
        card, row = _sku_reads(api, sku, f"E2E-PLAN-{run}")
        expected = _sku_usage(1, ["EUR"], approved=1, plans=2)
        assert card["usage"] == expected, card
        assert row["usage"] == expected, row

        # The entry a plan names cannot be deleted.
        r = api.delete(f"{PRICING}/price-book-entries/{entry}")
        assert r.status_code == 409, r.text
        assert "ENTRY_IN_USE" in r.text, r.text
    finally:
        for kind, quorum in found.items():
            _set_quorum(api, kind, quorum)


def _book(api, currency: str, run: str) -> str:
    r = api.post(
        f"{PRICING}/price-books",
        json={
            "code": f"{currency.lower()}-nocat-{run}",
            "name": f"{currency} no category {run}",
            "currency": currency,
        },
        headers=_key(),
    )
    assert r.status_code == 201, r.text
    return r.json()["id"]


def _monthly_entry(api, book: str, sku: str, model: str):
    return api.post(
        f"{PRICING}/price-books/{book}/entries",
        json={"sku_id": sku, "period": "month", "model": model},
        headers=_key(),
    )


def _draft_price(api, entry: dict, money: dict, start: str) -> None:
    """A draft price carries only its money; its model is echoed from its entry (D-427)."""
    r = api.post(
        f"{PRICING}/price-book-entries/{entry['id']}/prices",
        json={"price": money, "eligibility": "all", "effective_from": start},
        headers=_key(),
    )
    assert r.status_code == 201, r.text
    price = r.json()["items"][0]
    assert (price["state"], price["model"], price["price_json"]) == (
        "draft",
        entry["model"],
        money,
    ), price


@pytest.mark.timeout(120)
def test_a_sku_without_a_category_is_priced_in_two_models_and_its_reads_carry_its_usage(api):
    """P-D-196, D-427 and D-428 / P-D-197 over HTTP, on one SKU.

    A SKU created without a category is 201 with ``category_id: null``, publishes and is priced.
    The entry door wants a model its charge kind allows, the entry PATCH does not take one, and
    a price carries none (money that does not fit its entry's model is ``PRICE_MISSING``).
    Its one SKU x charge kind x period takes two entries of different models in one EUR book
    (201 both; the same model again is 409 ``ENTRY_KEY_TAKEN``), and a third in a USD book. A
    plan publishes rev 1 with the flat entry, and its copy rev 2 re-points the item to the
    per-unit entry: the plan names both entries, each entry counts it and the SKU counts it
    once. Publishing rev 2 leaves the flat entry named only by the superseded rev 1
    (``plans_superseded_only``), and each revision resolves in its own entry's model. The SKU
    card and the SKU list carry the counts through the port pricing fills. The pricing policy
    is restored at the end.
    """
    run = uuid.uuid4().hex[:8]
    code = f"E2E-NOCAT-{run}"
    before, _ = _policy(api)
    found = {
        kind: before["overrides"].get(kind, before["default_quorum"])
        for kind in ("prices", "plan_revision")
    }
    try:
        # Products: a recurring SKU with no category, published at quorum 0.
        r = api.put(f"{PRODUCTS}/approval-policy", json={"quorum": 0})
        assert r.status_code == 200, r.text
        r = api.post(
            f"{PRODUCTS}/skus",
            json={"code": code, "name": f"E2E no category {run}", "type": "recurring"},
        )
        assert r.status_code == 201, r.text
        assert r.json()["category_id"] is None, r.text
        sku = r.json()["id"]
        r = api.post(f"{PRODUCTS}/skus/{sku}/submit", json={})
        assert r.status_code == 200, r.text
        assert r.json()["applied"] is True, r.text
        published = r.json()["sku"]
        assert (published["lifecycle"], published["category_id"]) == ("published", None), r.text
        # A category filter never matches a SKU without one, and such a SKU never keeps a
        # category in use; browse serves it.
        r = api.post(
            f"{PRODUCTS}/categories",
            json={"code": f"e2e-nocat-{run}", "name": f"E2E no category {run}"},
        )
        assert r.status_code == 201, r.text
        unused = r.json()["id"]
        r = api.get(f"{PRODUCTS}/skus", params={"q": code, "category": unused})
        assert r.status_code == 200, r.text
        assert r.json()["items"] == [], r.text
        r = api.post(f"{PRODUCTS}/categories/{unused}/retire", json={})
        assert r.status_code == 200, r.text
        assert r.json()["status"] == "retired", r.text
        r = api.get(f"{PRODUCTS}/browse", params={"kind": "sku", "$filter": f"entity_id eq {sku}"})
        assert r.status_code == 200, r.text
        assert [row["entity_id"] for row in r.json()["rows"]] == [sku], r.text

        _set_quorum(api, "prices", 0)
        _set_quorum(api, "plan_revision", 0)
        eur = _book(api, "EUR", run)
        usd = _book(api, "USD", run)

        # The entry create requires a model its charge kind allows (D-427).
        r = api.post(
            f"{PRICING}/price-books/{eur}/entries",
            json={"sku_id": sku, "period": "month"},
            headers=_key(),
        )
        assert r.status_code == 400, r.text
        for model, refusal in (
            ("stair", "MODEL_INVALID"),
            ("graduated", "MODEL_KIND_CHARGEKIND_MISMATCH"),
        ):
            r = _monthly_entry(api, eur, sku, model)
            assert r.status_code == 400, r.text
            assert refusal in r.text, r.text

        # Two models of one SKU x kind x period are two entries of one book (D-427).
        made = [_monthly_entry(api, eur, sku, model) for model in ("flat", "per_unit")]
        assert [m.status_code for m in made] == [201, 201], [m.text for m in made]
        flat, per_unit = (m.json() for m in made)
        assert flat["id"] != per_unit["id"], made
        assert [
            (e["sku_id"], e["charge_kind"], e["period"], e["model"], e["reference_state"])
            for e in (flat, per_unit)
        ] == [
            (sku, "recurring", "month", "flat", "confirmed"),
            (sku, "recurring", "month", "per_unit", "confirmed"),
        ], made
        again = _monthly_entry(api, eur, sku, "flat")
        assert again.status_code == 409, again.text
        assert "ENTRY_KEY_TAKEN" in again.text, again.text
        # The model is fixed for the entry's life: the PATCH does not carry it.
        r = api.get(f"{PRICING}/price-book-entries/{flat['id']}")
        assert r.status_code == 200, r.text
        r = api.patch(
            f"{PRICING}/price-book-entries/{flat['id']}",
            json={"model": "per_unit"},
            headers={"If-Match": r.headers["etag"]},
        )
        assert r.status_code == 400, r.text
        r = api.get(f"{PRICING}/price-book-entries/{flat['id']}")
        assert (r.json()["model"], r.json()["version"]) == ("flat", flat["version"]), r.text
        r = _monthly_entry(api, usd, sku, "flat")
        assert r.status_code == 201, r.text
        dollar = r.json()

        # A price carries no model, and its money must fit its entry's.
        start = (datetime.date.today() + datetime.timedelta(days=30)).isoformat()
        for body, refusal in (
            ({"model": "flat", "price": {"amount": "30.00"}}, None),
            ({"price": {"rate": "2.50"}}, "PRICE_MISSING"),
        ):
            r = api.post(
                f"{PRICING}/price-book-entries/{flat['id']}/prices",
                json={**body, "eligibility": "all", "effective_from": start},
                headers=_key(),
            )
            assert r.status_code == 400, r.text
            assert refusal is None or refusal in r.text, r.text

        # Each entry's price in its model; the EUR pair approved, the USD price left a draft.
        _draft_price(api, flat, {"amount": "30.00"}, start)
        _draft_price(api, per_unit, {"rate": "2.50"}, start)
        _draft_price(api, dollar, {"amount": "33.00"}, start)
        r = api.post(f"{PRICING}/price-books/{eur}/publish-changes", json={}, headers=_key())
        assert r.status_code == 201, r.text
        assert r.json()["applied"] is True, r.text
        r = api.get(f"{PRICING}/price-books/{eur}/entries")
        assert r.status_code == 200, r.text
        assert {e["id"]: (e["model"], e["usage"]) for e in r.json()["items"]} == {
            flat["id"]: ("flat", _entry_usage(approved=1)),
            per_unit["id"]: ("per_unit", _entry_usage(approved=1)),
        }, r.text

        # The SKU reads: three entries in two currencies, prices by state, no plan yet.
        card, row = _sku_reads(api, sku, code)
        assert (card["sku"]["category_id"], row["category_id"]) == (None, None), (card, row)
        unplanned = _sku_usage(3, ["EUR", "USD"], approved=2, draft=1)
        assert (card["usage"], row["usage"]) == (unplanned, unplanned), (card, row)

        # A plan publishes rev 1 with the flat entry.
        r = api.post(
            f"{PRICING}/plans",
            json={"code": f"plan-nocat-{run}", "name": f"Plan no category {run}", "book_id": eur},
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
            json={"sku_id": sku, "price_book_entry_id": flat["id"], "treatment": "paid"},
            headers=_key(),
        )
        assert r.status_code == 201, r.text
        r = api.post(f"{PRICING}/plan-revisions/{rev1}/submit", json={}, headers=_key())
        assert r.status_code == 201, r.text
        assert r.json()["revision"]["state"] == "published", r.text

        # Its copy rev 2 picks the per-unit entry: the item keeps its SKU, changes its entry.
        r = api.post(f"{PRICING}/plans/{plan}/revisions", json={}, headers=_key())
        assert r.status_code == 201, r.text
        rev2 = r.json()["id"]
        [item] = _revision(api, rev2)["items"]
        assert (item["sku_id"], item["price_book_entry_id"]) == (sku, flat["id"]), item
        r = api.patch(
            f"{PRICING}/plan-items/{item['id']}",
            json={"price_book_entry_id": per_unit["id"]},
            headers={"If-Match": f'"{item["version"]}"'},
        )
        assert r.status_code == 200, r.text
        assert r.json()["price_book_entry_id"] == per_unit["id"], r.text

        # The plan names both entries: each entry counts it, the SKU counts it once.
        assert _usage(api, flat["id"]) == _entry_usage(approved=1, plans=1)
        assert _usage(api, per_unit["id"]) == _entry_usage(approved=1, plans=1)
        assert _usage(api, dollar["id"]) == _entry_usage(draft=1)
        planned = _sku_usage(3, ["EUR", "USD"], approved=2, draft=1, plans=1)
        card, row = _sku_reads(api, sku, code)
        assert (card["usage"], row["usage"]) == (planned, planned), (card, row)

        # Rev 2 published supersedes rev 1: the flat entry is named by history only.
        r = api.post(f"{PRICING}/plan-revisions/{rev2}/submit", json={}, headers=_key())
        assert r.status_code == 201, r.text
        assert r.json()["revision"]["state"] == "published", r.text
        assert _revision(api, rev1)["state"] == "superseded"
        assert _usage(api, flat["id"]) == _entry_usage(approved=1, superseded_only=1)
        assert _usage(api, per_unit["id"]) == _entry_usage(approved=1, plans=1)
        card, row = _sku_reads(api, sku, code)
        assert (card["usage"], row["usage"]) == (planned, planned), (card, row)

        # Each revision resolves in its own entry's model and money.
        for revision, entry, model, money in (
            (rev1, flat, "flat", {"amount": "30.00"}),
            (rev2, per_unit, "per_unit", {"rate": "2.50"}),
        ):
            resolved = _resolve(api, revision, start)
            [resolved_item] = resolved["items"]
            assert (resolved_item["price_book_entry_id"], resolved_item["model"]) == (
                entry["id"],
                model,
            ), resolved
            assert _binding(resolved)["price"] == money, resolved
    finally:
        for kind, quorum in found.items():
            _set_quorum(api, kind, quorum)
