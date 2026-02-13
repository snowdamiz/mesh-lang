# Phase 67: Remote Spawn & LLVM Integration - Research

**Researched:** 2026-02-12
**Domain:** Remote actor spawning via distribution protocol, function name registry for cross-binary compatibility, and wiring the Node module into the LLVM codegen pipeline
**Confidence:** HIGH

## Summary

Phase 67 delivers three concrete capabilities: (1) `Node.spawn(node, function, args)` spawns an actor on a remote node and returns a usable local PID; (2) `Node.spawn_link(node, function, args)` spawns-and-links so remote actor crashes propagate back to the caller; (3) remote spawn uses function _names_ (not pointers) so that differently-compiled binaries can spawn each other's functions.

The phase has a dual nature that explains its title. The "Remote Spawn" half is a runtime feature -- new `DIST_SPAWN` / `DIST_SPAWN_LINK` wire messages, a function name registry in `snow-rt`, and `snow_node_spawn` / `snow_node_spawn_link` extern "C" APIs. The "LLVM Integration" half is a compiler feature -- the `Node` module does not exist anywhere in the codegen pipeline yet. The runtime has `snow_node_start`, `snow_node_connect`, `snow_node_self`, `snow_node_list`, and `snow_node_monitor` as extern "C" functions, but none of them are declared as LLVM intrinsics or lowered in the MIR. Phase 67 must wire ALL of these into the compiler (intrinsic declarations, MIR lowering, and codegen) alongside the new spawn/spawn_link functions.

The critical architectural challenge is the function name registry (EXEC-03). Currently, `snow_actor_spawn` takes a raw `fn_ptr` (memory address) which is meaningless across processes. For remote spawn, the caller sends a function name string over the wire; the remote node looks it up in a registry mapping names to function pointers. This registry must be populated at program startup. Two approaches are viable: (A) codegen emits a `snow_register_function("name", fn_ptr)` call for every top-level function during `main` initialization, or (B) codegen emits a static array of `{name, fn_ptr}` pairs and a single `snow_register_all_functions(array, count)` call. Option B is cleaner (single registration call, no N startup calls).

**Primary recommendation:** Structure as three plans: (1) Function name registry -- build the `FxHashMap<String, *const u8>` registry in `snow-rt`, add `snow_register_function` and `snow_lookup_function` extern "C" APIs, emit registration calls from codegen for all top-level and actor functions; (2) Remote spawn runtime -- add `DIST_SPAWN` / `DIST_SPAWN_LINK` wire messages, `snow_node_spawn` / `snow_node_spawn_link` extern "C" APIs, and reader-loop handling that looks up the function name, spawns locally, and replies with the new PID; (3) Node module LLVM integration -- add "Node" to STDLIB_MODULES, declare all `snow_node_*` intrinsics, lower `Node.spawn`, `Node.spawn_link`, `Node.start`, `Node.connect`, `Node.self`, `Node.list`, `Node.monitor` calls from MIR to LLVM IR.

## Standard Stack

### Core (already in Cargo.toml -- zero new dependencies)
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| parking_lot | 0.12 | RwLock for function name registry | Already used throughout codebase |
| rustc-hash | 2 | FxHashMap for function registry | Already used in node state and scheduler |

### Supporting (from existing crate modules)
| Module | Purpose | When to Use |
|--------|---------|-------------|
| `dist::node` | `NodeState`, `NodeSession`, `write_msg`, `read_dist_msg`, session lookup | Sending/receiving DIST_SPAWN wire messages |
| `actor::mod` | `snow_actor_spawn`, `global_scheduler()`, `local_send()` | Spawning actor locally on the remote side |
| `actor::process` | `ProcessId`, `ProcessId::from_remote()` | Constructing remote PIDs for spawn reply |
| `actor::link` | `link()` function for spawn_link | Establishing bidirectional link after remote spawn |
| `actor::scheduler` | `Scheduler::spawn()` | Actually spawning the actor process |
| `codegen::intrinsics` | `declare_intrinsics()` | Declaring snow_node_* functions in LLVM module |
| `mir::lower` | `STDLIB_MODULES`, `lower_field_access_call` | Lowering `Node.spawn()` calls to MIR |
| `mir::mod` | `MirExpr`, `MirType` | Representing remote spawn in MIR (or as stdlib call) |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| FxHashMap<String, *const u8> registry | Static ELF symbol table lookup (dlsym) | dlsym is OS-dependent, slower, and doesn't work for LLVM JIT-compiled functions. FxHashMap is portable and O(1). |
| Per-function registration calls at startup | Static array + single bulk registration call | Static array is cleaner but requires codegen to emit array construction. Per-function calls are simpler but add N function calls at startup. Recommend static array for clean startup. |
| New MIR node (MirExpr::RemoteSpawn) | Reuse existing stdlib call lowering pattern | New MIR node adds complexity to every MIR pass (free vars, TCE, etc.). Stdlib call pattern reuses existing infrastructure and is how all other Node/Process calls work. Recommend stdlib call. |
| Request-reply spawn protocol (synchronous) | Fire-and-forget spawn (asynchronous PID fabrication) | Request-reply is correct because the caller needs the actual PID. Fire-and-forget would require fabricating a PID before the remote side allocates it, risking collisions. Request-reply has latency cost but is correct. |
| Fabricate remote PID locally | Remote node returns actual PID in spawn reply | Fabricating locally requires synchronizing PID counters across nodes (complex, fragile). Having the remote node allocate the PID and return it is how Erlang works. Adds one network round-trip but is correct and simple. |

**Installation:** No new dependencies. All work is within existing `snow-rt/src/dist/`, `snow-rt/src/actor/`, `snow-codegen/src/codegen/`, and `snow-codegen/src/mir/`.

## Architecture Patterns

### Recommended Project Structure
```
crates/snow-rt/src/
├── dist/
│   ├── mod.rs           # MODIFIED: re-export function registry
│   ├── node.rs          # MODIFIED: DIST_SPAWN/DIST_SPAWN_LINK wire messages,
│   │                    #           snow_node_spawn/snow_node_spawn_link extern "C",
│   │                    #           reader loop handlers for spawn request/reply
│   └── registry.rs      # NEW: function name registry (FxHashMap<String, fn_ptr>)
│                        #   OR inline in node.rs if small enough
├── actor/
│   ├── mod.rs           # MODIFIED: snow_register_function extern "C",
│   │                    #           snow_lookup_function extern "C"
│   └── ...              # (no other changes)
└── lib.rs               # MODIFIED: re-export new extern "C" functions

crates/snow-codegen/src/
├── codegen/
│   ├── intrinsics.rs    # MODIFIED: declare snow_node_*, snow_register_function,
│   │                    #           snow_lookup_function
│   ├── expr.rs          # MODIFIED: codegen for Node.spawn/spawn_link calls
│   └── mod.rs           # MODIFIED: emit function registration calls in main wrapper
├── mir/
│   └── lower.rs         # MODIFIED: add "Node" to STDLIB_MODULES,
│                        #           lower Node.spawn/spawn_link/start/connect/self/list/monitor
└── ...
```

### Pattern 1: Function Name Registry

**What:** A global `OnceLock<RwLock<FxHashMap<String, *const u8>>>` mapping Snow function names to their in-process function pointers. Populated at program startup by codegen-emitted registration calls. Queried by the remote spawn handler to resolve function names received over the wire.

**When to use:** Every Snow binary registers its top-level functions at startup. Remote spawn requests look up function names in this registry.

**Rationale:** Raw function pointers are process-local memory addresses. When node A tells node B to "spawn function X", the pointer from A's address space is meaningless in B. The function name is stable across compilations (assuming the same source code). The registry bridges names to pointers.

**Registry scope:** Only top-level named functions and actor entry functions need registration. Closures, lifted lambda functions, and internal compiler-generated functions do NOT need registration because they cannot be meaningfully spawned remotely (they capture environment pointers that are address-space-local).

```rust
// Source: New in actor/mod.rs or dist/registry.rs

use std::sync::OnceLock;
use parking_lot::RwLock;
use rustc_hash::FxHashMap;

/// Global function name registry for remote spawn.
///
/// Maps Snow function names (as they appear in source code) to their
/// in-process function pointers. Populated at startup by codegen-emitted
/// registration calls.
static FUNCTION_REGISTRY: OnceLock<RwLock<FxHashMap<String, *const u8>>> = OnceLock::new();

fn function_registry() -> &'static RwLock<FxHashMap<String, *const u8>> {
    FUNCTION_REGISTRY.get_or_init(|| RwLock::new(FxHashMap::default()))
}

/// Register a named function for remote spawn.
///
/// Called by codegen-emitted initialization code for each top-level function.
/// The name should match the Snow source function name (e.g., "worker", "echo").
///
/// # Safety
/// fn_ptr must be a valid function pointer that follows the Snow actor ABI.
#[no_mangle]
pub extern "C" fn snow_register_function(
    name_ptr: *const u8,
    name_len: u64,
    fn_ptr: *const u8,
) {
    let name = unsafe {
        std::str::from_utf8_unchecked(
            std::slice::from_raw_parts(name_ptr, name_len as usize)
        ).to_string()
    };
    function_registry().write().insert(name, fn_ptr);
}

/// Look up a function pointer by name.
///
/// Returns the function pointer if found, null otherwise.
/// Used internally by the remote spawn handler.
pub(crate) fn lookup_function(name: &str) -> Option<*const u8> {
    function_registry().read().get(name).copied()
}
```

**Function naming convention:** Use the MIR function name (e.g., `"worker"`, `"echo"`, `"counter_Int"` for monomorphized generics). This is the same name used in the LLVM module's function table. Actor entry functions follow the convention `"actor_name_start"` (the generated start function that wraps the actor loop).

### Pattern 2: Remote Spawn Wire Protocol (Request-Reply)

**What:** A two-message protocol: the spawning node sends a `DIST_SPAWN` request containing the function name and serialized arguments; the remote node spawns the actor, and replies with a `DIST_SPAWN_REPLY` containing the new PID. The spawning node blocks (yields the coroutine) until the reply arrives.

**When to use:** `Node.spawn(node, function, args)` and `Node.spawn_link(node, function, args)`.

**Rationale:** The caller needs a usable PID immediately after `Node.spawn` returns. The remote node must allocate the PID (because PIDs embed node_id and creation which only the remote node knows for its own processes). Therefore, a synchronous request-reply is required. The calling actor yields (like `receive` does) while waiting for the reply, so other actors continue running.

```
DIST_SPAWN wire format:
  [DIST_SPAWN tag: 0x19]
  [u64 request_id LE]           -- correlates request to reply
  [u64 requester_pid LE]        -- who to send the reply to
  [u8 link_flag]                -- 0=spawn, 1=spawn_link
  [u16 fn_name_len LE]
  [fn_name bytes]
  [u16 args_count LE]
  [serialized args bytes]       -- STF-encoded argument values

DIST_SPAWN_REPLY wire format:
  [DIST_SPAWN_REPLY tag: 0x1A]
  [u64 request_id LE]           -- correlates to the request
  [u8 status]                   -- 0=ok, 1=error (function not found)
  [u64 spawned_pid LE]          -- the new PID (only meaningful if status=0)
```

**Tag allocation:** Current tags use 0x10-0x18 and 0xF0-0xF1. Next available: 0x19 for DIST_SPAWN, 0x1A for DIST_SPAWN_REPLY.

### Pattern 3: Remote Spawn Request Handling (Remote Side)

**What:** When the reader loop receives `DIST_SPAWN`, it: (1) extracts the function name, (2) looks it up in the function registry, (3) if found, deserializes the arguments and calls `snow_actor_spawn` locally, (4) if `link_flag=1`, establishes a bidirectional link between the requester PID and the new PID (local side + DIST_LINK back), (5) sends `DIST_SPAWN_REPLY` with the new PID. If the function is not found, sends a reply with status=error.

**When to use:** In the reader_loop_session match arm for `DIST_SPAWN`.

**Rationale:** The remote node is the authority for the spawned process. It allocates the PID from its own counter, registers the process in its own process table, and runs it on its own scheduler. The spawning node only gets a PID reference to the remote process.

```rust
DIST_SPAWN => {
    // Wire: [tag][u64 req_id][u64 requester_pid][u8 link_flag]
    //       [u16 fn_name_len][fn_name][u16 args_count][args_data]
    if msg.len() >= 20 {
        let req_id = u64::from_le_bytes(msg[1..9].try_into().unwrap());
        let requester_pid = ProcessId(u64::from_le_bytes(msg[9..17].try_into().unwrap()));
        let link_flag = msg[17];
        let fn_name_len = u16::from_le_bytes(msg[18..20].try_into().unwrap()) as usize;

        if msg.len() >= 20 + fn_name_len {
            let fn_name = std::str::from_utf8(&msg[20..20 + fn_name_len]).unwrap_or("");
            let args_data = &msg[20 + fn_name_len..];

            match lookup_function(fn_name) {
                Some(fn_ptr) => {
                    // Spawn the actor locally.
                    let pid = sched.spawn(fn_ptr, args_data.as_ptr(), args_data.len() as u64, 1);

                    // If spawn_link, establish bidirectional link.
                    if link_flag == 1 {
                        // Add requester_pid to new process's links
                        if let Some(proc_arc) = sched.get_process(pid) {
                            proc_arc.lock().links.insert(requester_pid);
                        }
                        // Send DIST_LINK back so the requester's node records the reverse link
                        send_dist_link(pid, requester_pid, &session);
                    }

                    // Construct the remote-visible PID.
                    // The spawned process has a local PID (node_id=0).
                    // The requester needs a PID with OUR node_id and creation.
                    let remote_pid = ProcessId::from_remote(
                        session.node_id, // OUR node_id as seen by THEM
                        node_state().map(|s| s.creation()).unwrap_or(0),
                        pid.local_id(),
                    );

                    // Send DIST_SPAWN_REPLY.
                    send_spawn_reply(&session, req_id, 0, remote_pid.as_u64());
                }
                None => {
                    send_spawn_reply(&session, req_id, 1, 0);
                }
            }
        }
    }
}
```

**Important PID encoding note:** The spawned process gets a local PID on the remote node (node_id=0). But the requester needs a PID with the remote node's node_id and creation embedded, so that messages sent to this PID route correctly through the distribution layer. The remote node must construct this "as seen by the requester" PID in the reply. This means the reply includes the local_id of the spawned process, and the requester reconstructs the full PID using its own knowledge of the remote node's node_id and creation (from the session).

**Correction:** Actually, the PID encoding is simpler. The remote node sends back its own local PID (node_id=0 form). The requesting node, upon receiving the reply, constructs the proper remote PID using the session's `node_id` and `creation`. This keeps the wire format clean (just the local_id) and the PID construction on the side that knows the mapping.

### Pattern 4: Remote Spawn Caller Side (Blocking Wait)

**What:** The calling actor sends `DIST_SPAWN`, then enters a waiting state (similar to `receive`) until a `DIST_SPAWN_REPLY` with the matching request_id arrives. The reply is delivered as a special message to the caller's mailbox with a reserved type_tag.

**When to use:** In the `snow_node_spawn` / `snow_node_spawn_link` extern "C" functions.

**Rationale:** The caller must block until the reply arrives because `Node.spawn()` must return a PID. Using the existing mailbox + yield mechanism ensures other actors continue running during the wait.

```rust
/// Special type_tag for spawn reply messages.
pub(crate) const SPAWN_REPLY_TAG: u64 = u64::MAX - 4;

#[no_mangle]
pub extern "C" fn snow_node_spawn(
    node_ptr: *const u8,
    node_len: u64,
    fn_name_ptr: *const u8,
    fn_name_len: u64,
    args_ptr: *const u8,
    args_size: u64,
    link_flag: u8,
) -> u64 {
    // 1. Get current PID and validate node state.
    let my_pid = stack::get_current_pid().unwrap_or(ProcessId(u64::MAX));
    let state = match node_state() { Some(s) => s, None => return 0 };

    // 2. Look up session for target node.
    let node_name = unsafe { std::str::from_utf8_unchecked(...) };
    let session = match state.sessions.read().get(node_name) {
        Some(s) => Arc::clone(s),
        None => return 0, // Not connected
    };

    // 3. Generate request_id, build DIST_SPAWN message, send.
    let req_id = generate_request_id();
    let mut payload = build_spawn_payload(req_id, my_pid, link_flag, fn_name, args);
    let mut stream = session.stream.lock().unwrap();
    let _ = write_msg(&mut *stream, &payload);
    drop(stream);

    // 4. Wait for DIST_SPAWN_REPLY in mailbox.
    // The reader thread will deliver a message with SPAWN_REPLY_TAG
    // containing [u64 req_id][u8 status][u64 pid].
    loop {
        let msg = snow_actor_receive(-1); // block indefinitely
        // Check if it's a spawn reply for our req_id
        if is_spawn_reply(msg, req_id) {
            let (status, remote_local_id) = decode_spawn_reply(msg);
            if status == 0 {
                // Construct the remote PID.
                let remote_pid = ProcessId::from_remote(
                    session.node_id,
                    session.creation,
                    remote_local_id,
                );
                // If spawn_link, add remote_pid to our links.
                if link_flag == 1 {
                    if let Some(my_proc) = sched.get_process(my_pid) {
                        my_proc.lock().links.insert(remote_pid);
                    }
                }
                return remote_pid.as_u64();
            } else {
                return 0; // Function not found
            }
        }
        // If not our reply, put message back (or handle normally)
    }
}
```

**Blocking concern:** The caller yields to the scheduler while waiting, so other actors run. However, the caller's mailbox will accumulate non-reply messages during the wait. Two options: (A) Selective receive -- scan the mailbox for the spawn reply, skip other messages. This matches Erlang's selective receive semantics. (B) Use a separate reply channel (e.g., a per-request condvar). Option A is simpler and consistent with the existing receive mechanism.

**Recommendation:** Use a simpler approach -- the reader thread delivers the DIST_SPAWN_REPLY as a message with SPAWN_REPLY_TAG to the requester's mailbox. The `snow_node_spawn` function enters a receive-like loop that checks for SPAWN_REPLY_TAG with the matching request_id. Non-matching messages remain in the mailbox. This reuses existing infrastructure (mailbox, yield, wake) without adding new synchronization primitives.

### Pattern 5: Node Module LLVM Integration

**What:** Wire the `Node` module into the compiler pipeline so that Snow source code like `Node.start(...)`, `Node.spawn(...)` etc. compiles to LLVM IR calls to the corresponding `snow_node_*` runtime functions.

**When to use:** Whenever a Snow program uses `Node.*` calls.

**Rationale:** Currently, the runtime has all the `snow_node_*` extern "C" functions but the compiler does not know about them. There are zero LLVM intrinsic declarations for any `snow_node_*` function, "Node" is not in the `STDLIB_MODULES` list, and there is no MIR lowering for Node calls. This means no Snow program can currently call any Node API.

**Three integration points:**

1. **`intrinsics.rs`** -- Declare all `snow_node_*` and `snow_register_function` / `snow_lookup_function` functions in the LLVM module:

```rust
// snow_node_start(name_ptr: ptr, name_len: u64, cookie_ptr: ptr, cookie_len: u64) -> i64
module.add_function("snow_node_start",
    i64_type.fn_type(&[ptr_type.into(), i64_type.into(), ptr_type.into(), i64_type.into()], false),
    Some(Linkage::External));

// snow_node_connect(name_ptr: ptr, name_len: u64) -> i64
module.add_function("snow_node_connect",
    i64_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
    Some(Linkage::External));

// snow_node_self() -> ptr
module.add_function("snow_node_self",
    ptr_type.fn_type(&[], false),
    Some(Linkage::External));

// snow_node_list() -> ptr
module.add_function("snow_node_list",
    ptr_type.fn_type(&[], false),
    Some(Linkage::External));

// snow_node_monitor(node_ptr: ptr, node_len: u64) -> u64
module.add_function("snow_node_monitor",
    i64_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
    Some(Linkage::External));

// snow_node_spawn(node_ptr: ptr, node_len: u64,
//                 fn_name_ptr: ptr, fn_name_len: u64,
//                 args_ptr: ptr, args_size: u64, link_flag: u8) -> u64
module.add_function("snow_node_spawn",
    i64_type.fn_type(&[ptr_type.into(), i64_type.into(),
                       ptr_type.into(), i64_type.into(),
                       ptr_type.into(), i64_type.into(),
                       i8_type.into()], false),
    Some(Linkage::External));

// snow_register_function(name_ptr: ptr, name_len: u64, fn_ptr: ptr) -> void
module.add_function("snow_register_function",
    void_type.fn_type(&[ptr_type.into(), i64_type.into(), ptr_type.into()], false),
    Some(Linkage::External));
```

2. **`lower.rs`** -- Add `"Node"` to `STDLIB_MODULES` and handle `Node.*` calls:

```rust
const STDLIB_MODULES: &[&str] = &[
    "String", "IO", "Env", "File", "List", "Map", "Set", "Tuple",
    "Range", "Queue", "HTTP", "JSON", "Json", "Request", "Job",
    "Math", "Int", "Float", "Timer", "Sqlite", "Pg", "Ws",
    "Node",  // NEW: Phase 67
];
```

Then in the field access lowering for `Node.*`, map to `MirExpr::Call` nodes that call the appropriate runtime functions. For example, `Node.spawn(node, echo, 42)` lowers to a call to `snow_node_spawn` with the node string, the function name `"echo"` as a string constant, and the serialized args.

3. **`mod.rs` (codegen)** -- In `generate_main_wrapper`, emit function registration calls:

```rust
// After snow_rt_init and snow_rt_init_actor, register all top-level functions.
for func in &self.mir_functions {
    if !func.is_closure_fn && !func.name.starts_with("__") {
        // Emit: snow_register_function("func_name", strlen, fn_ptr)
        let name_const = self.builder.build_global_string_ptr(&func.name, "fn_name");
        let name_len = self.context.i64_type().const_int(func.name.len() as u64, false);
        let fn_ptr = self.functions.get(&func.name).unwrap();
        let register_fn = get_intrinsic(&self.module, "snow_register_function");
        self.builder.build_call(register_fn,
            &[name_const.into(), name_len.into(), fn_ptr.as_global_value().as_pointer_value().into()],
            "");
    }
}
```

### Pattern 6: Spawn-and-Link Protocol

**What:** `Node.spawn_link` is `Node.spawn` with `link_flag=1`. The remote node establishes the link on its side (adds requester_pid to the new process's links) and sends a `DIST_LINK` back. The requesting node adds the remote PID to the caller's links upon receiving the reply.

**When to use:** `Node.spawn_link(node, function, args)`.

**Rationale:** Atomicity of spawn-and-link is important. If the link were set up after spawn returns, the spawned process could crash before the link is established, and the crash would not propagate. By handling it as part of the spawn protocol, the link is established before the spawned actor starts executing user code.

**Race condition:** Even with the link_flag, there's a window between spawning and the caller receiving the reply. If the spawned actor crashes in this window, the DIST_EXIT arrives before the DIST_SPAWN_REPLY. The caller must handle this by checking for DIST_EXIT signals for the spawned PID during the reply wait. In practice this is rare, and Erlang has the same race window.

### Pattern 7: Argument Serialization for Remote Spawn

**What:** Arguments to the remotely-spawned function must be serialized using the Snow Term Format (STF) wire encoding (from Phase 63's `dist::wire` module). The caller serializes the args; the remote side deserializes and passes them to the spawn function.

**When to use:** Building the DIST_SPAWN message payload.

**Rationale:** The existing STF encoder/decoder handles all Snow types (Int, Float, Bool, String, Tuple, List, Map, Set). Function pointers and closures are rejected at serialization time (per MSG-05). This is exactly the right behavior -- you cannot pass a closure as an argument to a remotely-spawned function (because closures capture local pointers).

**Codegen consideration:** The codegen for `Node.spawn(node, func, arg1, arg2)` must: (1) evaluate `arg1` and `arg2`, (2) pack them into an args buffer (same as local `ActorSpawn` codegen), (3) pass the packed buffer to `snow_node_spawn`. The runtime then includes this buffer in the DIST_SPAWN message. On the remote side, the buffer is passed directly to `sched.spawn(fn_ptr, args_ptr, args_size, priority)` which passes it through to the actor entry function.

**Key insight:** The args buffer format is already compatible. Local spawn packs args as an array of i64 values (ints directly, pointers via ptrtoint). For remote spawn, the args must be STF-encoded instead (because pointer values are meaningless across processes). This means `snow_node_spawn` must receive args in a different format than `snow_actor_spawn`. Two options:
- (A) Codegen emits STF encoding for remote spawn args (complex, requires codegen to know about STF).
- (B) Runtime receives the same i64-packed format and re-encodes to STF before sending over the wire (simpler for codegen, but the runtime must know the types of the args to encode them correctly).
- (C) Use the same i64-packed format on the wire, since both nodes run the same architecture (both are 64-bit, little-endian). This only works if args don't contain heap pointers (strings, lists, etc.).

**Recommendation:** Option A variant -- Codegen packs args the same way as local spawn (i64 array). The `snow_node_spawn` runtime function receives these packed args. Since Snow's remote spawn is between nodes running the same compiled binary (or at least same source), the i64-packed format is sufficient for primitive types (Int, Float, Bool, Pid). For heap-allocated types (String, List, Map), the runtime re-encodes them as STF before sending. This is the simplest path that handles the common case (actors spawned with integer/boolean state) and correctly handles complex types.

### Anti-Patterns to Avoid
- **Sending raw function pointers over the wire:** Function pointers are process-local memory addresses. Always send function NAMES. The EXEC-03 requirement exists for this reason.
- **Fabricating PIDs on the caller side:** The PID must be allocated by the node that hosts the process. Do not invent a PID before the remote node confirms the spawn. Always use request-reply.
- **Registering closure/lambda functions:** Closure functions capture environment pointers. They cannot be spawned remotely because the captured values would be dangling pointers on the remote node. Only register named top-level functions.
- **Holding session locks during spawn wait:** The caller blocks while waiting for DIST_SPAWN_REPLY. It must NOT hold any session or node state locks during this wait, or other actors on the same node will deadlock on distribution operations.
- **Assuming both nodes have the same functions:** If node A tries to spawn a function that node B doesn't have (different binary), the spawn must fail gracefully. The reply protocol includes a status field for this case.
- **Ignoring the creation counter in spawned PIDs:** The PID returned to the caller must include the remote node's current creation counter. If the remote node restarts, old PIDs are invalid. The creation counter from the session handshake must be used.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Wire message framing | Custom framing | Existing `write_msg` / `read_dist_msg` length-prefixed protocol | Already used by all distribution messages |
| Session/node lookup | Linear scan | Existing `sessions` and `node_id_map` in `NodeState` | FxHashMap already provides O(1) lookup |
| Local actor spawning | Custom spawn logic | Existing `sched.spawn(fn_ptr, args_ptr, args_size, priority)` | Already handles PID allocation, process table, work stealing |
| Link establishment | Custom link logic | Existing `link::link()` and `send_dist_link()` from Phase 66 | Already handles bidirectional link registration |
| Argument serialization | Custom format | Existing STF encoder/decoder in `dist::wire` (for complex types) | Already handles all Snow types, rejects closures |
| Coroutine yield/wake | Custom blocking | Existing `ProcessState::Waiting` + mailbox + `wake_process` | Already used by `receive`, `service_call`, `job_await` |
| LLVM function declaration | Manual IR | Existing `declare_intrinsics` + `get_intrinsic` pattern | All other runtime functions use this pattern |
| Stdlib module call lowering | New MIR nodes | Existing STDLIB_MODULES pattern in `lower.rs` | All other module calls (List, Map, Timer, etc.) use this pattern |

## Common Pitfalls

### Pitfall 1: Spawn Reply Arriving After Caller Times Out or Exits
**What goes wrong:** The caller sends DIST_SPAWN but exits or crashes before the reply arrives. The reply message arrives in a dead actor's mailbox and is silently dropped. The remote actor was spawned but the caller never gets the PID.
**Why it happens:** Network latency or caller crash during the spawn wait.
**How to avoid:** If spawn_link was used, the remote actor will receive a DIST_EXIT when the caller crashes (Phase 66 infrastructure handles this). If regular spawn was used, the remote actor continues running -- this is correct behavior (fire-and-forget semantics for the spawned actor itself). The only issue is resource leak if the spawned actor was expected to be supervised. Accept this as inherent to distributed spawn (Erlang has the same behavior).
**Warning signs:** Orphaned remote actors after caller crashes during spawn.

### Pitfall 2: Function Name Collision in Registry
**What goes wrong:** Two functions with the same name (e.g., from different modules) collide in the registry. Only the last-registered function pointer is stored.
**Why it happens:** Snow's MIR lowering already handles name mangling (e.g., `counter_Int` for monomorphized generics), but module-qualified names could still collide if two modules define functions with the same mangled name.
**How to avoid:** Use fully-qualified names including module prefix in the registry key (e.g., `"MyModule.worker"` or `"worker_Int"`). The MIR function name is already unique within a compilation unit -- use that directly.
**Warning signs:** Remote spawn invokes the wrong function.

### Pitfall 3: Node Module Not in STDLIB_MODULES
**What goes wrong:** `Node.spawn(...)` in Snow source code fails to compile with "unknown function" or "unknown module" error.
**Why it happens:** The `STDLIB_MODULES` list in `lower.rs` does not include `"Node"`. The field access lowering path checks this list to determine if a qualified call like `Node.spawn` is a stdlib call.
**How to avoid:** Add `"Node"` to the `STDLIB_MODULES` constant. Verify all `Node.*` calls are handled in the stdlib call lowering switch.
**Warning signs:** Compilation errors on any `Node.*` call.

### Pitfall 4: Blocking Main Thread with Node.spawn
**What goes wrong:** `Node.spawn` is called from the main thread (not from an actor coroutine). The main thread tries to yield/wait for the reply, but it's not running in a coroutine context, causing a panic.
**Why it happens:** `snow_node_spawn` uses coroutine yield (like `receive`) to wait for the reply. The main thread is not a coroutine.
**How to avoid:** Check `stack::CURRENT_YIELDER` before attempting to yield. If called from the main thread, use a different blocking mechanism (e.g., `std::sync::mpsc` channel or `Condvar`). Alternatively, document that `Node.spawn` must be called from within an actor.
**Warning signs:** Panic when calling `Node.spawn` from `main()`.

### Pitfall 5: Args Buffer Lifetime for Remote Spawn
**What goes wrong:** The args buffer is allocated on the GC heap (like local spawn), but the DIST_SPAWN message is sent asynchronously. By the time the wire message is built, the GC may have collected the args buffer.
**Why it happens:** Local spawn passes the args pointer to the scheduler which copies it immediately. Remote spawn must serialize the args into the wire message before sending.
**How to avoid:** In `snow_node_spawn`, immediately copy the args data into the wire message payload buffer (a `Vec<u8>` on the Rust heap, not the GC heap). Do not retain a pointer to the GC-allocated args buffer after building the message.
**Warning signs:** Corrupted args on the remote side, segfaults during DIST_SPAWN message construction.

### Pitfall 6: PID Node-ID Assignment in Spawn Reply
**What goes wrong:** The remote node sends back the spawned process's local PID (node_id=0). The caller uses it as-is, resulting in a PID that looks local. Messages sent to this PID go to the local process table instead of the remote node.
**Why it happens:** The spawned process has node_id=0 on its home node. The caller must reconstruct it with the remote node's node_id.
**How to avoid:** The DIST_SPAWN_REPLY sends only the 40-bit local_id of the spawned process. The caller reconstructs the full PID using `ProcessId::from_remote(session.node_id, session.creation, local_id)`.
**Warning signs:** Messages to remotely-spawned actors are silently dropped or delivered to wrong local process.

### Pitfall 7: Missing Process.monitor and Process.demonitor Intrinsics
**What goes wrong:** Phase 66 added `snow_process_monitor` and `snow_process_demonitor` extern "C" functions to the runtime, but they are not declared as LLVM intrinsics. Snow code calling `Process.monitor(pid)` will fail to compile.
**Why it happens:** Phase 66 focused on the runtime implementation but did not wire the new functions into the codegen pipeline (no intrinsic declarations, no MIR lowering for `Process.monitor`).
**How to avoid:** Phase 67's LLVM integration work should also wire `snow_process_monitor` and `snow_process_demonitor` into intrinsics.rs and lower.rs. Consider adding "Process" to STDLIB_MODULES if it's not already there, or handling it as a special case like the existing `Process.monitor` pattern.
**Warning signs:** Compilation errors on `Process.monitor(pid)` calls.

## Code Examples

### Complete snow_node_spawn Runtime Function
```rust
// Source: New extern "C" API in dist/node.rs

/// Atomic counter for spawn request IDs.
static SPAWN_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

/// Reserved type_tag for spawn reply messages.
pub(crate) const SPAWN_REPLY_TAG: u64 = u64::MAX - 4;

/// New distribution message tag: remote spawn request.
pub(crate) const DIST_SPAWN: u8 = 0x19;
/// New distribution message tag: remote spawn reply.
pub(crate) const DIST_SPAWN_REPLY: u8 = 0x1A;

/// Spawn an actor on a remote node.
///
/// Called from compiled Snow code via `Node.spawn(node, function, args)`.
/// Blocks the calling actor (yields coroutine) until the remote node replies
/// with the spawned PID.
///
/// # Arguments
/// - `node_ptr`, `node_len`: Target node name (UTF-8)
/// - `fn_name_ptr`, `fn_name_len`: Function name to spawn (UTF-8)
/// - `args_ptr`, `args_size`: Packed argument buffer (i64 array)
/// - `link_flag`: 0=spawn, 1=spawn_link
///
/// # Returns
/// - Remote PID (u64) on success
/// - 0 on failure (not connected, function not found, etc.)
#[no_mangle]
pub extern "C" fn snow_node_spawn(
    node_ptr: *const u8,
    node_len: u64,
    fn_name_ptr: *const u8,
    fn_name_len: u64,
    args_ptr: *const u8,
    args_size: u64,
    link_flag: u8,
) -> u64 {
    use crate::actor::{stack, global_scheduler};
    use crate::actor::process::{ProcessId, ProcessState, Message};
    use crate::actor::heap::MessageBuffer;

    let my_pid = match stack::get_current_pid() {
        Some(pid) => pid,
        None => return 0,
    };

    let state = match node_state() {
        Some(s) => s,
        None => return 0,
    };

    let node_name = unsafe {
        std::str::from_utf8(std::slice::from_raw_parts(node_ptr, node_len as usize))
            .unwrap_or("")
    };

    let fn_name = unsafe {
        std::str::from_utf8(std::slice::from_raw_parts(fn_name_ptr, fn_name_len as usize))
            .unwrap_or("")
    };

    // Look up session.
    let session = {
        let sessions = state.sessions.read();
        match sessions.get(node_name) {
            Some(s) => Arc::clone(s),
            None => return 0,
        }
    };

    // Generate request ID.
    let req_id = SPAWN_REQUEST_ID.fetch_add(1, Ordering::Relaxed);

    // Build DIST_SPAWN payload.
    let args_data = if args_ptr.is_null() || args_size == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(args_ptr, args_size as usize) }
    };

    let fn_name_bytes = fn_name.as_bytes();
    let mut payload = Vec::with_capacity(1 + 8 + 8 + 1 + 2 + fn_name_bytes.len() + args_data.len());
    payload.push(DIST_SPAWN);
    payload.extend_from_slice(&req_id.to_le_bytes());
    payload.extend_from_slice(&my_pid.as_u64().to_le_bytes());
    payload.push(link_flag);
    payload.extend_from_slice(&(fn_name_bytes.len() as u16).to_le_bytes());
    payload.extend_from_slice(fn_name_bytes);
    payload.extend_from_slice(args_data);

    // Send the request.
    {
        let mut stream = session.stream.lock().unwrap();
        if write_msg(&mut *stream, &payload).is_err() {
            return 0;
        }
    }

    // Wait for DIST_SPAWN_REPLY in mailbox.
    // The reader thread will deliver it as a message with SPAWN_REPLY_TAG.
    let sched = global_scheduler();
    loop {
        // Enter waiting state and yield.
        if let Some(proc_arc) = sched.get_process(my_pid) {
            let mut proc = proc_arc.lock();
            // Check mailbox for existing reply first.
            if let Some(reply_msg) = find_spawn_reply(&mut proc, req_id) {
                return decode_spawn_reply_pid(reply_msg, &session);
            }
            proc.state = ProcessState::Waiting;
            drop(proc);
        }

        // Yield to scheduler.
        stack::yield_current();

        // Woken up -- check mailbox again.
        if let Some(proc_arc) = sched.get_process(my_pid) {
            let mut proc = proc_arc.lock();
            if let Some(reply_msg) = find_spawn_reply(&mut proc, req_id) {
                let pid = decode_spawn_reply_pid(reply_msg, &session);
                // If spawn_link, add remote PID to our links.
                if link_flag == 1 && pid != 0 {
                    proc.links.insert(ProcessId(pid));
                }
                return pid;
            }
        }
    }
}
```

### Reader Loop DIST_SPAWN_REPLY Handler
```rust
// Source: Extension of reader_loop_session in dist/node.rs

DIST_SPAWN_REPLY => {
    // Wire format: [tag][u64 req_id][u8 status][u64 spawned_local_id]
    if msg.len() >= 18 {
        let req_id = u64::from_le_bytes(msg[1..9].try_into().unwrap());
        let status = msg[9];
        let spawned_local_id = u64::from_le_bytes(msg[10..18].try_into().unwrap());

        // Deliver to the requester's mailbox as a SPAWN_REPLY_TAG message.
        // The requester's PID is not in the reply -- we need another mechanism.
        // Option: the reader thread maintains a map of pending spawn requests
        // (req_id -> requester_pid). Populated when DIST_SPAWN is sent.
        //
        // Simpler: encode requester_pid in the reply itself.
        // We can use SPAWN_REPLY_TAG messages delivered to ALL processes
        // whose mailbox will match on req_id.
        //
        // Simplest: the requester stored req_id in its own state. The reader
        // thread can't know which process to deliver to without tracking.
        //
        // Best: Add a pending_spawn_requests map to NodeSession:
        //   pending_spawns: Mutex<FxHashMap<u64, ProcessId>>
        // Populated before sending DIST_SPAWN, read here, removed after delivery.
    }
}
```

### Codegen: Function Registration in Main Wrapper
```rust
// Source: Modified generate_main_wrapper in codegen/mod.rs

fn generate_main_wrapper(&mut self, entry_name: &str) -> Result<(), String> {
    // ... existing init code ...

    // Register all top-level functions for remote spawn.
    let register_fn = get_intrinsic(&self.module, "snow_register_function");
    for func in &self.mir_functions {
        // Skip closure functions and internal compiler-generated functions.
        if func.is_closure_fn || func.name.starts_with("__") {
            continue;
        }

        // Create a global string constant for the function name.
        let name_str = self.builder.build_global_string_ptr(&func.name, &format!("fn_name_{}", func.name))
            .map_err(|e| e.to_string())?;
        let name_len = self.context.i64_type().const_int(func.name.len() as u64, false);

        // Get the function pointer.
        let fn_val = self.functions.get(&func.name)
            .ok_or_else(|| format!("Function '{}' not found for registration", func.name))?;

        self.builder.build_call(
            register_fn,
            &[
                name_str.as_pointer_value().into(),
                name_len.into(),
                fn_val.as_global_value().as_pointer_value().into(),
            ],
            "",
        ).map_err(|e| e.to_string())?;
    }

    // ... rest of main wrapper ...
}
```

### MIR Lowering: Node.spawn Call
```rust
// Source: Modified lower_field_access_call in mir/lower.rs

// In the STDLIB_MODULES handling section, when base_name == "Node":
"Node" => match field_name.as_str() {
    "spawn" => {
        // Node.spawn(node, function, args...)
        // Lowers to: call snow_node_spawn(node_ptr, node_len, fn_name_ptr, fn_name_len, args_ptr, args_size, 0)
        // The function argument must be a named function reference, not a closure.
        // Extract function name as string constant.
        self.lower_node_spawn(args, false)
    }
    "spawn_link" => {
        // Node.spawn_link(node, function, args...)
        self.lower_node_spawn(args, true)
    }
    "start" => {
        // Node.start(name, cookie: cookie_str)
        // Already implemented as snow_node_start in runtime
        self.lower_node_start(args)
    }
    "connect" => {
        // Node.connect(node_name)
        self.lower_node_connect(args)
    }
    "self" => {
        // Node.self() -> String
        MirExpr::Call {
            func: Box::new(MirExpr::Var("snow_node_self".to_string(),
                MirType::FnPtr(vec![], Box::new(MirType::String)))),
            args: vec![],
            ty: MirType::String,
        }
    }
    "list" => {
        // Node.list() -> List<String>
        MirExpr::Call {
            func: Box::new(MirExpr::Var("snow_node_list".to_string(),
                MirType::FnPtr(vec![], Box::new(MirType::Ptr)))),
            args: vec![],
            ty: MirType::Ptr,
        }
    }
    "monitor" => {
        // Node.monitor(node_name)
        self.lower_node_monitor(args)
    }
    _ => panic!("Unknown Node function: {}", field_name),
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| No remote spawn | `Node.spawn(node, func, args)` with reply protocol | Phase 67 | Users can distribute work across nodes |
| Function pointers for spawn | Function name registry for remote spawn | Phase 67 | Cross-binary compatibility; different compilations can interop |
| Node module not in compiler | Full LLVM integration for all Node.* calls | Phase 67 | Snow programs can use distributed features |
| No function registration | Codegen emits registration calls at startup | Phase 67 | Runtime knows function names for remote lookup |

**Deprecated/outdated:**
- Direct `fn_ptr` passing in spawn is still used for local spawn (no change). Remote spawn adds a parallel path using function names.
- The `snow_node_start`, `snow_node_connect`, `snow_node_self`, `snow_node_list`, `snow_node_monitor` functions exist in the runtime but are currently dead code from the compiler's perspective. Phase 67 makes them live.

## Open Questions

1. **Function Name Granularity for Remote Spawn**
   - What we know: MIR function names include monomorphization suffixes (e.g., `worker_Int`, `echo_String`). Actor start functions are named `actor_name_start`.
   - What's unclear: Should users pass the original Snow function name (`worker`) or the mangled name (`worker_Int`)? Should the compiler generate both registry entries?
   - Recommendation: Register both the original name AND the mangled name. The user writes `Node.spawn(node, worker, 42)` which lowering converts to the function name `"worker"`. If there are multiple monomorphized versions, the registry stores the one with matching argument types. For the common case of actors (which take concrete types), the mangled name is unique. Register under both names for convenience.

2. **Spawn Timeout**
   - What we know: The caller blocks indefinitely waiting for DIST_SPAWN_REPLY. If the remote node is slow or the message is lost, the caller hangs forever.
   - What's unclear: Should there be a timeout? What should the default be?
   - Recommendation: Add an optional timeout (default 5000ms). If the reply doesn't arrive within the timeout, return 0 (failure). This prevents permanent hangs. The timeout can be configurable via an optional argument: `Node.spawn(node, func, args, timeout: 5000)`.

3. **Error Handling for Failed Remote Spawn**
   - What we know: `Node.spawn` currently returns 0 on failure (not connected, function not found, etc.). This is a bare integer with no error information.
   - What's unclear: Should the API return a `Result` type instead? Or should it crash the caller (like Erlang's `spawn/4` which raises `badarg` for invalid node)?
   - Recommendation: Return a `Result` type or crash with a panic. Returning 0 silently hides errors. A crash is safer (consistent with Erlang's behavior). Use a panic with descriptive message: "Node.spawn failed: node 'X' not connected" or "Node.spawn failed: function 'Y' not found on remote node."

4. **Pending Spawn Request Tracking**
   - What we know: When DIST_SPAWN_REPLY arrives on the reader thread, it needs to know which actor to deliver it to. The reply contains only `req_id`, `status`, and `spawned_pid`.
   - What's unclear: How does the reader thread map `req_id` to the waiting actor?
   - Recommendation: Add a `pending_spawns: Mutex<FxHashMap<u64, ProcessId>>` field to `NodeSession`. Before sending DIST_SPAWN, register `(req_id -> my_pid)` in this map. When DIST_SPAWN_REPLY arrives, look up the requester PID and deliver the reply message to their mailbox. Clean up the entry after delivery.

5. **Process.monitor and Process.demonitor LLVM Integration**
   - What we know: Phase 66 added `snow_process_monitor` and `snow_process_demonitor` to the runtime but did not add them to the compiler. The "Process" module concept does not exist in `STDLIB_MODULES`.
   - What's unclear: Should Phase 67 also wire `Process.monitor` and `Process.demonitor` into the compiler, or defer to a separate phase?
   - Recommendation: Include it in Phase 67's LLVM integration work. It's a small addition (two intrinsic declarations, two lowering cases) and completes the fault-tolerance API surface. Add "Process" to STDLIB_MODULES alongside "Node".

6. **Node.spawn with Actor Functions vs Regular Functions**
   - What we know: In Snow, actors are defined with the `actor` keyword and compiled to a `_start` function that wraps the loop. Regular functions can also be spawned as actors.
   - What's unclear: When a user writes `Node.spawn(node, my_actor, initial_state)`, should the lowering use the actor's `_start` function name or the loop function name?
   - Recommendation: Use the same naming convention as local spawn. Local spawn of actors already generates a call to the `_start` function. For remote spawn, the registered function name should be the `_start` function name. The lowering should detect actor references and use the appropriate start function name, just as it does for local `spawn(my_actor, args)`.

## Sources

### Primary (HIGH confidence)
- **Codebase analysis** (direct file reads):
  - `crates/snow-rt/src/dist/node.rs` -- NodeState (line 46), NodeSession, wire message tags (DIST_SEND=0x10 through DIST_MONITOR_EXIT=0x18), reader_loop_session dispatch (line 545), write_msg/read_dist_msg framing, snow_node_start (line 1913), snow_node_connect (line 2009), snow_node_self (line 2102), snow_node_list (line 2120), cleanup_session, send_dist_link (line 374), send_dist_exit (line 441)
  - `crates/snow-rt/src/actor/mod.rs` -- snow_actor_spawn (line 131) taking fn_ptr/args/args_size/priority, snow_actor_send (line 261) with locality check, snow_node_monitor (line 1243), local_send (line 275), dist_send (line 316)
  - `crates/snow-rt/src/actor/scheduler.rs` -- SpawnRequest struct (line 50) with fn_ptr field, Scheduler::spawn (line 157), handle_process_exit (line 607) with local/remote link partitioning
  - `crates/snow-rt/src/actor/process.rs` -- ProcessId struct with node_id/creation/local_id bit packing, ProcessId::from_remote (line 77), is_local (line 68)
  - `crates/snow-rt/src/actor/link.rs` -- link/unlink functions, propagate_exit, EXIT_SIGNAL_TAG, DOWN_SIGNAL_TAG, encode_exit_signal/decode_exit_signal
  - `crates/snow-rt/src/actor/stack.rs` -- CoroutineHandle::new (line 131) casting fn_ptr to extern "C" fn, yield_current
  - `crates/snow-rt/src/lib.rs` -- current extern "C" re-exports (line 102: snow_node_self, snow_node_list, snow_node_start, snow_node_connect)
  - `crates/snow-codegen/src/codegen/intrinsics.rs` -- declare_intrinsics (line 16), all current intrinsic declarations. NO snow_node_* intrinsics exist. NO snow_register_function exists.
  - `crates/snow-codegen/src/codegen/expr.rs` -- codegen_actor_spawn (line 1635) generating snow_actor_spawn call with fn_ptr and packed i64 args array
  - `crates/snow-codegen/src/codegen/mod.rs` -- CodeGen struct (line 43), compile_function (line 356), functions map (line 65), generate_main_wrapper
  - `crates/snow-codegen/src/mir/mod.rs` -- MirExpr::ActorSpawn (line 243) with func/args/priority, MirType::Pid (line 91)
  - `crates/snow-codegen/src/mir/lower.rs` -- STDLIB_MODULES (line 9274) listing all known modules (Node is NOT in the list), lower_spawn_expr (line 9175), lower_link_expr (line 9264), field access lowering for stdlib calls (line 5808)
  - `.planning/REQUIREMENTS.md` -- EXEC-01, EXEC-02, EXEC-03 definitions (lines 42-44)
  - `.planning/ROADMAP.md` -- Phase 67 success criteria and dependencies
  - `.planning/phases/66-remote-links-monitors-failure-handling/66-RESEARCH.md` -- complete Phase 66 architecture including DIST_LINK/DIST_EXIT, handle_node_disconnect, remote monitors

### Secondary (MEDIUM confidence)
- [Erlang Distribution Protocol](https://erlang.org/doc/apps/erts/erl_dist_protocol.html) -- SPAWN_REQUEST (tag 29), SPAWN_REQUEST_TT (tag 30), SPAWN_REPLY (tag 31), SPAWN_REPLY_TT (tag 32) wire messages for remote spawn
- [Erlang spawn/4](https://www.erlang.org/doc/man/erlang.html#spawn-4) -- `spawn(Node, Module, Function, Args)` semantics: Module:Function/Arity must exist on remote node, returns PID, raises badarg if Node not connected
- [Erlang spawn_link/4](https://www.erlang.org/doc/man/erlang.html#spawn_link-4) -- Atomic spawn-and-link semantics, crash propagation

### Tertiary (LOW confidence)
- Erlang's `code:ensure_loaded/1` -- verifying module availability before remote spawn. Snow's function registry serves the same purpose but without module granularity.

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- zero new dependencies; all existing infrastructure reused
- Architecture (function name registry): HIGH -- straightforward FxHashMap, well-established pattern from Erlang's MFA (Module/Function/Arity) system
- Architecture (remote spawn wire protocol): HIGH -- follows Erlang's SPAWN_REQUEST/SPAWN_REPLY pattern, clear request-reply semantics
- Architecture (LLVM integration): HIGH -- follows exact same pattern as all other stdlib modules (Timer, HTTP, Pg, etc.), mechanical addition
- Architecture (spawn-and-link): HIGH -- reuses Phase 66's DIST_LINK infrastructure, straightforward extension
- Pitfalls: HIGH -- derived from analyzing actual code paths, PID encoding, and distributed systems race conditions
- Open questions: MEDIUM -- function name granularity and spawn timeout are design choices with clear recommendations

**Research date:** 2026-02-12
**Valid until:** 2026-03-14 (stable domain; all dependencies frozen)
