---
status: resolved
trigger: "mesher binary crashes with Bus error: 10 after ~10 seconds of running"
created: 2026-02-15T00:00:00Z
updated: 2026-02-15T00:20:00Z
---

## Current Focus

hypothesis: CONFIRMED - same root cause as previous debug session (mesher-bus-error.md). The fix (replacing TargetData::create("") with target_machine.get_target_data()) is already applied in the compiler source. The user's binary was stale (compiled with pre-fix compiler).
test: rebuilt compiler + mesher binary and ran for 35 seconds
expecting: no crash
next_action: verify no remaining TargetData::create("") in non-test code, confirm fix is committed

## Symptoms

expected: Mesher binary runs persistently, serving HTTP on :8080 and WebSocket on :8081
actual: Mesher starts fine, all services initialize ("Foundation ready", servers listening), runs for ~10 seconds (2 load monitor ticks, 1 health check), then crashes with "Bus error: 10" (exit code 138)
errors: "sh: line 1: 32569 Bus error: 10 ./mesher" followed by "npm run dev:mesher exited with code 138"
reproduction: Run `npm run dev` which starts the mesher. It crashes consistently after startup.
started: Recurring issue after recent build (phase 95 complete, v9.0 milestone done)

## Eliminated

- hypothesis: Stack overflow from actor recursive calls without TCE
  evidence: TCE (tail call elimination) is properly implemented. All timer actors (load_monitor, health_checker, stream_drain_ticker, etc.) have their recursive calls rewritten to TailCall nodes which compile to `br label %tce_loop` in LLVM IR.
  timestamp: 2026-02-15T00:05:00Z

- hypothesis: Args buffer freed by parent GC before child reads it
  evidence: The args buffer is read immediately by the wrapper function when the actor starts. By the time GC could trigger on the parent, the args are already copied into the child's local variables.
  timestamp: 2026-02-15T00:08:00Z

- hypothesis: Service message format mismatch causing bad pointer dereference
  evidence: Message layout is correct: [u64 type_tag][u64 data_len][data bytes]. Service loop reads at correct offsets (data_ptr = msg_ptr + 16, tag at +0, caller at +8, args at +16).
  timestamp: 2026-02-15T00:10:00Z

- hypothesis: New instance of TargetData::create("") in non-test code
  evidence: Grepped entire codebase -- only 2 instances remain, both in types.rs tests (lines 290, 318). All production code uses self.target_machine.get_target_data().
  timestamp: 2026-02-15T00:15:00Z

## Evidence

- timestamp: 2026-02-15T00:02:00Z
  checked: Previous debug session .planning/debug/resolved/mesher-bus-error.md
  found: Identical bug was already diagnosed and resolved. Root cause: TargetData::create("") uses LLVM default data layout (x86-32 Linux defaults) which computes wrong struct store sizes on arm64-apple-darwin. Fix: replace with self.target_machine.get_target_data().
  implication: This is a recurrence of the same bug, likely from a stale binary.

- timestamp: 2026-02-15T00:05:00Z
  checked: All 6 codegen sites that previously used TargetData::create("")
  found: 5 in expr.rs and 1 in types.rs (parameter threaded from mod.rs) are all fixed. Only test code still uses TargetData::create("").
  implication: The compiler source code has the correct fix applied.

- timestamp: 2026-02-15T00:08:00Z
  checked: Generated LLVM IR for service state struct sizes
  found: RegistryState=40 bytes (5 x i64), RateLimitState=24 bytes (ptr + 2 x i64), StreamState=8 bytes (ptr), ProcessorState=16 bytes (2 x i64). These are correct.
  implication: The compiler is generating correct code with the fix.

- timestamp: 2026-02-15T00:12:00Z
  checked: Rebuilt compiler (cargo build --release) and mesher (meshc build mesher/)
  found: Both compile successfully.
  implication: Fix compiles cleanly.

- timestamp: 2026-02-15T00:15:00Z
  checked: Runtime test - ran mesher for 35 seconds
  found: Binary runs continuously: 6 load monitor ticks, 3 health checks, 1 alert evaluator check. No crash. Process still running at kill time.
  implication: Fix is verified - no bus error.

## Resolution

root_cause: Same as previous debug session (mesher-bus-error.md). The user was running a stale binary compiled with a pre-fix compiler. The root cause is TargetData::create("") in codegen using LLVM's default x86-32 data layout instead of the actual arm64-apple-darwin target layout, causing incorrect struct size computation for service state types. This leads to heap truncation of service state in make_tuple, followed by SIGBUS when the service loop tries to load a full struct from a truncated/invalid pointer.
fix: Already applied in previous session - all production TargetData::create("") replaced with self.target_machine.get_target_data(). User needs to rebuild: `npm run build:compiler && npm run build:mesher` (or just `npm run dev` which rebuilds automatically).
verification: Rebuilt compiler + mesher, ran for 35 seconds with full timer activity. No crash.
files_changed: []
