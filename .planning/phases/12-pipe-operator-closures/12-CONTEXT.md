# Phase 12: Pipe Operator Closures - Context

**Gathered:** 2026-02-07
**Status:** Ready for planning

<domain>
## Phase Boundary

Parse inline closures inside pipe chains so that `list |> Enum.map(fn x -> x * 2 end)` and similar expressions compile and execute correctly. This phase fixes a parser limitation — closures already work in Snow, but fail when used as arguments inside pipe chain calls.

</domain>

<decisions>
## Implementation Decisions

### Closure syntax
- Use `fn params -> body end` syntax (Elixir-style)
- Support multi-clause closures with `|` separator: `fn 0 -> "zero" | n -> to_string(n) end`
- Full pattern matching in closure parameters (destructuring tuples, structs, etc.)
- Two body forms: `fn x -> expr end` for one-liners, `fn x do ... end` for multi-line blocks

### Nesting behavior
- No limit on nesting depth — arbitrary closure nesting inside pipe chains is legal
- Balanced matching for `end` tokens — each `fn`/`do` gets its own `end`, parser counts nesting depth
- `do/end` blocks inside closures support full statements (let, case, if, etc.) — same as named function bodies

### Error messages
- Missing `end` errors point back to the opening `fn` token: "unclosed closure starting at line X — expected `end`"
- Bail on first closure parse error in a pipe chain (no recovery, user fixes one at a time)
- When user writes bare closure as pipe target (`|> fn x -> x end`), suggest the fix: "unexpected closure as pipe target — did you mean `|> Func(fn x -> x end)`?"

### Edge cases
- Closures support any number of parameters (comma-separated): `fn acc, x -> acc + x end`
- Guard clauses supported in closure clauses: `fn x when x > 0 -> x | x -> -x end`
- Pipe operator precedence follows Elixir's rules — pipe has low precedence, closure body extends to `end`

### Claude's Discretion
- Whether pipes are allowed inside closure bodies that are themselves pipe arguments (e.g., `|> Enum.map(fn x -> x |> transform() end)`) — likely yes for consistency
- Whether closures can be the source (left-hand side) of a pipe chain
- Terminology in error messages ("closure" vs "anonymous function")

</decisions>

<specifics>
## Specific Ideas

- Follow Elixir's precedence model for pipe vs closure interaction
- Multi-clause closures mirror Phase 11's multi-clause named function design — reuse the same pattern matching and guard infrastructure
- `do/end` block form for closures parallels `do/end` for named functions — consistency matters

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 12-pipe-operator-closures*
*Context gathered: 2026-02-07*
