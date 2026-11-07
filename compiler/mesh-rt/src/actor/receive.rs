//! Application selective receive. Saved messages keep both their heap root and
//! mailbox reservation until an arm consumes them or the actor is finalized.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::mailbox::Reservation;
use super::{stack, ProcessState};

struct SavedMessage {
    pointer: usize,
    _reservation: Reservation,
}

struct Scan {
    next: usize,
    remaining: Option<usize>,
    deadline: Option<Instant>,
}

impl Scan {
    fn finished(&self, now: Instant) -> bool {
        self.remaining == Some(0) || self.deadline.is_some_and(|deadline| now >= deadline)
    }

    fn advance(&mut self) {
        self.next += 1;
        if let Some(remaining) = &mut self.remaining {
            *remaining -= 1;
        }
    }
}

#[derive(Default)]
pub(crate) struct ReceiveState {
    saved: VecDeque<SavedMessage>,
    scan: Option<Scan>,
}

impl ReceiveState {
    pub(crate) fn assert_idle(&self) {
        assert!(
            self.scan.is_none(),
            "receive is not allowed inside a receive matching guard"
        );
    }

    pub(crate) fn roots(&self) -> impl Iterator<Item = usize> + '_ {
        self.saved.iter().map(|message| message.pointer)
    }

    pub(crate) fn pop_front(&mut self) -> Option<*const u8> {
        self.saved
            .pop_front()
            .map(|message| message.pointer as *const u8)
    }
}

#[no_mangle]
pub extern "C-unwind" fn mesh_actor_receive_begin(timeout_ms: i64) {
    let (pid, me) = super::running_process();
    let mut process = me.lock();
    process.receive.assert_idle();
    let remaining =
        (timeout_ms == 0).then(|| process.receive.saved.len() + process.mailbox.application_len());
    process.receive.scan = Some(Scan {
        next: 0,
        remaining,
        deadline: (timeout_ms > 0)
            .then(|| Instant::now() + Duration::from_millis(timeout_ms as u64)),
    });
    if stack::CURRENT_YIELDER.with(|slot| slot.yielder.get().is_some()) {
        if let Some(deadline) = process.receive.scan.as_ref().unwrap().deadline {
            super::wake_at(pid, deadline);
        }
    }
}

/// Return each candidate once. Failed matches already live in `saved`, so a
/// yield, guard panic, or cancellation never leaves a detached unrooted value.
#[no_mangle]
pub extern "C-unwind" fn mesh_actor_receive_next() -> *const u8 {
    let scheduler = super::global_scheduler();
    let (pid, me) = super::running_process();
    let in_coroutine = stack::CURRENT_YIELDER.with(|slot| slot.yielder.get().is_some());
    let mut wait = super::MainThreadWait::new();
    super::mesh_reduction_check();
    loop {
        let ending =
            in_coroutine && scheduler.is_shutdown() && !super::others_running(scheduler, pid);
        let mut process = me.lock();
        if matches!(process.state, ProcessState::Exited(_)) {
            process.receive.scan = None;
            drop(process);
            super::mesh_actor_stop();
        }
        let scan = process
            .receive
            .scan
            .as_ref()
            .expect("receive matching has begun");
        if scan.finished(Instant::now()) {
            process.receive.scan = None;
            return std::ptr::null();
        }
        let index = scan.next;
        if index == process.receive.saved.len() {
            if let Some((message, reservation)) = process.mailbox.retain_application() {
                let pointer = super::copy_message(&mut process, message) as usize;
                process.receive.saved.push_back(SavedMessage {
                    pointer,
                    _reservation: reservation,
                });
            }
        }
        if let Some(message) = process.receive.saved.get(index) {
            let pointer = message.pointer as *const u8;
            process.receive.scan.as_mut().unwrap().advance();
            return pointer;
        }
        if ending {
            process.receive.scan = None;
            return std::ptr::null();
        }
        if in_coroutine {
            // The expiry check and Waiting publication share the lock taken
            // by the timer reactor and senders, so neither wakeup can be lost.
            process.set_live_state(ProcessState::Waiting);
            drop(process);
            stack::yield_current();
        } else {
            drop(process);
            wait.pause();
        }
    }
}

/// The generated decision tree calls this before the selected arm body, which
/// is free to receive again. Only this transition releases mailbox capacity.
#[no_mangle]
pub extern "C-unwind" fn mesh_actor_receive_accept() {
    let (_, me) = super::running_process();
    let mut process = me.lock();
    if matches!(process.state, ProcessState::Exited(_)) {
        drop(process);
        super::mesh_actor_stop();
    }
    let scan = process
        .receive
        .scan
        .take()
        .expect("receive matching has begun");
    process
        .receive
        .saved
        .remove(scan.next.checked_sub(1).expect("a candidate was tested"))
        .expect("the selected candidate remains saved");
}

/// A nonlocal return/break/continue from a guard abandons this scan while
/// keeping all its messages available to the next receive.
#[no_mangle]
pub extern "C-unwind" fn mesh_actor_receive_abort() {
    let (_, me) = super::running_process();
    me.lock().receive.scan = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::{
        self, heap::MessageBuffer, mailbox::Mailbox, Message, Priority, Process, ProcessId,
    };
    use std::sync::{Arc, Barrier};

    struct MainProcess(ProcessId);
    impl MainProcess {
        fn new() -> Self {
            actor::mesh_rt_init_actor(1);
            let pid = actor::global_scheduler().create_main_process();
            stack::set_current_pid(pid);
            Self(pid)
        }
        fn send(&self, value: u64) -> i64 {
            actor::local_send(self.0.as_u64(), value.to_le_bytes().as_ptr(), 8)
        }
    }
    impl Drop for MainProcess {
        fn drop(&mut self) {
            actor::global_scheduler()
                .process_table()
                .write()
                .remove(&self.0);
            stack::clear_current_pid();
        }
    }
    fn word(pointer: *const u8) -> u64 {
        assert!(!pointer.is_null());
        unsafe { pointer.add(16).cast::<u64>().read() }
    }

    fn nested_message() -> Message {
        use actor::{heap::ActorHeap, msg_shape};
        let text = b"nested-payload";
        let size = 8 + text.len();
        let mut sender = ActorHeap::new();
        let string = sender.alloc(size, 8);
        unsafe {
            string.cast::<usize>().write(text.len());
            std::ptr::copy_nonoverlapping(text.as_ptr(), string.add(8), text.len());
        }
        // Message.Keep(Inner.Payload(String, Int)) places the string at byte 9.
        let mut data = vec![0; 25];
        data[9..17].copy_from_slice(&(string as usize).to_ne_bytes());
        data[17..25].copy_from_slice(&42u64.to_ne_bytes());
        let shape = [
            17,
            msg_shape::SUM,
            1,
            0,
            1,
            1,
            7,
            msg_shape::SUM,
            1,
            0,
            2,
            8,
            15,
            16,
            16,
            msg_shape::STRING,
            msg_shape::SCALAR,
        ];
        let captured = unsafe { msg_shape::capture(&sender, &data, 0, shape.as_ptr()) };
        assert_eq!(captured.relocs, vec![(9, 0)]);
        assert_eq!(captured.objects.len(), 1);
        let mut buffer = MessageBuffer::new(data, 0);
        buffer.captured = captured;
        Message { buffer }
    }

    #[test]
    fn saved_capacity_order_and_one_copy_survive_multiple_scans() {
        let main = MainProcess::new();
        let me = actor::global_scheduler().get_process(main.0).unwrap();
        me.lock().mailbox = Arc::new(Mailbox::bounded(3, 24));
        for value in [1, 2, 3] {
            assert_eq!(main.send(value), 0);
        }
        mesh_actor_receive_begin(0);
        let first = mesh_actor_receive_next();
        assert_eq!(word(first), 1);
        assert_eq!(word(mesh_actor_receive_next()), 2);
        assert_eq!(main.send(4), 2, "saved messages still occupy capacity");
        assert_eq!(me.lock().mailbox.byte_len(), 24);
        mesh_actor_receive_accept();
        assert_eq!(main.send(4), 0, "only consumption releases capacity");
        assert_eq!(
            me.lock().mailbox.try_push_control(Message {
                buffer: MessageBuffer::new(99u64.to_le_bytes().to_vec(), 99),
            }),
            Ok(())
        );
        assert_eq!(
            word(actor::actor_receive_matching(0, |message| message
                .buffer
                .type_tag
                == 99)),
            99
        );
        assert_eq!(
            me.lock().mailbox.len(),
            3,
            "reply selection preserves saved capacity"
        );
        mesh_actor_receive_begin(0);
        assert_eq!(
            mesh_actor_receive_next(),
            first,
            "retained message was not recopied"
        );
        assert_eq!(word(mesh_actor_receive_next()), 3);
        mesh_actor_receive_accept();
        assert_eq!(word(actor::mesh_actor_receive(0)), 1);
        assert_eq!(word(actor::mesh_actor_receive(0)), 4);
        assert_eq!(me.lock().mailbox.len(), 0);
        assert_eq!(me.lock().mailbox.byte_len(), 0);
    }

    #[test]
    fn arrival_between_candidates_is_ordered_and_internal_reply_is_not_tested() {
        let main = MainProcess::new();
        main.send(1);
        let barrier = Arc::new(Barrier::new(2));
        let sender_barrier = barrier.clone();
        let pid = main.0;
        let sender = std::thread::spawn(move || {
            sender_barrier.wait();
            let process = actor::global_scheduler().get_process(pid).unwrap();
            process.lock().mailbox.push(Message {
                buffer: MessageBuffer::new(99u64.to_le_bytes().to_vec(), 99),
            });
            assert_eq!(
                actor::local_send(pid.as_u64(), 2u64.to_le_bytes().as_ptr(), 8),
                0
            );
            sender_barrier.wait();
        });
        mesh_actor_receive_begin(-1);
        assert_eq!(word(mesh_actor_receive_next()), 1);
        barrier.wait();
        barrier.wait();
        assert_eq!(word(mesh_actor_receive_next()), 2);
        mesh_actor_receive_accept();
        assert_eq!(
            word(actor::actor_receive_matching(0, |msg| msg.buffer.type_tag == 99)),
            99
        );
        assert_eq!(word(actor::mesh_actor_receive(0)), 1);
        sender.join().unwrap();
    }

    #[test]
    fn zero_timeout_scans_only_the_initial_queue_and_next_receive_retests() {
        let main = MainProcess::new();
        main.send(1);
        mesh_actor_receive_begin(0);
        assert_eq!(word(mesh_actor_receive_next()), 1);
        main.send(2);
        assert!(mesh_actor_receive_next().is_null());
        mesh_actor_receive_begin(0);
        assert_eq!(word(mesh_actor_receive_next()), 1);
        assert_eq!(word(mesh_actor_receive_next()), 2);
        assert!(mesh_actor_receive_next().is_null());
        assert_eq!(word(actor::mesh_actor_receive(0)), 1);
        assert_eq!(word(actor::mesh_actor_receive(0)), 2);
    }

    #[test]
    fn injected_deadline_does_not_restart_when_a_message_arrives() {
        let main = MainProcess::new();
        let me = actor::global_scheduler().get_process(main.0).unwrap();
        main.send(1);
        mesh_actor_receive_begin(60_000);
        let deadline = me.lock().receive.scan.as_ref().unwrap().deadline.unwrap();
        assert_eq!(word(mesh_actor_receive_next()), 1);
        main.send(2);
        assert_eq!(
            me.lock().receive.scan.as_ref().unwrap().deadline,
            Some(deadline)
        );
        me.lock().receive.scan.as_mut().unwrap().deadline = Some(Instant::now());
        assert!(mesh_actor_receive_next().is_null());
        assert_eq!(word(actor::mesh_actor_receive(0)), 1);
        assert_eq!(word(actor::mesh_actor_receive(0)), 2);
    }

    #[test]
    fn guard_reentrancy_panics_and_finalization_releases_retained_capacity() {
        let main = MainProcess::new();
        let me = actor::global_scheduler().get_process(main.0).unwrap();
        me.lock().mailbox.push(nested_message());
        mesh_actor_receive_begin(0);
        mesh_actor_receive_next();
        assert!(std::panic::catch_unwind(|| actor::mesh_actor_receive(0)).is_err());
        assert!(std::panic::catch_unwind(|| mesh_actor_receive_begin(0)).is_err());
        let mut process = me.lock();
        process.begin_exit_finalization(actor::ExitReason::Killed);
        process.receive.assert_idle();
        assert_eq!(process.mailbox.len(), 0);
        assert_eq!(process.mailbox.byte_len(), 0);
        assert_eq!(process.receive.roots().count(), 0);
        process.heap.collect(std::ptr::null(), std::ptr::null());
        assert_eq!(process.heap.total_bytes(), 0);
    }

    #[test]
    fn nested_captured_payload_survives_saved_roots_without_stack_scanning() {
        let main = MainProcess::new();
        let me = actor::global_scheduler().get_process(main.0).unwrap();
        let text = b"nested-payload";
        let size = 8 + text.len();
        me.lock().mailbox.push(nested_message());

        mesh_actor_receive_begin(0);
        let pointer = mesh_actor_receive_next();
        let payload = unsafe { pointer.add(25).cast::<*const u8>().read_unaligned() };
        let mut guard = me.lock();
        let process = &mut *guard;
        let before = process.heap.total_bytes();
        let empty = std::ptr::null();
        process
            .heap
            .collect_with_roots(empty, empty, process.receive.roots());
        let live = process.heap.live_allocation_size(payload);
        let after = process.heap.total_bytes();
        let reused = process.heap.alloc(size, 8);
        eprintln!(
            "nested payload live={live:?}; bytes before={before}, after={after}; reused={}",
            std::ptr::eq(reused, payload)
        );
        assert!(live.is_some(), "saved nested string was swept");
        assert!(
            !std::ptr::eq(reused, payload),
            "saved nested string was reused"
        );
        assert_eq!(
            unsafe { std::slice::from_raw_parts(payload.add(8), text.len()) },
            text
        );
        assert_eq!(after, before, "all captured bytes remain accounted for");
        assert!(process.heap_borrows.is_empty());
        assert_eq!(process.mailbox.byte_len(), 25);
        drop(guard);

        mesh_actor_receive_abort();
        {
            let mut guard = me.lock();
            let process = &mut *guard;
            process
                .heap
                .collect_with_roots(empty, empty, process.receive.roots());
            assert_eq!(process.heap.total_bytes(), before);
        }
        mesh_actor_receive_begin(0);
        assert_eq!(mesh_actor_receive_next(), pointer, "no second copy");
        mesh_actor_receive_accept();
        let mut process = me.lock();
        assert_eq!(process.mailbox.byte_len(), 0);
        assert_eq!(process.receive.roots().count(), 0);
        let value_root = [payload as usize];
        process.heap.collect(
            unsafe { value_root.as_ptr().add(1).cast() },
            value_root.as_ptr().cast(),
        );
        assert_eq!(
            process.heap.total_bytes(),
            size + actor::heap::GC_HEADER_SIZE
        );
        process.heap.collect(empty, empty);
        assert_eq!(process.heap.total_bytes(), 0);
    }

    #[test]
    fn ordinary_receive_releases_saved_root_and_keeps_only_the_consumed_value() {
        let main = MainProcess::new();
        let me = actor::global_scheduler().get_process(main.0).unwrap();
        me.lock().mailbox.push(nested_message());
        mesh_actor_receive_begin(0);
        let pointer = mesh_actor_receive_next();
        assert!(mesh_actor_receive_next().is_null());
        assert_eq!(actor::mesh_actor_receive(0), pointer);
        let value_root = [unsafe { pointer.add(25).cast::<usize>().read_unaligned() }];
        let mut process = me.lock();
        assert_eq!(process.mailbox.len(), 0);
        assert_eq!(process.mailbox.byte_len(), 0);
        assert_eq!(process.receive.roots().count(), 0);
        process.heap.collect(
            unsafe { value_root.as_ptr().add(1).cast() },
            value_root.as_ptr().cast(),
        );
        assert!(process.heap.live_allocation_size(pointer).is_none());
        assert!(process
            .heap
            .live_allocation_size(value_root[0] as *const u8)
            .is_some());
        process.heap.collect(std::ptr::null(), std::ptr::null());
        assert_eq!(process.heap.total_bytes(), 0);
    }

    #[test]
    fn shared_and_interior_references_keep_the_existing_process_loan_lifetime() {
        use actor::msg_shape;
        let main = MainProcess::new();
        let me = actor::global_scheduler().get_process(main.0).unwrap();
        let owner = Arc::new(parking_lot::Mutex::new(Process::new(
            ProcessId::next(),
            Priority::Normal,
        )));
        let mut data = vec![0; 25];
        let captured = {
            let mut process = owner.lock();
            let interior = unsafe { process.heap.alloc(16, 8).add(8) } as usize;
            let shared = process.heap.alloc(16, 8) as usize;
            data[1..9].copy_from_slice(&interior.to_ne_bytes());
            data[9..17].copy_from_slice(&shared.to_ne_bytes());
            data[17..25].copy_from_slice(&73usize.to_ne_bytes());
            let shape = [
                11,
                msg_shape::AGG,
                3,
                1,
                9,
                9,
                10,
                17,
                10,
                msg_shape::STRING,
                msg_shape::SHARED,
            ];
            let captured = unsafe { msg_shape::capture(&process.heap, &data, 0, shape.as_ptr()) };
            assert!(captured.objects.is_empty());
            assert_eq!(captured.lend, vec![interior, shared, 73]);
            captured
        };
        let mut buffer = MessageBuffer::new(data, 0);
        buffer.borrows = actor::scheduler::lend_words(&owner, &captured.lend);
        buffer.captured = captured;
        assert_eq!(buffer.borrows.len(), 1);
        let loan = Arc::downgrade(&buffer.borrows[0]._lent);
        me.lock().mailbox.push(Message { buffer });
        mesh_actor_receive_begin(0);
        mesh_actor_receive_next();
        mesh_actor_receive_abort();
        owner
            .lock()
            .heap
            .collect(std::ptr::null(), std::ptr::null());
        assert_eq!(owner.lock().heap.total_bytes(), 64);
        actor::mesh_actor_receive(0);
        assert_eq!(me.lock().mailbox.byte_len(), 0);
        assert_eq!(
            loan.strong_count(),
            1,
            "a consumed value can still borrow its owner"
        );
        owner
            .lock()
            .heap
            .collect(std::ptr::null(), std::ptr::null());
        assert_eq!(owner.lock().heap.total_bytes(), 64);
        drop(me);
        drop(main);
        assert_eq!(loan.strong_count(), 0);
        owner
            .lock()
            .heap
            .collect(std::ptr::null(), std::ptr::null());
        assert_eq!(owner.lock().heap.total_bytes(), 0);
    }

    #[test]
    fn guard_panic_releases_nested_payload_and_capacity() {
        actor::mesh_rt_init_actor(1);
        extern "C-unwind" fn entry(args: *const u8) {
            let barrier = unsafe { Box::from_raw(args as *mut Arc<Barrier>) };
            let (_, me) = actor::running_process();
            me.lock().mailbox.push(nested_message());
            mesh_actor_receive_begin(-1);
            mesh_actor_receive_next();
            barrier.wait();
            barrier.wait();
            panic!("nested payload guard panic");
        }
        let barrier = Arc::new(Barrier::new(2));
        let scheduler = actor::global_scheduler();
        let pid = scheduler.spawn(
            entry as *const u8,
            Box::into_raw(Box::new(barrier.clone())).cast(),
            0,
            1,
        );
        let process = scheduler.get_process(pid).unwrap();
        barrier.wait();
        assert_eq!(process.lock().mailbox.byte_len(), 25);
        barrier.wait();
        let limit = Instant::now() + Duration::from_secs(5);
        while scheduler.get_process(pid).is_some() {
            assert!(Instant::now() < limit, "panicking actor was not finalized");
            std::thread::yield_now();
        }
        let mut process = process.lock();
        process.receive.assert_idle();
        assert_eq!(process.mailbox.byte_len(), 0);
        assert_eq!(process.receive.roots().count(), 0);
        process.heap.collect(std::ptr::null(), std::ptr::null());
        assert_eq!(process.heap.total_bytes(), 0);
    }

    #[test]
    fn saved_payload_is_an_explicit_transitive_gc_root() {
        let mut process = Process::new(ProcessId::next(), Priority::Normal);
        let nested = process.heap.alloc(8, 8);
        unsafe {
            nested.cast::<u64>().write(12345);
        }
        process.mailbox.push(Message {
            buffer: MessageBuffer::new((nested as u64).to_le_bytes().to_vec(), 0),
        });
        let (message, reservation) = process.mailbox.retain_application().unwrap();
        let pointer = actor::copy_message(&mut process, message) as usize;
        process.receive.saved.push_back(SavedMessage {
            pointer,
            _reservation: reservation,
        });
        let empty = std::ptr::null();
        process
            .heap
            .collect_with_roots(empty, empty, process.receive.roots());
        for _ in 0..100 {
            process.heap.alloc(8, 8);
        }
        let payload = word(process.receive.pop_front().unwrap()) as *const u64;
        assert_eq!(unsafe { payload.read() }, 12345);
        process.heap.collect(empty, empty);
        assert_eq!(process.heap.total_bytes(), 0);
    }
    #[test]
    fn waiting_scan_wakes_for_arrival_without_retesting_saved_message() {
        actor::mesh_rt_init_actor(1);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let sender = std::thread::spawn(move || {
            let pid = ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            let scheduler = actor::global_scheduler();
            let process = scheduler.get_process(pid).unwrap();
            let limit = Instant::now() + Duration::from_secs(5);
            while !matches!(process.lock().state, ProcessState::Waiting) {
                assert!(Instant::now() < limit, "scan did not suspend");
                std::thread::yield_now();
            }
            assert_eq!(process.lock().mailbox.len(), 1);
            assert_eq!(
                actor::local_send(pid.as_u64(), 2u64.to_le_bytes().as_ptr(), 8),
                0
            );
        });
        let result = actor::in_actor(move || {
            let (pid, _) = actor::running_process();
            actor::local_send(pid.as_u64(), 1u64.to_le_bytes().as_ptr(), 8);
            mesh_actor_receive_begin(5000);
            assert_eq!(word(mesh_actor_receive_next()), 1);
            ready_tx.send(pid).unwrap();
            let matched = word(mesh_actor_receive_next());
            mesh_actor_receive_accept();
            (matched, word(actor::mesh_actor_receive(0)))
        });
        sender.join().unwrap();
        assert_eq!(result, (2, 1));
    }

    #[test]
    fn cancellation_of_a_suspended_scan_releases_saved_reservations() {
        actor::mesh_rt_init_actor(1);
        extern "C-unwind" fn entry(_: *const u8) {
            let (_, me) = actor::running_process();
            me.lock().mailbox.push(nested_message());
            mesh_actor_receive_begin(-1);
            mesh_actor_receive_next();
            mesh_actor_receive_next();
        }
        let scheduler = actor::global_scheduler();
        let pid = scheduler.spawn(entry as *const u8, std::ptr::null(), 0, 1);
        let process = scheduler.get_process(pid).unwrap();
        let limit = Instant::now() + Duration::from_secs(5);
        while !matches!(process.lock().state, ProcessState::Waiting) {
            assert!(Instant::now() < limit, "scan did not suspend");
            std::thread::yield_now();
        }
        let mailbox = process.lock().mailbox.clone();
        assert_eq!(mailbox.len(), 1);
        assert_eq!(mailbox.byte_len(), 25);
        {
            let mut guard = process.lock();
            let process = &mut *guard;
            let bytes = process.heap.total_bytes();
            process.heap.collect_with_roots(
                std::ptr::null(),
                std::ptr::null(),
                process.receive.roots(),
            );
            assert_eq!(process.heap.total_bytes(), bytes);
        }
        actor::deliver_exit_signal(scheduler, pid, actor::ExitReason::Killed);
        while scheduler.get_process(pid).is_some() {
            assert!(Instant::now() < limit, "cancelled actor was not finalized");
            std::thread::yield_now();
        }
        process.lock().receive.assert_idle();
        assert_eq!(mailbox.len(), 0);
        assert_eq!(mailbox.byte_len(), 0);
        let mut process = process.lock();
        assert_eq!(process.receive.roots().count(), 0);
        process.heap.collect(std::ptr::null(), std::ptr::null());
        assert_eq!(process.heap.total_bytes(), 0);
    }

    #[test]
    fn all_small_selection_sets_agree_with_fifo_model() {
        for mask in 0..16 {
            let main = MainProcess::new();
            for n in 0..4 {
                main.send(n);
            }
            mesh_actor_receive_begin(0);
            let mut selected = None;
            loop {
                let candidate = mesh_actor_receive_next();
                if candidate.is_null() {
                    break;
                }
                let n = word(candidate);
                if mask & (1 << n) != 0 {
                    mesh_actor_receive_accept();
                    selected = Some(n);
                    break;
                }
            }
            assert_eq!(selected, (0..4).find(|n| mask & (1 << n) != 0));
            let remaining: Vec<_> = (0..4).filter(|n| Some(*n) != selected).collect();
            let actual: Vec<_> = (0..remaining.len())
                .map(|_| word(actor::mesh_actor_receive(0)))
                .collect();
            assert_eq!(actual, remaining);
            assert!(actor::mesh_actor_receive(0).is_null());
        }
    }
    #[test]
    fn cancellation_during_guard_prevents_the_selected_body() {
        use std::sync::atomic::{AtomicBool, Ordering};
        actor::mesh_rt_init_actor(1);
        struct Gate {
            barrier: Arc<Barrier>,
            ran_body: Arc<AtomicBool>,
        }
        extern "C-unwind" fn entry(args: *const u8) {
            let gate = unsafe { Box::from_raw(args as *mut Gate) };
            let (_, me) = actor::running_process();
            me.lock().mailbox.push(nested_message());
            mesh_actor_receive_begin(-1);
            mesh_actor_receive_next();
            gate.barrier.wait();
            gate.barrier.wait();
            mesh_actor_receive_accept();
            gate.ran_body.store(true, Ordering::Release);
        }
        let barrier = Arc::new(Barrier::new(2));
        let ran_body = Arc::new(AtomicBool::new(false));
        let gate = Box::new(Gate {
            barrier: barrier.clone(),
            ran_body: ran_body.clone(),
        });
        let scheduler = actor::global_scheduler();
        let pid = scheduler.spawn(entry as *const u8, Box::into_raw(gate).cast(), 0, 1);
        let process = scheduler.get_process(pid).unwrap();
        barrier.wait();
        actor::deliver_exit_signal(scheduler, pid, actor::ExitReason::Killed);
        barrier.wait();
        let limit = Instant::now() + Duration::from_secs(5);
        while scheduler.get_process(pid).is_some() {
            assert!(Instant::now() < limit, "cancelled guard was not finalized");
            std::thread::yield_now();
        }
        assert!(!ran_body.load(Ordering::Acquire));
        let mut process = process.lock();
        process.receive.assert_idle();
        assert_eq!(process.mailbox.len(), 0);
        assert_eq!(process.mailbox.byte_len(), 0);
        assert_eq!(process.receive.roots().count(), 0);
        process.heap.collect(std::ptr::null(), std::ptr::null());
        assert_eq!(process.heap.total_bytes(), 0);
    }
}
