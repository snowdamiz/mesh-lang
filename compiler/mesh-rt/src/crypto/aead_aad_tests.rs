//! The transport contract permits 128 KiB of authenticated metadata.
//! Exercise provider and public-runtime validation independently of payload size.
use super::*;
use crate::actor::{Priority, ProcessId};
use crate::secret::destroy_owned;

// Contract literals intentionally do not derive from the implementation limit.
const AAD_LIMIT: usize = 131_072;
const BODY: &[u8] = b"bounded transport payload";

#[derive(Clone, Copy)]
enum Algorithm {
    ChaCha,
    XChaCha,
}

impl Algorithm {
    fn seal(self, aad: &[u8]) -> Result<Vec<u8>, ProviderError> {
        match self {
            Self::ChaCha => SystemProvider.chacha20poly1305_seal(&[19; 32], &[7; 12], aad, BODY),
            Self::XChaCha => SystemProvider.xchacha20poly1305_seal(&[19; 32], &[7; 24], aad, BODY),
        }
    }

    fn open(self, aad: &[u8], output: &mut Zeroizing<Vec<u8>>) -> Result<(), ProviderError> {
        match self {
            Self::ChaCha => SystemProvider.chacha20poly1305_open(&[19; 32], &[7; 12], aad, output),
            Self::XChaCha => {
                SystemProvider.xchacha20poly1305_open(&[19; 32], &[7; 24], aad, output)
            }
        }
    }

    fn ffi_seal(
        self,
        process: &Process,
        key: *const MeshSecretHandle,
        aad: &[u8],
    ) -> Result<Vec<u8>, CryptoFailure> {
        match self {
            Self::ChaCha => aead_seal_for_process(
                process,
                &SystemProvider,
                key,
                bytes(&[7; 12]),
                bytes(aad),
                bytes(BODY),
            ),
            Self::XChaCha => xchacha20poly1305_seal_for_process(
                process,
                &SystemProvider,
                key,
                bytes(&[7; 24]),
                bytes(aad),
                bytes(BODY),
            ),
        }
    }

    fn ffi_open(
        self,
        process: &Process,
        key: *const MeshSecretHandle,
        aad: &[u8],
        ciphertext: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, CryptoFailure> {
        match self {
            Self::ChaCha => aead_open_for_process(
                process,
                &SystemProvider,
                key,
                bytes(&[7; 12]),
                bytes(aad),
                bytes(ciphertext),
            ),
            Self::XChaCha => xchacha20poly1305_open_for_process(
                process,
                &SystemProvider,
                key,
                bytes(&[7; 24]),
                bytes(aad),
                bytes(ciphertext),
            ),
        }
    }
}

fn bytes(input: &[u8]) -> *mut MeshBytes {
    mesh_bytes_new(input.as_ptr(), input.len() as u64)
}

fn provider_contract(algorithm: Algorithm) {
    for length in [65_536, 65_537, AAD_LIMIT] {
        let mut aad = vec![0x37; length];
        let ciphertext = algorithm
            .seal(&aad)
            .expect("AAD within the 128-KiB contract must seal");
        let mut output = Zeroizing::new(ciphertext.clone());
        algorithm.open(&aad, &mut output).unwrap();
        assert_eq!(&output[..], BODY);
        // Authentication must cover the tail beyond the former 64-KiB bound.
        aad[length - 1] ^= 1;
        let mut output = Zeroizing::new(ciphertext);
        assert_eq!(
            algorithm.open(&aad, &mut output),
            Err(ProviderError::AuthenticationFailed)
        );
        assert!(
            output.is_empty(),
            "authentication failure retained plaintext"
        );
    }
    let oversized = vec![0x37; AAD_LIMIT + 1];
    assert_eq!(
        algorithm.seal(&oversized),
        Err(ProviderError::InvalidLength)
    );
    let valid = algorithm.seal(b"retry").unwrap();
    let mut output = Zeroizing::new(valid.clone());
    assert_eq!(
        algorithm.open(&oversized, &mut output),
        Err(ProviderError::InvalidLength)
    );
    assert!(output.is_empty(), "length failure retained output");
    let mut output = Zeroizing::new(valid);
    algorithm.open(b"retry", &mut output).unwrap();
    assert_eq!(&output[..], BODY);
}

struct OwnerGuard(ProcessId);
impl Drop for OwnerGuard {
    fn drop(&mut self) {
        destroy_owned(self.0);
    }
}

fn ffi_contract(algorithm: Algorithm, owner: ProcessId) {
    crate::mesh_rt_init();
    let _cleanup = OwnerGuard(owner);
    let mut process = Process::new(owner, Priority::Normal);
    let material = insert_owned_resource(
        &mut process,
        ResourceKind::SecretBytes,
        Zeroizing::new(vec![19; 32].into_boxed_slice()),
    )
    .unwrap();
    let key = aead_key_for_process(&mut process, material).unwrap();
    for length in [65_536, 65_537, AAD_LIMIT] {
        let mut aad = vec![0x37; length];
        let ciphertext = algorithm
            .ffi_seal(&process, key, &aad)
            .expect("public runtime must admit bounded AAD");
        assert_eq!(ciphertext, algorithm.seal(&aad).unwrap());
        assert_eq!(
            &algorithm
                .ffi_open(&process, key, &aad, &ciphertext)
                .unwrap()[..],
            BODY
        );
        aad[length - 1] ^= 1;
        assert_eq!(
            algorithm
                .ffi_open(&process, key, &aad, &ciphertext)
                .unwrap_err()
                .tag,
            CryptoErrorTag::AuthenticationFailed
        );
    }
    let oversized = vec![0x37; AAD_LIMIT + 1];
    let valid = algorithm.ffi_seal(&process, key, b"retry").unwrap();
    for error in [
        algorithm.ffi_seal(&process, key, &oversized).unwrap_err(),
        algorithm
            .ffi_open(&process, key, &oversized, &valid)
            .unwrap_err(),
    ] {
        assert_eq!(
            error,
            failure(
                CryptoErrorTag::InvalidLength,
                AAD_LIMIT as i64,
                (AAD_LIMIT + 1) as i64
            )
        );
    }
    // Both validation and authentication failures leave the borrowed key usable.
    assert_eq!(
        &algorithm.ffi_open(&process, key, b"retry", &valid).unwrap()[..],
        BODY
    );
    assert_eq!(algorithm.ffi_seal(&process, key, b"retry").unwrap(), valid);
}

#[test]
fn chacha_provider_accepts_bounded_aad_and_clears_failed_open() {
    provider_contract(Algorithm::ChaCha);
}
#[test]
fn xchacha_provider_accepts_bounded_aad_and_clears_failed_open() {
    provider_contract(Algorithm::XChaCha);
}
#[test]
fn chacha_ffi_accepts_bounded_aad_and_preserves_borrowed_key() {
    ffi_contract(Algorithm::ChaCha, ProcessId(980_101));
}
#[test]
fn xchacha_ffi_accepts_bounded_aad_and_preserves_borrowed_key() {
    ffi_contract(Algorithm::XChaCha, ProcessId(980_102));
}
