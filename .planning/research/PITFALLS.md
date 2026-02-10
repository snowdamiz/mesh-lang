# Pitfalls Research: v1.9 Stdlib & Ergonomics Features

**Domain:** Compiler feature addition -- math stdlib, error propagation operator, receive timeouts, timer primitives, collection operations, and tail-call elimination for a statically-typed, functional-first, LLVM-compiled language with actor concurrency
**Researched:** 2026-02-09
**Confidence:** HIGH (based on direct Snow codebase analysis of 73,384 lines across 11 crates, LLVM 21 + Inkwell 0.8 documentation, LLVM Language Reference for musttail semantics, and established compiler engineering knowledge)

**Scope:** This document covers pitfalls specific to adding 6 features to the Snow compiler for v1.9. Each feature is analyzed against Snow's existing architecture: extern "C" runtime ABI, uniform u64 storage, per-actor GC with conservative stack scanning, callback-based dispatch for generic operations, single LLVM module via MIR merge, and immutable collection semantics.

---

## Critical Pitfalls

Mistakes that cause rewrites, soundness holes, or silent codegen bugs.

---

### Pitfall 1: Math FFI Via libm Breaks Cross-Platform Linking

**What goes wrong:**
The natural approach is to call C libm functions (`sqrt`, `pow`, `sin`, etc.) directly from LLVM IR using `declare double @sqrt(double)`. On macOS, these functions are available in `libSystem` (automatically linked). On Linux, they require explicit `-lm` linkage. The current linker driver in `link.rs` (line 46-52) invokes `cc` with only `-lsnow_rt` -- it does NOT pass `-lm`. Math functions will link fine on macOS but produce "undefined symbol" errors on Linux.

A subtler issue: some libm functions have different precision guarantees across platforms. `pow(2.0, 0.5)` may return slightly different results on glibc vs musl vs macOS's libm.

**Why it happens:**
macOS bundles libm into libSystem, masking the missing `-lm` flag. Developers on macOS never see the failure until someone tries Linux.

**Consequences:**
- All Snow programs using math functions fail to compile on Linux
- Users discover this post-deployment, not during development
- If using the Rust `libm` crate instead of system libm, the ABI boundary is different -- Rust functions are not `extern "C"` by default

**Prevention:**
1. Add `-lm` to the linker invocation in `link.rs` unconditionally (it's harmless on macOS)
2. Use LLVM intrinsics where available (`llvm.sqrt`, `llvm.pow`, `llvm.fabs`, `llvm.floor`, `llvm.ceil`, `llvm.round`) -- these are lowered by LLVM to optimal platform code and require NO library linking for the subset LLVM supports natively
3. For functions LLVM does not have intrinsics for (e.g., `atan2`, trigonometric functions), declare them as external in `intrinsics.rs` and ensure `-lm` in `link.rs`
4. Add a Linux CI check that exercises math functions

**Detection:**
- Linking errors on Linux with "undefined reference to `sqrt`"
- E2E tests pass on macOS but fail on Linux

**Confidence:** HIGH -- verified by reading `link.rs` directly; the `-lm` flag is absent.

---

### Pitfall 2: ? Operator Requires Early Return, Which Snow's Expression-Oriented Codegen Does Not Naturally Support

**What goes wrong:**
The `?` operator on `expr?` must: (1) evaluate `expr`, (2) check if it's `Ok(v)` or `Err(e)`, (3) if `Ok`, unwrap to `v` and continue, (4) if `Err`, return `Err(e)` from the enclosing function immediately. Step 4 is the problem.

Snow's codegen (`compile_function` in mod.rs:345-472) compiles the body as a single expression and emits the return at the end. `MirExpr::Return` exists (codegen/expr.rs:1359-1369) and correctly emits `build_return`, but it leaves the builder in a terminated block. If `?` appears in the middle of an expression chain (e.g., `let x = foo()? + bar()?`), the second `?` will try to emit code after a return instruction -- LLVM will reject this with a verification error ("Terminator found in the middle of a basic block").

The existing `Return` codegen returns a dummy value after emitting the return instruction (line 1368), but this only works if it's the last thing evaluated. In a `?` expression, control flow must branch: one path returns early, one path continues.

**Why it happens:**
The `?` operator is syntactically an expression but semantically introduces control flow (branching + potential early return). This is fundamentally different from Snow's current expressions, all of which produce values without returning from the function.

**Consequences:**
- Naive implementation (lowering `?` to `match` + `Return`) produces invalid LLVM IR when `?` is nested or appears mid-expression
- LLVM module verification fails with "Terminator found in the middle of a basic block"
- If worked around by splitting into multiple basic blocks, every `?` usage creates 3 blocks (check, early-return, continue), significantly complicating codegen for common patterns like `let x = foo()? + bar()?`

**Prevention:**
Lower `?` in MIR as a `Match` with two arms, where the `Err` arm contains a `Return`. The codegen for `Match` already handles branching to separate basic blocks with a merge block. The key insight: the `Err` arm's block will be terminated by the return, so the merge block's phi node should only have the `Ok` arm as a predecessor. This matches how the existing pattern match codegen handles `Never`-typed arms.

Concretely, `expr?` should lower to:
```
match expr do
  Ok(v) -> v
  Err(e) -> return Err(e)
end
```

This reuses the existing pattern compilation infrastructure (pattern/compile.rs) and the existing `ConstructVariant` + `Return` MIR nodes. No new codegen primitives needed.

**Detection:**
- LLVM verification failure: "Terminator found in the middle of a basic block"
- Crash when `?` appears anywhere other than the final expression in a function

**Confidence:** HIGH -- verified by reading codegen_return (expr.rs:1359-1369) and compile_function (mod.rs:345-472); the control flow issue is structural.

---

### Pitfall 3: Tail-Call Elimination With musttail Requires Matching Caller/Callee Signatures

**What goes wrong:**
LLVM's `musttail` marker (required for guaranteed TCE, available via Inkwell 0.8's `set_tail_call_kind` on LLVM 21) enforces strict rules: the caller and callee must have identical calling conventions, matching parameter types, matching return types, and the call must immediately precede a `ret` instruction. Snow functions use the default C calling convention and pass all values as `i64`/`ptr`/`f64`. For self-recursion (`fn factorial(n) = if n <= 1 then 1 else n * factorial(n - 1)`), the signatures match trivially.

For mutual recursion (`fn even(n) = if n == 0 then true else odd(n - 1)` / `fn odd(n) = if n == 0 then false else even(n - 1)`), signatures must also match. If `even` returns `Bool` (i8) and `odd` returns `Bool` (i8), the LLVM types match. But if one function returns `Int` (i64) and the other returns `Bool` (i8), `musttail` will fail with a fatal backend error: "failed to perform tail call elimination on a call site marked musttail."

More critically: any operation between the tail call and the return invalidates `musttail`. Snow's codegen inserts `snow_reduction_check()` after every function call (expr.rs:634-635). A tail call followed by a reduction check followed by a return is NOT a valid musttail position.

**Why it happens:**
1. LLVM `musttail` is a hard contract -- if it cannot be honored, compilation fails (not silently dropped)
2. Reduction checks are inserted unconditionally after all calls for actor scheduler fairness
3. Snow's uniform u64 storage helps (most things are i64), but Bool (i8) and Float (f64) break type matching

**Consequences:**
- If musttail is applied to calls with mismatched return types: fatal LLVM backend error, compiler crash
- If musttail is applied to calls followed by reduction checks: LLVM verification error
- If using `tail` (soft hint) instead of `musttail`: LLVM may or may not optimize it, no guarantee, stack overflow on deep mutual recursion

**Prevention:**
1. Use the loop-transformation approach for self-recursion (rewrite `f(args) { ... f(new_args) }` to `loop { args = new_args }`). This is guaranteed, platform-independent, and does not require musttail. LLVM's own TailRecursionElimination pass does this, but only at `-O2+` -- Snow needs it at `-O0` too.
2. For mutual recursion, either:
   a. Normalize all function return types to i64 (the uniform storage already does this for most types), or
   b. Use a trampoline: rewrite mutually recursive functions into a single dispatcher function with a loop, converting tail calls into `continue` with updated arguments
3. Skip `snow_reduction_check()` insertion for calls in tail position -- the callee will do its own reduction check
4. If using `musttail`, verify at MIR level that caller/callee signatures match before emitting the marker

**Detection:**
- LLVM fatal error: "failed to perform tail call elimination on a call site marked musttail"
- Stack overflow in programs with mutual recursion that "should" be TCE'd
- Missing reduction checks causing actor starvation (if checks are skipped too aggressively)

**Confidence:** HIGH -- verified Inkwell 0.8 has `set_tail_call_kind` on LLVM 21; verified reduction check insertion in expr.rs:634; LLVM musttail restrictions verified via LLVM Language Reference.

---

### Pitfall 4: Receive Timeout After Clause -- Codegen Ignores timeout_body

**What goes wrong:**
The receive timeout infrastructure is almost complete. The parser handles `after` clauses (lower.rs:7160-7168). MIR has `ActorReceive { arms, timeout_ms, timeout_body }` (mir/mod.rs:263-269). The runtime's `snow_actor_receive(timeout_ms)` correctly returns null on timeout (actor/mod.rs:315-399). BUT: the codegen explicitly discards `timeout_body` with `timeout_body: _` (codegen/expr.rs:129) and never generates code for what should happen when the timeout fires.

Currently, when `snow_actor_receive` returns null (timeout), the codegen proceeds to load from the null pointer (data_ptr = msg_ptr + 16 where msg_ptr is null), causing a segfault.

**Why it happens:**
The receive codegen (expr.rs:1540-1648) was written for infinite-wait receives. It unconditionally loads message data from the returned pointer without checking for null. The timeout path was explicitly deferred: `timeout_body: _` on line 129.

**Consequences:**
- Any program using `receive ... after 1000 -> :timeout end` will segfault when the timeout fires
- The null pointer dereference occurs in LLVM-generated code, producing an opaque signal 11 crash
- Users cannot write timeout-based patterns (heartbeat checks, retry loops, supervision timeouts)

**Prevention:**
After calling `snow_actor_receive(timeout_val)`, check if the returned pointer is null:
1. `icmp eq msg_ptr, null` -- branch to timeout block if null, normal block if non-null
2. Normal block: existing message extraction + arm matching code
3. Timeout block: codegen the `timeout_body` expression
4. Both blocks branch to a merge block with a phi node for the result value
5. The result type of the timeout_body must match the result type of the receive arms -- enforce this in typeck

This is structurally similar to if/else codegen, which Snow already handles correctly.

**Detection:**
- Segfault (signal 11) when receive timeout fires
- E2E test: `receive do msg -> msg after 100 -> "timeout" end` -- crashes instead of returning "timeout"

**Confidence:** HIGH -- verified by reading codegen/expr.rs:126-131 where `timeout_body: _` explicitly discards the body, and expr.rs:1557-1566 where null is never checked.

---

### Pitfall 5: Collection sort() Requires Comparator Callback But Comparator Type Depends on Element Type

**What goes wrong:**
Adding `snow_list_sort(list, elem_cmp)` follows the existing callback pattern (like `snow_list_compare`, `snow_list_eq`). The runtime function receives a `*mut u8` function pointer that compares two `u64` elements. The problem: the MIR lowerer must synthesize the correct comparator function pointer for the concrete element type at each call site.

For `List<Int>.sort()`, the comparator is `snow_int_compare`. For `List<String>.sort()`, it's `snow_string_compare` (which does not yet exist -- noted as tech debt at lower.rs:5799). For `List<Point>.sort()` where Point implements Ord, it needs the monomorphized `Ord__compare__Point` function. For `List<List<Int>>.sort()`, it needs a synthetic wrapper function (like the v1.4 nested Display wrappers).

If the MIR lowerer fails to synthesize the correct comparator for any element type, the sort will either: (a) compare raw u64 bit patterns (wrong for strings, floats, structs), (b) call the wrong function (memory corruption), or (c) fail to compile.

**Why it happens:**
The callback dispatch pattern works well but requires per-call-site specialization in MIR. Each new callback-taking operation multiplies the MIR lowerer's complexity. The existing `snow_list_to_string`, `snow_list_eq`, and `snow_list_compare` already demonstrate the pattern, but sort adds the constraint that the comparator must provide a total ordering -- partial orderings cause undefined behavior in sorting algorithms.

**Consequences:**
- Silent data corruption if wrong comparator used (e.g., comparing string pointers as integers)
- Compilation failure for nested generic types (List<List<T>>) if synthetic wrapper not generated
- Panic or infinite loop if comparator doesn't provide total ordering (NaN floats, custom Ord impls with bugs)

**Prevention:**
1. Require the `Ord` trait constraint on the element type for `sort()` -- this is already enforced at the typeck level for `compare()`
2. Reuse the existing comparator synthesis from `snow_list_compare` codegen -- the MIR lowerer already resolves the correct callback for Ord-constrained operations
3. For Float sorting, handle NaN explicitly: either error at compile time ("Float does not implement Ord") or use a total ordering that puts NaN last
4. Add the missing `snow_string_compare` to the runtime (it's already flagged as tech debt)
5. Test with every element type that has Ord: Int, Float, String, Bool, user structs, nested collections

**Detection:**
- Sort produces wrong order for string lists (comparing pointer values instead of string content)
- ICE (internal compiler error) when sorting List<List<Int>> -- missing synthetic wrapper

**Confidence:** HIGH -- verified callback pattern in list.rs:316-374 and the tech debt note at lower.rs:5799.

---

## Moderate Pitfalls

Mistakes that cause significant rework or user-facing bugs, but are contained to a single feature.

---

### Pitfall 6: Timer Primitives (sleep, send_after) Must Yield to Scheduler, Not Block OS Thread

**What goes wrong:**
The natural implementation of `sleep(ms)` is `std::thread::sleep(Duration::from_millis(ms))`. But Snow actors run as coroutines multiplexed across a fixed pool of OS worker threads (scheduler.rs:1-27). If an actor calls `std::thread::sleep`, it blocks the entire OS thread, starving all other actors pinned to that thread (since coroutines are `!Send` and stay on their creation thread).

Similarly, `send_after(pid, msg, delay_ms)` cannot simply spawn a Rust thread that sleeps -- it must integrate with the actor scheduler to avoid thread exhaustion.

**Why it happens:**
Coroutine-based schedulers require all blocking operations to yield to the scheduler rather than blocking the OS thread. This is a fundamental property of M:N scheduling that every new blocking primitive must respect.

**Consequences:**
- `sleep(5000)` in one actor freezes all other actors on the same OS thread for 5 seconds
- With N worker threads and N sleeping actors, the entire runtime deadlocks
- `send_after` that spawns OS threads creates threads unbounded -- 10,000 `send_after` calls create 10,000 threads

**Prevention:**
1. Implement `snow_sleep(ms)` as a yield loop: set actor state to Waiting, record a deadline (`Instant::now() + duration`), yield to scheduler. On resume, check if deadline passed; if not, yield again. This mirrors the pattern already used in `snow_actor_receive` for timeout (actor/mod.rs:365-399).
2. Implement `send_after(pid, msg, delay_ms)` as a lightweight actor spawn: spawn a new actor that runs `sleep(delay_ms); send(pid, msg)`. Since actors are lightweight (~2KB stack), this is cheap. Alternatively, add a timer wheel to the scheduler for O(1) timer management.
3. Do NOT use `std::thread::sleep` or `std::thread::spawn` anywhere in timer primitives.

**Detection:**
- Actor throughput drops to zero when any actor calls sleep
- Benchmark: spawn 100 actors that each sleep(10), measure total time. Should be ~10ms, not ~1000ms.

**Confidence:** HIGH -- verified scheduler architecture in scheduler.rs:1-27 and coroutine constraints (corosensei `!Send`) in stack.rs.

---

### Pitfall 7: ? Operator Type Inference -- Enclosing Function Must Return Result<T, E>

**What goes wrong:**
The `?` operator is only valid inside a function that returns `Result<T, E>`. If used in a function returning `Int`, the `Err` arm tries to `return Err(e)` where the function expects `Int` -- type mismatch. This constraint must be checked during type inference, not during codegen.

Snow's type checker (infer.rs) tracks the return type of the current function being inferred. But there's a subtlety: in multi-clause functions (`fn foo(0) = ...; fn foo(n) = ...`), all clauses share the return type. And in closures, the `?` operator must propagate to the closure's return type, not the enclosing function's return type.

A deeper issue: `?` on `Result<T, E1>` inside a function returning `Result<T, E2>` where `E1 != E2` would require error type conversion (Rust uses `From` trait for this). Snow does not have a `From` trait. If E1 and E2 differ, the types don't unify.

**Why it happens:**
The `?` operator implicitly constrains both the expression's type (must be `Result<T, E>`) and the enclosing function's return type (must be `Result<_, E>`). These bidirectional constraints are unusual in Snow's HM inference.

**Consequences:**
- Without proper validation: type error at codegen time (too late), or unsound code that returns Err where Int is expected
- Confusing error messages: "cannot unify Result<String, String> with Int" when user just wanted `?` on a file read

**Prevention:**
1. During inference of `expr?`, unify `expr`'s type with `Result<T_fresh, E_fresh>` and unify the enclosing function's return type with `Result<_, E_fresh>`
2. If the function's return type cannot unify with `Result<_, _>`, emit a dedicated error: "the `?` operator can only be used in functions that return Result<T, E>"
3. For closures, check the closure's return type, not the outer function
4. Do NOT implement error conversion (From trait) in v1.9 -- require exact E type match. Document this limitation.

**Detection:**
- Type error when `?` is used in a non-Result function
- ICE if typeck doesn't catch this and codegen tries to return Err from an Int function

**Confidence:** HIGH -- verified Result is a builtin sum type (infer.rs:756-787) with generic params T, E; verified HM inference flow.

---

### Pitfall 8: Collection Operations on Immutable Data -- O(n) Copy Cost Compounds to O(n^2) in Chains

**What goes wrong:**
Snow's collections use immutable semantics: every operation returns a new collection (list.rs:76-88 shows `snow_list_append` allocating a new list). Adding operations like `sort`, `zip`, `split`, and `join` that each return new collections means a chain like `list.sort().filter(f).zip(other)` creates 3 intermediate copies. For a list of N elements, sort is O(N log N) but also allocates a new N-element list, filter allocates up to N elements, and zip allocates up to N elements.

This is inherent to immutable semantics and usually acceptable, but the pitfall is in the runtime implementation: if `snow_list_sort` is implemented by copying to a Rust Vec, sorting, then copying back to a Snow list, there are 3 copies instead of 1. Similarly, if sort allocates via `snow_gc_alloc_actor` for each comparison (which it won't, but other operations might), GC pressure compounds.

**Why it happens:**
Immutable semantics require allocation per operation. This is a known tradeoff, not a bug. The pitfall is implementing operations with unnecessary extra copies or allocations beyond the inherent minimum.

**Consequences:**
- 2-3x slower than necessary if implementations do redundant copies
- GC pressure causes more frequent collections in actor heaps
- Users complain about performance on large collections

**Prevention:**
1. Sort in-place on the NEW copy: allocate the new list, copy elements into it, sort the copy in-place using a Rust sort (which operates on the raw `u64` array via the callback). One allocation, one sort.
2. For `zip`: allocate the result list once at `min(len_a, len_b)` capacity
3. For `split`: allocate the result list of lists, each sublist allocated once
4. For `join`: calculate total length first, allocate once
5. Use `snow_list_builder_new(capacity)` + `snow_list_builder_push` pattern (already exists, list.rs:285-304) for operations that build lists incrementally

**Detection:**
- Benchmark: sort a 10,000-element list. Should be ~1ms, not ~10ms.
- Heap size after sort should be ~2x the original list (old + new), not more.

**Confidence:** HIGH -- verified immutable copy pattern in list.rs:76-88 and list builder pattern in list.rs:285-304.

---

### Pitfall 9: send_after Creates Actor But Doesn't Link -- Crash Goes Unnoticed

**What goes wrong:**
If `send_after(pid, msg, delay_ms)` is implemented by spawning a temporary actor that sleeps then sends, that actor is not linked to anything. If the target actor dies before the timer fires, the send goes to a dead mailbox (silently dropped -- or the temporary actor crashes). If the temporary actor itself crashes (e.g., OOM during sleep), nobody is notified.

In Erlang/OTP, `erlang:send_after/3` returns a timer reference that can be cancelled. Snow would need a similar mechanism, or at minimum, the temporary actor should be linked to the caller.

**Why it happens:**
Timer primitives feel simple but interact with the actor lifecycle (links, monitors, supervision). A fire-and-forget timer that outlives its creator is a resource leak.

**Consequences:**
- Timer fires but message goes to dead process (silent failure)
- Timer actors accumulate if created faster than they expire (memory leak)
- No way to cancel a pending timer (architectural limitation if not designed in)

**Prevention:**
1. Return a timer reference (PID of the timer actor) from `send_after` so it can be cancelled
2. Link the timer actor to the calling actor so it dies if the caller dies
3. Alternatively, implement timers in the scheduler itself (timer wheel) rather than as actors -- more efficient but more complex
4. For v1.9, the actor-based approach is simpler and sufficient. Add cancellation support in a future version.

**Detection:**
- Timer fires after target actor has exited -- message silently lost
- Long-running server slowly accumulates timer actor corpses

**Confidence:** MEDIUM -- based on established actor model patterns; Snow's specific actor lifecycle behavior verified in scheduler.rs and link.rs.

---

### Pitfall 10: TCE Analysis Must Handle Let Bindings and Match Arms in Tail Position

**What goes wrong:**
Detecting whether a call is in tail position is straightforward for `return f(x)` but subtle for Snow's expression-oriented design. In Snow, the last expression in a function body is the return value. Consider:

```
fn factorial(n, acc) do
  let result = if n <= 1 do
    acc
  else
    factorial(n - 1, n * acc)
  end
  result
end
```

Here, `factorial(n - 1, n * acc)` is in tail position because: it's the else branch of an if, which is bound to `result` via let, which is the last expression (and thus returned). Detecting this requires traversing through Let, If, Match, and Block expressions.

Even harder: `fn foo(x) = match x do 1 -> bar(x); _ -> baz(x) end` -- both `bar(x)` and `baz(x)` are in tail position, but only if the match is itself in tail position.

If TCE analysis is too conservative (only handling direct `f(x)` at function body level), most real-world tail calls are missed. If too aggressive (marking calls as tail that aren't), stack cleanup is wrong.

**Why it happens:**
Expression-oriented languages make tail position identification recursive. The tail position propagates inward through if/else, match, let, and block expressions.

**Consequences:**
- Too conservative: most recursive functions still overflow the stack
- Too aggressive: incorrect code (return value not preserved, stack corruption)

**Prevention:**
1. Implement tail position analysis as a recursive MIR pass that propagates a `is_tail_position` flag:
   - Function body: is_tail_position = true
   - Let binding body: propagate tail status to the body (last expr after the let)
   - If/else: propagate to both branches IF the if/else is in tail position
   - Match: propagate to all arm bodies IF the match is in tail position
   - Block: propagate to the last expression in the block
   - Call: if in tail position AND callee matches, mark as tail call
2. Start with self-recursion only (safer, simpler, covers 80% of use cases)
3. Add mutual recursion support separately after self-recursion is validated

**Detection:**
- Stack overflow on `factorial(1000000, 1)` when TCE should prevent it
- Wrong return value when a non-tail call is incorrectly marked as tail

**Confidence:** HIGH -- verified by reading Snow's MIR structure (mir/mod.rs) where Let, If, Match, and Block are all expression-valued.

---

## Minor Pitfalls

Issues that cause friction or minor bugs but are straightforward to fix.

---

### Pitfall 11: Intrinsic Declarations for Math Functions Must Match libm's Exact Signatures

**What goes wrong:**
LLVM intrinsics for math (`llvm.sqrt.f64`, `llvm.pow.f64`, `llvm.fabs.f64`, etc.) take and return `f64`. Snow stores floats as f64 internally but passes them through the uniform u64 storage layer via bit-casting (f64 bits stored in i64). If the intrinsic declaration in `intrinsics.rs` uses `i64` parameters (matching Snow's storage convention) instead of `f64`, LLVM will either fail verification or produce incorrect results from bit reinterpretation.

For non-intrinsic libm functions (declared as `extern double sqrt(double)`), the same issue applies: the LLVM function declaration must use `f64`, and the caller must bitcast from i64 to f64 before the call and f64 to i64 after.

**Why it happens:**
Snow's uniform u64 storage means float values are stored as `i64` at the LLVM level. Every float operation already does bitcast(i64 -> f64) before the operation and bitcast(f64 -> i64) after. Math functions must follow the same pattern, but it's easy to forget when adding new declarations.

**Prevention:**
1. Declare math intrinsics with `f64` parameter and return types in `intrinsics.rs`
2. In codegen, bitcast arguments from i64 to f64 before calling, and bitcast result from f64 to i64 after
3. Follow the exact same pattern used for existing float operations (arithmetic, comparison)
4. Test with `Math.sqrt(2.0)` -- should return ~1.414, not garbage from bit reinterpretation

**Detection:**
- Math functions return NaN or wildly wrong values
- LLVM verification error about type mismatch on intrinsic call

**Confidence:** HIGH -- verified uniform u64 storage design (PROJECT.md line 165) and float handling pattern in codegen.

---

### Pitfall 12: String split() and join() -- Inconsistent Return Types Across Collections

**What goes wrong:**
`String.split(delimiter)` returns a `List<String>`. `List<String>.join(delimiter)` returns a `String`. These cross collection boundaries -- a String operation produces a List, and a List operation produces a String. The type checker and MIR lowerer must handle these cross-type returns.

The current stdlib method dispatch (lower.rs line 7234-7245) maps bare names to runtime functions based on context. If `split` is registered as both a String method and (in the future) a List method, name resolution becomes ambiguous. The v1.6 method resolution priority (module > service > variant > struct field > method, from PROJECT.md) handles this, but the MIR builtin name mapping is a flat table.

**Why it happens:**
Operations like split/join bridge between String and List<String>. The type system handles this fine (different operations on different types), but the builtin name mapping and method dispatch must route correctly.

**Consequences:**
- `"a,b,c".split(",")` could resolve to the wrong function if `split` is ambiguous
- Type checker infers wrong return type if method not properly registered

**Prevention:**
1. Register `split` under the String module: `"string_split" => "snow_string_split"` in the builtin map
2. Register `join` under the List module: `"list_join" => "snow_list_join"` in the builtin map
3. The dot-syntax method dispatch (v1.6) already resolves based on receiver type, so `"hello".split(",")` routes to String and `["a", "b"].join(",")` routes to List
4. For `find`, which exists on multiple collection types, ensure each variant is registered with its type-prefixed name

**Detection:**
- Method resolution error or wrong method called when using split/join
- Type inference produces wrong type for split result

**Confidence:** HIGH -- verified builtin name mapping in lower.rs:7208-7340 and method dispatch in PROJECT.md.

---

### Pitfall 13: GC Safety During Long-Running Sort Operations

**What goes wrong:**
A sort operation on a large list (e.g., 100,000 elements) runs the comparator callback N*log(N) times. During this time, the actor is inside `snow_list_sort` (a Rust runtime function), not yielding to the scheduler. GC cannot run because GC only triggers at yield points (PROJECT.md line 152: "GC at yield points only"). Other actors on the same worker thread are starved.

More subtly: if the comparator callback is a closure that captures GC-managed values, and a GC happens to trigger inside the callback (hypothetically), the closure's captured values could be moved. But since GC only runs at yield points and sort doesn't yield, this specific scenario cannot happen in the current design. The real risk is scheduler starvation, not GC corruption.

**Why it happens:**
Runtime functions written in Rust (extern "C") are opaque to the scheduler. The reduction counter (`snow_reduction_check`) is not called during runtime operations, only between Snow-level function calls.

**Consequences:**
- Sorting a large list (>10K elements) causes visible latency spikes in other actors on the same thread
- Reduction count not decremented during sort -- actor gets "free" CPU time proportional to list size
- In extreme cases, supervisor timeouts fire because supervised actors can't respond while sort is running on their thread

**Prevention:**
1. For v1.9, accept this limitation. Sort operations are typically fast (< 1ms for 10K elements), and the same issue exists for all runtime functions (map, filter, reduce, etc.)
2. If performance becomes an issue: insert periodic `snow_reduction_check()` calls inside the sort loop (every N comparisons). This requires the Rust sort implementation to call back into the runtime, which is architecturally complex.
3. Document that large collection operations may cause brief scheduler pauses

**Detection:**
- Actor response latency spikes correlating with large sort operations
- Benchmark: sort 100K elements while measuring other actors' response time

**Confidence:** MEDIUM -- this is a known architectural property of cooperative scheduling, not specific to sort.

---

### Pitfall 14: zip() Must Handle Different-Length Lists Gracefully

**What goes wrong:**
`List.zip(other)` should combine two lists into a list of tuples. If the lists have different lengths, the behavior must be well-defined. Three options: (a) truncate to shorter length (Erlang/Elixir, Python), (b) error on mismatch, (c) pad shorter with a default value. If not explicitly decided, different implementations may assume different semantics.

Snow's tuples already exist in the runtime (tuple.rs), stored as `{ u64 len, u64[] elements }`. The result `List<(A, B)>` requires creating tuple values for each pair. The GC must be able to trace through these tuples -- since tuples are heap-allocated and use conservative stack scanning, this should work automatically.

**Why it happens:**
zip semantics vary across languages. Without an explicit decision, the implementation might change between versions or behave inconsistently.

**Consequences:**
- User surprise when `[1,2,3].zip([4,5])` returns `[(1,4), (2,5)]` instead of an error
- If tuples are not properly GC-traced, memory leak or use-after-free

**Prevention:**
1. Choose truncate-to-shorter semantics (matches Elixir and most functional languages)
2. Document the choice explicitly
3. Ensure tuple allocation uses `snow_gc_alloc_actor` (it already does per tuple.rs patterns)

**Detection:**
- Incorrect zip result when lists have different lengths
- Memory leak in long-running actor that frequently zips lists

**Confidence:** HIGH -- straightforward design decision; GC safety verified via existing tuple allocation pattern.

---

### Pitfall 15: TCE Must Not Break Stack Traces for Debugging

**What goes wrong:**
Tail-call elimination replaces call frames with jumps. When an error occurs in a tail-called function, the stack trace only shows the final function in the chain, not the sequence of tail calls that led to it. For debugging, this can be confusing: `factorial(5, 1)` crashes and the stack trace shows only `factorial` at some intermediate value, not the original call site.

**Why it happens:**
TCE fundamentally trades debugging information for stack space. This is an inherent tradeoff, not a bug.

**Consequences:**
- Stack traces are less useful for debugging recursive functions
- Users unfamiliar with TCE are confused by "missing" stack frames

**Prevention:**
1. Only apply TCE at optimization level >= 1. At -O0 (debug), preserve full call stacks.
2. Alternatively, always apply TCE (since it's correctness-critical for deep recursion) but add a diagnostic note to error messages: "Note: some stack frames elided due to tail-call optimization"
3. For v1.9, apply TCE unconditionally (it's needed for correctness in functional style) but document the stack trace impact

**Detection:**
- Users report "impossible" stack traces where function A calls B but only B appears in the trace

**Confidence:** HIGH -- inherent property of TCE; well-documented in language implementation literature.

---

## Phase-Specific Warnings

| Phase Topic | Likely Pitfall | Mitigation |
|---|---|---|
| Math stdlib via libm | Missing `-lm` on Linux; wrong types for LLVM intrinsics | Use LLVM intrinsics where possible; add `-lm` to linker; bitcast i64<->f64 |
| ? operator | Early return control flow breaks expression codegen; type inference for enclosing function return type | Lower to match+return in MIR; validate enclosing fn returns Result |
| Receive timeout after clause | Null pointer dereference on timeout; timeout_body ignored | Add null check after snow_actor_receive; branch to timeout block |
| Timer primitives | OS thread blocking instead of coroutine yield; unlinked timer actors | Yield-loop for sleep; return timer ref from send_after; link to caller |
| Collection operations | Wrong comparator for generic sort; O(n) copy chains; name collision in builtin map | Reuse Ord callback synthesis; sort new copy in-place; type-prefix all names |
| Tail-call elimination | reduction_check after tail call invalidates musttail; signature mismatch; tail position detection through expressions | Loop transform for self-recursion; skip reduction check for tail calls; recursive is_tail_position analysis |

---

## Integration Pitfalls -- Features That Interact

### ? Operator + Receive Timeout

If a receive with timeout returns `Result<T, E>` (e.g., `Ok(msg)` on success, `Err("timeout")` on timeout), and the user writes `let msg = receive ... after 1000 -> Err("timeout") end?`, the `?` operator must work on the receive expression's result. This requires the receive codegen to produce a proper Result value (ConstructVariant), not a raw message pointer.

**Prevention:** Design receive-with-timeout to return the message type directly (not Result), with the after clause being a separate expression that produces the same type. This matches Erlang semantics. If Result wrapping is desired, make it explicit in user code.

### Collection sort() + Tail-Call Elimination

A recursive mergesort implementation in Snow would benefit from TCE. But if sort is implemented in the Rust runtime (the recommended approach), TCE doesn't apply -- Rust's own TCE is not guaranteed. If sort is implemented in Snow itself, TCE matters but the callback overhead for comparisons makes the pure-Snow approach slower.

**Prevention:** Implement sort in the Rust runtime for performance. TCE is irrelevant for runtime functions.

### Timer Primitives + GC

A sleeping actor (yielded, waiting for timer) still has a live heap. If many actors are sleeping (e.g., 10,000 `send_after` timer actors), they each hold a small heap that cannot be collected until they wake and exit. This is a minor memory concern but worth noting.

**Prevention:** Keep timer actors minimal (no captured closures, no heap allocations beyond the message to send).

### Math Stdlib + Collection Operations

Users will want `list.map(fn (x) -> Math.sqrt(x) end)`. This requires Math functions to be callable from closures, which means they must be declared as known functions in the MIR lowerer. LLVM math intrinsics cannot be called indirectly (they must be direct calls). If `Math.sqrt` is lowered to `llvm.sqrt.f64`, it cannot be passed as a function pointer.

**Prevention:** Implement math functions as runtime extern "C" functions (e.g., `snow_math_sqrt`) that internally call the LLVM intrinsic or libm. This way they can be referenced as function pointers for higher-order use. The indirection cost is negligible.

---

## Sources

- Snow codebase analysis: codegen/expr.rs, codegen/mod.rs, mir/mod.rs, mir/lower.rs, snow-rt/src/actor/mod.rs, snow-rt/src/collections/list.rs, snow-rt/src/io.rs, link.rs
- [LLVM Language Reference - musttail](https://llvm.org/docs/LangRef.html) -- caller/callee signature matching requirements
- [Inkwell CallSiteValue documentation](https://thedan64.github.io/inkwell/inkwell/values/struct.CallSiteValue.html) -- set_tail_call_kind available on LLVM 18+
- [LLVM musttail implementation review](https://reviews.llvm.org/D99517) -- original musttail implementation with constraints
- [LLVM musttail backend failures](https://github.com/llvm/llvm-project/issues/54964) -- platform-specific musttail issues
- [Tail call elimination approaches](https://notes.eatonphil.com/tail-call-elimination.html) -- loop transformation vs trampoline
- [Rust libm crate](https://github.com/rust-lang/libm) -- pure Rust math functions for portability
- [LLVM Tail Recursion Elimination pass](https://llvm.org/doxygen/TailRecursionElimination_8cpp_source.html) -- self-recursion to loop transformation
- Snow PROJECT.md -- architectural decisions and constraints documentation
