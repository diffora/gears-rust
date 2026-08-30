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
}

impl Default for ProductsConfig {
    fn default() -> Self {
        Self {
            idempotency_retention_hours: IDEMPOTENCY_RETENTION_FLOOR_HOURS,
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
        self.idempotency_retention_hours.clamp(
            IDEMPOTENCY_RETENTION_FLOOR_HOURS,
            IDEMPOTENCY_RETENTION_CEILING_HOURS,
        )
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod config_tests;
