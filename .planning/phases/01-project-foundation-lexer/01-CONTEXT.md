# Phase 1: Project Foundation & Lexer - Context

**Gathered:** 2026-02-06
**Status:** Ready for planning

<domain>
## Phase Boundary

Reproducible Rust workspace with pinned LLVM 18, snapshot test infrastructure (insta), and a hand-written lexer that tokenizes all Snow syntax with accurate span tracking (line, column, byte offset). The lexer produces a complete token stream covering keywords, operators, literals, identifiers, comments, and string interpolation markers. Error recovery ensures the lexer can continue past invalid tokens.

</domain>

<decisions>
## Implementation Decisions

### Token vocabulary
- Elixir-inspired keywords adapted for static typing: `fn`, `let`, `struct`, `trait`, `module`, `import`, `if`/`else`, `case`/`match`, `do`/`end`, `receive`, `spawn`, etc.
- Full operator set: arithmetic (+, -, *, /), comparison (==, !=, <, >, <=, >=), logical (&&, ||, !), pipe (|>), range (..), string concat (<>), list concat (++), match (=), arrow (->), fat arrow (=>), type annotation (::)
- Reserve all planned keywords upfront (including future-phase keywords like `spawn`, `receive`, `supervisor`, `send`, `link`, `monitor`, `trap`, `cond`, `with`) to prevent identifier conflicts in later phases
- Atoms vs enums: Claude's discretion — decide based on type system and actor messaging compatibility

### String & interpolation syntax
- Double-quoted strings ("hello") for single-line
- Triple-quoted strings ("""multi\nline""") for multi-line
- No single-quoted strings
- Interpolation syntax: `${expr}` (JS/Kotlin style)
- Both double-quoted and triple-quoted strings support interpolation
- Arbitrary expressions allowed inside `${}` (not just identifiers)
- Lexer emits interpolation as a sequence of string-part and expression tokens

### Comment & whitespace rules
- Line comments: `#`
- Block comments: `#= ... =#` (nestable, Julia-style)
- Doc comment syntax: Claude's discretion — pick approach that fits Snow's syntax
- Whitespace is NOT significant — blocks use `do`/`end` delimiters
- Newlines are statement terminators (like Elixir/Go)
- Multi-line expressions continue across newlines when inside parens, brackets, or after operators

### Numeric literals
- Integer formats: decimal (42), hex (0xFF), binary (0b1010), octal (0o777)
- Underscore separators allowed: 1_000_000, 0xFF_FF, 0b1111_0000
- Float syntax: basic (3.14) and scientific notation (1.0e10, 2.5e-3)
- Distinct Int vs Float types: Claude's discretion — decide based on type system implications

### Claude's Discretion
- Atom syntax decision (atoms vs enum-only approach)
- Doc comment syntax choice
- Distinct Int/Float types vs single Number type
- Exact keyword list (the full set beyond what was discussed)
- Error recovery strategy details
- Token representation internals

</decisions>

<specifics>
## Specific Ideas

- Elixir heritage is the north star — syntax should feel familiar to Elixir/Ruby developers
- `${expr}` interpolation over `#{expr}` — deliberate departure from Elixir here for JS/Kotlin familiarity
- `#= =#` block comments chosen to stay in the `#` comment family while being nestable
- Reserved keywords should cover all 10 phases worth of planned syntax to avoid breaking changes

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 01-project-foundation-lexer*
*Context gathered: 2026-02-06*
