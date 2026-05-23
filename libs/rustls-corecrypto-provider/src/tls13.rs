//! TLS 1.3 cipher suite registrations.
//!
//! Two suites, both FIPS-approved:
//! - `TLS13_AES_128_GCM_SHA256`
//! - `TLS13_AES_256_GCM_SHA384`
//!
//! ChaCha20-Poly1305 is intentionally excluded — it is not FIPS-approved.
//!
//! ## Wire format (RFC 8446 §5.2)
//!
//! - Nonce: 12-byte static IV XORed with big-endian 64-bit sequence number
//!   (zero-padded to 12 bytes), constructed via [`Nonce::new`].
//! - AAD: 5-byte record header `ContentType(0x17) || LegacyVersion(0x0303)
//!   || Length(BE u16)` where length is plaintext + 1 (ContentType) + 16
//!   (tag), constructed via [`make_tls13_aad`].
//! - Inner plaintext: payload + 1 byte ContentType (the actual record type,
//!   not 0x17). The trailing byte distinguishes Handshake/Alert/etc. records.

use rustls::crypto::cipher::{
    AeadKey, InboundOpaqueMessage, InboundPlainMessage, Iv, MessageDecrypter, MessageEncrypter,
    Nonce, OutboundOpaqueMessage, OutboundPlainMessage, PrefixedPayload, Tls13AeadAlgorithm,
    UnsupportedOperationError, make_tls13_aad,
};
use rustls::{ConnectionTrafficSecrets, ContentType, ProtocolVersion};
use zeroize::Zeroizing;

use crate::aead;

// =========================================================================
// AEAD algorithm wrappers
// =========================================================================

#[derive(Debug)]
pub struct Aes128Gcm;
#[derive(Debug)]
pub struct Aes256Gcm;

const TAG_LEN: usize = aead::TAG_LEN;

impl Tls13AeadAlgorithm for Aes128Gcm {
    fn encrypter(&self, key: AeadKey, iv: Iv) -> Box<dyn MessageEncrypter> {
        Box::new(Tls13Encrypter {
            key: Zeroizing::new(key.as_ref().to_vec()),
            iv,
        })
    }
    fn decrypter(&self, key: AeadKey, iv: Iv) -> Box<dyn MessageDecrypter> {
        Box::new(Tls13Decrypter {
            key: Zeroizing::new(key.as_ref().to_vec()),
            iv,
        })
    }
    fn key_len(&self) -> usize {
        aead::AES128_KEY_LEN
    }
    fn extract_keys(
        &self,
        key: AeadKey,
        iv: Iv,
    ) -> Result<ConnectionTrafficSecrets, UnsupportedOperationError> {
        Ok(ConnectionTrafficSecrets::Aes128Gcm { key, iv })
    }
    fn fips(&self) -> bool {
        // Runtime witness — see [`crate::oe::fips_witness_ok`].
        crate::oe::fips_witness_ok()
    }
}

impl Tls13AeadAlgorithm for Aes256Gcm {
    fn encrypter(&self, key: AeadKey, iv: Iv) -> Box<dyn MessageEncrypter> {
        Box::new(Tls13Encrypter {
            key: Zeroizing::new(key.as_ref().to_vec()),
            iv,
        })
    }
    fn decrypter(&self, key: AeadKey, iv: Iv) -> Box<dyn MessageDecrypter> {
        Box::new(Tls13Decrypter {
            key: Zeroizing::new(key.as_ref().to_vec()),
            iv,
        })
    }
    fn key_len(&self) -> usize {
        aead::AES256_KEY_LEN
    }
    fn extract_keys(
        &self,
        key: AeadKey,
        iv: Iv,
    ) -> Result<ConnectionTrafficSecrets, UnsupportedOperationError> {
        Ok(ConnectionTrafficSecrets::Aes256Gcm { key, iv })
    }
    fn fips(&self) -> bool {
        // Runtime witness — see [`crate::oe::fips_witness_ok`].
        crate::oe::fips_witness_ok()
    }
}

// =========================================================================
// Encrypter / Decrypter
// =========================================================================

struct Tls13Encrypter {
    /// AEAD session key. `Zeroizing` wipes the bytes on drop so the key
    /// does not linger in the heap after the connection closes.
    key: Zeroizing<Vec<u8>>,
    iv: Iv,
}

impl MessageEncrypter for Tls13Encrypter {
    fn encrypt(
        &mut self,
        msg: OutboundPlainMessage<'_>,
        seq: u64,
    ) -> Result<OutboundOpaqueMessage, rustls::Error> {
        // Inner plaintext = payload || ContentType (1 byte). RFC 8446 §5.2.
        // `Zeroizing` ensures the cleartext is wiped from the heap after
        // the encrypt call returns.
        let mut pt: Zeroizing<Vec<u8>> = Zeroizing::new(Vec::with_capacity(msg.payload.len() + 1));
        msg.payload.copy_to_vec(&mut pt);
        pt.push(msg.typ.into());

        let nonce = Nonce::new(&self.iv, seq);
        let aad = make_tls13_aad(pt.len() + TAG_LEN);

        let mut ct = vec![0u8; pt.len()];
        let tag = aead::encrypt(&self.key, &nonce.0, aad.as_ref(), &pt, &mut ct)
            .map_err(|e| rustls::Error::General(format!("AES-GCM encrypt: {e}")))?;

        let mut payload = PrefixedPayload::with_capacity(ct.len() + TAG_LEN);
        payload.extend_from_slice(&ct);
        payload.extend_from_slice(&tag);

        Ok(OutboundOpaqueMessage::new(
            ContentType::ApplicationData,
            ProtocolVersion::TLSv1_2,
            payload,
        ))
    }

    fn encrypted_payload_len(&self, payload_len: usize) -> usize {
        payload_len + 1 + TAG_LEN
    }
}

struct Tls13Decrypter {
    /// AEAD session key. `Zeroizing` wipes the bytes on drop so the key
    /// does not linger in the heap after the connection closes.
    key: Zeroizing<Vec<u8>>,
    iv: Iv,
}

impl MessageDecrypter for Tls13Decrypter {
    fn decrypt<'a>(
        &mut self,
        mut msg: InboundOpaqueMessage<'a>,
        seq: u64,
    ) -> Result<InboundPlainMessage<'a>, rustls::Error> {
        let payload_len = msg.payload.len();
        if payload_len < TAG_LEN {
            return Err(rustls::Error::DecryptError);
        }
        let nonce = Nonce::new(&self.iv, seq);
        let aad = make_tls13_aad(payload_len);

        let ct_len = payload_len - TAG_LEN;
        // Split: ciphertext | tag (last 16 bytes).
        let payload = &mut msg.payload[..];
        let (ct_buf, tag_buf) = payload.split_at_mut(ct_len);

        let mut tag = [0u8; TAG_LEN];
        tag.copy_from_slice(tag_buf);

        // Decrypt into a separate buffer, then copy back. (Apple's GCM may
        // refuse fully aliased in/out; safer to keep them disjoint.)
        // `Zeroizing` wipes the cleartext from this temp on drop.
        let mut pt: Zeroizing<Vec<u8>> = Zeroizing::new(vec![0u8; ct_len]);
        aead::decrypt(&self.key, &nonce.0, aad.as_ref(), ct_buf, &mut pt, &tag)
            .map_err(|_| rustls::Error::DecryptError)?;
        ct_buf.copy_from_slice(&pt);

        // Strip tag from the payload then let rustls strip the TLS 1.3
        // padding + inner ContentType byte.
        msg.payload.truncate(ct_len);
        msg.into_tls13_unpadded_message()
    }
}

// =========================================================================
// Public statics — the AEAD wrappers consumed by `provider::default_provider`.
// =========================================================================

pub static AES_128_GCM: Aes128Gcm = Aes128Gcm;
pub static AES_256_GCM: Aes256Gcm = Aes256Gcm;

#[cfg(test)]
#[path = "tls13_tests.rs"]
mod tests;
