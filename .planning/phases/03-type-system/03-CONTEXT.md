# Phase 3: Type System - Context

**Gathered:** 2026-02-06
**Status:** Ready for planning

<domain>
## Phase Boundary

Hindley-Milner type inference engine that type-checks Snow programs without requiring type annotations. Supports generics, structs, traits (as "interfaces"), Option/Result types. This phase delivers the inference engine and type-checking pass — pattern matching exhaustiveness and ADTs are Phase 4, codegen is Phase 5.

</domain>

<decisions>
## Implementation Decisions

### Type annotation syntax
- Angle brackets for generics: `List<T>`, `Option<Int>`, `Result<String, Error>`
- Function params always annotated, return type optional: `fn add(x: Int, y: Int) -> Int` or `fn add(x: Int, y: Int)`
- Return type uses arrow syntax: `-> Int`
- Option sugar: `Int?` means `Option<Int>`
- Result sugar: `T!E` means `Result<T, E>`

### Inference boundaries
- Full inference for function parameters — `fn add(x, y)` is valid, types inferred from usage
- Struct field types always annotated in the definition — `struct User do name: String, age: Int end`
- When inference fails or is ambiguous: hard error with suggestion of what annotation to add ("Cannot infer type of x. Try adding: x: Int")
- No numeric literal defaults — ambiguous numeric types are errors like everything else
- No wildcard type annotation (`_`) — either annotate with a real type or leave it off

### Error experience
- Elm-level thoroughness with minimal (Go-like) tone — concise messages, no conversational framing, but still show both sides of conflicts and suggest fixes
- Show endpoints only for inference chains — "expected Int, found String" with locations of both, no full inference trace
- Always suggest fixes when a plausible fix exists (Option wrapping, missing trait impl, type coercion, etc.)
- Error format: terse one-liner with labeled source spans, not paragraphs of explanation

### Trait & generic design
- `interface` keyword for trait definitions: `interface Printable do fn to_string(self) -> String end`
- Where clause for generic constraints: `fn foo<T>(x: T) where T: Printable` — no inline bounds
- Option and Result are fully built-in — compiler has deep awareness for sugar (Int?, T!E), optimizations, better error messages, and automatic propagation

### Claude's Discretion
- Implementation syntax for interfaces (impl block style, keyword choice)
- Internal type representation (ena-based union-find vs other approaches)
- Unification algorithm details
- How propagation operator (like Rust's `?`) looks syntactically — if included in this phase at all

</decisions>

<specifics>
## Specific Ideas

- Elm-quality diagnostics but Go-minimal tone — imagine Elm's structure (show conflict, suggest fix) delivered in Go's brevity (one line + spans, no prose)
- Full inference including function params is a strong commitment — the system should feel like "types are there but you never write them unless you want to"
- Option/Result as fully built-in means the compiler can give specialized, helpful errors for the most common type mistakes (nil access, unhandled errors)

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 03-type-system*
*Context gathered: 2026-02-06*
