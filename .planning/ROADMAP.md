# Roadmap: Snow

## Milestones

- ✅ **v1.0 MVP** - Phases 1-10 (shipped 2026-02-07)
- 🚧 **v1.1 Language Polish** - Phases 11-15 (in progress)

## Phases

<details>
<summary>✅ v1.0 MVP (Phases 1-10) - SHIPPED 2026-02-07</summary>

55 plans across 10 phases. Full compiler pipeline, actor runtime, supervision trees,
standard library, and developer tooling. See milestones/v1.0-ROADMAP.md for details.

</details>

### 🚧 v1.1 Language Polish (In Progress)

**Milestone Goal:** Fix all five documented v1.0 limitations -- multi-clause functions, string pattern matching, pipe operator with closures, actor-per-connection HTTP, and generic map types -- to make the language feel complete and polished.

- [x] **Phase 11: Multi-Clause Functions** - Define functions with multiple pattern-matched clauses
- [x] **Phase 12: Pipe Operator Closures** - Parse inline closures inside pipe chains
- [ ] **Phase 13: String Pattern Matching** - Compile-time string comparison in case expressions
- [ ] **Phase 14: Generic Map Types** - Map<K, V> with string keys and generic functions
- [ ] **Phase 15: HTTP Actor Model** - Actor-per-connection HTTP server with supervision

## Phase Details

### Phase 11: Multi-Clause Functions
**Goal**: Users can define functions with multiple pattern-matched clauses instead of wrapping everything in case expressions
**Depends on**: v1.0 complete
**Requirements**: SYN-01, SYN-02, SYN-03
**Success Criteria** (what must be TRUE):
  1. User can write `fn fib(0) = 0`, `fn fib(1) = 1`, `fn fib(n) = fib(n-1) + fib(n-2)` and it compiles and runs correctly
  2. Compiler raises an exhaustiveness warning when multi-clause function does not cover all cases (e.g., missing wildcard clause for an Int-typed parameter)
  3. Type inference correctly unifies the return type across all clauses -- a function with clauses returning Int and String is a type error
  4. Multi-clause functions work with all existing pattern types (literals, variables, wildcards, constructors)
**Plans**: 3 plans

Plans:
- [x] 11-01-PLAN.md -- Parser + AST: = expr body form, pattern params, guard clauses
- [x] 11-02-PLAN.md -- Type checker: clause grouping, desugaring, validation, guard relaxation
- [x] 11-03-PLAN.md -- MIR lowering, formatter, e2e tests

### Phase 12: Pipe Operator Closures
**Goal**: Users can pipe values into expressions containing inline closures without parser errors
**Depends on**: v1.0 complete
**Requirements**: SYN-04, SYN-05
**Success Criteria** (what must be TRUE):
  1. `list |> Enum.map(fn x -> x * 2 end)` parses and executes correctly
  2. Nested `do/end` blocks inside pipe chains parse correctly (e.g., `conn |> handle(fn req -> if req.valid do ... end end)`)
  3. Multiple chained pipes with closures work (e.g., `list |> Enum.map(fn x -> x + 1 end) |> Enum.filter(fn x -> x > 3 end)`)
**Plans**: 3 plans

Plans:
- [x] 12-01-PLAN.md -- Parser rewrite: bare params, do/end body, multi-clause, guards, error messages, snapshot tests
- [x] 12-02-PLAN.md -- Formatter, type checker/MIR multi-clause support, e2e tests
- [x] 12-03-PLAN.md -- Gap closure: pipe-aware call inference in type checker, pipe+closure e2e tests

### Phase 13: String Pattern Matching
**Goal**: Users can match on string literals in case expressions with compile-time generated code instead of runtime fallback
**Depends on**: v1.0 complete
**Requirements**: PAT-01, PAT-02
**Success Criteria** (what must be TRUE):
  1. `case name do "alice" -> ... "bob" -> ... end` compiles to direct string comparison (no runtime dispatch overhead)
  2. Compiler requires a wildcard/default clause when string match is non-exhaustive (strings are an open set)
  3. String patterns can be mixed with variable bindings in the same case expression (e.g., `"hello" -> ...` alongside `other -> ...`)
**Plans**: TBD

Plans:
- [ ] 13-01: TBD

### Phase 14: Generic Map Types
**Goal**: Map type supports generic key/value types so users can build maps with string keys and any value type
**Depends on**: v1.0 complete
**Requirements**: MAP-01, MAP-02, MAP-03
**Success Criteria** (what must be TRUE):
  1. `Map<String, Int>` and `Map<String, String>` are valid types -- Map is no longer hardcoded to `Map<Int, Int>`
  2. `Map.put(m, "name", "Alice")` and `Map.get(m, "name")` compile and work correctly with string keys
  3. Map literal syntax `%{"name" => "Alice", "age" => 30}` parses and type-checks correctly
  4. Type inference correctly infers Map generic parameters from usage (user rarely needs to annotate Map types)
**Plans**: TBD

Plans:
- [ ] 14-01: TBD
- [ ] 14-02: TBD

### Phase 15: HTTP Actor Model
**Goal**: HTTP server uses lightweight actor processes per connection instead of OS threads, with crash isolation per connection
**Depends on**: v1.0 complete
**Requirements**: HTTP-01, HTTP-02
**Success Criteria** (what must be TRUE):
  1. HTTP server spawns a lightweight actor (not OS thread) for each incoming connection
  2. A crash in one connection handler does not affect other active connections (actor isolation)
  3. A Snow HTTP server program that worked under v1.0 thread model continues to work with the actor model (backward-compatible API)
**Plans**: TBD

Plans:
- [ ] 15-01: TBD

## Progress

**Execution Order:**
Phases execute in numeric order: 11 -> 12 -> 13 -> 14 -> 15

| Phase | Milestone | Plans Complete | Status | Completed |
|-------|-----------|----------------|--------|-----------|
| 11. Multi-Clause Functions | v1.1 | 3/3 | ✓ Complete | 2026-02-07 |
| 12. Pipe Operator Closures | v1.1 | 3/3 | ✓ Complete | 2026-02-07 |
| 13. String Pattern Matching | v1.1 | 0/TBD | Not started | - |
| 14. Generic Map Types | v1.1 | 0/TBD | Not started | - |
| 15. HTTP Actor Model | v1.1 | 0/TBD | Not started | - |
