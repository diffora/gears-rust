//! Create `bss.pricing_price_window` — the time axis of a published price
//! (`design/07-pricewindow-linkage.md` §6, `cpt-cf-bss-pricing-state-price-window`),
//! the slice-owned store D-03 puts in this gear rather than in the effective-dating
//! UC it absorbed.
//!
//! One row is one half-open interval `[effective_from, effective_to)` on one
//! price row, and therefore on that row's canonical scope key. **Half-open is
//! the load-bearing part**: `effective_to = next.effective_from` is adjacency
//! and not a gap (§9 names that false positive as one the coverage checker must
//! not produce), and `chk_pricing_price_window_interval` is written to admit it.
//! `effective_to IS NULL` is open-ended — a window that never expires
//! (`inst-ws-expire`) — and not a missing value.
//!
//! The §4 state machine lives in the constraints and not only in the domain, for
//! `pricing_plan`'s reason: a rule that lives only in application code is one
//! ad-hoc `UPDATE` away from being bypassed, and what it would be bypassing here
//! is which price a subscriber was charged over an interval that has already
//! been billed. `scheduled -> active` (`inst-ws-activate`),
//! `scheduled -> cancelled` (`inst-ws-cancel`), `active -> expired`
//! (`inst-ws-expire`), and nothing else; `expired` and `cancelled` are immutable
//! history (`inst-ws-immutable`).
//!
//! # The audit timestamps: what is actually true, and why the naive `CHECK` is
//! not
//!
//! The three flip timestamps are constrained as **biconditionals against the
//! reachable state set**, which is narrower than "one column per state":
//!
//! * `(state IN ('active','expired')) = (activated_at IS NOT NULL)`. An
//!   `expired` window **was** `active`, so it carries `activated_at` too — the
//!   naive `(state = 'active') = (activated_at IS NOT NULL)` would make every
//!   expiry unstorable, which is the form an implementer writes first. The
//!   left-hand side names two states for that reason.
//! * `cancelled` is deliberately **not** in that set, and that is a fact about
//!   §4 rather than an omission: the only edge into `cancelled` leaves
//!   `scheduled` (`inst-ws-cancel` — "an active or historical window is never
//!   cancelled"), so a cancelled window never activated and an `activated_at` on
//!   one is a record of something that did not happen.
//! * `(state = 'expired') = (expired_at IS NOT NULL)` and
//!   `(state = 'cancelled') = (cancelled_at IS NOT NULL)` are the plain
//!   biconditionals, in `chk_pricing_approval_decided_at`'s idiom.
//!
//! Both directions are asserted on purpose. The one-way form
//! `state IN ('scheduled','cancelled') OR activated_at IS NOT NULL` refuses an
//! `active` row with no `activated_at` and **accepts** a `scheduled` row that
//! claims to have been activated, which is a lie the store would then hold about
//! when a price took effect. §6 states no co-nullability at all, so this is a
//! reading of §4's edges rather than a transcription, and it is reported as one.
//!
//! # §4's transition **conditions**, and the three of them a row-local CHECK
//! carries
//!
//! §4's edges are conditional and the biconditionals above say nothing about the
//! conditions: `inst-ws-activate` fires **WHEN `now ≥ effectiveFrom`**,
//! `inst-ws-expire` **WHEN `now ≥ effectiveTo`**, and — verbatim — *"an
//! open-ended window never expires"*. Three of those conditions are decidable
//! from one row's own immutable columns, so three CHECKs carry them:
//!
//! * `chk_pricing_price_window_activation_order` —
//!   `activated_at IS NULL OR activated_at >= effective_from`. The stamped
//!   instant of an activation cannot precede the start it was the arrival of. A
//!   row that claims otherwise is the "activation that never happened" the
//!   INSERT-guard note below names, and it was reachable through
//!   [`window_repo::transition`](crate::infra::storage::repo::window_repo::transition)
//!   until 2026-08-04.
//! * `chk_pricing_price_window_expiry_order` —
//!   `expired_at IS NULL OR expired_at >= effective_to`. The same rule on the
//!   other edge. It is deliberately **silent** when `effective_to` is NULL (the
//!   comparison is then NULL and a CHECK admits NULL), because that case has its
//!   own constraint below and one statement refused by two of them is a removal
//!   proof that proves neither.
//! * `chk_pricing_price_window_open_ended` —
//!   `NOT (state = 'expired' AND effective_to IS NULL)`. `inst-ws-expire`'s own
//!   sentence, made physical: there is no instant an open-ended window's expiry
//!   could be the arrival of.
//!
//! **This is not the whole of §4's conditions and the residue is named.** The
//! two clock-dependent halves — that `now` really has reached the boundary at the
//! moment of the flip — are not here, because a CHECK cannot read the clock
//! without becoming the one guard in the chain that answers differently depending
//! on when it is asked. What these three give is the durable half: whatever the
//! flip's own instant was, the row cannot record it as having happened before the
//! boundary it was triggered by. The transient half is
//! [`window_repo::transition`](crate::infra::storage::repo::window_repo::transition)'s,
//! and it carries the condition **into the `UPDATE`'s `WHERE`** rather than only
//! into a pre-check, so a sweep and a concurrent writer cannot step between the
//! read and the write.
//!
//! # `effective_from` is frozen **always**, which is stricter than
//! `inst-ws-immutable`
//!
//! The instruction freezes `effective_from` *once it has passed*. The whitelist
//! below freezes it from the moment the row exists, and that is a **deliberate
//! narrowing of the instruction's literal scope**, recorded here and reported
//! rather than left to read as the rule:
//!
//! * there is no sanctioned writer of a scheduled window's start — a re-schedule
//!   is a cancel plus a new window, because `WINDOW_START_IN_PAST`
//!   (`inst-ws-future-start`, D-63) bounds *creation* and would otherwise be
//!   evadable by creating a legal window and then moving its start backwards;
//! * a trigger that admitted a mutable future start would have to read the clock
//!   to know whether this particular start had passed, so the guard would be the
//!   one thing in the chain that answers differently depending on when it is
//!   asked.
//!
//! The direction is the safe one and the difference is a real one. Widening the
//! whitelist to admit a future start needs a decision, not a patch.
//!
//! # Non-overlap per canonical scope key is **not here**, and that is a choice
//!
//! §6 requires "non-overlap per canonical scope key enforced inside every
//! mutation". No **declarative** schema object can state it: the canonical scope
//! key is eight columns of `pricing_price` and this table carries `price_id` and
//! nothing else of it, so a `UNIQUE` index has no columns to name, a
//! partial-index predicate sees only its own row (neither the parent's key nor a
//! sibling window's interval), and a range exclusion constraint would need
//! `btree_gist` on Postgres, has no `SQLite` expression at all, and would still
//! be per-`price_id` rather than per-key.
//!
//! **A trigger could carry it, and saying otherwise would be overstating the
//! case.** `pricing_price_tier_band_parent_kind` in `m20260802_000011` is
//! already a cross-table trigger in this chain: a `BEFORE INSERT OR UPDATE` arm
//! here could join `pricing_price` on the ten axes and scan the key's
//! occupying windows. It is not built, and the reason is scope rather than
//! impossibility — the rule needs the same key resolution, the same
//! occupying-state set and the same half-open arithmetic
//! [`window_repo`](crate::infra::storage::repo::window_repo) already holds, and a
//! second procedural spelling of it (PL/pgSQL plus a `SQLite` trigger body) would
//! be two more answers to which intervals collide.
//!
//! The residue that choice leaves is stated rather than implied: **non-overlap is
//! the only invariant of this table guarded at one layer instead of two**, so a
//! writer that reaches the table past
//! [`window_repo`](crate::infra::storage::repo::window_repo) — raw SQL, a
//! backfill, a future repository — meets nothing. That is the one guarantee a
//! reader must not read into the schema.
//!
//! # The trigger, and the arms
//!
//! The whitelist shape of `m20260802_000002_create_pricing_price` rather than an
//! unconditional ban, because most of what this table does is legal movement.
//! Five arms, in this order:
//!
//! 1. **`DELETE` is always refused.** "Cancel is a state, not a deletion" (§6,
//!    verbatim). There is no state, no lifecycle and no actor for which a window
//!    row may be removed: a cancelled window is the evidence that an operator
//!    unscheduled a price change, and an expired one is how a past instant is
//!    priced on replay.
//! 2. **An `UPDATE` of an `expired` or `cancelled` window is refused outright**
//!    — `inst-ws-immutable`'s "expired/cancelled windows are immutable history",
//!    which the transition arm alone does not give: a terminal row whose `state`
//!    stays put could otherwise still have its `effective_to` moved, rewriting
//!    the interval a billed period was priced over. `pricing_approval`'s
//!    "a decided record is immutable" arm is the same shape.
//! 3. **The frozen-column whitelist.** `window_id`, `tenant_id`, `price_id`,
//!    `effective_from`, `reason_code`, `created_by` and `created_at` are
//!    immutable — `price_id` because §6 makes the window/price binding immutable
//!    after creation (and with it the key the window is filed under),
//!    `effective_from` per the narrowing above. The **only** mutable columns are
//!    `state`, `effective_to` and the three flip timestamps.
//! 4. **The sanctioned transitions**, §4's three edges and no others.
//! 5. **`effective_to` may only be moved while it is in the future, and only to
//!    a future instant** — §6's "permitted UPDATEs: state-machine transitions,
//!    **future** `effective_to` adjustment". Both halves: a move *to* the past
//!    would reprice an interval that has already elapsed, and a move *of* an end
//!    that has already elapsed would resurrect coverage the key had lost. Moving
//!    `effective_to` to NULL is permitted — that is the open-ended extension of
//!    `inst-ws-expire`, and it removes no coverage.
//!
//! Membership is tested rather than change, as `pricing_plan_append_only` tests
//! it: a `NEW IS DISTINCT FROM OLD` conjunct on arm 4 would let the `SQLite`
//! mirror accept a no-op the Postgres branch refuses, and a backend divergence
//! is worse than the hole it would close.
//!
//! There is **no born-in-a-state `INSERT` guard**, and the honest reason is a
//! trade-off rather than the one first written here. `m20260802_000015` refuses a
//! record born anything but `submitted` because a record born `approved` defeats
//! the two-person rule *by existing* — there is no `UPDATE` for the decision plane
//! to guard. Nothing here is defeated that way: a window born `active` covers
//! exactly the interval it would have covered had it been flipped there, and the
//! biconditionals plus the three ordering CHECKs above now refuse every incoherent
//! born state a row-local predicate can see.
//!
//! What actually forbids the guard is narrower and worth writing down, because the
//! earlier reason — "later groups seed mid-life windows as fixtures" — was not the
//! constraint: **arm 5's "an end that has already passed" half is only testable by
//! a born-in-a-state INSERT.** No sanctioned mutation can produce a row whose
//! `effective_to` is already behind the clock, so `postgres_window.rs` and
//! `sqlite_window_guards.rs` both seed one straight into that shape, and a born-`scheduled`
//! INSERT guard would take that statement — and with it the proof of half of an arm
//! — away from the suite that proves this table. The residue is the same one and it
//! is still named: a row born `expired` whose `expired_at` sits at or after its
//! `effective_to` and whose `activated_at` sits at or after its `effective_from`
//! records a lifecycle nobody drove, and nothing in this table refuses it.
//!
//! There is no `REVOKE`. It names a deployment role this migration does not own
//! and `SQLite` has no `GRANT`/`REVOKE` at all; the trigger is the portable half
//! of the discipline §6 calls "REVOKE + column-whitelist trigger discipline"
//! (`m20260802_000001_create_pricing_plan.rs`).
//!
//! **Backend differences.** The systematic type mirror (`uuid` -> `text`,
//! `timestamptz` -> `text`), `now()` -> `(CURRENT_TIMESTAMP)`, and the trigger
//! split: Postgres carries one PL/pgSQL function interpolating the offending
//! values, while `SQLite` has no procedural language and `RAISE(ABORT, ...)`
//! takes a **literal** message only, so the five rules become five triggers with
//! fixed messages, `IS DISTINCT FROM` written `IS NOT`, and each arm's `WHEN`
//! carrying the terminal-state exclusion that arm 2's early return gives for
//! free on Postgres — so that exactly one of them can fire on any one statement,
//! in the same order.
//!
//! One further `SQLite` caveat is real rather than cosmetic, and it is the one this
//! mirror got **wrong** until 2026-08-04. Every instant is `text` on this backend,
//! so every comparison is lexicographic, and the rule that makes that safe has two
//! halves rather than one:
//!
//! * **Between two stored instants**, lexicographic comparison is exact. Every
//!   writer renders a `DateTime<Utc>` the same way — `SeaORM` produces RFC 3339
//!   with a `T` separator and a `+00:00` offset, measured, e.g.
//!   `2026-08-04T10:09:45+00:00` — and that rendering is fixed-width, zero-padded
//!   and monotonic, so `chk_pricing_price_window_interval`,
//!   `chk_pricing_price_window_activation_order`,
//!   `chk_pricing_price_window_expiry_order` and `list_for_plan`'s `ORDER BY` all
//!   compare the columns directly. This is `m20260802_000002`'s `grandfather_until`
//!   caveat, one table over. The cost is that a writer producing a *different*
//!   spelling silently breaks the ordering, which is why the mirror suite's
//!   fixtures are written in the deployed rendering and not in a legible one.
//! * **Against `CURRENT_TIMESTAMP`, it is not** — and arm 5 is the only place one
//!   side of a comparison is the clock. `CURRENT_TIMESTAMP` renders
//!   `YYYY-MM-DD HH:MM:SS`: a **space** at byte 11 where the stored text has `T`.
//!   The arm therefore normalizes with `datetime(…)`, which parses both spellings,
//!   applies the offset and yields the clock's own shape. The `substr(…, 1, 19)`
//!   prefix this arm carried before did not: byte 11 compared `'T'` (0x54) against
//!   `' '` (0x20), so **every stored instant on today's UTC date sorted greater
//!   than the clock and read as future**, and the arm was inoperative for exactly
//!   the same-day window. Cross-date it happened to be right, because the date
//!   prefix dominates before byte 11 is reached — which is why a suite of cross-date
//!   fixtures kept it green. `datetime(…)` truncates to whole seconds, so a
//!   sub-second-in-the-future end reads as *now* and the arm refuses it: that is a
//!   divergence from the Postgres branch's full-precision comparison, it is in the
//!   fail-closed direction, and a D-144 instant is quantized to the millisecond so
//!   the only affected window is one being moved within the current second.
//!
//! Postgres compares `timestamptz` values and needs none of this. The Postgres
//! `down` drops the function as well as the table; the `SQLite` one drops only the
//! table.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.pricing_price_window (
        window_id      uuid        NOT NULL PRIMARY KEY,
        tenant_id      uuid        NOT NULL,
        price_id       uuid        NOT NULL,
        effective_from timestamptz NOT NULL,
        effective_to   timestamptz,
        state          text        NOT NULL,
        reason_code    text        NOT NULL,
        created_by     uuid        NOT NULL,
        created_at     timestamptz NOT NULL DEFAULT now(),
        activated_at   timestamptz,
        expired_at     timestamptz,
        cancelled_at   timestamptz,
        CONSTRAINT chk_pricing_price_window_state CHECK (
            state IN ('scheduled','active','expired','cancelled')),
        -- Half-open and non-empty. `effective_to = effective_from` is not a
        -- zero-length window, it is a mistake; `effective_to = next.effective_from`
        -- between two rows is adjacency and is legal, which is why the bound is
        -- strict and there is no cross-row rule here at all.
        CONSTRAINT chk_pricing_price_window_interval CHECK (
            effective_to IS NULL OR effective_to > effective_from),
        -- An `expired` window was `active`, so it carries `activated_at`; a
        -- `cancelled` one never was, so it must not. See the module doc.
        CONSTRAINT chk_pricing_price_window_activated_at CHECK (
            (state IN ('active','expired')) = (activated_at IS NOT NULL)),
        CONSTRAINT chk_pricing_price_window_expired_at CHECK (
            (state = 'expired') = (expired_at IS NOT NULL)),
        CONSTRAINT chk_pricing_price_window_cancelled_at CHECK (
            (state = 'cancelled') = (cancelled_at IS NOT NULL)),
        -- Section 4's transition CONDITIONS, the three halves a row-local predicate
        -- can decide. `inst-ws-activate` fires WHEN `now >= effectiveFrom`, so an
        -- activation cannot be stamped before the start it was the arrival of.
        CONSTRAINT chk_pricing_price_window_activation_order CHECK (
            activated_at IS NULL OR activated_at >= effective_from),
        -- `inst-ws-expire` WHEN `now >= effectiveTo`, the same rule one edge over.
        -- Deliberately silent when `effective_to` is NULL - the comparison is then
        -- NULL, a CHECK admits NULL, and the constraint below owns that case alone.
        CONSTRAINT chk_pricing_price_window_expiry_order CHECK (
            expired_at IS NULL OR expired_at >= effective_to),
        -- `inst-ws-expire`, verbatim: an open-ended window never expires. There is
        -- no instant such an expiry could be the arrival of.
        CONSTRAINT chk_pricing_price_window_open_ended CHECK (
            NOT (state = 'expired' AND effective_to IS NULL)),
        CONSTRAINT fk_pricing_price_window_price FOREIGN KEY (price_id)
            REFERENCES bss.pricing_price (price_id)
    )",
    // Coverage and the overlap check read a key's windows by row.
    "CREATE INDEX idx_pricing_price_window_price
        ON bss.pricing_price_window (tenant_id, price_id)",
    // §10: "the activation job scans by `(state, effective_from)` index in
    // batches".
    "CREATE INDEX idx_pricing_price_window_due
        ON bss.pricing_price_window (state, effective_from)",
    "CREATE OR REPLACE FUNCTION bss.pricing_price_window_append_only() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'DELETE' THEN
            RAISE EXCEPTION
              'pricing_price_window: DELETE of window % is not permitted; cancel is a state, not a deletion',
              OLD.window_id;
          END IF;

          IF OLD.state IN ('expired','cancelled') THEN
            RAISE EXCEPTION
              'pricing_price_window: window % is %; an expired or cancelled window is immutable history',
              OLD.window_id, OLD.state;
          END IF;

          IF NEW.window_id      IS DISTINCT FROM OLD.window_id
          OR NEW.tenant_id      IS DISTINCT FROM OLD.tenant_id
          OR NEW.price_id       IS DISTINCT FROM OLD.price_id
          OR NEW.effective_from IS DISTINCT FROM OLD.effective_from
          OR NEW.reason_code    IS DISTINCT FROM OLD.reason_code
          OR NEW.created_by     IS DISTINCT FROM OLD.created_by
          OR NEW.created_at     IS DISTINCT FROM OLD.created_at THEN
            RAISE EXCEPTION
              'pricing_price_window: window % is bound to its price row and its start; only state, effective_to and the flip timestamps may move',
              OLD.window_id;
          END IF;

          IF NEW.state IS DISTINCT FROM OLD.state
             AND NOT (OLD.state = 'scheduled' AND NEW.state IN ('active','cancelled'))
             AND NOT (OLD.state = 'active'    AND NEW.state = 'expired') THEN
            RAISE EXCEPTION
              'pricing_price_window: state % -> % is not a sanctioned transition',
              OLD.state, NEW.state;
          END IF;

          IF NEW.effective_to IS DISTINCT FROM OLD.effective_to
             AND ((NEW.effective_to IS NOT NULL AND NEW.effective_to <= now())
               OR (OLD.effective_to IS NOT NULL AND OLD.effective_to <= now())) THEN
            RAISE EXCEPTION
              'pricing_price_window: the effective_to of window % may only be moved while it is in the future, and only to a future instant',
              OLD.window_id;
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_pricing_price_window_append_only
        BEFORE UPDATE OR DELETE ON bss.pricing_price_window
        FOR EACH ROW EXECUTE FUNCTION bss.pricing_price_window_append_only()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.pricing_price_window",
    "DROP FUNCTION IF EXISTS bss.pricing_price_window_append_only()",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------
//
// Systematic transforms from the Postgres variant:
// * schema prefix `bss.` dropped (single namespace);
// * `uuid` -> `text`, `timestamptz` -> `text`;
// * `now()` -> `(CURRENT_TIMESTAMP)`;
// * the single PL/pgSQL trigger becomes five `RAISE(ABORT, ...)` triggers
//   (SQLite has no `BEFORE UPDATE OR DELETE`, no procedural language and no
//   message interpolation), `IS DISTINCT FROM` becomes `IS NOT`, and every arm
//   below the terminal one repeats that arm's exclusion in its `WHEN` so that
//   one statement fires one trigger, in the Postgres order.
// Every CHECK, the FK and both indexes are preserved, name for name. See the
// module doc for the lexicographic `effective_to` caveat on arm 5.

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_price_window (
        window_id      text NOT NULL PRIMARY KEY,
        tenant_id      text NOT NULL,
        price_id       text NOT NULL,
        effective_from text NOT NULL,
        effective_to   text,
        state          text NOT NULL,
        reason_code    text NOT NULL,
        created_by     text NOT NULL,
        created_at     text NOT NULL DEFAULT (CURRENT_TIMESTAMP),
        activated_at   text,
        expired_at     text,
        cancelled_at   text,
        CONSTRAINT chk_pricing_price_window_state CHECK (
            state IN ('scheduled','active','expired','cancelled')),
        CONSTRAINT chk_pricing_price_window_interval CHECK (
            effective_to IS NULL OR effective_to > effective_from),
        CONSTRAINT chk_pricing_price_window_activated_at CHECK (
            (state IN ('active','expired')) = (activated_at IS NOT NULL)),
        CONSTRAINT chk_pricing_price_window_expired_at CHECK (
            (state = 'expired') = (expired_at IS NOT NULL)),
        CONSTRAINT chk_pricing_price_window_cancelled_at CHECK (
            (state = 'cancelled') = (cancelled_at IS NOT NULL)),
        -- The three ordering rules, in the same text as the Postgres branch: these
        -- compare two STORED instants, which the mirror renders identically, so no
        -- normalization is wanted here. See the module doc's two-halves caveat -
        -- only arm 5, whose other side is the clock, needs `datetime(...)`.
        CONSTRAINT chk_pricing_price_window_activation_order CHECK (
            activated_at IS NULL OR activated_at >= effective_from),
        CONSTRAINT chk_pricing_price_window_expiry_order CHECK (
            expired_at IS NULL OR expired_at >= effective_to),
        CONSTRAINT chk_pricing_price_window_open_ended CHECK (
            NOT (state = 'expired' AND effective_to IS NULL)),
        CONSTRAINT fk_pricing_price_window_price FOREIGN KEY (price_id)
            REFERENCES pricing_price (price_id)
    )",
    "CREATE INDEX idx_pricing_price_window_price
        ON pricing_price_window (tenant_id, price_id)",
    "CREATE INDEX idx_pricing_price_window_due
        ON pricing_price_window (state, effective_from)",
    "CREATE TRIGGER trg_pricing_price_window_no_delete
        BEFORE DELETE ON pricing_price_window
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_window: DELETE of a window is not permitted; cancel is a state, not a deletion');
        END",
    "CREATE TRIGGER trg_pricing_price_window_immutable_history
        BEFORE UPDATE ON pricing_price_window
        FOR EACH ROW WHEN OLD.state IN ('expired','cancelled')
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_window: an expired or cancelled window is immutable history');
        END",
    "CREATE TRIGGER trg_pricing_price_window_frozen_columns
        BEFORE UPDATE ON pricing_price_window
        FOR EACH ROW WHEN OLD.state NOT IN ('expired','cancelled')
          AND (NEW.window_id      IS NOT OLD.window_id
            OR NEW.tenant_id      IS NOT OLD.tenant_id
            OR NEW.price_id       IS NOT OLD.price_id
            OR NEW.effective_from IS NOT OLD.effective_from
            OR NEW.reason_code    IS NOT OLD.reason_code
            OR NEW.created_by     IS NOT OLD.created_by
            OR NEW.created_at     IS NOT OLD.created_at)
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_window: the window is bound to its price row and its start; only state, effective_to and the flip timestamps may move');
        END",
    "CREATE TRIGGER trg_pricing_price_window_flip_whitelist
        BEFORE UPDATE ON pricing_price_window
        FOR EACH ROW WHEN OLD.state NOT IN ('expired','cancelled')
          AND NEW.state IS NOT OLD.state
          AND NOT (OLD.state = 'scheduled' AND NEW.state IN ('active','cancelled'))
          AND NOT (OLD.state = 'active'    AND NEW.state = 'expired')
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_window: state transition is not a sanctioned one');
        END",
    "CREATE TRIGGER trg_pricing_price_window_future_end
        BEFORE UPDATE ON pricing_price_window
        FOR EACH ROW WHEN OLD.state NOT IN ('expired','cancelled')
          AND NEW.effective_to IS NOT OLD.effective_to
          AND ((NEW.effective_to IS NOT NULL
                AND datetime(NEW.effective_to) <= CURRENT_TIMESTAMP)
            OR (OLD.effective_to IS NOT NULL
                AND datetime(OLD.effective_to) <= CURRENT_TIMESTAMP))
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_window: effective_to may only be moved while it is in the future, and only to a future instant');
        END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_price_window"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
