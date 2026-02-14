# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-13)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural and clean as writing sequential code, with the safety net of supervision and fault tolerance built into the language.
**Current focus:** v7.0 Phase 76 - Iterator Protocol

## Current Position

Phase: 76 of 79 (Iterator Protocol)
Plan: 0 of TBD in current phase
Status: Ready to plan
Last activity: 2026-02-13 -- Phase 75 (Numeric Traits) complete, verified 11/11 must-haves

Progress: [██░░░░░░░░] 33% (v7.0)

## Performance Metrics

**All-time Totals:**
- Plans completed: 206
- Phases completed: 75
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

Last session: 2026-02-13
Stopped at: Phase 75 (Numeric Traits) complete, verified passed
Resume file: None
Next action: `/gsd:plan-phase 76` (Iterator Protocol)
