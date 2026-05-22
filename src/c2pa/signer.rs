//! C2PA signer construction.
//!
//! Two paths:
//!
//! - [`load_test_signer`] returns an ES256 signer built from the test cert
//!   bundled with the crate (`assets/c2pa/es256.{key,pub}`, the same fixture
//!   the upstream `c2pa-rs` test suite uses). Verifiers flag this as untrusted
//!   — fine for development, not fine for production.
//! - [`build_signer`] constructs a signer from caller-supplied PEM bytes.
//!   Production deployments use this with a real cert chain issued by a
//!   C2PA-trusted CA (or, eventually, a Polar.sh-issued org cert).
//!
//! The test cert + key are **not** secrets — they're public fixtures. Shipping
//! them in the binary is intentional so that `wavelet c2pa sign` works zero-config
//! out of the box. Anything signed with the test cert verifies hash-chain-wise
//! but won't carry a trusted-issuer badge in Content Credentials UIs.

use super::C2paError;
use c2pa::{create_signer, Signer, SigningAlg};

/// Claim-generator string embedded in every wavelet-signed manifest. Surfaced
/// by Content Credentials viewers as "Signed by …".
pub const WAVELET_CLAIM_GENERATOR: &str =
    "wavelet-director v0 (https://github.com/shinyobjectz-sh/workbooks)";

const TEST_CERT_PEM: &[u8] = include_bytes!("../../assets/c2pa/es256.pub");
const TEST_KEY_PEM: &[u8] = include_bytes!("../../assets/c2pa/es256.key");

/// Caller-supplied signing material. Both fields hold raw PEM bytes (cert
/// chain in `cert_pem`, private key in `key_pem`).
#[derive(Debug, Clone)]
pub struct SigningKey {
    /// PEM-encoded certificate chain (leaf first, then intermediates).
    pub cert_pem: Vec<u8>,
    /// PEM-encoded private key matching the leaf cert.
    pub key_pem: Vec<u8>,
    /// Signing algorithm. Must match the key type.
    pub alg: SigningAlg,
}

/// Build a C2PA signer from caller-supplied PEM bytes. The returned signer
/// owns its key material; the input slices can be dropped.
pub fn build_signer(key: SigningKey) -> Result<Box<dyn Signer + Send + Sync>, C2paError> {
    create_signer::from_keys(&key.cert_pem, &key.key_pem, key.alg, None).map_err(C2paError::Sdk)
}

/// Build the bundled-test ES256 signer. Use only for development and tests —
/// outputs won't carry a trusted-issuer badge in C2PA UIs.
pub fn load_test_signer() -> Result<Box<dyn Signer + Send + Sync>, C2paError> {
    create_signer::from_keys(TEST_CERT_PEM, TEST_KEY_PEM, SigningAlg::Es256, None)
        .map_err(C2paError::Sdk)
}
