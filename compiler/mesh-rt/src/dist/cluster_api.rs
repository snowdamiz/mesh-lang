//! Read-only application-facing cluster introspection APIs.

use crate::collections::map;
use crate::string::mesh_string_new;

fn mesh_string(value: &str) -> u64 {
    mesh_string_new(value.as_ptr(), value.len() as u64) as u64
}

fn put_int(mut result: *mut u8, key: &str, value: u64) -> *mut u8 {
    result = map::mesh_map_put(result, mesh_string(key), value.min(i64::MAX as u64));
    result
}

fn put_string(mut result: *mut u8, key: &str, value: &str) -> *mut u8 {
    result = map::mesh_map_put(result, mesh_string(key), mesh_string(value));
    result
}

fn duration_nanos(duration: std::time::Duration) -> u64 {
    duration.as_nanos().min(u64::MAX.into()) as u64
}

/// Return bounded local and cluster capacity counters as `Map<String, Int>`.
#[no_mangle]
pub extern "C" fn mesh_cluster_capacity() -> *mut u8 {
    let telemetry = super::telemetry::runtime_telemetry().snapshot();
    let (minimum, maximum) = crate::actor::GLOBAL_SCHEDULER
        .get()
        .map(|scheduler| scheduler.worker_bounds())
        .unwrap_or((
            telemetry.active_workers as usize,
            telemetry.active_workers as usize,
        ));
    let runtime = super::operator::operator_runtime_snapshot().ok();
    let mut result = map::mesh_map_new_typed(1);
    result = put_int(
        result,
        "local_active_workers",
        telemetry.active_workers.into(),
    );
    result = put_int(result, "local_min_workers", minimum as u64);
    result = put_int(result, "local_max_workers", maximum as u64);
    result = put_int(
        result,
        "desired_nodes",
        runtime
            .as_ref()
            .map_or(1, |snapshot| snapshot.desired_capacity)
            .into(),
    );
    result = put_int(
        result,
        "observed_nodes",
        runtime
            .as_ref()
            .map_or(1, |snapshot| snapshot.observed_capacity)
            .into(),
    );
    result = put_int(
        result,
        "ready_nodes",
        runtime
            .as_ref()
            .map_or(1, |snapshot| snapshot.ready_capacity)
            .into(),
    );
    put_int(
        result,
        "draining_nodes",
        runtime
            .as_ref()
            .map_or(0, |snapshot| snapshot.draining_capacity)
            .into(),
    )
}

/// Return normalized local pressure and its dominant component as `Map<String, String>`.
#[no_mangle]
pub extern "C" fn mesh_cluster_pressure() -> *mut u8 {
    let telemetry = super::telemetry::runtime_telemetry().snapshot();
    let pressure = super::telemetry::PressureSnapshot::calculate(
        telemetry.inflight_requests,
        128,
        telemetry.p95_queue_wait,
        std::time::Duration::from_millis(25),
        telemetry.runnable_actors,
        telemetry.active_workers,
        0.0,
    );
    let mut result = map::mesh_map_new_typed(1);
    result = put_string(result, "score", &format!("{:.6}", pressure.score));
    result = put_string(result, "dominant_signal", pressure.dominant_signal);
    put_string(
        result,
        "telemetry_complete",
        if super::operator::operator_runtime_snapshot()
            .is_ok_and(|snapshot| snapshot.telemetry_complete)
        {
            "true"
        } else {
            "false"
        },
    )
}

/// Return fixed-name local runtime counters and gauges as `Map<String, Int>`.
#[no_mangle]
pub extern "C" fn mesh_cluster_telemetry() -> *mut u8 {
    let telemetry = super::telemetry::runtime_telemetry().snapshot();
    let mut result = map::mesh_map_new_typed(1);
    for (key, value) in [
        ("active_workers", telemetry.active_workers.into()),
        ("configured_workers", telemetry.configured_workers.into()),
        ("runnable_actors", telemetry.runnable_actors),
        (
            "global_run_queue_depth",
            telemetry.global_run_queue_depth.into(),
        ),
        (
            "scheduler_busy_nanoseconds_total",
            duration_nanos(telemetry.scheduler_busy_time),
        ),
        (
            "scheduler_idle_nanoseconds_total",
            duration_nanos(telemetry.scheduler_idle_time),
        ),
        ("mailbox_messages", telemetry.mailbox_messages),
        ("mailbox_depth_p50", telemetry.mailbox_depth_p50),
        ("mailbox_depth_p95", telemetry.mailbox_depth_p95),
        ("mailbox_depth_max", telemetry.mailbox_depth_max),
        ("http_connections", telemetry.http_connections.into()),
        ("http_inflight_requests", telemetry.inflight_requests.into()),
        ("http_queued_requests", telemetry.queued_requests.into()),
        ("http_queued_bytes", telemetry.queued_bytes),
        ("http_rejected_requests_total", telemetry.rejected_requests),
        (
            "http_outstanding_reservations",
            telemetry.outstanding_reservations.into(),
        ),
        (
            "http_queue_wait_p95_nanoseconds",
            duration_nanos(telemetry.p95_queue_wait),
        ),
        (
            "http_service_p95_nanoseconds",
            duration_nanos(telemetry.p95_service_time),
        ),
        (
            "http_end_to_end_p95_nanoseconds",
            duration_nanos(telemetry.p95_end_to_end_time),
        ),
        (
            "remote_dispatch_queued_items",
            telemetry.remote_dispatch_queued_items.into(),
        ),
        (
            "remote_dispatch_queued_bytes",
            telemetry.remote_dispatch_queued_bytes,
        ),
        (
            "remote_dispatch_timeouts_total",
            telemetry.remote_dispatch_timeouts,
        ),
        (
            "remote_dispatch_retries_total",
            telemetry.remote_dispatch_retries,
        ),
        (
            "remote_dispatch_circuit_rejections_total",
            telemetry.remote_dispatch_circuit_rejections,
        ),
        (
            "remote_dispatch_queue_rejections_total",
            telemetry.remote_dispatch_queue_rejections,
        ),
        (
            "remote_dispatch_open_circuits",
            telemetry.remote_dispatch_open_circuits.into(),
        ),
        (
            "process_resident_memory_bytes",
            telemetry.process_resident_memory_bytes.unwrap_or(0),
        ),
        (
            "cpu_available_parallelism",
            telemetry.cpu_available_parallelism.into(),
        ),
    ] {
        result = put_int(result, key, value);
    }
    result
}

/// Return the configured combined role string.
#[no_mangle]
pub extern "C" fn mesh_cluster_role() -> *mut u8 {
    let role = std::env::var("MESH_ROLES")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "gateway,worker".to_string());
    mesh_string_new(role.as_ptr(), role.len() as u64) as *mut u8
}

/// Return this process's current lifecycle state.
#[no_mangle]
pub extern "C" fn mesh_cluster_state() -> *mut u8 {
    let node_id = super::node::node_state()
        .map(|state| state.name.clone())
        .unwrap_or_else(|| "mesh-local-node".to_string());
    let state = super::routing::local_load_report(&node_id, Default::default())
        .state
        .as_str();
    mesh_string_new(state.as_ptr(), state.len() as u64) as *mut u8
}
