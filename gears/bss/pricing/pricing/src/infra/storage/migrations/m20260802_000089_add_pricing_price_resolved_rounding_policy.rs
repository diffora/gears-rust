//! `pricing_price.resolved_rounding_policy` — the rounding policy a publish
//! actually resolved, frozen beside `resolved_tax_category` and kept **apart
//! from the authored `rounding_policy_ref`**.
//!
//! # Why a second column and not the one that was already there
//!
//! Freezing the resolution is right, and its reason is on
//! `price_repo::publish_rows`: a charge replayed from a pinned `CatalogVersion`
//! must round the way it rounded when the version was cut, and a tenant who later
//! flips `default_rounding_policy_ref` must not silently re-round every already
//! frozen version.
//!
//! The publish wrote that resolution **over `rounding_policy_ref` itself**, which
//! is the column the author sets. The two facts then became one column and the
//! authored one was the one lost:
//!
//! - a row that deliberately carried no policy of its own — leaning on the
//!   tenant default, which is a live setting — came back from publish indelibly
//!   naming a value, so the default stopped reaching it forever after;
//! - `price_repo::authored_content` and `infra::supersession` carry that value
//!   into every successor and copy draft as if a person had typed it, so the
//!   conflation propagates rather than staying on the row that published;
//! - and the publish-time vocabulary rule (`RoundingPolicyDeclared`) judges the
//!   tenant default only for rows that carry none of their own, so after one
//!   publish there is nothing left for it to judge.
//!
//! `resolved_tax_category` had this shape right from `m20260802_000039` — the
//! authored `tax_category_ref` stays put and the resolution lands beside it. This
//! is that column's twin, three findings later.
//!
//! Nullable, like its sibling: `NULL` is "this row resolved no policy", which for
//! a published row cannot happen (`RoundingPolicyResolved` refuses the publish)
//! and for a draft is simply the truth.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] =
    &["ALTER TABLE bss.pricing_price ADD COLUMN resolved_rounding_policy text"];

const PG_DOWN_STATEMENTS: &[&str] =
    &["ALTER TABLE bss.pricing_price DROP COLUMN resolved_rounding_policy"];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------

const SQLITE_UP_STATEMENTS: &[&str] =
    &["ALTER TABLE pricing_price ADD COLUMN resolved_rounding_policy text"];

const SQLITE_DOWN_STATEMENTS: &[&str] =
    &["ALTER TABLE pricing_price DROP COLUMN resolved_rounding_policy"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
