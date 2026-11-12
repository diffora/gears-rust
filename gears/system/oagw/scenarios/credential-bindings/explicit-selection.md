# Explicit credential selection (planned)

These scenarios specify [ADR-0021](../../docs/ADR/0021-credential-bindings.md). Run store-dependent cases against the same backends as the four-level alias suite. Use a recording mock provider, a CredStore spy and a controlled clock. No provider credentials are needed.

## Two accounts on one route

Configure one local API-key upstream with default key D and names `research` (A) and `production` (B). Send the same POST and body with no selector, then once per name. Expect exactly three provider calls with D, A and B respectively. Compare the entire first outbound request to the legacy baseline. Check that no selector is present, including with full passthrough or a header rule that reintroduces it. If a rule/transform replaces the auth value, expect a gateway denial and no provider call. Repeat for SSE and WebSocket handshakes. Each stream/upgrade consumes one request on admission.

## Selection and preview denials

Send an empty, duplicate, comma-joined, mixed-case value or reserved `default` selector. Expect invalid argument and no provider call. An unrelated tenant's valid name, a missing name and a concealed use denial return indistinguishable 404s. The same cases through SDK preview return the same denial. An authorized caller selecting a disabled row gets 503 and Retry-After; deletion yields 404. Check management get/list and pagination under limited grants. For denials before auth assert zero secret reads; successful preview returns only safe identifiers and also makes zero secret reads, debits and outbound calls. Revoke a use grant and repeat.

## Destination and hierarchy

Create four same-alias layers and exercise the ADR truth table, including a visible ancestor with absent auth due to shared CORS. An authorized local shadow can use its own name with the required grants. A descendant cannot enumerate or borrow a parent's name. Turn on enforce above an existing local binding; invoke and preview must now fail. Configure a pool whose alias remains unchanged while a new host, port or scheme falls outside the approved set. Try directing the request to that endpoint with the target-host header. Expect denial before secret access. Narrowing to approved endpoints works. Check disabled upstream handling, SSRF, HTTPS-only, route choice and enforced guards on named and omitted-selector calls.

## Request counts

Set A to 1/minute, give B capacity, and hold time fixed. First A reaches the mock, second A returns a gateway 429 with retry guidance, B succeeds. Exhaust the upstream shared pool and repeat with B: upstream rejection occurs before B is debited. Separately exhaust the route: earlier successful debits stand. Verify success header aggregation, refilling, independence across tenants, reset on a rate change, unchanged capacity on secret rotation, and fresh UUID/bucket after delete/re-create. A provider-generated 429 remains upstream-sourced and produces exactly one call.

## Storage and registry

Race two creates for the same normalized name; exactly one succeeds. Race parent removal with child creation on each backend and check for orphans. Provision a valid declaration twice and check stable identity; rotate its secret and verify no other mutable fields change. For invalid names, duplicate declarations and inaccessible references verify the documented skip/retry and preservation of previously valid state. Attempt an API update of a registry row. Race a new API child with reconciliation of its disappearing registry parent; if the child commits first the parent survives, otherwise the child cannot commit. Verify fresh binding identity after confirmed removal and reappearance of the same registry source. Inspect access logs and errors with unique canary secret values; no value or auth header may appear, and metric labels contain neither names nor IDs.

## Two replicas and rollback

With a shared DB, exercise named proxy and preview on replica B, then miss on an unknown name. Create it on A and verify B sees it on next admission. Drop change notifications; rotate, disable, delete or change the parent's policy on A and repeat on B after each acknowledged commit. New admissions use the current configuration or fail before forwarding. On failed/slow authoritative reads, observe the documented availability failure at the read deadline, even if B has an old alias/route cache entry. On a binding delete verify local counter cleanup and remote cleanup after delivery of the change event.

With an old and a capable replica behind the same ingress, keep named admission closed. The negative control is an old replica that would strip the selector and send D. Verify at the mock that the release procedure sends no named request to that replica, including SDK paths. After the last old replica and writer have left, enable selectors and verify A/B. For rollback, block new named traffic, drain it and fence registry writers before reintroducing the old version. Omitted-selector traffic must still use D.
