use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, Instant};

use super::continuity_store::{
    ContinuityStore, ContinuityStoreLimits, SqliteContinuityStore, StoredContinuityPhase,
    StoredContinuityRecord,
};
use super::routing::{LoadReportRegistry, NodeLoadReport};
use super::scaling::{
    reconcile_scale_up, select_drain_candidate, Autoscaler, ControlTerm, DesiredCapacity,
    DesiredRevision, DrainCandidate, FakeCapacityDriver, ScalingPolicy, ScalingSample,
};
use super::telemetry::{NodeLifecycleState, NodeRoles};

fn scaling_sample(at: Instant, inflight: u64, ready: u16) -> ScalingSample {
    ScalingSample {
        observed_at: at,
        cluster_inflight: inflight,
        cluster_pressure_ewma: inflight as f64 / 10.0,
        ready_nodes: ready,
        reports_complete: true,
        driver_healthy: true,
        controller_stable: true,
        continuity_healthy: true,
        drain_incomplete: false,
    }
}

#[test]
fn model_property_desired_capacity_always_remains_within_bounds() {
    let start = Instant::now();
    for min_nodes in 1..=4 {
        for max_nodes in min_nodes..=8 {
            for current in min_nodes..=max_nodes {
                for inflight in [0, 1, 9, 10, 11, 100, 10_000] {
                    let policy = ScalingPolicy {
                        min_nodes,
                        max_nodes,
                        target_inflight_per_node: 10,
                        scale_up_window_millis: 1,
                        scale_down_window_millis: 2,
                        cooldown_millis: 0,
                        max_scale_up_step: 3,
                        max_scale_down_step: 2,
                        max_unavailable: min_nodes.saturating_sub(1),
                    };
                    let mut autoscaler = Autoscaler::new(policy).expect("valid policy");
                    autoscaler.evaluate(current, scaling_sample(start, inflight, current));
                    let decision = autoscaler.evaluate(
                        current,
                        scaling_sample(start + Duration::from_millis(3), inflight, current),
                    );
                    assert!(
                        (min_nodes..=max_nodes).contains(&decision.bounded_desired),
                        "min={min_nodes} max={max_nodes} current={current} inflight={inflight} decision={decision:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn model_property_scale_down_never_selects_an_unsafe_candidate() {
    for unavailable in 0..=2 {
        for max_unavailable in 1..=2 {
            for flags in 0_u8..8 {
                let candidate = DrainCandidate {
                    node_id: format!("candidate-{flags}"),
                    transferable_load: u64::from(flags),
                    active_ownership_transfers: u32::from(flags),
                    controller_voter: flags & 1 != 0,
                    only_active_copy: flags & 2 != 0,
                    unique_capability: flags & 4 != 0,
                    template_revision: "v1".to_string(),
                };
                let result = select_drain_candidate(
                    std::slice::from_ref(&candidate),
                    unavailable,
                    max_unavailable,
                );
                if unavailable >= max_unavailable
                    || candidate.controller_voter
                    || candidate.only_active_copy
                    || candidate.unique_capability
                {
                    assert!(result.is_err(), "unsafe candidate selected: {candidate:?}");
                } else {
                    assert_eq!(result.expect("safe candidate"), candidate);
                }
            }
        }
    }
}

#[test]
fn model_property_arbitrary_driver_retries_create_one_operation_per_ordinal() {
    for desired_workers in 1..=12 {
        let driver = FakeCapacityDriver::new();
        let desired = DesiredCapacity {
            revision: DesiredRevision(7),
            worker_nodes: desired_workers,
            gateway_nodes: 0,
            template_revision: "v1".to_string(),
        };
        for _ in 0..32 {
            reconcile_scale_up(&driver, "model-cluster", ControlTerm(3), &desired, 0)
                .expect("retry reconciliation");
        }
        let observed = super::scaling::CapacityDriver::observe_capacity(&driver, "model-cluster")
            .expect("observe capacity");
        let operation_ids = observed
            .nodes
            .iter()
            .map(|node| node.operation_id.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(observed.nodes.len(), desired_workers as usize);
        assert_eq!(operation_ids.len(), desired_workers as usize);
    }
}

fn report(sequence: u64) -> NodeLoadReport {
    NodeLoadReport {
        protocol_version: 3,
        node_id: "worker-a".to_string(),
        boot_id: "boot-a".to_string(),
        roles: NodeRoles::new(false, false, true),
        state: NodeLifecycleState::Ready,
        capacity_units: 1,
        active_workers: 2,
        runnable_actors: 0,
        inflight: sequence as u32,
        queued_items: 0,
        queued_bytes: 0,
        outstanding_reservations: 0,
        p95_queue_wait: Duration::ZERO,
        memory_pressure: 0.0,
        decision_pressure_ewma: 0.0,
        sequence,
        control_term: 1,
        membership_generation: 1,
        failure_domain: "zone-a".to_string(),
        handlers: BTreeSet::new(),
    }
}

#[test]
fn model_property_reordered_and_duplicate_load_reports_are_rejected_safely() {
    for first in 1..=16 {
        for second in 1..=16 {
            let registry = LoadReportRegistry::default();
            let now = Instant::now();
            registry.apply(report(first), now).expect("first report");
            let result = registry.apply(report(second), now);
            if second <= first {
                assert_eq!(result, Err("load_report_out_of_order".to_string()));
                assert_eq!(
                    registry
                        .report("worker-a", now, Duration::from_secs(1))
                        .expect("retained report")
                        .sequence,
                    first
                );
            } else {
                result.expect("newer report");
                assert_eq!(
                    registry
                        .report("worker-a", now, Duration::from_secs(1))
                        .expect("new report")
                        .sequence,
                    second
                );
            }
        }
    }
}

fn stored_record(version: u64, owner_generation: u64) -> StoredContinuityRecord {
    StoredContinuityRecord {
        operation_key: "operation-a".to_string(),
        request_hash: "hash-a".to_string(),
        request_body: Vec::new(),
        runtime_record: Vec::new(),
        owner_node: format!("worker-{owner_generation}"),
        ownership_generation: owner_generation,
        attempts: vec![format!("attempt-{version}")],
        phase: StoredContinuityPhase::Completed,
        replica_set: vec!["replica-a".to_string()],
        created_at_millis: 1,
        updated_at_millis: version,
        terminal_at_millis: Some(version),
        expires_at_millis: Some(1_000 + version),
        response_metadata: Vec::new(),
        response_body: version.to_le_bytes().to_vec(),
        control_term: owner_generation,
        schema_version: 1,
        version,
    }
}

#[test]
fn model_property_continuity_merge_is_idempotent_and_converges_under_reordering() {
    let permutations = [
        [1_u64, 2, 3],
        [1, 3, 2],
        [2, 1, 3],
        [2, 3, 1],
        [3, 1, 2],
        [3, 2, 1],
    ];
    for versions in permutations {
        let store =
            SqliteContinuityStore::open(Path::new(":memory:"), ContinuityStoreLimits::default())
                .expect("store");
        for version in versions {
            let record = stored_record(version, version);
            store.upsert(&record).expect("merge");
            store.upsert(&record).expect("idempotent duplicate");
        }
        let final_record = store.get("operation-a").expect("lookup").expect("record");
        assert_eq!(final_record.version, 3);
        assert_eq!(final_record.ownership_generation, 3);
        assert_eq!(final_record.owner_node, "worker-3");
    }
}

#[test]
fn model_property_threshold_oscillation_is_bounded_by_cooldown() {
    let policy = ScalingPolicy {
        min_nodes: 2,
        max_nodes: 8,
        target_inflight_per_node: 10,
        scale_up_window_millis: 100,
        scale_down_window_millis: 200,
        cooldown_millis: 1_000,
        max_scale_up_step: 1,
        max_scale_down_step: 1,
        max_unavailable: 1,
    };
    let mut autoscaler = Autoscaler::new(policy).expect("valid oscillation policy");
    let start = Instant::now();
    let mut current = 2_u16;
    let mut mutations = 0_u64;
    for sample in 0..2_000_u64 {
        let inflight = if sample % 2 == 0 { 19 } else { 21 };
        let decision = autoscaler.evaluate(
            current,
            scaling_sample(
                start + Duration::from_millis(sample.saturating_mul(50)),
                inflight,
                current,
            ),
        );
        assert!((2..=8).contains(&decision.bounded_desired));
        if decision.bounded_desired != current {
            assert_eq!(current.abs_diff(decision.bounded_desired), 1);
            current = decision.bounded_desired;
            mutations = mutations.saturating_add(1);
        }
    }

    // 100 seconds of threshold chatter cannot produce more than one mutation
    // per 1-second cooldown (plus the initial boundary transition).
    assert!(
        mutations <= 101,
        "oscillation escaped cooldown: {mutations}"
    );
}
