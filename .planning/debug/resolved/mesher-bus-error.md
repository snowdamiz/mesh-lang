---
status: resolved
trigger: "mesher binary crashes with Bus error: 10 after startup"
created: 2026-02-15T00:00:00Z
updated: 2026-02-15T00:15:00Z
---

## Current Focus

hypothesis: CONFIRMED and FIXED
test: N/A
expecting: N/A
next_action: Archive

## Symptoms

expected: mesher binary runs continuously serving HTTP on :8080 and WebSocket on :8081
actual: mesher starts successfully, runs for ~20 seconds through several health check and load monitor cycles, then crashes with "Bus error: 10" (signal 10 = SIGBUS)
errors: "sh: line 1: 25240 Bus error: 10           ./mesher" — exit code 138
reproduction: Run `npm run dev` which starts mesher via `meshc build mesher/ && cd mesher && ./mesher`
started: Current state — recent regression from uncommitted changes

## Eliminated

- hypothesis: TargetData::create("") returns wrong sizes but get_target_data() returns correct ones
  evidence: Initial IR check seemed to show same results, but that was a stale .ll file from before the fix. Debug logging confirmed get_target_data() returns correct sizes (RegistryState=40, RateLimitState=24).
  timestamp: 2026-02-15T00:10:00Z

## Evidence

- timestamp: 2026-02-15T00:01:00Z
  checked: git diff of all uncommitted changes
  found: Three areas changed - (1) expr.rs actor spawn serialization changed from fixed 8-byte-per-arg i64 array to variable-size layout where structs use their full LLVM store size, (2) lower.rs changed service init return type from MirType::Int to actual init_ret_ty (struct type), (3) node.rs changed mesh_node_self to return empty string instead of null, (4) schema.mpl changed uuidv7 to gen_random_uuid
  implication: Changes (1) and (2) work together -- the MIR now correctly types the init state as a struct, and the spawn serializer now stores structs at their full size. The tuple encoding in codegen_make_tuple also needs correct sizes.

- timestamp: 2026-02-15T00:05:00Z
  checked: LLVM IR for service loop state handling
  found: Service loops correctly load struct-typed state (e.g., load %RegistryState from args pointer). The actor spawn correctly stores struct state at full byte size. But codegen_make_tuple (tuple encoding for handler return values) used TargetData::create("") which computes wrong sizes.
  implication: When a call handler returns (new_state, reply) as a tuple, the new_state struct gets encoded into the tuple. With wrong TargetData, large structs were treated as small (<=8 bytes) and truncated to a single i64.

- timestamp: 2026-02-15T00:08:00Z
  checked: LLVM default data layout behavior
  found: TargetData::create("") uses LLVM's default data layout (x86-32 Linux defaults) which reports wrong sizes for struct types on arm64-apple-darwin.
  implication: All 6 uses of TargetData::create("") in the codebase could produce wrong type sizes.

- timestamp: 2026-02-15T00:12:00Z
  checked: Debug logging of struct sizes with get_target_data()
  found: With self.target_machine.get_target_data(), struct sizes are correct: RegistryState=40, RateLimitState=24, StreamState=8, ProcessorState=16.
  implication: Fix is correct -- the target machine's data layout computes proper sizes.

- timestamp: 2026-02-15T00:13:00Z
  checked: Runtime verification -- mesher binary runs for 30+ seconds
  found: Binary starts all services, completes 3 health checks and 6 load monitor cycles without crashing. Exit code 144 (SIGTERM from our test kill) instead of 138 (SIGBUS).
  implication: Fix verified -- no more bus error.

- timestamp: 2026-02-15T00:14:00Z
  checked: Test suite
  found: All 176 mesh-codegen tests pass.
  implication: No regressions from the fix.

## Resolution

root_cause: TargetData::create("") uses LLVM's default data layout (x86-32 Linux defaults) which computes incorrect struct store sizes on arm64-apple-darwin. This caused codegen_make_tuple to use the "small struct" path (truncate to single i64) for service state structs that are actually 24-48 bytes. When a service call handler returns (new_state, reply) as a tuple, the new_state struct was truncated to its first 8 bytes. The service loop then extracted this truncated value via mesh_tuple_first, interpreted it as a pointer (inttoptr), and tried to load a full struct from that address -- causing SIGBUS.
fix: Replaced all 6 instances of TargetData::create("") with self.target_machine.get_target_data() (5 in expr.rs, 1 in types.rs via parameter threading). The target machine's data layout correctly reports struct sizes for the actual target architecture.
verification: Binary runs 30+ seconds without crash (3 health checks, 6 load monitor cycles). All 176 tests pass.
files_changed: [crates/mesh-codegen/src/codegen/expr.rs, crates/mesh-codegen/src/codegen/types.rs, crates/mesh-codegen/src/codegen/mod.rs]
