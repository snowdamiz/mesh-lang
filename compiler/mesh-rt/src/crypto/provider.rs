use chacha20poly1305::aead::{AeadInPlace, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Tag};
use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use ring::digest::{digest, SHA256, SHA512};
use ring::hkdf::{KeyType, Salt, HKDF_SHA256};
use ring::hmac::{sign, Key, HMAC_SHA256};
use ring::rand::{SecureRandom, SystemRandom};
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};
use zeroize::{Zeroize, Zeroizing};

const MAX_RANDOM_BYTES: usize = 64 * 1024;
const MAX_INPUT_BYTES: usize = 64 * 1024;
const MAX_HKDF_OUTPUT_BYTES: usize = 255 * 32;
const AEAD_TAG_BYTES: usize = 16;
const MAX_AEAD_CIPHERTEXT_BYTES: usize = MAX_INPUT_BYTES + AEAD_TAG_BYTES;

struct HkdfOutputLength(usize);

impl KeyType for HkdfOutputLength {
    fn len(&self) -> usize {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProviderError {
    EntropyUnavailable,
    InvalidLength,
    InvalidPublicKey,
    AuthenticationFailed,
}

pub(crate) trait CryptoProvider {
    fn fill_random(&self, output: &mut [u8]) -> Result<(), ProviderError>;

    fn sha256(&self, input: &[u8]) -> [u8; 32] {
        let value = digest(&SHA256, input);
        let mut output = [0; 32];
        output.copy_from_slice(value.as_ref());
        output
    }

    fn sha512(&self, input: &[u8]) -> [u8; 64] {
        let value = digest(&SHA512, input);
        let mut output = [0; 64];
        output.copy_from_slice(value.as_ref());
        output
    }

    fn hmac_sha256(
        &self,
        key: &[u8],
        message: &[u8],
        output: &mut [u8; 32],
    ) -> Result<(), ProviderError> {
        if key.len() > MAX_INPUT_BYTES || message.len() > MAX_INPUT_BYTES {
            output.zeroize();
            return Err(ProviderError::InvalidLength);
        }
        let tag = sign(&Key::new(HMAC_SHA256, key), message);
        output.copy_from_slice(tag.as_ref());
        Ok(())
    }

    fn hkdf_sha256(
        &self,
        input_key: &[u8],
        salt: &[u8],
        info: &[u8],
        output: &mut [u8],
    ) -> Result<(), ProviderError> {
        if input_key.len() > MAX_INPUT_BYTES
            || salt.len() > MAX_INPUT_BYTES
            || info.len() > MAX_INPUT_BYTES
            || output.is_empty()
            || output.len() > MAX_HKDF_OUTPUT_BYTES
        {
            output.zeroize();
            return Err(ProviderError::InvalidLength);
        }
        let salt = Salt::new(HKDF_SHA256, salt);
        let pseudo_random_key = salt.extract(input_key);
        let info = [info];
        let Ok(output_key) = pseudo_random_key.expand(&info, HkdfOutputLength(output.len())) else {
            output.zeroize();
            return Err(ProviderError::InvalidLength);
        };
        if output_key.fill(output).is_err() {
            output.zeroize();
            return Err(ProviderError::InvalidLength);
        }
        Ok(())
    }

    fn x25519_public(&self, private_key: &[u8; 32]) -> [u8; 32] {
        let private_key = StaticSecret::from(*private_key);
        X25519PublicKey::from(&private_key).to_bytes()
    }

    fn x25519_shared(
        &self,
        private_key: &[u8; 32],
        peer_public_key: &[u8; 32],
        output: &mut [u8; 32],
    ) -> Result<(), ProviderError> {
        let private_key = StaticSecret::from(*private_key);
        let peer_public_key = X25519PublicKey::from(*peer_public_key);
        let shared_secret = private_key.diffie_hellman(&peer_public_key);
        if !shared_secret.was_contributory() {
            output.zeroize();
            return Err(ProviderError::InvalidPublicKey);
        }
        output.copy_from_slice(shared_secret.as_bytes());
        Ok(())
    }

    fn ed25519_public(&self, private_key: &[u8; 32]) -> [u8; 32] {
        SigningKey::from_bytes(private_key)
            .verifying_key()
            .to_bytes()
    }

    fn ed25519_sign(
        &self,
        private_key: &[u8; 32],
        message: &[u8],
    ) -> Result<[u8; 64], ProviderError> {
        if message.len() > MAX_INPUT_BYTES {
            return Err(ProviderError::InvalidLength);
        }
        Ok(SigningKey::from_bytes(private_key).sign(message).to_bytes())
    }

    fn ed25519_verify(
        &self,
        public_key: &[u8; 32],
        message: &[u8],
        signature: &[u8; 64],
    ) -> Result<bool, ProviderError> {
        if message.len() > MAX_INPUT_BYTES {
            return Err(ProviderError::InvalidLength);
        }
        let public_key =
            VerifyingKey::from_bytes(public_key).map_err(|_| ProviderError::InvalidPublicKey)?;
        let signature = Signature::from_bytes(signature);
        Ok(public_key.verify_strict(message, &signature).is_ok())
    }

    fn chacha20poly1305_seal(
        &self,
        key: &[u8; 32],
        nonce: &[u8; 12],
        associated_data: &[u8],
        plaintext: &[u8],
    ) -> Result<Vec<u8>, ProviderError> {
        if associated_data.len() > MAX_INPUT_BYTES || plaintext.len() > MAX_INPUT_BYTES {
            return Err(ProviderError::InvalidLength);
        }
        let cipher = ChaCha20Poly1305::new(key.into());
        let mut working = Zeroizing::new(plaintext.to_vec());
        let tag = cipher
            .encrypt_in_place_detached(nonce.into(), associated_data, &mut working[..])
            .map_err(|_| ProviderError::InvalidLength)?;
        let mut ciphertext = Vec::with_capacity(working.len() + AEAD_TAG_BYTES);
        ciphertext.extend_from_slice(&working);
        ciphertext.extend_from_slice(&tag);
        Ok(ciphertext)
    }

    fn chacha20poly1305_open(
        &self,
        key: &[u8; 32],
        nonce: &[u8; 12],
        associated_data: &[u8],
        ciphertext: &mut Zeroizing<Vec<u8>>,
    ) -> Result<(), ProviderError> {
        if associated_data.len() > MAX_INPUT_BYTES || ciphertext.len() > MAX_AEAD_CIPHERTEXT_BYTES {
            ciphertext.zeroize();
            return Err(ProviderError::InvalidLength);
        }
        if ciphertext.len() < AEAD_TAG_BYTES {
            ciphertext.zeroize();
            return Err(ProviderError::AuthenticationFailed);
        }
        let cipher = ChaCha20Poly1305::new(key.into());
        let tag_offset = ciphertext.len() - AEAD_TAG_BYTES;
        let mut tag = [0; AEAD_TAG_BYTES];
        tag.copy_from_slice(&ciphertext[tag_offset..]);
        ciphertext.truncate(tag_offset);
        if cipher
            .decrypt_in_place_detached(
                nonce.into(),
                associated_data,
                &mut ciphertext[..],
                Tag::from_slice(&tag),
            )
            .is_err()
        {
            ciphertext.zeroize();
            return Err(ProviderError::AuthenticationFailed);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SystemProvider;

impl CryptoProvider for SystemProvider {
    fn fill_random(&self, output: &mut [u8]) -> Result<(), ProviderError> {
        if output.len() > MAX_RANDOM_BYTES {
            output.zeroize();
            return Err(ProviderError::InvalidLength);
        }
        if SystemRandom::new().fill(output).is_err() {
            output.zeroize();
            return Err(ProviderError::EntropyUnavailable);
        }
        Ok(())
    }
}

#[cfg(any(test, feature = "fuzzing"))]
enum FixedEntropy<'a> {
    Bytes(&'a [u8]),
    #[cfg(test)]
    Failure,
}

#[cfg(any(test, feature = "fuzzing"))]
pub(crate) struct FixedProvider<'a> {
    entropy: FixedEntropy<'a>,
}

#[cfg(any(test, feature = "fuzzing"))]
impl<'a> FixedProvider<'a> {
    pub(crate) fn with_random(bytes: &'a [u8]) -> Self {
        Self {
            entropy: FixedEntropy::Bytes(bytes),
        }
    }

    #[cfg(test)]
    pub(crate) fn entropy_failure() -> Self {
        Self {
            entropy: FixedEntropy::Failure,
        }
    }
}

#[cfg(any(test, feature = "fuzzing"))]
impl CryptoProvider for FixedProvider<'_> {
    fn fill_random(&self, output: &mut [u8]) -> Result<(), ProviderError> {
        let bytes = match self.entropy {
            FixedEntropy::Bytes(bytes) => bytes,
            #[cfg(test)]
            FixedEntropy::Failure => {
                output.zeroize();
                return Err(ProviderError::EntropyUnavailable);
            }
        };
        if output.len() > MAX_RANDOM_BYTES || output.len() > bytes.len() {
            output.zeroize();
            return Err(ProviderError::InvalidLength);
        }
        output.copy_from_slice(&bytes[..output.len()]);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{CryptoProvider, FixedProvider, ProviderError, SystemProvider};
    use zeroize::Zeroizing;

    fn decode_hex<const N: usize>(value: &str) -> [u8; N] {
        assert_eq!(value.len(), N * 2);
        let mut output = [0; N];
        for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
            output[index] = u8::from_str_radix(std::str::from_utf8(chunk).unwrap(), 16).unwrap();
        }
        output
    }

    #[test]
    fn fixed_provider_returns_configured_random_bytes() {
        let provider = FixedProvider::with_random(&[0x11, 0x22, 0x33, 0x44]);
        let mut output = [0; 4];

        provider.fill_random(&mut output).unwrap();

        assert_eq!(output, [0x11, 0x22, 0x33, 0x44]);
    }

    #[test]
    fn fixed_provider_reports_entropy_failure() {
        let provider = FixedProvider::entropy_failure();
        let mut output = [0xaa; 32];

        let error = provider.fill_random(&mut output).unwrap_err();

        assert_eq!(
            (error, output),
            (ProviderError::EntropyUnavailable, [0; 32])
        );
    }

    #[test]
    fn fixed_provider_clears_oversized_random_output() {
        let entropy = vec![0x11; 64 * 1024 + 1];
        let provider = FixedProvider::with_random(&entropy);
        let mut output = vec![0xaa; 64 * 1024 + 1];

        let error = provider.fill_random(&mut output).unwrap_err();

        assert_eq!(
            (error, output.iter().all(|byte| *byte == 0)),
            (ProviderError::InvalidLength, true)
        );
    }

    #[test]
    fn system_provider_reads_operating_system_entropy() {
        let mut output = [0; 32];

        let result = SystemProvider.fill_random(&mut output);

        assert!(result.is_ok(), "system entropy failed: {result:?}");
    }

    #[test]
    fn sha256_matches_nist_abc_vector() {
        let expected =
            decode_hex("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");

        let digest = SystemProvider.sha256(b"abc");

        assert_eq!(digest, expected);
    }

    #[test]
    fn sha512_matches_nist_abc_vector() {
        let expected = decode_hex(
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2\
             192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
        );

        let digest = SystemProvider.sha512(b"abc");

        assert_eq!(digest, expected);
    }

    #[test]
    fn hmac_sha256_matches_rfc4231_case_one() {
        let key = [0x0b; 20];
        let expected =
            decode_hex("b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7");
        let mut tag = [0; 32];

        SystemProvider
            .hmac_sha256(&key, b"Hi There", &mut tag)
            .unwrap();

        assert_eq!(tag, expected);
    }

    #[test]
    fn hmac_sha256_rejects_oversized_keys_and_messages() {
        let oversized = vec![0; 64 * 1024 + 1];
        let mut key_tag = [0xaa; 32];
        let mut message_tag = [0xaa; 32];

        let key_result = SystemProvider.hmac_sha256(&oversized, b"message", &mut key_tag);
        let message_result = SystemProvider.hmac_sha256(&[0x42], &oversized, &mut message_tag);

        assert_eq!(
            (key_result, message_result, key_tag, message_tag),
            (
                Err(ProviderError::InvalidLength),
                Err(ProviderError::InvalidLength),
                [0; 32],
                [0; 32]
            )
        );
    }

    #[test]
    fn hkdf_sha256_matches_rfc5869_case_one() {
        let input_key = [0x0b; 22];
        let salt = decode_hex::<13>("000102030405060708090a0b0c");
        let info = decode_hex::<10>("f0f1f2f3f4f5f6f7f8f9");
        let expected = decode_hex::<42>(
            "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf\
             34007208d5b887185865",
        );
        let mut output = [0; 42];

        SystemProvider
            .hkdf_sha256(&input_key, &salt, &info, &mut output)
            .unwrap();

        assert_eq!(output, expected);
    }

    #[test]
    fn hkdf_sha256_rejects_output_lengths_outside_rfc5869_bounds() {
        let empty_result = SystemProvider.hkdf_sha256(b"key", b"salt", b"info", &mut []);
        let mut oversized = vec![0xaa; 255 * 32 + 1];
        let oversized_result = SystemProvider.hkdf_sha256(b"key", b"salt", b"info", &mut oversized);

        assert_eq!(
            (
                empty_result,
                oversized_result,
                oversized.iter().all(|byte| *byte == 0)
            ),
            (
                Err(ProviderError::InvalidLength),
                Err(ProviderError::InvalidLength),
                true
            )
        );
    }

    #[test]
    fn hkdf_sha256_rejects_oversized_key_salt_and_info() {
        let oversized = vec![0; 64 * 1024 + 1];
        let mut key_output = [0xaa; 32];
        let mut salt_output = [0xaa; 32];
        let mut info_output = [0xaa; 32];

        let key_result = SystemProvider.hkdf_sha256(&oversized, b"salt", b"info", &mut key_output);
        let salt_result = SystemProvider.hkdf_sha256(b"key", &oversized, b"info", &mut salt_output);
        let info_result = SystemProvider.hkdf_sha256(b"key", b"salt", &oversized, &mut info_output);

        assert_eq!(
            (
                key_result,
                salt_result,
                info_result,
                key_output,
                salt_output,
                info_output
            ),
            (
                Err(ProviderError::InvalidLength),
                Err(ProviderError::InvalidLength),
                Err(ProviderError::InvalidLength),
                [0; 32],
                [0; 32],
                [0; 32]
            )
        );
    }

    #[test]
    fn x25519_public_key_matches_rfc7748_alice_vector() {
        let private_key =
            decode_hex::<32>("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a");
        let expected =
            decode_hex("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a");

        let public_key = SystemProvider.x25519_public(&private_key);

        assert_eq!(public_key, expected);
    }

    #[test]
    fn x25519_public_key_matches_mlswg_treekem_vector() {
        // MLSWG treekem.json, vector 0, cipher suite 1, leaf 0.
        let private_key =
            decode_hex::<32>("7957e5368bbfe5dcfcce0cac8868d3d1943d7733f5c77ef582ee84ab6a281176");
        let expected =
            decode_hex("c929637bda524adad04ac85cf8ab7164d9cd88139aef5c9f7157902707c3fb58");

        assert_eq!(SystemProvider.x25519_public(&private_key), expected);
    }

    #[test]
    fn x25519_shared_secret_matches_rfc7748_vector() {
        let alice_private =
            decode_hex::<32>("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a");
        let bob_public =
            decode_hex("de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f");
        let expected =
            decode_hex("4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742");
        let mut shared_secret = [0; 32];

        SystemProvider
            .x25519_shared(&alice_private, &bob_public, &mut shared_secret)
            .unwrap();

        assert_eq!(shared_secret, expected);
    }

    #[test]
    fn x25519_rejects_non_contributory_public_keys_and_clears_output() {
        let private_key = [0x42; 32];
        let mut shared_secret = [0xaa; 32];

        let result = SystemProvider.x25519_shared(&private_key, &[0; 32], &mut shared_secret);

        assert_eq!(
            (result, shared_secret),
            (Err(ProviderError::InvalidPublicKey), [0; 32])
        );
    }

    #[test]
    fn ed25519_public_key_matches_rfc8032_vector_one() {
        let seed = decode_hex("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60");
        let expected =
            decode_hex("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a");

        let public_key = SystemProvider.ed25519_public(&seed);

        assert_eq!(public_key, expected);
    }

    #[test]
    fn ed25519_signature_matches_rfc8032_vector_one() {
        let seed = decode_hex("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60");
        let expected = decode_hex::<64>(
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155\
             5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
        );

        let signature = SystemProvider.ed25519_sign(&seed, b"").unwrap();

        assert_eq!(signature, expected);
    }

    #[test]
    fn ed25519_sign_and_verify_reject_oversized_messages() {
        let message = vec![0; 64 * 1024 + 1];

        let sign_result = SystemProvider.ed25519_sign(&[0x42; 32], &message);
        let verify_result = SystemProvider.ed25519_verify(&[0; 32], &message, &[0; 64]);

        assert_eq!(
            (sign_result, verify_result),
            (
                Err(ProviderError::InvalidLength),
                Err(ProviderError::InvalidLength)
            )
        );
    }

    #[test]
    fn ed25519_verify_accepts_rfc8032_vector_one() {
        let public_key =
            decode_hex("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a");
        let signature = decode_hex::<64>(
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155\
             5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
        );

        let verified = SystemProvider
            .ed25519_verify(&public_key, b"", &signature)
            .unwrap();

        assert!(verified);
    }

    #[test]
    fn ed25519_verify_returns_false_for_a_changed_signature() {
        let public_key =
            decode_hex("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a");
        let mut signature = decode_hex::<64>(
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155\
             5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
        );
        signature[0] ^= 1;

        let result = SystemProvider.ed25519_verify(&public_key, b"", &signature);

        assert_eq!(result, Ok(false));
    }

    #[test]
    fn chacha20poly1305_seal_matches_rfc8439_vector() {
        let key =
            decode_hex::<32>("808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f");
        let nonce = decode_hex::<12>("070000004041424344454647");
        let associated_data = decode_hex::<12>("50515253c0c1c2c3c4c5c6c7");
        let plaintext = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
        let expected = decode_hex::<130>(
            "d31a8d34648e60db7b86afbc53ef7ec2a4aded51296e08fea9e2b5a736ee62d6\
             3dbea45e8ca9671282fafb69da92728b1a71de0a9e060b2905d6a5b67ecd3b36\
             92ddbd7f2d778b8c9803aee328091b58fab324e4fad675945585808b4831d7bc\
             3ff4def08e4b7a9de576d26586cec64b61161ae10b594f09e26a7e902ecbd060\
             0691",
        );

        let ciphertext = SystemProvider
            .chacha20poly1305_seal(&key, &nonce, &associated_data, plaintext)
            .unwrap();

        assert_eq!(ciphertext, expected);
    }

    #[test]
    fn chacha20poly1305_open_matches_rfc8439_vector() {
        let key =
            decode_hex::<32>("808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f");
        let nonce = decode_hex::<12>("070000004041424344454647");
        let associated_data = decode_hex::<12>("50515253c0c1c2c3c4c5c6c7");
        let ciphertext = decode_hex::<130>(
            "d31a8d34648e60db7b86afbc53ef7ec2a4aded51296e08fea9e2b5a736ee62d6\
             3dbea45e8ca9671282fafb69da92728b1a71de0a9e060b2905d6a5b67ecd3b36\
             92ddbd7f2d778b8c9803aee328091b58fab324e4fad675945585808b4831d7bc\
             3ff4def08e4b7a9de576d26586cec64b61161ae10b594f09e26a7e902ecbd060\
             0691",
        );
        let expected = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";

        let mut plaintext = Zeroizing::new(ciphertext.to_vec());

        SystemProvider
            .chacha20poly1305_open(&key, &nonce, &associated_data, &mut plaintext)
            .unwrap();

        assert_eq!(plaintext.as_slice(), expected);
    }

    #[test]
    fn chacha20poly1305_open_reports_authentication_failure_for_wrong_inputs() {
        let key = [0x11; 32];
        let nonce = [0x22; 12];
        let associated_data = b"associated data";
        let ciphertext = SystemProvider
            .chacha20poly1305_seal(&key, &nonce, associated_data, b"plaintext")
            .unwrap();
        let mut changed_ciphertext = ciphertext.clone();
        changed_ciphertext[0] ^= 1;

        let mut changed_ciphertext = Zeroizing::new(changed_ciphertext);
        let mut wrong_key_ciphertext = Zeroizing::new(ciphertext.clone());
        let mut wrong_associated_data_ciphertext = Zeroizing::new(ciphertext);

        let changed_ciphertext_result = SystemProvider.chacha20poly1305_open(
            &key,
            &nonce,
            associated_data,
            &mut changed_ciphertext,
        );
        let wrong_key = SystemProvider.chacha20poly1305_open(
            &[0x33; 32],
            &nonce,
            associated_data,
            &mut wrong_key_ciphertext,
        );
        let wrong_associated_data = SystemProvider.chacha20poly1305_open(
            &key,
            &nonce,
            b"wrong",
            &mut wrong_associated_data_ciphertext,
        );

        assert_eq!(
            (
                changed_ciphertext_result,
                wrong_key,
                wrong_associated_data,
                changed_ciphertext.is_empty(),
                wrong_key_ciphertext.is_empty(),
                wrong_associated_data_ciphertext.is_empty()
            ),
            (
                Err(ProviderError::AuthenticationFailed),
                Err(ProviderError::AuthenticationFailed),
                Err(ProviderError::AuthenticationFailed),
                true,
                true,
                true
            )
        );
    }

    #[test]
    fn chacha20poly1305_open_rejects_flipped_and_truncated_tags_without_plaintext() {
        let key = [0x11; 32];
        let nonce = [0x22; 12];
        let ciphertext = SystemProvider
            .chacha20poly1305_seal(&key, &nonce, b"aad", b"plaintext")
            .unwrap();
        let mut flipped_tag = Zeroizing::new(ciphertext.clone());
        let last = flipped_tag.len() - 1;
        flipped_tag[last] ^= 1;
        let mut truncated_tag = Zeroizing::new(ciphertext[..15].to_vec());

        let flipped_result =
            SystemProvider.chacha20poly1305_open(&key, &nonce, b"aad", &mut flipped_tag);
        let truncated_result =
            SystemProvider.chacha20poly1305_open(&key, &nonce, b"aad", &mut truncated_tag);

        assert_eq!(
            (
                flipped_result,
                truncated_result,
                flipped_tag.is_empty(),
                truncated_tag.is_empty()
            ),
            (
                Err(ProviderError::AuthenticationFailed),
                Err(ProviderError::AuthenticationFailed),
                true,
                true
            )
        );
    }

    #[test]
    fn chacha20poly1305_seal_rejects_oversized_plaintext_and_associated_data() {
        let oversized = vec![0; 64 * 1024 + 1];

        let plaintext_result =
            SystemProvider.chacha20poly1305_seal(&[0; 32], &[0; 12], b"aad", &oversized);
        let associated_data_result =
            SystemProvider.chacha20poly1305_seal(&[0; 32], &[0; 12], &oversized, b"plaintext");

        assert_eq!(
            (plaintext_result, associated_data_result),
            (
                Err(ProviderError::InvalidLength),
                Err(ProviderError::InvalidLength)
            )
        );
    }

    #[test]
    fn chacha20poly1305_open_rejects_oversized_ciphertext_and_associated_data() {
        let oversized = vec![0; 64 * 1024 + 16 + 1];
        let valid_ciphertext = SystemProvider
            .chacha20poly1305_seal(&[0; 32], &[0; 12], b"aad", b"plaintext")
            .unwrap();

        let mut oversized_ciphertext = Zeroizing::new(oversized.clone());
        let mut valid_ciphertext = Zeroizing::new(valid_ciphertext);
        let ciphertext_result = SystemProvider.chacha20poly1305_open(
            &[0; 32],
            &[0; 12],
            b"aad",
            &mut oversized_ciphertext,
        );
        let associated_data_result = SystemProvider.chacha20poly1305_open(
            &[0; 32],
            &[0; 12],
            &oversized,
            &mut valid_ciphertext,
        );
        let errors = (ciphertext_result.err(), associated_data_result.err());

        assert_eq!(
            errors,
            (
                Some(ProviderError::InvalidLength),
                Some(ProviderError::InvalidLength)
            )
        );
    }
}
