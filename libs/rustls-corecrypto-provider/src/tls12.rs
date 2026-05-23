//! TLS 1.2 cipher suite registrations + PRF.
//!
//! Four FIPS-approved GCM cipher suites:
//! - `ECDHE_ECDSA_WITH_AES_128_GCM_SHA256`
//! - `ECDHE_ECDSA_WITH_AES_256_GCM_SHA384`
//! - `ECDHE_RSA_WITH_AES_128_GCM_SHA256`
//! - `ECDHE_RSA_WITH_AES_256_GCM_SHA384`
//!
//! ## Wire format (RFC 5288)
//!
//! Each record body is `explicit_nonce(8) || ciphertext || tag(16)`. The
//! full AEAD nonce is `implicit_iv(4) || explicit_nonce(8)`, where the
//! implicit IV comes from the TLS 1.2 key_block (`fixed_iv_len = 4`) and the
//! explicit nonce is the (per-record) 8-byte counter sent in the clear.
//!
//! AAD = `seq_num(8) || ContentType(1) || ProtocolVersion(2) || Length(2)`
//! (constructed via [`make_tls12_aad`]). Length is plaintext length, NOT
//! including the explicit nonce or tag.

use rustls::ConnectionTrafficSecrets;
use rustls::crypto::ActiveKeyExchange;
use rustls::crypto::cipher::{
    AeadKey, InboundOpaqueMessage, InboundPlainMessage, KeyBlockShape, MessageDecrypter,
    MessageEncrypter, OutboundOpaqueMessage, OutboundPlainMessage, PrefixedPayload,
    Tls12AeadAlgorithm, UnsupportedOperationError, make_tls12_aad,
};
use rustls::crypto::tls12::{Prf, PrfUsingHmac};
use zeroize::Zeroizing;

use crate::aead;
use crate::hmac::{HMAC_SHA256, HMAC_SHA384};

const EXPLICIT_NONCE_LEN: usize = 8;
const IMPLICIT_IV_LEN: usize = 4;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = aead::TAG_LEN;

// =========================================================================
// AEAD algorithm wrappers
// =========================================================================

#[derive(Debug)]
pub struct Aes128Gcm;
#[derive(Debug)]
pub struct Aes256Gcm;

impl Tls12AeadAlgorithm for Aes128Gcm {
    fn encrypter(&self, key: AeadKey, iv: &[u8], extra: &[u8]) -> Box<dyn MessageEncrypter> {
        // RFC 5288 §3 / RFC 5246 §6.2.3.3: explicit_nonce must be unique
        // per (key, fixed_iv) pair. Two RFC-compliant constructions are
        // common in the ecosystem:
        //
        // 1. `extra ^ seq.to_be_bytes()` — XOR the rustls-provided
        //    per-connection 8-byte random `extra` with the big-endian
        //    sequence number. This is what rustls's own aws-lc-rs and
        //    ring providers do; the per-connection random adds defense
        //    against cross-connection nonce-reuse if a key-extraction
        //    state-corruption attack ever weakened the seq counter.
        // 2. `seq.to_be_bytes()` alone — also unique-per-record, but
        //    predictable and slightly weaker against the same attack.
        //
        // We adopt (1) to match upstream rustls behaviour and audit
        // posture. The decrypter side does not care which construction
        // the peer used — it reads explicit_nonce verbatim from the
        // wire and trusts GCM authentication to catch any disagreement.
        make_encrypter(key, iv, extra)
    }
    fn decrypter(&self, key: AeadKey, iv: &[u8]) -> Box<dyn MessageDecrypter> {
        make_decrypter(key, iv)
    }
    fn key_block_shape(&self) -> KeyBlockShape {
        KeyBlockShape {
            enc_key_len: aead::AES128_KEY_LEN,
            fixed_iv_len: IMPLICIT_IV_LEN,
            explicit_nonce_len: EXPLICIT_NONCE_LEN,
        }
    }
    fn extract_keys(
        &self,
        key: AeadKey,
        iv: &[u8],
        explicit: &[u8],
    ) -> Result<ConnectionTrafficSecrets, UnsupportedOperationError> {
        Ok(ConnectionTrafficSecrets::Aes128Gcm {
            key,
            iv: build_iv(iv, explicit),
        })
    }
    fn fips(&self) -> bool {
        // Runtime witness — see [`crate::oe::fips_witness_ok`].
        crate::oe::fips_witness_ok()
    }
}

impl Tls12AeadAlgorithm for Aes256Gcm {
    fn encrypter(&self, key: AeadKey, iv: &[u8], extra: &[u8]) -> Box<dyn MessageEncrypter> {
        // See `Aes128Gcm::encrypter` for the explicit-nonce policy.
        make_encrypter(key, iv, extra)
    }
    fn decrypter(&self, key: AeadKey, iv: &[u8]) -> Box<dyn MessageDecrypter> {
        make_decrypter(key, iv)
    }
    fn key_block_shape(&self) -> KeyBlockShape {
        KeyBlockShape {
            enc_key_len: aead::AES256_KEY_LEN,
            fixed_iv_len: IMPLICIT_IV_LEN,
            explicit_nonce_len: EXPLICIT_NONCE_LEN,
        }
    }
    fn extract_keys(
        &self,
        key: AeadKey,
        iv: &[u8],
        explicit: &[u8],
    ) -> Result<ConnectionTrafficSecrets, UnsupportedOperationError> {
        Ok(ConnectionTrafficSecrets::Aes256Gcm {
            key,
            iv: build_iv(iv, explicit),
        })
    }
    fn fips(&self) -> bool {
        // Runtime witness — see [`crate::oe::fips_witness_ok`].
        crate::oe::fips_witness_ok()
    }
}

fn build_iv(iv: &[u8], explicit: &[u8]) -> rustls::crypto::cipher::Iv {
    // Fail-loud in release: a wrong-length IV from rustls would mean a
    // serious upstream contract break, and the silent panic via
    // `copy_from_slice` later is harder to diagnose than an explicit one.
    assert_eq!(
        iv.len(),
        IMPLICIT_IV_LEN,
        "TLS 1.2 implicit IV must be {IMPLICIT_IV_LEN} bytes"
    );
    assert_eq!(
        explicit.len(),
        EXPLICIT_NONCE_LEN,
        "TLS 1.2 explicit nonce must be {EXPLICIT_NONCE_LEN} bytes"
    );
    let mut full = [0u8; NONCE_LEN];
    full[..IMPLICIT_IV_LEN].copy_from_slice(iv);
    full[IMPLICIT_IV_LEN..].copy_from_slice(explicit);
    rustls::crypto::cipher::Iv::copy(&full)
}

// rustls-API note (A-1, comment-only per review): the `extra` parameter
// here is the 8-byte per-connection random rustls passes through from
// the TLS 1.2 key_block (RFC 5246 §6.3) for use in deriving the
// explicit_nonce. We follow the aws-lc-rs / ring providers' construction
// (`extra XOR seq.to_be_bytes()`) verbatim; this is *not* a rustls trait
// contract — rustls treats `extra` as opaque per-connection material.
// If a future rustls release changes the semantics of `extra` (e.g. to
// "use as full nonce" rather than "salt to XOR with seq") this function
// will silently produce nonces that don't match peers, surfacing as
// `DecryptError` in handshake_smoke. The regression test
// `tls12_explicit_nonce_is_extra_xor_seq` pins the current wire shape.
fn make_encrypter(key: AeadKey, iv: &[u8], extra: &[u8]) -> Box<dyn MessageEncrypter> {
    assert_eq!(
        iv.len(),
        IMPLICIT_IV_LEN,
        "TLS 1.2 implicit IV must be {IMPLICIT_IV_LEN} bytes"
    );
    assert_eq!(
        extra.len(),
        EXPLICIT_NONCE_LEN,
        "TLS 1.2 explicit nonce material must be {EXPLICIT_NONCE_LEN} bytes"
    );
    // Pre-assemble the 12-byte full nonce template: implicit_iv(4) || extra(8).
    // On each encrypt we XOR `seq.to_be_bytes()` into the trailing 8 bytes,
    // matching rustls's aws-lc-rs / ring providers.
    let mut full_iv = [0u8; NONCE_LEN];
    full_iv[..IMPLICIT_IV_LEN].copy_from_slice(iv);
    full_iv[IMPLICIT_IV_LEN..].copy_from_slice(extra);
    Box::new(Tls12Encrypter {
        key: Zeroizing::new(key.as_ref().to_vec()),
        full_iv,
    })
}

fn make_decrypter(key: AeadKey, iv: &[u8]) -> Box<dyn MessageDecrypter> {
    assert_eq!(
        iv.len(),
        IMPLICIT_IV_LEN,
        "TLS 1.2 implicit IV must be {IMPLICIT_IV_LEN} bytes"
    );
    let mut implicit = [0u8; IMPLICIT_IV_LEN];
    implicit.copy_from_slice(iv);
    Box::new(Tls12Decrypter {
        key: Zeroizing::new(key.as_ref().to_vec()),
        implicit_iv: implicit,
    })
}

// =========================================================================
// Encrypter / Decrypter
// =========================================================================

struct Tls12Encrypter {
    /// AEAD session key. `Zeroizing` wipes the bytes on drop.
    key: Zeroizing<Vec<u8>>,
    /// Pre-assembled 12-byte nonce template: `implicit_iv(4) || extra(8)`.
    /// Each `encrypt` call XORs `seq.to_be_bytes()` into the trailing
    /// 8 bytes to derive the per-record nonce (see `Tls12AeadAlgorithm::encrypter`
    /// comment for rationale).
    full_iv: [u8; NONCE_LEN],
}

impl MessageEncrypter for Tls12Encrypter {
    fn encrypt(
        &mut self,
        msg: OutboundPlainMessage<'_>,
        seq: u64,
    ) -> Result<OutboundOpaqueMessage, rustls::Error> {
        let pt_len = msg.payload.len();
        // Derive the per-record nonce: implicit_iv(4) is left untouched;
        // the trailing 8 bytes are `extra XOR seq.to_be_bytes()`. Matches
        // rustls's aws-lc-rs / ring TLS 1.2 nonce construction.
        let mut nonce = self.full_iv;
        let seq_be = seq.to_be_bytes();
        for i in 0..EXPLICIT_NONCE_LEN {
            nonce[IMPLICIT_IV_LEN + i] ^= seq_be[i];
        }

        let aad = make_tls12_aad(seq, msg.typ, msg.version, pt_len);

        // `Zeroizing` wipes the plaintext on drop after the encrypt call.
        let mut pt: Zeroizing<Vec<u8>> = Zeroizing::new(Vec::with_capacity(pt_len));
        msg.payload.copy_to_vec(&mut pt);

        let mut ct = vec![0u8; pt_len];
        let tag = aead::encrypt(&self.key, &nonce, aad.as_ref(), &pt, &mut ct)
            .map_err(|e| rustls::Error::General(format!("AES-GCM encrypt: {e}")))?;

        // Wire: explicit_nonce(8) || ciphertext || tag(16).
        let mut payload = PrefixedPayload::with_capacity(EXPLICIT_NONCE_LEN + ct.len() + TAG_LEN);
        payload.extend_from_slice(&nonce[IMPLICIT_IV_LEN..]); // explicit
        payload.extend_from_slice(&ct);
        payload.extend_from_slice(&tag);

        Ok(OutboundOpaqueMessage::new(msg.typ, msg.version, payload))
    }

    fn encrypted_payload_len(&self, payload_len: usize) -> usize {
        EXPLICIT_NONCE_LEN + payload_len + TAG_LEN
    }
}

struct Tls12Decrypter {
    /// AEAD session key. `Zeroizing` wipes the bytes on drop.
    key: Zeroizing<Vec<u8>>,
    implicit_iv: [u8; IMPLICIT_IV_LEN],
}

impl MessageDecrypter for Tls12Decrypter {
    fn decrypt<'a>(
        &mut self,
        mut msg: InboundOpaqueMessage<'a>,
        seq: u64,
    ) -> Result<InboundPlainMessage<'a>, rustls::Error> {
        let payload_len = msg.payload.len();
        if payload_len < EXPLICIT_NONCE_LEN + TAG_LEN {
            return Err(rustls::Error::DecryptError);
        }
        let ct_len = payload_len - EXPLICIT_NONCE_LEN - TAG_LEN;

        // Split: explicit_nonce(8) | ciphertext(ct_len) | tag(16).
        let payload = &mut msg.payload[..];
        let mut nonce = [0u8; NONCE_LEN];
        nonce[..IMPLICIT_IV_LEN].copy_from_slice(&self.implicit_iv);
        nonce[IMPLICIT_IV_LEN..].copy_from_slice(&payload[..EXPLICIT_NONCE_LEN]);

        let aad = make_tls12_aad(seq, msg.typ, msg.version, ct_len);

        let ct_start = EXPLICIT_NONCE_LEN;
        let tag_start = payload_len - TAG_LEN;
        let mut tag = [0u8; TAG_LEN];
        tag.copy_from_slice(&payload[tag_start..]);

        // `Zeroizing` wipes the decrypted plaintext from this temp on drop.
        let mut pt: Zeroizing<Vec<u8>> = Zeroizing::new(vec![0u8; ct_len]);
        aead::decrypt(
            &self.key,
            &nonce,
            aad.as_ref(),
            &payload[ct_start..tag_start],
            &mut pt,
            &tag,
        )
        .map_err(|_| rustls::Error::DecryptError)?;

        // Shift the plaintext to the front of the payload, truncate.
        payload[..ct_len].copy_from_slice(&pt);
        msg.payload.truncate(ct_len);
        Ok(msg.into_plain_message())
    }
}

// =========================================================================
// Public statics
// =========================================================================

pub static AES_128_GCM: Aes128Gcm = Aes128Gcm;
pub static AES_256_GCM: Aes256Gcm = Aes256Gcm;

// TLS 1.2 PRF (RFC 5246 §5) is a P_hash construction over HMAC. rustls
// provides `PrfUsingHmac` as a generic wrapper, but its `fips()` returns
// `false` because rustls intentionally does not treat HMAC-composed PRF
// as FIPS-validated.
//
// Compare with the rustls aws_lc_rs provider: when compiled with
// `--features fips` it bypasses `PrfUsingHmac` entirely and uses a
// dedicated `Tls12Prf` backed by aws-lc-fips's separately CAVS-validated
// `tls_prf::Algorithm` primitive (NIST SP 800-135 §4.2.2 Component
// Validation List "TlsKdfPrf"). That is what makes their PRF FIPS-claim
// auditable.
//
// Apple corecrypto **does not** expose a CAVS-listed dedicated TLS PRF
// primitive — Security.framework / CommonCrypto only ship the HMAC and
// hash primitives. We therefore use `PrfUsingHmac` honestly: it is
// constructed from FIPS-validated HMAC primitives, but the PRF as a whole
// is NOT itself CAVS-validated. We do NOT override `fips()` — it returns
// the default `false`.
//
// Practical consequence: `Tls12CipherSuite::fips()` is `false` for every
// TLS 1.2 cipher suite this provider exposes, so `ServerConfig::fips()` /
// `ClientConfig::fips()` is `true` ONLY when the negotiated protocol is
// TLS 1.3 (where HKDF is the Approved KDF per SP 800-56C and the chain
// is HMAC-anchored without a separate PRF step). FIPS-conscious callers
// must restrict their config to TLS 1.3 — see ADR 0004 "FIPS posture".

/// TLS 1.2 PRF using HMAC-SHA-256.
///
/// Not FIPS-validated as a composite primitive (see module comment).
#[derive(Debug)]
pub struct PrfSha256;
/// TLS 1.2 PRF using HMAC-SHA-384.
///
/// Not FIPS-validated as a composite primitive (see module comment).
#[derive(Debug)]
pub struct PrfSha384;

pub static PRF_SHA256: PrfSha256 = PrfSha256;
pub static PRF_SHA384: PrfSha384 = PrfSha384;

impl Prf for PrfSha256 {
    fn for_key_exchange(
        &self,
        output: &mut [u8; 48],
        kx: Box<dyn ActiveKeyExchange>,
        peer_pub_key: &[u8],
        label: &[u8],
        seed: &[u8],
    ) -> Result<(), rustls::Error> {
        PrfUsingHmac(&HMAC_SHA256).for_key_exchange(output, kx, peer_pub_key, label, seed)
    }
    fn for_secret(&self, output: &mut [u8], secret: &[u8], label: &[u8], seed: &[u8]) {
        PrfUsingHmac(&HMAC_SHA256).for_secret(output, secret, label, seed)
    }
    // Intentionally NOT overridden — default `false`. See module comment.
}

impl Prf for PrfSha384 {
    fn for_key_exchange(
        &self,
        output: &mut [u8; 48],
        kx: Box<dyn ActiveKeyExchange>,
        peer_pub_key: &[u8],
        label: &[u8],
        seed: &[u8],
    ) -> Result<(), rustls::Error> {
        PrfUsingHmac(&HMAC_SHA384).for_key_exchange(output, kx, peer_pub_key, label, seed)
    }
    fn for_secret(&self, output: &mut [u8], secret: &[u8], label: &[u8], seed: &[u8]) {
        PrfUsingHmac(&HMAC_SHA384).for_secret(output, secret, label, seed)
    }
    // Intentionally NOT overridden — default `false`. See module comment.
}

#[cfg(test)]
#[path = "tls12_tests.rs"]
mod tests;
