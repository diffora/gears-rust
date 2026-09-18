//! Create `bss.pricing_draft_window` — revision-owned window intentions (D-374).
//!
//! These rows are **not** `pricing_price_window` and admit no four-state token.
//! They version with the plan revision: keyed
//! `(tenant_id, plan_id, plan_revision, operation_id)`, frozen when **that**
//! revision leaves `draft`. Abandoned revisions **keep** their rows for audit;
//! the DELETE trigger refuses a non-draft parent, so `PlanRepo::abandon_draft`
//! must not drop them.
//!
//! A `create` uses `operation_id = target_window_id` (the authored window id
//! that survives publish). Live `adjust_end` / `cancel` operations mint their
//! own operation id and name the live window as `target_window_id`. Unique per
//! target on one revision so composition never sees two ops on one window.
//!
//! Overlap is **not** a constraint here: it is judged against the composed
//! proposal, not across independent revisions or against live exclusion.
//!
//! Dependency: `pricing_plan`, `pricing_price`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.pricing_draft_window (
            tenant_id         uuid        NOT NULL,
            plan_id           uuid        NOT NULL,
            plan_revision     bigint      NOT NULL,
            operation_id      uuid        NOT NULL,
            target_window_id  uuid        NOT NULL,
            action            text        NOT NULL,
            price_id          uuid,
            start_kind        text,
            effective_from    timestamptz,
            effective_to      timestamptz,
            reason_code       text        NOT NULL,
            CONSTRAINT chk_pricing_draft_window_action CHECK (action IN ('create', 'adjust_end', 'cancel')),
            CONSTRAINT chk_pricing_draft_window_reason_code CHECK (length(btrim(reason_code)) > 0),
            CONSTRAINT chk_pricing_draft_window_interval CHECK (
                effective_from IS NULL OR effective_to IS NULL OR effective_to > effective_from
            ),
            CONSTRAINT chk_pricing_draft_window_shape CHECK (
                (action = 'create'
                    AND operation_id = target_window_id
                    AND price_id IS NOT NULL
                    AND start_kind IN ('at', 'at_publish')
                    AND ((start_kind = 'at' AND effective_from IS NOT NULL)
                        OR (start_kind = 'at_publish' AND effective_from IS NULL)))
                OR (action = 'adjust_end'
                    AND start_kind IS NULL
                    AND effective_from IS NULL
                    AND price_id IS NULL)
                OR (action = 'cancel'
                    AND start_kind IS NULL
                    AND effective_from IS NULL
                    AND effective_to IS NULL
                    AND price_id IS NULL)
            ),
            CONSTRAINT fk_pricing_draft_window_revision FOREIGN KEY (plan_id, plan_revision)
                REFERENCES bss.pricing_plan(plan_id, revision),
            CONSTRAINT fk_pricing_draft_window_price FOREIGN KEY (price_id)
                REFERENCES bss.pricing_price(price_id),
            CONSTRAINT uq_pricing_draft_window_target UNIQUE (tenant_id, plan_id, plan_revision, target_window_id),
            CONSTRAINT pricing_draft_window_pkey PRIMARY KEY (tenant_id, plan_id, plan_revision, operation_id)
        )",
    "CREATE INDEX idx_pricing_draft_window_revision ON bss.pricing_draft_window USING btree (tenant_id, plan_id, plan_revision)",
    "CREATE OR REPLACE FUNCTION bss.pricing_draft_window_append_only() RETURNS trigger AS $$
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
                'pricing_draft_window: % of a draft window under a % plan revision is not permitted',
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
              'pricing_draft_window: % of a draft window under a % plan revision is not permitted',
              TG_OP, coalesce(parent_state, 'missing');
          END IF;

          IF parent_tenant IS DISTINCT FROM NEW.tenant_id THEN
            RAISE EXCEPTION
              'pricing_draft_window: plan revision %/% belongs to another tenant and may not hold this row',
              NEW.plan_id, NEW.plan_revision;
          END IF;

          IF NEW.price_id IS NOT NULL THEN
            SELECT tenant_id, plan_id INTO price_tenant, price_plan
              FROM bss.pricing_price
             WHERE price_id = NEW.price_id;
            IF price_tenant IS DISTINCT FROM NEW.tenant_id OR price_plan IS DISTINCT FROM NEW.plan_id THEN
              RAISE EXCEPTION
                'pricing_draft_window: price % does not belong to plan % of this tenant',
                NEW.price_id, NEW.plan_id;
            END IF;
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_pricing_draft_window_append_only BEFORE INSERT OR DELETE OR UPDATE ON bss.pricing_draft_window FOR EACH ROW EXECUTE FUNCTION bss.pricing_draft_window_append_only()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.pricing_draft_window",
    "DROP FUNCTION IF EXISTS bss.pricing_draft_window_append_only()",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_draft_window (
            tenant_id         text    NOT NULL,
            plan_id           text    NOT NULL,
            plan_revision     bigint  NOT NULL,
            operation_id      text    NOT NULL,
            target_window_id  text    NOT NULL,
            action            text    NOT NULL,
            price_id          text,
            start_kind        text,
            effective_from    text,
            effective_to      text,
            reason_code       text    NOT NULL,
            PRIMARY KEY (tenant_id, plan_id, plan_revision, operation_id),
            CONSTRAINT chk_pricing_draft_window_action CHECK (action IN ('create', 'adjust_end', 'cancel')),
            CONSTRAINT chk_pricing_draft_window_reason_code CHECK (length(trim(reason_code)) > 0),
            CONSTRAINT chk_pricing_draft_window_interval CHECK (
                effective_from IS NULL OR effective_to IS NULL OR effective_to > effective_from
            ),
            CONSTRAINT chk_pricing_draft_window_shape CHECK (
                (action = 'create'
                    AND operation_id = target_window_id
                    AND price_id IS NOT NULL
                    AND start_kind IN ('at', 'at_publish')
                    AND ((start_kind = 'at' AND effective_from IS NOT NULL)
                        OR (start_kind = 'at_publish' AND effective_from IS NULL)))
                OR (action = 'adjust_end'
                    AND start_kind IS NULL
                    AND effective_from IS NULL
                    AND price_id IS NULL)
                OR (action = 'cancel'
                    AND start_kind IS NULL
                    AND effective_from IS NULL
                    AND effective_to IS NULL
                    AND price_id IS NULL)
            ),
            CONSTRAINT fk_pricing_draft_window_revision FOREIGN KEY (plan_id, plan_revision)
                REFERENCES pricing_plan(plan_id, revision),
            CONSTRAINT fk_pricing_draft_window_price FOREIGN KEY (price_id)
                REFERENCES pricing_price(price_id),
            CONSTRAINT uq_pricing_draft_window_target UNIQUE (tenant_id, plan_id, plan_revision, target_window_id)
        )",
    "CREATE INDEX idx_pricing_draft_window_revision ON pricing_draft_window (tenant_id, plan_id, plan_revision)",
    "CREATE TRIGGER trg_pricing_draft_window_no_delete BEFORE DELETE ON pricing_draft_window FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'pricing_draft_window: DELETE of a draft window under a non-draft plan revision is not permitted') WHERE NOT EXISTS (SELECT 1 FROM pricing_plan WHERE plan_id = OLD.plan_id AND revision = OLD.plan_revision AND lifecycle_state = 'draft'); END",
    "CREATE TRIGGER trg_pricing_draft_window_no_insert BEFORE INSERT ON pricing_draft_window FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'pricing_draft_window: INSERT of a draft window under a non-draft plan revision is not permitted') WHERE NOT EXISTS (SELECT 1 FROM pricing_plan WHERE plan_id = NEW.plan_id AND revision = NEW.plan_revision AND lifecycle_state = 'draft'); END",
    "CREATE TRIGGER trg_pricing_draft_window_no_update BEFORE UPDATE ON pricing_draft_window FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'pricing_draft_window: UPDATE of a draft window under a non-draft plan revision is not permitted') WHERE NOT EXISTS (SELECT 1 FROM pricing_plan WHERE plan_id = OLD.plan_id AND revision = OLD.plan_revision AND lifecycle_state = 'draft') OR NOT EXISTS (SELECT 1 FROM pricing_plan WHERE plan_id = NEW.plan_id AND revision = NEW.plan_revision AND lifecycle_state = 'draft'); END",
    "CREATE TRIGGER trg_pricing_draft_window_same_tenant_as_its_revision_on_insert BEFORE INSERT ON pricing_draft_window FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'pricing_draft_window: the plan revision belongs to another tenant and may not hold this row') WHERE EXISTS (SELECT 1 FROM pricing_plan WHERE plan_id = NEW.plan_id AND revision = NEW.plan_revision) AND NOT EXISTS (SELECT 1 FROM pricing_plan WHERE plan_id = NEW.plan_id AND revision = NEW.plan_revision AND tenant_id = NEW.tenant_id); END",
    "CREATE TRIGGER trg_pricing_draft_window_same_tenant_as_its_revision_on_update BEFORE UPDATE ON pricing_draft_window FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'pricing_draft_window: the plan revision belongs to another tenant and may not hold this row') WHERE EXISTS (SELECT 1 FROM pricing_plan WHERE plan_id = NEW.plan_id AND revision = NEW.plan_revision) AND NOT EXISTS (SELECT 1 FROM pricing_plan WHERE plan_id = NEW.plan_id AND revision = NEW.plan_revision AND tenant_id = NEW.tenant_id); END",
    "CREATE TRIGGER trg_pricing_draft_window_price_on_plan_insert BEFORE INSERT ON pricing_draft_window FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'pricing_draft_window: price does not belong to plan of this tenant') WHERE NEW.price_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM pricing_price WHERE price_id = NEW.price_id AND tenant_id = NEW.tenant_id AND plan_id = NEW.plan_id); END",
    "CREATE TRIGGER trg_pricing_draft_window_price_on_plan_update BEFORE UPDATE ON pricing_draft_window FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'pricing_draft_window: price does not belong to plan of this tenant') WHERE NEW.price_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM pricing_price WHERE price_id = NEW.price_id AND tenant_id = NEW.tenant_id AND plan_id = NEW.plan_id); END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_draft_window"];

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
