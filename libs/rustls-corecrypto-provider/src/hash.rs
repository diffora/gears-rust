//! SHA-256 and SHA-384 implementations of [`rustls::crypto::hash::Hash`].
//!
//! Backed by CommonCrypto's `CC_SHA256_*` / `CC_SHA384_*` functions. The
//! `Context` state is the C-side `CC_SHA256_CTX` / `CC_SHA512_CTX` struct
//! (plain POD), which lets us implement `fork` (snapshot) by trivial memcpy.
//!
//! **`u32` length-parameter caveat.** CommonCrypto's `CC_SHA*_Update`
//! takes the input length as a `CC_LONG` (= `u32`). A single Rust slice
//! can be up to `usize::MAX` bytes — on 64-bit platforms that exceeds the
//! C ABI's 4 GiB limit. To avoid silent truncation we chunk every call
//! into ≤ `u32::MAX`-byte sub-calls. Each iteration is bytes-equivalent
//! to one big `Update` because SHA-2 is a length-extension-safe Merkle-
//! Damgård construction.

use core::ffi::c_void;
use core::mem::MaybeUninit;

use rustls::crypto::hash::{Context, Hash, HashAlgorithm, Output};

use crate::ffi::commoncrypto as cc;

/// Per-call chunk limit for `CC_SHA*_Update`: the C API takes a `u32`
/// length, so longer slices must be fed in pieces. Using `u32::MAX`
/// directly means at most one extra FFI hop per 4 GiB.
const CC_UPDATE_MAX: usize = u32::MAX as usize;

// =========================================================================
// SHA-256
// =========================================================================

#[derive(Debug)]
pub struct Sha256;

pub static SHA256: Sha256 = Sha256;

impl Hash for Sha256 {
    fn start(&self) -> Box<dyn Context> {
        let mut ctx = MaybeUninit::<cc::CC_SHA256_CTX>::uninit();
        // SAFETY: `CC_SHA256_Init` initialises every field of the context.
        unsafe {
            assert_eq!(cc::CC_SHA256_Init(ctx.as_mut_ptr()), 1);
            Box::new(Sha256Context {
                ctx: ctx.assume_init(),
            })
        }
    }

    fn hash(&self, data: &[u8]) -> Output {
        // For oneshot we route through Init+Update+Final so the chunking
        // logic is shared with the streaming path. The single-call
        // `CC_SHA256(data, len, out)` would silently truncate `len` to
        // `u32` on inputs ≥ 4 GiB.
        let mut ctx = MaybeUninit::<cc::CC_SHA256_CTX>::uninit();
        let mut out = [0u8; cc::CC_SHA256_DIGEST_LENGTH];
        // SAFETY: `CC_SHA256_Init` initialises every field; we then call
        // chunked Update via `sha256_update_all` (each FFI call is `u32`-
        // bounded); Final consumes the now-initialised ctx.
        unsafe {
            assert_eq!(cc::CC_SHA256_Init(ctx.as_mut_ptr()), 1);
            sha256_update_all(ctx.as_mut_ptr(), data);
            cc::CC_SHA256_Final(out.as_mut_ptr(), ctx.as_mut_ptr());
        }
        Output::new(&out)
    }

    fn algorithm(&self) -> HashAlgorithm {
        HashAlgorithm::SHA256
    }

    fn output_len(&self) -> usize {
        cc::CC_SHA256_DIGEST_LENGTH
    }

    fn fips(&self) -> bool {
        // Runtime witness — see [`crate::oe::fips_witness_ok`].
        crate::oe::fips_witness_ok()
    }
}

/// Feed `data` to `CC_SHA256_Update` in ≤ `u32::MAX`-byte chunks. Single-
/// call usage is the common case (chunking adds zero extra calls for
/// inputs ≤ 4 GiB - 1).
///
/// # Safety
///
/// `ctx` must be a fully-initialised, non-null `CC_SHA256_CTX` pointer.
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn sha256_update_all(ctx: *mut cc::CC_SHA256_CTX, mut data: &[u8]) {
    while !data.is_empty() {
        let take = data.len().min(CC_UPDATE_MAX);
        cc::CC_SHA256_Update(ctx, data.as_ptr() as *const c_void, take as u32);
        data = &data[take..];
    }
}

#[derive(Clone)]
struct Sha256Context {
    ctx: cc::CC_SHA256_CTX,
}

impl Context for Sha256Context {
    fn fork_finish(&self) -> Output {
        let mut clone = self.clone();
        let mut out = [0u8; cc::CC_SHA256_DIGEST_LENGTH];
        // SAFETY: `clone.ctx` is fully initialised; `out` is correctly sized.
        unsafe {
            cc::CC_SHA256_Final(out.as_mut_ptr(), &mut clone.ctx);
        }
        Output::new(&out)
    }

    fn fork(&self) -> Box<dyn Context> {
        Box::new(self.clone())
    }

    fn finish(mut self: Box<Self>) -> Output {
        let mut out = [0u8; cc::CC_SHA256_DIGEST_LENGTH];
        // SAFETY: same as `fork_finish` but consuming.
        unsafe {
            cc::CC_SHA256_Final(out.as_mut_ptr(), &mut self.ctx);
        }
        Output::new(&out)
    }

    fn update(&mut self, data: &[u8]) {
        // SAFETY: `self.ctx` is initialised in `start`. Chunking guards
        // against `data.len() > u32::MAX` — CC_SHA256_Update's length
        // parameter is `u32`.
        unsafe {
            sha256_update_all(&mut self.ctx, data);
        }
    }
}

// =========================================================================
// SHA-384
// =========================================================================

#[derive(Debug)]
pub struct Sha384;

pub static SHA384: Sha384 = Sha384;

impl Hash for Sha384 {
    fn start(&self) -> Box<dyn Context> {
        let mut ctx = MaybeUninit::<cc::CC_SHA512_CTX>::uninit();
        // SAFETY: `CC_SHA384_Init` initialises every field of the context.
        unsafe {
            assert_eq!(cc::CC_SHA384_Init(ctx.as_mut_ptr()), 1);
            Box::new(Sha384Context {
                ctx: ctx.assume_init(),
            })
        }
    }

    fn hash(&self, data: &[u8]) -> Output {
        let mut ctx = MaybeUninit::<cc::CC_SHA512_CTX>::uninit();
        let mut out = [0u8; cc::CC_SHA384_DIGEST_LENGTH];
        // SAFETY: parallel to Sha256::hash — Init + chunked Update + Final.
        unsafe {
            assert_eq!(cc::CC_SHA384_Init(ctx.as_mut_ptr()), 1);
            sha384_update_all(ctx.as_mut_ptr(), data);
            cc::CC_SHA384_Final(out.as_mut_ptr(), ctx.as_mut_ptr());
        }
        Output::new(&out)
    }

    fn algorithm(&self) -> HashAlgorithm {
        HashAlgorithm::SHA384
    }

    fn output_len(&self) -> usize {
        cc::CC_SHA384_DIGEST_LENGTH
    }

    fn fips(&self) -> bool {
        // Runtime witness — see [`crate::oe::fips_witness_ok`].
        crate::oe::fips_witness_ok()
    }
}

/// Same chunking helper as `sha256_update_all`, for SHA-384.
///
/// # Safety
///
/// `ctx` must be a fully-initialised, non-null `CC_SHA512_CTX` pointer.
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn sha384_update_all(ctx: *mut cc::CC_SHA512_CTX, mut data: &[u8]) {
    while !data.is_empty() {
        let take = data.len().min(CC_UPDATE_MAX);
        cc::CC_SHA384_Update(ctx, data.as_ptr() as *const c_void, take as u32);
        data = &data[take..];
    }
}

#[derive(Clone)]
struct Sha384Context {
    ctx: cc::CC_SHA512_CTX,
}

impl Context for Sha384Context {
    fn fork_finish(&self) -> Output {
        let mut clone = self.clone();
        let mut out = [0u8; cc::CC_SHA384_DIGEST_LENGTH];
        unsafe {
            cc::CC_SHA384_Final(out.as_mut_ptr(), &mut clone.ctx);
        }
        Output::new(&out)
    }

    fn fork(&self) -> Box<dyn Context> {
        Box::new(self.clone())
    }

    fn finish(mut self: Box<Self>) -> Output {
        let mut out = [0u8; cc::CC_SHA384_DIGEST_LENGTH];
        unsafe {
            cc::CC_SHA384_Final(out.as_mut_ptr(), &mut self.ctx);
        }
        Output::new(&out)
    }

    fn update(&mut self, data: &[u8]) {
        // SAFETY: `self.ctx` is initialised in `start`. Chunking guards
        // against `data.len() > u32::MAX`.
        unsafe {
            sha384_update_all(&mut self.ctx, data);
        }
    }
}

#[cfg(test)]
#[path = "hash_tests.rs"]
mod tests;
