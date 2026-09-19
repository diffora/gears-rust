//! Create `bss.pricing_price` — the price rows, and with them the price
//! **history** (`design/01-foundation.md` §3.7): superseded rows are retained
//! in this same table, chained by `supersedes_price_id`; there is no separate
//! history table and no row is ever moved or deleted.
//!
//! The eight canonical scope-key columns (`plan_id`, `currency`, `region`,
//! `price_overlay`, `phase`, `price_eligibility`, `charge_kind`, `cohort` —
//! §4.1) carry a partial `UNIQUE` over `lifecycle_state = 'published'`: at most
//! one **current** row per key. `cohort` is stored as a `NOT NULL` text token
//! (`none`, or the cutover instant) rather than a nullable timestamp precisely
//! because it is an index column — distinct `NULL`s compare as distinct in a
//! Postgres unique index, so a nullable cohort would let two current rows share
//! a key.
//!
//! Both scope-key indexes lead with **`tenant_id`**, which the key itself does
//! not carry. The sibling index over the same table — Slice 2's
//! meter-injectivity partial `UNIQUE` (`02-plan-definition.md` §6, below) — was
//! review-fixed *to* `(tenant_id, plan_id, ...)`, and two indexes on one table
//! that disagree about whether a uniqueness scope starts at the tenant are two
//! different answers to "how far does this row's uniqueness reach". Nothing
//! observable moves today, because `plan_id` is a uuid and a plan belongs to one
//! tenant; what moves is that the index no longer relies on that being true.
//!
//! A **second** partial `UNIQUE` covers `lifecycle_state = 'draft'`, and it is
//! not symmetry. The published index cannot see a draft, so two concurrent
//! authoring calls on one key both find the key free and both commit — landing
//! exactly the second draft that `03-price-structure.md` `inst-pr-return`
//! (D-21) puts scope-key duplication among the save-time checks to refuse, and
//! leaving publish to discover the ambiguity a round trip later. A repository
//! pre-check is a read and cannot decide a race; this index is what does, the
//! way `uq_pricing_plan_open_draft` does for the plan's one editable revision.
//! The two indexes are disjoint by construction, so a key may still hold a
//! draft **and** its published predecessor at once — which is the state the
//! D-88 supersession unit works in, and the reason this is a second index
//! rather than a widened first one.
//!
//! A **third** partial `UNIQUE` — `uq_pricing_price_meter_line_current` — is
//! Slice 2's meter injectivity (`02-plan-definition.md` §6,
//! `inst-cmp-injective` / D-103) said where it can be enforced, and every way
//! it departs from the scope-key index above is load-bearing.
//!
//! It keys **per line, not per plan**. The rule once read "each usage plan
//! revision maps exactly one `meteringUnit`", and that stronger claim was
//! contradicted by three rules of its own slice and enforced by none
//! (D-103, 2026-07-31 review fix): D-84's per-market completeness ranges over
//! "every `(meter, dimensionKey)` line the plan prices", D-43's grants scope to
//! a **set** of metering units on one plan, and this index has always carried
//! `meter` **and** `dimension_key` — it implemented the per-line reading while
//! the prose still claimed the per-plan one. A `PaaS` plan pricing cloudlets,
//! storage and egress is one plan, not three. What is ambiguous, and what fails
//! publish, is a **duplicate line within one scope-key slice**.
//!
//! **It has a `published` arm and no `draft` twin, and that asymmetry is
//! deliberate** (review A1-5, recorded here so the question retires). The scope-key
//! pair above has both arms, and the reason its draft arm is load-bearing —
//! a pre-check is a read and cannot decide a race — is a general argument that
//! would apply here too. It does not have to: meter injectivity is stated as a
//! **publish** rule (`inst-cmp-injective`), so a draft holding a duplicate meter
//! line is a state the design admits and refuses one step later, where the whole
//! revision's line set is judged at once. Scope-key duplication is different in
//! kind: D-21 puts it among the **save-time** checks, so a second draft on one key
//! must never land, and only an index can promise that.
//!
//! `charge_kind` is **absent**, which is §6's own spelling and not an omission
//! this migration should repair. A meter is a usage row's column, so the axis
//! would discriminate nothing it is here to discriminate — what it would do is
//! let two rows pricing one line escape each other by disagreeing about their
//! charge kind, which is the ambiguity rather than an escape from it.
//!
//! `dimension_key` is `NOT NULL DEFAULT ''` — the empty-tuple sentinel — for
//! the reason `cohort` is a `NOT NULL` token (2026-07-28 review fix, confirmed
//! 2026-07-31). Undimensioned rows are the *ordinary* usage line, and under a
//! nullable column two of them on one key would compare as distinct here on
//! both engines and both land: the plan would price one meter twice with
//! nothing having objected, which is the very ambiguity this index exists to
//! refuse, in its commonest shape.
//!
//! `cohort` is **in** the key, as ADR-0002's generation axis. Without it a
//! second grandfathering cutover on a usage line would collide with the
//! generation the first cutover retained — the index would then refuse the
//! cutover instead of the duplicate, and the one operation that legitimately
//! adds a row to a line is the one it stopped.
//!
//! The predicate is `lifecycle_state = 'published'`, the same one the scope-key
//! index carries and sufficient for the same reason (2026-07-30 review fix): a
//! predecessor reads `superseded` the instant its successor commits. It is
//! deliberately **not** scoped by plan revision — an earlier spelling named a
//! `plan_revision` column `pricing_price` does not have — and the FR's "per
//! plan revision" reading is realized as current-rows-per-plan, historical
//! revisions keeping theirs through the supersession chain.
//!
//! `AND meter IS NOT NULL` is this migration's **addition** to that spelling
//! rather than the design's words, and it is semantically inert: a NULL `meter`
//! compares as distinct from every other value in a unique index on both
//! engines, so such a row could not have collided here in any case. What the
//! conjunct buys is that the index holds no entry at all for the recurring,
//! one-time and setup rows it can never speak about.
//!
//! `row_version` — the row's `ETag` — is here because **D-141** (2026-08-02) put
//! it here. Foundation §3.7's `pricing_price` bullet carries it explicitly, and
//! names the same three normative surfaces this comment lists as the reason a
//! per-row tag was required rather than a plan-level one.
//!
//! It read the other way until 2026-08-20 — that §3.7 *omitted* the column and
//! the omission was "a defect this migration reports rather than reconciles".
//! That was true when written and was reconciled by D-141 the same week; the
//! comment outlived it and sent a review pass to re-fix a closed thing. The
//! three surfaces:
//! `03-price-structure.md` §5 gives
//! `PATCH /bss-pricing/v1/plans/{planId}/prices/{priceId}` — "Update a draft
//! row" — the idempotency column `ETag`; `12-operator-efficiency.md`
//! `inst-bk-phase1` / `inst-bk-phase2` (D-118) have a bulk import edit "existing
//! draft rows under their `ETags`" with a conflict failing "only that row",
//! which is unsayable unless the tag is per row; and
//! `07-pricewindow-linkage.md` `inst-co-single-pending` draws the boundary
//! itself — "`ETag` protects rows, this rule protects change units". A per-row
//! rule needs a per-row column, so the column lands here and the design set is
//! left to record the omission.
//!
//! The physical guard is the append-only trigger with a **column whitelist**
//! (§4.3). A published row permits exactly two moves: the state-machine
//! transition `published -> superseded` (its two sanctioned producers are the
//! supersession unit and the grandfathering cutover commit, D-100), and
//! **monotonic tightening** of `grandfather_until` — setting it when null, or
//! moving it earlier. Loosening it (clearing it, or moving it later) is
//! rejected, as is any change to a price, scope or model column; DELETE of a
//! non-draft row is always rejected. Never-published draft rows stay mutable
//! and deletable.
//!
//! **The draft plane is guarded for transitions too (D-153, 2026-08-03, amended
//! in place while building the publish commit).** A column whitelist is scoped
//! to *published* rows by construction, so it says nothing about where a
//! **draft** row may go — and this trigger returned early for one. The price
//! row's state machine (`03-price-structure.md` §4) has exactly one edge out of
//! `draft`, to `published`, and until now nothing physical held it. A draft row
//! moved straight to `superseded` satisfies every constraint on this table and
//! lands **outside both** partial `UNIQUE` predicates: its key reads free on the
//! published plane *and* on the draft plane, so the guarantee D-148 had just
//! bought — the second concurrent creator is refused — is undone by one UPDATE,
//! and `inst-ps-nodelete` then makes the ghost undeletable, on a key no
//! supersession chain reaches because the row was never current. The trigger now
//! constrains the draft row's `lifecycle_state` as well: `draft -> draft |
//! published` and nothing else, exactly as `pricing_plan`'s
//! `trg_pricing_plan_draft_flip_whitelist` does for its own state set (D-145).
//! **No new code** — no API offers the transition and no caller can provoke it;
//! this is the physical floor under a state machine the engine already honours,
//! the same posture as the D-148 index itself.
//!
//! `row_version` is frozen by that same whitelist, alongside the content it
//! tags. An entity tag denotes a representation, and a published row's
//! representation cannot change — so a tag that moved under it would tell a
//! caller its cached copy is stale when it is not, and turn every `If-Match`
//! submit that had correctly read the row into a spurious `STALE_VERSION`. The
//! tag advances only where content does: on the draft plane.
//!
//! **Backend differences.** As in the plan table, Postgres uses one PL/pgSQL
//! trigger with interpolated messages and `SQLite` uses five `RAISE(ABORT, ...)`
//! triggers with literal ones. One further `SQLite` caveat is real rather than
//! cosmetic: `grandfather_until` is `text` there, so the monotonicity comparison
//! is **lexicographic**, which coincides with chronological order only for the
//! canonical fixed-width UTC rendering `SeaORM` writes. Postgres compares
//! `timestamptz` values.
//!
//! **The Slice-3 columns** (`design/03-price-structure.md` §6) —
//! `quantity_source`, `manual_quantity`, `package_size`,
//! `package_price_minor` — are slice-owned on this Foundation-owned table, and
//! they land on the row rather than in a table of their own because each is
//! single-valued per row: a `package` row's block size belongs to that row the
//! way `amount_minor` belongs to a `flat` one. Only the band set is
//! many-per-row, so only the band set gets a child table
//! (`pricing_price_tier_band`).
//!
//! §6's structural-exclusivity rule splits along that same line, and its
//! package half is here: package fields are permitted only on
//! `model_kind = 'package'`, which is a statement about one row and therefore
//! sayable as a row CHECK. The band half — band rows forbidden unless the kind
//! is `graduated` or `volume` — reads the parent from the child and is a
//! trigger over there.
//!
//! That CHECK spells the kind test `model_kind IS NOT NULL AND
//! model_kind = 'package'` rather than the shorter `model_kind = 'package'`,
//! and the longer form is the whole constraint. `model_kind` is nullable — a
//! draft may be authored before its kind is — so on a **kindless** row the
//! short comparison evaluates to NULL, `FALSE OR NULL` is NULL, and both
//! engines count a NULL CHECK result as satisfied. The rule would then admit
//! exactly the row it exists to refuse: package block fields with no kind to
//! give them meaning, which no Slice-3 rule reads and no rating applies. The
//! band half of the same §6 rule already says this out loud — its trigger tests
//! `parent_kind IS NULL OR parent_kind NOT IN (...)` and names the state
//! `kindless` in its message — so the two halves now refuse the same row.
//!
//! **Every token column is `CHECK`-constrained to the set its domain enum
//! renders**, and the ones that are only ever written by this gear are no
//! exception. `infra::storage::repo::price_repo` reads each of them back
//! through the inverse of that enum's `as_str()` and answers
//! [`RepoError::CorruptRow`] for anything else, on the stated ground that a
//! foreign token is an invariant breach rather than a caller mistake. That
//! ground is only true if the column cannot hold such a token in the first
//! place: without the CHECK, one `UPDATE` from a migration script or a
//! console session leaves a row that every read of it answers as an internal
//! fault forever, with nothing in the schema having objected at the moment the
//! value landed. Every one of them therefore has a negative case in
//! `tests/sqlite_price_checks.rs`, because a repository that writes only legal
//! values catches a CHECK that is too *narrow* and never one that has stopped
//! refusing. The one token no row CHECK can reach is `rolloverPolicy`, which
//! lives **inside** the `included_allowance` jsonb.
//!
//! `price_eligibility` admits **three** classes, not the two the grandfathering
//! cutover moves between: `new_subscriptions_only` is normative in its own right
//! (PRD §1.4 glossary and §6.9, AC #59, `07-pricewindow-linkage.md` W3 /
//! `inst-el-fields` / `inst-el-msw`, D-78, D-132) and sits between the other two
//! in the most-specific-wins order. It pairs with `cohort = 'none'` like
//! `all_subscriptions` does, so the biconditional below is unaffected — the
//! cohort axis discriminates *retained* generations and this class retains
//! nobody — and so is the `grandfather_until` pairing, which stays exclusive to
//! `existing_grandfathered`.
//!
//! `lifecycle_state` is the one token column whose CHECK is deliberately
//! **narrower** than the enum that renders it. `domain::lifecycle::LifecycleState`
//! is shared with plan revisions, which legitimately reach `retired`
//! (`01-foundation.md` §3.7, D-128) and `abandoned` (D-145); the **price-row**
//! state machine (`03-price-structure.md` §4) has three states — draft,
//! published, superseded — and no edge to either. A row in either state would
//! fall outside both partial `UNIQUE` indexes below, so the
//! one-current-row-per-key guarantee would simply stop covering it: the key
//! would read as free and take a second published row beside it. `abandoned`
//! has nothing to express here in any case — D-145 is scoped to the plan
//! revision row, and a never-published **draft price row stays deletable**
//! (§4.3, `inst-ps-nodelete`), which is why `DELETE` below is rejected for
//! published rows only while `pricing_plan` rejects it outright.
//!
//! [`RepoError::CorruptRow`]: crate::infra::storage::RepoError::CorruptRow
//!
//! `included_allowance` is a **Slice-10-declared** column
//! (`design/10-advanced-primitives.md` §6) carried here ahead of its slice, and
//! nothing of that slice's behaviour comes with it: no declaration is compiled,
//! no `$0` band is synthesized, no marker is projected (D-45 / D-130 are
//! Slice-10 work). It is a column because two standing pieces of this gear
//! already read the field — `domain::price_row::PriceRow` carries it, and the
//! D-129 supersession-unit guard compares it across a `carry` row's successor —
//! and a row storage without the column would let the round trip drop the one
//! field that guard looks at, which the guard would then read as "nothing
//! changed" rather than as an error.
//!
//! All five join the frozen-column whitelist, for the reason the whitelist
//! exists: they are content, and a published row's content does not move.
//!
//! # About this file
//!
//! # What the columns beyond §6's list are for, and the two rules they share
//!
//! **Not a roster.** The table carries more columns than this paragraph names and
//! the guard freezes more still. The two schema goldens hold the roster as the
//! engine created it; a completeness claim taken from the prose below is a claim
//! about a list nothing keeps current. What follows is what the groups are *for*.
//!
//! Tax: `tax_category_ref` and `resolved_tax_category` (D-42's authored reference and
//! the value publish froze). Proration: `proration_basis`. Reservation:
//! `reservation_flavor` and `reserved_rate_nano`. Discount: `discount_ref`.
//! Rounding: `rounding_policy_ref`, the authored reference, and
//! `resolved_rounding_policy`, the value publish froze against the tenant's declared
//! vocabulary. Purchase and usage minima: `min_qty_purchase`, `min_qty_usage` and
//! `min_qty_usage_fallback`. Tiering: `tier_aggregation_window` and
//! `tier_qualification_window`. Plus `unit_rate_nano`, the `per_unit` rate, and
//! `row_version`.
//!
//! **Two rules run through all of them.**
//!
//! *An amount column holds amounts and a rate column holds rates* (D-311). A rate is
//! a `bigint` counting 10⁻⁹ minor units and carries `_nano` in its name;
//! `amount_minor` counts minor units and nothing else. `unit_rate_nano` and
//! `reserved_rate_nano` exist because sharing `amount_minor` by `model_kind` made one
//! column mean two things — and the read and write paths disagreed by 10⁹ while every
//! gate was green.
//!
//! *Every one of them is frozen on a published row.* `pricing_price_append_only`
//! enumerates the frozen columns — a column absent from its predicate is a
//! published price that can be edited after the fact. The guard body is rewritten
//! with each column, in the same act, and the two schema goldens carry the
//! predicate as the engine created it: read the roster there rather than from this
//! paragraph, which names what the columns are *for* and not what the guard holds.
//!
//! Dependency level 0 **by column**: it declares no foreign key. Its
//! `pricing_price_tier_band_parent_kind` trigger body reads
//! `bss.pricing_price_tier_band`, which `000038` creates, so the chain's one
//! binding rule — a table sorts after every table it references
//! (`migrations.rs`) — is inverted here. A forward install survives it because
//! both engines resolve trigger-body names at execution rather than at creation;
//! a rollback of `000038` alone does not, and leaves this table's trigger
//! pointing at a dropped relation so that every UPDATE of `pricing_price` fails.
//! Columns read identity first, then content by name, then the audit columns.
//!
//! The SQL is generated by `tasks/emit_chain.py` from the frozen schema goldens and
//! is rewritten on every run; this doc is not. What dissolved into this migration is
//! recorded in `tasks/migration-inventory.md`, which is where to look for the chain's
//! own history — nothing above narrates it, because a fresh-install chain has none.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.pricing_price (
            tenant_id                 uuid        NOT NULL,
            price_id                  uuid        NOT NULL,
            market_price_id           uuid        NOT NULL,
            line_version_id           uuid        NOT NULL,
            charge_line_id            uuid        NOT NULL,
            plan_id                   uuid        NOT NULL,
            plan_revision             bigint      NOT NULL,
            amount_minor              bigint,
            unit_rate_nano            bigint,
            package_price_minor       bigint,
            reserved_rate_nano        bigint,
            tax_inclusive             boolean     NOT NULL DEFAULT false,
            tax_category_ref          text,
            resolved_tax_category     text,
            rounding_policy_ref       text,
            resolved_rounding_policy  text,
            grandfather_until         timestamptz,
            supersedes_price_id       uuid,
            lifecycle_state           text        NOT NULL,
            created_at_utc            timestamptz NOT NULL DEFAULT now(),
            created_by                uuid        NOT NULL,
            row_version               bigint      NOT NULL DEFAULT 0,
            CONSTRAINT chk_pricing_price_amount_non_negative CHECK (amount_minor IS NULL OR amount_minor >= 0),
            CONSTRAINT chk_pricing_price_lifecycle_state CHECK (lifecycle_state IN ('draft','published','superseded')),
            CONSTRAINT chk_pricing_price_package_price CHECK (package_price_minor IS NULL OR package_price_minor >= 0),
            CONSTRAINT chk_pricing_price_reserved_rate_nano CHECK (reserved_rate_nano IS NULL OR reserved_rate_nano >= 0),
            CONSTRAINT chk_pricing_price_row_version CHECK (row_version >= 0),
            CONSTRAINT chk_pricing_price_unit_rate_nano CHECK (unit_rate_nano IS NULL OR unit_rate_nano >= 0),
            CONSTRAINT chk_pricing_price_revision CHECK (plan_revision >= 0),
            CONSTRAINT fk_pricing_price_market_line FOREIGN KEY (tenant_id, market_price_id, charge_line_id)
                REFERENCES bss.pricing_market_price (tenant_id, market_price_id, charge_line_id),
            CONSTRAINT fk_pricing_price_version_line FOREIGN KEY (tenant_id, line_version_id, charge_line_id)
                REFERENCES bss.pricing_charge_line_version (tenant_id, line_version_id, charge_line_id),
            CONSTRAINT fk_pricing_price_line_plan FOREIGN KEY (tenant_id, charge_line_id, plan_id)
                REFERENCES bss.pricing_charge_line (tenant_id, charge_line_id, plan_id),
            CONSTRAINT uq_pricing_price_tenant_id UNIQUE (tenant_id, price_id),
            CONSTRAINT uq_pricing_price_id_market UNIQUE (tenant_id, price_id, market_price_id),
            CONSTRAINT uq_pricing_price_id_version UNIQUE (tenant_id, price_id, line_version_id),
            CONSTRAINT pricing_price_pkey PRIMARY KEY (price_id)
        )",
    "CREATE INDEX idx_pricing_price_line ON bss.pricing_price USING btree (tenant_id, charge_line_id, lifecycle_state)",
    "CREATE INDEX idx_pricing_price_plan ON bss.pricing_price USING btree (tenant_id, plan_id, lifecycle_state)",
    "CREATE INDEX idx_pricing_price_market ON bss.pricing_price USING btree (tenant_id, market_price_id, lifecycle_state)",
    "CREATE INDEX idx_pricing_price_supersedes ON bss.pricing_price USING btree (tenant_id, supersedes_price_id) WHERE (supersedes_price_id IS NOT NULL)",
    "CREATE UNIQUE INDEX uq_pricing_price_market_draft ON bss.pricing_price USING btree (tenant_id, market_price_id) WHERE (lifecycle_state = 'draft'::text)",
    "CREATE OR REPLACE FUNCTION bss.pricing_price_append_only() RETURNS trigger AS $$
        DECLARE
          elig text;
        BEGIN
          IF TG_OP = 'DELETE' THEN
            IF OLD.lifecycle_state <> 'draft' THEN
              RAISE EXCEPTION 'pricing_price: DELETE of a % row is not permitted',
                OLD.lifecycle_state;
            END IF;
            RETURN OLD;
          END IF;

          IF NEW.grandfather_until IS NOT NULL THEN
            SELECT l.price_eligibility INTO elig
              FROM bss.pricing_charge_line l
             WHERE l.tenant_id = NEW.tenant_id AND l.charge_line_id = NEW.charge_line_id;
            IF elig IS DISTINCT FROM 'existing_grandfathered' THEN
              RAISE EXCEPTION
                'pricing_price: grandfather_until is permitted only on an existing_grandfathered line (row %)',
                NEW.price_id;
            END IF;
          END IF;

          IF OLD.lifecycle_state = 'draft' THEN
            IF NEW.lifecycle_state NOT IN ('draft', 'published') THEN
              RAISE EXCEPTION
                'pricing_price: lifecycle_state draft -> % is not a sanctioned transition',
                NEW.lifecycle_state;
            END IF;
            RETURN NEW;
          END IF;

          IF NEW.price_id                  IS DISTINCT FROM OLD.price_id
          OR NEW.tenant_id                 IS DISTINCT FROM OLD.tenant_id
          OR NEW.market_price_id           IS DISTINCT FROM OLD.market_price_id
          OR NEW.line_version_id           IS DISTINCT FROM OLD.line_version_id
          OR NEW.charge_line_id            IS DISTINCT FROM OLD.charge_line_id
          OR NEW.plan_id                   IS DISTINCT FROM OLD.plan_id
          OR NEW.plan_revision             IS DISTINCT FROM OLD.plan_revision
          OR NEW.amount_minor              IS DISTINCT FROM OLD.amount_minor
          OR NEW.unit_rate_nano            IS DISTINCT FROM OLD.unit_rate_nano
          OR NEW.package_price_minor       IS DISTINCT FROM OLD.package_price_minor
          OR NEW.reserved_rate_nano        IS DISTINCT FROM OLD.reserved_rate_nano
          OR NEW.tax_inclusive             IS DISTINCT FROM OLD.tax_inclusive
          OR NEW.tax_category_ref          IS DISTINCT FROM OLD.tax_category_ref
          OR NEW.resolved_tax_category     IS DISTINCT FROM OLD.resolved_tax_category
          OR NEW.resolved_rounding_policy  IS DISTINCT FROM OLD.resolved_rounding_policy
          OR NEW.rounding_policy_ref       IS DISTINCT FROM OLD.rounding_policy_ref
          OR NEW.supersedes_price_id       IS DISTINCT FROM OLD.supersedes_price_id
          OR NEW.created_by                IS DISTINCT FROM OLD.created_by
          OR NEW.created_at_utc            IS DISTINCT FROM OLD.created_at_utc
          OR NEW.row_version               IS DISTINCT FROM OLD.row_version THEN
            RAISE EXCEPTION
              'pricing_price: row % is published; price, market-policy and entity-tag columns are immutable',
              OLD.price_id;
          END IF;

          IF NEW.lifecycle_state IS DISTINCT FROM OLD.lifecycle_state
             AND NOT (OLD.lifecycle_state = 'published'
                      AND NEW.lifecycle_state = 'superseded') THEN
            RAISE EXCEPTION 'pricing_price: lifecycle_state % -> % is not a sanctioned transition',
              OLD.lifecycle_state, NEW.lifecycle_state;
          END IF;

          IF NEW.grandfather_until IS DISTINCT FROM OLD.grandfather_until
             AND (NEW.grandfather_until IS NULL
                  OR (OLD.grandfather_until IS NOT NULL
                      AND NEW.grandfather_until > OLD.grandfather_until)) THEN
            RAISE EXCEPTION
              'pricing_price: grandfather_until may only be tightened, never loosened (row %)',
              OLD.price_id;
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_pricing_price_append_only BEFORE DELETE OR UPDATE ON bss.pricing_price FOR EACH ROW EXECUTE FUNCTION bss.pricing_price_append_only()",
    "CREATE OR REPLACE FUNCTION bss.pricing_price_grandfather_class() RETURNS trigger AS $$
        DECLARE
          elig text;
        BEGIN
          IF NEW.grandfather_until IS NULL THEN
            RETURN NEW;
          END IF;
          SELECT l.price_eligibility INTO elig
            FROM bss.pricing_charge_line l
           WHERE l.tenant_id = NEW.tenant_id AND l.charge_line_id = NEW.charge_line_id;
          IF elig IS DISTINCT FROM 'existing_grandfathered' THEN
            RAISE EXCEPTION
              'pricing_price: grandfather_until is permitted only on an existing_grandfathered line (row %)',
              NEW.price_id;
          END IF;
          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_pricing_price_grandfather_class BEFORE INSERT ON bss.pricing_price FOR EACH ROW EXECUTE FUNCTION bss.pricing_price_grandfather_class()",
    // **The money half of the old `chk_pricing_price_package_fields_kind`.**
    // That CHECK read "a block field requires `model_kind = 'package'`" over two
    // columns of one row. The block *size* is shared geometry and moved to
    // `pricing_charge_line_version` with the kind, where the rule is still a
    // CHECK. The block *price* is market money and stayed here — and the kind it
    // needs is now one table over, which no CHECK can reach. Without this guard
    // the split would have silently dropped half a rule: a `flat` line could
    // carry a package price and nothing would say so.
    "CREATE OR REPLACE FUNCTION bss.pricing_price_package_price_kind() RETURNS trigger AS $$
        DECLARE
          kind text;
        BEGIN
          IF NEW.package_price_minor IS NULL THEN
            RETURN NEW;
          END IF;
          SELECT v.model_kind INTO kind
            FROM bss.pricing_charge_line_version v
           WHERE v.tenant_id = NEW.tenant_id AND v.line_version_id = NEW.line_version_id;
          IF kind IS DISTINCT FROM 'package' THEN
            RAISE EXCEPTION
              'pricing_price: package_price_minor is permitted only on a package line version (row %)',
              NEW.price_id;
          END IF;
          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_pricing_price_package_price_kind BEFORE INSERT OR UPDATE ON bss.pricing_price FOR EACH ROW EXECUTE FUNCTION bss.pricing_price_package_price_kind()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.pricing_price",
    "DROP FUNCTION IF EXISTS bss.pricing_price_append_only()",
    "DROP FUNCTION IF EXISTS bss.pricing_price_grandfather_class()",
    "DROP FUNCTION IF EXISTS bss.pricing_price_package_price_kind()",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_price (
            tenant_id                 text       NOT NULL,
            price_id                  text       NOT NULL,
            market_price_id           text       NOT NULL,
            line_version_id           text       NOT NULL,
            charge_line_id            text       NOT NULL,
            plan_id                   text       NOT NULL,
            plan_revision             bigint     NOT NULL,
            amount_minor              bigint,
            unit_rate_nano            bigint,
            package_price_minor       bigint,
            reserved_rate_nano        bigint,
            tax_inclusive             boolean    NOT NULL DEFAULT false,
            tax_category_ref          text,
            resolved_tax_category     text,
            rounding_policy_ref       text,
            resolved_rounding_policy  text,
            grandfather_until         text,
            supersedes_price_id       text,
            lifecycle_state           text       NOT NULL,
            created_at_utc            text       NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%S', 'now') || '+00:00'),
            created_by                text       NOT NULL,
            row_version               bigint     NOT NULL DEFAULT 0,
            PRIMARY KEY (price_id),
            CONSTRAINT chk_pricing_price_amount_non_negative CHECK (amount_minor IS NULL OR amount_minor >= 0),
            CONSTRAINT chk_pricing_price_lifecycle_state CHECK (lifecycle_state IN ('draft','published','superseded')),
            CONSTRAINT chk_pricing_price_package_price CHECK (package_price_minor IS NULL OR package_price_minor >= 0),
            CONSTRAINT chk_pricing_price_reserved_rate_nano CHECK (reserved_rate_nano IS NULL OR reserved_rate_nano >= 0),
            CONSTRAINT chk_pricing_price_row_version CHECK (row_version >= 0),
            CONSTRAINT chk_pricing_price_unit_rate_nano CHECK (unit_rate_nano IS NULL OR unit_rate_nano >= 0),
            CONSTRAINT chk_pricing_price_revision CHECK (plan_revision >= 0),
            CONSTRAINT fk_pricing_price_market_line FOREIGN KEY (tenant_id, market_price_id, charge_line_id)
                REFERENCES pricing_market_price (tenant_id, market_price_id, charge_line_id),
            CONSTRAINT fk_pricing_price_version_line FOREIGN KEY (tenant_id, line_version_id, charge_line_id)
                REFERENCES pricing_charge_line_version (tenant_id, line_version_id, charge_line_id),
            CONSTRAINT fk_pricing_price_line_plan FOREIGN KEY (tenant_id, charge_line_id, plan_id)
                REFERENCES pricing_charge_line (tenant_id, charge_line_id, plan_id),
            CONSTRAINT uq_pricing_price_tenant_id UNIQUE (tenant_id, price_id),
            CONSTRAINT uq_pricing_price_id_market UNIQUE (tenant_id, price_id, market_price_id),
            CONSTRAINT uq_pricing_price_id_version UNIQUE (tenant_id, price_id, line_version_id)
        )",
    "CREATE INDEX idx_pricing_price_line ON pricing_price (tenant_id, charge_line_id, lifecycle_state)",
    "CREATE INDEX idx_pricing_price_plan ON pricing_price (tenant_id, plan_id, lifecycle_state)",
    "CREATE INDEX idx_pricing_price_market ON pricing_price (tenant_id, market_price_id, lifecycle_state)",
    "CREATE INDEX idx_pricing_price_supersedes ON pricing_price (tenant_id, supersedes_price_id) WHERE supersedes_price_id IS NOT NULL",
    "CREATE UNIQUE INDEX uq_pricing_price_market_draft ON pricing_price (tenant_id, market_price_id) WHERE lifecycle_state = 'draft'",
    "CREATE TRIGGER trg_pricing_price_draft_flip_whitelist BEFORE UPDATE ON pricing_price FOR EACH ROW WHEN OLD.lifecycle_state = 'draft' AND NEW.lifecycle_state NOT IN ('draft','published') BEGIN SELECT RAISE(ABORT, 'pricing_price: lifecycle_state transition is not sanctioned'); END",
    "CREATE TRIGGER trg_pricing_price_flip_whitelist BEFORE UPDATE ON pricing_price FOR EACH ROW WHEN OLD.lifecycle_state <> 'draft' AND NEW.lifecycle_state IS NOT OLD.lifecycle_state AND NOT (OLD.lifecycle_state = 'published' AND NEW.lifecycle_state = 'superseded') BEGIN SELECT RAISE(ABORT, 'pricing_price: lifecycle_state transition is not sanctioned'); END",
    "CREATE TRIGGER trg_pricing_price_frozen_columns BEFORE UPDATE ON pricing_price FOR EACH ROW WHEN OLD.lifecycle_state <> 'draft' AND (NEW.price_id IS NOT OLD.price_id OR NEW.tenant_id IS NOT OLD.tenant_id OR NEW.market_price_id IS NOT OLD.market_price_id OR NEW.line_version_id IS NOT OLD.line_version_id OR NEW.charge_line_id IS NOT OLD.charge_line_id OR NEW.plan_id IS NOT OLD.plan_id OR NEW.plan_revision IS NOT OLD.plan_revision OR NEW.amount_minor IS NOT OLD.amount_minor OR NEW.unit_rate_nano IS NOT OLD.unit_rate_nano OR NEW.package_price_minor IS NOT OLD.package_price_minor OR NEW.reserved_rate_nano IS NOT OLD.reserved_rate_nano OR NEW.tax_inclusive IS NOT OLD.tax_inclusive OR NEW.tax_category_ref IS NOT OLD.tax_category_ref OR NEW.resolved_tax_category IS NOT OLD.resolved_tax_category OR NEW.resolved_rounding_policy IS NOT OLD.resolved_rounding_policy OR NEW.rounding_policy_ref IS NOT OLD.rounding_policy_ref OR NEW.supersedes_price_id IS NOT OLD.supersedes_price_id OR NEW.created_by IS NOT OLD.created_by OR NEW.created_at_utc IS NOT OLD.created_at_utc OR NEW.row_version IS NOT OLD.row_version) BEGIN SELECT RAISE(ABORT, 'pricing_price: row is published; price, market-policy and entity-tag columns are immutable'); END",
    "CREATE TRIGGER trg_pricing_price_grandfather_monotonic BEFORE UPDATE ON pricing_price FOR EACH ROW WHEN OLD.lifecycle_state <> 'draft' AND NEW.grandfather_until IS NOT OLD.grandfather_until AND (NEW.grandfather_until IS NULL OR (OLD.grandfather_until IS NOT NULL AND NEW.grandfather_until > OLD.grandfather_until)) BEGIN SELECT RAISE(ABORT, 'pricing_price: grandfather_until may only be tightened, never loosened'); END",
    "CREATE TRIGGER trg_pricing_price_no_delete BEFORE DELETE ON pricing_price FOR EACH ROW WHEN OLD.lifecycle_state <> 'draft' BEGIN SELECT RAISE(ABORT, 'pricing_price: DELETE of a non-draft row is not permitted'); END",
    "CREATE TRIGGER trg_pricing_price_grandfather_class_insert BEFORE INSERT ON pricing_price FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'pricing_price: grandfather_until is permitted only on an existing_grandfathered line') WHERE NEW.grandfather_until IS NOT NULL AND NOT EXISTS (SELECT 1 FROM pricing_charge_line WHERE tenant_id = NEW.tenant_id AND charge_line_id = NEW.charge_line_id AND price_eligibility = 'existing_grandfathered'); END",
    "CREATE TRIGGER trg_pricing_price_grandfather_class_update BEFORE UPDATE ON pricing_price FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'pricing_price: grandfather_until is permitted only on an existing_grandfathered line') WHERE NEW.grandfather_until IS NOT NULL AND NOT EXISTS (SELECT 1 FROM pricing_charge_line WHERE tenant_id = NEW.tenant_id AND charge_line_id = NEW.charge_line_id AND price_eligibility = 'existing_grandfathered'); END",
    "CREATE TRIGGER trg_pricing_price_package_price_kind_insert BEFORE INSERT ON pricing_price FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'pricing_price: package_price_minor is permitted only on a package line version') WHERE NEW.package_price_minor IS NOT NULL AND NOT EXISTS (SELECT 1 FROM pricing_charge_line_version WHERE tenant_id = NEW.tenant_id AND line_version_id = NEW.line_version_id AND model_kind = 'package'); END",
    "CREATE TRIGGER trg_pricing_price_package_price_kind_update BEFORE UPDATE ON pricing_price FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'pricing_price: package_price_minor is permitted only on a package line version') WHERE NEW.package_price_minor IS NOT NULL AND NOT EXISTS (SELECT 1 FROM pricing_charge_line_version WHERE tenant_id = NEW.tenant_id AND line_version_id = NEW.line_version_id AND model_kind = 'package'); END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_price"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(self.name(), manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(
            self.name(),
            manager,
            PG_DOWN_STATEMENTS,
            SQLITE_DOWN_STATEMENTS,
        )
        .await
    }
}
