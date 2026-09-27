//! An elastic scheduler starts its local autoscaler, which adds workers
//! while actors wait to run. The scheduler is process-wide: this process
//! starts one, and a child process of this binary starts another.

use std::time::{Duration, Instant};

use mesh_rt::dist::telemetry::runtime_telemetry;
use mesh_rt::{mesh_actor_spawn, mesh_rt_init_actor};

/// Holds its worker, so the actors spawned after it wait as runnable.
extern "C" fn busy(_: *const u8) {
    std::thread::sleep(Duration::from_millis(1_500));
}

#[test]
fn an_elastic_scheduler_grows_while_actors_wait() {
    std::env::set_var("MESH_SCHEDULER_MIN_WORKERS", "1");
    std::env::set_var("MESH_SCHEDULER_MAX_WORKERS", "3");
    std::env::set_var("MESH_SCHEDULER_TARGET_RUNNABLE", "1");
    std::env::set_var("MESH_SCHEDULER_SCALE_UP_WINDOW_MS", "1");
    std::env::set_var("MESH_SCHEDULER_COOLDOWN_MS", "0");
    mesh_rt_init_actor(1);
    assert_eq!(runtime_telemetry().snapshot().active_workers, 1);
    for _ in 0..6 {
        mesh_actor_spawn(busy as *const u8, std::ptr::null(), 0, 1);
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while runtime_telemetry().snapshot().active_workers < 3 {
        assert!(Instant::now() < deadline, "the scheduler never grew");
        std::thread::sleep(Duration::from_millis(20));
    }
}

const INVALID_CHILD_ENV: &str = "MESH_TEST_LOCAL_AUTOSCALER_INVALID";

/// A local policy the environment makes invalid leaves the scheduler at
/// its minimum and says so. The scheduler starts once per process, so a
/// child process of this binary starts it.
#[test]
fn an_invalid_local_policy_keeps_the_minimum() {
    if std::env::var_os(INVALID_CHILD_ENV).is_some() {
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "the_scheduler_under_an_invalid_local_policy",
            "--exact",
            "--nocapture",
        ])
        .env(INVALID_CHILD_ENV, "1")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(
        stderr.contains("mesh scheduler: local autoscaling configuration invalid; keeping minimum"),
        "{stderr}"
    );
}

/// The child process of the test above.
#[test]
fn the_scheduler_under_an_invalid_local_policy() {
    if std::env::var_os(INVALID_CHILD_ENV).is_none() {
        return;
    }
    std::env::set_var("MESH_SCHEDULER_MIN_WORKERS", "1");
    std::env::set_var("MESH_SCHEDULER_MAX_WORKERS", "3");
    std::env::set_var("MESH_SCHEDULER_TARGET_RUNNABLE", "0");
    mesh_rt_init_actor(1);
    assert_eq!(runtime_telemetry().snapshot().active_workers, 1);
}
