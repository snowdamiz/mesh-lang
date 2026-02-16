# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-16)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural and clean as writing sequential code, with the safety net of supervision and fault tolerance built into the language.
**Current focus:** v10.0 ORM -- Phase 101 (Migration System)

## Current Position

Phase: 101 of 102 (Migration System) -- COMPLETE
Plan: 3 of 3 in current phase (all plans complete)
Status: Phase complete, verified PASSED
Last activity: 2026-02-16 -- Phase 101 complete (Migration System)

Progress: [█████████░] 85% (17/20 plans)

## Performance Metrics

**All-time Totals:**
- Plans completed: 297
- Phases completed: 106
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
- 97-02: Pure Rust SQL helpers separated from extern C wrappers for unit testability without GC.
- 97-02: WHERE clause format: 'column op' space-separated (e.g. 'name =', 'age >', 'status IS NULL').
- 97-02: IS NULL/IS NOT NULL WHERE clauses do not consume parameter slots.
- 97-02: UPDATE parameter numbering: SET columns $1..$N, WHERE clauses continue from $N+1.
- 98-01: Query type signatures use Atom type for field/operator parameters (atoms are typeck-distinct, lower to StringLit at MIR).
- 98-01: Ptr and Atom type constructors added to resolve_con (Ptr->MirType::Ptr, Atom->MirType::String).
- 98-01: Parser accepts WHERE_KW after dot for Query.where() field access syntax.
- 98-01: Schema pipe transform deferred: explicit Query.from(User.__table__()) used instead of implicit User |> Query.where().
- 98-02: Repo functions use PoolHandle (MirType::Int / i64) for pool parameter, matching existing Pool.query pattern.
- 98-02: SQL builder separated into pure Rust functions for unit testability without GC.
- 98-02: Repo.count uses SELECT COUNT(*) with 'count' column key extraction; Repo.exists uses SELECT EXISTS(SELECT 1 ... LIMIT 1).
- 98-03: Map fields extracted via direct internal structure access (header + entries array) for Repo.insert/update.
- 98-03: ORM SQL builders exposed as pub(crate) wrappers from orm.rs for cross-module reuse by repo.rs.
- 98-03: Repo.transaction takes pool (not conn), manages full checkout/begin/callback/commit-or-rollback/checkin lifecycle.
- 98-03: All write operations use RETURNING * -- insert/update/delete return the affected row as Map<String,String>.
- 99-01: Changeset uses 8-slot/64-byte GC-allocated layout (data, changes, errors, valid, field_types, table, pk, action), matching Query's slot-based pattern.
- 99-01: Validators follow clone-check-error-return pattern: each clones the changeset so all validators run without short-circuiting.
- 99-01: First error per field wins; subsequent validators skip fields with existing errors.
- 99-01: Typeck signatures use concrete types (Map<String,String>, List<Atom>) for user-facing params; Ptr only for opaque changeset returns.
- 99-01: Parser allows CAST_KW after dot in field access (Changeset.cast) since cast is both a service keyword and a method name.
- 99-02: PG errors use tab-separated structured format (sqlstate\tconstraint\ttable\tcolumn\tmessage) for constraint mapping.
- 99-02: Constraint names parsed via PostgreSQL convention: {table}_{column}_{key|fkey|pkey|check} for field extraction.
- 99-02: Invalid changesets short-circuit before SQL execution, returning Err(changeset) immediately.
- 99-02: Unmapped PG errors get generic _base error on changeset rather than losing error information.
- 100-01: __relationship_meta__() returns 5-field "kind:name:target:fk:target_table" encoding for preloader consumption.
- 100-01: FK convention: has_many/has_one uses {owner_lowercase}_id, belongs_to uses {assoc_name}_id (Ecto convention).
- 100-01: Target table inferred by naive pluralization {target_lowercase}s, consistent with __table__() convention.
- 100-02: Repo.preload uses separate WHERE fk IN (...) query per association level (not JOINs), matching Ecto's design.
- 100-02: Nested preloading uses positional tracking (parent_idx, pos_in_list) for re-stitching enriched rows.
- 101-01: Migration DDL builders follow exact same pattern as orm.rs: pure Rust helpers + extern C wrappers.
- 101-01: Column definitions use colon-separated encoding (name:TYPE:CONSTRAINTS) for create_table and add_column.
- 101-01: Index names auto-generated as idx_table_col1_col2 convention.
- 101-01: All 8 Migration functions return Result<Int, String> matching Repo pattern.
- 101-03: Howard Hinnant civil_from_days algorithm for chrono-free UTC timestamp formatting (no external deps).
- 101-03: Migration name validation: lowercase ASCII + digits + underscores only.
- 101-03: Scaffold template includes documented Migration DSL examples as comments in up/down stubs.
- 101-02: Native Rust PG API (NativePgConn) added to mesh-rt for GC-free database access from meshc.
- 101-02: Synthetic Mesh compilation used only for migration execution (up/down), not tracking table operations.
- 101-02: Migration tracking via _mesh_migrations table (version BIGINT PK, name TEXT, applied_at TIMESTAMPTZ).

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
Stopped at: Phase 101 complete (Migration System) -- verified PASSED
Resume file: None
Next action: `/gsd:plan-phase 102` (Mesher Rewrite)
