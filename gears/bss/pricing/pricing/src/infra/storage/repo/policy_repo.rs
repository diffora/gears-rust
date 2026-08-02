//! Repository over `pricing_policy_object` — the per-tenant policy store
//! (`design/01-foundation.md` §3.7).
//!
//! **One repository over the table, and one method on it — that is the
//! narrowing this file is.** The row carries four more policies, and each is
//! read on a different path at a different moment: the approval threshold at
//! §4.2 step 3, the tax-display mode in Slice 4, the default rounding policy at
//! the publish freeze, the notice period when Slice 11 schedules an enforced
//! migration. None of those paths exists yet, each wants a different value shape,
//! and each carries its own reading of absence. A method returning the whole row
//! today would answer four questions nobody is asking and would fix one shape for
//! four readers that have not been written — so the row's other half gets its
//! methods **here**, beside this one, when its slices land. A second repository
//! over one table is what would be wrong; a second method is not.
//!
//! What this one reads is exactly what the **authoring** path resolves: the four
//! §14 caps and the descriptor required-set extension D-152 put in this table.
//!
//! **Read-only, deliberately.** There is no upsert here because a policy change
//! is not a row write: D-10/D-13 route policy changes through the same approval
//! workflow as a publish, so a repository that let a caller set a cap directly
//! would be that workflow's write without its approval — and the caps are
//! exactly the values an approval exists to hold, since one UPDATE can make
//! every plan of a tenant unpublishable. The policy-administration path owns the
//! write and does not exist yet.
//!
//! # The defect this closes, and why it is not a cache
//!
//! Four ratified numbers and the descriptor required-set extension were called
//! **tenant-configurable** by a ratified NFR and a pinned assumption while the
//! only carrier the code had was the gear's configuration section, which is per
//! **deployment**. Every tenant of a deployment shared one cap, and the
//! descriptor required-set had no declaration surface at all — so
//! `DESCRIPTOR_INCOMPLETE` could only ever check D-48's pinned three and "config
//! extensible without a schema change" described a capability with no
//! configuration to exercise it (D-152).
//!
//! [`PolicyObjectRepo::authoring_policy`] is on the **authoring** path — the
//! §4.2 step-2 pre-check and the publish commit's re-validation — and never on a
//! resolution path. It is a point read on the primary key, once per validation
//! run, so nothing here is memoized: a cached cap is a cap that keeps rejecting
//! plans after an operator has raised it, and the two runs of one publish would
//! be free to disagree about the limit they enforced.
//!
//! # The carrier is provisional
//!
//! These settings read from a pricing table today because there is no settings
//! gear in this repository to read them from; D-152's confirmation records that
//! they are expected to move to one when it exists, and that
//! `gears/simple-user-settings` is not it (per **user**, so a tenant-wide cap
//! has no row to occupy). Build against this repository, and expect the move.

use std::collections::BTreeSet;

use sea_orm::{ColumnTrait, Condition, EntityTrait, JsonValue};
use toolkit_db::secure::{AccessScope, SecureEntityExt};
use toolkit_db::{DBProvider, DbError};
use uuid::Uuid;

use crate::config::LimitsConfig;
use crate::domain::plan_rules::{CustomIntervalBounds, DescriptorSetComplete};
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::policy_object;

/// One tenant's authoring-time configuration, already resolved against the
/// deployment defaults.
///
/// Resolved on the way out of storage rather than at the call sites, because a
/// caller holding "the tenant's value or else the deployment's" is a caller free
/// to forget the second half — and the half it would forget is the one that
/// keeps the ratified launch numbers from moving for every tenant that has
/// configured nothing.
///
/// Not a domain type: it is built from a row and from
/// [`LimitsConfig`](crate::config::LimitsConfig), both of which are
/// infrastructure. What it hands the domain is the two configured rules, already
/// constructed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthoringPolicy {
    max_tier_bands_per_row: u32,
    max_price_rows_per_plan: u32,
    max_custom_interval_days: u32,
    max_custom_interval_months: u32,
    additional_required_descriptors: Vec<String>,
}

impl AuthoringPolicy {
    /// The deployment's ratified launch values, which is what a tenant with no
    /// policy row is governed by.
    ///
    /// The descriptor extension is empty here and has no configuration field of
    /// its own: a per-**deployment** extension would let one deployment's
    /// tenants share a Billing contract they do not share, which is the half of
    /// D-152 that carries the sharpest cost. The absent extension is the pinned
    /// v1 three, and nothing else can produce them.
    #[must_use]
    pub fn from_deployment_defaults(limits: &LimitsConfig) -> Self {
        Self {
            max_tier_bands_per_row: limits.max_tier_bands_per_row,
            max_price_rows_per_plan: limits.max_price_rows_per_plan,
            max_custom_interval_days: limits.max_custom_interval_days,
            max_custom_interval_months: limits.max_custom_interval_months,
            additional_required_descriptors: Vec::new(),
        }
    }

    /// The `customEveryN` rule bound to this tenant's caps.
    #[must_use]
    pub const fn interval_bounds(&self) -> CustomIntervalBounds {
        CustomIntervalBounds::new(
            self.max_custom_interval_days,
            self.max_custom_interval_months,
        )
    }

    /// The descriptor rule bound to this tenant's extension of D-48 v1.
    #[must_use]
    pub fn descriptor_rule(&self) -> DescriptorSetComplete {
        DescriptorSetComplete::extending_v1(self.additional_required_descriptors.clone())
    }

    /// The tier-band soft cap in force for this tenant.
    ///
    /// No rule in this crate reads it yet: `nfr-size-limits` makes both soft
    /// caps a **SHOULD** that emits a publish warning, and §5 registers no code
    /// for either, so minting one here would put a discriminator on the wire
    /// that no document defines. The value is carried because the carrier is
    /// what D-152 decided — a cap whose per-tenant value is unreadable is the
    /// same silence one layer down.
    #[must_use]
    pub const fn max_tier_bands_per_row(&self) -> u32 {
        self.max_tier_bands_per_row
    }

    /// The price-row soft cap in force for this tenant; see
    /// [`AuthoringPolicy::max_tier_bands_per_row`] for why nothing reads it yet.
    #[must_use]
    pub const fn max_price_rows_per_plan(&self) -> u32 {
        self.max_price_rows_per_plan
    }

    /// The descriptor keys this tenant requires beyond D-48 v1's three, in the
    /// order the report will name them.
    #[must_use]
    pub fn additional_required_descriptors(&self) -> &[String] {
        &self.additional_required_descriptors
    }
}

/// `SeaORM`-backed reader of the per-tenant policy object.
#[derive(Clone)]
pub struct PolicyObjectRepo {
    db: DBProvider<DbError>,
    /// What a tenant with no policy row is governed by. Held here rather than
    /// taken per call so that no caller can read a cap without the default
    /// behind it.
    defaults: AuthoringPolicy,
}

impl PolicyObjectRepo {
    /// Build over one database provider and the deployment's ratified defaults.
    #[must_use]
    pub fn new(db: DBProvider<DbError>, limits: &LimitsConfig) -> Self {
        Self {
            db,
            defaults: AuthoringPolicy::from_deployment_defaults(limits),
        }
    }

    /// Resolve `tenant_id`'s authoring-time caps and descriptor required-set.
    ///
    /// **A tenant with no policy row gets the ratified launch values**, and so
    /// does a tenant whose row leaves a cap null: the resolution is per column,
    /// not per row, because a tenant that configured one cap did not thereby ask
    /// for the other three to change. This is the clause that keeps D-152 from
    /// moving any of the ratified numbers.
    ///
    /// SQL-level BOLA: a foreign tenant's policy is invisible, which resolves to
    /// the deployment defaults rather than to someone else's caps — the same
    /// fail-to-absent reading every read in this module takes, and here it is
    /// the safe direction, since the alternative is enforcing one tenant's
    /// limits on another's catalog.
    ///
    /// # Errors
    /// [`RepoError::Db`] on a scope or storage failure;
    /// [`RepoError::CorruptRow`] when a stored cap is not a positive count the
    /// domain can count in, or `additional_required_descriptors` is not a JSON
    /// array of strings. Both are invariant breaches: the caps carry positivity
    /// `CHECK`s and the column is `NOT NULL DEFAULT '[]'`, so a row that reads
    /// otherwise was written around this gear.
    pub async fn authoring_policy(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
    ) -> Result<AuthoringPolicy, RepoError> {
        let conn = self
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("conn: {e}")))?;
        let row = policy_object::Entity::find()
            .secure()
            .scope_with(scope)
            .filter(Condition::all().add(policy_object::Column::TenantId.eq(tenant_id)))
            .one(&conn)
            .await
            .map_err(|e| RepoError::Db(format!("read policy object: {e}")))?;
        let Some(row) = row else {
            return Ok(self.defaults.clone());
        };
        Ok(AuthoringPolicy {
            max_tier_bands_per_row: cap(
                "pricing_policy_object.max_tier_bands_per_row",
                row.max_tier_bands_per_row,
                self.defaults.max_tier_bands_per_row,
            )?,
            max_price_rows_per_plan: cap(
                "pricing_policy_object.max_price_rows_per_plan",
                row.max_price_rows_per_plan,
                self.defaults.max_price_rows_per_plan,
            )?,
            max_custom_interval_days: cap(
                "pricing_policy_object.max_custom_interval_days",
                row.max_custom_interval_days,
                self.defaults.max_custom_interval_days,
            )?,
            max_custom_interval_months: cap(
                "pricing_policy_object.max_custom_interval_months",
                row.max_custom_interval_months,
                self.defaults.max_custom_interval_months,
            )?,
            additional_required_descriptors: read_required_keys(
                &row.additional_required_descriptors,
            )?,
        })
    }
}

/// Resolve one cap column against the deployment default.
///
/// A null column is the ratified value and never an error. A stored zero or a
/// negative is an **invariant breach**, not a caller mistake — the column's
/// `CHECK` refuses both — and it surfaces rather than silently falling back,
/// because falling back would make a cap nobody can satisfy indistinguishable
/// from a cap nobody configured.
fn cap(column: &str, stored: Option<i32>, default: u32) -> Result<u32, RepoError> {
    let Some(value) = stored else {
        return Ok(default);
    };
    u32::try_from(value)
        .ok()
        .filter(|count| *count > 0)
        .ok_or_else(|| RepoError::CorruptRow(format!("{column} holds {value}, not a positive cap")))
}

/// Read the descriptor required-set extension out of its JSON column.
///
/// De-duplicated and first-seen order kept, for
/// [`DescriptorSetComplete::extending_v1`]'s reason: a key named twice is a typo
/// and reporting one absence twice reads as two faults. A column that is not an
/// array of strings is an invariant breach — it is `NOT NULL DEFAULT '[]'` and
/// no CHECK looks inside a JSON document, the same weaker ground the add-on edge
/// sets rest on.
fn read_required_keys(stored: &JsonValue) -> Result<Vec<String>, RepoError> {
    let malformed = || {
        RepoError::CorruptRow(format!(
            "pricing_policy_object.additional_required_descriptors is not a JSON array of \
             strings: {stored}"
        ))
    };
    let names = stored
        .as_array()
        .ok_or_else(malformed)?
        .iter()
        .map(|value| value.as_str().map(ToOwned::to_owned).ok_or_else(malformed))
        .collect::<Result<Vec<String>, RepoError>>()?;
    // De-duplicated after the whole array has been read, not during: a filter
    // over `Result`s drops the failures as well as the duplicates, and a
    // malformed entry would then read as a tenant that configured nothing.
    let mut seen = BTreeSet::new();
    Ok(names
        .into_iter()
        .filter(|name| seen.insert(name.clone()))
        .collect())
}

#[cfg(test)]
#[path = "policy_repo_tests.rs"]
mod policy_repo_tests;
