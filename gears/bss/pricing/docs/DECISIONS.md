# Pricing — Decision Register (PriceBook)

**Replaces:** the charge-line/market-price register on `bss/products-backup` (`3a38f0b28`), ending at D-383.
Historical decisions remain there and in git history; only living rules restated below govern the new model.
**Authority:** `docs/superpowers/specs/2026-09-24-pricebook-model-design.md`, especially §2, §2.2 and §13,
then this register, then code, then descriptive prose. D-399 is the explicit phase-plan deviation; D-403 overrides
the spec's 422 wording.
**Status:** model decisions accepted; implementation remains unchecked in the FEATUREs.

<!-- toc -->

- [Register](#register)
- [Entries](#entries)

<!-- /toc -->

## Register

| ID | Priority | Decision | Status / source |
| --- | --- | --- | --- |
| D-384 | H | Books own currency and validity | DECIDED 2026-09-25 · §2 decision 5; §5; §8 |
| D-385 | H | One registered dimension, independent value chains | DECIDED 2026-09-25 · §2 decision 4; §5; §14 |
| D-386 | H | SKU type defines the price key and allowed models | DECIDED 2026-09-25 · §2 decision 5; §5 |
| D-387 | H | Tier bands are half-open | DECIDED 2026-09-25 · §5 tier bands; §10 |
| D-388 | H | Minimum fee is per row per subscription per period | DECIDED 2026-09-25 · §2 decision 13; §5 |
| D-389 | H | Descriptors bind from durable SKU versions | DECIDED 2026-09-25 · §2 decision 14; §2.2; §7.1 |
| D-390 | H | Windows normalize per chain and preserve usage structure | DECIDED 2026-09-25 · §2 decision 16; §5 |
| D-391 | H | Temporary changes keep pair identity or resume fallback | DECIDED 2026-09-25 · §5 temporary pairs |
| D-392 | H | Publish changes is a selected book batch | DECIDED 2026-09-25 · §2 decision 7; §6; §8 |
| D-393 | H | One unit engine, quorum and generations | DECIDED 2026-09-25 · §2 decision 8; §2.2; §6 |
| D-394 | H | Plans are versioned structure bound to one book | DECIDED 2026-09-25 · §5 plans; §6; §8 |
| D-395 | H | Promotions are versioned and migrations are requests | DECIDED 2026-09-25 · §5; §6; §11 phase 3 |
| D-396 | H | One replay store and optimistic conditional writes | DECIDED 2026-09-25 · §2.2; §3 items 23 and 27; §7.2 |
| D-397 | H | Consumer pins replace cohorts and catalog versions | DECIDED 2026-09-25 · §2 decision 6; §7.1; §12 |
| D-398 | H | Reference reservation closes the lifecycle race | DECIDED 2026-09-25 · §2 decision 17; §13 |
| D-399 | H | No SkuChanged listener or local SKU cache in phase 2 | DECIDED 2026-09-25 · Phase 2 plan, Global Constraints; deviation from spec §7.3 and §11 |
| D-400 | H | Toolkit outbox and broker TypedEvent own event delivery | DECIDED 2026-09-25 · Phase 2 plan Task 2a.2 and 2c.8; Products phase 1 pattern |
| D-401 | H | Reference work is a durable op written before reserve | DECIDED 2026-09-25 · Phase 2 plan 2c.1/2c.6; plan review findings 2, 3 |
| D-402 | H | The pair guard compares SKU metering as of each row's start | DECIDED 2026-09-25 · Phase 2 reconciliation matrix row 24; spec §5 pair guard; supersession-continuity family |
| D-403 | M | Validation refusals are 400 with a code; no wire 422 | DECIDED 2026-09-25 · toolkit canonical error mapping; Products phase 1 behaviour; deviation from spec §2 decision 16 and §6 |
| D-404 | H | A draft belongs to its author | DECIDED 2026-09-25 · Phase 2 review (chains MEDIUM-1, docs F2); spec §6 "author ≠ approver" (finding 8) |
| D-405 | M | Publish changes completes a selected pair | DECIDED 2026-09-25 · Phase 2 plan and reconciliation row 23; Phase 2 review (docs F4) |

## Entries

#### D-384 [H] Books own currency and validity

One currency book owns its SKU × charge kind × period prices. Book code is unique within the tenant. Revisions select a book; every plan reading it shares its money. A plan-specific exception uses another book or SKU, not variant. Book export is a single read-only JSON GET (decision 16). See ADR-0001.

**Source:** §2 decision 5; §5; §8.

#### D-385 [H] One registered dimension, independent value chains

A tenant-level dimension_key registry, initially region, supplies values. Each price selects at most one key; null dim_value is the default chain. Resolve chooses an in-force value row before default. A default is optional and coverage is judged per value. A value with rows cannot be removed; dimension_key changes only while no row has a value. See ADR-0002.

**Source:** §2 decision 4; §5; §14.

#### D-386 [H] SKU type defines the price key and allowed models

The book key is (sku_id, charge_kind, normalized period); there is no plan, phase, variant, cohort, region or currency axis within that key. Recurring uses month/year and flat or per_unit; one_time has no period and flat or per_unit; usage has no period and per_unit, graduated, volume or package. Bundle SKUs cannot be priced. A mismatch is CHARGE_KIND_SKU_TYPE.

**Source:** §2 decision 5; §5.

#### D-387 [H] Tier bands are half-open

All tier bands use [from, to). Quantity 1000 belongs to the band beginning at 1000, including volume cliffs. Correct the prototype comparison qty <= upTo when porting; the surviving tier-boundary goldens are the arithmetic oracle. Do not copy the prototype boundary bug.

**Source:** §5 tier bands; §10.

#### D-388 [H] Minimum fee is per row per subscription per period

Aggregate every bound value and slice rated by the same row, deduct included quantities, then apply its floor prorated by the fraction of the period covered, before promotions. Two values sharing a default row share one floor. Separate valued rows carry separate floors. No plan cap or plan minimum survives.

**Source:** §2 decision 13; §5.

#### D-389 [H] Descriptors bind from durable SKU versions

Rows do not freeze GL, tax, invoice descriptors, metering or timing. Consumers bind the SKU version in force at the period start via versions?as_of; equal effective dates choose the highest published_version. An earlier pin keeps its descriptors forever. A descriptor change creates no refreeze row or pricing approval unit.

**Source:** §2 decision 14; §2.2; §7.1.

#### D-390 [H] Windows normalize per chain and preserve usage structure

On approval, sort approved rows within each (price_id, dim_value), set predecessor effective_to to successor effective_from, and enforce one approved start per chain. The default tail stays open; a value tail may explicitly end and resume default fallback. Re-read chains transactionally, with serializable Postgres isolation. Usage successors cannot change model kind, package size or SKU metering as of each row's start (D-402): CHAIN_MODEL_CHANGED at submit, revalidated at apply.

**Source:** §2 decision 16; §5.

#### D-391 [H] Temporary changes keep pair identity or resume fallback

On an existing chain, temporary_until creates a promo row and a return row in one unit. The return copies the money versionAt would apply at the end and inherits dim_value. Common-date shifts preserve duration. A return to an explicitly closed row (a temporary nested in a value's closed row) ends explicitly at that row's end, which a common-date shift does not move, so the value falls back to the default after it. Submit and apply re-derive that copy from the approved chain as it stands, on the shifted end: a return that no longer names the row in force there, or no longer carries its model, price and min_fee, and a single closed row whose own chain now has a row in force on its end, are refused PAIR_RETURN_STALE (400 at submit, APPLY_REFUSED at apply); the author re-drafts. A value with no own chain gets one closed row and no synthetic return copy of default. A temporary that ends exactly where the chain's next approved row starts is the promo row alone: that row already ends it. Pair edits, selection and submission cannot orphan a companion.

**Source:** §5 temporary pairs.

#### D-392 [H] Publish changes is a selected book batch

List all draft book rows with full predecessor, proposed content and impact, pre-select all, then submit the operator-selected atomic rows and optional common_effective_date as one price_rows unit; a ticked half of a temporary pair brings its partner (D-405). Book money and plan structure remain independently approved. A rejected revision cannot undo an approved repricing that also affects the old revision.

**Source:** §2 decision 7; §6; §8.

#### D-393 [H] One unit engine, quorum and generations

bss-approval owns the shared engine shape; pricing owns prefixed tables and subjects. Quorum is tenant policy with kind overrides and fail-safe one when * is absent. No materiality. Submitter and all item authors are excluded from approval; an item's author is its row's creator, the only principal who may edit it (D-404). Votes carry generation; drift commits refreshed items/snapshot/hash, increments generation and marks prior decisions stale (UNIT_STALE). Conditional unit version yields UNIT_CONTENDED on lost races. Quorum zero, reject and withdraw all write terminal audit and ApprovalUnitDecided. See ADR-0003.

**Source:** §2 decision 8; §2.2; §6.

#### D-394 [H] Plans are versioned structure bound to one book

Phase 3 publishes immutable revisions containing one book, paid/optional/included items, minimal Grants and optional sold-as bundle. Validate recurring frequency, meter uniqueness, usage-only included quantities, SKU lifecycle, foreign-book prices, book validity and coverage per value. blocked_by is computed from pending row units. Clone makes a draft; publishing a revision never moves existing pins.

**Source:** §5 plans; §6; §8.

#### D-395 [H] Promotions are versioned and migrations are requests

Phase 3 forbids overlapping plan promotions; [from_date, to_date) applies by period start. Approved edits increment promotion version and bindings pin id/version. An approved migration to a published revision persists its preview and emits SubscriptionMigrationRequested; Subscriptions executes movement and confirms retirement in its separate plan. Pricing never reports a request as an executed move.

**Source:** §5; §6; §11 phase 3.

#### D-396 [H] One replay store and optimistic conditional writes

All pricing POSTs require Idempotency-Key. The 24-hour store is keyed by tenant, concrete endpoint and client_key; payload hash guards replay. The hash is over canonical JSON (object keys sorted at every depth), so the order a client sends keys in never turns a retry into IDEMPOTENCY_CONFLICT. Check it before reservations or unit work and claim/respond in the mutation transaction. Approval units have no idempotency column, despite the superseded §6 sketch. If-Match protects PATCH/PUT; conditional pending ownership and unit versions replace row locks. Persist audit and domain events with the mutation.

**Source:** §2.2; §3 items 23 and 27; §7.2.

#### D-397 [H] Consumer pins replace cohorts and catalog versions

Phase 4 resolve returns a full per-item chain matrix, versioned descriptors and promotion inputs, never totals. Renewals walk all successors from their pin and stop before the first new row; signup selects the in-force row. Approved rows remain readable by id forever, including keep_for_bound predecessors. Usage binds lazily per value and slices at row boundaries. Quote is the Studio totals preview, not the consumer contract.

**Source:** §2 decision 6; §7.1; §12.

#### D-398 [H] Reference reservation closes the lifecycle race

Before creating a price, reserve in Products, re-read the SKU, then write the object and receipt durably before confirming. Reserve/fence guards share the Products database. Never release because confirm timed out. Registry outage before the price write is REGISTRY_UNAVAILABLE and writes no price. Only price references are built in phase 2; plan_item and sold_as follow in phase 3. See ADR-0004; bookkeeping: see D-401. The earlier pending_release wording is superseded by D-401.

**Source:** §2 decision 17; §13.

#### D-399 [H] No SkuChanged listener or local SKU cache in phase 2

Pricing reads bss_products_sdk::ProductsClient at write time for type and lifecycle, including a re-read after reservation. Resolve binds descriptors from versions?as_of in phase 4. Phase 2 deliberately does not subscribe to SkuChanged or keep a local SKU read model. Add a cache only after measurement justifies its consistency and operational cost. This supersedes the earlier listener wording for this phase.

**Source:** Phase 2 plan, Global Constraints; deviation from spec §7.3 and §11.

#### D-400 [H] Toolkit outbox and broker TypedEvent own event delivery

Retire the gear-authored pricing_outbox without a relay in phase 2b. The new chain uses toolkit outbox migrations with prefix bss_pricing_outbox; writers accept the same scoped transaction as the state change. Events implement broker TypedEvent and retain the envelope-encoded interim sink pattern proved by Products. Only committed rows dispatch. Core payloads are PriceRowsPublished, ApprovalUnitDecided and PriceReferenceLost; phase 3 adds plan, promotion and migration events.

**Source:** Phase 2 plan Task 2a.2 and 2c.8; Products phase 1 pattern.

#### D-401 [H] Reference work is a durable op written before reserve

**Status:** DECIDED 2026-09-25.

Tx A claims the Idempotency-Key, mints price_id and inserts pricing_reference_op with kind create_price and state reserving before reserve. Reserve is idempotent per (owner, kind, ref_id). Re-read the SKU after reserve; Tx B inserts the price with reservation_id and reference_state = confirmation_pending and moves the op to written. Confirm succeeds before Tx C sets the price to confirmed, the op to done and answers the key. A refusal after reserve moves the op to cancelling, then release, then done. A door that gets no definite answer before the write (the reserve, or the SKU re-read after it) moves the op to cancelling, releases the key claim and answers 503: a 503 never becomes a price, and the cancellation releases any receipt. The ticker never makes a first reservation for a user: it cancels a create still reserving without a receipt, and completes one holding a receipt only when its door is gone. Delete removes the price and inserts a delete_price op in releasing in one transaction.

A ticker drives every op not done with bounded backoff and never drops one. It also reconciles confirmed prices through states(): a released reservation on a live price is re-reserved through a rereserve_price op when the SKU is not fenced; otherwise the price becomes lost, new rows fail PRICE_REFERENCE_LOST and PriceReferenceLost is emitted. A reservation released before its confirm follows the same rule: Tx C keeps the price confirmation_pending and starts a rereserve_price op, so a create is never answered lost. Only a reservation refused because the SKU is fenced, retiring or retired makes a price lost; any other refusal of a re-reservation is retried. Reconciliation also scans lost prices and re-reserves those whose SKU admits a reservation again (a lifted fence). Never release because a confirm timed out. An op has no foreign key to the price and outlives removal. This supersedes D-398's earlier bookkeeping.

**Source:** Phase 2 plan 2c.1/2c.6; plan review findings 2, 3.

#### D-402 [H] The pair guard compares SKU metering as of each row's start

**Status:** DECIDED 2026-09-25.

On a usage chain the successor keeps model, package_size and the SKU's (unit, usage_type_ref) read from the SKU version in force at each row's effective_from; otherwise CHAIN_MODEL_CHANGED. Products freezes the SKU type while referenced but versions its metering, hence the dated read. The meter is included because the supersession-continuity family (spec §5 "asserts exactly this") rejects a meter change. Submit checks the guard and apply rechecks it. A row that starts before the SKU's first version is compared with that first version's metering, never with "no metering". A Products refusal of the dated read (for example 403 for a caller without SKU read) reaches the caller with its own status and code; only unavailability (5xx, timeout, rate limit, a lost race) is 503 REGISTRY_UNAVAILABLE.

**Source:** Phase 2 reconciliation matrix row 24; spec §5 pair guard; supersession-continuity family.

#### D-403 [M] Validation refusals are 400 with a code; no wire 422

**Status:** DECIDED 2026-09-25.

The toolkit's canonical errors have no 422: InvalidArgument and FailedPrecondition both answer 400, and Products phase 1 already answers its failed submit checks with 400. Pricing keeps its route census rule that no operation declares a 422. Therefore CHAIN_MODEL_CHANGED, PAIR_SPLIT and every other pure-rule refusal at a door or at submit are 400 with their stable code in the problem body, and no approval unit is created. Conflicts stay 409 (ROW_LOCKED_PENDING, ROW_NOT_DRAFT, PRICE_REFERENCE_LOST, UNIT_CONTENDED, APPLY_REFUSED). The spec's "422 at submit" wording in §2 decision 16 and the §6 trait comment are superseded by this entry.

**Source:** toolkit canonical error mapping; Products phase 1 behaviour; deviation from spec §2 decision 16 and §6.

#### D-404 [H] A draft belongs to its author

**Status:** DECIDED 2026-09-25.

A draft belongs to its author: only its creator edits or deletes it. PATCH or DELETE of a draft price row by anyone but its created_by is 403 NOT_DRAFT_AUTHOR, and a temporary pair's partner follows its creator. Separation of duties (D-393) excludes the submitter and every item's created_by; because no one else can change a draft, every number in a unit is its item author's, and an editor can never approve money they wrote under another author's name. Products applies the same rule to SKU drafts. Another author proposes a different number with a draft of their own.

**Source:** Phase 2 review (chains MEDIUM-1, docs F2); spec §6 "author ≠ approver" (finding 8).

#### D-405 [M] Publish changes completes a selected pair

**Status:** DECIDED 2026-09-25.

POST /price-books/{id}/publish-changes that ticks one half of a temporary pair adds the other half to the unit and records it in the snapshot's added_partner: a pair is never split, and an untick of one half is not refused. Submitting one half alone through POST /rows/{id}/submit stays 400 PAIR_SPLIT, and the subject's submit validation still refuses a partial pair (PAIR_SPLIT) as a safety net. A foreign-book row is refused ROW_NOT_IN_BOOK with no unit.

**Source:** Phase 2 plan and reconciliation row 23; Phase 2 review (docs F4).
