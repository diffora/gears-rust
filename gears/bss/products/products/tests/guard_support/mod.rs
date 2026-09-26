//! What the schema-guard suites of both tiers share (P-D-195): the refusal they expect, and the
//! `products_sku_reference` shape the phase 2 rename replaced in place (`64e694a9f^`,
//! `m20260925_000006`), copied verbatim so a test can rebuild it.

#![allow(dead_code, reason = "each test binary uses part of the module")]

/// The guard's migration name; it sorts before every other migration of the gear.
pub const GUARD: &str = "m0000_products_refuse_a_legacy_or_stale_schema";

/// Every table the legacy chain creates, counted independently of the guard's constant:
/// `git show bss/products-backup:gears/bss/products/products/src/infra/storage/migrations/*` has 82
/// `CREATE TABLE` statements (both dialects, every name a literal, no `RENAME TO`) naming these 40.
/// The legacy set the guard refuses is this list minus what a fresh chain creates (P-D-195, H1).
pub const LEGACY_CHAIN_TABLES: [&str; 40] = [
    "products_approval",
    "products_approval_decision",
    "products_attribute_definition",
    "products_attribute_value",
    "products_audit_log",
    "products_breakglass_session",
    "products_bulk_batch",
    "products_bulk_row",
    "products_catalog_version",
    "products_catalog_version_capture",
    "products_catalog_version_counter",
    "products_catalog_version_entry",
    "products_catalog_version_request",
    "products_category",
    "products_correction_override",
    "products_deferred_retirement",
    "products_entity_version",
    "products_freeze_ack",
    "products_freeze_participant",
    "products_idempotency",
    "products_identity_ref",
    "products_materiality_policy",
    "products_metadata",
    "products_pii_allowlist",
    "products_product",
    "products_product_category",
    "products_read_checkpoint",
    "products_read_deferred_intent",
    "products_read_delivery_state",
    "products_read_entity",
    "products_read_freeze_status",
    "products_read_inbox",
    "products_read_poison",
    "products_read_stamp",
    "products_recognized_set",
    "products_reference_member",
    "products_reference_producer",
    "products_reference_watermark",
    "products_scheduled_transition",
    "products_sku",
];

/// P-D-195's refusal, word for word.
#[must_use]
pub fn refusal(kind: &str, found: &str) -> String {
    format!(
        "bss-products: this database holds a {kind} bss-products schema ({found}); PriceBook \
         does not migrate it \u{2014} start from an empty data root / empty bss-products tables"
    )
}

/// What the guard reports for the pre-rename reference table.
pub const STALE_REFERENCE: &str =
    "products_sku_reference whose ref_kind CHECK does not admit price_book_entry";

pub const SKU_REFERENCE_BEFORE_RENAME_SQLITE: &[&str] = &[
    "CREATE TABLE products_sku_reference (\n  id text PRIMARY KEY, tenant_id text NOT NULL, sku_id text NOT NULL REFERENCES products_sku(id),\n  owner_gear text NOT NULL, ref_kind text NOT NULL CHECK (ref_kind IN ('price','plan_item','sold_as')), ref_id text NOT NULL,\n  state text NOT NULL CHECK (state IN ('reserved','confirmed','released')),\n  reserved_by text NOT NULL, reserved_at text NOT NULL, confirmed_at text NULL, released_at text NULL, released_by text NULL, release_reason text NULL, forced integer NOT NULL DEFAULT false)",
    "CREATE UNIQUE INDEX uq_products_sku_reference_live ON products_sku_reference (tenant_id, owner_gear, ref_kind, ref_id) WHERE state <> 'released'",
    "CREATE INDEX ix_products_sku_reference_live ON products_sku_reference (sku_id, state) WHERE state <> 'released'",
];
pub const SKU_REFERENCE_BEFORE_RENAME_PG: &[&str] = &[
    "CREATE TABLE bss.products_sku_reference (\n  id uuid PRIMARY KEY, tenant_id uuid NOT NULL, sku_id uuid NOT NULL REFERENCES bss.products_sku(id),\n  owner_gear text NOT NULL, ref_kind text NOT NULL CHECK (ref_kind IN ('price','plan_item','sold_as')), ref_id uuid NOT NULL,\n  state text NOT NULL CHECK (state IN ('reserved','confirmed','released')),\n  reserved_by uuid NOT NULL, reserved_at timestamptz NOT NULL, confirmed_at timestamptz NULL, released_at timestamptz NULL, released_by uuid NULL, release_reason text NULL, forced boolean NOT NULL DEFAULT false)",
    "CREATE UNIQUE INDEX uq_products_sku_reference_live ON bss.products_sku_reference USING btree (tenant_id, owner_gear, ref_kind, ref_id) WHERE state <> 'released'",
    "CREATE INDEX ix_products_sku_reference_live ON bss.products_sku_reference USING btree (sku_id, state) WHERE state <> 'released'",
];
