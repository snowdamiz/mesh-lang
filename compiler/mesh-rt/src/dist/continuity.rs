//! Runtime-owned keyed continuity state machine and healthy-path cluster sync.
//!
//! This module lifts the request-key continuity contract out of Mesh app code
//! and into `mesh-rt`. The registry owns:
//!
//! - request-key dedupe vs conflict decisions
//! - attempt token / attempt id generation
//! - completion transitions
//! - explicit owner / replica status fields
//! - healthy-path record replication across connected nodes
//!
//! The state model stays explicit so later slices can add fail-closed
//! durability and owner-loss recovery without changing the record shape.

use crate::gc::mesh_gc_alloc_actor;
use crate::io::{alloc_result, MeshResult};
use crate::string::{mesh_string_new, MeshString};
use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, OnceLock};

use super::continuity_store::{
    configured_continuity_store, ContinuityLogEntry, ContinuityStore, SnapshotChunk,
};

const CONTINUITY_CONFLICT_REASON: &str = "request_key_conflict";
const ATTEMPT_ID_MISMATCH: &str = "attempt_id_mismatch";
const EXECUTION_NODE_MISSING: &str = "execution_node_missing";
const REQUEST_KEY_NOT_FOUND: &str = "request_key_not_found";
const OWNER_NODE_MISSING: &str = "owner_node_missing";
const REQUEST_KEY_MISSING: &str = "request_key_missing";
const PAYLOAD_HASH_MISSING: &str = "payload_hash_missing";
const ATTEMPT_ID_MISSING: &str = "attempt_id_missing";
const REPLICA_NODE_MISSING: &str = "replica_node_missing";
const REPLICA_MATCHES_OWNER: &str = "replica_matches_owner";
const INVALID_REPLICATION_COUNT: &str = "invalid_replication_count";
const INVALID_REQUIRED_REPLICA_COUNT: &str = "invalid_required_replica_count";
const REPLICA_REQUIRED_UNAVAILABLE: &str = "replica_required_unavailable";
const REPLICA_PREPARE_TIMEOUT: &str = "replica_prepare_timeout";
const TRANSITION_REJECTED_ALREADY_COMPLETED: &str = "transition_rejected:already_completed";
const TRANSITION_REJECTED_PHASE: &str = "transition_rejected:phase";
const CONTINUITY_ROLE_ENV: &str = "MESH_CONTINUITY_ROLE";
const CONTINUITY_PROMOTION_EPOCH_ENV: &str = "MESH_CONTINUITY_PROMOTION_EPOCH";
const STANDBY_OWNER_LOST_INVALID: &str = "standby_owner_lost_invalid";
#[cfg_attr(not(test), allow(dead_code))]
const PROMOTION_REJECTED_NOT_STANDBY: &str = "promotion_rejected:not_standby";
#[cfg_attr(not(test), allow(dead_code))]
const PROMOTION_REJECTED_NO_MIRRORED_STATE: &str = "promotion_rejected:no_mirrored_state";
const STALE_PROMOTION_EPOCH_REJECTED: &str = "stale_promotion_epoch_rejected";

pub(crate) fn request_key_fingerprint(request_key: &str) -> String {
    let digest = Sha256::digest(request_key.as_bytes());
    let short = digest[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("sha256:{short}")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContinuityPhase {
    Submitted,
    Completed,
    Rejected,
}

impl ContinuityPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            ContinuityPhase::Submitted => "submitted",
            ContinuityPhase::Completed => "completed",
            ContinuityPhase::Rejected => "rejected",
        }
    }

    fn to_wire(self) -> u8 {
        match self {
            ContinuityPhase::Submitted => 0,
            ContinuityPhase::Completed => 1,
            ContinuityPhase::Rejected => 2,
        }
    }

    fn from_wire(value: u8) -> Result<Self, String> {
        match value {
            0 => Ok(Self::Submitted),
            1 => Ok(Self::Completed),
            2 => Ok(Self::Rejected),
            _ => Err(format!("invalid continuity phase {}", value)),
        }
    }

    fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Rejected)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContinuityResult {
    Pending,
    Succeeded,
    Rejected,
}

impl ContinuityResult {
    pub fn as_str(self) -> &'static str {
        match self {
            ContinuityResult::Pending => "pending",
            ContinuityResult::Succeeded => "succeeded",
            ContinuityResult::Rejected => "rejected",
        }
    }

    fn to_wire(self) -> u8 {
        match self {
            ContinuityResult::Pending => 0,
            ContinuityResult::Succeeded => 1,
            ContinuityResult::Rejected => 2,
        }
    }

    fn from_wire(value: u8) -> Result<Self, String> {
        match value {
            0 => Ok(Self::Pending),
            1 => Ok(Self::Succeeded),
            2 => Ok(Self::Rejected),
            _ => Err(format!("invalid continuity result {}", value)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplicaStatus {
    Unassigned,
    Preparing,
    Mirrored,
    OwnerLost,
    PreAdmissionRejected,
    Rejected,
    DegradedContinuing,
}

impl ReplicaStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ReplicaStatus::Unassigned => "unassigned",
            ReplicaStatus::Preparing => "preparing",
            ReplicaStatus::Mirrored => "mirrored",
            ReplicaStatus::OwnerLost => "owner_lost",
            ReplicaStatus::PreAdmissionRejected => "pre_admission_rejected",
            ReplicaStatus::Rejected => "rejected",
            ReplicaStatus::DegradedContinuing => "degraded_continuing",
        }
    }

    fn to_wire(self) -> u8 {
        match self {
            ReplicaStatus::Unassigned => 0,
            ReplicaStatus::Preparing => 1,
            ReplicaStatus::Mirrored => 2,
            ReplicaStatus::OwnerLost => 3,
            ReplicaStatus::Rejected => 4,
            ReplicaStatus::DegradedContinuing => 5,
            ReplicaStatus::PreAdmissionRejected => 6,
        }
    }

    fn from_wire(value: u8) -> Result<Self, String> {
        match value {
            0 => Ok(Self::Unassigned),
            1 => Ok(Self::Preparing),
            2 => Ok(Self::Mirrored),
            3 => Ok(Self::OwnerLost),
            4 => Ok(Self::Rejected),
            5 => Ok(Self::DegradedContinuing),
            6 => Ok(Self::PreAdmissionRejected),
            _ => Err(format!("invalid continuity replica status {}", value)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContinuityClusterRole {
    Primary,
    Standby,
}

impl ContinuityClusterRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Standby => "standby",
        }
    }

    fn to_wire(self) -> u8 {
        match self {
            Self::Primary => 0,
            Self::Standby => 1,
        }
    }

    fn from_wire(value: u8) -> Result<Self, String> {
        match value {
            0 => Ok(Self::Primary),
            1 => Ok(Self::Standby),
            _ => Err(format!("invalid continuity cluster role {}", value)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplicationHealth {
    LocalOnly,
    Healthy,
    Degraded,
    Unavailable,
}

impl ReplicationHealth {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalOnly => "local_only",
            Self::Healthy => "healthy",
            Self::Degraded => "degraded",
            Self::Unavailable => "unavailable",
        }
    }

    fn to_wire(self) -> u8 {
        match self {
            Self::LocalOnly => 0,
            Self::Healthy => 1,
            Self::Degraded => 2,
            Self::Unavailable => 3,
        }
    }

    fn from_wire(value: u8) -> Result<Self, String> {
        match value {
            0 => Ok(Self::LocalOnly),
            1 => Ok(Self::Healthy),
            2 => Ok(Self::Degraded),
            3 => Ok(Self::Unavailable),
            _ => Err(format!("invalid continuity replication health {}", value)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ContinuityAuthorityConfig {
    cluster_role: ContinuityClusterRole,
    promotion_epoch: u64,
}

impl Default for ContinuityAuthorityConfig {
    fn default() -> Self {
        Self {
            cluster_role: ContinuityClusterRole::Primary,
            promotion_epoch: 0,
        }
    }
}

impl ContinuityAuthorityConfig {
    fn validate(self) -> Result<Self, String> {
        Ok(self)
    }

    fn from_record(record: &ContinuityRecord) -> Self {
        Self {
            cluster_role: record.cluster_role,
            promotion_epoch: record.promotion_epoch,
        }
    }

    fn follower_for_epoch(self, promotion_epoch: u64) -> Self {
        Self {
            cluster_role: ContinuityClusterRole::Standby,
            promotion_epoch,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn promoted(self) -> Self {
        Self {
            cluster_role: ContinuityClusterRole::Primary,
            promotion_epoch: self.promotion_epoch.saturating_add(1),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContinuityAuthorityStatus {
    pub cluster_role: ContinuityClusterRole,
    pub promotion_epoch: u64,
    pub replication_health: ReplicationHealth,
}

fn parse_authority_config(
    role: Option<&str>,
    promotion_epoch: Option<&str>,
) -> Result<ContinuityAuthorityConfig, String> {
    let cluster_role = match role.map(str::trim) {
        None | Some("") => ContinuityClusterRole::Primary,
        Some("primary") => ContinuityClusterRole::Primary,
        Some("standby") => ContinuityClusterRole::Standby,
        Some(other) => return Err(format!("invalid continuity role {other}")),
    };
    let promotion_epoch = match promotion_epoch.map(str::trim) {
        None | Some("") => 0,
        Some(raw) => raw
            .parse::<u64>()
            .map_err(|_| format!("invalid continuity promotion epoch {raw}"))?,
    };
    ContinuityAuthorityConfig {
        cluster_role,
        promotion_epoch,
    }
    .validate()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContinuityRecord {
    pub request_key: String,
    pub payload_hash: String,
    /// Monotonic state version within one request identity.
    pub(crate) record_version: u64,
    /// Encoded request needed to resume an in-flight operation after owner loss.
    /// Empty for declared work that reconstructs its arguments from the key.
    pub(crate) request_payload: Vec<u8>,
    pub attempt_id: String,
    pub phase: ContinuityPhase,
    pub result: ContinuityResult,
    pub ingress_node: String,
    pub owner_node: String,
    /// Replica targets selected for this attempt.
    ///
    /// `replica_node` remains the language/FFI-compatible primary replica.
    pub(crate) replica_nodes: Vec<String>,
    /// Replica targets that actually acknowledged the current attempt.
    /// Runtime safety decisions must use this complete set.
    pub(crate) acknowledged_replica_nodes: Vec<String>,
    pub replica_node: String,
    pub replication_count: u64,
    pub replica_status: ReplicaStatus,
    pub cluster_role: ContinuityClusterRole,
    pub promotion_epoch: u64,
    pub replication_health: ReplicationHealth,
    pub execution_node: String,
    pub routed_remotely: bool,
    pub fell_back_locally: bool,
    pub error: String,
    pub(crate) declared_handler_runtime_name: String,
}

impl ContinuityRecord {
    /// Returns the declared-handler runtime name associated with this record.
    ///
    /// Records created through non-declared continuity paths return an empty string.
    pub fn declared_handler_runtime_name(&self) -> &str {
        &self.declared_handler_runtime_name
    }

    /// Returns the retained request bytes used by recoverable HTTP handlers.
    pub fn request_payload(&self) -> &[u8] {
        &self.request_payload
    }

    /// Returns the exact acknowledged replica holders for this attempt.
    pub fn replica_nodes(&self) -> &[String] {
        &self.replica_nodes
    }

    pub fn acknowledged_replica_nodes(&self) -> &[String] {
        &self.acknowledged_replica_nodes
    }

    pub(crate) fn canonical_replica_nodes(&self) -> Vec<String> {
        if self.replica_nodes.is_empty() {
            if self.replica_node.is_empty() {
                Vec::new()
            } else {
                vec![self.replica_node.clone()]
            }
        } else {
            self.replica_nodes.clone()
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.request_key.is_empty() {
            return Err(REQUEST_KEY_MISSING.to_string());
        }
        if self.payload_hash.is_empty() {
            return Err(PAYLOAD_HASH_MISSING.to_string());
        }
        if self.attempt_id.is_empty() {
            return Err(ATTEMPT_ID_MISSING.to_string());
        }
        if self.record_version == 0 {
            return Err("continuity_record_version_invalid".to_string());
        }
        if self.owner_node.is_empty() {
            return Err(OWNER_NODE_MISSING.to_string());
        }
        if self.replication_count == 0 {
            return Err(INVALID_REPLICATION_COUNT.to_string());
        }
        let replicas = self.canonical_replica_nodes();
        let mut normalized = replicas.clone();
        normalized.sort();
        normalized.dedup();
        if normalized.len() != replicas.len()
            || replicas.iter().any(String::is_empty)
            || replicas.iter().any(|node| node == &self.owner_node)
            || (!self.replica_node.is_empty() && !replicas.contains(&self.replica_node))
        {
            return Err("continuity_replica_set_invalid".to_string());
        }
        let mut acknowledgements = self.acknowledged_replica_nodes.clone();
        acknowledgements.sort();
        acknowledgements.dedup();
        if acknowledgements.len() != self.acknowledged_replica_nodes.len()
            || acknowledgements
                .iter()
                .any(|replica| !replicas.contains(replica))
        {
            return Err("continuity_replica_ack_set_invalid".to_string());
        }
        if !self.replica_node.is_empty() && self.replica_node == self.owner_node {
            return Err(REPLICA_MATCHES_OWNER.to_string());
        }
        if self.cluster_role == ContinuityClusterRole::Standby
            && self.replica_status == ReplicaStatus::OwnerLost
        {
            return Err(STANDBY_OWNER_LOST_INVALID.to_string());
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct SubmitRequest {
    pub request_key: String,
    pub payload_hash: String,
    pub(crate) request_payload: Vec<u8>,
    pub ingress_node: String,
    pub owner_node: String,
    pub(crate) replica_nodes: Vec<String>,
    pub replica_node: String,
    pub replication_count: u64,
    pub required_replica_count: u64,
    pub routed_remotely: bool,
    pub fell_back_locally: bool,
    pub cluster_role: ContinuityClusterRole,
    pub promotion_epoch: u64,
    pub(crate) declared_handler_runtime_name: String,
}

impl SubmitRequest {
    fn validate(&self) -> Result<(), String> {
        if self.request_key.is_empty() {
            return Err(REQUEST_KEY_MISSING.to_string());
        }
        if self.payload_hash.is_empty() {
            return Err(PAYLOAD_HASH_MISSING.to_string());
        }
        if self.owner_node.is_empty() {
            return Err(OWNER_NODE_MISSING.to_string());
        }
        if self.replication_count == 0 {
            return Err(INVALID_REPLICATION_COUNT.to_string());
        }
        if self.required_replica_count > self.replication_count.saturating_sub(1) {
            return Err(INVALID_REQUIRED_REPLICA_COUNT.to_string());
        }
        let mut replicas = self.replica_nodes.clone();
        if replicas.is_empty() && !self.replica_node.is_empty() {
            replicas.push(self.replica_node.clone());
        }
        let mut normalized = replicas.clone();
        normalized.sort();
        normalized.dedup();
        if normalized.len() != replicas.len()
            || replicas.iter().any(String::is_empty)
            || replicas.iter().any(|node| node == &self.owner_node)
            || (!self.replica_node.is_empty() && !replicas.contains(&self.replica_node))
        {
            return Err("continuity_replica_set_invalid".to_string());
        }
        if !self.replica_node.is_empty() && self.replica_node == self.owner_node {
            return Err(REPLICA_MATCHES_OWNER.to_string());
        }
        ContinuityAuthorityConfig {
            cluster_role: self.cluster_role,
            promotion_epoch: self.promotion_epoch,
        }
        .validate()?;
        Ok(())
    }

    fn requires_replica_prepare(&self) -> bool {
        self.required_replica_count > 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitOutcome {
    Created,
    Duplicate,
    Conflict,
    Rejected,
}

impl SubmitOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            SubmitOutcome::Created => "created",
            SubmitOutcome::Duplicate => "duplicate",
            SubmitOutcome::Conflict => "conflict",
            SubmitOutcome::Rejected => "rejected",
        }
    }
}

#[derive(Clone, Debug)]
pub struct SubmitDecision {
    pub outcome: SubmitOutcome,
    pub record: ContinuityRecord,
    pub conflict_reason: String,
}

#[derive(Clone, Debug, Default)]
pub struct ContinuitySnapshot {
    pub next_attempt_token: u64,
    pub records: Vec<ContinuityRecord>,
}

#[derive(Default)]
struct ContinuityInner {
    next_attempt_token: u64,
    authority: ContinuityAuthorityConfig,
    requests: FxHashMap<String, ContinuityRecord>,
}

pub struct ContinuityRegistry {
    inner: RwLock<ContinuityInner>,
}

impl ContinuityRegistry {
    pub fn new() -> Self {
        Self::new_with_authority(ContinuityAuthorityConfig::default())
    }

    fn new_with_authority(authority: ContinuityAuthorityConfig) -> Self {
        Self {
            inner: RwLock::new(ContinuityInner {
                authority,
                ..ContinuityInner::default()
            }),
        }
    }

    pub(crate) fn authority(&self) -> ContinuityAuthorityConfig {
        self.inner.read().authority
    }

    pub fn authority_status(&self) -> ContinuityAuthorityStatus {
        let inner = self.inner.read();
        ContinuityAuthorityStatus {
            cluster_role: inner.authority.cluster_role,
            promotion_epoch: inner.authority.promotion_epoch,
            replication_health: authority_replication_health(inner.requests.values()),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn promote_authority(&self) -> Result<ContinuityAuthorityConfig, String> {
        let mut inner = self.inner.write();
        if inner.authority.cluster_role != ContinuityClusterRole::Standby {
            return Err(PROMOTION_REJECTED_NOT_STANDBY.to_string());
        }
        if inner.requests.is_empty() {
            return Err(PROMOTION_REJECTED_NO_MIRRORED_STATE.to_string());
        }

        let previous = inner.authority;
        let next = previous.promoted();
        inner.authority = next;
        reproject_records_for_authority_change(&mut inner.requests, previous, next);
        let watermark = inner.next_attempt_token;
        let records: Vec<ContinuityRecord> = inner.requests.values().cloned().collect();
        drop(inner);

        log_promotion(previous, next);
        for record in &records {
            broadcast_continuity_upsert(watermark, record);
        }
        Ok(next)
    }

    pub fn next_attempt_token(&self) -> u64 {
        self.inner.read().next_attempt_token
    }

    pub fn snapshot(&self) -> ContinuitySnapshot {
        let inner = self.inner.read();
        ContinuitySnapshot {
            next_attempt_token: inner.next_attempt_token,
            records: inner.requests.values().cloned().collect(),
        }
    }

    pub fn record(&self, request_key: &str) -> Option<ContinuityRecord> {
        self.inner.read().requests.get(request_key).cloned()
    }

    /// Reserves a monotonically increasing attempt fence for a cooperative
    /// ownership transfer. Reserving does not modify the active record, so a
    /// failed replica prepare leaves the previous owner authoritative.
    pub(crate) fn reserve_transfer_attempt(&self) -> (u64, String) {
        let mut inner = self.inner.write();
        let token = inner.next_attempt_token;
        inner.next_attempt_token = inner.next_attempt_token.saturating_add(1);
        (inner.next_attempt_token, attempt_id_from_token(token))
    }

    /// Atomically installs a drain replacement only while the expected
    /// attempt is still current. This compare-and-swap fences concurrent
    /// completion, retry, and duplicate drain orchestrators.
    pub(crate) fn commit_drain_replacement(
        &self,
        expected_attempt_id: &str,
        record: ContinuityRecord,
    ) -> Result<ContinuityRecord, String> {
        record.validate()?;
        let mut inner = self.inner.write();
        let current = inner
            .requests
            .get(&record.request_key)
            .ok_or_else(|| REQUEST_KEY_NOT_FOUND.to_string())?;
        if current.attempt_id != expected_attempt_id {
            return Err("continuity_drain_attempt_fenced".to_string());
        }
        if current.phase != ContinuityPhase::Submitted
            || current.result != ContinuityResult::Pending
        {
            return Err("continuity_drain_record_not_active".to_string());
        }
        let mut previous_participants =
            BTreeSet::from([current.ingress_node.clone(), current.owner_node.clone()]);
        previous_participants.extend(current.replica_nodes().iter().cloned());
        update_next_attempt_token(&mut inner, &record, None);
        inner
            .requests
            .insert(record.request_key.clone(), record.clone());
        let watermark = inner.next_attempt_token;
        drop(inner);

        log_drain_replacement(&record, expected_attempt_id);
        broadcast_continuity_upsert(watermark, &record);
        for participant in previous_participants {
            send_continuity_upsert_to_node(watermark, &record, &participant);
        }
        Ok(record)
    }

    #[cfg(test)]
    pub(crate) fn clear_for_test(&self) {
        *self.inner.write() = ContinuityInner {
            authority: ContinuityAuthorityConfig::default(),
            ..ContinuityInner::default()
        };
    }

    pub fn submit(&self, request: SubmitRequest) -> Result<SubmitDecision, String> {
        self.submit_with_hooks(
            request,
            super::node::prepare_continuity_replica,
            super::node::continuity_owner_loss_recovery_eligible,
        )
    }

    #[cfg(test)]
    fn submit_with_replica_prepare<F>(
        &self,
        request: SubmitRequest,
        prepare_replica: F,
    ) -> Result<SubmitDecision, String>
    where
        F: FnOnce(&ContinuityRecord) -> Result<(), String>,
    {
        self.submit_with_hooks(
            request,
            |record| prepare_replica(record).map(|()| record.canonical_replica_nodes()),
            |_, _| false,
        )
    }

    fn submit_with_hooks<F, G>(
        &self,
        request: SubmitRequest,
        prepare_replica: F,
        recovery_eligible: G,
    ) -> Result<SubmitDecision, String>
    where
        F: FnOnce(&ContinuityRecord) -> Result<Vec<String>, String>,
        G: FnOnce(&ContinuityRecord, &SubmitRequest) -> bool,
    {
        request.validate()?;

        let requires_replica_prepare = request.requires_replica_prepare();
        let mut inner = self.inner.write();
        if let Some(existing) = inner.requests.get(&request.request_key).cloned() {
            if existing.payload_hash == request.payload_hash {
                if recovery_eligible(&existing, &request)
                    || existing.replica_status == ReplicaStatus::PreAdmissionRejected
                {
                    let attempt_token = inner.next_attempt_token;
                    inner.next_attempt_token += 1;
                    let next =
                        transition_retry_rollover_record(&existing, &request, attempt_token)?;
                    inner
                        .requests
                        .insert(request.request_key.clone(), next.clone());
                    let watermark = inner.next_attempt_token;
                    drop(inner);

                    log_recovery_rollover(&existing, &next);
                    return self.finalize_submit_decision(
                        requires_replica_prepare,
                        request.required_replica_count,
                        watermark,
                        next,
                        prepare_replica,
                    );
                }

                log_duplicate(&existing);
                return Ok(SubmitDecision {
                    outcome: SubmitOutcome::Duplicate,
                    record: existing,
                    conflict_reason: String::new(),
                });
            }

            log_conflict(&existing, &request.request_key, CONTINUITY_CONFLICT_REASON);
            return Ok(SubmitDecision {
                outcome: SubmitOutcome::Conflict,
                record: existing,
                conflict_reason: CONTINUITY_CONFLICT_REASON.to_string(),
            });
        }

        let attempt_token = inner.next_attempt_token;
        inner.next_attempt_token += 1;
        let record = continuity_submitted_record(&request, attempt_token);
        inner
            .requests
            .insert(request.request_key.clone(), record.clone());
        let watermark = inner.next_attempt_token;
        drop(inner);

        self.finalize_submit_decision(
            requires_replica_prepare,
            request.required_replica_count,
            watermark,
            record,
            prepare_replica,
        )
    }

    fn finalize_submit_decision<F>(
        &self,
        requires_replica_prepare: bool,
        required_replica_count: u64,
        watermark: u64,
        record: ContinuityRecord,
        prepare_replica: F,
    ) -> Result<SubmitDecision, String>
    where
        F: FnOnce(&ContinuityRecord) -> Result<Vec<String>, String>,
    {
        log_submit(&record, required_replica_count);

        if !requires_replica_prepare {
            broadcast_continuity_upsert(watermark, &record);
            return Ok(SubmitDecision {
                outcome: SubmitOutcome::Created,
                record,
                conflict_reason: String::new(),
            });
        }

        if record.replica_node.is_empty() {
            let rejected = self.reject_pre_admission_request(
                &record.request_key,
                &record.attempt_id,
                REPLICA_REQUIRED_UNAVAILABLE,
            )?;
            return Ok(SubmitDecision {
                outcome: SubmitOutcome::Rejected,
                record: rejected,
                conflict_reason: String::new(),
            });
        }

        match prepare_replica(&record) {
            Ok(acknowledged_replica_nodes) => {
                let acked = self.acknowledge_replica_prepare_nodes(
                    &record.request_key,
                    &record.attempt_id,
                    acknowledged_replica_nodes,
                )?;
                Ok(SubmitDecision {
                    outcome: SubmitOutcome::Created,
                    record: acked,
                    conflict_reason: String::new(),
                })
            }
            Err(reason) => {
                let durable_reason = if reason.is_empty() {
                    REPLICA_PREPARE_TIMEOUT.to_string()
                } else {
                    reason
                };
                let rejected = self.reject_pre_admission_request(
                    &record.request_key,
                    &record.attempt_id,
                    &durable_reason,
                )?;
                Ok(SubmitDecision {
                    outcome: SubmitOutcome::Rejected,
                    record: rejected,
                    conflict_reason: String::new(),
                })
            }
        }
    }

    pub fn mark_completed(
        &self,
        request_key: &str,
        attempt_id: &str,
        execution_node: &str,
    ) -> Result<ContinuityRecord, String> {
        let mut inner = self.inner.write();
        let record = inner
            .requests
            .get(request_key)
            .cloned()
            .ok_or_else(|| REQUEST_KEY_NOT_FOUND.to_string())?;
        let active_attempt_id = record.attempt_id.clone();

        let next = match transition_completed_record(record, attempt_id, execution_node) {
            Ok(next) => next,
            Err(reason) => {
                log_completion_rejected(request_key, attempt_id, &active_attempt_id, &reason);
                return Err(reason);
            }
        };
        inner.requests.insert(request_key.to_string(), next.clone());
        let watermark = inner.next_attempt_token;
        drop(inner);

        log_completion(&next);
        broadcast_continuity_upsert(watermark, &next);
        Ok(next)
    }

    pub fn mirror_prepare(&self, record: ContinuityRecord) -> Result<ContinuityRecord, String> {
        record.validate()?;
        if record.replica_node.is_empty() {
            return Err(REPLICA_NODE_MISSING.to_string());
        }

        let mut inner = self.inner.write();
        let merged = match inner.requests.get(&record.request_key).cloned() {
            Some(existing) => {
                if existing.payload_hash != record.payload_hash {
                    return Err(CONTINUITY_CONFLICT_REASON.to_string());
                }
                let progress = existing
                    .promotion_epoch
                    .cmp(&record.promotion_epoch)
                    .then_with(|| {
                        match (
                            parse_attempt_token(&existing.attempt_id),
                            parse_attempt_token(&record.attempt_id),
                        ) {
                            (Some(left), Some(right)) => left.cmp(&right),
                            _ if existing.attempt_id == record.attempt_id => {
                                std::cmp::Ordering::Equal
                            }
                            _ => std::cmp::Ordering::Greater,
                        }
                    })
                    .then_with(|| existing.record_version.cmp(&record.record_version));
                match progress {
                    std::cmp::Ordering::Greater => {
                        return Err("stale_replica_prepare".to_string());
                    }
                    std::cmp::Ordering::Equal => {
                        if existing.owner_node != record.owner_node {
                            return Err("owner_node_mismatch".to_string());
                        }
                        if existing.canonical_replica_nodes() != record.canonical_replica_nodes() {
                            return Err("replica_set_mismatch".to_string());
                        }
                    }
                    std::cmp::Ordering::Less
                        if existing.attempt_id == record.attempt_id
                            && existing.owner_node != record.owner_node =>
                    {
                        return Err("owner_change_requires_new_attempt".to_string());
                    }
                    std::cmp::Ordering::Less => {}
                }
                let preferred = preferred_record(existing.clone(), record.clone());
                if preferred == existing && preferred != record {
                    return Err("stale_replica_prepare".to_string());
                }
                preferred
            }
            None => record,
        };
        update_next_attempt_token(&mut inner, &merged, None);
        inner
            .requests
            .insert(merged.request_key.clone(), merged.clone());
        drop(inner);

        log_replica_prepare(&merged);
        // Prepare is the first half of a fenced ownership/replica change. It
        // must be durable on the selected replica, but it is not authoritative
        // cluster state until the coordinator installs and broadcasts the
        // mirrored record. Rebroadcasting this provisional record can race the
        // coordinator's compare-and-swap and leave every node fenced on an
        // attempt that never committed.
        super::continuity_store::persist_replica_prepare(&merged)?;
        Ok(merged)
    }

    pub fn acknowledge_replica_prepare(
        &self,
        request_key: &str,
        attempt_id: &str,
    ) -> Result<ContinuityRecord, String> {
        let replica_nodes = self
            .record(request_key)
            .ok_or_else(|| REQUEST_KEY_NOT_FOUND.to_string())?
            .canonical_replica_nodes();
        self.acknowledge_replica_prepare_nodes(request_key, attempt_id, replica_nodes)
    }

    fn acknowledge_replica_prepare_nodes(
        &self,
        request_key: &str,
        attempt_id: &str,
        acknowledged_replica_nodes: Vec<String>,
    ) -> Result<ContinuityRecord, String> {
        let mut inner = self.inner.write();
        let record = inner
            .requests
            .get(request_key)
            .cloned()
            .ok_or_else(|| REQUEST_KEY_NOT_FOUND.to_string())?;
        let next = transition_replica_ack_record(record, attempt_id, acknowledged_replica_nodes)?;
        next.validate()?;
        inner.requests.insert(request_key.to_string(), next.clone());
        let watermark = inner.next_attempt_token;
        drop(inner);

        log_replica_ack(&next);
        broadcast_continuity_upsert(watermark, &next);
        Ok(next)
    }

    pub(crate) fn acknowledge_replica_node(
        &self,
        request_key: &str,
        attempt_id: &str,
        replica_node: &str,
    ) -> Result<ContinuityRecord, String> {
        let record = self
            .record(request_key)
            .ok_or_else(|| REQUEST_KEY_NOT_FOUND.to_string())?;
        let mut acknowledgements = record.acknowledged_replica_nodes.clone();
        if !acknowledgements.iter().any(|node| node == replica_node) {
            acknowledgements.push(replica_node.to_string());
        }
        self.acknowledge_replica_prepare_nodes(request_key, attempt_id, acknowledgements)
    }

    pub fn reject_durable_request(
        &self,
        request_key: &str,
        attempt_id: &str,
        reason: &str,
    ) -> Result<ContinuityRecord, String> {
        self.reject_request(request_key, attempt_id, reason, ReplicaStatus::Rejected)
    }

    fn reject_pre_admission_request(
        &self,
        request_key: &str,
        attempt_id: &str,
        reason: &str,
    ) -> Result<ContinuityRecord, String> {
        self.reject_request(
            request_key,
            attempt_id,
            reason,
            ReplicaStatus::PreAdmissionRejected,
        )
    }

    fn reject_request(
        &self,
        request_key: &str,
        attempt_id: &str,
        reason: &str,
        replica_status: ReplicaStatus,
    ) -> Result<ContinuityRecord, String> {
        let mut inner = self.inner.write();
        let record = inner
            .requests
            .get(request_key)
            .cloned()
            .ok_or_else(|| REQUEST_KEY_NOT_FOUND.to_string())?;
        // Owner loss is already a stronger fence than a late transport or
        // handler failure from that attempt. Keep it recoverable so the
        // replacement owner (or the next same-key submission) can roll the
        // attempt forward instead of turning every retry into a permanent 503.
        if record.attempt_id == attempt_id && record.replica_status == ReplicaStatus::OwnerLost {
            return Ok(record);
        }
        let next = transition_rejected_record(record, attempt_id, reason, replica_status)?;
        inner.requests.insert(request_key.to_string(), next.clone());
        let watermark = inner.next_attempt_token;
        drop(inner);

        log_rejection(&next, reason);
        broadcast_continuity_upsert(watermark, &next);
        Ok(next)
    }

    pub fn mark_owner_loss_records_for_node_loss(&self, owner_node: &str) -> Vec<ContinuityRecord> {
        let mut inner = self.inner.write();
        let watermark = inner.next_attempt_token;
        let request_keys: Vec<String> = inner.requests.keys().cloned().collect();
        let mut owner_lost_records = Vec::new();

        for request_key in &request_keys {
            let Some(record) = inner.requests.get(request_key).cloned() else {
                continue;
            };
            let Some(owner_lost) = transition_owner_lost_record(record, owner_node) else {
                continue;
            };
            inner
                .requests
                .insert(request_key.clone(), owner_lost.clone());
            owner_lost_records.push(owner_lost);
        }
        drop(inner);

        for record in &owner_lost_records {
            log_owner_lost(record, owner_node);
            broadcast_continuity_upsert(watermark, record);
        }

        owner_lost_records
    }

    /// Mark only the currently executing request as owner-lost.
    ///
    /// A request timeout is not proof that the whole node disappeared: the
    /// owner may merely be slow or its application queue may be saturated.
    /// Node-disconnect handling uses `mark_owner_loss_records_for_node_loss`,
    /// while request-scoped transport failures use this fenced transition so
    /// unrelated work on the same owner is never replayed speculatively.
    pub fn mark_owner_loss_for_request(
        &self,
        request_key: &str,
        attempt_id: &str,
        owner_node: &str,
    ) -> Result<Option<ContinuityRecord>, String> {
        let mut inner = self.inner.write();
        let Some(record) = inner.requests.get(request_key).cloned() else {
            return Err(REQUEST_KEY_NOT_FOUND.to_string());
        };
        if record.attempt_id != attempt_id || record.owner_node != owner_node {
            return Ok(None);
        }
        let Some(owner_lost) = transition_owner_lost_record(record, owner_node) else {
            return Ok(None);
        };
        inner
            .requests
            .insert(request_key.to_string(), owner_lost.clone());
        let watermark = inner.next_attempt_token;
        drop(inner);

        log_owner_lost(&owner_lost, owner_node);
        broadcast_continuity_upsert(watermark, &owner_lost);
        Ok(Some(owner_lost))
    }

    pub fn degrade_replica_records_for_node_loss(
        &self,
        replica_node: &str,
    ) -> Vec<ContinuityRecord> {
        let mut inner = self.inner.write();
        let watermark = inner.next_attempt_token;
        let request_keys: Vec<String> = inner.requests.keys().cloned().collect();
        let mut degraded_records = Vec::new();

        for request_key in &request_keys {
            let Some(record) = inner.requests.get(request_key).cloned() else {
                continue;
            };
            let Some(degraded) = transition_degraded_record(record, replica_node) else {
                continue;
            };
            inner.requests.insert(request_key.clone(), degraded.clone());
            degraded_records.push(degraded);
        }
        drop(inner);

        for record in &degraded_records {
            log_degraded(record, replica_node);
            broadcast_continuity_upsert(watermark, record);
        }

        degraded_records
    }

    pub fn degrade_replication_health_for_node_loss(
        &self,
        node_name: &str,
    ) -> Vec<ContinuityRecord> {
        let mut inner = self.inner.write();
        let watermark = inner.next_attempt_token;
        let request_keys: Vec<String> = inner.requests.keys().cloned().collect();
        let mut degraded_records = Vec::new();

        for request_key in &request_keys {
            let Some(record) = inner.requests.get(request_key).cloned() else {
                continue;
            };
            let Some(degraded) = transition_replication_health_record(record, node_name) else {
                continue;
            };
            inner.requests.insert(request_key.clone(), degraded.clone());
            degraded_records.push(degraded);
        }
        drop(inner);

        for record in &degraded_records {
            log_replication_degraded(record, node_name);
            broadcast_continuity_upsert(watermark, record);
        }

        degraded_records
    }

    pub fn merge_remote_record(
        &self,
        next_attempt_token: u64,
        record: ContinuityRecord,
    ) -> Result<(), String> {
        record.validate()?;
        let incoming_authority = ContinuityAuthorityConfig::from_record(&record);
        let mut inner = self.inner.write();
        update_next_attempt_token(&mut inner, &record, Some(next_attempt_token));

        if let Some(next_authority) = observe_remote_authority(inner.authority, incoming_authority)
        {
            let previous = inner.authority;
            inner.authority = next_authority;
            reproject_records_for_authority_change(&mut inner.requests, previous, next_authority);
            log_authority_fenced(
                previous,
                next_authority,
                &record.request_key,
                &record.attempt_id,
            );
        }

        if incoming_authority.promotion_epoch < inner.authority.promotion_epoch {
            log_stale_epoch_rejected(&record, inner.authority);
            return Err(STALE_PROMOTION_EPOCH_REJECTED.to_string());
        }

        let projected = project_remote_record(record, inner.authority)?;
        if parse_attempt_token(&projected.attempt_id).is_none() {
            return Ok(());
        }
        let merged = match inner.requests.get(&projected.request_key).cloned() {
            Some(existing) => {
                if existing.payload_hash != projected.payload_hash {
                    return Ok(());
                }
                preferred_record(existing, projected)
            }
            None => projected,
        };
        inner
            .requests
            .insert(merged.request_key.clone(), merged.clone());
        let watermark = inner.next_attempt_token;
        drop(inner);

        // Remote merges are authoritative state changes too. Keeping only the
        // in-memory registry current leaves the node-local durable store with
        // an active predecessor, which can survive restart and falsely block
        // scale-down as an only-copy responsibility.
        super::continuity_store::persist_runtime_record(watermark, &merged);
        Ok(())
    }

    pub fn merge_snapshot(&self, snapshot: ContinuitySnapshot) -> Result<(), String> {
        let mut inner = self.inner.write();
        if inner.next_attempt_token < snapshot.next_attempt_token {
            inner.next_attempt_token = snapshot.next_attempt_token;
        }
        let mut merged_records = Vec::new();
        for record in snapshot.records {
            record.validate()?;
            let incoming_authority = ContinuityAuthorityConfig::from_record(&record);
            if let Some(next_authority) =
                observe_remote_authority(inner.authority, incoming_authority)
            {
                let previous = inner.authority;
                inner.authority = next_authority;
                reproject_records_for_authority_change(
                    &mut inner.requests,
                    previous,
                    next_authority,
                );
                log_authority_fenced(
                    previous,
                    next_authority,
                    &record.request_key,
                    &record.attempt_id,
                );
            }
            update_next_attempt_token(&mut inner, &record, None);
            if incoming_authority.promotion_epoch < inner.authority.promotion_epoch {
                log_stale_epoch_rejected(&record, inner.authority);
                continue;
            }
            let projected = project_remote_record(record, inner.authority)?;
            if parse_attempt_token(&projected.attempt_id).is_none() {
                continue;
            }
            let merged = match inner.requests.get(&projected.request_key).cloned() {
                Some(existing) => {
                    if existing.payload_hash != projected.payload_hash {
                        continue;
                    }
                    preferred_record(existing, projected)
                }
                None => projected,
            };
            inner
                .requests
                .insert(merged.request_key.clone(), merged.clone());
            merged_records.push(merged);
        }
        let watermark = inner.next_attempt_token;
        drop(inner);
        for record in &merged_records {
            super::continuity_store::persist_runtime_record(watermark, record);
        }
        Ok(())
    }
}

impl Default for ContinuityRegistry {
    fn default() -> Self {
        Self::new()
    }
}

static CONTINUITY_REGISTRY: OnceLock<ContinuityRegistry> = OnceLock::new();

fn init_continuity_registry() -> ContinuityRegistry {
    let authority = parse_authority_config(
        std::env::var(CONTINUITY_ROLE_ENV).ok().as_deref(),
        std::env::var(CONTINUITY_PROMOTION_EPOCH_ENV)
            .ok()
            .as_deref(),
    )
    .unwrap_or_default();
    ContinuityRegistry::new_with_authority(authority)
}

fn current_authority_config() -> ContinuityAuthorityConfig {
    continuity_registry().authority()
}

pub fn continuity_registry() -> &'static ContinuityRegistry {
    CONTINUITY_REGISTRY.get_or_init(init_continuity_registry)
}

pub fn attempt_id_from_token(token: u64) -> String {
    format!("attempt-{}", token)
}

fn parse_attempt_token(attempt_id: &str) -> Option<u64> {
    attempt_id.strip_prefix("attempt-")?.parse().ok()
}

fn initial_replica_status(replica_node: &str) -> ReplicaStatus {
    if replica_node.is_empty() {
        ReplicaStatus::Unassigned
    } else {
        ReplicaStatus::Preparing
    }
}

fn initial_replication_health() -> ReplicationHealth {
    ReplicationHealth::LocalOnly
}

fn continuity_submitted_record(request: &SubmitRequest, attempt_token: u64) -> ContinuityRecord {
    let replica_nodes = if request.replica_nodes.is_empty() {
        if request.replica_node.is_empty() {
            Vec::new()
        } else {
            vec![request.replica_node.clone()]
        }
    } else {
        request.replica_nodes.clone()
    };
    let replica_node = if request.replica_node.is_empty() {
        replica_nodes.first().cloned().unwrap_or_default()
    } else {
        request.replica_node.clone()
    };
    ContinuityRecord {
        request_key: request.request_key.clone(),
        payload_hash: request.payload_hash.clone(),
        record_version: 1,
        request_payload: request.request_payload.clone(),
        attempt_id: attempt_id_from_token(attempt_token),
        phase: ContinuityPhase::Submitted,
        result: ContinuityResult::Pending,
        ingress_node: request.ingress_node.clone(),
        owner_node: request.owner_node.clone(),
        replica_nodes,
        acknowledged_replica_nodes: Vec::new(),
        replica_node: replica_node.clone(),
        replication_count: request.replication_count,
        replica_status: initial_replica_status(&replica_node),
        cluster_role: request.cluster_role,
        promotion_epoch: request.promotion_epoch,
        replication_health: initial_replication_health(),
        execution_node: String::new(),
        routed_remotely: request.routed_remotely,
        fell_back_locally: request.fell_back_locally,
        error: String::new(),
        declared_handler_runtime_name: request.declared_handler_runtime_name.clone(),
    }
}

fn replica_status_rank(status: ReplicaStatus) -> u8 {
    match status {
        ReplicaStatus::Unassigned => 0,
        ReplicaStatus::Preparing => 1,
        ReplicaStatus::Mirrored => 2,
        ReplicaStatus::OwnerLost => 3,
        ReplicaStatus::DegradedContinuing => 4,
        ReplicaStatus::PreAdmissionRejected => 5,
        ReplicaStatus::Rejected => 6,
    }
}

fn replication_health_rank(health: ReplicationHealth) -> u8 {
    match health {
        ReplicationHealth::LocalOnly => 0,
        ReplicationHealth::Unavailable => 1,
        ReplicationHealth::Degraded => 2,
        ReplicationHealth::Healthy => 3,
    }
}

fn authority_replication_health<'a>(
    records: impl Iterator<Item = &'a ContinuityRecord>,
) -> ReplicationHealth {
    let mut saw_degraded = false;
    let mut saw_healthy = false;

    for record in records {
        match record.replication_health {
            ReplicationHealth::Unavailable => return ReplicationHealth::Unavailable,
            ReplicationHealth::Degraded => saw_degraded = true,
            ReplicationHealth::Healthy => saw_healthy = true,
            ReplicationHealth::LocalOnly => {}
        }
    }

    if saw_degraded {
        ReplicationHealth::Degraded
    } else if saw_healthy {
        ReplicationHealth::Healthy
    } else {
        ReplicationHealth::LocalOnly
    }
}

fn observe_remote_authority(
    local: ContinuityAuthorityConfig,
    incoming: ContinuityAuthorityConfig,
) -> Option<ContinuityAuthorityConfig> {
    if incoming.promotion_epoch > local.promotion_epoch {
        return Some(local.follower_for_epoch(incoming.promotion_epoch));
    }
    None
}

fn project_remote_record(
    mut record: ContinuityRecord,
    authority: ContinuityAuthorityConfig,
) -> Result<ContinuityRecord, String> {
    let role_changed = record.cluster_role != authority.cluster_role;
    record.cluster_role = authority.cluster_role;
    record.promotion_epoch = authority.promotion_epoch;
    if role_changed {
        record.replication_health = ReplicationHealth::Healthy;
    }
    record.validate()?;
    Ok(record)
}

fn project_record_for_authority_change(
    mut record: ContinuityRecord,
    previous: ContinuityAuthorityConfig,
    next: ContinuityAuthorityConfig,
) -> ContinuityRecord {
    let was_pending =
        record.phase == ContinuityPhase::Submitted && record.result == ContinuityResult::Pending;
    let moving_to_primary = previous.cluster_role != ContinuityClusterRole::Primary
        && next.cluster_role == ContinuityClusterRole::Primary;

    record.cluster_role = next.cluster_role;
    record.promotion_epoch = next.promotion_epoch;
    record.record_version = record.record_version.saturating_add(1);

    if moving_to_primary {
        if was_pending && !record.replica_node.is_empty() {
            record.replica_status = ReplicaStatus::OwnerLost;
            record.replication_health = ReplicationHealth::Unavailable;
            record.error = format!("owner_lost:{}", record.owner_node);
        } else if record.replica_node.is_empty() {
            record.replication_health = ReplicationHealth::LocalOnly;
        } else if record.replication_health == ReplicationHealth::Healthy {
            record.replication_health = ReplicationHealth::Unavailable;
        }
    }

    if next.cluster_role == ContinuityClusterRole::Standby {
        record.replication_health = ReplicationHealth::Healthy;
        if record.replica_status == ReplicaStatus::OwnerLost {
            record.replica_status = ReplicaStatus::Mirrored;
            if record.error.starts_with("owner_lost:") {
                record.error.clear();
            }
        }
    }

    debug_assert!(record.validate().is_ok());
    record
}

fn reproject_records_for_authority_change(
    requests: &mut FxHashMap<String, ContinuityRecord>,
    previous: ContinuityAuthorityConfig,
    next: ContinuityAuthorityConfig,
) {
    for record in requests.values_mut() {
        let projected = project_record_for_authority_change(record.clone(), previous, next);
        *record = projected;
    }
}

fn preferred_record(existing: ContinuityRecord, incoming: ContinuityRecord) -> ContinuityRecord {
    if incoming.promotion_epoch < existing.promotion_epoch {
        return existing;
    }
    if incoming.promotion_epoch > existing.promotion_epoch {
        return incoming;
    }

    let existing_attempt = parse_attempt_token(&existing.attempt_id);
    let incoming_attempt = parse_attempt_token(&incoming.attempt_id);

    // A higher attempt in Preparing is only a replica-local provisional write.
    // It cannot erase a terminal result from the previously authoritative
    // attempt. Once the replacement commits it is Mirrored (or explicitly
    // degraded), and normal attempt ordering fences the predecessor.
    if existing_attempt != incoming_attempt {
        if existing.phase.is_terminal() && incoming.replica_status == ReplicaStatus::Preparing {
            return existing;
        }
        if incoming.phase.is_terminal() && existing.replica_status == ReplicaStatus::Preparing {
            return incoming;
        }
    }
    match (existing_attempt, incoming_attempt) {
        (Some(left), Some(right)) if right < left => return existing,
        (Some(left), Some(right)) if right > left => return incoming,
        (Some(_), None) => return existing,
        (None, Some(_)) => return incoming,
        _ => {}
    }

    if existing.phase.is_terminal() && !incoming.phase.is_terminal() {
        return existing;
    }
    if !existing.phase.is_terminal() && incoming.phase.is_terminal() {
        return incoming;
    }

    if incoming.record_version < existing.record_version {
        return existing;
    }
    if incoming.record_version > existing.record_version {
        return incoming;
    }

    // A repair grows the acknowledged set without allocating a new attempt.
    // Prefer that progress over the otherwise higher-ranked degraded status,
    // while still allowing a node-loss transition to shrink the set and win
    // through the normal status ranking below.
    if existing.canonical_replica_nodes() == incoming.canonical_replica_nodes() {
        match incoming
            .acknowledged_replica_nodes
            .len()
            .cmp(&existing.acknowledged_replica_nodes.len())
        {
            std::cmp::Ordering::Greater => return incoming,
            std::cmp::Ordering::Less
                if existing.replica_status == ReplicaStatus::Mirrored
                    && incoming.replica_status == ReplicaStatus::DegradedContinuing =>
            {
                return incoming;
            }
            std::cmp::Ordering::Less => return existing,
            std::cmp::Ordering::Equal => {}
        }
    }

    let existing_rank = replica_status_rank(existing.replica_status);
    let incoming_rank = replica_status_rank(incoming.replica_status);
    if existing_rank > incoming_rank {
        return existing;
    }
    if incoming_rank > existing_rank {
        return incoming;
    }

    let existing_health = replication_health_rank(existing.replication_health);
    let incoming_health = replication_health_rank(incoming.replication_health);
    if existing_health > incoming_health {
        return existing;
    }
    if incoming_health > existing_health {
        return incoming;
    }

    incoming
}

fn update_next_attempt_token(
    inner: &mut ContinuityInner,
    record: &ContinuityRecord,
    watermark: Option<u64>,
) {
    if let Some(watermark) = watermark {
        inner.next_attempt_token = inner.next_attempt_token.max(watermark);
    }
    if let Some(token) = parse_attempt_token(&record.attempt_id) {
        inner.next_attempt_token = inner.next_attempt_token.max(token + 1);
    }
}

fn transition_retry_rollover_record(
    existing: &ContinuityRecord,
    request: &SubmitRequest,
    attempt_token: u64,
) -> Result<ContinuityRecord, String> {
    if existing.request_key != request.request_key || existing.payload_hash != request.payload_hash
    {
        return Err(CONTINUITY_CONFLICT_REASON.to_string());
    }
    let owner_loss_retry = existing.phase == ContinuityPhase::Submitted
        && existing.result == ContinuityResult::Pending;
    let pre_admission_retry = existing.phase == ContinuityPhase::Rejected
        && existing.result == ContinuityResult::Rejected
        && existing.replica_status == ReplicaStatus::PreAdmissionRejected;
    if !owner_loss_retry && !pre_admission_retry {
        return Err(TRANSITION_REJECTED_PHASE.to_string());
    }

    let mut recovery_request = request.clone();
    if recovery_request.replication_count == 0 {
        recovery_request.replication_count = existing.replication_count;
    }
    if recovery_request.declared_handler_runtime_name.is_empty() {
        recovery_request.declared_handler_runtime_name =
            existing.declared_handler_runtime_name.clone();
    }
    if recovery_request.request_payload.is_empty() {
        recovery_request.request_payload = existing.request_payload.clone();
    }

    let mut recovered = continuity_submitted_record(&recovery_request, attempt_token);
    recovered.record_version = existing.record_version.saturating_add(1);
    Ok(recovered)
}

fn transition_completed_record(
    record: ContinuityRecord,
    attempt_id: &str,
    execution_node: &str,
) -> Result<ContinuityRecord, String> {
    if record.attempt_id != attempt_id {
        return Err(ATTEMPT_ID_MISMATCH.to_string());
    }
    if execution_node.is_empty() {
        return Err(EXECUTION_NODE_MISSING.to_string());
    }
    if record.phase == ContinuityPhase::Completed {
        if record.execution_node == execution_node {
            return Ok(record);
        }
        return Err(TRANSITION_REJECTED_ALREADY_COMPLETED.to_string());
    }
    if record.phase != ContinuityPhase::Submitted {
        return Err(TRANSITION_REJECTED_PHASE.to_string());
    }

    let record_version = record.record_version.saturating_add(1);
    Ok(ContinuityRecord {
        record_version,
        phase: ContinuityPhase::Completed,
        result: ContinuityResult::Succeeded,
        execution_node: execution_node.to_string(),
        error: String::new(),
        ..record
    })
}

fn transition_replica_ack_record(
    record: ContinuityRecord,
    attempt_id: &str,
    acknowledged_replica_nodes: Vec<String>,
) -> Result<ContinuityRecord, String> {
    if record.attempt_id != attempt_id {
        return Err(ATTEMPT_ID_MISMATCH.to_string());
    }
    if record.replica_node.is_empty()
        || record.phase == ContinuityPhase::Rejected
        || record.replica_status == ReplicaStatus::OwnerLost
    {
        return Ok(record);
    }

    let mut acknowledged_replica_nodes = acknowledged_replica_nodes;
    acknowledged_replica_nodes.sort();
    acknowledged_replica_nodes.dedup();
    let required_acknowledgements = (record.replication_count / 2) as usize;
    let reached_threshold = acknowledged_replica_nodes.len() >= required_acknowledgements;
    if !reached_threshold && !super::continuity_store::degraded_durability_enabled() {
        return Err("continuity_replica_ack_threshold_unmet".to_string());
    }
    let record_version = record.record_version.saturating_add(1);
    Ok(ContinuityRecord {
        record_version,
        acknowledged_replica_nodes,
        replica_status: if reached_threshold {
            ReplicaStatus::Mirrored
        } else {
            ReplicaStatus::DegradedContinuing
        },
        replication_health: if reached_threshold {
            ReplicationHealth::Healthy
        } else {
            ReplicationHealth::Degraded
        },
        error: if reached_threshold {
            String::new()
        } else {
            "continuity_replica_ack_threshold_degraded".to_string()
        },
        ..record
    })
}

fn transition_rejected_record(
    record: ContinuityRecord,
    attempt_id: &str,
    reason: &str,
    replica_status: ReplicaStatus,
) -> Result<ContinuityRecord, String> {
    if record.attempt_id != attempt_id {
        return Err(ATTEMPT_ID_MISMATCH.to_string());
    }
    if record.phase == ContinuityPhase::Completed {
        return Err(TRANSITION_REJECTED_ALREADY_COMPLETED.to_string());
    }
    if record.phase == ContinuityPhase::Rejected {
        return Ok(record);
    }

    let record_version = record.record_version.saturating_add(1);
    Ok(ContinuityRecord {
        record_version,
        phase: ContinuityPhase::Rejected,
        result: ContinuityResult::Rejected,
        replica_status,
        replication_health: ReplicationHealth::Unavailable,
        error: reason.to_string(),
        ..record
    })
}

fn transition_owner_lost_record(
    record: ContinuityRecord,
    owner_node: &str,
) -> Option<ContinuityRecord> {
    if record.cluster_role != ContinuityClusterRole::Primary
        || record.owner_node != owner_node
        || record.phase != ContinuityPhase::Submitted
        || record.result != ContinuityResult::Pending
        || !matches!(
            record.replica_status,
            ReplicaStatus::Preparing | ReplicaStatus::Mirrored
        )
        || record.acknowledged_replica_nodes.is_empty()
    {
        return None;
    }

    let record_version = record.record_version.saturating_add(1);
    Some(ContinuityRecord {
        record_version,
        replica_status: ReplicaStatus::OwnerLost,
        replication_health: ReplicationHealth::Unavailable,
        error: format!("owner_lost:{owner_node}"),
        ..record
    })
}

fn transition_degraded_record(
    record: ContinuityRecord,
    replica_node: &str,
) -> Option<ContinuityRecord> {
    if record.cluster_role != ContinuityClusterRole::Primary
        || !record
            .acknowledged_replica_nodes
            .iter()
            .any(|node| node == replica_node)
        || record.phase != ContinuityPhase::Submitted
        || record.result != ContinuityResult::Pending
        || record.replica_status != ReplicaStatus::Mirrored
    {
        return None;
    }

    let mut acknowledged_replica_nodes = record.acknowledged_replica_nodes.clone();
    acknowledged_replica_nodes.retain(|node| node != replica_node);
    let primary_replica = acknowledged_replica_nodes
        .first()
        .cloned()
        .unwrap_or_default();
    let record_version = record.record_version.saturating_add(1);
    Some(ContinuityRecord {
        record_version,
        acknowledged_replica_nodes,
        replica_node: primary_replica,
        replica_status: ReplicaStatus::DegradedContinuing,
        replication_health: ReplicationHealth::Degraded,
        error: format!("replica_lost:{replica_node}"),
        ..record
    })
}

fn transition_replication_health_record(
    record: ContinuityRecord,
    node_name: &str,
) -> Option<ContinuityRecord> {
    if record.cluster_role != ContinuityClusterRole::Standby
        || record.phase != ContinuityPhase::Submitted
        || record.result != ContinuityResult::Pending
        || record.replication_health == ReplicationHealth::Unavailable
        || (record.owner_node != node_name
            && !record
                .acknowledged_replica_nodes
                .iter()
                .any(|node| node == node_name))
    {
        return None;
    }

    let record_version = record.record_version.saturating_add(1);
    Some(ContinuityRecord {
        record_version,
        replication_health: ReplicationHealth::Degraded,
        error: format!("replication_source_lost:{node_name}"),
        ..record
    })
}

fn continuity_diagnostic(
    transition: &str,
    record: &ContinuityRecord,
) -> crate::dist::operator::OperatorDiagnosticRecord {
    let mut metadata = vec![
        ("phase".to_string(), record.phase.as_str().to_string()),
        ("result".to_string(), record.result.as_str().to_string()),
        (
            "replication_count".to_string(),
            record.replication_count.to_string(),
        ),
    ];
    if !record.declared_handler_runtime_name.is_empty() {
        metadata.push((
            "declared_handler_runtime_name".to_string(),
            record.declared_handler_runtime_name.clone(),
        ));
    }

    crate::dist::operator::OperatorDiagnosticRecord {
        transition: transition.to_string(),
        request_key: Some(request_key_fingerprint(&record.request_key)),
        attempt_id: Some(record.attempt_id.clone()),
        owner_node: Some(record.owner_node.clone()),
        replica_node: Some(record.replica_node.clone()),
        execution_node: if record.execution_node.is_empty() {
            None
        } else {
            Some(record.execution_node.clone())
        },
        cluster_role: Some(record.cluster_role.as_str().to_string()),
        promotion_epoch: Some(record.promotion_epoch),
        replication_health: Some(record.replication_health.as_str().to_string()),
        replica_status: Some(record.replica_status.as_str().to_string()),
        reason: if record.error.is_empty() {
            None
        } else {
            Some(record.error.clone())
        },
        metadata,
    }
}

fn log_submit(record: &ContinuityRecord, required_replica_count: u64) {
    let mut diagnostic = continuity_diagnostic("submit", record);
    diagnostic.metadata.push((
        "required_replicas".to_string(),
        required_replica_count.to_string(),
    ));
    crate::dist::operator::record_diagnostic(diagnostic);
    eprintln!(
        "[mesh-rt continuity] transition=submit request_key={} attempt_id={} ingress={} owner={} replica={} replication_count={} required_replicas={} cluster_role={} promotion_epoch={} replication_health={} replica_status={} phase={}",
        request_key_fingerprint(&record.request_key),
        record.attempt_id,
        record.ingress_node,
        record.owner_node,
        record.replica_node,
        record.replication_count,
        required_replica_count,
        record.cluster_role.as_str(),
        record.promotion_epoch,
        record.replication_health.as_str(),
        record.replica_status.as_str(),
        record.phase.as_str(),
    );
}

fn log_drain_replacement(record: &ContinuityRecord, previous_attempt_id: &str) {
    let mut diagnostic = continuity_diagnostic("drain_replacement", record);
    diagnostic.metadata.push((
        "previous_attempt_id".to_string(),
        previous_attempt_id.to_string(),
    ));
    diagnostic.metadata.push((
        "replica_set".to_string(),
        record.canonical_replica_nodes().join(","),
    ));
    crate::dist::operator::record_diagnostic(diagnostic);
    eprintln!(
        "[mesh-rt continuity] transition=drain_replacement request_key={} previous_attempt_id={} next_attempt_id={} owner={} replicas={}",
        request_key_fingerprint(&record.request_key),
        previous_attempt_id,
        record.attempt_id,
        record.owner_node,
        record.canonical_replica_nodes().join(","),
    );
}

fn log_recovery_rollover(previous: &ContinuityRecord, next: &ContinuityRecord) {
    let mut diagnostic = continuity_diagnostic("recovery_rollover", next);
    diagnostic.metadata.extend([
        (
            "previous_attempt_id".to_string(),
            previous.attempt_id.clone(),
        ),
        ("previous_owner".to_string(), previous.owner_node.clone()),
        ("next_owner".to_string(), next.owner_node.clone()),
        ("next_replica".to_string(), next.replica_node.clone()),
    ]);
    crate::dist::operator::record_diagnostic(diagnostic);
    eprintln!(
        "[mesh-rt continuity] transition=recovery_rollover request_key={} previous_attempt_id={} next_attempt_id={} previous_owner={} next_owner={} next_replica={} cluster_role={} promotion_epoch={} replication_health={} next_replica_status={} phase={}",
        request_key_fingerprint(&next.request_key),
        previous.attempt_id,
        next.attempt_id,
        previous.owner_node,
        next.owner_node,
        next.replica_node,
        next.cluster_role.as_str(),
        next.promotion_epoch,
        next.replication_health.as_str(),
        next.replica_status.as_str(),
        next.phase.as_str(),
    );
}

fn log_duplicate(record: &ContinuityRecord) {
    crate::dist::operator::record_diagnostic(continuity_diagnostic("duplicate", record));
    eprintln!(
        "[mesh-rt continuity] transition=duplicate request_key={} attempt_id={} phase={} result={} owner={} replica={} cluster_role={} promotion_epoch={} replication_health={}",
        request_key_fingerprint(&record.request_key),
        record.attempt_id,
        record.phase.as_str(),
        record.result.as_str(),
        record.owner_node,
        record.replica_node,
        record.cluster_role.as_str(),
        record.promotion_epoch,
        record.replication_health.as_str(),
    );
}

fn log_conflict(record: &ContinuityRecord, request_key: &str, reason: &str) {
    let mut diagnostic = continuity_diagnostic("conflict", record);
    diagnostic.request_key = Some(request_key_fingerprint(request_key));
    diagnostic.reason = Some(reason.to_string());
    crate::dist::operator::record_diagnostic(diagnostic);
    eprintln!(
        "[mesh-rt continuity] transition=conflict request_key={} stored_attempt_id={} stored_phase={} stored_result={} cluster_role={} promotion_epoch={} replication_health={} reason={}",
        request_key_fingerprint(request_key),
        record.attempt_id,
        record.phase.as_str(),
        record.result.as_str(),
        record.cluster_role.as_str(),
        record.promotion_epoch,
        record.replication_health.as_str(),
        reason,
    );
}

fn log_completion(record: &ContinuityRecord) {
    crate::dist::operator::record_diagnostic(continuity_diagnostic("completed", record));
    eprintln!(
        "[mesh-rt continuity] transition=completed request_key={} attempt_id={} execution={} owner={} replica={} cluster_role={} promotion_epoch={} replication_health={} replica_status={}",
        request_key_fingerprint(&record.request_key),
        record.attempt_id,
        record.execution_node,
        record.owner_node,
        record.replica_node,
        record.cluster_role.as_str(),
        record.promotion_epoch,
        record.replication_health.as_str(),
        record.replica_status.as_str(),
    );
}

fn log_completion_rejected(
    request_key: &str,
    attempt_id: &str,
    active_attempt_id: &str,
    reason: &str,
) {
    crate::dist::operator::record_diagnostic(crate::dist::operator::OperatorDiagnosticRecord {
        transition: "completion_rejected".to_string(),
        request_key: Some(request_key_fingerprint(request_key)),
        attempt_id: Some(attempt_id.to_string()),
        reason: Some(reason.to_string()),
        metadata: vec![(
            "active_attempt_id".to_string(),
            active_attempt_id.to_string(),
        )],
        ..crate::dist::operator::OperatorDiagnosticRecord::default()
    });
    eprintln!(
        "[mesh-rt continuity] transition=completion_rejected request_key={} attempt_id={} active_attempt_id={} reason={}",
        request_key_fingerprint(request_key),
        attempt_id,
        active_attempt_id,
        reason,
    );
}

fn log_replica_prepare(record: &ContinuityRecord) {
    crate::dist::operator::record_diagnostic(continuity_diagnostic("replica_prepare", record));
    eprintln!(
        "[mesh-rt continuity] transition=replica_prepare request_key={} attempt_id={} owner={} replica={} cluster_role={} promotion_epoch={} replication_health={} replica_status={}",
        request_key_fingerprint(&record.request_key),
        record.attempt_id,
        record.owner_node,
        record.replica_node,
        record.cluster_role.as_str(),
        record.promotion_epoch,
        record.replication_health.as_str(),
        record.replica_status.as_str(),
    );
}

fn log_replica_ack(record: &ContinuityRecord) {
    crate::dist::operator::record_diagnostic(continuity_diagnostic("replica_ack", record));
    eprintln!(
        "[mesh-rt continuity] transition=replica_ack request_key={} attempt_id={} owner={} replica={} cluster_role={} promotion_epoch={} replication_health={} replica_status={}",
        request_key_fingerprint(&record.request_key),
        record.attempt_id,
        record.owner_node,
        record.replica_node,
        record.cluster_role.as_str(),
        record.promotion_epoch,
        record.replication_health.as_str(),
        record.replica_status.as_str(),
    );
}

fn log_rejection(record: &ContinuityRecord, reason: &str) {
    let mut diagnostic = continuity_diagnostic("rejected", record);
    diagnostic.reason = Some(reason.to_string());
    crate::dist::operator::record_diagnostic(diagnostic);
    eprintln!(
        "[mesh-rt continuity] transition=rejected request_key={} attempt_id={} owner={} replica={} cluster_role={} promotion_epoch={} replication_health={} replica_status={} reason={}",
        request_key_fingerprint(&record.request_key),
        record.attempt_id,
        record.owner_node,
        record.replica_node,
        record.cluster_role.as_str(),
        record.promotion_epoch,
        record.replication_health.as_str(),
        record.replica_status.as_str(),
        reason,
    );
}

fn log_owner_lost(record: &ContinuityRecord, owner_node: &str) {
    let mut diagnostic = continuity_diagnostic("owner_lost", record);
    diagnostic.reason = Some(format!("owner_lost:{owner_node}"));
    crate::dist::operator::record_diagnostic(diagnostic);
    eprintln!(
        "[mesh-rt continuity] transition=owner_lost request_key={} attempt_id={} owner={} replica={} cluster_role={} promotion_epoch={} replication_health={} replica_status={} reason=owner_lost:{}",
        request_key_fingerprint(&record.request_key),
        record.attempt_id,
        record.owner_node,
        record.replica_node,
        record.cluster_role.as_str(),
        record.promotion_epoch,
        record.replication_health.as_str(),
        record.replica_status.as_str(),
        owner_node,
    );
}

fn log_degraded(record: &ContinuityRecord, replica_node: &str) {
    let mut diagnostic = continuity_diagnostic("degraded", record);
    diagnostic.reason = Some(format!("replica_lost:{replica_node}"));
    crate::dist::operator::record_diagnostic(diagnostic);
    eprintln!(
        "[mesh-rt continuity] transition=degraded request_key={} attempt_id={} owner={} replica={} cluster_role={} promotion_epoch={} replication_health={} replica_status={} reason=replica_lost:{}",
        request_key_fingerprint(&record.request_key),
        record.attempt_id,
        record.owner_node,
        record.replica_node,
        record.cluster_role.as_str(),
        record.promotion_epoch,
        record.replication_health.as_str(),
        record.replica_status.as_str(),
        replica_node,
    );
}

fn log_replication_degraded(record: &ContinuityRecord, node_name: &str) {
    let mut diagnostic = continuity_diagnostic("replication_degraded", record);
    diagnostic.reason = Some(format!("replication_source_lost:{node_name}"));
    crate::dist::operator::record_diagnostic(diagnostic);
    eprintln!(
        "[mesh-rt continuity] transition=replication_degraded request_key={} attempt_id={} owner={} replica={} cluster_role={} promotion_epoch={} replication_health={} replica_status={} reason=replication_source_lost:{}",
        request_key_fingerprint(&record.request_key),
        record.attempt_id,
        record.owner_node,
        record.replica_node,
        record.cluster_role.as_str(),
        record.promotion_epoch,
        record.replication_health.as_str(),
        record.replica_status.as_str(),
        node_name,
    );
}

#[cfg_attr(not(test), allow(dead_code))]
fn log_promotion(previous: ContinuityAuthorityConfig, next: ContinuityAuthorityConfig) {
    crate::dist::operator::record_diagnostic(crate::dist::operator::OperatorDiagnosticRecord {
        transition: "promote".to_string(),
        cluster_role: Some(next.cluster_role.as_str().to_string()),
        promotion_epoch: Some(next.promotion_epoch),
        metadata: vec![
            (
                "previous_role".to_string(),
                previous.cluster_role.as_str().to_string(),
            ),
            (
                "previous_epoch".to_string(),
                previous.promotion_epoch.to_string(),
            ),
        ],
        ..crate::dist::operator::OperatorDiagnosticRecord::default()
    });
    eprintln!(
        "[mesh-rt continuity] transition=promote previous_role={} previous_epoch={} next_role={} next_epoch={}",
        previous.cluster_role.as_str(),
        previous.promotion_epoch,
        next.cluster_role.as_str(),
        next.promotion_epoch,
    );
}

fn log_authority_fenced(
    previous: ContinuityAuthorityConfig,
    next: ContinuityAuthorityConfig,
    request_key: &str,
    attempt_id: &str,
) {
    crate::dist::operator::record_diagnostic(crate::dist::operator::OperatorDiagnosticRecord {
        transition: "fenced_rejoin".to_string(),
        request_key: Some(request_key_fingerprint(request_key)),
        attempt_id: Some(attempt_id.to_string()),
        cluster_role: Some(next.cluster_role.as_str().to_string()),
        promotion_epoch: Some(next.promotion_epoch),
        metadata: vec![
            (
                "previous_role".to_string(),
                previous.cluster_role.as_str().to_string(),
            ),
            (
                "previous_epoch".to_string(),
                previous.promotion_epoch.to_string(),
            ),
        ],
        ..crate::dist::operator::OperatorDiagnosticRecord::default()
    });
    eprintln!(
        "[mesh-rt continuity] transition=fenced_rejoin request_key={} attempt_id={} previous_role={} previous_epoch={} next_role={} next_epoch={}",
        request_key_fingerprint(request_key),
        attempt_id,
        previous.cluster_role.as_str(),
        previous.promotion_epoch,
        next.cluster_role.as_str(),
        next.promotion_epoch,
    );
}

fn log_stale_epoch_rejected(record: &ContinuityRecord, authority: ContinuityAuthorityConfig) {
    let mut diagnostic = continuity_diagnostic("stale_epoch_rejected", record);
    diagnostic.reason = Some(STALE_PROMOTION_EPOCH_REJECTED.to_string());
    diagnostic.metadata.extend([
        (
            "incoming_role".to_string(),
            record.cluster_role.as_str().to_string(),
        ),
        (
            "incoming_epoch".to_string(),
            record.promotion_epoch.to_string(),
        ),
        (
            "local_role".to_string(),
            authority.cluster_role.as_str().to_string(),
        ),
        (
            "local_epoch".to_string(),
            authority.promotion_epoch.to_string(),
        ),
    ]);
    crate::dist::operator::record_diagnostic(diagnostic);
    eprintln!(
        "[mesh-rt continuity] transition=stale_epoch_rejected request_key={} attempt_id={} incoming_role={} incoming_epoch={} local_role={} local_epoch={} replica_status={} phase={}",
        request_key_fingerprint(&record.request_key),
        record.attempt_id,
        record.cluster_role.as_str(),
        record.promotion_epoch,
        authority.cluster_role.as_str(),
        authority.promotion_epoch,
        record.replica_status.as_str(),
        record.phase.as_str(),
    );
}

pub(crate) fn broadcast_continuity_upsert(next_attempt_token: u64, record: &ContinuityRecord) {
    super::continuity_store::persist_runtime_record(next_attempt_token, record);
    let state = match super::node::node_state() {
        Some(s) => s,
        None => return,
    };

    let payload = match encode_upsert_payload(next_attempt_token, record) {
        Ok(payload) => payload,
        Err(_) => return,
    };

    let sessions: Vec<Arc<super::node::NodeSession>> = {
        let map = state.sessions.read();
        map.values().map(Arc::clone).collect()
    };

    for session in &sessions {
        let participates = session.remote_has_role("controller")
            || session.remote_name == record.ingress_node
            || session.remote_name == record.owner_node
            || record
                .replica_nodes()
                .iter()
                .any(|replica| replica == &session.remote_name);
        if participates {
            // The active owner must observe its fenced attempt before a
            // reservation/query for that attempt can overtake it. Both sends
            // originate only after this function returns, so placing the
            // owner's state fence on the same FIFO control lane establishes
            // the required transport order. Other participants retain the
            // high-throughput continuity lane.
            let class = if session.remote_name == record.owner_node
                && record.phase == ContinuityPhase::Submitted
            {
                super::node::OutboundClass::Control
            } else {
                super::node::OutboundClass::Continuity
            };
            let _ = session.send(class, payload.clone());
        }
    }
}

fn send_continuity_upsert_to_node(
    next_attempt_token: u64,
    record: &ContinuityRecord,
    target: &str,
) {
    let Some(state) = super::node::node_state() else {
        return;
    };
    let Some(session) = state.sessions.read().get(target).cloned() else {
        return;
    };
    let Ok(payload) = encode_upsert_payload(next_attempt_token, record) else {
        return;
    };
    let _ = session.send(super::node::OutboundClass::Continuity, payload);
}

pub(crate) fn send_continuity_sync(session: &Arc<super::node::NodeSession>) {
    let snapshot = continuity_registry().snapshot();
    if snapshot.records.is_empty() && snapshot.next_attempt_token == 0 {
        send_durable_store_sync(session);
        return;
    }

    if session
        .negotiated_protocol
        .capabilities
        .contains(super::protocol::Capabilities::CHUNKED_SNAPSHOTS)
    {
        for record in &snapshot.records {
            if let Ok(payload) = encode_upsert_payload(snapshot.next_attempt_token, record) {
                let _ = session.send(super::node::OutboundClass::Snapshot, payload);
            }
        }
        send_durable_store_sync(session);
    } else if let Ok(payload) = encode_sync_payload(&snapshot) {
        let _ = session.send(super::node::OutboundClass::Snapshot, payload);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoreSnapshotAck {
    snapshot_id: String,
    next_sequence: u32,
    high_water_mark: u64,
    complete: bool,
}

#[derive(Debug)]
struct IncomingStoreSnapshot {
    next_sequence: u32,
    high_water_mark: u64,
    snapshot_checksum: [u8; 32],
    chunk_checksums: Vec<[u8; 32]>,
}

static INCOMING_STORE_SNAPSHOTS: OnceLock<
    Mutex<BTreeMap<(String, String), IncomingStoreSnapshot>>,
> = OnceLock::new();
static OUTGOING_STORE_SNAPSHOT_ACKS: OnceLock<Mutex<BTreeMap<(String, String), u32>>> =
    OnceLock::new();

fn incoming_store_snapshots() -> &'static Mutex<BTreeMap<(String, String), IncomingStoreSnapshot>> {
    INCOMING_STORE_SNAPSHOTS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn outgoing_store_snapshot_acks() -> &'static Mutex<BTreeMap<(String, String), u32>> {
    OUTGOING_STORE_SNAPSHOT_ACKS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn encode_tagged_json<T: Serialize>(tag: u8, value: &T) -> Result<Vec<u8>, String> {
    let encoded = serde_json::to_vec(value)
        .map_err(|error| format!("continuity_sync_encode_failed:{error}"))?;
    let mut frame = Vec::with_capacity(1 + encoded.len());
    frame.push(tag);
    frame.extend_from_slice(&encoded);
    Ok(frame)
}

fn decode_tagged_json<T: for<'de> Deserialize<'de>>(
    expected_tag: u8,
    frame: &[u8],
) -> Result<T, String> {
    if frame.first().copied() != Some(expected_tag) {
        return Err("continuity_sync_tag_invalid".to_string());
    }
    serde_json::from_slice(&frame[1..])
        .map_err(|error| format!("continuity_sync_decode_failed:{error}"))
}

fn send_durable_store_sync(session: &Arc<super::node::NodeSession>) {
    if !session
        .negotiated_protocol
        .capabilities
        .contains(super::protocol::Capabilities::CHUNKED_SNAPSHOTS)
    {
        return;
    }
    let Some(store) = configured_continuity_store() else {
        return;
    };
    let configured = super::continuity_store::runtime_snapshot_chunk_bytes();
    let chunk_bytes = configured
        .max(128)
        .min((session.negotiated_protocol.max_frame_bytes as usize / 6).max(128));
    let Ok(chunks) = store.snapshot_chunks(chunk_bytes) else {
        return;
    };
    let Some(first) = chunks.first() else {
        return;
    };
    let resume_at = outgoing_store_snapshot_acks()
        .lock()
        .unwrap()
        .get(&(session.remote_name.clone(), first.snapshot_id.clone()))
        .copied()
        .unwrap_or(0);
    let high_water_mark = first.high_water_mark;
    for chunk in chunks
        .into_iter()
        .filter(|chunk| chunk.sequence >= resume_at)
    {
        let Ok(frame) = encode_tagged_json(super::node::DIST_CONTINUITY_STORE_SNAPSHOT, &chunk)
        else {
            return;
        };
        if session
            .send(super::node::OutboundClass::Snapshot, frame)
            .is_err()
        {
            return;
        }
    }
    let mut cursor = high_water_mark;
    loop {
        let Ok(entries) = store.log_entries_after(cursor, 256) else {
            return;
        };
        if entries.is_empty() {
            break;
        }
        for entry in &entries {
            let Ok(frame) = encode_tagged_json(super::node::DIST_CONTINUITY_STORE_LOG_ENTRY, entry)
            else {
                return;
            };
            if session
                .send(super::node::OutboundClass::Snapshot, frame)
                .is_err()
            {
                return;
            }
            cursor = entry.sequence;
        }
        if entries.len() < 256 {
            break;
        }
    }
}

pub(crate) fn handle_store_snapshot_chunk(
    session: &Arc<super::node::NodeSession>,
    frame: &[u8],
) -> Result<(), String> {
    let chunk: SnapshotChunk =
        decode_tagged_json(super::node::DIST_CONTINUITY_STORE_SNAPSHOT, frame)?;
    if !chunk.verify() {
        return Err("continuity_snapshot_checksum_mismatch".to_string());
    }
    let key = (session.remote_name.clone(), chunk.snapshot_id.clone());
    let mut incoming = incoming_store_snapshots().lock().unwrap();
    let state = incoming
        .entry(key.clone())
        .or_insert_with(|| IncomingStoreSnapshot {
            next_sequence: 0,
            high_water_mark: chunk.high_water_mark,
            snapshot_checksum: chunk.snapshot_checksum,
            chunk_checksums: Vec::new(),
        });
    if state.high_water_mark != chunk.high_water_mark
        || state.snapshot_checksum != chunk.snapshot_checksum
    {
        incoming.remove(&key);
        return Err("continuity_snapshot_identity_changed".to_string());
    }
    if chunk.sequence > state.next_sequence {
        return Err(format!(
            "continuity_snapshot_chunk_gap:expected={}:actual={}",
            state.next_sequence, chunk.sequence
        ));
    }
    if chunk.sequence == state.next_sequence {
        let store = configured_continuity_store()
            .ok_or_else(|| "continuity_store_not_configured".to_string())?;
        store.apply_snapshot_chunk(&chunk)?;
        state.chunk_checksums.push(chunk.checksum);
        state.next_sequence = state
            .next_sequence
            .checked_add(1)
            .ok_or_else(|| "continuity_snapshot_sequence_exhausted".to_string())?;
    }
    let mut complete = false;
    if chunk.final_chunk && chunk.sequence + 1 == state.next_sequence {
        let mut hasher = Sha256::new();
        for checksum in &state.chunk_checksums {
            hasher.update(checksum);
        }
        if <[u8; 32]>::from(hasher.finalize()) != state.snapshot_checksum {
            incoming.remove(&key);
            return Err("continuity_snapshot_final_checksum_mismatch".to_string());
        }
        complete = true;
    }
    let ack = StoreSnapshotAck {
        snapshot_id: chunk.snapshot_id,
        next_sequence: state.next_sequence,
        high_water_mark: state.high_water_mark,
        complete,
    };
    if complete {
        incoming.remove(&key);
    }
    drop(incoming);
    let payload = encode_tagged_json(super::node::DIST_CONTINUITY_STORE_SNAPSHOT_ACK, &ack)?;
    session.send(super::node::OutboundClass::Control, payload)?;
    if complete {
        crate::dist::readiness::mark_initial_state_synchronized();
    }
    Ok(())
}

pub(crate) fn handle_store_snapshot_ack(
    session: &Arc<super::node::NodeSession>,
    frame: &[u8],
) -> Result<(), String> {
    let ack: StoreSnapshotAck =
        decode_tagged_json(super::node::DIST_CONTINUITY_STORE_SNAPSHOT_ACK, frame)?;
    if ack.snapshot_id.is_empty() {
        return Err("continuity_snapshot_ack_id_missing".to_string());
    }
    outgoing_store_snapshot_acks().lock().unwrap().insert(
        (session.remote_name.clone(), ack.snapshot_id),
        ack.next_sequence,
    );
    if ack.complete {
        if let Some(store) = configured_continuity_store() {
            store.acknowledge_replica_safe_point(&session.remote_name, ack.high_water_mark)?;
            let _ = store.compact_log_to_replica_safe_point()?;
        }
    }
    Ok(())
}

pub(crate) fn handle_store_log_entry(
    session: &Arc<super::node::NodeSession>,
    frame: &[u8],
) -> Result<(), String> {
    let entry: ContinuityLogEntry =
        decode_tagged_json(super::node::DIST_CONTINUITY_STORE_LOG_ENTRY, frame)?;
    configured_continuity_store()
        .ok_or_else(|| "continuity_store_not_configured".to_string())?
        .apply_log_entry(&entry)?;
    let ack = StoreSnapshotAck {
        snapshot_id: "incremental-log".to_string(),
        next_sequence: 0,
        high_water_mark: entry.sequence,
        complete: true,
    };
    let frame = encode_tagged_json(super::node::DIST_CONTINUITY_STORE_SNAPSHOT_ACK, &ack)?;
    session.send(super::node::OutboundClass::Control, frame)
}

pub(crate) fn encode_upsert_payload(
    next_attempt_token: u64,
    record: &ContinuityRecord,
) -> Result<Vec<u8>, String> {
    let encoded = encode_record(record)?;
    let mut payload = Vec::with_capacity(1 + 8 + 4 + encoded.len());
    payload.push(super::node::DIST_CONTINUITY_UPSERT);
    payload.extend_from_slice(&next_attempt_token.to_le_bytes());
    payload.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
    payload.extend_from_slice(&encoded);
    Ok(payload)
}

pub(crate) fn decode_upsert_payload(data: &[u8]) -> Result<(u64, ContinuityRecord), String> {
    if data.len() < 13 {
        return Err("continuity upsert payload too short".to_string());
    }
    let next_attempt_token = u64::from_le_bytes(data[1..9].try_into().unwrap());
    let record_len = u32::from_le_bytes(data[9..13].try_into().unwrap()) as usize;
    if data.len() != 13 + record_len {
        return Err("continuity upsert payload length mismatch".to_string());
    }
    let record = decode_record(&data[13..])?;
    Ok((next_attempt_token, record))
}

pub(crate) fn encode_sync_payload(snapshot: &ContinuitySnapshot) -> Result<Vec<u8>, String> {
    let mut payload = Vec::new();
    payload.push(super::node::DIST_CONTINUITY_SYNC);
    payload.extend_from_slice(&snapshot.next_attempt_token.to_le_bytes());
    payload.extend_from_slice(&(snapshot.records.len() as u32).to_le_bytes());
    for record in &snapshot.records {
        let encoded = encode_record(record)?;
        payload.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
        payload.extend_from_slice(&encoded);
    }
    Ok(payload)
}

pub(crate) fn decode_sync_payload(data: &[u8]) -> Result<ContinuitySnapshot, String> {
    if data.len() < 13 {
        return Err("continuity sync payload too short".to_string());
    }
    let next_attempt_token = u64::from_le_bytes(data[1..9].try_into().unwrap());
    let count = u32::from_le_bytes(data[9..13].try_into().unwrap()) as usize;
    let mut pos = 13;
    let mut records = Vec::with_capacity(count);
    for _ in 0..count {
        if pos + 4 > data.len() {
            return Err("continuity sync payload truncated".to_string());
        }
        let record_len = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        if pos + record_len > data.len() {
            return Err("continuity sync record payload truncated".to_string());
        }
        records.push(decode_record(&data[pos..pos + record_len])?);
        pos += record_len;
    }
    if pos != data.len() {
        return Err("continuity sync payload had trailing bytes".to_string());
    }
    Ok(ContinuitySnapshot {
        next_attempt_token,
        records,
    })
}

fn encode_record(record: &ContinuityRecord) -> Result<Vec<u8>, String> {
    record.validate()?;
    let mut out = Vec::new();
    put_string(&mut out, &record.request_key)?;
    put_string(&mut out, &record.payload_hash)?;
    put_string(&mut out, &record.attempt_id)?;
    out.push(record.phase.to_wire());
    out.push(record.result.to_wire());
    put_string(&mut out, &record.ingress_node)?;
    put_string(&mut out, &record.owner_node)?;
    put_string(&mut out, &record.replica_node)?;
    out.extend_from_slice(&record.replication_count.to_le_bytes());
    out.push(record.replica_status.to_wire());
    out.push(record.cluster_role.to_wire());
    out.extend_from_slice(&record.promotion_epoch.to_le_bytes());
    out.push(record.replication_health.to_wire());
    put_string(&mut out, &record.execution_node)?;
    out.push(record.routed_remotely as u8);
    out.push(record.fell_back_locally as u8);
    put_string(&mut out, &record.error)?;
    put_string(&mut out, &record.declared_handler_runtime_name)?;
    out.extend_from_slice(b"RVRS");
    out.extend_from_slice(&record.record_version.to_le_bytes());
    let replica_nodes = record.canonical_replica_nodes();
    if replica_nodes.len() > 1 {
        out.extend_from_slice(b"RSET");
        let count: u16 = replica_nodes
            .len()
            .try_into()
            .map_err(|_| "continuity replica set too large".to_string())?;
        out.extend_from_slice(&count.to_le_bytes());
        for replica in replica_nodes {
            put_string(&mut out, &replica)?;
        }
    }
    if !record.acknowledged_replica_nodes.is_empty() {
        out.extend_from_slice(b"RACK");
        let count: u16 = record
            .acknowledged_replica_nodes
            .len()
            .try_into()
            .map_err(|_| "continuity replica acknowledgement set too large".to_string())?;
        out.extend_from_slice(&count.to_le_bytes());
        for replica in &record.acknowledged_replica_nodes {
            put_string(&mut out, replica)?;
        }
    }
    if !record.request_payload.is_empty() {
        out.extend_from_slice(b"RPAY");
        let length: u32 = record
            .request_payload
            .len()
            .try_into()
            .map_err(|_| "continuity request payload too large".to_string())?;
        out.extend_from_slice(&length.to_le_bytes());
        out.extend_from_slice(&record.request_payload);
    }
    Ok(out)
}

fn decode_record(data: &[u8]) -> Result<ContinuityRecord, String> {
    let mut pos = 0;
    let request_key = take_string(data, &mut pos)?;
    let payload_hash = take_string(data, &mut pos)?;
    let attempt_id = take_string(data, &mut pos)?;
    let phase = ContinuityPhase::from_wire(take_u8(data, &mut pos)?)?;
    let result = ContinuityResult::from_wire(take_u8(data, &mut pos)?)?;
    let ingress_node = take_string(data, &mut pos)?;
    let owner_node = take_string(data, &mut pos)?;
    let replica_node = take_string(data, &mut pos)?;
    let replication_count = take_u64(data, &mut pos)?;
    let replica_status = ReplicaStatus::from_wire(take_u8(data, &mut pos)?)?;
    let cluster_role = ContinuityClusterRole::from_wire(take_u8(data, &mut pos)?)?;
    let promotion_epoch = take_u64(data, &mut pos)?;
    let replication_health = ReplicationHealth::from_wire(take_u8(data, &mut pos)?)?;
    let execution_node = take_string(data, &mut pos)?;
    let routed_remotely = take_u8(data, &mut pos)? != 0;
    let fell_back_locally = take_u8(data, &mut pos)? != 0;
    let error = take_string(data, &mut pos)?;
    let declared_handler_runtime_name = take_string(data, &mut pos)?;
    let mut replica_nodes = if replica_node.is_empty() {
        Vec::new()
    } else {
        vec![replica_node.clone()]
    };
    let mut acknowledged_replica_nodes = Vec::new();
    let mut request_payload = Vec::new();
    let mut record_version = 1;
    while pos < data.len() {
        let tag = data
            .get(pos..pos + 4)
            .ok_or_else(|| "continuity record extension truncated".to_string())?;
        pos += 4;
        match tag {
            b"RSET" => {
                if pos + 2 > data.len() {
                    return Err("continuity replica set truncated".to_string());
                }
                let count = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap()) as usize;
                pos += 2;
                let mut replicas = Vec::with_capacity(count);
                for _ in 0..count {
                    replicas.push(take_string(data, &mut pos)?);
                }
                replica_nodes = replicas;
            }
            b"RPAY" => {
                if pos + 4 > data.len() {
                    return Err("continuity request payload length truncated".to_string());
                }
                let length = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
                pos += 4;
                let end = pos
                    .checked_add(length)
                    .ok_or_else(|| "continuity request payload length overflow".to_string())?;
                request_payload = data
                    .get(pos..end)
                    .ok_or_else(|| "continuity request payload truncated".to_string())?
                    .to_vec();
                pos = end;
            }
            b"RACK" => {
                if pos + 2 > data.len() {
                    return Err("continuity replica acknowledgement set truncated".to_string());
                }
                let count = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap()) as usize;
                pos += 2;
                acknowledged_replica_nodes = Vec::with_capacity(count);
                for _ in 0..count {
                    acknowledged_replica_nodes.push(take_string(data, &mut pos)?);
                }
            }
            b"RVRS" => {
                record_version = take_u64(data, &mut pos)?;
            }
            _ => return Err("continuity record extension invalid".to_string()),
        }
    }
    if acknowledged_replica_nodes.is_empty() && replica_status == ReplicaStatus::Mirrored {
        acknowledged_replica_nodes = replica_nodes.clone();
    }
    let record = ContinuityRecord {
        request_key,
        payload_hash,
        record_version,
        request_payload,
        attempt_id,
        phase,
        result,
        ingress_node,
        owner_node,
        replica_nodes,
        acknowledged_replica_nodes,
        replica_node,
        replication_count,
        replica_status,
        cluster_role,
        promotion_epoch,
        replication_health,
        execution_node,
        routed_remotely,
        fell_back_locally,
        error,
        declared_handler_runtime_name,
    };
    record.validate()?;
    Ok(record)
}

pub(crate) fn encode_record_payload(record: &ContinuityRecord) -> Result<Vec<u8>, String> {
    encode_record(record)
}

pub(crate) fn decode_record_payload(data: &[u8]) -> Result<ContinuityRecord, String> {
    decode_record(data)
}

/// Rebuilds the in-memory registry from this node's durable journal before it
/// joins routing or control-plane service.
pub(crate) fn hydrate_runtime_continuity_from_store() -> Result<usize, String> {
    let records = super::continuity_store::load_runtime_records()?;
    hydrate_runtime_records(continuity_registry(), records)
}

fn hydrate_runtime_records(
    registry: &ContinuityRegistry,
    records: Vec<Vec<u8>>,
) -> Result<usize, String> {
    let mut hydrated = 0_usize;
    for encoded in records {
        let record = decode_record(&encoded)?;
        registry.merge_remote_record(0, record)?;
        hydrated = hydrated.saturating_add(1);
    }
    Ok(hydrated)
}

fn put_string(out: &mut Vec<u8>, value: &str) -> Result<(), String> {
    let bytes = value.as_bytes();
    let len: u16 = bytes
        .len()
        .try_into()
        .map_err(|_| format!("continuity string too large: {}", bytes.len()))?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

fn take_string(data: &[u8], pos: &mut usize) -> Result<String, String> {
    if *pos + 2 > data.len() {
        return Err("continuity string length truncated".to_string());
    }
    let len = u16::from_le_bytes(data[*pos..*pos + 2].try_into().unwrap()) as usize;
    *pos += 2;
    if *pos + len > data.len() {
        return Err("continuity string bytes truncated".to_string());
    }
    let value = std::str::from_utf8(&data[*pos..*pos + len])
        .map_err(|e| e.to_string())?
        .to_string();
    *pos += len;
    Ok(value)
}

fn take_u8(data: &[u8], pos: &mut usize) -> Result<u8, String> {
    if *pos >= data.len() {
        return Err("continuity payload truncated".to_string());
    }
    let value = data[*pos];
    *pos += 1;
    Ok(value)
}

fn take_u64(data: &[u8], pos: &mut usize) -> Result<u64, String> {
    if *pos + 8 > data.len() {
        return Err("continuity u64 truncated".to_string());
    }
    let value = u64::from_le_bytes(data[*pos..*pos + 8].try_into().unwrap());
    *pos += 8;
    Ok(value)
}

#[repr(C)]
pub struct MeshContinuityAuthorityStatus {
    pub cluster_role: *mut MeshString,
    pub promotion_epoch: i64,
    pub replication_health: *mut MeshString,
}

#[repr(C)]
pub struct MeshContinuityRecord {
    pub request_key: *mut MeshString,
    pub payload_hash: *mut MeshString,
    pub attempt_id: *mut MeshString,
    pub phase: *mut MeshString,
    pub result: *mut MeshString,
    pub ingress_node: *mut MeshString,
    pub owner_node: *mut MeshString,
    pub replica_node: *mut MeshString,
    pub replication_count: i64,
    pub replica_status: *mut MeshString,
    pub cluster_role: *mut MeshString,
    pub promotion_epoch: i64,
    pub replication_health: *mut MeshString,
    pub execution_node: *mut MeshString,
    pub routed_remotely: bool,
    pub fell_back_locally: bool,
    pub error: *mut MeshString,
}

#[repr(C)]
pub struct MeshContinuitySubmitDecision {
    pub outcome: *mut MeshString,
    pub conflict_reason: *mut MeshString,
    pub record: MeshContinuityRecord,
}

fn alloc_mesh_value<T>(value: T) -> *mut T {
    unsafe {
        let ptr = mesh_gc_alloc_actor(
            std::mem::size_of::<T>() as u64,
            std::mem::align_of::<T>() as u64,
        ) as *mut T;
        ptr.write(value);
        ptr
    }
}

fn mesh_string_ptr(value: &str) -> *mut MeshString {
    mesh_string_new(value.as_ptr(), value.len() as u64)
}

fn mesh_int_from_u64(value: u64) -> i64 {
    if value > i64::MAX as u64 {
        i64::MAX
    } else {
        value as i64
    }
}

fn mesh_authority_status(status: ContinuityAuthorityStatus) -> MeshContinuityAuthorityStatus {
    MeshContinuityAuthorityStatus {
        cluster_role: mesh_string_ptr(status.cluster_role.as_str()),
        promotion_epoch: mesh_int_from_u64(status.promotion_epoch),
        replication_health: mesh_string_ptr(status.replication_health.as_str()),
    }
}

fn mesh_record(record: &ContinuityRecord) -> MeshContinuityRecord {
    MeshContinuityRecord {
        request_key: mesh_string_ptr(&record.request_key),
        payload_hash: mesh_string_ptr(&record.payload_hash),
        attempt_id: mesh_string_ptr(&record.attempt_id),
        phase: mesh_string_ptr(record.phase.as_str()),
        result: mesh_string_ptr(record.result.as_str()),
        ingress_node: mesh_string_ptr(&record.ingress_node),
        owner_node: mesh_string_ptr(&record.owner_node),
        replica_node: mesh_string_ptr(&record.replica_node),
        replication_count: mesh_int_from_u64(record.replication_count),
        replica_status: mesh_string_ptr(record.replica_status.as_str()),
        cluster_role: mesh_string_ptr(record.cluster_role.as_str()),
        promotion_epoch: mesh_int_from_u64(record.promotion_epoch),
        replication_health: mesh_string_ptr(record.replication_health.as_str()),
        execution_node: mesh_string_ptr(&record.execution_node),
        routed_remotely: record.routed_remotely,
        fell_back_locally: record.fell_back_locally,
        error: mesh_string_ptr(&record.error),
    }
}

fn mesh_submit_decision(decision: &SubmitDecision) -> MeshContinuitySubmitDecision {
    MeshContinuitySubmitDecision {
        outcome: mesh_string_ptr(decision.outcome.as_str()),
        conflict_reason: mesh_string_ptr(&decision.conflict_reason),
        record: mesh_record(&decision.record),
    }
}

fn continuity_ok_authority_status(status: ContinuityAuthorityStatus) -> *mut MeshResult {
    alloc_result(
        0,
        alloc_mesh_value(mesh_authority_status(status)) as *mut u8,
    )
}

fn continuity_ok_record(record: &ContinuityRecord) -> *mut MeshResult {
    alloc_result(0, alloc_mesh_value(mesh_record(record)) as *mut u8)
}

fn continuity_ok_submit_decision(decision: &SubmitDecision) -> *mut MeshResult {
    alloc_result(
        0,
        alloc_mesh_value(mesh_submit_decision(decision)) as *mut u8,
    )
}

fn continuity_err_string(reason: &str) -> *mut MeshResult {
    alloc_result(
        1,
        mesh_string_new(reason.as_ptr(), reason.len() as u64) as *mut u8,
    )
}

fn mesh_string_to_owned(value: *const MeshString) -> String {
    unsafe { (*value).as_str().to_string() }
}

fn continuity_submit_impl(request: SubmitRequest) -> *mut MeshResult {
    match continuity_registry().submit(request) {
        Ok(decision) => continuity_ok_submit_decision(&decision),
        Err(reason) => continuity_err_string(&reason),
    }
}

#[no_mangle]
pub extern "C" fn mesh_continuity_submit_with_durability(
    request_key: *const MeshString,
    payload_hash: *const MeshString,
    ingress_node: *const MeshString,
    owner_node: *const MeshString,
    replica_node: *const MeshString,
    required_replica_count: u64,
    routed_remotely: i8,
    fell_back_locally: i8,
) -> *mut MeshResult {
    let authority = current_authority_config();
    let request = SubmitRequest {
        request_key: mesh_string_to_owned(request_key),
        payload_hash: mesh_string_to_owned(payload_hash),
        request_payload: Vec::new(),
        ingress_node: mesh_string_to_owned(ingress_node),
        owner_node: mesh_string_to_owned(owner_node),
        replica_nodes: Vec::new(),
        replica_node: mesh_string_to_owned(replica_node),
        replication_count: required_replica_count.saturating_add(1),
        required_replica_count,
        routed_remotely: routed_remotely != 0,
        fell_back_locally: fell_back_locally != 0,
        cluster_role: authority.cluster_role,
        promotion_epoch: authority.promotion_epoch,
        declared_handler_runtime_name: String::new(),
    };

    continuity_submit_impl(request)
}

#[no_mangle]
pub extern "C" fn mesh_continuity_submit_declared_work(
    runtime_name: *const MeshString,
    request_key: *const MeshString,
    payload_hash: *const MeshString,
    required_replica_count: i64,
) -> *mut MeshResult {
    let runtime_name = mesh_string_to_owned(runtime_name);
    let request_key = mesh_string_to_owned(request_key);
    let payload_hash = mesh_string_to_owned(payload_hash);
    if required_replica_count < 0 {
        return continuity_err_string(INVALID_REQUIRED_REPLICA_COUNT);
    }
    let required_replica_count =
        match super::node::required_replica_count_for_runtime_name(&runtime_name) {
            Ok(value) => value,
            Err(reason) => return continuity_err_string(&reason),
        };
    match super::node::submit_declared_work(
        &runtime_name,
        &request_key,
        &payload_hash,
        required_replica_count,
    ) {
        Ok(decision) => continuity_ok_submit_decision(&decision),
        Err(reason) => continuity_err_string(&reason),
    }
}

#[no_mangle]
pub extern "C" fn mesh_continuity_submit(
    request_key: *const MeshString,
    payload_hash: *const MeshString,
    ingress_node: *const MeshString,
    owner_node: *const MeshString,
    replica_node: *const MeshString,
    routed_remotely: i8,
    fell_back_locally: i8,
) -> *mut MeshResult {
    mesh_continuity_submit_with_durability(
        request_key,
        payload_hash,
        ingress_node,
        owner_node,
        replica_node,
        0,
        routed_remotely,
        fell_back_locally,
    )
}

#[no_mangle]
pub extern "C" fn mesh_continuity_status(request_key: *const MeshString) -> *mut MeshResult {
    let request_key = mesh_string_to_owned(request_key);
    match continuity_registry().record(&request_key) {
        Some(record) => continuity_ok_record(&record),
        None => continuity_err_string(REQUEST_KEY_NOT_FOUND),
    }
}

#[no_mangle]
pub extern "C" fn mesh_continuity_authority_status() -> *mut MeshResult {
    continuity_ok_authority_status(continuity_registry().authority_status())
}

#[no_mangle]
pub extern "C" fn mesh_continuity_mark_completed(
    request_key: *const MeshString,
    attempt_id: *const MeshString,
    execution_node: *const MeshString,
) -> *mut MeshResult {
    let request_key = mesh_string_to_owned(request_key);
    let attempt_id = mesh_string_to_owned(attempt_id);
    let execution_node = mesh_string_to_owned(execution_node);
    match continuity_registry().mark_completed(&request_key, &attempt_id, &execution_node) {
        Ok(record) => continuity_ok_record(&record),
        Err(reason) => continuity_err_string(&reason),
    }
}

#[no_mangle]
pub extern "C" fn mesh_continuity_complete_declared_work(
    request_key: *const MeshString,
    attempt_id: *const MeshString,
) -> *mut MeshResult {
    let request_key = mesh_string_to_owned(request_key);
    let attempt_id = mesh_string_to_owned(attempt_id);
    match super::node::complete_declared_work(&request_key, &attempt_id) {
        Ok(record) => continuity_ok_record(&record),
        Err(reason) => continuity_err_string(&reason),
    }
}

#[no_mangle]
pub extern "C" fn mesh_continuity_acknowledge_replica(
    request_key: *const MeshString,
    attempt_id: *const MeshString,
) -> *mut MeshResult {
    let request_key = mesh_string_to_owned(request_key);
    let attempt_id = mesh_string_to_owned(attempt_id);
    match continuity_registry().acknowledge_replica_prepare(&request_key, &attempt_id) {
        Ok(record) => continuity_ok_record(&record),
        Err(reason) => continuity_err_string(&reason),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collections::map;
    use crate::gc::mesh_gc_alloc_actor;
    use crate::http::server::{
        decode_http_response_payload, encode_http_request_payload,
        invoke_route_handler_from_payload, mesh_http_response_new, MeshHttpRequest,
        MeshHttpResponse,
    };
    use std::sync::atomic::{AtomicU64, Ordering};

    fn continuity_fresh_registry() -> ContinuityRegistry {
        ContinuityRegistry::new()
    }

    static ROUTE_BOUNDARY_HANDLER_CALLS: AtomicU64 = AtomicU64::new(0);

    extern "C" fn route_boundary_handler(request: *mut u8) -> *mut u8 {
        ROUTE_BOUNDARY_HANDLER_CALLS.fetch_add(1, Ordering::Relaxed);
        let body_ptr = crate::http::server::mesh_http_request_body(request);
        let body = unsafe { (*(body_ptr as *const MeshString)).as_str().to_string() };
        let response_body = format!("handled:{body}");
        let body_ptr = mesh_string_new(response_body.as_ptr(), response_body.len() as u64);
        mesh_http_response_new(200, body_ptr)
    }

    fn route_request_payload(body: &str) -> Vec<u8> {
        unsafe {
            let request_ptr = mesh_gc_alloc_actor(
                std::mem::size_of::<MeshHttpRequest>() as u64,
                std::mem::align_of::<MeshHttpRequest>() as u64,
            ) as *mut MeshHttpRequest;
            (*request_ptr).method = mesh_string_new(b"POST".as_ptr(), 4) as *mut u8;
            (*request_ptr).path = mesh_string_new(b"/todos".as_ptr(), 6) as *mut u8;
            (*request_ptr).body = mesh_string_new(body.as_ptr(), body.len() as u64) as *mut u8;
            (*request_ptr).query_params = map::mesh_map_new_typed(1);
            (*request_ptr).headers = map::mesh_map_new_typed(1);
            (*request_ptr).path_params = map::mesh_map_new_typed(1);
            encode_http_request_payload(request_ptr as *mut u8).expect("encode route request")
        }
    }

    fn continuity_registry_with_authority(
        authority: ContinuityAuthorityConfig,
    ) -> ContinuityRegistry {
        ContinuityRegistry::new_with_authority(authority)
    }

    fn continuity_submit_request(
        request_key: &str,
        payload_hash: &str,
        replica_node: &str,
        required_replica_count: u64,
    ) -> SubmitRequest {
        continuity_submit_request_fixture(SubmitRequestFixture {
            request_key,
            payload_hash,
            owner_node: "owner@host",
            replica_node,
            required_replica_count,
            replication_count: required_replica_count.saturating_add(1),
            cluster_role: ContinuityClusterRole::Primary,
            promotion_epoch: 0,
        })
    }

    fn continuity_submit_request_with_owner(
        request_key: &str,
        payload_hash: &str,
        owner_node: &str,
        replica_node: &str,
        required_replica_count: u64,
    ) -> SubmitRequest {
        continuity_submit_request_fixture(SubmitRequestFixture {
            request_key,
            payload_hash,
            owner_node,
            replica_node,
            required_replica_count,
            replication_count: required_replica_count.saturating_add(1),
            cluster_role: ContinuityClusterRole::Primary,
            promotion_epoch: 0,
        })
    }

    fn continuity_submit_request_with_authority(
        request_key: &str,
        payload_hash: &str,
        owner_node: &str,
        replica_node: &str,
        required_replica_count: u64,
        cluster_role: ContinuityClusterRole,
        promotion_epoch: u64,
    ) -> SubmitRequest {
        continuity_submit_request_fixture(SubmitRequestFixture {
            request_key,
            payload_hash,
            owner_node,
            replica_node,
            required_replica_count,
            replication_count: required_replica_count.saturating_add(1),
            cluster_role,
            promotion_epoch,
        })
    }

    struct SubmitRequestFixture<'a> {
        request_key: &'a str,
        payload_hash: &'a str,
        owner_node: &'a str,
        replica_node: &'a str,
        required_replica_count: u64,
        replication_count: u64,
        cluster_role: ContinuityClusterRole,
        promotion_epoch: u64,
    }

    fn continuity_submit_request_fixture(fixture: SubmitRequestFixture<'_>) -> SubmitRequest {
        let SubmitRequestFixture {
            request_key,
            payload_hash,
            owner_node,
            replica_node,
            required_replica_count,
            replication_count,
            cluster_role,
            promotion_epoch,
        } = fixture;
        SubmitRequest {
            request_key: request_key.to_string(),
            payload_hash: payload_hash.to_string(),
            request_payload: Vec::new(),
            ingress_node: "ingress@host".to_string(),
            owner_node: owner_node.to_string(),
            replica_nodes: if replica_node.is_empty() {
                Vec::new()
            } else {
                vec![replica_node.to_string()]
            },
            replica_node: replica_node.to_string(),
            replication_count,
            required_replica_count,
            routed_remotely: true,
            fell_back_locally: false,
            cluster_role,
            promotion_epoch,
            declared_handler_runtime_name: String::new(),
        }
    }

    fn standby_authority(epoch: u64) -> ContinuityAuthorityConfig {
        ContinuityAuthorityConfig {
            cluster_role: ContinuityClusterRole::Standby,
            promotion_epoch: epoch,
        }
    }

    fn primary_authority(epoch: u64) -> ContinuityAuthorityConfig {
        ContinuityAuthorityConfig {
            cluster_role: ContinuityClusterRole::Primary,
            promotion_epoch: epoch,
        }
    }

    #[test]
    fn continuity_submit_created_duplicate_and_conflict() {
        let registry = continuity_fresh_registry();

        let created = registry
            .submit(continuity_submit_request(
                "req-1",
                "hash-a",
                "replica@host",
                0,
            ))
            .unwrap();
        assert_eq!(created.outcome, SubmitOutcome::Created);
        assert_eq!(created.record.attempt_id, "attempt-0");
        assert_eq!(created.record.phase, ContinuityPhase::Submitted);
        assert_eq!(created.record.replica_status, ReplicaStatus::Preparing);

        let duplicate = registry
            .submit(continuity_submit_request(
                "req-1",
                "hash-a",
                "replica@host",
                0,
            ))
            .unwrap();
        assert_eq!(duplicate.outcome, SubmitOutcome::Duplicate);
        assert_eq!(duplicate.record.attempt_id, "attempt-0");

        let conflict = registry
            .submit(continuity_submit_request(
                "req-1",
                "hash-b",
                "replica@host",
                0,
            ))
            .unwrap();
        assert_eq!(conflict.outcome, SubmitOutcome::Conflict);
        assert_eq!(conflict.conflict_reason, CONTINUITY_CONFLICT_REASON);
        assert_eq!(registry.next_attempt_token(), 1);
    }

    #[test]
    fn continuity_submit_recovery_retry_rolls_attempt_after_owner_loss() {
        let registry = continuity_fresh_registry();
        let initial = registry
            .submit(continuity_submit_request(
                "req-1",
                "hash-a",
                "replica@host",
                0,
            ))
            .unwrap();

        let recovered = registry
            .submit_with_hooks(
                continuity_submit_request_with_owner("req-1", "hash-a", "replica@host", "", 0),
                |record| Ok(record.canonical_replica_nodes()),
                |existing, request| {
                    existing.phase == ContinuityPhase::Submitted
                        && existing.result == ContinuityResult::Pending
                        && existing.owner_node != request.owner_node
                },
            )
            .unwrap();

        assert_eq!(recovered.outcome, SubmitOutcome::Created);
        assert_eq!(recovered.record.attempt_id, "attempt-1");
        assert_eq!(recovered.record.phase, ContinuityPhase::Submitted);
        assert_eq!(recovered.record.result, ContinuityResult::Pending);
        assert_eq!(recovered.record.owner_node, "replica@host");
        assert_eq!(recovered.record.replica_node, "");
        assert_eq!(recovered.record.replica_status, ReplicaStatus::Unassigned);
        assert_eq!(recovered.record.execution_node, "");
        assert_eq!(recovered.record.error, "");
        assert_eq!(registry.next_attempt_token(), 2);

        let rerolled = registry
            .submit_with_hooks(
                continuity_submit_request_with_owner("req-1", "hash-a", "owner-2@host", "", 0),
                |record| Ok(record.canonical_replica_nodes()),
                |existing, request| {
                    existing.phase == ContinuityPhase::Submitted
                        && existing.result == ContinuityResult::Pending
                        && existing.owner_node != request.owner_node
                },
            )
            .unwrap();
        assert_eq!(rerolled.outcome, SubmitOutcome::Created);
        assert_eq!(rerolled.record.attempt_id, "attempt-2");
        assert_eq!(rerolled.record.owner_node, "owner-2@host");
        assert_eq!(registry.next_attempt_token(), 3);

        let stored = registry.record("req-1").expect("recovered record present");
        assert_eq!(stored.attempt_id, rerolled.record.attempt_id);
        assert_ne!(stored.attempt_id, initial.record.attempt_id);
    }

    #[test]
    fn automatic_recovery_rolls_attempt_after_owner_loss() {
        continuity_submit_recovery_retry_rolls_attempt_after_owner_loss();
    }

    #[test]
    fn continuity_submit_recovery_retry_stays_duplicate_when_owner_is_still_authoritative() {
        let registry = continuity_fresh_registry();
        let initial = registry
            .submit(continuity_submit_request(
                "req-1",
                "hash-a",
                "replica@host",
                0,
            ))
            .unwrap();

        let duplicate = registry
            .submit_with_hooks(
                continuity_submit_request_with_owner("req-1", "hash-a", "replica@host", "", 0),
                |record| Ok(record.canonical_replica_nodes()),
                |_, _| false,
            )
            .unwrap();

        assert_eq!(duplicate.outcome, SubmitOutcome::Duplicate);
        assert_eq!(duplicate.record.attempt_id, initial.record.attempt_id);
        assert_eq!(registry.next_attempt_token(), 1);
    }

    #[test]
    fn continuity_submit_rejects_invalid_required_replica_count() {
        let registry = continuity_fresh_registry();
        let err = registry
            .submit(continuity_submit_request_fixture(SubmitRequestFixture {
                request_key: "req-1",
                payload_hash: "hash-a",
                owner_node: "owner@host",
                replica_node: "replica@host",
                required_replica_count: 2,
                replication_count: 2,
                cluster_role: ContinuityClusterRole::Primary,
                promotion_epoch: 0,
            }))
            .unwrap_err();
        assert_eq!(err, INVALID_REQUIRED_REPLICA_COUNT);
    }

    #[test]
    fn continuity_submit_preserves_replication_count_and_runtime_name() {
        let registry = continuity_fresh_registry();
        let mut request = continuity_submit_request("req-1", "hash-a", "replica@host", 1);
        request.declared_handler_runtime_name = "Work.handle_submit".to_string();

        let decision = registry
            .submit_with_replica_prepare(request, |_| Ok(()))
            .unwrap();

        assert_eq!(decision.outcome, SubmitOutcome::Created);
        assert_eq!(decision.record.replication_count, 2);
        assert_eq!(
            decision.record.declared_handler_runtime_name(),
            "Work.handle_submit"
        );
        let stored = registry.record("req-1").expect("stored record");
        assert_eq!(stored.replication_count, 2);
        assert_eq!(stored.declared_handler_runtime_name(), "Work.handle_submit");
    }

    #[test]
    fn continuity_submit_supports_more_than_one_record_replica() {
        let registry = continuity_fresh_registry();
        let mut prepared = false;

        let decision = registry
            .submit_with_replica_prepare(
                continuity_submit_request_fixture(SubmitRequestFixture {
                    request_key: "req-1",
                    payload_hash: "hash-a",
                    owner_node: "owner@host",
                    replica_node: "replica@host",
                    required_replica_count: 2,
                    replication_count: 3,
                    cluster_role: ContinuityClusterRole::Primary,
                    promotion_epoch: 0,
                }),
                |_| {
                    prepared = true;
                    Ok(())
                },
            )
            .unwrap();

        assert!(prepared);
        assert_eq!(decision.outcome, SubmitOutcome::Created);
        assert_eq!(decision.record.phase, ContinuityPhase::Submitted);
        assert_eq!(decision.record.replication_count, 3);
        assert_eq!(decision.record.replica_status, ReplicaStatus::Mirrored);
        let stored = registry.record("req-1").expect("stored record");
        assert_eq!(stored.replication_count, 3);
        assert!(stored.error.is_empty());
    }

    #[test]
    fn three_total_replicas_require_one_record_ack_for_strict_majority() {
        let registry = continuity_fresh_registry();
        let mut request = continuity_submit_request_fixture(SubmitRequestFixture {
            request_key: "req-majority",
            payload_hash: "hash-majority",
            owner_node: "owner@host",
            replica_node: "replica-a@host",
            required_replica_count: 2,
            replication_count: 3,
            cluster_role: ContinuityClusterRole::Primary,
            promotion_epoch: 0,
        });
        request.replica_nodes = vec!["replica-a@host".to_string(), "replica-b@host".to_string()];

        let decision = registry
            .submit_with_hooks(
                request,
                |_| Ok(vec!["replica-a@host".to_string()]),
                |_, _| false,
            )
            .expect("one replica plus the owner forms a majority of three");

        assert_eq!(
            decision.record.acknowledged_replica_nodes(),
            &["replica-a@host".to_string()]
        );
    }

    #[test]
    fn durable_runtime_record_hydrates_complete_inflight_state() {
        let source = continuity_fresh_registry();
        let decision = source
            .submit_with_replica_prepare(
                continuity_submit_request("req-hydrate", "hash-hydrate", "replica@host", 1),
                |_| Ok(()),
            )
            .expect("create source record");
        let encoded = encode_record(&decision.record).expect("encode durable runtime record");
        let restored = continuity_fresh_registry();

        let hydrated = hydrate_runtime_records(&restored, vec![encoded]).expect("hydrate record");

        assert_eq!(hydrated, 1);
        assert_eq!(restored.record("req-hydrate"), Some(decision.record));
    }

    #[test]
    fn default_count_route_completion_keeps_runtime_name_and_count_truth() {
        let registry = continuity_fresh_registry();
        ROUTE_BOUNDARY_HANDLER_CALLS.store(0, Ordering::Relaxed);

        let mut request = continuity_submit_request_fixture(SubmitRequestFixture {
            request_key: "http-route::Api.Todos.handle_list_todos::1",
            payload_hash: "payload-hash-1",
            owner_node: "owner@host",
            replica_node: "replica@host",
            required_replica_count: 1,
            replication_count: 2,
            cluster_role: ContinuityClusterRole::Primary,
            promotion_epoch: 0,
        });
        request.declared_handler_runtime_name = "Api.Todos.handle_list_todos".to_string();

        let decision = registry
            .submit_with_replica_prepare(request, |_| Ok(()))
            .expect("submit clustered route request");
        assert_eq!(decision.outcome, SubmitOutcome::Created);
        assert_eq!(decision.record.replica_status, ReplicaStatus::Mirrored);

        let response_payload = invoke_route_handler_from_payload(
            route_boundary_handler as *mut u8,
            &route_request_payload("payload"),
        )
        .expect("invoke route handler from payload");
        let response_ptr =
            decode_http_response_payload(&response_payload).expect("decode route response");
        let response = unsafe { &*(response_ptr as *const MeshHttpResponse) };
        assert_eq!(response.status, 200);
        let response_body = unsafe { (*(response.body as *const MeshString)).as_str() };
        assert_eq!(response_body, "handled:payload");
        assert_eq!(ROUTE_BOUNDARY_HANDLER_CALLS.load(Ordering::Relaxed), 1);

        let completed = registry
            .mark_completed(
                &decision.record.request_key,
                &decision.record.attempt_id,
                "owner@host",
            )
            .expect("mark completed");
        assert_eq!(completed.phase, ContinuityPhase::Completed);
        assert_eq!(completed.result, ContinuityResult::Succeeded);
        assert_eq!(completed.execution_node, "owner@host");
        assert_eq!(completed.replication_count, 2);
        assert_eq!(
            completed.declared_handler_runtime_name(),
            "Api.Todos.handle_list_todos"
        );
    }

    #[test]
    fn continuity_authority_allows_standby_epoch_after_fencing() {
        let authority = parse_authority_config(Some("standby"), Some("1")).unwrap();
        assert_eq!(authority.cluster_role, ContinuityClusterRole::Standby);
        assert_eq!(authority.promotion_epoch, 1);
    }

    #[test]
    fn continuity_merge_projects_remote_truth_into_standby_role() {
        let primary = continuity_fresh_registry();
        let standby = continuity_registry_with_authority(standby_authority(0));

        let primary_record = primary
            .submit_with_replica_prepare(
                continuity_submit_request("req-1", "hash-a", "replica@host", 1),
                |_| Ok(()),
            )
            .unwrap()
            .record;

        standby
            .merge_remote_record(1, primary_record.clone())
            .expect("merge standby mirrored record");

        let mirrored = standby
            .record("req-1")
            .expect("standby mirrored record present");
        assert_eq!(mirrored.cluster_role, ContinuityClusterRole::Standby);
        assert_eq!(mirrored.promotion_epoch, 0);
        assert_eq!(mirrored.replication_health, ReplicationHealth::Healthy);
        assert_eq!(mirrored.replica_status, primary_record.replica_status);
    }

    #[test]
    fn continuity_promotion_rejects_standby_without_mirrored_state() {
        let standby = continuity_registry_with_authority(standby_authority(0));
        let err = standby.promote_authority().unwrap_err();
        assert_eq!(err, PROMOTION_REJECTED_NO_MIRRORED_STATE);
    }

    #[test]
    fn automatic_promotion_rejects_without_mirrored_state() {
        continuity_promotion_rejects_standby_without_mirrored_state();
    }

    #[test]
    fn continuity_promotion_marks_mirrored_pending_record_owner_lost_and_reuses_retry_rollover() {
        let primary = continuity_fresh_registry();
        let standby = continuity_registry_with_authority(standby_authority(0));

        let primary_record = primary
            .submit_with_replica_prepare(
                continuity_submit_request("req-1", "hash-a", "replica@host", 1),
                |_| Ok(()),
            )
            .unwrap()
            .record;

        standby
            .merge_remote_record(1, primary_record)
            .expect("merge standby mirrored record");

        let promoted = standby
            .promote_authority()
            .expect("promote standby authority");
        assert_eq!(promoted, primary_authority(1));
        assert_eq!(standby.authority(), primary_authority(1));

        let promoted_record = standby.record("req-1").expect("promoted record present");
        assert_eq!(promoted_record.cluster_role, ContinuityClusterRole::Primary);
        assert_eq!(promoted_record.promotion_epoch, 1);
        assert_eq!(promoted_record.replica_status, ReplicaStatus::OwnerLost);
        assert_eq!(
            promoted_record.replication_health,
            ReplicationHealth::Unavailable
        );
        assert_eq!(promoted_record.error, "owner_lost:owner@host");

        let recovered = standby
            .submit(continuity_submit_request_with_authority(
                "req-1",
                "hash-a",
                "replica@host",
                "",
                0,
                ContinuityClusterRole::Primary,
                1,
            ))
            .expect("recovery submit after promotion");
        assert_eq!(recovered.outcome, SubmitOutcome::Created);
        assert_eq!(recovered.record.attempt_id, "attempt-1");
        assert_eq!(recovered.record.owner_node, "replica@host");
        assert_eq!(
            recovered.record.cluster_role,
            ContinuityClusterRole::Primary
        );
        assert_eq!(recovered.record.promotion_epoch, 1);
    }

    #[test]
    fn automatic_promotion_promotes_mirrored_pending_record_and_reuses_retry_rollover() {
        continuity_promotion_marks_mirrored_pending_record_owner_lost_and_reuses_retry_rollover();
    }

    #[test]
    fn continuity_repeated_promotion_rejects_already_promoted_primary() {
        let primary = continuity_fresh_registry();
        let standby = continuity_registry_with_authority(standby_authority(0));

        let primary_record = primary
            .submit_with_replica_prepare(
                continuity_submit_request("req-1", "hash-a", "replica@host", 1),
                |_| Ok(()),
            )
            .unwrap()
            .record;
        standby
            .merge_remote_record(1, primary_record)
            .expect("merge standby mirrored record");

        standby
            .promote_authority()
            .expect("first promotion succeeds");
        let err = standby.promote_authority().unwrap_err();
        assert_eq!(err, PROMOTION_REJECTED_NOT_STANDBY);
    }

    #[test]
    fn continuity_merge_higher_epoch_truth_fences_same_identity_rejoin() {
        let registry = continuity_registry_with_authority(primary_authority(0));
        let local = registry
            .submit(continuity_submit_request(
                "req-1",
                "hash-a",
                "replica@host",
                0,
            ))
            .unwrap();

        let incoming = ContinuityRecord {
            phase: ContinuityPhase::Completed,
            result: ContinuityResult::Succeeded,
            execution_node: "worker@new-primary".to_string(),
            cluster_role: ContinuityClusterRole::Primary,
            promotion_epoch: 1,
            replication_health: ReplicationHealth::Healthy,
            ..local.record.clone()
        };

        registry
            .merge_remote_record(2, incoming)
            .expect("merge higher epoch record");

        assert_eq!(registry.authority(), standby_authority(1));
        let merged = registry.record("req-1").expect("merged record present");
        assert_eq!(merged.cluster_role, ContinuityClusterRole::Standby);
        assert_eq!(merged.promotion_epoch, 1);
        assert_eq!(merged.phase, ContinuityPhase::Completed);
        assert_eq!(merged.execution_node, "worker@new-primary");
        assert_eq!(merged.replication_health, ReplicationHealth::Healthy);
    }

    #[test]
    fn continuity_merge_rejects_stale_lower_epoch_completion_before_projection() {
        let registry = continuity_registry_with_authority(primary_authority(1));
        let current = registry
            .submit(continuity_submit_request_with_authority(
                "req-1",
                "hash-a",
                "replica@host",
                "",
                0,
                ContinuityClusterRole::Primary,
                1,
            ))
            .unwrap();

        let stale = ContinuityRecord {
            phase: ContinuityPhase::Completed,
            result: ContinuityResult::Succeeded,
            execution_node: "worker@old-primary".to_string(),
            cluster_role: ContinuityClusterRole::Primary,
            promotion_epoch: 0,
            replication_health: ReplicationHealth::Healthy,
            ..current.record.clone()
        };

        let err = registry.merge_remote_record(2, stale).unwrap_err();
        assert_eq!(err, STALE_PROMOTION_EPOCH_REJECTED);

        let stored = registry.record("req-1").expect("current record present");
        assert_eq!(stored.cluster_role, ContinuityClusterRole::Primary);
        assert_eq!(stored.promotion_epoch, 1);
        assert_eq!(stored.phase, ContinuityPhase::Submitted);
        assert_eq!(stored.execution_node, "");
    }

    #[test]
    fn continuity_standby_truth_degrades_replication_health_without_owner_loss() {
        let primary = continuity_fresh_registry();
        let standby = continuity_registry_with_authority(standby_authority(0));

        let primary_record = primary
            .submit_with_replica_prepare(
                continuity_submit_request("req-1", "hash-a", "replica@host", 1),
                |_| Ok(()),
            )
            .unwrap()
            .record;

        standby
            .merge_remote_record(1, primary_record)
            .expect("merge standby mirrored record");

        assert!(standby
            .mark_owner_loss_records_for_node_loss("owner@host")
            .is_empty());

        let degraded = standby.degrade_replication_health_for_node_loss("owner@host");
        assert_eq!(degraded.len(), 1);
        assert_eq!(degraded[0].cluster_role, ContinuityClusterRole::Standby);
        assert_eq!(degraded[0].replication_health, ReplicationHealth::Degraded);

        let stored = standby
            .record("req-1")
            .expect("standby degraded record present");
        assert_eq!(stored.replica_status, ReplicaStatus::Mirrored);
        assert_eq!(stored.replication_health, ReplicationHealth::Degraded);
        assert_eq!(stored.error, "replication_source_lost:owner@host");
    }

    #[test]
    fn continuity_merge_prefers_healthier_state_at_same_epoch() {
        let registry = continuity_fresh_registry();
        let created = registry
            .submit(continuity_submit_request(
                "req-1",
                "hash-a",
                "replica@host",
                0,
            ))
            .unwrap();

        let unavailable = ContinuityRecord {
            replication_health: ReplicationHealth::Unavailable,
            ..created.record.clone()
        };
        registry
            .merge_remote_record(1, unavailable)
            .expect("merge unavailable record");

        let healthy = ContinuityRecord {
            replication_health: ReplicationHealth::Healthy,
            ..created.record.clone()
        };
        registry
            .merge_remote_record(1, healthy)
            .expect("merge healthy record");

        let merged = registry.record("req-1").expect("merged record present");
        assert_eq!(merged.replication_health, ReplicationHealth::Healthy);
    }

    #[test]
    fn continuity_submit_with_required_replica_rejects_when_replica_missing() {
        let registry = continuity_fresh_registry();

        let decision = registry
            .submit_with_replica_prepare(
                continuity_submit_request("req-1", "hash-a", "", 1),
                |_| Ok(()),
            )
            .unwrap();

        assert_eq!(decision.outcome, SubmitOutcome::Rejected);
        assert_eq!(decision.record.phase, ContinuityPhase::Rejected);
        assert_eq!(decision.record.result, ContinuityResult::Rejected);
        assert_eq!(
            decision.record.replica_status,
            ReplicaStatus::PreAdmissionRejected
        );
        assert_eq!(decision.record.error, REPLICA_REQUIRED_UNAVAILABLE);
    }

    #[test]
    fn continuity_submit_retries_pre_admission_rejection_and_preserves_conflict() {
        let registry = continuity_fresh_registry();

        let initial = registry
            .submit_with_replica_prepare(
                continuity_submit_request("req-1", "hash-a", "replica@host", 1),
                |_| Err(REPLICA_PREPARE_TIMEOUT.to_string()),
            )
            .unwrap();
        assert_eq!(initial.outcome, SubmitOutcome::Rejected);
        assert_eq!(
            decode_record(&encode_record(&initial.record).unwrap())
                .unwrap()
                .replica_status,
            ReplicaStatus::PreAdmissionRejected
        );

        let retried = registry
            .submit_with_replica_prepare(
                continuity_submit_request("req-1", "hash-a", "replica@host", 1),
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(retried.outcome, SubmitOutcome::Created);
        assert_ne!(retried.record.attempt_id, initial.record.attempt_id);
        assert_eq!(retried.record.phase, ContinuityPhase::Submitted);
        assert_eq!(retried.record.result, ContinuityResult::Pending);
        assert_eq!(retried.record.replica_status, ReplicaStatus::Mirrored);
        assert_eq!(retried.record.error, "");

        let conflict = registry
            .submit_with_replica_prepare(
                continuity_submit_request("req-1", "hash-b", "replica@host", 1),
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(conflict.outcome, SubmitOutcome::Conflict);
        assert_eq!(conflict.record.attempt_id, retried.record.attempt_id);
    }

    #[test]
    fn continuity_submit_keeps_handler_rejections_terminal() {
        let registry = continuity_fresh_registry();
        let created = registry
            .submit_with_replica_prepare(
                continuity_submit_request("req-1", "hash-a", "", 0),
                |_| Ok(()),
            )
            .unwrap();
        let rejected = registry
            .reject_durable_request("req-1", &created.record.attempt_id, "handler_failed")
            .unwrap();

        let duplicate = registry
            .submit_with_replica_prepare(
                continuity_submit_request("req-1", "hash-a", "", 0),
                |_| Ok(()),
            )
            .unwrap();

        assert_eq!(duplicate.outcome, SubmitOutcome::Duplicate);
        assert_eq!(duplicate.record, rejected);
    }

    #[test]
    fn continuity_submit_with_required_replica_mirrors_after_prepare_ack() {
        let registry = continuity_fresh_registry();

        let decision = registry
            .submit_with_replica_prepare(
                continuity_submit_request("req-1", "hash-a", "replica@host", 1),
                |_| Ok(()),
            )
            .unwrap();

        assert_eq!(decision.outcome, SubmitOutcome::Created);
        assert_eq!(decision.record.phase, ContinuityPhase::Submitted);
        assert_eq!(decision.record.replica_status, ReplicaStatus::Mirrored);
        assert_eq!(decision.record.error, "");
    }

    #[test]
    fn continuity_submit_with_required_replica_rejects_on_prepare_error() {
        let registry = continuity_fresh_registry();

        let decision = registry
            .submit_with_replica_prepare(
                continuity_submit_request("req-1", "hash-a", "replica@host", 1),
                |_| Err("replica_prepare_unavailable".to_string()),
            )
            .unwrap();

        assert_eq!(decision.outcome, SubmitOutcome::Rejected);
        assert_eq!(decision.record.phase, ContinuityPhase::Rejected);
        assert_eq!(decision.record.result, ContinuityResult::Rejected);
        assert_eq!(
            decision.record.replica_status,
            ReplicaStatus::PreAdmissionRejected
        );
        assert_eq!(decision.record.error, "replica_prepare_unavailable");
    }

    #[test]
    fn continuity_mark_completed_requires_matching_attempt_id() {
        let registry = continuity_fresh_registry();
        let created = registry
            .submit(continuity_submit_request(
                "req-1",
                "hash-a",
                "replica@host",
                0,
            ))
            .unwrap();

        let err = registry
            .mark_completed("req-1", "attempt-99", "worker@host")
            .unwrap_err();
        assert_eq!(err, ATTEMPT_ID_MISMATCH);

        let completed = registry
            .mark_completed("req-1", &created.record.attempt_id, "worker@host")
            .unwrap();
        assert_eq!(completed.phase, ContinuityPhase::Completed);
        assert_eq!(completed.result, ContinuityResult::Succeeded);
        assert_eq!(completed.execution_node, "worker@host");
    }

    #[test]
    fn continuity_mark_completed_rejects_stale_attempt_after_recovery_rollover() {
        let registry = continuity_fresh_registry();
        let initial = registry
            .submit(continuity_submit_request(
                "req-1",
                "hash-a",
                "replica@host",
                0,
            ))
            .unwrap();
        let recovered = registry
            .submit_with_hooks(
                continuity_submit_request_with_owner("req-1", "hash-a", "replica@host", "", 0),
                |record| Ok(record.canonical_replica_nodes()),
                |existing, request| existing.owner_node != request.owner_node,
            )
            .unwrap();

        let stale = registry
            .mark_completed("req-1", &initial.record.attempt_id, "worker@host")
            .unwrap_err();
        assert_eq!(stale, ATTEMPT_ID_MISMATCH);

        let completed = registry
            .mark_completed("req-1", &recovered.record.attempt_id, "worker@host")
            .unwrap();
        assert_eq!(completed.attempt_id, recovered.record.attempt_id);
        assert_eq!(completed.phase, ContinuityPhase::Completed);
        assert_eq!(completed.execution_node, "worker@host");
    }

    #[test]
    fn continuity_replica_prepare_ack_and_reject_transitions() {
        let registry = continuity_fresh_registry();
        let created = registry
            .submit(continuity_submit_request(
                "req-1",
                "hash-a",
                "replica@host",
                0,
            ))
            .unwrap()
            .record;

        let mirrored = registry.mirror_prepare(created.clone()).unwrap();
        assert_eq!(mirrored.replica_status, ReplicaStatus::Preparing);

        let acked = registry
            .acknowledge_replica_prepare("req-1", &created.attempt_id)
            .unwrap();
        assert_eq!(acked.replica_status, ReplicaStatus::Mirrored);

        let rejected = registry
            .reject_durable_request("req-1", &created.attempt_id, "replica_unavailable")
            .unwrap();
        assert_eq!(rejected.phase, ContinuityPhase::Rejected);
        assert_eq!(rejected.result, ContinuityResult::Rejected);
        assert_eq!(rejected.replica_status, ReplicaStatus::Rejected);
        assert_eq!(rejected.error, "replica_unavailable");
    }

    #[test]
    fn replica_prepare_accepts_monotonic_replica_replacement() {
        let registry = continuity_fresh_registry();
        let existing = registry
            .submit(continuity_submit_request(
                "req-1",
                "hash-a",
                "replica-a@host",
                0,
            ))
            .unwrap()
            .record;
        let mut replacement = existing.clone();
        replacement.record_version = replacement.record_version.saturating_add(1);
        replacement.replica_nodes = vec!["replica-b@host".to_string()];
        replacement.replica_node = "replica-b@host".to_string();
        replacement.acknowledged_replica_nodes.clear();

        let mirrored = registry.mirror_prepare(replacement.clone()).unwrap();
        assert_eq!(mirrored, replacement);
    }

    #[test]
    fn replica_prepare_accepts_fenced_owner_transfer_and_rejects_stale_attempt() {
        let registry = continuity_fresh_registry();
        let existing = registry
            .submit(continuity_submit_request(
                "req-1",
                "hash-a",
                "replica-a@host",
                0,
            ))
            .unwrap()
            .record;
        let mut transferred = existing.clone();
        transferred.record_version = transferred.record_version.saturating_add(1);
        transferred.attempt_id = "attempt-1".to_string();
        transferred.owner_node = "owner-b@host".to_string();
        transferred.replica_nodes = vec!["replica-b@host".to_string()];
        transferred.replica_node = "replica-b@host".to_string();
        transferred.acknowledged_replica_nodes.clear();

        assert_eq!(
            registry.mirror_prepare(transferred.clone()).unwrap(),
            transferred
        );
        assert_eq!(
            registry.mirror_prepare(existing).unwrap_err(),
            "stale_replica_prepare"
        );
    }

    #[test]
    fn replica_prepare_cannot_regress_terminal_record() {
        let registry = continuity_fresh_registry();
        let submitted = registry
            .submit(continuity_submit_request(
                "req-1",
                "hash-a",
                "replica-a@host",
                0,
            ))
            .unwrap()
            .record;
        let completed = registry
            .mark_completed("req-1", &submitted.attempt_id, "owner@host")
            .unwrap();
        let mut stale = submitted;
        stale.record_version = completed.record_version.saturating_add(1);

        assert_eq!(
            registry.mirror_prepare(stale).unwrap_err(),
            "stale_replica_prepare"
        );
        assert_eq!(registry.record("req-1").unwrap(), completed);
    }

    #[test]
    fn provisional_higher_attempt_cannot_erase_terminal_result_until_committed() {
        let registry = continuity_fresh_registry();
        let submitted = registry
            .submit(continuity_submit_request(
                "req-1",
                "hash-a",
                "replica-a@host",
                0,
            ))
            .unwrap()
            .record;
        let completed = registry
            .mark_completed("req-1", &submitted.attempt_id, "owner@host")
            .unwrap();

        let mut provisional = submitted;
        provisional.attempt_id = "attempt-1".to_string();
        provisional.owner_node = "owner-b@host".to_string();
        provisional.record_version = completed.record_version.saturating_add(1);
        provisional.replica_status = ReplicaStatus::Preparing;
        provisional.replication_health = ReplicationHealth::Unavailable;
        provisional.acknowledged_replica_nodes.clear();

        assert_eq!(
            preferred_record(completed.clone(), provisional.clone()),
            completed
        );

        let mut committed = provisional;
        committed.replica_status = ReplicaStatus::Mirrored;
        committed.replication_health = ReplicationHealth::Healthy;
        committed.acknowledged_replica_nodes = committed.replica_nodes.clone();
        assert_eq!(preferred_record(completed, committed.clone()), committed);
    }

    #[test]
    fn continuity_disconnect_marks_owner_lost_records_recoverable() {
        let registry = continuity_fresh_registry();
        let accepted = registry
            .submit_with_replica_prepare(
                continuity_submit_request("req-1", "hash-a", "replica@host", 1),
                |_| Ok(()),
            )
            .unwrap()
            .record;

        let owner_lost = registry.mark_owner_loss_records_for_node_loss("owner@host");
        assert_eq!(owner_lost.len(), 1);
        assert_eq!(owner_lost[0].request_key, accepted.request_key);
        assert_eq!(owner_lost[0].replica_status, ReplicaStatus::OwnerLost);
        assert_eq!(owner_lost[0].error, "owner_lost:owner@host");

        let acked_again = registry
            .acknowledge_replica_prepare("req-1", &accepted.attempt_id)
            .unwrap();
        assert_eq!(acked_again.replica_status, ReplicaStatus::OwnerLost);
        assert_eq!(acked_again.error, "owner_lost:owner@host");

        let late_timeout = registry
            .reject_durable_request(
                "req-1",
                &accepted.attempt_id,
                "clustered_http_reservation_timeout",
            )
            .unwrap();
        assert_eq!(late_timeout.phase, ContinuityPhase::Submitted);
        assert_eq!(late_timeout.replica_status, ReplicaStatus::OwnerLost);
        assert_eq!(late_timeout.error, "owner_lost:owner@host");
    }

    #[test]
    fn request_timeout_marks_only_the_fenced_request_owner_lost() {
        let registry = continuity_fresh_registry();
        let first = registry
            .submit_with_replica_prepare(
                continuity_submit_request("req-1", "hash-a", "replica@host", 1),
                |_| Ok(()),
            )
            .unwrap()
            .record;
        let second = registry
            .submit_with_replica_prepare(
                continuity_submit_request("req-2", "hash-b", "replica@host", 1),
                |_| Ok(()),
            )
            .unwrap()
            .record;

        let transitioned = registry
            .mark_owner_loss_for_request("req-1", &first.attempt_id, "owner@host")
            .unwrap()
            .expect("request transitioned");
        assert_eq!(transitioned.replica_status, ReplicaStatus::OwnerLost);
        assert_eq!(
            registry.record("req-2").unwrap().replica_status,
            second.replica_status
        );
        assert!(registry
            .mark_owner_loss_for_request("req-2", "stale-attempt", "owner@host")
            .unwrap()
            .is_none());
    }

    #[test]
    fn continuity_submit_recovery_retry_uses_owner_lost_state_on_ordinary_submit_path() {
        let registry = continuity_fresh_registry();
        let initial = registry
            .submit_with_replica_prepare(
                continuity_submit_request("req-1", "hash-a", "replica@host", 1),
                |_| Ok(()),
            )
            .unwrap();

        let owner_lost = registry.mark_owner_loss_records_for_node_loss("owner@host");
        assert_eq!(owner_lost.len(), 1);

        let recovered = registry
            .submit(continuity_submit_request_with_owner(
                "req-1",
                "hash-a",
                "replica@host",
                "",
                0,
            ))
            .unwrap();
        assert_eq!(recovered.outcome, SubmitOutcome::Created);
        assert_eq!(recovered.record.attempt_id, "attempt-1");
        assert_eq!(recovered.record.owner_node, "replica@host");
        assert_eq!(recovered.record.replica_status, ReplicaStatus::Unassigned);
        assert_eq!(recovered.record.error, "");

        let stale = registry
            .mark_completed("req-1", &initial.record.attempt_id, "owner@host")
            .unwrap_err();
        assert_eq!(stale, ATTEMPT_ID_MISMATCH);
    }

    #[test]
    fn continuity_owner_loss_ignores_unrelated_nodes_and_terminal_records() {
        let registry = continuity_fresh_registry();
        let pending = registry
            .submit_with_replica_prepare(
                continuity_submit_request("req-1", "hash-a", "replica@host", 1),
                |_| Ok(()),
            )
            .unwrap();
        let completed = registry
            .submit(continuity_submit_request_with_owner(
                "req-2",
                "hash-b",
                "owner@host",
                "replica@host",
                0,
            ))
            .unwrap();
        registry
            .mark_completed("req-2", &completed.record.attempt_id, "worker@host")
            .unwrap();

        assert!(registry
            .mark_owner_loss_records_for_node_loss("someone-else@host")
            .is_empty());

        let owner_lost = registry.mark_owner_loss_records_for_node_loss("owner@host");
        assert_eq!(owner_lost.len(), 1);
        assert_eq!(owner_lost[0].request_key, pending.record.request_key);

        let completed_record = registry.record("req-2").expect("completed record present");
        assert_eq!(completed_record.phase, ContinuityPhase::Completed);
        assert_eq!(completed_record.replica_status, ReplicaStatus::Preparing);
    }

    #[test]
    fn continuity_owner_loss_transition_is_idempotent_for_repeated_disconnects() {
        let registry = continuity_fresh_registry();
        registry
            .submit_with_replica_prepare(
                continuity_submit_request("req-1", "hash-a", "replica@host", 1),
                |_| Ok(()),
            )
            .unwrap();

        let first = registry.mark_owner_loss_records_for_node_loss("owner@host");
        let second = registry.mark_owner_loss_records_for_node_loss("owner@host");

        assert_eq!(first.len(), 1);
        assert!(second.is_empty());
    }

    #[test]
    fn continuity_merge_prefers_owner_lost_over_stale_mirrored() {
        let registry = continuity_fresh_registry();
        let accepted = registry
            .submit_with_replica_prepare(
                continuity_submit_request("req-1", "hash-a", "replica@host", 1),
                |_| Ok(()),
            )
            .unwrap()
            .record;

        let owner_lost = registry
            .mark_owner_loss_records_for_node_loss("owner@host")
            .into_iter()
            .next()
            .expect("owner-lost record present");

        let stale_mirrored = ContinuityRecord {
            replica_status: ReplicaStatus::Mirrored,
            error: String::new(),
            ..accepted
        };
        registry
            .merge_remote_record(1, stale_mirrored)
            .expect("merge stale mirrored record");

        let merged = registry.record("req-1").expect("merged record present");
        assert_eq!(merged.replica_status, ReplicaStatus::OwnerLost);
        assert_eq!(merged.error, owner_lost.error);
    }

    #[test]
    fn continuity_disconnect_degrades_mirrored_pending_records() {
        let registry = continuity_fresh_registry();
        let accepted = registry
            .submit_with_replica_prepare(
                continuity_submit_request("req-1", "hash-a", "replica@host", 1),
                |_| Ok(()),
            )
            .unwrap()
            .record;

        let degraded = registry.degrade_replica_records_for_node_loss("replica@host");
        assert_eq!(degraded.len(), 1);
        assert_eq!(degraded[0].request_key, accepted.request_key);
        assert_eq!(
            degraded[0].replica_status,
            ReplicaStatus::DegradedContinuing
        );
        assert_eq!(degraded[0].error, "replica_lost:replica@host");

        let acked_again = registry
            .acknowledge_replica_prepare("req-1", &accepted.attempt_id)
            .unwrap();
        assert_eq!(
            acked_again.replica_status,
            ReplicaStatus::DegradedContinuing
        );
    }

    #[test]
    fn continuity_snapshot_merge_prefers_terminal_record_and_advances_counter() {
        let left = continuity_fresh_registry();
        let right = continuity_fresh_registry();

        let created = left
            .submit(continuity_submit_request(
                "req-1",
                "hash-a",
                "replica@host",
                0,
            ))
            .unwrap()
            .record;
        let completed = left
            .mark_completed("req-1", &created.attempt_id, "worker@host")
            .unwrap();

        right
            .merge_remote_record(7, created.clone())
            .expect("merge created record");
        right
            .merge_snapshot(ContinuitySnapshot {
                next_attempt_token: 7,
                records: vec![completed.clone()],
            })
            .expect("merge snapshot");

        let merged = right.record("req-1").expect("merged record present");
        assert_eq!(merged.phase, ContinuityPhase::Completed);
        assert_eq!(merged.execution_node, "worker@host");
        assert_eq!(right.next_attempt_token(), 7);
    }

    #[test]
    fn continuity_merge_remote_record_rejects_stale_terminal_attempt_after_rollover() {
        let registry = continuity_fresh_registry();
        let initial = registry
            .submit(continuity_submit_request(
                "req-1",
                "hash-a",
                "replica@host",
                0,
            ))
            .unwrap();
        let recovered = registry
            .submit_with_hooks(
                continuity_submit_request_with_owner("req-1", "hash-a", "replica@host", "", 0),
                |record| Ok(record.canonical_replica_nodes()),
                |existing, request| existing.owner_node != request.owner_node,
            )
            .unwrap();

        let stale_completed = ContinuityRecord {
            phase: ContinuityPhase::Completed,
            result: ContinuityResult::Succeeded,
            execution_node: "worker@host".to_string(),
            replica_status: ReplicaStatus::Mirrored,
            error: String::new(),
            ..initial.record.clone()
        };
        registry
            .merge_remote_record(2, stale_completed)
            .expect("merge stale completed record");

        let merged = registry.record("req-1").expect("merged record present");
        assert_eq!(merged.attempt_id, recovered.record.attempt_id);
        assert_eq!(merged.phase, ContinuityPhase::Submitted);
        assert_eq!(merged.result, ContinuityResult::Pending);
        assert_eq!(registry.next_attempt_token(), 2);
    }

    #[test]
    fn continuity_merge_remote_record_ignores_invalid_attempt_ids() {
        let registry = continuity_fresh_registry();
        let created = registry
            .submit(continuity_submit_request(
                "req-1",
                "hash-a",
                "replica@host",
                0,
            ))
            .unwrap();

        let malformed = ContinuityRecord {
            attempt_id: "not-an-attempt".to_string(),
            phase: ContinuityPhase::Completed,
            result: ContinuityResult::Succeeded,
            execution_node: "worker@host".to_string(),
            replica_status: ReplicaStatus::Mirrored,
            error: String::new(),
            ..created.record.clone()
        };
        registry
            .merge_remote_record(5, malformed)
            .expect("ignore malformed upsert");

        let merged = registry.record("req-1").expect("merged record present");
        assert_eq!(merged.attempt_id, created.record.attempt_id);
        assert_eq!(merged.phase, ContinuityPhase::Submitted);
        assert_eq!(registry.next_attempt_token(), 5);
    }

    #[test]
    fn continuity_merge_snapshot_rejects_stale_terminal_attempt_after_rollover() {
        let registry = continuity_fresh_registry();
        let initial = registry
            .submit(continuity_submit_request(
                "req-1",
                "hash-a",
                "replica@host",
                0,
            ))
            .unwrap();
        let recovered = registry
            .submit_with_hooks(
                continuity_submit_request_with_owner("req-1", "hash-a", "replica@host", "", 0),
                |record| Ok(record.canonical_replica_nodes()),
                |existing, request| existing.owner_node != request.owner_node,
            )
            .unwrap();

        let stale_completed = ContinuityRecord {
            phase: ContinuityPhase::Completed,
            result: ContinuityResult::Succeeded,
            execution_node: "worker@host".to_string(),
            replica_status: ReplicaStatus::Mirrored,
            error: String::new(),
            ..initial.record.clone()
        };
        registry
            .merge_snapshot(ContinuitySnapshot {
                next_attempt_token: 2,
                records: vec![stale_completed],
            })
            .expect("merge stale snapshot");

        let merged = registry.record("req-1").expect("merged record present");
        assert_eq!(merged.attempt_id, recovered.record.attempt_id);
        assert_eq!(merged.phase, ContinuityPhase::Submitted);
        assert_eq!(merged.result, ContinuityResult::Pending);
        assert_eq!(registry.next_attempt_token(), 2);
    }

    #[test]
    fn continuity_merge_prefers_degraded_over_stale_mirrored() {
        let registry = continuity_fresh_registry();
        let accepted = registry
            .submit_with_replica_prepare(
                continuity_submit_request("req-1", "hash-a", "replica@host", 1),
                |_| Ok(()),
            )
            .unwrap()
            .record;

        let degraded = registry
            .degrade_replica_records_for_node_loss("replica@host")
            .into_iter()
            .next()
            .expect("degraded record present");

        let stale_mirrored = ContinuityRecord {
            replica_status: ReplicaStatus::Mirrored,
            error: String::new(),
            ..accepted
        };
        registry
            .merge_remote_record(1, stale_mirrored)
            .expect("merge stale mirrored record");

        let merged = registry.record("req-1").expect("merged record present");
        assert_eq!(merged.replica_status, ReplicaStatus::DegradedContinuing);
        assert_eq!(merged.error, degraded.error);
    }

    #[test]
    fn continuity_upsert_wire_roundtrip() {
        let record = ContinuityRecord {
            request_key: "req-1".to_string(),
            payload_hash: "hash-a".to_string(),
            record_version: 3,
            request_payload: b"encoded-request".to_vec(),
            attempt_id: "attempt-4".to_string(),
            phase: ContinuityPhase::Submitted,
            result: ContinuityResult::Pending,
            ingress_node: "ingress@host".to_string(),
            owner_node: "owner@host".to_string(),
            replica_nodes: vec!["replica@host".to_string()],
            acknowledged_replica_nodes: vec!["replica@host".to_string()],
            replica_node: "replica@host".to_string(),
            replication_count: 2,
            replica_status: ReplicaStatus::Mirrored,
            cluster_role: ContinuityClusterRole::Primary,
            promotion_epoch: 0,
            replication_health: ReplicationHealth::Healthy,
            execution_node: String::new(),
            routed_remotely: true,
            fell_back_locally: false,
            error: String::new(),
            declared_handler_runtime_name: String::new(),
        };

        let payload = encode_upsert_payload(5, &record).expect("encode upsert payload");
        assert_eq!(payload[0], super::super::node::DIST_CONTINUITY_UPSERT);

        let (watermark, decoded) = decode_upsert_payload(&payload).expect("decode upsert payload");
        assert_eq!(watermark, 5);
        assert_eq!(decoded, record);
    }

    #[test]
    fn continuity_sync_wire_roundtrip() {
        let snapshot = ContinuitySnapshot {
            next_attempt_token: 9,
            records: vec![
                ContinuityRecord {
                    request_key: "req-1".to_string(),
                    payload_hash: "hash-a".to_string(),
                    record_version: 3,
                    request_payload: b"first-request".to_vec(),
                    attempt_id: "attempt-4".to_string(),
                    phase: ContinuityPhase::Completed,
                    result: ContinuityResult::Succeeded,
                    ingress_node: "ingress@host".to_string(),
                    owner_node: "owner@host".to_string(),
                    replica_nodes: vec!["replica@host".to_string()],
                    acknowledged_replica_nodes: vec!["replica@host".to_string()],
                    replica_node: "replica@host".to_string(),
                    replication_count: 2,
                    replica_status: ReplicaStatus::Mirrored,
                    cluster_role: ContinuityClusterRole::Primary,
                    promotion_epoch: 0,
                    replication_health: ReplicationHealth::Healthy,
                    execution_node: "worker@host".to_string(),
                    routed_remotely: true,
                    fell_back_locally: false,
                    error: String::new(),
                    declared_handler_runtime_name: String::new(),
                },
                ContinuityRecord {
                    request_key: "req-2".to_string(),
                    payload_hash: "hash-b".to_string(),
                    record_version: 2,
                    request_payload: Vec::new(),
                    attempt_id: "attempt-7".to_string(),
                    phase: ContinuityPhase::Rejected,
                    result: ContinuityResult::Rejected,
                    ingress_node: "ingress@host".to_string(),
                    owner_node: "owner@host".to_string(),
                    replica_nodes: vec!["replica@host".to_string()],
                    acknowledged_replica_nodes: Vec::new(),
                    replica_node: "replica@host".to_string(),
                    replication_count: 2,
                    replica_status: ReplicaStatus::Rejected,
                    cluster_role: ContinuityClusterRole::Primary,
                    promotion_epoch: 0,
                    replication_health: ReplicationHealth::Unavailable,
                    execution_node: String::new(),
                    routed_remotely: true,
                    fell_back_locally: false,
                    error: "replica_unavailable".to_string(),
                    declared_handler_runtime_name: String::new(),
                },
            ],
        };

        let payload = encode_sync_payload(&snapshot).expect("encode sync payload");
        assert_eq!(payload[0], super::super::node::DIST_CONTINUITY_SYNC);

        let decoded = decode_sync_payload(&payload).expect("decode sync payload");
        assert_eq!(decoded.next_attempt_token, snapshot.next_attempt_token);
        assert_eq!(decoded.records, snapshot.records);
    }
}
