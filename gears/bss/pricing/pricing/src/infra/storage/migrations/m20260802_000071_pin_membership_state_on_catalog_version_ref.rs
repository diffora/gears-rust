//! `pricing_catalog_version_ref` gains the one membership fact a mutation
//! moves — the interval's end, as the publish unit that minted the version
//! judged it.
//!
//! # The defect, which is `m20260802_000004`'s own, one plane over
//!
//! That migration added `subject_revision` and `subject_lifecycle_state`
//! because §4.4's "the projection source is the plan's **current** revision"
//! freezes whatever is current when the sweep arrives — up to D-47's five-minute
//! batching **maximum** after the commit — into an INSERT-only delta on the
//! ≥ 7-year horizon, in a store whose whole contract is that a completed version
//! never changes. D-165 made the pin normative: what a version freezes is what
//! its own publish **judged**, frozen on the ref row at commit and read from
//! there.
//!
//! The membership plane was exempt from that fix for exactly as long as it
//! minted **one** publish unit per membership row. `MembershipSubjectDelta`'s
//! own doc said so and named the obligation: *"whoever wires `enroll` and
//! `end_membership` into the registry request/pending-ref path owes that premise
//! a second look"*. Task 6 wired them — `POST …/members` and `PATCH
//! …/members/{id}` are each their own publish unit — and the premise stopped
//! holding in the same commit. Enroll, then end the membership before the sweep,
//! and the projector's live read of the row froze *the ended state* under **both**
//! versions: the enrollment's version says the payer's membership had already
//! ended at an instant when it had not, permanently, to the Tariffs surface that
//! resolves the group at `t` (D-30).
//!
//! # Why a column here rather than a revision table there
//!
//! The plan and overlay planes pin a **revision**, because their content is
//! revision-scoped and immutable once published: the ref names which immutable
//! rows to read and the projector reads them. `pricing_group_membership` has no
//! such table — a membership is minted by `enroll` and thereafter mutated **in
//! place**, and `group_membership_repo`'s only in-place mutation is
//! `end_membership` narrowing `effective_to` (`inst-ms-time`: "ending early =
//! setting `to`"; a *move* is an end plus a new row, never an edit of an
//! existing one). So the immutable half of a membership is the row itself and
//! the moving half is one column, which is precisely the shape
//! `subject_lifecycle_state` already has on the plan plane: the fact that moves
//! is frozen on the ref, the facts that cannot are read from the truth row.
//!
//! Giving the membership plane a revision-scoped content table was the
//! alternative and was rejected as a second store for content that has exactly
//! one mutable field: it would duplicate every membership row per mutation to
//! carry a value the ref row already has a place for, and put the truth and the
//! copy in a position to disagree.
//!
//! # `subject_revision` carries the row version, and that is what makes NULL
//! readable
//!
//! `NULL` in `subject_effective_to` has to mean *"judged open-ended"* — the
//! ordinary state of a live membership — so it cannot also mean "no pin was
//! written". The membership arm therefore pins the row's `row_version` in
//! `subject_revision` (the column's own doc calls it "the revision of the
//! subject the publish unit judged", which is what a row version is for a row
//! that has no revisions), and the projector **refuses** a membership subject
//! arriving without one rather than defaulting — `pinned_revision`'s existing
//! rule, applied to the kind that had been exempt from it. A pin's absence is
//! then a fault this gear can name, and its `NULL` end is a fact.
//!
//! # `ALTER TABLE`, not a rebuild
//!
//! `m20260802_000036` rebuilt this table because it widened the **primary key**,
//! which neither engine can do in place. A nullable column with no `CHECK` is
//! an `ALTER TABLE … ADD COLUMN` on both, so the rebuild — and the risk
//! `000036`'s own doc records of a restatement losing an object — is not bought
//! here. No `CHECK` pairs the column with `subject_kind`: the pairing rule is
//! per-kind and lives in the projector, which is where every other per-kind
//! reading of these columns already is (`subject_of`), and a `CHECK` would be a
//! second, partial spelling of it.
//!
//! **Reported as a divergence**, `m20260802_000004`'s own status for the two
//! columns it added: §3.7 lists neither of them nor this one, and §4.4's pin
//! rule (D-165) is stated for the plan plane only. What the set owes is the
//! sentence saying a membership publish unit pins the interval end it judged;
//! what it does not owe is a different mechanism, since this is D-165's applied
//! to the one plane whose truth row has no revisions.
//!
//! **Backend differences.** The systematic type mirror only: `timestamptz` on
//! Postgres, `text` on `SQLite`, exactly as `requested_at` and `committed_at`
//! are spelled on this same table.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &["ALTER TABLE bss.pricing_catalog_version_ref
        ADD COLUMN subject_effective_to timestamptz"];

const PG_DOWN_STATEMENTS: &[&str] = &["ALTER TABLE bss.pricing_catalog_version_ref
        DROP COLUMN IF EXISTS subject_effective_to"];

const SQLITE_UP_STATEMENTS: &[&str] = &["ALTER TABLE pricing_catalog_version_ref
        ADD COLUMN subject_effective_to text"];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["ALTER TABLE pricing_catalog_version_ref
        DROP COLUMN subject_effective_to"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
