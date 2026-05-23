//! HKDF (RFC 5869) implementation of [`rustls::crypto::tls13::Hkdf`].
//!
//! Pure Rust on top of our HMAC primitives — RFC 5869 specifies HKDF entirely
//! in terms of HMAC, so no additional FFI is needed. The PRK and intermediate
//! `T(i)` values are stored on the stack, zeroized on drop.

use rustls::crypto::hmac::Hmac;
use rustls::crypto::hmac::Tag;
use rustls::crypto::tls13::{Hkdf, HkdfExpander, OkmBlock, OutputLengthError};
use zeroize::{Zeroize, Zeroizing};

use crate::hmac::{HMAC_SHA256, HMAC_SHA384};

// =========================================================================
// HKDF-SHA-256
// =========================================================================

#[derive(Debug)]
pub struct HkdfSha256;

pub static HKDF_SHA256: HkdfSha256 = HkdfSha256;

impl Hkdf for HkdfSha256 {
    fn extract_from_zero_ikm(&self, salt: Option<&[u8]>) -> Box<dyn HkdfExpander> {
        extract::<32>(&HMAC_SHA256, salt, &[0u8; 32])
    }

    fn extract_from_secret(&self, salt: Option<&[u8]>, secret: &[u8]) -> Box<dyn HkdfExpander> {
        extract::<32>(&HMAC_SHA256, salt, secret)
    }

    fn expander_for_okm(&self, okm: &OkmBlock) -> Box<dyn HkdfExpander> {
        Box::new(Sha256Expander {
            prk: copy_prk::<32>(okm.as_ref()),
        })
    }

    fn hmac_sign(&self, key: &OkmBlock, message: &[u8]) -> Tag {
        HMAC_SHA256.with_key(key.as_ref()).sign(&[message])
    }

    fn fips(&self) -> bool {
        // Runtime witness — see [`crate::oe::fips_witness_ok`].
        crate::oe::fips_witness_ok()
    }
}

// =========================================================================
// HKDF-SHA-384
// =========================================================================

#[derive(Debug)]
pub struct HkdfSha384;

pub static HKDF_SHA384: HkdfSha384 = HkdfSha384;

impl Hkdf for HkdfSha384 {
    fn extract_from_zero_ikm(&self, salt: Option<&[u8]>) -> Box<dyn HkdfExpander> {
        extract::<48>(&HMAC_SHA384, salt, &[0u8; 48])
    }

    fn extract_from_secret(&self, salt: Option<&[u8]>, secret: &[u8]) -> Box<dyn HkdfExpander> {
        extract::<48>(&HMAC_SHA384, salt, secret)
    }

    fn expander_for_okm(&self, okm: &OkmBlock) -> Box<dyn HkdfExpander> {
        Box::new(Sha384Expander {
            prk: copy_prk::<48>(okm.as_ref()),
        })
    }

    fn hmac_sign(&self, key: &OkmBlock, message: &[u8]) -> Tag {
        HMAC_SHA384.with_key(key.as_ref()).sign(&[message])
    }

    fn fips(&self) -> bool {
        // Runtime witness — see [`crate::oe::fips_witness_ok`].
        crate::oe::fips_witness_ok()
    }
}

// =========================================================================
// extract / expand machinery
// =========================================================================

fn extract<const N: usize>(
    hmac: &dyn Hmac,
    salt: Option<&[u8]>,
    secret: &[u8],
) -> Box<dyn HkdfExpander>
where
    Sha256Or384Expander<N>: HkdfExpander + 'static,
{
    let salt_bytes;
    let salt_ref: &[u8] = match salt {
        Some(s) => s,
        None => {
            salt_bytes = [0u8; N];
            &salt_bytes
        }
    };
    let tag = hmac.with_key(salt_ref).sign(&[secret]);
    let mut prk = [0u8; N];
    prk.copy_from_slice(tag.as_ref());
    Box::new(Sha256Or384Expander::<N> { prk })
}

fn copy_prk<const N: usize>(bytes: &[u8]) -> [u8; N] {
    assert_eq!(
        bytes.len(),
        N,
        "OkmBlock length must match HKDF hash output length"
    );
    let mut out = [0u8; N];
    out.copy_from_slice(bytes);
    out
}

/// Generic expander parameterised by hash output length.
///
/// We implement the trait twice (via type aliases) for the SHA-256 (N=32)
/// and SHA-384 (N=48) widths, because the `HkdfExpander` impl needs to know
/// which HMAC primitive to call.
struct Sha256Or384Expander<const N: usize> {
    prk: [u8; N],
}

impl<const N: usize> Drop for Sha256Or384Expander<N> {
    fn drop(&mut self) {
        self.prk.zeroize();
    }
}

type Sha256Expander = Sha256Or384Expander<32>;
type Sha384Expander = Sha256Or384Expander<48>;

fn expand<const N: usize>(
    hmac: &dyn Hmac,
    prk: &[u8; N],
    info: &[&[u8]],
    output: &mut [u8],
) -> Result<(), OutputLengthError> {
    // RFC 5869 §2.3: L ≤ 255 · HashLen.
    if output.len() > 255 * N {
        return Err(OutputLengthError);
    }
    let key = hmac.with_key(prk);
    // `prev` carries T(i-1), which is HKDF intermediate output and so
    // sensitive. Wrapped in `Zeroizing` so an unwind from inside the
    // loop (e.g. a panicking custom HMAC impl) still wipes it. Sized to
    // SHA-384 (48 bytes) — the upper bound of our registered hashes.
    let mut prev: Zeroizing<[u8; 48]> = Zeroizing::new([0u8; 48]);
    let mut prev_len = 0usize;
    let mut written = 0usize;
    let mut counter: u8 = 1;

    while written < output.len() {
        // T(i) = HMAC(PRK, T(i-1) || info... || i)
        let counter_byte = [counter];
        // We must build the input slice list dynamically; rustls hmac's
        // `sign(slices)` walks them in order.
        let mut chunks: Vec<&[u8]> = Vec::with_capacity(info.len() + 2);
        if prev_len > 0 {
            chunks.push(&prev[..prev_len]);
        }
        for c in info {
            chunks.push(*c);
        }
        chunks.push(&counter_byte);

        let tag = key.sign(&chunks);
        let block = tag.as_ref();
        debug_assert_eq!(block.len(), N);

        let take = core::cmp::min(N, output.len() - written);
        output[written..written + take].copy_from_slice(&block[..take]);
        written += take;

        // Carry T(i) forward.
        prev[..N].copy_from_slice(block);
        prev_len = N;
        counter = counter.wrapping_add(1);
        if counter == 0 {
            // Should have already returned via length check above.
            return Err(OutputLengthError);
        }
    }

    // `prev`'s Drop wipes the residual T(i) — explicit zeroize would be
    // redundant.
    Ok(())
}

impl HkdfExpander for Sha256Or384Expander<32> {
    fn expand_slice(&self, info: &[&[u8]], output: &mut [u8]) -> Result<(), OutputLengthError> {
        expand::<32>(&HMAC_SHA256, &self.prk, info, output)
    }

    fn expand_block(&self, info: &[&[u8]]) -> OkmBlock {
        let mut buf = [0u8; 32];
        expand::<32>(&HMAC_SHA256, &self.prk, info, &mut buf)
            .expect("expand_block: hash_len fits within RFC 5869 limit");
        let block = OkmBlock::new(&buf);
        buf.zeroize();
        block
    }

    fn hash_len(&self) -> usize {
        32
    }
}

impl HkdfExpander for Sha256Or384Expander<48> {
    fn expand_slice(&self, info: &[&[u8]], output: &mut [u8]) -> Result<(), OutputLengthError> {
        expand::<48>(&HMAC_SHA384, &self.prk, info, output)
    }

    fn expand_block(&self, info: &[&[u8]]) -> OkmBlock {
        let mut buf = [0u8; 48];
        expand::<48>(&HMAC_SHA384, &self.prk, info, &mut buf)
            .expect("expand_block: hash_len fits within RFC 5869 limit");
        let block = OkmBlock::new(&buf);
        buf.zeroize();
        block
    }

    fn hash_len(&self) -> usize {
        48
    }
}

#[cfg(test)]
#[path = "hkdf_tests.rs"]
mod tests;
