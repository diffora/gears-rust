//! `pricing_policy_object.tax_display_policy_mode` — C4's fail-closed switch
//! (`design/04-currency-tax.md` §6, `inst-td-policy`).
//!
//! # The column §6 declares, beside the one that shares its name
//!
//! §6: *"**Tax-display policy** — a `pricing_policy_object` entry
//! (Foundation-owned table): `mode ∈ {fail_closed (default), warn}`;
//! per-tenant."* That is an **enforcement** mode over the two incomplete-basis
//! arms of `inst-td-policy`.
//!
//! `pricing_policy_object` already carries `tax_display_mode`, and it is a
//! **different fact**: `CHECK (tax_display_mode IN ('tax_inclusive',
//! 'tax_exclusive'))` — a *display basis* default, not an enforcement mode. The
//! declared value set is literally unstorable in it. Two further measurements
//! decided the shape of this migration:
//!
//! * **nothing in `src/` reads `tax_display_mode`** — no accessor on
//!   `AuthoringPolicy`, no domain type, no rule; the only references are the two
//!   migrations that create it and three test fixtures that set it;
//! * **`01-foundation.md` never declares it** — §3.7's policy-object cell lists
//!   the caps, the rounding default, the notice period and the descriptor
//!   extension, and no tax column.
//!
//! So the existing column is an implementation invention sitting on the name the
//! document asks for. This migration adds the column the document actually
//! declares and **leaves the other one alone**, which is register entry `T-2`'s
//! recommendation and its reasoning: repurposing it would need a `CHECK`
//! widening, which on `SQLite` is `m20260802_000019`'s create-copy-drop-rename
//! rebuild taking every trigger and index on the table with it, and would leave
//! the column's name describing neither of its two meanings. Dropping it is the
//! honest end state and is a **destructive** change to a Foundation-owned column
//! this slice did not author — a decision, not a side effect of an adjacent
//! slice. `T-2` keeps it open.
//!
//! # `fail_closed` is the default and it is the ratified one
//!
//! C4: *"Fail-closed for **all** tenants … both block publish unless the tenant
//! policy explicitly selects warn."* `NOT NULL DEFAULT 'fail_closed'` is
//! therefore not a convenience — it is the rule, applied to every tenant that has
//! never configured anything and to every tenant row that predates this column.
//! A nullable column would give "unconfigured" a second spelling and invite a
//! reader to treat it as "no policy", which is the one reading C4 forbids.
//!
//! # The `CHECK` is affordable here, unlike a widening
//!
//! This is an `ADD COLUMN` on both engines, and `SQLite` accepts a `CHECK` in
//! `ADD COLUMN` — it is only *altering* an existing one that has no incremental
//! form. So the constraint costs nothing and the two backends' statements differ
//! by the schema qualifier alone.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &["ALTER TABLE bss.pricing_policy_object
        ADD COLUMN tax_display_policy_mode text NOT NULL DEFAULT 'fail_closed'
        CONSTRAINT chk_pricing_policy_object_tax_display_policy
        CHECK (tax_display_policy_mode IN ('fail_closed', 'warn'))"];

const PG_DOWN_STATEMENTS: &[&str] =
    &["ALTER TABLE bss.pricing_policy_object DROP COLUMN tax_display_policy_mode"];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------

const SQLITE_UP_STATEMENTS: &[&str] = &["ALTER TABLE pricing_policy_object
        ADD COLUMN tax_display_policy_mode text NOT NULL DEFAULT 'fail_closed'
        CONSTRAINT chk_pricing_policy_object_tax_display_policy
        CHECK (tax_display_policy_mode IN ('fail_closed', 'warn'))"];

const SQLITE_DOWN_STATEMENTS: &[&str] =
    &["ALTER TABLE pricing_policy_object DROP COLUMN tax_display_policy_mode"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
