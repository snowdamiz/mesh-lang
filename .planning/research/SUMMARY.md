# Research Summary: Module System

**Project:** Snow Compiler - Module System (v1.8)
**Domain:** Multi-file compilation with module resolution, cross-file type checking, visibility enforcement
**Researched:** 2026-02-09
**Overall confidence:** HIGH

## Executive Summary

Snow's module system requires ZERO new external dependencies. The implementation extends the existing single-file compilation pipeline to handle multiple `.snow` files with import resolution, visibility enforcement, and unified code generation. All necessary syntax (module, import, from...import, pub, qualified paths) is already parsed by the existing parser -- the work is entirely in the semantic layers (type checker, MIR lowering, codegen) and the compiler driver.

The key architectural decision is to compile all modules into a single LLVM module after merging their MIR representations. This avoids the complexity of cross-module LLVM symbol resolution and linking, gives LLVM maximum optimization opportunity, and keeps the codegen pipeline unchanged. The trade-off is full recompilation on every change, which is acceptable at Snow's project scale.

The most critical risk is type identity across module boundaries: when module B imports a struct from module A, both modules must recognize the struct as the same type. This requires module-qualified type names throughout the entire pipeline (TypeRegistry, MirType, LLVM struct types). Getting this wrong produces silent memory corruption or "expected Point, got Point" type errors. The prevention is straightforward -- qualify all type names from the start -- but must be applied consistently across 70K+ lines of existing code.

A hand-written topological sort (Kahn's algorithm, ~40 lines) handles dependency ordering and cycle detection. This is preferred over petgraph (10K+ line dependency) because the module graph is a simple DAG with typically 5-50 nodes.

## Key Findings

**Stack:** No new dependencies. Hand-written toposort, extended TypeEnv with module exports, MIR merge with name mangling. Existing rustc-hash, rowan, ariadne, inkwell are sufficient.

**Architecture:** Parse all files independently, build dependency graph, type-check in topological order accumulating exports, merge MIR, compile single LLVM module. The pipeline is: Discovery -> Parse All -> Graph + Toposort -> Sequential Typecheck -> MIR Lower + Merge -> Codegen.

**Critical pitfall:** Type identity across modules. Module-qualified names must be used as keys in TypeRegistry, MirType, and LLVM type caches from day one. Retrofitting this after building on bare names would require touching every pipeline stage.

## Implications for Roadmap

Based on research, suggested phase structure:

1. **Module Graph Foundation** - File discovery, module naming convention, ModuleGraph data structure, topological sort with cycle detection
   - Addresses: FEATURES.md multi-file discovery, circular dependency detection
   - Avoids: PITFALLS.md P5 (cycle detection bugs), P11 (file system edge cases)
   - This is pure infrastructure with no impact on existing compilation

2. **Multi-File Parse + Build Pipeline** - Parse all files, extend snowc build to orchestrate multi-file compilation, single-file backward compatibility
   - Addresses: FEATURES.md multi-file compilation
   - Avoids: PITFALLS.md P7 (backward compatibility breaks)
   - Existing tests must continue to pass unchanged

3. **Cross-Module Type Checking** - Extend TypeEnv with module exports, check_with_imports entry point, module-qualified names in TypeRegistry, export collection from pub items
   - Addresses: FEATURES.md import resolution, qualified access, cross-module function calls, cross-module struct/sum type sharing
   - Avoids: PITFALLS.md P1 (type identity), P3 (no cross-module type context)
   - This is the largest and most complex phase

4. **Visibility Enforcement** - Enforce pub/private boundaries, validate public interfaces
   - Addresses: FEATURES.md visibility (pub/private)
   - Avoids: PITFALLS.md P6 (visibility leaks)
   - Can be a focused phase because it is a validation pass on top of working cross-module types

5. **MIR Merge + Name Mangling + Codegen** - Module-qualified function names, MIR merge, unified codegen
   - Addresses: FEATURES.md name collision prevention, ARCHITECTURE.md MIR merge pattern
   - Avoids: PITFALLS.md P2 (name mangling collisions), P4 (monomorphization explosion)
   - Natural final phase: once types check, lowering and codegen follow existing patterns

6. **Diagnostics + Integration** - Multi-file error messages, qualified type names in errors, comprehensive e2e tests
   - Addresses: FEATURES.md differentiators (unused import warnings, LSP support)
   - Avoids: PITFALLS.md P12 (useless error messages)
   - Polish phase: working but rough module system becomes production-ready

**Phase ordering rationale:**
- Foundation before semantics: Module graph and file discovery must exist before type checking can use them
- Parse before check: All files must be parsed before dependency graph can be built from import declarations
- Type checking before codegen: Cross-module types must resolve before MIR lowering can produce correct output
- Visibility after types: Pub/private enforcement is a constraint on top of working cross-module type resolution
- Diagnostics last: Good error messages are important but not blocking for correct compilation

**Research flags for phases:**
- Phase 3 (Cross-Module Type Checking): Most complex phase. Likely needs deeper design work on TypeRegistry sharing and qualified name resolution. The interface between existing `check()` and new `check_with_imports()` requires careful design.
- Phase 5 (MIR Merge): Standard patterns. The monomorphization pass runs unchanged on merged MIR. Name mangling is mechanical.
- Phase 1, 2: Standard patterns. File discovery and graph algorithms are well-understood.

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| Stack | HIGH | Direct codebase analysis confirms all integration points. No new deps needed. Evaluated and rejected petgraph, lasso, salsa. |
| Features | HIGH | Module systems are deeply studied. Snow's requirements (file-based modules, pub/private, qualified access) are standard patterns. |
| Architecture | HIGH | Single-LLVM-module approach, topo-sort type checking, MIR merge are all established patterns used by production compilers. |
| Pitfalls | HIGH | Critical pitfalls (type identity, name mangling, cross-module inference) identified from direct codebase analysis. Prevention strategies grounded in existing code patterns. |

## Gaps to Address

- **In-file module blocks (`module Foo do ... end`) semantics:** Currently ignored by typeck. How these interact with file-based modules needs design -- recommended: keep ignoring for v1.8, define semantics in future milestone.
- **Trait coherence / orphan rules:** Basic duplicate detection needed when merging TraitRegistries from multiple modules. Full orphan rules can be deferred.
- **LSP multi-file support:** The LSP server currently handles single files. Multi-file awareness is needed for go-to-definition across modules, but can follow core compilation support.
- **REPL module support:** The REPL currently evaluates single expressions. Module imports in the REPL need design work (defer to future).
- **Incremental compilation:** Not in scope. Full recompilation on every change is acceptable for v1.8 project sizes.

---
*Research completed: 2026-02-09*
*Ready for roadmap: yes*
