use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use clap::{Args, Subcommand};
use mesh_rt::{
    query_operator_continuity_list_remote, query_operator_continuity_status_remote,
    query_operator_control_remote, query_operator_diagnostics_remote,
    query_operator_runtime_remote, query_operator_status_remote, sign_operator_control_request,
    ContinuityRecord, OperatorControlAction, OperatorControlRequest, OperatorRuntimeSnapshot,
    DEFAULT_OPERATOR_QUERY_TIMEOUT,
};
use serde_json::json;

#[derive(Subcommand, Debug)]
pub enum ClusterCommand {
    /// Show runtime-owned membership and authority for a clustered node.
    Status(ClusterStatusArgs),
    /// Show the complete runtime-owned operator snapshot for a clustered node.
    Snapshot(ClusterRuntimeArgs),
    /// Show runtime-owned continuity status for one request key, or list recent continuity records.
    Continuity(ClusterContinuityArgs),
    /// Show recent runtime-owned failover and continuity diagnostics.
    Diagnostics(ClusterDiagnosticsArgs),
    /// Show desired, observed, Ready, and draining capacity.
    Capacity(ClusterRuntimeArgs),
    /// Show cluster and per-node pressure with dominant signals.
    Pressure(ClusterRuntimeArgs),
    /// Show routing eligibility, load reports, and reservations.
    Routing(ClusterRuntimeArgs),
    /// Show scheduler and horizontal scaling state.
    Scaling(ClusterRuntimeArgs),
    /// Show ordered control, scaling, and continuity events.
    Events(ClusterDiagnosticsArgs),
    /// Explain the retained placement and current candidate state for a request.
    Explain(ClusterExplainArgs),
    /// Pause or resume autonomous capacity changes.
    Autoscale {
        #[command(subcommand)]
        action: AutoscaleCommand,
    },
    /// Set an authenticated manual desired-capacity override.
    Scale(ClusterScaleArgs),
    /// Begin graceful drain for a node.
    Drain(ClusterDrainArgs),
    /// Cancel a previously requested node drain.
    CancelDrain(ClusterDrainArgs),
}

#[derive(Subcommand, Debug)]
pub enum AutoscaleCommand {
    Pause(ClusterControlTargetArgs),
    Resume(ClusterControlTargetArgs),
}

#[derive(Args, Debug)]
pub struct ClusterControlTargetArgs {
    /// Cluster node receiving the control request (name@host:port)
    pub target: String,

    #[command(flatten)]
    pub authorization: ClusterControlAuthorization,
}

#[derive(Args, Debug)]
pub struct ClusterScaleArgs {
    /// Cluster node receiving the control request (name@host:port)
    pub target: String,

    /// Desired number of worker nodes
    pub worker_nodes: u16,

    #[command(flatten)]
    pub authorization: ClusterControlAuthorization,
}

#[derive(Args, Debug)]
pub struct ClusterDrainArgs {
    /// Cluster node receiving the control request (name@host:port)
    pub target: String,

    /// Stable node identity to drain
    pub node_id: String,

    #[command(flatten)]
    pub authorization: ClusterControlAuthorization,
}

#[derive(Args, Debug)]
pub struct ClusterControlAuthorization {
    /// Owner-only file containing the shared data-plane cookie (defaults to MESH_CLUSTER_COOKIE)
    #[arg(long, value_name = "PATH")]
    pub cookie_file: Option<PathBuf>,

    /// Owner-only file containing the operator signing key (defaults to MESH_OPERATOR_KEY)
    #[arg(long, value_name = "PATH")]
    pub operator_key_file: Option<PathBuf>,

    /// Stable cluster identity
    #[arg(long, default_value = "mesh")]
    pub cluster_id: String,

    /// Audited operator identity
    #[arg(long, default_value = "meshc")]
    pub actor: String,

    /// Audited reason for the control change
    #[arg(long, default_value = "operator request")]
    pub reason: String,

    /// Explicit monotonic sequence for automation; defaults to current microseconds
    #[arg(long)]
    pub sequence: Option<u64>,

    /// Query timeout in milliseconds
    #[arg(long, default_value_t = DEFAULT_OPERATOR_QUERY_TIMEOUT.as_millis() as u64)]
    pub timeout_ms: u64,

    /// Emit JSON instead of human-readable output
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ClusterRuntimeArgs {
    /// Cluster node to inspect (name@host:port)
    pub target: String,

    /// Owner-only file containing the shared cluster cookie (defaults to MESH_CLUSTER_COOKIE)
    #[arg(long, value_name = "PATH")]
    pub cookie_file: Option<PathBuf>,

    /// Query timeout in milliseconds
    #[arg(long, default_value_t = DEFAULT_OPERATOR_QUERY_TIMEOUT.as_millis() as u64)]
    pub timeout_ms: u64,

    /// Emit JSON instead of human-readable output
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ClusterExplainArgs {
    /// Cluster node to inspect (name@host:port)
    pub target: String,

    /// Retained continuity request or operation key
    pub request_key: String,

    /// Owner-only file containing the shared cluster cookie (defaults to MESH_CLUSTER_COOKIE)
    #[arg(long, value_name = "PATH")]
    pub cookie_file: Option<PathBuf>,

    /// Query timeout in milliseconds
    #[arg(long, default_value_t = DEFAULT_OPERATOR_QUERY_TIMEOUT.as_millis() as u64)]
    pub timeout_ms: u64,

    /// Emit JSON instead of human-readable output
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ClusterStatusArgs {
    /// Cluster node to inspect (name@host:port)
    pub target: String,

    /// Owner-only file containing the shared cluster cookie (defaults to MESH_CLUSTER_COOKIE)
    #[arg(long, value_name = "PATH")]
    pub cookie_file: Option<PathBuf>,

    /// Query timeout in milliseconds
    #[arg(long, default_value_t = DEFAULT_OPERATOR_QUERY_TIMEOUT.as_millis() as u64)]
    pub timeout_ms: u64,

    /// Emit JSON instead of human-readable output
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ClusterContinuityArgs {
    /// Cluster node to inspect (name@host:port)
    pub target: String,

    /// Optional request key. When omitted, lists recent continuity records.
    pub request_key: Option<String>,

    /// Max records to return when listing continuity records.
    #[arg(long)]
    pub limit: Option<usize>,

    /// Owner-only file containing the shared cluster cookie (defaults to MESH_CLUSTER_COOKIE)
    #[arg(long, value_name = "PATH")]
    pub cookie_file: Option<PathBuf>,

    /// Query timeout in milliseconds
    #[arg(long, default_value_t = DEFAULT_OPERATOR_QUERY_TIMEOUT.as_millis() as u64)]
    pub timeout_ms: u64,

    /// Emit JSON instead of human-readable output
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ClusterDiagnosticsArgs {
    /// Cluster node to inspect (name@host:port)
    pub target: String,

    /// Max diagnostic entries to return
    #[arg(long)]
    pub limit: Option<usize>,

    /// Owner-only file containing the shared cluster cookie (defaults to MESH_CLUSTER_COOKIE)
    #[arg(long, value_name = "PATH")]
    pub cookie_file: Option<PathBuf>,

    /// Query timeout in milliseconds
    #[arg(long, default_value_t = DEFAULT_OPERATOR_QUERY_TIMEOUT.as_millis() as u64)]
    pub timeout_ms: u64,

    /// Emit JSON instead of human-readable output
    #[arg(long)]
    pub json: bool,
}

pub fn run_cluster_command(command: ClusterCommand) -> Result<(), String> {
    match command {
        ClusterCommand::Status(args) => run_status(args),
        ClusterCommand::Snapshot(args) => run_snapshot(args),
        ClusterCommand::Continuity(args) => run_continuity(args),
        ClusterCommand::Diagnostics(args) => run_diagnostics(args),
        ClusterCommand::Capacity(args) => run_capacity(args),
        ClusterCommand::Pressure(args) => run_pressure(args),
        ClusterCommand::Routing(args) => run_routing(args),
        ClusterCommand::Scaling(args) => run_scaling(args),
        ClusterCommand::Events(args) => run_diagnostics(args),
        ClusterCommand::Explain(args) => run_explain(args),
        ClusterCommand::Autoscale { action } => match action {
            AutoscaleCommand::Pause(args) => run_control(
                args.target,
                args.authorization,
                OperatorControlAction::PauseAutoscaler,
            ),
            AutoscaleCommand::Resume(args) => run_control(
                args.target,
                args.authorization,
                OperatorControlAction::ResumeAutoscaler,
            ),
        },
        ClusterCommand::Scale(args) => run_control(
            args.target,
            args.authorization,
            OperatorControlAction::SetDesiredCapacity {
                worker_nodes: args.worker_nodes,
            },
        ),
        ClusterCommand::Drain(args) => run_control(
            args.target,
            args.authorization,
            OperatorControlAction::DrainNode {
                node_id: args.node_id,
            },
        ),
        ClusterCommand::CancelDrain(args) => run_control(
            args.target,
            args.authorization,
            OperatorControlAction::CancelDrain {
                node_id: args.node_id,
            },
        ),
    }
}

fn run_control(
    target: String,
    authorization: ClusterControlAuthorization,
    action: OperatorControlAction,
) -> Result<(), String> {
    let cookie = cluster_cookie(authorization.cookie_file.as_deref())?;
    let operator_key = secret_from_file_or_env(
        authorization.operator_key_file.as_deref(),
        "MESH_OPERATOR_KEY",
        "operator key",
        "--operator-key-file",
    )?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let sequence = authorization
        .sequence
        .unwrap_or_else(|| now.as_micros().try_into().unwrap_or(u64::MAX));
    let request = sign_operator_control_request(
        OperatorControlRequest {
            schema_version: 1,
            cluster_id: authorization.cluster_id,
            actor: authorization.actor,
            sequence,
            expires_at_unix_millis: now
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX)
                .saturating_add(30_000),
            reason: authorization.reason,
            action,
            signature: String::new(),
        },
        &operator_key,
    )?;
    let outcome =
        query_operator_control_remote(&target, &cookie, request, timeout(authorization.timeout_ms))
            .map_err(|error| error.to_string())?;
    if authorization.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&outcome).expect("serialize cluster control outcome json")
        );
    } else {
        println!("target: {target}");
        println!("accepted: {}", outcome.accepted);
        println!("control_sequence: {}", outcome.control_sequence);
        println!("autoscaler_paused: {}", outcome.autoscaler_paused);
        println!(
            "desired_capacity_override: {}",
            outcome
                .desired_capacity_override
                .map(|value| value.to_string())
                .unwrap_or_else(|| "(none)".to_string())
        );
        println!("drain_intents: {}", outcome.drain_intents.join(","));
    }
    Ok(())
}

fn runtime_snapshot(args: &ClusterRuntimeArgs) -> Result<OperatorRuntimeSnapshot, String> {
    let cookie = cluster_cookie(args.cookie_file.as_deref())?;
    query_operator_runtime_remote(&args.target, &cookie, timeout(args.timeout_ms))
        .map_err(|error| error.to_string())
}

fn run_snapshot(args: ClusterRuntimeArgs) -> Result<(), String> {
    let snapshot = runtime_snapshot(&args)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&snapshot)
                .expect("serialize complete cluster runtime snapshot")
        );
        return Ok(());
    }

    println!("target: {}", args.target);
    println!("local_node: {}", snapshot.local_node);
    println!("telemetry_complete: {}", snapshot.telemetry_complete);
    println!(
        "capacity: desired={} observed={} ready={} draining={} min={} max={}",
        snapshot.desired_capacity,
        snapshot.observed_capacity,
        snapshot.ready_capacity,
        snapshot.draining_capacity,
        snapshot.scheduler_min_workers,
        snapshot.scheduler_max_workers,
    );
    println!("autoscaler_paused: {}", snapshot.autoscaler_paused);
    if let Some(consensus) = &snapshot.consensus {
        println!(
            "consensus: state={} term={} leader={} applied={} voters={:?}",
            consensus.state,
            consensus.current_term,
            consensus
                .current_leader
                .map_or_else(|| "(none)".to_string(), |leader| leader.to_string()),
            consensus
                .last_applied_log
                .map_or_else(|| "(none)".to_string(), |index| index.to_string()),
            consensus.voter_ids,
        );
    } else {
        println!("consensus: disabled");
    }
    println!(
        "autonomous: configured={} running={} leader={} state={} desired_workers={}",
        snapshot.autonomous.configured,
        snapshot.autonomous.running,
        snapshot.autonomous.leader,
        snapshot.autonomous.state,
        snapshot.autonomous.desired_workers,
    );
    if snapshot.nodes.is_empty() {
        println!("nodes: (none)");
    } else {
        println!("nodes:");
        for node in snapshot.nodes {
            println!(
                "- node={} roles={} state={} eligible={} pressure={:.3} inflight={} queued={}",
                node.node_id,
                node.roles.join(","),
                node.state,
                node.routing_eligible,
                node.pressure,
                node.inflight,
                node.queued_items,
            );
        }
    }
    Ok(())
}

fn run_capacity(args: ClusterRuntimeArgs) -> Result<(), String> {
    let snapshot = runtime_snapshot(&args)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "schema_version": snapshot.schema_version,
                "target": args.target,
                "desired": snapshot.desired_capacity,
                "observed": snapshot.observed_capacity,
                "ready": snapshot.ready_capacity,
                "draining": snapshot.draining_capacity,
                "pending": snapshot.desired_capacity.saturating_sub(snapshot.observed_capacity),
                "telemetry_complete": snapshot.telemetry_complete,
            }))
            .expect("serialize cluster capacity json")
        );
    } else {
        println!("target: {}", args.target);
        println!("desired: {}", snapshot.desired_capacity);
        println!("observed: {}", snapshot.observed_capacity);
        println!("ready: {}", snapshot.ready_capacity);
        println!("draining: {}", snapshot.draining_capacity);
        println!(
            "pending: {}",
            snapshot
                .desired_capacity
                .saturating_sub(snapshot.observed_capacity)
        );
        println!("telemetry_complete: {}", snapshot.telemetry_complete);
    }
    Ok(())
}

fn run_pressure(args: ClusterRuntimeArgs) -> Result<(), String> {
    let snapshot = runtime_snapshot(&args)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "schema_version": snapshot.schema_version,
                "target": args.target,
                "telemetry_complete": snapshot.telemetry_complete,
                "local_telemetry": &snapshot.local_telemetry,
                "nodes": &snapshot.nodes,
            }))
            .expect("serialize cluster pressure json")
        );
    } else {
        println!("target: {}", args.target);
        println!("telemetry_complete: {}", snapshot.telemetry_complete);
        println!(
            "local: workers={}/{} runnable={} inflight={} queued={} rejected={} p95_queue_wait_ms={} p95_service_ms={} p95_end_to_end_ms={} rss_bytes={} cpu_available={}",
            snapshot.local_telemetry.active_workers,
            snapshot.local_telemetry.configured_workers,
            snapshot.local_telemetry.runnable_actors,
            snapshot.local_telemetry.inflight_requests,
            snapshot.local_telemetry.queued_requests,
            snapshot.local_telemetry.rejected_requests,
            snapshot.local_telemetry.p95_queue_wait.as_millis(),
            snapshot.local_telemetry.p95_service_time.as_millis(),
            snapshot.local_telemetry.p95_end_to_end_time.as_millis(),
            snapshot
                .local_telemetry
                .process_resident_memory_bytes
                .map_or_else(|| "unavailable".to_string(), |bytes| bytes.to_string()),
            snapshot.local_telemetry.cpu_available_parallelism,
        );
        if snapshot.nodes.is_empty() {
            println!("nodes: (missing telemetry)");
        } else {
            println!("nodes:");
            for node in snapshot.nodes {
                println!(
                    "- node={} pressure={:.3} dominant_signal={} inflight={} queued={} runnable={}",
                    node.node_id,
                    node.pressure,
                    node.dominant_signal,
                    node.inflight,
                    node.queued_items,
                    node.runnable_actors
                );
            }
        }
    }
    Ok(())
}

fn run_routing(args: ClusterRuntimeArgs) -> Result<(), String> {
    let snapshot = runtime_snapshot(&args)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "schema_version": snapshot.schema_version,
                "target": args.target,
                "telemetry_complete": snapshot.telemetry_complete,
                "remote_dispatch": {
                    "queued_items": snapshot.local_telemetry.remote_dispatch_queued_items,
                    "queued_bytes": snapshot.local_telemetry.remote_dispatch_queued_bytes,
                    "timeouts": snapshot.local_telemetry.remote_dispatch_timeouts,
                    "retries": snapshot.local_telemetry.remote_dispatch_retries,
                    "circuit_rejections": snapshot.local_telemetry.remote_dispatch_circuit_rejections,
                    "queue_rejections": snapshot.local_telemetry.remote_dispatch_queue_rejections,
                    "open_circuits": snapshot.local_telemetry.remote_dispatch_open_circuits,
                },
                "peer_sessions": &snapshot.local_peer_sessions,
                "candidates": &snapshot.nodes,
            }))
            .expect("serialize cluster routing json")
        );
    } else {
        println!("target: {}", args.target);
        println!(
            "remote_dispatch: queued_items={} queued_bytes={} timeouts={} retries={} circuit_rejections={} queue_rejections={} open_circuits={}",
            snapshot.local_telemetry.remote_dispatch_queued_items,
            snapshot.local_telemetry.remote_dispatch_queued_bytes,
            snapshot.local_telemetry.remote_dispatch_timeouts,
            snapshot.local_telemetry.remote_dispatch_retries,
            snapshot.local_telemetry.remote_dispatch_circuit_rejections,
            snapshot.local_telemetry.remote_dispatch_queue_rejections,
            snapshot.local_telemetry.remote_dispatch_open_circuits,
        );
        for session in &snapshot.local_peer_sessions {
            let max_utilization = session
                .lanes
                .iter()
                .map(|lane| lane.utilization)
                .fold(0.0_f64, f64::max);
            println!(
                "peer: node={} healthy={} circuit={} send_buffer_utilization={:.3}",
                session.peer, session.healthy, session.circuit_state, max_utilization
            );
        }
        for node in snapshot.nodes {
            println!(
                "- node={} state={} routing_eligible={} capacity_units={} reservations={} report_sequence={} generation={}",
                node.node_id,
                node.state,
                node.routing_eligible,
                node.capacity_units,
                node.reservations,
                node.report_sequence,
                node.membership_generation
            );
        }
    }
    Ok(())
}

fn run_scaling(args: ClusterRuntimeArgs) -> Result<(), String> {
    let snapshot = runtime_snapshot(&args)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "schema_version": snapshot.schema_version,
                "target": args.target,
                "autoscaler_paused": snapshot.autoscaler_paused,
                "desired_capacity": snapshot.desired_capacity,
                "scheduler": {
                    "minimum": snapshot.scheduler_min_workers,
                    "maximum": snapshot.scheduler_max_workers,
                    "active": snapshot.scheduler_active_workers,
                },
                "local_telemetry": &snapshot.local_telemetry,
                "continuity_store": &snapshot.local_continuity_store,
                "continuity_store_error": &snapshot.local_continuity_store_error,
            }))
            .expect("serialize cluster scaling json")
        );
    } else {
        println!("target: {}", args.target);
        println!("autoscaler_paused: {}", snapshot.autoscaler_paused);
        println!("desired_capacity: {}", snapshot.desired_capacity);
        println!("scheduler_min_workers: {}", snapshot.scheduler_min_workers);
        println!("scheduler_max_workers: {}", snapshot.scheduler_max_workers);
        println!(
            "scheduler_active_workers: {}",
            snapshot.scheduler_active_workers
        );
        println!(
            "scheduler_run_queues: global={} workers={:?}",
            snapshot.local_telemetry.global_run_queue_depth,
            snapshot.local_telemetry.worker_run_queue_depths,
        );
        println!(
            "scheduler_time: busy_ms={} idle_ms={} mailbox_messages={} mailbox_p95={}",
            snapshot.local_telemetry.scheduler_busy_time.as_millis(),
            snapshot.local_telemetry.scheduler_idle_time.as_millis(),
            snapshot.local_telemetry.mailbox_messages,
            snapshot.local_telemetry.mailbox_depth_p95,
        );
        if let Some(store) = &snapshot.local_continuity_store {
            println!(
                "continuity_store: active={} terminal={} disk_bytes={} compaction_lag={} replication_lag={}",
                store.active_records,
                store.terminal_records,
                store.disk_bytes,
                store.compaction_lag,
                store
                    .replication_lag
                    .map_or_else(|| "unavailable".to_string(), |lag| lag.to_string()),
            );
        } else if let Some(error) = &snapshot.local_continuity_store_error {
            println!("continuity_store: error={error}");
        } else {
            println!("continuity_store: disabled");
        }
        for operation in &snapshot.local_telemetry.capacity_driver_operations {
            println!(
                "capacity_driver: operation={} count={} errors={} p95_latency_ms={}",
                operation.operation,
                operation.count,
                operation.errors,
                operation.p95_latency.as_millis(),
            );
        }
    }
    Ok(())
}

fn run_explain(args: ClusterExplainArgs) -> Result<(), String> {
    let cookie = cluster_cookie(args.cookie_file.as_deref())?;
    let query_timeout = timeout(args.timeout_ms);
    let record = query_operator_continuity_status_remote(
        &args.target,
        &cookie,
        &args.request_key,
        query_timeout,
    )
    .map_err(|error| error.to_string())?;
    let runtime = query_operator_runtime_remote(&args.target, &cookie, query_timeout)
        .map_err(|error| error.to_string())?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "schema_version": 1,
                "target": args.target,
                "record": continuity_record_json(&record),
                "current_candidates": runtime.nodes,
            }))
            .expect("serialize cluster explain json")
        );
    } else {
        print_continuity_record(&record);
        println!("current_candidates:");
        for node in runtime.nodes {
            println!(
                "- node={} eligible={} state={} pressure={:.3} dominant_signal={}",
                node.node_id,
                node.routing_eligible,
                node.state,
                node.pressure,
                node.dominant_signal
            );
        }
    }
    Ok(())
}

fn run_status(args: ClusterStatusArgs) -> Result<(), String> {
    let cookie = cluster_cookie(args.cookie_file.as_deref())?;
    let timeout = timeout(args.timeout_ms);
    let snapshot = query_operator_status_remote(&args.target, &cookie, timeout)
        .map_err(|error| error.to_string())?;

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "target": args.target,
                "membership": {
                    "local_node": snapshot.membership.local_node,
                    "peer_nodes": snapshot.membership.peer_nodes,
                    "nodes": snapshot.membership.nodes,
                },
                "authority": {
                    "cluster_role": snapshot.authority.cluster_role,
                    "promotion_epoch": snapshot.authority.promotion_epoch,
                    "replication_health": snapshot.authority.replication_health,
                },
            }))
            .expect("serialize cluster status json")
        );
        return Ok(());
    }

    println!("target: {}", args.target);
    println!("local_node: {}", snapshot.membership.local_node);
    println!("peer_nodes:");
    if snapshot.membership.peer_nodes.is_empty() {
        println!("  - (none)");
    } else {
        for peer in &snapshot.membership.peer_nodes {
            println!("  - {}", peer);
        }
    }
    println!("nodes:");
    for node in &snapshot.membership.nodes {
        println!("  - {}", node);
    }
    println!("cluster_role: {}", snapshot.authority.cluster_role);
    println!("promotion_epoch: {}", snapshot.authority.promotion_epoch);
    println!(
        "replication_health: {}",
        snapshot.authority.replication_health
    );
    Ok(())
}

fn run_continuity(args: ClusterContinuityArgs) -> Result<(), String> {
    if args.request_key.is_some() && args.limit.is_some() {
        return Err(
            "meshc cluster continuity does not accept --limit when request_key is provided"
                .to_string(),
        );
    }

    let cookie = cluster_cookie(args.cookie_file.as_deref())?;
    let timeout = timeout(args.timeout_ms);

    if let Some(request_key) = args.request_key.as_deref() {
        let record =
            query_operator_continuity_status_remote(&args.target, &cookie, request_key, timeout)
                .map_err(|error| error.to_string())?;
        if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "target": args.target,
                    "record": continuity_record_json(&record),
                }))
                .expect("serialize cluster continuity json")
            );
            return Ok(());
        }

        print_continuity_record(&record);
        return Ok(());
    }

    let list = query_operator_continuity_list_remote(&args.target, &cookie, args.limit, timeout)
        .map_err(|error| error.to_string())?;
    if args.json {
        let records: Vec<_> = list.records.iter().map(continuity_record_json).collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "target": args.target,
                "total_records": list.total_records,
                "truncated": list.truncated,
                "records": records,
            }))
            .expect("serialize cluster continuity list json")
        );
        return Ok(());
    }

    println!("target: {}", args.target);
    println!("total_records: {}", list.total_records);
    println!("truncated: {}", list.truncated);
    if list.records.is_empty() {
        println!("records: (none)");
        return Ok(());
    }

    println!("records:");
    for record in &list.records {
        println!(
            "- request_key={} attempt_id={} phase={} result={} owner={} replica={} replication_count={} execution={} declared_handler_runtime_name={} replica_status={} cluster_role={} promotion_epoch={} replication_health={} error={}",
            record.request_key,
            record.attempt_id,
            record.phase.as_str(),
            record.result.as_str(),
            record.owner_node,
            record.replica_node,
            record.replication_count,
            record.execution_node,
            record.declared_handler_runtime_name(),
            record.replica_status.as_str(),
            record.cluster_role.as_str(),
            record.promotion_epoch,
            record.replication_health.as_str(),
            record.error,
        );
    }
    Ok(())
}

fn run_diagnostics(args: ClusterDiagnosticsArgs) -> Result<(), String> {
    let cookie = cluster_cookie(args.cookie_file.as_deref())?;
    let timeout = timeout(args.timeout_ms);
    let snapshot = query_operator_diagnostics_remote(&args.target, &cookie, args.limit, timeout)
        .map_err(|error| error.to_string())?;

    if args.json {
        let entries: Vec<_> = snapshot
            .entries
            .iter()
            .map(|entry| {
                json!({
                    "sequence": entry.sequence,
                    "transition": entry.transition,
                    "request_key": entry.request_key,
                    "attempt_id": entry.attempt_id,
                    "owner_node": entry.owner_node,
                    "replica_node": entry.replica_node,
                    "execution_node": entry.execution_node,
                    "cluster_role": entry.cluster_role,
                    "promotion_epoch": entry.promotion_epoch,
                    "replication_health": entry.replication_health,
                    "replica_status": entry.replica_status,
                    "reason": entry.reason,
                    "metadata": entry
                        .metadata
                        .iter()
                        .map(|(key, value)| json!({"key": key, "value": value}))
                        .collect::<Vec<_>>(),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "target": args.target,
                "total_entries": snapshot.total_entries,
                "dropped_entries": snapshot.dropped_entries,
                "buffer_capacity": snapshot.buffer_capacity,
                "truncated": snapshot.truncated,
                "entries": entries,
            }))
            .expect("serialize cluster diagnostics json")
        );
        return Ok(());
    }

    println!("target: {}", args.target);
    println!("total_entries: {}", snapshot.total_entries);
    println!("dropped_entries: {}", snapshot.dropped_entries);
    println!("buffer_capacity: {}", snapshot.buffer_capacity);
    println!("truncated: {}", snapshot.truncated);
    if snapshot.entries.is_empty() {
        println!("entries: (none)");
        return Ok(());
    }

    println!("entries:");
    for entry in &snapshot.entries {
        println!(
            "- seq={} transition={} request_key={} attempt_id={} owner={} replica={} execution={} cluster_role={} promotion_epoch={} replication_health={} replica_status={} reason={}",
            entry.sequence,
            entry.transition,
            entry.request_key.as_deref().unwrap_or(""),
            entry.attempt_id.as_deref().unwrap_or(""),
            entry.owner_node.as_deref().unwrap_or(""),
            entry.replica_node.as_deref().unwrap_or(""),
            entry.execution_node.as_deref().unwrap_or(""),
            entry.cluster_role.as_deref().unwrap_or(""),
            entry
                .promotion_epoch
                .map(|value| value.to_string())
                .unwrap_or_default(),
            entry.replication_health.as_deref().unwrap_or(""),
            entry.replica_status.as_deref().unwrap_or(""),
            entry.reason.as_deref().unwrap_or(""),
        );
        for (key, value) in &entry.metadata {
            println!("    {}={}", key, value);
        }
    }
    Ok(())
}

fn cluster_cookie(file: Option<&Path>) -> Result<String, String> {
    secret_from_file_or_env(
        file,
        "MESH_CLUSTER_COOKIE",
        "cluster cookie",
        "--cookie-file",
    )
}

fn secret_from_file_or_env(
    file: Option<&Path>,
    environment_variable: &str,
    label: &str,
    file_flag: &str,
) -> Result<String, String> {
    if let Some(path) = file {
        return read_secret_file(path, label);
    }

    match std::env::var(environment_variable) {
        Ok(value) if !value.trim().is_empty() => Ok(value.trim().to_string()),
        Ok(_) => Err(format!(
            "meshc cluster: {environment_variable} must not be blank"
        )),
        Err(_) => Err(format!(
            "meshc cluster: {environment_variable} is required unless {file_flag} is provided"
        )),
    }
}

fn read_secret_file(path: &Path, label: &str) -> Result<String, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "meshc cluster: cannot inspect {label} file {}: {error}",
            path.display()
        )
    })?;
    if !metadata.file_type().is_file() {
        return Err(format!(
            "meshc cluster: {label} path {} must be a regular file",
            path.display()
        ));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(format!(
                "meshc cluster: {label} file {} must not be readable or writable by group or others (use mode 0600)",
                path.display()
            ));
        }
    }

    let value = fs::read_to_string(path).map_err(|error| {
        format!(
            "meshc cluster: cannot read {label} file {}: {error}",
            path.display()
        )
    })?;
    let value = value.trim();
    if value.is_empty() {
        return Err(format!(
            "meshc cluster: {label} file {} must not be blank",
            path.display()
        ));
    }
    Ok(value.to_string())
}

fn timeout(timeout_ms: u64) -> Duration {
    Duration::from_millis(timeout_ms.max(1))
}

fn continuity_record_json(record: &ContinuityRecord) -> serde_json::Value {
    json!({
        "request_key": record.request_key,
        "payload_hash": record.payload_hash,
        "attempt_id": record.attempt_id,
        "phase": record.phase.as_str(),
        "result": record.result.as_str(),
        "ingress_node": record.ingress_node,
        "owner_node": record.owner_node,
        "replica_node": record.replica_node,
        "replication_count": record.replication_count,
        "replica_status": record.replica_status.as_str(),
        "cluster_role": record.cluster_role.as_str(),
        "promotion_epoch": record.promotion_epoch,
        "replication_health": record.replication_health.as_str(),
        "execution_node": record.execution_node,
        "declared_handler_runtime_name": record.declared_handler_runtime_name(),
        "routed_remotely": record.routed_remotely,
        "fell_back_locally": record.fell_back_locally,
        "error": record.error,
    })
}

fn print_continuity_record(record: &ContinuityRecord) {
    println!("request_key: {}", record.request_key);
    println!("attempt_id: {}", record.attempt_id);
    println!("phase: {}", record.phase.as_str());
    println!("result: {}", record.result.as_str());
    println!("ingress_node: {}", record.ingress_node);
    println!("owner_node: {}", record.owner_node);
    println!("replica_node: {}", record.replica_node);
    println!("replication_count: {}", record.replication_count);
    println!("execution_node: {}", record.execution_node);
    println!(
        "declared_handler_runtime_name: {}",
        record.declared_handler_runtime_name()
    );
    println!("replica_status: {}", record.replica_status.as_str());
    println!("cluster_role: {}", record.cluster_role.as_str());
    println!("promotion_epoch: {}", record.promotion_epoch);
    println!("replication_health: {}", record.replication_health.as_str());
    println!("routed_remotely: {}", record.routed_remotely);
    println!("fell_back_locally: {}", record.fell_back_locally);
    println!("error: {}", record.error);
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::read_secret_file;

    #[test]
    fn secret_file_trims_line_endings() {
        let mut file = NamedTempFile::new().expect("create secret file");
        file.write_all(b"rotating-key-old,rotating-key-new\n")
            .expect("write secret file");

        assert_eq!(
            read_secret_file(file.path(), "test secret").expect("read secret file"),
            "rotating-key-old,rotating-key-new"
        );
    }

    #[cfg(unix)]
    #[test]
    fn secret_file_rejects_group_or_other_access() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let mut file = NamedTempFile::new().expect("create secret file");
        file.write_all(b"secret\n").expect("write secret file");
        fs::set_permissions(file.path(), fs::Permissions::from_mode(0o640))
            .expect("set insecure permissions");

        let error = read_secret_file(file.path(), "test secret")
            .expect_err("insecure permissions must fail closed");
        assert!(error.contains("use mode 0600"), "unexpected error: {error}");
    }
}
