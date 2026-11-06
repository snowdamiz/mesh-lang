//! Internal RFC 9458 request/response encapsulation for suite 32/1/3 only.
//!
//! Public wrappers live in `ohttp_ffi`. Contexts are actor-local, one-shot
//! resources, not serializable keys or storage values. Binary HTTP parsing,
//! trusted configuration selection, HTTP routing and replay policy are callers'
//! responsibilities; successful decryption alone does not authorize a request.

use super::provider::{CryptoProvider, MAX_OHTTP_WIRE_BYTES};
use super::{
    failure, hpke_derive_private_key, hpke_labeled_expand, hpke_labeled_extract,
    hpke_shared_secret, provider_failure, resource_failure, CryptoFailure, HPKE_SUITE_ID,
};
use crate::actor::Process;
use crate::protected_memory::ProtectedBytes;
#[cfg(test)]
use crate::secret::consume_owned_resource;
use crate::secret::{
    insert_owned_resource, prepare_owned_resource, validate_prepared_owned_resource,
    CryptoErrorTag, MeshSecretHandle, ResourceError, ResourceKind,
};
use zeroize::Zeroizing;

const HEADER_BYTES: usize = 7;
const ENC_BYTES: usize = 32;
const TAG_BYTES: usize = 16;
const RESPONSE_NONCE_BYTES: usize = 32;
pub(super) const REQUEST_OVERHEAD: usize = HEADER_BYTES + ENC_BYTES + TAG_BYTES;
pub(super) const RESPONSE_OVERHEAD: usize = RESPONSE_NONCE_BYTES + TAG_BYTES;
const REQUEST_LABEL: &[u8] = b"message/bhttp request";
const RESPONSE_LABEL: &[u8] = b"message/bhttp response";

fn header(key_id: u8) -> [u8; HEADER_BYTES] {
    [key_id, 0, 32, 0, 1, 0, 3]
}

fn request_info(header: &[u8]) -> Vec<u8> {
    let mut info = Vec::with_capacity(REQUEST_LABEL.len() + 1 + HEADER_BYTES);
    info.extend_from_slice(REQUEST_LABEL);
    info.push(0);
    info.extend_from_slice(header);
    info
}

fn check_plaintext(plaintext: &[u8], overhead: usize) -> Result<(), CryptoFailure> {
    let maximum = MAX_OHTTP_WIRE_BYTES - overhead;
    if plaintext.len() > maximum {
        return Err(failure(
            CryptoErrorTag::InvalidLength,
            maximum as i64,
            plaintext.len() as i64,
        ));
    }
    Ok(())
}

fn check_wire(wire: &[u8], minimum: usize) -> Result<(), CryptoFailure> {
    if wire.len() < minimum || wire.len() > MAX_OHTTP_WIRE_BYTES {
        return Err(failure(
            CryptoErrorTag::InvalidLength,
            if wire.len() < minimum {
                minimum
            } else {
                MAX_OHTTP_WIRE_BYTES
            } as i64,
            wire.len() as i64,
        ));
    }
    Ok(())
}

struct RequestKeys {
    key: Zeroizing<[u8; 32]>,
    nonce: Zeroizing<[u8; 12]>,
    response_secret: Zeroizing<[u8; 32]>,
}

fn request_keys(provider: &impl CryptoProvider, shared: &[u8; 32], info: &[u8]) -> RequestKeys {
    // RFC 9180 base-mode key schedule, then Export(response label, Nk).
    // Ordinary HPKE entry points retain their existing implementation and bound.
    let psk_id_hash = hpke_labeled_extract(provider, HPKE_SUITE_ID, &[], b"psk_id_hash", &[]);
    let info_hash = hpke_labeled_extract(provider, HPKE_SUITE_ID, &[], b"info_hash", info);
    let mut context = Zeroizing::new([0; 65]);
    context[1..33].copy_from_slice(&psk_id_hash[..]);
    context[33..].copy_from_slice(&info_hash[..]);
    let secret = hpke_labeled_extract(provider, HPKE_SUITE_ID, shared, b"secret", &[]);
    let key = hpke_labeled_expand(provider, HPKE_SUITE_ID, &secret, b"key", &context[..]);
    let nonce = hpke_labeled_expand(
        provider,
        HPKE_SUITE_ID,
        &secret,
        b"base_nonce",
        &context[..],
    );
    let exporter =
        hpke_labeled_expand::<32>(provider, HPKE_SUITE_ID, &secret, b"exp", &context[..]);
    let response_secret =
        hpke_labeled_expand(provider, HPKE_SUITE_ID, &exporter, b"sec", RESPONSE_LABEL);
    RequestKeys {
        key,
        nonce,
        response_secret,
    }
}

fn save_context(
    process: &mut Process,
    kind: ResourceKind,
    response_secret: &[u8; 32],
    enc: &[u8; 32],
) -> Result<*mut MeshSecretHandle, CryptoFailure> {
    let mut bytes = ProtectedBytes::zeroed(64)
        .map_err(ResourceError::from)
        .map_err(resource_failure)?;
    bytes[..32].copy_from_slice(response_secret);
    bytes[32..].copy_from_slice(enc);
    insert_owned_resource(process, kind, bytes).map_err(resource_failure)
}

pub(crate) fn encapsulate_request(
    provider: &impl CryptoProvider,
    process: &mut Process,
    key_id: u8,
    recipient_public: &[u8; 32],
    plaintext: &[u8],
) -> Result<(Vec<u8>, *mut MeshSecretHandle), CryptoFailure> {
    check_plaintext(plaintext, REQUEST_OVERHEAD)?;
    let mut ikm = Zeroizing::new([0; 32]);
    provider
        .fill_random(&mut ikm[..])
        .map_err(provider_failure)?;
    let ephemeral = hpke_derive_private_key(provider, &ikm);
    let enc = provider.x25519_public(&ephemeral);
    let mut dh = Zeroizing::new([0; 32]);
    provider
        .x25519_shared(&ephemeral, recipient_public, &mut dh)
        .map_err(provider_failure)?;
    let shared = hpke_shared_secret(provider, &dh, &enc, recipient_public);
    let header = header(key_id);
    let keys = request_keys(provider, &shared, &request_info(&header));
    let ciphertext = provider
        .ohttp_chacha20poly1305_seal(&keys.key, &keys.nonce, plaintext)
        .map_err(provider_failure)?;
    let mut wire = Vec::with_capacity(HEADER_BYTES + ENC_BYTES + ciphertext.len());
    wire.extend_from_slice(&header);
    wire.extend_from_slice(&enc);
    wire.extend_from_slice(&ciphertext);
    let context = save_context(
        process,
        ResourceKind::OhttpClientContext,
        &keys.response_secret,
        &enc,
    )?;
    Ok((wire, context))
}

pub(crate) fn decapsulate_request(
    provider: &impl CryptoProvider,
    process: &mut Process,
    recipient_private: *const MeshSecretHandle,
    expected_key_id: u8,
    wire: &[u8],
) -> Result<(Zeroizing<Vec<u8>>, *mut MeshSecretHandle), CryptoFailure> {
    check_wire(wire, REQUEST_OVERHEAD)?;
    if wire[0] != expected_key_id {
        return Err(failure(
            CryptoErrorTag::InvalidKey,
            expected_key_id as i64,
            wire[0] as i64,
        ));
    }
    if wire[1..HEADER_BYTES] != header(expected_key_id)[1..] {
        return Err(failure(CryptoErrorTag::UnsupportedOperation, 0, 0));
    }
    let private =
        prepare_owned_resource(process, recipient_private, ResourceKind::X25519PrivateKey)
            .map_err(resource_failure)?;
    let key: &[u8; 32] = private
        .bytes
        .as_ref()
        .try_into()
        .map_err(|_| failure(CryptoErrorTag::InvalidKey, 32, private.bytes.len() as i64))?;
    let enc: &[u8; 32] = wire[HEADER_BYTES..HEADER_BYTES + ENC_BYTES]
        .try_into()
        .unwrap();
    let public = provider.x25519_public(key);
    let mut dh = Zeroizing::new([0; 32]);
    provider
        .x25519_shared(key, enc, &mut dh)
        .map_err(provider_failure)?;
    let shared = hpke_shared_secret(provider, &dh, enc, &public);
    let keys = request_keys(provider, &shared, &request_info(&wire[..HEADER_BYTES]));
    let mut plaintext = Zeroizing::new(wire[HEADER_BYTES + ENC_BYTES..].to_vec());
    provider
        .ohttp_chacha20poly1305_open(&keys.key, &keys.nonce, &mut plaintext)
        .map_err(provider_failure)?;
    // Do not publish plaintext or a derived context after a provider callback
    // destroyed the borrowed key or its actor exited.
    validate_prepared_owned_resource(process, &private, ResourceKind::X25519PrivateKey)
        .map_err(resource_failure)?;
    let context = save_context(
        process,
        ResourceKind::OhttpGatewayContext,
        &keys.response_secret,
        enc,
    )?;
    Ok((plaintext, context))
}

fn response_keys(
    provider: &impl CryptoProvider,
    context: &[u8],
    response_nonce: &[u8; RESPONSE_NONCE_BYTES],
) -> Result<(Zeroizing<[u8; 32]>, Zeroizing<[u8; 12]>), CryptoFailure> {
    if context.len() != 64 {
        return Err(failure(
            CryptoErrorTag::InvalidKey,
            64,
            context.len() as i64,
        ));
    }
    let mut salt = [0; ENC_BYTES + RESPONSE_NONCE_BYTES];
    salt[..ENC_BYTES].copy_from_slice(&context[32..]);
    salt[ENC_BYTES..].copy_from_slice(response_nonce);
    let mut prk = Zeroizing::new([0; 32]);
    provider
        .hmac_sha256(&salt, &context[..32], &mut prk)
        .map_err(provider_failure)?;
    // RFC 9458 ordinary HKDF-Expand, not HPKE LabeledExpand. Each length fits
    // one HMAC block, with the required first-block suffix 0x01.
    let mut key = Zeroizing::new([0; 32]);
    provider
        .hmac_sha256(&*prk, b"key\x01", &mut key)
        .map_err(provider_failure)?;
    let mut nonce_block = Zeroizing::new([0; 32]);
    provider
        .hmac_sha256(&*prk, b"nonce\x01", &mut nonce_block)
        .map_err(provider_failure)?;
    let mut nonce = Zeroizing::new([0; 12]);
    nonce.copy_from_slice(&nonce_block[..12]);
    Ok((key, nonce))
}

#[cfg(test)]
pub(crate) fn encapsulate_response(
    provider: &impl CryptoProvider,
    process: &Process,
    gateway_context: *const MeshSecretHandle,
    plaintext: &[u8],
) -> Result<Vec<u8>, CryptoFailure> {
    // Owner/role refusal preserves the rightful resource. A valid resource is
    // consumed before every later fallible step, including bad input or entropy.
    let context =
        consume_owned_resource(process, gateway_context, ResourceKind::OhttpGatewayContext)
            .map_err(resource_failure)?;
    encapsulate_response_consumed(provider, context, plaintext)
}

// Taking the protected allocation by value prevents a wrapper from keeping a
// reusable context after consumption, including malformed FFI byte input.
pub(super) fn encapsulate_response_consumed(
    provider: &impl CryptoProvider,
    context: ProtectedBytes,
    plaintext: &[u8],
) -> Result<Vec<u8>, CryptoFailure> {
    check_plaintext(plaintext, RESPONSE_OVERHEAD)?;
    let mut response_nonce = [0; RESPONSE_NONCE_BYTES];
    provider
        .fill_random(&mut response_nonce)
        .map_err(provider_failure)?;
    let (key, nonce) = response_keys(provider, &context, &response_nonce)?;
    let ciphertext = provider
        .ohttp_chacha20poly1305_seal(&key, &nonce, plaintext)
        .map_err(provider_failure)?;
    let mut wire = Vec::with_capacity(RESPONSE_NONCE_BYTES + ciphertext.len());
    wire.extend_from_slice(&response_nonce);
    wire.extend_from_slice(&ciphertext);
    Ok(wire)
}

#[cfg(test)]
pub(crate) fn decapsulate_response(
    provider: &impl CryptoProvider,
    process: &Process,
    client_context: *const MeshSecretHandle,
    wire: &[u8],
) -> Result<Zeroizing<Vec<u8>>, CryptoFailure> {
    let context = consume_owned_resource(process, client_context, ResourceKind::OhttpClientContext)
        .map_err(resource_failure)?;
    decapsulate_response_consumed(provider, context, wire)
}

pub(super) fn decapsulate_response_consumed(
    provider: &impl CryptoProvider,
    context: ProtectedBytes,
    wire: &[u8],
) -> Result<Zeroizing<Vec<u8>>, CryptoFailure> {
    check_wire(wire, RESPONSE_OVERHEAD)?;
    let response_nonce: &[u8; RESPONSE_NONCE_BYTES] =
        wire[..RESPONSE_NONCE_BYTES].try_into().unwrap();
    let (key, nonce) = response_keys(provider, &context, response_nonce)?;
    let mut plaintext = Zeroizing::new(wire[RESPONSE_NONCE_BYTES..].to_vec());
    provider
        .ohttp_chacha20poly1305_open(&key, &nonce, &mut plaintext)
        .map_err(provider_failure)?;
    Ok(plaintext)
}
