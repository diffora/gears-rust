//! Repository over `pricing_policy_object` — the per-tenant policy store
//! (`design/01-foundation.md` §3.7).
//!
//! **One repository over the table, and one method on it — that is the
//! narrowing this file is.** The row carries three more policies, and each is
//! read on a different path at a different moment: the approval threshold at
//! §4.2 step 3, the tax-display mode in Slice 4, the notice period when Slice 11
//! schedules an enforced migration. None of those paths exists yet, each wants a
//! different value shape, and each carries its own reading of absence. A method
//! returning the whole row today would answer three questions nobody is asking
//! and would fix one shape for three readers that have not been written — so the
//! row's other half gets its methods **here**, beside this one, when its slices
//! land. A second repository over one table is what would be wrong; a second
//! method is not.
//!
//! What this one reads is exactly what the **authoring** path resolves: the four
//! §14 caps and the descriptor required-set extension D-152 put in this table,
//! and — since 2026-08-03, when the publish path that reads it landed — the
//! **default rounding policy**. That one joined this method rather than getting
//! one of its own because it is resolved at the same moment and by the same
//! caller: the §4.2 step-2 pre-check and the commit's re-validation each build
//! one rule set for one tenant, and two reads to build it would be two chances
//! for the two runs to disagree about what the tenant had configured.
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

use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait, JsonValue};
use toolkit_db::secure::{
    AccessScope, DBRunner, SecureEntityExt, SecureInsertExt, SecureUpdateExt,
};
use toolkit_db::{DBProvider, DbError};
use uuid::Uuid;

use crate::config::LimitsConfig;
use crate::domain::audit::AuditStamp;
use crate::domain::plan_rules::{CustomIntervalBounds, DescriptorSetComplete};
use crate::domain::tax_display::TaxDisplayPolicy;
use crate::infra::storage::entity::policy_object;
use crate::infra::storage::{RepoError, contention_or_db};

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
    default_rounding_policy_ref: Option<String>,
    tax_display_policy_mode: String,
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
            // Absent, and absent is the fail-closed reading: a deployment-wide
            // rounding default would decide the last minor unit of every charge
            // of every tenant that never asked for one, which is precisely the
            // implicit rounding PRD §17.4 refuses. A tenant without an entry
            // simply requires every published row to carry its own.
            default_rounding_policy_ref: None,
            // C4 is fail-closed "for **all** tenants", so a tenant with no
            // policy row is governed by it exactly as one with a row that says
            // so. There is no deployment knob here for the same reason there is
            // none for the rounding default.
            tax_display_policy_mode: TaxDisplayPolicy::FailClosed.as_str().to_owned(),
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
    /// Read by
    /// [`PlanSizeWithinSoftCaps`](crate::domain::publish::rules::PlanSizeWithinSoftCaps)
    /// as of D-160, which named the advisory code `nfr-size-limits`' **SHOULD**
    /// needed. An earlier version of this doc said nothing read it and gave the
    /// reason: minting a discriminator no document defined would have put a code
    /// on the wire no consumer could act on. The document defines it now.
    ///
    /// It stays **soft**: the finding rides `warnings[]` and never blocks.
    #[must_use]
    pub const fn max_tier_bands_per_row(&self) -> u32 {
        self.max_tier_bands_per_row
    }

    /// The price-row soft cap in force for this tenant; see
    /// [`AuthoringPolicy::max_tier_bands_per_row`] for what reads it and why it
    /// never blocks.
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

    /// The tenant's default rounding policy, when they have configured one.
    ///
    /// `None` is not "no policy applies": it is the state in which **every**
    /// published row must carry its own `rounding_policy_ref` or the publish
    /// fails with `ROUNDING_POLICY_UNRESOLVED` (§3.3, PRD §17.4). There is
    /// deliberately no deployment fallback behind it, unlike the four caps
    /// above: a cap has a ratified launch number and rounding has no safe
    /// default at all, which is the whole of why the code exists.
    #[must_use]
    pub fn default_rounding_policy_ref(&self) -> Option<&str> {
        self.default_rounding_policy_ref.as_deref()
    }

    /// C4's tax-display enforcement mode.
    ///
    /// An unreadable token is **not** a fallback to the default: the column's
    /// `CHECK` admits exactly two values, so a third is an invariant breach and
    /// resolving it to `fail_closed` would hide a corrupt row behind the safe
    /// answer. It surfaces, which is `cap_or_default`'s discipline one field
    /// over — a stored value the schema forbids is not a tenant preference.
    ///
    /// # Errors
    /// [`RepoError::CorruptRow`] when the stored token is outside the `CHECK`.
    pub fn tax_display_policy(&self) -> Result<TaxDisplayPolicy, RepoError> {
        TaxDisplayPolicy::parse(&self.tax_display_policy_mode).ok_or_else(|| {
            RepoError::CorruptRow(format!(
                "pricing_policy_object.tax_display_policy_mode `{}`",
                self.tax_display_policy_mode
            ))
        })
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
        self.authoring_policy_on(&conn, scope, tenant_id).await
    }

    /// The same resolution, through whichever runner the caller holds.
    ///
    /// The publish commit re-resolves the policy **inside its transaction**,
    /// where `Db::conn()` is refused by the toolkit's transaction-bypass guard —
    /// and it re-resolves it rather than carrying the pre-check's copy because
    /// the caps are part of the state §4.2's second run re-validates. An
    /// operator lowering a cap between submit and commit is exactly the moved
    /// world that clause exists to catch.
    ///
    /// # Errors
    /// [`RepoError::Db`] on a scope or storage failure;
    /// [`RepoError::CorruptRow`] when a stored cap is not a positive count the
    /// domain can count in, or `additional_required_descriptors` is not a JSON
    /// array of strings.
    pub async fn authoring_policy_on(
        &self,
        runner: &impl DBRunner,
        scope: &AccessScope,
        tenant_id: Uuid,
    ) -> Result<AuthoringPolicy, RepoError> {
        let row = policy_object::Entity::find()
            .secure()
            .scope_with(scope)
            .filter(Condition::all().add(policy_object::Column::TenantId.eq(tenant_id)))
            .one(runner)
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
            // Taken as stored, with no deployment fallback: see
            // `AuthoringPolicy::default_rounding_policy_ref`.
            default_rounding_policy_ref: row.default_rounding_policy_ref,
            tax_display_policy_mode: row.tax_display_policy_mode,
        })
    }
}

/// Set the tenant's tax-display enforcement mode (§5's `PUT`, C4).
///
/// **Upsert, because a tenant with no policy row is the ordinary state.** C4
/// governs every tenant whether or not one has ever written a policy object, so
/// the first `PUT` has to be able to create the row — and every other column
/// then takes the schema default, which is the ratified launch value each of
/// them documents.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure.
pub async fn set_tax_display_policy(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    mode: TaxDisplayPolicy,
    stamp: &AuditStamp,
) -> Result<(), RepoError> {
    let updated = policy_object::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            policy_object::Column::TaxDisplayPolicyMode,
            Expr::value(mode.as_str()),
        )
        .col_expr(
            policy_object::Column::UpdatedAtUtc,
            Expr::value(stamp.recorded_at),
        )
        .col_expr(
            policy_object::Column::UpdatedBy,
            Expr::value(stamp.actor_principal_id),
        )
        .filter(Condition::all().add(policy_object::Column::TenantId.eq(tenant_id)))
        .exec(runner)
        .await
        .map_err(|e| RepoError::Db(format!("update tax-display policy: {e}")))?;
    if updated.rows_affected > 0 {
        return Ok(());
    }

    let row = policy_object::ActiveModel {
        tenant_id: Set(tenant_id),
        tax_display_policy_mode: Set(mode.as_str().to_owned()),
        updated_at_utc: Set(stamp.recorded_at),
        updated_by: Set(stamp.actor_principal_id),
        ..Default::default()
    };
    policy_object::Entity::insert(row.clone())
        .secure()
        .scope_with_model(scope, &row)
        .map_err(|e| RepoError::Db(format!("scope pricing_policy_object: {e}")))?
        .exec(runner)
        .await
        .map(|_| ())
        .map_err(|e| contention_or_db(&e, "pricing_policy_object", "insert policy object"))
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
