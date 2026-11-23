use std::sync::Arc;
use std::time::Instant;

use crossbeam_channel::{Receiver, Sender, TryRecvError};
use parking_lot::Mutex;

use crate::actor::{
    global_scheduler,
    process::{ExitReason, ProcessId},
};

use super::runner::JobProcess;
use super::worker::{boot_millis, host_deadline, invoke_in_current_context, Handler, WorkerPoll};

struct Authority {
    deadline: u64,
    lease: u64,
    stopped: bool,
    reply: Option<Result<String, String>>,
}

impl Authority {
    fn check(&self) -> Result<(), WorkerPoll> {
        if self.stopped {
            return Err(WorkerPoll::Exited(Err("trusted job was stopped".into())));
        }
        let now = boot_millis().map_err(|error| WorkerPoll::Exited(Err(error.to_string())))?;
        if now < self.deadline.min(self.lease) {
            Ok(())
        } else if self.deadline <= self.lease {
            Err(WorkerPoll::TimedOut)
        } else {
            Err(WorkerPoll::LeaseExpired)
        }
    }

    fn exited(&mut self, reason: ExitReason) -> WorkerPoll {
        if let Err(outcome) = self.check() {
            return outcome;
        }
        let result = match reason {
            ExitReason::Normal => self
                .reply
                .take()
                .unwrap_or_else(|| Err("trusted job actor stopped without an outcome".into())),
            ExitReason::Error(message) => Err(message),
            _ => Err("trusted job actor was terminated".into()),
        };
        WorkerPoll::Exited(result)
    }
}

pub(super) struct TrustedAttempt {
    pid: ProcessId,
    authority: Arc<Mutex<Authority>>,
    result: Receiver<WorkerPoll>,
}

impl TrustedAttempt {
    pub(super) fn start(
        handler: Handler,
        context: String,
        deadline: Instant,
        lease: u64,
        wake: Sender<()>,
    ) -> Result<Self, String> {
        let authority = Arc::new(Mutex::new(Authority {
            deadline: host_deadline(deadline)?,
            lease,
            stopped: false,
            reply: None,
        }));
        let (send, result) = crossbeam_channel::bounded(1);
        let entry = Arc::clone(&authority);
        let exit = Arc::clone(&authority);
        let pid = global_scheduler().spawn_owned(
            move || {
                if entry.lock().check().is_ok() {
                    let reply =
                        invoke_in_current_context(context, |input| unsafe { handler(input) });
                    entry.lock().reply = Some(reply);
                }
            },
            move |reason| {
                // The scheduler has unwound the actor and finished exit cleanup.
                let _ = send.try_send(exit.lock().exited(reason));
                let _ = wake.try_send(());
            },
        );
        Ok(Self {
            pid,
            authority,
            result,
        })
    }
}

impl JobProcess for TrustedAttempt {
    fn reply_pending(&self) -> bool {
        !self.result.is_empty()
    }

    fn poll(&mut self, _: Instant) -> Result<WorkerPoll, String> {
        match self.result.try_recv() {
            Ok(reply) => Ok(reply),
            Err(TryRecvError::Empty) => Ok(WorkerPoll::Running),
            Err(TryRecvError::Disconnected) => Ok(WorkerPoll::Exited(Err(
                "scheduler dropped the trusted job actor".into(),
            ))),
        }
    }

    fn renew(&mut self, lease: u64) -> Result<(), String> {
        let mut state = self.authority.lock();
        state
            .check()
            .map_err(|_| "trusted job could not accept renewal after its deadline")?;
        state.lease = state.lease.max(lease);
        Ok(())
    }

    fn kill(&mut self) -> Result<(), String> {
        if !std::mem::replace(&mut self.authority.lock().stopped, true) {
            if let Some(process) = crate::actor::process(self.pid) {
                global_scheduler().force_exit(self.pid, process.lock(), ExitReason::Shutdown);
            }
        }
        Ok(())
    }
}

impl Drop for TrustedAttempt {
    fn drop(&mut self) {
        let _ = self.kill();
    }
}
