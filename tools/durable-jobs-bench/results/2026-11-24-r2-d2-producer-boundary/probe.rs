// Benchmark-only broker observer. One fixed-size buffer survives until "stop".
use std::cell::RefCell;
use std::sync::LazyLock;

static ENABLED: LazyLock<bool> = LazyLock::new(|| {
    std::env::var_os("MESH_R2_PRODUCER_PROBE").as_deref() == Some(std::ffi::OsStr::new("1"))
});

struct Trace {
    active: bool,
    complete: bool,
    len: usize,
    events: [(&'static str, u64); 40],
}

thread_local! {
    static TRACE: RefCell<Trace> = const {
        RefCell::new(Trace { active: false, complete: false, len: 0, events: [("", 0); 40] })
    };
}

fn clock() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) } != 0 {
        return 0; // The offline reducer refuses a missing clock.
    }
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

impl Trace {
    fn mark(&mut self, name: &'static str) {
        if let Some(event) = self.events.get_mut(self.len) {
            *event = (name, clock());
        }
        self.len += 1;
    }
}

pub(crate) fn line(line: &str) {
    if !*ENABLED {
        return;
    }
    TRACE.with(|trace| {
        let mut trace = trace.borrow_mut();
        if line.starts_with(r#"{"verb":"enqueue","batch":"#) {
            trace.active = true;
            trace.mark("broker_input_ready");
        } else if line == "stop" {
            // Output, allocation and formatting are outside the measured arm.
            for (name, ns) in trace.events.iter().take(trace.len) {
                eprintln!("r2_producer_stage {name} {ns}");
            }
            if !trace.complete || trace.len > trace.events.len() {
                eprintln!("r2_producer_stage invalid");
            }
        }
    });
}

pub(crate) fn publish() {
    if !*ENABLED {
        return;
    }
    TRACE.with(|trace| {
        let mut trace = trace.borrow_mut();
        if trace.active {
            trace.mark("broker_publish");
            trace.active = false;
            trace.complete = true;
        }
    });
}

pub(crate) struct Request {
    ready: &'static str,
    sent: &'static str,
    bulk_first_row: bool,
}

pub(crate) fn begin(message: &[u8]) -> Option<Request> {
    if !*ENABLED {
        return None;
    }
    TRACE.with(|trace| {
        let mut trace = trace.borrow_mut();
        if !trace.active {
            return None;
        }
        // This producer uses simple commands and unnamed Parse/Bind. Probe
        // only the fixed SQL prefixes, without recording SQL or parameters.
        let sql = match message.first()? {
            b'Q' => message.get(5..)?,
            b'P' => message.get(6..)?,
            _ => return None,
        };
        for (prefix, send, sent, ready) in [
            (
                b"BEGIN\0".as_slice(),
                "begin_send",
                "begin_sent",
                "begin_ready",
            ),
            (
                b"SET LOCAL mesh_jobs.publish_wakeups = on",
                "wake_send",
                "wake_sent",
                "wake_ready",
            ),
            (
                b"INSERT INTO public.mesh_jobs_parity_product",
                "product_send",
                "product_sent",
                "product_ready",
            ),
            (
                b"SAVEPOINT mesh_jobs_enqueue\0",
                "savepoint_send",
                "savepoint_sent",
                "savepoint_ready",
            ),
            (
                b"SELECT CASE WHEN pg_catalog.current_setting('server_version_num')",
                "guard_send",
                "guard_sent",
                "guard_ready",
            ),
            (
                b"INSERT INTO mesh_jobs.admissions",
                "admission_send",
                "admission_sent",
                "admission_ready",
            ),
            (
                b"SELECT ordinal, receipt FROM mesh_jobs.enqueue_batch_v1(",
                "bulk_send",
                "bulk_sent",
                "bulk_ready",
            ),
            (
                b"RELEASE SAVEPOINT mesh_jobs_enqueue\0",
                "release_send",
                "release_sent",
                "release_ready",
            ),
            (
                b"SELECT pg_current_xact_id()::text AS xid",
                "xid_send",
                "xid_sent",
                "xid_ready",
            ),
            (b"COMMIT\0", "commit_send", "commit_sent", "commit_ready"),
        ] {
            if sql.starts_with(prefix) {
                trace.mark(send);
                return Some(Request {
                    ready,
                    sent,
                    bulk_first_row: send == "bulk_send",
                });
            }
        }
        None
    })
}

impl Request {
    pub(crate) fn sent(&self) {
        TRACE.with(|trace| trace.borrow_mut().mark(self.sent));
    }

    pub(crate) fn ready(&self) {
        TRACE.with(|trace| trace.borrow_mut().mark(self.ready));
    }

    pub(crate) fn row(&mut self, tag: u8) {
        if self.bulk_first_row && tag == b'D' {
            self.bulk_first_row = false;
            TRACE.with(|trace| trace.borrow_mut().mark("bulk_first_row"));
        }
    }
}
