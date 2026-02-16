---
status: resolved
trigger: "Mesher binary crashes with SIGBUS (Bus error: 10) when HTTP requests hit dashboard endpoints"
created: 2026-02-15T00:00:00Z
updated: 2026-02-15T20:45:00Z
---

## Current Focus

hypothesis: CONFIRMED - Unresolved type variables in typeck cause Map.get/List.get return values to be discarded
test: Build and run mesher, curl all dashboard endpoints
expecting: All endpoints return 200 with JSON, no crash
next_action: Archive session

## Symptoms

expected: Mesher handles HTTP requests to dashboard endpoints and returns JSON responses
actual: Mesher crashes with "Bus error: 10" (exit code 138) immediately when dashboard API requests arrive
errors: "sh: line 1: 37873 Bus error: 10           ./mesher"
reproduction: Build and run the mesher, then curl dashboard endpoints
started: Persistent issue across multiple fix attempts

## Eliminated

- hypothesis: TargetData::create("") causes wrong data layout
  evidence: Fix applied (replaced with get_target_data()) but crash persists
  timestamp: prior to this session

- hypothesis: Missing module.set_data_layout() causes misaligned struct layout
  evidence: Fix applied but crash persists
  timestamp: prior to this session

## Evidence

- timestamp: 2026-02-15T00:01:00Z
  checked: Dashboard endpoints with invalid UUID ("default")
  found: Returns 500 error properly, no crash. Error path works fine.
  implication: Crash is on the success path, not error path.

- timestamp: 2026-02-15T00:02:00Z
  checked: /dashboard/health with valid UUID (00000000-0000-0000-0000-000000000002)
  found: Mesher crashes (SIGBUS). Curl gets empty reply, process dies.
  implication: Crash happens when query succeeds and code processes Ok(rows) result.

- timestamp: 2026-02-15T00:10:00Z
  checked: Generated LLVM IR for respond_health function
  found: Map.get results allocated as {} (empty struct/Unit), return values discarded, null passed to mesh_string_concat
  implication: codegen_call returns empty struct for Unit-typed calls, discarding the actual i64 return value

- timestamp: 2026-02-15T00:15:00Z
  checked: MIR lowering resolve_range for Map.get call expressions
  found: typeck stores Ty::Var(TyVar(N)) (unresolved type variable) for the call range
  implication: resolve_type maps Ty::Var to MirType::Unit, causing the call to have ty: Unit

- timestamp: 2026-02-15T00:20:00Z
  checked: Added diagnostic to MIR lowering for Unit-typed calls to known functions
  found: ALL Map.get and List.get calls resolve to Some(Var(TyVar(N))) - unresolved type variables. 102 calls affected across the entire mesher codebase.
  implication: This is a systemic issue in the typeck where function return types remain as unresolved type variables when the function is type-checked before its call site

- timestamp: 2026-02-15T00:30:00Z
  checked: Fix applied - fall back to callee's known return type when resolve_range returns Unit
  found: LLVM IR now correctly stores i64 return values and converts to ptr via inttoptr for string operations
  implication: Fix correctly preserves return values that were previously discarded

- timestamp: 2026-02-15T00:40:00Z
  checked: Full end-to-end test - all 4 dashboard endpoints with valid UUID
  found: All return HTTP 200 with proper JSON. No crash. Mesher stays alive.
  implication: Fix verified - SIGBUS is resolved

## Resolution

root_cause: The typeck stores unresolved type variables (Ty::Var) for return types of generic functions like Map.get and List.get when the function is type-checked before the call site that provides concrete types. The MIR type resolver maps Ty::Var to MirType::Unit. In codegen, calls with ty=Unit have their return values discarded (codegen_call returns empty struct {}). The actual i64 return value from mesh_map_get is thrown away, and null is passed to mesh_string_concat, causing SIGBUS on arm64.

fix: In lower_call_expr (crates/mesh-codegen/src/mir/lower.rs), added a fallback that checks the callee's declared return type from its FnPtr signature and known_functions when resolve_range returns Unit. For mesh_map_get this gives MirType::Int, which causes the codegen to properly store the i64 return value. When the i64 (which is actually a pointer packed as integer) is later used in string operations, the existing argument coercion in codegen_call converts it to ptr via inttoptr.

verification: Built and ran the mesher, curled all 4 dashboard endpoints (/health, /levels, /volume, /top-issues) with a valid project UUID. All return HTTP 200 with proper JSON responses. No SIGBUS. Mesher remains alive after all requests. Error paths (invalid UUID) also still work correctly (HTTP 500).

files_changed:
- crates/mesh-codegen/src/mir/lower.rs
