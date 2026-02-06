# Phase 2: Parser & AST - Context

**Gathered:** 2026-02-06
**Status:** Ready for planning

<domain>
## Phase Boundary

Recursive descent parser that transforms the Phase 1 token stream into a complete AST representing all Snow language constructs. Includes lossless syntax tree for future tooling. Type checking, exhaustiveness checking, and codegen are separate phases.

</domain>

<decisions>
## Implementation Decisions

### AST representation
- Lossless CST + AST approach: concrete syntax tree preserves all tokens (whitespace, comments, parens) and a typed AST is derived from the CST
- CST library and arena vs tree allocation: Claude's Discretion
- Expression vs statement distinction: Claude's Discretion (choose based on Snow's Elixir-inspired "everything returns a value" semantics and what's cleanest for type checking)

### Syntax resolution
- Newlines are significant: a newline terminates an expression unless continuation is obvious (open paren, trailing operator, pipe, comma)
- Parentheses always required for function calls: `greet("world")` not `greet "world"`
- Pipe operator `|>` passes value as first argument (Elixir-style): `x |> foo(y)` becomes `foo(x, y)`
- Trailing closures supported: `list.map() do |x| ... end` — block passed after closing paren

### Error recovery
- Report first error only — stop parsing at the first error, no recovery/synchronization
- This simplifies the parser significantly; no need for synchronization point logic
- Error messages should be a blend of Elm-friendly and Rust-precise: conversational sentence explaining the problem + precise source spans with underlines
- Unclosed delimiters always reference where they were opened: "Expected `end` to close `do` block started at line 5, column 3"
- No fix suggestions in Phase 2 — descriptive errors only, suggestion engine deferred to Phase 10 (Tooling)

### Module & visibility
- Module nesting strategy: Claude's Discretion (pick what fits Snow's Elixir-inspired syntax)
- Import style: `import Math` for whole module, `from Math import sqrt, pow` for selective imports
- No glob imports — `from Math import *` is not allowed, must always name imports explicitly
- Visibility default (pub vs priv): Claude's Discretion

### Claude's Discretion
- Arena allocation vs Box/Rc tree for CST/AST node storage
- Whether to use rowan or custom CST implementation
- Expression-only vs expression+statement AST node design
- Nested modules vs one-module-per-file
- Private-by-default+pub vs public-by-default+priv visibility

</decisions>

<specifics>
## Specific Ideas

- Error messages should feel like a conversation with a knowledgeable friend who also shows you exactly where the problem is — Elm's warmth with Rust's precision
- Trailing closure syntax inspired by Ruby/Swift — makes DSL-style code and callbacks read naturally
- `from X import y, z` chosen over Elixir/Rust styles for readability — Python-ish, immediately clear what's being imported
- No glob imports is a deliberate design choice for a compiled language — forces explicitness

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 02-parser-ast*
*Context gathered: 2026-02-06*
