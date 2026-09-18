//! Create `bss.pricing_window_baseline` — captured live window **references**
//! for one plan revision (D-374).
//!
//! Unchanged entries stay references, never copies, at publish. Refresh is an
//! explicit draft mutation (Task 4); this table only stores the captured set.
//! Frozen with the owner revision the same way `pricing_draft_window` is.
//! Abandoned revisions keep their rows.
//!
//! This is not an implicit-window backfill and not a live `pricing_price_window`
//! row. `mutation_seq` is the operator act tag captured at open/refresh.
//!
//! Dependency: `pricing_plan`, `pricing_price`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.pricing_window_baseline (
            tenant_id       uuid        NOT NULL,
            plan_id         uuid        NOT NULL,
            plan_revision   bigint      NOT NULL,
            window_id       uuid        NOT NULL,
            price_id        uuid        NOT NULL,
            mutation_seq    bigint      NOT NULL,
            effective_from  timestamptz NOT NULL,
            effective_to    timestamptz,
            cancelled       boolean     NOT NULL,
            CONSTRAINT chk_pricing_window_baseline_mutation_seq CHECK (mutation_seq >= 0),
            CONSTRAINT chk_pricing_window_baseline_interval CHECK (
                effective_to IS NULL OR effective_to > effective_from
            ),
            CONSTRAINT fk_pricing_window_baseline_revision FOREIGN KEY (plan_id, plan_revision)
                REFERENCES bss.pricing_plan(plan_id, revision),
            CONSTRAINT fk_pricing_window_baseline_price FOREIGN KEY (price_id)
                REFERENCES bss.pricing_price(price_id),
            CONSTRAINT pricing_window_baseline_pkey PRIMARY KEY (tenant_id, plan_id, plan_revision, window_id)
        )",
    "CREATE INDEX idx_pricing_window_baseline_revision ON bss.pricing_window_baseline USING btree (tenant_id, plan_id, plan_revision)",
    "CREATE OR REPLACE FUNCTION bss.pricing_window_baseline_append_only() RETURNS trigger AS $$
        DECLARE
          parent_state  text;
          parent_tenant uuid;
          price_tenant  uuid;
          price_plan    uuid;
        BEGIN
          IF TG_OP <> 'INSERT' THEN
            SELECT lifecycle_state INTO parent_state
              FROM bss.pricing_plan
             WHERE plan_id = OLD.plan_id AND revision = OLD.plan_revision;
            IF parent_state IS DISTINCT FROM 'draft' THEN
              RAISE EXCEPTION
                'pricing_window_baseline: % of a baseline under a % plan revision is not permitted',
                TG_OP, coalesce(parent_state, 'missing');
            END IF;
          END IF;

          IF TG_OP = 'DELETE' THEN
            RETURN OLD;
          END IF;

          SELECT lifecycle_state, tenant_id INTO parent_state, parent_tenant
            FROM bss.pricing_plan
           WHERE plan_id = NEW.plan_id AND revision = NEW.plan_revision;
          IF parent_state IS DISTINCT FROM 'draft' THEN
            RAISE EXCEPTION
              'pricing_window_baseline: % of a baseline under a % plan revision is not permitted',
              TG_OP, coalesce(parent_state, 'missing');
          END IF;

          IF parent_tenant IS DISTINCT FROM NEW.tenant_id THEN
            RAISE EXCEPTION
              'pricing_window_baseline: plan revision %/% belongs to another tenant and may not hold this row',
              NEW.plan_id, NEW.plan_revision;
          END IF;

          SELECT tenant_id, plan_id INTO price_tenant, price_plan
            FROM bss.pricing_price
           WHERE price_id = NEW.price_id;
          IF price_tenant IS DISTINCT FROM NEW.tenant_id OR price_plan IS DISTINCT FROM NEW.plan_id THEN
            RAISE EXCEPTION
              'pricing_window_baseline: price % does not belong to plan % of this tenant',
              NEW.price_id, NEW.plan_id;
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_pricing_window_baseline_append_only BEFORE INSERT OR DELETE OR UPDATE ON bss.pricing_window_baseline FOR EACH ROW EXECUTE FUNCTION bss.pricing_window_baseline_append_only()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.pricing_window_baseline",
    "DROP FUNCTION IF EXISTS bss.pricing_window_baseline_append_only()",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_window_baseline (
            tenant_id       text    NOT NULL,
            plan_id         text    NOT NULL,
            plan_revision   bigint  NOT NULL,
            window_id       text    NOT NULL,
            price_id        text    NOT NULL,
            mutation_seq    bigint  NOT NULL,
            effective_from  text    NOT NULL,
            effective_to    text,
            cancelled       boolean NOT NULL,
            PRIMARY KEY (tenant_id, plan_id, plan_revision, window_id),
            CONSTRAINT chk_pricing_window_baseline_mutation_seq CHECK (mutation_seq >= 0),
            CONSTRAINT chk_pricing_window_baseline_interval CHECK (
                effective_to IS NULL OR effective_to > effective_from
            ),
            CONSTRAINT fk_pricing_window_baseline_revision FOREIGN KEY (plan_id, plan_revision)
                REFERENCES pricing_plan(plan_id, revision),
            CONSTRAINT fk_pricing_window_baseline_price FOREIGN KEY (price_id)
                REFERENCES pricing_price(price_id)
        )",
    "CREATE INDEX idx_pricing_window_baseline_revision ON pricing_window_baseline (tenant_id, plan_id, plan_revision)",
    "CREATE TRIGGER trg_pricing_window_baseline_no_delete BEFORE DELETE ON pricing_window_baseline FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'pricing_window_baseline: DELETE of a baseline under a non-draft plan revision is not permitted') WHERE NOT EXISTS (SELECT 1 FROM pricing_plan WHERE plan_id = OLD.plan_id AND revision = OLD.plan_revision AND lifecycle_state = 'draft'); END",
    "CREATE TRIGGER trg_pricing_window_baseline_no_insert BEFORE INSERT ON pricing_window_baseline FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'pricing_window_baseline: INSERT of a baseline under a non-draft plan revision is not permitted') WHERE NOT EXISTS (SELECT 1 FROM pricing_plan WHERE plan_id = NEW.plan_id AND revision = NEW.plan_revision AND lifecycle_state = 'draft'); END",
    "CREATE TRIGGER trg_pricing_window_baseline_no_update BEFORE UPDATE ON pricing_window_baseline FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'pricing_window_baseline: UPDATE of a baseline under a non-draft plan revision is not permitted') WHERE NOT EXISTS (SELECT 1 FROM pricing_plan WHERE plan_id = OLD.plan_id AND revision = OLD.plan_revision AND lifecycle_state = 'draft') OR NOT EXISTS (SELECT 1 FROM pricing_plan WHERE plan_id = NEW.plan_id AND revision = NEW.plan_revision AND lifecycle_state = 'draft'); END",
    "CREATE TRIGGER trg_pricing_window_baseline_same_tenant_as_its_revision_on_insert BEFORE INSERT ON pricing_window_baseline FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'pricing_window_baseline: the plan revision belongs to another tenant and may not hold this row') WHERE EXISTS (SELECT 1 FROM pricing_plan WHERE plan_id = NEW.plan_id AND revision = NEW.plan_revision) AND NOT EXISTS (SELECT 1 FROM pricing_plan WHERE plan_id = NEW.plan_id AND revision = NEW.plan_revision AND tenant_id = NEW.tenant_id); END",
    "CREATE TRIGGER trg_pricing_window_baseline_same_tenant_as_its_revision_on_update BEFORE UPDATE ON pricing_window_baseline FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'pricing_window_baseline: the plan revision belongs to another tenant and may not hold this row') WHERE EXISTS (SELECT 1 FROM pricing_plan WHERE plan_id = NEW.plan_id AND revision = NEW.plan_revision) AND NOT EXISTS (SELECT 1 FROM pricing_plan WHERE plan_id = NEW.plan_id AND revision = NEW.plan_revision AND tenant_id = NEW.tenant_id); END",
    "CREATE TRIGGER trg_pricing_window_baseline_price_on_plan_insert BEFORE INSERT ON pricing_window_baseline FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'pricing_window_baseline: price does not belong to plan of this tenant') WHERE NOT EXISTS (SELECT 1 FROM pricing_price WHERE price_id = NEW.price_id AND tenant_id = NEW.tenant_id AND plan_id = NEW.plan_id); END",
    "CREATE TRIGGER trg_pricing_window_baseline_price_on_plan_update BEFORE UPDATE ON pricing_window_baseline FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'pricing_window_baseline: price does not belong to plan of this tenant') WHERE NOT EXISTS (SELECT 1 FROM pricing_price WHERE price_id = NEW.price_id AND tenant_id = NEW.tenant_id AND plan_id = NEW.plan_id); END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_window_baseline"];

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
