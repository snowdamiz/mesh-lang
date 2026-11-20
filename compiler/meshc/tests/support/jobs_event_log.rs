use super::{assert_context, eventually, Proof, Worker};
use serde_json::{json, Value};
use std::fs::{self, File};
use std::io::{ErrorKind, Write};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::Stdio;

pub(super) fn prove(proof: &Proof<'_>) {
    let label = "event-log-backpressure";
    let receipt = proof.enqueue(json!({
        "worker": "proof", "args": {"label": label}, "max_attempts": 1,
    }));
    let markers = proof.dir.join(label);
    fs::create_dir(&markers).unwrap();
    let output = File::create(markers.join("output")).unwrap();
    let (stderr, reader) = saturated_stderr();
    let config = json!({
        "namespace": proof.namespace,
        "queues": {"default": 1},
        "workers": [{"worker": "proof", "version": 1}],
        "shutdown_grace_ms": 8000,
        "cron_interval_ms": 0,
        "event_buffer": 16,
        "event_log": true,
    });
    let mut worker = Worker {
        child: Some(
            proof
                .command("serve_with_post_join_status", &config.to_string())
                .process_group(0)
                .env("MESH_JOBS_PROOF_WORKER", label)
                .env("MESH_JOBS_PROOF_MARKERS", &markers)
                .env("MESH_JOBS_PROOF_EVENTS", "true")
                .stdout(output)
                .stderr(stderr)
                .spawn()
                .expect("start compiled Jobs fixture with saturated stderr"),
        ),
        markers,
    };

    // The PostgreSQL observer is independent of the fixture's status reports.
    let completed = proof.wait_state(label, "completed");
    assert_eq!(completed["id"], receipt["id"]);
    assert_eq!(completed["attempt"], 1);
    let effects = proof.effects(label);
    assert_eq!(effects.len(), 1);
    assert_context(&effects[0], &receipt, label, 1);

    worker.signal();
    let joined: Value = eventually("Jobs.join with stderr still saturated", || {
        worker.assert_running();
        serde_json::from_slice(&fs::read(worker.markers.join("joined")).ok()?).ok()
    });
    assert_eq!(joined["state"], "stopped", "{joined}");
    assert_eq!(joined["drained"], true, "{joined}");
    assert_eq!(joined["outcomes"]["completed"], 1, "{joined}");
    assert_eq!(joined["events_dropped"], 0, "{joined}");
    assert_eq!(joined["event_logs_dropped"], 0, "{joined}");

    eventually("public event reader to close after Jobs.join", || {
        worker.assert_running();
        (worker.event_batches().last()?["closed"] == true).then_some(())
    });
    let events = worker.events();
    assert_eq!(
        events
            .iter()
            .map(|event| event["event"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "runner_started",
            "attempt_started",
            "settlement_committed",
            "runner_stopped"
        ],
    );
    assert_eq!(events[2]["attempt"]["job_id"], receipt["id"]);
    assert_eq!(events[2]["settlement"], "completed");

    // Closing the unread peer fails the first write. All later events must
    // also count as dropped, including those still buffered at join.
    drop(reader);
    let expected_drops = events.len() as u64;
    eventually(
        "Jobs.status to count all logger drops after Jobs.join",
        || {
            worker.assert_running();
            let bytes = fs::read(worker.markers.join("post-join-status")).ok()?;
            let status: Value = serde_json::from_slice(&bytes).ok()?;
            assert_eq!(status["runner_id"], joined["runner_id"], "{status}");
            assert_eq!(status["state"], "stopped", "{status}");
            let dropped = status["event_logs_dropped"].as_u64().unwrap();
            assert!(dropped <= expected_drops, "{status}");
            (dropped == expected_drops).then_some(())
        },
    );
    File::create(worker.markers.join("finish-post-join")).unwrap();
    let final_status = worker.join();
    assert_eq!(final_status["runner_id"], joined["runner_id"]);
    assert_eq!(final_status["state"], "stopped");
    assert_eq!(final_status["outcomes"], joined["outcomes"]);
    assert_eq!(final_status["event_logs_dropped"], expected_drops);
    println!("compiled Jobs event logger counted {expected_drops} drops after join");
}

fn saturated_stderr() -> (Stdio, UnixStream) {
    let (mut writer, reader) = UnixStream::pair().unwrap();
    writer.set_nonblocking(true).unwrap();
    let padding = [0; 4096];
    let mut bytes = padding.as_slice();
    for _ in 0..4096 {
        match writer.write(bytes) {
            Ok(written) => assert!(written > 0, "stderr padding made no progress"),
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                if bytes.len() == 1 {
                    writer.set_nonblocking(false).unwrap();
                    return (Stdio::from(OwnedFd::from(writer)), reader);
                }
                // Verify that even the smallest write cannot fit.
                bytes = &padding[..1];
            }
            Err(error) => panic!("could not prefill fixture stderr: {error}"),
        }
    }
    panic!("fixture stderr did not saturate within the padding limit");
}
