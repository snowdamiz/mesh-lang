//! Authenticated external capacity-driver protocol.
//!
//! Controllers use this narrow mTLS/HMAC channel when provider credentials
//! live in a dedicated process. Requests contain typed driver operations and a
//! fixed Docker template; arbitrary commands are never accepted.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use hmac::{Hmac, Mac};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use rustls::server::WebPkiClientVerifier;
use rustls::{
    ClientConfig, ClientConnection, RootCertStore, ServerConfig, ServerConnection, StreamOwned,
};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use super::scaling::{
    CapacityDriver, CapacityObservation, DockerCapacityDriver, DockerDriverConfig, DriverOperation,
    ObservedCapacityNode,
};

const DRIVER_SERVICE_SCHEMA_VERSION: u16 = 1;
const DRIVER_SERVICE_MAX_FRAME: usize = 4 * 1024 * 1024;
const DRIVER_SERVICE_MAX_CONCURRENT_CONNECTIONS: usize = 64;
const DRIVER_SERVICE_MAX_REPLAY_ENTRIES: usize = 8_192;
const DRIVER_SERVICE_IO_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteDockerTemplate {
    pub image: String,
    pub pool: String,
    pub network: Option<String>,
    pub environment: Vec<String>,
    pub operation_timeout_millis: u64,
}

impl std::fmt::Debug for RemoteDockerTemplate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RemoteDockerTemplate")
            .field("image", &self.image)
            .field("pool", &self.pool)
            .field("network", &self.network)
            .field(
                "environment",
                &format_args!("[redacted; {}]", self.environment.len()),
            )
            .field("operation_timeout_millis", &self.operation_timeout_millis)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum DriverServiceAction {
    Validate,
    Observe {
        cluster_id: String,
    },
    Ensure {
        operation: DriverOperation,
    },
    BeginDrain {
        operation: DriverOperation,
        node_id: String,
    },
    Terminate {
        operation: DriverOperation,
        node_id: String,
    },
    GetOperation {
        operation_id: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct DriverServiceRequest {
    schema_version: u16,
    request_id: String,
    issued_at_unix_millis: u64,
    expires_at_unix_millis: u64,
    template: RemoteDockerTemplate,
    action: DriverServiceAction,
    signature: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "payload", rename_all = "snake_case")]
enum DriverServicePayload {
    Valid,
    Observation {
        nodes: Vec<ObservedCapacityNodeWire>,
    },
    Operation {
        operation: DriverOperation,
    },
    OptionalOperation {
        operation: Option<DriverOperation>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct ObservedCapacityNodeWire {
    node_id: String,
    operation_id: String,
    control_term: super::scaling::ControlTerm,
    desired_revision: super::scaling::DesiredRevision,
    template_revision: String,
    lifecycle: super::scaling::CapacityNodeLifecycle,
}

impl From<ObservedCapacityNode> for ObservedCapacityNodeWire {
    fn from(value: ObservedCapacityNode) -> Self {
        Self {
            node_id: value.node_id,
            operation_id: value.operation_id,
            control_term: value.control_term,
            desired_revision: value.desired_revision,
            template_revision: value.template_revision,
            lifecycle: value.lifecycle,
        }
    }
}

impl From<ObservedCapacityNodeWire> for ObservedCapacityNode {
    fn from(value: ObservedCapacityNodeWire) -> Self {
        Self {
            node_id: value.node_id,
            operation_id: value.operation_id,
            control_term: value.control_term,
            desired_revision: value.desired_revision,
            template_revision: value.template_revision,
            lifecycle: value.lifecycle,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct DriverServiceResponse {
    schema_version: u16,
    request_id: String,
    result: Result<DriverServicePayload, String>,
    signature: String,
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn decode_der_env(name: &str) -> Result<Vec<u8>, String> {
    let raw = std::env::var(name).map_err(|_| format!("driver_tls_environment_missing:{name}"))?;
    base64::engine::general_purpose::STANDARD
        .decode(raw.trim())
        .map_err(|_| format!("driver_tls_environment_invalid:{name}"))
}

fn decode_ca_keyring_env(name: &str) -> Result<Vec<CertificateDer<'static>>, String> {
    let raw = std::env::var(name).map_err(|_| format!("driver_tls_environment_missing:{name}"))?;
    let roots = raw
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            base64::engine::general_purpose::STANDARD
                .decode(value)
                .map(CertificateDer::from)
                .map_err(|_| format!("driver_tls_environment_invalid:{name}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if roots.is_empty() {
        return Err(format!("driver_tls_environment_invalid:{name}"));
    }
    Ok(roots)
}

fn shared_keyring_from_env(name: &str) -> Result<Vec<String>, String> {
    let raw = std::env::var(name).map_err(|_| format!("driver_shared_key_missing:{name}"))?;
    parse_shared_keyring(&raw).map_err(|_| format!("driver_shared_key_invalid:{name}"))
}

fn parse_shared_keyring(raw: &str) -> Result<Vec<String>, String> {
    let keys = raw
        .split(',')
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if keys.is_empty() || keys.iter().any(|key| key.len() < 32) {
        return Err("driver_shared_keyring_invalid".to_string());
    }
    Ok(keys)
}

fn tls_client_config() -> Result<Arc<ClientConfig>, String> {
    let cas = decode_ca_keyring_env("MESH_DOCKER_DRIVER_CA_DER_B64")?;
    let certificate =
        CertificateDer::from(decode_der_env("MESH_DOCKER_DRIVER_CLIENT_CERT_DER_B64")?);
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(decode_der_env(
        "MESH_DOCKER_DRIVER_CLIENT_KEY_DER_B64",
    )?));
    let mut roots = RootCertStore::empty();
    for ca in cas {
        roots
            .add(ca)
            .map_err(|_| "driver_tls_ca_invalid".to_string())?;
    }
    ClientConfig::builder()
        .with_root_certificates(roots)
        .with_client_auth_cert(vec![certificate], key)
        .map(Arc::new)
        .map_err(|_| "driver_tls_client_identity_invalid".to_string())
}

fn tls_server_config() -> Result<Arc<ServerConfig>, String> {
    let cas = decode_ca_keyring_env("MESH_DOCKER_DRIVER_CA_DER_B64")?;
    let certificate =
        CertificateDer::from(decode_der_env("MESH_DOCKER_DRIVER_SERVER_CERT_DER_B64")?);
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(decode_der_env(
        "MESH_DOCKER_DRIVER_SERVER_KEY_DER_B64",
    )?));
    let mut roots = RootCertStore::empty();
    for ca in cas {
        roots
            .add(ca)
            .map_err(|_| "driver_tls_ca_invalid".to_string())?;
    }
    let verifier = WebPkiClientVerifier::builder(Arc::new(roots))
        .build()
        .map_err(|_| "driver_tls_client_verifier_invalid".to_string())?;
    ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(vec![certificate], key)
        .map(Arc::new)
        .map_err(|_| "driver_tls_server_identity_invalid".to_string())
}

fn signature_bytes<T: Serialize>(value: &T, key: &str) -> Result<String, String> {
    let encoded = serde_json::to_vec(value)
        .map_err(|_| "driver_service_signature_payload_invalid".to_string())?;
    let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes())
        .map_err(|_| "driver_service_shared_key_invalid".to_string())?;
    mac.update(&encoded);
    Ok(mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn request_signature(request: &DriverServiceRequest, key: &str) -> Result<String, String> {
    signature_bytes(
        &(
            request.schema_version,
            &request.request_id,
            request.issued_at_unix_millis,
            request.expires_at_unix_millis,
            &request.template,
            &request.action,
        ),
        key,
    )
}

fn response_signature(response: &DriverServiceResponse, key: &str) -> Result<String, String> {
    signature_bytes(
        &(
            response.schema_version,
            &response.request_id,
            &response.result,
        ),
        key,
    )
}

fn signature_matches<T>(
    keys: &[String],
    signature: &str,
    expected: impl Fn(&T, &str) -> Result<String, String>,
    value: &T,
) -> bool {
    let Some(signature) = decode_hex_signature(signature) else {
        return false;
    };
    keys.iter().any(|key| {
        let Ok(candidate) = expected(value, key) else {
            return false;
        };
        let Some(candidate) = decode_hex_signature(&candidate) else {
            return false;
        };
        bool::from(signature.ct_eq(&candidate))
    })
}

fn decode_hex_signature(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 {
        return None;
    }
    let mut bytes = [0_u8; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(bytes)
}

fn write_frame(stream: &mut impl Write, payload: &[u8]) -> Result<(), String> {
    if payload.is_empty() || payload.len() > DRIVER_SERVICE_MAX_FRAME {
        return Err("driver_service_frame_size_invalid".to_string());
    }
    let len: u32 = payload
        .len()
        .try_into()
        .map_err(|_| "driver_service_frame_size_invalid".to_string())?;
    stream
        .write_all(&len.to_be_bytes())
        .and_then(|_| stream.write_all(payload))
        .and_then(|_| stream.flush())
        .map_err(|error| format!("driver_service_write_failed:{error}"))
}

fn read_frame(stream: &mut impl Read) -> Result<Vec<u8>, String> {
    let mut len = [0_u8; 4];
    stream
        .read_exact(&mut len)
        .map_err(|error| format!("driver_service_read_failed:{error}"))?;
    let len = u32::from_be_bytes(len) as usize;
    if len == 0 || len > DRIVER_SERVICE_MAX_FRAME {
        return Err("driver_service_frame_size_invalid".to_string());
    }
    let mut payload = vec![0_u8; len];
    stream
        .read_exact(&mut payload)
        .map_err(|error| format!("driver_service_read_failed:{error}"))?;
    Ok(payload)
}

pub struct RemoteDockerCapacityDriver {
    endpoint: String,
    server_name: String,
    template: RemoteDockerTemplate,
    tls: Arc<ClientConfig>,
    shared_keys: Vec<String>,
    timeout: Duration,
}

impl std::fmt::Debug for RemoteDockerCapacityDriver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RemoteDockerCapacityDriver")
            .field("endpoint", &self.endpoint)
            .field("server_name", &self.server_name)
            .field("template", &self.template)
            .field("shared_keys", &"[redacted]")
            .finish_non_exhaustive()
    }
}

impl RemoteDockerCapacityDriver {
    pub fn from_environment(template: RemoteDockerTemplate) -> Result<Self, String> {
        let endpoint = std::env::var("MESH_DOCKER_DRIVER_ENDPOINT")
            .map_err(|_| "docker_driver_endpoint_missing".to_string())?;
        let server_name = std::env::var("MESH_DOCKER_DRIVER_SERVER_NAME")
            .unwrap_or_else(|_| "docker-driver".to_string());
        let shared_keys = shared_keyring_from_env("MESH_DOCKER_DRIVER_SHARED_KEY")
            .map_err(|_| "docker_driver_shared_key_missing_or_invalid".to_string())?;
        if endpoint.trim().is_empty() {
            return Err("docker_driver_remote_configuration_invalid".to_string());
        }
        let timeout = Duration::from_millis(template.operation_timeout_millis);
        Ok(Self {
            endpoint,
            server_name,
            template,
            tls: tls_client_config()?,
            shared_keys,
            timeout,
        })
    }

    fn request(&self, action: DriverServiceAction) -> Result<DriverServicePayload, String> {
        let now = unix_millis();
        let mut request = DriverServiceRequest {
            schema_version: DRIVER_SERVICE_SCHEMA_VERSION,
            request_id: format!("{:032x}", rand::random::<u128>()),
            issued_at_unix_millis: now,
            expires_at_unix_millis: now.saturating_add(self.timeout.as_millis() as u64),
            template: self.template.clone(),
            action,
            signature: String::new(),
        };
        request.signature = request_signature(&request, &self.shared_keys[0])?;
        let tcp = TcpStream::connect(&self.endpoint)
            .map_err(|error| format!("driver_service_connect_failed:{error}"))?;
        tcp.set_read_timeout(Some(self.timeout)).ok();
        tcp.set_write_timeout(Some(self.timeout)).ok();
        let server_name = ServerName::try_from(self.server_name.clone())
            .map_err(|_| "driver_service_server_name_invalid".to_string())?;
        let connection = ClientConnection::new(self.tls.clone(), server_name)
            .map_err(|_| "driver_service_tls_client_failed".to_string())?;
        let mut stream = StreamOwned::new(connection, tcp);
        write_frame(
            &mut stream,
            &serde_json::to_vec(&request)
                .map_err(|_| "driver_service_request_encode_failed".to_string())?,
        )?;
        let response: DriverServiceResponse = serde_json::from_slice(&read_frame(&mut stream)?)
            .map_err(|_| "driver_service_response_decode_failed".to_string())?;
        if response.schema_version != DRIVER_SERVICE_SCHEMA_VERSION
            || response.request_id != request.request_id
            || !signature_matches(
                &self.shared_keys,
                &response.signature,
                response_signature,
                &response,
            )
        {
            return Err("driver_service_response_authentication_failed".to_string());
        }
        response.result
    }
}

impl CapacityDriver for RemoteDockerCapacityDriver {
    fn validate_configuration(&self) -> Result<(), String> {
        match self.request(DriverServiceAction::Validate)? {
            DriverServicePayload::Valid => Ok(()),
            _ => Err("driver_service_reply_kind_invalid".to_string()),
        }
    }

    fn observe_capacity(&self, cluster_id: &str) -> Result<CapacityObservation, String> {
        match self.request(DriverServiceAction::Observe {
            cluster_id: cluster_id.to_string(),
        })? {
            DriverServicePayload::Observation { nodes } => Ok(CapacityObservation {
                nodes: nodes.into_iter().map(Into::into).collect(),
            }),
            _ => Err("driver_service_reply_kind_invalid".to_string()),
        }
    }

    fn ensure_node(&self, operation: &DriverOperation) -> Result<DriverOperation, String> {
        match self.request(DriverServiceAction::Ensure {
            operation: operation.clone(),
        })? {
            DriverServicePayload::Operation { operation } => Ok(operation),
            _ => Err("driver_service_reply_kind_invalid".to_string()),
        }
    }

    fn begin_drain(
        &self,
        operation: &DriverOperation,
        node_id: &str,
    ) -> Result<DriverOperation, String> {
        match self.request(DriverServiceAction::BeginDrain {
            operation: operation.clone(),
            node_id: node_id.to_string(),
        })? {
            DriverServicePayload::Operation { operation } => Ok(operation),
            _ => Err("driver_service_reply_kind_invalid".to_string()),
        }
    }

    fn terminate_node(
        &self,
        operation: &DriverOperation,
        node_id: &str,
    ) -> Result<DriverOperation, String> {
        match self.request(DriverServiceAction::Terminate {
            operation: operation.clone(),
            node_id: node_id.to_string(),
        })? {
            DriverServicePayload::Operation { operation } => Ok(operation),
            _ => Err("driver_service_reply_kind_invalid".to_string()),
        }
    }

    fn get_operation(&self, operation_id: &str) -> Result<Option<DriverOperation>, String> {
        match self.request(DriverServiceAction::GetOperation {
            operation_id: operation_id.to_string(),
        })? {
            DriverServicePayload::OptionalOperation { operation } => Ok(operation),
            _ => Err("driver_service_reply_kind_invalid".to_string()),
        }
    }
}

struct DockerDriverService {
    allowed_cluster: String,
    allowed_pool: String,
    allowed_image: String,
    allowed_network: Option<String>,
    allowed_environment_names: BTreeSet<String>,
    shared_keys: Vec<String>,
    driver: Mutex<Option<(RemoteDockerTemplate, Arc<DockerCapacityDriver>)>>,
    seen_request_ids: Mutex<BTreeMap<String, u64>>,
    active_connections: AtomicUsize,
    inject_ensure_response_loss_once: AtomicBool,
    inject_api_timeout_once: AtomicBool,
    inject_unhealthy_worker_once: AtomicBool,
}

struct ActiveConnectionGuard<'a>(&'a AtomicUsize);

impl Drop for ActiveConnectionGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn claim_request_id(
    seen: &mut BTreeMap<String, u64>,
    request_id: &str,
    expires_at_unix_millis: u64,
    now: u64,
) -> Result<(), String> {
    seen.retain(|_, expires_at| *expires_at >= now);
    if seen.contains_key(request_id) {
        return Err("driver_service_request_replayed".to_string());
    }
    if seen.len() >= DRIVER_SERVICE_MAX_REPLAY_ENTRIES {
        return Err("driver_service_replay_cache_saturated".to_string());
    }
    seen.insert(request_id.to_string(), expires_at_unix_millis);
    Ok(())
}

impl DockerDriverService {
    fn driver(&self, template: &RemoteDockerTemplate) -> Result<Arc<DockerCapacityDriver>, String> {
        let environment_names = template
            .environment
            .iter()
            .map(|entry| {
                entry
                    .split_once('=')
                    .map(|(name, _)| name)
                    .filter(|name| {
                        !name.is_empty()
                            && name
                                .bytes()
                                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                    })
                    .ok_or_else(|| "driver_service_environment_invalid".to_string())
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        if template.pool != self.allowed_pool
            || template.image != self.allowed_image
            || template.network != self.allowed_network
            || environment_names.len() != template.environment.len()
            || environment_names
                != self
                    .allowed_environment_names
                    .iter()
                    .map(String::as_str)
                    .collect()
            || template.operation_timeout_millis == 0
        {
            return Err("driver_service_template_not_allowed".to_string());
        }
        let mut configured = self.driver.lock().unwrap();
        if let Some((existing, driver)) = &*configured {
            if existing != template {
                return Err("driver_service_template_revision_conflict".to_string());
            }
            return Ok(driver.clone());
        }
        let driver = Arc::new(DockerCapacityDriver::new(DockerDriverConfig {
            binary: std::env::var_os("MESH_DOCKER_BINARY")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("docker")),
            execution_prefix: Vec::new(),
            image: template.image.clone(),
            pool: template.pool.clone(),
            network: template.network.clone(),
            environment: template.environment.clone(),
            environment_file_mount: None,
            operation_timeout: Duration::from_millis(template.operation_timeout_millis),
        }));
        driver.validate_configuration()?;
        *configured = Some((template.clone(), driver.clone()));
        Ok(driver)
    }

    fn execute(&self, request: &DriverServiceRequest) -> Result<DriverServicePayload, String> {
        let now = unix_millis();
        if request.schema_version != DRIVER_SERVICE_SCHEMA_VERSION
            || request.request_id.len() != 32
            || !request
                .request_id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || request.issued_at_unix_millis > now.saturating_add(30_000)
            || request.expires_at_unix_millis < now
            || request.expires_at_unix_millis
                > request.issued_at_unix_millis.saturating_add(300_000)
            || !signature_matches(
                &self.shared_keys,
                &request.signature,
                request_signature,
                request,
            )
        {
            return Err("driver_service_request_authentication_failed".to_string());
        }
        claim_request_id(
            &mut self.seen_request_ids.lock().unwrap(),
            &request.request_id,
            request.expires_at_unix_millis,
            now,
        )?;
        let driver = self.driver(&request.template)?;
        if matches!(&request.action, DriverServiceAction::Observe { .. })
            && self.inject_api_timeout_once.swap(false, Ordering::AcqRel)
        {
            eprintln!(
                "mesh capacity driver: transition=fault_injected fault=docker_api_timeout_once"
            );
            return Err("docker_driver_api_timeout".to_string());
        }
        let operation_cluster = |operation: &DriverOperation| {
            (operation.cluster_id == self.allowed_cluster)
                .then_some(())
                .ok_or_else(|| "driver_service_cluster_not_allowed".to_string())
        };
        match &request.action {
            DriverServiceAction::Validate => {
                driver.validate_configuration()?;
                Ok(DriverServicePayload::Valid)
            }
            DriverServiceAction::Observe { cluster_id } => {
                if cluster_id != &self.allowed_cluster {
                    return Err("driver_service_cluster_not_allowed".to_string());
                }
                Ok(DriverServicePayload::Observation {
                    nodes: driver
                        .observe_capacity(cluster_id)?
                        .nodes
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                })
            }
            DriverServiceAction::Ensure { operation } => {
                operation_cluster(operation)?;
                let operation = driver.ensure_node(operation)?;
                if self
                    .inject_unhealthy_worker_once
                    .swap(false, Ordering::AcqRel)
                {
                    let node_id = operation
                        .node_id
                        .as_deref()
                        .ok_or_else(|| "driver_service_unhealthy_fault_node_missing".to_string())?;
                    let status = Command::new(
                        std::env::var_os("MESH_DOCKER_BINARY").unwrap_or_else(|| "docker".into()),
                    )
                    .args(["stop", "--time", "1", node_id])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .map_err(|error| format!("driver_service_unhealthy_fault_failed:{error}"))?;
                    if !status.success() {
                        return Err("driver_service_unhealthy_fault_failed".to_string());
                    }
                    eprintln!(
                        "mesh capacity driver: transition=fault_injected fault=unhealthy_new_worker_once node_id={node_id}"
                    );
                }
                Ok(DriverServicePayload::Operation { operation })
            }
            DriverServiceAction::BeginDrain { operation, node_id } => {
                operation_cluster(operation)?;
                Ok(DriverServicePayload::Operation {
                    operation: driver.begin_drain(operation, node_id)?,
                })
            }
            DriverServiceAction::Terminate { operation, node_id } => {
                operation_cluster(operation)?;
                Ok(DriverServicePayload::Operation {
                    operation: driver.terminate_node(operation, node_id)?,
                })
            }
            DriverServiceAction::GetOperation { operation_id } => {
                let operation = driver.get_operation(operation_id)?;
                if operation
                    .as_ref()
                    .is_some_and(|operation| operation.cluster_id != self.allowed_cluster)
                {
                    return Err("driver_service_cluster_not_allowed".to_string());
                }
                Ok(DriverServicePayload::OptionalOperation { operation })
            }
        }
    }
}

fn handle_service_connection(
    tcp: TcpStream,
    tls: Arc<ServerConfig>,
    service: Arc<DockerDriverService>,
) -> Result<(), String> {
    tcp.set_read_timeout(Some(DRIVER_SERVICE_IO_TIMEOUT)).ok();
    tcp.set_write_timeout(Some(DRIVER_SERVICE_IO_TIMEOUT)).ok();
    let connection =
        ServerConnection::new(tls).map_err(|_| "driver_service_tls_server_failed".to_string())?;
    let mut stream = StreamOwned::new(connection, tcp);
    let payload = read_frame(&mut stream)?;
    let request: DriverServiceRequest = serde_json::from_slice(&payload)
        .map_err(|_| "driver_service_request_decode_failed".to_string())?;
    let result = service.execute(&request);
    if matches!(&request.action, DriverServiceAction::Ensure { .. })
        && result.is_ok()
        && service
            .inject_ensure_response_loss_once
            .swap(false, Ordering::AcqRel)
    {
        // The provider mutation already succeeded. Dropping this authenticated
        // channel before the reply forces the controller to re-observe and
        // adopt by operation id instead of issuing a duplicate create.
        eprintln!(
            "mesh capacity driver: transition=fault_injected fault=ensure_response_loss_once"
        );
        return Err("driver_service_injected_response_loss".to_string());
    }
    let mut response = DriverServiceResponse {
        schema_version: DRIVER_SERVICE_SCHEMA_VERSION,
        request_id: request.request_id.clone(),
        result,
        signature: String::new(),
    };
    response.signature = response_signature(&response, &service.shared_keys[0])?;
    write_frame(
        &mut stream,
        &serde_json::to_vec(&response)
            .map_err(|_| "driver_service_response_encode_failed".to_string())?,
    )
}

/// Runs the dedicated Docker driver service until its listener is closed.
pub fn serve_docker_driver_from_env() -> Result<(), String> {
    let listen =
        std::env::var("MESH_DOCKER_DRIVER_LISTEN").unwrap_or_else(|_| "0.0.0.0:7443".to_string());
    let allowed_cluster = std::env::var("MESH_DOCKER_DRIVER_ALLOWED_CLUSTER")
        .map_err(|_| "driver_service_allowed_cluster_missing".to_string())?;
    let allowed_pool = std::env::var("MESH_DOCKER_DRIVER_ALLOWED_POOL")
        .map_err(|_| "driver_service_allowed_pool_missing".to_string())?;
    let allowed_image = std::env::var("MESH_DOCKER_DRIVER_ALLOWED_IMAGE")
        .map_err(|_| "driver_service_allowed_image_missing".to_string())?;
    let allowed_network = std::env::var("MESH_DOCKER_DRIVER_ALLOWED_NETWORK")
        .map_err(|_| "driver_service_allowed_network_missing".to_string())?;
    let allowed_network =
        (!allowed_network.trim().is_empty()).then(|| allowed_network.trim().to_string());
    let allowed_environment_names_raw = std::env::var("MESH_DOCKER_DRIVER_ALLOWED_ENV_NAMES")
        .map_err(|_| "driver_service_allowed_environment_names_missing".to_string())?;
    let allowed_environment_name_list = allowed_environment_names_raw
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    let allowed_environment_names = allowed_environment_name_list
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let shared_keys = shared_keyring_from_env("MESH_DOCKER_DRIVER_SHARED_KEY")
        .map_err(|_| "driver_service_shared_key_missing_or_invalid".to_string())?;
    if allowed_cluster.trim().is_empty()
        || allowed_pool.trim().is_empty()
        || allowed_image.trim().is_empty()
        || allowed_environment_names.is_empty()
        || allowed_environment_names.len() != allowed_environment_name_list.len()
        || allowed_environment_names.iter().any(|name| {
            !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
    {
        return Err("driver_service_configuration_invalid".to_string());
    }
    let listener = TcpListener::bind(&listen)
        .map_err(|error| format!("driver_service_bind_failed:{error}"))?;
    let tls = tls_server_config()?;
    let service = Arc::new(DockerDriverService {
        allowed_cluster,
        allowed_pool,
        allowed_image,
        allowed_network,
        allowed_environment_names,
        shared_keys,
        driver: Mutex::new(None),
        seen_request_ids: Mutex::new(BTreeMap::new()),
        active_connections: AtomicUsize::new(0),
        inject_ensure_response_loss_once: AtomicBool::new(env_fault_enabled(
            "ensure_response_loss_once",
        )),
        inject_api_timeout_once: AtomicBool::new(env_fault_enabled("docker_api_timeout_once")),
        inject_unhealthy_worker_once: AtomicBool::new(env_fault_enabled(
            "unhealthy_new_worker_once",
        )),
    });
    eprintln!("mesh capacity driver: listening on {listen}");
    for connection in listener.incoming() {
        match connection {
            Ok(tcp) => {
                if service
                    .active_connections
                    .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                        (active < DRIVER_SERVICE_MAX_CONCURRENT_CONNECTIONS).then_some(active + 1)
                    })
                    .is_err()
                {
                    eprintln!(
                        "mesh capacity driver: request_rejected reason=connection_limit_reached"
                    );
                    continue;
                }
                let tls = tls.clone();
                let service = service.clone();
                std::thread::spawn(move || {
                    let _active = ActiveConnectionGuard(&service.active_connections);
                    if let Err(error) = handle_service_connection(tcp, tls, service.clone()) {
                        eprintln!("mesh capacity driver: request_failed reason={error}");
                    }
                });
            }
            Err(error) => eprintln!("mesh capacity driver: accept_failed reason={error}"),
        }
    }
    Ok(())
}

fn env_fault_enabled(expected: &str) -> bool {
    std::env::var("MESH_DOCKER_DRIVER_FAULTS")
        .ok()
        .is_some_and(|raw| fault_list_enabled(&raw, expected))
}

fn fault_list_enabled(raw: &str, expected: &str) -> bool {
    raw.split(',')
        .map(str::trim)
        .any(|configured| configured == expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_service_signatures_cover_action_and_result() {
        let key = "0123456789abcdef0123456789abcdef";
        let mut request = DriverServiceRequest {
            schema_version: DRIVER_SERVICE_SCHEMA_VERSION,
            request_id: "0123456789abcdef0123456789abcdef".to_string(),
            issued_at_unix_millis: 1,
            expires_at_unix_millis: 2,
            template: RemoteDockerTemplate {
                image: "image@sha256:abc".to_string(),
                pool: "workers".to_string(),
                network: Some("mesh".to_string()),
                environment: Vec::new(),
                operation_timeout_millis: 1_000,
            },
            action: DriverServiceAction::Validate,
            signature: String::new(),
        };
        request.signature = request_signature(&request, key).unwrap();
        assert_eq!(request.signature, request_signature(&request, key).unwrap());
        request.action = DriverServiceAction::Observe {
            cluster_id: "other".to_string(),
        };
        assert_ne!(request.signature, request_signature(&request, key).unwrap());
    }

    #[test]
    fn remote_driver_template_debug_redacts_environment_values() {
        let template = RemoteDockerTemplate {
            image: "image@sha256:abc".to_string(),
            pool: "workers".to_string(),
            network: Some("mesh".to_string()),
            environment: vec!["DATABASE_URL=postgres://debug-secret".to_string()],
            operation_timeout_millis: 1_000,
        };

        let rendered = format!("{template:?}");

        assert!(!rendered.contains("postgres://debug-secret"));
        assert!(rendered.contains("[redacted; 1]"));
    }

    #[test]
    fn driver_service_rejects_unapproved_network_and_environment_shape() {
        let service = DockerDriverService {
            allowed_cluster: "cluster-a".to_string(),
            allowed_pool: "workers".to_string(),
            allowed_image: "image@sha256:abc".to_string(),
            allowed_network: Some("mesh-private".to_string()),
            allowed_environment_names: BTreeSet::from(["PORT".to_string()]),
            shared_keys: vec!["0123456789abcdef0123456789abcdef".to_string()],
            driver: Mutex::new(None),
            seen_request_ids: Mutex::new(BTreeMap::new()),
            active_connections: AtomicUsize::new(0),
            inject_ensure_response_loss_once: AtomicBool::new(false),
            inject_api_timeout_once: AtomicBool::new(false),
            inject_unhealthy_worker_once: AtomicBool::new(false),
        };
        let mut template = RemoteDockerTemplate {
            image: "image@sha256:abc".to_string(),
            pool: "workers".to_string(),
            network: Some("host-sensitive".to_string()),
            environment: vec!["PORT=8080".to_string()],
            operation_timeout_millis: 1_000,
        };

        assert_eq!(
            service.driver(&template).unwrap_err(),
            "driver_service_template_not_allowed"
        );
        template.network = Some("mesh-private".to_string());
        template
            .environment
            .push("UNAPPROVED_SECRET=value".to_string());
        assert_eq!(
            service.driver(&template).unwrap_err(),
            "driver_service_template_not_allowed"
        );
    }

    #[test]
    fn driver_service_keyring_allows_rolling_hmac_rotation() {
        let old = "old-driver-key-0123456789abcdef01";
        let new = "new-driver-key-0123456789abcdef01";
        let keys = parse_shared_keyring(&format!("{new},{old}")).expect("keyring");
        let mut request = DriverServiceRequest {
            schema_version: DRIVER_SERVICE_SCHEMA_VERSION,
            request_id: "0123456789abcdef0123456789abcdef".to_string(),
            issued_at_unix_millis: 1,
            expires_at_unix_millis: 2,
            template: RemoteDockerTemplate {
                image: "image@sha256:abc".to_string(),
                pool: "workers".to_string(),
                network: Some("mesh".to_string()),
                environment: Vec::new(),
                operation_timeout_millis: 1_000,
            },
            action: DriverServiceAction::Validate,
            signature: String::new(),
        };
        request.signature = request_signature(&request, old).unwrap();

        assert!(signature_matches(
            &keys,
            &request.signature,
            request_signature,
            &request,
        ));
        assert!(!signature_matches(
            &[new.to_string()],
            &request.signature,
            request_signature,
            &request,
        ));
    }

    #[test]
    fn driver_service_fault_list_is_exact_and_comma_separated() {
        let faults = "ensure_response_loss_once, docker_api_timeout_once";
        assert!(fault_list_enabled(faults, "ensure_response_loss_once"));
        assert!(fault_list_enabled(faults, "docker_api_timeout_once"));
        assert!(!fault_list_enabled(faults, "timeout"));
    }

    #[test]
    fn driver_service_request_ids_are_single_use_and_expiry_bounded() {
        let mut seen = BTreeMap::new();
        claim_request_id(&mut seen, "request-1", 200, 100).unwrap();
        assert_eq!(
            claim_request_id(&mut seen, "request-1", 200, 100),
            Err("driver_service_request_replayed".to_string())
        );
        claim_request_id(&mut seen, "request-2", 300, 201).unwrap();
        assert!(!seen.contains_key("request-1"));
        assert!(seen.contains_key("request-2"));
    }
}
