//! Typed gear configuration.

use serde::Deserialize;

/// The retention floor `design/01-foundation.md` §3.2
/// `inst-fd-idem-retention` and C6 pin: `max(24h, max_freeze_timeout)`, whose
/// second half has no source until the catalog-version feature exports it.
///
/// A **floor**, not a default: a window shorter than this expires a key while
/// the client that owns it is still retrying, and the next request on that
/// key takes it over and **re-executes the guarded mutation** — at-most-once
/// silently off. `ProductsConfig::default` happens to supply this same value,
/// which is why an unconfigured boot needs no clamp; a *configured* one does.
pub const IDEMPOTENCY_RETENTION_FLOOR_HOURS: u32 = 24;

/// The longest retention window this gear will stamp: ten years, in hours.
///
/// The field is a `u32` of hours, and its largest value is roughly 490 000
/// years — far past what `chrono` can add to an instant, so
/// `DateTime::checked_add_signed` returns `None` and the stamp has no
/// representable answer at all. A ceiling is what keeps the resolution
/// **total**: every `u32` an operator can write maps to a window that is
/// neither below the floor nor unrepresentable, so no caller downstream has
/// to invent one. Ten years is chosen because the value being resolved is how
/// long a *client's retry key* is remembered; anything past a decade is a
/// mis-entered unit (seconds or minutes pasted into an hours field), not a
/// retention policy anyone wrote on purpose.
pub const IDEMPOTENCY_RETENTION_CEILING_HOURS: u32 = 24 * 365 * 10;

/// The default row ceiling: the design's own sizing fixture is the
/// ten-thousand-SKU onboarding case, so the shipped default admits it and
/// leaves headroom rather than making the fixture the bound.
pub const BULK_MAX_ROWS_DEFAULT: u32 = 50_000;

/// `TAXONOMY_LIMIT`'s depth ceiling — **interim, P-D-107 arm 1**. See
/// [`ProductsConfig::taxonomy_max_depth`] for why eight.
pub const TAXONOMY_MAX_DEPTH_DEFAULT: u32 = 8;

/// `TAXONOMY_LIMIT`'s fan-out ceiling — **interim, P-D-107 arm 1**. See
/// [`ProductsConfig::taxonomy_max_children_per_node`].
pub const TAXONOMY_MAX_CHILDREN_DEFAULT: u32 = 1_000;

/// `METADATA_LIMIT`'s key-count ceiling — **interim, P-D-107 arm 1**. The
/// three metadata ceilings share one reason; see
/// [`ProductsConfig::metadata_max_keys`].
pub const METADATA_MAX_KEYS_DEFAULT: u32 = 50;

/// `METADATA_LIMIT`'s key-length ceiling, in bytes — **interim, P-D-107 arm 1**.
pub const METADATA_MAX_KEY_BYTES_DEFAULT: u32 = 128;

/// `METADATA_LIMIT`'s value-length ceiling, in bytes — **interim, P-D-107 arm 1**.
pub const METADATA_MAX_VALUE_BYTES_DEFAULT: u32 = 2_048;

/// Shipped default for [`ProductsConfig::attribute_values_max_per_patch`].
pub const ATTRIBUTE_VALUES_MAX_PER_PATCH_DEFAULT: u32 = 200;

/// The longest entity `name` a door admits, in bytes. A fixed limit and not
/// a knob: the column is unbounded text, and what the ceiling bounds is the
/// row, the normalized-name index entry and the frozen version content the
/// value lands in — none of which an operator tunes per deployment
/// (review wave 1, P-D-163).
pub const ENTITY_NAME_MAX_BYTES: usize = 256;

/// The longest `product_code` / `sku_code` a door admits, in bytes; see
/// [`ENTITY_NAME_MAX_BYTES`] for why it is fixed.
pub const ENTITY_CODE_MAX_BYTES: usize = 128;

/// The runner's claim lease, in seconds — **interim, P-D-113 arm 4**. See
/// [`ProductsConfig::activation_claim_lease_secs`].
pub const ACTIVATION_CLAIM_LEASE_SECS_DEFAULT: u32 = 60;

/// The runner's per-row attempt budget — **interim, P-D-113 arm 4**. See
/// [`ProductsConfig::activation_attempt_budget`].
pub const ACTIVATION_ATTEMPT_BUDGET_DEFAULT: u32 = 5;

/// Retention per record class, in days — **interim, P-D-118**: PRD §15's
/// *"statutory max"*, taken as the longest common one. One constant for the
/// three classes because the interim policy does not distinguish them; the
/// three fields exist so Legal and Finance can, per jurisdiction.
pub const RETENTION_DAYS_DEFAULT: u32 = 3_650;

/// The age-triggered tombstone's operand, in days of inactivity — **interim,
/// P-D-118**. See [`ProductsConfig::pseudonymization_age_days`].
pub const PSEUDONYMIZATION_AGE_DAYS_DEFAULT: u32 = 730;

/// The restore drill's cadence, in hours — **interim, P-D-118**.
pub const DRILL_CADENCE_HOURS_DEFAULT: u32 = 24;

/// The usage-type resolver's bound, in milliseconds — **interim, P-D-121**.
pub const USAGE_TYPE_RESOLVER_TIMEOUT_MS_DEFAULT: u32 = 2_000;

/// The break-glass window's interim, in hours — **P-D-132**, `PRD` §17.1.
pub const BREAKGLASS_WINDOW_HOURS_DEFAULT: u32 = 4;

/// The post-hoc review SLA's interim, in hours — **P-D-133**.
pub const BREAKGLASS_REVIEW_SLA_HOURS_DEFAULT: u32 = 24;

/// The held-retirement alert threshold's interim, in hours — **P-D-133**.
pub const RETIREMENT_HELD_ALERT_HOURS_DEFAULT: u32 = 72;

/// `design/07` §17.1's interim freshness threshold, in minutes.
pub const REFERENCE_FRESHNESS_MINUTES_DEFAULT: u32 = 15;

/// `inst-ws-not-future`'s interim clock-skew tolerance, in minutes.
pub const WATERMARK_SKEW_MINUTES_DEFAULT: u32 = 5;

/// §17.1's interim tripwire rate: more than five break-glass corrections
/// in a rolling thirty days fires it.
pub const TRIPWIRE_MAX_OVERRIDES_DEFAULT: u32 = 5;

/// The default per-tenant concurrent-batch ceiling. Small on purpose: a
/// batch is an operator act with an approval attached, and a tenant holding
/// many at once is the accident the ceiling exists to catch.
pub const BULK_MAX_CONCURRENT_DEFAULT: u32 = 5;

/// [`ProductsConfig::bulk_batch_ttl_hours`]'s interim default (**P-D-127**
/// row 6): a `reported` batch nobody approves is abandoned by the reaper
/// after a week, releasing the tenant's concurrency slot.
pub const BULK_BATCH_TTL_HOURS_DEFAULT: u32 = 168;

/// [`ProductsConfig::read_path_qps_ceiling`]'s interim default: the per-tenant
/// requests-per-second ceiling above which the `ReadPathLimiter` sheds with
/// `503 READ_MODEL_OVERLOADED` (`dod-degradation`, P-D-150).
pub const READ_PATH_QPS_CEILING_DEFAULT: u32 = 200;

/// [`ProductsConfig::read_poison_retry_ceiling`]'s interim default: how many
/// passes a poison inbox row is retried before it is parked for good and
/// alarmed (P-D-126 rows 9 and 12, on the P-D-107 idiom).
pub const READ_POISON_RETRY_CEILING_DEFAULT: u32 = 5;

/// [`ProductsConfig::read_convergence_budget_secs`]'s interim default: the
/// commit-to-projected budget past which `read_model_lag` is raised while
/// serving continues (`inst-dg-lag`).
pub const READ_CONVERGENCE_BUDGET_SECS_DEFAULT: u32 = 5;

/// [`ProductsConfig::read_dashboard_poll_secs`]'s interim default: the polled
/// dashboards' cadence (P-D-126 row 10).
pub const READ_DASHBOARD_POLL_SECS_DEFAULT: u32 = 30;

/// [`ProductsConfig::read_inbox_retention_hours`]'s interim default: how long
/// consumed inbox rows are kept for replay before the sweep takes them.
pub const READ_INBOX_RETENTION_HOURS_DEFAULT: u32 = 72;

/// The gear's boot configuration.
///
/// @cpt-cf-bss-products-fr-idempotent-authoring
///
/// Every field has a default, so a boot that configures the gear at all gets a
/// working one; `deny_unknown_fields` is what turns a typo in the operator's
/// file into a boot failure rather than a silently ignored setting.
///
/// A typo in a *value* has no such spelling, which is why
/// [`Self::resolved_idempotency_retention_hours`] exists: `deny_unknown_fields`
/// catches `idempotency_retention_hous`, and nothing in serde catches a `0`.
#[allow(clippy::struct_excessive_bools)] // the operator switches are booleans by nature
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, default)]
pub struct ProductsConfig {
    /// How long an idempotency key is retained, in hours, **as the operator
    /// wrote it**.
    ///
    /// The floor the design pins is 24 hours **and** at least the maximum
    /// freeze timeout, which the catalog-version feature exports. Until that
    /// feature exists the second half has no source, so this carries the first.
    ///
    /// Read this field only to report what was configured;
    /// [`Self::resolved_idempotency_retention_hours`] is what anything
    /// stamping an expiry must use.
    pub idempotency_retention_hours: u32,

    /// The freeze timeout, in hours (**P-D-84** — the field `design/01`'s
    /// retention floor and `inst-fz-timeout` both presupposed): past it an
    /// `open` version stays non-posting-safe — the timeout fails **closed**
    /// — and the coalescer's sweep raises `freeze_overdue` naming the
    /// silent participants. Per-deployment, so `max_freeze_timeout` IS this
    /// value; it floors the idempotency retention through
    /// [`Self::resolved_idempotency_retention_hours`], and
    /// [`Self::validate`] refuses a value above the retention ceiling at
    /// boot so that clamp stays total (P-D-84 arm 6).
    ///
    /// The default equals the shipped 24-hour floor constant: the timeout's
    /// floor contribution changes nothing until an operator configures
    /// more.
    ///
    /// @cpt-dod:cpt-cf-bss-products-dod-freeze-timeout:p1
    pub freeze_timeout_hours: u32,

    /// The maximum rows one bulk batch may carry (`inst-bm-limits`), the
    /// first of `BULK_LIMIT`'s two operands. The default carries the
    /// sizing fixture the design names — the ten-thousand-SKU onboarding
    /// case — with headroom, so the shipped bound refuses nothing that
    /// case does.
    pub bulk_max_rows_per_batch: u32,

    /// The maximum batches one tenant may hold outside a terminal state
    /// (`inst-bm-limits`), `BULK_LIMIT`'s second operand — checked at the
    /// import door **and** re-checked by the worker at claim (P-D-54: a
    /// ceiling checked only by the door drifts as batches hang). Both
    /// bounds are `inst-bm-limits`' — an instruction no `DoD` carries by
    /// name, so the marker rides `dod-import-door`, which is where the
    /// refusal they produce is obliged.
    pub bulk_max_concurrent_batches_per_tenant: u32,

    /// How long a `reported` batch waits for its approval before the reaper
    /// abandons it (`reported -> abandoned`, P-D-69's state; **P-D-127**
    /// row 6 names the reaper and this interim of a week). The batch approval
    /// is superseded by the abandonment; the tenant's slot is released.
    /// `dod-batch-state-machine`.
    pub bulk_batch_ttl_hours: u32,

    /// `08`'s per-tenant read ceiling, requests per second: above it the
    /// `ReadPathLimiter` sheds with `503 READ_MODEL_OVERLOADED` and
    /// `Retry-After` (`dod-degradation`; P-D-150).
    pub read_path_qps_ceiling: u32,

    /// How many projector passes retry a poison inbox row before it stays
    /// parked and alarmed (P-D-126 rows 9 and 12).
    pub read_poison_retry_ceiling: u32,

    /// The commit-to-projected budget, seconds: past it the projector raises
    /// `read_model_lag` while the read path keeps serving (`inst-dg-lag`).
    pub read_convergence_budget_secs: u32,

    /// The polled dashboards' cadence, seconds (P-D-126 row 10).
    pub read_dashboard_poll_secs: u32,

    /// How long consumed inbox rows stay for replay before the sweep.
    pub read_inbox_retention_hours: u32,

    /// The active locale set `display_attributes` are materialised for
    /// (P-D-126 row 2); an empty set is refused at boot.
    pub read_active_locales: Vec<String>,

    /// How long a posted reference watermark stays **fresh**, in minutes
    /// (interim 15 — **P-D-87** arm 1). Past it a producer's verdict is
    /// `conservatively_referenced(stale)` rather than a fresh zero, which
    /// is the direction that never falsely frees a referenced SKU.
    ///
    /// Read through [`Self::reference_freshness`] rather than directly:
    /// `04-lifecycle`'s activation runner polls on this interval and needs
    /// it as a `Duration`.
    pub reference_freshness_minutes: u32,

    /// How far above the receiving clock a posted `watermark_at` may sit
    /// before it is refused `WATERMARK_FUTURE` and alerted, in minutes
    /// (interim 5 — **P-D-87** arm 1). The bound is `p1` rather than
    /// hygiene: one accepted future-dated post makes its producer read
    /// permanently fresh, freezes its member set behind
    /// `WATERMARK_REGRESSION`, and leaves every SKU outside that frozen
    /// set reading fresh-zero — the never-falsely-free invariant inverted
    /// by one timestamp.
    pub watermark_skew_tolerance_minutes: u32,

    /// How many break-glass corrections in a rolling thirty days the
    /// tripwire admits before it fires (interim 5 — **P-D-87** arm 1).
    pub tripwire_max_overrides_per_30_days: u32,

    /// Whether the break-glass correction arm is available at all
    /// (**P-D-71** arm 1 named the flag **enable-positive**, so `false`
    /// means the arm is disabled and `BREAKGLASS_CORRECTION_DISABLED` is
    /// what the door answers). Per-deployment and boot-time: a policy
    /// gate, not an incident tool — the emergency surface is `05`'s read
    /// elevation.
    ///
    /// @cpt-dod:cpt-cf-bss-products-dod-reference-config:p1
    pub breakglass_correction_enabled: bool,

    /// Whether a boot without a reachable event-broker is a **failure**.
    ///
    /// `Gear::init` binds the broker SDK's producer when `ClientHub` carries an
    /// `EventBrokerApi` and falls back to a holding processor when it does not,
    /// so a deployment with no broker still boots and accumulates its events
    /// undelivered. That fallback is deliberate (P-D-47's letter says otherwise;
    /// see `infra::broker`'s module doc) and it has one dangerous property: it
    /// is **indistinguishable from a broker the gear failed to reach**, and the
    /// only signal is one `warn!` line.
    ///
    /// This is the operator's switch for that. `true` turns the fallback into a
    /// boot failure, so a deployment that is supposed to publish cannot
    /// silently stop publishing.
    ///
    /// **Default `false`, and that default is a measurement rather than a
    /// preference**: as of 2026-08-30 no gear in this workspace registers a
    /// `dyn EventBrokerApi` in any `ClientHub`, so defaulting to `true` would
    /// make this gear un-bootable everywhere today. The default is expected to
    /// invert the moment a provider exists.
    pub require_broker: bool,

    /// Whether `04`'s post-v1 EOL machinery is switched on — `mustMigrateBy`
    /// on a retirement and the consumer-acknowledgment lockout
    /// (`inst-rt-eol-lockout`, `dod-eol-lockout`, **P-D-132**: EOL stays
    /// post-v1). **Default `false`**: with the flag off a retirement carrying
    /// `mustMigrateBy` is refused `EOL_DISABLED` and the payload field is never
    /// populated, so a `vN` consumer's schema keeps the field and reads it
    /// absent. Flipping it on is the post-v1 launch's act, not a deployment's.
    pub eol_enabled: bool,

    /// The tenant's default locale — step 3 of `02`'s attribute-value fallback
    /// chain (`design/02` `inst-av-resolve`, **P-D-101**).
    ///
    /// **Empty is admitted and means "no preference".** The chain is
    /// `(locale, region, brand) → (locale, brand) → (default-locale, brand) →
    /// global`, and `inst-av-resolve` anchors its **totality on step 4's global
    /// fallback, deliberately not on this value**: an ungoverned config field
    /// with no re-validation would un-total the chain for every
    /// already-published entity the moment it changed. So an unset default
    /// locale skips step 3 and resolution still succeeds — this field shortens
    /// the path, it does not decide whether there is one.
    ///
    /// **That is also why [`ProductsConfig::validate`] does not refuse an empty
    /// value**, though P-D-101 called for boot validation. `Gear::init` calls
    /// `config_or_default()` and then `validate()`, so refusing the default
    /// would make the gear un-bootable in every deployment that ships no
    /// config — the same measurement `require_broker`'s doc above records for
    /// its own default. What `validate` does refuse is a value that is
    /// non-empty and untrimmed, because a stored coordinate can never match it.
    ///
    /// **The locale's shape is not validated**, and that is a boundary rather
    /// than an omission: no document in the set names a locale grammar, and
    /// inventing one here would put a vocabulary in the config layer that
    /// `products_attribute_value`'s own `locale` column does not enforce.
    ///
    /// **P-D-101 struck the per-brand half.** There is deliberately no
    /// `default_locale_per_brand`: a second ungoverned config value under a
    /// step that cannot change whether resolution succeeds doubles the exposure
    /// the paragraph above refuses. If the capability is ever asked for, it
    /// returns as a governed fourth coordinate kind, re-validated.
    pub default_locale: String,

    /// Maximum category-tree depth (`inst-tx-governed-op` step 3's first
    /// operand, `TAXONOMY_LIMIT`). **Interim 8 — P-D-107 arm 1**, which the
    /// design defers to the NFR workshop: `design/02` C3 makes both taxonomy
    /// limits *"configured policies whose values PRD §7
    /// `nfr-scale-extensibility` defers"*, so a number had to exist before
    /// either `DoD` could be built.
    ///
    /// Eight because the walk runs **inside the write transaction** under the
    /// per-tenant taxonomy writer lock (§3.4), so depth is what bounds that
    /// lock's hold: real catalog taxonomies run four to six levels, and eight
    /// leaves headroom without letting one re-parent hold the lock over an
    /// unbounded chain.
    pub taxonomy_max_depth: u32,

    /// Maximum children of one category (`TAXONOMY_LIMIT`'s second operand).
    /// **Interim 1000 — P-D-107 arm 1.**
    ///
    /// Anchored to the PRD's own sizing target the way
    /// [`Self::bulk_max_rows_per_batch`] is: at *"≥ 10K SKUs per tenant"*
    /// (PRD §7) a node admitting a thousand children still classifies that
    /// catalogue two levels down, so the bound refuses nothing the stated
    /// case needs, while it does bound one level's fan-out for the walk above.
    pub taxonomy_max_children_per_node: u32,

    /// Maximum keys in one entity's metadata map (`METADATA_LIMIT`'s first
    /// operand). **Interim 50 — P-D-107 arm 1.**
    ///
    /// The three metadata caps share one reason: **P-D-06** puts this map
    /// *outside* frozen version content, so nothing here is versioned, frozen
    /// or rendered. The caps exist so the map cannot become a shadow content
    /// store that escapes all three. Fifty keys, 128-byte keys and 2 KiB
    /// values put the worst case near 106 KiB per entity — annotation-sized —
    /// while one 2 KiB value still holds a URL, an owner, a ticket reference
    /// or a short note comfortably.
    pub metadata_max_keys: u32,

    /// Maximum bytes in one metadata **key** (`METADATA_LIMIT`'s second
    /// operand). **Interim 128 — P-D-107 arm 1**; see
    /// [`Self::metadata_max_keys`] for the shared reason.
    pub metadata_max_key_bytes: u32,

    /// Maximum bytes in one metadata **value** (`METADATA_LIMIT`'s third
    /// operand). **Interim 2048 — P-D-107 arm 1**; see
    /// [`Self::metadata_max_keys`] for the shared reason.
    pub metadata_max_value_bytes: u32,

    /// Maximum coordinates one category live-value patch may carry.
    /// **Interim 200 — P-D-163.** Every coordinate is one row write inside
    /// one transaction under the category's token, so the list's length is
    /// the transaction's length; two hundred holds a definition across every
    /// locale, region and brand a tenant realistically carries, and a larger
    /// change is two patches.
    pub attribute_values_max_per_patch: u32,

    /// How long a worker's claim on a `products_scheduled_transition` row
    /// holds before another worker may reclaim it, in seconds.
    /// **Interim 60 — P-D-113 arm 4.** The lifecycle loop ticks every second
    /// and a flip is one transaction, so a minute tolerates a slow flip and
    /// frees a crashed worker's row within a minute — which is the trade a
    /// lease exists to make. `ClaimLease` carried no value before this.
    pub activation_claim_lease_secs: u32,

    /// How many times a worker may pick up one scheduled row before the
    /// transition finishes `failed` for exhausting its budget.
    /// **Interim 5 — P-D-113 arm 4.** A pin mismatch is terminal on its first
    /// try, so this bounds only the transient-dependency arm, and a dependency
    /// that has not returned in five polls a second apart is not transient in
    /// any sense the runner can act on. `attempt` increments on every claim
    /// (arm 3), so this is *"how many times has a worker picked this up"*.
    pub activation_attempt_budget: u32,

    /// Retention of the **financial** record class, in days. **Interim 3650 —
    /// P-D-118.** PRD §15's own interim policy is *"financial/version/audit →
    /// statutory max"* with final durations per jurisdiction owned by Legal
    /// and Finance; ten years is the longest common statutory maximum, chosen
    /// so no jurisdiction's record is deleted early before Legal narrows it.
    /// The GC reads this; a DDL trigger cannot (item 27), which is why the
    /// window is here and the trigger guards only against unauthorised
    /// deletion.
    pub retention_days_financial: u32,

    /// Retention of the **version** record class, in days. **Interim 3650 —
    /// P-D-118**; see [`Self::retention_days_financial`] for the anchoring.
    pub retention_days_version: u32,

    /// Retention of the **audit** record class, in days. **Interim 3650 —
    /// P-D-118**; see [`Self::retention_days_financial`].
    pub retention_days_audit: u32,

    /// Age of a principal's **last activity** in a tenant after which the
    /// age-triggered tombstone fires, in days. **Interim 730 — P-D-118.**
    /// Anchored to `inst-er-age`'s own operand — last activity, not first
    /// appearance, since age-since-first-appearance *"would tombstone an
    /// active employee mid-employment"* (M2) — and two years without a stamped
    /// act is not an active employee.
    pub pseudonymization_age_days: u32,

    /// How often the restore drill runs, in hours. **Interim 24 — P-D-118**,
    /// so a corrupt backup is found within a day.
    pub drill_cadence_hours: u32,

    /// Where the restore drill reads its **restored copy** — the one the
    /// platform provides (**P-D-133**: the gear owns the probe, the platform
    /// owns the restore). **Optional and with no default** (**P-D-135**), on
    /// the P-D-107 idiom: there is no value that is right for every
    /// deployment, and a default pointing anywhere would make an unconfigured
    /// drill silently verify the *live* database, which proves nothing about
    /// a backup.
    ///
    /// `None` is a legitimate deployment state and not a failure: the drill
    /// still runs, still writes its audit row with outcome `no_target`, and
    /// still raises `products_restore_drill_unverifiable` — a drill that
    /// cannot run is not a passed drill, and silence is what P-D-133's
    /// *"report, never skip"* forbids.
    ///
    /// A **present but blank** value is refused at boot: it is an operator
    /// who meant to configure a target, and treating it as `None` would turn
    /// that into a decade of warnings nobody reads as a typo.
    pub drill_target_dsn: Option<String>,

    /// How long the publish path waits on the usage-type resolver, in
    /// milliseconds. **Interim 2000 — P-D-121.** `design/03` said *"a short
    /// timeout"* and named no number. Two seconds because the resolve runs
    /// **before** the publish transaction and hands the phase a `Resolution`
    /// (P-D-121 row 19, `MaterialityEvaluator`'s shape), so the bound costs
    /// latency on the publish path and not a held lock.
    pub usage_type_resolver_timeout_ms: u32,

    /// The break-glass elevation window, in hours. **Interim 4 — P-D-132**,
    /// `PRD` §17.1's row made configuration. Hard expiry and **no renewal**:
    /// a second window is a second session and a second two-person ceremony.
    pub breakglass_window_hours: u32,

    /// How long after a break-glass session expires its post-hoc review may
    /// take, in hours. **Interim 24 — P-D-133**; the obligation alert fires
    /// when it lapses.
    pub breakglass_review_sla_hours: u32,

    /// How old a deferred retirement may be before `retirement_held` fires,
    /// in hours. **Interim 72 — P-D-133**; the deferred-intent dashboard is
    /// the surface, this is its threshold.
    pub retirement_held_alert_hours: u32,
}

impl Default for ProductsConfig {
    fn default() -> Self {
        Self {
            idempotency_retention_hours: IDEMPOTENCY_RETENTION_FLOOR_HOURS,
            freeze_timeout_hours: IDEMPOTENCY_RETENTION_FLOOR_HOURS,
            bulk_max_rows_per_batch: BULK_MAX_ROWS_DEFAULT,
            bulk_max_concurrent_batches_per_tenant: BULK_MAX_CONCURRENT_DEFAULT,
            bulk_batch_ttl_hours: BULK_BATCH_TTL_HOURS_DEFAULT,
            read_path_qps_ceiling: READ_PATH_QPS_CEILING_DEFAULT,
            read_poison_retry_ceiling: READ_POISON_RETRY_CEILING_DEFAULT,
            read_convergence_budget_secs: READ_CONVERGENCE_BUDGET_SECS_DEFAULT,
            read_dashboard_poll_secs: READ_DASHBOARD_POLL_SECS_DEFAULT,
            read_inbox_retention_hours: READ_INBOX_RETENTION_HOURS_DEFAULT,
            read_active_locales: vec!["en".to_owned()],
            reference_freshness_minutes: REFERENCE_FRESHNESS_MINUTES_DEFAULT,
            watermark_skew_tolerance_minutes: WATERMARK_SKEW_MINUTES_DEFAULT,
            tripwire_max_overrides_per_30_days: TRIPWIRE_MAX_OVERRIDES_DEFAULT,
            breakglass_correction_enabled: false,
            require_broker: false,
            eol_enabled: false,
            // Absent, not a guess. See the field's own doc: an unset
            // default locale skips step 3 and the chain stays total.
            default_locale: String::new(),
            taxonomy_max_depth: TAXONOMY_MAX_DEPTH_DEFAULT,
            taxonomy_max_children_per_node: TAXONOMY_MAX_CHILDREN_DEFAULT,
            metadata_max_keys: METADATA_MAX_KEYS_DEFAULT,
            metadata_max_key_bytes: METADATA_MAX_KEY_BYTES_DEFAULT,
            metadata_max_value_bytes: METADATA_MAX_VALUE_BYTES_DEFAULT,
            attribute_values_max_per_patch: ATTRIBUTE_VALUES_MAX_PER_PATCH_DEFAULT,
            activation_claim_lease_secs: ACTIVATION_CLAIM_LEASE_SECS_DEFAULT,
            activation_attempt_budget: ACTIVATION_ATTEMPT_BUDGET_DEFAULT,
            retention_days_financial: RETENTION_DAYS_DEFAULT,
            retention_days_version: RETENTION_DAYS_DEFAULT,
            retention_days_audit: RETENTION_DAYS_DEFAULT,
            pseudonymization_age_days: PSEUDONYMIZATION_AGE_DAYS_DEFAULT,
            drill_cadence_hours: DRILL_CADENCE_HOURS_DEFAULT,
            // No default: see the field's own doc. An unconfigured drill
            // verifies nothing and says so, which is the honest state.
            drill_target_dsn: None,
            usage_type_resolver_timeout_ms: USAGE_TYPE_RESOLVER_TIMEOUT_MS_DEFAULT,
            breakglass_window_hours: BREAKGLASS_WINDOW_HOURS_DEFAULT,
            breakglass_review_sla_hours: BREAKGLASS_REVIEW_SLA_HOURS_DEFAULT,
            retirement_held_alert_hours: RETIREMENT_HELD_ALERT_HOURS_DEFAULT,
        }
    }
}

impl ProductsConfig {
    /// The configured window, clamped into
    /// `[IDEMPOTENCY_RETENTION_FLOOR_HOURS, IDEMPOTENCY_RETENTION_CEILING_HOURS]`
    /// — the value every expiry stamp is taken from.
    ///
    /// # Clamped, not refused, and why
    ///
    /// The design does not state a *validity predicate* on this field; it
    /// states a resolution — retention **is** `max(24h, max_freeze_timeout)`.
    /// A `max` is a clamp by construction, so clamping is the design's own
    /// arithmetic rather than a policy invented here to be lenient. Refusing
    /// the boot would take a whole registry offline over a value the design
    /// already says how to resolve, and would do it on the restart of a
    /// deployment that had been serving happily.
    ///
    /// The operator's mistake does not become invisible in exchange: the
    /// gear's `init` compares this answer with the configured field and logs
    /// the raise at `WARN`, naming both numbers. What must never happen is
    /// the third option — carrying a `0` through to
    /// `crate::api::rest`'s `idempotency_expiry`, which stamps
    /// `expires_at == now`, so the very next request reads the key as expired,
    /// takes it over, and runs the guarded mutation a second time under one
    /// key. That is at-most-once off with no boot failure and no log at all,
    /// and it is the outcome both other options exist to rule out.
    #[must_use]
    pub fn resolved_idempotency_retention_hours(&self) -> u32 {
        // The design's floor is `max(24h, max_freeze_timeout)`
        // (`inst-fd-idem-retention`, C6); the second operand's source is the
        // catalog-version feature's export — this field, per-deployment
        // (P-D-84 arm 5). `validate` holds the floor at or under the
        // ceiling, so the clamp's `min <= max` precondition is a boot
        // invariant rather than a runtime hope.
        self.idempotency_retention_hours.clamp(
            IDEMPOTENCY_RETENTION_FLOOR_HOURS.max(self.freeze_timeout_hours),
            IDEMPOTENCY_RETENTION_CEILING_HOURS,
        )
    }

    /// The freshness threshold as a `Duration` — the export
    /// `features/lifecycle.md` §7 row 8 needs, since `04`'s activation
    /// runner polls a deferred flip on exactly this interval (no event
    /// exists for a watermark, which is state rather than history).
    /// `usage_type_resolver_timeout_ms` as a `Duration` — the bound on one
    /// collector call at publish (P-D-121 row 12, interim 2000).
    #[must_use]
    pub fn usage_type_resolver_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_millis(u64::from(self.usage_type_resolver_timeout_ms))
    }

    #[must_use]
    pub fn reference_freshness(&self) -> std::time::Duration {
        std::time::Duration::from_secs(u64::from(self.reference_freshness_minutes) * 60)
    }

    /// The skew tolerance as a `Duration`, the watermark door's own bound.
    #[must_use]
    pub fn watermark_skew_tolerance(&self) -> std::time::Duration {
        std::time::Duration::from_secs(u64::from(self.watermark_skew_tolerance_minutes) * 60)
    }

    /// Boot-time validation (P-D-84 arm 6): a `freeze_timeout_hours` above
    /// the ten-year retention ceiling would invert the retention clamp into
    /// a panic, so it is refused before anything runs — alongside the
    /// zero-value checks whose fields admit no working zero.
    ///
    /// # Errors
    ///
    /// A sentence naming the field, the configured value and the ceiling.
    pub fn validate(&self) -> Result<(), String> {
        if self.reference_freshness_minutes == 0 {
            return Err(
                "reference_freshness_minutes = 0 makes every watermark stale on arrival".to_owned(),
            );
        }
        for (name, value) in [
            ("taxonomy_max_depth", self.taxonomy_max_depth),
            (
                "taxonomy_max_children_per_node",
                self.taxonomy_max_children_per_node,
            ),
            ("metadata_max_keys", self.metadata_max_keys),
            ("metadata_max_key_bytes", self.metadata_max_key_bytes),
            ("metadata_max_value_bytes", self.metadata_max_value_bytes),
            (
                "attribute_values_max_per_patch",
                self.attribute_values_max_per_patch,
            ),
            (
                "activation_claim_lease_secs",
                self.activation_claim_lease_secs,
            ),
            ("activation_attempt_budget", self.activation_attempt_budget),
            ("retention_days_financial", self.retention_days_financial),
            ("retention_days_version", self.retention_days_version),
            ("retention_days_audit", self.retention_days_audit),
            ("pseudonymization_age_days", self.pseudonymization_age_days),
            ("drill_cadence_hours", self.drill_cadence_hours),
            (
                "usage_type_resolver_timeout_ms",
                self.usage_type_resolver_timeout_ms,
            ),
            ("breakglass_window_hours", self.breakglass_window_hours),
            (
                "breakglass_review_sla_hours",
                self.breakglass_review_sla_hours,
            ),
            (
                "retirement_held_alert_hours",
                self.retirement_held_alert_hours,
            ),
        ] {
            if value == 0 {
                return Err(format!(
                    "{name} = 0 admits nothing at all: a cap of zero refuses the first category or \
                     the first metadata key, so the door it guards can never succeed"
                ));
            }
        }
        if self.bulk_max_rows_per_batch == 0 {
            return Err("bulk_max_rows_per_batch = 0 admits no batch at all".to_owned());
        }
        if self.bulk_max_concurrent_batches_per_tenant == 0 {
            return Err(
                "bulk_max_concurrent_batches_per_tenant = 0 admits no batch at all".to_owned(),
            );
        }
        if self.bulk_batch_ttl_hours == 0 {
            return Err(
                "bulk_batch_ttl_hours = 0 would abandon every reported batch on the next tick"
                    .to_owned(),
            );
        }
        if self.read_path_qps_ceiling == 0 {
            return Err("read_path_qps_ceiling = 0 sheds every read".to_owned());
        }
        if self.read_poison_retry_ceiling == 0 || self.read_dashboard_poll_secs == 0 {
            return Err(
                "read_poison_retry_ceiling and read_dashboard_poll_secs must be at least 1"
                    .to_owned(),
            );
        }
        if self.read_active_locales.is_empty()
            || self
                .read_active_locales
                .iter()
                .any(|locale| locale.trim().is_empty())
        {
            return Err(
                "read_active_locales must name at least one non-blank locale (P-D-126 row 2)"
                    .to_owned(),
            );
        }
        if self
            .drill_target_dsn
            .as_ref()
            .is_some_and(|dsn| dsn.trim().is_empty())
        {
            return Err(
                "drill_target_dsn is present but blank: an operator who meant to configure a \
                 restore target would otherwise get a decade of `no_target` warnings that read \
                 as a deployment choice rather than as a typo"
                    .to_owned(),
            );
        }
        if !self.default_locale.is_empty() && self.default_locale != self.default_locale.trim() {
            return Err(format!(
                "default_locale = `{}` carries leading or trailing whitespace; no stored \
                 attribute-value coordinate can match it, so step 3 of the fallback chain \
                 would silently never fire",
                self.default_locale
            ));
        }
        if self.freeze_timeout_hours > IDEMPOTENCY_RETENTION_CEILING_HOURS {
            return Err(format!(
                "freeze_timeout_hours = {} exceeds the retention ceiling of {} hours; \
                 the idempotency retention clamp would be inverted",
                self.freeze_timeout_hours, IDEMPOTENCY_RETENTION_CEILING_HOURS
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod config_tests;
