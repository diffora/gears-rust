//! RSA signing and verification through the same provider as TLS. PKCS#1 and
//! PKCS#8 are used only to parse and serialize key structure; they perform no
//! cryptographic operations.

use std::sync::Arc;

// pkcs1 0.7 and pkcs8 0.11 sit on different `der` majors; each needs its own.
use pkcs1::der::{Decode as _, Encode};
use pkcs8::der::Decode as _;
use pkcs8::{ObjectIdentifier, PrivateKeyInfoRef};
use rustls::SignatureScheme;
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer, alg_id};
use rustls::sign::SigningKey;

use super::{CryptoError, backend};

const RSA_ENCRYPTION: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
const PAIRWISE_MESSAGE: &[u8] = b"toolkit-security RSA key consistency";

/// RSA signature algorithms supported by the platform's RSA JWKS provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RsaAlgorithm {
    /// RSASSA-PKCS1-v1_5 with SHA-256.
    Pkcs1Sha256,
    /// RSASSA-PKCS1-v1_5 with SHA-384.
    Pkcs1Sha384,
    /// RSASSA-PKCS1-v1_5 with SHA-512.
    Pkcs1Sha512,
    /// RSASSA-PSS with SHA-256 and a digest-length salt.
    PssSha256,
    /// RSASSA-PSS with SHA-384 and a digest-length salt.
    PssSha384,
    /// RSASSA-PSS with SHA-512 and a digest-length salt.
    PssSha512,
}

impl RsaAlgorithm {
    fn signature_id(self) -> alg_id::AlgorithmIdentifier {
        match self {
            Self::Pkcs1Sha256 => alg_id::RSA_PKCS1_SHA256,
            Self::Pkcs1Sha384 => alg_id::RSA_PKCS1_SHA384,
            Self::Pkcs1Sha512 => alg_id::RSA_PKCS1_SHA512,
            Self::PssSha256 => alg_id::RSA_PSS_SHA256,
            Self::PssSha384 => alg_id::RSA_PSS_SHA384,
            Self::PssSha512 => alg_id::RSA_PSS_SHA512,
        }
    }

    fn scheme(self) -> SignatureScheme {
        match self {
            Self::Pkcs1Sha256 => SignatureScheme::RSA_PKCS1_SHA256,
            Self::Pkcs1Sha384 => SignatureScheme::RSA_PKCS1_SHA384,
            Self::Pkcs1Sha512 => SignatureScheme::RSA_PKCS1_SHA512,
            Self::PssSha256 => SignatureScheme::RSA_PSS_SHA256,
            Self::PssSha384 => SignatureScheme::RSA_PSS_SHA384,
            Self::PssSha512 => SignatureScheme::RSA_PSS_SHA512,
        }
    }
}

/// An RSA signing key owned by the configured OS backend.
///
/// Imported from unencrypted PKCS#8 (`rsaEncryption`, two primes) with the
/// same modulus and exponent limits as [`verify_rsa_components`]. The backend
/// key is checked by a sign/verify round trip on import. No key generation:
/// keys are provisioned externally.
#[derive(Clone)]
pub struct RsaKeyPair {
    inner: Arc<dyn SigningKey>,
    modulus: Vec<u8>,
    public_exponent: Vec<u8>,
}

impl std::fmt::Debug for RsaKeyPair {
    /// Shows the modulus size only; the private half never leaves the backend.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RsaKeyPair")
            .field("modulus_bits", &modulus_bits(&self.modulus))
            .finish_non_exhaustive()
    }
}

impl RsaKeyPair {
    /// Import PKCS#8 DER with algorithm `rsaEncryption` (absent or NULL
    /// parameters). RSASSA-PSS-restricted and multi-prime keys are rejected.
    ///
    /// # Errors
    /// Returns an error for unsupported encodings, a modulus outside
    /// 2048–8192 bits, a key the backend rejects or that fails the pairwise
    /// check, and an unavailable backend.
    pub fn from_pkcs8(bytes: &[u8]) -> Result<Self, CryptoError> {
        Self::from_pkcs8_with_provider(bytes, backend::provider()?)
    }

    /// Import an unencrypted PKCS#8 PEM document (`-----BEGIN PRIVATE KEY-----`)
    /// with the same checks as [`Self::from_pkcs8`] and the same strict
    /// RFC 7468 decoding as [`EcdsaP256KeyPair::from_pkcs8_pem`](super::signature::EcdsaP256KeyPair::from_pkcs8_pem).
    ///
    /// # Errors
    /// Returns an error for a different PEM label, including PKCS#1
    /// `RSA PRIVATE KEY` and `ENCRYPTED PRIVATE KEY`, for a non-conforming
    /// encapsulation, and for every [`Self::from_pkcs8`] failure.
    pub fn from_pkcs8_pem(pem: &str) -> Result<Self, CryptoError> {
        super::with_pkcs8_pem(pem, Self::from_pkcs8)
    }

    pub(super) fn from_pkcs8_with_provider(
        bytes: &[u8],
        provider: &rustls::crypto::CryptoProvider,
    ) -> Result<Self, CryptoError> {
        let pki = PrivateKeyInfoRef::from_der(bytes).map_err(|_| CryptoError::InvalidKey)?;
        // Only rsaEncryption: never drop RSASSA-PSS key constraints.
        if pki.algorithm.oid != RSA_ENCRYPTION
            || pki
                .algorithm
                .parameters
                .is_some_and(|params| params != pkcs8::der::asn1::AnyRef::NULL)
        {
            return Err(CryptoError::InvalidKey);
        }
        let rsa = pkcs1::RsaPrivateKey::from_der(pki.private_key.as_bytes())
            .map_err(|_| CryptoError::InvalidKey)?;
        if rsa.other_prime_infos.is_some() {
            return Err(CryptoError::InvalidKey);
        }
        let modulus = rsa.modulus.as_bytes();
        let public_exponent = rsa.public_exponent.as_bytes();
        validate_components(modulus, public_exponent)?;
        let inner = provider
            .key_provider
            .load_private_key(PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
                bytes.to_vec(),
            )))
            .map_err(|_| CryptoError::InvalidKey)?;
        let key = Self {
            inner,
            modulus: modulus.to_vec(),
            public_exponent: public_exponent.to_vec(),
        };
        let algorithm = RsaAlgorithm::Pkcs1Sha256;
        let signature = key.sign(algorithm, PAIRWISE_MESSAGE)?;
        verify_with_provider(
            provider,
            algorithm,
            &key.modulus,
            &key.public_exponent,
            PAIRWISE_MESSAGE,
            &signature,
        )
        // A signature that does not verify means the backend key does not
        // match the PKCS#8 public components; anything else is a backend gap.
        .map_err(|error| match error {
            CryptoError::InvalidSignature => CryptoError::InvalidKey,
            other => other,
        })?;
        Ok(key)
    }

    /// Return the modulus `n` as an unsigned big-endian integer without
    /// leading zeros (the JWK `n` member before base64url).
    #[must_use]
    pub fn modulus(&self) -> &[u8] {
        &self.modulus
    }

    /// Return the public exponent `e` in the same encoding as [`Self::modulus`].
    #[must_use]
    pub fn public_exponent(&self) -> &[u8] {
        &self.public_exponent
    }

    /// Sign the message (not prehashed) with the exact algorithm requested.
    /// The signature is as long as the modulus.
    ///
    /// # Errors
    /// Returns `UnsupportedAlgorithm` if the backend does not offer that
    /// scheme for this key, `MessageTooLong` past the portable length limit,
    /// and `SigningFailed` for a native failure or a malformed signature.
    pub fn sign(&self, algorithm: RsaAlgorithm, message: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if message.len() > u32::MAX as usize {
            return Err(CryptoError::MessageTooLong);
        }
        let signer = self
            .inner
            .choose_scheme(&[algorithm.scheme()])
            .ok_or(CryptoError::UnsupportedAlgorithm)?;
        // A backend must never substitute a different scheme.
        if signer.scheme() != algorithm.scheme() {
            return Err(CryptoError::UnsupportedAlgorithm);
        }
        let signature = signer
            .sign(message)
            .map_err(|_| CryptoError::SigningFailed)?;
        if signature.len() != self.modulus.len() {
            return Err(CryptoError::SigningFailed);
        }
        Ok(signature)
    }
}

/// Verify an RSA signature over the original message (without prehashing).
///
/// `n` and `e` are unsigned big-endian integers without leading zero bytes,
/// as in a JWK. Moduli must be odd and 2048–8192 bits. Exponents must be odd,
/// at least 3 and at most 33 bits. The selected algorithm is never substituted.
///
/// # Errors
/// Returns an error for invalid key/signature bytes, an oversized message,
/// unavailable backend or unsupported algorithm. No fallback is attempted.
pub fn verify_rsa_components(
    algorithm: RsaAlgorithm,
    n: &[u8],
    e: &[u8],
    message: &[u8],
    signature: &[u8],
) -> Result<(), CryptoError> {
    verify_with_provider(backend::provider()?, algorithm, n, e, message, signature)
}

// The public entry point validates the production provider before reaching here.
pub(super) fn verify_with_provider(
    provider: &rustls::crypto::CryptoProvider,
    algorithm: RsaAlgorithm,
    n: &[u8],
    e: &[u8],
    message: &[u8],
    signature: &[u8],
) -> Result<(), CryptoError> {
    validate_components(n, e)?;
    if message.len() > u32::MAX as usize {
        return Err(CryptoError::MessageTooLong);
    }
    if signature.len() != n.len() {
        return Err(CryptoError::InvalidSignature);
    }
    let public_key = pkcs1::RsaPublicKey {
        modulus: pkcs1::UintRef::new(n).map_err(|_| CryptoError::InvalidKey)?,
        public_exponent: pkcs1::UintRef::new(e).map_err(|_| CryptoError::InvalidKey)?,
    }
    .to_der()
    .map_err(|_| CryptoError::InvalidKey)?;
    let algorithms = provider
        .signature_verification_algorithms
        .mapping
        .iter()
        .find(|(scheme, _)| *scheme == algorithm.scheme())
        .map(|(_, algorithms)| *algorithms)
        .ok_or(CryptoError::UnsupportedAlgorithm)?;
    if algorithms.iter().any(|verifier| {
        verifier.public_key_alg_id() == alg_id::RSA_ENCRYPTION
            && verifier.signature_alg_id() == algorithm.signature_id()
            && verifier
                .verify_signature(&public_key, message, signature)
                .is_ok()
    }) {
        Ok(())
    } else {
        Err(CryptoError::InvalidSignature)
    }
}

fn validate_components(n: &[u8], e: &[u8]) -> Result<(), CryptoError> {
    // Check canonical encodings before UintRef normalizes leading zeros.
    if n.is_empty() || e.is_empty() || n[0] == 0 || e[0] == 0 {
        return Err(CryptoError::InvalidKey);
    }
    if n.len() > 1024 || e.len() > 5 {
        return Err(CryptoError::InvalidKey);
    }
    let modulus_bits = modulus_bits(n);
    let exponent = e
        .iter()
        .fold(0u64, |value, byte| (value << 8) | u64::from(*byte));
    if !(2048..=8192).contains(&modulus_bits)
        || n[n.len() - 1] & 1 == 0
        || exponent < 3
        || exponent & 1 == 0
        || exponent >= (1u64 << 33)
    {
        return Err(CryptoError::InvalidKey);
    }
    Ok(())
}

/// Bit length of a canonical (no leading zero byte) big-endian integer.
fn modulus_bits(n: &[u8]) -> usize {
    n.first()
        .map_or(0, |first| n.len() * 8 - first.leading_zeros() as usize)
}

#[cfg(test)]
#[path = "rsa_tests.rs"]
mod tests;
