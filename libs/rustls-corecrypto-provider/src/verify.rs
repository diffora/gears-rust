//! Signature verification algorithms wired into [`rustls::crypto::WebPkiSupportedAlgorithms`].
//!
//! Each algorithm is a unit struct implementing
//! [`rustls::crypto::SignatureVerificationAlgorithm`]. The trait is invoked
//! by rustls when validating server certificates and the TLS 1.3
//! `CertificateVerify` message.
//!
//! Public-key bytes arrive in their conventional X.509 SPKI subjectPublicKey
//! form:
//! - ECDSA: uncompressed point (`0x04 || X || Y`).
//! - RSA: DER-encoded `RSAPublicKey` (`SEQUENCE { modulus, publicExponent }`).
//!
//! Signature bytes:
//! - ECDSA: DER `ECDSA-Sig-Value { r, s }`.
//! - RSA-PSS, RSA-PKCS#1 v1.5: raw signature bytes.
//!
//! Verification delegates to `SecKey::verify_signature` after importing the
//! public key via [`crate::ffi::security::import_public_key`].

use rustls::SignatureScheme;
use rustls::crypto::WebPkiSupportedAlgorithms;
use rustls::pki_types::AlgorithmIdentifier;
use rustls::pki_types::InvalidSignature;
use rustls::pki_types::SignatureVerificationAlgorithm;
use rustls::pki_types::alg_id;
use security_framework::key::Algorithm;

use crate::ffi::security::{PublicKeyKind, import_public_key};

// =========================================================================
// Algorithm unit structs
// =========================================================================

#[derive(Debug)]
struct EcdsaP256Sha256;
#[derive(Debug)]
struct EcdsaP384Sha384;
#[derive(Debug)]
struct EcdsaP521Sha512;

#[derive(Debug)]
struct RsaPssSha256;
#[derive(Debug)]
struct RsaPssSha384;
#[derive(Debug)]
struct RsaPssSha512;

#[derive(Debug)]
struct RsaPkcs1Sha256;
#[derive(Debug)]
struct RsaPkcs1Sha384;
#[derive(Debug)]
struct RsaPkcs1Sha512;

// =========================================================================
// SignatureVerificationAlgorithm impls
// =========================================================================

impl SignatureVerificationAlgorithm for EcdsaP256Sha256 {
    fn verify_signature(
        &self,
        public_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> Result<(), InvalidSignature> {
        verify_with_seckey(
            PublicKeyKind::EcSecPrimeRandomP256,
            Algorithm::ECDSASignatureMessageX962SHA256,
            public_key,
            message,
            signature,
        )
    }
    fn public_key_alg_id(&self) -> AlgorithmIdentifier {
        alg_id::ECDSA_P256
    }
    fn signature_alg_id(&self) -> AlgorithmIdentifier {
        alg_id::ECDSA_SHA256
    }
    fn fips(&self) -> bool {
        // Runtime witness — see [`crate::oe::fips_witness_ok`].
        crate::oe::fips_witness_ok()
    }
}

impl SignatureVerificationAlgorithm for EcdsaP384Sha384 {
    fn verify_signature(
        &self,
        public_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> Result<(), InvalidSignature> {
        verify_with_seckey(
            PublicKeyKind::EcSecPrimeRandomP384,
            Algorithm::ECDSASignatureMessageX962SHA384,
            public_key,
            message,
            signature,
        )
    }
    fn public_key_alg_id(&self) -> AlgorithmIdentifier {
        alg_id::ECDSA_P384
    }
    fn signature_alg_id(&self) -> AlgorithmIdentifier {
        alg_id::ECDSA_SHA384
    }
    fn fips(&self) -> bool {
        // Runtime witness — see [`crate::oe::fips_witness_ok`].
        crate::oe::fips_witness_ok()
    }
}

impl SignatureVerificationAlgorithm for EcdsaP521Sha512 {
    fn verify_signature(
        &self,
        public_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> Result<(), InvalidSignature> {
        verify_with_seckey(
            PublicKeyKind::EcSecPrimeRandomP521,
            Algorithm::ECDSASignatureMessageX962SHA512,
            public_key,
            message,
            signature,
        )
    }
    fn public_key_alg_id(&self) -> AlgorithmIdentifier {
        alg_id::ECDSA_P521
    }
    fn signature_alg_id(&self) -> AlgorithmIdentifier {
        alg_id::ECDSA_SHA512
    }
    fn fips(&self) -> bool {
        // Runtime witness — see [`crate::oe::fips_witness_ok`].
        crate::oe::fips_witness_ok()
    }
}

macro_rules! impl_rsa_pss {
    ($ty:ty, $alg:ident, $sig_alg_id:ident) => {
        impl SignatureVerificationAlgorithm for $ty {
            fn verify_signature(
                &self,
                public_key: &[u8],
                message: &[u8],
                signature: &[u8],
            ) -> Result<(), InvalidSignature> {
                verify_with_seckey(
                    PublicKeyKind::RsaPkcs1,
                    Algorithm::$alg,
                    public_key,
                    message,
                    signature,
                )
            }
            fn public_key_alg_id(&self) -> AlgorithmIdentifier {
                alg_id::RSA_ENCRYPTION
            }
            fn signature_alg_id(&self) -> AlgorithmIdentifier {
                alg_id::$sig_alg_id
            }
            fn fips(&self) -> bool {
                // Runtime witness — see [`crate::oe::fips_witness_ok`].
                crate::oe::fips_witness_ok()
            }
        }
    };
}

impl_rsa_pss!(RsaPssSha256, RSASignatureMessagePSSSHA256, RSA_PSS_SHA256);
impl_rsa_pss!(RsaPssSha384, RSASignatureMessagePSSSHA384, RSA_PSS_SHA384);
impl_rsa_pss!(RsaPssSha512, RSASignatureMessagePSSSHA512, RSA_PSS_SHA512);

macro_rules! impl_rsa_pkcs1 {
    ($ty:ty, $alg:ident, $sig_alg_id:ident) => {
        impl SignatureVerificationAlgorithm for $ty {
            fn verify_signature(
                &self,
                public_key: &[u8],
                message: &[u8],
                signature: &[u8],
            ) -> Result<(), InvalidSignature> {
                verify_with_seckey(
                    PublicKeyKind::RsaPkcs1,
                    Algorithm::$alg,
                    public_key,
                    message,
                    signature,
                )
            }
            fn public_key_alg_id(&self) -> AlgorithmIdentifier {
                alg_id::RSA_ENCRYPTION
            }
            fn signature_alg_id(&self) -> AlgorithmIdentifier {
                alg_id::$sig_alg_id
            }
            fn fips(&self) -> bool {
                // Runtime witness — see [`crate::oe::fips_witness_ok`].
                crate::oe::fips_witness_ok()
            }
        }
    };
}

impl_rsa_pkcs1!(
    RsaPkcs1Sha256,
    RSASignatureMessagePKCS1v15SHA256,
    RSA_PKCS1_SHA256
);
impl_rsa_pkcs1!(
    RsaPkcs1Sha384,
    RSASignatureMessagePKCS1v15SHA384,
    RSA_PKCS1_SHA384
);
impl_rsa_pkcs1!(
    RsaPkcs1Sha512,
    RSASignatureMessagePKCS1v15SHA512,
    RSA_PKCS1_SHA512
);

// =========================================================================
// Shared verification helper
// =========================================================================

fn verify_with_seckey(
    kind: PublicKeyKind,
    algorithm: Algorithm,
    public_key: &[u8],
    message: &[u8],
    signature: &[u8],
) -> Result<(), InvalidSignature> {
    let key = import_public_key(public_key, kind).map_err(|_| InvalidSignature)?;
    match key.verify_signature(algorithm, message, signature) {
        Ok(true) => Ok(()),
        Ok(false) | Err(_) => Err(InvalidSignature),
    }
}

// =========================================================================
// WebPkiSupportedAlgorithms static
// =========================================================================

static ALL_SIG_ALGS: &[&'static dyn SignatureVerificationAlgorithm] = &[
    &EcdsaP256Sha256,
    &EcdsaP384Sha384,
    &EcdsaP521Sha512,
    &RsaPssSha256,
    &RsaPssSha384,
    &RsaPssSha512,
    &RsaPkcs1Sha256,
    &RsaPkcs1Sha384,
    &RsaPkcs1Sha512,
];

static MAPPING: &[(
    SignatureScheme,
    &[&'static dyn SignatureVerificationAlgorithm],
)] = &[
    (SignatureScheme::ECDSA_NISTP256_SHA256, &[&EcdsaP256Sha256]),
    (SignatureScheme::ECDSA_NISTP384_SHA384, &[&EcdsaP384Sha384]),
    (SignatureScheme::ECDSA_NISTP521_SHA512, &[&EcdsaP521Sha512]),
    (SignatureScheme::RSA_PSS_SHA256, &[&RsaPssSha256]),
    (SignatureScheme::RSA_PSS_SHA384, &[&RsaPssSha384]),
    (SignatureScheme::RSA_PSS_SHA512, &[&RsaPssSha512]),
    (SignatureScheme::RSA_PKCS1_SHA256, &[&RsaPkcs1Sha256]),
    (SignatureScheme::RSA_PKCS1_SHA384, &[&RsaPkcs1Sha384]),
    (SignatureScheme::RSA_PKCS1_SHA512, &[&RsaPkcs1Sha512]),
];

pub static SUPPORTED_SIG_ALGS: WebPkiSupportedAlgorithms = WebPkiSupportedAlgorithms {
    all: ALL_SIG_ALGS,
    mapping: MAPPING,
};

#[cfg(test)]
#[path = "verify_tests.rs"]
mod tests;
