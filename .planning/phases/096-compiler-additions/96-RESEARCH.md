# Phase 96: Compiler Additions - Research

**Researched:** 2026-02-16
**Domain:** Mesh compiler internals -- lexer, parser, type checker, MIR, LLVM codegen
**Confidence:** HIGH

## Summary

This phase adds five new language features (atoms, keyword arguments, multi-line pipes, struct update syntax, deriving(Schema)) and fixes two bugs (Map.collect string key propagation, cross-module from_row resolution) to the Mesh compiler. All changes are internal to the existing Rust codebase across the `mesh-lexer`, `mesh-parser`, `mesh-typeck`, `mesh-codegen`, and `mesh-common` crates. No external dependencies are needed.

The compiler pipeline is well-established across 95 shipped phases. Each feature follows the same flow: lexer token addition (mesh-common/token.rs + mesh-lexer), SyntaxKind mapping (mesh-parser/syntax_kind.rs), CST node kind addition (mesh-parser/syntax_kind.rs), parser grammar rules (mesh-parser/parser/expressions.rs or items.rs), AST accessor methods (mesh-parser/ast/), type checker handling (mesh-typeck/infer.rs), MIR lowering (mesh-codegen/mir/lower.rs), and LLVM codegen (mesh-codegen/codegen/expr.rs). The codebase has clear patterns for each of these layers, with extensive prior art from features like map literals (`%{}`), struct literals, deriving clauses, and pipe expressions.

The key architectural insight is that atoms compile to string constants at the LLVM level -- they are semantically equivalent to strings but carry a distinct `Atom` type in the type checker. This type distinction enables the ORM query builder to accept field references (`:name`, `:email`) as a typed parameter rather than raw strings, providing compile-time field name validation in later phases. Keyword arguments are a parser-level desugaring: `where(name: "Alice")` becomes `where(%{"name" => "Alice"})`. Multi-line pipe is a parser-level continuation rule. Struct update is a parser + typeck + codegen feature that creates a new struct with some fields overridden.

**Primary recommendation:** Implement features in dependency order: atoms first (lexer through codegen), then keyword args and multi-line pipes (parser-only with minor typeck), then struct update (parser + typeck + codegen), then deriving(Schema) (typeck + MIR + codegen), and finally bugfixes. Each feature should be fully end-to-end tested before moving to the next.

## Standard Stack

### Core

| Crate | Location | Purpose | Relevance |
|-------|----------|---------|-----------|
| mesh-common | crates/mesh-common | Token types, spans | Add `Atom` to TokenKind |
| mesh-lexer | crates/mesh-lexer | Tokenization | Lex `:name` atom literals |
| mesh-parser | crates/mesh-parser | CST/AST construction | Parse atoms, kwargs, multi-line pipe, struct update |
| mesh-typeck | crates/mesh-typeck | Type inference, trait dispatch | Atom type, keyword desugaring types, struct update validation, deriving(Schema) |
| mesh-codegen | crates/mesh-codegen | MIR lowering + LLVM IR | Atom codegen, struct update codegen, Schema metadata functions |
| mesh-rt | crates/mesh-rt | Runtime support | No changes expected this phase |

### Supporting

| Library | Version | Purpose | When Used |
|---------|---------|---------|-----------|
| rowan | 0.16 | CST green tree construction | All parser work |
| ena | 0.14 | Union-find for type inference | Type checker unification |
| inkwell | 0.8.0 (LLVM 21.1) | LLVM IR generation | Codegen for atoms, struct update |
| insta | 1.46 | Snapshot testing | Testing parser/typeck output |

## Architecture Patterns

### Compiler Pipeline (Established)

```
Source text
    --> Lexer (mesh-lexer) --> Vec<Token>
    --> Parser (mesh-parser) --> rowan GreenNode (CST)
    --> Typed AST wrappers (mesh-parser/ast/) --> zero-cost views over CST
    --> Type checker (mesh-typeck/infer.rs) --> TypeckResult (type map + errors)
    --> MIR lowering (mesh-codegen/mir/lower.rs) --> MirModule
    --> LLVM codegen (mesh-codegen/codegen/) --> LLVM IR --> native binary
```

### Pattern 1: Adding a New Token Kind

**What:** Every new syntax element requires additions across 4 files in a specific order.
**When to use:** Atoms (`:name` literals).

**Steps:**
1. Add variant to `TokenKind` in `crates/mesh-common/src/token.rs`
2. Update variant count in tests
3. Add corresponding `SyntaxKind` variant in `crates/mesh-parser/src/syntax_kind.rs`
4. Add `TokenKind -> SyntaxKind` mapping in the `From<TokenKind>` impl
5. Update variant count tests
6. Add lexer rule in `crates/mesh-lexer/src/lib.rs`
7. Add parser handling in expression/item parsers
8. Add AST accessor methods

**Example -- how map literals were added:**
```rust
// 1. TokenKind (already exists for %)
// In token.rs: Percent variant already present

// 2. In syntax_kind.rs: MAP_LITERAL, MAP_ENTRY node kinds already present

// 3. In parser/expressions.rs:
SyntaxKind::PERCENT => {
    if p.nth(1) == SyntaxKind::L_BRACE {
        Some(parse_map_literal(p))
    } else {
        p.error("expected expression");
        None
    }
}
```

### Pattern 2: Adding a New Composite CST Node

**What:** Parser produces a new node kind wrapping child tokens/nodes.
**When to use:** Struct update expression, atom literal, keyword argument.

**Steps:**
1. Add node kind to `SyntaxKind` enum (e.g., `STRUCT_UPDATE_EXPR`, `ATOM_LITERAL`)
2. Add AST wrapper struct in appropriate ast module using `ast_node!` macro
3. Implement accessor methods on the wrapper
4. Add variant to `Expr` or `Item` enum and update `cast()` method
5. Handle in type checker (`infer_expr` or `infer_item`)
6. Handle in MIR lowering (`lower_expr` or `lower_item`)
7. Handle in codegen (`codegen_expr`)

### Pattern 3: Deriving Trait Registration

**What:** `deriving(TraitName)` in struct/sum type bodies auto-registers trait implementations.
**When to use:** deriving(Schema) infrastructure.

**Existing pattern from deriving(Json, Row, Eq, etc.):**
```rust
// In infer.rs, inside the struct registration function:
let has_deriving = struct_def.has_deriving_clause();
let derive_list = struct_def.deriving_traits();

// Validate derive names
let valid_derives = ["Eq", "Ord", "Display", "Debug", "Hash", "Json", "Row"];
for trait_name in &derive_list {
    if !valid_derives.contains(&trait_name.as_str()) {
        ctx.errors.push(TypeError::UnknownDeriveTrait { ... });
    }
}

// Register trait impl
if derive_list.iter().any(|t| t == "Row") {
    let mut methods = FxHashMap::default();
    methods.insert("from_row".to_string(), ImplMethodSig { ... });
    trait_registry.register_impl(TraitImplDef { ... });
}
```

### Pattern 4: Parser Newline Handling

**What:** Mesh treats newlines as significant statement terminators unless inside delimiters.
**When to use:** Multi-line pipe chain support.

**Existing mechanism:**
```rust
// In parser/mod.rs:
fn is_newline_insignificant(&self) -> bool {
    self.paren_depth > 0 || self.bracket_depth > 0 || self.brace_depth > 0
}

fn should_skip(&self, kind: &TokenKind) -> bool {
    match kind {
        TokenKind::Newline => self.is_newline_insignificant(),
        _ => false,
    }
}
```

For multi-line pipes, the parser needs to check if a line starts with `|>` and treat the previous newline as a continuation rather than a statement terminator.

### Anti-Patterns to Avoid

- **Adding to MirType for atoms:** Atoms are string constants at runtime. Do NOT add a `MirType::Atom` variant. Use `MirType::String` with `MirExpr::StringLit` for codegen. The distinction is only in the type checker (`Ty::Con(TyCon::new("Atom"))`).
- **Modifying lexer state machine for keyword args:** Keyword arguments are a parser-level construct. The lexer already emits `Ident`, `Colon`, and the value tokens. The parser recognizes `ident: expr` inside argument lists and desugars to map entries.
- **Hand-rolling struct field copying for struct update:** The struct update codegen should allocate a new struct, copy all fields from the base, then overwrite the specified fields. Use the existing `codegen_struct_lit` pattern with field-by-field GEP stores.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Token/SyntaxKind mapping | Manual dispatch | `From<TokenKind> for SyntaxKind` impl | Exhaustive, tested, one canonical place |
| AST node boilerplate | Manual struct + cast | `ast_node!` macro | Zero-cost wrappers, consistent API |
| Type constructor | Manual string matching | `TyCon::new("Atom")`, `Ty::Con(...)` | Integrates with existing HM inference |
| Trait impl registration | Custom registry | `TraitRegistry::register_impl` | Already handles method dispatch, cross-module resolution |
| Struct field layout | Custom memory layout | `self.struct_types` map in codegen | Already handles field ordering, GEP indices |

**Key insight:** The compiler has 95 phases of accumulated infrastructure. Every new feature should use the established patterns and registries rather than creating parallel systems.

## Common Pitfalls

### Pitfall 1: Token Count Test Breakage

**What goes wrong:** Adding new TokenKind variants without updating the hardcoded count assertions in `token.rs::tests` and `syntax_kind.rs::tests`.
**Why it happens:** These tests explicitly assert variant counts (currently 96 for TokenKind) to catch accidental additions/removals.
**How to avoid:** When adding `Atom` to TokenKind, update the count in `token_kind_variant_count` test from 96 to 97 (or however many are added). Similarly update `all_token_kinds_convert_to_syntax_kind` test list and `syntax_kind_has_enough_variants` assertion.
**Warning signs:** Tests fail with "expected 96 variants, got 97".

### Pitfall 2: Newline Significance in Multi-line Pipe

**What goes wrong:** `|>` at line start gets parsed as a new statement starting with a `|>` operator (which is invalid in prefix position), rather than continuing the previous expression.
**Why it happens:** At top-level (zero delimiter depth), newlines are significant. The parser sees NEWLINE, treats it as a statement terminator, then sees PIPE which doesn't start a valid expression.
**How to avoid:** In the main parse loop (or in `expr_bp`), after parsing a complete expression and encountering a NEWLINE, peek ahead to check if the next significant token is `PIPE` (`|>`). If so, treat the newline as insignificant (continuation) and continue the Pratt parser loop. This is analogous to how Go handles automatic semicolons.
**Warning signs:** Parse error "expected expression" when a line starts with `|>`.

### Pitfall 3: Struct Update vs Map Literal Ambiguity

**What goes wrong:** `%{user | name: "Bob"}` gets parsed as a map literal with `user | name` as the first key expression.
**Why it happens:** The parser currently parses `%{` as map literal start, then expects `expr => expr` entries. The `user | name: "Bob"` doesn't match this pattern.
**How to avoid:** In `parse_map_literal`, after consuming `%{`, check if the pattern is `ident |` or `ident BAR`. If the first expression inside `%{...}` is an identifier followed by `|` (BAR token), parse as struct update instead. The key disambiguator is the BAR token after the first identifier -- map entries use `=>` (FAT_ARROW), not `|` (BAR).
**Warning signs:** Parse error "expected `=>`" when writing struct update syntax.

### Pitfall 4: Atom vs Colon Ambiguity in Different Contexts

**What goes wrong:** `:name` in keyword arguments (`where(name: "Alice")`) gets confused with atom literals (`:name`).
**Why it happens:** Both use the `:` character. In keyword args, the `:` follows an identifier (`name:`). In atoms, the `:` precedes an identifier (`:name`).
**How to avoid:** These are lexically distinct: keyword args have `Ident Colon`, atoms have `Colon Ident` (no space between). The lexer should lex `:name` (colon immediately followed by identifier chars with no whitespace) as a single `Atom` token. `name:` remains `Ident Colon`. The disambiguation happens at the lexer level based on position of the colon relative to the identifier.
**Warning signs:** `:name` inside atoms gets tokenized as `Colon Ident` instead of `Atom`.

### Pitfall 5: deriving(Schema) Must Not Conflict with Existing deriving Traits

**What goes wrong:** `deriving(Schema)` triggers "unknown derive trait" error because "Schema" is not in the `valid_derives` list.
**Why it happens:** The existing derive validation in `infer.rs` has a hardcoded list: `["Eq", "Ord", "Display", "Debug", "Hash", "Json", "Row"]`. "Schema" needs to be added.
**How to avoid:** Add "Schema" to the `valid_derives` array. Implement the Schema derive handling after the existing Row/Json handlers, following the same pattern.
**Warning signs:** TypeError::UnknownDeriveTrait for "Schema".

### Pitfall 6: Cross-Module from_row Trait Method Resolution

**What goes wrong:** When `User.from_row(row)` is called from a different module than where `User` is defined, the trait method `FromRow__from_row__User` is not found.
**Why it happens:** The MIR lowerer qualifies function names with module prefixes for non-public functions. Trait method names like `FromRow__from_row__User` may get double-qualified or the lookup may fail because the method was registered in a different module's namespace.
**How to avoid:** Ensure trait method names generated by `deriving(Row)` are always unqualified (they are global symbols). Check that `qualify_name` in `lower.rs` correctly handles the `FromRow__` prefix pattern (it should already, as trait impls use the `trait__method__type` naming convention which is excluded from module prefixing).
**Warning signs:** LLVM linker error "undefined symbol: ModuleName__FromRow__from_row__User".

## Code Examples

### Atom Literal -- Lexer Addition

```rust
// In mesh-common/src/token.rs, add to TokenKind:
/// Atom literal (`:name`, `:email`, `:asc`).
Atom,

// In mesh-lexer/src/lib.rs, modify lex_colon:
fn lex_colon(&mut self, start: u32) -> Token {
    self.cursor.advance(); // consume ':'
    match self.cursor.peek() {
        Some(':') => {
            self.cursor.advance();
            Token::new(TokenKind::ColonColon, start, self.cursor.pos())
        }
        Some(c) if is_ident_start(c) => {
            // Atom literal: :name, :email, :asc
            self.cursor.advance(); // consume first ident char
            self.cursor.eat_while(is_ident_continue);
            Token::new(TokenKind::Atom, start, self.cursor.pos())
        }
        _ => Token::new(TokenKind::Colon, start, self.cursor.pos()),
    }
}
```

### Atom Literal -- Type Checker

```rust
// In mesh-typeck/src/infer.rs, in the expression inference:
// Atom literals have type Atom (a distinct type from String)
// The atom value is the text after the colon: `:name` -> "name"
Ty::Con(TyCon::new("Atom"))
```

### Atom Literal -- MIR Lowering + Codegen

```rust
// In MIR, atoms lower to StringLit with the atom name:
// :name -> MirExpr::StringLit("name".to_string(), MirType::String)
// At LLVM level, atoms are global string constants -- identical to string literals.
// The type distinction exists only in the type checker, not at runtime.
```

### Keyword Arguments -- Parser Desugaring

```rust
// where(name: "Alice", age: 30) desugars to:
// where(%{"name" => "Alice", "age" => 30})
//
// In parse_arg_list, when we see Ident Colon (but NOT ColonColon),
// treat all key: value pairs as a synthetic map literal argument.
// The parser constructs a MAP_LITERAL node with MAP_ENTRY children
// where keys are string literals (from the identifier text).
```

### Multi-line Pipe -- Parser Continuation

```rust
// In the expression parser (expr_bp), after completing a full expression,
// before checking if the current token is a newline that terminates the statement:
//
// users
//   |> Enum.filter(fn u -> u.active end)
//   |> Enum.map(fn u -> u.name end)
//
// The parser must look ahead past newlines to check for |> continuation.
// This can be done in the main parse loop: after parsing an item/expression,
// if current() == NEWLINE, peek past newlines for PIPE. If found,
// do not treat the newline as a statement terminator.
```

### Struct Update -- Full Pipeline

```rust
// Mesh syntax:
// %{user | name: "Bob", email: "bob@example.com"}
//
// Parser: produces STRUCT_UPDATE_EXPR with:
//   - base expression (the identifier `user`)
//   - list of STRUCT_LITERAL_FIELD nodes (name: "Bob", email: "bob@example.com")
//
// Type checker:
//   - Infer type of base expression (must be a struct type)
//   - Verify all override fields exist in the struct
//   - Verify override field types match
//   - Result type = same struct type as base
//
// MIR:
//   - Lower to: allocate new struct, copy all fields from base, overwrite listed fields
//   - MirExpr::StructUpdate { base, overrides, ty }  (new MirExpr variant)
//
// Codegen:
//   - Allocate struct on stack
//   - For each field in struct definition:
//     - If field is in override list: store override value
//     - Else: load from base struct, store in new struct
//   - Return pointer to new struct
```

### deriving(Schema) -- Registration Pattern

```rust
// In infer.rs, when processing a struct with deriving(Schema):
//
// struct User do
//   id :: String
//   name :: String
//   email :: String
// end deriving(Schema)
//
// This should generate:
// 1. A __table__() function returning "users" (pluralized, lowercased struct name)
// 2. A __fields__() function returning field metadata
// 3. A __primary_key__() function returning "id" (default or configured)
//
// These are registered as regular functions in the module's namespace,
// with mangled names like "User____table__" to avoid conflicts.
// They return compile-time constant values (string literals, list literals).
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| No atoms (Phase 1 decision) | Atoms as compile-time string constants with distinct Atom type | Phase 96 (this phase) | Enables field references like `:name` for ORM query builder |
| No keyword arguments | `name: value` desugars to Map inside argument lists | Phase 96 (this phase) | Ergonomic DSL syntax for where/order_by/etc. |
| Single-line pipe expressions only | `\|>` at line start continues previous expression | Phase 96 (this phase) | Readable multi-step query chains |
| No struct update syntax | `%{struct \| field: value}` produces new struct | Phase 96 (this phase) | Immutable data transformation for changesets |
| deriving supports Eq/Ord/Display/Debug/Hash/Json/Row | Also supports Schema | Phase 96 (this phase) | Compile-time ORM metadata generation |

**Note on atoms:** Phase 1 research recommended "no atoms" for the core language because open-world atoms conflict with HM type inference. This phase adds atoms as a CLOSED form -- `:name` is syntactic sugar for a typed string constant, not an open-world atom system like Erlang/Elixir. The Atom type is opaque, and atoms can only be used where the ORM API expects `Atom` parameters. They do not participate in pattern matching or exhaustiveness checking as individual values.

## Open Questions

1. **Atom equality semantics**
   - What we know: Atoms compile to string constants. Two identical atoms (`:name` in different places) should be equal.
   - What's unclear: Should atoms be compared by pointer (interned) or by string content? Pointer comparison is faster but requires a global intern table.
   - Recommendation: Use string comparison. The ORM use case involves comparing atoms to field names (strings), so string semantics are natural. Interning is premature optimization that adds runtime complexity.

2. **Keyword argument scope**
   - What we know: Keyword args should work in function call argument lists.
   - What's unclear: Should keyword args work ONLY at the end of an argument list (like Python), or can they appear anywhere? Can positional and keyword args be mixed?
   - Recommendation: Keyword args should be a single map literal synthesized from all `key: value` pairs at the end of the argument list. Positional args come first. `f(a, b, name: "x", age: 30)` -> `f(a, b, %{"name" => "x", "age" => 30})`. This matches Elixir's keyword list semantics.

3. **Multi-line pipe implementation strategy**
   - What we know: The parser needs to look ahead past newlines for `|>`.
   - What's unclear: Should this be implemented as a lexer-level transformation (suppress newlines before `|>`), a parser-level peek, or a separate "line continuation" pass?
   - Recommendation: Parser-level peek. In the main expression parsing loop, after parsing an expression and seeing a NEWLINE, peek ahead past whitespace/newlines/comments. If `|>` is found, continue the Pratt loop instead of terminating the statement. This is localized and doesn't affect other syntax.

4. **Struct update -- base expression restrictions**
   - What we know: `%{user | name: "Bob"}` where `user` is a variable of struct type.
   - What's unclear: Can the base be an arbitrary expression (`%{get_user() | name: "Bob"}`)? Or only a variable?
   - Recommendation: Allow any expression as the base, matching Elixir's `%{struct | field: value}` where `struct` is any expression. The type checker validates the result type.

5. **Relationship declaration syntax in struct bodies**
   - What we know: `belongs_to :user, User` / `has_many :posts, Post` inside struct bodies.
   - What's unclear: How these declarations interact with struct field lists. Are they separate from fields? Do they produce virtual fields on the struct?
   - Recommendation: Relationship declarations are metadata-only -- they do NOT add fields to the struct's runtime layout. They are parsed as special declarations inside the struct body (after the field list) and recorded in the Schema metadata. At the type checker level, they add entries to a relationship registry, not to the struct's field list. Preloading fills these at runtime via separate queries.

## Sources

### Primary (HIGH confidence)
- Codebase analysis: `crates/mesh-common/src/token.rs` -- 96 TokenKind variants, complete lexer vocabulary
- Codebase analysis: `crates/mesh-lexer/src/lib.rs` -- Hand-written lexer with state machine, ~700 lines
- Codebase analysis: `crates/mesh-parser/src/syntax_kind.rs` -- Full CST node kind enumeration, ~90+ composite nodes
- Codebase analysis: `crates/mesh-parser/src/parser/expressions.rs` -- Pratt parser with binding power tables
- Codebase analysis: `crates/mesh-parser/src/parser/mod.rs` -- Newline significance via delimiter depth tracking
- Codebase analysis: `crates/mesh-parser/src/ast/` -- Zero-cost typed AST wrappers via `ast_node!` macro
- Codebase analysis: `crates/mesh-typeck/src/infer.rs` -- HM type inference, 7662 lines, TypeRegistry, deriving infrastructure
- Codebase analysis: `crates/mesh-typeck/src/ty.rs` -- Ty/TyCon/Scheme type representation
- Codebase analysis: `crates/mesh-codegen/src/mir/mod.rs` -- MirModule/MirExpr/MirType definitions
- Codebase analysis: `crates/mesh-codegen/src/mir/lower.rs` -- AST-to-MIR lowering, 13211 lines
- Codebase analysis: `crates/mesh-codegen/src/codegen/expr.rs` -- LLVM IR generation for expressions
- Codebase analysis: `.planning/REQUIREMENTS.md` -- COMP-01 through COMP-08 requirements
- Codebase analysis: `.planning/ROADMAP.md` -- Phase 96 plan breakdown (5 plans)

### Secondary (MEDIUM confidence)
- Phase 1 Research: `.planning/phases/01-project-foundation-lexer/01-RESEARCH.md` -- Original "no atoms" decision with rationale; atoms now being added in limited form for ORM use case

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- This is the existing compiler codebase, fully understood through 95 shipped phases
- Architecture: HIGH -- Clear patterns established across all compiler layers with extensive prior art
- Pitfalls: HIGH -- Based on direct analysis of existing code (newline handling, deriving validation, struct parsing)
- Atom design: MEDIUM -- The "atoms as typed strings" approach is a design decision; runtime semantics (interning vs string comparison) still open
- deriving(Schema) design: MEDIUM -- The metadata function generation pattern is new but follows established deriving patterns closely

**Research date:** 2026-02-16
**Valid until:** 2026-03-16 (compiler internals are stable, controlled by this project)
