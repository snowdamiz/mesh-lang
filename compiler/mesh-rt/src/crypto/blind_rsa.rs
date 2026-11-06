//! Profile BR1: RSABSSA-SHA384-PSS-Deterministic, exactly RSA-2048/e65537.
use super::provider::{CryptoProvider, ProviderError};
use crypto_bigint::{
    modular::{MontyForm, MontyParams},
    Odd, U2048, U64,
};
use ring::{
    digest::{digest, SHA256, SHA384},
    signature::{RsaPublicKeyComponents, RSA_PSS_2048_8192_SHA384},
};
use subtle::{Choice, ConditionallySelectable, ConstantTimeEq, ConstantTimeLess};
use zeroize::{Zeroize, Zeroizing};

pub(crate) const SPKI_LEN: usize = 342;
pub(crate) const STATE_LEN: usize = 336;
const MAX_INPUT: usize = 65536;
const SPKI_PREFIX: &[u8] = &[
    0x30, 0x82, 0x1, 0x52, 0x30, 0x3d, 0x6, 0x9, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0xd, 0x1, 0x1, 0xa,
    0x30, 0x30, 0xa0, 0xd, 0x30, 0xb, 0x6, 0x9, 0x60, 0x86, 0x48, 0x1, 0x65, 0x3, 0x4, 0x2, 0x2,
    0xa1, 0x1a, 0x30, 0x18, 0x6, 0x9, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0xd, 0x1, 0x1, 0x8, 0x30, 0xb,
    0x6, 0x9, 0x60, 0x86, 0x48, 0x1, 0x65, 0x3, 0x4, 0x2, 0x2, 0xa2, 0x3, 0x2, 0x1, 0x30, 0x3,
    0x82, 0x1, 0xf, 0x0, 0x30, 0x82, 0x1, 0xa, 0x2, 0x82, 0x1, 0x1, 0x0,
];
const SPKI_SUFFIX: &[u8] = &[2, 3, 1, 0, 1];
type Monty = MontyForm<{ U2048::LIMBS }>;
type Params = MontyParams<{ U2048::LIMBS }>;
pub(crate) struct PublicKey {
    pub n: [u8; 256],
    modulus: U2048,
    params: Params,
}
pub(crate) fn parse_spki(spki: &[u8]) -> Result<PublicKey, ProviderError> {
    if spki.len() != SPKI_LEN || &spki[..81] != SPKI_PREFIX || &spki[337..] != SPKI_SUFFIX {
        return Err(ProviderError::InvalidPublicKey);
    }
    let n: [u8; 256] = spki[81..337].try_into().unwrap();
    if n[0] & 0x80 == 0 || n[255] & 1 == 0 {
        return Err(ProviderError::InvalidPublicKey);
    }
    let modulus = U2048::from_be_slice(&n);
    let params = Params::new(Odd::new(modulus).unwrap());
    Ok(PublicKey { n, modulus, params })
}
#[cfg(any(target_os = "linux", target_os = "macos", test))]
fn spki(n: &[u8; 256]) -> Vec<u8> {
    let mut out = Vec::with_capacity(SPKI_LEN);
    out.extend_from_slice(SPKI_PREFIX);
    out.extend_from_slice(n);
    out.extend_from_slice(SPKI_SUFFIX);
    out
}
fn hash384(data: &[u8]) -> [u8; 48] {
    digest(&SHA384, data).as_ref().try_into().unwrap()
}
#[cfg(any(target_os = "linux", target_os = "macos", test))]
fn public_pow(key: &PublicKey, input: &U2048) -> U2048 {
    Monty::new(input, key.params)
        .pow_bounded_exp(&U64::from_u64(65537), 17)
        .retrieve()
}
#[cfg(any(target_os = "linux", target_os = "macos", test))]
fn check_raw_response(key: &PublicKey, input: &[u8], output: &[u8]) -> Result<(), ProviderError> {
    if input.len() != 256 || output.len() != 256 {
        return Err(ProviderError::InternalFailure);
    }
    let response = Zeroizing::new(U2048::from_be_slice(output));
    let checked = Zeroizing::new(public_pow(key, &response));
    let checked_bytes = Zeroizing::new(checked.to_be_bytes());
    if !bool::from(checked_bytes.ct_eq(input)) {
        return Err(ProviderError::InternalFailure);
    }
    Ok(())
}
/// RFC8017 EMSA-PSS; all lengths and loop bounds are public BR1 parameters.
fn encode(message: &[u8], salt: &[u8], width: usize) -> Zeroizing<Vec<u8>> {
    let hashed = Zeroizing::new(hash384(message));
    let mut buffer = Zeroizing::new(vec![0; 56 + salt.len()]);
    buffer[8..56].copy_from_slice(&*hashed);
    buffer[56..].copy_from_slice(salt);
    let h = Zeroizing::new(hash384(&buffer));
    let db_len = width - 49;
    let mut db = Zeroizing::new(vec![0; db_len]);
    db[db_len - salt.len() - 1] = 1;
    db[db_len - salt.len()..].copy_from_slice(salt);
    let mut mgf_input = Zeroizing::new([0u8; 52]);
    mgf_input[..48].copy_from_slice(&*h);
    for (i, chunk) in db.chunks_mut(48).enumerate() {
        mgf_input[48..].copy_from_slice(&(i as u32).to_be_bytes());
        let mask = Zeroizing::new(hash384(&*mgf_input));
        for (j, b) in chunk.iter_mut().enumerate() {
            *b ^= mask[j];
        }
    }
    db[0] &= 0x7f;
    db.extend_from_slice(&*h);
    db.push(0xbc);
    db
}
pub(crate) fn blind<P: CryptoProvider + ?Sized>(
    provider: &P,
    public: &[u8],
    message: &[u8],
) -> Result<(Vec<u8>, Zeroizing<Vec<u8>>), ProviderError> {
    if message.len() > MAX_INPUT {
        return Err(ProviderError::InvalidLength);
    }
    let key = parse_spki(public)?;
    let mut salt = Zeroizing::new([0u8; 48]);
    provider.fill_random(&mut *salt)?;
    let encoded = encode(message, &*salt, 256);
    // RFC9474 4.2 requires the encoded representative itself to be a unit.
    // The public modulus fixes SafeGcd's bound at 2048 bits on this path too.
    let em = Zeroizing::new(U2048::from_be_slice(&encoded));
    let em_inverse = em.inv_odd_mod(&Odd::new(key.modulus).unwrap());
    let em_is_unit = em_inverse.is_some();
    let _em_inverse = Zeroizing::new(em_inverse.unwrap_or(U2048::ZERO));
    if !bool::from(em_is_unit) {
        return Err(ProviderError::InvalidPublicKey);
    }

    // Fixed128 draws select the first uniform nonzero integer<n without secret
    // branch/loop bounds. With a full2048-bit modulus, all-rejected<=2^-128.
    let mut selected = Zeroizing::new(U2048::ONE);
    let mut found = Choice::from(0);
    let mut random = Zeroizing::new([0u8; 256]);
    for _ in 0..128 {
        provider.fill_random(&mut *random)?;
        let mut candidate = Zeroizing::new(U2048::from_be_slice(&*random));
        let valid = !candidate.ct_eq(&U2048::ZERO) & candidate.ct_lt(&key.modulus);
        *selected = U2048::conditional_select(&selected, &candidate, valid & !found);
        found |= valid;
        candidate.zeroize();
    }
    let inverse = selected.inv_odd_mod(&Odd::new(key.modulus).unwrap());
    let inverse_valid = inverse.is_some();
    let inverse = Zeroizing::new(inverse.unwrap_or(U2048::ZERO));
    // SafeGcd's iteration count is max(bits(n),bits(r)); n is exactly2048 bits
    // and0<r<n, so that bound is public and fixed for this entire profile.
    if !bool::from(found) || !bool::from(inverse_valid) {
        return Err(ProviderError::EntropyUnavailable);
    }
    let power = Zeroizing::new(
        Monty::new(&selected, key.params).pow_bounded_exp(&U64::from_u64(65537), 17),
    );
    let em_monty = Zeroizing::new(Monty::new(&em, key.params));
    let blinded_monty = Zeroizing::new(em_monty.mul(&power));
    let blinded = Zeroizing::new(blinded_monty.retrieve());
    let mut state = Zeroizing::new(vec![0; STATE_LEN]);
    let inverse_bytes = Zeroizing::new(inverse.to_be_bytes());
    state[..256].copy_from_slice(&*inverse_bytes);
    state[256..288].copy_from_slice(digest(&SHA256, public).as_ref());
    let message_hash = Zeroizing::new(hash384(message));
    state[288..].copy_from_slice(&*message_hash);
    Ok((blinded.to_be_bytes().to_vec(), state))
}
pub(crate) fn verify(
    public: &[u8],
    message: &[u8],
    signature: &[u8],
) -> Result<bool, ProviderError> {
    if message.len() > MAX_INPUT {
        return Err(ProviderError::InvalidLength);
    }
    let key = parse_spki(public)?;
    if signature.len() != 256 {
        return Err(ProviderError::InvalidSignature);
    }
    Ok(RsaPublicKeyComponents {
        n: &key.n[..],
        e: &[1, 0, 1][..],
    }
    .verify(&RSA_PSS_2048_8192_SHA384, message, signature)
    .is_ok())
}
pub(crate) fn finalize(
    public: &[u8],
    message: &[u8],
    response: &[u8],
    state: &[u8],
) -> Result<Vec<u8>, ProviderError> {
    if message.len() > MAX_INPUT {
        return Err(ProviderError::InvalidLength);
    }
    let key = parse_spki(public)?;
    if response.len() != 256 {
        return Err(ProviderError::InvalidSignature);
    }
    if state.len() != STATE_LEN {
        return Err(ProviderError::InvalidKey);
    }
    let key_hash = digest(&SHA256, public);
    let message_hash = Zeroizing::new(hash384(message));
    if !bool::from(state[256..288].ct_eq(key_hash.as_ref()) & state[288..].ct_eq(&*message_hash)) {
        return Err(ProviderError::AuthenticationFailed);
    }
    let value = Zeroizing::new(U2048::from_be_slice(response));
    let inverse = Zeroizing::new(U2048::from_be_slice(&state[..256]));
    if !bool::from(value.ct_lt(&key.modulus))
        || !bool::from(inverse.ct_lt(&key.modulus))
        || bool::from(inverse.ct_eq(&U2048::ZERO))
    {
        return Err(ProviderError::AuthenticationFailed);
    }
    let value_monty = Zeroizing::new(Monty::new(&value, key.params));
    let inverse_monty = Zeroizing::new(Monty::new(&inverse, key.params));
    let product = Zeroizing::new(value_monty.mul(&inverse_monty).retrieve());
    let signature = Zeroizing::new(product.to_be_bytes().to_vec());
    if !verify(public, message, &signature)? {
        return Err(ProviderError::AuthenticationFailed);
    }
    Ok(signature.to_vec())
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod server {
    use super::*;
    use aws_lc_sys as aws;
    use std::{mem::MaybeUninit, ptr};
    struct Key(*mut aws::RSA);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe { aws::RSA_free(self.0) }
        }
    }
    struct Pkey(*mut aws::EVP_PKEY);
    impl Drop for Pkey {
        fn drop(&mut self) {
            unsafe { aws::EVP_PKEY_free(self.0) }
        }
    }
    struct Builder(aws::CBB);
    impl Drop for Builder {
        fn drop(&mut self) {
            unsafe {
                let data = aws::CBB_data(&self.0);
                let len = aws::CBB_len(&self.0);
                if !data.is_null() {
                    aws::OPENSSL_cleanse(data as *mut _, len)
                }
                aws::CBB_cleanup(&mut self.0)
            }
        }
    }
    fn check(rsa: Key) -> Result<Key, ProviderError> {
        unsafe {
            if aws::RSA_bits(rsa.0) != 2048
                || aws::BN_get_word(aws::RSA_get0_e(rsa.0)) != 65537
                || aws::RSA_check_key(rsa.0) != 1
                || aws::RSA_blinding_on(rsa.0, ptr::null_mut()) != 1
                || aws::RSA_test_flags(rsa.0, aws::RSA_FLAG_NO_BLINDING as i32) != 0
            {
                return Err(ProviderError::InvalidKey);
            }
        }
        Ok(rsa)
    }
    fn load(secret: &[u8]) -> Result<Key, ProviderError> {
        if secret.is_empty() || secret.len() > MAX_INPUT {
            return Err(ProviderError::InvalidKey);
        }
        let mut cbs = MaybeUninit::<aws::CBS>::uninit();
        unsafe {
            aws::CBS_init(cbs.as_mut_ptr(), secret.as_ptr(), secret.len());
            let mut cbs = cbs.assume_init();
            let key = aws::EVP_parse_private_key(&mut cbs);
            if key.is_null() {
                return Err(ProviderError::InvalidKey);
            }
            let key = Pkey(key);
            if aws::CBS_len(&cbs) != 0 {
                return Err(ProviderError::InvalidKey);
            }
            let rsa = aws::EVP_PKEY_get1_RSA(key.0);
            if rsa.is_null() {
                return Err(ProviderError::InvalidKey);
            }
            check(Key(rsa))
        }
    }
    fn modulus(key: &Key) -> Result<[u8; 256], ProviderError> {
        let mut n = [0; 256];
        if unsafe { aws::BN_bn2bin_padded(n.as_mut_ptr(), 256, aws::RSA_get0_n(key.0)) } != 1 {
            return Err(ProviderError::InternalFailure);
        }
        Ok(n)
    }
    pub(super) fn public(secret: &[u8]) -> Result<Vec<u8>, ProviderError> {
        let key = load(secret)?;
        Ok(spki(&modulus(&key)?))
    }
    pub(super) fn generate() -> Result<Zeroizing<Vec<u8>>, ProviderError> {
        unsafe {
            let rsa = aws::RSA_new();
            if rsa.is_null() {
                return Err(ProviderError::ResourceLimitExceeded);
            }
            let rsa = Key(rsa);
            let exponent = aws::BN_new();
            if exponent.is_null() {
                return Err(ProviderError::ResourceLimitExceeded);
            }
            let good = aws::BN_set_word(exponent, 65537) == 1
                && aws::RSA_generate_key_ex(rsa.0, 2048, exponent, ptr::null_mut()) == 1;
            aws::BN_free(exponent);
            if !good {
                return Err(ProviderError::EntropyUnavailable);
            }
            let rsa = check(rsa)?;
            let pkey = aws::EVP_PKEY_new();
            if pkey.is_null() {
                return Err(ProviderError::ResourceLimitExceeded);
            }
            let pkey = Pkey(pkey);
            if aws::EVP_PKEY_set1_RSA(pkey.0, rsa.0) != 1 {
                return Err(ProviderError::InternalFailure);
            }
            let mut cbb = MaybeUninit::<aws::CBB>::uninit();
            if aws::CBB_init(cbb.as_mut_ptr(), 2048) != 1 {
                return Err(ProviderError::ResourceLimitExceeded);
            }
            let mut cbb = Builder(cbb.assume_init());
            if aws::EVP_marshal_private_key(&mut cbb.0, pkey.0) != 1 {
                return Err(ProviderError::InternalFailure);
            }
            let bytes = std::slice::from_raw_parts(aws::CBB_data(&cbb.0), aws::CBB_len(&cbb.0));
            let result = Zeroizing::new(bytes.to_vec());
            aws::OPENSSL_cleanse(aws::CBB_data(&cbb.0) as *mut _, aws::CBB_len(&cbb.0));
            Ok(result)
        }
    }
    pub(super) fn sign(secret: &[u8], input: &[u8]) -> Result<Vec<u8>, ProviderError> {
        if input.len() != 256 {
            return Err(ProviderError::InvalidLength);
        }
        let key = load(secret)?;
        let public = spki(&modulus(&key)?);
        let parsed = parse_spki(&public)?;
        let m = U2048::from_be_slice(input);
        if !bool::from(m.ct_lt(&parsed.modulus)) {
            return Err(ProviderError::InvalidSignature);
        }
        let mut result = Zeroizing::new(vec![0; 256]);
        // These three public representatives have publicly known RSA images. AWS-LC
        // intentionally rejects them; identities preserve the full RFC9474 domain.
        if m == U2048::ZERO || m == U2048::ONE || m == parsed.modulus.wrapping_sub(&U2048::ONE) {
            result.copy_from_slice(input)
        } else {
            let mut len = 0;
            let ok = unsafe {
                aws::RSA_sign_raw(
                    key.0,
                    &mut len,
                    result.as_mut_ptr(),
                    256,
                    input.as_ptr(),
                    256,
                    aws::RSA_NO_PADDING as i32,
                )
            };
            if ok != 1 || len != 256 {
                return Err(ProviderError::InternalFailure);
            }
        }
        check_raw_response(&parsed, input, &result)?;
        Ok(result.to_vec())
    }
}
pub(crate) fn public_from_secret(secret: &[u8]) -> Result<Vec<u8>, ProviderError> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        server::public(secret)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = secret;
        Err(ProviderError::UnsupportedTarget)
    }
}
pub(crate) fn generate() -> Result<Zeroizing<Vec<u8>>, ProviderError> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        server::generate()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Err(ProviderError::UnsupportedTarget)
    }
}
pub(crate) fn sign(secret: &[u8], input: &[u8]) -> Result<Vec<u8>, ProviderError> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        server::sign(secret, input)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (secret, input);
        Err(ProviderError::UnsupportedTarget)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::provider::SystemProvider;
    fn vectors() -> serde_json::Value {
        serde_json::from_str(include_str!(
            "../../../../tests/vectors/blind-rsa/rfc9578-type2.json"
        ))
        .unwrap()
    }
    fn hex(s: &str) -> Vec<u8> {
        s.as_bytes()
            .chunks_exact(2)
            .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
            .collect()
    }
    #[test]
    fn rfc9578_public_verification_and_raw_issuer_signatures() {
        for v in vectors()["vectors"].as_array().unwrap() {
            let public = hex(v["pkI"].as_str().unwrap());
            let message = hex(v["token_input"].as_str().unwrap());
            let sig = hex(v["authenticator"].as_str().unwrap());
            assert!(verify(&public, &message, &sig).unwrap());
            let mut bad = sig.clone();
            bad[0] ^= 1;
            assert!(!verify(&public, &message, &bad).unwrap());
            let secret = hex(v["pkcs8_der"].as_str().unwrap());
            assert_eq!(public_from_secret(&secret).unwrap(), public);
            let blinded = hex(v["blinded_msg"].as_str().unwrap());
            let expected = hex(v["token_response"].as_str().unwrap());
            assert_eq!(&sign(&secret, &blinded).unwrap()[..], expected);
        }
    }
    #[test]
    fn blinded_roundtrip_binds_message_key_and_rejects_malformed_spki() {
        let v = &vectors()["vectors"][0];
        let public = hex(v["pkI"].as_str().unwrap());
        let secret = hex(v["pkcs8_der"].as_str().unwrap());
        let (request, state) = blind(&SystemProvider, &public, b"binary\0message").unwrap();
        let response = sign(&secret, &request).unwrap();
        let signature = finalize(&public, b"binary\0message", &response, &state).unwrap();
        assert!(verify(&public, b"binary\0message", &signature).unwrap());
        assert!(finalize(&public, b"wrong", &response, &state).is_err());
        for n in [0, 1, SPKI_LEN - 1, SPKI_LEN + 1] {
            assert!(parse_spki(&vec![0; n]).is_err());
        }
        for i in [0, 4, 20, 53, SPKI_LEN - 1] {
            let mut bad = public.clone();
            bad[i] ^= 1;
            assert!(parse_spki(&bad).is_err());
        }
    }
    #[test]
    fn deterministic_client_matches_all_rfc9578_vectors() {
        struct Entropy {
            salt: Vec<u8>,
            factor: Vec<u8>,
        }
        impl CryptoProvider for Entropy {
            fn fill_random(&self, out: &mut [u8]) -> Result<(), ProviderError> {
                let source = if out.len() == 48 {
                    &self.salt
                } else {
                    &self.factor
                };
                out.copy_from_slice(source);
                Ok(())
            }
        }
        for v in vectors()["vectors"].as_array().unwrap() {
            let public = hex(v["pkI"].as_str().unwrap());
            let message = hex(v["token_input"].as_str().unwrap());
            let entropy = Entropy {
                salt: hex(v["salt"].as_str().unwrap()),
                factor: hex(v["blind"].as_str().unwrap()),
            };
            let (request, state) = blind(&entropy, &public, &message).unwrap();
            assert_eq!(request, hex(v["blinded_msg"].as_str().unwrap()));
            let signature = finalize(
                &public,
                &message,
                &hex(v["token_response"].as_str().unwrap()),
                &state,
            )
            .unwrap();
            assert_eq!(signature, hex(v["authenticator"].as_str().unwrap()));
        }
    }
    #[test]
    fn rfc9474_all_pss_encodings_and_unblinding_vectors() {
        use crypto_bigint::U4096;
        type M = MontyForm<{ U4096::LIMBS }>;
        let vectors: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/vectors/blind-rsa/rfc9474-appendix-a.json"
        ))
        .unwrap();
        for v in vectors["vectors"].as_array().unwrap() {
            let salt = hex(v["salt"].as_str().unwrap());
            let encoded = encode(&hex(v["prepared_msg"].as_str().unwrap()), &salt, 512);
            assert_eq!(&*encoded, &hex(v["encoded_msg"].as_str().unwrap()));
            let n = U4096::from_be_slice(&hex(v["n"].as_str().unwrap()));
            let params = MontyParams::new(Odd::new(n).unwrap());
            let response =
                Zeroizing::new(U4096::from_be_slice(&hex(v["blind_sig"].as_str().unwrap())));
            let inverse = Zeroizing::new(U4096::from_be_slice(&hex(v["inv"].as_str().unwrap())));
            let signature = Zeroizing::new(
                M::new(&response, params)
                    .mul(&M::new(&inverse, params))
                    .retrieve(),
            );
            assert_eq!(
                &signature.to_be_bytes()[..],
                hex(v["sig"].as_str().unwrap())
            );
            let recovered = M::new(&signature, params)
                .pow_bounded_exp(&U64::from_u64(65537), 17)
                .retrieve();
            assert_eq!(&recovered.to_be_bytes()[..], &*encoded);
            let mut wrong_size = vec![0; SPKI_LEN + 256];
            wrong_size[..81].copy_from_slice(SPKI_PREFIX);
            assert!(parse_spki(&wrong_size).is_err());
        }
    }
    #[test]
    fn entropy_failure_invalid_range_and_message_bounds_fail_closed() {
        use crate::crypto::provider::FixedProvider;
        let v = &vectors()["vectors"][0];
        let public = hex(v["pkI"].as_str().unwrap());
        let secret = hex(v["pkcs8_der"].as_str().unwrap());
        assert!(matches!(
            blind(&FixedProvider::entropy_failure(), &public, b""),
            Err(ProviderError::EntropyUnavailable)
        ));
        assert!(matches!(
            blind(&FixedProvider::with_random(&[0; 256]), &public, b""),
            Err(ProviderError::EntropyUnavailable)
        ));
        assert!(matches!(
            blind(&SystemProvider, &public, &vec![0; MAX_INPUT + 1]),
            Err(ProviderError::InvalidLength)
        ));
        for size in [0, 255, 257] {
            assert!(matches!(
                sign(&secret, &vec![0; size]),
                Err(ProviderError::InvalidLength)
            ));
            assert!(matches!(
                verify(&public, b"", &vec![0; size]),
                Err(ProviderError::InvalidSignature)
            ));
        }
        let key = parse_spki(&public).unwrap();
        let offset = secret.windows(256).position(|part| part == key.n).unwrap();
        let mut short = secret.clone();
        short[offset] &= 0x7f;
        assert_eq!(public_from_secret(&short), Err(ProviderError::InvalidKey));
        let mut wrong_e = secret.clone();
        let exponent_offset = offset + 256;
        assert_eq!(&wrong_e[exponent_offset..exponent_offset + 5], SPKI_SUFFIX);
        wrong_e[exponent_offset + 4] = 3;
        assert_eq!(public_from_secret(&wrong_e), Err(ProviderError::InvalidKey));
        assert_eq!(sign(&secret, &key.n), Err(ProviderError::InvalidSignature));
        for m in [
            U2048::ZERO,
            U2048::ONE,
            key.modulus.wrapping_sub(&U2048::ONE),
        ] {
            let bytes = m.to_be_bytes();
            assert_eq!(sign(&secret, &bytes).unwrap(), bytes);
        }
        assert_eq!(
            public_from_secret(b"malformed PKCS8"),
            Err(ProviderError::InvalidKey)
        );
        let pkcs1_offset = secret[1..]
            .windows(2)
            .position(|part| part == [0x30, 0x82])
            .unwrap()
            + 1;
        assert_eq!(
            public_from_secret(&secret[pkcs1_offset..]),
            Err(ProviderError::InvalidKey)
        );
        let mut multiprime = secret.clone();
        assert_eq!(&multiprime[pkcs1_offset + 4..pkcs1_offset + 7], &[2, 1, 0]);
        multiprime[pkcs1_offset + 6] = 1;
        assert_eq!(
            public_from_secret(&multiprime),
            Err(ProviderError::InvalidKey)
        );
        assert_eq!(
            public_from_secret(&vec![0; MAX_INPUT + 1]),
            Err(ProviderError::InvalidKey)
        );
        let mut trailing = secret.clone();
        trailing.push(0);
        assert_eq!(
            public_from_secret(&trailing),
            Err(ProviderError::InvalidKey)
        );
        let (request, state) = blind(&SystemProvider, &public, b"").unwrap();
        let response = sign(&secret, &request).unwrap();
        assert!(verify(
            &public,
            b"",
            &finalize(&public, b"", &response, &state).unwrap()
        )
        .unwrap());
        let (request, state) = blind(&SystemProvider, &public, &vec![0; MAX_INPUT]).unwrap();
        let response = sign(&secret, &request).unwrap();
        assert!(finalize(&public, &vec![0; MAX_INPUT], &response, &state).is_ok());
        let mut invalid = state.to_vec();
        invalid[..256].fill(0);
        assert_eq!(
            finalize(&public, &vec![0; MAX_INPUT], &response, &invalid),
            Err(ProviderError::AuthenticationFailed)
        );
        assert_eq!(sign(b"invalid", &[0; 256]), Err(ProviderError::InvalidKey));
    }
    #[test]
    fn generated_key_is_exact_profile_and_validates_before_trivial_signing() {
        let secret = generate().unwrap();
        let public = public_from_secret(&secret).unwrap();
        assert_eq!(public.len(), SPKI_LEN);
        let (request, state) = blind(&SystemProvider, &public, b"generated").unwrap();
        let response = sign(&secret, &request).unwrap();
        assert!(verify(
            &public,
            b"generated",
            &finalize(&public, b"generated", &response, &state).unwrap()
        )
        .unwrap());
        let mut corrupt = secret.to_vec();
        let last = corrupt.len() - 1;
        corrupt[last] ^= 1;
        assert_eq!(sign(&corrupt, &[0; 256]), Err(ProviderError::InvalidKey));
    }
    #[test]
    #[ignore = "host-sensitive supporting evidence; run release with --ignored --exact"]
    fn blind_rsa_sign_timing_distribution() {
        use std::{hint::black_box, time::Instant};
        const THRESHOLD: f64 = 10.0;
        let samples = std::env::var("MESH_TIMING_SAMPLES")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(1000)
            .max(200);
        let v = &vectors()["vectors"][0];
        let secret = Zeroizing::new(hex(v["pkcs8_der"].as_str().unwrap()));
        let public = hex(v["pkI"].as_str().unwrap());
        let key = parse_spki(&public).unwrap();
        let mut low = [0; 256];
        low[255] = 2;
        let mut random = [0; 256];
        SystemProvider.fill_random(&mut random).unwrap();
        random[0] &= 0x7f;
        random[0] |= 1;
        assert!(bool::from(
            U2048::from_be_slice(&random).ct_lt(&key.modulus)
        ));
        // Each group runs the complete imported-key validation, blinded raw
        // transform, and independent public fault check. The control repeats
        // the identical low representative in another rotated group.
        for i in 0..64 {
            black_box(sign(&secret, if i % 2 == 0 { &low } else { &random }).unwrap());
        }
        let mut groups: [Vec<f64>; 3] = std::array::from_fn(|_| Vec::with_capacity(samples));
        for sample in 0..samples {
            let mut order = [0, 1, 2];
            order.rotate_left(sample % 3);
            for group in order {
                let input = if group == 1 { &random } else { &low };
                let start = Instant::now();
                black_box(sign(black_box(&secret), black_box(input)).unwrap());
                groups[group].push(start.elapsed().as_nanos() as f64);
            }
        }
        let stats = |values: &[f64]| {
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            let variance =
                values.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (values.len() - 1) as f64;
            (mean, variance)
        };
        let score = |left: &[f64], right: &[f64]| {
            let (a, av) = stats(left);
            let (b, bv) = stats(right);
            let error = (av / left.len() as f64 + bv / right.len() as f64).sqrt();
            if error == 0.0 {
                if a == b {
                    0.0
                } else {
                    f64::INFINITY
                }
            } else {
                ((a - b) / error).abs()
            }
        };
        let welch_t = score(&groups[0], &groups[1]);
        let control_t = score(&groups[0], &groups[2]);
        let inconclusive = control_t >= THRESHOLD;
        let passed = inconclusive || welch_t < THRESHOLD;
        println!("MESH_TIMING_JSON={{\"schema_version\":2,\"boundary\":\"Crypto.blind_rsa_sign\",\"samples_per_group\":{samples},\"repetitions_per_sample\":1,\"low_mean_ns\":{:.3},\"random_mean_ns\":{:.3},\"control_mean_ns\":{:.3},\"welch_t\":{welch_t:.6},\"control_t\":{control_t:.6},\"threshold\":{THRESHOLD},\"inconclusive\":{inconclusive},\"passed\":{passed}}}",stats(&groups[0]).0,stats(&groups[1]).0,stats(&groups[2]).0);
        assert!(
            passed,
            "raw RSA timing groups separated while identical-input control did not"
        );
    }
    #[test]
    fn raw_response_fault_check_rejects_provider_corruption() {
        let v = &vectors()["vectors"][0];
        let public = hex(v["pkI"].as_str().unwrap());
        let key = parse_spki(&public).unwrap();
        let input = hex(v["blinded_msg"].as_str().unwrap());
        let response = hex(v["token_response"].as_str().unwrap());
        assert!(check_raw_response(&key, &input, &response).is_ok());
        let mut corrupted = response.clone();
        corrupted[100] ^= 1;
        assert_eq!(
            check_raw_response(&key, &input, &corrupted),
            Err(ProviderError::InternalFailure)
        );
        for m in [
            U2048::ZERO,
            U2048::ONE,
            key.modulus.wrapping_sub(&U2048::ONE),
        ] {
            let bytes = m.to_be_bytes();
            assert!(check_raw_response(&key, &bytes, &bytes).is_ok());
        }
    }

    #[test]
    fn rejects_pss_representative_not_coprime_to_public_modulus() {
        struct OneFactor;
        impl CryptoProvider for OneFactor {
            fn fill_random(&self, out: &mut [u8]) -> Result<(), ProviderError> {
                out.fill(0);
                if out.len() == 256 {
                    out[255] = 1
                }
                Ok(())
            }
        }
        let public = spki(&[255; 256]); // 2^2048-1 is an odd full-width multiple of 3.
        let message = (0..256u16)
            .map(|i| i.to_be_bytes())
            .find(|message| {
                encode(message, &[0; 48], 256)
                    .iter()
                    .map(|b| *b as usize)
                    .sum::<usize>()
                    % 3
                    == 0
            })
            .unwrap();
        assert!(matches!(
            blind(&OneFactor, &public, &message),
            Err(ProviderError::InvalidPublicKey)
        ));
    }
}
