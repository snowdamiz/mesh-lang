# Phase 5: LLVM Codegen & Native Binaries - Context

**Gathered:** 2026-02-06
**Status:** Ready for planning

<domain>
## Phase Boundary

Complete compilation pipeline from Snow source to native single-binary executables for all sequential language features: functions, pattern matching, algebraic data types, closures, pipe operator, and string interpolation. Actor runtime and concurrency are separate phases (6+).

</domain>

<decisions>
## Implementation Decisions

### Mid-level IR design
- Claude's discretion on whether to introduce a dedicated MIR or lower directly from typed AST to LLVM IR
- Claude's discretion on monomorphization vs type erasure for generics
- Claude's discretion on ADT memory layout (tagged unions, pointer tagging, etc.)
- Strings are GC-managed — prepare for per-actor GC from Phase 6 even though actors aren't implemented yet. This means a simple GC (at minimum a bump allocator or basic mark-sweep) needs to exist in the runtime stub

### Pattern match compilation
- Claude's discretion on decision tree vs backtracking strategy and optimization level
- Guards stay restricted (comparisons, boolean ops, literals, name refs, named function calls) — do not expand guard expressiveness
- Runtime match failure (guarded arms edge case) panics with source location + "non-exhaustive match" message and aborts
- Or-patterns duplicate the arm body for each alternative — simpler codegen, let LLVM deduplicate

### Closure & capture strategy
- Claude's discretion on capture semantics (copy vs reference) based on Snow's semantics and GC model
- Claude's discretion on closure representation (fat pointers vs heap objects) — should align with GC-managed memory model
- Claude's discretion on partial application / currying support
- Claude's discretion on pipe operator compilation strategy (syntactic sugar vs special IR node)

### CLI & binary output
- `snowc build <dir>` is the primary command — project-based compilation, not single-file
- Support both -O0 (debug) and -O2 (release) optimization levels
- Reuse ariadne diagnostic rendering for all compilation errors (consistent with parse/type-check errors)
- `snowc build --emit-llvm` flag to dump .ll file alongside binary for codegen inspection/debugging

### Claude's Discretion
- MIR vs direct lowering architecture
- Monomorphization vs type erasure
- ADT memory layout strategy
- Decision tree algorithm and optimization level
- Closure capture semantics and representation
- Partial application support
- Pipe operator compilation approach

</decisions>

<specifics>
## Specific Ideas

- GC-managed strings chosen explicitly to prepare for Phase 6's per-actor GC — the runtime stub in Phase 5 should lay groundwork for this
- Project-based build (`snowc build <dir>`) implies some project structure discovery (entry point detection or manifest)
- The success criteria require: macOS + Linux cross-platform, under 5 seconds for 100-line programs at -O0, and single binary with no external runtime dependencies

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 05-llvm-codegen-native-binaries*
*Context gathered: 2026-02-06*
