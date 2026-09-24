//! Per-actor GC-aware heap with free-list allocator.
//!
//! Each Mesh actor gets its own heap for memory allocation. This eliminates
//! global arena contention and enables per-actor memory reclamation via
//! mark-sweep garbage collection.
//!
//! Every allocation prepends a 16-byte `GcHeader` before the user data.
//! All live objects are linked via an intrusive all-objects list for sweep
//! traversal. Freed blocks are placed in size-segregated free bins for reuse
//! before bump-allocating new pages, and each page keeps a bitmap of object
//! starts so the conservative mark phase can resolve interior pointers.

use std::ptr;
use std::sync::{Arc, Weak};

/// Default page size for actor heaps: 64 KiB.
const ACTOR_PAGE_SIZE: usize = 64 * 1024;

// ---------------------------------------------------------------------------
// GcHeader
// ---------------------------------------------------------------------------

/// Size of the GcHeader in bytes.
pub const GC_HEADER_SIZE: usize = 16;

/// Mark bit in GcHeader flags: object is reachable (set during mark phase).
pub const MARK_BIT: u8 = 0x01;

/// Free bit in GcHeader flags: object is on the free list.
pub const FREE_BIT: u8 = 0x02;

/// Object header prepended to every GC-managed allocation.
///
/// The user-visible pointer starts immediately after this header.
/// The `next` pointer serves dual purpose: when the object is live, it links
/// into the all-objects list; when freed, it links into the free list.
#[repr(C)]
pub struct GcHeader {
    /// Size of the user data in bytes (not including the header).
    pub size: u32,
    /// Flags: bit 0 = marked, bit 1 = free.
    pub flags: u8,
    /// Reserved padding for 8-byte alignment of the `next` pointer.
    pub _pad: [u8; 3],
    /// Next pointer: links into the all-objects list or free list.
    pub next: *mut GcHeader,
}

// GcHeader contains a raw pointer but is only used within a single actor's
// heap (never shared across threads). Mark as Send so ActorHeap can be Send.
unsafe impl Send for GcHeader {}

impl GcHeader {
    /// Returns true if the mark bit is set.
    #[inline]
    pub fn is_marked(&self) -> bool {
        self.flags & MARK_BIT != 0
    }

    /// Set the mark bit.
    #[inline]
    pub fn set_marked(&mut self) {
        self.flags |= MARK_BIT;
    }

    /// Clear the mark bit.
    #[inline]
    pub fn clear_marked(&mut self) {
        self.flags &= !MARK_BIT;
    }

    /// Returns true if the free bit is set.
    #[inline]
    pub fn is_free(&self) -> bool {
        self.flags & FREE_BIT != 0
    }

    /// Set the free bit.
    #[inline]
    pub fn set_free(&mut self) {
        self.flags |= FREE_BIT;
    }

    /// Clear the free bit.
    #[inline]
    pub fn clear_free(&mut self) {
        self.flags &= !FREE_BIT;
    }

    /// Returns a pointer to the user data (past the header).
    #[inline]
    pub fn data_ptr(&mut self) -> *mut u8 {
        unsafe { (self as *mut GcHeader as *mut u8).add(GC_HEADER_SIZE) }
    }

    /// Recover the GcHeader pointer from a user data pointer.
    ///
    /// # Safety
    ///
    /// `data` must point to user data that was allocated via `ActorHeap::alloc`,
    /// i.e., it must have a valid GcHeader immediately preceding it.
    #[inline]
    pub unsafe fn from_data_ptr(data: *mut u8) -> *mut GcHeader {
        data.sub(GC_HEADER_SIZE) as *mut GcHeader
    }
}

// ---------------------------------------------------------------------------
// ActorHeap
// ---------------------------------------------------------------------------

/// Default GC pressure threshold: 256 KiB.
const DEFAULT_GC_THRESHOLD: usize = 256 * 1024;

/// The threshold a heap starts from and never goes below. `MESH_GC_STRESS`
/// makes it zero, so a heap collects at every opportunity: a root the
/// collector cannot see then fails at once instead of once in a while.
fn min_gc_threshold() -> usize {
    static STRESS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *STRESS.get_or_init(|| std::env::var_os("MESH_GC_STRESS").is_some()) {
        0
    } else {
        DEFAULT_GC_THRESHOLD
    }
}

// ---------------------------------------------------------------------------
// Free bins
// ---------------------------------------------------------------------------

/// Blocks smaller than this get one bin per exact byte size, so recycling
/// same-sized objects (the common churn) is a single pop.
const SMALL_BINS: usize = 256;
/// Larger blocks share one bin per power of two, up to the `u32` size limit.
const NUM_BINS: usize = SMALL_BINS + 24;
const BITMAP_WORDS: usize = NUM_BINS.div_ceil(64);
/// A reused block may be at most this many times the requested size. Reuse
/// wipes and accounts for the whole block, so without a bound a short string
/// recycling a freed 24 KiB list pays for 24 KiB of both.
// ponytail: larger free blocks wait for a similar-sized request; split them
// (adding start bits) if idle big blocks ever matter.
const MAX_REUSE_WASTE_FACTOR: usize = 4;
/// Entries examined per bin before moving on.
// ponytail: a fitting block deeper than this is skipped and the heap grows
// instead; sort power-of-two bins by size if that ever shows up.
const MAX_BIN_PROBES: usize = 8;

fn bin_for(size: usize) -> usize {
    if size < SMALL_BINS {
        size
    } else {
        SMALL_BINS + (size.ilog2() - SMALL_BINS.ilog2()) as usize
    }
}

/// Freed blocks segregated by size, with a bitmap of the non-empty bins.
///
/// Every block in a bin above `bin_for(size)` is at least `size` bytes, so a
/// request never walks past blocks that cannot fit it.
struct FreeBins {
    heads: [*mut GcHeader; NUM_BINS],
    nonempty: [u64; BITMAP_WORDS],
}

impl FreeBins {
    fn new() -> Box<Self> {
        Box::new(FreeBins {
            heads: [ptr::null_mut(); NUM_BINS],
            nonempty: [0; BITMAP_WORDS],
        })
    }

    fn push(&mut self, header: *mut GcHeader) {
        let bin = bin_for(unsafe { (*header).size } as usize);
        unsafe { (*header).next = self.heads[bin] };
        self.heads[bin] = header;
        self.nonempty[bin / 64] |= 1 << (bin % 64);
    }

    /// First non-empty bin at or above `bin`.
    fn next_nonempty(&self, bin: usize) -> Option<usize> {
        let mut word = bin / 64;
        let mut bits = *self.nonempty.get(word)? & (u64::MAX << (bin % 64));
        while bits == 0 {
            word += 1;
            bits = *self.nonempty.get(word)?;
        }
        Some(word * 64 + bits.trailing_zeros() as usize)
    }

    /// Unlink and return a block from `bin` of `size..=max_size` bytes.
    fn take(
        &mut self,
        bin: usize,
        size: usize,
        max_size: usize,
        align: usize,
    ) -> Option<*mut GcHeader> {
        let mut prev: *mut GcHeader = ptr::null_mut();
        let mut current = self.heads[bin];
        for _ in 0..MAX_BIN_PROBES {
            if current.is_null() {
                break;
            }
            let header = unsafe { &mut *current };
            let fits = (size..=max_size).contains(&(header.size as usize));
            if fits && header.data_ptr() as usize % align == 0 {
                if prev.is_null() {
                    self.heads[bin] = header.next;
                } else {
                    unsafe { (*prev).next = header.next };
                }
                if self.heads[bin].is_null() {
                    self.nonempty[bin / 64] &= !(1 << (bin % 64));
                }
                return Some(current);
            }
            prev = current;
            current = header.next;
        }
        None
    }

    fn any(&self) -> *mut GcHeader {
        self.next_nonempty(0)
            .map_or(ptr::null_mut(), |bin| self.heads[bin])
    }
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

/// Headers start on 8-byte granules; one bitmap bit covers one granule.
const GRANULE: usize = 8;

/// One contiguous block of heap memory plus a bitmap of where objects start.
///
/// The bitmap lets the conservative scan resolve a candidate word to its
/// object without walking every object in the heap. Blocks are never split,
/// merged or moved, so bits are only ever added.
struct Page {
    /// Zeroed backing store; `u64` elements keep `base` granule-aligned.
    mem: Vec<u64>,
    base: usize,
    /// Bit `n` is set when a `GcHeader` begins at `base + n * GRANULE`.
    starts: Vec<u64>,
}

impl Page {
    fn new(bytes: usize) -> Self {
        let granules = bytes.div_ceil(GRANULE);
        let mut mem = vec![0u64; granules];
        let base = mem.as_mut_ptr() as usize;
        Page {
            mem,
            base,
            starts: vec![0; granules.div_ceil(64)],
        }
    }

    fn len(&self) -> usize {
        self.mem.len() * GRANULE
    }

    /// Offset of the header whose data pointer is the first `align`-aligned
    /// address at or after `offset + GC_HEADER_SIZE`.
    fn header_offset(&self, offset: usize, align: usize) -> usize {
        let data_addr = (self.base + offset + GC_HEADER_SIZE + align - 1) & !(align - 1);
        data_addr - GC_HEADER_SIZE - self.base
    }

    fn mark_start(&mut self, header_offset: usize) {
        let granule = header_offset / GRANULE;
        self.starts[granule / 64] |= 1 << (granule % 64);
    }

    /// The last header starting at or before `offset`.
    fn header_at_or_before(&self, offset: usize) -> Option<*mut GcHeader> {
        let granule = offset / GRANULE;
        let mut word = granule / 64;
        let mut bits = self.starts[word] & (u64::MAX >> (63 - granule % 64));
        while bits == 0 {
            word = word.checked_sub(1)?;
            bits = self.starts[word];
        }
        let start = word * 64 + 63 - bits.leading_zeros() as usize;
        Some((self.base + start * GRANULE) as *mut GcHeader)
    }
}

/// Objects lent to an actor this heap's owner spawned.
///
/// Spawn arguments are passed by reference, so the spawned actor can hold
/// pointers into this heap that no stack or object here still mentions. The
/// collector only sees its own actor's roots; without this record it would
/// free (and reuse) values the borrower is still reading.
struct Lent {
    /// Argument words pointing into this heap; marked like stack roots.
    roots: Vec<usize>,
    /// Dead once the borrowing actor's process has been dropped.
    borrower: Weak<()>,
}

impl Lent {
    fn is_live(&self) -> bool {
        self.borrower.strong_count() > 0
    }
}

/// Per-actor heap with GcHeader-prepended free-list allocator.
///
/// Owns a list of pages and bump-allocates within the current page.
/// Every allocation prepends a 16-byte `GcHeader` and links the object
/// into the `all_objects` intrusive list. Freed blocks are placed in
/// size-segregated free bins for reuse before bump-allocating new pages.
pub struct ActorHeap {
    /// Owned page list, in creation order; the last page takes bump allocations.
    /// Empty until the first allocation, so an actor that never allocates owns no memory.
    pages: Vec<Page>,
    /// `(start, end, index into pages)`, sorted by address for pointer lookup.
    page_ranges: Vec<(usize, usize, usize)>,
    /// Bump offset into the current (last) page.
    offset: usize,
    /// Total bytes allocated (including headers) for GC trigger heuristics.
    total_allocated: usize,

    /// Head of the intrusive all-objects linked list (for sweep traversal).
    all_objects: *mut GcHeader,
    /// Freed blocks available for reuse; allocated by the first sweep.
    free_bins: Option<Box<FreeBins>>,
    /// Roots held on behalf of spawned actors; see [`ActorHeap::lend`].
    lent: Vec<Lent>,

    /// Heap pressure threshold in bytes. When `total_allocated >= gc_threshold`,
    /// the GC should be triggered.
    gc_threshold: usize,
    /// Re-entrancy guard: prevents GC from triggering during GC.
    gc_in_progress: bool,
}

// Raw pointers in ActorHeap are only accessed from the owning actor's thread.
unsafe impl Send for ActorHeap {}

impl ActorHeap {
    /// Create a new, empty per-actor heap. Pages are 64 KiB.
    pub fn new() -> Self {
        ActorHeap {
            pages: Vec::new(),
            page_ranges: Vec::new(),
            offset: 0,
            total_allocated: 0,
            all_objects: ptr::null_mut(),
            free_bins: None,
            lent: Vec::new(),
            gc_threshold: min_gc_threshold(),
            gc_in_progress: false,
        }
    }

    /// Allocate `size` bytes with the given `align`ment.
    ///
    /// Returns a pointer to zeroed memory within this actor's heap.
    /// The pointer is past the GcHeader -- callers see only user data.
    /// The pointer is valid until the object is collected or `reset()` is called.
    pub fn alloc(&mut self, size: usize, align: usize) -> *mut u8 {
        self.alloc_with_size_policy(size, align, false)
    }

    /// Allocate an object whose header must describe exactly `size` bytes.
    /// Larger free blocks are left available for ordinary reuse.
    pub(crate) fn alloc_exact(&mut self, size: usize, align: usize) -> *mut u8 {
        self.alloc_with_size_policy(size, align, true)
    }

    fn alloc_with_size_policy(&mut self, size: usize, align: usize, exact: bool) -> *mut u8 {
        let align = if align == 0 { 1 } else { align };

        // 1. Try the free list first: find a block with sufficient size.
        if let Some(data_ptr) = self.alloc_from_free_list(size, align, exact) {
            return data_ptr;
        }

        // 2. Bump-allocate: GcHeader + user data from pages.
        self.bump_alloc_with_header(size, align)
    }

    /// Try to allocate from the free bins (smallest fitting bin first).
    ///
    /// A free block of `size` up to `MAX_REUSE_WASTE_FACTOR * size` bytes
    /// whose data pointer satisfies `align` may be reused; exact mode requires
    /// equal size. The block is unlinked, its full physical capacity wiped,
    /// FREE_BIT cleared, and it is linked into all_objects.
    fn alloc_from_free_list(&mut self, size: usize, align: usize, exact: bool) -> Option<*mut u8> {
        let bins = self.free_bins.as_deref_mut()?;
        let max_size = if exact {
            size
        } else {
            size.saturating_mul(MAX_REUSE_WASTE_FACTOR)
        };
        let mut bin = bin_for(size);
        let current = loop {
            bin = bins.next_nonempty(bin)?;
            if bin > bin_for(max_size) {
                return None;
            }
            if let Some(found) = bins.take(bin, size, max_size, align) {
                break found;
            }
            bin += 1;
        };

        let header = unsafe { &mut *current };
        let reusable_capacity = header.size as usize;
        // Reuse counts toward the GC trigger like any other allocation.
        // Otherwise each cycle recycles every freed block and then grows the
        // heap by another full threshold before collecting again.
        self.total_allocated += GC_HEADER_SIZE + reusable_capacity;

        // Clear the free bit, zero flags, link into all_objects list.
        header.flags = 0;
        header.next = self.all_objects;
        self.all_objects = current;

        // Wipe the whole reusable block so bytes outside a smaller
        // ordinary allocation cannot survive reuse.
        let data = header.data_ptr();
        unsafe {
            ptr::write_bytes(data, 0, reusable_capacity);
        }

        Some(data)
    }

    /// Bump-allocate `GC_HEADER_SIZE + size` bytes from pages and initialize
    /// the GcHeader.
    fn bump_alloc_with_header(&mut self, size: usize, align: usize) -> *mut u8 {
        // Headers sit on granule boundaries so the start bitmap can name them.
        let align = align.max(GRANULE);
        let total = GC_HEADER_SIZE + size;

        // We need the USER DATA pointer (header + GC_HEADER_SIZE) to satisfy
        // the requested alignment, so the header offset is derived from it.
        let in_current_page = self.pages.last().and_then(|page| {
            let header_offset = page.header_offset(self.offset, align);
            (header_offset + total <= page.len()).then_some(header_offset)
        });
        let header_offset = match in_current_page {
            Some(header_offset) => header_offset,
            None => {
                // Allocate a new page. If the total exceeds the default page
                // size, allocate a page large enough (with room for alignment
                // padding).
                let max_padding = if align > GC_HEADER_SIZE { align } else { 0 };
                self.push_page((total + max_padding).max(ACTOR_PAGE_SIZE))
                    .header_offset(0, align)
            }
        };

        let page = self.pages.last_mut().unwrap();
        page.mark_start(header_offset);
        let header_ptr = (page.base + header_offset) as *mut GcHeader;
        unsafe {
            header_ptr.write(GcHeader {
                size: size as u32,
                flags: 0,
                _pad: [0; 3],
                next: self.all_objects,
            });
        }
        self.all_objects = header_ptr;

        self.offset = header_offset + total;
        self.total_allocated += total;

        unsafe { (*header_ptr).data_ptr() }
    }

    fn push_page(&mut self, bytes: usize) -> &Page {
        let page = Page::new(bytes);
        let range = (page.base, page.base + page.len(), self.pages.len());
        let at = self
            .page_ranges
            .partition_point(|&(start, ..)| start < range.0);
        self.page_ranges.insert(at, range);
        self.pages.push(page);
        self.pages.last().unwrap()
    }

    /// Size of the live allocation that starts exactly at `data`, if there is one.
    pub(crate) fn live_allocation_size(&self, data: *const u8) -> Option<usize> {
        let header = self.find_object_containing(data)?;
        unsafe { ((*header).data_ptr() as *const u8 == data).then(|| (*header).size as usize) }
    }

    /// Whether `ptr` lies in one of this heap's pages.
    pub(crate) fn contains_address(&self, ptr: *const u8) -> bool {
        let addr = ptr as usize;
        let after = self
            .page_ranges
            .partition_point(|&(start, ..)| start <= addr);
        after > 0 && addr < self.page_ranges[after - 1].1
    }

    /// True when `data` is the start of a live allocation of at least `size` bytes.
    pub(crate) fn is_live_allocation(&self, data: *const u8, size: usize) -> bool {
        self.live_allocation_size(data)
            .is_some_and(|allocated| allocated >= size)
    }

    /// Keep every object that `words` point into alive until the returned
    /// token is dropped. Returns `None` when no word points into this heap, so
    /// plain integers and static data cost the borrower nothing.
    ///
    /// Words are matched conservatively, like stack words: an integer that
    /// happens to equal an address here only retains an object for longer.
    pub(crate) fn lend(&mut self, words: &[usize]) -> Option<Arc<()>> {
        let roots: Vec<usize> = words
            .iter()
            .copied()
            .filter(|&word| self.find_object_containing(word as *const u8).is_some())
            .collect();
        if roots.is_empty() {
            return None;
        }

        // A heap that never collects (the main thread's) still has to forget
        // finished borrowers, so prune whenever the list is about to grow.
        if self.lent.len() == self.lent.capacity() {
            self.lent.retain(Lent::is_live);
        }
        let token = Arc::new(());
        self.lent.push(Lent {
            roots,
            borrower: Arc::downgrade(&token),
        });
        Some(token)
    }

    /// Drop all pages and start fresh.
    ///
    /// Used for actor termination cleanup or after full GC sweep.
    pub fn reset(&mut self) {
        self.pages.clear();
        self.page_ranges.clear();
        self.offset = 0;
        self.total_allocated = 0;
        self.all_objects = ptr::null_mut();
        self.free_bins = None;
        self.lent.clear();
    }

    /// Returns the total number of bytes allocated from this heap
    /// (including GcHeader overhead).
    pub fn total_bytes(&self) -> usize {
        self.total_allocated
    }

    /// Returns true if the heap has exceeded its GC pressure threshold.
    pub fn should_collect(&self) -> bool {
        !self.gc_in_progress && self.total_allocated >= self.gc_threshold
    }

    /// Returns a pointer to the head of the all-objects list.
    pub fn all_objects_head(&self) -> *mut GcHeader {
        self.all_objects
    }

    /// Returns a free block available for reuse, or null if there is none.
    pub fn free_list_head(&self) -> *mut GcHeader {
        self.free_bins
            .as_deref()
            .map_or(ptr::null_mut(), FreeBins::any)
    }

    /// Returns whether GC is currently in progress.
    pub fn gc_in_progress(&self) -> bool {
        self.gc_in_progress
    }

    /// Set the GC-in-progress flag.
    pub fn set_gc_in_progress(&mut self, value: bool) {
        self.gc_in_progress = value;
    }

    /// Set the all-objects head pointer (used by sweep phase).
    pub fn set_all_objects_head(&mut self, head: *mut GcHeader) {
        self.all_objects = head;
    }

    /// Add a header to the free bins (used by sweep phase).
    pub fn add_to_free_list(&mut self, header: *mut GcHeader) {
        self.free_bins
            .get_or_insert_with(FreeBins::new)
            .push(header);
    }

    /// Returns the GC threshold in bytes.
    pub fn gc_threshold(&self) -> usize {
        self.gc_threshold
    }

    /// Set the GC threshold in bytes.
    pub fn set_gc_threshold(&mut self, threshold: usize) {
        self.gc_threshold = threshold;
    }

    /// Subtract from total_allocated (used after sweep frees objects).
    pub fn subtract_allocated(&mut self, bytes: usize) {
        self.total_allocated = self.total_allocated.saturating_sub(bytes);
    }

    // -----------------------------------------------------------------------
    // Mark-Sweep Garbage Collection
    // -----------------------------------------------------------------------

    /// Run a full mark-sweep garbage collection cycle.
    ///
    /// Conservatively scans the coroutine stack between `stack_bottom` and
    /// `stack_top` for roots, marks all transitively reachable objects, then
    /// sweeps unreachable objects onto the free list.
    ///
    /// `stack_top` has the lower address (stack grows downward on x86-64/ARM64).
    /// `stack_bottom` has the higher address (the base of the coroutine stack).
    ///
    /// This method is guarded against re-entrancy: if `gc_in_progress` is
    /// already set, the call is a no-op.
    pub fn collect(&mut self, stack_bottom: *const u8, stack_top: *const u8) {
        if self.gc_in_progress {
            return;
        }
        self.gc_in_progress = true;

        self.mark_from_roots(stack_bottom, stack_top);
        self.sweep();

        self.gc_in_progress = false;
    }

    /// Mark phase: conservatively scan the stack and trace all reachable objects.
    ///
    /// 1. Walk the stack from `stack_top` (low address) to `stack_bottom`
    ///    (high address), treating every 8-byte-aligned word as a potential
    ///    pointer. If it points into a live object in this heap, mark it as
    ///    a root.
    ///
    /// 2. Process a worklist (tricolor marking): for each marked object, scan
    ///    its body for further heap pointers and mark those transitively.
    ///
    /// The worklist is a `Vec` allocated on the system heap (via Rust's
    /// allocator), NOT on the GC heap, to avoid re-entrancy issues.
    fn mark_from_roots(&mut self, stack_bottom: *const u8, stack_top: *const u8) {
        // Worklist lives on the system heap (Rust Vec -> malloc).
        let mut worklist: Vec<*mut GcHeader> = Vec::new();

        // Ensure stack_top <= stack_bottom (stack_top is lower address).
        let (lo, hi) = if (stack_top as usize) <= (stack_bottom as usize) {
            (stack_top as usize, stack_bottom as usize)
        } else {
            (stack_bottom as usize, stack_top as usize)
        };

        // Phase 1: Conservative stack scanning.
        // Walk every 8-byte-aligned word in the stack range.
        let aligned_lo = (lo + 7) & !7; // round up to 8-byte alignment
        let mut addr = aligned_lo;
        while addr + 8 <= hi {
            let word = unsafe { *(addr as *const usize) };
            self.mark_word(word, &mut worklist);
            addr += 8;
        }

        // Objects lent to spawned actors are roots for as long as the
        // borrower lives, even though nothing in this actor mentions them.
        self.lent.retain(Lent::is_live);
        for lent in &self.lent {
            for &root in &lent.roots {
                self.mark_word(root, &mut worklist);
            }
        }

        // Phase 2: Worklist-based transitive marking (tricolor).
        while let Some(header) = worklist.pop() {
            let hdr = unsafe { &*header };
            let data_start = unsafe { (header as *mut u8).add(GC_HEADER_SIZE) };
            let body_size = hdr.size as usize;

            // Scan every 8-byte word in the object body.
            let mut offset = 0;
            while offset + 8 <= body_size {
                let word = unsafe { *(data_start.add(offset) as *const usize) };
                self.mark_word(word, &mut worklist);
                offset += 8;
            }
        }
    }

    /// Mark the live object `word` points into, if any, and queue it for tracing.
    fn mark_word(&self, word: usize, worklist: &mut Vec<*mut GcHeader>) {
        if let Some(header) = self.find_object_containing(word as *const u8) {
            let hdr = unsafe { &mut *header };
            if !hdr.is_marked() {
                hdr.set_marked();
                worklist.push(header);
            }
        }
    }

    /// Check if `ptr` points into a live (non-free) object in this heap.
    ///
    /// Finds the page holding `ptr`, then the nearest object start at or
    /// before it in that page's bitmap, and checks that the object's data
    /// range `[data_ptr, data_ptr + size)` contains `ptr`.
    ///
    /// This handles interior pointers: a pointer anywhere within an object's
    /// body identifies that object as reachable.
    ///
    /// Returns `Some(header_ptr)` if found, `None` otherwise.
    fn find_object_containing(&self, ptr: *const u8) -> Option<*mut GcHeader> {
        let addr = ptr as usize;

        // Quick check: most scanned words are not heap addresses at all.
        if addr < self.page_ranges.first()?.0 || addr >= self.page_ranges.last()?.1 {
            return None;
        }
        let after = self
            .page_ranges
            .partition_point(|&(start, ..)| start <= addr);
        let (start, end, index) = self.page_ranges[after - 1];
        if addr >= end {
            return None;
        }

        // An object's data begins GC_HEADER_SIZE past its header, and the next
        // header lies beyond its data, so the owner is the last header at or
        // before `addr - GC_HEADER_SIZE`.
        let offset = (addr - start).checked_sub(GC_HEADER_SIZE)?;
        let current = self.pages[index].header_at_or_before(offset)?;
        let header = unsafe { &*current };
        let data_end = current as usize + GC_HEADER_SIZE + header.size as usize;
        // Skip free objects -- a pointer to freed memory is not a root.
        (!header.is_free() && addr < data_end).then_some(current)
    }

    /// Sweep phase: walk the all-objects list and free unmarked objects.
    ///
    /// For each object in the all-objects list:
    /// - If marked: clear the mark bit, keep in the list.
    /// - If NOT marked: unlink from the list, set FREE_BIT, add to the free bins.
    ///
    /// Rebuilds the all-objects list in-place using a prev-pointer technique,
    /// then resets `total_allocated` to the surviving bytes and lets the next
    /// collection wait until the heap has grown by as much again. A fixed
    /// threshold would re-collect at every yield once the live set passed it.
    fn sweep(&mut self) {
        let mut current = self.all_objects;
        let mut prev: *mut GcHeader = ptr::null_mut();
        let mut new_head = self.all_objects;
        let mut first = true;
        let mut live_bytes = 0usize;

        while !current.is_null() {
            let header = unsafe { &mut *current };
            let next = header.next;

            if header.is_marked() {
                // Reachable: clear mark bit and keep in list.
                header.clear_marked();
                live_bytes += GC_HEADER_SIZE + header.size as usize;
                if first {
                    new_head = current;
                    first = false;
                }
                prev = current;
                current = next;
            } else {
                // Unreachable: unlink from all_objects and add to the free bins.
                if !prev.is_null() {
                    unsafe {
                        (*prev).next = next;
                    }
                } else {
                    // We're removing the head.
                    new_head = next;
                }

                header.set_free();
                self.add_to_free_list(current);

                current = next;
                // prev stays the same -- we removed current.
            }
        }

        self.all_objects = if first { ptr::null_mut() } else { new_head };
        self.total_allocated = live_bytes;
        self.gc_threshold = match min_gc_threshold() {
            0 => 0,
            floor => floor.max(live_bytes.saturating_mul(2)),
        };
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
            .field("all_objects", &(!self.all_objects.is_null()))
            .field("free_list", &(!self.free_list_head().is_null()))
            .field("gc_threshold", &self.gc_threshold)
            .field("gc_in_progress", &self.gc_in_progress)
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
    /// Heap values the message references, detached from the sender's heap and
    /// rebuilt in the receiver's; see `msg_shape`.
    pub(crate) captured: super::msg_shape::Captured,
    /// Loans for references that could not be copied. They travel with the
    /// message and become the receiver's.
    pub(crate) borrows: Vec<super::process::HeapBorrow>,
}

impl MessageBuffer {
    /// Create a new message buffer from raw bytes and a type tag.
    pub fn new(data: Vec<u8>, type_tag: u64) -> Self {
        MessageBuffer {
            data,
            type_tag,
            captured: Default::default(),
            borrows: Vec::new(),
        }
    }

    /// Note who will receive this message. A loan from the receiver's own heap
    /// (a message to itself) drops its pin, or the process would own itself
    /// and never be freed.
    pub(crate) fn addressed_to(&mut self, receiver: &Arc<parking_lot::Mutex<super::Process>>) {
        for loan in &mut self.borrows {
            if loan
                .owner
                .as_ref()
                .is_some_and(|owner| Arc::ptr_eq(owner, receiver))
            {
                loan.owner = None;
            }
        }
    }

    /// Deep-copy this message's data into the target actor's heap.
    ///
    /// Allocates space in the target heap (with GcHeader prepended
    /// automatically), copies the data bytes, and returns a pointer
    /// to the copy within the target heap.
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
    fn test_gc_header_layout() {
        // GcHeader must be exactly 16 bytes.
        assert_eq!(
            std::mem::size_of::<GcHeader>(),
            GC_HEADER_SIZE,
            "GcHeader must be exactly 16 bytes"
        );

        // Verify data_ptr / from_data_ptr round-trip.
        let mut heap = ActorHeap::new();
        let data_ptr = heap.alloc(64, 8);
        assert!(!data_ptr.is_null());

        let header_ptr = unsafe { GcHeader::from_data_ptr(data_ptr) };
        assert!(!header_ptr.is_null());

        let recovered_data = unsafe { (*header_ptr).data_ptr() };
        assert_eq!(
            data_ptr, recovered_data,
            "data_ptr/from_data_ptr round-trip"
        );

        // Verify header fields.
        let header = unsafe { &*header_ptr };
        assert_eq!(header.size, 64);
        assert_eq!(header.flags, 0);
        assert!(!header.is_marked());
        assert!(!header.is_free());
    }

    #[test]
    fn test_gc_header_flags() {
        let mut header = GcHeader {
            size: 100,
            flags: 0,
            _pad: [0; 3],
            next: ptr::null_mut(),
        };

        // Mark bit.
        assert!(!header.is_marked());
        header.set_marked();
        assert!(header.is_marked());
        assert!(!header.is_free());
        header.clear_marked();
        assert!(!header.is_marked());

        // Free bit.
        assert!(!header.is_free());
        header.set_free();
        assert!(header.is_free());
        assert!(!header.is_marked());
        header.clear_free();
        assert!(!header.is_free());

        // Both bits.
        header.set_marked();
        header.set_free();
        assert!(header.is_marked());
        assert!(header.is_free());
        assert_eq!(header.flags, MARK_BIT | FREE_BIT);
    }

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
        heap.alloc(8, 8);
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

        // Reset should clear everything including GC lists.
        heap.reset();
        assert_eq!(heap.total_bytes(), 0);
        assert!(heap.pages.is_empty());
        assert_eq!(heap.offset, 0);
        assert!(heap.all_objects.is_null());
        assert!(heap.free_list_head().is_null());
    }

    #[test]
    fn test_actor_heap_total_bytes() {
        let mut heap = ActorHeap::new();
        assert_eq!(heap.total_bytes(), 0);

        // Each alloc adds GC_HEADER_SIZE + requested size.
        heap.alloc(100, 8);
        assert_eq!(heap.total_bytes(), GC_HEADER_SIZE + 100);

        heap.alloc(200, 8);
        assert_eq!(heap.total_bytes(), 2 * GC_HEADER_SIZE + 300);
    }

    #[test]
    fn test_all_objects_list() {
        let mut heap = ActorHeap::new();

        // Allocate 3 objects.
        let _p1 = heap.alloc(32, 8);
        let _p2 = heap.alloc(64, 8);
        let _p3 = heap.alloc(16, 8);

        // Walk the all_objects list and count entries.
        let mut count = 0;
        let mut current = heap.all_objects_head();
        while !current.is_null() {
            count += 1;
            let header = unsafe { &*current };
            assert!(!header.is_free());
            assert!(!header.is_marked());
            current = header.next;
        }
        assert_eq!(count, 3, "all_objects list should contain 3 objects");
    }

    #[test]
    fn test_free_list_reuse() {
        let mut heap = ActorHeap::new();

        // Allocate an object.
        let ptr1 = heap.alloc(64, 8);
        assert!(!ptr1.is_null());
        let header1 = unsafe { GcHeader::from_data_ptr(ptr1) };

        // Record total_allocated after first alloc.
        let allocated_after_first = heap.total_bytes();

        // Manually free it: unlink from all_objects, set FREE, add to free list.
        // (In normal GC, sweep does this; here we simulate.)
        let next_in_all = unsafe { (*header1).next };
        heap.set_all_objects_head(next_in_all);
        unsafe {
            (*header1).set_free();
        }
        heap.add_to_free_list(header1);
        heap.subtract_allocated(GC_HEADER_SIZE + 64);

        // Allocate the same size -- should reuse from free list.
        let ptr2 = heap.alloc(64, 8);
        assert!(!ptr2.is_null());

        // The reused block should be the same memory region.
        assert_eq!(ptr1, ptr2, "free-list reuse should return the same pointer");

        // Reuse is accounted exactly like the original allocation, so the
        // GC trigger sees recycled blocks too.
        assert_eq!(heap.total_bytes(), allocated_after_first);

        // The header should no longer be free.
        let header2 = unsafe { &*GcHeader::from_data_ptr(ptr2) };
        assert!(!header2.is_free());
        assert_eq!(header2.size, 64);
    }

    #[test]
    fn test_free_list_larger_block() {
        let mut heap = ActorHeap::new();

        // Allocate a large block and free it.
        let ptr_big = heap.alloc(256, 8);
        unsafe { ptr::write_bytes(ptr_big, 0xA5, 256) };
        let header_big = unsafe { GcHeader::from_data_ptr(ptr_big) };

        // Unlink from all_objects, add to free list.
        let next = unsafe { (*header_big).next };
        heap.set_all_objects_head(next);
        unsafe {
            (*header_big).set_free();
        }
        heap.add_to_free_list(header_big);

        // Allocate a smaller block -- should reuse the larger freed block.
        let ptr_small = heap.alloc(64, 8);
        assert_eq!(
            ptr_big, ptr_small,
            "should reuse larger free block for smaller request"
        );

        let header = unsafe { &*GcHeader::from_data_ptr(ptr_small) };
        // Normal first-fit reuse preserves the physical block capacity.
        assert_eq!(header.size, 256);
        // Reuse wipes the entire physical capacity, including bytes beyond
        // the requested object body.
        let reused_capacity = unsafe { std::slice::from_raw_parts(ptr_small, 256) };
        assert!(reused_capacity.iter().all(|byte| *byte == 0));
    }

    #[test]
    fn test_exact_allocation_skips_larger_free_block() {
        let mut heap = ActorHeap::new();
        let ptr_big = heap.alloc(256, 8);
        let header_big = unsafe { GcHeader::from_data_ptr(ptr_big) };
        let next = unsafe { (*header_big).next };
        heap.set_all_objects_head(next);
        unsafe { (*header_big).set_free() };
        heap.add_to_free_list(header_big);

        let ptr_exact = heap.alloc_exact(64, 8);

        assert_ne!(ptr_exact, ptr_big);
        let header = unsafe { &*GcHeader::from_data_ptr(ptr_exact) };
        assert_eq!(header.size, 64);
        assert_eq!(heap.free_list_head(), header_big);
    }

    #[test]
    fn test_free_list_reuse_respects_requested_alignment() {
        let mut heap = ActorHeap::new();
        let mut candidate = heap.alloc(64, 8);
        if candidate as usize % 64 == 0 {
            candidate = heap.alloc(64, 8);
        }
        assert_ne!(candidate as usize % 64, 0);
        let candidate_header = unsafe { GcHeader::from_data_ptr(candidate) };
        let next = unsafe { (*candidate_header).next };
        heap.set_all_objects_head(next);
        unsafe { (*candidate_header).set_free() };
        heap.add_to_free_list(candidate_header);

        let aligned = heap.alloc(32, 64);

        assert_ne!(aligned, candidate);
        assert_eq!(aligned as usize % 64, 0);
        assert_eq!(heap.free_list_head(), candidate_header);
    }

    #[test]
    fn test_should_collect() {
        let mut heap = ActorHeap::new();
        heap.set_gc_threshold(100);

        assert!(!heap.should_collect());

        // Allocate enough to exceed the threshold.
        // Each alloc adds GC_HEADER_SIZE + size.
        heap.alloc(50, 8); // 66 bytes
        assert!(!heap.should_collect());

        heap.alloc(50, 8); // 66 more = 132 total, exceeds 100
        assert!(heap.should_collect());

        // When GC is in progress, should_collect returns false.
        heap.set_gc_in_progress(true);
        assert!(!heap.should_collect());
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

        // Verify the GcHeader is present.
        let header = unsafe { &*GcHeader::from_data_ptr(ptr) };
        assert_eq!(header.size as usize, data.len());
        assert!(!header.is_free());
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

    // -----------------------------------------------------------------------
    // Mark-Sweep GC Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_collect_frees_unreachable() {
        // Allocate 5 objects, don't reference them from the stack.
        // Collect with an empty stack range -- all should be freed.
        let mut heap = ActorHeap::new();
        let _p1 = heap.alloc(32, 8);
        let _p2 = heap.alloc(64, 8);
        let _p3 = heap.alloc(16, 8);
        let _p4 = heap.alloc(48, 8);
        let _p5 = heap.alloc(24, 8);

        assert!(heap.total_bytes() > 0);

        // Use an empty stack range (both pointers equal) so no roots are found.
        let dummy: u64 = 0;
        let stack_ptr = &dummy as *const u64 as *const u8;
        heap.collect(stack_ptr, stack_ptr);

        // All objects should have been swept to the free list.
        assert!(
            heap.all_objects_head().is_null(),
            "all_objects should be empty after collecting unreachable objects"
        );
        assert!(
            !heap.free_list_head().is_null(),
            "free_list should be non-empty after sweep"
        );
        assert_eq!(
            heap.total_bytes(),
            0,
            "total_allocated should be 0 after collecting all unreachable objects"
        );
    }

    #[test]
    fn test_collect_retains_reachable() {
        // Allocate an object, create a fake stack frame containing its pointer,
        // then collect. The object should NOT be freed.
        let mut heap = ActorHeap::new();
        let ptr = heap.alloc(64, 8);
        let original_total = heap.total_bytes();

        // Create a fake stack frame: an array containing the pointer value.
        // The GC will scan this as the stack and find the pointer.
        let fake_stack: [usize; 4] = [0, ptr as usize, 0, 0];
        let stack_bottom = unsafe {
            (&fake_stack[0] as *const usize as *const u8).add(std::mem::size_of_val(&fake_stack))
        };
        let stack_top = &fake_stack[0] as *const usize as *const u8;

        heap.collect(stack_bottom, stack_top);

        // The object should be retained (reachable from the fake stack).
        assert!(
            !heap.all_objects_head().is_null(),
            "reachable object should survive GC"
        );
        assert_eq!(
            heap.total_bytes(),
            original_total,
            "total_allocated should be unchanged for reachable objects"
        );

        // The mark bit should be cleared after sweep.
        let header = unsafe { &*GcHeader::from_data_ptr(ptr) };
        assert!(
            !header.is_marked(),
            "mark bit should be cleared after sweep"
        );
    }

    #[test]
    fn test_collect_reduces_total_bytes() {
        // Allocate 10 objects, collect with empty roots. Total bytes should drop to 0.
        let mut heap = ActorHeap::new();
        for _ in 0..10 {
            heap.alloc(100, 8);
        }

        let before = heap.total_bytes();
        assert!(before > 0);

        let dummy: u64 = 0;
        let stack_ptr = &dummy as *const u64 as *const u8;
        heap.collect(stack_ptr, stack_ptr);

        assert_eq!(heap.total_bytes(), 0);
        assert!(
            heap.total_bytes() < before,
            "total_bytes should decrease after collection"
        );
    }

    #[test]
    fn test_gc_in_progress_guard() {
        // Verify gc_in_progress prevents re-entrant collection.
        let mut heap = ActorHeap::new();
        let _p = heap.alloc(64, 8);
        let before = heap.total_bytes();

        // Manually set gc_in_progress to true.
        heap.set_gc_in_progress(true);

        // Attempt collect -- should be a no-op due to re-entrancy guard.
        let dummy: u64 = 0;
        let stack_ptr = &dummy as *const u64 as *const u8;
        heap.collect(stack_ptr, stack_ptr);

        // Nothing should have changed.
        assert_eq!(
            heap.total_bytes(),
            before,
            "collect should be no-op when gc_in_progress is true"
        );
        assert!(
            !heap.all_objects_head().is_null(),
            "all_objects should be unchanged when gc_in_progress"
        );

        // gc_in_progress should still be true (collect was a no-op).
        assert!(heap.gc_in_progress());
    }

    #[test]
    fn test_collect_transitive_reachability() {
        // Object A (on fake stack) points to Object B. Both should survive.
        let mut heap = ActorHeap::new();

        // Allocate B first, then A. A's body will contain a pointer to B.
        let ptr_b = heap.alloc(64, 8);
        let ptr_a = heap.alloc(64, 8);

        // Write ptr_b into A's body so the mark phase traces A -> B.
        unsafe {
            *(ptr_a as *mut usize) = ptr_b as usize;
        }

        // Allocate a third object C that is NOT referenced.
        let _ptr_c = heap.alloc(64, 8);

        // Fake stack contains only ptr_a.
        let fake_stack: [usize; 4] = [0, ptr_a as usize, 0, 0];
        let stack_top = &fake_stack[0] as *const usize as *const u8;
        let stack_bottom = unsafe { stack_top.add(std::mem::size_of_val(&fake_stack)) };

        heap.collect(stack_bottom, stack_top);

        // A and B should survive, C should be freed.
        // Count surviving objects.
        let mut count = 0;
        let mut current = heap.all_objects_head();
        while !current.is_null() {
            count += 1;
            current = unsafe { (*current).next };
        }
        assert_eq!(
            count, 2,
            "A and B should survive (transitive reachability), C should be freed"
        );

        // total_allocated should reflect only A and B.
        assert_eq!(
            heap.total_bytes(),
            2 * (GC_HEADER_SIZE + 64),
            "total_bytes should reflect only the two surviving objects"
        );
    }

    #[test]
    fn test_find_object_containing_interior_pointer() {
        // A pointer into the middle of an object's body should identify that object.
        let mut heap = ActorHeap::new();
        let ptr = heap.alloc(128, 8);

        // Interior pointer: 64 bytes into the object.
        let interior = unsafe { ptr.add(64) };
        let found = heap.find_object_containing(interior);
        assert!(
            found.is_some(),
            "interior pointer should find the containing object"
        );

        let header = found.unwrap();
        let data_start = unsafe { (*header).data_ptr() };
        assert_eq!(
            data_start, ptr,
            "found object should be the one containing the interior pointer"
        );
    }

    #[test]
    fn test_find_object_containing_out_of_range() {
        let heap = ActorHeap::new();

        // Pointer outside any page should return None.
        let random_ptr = 0xDEADBEEF_usize as *const u8;
        assert!(heap.find_object_containing(random_ptr).is_none());
    }

    /// Free a live allocation the way sweep does, for bin tests.
    fn free_block(heap: &mut ActorHeap, data: *mut u8) {
        let header = unsafe { GcHeader::from_data_ptr(data) };
        assert_eq!(heap.all_objects_head(), header, "free the newest object");
        heap.set_all_objects_head(unsafe { (*header).next });
        unsafe { (*header).set_free() };
        heap.add_to_free_list(header);
    }

    #[test]
    fn test_bin_boundaries() {
        assert_eq!(bin_for(0), 0);
        assert_eq!(bin_for(SMALL_BINS - 1), SMALL_BINS - 1);
        assert_eq!(bin_for(SMALL_BINS), SMALL_BINS);
        assert_eq!(bin_for(2 * SMALL_BINS - 1), SMALL_BINS);
        assert_eq!(bin_for(2 * SMALL_BINS), SMALL_BINS + 1);
        assert_eq!(bin_for(u32::MAX as usize), NUM_BINS - 1);
    }

    #[test]
    fn test_free_bins_reuse_smallest_fitting_block() {
        let mut heap = ActorHeap::new();
        let blocks: Vec<*mut u8> = [24, 40, 300, 5000]
            .iter()
            .map(|&size| heap.alloc(size, 8))
            .collect();
        for &block in blocks.iter().rev() {
            free_block(&mut heap, block);
        }

        // Skips the 24-byte block without walking it; 40 is the tightest fit.
        assert_eq!(heap.alloc(32, 8), blocks[1]);
        // Crosses from the exact bins into the power-of-two bins.
        assert_eq!(heap.alloc(2000, 8), blocks[3]);
        assert_eq!(heap.alloc_exact(24, 8), blocks[0]);
        // A block far larger than the request is left for a closer match.
        assert_ne!(heap.alloc(8, 8), blocks[2]);
        assert_eq!(heap.alloc(300 / MAX_REUSE_WASTE_FACTOR, 8), blocks[2]);
        assert!(heap.free_list_head().is_null());
    }

    #[test]
    fn test_same_bin_block_that_is_too_small_is_not_reused() {
        let mut heap = ActorHeap::new();
        let small = heap.alloc(300, 8);
        free_block(&mut heap, small);

        // 300 and 400 share the 256..512 bin; the block must still fit.
        assert_ne!(heap.alloc(400, 8), small);
        assert_eq!(heap.free_list_head(), unsafe {
            GcHeader::from_data_ptr(small)
        });
    }

    #[test]
    fn test_heap_owns_no_memory_until_first_alloc() {
        let mut heap = ActorHeap::new();
        assert!(heap.pages.is_empty());
        assert!(heap.find_object_containing(8 as *const u8).is_none());
        heap.alloc(8, 8);
        assert_eq!(heap.pages.len(), 1);
    }

    #[test]
    fn test_find_object_containing_resolves_every_object() {
        // Mixed sizes and alignments across several pages, including a block
        // larger than a page and bitmap word boundaries.
        let mut heap = ActorHeap::new();
        let sizes = [1, 8, 24, 100, 4096, 70_000, 16, 513];
        let objects: Vec<(*mut u8, usize)> = (0..400)
            .map(|i| {
                let size = sizes[i % sizes.len()];
                (heap.alloc(size, if i % 7 == 0 { 64 } else { 8 }), size)
            })
            .collect();
        assert!(heap.pages.len() > 2);

        for &(data, size) in &objects {
            let header = unsafe { GcHeader::from_data_ptr(data) };
            assert_eq!(heap.find_object_containing(data), Some(header));
            let last = unsafe { data.add(size - 1) };
            assert_eq!(heap.find_object_containing(last), Some(header));
            // One past the end and the header itself belong to no object.
            assert_ne!(
                heap.find_object_containing(unsafe { data.add(size) }),
                Some(header)
            );
            assert!(heap.find_object_containing(header as *const u8).is_none());
        }
    }

    #[test]
    fn test_garbage_churn_does_not_grow_the_heap() {
        // Nothing is live, so a steady stream of garbage must keep recycling
        // the same pages instead of growing by a threshold every cycle.
        let mut heap = ActorHeap::new();
        let dummy: u64 = 0;
        let stack = &dummy as *const u64 as *const u8;
        for _ in 0..200 {
            while !heap.should_collect() {
                heap.alloc(32, 8);
            }
            heap.collect(stack, stack);
        }
        let pages_for_one_cycle = DEFAULT_GC_THRESHOLD.div_ceil(ACTOR_PAGE_SIZE) + 1;
        assert!(
            heap.pages.len() <= pages_for_one_cycle,
            "heap grew to {} pages",
            heap.pages.len()
        );
    }

    #[test]
    fn test_lent_objects_survive_until_the_borrower_is_gone() {
        // Nothing in this heap's own roots mentions `outer`, which points at `inner`.
        let mut heap = ActorHeap::new();
        let inner = heap.alloc(32, 8);
        let outer = heap.alloc(32, 8);
        unsafe { *(outer as *mut usize) = inner as usize };
        let _garbage = heap.alloc(32, 8);

        let dummy: u64 = 0;
        let stack = &dummy as *const u64 as *const u8;
        let interior = outer as usize + 8;
        assert!(
            heap.lend(&[7, 0]).is_none(),
            "plain integers borrow nothing"
        );
        let loan = heap.lend(&[7, interior]).expect("word points into heap");

        heap.collect(stack, stack);
        assert!(heap.is_live_allocation(outer, 32));
        assert!(
            heap.is_live_allocation(inner, 32),
            "traced through the loan"
        );
        assert_eq!(heap.total_bytes(), 2 * (GC_HEADER_SIZE + 32));

        drop(loan);
        heap.collect(stack, stack);
        assert!(heap.all_objects_head().is_null());
        assert!(heap.lent.is_empty());
    }

    #[test]
    fn test_heap_that_never_collects_forgets_finished_borrowers() {
        let mut heap = ActorHeap::new();
        let value = heap.alloc(16, 8) as usize;
        for _ in 0..10_000 {
            drop(heap.lend(&[value]));
        }
        assert!(heap.lent.len() <= 8, "{} stale loans kept", heap.lent.len());
    }

    #[test]
    fn test_threshold_grows_with_live_set() {
        let mut heap = ActorHeap::new();
        let live = heap.alloc(2 * DEFAULT_GC_THRESHOLD, 8);
        assert!(heap.should_collect());

        let fake_stack: [usize; 1] = [live as usize];
        let stack_top = fake_stack.as_ptr() as *const u8;
        let stack_bottom = unsafe { stack_top.add(std::mem::size_of_val(&fake_stack)) };
        heap.collect(stack_bottom, stack_top);

        // A live set above the default threshold must not re-collect at once.
        assert_eq!(heap.gc_threshold(), 2 * heap.total_bytes());
        assert!(!heap.should_collect());
    }

    #[test]
    fn test_collect_then_reuse() {
        // After collection, freed objects should be reusable via the free list.
        let mut heap = ActorHeap::new();
        let _p1 = heap.alloc(64, 8);
        let _p2 = heap.alloc(64, 8);

        // Collect with empty roots to free everything.
        let dummy: u64 = 0;
        let stack_ptr = &dummy as *const u64 as *const u8;
        heap.collect(stack_ptr, stack_ptr);

        assert_eq!(heap.total_bytes(), 0);
        assert!(!heap.free_list_head().is_null());

        // Allocate again -- should reuse from free list.
        let p3 = heap.alloc(64, 8);
        assert!(!p3.is_null());
        // Should have come from the free list, and counts toward the trigger.
        assert_eq!(heap.total_bytes(), GC_HEADER_SIZE + 64);
        assert!(!heap.all_objects_head().is_null());
    }
}
