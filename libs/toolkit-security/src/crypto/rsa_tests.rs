use super::{CryptoError, RsaAlgorithm, validate_components, verify_rsa_components};
#[cfg(feature = "jwt")]
use super::{SignatureScheme, verify_with_provider};

#[test]
fn rejects_noncanonical_or_weak_public_components() {
    let valid = vec![0xff; 256];
    assert_eq!(validate_components(&valid, &[1, 0, 1]), Ok(()));
    for n in [vec![], vec![0; 256], vec![0xff; 255], vec![0xff; 1025]] {
        assert_eq!(validate_components(&n, &[3]), Err(CryptoError::InvalidKey));
    }
    let mut too_small = valid.clone();
    too_small[0] = 0x7f;
    assert_eq!(
        validate_components(&too_small, &[3]),
        Err(CryptoError::InvalidKey)
    );
    let mut even = valid.clone();
    even[255] = 0xfe;
    assert_eq!(
        validate_components(&even, &[3]),
        Err(CryptoError::InvalidKey)
    );
    for e in [
        vec![],
        vec![0, 3],
        vec![1],
        vec![2],
        vec![4],
        vec![2, 0, 0, 0, 1],
    ] {
        assert_eq!(
            validate_components(&valid, &e),
            Err(CryptoError::InvalidKey)
        );
    }
    assert_eq!(
        validate_components(&vec![0xff; 1024], &[1, 0xff, 0xff, 0xff, 0xff]),
        Ok(())
    );
}

#[test]
fn rejects_wrong_signature_size_before_dispatch() {
    assert_eq!(
        verify_rsa_components(
            RsaAlgorithm::Pkcs1Sha256,
            &vec![0xff; 256],
            &[3],
            b"message",
            &[0; 255]
        ),
        Err(CryptoError::InvalidSignature)
    );
}

#[cfg(feature = "jwt")]
#[test]
fn provider_mapping_cannot_substitute_a_different_rsa_hash() {
    use aws_lc_rs::{
        rand::SystemRandom,
        signature::{self, KeyPair},
    };
    use pkcs1::der::Decode;

    let encoding =
        jsonwebtoken::EncodingKey::from_rsa_pem(include_bytes!("testdata/rsa-private.pem"))
            .unwrap();
    let key = signature::RsaKeyPair::from_pkcs8(encoding.inner())
        .or_else(|_| signature::RsaKeyPair::from_der(encoding.inner()))
        .unwrap();
    let public = pkcs1::RsaPublicKey::from_der(key.public_key().as_ref()).unwrap();
    let message = b"exact RSA hash selection";
    let mut signature = vec![0; key.public_modulus_len()];
    key.sign(
        &signature::RSA_PKCS1_SHA384,
        &SystemRandom::new(),
        message,
        &mut signature,
    )
    .unwrap();
    let mut provider = rustls::crypto::aws_lc_rs::default_provider();
    let sha384 = provider
        .signature_verification_algorithms
        .mapping
        .iter()
        .find(|(scheme, _)| *scheme == SignatureScheme::RSA_PKCS1_SHA384)
        .unwrap()
        .1;
    provider.signature_verification_algorithms.mapping =
        Box::leak(vec![(SignatureScheme::RSA_PKCS1_SHA256, sha384)].into_boxed_slice());
    assert_eq!(
        verify_with_provider(
            &provider,
            RsaAlgorithm::Pkcs1Sha256,
            public.modulus.as_bytes(),
            public.public_exponent.as_bytes(),
            message,
            &signature
        ),
        Err(CryptoError::InvalidSignature)
    );
}

mod key_pair {
    use super::super::{RsaAlgorithm, RsaKeyPair, verify_rsa_components};
    use crate::crypto::CryptoError;
    use aws_lc_rs::signature::{self, UnparsedPublicKey};
    use pem_rfc7468::LineEnding;
    use pkcs1::der::Encode as _;

    const PEM: &str = include_str!("testdata/rsa-private.pem");
    const PEM_1024: &str = include_str!("testdata/rsa-1024-private.pem");
    const EC_KEY: &[u8] = include_bytes!("../../tests/fixtures/p256-test-key.pk8");
    const MESSAGE: &[u8] = b"toolkit-security RSA signing";

    fn der() -> Vec<u8> {
        pem_rfc7468::decode_vec(PEM.as_bytes()).unwrap().1
    }

    fn public_der(key: &RsaKeyPair) -> Vec<u8> {
        pkcs1::RsaPublicKey {
            modulus: pkcs1::UintRef::new(key.modulus()).unwrap(),
            public_exponent: pkcs1::UintRef::new(key.public_exponent()).unwrap(),
        }
        .to_der()
        .unwrap()
    }

    #[test]
    fn signatures_verify_with_the_facade_and_an_aws_lc_oracle() {
        let key = RsaKeyPair::from_pkcs8_pem(PEM).unwrap();
        assert_eq!(key.modulus().len(), 256);
        let public = public_der(&key);
        for (algorithm, oracle) in [
            (
                RsaAlgorithm::Pkcs1Sha256,
                &signature::RSA_PKCS1_2048_8192_SHA256,
            ),
            (
                RsaAlgorithm::Pkcs1Sha384,
                &signature::RSA_PKCS1_2048_8192_SHA384,
            ),
            (
                RsaAlgorithm::Pkcs1Sha512,
                &signature::RSA_PKCS1_2048_8192_SHA512,
            ),
            (
                RsaAlgorithm::PssSha256,
                &signature::RSA_PSS_2048_8192_SHA256,
            ),
            (
                RsaAlgorithm::PssSha384,
                &signature::RSA_PSS_2048_8192_SHA384,
            ),
            (
                RsaAlgorithm::PssSha512,
                &signature::RSA_PSS_2048_8192_SHA512,
            ),
        ] {
            let sig = key.sign(algorithm, MESSAGE).unwrap();
            assert_eq!(sig.len(), 256, "{algorithm:?}");
            verify_rsa_components(
                algorithm,
                key.modulus(),
                key.public_exponent(),
                MESSAGE,
                &sig,
            )
            .unwrap();
            // Independent oracle: native macOS/Windows signatures must verify in AWS-LC.
            UnparsedPublicKey::new(oracle, &public)
                .verify(MESSAGE, &sig)
                .unwrap();
            assert!(
                UnparsedPublicKey::new(oracle, &public)
                    .verify(b"tampered", &sig)
                    .is_err()
            );
        }
        // The scheme is exact: a PSS signature is not a PKCS#1 v1.5 one.
        let pss = key.sign(RsaAlgorithm::PssSha256, MESSAGE).unwrap();
        assert_eq!(
            verify_rsa_components(
                RsaAlgorithm::Pkcs1Sha256,
                key.modulus(),
                key.public_exponent(),
                MESSAGE,
                &pss
            ),
            Err(CryptoError::InvalidSignature)
        );
    }

    #[test]
    fn der_and_pem_imports_agree() {
        let from_der = RsaKeyPair::from_pkcs8(&der()).unwrap();
        let from_pem = RsaKeyPair::from_pkcs8_pem(&format!("\n  {PEM}\n")).unwrap();
        assert_eq!(from_der.modulus(), from_pem.modulus());
        assert_eq!(from_der.public_exponent(), [1, 0, 1]);
        assert!(RsaKeyPair::from_pkcs8_pem(&PEM.replace('\n', "\r\n")).is_ok());
    }

    #[test]
    fn rejects_other_labels_weak_moduli_pss_restricted_and_non_rsa_keys() {
        let der = der();
        for label in ["RSA PRIVATE KEY", "ENCRYPTED PRIVATE KEY", "PUBLIC KEY"] {
            let other = pem_rfc7468::encode_string(label, LineEnding::LF, &der).unwrap();
            assert!(RsaKeyPair::from_pkcs8_pem(&other).is_err(), "{label}");
        }
        assert!(RsaKeyPair::from_pkcs8_pem("").is_err());
        assert!(RsaKeyPair::from_pkcs8_pem(&PEM[..64]).is_err());
        assert_eq!(
            RsaKeyPair::from_pkcs8_pem(PEM_1024).unwrap_err(),
            CryptoError::InvalidKey
        );
        // rsaEncryption's last arc changed to RSASSA-PSS (1.2.840.113549.1.1.10).
        let mut pss = der.clone();
        let oid_at = pss
            .windows(11)
            .position(|w| w == [6, 9, 42, 134, 72, 134, 247, 13, 1, 1, 1])
            .unwrap();
        pss[oid_at + 10] = 10;
        assert_eq!(
            RsaKeyPair::from_pkcs8(&pss).unwrap_err(),
            CryptoError::InvalidKey
        );
        assert_eq!(
            RsaKeyPair::from_pkcs8(EC_KEY).unwrap_err(),
            CryptoError::InvalidKey
        );
        for length in [0, 32, 256] {
            assert!(RsaKeyPair::from_pkcs8(&vec![0; length]).is_err());
        }
        // A flipped bit inside the private key must fail import (backend
        // parse or the pairwise check), never yield a key that signs garbage.
        let mut corrupted = der;
        let at = corrupted.len() - 40;
        corrupted[at] ^= 1;
        assert!(RsaKeyPair::from_pkcs8(&corrupted).is_err());
    }

    #[test]
    fn debug_output_shows_only_the_modulus_size() {
        let key = RsaKeyPair::from_pkcs8(&der()).unwrap();
        let rendered = format!("{key:?}");
        assert!(rendered.contains("modulus_bits: 2048"), "{rendered}");
        // Neither the modulus (decimal byte list) nor anything key-sized.
        let modulus = format!("{:?}", &key.modulus()[..8]);
        assert!(
            !rendered.contains(&modulus[1..modulus.len() - 1]),
            "{rendered}"
        );
        assert!(rendered.len() < 64, "{rendered}");
    }

    #[test]
    fn rejects_non_null_rsa_encryption_parameters() {
        // AlgorithmIdentifier { rsaEncryption, NULL } with NULL (05 00)
        // swapped for an empty OCTET STRING (04 00): same length, no refit.
        let mut der = der();
        let at = der
            .windows(13)
            .position(|w| w == [6, 9, 42, 134, 72, 134, 247, 13, 1, 1, 1, 5, 0])
            .unwrap();
        der[at + 11] = 4;
        assert_eq!(
            RsaKeyPair::from_pkcs8_with_provider(&der, &unreachable_backend()).unwrap_err(),
            CryptoError::InvalidKey
        );
    }

    /// The facade's own structural checks must reject the key before any
    /// backend sees it: AWS-LC is strict, other backends may not be.
    #[derive(Debug)]
    struct UnreachableKeyProvider;

    impl rustls::crypto::KeyProvider for UnreachableKeyProvider {
        fn load_private_key(
            &self,
            _: rustls::pki_types::PrivateKeyDer<'static>,
        ) -> Result<std::sync::Arc<dyn rustls::sign::SigningKey>, rustls::Error> {
            panic!("structurally invalid key reached the backend")
        }
    }

    fn unreachable_backend() -> rustls::crypto::CryptoProvider {
        rustls::crypto::CryptoProvider {
            key_provider: &UnreachableKeyProvider,
            ..rustls::crypto::aws_lc_rs::default_provider()
        }
    }

    #[test]
    fn rejects_multi_prime_keys() {
        let pem = include_str!("testdata/rsa-3prime-private.pem");
        let der = pem_rfc7468::decode_vec(pem.as_bytes()).unwrap().1;
        assert_eq!(
            RsaKeyPair::from_pkcs8_with_provider(&der, &unreachable_backend()).unwrap_err(),
            CryptoError::InvalidKey
        );
        assert!(RsaKeyPair::from_pkcs8_pem(pem).is_err());
    }

    #[test]
    fn pairwise_check_rejects_a_key_whose_signature_does_not_verify() {
        use super::super::RsaKeyPair as Pair;
        use rustls::SignatureScheme;
        // The verifier mapped to PKCS1_SHA256 is really SHA-384, so the
        // import-time self-signature cannot verify.
        let mut provider = rustls::crypto::aws_lc_rs::default_provider();
        let sha384 = provider
            .signature_verification_algorithms
            .mapping
            .iter()
            .find(|(scheme, _)| *scheme == SignatureScheme::RSA_PKCS1_SHA384)
            .unwrap()
            .1;
        provider.signature_verification_algorithms.mapping =
            Box::leak(vec![(SignatureScheme::RSA_PKCS1_SHA256, sha384)].into_boxed_slice());
        assert_eq!(
            Pair::from_pkcs8_with_provider(&der(), &provider).unwrap_err(),
            CryptoError::InvalidKey
        );
        assert!(
            Pair::from_pkcs8_with_provider(&der(), &rustls::crypto::aws_lc_rs::default_provider())
                .is_ok()
        );
    }

    /// Backend double: signs with a fixed scheme and output length.
    #[derive(Debug)]
    struct FakeKey {
        scheme: rustls::SignatureScheme,
        len: usize,
    }

    impl rustls::sign::SigningKey for FakeKey {
        fn choose_scheme(
            &self,
            _: &[rustls::SignatureScheme],
        ) -> Option<Box<dyn rustls::sign::Signer>> {
            Some(Box::new(FakeSigner {
                scheme: self.scheme,
                len: self.len,
            }))
        }

        fn algorithm(&self) -> rustls::SignatureAlgorithm {
            rustls::SignatureAlgorithm::RSA
        }
    }

    #[derive(Debug)]
    struct FakeSigner {
        scheme: rustls::SignatureScheme,
        len: usize,
    }

    impl rustls::sign::Signer for FakeSigner {
        fn sign(&self, _: &[u8]) -> Result<Vec<u8>, rustls::Error> {
            Ok(vec![1; self.len])
        }

        fn scheme(&self) -> rustls::SignatureScheme {
            self.scheme
        }
    }

    fn with_backend(scheme: rustls::SignatureScheme, len: usize) -> RsaKeyPair {
        RsaKeyPair {
            inner: std::sync::Arc::new(FakeKey { scheme, len }),
            modulus: vec![0xff; 256],
            public_exponent: vec![1, 0, 1],
        }
    }

    #[test]
    fn sign_refuses_a_substituted_scheme_or_a_wrong_length_signature() {
        use rustls::SignatureScheme;
        assert_eq!(
            with_backend(SignatureScheme::RSA_PSS_SHA256, 256)
                .sign(RsaAlgorithm::Pkcs1Sha256, MESSAGE)
                .unwrap_err(),
            CryptoError::UnsupportedAlgorithm
        );
        assert_eq!(
            with_backend(SignatureScheme::RSA_PKCS1_SHA256, 255)
                .sign(RsaAlgorithm::Pkcs1Sha256, MESSAGE)
                .unwrap_err(),
            CryptoError::SigningFailed
        );
        assert_eq!(
            with_backend(SignatureScheme::RSA_PKCS1_SHA256, 256)
                .sign(RsaAlgorithm::Pkcs1Sha256, MESSAGE)
                .unwrap()
                .len(),
            256
        );
    }
}
