# Pricing — Decision Register (PriceBook)

**Replaces:** the charge-line/market-price register on `bss/products-backup` (`3a38f0b28`), ending at D-383.
Historical decisions remain there and in git history; only living rules restated below govern the new model.
**Authority:** `docs/superpowers/specs/2026-09-24-pricebook-model-design.md`, especially §2, §2.2 and §13,
then this register, then code, then descriptive prose. D-399 is the explicit phase-plan deviation; D-403 overrides
the spec's 422 wording; D-407 (items as a sub-resource) and D-413 (a copied reference attaches after its write) are
the phase 3 plan's deviations; the owner defers promotions (D-409), migration requests and retirement (D-410),
and the sold-as bundle and grants (D-411), and drops quote and the Studio wiring (D-415).
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
| D-386 | H | SKU type defines the entry key and allowed models | DECIDED 2026-09-25 · §2 decision 5; §5 |
| D-387 | H | Tier bands are half-open | DECIDED 2026-09-25 · §5 tier bands; §10 |
| D-388 | H | Minimum fee is per price per subscription per period | DECIDED 2026-09-25 · §2 decision 13; §5 |
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
| D-402 | H | The pair guard compares SKU metering as of each price's start | DECIDED 2026-09-25 · Phase 2 reconciliation matrix row 24; spec §5 pair guard; supersession-continuity family |
| D-403 | M | Validation refusals are 400 with a code; no wire 422 | DECIDED 2026-09-25 · toolkit canonical error mapping; Products phase 1 behaviour; deviation from spec §2 decision 16 and §6 |
| D-404 | H | A draft belongs to its author | DECIDED 2026-09-25 · Phase 2 review (chains MEDIUM-1, docs F2); spec §6 "author ≠ approver" (finding 8) |
| D-405 | M | Publish changes completes a selected pair | DECIDED 2026-09-25 · Phase 2 plan and reconciliation row 23; Phase 2 review (docs F4) |
| D-406 | H | A temporary window is not crossed | DECIDED 2026-09-25 · Phase 2 second review (behaviour MEDIUM-2) |
| D-407 | H | Plan items are reserved references; items are a sub-resource | DECIDED 2026-09-25 · Phase 3 plan rev 2; plan review HIGH 2; owner, 2026-09-25 (kind plan_item only); deviation from spec §7.2 |
| D-408 | H | Plan checks read every SKU fresh; descriptors are information, never content | DECIDED 2026-09-25 · Phase 3 plan rev 2; plan review MEDIUM 6; phase 2 "owed to phase 3" |
| D-409 | H | Promotions are deferred (owner, 2026-09-25) | DECIDED 2026-09-25 · Owner, 2026-09-25, during Run 3.1; Phase 3 plan rev 3; spec §2.4 |
| D-410 | H | Migration requests and plan retirement are deferred (owner, 2026-09-25) | DECIDED 2026-09-25 · Owner, 2026-09-25, during Run 3.1; spec §11 phase 3 |
| D-411 | H | The sold-as bundle and plan grants are deferred (owner, 2026-09-25) | DECIDED 2026-09-25 · Owner, 2026-09-25, during Run 3.1; spec §5 plan_revision |
| D-412 | M | The phase 2 schema is edited in place until the first deployment | DECIDED 2026-09-25 · Phase 3 plan rev 2; plan review LOW 15 |
| D-413 | H | A copied item attaches its reference after the write | DECIDED 2026-09-25 · Phase 3 plan rev 2; plan review HIGH 1; deviation from D-401's reserve before write |
| D-414 | H | A revision's references outlive it | DECIDED 2026-09-25 · Phase 3 plan rev 2; plan review LOW 14 |
| D-415 | H | Quote is not built; the Studio is not wired to the API (owner, 2026-09-25) | DECIDED 2026-09-25 · Owner, 2026-09-25, during Run 3.1; deviation from spec §7.1 and §11 phase 4 |
| D-416 | H | Descriptors are read best-effort; the reads a rule needs stay hard | DECIDED 2026-09-26 · Phase 3 review, fix run 7 (plans F2, surface S-1, docs F1 and F2) |
| D-417 | M | The last revision of a never-published plan takes the plan with it | DECIDED 2026-09-26 · Phase 3 review, fix run 7 (plans F1, surface S-2) |
| D-418 | H | A plan revision is submitted under plan:submit | DECIDED 2026-09-26 · Phase 3 review, fix run 7 (surface S-4); corrects the run 3.4 brief |

## Entries

#### D-384 [H] Books own currency and validity

One currency book owns its SKU × charge kind × period entries. Book code is unique within the tenant. Revisions select a book; every plan reading it shares its money. A plan-specific exception uses another book or SKU, not variant. Book export is a single read-only JSON GET (decision 16). See ADR-0001.

**Source:** §2 decision 5; §5; §8.

#### D-385 [H] One registered dimension, independent value chains

A tenant-level dimension_key registry, initially region, supplies values. Each entry selects at most one key; null dim_value is the default chain. Resolve chooses an in-force value price before default. A default is optional and coverage is judged per value. A value with prices cannot be removed; dimension_key changes only while no price has a value. See ADR-0002.

**Source:** §2 decision 4; §5; §14.

#### D-386 [H] SKU type defines the entry key and allowed models

The book key is (sku_id, charge_kind, normalized period); there is no plan, phase, variant, cohort, region or currency axis within that key. Recurring uses month/year and flat or per_unit; one_time has no period and flat or per_unit; usage has no period and per_unit, graduated, volume or package. Bundle SKUs cannot be priced. A mismatch is CHARGE_KIND_SKU_TYPE.

**Source:** §2 decision 5; §5.

#### D-387 [H] Tier bands are half-open

All tier bands use [from, to). Quantity 1000 belongs to the band beginning at 1000, including volume cliffs. Correct the prototype comparison qty <= upTo when porting; the surviving tier-boundary goldens are the arithmetic oracle. Do not copy the prototype boundary bug.

**Source:** §5 tier bands; §10.

#### D-388 [H] Minimum fee is per price per subscription per period

Aggregate every bound value and slice rated by the same price, deduct included quantities, then apply its floor prorated by the fraction of the period covered, before promotions. Two values sharing a default price share one floor. Separate valued prices carry separate floors. No plan cap or plan minimum survives. Pricing stores and validates min_fee; Rating applies the floor (D-415).

**Source:** §2 decision 13; §5.

#### D-389 [H] Descriptors bind from durable SKU versions

Prices do not freeze GL, tax, invoice descriptors, metering or timing. Consumers bind the SKU version in force at the period start via versions?as_of; equal effective dates choose the highest published_version. An earlier pin keeps its descriptors forever. A descriptor change creates no refreeze price or pricing approval unit.

**Source:** §2 decision 14; §2.2; §7.1.

#### D-390 [H] Windows normalize per chain and preserve usage structure

On approval, sort approved prices within each (price_book_entry_id, dim_value), set predecessor effective_to to successor effective_from, and enforce one approved start per chain. The default tail stays open; a value tail may explicitly end and resume default fallback. Re-read chains transactionally, with serializable Postgres isolation. Usage successors cannot change model kind, package size or SKU metering as of each price's start (D-402): CHAIN_MODEL_CHANGED at submit, revalidated at apply.

**Source:** §2 decision 16; §5.

#### D-391 [H] Temporary changes keep pair identity or resume fallback

On an existing chain, temporary_until creates a promo price and a return price in one unit. The return copies the money versionAt would apply at the end and inherits dim_value. Common-date shifts preserve duration; a shift that would carry a temporary price across the start of another price of its chain, where apply would cut it short, is refused TEMPORARY_SPANS_A_CHANGE (D-406). A return to an explicitly closed price (a temporary nested in a value's closed price) ends explicitly at that price's end, which a common-date shift does not move, so the value falls back to the default after it. Submit and apply re-derive that copy from the approved chain as it stands, on the shifted end: a return that no longer names the price in force there, or no longer carries its model, price and min_fee, and a single closed price whose own chain now has a price in force on its end, are refused PAIR_RETURN_STALE (400 at submit, APPLY_REFUSED at apply); the author re-drafts. When the price in force on that end is itself a price of the same unit, the return is right only if that price is another pair's return naming the same restored price with the same model, price and min_fee (two pairs on one chain, both drafted against it); any other price of the unit there makes it stale. A value with no own chain gets one closed price and no synthetic return copy of default. A temporary that ends exactly where the chain's next approved price starts is the promo price alone: that price already ends it. Pair edits, selection and submission cannot orphan a companion.

**Source:** §5 temporary pairs.

#### D-392 [H] Publish changes is a selected book batch

List all draft book prices with full predecessor, proposed content and impact, pre-select all, then submit the operator-selected atomic prices and optional common_effective_date as one prices unit; a ticked half of a temporary pair brings its partner (D-405). Book money and plan structure remain independently approved. A rejected revision cannot undo an approved repricing that also affects the old revision.

**Source:** §2 decision 7; §6; §8.

#### D-393 [H] One unit engine, quorum and generations

bss-approval owns the shared engine shape; pricing owns prefixed tables and subjects. Quorum is tenant policy with kind overrides and fail-safe one when * is absent. No materiality. Submitter and all item authors are excluded from approval; an item's author is its price's creator, the only principal who may edit it (D-404). Votes carry generation; drift commits refreshed items/snapshot/hash, increments generation and marks prior decisions stale (UNIT_STALE). Conditional unit version yields UNIT_CONTENDED on lost races. Quorum zero, reject and withdraw all write terminal audit and ApprovalUnitDecided. See ADR-0003.

**Source:** §2 decision 8; §2.2; §6.

#### D-394 [H] Plans are versioned structure bound to one book

Phase 3 publishes immutable revisions containing one book, paid/optional/included items, minimal Grants and optional sold-as bundle. Validate recurring frequency, meter uniqueness, usage-only included quantities, SKU lifecycle, foreign-book entries, book validity and coverage per value. blocked_by is computed from pending price units. Clone makes a draft; publishing a revision never moves existing pins. Grants and the sold-as bundle are deferred (D-411), and so is retirement (D-410).

**Source:** §5 plans; §6; §8.

#### D-395 [H] Promotions are versioned and migrations are requests

Phase 3 forbids overlapping plan promotions; [from_date, to_date) applies by period start. Approved edits increment promotion version and bindings pin id/version. An approved migration to a published revision persists its preview and emits SubscriptionMigrationRequested; Subscriptions executes movement and confirms retirement in its separate plan. Pricing never reports a request as an executed move. Both halves are deferred by the owner: promotions (D-409) and migration requests (D-410).

**Source:** §5; §6; §11 phase 3.

#### D-396 [H] One replay store and optimistic conditional writes

All pricing POSTs require Idempotency-Key. The 24-hour store is keyed by tenant, concrete endpoint and client_key; payload hash guards replay. The hash is over canonical JSON (object keys sorted at every depth), so the order a client sends keys in never turns a retry into IDEMPOTENCY_CONFLICT. Check it before reservations or unit work and claim/respond in the mutation transaction. Approval units have no idempotency column, despite the superseded §6 sketch. If-Match protects PATCH/PUT; conditional pending ownership and unit versions replace row locks. Persist audit and domain events with the mutation.

**Source:** §2.2; §3 items 23 and 27; §7.2.

#### D-397 [H] Consumer pins replace cohorts and catalog versions

Phase 4 resolve returns a full per-item chain matrix, versioned descriptors and promotion inputs (deferred with promotions, D-409), never totals. Renewals walk all successors from their pin and stop before the first new price; signup selects the in-force price. Approved prices remain readable by id forever, including keep_for_bound predecessors. Usage binds lazily per value and slices at price boundaries. Quote is the Studio totals preview, not the consumer contract, and it is not built (D-415).

**Source:** §2 decision 6; §7.1; §12.

#### D-398 [H] Reference reservation closes the lifecycle race

Before creating an entry, reserve in Products, re-read the SKU, then write the object and receipt durably before confirming. Reserve/fence guards share the Products database. Never release because confirm timed out. Registry outage before the entry write is REGISTRY_UNAVAILABLE and writes no entry. Only entry references are built in phase 2; plan_item follows in phase 3, and sold_as is deferred (D-411). See ADR-0004; bookkeeping: see D-401. The earlier pending_release wording is superseded by D-401.

**Source:** §2 decision 17; §13.

#### D-399 [H] No SkuChanged listener or local SKU cache in phase 2

Pricing reads bss_products_sdk::ProductsClient at write time for type and lifecycle, including a re-read after reservation. Resolve binds descriptors from versions?as_of in phase 4. Phase 2 deliberately does not subscribe to SkuChanged or keep a local SKU read model. Add a cache only after measurement justifies its consistency and operational cost. This supersedes the earlier listener wording for this phase.

**Source:** Phase 2 plan, Global Constraints; deviation from spec §7.3 and §11.

#### D-400 [H] Toolkit outbox and broker TypedEvent own event delivery

Retire the gear-authored pricing_outbox without a relay in phase 2b. The new chain uses toolkit outbox migrations with prefix bss_pricing_outbox; writers accept the same scoped transaction as the state change. Events implement broker TypedEvent and retain the envelope-encoded interim sink pattern proved by Products. Only committed outbox rows dispatch. Core payloads are PricesPublished, ApprovalUnitDecided and PriceBookEntryReferenceLost; phase 3 adds plan events (promotion events are deferred, D-409; migration and retirement events, D-410).

**Source:** Phase 2 plan Task 2a.2 and 2c.8; Products phase 1 pattern.

#### D-401 [H] Reference work is a durable op written before reserve

**Status:** DECIDED 2026-09-25.

Tx A claims the Idempotency-Key, mints price_book_entry_id and inserts pricing_reference_op with kind create_entry and state reserving before reserve. Reserve is idempotent per (owner, kind, ref_id). Re-read the SKU after reserve; Tx B inserts the entry with reservation_id and reference_state = confirmation_pending and moves the op to written. Confirm succeeds before Tx C sets the entry to confirmed, the op to done and answers the key. A refusal after reserve moves the op to cancelling, then release, then done. A door that gets no definite answer before the write (the reserve, or the SKU re-read after it) moves the op to cancelling, releases the key claim and answers 503: a 503 never becomes an entry, and the cancellation releases any receipt. Any other error that ends the door's drive after its reserve and before its write (409 CONTENDED, a 500) cancels the create the same way before it is answered, so no answered error becomes an entry later; only a failed cancellation leaves the op to the ticker. The ticker never makes a first reservation for a user: it cancels a create still reserving without a receipt, and completes one holding a receipt only when its door is gone. Delete removes the entry and inserts a delete_entry op in releasing in one transaction.

A ticker drives every op not done with bounded backoff and never drops one. It also reconciles confirmed entries through states(): a released reservation on a live entry is re-reserved through a rereserve_entry op when the SKU is not fenced; otherwise the entry becomes lost, new prices fail ENTRY_REFERENCE_LOST and PriceBookEntryReferenceLost is emitted. A reservation released before its confirm follows the same rule: Tx C keeps the entry confirmation_pending and starts a rereserve_entry op, so a create is never answered lost. A reservation Products answers 404 for (it does not know the id, for example after a restore from an older backup) is treated as released: at confirm it takes this rereserve path, and at release it counts as released. Only a reservation refused because the SKU is fenced, retiring or retired makes an entry lost; any other refusal of a re-reservation is retried. Reconciliation also scans lost entries and re-reserves those whose SKU admits a reservation again (a lifted fence). Never release because a confirm timed out. An op has no foreign key to the entry and outlives removal. This supersedes D-398's earlier bookkeeping. Phase 3 renames the op kinds create, delete and rereserve, adds attach (D-413), and has an op name its reference as (ref_kind, ref_id), so plan items use the same machine (D-407, D-412).

**Source:** Phase 2 plan 2c.1/2c.6; plan review findings 2, 3.

#### D-402 [H] The pair guard compares SKU metering as of each price's start

**Status:** DECIDED 2026-09-25.

On a usage chain the successor keeps model, package_size and the SKU's (unit, usage_type_ref) read from the SKU version in force at each price's effective_from; otherwise CHAIN_MODEL_CHANGED. Products freezes the SKU type while referenced but versions its metering, hence the dated read. The meter is included because the supersession-continuity family (spec §5 "asserts exactly this") rejects a meter change. Submit checks the guard and apply rechecks it. A price that starts before the SKU's first version is compared with that first version's metering, never with "no metering". A Products refusal of the dated read (for example 403 for a caller without SKU read) reaches the caller with its own status and code; only unavailability (5xx, timeout, rate limit, a lost race) is 503 REGISTRY_UNAVAILABLE.

**Source:** Phase 2 reconciliation matrix row 24; spec §5 pair guard; supersession-continuity family.

#### D-403 [M] Validation refusals are 400 with a code; no wire 422

**Status:** DECIDED 2026-09-25.

The toolkit's canonical errors have no 422: InvalidArgument and FailedPrecondition both answer 400, and Products phase 1 already answers its failed submit checks with 400. Pricing keeps its route census rule that no operation declares a 422. Therefore CHAIN_MODEL_CHANGED, PAIR_SPLIT and every other pure-rule refusal at a door or at submit are 400 with their stable code in the problem body, and no approval unit is created. Conflicts stay 409 (PRICE_LOCKED_PENDING, PRICE_NOT_DRAFT, ENTRY_REFERENCE_LOST, UNIT_CONTENDED, APPLY_REFUSED). The spec's "422 at submit" wording in §2 decision 16 and the §6 trait comment are superseded by this entry.

**Source:** toolkit canonical error mapping; Products phase 1 behaviour; deviation from spec §2 decision 16 and §6.

#### D-404 [H] A draft belongs to its author

**Status:** DECIDED 2026-09-25.

A draft belongs to its author: only its creator edits or deletes it. PATCH or DELETE of a draft price by anyone but its created_by is 403 NOT_DRAFT_AUTHOR, and a temporary pair's partner follows its creator. The entry delete honours it: DELETE /price-book-entries/{id} is 403 NOT_DRAFT_AUTHOR, naming the first such price, while a draft price of the entry was created by someone other than the caller; rejected prices are history and do not block. Separation of duties (D-393) excludes the submitter and every item's created_by; because no one else can change a draft, every number in a unit is its item author's, and an editor can never approve money they wrote under another author's name. Products applies the same rule to SKU drafts. Another author proposes a different number with a draft of their own.

**Source:** Phase 2 review (chains MEDIUM-1, docs F2); spec §6 "author ≠ approver" (finding 8).

#### D-405 [M] Publish changes completes a selected pair

**Status:** DECIDED 2026-09-25.

POST /price-books/{id}/publish-changes that ticks one half of a temporary pair adds the other half to the unit and records it in the snapshot's added_partner: a pair is never split, and an untick of one half is not refused. Submitting one half alone through POST /prices/{id}/submit stays 400 PAIR_SPLIT, and the subject's submit validation still refuses a partial pair (PAIR_SPLIT) as a safety net. A foreign-book price is refused PRICE_NOT_IN_BOOK with no unit.

**Source:** Phase 2 plan and reconciliation row 23; Phase 2 review (docs F4).

#### D-406 [H] A temporary window is not crossed

**Status:** DECIDED 2026-09-25.

On one chain (entry, dim_value), a proposed price that is neither temporary nor a pair's return may not start inside [effective_from, temporary_until) of an approved temporary price or of a temporary price in the same unit: the promo's end (its return, or its closed end) would undo it. It is refused 400 PRICE_INSIDE_TEMPORARY at the draft door (create, and a PATCH that moves the start), at submit, and at apply as APPLY_REFUSED. A temporary price whose window (effective_from, temporary_until) strictly contains the start of an approved price of its chain, or of a price in the same unit, is refused 400 TEMPORARY_SPANS_A_CHANGE at the draft door, at submit after a common-date shift, and at apply: normalisation would cut the promo at that start. A price that starts exactly on temporary_until is allowed, because it ends the promo. A nested pair's return belongs to its pair and is not refused by the first rule. The alternative, a price scheduled inside a promo that takes effect at the promo's end, needs a return re-derived after approval; it is left to the owner.

**Source:** Phase 2 second review (behaviour MEDIUM-2).

#### D-407 [H] Plan items are reserved references; items are a sub-resource

**Status:** DECIDED 2026-09-25.

An item added by POST /plan-revisions/{id}/items reserves kind plan_item with ref_id = the item id, through phase 2's create op (D-401: the op before the reserve, the SKU re-read, the write, the confirm; a 503 writes nothing). plan_item is the only new reference kind of phase 3: the sold_as kind waits with the sold-as bundle (D-411). Items are POST /plan-revisions/{id}/items and PATCH or DELETE /plan-items/{id}, not a list inside PATCH /plan-revisions/{id}. This deviates from spec §7.2 on purpose: each added or removed item is one op with its own Idempotency-Key and its own recovery.

**Source:** Phase 3 plan rev 2; plan review HIGH 2; owner, 2026-09-25 (kind plan_item only); deviation from spec §7.2 (items inside the revision PATCH).

#### D-408 [H] Plan checks read every SKU fresh; descriptors are information, never content

**Status:** DECIDED 2026-09-25.

Checks, submit and apply read each item's SKU through sku_for_write (D-399). A deprecated SKU may stay in a new revision of the SAME plan that carries it over from the published revision; it cannot be added, and a clone (a new plan) that carries it is red (ITEM_SKU_DEPRECATED). A draft, retiring or retired SKU is red (ITEM_SKU_UNAVAILABLE). A bundle SKU cannot be an item (ITEM_BUNDLE_SKU). An entry that a plan item names cannot be deleted (ENTRY_IN_USE). The approval snapshots of revisions and of prices carry each SKU's current descriptors for the reviewer, OUTSIDE the fingerprinted after: a GL change must not refresh every pending unit. They are gathered in collect or validate_submit and cached on the subject, because snapshot is synchronous. Being information, their read is best-effort and never refuses a submit, a vote or a reject (D-416).

**Source:** Phase 3 plan rev 2; plan review MEDIUM 6; phase 2 "owed to phase 3" (fresh SKU reads, current descriptors in snapshots).

#### D-409 [H] Promotions are deferred (owner, 2026-09-25)

**Status:** DECIDED 2026-09-25.

The owner took promotions out of phase 3: there are no promotion tables, doors, promotion approval kind or PromotionPublished event until the owner brings them back. resolve's "active promotion (id, version)" is deferred with them (spec §2.4). The promotion DoDs in features/promotions-migrations.md stay unticked, each marked deferred by the owner; where DESIGN, slice 06 or the features describe promotion routes or tables, the text stays and is marked deferred. The promotion half of D-395 waits with them. The phase 3 plan's earlier shape for promotions (one identity row, one row per version, overlap against each other promotion's current approved and open pending version, end-today and cancel as new versions) is kept in the plan's revision 2 for their return.

**Source:** Owner, 2026-09-25, during Run 3.1; Phase 3 plan rev 3; spec §2.4. Supersedes the Phase 3 plan rev 2 D-409 (versioned promotion rows).

#### D-410 [H] Migration requests and plan retirement are deferred (owner, 2026-09-25)

**Status:** DECIDED 2026-09-25.

The owner took migration requests, their preview and plan retirement out of phase 3: there is no migration-request table, no POST /plans/{id}/migrations or /retire door, no migration approval kind, no retiring plan state, and no SubscriptionMigrationRequested or PlanRetired event until the owner brings them back. The migration and retirement DoDs stay unticked, each marked deferred by the owner; where the DESIGN, the PRD, slices 04 and 06 or the features describe these routes, tables or events, the text stays and is marked deferred. The migration half of D-395 and the retirement prerequisite of D-394 wait with them. The phase 3 plan's rev 2 shape (caller-supplied subscription periods, a request that moves nothing in pricing, retirement as a request against another plan) is kept in the plan for their return.

**Source:** Owner, 2026-09-25, during Run 3.1; spec §11 phase 3. Supersedes the Phase 3 plan rev 2 D-410 (caller-supplied periods) and D-411 (retirement's request half).

#### D-411 [H] The sold-as bundle and plan grants are deferred (owner, 2026-09-25)

**Status:** DECIDED 2026-09-25.

The owner took the sold-as bundle SKU and the revision's grants out of phase 3: pricing_plan carries no bundle_sku_id and no bundle unique index, pricing_plan_revision carries no grants, bundle_sku_id or sold_as reference columns, the BUNDLE_SKU check is not built, and the sold_as reference kind waits with them. A bundle SKU still cannot be an item (ITEM_BUNDLE_SKU). Where the DESIGN, the PRD, slice 04 or the plans feature describe sold-as or grants, the text stays and is marked deferred. The minimal Grants and optional sold-as bundle of D-394 wait with them.

**Source:** Owner, 2026-09-25, during Run 3.1; spec §5 plan_revision.

#### D-412 [M] The phase 2 schema is edited in place until the first deployment

**Status:** DECIDED 2026-09-25.

The chain has never been deployed. This phase edits m20260926_000006 (reference ops) in place in Run 3.2 and adds m20260926_000010 onward. A database migrated before keeps the old shape silently (CREATE … IF NOT EXISTS); the phase 4 legacy-history guard is the protection.

**Source:** Phase 3 plan rev 2; plan review LOW 15.

#### D-413 [H] A copied item attaches its reference after the write

**Status:** DECIDED 2026-09-25.

POST /plans/{id}/revisions (copy) and POST /plans/{id}/clone write the revision and every copied item in ONE transaction with reference_state = unreserved and reservation_id NULL, plus one attach op per item. The attach op has the rereserve shape: reserve, then confirm; a refusal marks the item's reference lost. This deviates from "reserve before write" on purpose: the copied SKU is already protected by the SOURCE revision's live reference, which superseded and published revisions never release, so no retire or type fence can slip in between. Products admits a deprecated SKU for a reservation, so a carried-over deprecated SKU attaches. The door drives the attach ops best-effort and answers 201; the ticker finishes the rest. Submit and apply require every item reference to be confirmed, or confirmation_pending with a receipt; otherwise the checks show ITEM_REFERENCE_PENDING or ITEM_REFERENCE_LOST. POST …/items alone keeps D-401's create op.

**Source:** Phase 3 plan rev 2; plan review HIGH 1; deviation from D-401's reserve before write.

#### D-414 [H] A revision's references outlive it

**Status:** DECIDED 2026-09-25.

Items of published and superseded revisions keep their references, fail-safe, because a pinned subscription may still be rated on them. So a SKU ever sold in a revision cannot be retired until those references are released. The release waits until Subscriptions reports that no subscription pins the revision; that report is owed to the Subscriptions integration. The delete of a draft revision releases its items' references through delete ops.

**Source:** Phase 3 plan rev 2; plan review LOW 14.

#### D-415 [H] Quote is not built; the Studio is not wired to the API (owner, 2026-09-25)

**Status:** DECIDED 2026-09-25.

GET /pricing/v1/quote is not built in this programme, and the PriceBook Studio is not wired to the API; no Studio programme follows. Quote was the Studio's preview, not a consumer contract: consumers read resolve and GET /pricing/v1/prices/{id}. The minimum-fee floor arithmetic of D-388 belongs to Rating, with its acceptance example: two values rated 10 + 10 on one default price with min_fee 30 bill 30; two own prices with min_fee 30 each bill 60 (Rating T-D-38). cpt-cf-bss-pricing-fr-min-fee stays: pricing stores and validates min_fee and resolve returns it. In the PRD, DESIGN, slice 07 and features/read-contract-events.md every quote statement stays and is marked not built (D-415); the quote and minimum-fee-floor DoDs stay unticked for that reason.

**Source:** Owner, 2026-09-25, during Run 3.1; deviation from spec §7.1 (GET /pricing/v1/quote) and §11 phase 4.

#### D-416 [H] Descriptors are read best-effort; the reads a rule needs stay hard

**Status:** DECIDED 2026-09-26.

The SKU descriptors an approval unit shows (D-408) are information, so their read is best-effort: when Products cannot answer it (503) or definitely refuses it (for example 403 for a caller without products read), the snapshot records "descriptors": "unavailable" and the submit, the vote or the reject goes on. The reads that feed a RULE stay hard and are made as the caller: the plan checks (GET /plan-revisions/{id}/checks, and the plan_revision subject's validate_submit and apply) and a usage chain's dated metering (the pair guard, D-402). There only unavailability is 503 REGISTRY_UNAVAILABLE, and a refusal keeps Products' own status and code on submit, approve and reject alike, never 400 REGISTRY_REFUSED. So an approve-only reviewer votes on any unit whose rules read no SKU and rejects any unit, while a rule read needs products read too: the plan_revision submitter and its final approver (apply re-runs the checks), readers of the checks, plan item authors (the item door and its create re-read), and the submitter and final approver of a prices unit on a usage chain.

**Source:** Phase 3 review, fix run 7 (plans F2, surface S-1, docs F1 and F2; controller notes 4 and 5).

#### D-417 [M] The last revision of a never-published plan takes the plan with it

**Status:** DECIDED 2026-09-26.

DELETE /plan-revisions/{id} of the last revision of a plan that was never published (published_rev IS NULL and no other revision) deletes the plan in the same transaction, with a plan.delete audit row, and frees its code; the door still answers 204. Without it such a plan was stranded: a copy answered PLAN_UNPUBLISHED, a clone CLONE_SOURCE_UNPUBLISHED, no door deletes a plan, and its code stayed taken. Nothing refers to a never-published plan: it has no published revision, no pin and no event, and an approval unit names a revision by ref_id without a foreign key. A plan with a published revision keeps its behaviour: deleting its draft leaves the plan, its code and its published revision as they are.

**Source:** Phase 3 review, fix run 7 (plans F1, surface S-2; controller note 1).

#### D-418 [H] A plan revision is submitted under plan:submit

**Status:** DECIDED 2026-09-26.

POST /plan-revisions/{id}/submit checks the plan label's own submit action, plan:submit: the label-specific analogue of price:submit on POST /prices/{id}/submit and of price_book:submit on publish-changes. The plan label's actions are read, author and submit. approval_unit:submit, which the run 3.4 brief bound to this door, stays the right to withdraw a unit; it no longer lets a prices submitter submit, or read through the receipt, a plan revision, so a tenant can withhold plan submission from its prices submitters.

**Source:** Phase 3 review, fix run 7 (surface S-4); corrects the run 3.4 brief.
