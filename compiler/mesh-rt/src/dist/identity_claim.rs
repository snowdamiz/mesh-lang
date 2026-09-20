//! Signed, cluster-scoped identities carried by protocol-two handshakes.

use base64::Engine as _;
use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

pub const IDENTITY_ENVELOPE_ENV: &str = "MESH_NODE_IDENTITY_ENVELOPE_B64";
pub const IDENTITY_VERIFY_KEYS_ENV: &str = "MESH_NODE_IDENTITY_VERIFY_KEYS_B64";
pub const CAPACITY_IDENTITY_SIGNING_KEY_ENV: &str = "MESH_CAPACITY_IDENTITY_SIGNING_KEY_DER_B64";
pub const IDENTITY_SCHEMA_VERSION: u16 = 1;
const MAX_IDENTITY_ENVELOPE_BYTES: usize = 8 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeIdentityClaim {
    pub schema_version: u16,
    pub cluster_id: String,
    pub stable_node_id: String,
    /// Exact advertised node name, or `*` for transient operator clients.
    pub advertised_name: String,
    pub roles: Vec<String>,
    pub issued_at_unix_millis: u64,
    pub expires_at_unix_millis: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct SignedNodeIdentityEnvelope {
    claim: NodeIdentityClaim,
    signature_b64: String,
}

pub fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn canonical_claim_bytes(claim: &NodeIdentityClaim) -> Result<Vec<u8>, String> {
    serde_json::to_vec(claim).map_err(|_| "node_identity_claim_encode_failed".to_string())
}

fn validate_claim_shape(claim: &NodeIdentityClaim) -> Result<(), String> {
    let canonical_roles = canonical_roles(&claim.roles)?;
    if claim.schema_version != IDENTITY_SCHEMA_VERSION
        || claim.cluster_id.trim().is_empty()
        || claim.stable_node_id.trim().is_empty()
        || claim.advertised_name.trim().is_empty()
        || canonical_roles != claim.roles
        || claim.issued_at_unix_millis == 0
        || claim.expires_at_unix_millis <= claim.issued_at_unix_millis
        || claim.expires_at_unix_millis
            > claim
                .issued_at_unix_millis
                .saturating_add(31 * 24 * 60 * 60 * 1_000)
    {
        return Err("node_identity_claim_invalid".to_string());
    }
    Ok(())
}

pub fn canonical_roles(roles: &[String]) -> Result<Vec<String>, String> {
    let mut canonical = roles
        .iter()
        .map(|role| role.trim().to_ascii_lowercase())
        .collect::<Vec<_>>();
    if canonical.is_empty()
        || canonical.iter().any(|role| {
            !matches!(
                role.as_str(),
                "controller" | "gateway" | "worker" | "operator"
            )
        })
    {
        return Err("node_identity_roles_invalid".to_string());
    }
    canonical.sort();
    canonical.dedup();
    Ok(canonical)
}

pub fn generate_identity_signing_material() -> Result<(String, String), String> {
    let rng = SystemRandom::new();
    let key = Ed25519KeyPair::generate_pkcs8(&rng)
        .map_err(|_| "node_identity_signing_key_generation_failed".to_string())?;
    let pair = Ed25519KeyPair::from_pkcs8(key.as_ref())
        .map_err(|_| "node_identity_signing_key_invalid".to_string())?;
    Ok((
        base64::engine::general_purpose::STANDARD.encode(key.as_ref()),
        base64::engine::general_purpose::STANDARD.encode(pair.public_key().as_ref()),
    ))
}

pub fn sign_identity_claim(
    claim: &NodeIdentityClaim,
    signing_key_der_b64: &str,
) -> Result<String, String> {
    validate_claim_shape(claim)?;
    let key = base64::engine::general_purpose::STANDARD
        .decode(signing_key_der_b64.trim())
        .map_err(|_| "node_identity_signing_key_invalid".to_string())?;
    let pair = Ed25519KeyPair::from_pkcs8(&key)
        .map_err(|_| "node_identity_signing_key_invalid".to_string())?;
    let signature = pair.sign(&canonical_claim_bytes(claim)?);
    let envelope = SignedNodeIdentityEnvelope {
        claim: claim.clone(),
        signature_b64: base64::engine::general_purpose::STANDARD.encode(signature.as_ref()),
    };
    let encoded = serde_json::to_vec(&envelope)
        .map_err(|_| "node_identity_envelope_encode_failed".to_string())?;
    if encoded.len() > MAX_IDENTITY_ENVELOPE_BYTES {
        return Err("node_identity_envelope_too_large".to_string());
    }
    Ok(base64::engine::general_purpose::STANDARD.encode(encoded))
}

pub fn decode_and_verify_identity(
    envelope: &[u8],
    verify_keys_b64: &str,
    expected_cluster_id: &str,
    advertised_name: &str,
    now: u64,
) -> Result<NodeIdentityClaim, String> {
    if envelope.is_empty() || envelope.len() > MAX_IDENTITY_ENVELOPE_BYTES {
        return Err("node_identity_envelope_invalid".to_string());
    }
    let envelope: SignedNodeIdentityEnvelope = serde_json::from_slice(envelope)
        .map_err(|_| "node_identity_envelope_invalid".to_string())?;
    validate_claim_shape(&envelope.claim)?;
    if envelope.claim.cluster_id != expected_cluster_id
        || !envelope
            .claim
            .stable_node_id
            .starts_with(&format!("{expected_cluster_id}/"))
        || (envelope.claim.advertised_name != advertised_name
            && !(envelope.claim.advertised_name == "*"
                && envelope.claim.roles.len() == 1
                && envelope.claim.roles[0] == "operator")
            && !(advertised_name.starts_with("mesh-operator-query@")
                && envelope.claim.roles.iter().any(|role| role == "controller")))
        || envelope.claim.issued_at_unix_millis > now.saturating_add(30_000)
        || envelope.claim.expires_at_unix_millis < now
    {
        return Err("node_identity_claim_scope_invalid".to_string());
    }
    let signature = base64::engine::general_purpose::STANDARD
        .decode(envelope.signature_b64)
        .map_err(|_| "node_identity_signature_invalid".to_string())?;
    let claim = canonical_claim_bytes(&envelope.claim)?;
    let verified = verify_keys_b64.split(',').map(str::trim).any(|encoded| {
        let Ok(public_key) = base64::engine::general_purpose::STANDARD.decode(encoded) else {
            return false;
        };
        UnparsedPublicKey::new(&ED25519, public_key)
            .verify(&claim, &signature)
            .is_ok()
    });
    if !verified {
        return Err("node_identity_signature_invalid".to_string());
    }
    Ok(envelope.claim)
}

pub fn decode_envelope_b64(encoded: &str) -> Result<Vec<u8>, String> {
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .map_err(|_| "node_identity_envelope_invalid".to_string())?;
    if decoded.is_empty() || decoded.len() > MAX_IDENTITY_ENVELOPE_BYTES {
        return Err("node_identity_envelope_invalid".to_string());
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_identity_binds_cluster_stable_id_name_role_and_expiry() {
        let (private, public) = generate_identity_signing_material().unwrap();
        let claim = NodeIdentityClaim {
            schema_version: IDENTITY_SCHEMA_VERSION,
            cluster_id: "cluster-a".to_string(),
            stable_node_id: "cluster-a/worker/one".to_string(),
            advertised_name: "one@one:4370".to_string(),
            roles: vec!["worker".to_string()],
            issued_at_unix_millis: 100,
            expires_at_unix_millis: 200,
        };
        let envelope = sign_identity_claim(&claim, &private).unwrap();
        let envelope = decode_envelope_b64(&envelope).unwrap();
        assert_eq!(
            decode_and_verify_identity(&envelope, &public, "cluster-a", "one@one:4370", 150)
                .unwrap(),
            claim
        );
        assert!(
            decode_and_verify_identity(&envelope, &public, "cluster-b", "one@one:4370", 150)
                .is_err()
        );
        assert!(decode_and_verify_identity(
            &envelope,
            &public,
            "cluster-a",
            "controller@controller:4370",
            150
        )
        .is_err());
        assert!(decode_and_verify_identity(
            &envelope,
            &public,
            "cluster-a",
            "mesh-http-route@127.0.0.1:1",
            150
        )
        .is_err());
        assert!(
            decode_and_verify_identity(&envelope, &public, "cluster-a", "one@one:4370", 201)
                .is_err()
        );
    }

    #[test]
    fn signed_identity_verify_keyring_allows_rolling_issuer_rotation() {
        let (old_private, old_public) = generate_identity_signing_material().unwrap();
        let (_new_private, new_public) = generate_identity_signing_material().unwrap();
        let claim = NodeIdentityClaim {
            schema_version: IDENTITY_SCHEMA_VERSION,
            cluster_id: "cluster-a".to_string(),
            stable_node_id: "cluster-a/worker/one".to_string(),
            advertised_name: "one@one:4370".to_string(),
            roles: vec!["worker".to_string()],
            issued_at_unix_millis: 100,
            expires_at_unix_millis: 200,
        };
        let envelope = decode_envelope_b64(&sign_identity_claim(&claim, &old_private).unwrap())
            .expect("envelope");

        assert!(decode_and_verify_identity(
            &envelope,
            &format!("{new_public},{old_public}"),
            "cluster-a",
            "one@one:4370",
            150,
        )
        .is_ok());
        assert!(decode_and_verify_identity(
            &envelope,
            &new_public,
            "cluster-a",
            "one@one:4370",
            150,
        )
        .is_err());
    }
}
