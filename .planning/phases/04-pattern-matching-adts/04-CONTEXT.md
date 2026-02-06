# Phase 4: Pattern Matching & Algebraic Data Types - Context

**Gathered:** 2026-02-06
**Status:** Ready for planning

<domain>
## Phase Boundary

Exhaustive pattern matching compilation with algebraic data types (sum types), guards, and compile-time warnings for missing or redundant patterns. Sum types can be defined, constructed, and destructured. The compiler enforces exhaustiveness as a hard error and warns on redundant arms. Guards use restricted expressions (no user functions). Multi-clause function definitions with pattern matching are included.

</domain>

<decisions>
## Implementation Decisions

### Sum type syntax
- `type` keyword with `do/end` block (Elixir-style): `type Shape do Circle(Float) ... end`
- Variants can have named fields: `Rectangle(width: Float, height: Float)`
- Sum types support generic type parameters: `type Option<T> do Some(T) None end` — replaces compiler builtins
- Variants constructed via qualified access: `Shape.Circle(5.0)` — no global variant names

### Pattern syntax & nesting
- Or-patterns supported with pipe syntax: `Circle(_) | Point -> ...`
- As-patterns supported with `as` keyword: `Circle(_) as c -> use_circle(c)`
- Arbitrary nesting depth for destructuring: `Some(Circle(r)) -> r`
- Named fields destructure by name: `Rectangle(w: w, h: _) -> w`
- Pattern matching works in both case/match blocks AND function heads (multi-clause functions with exhaustiveness checking across clauses)

### Guard behavior
- Restricted guard expressions: comparisons (`>`, `<`, `==`), boolean ops (`and`/`or`/`not`), and specific builtin functions (no user-defined functions in guards) — Erlang/Elixir style
- Guards CAN reference bindings from the pattern: `Circle(r) when r > 0.0 -> ...`
- Guards do NOT count toward exhaustiveness — a guarded arm is treated as potentially non-matching, so a fallback is required
- Consistent `when` keyword in both match arms and function heads

### Compiler diagnostics
- Non-exhaustive match is a **hard error** (won't compile) — Rust approach
- Redundant/unreachable pattern arm is a **warning** (compiles, dead code flagged)
- Missing pattern errors list missing variants explicitly: "Non-exhaustive match: missing Circle(_, _), Point"

### Claude's Discretion
- Fix suggestion style for non-exhaustive errors (suggest explicit arms, wildcard, or both)
- Exact exhaustiveness algorithm implementation details (Maranget's or variation)
- Internal representation of sum types and pattern compilation

</decisions>

<specifics>
## Specific Ideas

- Generic sum types should replace the current compiler-builtin Option/Result from Phase 3 — they become user-definable ADTs
- Guard restriction set mirrors Erlang/Elixir's approach: keep guards simple and predictable, no side effects
- Multi-clause functions feel like Elixir's function head pattern matching — exhaustiveness checked across all clauses of the same function

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 04-pattern-matching-adts*
*Context gathered: 2026-02-06*
