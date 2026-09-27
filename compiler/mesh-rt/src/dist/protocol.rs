//! Versioned peer protocol contracts, capability negotiation, and retry guards.

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
    /// A budget of `percent` (at most 100) of the originals in each
    /// `window` (not zero), and at least `minimum_retries`.
    pub fn new(percent: u8, minimum_retries: u32, window: Duration, now: Instant) -> Self {
        Self {
            percent,
            minimum_retries,
            window,
            window_started: now,
            originals: 0,
            retries: 0,
        }
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
    /// A breaker that opens after `failure_threshold` (not zero) failures in
    /// a row and lets a probe through `reset_after` (not zero) later.
    pub fn new(failure_threshold: u32, reset_after: Duration) -> Self {
        Self {
            failure_threshold,
            reset_after,
            failures: 0,
            opened_at: None,
            probe_inflight: false,
        }
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

    /// A peer's hello that is cut short, out of range or shares no version
    /// with this node's is refused.
    #[test]
    fn hellos_out_of_range_are_refused() {
        let current = ProtocolHello::current([1; 16]);
        for (hello, reason) in [
            (
                ProtocolHello {
                    minimum_version: 0,
                    ..current.clone()
                },
                "protocol_version_range_invalid",
            ),
            (
                ProtocolHello {
                    max_frame_bytes: 1023,
                    ..current.clone()
                },
                "protocol_frame_bound_invalid",
            ),
            (
                ProtocolHello {
                    identity_envelope: vec![0; MAX_IDENTITY_ENVELOPE_BYTES + 1],
                    ..current.clone()
                },
                "protocol_identity_envelope_too_large",
            ),
        ] {
            assert_eq!(hello.validate(), Err(reason.to_string()));
        }
        let encoded = current.encode().unwrap();
        assert_eq!(
            ProtocolHello::decode(&encoded[..35]),
            Err("protocol_hello_invalid".to_string())
        );
        let mut one_length_byte = encoded.clone();
        one_length_byte.push(0);
        assert_eq!(
            ProtocolHello::decode(&one_length_byte),
            Err("protocol_hello_invalid".to_string())
        );
        let newer = ProtocolHello {
            minimum_version: PROTOCOL_V2 + 1,
            maximum_version: PROTOCOL_V2 + 1,
            ..current.clone()
        };
        assert_eq!(
            negotiate_protocol(&current, &newer),
            Err("protocol_no_common_version".to_string())
        );
    }

    /// Every header field of an envelope is checked before its payload is
    /// read: the frame bound, the payload length, the class and the
    /// final-chunk flag.
    #[test]
    fn envelope_headers_out_of_range_are_refused() {
        let envelope = ProtocolEnvelope {
            class: MessageClass::Operator,
            kind: 7,
            correlation_id: 42,
            chunk_sequence: 0,
            final_chunk: true,
            payload: b"payload".to_vec(),
        };
        let encoded = envelope.encode(1024).unwrap();
        assert_eq!(
            ProtocolEnvelope::decode(&encoded, 1024),
            Ok(envelope.clone())
        );
        assert_eq!(
            envelope.encode(ProtocolEnvelope::HEADER_BYTES as u32 + 6),
            Err("protocol_frame_bound_exceeded".to_string())
        );
        assert_eq!(
            ProtocolEnvelope::decode(&encoded, encoded.len() as u32 - 1),
            Err("protocol_frame_bound_exceeded".to_string())
        );
        assert_eq!(
            ProtocolEnvelope::decode(&encoded[..encoded.len() - 1], 1024),
            Err("protocol_payload_length_invalid".to_string())
        );
        let mut unknown_class = encoded.clone();
        unknown_class[4] = 9;
        assert_eq!(
            ProtocolEnvelope::decode(&unknown_class, 1024),
            Err("protocol_message_class_invalid".to_string())
        );
        let mut bad_flag = encoded;
        bad_flag[19] = 2;
        assert_eq!(
            ProtocolEnvelope::decode(&bad_flag, 1024),
            Err("protocol_final_chunk_flag_invalid".to_string())
        );
    }

    #[test]
    fn circuit_breaker_allows_one_probe_after_timeout() {
        let start = Instant::now();
        let mut breaker = CircuitBreaker::new(2, Duration::from_secs(1));
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
        let mut budget = RetryBudget::new(10, 1, Duration::from_secs(10), start);
        for _ in 0..20 {
            budget.record_original(start);
        }
        assert!(budget.try_retry(start));
        assert!(budget.try_retry(start));
        assert!(!budget.try_retry(start));
        assert!(budget.try_retry(start + Duration::from_secs(10)));
    }
}
