# Phase 17: Mark-Sweep Garbage Collector - Research

**Researched:** 2026-02-07
**Domain:** Runtime memory management -- mark-sweep GC replacing arena/bump allocation
**Confidence:** HIGH

## Summary

This phase replaces Snow's current arena/bump allocator (which never reclaims memory) with a mark-sweep garbage collector for per-actor heaps. The research focused on understanding the existing codebase architecture, identifying what objects are GC-managed vs stack-allocated, determining root enumeration strategy, and designing the object header layout and free-list allocator.

The key finding is that Snow's architecture is well-suited for per-actor GC: actors are already isolated with per-actor heaps (`ActorHeap`), the allocation ABI (`snow_gc_alloc` / `snow_gc_alloc_actor`) provides a clean interception point, and the coroutine-based scheduler gives natural GC safepoints at yield boundaries. The main challenge is root enumeration -- the current system has no object headers and no mechanism for tracing references from stack to heap.

Snow's value representation is uniformly 8 bytes (i64 for ints/bools/pids, pointers for strings/lists/maps/closures/tuples). This uniformity means the GC must either use **tagged pointers** to distinguish heap pointers from immediates, or use a **conservative approach** scanning for values that look like heap pointers. The tagged pointer approach is cleaner and avoids false retention, but requires changes to the codegen. The conservative approach is simpler to implement but may retain dead objects.

**Primary recommendation:** Implement a precise mark-sweep GC with object headers, a free-list allocator, and conservative stack scanning for root enumeration. Each GC-managed object gets a header (mark bit, size, type tag, next pointer). Trigger GC when an actor's heap exceeds a configurable pressure threshold. GC runs only when the actor yields (at `snow_reduction_check` points), ensuring other actors are unaffected.

## Standard Stack

This is a custom GC implementation within the existing Snow runtime (no external library dependencies). All work is done in the `snow-rt` crate.

### Core (existing, modified)
| Module | Location | Purpose | Changes Needed |
|--------|----------|---------|----------------|
| `gc.rs` | `crates/snow-rt/src/gc.rs` | Global GC allocation entry points | Replace arena with GC-aware allocator; add `snow_gc_collect` entry point |
| `actor/heap.rs` | `crates/snow-rt/src/actor/heap.rs` | Per-actor heap (currently bump allocator) | Replace with free-list allocator + object headers + mark-sweep logic |
| `actor/process.rs` | `crates/snow-rt/src/actor/process.rs` | Process Control Block | Add GC trigger threshold field, GC statistics |
| `actor/mod.rs` | `crates/snow-rt/src/actor/mod.rs` | Actor runtime ABI | Add GC trigger in `snow_reduction_check` or new safepoint |
| `actor/scheduler.rs` | `crates/snow-rt/src/actor/scheduler.rs` | M:N work-stealing scheduler | No changes needed -- GC is per-actor, scheduler is unaware |

### New Modules
| Module | Purpose |
|--------|---------|
| (none -- keep GC logic in existing `heap.rs` and `gc.rs`) | All GC logic stays in the actor heap module to keep the runtime cohesive |

### No External Dependencies Needed
The GC is a custom implementation. No new crate dependencies are required. The existing `parking_lot` mutex is sufficient for per-actor heap locking.

## Architecture Patterns

### Current Memory Layout (What We're Replacing)

```
Current: Arena/Bump Allocator
┌──────────────────────────────────────┐
│ Page 1 (64 KiB)                     │
│ [obj1][obj2][obj3][....free space...]│
│                    ^offset           │
├──────────────────────────────────────┤
│ Page 2 (64 KiB)                     │
│ [obj4][obj5][...free space........] │
└──────────────────────────────────────┘
- No object headers
- No tracking of individual allocations
- No way to free individual objects
- Memory only reclaimed when actor terminates (heap.reset())
```

### Target Memory Layout (Mark-Sweep with Object Headers)

```
New: Mark-Sweep with Object Headers + Free List
┌──────────────────────────────────────┐
│ Page 1 (64 KiB)                     │
│ [hdr|obj1][hdr|obj2][hdr|FREE][hdr|obj3]│
│  ^next chains through all objects    │
├──────────────────────────────────────┤
│ Page 2 (64 KiB)                     │
│ [hdr|obj4][hdr|FREE][hdr|obj5]      │
└──────────────────────────────────────┘

Object Header (16 bytes):
┌─────────┬──────────┬────────┬──────────┐
│ size:u32│ flags:u8 │ pad:u8 │ next:*   │  (16 bytes total)
│         │ marked   │        │ (8 bytes)│
│         │ free     │        │          │
└─────────┬──────────┴────────┴──────────┘
           │
           ├── bit 0: marked (for GC mark phase)
           ├── bit 1: free (on free list)
           └── bits 2-7: reserved
```

### Pattern 1: Object Header Prepended to Every Allocation

**What:** Every `snow_gc_alloc` call returns a pointer PAST the header. The header sits immediately before the user-visible pointer.
**When to use:** All GC-managed allocations (strings, lists, maps, tuples, closure environments, sets, queues, ranges).

```rust
// Source: Custom design for Snow runtime

/// Object header prepended to every GC-managed allocation.
/// The user-visible pointer starts immediately after this header.
#[repr(C)]
struct GcHeader {
    /// Total size of the allocation (NOT including the header).
    size: u32,
    /// Flags: bit 0 = marked, bit 1 = free
    flags: u8,
    /// Reserved padding for alignment
    _pad: [u8; 3],
    /// Intrusive linked list: points to next GcHeader in the all-objects list.
    next: *mut GcHeader,
}

const GC_HEADER_SIZE: usize = 16; // size(4) + flags(1) + pad(3) + next(8)
const MARK_BIT: u8 = 0x01;
const FREE_BIT: u8 = 0x02;

impl GcHeader {
    fn is_marked(&self) -> bool { self.flags & MARK_BIT != 0 }
    fn set_marked(&mut self) { self.flags |= MARK_BIT; }
    fn clear_marked(&mut self) { self.flags &= !MARK_BIT; }
    fn is_free(&self) -> bool { self.flags & FREE_BIT != 0 }
    fn set_free(&mut self) { self.flags |= FREE_BIT; }
    fn clear_free(&mut self) { self.flags &= !FREE_BIT; }

    /// Get pointer to the user data (past the header).
    fn data_ptr(&mut self) -> *mut u8 {
        unsafe { (self as *mut GcHeader as *mut u8).add(GC_HEADER_SIZE) }
    }

    /// Get header pointer from a user data pointer.
    unsafe fn from_data_ptr(data: *mut u8) -> *mut GcHeader {
        data.sub(GC_HEADER_SIZE) as *mut GcHeader
    }
}
```

### Pattern 2: Conservative Stack Scanning for Root Enumeration

**What:** Instead of precisely tracking which stack slots contain GC pointers, scan the coroutine stack (64 KiB) for values that look like they point into the actor's heap pages.
**When to use:** During the mark phase, before tracing the heap object graph.
**Why conservative:** Snow compiles to native code via LLVM. LLVM manages register allocation and stack layout. We don't have precise knowledge of which stack slots contain GC pointers vs integers. Conservative scanning treats any stack word that looks like a valid heap pointer as a root.

```rust
// Source: Custom design for Snow runtime

/// Conservatively scan the coroutine stack for potential GC roots.
/// Any value on the stack that points into one of the actor's heap pages
/// is treated as a live root.
fn conservative_scan_roots(
    stack_bottom: *const u8,
    stack_top: *const u8,
    heap: &ActorHeap,
) -> Vec<*mut GcHeader> {
    let mut roots = Vec::new();
    let mut ptr = stack_bottom as *const usize;
    let end = stack_top as *const usize;

    while ptr < end {
        let val = unsafe { *ptr };
        // Check if this value looks like a pointer into any heap page.
        if let Some(header) = heap.find_object_containing(val as *const u8) {
            if !header.is_free() {
                roots.push(header);
            }
        }
        ptr = unsafe { ptr.add(1) };
    }
    roots
}
```

### Pattern 3: GC Trigger at Yield Points

**What:** Check heap pressure at `snow_reduction_check` (already called at loop back-edges and function calls). If the actor's heap exceeds the pressure threshold, trigger collection.
**When to use:** Every time reductions are checked (already a safepoint).

```rust
// Source: Custom design for Snow runtime

// In snow_reduction_check():
fn maybe_trigger_gc(pid: ProcessId) {
    let sched = global_scheduler();
    if let Some(proc_arc) = sched.get_process(pid) {
        let mut proc = proc_arc.lock();
        if proc.heap.should_collect() {
            proc.heap.collect(); // Mark-sweep within this actor only
        }
    }
}
```

### Pattern 4: Mark Phase -- Tricolor Marking

**What:** Starting from roots, traverse the object graph marking all reachable objects.
**When to use:** During GC collection.

```rust
// Source: Based on Crafting Interpreters mark-sweep pattern

fn mark_phase(roots: &[*mut GcHeader], heap: &ActorHeap) {
    // Use a worklist (gray stack) to avoid deep recursion.
    let mut worklist: Vec<*mut GcHeader> = roots.to_vec();

    while let Some(header) = worklist.pop() {
        let header = unsafe { &mut *header };
        if header.is_marked() {
            continue; // Already visited
        }
        header.set_marked();

        // Scan the object's data for pointers to other heap objects.
        // Since Snow values are uniformly 8 bytes, scan every 8-byte word
        // in the object body for potential heap pointers.
        let data = header.data_ptr();
        let words = header.size as usize / 8;
        for i in 0..words {
            let val = unsafe { *(data as *const u64).add(i) };
            if let Some(child_header) = heap.find_object_containing(val as *const u8) {
                if !child_header.is_marked() && !child_header.is_free() {
                    worklist.push(child_header);
                }
            }
        }
    }
}
```

### Pattern 5: Sweep Phase -- Build Free List

**What:** Walk the all-objects list, free unmarked objects, clear mark bits on survivors.
**When to use:** After mark phase.

```rust
// Source: Based on Crafting Interpreters mark-sweep pattern

fn sweep_phase(heap: &mut ActorHeap) {
    let mut obj = heap.all_objects_head;
    let mut prev: *mut GcHeader = std::ptr::null_mut();

    while !obj.is_null() {
        let header = unsafe { &mut *obj };
        let next = header.next;

        if header.is_marked() {
            // Object is live -- clear mark for next cycle.
            header.clear_marked();
            prev = obj;
            obj = next;
        } else {
            // Object is unreachable -- add to free list.
            header.set_free();

            // Add to free list (size-segregated or single list).
            heap.add_to_free_list(obj);

            // Update linked list: unlink from all-objects.
            if !prev.is_null() {
                unsafe { (*prev).next = next; }
            } else {
                heap.all_objects_head = next;
            }

            heap.bytes_freed += GC_HEADER_SIZE + header.size as usize;
            obj = next;
        }
    }
}
```

### Pattern 6: Free-List Allocation (Replacing Bump Allocation)

**What:** When allocating, first check the free list for a block of sufficient size. Fall back to bump allocation from the current page if no suitable free block exists.
**When to use:** Every `snow_gc_alloc` / `snow_gc_alloc_actor` call.

```rust
// Source: Custom design for Snow runtime

fn alloc_from_free_list(
    free_list: &mut *mut GcHeader,
    size: usize,
) -> Option<*mut u8> {
    let mut current = *free_list;
    let mut prev: *mut GcHeader = std::ptr::null_mut();

    while !current.is_null() {
        let header = unsafe { &mut *current };
        if header.size as usize >= size {
            // Found a suitable block -- remove from free list.
            let next = header.next;
            if !prev.is_null() {
                unsafe { (*prev).next = next; }
            } else {
                *free_list = next;
            }
            header.clear_free();
            // Re-link into all-objects list (it stays linked).
            return Some(header.data_ptr());
        }
        prev = current;
        current = header.next; // Walk free list
    }
    None
}
```

### Anti-Patterns to Avoid

- **Global stop-the-world GC:** GC must be per-actor only. Never pause other actors. The actor's coroutine is already not running while GC happens (it runs during the same timeslice).
- **Precise stack maps from LLVM:** Generating precise GC stack maps from LLVM is extremely complex and fragile. Use conservative stack scanning instead.
- **Moving/copying collector:** Moving objects invalidates pointers. The existing codegen stores raw pointers to GC objects. A non-moving mark-sweep is the right choice.
- **Reference counting:** Would require deep changes to codegen to emit inc/dec ref operations at every assignment. Mark-sweep integrates better with the existing allocation-only ABI.
- **Separate free list from all-objects list:** Keep the all-objects list separate from the free list. The all-objects list is for traversing during sweep; the free list is for allocation reuse. Objects on the free list should still be on the all-objects list (with the free flag set), or removed entirely and their space tracked separately.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Per-actor heap isolation | Global GC with actor awareness | Per-actor `ActorHeap` with its own mark-sweep | Already have per-actor heaps; just need to add collection |
| Precise root enumeration | LLVM GC stack maps | Conservative stack scanning | LLVM GC support is complex, poorly documented, and fragile; conservative scanning is proven and sufficient |
| Concurrent collection | Background GC threads | Run GC inline during actor's timeslice | Per-actor isolation means no other actors are affected; simplicity wins |
| Generational GC | Young/old generation split | Single-generation mark-sweep | Explicitly out of scope (REQUIREMENTS.md); generational is future work |

**Key insight:** The per-actor isolation model means the GC implementation can be remarkably simple. Each actor's GC is equivalent to a single-threaded, non-concurrent, non-generational mark-sweep collector -- the simplest possible tracing GC. The complexity normally associated with GC (concurrent access, global pauses, generational barriers) is eliminated by the actor model.

## Common Pitfalls

### Pitfall 1: Forgetting the Global Arena Fallback

**What goes wrong:** The main thread and non-actor contexts use `snow_gc_alloc` (global arena), not per-actor heaps. If GC only handles per-actor heaps, the global arena grows unbounded.
**Why it happens:** The `snow_gc_alloc` function has a fallback path to the global arena when no actor context is available.
**How to avoid:** Keep the global arena as-is for non-actor contexts (main thread, startup). The global arena is small and doesn't need collection -- the main thread allocates very little. Focus GC on per-actor heaps only (via `snow_gc_alloc_actor` path).
**Warning signs:** Memory growth in programs that don't spawn actors.

### Pitfall 2: Missing Roots from Runtime Data Structures

**What goes wrong:** The GC only scans the coroutine stack for roots but misses pointers held in runtime-managed structures: the mailbox queue, service state, terminate callback arguments.
**Why it happens:** Messages in the mailbox contain pointers to heap objects (via `deep_copy_to_heap`). If GC doesn't consider these as roots, live objects get collected.
**How to avoid:** After conservative stack scanning, also scan the actor's mailbox messages and any actor-held state as roots. The `copy_msg_to_actor_heap` function allocates into the actor's heap, so those objects must be considered live.
**Warning signs:** Crashes after GC when an actor processes a message that was collected.

### Pitfall 3: Interior Pointers from Struct Field Access

**What goes wrong:** Snow's codegen uses `GEP` (GetElementPtr) to access struct fields. This creates interior pointers -- pointers into the middle of a GC-allocated object. Conservative stack scanning must recognize these as pointing to valid objects.
**Why it happens:** LLVM optimizations may hold interior pointers in registers or stack slots.
**How to avoid:** The `find_object_containing` function must check if a pointer falls anywhere within a heap page's allocated region, not just at the start of an object. Walk the all-objects list to find which object contains the pointer.
**Warning signs:** GC collects objects that are still in use via field references.

### Pitfall 4: GC During Allocation (Re-Entrancy)

**What goes wrong:** If GC is triggered during an allocation that itself triggers more allocations (e.g., the mark phase allocating a worklist), infinite recursion or corruption occurs.
**Why it happens:** The mark phase uses a `Vec<*mut GcHeader>` worklist which may allocate Rust heap memory, but this is fine since it's Rust heap (malloc), not Snow GC heap. The danger is if GC is triggered while mid-allocation in the GC heap.
**How to avoid:** Set a `gc_in_progress` flag on the heap. Skip GC trigger checks when the flag is set. The worklist for the mark phase should use Rust's standard allocator (Vec), not the GC heap.
**Warning signs:** Stack overflow or corruption during GC.

### Pitfall 5: Collecting Objects Reachable Through Collections (Lists, Maps)

**What goes wrong:** A Snow list `[str1, str2, str3]` stores string pointers as `u64` values in its data array. If the GC doesn't scan the list's body for pointers, the strings are collected while the list remains live.
**Why it happens:** Lists, maps, sets, etc. store `u64` values that may be pointers to other GC objects (strings, nested lists, etc.).
**How to avoid:** The mark phase must scan every word in every live object's body as a potential pointer. This is the conservative tracing approach -- treat every 8-byte word in an object as a potential pointer and check if it points into the heap.
**Warning signs:** String corruption or crashes after GC in programs that use collections.

### Pitfall 6: Header Alignment and Page Arithmetic

**What goes wrong:** Object headers must be properly aligned, and the header size must account for alignment of the user data that follows.
**Why it happens:** The header is 16 bytes (if using the design above), which is 8-byte aligned. User data starts at header + 16, which is also 8-byte aligned. This works for the common alignment=8 case. Larger alignments need padding.
**How to avoid:** Keep the header size as a multiple of 8 (16 bytes). For alignment > 8, add padding between the header and the data. In practice, all Snow allocations use alignment=8, so 16-byte headers work perfectly.
**Warning signs:** Alignment faults on ARM, or silent corruption on x86.

### Pitfall 7: String Runtime Functions Use Global Arena

**What goes wrong:** Functions like `snow_string_new`, `snow_string_concat`, etc. call `snow_gc_alloc` (global arena), not `snow_gc_alloc_actor`. These allocations bypass the per-actor heap and are never collected.
**Why it happens:** The string module was written before per-actor heaps existed.
**How to avoid:** Change all runtime allocation functions (strings, collections, JSON, file I/O, HTTP, env) to use `snow_gc_alloc_actor` instead of `snow_gc_alloc`. This routes allocations through the actor's heap when in an actor context.
**Warning signs:** Memory growth in actors that do heavy string operations. This is arguably the single biggest change needed.

### Pitfall 8: Coroutine Stack Boundaries for Conservative Scanning

**What goes wrong:** To conservatively scan the coroutine stack, we need to know the stack boundaries (bottom and current top). Corosensei provides the stack but doesn't expose the current stack pointer.
**Why it happens:** Corosensei abstracts away the stack details.
**How to avoid:** Capture the stack pointer at the GC safepoint (in `snow_reduction_check`) using inline assembly or a stack-local variable's address. The stack bottom is known from the `DefaultStack` allocation. Alternatively, the GC can be triggered BEFORE yielding (while still in the coroutine context), so the current stack frame provides the stack top.
**Warning signs:** Scanning too little (missing roots) or too much (scanning freed memory) of the stack.

## Code Examples

### ActorHeap with GC Support (New Design)

```rust
// Source: Custom design for Snow runtime

pub struct ActorHeap {
    /// Backing pages for bump allocation.
    pages: Vec<Vec<u8>>,
    /// Bump offset in current page.
    offset: usize,

    /// Head of the intrusive all-objects linked list.
    all_objects: *mut GcHeader,

    /// Head of the free list for reuse.
    free_list: *mut GcHeader,

    /// Total bytes currently allocated (live + free on free list).
    total_allocated: usize,
    /// Total live bytes (updated after each GC).
    live_bytes: usize,

    /// Threshold for triggering GC (bytes).
    gc_threshold: usize,
    /// Growth factor for threshold after each GC.
    gc_grow_factor: f64,

    /// Flag to prevent re-entrant GC.
    gc_in_progress: bool,
}

impl ActorHeap {
    const DEFAULT_GC_THRESHOLD: usize = 64 * 1024; // 64 KiB initial threshold
    const GC_GROW_FACTOR: f64 = 2.0;

    pub fn new() -> Self {
        ActorHeap {
            pages: vec![vec![0u8; ACTOR_PAGE_SIZE]],
            offset: 0,
            all_objects: std::ptr::null_mut(),
            free_list: std::ptr::null_mut(),
            total_allocated: 0,
            live_bytes: 0,
            gc_threshold: Self::DEFAULT_GC_THRESHOLD,
            gc_grow_factor: Self::GC_GROW_FACTOR,
            gc_in_progress: false,
        }
    }

    pub fn should_collect(&self) -> bool {
        !self.gc_in_progress && self.total_allocated >= self.gc_threshold
    }

    pub fn alloc(&mut self, size: usize, align: usize) -> *mut u8 {
        let total = GC_HEADER_SIZE + size;

        // 1. Try free list first.
        if let Some(ptr) = self.alloc_from_free_list(size) {
            return ptr;
        }

        // 2. Bump allocate from current page.
        self.bump_alloc(total, align)
    }
}
```

### GC Trigger Integration Point

```rust
// Source: Integration with existing snow_reduction_check in actor/mod.rs

#[no_mangle]
pub extern "C" fn snow_reduction_check() {
    // ... existing reduction counting logic ...

    // After reduction check, also check GC pressure.
    if let Some(pid) = stack::get_current_pid() {
        if stack::CURRENT_YIELDER.with(|c| c.get().is_some()) {
            // We're in a coroutine -- safe to GC.
            let sched = GLOBAL_SCHEDULER.get();
            if let Some(sched) = sched {
                if let Some(proc_arc) = sched.get_process(pid) {
                    let mut proc = proc_arc.lock();
                    if proc.heap.should_collect() {
                        // Capture stack pointer for conservative scanning.
                        let stack_top: *const u8 = &() as *const () as *const u8;
                        proc.heap.collect(stack_top);
                    }
                }
            }
        }
    }
}
```

### Routing All Runtime Allocations Through Actor Heap

```rust
// Source: Changes needed in snow-rt/src/string.rs (and all other modules)

// BEFORE (current):
pub extern "C" fn snow_string_new(data: *const u8, len: u64) -> *mut SnowString {
    let total = SnowString::HEADER_SIZE + len as usize;
    let ptr = snow_gc_alloc(total as u64, 8) as *mut SnowString;  // Global arena!
    // ...
}

// AFTER (with per-actor GC):
pub extern "C" fn snow_string_new(data: *const u8, len: u64) -> *mut SnowString {
    let total = SnowString::HEADER_SIZE + len as usize;
    let ptr = snow_gc_alloc_actor(total as u64, 8) as *mut SnowString;  // Actor heap!
    // ...
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Global arena (single Mutex) | Per-actor bump heap + global fallback | Phase 6 (v1.0) | Eliminated contention but no reclamation |
| `snow_gc_alloc` everywhere | `snow_gc_alloc_actor` for actor contexts | Phase 6 (v1.0) | Per-actor allocation path exists but most runtime fns still use global |
| No GC collection | **Phase 17: mark-sweep** | Now | Memory reclamation for long-running actors |

**Key architecture note:** BEAM/Erlang uses a **copying** semi-space collector, not mark-sweep. Snow is intentionally choosing mark-sweep because:
1. It avoids moving objects (no pointer fixup needed)
2. It works with LLVM-generated native code that holds raw pointers
3. It's simpler to implement
4. The per-actor isolation already provides the main latency benefit (no stop-the-world)

## Open Questions

1. **Stack scanning boundaries with corosensei**
   - What we know: Each coroutine has a 64 KiB stack allocated via `DefaultStack`. The stack grows downward on x86-64.
   - What's unclear: How to precisely determine the current stack top from within a coroutine. We need to capture a stack pointer value at the GC safepoint.
   - Recommendation: Use a simple `let marker: u8 = 0; let stack_top = &marker as *const u8;` to get a stack address near the top. This is a well-known technique. For the stack bottom, corosensei's `DefaultStack` provides the base address. Validate during implementation.

2. **Should the codegen emit `snow_gc_alloc` or `snow_gc_alloc_actor`?**
   - What we know: Currently, the codegen emits `snow_gc_alloc` for closure environments, spawn args, and tuples. The runtime functions (strings, lists, etc.) also use `snow_gc_alloc`.
   - What's unclear: Should the codegen be updated to call `snow_gc_alloc_actor` directly, or should `snow_gc_alloc` itself be changed to try the actor heap first?
   - Recommendation: Make `snow_gc_alloc` itself try the actor heap first (which it already partially does via `snow_gc_alloc_actor`). Actually, looking at the code, `snow_gc_alloc_actor` already does this -- it tries the actor heap and falls back to global. The real fix is to make ALL allocation sites use `snow_gc_alloc_actor` instead of `snow_gc_alloc`. This means updating the codegen AND runtime modules.

3. **Free list strategy: single list vs size-segregated**
   - What we know: Snow allocations are typically small (8-256 bytes for most objects). String allocations can be larger.
   - What's unclear: Whether a single free list with first-fit is sufficient, or whether size-segregated free lists (e.g., 8, 16, 32, 64, 128, 256, 512+ bytes) would improve allocation speed.
   - Recommendation: Start with a single free list with first-fit. Optimize to size-segregated if profiling shows allocation is a bottleneck. KISS for v1.2.

4. **Object header overhead**
   - What we know: A 16-byte header per object means 16 bytes overhead on every allocation. For a 8-byte string "hello", the overhead is 200%.
   - What's unclear: Whether this overhead is acceptable.
   - Recommendation: 16 bytes is standard for GC headers (comparable to JVM, V8, etc.). The BEAM's heap header is 1-2 words per term. For Snow, 16 bytes is fine -- the alternative (no headers, conservative everything) is worse. Small allocations are dominated by header overhead in any GC system.

## Sources

### Primary (HIGH confidence)
- **Snow codebase** (`crates/snow-rt/src/gc.rs`, `actor/heap.rs`, `actor/process.rs`, `actor/scheduler.rs`, `actor/stack.rs`, `actor/mod.rs`) -- complete read of existing GC and actor infrastructure
- **Snow codegen** (`crates/snow-codegen/src/codegen/expr.rs`, `codegen/types.rs`, `codegen/intrinsics.rs`) -- complete understanding of what emits GC allocation calls and how
- **Snow MIR** (`crates/snow-codegen/src/mir/mod.rs`) -- full MirType enum showing all types in the system
- **Snow runtime modules** (`string.rs`, `collections/list.rs`, `collections/map.rs`, etc.) -- all modules that allocate from GC

### Secondary (MEDIUM confidence)
- [Crafting Interpreters - Garbage Collection](https://craftinginterpreters.com/garbage-collection.html) -- well-documented mark-sweep GC implementation pattern; object headers, root enumeration, tricolor marking, adaptive thresholds
- [Erlang GC Documentation](https://www.erlang.org/doc/apps/erts/garbagecollection) -- per-process GC design in BEAM (though BEAM uses copying collector, not mark-sweep)
- [A Tour of Safe Tracing GC Designs in Rust](https://manishearth.github.io/blog/2021/04/05/a-tour-of-safe-tracing-gc-designs-in-rust/) -- Rust-specific GC design considerations
- [Designing a GC in Rust](https://manishearth.github.io/blog/2015/09/01/designing-a-gc-in-rust/) -- Root enumeration strategies, conservative vs precise scanning

### Tertiary (LOW confidence)
- [rust-gc library](https://github.com/Manishearth/rust-gc) -- Not directly applicable (it's for GC within Rust programs, not for implementing a GC for a compiled language), but validates the mark-sweep-in-Rust approach
- [Implementing a Safe Garbage Collector in Rust](https://coredumped.dev/2022/04/11/implementing-a-safe-garbage-collector-in-rust/) -- Context-based API pattern for GC in Rust

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- this is a custom implementation within an existing, well-understood codebase; no external dependencies to verify
- Architecture: HIGH -- mark-sweep GC is a well-understood algorithm; the per-actor isolation simplifies the design significantly; the codebase architecture (separate heap, ABI entry points, coroutine scheduling) maps cleanly to the required changes
- Pitfalls: HIGH -- identified from careful reading of the existing codebase; each pitfall maps to a specific code location

**Research date:** 2026-02-07
**Valid until:** Indefinite (this is foundational CS; mark-sweep GC algorithms don't change)
