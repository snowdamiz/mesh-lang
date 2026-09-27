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
    mesh_continuity_submit_declared_work, mesh_string_new, MeshString,
};

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
    assert_eq!(
        tag(mesh_continuity_complete_declared_work(
            text("ffi-missing"),
            text("attempt-0")
        )),
        1
    );
}
