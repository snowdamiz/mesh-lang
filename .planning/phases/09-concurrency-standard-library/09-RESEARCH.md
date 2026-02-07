# Phase 9: Concurrency Standard Library - Research

**Researched:** 2026-02-06
**Domain:** High-level concurrency abstractions (Service/Job) built on Snow's actor runtime primitives
**Confidence:** HIGH (based on thorough codebase analysis; all prior art exists in the Snow compiler)

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions

#### Service (GenServer) API shape
- Claude's discretion on definition style (module callbacks vs actor block extension vs other approach fitting Snow's patterns)
- Functional state management: handlers receive state, return new state (no mutable state)
- Both synchronous call (caller blocks for reply) and asynchronous cast (fire-and-forget) supported
- Generated typed helper functions: defining a Service auto-generates typed functions (e.g., Counter.increment(pid)) that wrap call/cast -- callers don't use generic Service.call() directly

#### Naming & conventions
- Snow-native names, NOT OTP names:
  - **Service** (not GenServer) for stateful server processes
  - **Job** (not Task) for async computation
- Callback names: **init**, **call**, **cast** (short, clean -- not handle_call/handle_cast)
- Claude's discretion on module structure (flat top-level vs namespace grouping)

#### Type system integration
- Claude's discretion on whether call and cast use separate message types or a single union type
- **Exhaustiveness enforced**: compiler error if a call/cast message variant has no matching handler arm
- **Per-variant reply types**: each call variant defines its own return type (e.g., GetCount returns Int, GetName returns String) -- caller gets back the exact type
- Job.await returns **Result<T, Error>** -- Ok(value) on success, Err on crash

#### Job (Task) semantics
- Claude's discretion on timeout behavior (required vs optional vs default)
- Claude's discretion on default supervision/linking behavior
- **Job.map included**: Job.map(list, fn) spawns parallel jobs per element and collects results
- Claude's discretion on crash-during-await behavior (Err result vs propagation -- should align with the Result<T, Error> return type)

### Claude's Discretion
- Service definition mechanism (module callbacks vs actor block extension)
- Call vs cast message type separation strategy
- Job timeout behavior
- Job supervision/linking defaults
- Crash-during-await semantics
- Module namespace structure (flat vs nested)

### Deferred Ideas (OUT OF SCOPE)
None -- discussion stayed within phase scope
</user_constraints>

## Summary

This phase builds two high-level concurrency abstractions -- **Service** (GenServer equivalent) and **Job** (Task equivalent) -- on top of Snow's existing actor runtime primitives (spawn, send, receive, link, supervisor). The key insight from codebase analysis is that Snow already has all the low-level building blocks: actors with typed Pid, message passing with type_tag-based dispatch, supervisors with restart strategies, bidirectional linking with exit signal propagation, and a module-qualified function resolution system (`stdlib_modules()` + `STDLIB_MODULES` + `map_builtin_name()`).

The implementation strategy is to treat Service and Job as **compiler-level abstractions that desugar to the existing actor primitives**. A Service definition compiles down to an actor with a receive loop, pattern matching on message type_tags, and reply channels. A Job compiles down to a linked actor that sends its result back to the caller. No new runtime primitives are needed -- only new compiler passes (type checking, MIR lowering) and a small number of new runtime helper functions for synchronous call/reply semantics.

The critical technical challenge is **per-variant reply types for Service.call**. Each call message variant can return a different type (e.g., `GetCount -> Int`, `GetName -> String`), and the caller must get back the exact type. This requires the compiler to track which call variant is being invoked and resolve the return type accordingly. The existing type system's `Ty::App`, `Ty::Fun`, and scheme generalization machinery can handle this, but it requires careful design of how Service definitions register their call/cast message types and reply types in the type environment.

**Primary recommendation:** Implement Service and Job as compiler desugaring passes. Service definitions are parsed as a new syntax form, type-checked to register message types and reply types, then lowered in MIR to a standard actor with a receive loop. Job functions are module-qualified calls (`Job.async`, `Job.await`, `Job.map`) that desugar to spawn+link+send patterns. Use the existing `stdlib_modules()` pattern for Job (it's a module like String/List). Service is a new item type (like actor/supervisor) because it needs custom syntax for handlers.

## Standard Stack

This phase does not introduce new external libraries. It builds entirely on the existing Snow compiler and runtime:

### Core (Existing Snow Infrastructure)

| Component | Location | Purpose | How Phase 9 Uses It |
|-----------|----------|---------|---------------------|
| Actor runtime | `snow-rt/src/actor/mod.rs` | spawn, send, receive, link, self | Service/Job desugar to these primitives |
| Supervisor runtime | `snow-rt/src/actor/supervisor.rs` | Restart strategies, child lifecycle | Service processes can be supervised |
| Type inference | `snow-typeck/src/infer.rs` | HM type inference, scheme generalization | New type rules for Service/Job |
| MIR lowering | `snow-codegen/src/mir/lower.rs` | AST -> MIR translation | New lowering for Service/Job syntax |
| LLVM codegen | `snow-codegen/src/codegen/expr.rs` | MIR -> LLVM IR | Reuses existing actor codegen |
| Exhaustiveness | `snow-typeck/src/exhaustiveness.rs` | Maranget's algorithm for pattern coverage | Enforces all call/cast variants handled |
| Module system | `stdlib_modules()` in infer.rs | Module-qualified function resolution | Job module functions |

### New Runtime Functions Needed

| Function | Signature | Purpose |
|----------|-----------|---------|
| `snow_service_call` | `(target_pid: u64, msg_ptr: *const u8, msg_size: u64, reply_buf: *mut u8) -> u64` | Synchronous call: send message, block for reply, return reply size |
| `snow_service_reply` | `(caller_pid: u64, reply_ptr: *const u8, reply_size: u64)` | Send reply from service handler back to caller |
| `snow_job_await` | `(target_pid: u64, timeout_ms: i64) -> *const u8` | Block until job completes and returns result |

## Architecture Patterns

### Pattern 1: Service as Compiler Desugaring

**What:** A `service` definition is a new top-level item that the compiler desugars into an actor with a structured receive loop. The compiler generates both the internal actor implementation and the external typed helper functions.

**When to use:** Always -- this is the core implementation strategy.

**Snow source (what the user writes):**
```elixir
service Counter do
  # Initial state
  fn init(start_val :: Int) :: Int do
    start_val
  end

  # Synchronous calls (caller blocks for reply)
  call GetCount() :: Int do |state|
    {state, state}   # {new_state, reply}
  end

  call Increment(amount :: Int) :: Int do |state|
    let new = state + amount
    {new, new}
  end

  # Asynchronous casts (fire-and-forget)
  cast Reset() do |state|
    0
  end
end
```

**What the compiler generates (conceptual desugaring):**

1. **A sum type for call messages** with embedded reply-channel type info:
   ```
   type CounterCall = GetCount | Increment(Int)
   ```

2. **A sum type for cast messages:**
   ```
   type CounterCast = Reset
   ```

3. **An internal actor function** that runs a receive loop dispatching on message type_tag:
   ```
   actor __counter_loop(state :: Int) do
     receive do
       {tag, caller_pid, payload} ->
         case tag do
           :call_get_count -> ...reply with state...
           :call_increment -> ...reply with new state...
           :cast_reset -> __counter_loop(0)
         end
     end
   end
   ```

4. **Typed helper functions** registered in the module namespace:
   ```
   Counter.start(init_arg) :: Pid<CounterMsg>   # spawn + init
   Counter.get_count(pid) :: Int                 # synchronous call
   Counter.increment(pid, amount) :: Int          # synchronous call
   Counter.reset(pid) :: ()                       # asynchronous cast
   ```

### Pattern 2: Service Message Wire Format

**What:** Messages between caller and service use a structured layout that includes a type_tag for dispatch and a caller PID for reply routing.

**Message layout for call:**
```
[u64 type_tag] [u64 caller_pid] [u8... payload]
```

**Message layout for cast:**
```
[u64 type_tag] [u8... payload]
```

**Reply layout:**
```
[u64 type_tag (reply marker)] [u8... reply_data]
```

The type_tag values are compiler-assigned constants, unique per service. The compiler generates `match` on type_tag in the receive loop body. The reply uses the existing `snow_actor_send` to send back to `caller_pid`.

For synchronous calls, the caller:
1. Sends the call message (with its own PID embedded)
2. Enters a blocking `snow_actor_receive` waiting for a reply message with a specific reply_tag
3. Returns the reply value

This is exactly the Erlang `gen_server:call/2` pattern: send `{From, Ref, Msg}`, caller blocks on `receive {Ref, Reply}`.

### Pattern 3: Job as Module-Qualified Functions

**What:** Job is implemented as a stdlib module (like String, List, Map) with functions that desugar to spawn+link+send patterns.

**Snow source:**
```elixir
# Simple async computation
let job = Job.async(fn -> expensive_computation() end)
let result = Job.await(job)  # Result<Int, String>

# Parallel map
let results = Job.map([1, 2, 3], fn x -> x * 2 end)  # [Result<Int, String>]
```

**Implementation:** `Job.async(f)` desugars to:
1. Spawn a linked actor that runs `f`
2. The actor sends its result back to the caller PID
3. Return the spawned PID (typed as `Pid<JobResult<T>>`)

`Job.await(pid)` desugars to:
1. Receive from the job's PID (blocking)
2. If normal message: unwrap as `Ok(value)`
3. If exit signal (EXIT_SIGNAL_TAG): return `Err(reason)`

### Pattern 4: Registering Service as a Module

**What:** A Service definition registers itself in `stdlib_modules()` and `STDLIB_MODULES` dynamically during type checking, so that `Counter.increment(pid, 5)` resolves via the existing `infer_field_access` -> `is_stdlib_module` -> `stdlib_modules()` path.

However, since Service names are user-defined (not hardcoded stdlib modules), this requires extending the module resolution system. The simplest approach: during type checking of a `service Counter do...end`, register "Counter" as a module name in a new `user_modules` map alongside the existing `stdlib_modules()`. The `infer_field_access` function already checks `is_stdlib_module()` -- extend it to also check user-defined service modules.

In MIR lowering, `Counter.increment(pid, 5)` would be lowered to a `Call` to a generated function `__service_counter_call_increment`, which itself emits the message send + blocking receive.

### Recommended Project Structure (within existing codebase)

```
crates/snow-parser/src/
  parser/items.rs          # Add service item parsing
  ast/item.rs              # Add ServiceDef AST node
  syntax_kind.rs           # Add SERVICE_KW, CALL_KW, CAST_KW tokens

crates/snow-typeck/src/
  infer.rs                 # Add infer_service_def(), extend module resolution
  exhaustiveness.rs        # (reuse as-is for call/cast variant coverage)

crates/snow-codegen/src/
  mir/mod.rs               # Add ServiceDef, ServiceCall, ServiceCast MIR nodes (or reuse existing)
  mir/lower.rs             # Add lower_service_def(), extend map_builtin_name
  codegen/expr.rs          # Reuse existing ActorSpawn/Send/Receive codegen
  codegen/intrinsics.rs    # Declare new runtime functions

crates/snow-rt/src/
  actor/service.rs         # NEW: snow_service_call, snow_service_reply runtime functions
  actor/mod.rs             # Export new service functions
```

### Anti-Patterns to Avoid

- **Don't add new MIR actor variants for Service:** Service should desugar to existing `ActorSpawn`, `ActorSend`, `ActorReceive` MIR nodes. Adding new MIR nodes would require changes throughout the entire codegen pipeline. The desugaring should happen at MIR lowering time.

- **Don't make Service a runtime concept:** The runtime should not know about "services." It only knows about actors, messages, and PIDs. Service is purely a compiler abstraction that generates structured actor code. The only new runtime functions are for synchronous call/reply (which are general-purpose, not Service-specific).

- **Don't hand-roll message serialization for each handler:** Use the existing message wire format (`[type_tag, data_len, data]`) with compiler-assigned type_tags. Each call/cast variant gets a unique tag. The same `snow_actor_send` / `snow_actor_receive` functions are used.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Message dispatch by type | Manual if/else chains in codegen | Compiler-generated `case` on type_tag using existing `MirExpr::Match` | Existing pattern match infrastructure handles this correctly |
| Synchronous call/reply | Custom blocking primitive | `snow_actor_send` for request + `snow_actor_receive` for reply | Reuses proven runtime primitives; only need a thin wrapper |
| Exhaustiveness checking | Custom coverage analysis for handlers | Existing `exhaustiveness.rs` Maranget algorithm | Already handles sum types with variants; feed it the service message type |
| Module-qualified access | Custom resolution for `Counter.method(pid)` | Extend existing `stdlib_modules()` + `STDLIB_MODULES` pattern | Proven approach used by String, List, Map, etc. |
| Type-safe spawn | Custom spawn logic for services | Existing `ActorSpawn` MIR node + codegen | Already handles function pointer + args serialization |
| Exit signal handling | Custom Job failure detection | Existing `EXIT_SIGNAL_TAG` + `link` + `trap_exit` | Battle-tested in supervisor implementation |

## Common Pitfalls

### Pitfall 1: Synchronous Call Deadlock

**What goes wrong:** If a Service calls itself (directly or indirectly through a cycle of calls), it deadlocks -- the receive loop is blocked waiting for the caller's reply, but the caller is the same process.

**Why it happens:** OTP has this exact problem. `gen_server:call(self(), msg)` deadlocks in Erlang too.

**How to avoid:**
- Document that self-calls are not supported (same as OTP)
- Optionally: detect self-call at runtime (check if caller_pid == self()) and return an error instead of deadlocking
- The timeout on `Job.await` and potentially on `Service.call` provides a safety net

**Warning signs:** Tests that hang indefinitely.

### Pitfall 2: Type_Tag Collision Between Services

**What goes wrong:** If two different Service definitions happen to generate the same type_tag values for their call/cast variants, messages could be dispatched to wrong handlers.

**Why it happens:** Type_tags derived from first 8 bytes of data (current approach) are not guaranteed unique across different services.

**How to avoid:**
- Assign type_tags at compile time using a deterministic scheme: hash of (service_name, variant_name) -> unique u64
- Each service's receive loop only matches its own tags, so even if there were a collision across services, messages go to the right service by PID routing. The real risk is collision WITHIN a single service, which the compiler controls.
- Use the service name as a namespace: `hash("Counter::GetCount")` vs `hash("Counter::Increment")`

**Warning signs:** Wrong handler invoked for a message.

### Pitfall 3: Reply Message Interfering with Normal Receive

**What goes wrong:** A service actor that traps exits or has other receive blocks could accidentally consume the reply message intended for a `Service.call` caller.

**Why it happens:** The mailbox is a FIFO queue. A reply message and an exit signal could arrive in any order.

**How to avoid:**
- Use a unique reply_tag per call invocation (e.g., include a monotonic counter or random nonce in the call message, and match it in the reply)
- In practice, the simplest approach: have the caller enter a dedicated receive that matches only the expected reply_tag. Since the caller is blocked on this receive, no other code runs that could consume the message.
- The runtime `snow_service_call` function should handle the send+receive atomically from the caller's perspective.

### Pitfall 4: Job.await After Job Has Already Exited

**What goes wrong:** If the Job actor finishes and sends its result before the caller calls `Job.await`, the result message sits in the caller's mailbox. If the caller has other receives that run before `Job.await`, those receives might consume the job's result message.

**Why it happens:** Messages are unstructured bytes with type_tags. If the job's result type_tag matches something another receive is expecting, it could be consumed.

**How to avoid:**
- Use a unique type_tag for job result messages (e.g., `JOB_RESULT_TAG` constant similar to `EXIT_SIGNAL_TAG`)
- The `Job.await` receive should match specifically on this tag + the job's PID
- Alternative: store the result in a runtime-side mailbox keyed by job PID, rather than using the caller's regular mailbox

### Pitfall 5: State Type Mismatch Between Init and Handlers

**What goes wrong:** The `init` function returns the initial state, and handlers receive/return state. If the types don't agree, runtime corruption occurs.

**Why it happens:** If the compiler doesn't unify the init return type with the handler state parameter type, they could diverge.

**How to avoid:**
- During `infer_service_def`, create a single type variable `state_ty` and unify:
  - `init` return type with `state_ty`
  - Each `call` handler's state parameter with `state_ty`
  - Each `call` handler's returned new-state (first element of tuple) with `state_ty`
  - Each `cast` handler's state parameter and return with `state_ty`
- This ensures all state flows through a single consistent type.

## Code Examples

### Example 1: Extending stdlib_modules() for Service

Based on the existing pattern in `snow-typeck/src/infer.rs` (lines 200-419):

```rust
// In infer_service_def(), after parsing the service definition:
// Register the service as a module in the type environment.

fn register_service_module(
    env: &mut TypeEnv,
    service_name: &str,        // e.g., "Counter"
    state_ty: Ty,
    call_variants: &[(String, Vec<Ty>, Ty)],  // (name, arg_types, reply_type)
    cast_variants: &[(String, Vec<Ty>)],       // (name, arg_types)
) {
    let pid_ty = Ty::pid(/* service message type */);

    // Register start function: Counter.start(init_arg) -> Pid<CounterMsg>
    let start_ty = Ty::fun(vec![/* init args */], pid_ty.clone());
    env.insert(
        format!("{}.start", service_name),
        Scheme::mono(start_ty),
    );

    // Register call helpers: Counter.get_count(pid) -> Int
    for (name, arg_types, reply_type) in call_variants {
        let mut params = vec![pid_ty.clone()]; // first arg is always the pid
        params.extend(arg_types.clone());
        let fn_ty = Ty::fun(params, reply_type.clone());
        env.insert(
            format!("{}.{}", service_name, to_snake_case(name)),
            Scheme::mono(fn_ty),
        );
    }

    // Register cast helpers: Counter.reset(pid) -> ()
    for (name, arg_types) in cast_variants {
        let mut params = vec![pid_ty.clone()];
        params.extend(arg_types.clone());
        let fn_ty = Ty::fun(params, Ty::Tuple(vec![]));
        env.insert(
            format!("{}.{}", service_name, to_snake_case(name)),
            Scheme::mono(fn_ty),
        );
    }
}
```

### Example 2: Extending STDLIB_MODULES for MIR Lowering

The MIR lowerer's `lower_field_access` (line 904 in lower.rs) checks `STDLIB_MODULES` for module-qualified access. For user-defined services, we need a dynamic check:

```rust
// In lower_field_access():
// After checking STDLIB_MODULES (static list), also check user-defined services.
if self.known_service_modules.contains(&base_name.as_str()) {
    let field = fa.field().map(|t| t.text().to_string()).unwrap_or_default();
    let prefixed = format!("__service_{}_{}", base_name.to_lowercase(), field);
    let ty = self.resolve_range(fa.syntax().text_range());
    return MirExpr::Var(prefixed, ty);
}
```

### Example 3: Job Module Registration (Type Checker)

Following the same pattern as List, Map, etc. in `stdlib_modules()`:

```rust
// In stdlib_modules(), add Job module:
let job_t = Ty::Con(TyCon::new("Job"));  // opaque Job handle

let mut job_mod = HashMap::new();

// Job.async(fn) -> Job<T>
// The function type is fn() -> T, Job wraps the return type
let t_var = Ty::Con(TyCon::new("T"));
job_mod.insert("async".to_string(), Scheme::poly(
    vec!["T".to_string()],
    Ty::fun(vec![Ty::fun(vec![], t_var.clone())], Ty::pid(t_var.clone())),
));

// Job.await(pid) -> Result<T, String>
job_mod.insert("await".to_string(), Scheme::poly(
    vec!["T".to_string()],
    Ty::fun(vec![Ty::pid(t_var.clone())], Ty::result(t_var.clone(), Ty::string())),
));

// Job.await_timeout(pid, timeout_ms) -> Result<T, String>
job_mod.insert("await_timeout".to_string(), Scheme::poly(
    vec!["T".to_string()],
    Ty::fun(
        vec![Ty::pid(t_var.clone()), Ty::int()],
        Ty::result(t_var.clone(), Ty::string()),
    ),
));

// Job.map(list, fn) -> List<Result<T, String>>
job_mod.insert("map".to_string(), Scheme::poly(
    vec!["T".to_string()],
    Ty::fun(
        vec![Ty::list_untyped(), Ty::fun(vec![Ty::int()], t_var.clone())],
        Ty::list_untyped(),  // List<Result<T, String>>
    ),
));

modules.insert("Job".to_string(), job_mod);
```

### Example 4: Runtime snow_service_call Implementation

```rust
// In snow-rt/src/actor/service.rs

/// Synchronous call to a service actor.
///
/// Sends a call message to the target and blocks until a reply arrives.
/// The call message embeds the caller's PID so the service knows where
/// to send the reply.
///
/// Message layout sent: [u64 type_tag] [u64 caller_pid] [u8... payload]
/// Reply layout received: [u64 reply_tag] [u8... reply_data]
///
/// Returns a pointer to the reply data in the caller's heap, or null on timeout.
#[no_mangle]
pub extern "C" fn snow_service_call(
    target_pid: u64,
    msg_tag: u64,
    payload_ptr: *const u8,
    payload_size: u64,
    timeout_ms: i64,
) -> *const u8 {
    let caller_pid = match stack::get_current_pid() {
        Some(pid) => pid.as_u64(),
        None => return std::ptr::null(),
    };

    // Build call message: [tag][caller_pid][payload]
    let mut msg_data = Vec::with_capacity(16 + payload_size as usize);
    msg_data.extend_from_slice(&msg_tag.to_le_bytes());
    msg_data.extend_from_slice(&caller_pid.to_le_bytes());
    if !payload_ptr.is_null() && payload_size > 0 {
        let payload = unsafe {
            std::slice::from_raw_parts(payload_ptr, payload_size as usize)
        };
        msg_data.extend_from_slice(payload);
    }

    // Send the call message
    snow_actor_send(target_pid, msg_data.as_ptr(), msg_data.len() as u64);

    // Block for reply (reuse existing receive mechanism)
    snow_actor_receive(timeout_ms)
}

/// Reply to a service call from within a handler.
///
/// Sends the reply data back to the caller PID.
#[no_mangle]
pub extern "C" fn snow_service_reply(
    caller_pid: u64,
    reply_ptr: *const u8,
    reply_size: u64,
) {
    snow_actor_send(caller_pid, reply_ptr, reply_size);
}
```

### Example 5: Service MIR Lowering (Conceptual)

A `service Counter` definition would lower to:

```
MirFunction {
    name: "__service_counter_loop",
    params: [("state", MirType::Int)],
    return_type: MirType::Unit,
    body: MirExpr::ActorReceive {
        arms: [
            // Call: GetCount
            MirMatchArm {
                pattern: MirPattern::Constructor("CallGetCount", [
                    MirPattern::Var("caller_pid", MirType::Int),
                ]),
                body: MirExpr::Block([
                    // reply = state
                    MirExpr::Call { func: "snow_service_reply", args: [caller_pid, state] },
                    // tail-call with same state
                    MirExpr::Call { func: "__service_counter_loop", args: [state] },
                ]),
            },
            // Call: Increment(amount)
            MirMatchArm {
                pattern: MirPattern::Constructor("CallIncrement", [
                    MirPattern::Var("caller_pid", MirType::Int),
                    MirPattern::Var("amount", MirType::Int),
                ]),
                body: MirExpr::Block([
                    MirExpr::Let { name: "new", value: state + amount },
                    MirExpr::Call { func: "snow_service_reply", args: [caller_pid, new] },
                    MirExpr::Call { func: "__service_counter_loop", args: [new] },
                ]),
            },
            // Cast: Reset
            MirMatchArm {
                pattern: MirPattern::Constructor("CastReset", []),
                body: MirExpr::Call { func: "__service_counter_loop", args: [0] },
            },
        ],
        timeout_ms: None,
        ty: MirType::Unit,
    },
}

// Plus generated helper functions:
MirFunction {
    name: "__service_counter_start",  // Counter.start(init_val)
    params: [("init_val", MirType::Int)],
    return_type: MirType::Int,  // Pid
    body: MirExpr::Block([
        // Call init to get initial state
        MirExpr::Let { name: "state", value: init_val },
        // Spawn the loop actor with initial state
        MirExpr::ActorSpawn { func: "__service_counter_loop", args: [state] },
    ]),
}

MirFunction {
    name: "__service_counter_get_count",  // Counter.get_count(pid)
    params: [("pid", MirType::Int)],
    return_type: MirType::Int,
    body: MirExpr::Call {
        func: "snow_service_call",
        args: [pid, GET_COUNT_TAG, null, 0, -1],
    },
}
```

## Discretion Recommendations

### 1. Service Definition Mechanism: Module-like Block Syntax (RECOMMENDED)

**Recommendation:** Use a new `service` keyword block (similar to how `actor` and `supervisor` are already block items).

**Rationale:**
- Snow already has `actor Name(params) do...end` and `supervisor Name do...end` as top-level items
- `service Name do...end` follows the same pattern naturally
- The parser already handles `actor` and `supervisor` as items in `parser/items.rs` -- adding `service` is straightforward
- A module-callback approach (separate `defmodule` + `use Service`) would require new module system infrastructure that doesn't exist yet

### 2. Call vs Cast Message Type Separation: Separate Internal Sum Types (RECOMMENDED)

**Recommendation:** Generate two internal sum types per service -- one for call messages and one for cast messages -- but present them to the user as a single unified namespace.

**Rationale:**
- Call variants need reply types; cast variants don't. Separating them internally simplifies the type machinery.
- The user doesn't see these sum types directly -- they write `call GetCount() :: Int` and `cast Reset()`.
- Exhaustiveness checking runs separately on call handlers and cast handlers.
- The message wire format uses different layouts (call includes caller_pid, cast doesn't), so separation is natural.

### 3. Job Timeout Behavior: Optional with Default (RECOMMENDED)

**Recommendation:** `Job.await(pid)` blocks indefinitely. `Job.await_timeout(pid, ms)` has an explicit timeout. No magic default timeout.

**Rationale:**
- Matches Elixir's `Task.await/2` which has a 5000ms default, but Snow should be explicit
- `Job.await(pid)` = `Job.await_timeout(pid, :infinity)` conceptually
- Users who want timeouts explicitly use `Job.await_timeout`
- Keeps the simple case simple

### 4. Job Supervision/Linking: Linked by Default (RECOMMENDED)

**Recommendation:** `Job.async(fn)` creates a linked job by default. If the job crashes, the caller receives an exit signal. `Job.await` converts exit signals to `Err(reason)`.

**Rationale:**
- Matches Elixir's `Task.async/1` which links by default
- The existing `snow_actor_link` + `EXIT_SIGNAL_TAG` infrastructure handles this perfectly
- The caller already needs to handle `Result<T, Error>` from `Job.await`, so crash-as-Err is natural
- If the caller crashes before awaiting, the link ensures the job is cleaned up

### 5. Crash-During-Await: Return Err (RECOMMENDED)

**Recommendation:** If a job crashes, `Job.await` returns `Err("process crashed: <reason>")`. The exit signal is converted to an error value rather than propagated.

**Rationale:**
- The return type is already `Result<T, Error>` -- using `Err` is the natural path
- The caller explicitly asked for the result by calling `await` -- they should get a value, not an unexpected crash
- Implementation: the `Job.await` receive loop checks for `EXIT_SIGNAL_TAG` messages from the job PID and converts them to `Err`
- The caller still needs `trap_exit = true` on itself for this to work. The `Job.await` runtime function should set this.

### 6. Module Namespace Structure: Flat with Service Name (RECOMMENDED)

**Recommendation:** Service functions are accessed as `ServiceName.method(pid, args)`. Job functions are accessed as `Job.async(fn)`, `Job.await(pid)`. No nested namespaces.

**Rationale:**
- Matches the existing pattern: `String.length(s)`, `List.map(l, f)`, `Map.get(m, k)`
- Users define `service Counter do...end` and call `Counter.start(0)`, `Counter.increment(pid, 5)`
- Job is a built-in module alongside String, IO, List, etc.

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| No high-level abstractions | Phase 9 adds Service/Job | This phase | Users get ergonomic patterns instead of raw actor primitives |
| Raw `actor` + `receive` | `service` with typed handlers | This phase | Compile-time safety for message handling |
| Manual spawn+link+receive for async work | `Job.async` + `Job.await` | This phase | Simple async pattern with Result-based error handling |

**Current state of Snow concurrency (before Phase 9):**
- Actors: `actor name(params) do receive do ... end end`
- Spawn: `spawn(actor_name)`
- Send/Receive: `send(pid, msg)` / `receive do pattern -> body end`
- Link: `link(pid)`
- Supervisor: `supervisor Name do strategy: ... child name do ... end end`
- All fully type-checked with `Pid<MessageType>`

## Key Implementation Insights from Codebase Analysis

### 1. Actor Type Registration Pattern (HIGH confidence)

From `infer_actor_def` (infer.rs:2998-3071): actors register as functions `actor_name :: (StateTypes...) -> Pid<M>` where M is inferred from receive patterns. Service should follow the same pattern but register multiple functions (start + each call/cast helper) rather than just one.

### 2. Module-Qualified Resolution Chain (HIGH confidence)

The full chain for `String.length(s)` is:
1. **Parser:** `FieldAccess` node with base=`String`, field=`length`
2. **Type checker:** `infer_field_access` -> `is_stdlib_module("String")` -> `stdlib_modules()["String"]["length"]` -> type scheme
3. **MIR lowerer:** `lower_field_access` -> `STDLIB_MODULES.contains("String")` -> `format!("{}_{}", "string", "length")` -> `map_builtin_name("string_length")` -> `"snow_string_length"`
4. **Codegen:** `MirExpr::Var("snow_string_length", ...)` -> looks up LLVM function by name

For Service, this chain needs to be extended at steps 2 and 3 with user-defined service names.

### 3. Message Wire Format (HIGH confidence)

From `snow_actor_send` (mod.rs:178-215) and `copy_msg_to_actor_heap` (mod.rs:297-337):
- Messages are raw bytes with a type_tag (first 8 bytes currently used as tag)
- Layout in actor heap: `[u64 type_tag][u64 data_len][u8... data]`
- The codegen stores values as i64 on the stack and passes ptr+size to send

For Service call messages, we need to prepend the caller_pid to the payload. This means the message layout becomes: `[u64 type_tag][u64 data_len][u64 caller_pid][u8... actual_payload]`.

### 4. Exhaustiveness Infrastructure (HIGH confidence)

From `exhaustiveness.rs`: the Maranget algorithm works on `Pat` (abstract patterns) and `TypeInfo` (which provides constructor signatures). To enforce exhaustive Service handlers, the compiler needs to:
1. Create a `TypeInfo` for the service's call message type with all call variant names as constructors
2. Build a `PatternMatrix` from the user's `call` handler definitions
3. Run `is_useful(matrix, [Wildcard])` -- if useful, a variant is missing -> error

This exactly matches how `case` exhaustiveness works on sum types.

### 5. Tail-Call Actor Loop (HIGH confidence)

From `infer_actor_def` (infer.rs:3017-3019): actors use self-recursive calls for state transitions (`counter(state + n)`). The service receive loop should use the same pattern: after handling a message, tail-call itself with the new state. The compiler already binds the actor name as a self-recursive function.

### 6. No New Runtime Primitives Strictly Needed (MEDIUM confidence)

Technically, `snow_service_call` and `snow_service_reply` can be implemented entirely using `snow_actor_send` and `snow_actor_receive`. The dedicated runtime functions are an optimization and clarity improvement, not a necessity. The compiler could generate inline send+receive sequences. However, dedicated runtime functions:
- Reduce generated code size
- Provide a single place for timeout logic
- Make debugging/tracing easier

## Open Questions

1. **Polymorphic Service State Type**
   - What we know: The state type is inferred from `init` and unified across all handlers.
   - What's unclear: Can the state type be generic? E.g., `service Cache<K, V>` with `Map<K, V>` state. The existing type system supports generics, but the Service definition syntax and registration would need to handle type parameters.
   - Recommendation: Start with monomorphic state. If needed, add generic services in a follow-up.

2. **Service Under Supervision**
   - What we know: Supervisors manage children via `child` blocks with `start: fn -> spawn(actor)`. Services would need the same pattern.
   - What's unclear: Should there be special syntax for supervised services, or should users manually create a supervisor with `start: fn -> Counter.start(0) end`?
   - Recommendation: No special syntax. Users create supervisors the normal way. `Counter.start(init_val)` returns a Pid that can be used in a supervisor child spec.

3. **Multiple Concurrent Calls to Same Service**
   - What we know: The service processes one message at a time (single receive loop). Multiple callers can send call messages, and they'll be processed FIFO.
   - What's unclear: If caller A and caller B both call the same service, does the reply routing work correctly? Yes -- each call message includes the caller_pid, so replies go to the right caller.
   - Recommendation: This works correctly by design. Document that calls are serialized (one at a time).

4. **Job.map Collecting Results**
   - What we know: `Job.map(list, fn)` spawns N jobs and collects results.
   - What's unclear: How to collect results from N jobs when each sends a separate message. Need to track which job returned which result and maintain ordering.
   - Recommendation: The runtime function `snow_job_map` should:
     1. Spawn N linked actors, each sending their result with their index
     2. Receive N result messages, storing them in order
     3. Return a list of `Result<T, Error>` values

## Sources

### Primary (HIGH confidence)
- **Snow codebase direct analysis:**
  - `crates/snow-rt/src/actor/mod.rs` -- Full runtime ABI (spawn, send, receive, link, supervisor)
  - `crates/snow-typeck/src/infer.rs` -- Type inference for actors, supervisors, modules, sum types
  - `crates/snow-codegen/src/mir/lower.rs` -- MIR lowering for actors, module-qualified access, builtin name mapping
  - `crates/snow-codegen/src/mir/mod.rs` -- MIR types, expressions, actor/supervisor MIR nodes
  - `crates/snow-codegen/src/codegen/expr.rs` -- LLVM codegen for actor spawn, send, receive
  - `crates/snow-codegen/src/codegen/intrinsics.rs` -- Runtime function declarations
  - `crates/snow-typeck/src/exhaustiveness.rs` -- Maranget pattern exhaustiveness checking
  - `crates/snow-parser/src/parser/items.rs` -- Item parsing (actor, supervisor, fn, struct)
  - `crates/snow-parser/src/ast/item.rs` -- AST node definitions
  - `crates/snow-rt/src/actor/supervisor.rs` -- Supervisor state management
  - `tests/e2e/` -- Working Snow programs demonstrating current syntax

### Secondary (MEDIUM confidence)
- OTP/Erlang GenServer and Task design patterns (conceptual, from training data)
- BEAM VM actor model patterns (conceptual, from training data)

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- based on direct codebase analysis, all infrastructure exists
- Architecture patterns: HIGH -- desugaring approach follows established patterns in the codebase
- Pitfalls: MEDIUM -- based on OTP experience and codebase analysis, but some edge cases in message routing need validation during implementation
- Discretion recommendations: MEDIUM -- informed choices but alternatives exist
- Code examples: MEDIUM -- conceptual but closely follow existing codebase patterns

**Research date:** 2026-02-06
**Valid until:** No expiry (internal codebase research, not dependent on external library versions)
