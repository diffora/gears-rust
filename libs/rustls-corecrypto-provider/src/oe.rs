//! Runtime Operational Environment (OE) validation for the macOS FIPS posture.
//!
//! A FIPS 140-3 claim is only valid when the running OS version lies inside
//! the Operational Environment listed on the active Apple corecrypto CMVP
//! certificate. This module reads the macOS product version at startup and
//! compares it against a major-version whitelist synchronised with the
//! "Compliance caveat" table in this crate's README.
//!
//! ## Runtime FIPS witness
//!
//! [`fips_witness_ok`] is the one entry point every `fips()` impl across
//! this crate delegates to. It returns `true` iff (a) the running macOS
//! major is inside [`SUPPORTED_OE_MACOS_MAJOR`], or (b) the override
//! env-var [`OE_OVERRIDE_ENV`] is set. Mirrors `rustls-cng-crypto`'s
//! `crate::fips::enabled()` posture on Windows — there is no startup
//! panic; an OE mismatch produces `fips() == false` everywhere (and a
//! single `tracing::warn!`), so downstream `ClientConfig::fips()` /
//! `ServerConfig::fips()` correctly report the runtime witness rather
//! than the design intent. The witness is cached process-wide via
//! [`std::sync::OnceLock`] so we pay one `sysctlbyname` call per process.
//!
//! See PRD §8.3 "Operational Environment validation at startup" and PRD §10
//! TODO-7 for the long-term automation plan.

use std::fmt;
use std::sync::OnceLock;

/// macOS major versions whose patch releases lie inside an active Apple
/// corecrypto CMVP certificate's Operational Environment.
///
/// Synchronise with the README "Compliance caveat" table. Major-only
/// matching is intentional: Apple's CMVP submissions cover the entire
/// patch family of a given macOS major (`13.x`, `14.x`, `15.x`), so a
/// patch-version bump on the deployment host does not invalidate the
/// claim — only a major-version bump does.
pub const SUPPORTED_OE_MACOS_MAJOR: &[u32] = &[12, 13, 14, 15, 26];

/// Environment variable that, when set to a non-empty value other than
/// `"0"`, forces [`fips_witness_ok`] to return `true` on a macOS major
/// outside [`SUPPORTED_OE_MACOS_MAJOR`]. Intended for CI on pre-release
/// macOS during the window between a major-version release and the
/// publication of Apple's next corecrypto CMVP submission — never for
/// production.
///
/// **Do not set in production.** Setting this in production asserts a
/// FIPS claim on an OS version that has not been validated.
///
/// (Note: prior versions of this crate panicked under `--features fips`
/// on OE mismatch and treated the env-var as a "downgrade to warning"
/// switch. The crate no longer panics — the override now flips the
/// witness from `false` to `true`. See the README "Runtime FIPS witness"
/// section.)
pub const OE_OVERRIDE_ENV: &str = "CYBERWARE_FIPS_OE_OVERRIDE";

/// Outcome of OE validation. Distinct from `rustls::Error` because this
/// is a deployment-environment problem, not a TLS-layer problem.
#[derive(Debug, Clone)]
pub enum OeError {
    /// `kern.osproductversion` reports a major version outside the
    /// supported whitelist.
    UnsupportedVersion {
        detected: (u32, u32),
        supported: &'static [u32],
    },
    /// `sysctlbyname` failed (e.g. EPERM in a sandbox) — we could not
    /// determine the running macOS version.
    SysctlFailed(String),
    /// `kern.osproductversion` returned an output we could not parse
    /// as `MAJOR.MINOR[.PATCH]`.
    ParseFailed(String),
}

impl fmt::Display for OeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OeError::UnsupportedVersion {
                detected,
                supported,
            } => write!(
                f,
                "running macOS {}.{} is not inside the Apple corecrypto CMVP cert OE \
                 (supported majors: {:?}); the runtime FIPS witness will report false \
                 and so will CryptoProvider::fips() / ClientConfig::fips(). \
                 Verify cert coverage at \
                 https://csrc.nist.gov/projects/cryptographic-module-validation-program. \
                 Set {OE_OVERRIDE_ENV}=1 to force the witness to true on an unvalidated \
                 macOS major (CI / pre-release only — never in production).",
                detected.0, detected.1, supported
            ),
            OeError::SysctlFailed(reason) => {
                write!(f, "kern.osproductversion sysctl failed: {reason}")
            }
            OeError::ParseFailed(s) => {
                write!(f, "could not parse macOS version string {s:?}")
            }
        }
    }
}

impl std::error::Error for OeError {}

/// Returns `true` if the user has explicitly opted out of the fail-closed
/// gate via [`OE_OVERRIDE_ENV`]. Treats `""` and `"0"` as not-set.
pub fn override_enabled() -> bool {
    match std::env::var(OE_OVERRIDE_ENV) {
        Ok(v) => !v.is_empty() && v != "0",
        Err(_) => false,
    }
}

/// Read the running macOS product version (e.g. "14.5.1") via
/// `sysctlbyname("kern.osproductversion", ...)` and parse the leading
/// `MAJOR.MINOR`.
///
/// Implementation note: we deliberately use the same syscall surface as
/// every other macOS process that asks the same question (`sw_vers`,
/// `Foundation.NSProcessInfo`, etc.). The string is the OS's
/// authoritative product-version, not the kernel version
/// (`kern.osrelease`, which numbers Darwin releases differently).
pub fn current_macos_version() -> Result<(u32, u32), OeError> {
    let raw = read_sysctl_string("kern.osproductversion")?;
    parse_version(&raw)
}

fn parse_version(s: &str) -> Result<(u32, u32), OeError> {
    let mut parts = s.split('.');
    let major = parts
        .next()
        .and_then(|p| p.parse::<u32>().ok())
        .ok_or_else(|| OeError::ParseFailed(s.to_owned()))?;
    // If a second segment exists it must parse as u32 -- silently
    // accepting non-numeric minor (e.g. "14.beta" -> minor=0) would
    // hide a corrupted sysctl reply.
    let minor = match parts.next() {
        Some(p) => p
            .parse::<u32>()
            .map_err(|_| OeError::ParseFailed(s.to_owned()))?,
        None => 0,
    };
    Ok((major, minor))
}

fn read_sysctl_string(name: &str) -> Result<String, OeError> {
    use std::ffi::CString;
    use std::os::raw::{c_int, c_void};

    // `libc::sysctlbyname` is transitively present in our dependency
    // graph (via `core-foundation`'s libc dep). We declare the extern
    // here rather than adding `libc` as a direct dep — single use site,
    // single sig, no risk of API drift.
    unsafe extern "C" {
        fn sysctlbyname(
            name: *const std::os::raw::c_char,
            oldp: *mut c_void,
            oldlenp: *mut usize,
            newp: *mut c_void,
            newlen: usize,
        ) -> c_int;
    }

    let cname = CString::new(name).map_err(|e| OeError::SysctlFailed(e.to_string()))?;

    // First call: query required buffer size.
    let mut len: usize = 0;
    // SAFETY: `cname` outlives the call; `oldp` is NULL so the kernel
    // only writes into `len`. Sole side effect: writes a usize.
    let rc = unsafe {
        sysctlbyname(
            cname.as_ptr(),
            std::ptr::null_mut(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return Err(OeError::SysctlFailed(format!(
            "size query for {name} returned errno={}",
            std::io::Error::last_os_error()
        )));
    }
    if len == 0 {
        return Err(OeError::SysctlFailed(format!(
            "{name} reported zero-length value"
        )));
    }

    let mut buf: Vec<u8> = vec![0; len];
    // SAFETY: same as above; `buf.len() == len` so the kernel will not
    // overrun. After the call we trim the trailing NUL.
    let rc = unsafe {
        sysctlbyname(
            cname.as_ptr(),
            buf.as_mut_ptr() as *mut c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return Err(OeError::SysctlFailed(format!(
            "value fetch for {name} returned errno={}",
            std::io::Error::last_os_error()
        )));
    }

    // `len` after the second call is the byte count actually written,
    // including the trailing NUL terminator that the kernel appends.
    if let Some(&0) = buf.get(len.saturating_sub(1)) {
        buf.truncate(len.saturating_sub(1));
    } else {
        buf.truncate(len);
    }

    String::from_utf8(buf).map_err(|e| OeError::SysctlFailed(format!("non-UTF-8 reply: {e}")))
}

/// Validate that the current macOS major version is inside
/// [`SUPPORTED_OE_MACOS_MAJOR`].
///
/// Returns `Ok(())` on match, `Err(OeError::*)` on mismatch or on any
/// failure to determine the OS version.
pub fn validate_oe() -> Result<(), OeError> {
    let (major, minor) = current_macos_version()?;
    if SUPPORTED_OE_MACOS_MAJOR.contains(&major) {
        Ok(())
    } else {
        Err(OeError::UnsupportedVersion {
            detected: (major, minor),
            supported: SUPPORTED_OE_MACOS_MAJOR,
        })
    }
}

/// Process-wide cached witness for the runtime FIPS posture. Populated on
/// first call to [`fips_witness_ok`] and never mutated afterwards.
///
/// ## Test-isolation hazard
///
/// `cargo test` shares one process across the entire test binary, and many
/// tests in this crate transitively prime this slot — any call into
/// `default_provider()` / `fips_provider()` reaches `fips_witness_ok` and
/// populates `FIPS_WITNESS` for the rest of the process. Once set, the
/// `OnceLock` does not re-evaluate; subsequent env-var manipulations
/// (e.g. via `temp_env` in `override_treats_empty_and_zero_as_unset`)
/// affect `override_enabled()` directly but **not** `fips_witness_ok`.
///
/// Tests that need to verify witness behaviour in non-default states
/// must exercise the pure-function policy
/// [`compute_fips_witness`] directly, not `fips_witness_ok` — see the
/// `compute_fips_witness_*` cases in the module's test section. A
/// `reset_witness_for_tests()` hook was intentionally not added: it
/// would require `unsafe` mutation of this `static OnceLock` and would
/// be unsound under `cargo test --test-threads > 1`.
static FIPS_WITNESS: OnceLock<bool> = OnceLock::new();

/// Pure-function policy: given the outcome of an OE check and whether the
/// override env-var is set, return whether the runtime FIPS witness
/// should report `true`. Extracted from [`fips_witness_ok`] so the policy
/// is unit-testable without touching the global cache.
fn compute_fips_witness(result: &Result<(), OeError>, override_set: bool) -> bool {
    result.is_ok() || override_set
}

/// Runtime FIPS witness — the single entry point every `fips()` impl in
/// this crate delegates to.
///
/// Returns `true` iff (a) the running macOS major is inside
/// [`SUPPORTED_OE_MACOS_MAJOR`], or (b) the override env-var
/// [`OE_OVERRIDE_ENV`] is set (intended for CI on pre-release macOS).
/// Otherwise returns `false` and `ClientConfig::fips()` /
/// `ServerConfig::fips()` correctly report the runtime witness rather
/// than design intent.
///
/// The witness is cached process-wide via [`OnceLock`] — one
/// `sysctlbyname` call per process. On the first call where the OE check
/// fails AND the override is not set, a single `tracing::warn!` is
/// emitted for telemetry.
///
/// Mirrors `rustls-cng-crypto`'s `crate::fips::enabled()` posture: no
/// startup panic; failure surfaces as `fips() == false` everywhere.
pub fn fips_witness_ok() -> bool {
    *FIPS_WITNESS.get_or_init(|| {
        let result = validate_oe();
        let override_set = override_enabled();
        let ok = compute_fips_witness(&result, override_set);
        if !ok {
            if let Err(err) = &result {
                tracing::warn!(
                    error = %err,
                    "FIPS witness: OE-validation failed; CryptoProvider::fips() will report \
                     false on this host (no panic). Set {OE_OVERRIDE_ENV}=1 to force the \
                     witness to true on pre-release macOS in CI only — never in production."
                );
            }
        } else if let Err(err) = &result {
            // OK because override was set — log so operators are not
            // surprised that fips() returns true on an unvalidated OE.
            tracing::warn!(
                error = %err,
                "FIPS witness: OE-validation failed but {OE_OVERRIDE_ENV} is set; \
                 reporting fips() == true on an unvalidated macOS version. \
                 This setting must not be used in production."
            );
        }
        ok
    })
}

#[cfg(test)]
#[path = "oe_tests.rs"]
mod tests;
