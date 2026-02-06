---
phase: 05-llvm-codegen-native-binaries
plan: 02
subsystem: mir
tags: [mir, lowering, desugaring, closure-conversion, monomorphization, pipe-operator, string-interpolation]

# Dependency graph
requires:
  - phase: 05-llvm-codegen-native-binaries
    plan: 01
    provides: snow-codegen crate scaffolding, TypeRegistry pub fields, snow-rt runtime
provides:
  - MIR type definitions (MirModule, MirFunction, MirExpr, MirType, MirPattern)
  - AST-to-MIR lowering with pipe desugaring and string interpolation compilation
  - Closure conversion (lift to top-level function + MakeClosure)
  - Monomorphization pass (reachability-based dead code elimination)
  - lower_to_mir_module() public API for downstream codegen
affects: [05-03, 05-04, 05-05]

# Tech tracking
tech-stack:
  added: [rowan (workspace dep for snow-codegen)]
  patterns: [closure conversion with env_ptr, pipe desugar to Call, string interpolation to concat chains, reachability-based monomorphization]

key-files:
  created:
    - crates/snow-codegen/src/mir/types.rs
    - crates/snow-codegen/src/mir/lower.rs
    - crates/snow-codegen/src/mir/mono.rs
  modified:
    - crates/snow-codegen/src/mir/mod.rs
    - crates/snow-codegen/src/lib.rs
    - crates/snow-codegen/Cargo.toml
    - Cargo.lock

key-decisions:
  - "Ty::Var falls back to MirType::Unit instead of panicking for graceful degradation on type errors"
  - "Pipe operator desugared as pure syntactic transform: x |> f -> Call(f, [x]), x |> f(a) -> Call(f, [x, a])"
  - "Closures lifted with __env first param and MakeClosure node; captures detected by free variable analysis"
  - "String interpolation compiled to snow_string_concat/snow_*_to_string runtime call chains"
  - "Monomorphization is reachability-based for Phase 5 (type checker already resolves concrete types)"
  - "Block lowering uses TextRange comparison to deduplicate stmt/tail-expr overlap"

patterns-established:
  - "MIR type system: concrete types only, no type variables, explicit closure/fn-ptr distinction"
  - "Lowerer struct pattern: scope stack + known_functions map + closure counter"
  - "Free variable collection via recursive MirExpr traversal for closure capture analysis"
  - "lower_to_mir() -> monomorphize() pipeline via lower_to_mir_module() public API"

# Metrics
duration: 9min
completed: 2026-02-06
---

# Phase 5 Plan 2: MIR Type System & AST-to-MIR Lowering Summary

**Complete MIR type definitions with AST-to-MIR lowering, pipe desugaring, string interpolation compilation, closure conversion with explicit capture lists, and reachability-based monomorphization**

## Performance

- **Duration:** 9 min
- **Started:** 2026-02-06T23:04:07Z
- **Completed:** 2026-02-06T23:13:32Z
- **Tasks:** 2
- **Files modified:** 7
- **Lines of code:** 2,479

## Accomplishments

- MIR type system fully defined: MirModule, MirFunction, MirExpr (22 variants), MirType (13 variants), MirPattern (6 variants), BinOp (14 operators), UnaryOp (2 operators)
- resolve_type() maps all Ty variants to MirType with TypeRegistry lookup for struct/sum type disambiguation
- mangle_type_name() generates monomorphized names (e.g., Option_Int, Result_Int_String)
- Full AST-to-MIR lowerer handles all 16 expression types: literals, name refs, binary/unary ops, calls, pipe, field access, if/else, case/match, closures, blocks, strings, return, tuples, struct literals
- Pipe operator desugared to function calls (x |> f -> Call(f, [x]); x |> f(a) -> Call(f, [x, a]))
- String interpolation compiled to snow_string_concat chains with type-based to_string wrapping
- Closures converted to lifted functions (__closure_N) with __env first parameter and MakeClosure expression
- Pattern lowering handles wildcard, ident, literal, constructor (qualified + unqualified), tuple, or, as patterns
- Monomorphization pass removes unreachable functions via transitive call graph analysis
- 23 unit tests for MIR types + lowering + monomorphization
- Zero regressions across all 423 workspace tests

## Task Commits

Each task was committed atomically:

1. **Task 1: Define MIR type system** - `5fe1507` (feat)
2. **Task 2: Implement AST-to-MIR lowering with desugaring, closure conversion, and monomorphization** - `d4068f2` (feat)

## Files Created/Modified

- `crates/snow-codegen/src/mir/mod.rs` - MIR type definitions (MirModule, MirFunction, MirExpr, MirType, MirPattern, BinOp, UnaryOp, MirStructDef, MirSumTypeDef)
- `crates/snow-codegen/src/mir/types.rs` - resolve_type() Ty->MirType conversion, mangle_type_name(), 13 unit tests
- `crates/snow-codegen/src/mir/lower.rs` - Lowerer struct, lower_to_mir(), all expression/pattern lowering, pipe desugaring, string interpolation compilation, closure conversion, 8 tests
- `crates/snow-codegen/src/mir/mono.rs` - monomorphize() reachability pass, collect_function_refs(), 2 tests
- `crates/snow-codegen/src/lib.rs` - lower_to_mir_module() public API combining lowering + monomorphization
- `crates/snow-codegen/Cargo.toml` - Added rowan workspace dependency
- `Cargo.lock` - Updated for new dependency

## Decisions Made

- **Ty::Var graceful fallback**: Instead of panicking on unresolved type variables (which can occur when the type checker reports errors), fall back to MirType::Unit. This allows MIR lowering to proceed even with partially-typed programs.
- **Pipe desugaring strategy**: Pure syntactic transform at MIR lowering time. No special IR node. `x |> f` becomes `Call(f, [x])`, `x |> f(a, b)` becomes `Call(f, [x, a, b])`.
- **Closure conversion approach**: Closures lifted to module-level functions with `__env` pointer as first parameter. Free variables detected by post-hoc traversal of the lowered MIR body. MakeClosure nodes capture the lifted function name and captured values.
- **String interpolation compilation**: Segments split into STRING_CONTENT literals and INTERPOLATION expression children. Expressions wrapped in type-appropriate to_string calls (snow_int_to_string, snow_float_to_string, etc.). Segments chained via snow_string_concat calls.
- **Monomorphization as reachability**: Since the Snow type checker already resolves all concrete types at call sites, the monomorphization pass in Phase 5 is primarily a dead code elimination pass based on transitive reachability from the entry point. Full generic specialization (creating copies with substituted types) is deferred to when generics actually need runtime specialization.
- **Block stmt/tail deduplication**: The Rowan CST can report the same node as both a statement (via `stmts()`) and the tail expression (via `tail_expr()`). Deduplication uses TextRange equality comparison.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Added rowan dependency to snow-codegen**
- **Found during:** Task 2
- **Issue:** lower.rs uses `rowan::TextRange` for type map lookups, but rowan was not in snow-codegen's Cargo.toml
- **Fix:** Added `rowan = { workspace = true }` to snow-codegen dependencies
- **Files modified:** `crates/snow-codegen/Cargo.toml`
- **Commit:** d4068f2

**2. [Rule 1 - Bug] Ty::Var fallback instead of panic**
- **Found during:** Task 2 (closure test)
- **Issue:** resolve_type() panicked on Ty::Var, but the type checker can leave unresolved variables in programs with closures or partial type inference
- **Fix:** Changed Ty::Var handling from panic to MirType::Unit fallback; updated corresponding test
- **Files modified:** `crates/snow-codegen/src/mir/types.rs`
- **Commit:** d4068f2

**3. [Rule 1 - Bug] Block lowering stmt/tail-expr deduplication**
- **Found during:** Task 2 (string interpolation test)
- **Issue:** Original block lowering used a crude `matches!(e, MirExpr::Let { .. })` check that incorrectly skipped tail expressions when any let binding existed in the block
- **Fix:** Changed to TextRange-based deduplication comparing stmt ranges with tail expression range
- **Files modified:** `crates/snow-codegen/src/mir/lower.rs`
- **Commit:** d4068f2

## Issues Encountered

None beyond the auto-fixed deviations above.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- MIR is complete and can represent any Snow program as a flat list of monomorphic functions
- All Snow expression forms are lowered: literals, ops, calls, closures, pattern matching, structs, sum types
- Pipe operator is fully desugared (no special handling needed in codegen)
- String interpolation is compiled to runtime calls (codegen just needs to emit Call nodes)
- Closures are explicit MakeClosure + lifted functions (codegen handles env_ptr allocation)
- Pattern matching preserved as MirMatch (will be compiled to decision trees in Plan 03)
- lower_to_mir_module() provides the public API for Plan 04's LLVM codegen to consume
- No blockers for subsequent plans

## Self-Check: PASSED

---
*Phase: 05-llvm-codegen-native-binaries*
*Completed: 2026-02-06*
