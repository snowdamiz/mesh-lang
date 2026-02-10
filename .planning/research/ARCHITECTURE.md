# Architecture Patterns: v1.9 Stdlib & Ergonomics

**Domain:** Compiler feature integration -- 6 new features into existing Snow compiler/runtime
**Researched:** 2026-02-09
**Confidence:** HIGH (all analysis based on direct codebase inspection)

## Existing Architecture Summary

The Snow compiler follows a linear pipeline with clear component boundaries:

```
Source -> snow-lexer -> snow-parser (Rowan CST)
      -> snow-typeck (HM inference, trait registry)
      -> snow-codegen (MIR lowering -> LLVM IR via inkwell 0.8.0 + llvm21-1)
      -> cc linker (object + libsnow_rt.a -> executable)
```

**Key architectural patterns already established:**

1. **Builtin registration:** `snow-typeck/src/builtins.rs` registers type signatures as `Scheme` entries in `TypeEnv`, using `TyVar(N)` for polymorphic functions (N ranges from 90000-99999 to avoid collisions).

2. **Intrinsic declaration:** `snow-codegen/src/codegen/intrinsics.rs` declares all `extern "C"` functions from `snow-rt` in the LLVM module with exact C ABI signatures.

3. **MIR as IR:** `snow-codegen/src/mir/mod.rs` defines `MirExpr` variants for each language feature. New features need new MirExpr variants or reuse existing ones.

4. **Closure callback pattern:** Higher-order runtime functions (list_map, list_filter, list_eq, list_compare) accept `fn_ptr: *mut u8, env_ptr: *mut u8` pairs, dispatching to bare fn or closure based on null-check of env_ptr.

5. **Uniform u64 storage:** All collection elements are stored as `u64`. Ints are direct, floats are bitcast, pointers are ptrtoint. MIR type information drives load/store conversions in codegen.

6. **Pratt parser:** `snow-parser/src/parser/expressions.rs` uses binding power tables for infix/prefix, with postfix operations (call, field access, index) at BP=25.

---

## Feature 1: Math Stdlib

### Integration Points

This feature requires changes in exactly 3 files, following the established pattern used by string/file/JSON stdlib functions.

**No parser changes needed.** Math functions are called as normal functions (`Math.sin(x)` or `math_sin(x)`) through the existing module-qualified call path.

### Architecture

```
builtins.rs: Register math_sin, math_cos, etc. as Scheme::mono(Ty::fun(vec![Ty::float()], Ty::float()))
intrinsics.rs: Declare snow_math_sin, snow_math_cos, etc. as extern "C" fn(f64) -> f64
snow-rt: New file crates/snow-rt/src/math.rs with #[no_mangle] pub extern "C" wrappers around libm
```

### Component Changes

| Component | File | Change | Complexity |
|-----------|------|--------|------------|
| snow-typeck | `builtins.rs` | Add ~15 math function type signatures | Low |
| snow-codegen | `intrinsics.rs` | Declare ~15 LLVM function signatures | Low |
| snow-codegen | `mir/lower.rs` | Map `Math.func` module calls to `math_func` (same as existing module pattern) | Low |
| snow-rt | `math.rs` (NEW) | Wrapper functions calling libm via Rust's `f64` methods | Low |
| snow-rt | `Cargo.toml` | No new deps -- Rust's std `f64` methods link to platform libm | None |
| snow-rt | `lib.rs` | Add `pub mod math;` | Trivial |

### Data Flow

```
Snow source: Math.sin(3.14)
-> Parser: CALL_EXPR with PATH "Math.sin" and ARG_LIST
-> Typeck: Resolves "math_sin" from builtins, unifies arg with Float, result with Float
-> MIR lowering: MirExpr::Call { func: Var("math_sin"), args: [...], ty: Float }
-> Codegen: Calls snow_math_sin(f64) -> f64 (direct intrinsic call, no conversion needed)
-> Runtime: snow_math_sin wraps f64::sin()
```

### Function List

Monomorphic, all `Float -> Float` unless noted:

| Function | Signature | Rust impl |
|----------|-----------|-----------|
| `math_sin` | `(Float) -> Float` | `f64::sin()` |
| `math_cos` | `(Float) -> Float` | `f64::cos()` |
| `math_tan` | `(Float) -> Float` | `f64::tan()` |
| `math_asin` | `(Float) -> Float` | `f64::asin()` |
| `math_acos` | `(Float) -> Float` | `f64::acos()` |
| `math_atan` | `(Float) -> Float` | `f64::atan()` |
| `math_atan2` | `(Float, Float) -> Float` | `f64::atan2()` |
| `math_sqrt` | `(Float) -> Float` | `f64::sqrt()` |
| `math_pow` | `(Float, Float) -> Float` | `f64::powf()` |
| `math_exp` | `(Float) -> Float` | `f64::exp()` |
| `math_log` | `(Float) -> Float` | `f64::ln()` |
| `math_log2` | `(Float) -> Float` | `f64::log2()` |
| `math_log10` | `(Float) -> Float` | `f64::log10()` |
| `math_floor` | `(Float) -> Float` | `f64::floor()` |
| `math_ceil` | `(Float) -> Float` | `f64::ceil()` |
| `math_round` | `(Float) -> Float` | `f64::round()` |
| `math_abs` | `(Float) -> Float` | `f64::abs()` |
| `math_min` | `(Float, Float) -> Float` | `f64::min()` |
| `math_max` | `(Float, Float) -> Float` | `f64::max()` |

Plus constants registered as `Scheme::mono(Ty::float())` in the env:

| Constant | Value |
|----------|-------|
| `math_pi` | `std::f64::consts::PI` |
| `math_e` | `std::f64::consts::E` |
| `math_inf` | `f64::INFINITY` |

**Note on constants:** These need special handling. They cannot be runtime function calls. Two options:
- **Option A (recommended):** Register as known constants in the MIR lowerer, emitting `MirExpr::FloatLit(PI, MirType::Float)` directly when `Math.pi` is referenced. This avoids a runtime call entirely.
- **Option B:** Runtime functions `snow_math_pi() -> f64` etc. Works but wasteful for constants.

### Recommended approach for constants

Handle in `mir/lower.rs` where module-qualified names are resolved. When lowering a `FieldAccess` or `NameRef` for `Math.pi`, `Math.e`, `Math.inf`, emit a float literal directly. This is consistent with how a constant-folding pass would work.

---

## Feature 2: ? Operator (Try/Early Return)

### Integration Points

This is the most cross-cutting feature. It touches the parser, typechecker, MIR, and codegen.

### Architecture

The `?` operator on `expr?` should:
1. Evaluate `expr` (which must be `Result<T, E>`)
2. If `Ok(val)`, unwrap to `val` (the expression result is `T`)
3. If `Err(e)`, early-return `Err(e)` from the enclosing function

This is semantically equivalent to:
```
case expr do
  Ok(val) -> val
  Err(e) -> return Err(e)
end
```

### Parser Changes

The `?` token (`SyntaxKind::QUESTION`) already exists in the lexer and parser. It is currently used only in type annotations (`Int?` -> `Option<Int>`). For expression context, it needs to be a postfix operator.

**Add to the postfix loop in `expr_bp` in `snow-parser/src/parser/expressions.rs`:**

```
// Postfix: try operator (?)
if current == SyntaxKind::QUESTION && POSTFIX_BP >= min_bp {
    let m = p.open_before(lhs);
    p.advance(); // ?
    lhs = p.close(m, SyntaxKind::TRY_EXPR);
    continue;
}
```

**New SyntaxKind variant needed:** `TRY_EXPR` in `syntax_kind.rs`.

**New AST node needed:** `TryExpr` in `snow-parser/src/ast/expr.rs`.

**Ambiguity with type annotations:** The `?` is already used as a type postfix (`Int?` = `Option<Int>`). There is no ambiguity because type annotations are parsed in a separate grammar rule (`items.rs` type parsing), not in expression context. The `?` in expression context is always the try operator.

### Typechecker Changes

In `snow-typeck/src/infer.rs`, when encountering a `TRY_EXPR` node:

1. Infer the inner expression type as `Result<T, E>`.
2. The result type of `expr?` is `T`.
3. The enclosing function's return type must unify with `Result<_, E>` (the error type must match).
4. If the inner expression is not a `Result`, emit a type error.

**Key constraint:** The `?` operator requires that the enclosing function returns `Result<_, E>` where `E` matches the error type of the expression. The typechecker needs to track the current function's return type (which it already does via `result_type` in `infer_with_imports`).

### MIR Representation

**Recommended: Desugaring in MIR lowering.** Lower `TryExpr` to:

```
MirExpr::Match {
    scrutinee: <inner_expr>,
    arms: [
        MirMatchArm {
            pattern: Constructor("Result_T_E", "Ok", [Var("__try_val", T)]),
            body: Var("__try_val", T),
        },
        MirMatchArm {
            pattern: Constructor("Result_T_E", "Err", [Var("__try_err", E)]),
            body: Return(ConstructVariant("Result_T_E", "Err", [Var("__try_err", E)])),
        },
    ],
    ty: T,
}
```

This reuses the existing pattern match and return infrastructure. **No new MirExpr variant needed.** No changes to codegen -- it uses existing `Match`, `Constructor`, `Return`, and `ConstructVariant` codegen. The pattern compile module (`pattern/compile.rs`) already handles `Constructor` patterns for sum types including `Result`.

### Component Changes

| Component | File | Change | Complexity |
|-----------|------|--------|------------|
| snow-parser | `syntax_kind.rs` | Add `TRY_EXPR` variant | Trivial |
| snow-parser | `parser/expressions.rs` | Add `?` postfix in `expr_bp` loop | Low |
| snow-parser | `ast/expr.rs` | Add `TryExpr` AST node | Low |
| snow-typeck | `infer.rs` | Handle `TRY_EXPR` node: check Result type, unify return type | Medium |
| snow-codegen | `mir/lower.rs` | Desugar `TryExpr` to Match+Return | Medium |
| snow-codegen | codegen/* | **None** -- reuses existing infrastructure | None |

### Data Flow

```
Snow source: let val = risky_fn()?
-> Parser: TRY_EXPR wrapping CALL_EXPR
-> Typeck: Inner type = Result<Int, String>, expression type = Int, function return = Result<_, String>
-> MIR: Match { scrutinee: Call("risky_fn"), arms: [Ok->unwrap, Err->Return(Err(e))] }
-> Codegen: Decision tree for Ok/Err tag check, branch to unwrap or return
-> LLVM: icmp tag==0 (Ok), br to ok_bb or err_bb, ok_bb extracts field, err_bb returns
```

---

## Feature 3: Receive Timeout Codegen

### Current State and Gap Analysis

The MIR has `ActorReceive { timeout_ms, timeout_body, arms, ty }` and MIR lowering (lower.rs `lower_receive_expr`) correctly populates `timeout_ms` and `timeout_body` from the `AFTER_CLAUSE` in the parser.

The runtime `snow_actor_receive(timeout_ms: i64) -> *const u8` (actor/mod.rs line 315) correctly returns `null` when the timeout expires (line 394-398 in coroutine path, line 358 in main thread path).

**The gap is in `codegen_actor_receive` (expr.rs line 1540).** Currently it:
1. Evaluates timeout value (or defaults to -1 for infinite)
2. Calls `snow_actor_receive(timeout_ms)` -- returns `ptr`
3. Performs GEP to skip 16-byte message header
4. Loads message data from data_ptr
5. Executes the first match arm body

**What is missing:** No null check on the returned `msg_ptr`. When timeout expires, `snow_actor_receive` returns null, but codegen proceeds to GEP and load, causing a segfault.

Additionally, the `timeout_body` parameter is received by `codegen_actor_receive` (as `timeout_body: _`, explicitly ignored at line 129) but never used.

### Architecture

The fix requires adding a null-check branch between the `snow_actor_receive` call and the message loading:

```
msg_ptr = call snow_actor_receive(timeout_ms)
is_null = icmp eq msg_ptr, null
br is_null, timeout_bb, message_bb

timeout_bb:
  <codegen timeout_body>   // or unit if no timeout body
  br merge_bb

message_bb:
  <existing message loading + arm dispatch>
  br merge_bb

merge_bb:
  phi [timeout_result, message_result]
```

### Component Changes

| Component | File | Change | Complexity |
|-----------|------|--------|------------|
| snow-codegen | `codegen/expr.rs` | Modify `codegen_actor_receive` to add null check + timeout branch | Medium |

**No other components need changes.** The parser already parses `after`, the typechecker already type-checks it, and the MIR lowerer already produces the timeout fields. This is purely a codegen gap.

### Detailed Codegen Change

In `codegen_actor_receive` (line 1540), after the call to `snow_actor_receive`:

1. Build `icmp eq msg_ptr, null` to check for timeout.
2. Create three basic blocks: `timeout_bb`, `message_bb`, `merge_bb`.
3. `br is_null, timeout_bb, message_bb`.
4. In `timeout_bb`: codegen the `timeout_body` expression from the MIR, then `br merge_bb`.
5. In `message_bb`: the existing message load + arm dispatch logic (lines 1568-1647), then `br merge_bb`.
6. In `merge_bb`: phi node merging the two results using an alloca+store+load pattern (consistent with `codegen_if`).

**When `timeout_ms` is `None` (infinite wait):** Skip the null check. `snow_actor_receive(-1)` never returns null. The existing code path is correct.

**When `timeout_ms` is `Some` but `timeout_body` is `None`:** This should not happen in practice. The MIR lowerer pairs them. If it does, produce unit value in timeout_bb.

---

## Feature 4: Timer Primitives

### Integration Points

Timer primitives (`send_after`, `send_interval`, `cancel_timer`) are runtime-level features that schedule delayed message sends.

### Architecture

```
Snow source: let timer_ref = Timer.send_after(pid, msg, 1000)
-> builtins.rs: Register timer_send_after :: (Pid, T, Int) -> Int (timer ref)
-> intrinsics.rs: Declare snow_timer_send_after(pid: i64, msg_ptr: ptr, msg_size: i64, delay_ms: i64) -> i64
-> snow-rt/timer.rs: Dedicated timer thread with priority queue
```

### Runtime Design

**Recommended: Dedicated timer thread.** A single background thread manages a priority queue (BinaryHeap ordered by deadline). `send_after` inserts into the queue, the thread sleeps until the next deadline, then calls `snow_actor_send`.

**Alternative considered but rejected: Scheduler integration.** Adding timer checks to each worker thread's inner loop would add latency to every actor yield/resume on the scheduler's hot path. The scheduler already handles `!Send` coroutines and work-stealing; adding timer checks complicates the critical section.

### Runtime Implementation Sketch

```rust
// snow-rt/src/timer.rs

use std::collections::BinaryHeap;
use std::sync::{Arc, Mutex, Condvar};
use std::cmp::Reverse;
use std::time::{Instant, Duration};

struct TimerEntry {
    deadline: Instant,
    target_pid: u64,
    msg_data: Vec<u8>,         // Deep-copied message bytes
    interval_ms: Option<u64>,  // Some = repeating, None = one-shot
    timer_id: u64,
    cancelled: bool,
}

// Global timer state protected by mutex + condvar
// Timer thread: sleep until next deadline via condvar.wait_timeout, fire expired timers
```

### Interaction with Scheduler

The timer thread calls `snow_actor_send` to deliver the delayed message. `snow_actor_send` pushes to the target actor's mailbox and sets the actor to `Ready` if it was `Waiting`. This is already thread-safe because `snow_actor_send` acquires the process table lock internally.

**No scheduler changes needed.** The timer fires a send, which wakes the waiting actor through the existing send-wake path.

### Component Changes

| Component | File | Change | Complexity |
|-----------|------|--------|------------|
| snow-typeck | `builtins.rs` | Register `timer_send_after`, `timer_send_interval`, `timer_cancel` | Low |
| snow-codegen | `intrinsics.rs` | Declare 3 timer intrinsics | Low |
| snow-codegen | `mir/lower.rs` | Map `Timer.send_after` etc. to `timer_send_after` | Low |
| snow-rt | `timer.rs` (NEW) | Timer thread + priority queue + send integration | Medium |
| snow-rt | `lib.rs` | Add `pub mod timer;` and init timer thread in `snow_rt_init` | Low |

### Type Signatures

| Function | Snow Type | C ABI |
|----------|-----------|-------|
| `timer_send_after` | `(Pid, T, Int) -> Int` | `snow_timer_send_after(pid: i64, msg_ptr: ptr, msg_size: i64, delay_ms: i64) -> i64` |
| `timer_send_interval` | `(Pid, T, Int) -> Int` | `snow_timer_send_interval(pid: i64, msg_ptr: ptr, msg_size: i64, interval_ms: i64) -> i64` |
| `timer_cancel` | `(Int) -> Bool` | `snow_timer_cancel(timer_ref: i64) -> i8` |

**Critical note on message serialization:** `send_after` must deep-copy the message data at call time, not at fire time. The message bytes might reference stack-allocated data in the calling actor that gets freed before the timer fires. The runtime implementation must `memcpy` the msg_ptr data into a heap-allocated `Vec<u8>`.

---

## Feature 5: Collection Sort

### Integration Points

Adding `List.sort(list)` and `List.sort_by(list, compare_fn)` requires a sort implementation in the runtime that accepts comparison callbacks.

### Sort Algorithm: Merge Sort

Reasons for merge sort over quicksort:

1. **Immutable semantics:** Snow lists are immutable. Sort must return a NEW list. Merge sort naturally produces new arrays during merge steps. Quicksort requires in-place mutation (partitioning), which conflicts with immutability unless you copy first then sort in-place -- but that is merge sort with extra copying.

2. **Stability:** Merge sort is stable. For a functional language, users expect `sort` to preserve relative order of equal elements.

3. **Predictable performance:** O(n log n) worst case, unlike quicksort's O(n^2) worst case.

4. **Existing callback pattern:** The runtime already passes `fn_ptr: *mut u8` for comparisons (`snow_list_compare` at intrinsics.rs line 400). Merge sort's comparison-driven nature maps directly to this callback pattern.

### Callback Design

Two API shapes:

```
// Ord-based: uses the Ord trait's compare method, dispatched by element type
List.sort(list)  ->  snow_list_sort(list: ptr, cmp_fn: ptr) -> ptr

// Custom comparator: user provides comparison closure
List.sort_by(list, fn)  ->  snow_list_sort_by(list: ptr, cmp_fn: ptr, cmp_env: ptr) -> ptr
```

For `List.sort(list)`, the MIR lowerer resolves the element type and passes the appropriate `Ord__compare__TypeName` function pointer. This is the same pattern used for `snow_list_compare` and `snow_list_eq` (see `wrap_collection_compare` pattern in the codebase).

For `List.sort_by(list, fn)`, the user passes a closure `(T, T) -> Ordering`, and the runtime dispatches via the bare fn / closure fn pattern (null-check on env_ptr).

### Component Changes

| Component | File | Change | Complexity |
|-----------|------|--------|------------|
| snow-typeck | `builtins.rs` | Register `list_sort` and `list_sort_by` type signatures | Low |
| snow-codegen | `intrinsics.rs` | Declare `snow_list_sort` and `snow_list_sort_by` | Low |
| snow-codegen | `mir/lower.rs` | Resolve Ord callback for element type (reuse existing trait dispatch pattern) | Medium |
| snow-rt | `collections/list.rs` | Implement merge sort with comparison callback | Medium |

### Type Signatures

| Function | Snow Type | Notes |
|----------|-----------|-------|
| `list_sort` | `(List<T>) -> List<T>` | Requires `T: Ord` |
| `list_sort_by` | `(List<T>, (T, T) -> Ordering) -> List<T>` | Custom comparator |

### Runtime Implementation Pattern

The implementation in `collections/list.rs` follows the same layout conventions: read `len` and `data` from the list header, allocate a new list with `alloc_list(len)`, perform merge sort using a temporary buffer (also GC-allocated), and return the sorted list.

```rust
#[no_mangle]
pub extern "C" fn snow_list_sort(list: *mut u8, cmp_fn: *mut u8) -> *mut u8 {
    type CmpFn = unsafe extern "C" fn(u64, u64) -> i64;
    unsafe {
        let len = list_len(list) as usize;
        if len <= 1 { return /* copy or return as-is */ }
        let f: CmpFn = std::mem::transmute(cmp_fn);
        // Bottom-up merge sort into new buffer
        // Return new list with sorted data
    }
}
```

---

## Feature 6: Tail Call Elimination (TCE)

### Integration Points

This is the most architecturally complex feature. It interacts with LLVM's tail call semantics and corosensei's stack model.

### Available LLVM Support

The project uses Inkwell 0.8.0 with feature `llvm21-1`. The `CallSiteValue::set_tail_call_kind(LLVMTailCallKind)` API is available, supporting `Tail`, `MustTail`, and `NoTail` kinds.

### Approach Analysis

**Option A: LLVM `musttail` annotation.**

Mark self-recursive tail calls with `musttail` and let LLVM handle the transformation.

Requirements for `musttail`:
- Caller and callee must have identical signatures (same number/type of params, same return type, same calling convention).
- The call must be immediately followed by a `ret` instruction.
- No alloca'd variables can be live at the call site.

Problems:
- Snow functions may have captured variables via closures, causing signature mismatches.
- `snow_reduction_check()` calls (yield points inserted before function calls by codegen) would break the tail position requirement. The reduction check must be moved BEFORE the tail call setup, not between the call and the return.
- `musttail` is fragile: if any constraint is violated, LLVM silently falls back to a normal call (no stack reuse), making it an unreliable guarantee.

**Option B: MIR-level loop transformation (recommended).**

Transform self-recursive tail calls into loops at the MIR level. This produces a `While` loop that existing codegen already handles.

Advantages:
1. No ABI constraints -- works with any function signature.
2. No interaction with corosensei yield points.
3. No dependency on LLVM optimization levels.
4. `snow_reduction_check()` can be placed in the loop body naturally.
5. Deterministic -- transformation either succeeds or reports that the call is not in tail position.

Disadvantages:
1. Only handles self-recursion (not mutual recursion -- future work).
2. Requires detecting tail position in MIR.

### Recommended Architecture: MIR Loop Transformation

#### New Pass: `mir/tce.rs`

A post-lowering MIR transformation pass that runs after `lower_to_mir_module` and before codegen.

#### Detection Phase

Scan each `MirFunction` for self-recursive tail calls:

1. Walk the function body to find `MirExpr::Call` where `func` is `Var(fn_name)` and `fn_name == current_function.name`.
2. Verify the call is in tail position using these rules:

A `MirExpr::Call` is in tail position if:
- It is the last expression in the function body.
- It is the last expression in `then_body` or `else_body` of an `If` that is itself in tail position.
- It is the last expression in a `Match` arm body where the `Match` is in tail position.
- It is the last expression in a `Block` that is in tail position.
- It is the body of a `Let` binding where the `Let` is in tail position.
- **NOT** in tail position if the call result is used (bound by Let with non-tail body, passed to BinOp, etc.).

#### Transformation Phase

For each function with detected self-recursive tail calls:

1. Replace the function body with a `While { cond: BoolLit(true), body: transformed_body }`.
2. Introduce mutable loop variables for each function parameter.
3. Replace tail-recursive calls with: assign new arg values to the loop variables, then `Continue`.
4. Wrap non-recursive return paths with explicit `Return`.

#### Mutable Loop Variables

MIR uses immutable `Let` bindings. The loop transformation needs mutable variables. Two new MirExpr variants:

```rust
/// Assign to a mutable variable (for TCE loop variables only).
MutAssign {
    name: String,
    value: Box<MirExpr>,
    ty: MirType,
},

/// Declare a mutable variable with initial value, body uses it.
MutLet {
    name: String,
    ty: MirType,
    value: Box<MirExpr>,
    body: Box<MirExpr>,
},
```

Codegen translates these to alloca + store/load. LLVM's mem2reg pass optimizes them into SSA registers.

#### Example Transformation

```
// Before TCE pass:
fn factorial(n: Int, acc: Int) -> Int =
  If { cond: n <= 0, then: acc, else: Call("factorial", [n-1, n*acc]) }

// After TCE pass:
fn factorial(n: Int, acc: Int) -> Int =
  MutLet { name: "__tce_n", value: Var("n"),
    MutLet { name: "__tce_acc", value: Var("acc"),
      While { cond: true,
        If { cond: Var("__tce_n") <= 0,
          then: Return(Var("__tce_acc")),
          else: Block [
            MutAssign { "__tce_acc", Var("__tce_n") * Var("__tce_acc") },
            MutAssign { "__tce_n", Var("__tce_n") - 1 },
            Continue,
          ]
        }
      }
    }
  }
```

#### Interaction with Corosensei

**No interaction.** The MIR loop transformation produces a `While` loop, which codegen already handles. Reduction checks are inserted at loop back-edges by existing codegen infrastructure. The corosensei coroutine can yield at any reduction check point without interfering with the loop.

### Component Changes

| Component | File | Change | Complexity |
|-----------|------|--------|------------|
| snow-codegen | `mir/tce.rs` (NEW) | Tail call detection + loop transformation pass | High |
| snow-codegen | `mir/mod.rs` | Add `MutAssign` and `MutLet` MirExpr variants, call TCE pass | Medium |
| snow-codegen | `codegen/expr.rs` | Handle `MutAssign` and `MutLet` codegen (alloca + store) | Medium |
| snow-codegen | `lib.rs` | Insert TCE pass into compilation pipeline | Low |
| snow-parser | -- | No changes | None |
| snow-typeck | -- | No changes | None |
| snow-rt | -- | No changes | None |

---

## Suggested Build Order

Based on dependency analysis between features:

### Phase 1: Math Stdlib
**Rationale:** Zero dependencies on other features. Follows the exact same pattern as existing string/file/JSON stdlib. Lowest risk, highest confidence. Provides immediate user value.

### Phase 2: Receive Timeout Codegen
**Rationale:** Fills a gap in existing infrastructure. The MIR and runtime already support it -- only codegen needs a null-check branch. Small, well-scoped change. Enables Phase 4 (timer primitives depend on receive timeouts working).

### Phase 3: ? Operator
**Rationale:** Parser + typechecker + MIR lowering changes, but desugars to existing Match + Return codegen. No runtime changes. Medium complexity but well-understood semantics.

### Phase 4: Timer Primitives
**Rationale:** Depends on Phase 2 (receive timeouts) because timers are typically used with `receive ... after ... end`. The runtime timer thread is self-contained but needs the receive timeout path to be solid.

### Phase 5: Collection Sort
**Rationale:** Depends on the existing Ord trait infrastructure. Follows the callback pattern of list_eq/list_compare. Medium complexity in both runtime (merge sort) and MIR lowering (Ord callback resolution).

### Phase 6: Tail Call Elimination
**Rationale:** Most complex feature. Independent of other features but highest risk. The MIR transformation pass is a new compiler pass pattern (first post-lowering pass). Should be built last to avoid blocking other features if it encounters difficulties.

```
Phase 1: Math Stdlib ------> (standalone, immediate value)
Phase 2: Receive Timeout --> Phase 4: Timer Primitives
Phase 3: ? Operator -------> (standalone, ergonomic value)
Phase 5: Collection Sort --> (standalone, uses existing patterns)
Phase 6: TCE -------------> (standalone but highest complexity)
```

---

## Anti-Patterns to Avoid

### Anti-Pattern 1: New MirExpr Variants for Simple Features
**What:** Adding new MirExpr variants when existing ones suffice (e.g., adding `MirExpr::TryOp` for the `?` operator).
**Why bad:** Every new MirExpr variant requires handling in `codegen_expr`, `ty()`, `collect_free_vars`, and every MirExpr match (30+ existing variants). This is O(variants) maintenance cost.
**Instead:** Desugar to existing MirExpr variants in MIR lowering (as recommended for `?` operator).

### Anti-Pattern 2: Blocking Timer Operations in Scheduler Hot Path
**What:** Adding timer checks inside the scheduler's worker loop.
**Why bad:** The scheduler loop runs for every actor yield/resume. Timer checks add latency to the critical path. The scheduler already handles `!Send` coroutines and work-stealing -- adding more logic to the inner loop increases the chance of correctness bugs.
**Instead:** Use a dedicated timer thread that calls `snow_actor_send` externally.

### Anti-Pattern 3: In-Place Mutation in Sort Runtime
**What:** Sorting a list's buffer in-place.
**Why bad:** Snow lists have immutable semantics. All mutation operations (append, tail, concat, reverse) return NEW lists. Other references to the list would see the mutation, violating the language's guarantees.
**Instead:** Always copy the data buffer before sorting. Return a new list pointer.

### Anti-Pattern 4: Using `musttail` for TCE
**What:** Relying on LLVM `musttail` for guaranteed tail calls.
**Why bad:** `musttail` has strict ABI requirements (identical caller/callee signatures, no live allocas, must precede `ret`). Snow's reduction checks (`snow_reduction_check()`) are inserted before calls and would need to be moved. Closure captures change effective signatures. If any constraint is violated, LLVM silently degrades to a normal call, making stack overflow possible with no warning.
**Instead:** MIR-level loop transformation, which is robust regardless of LLVM version or optimization level.

---

## Scalability Considerations

| Concern | Current (v1.8) | After v1.9 |
|---------|----------------|------------|
| Builtin count in builtins.rs | ~100 entries | ~120 entries (math adds ~20) |
| Intrinsic count in intrinsics.rs | ~80 declarations | ~85 declarations |
| MirExpr variants | 30 variants | 32 variants (MutAssign + MutLet for TCE) |
| Compiler passes | 1 (MIR lowering) | 2 (MIR lowering + TCE pass) |
| Runtime threads | N workers + 1 main | N workers + 1 main + 1 timer |
| snow-rt modules | 12 source files | 14 source files (math.rs + timer.rs) |

The builtin/intrinsic registration pattern scales linearly and is maintainable. The TCE pass introduces a second MIR pass, which is architecturally significant -- it establishes the pattern for future optimization passes (constant folding, dead code elimination, inlining, etc.).

---

## Sources

- Direct codebase inspection of all files referenced in the Component Changes tables above
- [Inkwell CallSiteValue API](https://thedan64.github.io/inkwell/inkwell/values/struct.CallSiteValue.html) -- confirms `set_tail_call_kind` available with `llvm21-1` feature
- [Corosensei GitHub](https://github.com/Amanieu/corosensei) -- stackful coroutine design, `!Send` coroutines, stack switching semantics
- [LLVM musttail semantics (D99517)](https://reviews.llvm.org/D99517) -- ABI requirements for musttail guarantees
- [Rust become keyword](https://doc.rust-lang.org/std/keyword.become.html) -- nightly-only explicit tail calls, incomplete as of 2026
