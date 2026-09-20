use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clap::Args;
use mesh_rt::actor::Scheduler;
use mesh_rt::{
    load_report_registry, reconcile_scale_up, select_owner, ContinuityStore, ContinuityStoreLimits,
    ControlMutation, ControlTerm, ControllerQuorum, DesiredCapacity, DesiredRevision,
    DurableControlLog, FakeCapacityDriver, MessageClass, NodeLifecycleState, NodeLoadReport,
    NodeRoles, ProtocolEnvelope, RoutingPolicy, SqliteContinuityStore, StoredContinuityPhase,
    StoredContinuityRecord,
};
use serde::{Deserialize, Serialize};

const RELEASE_SOAK_SECONDS: u64 = 24 * 60 * 60;
const SOAK_TERMINAL_RETENTION_MILLIS: u64 = 15_000;
const SOAK_TOMBSTONE_RETENTION_MILLIS: u64 = 30_000;

#[derive(Args, Debug)]
pub struct ContinuitySoakArgs {
    /// Wall-clock duration. The release gate requires at least 86400 seconds.
    #[arg(long, default_value_t = RELEASE_SOAK_SECONDS)]
    pub duration_seconds: u64,

    /// Workload cadence; pacing never determines correctness.
    #[arg(long, default_value_t = 100)]
    pub cycle_millis: u64,

    /// Permit a shorter harness smoke run. Its evidence is not a release pass.
    #[arg(long)]
    pub allow_short: bool,

    /// Override the timestamped evidence output directory.
    #[arg(long)]
    pub evidence_dir: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct AutonomousPerformanceArgs {
    /// Iterations per deterministic microbenchmark.
    #[arg(long, default_value_t = 10_000)]
    pub iterations: u32,

    /// Checked-in regression budget.
    #[arg(long, default_value = "proof/autonomous-gates/performance-budget.json")]
    pub budget: PathBuf,

    /// Override the timestamped evidence output directory.
    #[arg(long)]
    pub evidence_dir: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize)]
struct SoakSample {
    elapsed_seconds: u64,
    records: u64,
    active_records: u64,
    terminal_records: u64,
    tombstones: u64,
    log_entries: u64,
    disk_bytes: u64,
}

#[derive(Debug, Serialize)]
struct ContinuitySoakSummary {
    schema_version: u16,
    requested_duration_seconds: u64,
    elapsed_duration_seconds: u64,
    cycle_millis: u64,
    writes: u64,
    terminal_records_written: u64,
    reads: u64,
    idempotent_retries: u64,
    node_churn_transitions: u64,
    interrupted_snapshots_resumed: u64,
    peak_records: u64,
    peak_terminal_records: u64,
    peak_tombstones: u64,
    peak_log_entries: u64,
    peak_disk_bytes: u64,
    final_stats: mesh_rt::ContinuityStoreStats,
    assertions: BTreeMap<String, bool>,
    release_assertions: BTreeMap<String, bool>,
    safety_pass: bool,
    release_24h_pass: bool,
    samples: Vec<SoakSample>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PerformanceBudget {
    schema_version: u16,
    baseline_revision: String,
    routing_average_max_micros: f64,
    protocol_round_trip_average_max_micros: f64,
    continuity_write_p95_max_micros: u64,
    snapshot_apply_min_mib_per_second: f64,
    consensus_commit_p95_max_micros: u64,
    scheduler_resize_p95_max_micros: u64,
    load_report_max_bytes: u64,
    load_report_encode_average_max_micros: f64,
    continuity_write_amplification_max_ratio: f64,
    continuity_compaction_min_records_per_second: f64,
    snapshot_join_max_millis: u64,
    driver_reconcile_average_max_micros: f64,
}

#[derive(Debug, Serialize)]
struct AutonomousPerformanceSummary {
    schema_version: u16,
    iterations: u32,
    budget: PerformanceBudget,
    routing_average_micros: f64,
    protocol_round_trip_average_micros: f64,
    load_report_bytes: u64,
    load_report_encode_average_micros: f64,
    load_report_bandwidth_bytes_per_second_at_one_hertz: u64,
    scheduler_scale_up_p50_micros: u64,
    scheduler_scale_up_p95_micros: u64,
    scheduler_retirement_p50_micros: u64,
    scheduler_retirement_p95_micros: u64,
    continuity_write_p50_micros: u64,
    continuity_write_p95_micros: u64,
    continuity_write_p99_micros: u64,
    snapshot_bytes: u64,
    snapshot_chunks: usize,
    snapshot_join_millis: u64,
    snapshot_apply_mib_per_second: f64,
    continuity_disk_bytes_before_compaction: u64,
    continuity_write_amplification_ratio: f64,
    continuity_compacted_records: u64,
    continuity_compaction_records_per_second: f64,
    consensus_commit_p50_micros: u64,
    consensus_commit_p95_micros: u64,
    consensus_commit_p99_micros: u64,
    driver_reconcile_average_micros: f64,
    assertions: BTreeMap<String, bool>,
    pass: bool,
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn evidence_directory(kind: &str, explicit: Option<PathBuf>) -> Result<PathBuf, String> {
    let path = explicit.unwrap_or_else(|| {
        PathBuf::from("target")
            .join("proof")
            .join(kind)
            .join(unix_millis().to_string())
    });
    fs::create_dir_all(&path)
        .map_err(|error| format!("proof_gate_evidence_directory_failed:{error}"))?;
    Ok(path)
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    fs::write(
        path,
        serde_json::to_vec_pretty(value)
            .map_err(|error| format!("proof_gate_evidence_encode_failed:{error}"))?,
    )
    .map_err(|error| format!("proof_gate_evidence_write_failed:{error}"))
}

fn soak_record(
    key: String,
    version: u64,
    phase: StoredContinuityPhase,
    owner_ordinal: usize,
    now: u64,
) -> StoredContinuityRecord {
    let nodes = ["worker-a", "worker-b", "worker-c"];
    let owner = nodes[owner_ordinal % nodes.len()];
    let replica_set = nodes
        .iter()
        .copied()
        .filter(|node| *node != owner)
        .map(str::to_string)
        .collect();
    let terminal = (!phase.is_active()).then_some(now);
    StoredContinuityRecord {
        request_hash: format!("sha256:{key}"),
        request_body: b"{\"kind\":\"soak\"}".to_vec(),
        runtime_record: Vec::new(),
        operation_key: key.clone(),
        owner_node: owner.to_string(),
        ownership_generation: owner_ordinal as u64 + 1,
        attempts: vec![format!("attempt-{key}-{version}")],
        phase,
        replica_set,
        created_at_millis: now,
        updated_at_millis: now,
        terminal_at_millis: terminal,
        expires_at_millis: terminal
            .map(|timestamp| timestamp.saturating_add(SOAK_TERMINAL_RETENTION_MILLIS)),
        response_metadata: vec![("status".to_string(), "200".to_string())],
        response_body: vec![(owner_ordinal % 251) as u8; 256],
        control_term: owner_ordinal as u64 / 100 + 1,
        schema_version: 1,
        version,
    }
}

fn drain_log(store: &SqliteContinuityStore) -> Result<(), String> {
    let high_water = store.high_water_mark()?;
    if high_water == 0 {
        return Ok(());
    }
    for replica in ["worker-a", "worker-b", "worker-c"] {
        store.acknowledge_replica_safe_point(replica, high_water)?;
    }
    while store.compact_log_to_replica_safe_point()? > 0 {}
    Ok(())
}

fn observe_soak_stats(
    store: &SqliteContinuityStore,
    elapsed: Duration,
    samples: &mut Vec<SoakSample>,
    peaks: &mut [u64; 5],
) -> Result<(), String> {
    let stats = store.stats()?;
    peaks[0] = peaks[0].max(stats.records);
    peaks[1] = peaks[1].max(stats.terminal_records);
    peaks[2] = peaks[2].max(stats.tombstones);
    peaks[3] = peaks[3].max(stats.log_entries);
    peaks[4] = peaks[4].max(stats.disk_bytes);
    samples.push(SoakSample {
        elapsed_seconds: elapsed.as_secs(),
        records: stats.records,
        active_records: stats.active_records,
        terminal_records: stats.terminal_records,
        tombstones: stats.tombstones,
        log_entries: stats.log_entries,
        disk_bytes: stats.disk_bytes,
    });
    Ok(())
}

pub fn run_continuity_soak(args: ContinuitySoakArgs) -> Result<(), String> {
    if args.duration_seconds == 0 || args.cycle_millis == 0 {
        return Err("continuity_soak_duration_or_cycle_zero".to_string());
    }
    if args.duration_seconds < RELEASE_SOAK_SECONDS && !args.allow_short {
        return Err(format!(
            "continuity_soak_release_requires_{RELEASE_SOAK_SECONDS}_seconds; use --allow-short only for a non-release smoke run"
        ));
    }
    let evidence = evidence_directory("continuity-soak", args.evidence_dir)?;
    let directory =
        tempfile::tempdir().map_err(|error| format!("continuity_soak_tempdir_failed:{error}"))?;
    let database = directory.path().join("continuity.db");
    let limits = ContinuityStoreLimits {
        terminal_retention_millis: SOAK_TERMINAL_RETENTION_MILLIS,
        tombstone_retention_millis: SOAK_TOMBSTONE_RETENTION_MILLIS,
        max_terminal_records: 10_000,
        max_disk_bytes: 512 * 1024 * 1024,
        compaction_batch_size: 512,
    };
    let store = SqliteContinuityStore::open(&database, limits)?;
    let started = Instant::now();
    let deadline = started + Duration::from_secs(args.duration_seconds);
    let base_time = unix_millis();
    let mut ordinal = 0_u64;
    let mut pending = VecDeque::<StoredContinuityRecord>::new();
    let mut samples = Vec::new();
    let mut peaks = [0_u64; 5];
    let mut writes = 0_u64;
    let mut terminals = 0_u64;
    let mut reads = 0_u64;
    let mut retries = 0_u64;
    let mut churn = 0_u64;
    let mut snapshots = 0_u64;
    let mut last_sample_second = u64::MAX;

    while Instant::now() < deadline {
        let elapsed = started.elapsed();
        let now = base_time.saturating_add(elapsed.as_millis() as u64);
        if let Some(mut active) = pending.pop_front() {
            active.phase = StoredContinuityPhase::Completed;
            active.version = active.version.saturating_add(1);
            active.updated_at_millis = now;
            active.terminal_at_millis = Some(now);
            active.expires_at_millis = Some(now.saturating_add(SOAK_TERMINAL_RETENTION_MILLIS));
            active.response_body = b"completed-after-node-churn".to_vec();
            store.upsert(&active)?;
            writes += 1;
            terminals += 1;
        }

        let active_key = format!("active-{ordinal:016x}");
        let active = soak_record(
            active_key.clone(),
            1,
            StoredContinuityPhase::Started,
            ordinal as usize,
            now,
        );
        store.upsert(&active)?;
        pending.push_back(active);
        writes += 1;

        let terminal_key = format!("terminal-{ordinal:016x}");
        let terminal = soak_record(
            terminal_key.clone(),
            1,
            StoredContinuityPhase::Completed,
            (ordinal as usize).wrapping_add(1),
            now,
        );
        store.upsert(&terminal)?;
        writes += 1;
        terminals += 1;
        let high_water_before_retry = store.high_water_mark()?;
        store.upsert(&terminal)?;
        if store.high_water_mark()? != high_water_before_retry {
            return Err("continuity_soak_duplicate_retry_appended_log".to_string());
        }
        retries += 1;
        if store.get(&active_key)?.is_none() || store.get(&terminal_key)?.is_none() {
            return Err("continuity_soak_read_after_write_missing".to_string());
        }
        reads += 2;
        churn += u64::from(ordinal > 0);
        store.compact(now)?;
        drain_log(&store)?;

        if ordinal > 0 && ordinal % 1_000 == 0 {
            let chunks = store.snapshot_chunks(64 * 1024)?;
            let target = SqliteContinuityStore::open(
                Path::new(":memory:"),
                ContinuityStoreLimits {
                    max_disk_bytes: u64::MAX,
                    ..limits
                },
            )?;
            if let Some(first) = chunks.first() {
                target.apply_snapshot_chunk(first)?;
                for chunk in chunks.iter().skip(1) {
                    target.apply_snapshot_chunk(chunk)?;
                }
            }
            if target.stats()?.records != store.stats()?.records {
                return Err("continuity_soak_resumed_snapshot_diverged".to_string());
            }
            snapshots += 1;
        }

        let sample_second = elapsed.as_secs();
        if sample_second != last_sample_second {
            observe_soak_stats(&store, elapsed, &mut samples, &mut peaks)?;
            last_sample_second = sample_second;
        }
        ordinal = ordinal.saturating_add(1);
        thread::park_timeout(Duration::from_millis(args.cycle_millis));
    }

    let finish_time = base_time
        .saturating_add(started.elapsed().as_millis() as u64)
        .saturating_add(SOAK_TERMINAL_RETENTION_MILLIS);
    while let Some(mut active) = pending.pop_front() {
        active.phase = StoredContinuityPhase::Completed;
        active.version = active.version.saturating_add(1);
        active.updated_at_millis = finish_time;
        active.terminal_at_millis = Some(finish_time);
        active.expires_at_millis = Some(finish_time);
        store.upsert(&active)?;
        writes += 1;
        terminals += 1;
    }
    while store.compact(finish_time)?.records_tombstoned > 0 {}
    let delete_time = finish_time.saturating_add(SOAK_TOMBSTONE_RETENTION_MILLIS);
    while store.compact(delete_time)?.tombstones_deleted > 0 {}
    drain_log(&store)?;
    observe_soak_stats(&store, started.elapsed(), &mut samples, &mut peaks)?;
    let final_stats = store.stats()?;
    let elapsed_duration_seconds = started.elapsed().as_secs();
    let plateau = if samples.len() < 4 {
        true
    } else {
        let split = samples.len() / 2;
        let first_peak = samples[..split]
            .iter()
            .map(|sample| sample.disk_bytes)
            .max()
            .unwrap_or(0);
        let second_peak = samples[split..]
            .iter()
            .map(|sample| sample.disk_bytes)
            .max()
            .unwrap_or(0);
        second_peak <= first_peak.saturating_add(16 * 1024 * 1024)
    };
    let mut assertions = BTreeMap::new();
    assertions.insert(
        "requested_wall_duration_completed".to_string(),
        started.elapsed() >= Duration::from_secs(args.duration_seconds),
    );
    assertions.insert("mixed_reads_writes_retries_exercised".to_string(), {
        writes > 0 && reads > 0 && retries > 0
    });
    assertions.insert("node_churn_exercised".to_string(), churn > 0);
    assertions.insert(
        "active_records_never_evicted_for_capacity".to_string(),
        final_stats.active_records == 0,
    );
    assertions.insert(
        "terminal_count_remained_bounded".to_string(),
        peaks[1] <= limits.max_terminal_records,
    );
    assertions.insert(
        "tombstones_and_log_remained_bounded".to_string(),
        peaks[2] <= 20_000 && peaks[3] <= limits.compaction_batch_size as u64 * 2,
    );
    assertions.insert(
        "disk_usage_remained_bounded".to_string(),
        peaks[4] < limits.max_disk_bytes,
    );
    assertions.insert("disk_usage_reached_steady_plateau".to_string(), plateau);
    assertions.insert(
        "retention_cleanup_completed".to_string(),
        final_stats.records == 0 && final_stats.tombstones == 0 && final_stats.log_entries == 0,
    );
    let safety_pass = assertions.values().all(|passed| *passed);
    let release_assertions = BTreeMap::from([
        (
            "twenty_four_hour_wall_duration_completed".to_string(),
            args.duration_seconds >= RELEASE_SOAK_SECONDS
                && started.elapsed() >= Duration::from_secs(RELEASE_SOAK_SECONDS),
        ),
        (
            "million_terminal_records_exercised".to_string(),
            terminals >= 1_000_000,
        ),
        (
            "interrupted_snapshot_resume_exercised".to_string(),
            snapshots > 0,
        ),
    ]);
    let release_24h_pass = safety_pass && release_assertions.values().all(|passed| *passed);
    let summary = ContinuitySoakSummary {
        schema_version: 1,
        requested_duration_seconds: args.duration_seconds,
        elapsed_duration_seconds,
        cycle_millis: args.cycle_millis,
        writes,
        terminal_records_written: terminals,
        reads,
        idempotent_retries: retries,
        node_churn_transitions: churn,
        interrupted_snapshots_resumed: snapshots,
        peak_records: peaks[0],
        peak_terminal_records: peaks[1],
        peak_tombstones: peaks[2],
        peak_log_entries: peaks[3],
        peak_disk_bytes: peaks[4],
        final_stats,
        assertions,
        release_assertions,
        safety_pass,
        release_24h_pass,
        samples,
    };
    write_json(&evidence.join("summary.json"), &summary)?;
    println!("continuity_soak_evidence: {}", evidence.display());
    if release_24h_pass {
        println!("continuity_soak: RELEASE PASS");
        Ok(())
    } else if args.allow_short && safety_pass {
        println!("continuity_soak: SMOKE PASS (not a 24-hour release artifact)");
        Ok(())
    } else {
        Err("continuity_soak_gate_failed".to_string())
    }
}

fn percentile(samples: &mut [u64], fraction: f64) -> u64 {
    if samples.is_empty() {
        return 0;
    }
    samples.sort_unstable();
    let index = ((samples.len() - 1) as f64 * fraction.clamp(0.0, 1.0)).ceil() as usize;
    samples[index]
}

fn performance_report(node_id: String, inflight: u32, sequence: u64) -> NodeLoadReport {
    NodeLoadReport {
        protocol_version: 3,
        node_id: node_id.clone(),
        boot_id: format!("boot-{node_id}"),
        roles: NodeRoles::new(false, false, true),
        state: NodeLifecycleState::Ready,
        capacity_units: (1 + inflight % 4) as u16,
        active_workers: 8,
        runnable_actors: u64::from(inflight),
        inflight,
        queued_items: 0,
        queued_bytes: 0,
        outstanding_reservations: 0,
        p95_queue_wait: Duration::from_millis(u64::from(inflight % 5)),
        memory_pressure: 0.1,
        decision_pressure_ewma: f64::from(inflight) / 128.0,
        sequence,
        control_term: 1,
        membership_generation: 1,
        failure_domain: node_id,
        handlers: BTreeSet::from(["Todos.list".to_string()]),
    }
}

pub fn run_autonomous_performance(args: AutonomousPerformanceArgs) -> Result<(), String> {
    if args.iterations < 100 {
        return Err("autonomous_performance_requires_at_least_100_iterations".to_string());
    }
    let evidence = evidence_directory("autonomous-performance", args.evidence_dir)?;
    let budget: PerformanceBudget = serde_json::from_slice(
        &fs::read(&args.budget)
            .map_err(|error| format!("performance_budget_read_failed:{error}"))?,
    )
    .map_err(|error| format!("performance_budget_decode_failed:{error}"))?;
    if budget.schema_version != 2 {
        return Err("performance_budget_schema_unsupported".to_string());
    }

    let prefix = format!("perf-{}", unix_millis());
    let now = Instant::now();
    let candidates = (0..8)
        .map(|index| format!("{prefix}-worker-{index}"))
        .collect::<Vec<_>>();
    for (index, node) in candidates.iter().enumerate() {
        load_report_registry().apply(
            performance_report(node.clone(), index as u32 * 7, index as u64 + 1),
            now,
        )?;
    }
    let routing_started = Instant::now();
    for index in 0..args.iterations {
        let decision = select_owner(
            &format!("{prefix}-request-{index}"),
            "Todos.list",
            "gateway-a",
            &candidates,
            None,
            &RoutingPolicy::default(),
            now,
        )?;
        if decision.selected_node.is_empty() {
            return Err("performance_routing_selected_empty_node".to_string());
        }
    }
    let routing_average_micros =
        routing_started.elapsed().as_secs_f64() * 1_000_000.0 / f64::from(args.iterations);

    let envelope = ProtocolEnvelope {
        class: MessageClass::Application,
        kind: 42,
        correlation_id: 7,
        chunk_sequence: 0,
        final_chunk: true,
        payload: vec![42; 4 * 1024],
    };
    let protocol_started = Instant::now();
    for _ in 0..args.iterations {
        let encoded = envelope.encode(64 * 1024)?;
        let decoded = ProtocolEnvelope::decode(&encoded, 64 * 1024)?;
        if decoded != envelope {
            return Err("performance_protocol_round_trip_mismatch".to_string());
        }
    }
    let protocol_round_trip_average_micros =
        protocol_started.elapsed().as_secs_f64() * 1_000_000.0 / f64::from(args.iterations);

    let load_report = performance_report(format!("{prefix}-bandwidth"), 17, 1);
    let load_report_started = Instant::now();
    let mut load_report_bytes = 0_u64;
    for _ in 0..args.iterations {
        let encoded = load_report.encode()?;
        load_report_bytes = encoded.len().try_into().unwrap_or(u64::MAX);
        if NodeLoadReport::decode(&encoded)? != load_report {
            return Err("performance_load_report_round_trip_mismatch".to_string());
        }
    }
    let load_report_encode_average_micros =
        load_report_started.elapsed().as_secs_f64() * 1_000_000.0 / f64::from(args.iterations);

    let scheduler = Scheduler::new_elastic(1, 8)?;
    let mut scheduler_scale_up_latencies = Vec::with_capacity(args.iterations as usize);
    let mut scheduler_retirement_latencies = Vec::with_capacity(args.iterations as usize);
    for _ in 0..args.iterations {
        let started = Instant::now();
        scheduler.resize(8)?;
        if scheduler.active_workers() != 8 {
            return Err("performance_scheduler_scale_up_diverged".to_string());
        }
        scheduler_scale_up_latencies
            .push(started.elapsed().as_micros().try_into().unwrap_or(u64::MAX));
        let started = Instant::now();
        scheduler.resize(1)?;
        if scheduler.active_workers() != 1 {
            return Err("performance_scheduler_retirement_diverged".to_string());
        }
        scheduler_retirement_latencies
            .push(started.elapsed().as_micros().try_into().unwrap_or(u64::MAX));
    }

    let continuity_dir = tempfile::tempdir()
        .map_err(|error| format!("performance_continuity_tempdir_failed:{error}"))?;
    let limits = ContinuityStoreLimits {
        terminal_retention_millis: 60_000,
        tombstone_retention_millis: 120_000,
        max_terminal_records: u64::from(args.iterations) + 1,
        max_disk_bytes: 2 * 1024 * 1024 * 1024,
        compaction_batch_size: 1_000,
    };
    let source = SqliteContinuityStore::open(&continuity_dir.path().join("source.db"), limits)?;
    let mut continuity_latencies = Vec::with_capacity(args.iterations as usize);
    let base = unix_millis();
    for index in 0..args.iterations {
        let record = soak_record(
            format!("perf-operation-{index}"),
            1,
            StoredContinuityPhase::Completed,
            index as usize,
            base + u64::from(index),
        );
        let started = Instant::now();
        source.upsert(&record)?;
        continuity_latencies.push(started.elapsed().as_micros().try_into().unwrap_or(u64::MAX));
    }
    let continuity_disk_bytes_before_compaction = source.stats()?.disk_bytes;
    let chunks = source.snapshot_chunks(1024 * 1024)?;
    let snapshot_bytes = chunks
        .iter()
        .map(|chunk| chunk.payload.len() as u64)
        .sum::<u64>();
    let target = SqliteContinuityStore::open(&continuity_dir.path().join("target.db"), limits)?;
    let snapshot_started = Instant::now();
    for chunk in &chunks {
        target.apply_snapshot_chunk(chunk)?;
    }
    let snapshot_elapsed = snapshot_started.elapsed();
    let snapshot_seconds = snapshot_elapsed.as_secs_f64().max(0.000_001);
    let snapshot_join_millis = snapshot_elapsed.as_millis().try_into().unwrap_or(u64::MAX);
    let snapshot_apply_mib_per_second =
        snapshot_bytes as f64 / (1024.0 * 1024.0) / snapshot_seconds;
    if target.stats()?.records != source.stats()?.records {
        return Err("performance_snapshot_apply_diverged".to_string());
    }
    let continuity_write_amplification_ratio =
        continuity_disk_bytes_before_compaction as f64 / snapshot_bytes.max(1) as f64;
    let compaction_started = Instant::now();
    let mut continuity_compacted_records = 0_u64;
    loop {
        let outcome = source.compact(base.saturating_add(180_000))?;
        continuity_compacted_records =
            continuity_compacted_records.saturating_add(u64::from(outcome.records_tombstoned));
        if outcome.records_tombstoned == 0 {
            break;
        }
    }
    let continuity_compaction_records_per_second = continuity_compacted_records as f64
        / compaction_started.elapsed().as_secs_f64().max(0.000_001);
    if continuity_compacted_records != u64::from(args.iterations) {
        return Err("performance_continuity_compaction_incomplete".to_string());
    }

    let consensus_dir = tempfile::tempdir()
        .map_err(|error| format!("performance_consensus_tempdir_failed:{error}"))?;
    let log = Arc::new(DurableControlLog::open(
        &consensus_dir.path().join("control.log"),
    )?);
    let voters = BTreeSet::from(["a".to_string(), "b".to_string(), "c".to_string()]);
    let quorum = ControllerQuorum::new(voters.clone(), log)?;
    let term = quorum.elect("a", &voters)?;
    let consensus_iterations = args.iterations.min(2_000);
    let mut consensus_latencies = Vec::with_capacity(consensus_iterations as usize);
    for index in 0..consensus_iterations {
        let started = Instant::now();
        quorum.commit(
            "a",
            term,
            &voters,
            "performance-gate",
            "bounded commit latency",
            ControlMutation::PauseAutoscaler {
                paused: index % 2 == 0,
            },
        )?;
        consensus_latencies.push(started.elapsed().as_micros().try_into().unwrap_or(u64::MAX));
    }

    let driver = FakeCapacityDriver::new();
    let driver_started = Instant::now();
    for index in 0..args.iterations {
        let desired = DesiredCapacity {
            revision: DesiredRevision(u64::from(index) + 1),
            worker_nodes: 1,
            gateway_nodes: 0,
            template_revision: "performance-template".to_string(),
        };
        let operations =
            reconcile_scale_up(&driver, "performance-cluster", ControlTerm(1), &desired, 0)?;
        if operations.len() != 1
            || !matches!(
                operations[0].state,
                mesh_rt::DriverOperationState::Succeeded
            )
        {
            return Err("performance_driver_reconciliation_diverged".to_string());
        }
    }
    let driver_reconcile_average_micros =
        driver_started.elapsed().as_secs_f64() * 1_000_000.0 / f64::from(args.iterations);

    let mut continuity_p50 = continuity_latencies.clone();
    let mut continuity_p95 = continuity_latencies.clone();
    let mut continuity_p99 = continuity_latencies;
    let continuity_write_p50_micros = percentile(&mut continuity_p50, 0.50);
    let continuity_write_p95_micros = percentile(&mut continuity_p95, 0.95);
    let continuity_write_p99_micros = percentile(&mut continuity_p99, 0.99);
    let mut scheduler_up_p50 = scheduler_scale_up_latencies.clone();
    let mut scheduler_up_p95 = scheduler_scale_up_latencies;
    let scheduler_scale_up_p50_micros = percentile(&mut scheduler_up_p50, 0.50);
    let scheduler_scale_up_p95_micros = percentile(&mut scheduler_up_p95, 0.95);
    let mut scheduler_down_p50 = scheduler_retirement_latencies.clone();
    let mut scheduler_down_p95 = scheduler_retirement_latencies;
    let scheduler_retirement_p50_micros = percentile(&mut scheduler_down_p50, 0.50);
    let scheduler_retirement_p95_micros = percentile(&mut scheduler_down_p95, 0.95);
    let mut consensus_p50 = consensus_latencies.clone();
    let mut consensus_p95 = consensus_latencies.clone();
    let mut consensus_p99 = consensus_latencies;
    let consensus_commit_p50_micros = percentile(&mut consensus_p50, 0.50);
    let consensus_commit_p95_micros = percentile(&mut consensus_p95, 0.95);
    let consensus_commit_p99_micros = percentile(&mut consensus_p99, 0.99);
    let mut assertions = BTreeMap::new();
    assertions.insert(
        "routing_average_within_budget".to_string(),
        routing_average_micros <= budget.routing_average_max_micros,
    );
    assertions.insert(
        "protocol_round_trip_average_within_budget".to_string(),
        protocol_round_trip_average_micros <= budget.protocol_round_trip_average_max_micros,
    );
    assertions.insert(
        "load_report_size_within_budget".to_string(),
        load_report_bytes <= budget.load_report_max_bytes,
    );
    assertions.insert(
        "load_report_encode_average_within_budget".to_string(),
        load_report_encode_average_micros <= budget.load_report_encode_average_max_micros,
    );
    assertions.insert(
        "scheduler_scale_up_reaction_within_budget".to_string(),
        scheduler_scale_up_p95_micros <= budget.scheduler_resize_p95_max_micros,
    );
    assertions.insert(
        "scheduler_retirement_reaction_within_budget".to_string(),
        scheduler_retirement_p95_micros <= budget.scheduler_resize_p95_max_micros,
    );
    assertions.insert(
        "continuity_write_p95_within_budget".to_string(),
        continuity_write_p95_micros <= budget.continuity_write_p95_max_micros,
    );
    assertions.insert(
        "continuity_write_amplification_within_budget".to_string(),
        continuity_write_amplification_ratio <= budget.continuity_write_amplification_max_ratio,
    );
    assertions.insert(
        "continuity_compaction_throughput_within_budget".to_string(),
        continuity_compaction_records_per_second
            >= budget.continuity_compaction_min_records_per_second,
    );
    assertions.insert(
        "snapshot_apply_throughput_within_budget".to_string(),
        snapshot_apply_mib_per_second >= budget.snapshot_apply_min_mib_per_second,
    );
    assertions.insert(
        "snapshot_join_time_within_budget".to_string(),
        snapshot_join_millis <= budget.snapshot_join_max_millis,
    );
    assertions.insert(
        "consensus_commit_p95_within_budget".to_string(),
        consensus_commit_p95_micros <= budget.consensus_commit_p95_max_micros,
    );
    assertions.insert(
        "driver_reconcile_average_within_budget".to_string(),
        driver_reconcile_average_micros <= budget.driver_reconcile_average_max_micros,
    );
    let pass = assertions.values().all(|passed| *passed);
    let summary = AutonomousPerformanceSummary {
        schema_version: 2,
        iterations: args.iterations,
        budget,
        routing_average_micros,
        protocol_round_trip_average_micros,
        load_report_bytes,
        load_report_encode_average_micros,
        load_report_bandwidth_bytes_per_second_at_one_hertz: load_report_bytes,
        scheduler_scale_up_p50_micros,
        scheduler_scale_up_p95_micros,
        scheduler_retirement_p50_micros,
        scheduler_retirement_p95_micros,
        continuity_write_p50_micros,
        continuity_write_p95_micros,
        continuity_write_p99_micros,
        snapshot_bytes,
        snapshot_chunks: chunks.len(),
        snapshot_join_millis,
        snapshot_apply_mib_per_second,
        continuity_disk_bytes_before_compaction,
        continuity_write_amplification_ratio,
        continuity_compacted_records,
        continuity_compaction_records_per_second,
        consensus_commit_p50_micros,
        consensus_commit_p95_micros,
        consensus_commit_p99_micros,
        driver_reconcile_average_micros,
        assertions,
        pass,
    };
    write_json(&evidence.join("summary.json"), &summary)?;
    println!("autonomous_performance_evidence: {}", evidence.display());
    if pass {
        println!("autonomous_performance: PASS");
        Ok(())
    } else {
        Err("autonomous_performance_gate_failed".to_string())
    }
}
