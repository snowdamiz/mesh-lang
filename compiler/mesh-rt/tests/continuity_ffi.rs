//! The continuity FFI works on the process-wide continuity registry, and
//! the records it leaves change the authority status every other test in a
//! process would read, so it runs in a process of its own.

use mesh_rt::dist::continuity::{
    mesh_continuity_complete_declared_work, MeshContinuitySubmitDecision,
};
use mesh_rt::io::MeshResult;
use mesh_rt::{
    mesh_continuity_acknowledge_replica, mesh_continuity_authority_status,
    mesh_continuity_mark_completed, mesh_continuity_status, mesh_continuity_submit,
    mesh_continuity_submit_declared_work, mesh_register_declared_handler, mesh_rt_init_actor,
    mesh_string_new, MeshString,
};

/// Declared work that does nothing with its request key and attempt.
extern "C" fn declared_work(_args: *const u8) {}

#[test]
fn the_continuity_ffi_answers_through_mesh_results() {
    let text =
        |value: &str| mesh_string_new(value.as_ptr(), value.len() as u64) as *const MeshString;
    let tag = |result: *mut MeshResult| unsafe { (*result).tag };
    let key = "ffi-continuity-request";
    let submitted = mesh_continuity_submit(
        text(key),
        text("hash"),
        text("ingress@host"),
        text("owner@host"),
        text("replica@host"),
        1,
        0,
    );
    assert_eq!(tag(submitted), 0);
    let (outcome, attempt) = unsafe {
        let decision = &*((*submitted).value as *const MeshContinuitySubmitDecision);
        (
            (*decision.outcome).as_str().to_string(),
            (*decision.record.attempt_id).as_str().to_string(),
        )
    };
    assert_eq!(outcome, "created");
    assert_eq!(tag(mesh_continuity_status(text(key))), 0);
    assert_eq!(tag(mesh_continuity_status(text("ffi-missing"))), 1);
    assert_eq!(tag(mesh_continuity_authority_status()), 0);
    assert_eq!(
        tag(mesh_continuity_acknowledge_replica(
            text(key),
            text(&attempt)
        )),
        0
    );
    assert_eq!(
        tag(mesh_continuity_acknowledge_replica(
            text("ffi-missing"),
            text(&attempt)
        )),
        1
    );
    assert_eq!(
        tag(mesh_continuity_mark_completed(
            text(key),
            text("attempt-x"),
            text("owner@host")
        )),
        1
    );
    assert_eq!(
        tag(mesh_continuity_mark_completed(
            text(key),
            text(&attempt),
            text("owner@host")
        )),
        0
    );
    assert_eq!(
        tag(mesh_continuity_submit(
            text(""),
            text("hash"),
            text("ingress@host"),
            text("owner@host"),
            text(""),
            0,
            0
        )),
        1
    );
    assert_eq!(
        tag(mesh_continuity_submit_declared_work(
            text("Ffi.unregistered"),
            text("ffi-declared"),
            text("hash"),
            -1
        )),
        1
    );
    assert_eq!(
        tag(mesh_continuity_submit_declared_work(
            text("Ffi.unregistered"),
            text("ffi-declared"),
            text("hash"),
            0
        )),
        1
    );
    // A registered handler runs declared work as an actor; a request the
    // registry refuses is the submit's error.
    mesh_rt_init_actor(1);
    let (runtime, executable) = ("Ffi.work", "ffi_work");
    mesh_register_declared_handler(
        runtime.as_ptr(),
        runtime.len() as u64,
        executable.as_ptr(),
        executable.len() as u64,
        1,
        declared_work as *const u8,
    );
    let declared =
        mesh_continuity_submit_declared_work(text(runtime), text("ffi-declared"), text("hash"), 0);
    assert_eq!(tag(declared), 0);
    let declared_attempt = unsafe {
        let decision = &*((*declared).value as *const MeshContinuitySubmitDecision);
        (*decision.record.attempt_id).as_str().to_string()
    };
    // The declared work completes on this node under its own attempt.
    assert_eq!(
        tag(mesh_continuity_complete_declared_work(
            text("ffi-declared"),
            text(&declared_attempt)
        )),
        0
    );
    assert_eq!(
        tag(mesh_continuity_submit_declared_work(
            text(runtime),
            text(""),
            text("hash"),
            0
        )),
        1
    );
    assert_eq!(
        tag(mesh_continuity_complete_declared_work(
            text("ffi-missing"),
            text("attempt-0")
        )),
        1
    );
}

const ROLE_CHILD_ENV: &str = "MESH_TEST_CONTINUITY_ROLE_CHILD";

/// A mistyped continuity role must not quietly make a standby a primary:
/// the registry's first use stops the process. The registry starts once
/// per process, so a child process of this binary starts it.
#[test]
fn a_mistyped_continuity_role_stops_the_process() {
    if std::env::var_os(ROLE_CHILD_ENV).is_some() {
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "the_registry_under_a_mistyped_role",
            "--exact",
            "--nocapture",
        ])
        .env(ROLE_CHILD_ENV, "1")
        .env("MESH_CONTINUITY_ROLE", "standyb")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid MESH_CONTINUITY_ROLE `standyb`: expected primary or standby"),
        "{stderr}"
    );
}

/// The child process of the test above.
#[test]
fn the_registry_under_a_mistyped_role() {
    if std::env::var_os(ROLE_CHILD_ENV).is_none() {
        return;
    }
    mesh_continuity_authority_status();
    unreachable!("the registry started under a mistyped role");
}
