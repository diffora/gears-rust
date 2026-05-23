//! HMAC-SHA-256 / HMAC-SHA-384 implementations of [`rustls::crypto::hmac::Hmac`].
//!
//! Backed by CommonCrypto's `CCHmac*` family. We keep a `CCHmacContext`
//! initialised with the key once at `with_key`-time; on each `sign` we
//! memcpy-clone it (the struct is POD with the key state baked in), then
//! Update + Final on the clone.

use core::ffi::c_void;
use core::mem::MaybeUninit;

use rustls::crypto::hmac::{Hmac, Key, Tag};
use zeroize::Zeroize;

use crate::ffi::commoncrypto as cc;

/// Maximum digest size across every HMAC algorithm this module wires up.
/// Sized to SHA-512 (64 bytes) so a future contributor adding
/// `kCCHmacAlgSHA512` to [`HmacSha256::with_key`] / [`HmacSha384::with_key`]
/// cannot silently overflow the per-sign stack buffer.
///
/// **Invariant** (pinned by the `const _: () = assert!(...)` below): the
/// constant must be greater than or equal to every `tag_len` reachable
/// from [`HmacKey::new`]'s call sites.
const MAX_HMAC_DIGEST: usize = 64;

// SHA-256 = 32 bytes, SHA-384 = 48 bytes; both must fit. Compile-time
// guard so an accidental shrink of MAX_HMAC_DIGEST fails the build, not
// the runtime. Test-gap #7.
const _: () = assert!(MAX_HMAC_DIGEST >= 32);
const _: () = assert!(MAX_HMAC_DIGEST >= 48);
const _: () = assert!(MAX_HMAC_DIGEST >= 64); // anticipates SHA-512 addition.

// =========================================================================
// HMAC-SHA-256
// =========================================================================

#[derive(Debug)]
pub struct HmacSha256;

pub static HMAC_SHA256: HmacSha256 = HmacSha256;

impl Hmac for HmacSha256 {
    fn with_key(&self, key: &[u8]) -> Box<dyn Key> {
        Box::new(HmacKey::new(cc::kCCHmacAlgSHA256, key, 32))
    }

    fn hash_output_len(&self) -> usize {
        32
    }

    fn fips(&self) -> bool {
        // Runtime witness — see [`crate::oe::fips_witness_ok`].
        crate::oe::fips_witness_ok()
    }
}

// =========================================================================
// HMAC-SHA-384
// =========================================================================

#[derive(Debug)]
pub struct HmacSha384;

pub static HMAC_SHA384: HmacSha384 = HmacSha384;

impl Hmac for HmacSha384 {
    fn with_key(&self, key: &[u8]) -> Box<dyn Key> {
        Box::new(HmacKey::new(cc::kCCHmacAlgSHA384, key, 48))
    }

    fn hash_output_len(&self) -> usize {
        48
    }

    fn fips(&self) -> bool {
        // Runtime witness — see [`crate::oe::fips_witness_ok`].
        crate::oe::fips_witness_ok()
    }
}

// =========================================================================
// shared key wrapper
// =========================================================================

/// A keyed HMAC context.
///
/// The C-side `CCHmacContext` is a fixed-size POD; we memcpy-clone it per
/// `sign` to support multiple signing operations with the same key.
///
/// `Send + Sync` are inferred from the fields — `CCHmacAlgorithm` is a
/// `u32`, `CCHmacContext` is `[u32; 96]`, `tag_len` is a `usize`; all
/// auto-implement `Send + Sync`. `sign_concat` takes `&self` and clones
/// `self.template` byte-for-byte into a local, so concurrent calls
/// cannot race on the C-side state; CommonCrypto's HMAC primitives do
/// not touch process-wide state.
struct HmacKey {
    algorithm: cc::CCHmacAlgorithm,
    template: cc::CCHmacContext,
    tag_len: usize,
}

impl HmacKey {
    fn new(algorithm: cc::CCHmacAlgorithm, key: &[u8], tag_len: usize) -> Self {
        let mut template = MaybeUninit::<cc::CCHmacContext>::uninit();
        // SAFETY: `CCHmacInit` initialises every field of the context. Passing
        // empty key (`key.len() == 0`) is documented to derive a zero-length
        // key, which is sound; rustls never invokes that path in practice.
        unsafe {
            cc::CCHmacInit(
                template.as_mut_ptr(),
                algorithm,
                key.as_ptr() as *const c_void,
                key.len(),
            );
            Self {
                algorithm,
                template: template.assume_init(),
                tag_len,
            }
        }
    }
}

impl Drop for HmacKey {
    fn drop(&mut self) {
        // CCHmacContext's `ctx: [u32; 96]` field is exposed; zero it before
        // release. Defense-in-depth — Apple may also zeroize internally but
        // we don't rely on that.
        self.template.ctx.zeroize();
    }
}

impl Key for HmacKey {
    fn sign_concat(&self, first: &[u8], middle: &[&[u8]], last: &[u8]) -> Tag {
        // memcpy-clone the keyed template so we can call Update+Final without
        // disturbing it (the trait permits repeated signing with one Key).
        let mut ctx = self.template.clone();

        let _ = self.algorithm; // silence unused-field lint; kept for diagnostics
        // Sized to `MAX_HMAC_DIGEST` (SHA-512 width) so a future
        // SHA-512 contributor cannot silently overflow this buffer.
        // The compile-time `const _: () = assert!(MAX_HMAC_DIGEST >= 48)`
        // above guarantees this is wide enough for every algorithm
        // currently registered.
        let mut out = [0u8; MAX_HMAC_DIGEST];
        // `assert!` (not `debug_assert!`) — the cost is one cmp/jmp per
        // HMAC, and the buffer-overflow class is gone for good even if a
        // future caller passes a > 64-byte digest. The compile-time
        // `const _: () = assert!(MAX_HMAC_DIGEST >= …)` guards above make
        // this unreachable for the registered algorithms today, but
        // belt-and-braces is cheap.
        assert!(
            self.tag_len <= MAX_HMAC_DIGEST,
            "HMAC tag_len {} exceeds MAX_HMAC_DIGEST {MAX_HMAC_DIGEST}",
            self.tag_len
        );

        // SAFETY: ctx is fully initialised; all data pointers are valid for
        // the given lengths; out is sized to fit the largest tag.
        unsafe {
            if !first.is_empty() {
                cc::CCHmacUpdate(&mut ctx, first.as_ptr() as *const c_void, first.len());
            }
            for chunk in middle {
                if chunk.is_empty() {
                    continue;
                }
                cc::CCHmacUpdate(&mut ctx, chunk.as_ptr() as *const c_void, chunk.len());
            }
            if !last.is_empty() {
                cc::CCHmacUpdate(&mut ctx, last.as_ptr() as *const c_void, last.len());
            }
            cc::CCHmacFinal(&mut ctx, out.as_mut_ptr() as *mut c_void);
        }

        // Zero the cloned context after use.
        ctx.ctx.zeroize();

        let tag = Tag::new(&out[..self.tag_len]);
        // Wipe our local MAC buffer — the constructed `Tag` owns its own copy.
        out.zeroize();
        tag
    }

    fn tag_len(&self) -> usize {
        self.tag_len
    }
}

#[cfg(test)]
#[path = "hmac_tests.rs"]
mod tests;
