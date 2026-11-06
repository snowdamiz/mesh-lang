//! Explicit portable ML-KEM-768 backend. Retained Mesh resources store seeds,
//! never libcrux expanded private keys.

use libcrux_ml_kem::mlkem768::{self, MlKem768Ciphertext, MlKem768PrivateKey, MlKem768PublicKey};
use zeroize::{Zeroize, Zeroizing};

use super::{failure, provider_failure, CryptoErrorTag, CryptoFailure, CryptoProvider};

const EXPANDED_PRIVATE_BYTES: usize = 2400;
const _: () = assert!(MlKem768PrivateKey::len() == EXPANDED_PRIVATE_BYTES);
const _: () = assert!(MlKem768PublicKey::len() == super::MLKEM_PUBLIC_KEY_BYTES);
const _: () = assert!(MlKem768Ciphertext::len() == super::MLKEM_CIPHERTEXT_BYTES);

// Upstream's expanded private-key container has no zeroizing Drop. Keep it
// behind a non-Clone owner so each returned expanded buffer has one wipe.
struct ExpandedPrivateKey(MlKem768PrivateKey);

impl Drop for ExpandedPrivateKey {
    fn drop(&mut self) {
        self.0[0..EXPANDED_PRIVATE_BYTES].zeroize();
        #[cfg(test)]
        LAST_DROPPED_PRIVATE_WAS_ZERO.with(|value| {
            value.set(Some(self.0.as_ref().iter().all(|byte| *byte == 0)));
        });
    }
}

fn expand(seed: &[u8; 64]) -> (ExpandedPrivateKey, MlKem768PublicKey) {
    // The upstream API takes its input by value. Wipe this adapter's own
    // seed copy; this does not claim to erase every library stack/register copy.
    let randomness = Zeroizing::new(*seed);
    let (private, public) = mlkem768::portable::generate_key_pair(*randomness).into_parts();
    (ExpandedPrivateKey(private), public)
}

pub(super) fn public_key(seed: &[u8; 64]) -> Vec<u8> {
    let (_private, public) = expand(seed);
    public.as_ref().to_vec()
}

/// Extract only public key parts through the existing wiped expanded-key owner.
pub(super) fn public_parts(seed: &[u8; 64]) -> ([u8; 64], [u8; 1152]) {
    let (private, public) = expand(seed);
    let mut header = [0u8; 64];
    header[..32].copy_from_slice(&public.as_ref()[1152..1184]);
    // FIPS203 expanded layout: dk_PKE1152 || ek1184 || H(ek)32 || z32.
    header[32..].copy_from_slice(&private.0[2336..2368]);
    let mut vector = [0u8; 1152];
    vector.copy_from_slice(&public.as_ref()[..1152]);
    (header, vector)
}

pub(super) fn encapsulate(
    provider: &impl CryptoProvider,
    public: &[u8; 1184],
) -> Result<(Vec<u8>, Zeroizing<Box<[u8]>>), CryptoFailure> {
    let public = MlKem768PublicKey::from(public);
    // Preserve FIPS 203 modulus validation and its precedence over entropy
    // failure. A fixed-length byte string alone is not an encapsulation key.
    if !mlkem768::portable::validate_public_key(&public) {
        return Err(failure(CryptoErrorTag::InvalidPublicKey, 0, 0));
    }
    let mut randomness = Zeroizing::new([0; super::MLKEM_SHARED_SECRET_BYTES]);
    provider
        .fill_random(randomness.as_mut())
        .map_err(provider_failure)?;
    let (ciphertext, shared) = mlkem768::portable::encapsulate(&public, *randomness);
    let shared = Zeroizing::new(shared);
    Ok((
        ciphertext.as_ref().to_vec(),
        Zeroizing::new(shared.to_vec().into_boxed_slice()),
    ))
}

pub(super) fn decapsulate(seed: &[u8; 64], ciphertext: &[u8; 1088]) -> Zeroizing<Box<[u8]>> {
    let (private, _) = expand(seed);
    // The private key is derived from a checked seed. Do not expose ciphertext
    // validity: every full-length ciphertext returns its implicit-rejection or
    // accepted shared secret, as required by the existing Mesh API.
    let shared = Zeroizing::new(mlkem768::portable::decapsulate(
        &private.0,
        &MlKem768Ciphertext::from(ciphertext),
    ));
    Zeroizing::new(shared.to_vec().into_boxed_slice())
}

#[cfg(test)]
thread_local! {
    static LAST_DROPPED_PRIVATE_WAS_ZERO: std::cell::Cell<Option<bool>> = const {
        std::cell::Cell::new(None)
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expanded_private_owner_erases_before_drop() {
        let (private, _) = mlkem768::portable::generate_key_pair([0x69; 64]).into_parts();
        assert!(private.as_ref().iter().any(|byte| *byte != 0));
        LAST_DROPPED_PRIVATE_WAS_ZERO.with(|value| value.set(None));
        drop(ExpandedPrivateKey(private));
        LAST_DROPPED_PRIVATE_WAS_ZERO.with(|value| assert_eq!(value.get(), Some(true)));
    }

    #[test]
    fn expanded_private_owner_erases_on_unwind() {
        LAST_DROPPED_PRIVATE_WAS_ZERO.with(|value| value.set(None));
        let result = std::panic::catch_unwind(|| {
            let (_private, _) = expand(&[0x69; 64]);
            panic!("synthetic caller failure after expansion");
        });
        assert!(result.is_err());
        LAST_DROPPED_PRIVATE_WAS_ZERO.with(|value| assert_eq!(value.get(), Some(true)));
    }

    #[test]
    fn portable_round_trip_fits_the_runtime_actor_stack() {
        use super::super::provider::FixedProvider;
        use corosensei::{stack::DefaultStack, Coroutine, CoroutineResult};

        let stack = DefaultStack::new(crate::actor::DEFAULT_STACK_SIZE).unwrap();
        let mut actor = Coroutine::<(), (), bool, _>::with_stack(stack, |_, ()| {
            let seed = [0x69; 64];
            let public = public_key(&seed);
            let (ciphertext, sender) = encapsulate(
                &FixedProvider::with_random(&[0x73; 32]),
                public.as_slice().try_into().unwrap(),
            )
            .unwrap();
            let receiver = decapsulate(&seed, ciphertext.as_slice().try_into().unwrap());
            sender.as_ref() == receiver.as_ref()
        });
        assert!(matches!(actor.resume(()), CoroutineResult::Return(true)));
    }
}
