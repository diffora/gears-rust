//! The read side's four design-introduced names, each addressable
//! (`design/08-read-models.md` §1.7; **P-D-07**, **P-D-39**, **P-D-70**).
//!
//! # Why the visibility contract is a `Condition` and not a predicate over rows
//!
//! `inst-rb-query` forbids post-filtering in as many words — *"post-filtering
//! is forbidden because a shed row must never have been fetched"* — and
//! `dod-visibility` states the same rule as *"The contract is applied at
//! query build. A row a caller may not see is not fetched."* So
//! [`VisibilityFilter::condition`] answers a `sea_orm` `Condition`, and this
//! module exposes **no** `fn visible(&row) -> bool`: the shape that would
//! admit the defect is not expressible here rather than merely discouraged.
//!
//! # The matrix is a total function over (state, surface)
//!
//! `dod-visibility`'s table is five states by three surfaces, and
//! [`serves`] is that table transposed into code — every cell, including the
//! three that read `never` for every surface. Written as a filter over a
//! served-set it would answer `never` for a state nobody had thought of,
//! which is the safe direction but is also how `retired`'s single carve-out
//! gets lost.
//!
//! **The fourth surface is P-D-70's, not this module's invention.** Arm 4
//! settles that *"`retired` is retrievable at `p1` through the by-id read
//! under an explicit state opt-in"* — *"no new route, one explicit
//! parameter, never the default"* — and the `DoD`'s own `retired`/filtered-
//! browse cell carries that sentence. A three-surface function would have to
//! either serve `retired` on filtered browse (false) or make the decision
//! unreachable (also false).
//!
//! # The stamp is a floor, and the advance rule lives here
//!
//! [`StalenessStamp`] carries `(as_of_catalog_version, projected_at)` and is
//! **a floor** (**P-D-07**): everything at or below it is reflected, and
//! later entity events may add, change **or remove** content relative to it.
//! The completeness reading was measured false — a retirement flip removes
//! content and increments no version — so a projector built on
//! strictly-additive would treat a legitimate removal as corruption.
//!
//! `as_of_catalog_version` is `Option`: a tenant that has published no
//! catalog version has no anchor, and **P-D-70** arm 6 makes the stamp *"one
//! per-tenant stamp row"* whose `projectedAt` advances on **every** apply,
//! version or none. That row ships as `products_read_stamp`
//! (`m20260901_000024`). [`advance_stamp`] is the advance rule's host: it
//! refuses to move the stamp until the caller reports the event's
//! changed-entity list as projected, and it never treats a content removal
//! as a version regression. The projector (`dod-projector`) drives it; this
//! module does not build that consumer.
//!
//! # `ReadProjector`'s subject is the event-driven family only
//!
//! §1.7 is explicit: *"the polled dashboard family of `inst-ps-dashboards`
//! is not its subject"*. [`ReadProjector::CONSUMED`] is therefore the roster
//! §1.8 declares Consumed, and [`POLLED_SURFACES`] is the set it must not
//! claim — kept as two lists so each stays falsifiable, the way the event
//! rosters are.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-read-seams:p1
//! @cpt-dod:cpt-cf-bss-products-dod-visibility:p1
//! @cpt-dod:cpt-cf-bss-products-dod-staleness-stamp:p1
//! @cpt-dod:cpt-cf-bss-products-dod-projection-table:p1

use bss_products_sdk::models::LifecycleState;
use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, Condition};
use toolkit_macros::domain_model;

use crate::infra::storage::entity::read_entity;

/// The single event-driven consumer (§1.7).
///
/// A named seam rather than a running consumer: `dod-projector` owns the
/// apply loop and is blocked by its own §7 rows. What this type carries is
/// the boundary — which events are its subject and which surfaces are not.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ReadProjector;

impl ReadProjector {
    /// **The consumer-visible order is the broker's, not the outbox's**, and
    /// the two are not the same claim.
    ///
    /// §1.7 words the projector as *"per-tenant ordered by the outbox
    /// `(tenant, aggregate)` keys"*, and `dod-read-seams` corrects it:
    /// `infra/events.rs` records that the `(tenant, aggregate)` hash is *"a
    /// *pipeline* invariant that P-D-47 supersedes for the guarantee a
    /// consumer actually reads"*, and the broker's ordering — a read-side
    /// `sequence` per `(topic, partition)`, one partition per tenant in
    /// publish order — is **stronger** than the key the envelope promises.
    /// A projector built to the envelope's key would be correct and would
    /// also be assuming less than it is given.
    pub const ORDERING: &'static str = "broker sequence per (topic, partition), one per tenant";

    /// §1.8's **Consumed** roster, by owning slice. Transcribed rather than
    /// derived: a list built from the code under test could only prove the
    /// code equals itself.
    pub const CONSUMED: [(&'static str, &'static str); 6] = [
        ("01", "publishes and discards"),
        ("04", "deprecation and retirement flips"),
        (
            "02",
            "Category*, CategoryDisplayUpdated, AttributeDefinitionUpdated",
        ),
        (
            "06",
            "CatalogVersionPublished - advances the StalenessStamp",
        ),
        ("03", "vocabulary events, for tier labels"),
        (
            "10",
            "no events - listed nowhere in section 1.8, and named here so its absence is deliberate",
        ),
    ];

    /// Whether the projector claims a surface. Answers `false` for every
    /// polled dashboard, which is the boundary §1.7 draws.
    #[must_use]
    pub fn claims(surface: &str) -> bool {
        !POLLED_SURFACES.contains(&surface)
    }
}

/// The three polled projections, which the projector does **not** feed
/// (`inst-ps-dashboards`): their sources emit no events at all — 04's
/// deferred table and the broker's delivery/DLQ state emit none, and 06's
/// acks and re-triggers are audit-plane.
pub const POLLED_SURFACES: [&str; 3] = [
    "products_read_deferred_intent",
    "products_read_freeze_status",
    "products_read_delivery_state",
];

/// The denormalized serving row's projection shape (§1.7).
///
/// The columns live on [`read_entity::Model`]; what this type adds is the
/// **name** §1.7 introduces plus the one rule a column list cannot carry —
/// which fields are display-resolved per locale and therefore materialized
/// rather than computed at read.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct BrowseProjection;

impl BrowseProjection {
    /// The fields materialized per locale coordinate for the tenant's active
    /// locales, so a browse response resolves no attribute at read time.
    pub const LOCALE_MATERIALIZED: [&'static str; 2] = ["display_attributes", "plan_tier_label"];
}

/// The per-tenant floor every response carries (§1.7, C3, **P-D-07**).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct StalenessStamp {
    /// The last catalog version projected, or `None` for a tenant that has
    /// published none — the anchorless case P-D-70 arm 6 states rather than
    /// glosses.
    pub as_of_catalog_version: Option<i64>,
    /// The last apply's instant. Advances on **every** apply, version or
    /// none, so the sole freshness signal always has a writer.
    pub projected_at: DateTime<Utc>,
}

impl StalenessStamp {
    /// The stamp a tenant's bootstrap writes before any catalog version
    /// exists. An apply, and it stamps.
    #[must_use]
    pub const fn anchorless(projected_at: DateTime<Utc>) -> Self {
        Self {
            as_of_catalog_version: None,
            projected_at,
        }
    }

    /// The stamp a **polled** surface carries: its own table's last apply,
    /// and no catalog version, because its content bears no relation to one
    /// (**P-D-70** arm 3 — *"every polled surface carries the stamp of its
    /// own table's last apply"*, which is what C3's every-response rule
    /// means for `products_read_delivery_state`).
    ///
    /// Identical in shape to [`Self::anchorless`] and named apart on
    /// purpose: the two answer different questions, and a single constructor
    /// would make a dashboard's stamp indistinguishable from a zero-version
    /// tenant's bootstrap.
    #[must_use]
    pub const fn polled(projected_at: DateTime<Utc>) -> Self {
        Self {
            as_of_catalog_version: None,
            projected_at,
        }
    }
}

/// How one projector apply touches the catalog-version half of the stamp.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StampCatalogTouch {
    /// Leave `as_of_catalog_version` alone — an entity-only apply (publish,
    /// retirement flip, …) that still advances `projected_at`.
    Unchanged,
    /// Set the floor to this catalog version — a `CatalogVersionPublished`
    /// whose changed-entity list has already been projected in this step.
    Set(i64),
    /// Explicit null: a zero-version tenant's bootstrap. The null is a
    /// stated value, not an absence.
    Anchorless,
}

/// One projector apply's stamp operands.
///
/// The projector builds this **after** projecting the event's changed-entity
/// list from frozen rows in the same step. [`advance_stamp`] refuses any
/// advance where [`Self::entities_projected`] is false — that is the
/// ordering obligation that keeps the stamp a floor rather than a claim.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct StampApply {
    /// How this apply moves the catalog-version coordinate.
    pub catalog: StampCatalogTouch,
    /// The apply's instant. Advances on every admitted apply.
    pub projected_at: DateTime<Utc>,
    /// Whether this step's changed-entity list has already been projected
    /// from frozen rows. Required for any advance, including an empty list
    /// (bootstrap / version-or-none apply with nothing to rewrite).
    pub entities_projected: bool,
}

/// Why [`advance_stamp`] refused to move the stamp.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StampAdvanceRefusal {
    /// The caller asked to stamp before projecting the event's own
    /// changed-entity list. Advancing now would claim a version whose
    /// content is missing.
    EntitiesNotYetProjected,
    /// `projected_at` must move forward on every apply; a non-increasing
    /// instant is not an apply.
    ProjectedAtDidNotAdvance,
}

/// Compute the next stamp from the current one and one apply.
///
/// # Errors
///
/// [`StampAdvanceRefusal`] when the ordering obligation is unmet or the
/// instant does not advance.
pub fn advance_stamp(
    current: Option<StalenessStamp>,
    apply: StampApply,
) -> Result<StalenessStamp, StampAdvanceRefusal> {
    if !apply.entities_projected {
        return Err(StampAdvanceRefusal::EntitiesNotYetProjected);
    }
    if let Some(prev) = current
        && apply.projected_at <= prev.projected_at
    {
        return Err(StampAdvanceRefusal::ProjectedAtDidNotAdvance);
    }
    let as_of_catalog_version = match apply.catalog {
        StampCatalogTouch::Unchanged => current.and_then(|s| s.as_of_catalog_version),
        StampCatalogTouch::Set(id) => Some(id),
        StampCatalogTouch::Anchorless => None,
    };
    Ok(StalenessStamp {
        as_of_catalog_version,
        projected_at: apply.projected_at,
    })
}

/// Whether the **completeness** reading would treat a content shrinkage at
/// an unchanged catalog version as corruption.
///
/// Completeness says the projection at stamp `S` is the full catalog of `S`
/// and may only grow for later versions. A retirement flip removes a row
/// and increments no version — legitimate under the floor, "corruption"
/// under completeness. This function exists so a test can arm against that
/// false alarm rather than only against additive cases.
#[must_use]
pub const fn completeness_rejects_removal(
    rows_before: usize,
    rows_after: usize,
    catalog_version_unchanged: bool,
) -> bool {
    catalog_version_unchanged && rows_after < rows_before
}

/// Whether the **floor** reading admits the same removal.
///
/// The floor asserts everything at or below the stamp is reflected, and
/// asserts nothing about content above it — including removals that leave
/// `as_of_catalog_version` alone while `projected_at` advances.
#[must_use]
pub fn floor_admits_removal(
    rows_before: usize,
    rows_after: usize,
    before: &StalenessStamp,
    after: &StalenessStamp,
) -> bool {
    rows_after < rows_before
        && before.as_of_catalog_version == after.as_of_catalog_version
        && after.projected_at > before.projected_at
}

/// Which surface is asking (`dod-visibility`'s three columns, plus P-D-70
/// arm 4's by-id read).
#[domain_model]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ReadSurface {
    /// Browse with no state filter.
    DefaultBrowse,
    /// Browse under `excludeDeprecated` — the only filter that changes a
    /// cell, and it changes exactly one.
    FilteredBrowse {
        /// Whether the caller asked to exclude deprecated rows.
        exclude_deprecated: bool,
    },
    /// The by-id read under an explicit state opt-in (**P-D-70** arm 4).
    /// Without the opt-in it behaves as the default browse does.
    ByIdRead {
        /// Whether the caller opted in to non-default states explicitly.
        state_opt_in: bool,
    },
    /// The history timeline — `retired`'s one carve-out.
    History,
}

/// Whether a surface serves a row in this state.
///
/// The full matrix, cell by cell. `deprecated` on the default browse is
/// served **with the flag**, which is the row's own `deprecated` column and
/// not a decision here.
#[must_use]
pub const fn serves(state: LifecycleState, surface: ReadSurface) -> bool {
    match state {
        // Served everywhere.
        LifecycleState::Published => true,
        // Served everywhere except a filtered browse that asked to exclude it.
        LifecycleState::Deprecated => match surface {
            ReadSurface::FilteredBrowse { exclude_deprecated } => !exclude_deprecated,
            ReadSurface::DefaultBrowse | ReadSurface::ByIdRead { .. } | ReadSurface::History => {
                true
            }
        },
        // Never on browse; the history carve-out, and the by-id read only
        // under an explicit opt-in (P-D-70 arm 4).
        LifecycleState::Retired => match surface {
            ReadSurface::History => true,
            ReadSurface::ByIdRead { state_opt_in } => state_opt_in,
            ReadSurface::DefaultBrowse | ReadSurface::FilteredBrowse { .. } => false,
        },
        // Never, on any surface. The projector still records them, which is
        // why the table's CHECK admits them.
        LifecycleState::Draft | LifecycleState::Discarded => false,
    }
}

/// The per-state contract, applied **at query build** (§1.7,
/// `inst-rb-visibility`).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct VisibilityFilter {
    surface: ReadSurface,
}

impl VisibilityFilter {
    /// The filter for one surface.
    #[must_use]
    pub const fn for_surface(surface: ReadSurface) -> Self {
        Self { surface }
    }

    /// The states this surface serves, in the roster's own order.
    #[must_use]
    pub fn served_states(self) -> Vec<LifecycleState> {
        [
            LifecycleState::Draft,
            LifecycleState::Published,
            LifecycleState::Deprecated,
            LifecycleState::Retired,
            LifecycleState::Discarded,
        ]
        .into_iter()
        .filter(|s| serves(*s, self.surface))
        .collect()
    }

    /// The `WHERE` fragment the query is built with.
    ///
    /// An `IN` over the served states rather than a `NOT IN` over the
    /// withheld ones: the negative form serves any state added later, and
    /// `lifecycle_state`'s roster is a five-value `CHECK` that a migration
    /// can widen.
    #[must_use]
    pub fn condition(self) -> Condition {
        let served: Vec<String> = self
            .served_states()
            .into_iter()
            .map(|s| s.as_str().to_owned())
            .collect();
        Condition::all().add(read_entity::Column::LifecycleState.is_in(served))
    }
}

/// The query-build scope predicate for one axis (**P-D-39**).
///
/// **Empty means unrestricted**, so the predicate matches a row whose set is
/// empty **or** contains the caller's claim. Written as containment alone it
/// hides every unrestricted row — which is the whole catalogue of a tenant
/// that has set no scopes.
///
/// # It is set membership, and `contains` was a leak
///
/// The column is a **comma-joined token set**, not a scalar:
/// [`crate::domain::containment::SCOPE_VALUE_SEPARATOR`] is `,` and
/// `domain::containment` owns the rule. `ColumnTrait::contains` renders an
/// unanchored `LIKE '%claim%'` with nothing escaped, and the first version of
/// this function used it. Measured consequences, each a cross-scope read:
/// claim `eu` matched a row stored `eur`, `eu-west` or `aus,eu-central`;
/// claim `us` matched `aus`; and a claim containing `%` matched **every**
/// restricted row in the table. `SQLite`'s `LIKE` is ASCII-case-insensitive
/// and Postgres's is not, so the two engines disagreed as well.
///
/// The predicate below matches a **token by position** — the whole value, or
/// the set's first, last or a middle member — so `eu` cannot match `eur` or
/// `aus,eu-central`. A claim carrying the separator or either wildcard is
/// refused the containment arm entirely rather than escaped: it cannot be a
/// member of a well-formed set, and there is then no operand a caller
/// supplies that can widen the predicate.
///
/// **One residual, recorded rather than closed**: `SQLite`'s `LIKE` is
/// ASCII-case-insensitive and Postgres's is not, so on `SQLite` a claim `eu`
/// also admits a stored token `EU`. Scope values are resolved before
/// persistence (`domain::containment`) and nothing in the crate writes a
/// mixed-case token; closing it needs custom SQL, and the Postgres tier is
/// the authority for the served behaviour.
///
/// # It carries no tenant predicate, and must not be used alone
///
/// This is one axis of a `WHERE` clause. The tenant comes from the secure
/// scope (`.secure().scope_with(scope)`), and a query built with this filter
/// and no scope serves every tenant's unrestricted rows — which P-D-39 makes
/// the majority of rows. The probes compose it through the secure path for
/// that reason: a bare `Entity::find()` example is the one a door would copy.
#[must_use]
pub fn scope_condition(column: read_entity::Column, claim: &str) -> Condition {
    let sep = crate::domain::containment::SCOPE_VALUE_SEPARATOR;
    let unrestricted = Condition::any().add(column.eq(""));

    // **Fail closed on anything that is not a single token.** A scope value
    // is a resolved region or brand identifier; `domain::containment` chose a
    // comma precisely because neither kind is expected to contain one. A
    // claim carrying the separator, or either `LIKE` wildcard, cannot be a
    // member of a well-formed set — and admitting it as a pattern is the
    // leak: `%` alone matched every restricted row. Dropping the containment
    // arm leaves only the unrestricted rows, which is the safe direction.
    if claim.is_empty() || claim.contains([sep, '%', '_', '\\']) {
        return unrestricted;
    }

    // Token membership by position, in plain `sea_orm`: the whole value, the
    // first member, the last, or a middle one. No custom SQL and no `ESCAPE`
    // clause, because the guard above means the claim carries no wildcard to
    // escape — the two engines therefore agree on the pattern.
    unrestricted
        .add(column.eq(claim))
        .add(column.like(format!("{claim}{sep}%")))
        .add(column.like(format!("%{sep}{claim}")))
        .add(column.like(format!("%{sep}{claim}{sep}%")))
}

#[cfg(test)]
#[path = "read_model_tests.rs"]
mod read_model_tests;
