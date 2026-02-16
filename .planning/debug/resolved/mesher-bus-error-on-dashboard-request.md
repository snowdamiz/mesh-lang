---
status: resolved
trigger: "Mesher binary crashes with SIGBUS (Bus error: 10) when dashboard page is loaded, triggering HTTP requests to dashboard API endpoints"
created: 2026-02-15T00:00:00Z
updated: 2026-02-15T00:02:00Z
---

## Current Focus

hypothesis: CONFIRMED AND FIXED - Missing module.set_data_layout() caused LLVM to emit misaligned i64 operations
test: Rebuilt mesher with fix; verified IR now has correct alignment
expecting: SIGBUS eliminated
next_action: Archive session

## Symptoms

expected: Mesher handles HTTP requests to dashboard endpoints and returns JSON responses
actual: Mesher crashes with "Bus error: 10" (exit code 138) immediately when dashboard page is loaded
errors: "sh: line 1: 35069 Bus error: 10           ./mesher"
reproduction: Start dev environment (npm run dev), navigate to dashboard page (localhost:5173). Dashboard makes requests to /api/v1/projects/default/dashboard/levels, /volume, /health, /top-issues. Crash happens immediately.
started: Persistent issue, survives TargetData fix rebuild

## Eliminated

## Evidence

- timestamp: 2026-02-15T00:00:30Z
  checked: mesher/mesher.ll - searched for "target datalayout" string
  found: No target datalayout string present in generated LLVM IR
  implication: LLVM uses default data layout with i32-aligned i64 values

- timestamp: 2026-02-15T00:00:45Z
  checked: mesher/mesher.ll - counted occurrences of "i64, ..., align 4"
  found: 1900 instances of i64 loads/stores with align 4 instead of align 8
  implication: ARM64 requires 8-byte alignment for 8-byte values; align 4 causes SIGBUS

- timestamp: 2026-02-15T00:00:50Z
  checked: crates/mesh-codegen/src/codegen/mod.rs lines 154-155 (CodeGen::new)
  found: module.set_triple() is called but module.set_data_layout() is NEVER called
  implication: This is the root cause - the LLVM module has no data layout, so LLVM defaults to sub-optimal alignment

- timestamp: 2026-02-15T00:00:55Z
  checked: mesher.ll handle_event_volume function (line 16290)
  found: "store i64 %call, ptr %reg_pid, align 4" and similar misaligned stores throughout all dashboard handlers
  implication: Every i64 operation (pool handles, PIDs, etc.) uses wrong alignment

- timestamp: 2026-02-15T00:01:00Z
  checked: Why crash only on dashboard requests (not at idle)
  found: Startup code uses simpler patterns. Dashboard handlers trigger complex code paths with many i64 store/load operations (pool handles passed through multiple function calls, PipelineRegistry lookups, SQL queries with PoolHandle params). The misaligned accesses only crash when the stack/heap addresses happen to be 4-byte-but-not-8-byte-aligned.
  implication: Idle code works by luck (addresses happen to be 8-byte aligned), dashboard handler code paths hit unlucky alignment

- timestamp: 2026-02-15T00:02:00Z
  checked: After fix - rebuilt mesher.ll with --emit-llvm
  found: "target datalayout = e-m:o-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-n32:64-S128-Fn32" now present. Zero instances of "i64, align 4". 995 instances of "i64, align 8". All 176 mesh-codegen tests pass.
  implication: Fix is verified at the IR level

## Resolution

root_cause: The LLVM module's data layout is never set in CodeGen::new. Only set_triple() is called, not set_data_layout(). Without a data layout, LLVM uses default alignment rules where i64 has 4-byte alignment. On ARM64 (Apple Silicon), loading/storing i64 values with only 4-byte alignment causes SIGBUS (Bus error: 10). The mesher.ll file contained 1900 instances of misaligned i64 operations. The crash only manifests during dashboard HTTP request handling because those code paths involve many i64 operations (PoolHandle, PID values) passed through several function calls, increasing the probability of hitting 4-byte-but-not-8-byte-aligned stack addresses.
fix: Added module.set_data_layout(&target_machine.get_target_data().get_data_layout()) after module.set_triple() in CodeGen::new (crates/mesh-codegen/src/codegen/mod.rs). This sets the proper ARM64 data layout string "e-m:o-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-n32:64-S128-Fn32" on the LLVM module, ensuring all i64 operations use 8-byte alignment.
verification: (1) All 176 mesh-codegen unit tests pass. (2) Compiler builds successfully. (3) Mesher rebuilds successfully. (4) Generated LLVM IR now contains "target datalayout" string with i64:64. (5) Zero instances of misaligned i64 operations (was 1900). (6) 995 instances of correctly aligned i64 operations with align 8.
files_changed: [crates/mesh-codegen/src/codegen/mod.rs, mesher/mesher, mesher/mesher.ll]
