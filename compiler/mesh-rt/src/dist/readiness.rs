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
    super::node::autonomous_mode_requested()
}

/// The roles `MESH_ROLES` gives this node (default `gateway,worker`), in
/// any case.
pub(crate) fn local_roles() -> NodeRoles {
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
    lifecycle_state(std::env::var("MESH_NODE_STATE").ok().as_deref())
}

/// The state `MESH_NODE_STATE` (`declared`) names, other than ready; or
/// else the one readiness finds.
fn lifecycle_state(declared: Option<&str>) -> NodeLifecycleState {
    declared
        .and_then(lifecycle_state_named)
        .filter(|state| *state != NodeLifecycleState::Ready)
        .or_else(|| lifecycle_state_named(&local_readiness_status().state))
        .unwrap_or(NodeLifecycleState::Ready)
}

fn lifecycle_state_named(name: &str) -> Option<NodeLifecycleState> {
    (0..=u8::MAX)
        .map_while(|value| NodeLifecycleState::from_u8(value).ok())
        .find(|state| state.as_str() == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A manual node is ready as soon as it runs. An autonomous one names
    /// each gate it has not passed; without a stable identity it has
    /// failed.
    #[test]
    fn readiness_gates_an_autonomous_node_and_not_a_manual_one() {
        let manual = local_readiness_status();
        assert!(manual.ready);
        assert_eq!(manual.state, "ready");
        assert_eq!(
            manual.gates,
            vec![gate("autonomous_mode", true, "manual_mode")]
        );

        let autonomous = super::super::node::in_autonomous_mode(local_readiness_status);
        assert!(!autonomous.ready);
        assert_eq!(autonomous.state, "failed");
        let failing: Vec<_> = autonomous
            .gates
            .iter()
            .filter(|gate| !gate.ready)
            .map(|gate| gate.reason.as_str())
            .collect();
        assert!(failing.contains(&"identity_or_mtls_missing"), "{failing:?}");
        assert_eq!(
            super::super::node::in_autonomous_mode(|| lifecycle_state(None)),
            NodeLifecycleState::Failed
        );
    }

    /// `MESH_NODE_STATE` names a node's state outright, but for ready,
    /// which readiness must find; an unknown name is no state.
    #[test]
    fn a_declared_node_state_overrides_all_but_ready() {
        for state in (0..=7).map(|value| NodeLifecycleState::from_u8(value).unwrap()) {
            let expected = if state == NodeLifecycleState::Ready {
                lifecycle_state(None)
            } else {
                state
            };
            assert_eq!(lifecycle_state(Some(state.as_str())), expected);
        }
        assert_eq!(lifecycle_state(Some("sleepy")), lifecycle_state(None));
        assert_eq!(lifecycle_state_named("sleepy"), None);
    }

    /// Roles come from `MESH_ROLES`, a gateway and worker by default, and a
    /// peer's transport counts as stable after two discovery intervals.
    #[test]
    fn a_node_has_its_default_roles_and_stability_window() {
        let roles = local_roles();
        assert_eq!(
            roles.contains(NodeRoles::WORKER),
            std::env::var("MESH_ROLES").map_or(true, |roles| roles.contains("worker"))
        );
        assert!(transport_stability_window() >= std::time::Duration::from_millis(450));
    }
}
