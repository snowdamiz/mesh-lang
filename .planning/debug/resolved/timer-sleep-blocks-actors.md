---
status: resolved
trigger: "timer-sleep-blocks-actors"
created: 2026-02-10T00:00:00Z
updated: 2026-02-10T00:00:00Z
---

## Current Focus

hypothesis: CONFIRMED - Test timeout too tight due to macOS Gatekeeper serialized first-run delays
test: N/A - verified fix
expecting: N/A
next_action: Archive session

## Symptoms

expected: Timer.sleep in one actor should not block other actors from running concurrently
actual: The compiled binary times out after 5 seconds, suggesting Timer.sleep IS blocking other actors
errors: thread 'test_timer_sleep_does_not_block_other_actors' panicked at crates/snowc/tests/e2e_concurrency_stdlib.rs:57:21: Binary timed out after 5 seconds
reproduction: cargo test -p snowc --test e2e_concurrency_stdlib test_timer_sleep_does_not_block_other_actors
started: Unknown

## Eliminated

- hypothesis: Timer.sleep blocks the thread instead of yielding to the scheduler
  evidence: snow_timer_sleep uses a yield loop (line 429-455 of actor/mod.rs). Coroutine actors yield and check deadline. Verified by running compiled binary directly - "fast" prints before "slow", confirming concurrent execution works.
  timestamp: 2026-02-10

- hypothesis: Scheduler doesn't resume sleeping actors
  evidence: Workers resume all non-Waiting suspended coroutines on each loop iteration. Timer.sleep keeps state as Ready (not Waiting), so the scheduler always resumes it.
  timestamp: 2026-02-10

- hypothesis: Scheduler shutdown takes too long
  evidence: Added timing instrumentation to snow_rt_run_scheduler. Shutdown signal takes 11us, wait completes in 148us. The ~1s overhead is NOT in scheduler shutdown.
  timestamp: 2026-02-10

## Evidence

- timestamp: 2026-02-10
  checked: Running compiled binary directly (outside test infrastructure)
  found: Binary produces correct output ("fast" then "slow") in ~0.5s (after Gatekeeper cache)
  implication: The concurrency logic works correctly. Timer.sleep does NOT block other actors.

- timestamp: 2026-02-10
  checked: First vs subsequent binary executions on macOS
  found: First run of a new binary takes ~1s extra, subsequent runs take 4-5ms. This is macOS Gatekeeper/XProtect first-run scanning.
  implication: Every test invocation gets a ~1s penalty because it compiles to a fresh temp directory.

- timestamp: 2026-02-10
  checked: 5 distinct binaries in parallel
  found: Gatekeeper serializes checks: 1st binary 1.4s, 2nd 2.4s, 3rd 3.4s, 4th 4.5s, 5th 5.6s. Each waits ~1s for its turn.
  implication: With 11 test binaries in parallel, worst-case startup delay is ~11 seconds.

- timestamp: 2026-02-10
  checked: Runtime instrumentation (snow_rt_init_actor and snow_rt_run_scheduler)
  found: Scheduler init: 777us. Shutdown: 148us. Total runtime overhead < 1ms.
  implication: The 1s overhead is macOS Gatekeeper, not the runtime.

- timestamp: 2026-02-10
  checked: Tests in isolation vs parallel
  found: test_timer_sleep_does_not_block_other_actors passes in isolation (~4s) but fails with 10s timeout when 11 tests run in parallel.
  implication: Parallel Gatekeeper serialization causes cumulative delay exceeding original 5s timeout.

## Resolution

root_cause: macOS Gatekeeper/XProtect serializes first-run security checks for new binaries. Each test compiles to a unique temp directory, creating a distinct binary that requires its own Gatekeeper check (~1s). When 11 tests run in parallel, the Gatekeeper queue causes the Nth binary to wait up to N seconds before even starting. The original 5-second timeout was insufficient for this queuing effect. Timer.sleep concurrency itself works correctly.
fix: (1) Reduced unnecessary sleep times in test programs (200ms->50ms, 500ms->150ms, etc.) to minimize actual execution time. (2) Increased all timeouts from 5-10s to 30s to accommodate parallel Gatekeeper delays with comfortable margin.
verification: All 11 tests pass consistently in parallel (3 consecutive runs: 16.03s, 15.16s, 15.31s). The originally failing test passes both in isolation (4.08s) and in parallel.
files_changed:
  - crates/snowc/tests/e2e_concurrency_stdlib.rs
