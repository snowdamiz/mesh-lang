enum Diagnostic {
    Message(String),
    Supervised(String),
    Flush(crossbeam_channel::Sender<()>),
}

impl Diagnostic {
    fn write(self) {
        use std::io::Write;
        match self {
            Self::Message(message) => {
                let _ = writeln!(std::io::stderr(), "{message}");
            }
            Self::Supervised(line) => {
                let _ = std::io::stderr().write_all(line.as_bytes());
                let mut pending = SUPERVISED_PENDING.lock();
                *pending -= 1;
                if *pending == 0 {
                    SUPERVISED_WRITTEN.notify_all();
                }
            }
            Self::Flush(done) => {
                let _ = done.send(());
            }
        }
    }
}

static DIAGNOSTICS: std::sync::OnceLock<crossbeam_channel::Sender<Diagnostic>> =
    std::sync::OnceLock::new();
static SUPERVISED_PENDING: parking_lot::Mutex<usize> = parking_lot::Mutex::new(0);
static SUPERVISED_WRITTEN: parking_lot::Condvar = parking_lot::Condvar::new();

fn diagnostic_writer() -> &'static crossbeam_channel::Sender<Diagnostic> {
    DIAGNOSTICS.get_or_init(|| {
        let (sender, receiver) = crossbeam_channel::bounded(16);
        let _ = std::thread::Builder::new()
            .name("mesh-diagnostics".to_string())
            .spawn(move || receiver.into_iter().for_each(Diagnostic::write));
        sender
    })
}

#[inline(never)]
pub(crate) fn prepare_supervised_diagnostics() {
    diagnostic_writer();
}

#[cold]
pub(super) fn report_supervised(reason: &str) {
    let mut line = String::with_capacity(reason.len() + 1);
    line.push_str(reason);
    line.push('\n');
    *SUPERVISED_PENDING.lock() += 1;
    // A full queue applies backpressure. A failed writer start falls back to
    // writing on this worker. The report is owned before the actor exits.
    if let Err(error) = diagnostic_writer().send(Diagnostic::Supervised(line)) {
        error.0.write();
    }
}

pub(crate) fn flush_supervised_diagnostics() {
    if *SUPERVISED_PENDING.lock() == 0 {
        return;
    }
    // Process.exit can race other producers. Wait for the queue prefix
    // accepted before this barrier, even if new failures keep arriving.
    let (done, written) = crossbeam_channel::bounded(1);
    if diagnostic_writer().send(Diagnostic::Flush(done)).is_ok() {
        let _ = written.recv();
        return;
    }
    // A writer that failed to start leaves the reporting threads doing I/O.
    let mut pending = SUPERVISED_PENDING.lock();
    while *pending != 0 {
        SUPERVISED_WRITTEN.wait(&mut pending);
    }
}

pub(super) fn enqueue_diagnostic(message: &str) {
    let _ = diagnostic_writer().try_send(Diagnostic::Message(message.to_owned()));
}

pub(crate) fn flush_deferred_diagnostics() {
    flush_supervised_diagnostics();
    if let Some(writer) = DIAGNOSTICS.get() {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(20);
        let (done, receiver) = crossbeam_channel::bounded(1);
        if writer
            .send_deadline(Diagnostic::Flush(done), deadline)
            .is_ok()
        {
            let _ = receiver.recv_deadline(deadline);
        }
    }
}
