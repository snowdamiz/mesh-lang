# Phase 11: Multi-Clause Functions - Context

**Gathered:** 2026-02-07
**Status:** Ready for planning

<domain>
## Phase Boundary

Define functions with multiple pattern-matched clauses instead of wrapping everything in case expressions. Includes guard clauses, exhaustiveness checking, and return type unification across clauses. This covers the syntax, semantics, and compiler support for multi-clause functions — not new pattern types or new matching capabilities.

</domain>

<decisions>
## Implementation Decisions

### Clause syntax
- Guard clauses supported with `when` keyword (e.g., `fn abs(n) when n < 0 = -n`)
- Guard expressions can be arbitrary expressions that return Bool, including function calls — not limited to simple comparisons

### Claude's Discretion: Clause syntax
- Whether to support both `= expr` (single-expression) and `do/end` (block body) forms, or one form only
- How clauses are grouped syntactically (consecutive standalone declarations vs single block)

### Clause ordering
- First-match wins — clauses tried top-to-bottom, first matching clause executes
- Wildcard/catch-all clause must be last — compiler error if a catch-all appears before other clauses
- Different arities are separate functions — `fn foo(x)` and `fn foo(x, y)` are `foo/1` and `foo/2`, not conflicting clauses

### Edge cases
- All parameters in a multi-parameter function support patterns (not just the first)

### Claude's Discretion: Error messages
- Exhaustiveness warning format and detail level
- Whether unreachable clauses produce a warning or error
- Return type mismatch error verbosity
- Runtime behavior when no clause matches (panic vs error tuple)

### Claude's Discretion: Edge cases
- Whether zero-arg multi-clause functions are supported (only meaningful with guards)
- Where multi-clause functions are valid (top level only vs everywhere functions work)
- Whether single-clause and multi-clause are distinct or seamlessly unified

</decisions>

<specifics>
## Specific Ideas

- Syntax example from roadmap: `fn fib(0) = 0`, `fn fib(1) = 1`, `fn fib(n) = fib(n-1) + fib(n-2)`
- Guards follow Elixir/Erlang style `when` keyword but allow arbitrary Bool expressions (not restricted to a guard subset)

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 11-multi-clause-functions*
*Context gathered: 2026-02-07*
