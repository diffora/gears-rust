# BSS Approvals — Decision Register

**Status:** the facade decisions for the inbox. The gears' own registers stay pricing D-470 and products P-D-227; their sources are pricing D-490 and products P-D-250.

<!-- toc -->

- [Register](#register)
- [Entries](#entries)
  - [AP-D-1 The inbox is a facade and has no authorization resource](#ap-d-1-the-inbox-is-a-facade-and-has-no-authorization-resource)
  - [AP-D-2 The merge, the cursor, the narrowing and book_id](#ap-d-2-the-merge-the-cursor-the-narrowing-and-book_id)
  - [AP-D-3 Grants and owner resolution](#ap-d-3-grants-and-owner-resolution)
  - [AP-D-4 Votes and idempotency](#ap-d-4-votes-and-idempotency)

<!-- /toc -->

## Register

| ID | Priority | Decision | Status / source |
| --- | --- | --- | --- |
| AP-D-1 | H | The inbox is a facade and has no authorization resource | DECIDED 2026-10-01 |
| AP-D-2 | H | The merge, the cursor, the narrowing and `book_id` | DECIDED 2026-10-01 · amended by Run 2 (pricing D-490, products P-D-250) |
| AP-D-3 | H | Grants and owner resolution | DECIDED 2026-10-01 |
| AP-D-4 | H | Votes and idempotency | DECIDED 2026-10-01 · amended by Run 2 (pricing D-490, products P-D-250) |

## Entries

### AP-D-1 The inbox is a facade and has no authorization resource

The units stay in the gear that writes them. The inbox reads and routes. It does not implement a database capability and it does not call `db_required`.

The gateway authenticates the caller. The inbox does not judge a grant. Each source door authorizes with its own resource. The problem type on an inbox refusal names the error envelope, not an authorization resource.

### AP-D-2 The merge, the cursor, the narrowing and book_id

The shared order is the facade's merge key: `submitted_at` as an instant, then the unit id in the same direction. That amends the per-gear order only by being the key the inbox merges on. Each gear's own door is unchanged.

Every source is asked on every page after its own key. The key becomes the last unit taken from that source, or stays. There is no exhausted state. The next cursor is present when any source had more or returned a unit that was not taken.

The cursor carries its order outside the narrowing hash. `$orderby` with a cursor is 400 `ORDER_WITH_CURSOR`. A changed narrowing is 400 `FILTER_MISMATCH`.

A kind outside a gear's closed set is an empty page and zero counts, computed in the source. `book_id` on products is that same empty set. `book_id` on pricing remains the alias of `ref_id`: it keeps `prices` units of that book and no `plan_revision`, whose reference is the revision. A state or id the door would refuse is that refusal for the whole read.

A source added to the configuration later starts from an empty key. A removed source's key is ignored.

Run 2 amends this entry (pricing D-490, products P-D-250). Each real source builds the pager's own `CursorV1` from its key and reads through its gear's list read, so the keyset is the pager's column compare. A kind that no gear records, such as `bogus`, is therefore an empty page and zero counts from every source: the inbox answers 200 with no unit and every readable source named `ok`, never 400.

### AP-D-3 Grants and owner resolution

On the list and the counts, a source that answers 403 is omitted and named `forbidden`. When every source answers 403, the read is 403 and the body names no gear. A source that answers 503, or is not registered, is 503 `SOURCE_UNAVAILABLE` naming it. `total` counts the readable sources only.

On the card and the vote, every source is asked. One hit wins. Two hits are 500 naming both. Otherwise any 503 or missing source is 503 `SOURCE_UNAVAILABLE`. Otherwise any 403, with the rest missing, is 403 whose body names no gear. Otherwise the unit is 404.

### AP-D-4 Votes and idempotency

The inbox resolves the owner with the card's rule, then calls that source's single `vote` with the request body bytes and the `Idempotency-Key` as received. The source calls its own vote door, under that door's grant, with that door's own path as the idempotency endpoint. The facade and the direct door therefore share one idempotency row.

The door's answer is returned unchanged: status, headers and body. The success body is the door's receipt. A refusal keeps the door's body, including `generation` on `GENERATION_MISMATCH` and `UNIT_STALE`.

Run 2 amends this entry (pricing D-490, products P-D-250). Each source sends the vote to its gear's vote door through that gear's router, under the gear's enforcer and the platform's error layer, with the body bytes as `application/json`. A refusal therefore leaves the door with its `instance` naming the door's path. The inbox marks its answer as a passthrough (`ForeignPassthrough`), so the platform's error layer around the inbox does not rewrite a refusal the door already shaped. A refusal's `trace_id` is the one the door's layer derived: the source forwards no trace header. A facade vote and a direct vote with the same key and body replay once in both gears, and the same key with another body is the door's 409 `IDEMPOTENCY_CONFLICT`, byte for byte.

Declared codes: 400 `GENERATION_REQUIRED`, `GENERATION_MISMATCH`, `UNIT_STALE`, `NOTE_REQUIRED`, `NOTE_TOO_LONG`, `BODY_UNEXPECTED`; 403 for the grant and `SOD_VIOLATION`; 404; 409 `DUPLICATE_VOTE`, `UNIT_ALREADY_DECIDED`, `IDEMPOTENCY_CONFLICT`; 503. These doors do not answer 412. `InboxUnit` has no version.
