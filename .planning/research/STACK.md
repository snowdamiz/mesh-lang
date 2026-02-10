# Technology Stack: v1.9 Stdlib & Ergonomics

**Project:** Snow compiler -- stdlib math, ? operator, receive timeout completion, timer primitives, collection operations, tail-call elimination
**Researched:** 2026-02-09
**Confidence:** HIGH (based on direct codebase analysis of all 12 crates, LLVM/Inkwell API verification, and platform linking research)

## Executive Summary

v1.9 requires **ZERO new Rust crate dependencies**. Every feature is implemented through:
1. **LLVM math intrinsics** (declared via Inkwell's `module.add_function("llvm.sqrt.f64", ...)` -- no libm crate needed)
2. **Rust standard library** math via `f64::sin()` etc. in `snow-rt` runtime functions (links against system libm automatically)
3. **Existing Inkwell 0.8 APIs** for tail-call elimination (`set_tail_call_kind` with `LLVMTailCallKindMustTail`, available on `llvm21-1` feature)
4. **Internal compiler changes** to parser, typeck, MIR, and codegen (no external dependencies)
5. **Internal runtime additions** to `snow-rt` (no external dependencies)

The only infrastructure change is adding `-lm` to the linker invocation on Linux in `snow-codegen/src/link.rs` (macOS links libm automatically via libSystem). This is a one-line change.

## Recommended Stack

### Core Framework (NO CHANGES)

| Technology | Version | Purpose | Status |
|------------|---------|---------|--------|
| Rust | stable 2021 edition | Compiler implementation | No change |
| Inkwell | 0.8.0 (`llvm21-1`) | LLVM IR generation | No change |
| LLVM | 21.1 | Backend codegen + optimization | No change |
| Rowan | 0.16 | CST for parser | No change |
| ena | 0.14 | Union-find for HM type inference | No change |
| ariadne | 0.6 | Diagnostic error reporting | No change |
| corosensei | 0.3 | Stackful coroutines for actors | No change |

### Runtime (snow-rt) -- NO NEW DEPENDENCIES

| Technology | Version | Purpose | Status |
|------------|---------|---------|--------|
| crossbeam-deque | 0.8 | Work-stealing scheduler | No change |
| crossbeam-utils | 0.8 | Concurrent utilities | No change |
| crossbeam-channel | 0.5 | MPMC channels | No change |
| parking_lot | 0.12 | Efficient mutexes | No change |
| rustc-hash | 2 | Fast hashing | No change |

### What NOT to Add

| Crate | Why NOT |
|-------|---------|
| `libm` (Rust crate) | Unnecessary -- Rust's `f64` methods (`sin()`, `cos()`, `sqrt()`, etc.) use LLVM intrinsics when compiled with optimizations, and link against system libm in debug. The `libm` crate is only needed for `no_std` environments, which Snow is not. |
| `num-traits` | Unnecessary -- Snow's math stdlib only needs concrete `f64` operations, not generic numeric traits. Direct `f64` method calls are simpler and faster. |
| Any sort crate | Unnecessary -- `snow_list_sort` can use Rust's `slice::sort_by` on the list's `[u64]` data region with a comparator callback. This is a ~30-line runtime function, not worth a dependency. |
| `timer`/`tokio-timer` | Unnecessary -- timer primitives (`sleep`, `send_after`) are implemented as dedicated runtime functions using `std::thread::sleep` and scheduler integration. The actor scheduler already has deadline-based wakeup. |
| `regex` | Not needed for v1.9. String `split` and `join` use exact string matching, not patterns. |

## Feature-by-Feature Stack Analysis

### 1. Math Stdlib (libm FFI)

**Approach:** Two-layer strategy -- LLVM intrinsics for hot-path math, runtime functions for the rest.

**Layer 1: LLVM Built-in Math Intrinsics (codegen layer)**

LLVM provides built-in intrinsics for common math functions that map directly to hardware instructions where available. Declare them in `snow-codegen/src/codegen/intrinsics.rs` using the same pattern as existing runtime function declarations:

```rust
// In declare_intrinsics():
let f64_to_f64 = f64_type.fn_type(&[f64_type.into()], false);
let f64_f64_to_f64 = f64_type.fn_type(&[f64_type.into(), f64_type.into()], false);

// LLVM recognizes the "llvm." prefix as built-in intrinsics
module.add_function("llvm.sqrt.f64", f64_to_f64, None);
module.add_function("llvm.sin.f64", f64_to_f64, None);
module.add_function("llvm.cos.f64", f64_to_f64, None);
module.add_function("llvm.pow.f64", f64_f64_to_f64, None);
module.add_function("llvm.exp.f64", f64_to_f64, None);
module.add_function("llvm.exp2.f64", f64_to_f64, None);
module.add_function("llvm.log.f64", f64_to_f64, None);
module.add_function("llvm.log2.f64", f64_to_f64, None);
module.add_function("llvm.log10.f64", f64_to_f64, None);
module.add_function("llvm.fabs.f64", f64_to_f64, None);
module.add_function("llvm.floor.f64", f64_to_f64, None);
module.add_function("llvm.ceil.f64", f64_to_f64, None);
module.add_function("llvm.round.f64", f64_to_f64, None);
module.add_function("llvm.trunc.f64", f64_to_f64, None);
module.add_function("llvm.copysign.f64", f64_f64_to_f64, None);
module.add_function("llvm.minnum.f64", f64_f64_to_f64, None);
module.add_function("llvm.maxnum.f64", f64_f64_to_f64, None);
```

These are NOT external function calls -- LLVM compiles them to native instructions (e.g., `vsqrtsd` on x86-64, `fsqrt` on ARM64) or inlines platform-optimized libm implementations.

**Why LLVM intrinsics over runtime FFI:** LLVM intrinsics enable constant folding (`sqrt(4.0)` -> `2.0` at compile time), vectorization, and platform-optimal instruction selection. Runtime functions cannot be optimized by LLVM.

**Layer 2: Runtime Functions for Non-Intrinsic Math (snow-rt)**

Functions without LLVM intrinsics are implemented in `snow-rt` using Rust's `f64` methods:

```rust
// In snow-rt/src/math.rs (new file)
#[no_mangle]
pub extern "C" fn snow_math_atan2(y: f64, x: f64) -> f64 { y.atan2(x) }

#[no_mangle]
pub extern "C" fn snow_math_tan(x: f64) -> f64 { x.tan() }

#[no_mangle]
pub extern "C" fn snow_math_asin(x: f64) -> f64 { x.asin() }

#[no_mangle]
pub extern "C" fn snow_math_acos(x: f64) -> f64 { x.acos() }

#[no_mangle]
pub extern "C" fn snow_math_atan(x: f64) -> f64 { x.atan() }

#[no_mangle]
pub extern "C" fn snow_math_sinh(x: f64) -> f64 { x.sinh() }

#[no_mangle]
pub extern "C" fn snow_math_cosh(x: f64) -> f64 { x.cosh() }

#[no_mangle]
pub extern "C" fn snow_math_tanh(x: f64) -> f64 { x.tanh() }
```

Rust's `f64` methods call into the platform's libm. On macOS, libm is part of libSystem (linked automatically). On Linux, `-lm` is needed at link time.

**Linker change for Linux (snow-codegen/src/link.rs):**

Currently line 52 adds the object file and library path. Add `-lm` on non-macOS:

```rust
// After existing linker args
#[cfg(not(target_os = "macos"))]
{
    cmd.arg("-lm");
}
```

This is the ONLY infrastructure change needed for math stdlib.

**Math module in typeck (snow-typeck/src/infer.rs and builtins.rs):**

Register a `Math` module in `stdlib_modules()` following the existing pattern (String, IO, etc.):

```rust
let mut math_mod = FxHashMap::default();
math_mod.insert("sqrt".into(), Scheme::mono(Ty::fun(vec![Ty::float()], Ty::float())));
math_mod.insert("sin".into(), Scheme::mono(Ty::fun(vec![Ty::float()], Ty::float())));
math_mod.insert("cos".into(), Scheme::mono(Ty::fun(vec![Ty::float()], Ty::float())));
// ... etc for all math functions
math_mod.insert("pi".into(), Scheme::mono(Ty::float()));  // constant
math_mod.insert("e".into(), Scheme::mono(Ty::float()));   // constant
modules.insert("Math".into(), math_mod);
```

**MIR lowering (snow-codegen/src/mir/lower.rs):**

Map `Math.sqrt(x)` calls to `llvm.sqrt.f64` intrinsic calls. Map `Math.atan2(y, x)` calls to `snow_math_atan2` runtime calls. The existing `known_functions` map handles this -- add entries:

```rust
self.known_functions.insert("Math.sqrt".into(), ...);
// which lowers to a Call on the "llvm.sqrt.f64" function
```

**Constants:** `Math.pi` and `Math.e` lower to `MirExpr::FloatLit(std::f64::consts::PI, MirType::Float)` during MIR lowering.

**Confidence:** HIGH -- LLVM math intrinsics are decades-old stable APIs. Inkwell's `add_function("llvm.sqrt.f64", ...)` works identically to how existing runtime functions are declared. Verified that macOS includes libm in libSystem; Linux needs `-lm`.

---

### 2. ? Operator (Error Propagation)

**Approach:** Parser postfix operator -> typeck validation -> MIR desugaring -> codegen as match on Result tag.

**Lexer:** The `Question` token (`?`) already exists in `snow-common/src/token.rs` (line 126) and is already mapped to `SyntaxKind::QUESTION` in `snow-parser/src/syntax_kind.rs` (line 97, 426). No lexer changes needed.

**Parser (snow-parser):**

Add `?` as a postfix operator in expression parsing. When the parser sees `?` after an expression, wrap it in a `TRY_EXPR` CST node:

```
expr?  =>  TRY_EXPR { inner: expr, QUESTION }
```

Add a new `SyntaxKind::TRY_EXPR` variant and an AST accessor `TryExpr` with method `inner() -> Option<Expr>`.

**Typeck (snow-typeck/src/infer.rs):**

When inferring a `TryExpr`:
1. Infer the inner expression's type
2. Verify it is `Result<T, E>` -- error if not
3. Verify the enclosing function's return type is also `Result<_, E>` with compatible error type -- error if not
4. The `TryExpr` itself has type `T` (the Ok payload)

This is a ~30-line addition to `infer_expr`.

**MIR Lowering (snow-codegen/src/mir/lower.rs):**

Desugar `expr?` into:

```
match expr {
  Ok(val) => val,
  Err(e) => return Err(e)
}
```

Since `Result` is already a sum type in Snow with `Ok` and `Err` variants, this desugars to the existing `MirExpr::Match` + `MirExpr::Return` + `MirExpr::ConstructVariant` nodes. No new MIR nodes needed.

**Codegen:** No codegen changes needed -- the desugared match/return uses existing codegen paths for sum type pattern matching.

**What already exists:**
- `Result<T, E>` as `Ty::result(ok, err)` -- typeck line 119
- `Ok(val)` and `Err(e)` constructors -- working sum type codegen
- Pattern matching on Result arms -- working
- Return from function -- working

**Confidence:** HIGH -- the `?` operator desugars entirely to existing constructs (match + return). The only new work is parser recognition and typeck validation.

---

### 3. Receive Timeout Completion

**Approach:** Wire the already-parsed `timeout_body` through to codegen.

**Current state:**
- Parser: `after` clause is already parsed (verified: `recv.after_clause()` exists in AST)
- Typeck: `after` clause timeout and body are already type-checked (lines 5985-5991 of infer.rs)
- MIR: `timeout_ms` and `timeout_body` are already lowered to `MirExpr::ActorReceive` fields (lines 7160-7172 of lower.rs)
- Codegen: `timeout_body` is explicitly **ignored** -- `timeout_body: _` on line 129 of expr.rs

**What needs to change (codegen only):**

In `codegen_actor_receive` (expr.rs, line 1540), after the `snow_actor_receive()` call returns a pointer:
1. Check if the pointer is null (timeout occurred)
2. If null AND timeout_body is `Some(body)`: codegen the timeout body expression
3. If null AND no timeout_body: return a default/unit value
4. If non-null: proceed with existing message pattern matching

This is a ~20-line change to add a null-check branch:

```rust
// After receiving msg_ptr:
if let Some(timeout_body) = timeout_body {
    // Create basic blocks for null check
    let timeout_bb = self.context.append_basic_block(current_fn, "timeout");
    let message_bb = self.context.append_basic_block(current_fn, "message");
    let merge_bb = self.context.append_basic_block(current_fn, "merge");

    let is_null = self.builder.build_is_null(msg_ptr, "is_timeout")?;
    self.builder.build_conditional_branch(is_null, timeout_bb, message_bb)?;

    // Timeout path: codegen the timeout body
    self.builder.position_at_end(timeout_bb);
    let timeout_val = self.codegen_expr(timeout_body)?;
    self.builder.build_unconditional_branch(merge_bb)?;

    // Message path: existing pattern matching
    self.builder.position_at_end(message_bb);
    let msg_val = /* existing message loading code */;
    self.builder.build_unconditional_branch(merge_bb)?;

    // Merge with phi node
    self.builder.position_at_end(merge_bb);
    let phi = self.builder.build_phi(result_llvm_type, "recv_result")?;
    phi.add_incoming(&[(&timeout_val, timeout_bb), (&msg_val, message_bb)]);
}
```

**Runtime:** No changes -- `snow_actor_receive` already returns null on timeout (lines 334, 396 of actor/mod.rs). The timeout_ms parameter is already passed through.

**Confidence:** HIGH -- all layers except final codegen emission are already implemented. This is purely wiring the existing `timeout_body` field through to LLVM IR.

---

### 4. Timer Primitives (sleep, send_after)

**Approach:** Runtime functions in `snow-rt`, declared as intrinsics, exposed as `Timer` stdlib module.

**`Timer.sleep(ms)`** -- Pause the current actor for `ms` milliseconds:

```rust
// In snow-rt/src/timer.rs (new file)
#[no_mangle]
pub extern "C" fn snow_timer_sleep(ms: i64) {
    if ms <= 0 { return; }

    let my_pid = match stack::get_current_pid() {
        Some(pid) => pid,
        None => {
            // Main thread: just sleep the OS thread
            std::thread::sleep(std::time::Duration::from_millis(ms as u64));
            return;
        }
    };

    // Actor context: set deadline and yield repeatedly until it passes
    let deadline = std::time::Instant::now()
        + std::time::Duration::from_millis(ms as u64);

    let sched = global_scheduler();
    loop {
        if std::time::Instant::now() >= deadline {
            return;
        }
        // Set Waiting state and yield
        if let Some(proc) = sched.get_process(my_pid) {
            proc.lock().state = ProcessState::Waiting;
        }
        stack::yield_current();
    }
}
```

This integrates with the existing scheduler -- the actor yields cooperatively and is periodically resumed by the scheduler's sweep loop, checking if its deadline has passed. No new scheduler infrastructure needed.

**`Timer.send_after(pid, ms, message)`** -- Send a message to `pid` after `ms` milliseconds:

```rust
#[no_mangle]
pub extern "C" fn snow_timer_send_after(
    target_pid: u64,
    delay_ms: i64,
    msg_ptr: *const u8,
    msg_size: u64,
) {
    // Deep-copy the message data before spawning the timer actor
    let data = if msg_ptr.is_null() || msg_size == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(msg_ptr, msg_size as usize).to_vec() }
    };

    let sched = global_scheduler();

    // Spawn a lightweight timer actor that sleeps then sends
    extern "C" fn timer_entry(args: *const u8) {
        // Decode: [u64 target_pid, u64 delay_ms, u64 data_len, u8... data]
        // ... sleep, then call snow_actor_send
    }

    // Encode args and spawn
    // ...
}
```

**Alternative (simpler):** Implement `send_after` as a spawn of a minimal actor that calls `snow_timer_sleep` then `snow_actor_send`. This reuses existing primitives entirely. The timer actor approach is exactly how Erlang/OTP implements `send_after` internally.

**Typeck:** Add `Timer` module to `stdlib_modules()`:

```rust
let mut timer_mod = FxHashMap::default();
timer_mod.insert("sleep".into(), Scheme::mono(Ty::fun(vec![Ty::int()], Ty::unit())));
timer_mod.insert("send_after".into(), Scheme::mono(
    Ty::fun(vec![Ty::pid_untyped(), Ty::int(), Ty::Var(...)], Ty::unit())
));
modules.insert("Timer".into(), timer_mod);
```

**Confidence:** HIGH -- `sleep` is trivial using existing scheduler yield mechanics. `send_after` is a composition of existing spawn + sleep + send primitives. No new infrastructure.

---

### 5. Collection Operations (sort, split, join, find, zip)

**Approach:** All implemented as `extern "C"` functions in `snow-rt` using existing list/string infrastructure. No external crates.

**List operations (snow-rt/src/collections/list.rs):**

| Function | Signature | Implementation |
|----------|-----------|----------------|
| `snow_list_sort` | `(list: *mut u8, cmp_fn: *mut u8, env: *mut u8) -> *mut u8` | Copy list data to temp `Vec<u64>`, call `sort_by` with cmp callback, allocate new list |
| `snow_list_find` | `(list: *mut u8, pred_fn: *mut u8, env: *mut u8) -> u64` | Linear scan, return first matching element as Option-encoded u64 |
| `snow_list_zip` | `(list_a: *mut u8, list_b: *mut u8) -> *mut u8` | Allocate tuple list of `min(len_a, len_b)` pairs |
| `snow_list_any` | `(list: *mut u8, pred_fn: *mut u8, env: *mut u8) -> i8` | Short-circuit linear scan returning boolean |
| `snow_list_all` | `(list: *mut u8, pred_fn: *mut u8, env: *mut u8) -> i8` | Short-circuit linear scan returning boolean |
| `snow_list_flat_map` | `(list: *mut u8, fn_ptr: *mut u8, env: *mut u8) -> *mut u8` | Map producing lists, then concat all |
| `snow_list_take` | `(list: *mut u8, n: i64) -> *mut u8` | Copy first n elements to new list |
| `snow_list_drop` | `(list: *mut u8, n: i64) -> *mut u8` | Copy elements after n to new list |
| `snow_list_contains` | `(list: *mut u8, elem: u64, eq_fn: *mut u8) -> i8` | Linear scan with equality callback |
| `snow_list_chunk` | `(list: *mut u8, size: i64) -> *mut u8` | Partition into sublists of given size |

**`snow_list_sort` implementation detail:**

```rust
#[no_mangle]
pub extern "C" fn snow_list_sort(
    list: *mut u8,
    cmp_fn: *mut u8,
    env_ptr: *mut u8,
) -> *mut u8 {
    type BareCmp = unsafe extern "C" fn(u64, u64) -> i64;
    type ClosureCmp = unsafe extern "C" fn(*mut u8, u64, u64) -> i64;

    unsafe {
        let len = list_len(list) as usize;
        if len <= 1 { return list; }

        // Copy data to a mutable Vec for sorting
        let src = list_data(list);
        let mut data: Vec<u64> = Vec::with_capacity(len);
        for i in 0..len {
            data.push(*src.add(i));
        }

        // Sort using Rust's stable sort (TimSort)
        if env_ptr.is_null() {
            let f: BareCmp = std::mem::transmute(cmp_fn);
            data.sort_by(|a, b| {
                let cmp = f(*a, *b);
                if cmp < 0 { std::cmp::Ordering::Less }
                else if cmp > 0 { std::cmp::Ordering::Greater }
                else { std::cmp::Ordering::Equal }
            });
        } else {
            let f: ClosureCmp = std::mem::transmute(cmp_fn);
            data.sort_by(|a, b| {
                let cmp = f(env_ptr, *a, *b);
                if cmp < 0 { std::cmp::Ordering::Less }
                else if cmp > 0 { std::cmp::Ordering::Greater }
                else { std::cmp::Ordering::Equal }
            });
        }

        // Allocate new list from sorted data
        alloc_list_from(data.as_ptr(), len as u64, len as u64)
    }
}
```

**Why Rust's `sort_by` and not an external sort crate:** Rust's standard library sort is a well-optimized TimSort implementation. At the scale Snow lists operate (hundreds to low thousands of elements), it is optimal. No crate adds value here.

**String operations (snow-rt/src/string.rs):**

| Function | Signature | Implementation |
|----------|-----------|----------------|
| `snow_string_split` | `(s: *const SnowString, delim: *const SnowString) -> *mut u8` | Rust `str::split()`, collect into Snow List of SnowStrings |
| `snow_string_join` | `(list: *mut u8, sep: *const SnowString) -> *mut SnowString` | Iterate list elements (SnowString ptrs), join with separator |

**`snow_string_split` returns a `List<String>`**, using the same GC-allocated list format as existing lists. Each element is a `u64` that is actually a pointer to a `SnowString`.

**`snow_string_join` takes a `List<String>` and a separator**, iterating the list and concatenating with the separator between elements.

**Typeck:** Extend `String` and `List` module entries:

```rust
string_mod.insert("split".into(), Scheme::mono(
    Ty::fun(vec![Ty::string(), Ty::string()], Ty::list(Ty::string()))
));
string_mod.insert("join".into(), Scheme::mono(
    Ty::fun(vec![Ty::list(Ty::string()), Ty::string()], Ty::string())
));
list_mod.insert("sort".into(), /* generic sort type */);
list_mod.insert("find".into(), /* generic find type */);
// etc.
```

**Generic collection operations (sort, find, zip, etc.):** These need generic type signatures in the type checker. The existing generic infrastructure (HM type inference with `Ty::Var`) handles this -- sort takes `(List<A>, fn(A, A) -> Int) -> List<A>`.

**Confidence:** HIGH -- follows exact same pattern as existing `snow_list_map`, `snow_list_filter`, `snow_list_reduce` which use the same callback function pointer convention (bare fn vs closure with env_ptr).

---

### 6. Tail-Call Elimination (TCE)

**Approach:** Detect tail calls during MIR lowering, mark them with `musttail` via Inkwell's `set_tail_call_kind` API, falling back to a loop-rewrite optimization for self-recursive tail calls.

**Inkwell API (verified for llvm21-1):**

Inkwell's `CallSiteValue` provides `set_tail_call_kind(kind: LLVMTailCallKind)` which is available on the `llvm21-1` feature (confirmed in Inkwell docs). The `LLVMTailCallKind` enum includes:
- `LLVMTailCallKindNone` -- no tail call
- `LLVMTailCallKindTail` -- hint (optimizer may eliminate)
- `LLVMTailCallKindMustTail` -- guaranteed elimination (fatal error if backend cannot)

**Strategy: Two-Tier TCE**

**Tier 1: Self-Recursive Tail Call -> Loop Rewrite (MIR level)**

The most common case in Snow is self-recursive functions (actor loops, list processing). These can be reliably eliminated at the MIR level by rewriting them as loops:

```
def factorial(n, acc) =
  if n <= 1 do acc
  else factorial(n - 1, n * acc) end

# Rewrites to:
def factorial(n, acc) =
  loop:
    if n <= 1 do return acc end
    (n, acc) = (n - 1, n * acc)
    goto loop
```

This is implemented in MIR lowering by:
1. Detecting that the function body's tail expression is a self-call
2. Replacing the self-call with parameter reassignment + continue in a synthetic loop
3. This produces `MirExpr::While` + `MirExpr::Let` (parameter rebinding) -- existing codegen handles these

**Why MIR-level rewrite over LLVM `musttail`:** LLVM's `musttail` has strict requirements -- the caller and callee must have identical signatures, the call must immediately precede a `ret`, and some backends (PowerPC, some ARM variants) cannot guarantee elimination. The MIR loop rewrite is 100% reliable across all targets.

**Tier 2: General Tail Calls -> LLVM `tail` Hint (Codegen level)**

For non-self tail calls (e.g., mutual recursion), annotate with `set_tail_call_kind(LLVMTailCallKindTail)` as a hint:

```rust
// In codegen_call or codegen_closure_call:
if is_tail_position {
    call_site.set_tail_call_kind(
        inkwell::LLVMTailCallKind::LLVMTailCallKindTail
    );
}
```

The `tail` hint allows LLVM's optimizer to eliminate the call if possible. It does not guarantee elimination but does not produce fatal errors on any backend.

**Tail Position Detection (MIR lowering):**

A call is in tail position if it is the last expression in a function body, the last expression of a branch in an if/match that is itself in tail position, or the last expression of a let-body chain.

Add a boolean `is_tail` parameter to `lower_expr` that tracks whether the current expression is in tail position:

```rust
fn lower_expr(&mut self, expr: &Expr, is_tail: bool) -> MirExpr {
    match ... {
        // The last expression in a function body is tail
        If { then, else_, .. } => {
            MirExpr::If {
                then_body: self.lower_expr(then, is_tail),
                else_body: self.lower_expr(else_, is_tail),
                ...
            }
        }
        Call { .. } if is_tail => {
            // Check if self-recursive -> loop rewrite
            // Otherwise mark for tail hint
            ...
        }
    }
}
```

**What NOT to do:** Do NOT use `LLVMTailCallKindMustTail` for general tail calls. It will produce fatal backend errors on some architectures. Reserve `musttail` only for cases where we can guarantee the call meets LLVM's strict requirements (same return type, same calling convention, immediately followed by ret).

**Confidence:** HIGH for Tier 1 (self-recursive loop rewrite -- well-understood transformation). MEDIUM for Tier 2 (LLVM `tail` hint -- behavior depends on optimizer and target, but never causes errors).

---

## Integration Points with Existing Crates

### snow-common (no changes)

No new types or utilities needed. The `Question` token already exists.

### snow-lexer (no changes)

`?` is already lexed as `TokenKind::Question`.

### snow-parser (additions)

| Addition | Purpose | Estimated Lines |
|----------|---------|----------------|
| `TRY_EXPR` syntax kind | CST node for `expr?` | ~5 |
| Parse `?` as postfix | In expression parser after primary | ~15 |
| `TryExpr` AST accessor | `inner() -> Option<Expr>` | ~10 |

### snow-typeck (additions)

| Addition | Purpose | Estimated Lines |
|----------|---------|----------------|
| `Math` stdlib module | Type signatures for math functions + constants | ~60 |
| `Timer` stdlib module | Type signatures for sleep, send_after | ~15 |
| `TryExpr` inference | Validate Result type, check function return | ~30 |
| Collection operation types | sort, find, zip, etc. in List/String modules | ~40 |
| String.split/join types | New entries in String module | ~10 |

### snow-codegen / MIR (additions)

| Addition | Purpose | Estimated Lines |
|----------|---------|----------------|
| `TryExpr` desugaring | `expr?` -> match Ok/Err with early return | ~40 |
| Math call routing | `Math.sqrt` -> `llvm.sqrt.f64` intrinsic | ~80 |
| Tail position tracking | `is_tail` parameter through `lower_expr` | ~50 |
| Self-recursive loop rewrite | Tail self-calls -> while loop | ~80 |

### snow-codegen / codegen (additions)

| Addition | Purpose | Estimated Lines |
|----------|---------|----------------|
| LLVM math intrinsic declarations | `llvm.sqrt.f64`, `llvm.sin.f64`, etc. | ~30 |
| Runtime math function declarations | `snow_math_atan2`, etc. | ~20 |
| Receive timeout_body codegen | Null-check branch + timeout body | ~30 |
| Timer function declarations | `snow_timer_sleep`, `snow_timer_send_after` | ~10 |
| Collection function declarations | `snow_list_sort`, `snow_string_split`, etc. | ~30 |
| Tail call annotation | `set_tail_call_kind` on calls in tail position | ~15 |

### snow-codegen / link (one-line change)

| Change | Purpose |
|--------|---------|
| Add `-lm` on Linux | Link libm for math runtime functions |

### snow-rt (additions)

| Addition | Purpose | Estimated Lines |
|----------|---------|----------------|
| `math.rs` (new module) | atan2, tan, asin, acos, atan, sinh, cosh, tanh | ~60 |
| `timer.rs` (new module) | sleep, send_after | ~80 |
| List sort, find, zip, etc. | New functions in `collections/list.rs` | ~200 |
| String split, join | New functions in `string.rs` | ~60 |

### Total Estimated Lines: ~960

## Alternatives Considered

| Category | Recommended | Alternative | Why Not |
|----------|-------------|-------------|---------|
| Math functions | LLVM intrinsics + Rust f64 methods | `libm` Rust crate | libm crate is for `no_std`; Rust std already provides f64 methods that link to system libm |
| Math functions | LLVM intrinsics for hot path | All via runtime FFI | LLVM intrinsics enable constant folding, vectorization, and platform-optimal codegen |
| ? operator | MIR desugaring to match/return | New MIR node `TryExpr` | Desugaring reuses existing codegen; new node requires new codegen path for same result |
| Timer sleep | Yield-loop with deadline | `std::thread::sleep` in actor | Thread sleep blocks the worker thread, preventing other actors from running on it |
| Timer send_after | Spawn timer actor | Scheduler timer wheel | Timer wheel adds complexity to scheduler for minimal benefit; timer actors are idiomatic Erlang/BEAM approach |
| List sort | Rust `slice::sort_by` | External sort crate | std sort is TimSort, optimal for this scale; no crate adds value |
| String split | Rust `str::split` in runtime | Regex-based split | Regex is massive dependency for exact-match splitting |
| Tail call elim | MIR loop rewrite (self-recursive) | LLVM `musttail` only | `musttail` fails on some backends (PowerPC, some ARM); loop rewrite is 100% reliable |
| Tail call elim | Two-tier (loop + hint) | Only loop rewrite | Loses opportunity for LLVM to optimize mutual recursion |

## Installation

No new dependencies to install:

```bash
# Existing build command works unchanged
cargo build -p snow-rt && cargo build -p snowc
```

The only system-level dependency is that Linux systems need libm installed (it is present on virtually all Linux installations as part of glibc/musl).

## Sources

### Primary (HIGH confidence)
- Direct codebase analysis: all 12 crates in workspace
- `snow-codegen/src/codegen/expr.rs` line 129: `timeout_body: _` explicitly ignored
- `snow-codegen/src/mir/lower.rs` lines 7160-7172: timeout_ms and timeout_body already lowered
- `snow-typeck/src/infer.rs` lines 5985-5991: after clause already type-checked
- `snow-common/src/token.rs` line 126: `Question` token exists
- `snow-parser/src/syntax_kind.rs` line 97: `QUESTION` syntax kind exists
- `snow-codegen/src/codegen/intrinsics.rs`: existing pattern for runtime function declarations
- `snow-codegen/src/link.rs`: existing linker invocation
- `snow-rt/src/collections/list.rs`: existing list operation patterns (map, filter, reduce)
- `snow-rt/src/actor/mod.rs` lines 314-419: receive timeout already returns null on timeout

### Secondary (HIGH confidence -- verified with official docs)
- [Inkwell CallSiteValue docs](https://thedan64.github.io/inkwell/inkwell/values/struct.CallSiteValue.html) -- `set_tail_call_kind` available on `llvm21-1`
- [LLVM Language Reference](https://llvm.org/docs/LangRef.html) -- `llvm.sqrt.f64`, `llvm.sin.f64`, etc. are stable built-in intrinsics
- [LLVM Math Intrinsics RFC](https://discourse.llvm.org/t/rfc-all-the-math-intrinsics/78294) -- confirms existing intrinsics and expansion plans

### Tertiary (MEDIUM confidence -- community/platform knowledge)
- macOS libm: [Linked automatically via libSystem](https://gcc-bugs.gcc.gnu.narkive.com/RDyEiHMr/gcc-lm-and-libm-a-for-mac-os-x) -- no `-lm` flag needed
- Linux libm: `-lm` required at link time, confirmed by multiple GCC/linker documentation sources
- [LLVM musttail issues](https://github.com/llvm/llvm-project/issues/108014) -- `musttail` can fail on certain backends, motivating Tier 1 loop rewrite approach

---
*Stack research for: Snow v1.9 Stdlib & Ergonomics features*
*Researched: 2026-02-09*
