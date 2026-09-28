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
    ContinuityRecord, OperatorControlAction, OperatorControlRequest, OperatorDiagnosticsSnapshot,
    OperatorRuntimeSnapshot, DEFAULT_OPERATOR_QUERY_TIMEOUT,
};
use serde_json::json;

#[derive(Subcommand, Debug)]
pub enum ClusterCommand {
    /// Show runtime-owned membership and authority for a clustered node.
    Status(ClusterRuntimeArgs),
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
    /// Show ordered control, scaling, and continuity events (the same log as `diagnostics`).
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
    #[command(flatten)]
    pub query: ClusterQueryArgs,

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
}

/// How every cluster command reaches its node and reports.
#[derive(Args, Debug)]
pub struct ClusterQueryArgs {
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
pub struct ClusterRuntimeArgs {
    /// Cluster node to inspect (name@host:port)
    pub target: String,

    #[command(flatten)]
    pub query: ClusterQueryArgs,
}

#[derive(Args, Debug)]
pub struct ClusterExplainArgs {
    /// Cluster node to inspect (name@host:port)
    pub target: String,

    /// Retained continuity request or operation key
    pub request_key: String,

    #[command(flatten)]
    pub query: ClusterQueryArgs,
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

    #[command(flatten)]
    pub query: ClusterQueryArgs,
}

#[derive(Args, Debug)]
pub struct ClusterDiagnosticsArgs {
    /// Cluster node to inspect (name@host:port)
    pub target: String,

    /// Max diagnostic entries to return
    #[arg(long)]
    pub limit: Option<usize>,

    #[command(flatten)]
    pub query: ClusterQueryArgs,
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
    let cookie = cluster_cookie(authorization.query.cookie_file.as_deref())?;
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
    let outcome = query_operator_control_remote(
        &target,
        &cookie,
        request,
        timeout(authorization.query.timeout_ms),
    )
    .map_err(|error| error.to_string())?;
    if authorization.query.json {
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
    let cookie = cluster_cookie(args.query.cookie_file.as_deref())?;
    query_operator_runtime_remote(&args.target, &cookie, timeout(args.query.timeout_ms))
        .map_err(|error| error.to_string())
}

fn run_snapshot(args: ClusterRuntimeArgs) -> Result<(), String> {
    let snapshot = runtime_snapshot(&args)?;
    if args.query.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&snapshot)
                .expect("serialize complete cluster runtime snapshot")
        );
        return Ok(());
    }
    print_lines(snapshot_lines(&args.target, &snapshot));
    Ok(())
}

fn print_lines(lines: Vec<String>) {
    for line in lines {
        println!("{line}");
    }
}

/// `meshc cluster snapshot` as text.
fn snapshot_lines(target: &str, snapshot: &OperatorRuntimeSnapshot) -> Vec<String> {
    let mut lines = vec![
        format!("target: {target}"),
        format!("local_node: {}", snapshot.local_node),
        format!("telemetry_complete: {}", snapshot.telemetry_complete),
        format!(
            "capacity: desired={} observed={} ready={} draining={} min={} max={}",
            snapshot.desired_capacity,
            snapshot.observed_capacity,
            snapshot.ready_capacity,
            snapshot.draining_capacity,
            snapshot.scheduler_min_workers,
            snapshot.scheduler_max_workers,
        ),
        format!("autoscaler_paused: {}", snapshot.autoscaler_paused),
    ];
    lines.push(match &snapshot.consensus {
        Some(consensus) => format!(
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
        ),
        None => "consensus: disabled".to_string(),
    });
    lines.push(format!(
        "autonomous: configured={} running={} leader={} state={} desired_workers={}",
        snapshot.autonomous.configured,
        snapshot.autonomous.running,
        snapshot.autonomous.leader,
        snapshot.autonomous.state,
        snapshot.autonomous.desired_workers,
    ));
    if snapshot.nodes.is_empty() {
        lines.push("nodes: (none)".to_string());
    } else {
        lines.push("nodes:".to_string());
        lines.extend(snapshot.nodes.iter().map(|node| {
            format!(
                "- node={} roles={} state={} eligible={} pressure={:.3} inflight={} queued={}",
                node.node_id,
                node.roles.join(","),
                node.state,
                node.routing_eligible,
                node.pressure,
                node.inflight,
                node.queued_items,
            )
        }));
    }
    lines
}

fn run_capacity(args: ClusterRuntimeArgs) -> Result<(), String> {
    let snapshot = runtime_snapshot(&args)?;
    if args.query.json {
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
    if args.query.json {
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
        print_lines(pressure_lines(&args.target, &snapshot));
    }
    Ok(())
}

/// `meshc cluster pressure` as text.
fn pressure_lines(target: &str, snapshot: &OperatorRuntimeSnapshot) -> Vec<String> {
    let mut lines = vec![
        format!("target: {target}"),
        format!("telemetry_complete: {}", snapshot.telemetry_complete),
        format!(
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
        ),
    ];
    if snapshot.nodes.is_empty() {
        lines.push("nodes: (missing telemetry)".to_string());
    } else {
        lines.push("nodes:".to_string());
        lines.extend(snapshot.nodes.iter().map(|node| {
            format!(
                "- node={} pressure={:.3} dominant_signal={} inflight={} queued={} runnable={}",
                node.node_id,
                node.pressure,
                node.dominant_signal,
                node.inflight,
                node.queued_items,
                node.runnable_actors
            )
        }));
    }
    lines
}

fn run_routing(args: ClusterRuntimeArgs) -> Result<(), String> {
    let snapshot = runtime_snapshot(&args)?;
    if args.query.json {
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
    if args.query.json {
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
        print_lines(scaling_lines(&args.target, &snapshot));
    }
    Ok(())
}

/// `meshc cluster scaling` as text.
fn scaling_lines(target: &str, snapshot: &OperatorRuntimeSnapshot) -> Vec<String> {
    let mut lines = vec![
        format!("target: {target}"),
        format!("autoscaler_paused: {}", snapshot.autoscaler_paused),
        format!("desired_capacity: {}", snapshot.desired_capacity),
        format!("scheduler_min_workers: {}", snapshot.scheduler_min_workers),
        format!("scheduler_max_workers: {}", snapshot.scheduler_max_workers),
        format!(
            "scheduler_active_workers: {}",
            snapshot.scheduler_active_workers
        ),
        format!(
            "scheduler_run_queues: global={} workers={:?}",
            snapshot.local_telemetry.global_run_queue_depth,
            snapshot.local_telemetry.worker_run_queue_depths,
        ),
        format!(
            "scheduler_time: busy_ms={} idle_ms={} mailbox_messages={} mailbox_p95={}",
            snapshot.local_telemetry.scheduler_busy_time.as_millis(),
            snapshot.local_telemetry.scheduler_idle_time.as_millis(),
            snapshot.local_telemetry.mailbox_messages,
            snapshot.local_telemetry.mailbox_depth_p95,
        ),
    ];
    lines.push(
        match (
            &snapshot.local_continuity_store,
            &snapshot.local_continuity_store_error,
        ) {
            (Some(store), _) => format!(
                "continuity_store: active={} terminal={} disk_bytes={} compaction_lag={} replication_lag={}",
                store.active_records,
                store.terminal_records,
                store.disk_bytes,
                store.compaction_lag,
                store
                    .replication_lag
                    .map_or_else(|| "unavailable".to_string(), |lag| lag.to_string()),
            ),
            (None, Some(error)) => format!("continuity_store: error={error}"),
            (None, None) => "continuity_store: disabled".to_string(),
        },
    );
    lines.extend(
        snapshot
            .local_telemetry
            .capacity_driver_operations
            .iter()
            .map(|operation| {
                format!(
                    "capacity_driver: operation={} count={} errors={} p95_latency_ms={}",
                    operation.operation,
                    operation.count,
                    operation.errors,
                    operation.p95_latency.as_millis(),
                )
            }),
    );
    lines
}

fn run_explain(args: ClusterExplainArgs) -> Result<(), String> {
    let cookie = cluster_cookie(args.query.cookie_file.as_deref())?;
    let query_timeout = timeout(args.query.timeout_ms);
    let record = query_operator_continuity_status_remote(
        &args.target,
        &cookie,
        &args.request_key,
        query_timeout,
    )
    .map_err(|error| error.to_string())?;
    let runtime = query_operator_runtime_remote(&args.target, &cookie, query_timeout)
        .map_err(|error| error.to_string())?;
    if args.query.json {
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

fn run_status(args: ClusterRuntimeArgs) -> Result<(), String> {
    let cookie = cluster_cookie(args.query.cookie_file.as_deref())?;
    let timeout = timeout(args.query.timeout_ms);
    let snapshot = query_operator_status_remote(&args.target, &cookie, timeout)
        .map_err(|error| error.to_string())?;

    if args.query.json {
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

    let cookie = cluster_cookie(args.query.cookie_file.as_deref())?;
    let timeout = timeout(args.query.timeout_ms);

    if let Some(request_key) = args.request_key.as_deref() {
        let record =
            query_operator_continuity_status_remote(&args.target, &cookie, request_key, timeout)
                .map_err(|error| error.to_string())?;
        if args.query.json {
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
    if args.query.json {
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
    let cookie = cluster_cookie(args.query.cookie_file.as_deref())?;
    let timeout = timeout(args.query.timeout_ms);
    let snapshot = query_operator_diagnostics_remote(&args.target, &cookie, args.limit, timeout)
        .map_err(|error| error.to_string())?;

    if args.query.json {
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
    print_lines(diagnostics_lines(&args.target, &snapshot));
    Ok(())
}

/// `meshc cluster diagnostics` as text.
fn diagnostics_lines(target: &str, snapshot: &OperatorDiagnosticsSnapshot) -> Vec<String> {
    let mut lines = vec![
        format!("target: {target}"),
        format!("total_entries: {}", snapshot.total_entries),
        format!("dropped_entries: {}", snapshot.dropped_entries),
        format!("buffer_capacity: {}", snapshot.buffer_capacity),
        format!("truncated: {}", snapshot.truncated),
    ];
    if snapshot.entries.is_empty() {
        lines.push("entries: (none)".to_string());
        return lines;
    }

    lines.push("entries:".to_string());
    for entry in &snapshot.entries {
        lines.push(format!(
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
        ));
        lines.extend(
            entry
                .metadata
                .iter()
                .map(|(key, value)| format!("    {key}={value}")),
        );
    }
    lines
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

    use super::*;

    /// A secret file as the commands read one: owner-only.
    fn secret(contents: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().expect("create secret file");
        file.write_all(contents.as_bytes())
            .expect("write secret file");
        file
    }

    #[test]
    fn the_text_forms_say_what_a_node_does_not_report() {
        let mut snapshot: OperatorRuntimeSnapshot = serde_json::from_value(json!({
            "schema_version": 1, "local_node": "a@127.0.0.1:1", "telemetry_complete": false,
            "desired_capacity": 0, "observed_capacity": 0, "ready_capacity": 0,
            "draining_capacity": 0, "autoscaler_paused": false,
            "scheduler_min_workers": 1, "scheduler_max_workers": 1,
            "scheduler_active_workers": 1, "nodes": [],
        }))
        .expect("a runtime snapshot");
        snapshot.local_continuity_store_error = Some("disk full".to_string());
        let has = |lines: Vec<String>, line: &str| lines.iter().any(|shown| shown == line);
        assert!(has(snapshot_lines("a", &snapshot), "nodes: (none)"));
        assert!(has(
            pressure_lines("a", &snapshot),
            "nodes: (missing telemetry)"
        ));
        assert!(has(
            scaling_lines("a", &snapshot),
            "continuity_store: error=disk full"
        ));
        let diagnostics = OperatorDiagnosticsSnapshot {
            entries: Vec::new(),
            total_entries: 0,
            dropped_entries: 0,
            buffer_capacity: 16,
            truncated: false,
        };
        assert!(has(diagnostics_lines("a", &diagnostics), "entries: (none)"));

        // A node with consensus and a continuity store says what they hold.
        snapshot.consensus = Some(mesh_rt::ConsensusRuntimeSnapshot {
            node_id: 1,
            node_name: "a".to_string(),
            state: "leader".to_string(),
            current_term: 2,
            current_leader: None,
            last_applied_log: None,
            voter_ids: vec![1],
            entries: Vec::new(),
        });
        snapshot.local_continuity_store = Some(mesh_rt::ContinuityStoreStats {
            records: 3,
            active_records: 1,
            terminal_records: 2,
            tombstones: 0,
            log_entries: 0,
            high_water_mark: 0,
            disk_bytes: 4096,
            replica_safe_point: None,
            compaction_lag: 0,
            replication_lag: None,
        });
        assert!(has(
            snapshot_lines("a", &snapshot),
            "consensus: state=leader term=2 leader=(none) applied=(none) voters=[1]"
        ));
        assert!(has(
            scaling_lines("a", &snapshot),
            "continuity_store: active=1 terminal=2 disk_bytes=4096 compaction_lag=0 replication_lag=unavailable"
        ));
    }

    /// A key shorter than 32 bytes signs nothing, and nothing is sent.
    #[test]
    fn a_control_request_needs_a_signing_key() {
        let cookie = secret("cookie\n");
        let key = secret("too-short\n");
        let authorization = ClusterControlAuthorization {
            query: ClusterQueryArgs {
                cookie_file: Some(cookie.path().to_path_buf()),
                timeout_ms: 100,
                json: false,
            },
            operator_key_file: Some(key.path().to_path_buf()),
            cluster_id: "mesh".to_string(),
            actor: "meshc".to_string(),
            reason: "test".to_string(),
            sequence: None,
        };
        assert_eq!(
            run_control(
                "nobody@127.0.0.1:1".to_string(),
                authorization,
                OperatorControlAction::PauseAutoscaler,
            ),
            Err("operator_control_key_missing".to_string())
        );
    }

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
