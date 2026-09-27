//! An elastic scheduler starts its local autoscaler, which adds workers
//! while actors wait to run. The scheduler is process-wide, so this binary
//! holds one test.

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
