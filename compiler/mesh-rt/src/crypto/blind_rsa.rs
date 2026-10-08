//! Blind RSA, profile BR1: RSABSSA-SHA384-PSS-Deterministic (RFC 9474 §5)
//! with 2,048-bit keys and e = 65,537, as RFC 9578 uses for token type
//! `0x0002`.
//!
//! - Every target: the public key is parsed against the exact RFC 9578 SPKI
//!   template, and EMSA-PSS encoding, blinding and unblinding run on
//!   `crypto-bigint` (constant-time Montgomery arithmetic, safegcd
//!   inversion). Verification, and the check inside finalize, is ring's
//!   `RSA_PSS_2048_8192_SHA384`.
//! - Linux and macOS only: key generation, PKCS#8 import and raw signing run
//!   on AWS-LC with RSA blinding on. Every signature is checked with RSAVP1
//!   before it is returned (RFC 9474 §4.3). Other targets never compile
//!   AWS-LC; their server functions fail with `UnsupportedTarget`.
//!
//! The helpers are generic over the modulus size so RFC 9474's 4,096-bit
//! vector runs through them; the public Mesh API accepts only BR1 keys.

use crypto_bigint::modular::{FixedMontyForm, FixedMontyParams};
use crypto_bigint::{CtLt, Odd, Uint, U2048};
use ring::signature::{RsaPublicKeyComponents, RSA_PSS_2048_8192_SHA384};
use zeroize::{Zeroize, Zeroizing};

use super::provider::CryptoProvider;
use super::{failure, provider_failure, CryptoFailure};
use crate::secret::CryptoErrorTag;

/// BR1's modulus, blinded message and signature length.
pub(crate) const MODULUS_BYTES: usize = 256;
pub(crate) const SALT_BYTES: usize = 48;
const HASH_BYTES: usize = 48;
const PUBLIC_EXPONENT: u32 = 65_537;
const PUBLIC_EXPONENT_BYTES: [u8; 3] = [0x01, 0x00, 0x01];
/// Blinding draws `r` again when it is not in `[1, n)` or shares a factor
/// with `n`. For a 2,048-bit modulus a draw fails with probability below
/// 1/2, so 64 failures in a row mean the entropy source is broken.
const MAX_BLINDING_DRAWS: usize = 64;

/// The limbs of a BR1 modulus.
pub(crate) const BR1_LIMBS: usize = U2048::LIMBS;
/// A BR1 public key: a 2,048-bit modulus and e = 65,537.
pub(crate) type Br1PublicKey = PublicKey<BR1_LIMBS>;

/// RFC 9578 §8.2.2: an SPKI with the RSASSA-PSS OID, SHA-384, MGF1-SHA-384
/// and a 48-byte salt, whose `RSAPublicKey` holds a 2,048-bit modulus (a
/// 257-byte INTEGER: a zero byte, then a byte with its top bit set) and the
/// exponent 65,537. Only these bytes around the modulus are accepted.
const SPKI_PREFIX: [u8; 81] = [
    0x30, 0x82, 0x01, 0x52, 0x30, 0x3d, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01,
    0x0a, 0x30, 0x30, 0xa0, 0x0d, 0x30, 0x0b, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04,
    0x02, 0x02, 0xa1, 0x1a, 0x30, 0x18, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01,
    0x08, 0x30, 0x0b, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x02, 0xa2, 0x03,
    0x02, 0x01, 0x30, 0x03, 0x82, 0x01, 0x0f, 0x00, 0x30, 0x82, 0x01, 0x0a, 0x02, 0x82, 0x01, 0x01,
    0x00,
];
const SPKI_SUFFIX: [u8; 5] = [0x02, 0x03, 0x01, 0x00, 0x01];
pub(crate) const SPKI_BYTES: usize = SPKI_PREFIX.len() + MODULUS_BYTES + SPKI_SUFFIX.len();

/// A 2,048-bit key as canonical PKCS#8 DER (`rsaEncryption`, as AWS-LC
/// marshals it) is 1,217 +/- 3 bytes: the lengths of `d` and the CRT values
/// vary by a byte or two. The resource and storage rule leaves room for a
/// few more, and nothing shorter than a key with every private component at
/// least 2^(bits - 64) can be stored.
pub(crate) const MIN_SECRET_KEY_BYTES: usize = 1_190;
pub(crate) const MAX_SECRET_KEY_BYTES: usize = 1_220;

fn invalid_public_key() -> CryptoFailure {
    failure(CryptoErrorTag::InvalidPublicKey, 0, 0)
}

fn invalid_signature(expected: usize, actual: usize) -> CryptoFailure {
    failure(
        CryptoErrorTag::InvalidSignature,
        expected as i64,
        actual as i64,
    )
}

pub(crate) fn unsupported_target() -> CryptoFailure {
    failure(CryptoErrorTag::UnsupportedTarget, 0, 0)
}

/// An RSA public key with e = 65,537 and an odd modulus exactly `LIMBS`
/// limbs wide with its top bit set.
pub(crate) struct PublicKey<const LIMBS: usize> {
    modulus: Odd<Uint<LIMBS>>,
    params: FixedMontyParams<LIMBS>,
    exponent: Uint<LIMBS>,
}

impl<const LIMBS: usize> PublicKey<LIMBS> {
    pub(crate) const BYTES: usize = Uint::<LIMBS>::BYTES;

    /// The key for a big-endian `modulus` of exactly `BYTES` bytes, odd and
    /// with its top bit set: a key of any other size is refused, never padded.
    pub(crate) fn from_modulus(modulus: &[u8]) -> Result<Self, CryptoFailure> {
        if modulus.len() != Self::BYTES || modulus[0] & 0x80 == 0 {
            return Err(invalid_public_key());
        }
        let modulus = Odd::new(Uint::<LIMBS>::from_be_slice(modulus))
            .into_option()
            .ok_or_else(invalid_public_key)?;
        Ok(Self {
            params: FixedMontyParams::new(modulus),
            modulus,
            exponent: Uint::from_u32(PUBLIC_EXPONENT),
        })
    }

    pub(crate) fn modulus_bytes(&self) -> Vec<u8> {
        self.modulus.as_ref().to_be_bytes().as_slice().to_vec()
    }

    /// `bytes` as an integer below the modulus, or `None`. `bytes` must be
    /// exactly `BYTES` long.
    fn element(&self, bytes: &[u8]) -> Option<Uint<LIMBS>> {
        let value = Uint::<LIMBS>::from_be_slice(bytes);
        value
            .ct_lt(self.modulus.as_ref())
            .to_bool()
            .then_some(value)
    }

    /// RSAVP1: `value^e mod n`. The exponent is public; the base may be
    /// secret, and Montgomery multiplication is constant-time in it.
    fn rsavp1(&self, value: &Uint<LIMBS>) -> FixedMontyForm<LIMBS> {
        FixedMontyForm::new(value, &self.params).pow_vartime(&self.exponent)
    }
}

/// The BR1 key an SPKI holds, if it is the RFC 9578 template byte for byte.
pub(crate) fn public_key_from_spki(spki: &[u8]) -> Result<Br1PublicKey, CryptoFailure> {
    if spki.len() != SPKI_BYTES {
        return Err(failure(
            CryptoErrorTag::InvalidPublicKey,
            SPKI_BYTES as i64,
            spki.len() as i64,
        ));
    }
    let (prefix, rest) = spki.split_at(SPKI_PREFIX.len());
    let (modulus, suffix) = rest.split_at(MODULUS_BYTES);
    if prefix != SPKI_PREFIX || suffix != SPKI_SUFFIX {
        return Err(invalid_public_key());
    }
    PublicKey::from_modulus(modulus)
}

/// The RFC 9578 SPKI of a BR1 modulus.
pub(crate) fn spki_for_modulus(modulus: &[u8; MODULUS_BYTES]) -> Vec<u8> {
    let mut spki = Vec::with_capacity(SPKI_BYTES);
    spki.extend_from_slice(&SPKI_PREFIX);
    spki.extend_from_slice(modulus);
    spki.extend_from_slice(&SPKI_SUFFIX);
    spki
}

/// EMSA-PSS-ENCODE (RFC 8017 §9.1.1) with SHA-384, MGF1-SHA-384 and a
/// 48-byte salt into `encoded`, for emBits = 8 * `encoded.len()` - 1: the
/// encoding for a modulus whose top bit is set.
fn emsa_pss_encode(
    provider: &impl CryptoProvider,
    message: &[u8],
    salt: &[u8],
    encoded: &mut [u8],
) {
    let length = encoded.len();
    debug_assert!(salt.len() == SALT_BYTES && length >= HASH_BYTES + SALT_BYTES + 2);
    let mut prefixed = Zeroizing::new([0u8; 8 + HASH_BYTES + SALT_BYTES]);
    prefixed[8..8 + HASH_BYTES].copy_from_slice(&provider.sha384(message));
    prefixed[8 + HASH_BYTES..].copy_from_slice(salt);
    let hash = Zeroizing::new(provider.sha384(&prefixed[..]));

    // DB = PS || 0x01 || salt, then masked with MGF1(H) in place.
    let (database, trailer) = encoded.split_at_mut(length - HASH_BYTES - 1);
    database.fill(0);
    let salt_offset = database.len() - SALT_BYTES;
    database[salt_offset - 1] = 0x01;
    database[salt_offset..].copy_from_slice(salt);
    let mut block = Zeroizing::new([0u8; HASH_BYTES + 4]);
    block[..HASH_BYTES].copy_from_slice(&hash[..]);
    for (counter, chunk) in database.chunks_mut(HASH_BYTES).enumerate() {
        block[HASH_BYTES..].copy_from_slice(&(counter as u32).to_be_bytes());
        let mut mask = Zeroizing::new(provider.sha384(&block[..]));
        for (byte, mask_byte) in chunk.iter_mut().zip(mask.iter()) {
            *byte ^= mask_byte;
        }
        mask.zeroize();
    }
    database[0] &= 0x7f;
    trailer[..HASH_BYTES].copy_from_slice(&hash[..]);
    trailer[HASH_BYTES] = 0xbc;
}

/// A blinded message and the inverse of its blinding factor, each as long as
/// the modulus.
pub(crate) struct Blinded {
    pub(crate) blinded: Vec<u8>,
    pub(crate) inverse: Zeroizing<Box<[u8]>>,
}

/// RFC 9474 §4.2 Blind for the identity-prepared `message`. The salt and
/// the blinding factor `r` come from one `fill_random` call, salt first.
pub(crate) fn blind<const LIMBS: usize>(
    provider: &impl CryptoProvider,
    key: &PublicKey<LIMBS>,
    message: &[u8],
) -> Result<Blinded, CryptoFailure> {
    let bytes = PublicKey::<LIMBS>::BYTES;
    for _ in 0..MAX_BLINDING_DRAWS {
        let mut randomness = Zeroizing::new(vec![0u8; SALT_BYTES + bytes]);
        provider
            .fill_random(&mut randomness)
            .map_err(provider_failure)?;
        let (salt, factor) = randomness.split_at(SALT_BYTES);
        let mut factor = Uint::<LIMBS>::from_be_slice(factor);
        let usable = factor.is_nonzero() & factor.ct_lt(key.modulus.as_ref());
        let inverse = factor.invert_odd_mod(&key.modulus);
        if !(usable & inverse.is_some()).to_bool() {
            factor.zeroize();
            continue;
        }
        let mut inverse = inverse.expect_copied("checked above");

        let mut encoded = Zeroizing::new(vec![0u8; bytes]);
        emsa_pss_encode(provider, message, salt, &mut encoded);
        let mut representative = Uint::<LIMBS>::from_be_slice(&encoded);
        // A representative that shares a factor with n would reveal it.
        if !representative
            .invert_odd_mod(&key.modulus)
            .is_some()
            .to_bool()
        {
            representative.zeroize();
            factor.zeroize();
            inverse.zeroize();
            return Err(invalid_public_key());
        }
        let mut masked = key.rsavp1(&factor);
        let mut product = FixedMontyForm::new(&representative, &key.params) * masked;
        let blinded = product.retrieve().to_be_bytes().as_slice().to_vec();
        let mut encoded_inverse = inverse.to_be_bytes();
        let inverse_bytes = Zeroizing::new(encoded_inverse.as_slice().to_vec().into_boxed_slice());
        encoded_inverse.as_mut_slice().zeroize();
        for value in [&mut representative, &mut factor, &mut inverse] {
            value.zeroize();
        }
        masked.zeroize();
        product.zeroize();
        return Ok(Blinded {
            blinded,
            inverse: inverse_bytes,
        });
    }
    Err(failure(CryptoErrorTag::EntropyUnavailable, 0, 0))
}

/// RSASSA-PSS-VERIFY with SHA-384, MGF1-SHA-384 and a 48-byte salt, through
/// ring. A malformed signature is simply not valid.
pub(crate) fn verify<const LIMBS: usize>(
    key: &PublicKey<LIMBS>,
    message: &[u8],
    signature: &[u8],
) -> bool {
    let modulus = key.modulus_bytes();
    RsaPublicKeyComponents {
        n: modulus.as_slice(),
        e: &PUBLIC_EXPONENT_BYTES[..],
    }
    .verify(&RSA_PSS_2048_8192_SHA384, message, signature)
    .is_ok()
}

/// RFC 9474 §4.4 Finalize: unblind `blind_signature` with `inverse` and
/// return the signature on `message` only if it verifies.
pub(crate) fn finalize<const LIMBS: usize>(
    key: &PublicKey<LIMBS>,
    message: &[u8],
    blind_signature: &[u8],
    inverse: &[u8],
) -> Result<Vec<u8>, CryptoFailure> {
    let bytes = PublicKey::<LIMBS>::BYTES;
    if blind_signature.len() != bytes {
        return Err(invalid_signature(bytes, blind_signature.len()));
    }
    let blind_signature = key
        .element(blind_signature)
        .ok_or_else(|| invalid_signature(bytes, bytes))?;
    // A state is the inverse for a key of this size; one made for another
    // key is refused here or by the verification below.
    if inverse.len() != bytes {
        return Err(invalid_signature(0, 0));
    }
    let mut inverse = key
        .element(inverse)
        .ok_or_else(|| invalid_signature(0, 0))?;
    let mut unblinding = FixedMontyForm::new(&inverse, &key.params);
    let signature = (FixedMontyForm::new(&blind_signature, &key.params) * unblinding)
        .retrieve()
        .to_be_bytes()
        .as_slice()
        .to_vec();
    inverse.zeroize();
    unblinding.zeroize();
    if verify(key, message, &signature) {
        Ok(signature)
    } else {
        Err(invalid_signature(0, 0))
    }
}

/// RFC 9474 §4.3's check after RSASP1: `signature^e mod n` is `blinded`.
/// Both are public values of the key's length.
#[cfg(any(test, target_os = "linux", target_os = "macos"))]
fn signature_opens_to<const LIMBS: usize>(
    key: &PublicKey<LIMBS>,
    signature: &[u8],
    blinded: &[u8],
) -> bool {
    use crypto_bigint::CtEq;
    let Some(signature) = key.element(signature) else {
        return false;
    };
    key.rsavp1(&signature)
        .retrieve()
        .ct_eq(&Uint::<LIMBS>::from_be_slice(blinded))
        .to_bool()
}

/// A BR1 blinded message: exactly 256 bytes and numerically below `n`.
#[cfg(any(test, feature = "fuzzing", target_os = "linux", target_os = "macos"))]
fn blinded_element<const LIMBS: usize>(
    key: &PublicKey<LIMBS>,
    blinded: &[u8],
) -> Result<(), CryptoFailure> {
    let bytes = PublicKey::<LIMBS>::BYTES;
    if blinded.len() != bytes {
        return Err(failure(
            CryptoErrorTag::InvalidLength,
            bytes as i64,
            blinded.len() as i64,
        ));
    }
    // RSASP1's "message representative out of range": the right length but
    // not an element of Z_n.
    key.element(blinded)
        .map(drop)
        .ok_or_else(|| failure(CryptoErrorTag::InvalidLength, bytes as i64, bytes as i64))
}

/// Blind RSA private-key operations on AWS-LC. A key is always handled as
/// canonical PKCS#8 DER; every call parses it and checks the BR1 profile.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) mod signer {
    use std::ptr;

    use aws_lc_sys as lc;
    use zeroize::Zeroizing;

    use super::{
        blinded_element, failure, signature_opens_to, Br1PublicKey, CryptoFailure, PublicKey,
        MAX_SECRET_KEY_BYTES, MIN_SECRET_KEY_BYTES, MODULUS_BYTES, PUBLIC_EXPONENT,
    };
    use crate::secret::CryptoErrorTag;

    const MODULUS_BITS: u32 = (MODULUS_BYTES * 8) as u32;

    fn invalid_key() -> CryptoFailure {
        failure(CryptoErrorTag::InvalidKey, 0, 0)
    }

    fn internal() -> CryptoFailure {
        failure(CryptoErrorTag::InternalFailure, 0, 0)
    }

    /// `failure`, after emptying AWS-LC's thread-local error queue.
    fn cleared(failure: CryptoFailure) -> CryptoFailure {
        unsafe { lc::ERR_clear_error() };
        failure
    }

    /// An owned `RSA`, freed (its bignums cleansed by `OPENSSL_free`) on
    /// drop.
    pub(super) struct Rsa(pub(super) *mut lc::RSA);

    impl Drop for Rsa {
        fn drop(&mut self) {
            unsafe { lc::RSA_free(self.0) }
        }
    }

    struct Pkey(*mut lc::EVP_PKEY);

    impl Drop for Pkey {
        fn drop(&mut self) {
            unsafe { lc::EVP_PKEY_free(self.0) }
        }
    }

    pub(super) struct Bignum(pub(super) *mut lc::BIGNUM);

    impl Drop for Bignum {
        fn drop(&mut self) {
            unsafe { lc::BN_free(self.0) }
        }
    }

    /// The key's modulus as BR1's 256 big-endian bytes.
    fn modulus(rsa: &Rsa) -> Result<[u8; MODULUS_BYTES], CryptoFailure> {
        let mut modulus = [0u8; MODULUS_BYTES];
        let written = unsafe {
            lc::BN_bn2bin_padded(modulus.as_mut_ptr(), MODULUS_BYTES, lc::RSA_get0_n(rsa.0))
        };
        if written == 1 {
            Ok(modulus)
        } else {
            Err(cleared(internal()))
        }
    }

    /// A private key of exactly 2,048 bits with e = 65,537 and blinding on.
    fn check_profile(rsa: &Rsa) -> Result<(), CryptoFailure> {
        unsafe {
            let exponent = lc::RSA_get0_e(rsa.0);
            if lc::RSA_bits(rsa.0) != MODULUS_BITS
                || exponent.is_null()
                || lc::BN_num_bits(exponent) > 32
                || lc::BN_get_word(exponent) != u64::from(PUBLIC_EXPONENT)
                || lc::RSA_get0_d(rsa.0).is_null()
                || lc::RSA_flags(rsa.0) & lc::RSA_FLAG_NO_BLINDING != 0
            {
                return Err(invalid_key());
            }
        }
        Ok(())
    }

    /// The RSA key of a PKCS#8 `rsaEncryption` private key with nothing after
    /// it, in the BR1 profile.
    fn parse(pkcs8: &[u8]) -> Result<Rsa, CryptoFailure> {
        unsafe {
            let mut input = std::mem::zeroed::<lc::CBS>();
            lc::CBS_init(&mut input, pkcs8.as_ptr(), pkcs8.len());
            let pkey = lc::EVP_parse_private_key(&mut input);
            if pkey.is_null() {
                return Err(cleared(invalid_key()));
            }
            let pkey = Pkey(pkey);
            if lc::CBS_len(&input) != 0 || lc::EVP_PKEY_id(pkey.0) != lc::EVP_PKEY_RSA {
                return Err(cleared(invalid_key()));
            }
            let rsa = lc::EVP_PKEY_get1_RSA(pkey.0);
            if rsa.is_null() {
                return Err(cleared(invalid_key()));
            }
            let rsa = Rsa(rsa);
            check_profile(&rsa)?;
            Ok(rsa)
        }
    }

    /// The key as PKCS#8 DER, as AWS-LC marshals it.
    pub(super) fn encode(rsa: &Rsa) -> Result<Zeroizing<Box<[u8]>>, CryptoFailure> {
        unsafe {
            let pkey = Pkey(lc::EVP_PKEY_new());
            if pkey.0.is_null() || lc::EVP_PKEY_set1_RSA(pkey.0, rsa.0) != 1 {
                return Err(cleared(internal()));
            }
            let mut output = std::mem::zeroed::<lc::CBB>();
            let mut data = ptr::null_mut();
            let mut length = 0;
            if lc::CBB_init(&mut output, MAX_SECRET_KEY_BYTES) != 1
                || lc::EVP_marshal_private_key(&mut output, pkey.0) != 1
                || lc::CBB_finish(&mut output, &mut data, &mut length) != 1
            {
                lc::CBB_cleanup(&mut output);
                return Err(cleared(internal()));
            }
            // OPENSSL_free cleanses the buffer before releasing it.
            let encoded = Zeroizing::new(
                std::slice::from_raw_parts(data, length)
                    .to_vec()
                    .into_boxed_slice(),
            );
            lc::OPENSSL_free(data.cast());
            Ok(encoded)
        }
    }

    /// The key as canonical PKCS#8, within the resource and storage length
    /// rule.
    fn canonical(rsa: &Rsa) -> Result<Zeroizing<Box<[u8]>>, CryptoFailure> {
        let encoded = encode(rsa)?;
        if (MIN_SECRET_KEY_BYTES..=MAX_SECRET_KEY_BYTES).contains(&encoded.len()) {
            Ok(encoded)
        } else {
            Err(invalid_key())
        }
    }

    /// A new RSA key of `bits` bits with public exponent `exponent`.
    pub(super) fn generate_rsa(bits: u32, exponent: u64) -> Result<Rsa, CryptoFailure> {
        unsafe {
            let rsa = Rsa(lc::RSA_new());
            let public_exponent = Bignum(lc::BN_new());
            if rsa.0.is_null()
                || public_exponent.0.is_null()
                || lc::BN_set_word(public_exponent.0, exponent) != 1
                || lc::RSA_generate_key_ex(rsa.0, bits as i32, public_exponent.0, ptr::null_mut())
                    != 1
                || lc::RSA_check_key(rsa.0) != 1
            {
                return Err(cleared(internal()));
            }
            Ok(rsa)
        }
    }

    /// A new BR1 key as canonical PKCS#8.
    pub(crate) fn generate() -> Result<Zeroizing<Box<[u8]>>, CryptoFailure> {
        let rsa = generate_rsa(MODULUS_BITS, u64::from(PUBLIC_EXPONENT))?;
        check_profile(&rsa)?;
        canonical(&rsa)
    }

    /// A PKCS#8 private key checked (`RSA_check_key`) and re-encoded
    /// canonically; anything else, or another size or exponent, is refused.
    pub(crate) fn import(pkcs8: &[u8]) -> Result<Zeroizing<Box<[u8]>>, CryptoFailure> {
        let rsa = parse(pkcs8)?;
        if unsafe { lc::RSA_check_key(rsa.0) } != 1 {
            return Err(cleared(invalid_key()));
        }
        canonical(&rsa)
    }

    /// The modulus of a stored key.
    pub(crate) fn public_modulus(pkcs8: &[u8]) -> Result<[u8; MODULUS_BYTES], CryptoFailure> {
        modulus(&parse(pkcs8)?)
    }

    /// RFC 9474 §4.3 BlindSign with a stored BR1 key.
    pub(crate) fn sign(pkcs8: &[u8], blinded: &[u8]) -> Result<Vec<u8>, CryptoFailure> {
        let rsa = parse(pkcs8)?;
        let key = Br1PublicKey::from_modulus(&modulus(&rsa)?).map_err(|_| invalid_key())?;
        sign_raw(&rsa, &key, blinded)
    }

    /// RSASP1 on a blinded message below `n` (`RSA_sign_raw`, no padding,
    /// RSA blinding on), released only once RSAVP1 opens the signature back
    /// to it.
    pub(super) fn sign_raw<const LIMBS: usize>(
        rsa: &Rsa,
        key: &PublicKey<LIMBS>,
        blinded: &[u8],
    ) -> Result<Vec<u8>, CryptoFailure> {
        blinded_element(key, blinded)?;
        let bytes = PublicKey::<LIMBS>::BYTES;
        let mut signature = vec![0u8; bytes];
        let mut length = 0;
        let signed = unsafe {
            lc::RSA_sign_raw(
                rsa.0,
                &mut length,
                signature.as_mut_ptr(),
                signature.len(),
                blinded.as_ptr(),
                blinded.len(),
                lc::RSA_NO_PADDING,
            )
        };
        if signed != 1 || length != bytes {
            return Err(cleared(internal()));
        }
        if !signature_opens_to(key, &signature, blinded) {
            return Err(internal());
        }
        Ok(signature)
    }
}

/// Where no signing provider is compiled (iOS, Android, Windows), and in
/// tests on every host: each server operation is `UnsupportedTarget`.
#[cfg(any(test, not(any(target_os = "linux", target_os = "macos"))))]
pub(crate) mod unsupported {
    use zeroize::Zeroizing;

    use super::{unsupported_target, CryptoFailure, MODULUS_BYTES};

    pub(crate) fn generate() -> Result<Zeroizing<Box<[u8]>>, CryptoFailure> {
        Err(unsupported_target())
    }

    pub(crate) fn import(_pkcs8: &[u8]) -> Result<Zeroizing<Box<[u8]>>, CryptoFailure> {
        Err(unsupported_target())
    }

    pub(crate) fn public_modulus(_pkcs8: &[u8]) -> Result<[u8; MODULUS_BYTES], CryptoFailure> {
        Err(unsupported_target())
    }

    pub(crate) fn sign(_pkcs8: &[u8], _blinded: &[u8]) -> Result<Vec<u8>, CryptoFailure> {
        Err(unsupported_target())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(crate) use self::unsupported as signer;

/// Whether this target signs: server-only functions check it before they
/// touch their arguments (after consuming what they consume).
pub(crate) fn server_target() -> Result<(), CryptoFailure> {
    if cfg!(any(target_os = "linux", target_os = "macos")) {
        Ok(())
    } else {
        Err(unsupported_target())
    }
}

/// The RFC 9578 Appendix A.2 issuer key, for fuzzing the public boundary
/// against a real key.
#[cfg(feature = "fuzzing")]
pub(crate) const RFC9578_SPKI_HEX: &str = "30820152303d06092a864886f70d01010a3030a00d300b0609608648016503040202a11a301806092a864886f70d010108300b0609608648016503040202a2030201300382010f003082010a0282010100cb1aed6b6a95f5b1ce013a4cfcab25b94b2e64a23034e4250a7eab43c0df3a8c12993af12b111908d4b471bec31d4b6c9ad9cdda90612a2ee903523e6de5a224d6b02f09e5c374d0cfe01d8f529c500a78a2f67908fa682b5a2b430c81eaf1af72d7b5e794fc98a3139276879757ce453b526ef9bf6ceb99979b8423b90f4461a22af37aab0cf5733f7597abe44d31c732db68a181c6cbbe607d8c0e52e0655fd9996dc584eca0be87afbcd78a337d17b1dba9e828bbd81e291317144e7ff89f55619709b096cbb9ea474cead264c2073fe49740c01f00e109106066983d21e5f83f086e2e823c879cd43cef700d2a352a9babd612d03cad02db134b7e225a5f0203010001";

/// Exercise the blind RSA boundary: SPKI parsing, blinding, finalizing and
/// verifying arbitrary bytes, and (sparsely) signing rejection.
#[cfg(feature = "fuzzing")]
pub(crate) fn fuzz_blind_rsa_boundaries(input: &[u8]) {
    use super::provider::FixedProvider;

    let _ = public_key_from_spki(input);
    let spki = crate::bytes::decode_hex(RFC9578_SPKI_HEX).expect("the RFC 9578 issuer key");
    let key = public_key_from_spki(&spki).expect("the RFC 9578 issuer key");
    let mut entropy = vec![0u8; SALT_BYTES + MODULUS_BYTES];
    for (index, byte) in entropy.iter_mut().enumerate() {
        *byte = input
            .get(index % input.len().max(1))
            .copied()
            .unwrap_or(0x5a);
    }
    if let Ok(blinded) = blind(&FixedProvider::with_random(&entropy), &key, input) {
        let _ = finalize(&key, input, &blinded.blinded, &blinded.inverse);
    }
    let _ = finalize(&key, input, input, input);
    let _ = verify(&key, input, input);
    let _ = blinded_element(&key, input);
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        static ISSUER: std::sync::OnceLock<Zeroizing<Box<[u8]>>> = std::sync::OnceLock::new();
        let issuer = ISSUER.get_or_init(|| signer::generate().expect("a fuzzing issuer key"));
        let _ = signer::import(input);
        let _ = signer::sign(input, input);
        let blinded = &input[..input.len().min(MODULUS_BYTES + 1)];
        let _ = signer::sign(issuer, blinded);
    }
}

#[cfg(test)]
mod tests;
