//! A node joining a peer receives the peer's continuity records and its
//! durable store: record upserts, then the store snapshot in chunks, then
//! the store log written while the snapshot was on its way. A process holds
//! one node, so this binary runs the joining node as a child process of
//! itself: the second test is that child's part and does nothing when the
//! binary runs on its own.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mesh_rt::{
    configured_continuity_store, continuity_registry, mesh_continuity_submit, mesh_node_connect,
    mesh_node_start, mesh_string_new,
};

const COOKIE: &str = "continuity-sync-cookie";
const TARGET_ENV: &str = "MESH_TEST_CONTINUITY_SYNC_TARGET";
const NAME_ENV: &str = "MESH_TEST_CONTINUITY_SYNC_NAME";
const RECORDS: usize = 300;
/// The joining node's line once it holds the source's state. libtest puts
/// the test's name before a test's first output, so it is found by suffix.
const SYNCED: &str = "continuity-sync: synced";

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn start_node(name: &str) {
    mesh_rt::actor::mesh_rt_init_actor(2);
    assert_eq!(
        mesh_node_start(
            name.as_ptr(),
            name.len() as u64,
            COOKIE.as_ptr(),
            COOKIE.len() as u64
        ),
        0
    );
}

fn submit(key: &str, ingress: &str, owner: &str) {
    let text = |value: &str| mesh_string_new(value.as_ptr(), value.len() as u64) as *const _;
    let result = mesh_continuity_submit(
        text(key),
        text("hash"),
        text(ingress),
        text(owner),
        text(""),
        0,
        0,
    );
    assert_eq!(unsafe { (*result).tag }, 0, "submit {key}");
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_joining_node_receives_the_continuity_records_and_store() {
    if std::env::var_os(TARGET_ENV).is_some() {
        return;
    }
    let directory = tempfile::tempdir().expect("tempdir");
    std::env::set_var("MESH_CONTINUITY_DB", directory.path().join("source.db"));
    // Small chunks make the snapshot many frames, so writes land while it
    // is on its way.
    std::env::set_var("MESH_CONTINUITY_SNAPSHOT_CHUNK_BYTES", "1024");
    let store = configured_continuity_store().expect("source store");
    let source = format!("source@127.0.0.1:{}", free_port());
    start_node(&source);
    for index in 0..RECORDS {
        submit(&format!("before-{index}"), &source, &source);
    }
    wait_until("the source store", || {
        store.stats().unwrap().records >= RECORDS as u64
    });

    let joiner = format!("joiner@127.0.0.1:{}", free_port());
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "a_joining_node_catches_up",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(TARGET_ENV, &source)
        .env(NAME_ENV, &joiner)
        .env("MESH_CONTINUITY_DB", directory.path().join("joiner.db"))
        .stdout(Stdio::piped())
        .spawn()
        .expect("joining node");
    let writing = Arc::new(AtomicBool::new(true));
    let writer = std::thread::spawn({
        let writing = Arc::clone(&writing);
        let source = source.clone();
        move || {
            let mut index = 0;
            while writing.load(Ordering::Acquire) {
                submit(&format!("during-{index}"), &source, &source);
                index += 1;
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    });
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let synced = lines.any(|line| line.is_ok_and(|line| line.ends_with(SYNCED)));
    writing.store(false, Ordering::Release);
    writer.join().unwrap();
    assert!(synced, "the joining node never caught up");

    // The joining node acknowledges the whole snapshot: the source records
    // how far that replica has the log and compacts the log up to there.
    wait_until("the snapshot acknowledgement", || {
        store
            .stats()
            .unwrap()
            .replica_safe_point
            .is_some_and(|point| point >= RECORDS as u64)
    });
    let stats = store.stats().unwrap();
    assert!(stats.log_entries < stats.high_water_mark, "{stats:?}");

    // A record the joining node owns goes to it at once.
    submit("owned-by-joiner", &source, &joiner);
    assert!(child.wait().unwrap().success());
}

/// The joining node, run by the test above as a child process.
#[test]
fn a_joining_node_catches_up() {
    let (Some(target), Ok(name)) = (std::env::var_os(TARGET_ENV), std::env::var(NAME_ENV)) else {
        return;
    };
    let target = target.into_string().unwrap();
    let store = configured_continuity_store().expect("joiner store");
    start_node(&name);
    assert_eq!(mesh_node_connect(target.as_ptr(), target.len() as u64), 0);
    wait_until("the source's records", || {
        (0..RECORDS).all(|index| {
            continuity_registry()
                .record(&format!("before-{index}"))
                .is_some()
        })
    });
    wait_until("the source's store", || {
        store.stats().unwrap().records >= RECORDS as u64
    });
    println!("{SYNCED}");
    wait_until("the record this node owns", || {
        continuity_registry().record("owned-by-joiner").is_some()
    });
}
