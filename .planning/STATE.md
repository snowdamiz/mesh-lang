# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-09)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.
**Current focus:** v1.8 Module System -- Phase 41 (MIR Merge Codegen) -- COMPLETE

## Current Position

Phase: 41 of 42 (MIR Merge Codegen) -- COMPLETE
Plan: 1 of 1 in current phase (41-01 complete)
Status: Phase 41 Complete
Last activity: 2026-02-09 -- Completed 41-01 (module-qualified name mangling + E2E tests)

Progress: [||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||] 100% (127/~127 plans est.)

## Performance Metrics

**All-time Totals:**
- Plans completed: 127
- Phases completed: 41
- Milestones shipped: 8 (v1.0-v1.7)
- Lines of Rust: 70,501
- Timeline: 5 days (2026-02-05 -> 2026-02-09)

## Accumulated Context

### Decisions

Decisions logged in PROJECT.md Key Decisions table.

Recent for v1.8:
- lower_to_mir_raw per module then merge_mir_modules with post-merge monomorphization -- prevents unreachable builtin codegen failures
- Track qualified_modules and imported_functions in TypeckResult -- MIR lowerer needs module awareness for qualified access and trait dispatch skip
- Single LLVM module approach (merge MIR, not separate compilation) -- avoids cross-module linking complexity
- Module-qualified type names from day one -- prevents type identity issues across module boundaries
- Hand-written Kahn's algorithm for toposort -- avoids petgraph dependency for simple DAG
- Sequential u32 IDs for ModuleId -- simple, zero-allocation, direct Vec indexing
- FxHashMap for module name-to-id lookup -- low overhead for small module counts
- Hidden directory skipping in file discovery -- prevents .git/.hidden from being treated as modules
- Alphabetical tie-breaking in toposort for deterministic compilation order across platforms
- Silent skip for unknown imports during graph construction -- Phase 39 handles error reporting
- Two-phase graph construction: register all modules first, then parse and build edges
- ProjectData struct retains parse results for downstream compilation -- eliminates double-parsing
- build_module_graph preserved as thin wrapper for backward compatibility with Phase 37 tests
- Parse errors retained in ProjectData without failing build_project -- caller handles error reporting
- Parse errors checked for ALL modules before type checking; type checking skipped if any parse errors
- Entry module found via is_entry flag in compilation_order, not hardcoded index
- infer() delegates to infer_with_imports(empty) for backward-compatible code reuse
- ImportContext pre-seeds TraitRegistry, TypeRegistry, TypeEnv before inference
- collect_exports extracts impl names from AST PATH nodes matching infer_impl_def pattern
- qualified_modules stored on InferCtx to avoid parameter threading cascade across 6000+ line file
- User modules checked before stdlib in both import handling and field access resolution
- Trait impls remain unconditionally exported (XMOD-05) while trait defs are gated by pub visibility
- PrivateItem error only for selective imports (from-import) -- qualified access to private items produces natural unbound/no-such-field errors
- qualify_name method with prefix checks for builtins, trait impls, pub fns, and main -- prevents incorrect prefixing
- user_fn_defs set tracks FnDef items separately from variant constructors for call-site qualification
- Module-qualified naming: ModuleName__private_fn using double-underscore separator at MIR level

### Research Notes

Research complete (see .planning/research/SUMMARY.md):
- All import/module/pub syntax already parsed by existing parser
- Phase 39 (cross-module type checking) is the critical complexity center
- Type identity across module boundaries is the primary risk
- No new dependencies needed

### Pending Todos

None.

### Blockers/Concerns

None.

## Session Continuity

Last session: 2026-02-09
Stopped at: Completed 41-01-PLAN.md (Phase 41 complete)
Resume file: None
Next action: Begin Phase 42 (Diagnostics & Integration)
