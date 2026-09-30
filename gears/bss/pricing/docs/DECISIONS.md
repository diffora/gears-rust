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
| D-386 | H | SKU type defines the entry key and allowed models | DECIDED 2026-09-25 · §2 decision 5; §5; amended by D-427 |
| D-387 | H | Tier bands are half-open | DECIDED 2026-09-25 · §5 tier bands; §10 |
| D-388 | H | Minimum fee is per price per subscription per period | DECIDED 2026-09-25 · §2 decision 13; §5; amended by D-467 |
| D-389 | H | Descriptors bind from durable SKU versions | DECIDED 2026-09-25 · §2 decision 14; §2.2; §7.1 |
| D-390 | H | Windows normalize per chain and preserve usage structure | DECIDED 2026-09-25 · §2 decision 16; §5 |
| D-391 | H | Temporary changes keep pair identity or resume fallback | DECIDED 2026-09-25 · §5 temporary pairs; amended by D-443 |
| D-392 | H | Publish changes is a selected book batch | DECIDED 2026-09-25 · §2 decision 7; §6; §8 |
| D-393 | H | One unit engine, quorum and generations | DECIDED 2026-09-25 · §2 decision 8; §2.2; §6 |
| D-394 | H | Plans are versioned structure bound to one book | DECIDED 2026-09-25 · §5 plans; §6; §8; amended by D-450, D-467 |
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
| D-407 | H | Plan items are reserved references; items are a sub-resource | DECIDED 2026-09-25 · Phase 3 plan rev 2; plan review HIGH 2; owner, 2026-09-25 (kind plan_item only); deviation from spec §7.2; amended by D-467 |
| D-408 | H | Plan checks read every SKU fresh; descriptors are information, never content | DECIDED 2026-09-25 · Phase 3 plan rev 2; plan review MEDIUM 6; phase 2 "owed to phase 3"; amended by D-453, D-465; extended by D-466 |
| D-409 | H | Promotions are deferred (owner, 2026-09-25) | DECIDED 2026-09-25 · Owner, 2026-09-25, during Run 3.1; Phase 3 plan rev 3; spec §2.4 |
| D-410 | H | Migration requests and plan retirement are deferred (owner, 2026-09-25) | DECIDED 2026-09-25 · Owner, 2026-09-25, during Run 3.1; spec §11 phase 3 |
| D-411 | H | The sold-as bundle and plan grants are deferred (owner, 2026-09-25) | DECIDED 2026-09-25 · Owner, 2026-09-25, during Run 3.1; spec §5 plan_revision |
| D-412 | M | The phase 2 schema is edited in place until the first deployment | DECIDED 2026-09-25 · Phase 3 plan rev 2; plan review LOW 15; closed by D-427 |
| D-413 | H | A copied item attaches its reference after the write | DECIDED 2026-09-25 · Phase 3 plan rev 2; plan review HIGH 1; deviation from D-401's reserve before write; amended by D-451, D-467 |
| D-414 | H | A revision's references outlive it | DECIDED 2026-09-25 · Phase 3 plan rev 2; plan review LOW 14 |
| D-415 | H | Quote is not built; the Studio is not wired to the API (owner, 2026-09-25) | DECIDED 2026-09-25 · Owner, 2026-09-25, during Run 3.1; deviation from spec §7.1 and §11 phase 4 |
| D-416 | H | Descriptors are read best-effort; the reads a rule needs stay hard | DECIDED 2026-09-26 · Phase 3 review, fix run 7 (plans F2, surface S-1, docs F1 and F2) |
| D-417 | M | The last revision of a never-published plan takes the plan with it | DECIDED 2026-09-26 · Phase 3 review, fix run 7 (plans F1, surface S-2) |
| D-418 | H | A plan revision is submitted under plan:submit | DECIDED 2026-09-26 · Phase 3 review, fix run 7 (surface S-4); corrects the run 3.4 brief |
| D-419 | H | Resolve answers one revision on one date, with the caller's pins | DECIDED 2026-09-26 · Phase 4 plan rev 3 (Run 4.2); spec §7.1; plan review M4 b, L4, L7; amended by D-454, D-467 |
| D-420 | H | The matrix and the walk: a binding is always in force | DECIDED 2026-09-26 · Phase 4 plan rev 3 (Run 4.2); owner, 2026-09-26 (no promotion rule; rule 4 as recommended); spec §2.4, §5, §7.1; plan review H2, M4, L1; amended by D-467 |
| D-421 | H | The binding carries resolved invoice inputs with their source | DECIDED 2026-09-26 · Phase 4 plan rev 3 (Run 4.2); PRD AC #13; plan review H3, L7; amended by D-467 |
| D-422 | H | The pinned price read serves approved money forever | DECIDED 2026-09-26 · Phase 4 plan rev 3 (Run 4.2); spec §7.1; plan review H4 |
| D-423 | H | Both gears refuse a legacy or stale schema at boot | DECIDED 2026-09-26 · Phase 4 plan rev 2 (Run 4.1); plan review H1, M1, M2, L6 |
| D-424 | H | Resolve reads SKU versions as pricing's system actor | DECIDED 2026-09-26 · Phase 4 review, fix run 8 (docs M1); amends D-421 |
| D-425 | H | The binding says where it ends for its holder | DECIDED 2026-09-26 · Phase 4 review, fix run 8 (contract C-1, docs M2); amends D-420 |
| D-426 | H | An entry's invoice line is locked once the entry carries money | DECIDED 2026-09-26 · Owner, 2026-09-26 (option 1 of three); amends D-421 |
| D-427 | H | The model belongs to the entry, fixed for its life, and is part of its key | DECIDED 2026-09-26 · Owner, 2026-09-26; phase 5 plan rev 2; closes D-412; amends D-386, D-390, D-391, D-401, D-402 |
| D-428 | H | Entries and SKUs report their usage | DECIDED 2026-09-26 · Owner, 2026-09-26; phase 5 plan rev 2; amended by D-440, D-453 |
| D-429 | M | The replay store's mechanics (twin of products P-D-198) | DECIDED 2026-09-27 · Carried from D-142 (backup `3a38f0b28`); decisions cleanup, owner 2026-09-27 |
| D-430 | M | A tier ladder's top band is open | DECIDED 2026-09-27 · Carried from D-17 (backup `3a38f0b28`); decisions cleanup, owner 2026-09-27 |
| D-431 | M | The request's correlation id is minted at the authoring edge | DECIDED 2026-09-27 · Carried from D-178 (backup `3a38f0b28`); decisions cleanup, owner 2026-09-27 |
| D-432 | M | If-Match on every write to a versioned row, and on a draft price's DELETE | DECIDED 2026-09-27 · Carried from D-141 (backup `3a38f0b28`); decisions cleanup, owner 2026-09-27; extends D-396 |
| D-433 | M | The audit log is append-only with a reserved sealing seam (twin of products P-D-200) | DECIDED 2026-09-27 · Carried from P-D-08, P-D-28, P-D-46, P-D-118 (backup `3a38f0b28`); decisions cleanup, owner 2026-09-27 |
| D-434 | M | Where a SKU is priced and sold: its entries across books, the plans that name it, one plan item | DECIDED 2026-09-27 · Owner, 2026-09-27; phase 6 plan rev 2; amended by D-440, D-453, D-460, D-461 |
| D-435 | M | An approval-policy override can be reset; the default cannot be deleted (twin of products P-D-216) | DECIDED 2026-09-27 · Owner, 2026-09-27; phase 6 plan rev 2 |
| D-436 | M | Dimension values edit one at a time and show their use | DECIDED 2026-09-27 · Owner, 2026-09-27; phase 6 plan rev 2 |
| D-437 | M | The default rounding is one of five modes; a tenant with no settings rounds half_even | DECIDED 2026-09-27 · Owner, 2026-09-27; phase 6 plan rev 2; the half_even default, Owner, 2026-09-28 |
| D-438 | M | The settings offer currencies and say who changed them | DECIDED 2026-09-27 · Owner, 2026-09-27; phase 6 plan rev 2 |
| D-439 | M | Closed sets are enums on the responses; requests keep strings and their codes (twin of products P-D-217) | DECIDED 2026-09-27 · Owner, 2026-09-27; phase 6 plan rev 2; amended by D-467 |
| D-440 | M | An entry's prices, its price in force and its approved prices by date | DECIDED 2026-09-28 · Owner, 2026-09-28; phase 7 plan rev 2; amends D-428, D-434; extended by D-456 |
| D-441 | M | Every book read carries its stats | DECIDED 2026-09-28 · Owner, 2026-09-28; phase 7 plan rev 2; amended by D-453 |
| D-442 | M | The book list pages on the toolkit's OData pager, searched by q and sku_id | DECIDED 2026-09-28 · Owner, 2026-09-28; phase 7 plan rev 2 |
| D-443 | M | A temporary draft's dates move, and its pair follows | DECIDED 2026-09-28 · Owner, 2026-09-28; phase 7 plan rev 2; amends D-391 |
| D-444 | M | A book has a description, and an unused book can be deleted | DECIDED 2026-09-28 · Owner, 2026-09-28; phase 7 plan rev 2; amended by D-453 |
| D-445 | L | An approval unit carries its submitter's note (twin of products P-D-219) | DECIDED 2026-09-28 · Owner, 2026-09-28; phase 7 plan rev 2; amended by D-464 |
| D-446 | M | A plan revision can be stored scheduled: the state, its index and its migration | DECIDED 2026-09-29 · Owner, 2026-09-28; phase 8 plan rev 2 |
| D-447 | M | A scheduled revision takes effect on its date: the effective state is derived | DECIDED 2026-09-29 · Owner, 2026-09-28; phase 8 plan rev 2 |
| D-448 | M | The storage writes of a scheduled revision: schedule, switch, unschedule and the due scan | DECIDED 2026-09-29 · Owner, 2026-09-28; phase 8 plan rev 2 |
| D-449 | M | An approval before the sale date schedules the revision | DECIDED 2026-09-29 · Owner, 2026-09-28; phase 8 plan rev 2 (run 8.2) |
| D-450 | M | The switch job persists a due switch on its date and announces it once | DECIDED 2026-09-29 · Owner, 2026-09-28; phase 8 plan rev 2 (run 8.2); plan review H1, M4, L2, L7 |
| D-451 | M | The copy, clone and unschedule doors catch a due switch up; one scheduled revision at a time | DECIDED 2026-09-29 · Owner, 2026-09-28; phase 8 plan rev 2 (run 8.2); plan review M1; amended by D-463 |
| D-452 | M | A scheduled revision can be withdrawn to a draft | DECIDED 2026-09-29 · Owner, 2026-09-28; phase 8 plan rev 2 (run 8.2); plan review M5 |
| D-453 | M | Every read derives the effective state; the counts read the stored state | DECIDED 2026-09-29 · Owner, 2026-09-28; phase 8 plan rev 2 (run 8.2); plan review M2, L5; amended by D-460, D-461 |
| D-454 | M | Resolve serves a scheduled revision from its sale date | DECIDED 2026-09-29 · Owner, 2026-09-28; phase 8 plan rev 2 (run 8.2); plan review M3 |
| D-455 | M | The outbox wakes its sequencer after the commit | DECIDED 2026-09-29 · Main sync of 2026-09-29 (toolkit-db 2bfc76aec); phase 8 plan rev 2 (run 8.2b) |
| D-456 | M | A plan names only a book its author may read | DECIDED 2026-09-29 · Whole-branch review PS-08 (fix run W1a); extends D-440; extended by D-463, D-468 |
| D-457 | M | Every text a request writes has an explicit length cap | DECIDED 2026-09-29 · Whole-branch review PS-09, PS-10, X-01 (fix run W1a); twin of a products decision in W1b; extended by D-468 |
| D-458 | M | The approval-unit list pages and reads its page set-based | DECIDED 2026-09-29 · Owner, 2026-09-29 (dispositions O2, "2"); whole-branch review PS-13 (fix run W1a) |
| D-459 | M | One approve-eligibility predicate for the engine and its readers | DECIDED 2026-09-30 · Phase 9 plan rev 2 (W2, binding; plan review W2, L5); extends D-393 |
| D-460 | M | The plans list names each plan's current revision and the one in effect | DECIDED 2026-09-30 · Owner, 2026-09-30 (validation 3 item 1); phase 9 plan rev 2 (decision 1; plan review M6, M7, W3, L4); amends D-434, D-453 |
| D-461 | M | A revision says who made it and when it was submitted and approved | DECIDED 2026-09-30 · Owner, 2026-09-30 (validation 3 item 7); phase 9 plan rev 2 (decision 2; plan review M6, M7, L1, L9); amends D-434, D-453 |
| D-462 | M | A pending revision shows its vote progress under plan read | DECIDED 2026-09-30 · Owner, 2026-09-30 (O-9a, "yes"); phase 9 plan rev 2 (decision 4; plan review M7, L5) |
| D-463 | M | A plan's sale date on create and clone | DECIDED 2026-09-30 · Owner, 2026-09-30 (validation 3 item 7); phase 9 plan rev 2 (decision 3; plan review L10); amends D-451, extends D-456 |
| D-464 | L | A plan submit and a publish-changes carry the submitter's note | DECIDED 2026-09-30 · Owner, 2026-09-30 (validation 3 item 6); phase 9 plan rev 2 (decision 5; plan review L3); amends D-445 |
| D-465 | M | A revision may carry again a deprecated SKU its plan sells | DECIDED 2026-09-30 · Owner, 2026-09-30 (O-9b, "yes"); phase 9 plan rev 2 (decision 6; plan review L2); amends D-408 |
| D-466 | M | Each check row names its items and its blocking prices | DECIDED 2026-09-30 · Owner, 2026-09-30 (validation 3 item 5); phase 9 plan rev 2 (decision 7); extends D-408 |
| D-467 | H | A plan item is a SKU and its entry: no treatment, no included quantity, no minimum quantity | DECIDED 2026-09-30 · Owner, 2026-09-30 (the included quantity, then the treatment, then qty_min removed); phase 9 plan rev 2 (run 9.2); amends D-388, D-394, D-407, D-413, D-419, D-420, D-421, D-439 |
| D-468 | M | A new plan's code follows a declared rule | DECIDED 2026-09-30 · Owner, 2026-09-30 (ask 39, "do it"); phase 9 run 9.2 scope addition; extends D-456, D-457 |

## Entries

#### D-384 [H] Books own currency and validity

One currency book owns its SKU × charge kind × period entries. Book code is unique within the tenant. Revisions select a book; every plan reading it shares its money. A plan-specific exception uses another book or SKU, not variant. Book export is a single read-only JSON GET (decision 16). See ADR-0001.

**Source:** §2 decision 5; §5; §8.

#### D-385 [H] One registered dimension, independent value chains

A tenant-level dimension_key registry, initially region, supplies values. Each entry selects at most one key; null dim_value is the default chain. Resolve chooses an in-force value price before default. A default is optional and coverage is judged per value. A value with prices cannot be removed; dimension_key changes only while no price has a value. See ADR-0002.

**Source:** §2 decision 4; §5; §14.

#### D-386 [H] SKU type defines the entry key and allowed models

The book key is (sku_id, charge_kind, normalized period, model); model joined it with D-427. There is no plan, phase, variant, cohort, region or currency axis within that key. Recurring uses month/year and flat or per_unit; one_time has no period and flat or per_unit; usage has no period and per_unit, graduated, volume or package. The model is the entry's, chosen at its create and fixed for its life (D-427); a model the charge kind does not allow is MODEL_KIND_CHARGEKIND_MISMATCH. Bundle SKUs cannot be priced. A charge kind that does not match the SKU type is CHARGE_KIND_SKU_TYPE.

**Source:** §2 decision 5; §5.

#### D-387 [H] Tier bands are half-open

All tier bands use [from, to). Quantity 1000 belongs to the band beginning at 1000, including volume cliffs. Correct the prototype comparison qty <= upTo when porting; the surviving tier-boundary goldens are the arithmetic oracle. Do not copy the prototype boundary bug.

**Source:** §5 tier bands; §10.

#### D-388 [H] Minimum fee is per price per subscription per period

Aggregate every bound value and slice rated by the same price, deduct included quantities, then apply its floor prorated by the fraction of the period covered, before promotions. Two values sharing a default price share one floor. Separate valued prices carry separate floors. No plan cap or plan minimum survives. Pricing stores and validates min_fee; Rating applies the floor (D-415).

D-467 amends this entry: no plan carries an included quantity any more, so no quantity is deducted before the floor; the floor and proration stay Rating's.

**Source:** §2 decision 13; §5. Amended by D-467.

#### D-389 [H] Descriptors bind from durable SKU versions

Prices do not freeze GL, tax, invoice descriptors, metering or timing. Consumers bind the SKU version in force at the period start via versions?as_of; equal effective dates choose the highest published_version. An earlier pin keeps its descriptors forever. A descriptor change creates no refreeze price or pricing approval unit.

**Source:** §2 decision 14; §2.2; §7.1.

#### D-390 [H] Windows normalize per chain and preserve usage structure

On approval, sort approved prices within each (price_book_entry_id, dim_value), set predecessor effective_to to successor effective_from, and enforce one approved start per chain. The default tail stays open; a value tail may explicitly end and resume default fallback. Re-read chains transactionally, with serializable Postgres isolation. Usage successors cannot change package size or SKU metering as of each price's start (D-402): CHAIN_MODEL_CHANGED at submit, revalidated at apply. The model cannot change on a chain at all: it is the entry's (D-427).

**Source:** §2 decision 16; §5.

#### D-391 [H] Temporary changes keep pair identity or resume fallback

On an existing chain, temporary_until creates a promo price and a return price in one unit. The return copies the money versionAt would apply at the end and inherits dim_value. Common-date shifts preserve duration; a shift that would carry a temporary price across the start of another price of its chain, where apply would cut it short, is refused TEMPORARY_SPANS_A_CHANGE (D-406). A return to an explicitly closed price (a temporary nested in a value's closed price) ends explicitly at that price's end, which a common-date shift does not move, so the value falls back to the default after it. Submit and apply re-derive that copy from the approved chain as it stands, on the shifted end: a return that no longer names the price in force there, or no longer carries its price and min_fee (the model is the entry's, D-427), and a single closed price whose own chain now has a price in force on its end, are refused PAIR_RETURN_STALE (400 at submit, APPLY_REFUSED at apply); the author re-drafts. When the price in force on that end is itself a price of the same unit, the return is right only if that price is another pair's return naming the same restored price with the same price and min_fee (two pairs on one chain, both drafted against it); any other price of the unit there makes it stale. A value with no own chain gets one closed price and no synthetic return copy of default. A temporary that ends exactly where the chain's next approved price starts is the promo price alone: that price already ends it. Pair edits, selection and submission cannot orphan a companion.

Amended by D-443: the temporary half of a draft takes new dates, and the PATCH builds its pair again over them in the same transaction — the return re-derived in place, deleted or created as the new end calls for — so a moved pair is never left for submit to refuse as stale.

**Source:** §5 temporary pairs.

#### D-392 [H] Publish changes is a selected book batch

List all draft book prices with full predecessor, proposed content and impact, pre-select all, then submit the operator-selected atomic prices and optional common_effective_date as one prices unit; a ticked half of a temporary pair brings its partner (D-405). Book money and plan structure remain independently approved. A rejected revision cannot undo an approved repricing that also affects the old revision.

**Source:** §2 decision 7; §6; §8.

#### D-393 [H] One unit engine, quorum and generations

bss-approval owns the shared engine shape; pricing owns prefixed tables and subjects. Quorum is tenant policy with kind overrides and fail-safe one when * is absent. No materiality. Submitter and all item authors are excluded from approval; an item's author is its price's creator, the only principal who may edit it (D-404). Votes carry generation; drift commits refreshed items/snapshot/hash, increments generation and marks prior decisions stale (UNIT_STALE). Conditional unit version yields UNIT_CONTENDED on lost races. Quorum zero, reject and withdraw all write terminal audit and ApprovalUnitDecided. See ADR-0003.

**Source:** §2 decision 8; §2.2; §6.

#### D-394 [H] Plans are versioned structure bound to one book

Phase 3 publishes immutable revisions containing one book, paid/optional/included items, minimal Grants and optional sold-as bundle. Validate recurring frequency, meter uniqueness, usage-only included quantities, SKU lifecycle, foreign-book entries, book validity and coverage per value. blocked_by is computed from pending price units. Clone makes a draft; publishing a revision never moves existing pins, and neither does the switch of a scheduled revision on its date (D-450). Grants and the sold-as bundle are deferred (D-411), and so is retirement (D-410).

D-467 amends this entry: a revision's items are SKUs, each with its entry in the plan's book; there are no paid, optional or included treatments and no included or minimum quantity, so neither "usage-only included quantities" nor any check keyed on a treatment remains.

**Source:** §5 plans; §6; §8. Amended by D-450, D-467.

#### D-395 [H] Promotions are versioned and migrations are requests

Phase 3 forbids overlapping plan promotions; [from_date, to_date) applies by period start. Approved edits increment promotion version and bindings pin id/version. An approved migration to a published revision persists its preview and emits SubscriptionMigrationRequested; Subscriptions executes movement and confirms retirement in its separate plan. Pricing never reports a request as an executed move. Both halves are deferred by the owner: promotions (D-409) and migration requests (D-410).

**Source:** §5; §6; §11 phase 3.

#### D-396 [H] One replay store and optimistic conditional writes

All pricing POSTs require Idempotency-Key. The 24-hour store is keyed by tenant, concrete endpoint and client_key; payload hash guards replay. The hash is over canonical JSON (object keys sorted at every depth), so the order a client sends keys in never turns a retry into IDEMPOTENCY_CONFLICT. Check it before reservations or unit work and claim/respond in the mutation transaction. Approval units have no idempotency column, despite the superseded §6 sketch. If-Match protects PATCH/PUT; conditional pending ownership and unit versions replace row locks. Persist audit and domain events with the mutation.

**Source:** §2.2; §3 items 23 and 27; §7.2.

#### D-397 [H] Consumer pins replace cohorts and catalog versions

Phase 4 resolve returns a full per-item chain matrix, versioned descriptors and promotion inputs (deferred with promotions, D-409), never totals. Renewals walk all successors from their pin and stop before the first new price; signup selects the in-force price. Approved prices remain readable by id forever, including keep_for_bound predecessors. Usage binds lazily per value, slices at a binding's ends_on (D-425) and resolves again with the pin on that date. Quote is the Studio totals preview, not the consumer contract, and it is not built (D-415).

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

Tx A claims the Idempotency-Key, mints price_book_entry_id and inserts pricing_reference_op with kind create_entry and state reserving before reserve. Reserve is idempotent per (owner, kind, ref_id). Re-read the SKU after reserve; Tx B inserts the entry with reservation_id and reference_state = confirmation_pending and moves the op to written. The op's outcome carries the create input (sku_id, period, dimension_key, invoice_line_override and, from D-427, model), and every op of an entry rebuilds it from the entry (a rereserve, a delete); Tx B re-judges the period and the model against the SKU type the reservation froze, and a refusal there is a 400 receipt. Confirm succeeds before Tx C sets the entry to confirmed, the op to done and answers the key. A refusal after reserve moves the op to cancelling, then release, then done. A door that gets no definite answer before the write (the reserve, or the SKU re-read after it) moves the op to cancelling, releases the key claim and answers 503: a 503 never becomes an entry, and the cancellation releases any receipt. Any other error that ends the door's drive after its reserve and before its write (409 CONTENDED, a 500) cancels the create the same way before it is answered, so no answered error becomes an entry later; only a failed cancellation leaves the op to the ticker. The ticker never makes a first reservation for a user: it cancels a create still reserving without a receipt, and completes one holding a receipt only when its door is gone. Delete removes the entry and inserts a delete_entry op in releasing in one transaction.

A ticker drives every op not done with bounded backoff and never drops one. It also reconciles confirmed entries through states(): a released reservation on a live entry is re-reserved through a rereserve_entry op when the SKU is not fenced; otherwise the entry becomes lost, new prices fail ENTRY_REFERENCE_LOST and PriceBookEntryReferenceLost is emitted. A reservation released before its confirm follows the same rule: Tx C keeps the entry confirmation_pending and starts a rereserve_entry op, so a create is never answered lost. A reservation Products answers 404 for (it does not know the id, for example after a restore from an older backup) is treated as released: at confirm it takes this rereserve path, and at release it counts as released. Only a reservation refused because the SKU is fenced, retiring or retired makes an entry lost; any other refusal of a re-reservation is retried. Reconciliation also scans lost entries and re-reserves those whose SKU admits a reservation again (a lifted fence). Never release because a confirm timed out. An op has no foreign key to the entry and outlives removal. This supersedes D-398's earlier bookkeeping. Phase 3 renames the op kinds create, delete and rereserve, adds attach (D-413), and has an op name its reference as (ref_kind, ref_id), so plan items use the same machine (D-407, D-412).

**Source:** Phase 2 plan 2c.1/2c.6; plan review findings 2, 3.

#### D-402 [H] The pair guard compares SKU metering as of each price's start

**Status:** DECIDED 2026-09-25.

On a usage chain the successor keeps package_size and the SKU's (unit, usage_type_ref) read from the SKU version in force at each price's effective_from; otherwise CHAIN_MODEL_CHANGED. The model needs no comparison: every price of a chain has its entry's model (D-427). Products freezes the SKU type while referenced but versions its metering, hence the dated read. The meter is included because the supersession-continuity family (spec §5 "asserts exactly this") rejects a meter change. Submit checks the guard and apply rechecks it. A price that starts before the SKU's first version is compared with that first version's metering, never with "no metering". A Products refusal of the dated read (for example 403 for a caller without SKU read) reaches the caller with its own status and code; only unavailability (5xx, timeout, rate limit, a lost race) is 503 REGISTRY_UNAVAILABLE.

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

An item added by POST /plan-revisions/{id}/items reserves kind plan_item with ref_id = the item id, through phase 2's create op (D-401: the op before the reserve, the SKU re-read, the write, the confirm; a 503 writes nothing). plan_item is the only new reference kind of phase 3: the sold_as kind waits with the sold-as bundle (D-411). Items are POST /plan-revisions/{id}/items and PATCH or DELETE /plan-items/{id}, not a list inside PATCH /plan-revisions/{id}. This deviates from spec §7.2 on purpose: an added item is one op with its own Idempotency-Key and its own recovery, and a removed item is one delete op with its own recovery (the DELETE takes no key).

D-467 amends this entry: the item create takes sku_id and price_book_entry_id, both required, and the item PATCH takes only price_book_entry_id; treatment, included_qty and qty_min are 400 BODY_UNEXPECTED at both doors.

**Source:** Phase 3 plan rev 2; plan review HIGH 2; owner, 2026-09-25 (kind plan_item only); deviation from spec §7.2 (items inside the revision PATCH). Amended by D-467.

#### D-408 [H] Plan checks read every SKU fresh; descriptors are information, never content

**Status:** DECIDED 2026-09-25.

Checks, submit and apply read each item's SKU through sku_for_write (D-399). A deprecated SKU may stay in a new revision of the SAME plan that carries it over from the published revision; it cannot be added, and a clone (a new plan) that carries it is red (ITEM_SKU_DEPRECATED). A draft, retiring or retired SKU is red (ITEM_SKU_UNAVAILABLE). A bundle SKU cannot be an item (ITEM_BUNDLE_SKU). An entry that a plan item names cannot be deleted (ENTRY_IN_USE). The approval snapshots of revisions and of prices carry each SKU's current descriptors for the reviewer, OUTSIDE the fingerprinted after: a GL change must not refresh every pending unit. They are gathered in collect or validate_submit and cached on the subject, because snapshot is synchronous. Being information, their read is best-effort and never refuses a submit, a vote or a reject (D-416).

D-453 amends this entry: the published revision is the one in effect today (D-447). Once a scheduled revision is due, it is its plan's published revision and its predecessor is not, before the job persists the switch and after it. So the checks of both revisions answer the same on the two sides of the persist.

D-465 amends this entry: "newly added" does not cover a re-add. The item create admits a deprecated SKU that the plan's published revision in effect carries, as the checks do, so an item removed from a draft can be added back; any other deprecated SKU stays ITEM_SKU_DEPRECATED.

D-466 extends this entry: each check row names the items that turn it red and the pending prices behind its blocked_by.

**Source:** Phase 3 plan rev 2; plan review MEDIUM 6; phase 2 "owed to phase 3" (fresh SKU reads, current descriptors in snapshots). Amended by D-453, D-465; extended by D-466.

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

**Closed** by D-427 (2026-09-26): the chain is deployed, no shipped migration is edited again, and every schema change is a new migration after m20260926_000012.

**Source:** Phase 3 plan rev 2; plan review LOW 15.

#### D-413 [H] A copied item attaches its reference after the write

**Status:** DECIDED 2026-09-25.

POST /plans/{id}/revisions (copy) and POST /plans/{id}/clone write the revision and every copied item in ONE transaction with reference_state = unreserved and reservation_id NULL, plus one attach op per item. The attach op has the rereserve shape: reserve, then confirm; a losing refusal (a SKU fenced, retiring or retired at the reserve, or a lifecycle or type the SKU re-read refuses) marks the item's reference lost, and any other refusal, such as one of the caller's own grant for the reserve or the re-read, is retried and the ticker finishes the op as the system actor. Products authorizes the system actor to the tenant, so a refusal given to the system actor itself (a SKU Products no longer knows, say) is about the SKU and would never change: it marks the item lost, never an attach retried forever (phase 3 second review B-1). This deviates from "reserve before write" on purpose: the copied SKU is already protected by the SOURCE revision's live reference, which scheduled, published and superseded revisions never release, so no retire or type fence can slip in between. Products admits a deprecated SKU for a reservation, so a carried-over deprecated SKU attaches. The door drives the attach ops best-effort, stopping at the first that fails, and answers 201; the ticker finishes the rest. Submit and apply require every item reference to be confirmed, or confirmation_pending with a receipt; otherwise the checks show ITEM_REFERENCE_PENDING or ITEM_REFERENCE_LOST. POST …/items alone keeps D-401's create op. The source is the revision in effect: both doors switch a due scheduled revision first (D-451), and a copy is refused while a revision waits for its date.

D-467 amends this entry: a copied item is a new row, written paid with no quantity; a legacy item stored without an entry is copied without one (the column's CHECK then keeps it included), so the draft's checks show it ITEM_ENTRY_MISSING.

**Source:** Phase 3 plan rev 2; plan review HIGH 1; deviation from D-401's reserve before write. Amended by D-451, D-467.

#### D-414 [H] A revision's references outlive it

**Status:** DECIDED 2026-09-25.

Items of published and superseded revisions keep their references, fail-safe, because a pinned subscription may still be rated on them. So a SKU ever sold in a revision cannot be retired until those references are released. The release waits until Subscriptions reports that no subscription pins the revision; that report is owed to the Subscriptions integration. The delete of a draft revision releases its items' references through delete ops.

**Source:** Phase 3 plan rev 2; plan review LOW 14.

#### D-415 [H] Quote is not built; the Studio is not wired to the API (owner, 2026-09-25)

**Status:** DECIDED 2026-09-25.

GET /pricing/v1/quote is not built in this programme, and the PriceBook Studio is not wired to the API; no Studio programme follows. Quote was the Studio's preview, not a consumer contract: consumers read resolve and GET /bss-pricing/v1/prices/{id}. The minimum-fee floor arithmetic of D-388 belongs to Rating, with its acceptance example: two values rated 10 + 10 on one default price with min_fee 30 bill 30; two own prices with min_fee 30 each bill 60 (Rating T-D-38). cpt-cf-bss-pricing-fr-min-fee stays: pricing stores and validates min_fee and resolve returns it. In the PRD, DESIGN, slice 07 and features/read-contract-events.md every quote statement stays and is marked not built (D-415); the quote and minimum-fee-floor DoDs stay unticked for that reason.

**Source:** Owner, 2026-09-25, during Run 3.1; deviation from spec §7.1 (GET /pricing/v1/quote) and §11 phase 4.

#### D-416 [H] Descriptors are read best-effort; the reads a rule needs stay hard

**Status:** DECIDED 2026-09-26.

The SKU descriptors an approval unit shows (D-408) are information, so their read is best-effort: when Products cannot answer it (503) or definitely refuses it (for example 403 for a caller without products read), the snapshot records "descriptors": "unavailable" and the submit, the vote or the reject goes on. The reads that feed a RULE stay hard and are made as the caller: the plan checks (GET /plan-revisions/{id}/checks, and the plan_revision subject's validate_submit and apply) and a usage chain's dated metering (the pair guard, D-402). There only unavailability is 503 REGISTRY_UNAVAILABLE, and a refusal keeps Products' own status and code on submit, approve and reject alike, never 400 REGISTRY_REFUSED. So an approve-only reviewer votes on any unit whose rules read no SKU and rejects any unit, while a rule read needs products read too: the plan_revision submitter and its final approver (apply re-runs the checks), readers of the checks, plan item authors (the item door and its create re-read), price-book entry authors (the period rule and the create re-read), and the submitter and final approver of a prices unit on a usage chain.

**Source:** Phase 3 review, fix run 7 (plans F2, surface S-1, docs F1 and F2; controller notes 4 and 5).

#### D-417 [M] The last revision of a never-published plan takes the plan with it

**Status:** DECIDED 2026-09-26.

DELETE /plan-revisions/{id} of the last revision of a plan that was never published (published_rev IS NULL and no other revision) deletes the plan in the same transaction, with a plan.delete audit row, and frees its code; the door still answers 204. Without it such a plan was stranded: a copy answered PLAN_UNPUBLISHED, a clone CLONE_SOURCE_UNPUBLISHED, no door deletes a plan, and its code stayed taken. Nothing refers to a never-published plan: it has no published revision, no pin and no event, and an approval unit names a revision by ref_id without a foreign key. A plan with a published revision keeps its behaviour: deleting its draft leaves the plan, its code and its published revision as they are.

**Source:** Phase 3 review, fix run 7 (plans F1, surface S-2; controller note 1).

#### D-418 [H] A plan revision is submitted under plan:submit

**Status:** DECIDED 2026-09-26.

POST /plan-revisions/{id}/submit checks the plan label's own submit action, plan:submit: the label-specific analogue of price:submit on POST /prices/{id}/submit and of price_book:submit on publish-changes. The plan label's actions are read, author and submit. approval_unit:submit, which the run 3.4 brief bound to this door, stays the right to withdraw a unit; it no longer lets a prices submitter submit, or read through the receipt, a plan revision, so a tenant can withhold plan submission from its prices submitters.

**Source:** Phase 3 review, fix run 7 (surface S-4); corrects the run 3.4 brief.

#### D-419 [H] Resolve answers one revision on one date, with the caller's pins

**Status:** DECIDED 2026-09-26.

GET /bss-pricing/v1/resolve?plan_revision_id=&date=&item_id=&pins= (label plan, action read); it is spec §7.1's /pricing/v1/resolve below the gear's base. Only a published or superseded revision resolves, and a scheduled one from its sale date (D-454); a draft or pending one is 409 REVISION_NOT_PUBLISHED, and a scheduled one before its date 409 REVISION_NOT_YET_AVAILABLE. The state is the one the revision reads today (D-447). date is required, YYYY-MM-DD (400 DATE_INVALID). item_id is optional: resolve that one item only (404 if the revision has no such item); a consumer with many pins splits its request by item. pins is optional and comma-separated, each pin either price_id or price_id:dim_value: the first pins the price's own chain; the second pins a DEFAULT-chain price that the value dim_value was bound to. A pin must name an approved price of the tenant, of an entry an item of this revision names; with :dim_value it must be a default-chain price of that entry, and the value is NOT checked against today's registry (a value may have been removed since). Otherwise the whole request is 400 PIN_FOREIGN. Two pins for one (item, value) are 400 PIN_DUPLICATE; more than 1 000 pins are 400 PINS_TOO_MANY. Resolve is a read: it writes nothing (no audit row, no idempotency key, no binding) and returns no totals and no promotion (D-409, D-415).

**Source:** Phase 4 plan rev 3 (Run 4.2); spec §7.1; plan review M4 b (a removed value is not checked against the registry), L4 (the pin bound and the split by item) and L7. Amended by D-454, D-467: a resolved item carries no treatment, included_qty or qty_min.

#### D-420 [H] The matrix and the walk: a binding is always in force

**Status:** DECIDED 2026-09-26.

Per item, the chains are the default chain and one per value registered today for the entry's dimension key, plus any value a pin names (so a removed value still resolves for the subscription that holds it); an item without an entry has no chains. Per (item, value):

1. No pin (signup): the binding is the price in force on date, the value's own chain, else the default chain (domain::price::version_at); dim_used says which; neither gives uncovered: true and no binding (never an invented price, never a total).
2. With a pin: walk the pinned price's chain forward through approved successors with effective_from <= date: an all successor is taken; the walk stops before the first new successor (spec §7.1, D-397), except as rules 3 and 4 say. The last price reached is the binding; pinned_from names the pin.
3. A binding is always in force (generic; plan review H2 i and iii): if the walk ends on a price that is no longer in force on date (a closed value chain, a temporary price whose window ended, a chain with no successor), the binding is chosen as in rule 1 for that value (own chain, else default: the value reads the default again, spec §5); pinned_from still names the pin. This is also what happens to a pin on an ended temporary price. There is no promotion-specific rule: a temporary pair is walked like any other prices of its chain, and a new promo pair is NOT skipped (the owner, 2026-09-26: promo is not built now, spec §2.4 addendum); promotion-aware renewal comes back with promotions (D-409).
4. A default-chain pin for a value that later got its own chain (plan review M4 a): the binding moves to the value's own chain when the own price in force on date has eligibility all and started after the pin's effective_from (spec §7.1: a new all price changes the binding from the next period); a new own price does not move it. This is the owner's decision of 2026-09-26 (plan question 2, answered "the second question as recommended").

Read precisely: a price reached by the walk is no longer in force on date when it starts after date, or when its own end has passed. Its own end is read from the stored fields normalisation does not rewrite: temporary_until for a temporary price, else the stored end of an explicitly closed price (closed_explicitly), else none (amended by D-425, phase 4 review C-1). It is never a temporary price's stored effective_to: normalisation cuts that at the start of a pair nested inside the temporary price, and that start is a successor the walk did not take. An explicit close that a later pair cuts is stored as min(end, next) and binds until that stored end; no new column keeps the original end. The start of a successor the walk did not take does not end a price for the pin: a price stopped before a new successor stays the binding, and keep_for_bound marks exactly such a price. keep_for_bound predecessors stay readable and bindable; the resolve context carries the set of keep_for_bound price ids (the domain Price has no such field; plan review L1) and the binding reports it. Each binding reports its own end as ends_on (D-425).

**Source:** Phase 4 plan rev 3 (Run 4.2); the owner, 2026-09-26 (Q1: no promotion-specific renewal rule; Q2: rule 4 as recommended); spec §2.4 addendum, §5, §7.1, §12; plan review H2, M4, L1. Amended by D-425 (phase 4 review, fix run 8): the own end, and ends_on. Amended by D-467: only a legacy item stored without an entry has no chains.

#### D-421 [H] The binding carries resolved invoice inputs with their source

**Status:** DECIDED 2026-09-26.

Each item's SKU version is read as of date through sku_version_as_of (the detached registry read, made as pricing's system actor after the caller passed plan:read, D-424: consumers need no products read). A registry that cannot answer is 503; Products' definite refusal keeps its own status and code; a 404 for an unknown SKU gives sku_version: null, like no version on that date, never a pass-through 404 that reads like "revision not found". The item returns, each as { value, source } with source entry, sku, tenant or null: invoice_line_template = the entry's invoice_line_override, then the SKU version's template, then the tenant template for the charge kind (settings.invoice_line_templates); gl_code = the SKU, then the tenant default_gl; tax_category = the SKU, then the tenant default_tax_category; billing_timing = the SKU, then the tenant default_timing (PRD AC #13: advance on the SKU beats arrears as the tenant default). It also returns sku_version { published_version, effective_from } and meter { usage_type_ref, unit } of that version (null without a version). The rules mirror the prototype (ui-prototype pricebook 50-rules.js, resolveInvoiceLine and the DESCRIPTORS check) and the plan checks' Defaults. Revision-level: book_id, currency, currency_minor_digits (domain::book::minor_digits), rounding_policy (the tenant default_rounding) and date.

Read precisely: settings.invoice_line_templates is keyed by SKU type (recurring, usage, one_time, bundle; PUT /settings refuses any other key). A charge kind's name is its SKU type's, so the entry's charge kind is the key; an item without an entry takes its SKU version's type, as the prototype does. A source that is absent or blank falls through to the next one; when none is left the field is { value: null, source: null } (the prototype's built-in "{sku}" line is not adopted). billing_timing always has a value: the tenant default_timing is advance until the settings are written.

**Source:** Phase 4 plan rev 3 (Run 4.2); PRD AC #13 (fr-settings); spec §7.1 (what a pin carries), decision 14; plan review H3 and L7. Amended by D-424 (phase 4 review, fix run 8): the SKU version read is made as pricing's system actor. Amended by D-467: the item returns no stored treatment or quantity.

#### D-422 [H] The pinned price read serves approved money forever

**Status:** DECIDED 2026-09-26.

GET /bss-pricing/v1/prices/{id} (label price, action read), spec §7.1's /pricing/v1/prices/{id} below the gear's base, answers an APPROVED price of the tenant whatever its window (closed, followed by a later price, keep_for_bound) with its entry's SKU, charge kind, period, book and currency. It returns only stored facts: no status or other value computed from today, and no authoring internals (version, pending_unit_id, note, created_by). A draft, pending or rejected price, an unknown id and another tenant's id are 404, with the same body.

**Source:** Phase 4 plan rev 3 (Run 4.2); spec §7.1; plan review H4 (no value computed from today).

#### D-423 [H] Both gears refuse a legacy or stale schema at boot

**Status:** DECIDED 2026-09-26.

Each gear's chain starts with ONE guard migration, named to sort first under the toolkit runner's name sort: m0000_pricing_refuse_a_legacy_or_stale_schema here and m0000_products_refuse_a_legacy_or_stale_schema in Products (products P-D-195). It sorts before the coordination, broker and outbox migrations (m0001_…, m001_…), so it is pending on every database that predates phase 4 and runs there once, before anything of the gear is created. It creates nothing and reads the catalog only: sqlite_master and table_info on SQLite; information_schema and pg_constraint on Postgres, tables in schema bss. It refuses (the migration fails, and boot fails naming the gear) when it finds:

- a legacy table: one of the 45 pricing_* tables that the pre-PriceBook chain creates (bss/products-backup, m20260821_000001 to m20260921_000050, 47 tables) and today's chain does not. pricing_plan and pricing_price exist in both and are not evidence. The set is a constant in the guard; a test proves it disjoint from every table a fresh chain creates.
- a stale shape, a table that today's chain edited in place (D-412) or renamed: pricing_reference_op without the column ref_kind; pricing_price_row present (the pre-rename name); pricing_price without the column price_book_entry_id (the pre-rename, entry-shaped table). Because the guard sorts first, a pre-rename database, on which the renamed migrations 000005 and 000007 are pending too, meets the guard's refusal and not a raw SQL error from 000007.
- the legacy shape of a table that both chains create: pricing_plan without the column code (the legacy revision row). A clean-up that drops only the tables a refusal names leaves it, and m20260926_000010's CREATE TABLE IF NOT EXISTS would keep it and then fail on its code index with a raw SQL error (phase 4 review F2, fix run 8).

The refusal reads: bss-pricing: this database holds a <legacy|stale> bss-pricing schema (<what was found>); PriceBook does not migrate it — start from an empty data root / empty bss-pricing tables. A fresh database passes, and so does a database migrated by today's chain: the guard is pending there once and finds nothing. The schema goldens do not change.

**Source:** Phase 4 plan rev 2 (Run 4.1); plan review H1, M1, M2 and L6. It is the phase 4 legacy-history guard that D-412 names.

#### D-424 [H] Resolve reads SKU versions as pricing's system actor

**Status:** DECIDED 2026-09-26.

Rating and Subscriptions call GET /bss-pricing/v1/resolve as system subjects (bss-rating.system, bss-subscriptions.system; PRD §2.2 lists them as system actors). The Products registry refuses every system subject except bss-pricing.system (403 REFERENCE_OWNER_MISMATCH, DESIGN §3.5), so a SKU version read made as the caller refused every consumer, whatever its grants. Resolve therefore reads each item's SKU version as of date as pricing's system actor (reference_ticker::system_actor for the caller's tenant). It does so only after the caller has passed plan:read and the revision was found in the caller's tenant. This widens what a plan reader reads: a human caller with plan:read but no products read now receives, through resolve, the SKU-version fields (sku_version, meter, and the invoice inputs whose source is sku) of the SKUs its revision names. It receives no other SKU and nothing of another tenant: the SKU ids come only from the items of a revision found under the caller's plan:read scope, and the system actor reads in the caller's tenant. A consumer needs pricing plan:read for resolve and price:read for the pinned read, and never products read. The rest of D-421 stands: a Products that cannot answer is 503 REGISTRY_UNAVAILABLE, a definite refusal keeps Products' own status and code, and a 404 gives sku_version: null. The reads that feed a rule stay the caller's (D-416).

The registry's trust of pricing's system actor is in-process only; its threat model is products P-D-222 (whole-branch review RS-02, fix run W1b). A pricing door hands the registry its caller's context (the entry and plan item doors, the plan checks' SKU reads), so no REST door of either gear serves that actor: a caller whose context carries it in either half, the subject type bss-pricing.system or the id PRICING_SYSTEM_ACTOR, is 403 SYSTEM_ACTOR_RESERVED at every door, before the PDP (support::require_authenticated, which every door calls first; the test is bss_products_sdk::is_pricing_system_actor). Before, a token carrying both halves, with a pricing grant, got the registry's tenant-wide trust through an entry create (the second review of W1b, M1; fix run W1c). Rating's and Subscriptions' system subjects are not refused: they pass the edge and the PDP judges their plan:read, as above. Resolve still builds its own system actor after the caller passed plan:read.

**Source:** Phase 4 review, fix run 8 (docs M1: the decision change that the review names as the alternative; the orchestrator's decision). Amends D-421. Amended by the second review of W1b, M1 (fix run W1c): the REST edge refuses pricing's system actor.

#### D-425 [H] The binding says where it ends for its holder

**Status:** DECIDED 2026-09-26.

Each binding of GET /bss-pricing/v1/resolve carries ends_on: the binding's own end as D-420 rule 3 defines it. That is temporary_until for a temporary price, the stored end of an explicitly closed price, and null when the price has no end of its own. A consumer slices a period at ends_on, never at effective_to. effective_to stays in the binding as the stored window, for information only: the start of a successor sets it, and the start of a new successor that the pin did not take is not an end for a pinned subscription. Rule 3 reads the same own end, so a pinned temporary price binds until its temporary_until even when a pair nested inside it has cut its stored window (phase 4 review C-1).

A return to a TEMPORARY price (a pair nested in an outer pair) ends where that temporary price ends — its temporary_until, stored as an explicit close — so its binding carries that ends_on too (phase 4 second review M1, fix run 9).

**Source:** Phase 4 review, fix run 8 (contract C-1, docs M2; the orchestrator's decision). Amends D-420. Narrows spec §7.1's "Slices" bullet (a period is split at every row boundary of the bound chain): a successor's start inside a period is not a slice point for its holder; the binding's ends_on is (phase 4 second review L2).

#### D-426 [H] An entry's invoice line is locked once the entry carries money

**Status:** DECIDED 2026-09-26.

A PriceBookEntry stays outside approval: it is structure, and money reaches a customer only through an approved price (the prices kind) and a published plan revision (the plan_revision kind). The one entry field that reaches consumers directly is invoice_line_override: resolve returns it as the invoice line with source entry, ahead of the SKU version's template (D-421), while the same template on the SKU changes only through Products' sku_change approval. So once an entry has an approved or a pending price, PATCH /bss-pricing/v1/price-book-entries/{id} refuses a change of invoice_line_override (a new value or null) with 409 INVOICE_LINE_LOCKED; sending the stored value is no change. A draft or rejected price does not lock it. Another invoice line needs another entry. The prices of the entry are read tenant-scoped in the PATCH transaction.

**Source:** Owner, 2026-09-26 — asked why PriceBookEntry is not under approval, chose option 1 of three (lock the override once the entry carries money; the others: route override changes through a prices unit with an effective date, or accept the gap). Amends D-421.

#### D-427 [H] The model belongs to the entry, fixed for its life, and is part of its key

**Status:** DECIDED 2026-09-26.

A PriceBookEntry carries its model: pricing_price_book_entry.model is NOT NULL, one of flat, per_unit, graduated, volume and package, and allowed for the entry's charge kind (D-386). POST /bss-pricing/v1/price-books/{id}/entries requires model: an unknown string is 400 MODEL_INVALID, and a model that the charge kind does not allow is 400 MODEL_KIND_CHARGEKIND_MISMATCH. No PATCH changes it: the entry PATCH does not carry it and refuses an unknown field. The create is the durable op of D-401: the door judges model against a fresh SKU read before it claims anything, and Tx B judges it again against the SKU type the reservation froze; a refusal there is a 400 receipt, and the op releases its reservation. The op's persisted create input carries model, and a rereserve or delete op rebuilds it from the entry. An op stored before m20260926_000013 has no model. Such a create was posted under the key of its day (book, SKU, charge kind and period): when an entry already holds that key, Tx B gives the create that entry's model, so the insert meets the key and the op ends 409 ENTRY_KEY_TAKEN without a second entry. Otherwise it resolves to the model that the migration gives an entry without prices, the charge kind's default (flat for recurring and one_time, per_unit for usage). A rereserve or delete op does not write the model, so the entry keeps its own.

model joins the entry key: pricing_price_book_entry_key is (book_id, sku_id, charge_kind, coalesce(period, ''), model). So another model is another entry beside the old one in the same book, and a plan picks between them through its revision's items (a structural change under plan_revision approval). The create's refusal of a taken key stays 409 ENTRY_KEY_TAKEN, on the wider key. The book change of a draft revision (PATCH /plan-revisions/{id} with a new book_id) remaps an item to the new book's entry of the same (SKU, charge kind, period, model). The owner did not answer the plan review's key question (M4); model joins the key as the orchestrator's call, which the owner may overturn.

pricing_price.model is dropped. A price's price_json must match its entry's model: a mismatched shape stays 400 PRICE_MISSING. POST /price-book-entries/{id}/prices and PATCH /prices/{id} no longer carry model, and refuse it as an unknown field. The reads keep model, read-only and copied from the entry: every price of the authoring doors (PricingPriceDto) and GET /bss-pricing/v1/prices/{id}. GET /bss-pricing/v1/resolve carries model on the item (null for an item without an entry) and no longer on the binding. The pair guard compares package size and dated metering (D-402); the prices of one entry share its model, so the guard no longer compares a model, and CHAIN_MODEL_CHANGED stays for those two. A temporary pair's return no longer copies or compares a model (D-391). The fingerprinted before and after of a prices unit no longer carry model, so each prices unit that is pending at the deploy refreshes once, to a new generation, the first time it is judged.

The schema change is the forward migration m20260926_000013_model_on_the_entry, in the runner's transaction, on both dialects. On Postgres it first locks pricing_price_book_entry and pricing_price ACCESS EXCLUSIVE, so its check and its backfill judge the same prices (SQLite's writer lock already serialises). When the prices of an entry, in any state, carry two or more models, it fails, names the entries and changes nothing. Otherwise it adds the column (SQLite: NOT NULL DEFAULT 'flat' with its CHECK, then the backfill, and the default stays in the schema; Postgres: a nullable column, the backfill, SET NOT NULL and the named CHECK pricing_price_book_entry_model_check). The backfill gives each entry the one model of all its prices, or the charge kind's default when it has no price. Then the migration recreates the key index with model last and drops pricing_price.model. Its down is an explicit irreversible error. D-412 closes with this decision: no shipped migration is edited again, and every schema change is a new migration after m20260926_000012.

**Source:** Owner, 2026-09-26; phase 5 plan rev 2 (plan review H1, H2, M3, M4, L9, L11, L12). Amends D-386, D-390, D-391, D-401 and D-402; closes D-412.

#### D-428 [H] Entries and SKUs report their usage

**Status:** DECIDED 2026-09-26.

The two entry reads carry the entry's usage, for the SKUs screen and the book view. GET /bss-pricing/v1/price-book-entries/{id} and GET /bss-pricing/v1/price-books/{id}/entries answer PricingPriceBookEntryReadDto: the fields of the entry and usage = { prices: { approved, pending, draft }, plans, plans_superseded_only }. prices counts the entry's prices by state; a rejected price is not counted. plans counts the distinct plans that have a draft, pending, scheduled or published revision whose items name the entry; two revisions of one plan count once. plans_superseded_only counts the distinct plans that name the entry only through superseded revisions. Such a plan is history, but its items keep their references (D-414) and keep the entry ENTRY_IN_USE (D-408), so an entry with plans 0 can still refuse its delete. An entry counts in every reference state (confirmation_pending, confirmed, lost), and so does a plan item. The other answers that carry an entry keep the plain PricingPriceBookEntryDto without usage: the POST and PATCH answers, the stored Tx B receipt, the export and the publish-changes listing. The counts are numbers about the entry that the caller may read (price_book_entry read). They are read tenant-scoped, as the entry PATCH reads its prices, and need no price read or plan read. A request makes a fixed number of set-based reads, whatever the number of entries: the toolkit's QueryRecorder shows the same statement count for 10 and for 100 entries.

Pricing also fills the SKU usage port of Products (P-D-197): products-sdk SkuUsageV1, which pricing registers in the ClientHub at its init as dyn SkuUsageV1. For the SKU ids of one tenant it answers each distinct id once, as { sku_id, entries, currencies, prices { approved, pending, draft }, plans }. entries counts the SKU's entries in every book of the tenant, in every reference state. currencies are the distinct currencies of their books, sorted. prices adds up the counts of those entries. plans counts the distinct plans across all the SKU's entries, with the entry rule above: a plan that names two entries of the SKU counts once, where a sum of the entry counts would count it twice. An unknown id, the SKU of another tenant and a bundle SKU (it has no entry, D-386) answer zeros. The caller must hold price_book_entry read; otherwise the port answers 403, and Products shows no usage. Pricing reads only the SKU ids that it is given, and no Products data flows back into pricing.

Products P-D-212 adds the port's usage_sets(ctx, tenant): the tenant's priced SKUs (an entry in any book, in any reference state: entries above zero) and its in-plan SKUs (an entry named by a plan item of a draft, pending, scheduled or published revision: plans above zero), each sorted and distinct, under the same rule and scope, in two set-based statements whatever the number of SKUs. They are what the Products SKU list's priced and in_plan filters keep or drop.

D-440 amends the entry reads: usage.prices also counts the approved prices by where their window stands today (scheduled, active, superseded; approved is their sum), from the same grouped count, and each entry read carries current_price. The SKU usage port keeps products-sdk's PriceCounts unchanged.

D-453 amends the plan counts: they read the stored state, so a scheduled revision counts as a published one does, and a due revision's stored-published predecessor counts too until the switch is persisted. The switch job ends that over-count: it persists at most the ticker's limit (100) due revisions a cycle (a minute), oldest available_from first, so a backlog of more than 100 drains 100 a cycle, and a plan whose switch keeps failing over-counts until it is repaired (D-450).

**Source:** Owner, 2026-09-26; phase 5 plan rev 2 (the counts on the entry and on the SKU, option 1; the semantics confirmed by the owner; plan review M6, M7, L10). Amended by D-440, D-453.

#### D-429 [M] The replay store's mechanics (twin of products P-D-198)

**Status:** DECIDED 2026-09-27.

A key's row in pricing_idempotency is claimed or answered, and a CHECK ties the response pair to the state. The claim INSERT is the at-most-once gate: a door claims on the transaction that writes its act (D-396), and a reference-work door claims in Tx A with its op (D-401), so the claim commits before the reserve; when that create is cancelled before its write, the cancellation deletes the claim and frees the key. The endpoint is the concrete resource path, never the route template, and no in-flight deadline exists. The answered row stores the status and body the caller was told, so a replay reads no other row. An answer is stored only when its transaction commits: a refusal that rolls back takes the claim with it, and a committed refusal (Tx B's 400 receipt, D-401 and D-427) is stored and replays. Expiry is judged at claim time: an expired row is taken over by a compare-and-swap on the expires_at that was read, and the loser answers IDEMPOTENCY_KEY_IN_FLIGHT having executed nothing. The loser may even carry a different payload from the winner, and is still refused in-flight rather than for the mismatch, since its transaction never compared the two: it read the expired holder's digest, never the winner's. Apart from that loser, a matching live claimed row is IDEMPOTENCY_KEY_IN_FLIGHT, and a digest mismatch is IDEMPOTENCY_CONFLICT in either state. POST entry and POST plan item bind their claim to the durable reference op in Tx A (bind_op), and entity_ref holds that op's id. A bound claimed row is never taken over on expiry: a matching retry is IDEMPOTENCY_KEY_IN_FLIGHT until the op answers the key. The op's answer is kept a full 24 hours from the moment it is written, however late, so a retry after a late answer replays it. Every other door leaves entity_ref NULL. Products runs the same store (P-D-198), with two differences: products binds no op, so its entity_ref is always NULL; and its retention is configurable (24 hours to ten years), where pricing's is a fixed 24 hours.

**Source:** Carried from D-142 (backup `3a38f0b28`); decisions cleanup, owner 2026-09-27.

#### D-430 [M] A tier ladder's top band is open

**Status:** DECIDED 2026-09-27.

A graduated or volume price has at least one band (TIER_BAND_EMPTY), strictly ascending bounds below the top (TIER_BANDS_ORDER), and an open top band: a closed top band is TIER_TOP_CLOSED. No quantity above a last bound is left unrated; capping usage is not a price. The band edges themselves are D-387's.

**Source:** Carried from D-17 (backup `3a38f0b28`); decisions cleanup, owner 2026-09-27.

#### D-431 [M] The request's correlation id is minted at the authoring edge

**Status:** DECIDED 2026-09-27.

The authoring router mounts correlation::establish: each request gets one UUID v7 before any handler runs, and an inbound traceparent is not consumed. Every audit row the request writes carries it as correlation_id, so the rows of one call join. It is not the Idempotency-Key and is not derived from the payload. A handler reached without the layer answers 500 and never mints its own. A rereserve op is the exception: the reference ticker starts it, or the Tx C of a create, an attach or another rereserve op does when the reservation was released before its confirm — a Tx C the door's own drive may run inside a request — and it mints its own UUID v7 when it is created. The confirm and reference-lost audit rows its completion writes carry that id, which matches no request. Pricing never writes a NULL correlation_id. The read-contract router (resolve, the pinned price read) writes nothing and mounts none; events carry no correlation id. Products establishes no correlation and writes NULL (P-D-200).

**Source:** Carried from D-178 (backup `3a38f0b28`); decisions cleanup, owner 2026-09-27.

#### D-432 [M] If-Match on every write to a versioned row, and on a draft price's DELETE

**Status:** DECIDED 2026-09-27.

PATCH of books, entries, plans, plan revisions and plan items, PATCH /prices/{id} and DELETE /prices/{id} of a draft price, and PUT of settings, dimension keys and the approval policy require If-Match with the row's strong version: a missing or malformed header is 400, a stale one 409 STALE_REVISION. The DELETEs of an entry, a plan revision and a plan item take none, as built. This extends D-396's "If-Match protects PATCH/PUT".

**Source:** Carried from D-141 (backup `3a38f0b28`); decisions cleanup, owner 2026-09-27. Extends D-396.

#### D-433 [M] The audit log is append-only with a reserved sealing seam (twin of products P-D-200)

**Status:** DECIDED 2026-09-27.

pricing_audit refuses every DELETE by trigger and admits one UPDATE: unsealed to sealed, supplying chain_id, seq and row_hash (prev_hash NULL only on a segment head) with every record column unchanged. The gear writes seal_state = unsealed with the four seal columns NULL on every row and never seals, chains or verifies: sealing is a platform capability the columns are reserved for. The key is a surrogate audit_id, because seq is NULL until a row is sealed. No REVOKE UPDATE, DELETE is issued (a deployment role the migration does not own; SQLite has none). correlation_id is text: pricing writes its edge id (D-431), or on a rereserve op's rows the id the op minted, and never NULL; products writes NULL (P-D-200). error_code, attempted_key, session_id and ceremony_ref are carried in the DDL and written NULL. Products has the same table shape (P-D-200).

**Source:** Carried from P-D-08, P-D-28, P-D-46, P-D-118 (backup `3a38f0b28`); decisions cleanup, owner 2026-09-27.

#### D-434 [M] Where a SKU is priced and sold: its entries across books, the plans that name it, one plan item

**Status:** DECIDED 2026-09-27.

The SKUs screen shows, for one SKU, where it is priced and where it is sold (ask 8). The gear had the counts only (D-428).

- GET /bss-pricing/v1/price-book-entries?sku_id= (price_book_entry read) lists the tenant's entries of the SKU in every book and every reference state, read under the caller's entry scope, ordered by book code, charge kind, period, model and id. Each item is PricingSkuEntryDto: the entry's own fields, book_code, book_name and currency, usage (D-428) and current_price. current_price is the default chain's approved price in force today: no dimension value, effective_from on or before today and effective_to after it, chosen as resolve chooses a price in force (the latest start, then the latest version). It is null when no such price exists. A value chain's price is not the entry's current price. The money is shown only to a caller who also holds price_book read, the export's grant, judged a second time in the same request. Without that grant the entries are still listed, each with current_price null. A denial of that second judgement refuses nothing, and an unavailable policy fails the read with 503. When the grant's scope admits some books only, only their entries show money. sku_id is required: without exactly one well-formed sku_id, or with any other key, the answer is 400 QUERY_INVALID. An unknown SKU, or another tenant's, is an empty list, because pricing does not know which SKUs exist. The read makes seven statements whatever the number of entries: the entries, their books, the three usage reads, the books the grant admits and those books' default-chain approved prices. The QueryRecorder shows the same statements for 10 and for 100 entries.
- GET /bss-pricing/v1/plans?sku_id= (plan read) lists the plans that have a draft, pending, scheduled or published revision whose items name an entry of the SKU. This is the usage's plans definition (D-428; products P-D-212's in_plan): a plan that names the SKU only through superseded revisions does not count, and an included item without an entry does not count. The answer has the same schema as GET /plans (PricingPlanList, each plan with all its revision headers). The revisions, items and entries are read tenant-scoped. GET /plans itself becomes set-based: two statements whatever the number of plans (the plans, then all their revisions), where it made one revision read per plan. A malformed sku_id, or any other key, is 400 QUERY_INVALID. Before, the route took no query and ignored any key.
- GET /bss-pricing/v1/plan-items/{id} (plan read) answers PricingPlanItemReadDto: the item's fields (PricingPlanItemDto), plan_id, rev_no and state, which is its revision's state as it reads today (D-453). The ETag is the item's version, the value its PATCH takes as If-Match. An item the tenant does not hold is 404.

D-440 amends this entry: the two entry reads carry the same current_price, chosen and shown by the same functions (in_force; money_scope, the one second judgement of price_book read), and usage carries the approved prices by date from the same grouped count, so the SKU's entry list still makes seven statements.

D-453 amends this entry: the plan list, by SKU or not, and the item read render each revision's state as it reads today, derived in memory, so GET /plans keeps its two statements; which plans the SKU filter keeps is still read from the stored state.

D-460 amends this entry: each plan of GET /plans, by SKU or not, names its current revision with its item SKUs, read in one grouped statement over the listed plans' current revisions, so the list makes three statements whatever the number of plans. A current revision's sku_ids name every item, while the SKU filter keeps a plan only for an item with an entry, on the stored state: the two may differ.

D-461 amends this entry: each revision header says when it was submitted and approved, from the units the listed revisions name, read in one grouped statement more, so GET /plans makes four statements whatever the number of plans (one when there is none): the plans, their revisions, the current revisions' items and the units.

**Source:** Owner, 2026-09-27; phase 6 plan rev 2 (ask 8; plan review L11). Amended by D-440, D-453, D-460, D-461.

#### D-435 [M] An approval-policy override can be reset; the default cannot be deleted (twin of products P-D-216)

**Status:** DECIDED 2026-09-27.

The PUT sets an override but nothing removed one, so a kind once overridden never followed the default again (ask 11b). DELETE /bss-pricing/v1/approval-policy/{kind} (config settings) removes the kind's override at the policy the caller read. If-Match carries the policy's content tag, the tag the policy PUT takes (D-432). The kind then follows the default quorum again. The answer is 200 with the policy and its new tag, as the PUT answers. The path names the default as `*` (percent-encoded or not), as the PUT's body does. The default is never deleted (400 POLICY_DEFAULT_REQUIRED): a tenant always has a quorum to fall back to, and a tenant that never stored one follows the fail-safe one. PUT changes the default and nothing removes it. The refusals are judged in this order: 403 without config settings, before any precondition; 400 for a missing or malformed If-Match; 400 POLICY_DEFAULT_REQUIRED, or POLICY_KIND_INVALID for a kind other than prices and plan_revision; 409 STALE_REVISION; 404 when the kind has no override. A reset writes one audit row, approval_policy.reset. A unit already submitted keeps the quorum it copied (D-393). Products has the same door for its kinds (P-D-216).

**Source:** Owner, 2026-09-27; phase 6 plan rev 2 (ask 11b; plan review L8).

#### D-436 [M] Dimension values edit one at a time and show their use

**Status:** DECIDED 2026-09-27.

The Settings screen edits one key's values and shows which values are in use (ask 11c).

- The request and response shapes are split. PricingDimensions and PricingDimensionEntry stay the PUT's body only. PricingDimensionKeyPatch is the PATCH's body. Both refuse unknown fields. GET, PUT and PATCH /dimension-keys answer PricingDimensionRegistry: { items: [ { key, values: [ { value, usage: { prices } } ] } ] }. Breaking: each value is an object now, not a string, and a GET answer is no longer a PUT body.
- usage.prices counts the prices of any state whose entry names the key and whose chain is the value: draft, pending, approved and rejected. This differs from D-428's entry counts, which leave a rejected price out, because this count is the removal rule: a rejected or pending price still carries its value. It comes from ONE grouped count (the prices joined to their entries, grouped by the entry's key and the price's value). So GET makes two statements whatever the number of entries and prices (the registry, then the count). The content tag covers the stored registry only: a price written since does not move it.
- PATCH /bss-pricing/v1/dimension-keys { key, add, remove } (config settings, If-Match: the content tag the PUT takes) edits the values of one declared key: a stored key, or the seed key region while the tenant stores no registry. Keys are added and removed by the PUT only. Values are trimmed and empty ones dropped. The result keeps the key's values in their order without the removed ones, then the added ones in the order sent, and it is judged by the PUT's rule for a key. The refusals, in order: 403 without config settings; 400 for a missing or malformed If-Match; 409 STALE_REVISION; 400 DIM_NOT_DECLARED for another key; 400 DIM_VALUE_DUPLICATE for a value named twice across add and remove, or added while the key holds it; 400 DIM_VALUE_UNKNOWN for removing a value the key does not hold; 400 DIM_VALUE_INVALID or DIM_VALUES_FEW for the result; 409 DIM_VALUE_IN_USE for removing a value a price uses. The 409 names the value, for example "DIM_VALUE_IN_USE: region=us is used by 1 price". An empty patch writes nothing and answers the registry. A PATCH writes one audit row, dimension_keys.patch.
- The PUT judges its removals from the same grouped count and from one DISTINCT read of the keys that entries name, where it read every entry's prices one entry at a time. Its DIM_VALUE_IN_USE names the value and its DIMENSION_KEY_IN_USE names the key. Tests pin GET, PUT and PATCH at the same statements for 10 and for 100 entries; each key a write stores is still one write.

**Source:** Owner, 2026-09-27; phase 6 plan rev 2 (ask 11c; plan review M6).

#### D-437 [M] The default rounding is one of five modes; a tenant with no settings rounds half_even

**Status:** DECIDED 2026-09-27.

default_rounding is one of half_up, half_even, half_down, up and down. PUT /bss-pricing/v1/settings refuses any other value with 400 ROUNDING_INVALID; a blank value stays 400 ROUNDING_REQUIRED. No database CHECK is added. A stored value outside the set reads back as stored, and resolve carries it as rounding_policy. The tenant cannot save its settings again until it chooses one of the five, so the deployment has a hard pre-flight gate: `SELECT DISTINCT default_rounding FROM bss.pricing_settings` must return values inside the set, or a normalizing migration ships first. The default of a tenant that never wrote its settings is half_even, banker's rounding: it is aligned with the ledger PRD's platform default, so a tenant that sets nothing rounds its prices as the ledger rounds its postings. It was half_up until 2026-09-28. A revision drafted before the tenant's first settings write carries the default, and resolve reports it as rounding_policy. Explicit PUT bodies that name half_up stay valid.

**Source:** Owner, 2026-09-27; phase 6 plan rev 2 (ask 11d; plan review M5). The half_even default: Owner, 2026-09-28.

#### D-438 [M] The settings offer currencies and say who changed them

**Status:** DECIDED 2026-09-27.

- **The migration.** m20260927_000014_settings_currencies_and_author runs in the runner's transaction on both dialects and only adds columns. currencies is declared as invoice_line_templates is (jsonb NOT NULL DEFAULT '[]' on Postgres, text NOT NULL DEFAULT '[]' on SQLite, the entity's Json): every existing row offers any currency, so no tenant or book changes behaviour. updated_by is nullable and declared as the dialect's other uuid columns are (uuid on Postgres, text on SQLite, where the application stores a 16-byte blob). Its up skips a column that is already there, and its down drops the two columns. The upgrade tests run the gear's list without 000014, seed a settings row, apply 000014 alone and prove exactly two added columns on both dialects; the row survives, and the application reads and writes it again. The schema goldens gain exactly these two columns. m20260926_000013's upgrade tests pass with 000014 in their before state.
- **currencies.** PUT /bss-pricing/v1/settings requires currencies, a full replace; [] offers any currency, and a body without the field is 400, as any missing field is. Each code is spelled as a book's currency is (three uppercase ASCII letters; the workspace holds no ISO 4217 list, domain::book::currency_code) and appears once, else 400 CURRENCY_INVALID. The list is stored in the order sent. POST /bss-pricing/v1/price-books with a currency outside a non-empty list is 409 CURRENCY_NOT_OFFERED, judged after the book's own 400s. The settings are read tenant-scoped, so the book author needs no config grant. Existing books are untouched: the list restricts new books only.
- **Who and when.** The settings answer carries currencies, updated_at and updated_by. At version 0 (nothing written) both updated_at and updated_by are null. updated_by is also null on a row written before 000014. Every PUT stamps both: the time of the write and the caller's subject id.

Breaking: the PUT's body (currencies required), and a GET answer is a PUT body only without version, updated_at and updated_by.

**Source:** Owner, 2026-09-27; phase 6 plan rev 2 (asks 11f, 11g; plan review L6, L7).

#### D-439 [M] Closed sets are enums on the responses; requests keep strings and their codes (twin of products P-D-217)

**Status:** DECIDED 2026-09-27.

- **Responses.** Every closed set a response schema carries is an enum in the served OpenAPI. It holds exactly the tokens that the column stores and that the wire always carried, so the wire does not change and the golden contracts are not recorded again. The sets: an entry's charge_kind (recurring, usage, one_time), period (month, year), model (flat, per_unit, graduated, volume, package) and reference_state (confirmation_pending, confirmed, lost); a price's model, eligibility (all, new), state (draft, pending, approved, rejected) and display status (draft, pending, rejected, scheduled, active, superseded); an item's treatment (paid, optional, included) and reference_state (unreserved, confirmation_pending, confirmed, lost); a revision's state (draft, pending, scheduled, published, superseded; `scheduled` since D-446), and the resolved revision's (published, superseded, scheduled; `scheduled` since D-454); a reference op's kind, state and ref_kind; a unit's state, a decision (approve, reject) and a vote's outcome (pending, applied, rejected, withdrawn); the settings' default_timing (advance, arrears); a resolved input's source (entry, sku, tenant). The same sets appear in resolve (DESIGN §3.3, slice 07 §6) and in the pinned price read. Each set is one schema component with the Pricing prefix (api/rest/closed_sets.rs). A set with a domain enum maps to and from it, so a value added on one side only does not compile.
- **Stored values.** A database CHECK holds each stored set on both dialects: charge_kind, period (the entry's CHECK: month or year for a recurring entry, null for the others), model, the reference states, eligibility, the price's state, treatment, the revision's state, the reference op's three columns, the unit's state, the decision and default_timing. The read is fallible. Only a writer that goes around the CHECK can store a token outside its set, and the read answers it with CorruptRow, which names the row and the token: a 500 with the detail logged, never a panic and never a value that the enum does not hold. The display status, the vote's outcome and the source are computed from typed values.
- **Requests keep string.** A request field over the same set stays string in its schema, and its door judges it, so each field keeps its refusal code (D-403): MODEL_INVALID, ENTRY_PERIOD_INVALID, ELIGIBILITY_INVALID, TREATMENT_INVALID, TIMING_INVALID and ROUNDING_INVALID. A request enum would fail at deserialization, before the door, with a 400 that has no code. vhp-core asserts MODEL_INVALID, and spec-check P3 would find codes that are declared and never raised.
- **Response fields that stay string.** default_rounding and resolve's rounding_policy: no CHECK guards the column (D-437), and a legacy value reads back as stored. A normalizing migration with a CHECK is not made here: the phase takes only ADD COLUMN migrations, a CHECK on an existing SQLite column needs a table rebuild, and the deployment pre-flight (D-437) is the gate. A unit's kind and ref_type: the shared approval tables (bss_approval::ddl) have no CHECK on them, for the same reason. A check's code is a code vocabulary that the check builders write as literals, and a proposal's chain is a dimension value or default; neither is a closed set of stored values.
- **Proof.** tests/response_enums.rs reads the served spec: every listed field has its enum with the exact values in order and its nullability, no request body reaches an enum, and the fields above stay plain strings. An entry row poisoned on SQLite (the CHECK refuses the write; the test then bypasses it) reads 500, and the gear goes on serving. Unit tests pin each set's schema, wire and stored tokens as one list.

D-467 amends this entry: a plan item's treatment is no longer on any response or request, so its enum and TREATMENT_INVALID are gone; the column's CHECK stays and still holds the stored rows.

**Source:** Owner, 2026-09-27; phase 6 plan rev 2 (ask 12; plan review M5). Amended by D-467.

#### D-440 [M] An entry's prices, its price in force and its approved prices by date

**Status:** DECIDED 2026-09-28.

The Price Books screen opens a drawer per entry with its dated prices (ask 18). The only source was the whole-book export, and usage.prices.approved lumped the active, scheduled and superseded prices together.

- **The entry's prices.** GET /bss-pricing/v1/price-book-entries/{id}/prices answers PricingEntryPriceList { items }, each item a PricingPriceDto: every price of the entry in every state, each with its display status on the day of the request (domain::price::window_display: draft, pending or rejected as stored; an approved price is superseded when its window ended on or before today, scheduled when it starts after today, and active otherwise). The order is the default chain first, then each dimension value's chain in ascending order of the value; each chain by effective_from, then version_no, then id, as the export orders it. status keeps one status or several, comma-separated (status=draft,scheduled). An unknown or empty value, a repeated status and any other key are 400 QUERY_INVALID.
- **The grant.** The whole answer is money, so the grant is judged twice (plan review H1). price_book_entry read reaches the entry: 403 without it, and 404 ENTRY_NOT_FOUND for an entry the tenant does not hold, judged before the money. Then price_book read, judged a second time as D-434 judges it, must admit the entry's book: 403 PRICE_BOOK_READ_REQUIRED without the grant, or when its scope does not admit that book; 503 when the policy cannot judge. The order: 403 for entry read, 503 for the money's policy, 400 for the query, 404, then 403 for the money. The read makes three statements whatever the number of prices: the entry, its book under the grant, and its prices. The QueryRecorder shows the same statements for 10 and for 100 prices.
- **The price in force.** The two entry reads, GET /price-book-entries/{id} and GET /price-books/{id}/entries, carry current_price with D-434's value, shape and grant rule: the default chain's approved price in force today (a PricingPriceDto), or null when none is in force or when the caller's price_book read does not admit the entry's book. An unavailable policy fails the read with 503. The list judges its one book once. D-434's list and these reads share one function that chooses the price (in_force) and one that judges the grant (money_scope).
- **The approved prices by date.** usage.prices gains scheduled, active and superseded beside approved, pending and draft, so approved = scheduled + active + superseded for every entry. The tests prove it over value chains, a temporary price in force with its scheduled return, an ended gap and a price that ends today. The split comes from the same ONE grouped count that D-428 reads: per entry and state, it also sums the prices whose window ended on or before today and those not ended that start after it, in window_display's order (superseded first), with a UTC today bound once per request. So no usage read makes a statement more, and D-434's SKU entry list still makes seven. Every price the same request answers takes the same day. The split is pricing's own: products-sdk's PriceCounts, the SKU usage port, the SKU card and the SKU list are unchanged (plan review M1). The SKU's entry list (D-434) carries the same usage.
- **The frozen contract.** The golden price_book_entry_usage leaves out the three dated counts and current_price, because a frozen document holds no value of today. tests/book_reads.rs pins them.

Breaking for a consumer that compares usage as a closed object: it gains three fields, and the entry reads gain current_price. The gears-rust e2e is adapted in run 7.1; the vhp-core suite follows in run 7.4.

D-456 extends this entry: plan create, clone and a book-naming revision PATCH judge the same second price_book read on the book the plan names.

**Source:** Owner, 2026-09-28; phase 7 plan rev 2 (ask 18; plan review H1, M1, L4). Amends D-428 and D-434. Extended by D-456.

#### D-441 [M] Every book read carries its stats

**Status:** DECIDED 2026-09-28.

The Price Books screen lists books with their prices, SKUs, plans, pending changes and last change (ask 14). One request per book could not give distinct plans or a last change.

- **The shape.** GET /bss-pricing/v1/price-books and GET /bss-pricing/v1/price-books/{id} answer PricingPriceBookReadDto: the book's fields (PriceBookDto, flattened) and stats { entries, skus, plans, plans_superseded_only, prices { draft, pending, approved, scheduled, active, superseded, rejected }, pending_units, last_change_at }. The write answers (POST, PATCH, the stored receipt), the export and publish-changes keep PriceBookDto (plan review L8). The ETag of GET /price-books/{id} is unchanged.
- **The counts.** entries counts the book's entries in every reference state, and skus their distinct SKUs. plans counts the distinct plans with a draft, pending, scheduled or published revision whose book_id is the book, from the stored state (D-453). A plan whose revisions on the book are all superseded is not counted, and two revisions of one plan count once. plans_superseded_only counts the distinct plans that name the book only through superseded revisions: the plans with a revision of any state on the book, less those plans counts. Both come from plan_revision_repo::plans_on_books, one grouped statement, which is also the one read by which the book delete judges BOOK_IN_PLAN and BOOK_IN_PLAN_HISTORY (D-444). So plans = 0 and BOOK_IN_PLAN never disagree (plan review M5), and entries, plans and plans_superseded_only are all 0 exactly when the delete succeeds (If-Match aside). plans_superseded_only was added in the phase 7 fix run (additive); it is the book's twin of D-428's entry count of the same name. prices counts the prices of the book's entries by state. Unlike D-428's entry counts, a rejected price is counted, and the approved ones are also split as D-440 splits them. pending_units counts the book's prices units in review; a prices unit's ref_id is its book, and a unit of another kind is never counted.
- **The last change.** last_change_at is the latest of the book's updated_at, its entries' and their prices' updated_at, and its prices units' submitted_at and decided_at (plan review M4). A submit, a withdraw or a reject writes no updated_at, so it moves the last change through its unit. A deleted draft or entry leaves no row, so its deletion does not move it. The instants are compared as instants. On Postgres the maximum of a timestamptz is rendered in UTC to the microsecond that Postgres keeps. On SQLite, where RFC 3339 text does not sort as time within one second (P-D-213: …00Z after …00.5Z), the maximum is taken over a fixed-width key that pads the fraction to nine digits. Pricing writes every instant in UTC.
- **The statements.** A fixed number of grouped statements, one per source and none multiplying another (plan review M3): the entries (count, distinct SKUs, latest); the prices, each joined to its one entry (by book and state, with the dated sums and the latest); the plans that name the book (the live ones and all of them, in one grouped statement); and the prices units (pending, latest). A page makes its own statement and these four, and one book's read makes the book's statement and these four. The QueryRecorder shows the same five statements for 10 and for 100 books ($top=200), half of them named only by superseded revisions, and one book's read makes the same statements whether a plan holds it or only history does. Every source is read tenant-scoped: the counts are facts of a book the caller may read (price_book read), as D-428 reads an entry's usage.

**Source:** Owner, 2026-09-28; phase 7 plan rev 2 (ask 14; plan review M3, M4, M5, L8); phase 7 review (behaviour lens: plans_superseded_only). Amended by D-453.

#### D-442 [M] The book list pages on the toolkit's OData pager, searched by q and sku_id

**Status:** DECIDED 2026-09-28.

GET /price-books took no parameter and answered every book (ask 15). It now pages on the toolkit's OData pager, as the Products SKU list does (P-D-210).

- **The query.** $filter reads code, name, currency, valid_from and valid_until. The two dates are nullable: eq null is a book open on that side, and they filter only, never order. $orderby reads code and name, tie-broken by id, and the default order is the code. $top (alias limit) defaults to 200 and is clamped at 500, the categories' page, so a tenant's books stay on one page (plan review M2). cursor (alias $skiptoken) comes from page_info.
- **q and sku_id.** q is a case-insensitive substring of the code or the name, matched literally (%, _ and \ are escaped). On Postgres both sides fold through the ICU root collation und-x-icu, whatever the database's locale: a C database's own lower() folds ASCII only. The deployment's Postgres must be built with ICU, as P-D-210 says. On SQLite both sides fold through lower(), ASCII only. An empty q is no search. sku_id keeps the books with an entry of that SKU, in any reference state; it is a sub-select condition of the page's own statement. The cursor carries a hash of $filter, q and sku_id, so a cursor replayed under another narrowing is 400 FILTER_MISMATCH.
- **Refusals.** Authorization is judged first (price_book read). Any other plain key, a repeated key and a malformed sku_id are 400 QUERY_INVALID, the code pricing's other reads give a key (plan review M2 left the choice). The toolkit refuses $select, $count and every other option it does not take with UNSUPPORTED_QUERY_PARAM, and a filter or an order it cannot read with INVALID_FILTER or INVALID_ORDERBY_FIELD.
- **The answer.** The list answers `Page<PricingPriceBookReadDto>`: { items, page_info { next_cursor, prev_cursor, limit } }, each item with its stats (D-441). There is no total: the toolkit Page carries none, and the stats are the pattern for counts (plan review M6). Pricing takes the toolkit-odata dependency.

Breaking: the list pages, where it was unlimited, so a tenant with more than 200 books reads the rest through next_cursor; and an unknown key, which was ignored, is 400. items keeps its place and page_info is added. The deploy notes name both.

**Source:** Owner, 2026-09-28; phase 7 plan rev 2 (ask 15; plan review M2, M6, L3).

#### D-443 [M] A temporary draft's dates move, and its pair follows

**Status:** DECIDED 2026-09-28.

The Price Books screen edits a temporary draft's dates (ask 21). PATCH /prices/{id} refused every date of a temporary price with TEMPORARY_PRICE_FIXED, so the author deleted the pair and drafted it again.

- **The door.** PATCH /bss-pricing/v1/prices/{id} of the temporary half of an unlocked draft (the price that carries temporary_until) takes effective_from and temporary_until, by its author and under If-Match, as before. A PATCH that sends either date runs the pair builder (domain::price::temporary) again over the new dates, in the same transaction.
- **The shapes.** The builder makes a pair when a price of the chain is in force on the end (the return restores it); the promo alone when the chain's next approved price starts exactly on the end; and one explicitly closed price when nothing of the chain is in force there. Every move between them is reconciled. Pair → pair: the return is derived again in place (its id, version_no and author stay; its start is the new end; its money and min_fee are copied again from the price it restores, and its end from that price's own end). Pair → one price: the return is deleted. One price → pair: a return is created. One price → one price: the promo's end and closed_explicitly are written again.
- **The write order.** The pair references are foreign keys. Pair → pair: the promo at its version, then the return at its own version. Pair → one price: the promo first, its paired_price_id cleared, then the return is deleted at its version, with a price.delete audit row. One price → pair: the return first, naming the promo (which exists), then the promo names it; the return has a price.create audit row. A new return takes the entry's next version_no (the highest of all the entry's prices, plus one), never promo.version_no + 1, which another price may hold (the unique (price_book_entry_id, version_no) index). A concurrent writer that takes the number first makes the PATCH try again, as the create does.
- **The judgement.** Both halves are judged as the create judges them: each price's own rules (for example WINDOW_END_INVALID, WINDOW_START_IN_PAST, WINDOW_OVERLAP) and D-406 against the approved prices and the pair itself (PRICE_INSIDE_TEMPORARY, TEMPORARY_SPANS_A_CHANGE). A refusal writes nothing.
- **The answer.** 200 with the edited price: its paired_price_id names its partner now, or null, and the ETag is its new version. A kept return's version moves too, so a client reads it again before it edits the return.
- **What stays fixed.** TEMPORARY_PRICE_FIXED (400) is narrowed to: a return's own dates (the author edits the temporary half); the chain (dim_value) of either half; temporary_until on a price that is not the temporary half; and a null temporary_until. No PATCH makes a price temporary or ends its temporariness. A pair whose other half is not an unlocked draft is 409 PRICE_NOT_DRAFT, and another author's is 403 NOT_DRAFT_AUTHOR, as the delete judges them.
- **An edited return.** A return's money stays editable, and a move of its pair copies the money again from the restored price. Submit would refuse the edited return as PAIR_RETURN_STALE anyway (D-391), so the copy loses nothing that submit would keep.

**Source:** Owner, 2026-09-28; phase 7 plan rev 2 (ask 21; plan review H2, L5). Amends D-391.

#### D-444 [M] A book has a description, and an unused book can be deleted

**Status:** DECIDED 2026-09-28.

The Price Books screen shows a book's description and deletes a book that nothing uses (ask 16). A book had no free text, and no door removed a book.

- **The description.** Migration m20260928_000015_book_description adds pricing_price_book.description, a nullable text column, by ADD COLUMN on both dialects (a book written before it reads null). POST /price-books takes an optional description; PATCH /price-books/{id} keeps it when the field is omitted, replaces it with a value and clears it with null. It holds at most 2000 characters (Unicode scalar values), judged at the door: 400 BOOK_DESCRIPTION_TOO_LONG; there is no CHECK. It is stored as sent. Every book answer carries it: PriceBookDto (the write answers, the stored receipt, the export, publish-changes) and the book reads, which flatten it (D-441). A stored POST receipt younger than a day replays the answer it recorded, without the field.
- **The delete.** DELETE /bss-pricing/v1/price-books/{id} under the book write grant (price_book author) and If-Match answers 204 and writes a price_book.delete audit row. The refusals come in this order: 403 without the grant (authorization first); 400 for a missing or malformed If-Match; 404 for a book the tenant does not hold; 409 STALE_REVISION; 409 BOOK_HAS_ENTRIES for an entry of the book in any reference state; 409 BOOK_IN_PLAN for a plan with a draft, pending, scheduled or published revision on the book (D-453); 409 BOOK_IN_PLAN_HISTORY when only superseded revisions name the book. Both are judged from plan_revision_repo::plans_on_books, the read D-441's stats.plans and stats.plans_superseded_only count, and BOOK_HAS_ENTRIES from the read stats.entries counts. So the delete succeeds exactly when stats.entries, stats.plans and stats.plans_superseded_only are all 0, and a screen knows beforehand which refusal it would meet.
- **Superseded revisions keep the book.** A revision's book_id is a foreign key, and a revision may name a book without any entry of it (a revision moved to another book, or one with only included items). A book that only superseded revisions name has stats.plans = 0 and stats.plans_superseded_only > 0, and its delete is still refused: the history of those revisions keeps it. That refusal has its own code, BOOK_IN_PLAN_HISTORY, so BOOK_IN_PLAN keeps D-441's meaning.
- **No refusal for a pending unit.** A prices unit in review holds pending prices; a pending price keeps its entry (ENTRY_PRICES_IN_USE), and BOOK_HAS_ENTRIES is judged first. A plan-revision unit names its revision, which is not superseded while it is pending (BOOK_IN_PLAN). So no unit can be pending on a book without entries, and the plan's BOOK_LOCKED_PENDING is not a refusal (plan review L1).
- **A lost race.** A row that a concurrent writer adds after the door's reads meets the book's foreign key: Postgres waits for that writer and fails the delete on the key it names, and the entry's key is 409 BOOK_HAS_ENTRIES, a revision's 409 BOOK_IN_PLAN, never a 500. On SQLite one writer holds the database for the whole transaction, so the door's reads see every row. An entry create in flight loses cleanly the other way: its Tx B finds no book (BOOK_NOT_FOUND), which cancels the op and releases the reservation.
- **What stays.** The book's audit rows stay. A decided unit (rejected or withdrawn) that named the book stays readable: its card and the unit list answer with its ref_id, its decisions and the impact of its stored items, without the book. An approved unit's prices keep their entries, so its book is never deleted. Like a deleted draft SKU's create (P-D-206), the book create's Idempotency-Key still replays its 201 for a day, naming the deleted book; the code is free again.

**Source:** Owner, 2026-09-28; phase 7 plan rev 2 (ask 16; plan review L1, L8); phase 7 review (behaviour lens: the stats predict the delete). Amended by D-453.

#### D-445 [L] An approval unit carries its submitter's note (twin of products P-D-219)

**Status:** DECIDED 2026-09-28.

The approval library's unit now carries `submit_note`, the submitter's own words, which products' submit doors take (P-D-219, ask 4b). Pricing shares the unit shape and the approval DDL, so it gets the column too.

- **The column.** Migration m20260928_000016_unit_submit_note adds pricing_approval_unit.submit_note, a nullable text column without a default or a CHECK. It uses the library's separate step, bss_approval::ddl::apply_add_submit_note: ADD COLUMN with IF NOT EXISTS on Postgres, and a catalog check first on SQLite, so it replays. The library's ddl::up() is the body of m20260926_000002 and stays as it shipped (plan review H3). down drops the column the same way. A unit written before the migration reads null.
- **The reads.** GET /approval-units, GET /approval-units/{id} and every answer that carries a unit (the submit, vote and publish-changes receipts) carry submit_note.
- **No note from pricing's doors.** POST /prices/{id}/submit, POST /price-books/{id}/publish-changes and POST /plan-revisions/{id}/submit take no note, and their bodies are unchanged. So submit_note is null on every pricing unit. A door that takes a note later passes it through the library's SubmitRequest.note.
- **Not content.** The note is not part of the snapshot or of snapshot_hash, and a stale refresh keeps it.

D-464 amends this entry: POST /plan-revisions/{id}/submit and POST /price-books/{id}/publish-changes take an optional note, which they pass through SubmitRequest.note to the unit's submit_note; POST /prices/{id}/submit still takes none. So submit_note is null on a pricing unit only when its submitter sent no note.

**Source:** Owner, 2026-09-28; phase 7 plan rev 2 (ask 4b; plan review H3). Amended by D-464.

#### D-446 [M] A plan revision can be stored scheduled: the state, its index and its migration

**Status:** DECIDED 2026-09-29.

The owner approved scheduled plan revisions (2026-09-28): a plan change is approved today and takes effect on a later date. Until that date the plan keeps selling its current revision.

- **The state.** A revision state `scheduled` joins draft, pending, published and superseded. An approved revision whose sale date (`available_from`) is after the apply's day is stored `scheduled`: its lock is cleared, `approved_by_unit_id` names the unit, and `published_at` stays null. The plan's published revision and its `published_rev` do not move. A null or past `available_from` publishes at once, as before. A revision that was published at once before this entry stays published: there is no backfill. The apply makes this choice (D-449), and the switch on the date is announced by whoever persists it (D-450, D-451).
- **Why a stored state.** The published partial unique index admits one published revision per plan. A reader that filters `state = 'published'` does not see a scheduled revision, so no reader sells the future revision early. A reader that filters `state <> 'superseded'` (the BOOK_IN_PLAN family, D-441) counts it as a holder of its book and its entries, which is correct. The alternative, "published with a null published_at", would need a second test in every revision reader, and one missed reader would sell the future revision today.
- **One per plan.** The partial unique index pricing_plan_revision_scheduled ON (plan_id) WHERE state = 'scheduled' admits one scheduled revision per plan. A second one is 409 REVISION_SCHEDULED_EXISTS. Postgres names the index; SQLite names only the column, so the write that sets the state tells the two single-column indexes apart.
- **The migration.** The chain is deployed, so the change is the forward migration m20260929_000017_revision_scheduled. Postgres drops chk_pricing_plan_revision_state and adds it again with 'scheduled' (DROP CONSTRAINT IF EXISTS, so a replay changes nothing), then creates the index. SQLite cannot change a CHECK. The toolkit runner runs each up() in a transaction, where PRAGMA foreign_keys=OFF has no effect, so the SQLite arm rebuilds the family without a PRAGMA, as products m20260925_000007 does (P-D-196). The family is pricing_plan_revision and its only child, pricing_plan_item. The steps: create both new tables (000011's and 000012's text, word for word, except the wider CHECK), copy every row, drop the child and then the parent, rename the new tables, recreate the two partial indexes with their original text, and create the new index. down() is an explicit irreversible error.
- **The proof.** Upgrade tests on both dialects go through the real runner, from a database migrated without 000017, with plans, revisions in every old state and items. Only 000017 is pending. The dump differs by the CHECK and the new index only, each table's text by the CHECK only, and every row and foreign key survives. The CHECK admits scheduled and refuses an unknown state, and the new index refuses a second scheduled revision. An upgraded database equals a fresh one, and a replay applies nothing. The schema goldens were recorded again once: only the CHECK and the index changed. The schema guard needs no change.
- **The wire.** The closed set PricingRevisionState gains scheduled, so no read answers 500 on a stored scheduled row. Run 8.1 rendered the stored state everywhere; run 8.2 derives the effective state in every read that renders one (D-453), and /resolve serves a scheduled revision from its date (D-454).
- **The counts.** The counts read the stored state: stats.plans and BOOK_IN_PLAN (D-441, D-444), the SKU's usage.plans (D-434), the entry usage (D-428) and the plans that name a SKU. They count a scheduled revision as they count a published one. Once it is due, they also count its stored-published predecessor until the switch is persisted. The switch job ends that over-count: it persists at most the ticker's limit (100) due revisions a cycle (a minute), oldest available_from first, so a backlog of more than 100 drains 100 a cycle, and a plan whose switch keeps failing over-counts until it is repaired (D-450, D-453).
- **Deploy and rollback.** An old pod refuses a stored scheduled row in its closed sets and answers 500 for that tenant's plan list, SKU usage and item reads, and its copy door can open a draft beside the waiting revision. So the deploy is not a rolling update (plan rev 2 H2). First dump the pricing and products tables, both runner history tables and both gears' outbox tables: a restore without the outboxes sends their events again. Then scale core-server to 0, set the image and scale it to 1. A plain set image back is not safe once one scheduled row exists. A rollback first finds the rows with state = 'scheduled' and unschedules each one that is not yet due. A row that is already due cannot be unscheduled: the door answers 409 REVISION_IN_EFFECT, and its catch-up rolls back with the refusal. The job switches such a row, and it is then an ordinary published revision that the old code reads. So the image goes back only when no row with state = 'scheduled' is left. The other rollback is to restore the dump.

**Source:** Owner, 2026-09-28 (scheduled plan revisions); phase 8 plan rev 2 (decisions 1, 2, 10; plan review H2, L3, L4, L5, L8); phase 8 review (behaviour B4; docs L2, L3).

#### D-447 [M] A scheduled revision takes effect on its date: the effective state is derived

**Status:** DECIDED 2026-09-29.

A scheduled revision must take effect on its date with nobody acting and before any job runs. So every read derives the effective state from the stored rows, as a price's display state is derived (domain::price::status).

- **The rule.** domain::plan::effective(revisions, today) is a pure function. A scheduled revision whose available_from is on or before today reads as published, with published_at at 00:00 UTC of available_from. The stored-published revision of its plan reads as superseded and keeps its own published_at. The plan's published_rev reads as the due revision's rev_no (domain::plan::published_rev). Every other revision reads as stored. One list may hold the revisions of many plans; plans are told apart by plan_id.
- **Today.** Today is the UTC date of now, as the checks use it (domain::plan::sale_date).
- **A revision with no date.** A scheduled revision with a null available_from is never due. The storage writes none, because the apply schedules only a future date. The persisted switch (D-448) reads such a row the same way.
- **Reads never write.** The derivation runs in memory, so a GET door writes nothing. The plan read and list and the checks derive over the revisions that they read already, so their statement count does not change. The revision read, the item read and /resolve read their plan's revisions once more, and the impact reads the revisions of its plans in one statement (D-453).
- **The persisted switch agrees.** D-448's switch writes the same states and the same published_at. Only the two revision rows' version and updated_at change when the switch is persisted; the plan row does not change (D-448).
- **Where it applies.** The plan read and list, the revision read, the item read, /resolve and the prices unit's live impact use it (D-453, D-454; plan rev 2 M2, M3). So do the checks: the published revision that a deprecated SKU may be carried from (D-408) is the one in effect today.

**Source:** Owner, 2026-09-28; phase 8 plan rev 2 (decision 3); phase 8 review (behaviour B2; docs L1).

#### D-448 [M] The storage writes of a scheduled revision: schedule, switch, unschedule and the due scan

**Status:** DECIDED 2026-09-29.

The doors and the job of run 8.2 call four storage functions of plan_revision_repo. Each one is conditional on the state it reads, so a lost race changes nothing.

- **schedule(tenant, id, unit, now).** A pending revision that the unit holds becomes scheduled. pending_unit_id becomes null, approved_by_unit_id names the unit, the version increases and updated_at is now. published_at stays null. Any other state, or another unit's revision, is 409 REVISION_NOT_PENDING, and a second scheduled revision of the plan is 409 REVISION_SCHEDULED_EXISTS. The write does not read the date: the apply decides between schedule and publish (D-446).
- **switch_due(tenant, plan_id, now).** This persists the plan's due switch in the caller's transaction. First it finds the due scheduled revision (state scheduled, available_from on or before the UTC date of now). Then it supersedes the stored-published revision, publishes the due one with published_at at 00:00 UTC of its date, and writes published_rev through plan_repo::advance_published. That write changes neither the plan's version nor its updated_at: published_rev is a projection, so an If-Match that a client read before the switch stays valid (plan rev 2 L6). The two revision rows increase their version and set updated_at, as every write does. The result names the superseded revision (none for a plan's first publication), the published revision, its unit, its rev_no and its book: what a PlanRevisionPublished event names. There is a result only when the update from scheduled to published changed its row. Nothing due, or a switch that another writer made first, returns nothing and writes nothing. A predecessor superseded with no revision published after it is refused with 409 STALE_REVISION and not committed. A scheduled row that names no approving unit is a corrupt row, and nothing is written.
- **unschedule(tenant, id, now).** A scheduled revision that is not yet due (its available_from is after the UTC date of now, or null) becomes an unlocked draft. approved_by_unit_id becomes null, the version increases, and its items stay. The state condition is the concurrency control; there is no If-Match (plan rev 2 M5). Any other state, or a due revision, is 409 REVISION_NOT_SCHEDULED. A draft or pending revision beside it is 409 REVISION_DRAFT_EXISTS (the open index). The applied unit stays applied in its history.
- **due_scheduled(today, limit).** This is the job's scan: the due scheduled revisions of every tenant, ordered by available_from and then id, at most limit. It reads across tenants on purpose (AccessScope::allow_all(), as the reference ticker's scans do). The job then switches each plan in its tenant's scope.
- **Now, not today.** switch_due and unschedule take the instant now and not a date. The revision rows need it for updated_at, and today is always its UTC date, so a caller cannot pass two times that disagree.
- **The callers.** The apply calls schedule (D-449). plan_revisions::catch_up calls switch_due, enqueues the event and writes the audit row plan_revision.switch under the system actor (plan rev 2 L2), for the ticker duty (D-450) and the copy, clone and unschedule doors (D-451). The unschedule door calls unschedule (D-452).
- **The tests.** Each write's from-states, an idempotent switch, a switch that races itself (two connections on SQLite, two pools on Postgres: exactly one switches), a second scheduled revision refused, the plan's version unchanged by a switch, and the due scan, on both dialects. A probe of each behaviour was armed, caught by these tests, and reverted.

**Source:** Owner, 2026-09-28; phase 8 plan rev 2 (decisions 4, 5, 7; plan review L2, L6, M5).

#### D-449 [M] An approval before the sale date schedules the revision

**Status:** DECIDED 2026-09-29.

A plan change can be approved today and take effect on a later date (D-446). The apply decides which.

- **The choice.** The apply of a plan_revision unit judges the checks again, as before. Then it compares the revision's available_from with the UTC day of the apply. A later date stores the revision scheduled through plan_revision_repo::schedule (D-448): nothing is superseded, the plan's published revision and published_rev do not move, and the subject's superseded() stays None. A null date, or a date on or before that day, publishes at once, as before.
- **The unit.** The unit is applied in both cases: it is approved, its ApprovalUnitDecided is enqueued, and the receipt says applied. The receipt's revision reads scheduled, with a null published_at. Quorum 0 applies at the submit with the same choice.
- **No event at the apply.** The approval door enqueues PlanRevisionPublished only for a revision that its apply published now (PlanRevisionSubject::published_now). A scheduled revision is announced at its switch (D-450).
- **The tests.** A future date schedules through an approving vote and at quorum 0; a date of today publishes at once, and the tests of a null date stay green unchanged.

**Source:** Owner, 2026-09-28; phase 8 plan rev 2 (decision 1, run 8.2).

#### D-450 [M] The switch job persists a due switch on its date and announces it once

**Status:** DECIDED 2026-09-29.

Every read shows a due switch at once (D-453). The job makes it exact in storage and announces it.

- **The duty.** The pricing ticker runs a switch duty first in its tick: on the first tick, then every 60 ticks (reference_ticker::SWITCH_EVERY), about once a minute at the one-second tick. The period is fixed in the code, as the ticker's other knobs are; BssPricingConfig has no key for it (plan rev 2 M4).
- **Its own error handling.** The duty never fails the tick. A failed scan, or a failed plan, is a warning, and the other plans go on. It runs before the reference duties, so their early returns and a failing reconciliation cannot skip it. A test pins this with a reconciliation that fails because the registry is absent. The tick count moves right after the duty and before the reference duties, so a reference scan that keeps failing cannot hold the count off the duty's period either. A second test fails the reference-op scan on 180 ticks in a row (its table is renamed), and the due revision is switched on the 60th of them.
- **The scan.** plan_revision_repo::due_scheduled on the UTC day of the ticker's clock, across tenants (allow_all), at most the ticker's limit (100), by available_from and then id. Each plan is switched once per cycle, in its own serializable transaction, in its tenant's scope. A due revision beyond the limit waits for the next cycle.
- **One transaction per plan.** plan_revisions::catch_up runs switch_due (D-448). Only when it switched does the same transaction enqueue PlanRevisionPublished and write the audit row plan_revision.switch under pricing's system actor (plan rev 2 L2). A switch that another writer made first returns nothing and announces nothing, so each switch is announced exactly once.
- **The event.** It has the apply's type and shape: the plan, the revision, its rev_no and its book, superseded_revision_id from the switch (null for a plan's first publication), and unit_id, the revision's approved_by_unit_id. actor_ref is the actor of the unit's latest approving decision that is not stale and is of the unit's current generation. At quorum 0 the unit records no decision, so actor_ref is the unit's submitter (plan rev 2 H1). It is never the system actor.
- **No second judgement.** The switch does not run the checks again. The apply judged the revision on its sale date. The reference registry fences a SKU. An approved price chain loses its coverage only to a successor. A book's dates can move under a published revision today, and GET /plan-revisions/{id}/checks shows red in both cases. A value chain that ends explicitly is the same case (plan rev 2 L7). A SKU that is deprecated between the apply and the date is not. While the revision waits, its checks show ITEM_SKU_DEPRECATED red when the published revision does not carry the SKU. From the date the revision is in effect and carries the SKU itself, so the row is ok (D-408). The checks derive the revision in effect (D-447), so the row does not change when the job persists the switch.
- **Pins.** A switch moves no subscription pin (D-394).
- **A corrupt row.** A scheduled revision whose unit is gone fails its plan's switch, which is tried again every cycle, while the reads already show the switch. It keeps its place in the due scan's order, so once it is among the 100 oldest due revisions it takes one of the limit's places in every cycle until it is repaired.

**Source:** Owner, 2026-09-28; phase 8 plan rev 2 (decision 5; plan review H1, M4, L2, L7); phase 8 review (behaviour B1, B2, B4).

#### D-451 [M] The copy, clone and unschedule doors catch a due switch up; one scheduled revision at a time

**Status:** DECIDED 2026-09-29.

- **Where.** POST /plans/{id}/revisions (the copy), POST /plans/{id}/clone and POST /plan-revisions/{id}/unschedule call plan_revisions::catch_up first in their transaction: after the key's claim, before they judge anything. A catch-up that switched enqueues the job's event and audit row; the audit row carries the request's correlation. When the door then refuses, the catch-up rolls back with it, and the job persists the switch later.
- **The copy.** After the catch-up, a revision that waits for its sale date refuses a new draft: 409 REVISION_SCHEDULED. Withdraw it (D-452) or wait for its date. A due revision has just been switched, so the copy is of the revision in effect. *Counter-argument:* an operator cannot prepare the next change while one waits. Accepted for now: a chain of future revisions is a separate design.
- **The clone.** The clone copies the stored published revision. After the catch-up it is the revision in effect, so from 00:00 UTC of the date the clone copies the new revision, also when the job is down.
- **Nowhere else (plan rev 2 M1).** The draft's PATCH and DELETE, the item writes, the submit and the apply never meet a scheduled sibling. The open index admits one draft or pending revision per plan, the copy refuses a new draft while a revision waits, and unschedule turns the waiting revision itself into the draft. A catch-up in those writes would be dead code.
- **The invariant.** No draft or pending revision stands beside a scheduled one. A test drives every door that opens or moves a revision (the copy, the draft's PATCH and DELETE, the submit, the item writes, the votes on the applied unit, the clone and the plan create) and then finds the plan with exactly its published and its scheduled revision.

D-463 amends this entry: the clone copies the source's sale date unless its body names another date, which overrides it, or null, which clears it.

**Source:** Owner, 2026-09-28; phase 8 plan rev 2 (decisions 4, 6; plan review M1). Amended by D-463.

#### D-452 [M] A scheduled revision can be withdrawn to a draft

**Status:** DECIDED 2026-09-29.

- **The door.** POST /bss-pricing/v1/plan-revisions/{id}/unschedule, under plan submit (D-418): it withdraws an approved change, which plan author alone may not do (plan rev 2 M5). It takes an Idempotency-Key with the replay store's claim and answer, as every pricing POST does (D-396), and no body. It takes no If-Match: its write is conditional on the state scheduled, and that is its concurrency control.
- **The order.** Authorization, the key's claim, the revision (404 for one the tenant does not hold), the plan's catch-up (D-451), then the state.
- **The codes, by the state after the catch-up.** A published revision, stored so or due and just switched, is 409 REVISION_IN_EFFECT. A draft, pending or superseded revision is 409 REVISION_NOT_SCHEDULED.
- **The answer.** 200 with the draft (PricingPlanRevisionDto): approved_by_unit_id null, the version moved, its items and their references kept. Its ETag is its version, which its PATCH takes. The audit row is plan_revision.unschedule, under the caller. There is no event, and the applied unit stays applied in its history.
- **After it.** The draft is edited as any draft is: D-404's author rule is unchanged, so its created_by edits it. It is submitted again as any draft is; without a date it publishes at once.

**Source:** Owner, 2026-09-28; phase 8 plan rev 2 (decision 7; plan review M5).

#### D-453 [M] Every read derives the effective state; the counts read the stored state

**Status:** DECIDED 2026-09-29.

- **The reads.** Every read that renders a revision's state renders the state it reads today (D-447): domain::plan::effective over its plan's stored revisions. These are the plan read and list (each revision header's state and published_at, and published_rev through domain::plan::published_rev), GET /plans?sku_id=, the revision read, the item read's state, and the prices' impact plans (the approval queue's card and list, the publish-changes preview). A prices unit's stored snapshot records the impact of its own day and stays history. /resolve is D-454. The checks (GET /plan-revisions/{id}/checks) judge a deprecated SKU against the revision in effect today, which they derive from the revisions that they read already (D-408, D-447).
- **No write, and no statement more for a list.** A read never writes. The plan list derives in memory over the revisions that it reads already, so it keeps its two statements; the QueryRecorder pins this with due revisions among the plans. The revision read, the item read and /resolve read their plan's revisions once more, and the impact reads the revisions of its plans in one statement.
- **A write answers what it wrote.** The answers of the writes (the create, copy, clone, PATCH, submit and unschedule answers) render the stored state, which is the state the write wrote. No draft or pending revision stands beside a due one (D-451), so the two agree.
- **The counts.** The counts read the stored state and filter state <> 'superseded': stats.plans and BOOK_IN_PLAN (D-441, D-444), the entry usage and the SKU usage's plans (D-428), the in-plan SKUs (products P-D-212) and the plans GET /plans?sku_id= keeps (D-434). They count a scheduled revision as they count a published one. They also count a due revision's stored-published predecessor until the switch is persisted. The job ends that over-count: it persists at most the ticker's limit (100) due revisions a cycle (a minute), oldest available_from first, so a backlog of more than 100 drains 100 a cycle, and a plan whose switch keeps failing over-counts until it is repaired (D-446, D-450). A test reads a due revision before the persist: its predecessor's book still counts the plan, and after the job's tick only history names that book. plans_superseded_only stays "only superseded".
- **Where the texts changed.** products-sdk SkuUsage.plans and SkuUsageSets.in_plan, products P-D-212, D-408, D-413, D-419, D-428, D-434, D-441, D-444, the published_rev field of PricingPlanDto, and the served texts of the entry usage, the book stats, the two entry reads, the book list and the book delete.

D-460 amends this entry: the plan list derives each plan's current revision and the one in effect in memory as well, and reads the current revisions' items in one statement more, so it makes three statements; the QueryRecorder pins them with due revisions among the plans.

D-461 amends this entry: the plan list also reads the units its revisions name, one statement more: four. A write answers what it wrote with the unit it holds: the submit receipt's revision carries its unit's instants.

**Source:** Owner, 2026-09-28; phase 8 plan rev 2 (decisions 3, 10; plan review M2, L5); phase 8 review (behaviour B2, B3, B4; docs M1, L2). Amended by D-460, D-461.

#### D-454 [M] Resolve serves a scheduled revision from its sale date

**Status:** DECIDED 2026-09-29.

- **The state.** GET /resolve judges the revision by the state that it reads today (D-447), not by its stored state.
- **Due.** Once due, stored or derived, a revision resolves like any published revision, on every date (D-419), and its predecessor like any superseded one. So an answer does not change when the job persists the switch; a test asks the same questions before and after the persist (plan rev 2 M3).
- **Waiting.** A revision that still waits for its date resolves on a date on or after its available_from, with the resolved state scheduled: PricingResolvedRevisionState gains scheduled, after published and superseded. A date before its available_from is 409 REVISION_NOT_YET_AVAILABLE. A scheduled revision without a date is never due (D-447) and never available.
- **Draft and pending.** They stay 409 REVISION_NOT_PUBLISHED.
- *Rejected alternative:* fence every revision by its available_from. It would change D-419 for revisions that were published at once.

**Source:** Owner, 2026-09-28; phase 8 plan rev 2 (decision 8; plan review M3).

#### D-455 [M] The outbox wakes its sequencer after the commit

**Status:** DECIDED 2026-09-29.

- **The defect.** Since the main sync, toolkit-db's Outbox::enqueue does not mark its partition dirty. It returns a Wake, which marks the partition and wakes the sequencers when it is fired, after the commit (toolkit-db 2bfc76aec). events::enqueue fired that Wake at once, inside the caller's transaction. A sequencer woken then read the partition before the commit, found nothing and cleared the flag. The committed row then waited for the cold reconciler, a minute at the default profile.
- **The handle.** events::TxOutbox is the event sink as one transaction sees it. events::enqueue takes it in place of the EventSink. It adds each event's Wake to the handle and fires nothing. The clones of a handle share it.
- **The transaction.** events::transaction runs the work in the retrying transaction, with a new handle over the gear's sink. A retried attempt first discards the wakes of the attempt before it, which rolled back. When the transaction commits, the handle fires once. When it fails, the handle is discarded. The doors call it through support::transaction_with_events, support::unit_transaction_with_events and support::unit_transaction_door_with_events. These keep the isolation, the retries and the codes of support::transaction and the unit transaction: serializable, CONTENDED or UNIT_CONTENDED.
- **The writers.** Every writer that enqueues runs in it. These are the submit doors of a price, of publish-changes and of a plan revision, and the vote door (PricesPublished, PlanRevisionPublished, ApprovalUnitDecided). They are also the copy, clone and unschedule doors and the switch job, which catch a due switch up (D-450, D-451), and the commit of the reference work, which marks a reference lost (PriceBookEntryReferenceLost, PlanReferenceLost). No approval subject of pricing enqueues in its apply: the doors enqueue after the engine returns, in the same transaction.
- **The approval engine.** bss-approval changes no signature. An effect that may act only after the commit never passes through the engine: the gear keeps it, and the ApprovalSubject doc says so.
- **The census.** A test pins that TxOutbox::new occurs in src only in events::transaction, and that no other file fires or discards a wake. The census of the unit doors also requires their _with_events transaction.
- **The tests.** tests/broker_producer.rs drives the real in-process broker over a database with four connections. With one connection the sequencer queues behind the transaction, and the race does not show. A committed enqueue is delivered at once, although its transaction goes on for 300 ms after it. A rolled-back one wakes nothing: a row committed before it with its wake discarded stays undelivered in the same partition. A retried attempt's wake is dropped. The two events of a quorum-zero price submit are delivered at once. A probe that fires the wake inside the transaction again turns the first test red.
- **The interim envelope** (whole-branch review PS-21). Without a broker, events::enqueue writes the SDK's producer-outbox envelope by hand (events::interim_envelope), with producer_mode stateless, on the queue a bound producer later reads (bss_pricing_events). Stateless is right although the bound producer registers as monotonic. The interim arm has no producer registration, so it knows no producer_id, and the SDK's processor refuses a monotonic or chained envelope without one (ProducerOutboxEnvelope::to_event). A held monotonic row would never drain. A stateless row drains once a broker is bound, published without the producer's sequence, so the broker does not deduplicate it by sequence; every row enqueued after the bind carries it. A test deserializes the interim envelope as event_broker_sdk::producer::ProducerOutboxEnvelope and serializes it back unchanged, so a change of the SDK's envelope fails that test.
- *Rejected alternative:* each enqueue returns its Wake, and every function on the path returns it to the transaction (the toolkit's outbox::in_transaction and main's gears). The engine's apply returns nothing, so the products subjects would need a second mechanism. An error after an enqueue would also drop an unfired Wake, which the toolkit logs as a leak. The handle gives both gears one design (products P-D-221).

**Source:** Main sync of 2026-09-29 (sync report, port 3; toolkit-db 2bfc76aec); phase 8 plan rev 2 (run 8.2b).

#### D-456 [M] A plan names only a book its author may read

**Status:** DECIDED 2026-09-29.

- **The defect.** POST /plans and a PATCH /plan-revisions/{id} with a book_id checked the book with a tenant scope only, and the only PDP decision of the request was plan author. POST /plans/{id}/clone named the source's book the same way. So a caller could attach any book of its tenant, one its price_book grants exclude included. Once that revision was published, GET /resolve served the book's prices under plan read, while GET /price-book-entries/{id}/prices refuses the same money without price_book read (D-440).
- **The rule.** These three doors judge price_book read a second time, as D-440 judges the money, on the book the plan names: the body's book_id for the create and the PATCH, the source's published revision's book for the clone. The grant must admit that book. A denial, or a grant whose scope does not admit the book, is 403 PRICE_BOOK_READ_REQUIRED, and nothing is written. A policy that cannot judge is 503.
- **The order.** plan author first (403), then the money's policy (503), then the body (400), the book the tenant does not hold (404), then 403 PRICE_BOOK_READ_REQUIRED. A PATCH judges the money only when it names a book, so it reads its body first: one without book_id asks nothing of the money's policy. The clone judges the book it will copy, after a due scheduled revision is switched (D-451), and the refusal rolls the switch back with the rest.
- **What does not change.** The copy (POST /plans/{id}/revisions) keeps its plan's own book and judges nothing more. Resolve still reads under plan read (D-424); from now on a revision names only a book its author could read when the book was named.
- **The tests.** tests/book_reads.rs judges the three doors under a grant narrowed to another book (403) and under a policy that cannot judge (503), with nothing written, and then under a grant that admits the book. A caller holding plan author alone is now refused POST /plans and the clone (tests/plan_doors.rs, tests/plan_clone.rs).
- **Breaking for a caller** that holds plan author without price_book read on the book: POST /plans, the clone and a book-naming PATCH now answer 403. The vhp-core e2e plan authors hold every pricing action (CATALOG_AUTHOR), and its narrower actors are refused plan author first, so its grants suffice; a new scenario would give one actor every grant but price_book read and expect the three 403s.

D-463 extends this entry: a malformed available_from on POST /plans or the clone is 400 DATE_INVALID, among the body's refusals: after the money's policy (503) and before the 404 of the book or of the source plan.

D-468 extends this entry: a new plan's code off the rule is 400 PLAN_CODE_INVALID, among the body's refusals, after PLAN_CODE_REQUIRED and before DATE_INVALID.

**Source:** Whole-branch review of 2026-09-29, PS-08 (fix run W1a; the orchestrator's scope decision). Extends D-440. Extended by D-463, D-468.

#### D-457 [M] Every text a request writes has an explicit length cap

**Status:** DECIDED 2026-09-29.

- **The defect.** A book's and a plan's code and name had only a blank check, and a price's and a vote's note, the settings' GL code, tax category and invoice line templates, an entry's invoice line override and the dimension keys and values had none at all. The request body limit was their only bound, while one list page repeats a book's code and name up to 500 times. Only a book's description had a cap (D-444).
- **The caps**, counted in characters (Unicode scalar values), as a description always was: a code 64 (a book's, a plan's, a new dimension key, a new dimension value); a name 200 (a book's, a plan's); a note or a description 2000 (a price's note, a vote's note, a book's description); a GL code and a tax category 64 (the settings' default_gl and default_tax_category); an invoice line template 2000 (an entry's invoice_line_override, each template of the settings). The values live in domain::caps; the note cap is the approval engine's NOTE_MAX_CHARS, and a const assertion ties the two.
- **The refusal.** 400 FIELD_TOO_LONG with the field named (a field violation on code, name, invoice_line_override, default_gl, default_tax_category, invoice_line_templates, key, values or add), the shape NOTE_TOO_LONG already has. A note keeps NOTE_TOO_LONG on note, and a description BOOK_DESCRIPTION_TOO_LONG (D-444). Each door judges its body right after it parses it, before it reads or writes anything, so a too long text is refused before a 404 or a 409. The two full-replace PUTs are the exception: the PUT of the dimension registry and the settings PUT judge their texts against the stored row, after their If-Match (below). The vote note is judged by the approval engine (bss-approval, NoteTooLong) before its first write, on approve and on reject.
- **Stored rows.** Only a write is judged, only on the fields its body carries, and only writes of new text are judged. A stored row over a cap stays readable, and a PATCH that does not carry the field leaves it as it is. A text that must name a stored row is never capped: a price's dim_value (400 DIM_VALUE_UNKNOWN otherwise), an entry's dimension_key (400 DIM_NOT_DECLARED otherwise), and the registry PATCH's key and the values it removes. The PUT of the registry caps only a key the stored registry does not hold, and only the values their key does not hold. So a key or a value stored before the caps and longer than 64 characters never locks the registry: every PUT must carry a key an entry names (else 409 DIMENSION_KEY_IN_USE) and a value a price uses (else 409 DIM_VALUE_IN_USE), and it passes; a PATCH names the key and removes a value nothing uses (the second review of W1a, L1; tests/dimension_values.rs). The settings PUT replaces the whole row, so every write sends the stored texts back: it caps only a default_gl or a default_tax_category other than the stored one, and only a template of invoice_line_templates other than the one stored for its SKU type. So settings stored before the caps with a longer GL code, tax category or template never lock the settings: a write that changes only the rounding passes, and only a changed text is refused (the second review of W1b, L2; tests/settings_doors.rs).
- **The tests.** tests/book_writes.rs sends one text over each cap, door by door, and each is 400 with its code and field, with nothing written; the caps themselves pass in two-byte characters. tests/approval_doors.rs does the same for the vote note.
- **Products** applies the same caps in its own decision, P-D-225 (fix run W1b). The vhp-core e2e would add one refusal: a book created with a 65-character code is 400 FIELD_TOO_LONG on code.

D-468 extends this entry: a new plan's code also follows a rule of at most 32 characters; the 64-character cap is judged first.

**Source:** Whole-branch review of 2026-09-29, PS-09, PS-10 and X-01 (fix run W1a; the dispositions' "Length caps"); the second reviews of W1a (L1) and W1b (L2). Extended by D-468.

#### D-458 [M] The approval-unit list pages and reads its page set-based

**Status:** DECIDED 2026-09-29.

- **The defect.** GET /bss-pricing/v1/approval-units answered every unit of the tenant that its filters kept, and read each unit's items, its decisions and its impact's plans with statements of their own: a tenant with many units got an unbounded answer in a number of statements that grew with it.
- **The page.** The list takes limit (200 by default, clamped at 500, the book list's rule, D-442) and cursor, the opaque continuation of a page's page_info.next_cursor, the toolkit pager's cursor as the book list's. The order stays submission order (submitted_at), with the unit id breaking a tie. The answer is { items, page_info }: items keeps its shape, and page_info (next_cursor, prev_cursor, limit) is added. The cursor carries a hash of the narrowing (state, kind and the referenced aggregate, ref_id or book_id), so a cursor replayed under another narrowing is 400 FILTER_MISMATCH. A cursor that does not read is 400, and a limit that is not a number 400 QUERY_INVALID.
- **The reads.** A page reads its units, then all their items, all their decisions and the plans their impact names, each in one statement (infra::prices::PlansReading, the four statements of plans_reading over the page's entries). The QueryRecorder shows the same statements for 10 and for 100 units, each with an item, a vote and a plan naming its entry.
- **Breaking for a caller** that reads the list whole: a tenant with more than 200 units matching its filters gets them over several pages, and must follow next_cursor. The Studio (the pricing-mfe) has to follow it. The gears-rust tests that read the list whole follow it (entry_support::Fixture::all_units); the gears-rust e2e reads no list. The vhp-core e2e reads it whole in tests/bss-pricing/test_pricing_isolation.py (two reads) and test_pricing_prices.py (the pending queue), each with far fewer than 200 units, so they pass unchanged; a vhp-core change would only make them follow next_cursor.
- **Products** gets the same list in its own decision, P-D-224 (fix run W1b).

**Source:** Owner, 2026-09-29 (the dispositions' O2, answered "2"); whole-branch review PS-13 (fix run W1a, a scope addition).

#### D-459 [M] One approve-eligibility predicate for the engine and its readers

**Status:** DECIDED 2026-09-30.

- **The risk.** A read that shows whether its caller may approve a unit, or how many votes the unit has, would copy the rules of the approval library's private rules module: the terminal state, the separation of duties, the duplicate vote and which votes count. A copy drifts: the read says the caller may approve, and the vote door answers 403 SOD_VIOLATION.
- **The rule.** bss_approval exports one function, approve_eligibility(unit, items, decisions, actor), and its result ApproveEligibility { approvals, refusal }. approvals counts the approve votes of the unit's current generation that are not stale; a reject, a stale vote and a vote of an earlier generation do not count. refusal is none when the actor may approve, else the engine's refusal, in the engine's order: UNIT_ALREADY_DECIDED for a terminal unit, SOD_VIOLATION for its submitter or an item author, DUPLICATE_VOTE for an actor who voted in the current generation. The items are the unit's stored items, which are the current generation's: a stale refresh rewrites them. The rules module stays private.
- **The engine uses it.** evaluate_approve, which Engine::approve calls once it has loaded the unit at the reviewer's generation, judges through it: its refusal is the vote's error, and an eligible vote pends at approvals + 1 or applies. So the engine answers as before: the library's rule tests keep their expectations, and both gears' door tests pass unchanged.
- **The readers.** A pending plan revision's progress reads approvals through it (D-462). Products and pricing compute caller_can_approve through it in run 9.3.
- **The tests.** A table over quorum 0, 1 and 2, a stale vote, a vote of an earlier generation and a decided unit, for the submitter, an item author, a voter of each generation and a fresh reviewer: the predicate's refusal is evaluate_approve's error, and a counted vote's have is approvals + 1. An engine test drives Engine::approve through a quorum-2 unit: a refused submitter, a first vote, a duplicate, a content drift and its refresh, the first reviewer's vote on the new generation, the apply and a vote after it. Before each vote the predicate over the stored rows answers what the vote meets. A probe that counted stale votes turned three tests red.

D-393's engine keeps its rules; this entry makes them one function that the readers call.

**Source:** Phase 9 plan rev 2 (W2, binding: the review's alternative W2; L5 for #40's counts).

#### D-460 [M] The plans list names each plan's current revision and the one in effect

**Status:** DECIDED 2026-09-30.

The plans screen shows, per plan, the revision being changed or waiting and the one it sells today (ask 27). The list carried only the revision headers, so the screen read every revision to count its items.

- **The rule.** GET /plans and GET /plans/{id}, and every answer of PricingPlanDto (the create, the clone and the rename), carry two objects:
  - current, PricingPlanCurrent { revision_id, rev_no, state, item_count, sku_ids, created_by }: the draft or pending revision (a plan holds at most one, D-451), else the scheduled one still waiting for its date, else the published one in effect. It is chosen over the states the revisions read today (D-447; domain::plan::current), so a due scheduled revision whose switch is not persisted is current, and reads published. item_count counts every item, an included item without an entry too; sku_ids names each item's SKU, in ascending order; created_by is the revision's author, who edits it while it is a draft (D-404), not the plan's.
  - in_effect, PricingPlanInEffect { revision_id, rev_no }: the published revision in effect today (domain::plan::in_effect), the one the plan sells.
  - current is null for a plan without revisions; in_effect is null before the first publication.
- **Not the SKU filter.** sku_ids names every item of the current revision; GET /plans?sku_id= keeps a plan for an item with an entry, judged on the stored state of its draft, pending, scheduled or published revisions (D-434). The two may differ, and the served text says so.
- **The reads.** The items come from one grouped read over the listed plans' current revisions (plan_item_repo::skus_of_revisions: revision and SKU only). GET /plans makes three statements whatever the number of plans: the plans, their revisions and the current revisions' items; one when the tenant has no plan. The grouped read runs for an empty list too, so the count does not depend on the rows. The QueryRecorder shows the same statements for 10 and for 100 plans.
- **The write answers.** A write answers what it wrote (D-453): the create answers its empty draft (item_count 0), the clone its new draft with the items it copied, and the rename the plan as its read shows it.
- **No ready flag.** The list carries no ready flag and no count of red checks: the checks read every item SKU from Products (D-408), so the list would make a Products read per SKU of every draft. The screen reads GET /plan-revisions/{id}/checks per draft.
- **The tests.** tests/plan_overview.rs: the current revision and the one in effect for a draft only, a pending only, a published only, a draft, a pending and a waiting scheduled revision beside the published one, a due scheduled revision whose switch is not persisted (both name it, and its stored state stays scheduled), and a plan without revisions; the plan read agrees with its list row; sku_ids against the SKU filter; the create, clone and rename answers; the statements for 10 and 100 plans. domain::plan's tests pin the choice over the effective states. Probes that chose the published revision before the scheduled one, and that read the items per plan, were caught.

**Source:** Owner, 2026-09-30 (validation 3 item 1); phase 9 plan rev 2 (decision 1; plan review M6, M7, W3, L4). Amends D-434 and D-453.

#### D-461 [M] A revision says who made it and when it was submitted and approved

**Status:** DECIDED 2026-09-30.

The plans screen shows who made each revision and when it was submitted and approved (ask 33). A header carried its state and published_at only, and a scheduled revision has a null published_at until its date.

- **The fields.** PricingPlanRevisionHeader gains created_by and created_at, the row's; the revision read already carries them. Both the header and PricingPlanRevisionDto gain submitted_at and approved_at:
  - submitted_at is the submission instant of the unit the revision names: its pending unit (pending_unit_id), or the unit that approved it (approved_by_unit_id).
  - approved_at is the approving unit's decided_at. A scheduled revision needs it, because its published_at is null until its date.
  - A draft names no unit, so both are null: also a draft back from a reject or a withdraw (its lock is cleared) or from an unschedule (approved_by_unit_id is cleared, D-452). A superseded revision keeps its own unit's instants. A unit that does not read leaves both null.
- **The reads.** The plan read and list read the units that their revisions name in ONE grouped statement (approval_repo::unit_instants: id, submitted_at and decided_at only), so GET /plans makes four statements whatever the number of plans (D-434, D-453, D-460). The revision read reads the one unit its revision names. The instants are read under plan read: two instants, no actor and no content (the owner's O-9a covers them with D-462).
- **The write answers.** A write answers what it wrote (D-453). The submit receipt's revision carries its new unit's submitted_at, and approved_at when quorum 0 applied it at once; the copy, the revision PATCH and the unschedule answer a draft, with neither.
- **The tests.** tests/plan_overview.rs: the header of a pending, a scheduled, a published and a superseded revision against its unit's own read, and of a draft back from a reject, a withdraw and an unschedule; the revision read agrees; created_by and created_at are the row's; the write answers; the list's four statements for 10 and 100 plans. Probes that took submitted_at from the approving unit only, and that dropped the instants from the receipt, were caught.

**Source:** Owner, 2026-09-30 (validation 3 item 7); phase 9 plan rev 2 (decision 2; plan review M6, M7, L1, L9). Amends D-434 and D-453.

#### D-462 [M] A pending revision shows its vote progress under plan read

**Status:** DECIDED 2026-09-30.

The plan's screen shows how far a pending revision is from its quorum (ask 40).

- **The grant (O-9a).** The owner ruled on 2026-09-30 that a plan reader may see the vote progress of a pending revision, counts only, without the approval-unit read grant. *Counter-argument:* it tells a reader that a change is in review and how close it is, which before needed the unit grant.
- **The field.** PricingPlanRevisionDto gains approval, PricingPlanApprovalProgress { unit_id, approvals, quorum_required }, while the revision reads pending, and null in every other state: on GET /plan-revisions/{id} and on every answer of the DTO. A submit receipt of a pending unit carries approvals 0; one applied at once carries null.
- **The count.** approvals is D-459's count through bss_approval::approve_eligibility over the unit, its stored items and its decisions: the approve votes of the current generation that are not stale, the votes the vote door counts. A duplicate vote adds nothing, and a stale refresh turns earlier votes stale, so they stop counting. quorum_required is the unit's own. No actor id, note or snapshot is shown; the unit's reads keep their grant.
- **The reads.** The revision read reads its unit, and for a pending one its items and its decisions: at most three statements more, only for a revision that names a unit.
- **The tests.** tests/plan_overview.rs: quorum 2 through its votes (approvals moves as the receipt's have does, a duplicate vote adds nothing, a refresh makes the first vote stale and it stops counting, the apply clears the field), quorum 1 and quorum 0 at the submit, a draft; a caller holding plan read only reads it. Probes that counted stale votes, that dropped the progress from the read and from the receipt, were caught.

**Source:** Owner, 2026-09-30 (O-9a, "yes"); phase 9 plan rev 2 (decision 4; plan review M7, L5).

#### D-463 [M] A plan's sale date on create and clone

**Status:** DECIDED 2026-09-30.

A new plan's first revision took its sale date only through a second call, the revision PATCH (ask 34). The date decides at approval whether a revision is scheduled or published (D-449).

- **The create.** POST /plans takes an optional available_from, YYYY-MM-DD: rev 1's sale date. Omitted or null means "at publish", as before.
- **The clone.** POST /plans/{id}/clone takes an optional available_from. Omitted, rev 1 keeps the source's sale date, as before (D-451). A date overrides it. Null clears it: "at publish". The clone request tells the three apart, as the revision PATCH does.
- **The refusal.** The date is judged as the revision PATCH judges it: a date that does not read is 400 DATE_INVALID on available_from, and nothing is written. It is one of D-456's body refusals: after the money's policy (503), with PLAN_CODE_REQUIRED, before the 404 of the book (the create) or of the source plan (the clone). Like the PATCH, the door refuses only a date that does not read; the checks judge the sale date (D-408).
- **The texts.** The served texts of the create and the clone name the field and DATE_INVALID.
- **The tests.** tests/plan_doors.rs: a create with a date, without one, with null, and with a malformed date on a known and on an unknown book (400 both times, nothing written). tests/plan_clone.rs: a clone that keeps, overrides and clears the source's date, and a malformed date on a known and an unknown source plan; the source is unchanged. Probes that dropped the create's date and ignored the clone's override were caught.

**Source:** Owner, 2026-09-30 (validation 3 item 7); phase 9 plan rev 2 (decision 3; plan review L10). Amends D-451; extends D-456.

#### D-464 [L] A plan submit and a publish-changes carry the submitter's note

**Status:** DECIDED 2026-09-30.

The approvals screen shows why a unit was submitted (ask 44). Products' submit doors take a note (P-D-219); pricing's took none (D-445), so a plan change or a batch of prices reached its approver without one.

- **The plan submit.** POST /plan-revisions/{id}/submit takes an optional body { note } (PricingPlanRevisionSubmitRequest; the served body is optional), products P-D-219's body rule: no body, {} and note: null carry no note, and any other key is 400 BODY_UNEXPECTED, the rule of the empty body it replaces. A note that is neither text nor null is 400, as a body that does not read. The Idempotency-Key's digest covers the body as sent ({} for none).
- **Publish-changes.** POST /price-books/{id}/publish-changes takes an optional note beside price_ids and common_effective_date; omitted or null carries none. Its body rules are otherwise unchanged: it still needs a JSON object, and a key it does not know is 400 as before.
- **The cap.** Both doors judge the note right after they parse the body, before they read anything: at most 2000 characters, counted as Unicode scalar values (bss_approval::NOTE_MAX_CHARS, D-457), else 400 NOTE_TOO_LONG on note, and nothing is written. So a revision or a book the tenant does not hold answers the 400 too. The engine's submit caps no note, so the door is its only judge.
- **Where it goes.** The note passes through the library's SubmitRequest.note onto the unit's submit_note: the submit receipt and every unit read carry it. It is not content (D-445): neither the snapshot nor the fingerprint carries it.
- **The single price.** POST /prices/{id}/submit takes no note: its body stays empty, and any key is 400 BODY_UNEXPECTED. A note for prices travels with publish-changes. Its served text says so.
- **The tests.** tests/plan_revision_approvals.rs: a note stored and read back; no body, {} and a null note; 2000 two-byte characters pass; 2001 are 400 NOTE_TOO_LONG on a revision and on an unknown one, with nothing written; a stray key alone and beside a note is 400 BODY_UNEXPECTED. The pin that the plan submit takes no body now pins that it takes nothing but a note. tests/approval_doors.rs: publish-changes stores a note and a null one, refuses one over the cap before the 404 of an unknown book, and the price submit refuses a note. Probes that dropped either door's cap were caught.

**Source:** Owner, 2026-09-30 (validation 3 item 6); phase 9 plan rev 2 (decision 5; plan review L3). Amends D-445 and products P-D-219's Pricing bullet.

#### D-465 [M] A revision may carry again a deprecated SKU its plan sells

**Status:** DECIDED 2026-09-30.

The checks accept a deprecated SKU that the plan's published revision in effect carries (D-408), but the item door refused every deprecated SKU. So an item removed from a copied draft could not be added back, although the copy had carried it (ask 36).

- **The ruling (O-9b).** The owner ruled on 2026-09-30 that D-408's "newly added" does not cover a re-add. *Counter-argument:* make the check strict instead, so a deprecated SKU never re-enters a draft; not chosen.
- **The rule.** POST /plan-revisions/{id}/items admits a deprecated SKU when the plan's published revision in effect today (D-447) carries it. Any other deprecated SKU stays 400 ITEM_SKU_DEPRECATED on sku_id. A clone is a new plan with no revision in effect, so its drafts add no deprecated SKU; the SKUs it carried stay red in its checks (D-408).
- **One source.** plans::published_skus gives the item SKUs of the published revision in effect, from the same function the checks read them with (stored_context). The door judges it in its admissibility read, before any claim or reservation. The create op's SKU re-read (reference_work::observe_sku) judges it on its reserving step, reading the op's revision and its plan on the clock's day, so the create answers what the door answers. A revision that stops carrying the SKU between the two is judged again by the checks at submit and at apply.
- **Unchanged.** An attach and a rereserve admit a deprecated SKU as before (D-413). A draft, retiring or retired SKU and a bundle SKU stay refused.
- **The texts.** The served text of the item create names ITEM_SKU_DEPRECATED and the rule.
- **The tests.** tests/plan_item_doors.rs: a SKU deprecated after its plan's publication, removed from the copy and added back (201, confirmed, its check green); another deprecated SKU refused; the carried SKU refused in a clone. tests/plan_item_references.rs: the create op, below the door, writes a carried deprecated SKU and refuses another with the door's answer, its op keeping SKU_DEPRECATED. A probe of the door alone turned the door test red and left the op's green; a probe of the op turned both red.

**Source:** Owner, 2026-09-30 (O-9b, "yes"); phase 9 plan rev 2 (decision 6; plan review L2). Amends D-408.

#### D-466 [M] Each check row names its items and its blocking prices

**Status:** DECIDED 2026-09-30.

The plan's items table marks the items that a failing check is about (ask 29). A check row named its items only in the prose of detail, and blocked_by named approval units only, so the screen kept a copy of the rules.

- **The fields.** Each row of GET /plan-revisions/{id}/checks, and each row of the REVISION_CHECKS_RED body (whose detail is the red rows as the checks door renders them), gains two fields. code, ok, label, detail, info and blocked_by keep their names and their meaning.
  - subjects, [PricingPlanCheckSubject { item_id, sku_id, price_book_entry_id }]: the items that turn the row red, in the revision's item order. price_book_entry_id is null for an item that names no entry. A green row names no item. A plan-wide row names none either: PLAN_NAME, PLAN_BOOK, PLAN_BOOK_VALIDITY, PLAN_ITEMS and the information rows DESCRIPTORS and APPROVAL.
  - blocked_by_prices, [PricingPlanCheckBlockingPrice { unit_id, price_id, price_book_entry_id }]: the pending prices behind blocked_by, one per price, ordered by unit and then by price. blocked_by is built as before: the units that hold a pending price of an entry that an uncovered item names, on any chain, because the default chain can cover a value. The units of blocked_by_prices are exactly blocked_by.
- **Which items.** An item-level row names the items that its detail lists. ITEM_ENTRY_MISSING, ITEM_ENTRY_SKU_MISMATCH, ITEM_ENTRY_LOST, ITEM_BUNDLE_SKU, CHARGE_KIND_SKU_TYPE, ITEM_BOOK_FOREIGN, ITEM_UNCOVERED, ITEM_SKU_DEPRECATED, ITEM_SKU_UNAVAILABLE, ITEM_REFERENCE_PENDING and ITEM_REFERENCE_LOST name each item they refuse. METER_DUPLICATE names both items of each pair that meters one usage type. FREQUENCY_MIXED names every recurring item priced in the plan's book, because each bills in one of the mixed periods. Only ITEM_UNCOVERED has a blocked_by, so only it has blocked_by_prices.
- **The reads.** No read is added: the checks' context already holds each item, the entry it names and that entry's pending prices with their units.
- **The tests.** src/domain/plan_tests.rs: a table over every code that the checks emit, enumerated from what the checks answer over the table's contexts, each context turning its code red (the information rows stay green) and naming its items; in every context a green row names no item and no price, and each row's blocked_by is exactly the units of its blocked_by_prices; an uncovered entry's pending prices, one row per price. tests/plan_item_doors.rs: ITEM_UNCOVERED names its item and the pending price of a submitted prices unit, and names neither once the unit is approved. tests/plan_revision_approvals.rs: the REVISION_CHECKS_RED body carries both fields and equals the checks door's red rows. Probes that dropped the blocking prices, the mixed periods' items and the DTO's subjects were caught.

**Source:** Owner, 2026-09-30 (validation 3 item 5); phase 9 plan rev 2 (decision 7). Extends D-408.

#### D-467 [H] A plan item is a SKU and its entry: no treatment, no included quantity, no minimum quantity

**Status:** DECIDED 2026-09-30.

The owner asked for how long an included quantity is included, and learned that it had no defined period, no proration and no rollover rule. The owner then removed the field for now, and with it the treatment and the minimum quantity. A plan item is now a SKU and its entry in the plan's book.

- **The API.** POST /plan-revisions/{id}/items takes sku_id and price_book_entry_id, and both are required: a missing or null entry is 400 ITEM_ENTRY_MISSING on price_book_entry_id. PATCH /plan-items/{id} takes only price_book_entry_id; a null one is 400 ITEM_ENTRY_MISSING, and so is a PATCH that leaves an item without an entry. At both doors a body that carries treatment, included_qty or qty_min is 400 BODY_UNEXPECTED on that key, the rule for a stray key, judged with the body, before any read; any other key the body does not know is refused by its parse, as before. TREATMENT_INVALID, INCLUDED_QTY_INVALID and QTY_MIN_INVALID are gone.
- **The reads.** No read shows the three fields: the item read and every item answer (PricingPlanItemDto), the revision read, GET /resolve (PricingResolveItemDto) and the snapshots of the units submitted from now on. The treatment's closed set (PricingTreatment) is gone from the served spec.
- **Storage.** There is no migration: the columns and their CHECKs stay. Every row written from now on stores treatment = 'paid', included_qty = NULL and qty_min = NULL (plan::stored_treatment): the item create op, the copy and the clone, and the item PATCH, which rewrites the row it changes in that shape. The one exception is the copy of a legacy item stored without an entry: its entry stays null, so the column's CHECK keeps its treatment 'included', with no quantity. A create op stored before D-467 carries the old fields in its input; they are ignored.
- **Legacy rows.** Rows stored before D-467 keep reading, with the fields hidden. A legacy included item answers price_book_entry_id: null, and resolves with no chains. Published history is not rewritten: a published or superseded revision keeps its rows as stored.
- **The checks.** INCLUDED_QTY and every branch keyed on a treatment or a quantity go. FREQUENCY_MIXED, METER_DUPLICATE and ITEM_UNCOVERED stay. An item without an entry is ITEM_ENTRY_MISSING, whatever it was stored as, and meters nothing; so a legacy included item in a draft is ITEM_ENTRY_MISSING, and its author removes it or gives it an entry. The row's label is "Every item points at a price".
- **The units pending at the deploy.** The fingerprinted content of a plan revision is its book, its sale date and its items by SKU, each item its SKU and its entry; before D-467 it also held each item's treatment and quantities. Pricing re-derives a unit's content on every vote, so every plan_revision unit pending at the deploy finds its content changed once. Measured in tests/plan_items_legacy.rs:
  - the first approve or reject refreshes the unit at the next generation, records no vote, and answers 400 UNIT_STALE with the new generation; the votes of the older generation stop counting;
  - a unit whose revision holds only priced items (the stand's qty_min item) then applies on the approve of the new generation;
  - a unit whose revision holds a legacy included item is refused at the apply of the new generation, 409 APPLY_REFUSED with REVISION_CHECKS_RED: ITEM_ENTRY_MISSING, and nothing is published; the reject of the new generation returns the revision to a draft, whose author fixes it. No unit is left that can be neither approved nor rejected.
- **The frozen read contract (D-419 to D-422).** treatment, included_qty and qty_min leave each resolved item; the consumer goldens under tests/contract/ were recorded again, and they differ only by these keys. The pinned price read (D-422) is unchanged. No code outside pricing reads the three fields; the ledger's treatment is another concept, and pricing-sdk has none.
- **Cross-gear effects.**
  - No optional (add-on) and no included items remain in pricing plans.
  - Main's orders PRDs assumed optional items: the orders-lifecycle PRD's add-on selection on the order line (gears/bss/orders-lifecycle/docs/PRD.md, the submit gate's add-on bounds and the resolved open question on add-on selection) and the orders-changes PRD's "Add-On Selection on the Order Line". They are named here and not edited.
  - Rating's T-D-38 floor is applied "after included quantities", and that step has no operand now; Rating's register records the amendment (gears/bss/rating/docs/DECISIONS.md).
  - The vhp-core e2e sends and asserts treatment; it follows in run 9.5.
- **The tests.** src/domain/plan_tests.rs: the checks without a treatment, an entry-less item ITEM_ENTRY_MISSING and metering nothing, the check list without INCLUDED_QTY. tests/plan_item_doors.rs: each key refused at both doors with nothing reserved or written, the entry required, a null entry refused, a new row stored paid with no quantity. tests/plan_item_references.rs: the create op writes paid with no quantity. tests/plan_items_legacy.rs seeds the stand's shapes (an included item in a draft, in a pending revision and in a published one; a qty_min item pending and published; two pending units whose content carries the old fields) and reads, resolves, copies, checks, approves and rejects them as above. tests/response_enums.rs: no plan item schema carries the keys, and the create requires its entry. Probes that let a key through, copied the source's treatment and quantities, fingerprinted the treatment again, spared an entry-less item from ITEM_ENTRY_MISSING, kept the PATCHed row's shape and wrote a quantity from the create op were caught.

**Source:** Owner, 2026-09-30 (the phase 9 plan's section "plan items lose treatment, included_qty and qty_min"); phase 9 plan rev 2 (run 9.2). Amends D-388, D-394, D-407, D-413, D-419, D-420, D-421 and D-439.

#### D-468 [M] A new plan's code follows a declared rule

**Status:** DECIDED 2026-09-30.

A plan's code had a blank check and a length cap only (D-457), stored as sent, so `PRO`, `pro` and `PRO ` could coexist under the exact unique index (ask 39). The owner asked for a rule (the scope addition to run 9.2).

- **The rule.** On POST /plans and POST /plans/{id}/clone the new plan's code matches `^[A-Z0-9][A-Z0-9_-]{0,31}$`: 1 to 32 characters (domain::plan::CODE_MAX) of upper-case ASCII letters, digits, `-` and `_`, starting with a letter or a digit (domain::plan::code_follows_the_rule). Otherwise the answer is 400 PLAN_CODE_INVALID on field code, and nothing is written.
- **As sent.** The code is judged as sent, with no trim and no case folding: `pro`, `PRO ` and ` PRO` are all invalid, and a valid code is stored as sent.
- **The order.** The length cap comes first, with the body, before any read: a code of 65 characters or more is still 400 FIELD_TOO_LONG (D-457). Then a blank code is 400 PLAN_CODE_REQUIRED, as before. Then the rule: a code of 33 to 64 characters, or any other code off the rule, is 400 PLAN_CODE_INVALID. Then the rest, in D-456's order: DATE_INVALID, the book's or the source plan's 404, PRICE_BOOK_READ_REQUIRED, and 409 PLAN_CODE_TAKEN.
- **Stored codes are grandfathered.** There is no migration. A code stored before the rule keeps reading and is never judged again: the plan reads, lists, is renamed (the plan PATCH takes a name only and judges no code) and is cloned, and only the clone's new code is judged. The Benidorm stand holds 113 lower-case codes such as `plan-6dfd4733` from the e2e. Uniqueness stays exact, as before; because every new code is upper-case, no two plans created from now on can differ in case alone.
- **Out of scope.** The codes of books, SKUs and categories.
- **The texts.** The served texts of both doors name the rule and PLAN_CODE_INVALID in their refusal lists.
- **The e2e.** The gears-rust e2e creates upper-case plan and clone codes. The vhp-core e2e (tests/e2e/tests/lib/pricing.py create_plan, and its clone codes) creates lower-case codes; it is fixed in run 9.5 and must land with this image.
- **The tests.** tests/plan_codes.rs: valid codes (one character, a digit, `-` and `_`, 32 characters) stored as sent; each invalid shape refused with its code and nothing written (lower and mixed case, a trailing, a leading and an inner space, a leading `-` or `_`, a dot, a non-ASCII letter, 33 and 64 characters: PLAN_CODE_INVALID; empty and blank: PLAN_CODE_REQUIRED; 65 characters: FIELD_TOO_LONG); the order against the book's 404, a malformed date and an over-long name; the clone under the same rule, before the source's 404; a grandfathered lower-case plan that reads, is renamed and clones to a valid code, and a new upper-case code beside a stored lower-case one. The suites' plan fixtures now create upper-case codes. Probes that dropped the rule, judged a blank code by the rule, skipped the clone's code and admitted lower case were caught.

**Source:** Owner, 2026-09-30 (ask 39; backend asks validation 3, "confirm there is no charset rule"; phase 9 plan rev 2 L8 had moved it to Out as an open question). Extends D-456 (the order of the body's refusals) and D-457 (the code's cap).
