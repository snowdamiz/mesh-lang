# Phase 6: Actor Runtime - Research

**Researched:** 2026-02-06
**Domain:** Preemptive actor runtime with work-stealing scheduler, per-actor GC, and typed message passing
**Confidence:** MEDIUM (novel systems-level engineering with well-understood prior art)

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions

#### Actor spawning & lifecycle
- Dedicated `actor` keyword block syntax -- actors are a first-class language construct, not just spawned closures
- Recursive functional state (Erlang-style) -- state passed as argument, updated by calling self with new state. No mutable state bindings.
- Terminate callback supported -- actors can define cleanup logic that runs before full termination (resource cleanup, final messages)

#### Message passing semantics
- Strict FIFO mailbox -- messages processed in arrival order, no selective receive
- Typed messages checked at compile time via Pid typing

#### Scheduling & preemption
- Reduction counting (BEAM-style) -- each actor gets N reductions before being preempted. Ensures fairness even with tight loops.
- M:N threading -- one OS thread per CPU core, actors multiplexed across cores with work-stealing for load balancing
- Basic priority levels -- high/normal/low. High-priority actors scheduled first. Useful for system-level actors.
- Crash behavior: actor killed, exit reason propagated to linked processes. No global impact.

#### Typed actor identity (Pid)
- Named process registration -- `register(pid, :name)` for global lookup by atom/string
- `self()` returns the actor's own Pid -- essential for reply-to patterns in messages
- Untyped `Pid` allowed as escape hatch -- Pid without type parameter permitted but requires runtime type check on send. Enables heterogeneous collections of actors.

### Claude's Discretion
- Normal exit behavior (whether return value is accessible to parent/linked process, or silent cleanup)
- Send semantics (fire-and-forget only vs both send and call primitives at the base level)
- Receive timeout support (after clause vs separate timer mechanism)
- Unmatched message handling (consistent with strict FIFO -- likely crash or drop-with-warning)
- Pid typing strategy (Pid<M> single message type vs Pid<Protocol> -- based on what Snow's current type system supports)

### Deferred Ideas (OUT OF SCOPE)
None -- discussion stayed within phase scope
</user_constraints>

## Summary

This phase builds the core actor runtime for Snow -- the highest engineering risk in the entire project. It requires three major subsystems working together: (1) a work-stealing M:N scheduler that multiplexes lightweight actor processes across CPU cores, (2) per-actor heap isolation with garbage collection (upgrading from Phase 5's bump allocator), and (3) compiler instrumentation for reduction counting to enable preemptive scheduling of compiled native code.

The standard approach, validated by BEAM (Erlang/OTP), Pony, and Lunatic, is to give each actor its own stack and heap, use stackful coroutines for context switching, and insert yield-check points at function calls and loop back-edges in the compiled output. The runtime is a Rust library (snow-rt) using crossbeam-deque for work-stealing queues and corosensei for fast, safe stack switching. The compiler instruments Snow code at the LLVM IR level to decrement a reduction counter and yield when exhausted.

The key architectural insight is that Snow already has all concurrency-related keywords reserved in the lexer (spawn, send, receive, link, monitor, trap, after, self), the type system supports generic type applications (`Ty::App` for `Pid<MessageType>`), and the runtime is already a standalone Rust crate (snow-rt) compiled as a static library. The work is primarily: (a) building the scheduler and process model in Rust, (b) extending the MIR with actor operations, (c) generating LLVM IR that calls into the runtime for spawn/send/receive, and (d) inserting reduction-counting yield checks.

**Primary recommendation:** Build the runtime bottom-up: scheduler and process model first (tested standalone in Rust), then per-actor GC, then compiler integration (new MIR nodes, codegen for spawn/send/receive/yield), then typed Pid in the type checker, then process linking. Each layer should be independently testable before integration.

## Standard Stack

### Core

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| crossbeam-deque | 0.8.6 | Work-stealing deques for scheduler run queues | Battle-tested, used by Tokio/Rayon, provides Worker/Stealer/Injector primitives |
| crossbeam-utils | latest | Scoped threads, CachePadded for false-sharing prevention | Essential companion to crossbeam-deque |
| corosensei | 0.3.2 | Stackful coroutine context switching | Fast (nanosecond switches), safe API, supports x86_64+AArch64 on macOS/Linux, guard pages |
| crossbeam-channel | latest | MPSC/MPMC channels for inter-scheduler communication | Bounded/unbounded, select support, used for runtime control signals |

### Supporting

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| rustc-hash | 2 | Fast hash maps (already in workspace) | Process registry, Pid-to-Process lookup tables |
| parking_lot | latest | Fast mutexes/rwlocks | Process registry concurrent access, named registration |
| atomic-waker | latest | Efficient waker notification | Scheduler sleep/wake when work arrives |

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| corosensei | setjmp/longjmp | Unsafe, breaks Rust destructors, no guard pages -- avoid |
| corosensei | LLVM coroutines (stackless) | Can't yield from arbitrary call depth, only at explicit suspend points -- insufficient for reduction counting |
| corosensei | Tokio async tasks | Cooperative only, no preemption, wrong abstraction for actors with blocking receive |
| crossbeam-deque | Custom lock-free queue | High complexity, crossbeam is proven and audited |
| Per-actor bump allocator | Global shared GC | Defeats actor isolation, causes stop-the-world pauses, not BEAM-like |

**Dependencies to add to snow-rt/Cargo.toml:**
```toml
[dependencies]
crossbeam-deque = "0.8"
crossbeam-utils = "0.8"
crossbeam-channel = "0.5"
corosensei = { version = "0.3", features = ["default-stack", "unwind"] }
parking_lot = "0.12"
```

## Architecture Patterns

### Recommended Project Structure (snow-rt expansion)

```
crates/snow-rt/
  src/
    lib.rs                  # Re-exports, snow_rt_init expanded
    gc.rs                   # Per-actor heap allocator (replaces global arena)
    string.rs               # String ops (unchanged API, new per-actor allocation)
    panic.rs                # Panic handler (becomes per-actor crash)
    actor/
      mod.rs                # Actor struct, ActorId, ProcessControlBlock
      mailbox.rs            # FIFO mailbox (lock-free MPSC queue)
      pid.rs                # Pid representation, process registry
      state.rs              # Actor lifecycle states
    scheduler/
      mod.rs                # Scheduler main loop, thread pool
      worker.rs             # Per-core worker thread with run queue
      stealing.rs           # Work-stealing logic
      reduction.rs          # Reduction counter management
      priority.rs           # Priority queue (high/normal/low)
    signal.rs               # Exit signals, link/unlink, process linking
    registry.rs             # Named process registration (atom -> Pid)
```

### Pattern 1: Process Control Block (PCB)

**What:** Each actor is represented by a PCB (following BEAM terminology) containing its stack, heap, mailbox, reduction counter, link set, and state.

**When to use:** Every actor spawn creates a PCB. The scheduler operates on PCBs.

```rust
// Conceptual -- Snow runtime (Rust)
pub struct ProcessControlBlock {
    pub id: ActorId,
    pub priority: Priority,
    pub state: AtomicU8,  // Running, Runnable, Waiting, Exiting
    pub reductions_left: i32,
    pub mailbox: Mailbox,
    pub heap: ActorHeap,
    pub coroutine: Option<Coroutine<(), (), ()>>,
    pub links: Mutex<HashSet<ActorId>>,
    pub trap_exit: bool,
    pub exit_reason: Option<ExitReason>,
}
```

### Pattern 2: M:N Scheduler with Work Stealing

**What:** One OS thread per CPU core. Each thread owns a Worker deque. Actors are pushed onto local deques. When a worker's deque is empty, it steals from others via Stealer handles.

**When to use:** This is THE scheduling pattern. All actor execution goes through this.

```rust
// Conceptual scheduler loop per worker thread
fn worker_loop(worker: Worker<Arc<PCB>>, stealers: Vec<Stealer<Arc<PCB>>>, injector: Arc<Injector<Arc<PCB>>>) {
    loop {
        // 1. Pop from local queue
        if let Some(pcb) = worker.pop() {
            run_actor(pcb);
            continue;
        }
        // 2. Steal from injector (global queue)
        if let Steal::Success(pcb) = injector.steal_batch_and_pop(&worker) {
            run_actor(pcb);
            continue;
        }
        // 3. Steal from other workers
        for stealer in &stealers {
            if let Steal::Success(pcb) = stealer.steal_batch_and_pop(&worker) {
                run_actor(pcb);
                break;
            }
        }
        // 4. Sleep briefly if no work found (park/unpark)
    }
}
```

### Pattern 3: Reduction Counting via Compiler Instrumentation

**What:** The compiler inserts calls to a reduction-check function at every function call site and loop back-edge. When the counter hits zero, the actor yields to the scheduler.

**When to use:** Every compiled Snow function and loop gets instrumented.

```llvm
; Conceptual LLVM IR instrumentation at function entry
define void @snow_user_function() {
entry:
  ; Decrement reduction counter and check
  %reds = call i32 @snow_reduction_check()
  %exhausted = icmp sle i32 %reds, 0
  br i1 %exhausted, label %yield, label %continue

yield:
  call void @snow_yield()    ; Context switch back to scheduler
  br label %continue

continue:
  ; ... actual function body ...
}
```

The runtime provides `snow_reduction_check()` and `snow_yield()` as extern "C" functions. `snow_yield()` saves the actor's context via corosensei and returns control to the scheduler.

### Pattern 4: Per-Actor Heap with Bump Allocation + Mark-Sweep

**What:** Each actor gets its own heap (arena). Allocation is fast bump allocation within the actor's pages. GC runs per-actor using mark-sweep, only when the actor is suspended (no stack to scan).

**When to use:** All GC-managed allocations (strings, tuples, closures, ADTs) go through the actor's local heap.

```rust
// Conceptual per-actor heap
pub struct ActorHeap {
    pages: Vec<Vec<u8>>,
    offset: usize,
    total_allocated: usize,
    gc_threshold: usize,  // Trigger GC when total_allocated exceeds this
}

impl ActorHeap {
    pub fn alloc(&mut self, size: usize, align: usize) -> *mut u8 {
        // Same bump allocation as current global arena
        // but scoped to this actor
    }

    pub fn collect(&mut self, roots: &[*const u8]) {
        // Mark-sweep: mark from roots, sweep unmarked
        // Only runs when actor is NOT executing (no stack)
    }
}
```

### Pattern 5: Actor Receive as Blocking Yield

**What:** When an actor calls `receive`, if no matching message exists, the actor suspends (state = Waiting) and yields to the scheduler. When a message arrives, the scheduler re-enqueues the actor.

**When to use:** Every `receive` expression in Snow code.

```rust
// Runtime implementation of receive
pub extern "C" fn snow_actor_receive(timeout_ms: i64) -> *const u8 {
    let pcb = current_pcb();

    // Try to dequeue a message
    if let Some(msg) = pcb.mailbox.try_pop() {
        return msg;
    }

    // No message: suspend and yield
    pcb.set_state(ActorState::Waiting);
    if timeout_ms > 0 {
        schedule_timeout(pcb.id, timeout_ms);
    }
    snow_yield();  // Returns when a message arrives or timeout fires

    // Re-check mailbox after wake
    pcb.mailbox.try_pop().unwrap_or(std::ptr::null())
}
```

### Anti-Patterns to Avoid

- **Shared mutable state between actors:** Every piece of data must be copied into messages or owned by exactly one actor. The per-actor heap enforces this.
- **Global stop-the-world GC:** Never pause all actors for GC. Each actor collects independently.
- **Spinlocks in scheduler:** Use parking/waking (futex-based) not busy-waiting when no work is available.
- **Unbounded mailboxes with no backpressure:** For Phase 6, mailboxes are unbounded (matching BEAM), but the planner should note this as a future concern.
- **OS thread per actor:** This defeats the entire purpose. Actors MUST be lighter than threads (target: ~1KB overhead per actor).

## Claude's Discretion Recommendations

Based on research into BEAM, Pony, and the capabilities of Snow's existing type system:

### Normal exit behavior
**Recommendation:** Silent cleanup. When an actor's function returns normally, linked processes receive `{:exit, pid, :normal}` signal which is ignored by default (matching BEAM semantics). The return value is NOT accessible to the parent -- use message passing for that. Rationale: Keeping return values would require complex shared-memory coordination that conflicts with actor isolation.

### Send semantics
**Recommendation:** Fire-and-forget `send` only at the base level. No `call` (synchronous request-reply) primitive in Phase 6. Rationale: `call` is a higher-level pattern built on `send` + `receive` with a unique reference tag. It belongs in Phase 9 (GenServer). Base-level `send` is simpler and matches BEAM's `!` operator.

### Receive timeout support
**Recommendation:** Support `after` clause in receive blocks. Syntax: `receive do ... after 5000 -> timeout_handler end`. Rationale: The `after` keyword is already reserved in the lexer. Timeouts are essential for practical actor programming (avoiding permanent blocks). Implementation via the scheduler's timer wheel is straightforward. This matches BEAM's `receive ... after` syntax.

### Unmatched message handling
**Recommendation:** Crash the actor with `{:exit, :no_match}` reason when a message arrives that matches no clause in the receive block. This is consistent with strict FIFO (messages cannot be skipped) and the fail-fast philosophy. The linked processes get the exit signal. Rationale: BEAM uses selective receive (skip non-matching) but Snow's CONTEXT.md explicitly locks "strict FIFO -- no selective receive." With FIFO, an unmatched message blocks the mailbox permanently, so crashing is the only sane option.

### Pid typing strategy
**Recommendation:** Use `Pid<M>` where M is the message type (single type parameter). The type system already supports `Ty::App(Box<Ty::Con("Pid")>, vec![message_type])` via the existing generic type application mechanism. For actors that accept multiple message types, use a sum type: `Pid<CounterMsg>` where `type CounterMsg = Increment | Decrement | GetCount(Pid<Int>)`. This leverages Phase 4's sum types naturally. The untyped `Pid` escape hatch is `Pid` without a type parameter (handled as `Ty::Con("Pid")` without App).

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Work-stealing deque | Custom lock-free deque | crossbeam-deque 0.8.6 | Lock-free concurrent data structure requires formal proof; crossbeam is battle-tested |
| Context switching | Custom assembly or setjmp/longjmp | corosensei 0.3.2 | Platform-specific assembly, unwinding, guard pages -- extremely error-prone |
| MPSC mailbox queue | Custom lock-free queue | crossbeam-channel or custom based on crossbeam-deque | Correct MPSC requires careful atomic ordering |
| Thread parking/waking | Custom futex wrapper | std::thread::park/unpark or crossbeam | OS-specific futex details are tricky |
| Hash maps for registry | Custom concurrent map | parking_lot::RwLock<FxHashMap> | Good enough for Phase 6; concurrent skip-list is premature optimization |

**Key insight:** The actor runtime's complexity comes from the scheduling and isolation architecture, not from individual data structures. Use proven concurrent building blocks and focus engineering effort on the integration: how the scheduler, GC, compiler instrumentation, and actor lifecycle interact.

## Common Pitfalls

### Pitfall 1: Stack Size for Actors
**What goes wrong:** Allocating too little stack per actor causes stack overflow crashes. Allocating too much wastes memory and prevents spawning many actors.
**Why it happens:** Default OS thread stacks are 8MB. Actors need much smaller stacks (8KB-64KB) but the right size depends on recursion depth.
**How to avoid:** Start with 64KB per actor stack (matching corosensei's default). corosensei provides guard pages that catch overflow with a clean segfault rather than silent corruption. Allow configurable stack size per actor as a future enhancement.
**Warning signs:** Segfaults on actor spawn when stack is too small; OOM when spawning 100K actors if stack is too large.

### Pitfall 2: Reduction Count Granularity
**What goes wrong:** Too few reductions per slice causes excessive context-switch overhead. Too many allows unfair monopolization.
**Why it happens:** BEAM uses 4000 reductions per context switch (was 2000 in older versions). One "reduction" = roughly one function call.
**How to avoid:** Start with 4000 reductions per actor time slice (matching modern BEAM). Make it configurable. Instrument at function call sites and loop back-edges only -- not every single instruction.
**Warning signs:** Benchmarks showing >10% overhead from yield checking; or tight-loop actors starving others.

### Pitfall 3: Message Copying vs. Ownership Transfer
**What goes wrong:** If messages share pointers between actor heaps, one actor's GC can free data another actor is using.
**Why it happens:** Per-actor heap isolation requires that messages are either deep-copied into the receiver's heap or use reference-counted shared objects.
**How to avoid:** For Phase 6, deep-copy all messages into the receiver's heap on send. This is what BEAM does for most terms. Large binaries can use reference-counted shared storage (like BEAM's Refc binaries) as a future optimization.
**Warning signs:** Use-after-free crashes when GC runs on sender's heap after message was sent.

### Pitfall 4: Deadlock in Scheduler Shutdown
**What goes wrong:** When the main actor exits, other actors may be blocked in receive. The scheduler threads may park waiting for work that never comes.
**Why it happens:** No clean shutdown protocol.
**How to avoid:** When the main actor (snow_main) exits, the runtime should: (1) send exit signals to all linked processes, (2) set a global "shutting down" flag, (3) wake all parked scheduler threads, (4) allow a grace period for cleanup, (5) forcibly terminate remaining actors.
**Warning signs:** Program hangs on exit instead of terminating cleanly.

### Pitfall 5: GC During Message Send
**What goes wrong:** If GC triggers while building a message (between allocating message space and copying data), the half-built message may be corrupted.
**Why it happens:** GC can be triggered by any allocation if the heap exceeds its threshold.
**How to avoid:** Either (a) pin the message being built so GC doesn't move it, or (b) only trigger GC at safe points (before message construction, not during), or (c) use a separate allocation for message construction that's outside the GC heap.
**Warning signs:** Corrupted messages arriving at receivers, especially under high allocation pressure.

### Pitfall 6: Process Registry Contention
**What goes wrong:** Named process lookup (`register`/`whereis`) becomes a bottleneck under high concurrency.
**Why it happens:** Global registry requires synchronization across all scheduler threads.
**How to avoid:** Use a RwLock (parking_lot) for the registry. Reads (whereis) are lock-free when there are no writers. Writes (register/unregister) are rare compared to reads. This is sufficient for Phase 6.
**Warning signs:** Benchmark showing registry lookup taking microseconds instead of nanoseconds.

## Code Examples

### Actor Syntax in Snow (proposed)

```snow
# A simple counter actor
actor Counter(count :: Int) do
  receive do
    :increment ->
      Counter(count + 1)
    :decrement ->
      Counter(count - 1)
    {:get, caller :: Pid<Int>} ->
      send(caller, count)
      Counter(count)
  end
end

# Spawning and messaging
let pid = spawn(Counter, 0)
send(pid, :increment)
send(pid, :increment)
send(pid, {:get, self()})
receive do
  count :: Int -> println("Count: ${count}")
end
```

### Runtime ABI -- New Extern "C" Functions

```rust
// New functions to add to snow-rt intrinsics

/// Initialize the actor runtime (scheduler threads, etc.)
/// Replaces simple snow_rt_init for actor-enabled programs.
#[no_mangle]
pub extern "C" fn snow_rt_init_actor(num_schedulers: u32) { ... }

/// Spawn a new actor. Returns a Pid (opaque pointer).
/// `fn_ptr` points to the actor's compiled entry function.
/// `args` is a pointer to the serialized initial arguments.
/// `args_size` is the byte size of the arguments.
#[no_mangle]
pub extern "C" fn snow_actor_spawn(
    fn_ptr: *const u8,
    args: *const u8,
    args_size: u64,
    priority: u8,
) -> u64 { ... }  // Returns ActorId as u64

/// Send a message to an actor by Pid.
/// `msg` is a pointer to the serialized message data.
/// `msg_size` is the byte size.
#[no_mangle]
pub extern "C" fn snow_actor_send(
    target_pid: u64,
    msg: *const u8,
    msg_size: u64,
) { ... }

/// Receive a message from the current actor's mailbox.
/// Blocks (yields) if no message is available.
/// Returns pointer to message data in current actor's heap.
#[no_mangle]
pub extern "C" fn snow_actor_receive(
    timeout_ms: i64,  // -1 = infinite, 0 = non-blocking, >0 = ms
) -> *const u8 { ... }

/// Get the current actor's Pid.
#[no_mangle]
pub extern "C" fn snow_actor_self() -> u64 { ... }

/// Link current actor to another actor.
#[no_mangle]
pub extern "C" fn snow_actor_link(other_pid: u64) { ... }

/// Check reduction counter and yield if exhausted.
/// Called by compiler-inserted instrumentation.
#[no_mangle]
pub extern "C" fn snow_reduction_check() { ... }

/// Allocate from the CURRENT ACTOR's heap (replaces global snow_gc_alloc).
#[no_mangle]
pub extern "C" fn snow_gc_alloc(size: u64, align: u64) -> *mut u8 { ... }

/// Register current actor with a name.
#[no_mangle]
pub extern "C" fn snow_actor_register(
    name: *const u8,
    name_len: u64,
) { ... }

/// Look up a registered actor by name.
#[no_mangle]
pub extern "C" fn snow_actor_whereis(
    name: *const u8,
    name_len: u64,
) -> u64 { ... }  // Returns ActorId or 0 if not found
```

### MIR Extensions for Actor Operations

```rust
// New MirExpr variants needed
pub enum MirExpr {
    // ... existing variants ...

    /// Spawn a new actor process.
    ActorSpawn {
        fn_name: String,          // Compiled actor entry function
        args: Vec<MirExpr>,       // Initial arguments
        priority: Priority,
        ty: MirType,              // Pid<MessageType>
    },

    /// Send a message to a Pid.
    ActorSend {
        target: Box<MirExpr>,     // Pid expression
        message: Box<MirExpr>,    // Message expression
        ty: MirType,              // Unit
    },

    /// Receive from current actor's mailbox.
    ActorReceive {
        arms: Vec<MirMatchArm>,   // Pattern matching on messages
        timeout: Option<Box<MirExpr>>,  // Optional after clause
        ty: MirType,
    },

    /// Get self() Pid.
    ActorSelf {
        ty: MirType,              // Pid<OwnMessageType>
    },

    /// Link to another actor.
    ActorLink {
        target: Box<MirExpr>,     // Pid expression
        ty: MirType,              // Unit
    },
}

// New MirType variant
pub enum MirType {
    // ... existing variants ...

    /// Actor Pid type, optionally parameterized by message type.
    Pid(Option<Box<MirType>>),    // Pid<Int> = Pid(Some(Int)), untyped Pid = Pid(None)
}
```

### Compiler Instrumentation for Yield Points

```rust
// In codegen, instrument every function call and loop back-edge:
fn codegen_call_with_reduction_check(&mut self, call: &MirExpr) -> Result<BasicValueEnum, String> {
    // Insert reduction check BEFORE the call
    let check_fn = self.module.get_function("snow_reduction_check").unwrap();
    self.builder.build_call(check_fn, &[], "").map_err(|e| e.to_string())?;

    // Now emit the actual call
    self.codegen_call(call)
}

fn codegen_loop_backedge_with_reduction_check(&mut self, backedge_bb: BasicBlock) {
    // Insert reduction check at loop back-edge
    let check_fn = self.module.get_function("snow_reduction_check").unwrap();
    self.builder.build_call(check_fn, &[], "").map_err(|e| e.to_string())?;

    // Branch back to loop header
    self.builder.build_unconditional_branch(backedge_bb).unwrap();
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| OS threads per actor (Akka pre-ForkJoin) | M:N green threads (BEAM, Pony, Go) | 2000s | 100K+ actors possible |
| Global shared heap GC | Per-actor isolated heaps (BEAM, Pony ORCA) | BEAM since inception | No stop-the-world pauses |
| Time-slice preemption | Reduction counting (BEAM) | BEAM since inception | Fair scheduling without timer interrupts |
| setjmp/longjmp context switch | Platform-specific assembly (corosensei, libfringe) | 2020s | Safe, fast (<10ns), debugger-compatible |
| Rust libgreen (M:N threading) | Removed in Rust 1.0, replaced by async/await ecosystem | 2014 | Snow must build its own runtime (no stdlib support) |
| Cooperative scheduling (Tokio) | Fine for I/O-bound; insufficient for CPU-bound actor fairness | N/A | Snow needs reduction counting, not just async/await |

**Deprecated/outdated:**
- Rust's `libgreen` (removed 2014): Was M:N threading in std, removed for zero-cost abstraction reasons. Snow builds its own.
- `libfringe`: Predecessor to corosensei, unmaintained. Use corosensei instead.
- `context` crate (Boost.Context port): Less maintained than corosensei, fewer safety guarantees.

## Per-Actor GC Strategy

The current Phase 5 GC is a global mutex-protected bump allocator with no collection. Phase 6 must upgrade this to per-actor heaps.

**Approach: Per-actor bump allocation with deferred mark-sweep.**

1. **Allocation:** Each actor has its own `ActorHeap` (bump allocator, identical algorithm to current `Arena`). `snow_gc_alloc` reads a thread-local pointer to the current actor's heap.
2. **Collection trigger:** When an actor's heap exceeds a threshold (e.g., 256KB), schedule GC for the next time the actor yields.
3. **Collection algorithm:** Simple mark-sweep. Roots are the actor's stack frame values (passed by the compiled code). Mark all reachable objects, sweep (free) unreachable ones. Since actors are isolated, this only stops the one actor being collected.
4. **No collection during execution:** GC only runs when the actor is not executing (between reductions or when blocked on receive). This eliminates the need for stack maps or safepoints inside running code.
5. **Message handling:** Messages are deep-copied from sender's heap to receiver's heap during `snow_actor_send`. This maintains heap isolation.

**Phase 6 simplification:** Start with bump allocation only (no collection), matching Phase 5. Add mark-sweep collection as a later task within Phase 6 or defer to a 6.1 patch phase. The key structural change is per-actor heaps, not the collection algorithm.

## Context Switching Implementation Detail

Snow compiles to native code via LLVM. Actors run as stackful coroutines via corosensei.

**How it works:**
1. Each actor's entry function is wrapped in a `Coroutine::new()` call at spawn time.
2. The actor runs on corosensei's managed stack (64KB default, with guard page).
3. When `snow_reduction_check()` detects exhausted reductions, it calls `yielder.suspend(())` which saves the actor's CPU registers and stack pointer, then returns control to the scheduler.
4. The scheduler picks the next actor and calls `coroutine.resume(())` on its PCB, which restores that actor's registers and stack pointer.
5. `receive` with no available messages also calls `yielder.suspend(())` after setting state to Waiting.

**Critical detail:** The `Yielder` must be accessible from the compiled Snow code. The approach is to store a pointer to the Yielder in a thread-local variable that `snow_yield()` reads. Since each scheduler thread runs one actor at a time, thread-local storage is safe.

```rust
thread_local! {
    static CURRENT_YIELDER: Cell<*const ()> = Cell::new(std::ptr::null());
    static CURRENT_PCB: Cell<*const ProcessControlBlock> = Cell::new(std::ptr::null());
}
```

## Open Questions

1. **Message serialization format**
   - What we know: Messages must be deep-copied between heaps. BEAM copies terms word-by-word.
   - What's unclear: Should Snow define a serialization format, or just memcpy tagged values? Tagged values require a runtime type tag system.
   - Recommendation: Use tagged values (type tag + payload). The MIR already has type information; embed a small type tag in the runtime representation. Start simple: each value is `{ u8 tag, [u8; N] payload }` where N depends on the type.

2. **How `actor` blocks compile to functions**
   - What we know: The user writes `actor Counter(state) do receive ... end end`. The compiler must lower this to a loop function that receives messages and recurses with new state.
   - What's unclear: Exact compilation strategy -- does the actor block become a single function? Multiple functions?
   - Recommendation: Lower `actor Name(args) do receive do ... end end` to a function `__actor_Name(args) -> Never` that contains a loop: receive message, pattern match, call self recursively with new state. The recursive call is a tail call (or loop iteration) to avoid stack growth.

3. **Integration with existing `main` wrapper**
   - What we know: Currently `main` calls `snow_rt_init()` then the Snow entry function. With actors, `main` must initialize the scheduler, run `snow_main` as the initial actor, and wait for completion.
   - What's unclear: Does `snow_main` run as an actor, or as a special "main process" like BEAM's init?
   - Recommendation: `snow_main` runs as an actor (the "init" process). When it exits, the runtime shuts down. This matches BEAM behavior and simplifies the model.

4. **Stack size vs. actor count tradeoff**
   - What we know: 64KB stack x 100K actors = 6.4GB virtual memory. Physical memory depends on page faults.
   - What's unclear: Will 100K actors with 64KB stacks work on typical systems?
   - Recommendation: Use 64KB virtual allocation with guard pages. Modern OS lazy-commits pages, so actual memory is much less. corosensei's default-stack feature handles this. Verify with the 100K actor success criterion early.

5. **Typed Pid runtime representation**
   - What we know: `Pid<T>` is checked at compile time. At runtime, a Pid is just a u64 (actor ID).
   - What's unclear: Does the runtime need to store the message type for untyped Pid runtime checks?
   - Recommendation: At runtime, Pid is always u64. Type checking is purely compile-time for `Pid<T>`. For untyped `Pid`, the runtime stores a type tag hash in the PCB, and `snow_actor_send` validates at runtime that the message tag matches. This is the "escape hatch" cost.

## Sources

### Primary (HIGH confidence)
- [Erlang GC Documentation](https://www.erlang.org/doc/apps/erts/garbagecollection.html) -- Per-process GC algorithm details
- [The BEAM Book - Scheduling](https://github.com/happi/theBeamBook/blob/master/chapters/scheduling.asciidoc) -- Reduction counting, scheduler architecture, work stealing
- [crossbeam-deque docs](https://docs.rs/crossbeam-deque) -- Work-stealing deque API (v0.8.6)
- [corosensei GitHub](https://github.com/Amanieu/corosensei) -- Stackful coroutine API, platform support, performance
- [Erlang Process Reference Manual](https://www.erlang.org/doc/system/ref_man_processes.html) -- Link/monitor/exit signal semantics
- Snow codebase analysis -- snow-rt, snow-codegen, MIR types, type system (direct file reads)

### Secondary (MEDIUM confidence)
- [AppSignal: Deep Diving Into Erlang Scheduler](https://blog.appsignal.com/2024/04/23/deep-diving-into-the-erlang-scheduler.html) -- Verified against BEAM Book
- [Tokio scheduler blog post](https://tokio.rs/blog/2019-10-scheduler) -- Work-stealing patterns, verified with crossbeam-deque docs
- [Pony language actor runtime](https://dev.to/viz-x/pony-the-actor-model-language-built-for-high-safety-concurrency-c2a) -- Per-actor GC, work-stealing, typed messaging patterns
- [Lunatic runtime](https://lunatic.solutions/) -- BEAM-like runtime in Rust using Wasm for isolation

### Tertiary (LOW confidence)
- [Erlang GC Details blog](https://hamidreza-s.github.io/erlang%20garbage%20collection%20memory%20layout%20soft%20realtime/2015/08/24/erlang-garbage-collection-details-and-why-it-matters.html) -- Useful detail but older post
- [rust-fibers Medium article](https://medium.com/@ksaritek/building-lightweight-coroutines-in-rust-introducing-rust-fibers-53b91625a9de) -- Educational but not authoritative
- [Pony ORCA paper](https://www.ponylang.io/media/papers/opsla237-clebsch.pdf) -- Academic paper, details may not directly apply

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- crossbeam-deque and corosensei are well-documented, version-verified Rust crates
- Architecture (scheduler/PCB): MEDIUM -- based on BEAM architecture adapted for native compilation; novel integration work
- Architecture (per-actor GC): MEDIUM -- Erlang approach is well-documented; implementing it in Rust with LLVM codegen is less proven
- Compiler instrumentation: MEDIUM -- reduction counting concept is proven in BEAM; inserting yield checks in LLVM IR is novel for Snow
- Pitfalls: MEDIUM -- drawn from BEAM/Pony prior art and Rust concurrency experience
- Pid typing: MEDIUM -- Snow's type system supports the needed generics; exact integration needs validation

**Research date:** 2026-02-06
**Valid until:** 2026-03-06 (30 days -- stable domain, libraries versioned)
