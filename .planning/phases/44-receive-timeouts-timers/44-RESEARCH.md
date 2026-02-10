# Phase 44: Receive Timeouts & Timers - Research

**Researched:** 2026-02-09
**Domain:** Compiler codegen (LLVM IR), runtime actor scheduling, stdlib module registration
**Confidence:** HIGH

## Summary

Phase 44 requires two distinct subsystems: (1) completing the `receive ... after ms -> body` codegen gap, and (2) adding `Timer.sleep(ms)` and `Timer.send_after(pid, ms, msg)` stdlib primitives. The first is a narrow codegen fix -- parsing, type-checking, and MIR lowering for the `after` clause are already fully implemented; only the LLVM IR generation ignores the `timeout_body` (line 130 of `expr.rs` has `timeout_body: _`). The second builds on top of the first, since `Timer.sleep` is naturally implemented as a receive-with-timeout that has zero arms (empty mailbox wait).

The runtime already supports timeouts at the `snow_actor_receive(timeout_ms)` level: passing a positive value blocks up to that many milliseconds and returns null on timeout. The scheduler correctly handles Waiting/Ready state transitions with deadline checking. The codegen gap is that when `snow_actor_receive` returns null (timeout), the current code does not branch to the timeout body -- it falls through to dereference the null pointer, causing a segfault.

**Primary recommendation:** Implement RECV-01/RECV-02 first (receive after codegen ~20-30 lines), then TIMER-01/TIMER-02 as a second plan using the established stdlib module pattern (type registration in typeck, name mapping in lower.rs, runtime functions in snow-rt).

## Standard Stack

### Core (No New Dependencies)

| Component | Location | Purpose | Status |
|-----------|----------|---------|--------|
| snow_actor_receive(timeout_ms) | snow-rt/src/actor/mod.rs:315 | Runtime receive with timeout support | COMPLETE - already handles positive timeout_ms, returns null on expiry |
| AfterClause AST node | snow-parser/src/ast/expr.rs:690 | Parse tree for `after ms -> body` | COMPLETE |
| parse_after_clause | snow-parser/src/parser/expressions.rs:1362 | Parser for after clause | COMPLETE |
| infer_receive (after handling) | snow-typeck/src/infer.rs:6014-6028 | Type-checks timeout expr as Int, unifies body type with arms | COMPLETE |
| MirExpr::ActorReceive | snow-codegen/src/mir/mod.rs:263 | MIR node with timeout_ms and timeout_body fields | COMPLETE |
| lower_receive_expr | snow-codegen/src/mir/lower.rs:7138 | MIR lowering includes timeout_ms and timeout_body | COMPLETE |
| codegen_actor_receive | snow-codegen/src/codegen/expr.rs:1710 | LLVM IR generation -- **IGNORES timeout_body** | GAP |

### Supporting

| Component | Purpose | When to Use |
|-----------|---------|-------------|
| declare_intrinsics (intrinsics.rs) | Declare new runtime functions in LLVM module | For Timer.sleep and Timer.send_after |
| stdlib_modules() (infer.rs:210) | Register Timer module type signatures | For Timer.sleep and Timer.send_after |
| STDLIB_MODULES (lower.rs:7202) | Module name list for field access resolution | Add "Timer" |
| map_builtin_name (lower.rs) | Name mapping timer_sleep -> snow_timer_sleep | For Timer functions |
| known_functions (lower.rs) | MIR function type signatures | For Timer functions |

### Alternatives Considered

None. The architecture is fully established. Every stdlib module follows the same pattern. No design alternatives needed.

## Architecture Patterns

### Pattern 1: Receive-with-timeout Codegen (RECV-01)

**What:** Add null-pointer check after `snow_actor_receive(timeout_ms)` call and branch to timeout body when null.

**When to use:** When `timeout_ms` is present (i.e., the `after` clause exists in source).

**Current code (expr.rs:127-132):**
```rust
MirExpr::ActorReceive {
    arms,
    timeout_ms,
    timeout_body: _,   // <-- IGNORED
    ty,
} => self.codegen_actor_receive(arms, timeout_ms.as_deref(), ty),
```

**Required change:** Pass `timeout_body` to `codegen_actor_receive` and add null-check branching.

**Reference pattern (service loop at expr.rs:2373-2390):**
```rust
// Check for null (shutdown signal). If null, exit the loop.
let is_null = self.builder
    .build_is_null(msg_ptr, "msg_is_null")?;
self.builder
    .build_conditional_branch(is_null, timeout_bb, continue_bb)?;

// Timeout block: execute timeout body
self.builder.position_at_end(timeout_bb);
let timeout_val = self.codegen_expr(timeout_body)?;
// ... store to result_alloca, branch to merge

// Continue block: process message through arms
self.builder.position_at_end(continue_bb);
// ... existing arm processing
```

**Reference pattern (if-expression at expr.rs:1048-1106):**
The if-expression uses `result_alloca` + `build_conditional_branch` + phi-like merge. The receive timeout should use the same pattern: allocate result, branch on null, store from either timeout body or arm body, load from merge block.

### Pattern 2: Stdlib Module Registration (Timer module)

**What:** Register Timer.sleep and Timer.send_after following the established Math/Job module pattern.

**Steps (same as every other stdlib module):**

1. **Type checker (infer.rs:stdlib_modules):** Add Timer module with function signatures
2. **Type checker (infer.rs:STDLIB_MODULE_NAMES):** Add "Timer" to the list
3. **MIR lower (lower.rs:STDLIB_MODULES):** Add "Timer" to the const array
4. **MIR lower (lower.rs:map_builtin_name):** Add `"timer_sleep" => "snow_timer_sleep"` and `"timer_send_after" => "snow_timer_send_after"` mappings
5. **MIR lower (lower.rs:known_functions):** Add MIR type signatures
6. **Intrinsics (intrinsics.rs:declare_intrinsics):** Declare LLVM function types
7. **Runtime (snow-rt):** Implement `snow_timer_sleep` and `snow_timer_send_after` as extern "C" functions
8. **Runtime (snow-rt/src/lib.rs):** Re-export new functions

### Pattern 3: Timer.sleep Implementation

**What:** `Timer.sleep(ms)` suspends the current actor for `ms` milliseconds without blocking other actors.

**Implementation approach:** Use `snow_actor_receive(ms)` internally. Since no message is expected (the function is Timer.sleep, not a receive), it simply calls receive with the timeout and discards any message that happens to arrive. A simpler and cleaner approach: directly use the scheduler's yield mechanism with a deadline.

**Recommended implementation:**
```rust
#[no_mangle]
pub extern "C" fn snow_timer_sleep(ms: i64) {
    // Use snow_actor_receive(ms) which blocks for up to ms milliseconds.
    // Any message received is pushed back to the front of the mailbox.
    // If no message arrives, returns null -- exactly the sleep behavior.
    let msg_ptr = snow_actor_receive(ms);
    if !msg_ptr.is_null() {
        // A message arrived during sleep -- push it back to mailbox.
        // This is the key subtlety: sleep should not consume messages.
        push_message_back(msg_ptr);
    }
}
```

**Critical subtlety:** `Timer.sleep` must NOT consume messages from the mailbox. If a message arrives during the sleep period, it must be preserved. Two approaches:
- **Option A:** Implement sleep using the yield/deadline mechanism directly (bypass receive). Set state to Waiting, yield, check deadline on resume, repeat until expired.
- **Option B:** Use `snow_actor_receive(ms)` but push any received message back to the front of the mailbox.

**Recommendation:** Option A is cleaner -- it avoids the message-push-back complexity and is a more faithful sleep semantic. The implementation mirrors `snow_actor_receive` but without the mailbox check:

```rust
#[no_mangle]
pub extern "C" fn snow_timer_sleep(ms: i64) {
    if ms <= 0 { return; }
    let deadline = Instant::now() + Duration::from_millis(ms as u64);
    let my_pid = match stack::get_current_pid() { Some(pid) => pid, None => return };
    let sched = global_scheduler();
    let in_coroutine = stack::CURRENT_YIELDER.with(|c| c.get().is_some());
    if !in_coroutine {
        // Main thread: just sleep
        std::thread::sleep(Duration::from_millis(ms as u64));
        return;
    }
    loop {
        if let Some(proc_arc) = sched.get_process(my_pid) {
            proc_arc.lock().state = ProcessState::Waiting;
        }
        stack::yield_current();
        if Instant::now() >= deadline { break; }
    }
    if let Some(proc_arc) = sched.get_process(my_pid) {
        proc_arc.lock().state = ProcessState::Ready;
    }
}
```

### Pattern 4: Timer.send_after Implementation

**What:** `Timer.send_after(pid, ms, msg)` schedules a message to be sent to `pid` after `ms` milliseconds.

**Implementation approach:** Spawn a lightweight helper actor that sleeps for `ms` then sends `msg` to `pid`. This is the Erlang approach and fits naturally with the actor model.

```rust
#[no_mangle]
pub extern "C" fn snow_timer_send_after(target_pid: u64, ms: i64, msg_ptr: *const u8, msg_size: u64) {
    // Spawn a timer actor that:
    // 1. Sleeps for ms milliseconds
    // 2. Sends msg to target_pid
    // 3. Exits
    // This is fire-and-forget; the timer actor is independent.
}
```

**Key design question:** The signature from Snow's perspective is `Timer.send_after(pid, ms, msg)` where `msg` is the message value. The runtime needs the raw bytes. The codegen must serialize the message the same way `send` does (into a buffer with type_tag + data), then pass that buffer to the runtime function. This is the same pattern as `snow_actor_send`.

### Anti-Patterns to Avoid

- **Blocking the scheduler thread:** Timer.sleep must yield to the scheduler, not call `std::thread::sleep` in a coroutine context. Using `std::thread::sleep` would block the entire worker thread, preventing other actors from running.
- **Consuming messages during sleep:** Timer.sleep must NOT pop messages from the mailbox. A sleeping actor should still accumulate messages.
- **Using OS timers:** Do NOT use `std::thread::spawn` for each timer -- that defeats the lightweight actor model. Use the actor system itself (spawn a timer actor) or the cooperative yield mechanism.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Null-check after receive | Custom null detection | `build_is_null` + `build_conditional_branch` (Inkwell API) | Already used in service loop codegen |
| Timer scheduling | OS timer threads | Spawn a timer actor (snow_actor_spawn) | Consistent with actor model; no new threading |
| Sleep without message consumption | Complex mailbox manipulation | Direct yield loop with deadline check | Clean; avoids touching mailbox state |
| Stdlib module registration | Custom resolution paths | Follow Math/Job module pattern exactly | 7 touchpoints, all well-established |

**Key insight:** The entire infrastructure for this phase already exists. The codegen gap is literally ~20-30 lines. The Timer module follows a pattern used by 16+ existing stdlib modules.

## Common Pitfalls

### Pitfall 1: Segfault on Timeout (The Main Bug)
**What goes wrong:** Without the null-check, `snow_actor_receive(timeout_ms)` returns null on timeout, then the codegen dereferences it via `build_gep` at offset 16 (data_ptr), causing a segfault.
**Why it happens:** `timeout_body: _` in the match arm discards the timeout body, so no branching logic is generated.
**How to avoid:** Add null-check after `snow_actor_receive`, branch to timeout body when null.
**Warning signs:** Any `receive ... after` clause in user code will segfault at runtime.

### Pitfall 2: Timer.sleep Consuming Messages
**What goes wrong:** If Timer.sleep is implemented via `snow_actor_receive(ms)`, any message that arrives during the sleep is consumed and lost.
**Why it happens:** `snow_actor_receive` pops from the mailbox before returning.
**How to avoid:** Implement sleep using direct yield+deadline loop, bypassing the mailbox entirely.
**Warning signs:** Messages sent to a sleeping actor are silently dropped.

### Pitfall 3: Timer.send_after Message Serialization
**What goes wrong:** The msg argument in `Timer.send_after(pid, ms, msg)` needs to be serialized the same way `send` serializes messages. If the codegen doesn't serialize correctly, the receiver gets garbage data.
**Why it happens:** `send` in the compiler constructs a message buffer with type_tag and data; `Timer.send_after` must do the same.
**How to avoid:** The codegen for `Timer.send_after` should serialize the message the same way `send` does, then pass the raw buffer to the runtime function. Alternatively, the runtime function can receive the pre-serialized buffer (simpler).
**Warning signs:** `Timer.send_after` messages arrive corrupted or with wrong type_tags.

### Pitfall 4: Scheduler Wake Timing for Sleep
**What goes wrong:** A sleeping actor sets state to Waiting and yields. The scheduler only resumes it when its state is changed back to Ready. Without a wake mechanism, the actor sleeps forever.
**Why it happens:** The scheduler doesn't have a timer-based wake system; it relies on message sends to change Waiting->Ready.
**How to avoid:** The sleep loop should NOT set state to Waiting indefinitely. Instead, after each yield, check the deadline. The worker thread will naturally resume suspended coroutines in its loop, giving the actor a chance to check its deadline.
**Warning signs:** Timer.sleep(100) blocks forever because no message wakes the actor.

**CRITICAL INSIGHT:** The scheduler worker loop at scheduler.rs:398 skips Waiting processes. So if Timer.sleep sets the process to Waiting and yields, it will NEVER be resumed unless a message arrives. The sleep implementation must either:
- (a) NOT set state to Waiting -- just yield normally (state stays Ready/Running), letting the scheduler resume it on next cycle, then check deadline
- (b) Set state to Waiting BUT also register a wake-up timer that transitions the state back to Ready after ms milliseconds

**Recommendation:** Option (a) is simpler. The actor stays Ready, gets scheduled normally, checks `Instant::now() >= deadline`, and if not expired, yields again. This does burn a few extra context switches but is completely safe and simple. The overhead is minimal because the scheduler's backoff logic reduces idle polling.

### Pitfall 5: Return Type of Receive-with-After
**What goes wrong:** The timeout body must return the same type as the receive arms. The type checker already enforces this (infer.rs:6022-6024), but the codegen must also correctly handle the LLVM type -- the result_alloca must be typed to match both branches.
**Why it happens:** Type mismatch between timeout body value and arm body value at the LLVM level.
**How to avoid:** Use the same result type (`ty` parameter) for both branches. The type checker guarantees consistency.
**Warning signs:** LLVM verifier errors about type mismatches in phi nodes or stores.

## Code Examples

### Example 1: Snow Source -- Receive with After

```snow
actor Pinger do
  receive do
    msg -> println("Got: " <> msg)
  after 5000 ->
    println("Timed out after 5 seconds")
  end
end
```

### Example 2: Snow Source -- Timer.sleep

```snow
actor Worker do
  println("Starting work...")
  Timer.sleep(1000)
  println("Finished after 1 second")
end
```

### Example 3: Snow Source -- Timer.send_after

```snow
actor Reminder(self_pid) do
  Timer.send_after(self_pid, 3000, "wake up")
  receive do
    msg -> println("Reminder: " <> msg)
  end
end
```

### Example 4: Codegen Pattern for Null-Check (from service loop)

```rust
// Already exists at expr.rs:2376-2390
let is_null = self.builder
    .build_is_null(msg_ptr, "msg_is_null")
    .map_err(|e| e.to_string())?;
self.builder
    .build_conditional_branch(is_null, exit_bb, continue_bb)
    .map_err(|e| e.to_string())?;
```

### Example 5: Stdlib Module Registration Pattern (from Math)

```rust
// infer.rs -- Type signatures
let mut timer_mod = HashMap::new();
timer_mod.insert("sleep".to_string(), Scheme::mono(Ty::fun(vec![Ty::int()], Ty::Tuple(vec![]))));
timer_mod.insert("send_after".to_string(), Scheme { /* ... */ });
modules.insert("Timer".to_string(), timer_mod);

// lower.rs -- STDLIB_MODULES
const STDLIB_MODULES: &[&str] = &[..., "Timer"];

// lower.rs -- map_builtin_name
"timer_sleep" => "snow_timer_sleep".to_string(),
"timer_send_after" => "snow_timer_send_after".to_string(),
```

## Existing Infrastructure Analysis

### What Is Already Done (COMPLETE)

| Component | File | Lines | Status |
|-----------|------|-------|--------|
| `after` keyword in lexer | snow-common/src/token.rs:31,197 | 2 | COMPLETE |
| AFTER_KW/AFTER_CLAUSE syntax kinds | snow-parser/src/syntax_kind.rs:25,309 | 2 | COMPLETE |
| parse_receive_expr (handles after) | snow-parser/src/parser/expressions.rs:1297 | 40 | COMPLETE |
| parse_after_clause | snow-parser/src/parser/expressions.rs:1362 | 17 | COMPLETE |
| AfterClause AST node | snow-parser/src/ast/expr.rs:690-702 | 13 | COMPLETE |
| ReceiveExpr.after_clause() | snow-parser/src/ast/expr.rs:661 | 3 | COMPLETE |
| infer_receive (timeout type check) | snow-typeck/src/infer.rs:6014-6028 | 15 | COMPLETE |
| MirExpr::ActorReceive (timeout fields) | snow-codegen/src/mir/mod.rs:263-272 | 10 | COMPLETE |
| lower_receive_expr (timeout lowering) | snow-codegen/src/mir/lower.rs:7163-7170 | 8 | COMPLETE |
| Formatter AFTER_CLAUSE handling | snow-fmt/src/walker.rs:1542-1544 | 3 | COMPLETE |
| snow_actor_receive(timeout_ms) runtime | snow-rt/src/actor/mod.rs:315-419 | 105 | COMPLETE |
| Scheduler Waiting/Ready transitions | snow-rt/src/actor/scheduler.rs | -- | COMPLETE |

### What Is The Gap (RECV-01 codegen)

| Component | File | Lines | What's Missing |
|-----------|------|-------|----------------|
| Match arm discards timeout_body | snow-codegen/src/codegen/expr.rs:130 | 1 | `timeout_body: _` |
| codegen_actor_receive signature | snow-codegen/src/codegen/expr.rs:1710 | 1 | No timeout_body parameter |
| No null-check after receive | snow-codegen/src/codegen/expr.rs:1728-1736 | 0 | Missing `build_is_null` + branch |
| No timeout body codegen | -- | 0 | Missing codegen of timeout body |

**Estimated size of RECV-01 codegen fix:** ~20-30 lines of Rust in `codegen_actor_receive`.

### What Is Needed (Timer module -- TIMER-01, TIMER-02)

| Component | File | Estimated Lines | Pattern Reference |
|-----------|------|----------------|-------------------|
| Timer type signatures | snow-typeck/src/infer.rs | ~8 | Math module at line 485 |
| Timer in STDLIB_MODULE_NAMES | snow-typeck/src/infer.rs:512 | 1 | Add "Timer" to list |
| Timer in STDLIB_MODULES | snow-codegen/src/mir/lower.rs:7202 | 1 | Add "Timer" to list |
| timer_sleep/timer_send_after name mapping | snow-codegen/src/mir/lower.rs:map_builtin_name | 2 | "math_abs" pattern |
| Timer known_functions | snow-codegen/src/mir/lower.rs | 2 | Job functions pattern |
| LLVM declarations | snow-codegen/src/codegen/intrinsics.rs | ~6 | Job functions pattern |
| snow_timer_sleep runtime | snow-rt (new file or actor/mod.rs) | ~30 | Yield+deadline loop |
| snow_timer_send_after runtime | snow-rt | ~40 | Spawn timer actor |
| Re-exports in lib.rs | snow-rt/src/lib.rs | 2 | Existing pattern |
| E2E tests | snowc/tests/ | ~80 | e2e_actors.rs pattern |

## State of the Art

| Component | Current State | What This Phase Adds |
|-----------|--------------|---------------------|
| Receive expression | Full pipeline except codegen timeout branch | Null-check + timeout body codegen |
| Timer primitives | None | Timer.sleep, Timer.send_after |
| Actor sleep | Only via receive with no arms (hacky) | First-class Timer.sleep |
| Delayed messages | Not possible | Timer.send_after |

## Open Questions

1. **Timer.send_after message serialization**
   - What we know: `send` in the compiler serializes messages into a buffer at codegen time. Timer.send_after needs the same serialization.
   - What's unclear: Should the codegen pre-serialize the message and pass raw bytes to the runtime, or should the runtime receive a typed value?
   - Recommendation: Pre-serialize at codegen (same as `send`), pass `(target_pid, ms, msg_ptr, msg_size)` to runtime. This matches the existing `snow_actor_send` signature pattern. The codegen for `Timer.send_after(pid, ms, msg)` should serialize `msg` the same way send serializes, then call `snow_timer_send_after(pid, ms, msg_ptr, msg_size)`.

2. **Timer.send_after return type**
   - What we know: Erlang's `send_after` returns a timer reference that can be cancelled. Snow doesn't have a cancel mechanism.
   - Recommendation: Return `Unit` (fire-and-forget). Timer cancellation is a future enhancement if ever needed.

3. **Timer.sleep in non-actor context**
   - What we know: Timer.sleep should only be called inside an actor (like receive). Calling from main thread would use `std::thread::sleep`.
   - Recommendation: Support both contexts (main thread uses `thread::sleep`, actor uses yield loop). This is the same pattern as `snow_actor_receive`.

## Dependency Ordering

```
RECV-01 (receive after codegen) ──> RECV-02 (already done in typeck)
                                          |
                                          v
TIMER-01 (Timer.sleep) ──────────> uses receive-with-timeout internally
                                          |
                                          v
TIMER-02 (Timer.send_after) ──────> uses Timer.sleep + send
```

**Plan ordering:**
- **Plan 44-01:** RECV-01 + RECV-02 (receive after codegen + verify type checking)
- **Plan 44-02:** TIMER-01 + TIMER-02 (Timer.sleep + Timer.send_after)

RECV-02 is already complete in the type checker but needs e2e verification. Include it in Plan 01.

## Sources

### Primary (HIGH confidence)
- snow-codegen/src/codegen/expr.rs:127-132 -- Confirmed timeout_body is discarded (`_`)
- snow-codegen/src/codegen/expr.rs:1710-1818 -- codegen_actor_receive implementation verified
- snow-rt/src/actor/mod.rs:315-419 -- snow_actor_receive timeout handling verified
- snow-typeck/src/infer.rs:5962-6031 -- infer_receive with after clause verified
- snow-codegen/src/mir/lower.rs:7138-7178 -- MIR lowering verified complete
- snow-parser/src/parser/expressions.rs:1296-1379 -- Parser verified complete
- snow-codegen/src/codegen/expr.rs:2373-2390 -- Service loop null-check pattern verified
- snow-codegen/src/codegen/expr.rs:1048-1106 -- If-expression branching pattern verified
- snow-rt/src/actor/scheduler.rs:398-407 -- Scheduler skips Waiting processes (critical for sleep design)
- snow-rt/src/actor/job.rs -- Job module pattern for stdlib registration

### Secondary (MEDIUM confidence)
- Erlang/OTP receive-after semantics -- well-known BEAM pattern, Snow follows this model

## Metadata

**Confidence breakdown:**
- Receive after codegen gap: HIGH -- directly verified by reading source; `timeout_body: _` on line 130 is definitive
- Timer.sleep implementation: HIGH -- yield mechanism understood from scheduler source; only question is Waiting vs Ready state
- Timer.send_after implementation: HIGH -- spawn-a-timer-actor pattern is standard; message serialization follows send pattern
- Pitfall analysis: HIGH -- null-dereference path traced through codegen; scheduler Waiting-skip behavior confirmed in source

**Research date:** 2026-02-09
**Valid until:** 2026-03-09 (stable infrastructure, no expected changes)
