//! D-373: descriptors follow invoice-line grain. The shipped chain is unchanged.
//!
//! Prices are not revision-scoped. A plan's historical descriptor pairs must
//! therefore agree before a value can be assigned to its rows. Conflicting
//! histories require an explicit operator mapping; choosing the latest is unsafe.
//! Both engines run the preflight, backfill and guard replacement transactionally.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use sea_orm::{ConnectionTrait, DbBackend, Statement};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PRICE_FIELDS: [&str; 4] = [
    "invoice_line_template",
    "gl_code_ref",
    "resolved_invoice_line_template",
    "resolved_gl_code",
];
const DEFAULT_TEMPLATES: &str = r#"{"recurring":"{sku} - {period}","usage":"{sku}, {unit}","one_time":"{sku}"}"#;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    fn use_transaction(&self) -> Option<bool> {
        Some(true)
    }

    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        preflight(manager).await?;
        let guards = take_guards(manager).await?;
        let db = manager.get_connection();
        let prefix = prefix(manager);
        for field in PRICE_FIELDS {
            db.execute_unprepared(&format!(
                "ALTER TABLE {prefix}pricing_price ADD COLUMN {field} text"
            ))
            .await?;
        }
        let json_type = if manager.get_database_backend() == DbBackend::Postgres {
            "jsonb"
        } else {
            "text"
        };
        db.execute_unprepared(&format!(
            "ALTER TABLE {prefix}pricing_plan ADD COLUMN descriptor_ext {json_type} NOT NULL DEFAULT '{{}}'"
        )).await?;
        db.execute_unprepared(&format!(
            "ALTER TABLE {prefix}pricing_policy_object ADD COLUMN default_gl_code_ref text"
        ))
        .await?;
        let check = if manager.get_database_backend() == DbBackend::Postgres {
            "jsonb_typeof(default_line_templates) = 'object' AND default_line_templates - ARRAY['recurring','usage','one_time'] = '{}'::jsonb AND COALESCE(jsonb_typeof(default_line_templates->'recurring') = 'string', false) AND COALESCE(jsonb_typeof(default_line_templates->'usage') = 'string', false) AND COALESCE(jsonb_typeof(default_line_templates->'one_time') = 'string', false)"
        } else {
            "json_valid(default_line_templates) AND json_type(default_line_templates) = 'object' AND json_remove(default_line_templates, '$.recurring', '$.usage', '$.one_time') = '{}' AND COALESCE(json_type(default_line_templates, '$.recurring') = 'text', 0) AND COALESCE(json_type(default_line_templates, '$.usage') = 'text', 0) AND COALESCE(json_type(default_line_templates, '$.one_time') = 'text', 0)"
        };
        db.execute_unprepared(&format!(
            "ALTER TABLE {prefix}pricing_policy_object ADD COLUMN default_line_templates {json_type} NOT NULL DEFAULT '{DEFAULT_TEMPLATES}' CONSTRAINT chk_pricing_policy_line_templates CHECK ({check})"
        )).await?;

        db.execute_unprepared(&format!(
            "UPDATE {prefix}pricing_plan SET descriptor_ext = COALESCE((SELECT d.additional_fields FROM {prefix}pricing_plan_descriptor_set d WHERE d.tenant_id = {prefix}pricing_plan.tenant_id AND d.plan_id = {prefix}pricing_plan.plan_id AND d.plan_revision = {prefix}pricing_plan.revision), '{{}}')"
        )).await?;
        let source = |field: &str| {
            format!(
                "(SELECT d.{field} FROM {prefix}pricing_plan pl LEFT JOIN {prefix}pricing_plan_descriptor_set d ON d.tenant_id = pl.tenant_id AND d.plan_id = pl.plan_id AND d.plan_revision = pl.revision WHERE pl.tenant_id = {prefix}pricing_price.tenant_id AND pl.plan_id = {prefix}pricing_price.plan_id ORDER BY pl.revision LIMIT 1)"
            )
        };
        db.execute_unprepared(&format!(
            "UPDATE {prefix}pricing_price SET invoice_line_template = {template}, gl_code_ref = {gl}, resolved_invoice_line_template = CASE WHEN lifecycle_state <> 'draft' THEN {template} ELSE NULL END, resolved_gl_code = CASE WHEN lifecycle_state <> 'draft' THEN {gl} ELSE NULL END",
            template = source("invoice_line_template"), gl = source("gl_code")
        )).await?;
        verify_backfill(manager).await?;
        db.execute_unprepared(&format!("DROP TABLE {prefix}pricing_plan_descriptor_set"))
            .await?;
        if manager.get_database_backend() == DbBackend::Postgres {
            db.execute_unprepared("DROP FUNCTION bss.pricing_plan_descriptor_set_append_only()")
                .await?;
            db.execute_unprepared("ALTER TABLE bss.pricing_plan DROP COLUMN invoice_grouping_key")
                .await?;
        } else {
            rebuild_sqlite(manager, true).await?;
        }
        restore_guards(manager, guards, true).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let guards = take_guards(manager).await?;
        let db = manager.get_connection();
        if manager.get_database_backend() == DbBackend::Postgres {
            for field in PRICE_FIELDS {
                db.execute_unprepared(&format!(
                    "ALTER TABLE bss.pricing_price DROP COLUMN {field}"
                ))
                .await?;
            }
            db.execute_unprepared("ALTER TABLE bss.pricing_plan DROP COLUMN descriptor_ext, ADD COLUMN invoice_grouping_key text").await?;
            db.execute_unprepared("ALTER TABLE bss.pricing_policy_object DROP COLUMN default_gl_code_ref, DROP COLUMN default_line_templates").await?;
        } else {
            rebuild_sqlite(manager, false).await?;
        }
        restore_guards(manager, guards, false).await?;
        // Rollback restores the old shape, never fabricates a descriptor set.
        super::m20260821_000034_create_pricing_plan_descriptor_set::Migration
            .up(manager)
            .await
    }
}

fn prefix(manager: &SchemaManager<'_>) -> &'static str {
    if manager.get_database_backend() == DbBackend::Postgres {
        "bss."
    } else {
        ""
    }
}

type Pair = (Option<String>, Option<String>);

async fn preflight(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let p = prefix(manager);
    let rows = manager.get_connection().query_all_raw(Statement::from_string(
        manager.get_database_backend(), format!(
            "SELECT CAST(pl.tenant_id AS text) AS tenant, CAST(pl.plan_id AS text) AS plan, pl.revision, d.invoice_line_template, d.gl_code FROM {p}pricing_plan pl LEFT JOIN {p}pricing_plan_descriptor_set d ON d.tenant_id = pl.tenant_id AND d.plan_id = pl.plan_id AND d.plan_revision = pl.revision WHERE EXISTS (SELECT 1 FROM {p}pricing_price r WHERE r.tenant_id = pl.tenant_id AND r.plan_id = pl.plan_id) ORDER BY pl.tenant_id, pl.plan_id, pl.revision"
        ))).await?;
    let mut histories: BTreeMap<(String, String), (Pair, i64)> = BTreeMap::new();
    for row in rows {
        let key = (
            row.try_get::<String>("", "tenant")?,
            row.try_get::<String>("", "plan")?,
        );
        let revision = row.try_get::<i64>("", "revision")?;
        let pair = (
            row.try_get::<Option<String>>("", "invoice_line_template")?,
            row.try_get::<Option<String>>("", "gl_code")?,
        );
        if let Some((previous, previous_revision)) = histories.get(&key) {
            if previous != &pair {
                return Err(DbErr::Custom(format!(
                    "D-373: ambiguous descriptors for tenant {} plan {} revisions {previous_revision} and {revision}; prices are not revision-scoped",
                    key.0, key.1
                )));
            }
        } else {
            histories.insert(key, (pair, revision));
        }
    }
    let missing = count(manager, &format!(
        "SELECT COUNT(*) AS n FROM {p}pricing_price r WHERE r.lifecycle_state <> 'draft' AND NOT EXISTS (SELECT 1 FROM {p}pricing_plan_descriptor_set d WHERE d.tenant_id = r.tenant_id AND d.plan_id = r.plan_id AND trim(COALESCE(d.invoice_line_template, '')) <> '' AND trim(COALESCE(d.gl_code, '')) <> '')"
    )).await?;
    if missing != 0 {
        return Err(DbErr::Custom(format!(
            "D-373: {missing} previously published price rows have no complete legacy descriptors"
        )));
    }
    Ok(())
}

async fn count(manager: &SchemaManager<'_>, sql: &str) -> Result<i64, DbErr> {
    manager
        .get_connection()
        .query_one_raw(Statement::from_string(manager.get_database_backend(), sql))
        .await?
        .ok_or_else(|| DbErr::Custom("D-373: count query returned no row".into()))?
        .try_get("", "n")
}

async fn verify_backfill(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let p = prefix(manager);
    let eq = if manager.get_database_backend() == DbBackend::Postgres {
        "IS NOT DISTINCT FROM"
    } else {
        "IS"
    };
    let expected = count(manager, &format!("SELECT COUNT(*) AS n FROM {p}pricing_price r WHERE EXISTS (SELECT 1 FROM {p}pricing_plan_descriptor_set d WHERE d.tenant_id = r.tenant_id AND d.plan_id = r.plan_id)")).await?;
    let actual = count(manager, &format!("SELECT COUNT(*) AS n FROM {p}pricing_price r WHERE EXISTS (SELECT 1 FROM {p}pricing_plan_descriptor_set d WHERE d.tenant_id = r.tenant_id AND d.plan_id = r.plan_id AND r.invoice_line_template {eq} d.invoice_line_template AND r.gl_code_ref {eq} d.gl_code AND (r.lifecycle_state = 'draft' OR (r.resolved_invoice_line_template {eq} d.invoice_line_template AND r.resolved_gl_code {eq} d.gl_code)))")).await?;
    if actual != expected {
        return Err(DbErr::Custom(format!(
            "D-373: descriptor backfill expected {expected} rows, filled {actual}"
        )));
    }
    Ok(())
}

/// Capture the exact installed guard bodies so unrelated guard changes survive.
async fn take_guards(manager: &SchemaManager<'_>) -> Result<Vec<String>, DbErr> {
    let db = manager.get_connection();
    if manager.get_database_backend() == DbBackend::Postgres {
        let mut guards = Vec::new();
        for table in ["pricing_plan", "pricing_price"] {
            let row = db.query_one_raw(Statement::from_string(DbBackend::Postgres,
                format!("SELECT pg_get_functiondef('bss.{table}_append_only()'::regprocedure) AS sql")))
                .await?.ok_or_else(|| DbErr::Custom(format!("D-373: missing {table} guard")))?;
            guards.push(row.try_get("", "sql")?);
            db.execute_unprepared(&format!(
                "ALTER TABLE bss.{table} DISABLE TRIGGER trg_{table}_append_only"
            ))
            .await?;
        }
        return Ok(guards);
    }
    let rows = db.query_all_raw(Statement::from_string(DbBackend::Sqlite,
        "SELECT name, sql FROM sqlite_master WHERE type = 'trigger' AND tbl_name IN ('pricing_plan','pricing_price') ORDER BY name")).await?;
    let mut guards = Vec::new();
    for row in rows {
        let name: String = row.try_get("", "name")?;
        guards.push(row.try_get("", "sql")?);
        db.execute_unprepared(&format!("DROP TRIGGER {}", quote(&name)))
            .await?;
    }
    Ok(guards)
}

async fn restore_guards(
    manager: &SchemaManager<'_>,
    guards: Vec<String>,
    up: bool,
) -> Result<(), DbErr> {
    let pg = manager.get_database_backend() == DbBackend::Postgres;
    let op = if pg { "IS DISTINCT FROM" } else { "IS NOT" };
    for mut sql in guards {
        if up {
            sql = sql
                .replace("NEW.invoice_grouping_key", "NEW.descriptor_ext")
                .replace("OLD.invoice_grouping_key", "OLD.descriptor_ext");
            if sql.contains("pricing_price") && sql.contains("OR NEW.currency") {
                let mut fields = String::new();
                for field in PRICE_FIELDS {
                    write!(fields, "OR NEW.{field} {op} OLD.{field} ")
                        .map_err(|e| DbErr::Custom(format!("format descriptor guard: {e}")))?;
                }
                sql = sql.replacen("OR NEW.currency", &format!("{fields}OR NEW.currency"), 1);
            }
        } else {
            sql = sql
                .replace("NEW.descriptor_ext", "NEW.invoice_grouping_key")
                .replace("OLD.descriptor_ext", "OLD.invoice_grouping_key");
            for field in PRICE_FIELDS {
                sql = sql.replace(&format!("OR NEW.{field} {op} OLD.{field} "), "");
            }
        }
        manager.get_connection().execute_unprepared(&sql).await?;
    }
    if pg {
        for table in ["pricing_plan", "pricing_price"] {
            manager
                .get_connection()
                .execute_unprepared(&format!(
                    "ALTER TABLE bss.{table} ENABLE TRIGGER trg_{table}_append_only"
                ))
                .await?;
        }
    }
    Ok(())
}

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Split CREATE TABLE members at top-level commas, respecting quoted defaults.
fn declarations(sql: &str) -> Result<(&str, Vec<String>, &str), DbErr> {
    let first = sql
        .find('(')
        .ok_or_else(|| DbErr::Custom("D-373: table DDL has no column list".into()))?;
    let last = sql
        .rfind(')')
        .ok_or_else(|| DbErr::Custom("D-373: table DDL has no closing parenthesis".into()))?;
    let content = &sql[first + 1..last];
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0;
    let mut quoted = None;
    let mut chars = content.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if let Some(q) = quoted {
            if c == q {
                if chars.peek().is_some_and(|(_, next)| *next == q) {
                    chars.next();
                } else {
                    quoted = None;
                }
            }
        } else {
            match c {
                '\'' | '"' | '`' => quoted = Some(c),
                '(' => depth += 1,
                ')' => depth -= 1,
                ',' if depth == 0 => {
                    parts.push(content[start..i].to_owned());
                    start = i + 1;
                }
                _ => {}
            }
        }
    }
    parts.push(content[start..].to_owned());
    Ok((&sql[..=first], parts, &sql[last..]))
}

fn removed(table: &str, column: &str, up: bool) -> bool {
    if up {
        return table == "pricing_plan" && column == "invoice_grouping_key";
    }
    (table == "pricing_plan" && column == "descriptor_ext")
        || (table == "pricing_price" && PRICE_FIELDS.contains(&column))
        || (table == "pricing_policy_object"
            && ["default_gl_code_ref", "default_line_templates"].contains(&column))
}

/// Rebuild roots and all FK descendants without disabling foreign keys.
#[allow(
    clippy::cognitive_complexity,
    reason = "ordered FK closure rebuild keeps backup, child-first drop, parent-first restore and validation in one transaction"
)]
async fn rebuild_sqlite(manager: &SchemaManager<'_>, up: bool) -> Result<(), DbErr> {
    let db = manager.get_connection();
    let rows = db.query_all_raw(Statement::from_string(DbBackend::Sqlite,
        "SELECT name, sql FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")).await?;
    let mut schemas = BTreeMap::new();
    let mut parents = BTreeMap::new();
    for row in rows {
        let name: String = row.try_get("", "name")?;
        let fks = db
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                format!("PRAGMA foreign_key_list({})", quote(&name)),
            ))
            .await?;
        parents.insert(
            name.clone(),
            fks.iter()
                .map(|fk| fk.try_get::<String>("", "table"))
                .collect::<Result<BTreeSet<_>, _>>()?,
        );
        schemas.insert(name, row.try_get::<String>("", "sql")?);
    }
    let mut selected = BTreeSet::from(["pricing_plan".to_owned()]);
    if !up {
        selected.extend([
            "pricing_price".to_owned(),
            "pricing_policy_object".to_owned(),
        ]);
    }
    loop {
        let before = selected.len();
        for (table, refs) in &parents {
            if !refs.is_disjoint(&selected) {
                selected.insert(table.clone());
            }
        }
        if selected.len() == before {
            break;
        }
    }
    let mut remaining = selected.clone();
    let mut ordered = Vec::new();
    while !remaining.is_empty() {
        let table = remaining
            .iter()
            .find(|t| parents[*t].is_disjoint(&remaining))
            .cloned()
            .ok_or_else(|| {
                DbErr::Custom("D-373: cyclic SQLite foreign keys cannot be rebuilt".into())
            })?;
        remaining.remove(&table);
        ordered.push(table);
    }
    let objects = db.query_all_raw(Statement::from_string(DbBackend::Sqlite,
        "SELECT type, tbl_name, sql FROM sqlite_master WHERE type IN ('index','trigger') AND sql IS NOT NULL ORDER BY type, name")).await?;
    let mut indexes = Vec::new();
    let mut triggers = Vec::new();
    for row in objects {
        if !selected.contains(&row.try_get::<String>("", "tbl_name")?) {
            continue;
        }
        let sql = row.try_get::<String>("", "sql")?;
        if row.try_get::<String>("", "type")? == "trigger" {
            triggers.push(sql);
        } else {
            indexes.push(sql);
        }
    }
    let mut columns = BTreeMap::new();
    for table in &ordered {
        let info = db
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                format!("PRAGMA table_info({})", quote(table)),
            ))
            .await?;
        let names = info
            .iter()
            .map(|r| r.try_get::<String>("", "name"))
            .collect::<Result<Vec<_>, _>>()?;
        columns.insert(
            table.clone(),
            names
                .into_iter()
                .filter(|c| !removed(table, c, up))
                .map(|c| quote(&c))
                .collect::<Vec<_>>()
                .join(","),
        );
        db.execute_unprepared(&format!(
            "CREATE TEMP TABLE {} AS SELECT * FROM {}",
            quote(&format!("d373_backup_{table}")),
            quote(table)
        ))
        .await?;
    }
    for table in ordered.iter().rev() {
        db.execute_unprepared(&format!("DROP TABLE {}", quote(table)))
            .await?;
    }
    for table in &ordered {
        let (head, parts, tail) = declarations(&schemas[table])?;
        let parts = parts
            .into_iter()
            .filter_map(|part| {
                let column = part
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim_matches('"');
                if !up && table == "pricing_plan" && column == "descriptor_ext" {
                    Some("invoice_grouping_key text".to_owned())
                } else if removed(table, column, up) {
                    None
                } else {
                    Some(part)
                }
            })
            .collect::<Vec<_>>();
        db.execute_unprepared(&format!("{head}{}{tail}", parts.join(",")))
            .await?;
    }
    for sql in indexes {
        db.execute_unprepared(&sql).await?;
    }
    for table in &ordered {
        let cols = &columns[table];
        db.execute_unprepared(&format!(
            "INSERT INTO {} ({cols}) SELECT {cols} FROM {}",
            quote(table),
            quote(&format!("d373_backup_{table}"))
        ))
        .await?;
    }
    for sql in triggers {
        db.execute_unprepared(&sql).await?;
    }
    for table in &ordered {
        db.execute_unprepared(&format!(
            "DROP TABLE {}",
            quote(&format!("d373_backup_{table}"))
        ))
        .await?;
    }
    let violations = db
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            "PRAGMA foreign_key_check",
        ))
        .await?;
    if !violations.is_empty() {
        return Err(DbErr::Custom(format!(
            "D-373: {} foreign-key violations after rebuild",
            violations.len()
        )));
    }
    Ok(())
}
