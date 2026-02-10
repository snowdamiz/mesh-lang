# Feature Landscape: v1.9 Stdlib & Ergonomics

**Domain:** Programming language compiler -- stdlib expansion, error ergonomics, actor timer primitives, collection operations, and tail-call elimination
**Researched:** 2026-02-09
**Confidence:** HIGH (all features are well-studied across Rust, Erlang/Elixir, Haskell, Scheme)

---

## Current State in Snow

Before defining features, here is what already exists and directly affects this milestone:

**Working (infrastructure these features build on):**
- Full compiler pipeline: lexer -> parser -> HM type inference -> MIR lowering -> LLVM codegen
- `Result<T, E>` with `Ok`/`Err` variants, `Option<T>` with `Some`/`None` variants
- Pattern matching with exhaustiveness checking (`case` expressions)
- `return` expression for early returns from functions
- Collections: `List` (map/filter/reduce/reverse/head/tail/get/concat/length), `Map` (new/put/get/has_key/delete/size/keys/values), `Set` (new/add/remove/contains/size/union/intersection), `Queue`, `Range`
- String ops: length, slice, contains, starts_with, ends_with, trim, to_upper, to_lower, replace
- Actor runtime with typed `Pid<M>`, `spawn`, `send`, `receive`, `self()`, `link`
- `receive` expression with `after` clause **already parsed** (AST has `AfterClause`, lexer has `after` keyword, MIR has `timeout_ms` and `timeout_body` fields on `ActorReceive`)
- Runtime's `snow_actor_receive(timeout_ms: i64)` already supports timeouts (returns null on timeout)
- Services, Jobs, Supervision trees all operational
- 6 stdlib protocols: Display, Debug, Eq, Ord, Hash, Default with auto-derive
- Module system with qualified/selective imports, pub visibility
- Pipe operator `|>` for function chaining
- Existing builtin registration pattern in `snow-typeck/src/builtins.rs`
- Existing intrinsic declaration pattern in `snow-codegen/src/codegen/intrinsics.rs`
- Runtime callback calling conventions (used by `snow_list_map`, `snow_list_filter`, etc.)

**Partially implemented (needs completion):**
- Receive `after` clause: parsed and lowered to MIR, but **codegen ignores `timeout_body`** (the field is matched as `timeout_body: _` in `expr.rs` line 129). The timeout value is passed to the runtime, but the body that should execute on timeout is discarded.

**Not yet built (what this milestone adds):**
- Math stdlib module
- `?` operator for Result/Option propagation
- Receive timeout codegen completion (wire `timeout_body` through)
- Timer primitives (sleep, send_after)
- Additional collection operations (sort, find, zip, etc.)
- String split/join/to_int/to_float
- Tail-call elimination

---

## Table Stakes

Features users expect. Missing = product feels incomplete.

### 1. Math Stdlib

Every programming language provides a core math module. Users writing anything beyond trivial programs need these immediately. Erlang provides `math:sqrt/1`, `math:pow/2`, `math:pi/0`, `math:floor/1`, `math:ceil/1`. Python's `math` module, Kotlin's `kotlin.math` package, and C's `<math.h>` all provide the same core set.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| `Math.abs(x)` | Universal across all languages. Needed for distance calculations, normalization. | Low | For Int: LLVM select instruction (`if x < 0 then -x else x`). For Float: call libc `fabs`. Register two overloads in builtins. |
| `Math.min(a, b)` | Universal. Needed for bounds checking, clamping. | Low | Simple comparison + select. Int and Float overloads. |
| `Math.max(a, b)` | Universal. Always paired with min. | Low | Same implementation pattern as min. |
| `Math.pow(base, exp)` | Universal. Needed for any numeric computation. | Low | Float: call libc `pow(f64, f64)`. Int exponentiation: implement iterative squaring in runtime (`snow_math_pow_int`). |
| `Math.sqrt(x)` | Universal. Needed for geometry, statistics. | Low | Call libc `sqrt`. Takes Float, returns Float. |
| `Math.floor(x)` | Universal rounding function. Converting Float to Int requires this. | Low | Call libc `floor`. Return Int (this is what users want 95% of the time; Erlang's `math:floor/1` returns Float but `trunc/1` exists for Int). |
| `Math.ceil(x)` | Universal rounding function. Always paired with floor. | Low | Call libc `ceil`. Return Int to match floor. |
| `Math.round(x)` | Users expect this alongside floor/ceil. Omitting it feels incomplete. | Low | Call libc `round` or `llvm.round`. Return Int. |
| `Math.pi` | Universal constant. Any trigonometric work needs it. | Low | Constant value `3.141592653589793`. Can be a compile-time constant or a zero-argument function. |
| `Int.to_float(x)` | Type conversion between numeric types is fundamental. | Low | LLVM `sitofp` instruction. |
| `Float.to_int(x)` | Type conversion between numeric types is fundamental. | Low | LLVM `fptosi` instruction. Truncates toward zero. |

**Implementation pattern:** Follow the existing String operations model:
1. Add runtime functions in `snow-rt` (e.g., `snow_math_abs_int`, `snow_math_sqrt`)
2. Declare intrinsics in `snow-codegen/src/codegen/intrinsics.rs`
3. Register built-in function types in `snow-typeck/src/builtins.rs`
4. Handle in MIR lowering as method calls on the `Math` module

**What NOT to include in v1.9:**
- Trigonometric functions (sin, cos, tan, asin, acos, atan, atan2) -- defer to future batch
- Hyperbolic functions (sinh, cosh, tanh) -- niche, defer
- Logarithmic functions (log, log2, log10) -- defer to future batch
- Error functions (erf, erfc) -- very niche, defer
- `Math.tau` constant -- defer

**Rationale for scope:** The milestone description specifies `abs, min, max, pow, sqrt, floor, ceil`. Adding `round`, `pi`, `Int.to_float`, and `Float.to_int` costs almost nothing and users will immediately ask for them. Trig/log functions are a natural second batch but are not table stakes for a general-purpose language at this stage.

**Confidence:** HIGH -- based on Erlang's `math` module, Python's `math`, Kotlin's `kotlin.math`, C's `<math.h>`. These functions are identical across every language.

**Dependencies:** None on other v1.9 features. Requires new entries in intrinsics.rs, new runtime functions, new builtin registrations.

---

### 2. Question Mark (?) Operator for Result/Option Propagation

The `?` operator is the single most impactful ergonomic feature for error handling. Without it, every Result-returning call requires a verbose `case` expression. With it, the "happy path" reads linearly.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| `expr?` on `Result<T, E>` | Rust proved this is the gold standard for error propagation. Unwraps `Ok(val)` to `val`, early-returns `Err(e)` from enclosing function. | **Med** | Requires: new postfix operator in lexer/parser, type checking that enclosing function returns `Result`, desugaring to match in MIR or directly in typeck lowering. |
| `expr?` on `Option<T>` | Rust supports this. Unwraps `Some(val)` to `val`, early-returns `None`. | Med | Same mechanism, different variant names. Enclosing function must return `Option`. |

**How comparable languages handle this:**

| Language | Mechanism | Syntax | Error Type Conversion | Key Insight |
|----------|-----------|--------|----------------------|-------------|
| **Rust** | `?` postfix operator | `let val = expr?;` | Automatic via `From` trait: `Err(e)` becomes `Err(From::from(e))` | Desugars to `match Try::branch(expr) { Continue(v) => v, Break(r) => return FromResidual::from_residual(r) }` |
| **Swift** | `try` prefix keyword | `let val = try expr` | Automatic via `Error` protocol conformance | Prefix `try` marks potential failure points. `try?` converts to Optional, `try!` force-unwraps. |
| **Kotlin** | No dedicated operator | `runCatching { expr }.getOrThrow()` | Manual via `Result.getOrThrow()` | Kotlin's `Result` type is less ergonomic than Rust's. Community uses Arrow's `Either` for typed errors. |

**Snow-idiomatic design:**

Use Rust's model: `expr?` is a postfix unary operator that desugars to:

```snow
# For Result<T, E>:
case expr {
  Ok(val) -> val
  Err(e) -> return Err(e)
}

# For Option<T>:
case expr {
  Some(val) -> val
  None -> return None
}
```

**Key design decisions:**

1. **No From-trait conversion (yet).** Rust's `?` converts error types via `From::from(e)`. Snow does not have a `From` trait. For v1.9, require exact type match: the `?`'d expression's error type must match the enclosing function's return error type. This avoids a large design surface while giving 90% of the value.

2. **Postfix, not prefix.** Rust's postfix `?` reads better in chains: `file.read()?.parse()?`. Swift's prefix `try` disrupts reading flow. Snow already has pipe `|>` which is postfix-oriented. Postfix `?` is consistent with the language's left-to-right data flow philosophy.

3. **Works in functions only, not closures (initially).** Closures complicate the "enclosing function" lookup because `?` would return from the closure, not the outer function. This matches Rust's behavior (closure `?` returns from the closure). Start simple: `?` in a function returns from that function. In a closure, `?` returns from the closure (if the closure's return type is Result/Option).

4. **Type checking constraint:** The function or closure containing `?` must have return type `Result<T, E>` (for Result `?`) or `Option<T>` (for Option `?`). This is a compile-time check in `snow-typeck`. Error: "cannot use `?` in function returning `Int`; function must return `Result` or `Option`".

5. **Precedence:** Binds tighter than `|>` but looser than `.` field access and function call `()`. So `foo.bar()?` applies `?` to the call result. And `x? |> process` applies `?` to `x` first, then pipes the unwrapped value.

**Implementation layers:**
- **Lexer:** `?` already exists as a token (verify). If not, add `QUESTION` token.
- **Parser:** Parse as postfix unary expression. New AST node `TryExpr` wrapping an inner expression.
- **Typeck:** Check inner expression is `Result<T, E>` or `Option<T>`. Check enclosing function returns compatible type. Infer result type as `T`.
- **MIR lowering:** Desugar `TryExpr` to a `Case` expression with `Return(Err(e))` or `Return(None)`.
- **Codegen:** No new codegen needed -- uses existing Case + Return codegen.

**Confidence:** HIGH -- Rust's `?` operator desugaring is thoroughly documented. Snow already has `Result<T,E>`, `Option<T>`, pattern matching, and `return` expressions. The desugaring is mechanical.

---

### 3. Receive Timeouts (Complete the After Clause)

This is **80% implemented**. The gap is specifically in codegen -- the `timeout_body` is parsed, lowered to MIR, but silently discarded during LLVM IR generation.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| `receive { ... } after ms -> body` | Erlang/Elixir's foundational concurrency pattern. Every actor system needs timeouts to avoid deadlocks and implement retry logic. | **Low** (infrastructure exists) | Fix codegen to branch to timeout body when `snow_actor_receive` returns null. |

**How Erlang handles this (official semantics from Erlang System Documentation v28.3.1):**

```erlang
receive
    {request, From, Data} -> handle(From, Data);
    {info, Msg} -> log(Msg)
after
    5000 -> timeout_action()
end
```

Semantics:
- `ExprT` evaluates to an integer (milliseconds) or the atom `infinity`
- If no matching message arrives within `ExprT` ms, `BodyT` executes instead
- The value of `BodyT` becomes the value of the entire `receive...after` expression
- **Timeout value 0:** Checks mailbox once, immediately executes body if empty. Does NOT yield to scheduler first.
- **Maximum timeout:** 4,294,967,295 ms (~49.7 days) in Erlang. Snow should use i64 for effectively unlimited range.
- The timer starts when the `receive` begins executing, not when the process was spawned

**Snow syntax (already parsed and AST exists):**

```snow
receive {
  msg -> handle(msg)
} after 5000 -> {
  println("timed out")
  default_value
}
```

**What specifically needs to happen:**

1. **Codegen fix in `snow-codegen/src/codegen/expr.rs`:** The `codegen_actor_receive` function currently calls `snow_actor_receive(timeout_val)`, checks for null, but does NOT codegen the `timeout_body`. When the runtime returns null (timeout), branch to a new basic block that generates IR for the timeout body expression. The timeout body's result becomes the value of the entire receive expression.

2. **Type checking:** The timeout body's type must unify with the receive arms' types. The whole `receive...after` expression has one result type. If arms return `String` and timeout body returns `Int`, that is a type error.

3. **MIR already carries timeout_body:** The `MirExpr::ActorReceive` node has `timeout_ms: Option<Box<MirExpr>>` and `timeout_body: Option<Box<MirExpr>>`. Both are populated by the MIR lowerer. Only codegen ignores the body.

**Confidence:** HIGH -- the runtime supports timeouts, the parser supports the syntax, MIR carries the data. This is a codegen wiring fix.

---

### 4. Timer Primitives (sleep, send_after)

Actor systems universally provide timer primitives. Without them, implementing periodic tasks, heartbeats, retries, or delayed messages requires ugly workarounds.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| `Timer.sleep(ms)` | Universal. Every language has this. Erlang's `timer:sleep/1` is literally `receive after T -> ok end`. Elixir's `Process.sleep/1`. | Low | Implement as `snow_actor_receive(ms)` in actor context (yields to scheduler), `std::thread::sleep` in non-actor context. |
| `Timer.send_after(pid, ms, msg)` | Erlang's `erlang:send_after/3`, Elixir's `Process.send_after/3`. Core building block for delayed messages, periodic tasks, heartbeat patterns. | **Med** | Requires spawning a background task or using OS timers. Returns a timer reference for potential cancellation. |

**How Erlang/Elixir implements timers:**

- `timer:sleep(Time)` -- implemented as bare `receive after T -> ok end`. Suspends the process.
- `erlang:send_after(Time, Dest, Msg)` -- BIF that schedules a message delivery after Time ms. Returns a timer reference. In OTP 25+, one-shot timers use `erlang:send_after` directly in the client process without going through the timer server, making them very efficient.
- `Process.send_after(pid, msg, time)` in Elixir -- wraps `erlang:send_after/3`. The timer is automatically canceled if the destination pid is not alive.
- Timer references can be canceled with `erlang:cancel_timer/1` or inspected with `erlang:read_timer/1`.

**Snow design:**

**`Timer.sleep(ms: Int) -> ()`** -- Suspends the current actor for `ms` milliseconds.
- In actor context: calls `snow_actor_receive(ms)`, discards the null result. The actor yields to the scheduler during the wait, not burning CPU.
- In non-actor context (main thread): calls `std::thread::sleep`.
- Implementation: single runtime function `snow_timer_sleep(ms: i64)`.

**`Timer.send_after(target: Pid<M>, ms: Int, msg: M) -> TimerRef`** -- Schedules sending `msg` to `target` after `ms` milliseconds.
- Implementation: spawn a lightweight internal actor that calls `snow_timer_sleep(ms)` then `snow_actor_send(target, msg)`. Return the spawned actor's PID wrapped as a `TimerRef`.
- This is the same approach as Erlang's pre-OTP-25 timer module (spawn a process that sleeps then sends).
- `TimerRef` is an opaque wrapper around a PID. Can be used for future `Timer.cancel` support.

**Defer to future:**
- `Timer.cancel(ref)` / `Timer.read(ref)` -- Requires sending a cancel message to the timer actor. Doable but adds complexity.
- `Timer.send_interval(target, ms, msg)` -- Periodic timer. Requires the timer actor to loop.

**Confidence:** MEDIUM for `Timer.sleep` (straightforward, reuses receive timeout). MEDIUM for `Timer.send_after` (requires new runtime code but the pattern is well-understood from Erlang).

**Dependencies:** `Timer.sleep` depends on receive timeouts working (feature 3). `Timer.send_after` depends on `Timer.sleep` and existing actor send.

---

### 5. Collection Operations

Collections are the workhorse of any language. Snow has good basic coverage but is missing operations users reach for constantly. Haskell's Prelude provides `zip`, `find`, `any`, `all`, `sort` out of the box. Elixir's `Enum` module provides 70+ functions. Rust's `Iterator` trait provides `find`, `any`, `all`, `zip`, `enumerate`, `flat_map`, `take`, `drop`, `sort`.

#### List Operations

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| `List.sort(list, cmp_fn)` | Table stakes. Every language has sort. Needed for ordered display, binary search, deduplication. | **Med** | Implement merge sort (stable, O(n log n), no mutation needed for immutable lists). Requires 2-argument comparator callback returning Int (negative/zero/positive). |
| `List.find(list, pred_fn)` | Table stakes. Haskell `find`, Elixir `Enum.find`, Rust `iter().find()`. | Low | Linear scan. Returns `Option<T>`. Short-circuits on first match. |
| `List.any(list, pred_fn)` | Table stakes. Haskell `any`, Elixir `Enum.any?`, Rust `iter().any()`. | Low | Short-circuiting linear scan. Returns Bool. |
| `List.all(list, pred_fn)` | Table stakes. Always paired with `any`. | Low | Short-circuiting linear scan. Returns Bool. |
| `List.zip(list_a, list_b)` | Table stakes in functional languages. Haskell `zip`, Elixir `Enum.zip`. | Low | Pair elements into tuples `(A, B)`. Truncates to shorter list length. Returns `List<(A, B)>`. |
| `List.flat_map(list, fn)` | Expected in functional languages. Haskell `concatMap`, Elixir `Enum.flat_map`. | Low | Map then flatten. Dedicated runtime function avoids intermediate list allocation. |
| `List.flatten(list)` | Expected. Flatten `List<List<T>>` to `List<T>`. | Low | Iterate nested lists, concat elements into one list. |
| `List.enumerate(list)` | Common utility. Elixir `Enum.with_index`, Python `enumerate`, Rust `iter().enumerate()`. | Low | Returns `List<(Int, T)>` -- pairs each element with its 0-based index. |
| `List.take(list, n)` | Common utility. Get first n elements. Haskell `take`, Elixir `Enum.take`. | Low | Copy first min(n, len) elements to new list. |
| `List.drop(list, n)` | Common utility. Skip first n elements. Always paired with take. | Low | Copy elements starting at index n. |
| `List.contains(list, elem)` | Table stakes. Check membership. Elixir `Enum.member?`, Python `in`. | Low | Linear scan with equality check. Requires Eq on element type (dispatch to `Eq__eq__T` like `snow_list_eq`). |
| `List.sum(list)` | Very common for numeric lists. Elixir `Enum.sum`. | Low | Reduce with `+`. Only works on Int/Float. |

#### String Operations

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| `String.split(s, delim)` | Table stakes. Every language has string split. Essential for parsing CSV, tokens, paths. | Low | Split by delimiter string. Returns `List<String>`. New runtime function `snow_string_split`. |
| `String.join(list, sep)` | Table stakes. Always paired with split. Elixir `Enum.join`, Python `sep.join(list)`. | Low | Join list of strings with separator. New runtime function `snow_string_join`. |
| `String.to_int(s)` | Table stakes. String-to-number parsing. Every language provides this. | Low | Returns `Option<Int>`. Parse failure returns `None`. New runtime function returning `SnowOption`. |
| `String.to_float(s)` | Table stakes. Always paired with to_int. | Low | Returns `Option<Float>`. Same pattern as to_int. |
| `String.chars(s)` | Common. Get individual characters. Elixir `String.graphemes`. | Low | Returns `List<String>` (each character as a single-char string). |
| `String.index_of(s, needle)` | Common. Find position of substring. | Low | Returns `Option<Int>`. |

#### Map Operations

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| `Map.merge(a, b)` | Common. Combine two maps. Elixir `Map.merge/2`. | Low | Values from `b` overwrite `a` on key conflict. Iterate `b`, put each entry into copy of `a`. |
| `Map.to_list(map)` | Common. Convert to list of key-value tuples. | Low | Iterate entries, build list of `(K, V)` tuples. |
| `Map.from_list(list)` | Common. Build map from list of tuples. | Low | Iterate list, insert each `(K, V)` pair. |
| `Map.map_values(map, fn)` | Common. Transform values keeping keys. Elixir `Map.new(map, fn)`. | Med | Iterate entries, apply fn to each value, build new map. Requires callback calling convention. |
| `Map.filter(map, pred_fn)` | Common. Filter entries by predicate. | Med | Iterate entries, keep matching ones. Predicate takes `(K, V)` and returns Bool. |

#### Set Operations

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| `Set.difference(a, b)` | Table stakes set operation alongside existing union/intersection. | Low | Elements in `a` not in `b`. Iterate `a`, check `!Set.contains(b, elem)`. |
| `Set.to_list(set)` | Common. Convert set to list. | Low | Iterate elements, build list. |
| `Set.from_list(list)` | Common. Build set from list. | Low | Iterate list, add each element. |
| `Set.is_subset(a, b)` | Common set theory operation. | Low | Check `Set.all(a, fn(x) -> Set.contains(b, x))`. |
| `Set.map(set, fn)` | Useful. Transform elements. | Med | Map then rebuild set. |
| `Set.filter(set, pred)` | Useful. Filter elements. | Med | Iterate, keep matching. |

**Implementation pattern:** All collection operations follow the existing pattern established by `snow_list_map` and `snow_list_filter`:
1. Runtime function in `snow-rt/src/collections/{list,map,set}.rs`
2. Intrinsic declaration in `snow-codegen/src/codegen/intrinsics.rs`
3. Builtin type registration in `snow-typeck/src/builtins.rs`
4. Callback convention: `fn_ptr: *const u8, env_ptr: *const u8` (env_ptr is null for non-closures)

**Priority for v1.9:**
1. **Must have:** List.sort, List.find, List.any, List.all, List.contains, String.split, String.join, String.to_int, String.to_float
2. **Should have:** List.zip, List.flat_map, List.enumerate, List.take, List.drop, Map.merge, Map.to_list, Map.from_list, Set.difference, Set.to_list, Set.from_list
3. **Nice to have:** List.flatten, List.sum, Map.map_values, Map.filter, Set.map, Set.filter, Set.is_subset, String.chars, String.index_of

**Confidence:** HIGH -- these operations are universal across Haskell, Elixir, Rust, Python. The runtime infrastructure (callback conventions, GC allocation, immutable copy-on-write) already exists.

---

### 6. Tail-Call Elimination (Self-Recursive)

TCE is not a "nice to have" -- it is a **hard requirement** for an actor-based language. Without TCE, every actor's top-level receive loop grows the stack unboundedly until the process crashes. Robert Virding (co-creator of Erlang): "the main case where TCO is critical is in process top-loops. These functions never return so they will build up a stack never to release it."

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| Self-recursive tail calls | Minimum viable TCE. Function calling itself in tail position. Covers the critical actor loop case. | **Med** | Transform to loop in MIR. Well-understood technique used by every functional language compiler. |
| Mutually-recursive tail calls | Full TCE. Function A calls B in tail position, B calls A. Scheme mandates this. | **High** | Requires LLVM `musttail` or trampolining. Defer to future milestone. |

**How comparable languages guarantee this:**

| Language | Guarantee | Mechanism | Scope |
|----------|-----------|-----------|-------|
| **Scheme** (R7RS) | **Mandated by spec.** All tail calls eliminated. | Implementation-defined (CPS, trampolining, etc.) | All tail calls including mutual recursion |
| **Erlang/BEAM** | **"Last call optimization."** Guaranteed for all tail calls. | BEAM VM replaces call with jump when last operation is a call. | All tail calls including mutual recursion. Critical for process loops. |
| **Haskell** (GHC) | Inherent via lazy evaluation + STG machine. | Continuation-passing, thunk evaluation. | Effectively all tail calls. |
| **Rust** | **No guarantee.** LLVM may optimize opportunistically. | Best-effort by optimization passes. | None (by spec). |
| **Scala** | `@tailrec` annotation guarantees self-recursion; compile error if not tail-recursive. | Compiler transforms to loop. | Self-recursion only (JVM limitation). |
| **LLVM** | `musttail` attribute on call instructions guarantees tail call or compilation fails. | Backend rewrites call to jump, reuses stack frame. Constraints: caller/callee must have compatible signatures. | Per-call-site opt-in. |

**Snow design for v1.9: Self-recursive TCE via MIR loop transformation.**

This covers:
- Actor receive loops: `fn loop(state) { receive { msg -> loop(new_state) } }`
- Recursive list processing: `fn sum(list, acc) { case list { [] -> acc, [h, ...t] -> sum(t, acc + h) } }`
- All single-function tail recursion

**Implementation approach:**

1. **Detect tail position in MIR lowering.** A call is in tail position when:
   - It is the return value of the function body
   - It is the last expression of an `if`/`else` branch when the `if` is in tail position
   - It is the body of a `case` arm when the `case` is in tail position
   - It is the last expression of a block when the block is in tail position
   - It is the body of a `receive` arm or `after` clause when the receive is in tail position
   - NOT: inside a `let` binding's value position (unless it is the last expression and the call IS the value)
   - NOT: as an argument to another function
   - NOT: inside a closure body (the closure is a different function)
   - NOT: after `?` (the result is fed into the match desugaring)

2. **Transform self-recursive tail calls to loop.** In MIR, when a function's tail-position call targets itself:
   ```
   fn factorial(n: Int, acc: Int) -> Int {
     if n <= 1 { acc }
     else { factorial(n - 1, acc * n) }
   }
   ```
   Becomes (conceptually):
   ```
   fn factorial(n: Int, acc: Int) -> Int {
     loop {
       if n <= 1 { return acc }
       else { let new_n = n - 1; let new_acc = acc * n; n = new_n; acc = new_acc; continue }
     }
   }
   ```

3. **No LLVM `musttail` needed for self-recursion.** The loop transformation is simpler, more portable, and doesn't have `musttail`'s signature-matching constraints. It also produces better code because LLVM can optimize the loop body freely.

4. **Optional `@tailrec` annotation (differentiator).** Like Scala, Snow could provide an annotation that causes a compile error if the function is NOT tail-recursive. This catches accidental non-tail recursion:
   ```snow
   @tailrec
   fn loop(state: State) -> State {
     let new_state = process(state)
     loop(new_state)  # OK -- tail position
   }
   ```

**What qualifies as "tail position" formally:**
```
tail(fn body)           = tail(body)
tail(block { ...; e })  = tail(e)  -- last expression of block
tail(if c { a } else { b }) = tail(a) AND tail(b)
tail(case x { p1 -> e1, p2 -> e2, ... }) = tail(e1) AND tail(e2) AND ...
tail(receive { p -> e } after t -> b) = tail(e) AND tail(b)
tail(let x = v; rest)   = tail(rest)  -- the rest after the let
tail(f(args))           = TAIL CALL if f is the current function
tail(other)             = NOT a tail call
```

**Confidence:** HIGH for self-recursive TCE (well-understood MIR transformation, textbook algorithm). MEDIUM for mutual TCE via `musttail` (LLVM support is maturing per 2025 developments but constraints are tricky; defer to future).

**Dependencies:** Touches MIR lowering in `snow-codegen/src/mir/lower.rs`. No runtime changes needed. Independent of all other v1.9 features.

---

## Differentiators

Features that set Snow apart. Not expected, but valued.

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| `?` operator works with pipe `\|>` | `read_file(path) \|> parse_json?` -- error propagation in pipelines. Unique combination of Snow's pipe + Rust's `?`. | Med | Requires careful precedence: `(read_file(path) \|> parse_json)?` applies `?` to the pipe result. Natural if `?` binds tighter than `\|>`. |
| Type-safe receive timeouts | Erlang's `after` body can return any type at runtime. Snow's type checker ensures timeout body type matches receive arm types at compile time. | Low | Already have the type checking infrastructure. Unify timeout body type with arm types in typeck. |
| `@tailrec` compile-time guarantee | Unlike Rust (no guarantee) or C (best-effort), Snow can emit a compile error if a function annotated `@tailrec` is NOT tail-recursive. Scala does this. Catches bugs early. | Med | Optional annotation. MIR analysis: verify all recursive calls are in tail position. If not, emit error with the specific non-tail call site. |
| `Timer.send_after` with typed Pid | Erlang's `send_after` is untyped. Snow's `Timer.send_after(pid: Pid<M>, ms: Int, msg: M)` ensures the delayed message matches the target actor's expected type at compile time. | Low | Natural extension of Snow's typed Pid system. No extra work needed -- the existing type checker handles this. |
| `List.sort` with Ord trait default | `List.sort(list)` without explicit comparator when element type implements Ord. Convenience for common case. | Med | Requires trait-dispatched comparator generation. Can be added after basic `List.sort(list, cmp_fn)` works. |
| `Math` as a module, not global functions | `Math.abs(x)` instead of `abs(x)`. Keeps the global namespace clean. Matches Erlang (`math:sqrt/1`) and Python (`math.sqrt(x)`). | Low | Module system already supports qualified access. |

---

## Anti-Features

Features to explicitly NOT build.

| Anti-Feature | Why Avoid | What to Do Instead |
|--------------|-----------|-------------------|
| `try`/`catch` exception handling | Snow uses `Result<T,E>` for recoverable errors and panics for unrecoverable errors. Adding exceptions creates two competing error models, dilutes the `?` operator's value, and makes control flow unpredictable in an actor system. | Keep `Result<T,E>` + `?` operator. Use `case` for explicit handling. |
| Implicit error type conversion in `?` | Rust's `From` trait in `?` is powerful but requires designing a `From` trait, which is a significant type system feature. | For v1.9, require exact error type match in `?`. Add `From`-based conversion as a separate future milestone feature. |
| `Try` trait (generalizing `?` to arbitrary types) | Rust's `Try` trait v2 generalizes `?` beyond Result/Option to any type implementing the trait. Took Rust years to stabilize. Massive design surface. | Hardcode `?` for `Result` and `Option` only. Sufficient for years of use. |
| Generic timer wheel / scheduling framework | Erlang's `timer` module is a gen_server with complex interval management, ETS tables, and timer references. Over-engineering for v1.9. | Provide `Timer.sleep` and `Timer.send_after` as thin wrappers. Build scheduling abstractions in a future milestone. |
| Lazy evaluation for collections | Haskell's lazy lists are powerful but add enormous complexity (thunks, space leaks, debugging difficulty). Incompatible with Snow's eager evaluation model. | Keep eager evaluation. All collection operations return fully materialized collections. Add lazy iterators as a separate future feature if benchmarking shows need. |
| Mutual tail-call elimination in v1.9 | Requires `musttail` or trampolining. The signature-matching constraints of `musttail` are complex and error-prone. Self-recursive TCE covers 95%+ of real use cases including all actor loops. | Implement self-recursive TCE only. Document that mutual recursion does not get TCE. Add `musttail`-based mutual TCE in a future milestone. |
| `List.sort` without explicit comparator (as the ONLY API) | Implicit sorting relying on Ord trait dispatch is convenient but requires generating comparator functions from trait impls at compile time. | Require explicit comparator for v1.9: `List.sort(list, fn(a, b) -> compare(a, b))`. Add defaulting `List.sort(list)` as a differentiator later. |
| Mutable sort (in-place) | Snow collections are immutable. In-place sort violates this core principle. | `List.sort` returns a new sorted list. The runtime can optimize internally but the API is immutable. |

---

## Feature Dependencies

```
Math stdlib                         (INDEPENDENT -- no deps on other v1.9 features)
  +-- Math.abs, min, max, pow, sqrt, floor, ceil, round, pi
  +-- Int.to_float, Float.to_int

? operator                          (INDEPENDENT -- uses existing Result/Option/case/return)
  +-- Lexer: ? token
  +-- Parser: TryExpr postfix node
  +-- Typeck: Result/Option check, return type check
  +-- MIR: desugar to Case + Return
  :
  +-> ? in pipe expressions         (DEPENDS on ? operator base)

Receive timeouts (after clause)     (PARTIALLY DONE -- needs codegen fix only)
  |
  +-> Timer.sleep                   (DEPENDS on receive timeouts working)
  |
  +-> Timer.send_after              (DEPENDS on working actor send + new runtime timer code)

Collection ops                      (INDEPENDENT -- extends existing collection runtime)
  +-- List.sort                     (needs 2-arg comparator callback convention)
  +-- List.find                     (needs Option return from runtime -- SnowOption exists)
  +-- List.zip                      (needs Tuple creation in runtime)
  +-- String.split / join           (new runtime functions)
  +-- String.to_int / to_float      (new runtime functions returning SnowOption)
  +-- Map.merge / to_list / from_list (new runtime functions)
  +-- Set.difference / to_list / from_list (new runtime functions)

Tail-call elimination               (INDEPENDENT -- MIR transformation, no runtime changes)
  +-- Tail position analysis
  +-- MIR loop transformation for self-recursion
  :
  +-> @tailrec annotation           (DEPENDS on tail position analysis)
```

**Critical dependency chain:** Receive timeouts (fix) -> Timer.sleep -> Timer.send_after

**Fully independent features (can be parallelized):**
- Math stdlib
- `?` operator
- Collection ops
- Tail-call elimination
- (Each is independent of the others AND independent of the timeout chain)

---

## MVP Recommendation

### Must build (core of v1.9):

1. **Tail-call elimination (self-recursive)** -- Without this, actor loops are ticking time bombs. Every actor in every Snow program depends on this. HIGHEST PRIORITY despite being the most complex.

2. **`?` operator for Result/Option** -- The single biggest ergonomic win. Transforms error handling from verbose case expressions to clean linear code. Medium complexity but high reward.

3. **Receive timeouts (complete the after clause)** -- Already 80% implemented. Fixes a gap where syntax parses but does not execute. Lowest effort, unblocks Timer primitives.

4. **Math stdlib (abs, min, max, pow, sqrt, floor, ceil, round, pi, type conversions)** -- Low effort, high value. Users need basic math immediately.

5. **Core collection ops** -- List.sort, List.find, List.any, List.all, List.contains, String.split, String.join, String.to_int, String.to_float

### Should build (completes the milestone):

6. **Timer.sleep** -- Thin wrapper over receive timeout. Near-zero effort once timeouts work.

7. **Timer.send_after** -- More effort (runtime timer infrastructure) but essential for real actor patterns like heartbeats, retries, delayed processing.

8. **Extended collection ops** -- List.zip, List.flat_map, List.enumerate, List.take, List.drop, Map.merge, Map.to_list, Map.from_list, Set.difference, Set.to_list, Set.from_list

### Defer to future milestone:

- Mutual tail-call elimination (LLVM `musttail`)
- `From` trait for `?` error type conversion
- Timer.cancel / Timer.read / Timer.send_interval
- Trigonometric/logarithmic math functions
- Lazy iterators
- Generic sorting without explicit comparator
- `@tailrec` annotation (can be added any time after TCE works)

---

## Complexity Assessment

| Feature | Estimated Effort | Risk | Notes |
|---------|-----------------|------|-------|
| Math stdlib (all functions) | 2-3 days | LOW | Follows established intrinsic/runtime/builtin pattern. Mostly calls to libc. |
| `?` operator | 3-5 days | MEDIUM | Touches lexer, parser, typeck, MIR. Precedence interactions with pipe need care. |
| Receive timeout codegen fix | 0.5-1 day | LOW | Infrastructure exists. Add null-check branch to timeout body codegen. |
| Timer.sleep | 0.5 day | LOW | Thin wrapper over receive(-1) with timeout. |
| Timer.send_after | 2-3 days | MEDIUM | New runtime code: spawn background actor, sleep, send. Timer reference type. |
| List.sort | 2-3 days | MEDIUM | Merge sort with callback comparator. Most complex collection op. |
| Other List ops (find, any, all, zip, etc.) | 3-4 days | LOW | Each is a short runtime function. Batch implementation. |
| String split/join/to_int/to_float | 1-2 days | LOW | Standard string operations in Rust runtime. |
| Map/Set extended ops | 2-3 days | LOW | Follow existing Map/Set implementation patterns. |
| Self-recursive TCE | 4-6 days | **HIGH** | MIR analysis for tail position + loop transformation. Must handle all expression forms correctly. Most technically challenging feature. |

**Total estimated effort:** 20-30 days

**Key risks:**
1. **Tail-call elimination correctness.** Tail position analysis must be correct for ALL expression forms (if/else, case, receive, blocks, let-chains). Missing a case means silent stack overflow in production. Extensive testing required.
2. **`?` operator precedence.** Interaction with `|>` pipe, `.` field access, and function call `()` must be intuitive. Wrong precedence causes confusing parse errors.
3. **Timer.send_after type safety.** The delayed message must match the target actor's type. With typed `Pid<M>`, this should be enforced by the existing type system, but the runtime timer actor is untyped internally.

---

## Sources

### ? Operator / Error Propagation
- [Rust Reference: Operator Expressions](https://doc.rust-lang.org/reference/expressions/operator-expr.html) -- HIGH confidence
- [Rust by Example: ? Operator](https://doc.rust-lang.org/rust-by-example/std/result/question_mark.html) -- HIGH confidence
- [Rust RFC 3058: Try Trait v2](https://rust-lang.github.io/rfcs/3058-try-trait-v2.html) -- HIGH confidence
- [Swift Error Handling Rationale](https://github.com/swiftlang/swift/blob/main/docs/ErrorHandlingRationale.md) -- HIGH confidence
- [Swift Typed Throws vs Rust](https://alejandromp.com/development/blog/rust-error-handling-swift-script/) -- MEDIUM confidence
- [Kotlin runCatching API](https://kotlinlang.org/api/core/kotlin-stdlib/kotlin/run-catching.html) -- HIGH confidence
- [Desugaring the Try Operator (Rust)](https://tech.loveholidays.com/beneath-the-icing-in-rust-desugaring-the-try-operator-c4d0c2aea3c1) -- MEDIUM confidence

### Receive Timeouts
- [Erlang System Documentation: Expressions (receive...after)](https://www.erlang.org/doc/system/expressions.html) -- HIGH confidence
- [Erlang Programming/Timeouts Wikibook](https://en.wikibooks.org/wiki/Erlang_Programming/Timeouts) -- MEDIUM confidence
- [Elixir Process module v1.19.5](https://hexdocs.pm/elixir/Process.html) -- HIGH confidence

### Timer Primitives
- [Erlang timer module stdlib v7.2](https://www.erlang.org/doc/apps/stdlib/timer.html) -- HIGH confidence
- [Elixir Process.send_after docs](https://hexdocs.pm/elixir/Process.html) -- HIGH confidence
- [Elixir School: TIL about Process.send_after](https://elixirschool.com/blog/til-send-after) -- MEDIUM confidence
- [OTP timer modernization PR #4811](https://github.com/erlang/otp/pull/4811) -- MEDIUM confidence

### Math Stdlib
- [Erlang math module stdlib v7.2](https://www.erlang.org/doc/apps/stdlib/math.html) -- HIGH confidence
- [Python math module](https://docs.python.org/3/library/math.html) -- HIGH confidence
- [Kotlin kotlin.math package](https://kotlinlang.org/api/core/kotlin-stdlib/kotlin.math/) -- HIGH confidence
- [C mathematical functions (Wikipedia)](https://en.wikipedia.org/wiki/C_mathematical_functions) -- MEDIUM confidence

### Collection Operations
- [Higher-order list operations in Racket and Haskell](https://matt.might.net/articles/higher-order-list-operations/) -- MEDIUM confidence
- Elixir Enum module (training data) -- MEDIUM confidence
- Haskell Prelude list functions (training data) -- MEDIUM confidence

### Tail-Call Elimination
- [Tail call (Wikipedia)](https://en.wikipedia.org/wiki/Tail_call) -- MEDIUM confidence
- [LLVM musttail implementation blog (2025)](https://blog.reverberate.org/2025/02/10/tail-call-updates.html) -- HIGH confidence
- [Erlang-questions: TCO discussion (Robert Virding)](http://erlang.org/pipermail/erlang-questions/2016-October/090663.html) -- MEDIUM confidence
- [Elixir Forum: TCO in Elixir/Erlang](https://elixirforum.com/t/tail-call-optimization-in-elixir-erlang-not-as-efficient-and-important-as-you-probably-think/880) -- MEDIUM confidence
- [LLVM Guaranteed Efficient Tail Calls (design notes)](https://nondot.org/sabre/LLVMNotes/GuaranteedEfficientTailCalls.txt) -- HIGH confidence

### Snow Codebase (direct inspection)
- `crates/snow-parser/src/ast/expr.rs` -- `AfterClause` already parsed, `ReceiveExpr` has `after_clause()` method
- `crates/snow-codegen/src/codegen/expr.rs` line 129 -- `timeout_body: _` (ignored in codegen)
- `crates/snow-codegen/src/codegen/intrinsics.rs` -- `snow_actor_receive(timeout_ms)` declared
- `crates/snow-rt/src/actor/mod.rs` line 315 -- `snow_actor_receive` returns null on timeout
- `crates/snow-rt/src/collections/list.rs` -- existing list operations follow alloc+copy immutable pattern
- `crates/snow-typeck/src/builtins.rs` -- existing pattern for registering built-in functions
- `crates/snow-common/src/token.rs` line 197 -- `after` keyword already in lexer
- `crates/snow-codegen/src/mir/mod.rs` line 263 -- `ActorReceive` MIR node has `timeout_ms` and `timeout_body` fields

---
*Feature research for: Snow Language v1.9 Stdlib & Ergonomics*
*Researched: 2026-02-09*
