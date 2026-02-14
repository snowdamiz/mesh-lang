# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-13)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural and clean as writing sequential code, with the safety net of supervision and fault tolerance built into the language.
**Current focus:** v7.0 Phase 77 - From/Into Conversion

## Current Position

Phase: 77 of 79 (From/Into Conversion)
Plan: 3 of 3 in current phase (COMPLETE)
Status: Phase 77 complete (gap closure done)
Last activity: 2026-02-14 -- Plan 77-03 (Gap Closure: Struct Error Types) complete, 2/2 tasks

Progress: [██████████] 100% (v7.0)

## Performance Metrics

**All-time Totals:**
- Plans completed: 209
- Phases completed: 76
- Milestones shipped: 16 (v1.0-v6.0)
- Lines of Rust: ~93,500
- Lines of website: ~5,100
- Timeline: 9 days (2026-02-05 -> 2026-02-13)

## Accumulated Context

### Decisions

(See PROJECT.md Key Decisions table for full log)

**Phase 74-01:**
- Used dedicated parse_impl_body instead of modifying shared parse_item_block_body to avoid changing code paths used by module/function bodies
- Created separate ASSOC_TYPE_BINDING SyntaxKind (not reusing TYPE_ALIAS_DEF) for distinct CST semantics

**Phase 74-02:**
- Uppercase Self is IDENT (not SELF_KW) -- resolution uses IDENT text matching
- Associated type bindings extracted by iterating tokens after EQ in ASSOC_TYPE_BINDING node
- resolve_self_assoc_type filters whitespace trivia before pattern matching

**Phase 74-03:**
- Skip trait method return type comparison when expected type contains Self (Self.Item resolves only in impl context)
- MIR lowering naturally handles associated types: ImplDef.methods() already skips ASSOC_TYPE_BINDING nodes via FnDef::cast
- ExportedSymbols carries associated types through clone (no changes needed)

**Phase 75-01:**
- Primitives (Int/Float) bypass Neg trait check via fast path for backward compat and performance
- Output resolution falls back to operand type when no Output defined (backward compat)

**Phase 75-02:**
- Parser disambiguates self keyword: self() is actor self-call, self.x is method receiver field access
- Typeck fn_ty for impl methods includes all params (self + non-self) for correct MIR lowering
- Comparison ops keep Bool return; arithmetic ops use Output type from resolve_range
- [Phase 76]: Used opaque TyCon names (ListIterator, MapIterator, etc.) for iterator handle types in trait registry
- [Phase 76]: Two-trait protocol: Iterable for collections, Iterator for stateful cursor; infer_for_in checks Iterable first then Iterator

**Phase 76-02:**
- Two-phase iterator function resolution: user-compiled functions first, then built-in runtime mapping table
- Iterator handle types (ListIterator etc.) resolve to MirType::Ptr (opaque pointers)
- Iter.from() delegates to mesh_list_iter_new; future phases can add type-tag dispatch
- MeshOption struct layout: { tag: u8, value: *mut u8 } with tag 0=Some, 1=None

**Phase 77-01:**
- Synthetic Into generation inserts directly into impls HashMap (no re-entry to register_impl) to avoid infinite recursion
- Duplicate detection for parameterized traits compares trait_type_args via unification
- String.from uses polymorphic TyVar(91100) input; Float.from is monomorphic (Int only)
- GENERIC_ARG_LIST extraction in infer_impl_def uses direct children of IMPL_DEF node

**Phase 77-02:**
- mangle_trait_method helper centralizes parameterized trait mangling (From_Int__from__Float)
- Static trait methods (no self) do not prepend impl_type to param list (fixes From.from)
- ctx.errors.truncate used for error rollback when From impl exists in ? operator type checking
- Monomorphized Result name parsing extracts error type (Result_Int_String -> String)

**Phase 77-03:**
- MirType::Struct normalized to Ptr in lower_try_result From conversion to match Result { i8, ptr } layout
- Struct-to-ptr coercion in codegen_call: GC-allocate struct return values when MIR expects Ptr
- Two-layer fix needed: MIR normalization + codegen coercion (user-defined functions return structs by value, not as pointers)

### Research Notes

v7.0 research completed (HIGH confidence). Key findings:
- Associated types are foundational -- Iterator, Numeric Output, and Collect all depend on them
- Monomorphization simplifies design vs Rust (every projection must normalize before MIR)
- Existing for-in loops MUST be preserved as-is; Iterator-based for-in is a fallback path
- From/Into uses synthetic impl generation (not blanket impls)
- Depth limit (64) needed for projection resolution to prevent infinite loops

### Pending Todos

None.

### Blockers/Concerns

None.

### Quick Tasks Completed

| # | Description | Date | Commit | Directory |
|---|-------------|------|--------|-----------|
| 1 | Rename project from Snow to Mesh, change .snow file extension to .mpl | 2026-02-13 | 3fe109e1 | [1-rename-project-from-snow-to-mesh-change-](./quick/1-rename-project-from-snow-to-mesh-change-/) |
| 2 | Write article: How Opus 4.6 and I Built a Production-Ready Programming Language in 9 Days | 2026-02-13 | (current) | [2-mesh-story-article](./quick/2-mesh-story-article/) |

## Session Continuity

Last session: 2026-02-14
Stopped at: Completed 77-03-PLAN.md (Gap Closure: Struct Error Types in Result)
Resume file: None
Next action: Phase 77 fully complete (all 4 success criteria verified). Proceed to Phase 78 if planned.
