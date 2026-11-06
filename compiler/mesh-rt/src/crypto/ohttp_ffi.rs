//! Bounded public OHTTP ABI. The response role is consumed before parsing its
//! payload, while wrong owner/role leaves the rightful resource untouched.
use super::ohttp;
use super::provider::{SystemProvider, MAX_OHTTP_WIRE_BYTES};
use super::{
    allocate_value, bytes_value, crypto_result, failure, required_bytes, resource_failure,
    with_current_process, x25519_public_key_bytes, CryptoFailure, MeshX25519PublicKey,
};
use crate::bytes::MeshBytes;
use crate::io::MeshResult;
use crate::secret::{consume_owned_resource, CryptoErrorTag, MeshSecretHandle, ResourceKind};

#[repr(C)]
pub struct MeshOhttpRequest {
    pub wire: *mut MeshBytes,
    pub response_context: *mut MeshSecretHandle,
}
#[repr(C)]
pub struct MeshOhttpRequestOpening {
    pub plaintext: *mut MeshBytes,
    pub response_context: *mut MeshSecretHandle,
}
fn key_id(value: i64) -> Result<u8, CryptoFailure> {
    u8::try_from(value).map_err(|_| failure(CryptoErrorTag::InvalidLength, 255, value))
}

#[no_mangle]
pub extern "C" fn mesh_ohttp_encapsulate_request(
    id: i64,
    recipient: *const MeshX25519PublicKey,
    plaintext: *const MeshBytes,
) -> *mut MeshResult {
    crypto_result((|| {
        let id = key_id(id)?;
        let recipient = unsafe { x25519_public_key_bytes(recipient)? };
        let plaintext =
            unsafe { required_bytes(plaintext, MAX_OHTTP_WIRE_BYTES - ohttp::REQUEST_OVERHEAD)? };
        let (wire, response_context) = with_current_process(|process| {
            ohttp::encapsulate_request(&SystemProvider, process, id, recipient, plaintext)
        })?;
        Ok(allocate_value(MeshOhttpRequest {
            wire: bytes_value(&wire),
            response_context,
        }))
    })())
}

#[no_mangle]
pub extern "C" fn mesh_ohttp_decapsulate_request(
    key: *const MeshSecretHandle,
    id: i64,
    wire: *const MeshBytes,
) -> *mut MeshResult {
    crypto_result((|| {
        let id = key_id(id)?;
        let wire = unsafe { required_bytes(wire, MAX_OHTTP_WIRE_BYTES)? };
        let (plaintext, response_context) = with_current_process(|process| {
            ohttp::decapsulate_request(&SystemProvider, process, key, id, wire)
        })?;
        Ok(allocate_value(MeshOhttpRequestOpening {
            plaintext: bytes_value(&plaintext),
            response_context,
        }))
    })())
}

#[no_mangle]
pub extern "C" fn mesh_ohttp_encapsulate_response(
    handle: *const MeshSecretHandle,
    plaintext: *const MeshBytes,
) -> *mut MeshResult {
    crypto_result(
        with_current_process(|process| {
            let context =
                consume_owned_resource(process, handle, ResourceKind::OhttpGatewayContext)
                    .map_err(resource_failure)?;
            // This must stay after owned-role consumption, including null/oversize.
            let plaintext = unsafe {
                required_bytes(plaintext, MAX_OHTTP_WIRE_BYTES - ohttp::RESPONSE_OVERHEAD)?
            };
            ohttp::encapsulate_response_consumed(&SystemProvider, context, plaintext)
        })
        .map(|wire| bytes_value(&wire)),
    )
}

#[no_mangle]
pub extern "C" fn mesh_ohttp_decapsulate_response(
    handle: *const MeshSecretHandle,
    wire: *const MeshBytes,
) -> *mut MeshResult {
    crypto_result(
        with_current_process(|process| {
            let context = consume_owned_resource(process, handle, ResourceKind::OhttpClientContext)
                .map_err(resource_failure)?;
            let wire = unsafe { required_bytes(wire, MAX_OHTTP_WIRE_BYTES)? };
            ohttp::decapsulate_response_consumed(&SystemProvider, context, wire)
        })
        .map(|plaintext| bytes_value(&plaintext)),
    )
}
