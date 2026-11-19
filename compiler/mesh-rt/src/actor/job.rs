//! Job (async task) runtime support for Mesh.
//!
//! Jobs provide a simple async computation pattern:
//! - `Job.async(fn)` spawns a linked actor that runs the function and sends
//!   its result back
//! - `Job.await(pid)` blocks until the job completes, returning `Result<T, String>`
//! - `Job.await_timeout(pid, ms)` same as await but with timeout
//! - `Job.map(list, fn)` spawns parallel jobs and collects results in order
//!
//! The caller reserves a completion slot for each linked job before it runs.
//! The child stages a detached result, and exit finalization publishes the
//! result or exit reason in that slot.
//!
//! ## Result Layout
//!
//! Returns a `MeshResult` (same as File/IO/JSON):
//! - tag 0 = Ok (value is the job's return value)
//! - tag 1 = Err (value is a string describing the crash reason)

use std::task::Poll;

use rustc_hash::FxHashMap;

use crate::gc::mesh_gc_alloc_actor;
use crate::io::{alloc_result, err_result};

use super::heap::MessageBuffer;
use super::process::{ExitReason, Message, ProcessId, ProcessState};

/// Type tag used to materialize a detached job result.
pub const JOB_RESULT_TAG: u64 = u64::MAX - 1;

const MAX_BUFFERED_REPLIES: usize = 1024;
const MAX_BUFFERED_REPLY_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy)]
pub(super) enum JobRetention {
    Buffered,
    Reserved,
}

enum JobSlot {
    Pending {
        staged: Option<Message>,
        retention: JobRetention,
    },
    Ready {
        reply: Result<Message, ExitReason>,
        buffered_bytes: Option<usize>,
    },
}

enum JobFailure {
    Exited(ExitReason),
    Unavailable,
}

#[derive(Default)]
pub(super) struct JobSlots {
    slots: FxHashMap<ProcessId, JobSlot>,
    buffered: usize,
    buffered_bytes: usize,
    waiting_for: Option<ProcessId>,
}

impl JobSlots {
    pub(super) fn contains(&self, pid: ProcessId) -> bool {
        self.slots.contains_key(&pid)
    }

    pub(super) fn register(&mut self, pid: ProcessId, retention: JobRetention) {
        self.slots.insert(
            pid,
            JobSlot::Pending {
                staged: None,
                retention,
            },
        );
    }

    fn stage(&mut self, pid: ProcessId, message: Message) {
        if let Some(JobSlot::Pending { staged, .. }) = self.slots.get_mut(&pid) {
            *staged = Some(message);
        }
    }

    pub(super) fn finish(
        &mut self,
        pid: ProcessId,
        reason: &ExitReason,
        returned_normally: bool,
    ) -> bool {
        let Some(JobSlot::Pending { staged, retention }) = self.slots.get_mut(&pid) else {
            return false;
        };

        let buffered_bytes = matches!(retention, JobRetention::Buffered).then(|| {
            if returned_normally {
                staged.as_ref().map_or(0, |msg| {
                    msg.buffer.data().len()
                        + msg.buffer.references.as_ref().map_or(0, |refs| {
                            refs.captured
                                .objects
                                .iter()
                                .map(|obj| obj.bytes.len())
                                .sum()
                        })
                })
            } else {
                exit_reason_bytes(reason)
            }
        });
        if buffered_bytes.is_some_and(|bytes| {
            self.buffered >= MAX_BUFFERED_REPLIES
                || self.buffered_bytes.saturating_add(bytes) > MAX_BUFFERED_REPLY_BYTES
        }) && self.waiting_for != Some(pid)
        {
            self.slots.remove(&pid);
            return true;
        }

        let reply = if returned_normally {
            staged.take().ok_or(ExitReason::Normal)
        } else {
            Err(reason.clone())
        };
        self.slots.insert(
            pid,
            JobSlot::Ready {
                reply,
                buffered_bytes,
            },
        );
        if let Some(bytes) = buffered_bytes {
            self.buffered += 1;
            self.buffered_bytes += bytes;
        }
        true
    }

    fn poll(&mut self, pid: ProcessId, timeout_ms: i64) -> Poll<Result<Message, JobFailure>> {
        match self.slots.get(&pid) {
            None => return Poll::Ready(Err(JobFailure::Unavailable)),
            Some(JobSlot::Pending { .. }) => {
                if timeout_ms != 0 {
                    self.waiting_for = Some(pid);
                }
                return Poll::Pending;
            }
            Some(JobSlot::Ready { .. }) => {}
        }
        let Some(JobSlot::Ready {
            reply,
            buffered_bytes,
        }) = self.slots.remove(&pid)
        else {
            unreachable!("caller holds the completion slot lock");
        };
        if let Some(bytes) = buffered_bytes {
            self.buffered -= 1;
            self.buffered_bytes -= bytes;
        }
        Poll::Ready(reply.map_err(JobFailure::Exited))
    }

    fn end_wait(&mut self) {
        if let Some(pid) = self.waiting_for.take() {
            // A reply may arrive after the timeout wins but before the caller resumes.
            if self.buffered > MAX_BUFFERED_REPLIES
                || self.buffered_bytes > MAX_BUFFERED_REPLY_BYTES
            {
                let _ = self.poll(pid, 0);
            }
        }
    }
}

fn exit_reason_bytes(mut reason: &ExitReason) -> usize {
    while let ExitReason::Linked(_, inner) = reason {
        reason = inner;
    }
    match reason {
        ExitReason::Error(text) | ExitReason::Custom(text) => text.len(),
        _ => 0,
    }
}

/// Store a job's scalar result in owned payload memory.
///
/// Generic `Result<T, String>` uses a pointer payload slot, and pattern
/// lowering loads a scalar T through it, so a raw integer cast to a pointer
/// (for example `42 as *mut u8`) is not a valid result payload. References
/// are not boxed: they are the payload.
fn box_job_value(value: i64) -> *mut u8 {
    unsafe {
        let ptr = mesh_gc_alloc_actor(std::mem::size_of::<i64>() as u64, 8) as *mut i64;
        ptr.write(value);
        ptr.cast()
    }
}

// ---------------------------------------------------------------------------
// extern "C" ABI functions
// ---------------------------------------------------------------------------

/// Spawn an async job that runs `fn_ptr(env_ptr)` and sends its result back.
///
/// Returns the PID of the spawned job actor.
///
/// - `fn_ptr`: pointer to the function to run (signature: fn(env) -> i64)
/// - `env_ptr`: pointer to the closure environment
#[no_mangle]
pub extern "C" fn mesh_job_async(fn_ptr: *const u8, env_ptr: *const u8) -> u64 {
    mesh_job_async_shaped(fn_ptr, env_ptr, std::ptr::null())
}

/// `mesh_job_async` for a job whose result references heap values.
///
/// The job actor exits as soon as it has sent its result, and its heap goes
/// with it. `result_shape` describes the one-slot result (see `msg_shape`) so
/// the caller receives its own copy.
#[no_mangle]
pub extern "C" fn mesh_job_async_shaped(
    fn_ptr: *const u8,
    env_ptr: *const u8,
    result_shape: *const u32,
) -> u64 {
    let (caller, _) = super::running_process();
    let words = [
        fn_ptr as u64,
        env_ptr as u64,
        caller.as_u64(),
        result_shape as u64,
    ];
    spawn_job(
        caller,
        job_entry as *const u8,
        &words,
        JobRetention::Buffered,
    )
}

/// Spawn a job actor for `caller`, which traps its link to it, handing
/// `entry` the argument `words`. They go on the caller's heap, so that the
/// job gets its own copy and what they point at stays alive for it (see
/// `adopt_spawn_args`).
fn spawn_job(caller: ProcessId, entry: *const u8, words: &[u64], retention: JobRetention) -> u64 {
    let size = std::mem::size_of_val(words);
    let args = mesh_gc_alloc_actor(size as u64, 8) as *mut u64;
    unsafe { std::ptr::copy_nonoverlapping(words.as_ptr(), args, words.len()) };
    super::global_scheduler()
        .spawn_job(entry, args as *const u8, size as u64, caller, retention)
        .as_u64()
}

/// Word `index` of a job actor's arguments.
fn arg(args: *const u8, index: usize) -> u64 {
    unsafe { (args.add(8 * index) as *const u64).read_unaligned() }
}

/// Entry function for job actors: `[fn_ptr][env_ptr][caller][result_shape]`.
extern "C-unwind" fn job_entry(args: *const u8) {
    let user_fn: extern "C-unwind" fn(*const u8) -> i64 =
        unsafe { std::mem::transmute(arg(args, 0) as *const u8) };
    let result = user_fn(arg(args, 1) as *const u8);
    stage_job_result(arg(args, 2), result, arg(args, 3) as *const u32);
}

/// Stage the return value for exit finalization. The value must leave the
/// job's heap before the actor exits and takes its heap with it.
fn stage_job_result(caller_pid: u64, result: i64, result_shape: *const u32) {
    let sched = super::global_scheduler();
    let (job, _) = super::running_process();
    let Some(caller) = sched.get_process(ProcessId(caller_pid)) else {
        return;
    };

    let mut data = Vec::with_capacity(16);
    data.extend_from_slice(&result.to_le_bytes());
    data.extend_from_slice(&u64::from(!result_shape.is_null()).to_le_bytes());
    let mut buffer = MessageBuffer::new(data, JOB_RESULT_TAG);
    super::detach_from_sender(sched, &mut buffer, 0, result_shape);
    buffer.addressed_to(&caller);

    let mut caller = caller.lock();
    if !matches!(caller.state, ProcessState::Exited(_)) {
        caller.jobs.stage(job, Message { buffer });
    }
}

/// Block until the job completes and return its result.
#[no_mangle]
pub extern "C-unwind" fn mesh_job_await(job_pid: u64) -> *const u8 {
    await_job(job_pid, -1).unwrap_or_else(|| super::mesh_actor_stop())
}

/// Wait up to `timeout_ms` for the job's result without cancelling the job.
#[no_mangle]
pub extern "C-unwind" fn mesh_job_await_timeout(job_pid: u64, timeout_ms: i64) -> *const u8 {
    await_job(job_pid, timeout_ms).unwrap_or_else(|| err_result("timeout") as *const u8)
}

fn await_job(job_pid: u64, timeout_ms: i64) -> Option<*const u8> {
    let job_pid = ProcessId(job_pid);
    let reply = super::receive_with(super::global_scheduler(), timeout_ms, |caller| match caller
        .jobs
        .poll(job_pid, timeout_ms)
    {
        Poll::Pending => None,
        Poll::Ready(reply) => {
            Some(reply.map(|message| super::copy_msg_to_actor_heap(caller, message)))
        }
    });
    if timeout_ms != 0 {
        super::running_process().1.lock().jobs.end_wait();
    }
    Some(match reply? {
        Ok(message) => decode_job_result(message),
        Err(JobFailure::Exited(reason)) => job_exit_result(reason),
        Err(JobFailure::Unavailable) => err_result("job result unavailable") as *const u8,
    })
}

fn decode_job_result(message: *const u8) -> *const u8 {
    unsafe {
        // The 16-byte message header is followed by the result and a pointer flag.
        let result = (message.add(16) as *const i64).read_unaligned();
        let is_pointer = (message.add(24) as *const u64).read_unaligned() != 0;
        let payload = if is_pointer {
            result as usize as *mut u8
        } else {
            box_job_value(result)
        };
        alloc_result(0, payload) as *const u8
    }
}

fn job_exit_result(reason: ExitReason) -> *const u8 {
    let message = match &reason {
        ExitReason::Normal => "normal",
        ExitReason::Killed => "killed",
        ExitReason::Shutdown => "shutdown",
        ExitReason::Error(text) | ExitReason::Custom(text) => text,
        _ => "job crashed",
    };
    err_result(message) as *const u8
}

/// Spawn parallel jobs for each element of a list, collect results in order.
///
/// For each element in the input list:
/// 1. Spawn a job that calls `fn_ptr(env_ptr, element)` (note: the closure
///    receives the element as its argument, with env_ptr for captures)
/// 2. Collect all job PIDs
/// 3. Await each job in order
/// 4. Build a result list of MeshResult values
///
/// - `list_ptr`: pointer to a Mesh list (MeshList)
/// - `fn_ptr`: pointer to the mapping function
/// - `env_ptr`: pointer to the closure environment
///
/// Returns a pointer to a new Mesh list containing MeshResult values.
#[no_mangle]
pub extern "C-unwind" fn mesh_job_map(
    list_ptr: *const u8,
    fn_ptr: *const u8,
    env_ptr: *const u8,
) -> *const u8 {
    mesh_job_map_shaped(list_ptr, fn_ptr, env_ptr, std::ptr::null())
}

/// `mesh_job_map` for a mapping function whose results reference heap values;
/// see `mesh_job_async_shaped`.
#[no_mangle]
pub extern "C-unwind" fn mesh_job_map_shaped(
    list_ptr: *const u8,
    fn_ptr: *const u8,
    env_ptr: *const u8,
    result_shape: *const u32,
) -> *const u8 {
    use crate::collections::list::{
        mesh_list_append, mesh_list_get, mesh_list_length, mesh_list_new,
    };

    let (caller, process) = super::running_process();
    let len = mesh_list_length(list_ptr as *mut u8);
    let jobs: Vec<u64> = (0..len)
        .map(|i| {
            let element = mesh_list_get(list_ptr as *mut u8, i);
            let (function, env, shape) = (fn_ptr as u64, env_ptr as u64, result_shape as u64);
            let words = [function, env, element, caller.as_u64(), shape];
            spawn_job(
                caller,
                map_job_entry as *const u8,
                &words,
                JobRetention::Reserved,
            )
        })
        .collect();

    // Await each job in order and build the result list.
    let mut result_list = mesh_list_new();
    for job in jobs {
        result_list = mesh_list_append(result_list, mesh_job_await(job) as u64);
    }
    process.lock().jobs.slots.shrink_to(MAX_BUFFERED_REPLIES);
    result_list as *const u8
}

/// Entry function for map job actors:
/// `[fn_ptr][env_ptr][element][caller][result_shape]`.
extern "C-unwind" fn map_job_entry(args: *const u8) {
    let user_fn: extern "C-unwind" fn(*const u8, i64) -> i64 =
        unsafe { std::mem::transmute(arg(args, 0) as *const u8) };
    let result = user_fn(arg(args, 1) as *const u8, arg(args, 2) as i64);
    stage_job_result(arg(args, 3), result, arg(args, 4) as *const u32);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::in_actor;
    use crate::io::MeshResult;

    /// A `MeshResult` as `(tag, scalar or error text)`.
    fn outcome(result: *const u8) -> (u8, String) {
        let result = unsafe { &*(result as *const MeshResult) };
        let value = if result.tag == 0 {
            unsafe { *(result.value as *const i64) }.to_string()
        } else {
            unsafe { (*(result.value as *const crate::string::MeshString)).as_str() }.to_string()
        };
        (result.tag, value)
    }

    extern "C-unwind" fn forty_two(_env: *const u8) -> i64 {
        42
    }

    extern "C-unwind" fn failing(_env: *const u8) -> i64 {
        crate::panic::raise(format_args!("the job failed"))
    }

    extern "C-unwind" fn slow(_env: *const u8) -> i64 {
        crate::actor::mesh_timer_sleep(50);
        7
    }

    extern "C-unwind" fn tenfold_or_fail(_env: *const u8, n: i64) -> i64 {
        if n == 2 {
            crate::panic::raise(format_args!("element two failed"))
        }
        n * 10
    }

    /// A job that fails is an `Err` to the actor that awaits it, which goes
    /// on; one that is slow is a timeout first and its result later.
    #[test]
    fn await_reports_a_result_a_failure_and_a_timeout() {
        let outcomes = in_actor(|| {
            let job = |f: extern "C-unwind" fn(*const u8) -> i64| {
                mesh_job_async(f as *const u8, std::ptr::null())
            };
            let (ok, failed, slow_job) = (job(forty_two), job(failing), job(slow));
            // A program's own message waits for the actor's receive.
            let (me, _) = crate::actor::running_process();
            crate::actor::local_send(me.as_u64(), 5u64.to_le_bytes().as_ptr(), 8);
            [
                outcome(mesh_job_await_timeout(ok, 5_000)),
                outcome(mesh_job_await(failed)),
                outcome(mesh_job_await_timeout(slow_job, 1)),
                outcome(mesh_job_await(slow_job)),
            ]
        });
        let error = |text: &str| (1, format!("Mesh panic: {text}"));
        assert_eq!(
            outcomes,
            [
                (0, "42".to_string()),
                error("the job failed"),
                (1, "timeout".to_string()),
                (0, "7".to_string())
            ]
        );
    }

    #[test]
    fn finished_jobs_keep_their_results_when_the_callers_mailbox_is_full() {
        in_actor(|| {
            let (caller, _) = super::super::running_process();
            let send =
                |n: u64| super::super::local_send(caller.as_u64(), n.to_le_bytes().as_ptr(), 8);
            for n in 0..super::super::mailbox::DEFAULT_MAILBOX_MAX_ITEMS as u64 {
                assert_eq!(send(n), 0);
            }
            assert_eq!(send(0), 2);

            let start = |f: extern "C-unwind" fn(*const u8) -> i64| {
                mesh_job_async(f as *const u8, std::ptr::null())
            };
            let jobs = [start(forty_two), start(forty_two), start(failing)];
            let scheduler = super::super::global_scheduler();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            for job in jobs {
                while scheduler.get_process(ProcessId(job)).is_some() {
                    assert!(std::time::Instant::now() < deadline, "job did not exit");
                    super::super::mesh_timer_sleep(1);
                }
            }

            assert_eq!(send(0), 2);
            assert_eq!(
                jobs.map(|job| outcome(mesh_job_await_timeout(job, 0))),
                [
                    (0, "42".into()),
                    (0, "42".into()),
                    (1, "Mesh panic: the job failed".into()),
                ]
            );
            assert_eq!(
                outcome(mesh_job_await_timeout(jobs[0], 0)),
                (1, "job result unavailable".into())
            );
            assert_eq!(send(0), 2);
        });
    }

    #[test]
    fn an_explicit_exit_wins_over_a_jobs_late_return() {
        extern "C-unwind" fn stop_before_return(_args: *const u8) -> i64 {
            let (job, _) = super::super::running_process();
            super::super::mesh_actor_exit(job.as_u64(), 0);
            42
        }

        let result = in_actor(|| {
            let job = mesh_job_async(stop_before_return as *const u8, std::ptr::null());
            outcome(mesh_job_await_timeout(job, 5_000))
        });
        assert_eq!(result, (1, "normal".into()));
    }

    #[test]
    fn unread_failures_respect_the_byte_limit_and_release_it_when_read() {
        let mut slots = JobSlots::default();
        let reason = ExitReason::Error("x".repeat(MAX_BUFFERED_REPLY_BYTES / 2));
        for pid in 1..=3 {
            slots.register(ProcessId(pid), JobRetention::Buffered);
            assert!(slots.finish(ProcessId(pid), &reason, false));
        }
        assert!(matches!(
            slots.poll(ProcessId(3), 0),
            Poll::Ready(Err(JobFailure::Unavailable))
        ));
        assert!(matches!(
            slots.poll(ProcessId(1), 0),
            Poll::Ready(Err(JobFailure::Exited(_)))
        ));
        slots.register(ProcessId(4), JobRetention::Buffered);
        assert!(slots.finish(ProcessId(4), &reason, false));
        assert!(matches!(
            slots.poll(ProcessId(4), 0),
            Poll::Ready(Err(JobFailure::Exited(_)))
        ));
    }

    #[test]
    fn excess_uncollected_replies_are_unavailable_without_blocking_job_map() {
        use crate::collections::list::{mesh_list_append, mesh_list_get, mesh_list_new};

        extern "C-unwind" fn map_value(_env: *const u8, n: i64) -> i64 {
            if n == 1 {
                super::super::mesh_timer_sleep(50);
            }
            n * 10
        }

        in_actor(|| {
            let jobs: Vec<_> = (0..=MAX_BUFFERED_REPLIES)
                .map(|_| mesh_job_async(forty_two as *const u8, std::ptr::null()))
                .collect();
            let scheduler = super::super::global_scheduler();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while jobs
                .iter()
                .any(|job| scheduler.get_process(ProcessId(*job)).is_some())
            {
                assert!(std::time::Instant::now() < deadline, "jobs did not exit");
                super::super::mesh_timer_sleep(1);
            }

            let list = mesh_list_append(mesh_list_append(mesh_list_new(), 1), 2);
            let mapped = mesh_job_map(list, map_value as *const u8, std::ptr::null()) as *mut u8;
            assert_eq!(
                outcome(mesh_list_get(mapped, 0) as *const u8),
                (0, "10".into())
            );
            assert_eq!(
                outcome(mesh_list_get(mapped, 1) as *const u8),
                (0, "20".into())
            );

            let mut successes = 0;
            for job in jobs {
                match outcome(mesh_job_await_timeout(job, 0)) {
                    (0, value) if value == "42" => successes += 1,
                    (1, error) if error == "job result unavailable" => {}
                    reply => panic!("unexpected reply: {reply:?}"),
                }
            }
            assert_eq!(successes, MAX_BUFFERED_REPLIES);
        });
    }

    /// Job.map reports each element's job, a failed one among them.
    #[test]
    fn map_reports_each_element_in_order() {
        use crate::collections::list::{mesh_list_append, mesh_list_get, mesh_list_new};
        let outcomes = in_actor(|| {
            let list = [1, 2, 3]
                .into_iter()
                .fold(mesh_list_new(), |list, n| mesh_list_append(list, n));
            let f = tenfold_or_fail as *const u8;
            let results = mesh_job_map(list, f, std::ptr::null()) as *mut u8;
            (0..3)
                .map(|i| outcome(mesh_list_get(results, i) as *const u8))
                .collect::<Vec<_>>()
        });
        assert_eq!(
            outcomes,
            [
                (0, "10".to_string()),
                (1, "Mesh panic: element two failed".to_string()),
                (0, "30".to_string())
            ]
        );
    }

    /// A job's result for a caller that has gone goes nowhere.
    #[test]
    fn a_result_for_a_caller_that_has_gone_is_dropped() {
        in_actor(|| stage_job_result(u64::MAX >> 24, 1, std::ptr::null()));
    }

    /// What a crashed job's `await` says, for each way its actor can end.
    #[test]
    fn a_crashed_job_is_an_error_naming_its_exit_reason() {
        use crate::actor::process::ExitReason;
        crate::gc::mesh_rt_init();
        for (reason, text) in [
            (ExitReason::Normal, "normal"),
            (ExitReason::Killed, "killed"),
            (ExitReason::Shutdown, "shutdown"),
            (ExitReason::Error("boom".to_string()), "boom"),
            (ExitReason::Custom("mine".to_string()), "mine"),
            (ExitReason::Noconnection, "job crashed"),
        ] {
            assert_eq!(outcome(job_exit_result(reason)), (1, text.to_string()));
        }
    }

    #[test]
    fn test_job_result_tag_distinct_from_exit() {
        assert_ne!(JOB_RESULT_TAG, super::super::link::EXIT_SIGNAL_TAG);
    }

    #[test]
    fn test_decode_job_result_message() {
        crate::gc::mesh_rt_init();

        // Build a message as it would appear after mesh_actor_receive:
        // [u64 type_tag][u64 data_len][u64 JOB_RESULT_TAG][u64 job_pid][i64 result_value][u64 is_pointer]
        let mut msg = Vec::new();
        msg.extend_from_slice(&JOB_RESULT_TAG.to_le_bytes()); // type_tag
        msg.extend_from_slice(&16u64.to_le_bytes()); // data_len
        msg.extend_from_slice(&99i64.to_le_bytes()); // data: value
        msg.extend_from_slice(&0u64.to_le_bytes()); // data: a scalar

        assert_eq!(
            outcome(decode_job_result(msg.as_ptr())),
            (0, "99".to_string())
        );
    }
}
