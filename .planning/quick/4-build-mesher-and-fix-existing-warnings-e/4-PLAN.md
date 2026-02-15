---
phase: quick-4
plan: 01
type: execute
wave: 1
depends_on: []
files_modified:
  - crates/mesh-codegen/src/mir/lower.rs
  - crates/mesh-codegen/src/codegen/expr.rs
  - crates/mesh-lexer/src/lib.rs
  - crates/mesh-lexer/src/cursor.rs
  - crates/mesh-parser/src/parser/mod.rs
  - crates/mesh-rt/src/db/pg.rs
  - crates/mesh-rt/src/collections/list.rs
  - crates/meshc/src/main.rs
  - crates/meshc/src/discovery.rs
autonomous: true

must_haves:
  truths:
    - "cargo run -- build mesher/ produces no Rust compiler warnings"
    - "cargo run -- build mesher/ produces no mesh-codegen warnings"
    - "cargo run -- build mesher/ compiles to mesher/mesher binary successfully"
    - "cargo test passes (no regressions)"
  artifacts:
    - path: "mesher/mesher"
      provides: "Compiled Mesh binary"
  key_links: []
---

<objective>
Clean up all compiler warnings when building the mesher project.

Purpose: The mesher project compiles successfully (produces mesher/mesher binary) but emits 353 false-positive "[mesh-codegen] warning: could not be resolved as a trait method" warnings from the MIR lowerer, plus 15 Rust compiler warnings (unused variables, dead code, unused assignments). Both categories should be silenced properly.

Output: Warning-free build of mesher/ and clean cargo build.
</objective>

<execution_context>
@/Users/sn0w/.claude/get-shit-done/workflows/execute-plan.md
@/Users/sn0w/.claude/get-shit-done/templates/summary.md
</execution_context>

<context>
@.planning/STATE.md
</context>

<tasks>

<task type="auto">
  <name>Task 1: Fix false-positive MIR lowerer trait method warnings</name>
  <files>crates/mesh-codegen/src/mir/lower.rs</files>
  <action>
The warning at line ~5432-5438 fires for ALL module-scoped helper functions and compiler-generated service dispatch functions (__service_*_start, __service_*_call_*, __service_*_cast_*, mesh_println, mesh_process_register, mesh_process_whereis, and Module__helper_name patterns). These are NOT trait method calls -- they are:

1. Module-scoped helper functions (e.g., `Api_Alerts__format_nullable_ts`) -- called within their own module, resolved as plain functions, not methods on a type
2. Compiler-generated service dispatch stubs (e.g., `__service_ratelimiter_start`, `__service_pipelineregistry_call_get_pool`)
3. Runtime intrinsics (e.g., `mesh_println`, `mesh_process_register`, `mesh_process_whereis`)

The current warning fires when `self.lookup_var(name)` returns None, but these functions ARE in `self.known_functions` -- they just don't go through the trait method dispatch path above. The warning is a false positive because the function exists and will be resolved during codegen.

Fix: Before emitting the warning, also check `self.known_functions.contains_key(name)`. If the function exists in known_functions, skip the warning. The warning should only fire when neither lookup_var NOR known_functions can find the function. Additionally, suppress warnings for names matching the patterns `__service_*` and `mesh_*` (runtime intrinsics) since these are always generated/provided by the compiler/runtime.

The fix should be at the `if self.lookup_var(name).is_none()` check around line 5432. Change to:
```rust
if self.lookup_var(name).is_none()
    && !self.known_functions.contains_key(name)
    && !name.starts_with("__service_")
    && !name.starts_with("mesh_")
{
```

Verify the `known_functions` field exists on the lowerer struct. If it's named differently, find the correct field by searching for where module-scoped functions are registered.
  </action>
  <verify>Run `cargo run -- build mesher/ 2>&1 | grep -c "could not be resolved"` -- should return 0. Run `cargo test` to ensure no regressions.</verify>
  <done>Zero "[mesh-codegen] warning: could not be resolved as a trait method" warnings when building mesher/</done>
</task>

<task type="auto">
  <name>Task 2: Fix Rust compiler warnings (unused variables, dead code)</name>
  <files>
    crates/mesh-codegen/src/codegen/expr.rs
    crates/mesh-codegen/src/mir/lower.rs
    crates/mesh-lexer/src/lib.rs
    crates/mesh-lexer/src/cursor.rs
    crates/mesh-parser/src/parser/mod.rs
    crates/mesh-rt/src/db/pg.rs
    crates/mesh-rt/src/collections/list.rs
    crates/meshc/src/main.rs
    crates/meshc/src/discovery.rs
  </files>
  <action>
Fix all 15 Rust compiler warnings. For each warning, apply the minimal fix:

**Unused variables (prefix with _):**
1. `crates/mesh-codegen/src/codegen/expr.rs:1174` -- `ptr_type` -> `_ptr_type`
2. `crates/mesh-codegen/src/mir/lower.rs:2599` -- `fields` -> `_fields`
3. `crates/mesh-codegen/src/mir/lower.rs:5570` -- `ref name` -> `ref _name`
4. `crates/mesh-codegen/src/mir/lower.rs:8178` -- `err_body_ty` -> `_err_body_ty`
5. `crates/mesh-codegen/src/mir/lower.rs:8343` -- `total_size` -> `_total_size`
6. `crates/meshc/src/main.rs:237` -- `entry_idx` -> `_entry_idx`

**Unused assignment:**
7. `crates/mesh-rt/src/db/pg.rs:836` -- `let mut last_txn_status: u8 = b'I'` -- prefix with underscore: `let mut _last_txn_status: u8 = b'I'`

**Dead code (prefix with _ or add #[allow(dead_code)]):**
8. `crates/mesh-lexer/src/lib.rs:28` -- `source` field on Lexer -- add `#[allow(dead_code)]` above the field, or prefix with `_source` if it won't break anything (check for usages first)
9. `crates/mesh-lexer/src/cursor.rs:49` -- `is_eof` method -- add `#[allow(dead_code)]` above the method (it may be useful in future)
10. `crates/mesh-parser/src/parser/mod.rs:56` -- `Error` variant -- add `#[allow(dead_code)]` above the variant
11. `crates/mesh-parser/src/parser/mod.rs:200` -- `at_any` method -- add `#[allow(dead_code)]`
12. `crates/mesh-parser/src/parser/mod.rs:286` -- `advance_with_error` method -- add `#[allow(dead_code)]`
13. `crates/mesh-rt/src/collections/list.rs:27` -- `list_cap` function -- add `#[allow(dead_code)]`
14. `crates/mesh-codegen/src/mir/lower.rs:370` -- `resolve_range_closure` method -- add `#[allow(dead_code)]`
15. `crates/mesh-codegen/src/mir/lower.rs:4939` -- `lower_let_binding` method -- add `#[allow(dead_code)]`
16. `crates/mesh-codegen/src/mir/lower.rs:8838` -- `variant_name` field in CallInfo -- add `#[allow(dead_code)]`
17. `crates/mesh-codegen/src/mir/lower.rs:8847` -- `variant_name` field in CastInfo -- add `#[allow(dead_code)]`
18. `crates/meshc/src/main.rs:468` -- `report_diagnostics` function -- add `#[allow(dead_code)]`
19. `crates/meshc/src/discovery.rs:258` -- `build_module_graph` function -- add `#[allow(dead_code)]`

Use `#[allow(dead_code)]` for methods/functions/fields that appear intentionally kept for future use. Use `_` prefix for truly unused bindings in destructuring/let.
  </action>
  <verify>Run `cargo build 2>&1 | grep "^warning:"` -- should return zero warning lines. Run `cargo test` to ensure no regressions.</verify>
  <done>Zero Rust compiler warnings during cargo build and cargo run -- build mesher/</done>
</task>

</tasks>

<verification>
1. `cargo build 2>&1 | grep -c "^warning:"` returns 0
2. `cargo run -- build mesher/ 2>&1 | grep -c "could not be resolved"` returns 0
3. `cargo run -- build mesher/ 2>&1 | grep "Compiled:"` shows successful binary output
4. `cargo test` passes with no failures
</verification>

<success_criteria>
- Build mesher/ produces zero warnings (both Rust-level and mesh-codegen level)
- Binary mesher/mesher is produced successfully
- All existing tests pass (cargo test)
</success_criteria>

<output>
After completion, create `.planning/quick/4-build-mesher-and-fix-existing-warnings-e/4-SUMMARY.md`
</output>
