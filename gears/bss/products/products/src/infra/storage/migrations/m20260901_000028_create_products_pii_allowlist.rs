//! Create `bss.products_pii_allowlist` — the Legal-governed allow-list of
//! person-named strings the PII detector admits (**P-D-117** items 23 and 31;
//! `design/10-retention-erasure.md` `inst-pp-allowlist`) — and the
//! tombstone-inclusive read index **P-D-118** item 18 routed here.
//!
//! # The roster is P-D-117's, and the match rule is what the columns pin
//!
//! `(tenant_id, entry_id, value_normalized, justification, signed_off_by,
//! signed_off_at, state, created_at, updated_at)`. No document named a column
//! of this table before P-D-117 and the table did not exist; that entry is the
//! roster and this file is its first reader.
//!
//! **The match is exact equality on `value_normalized`, never a pattern.** C2
//! calls the list a *"curated allow-list for legitimate person-named
//! products"* — a list of names — and the narrowest rule is the one that
//! cannot admit more than Legal signed off. A pattern column would let one
//! signed-off entry widen itself after the sign-off, which is the control the
//! paper artifact exists to be.
//!
//! **The normalization is stated here because it is the whole of the rule**,
//! and [`crate::domain::retention::normalize_allowlist_value`] is its one
//! implementation: Unicode **NFKC**, then `trim`, then internal whitespace
//! runs collapsed to a single `U+0020`, then lowercase. The stored column is
//! always the output of that function and the detector compares its own
//! normalized subject against it, so both sides of the equality pass through
//! one code path. NFKC before case folding because the compatibility
//! decomposition is what makes a full-width or ligatured spelling reach the
//! same bytes; `trim` and the run collapse because an operator pasting a name
//! out of a document brings its whitespace with it; lowercase last because
//! folding a decomposed form is what `str::to_lowercase` is defined over.
//!
//! # `UNIQUE … WHERE state = 'active'`, and why revocation is a flip
//!
//! **P-D-47**'s reasoning, one table over: a revoked entry keeps its sign-off
//! on record, so revocation is a `state` flip and never a `DELETE`. The
//! uniqueness that matters is *"at most one **active** entry per normalized
//! value per tenant"* — a value revoked in March and signed off again in June
//! is two rows and two sign-offs, which is exactly the audit trail the control
//! is. A total `UNIQUE (tenant_id, value_normalized)` would force the second
//! sign-off to overwrite the first and destroy the evidence that the first
//! ever existed; the partial predicate is what keeps both.
//!
//! `entry_id` rather than the value as the key: the value is the *match*
//! operand and the governed act's aggregate is the *entry* (**P-D-118** item
//! 26 partitions `PiiAllowlistChanged` on `entry_id`), so an address that
//! survives a revoke-and-re-sign is the one the event can carry.
//!
//! `signed_off_by` and `signed_off_at` are `NOT NULL`: the mandatory Legal
//! sign-off reference is what `inst-pp-allowlist` refuses an entry without,
//! and a nullable column would make that refusal the door's alone. The refusal
//! itself rides `01`'s `VALIDATION` naming the field (**P-D-64**), so the
//! `NOT NULL` is the backstop and not the message.
//!
//! `justification` and `signed_off_by` are operator free text and go through
//! the content-PII write block at the door (**P-D-117** item 12): this table
//! is a PII store by construction and takes the identity map's posture.
//! `CHECK`s hold them non-empty so an entry cannot be signed off by the empty
//! string.
//!
//! # The index that rides this migration, and why it rides it
//!
//! **`idx_products_identity_ref_principal_tombstone`** — `(tenant_id,
//! principal_ref, tombstoned_at)` on `products_identity_ref`, routed here by
//! **P-D-118** item 18 on the same *"an index rides the change that makes its
//! read live"* reasoning P-D-110 and P-D-111 used. The compliance export walks
//! a principal's refs **including tombstoned ones**, and the only covering
//! index for that walk is the partial `uq_products_identity_ref_active`, whose
//! `WHERE tombstoned_at IS NULL` excludes exactly the rows a DSAR after an
//! erasure exists to return. `idx_products_identity_ref_principal` offers the
//! `(tenant_id, principal_ref)` prefix and leaves the tombstone column to a
//! row read.
//!
//! # Backend differences
//!
//! `uuid` becomes `text`, `timestamptz` becomes `text`, and the `bss.`
//! qualification is dropped. Every `CHECK`, the key, the partial unique and
//! both index predicates are preserved on both sides.
//!
//! **No `DELETE` guard.** The table is governed state and not evidence: an
//! entry is retired by the `state` flip above, and nothing in `design/10`
//! makes the row itself append-only. The sign-off it carries is evidence, and
//! what protects that is the flip plus the audit row the governed act writes —
//! not a trigger this file would be the first in the chain to invent.
//!
//! **No marker.** `dod-pii-allowlist` also obliges the door, the
//! `GovernedLiveOp` subject its mutation carries and the `PiiAllowlistChanged`
//! event; the table is what this file ships.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.products_pii_allowlist (
            tenant_id        uuid        NOT NULL,
            entry_id         uuid        NOT NULL,
            value_normalized text        NOT NULL,
            justification    text        NOT NULL,
            signed_off_by    text        NOT NULL,
            signed_off_at    timestamptz NOT NULL,
            state            text        NOT NULL,
            created_at       timestamptz NOT NULL,
            updated_at       timestamptz NOT NULL,
            CONSTRAINT products_pii_allowlist_pkey PRIMARY KEY (tenant_id, entry_id),
            CONSTRAINT chk_products_pii_allowlist_state CHECK (state IN ('active', 'revoked')),
            CONSTRAINT chk_products_pii_allowlist_value CHECK (value_normalized <> ''),
            CONSTRAINT chk_products_pii_allowlist_justification CHECK (justification <> ''),
            CONSTRAINT chk_products_pii_allowlist_signed_off_by CHECK (signed_off_by <> '')
        )",
    "CREATE UNIQUE INDEX uq_products_pii_allowlist_active ON bss.products_pii_allowlist USING btree (tenant_id, value_normalized) WHERE state = 'active'",
    "CREATE INDEX idx_products_identity_ref_principal_tombstone ON bss.products_identity_ref USING btree (tenant_id, principal_ref, tombstoned_at)",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP INDEX IF EXISTS bss.idx_products_identity_ref_principal_tombstone",
    "DROP INDEX IF EXISTS bss.uq_products_pii_allowlist_active",
    "DROP TABLE IF EXISTS bss.products_pii_allowlist",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE products_pii_allowlist (
            tenant_id        text NOT NULL,
            entry_id         text NOT NULL,
            value_normalized text NOT NULL,
            justification    text NOT NULL,
            signed_off_by    text NOT NULL,
            signed_off_at    text NOT NULL,
            state            text NOT NULL,
            created_at       text NOT NULL,
            updated_at       text NOT NULL,
            PRIMARY KEY (tenant_id, entry_id),
            CONSTRAINT chk_products_pii_allowlist_state CHECK (state IN ('active', 'revoked')),
            CONSTRAINT chk_products_pii_allowlist_value CHECK (value_normalized <> ''),
            CONSTRAINT chk_products_pii_allowlist_justification CHECK (justification <> ''),
            CONSTRAINT chk_products_pii_allowlist_signed_off_by CHECK (signed_off_by <> '')
        )",
    "CREATE UNIQUE INDEX uq_products_pii_allowlist_active ON products_pii_allowlist (tenant_id, value_normalized) WHERE state = 'active'",
    "CREATE INDEX idx_products_identity_ref_principal_tombstone ON products_identity_ref (tenant_id, principal_ref, tombstoned_at)",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "DROP INDEX IF EXISTS idx_products_identity_ref_principal_tombstone",
    "DROP INDEX IF EXISTS uq_products_pii_allowlist_active",
    "DROP TABLE IF EXISTS products_pii_allowlist",
];

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
