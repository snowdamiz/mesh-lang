# Service Call Reply Type Segfault (SIGSEGV / exit 139)

## Summary

When Mesher receives an authenticated HTTP request, service calls (e.g. `RateLimiter.check_limit`, `EventProcessor.process_event`) crash with SIGSEGV (exit code 139). The root cause is that service call helper functions always used `MirType::Int` as their return type, regardless of the actual reply type. This causes incorrect LLVM IR for non-Int reply types (Bool, SumType, String, Struct, etc.).

## Root Cause

In `crates/mesh-codegen/src/mir/lower.rs`, the service call lowering hardcoded `MirType::Int` in three places:

1. **Known function registration** (~line 9797): `Box::new(MirType::Int)` — the FnPtr return type
2. **Call expression type** (~line 9854): `ty: MirType::Int` — the MIR Call node's type
3. **MirFunction return_type** (~line 9860): `return_type: MirType::Int`

This meant `codegen_service_call_helper` in `expr.rs` always received `reply_ty = MirType::Int`, causing it to return the raw i64 reply value without any conversion. For types that need conversion (SumType via inttoptr+load, Bool via truncate, String via inttoptr, etc.), this produces type mismatches or garbage values at the LLVM level.

### Concrete example: EventProcessor

`EventProcessor.process_event` returns `String!String` (aka `Result<String, String>`), which is `MirType::SumType("Result_String_String")` — a 16-byte `{i8, ptr}` struct. The handler encodes this as a heap-allocated pointer stored as i64 in the reply message. But because the helper function returned raw i64 (MirType::Int), the caller tried to use that i64 as a 16-byte struct value, reading garbage memory → segfault.

### Concrete example: RateLimiter

`RateLimiter.check_limit` returns `Bool`. With `MirType::Int`, the function returns i64 in LLVM. The caller expects i64 and uses it in a conditional. Since Bool true = 1 and Int 1 is truthy, this *appeared* to work before, but once the return type was corrected to `MirType::Bool` (LLVM `i1`), the full conversion path is needed.

## What Was Tried

### Attempt 1: Use `resolve_range(block.syntax().text_range())`

**Approach**: Get the handler body Block's syntax range, look up its type in the type checker's `HashMap<TextRange, Ty>`, extract the second tuple element as the reply type.

```rust
let reply_type = handler.body()
    .map(|block| self.resolve_range(block.syntax().text_range()))
    .and_then(|ty| { /* extract Tuple element [1] */ })
    .unwrap_or(MirType::Int);
```

**Result**: Did not work. Always fell back to `MirType::Int`.

**Why it failed**: The type checker (`mesh-typeck/src/infer.rs`) does NOT store a type entry for the BLOCK node itself. It stores types for individual expressions within the block, but not the block wrapper node. So `self.types.get(&block_range)` returns `None`, and `resolve_range` returns `MirType::Unit`, which doesn't match `MirType::Tuple(...)`, causing fallback to `MirType::Int`.

### Attempt 2: Use `block.tail_expr().syntax().text_range()`

**Approach**: Instead of the block's range, use the tail expression (last expression in the block). The type checker DOES record types for expressions. The tail expression of a handler body like `check_limit_impl(state, project_id)` has type `Tuple(RateLimitState, Bool)`.

```rust
let reply_type = handler.body()
    .and_then(|block| block.tail_expr())
    .map(|expr| self.resolve_range(expr.syntax().text_range()))
    .and_then(|ty| { /* extract Tuple element [1] */ })
    .unwrap_or(MirType::Int);
```

**Result**: Type resolution now works correctly (confirmed via debug eprintln). All handlers get correct reply types:
- `CheckLimit` → `Bool`
- `ProcessEvent` → `SumType("Result_String_String")`
- `IsStreamClient` → `Bool`
- `GetProjectId` → `String`
- `GetPool` → `Int`
- `GetRateLimiter` → `Pid(None)`
- Various user/org/project services → `SumType("Result_*_String")`

**However**: Mesher still crashes with exit 139 on the first authenticated request. Also returns HTTP 429 (rate limited) on the very first request, which should be allowed (limit is 1000/60s).

### Codegen changes (expr.rs)

Added type-aware reply conversion in `codegen_service_call_helper` (~line 4049):

- **SumType**: Check struct size. ≤8 bytes: alloca i64, store, load as struct type. >8 bytes: inttoptr, load struct from heap pointer.
- **Struct**: Same size-based branching as SumType.
- **String/Ptr**: inttoptr to get pointer back.
- **Bool**: `build_int_truncate(reply_i64, i1)` to get i1 from i64.
- **Float**: alloca i64, store, load as f64 (bitcast via memory).
- **Default (Int, Pid, Unit)**: Return raw i64 unchanged.

## Current State of Code Changes

### `crates/mesh-codegen/src/mir/lower.rs`

1. Added `reply_type: MirType` field to `CallInfo` struct (~line 9397-9401)
2. Reply type determination using `tail_expr()` approach (~line 9442-9459)
3. Three usage sites changed from `MirType::Int` to `info.reply_type.clone()`:
   - Known function FnPtr return type (~line 9797)
   - Call expression `ty` field (~line 9854)
   - MirFunction `return_type` (~line 9860)

### `crates/mesh-codegen/src/codegen/expr.rs`

1. Full `match reply_ty` block replacing the old scalar-only path (~lines 4049-4137)
2. Handles: SumType, Struct, String, Ptr, Bool, Float, and default (Int/Pid/Unit)

## Test Results

- **179 codegen tests**: All pass
- **509 runtime tests**: All pass
- **91/93 E2E tests**: Pass (2 pre-existing HTTP test failures)
- **4 service tests**: All pass
- **4 supervisor tests**: All pass
- **8 tooling tests**: All pass

## What Still Fails

Live Mesher testing with PostgreSQL:

1. **HTTP 429 on first request**: `RateLimiter.check_limit` returns `Bool`. The rate limiter should allow the first request (0 < 1000), but returns "rate limited". This suggests the Bool value is being inverted or corrupted somewhere in the reply path.

2. **SIGSEGV after first request**: Server crashes with exit 139 after responding to the first request. The crash likely occurs in a subsequent service call (possibly EventProcessor.process_event or one of the PipelineRegistry calls), or during cleanup/GC after the request.

## Key Technical Details

### Service call mechanism

1. Caller invokes `ServiceName.method(pid, args...)` which calls a generated helper function
2. Helper packs args into a message buffer: `[u64 tag][u64 caller_pid][i64 args...]`
3. Calls `mesh_service_call(pid, msg_ptr, msg_size)` — blocks until reply
4. Reply comes back as a message pointer; reply data is at offset +16 bytes
5. Loads an i64 from the reply data area
6. **Must convert that i64 back to the correct type** (this is what was broken)

### Tuple element encoding (how values become i64)

- Small structs (≤8 bytes): bitcast struct to i64
- Large structs (>8 bytes): heap-allocated, pointer stored as i64 via ptrtoint
- Pointers (String, Ptr): ptrtoint to i64
- Scalars (Int, Pid): value IS the i64
- Bool: zero-extended from i1 to i64

### Type checker types map

- `HashMap<TextRange, Ty>` — maps AST node text ranges to inferred types
- Types are resolved through union-find before being returned (`ctx.resolve(ty)`)
- Stores types for: expressions, parameters, let bindings, function definitions
- Does NOT store types for: Block nodes, CallHandler nodes, CastHandler nodes
- The tail expression of a block IS stored (it's an expression)

### MIR type mapping for service replies

| Mesh type | MirType | LLVM type | Conversion needed |
|-----------|---------|-----------|-------------------|
| Int | Int | i64 | None (raw i64) |
| Bool | Bool | i1 | truncate i64→i1 |
| Float | Float | f64 | bitcast via alloca |
| String | String | ptr | inttoptr |
| Result<A,B> | SumType("Result_A_B") | {i8, ptr} | inttoptr + load (16 bytes, heap) |
| Pid | Pid(None) | i64 | None (raw i64) |
| PoolHandle | Int | i64 | None (raw i64) |
| Struct (small) | Struct("Name") | struct type | bitcast via alloca |
| Struct (large) | Struct("Name") | struct type | inttoptr + load |

## Open Questions / Next Steps

1. **Why does Bool reply cause 429?** The rate limiter returns `true` (allowed) but the caller acts as if it's `false`. Possible causes:
   - The function signature change (returning i1 instead of i64) may interact badly with the calling convention or how the runtime passes values
   - The `build_int_truncate` may not preserve the correct bit
   - There may be a mismatch between what the service loop stores in the reply message and what the caller loads

2. **Why does it still segfault?** Even after the Bool issue, something crashes. Possible causes:
   - The service loop itself may have issues with how it encodes the reply tuple — the handler function's return type changed from i64 to the actual type, but the service loop code that extracts the reply from the (state, reply) tuple may not handle non-i64 return types correctly
   - The GC may interact poorly with the new pointer types
   - There may be additional places in the codegen that assume service call helpers return i64

3. **Service loop reply extraction**: The service loop calls the handler function and gets back a value. It then needs to extract the second tuple element (reply) and store it as i64 in the reply message. If the handler function now returns a non-i64 type, the service loop's extraction code may need corresponding changes.

4. **Handler function vs. call helper function**: There are TWO functions per call handler:
   - The **handler function** (runs inside the service actor loop) — takes state + args, returns (new_state, reply)
   - The **call helper function** (called by the client) — sends message, waits for reply, converts i64 back to reply type

   The changes so far affect the **call helper function's return type**. But the **handler function** is also generated with a return type, and the **service loop** extracts the reply from the handler's return value. These may also need attention.

5. **Verify with a minimal test**: Create a simple .mpl file with a service that returns Bool and another that returns a Result type. Test outside of the full Mesher context to isolate the issue.
