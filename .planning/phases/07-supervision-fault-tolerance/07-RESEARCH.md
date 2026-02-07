# Phase 7: Supervision & Fault Tolerance - Research

**Researched:** 2026-02-06
**Domain:** OTP-style supervision trees with restart strategies, let-it-crash semantics, typed child specs
**Confidence:** HIGH (well-understood OTP prior art, comprehensive codebase analysis)

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions

#### Supervisor syntax & API
- Supervision trees nestable from the start -- a supervisor can supervise other supervisors
- Children start sequentially in declared order (Erlang style) -- if one fails to start, remaining children are not started and supervisor fails

#### Restart strategies & limits
- All four OTP strategies: one_for_one, one_for_all, rest_for_one, simple_one_for_one
- Restart limits follow Erlang model: max_restarts within max_seconds (e.g., 3 restarts in 5 seconds)
- Supervisor terminates when restart limit exceeded -- propagates up to parent supervisor (standard OTP escalation)
- Per-supervisor configurable limits with sensible defaults (Erlang defaults: 3 restarts in 5 seconds)

#### Crash propagation & trap_exit
- Structured ExitReason sum type: Normal, Shutdown, Custom(String) -- typed and pattern-matchable at compile time
- Fresh Pid on restart (Erlang style) -- old Pid references become stale, named registration handles lookup
- Erlang exit-to-restart semantics: permanent children restart on any exit (normal or abnormal), transient only on abnormal, temporary never

#### Child specification
- All three restart types: permanent (always restart), transient (restart on abnormal), temporary (never restart)
- Configurable shutdown per-child: timeout in ms or brutal_kill, with sensible default (e.g., 5000ms)
- Full compile-time validation of child specs -- supervisor knows child message types, mismatched start functions or invalid specs caught at compile time

### Claude's Discretion
- Supervisor definition syntax (dedicated block vs function-based vs trait-based)
- Supervisor naming policy (auto vs explicit registration)
- trap_exit mechanism design
- Child spec struct/representation design
- Shutdown signal implementation details
- simple_one_for_one dynamic child management internals

### Deferred Ideas (OUT OF SCOPE)
None -- discussion stayed within phase scope
</user_constraints>

## Summary

Phase 7 adds OTP-style supervision trees to Snow. A supervisor is a special actor that monitors child actors, automatically restarts them on failure according to configurable strategies (one_for_one, one_for_all, rest_for_one, simple_one_for_one), and escalates to its parent supervisor when restart limits are exceeded. The supervisor builds directly on Phase 6's process linking (`link.rs`), exit signal propagation (`EXIT_SIGNAL_TAG = u64::MAX`), trap_exit flag, named process registry, and terminate callbacks.

The implementation requires work at three layers: (1) the runtime (snow-rt) where the Supervisor actor struct, restart logic, child lifecycle management, and shutdown sequencing live; (2) the compiler frontend where supervisor syntax is parsed, type-checked, and lowered to MIR; and (3) compile-time child spec validation where the type checker ensures child start functions and message types are correct. The runtime is the heaviest layer -- the supervisor is itself an actor that traps exits and implements the restart state machine.

The standard approach, following Erlang/OTP, is to make the supervisor an actor with `trap_exit = true` that receives exit signals as messages, pattern-matches on them, and applies the configured restart strategy. The supervisor maintains an ordered list of child specs and their current PIDs, a restart history (timestamps for restart limit tracking), and the strategy/limits configuration. The Erlang supervisor's `init/1` callback pattern maps naturally to Snow's `supervisor` block syntax.

**Primary recommendation:** Implement supervisors as runtime-level actors (not a compiler primitive). Use the existing `supervisor` keyword (already reserved in the lexer) for a dedicated block syntax. The supervisor runs as a normal actor with `trap_exit = true` that receives exit signals and manages child lifecycles. Child specs are Snow structs validated at compile time. The runtime provides supervisor-specific functions (`snow_supervisor_start`, `snow_supervisor_start_child`, etc.) while the compiler validates child spec types.

## Standard Stack

### Core

No new external dependencies needed. Phase 7 builds entirely on the existing snow-rt infrastructure.

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| snow-rt (existing) | N/A | Runtime: scheduler, process linking, exit signals, registry | Already provides all concurrency primitives needed |
| snow-parser (existing) | N/A | Parser: `supervisor` keyword already reserved | Existing parsing infrastructure for new syntax |
| snow-typeck (existing) | N/A | Type checker: child spec validation | Existing Pid<M> typing for message type validation |
| snow-codegen (existing) | N/A | MIR + LLVM codegen for supervisor constructs | Existing actor codegen patterns to extend |

### Supporting

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| std::time::Instant | stdlib | Restart timestamp tracking for restart limits | Every restart event records a timestamp |
| std::collections::VecDeque | stdlib | Restart history (sliding window of timestamps) | Checking max_restarts within max_seconds |

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Runtime-level supervisor actor | Compiler-generated supervisor code | Runtime actor is simpler, more flexible, matches Erlang; compiler-generated would be harder to debug |
| `supervisor` block syntax | Function-based API only | Block syntax is more readable and enables compile-time validation; function API alone loses type information |
| VecDeque for restart history | Circular buffer | VecDeque is simpler, restart history is small (max ~10-20 entries) |

## Architecture Patterns

### Recommended Project Structure

```
crates/snow-rt/src/
  actor/
    mod.rs                # Updated: new supervisor extern "C" functions
    process.rs            # Updated: ExitReason::Shutdown variant, trap_exit_set fn
    link.rs               # Updated: shutdown signal handling
    supervisor.rs         # NEW: Supervisor struct, strategies, restart logic, child management
    child_spec.rs         # NEW: ChildSpec, RestartType, ShutdownType

crates/snow-parser/src/
  parser/items.rs         # Updated: parse_supervisor_def
  syntax_kind.rs          # Updated: SUPERVISOR_DEF, CHILD_SPEC, STRATEGY nodes
  ast/item.rs             # Updated: SupervisorDef AST node

crates/snow-typeck/src/
  infer.rs                # Updated: infer_supervisor_def, validate_child_specs

crates/snow-codegen/src/
  mir/mod.rs              # Updated: SupervisorStart MIR node
  mir/lower.rs            # Updated: lower_supervisor_def
  codegen/expr.rs         # Updated: codegen_supervisor_start
  codegen/intrinsics.rs   # Updated: declare supervisor runtime functions
```

### Pattern 1: Supervisor as a trap_exit Actor

**What:** A supervisor is an actor that sets `trap_exit = true` on itself, then enters a receive loop processing exit signals from children. When a child exits, the supervisor receives the exit signal as a message (not a crash) and applies the restart strategy.

**When to use:** This is THE pattern for supervision. Every supervisor instance follows this.

**Existing infrastructure:**
- `Process.trap_exit` field exists (Phase 6, default false)
- `link::propagate_exit` already delivers exit signals as messages when `trap_exit = true`
- `EXIT_SIGNAL_TAG = u64::MAX` identifies exit signals in the mailbox
- `link::encode_exit_signal` serializes `(exiting_pid, reason)` into message data

**Supervisor receive loop (conceptual Rust):**
```rust
fn supervisor_loop(sup: &mut SupervisorState) {
    loop {
        // Block on receive -- the scheduler will wake us when a child exits
        let msg = snow_actor_receive(-1); // infinite wait

        if msg_is_exit_signal(msg) {
            let (child_pid, reason) = decode_exit_signal(msg);
            handle_child_exit(sup, child_pid, reason);
        }
        // Supervisors ignore non-exit messages (or could log them)
    }
}

fn handle_child_exit(sup: &mut SupervisorState, child_pid: ProcessId, reason: ExitReason) {
    let child_spec = sup.find_child_by_pid(child_pid);

    match child_spec.restart_type {
        RestartType::Permanent => restart_child(sup, child_spec),
        RestartType::Transient => {
            if !matches!(reason, ExitReason::Normal | ExitReason::Shutdown) {
                restart_child(sup, child_spec);
            }
        }
        RestartType::Temporary => {
            // Never restart. Remove from children list.
            sup.remove_child(child_pid);
        }
    }
}
```

### Pattern 2: Restart Strategies as Strategy Dispatch

**What:** The four restart strategies differ only in which children are stopped and restarted when one child exits. Factor this into a strategy dispatch function.

**When to use:** Every child exit routes through the strategy dispatcher.

```rust
fn apply_strategy(sup: &mut SupervisorState, failed_child: &ChildSpec, reason: &ExitReason) {
    match sup.strategy {
        Strategy::OneForOne => {
            // Restart only the failed child
            restart_child(sup, failed_child);
        }
        Strategy::OneForAll => {
            // Terminate all children in reverse start order, then restart all
            terminate_all_children(sup);
            start_all_children(sup);
        }
        Strategy::RestForOne => {
            // Terminate the failed child and all children started after it
            let idx = sup.child_index(failed_child);
            terminate_children_from(sup, idx); // idx..end, in reverse order
            start_children_from(sup, idx);     // idx..end, in start order
        }
        Strategy::SimpleOneForOne => {
            // Same as one_for_one but for dynamic children
            restart_child(sup, failed_child);
        }
    }
}
```

### Pattern 3: Restart Limit Tracking via Sliding Window

**What:** Track restart timestamps in a VecDeque. Before each restart, check if the number of restarts in the last `max_seconds` exceeds `max_restarts`. If so, the supervisor itself terminates.

**When to use:** Every restart attempt checks the limit first.

```rust
fn check_restart_limit(sup: &mut SupervisorState) -> bool {
    let now = Instant::now();
    let window_start = now - Duration::from_secs(sup.max_seconds);

    // Remove timestamps older than the window
    while let Some(&oldest) = sup.restart_history.front() {
        if oldest < window_start {
            sup.restart_history.pop_front();
        } else {
            break;
        }
    }

    if sup.restart_history.len() >= sup.max_restarts as usize {
        // Limit exceeded -- supervisor must terminate
        false
    } else {
        sup.restart_history.push_back(now);
        true
    }
}
```

### Pattern 4: Ordered Shutdown Sequence

**What:** When a supervisor shuts down (either voluntarily or due to restart limit exceeded), it terminates children in reverse start order. Each child is given its configured shutdown timeout, then killed if it doesn't exit in time.

**When to use:** Supervisor termination, one_for_all restarts, rest_for_one restarts.

```rust
fn terminate_child(sup: &SupervisorState, child: &ChildState) {
    match child.spec.shutdown {
        Shutdown::BrutalKill => {
            // Immediately kill the child process
            kill_process(child.pid);
        }
        Shutdown::Timeout(ms) => {
            // Send shutdown exit signal
            send_exit_signal(child.pid, ExitReason::Shutdown);
            // Wait up to ms milliseconds for the child to exit
            if !wait_for_exit(child.pid, ms) {
                // Timeout: force kill
                kill_process(child.pid);
            }
        }
    }
}
```

### Pattern 5: Child Spec as Typed Struct

**What:** Child specifications are Snow structs with compile-time validation. The type checker ensures the start function matches the expected actor type and message type.

**When to use:** Every supervisor definition includes child specs.

```snow
# Proposed Snow syntax for child spec
supervisor MySupervisor do
  strategy: one_for_one
  max_restarts: 3
  max_seconds: 5

  child Worker1 do
    start: fn -> spawn(MyWorker, initial_state) end
    restart: permanent
    shutdown: 5000
  end

  child Worker2 do
    start: fn -> spawn(AnotherWorker) end
    restart: transient
    shutdown: brutal_kill
  end
end
```

### Anti-Patterns to Avoid

- **Supervisor doing application logic:** Supervisors should ONLY supervise. No business logic in the supervisor process. This is an Erlang best practice.
- **Sharing PIDs after restart:** When a child restarts, it gets a fresh PID. Code holding the old PID will send to a dead process. Use named registration for service lookup.
- **Circular supervision:** A supervisor supervising its own ancestor creates a deadlock on shutdown. Supervision trees must be acyclic.
- **Ignoring restart limits:** Without restart limits, a crash-loop child will restart infinitely. Always set reasonable limits.
- **Synchronous restart in the message handler:** Restart must not block the supervisor's receive loop. If starting a child is slow, it blocks all other exit signal processing. Phase 7 can start children synchronously (matching Erlang) but note this for future optimization.

## Claude's Discretion Recommendations

### 1. Supervisor Definition Syntax: Dedicated `supervisor` Block

**Recommendation:** Use a dedicated `supervisor` keyword block syntax, consistent with the existing `actor` keyword block.

**Rationale:**
- The `supervisor` keyword is already reserved in the lexer (TokenKind::Supervisor, SyntaxKind::SUPERVISOR_KW)
- Snow already uses dedicated keyword blocks for actors (`actor Counter(state) do ... end`)
- A dedicated block enables full compile-time validation of child specs within the block
- The pattern is familiar: `supervisor Name do ... end` parallels `actor Name do ... end`
- Block syntax allows the compiler to see all child specs at once, enabling typed supervision (CONC-07)

**Proposed syntax:**
```snow
supervisor AppSupervisor do
  strategy: one_for_one
  max_restarts: 3
  max_seconds: 5

  child WorkerPool do
    start: fn -> spawn(Worker, 0) end
    restart: permanent
    shutdown: 5000
  end

  child Logger do
    start: fn -> spawn(LoggerActor) end
    restart: permanent
    shutdown: brutal_kill
  end
end

# Usage
fn main() do
  let sup = spawn(AppSupervisor)
end
```

**For simple_one_for_one, the syntax adapts:**
```snow
supervisor WorkerPool do
  strategy: simple_one_for_one
  max_restarts: 10
  max_seconds: 60

  # Template child spec -- new children are started via start_child(sup, args)
  child_template do
    start: fn (id) -> spawn(Worker, id) end
    restart: transient
    shutdown: 5000
  end
end

# Dynamic child management
fn main() do
  let pool = spawn(WorkerPool)
  start_child(pool, 1)  # Starts Worker with id=1
  start_child(pool, 2)  # Starts Worker with id=2
end
```

### 2. Supervisor Naming Policy: Explicit Registration (Consistent with Phase 6)

**Recommendation:** Supervisors follow the same naming policy as regular actors -- explicit `register(pid, name)` calls. No auto-registration.

**Rationale:**
- Phase 6 established explicit naming via `snow_actor_register`
- Auto-registration would be an inconsistency with the existing actor model
- Erlang supervisors are optionally named too ({local, Name} or anonymous)
- Users can register supervisors by name if needed for lookup

### 3. trap_exit Mechanism: Automatic for Supervisors, Opt-in for Regular Actors

**Recommendation:** The supervisor block automatically sets `trap_exit = true` on the supervisor process at spawn time. Regular actors can opt in by calling a new `trap_exit()` builtin function.

**Rationale:**
- `Process.trap_exit` field already exists in process.rs (Phase 6, default false)
- `link::propagate_exit` already checks `proc.trap_exit` and delivers exit signals as messages when true
- Supervisors MUST trap exits to function -- making this automatic prevents footguns
- Regular actors sometimes need trap_exit for graceful shutdown (e.g., cleanup on peer crash)
- Implementation: supervisor spawn function sets `trap_exit = true` right after spawn; for regular actors, add `snow_actor_trap_exit()` extern "C" function

### 4. Child Spec Representation: Snow Struct with Compile-Time Validation

**Recommendation:** Child specs are represented as a ChildSpec struct in the runtime. The compiler parses child spec blocks within the supervisor definition and validates:
1. The start function returns `Pid<M>` for some message type M
2. The restart type is one of permanent/transient/temporary
3. The shutdown value is a positive integer or `brutal_kill`

**Runtime representation (Rust):**
```rust
pub struct ChildSpec {
    pub id: String,
    pub start_fn: *const u8,     // Function pointer to spawn function
    pub start_args: Vec<u8>,     // Serialized initial arguments
    pub restart_type: RestartType,
    pub shutdown: ShutdownType,
    pub child_type: ChildType,   // Worker or Supervisor (for shutdown ordering)
}

pub enum RestartType {
    Permanent,  // Always restart
    Transient,  // Restart on abnormal exit only
    Temporary,  // Never restart
}

pub enum ShutdownType {
    BrutalKill,
    Timeout(u64),  // milliseconds
}

pub enum ChildType {
    Worker,
    Supervisor,
}
```

**Compile-time validation:**
- The type checker validates that each child's start function is callable and returns a Pid
- The supervisor block tracks the message types of all children
- Mismatched start function signatures produce E-level compile errors
- Invalid restart/shutdown values produce E-level compile errors

### 5. Shutdown Signal Implementation

**Recommendation:** Introduce a `Shutdown` variant in `ExitReason` (the user already requested this: "Structured ExitReason sum type: Normal, Shutdown, Custom(String)"). The shutdown signal is sent to children during ordered termination. Children with `trap_exit = true` receive it as a message and can perform cleanup.

**Changes to ExitReason:**
```rust
pub enum ExitReason {
    Normal,
    Shutdown,                        // NEW: clean supervisor-initiated shutdown
    Error(String),
    Killed,
    Linked(ProcessId, Box<ExitReason>),
    Custom(String),                  // NEW: user-defined exit reasons
}
```

**Shutdown flow:**
1. Supervisor sends `exit(child_pid, Shutdown)` signal
2. If child has terminate callback, it's invoked with Shutdown reason
3. Child processes Shutdown as a clean exit (transient children do NOT restart on Shutdown)
4. If child doesn't exit within timeout, supervisor sends `exit(child_pid, Kill)` (brutal kill)

### 6. simple_one_for_one Dynamic Child Management

**Recommendation:** For simple_one_for_one, the supervisor maintains a template child spec and a dynamic list of active children. Children are added via `start_child(supervisor_pid, extra_args)` and can be removed via `terminate_child(supervisor_pid, child_pid)`.

**Implementation:**
- The supervisor holds one ChildSpec template
- `start_child` sends a message to the supervisor, which spawns the child with the template's start function + the extra args
- The supervisor links to each dynamic child
- On child exit, standard restart logic applies (based on template's restart type)
- `terminate_child` sends a message to the supervisor, which terminates the specific child

**Runtime functions:**
```rust
#[no_mangle]
pub extern "C" fn snow_supervisor_start_child(sup_pid: u64, args_ptr: *const u8, args_size: u64) -> u64
#[no_mangle]
pub extern "C" fn snow_supervisor_terminate_child(sup_pid: u64, child_pid: u64) -> u64
```

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Exit signal delivery | Custom notification system | Existing `link::propagate_exit` + `EXIT_SIGNAL_TAG` | Already works, battle-tested in Phase 6 |
| Process naming | Custom supervisor registry | Existing `ProcessRegistry` | Name-based lookup for fresh PIDs after restart |
| Coroutine management | Custom stack switching | Existing `CoroutineHandle` + corosensei | Supervisor is just another actor |
| Timer/timeout | Custom timer wheel | `std::time::Instant` + scheduler polling | Simple enough for shutdown timeouts |
| Concurrent data structures | Custom lock-free structures | Existing `parking_lot::Mutex/RwLock` | Supervisor state is single-writer (the supervisor itself) |

**Key insight:** The supervisor is architecturally just an actor that happens to trap exits and manage child lifecycles. Nearly all infrastructure already exists from Phase 6. The real work is the supervisor state machine (strategy dispatch, restart tracking, ordered shutdown) and the compiler-side syntax/validation.

## Common Pitfalls

### Pitfall 1: Restart Storm

**What goes wrong:** A child has a startup bug (e.g., fails to connect to a service). The supervisor restarts it immediately. It crashes again. This repeats until the restart limit is hit, killing the supervisor, which cascades up the tree.
**Why it happens:** No delay between restarts. The child fails fast, exhausting the restart budget in milliseconds.
**How to avoid:** Erlang does not add restart delays by default (the restart limit is the defense). Snow should match Erlang: restart immediately, rely on the sliding window limit (max_restarts in max_seconds). Document that restart limits should be tuned for the expected failure rate. An exponential backoff feature can be added in Phase 9 or later.
**Warning signs:** Supervisor terminates immediately after startup. Restart history shows all timestamps within milliseconds.

### Pitfall 2: Deadlock During Ordered Shutdown

**What goes wrong:** Supervisor sends shutdown signal to child A, waits for it to exit. Child A tries to send a message to child B (which the supervisor hasn't shut down yet). Child B sends to A. Deadlock.
**Why it happens:** Shutdown is sequential (reverse start order). Children may have dependencies that create communication cycles.
**How to avoid:** Shutdown timeout is essential. If a child doesn't exit within its timeout, force-kill it. Erlang uses 5000ms default timeout for workers and infinity for supervisor children. Snow should default to 5000ms for workers and infinity for supervisor-type children.
**Warning signs:** Supervisor hangs during shutdown. Stack trace shows child blocked on send/receive.

### Pitfall 3: Stale PID References After Restart

**What goes wrong:** Actor A holds `let worker = spawn(Worker, 0)`. Worker crashes and supervisor restarts it with a new PID. Actor A still sends to the old PID, which is dead.
**Why it happens:** PIDs are fresh on restart (Erlang style, per user decision). Code holding old PIDs doesn't know about the restart.
**How to avoid:** Use named registration for any actor that might be restarted. After restart, the supervisor re-registers the child under the same name. Actors look up by name instead of holding PIDs directly. This is standard Erlang practice.
**Warning signs:** Messages being sent to dead PIDs. `snow_actor_send` silently drops messages to nonexistent processes.

### Pitfall 4: Supervisor Not Receiving Exit Signals

**What goes wrong:** Supervisor spawns children but doesn't link to them. When children crash, the supervisor never finds out.
**Why it happens:** Forgot to call `link()` between supervisor and child.
**How to avoid:** The supervisor must automatically link to every child it starts. This should be built into the supervisor's `start_child` logic, not left to the user. The runtime's supervisor spawn function should handle this.
**Warning signs:** Children crash silently, supervisor sits idle.

### Pitfall 5: Non-Deterministic Child Start Order

**What goes wrong:** Children start in arbitrary order. Child B depends on child A being available, but B starts first and fails.
**Why it happens:** Concurrent child spawning or non-deterministic scheduling.
**How to avoid:** Per user decision, children start sequentially in declared order. The supervisor spawns child 1, waits for it to be Running/Ready, then spawns child 2, etc. If any child fails to start, remaining children are not started and the supervisor itself fails.
**Warning signs:** Intermittent startup failures that depend on timing.

### Pitfall 6: ExitReason Encoding Mismatch

**What goes wrong:** The supervisor receives an exit signal but can't decode the reason because the encoding format changed.
**Why it happens:** The ExitReason encoding in `link::encode_exit_signal` and the decoding in the supervisor don't match.
**How to avoid:** Use a single shared encode/decode module. The existing `link.rs` already has `encode_exit_signal` with format `[u64 pid, u8 reason_tag, ...data]`. The supervisor's decode logic must match this exactly. Add a `decode_exit_signal` function in `link.rs` alongside the existing `encode_exit_signal`.
**Warning signs:** Supervisor crashes with "unknown exit reason" or garbled data.

## Code Examples

### ExitReason Sum Type (Snow-level, user-facing)

```snow
# ExitReason as a Snow sum type -- users can pattern match on it
type ExitReason do
  Normal
  Shutdown
  Error(String)
  Custom(String)
end

# In a trap_exit actor, matching on exit signals:
actor Monitor() do
  receive do
    {:exit, pid, Normal} -> println("#{pid} exited normally")
    {:exit, pid, Shutdown} -> println("#{pid} shut down")
    {:exit, pid, Error(msg)} -> println("#{pid} crashed: #{msg}")
  end
end
```

### Supervisor Definition (proposed Snow syntax)

```snow
actor Counter(count :: Int) do
  receive do
    :increment -> Counter(count + 1)
    {:get, caller :: Pid<Int>} ->
      send(caller, count)
      Counter(count)
  end
end

supervisor CounterSup do
  strategy: one_for_one
  max_restarts: 3
  max_seconds: 5

  child counter1 do
    start: fn -> spawn(Counter, 0) end
    restart: permanent
    shutdown: 5000
  end
end

fn main() do
  let sup = spawn(CounterSup)
  # Counter is supervised -- if it crashes, it's restarted automatically
end
```

### Runtime Supervisor State (Rust)

```rust
/// Supervisor actor state, managed by the runtime.
pub struct SupervisorState {
    /// Supervision strategy.
    pub strategy: Strategy,
    /// Maximum restarts within the time window.
    pub max_restarts: u32,
    /// Time window in seconds for restart limit tracking.
    pub max_seconds: u64,
    /// Ordered list of child specifications.
    pub children: Vec<ChildState>,
    /// Restart history: timestamps of recent restarts.
    pub restart_history: VecDeque<Instant>,
}

pub struct ChildState {
    pub spec: ChildSpec,
    /// Current PID of the running child (None if not started or terminated).
    pub pid: Option<ProcessId>,
    /// Whether this child is currently running.
    pub running: bool,
}

pub enum Strategy {
    OneForOne,
    OneForAll,
    RestForOne,
    SimpleOneForOne,
}
```

### New Runtime ABI Functions

```rust
/// Start a supervisor with the given configuration.
/// The supervisor is spawned as an actor with trap_exit = true.
/// Returns the supervisor PID.
#[no_mangle]
pub extern "C" fn snow_supervisor_start(
    config_ptr: *const u8,  // Serialized SupervisorConfig (strategy + limits + child specs)
    config_size: u64,
) -> u64

/// Start a dynamic child under a simple_one_for_one supervisor.
/// Returns the child PID.
#[no_mangle]
pub extern "C" fn snow_supervisor_start_child(
    sup_pid: u64,
    args_ptr: *const u8,
    args_size: u64,
) -> u64

/// Terminate a specific child under a supervisor.
#[no_mangle]
pub extern "C" fn snow_supervisor_terminate_child(
    sup_pid: u64,
    child_id_ptr: *const u8,
    child_id_len: u64,
) -> u64

/// Get the count of active children under a supervisor.
#[no_mangle]
pub extern "C" fn snow_supervisor_count_children(sup_pid: u64) -> u64

/// Set trap_exit on the current process (for regular actors that want to trap).
#[no_mangle]
pub extern "C" fn snow_actor_trap_exit() -> ()

/// Send an exit signal to a process (for supervisor shutdown).
/// This is distinct from killing -- it sends a Shutdown reason.
#[no_mangle]
pub extern "C" fn snow_actor_exit(target_pid: u64, reason_tag: u8) -> ()
```

### Supervisor Receive Loop (Rust runtime, conceptual)

```rust
/// The supervisor's main receive loop. Runs as a normal actor with trap_exit.
fn supervisor_entry(state: *const u8) {
    let mut sup = decode_supervisor_state(state);

    // Start all children in order
    for child in &mut sup.children {
        let pid = start_child_process(&child.spec);
        match pid {
            Ok(pid) => {
                child.pid = Some(pid);
                child.running = true;
                // Link supervisor to child
                snow_actor_link(pid.as_u64());
            }
            Err(_) => {
                // Child failed to start -- shut down remaining and fail
                terminate_started_children(&sup);
                panic!("supervisor: child {} failed to start", child.spec.id);
            }
        }
    }

    // Main supervision loop
    loop {
        let msg = snow_actor_receive(-1);
        if is_exit_signal(msg) {
            let (child_pid, reason) = decode_exit_signal(msg);
            handle_child_exit(&mut sup, child_pid, reason);
        }
    }
}
```

## Existing Infrastructure Inventory

A detailed map of what Phase 6 provides and what Phase 7 needs to add/modify.

### Ready to Use (No Changes)

| Component | Location | What It Provides |
|-----------|----------|------------------|
| Process linking | `crates/snow-rt/src/actor/link.rs` | `link()`, `unlink()`, `propagate_exit()`, `EXIT_SIGNAL_TAG`, `encode_exit_signal()` |
| trap_exit flag | `crates/snow-rt/src/actor/process.rs:185` | `Process.trap_exit: bool` -- exists, default false |
| Named registry | `crates/snow-rt/src/actor/registry.rs` | `register()`, `whereis()`, `cleanup_process()`, global_registry singleton |
| Terminate callback | `crates/snow-rt/src/actor/process.rs:198` | `Process.terminate_callback: Option<TerminateCallback>` |
| Exit signal delivery | `crates/snow-rt/src/actor/link.rs:128` | trap_exit check: delivers exit signals as messages when true |
| Scheduler spawn | `crates/snow-rt/src/actor/scheduler.rs` | `Scheduler::spawn()`, worker loop, handle_process_exit |
| Process table | `crates/snow-rt/src/actor/scheduler.rs:67` | `ProcessTable = Arc<RwLock<FxHashMap<ProcessId, Arc<Mutex<Process>>>>>` |
| Lexer keywords | `crates/snow-common/src/token.rs:64,67` | `TokenKind::Supervisor`, `TokenKind::Trap` already reserved |
| SyntaxKind | `crates/snow-parser/src/syntax_kind.rs:57,60` | `SUPERVISOR_KW`, `TRAP_KW` mapped |
| Actor parser | `crates/snow-parser/src/parser/items.rs:841` | `parse_actor_def` as template for `parse_supervisor_def` |
| Actor type checking | `crates/snow-typeck/src/infer.rs:2709` | `infer_actor_def` as template for `infer_supervisor_def` |
| MIR actor nodes | `crates/snow-codegen/src/mir/mod.rs:233` | `ActorSpawn`, `ActorLink` patterns to extend |
| LLVM intrinsics | `crates/snow-codegen/src/codegen/intrinsics.rs` | Pattern for declaring new extern "C" functions |

### Needs Modification

| Component | Location | What to Change |
|-----------|----------|---------------|
| ExitReason | `crates/snow-rt/src/actor/process.rs:74` | Add `Shutdown` and `Custom(String)` variants |
| encode_exit_signal | `crates/snow-rt/src/actor/link.rs:62` | Handle new Shutdown/Custom variants + add `decode_exit_signal()` |
| handle_process_exit | `crates/snow-rt/src/actor/scheduler.rs:481` | Integration with supervisor restart logic |
| SyntaxKind | `crates/snow-parser/src/syntax_kind.rs` | Add SUPERVISOR_DEF, CHILD_SPEC_DEF, STRATEGY_CLAUSE nodes |
| Parser dispatch | `crates/snow-parser/src/parser/mod.rs:605` | Add SUPERVISOR_KW case |
| Item enum | `crates/snow-parser/src/ast/item.rs:71` | Add SupervisorDef variant |
| MIR intrinsics | `crates/snow-codegen/src/codegen/intrinsics.rs` | Declare supervisor runtime functions |

### Needs Creation

| Component | Purpose |
|-----------|---------|
| `crates/snow-rt/src/actor/supervisor.rs` | Core supervisor struct, strategy dispatch, restart logic, child management |
| `crates/snow-rt/src/actor/child_spec.rs` | ChildSpec, RestartType, ShutdownType structs, serialization |
| Supervisor extern "C" functions in `mod.rs` | `snow_supervisor_start`, `snow_supervisor_start_child`, `snow_supervisor_terminate_child`, `snow_actor_trap_exit`, `snow_actor_exit` |

## Plan Decomposition Guidance

The roadmap stub has 3 plans. Based on research, this decomposition maps well:

### Plan 07-01: Supervisor Runtime (Strategies, Child Specs, Restart Logic)

**Focus:** Pure Rust runtime implementation in snow-rt. No compiler changes.

**Scope:**
- `supervisor.rs`: SupervisorState, Strategy enum, restart logic, child lifecycle management
- `child_spec.rs`: ChildSpec, RestartType, ShutdownType structs
- ExitReason expansion: add Shutdown, Custom(String) variants
- `decode_exit_signal()` function in link.rs
- Supervisor entry function (the trap_exit receive loop)
- All four strategies: one_for_one, one_for_all, rest_for_one, simple_one_for_one
- Restart limit tracking (max_restarts in max_seconds sliding window)
- Ordered shutdown (reverse start order, timeout/brutal_kill per child)
- `snow_supervisor_start`, `snow_supervisor_start_child`, `snow_supervisor_terminate_child`, `snow_actor_trap_exit`, `snow_actor_exit` extern "C" functions
- Comprehensive Rust unit tests for each strategy, restart limits, shutdown sequencing

**Why first:** The runtime is the foundation. All supervision behavior must work at the Rust level before compiler integration. This follows the same pattern as Phase 6 (06-01 built the scheduler in Rust before compiler integration in 06-02/06-05).

### Plan 07-02: Compiler Integration (Supervisor Syntax, Parsing, MIR, Codegen)

**Focus:** Full compiler pipeline for supervisor blocks.

**Scope:**
- Parser: `parse_supervisor_def()` -- supervisor blocks with strategy, limits, child specs
- SyntaxKind additions: SUPERVISOR_DEF, CHILD_SPEC_DEF, etc.
- AST: SupervisorDef typed wrapper
- Type checker: `infer_supervisor_def()` -- validate child spec types
- MIR: SupervisorStart node
- MIR lowering: `lower_supervisor_def()`
- LLVM codegen: `codegen_supervisor_start()` -- emit calls to runtime functions
- Intrinsic declarations for supervisor runtime functions
- E2E test: write supervisor in Snow, compile, run

### Plan 07-03: Typed Supervision and Compile-Time Child Spec Validation

**Focus:** CONC-07 requirement -- the type checker validates child specs at compile time.

**Scope:**
- Type checker validates start functions return Pid<M> for known M
- Supervisor tracks child message types
- Mismatched child specs produce compile errors (new error codes)
- Invalid restart/shutdown values caught at compile time
- E2E tests for typed supervision (correct specs compile, wrong specs produce errors)
- Success criteria verification tests

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Manual restart loops | OTP supervisor module | Erlang/OTP since 1996 | Declarative supervision via child specs |
| Fixed restart strategies | Configurable strategy per supervisor | OTP inception | one_for_one/all/rest_for_one cover all common patterns |
| Unbounded restarts | max_restarts/max_seconds sliding window | OTP inception | Prevents infinite restart loops |
| Auto-shutdown only via restart limits | auto_shutdown flag (any_significant/all_significant) | OTP 24 (2021) | More control over when supervisors stop |
| Module-based supervisor init | Maps-based child specs (OTP 21+) | OTP 21 (2018) | Cleaner spec format, easier to construct |

**Deprecated/outdated:**
- Erlang's old tuple-based child specs `{Id, {M,F,A}, Restart, Shutdown, Type, Modules}` replaced by map-based specs in OTP 21. Snow should use the modern map/struct approach from the start.
- `auto_shutdown` feature (OTP 24) is useful but not in the user's requirements. Can be added later.

## Open Questions

1. **Supervisor-to-child communication for simple_one_for_one start_child**
   - What we know: `start_child(sup_pid, extra_args)` must send a message to the supervisor telling it to spawn a new child with the given args
   - What's unclear: The message format between the caller and the supervisor. Do we need a call/reply pattern (caller waits for the new child's PID)?
   - Recommendation: Use a send+receive pattern. Caller sends `{start_child, args, reply_to_pid}` to supervisor. Supervisor spawns child, sends back `{child_started, new_pid}`. This requires the caller to know the supervisor's Pid and do a receive for the response. This is exactly how Erlang's `supervisor:start_child/2` works internally (via gen_server:call).

2. **Process kill mechanism (brutal_kill)**
   - What we know: Need a way to forcefully terminate a process that ignores shutdown signals
   - What's unclear: The current runtime doesn't have a `kill_process(pid)` function. `handle_process_exit` is called when a process exits naturally or by crash.
   - Recommendation: Add `snow_actor_exit(target_pid, reason_tag)` that can send an untrappable kill signal. When reason is Kill, the scheduler should mark the process as Exited immediately and reclaim its coroutine, without invoking terminate callback. For Shutdown, deliver as a trappable signal (same as exit propagation with trap_exit semantics).

3. **How supervisors are spawned vs. regular actors**
   - What we know: Supervisors are actors with special behavior (trap_exit, child management)
   - What's unclear: Does `spawn(MySupervisor)` work the same as `spawn(MyActor)` from the caller's perspective? Does the supervisor's entry function need different codegen?
   - Recommendation: Yes, `spawn(MySupervisor)` returns a `Pid` like any actor. The supervisor's compiled entry function handles: (1) set trap_exit on self, (2) start children in order, (3) enter the supervision receive loop. The codegen generates this entry function from the supervisor block. From the outside, a supervisor is just an actor you can send messages to (for start_child, terminate_child).

## Sources

### Primary (HIGH confidence)
- [Erlang/OTP Supervisor Design Principles](https://www.erlang.org/doc/design_principles/sup_princ) -- Strategy descriptions, restart intensity, child specs, shutdown
- [Erlang supervisor module reference](https://www.erlang.org/doc/man/supervisor) -- Full API, child_spec() type, sup_flags() type, all field defaults
- Snow codebase analysis (direct file reads) -- process.rs, link.rs, scheduler.rs, registry.rs, mod.rs, stack.rs, parser, type checker, MIR, codegen

### Secondary (MEDIUM confidence)
- Phase 6 plan summaries (06-06-SUMMARY.md, 06-07-SUMMARY.md) -- Verified Phase 6 infrastructure readiness
- Phase 6 research (06-RESEARCH.md) -- Architecture patterns for actor runtime

### Tertiary (LOW confidence)
- None -- all findings verified against either official Erlang docs or direct codebase analysis

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- no new dependencies, building on existing infrastructure
- Architecture (supervisor runtime): HIGH -- direct adaptation of well-documented Erlang/OTP supervisor model, verified against official docs
- Architecture (compiler integration): HIGH -- follows exact same pattern as Phase 6 actor integration (parser -> typeck -> MIR -> codegen)
- Pitfalls: HIGH -- drawn from Erlang/OTP documentation and Phase 6 experience
- Typed supervision: MEDIUM -- compile-time child spec validation is novel (Erlang doesn't have it); design is clear but implementation details need validation

**Research date:** 2026-02-06
**Valid until:** 2026-03-08 (30 days -- stable domain, no external dependencies)
