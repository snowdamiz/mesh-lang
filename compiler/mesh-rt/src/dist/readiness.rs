//! Runtime-owned readiness gates for autonomous placement.

use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use super::telemetry::{NodeLifecycleState, NodeRoles};

static INITIAL_STATE_SYNCHRONIZED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadinessGate {
    pub name: String,
    pub ready: bool,
    pub reason: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeReadinessStatus {
    pub ready: bool,
    pub state: String,
    pub gates: Vec<ReadinessGate>,
}

pub(crate) fn mark_initial_state_synchronized() {
    INITIAL_STATE_SYNCHRONIZED.store(true, Ordering::Release);
}

fn gate(name: &str, ready: bool, reason: &str) -> ReadinessGate {
    ReadinessGate {
        name: name.to_string(),
        ready,
        reason: if ready { "ready" } else { reason }.to_string(),
    }
}

fn autonomous_requested() -> bool {
    std::env::var("MESH_CLUSTER_MODE")
        .is_ok_and(|value| value.trim().eq_ignore_ascii_case("autonomous"))
        || super::autonomous::embedded_autonomous_config()
            .is_some_and(|config| config.enabled && config.features.protocol_two)
}

fn local_roles() -> NodeRoles {
    let roles = std::env::var("MESH_ROLES").unwrap_or_else(|_| "gateway,worker".to_string());
    NodeRoles::new(
        roles
            .split(',')
            .any(|role| role.trim().eq_ignore_ascii_case("controller")),
        roles
            .split(',')
            .any(|role| role.trim().eq_ignore_ascii_case("gateway")),
        roles
            .split(',')
            .any(|role| role.trim().eq_ignore_ascii_case("worker")),
    )
}

fn transport_stability_window() -> std::time::Duration {
    let discovery_interval_millis = std::env::var("MESH_DISCOVERY_INTERVAL_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(5_000)
        .clamp(100, 30_000);
    std::time::Duration::from_millis(
        discovery_interval_millis
            .saturating_mul(2)
            .saturating_add(250),
    )
}

pub fn local_readiness_status() -> NodeReadinessStatus {
    if !autonomous_requested() {
        return NodeReadinessStatus {
            ready: true,
            state: NodeLifecycleState::Ready.as_str().to_string(),
            gates: vec![gate("autonomous_mode", true, "manual_mode")],
        };
    }

    let state = super::node::node_state();
    let roles = local_roles();
    let default_minimum_peers = if roles.contains(NodeRoles::CONTROLLER)
        && std::env::var("MESH_CONTROLLER_VOTERS").is_ok_and(|value| {
            value
                .split(',')
                .filter(|item| !item.trim().is_empty())
                .count()
                == 1
        }) {
        0
    } else {
        1
    };
    let minimum_peers = std::env::var("MESH_MIN_HEALTHY_PEERS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default_minimum_peers);
    let stable_identity = std::env::var("MESH_CLUSTER_ID")
        .is_ok_and(|value| !value.trim().is_empty())
        && std::env::var("MESH_STABLE_NODE_ID").is_ok_and(|value| !value.trim().is_empty())
        && std::env::var("MESH_TLS_CA_DER_B64").is_ok()
        && std::env::var("MESH_TLS_CERT_DER_B64").is_ok()
        && std::env::var("MESH_TLS_KEY_DER_B64").is_ok();
    let stability_window = transport_stability_window();
    let (peer_count, protocol_ready) = state.map_or((0, false), |state| {
        let sessions = state.sessions.read();
        let compatible = sessions.values().all(|session| {
            !session.shutdown.load(Ordering::Acquire)
                && session.negotiated_protocol.autonomous_enabled
        });
        let stable_compatible = sessions
            .values()
            .filter(|session| {
                !session.shutdown.load(Ordering::Acquire)
                    && session.negotiated_protocol.autonomous_enabled
                    && session.connected_at.elapsed() >= stability_window
            })
            .count();
        (
            sessions.len(),
            compatible && (minimum_peers == 0 || stable_compatible >= minimum_peers),
        )
    });
    let controller_consensus = if roles.contains(NodeRoles::CONTROLLER) {
        super::consensus::consensus_runtime_snapshot().is_some_and(|snapshot| {
            snapshot.current_leader.is_some()
                && !snapshot.voter_ids.is_empty()
                && snapshot.last_applied_log.is_some()
        })
    } else {
        true
    };
    let handlers_ready =
        !roles.contains(NodeRoles::WORKER) || super::node::declared_handler_count() > 0;
    let continuity_ready = super::continuity_store::configured_continuity_store().is_some();
    let synchronized = (peer_count == 0 && minimum_peers == 0)
        || INITIAL_STATE_SYNCHRONIZED.load(Ordering::Acquire);
    let scheduler_ready = crate::actor::GLOBAL_SCHEDULER.get().is_some();
    let application_ready = !std::env::var("MESH_APPLICATION_READY")
        .is_ok_and(|value| value.trim().eq_ignore_ascii_case("false") || value.trim() == "0");

    let gates = vec![
        gate(
            "stable_identity_and_mtls",
            stable_identity,
            "identity_or_mtls_missing",
        ),
        gate(
            "protocol_capabilities",
            protocol_ready,
            "autonomous_protocol_not_negotiated",
        ),
        gate(
            "handler_metadata",
            handlers_ready,
            "handlers_not_registered",
        ),
        gate(
            "continuity_store",
            continuity_ready,
            "continuity_store_not_ready",
        ),
        gate(
            "state_synchronization",
            synchronized,
            "initial_state_sync_incomplete",
        ),
        gate(
            "peer_connectivity",
            peer_count >= minimum_peers,
            "minimum_peer_connectivity_unmet",
        ),
        gate(
            "controller_quorum",
            controller_consensus,
            "controller_quorum_unavailable",
        ),
        gate(
            "scheduler_and_admission",
            scheduler_ready,
            "scheduler_not_initialized",
        ),
        gate(
            "application_readiness",
            application_ready,
            "application_readiness_failed",
        ),
    ];
    let ready = gates.iter().all(|gate| gate.ready);
    let lifecycle = if !stable_identity {
        NodeLifecycleState::Failed
    } else if peer_count < minimum_peers || !protocol_ready {
        NodeLifecycleState::Joining
    } else if !ready {
        NodeLifecycleState::Warming
    } else {
        NodeLifecycleState::Ready
    };
    NodeReadinessStatus {
        ready,
        state: lifecycle.as_str().to_string(),
        gates,
    }
}

pub(crate) fn local_lifecycle_state() -> NodeLifecycleState {
    if super::node::node_state().is_some_and(|state| super::operator::drain_requested(&state.name))
    {
        return NodeLifecycleState::Draining;
    }
    match std::env::var("MESH_NODE_STATE").as_deref() {
        Ok("provisioning") => NodeLifecycleState::Provisioning,
        Ok("joining") => NodeLifecycleState::Joining,
        Ok("warming") => NodeLifecycleState::Warming,
        Ok("draining") => NodeLifecycleState::Draining,
        Ok("terminating") => NodeLifecycleState::Terminating,
        Ok("removed") => NodeLifecycleState::Removed,
        Ok("failed") => NodeLifecycleState::Failed,
        _ => {
            let status = local_readiness_status();
            match status.state.as_str() {
                "joining" => NodeLifecycleState::Joining,
                "warming" => NodeLifecycleState::Warming,
                "failed" => NodeLifecycleState::Failed,
                _ => NodeLifecycleState::Ready,
            }
        }
    }
}
