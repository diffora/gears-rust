//! AES-128-GCM and AES-256-GCM via CommonCrypto's `CCCryptor*` family in
//! `kCCModeGCM`.
//!
//! For TLS we need a 16-byte authentication tag and a 12-byte nonce, which
//! are the AEAD-AES-GCM constants in TLS 1.2 and TLS 1.3. Phase 2 exposes
//! standalone `encrypt` / `decrypt` functions; Phase 4 wraps them in the
//! `Tls13AeadAlgorithm` / `Tls12AeadAlgorithm` traits.

use core::ffi::c_void;
use core::ptr;

use subtle::ConstantTimeEq;
use zeroize::Zeroize;

use crate::ffi::commoncrypto as cc;

pub const NONCE_LEN: usize = 12;
pub const TAG_LEN: usize = 16;
pub const AES128_KEY_LEN: usize = 16;
pub const AES256_KEY_LEN: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum AeadError {
    #[error("CommonCrypto returned status {0}")]
    CommonCrypto(cc::CCCryptorStatus),
    #[error("output buffer too small (need {needed}, have {have})")]
    OutputTooSmall { needed: usize, have: usize },
    #[error("invalid key length {0} (must be 16 or 32)")]
    InvalidKeyLen(usize),
    #[error("invalid nonce length {0} (must be 12)")]
    InvalidNonceLen(usize),
    #[error("authentication tag mismatch (ciphertext tampered or wrong key)")]
    TagMismatch,
}

/// RAII wrapper for `CCCryptorRef` that releases on drop.
struct Cryptor(cc::CCCryptorRef);

impl Drop for Cryptor {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: `self.0` was produced by `CCCryptorCreateWithMode`; releasing
            // a valid cryptor is always safe and idempotent w.r.t. our state.
            unsafe {
                cc::CCCryptorRelease(self.0);
            }
        }
    }
}

fn create_cryptor(op: cc::CCOperation, key: &[u8], iv: &[u8]) -> Result<Cryptor, AeadError> {
    if key.len() != AES128_KEY_LEN && key.len() != AES256_KEY_LEN {
        return Err(AeadError::InvalidKeyLen(key.len()));
    }
    if iv.len() != NONCE_LEN {
        return Err(AeadError::InvalidNonceLen(iv.len()));
    }
    let mut cryptor: cc::CCCryptorRef = ptr::null_mut();
    // SAFETY: All pointers are correctly typed and lifetimes outlast the call.
    // The IV is set via CCCryptorGCMAddIV after Create — passing it inside
    // Create produced inconsistent results across CommonCrypto revisions.
    let status = unsafe {
        cc::CCCryptorCreateWithMode(
            op,
            cc::kCCModeGCM,
            cc::kCCAlgorithmAES,
            cc::ccNoPadding,
            ptr::null(), // IV set via CCCryptorGCMAddIV below
            key.as_ptr() as *const c_void,
            key.len(),
            ptr::null(), // no tweak (XTS only)
            0,
            0, // num_rounds: 0 = default
            0, // options
            &mut cryptor,
        )
    };
    if status != cc::kCCSuccess {
        return Err(AeadError::CommonCrypto(status));
    }
    // SAFETY: cryptor is valid and freshly created; iv length is checked above.
    let s = unsafe { cc::CCCryptorGCMAddIV(cryptor, iv.as_ptr() as *const c_void, iv.len()) };
    if s != cc::kCCSuccess {
        // SAFETY: cryptor was allocated; release it before returning the error.
        unsafe {
            cc::CCCryptorRelease(cryptor);
        }
        return Err(AeadError::CommonCrypto(s));
    }
    Ok(Cryptor(cryptor))
}

fn add_aad(c: &Cryptor, aad: &[u8]) -> Result<(), AeadError> {
    if aad.is_empty() {
        return Ok(());
    }
    // SAFETY: cryptor is valid; aad is a valid slice.
    let s = unsafe { cc::CCCryptorGCMaddAAD(c.0, aad.as_ptr() as *const c_void, aad.len()) };
    if s != cc::kCCSuccess {
        return Err(AeadError::CommonCrypto(s));
    }
    Ok(())
}

fn run_update(c: &Cryptor, input: &[u8], output: &mut [u8]) -> Result<(), AeadError> {
    if output.len() < input.len() {
        return Err(AeadError::OutputTooSmall {
            needed: input.len(),
            have: output.len(),
        });
    }
    if input.is_empty() {
        return Ok(());
    }
    let mut moved: usize = 0;
    // SAFETY: pointers and lengths are correct.
    let s = unsafe {
        cc::CCCryptorUpdate(
            c.0,
            input.as_ptr() as *const c_void,
            input.len(),
            output.as_mut_ptr() as *mut c_void,
            output.len(),
            &mut moved,
        )
    };
    if s != cc::kCCSuccess {
        return Err(AeadError::CommonCrypto(s));
    }
    // GCM is a stream cipher: `moved` must equal input.len() byte-for-byte.
    // If CommonCrypto ever returns a short write the trailing bytes of
    // `output` are uninitialised; we must NOT return Ok in that case.
    if moved != input.len() {
        return Err(AeadError::CommonCrypto(cc::kCCUnspecifiedError));
    }
    Ok(())
}

fn finalize_tag(c: &Cryptor) -> Result<[u8; TAG_LEN], AeadError> {
    let mut tag = [0u8; TAG_LEN];
    let mut tag_len = TAG_LEN;
    // SAFETY: tag and tag_len are valid pointers to writable memory.
    // CCCryptorGCMFinal finalizes the GCM state and writes the (computed) tag.
    let s = unsafe { cc::CCCryptorGCMFinal(c.0, tag.as_mut_ptr() as *mut c_void, &mut tag_len) };
    if s != cc::kCCSuccess {
        return Err(AeadError::CommonCrypto(s));
    }
    if tag_len != TAG_LEN {
        return Err(AeadError::CommonCrypto(cc::kCCParamError));
    }
    Ok(tag)
}

/// Encrypt `plaintext` into `ciphertext_out`, returning the 16-byte tag.
///
/// `key` must be 16 or 32 bytes; `iv` exactly 12 bytes;
/// `ciphertext_out.len() >= plaintext.len()` (extra space is untouched).
pub fn encrypt(
    key: &[u8],
    iv: &[u8],
    aad: &[u8],
    plaintext: &[u8],
    ciphertext_out: &mut [u8],
) -> Result<[u8; TAG_LEN], AeadError> {
    let c = create_cryptor(cc::kCCEncrypt, key, iv)?;
    add_aad(&c, aad)?;
    run_update(&c, plaintext, ciphertext_out)?;
    finalize_tag(&c)
}

/// Decrypt `ciphertext` into `plaintext_out`, verifying `expected_tag`.
///
/// On tag mismatch the output buffer's contents are *unspecified* (typically
/// the speculatively-decrypted plaintext); callers must treat the entire
/// output as compromised and not use it. We zeroize the output on mismatch
/// as a defense-in-depth measure.
///
/// Callers that hold the resulting plaintext in their own buffer should wrap
/// it in `zeroize::Zeroizing` so the cleartext is wiped on drop — both the
/// TLS 1.3 and TLS 1.2 wrappers in this crate already do that.
pub fn decrypt(
    key: &[u8],
    iv: &[u8],
    aad: &[u8],
    ciphertext: &[u8],
    plaintext_out: &mut [u8],
    expected_tag: &[u8; TAG_LEN],
) -> Result<(), AeadError> {
    let c = create_cryptor(cc::kCCDecrypt, key, iv)?;
    add_aad(&c, aad)?;
    run_update(&c, ciphertext, plaintext_out)?;
    let computed = finalize_tag(&c)?;
    if computed.ct_eq(expected_tag).into() {
        Ok(())
    } else {
        // Wipe the speculatively-decrypted plaintext.
        plaintext_out[..ciphertext.len()].zeroize();
        Err(AeadError::TagMismatch)
    }
}

#[cfg(test)]
#[path = "aead_tests.rs"]
mod tests;
