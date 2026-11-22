use std::io::{self, Read};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::ptr;
use std::sync::Arc;

use parking_lot::Mutex;

use super::{deadline_exit_code, exit_worker, remaining, BOOTSTRAP_ERROR, PARENT_GONE};

// The worker process owns stdin. Read bootstrap frames without buffering any
// renewal bytes past the end of the context frame.
pub(super) struct Control(pub(super) RawFd);

impl Control {
    fn receive(&self, bytes: &mut [u8], flags: libc::c_int) -> io::Result<usize> {
        let count = unsafe { libc::recv(self.0, bytes.as_mut_ptr().cast(), bytes.len(), flags) };
        if count < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(count as usize)
    }

    pub(super) fn supervise(self, deadline: u64, lease: Arc<Mutex<u64>>) -> io::Result<()> {
        let accepted = *lease.lock();
        remaining(deadline, accepted);
        let fd = unsafe { libc::timerfd_create(libc::CLOCK_BOOTTIME, libc::TFD_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let timer = unsafe { OwnedFd::from_raw_fd(fd) };
        arm(timer.as_fd(), deadline.min(accepted))?;
        let (ready, started) = crossbeam_channel::bounded(0);
        std::thread::Builder::new()
            .name("mesh-job-watchdog".into())
            .spawn(move || {
                // Rust handlers must never read renewal bytes from the control socket.
                let stdin = io::stdin();
                let _control_lock = stdin.lock();
                if ready.send(()).is_ok() {
                    self.watch(timer, deadline, &lease);
                }
            })?;
        started.recv().map_err(io::Error::other)
    }

    fn watch(self, timer: OwnedFd, deadline: u64, lease: &Mutex<u64>) -> ! {
        let mut waiting = [
            libc::pollfd {
                fd: timer.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: self.0,
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let mut frame = [0; 8];
        let mut filled = 0;
        loop {
            if unsafe { libc::poll(waiting.as_mut_ptr(), waiting.len() as _, -1) } < 0 {
                if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                exit_worker(BOOTSTRAP_ERROR);
            }
            if waiting[0].revents & libc::POLLIN != 0 {
                exit_worker(deadline_exit_code(deadline, *lease.lock()));
            }
            if waiting[0].revents != 0 {
                exit_worker(BOOTSTRAP_ERROR);
            }

            // Read only the rest of this frame and never wait inside recv,
            // even if another reader consumed the reported readiness.
            match self.receive(&mut frame[filled..], libc::MSG_DONTWAIT) {
                Ok(0) => exit_worker(PARENT_GONE),
                Ok(count) => filled += count,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                    ) =>
                {
                    continue;
                }
                Err(_) => exit_worker(PARENT_GONE),
            }
            if filled == frame.len() {
                // The old cutoff can pass between poll and a complete read.
                let mut accepted = lease.lock();
                remaining(deadline, *accepted);
                let next = (*accepted).max(u64::from_le_bytes(frame));
                if deadline.min(next) != deadline.min(*accepted)
                    && arm(timer.as_fd(), deadline.min(next)).is_err()
                {
                    exit_worker(BOOTSTRAP_ERROR);
                }
                *accepted = next;
                filled = 0;
            }
        }
    }
}

impl Read for Control {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.receive(bytes, 0)
    }
}

fn arm(timer: BorrowedFd<'_>, cutoff: u64) -> io::Result<()> {
    // An all-zero value disarms the timer. supervise checks for expiry first,
    // and renewals can only move the cutoff forward.
    let value = libc::itimerspec {
        it_interval: libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        },
        it_value: libc::timespec {
            tv_sec: (cutoff / 1_000) as _,
            tv_nsec: ((cutoff % 1_000) * 1_000_000) as _,
        },
    };
    if unsafe {
        libc::timerfd_settime(
            timer.as_raw_fd(),
            libc::TFD_TIMER_ABSTIME,
            &value,
            ptr::null_mut(),
        )
    } < 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
