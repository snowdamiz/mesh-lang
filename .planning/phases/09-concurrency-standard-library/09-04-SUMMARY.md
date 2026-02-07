---
phase: "09"
plan: "04"
subsystem: "concurrency"
tags: [job, async, await, parallel-map, actor-spawn, runtime]
dependency-graph:
  requires: ["09-02", "09-03"]
  provides: ["job-runtime", "job-codegen", "async-task-pattern"]
  affects: ["09-05"]
tech-stack:
  added: []
  patterns: ["JOB_RESULT_TAG sentinel (u64::MAX-1) for job/exit discrimination", "closure-splitting codegen for HOF runtime intrinsics"]
key-files:
  created:
    - crates/snow-rt/src/actor/job.rs
  modified:
    - crates/snow-rt/src/actor/mod.rs
    - crates/snow-codegen/src/codegen/intrinsics.rs
    - crates/snow-codegen/src/mir/lower.rs
decisions:
  - id: "09-04-01"
    description: "JOB_RESULT_TAG = u64::MAX - 1 distinguishes job results from EXIT_SIGNAL_TAG = u64::MAX"
  - id: "09-04-02"
    description: "Job.await returns SnowResult (tag 0 = Ok, tag 1 = Err) matching existing Result layout"
  - id: "09-04-03"
    description: "Job entry function packs fn_ptr/env_ptr/caller_pid into GC-heap args buffer"
  - id: "09-04-04"
    description: "Job.map spawns one actor per list element, awaits all in order"
metrics:
  duration: "6min"
  completed: "2026-02-07"
---

# Phase 9 Plan 04: Job (Async Task) Runtime Summary

Job async task abstraction with runtime functions and LLVM codegen wiring for spawn, await, timeout, and parallel map.

## Task Commits

| # | Task | Commit | Key Changes |
|---|------|--------|-------------|
| 1 | Implement Job runtime functions | 6886a5b | job.rs: snow_job_async/await/await_timeout/map + pub mod job in mod.rs |
| 2 | Wire Job runtime into LLVM codegen | 61dcc8d | intrinsics.rs declarations + map_builtin_name fixes + known_functions |

## What Was Built

### Job Runtime (crates/snow-rt/src/actor/job.rs)

Four extern "C" runtime functions for the Job async task pattern:

**snow_job_async(fn_ptr, env_ptr) -> u64**
- Gets caller PID, packs [fn_ptr, env_ptr, caller_pid] into GC-heap args
- Spawns job actor via scheduler with `job_entry` as coroutine entry
- Job actor links to caller, calls fn_ptr(env_ptr), sends [JOB_RESULT_TAG][result], exits normally
- Returns spawned job PID

**snow_job_await(job_pid) -> ptr**
- Calls snow_actor_receive(-1) to block indefinitely
- Decodes message: JOB_RESULT_TAG -> Ok(value), EXIT_SIGNAL_TAG -> Err(reason)
- Returns heap-allocated SnowResult (tag 0 = Ok, tag 1 = Err)

**snow_job_await_timeout(job_pid, timeout_ms) -> ptr**
- Same as await but with timeout via snow_actor_receive(timeout_ms)
- Returns Err("timeout") if timeout expires

**snow_job_map(list_ptr, fn_ptr, env_ptr) -> ptr**
- Iterates input list, spawns map_job_entry actor per element
- Each actor calls fn_ptr(env_ptr, element) and sends result to caller
- Awaits all in order, builds result list of SnowResult values

### LLVM Codegen Wiring

**Intrinsics (intrinsics.rs):**
- snow_job_async: fn(ptr, ptr) -> i64
- snow_job_await: fn(i64) -> ptr
- snow_job_await_timeout: fn(i64, i64) -> ptr
- snow_job_map: fn(ptr, ptr, ptr) -> ptr

**MIR Lower (lower.rs):**
- Fixed map_builtin_name: job_await_all/job_await_any replaced with job_await_timeout/job_map (matching type checker)
- Updated known_functions with correct signatures accounting for closure struct splitting

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed incorrect map_builtin_name stubs from Plan 03**
- **Found during:** Task 2
- **Issue:** Plan 03 added stubs job_await_all/job_await_any but type checker (Plan 02) defines Job.await_timeout and Job.map
- **Fix:** Replaced with job_await_timeout -> snow_job_await_timeout and job_map -> snow_job_map
- **Files modified:** crates/snow-codegen/src/mir/lower.rs
- **Commit:** 61dcc8d

**2. [Rule 1 - Bug] Fixed incorrect known_functions types for Job**
- **Found during:** Task 2
- **Issue:** Plan 03 stubs had wrong return types (Int for await should be Ptr since it returns SnowResult)
- **Fix:** Updated snow_job_async to (Ptr, Ptr) -> Int (closure split produces 2 ptrs, returns PID), snow_job_await to (Int) -> Ptr, added snow_job_await_timeout and snow_job_map
- **Files modified:** crates/snow-codegen/src/mir/lower.rs
- **Commit:** 61dcc8d

## Verification

- cargo build: PASS (full workspace compiles)
- cargo test: PASS (all tests pass including 6 new job tests)
- Job runtime functions linked and callable: PASS (208 snow-rt tests pass)
- Job module resolves through STDLIB_MODULES: PASS (Job already in list)
- Intrinsic declarations match runtime signatures: PASS

## Next Phase Readiness

Plan 04 complete. Job runtime is fully wired. Plan 05 (E2E integration tests) can verify end-to-end compilation and execution of Job.async/await/map patterns.

## Self-Check: PASSED
