# Pricing UI contracts — 2026-09-10

This amendment describes backend changes in `diffora/glcode-pricing`, not a
claim that Pricing Studio has integrated them. D-361–D-367 amend the earlier
read/approval contracts without changing plan lifecycle or publish policy.
D-374 adds Working/Committed window reads (`view` as a door discriminator).

## Plan pending approvals

Both `GET /bss-pricing/v1/plans` items and `GET /bss-pricing/v1/plans/{planId}`
return the same field, replacing `in_review` and singular `pending_approval`:

```json
{
  "pending_approvals": [
    {
      "approval_id": "0198d438-42dd-7000-8000-000000000001",
      "subject_kind": "plan_revision",
      "submitted_at": "2026-09-10T09:00:00Z"
    }
  ]
}
```

An empty array means no submitted directly owned units. The included subject
kinds are `plan_revision`, `window` and `price_unit`, resolved through the
existing aggregate parser. Several independent units can belong to one plan.
Policy, taxonomy, overlay and bulk effects are not direct ownership and are
excluded. Order is `(submitted_at ASC, approval_id ASC)`.

One batch read serves the authorized page; it does not query once per plan or
introduce a stored review flag. A plan-read caller receives only these three
fields. Author identity, reason and proposed content remain approval-read data.
The revision-only submit guard and canonical scope-key conflicts are unchanged.

This is a branch API replacement: clients must switch both old fields to the
array. It has not been rolled out to another service or UI repository.

## Approval participant names

`GET /approvals` items and `GET /approvals/{id}.approval` use the same enriched
record. Existing scalar IDs remain; these additional fields are read-only:

```json
{
  "submitter_principal": "00000000-0000-0000-0000-0000000005c0",
  "approver_principal": null,
  "submitter": {
    "principal_id": "00000000-0000-0000-0000-0000000005c0",
    "display_name": "Alice Author",
    "name_status": "resolved"
  },
  "approver": null
}
```

`submitter` is always present on GET, even if its name cannot be resolved.
`approver` is null only when no approver principal is stored (pending or voided).
The name comes from ID-filtered `AccountManagementClient::list_users` in the caller's tenant,
under the original caller context: nonblank `display_name`, then joined
`first_name`/`last_name`, then nonblank `username`. No email field is exposed.
Pricing stores only its existing UUIDs: no profile table, name column, name in
the pin/audit, cross-request cache, or independent editing endpoint is added.
An IdP rename is visible on the next GET; this is not a historical-name snapshot.

The name states are:

| `name_status` | `display_name` | UI meaning |
| --- | --- | --- |
| `resolved` | Nonblank string | Show current participant name/login |
| `restricted` | null | Profile access denied; do not render as an anonymous actor |
| `not_found` | null | No user visible in the requested tenant; not proof of deletion |
| `unavailable` | null | AM absent, unsupported lookup, source failure, deadline or invalid profile |

Approval access is checked **before** resolving names. AM independently requires
its **user × list** permission; approval-read alone does not
grant it. No role grants or service impersonation are added by Pricing. Deployments
must provide the appropriate AM permission to reviewers to display actual names.
An AM failure does not hide an otherwise readable approval or prevent decisions;
the GET remains 200 with the explicit per-participant state. Private error details
and mismatched-ID profiles are not exposed. Mutation responses keep their original
`ApprovalView` and do not invoke AM; follow with GET to obtain enriched identities.
The minimal pending links on plans/taxonomies remain free of participant PII.
Both approval GETs return `Cache-Control: private, no-store`.

### Identity and scaling limits (not a completed production integration)

The default OIDC mapper uses UUID `sub`, and Keycloak's `IdpUser.id` is the provider
user UUID. Custom subject mappings/static principals are not automatically mapped.
A service principal is resolved only if AM exposes it as a tenant-visible user;
otherwise the same explicit failure states apply. The public service-account list
does not carry `subject_id`, so Pricing cannot safely join it by UUID or infer an
account name/kind from a missing user. No synthetic "System" name is substituted.

Within one response, IDs are deduplicated across both roles and all rows. At most
four batches run concurrently, each containing at most 200 UUIDs, with
**one two-second budget for the whole response**;
completed names survive, unfinished/queued names become unavailable. No lookup
starts after the deadline. AM is resolved lazily from ClientHub, so registration
order does not permanently disable names; an absent AM reports unavailable.

`ListUsersQuery::with_ids` constructs a typed, deduplicated UUID `in` filter.
Pricing follows its cursor pages with the same filter, never requests an unfiltered
tenant list, and rejects duplicate/unrequested IDs. Missing IDs become `not_found`
only after the filtered result is exhausted; a failed continuation preserves names
already obtained but marks unresolved IDs unavailable/restricted, not absent.

Keycloak supports this exact ID-set filter (including cursor pinning) and reads
tenant-group membership once per batch page, not once per participant. It still
enumerates members internally; large-tenant indexing/performance remains a provider
limitation. Reaching its safety cap now fails unavailable instead of returning a
truncated successful list. Static IdP already supports typed ID sets. A live
AM/Keycloak/IAM smoke test and service-principal mapping remain open. The automated
tests cover Pricing's adapter, SDK queries and Keycloak HTTP responses via wiremock;
they are not a deployment smoke test.

## Taxonomy references

Both taxonomy list and by-value GETs include:

```json
{
  "value": "eu",
  "display_name": "Europe",
  "edit_governed": true,
  "references": {
    "published_price_rows": 3,
    "active_overlay_scopes": 1
  }
}
```

The same `ValueReferences` drives the flag and both numbers. Returning the
counts adds no query to the existing list fold. Zero counts are explicit.
Published price **rows**, not plans, are counted; non-region classes report
zero on that plane. Overlay counts mean published revisions selecting the
value, with no new current-time predicate. Writes and approval pins omit these
read-only fields.

## Taxonomy pending proposals

Both taxonomy GETs include `pending_approvals: []` on every value, for all four
classes. It contains every submitted proposal for that exact tenant/class/value,
ordered by `(submitted_at ASC, approval_id ASC)`. The live label stays unchanged:

```json
{
  "value": "eu",
  "display_name": "EU",
  "pending_approvals": [
    {
      "approval_id": "0198d438-42dd-7000-8000-000000000002",
      "subject_kind": "taxonomy_value",
      "submitted_at": "2026-09-10T10:00:00Z",
      "content_access": "granted",
      "proposed_changes": { "display_name": "Europe" },
      "content_matches_pin": true
    }
  ]
}
```

`config × read` grants only the three link fields and `content_access`.
`proposed_changes` and `content_matches_pin` additionally require `approval × read`
on that specific unit, including resource-id and tenant constraints. Without it:

```json
{
  "approval_id": "0198d438-42dd-7000-8000-000000000002",
  "subject_kind": "taxonomy_value",
  "submitted_at": "2026-09-10T10:00:00Z",
  "content_access": "restricted"
}
```

Do not render restricted as "no changes". Show "Change awaiting approval;
proposal unavailable". An unavailable authorization service returns 503, not a
fabricated permission denial or empty list. With no pending units, no preview
authorization request is needed.

The patch comes from the existing approval subject, not a pending-name column
or another draft store. Only authored patch fields appear; omitted means keep,
`tax_category: null` clears and an explicit `tax_rate_present: false` is preserved
while these legacy region fields exist. Full reasons and participants are not
embedded. `content_matches_pin: false` marks a stale proposal; it must not be
presented as a valid replacement or a historical before-state snapshot.

Approve removes that unit from pending and applies its label atomically. Sibling
proposals remain visible, with a stale pin where appropriate; reject/withdraw/void
also remove decided units. Writes and approval before/after views omit all pending
enrichment, and POST/PATCH do not accept these read-only fields.
Those two request DTOs now explicitly reject unknown properties with 400;
previously serde ignored them despite the declaration DTO's contrary comment.

The read adds at most two approval-table queries and one PDP request for the
whole returned collection, never one detail request per value/unit. Because the
subject is encoded JSON in a string, the first query scans the tenant's submitted
taxonomy units and filters their parsed class/value against the authorized values;
the second intersects candidate ids with the unmodified approval-read scope.
Malformed persisted proposals fail the read without echoing proposal contents.
Metadata and pin freshness are advisory reads, not a lock on a later decision.

### Fresh reads and unchanged write preconditions

Plan and taxonomy ETags still cover authored content, not pending approvals or
reference counts: they remain the existing **write preconditions**. Enriched GETs
now always return fresh bodies with `Cache-Control: private, no-store`. They do
not evaluate `If-None-Match` or return 304, even when sent a matching tag. Plan
lists use the same no-store policy. This also prevents a cached taxonomy preview
from surviving a change in approval-read permissions. Actual writes still check
their existing tags; no pending/reference-only change invalidates an editor's
If-Match. Other Pricing resources retain their existing conditional-read contracts.

## Taxonomy approve applies the patch

1. Config author PATCHes a referenced value under its current per-value tag.
   Response is 202 with the approval id; the current name is unchanged.
2. An independent reviewer POSTs `/approvals/{approvalId}/approve`.
3. In one transaction, the service verifies the pin, records the decision,
   re-checks domain guards, applies the stored proposal and records its audit.
4. GET now returns the new name and tag. There is no second PATCH.

The reviewer needs `approval × approve`, not the author's `config × write`.
The original PDP scope first selects the exact approval. Only then are its
tenant constraints projected for that stored proposal's taxonomy/audit effect;
an approval resource id is not reused as a taxonomy or audit-chain id.

Any apply failure rolls back the verdict and its audit too. Retry receives
`APPROVAL_NOT_PENDING` and does not apply or audit twice. A stale original PATCH
gets `STALE_VERSION`; the client must re-read. Other approval kinds retain their
existing commit workflows. No sibling units are automatically voided.

Previously approved-but-unapplied units are not mass-replayed: the existing
exact-subject/exact-content PATCH path is retained for compatibility. Historical
pins are not rewritten. After application, approval detail's re-derived content
may no longer match its old pin; it is not a stored before-state snapshot.

## Tax-category dictionary: read-only Product Catalog port

`ProductCatalogClientV1::list_tax_categories(ctx)` returns
`Vec<CatalogTaxCategory { code, display_name }>`; it has no default data or CRUD.
The same configured provider serves:

```http
GET /bss-pricing/v1/catalog/tax-categories
```

```json
{
  "source": "local_dev_static",
  "items": [
    { "code": "DEV-TAX-SUBSCRIPTION", "display_name": "Subscription (demo)" },
    { "code": "DEV-TAX-USAGE", "display_name": "Usage (demo)" },
    { "code": "DEV-TAX-SUPPORT", "display_name": "Support (demo)" }
  ]
}
```

Gate: `config × read`. A registered provider has source `registry`. The demo is
only enabled by existing `product_catalog.mode = "local_dev_static_skus"`; its
codes are stable, explicitly fabricated, and have no rate/jurisdiction meaning.
An unconfigured provider answers 501, an unavailable one 503, and a configured
empty dictionary answers 200 with `items: []`. There is no production fallback,
category table or category CRUD; Products still has no implementation here.

The agreed target assigns the category solely on `price.tax_category_ref`;
Products owns definitions and Tax Engine owns rates/rules. **This tranche adds
the dictionary, not the migration:** region `tax_category`, its fallback and
`tax_rate_present` still exist, and backend category membership validation has
not yet been connected. Existing publish and GA sellability guards remain.

## Plan list queries and creation timestamps

The shared OData toolkit is unchanged. All new translations live in Pricing.
The database first selects the open draft, otherwise the current published/retired
revision, then filters, sorts and pages that projection. A name/state from an old
revision cannot make the plan match; use history for superseded/abandoned rows.

`$orderby` supports `plan_id`, `plan_name`, `lifecycle_state`,
`created_at`, `price_row_count`, with `asc`/`desc` and multiple keys. An absent
`plan_id` is appended ascending as the unique tie-breaker. Nullable names/cycles
sort **last in both directions**. Count ordering happens in SQL before LIMIT,
not by sorting the current page. On subsequent pages send `cursor` plus the same
`$filter`, but no `$orderby`; the cursor carries the effective order.

```http
GET /bss-pricing/v1/plans?$orderby=plan_name asc&limit=25
GET /bss-pricing/v1/plans?$orderby=price_row_count desc
GET /bss-pricing/v1/plans?$filter=contains(plan_name,'enterprise')
GET /bss-pricing/v1/plans?$filter=model_kind eq 'flat' and currency in ('EUR','USD')
GET /bss-pricing/v1/plans?$filter=has_pending_approvals eq true
```

Examples show decoded query text; URL-encode it in an actual request.
`plan_name eq` is exact. Substring/prefix/suffix search folds case using the
database's lower-case rules (SQLite's non-ASCII folding is not PostgreSQL's).
LIKE metacharacters are escaped, not treated as user wildcards. Existing toolkit
limitations remain: no collection lambda `any`, and no `eq null` on String fields.

The response arrays are still `model_kinds` and `currencies`. Query-only
`model_kind`/`currency` express independent membership predicates over the same
draft/published price-row set: a flat/USD row and a per_unit/EUR row jointly match
`model_kind eq 'flat' and currency eq 'EUR'`. `ne`/`not(eq)` mean no matching row.
Plans with zero rows have empty arrays and a count of zero. Other existing
filters (`plan_id`, `sku_id`, `plan_tier`, `lifecycle_state`)
remain supported.

`has_pending_approvals` is query-only; it is not a stored boolean or an additional
response flag. It uses the same submitted direct PlanRevision/Window/PriceUnit
ownership parser as `pending_approvals`. Its current implementation first loads
authorized authoring plan IDs under the original scope, then reads submitted
direct units into that exact metadata join, and retains the original scope on
the final paginated query. It is not an indexed approval-to-plan join; large
catalogues/pending queues still need performance validation of this filter.

On list/detail and plan mutation responses:

```json
{
  "created_at": "2026-08-01T09:00:00Z",
  "revision_created_at": "2026-09-10T14:30:00Z"
}
```

`created_at` comes from the earliest retained revision, not a new stored copy;
opening another revision does not change it. `revision_created_at` comes from
the shown revision. `created_at_utc` is removed from these plan DTOs and the plan
filter/sort contract; other resources and stored timestamps are unchanged.
Clients must migrate the old spelling deliberately because the meaning differs.

The original plan-read scope constrains both the canonical-revision subquery
and the page. Price/pending metadata is joined only to authorized tenant/plan
identities; a plan resource ID is not incorrectly used as a price/approval ID.

## Approval counts

```http
GET /bss-pricing/v1/approvals/counts
```

```json
{ "total": 42, "submitted": 7, "approved": 28, "rejected": 5, "voided": 2 }
```

One database aggregate over the whole original `approval × read` scope,
including resource-ID restrictions. All states are returned, including zeroes;
they sum to `total`. A withdrawal is `voided`, not a fifth state. No participant
profile lookup is made. No query filters or pagination in this first version:
any nonempty query string receives 400. Responses are private/no-store.

## Remaining work / decisions

- Production identity integration: real AM/IdP and IAM smoke test, large-tenant
  performance of the implemented batch lookup, and mapping for service/custom principals.
- Connect the dictionary to backend category validation once per operation.
- Remove region assignment/fallback with an explicit legacy-data and old-pin
  strategy; decide tax-rate readiness separately. Do not rewrite frozen history.
- Coordinate client rollout for fresh reads and the changed plan timestamp/query contract.

The [published explanatory HTML](https://artifacts.os.jele.io/pricing-branch)
has been refreshed for D-361–367. This does not deploy Pricing or integrate the
real Pricing Studio; the adjacent Studio artifact is unchanged.

## Working and Committed window reads (D-374)

`view` is a **door discriminator**, not an OData `$filter` field (same class as
products `kind`). Default remains `committed`: today's live `pricing_price_window`
table. Draft intentions never appear on the committed door.

Working reads are `plan × read` and require the parent plus revision:

| Surface | Required query |
|---|---|
| `GET /bss-pricing/v1/price-windows` | `view=working&plan_id={planId}&plan_revision={n}` |
| `GET /bss-pricing/v1/plans/{planId}/coverage` | `view=working&plan_revision={n}` |

Unknown `view` is 400. `view=working` on a revision that is not an open draft is
409 `DRAFT_WINDOW_CONTEXT_CHANGED`. Named `plan_id` is legal **only** with
`view=working`; on the committed collection it stays 400 (existing OData-only
contract). Working composes captured live baseline **references** plus draft
operations and returns provenance so a live record cannot be confused with a
draft intention. List and coverage keep a legally saved proposal visible after
the clock has moved (elapsed exact start, or a staged cancel/adjust whose live
state has changed); submit and commit still refuse those cases. Bind `view`,
parent and revision into the cursor fingerprint.
Historical published revisions are not mutable working views.

`at_publish` stays symbolic on Working until commit stamps `effective_from`.
Unchanged baseline ids are references, not copies.

## Line-first authoring: charge lines and market prices (2026-09-20)

**A price row is no longer authored, or read, as one object.** What every market
of a charge shares — the eight structural axes, the calculation model, tier
*geometry*, the usage policy, the descriptor and the proration contract — is a
**charge line**. What differs per currency and region — the amounts and rates that
fill that structure, and the market's tax and rounding policy — is a **market
price** filed under an exact line version. A Studio screen that edits "a price"
now edits one of the two, and the wire says which.

```text
POST/GET         /bss-pricing/v1/plans/{planId}/charge-lines
GET/PATCH/DELETE /bss-pricing/v1/plans/{planId}/charge-lines/{lineVersionId}
POST/GET         /bss-pricing/v1/plans/{planId}/charge-lines/{lineVersionId}/prices
GET/PATCH/DELETE /bss-pricing/v1/plans/{planId}/prices/{priceId}
```

`POST /plans/{planId}/prices` is **removed**, not deprecated: a price row cannot be
created without naming the structure it is priced against. The `priceId` routes
stay — windows, history and supersession address a monetary version by id — and
their `PATCH` now takes `{money, market_policy}` only.

A line response carries both identities separately, and its prices with theirs:

```json
{
  "charge_line_id": "…", "line_version_id": "…", "plan_revision": 0,
  "lifecycle_state": "draft", "row_version": 0,
  "scope_key": {"plan_id": "…", "price_overlay": "base", "phase": "…",
                "price_eligibility": "all_subscriptions", "charge_kind": "usage",
                "cohort": null, "sku_id": "…", "dimension_key": null},
  "structure": {"model_kind": "graduated", "meter": "cloudlets",
                "tiers": [{"from_qty": 0, "to_qty": 100}, {"from_qty": 100, "to_qty": null}]},
  "prices": [
    {"market_price_id": "…", "price_id": "…", "line_version_id": "…",
     "charge_line_id": "…", "currency": "USD", "region": "US",
     "money": {"tier_rates_nano_minor": [500, 400]},
     "market_policy": {"tax_inclusive": false},
     "lifecycle_state": "draft", "row_version": 0}
  ]
}
```

`charge_line_id` is stable across revisions; `line_version_id` is the content of
one revision and freezes on publication. `market_price_id` is the stable variant;
`price_id` is the monetary version. A revision keeps the logical ids and allocates
new version ids.

### Which tag a screen must hold

| Act | Precondition | Answers |
|---|---|---|
| `POST`/`PATCH`/`DELETE` a line | the **plan revision's** tag, `"<revision>-<version>"` | the moved plan tag |
| `POST` a market price | `Idempotency-Key`, no tag | the row's own `"<version>"` |
| `PATCH`/`DELETE` a market price | the **row's own** tag | the row's new tag |

A line's structure is the plan's content, so editing it moves the plan tag and a
screen holding a stale one gets `409 STALE_VERSION` — the same contract a draft
window write has. Filing or repricing a market does **not** move the plan tag, so a
pricing grid can post markets under a tag it captured once.

### Each door refuses the other's fields

Both request shapes set `deny_unknown_fields`. `currency`, `amount_minor` or
`tax_inclusive` on a structure is `400 unknown field`, and so is `model_kind`,
`tiers` or `package_size` on a market price. A field that is silently dropped is a
screen that believes it saved something, which is why neither door ignores them.
`meter` is derived from the SKU and refused on write **including an explicit
`null`**; `charge_kinds` is derived and never authored.

### Tier rates

Geometry is the line's and rates are the market's, joined by position:
`money.tier_rates_nano_minor` is one rate per tier of the line, in its quantity
order. A different count is `400 MARKET_TIER_RATE_COUNT_MISMATCH`. Sending none is
a legal unfinished draft. Editing the geometry keeps each market's rates where the
new ladder still has a tier at that position, so a grid does not lose its numbers
when a bound moves; a market left with fewer rates than tiers is reported by the
publish pre-check, not refused at save.

### Deleting

`DELETE` of a line version a market price still references is
`409 CHARGE_LINE_IN_USE`, naming a row to delete first. Deleting a line's only
version deletes the logical line and its markets, and frees the axes for a new one.

## Approval document and published reads carry the normalized graph (2026-09-20)

`GET /approvals/{id}` → `pinned_content` gains two arrays, shown because the content pin
(generation `v21`) now covers them:

```json
"charge_lines": [
  { "charge_line_id": "…", "line_version_id": "…",
    "scope_key": { "phase": "…", "sku_id": "…", "price_eligibility": "all_subscriptions",
                   "charge_kind": "recurring", "cohort": null, "dimension_key": "" },
    "structure": { "model_kind": "flat", "gl_code_ref": "4000", "billing_timing": "advance" } }
],
"market_prices": [
  { "market_price_id": "…", "price_id": "…", "line_version_id": "…",
    "currency": "EUR", "region": "eu", "money": { "amount_minor": 9900 } }
]
```

`structure` and `money` are the same shapes the charge-line routes use. `rows` is unchanged and
still the resolved join; the two arrays are what it drops. A reviewer UI should render a
**line with no entry in `market_prices`** prominently — it is a line that sells in no market,
and publish refuses it (`LINE_MARKET_PRICE_MISSING`).

Every approval opened before this deploy answers `APPROVAL_CONTENT_MISMATCH` on its next
decision (the pin generation moved) and has to be withdrawn and resubmitted.

The **published** plan payload (read model / `pricingSnapshotRef`) gains, on every
`prices[]` member, `chargeLineId`, `lineVersionId` and `marketPriceId` beside `priceId`, and on
every `windows[].intervals[]` member, `windowId` and `priceId`. All five are `null` only on a
delta built off the publish path; a published read always carries them.
