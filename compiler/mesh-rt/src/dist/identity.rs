//! Collision-resistant request identities and scoped idempotency keys.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use ring::rand::{SecureRandom, SystemRandom};
use sha2::{Digest, Sha256};

const MAX_IDEMPOTENCY_KEY_BYTES: usize = 255;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RequestId {
    stable_node_hash: [u8; 8],
    boot_id: [u8; 16],
    counter: u64,
}

impl RequestId {
    pub const fn counter(self) -> u64 {
        self.counter
    }

    pub fn as_bytes(self) -> [u8; 32] {
        let mut bytes = [0u8; 32];
        bytes[..8].copy_from_slice(&self.stable_node_hash);
        bytes[8..24].copy_from_slice(&self.boot_id);
        bytes[24..].copy_from_slice(&self.counter.to_be_bytes());
        bytes
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&hex(&self.as_bytes()))
    }
}

#[derive(Debug)]
pub struct RequestIdGenerator {
    stable_node_hash: [u8; 8],
    boot_id: [u8; 16],
    next_counter: AtomicU64,
}

impl RequestIdGenerator {
    pub fn new(stable_node_id: &str) -> Result<Self, String> {
        let stable_node_id = stable_node_id.trim();
        if stable_node_id.is_empty() {
            return Err("stable_node_id_missing".to_string());
        }
        let digest = Sha256::digest(stable_node_id.as_bytes());
        let mut stable_node_hash = [0u8; 8];
        stable_node_hash.copy_from_slice(&digest[..8]);
        let mut boot_id = [0u8; 16];
        SystemRandom::new()
            .fill(&mut boot_id)
            .map_err(|_| "request_boot_id_generation_failed".to_string())?;
        Ok(Self::with_parts(stable_node_hash, boot_id, 1))
    }

    pub const fn with_parts(
        stable_node_hash: [u8; 8],
        boot_id: [u8; 16],
        next_counter: u64,
    ) -> Self {
        Self {
            stable_node_hash,
            boot_id,
            next_counter: AtomicU64::new(next_counter),
        }
    }

    pub fn next(&self) -> Result<RequestId, String> {
        let counter = self
            .next_counter
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .map_err(|_| "request_id_counter_exhausted".to_string())?;
        Ok(RequestId {
            stable_node_hash: self.stable_node_hash,
            boot_id: self.boot_id,
            counter,
        })
    }
}

static REQUEST_ID_GENERATOR: OnceLock<RequestIdGenerator> = OnceLock::new();

pub fn request_id_generator() -> &'static RequestIdGenerator {
    REQUEST_ID_GENERATOR.get_or_init(|| {
        let stable_node_id = std::env::var("MESH_STABLE_NODE_ID")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| super::node::node_state().map(|state| state.name.clone()))
            .unwrap_or_else(|| "mesh-local-node".to_string());
        RequestIdGenerator::new(&stable_node_id)
            .unwrap_or_else(|_| RequestIdGenerator::with_parts([0; 8], rand::random(), 1))
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct OperationKey(String);

impl OperationKey {
    pub fn derive(
        application_id: &str,
        route_id: &str,
        tenant_scope: Option<&str>,
        caller_key: &str,
    ) -> Result<Self, String> {
        let application_id = required_scope(application_id, "application_id")?;
        let route_id = required_scope(route_id, "route_id")?;
        let caller_key = validate_idempotency_key(caller_key)?;
        let tenant_scope = tenant_scope.unwrap_or("").trim();

        let mut hasher = Sha256::new();
        hash_component(&mut hasher, application_id.as_bytes());
        hash_component(&mut hasher, route_id.as_bytes());
        hash_component(&mut hasher, tenant_scope.as_bytes());
        hash_component(&mut hasher, caller_key.as_bytes());
        Ok(Self(hex(&hasher.finalize())))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OperationKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AttemptId {
    pub request_id: RequestId,
    pub ordinal: u32,
}

impl fmt::Display for AttemptId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}-{:08x}", self.request_id, self.ordinal)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OwnershipGeneration(pub u64);

impl OwnershipGeneration {
    pub fn next(self) -> Result<Self, String> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or_else(|| "ownership_generation_exhausted".to_string())
    }
}

pub struct CanonicalHttpRequest<'a> {
    pub method: &'a str,
    pub route_id: &'a str,
    pub path_parameters: &'a [(String, String)],
    pub query_parameters: &'a [(String, String)],
    pub semantic_headers: &'a [(String, String)],
    pub body: &'a [u8],
    pub tenant_scope: Option<&'a str>,
}

impl CanonicalHttpRequest<'_> {
    pub fn hash(&self) -> Result<String, String> {
        let method = required_scope(self.method.trim(), "http_method")?.to_ascii_uppercase();
        let route_id = required_scope(self.route_id.trim(), "route_id")?;
        let mut path_parameters = self.path_parameters.to_vec();
        let mut query_parameters = self.query_parameters.to_vec();
        let mut semantic_headers = self.semantic_headers.to_vec();
        path_parameters.sort();
        query_parameters.sort();
        semantic_headers.iter_mut().for_each(|(name, _)| {
            *name = name.trim().to_ascii_lowercase();
        });
        semantic_headers.sort();

        let mut hasher = Sha256::new();
        hash_component(&mut hasher, method.as_bytes());
        hash_component(&mut hasher, route_id.as_bytes());
        hash_pairs(&mut hasher, &path_parameters);
        hash_pairs(&mut hasher, &query_parameters);
        hash_pairs(&mut hasher, &semantic_headers);
        hash_component(&mut hasher, self.body);
        hash_component(
            &mut hasher,
            self.tenant_scope.unwrap_or("").trim().as_bytes(),
        );
        Ok(hex(&hasher.finalize()))
    }
}

pub fn validate_idempotency_key(raw: &str) -> Result<&str, String> {
    if raw.is_empty() {
        return Err("idempotency_key_missing".to_string());
    }
    if raw.len() > MAX_IDEMPOTENCY_KEY_BYTES {
        return Err(format!(
            "idempotency_key_too_long:{}>{MAX_IDEMPOTENCY_KEY_BYTES}",
            raw.len()
        ));
    }
    if raw.trim() != raw || !raw.bytes().all(|byte| (0x21..=0x7e).contains(&byte)) {
        return Err("idempotency_key_invalid_characters".to_string());
    }
    Ok(raw)
}

fn required_scope<'a>(value: &'a str, label: &str) -> Result<&'a str, String> {
    if value.is_empty() {
        Err(format!("{label}_missing"))
    } else {
        Ok(value)
    }
}

fn hash_component(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

fn hash_pairs(hasher: &mut Sha256, pairs: &[(String, String)]) {
    hasher.update((pairs.len() as u64).to_be_bytes());
    for (key, value) in pairs {
        hash_component(hasher, key.as_bytes());
        hash_component(hasher, value.as_bytes());
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_ids_differ_across_nodes_for_same_counter() {
        let first = RequestIdGenerator::with_parts([1; 8], [2; 16], 1)
            .next()
            .expect("first id");
        let second = RequestIdGenerator::with_parts([3; 8], [2; 16], 1)
            .next()
            .expect("second id");

        assert_ne!(first, second);
    }

    #[test]
    fn request_ids_differ_across_boots_for_same_node_and_counter() {
        let first = RequestIdGenerator::with_parts([1; 8], [2; 16], 1)
            .next()
            .expect("first id");
        let second = RequestIdGenerator::with_parts([1; 8], [3; 16], 1)
            .next()
            .expect("second id");

        assert_ne!(first, second);
    }

    #[test]
    fn request_id_counter_exhaustion_fails_closed() {
        let generator = RequestIdGenerator::with_parts([1; 8], [2; 16], u64::MAX);

        assert_eq!(
            generator.next(),
            Err("request_id_counter_exhausted".to_string())
        );
    }

    #[test]
    fn operation_key_is_stable_for_same_scoped_key() {
        let first = OperationKey::derive("app", "Todos.create", Some("tenant-a"), "key-1")
            .expect("valid operation key");
        let second = OperationKey::derive("app", "Todos.create", Some("tenant-a"), "key-1")
            .expect("valid operation key");

        assert_eq!(first, second);
    }

    #[test]
    fn operation_key_changes_with_tenant_scope() {
        let first = OperationKey::derive("app", "Todos.create", Some("tenant-a"), "key-1")
            .expect("valid operation key");
        let second = OperationKey::derive("app", "Todos.create", Some("tenant-b"), "key-1")
            .expect("valid operation key");

        assert_ne!(first, second);
    }

    #[test]
    fn canonical_hash_is_independent_of_query_order() {
        let first_query = vec![
            ("page".to_string(), "1".to_string()),
            ("sort".to_string(), "name".to_string()),
        ];
        let second_query = vec![
            ("sort".to_string(), "name".to_string()),
            ("page".to_string(), "1".to_string()),
        ];
        let first = CanonicalHttpRequest {
            method: "GET",
            route_id: "Todos.list",
            path_parameters: &[],
            query_parameters: &first_query,
            semantic_headers: &[],
            body: &[],
            tenant_scope: None,
        };
        let second = CanonicalHttpRequest {
            method: "GET",
            route_id: "Todos.list",
            path_parameters: &[],
            query_parameters: &second_query,
            semantic_headers: &[],
            body: &[],
            tenant_scope: None,
        };

        assert_eq!(
            first.hash().expect("first hash"),
            second.hash().expect("second hash")
        );
    }
}
