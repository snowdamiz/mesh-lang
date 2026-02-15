---
status: resolved
trigger: "LLVM codegen errors - service state type mismatches, HTTP route arg counts, function return type mismatches"
created: 2026-02-14T00:00:00Z
updated: 2026-02-15T01:36:00Z
---

## Current Focus

hypothesis: CONFIRMED - All root causes identified and fixed
test: cargo run --release -- build mesher/
expecting: Compiled: mesher/mesher
next_action: Archive session

## Symptoms

expected: The mesher project should compile to a working binary through all stages including LLVM codegen
actual: LLVM module verification fails with multiple errors at the codegen stage
errors: |
  1. Call parameter type mismatches - service state loaded as i64 but handlers expect full struct types
  2. Incorrect argument counts on HTTP route registration (4 args vs expected different count)
  3. Function return type mismatches (ptr vs {i8, ptr} Result type, struct vs ptr)
  4. Other type mismatches (mesh_map_put, mesh_ws_send, mesh_string_concat, etc.)
reproduction: cargo run --release -- build mesher/
started: Predates Phase 90, in codegen layer

## Eliminated

## Evidence

- timestamp: 2026-02-14T00:00:30Z
  checked: codegen_service_loop in expr.rs lines 2920-2941
  found: State type determination only checks is_pointer_type() and defaults to i64. Never considers struct types like RateLimitState, StreamState, WriterState, etc.
  implication: ROOT CAUSE #1 - state is always loaded as i64 or ptr, never the actual LLVM struct type

- timestamp: 2026-02-14T00:00:35Z
  checked: codegen_call in expr.rs lines 643-649
  found: For non-user-fn calls, FnPtr args get a null env_ptr appended automatically. mesh_http_route_post/get declared with 3 params but receives 4 (router, path, handler_fn, null_env)
  implication: ROOT CAUSE #2 - FnPtr-to-closure expansion happens for ALL runtime intrinsics, even those that take a plain function pointer

- timestamp: 2026-02-14T00:00:40Z
  checked: MIR lowering of service handlers (lower.rs 8920-9030)
  found: Handler functions have actual state struct types as first param (init_ret_ty). Call handlers return MirType::Ptr (tuple pointer), cast handlers return the state struct type. The dispatch loop ignores these types.
  implication: Confirms #1 - the MIR correctly specifies state types, codegen ignores them

- timestamp: 2026-02-14T00:00:45Z
  checked: Service call/cast helper MIR lowering (lower.rs 9166-9218)
  found: __service_{name}_call_{method} has return_type MirType::Int, and ALL params are MirType::Int. But the handler args may actually be ptr/String types in the Mesh source.
  implication: ROOT CAUSE #3 - call helper params hardcoded to MirType::Int loses type info for ptr/String args

- timestamp: 2026-02-14T00:00:50Z
  checked: LLVM error output for return type mismatches
  found: "ret ptr %call { i8, ptr }" means functions returning ptr when callee expects Result type {i8, ptr}
  implication: ROOT CAUSE #4 - Return type coercion missing in compile_function

- timestamp: 2026-02-15T00:20:00Z
  checked: codegen_string_concat (BinOp path)
  found: <> operator goes through codegen_string_concat which directly calls mesh_string_concat without any type coercion. Unit-typed variables ({} empty struct) passed through unchanged.
  implication: ROOT CAUSE #5 - codegen_string_concat has no argument coercion, unlike codegen_call path

- timestamp: 2026-02-15T01:36:00Z
  checked: Full compiler run
  found: Compiled: mesher/mesher - valid Mach-O 64-bit ARM64 executable, ~19MB
  implication: All LLVM verification errors resolved

## Resolution

root_cause: |
  Six interacting issues in LLVM codegen and MIR lowering:

  1. Service dispatch loop state type: codegen_service_loop determined state type as either ptr or i64 using a binary check. Handler functions use actual struct types (RateLimitState, StreamState, etc.) which are neither ptr nor i64.

  2. FnPtr closure expansion: codegen_call unconditionally expanded FnPtr args to (fn_ptr, null_env) pairs for ALL non-user functions. HTTP route functions only expect a plain fn_ptr, causing arg count mismatches.

  3. Service helper param types: MIR lowering hardcoded all call/cast helper params as MirType::Int. Callers pass actual typed values (String ptrs, etc.), causing LLVM param type mismatches.

  4. Missing return value coercion: compile_function's return instruction used the body result directly without coercing to match the function's declared return type. Caused ptr vs {i8, ptr} and struct vs ptr mismatches.

  5. Missing arg coercion in codegen_string_concat: The <> operator path directly passes args to mesh_string_concat without coercion. Unit-typed ({}) variables were passed unchanged.

  6. Missing coercion cases in runtime intrinsic calls: Several type coercion directions were missing: i64->ptr (inttoptr), struct->i64 (heap-alloc+ptrtoint), empty struct (Unit)->null.

fix: |
  Applied 7 fixes across 2 files:

  A. expr.rs - codegen_service_loop: Use actual handler first parameter type instead of ptr/i64 binary. TryFrom<BasicMetadataTypeEnum> to get BasicTypeEnum. Also handle struct state in call handler result extraction (inttoptr + load).

  B. expr.rs - codegen_call FnPtr expansion: Check target function's param count before expanding FnPtr to (fn_ptr, null_env) pair. Only expand when the expanded count matches expected params.

  C. lower.rs - Service call/cast helpers: Store param_types in CallInfo/CastInfo structs. Use actual resolved types from type checker instead of hardcoded MirType::Int. Update known_functions entries to use actual param types.

  D. expr.rs - codegen_service_call_helper & codegen_service_cast_helper: Use new coerce_to_i64() method instead of into_int_value() to safely convert any value type to i64 for message buffers.

  E. mod.rs - compile_function return coercion: Added coerce_return_value() method that handles ptr->struct (load), struct->ptr (heap-alloc), int->ptr (inttoptr), ptr->int (ptrtoint), struct->struct (bitcast via alloca).

  F. expr.rs - Runtime intrinsic arg coercion: Added missing coercion cases: IntValue->PointerType (inttoptr), StructValue->IntType (heap-alloc+ptrtoint), empty StructValue->null/zero. Added Unit {} special case for user function calls too.

  G. expr.rs - codegen_string_concat: Added coercion for Unit and Int args to ptr before calling mesh_string_concat.

verification: |
  cargo run --release -- build mesher/
  Result: "Compiled: mesher/mesher" - valid Mach-O 64-bit ARM64 executable
  All previous LLVM module verification errors eliminated.
  No new errors introduced.

files_changed:
  - crates/mesh-codegen/src/codegen/expr.rs
  - crates/mesh-codegen/src/codegen/mod.rs
  - crates/mesh-codegen/src/mir/lower.rs
