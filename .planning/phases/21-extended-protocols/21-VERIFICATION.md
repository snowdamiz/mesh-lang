---
phase: 21-extended-protocols
verified: 2026-02-08T10:51:45Z
status: passed
score: 16/16 must-haves verified
re_verification: false
---

# Phase 21: Extended Protocols Verification Report

**Phase Goal:** Extended Protocols -- Hash, Default, default method implementations, collection Display/Debug
**Verified:** 2026-02-08T10:51:45Z
**Status:** passed
**Re-verification:** No -- initial verification

## Goal Achievement

### Observable Truths

| # | Truth | Status | Evidence |
|---|-------|--------|----------|
| 1 | A struct with Hash impl can be used as a Map key via Map.put and Map.get | ✓ VERIFIED | Map key interception code in lower.rs:2638-2659 hashes struct keys via Hash__hash__TypeName, test `map_put_with_struct_key_hashes` passes |
| 2 | Hash__hash__StructName returns a consistent i64 for the same struct value | ✓ VERIFIED | FNV-1a implementation in hash.rs:9-16, field chaining in lower.rs:2041-2125, test `hash_int_deterministic` passes |
| 3 | Primitive types (Int, Float, String, Bool) have Hash impls registered | ✓ VERIFIED | Trait registration in builtins.rs:786-819, test `hash_trait_registered_for_primitives` passes |
| 4 | Non-generic structs auto-derive Hash impls in typeck | ✓ VERIFIED | Auto-registration in infer.rs:1515-1530, Hash impl added alongside Debug/Eq/Ord |
| 5 | default() returns 0 for Int, 0.0 for Float, false for Bool, empty string for String | ✓ VERIFIED | Short-circuits in lower.rs:2679-2684, tests `default_int_short_circuits_to_literal`, `default_float_short_circuits_to_literal`, `default_bool_short_circuits_to_literal`, `default_string_short_circuits_to_literal` all pass |
| 6 | default() return type resolved from call-site context | ✓ VERIFIED | Type resolution in lower.rs:2674-2675 via mir_type_to_impl_name(&ty), mangled name constructed from resolved type |
| 7 | Default trait registered as compiler-known with static method (no self parameter) | ✓ VERIFIED | Trait registration in builtins.rs:822-834 with has_self: false, test `default_trait_registered_for_primitives` passes |
| 8 | An interface method can have an optional do...end body that serves as a default implementation | ✓ VERIFIED | Parser accepts optional body in items.rs:516-569, AST accessor in item.rs:434-436, tests `interface_method_with_default_body` and `interface_method_without_body` pass |
| 9 | An impl block that omits a method with a default body compiles without error | ✓ VERIFIED | Error skipping in traits.rs:107, test `default_method_skips_missing_error` passes |
| 10 | The default method body executes when no override is provided | ✓ VERIFIED | Default body lowering in lower.rs:772-867, test `default_method_body_lowered_for_concrete_type` passes |
| 11 | An impl block that provides an override uses the override, not the default | ✓ VERIFIED | Override check in lower.rs:532-533, only lowers default if method NOT in provided_methods |
| 12 | to_string([1, 2, 3]) returns "[1, 2, 3]" | ✓ VERIFIED | snow_list_to_string in list.rs:298-330, test `test_list_to_string` passes with expected output |
| 13 | to_string on a Map returns a readable key-value representation | ✓ VERIFIED | snow_map_to_string in map.rs:260-303 produces "%{k => v}" format, test `test_map_to_string` passes |
| 14 | to_string on a Set returns a readable set representation | ✓ VERIFIED | snow_set_to_string in set.rs:191-225 produces "#{elem, ...}" format, test `test_set_to_string` passes |
| 15 | String interpolation with collections works | ✓ VERIFIED | MIR dispatch in lower.rs:3825-3891, tests `list_display_emits_runtime_call`, `map_display_emits_runtime_call`, `set_display_emits_runtime_call` all pass |
| 16 | Collection Display resolves element types from typeck Ty::App | ✓ VERIFIED | Type resolution in lower.rs:3826-3831, 3846-3855, 3871-3875 extracts element/key/value types from Ty::App args |

**Score:** 16/16 truths verified

### Required Artifacts

| Artifact | Expected | Status | Details |
|----------|----------|--------|---------|
| `crates/snow-rt/src/hash.rs` | FNV-1a runtime functions | ✓ VERIFIED | 72 lines, exports snow_hash_int/float/bool/string/combine, FNV constants defined, tests pass |
| `crates/snow-rt/src/lib.rs` | Module declaration | ✓ VERIFIED | Line 28: `pub mod hash;` |
| `crates/snow-typeck/src/builtins.rs` | Hash trait registration | ✓ VERIFIED | Lines 786-819: Hash trait + primitive impls registered, test coverage |
| `crates/snow-typeck/src/infer.rs` | Hash auto-derive for structs | ✓ VERIFIED | Lines 1515-1530: Hash impl auto-registered for non-generic structs |
| `crates/snow-codegen/src/mir/lower.rs` | Hash MIR generation + Map key interception | ✓ VERIFIED | generate_hash_struct (line 2041), Map key hashing (lines 2638-2659), tests pass |
| `crates/snow-codegen/src/codegen/intrinsics.rs` | Hash runtime declarations | ✓ VERIFIED | Lines 360-373: All five hash functions declared, test coverage |
| `crates/snow-typeck/src/builtins.rs` | Default trait registration | ✓ VERIFIED | Lines 822-857: Default trait + primitive impls with has_self: false |
| `crates/snow-codegen/src/mir/lower.rs` | Default short-circuits + type resolution | ✓ VERIFIED | Lines 2670-2696: default() dispatch with primitive short-circuits to literals |
| `crates/snow-parser/src/parser/items.rs` | Optional do...end body parsing | ✓ VERIFIED | Lines 512-569: parse_interface_method accepts optional DO_KW block |
| `crates/snow-parser/src/ast/item.rs` | body() accessor on InterfaceMethod | ✓ VERIFIED | Lines 431-436: pub fn body(&self) -> Option<Block> |
| `crates/snow-typeck/src/traits.rs` | has_default_body flag | ✓ VERIFIED | Line 28: field added, line 107: used to skip missing-method errors |
| `crates/snow-codegen/src/mir/lower.rs` | Default method body lowering | ✓ VERIFIED | Lines 772-867: lower_default_method re-lowers per concrete type |
| `crates/snow-rt/src/collections/list.rs` | snow_list_to_string | ✓ VERIFIED | Lines 298-330: runtime helper with elem_to_str callback, tests pass |
| `crates/snow-rt/src/collections/map.rs` | snow_map_to_string | ✓ VERIFIED | Lines 260-303: runtime helper with key/val callbacks, tests pass |
| `crates/snow-rt/src/collections/set.rs` | snow_set_to_string | ✓ VERIFIED | Lines 191-225: runtime helper with elem_to_str callback, tests pass |
| `crates/snow-codegen/src/mir/lower.rs` | Collection Display dispatch | ✓ VERIFIED | Lines 3825-3891: wrap_collection_to_string resolves types and emits runtime calls |

### Key Link Verification

| From | To | Via | Status | Details |
|------|-----|-----|--------|---------|
| lower.rs | hash.rs | MIR calls snow_hash_int/combine | ✓ WIRED | generate_hash_struct emits calls to runtime functions, intrinsics declared |
| lower.rs | builtins.rs | TraitRegistry lookup for Hash impls | ✓ WIRED | Line 2641: has_impl("Hash", &ty_for_lookup) checks before hashing |
| lower.rs | builtins.rs | TraitRegistry lookup for Default impls | ✓ WIRED | default() resolution uses type_name from context to construct mangled name |
| items.rs | item.rs | Parser produces INTERFACE_METHOD with BLOCK | ✓ WIRED | parse_interface_method creates BLOCK child, body() reads it |
| infer.rs | traits.rs | Sets has_default_body on TraitMethodSig | ✓ WIRED | method.body().is_some() check sets flag (infer.rs infer_interface_def) |
| lower.rs | traits.rs | Checks has_default_body when processing impls | ✓ WIRED | Lines 532, 251: has_default_body checked before lowering default bodies |
| lower.rs | list.rs | MIR calls snow_list_to_string | ✓ WIRED | Line 3835: Call emitted with elem callback, intrinsic declared |
| lower.rs | map.rs | MIR calls snow_map_to_string | ✓ WIRED | Line 3860: Call emitted with key/val callbacks, intrinsic declared |
| lower.rs | set.rs | MIR calls snow_set_to_string | ✓ WIRED | Line 3879: Call emitted with elem callback, intrinsic declared |
| lower.rs | typeck | Ty::App element type resolution | ✓ WIRED | Lines 3826-3831, 3846-3855: Extracts elem/key/val types from Ty::App args |

### Requirements Coverage

No REQUIREMENTS.md entries explicitly mapped to Phase 21. Phase contributes to v1.3 milestone "Traits & Protocols" overall success criteria.

### Anti-Patterns Found

None. All implementations are substantive:
- No TODO/FIXME/placeholder comments in production code
- No stub patterns (empty returns, console.log only)
- All functions have real implementations with tests
- All tests pass (112 tests total across workspace, 0 failures)

### Test Coverage

**Plan 21-01 (Hash Protocol):**
- ✓ `hash_trait_registered_for_primitives` (builtins.rs)
- ✓ `hash_int_deterministic` (hash.rs)
- ✓ `hash_bool_deterministic` (hash.rs)
- ✓ `hash_combine_order_matters` (hash.rs)
- ✓ `hash_struct_generates_mir_function` (lower.rs)
- ✓ `hash_struct_field_chaining` (lower.rs)
- ✓ `map_put_with_struct_key_hashes` (lower.rs)

**Plan 21-02 (Default Protocol):**
- ✓ `default_trait_registered_for_primitives` (builtins.rs)
- ✓ `default_int_short_circuits_to_literal` (lower.rs)
- ✓ `default_float_short_circuits_to_literal` (lower.rs)
- ✓ `default_bool_short_circuits_to_literal` (lower.rs)
- ✓ `default_string_short_circuits_to_literal` (lower.rs)

**Plan 21-03 (Default Method Implementations):**
- ✓ `interface_method_with_default_body` (parser_tests.rs)
- ✓ `interface_method_without_body` (parser_tests.rs)
- ✓ `default_method_skips_missing_error` (lower.rs)
- ✓ `default_method_body_lowered_for_concrete_type` (lower.rs)

**Plan 21-04 (Collection Display):**
- ✓ `test_list_to_string` (list.rs)
- ✓ `test_list_to_string_empty` (list.rs)
- ✓ `test_map_to_string` (map.rs)
- ✓ `test_map_to_string_empty` (map.rs)
- ✓ `test_set_to_string` (set.rs)
- ✓ `test_set_to_string_empty` (set.rs)
- ✓ `list_display_emits_runtime_call` (lower.rs)
- ✓ `map_display_emits_runtime_call` (lower.rs)
- ✓ `set_display_emits_runtime_call` (lower.rs)

**Total:** 26 phase-specific tests, all passing. Full workspace test suite: 112 tests, 0 failures.

### Human Verification Required

None. All verification completed programmatically via:
1. Source code inspection (artifacts exist, substantive, wired)
2. Test execution (all tests pass)
3. Build verification (cargo build --workspace succeeds)

---

_Verified: 2026-02-08T10:51:45Z_
_Verifier: Claude (gsd-verifier)_
