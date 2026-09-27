//! Service runtime support for Mesh.
//!
//! Provides synchronous call/reply semantics on top of the actor message
//! passing primitives. A "service call" sends a message to a service actor,
//! then blocks the caller until a reply arrives.
//!
//! ## Message format
//!
//! **Call message TO service:** `[u64 type_tag][u64 caller_pid][i64... args]`
//! - type_tag: identifies which call/cast handler to dispatch to
//! - caller_pid: so the service knows where to send the reply
//! - args: handler arguments encoded as i64 values
//!
//! **Reply TO caller:** `[i64 reply_value]`, tagged `SERVICE_REPLY_TAG`
//! - A single i64 value (the return value from the call handler)
//!
//! The caller waits for exactly its reply, leaving the rest of its mailbox
//! as it is, and watches the service meanwhile: a call to a service that has
//! stopped, or that stops before it replies, panics in the caller.

use super::heap::MessageBuffer;
use super::process::{Message, ProcessId};
use super::GLOBAL_SCHEDULER;

/// Header tag of a service's reply.
pub(crate) const SERVICE_REPLY_TAG: u64 = u64::MAX - 6;
/// Header tag of the notice a caller gets when the service it waits on ends.
pub(crate) const SERVICE_GONE_TAG: u64 = u64::MAX - 7;

/// Synchronous service call: send a message to the target service and block
/// until a reply arrives.
///
/// 1. Get the caller's PID
/// 2. Build a call message: [u64 type_tag][u64 caller_pid][payload bytes]
/// 3. Queue it for the service, which the caller watches from now on
/// 4. Block until the reply arrives, or the service ends
/// 5. Return a pointer to the reply data
///
/// Returns a pointer to the reply message, allocated in the caller's heap:
/// `[u64 tag][u64 len][i64 reply]`.
///
/// - `target_pid`: PID of the service actor
/// - `msg_tag`: type tag identifying which handler to invoke
/// - `payload_ptr`: pointer to argument bytes (array of i64 values)
/// - `payload_size`: size of the payload in bytes
#[no_mangle]
pub extern "C-unwind" fn mesh_service_call(
    target_pid: u64,
    msg_tag: u64,
    payload_ptr: *const u8,
    payload_size: u64,
) -> *const u8 {
    mesh_service_call_shaped(
        target_pid,
        msg_tag,
        payload_ptr,
        payload_size,
        std::ptr::null(),
    )
}

/// Service messages are `[u64 tag][u64 caller_pid][u64 args...]`.
const PAYLOAD_OFFSET: usize = 16;

/// A service call whose arguments reference heap values. `shape` describes the
/// payload's argument slots (see `msg_shape`), so the service gets its own
/// copies while the caller still owns its heap.
#[no_mangle]
pub extern "C-unwind" fn mesh_service_call_shaped(
    target_pid: u64,
    msg_tag: u64,
    payload_ptr: *const u8,
    payload_size: u64,
    shape: *const u32,
) -> *const u8 {
    let sched = super::global_scheduler();
    let (caller, me) = super::running_process();
    let target = ProcessId(target_pid);
    if !target.is_local() {
        crate::panic::raise(format_args!(
            "service call to {target}: a service is called on its own node"
        ));
    }

    // [u64 type_tag][u64 caller_pid][payload bytes]
    let mut data = msg_tag.to_le_bytes().to_vec();
    data.extend_from_slice(&caller.as_u64().to_le_bytes());
    data.extend_from_slice(&super::message_bytes(payload_ptr, payload_size));
    // The service reads the handler's tag from the data; its receive takes
    // a program's messages.
    let mut buffer = MessageBuffer::new(data, super::PROGRAM_MESSAGE_TAG);
    super::detach_from_sender(sched, &mut buffer, PAYLOAD_OFFSET, shape);

    // Watched before the call is queued, so that however the service ends,
    // the caller hears of it rather than waiting for good.
    let gone = MessageBuffer::new(Vec::new(), SERVICE_GONE_TAG);
    let watch = super::watch(sched, &me, caller, target, gone);
    if super::deliver_local(sched, target, Message { buffer }) >= 2 {
        stop_watching(sched, &me, caller, watch);
        crate::panic::raise(format_args!(
            "service call to {target}: the service's mailbox is full"
        ));
    }
    let reply = super::actor_receive_matching(-1, |message| {
        matches!(
            message.buffer.type_tag,
            SERVICE_REPLY_TAG | SERVICE_GONE_TAG
        )
    });
    stop_watching(sched, &me, caller, watch);
    // The program is ending while the service still works on the call: the
    // caller stops, as a blocking `receive` does then.
    if reply.is_null() {
        super::mesh_actor_stop();
    }
    if unsafe { (reply as *const u64).read() } == SERVICE_GONE_TAG {
        crate::panic::raise(format_args!(
            "service call to {target}: the service stopped before it replied"
        ));
    }
    reply
}

/// End a call's watch on its service. Once the watch has fired, its notice
/// is queued, unless the call took it for its answer: discard it.
fn stop_watching(
    sched: &super::Scheduler,
    me: &std::sync::Arc<parking_lot::Mutex<super::Process>>,
    caller: ProcessId,
    watch: u64,
) {
    if !super::unwatch(sched, caller, watch) {
        me.lock()
            .mailbox
            .remove_first(|message| message.buffer.type_tag == SERVICE_GONE_TAG);
    }
}

/// Send a reply from the service actor back to the caller.
///
/// Called by the service's receive loop after processing a call handler.
/// The reply is a single i64 value sent as a raw message to the caller.
///
/// - `caller_pid`: PID of the caller that made the service call
/// - `reply_ptr`: pointer to the reply data bytes
/// - `reply_size`: size of the reply data in bytes
#[no_mangle]
pub extern "C" fn mesh_service_reply(caller_pid: u64, reply_ptr: *const u8, reply_size: u64) {
    mesh_service_reply_shaped(caller_pid, reply_ptr, reply_size, std::ptr::null());
}

/// A reply that references heap values. It must survive the service changing
/// state or terminating, so the caller gets its own copy.
#[no_mangle]
pub extern "C" fn mesh_service_reply_shaped(
    caller_pid: u64,
    reply_ptr: *const u8,
    reply_size: u64,
    shape: *const u32,
) {
    let sched = super::global_scheduler();
    let mut buffer = MessageBuffer::new(
        super::message_bytes(reply_ptr, reply_size),
        SERVICE_REPLY_TAG,
    );
    super::detach_from_sender(sched, &mut buffer, 0, shape);
    // The caller waits for this and nothing else: it goes in even when its
    // mailbox is full. A caller that has gone takes no reply.
    if let Some(caller) = sched.get_process(ProcessId(caller_pid)) {
        buffer.addressed_to(&caller);
        let caller = caller.lock();
        let _ = caller.mailbox.try_push_control(Message { buffer });
        sched.wake_if_waiting(ProcessId(caller_pid), caller);
    }
}

/// Fire-and-forget service message whose arguments reference heap values.
#[no_mangle]
pub extern "C" fn mesh_service_cast_shaped(
    target_pid: u64,
    data: *const u8,
    size: u64,
    shape: *const u32,
) {
    let bytes = unsafe { std::slice::from_raw_parts(data, size as usize) }.to_vec();
    let mut buffer = MessageBuffer::new(bytes, super::PROGRAM_MESSAGE_TAG);
    if let Some(sched) = GLOBAL_SCHEDULER.get() {
        super::detach_from_sender(sched, &mut buffer, PAYLOAD_OFFSET, shape);
    }
    send_owned(target_pid, buffer);
}

fn send_owned(target_pid: u64, mut buffer: MessageBuffer) {
    if let Some(sched) = GLOBAL_SCHEDULER.get() {
        if let Some(target) = sched.get_process(ProcessId(target_pid)) {
            buffer.addressed_to(&target);
            let mut proc = target.lock();
            proc.mailbox.push(Message { buffer });
            if matches!(proc.state, super::process::ProcessState::Waiting)
                && proc.set_live_state(super::process::ProcessState::Ready)
            {
                let worker = proc.worker;
                drop(proc);
                sched.wake_worker(worker, ProcessId(target_pid));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_payload_owns_strings_after_sender_storage_changes() {
        use crate::actor::msg_shape::{self, AGG, LEAF};

        // [tag][caller][String arg][Int arg], with the string on the caller's heap.
        let mut caller = crate::actor::ActorHeap::new();
        let value = "retained-é".as_bytes();
        let source = caller.alloc(8 + value.len(), 8);
        unsafe {
            (source as *mut u64).write(value.len() as u64);
            std::ptr::copy_nonoverlapping(value.as_ptr(), source.add(8), value.len());
        }
        let mut data = vec![0; 16];
        data.extend_from_slice(&(source as u64).to_ne_bytes());
        data.extend_from_slice(&42u64.to_ne_bytes());
        let shape = [6, AGG, 1, 0, 5, LEAF];

        let captured =
            unsafe { msg_shape::capture(&caller, &data, PAYLOAD_OFFSET, shape[..].as_ptr()) };
        unsafe { std::ptr::write_bytes(source, 0, 8 + value.len()) };

        assert_eq!(captured.relocs, vec![(16, 0)]);
        assert_eq!(&captured.objects[0].bytes[8..], value);
        assert_eq!(&data[24..], &42u64.to_ne_bytes());
    }

    #[test]
    fn test_service_reply_sends_message() {
        // Test that the call message format is correct.
        let msg_tag: u64 = 42;
        let caller: u64 = 123;
        let mut data = Vec::new();
        data.extend_from_slice(&msg_tag.to_le_bytes());
        data.extend_from_slice(&caller.to_le_bytes());
        data.extend_from_slice(&99i64.to_le_bytes()); // one arg

        assert_eq!(data.len(), 24); // 8 + 8 + 8

        // Verify we can decode the message format.
        let decoded_tag = u64::from_le_bytes(data[0..8].try_into().unwrap());
        let decoded_caller = u64::from_le_bytes(data[8..16].try_into().unwrap());
        let decoded_arg = i64::from_le_bytes(data[16..24].try_into().unwrap());

        assert_eq!(decoded_tag, 42);
        assert_eq!(decoded_caller, 123);
        assert_eq!(decoded_arg, 99);
    }

    #[test]
    fn test_service_call_message_no_args() {
        // A call message with no payload arguments.
        let msg_tag: u64 = 7;
        let caller: u64 = 456;
        let mut data = Vec::new();
        data.extend_from_slice(&msg_tag.to_le_bytes());
        data.extend_from_slice(&caller.to_le_bytes());

        assert_eq!(data.len(), 16); // just tag + caller_pid

        let decoded_tag = u64::from_le_bytes(data[0..8].try_into().unwrap());
        let decoded_caller = u64::from_le_bytes(data[8..16].try_into().unwrap());

        assert_eq!(decoded_tag, 7);
        assert_eq!(decoded_caller, 456);
    }

    use crate::actor::process::{ExitReason, ProcessState};
    use crate::actor::{global_scheduler, mesh_rt_init_actor, stack, Mailbox, PROGRAM_MESSAGE_TAG};

    type Shared = std::sync::Arc<parking_lot::Mutex<crate::actor::Process>>;

    fn process(pid: ProcessId) -> Shared {
        global_scheduler().get_process(pid).unwrap()
    }

    /// A caller and a service, as processes nothing runs.
    fn caller_and_service() -> (ProcessId, ProcessId) {
        mesh_rt_init_actor(1);
        let sched = global_scheduler();
        (sched.create_main_process(), sched.create_main_process())
    }

    /// Call `service` as `caller`: the reply's word, or the panic it raised.
    fn call(caller: ProcessId, service: ProcessId) -> Result<u64, String> {
        stack::set_current_pid(caller);
        let result = std::panic::catch_unwind(|| {
            let reply = mesh_service_call(service.as_u64(), 3, std::ptr::null(), 0);
            unsafe { (reply.add(16) as *const u64).read() }
        });
        stack::clear_current_pid();
        result.map_err(|panic| *panic.downcast::<String>().unwrap())
    }

    fn queue(pid: ProcessId, word: u64, tag: u64) {
        let buffer = MessageBuffer::new(word.to_le_bytes().to_vec(), tag);
        process(pid).lock().mailbox.push(Message { buffer });
    }

    fn mailbox_tags(pid: ProcessId) -> Vec<u64> {
        let process = process(pid);
        let process = process.lock();
        std::iter::from_fn(|| process.mailbox.pop())
            .map(|message| message.buffer.type_tag)
            .collect()
    }

    fn stop(pid: ProcessId) {
        process(pid).lock().state = ProcessState::Exited(ExitReason::Normal);
    }

    #[test]
    #[should_panic(expected = "compiled code runs in a process")]
    fn a_call_from_outside_any_process_is_a_bug_in_its_caller() {
        mesh_rt_init_actor(1);
        stack::clear_current_pid();
        mesh_service_call(1, 0, std::ptr::null(), 0);
    }

    /// The reply is the message tagged as one, not whatever came first; the
    /// rest of the mailbox stays as it was.
    #[test]
    fn a_call_takes_its_reply_and_leaves_other_messages_queued() {
        let (caller, service) = caller_and_service();
        queue(caller, 99, PROGRAM_MESSAGE_TAG);
        queue(caller, 42, SERVICE_REPLY_TAG);

        assert_eq!(call(caller, service), Ok(42));
        assert_eq!(mailbox_tags(caller), [PROGRAM_MESSAGE_TAG]);
        let request = process(service).lock().mailbox.pop().unwrap();
        assert_eq!(request.buffer.data[8..16], caller.as_u64().to_le_bytes());
        assert!(process(service).lock().monitored_by.is_empty());
    }

    /// A service that has ended answers no call: the caller panics.
    #[test]
    fn a_call_to_a_stopped_service_panics() {
        let (caller, service) = caller_and_service();
        stop(service);

        let error = call(caller, service).expect_err("no service to reply");
        assert!(error.contains("the service stopped before it replied"));
        assert!(process(caller).lock().monitors.is_empty());
        assert!(mailbox_tags(caller).is_empty());
    }

    /// A reply that came before the service ended is the answer, and the
    /// notice of its end goes unread.
    #[test]
    fn a_reply_before_the_service_ends_wins() {
        let (caller, service) = caller_and_service();
        stop(service);
        queue(caller, 7, SERVICE_REPLY_TAG);

        assert_eq!(call(caller, service), Ok(7));
        assert!(mailbox_tags(caller).is_empty(), "the notice is discarded");
    }

    #[test]
    fn a_call_to_a_full_mailbox_panics() {
        let (caller, service) = caller_and_service();
        process(service).lock().mailbox = std::sync::Arc::new(Mailbox::bounded(0, 1024));

        let error = call(caller, service).expect_err("no room for the call");
        assert!(error.contains("the service's mailbox is full"));
        assert!(process(caller).lock().monitors.is_empty());
        assert!(process(service).lock().monitored_by.is_empty());
    }

    #[test]
    fn a_call_to_another_node_panics() {
        let (caller, _) = caller_and_service();
        let remote = ProcessId::from_remote(3, 0, 9);

        let error = call(caller, remote).expect_err("a remote service is not called");
        assert!(error.contains("a service is called on its own node"));
    }

    /// A reply goes to a caller whose mailbox is full, which waits for it,
    /// and nowhere once the caller has gone.
    #[test]
    fn a_reply_reaches_a_full_mailbox_and_skips_a_missing_caller() {
        let (caller, _) = caller_and_service();
        process(caller).lock().mailbox = std::sync::Arc::new(Mailbox::bounded(0, 1024));
        let reply = 5u64.to_le_bytes();

        mesh_service_reply(caller.as_u64(), reply.as_ptr(), 8);
        mesh_service_reply(u64::MAX >> 24, reply.as_ptr(), 8);

        assert_eq!(mailbox_tags(caller), [SERVICE_REPLY_TAG]);
    }
}
