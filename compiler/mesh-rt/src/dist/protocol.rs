//! Versioned peer protocol contracts, capability negotiation, chunking, and retry guards.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

pub const PROTOCOL_V1: u16 = 1;
pub const PROTOCOL_V2: u16 = 2;
pub const DEFAULT_MAX_FRAME_BYTES: u32 = 1_048_576;
pub const MAX_NEGOTIATED_FRAME_BYTES: u32 = 16 * 1_048_576;
const HELLO_MAGIC: &[u8; 4] = b"MSH2";
const ENVELOPE_MAGIC: &[u8; 4] = b"MEV2";
const MAX_IDENTITY_ENVELOPE_BYTES: usize = 8 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities(u64);

impl Capabilities {
    pub const MULTIPLEXED_REQUESTS: Self = Self(1 << 0);
    pub const BOUNDED_LOAD_REPORTS: Self = Self(1 << 1);
    pub const CHUNKED_SNAPSHOTS: Self = Self(1 << 2);
    pub const DURABLE_CONTINUITY: Self = Self(1 << 3);
    pub const ADAPTIVE_ROUTING: Self = Self(1 << 4);
    pub const DRAIN_FENCING: Self = Self(1 << 5);
    pub const CONTROL_QUORUM: Self = Self(1 << 6);
    pub const AUTONOMOUS_REQUIRED: Self = Self(
        Self::MULTIPLEXED_REQUESTS.0
            | Self::BOUNDED_LOAD_REPORTS.0
            | Self::CHUNKED_SNAPSHOTS.0
            | Self::DURABLE_CONTINUITY.0
            | Self::ADAPTIVE_ROUTING.0
            | Self::DRAIN_FENCING.0
            | Self::CONTROL_QUORUM.0,
    );

    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u64 {
        self.0
    }

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }
}

impl std::ops::BitOr for Capabilities {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtocolHello {
    pub minimum_version: u16,
    pub maximum_version: u16,
    pub capabilities: Capabilities,
    pub max_frame_bytes: u32,
    pub boot_id: [u8; 16],
    /// Signed cluster/stable-node identity for autonomous protocol-two peers.
    pub identity_envelope: Vec<u8>,
}

impl ProtocolHello {
    pub fn current(boot_id: [u8; 16]) -> Self {
        Self {
            minimum_version: PROTOCOL_V1,
            maximum_version: PROTOCOL_V2,
            capabilities: Capabilities::AUTONOMOUS_REQUIRED,
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
            boot_id,
            identity_envelope: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.minimum_version == 0 || self.minimum_version > self.maximum_version {
            return Err("protocol_version_range_invalid".to_string());
        }
        if !(1024..=MAX_NEGOTIATED_FRAME_BYTES).contains(&self.max_frame_bytes) {
            return Err("protocol_frame_bound_invalid".to_string());
        }
        if self.identity_envelope.len() > MAX_IDENTITY_ENVELOPE_BYTES {
            return Err("protocol_identity_envelope_too_large".to_string());
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let mut bytes = Vec::with_capacity(38 + self.identity_envelope.len());
        bytes.extend_from_slice(HELLO_MAGIC);
        bytes.extend_from_slice(&self.minimum_version.to_be_bytes());
        bytes.extend_from_slice(&self.maximum_version.to_be_bytes());
        bytes.extend_from_slice(&self.capabilities.bits().to_be_bytes());
        bytes.extend_from_slice(&self.max_frame_bytes.to_be_bytes());
        bytes.extend_from_slice(&self.boot_id);
        if !self.identity_envelope.is_empty() {
            bytes.extend_from_slice(&(self.identity_envelope.len() as u16).to_be_bytes());
            bytes.extend_from_slice(&self.identity_envelope);
        }
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < 36 || bytes.get(..4) != Some(HELLO_MAGIC) {
            return Err("protocol_hello_invalid".to_string());
        }
        let identity_envelope = if bytes.len() == 36 {
            Vec::new()
        } else {
            if bytes.len() < 38 {
                return Err("protocol_hello_invalid".to_string());
            }
            let length = u16::from_be_bytes(bytes[36..38].try_into().unwrap()) as usize;
            if length > MAX_IDENTITY_ENVELOPE_BYTES || bytes.len() != 38 + length {
                return Err("protocol_hello_invalid".to_string());
            }
            bytes[38..].to_vec()
        };
        let hello = Self {
            minimum_version: u16::from_be_bytes(bytes[4..6].try_into().unwrap()),
            maximum_version: u16::from_be_bytes(bytes[6..8].try_into().unwrap()),
            capabilities: Capabilities::from_bits(u64::from_be_bytes(
                bytes[8..16].try_into().unwrap(),
            )),
            max_frame_bytes: u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
            boot_id: bytes[20..36].try_into().unwrap(),
            identity_envelope,
        };
        hello.validate()?;
        Ok(hello)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NegotiatedProtocol {
    pub version: u16,
    pub capabilities: Capabilities,
    pub max_frame_bytes: u32,
    pub autonomous_enabled: bool,
    pub disabled_reason: Option<String>,
}

pub fn negotiate_protocol(
    local: &ProtocolHello,
    remote: &ProtocolHello,
) -> Result<NegotiatedProtocol, String> {
    local.validate()?;
    remote.validate()?;
    let minimum = local.minimum_version.max(remote.minimum_version);
    let maximum = local.maximum_version.min(remote.maximum_version);
    if minimum > maximum {
        return Err("protocol_no_common_version".to_string());
    }
    let version = maximum;
    let capabilities = local.capabilities.intersection(remote.capabilities);
    let autonomous_enabled =
        version >= PROTOCOL_V2 && capabilities.contains(Capabilities::AUTONOMOUS_REQUIRED);
    Ok(NegotiatedProtocol {
        version,
        capabilities,
        max_frame_bytes: local.max_frame_bytes.min(remote.max_frame_bytes),
        autonomous_enabled,
        disabled_reason: (!autonomous_enabled).then(|| {
            if version < PROTOCOL_V2 {
                "protocol_two_not_negotiated".to_string()
            } else {
                "autonomous_capabilities_incomplete".to_string()
            }
        }),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MessageClass {
    Control = 1,
    Heartbeat = 2,
    Operator = 3,
    Application = 4,
    Snapshot = 5,
}

impl TryFrom<u8> for MessageClass {
    type Error = String;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Control),
            2 => Ok(Self::Heartbeat),
            3 => Ok(Self::Operator),
            4 => Ok(Self::Application),
            5 => Ok(Self::Snapshot),
            _ => Err("protocol_message_class_invalid".to_string()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtocolEnvelope {
    pub class: MessageClass,
    pub kind: u16,
    pub correlation_id: u64,
    pub chunk_sequence: u32,
    pub final_chunk: bool,
    pub payload: Vec<u8>,
}

impl ProtocolEnvelope {
    const HEADER_BYTES: usize = 4 + 1 + 2 + 8 + 4 + 1 + 4 + 32;

    pub fn encode(&self, max_frame_bytes: u32) -> Result<Vec<u8>, String> {
        let payload_len: u32 = self
            .payload
            .len()
            .try_into()
            .map_err(|_| "protocol_payload_too_large".to_string())?;
        if Self::HEADER_BYTES.saturating_add(self.payload.len()) > max_frame_bytes as usize {
            return Err("protocol_frame_bound_exceeded".to_string());
        }
        let checksum: [u8; 32] = Sha256::digest(&self.payload).into();
        let mut bytes = Vec::with_capacity(Self::HEADER_BYTES + self.payload.len());
        bytes.extend_from_slice(ENVELOPE_MAGIC);
        bytes.push(self.class as u8);
        bytes.extend_from_slice(&self.kind.to_be_bytes());
        bytes.extend_from_slice(&self.correlation_id.to_be_bytes());
        bytes.extend_from_slice(&self.chunk_sequence.to_be_bytes());
        bytes.push(u8::from(self.final_chunk));
        bytes.extend_from_slice(&payload_len.to_be_bytes());
        bytes.extend_from_slice(&checksum);
        bytes.extend_from_slice(&self.payload);
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8], max_frame_bytes: u32) -> Result<Self, String> {
        if bytes.len() > max_frame_bytes as usize {
            return Err("protocol_frame_bound_exceeded".to_string());
        }
        if bytes.len() < Self::HEADER_BYTES || bytes.get(..4) != Some(ENVELOPE_MAGIC) {
            return Err("protocol_envelope_truncated".to_string());
        }
        let payload_len = u32::from_be_bytes(bytes[20..24].try_into().unwrap()) as usize;
        if Self::HEADER_BYTES.saturating_add(payload_len) != bytes.len() {
            return Err("protocol_payload_length_invalid".to_string());
        }
        let payload = bytes[Self::HEADER_BYTES..].to_vec();
        let expected: [u8; 32] = bytes[24..56].try_into().unwrap();
        if <[u8; 32]>::from(Sha256::digest(&payload)) != expected {
            return Err("protocol_payload_checksum_mismatch".to_string());
        }
        Ok(Self {
            class: MessageClass::try_from(bytes[4])?,
            kind: u16::from_be_bytes(bytes[5..7].try_into().unwrap()),
            correlation_id: u64::from_be_bytes(bytes[7..15].try_into().unwrap()),
            chunk_sequence: u32::from_be_bytes(bytes[15..19].try_into().unwrap()),
            final_chunk: match bytes[19] {
                0 => false,
                1 => true,
                _ => return Err("protocol_final_chunk_flag_invalid".to_string()),
            },
            payload,
        })
    }
}

pub fn chunk_payload(
    class: MessageClass,
    kind: u16,
    correlation_id: u64,
    payload: &[u8],
    max_frame_bytes: u32,
) -> Result<Vec<ProtocolEnvelope>, String> {
    let chunk_bytes = (max_frame_bytes as usize)
        .checked_sub(ProtocolEnvelope::HEADER_BYTES)
        .filter(|value| *value > 0)
        .ok_or_else(|| "protocol_frame_bound_too_small".to_string())?;
    let chunk_count = payload.len().max(1).div_ceil(chunk_bytes);
    let mut envelopes = Vec::with_capacity(chunk_count);
    if payload.is_empty() {
        envelopes.push(ProtocolEnvelope {
            class,
            kind,
            correlation_id,
            chunk_sequence: 0,
            final_chunk: true,
            payload: Vec::new(),
        });
        return Ok(envelopes);
    }
    for (sequence, chunk) in payload.chunks(chunk_bytes).enumerate() {
        envelopes.push(ProtocolEnvelope {
            class,
            kind,
            correlation_id,
            chunk_sequence: sequence
                .try_into()
                .map_err(|_| "protocol_chunk_count_exceeded".to_string())?,
            final_chunk: sequence + 1 == chunk_count,
            payload: chunk.to_vec(),
        });
    }
    Ok(envelopes)
}

#[derive(Debug)]
struct PartialMessage {
    next_sequence: u32,
    bytes: Vec<u8>,
    opened_at: Instant,
}

#[derive(Debug)]
pub struct ChunkReassembler {
    max_messages: usize,
    max_message_bytes: usize,
    timeout: Duration,
    partial: BTreeMap<u64, PartialMessage>,
}

impl ChunkReassembler {
    pub fn new(
        max_messages: usize,
        max_message_bytes: usize,
        timeout: Duration,
    ) -> Result<Self, String> {
        if max_messages == 0 || max_message_bytes == 0 || timeout.is_zero() {
            return Err("protocol_reassembler_limits_invalid".to_string());
        }
        Ok(Self {
            max_messages,
            max_message_bytes,
            timeout,
            partial: BTreeMap::new(),
        })
    }

    pub fn push(
        &mut self,
        envelope: ProtocolEnvelope,
        now: Instant,
    ) -> Result<Option<Vec<u8>>, String> {
        self.expire(now);
        if envelope.chunk_sequence == 0 && envelope.final_chunk {
            if envelope.payload.len() > self.max_message_bytes {
                return Err("protocol_reassembled_message_too_large".to_string());
            }
            return Ok(Some(envelope.payload));
        }
        if envelope.chunk_sequence == 0 {
            if !self.partial.contains_key(&envelope.correlation_id)
                && self.partial.len() >= self.max_messages
            {
                return Err("protocol_reassembly_capacity_exhausted".to_string());
            }
            self.partial.insert(
                envelope.correlation_id,
                PartialMessage {
                    next_sequence: 1,
                    bytes: envelope.payload,
                    opened_at: now,
                },
            );
            return Ok(None);
        }
        let partial = self
            .partial
            .get_mut(&envelope.correlation_id)
            .ok_or_else(|| "protocol_chunk_without_start".to_string())?;
        if partial.next_sequence != envelope.chunk_sequence {
            self.partial.remove(&envelope.correlation_id);
            return Err("protocol_chunk_out_of_order".to_string());
        }
        if partial.bytes.len().saturating_add(envelope.payload.len()) > self.max_message_bytes {
            self.partial.remove(&envelope.correlation_id);
            return Err("protocol_reassembled_message_too_large".to_string());
        }
        partial.bytes.extend_from_slice(&envelope.payload);
        partial.next_sequence = partial.next_sequence.saturating_add(1);
        if envelope.final_chunk {
            return Ok(self
                .partial
                .remove(&envelope.correlation_id)
                .map(|message| message.bytes));
        }
        Ok(None)
    }

    pub fn expire(&mut self, now: Instant) {
        self.partial
            .retain(|_, message| now.saturating_duration_since(message.opened_at) <= self.timeout);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryClass {
    Never,
    SafeBeforeAdmission,
    SafeWithOperationKey,
}

pub fn classify_retry(error: &str, has_operation_key: bool) -> RetryClass {
    if matches!(
        error,
        "queue_full" | "peer_unavailable" | "connection_reset"
    ) {
        RetryClass::SafeBeforeAdmission
    } else if has_operation_key && matches!(error, "reply_timeout" | "owner_lost") {
        RetryClass::SafeWithOperationKey
    } else {
        RetryClass::Never
    }
}

/// Sliding-window retry budget. Original attempts earn a bounded percentage of
/// retries, preventing recovery traffic from amplifying an outage.
#[derive(Debug)]
pub struct RetryBudget {
    percent: u8,
    minimum_retries: u32,
    window: Duration,
    window_started: Instant,
    originals: u64,
    retries: u64,
}

impl RetryBudget {
    pub fn new(
        percent: u8,
        minimum_retries: u32,
        window: Duration,
        now: Instant,
    ) -> Result<Self, String> {
        if percent > 100 || window.is_zero() {
            return Err("retry_budget_limits_invalid".to_string());
        }
        Ok(Self {
            percent,
            minimum_retries,
            window,
            window_started: now,
            originals: 0,
            retries: 0,
        })
    }

    pub fn record_original(&mut self, now: Instant) {
        self.roll_window(now);
        self.originals = self.originals.saturating_add(1);
    }

    pub fn try_retry(&mut self, now: Instant) -> bool {
        self.roll_window(now);
        let percentage_allowance = self
            .originals
            .saturating_mul(u64::from(self.percent))
            .div_ceil(100);
        let allowance = percentage_allowance.max(u64::from(self.minimum_retries));
        if self.retries >= allowance {
            return false;
        }
        self.retries = self.retries.saturating_add(1);
        true
    }

    fn roll_window(&mut self, now: Instant) {
        if now.saturating_duration_since(self.window_started) >= self.window {
            self.window_started = now;
            self.originals = 0;
            self.retries = 0;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

#[derive(Debug)]
pub struct CircuitBreaker {
    failure_threshold: u32,
    reset_after: Duration,
    failures: u32,
    opened_at: Option<Instant>,
    probe_inflight: bool,
}

impl CircuitBreaker {
    pub fn new(failure_threshold: u32, reset_after: Duration) -> Result<Self, String> {
        if failure_threshold == 0 || reset_after.is_zero() {
            return Err("circuit_breaker_limits_invalid".to_string());
        }
        Ok(Self {
            failure_threshold,
            reset_after,
            failures: 0,
            opened_at: None,
            probe_inflight: false,
        })
    }

    pub fn state(&self, now: Instant) -> CircuitState {
        match self.opened_at {
            None => CircuitState::Closed,
            Some(opened) if now.saturating_duration_since(opened) >= self.reset_after => {
                CircuitState::HalfOpen
            }
            Some(_) => CircuitState::Open,
        }
    }

    pub fn allow(&mut self, now: Instant) -> bool {
        match self.state(now) {
            CircuitState::Closed => true,
            CircuitState::Open => false,
            CircuitState::HalfOpen if !self.probe_inflight => {
                self.probe_inflight = true;
                true
            }
            CircuitState::HalfOpen => false,
        }
    }

    pub fn record_success(&mut self) {
        self.failures = 0;
        self.opened_at = None;
        self.probe_inflight = false;
    }

    pub fn record_failure(&mut self, now: Instant) {
        self.probe_inflight = false;
        self.failures = self.failures.saturating_add(1);
        if self.failures >= self.failure_threshold {
            self.opened_at = Some(now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autonomous_mode_requires_every_capability() {
        let local = ProtocolHello::current([1; 16]);
        let mut remote = ProtocolHello::current([2; 16]);
        remote.capabilities = Capabilities::MULTIPLEXED_REQUESTS;
        let negotiated = negotiate_protocol(&local, &remote).expect("common protocol");

        assert_eq!(negotiated.version, PROTOCOL_V2);
        assert!(!negotiated.autonomous_enabled);
        assert_eq!(
            negotiated.disabled_reason.as_deref(),
            Some("autonomous_capabilities_incomplete")
        );
    }

    #[test]
    fn rolling_protocol_one_to_two_upgrade_and_rollback_is_reversible() {
        let current = ProtocolHello::current([1; 16]);
        let protocol_one = ProtocolHello {
            minimum_version: PROTOCOL_V1,
            maximum_version: PROTOCOL_V1,
            capabilities: Capabilities::default(),
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
            boot_id: [2; 16],
            identity_envelope: Vec::new(),
        };

        let mixed =
            negotiate_protocol(&current, &protocol_one).expect("mixed-version compatibility");
        assert_eq!(mixed.version, PROTOCOL_V1);
        assert!(!mixed.autonomous_enabled);
        assert_eq!(
            mixed.disabled_reason.as_deref(),
            Some("protocol_two_not_negotiated")
        );

        let upgraded = negotiate_protocol(&current, &ProtocolHello::current([3; 16]))
            .expect("all-current negotiation");
        assert_eq!(upgraded.version, PROTOCOL_V2);
        assert!(upgraded.autonomous_enabled);
        assert!(upgraded.disabled_reason.is_none());

        let rolled_back = negotiate_protocol(&protocol_one, &current)
            .expect("rollback compatibility remains available");
        assert_eq!(rolled_back.version, PROTOCOL_V1);
        assert!(!rolled_back.autonomous_enabled);
    }

    #[test]
    fn envelope_round_trip_rejects_corruption() {
        let envelope = ProtocolEnvelope {
            class: MessageClass::Application,
            kind: 7,
            correlation_id: 42,
            chunk_sequence: 0,
            final_chunk: true,
            payload: b"payload".to_vec(),
        };
        let encoded = envelope.encode(1024).expect("encode");
        assert_eq!(ProtocolEnvelope::decode(&encoded, 1024).unwrap(), envelope);
        let mut corrupted = encoded;
        *corrupted.last_mut().unwrap() ^= 1;
        assert_eq!(
            ProtocolEnvelope::decode(&corrupted, 1024),
            Err("protocol_payload_checksum_mismatch".to_string())
        );
    }

    #[test]
    fn chunked_message_reassembles_with_bounds() {
        let payload = vec![9; 4096];
        let chunks = chunk_payload(MessageClass::Snapshot, 1, 8, &payload, 256).unwrap();
        assert!(chunks.len() > 1);
        let start = Instant::now();
        let mut reassembler = ChunkReassembler::new(2, 8192, Duration::from_secs(1)).unwrap();
        let mut result = None;
        for chunk in chunks {
            result = reassembler.push(chunk, start).unwrap().or(result);
        }
        assert_eq!(result, Some(payload));
    }

    #[test]
    fn circuit_breaker_allows_one_probe_after_timeout() {
        let start = Instant::now();
        let mut breaker = CircuitBreaker::new(2, Duration::from_secs(1)).unwrap();
        breaker.record_failure(start);
        breaker.record_failure(start);
        assert!(!breaker.allow(start));
        assert!(breaker.allow(start + Duration::from_secs(1)));
        assert!(!breaker.allow(start + Duration::from_secs(1)));
        breaker.record_success();
        assert_eq!(breaker.state(start), CircuitState::Closed);
    }

    #[test]
    fn retry_budget_bounds_recovery_amplification() {
        let start = Instant::now();
        let mut budget = RetryBudget::new(10, 1, Duration::from_secs(10), start).unwrap();
        for _ in 0..20 {
            budget.record_original(start);
        }
        assert!(budget.try_retry(start));
        assert!(budget.try_retry(start));
        assert!(!budget.try_retry(start));
        assert!(budget.try_retry(start + Duration::from_secs(10)));
    }
}
