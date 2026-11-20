use super::{eventually, quoted, Proof, Worker, LEASE_MS, WAIT};
use serde_json::{json, Value};
use std::fs::{self, File};
use std::io;
use std::net::{Shutdown, TcpListener, TcpStream};
use std::os::unix::process::CommandExt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const QUEUE: &str = "wake café";

fn config(proof: &Proof<'_>, poll_ms: u64, listen: bool, claim_batch: bool) -> Value {
    json!({
        "namespace": proof.namespace,
        "queues": { (QUEUE): 1 },
        "workers": [{"worker": "proof", "version": 1}],
        "lease_ms": LEASE_MS,
        "poll_ms": poll_ms,
        "shutdown_grace_ms": 2000,
        "listen": listen,
        "claim_batch": claim_batch,
    })
}

fn spec(label: &str) -> Value {
    json!({"worker":"proof", "queue":QUEUE, "args":{"label":label}})
}

fn publisher(proof: &Proof<'_>, command: &str, value: &Value) -> Value {
    proof.run_command(
        command,
        proof
            .command(command, &value.to_string())
            .env("MESH_JOBS_PROOF_PUBLISH", "on"),
    )
}

pub(super) fn wake_status(worker: &Worker, ready: impl Fn(&Value) -> bool) -> Value {
    eventually("runner to report the expected wake source", || {
        let mut report = worker.status()?;
        ready(&report["wakeups"]).then(|| report["wakeups"].take())
    })
}

fn counter(wakeups: &Value, field: &str) -> u64 {
    wakeups[field].as_u64().unwrap()
}

fn listener_pid(proof: &Proof<'_>) -> Option<u32> {
    let rows = proof.sql(&format!(
        "SELECT pid FROM pg_stat_activity WHERE datname = current_database()
         AND query = 'LISTEN ' || chr(34) || 'mesh_jobs_wake_' || md5(convert_to({}, 'UTF8')) || chr(34)",
        quoted(&proof.namespace),
    ));
    if rows.is_empty() {
        None
    } else {
        Some(rows.parse().expect("exactly one listener session"))
    }
}

pub(super) fn await_subscription(worker: &Worker, after: u64) -> Value {
    wake_status(worker, |wake| {
        wake["subscribed"] == true
            && counter(wake, "subscriptions") > after
            && counter(wake, "subscription_scans") > after
    })
}

pub(super) fn prove(proof: &Proof<'_>) {
    let mut worker =
        proof.worker_with_config("wake-single", false, config(proof, 60_000, true, false));
    let mut baseline = await_subscription(&worker, 0);
    let mut committed = stage(proof, "stage_commit", "wake-committed");
    assert!(proof.job("wake-committed").is_null());
    assert!(proof.effects("wake-committed").is_empty());
    assert_eq!(
        wake_status(&worker, |wake| wake["subscribed"] == true)["notification_scans"],
        baseline["notification_scans"]
    );
    release(&committed);
    assert_eq!(committed.join()["state"], "available");
    proof.wait_state("wake-committed", "completed");
    baseline = wake_status(&worker, |wake| {
        counter(wake, "notification_scans") > counter(&baseline, "notification_scans")
    });
    assert_eq!(baseline["fallback_scans"], 0);
    eprintln!("J3 lane 2 passed: staged enqueue became visible and woke the runner after commit");

    let mut rolled_back = stage(proof, "stage_rollback", "wake-rolled-back");
    assert!(proof.job("wake-rolled-back").is_null());
    release(&rolled_back);
    assert!(rolled_back.join_where(Value::is_null).is_null());
    thread::sleep(Duration::from_millis(100));
    assert_eq!(
        wake_status(&worker, |wake| wake["subscribed"] == true)["notification_scans"],
        baseline["notification_scans"]
    );
    assert!(proof.job("wake-rolled-back").is_null());
    assert!(proof.effects("wake-rolled-back").is_empty());
    eprintln!("J3 lane 3 passed: rollback left neither a job nor a wake");

    assert_eq!(
        proof.control(json!({"action":"pause", "queue":QUEUE}))["state"],
        "paused"
    );
    assert_eq!(proof.enqueue(spec("wake-resume"))["state"], "available");
    assert!(proof.effects("wake-resume").is_empty());
    assert_eq!(
        publisher(proof, "control", &json!({"action":"resume", "queue":QUEUE}))["state"],
        "active"
    );
    proof.wait_state("wake-resume", "completed");
    baseline = wake_status(&worker, |wake| {
        counter(wake, "notification_scans") > counter(&baseline, "notification_scans")
    });
    assert_eq!(baseline["fallback_scans"], 0);
    eprintln!("J3 lane 4 passed: compiled queue resume woke a paused queue's work");

    let mut scheduled = spec("wake-scheduled");
    scheduled["delay_ms"] = json!(1800);
    assert_eq!(
        publisher(proof, "enqueue", &scheduled)["state"],
        "scheduled"
    );
    let due = proof.wait_state("wake-scheduled", "completed");
    assert!(due["attempted_at_ms"].as_i64().unwrap() >= due["scheduled_at_ms"].as_i64().unwrap());
    baseline = wake_status(&worker, |wake| {
        counter(wake, "due_scans") > counter(&baseline, "due_scans")
    });
    publisher(proof, "enqueue", &spec("wake-retry"));
    let retry = proof.wait_state("wake-retry", "completed");
    assert_eq!(retry["attempt"], 2);
    let timed = wake_status(&worker, |wake| {
        counter(wake, "due_scans") > counter(&baseline, "due_scans")
    });
    assert_eq!(timed["fallback_scans"], 0);
    eprintln!("J3 lane 5 passed: scheduled and retryable rows reached their due timers");

    let pid = listener_pid(proof).expect("idle listener");
    let deadline = Instant::now() + Duration::from_secs(2);
    worker.signal();
    let joined = worker.join();
    assert_eq!(joined["wakeups"]["subscribed"], false);
    eventually("listener backend to leave", || {
        listener_pid(proof).is_none().then_some(())
    });
    assert!(
        Instant::now() < deadline,
        "idle runner did not release listener {pid} within its grace"
    );
    eprintln!("J3 lane 10 passed: idle stop released the PostgreSQL listener and exited");

    let mut polling =
        proof.worker_with_config("wake-poll-only", false, config(proof, 100, false, false));
    proof.enqueue(spec("wake-poll-only"));
    proof.wait_state("wake-poll-only", "completed");
    assert_eq!(listener_pid(proof), None);
    polling.signal();
    assert!(polling.join()["wakeups"].is_null());
    eprintln!("J3 lane 9 passed: poll-only processing opened no listener session");

    if let Some((relay, url)) = Relay::new(proof.url) {
        prove_reconnect(proof, &relay, &url);
    } else {
        eprintln!("J3 lanes 6 and 7 require the verifier's loopback PostgreSQL URL for the relay");
    }
}

pub(super) fn prove_sparse_reference(proof: &Proof<'_>) {
    let mut settings = config(proof, 100, false, false);
    let fields = settings.as_object_mut().unwrap();
    fields.remove("listen");
    fields.remove("claim_batch");
    let mut worker = proof.worker_with_config("wake-reference", false, settings);
    assert!(proof.job("wake-committed").is_null());
    let receipt = proof.enqueue(spec("wake-committed"));
    let completed = proof.wait_state("wake-committed", "completed");
    assert_eq!(completed["id"], receipt["id"]);
    assert_eq!(completed["attempt"], 1);
    assert_eq!(proof.effects("wake-committed").len(), 1);
    worker.stop();
    eprintln!("J3 lane 1 passed: the reference runner polled the same committed sparse job; the candidate reported a notification scan in lane 2");
}

fn stage(proof: &Proof<'_>, operation: &str, label: &str) -> Worker {
    let markers = proof.dir.join(label);
    fs::create_dir(&markers).unwrap();
    let output = File::create(markers.join("output")).unwrap();
    let child = proof
        .command(operation, &spec(label).to_string())
        .env("MESH_JOBS_PROOF_MARKERS", &markers)
        .process_group(0)
        .stdout(output.try_clone().unwrap())
        .stderr(output)
        .spawn()
        .unwrap();
    let mut staged = Worker {
        child: Some(child),
        markers,
    };
    eventually("transaction to finish staging enqueue", || {
        staged.assert_running();
        staged.markers.join("staged").exists().then_some(())
    });
    staged
}

fn release(staged: &Worker) {
    File::create(staged.markers.join("commit")).unwrap();
}

fn prove_reconnect(proof: &Proof<'_>, relay: &Relay, url: &str) {
    let interrupt = |after| {
        // Wait for the pool, listener and batch claim sockets. A delayed
        // first claim must not take the gate reserved for the reconnect.
        eventually("initial batch runner sessions to reach the relay", || {
            (relay.accepted() >= after + 3).then_some(())
        });
        relay.interrupt_listener(proof)
    };
    let accepted = relay.accepted();
    let mut polling = proof.worker_with_url(
        "wake-interrupted",
        false,
        config(proof, 3000, true, true),
        url,
    );
    await_subscription(&polling, 0);
    let blocked = interrupt(accepted);
    publisher(proof, "enqueue", &spec("wake-interrupted"));
    proof.wait_state("wake-interrupted", "completed");
    let fallback = wake_status(&polling, |wake| counter(wake, "fallback_scans") > 0);
    assert_eq!(fallback["subscriptions"], 1);
    assert_eq!(fallback["subscribed"], false);
    assert_eq!(fallback["notification_scans"], 0);
    polling.stop();
    drop(blocked);
    eprintln!(
        "J3 lane 6 passed: batch runner fell back to polling during a blocked listener reconnect"
    );

    let accepted = relay.accepted();
    let mut reconnect = proof.worker_with_url(
        "wake-reconnect",
        false,
        config(proof, 60_000, true, true),
        url,
    );
    await_subscription(&reconnect, 0);
    let blocked = interrupt(accepted);
    publisher(proof, "enqueue", &spec("wake-reconnect"));
    assert_eq!(proof.job("wake-reconnect")["state"], "available");
    assert!(proof.effects("wake-reconnect").is_empty());
    blocked.release.send(()).unwrap();
    let catchup = await_subscription(&reconnect, 1);
    proof.wait_state("wake-reconnect", "completed");
    assert_eq!(catchup["fallback_scans"], 0);
    assert_eq!(catchup["notification_scans"], 0);
    reconnect.stop();
    eprintln!(
        "J3 lane 7 passed: a committed row during the outage was picked up by the resubscribe scan"
    );
}

pub(super) struct Gate {
    arrived: mpsc::Receiver<()>,
    release: mpsc::Sender<()>,
}

type Pause = (mpsc::Sender<()>, mpsc::Receiver<()>);

// The relay keeps the TLS/SCRAM bytes opaque. Once the server terminates the
// idle listener, the first new connection is its reconnect. Hold only that
// connection so both the claim session and the handler can still reach PG.
pub(super) struct Relay {
    address: std::net::SocketAddr,
    next: Arc<Mutex<Option<Pause>>>,
    stopped: Arc<AtomicBool>,
    accepted: Arc<AtomicUsize>,
    accept: Option<JoinHandle<()>>,
}

impl Relay {
    pub(super) fn new(url: &str) -> Option<(Self, String)> {
        let (scheme, rest) = url.split_once("://")?;
        let (authority, database) = rest.split_once('/')?;
        let (credentials, address) = authority.rsplit_once('@')?;
        let (host, port) = address.split_once(':')?;
        if host != "127.0.0.1" {
            return None;
        }
        let target = (std::net::Ipv4Addr::LOCALHOST, port.parse::<u16>().unwrap());
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let next = Arc::new(Mutex::new(None::<Pause>));
        let stopped = Arc::new(AtomicBool::new(false));
        let accepted = Arc::new(AtomicUsize::new(0));
        let pending = Arc::clone(&next);
        let stopping = Arc::clone(&stopped);
        let count = Arc::clone(&accepted);
        let accept = thread::spawn(move || {
            for client in listener.incoming() {
                if stopping.load(Ordering::Acquire) {
                    break;
                }
                let mut client = client.unwrap();
                let pause = pending.lock().unwrap().take();
                count.fetch_add(1, Ordering::Release);
                thread::spawn(move || {
                    if let Some((arrived, release)) = pause {
                        if arrived.send(()).is_err() || release.recv().is_err() {
                            return;
                        }
                    }
                    let mut upstream = TcpStream::connect(target).unwrap();
                    let mut request = client.try_clone().unwrap();
                    let mut response = upstream.try_clone().unwrap();
                    thread::scope(|scope| {
                        scope.spawn(move || {
                            let _ = io::copy(&mut request, &mut upstream);
                            let _ = upstream.shutdown(Shutdown::Both);
                            let _ = request.shutdown(Shutdown::Both);
                        });
                        let _ = io::copy(&mut response, &mut client);
                        let _ = client.shutdown(Shutdown::Both);
                        let _ = response.shutdown(Shutdown::Both);
                    });
                });
            }
        });
        let relay = Self {
            address,
            next,
            stopped,
            accepted,
            accept: Some(accept),
        };
        Some((
            relay,
            format!("{scheme}://{credentials}@{address}/{database}"),
        ))
    }

    fn hold_next(&self) -> Gate {
        let (start, arrived) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        assert!(self.next.lock().unwrap().replace((start, resume)).is_none());
        Gate { arrived, release }
    }

    pub(super) fn interrupt_listener(&self, proof: &Proof<'_>) -> Gate {
        let pid = listener_pid(proof).expect("listener before interruption");
        let gate = self.hold_next();
        assert_eq!(
            proof.sql(&format!("SELECT pg_terminate_backend({pid})")),
            "t"
        );
        gate.arrived
            .recv_timeout(WAIT)
            .expect("listener to start reconnecting");
        eventually("terminated listener session to leave PostgreSQL", || {
            listener_pid(proof).is_none().then_some(())
        });
        gate
    }

    fn accepted(&self) -> usize {
        self.accepted.load(Ordering::Acquire)
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        let _ = TcpStream::connect(self.address);
        self.accept.take().unwrap().join().unwrap();
    }
}
