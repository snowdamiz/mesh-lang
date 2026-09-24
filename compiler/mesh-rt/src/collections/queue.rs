//! GC-managed immutable FIFO Queue for the Mesh runtime.
//!
//! A queue is `{ buffer, head, tail }` (three words): its elements are
//! `buffer[head..tail]`, where `buffer` is a list whose length counts the
//! slots written by the queues that share it. A push onto the newest queue
//! of a buffer writes the next slot in place; a push onto an older queue,
//! whose next slot another push took, or onto a full buffer copies the
//! queue's own elements into a new buffer with room to spare. So a push is
//! amortized O(1), and every queue keeps its elements (it used to append
//! to a list, copying it: O(n) per push). A pop moves `head`.

use super::list::{
    list_slots, mesh_list_builder_new, mesh_list_builder_push, mesh_list_new, push_in_place,
};
use crate::gc::mesh_gc_alloc_actor;

// ── Internal helpers ──────────────────────────────────────────────────

/// Queue layout: { buffer: *mut u8 (list), head: u64, tail: u64 }
const QUEUE_SIZE: u64 = 24;

unsafe fn fields(queue: *const u8) -> (*mut u8, u64, u64) {
    let words = queue as *const u64;
    (*words as *mut u8, *words.add(1), *words.add(2))
}

unsafe fn alloc_queue(buffer: *mut u8, head: u64, tail: u64) -> *mut u8 {
    let p = mesh_gc_alloc_actor(QUEUE_SIZE, 8);
    let words = p as *mut u64;
    *words = buffer as u64;
    *words.add(1) = head;
    *words.add(2) = tail;
    p
}

// ── Public API ────────────────────────────────────────────────────────

/// Create an empty queue.
#[no_mangle]
pub extern "C" fn mesh_queue_new() -> *mut u8 {
    unsafe { alloc_queue(mesh_list_new(), 0, 0) }
}

/// Push an element to the back of the queue. Returns a NEW queue.
#[no_mangle]
pub extern "C" fn mesh_queue_push(queue: *mut u8, element: u64) -> *mut u8 {
    unsafe {
        let (buffer, head, tail) = fields(queue);
        let (written, data) = list_slots(buffer);
        if tail == written as u64 && push_in_place(buffer, element) {
            return alloc_queue(buffer, head, tail + 1);
        }
        let count = tail - head;
        let mut fresh = mesh_list_builder_new(((count + 1) * 2) as i64);
        for index in head..tail {
            fresh = mesh_list_builder_push(fresh, *data.add(index as usize));
        }
        fresh = mesh_list_builder_push(fresh, element);
        alloc_queue(fresh, 0, count + 1)
    }
}

/// Pop an element from the front. Returns a tuple-like struct:
/// `{ u64 element, u64 new_queue_ptr }` (16 bytes, GC-allocated).
///
/// Panics if the queue is empty.
#[no_mangle]
pub extern "C-unwind" fn mesh_queue_pop(queue: *mut u8) -> *mut u8 {
    unsafe {
        let (buffer, head, tail) = fields(queue);
        if head == tail {
            crate::panic::raise(format_args!("Queue.pop: the queue is empty"));
        }
        let element = *list_slots(buffer).1.add(head as usize);
        let new_queue = alloc_queue(buffer, head + 1, tail);
        // Return the tuple `(element, new_queue)` in the runtime tuple layout
        // `{ u64 len, u64[len] }`, so `Tuple.first`, `Tuple.second` and
        // `let (front, rest) = ...` read it like any other tuple.
        let result = mesh_gc_alloc_actor(24, 8);
        *(result as *mut u64) = 2;
        *((result as *mut u64).add(1)) = element;
        *((result as *mut u64).add(2)) = new_queue as u64;
        result
    }
}

/// Peek at the front element without removing it. Panics if empty.
#[no_mangle]
pub extern "C-unwind" fn mesh_queue_peek(queue: *mut u8) -> u64 {
    unsafe {
        let (buffer, head, tail) = fields(queue);
        if head == tail {
            crate::panic::raise(format_args!("Queue.peek: the queue is empty"));
        }
        *list_slots(buffer).1.add(head as usize)
    }
}

/// Return the total number of elements in the queue.
#[no_mangle]
pub extern "C" fn mesh_queue_size(queue: *mut u8) -> i64 {
    unsafe {
        let (_, head, tail) = fields(queue);
        (tail - head) as i64
    }
}

/// Returns 1 if the queue is empty, 0 otherwise.
#[no_mangle]
pub extern "C" fn mesh_queue_is_empty(queue: *mut u8) -> i8 {
    if mesh_queue_size(queue) == 0 {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::mesh_rt_init;

    #[test]
    fn test_queue_new_is_empty() {
        mesh_rt_init();
        let q = mesh_queue_new();
        assert_eq!(mesh_queue_size(q), 0);
        assert_eq!(mesh_queue_is_empty(q), 1);
    }

    #[test]
    fn test_queue_push_pop_fifo() {
        mesh_rt_init();
        let q = mesh_queue_new();
        let q = mesh_queue_push(q, 10);
        let q = mesh_queue_push(q, 20);
        let q = mesh_queue_push(q, 30);
        assert_eq!(mesh_queue_size(q), 3);

        // Pop should return elements in FIFO order, as the tuple
        // `{ len: 2, element, queue }`.
        let result = mesh_queue_pop(q);
        unsafe {
            assert_eq!(*(result as *const u64), 2);
            let elem = *((result as *const u64).add(1));
            let new_q = *((result as *const u64).add(2)) as *mut u8;
            assert_eq!(elem, 10);
            assert_eq!(mesh_queue_size(new_q), 2);

            let result2 = mesh_queue_pop(new_q);
            let elem2 = *((result2 as *const u64).add(1));
            assert_eq!(elem2, 20);
        }
    }

    #[test]
    fn test_queue_peek() {
        mesh_rt_init();
        let q = mesh_queue_new();
        let q = mesh_queue_push(q, 42);
        let q = mesh_queue_push(q, 99);
        assert_eq!(mesh_queue_peek(q), 42);
        // Peek doesn't remove the element.
        assert_eq!(mesh_queue_size(q), 2);
    }

    #[test]
    fn test_queue_immutability() {
        mesh_rt_init();
        let q1 = mesh_queue_new();
        let q2 = mesh_queue_push(q1, 1);
        assert_eq!(mesh_queue_size(q1), 0);
        assert_eq!(mesh_queue_size(q2), 1);
    }

    #[test]
    fn test_queue_versions_keep_their_elements() {
        // Pushes onto one queue share its buffer; each queue still sees only
        // what was pushed onto it.
        mesh_rt_init();
        let q1 = mesh_queue_push(mesh_queue_new(), 1);
        let q2 = mesh_queue_push(q1, 2);
        let q3 = mesh_queue_push(q1, 3);
        let q4 = mesh_queue_push(q2, 4);
        assert_eq!(mesh_queue_size(q1), 1);
        let drain = |mut q: *mut u8| {
            let mut out = Vec::new();
            while mesh_queue_is_empty(q) == 0 {
                let pair = mesh_queue_pop(q);
                unsafe {
                    out.push(*((pair as *const u64).add(1)));
                    q = *((pair as *const u64).add(2)) as *mut u8;
                }
            }
            out
        };
        assert_eq!(drain(q2), vec![1, 2]);
        assert_eq!(drain(q3), vec![1, 3]);
        assert_eq!(drain(q4), vec![1, 2, 4]);
        // A popped queue pushes after its own elements.
        let rest = unsafe { *((mesh_queue_pop(q4) as *const u64).add(2)) as *mut u8 };
        assert_eq!(drain(mesh_queue_push(rest, 5)), vec![2, 4, 5]);
        assert_eq!(drain(q4), vec![1, 2, 4]);
    }

    #[test]
    fn test_queue_is_empty_after_pop_all() {
        mesh_rt_init();
        let q = mesh_queue_new();
        let q = mesh_queue_push(q, 1);
        let result = mesh_queue_pop(q);
        unsafe {
            let new_q = *((result as *const u64).add(2)) as *mut u8;
            assert_eq!(mesh_queue_is_empty(new_q), 1);
        }
    }
}
