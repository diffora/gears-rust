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
//! # The backfill, which the refusal above makes mandatory rather than tidy
//!
//! Every membership ref written before this migration carries
//! `subject_revision = NULL`: the pre-fix `membership_publish::record_ref` used
//! `PendingVersionRow::for_subject(…, None, None, …)` for this kind, on the
//! stated ground that a membership had nothing to pin. So on any environment
//! that ran `POST …/members` or `PATCH …/members/{id}` before this migration —
//! the membership plane landed earlier on this same branch and the branch is on
//! the shared line — the projector's refusal lands on a **pre-existing** row,
//! and its consequence is not a logged fault but a stalled tenant:
//! `read_model::project_version` catches the error, counts the subject `failed`
//! and leaves the ref pending, the frontier advances only in version order
//! (D-114's prefix), and every later version of that tenant therefore queues
//! behind one un-pinnable ref, forever, with no operator remedy. A schema
//! change whose new rule refuses rows the old writer produced owes those rows a
//! value, in `up`, in the same transaction that installs the rule.
//!
//! **The value is the truth row's own, not a sentinel.** `up` joins each
//! unpinned membership ref to `pricing_group_membership` and copies
//! `row_version` and `effective_to` off it. What that claims is exactly this:
//! *the state this ref's publish judged is not recorded anywhere, and the best
//! available reading of it is the row as it stands at migration time* — which is
//! precisely what the pre-fix projector would have read for these very refs when
//! their sweep arrived, one live read later. So the backfill preserves the
//! behaviour those rows were written under, and preserves it at an instant
//! *closer* to their commit than the sweep would have. For a ref minted by
//! `end_membership` it is also simply correct: that mutation's output is the
//! ended interval, and nothing has moved the row since. For an enrolment whose
//! membership was ended before this migration it reproduces the original defect
//! (the ended state under the enrolment's version) — unfixably, because the
//! instant that publish judged was never written down, which is the whole reason
//! this migration exists.
//!
//! **A backfilled `0` was the alternative and is rejected, because `0` is a
//! genuine pin on this plane.** `PendingVersionRow::for_membership` pins the row
//! version the mutation *produced*, and an enrolment produces `0`
//! (`pricing_group_membership.row_version` starts there). A `0` written here
//! would therefore be indistinguishable from a real enrolment's pin — the exact
//! property the reviewer asked to be sure of — and, worse, it would pair with
//! `subject_effective_to = NULL`, which on this kind is a positive assertion that
//! the publish *judged the interval open-ended*. On a pre-existing
//! `end_membership` ref that is a false claim of the kind D-165 exists to
//! prevent: the read model would advertise a membership as still running under
//! the very version whose publish ended it. Copying the row makes no claim the
//! store cannot support and needs no sentinel to be read correctly.
//!
//! A membership ref whose truth row is **absent** is left untouched, and stays
//! refused. That row is D-128's invariant breach — a ref this gear wrote naming
//! a subject that is not there — and inventing a pin for it would only change
//! which fault the projector reports about it, never make it projectable.
//!
//! `down` is the exact inverse and can be, for the same reason the backfill is
//! needed: before this migration **no** membership ref carried a revision, so
//! nulling `subject_revision` on every `group_membership` row restores the
//! pre-migration state precisely, whether the value there was backfilled by `up`
//! or pinned afterwards by `for_membership`. `up` re-derives it from the truth
//! row on the way forward. The information `down` destroys is the interval end,
//! and that is inherent in dropping the column that holds it.
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
//! **Backend differences.** The column is the systematic type mirror only:
//! `timestamptz` on Postgres, `text` on `SQLite`, exactly as `requested_at` and
//! `committed_at` are spelled on this same table. The **backfill** is not a
//! mirror and the two arms share no clause: Postgres has `UPDATE … FROM` and a
//! real `uuid` type, so it is a join with a `::text` cast; the mirror has
//! neither, and additionally stores a `Uuid` as a 16-byte blob that no column
//! affinity converts, so the membership id is matched in hex there. Both arms are
//! executed — `postgres_schema_stores::the_membership_pin_backfills_the_refs_written_before_it`
//! and `sqlite_read_model::a_membership_ref_written_before_the_pin_existed_sweeps_after_the_backfill`,
//! each applying the chain with this migration withheld so the rows it meets
//! were written by the older schema.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_catalog_version_ref
        ADD COLUMN subject_effective_to timestamptz",
    // The backfill this migration's own rule makes mandatory -- see the module
    // doc. Joined on the tenant as well as the id: `membership_id` is a `uuid`
    // primary key and unique on its own, but a cross-tenant read of a scoped
    // table is not a shape this gear writes even where it would be harmless.
    "UPDATE bss.pricing_catalog_version_ref AS vref
        SET subject_revision     = truth.row_version,
            subject_effective_to = truth.effective_to
        FROM bss.pricing_group_membership AS truth
        WHERE vref.subject_kind = 'group_membership'
          AND vref.subject_revision IS NULL
          AND truth.tenant_id = vref.tenant_id
          AND truth.membership_id::text = vref.subject_ref",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    // Before this migration no membership ref carried a revision at all, so
    // this restores that state exactly rather than guessing which values `up`
    // wrote. The column dropped below is where the other half lived.
    "UPDATE bss.pricing_catalog_version_ref
        SET subject_revision = NULL
        WHERE subject_kind = 'group_membership'",
    "ALTER TABLE bss.pricing_catalog_version_ref
        DROP COLUMN IF EXISTS subject_effective_to",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE pricing_catalog_version_ref
        ADD COLUMN subject_effective_to text",
    // The same backfill, spelled as correlated subqueries because `SQLite` has
    // no `UPDATE … FROM`.
    //
    // **`membership_id` is matched in hex, and that is not decoration.** On this
    // mirror a `Uuid` reaches a column through `sqlx`, which encodes it as a
    // 16-byte **blob** — and a blob is the one storage class `SQLite`'s column
    // affinity never converts, so the value sits in a `text`-declared column as
    // bytes while `subject_ref`, a Rust `String`, sits beside it as characters.
    // A plain `=` between them is always false and the backfill would have been
    // a silent no-op on every `SQLite` database. The first term keeps the
    // dashed-text spelling working too, which is what a raw fixture writes.
    //
    // `row_version` is `NOT NULL`, so its subquery is NULL exactly when no truth
    // row matches -- which is the "leave an orphaned ref alone" clause the
    // Postgres arm gets from its join, without a third copy of the predicate.
    "UPDATE pricing_catalog_version_ref
        SET subject_revision = (
                SELECT truth.row_version FROM pricing_group_membership AS truth
                 WHERE truth.tenant_id = pricing_catalog_version_ref.tenant_id
                   AND (truth.membership_id = pricing_catalog_version_ref.subject_ref
                        OR lower(hex(truth.membership_id))
                           = lower(replace(pricing_catalog_version_ref.subject_ref, '-', '')))),
            subject_effective_to = (
                SELECT truth.effective_to FROM pricing_group_membership AS truth
                 WHERE truth.tenant_id = pricing_catalog_version_ref.tenant_id
                   AND (truth.membership_id = pricing_catalog_version_ref.subject_ref
                        OR lower(hex(truth.membership_id))
                           = lower(replace(pricing_catalog_version_ref.subject_ref, '-', ''))))
        WHERE subject_kind = 'group_membership'
          AND subject_revision IS NULL
          AND (
                SELECT truth.row_version FROM pricing_group_membership AS truth
                 WHERE truth.tenant_id = pricing_catalog_version_ref.tenant_id
                   AND (truth.membership_id = pricing_catalog_version_ref.subject_ref
                        OR lower(hex(truth.membership_id))
                           = lower(replace(pricing_catalog_version_ref.subject_ref, '-', '')))
              ) IS NOT NULL",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "UPDATE pricing_catalog_version_ref
        SET subject_revision = NULL
        WHERE subject_kind = 'group_membership'",
    "ALTER TABLE pricing_catalog_version_ref
        DROP COLUMN subject_effective_to",
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
