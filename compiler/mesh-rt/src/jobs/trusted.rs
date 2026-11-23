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
    fn expired_at(&self, now: u64) -> Option<WorkerPoll> {
        if now < self.deadline.min(self.lease) {
            None
        } else if self.deadline <= self.lease {
            Some(WorkerPoll::TimedOut)
        } else {
            Some(WorkerPoll::LeaseExpired)
        }
    }

    fn check(&self) -> Result<(), WorkerPoll> {
        if self.stopped {
            return Err(WorkerPoll::Exited(Err("trusted job was stopped".into())));
        }
        let now = boot_millis().map_err(|error| WorkerPoll::Exited(Err(error.to_string())))?;
        self.expired_at(now).map_or(Ok(()), Err)
    }

    fn renew_at(&mut self, lease: u64, now: u64) -> Result<(), String> {
        if self.stopped {
            return Ok(());
        }
        match self.expired_at(now) {
            None => self.lease = self.lease.max(lease),
            Some(WorkerPoll::LeaseExpired) => {
                return Err("trusted job could not accept renewal after its lease expired".into())
            }
            // The attempt already timed out. Leave the deadline in force and
            // let the supervisor report the timeout when the actor exits.
            Some(_) => {}
        }
        Ok(())
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
        let now = boot_millis().map_err(|error| error.to_string())?;
        state.renew_at(lease, now)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renewal_cannot_revive_an_expired_or_stopped_attempt() {
        let mut authority = Authority {
            deadline: 200,
            lease: 100,
            stopped: false,
            reply: None,
        };
        assert!(authority.renew_at(300, 100).is_err());
        assert!(matches!(
            authority.expired_at(100),
            Some(WorkerPoll::LeaseExpired)
        ));
        assert!(authority.renew_at(250, 99).is_ok());
        assert!(authority.renew_at(300, 200).is_ok());
        assert!(matches!(
            authority.expired_at(200),
            Some(WorkerPoll::TimedOut)
        ));
        assert_eq!(authority.lease, 250);

        authority.stopped = true;
        assert!(authority.renew_at(300, 190).is_ok());
        assert_eq!(authority.lease, 250);
    }
}

impl Drop for TrustedAttempt {
    fn drop(&mut self) {
        let _ = self.kill();
    }
}
