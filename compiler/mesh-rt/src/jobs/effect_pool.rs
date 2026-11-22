use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

pub(super) struct SessionPool<S> {
    state: Mutex<State<S>>,
    changed: Condvar,
}

#[cfg(test)]
type Waiters = usize;
#[cfg(not(test))]
type Waiters = ();

struct State<S> {
    idle: Vec<S>,
    checked_out: usize,
    closed: bool,
    interrupt: u64,
    _waiters: Waiters,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum CheckoutError {
    Unavailable,
    Cancelled,
}

pub(super) struct Checkout<'a, S> {
    pool: &'a SessionPool<S>,
    session: Option<S>,
}

impl<S> SessionPool<S> {
    pub(super) fn new(sessions: Vec<S>) -> Self {
        Self {
            state: Mutex::new(State {
                idle: sessions,
                checked_out: 0,
                closed: false,
                interrupt: 0,
                _waiters: Waiters::default(),
            }),
            changed: Condvar::new(),
        }
    }

    pub(super) fn checkout(
        &self,
        deadline: Instant,
        receipt_reserve: Duration,
        probe_interval: Duration,
        live: impl Fn() -> bool,
    ) -> Result<Checkout<'_, S>, CheckoutError> {
        let mut state = self.state.lock().unwrap();
        let mut waited = false;
        loop {
            // Probe the peer and authority outside the pool lock. A concurrent
            // interrupt must be observed before we can wait on an old probe.
            let interrupt = state.interrupt;
            drop(state);
            if !live() {
                return Err(CheckoutError::Cancelled);
            }
            state = self.state.lock().unwrap();
            let left = deadline.saturating_duration_since(Instant::now());
            let wait_left = left.saturating_sub(receipt_reserve);
            if state.closed || left.is_zero() || (waited && wait_left.is_zero()) {
                return Err(CheckoutError::Unavailable);
            }
            if interrupt != state.interrupt {
                continue;
            }
            if let Some(session) = state.idle.pop() {
                state.checked_out += 1;
                return Ok(Checkout {
                    pool: self,
                    session: Some(session),
                });
            }
            // Reserve receipt time only when waiting for a busy lane. An
            // immediately idle lane still uses the original call deadline.
            if state.checked_out == 0 || wait_left.is_zero() {
                return Err(CheckoutError::Unavailable);
            }
            #[cfg(test)]
            let () = { state._waiters += 1 };
            waited = true;
            (state, _) = self
                .changed
                .wait_timeout(state, wait_left.min(probe_interval))
                .unwrap();
            #[cfg(test)]
            let () = { state._waiters -= 1 };
        }
    }

    pub(super) fn exhausted_permanently(&self) -> bool {
        let state = self.state.lock().unwrap();
        state.closed || (state.idle.is_empty() && state.checked_out == 0)
    }

    pub(super) fn interrupt_waiters(&self) {
        let mut state = self.state.lock().unwrap();
        state.interrupt = state.interrupt.wrapping_add(1);
        self.changed.notify_all();
    }

    pub(super) fn close(&self) {
        let idle = {
            let mut state = self.state.lock().unwrap();
            state.closed = true;
            self.changed.notify_all();
            std::mem::take(&mut state.idle)
        };
        drop(idle);
    }
}

impl<S> Checkout<'_, S> {
    pub(super) fn session(&mut self) -> &mut S {
        self.session.as_mut().unwrap()
    }

    // Return the exact checkout after a complete idle reset, or when the
    // unchanged session never attached to the attempt.
    pub(super) fn reuse(mut self) {
        let mut state = self.pool.state.lock().unwrap();
        if !state.closed {
            state.idle.push(self.session.take().unwrap());
            state.checked_out -= 1;
            // A cancelled waiter may observe the release and exit. Wake all
            // so it cannot consume the only notification for this session.
            self.pool.changed.notify_all();
        }
    }
}

impl<S> Drop for Checkout<'_, S> {
    fn drop(&mut self) {
        if let Some(session) = self.session.take() {
            drop(session);
            let mut state = self.pool.state.lock().unwrap();
            state.checked_out -= 1;
            if state.checked_out == 0 {
                self.pool.changed.notify_all();
            }
        }
    }
}

#[cfg(test)]
#[path = "effect_pool_tests.rs"]
mod tests;
