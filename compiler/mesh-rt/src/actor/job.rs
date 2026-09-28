//! Job (async task) runtime support for Mesh.
//!
//! Jobs provide a simple async computation pattern:
//! - `Job.async(fn)` spawns a linked actor that runs the function and sends
//!   its result back
//! - `Job.await(pid)` blocks until the job completes, returning `Result<T, String>`
//! - `Job.await_timeout(pid, ms)` same as await but with timeout
//! - `Job.map(list, fn)` spawns parallel jobs and collects results in order
//!
//! ## Message Protocol
//!
//! Jobs use `JOB_RESULT_TAG` (u64::MAX - 1) to distinguish job results from
//! exit signals (`EXIT_SIGNAL_TAG` = u64::MAX). A job actor is linked to its
//! caller as it is spawned, and the caller traps that link: a job that fails
//! sends its caller an exit signal, which its await reports, instead of
//! taking the caller down. The job actor:
//! 1. Calls fn_ptr(env_ptr) to get the result
//! 2. Sends [JOB_RESULT_TAG][job_pid][result][is_pointer] to the caller
//! 3. Exits normally
//!
//! ## Result Layout
//!
//! Returns a `MeshResult` (same as File/IO/JSON):
//! - tag 0 = Ok (value is the job's return value)
//! - tag 1 = Err (value is a string describing the crash reason)

use crate::gc::mesh_gc_alloc_actor;
use crate::io::{alloc_result, err_result};

use super::heap::MessageBuffer;
use super::link::EXIT_SIGNAL_TAG;
use super::process::{Message, ProcessId};

/// Type tag for job result messages.
///
/// Distinct from EXIT_SIGNAL_TAG (u64::MAX) to allow the await logic to
/// differentiate between "job completed with a value" and "job crashed".
pub const JOB_RESULT_TAG: u64 = u64::MAX - 1;

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
    spawn_job(caller, job_entry as *const u8, &words)
}

/// Spawn a job actor for `caller`, which traps its link to it, handing
/// `entry` the argument `words`. They go on the caller's heap, so that the
/// job gets its own copy and what they point at stays alive for it (see
/// `adopt_spawn_args`).
fn spawn_job(caller: ProcessId, entry: *const u8, words: &[u64]) -> u64 {
    let size = std::mem::size_of_val(words);
    let args = mesh_gc_alloc_actor(size as u64, 8) as *mut u64;
    unsafe { std::ptr::copy_nonoverlapping(words.as_ptr(), args, words.len()) };
    super::global_scheduler()
        .spawn_linked(entry, args as *const u8, size as u64, 1, caller, true)
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
    send_job_result(arg(args, 2), result, arg(args, 3) as *const u32);
}

/// Send a finished job's result to its caller, tagged with JOB_RESULT_TAG.
///
/// The job actor is about to exit and take its heap with it, so whatever the
/// result references leaves that heap first, guided by `result_shape`. The
/// compiler gives a shape exactly when the result word is a reference.
fn send_job_result(caller_pid: u64, result: i64, result_shape: *const u32) {
    let sched = super::global_scheduler();

    // Message layout: [u64 JOB_RESULT_TAG][u64 job_pid][i64 result][u64 result_is_pointer]
    const RESULT_OFFSET: usize = 16;
    let (job, _) = super::running_process();
    let mut msg_data = Vec::with_capacity(32);
    msg_data.extend_from_slice(&JOB_RESULT_TAG.to_le_bytes());
    msg_data.extend_from_slice(&job.as_u64().to_le_bytes());
    msg_data.extend_from_slice(&result.to_le_bytes());
    msg_data.extend_from_slice(&u64::from(!result_shape.is_null()).to_le_bytes());

    let mut buffer = MessageBuffer::new(msg_data, JOB_RESULT_TAG);
    super::detach_from_sender(sched, &mut buffer, RESULT_OFFSET, result_shape);

    // The caller waits for this: it goes in even when the mailbox is full. A
    // caller that has gone takes no result.
    let target = ProcessId(caller_pid);
    if let Some(proc_arc) = sched.get_process(target) {
        buffer.addressed_to(&proc_arc);
        let proc = proc_arc.lock();
        let _ = proc.mailbox.try_push_control(Message { buffer });
        sched.wake_if_waiting(target, proc);
    }
}

/// Block until the job completes and return a `MeshResult`.
///
/// Receives messages from the job actor:
/// - `JOB_RESULT_TAG` message: extract the value, return Ok(value)
/// - `EXIT_SIGNAL_TAG` message: the job crashed, return Err(reason)
///
/// - `job_pid`: PID of the job actor to receive from
///
/// Returns a pointer to a heap-allocated MeshResult.
#[no_mangle]
pub extern "C-unwind" fn mesh_job_await(job_pid: u64) -> *const u8 {
    decode_job_message(super::receive_matching_or_stop(from_job(job_pid)))
}

/// Block until the job completes or timeout, returning a `MeshResult`.
///
/// Same as `mesh_job_await` but with a timeout in milliseconds.
/// If timeout expires before a result arrives, returns Err("timeout").
///
/// - `job_pid`: PID of the job actor
/// - `timeout_ms`: timeout in milliseconds
///
/// Returns a pointer to a heap-allocated MeshResult.
#[no_mangle]
pub extern "C-unwind" fn mesh_job_await_timeout(job_pid: u64, timeout_ms: i64) -> *const u8 {
    let msg_ptr = super::actor_receive_matching(timeout_ms, from_job(job_pid));
    if msg_ptr.is_null() {
        return err_result("timeout") as *const u8;
    }
    decode_job_message(msg_ptr)
}

/// Whether a message is job `job_pid`'s result, or its exit signal.
fn from_job(job_pid: u64) -> impl Fn(&Message) -> bool {
    move |message| {
        let pid_offset = match message.buffer.type_tag {
            JOB_RESULT_TAG => 8,
            EXIT_SIGNAL_TAG => 0,
            _ => return false,
        };
        let pid = &message.buffer.data[pid_offset..pid_offset + 8];
        u64::from_le_bytes(pid.try_into().unwrap()) == job_pid
    }
}

/// Decode a received message into a MeshResult.
///
/// Message layout from actor_receive: [u64 type_tag][u64 data_len][u8... data]
fn decode_job_message(msg_ptr: *const u8) -> *const u8 {
    unsafe {
        let word = |offset: usize| (msg_ptr.add(offset) as *const u64).read_unaligned();
        if word(0) == JOB_RESULT_TAG {
            // Data after the 16-byte header:
            // [u64 JOB_RESULT_TAG][u64 job_pid][i64 result][u64 result_is_pointer]
            // A `Result` payload is a pointer: a String, list or other
            // reference IS that pointer, exactly as `err_result` stores its
            // message, while a scalar sits in a box the consumer loads the
            // concrete T from. Boxing a reference made every `Ok(text)` read
            // the box as if it were the string.
            let result_value = word(32) as i64;
            let payload = if word(40) != 0 {
                result_value as usize as *mut u8
            } else {
                box_job_value(result_value)
            };
            alloc_result(0, payload) as *const u8
        } else {
            // The job's exit signal, the only other message it is waited for
            // by: its exit reason says what became of it.
            let data = std::slice::from_raw_parts(msg_ptr.add(16), word(8) as usize);
            let reason = match super::link::decode_exit_signal(data).map(|(_, reason)| reason) {
                Some(super::process::ExitReason::Normal) => "normal".to_string(),
                Some(super::process::ExitReason::Killed) => "killed".to_string(),
                Some(super::process::ExitReason::Shutdown) => "shutdown".to_string(),
                Some(
                    super::process::ExitReason::Error(text)
                    | super::process::ExitReason::Custom(text),
                ) => text,
                _ => "job crashed".to_string(),
            };
            err_result(&reason) as *const u8
        }
    }
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

    let (caller, _) = super::running_process();
    let len = mesh_list_length(list_ptr as *mut u8);
    let jobs: Vec<u64> = (0..len)
        .map(|i| {
            let element = mesh_list_get(list_ptr as *mut u8, i);
            let (function, env, shape) = (fn_ptr as u64, env_ptr as u64, result_shape as u64);
            let words = [function, env, element, caller.as_u64(), shape];
            spawn_job(caller, map_job_entry as *const u8, &words)
        })
        .collect();

    // Await each job in order and build the result list.
    let mut result_list = mesh_list_new();
    for job in jobs {
        let message = super::receive_matching_or_stop(from_job(job));
        result_list = mesh_list_append(result_list, decode_job_message(message) as u64);
    }
    result_list as *const u8
}

/// Entry function for map job actors:
/// `[fn_ptr][env_ptr][element][caller][result_shape]`.
extern "C-unwind" fn map_job_entry(args: *const u8) {
    let user_fn: extern "C-unwind" fn(*const u8, i64) -> i64 =
        unsafe { std::mem::transmute(arg(args, 0) as *const u8) };
    let result = user_fn(arg(args, 1) as *const u8, arg(args, 2) as i64);
    send_job_result(arg(args, 3), result, arg(args, 4) as *const u32);
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
        in_actor(|| send_job_result(u64::MAX >> 24, 1, std::ptr::null()));
    }

    /// What a crashed job's `await` says, for each way its actor can end.
    #[test]
    fn a_crashed_job_is_an_error_naming_its_exit_reason() {
        use crate::actor::process::ExitReason;
        crate::gc::mesh_rt_init();
        let error_of = |data: Vec<u8>| {
            let mut message = EXIT_SIGNAL_TAG.to_le_bytes().to_vec();
            message.extend_from_slice(&(data.len() as u64).to_le_bytes());
            message.extend_from_slice(&data);
            outcome(decode_job_message(message.as_ptr()))
        };
        let signal =
            |reason: ExitReason| super::super::link::encode_exit_signal(ProcessId(9), &reason);
        for (reason, text) in [
            (ExitReason::Normal, "normal"),
            (ExitReason::Killed, "killed"),
            (ExitReason::Shutdown, "shutdown"),
            (ExitReason::Error("boom".to_string()), "boom"),
            (ExitReason::Custom("mine".to_string()), "mine"),
            (ExitReason::Noconnection, "job crashed"),
        ] {
            assert_eq!(error_of(signal(reason)), (1, text.to_string()));
        }
        assert_eq!(
            error_of(vec![1, 2, 3]),
            (1, "job crashed".to_string()),
            "cut short"
        );
    }

    #[test]
    fn test_job_result_tag_distinct_from_exit() {
        assert_ne!(JOB_RESULT_TAG, EXIT_SIGNAL_TAG);
        assert_eq!(JOB_RESULT_TAG, u64::MAX - 1);
        assert_eq!(EXIT_SIGNAL_TAG, u64::MAX);
    }

    #[test]
    fn test_decode_job_result_message() {
        crate::gc::mesh_rt_init();

        // Build a message as it would appear after mesh_actor_receive:
        // [u64 type_tag][u64 data_len][u64 JOB_RESULT_TAG][u64 job_pid][i64 result_value][u64 is_pointer]
        let mut msg = Vec::new();
        msg.extend_from_slice(&JOB_RESULT_TAG.to_le_bytes()); // type_tag
        msg.extend_from_slice(&32u64.to_le_bytes()); // data_len
        msg.extend_from_slice(&JOB_RESULT_TAG.to_le_bytes()); // data: tag
        msg.extend_from_slice(&42u64.to_le_bytes()); // data: job pid
        msg.extend_from_slice(&99i64.to_le_bytes()); // data: value
        msg.extend_from_slice(&0u64.to_le_bytes()); // data: a scalar

        assert_eq!(
            outcome(decode_job_message(msg.as_ptr())),
            (0, "99".to_string())
        );
    }
}
