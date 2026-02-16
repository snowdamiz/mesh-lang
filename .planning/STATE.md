# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-16)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural and clean as writing sequential code, with the safety net of supervision and fault tolerance built into the language.
**Current focus:** v10.0 ORM -- Phase 97 (Schema Metadata + SQL Generation)

## Current Position

Phase: 97 of 102 (Schema Metadata + SQL Generation)
Plan: 1 of 2 in current phase
Status: Executing
Last activity: 2026-02-16 -- Completed 97-01 (Enhanced Schema Metadata codegen)

Progress: [████░░░░░░] 30% (6/20 plans)

## Performance Metrics

**All-time Totals:**
- Plans completed: 285
- Phases completed: 102
- Milestones shipped: 19 (v1.0-v9.0)
- Lines of Rust: ~98,800
- Lines of website: ~5,500
- Lines of Mesh: ~4020
- Timeline: 11 days (2026-02-05 -> 2026-02-15)

## Accumulated Context

### Decisions

- 96-01: Atoms lower to MirExpr::StringLit (string constants at LLVM level) -- no MirType::Atom needed. Type distinction is purely compile-time.
- 96-01: Atom lexing requires lowercase/underscore after colon to avoid ColonColon ambiguity.
- 96-01: ATOM_EXPR composite node wraps ATOM_LITERAL leaf token (follows LITERAL pattern).
- 96-02: Keyword args reuse MAP_LITERAL/MAP_ENTRY nodes with is_keyword_entry() detection via COLON vs FAT_ARROW.
- 96-02: Multi-line pipe continuation uses peek_past_newlines() in Pratt loop (not lexer-level newline suppression).
- 96-02: Keyword entry keys are NAME_REF nodes; typeck returns String type, MIR lowerer converts to StringLit.
- 96-03: Struct update %{base | field: value} disambiguated from map literal via parse-then-check (BAR vs FAT_ARROW after first expr).
- 96-03: Struct update reuses STRUCT_LITERAL_FIELD nodes; codegen copies base fields then overwrites specified overrides (value semantics).
- 96-04: Naive pluralization (lowercase + s) for table names; Phase 97 handles configurable table names.
- 96-04: Relationship metadata encoded as "kind:name:target" strings in List<String> to avoid complex map MIR.
- 96-04: Schema metadata functions use StructName.__method__() static syntax, same pattern as from_row/from_json.
- 96-04: Default primary key is always "id"; Phase 97 adds schema options for override.
- 96-04: Schema derive rejected on sum types with UnsupportedDerive error.
- 96-05: Pipe chain AST backward walk for string key detection (HM generalization severs type vars through Ptr bottleneck).
- 96-05: Separate mesh_map_collect_string_keys runtime function; codegen selects based on compile-time pipe chain analysis.
- 96-05: Cross-module __json_decode__ wrappers pre-generated before lower_source_file; ToJson/FromRow registered in known_functions.
- 96-05: collect_exports now exports deriving-generated trait impls (not just explicit ImplDef AST nodes).
- 97-01: Schema options use contextual identifiers (table/primary_key/timestamps) not @ annotations.
- 97-01: Column accessors use __field_col__ double-underscore pattern matching existing convention.
- 97-01: Timestamps inject inserted_at/updated_at as String fields into MirStructDef layout.
- 97-01: MirType -> SQL type mapping: Int->BIGINT, Float->DOUBLE PRECISION, Bool->BOOLEAN, String->TEXT.

### Roadmap Evolution

v10.0 ORM roadmap created 2026-02-16. 7 phases (96-102), 50 requirements across 7 categories. Research-recommended 7-phase structure adopted with strict dependency ordering: compiler additions first, then schema metadata, then query builder + repo, then changesets and relationships (parallel-capable), then migrations, finally Mesher rewrite validation.

### Pending Todos

None.

### Blockers/Concerns

Known limitations relevant to ORM development:
- ~~Map.collect integer key assumption~~ -- FIXED in 96-05 (COMP-07: pipe chain AST analysis + string key collect variant)
- ~~Single-line pipe chains only~~ -- FIXED in 96-02 (multi-line pipe continuation)
- ~~Cross-module from_row/from_json resolution edge cases~~ -- FIXED in 96-05 (COMP-08: BUILTIN_PREFIXES + cross-module wrapper pre-generation)

### Quick Tasks Completed

| # | Description | Date | Commit | Directory |
|---|-------------|------|--------|-----------|
| 1 | Rename project from Snow to Mesh, change .snow file extension to .mpl | 2026-02-13 | 3fe109e1 | [1-rename-project-from-snow-to-mesh-change-](./quick/1-rename-project-from-snow-to-mesh-change-/) |
| 2 | Write article: How Opus 4.6 and I Built a Production-Ready Programming Language in 9 Days | 2026-02-13 | (current) | [2-mesh-story-article](./quick/2-mesh-story-article/) |
| 3 | Validate codegen bug fixes (LLVM type coercion for service args, returns, actor messages) | 2026-02-15 | 7f429957 | [3-ensure-all-tests-still-pass-after-applyi](./quick/3-ensure-all-tests-still-pass-after-applyi/) |
| 4 | Build mesher and fix existing warnings (353 MIR false-positives + 15 Rust warnings) | 2026-02-15 | 2101b179 | [4-build-mesher-and-fix-existing-warnings-e](./quick/4-build-mesher-and-fix-existing-warnings-e/) |

## Session Continuity

Last session: 2026-02-16
Stopped at: Completed 97-01-PLAN.md (Enhanced Schema Metadata codegen)
Resume file: None
Next action: Execute 97-02-PLAN.md (Runtime SQL Generation)
