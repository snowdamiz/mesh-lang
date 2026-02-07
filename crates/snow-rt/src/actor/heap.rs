//! Per-actor bump allocator heap.
//!
//! Each Snow actor gets its own heap for memory allocation. This eliminates
//! global arena contention and enables per-actor memory reclamation when an
//! actor terminates or during per-actor GC (future).
//!
//! The allocation algorithm is identical to the global Arena in `gc.rs`:
//! a bump allocator with page-based backing storage.

/// Default page size for actor heaps: 64 KiB.
const ACTOR_PAGE_SIZE: usize = 64 * 1024;

/// Per-actor bump allocator heap.
///
/// Owns a list of pages and bump-allocates within the current page.
/// When a page is exhausted, a new one is allocated. Memory is only
/// reclaimed when `reset()` is called (e.g., on actor termination or GC).
pub struct ActorHeap {
    /// Owned page list. Each page is a heap-allocated byte buffer.
    pages: Vec<Vec<u8>>,
    /// Bump offset into the current (last) page.
    offset: usize,
    /// Total bytes allocated (across all pages) for GC trigger heuristics.
    total_allocated: usize,
}

impl ActorHeap {
    /// Create a new per-actor heap with an initial 64 KiB page.
    pub fn new() -> Self {
        let mut heap = ActorHeap {
            pages: Vec::new(),
            offset: 0,
            total_allocated: 0,
        };
        heap.pages.push(vec![0u8; ACTOR_PAGE_SIZE]);
        heap
    }

    /// Bump-allocate `size` bytes with the given `align`ment.
    ///
    /// Returns a pointer to zeroed memory within this actor's heap.
    /// The pointer is valid until `reset()` is called or the heap is dropped.
    pub fn alloc(&mut self, size: usize, align: usize) -> *mut u8 {
        let align = if align == 0 { 1 } else { align };

        if self.pages.is_empty() {
            self.pages.push(vec![0u8; ACTOR_PAGE_SIZE]);
            self.offset = 0;
        }

        // Align the current offset within the current page.
        let current_page_len = self.pages.last().map_or(0, |p| p.len());
        let aligned_offset = (self.offset + align - 1) & !(align - 1);

        if aligned_offset + size <= current_page_len {
            // Fits in the current page.
            let page = self.pages.last_mut().unwrap();
            let ptr = page[aligned_offset..].as_mut_ptr();
            self.offset = aligned_offset + size;
            self.total_allocated += size;
            ptr
        } else {
            // Allocate a new page. If the requested size exceeds the default
            // page size, allocate a page large enough to hold it.
            let new_page_size = if size > ACTOR_PAGE_SIZE {
                size + align
            } else {
                ACTOR_PAGE_SIZE
            };
            let mut page = vec![0u8; new_page_size];
            let aligned_start = {
                let base = page.as_ptr() as usize;
                let aligned = (base + align - 1) & !(align - 1);
                aligned - base
            };
            let ptr = page[aligned_start..].as_mut_ptr();
            self.offset = aligned_start + size;
            self.total_allocated += size;
            self.pages.push(page);
            ptr
        }
    }

    /// Drop all pages and start fresh.
    ///
    /// Used for actor termination cleanup or future per-actor GC.
    pub fn reset(&mut self) {
        self.pages.clear();
        self.offset = 0;
        self.total_allocated = 0;
    }

    /// Returns the total number of bytes allocated from this heap.
    pub fn total_bytes(&self) -> usize {
        self.total_allocated
    }
}

impl Default for ActorHeap {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ActorHeap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActorHeap")
            .field("pages", &self.pages.len())
            .field("offset", &self.offset)
            .field("total_allocated", &self.total_allocated)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// MessageBuffer
// ---------------------------------------------------------------------------

/// A serialized message representation for cross-heap copying.
///
/// When an actor sends a message to another actor, the data is serialized
/// into a `MessageBuffer` and then deep-copied into the target actor's heap.
/// This ensures complete isolation between actor heaps.
#[derive(Debug, Clone)]
pub struct MessageBuffer {
    /// Raw serialized message bytes.
    pub data: Vec<u8>,
    /// Type tag for pattern matching dispatch.
    ///
    /// In Phase 6, this is a simple hash of the type name. Future phases
    /// may use a more sophisticated type identification scheme.
    pub type_tag: u64,
}

impl MessageBuffer {
    /// Create a new message buffer from raw bytes and a type tag.
    pub fn new(data: Vec<u8>, type_tag: u64) -> Self {
        MessageBuffer { data, type_tag }
    }

    /// Deep-copy this message's data into the target actor's heap.
    ///
    /// Allocates space in the target heap, copies the data bytes, and
    /// returns a pointer to the copy within the target heap.
    pub fn deep_copy_to_heap(&self, heap: &mut ActorHeap) -> *mut u8 {
        if self.data.is_empty() {
            return std::ptr::null_mut();
        }
        let ptr = heap.alloc(self.data.len(), 8);
        // Safety: ptr points to a valid allocation of at least self.data.len() bytes.
        unsafe {
            std::ptr::copy_nonoverlapping(self.data.as_ptr(), ptr, self.data.len());
        }
        ptr
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_actor_heap_basic_alloc() {
        let mut heap = ActorHeap::new();
        let ptr1 = heap.alloc(16, 8);
        assert!(!ptr1.is_null());

        let ptr2 = heap.alloc(32, 8);
        assert!(!ptr2.is_null());

        // Pointers should be different.
        assert_ne!(ptr1, ptr2);
    }

    #[test]
    fn test_actor_heap_alignment() {
        let mut heap = ActorHeap::new();

        // Test various alignments.
        for &align in &[1, 2, 4, 8, 16, 32, 64] {
            let ptr = heap.alloc(8, align);
            assert!(!ptr.is_null());
            assert_eq!(
                ptr as usize % align,
                0,
                "pointer should be {}-byte aligned, got {:p}",
                align,
                ptr
            );
        }
    }

    #[test]
    fn test_actor_heap_large_alloc() {
        let mut heap = ActorHeap::new();
        // Allocate more than a page.
        let ptr = heap.alloc(128 * 1024, 8);
        assert!(!ptr.is_null());
        assert!(heap.pages.len() >= 2, "should have allocated a new page");
    }

    #[test]
    fn test_actor_heap_reset() {
        let mut heap = ActorHeap::new();

        // Allocate some memory.
        heap.alloc(1024, 8);
        heap.alloc(2048, 8);
        assert!(heap.total_bytes() > 0);
        assert!(!heap.pages.is_empty());

        // Reset should clear everything.
        heap.reset();
        assert_eq!(heap.total_bytes(), 0);
        assert!(heap.pages.is_empty());
        assert_eq!(heap.offset, 0);
    }

    #[test]
    fn test_actor_heap_total_bytes() {
        let mut heap = ActorHeap::new();
        assert_eq!(heap.total_bytes(), 0);

        heap.alloc(100, 8);
        assert_eq!(heap.total_bytes(), 100);

        heap.alloc(200, 8);
        assert_eq!(heap.total_bytes(), 300);
    }

    #[test]
    fn test_message_buffer_deep_copy() {
        let data = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
        let msg = MessageBuffer::new(data.clone(), 42);

        let mut target_heap = ActorHeap::new();
        let ptr = msg.deep_copy_to_heap(&mut target_heap);

        assert!(!ptr.is_null());

        // Verify the copied data matches.
        let copied = unsafe { std::slice::from_raw_parts(ptr, data.len()) };
        assert_eq!(copied, &data[..]);
    }

    #[test]
    fn test_message_buffer_empty_data() {
        let msg = MessageBuffer::new(Vec::new(), 0);
        let mut target_heap = ActorHeap::new();
        let ptr = msg.deep_copy_to_heap(&mut target_heap);
        assert!(ptr.is_null());
    }

    #[test]
    fn test_message_buffer_deep_copy_isolation() {
        // Verify that modifying the source buffer after copy does not affect
        // the data in the target heap.
        let mut data = vec![10u8, 20, 30, 40];
        let msg = MessageBuffer::new(data.clone(), 99);

        let mut target_heap = ActorHeap::new();
        let ptr = msg.deep_copy_to_heap(&mut target_heap);

        // Mutate the original data.
        data[0] = 255;

        // The copied data should be unchanged.
        let copied = unsafe { std::slice::from_raw_parts(ptr, 4) };
        assert_eq!(copied, &[10, 20, 30, 40]);
    }
}
