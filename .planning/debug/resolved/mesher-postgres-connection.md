---
status: resolved
trigger: "Mesher backend crashes with Bus error: 10 (exit code 138) after logging Schema error and Partition error"
created: 2026-02-15T00:00:00Z
updated: 2026-02-15T19:10:00Z
---

## Current Focus

hypothesis: RESOLVED - Three root causes found and fixed
test: Ran mesher for 65+ seconds with all services, health checks, alert evaluator, load monitor
expecting: Stable operation
next_action: Archive session

## Symptoms

expected: Mesher connects to PostgreSQL, creates schema, starts all services, and runs stably
actual: Mesher connects, logs "Schema error" and "Partition error", starts services (OrgService, ProjectService, UserService, StreamManager, RateLimiter, EventProcessor), then crashes with "Bus error: 10" (exit code 138)
errors: "[Mesher] Schema error", "[Mesher] Partition error", "Bus error: 10", exit code 138
reproduction: npm run dev:mesher (from project root). Docker PostgreSQL is running with correct credentials.
started: After successfully setting up PostgreSQL Docker container. Connection works now but schema/partition operations fail, then bus error occurs.

## Eliminated

- hypothesis: Bus error from GC stack scanning
  evidence: GC is conservative and scans coroutine stacks. The crash happens in service actor startup, not during GC.
  timestamp: 2026-02-15T12:00:00Z

- hypothesis: Bus error from mesh_reduction_check on main thread
  evidence: mesh_reduction_check checks CURRENT_YIELDER and returns early on main thread (not in coroutine)
  timestamp: 2026-02-15T12:00:00Z

## Evidence

- timestamp: 2026-02-15T11:30:00Z
  checked: Running mesher binary
  found: Process exits with code 138 (SIGBUS). Output shows schema/partition errors, then services start up to EventProcessor, then crash.
  implication: Crash happens shortly after actors begin executing, not during main thread startup.

- timestamp: 2026-02-15T11:35:00Z
  checked: PostgreSQL uuidv7() function
  found: uuidv7() does not exist in PostgreSQL 17. Schema creation fails because of this. This causes the "Schema error" and "Partition error" log messages.
  implication: Schema errors are a separate issue from the Bus error.

- timestamp: 2026-02-15T11:45:00Z
  checked: LLVM IR for __service_storagewriter_start
  found: State is treated as MirType::Int (i64) - only 8 bytes allocated for spawn args, but loop function loads full %WriterState struct (48 bytes).
  implication: Out-of-bounds memory read -> SIGBUS.

- timestamp: 2026-02-15T11:50:00Z
  checked: All __service_*_start functions in LLVM IR
  found: EVERY service start function has the identical bug. E.g., StorageWriter: 48 bytes needed, 8 allocated. ProcessorState: 16 bytes needed, 8 allocated.
  implication: All services affected, crash on first actor scheduled.

- timestamp: 2026-02-15T12:00:00Z
  checked: MIR lowering code in lower.rs lines 9340-9393
  found: Start function body hardcodes MirType::Int for all state types.
  implication: ROOT CAUSE #1 for SIGBUS.

- timestamp: 2026-02-15T18:50:00Z
  checked: After fixing SIGBUS, Node.self() returns null in standalone mode
  found: mesh_string_eq(null, "") causes panic in register_global_services.
  implication: ROOT CAUSE #2 -- null pointer from Node.self().

- timestamp: 2026-02-15T19:00:00Z
  checked: After fixing Node.self(), service call handler tuple returns
  found: Call handler return packs struct state by truncating to i64. Service loop then interprets this truncated value as a pointer and loads full struct -> SIGSEGV.
  implication: ROOT CAUSE #3 -- tuple StructValue packing truncates large structs.

- timestamp: 2026-02-15T19:10:00Z
  checked: Full stability test (65 seconds)
  found: All services start, 6 health checks pass, 2 alert evaluator runs succeed, 12+ load monitor checks pass, HTTP/WS servers running.
  implication: All three bugs fixed. Mesher runs stably.

## Resolution

root_cause: Three interconnected issues:

1. **SIGBUS (Bus error: 10)**: Service start function MIR codegen hardcodes MirType::Int for the init state type. This causes only 8 bytes to be allocated/copied when spawning the actor, but the actor loop loads the full struct (16-48 bytes), causing out-of-bounds memory access.

2. **Schema/Partition errors**: SQL schema uses uuidv7() which doesn't exist in PostgreSQL 17. gen_random_uuid() (from pgcrypto) should be used instead.

3. **SIGSEGV (after SIGBUS fix)**: Two sub-issues:
   a. Node.self() returns null pointer when node is not started; Mesh code compares it with "" causing null deref in mesh_string_eq.
   b. Tuple element packing in call handler returns truncates large struct values to 8 bytes. Service loop then interprets the truncated value as a pointer -> SIGSEGV.

fix: Four changes across four files:

1. crates/mesh-codegen/src/mir/lower.rs: Replace MirType::Int with init_ret_ty.clone() in service start function body (5 locations in lines 9346-9374).

2. crates/mesh-codegen/src/codegen/expr.rs: In codegen_actor_spawn, compute actual byte size per arg (using TargetData) instead of assuming 8 bytes. Store struct values directly to the buffer instead of coercing to i64.

3. crates/mesh-codegen/src/codegen/expr.rs: In tuple element packing (StructValue case), heap-allocate large structs (>8 bytes) and store the pointer, instead of truncating to first 8 bytes.

4. mesher/storage/schema.mpl: Replace all uuidv7() with gen_random_uuid().

5. crates/mesh-rt/src/dist/node.rs: mesh_node_self() returns empty string instead of null when node is not started.

verification: Mesher runs stably for 65+ seconds. All services start. Health checks pass every 10s. Alert evaluator runs every 30s. Load monitor runs every 5s. HTTP (:8080) and WebSocket (:8081) servers listening.

files_changed:
  - crates/mesh-codegen/src/mir/lower.rs
  - crates/mesh-codegen/src/codegen/expr.rs
  - crates/mesh-rt/src/dist/node.rs
  - mesher/storage/schema.mpl
