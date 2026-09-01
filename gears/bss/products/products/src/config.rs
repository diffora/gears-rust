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

/// The default per-tenant concurrent-batch ceiling. Small on purpose: a
/// batch is an operator act with an approval attached, and a tenant holding
/// many at once is the accident the ceiling exists to catch.
pub const BULK_MAX_CONCURRENT_DEFAULT: u32 = 5;

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
}

impl Default for ProductsConfig {
    fn default() -> Self {
        Self {
            idempotency_retention_hours: IDEMPOTENCY_RETENTION_FLOOR_HOURS,
            freeze_timeout_hours: IDEMPOTENCY_RETENTION_FLOOR_HOURS,
            bulk_max_rows_per_batch: BULK_MAX_ROWS_DEFAULT,
            bulk_max_concurrent_batches_per_tenant: BULK_MAX_CONCURRENT_DEFAULT,
            require_broker: false,
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

    /// Boot-time validation (P-D-84 arm 6): a `freeze_timeout_hours` above
    /// the ten-year retention ceiling would invert the clamp above into a
    /// panic, so it is refused before anything runs.
    ///
    /// # Errors
    ///
    /// A sentence naming the field, the configured value and the ceiling.
    pub fn validate(&self) -> Result<(), String> {
        if self.bulk_max_rows_per_batch == 0 {
            return Err("bulk_max_rows_per_batch = 0 admits no batch at all".to_owned());
        }
        if self.bulk_max_concurrent_batches_per_tenant == 0 {
            return Err(
                "bulk_max_concurrent_batches_per_tenant = 0 admits no batch at all".to_owned(),
            );
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
