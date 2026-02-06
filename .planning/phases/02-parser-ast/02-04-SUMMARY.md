---
phase: 02-parser-ast
plan: 04
subsystem: parser
tags: [declarations, patterns, types, fn, module, import, struct, visibility]
depends_on:
  requires: ["02-03"]
  provides: ["declaration-parsing", "pattern-parsing", "type-annotations", "parse-api", "source-file-entry"]
  affects: ["02-05"]
tech-stack:
  added: []
  patterns: ["item-dispatch", "pattern-ADT", "visibility-node", "type-annotation-node"]
key-files:
  created:
    - crates/snow-parser/src/parser/items.rs
    - crates/snow-parser/src/parser/patterns.rs
  modified:
    - crates/snow-parser/src/parser/mod.rs
    - crates/snow-parser/src/parser/expressions.rs
    - crates/snow-parser/src/lib.rs
    - crates/snow-parser/tests/parser_tests.rs
decisions:
  - "fn/def followed by IDENT = named function definition; fn followed by L_PAREN = closure expression"
  - "from is contextual (IDENT with text check), not a keyword"
  - "Glob imports (from M import *) produce error at parse time, not later"
  - "Patterns in match arms replace expression-based approach from 02-03"
  - "Let bindings support tuple destructuring via pattern parsing"
  - "Type annotations use shared parse_type() for params, lets, return types, and struct fields"
metrics:
  duration: 4min
  completed: 2026-02-06
---

# Phase 2 Plan 4: Declarations and Patterns Summary

Parser handles all Snow declaration forms (fn, module, import, struct) with visibility, plus pattern matching syntax and structural type annotations. Public parse() API implemented.

## Task Commits

| Task | Name | Commit | Key Changes |
|------|------|--------|-------------|
| 1 | Implement item parsers, patterns, types | e595a0b | items.rs, patterns.rs, parse_source_file, parse(), pattern integration |
| 2 | Add snapshot tests | 124066c | 24 new snapshot tests covering all declaration and pattern forms |

## What Was Built

### Item Parsers (items.rs)
- **parse_fn_def**: `[pub] fn|def name(params) [-> ReturnType] do body end` with VISIBILITY, NAME, PARAM_LIST, optional TYPE_ANNOTATION, BLOCK
- **parse_module_def**: `[pub] module Name do items end` with nested items support
- **parse_import_decl**: `import Module.Path` with dot-separated PATH node
- **parse_from_import_decl**: `from Module import name1, name2` with IMPORT_LIST; rejects `*` glob
- **parse_struct_def**: `[pub] struct Name[TypeParams] do field :: Type ... end` with STRUCT_FIELD, TYPE_PARAM_LIST
- **parse_type**: `Ident[A, B]` or `Mod.Type` with optional generics

### Pattern Parser (patterns.rs)
- **WILDCARD_PAT**: `_` (detected via IDENT text check since lexer emits `_` as Ident)
- **LITERAL_PAT**: int, float, bool, nil, string, negative numbers (`-42`)
- **IDENT_PAT**: identifier binding
- **TUPLE_PAT**: `(p1, p2, ...)` with nested patterns

### Top-Level Entry (mod.rs)
- **parse_source_file**: Loops parse_item_or_stmt until EOF, root SOURCE_FILE node
- **parse_item_or_stmt**: Dispatches pub/fn/def/module/import/from/struct/let/return or falls through to expression

### Public API (lib.rs)
- **parse(source)**: Full source file parsing using parse_source_file entry point
- Replaces the prior `todo!()` stub

### Integration Changes
- Match arms now use proper pattern nodes (LITERAL_PAT, IDENT_PAT, etc.) instead of expression nodes
- Let bindings support tuple destructuring patterns
- Block bodies dispatch to parse_item_or_stmt (items can appear in blocks)
- Type annotations use shared parse_type() everywhere

## Decisions Made

1. **fn/def disambiguation**: `fn IDENT` = named fn def, `fn (` = closure. This handles the ambiguity cleanly since named functions must have a name.
2. **from as contextual identifier**: `from` is not a keyword in Snow -- it's recognized by checking `current_text() == "from"` when the token is IDENT.
3. **Glob rejection at parse time**: `from Module import *` emits an error immediately rather than deferring to semantic analysis.
4. **Pattern nodes replace expressions in match arms**: 02-03 used expression nodes as placeholders; now proper LITERAL_PAT, IDENT_PAT, TUPLE_PAT, WILDCARD_PAT nodes are used.
5. **Let tuple destructuring**: `let (a, b) = pair` produces TUPLE_PAT with IDENT_PAT children.
6. **Shared parse_type()**: All type annotation positions (params, let bindings, return types, struct fields) use the same parse_type function from items.rs.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Updated 4 existing snapshot tests**
- **Found during:** Task 1
- **Issue:** Switching from expression-based patterns to proper pattern nodes changed LITERAL and NAME_REF to LITERAL_PAT and IDENT_PAT in case arms
- **Fix:** Accepted the new snapshots since they reflect the correct behavior
- **Files modified:** 4 snapshot files (case_simple, case_with_when_guard, match_boolean, error_let_missing_ident)

**2. [Rule 2 - Missing Critical] Removed dead parse_stmt function**
- **Found during:** Task 1
- **Issue:** parse_stmt was replaced by parse_item_or_stmt; dead code would cause confusion
- **Fix:** Removed the unused function

## Test Coverage

82 total snapshot tests (58 existing + 24 new):
- Function definitions: 5 tests (simple, pub, typed, def keyword, no params)
- Modules: 2 tests (simple, nested)
- Imports: 3 tests (simple, dotted path, from-import)
- Structs: 2 tests (simple, pub with generics)
- Patterns: 5 tests (literal, wildcard, tuple, negative literal, string)
- Let destructuring: 1 test
- Full programs: 3 tests (module+fns, imports+pipes, struct+fn)
- Error cases: 2 tests (fn missing name, glob import)
- parse() API: 2 tests (expression, let binding)

## Next Phase Readiness

Plan 02-05 (Error Recovery and Public API) can proceed. All declaration forms, patterns, and the parse() entry point are complete. The parser now covers all Phase 2 Snow grammar constructs.

## Self-Check: PASSED
