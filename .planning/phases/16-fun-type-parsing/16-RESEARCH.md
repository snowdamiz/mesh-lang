# Phase 16: Fun() Type Parsing - Research

**Researched:** 2026-02-07
**Domain:** Compiler internals -- parser, AST, type checker (Rust, rowan CST)
**Confidence:** HIGH

## Summary

This phase adds `Fun(Int, String) -> Bool` as a first-class function type annotation syntax to the Snow compiler. Currently, `Fun` is lexed as a regular IDENT and the parser treats `Fun(Int)` like a type constructor application (similar to `Option(Int)` -- though the parser actually expects angle brackets for generics, so it probably just parses `Fun` as a simple type name and ignores the parenthesized args or treats them as a separate expression). The `->` after a type annotation is consumed by the *function return type* parser, not the type annotation parser, which means `Fun(Int) -> String` currently gets misparsed.

The fix requires changes in three crates: **snow-parser** (recognize `Fun(...)` in `parse_type` and emit it as a new CST node or handle it within the existing type annotation infrastructure), **snow-typeck** (convert the parsed `Fun(...)` type annotation into `Ty::Fun(params, ret)` during type resolution), and ensuring the formatter (**snow-fmt**) and LSP (**snow-lsp**) handle the new syntax gracefully.

The type system already has full support for `Ty::Fun(Vec<Ty>, Box<Ty>)` -- the internal representation is complete and unification already works for function types. The gap is purely in the surface syntax parsing and the type annotation resolution pipeline.

**Primary recommendation:** Handle `Fun` as a special case in `parse_type()` -- when the parser sees `IDENT("Fun")` followed by `L_PAREN`, parse the parenthesized types as parameter types, then require `ARROW` and parse the return type, emitting a new `FUN_TYPE` CST node. In `resolve_type_annotation`, detect this node (or the `Fun` IDENT + ARROW pattern) and produce `Ty::Fun(params, ret)`.

## Standard Stack

This is internal compiler work. No new external libraries needed.

### Core
| Component | Version | Purpose | Why Standard |
|-----------|---------|---------|--------------|
| snow-parser | in-tree | CST parsing with rowan | Existing parser infrastructure |
| snow-typeck | in-tree | HM type inference with ena | Existing type checker |
| snow-common | in-tree | Token definitions, spans | Shared types |
| rowan | 0.15+ | Lossless CST framework | Used throughout compiler |
| ena | 0.14+ | Union-find for type inference | Used by InferCtx |

### Supporting
| Component | Version | Purpose | When to Use |
|-----------|---------|---------|-------------|
| snow-fmt | in-tree | Code formatter | Must handle Fun() type syntax in output |
| snow-lsp | in-tree | LSP server | Must handle Fun() types for hover/goto |

### No New Dependencies
This phase requires zero new crates. Everything is internal compiler modifications.

## Architecture Patterns

### Current Type Annotation Pipeline

```
Source: `fn apply(f :: Fun(Int) -> String, x :: Int) -> String`
                        ^^^^^^^^^^^^^^^^^
                        TYPE_ANNOTATION node

Lexer:  [IDENT("Fun"), L_PAREN, IDENT("Int"), R_PAREN, ARROW, IDENT("String")]

Parser: parse_type() sees IDENT("Fun"), advances past it.
        Then sees L_PAREN -- but parse_type() doesn't handle L_PAREN after IDENT.
        It falls through. The "(Int) -> String" part gets misparsed or ignored.

Type Checker: resolve_type_annotation() collects tokens from TYPE_ANNOTATION node.
              collect_annotation_tokens() collects IDENT, LT, GT, COMMA, QUESTION,
              BANG, L_PAREN, R_PAREN -- but NOT ARROW tokens.
              So even if the parser correctly emitted all tokens, the type checker
              would never see the `->` and return type.
```

### Required Changes

#### 1. Parser: `parse_type()` in `snow-parser/src/parser/items.rs`

The `parse_type()` function currently handles:
- Tuple types: `(A, B, C)` -- starts with `L_PAREN`
- Simple types: `Int`, `String` -- just an `IDENT`
- Qualified types: `Foo.Bar` -- `IDENT DOT IDENT`
- Generic applications: `List<Int>` -- `IDENT LT ... GT`
- Option sugar: `Int?` -- type + `QUESTION`
- Result sugar: `T!E` -- type + `BANG` + type

**Add:** When `parse_type()` encounters `IDENT("Fun")` followed by `L_PAREN`, treat it as a function type annotation:
```
Fun(ParamType1, ParamType2) -> ReturnType
```
Parse the parameter types in parentheses, expect `ARROW`, parse the return type.

**New CST node:** Add `FUN_TYPE` to `SyntaxKind` enum to wrap the entire function type annotation. This keeps the CST lossless and makes it easy for the type checker to identify function type annotations.

#### 2. Type Checker: `resolve_type_annotation()` and `parse_type_tokens()` in `snow-typeck/src/infer.rs`

Two approaches, use **approach A**:

**Approach A (Recommended): Handle at `collect_annotation_tokens` + `parse_type_tokens` level**
- Add `ARROW` to the list of collected tokens in `collect_annotation_tokens()`
- In `parse_type_tokens()`, when encountering `IDENT("Fun")` followed by `L_PAREN`, parse it as:
  - Skip "Fun"
  - Parse parenthesized parameter types
  - Expect ARROW
  - Parse return type
  - Return `Ty::Fun(param_tys, Box::new(ret_ty))`

**Approach B: Handle at CST node level**
- If we add `FUN_TYPE` to `SyntaxKind`, we can detect it in `resolve_type_annotation()` and extract children directly.

Use both: add the `FUN_TYPE` node for clean CST representation AND handle it in `parse_type_tokens` for the token-based resolution path.

#### 3. Formatter: `snow-fmt/src/walker.rs`

The formatter uses `walk_node()` which recursively walks CST nodes and emits tokens. If we add a `FUN_TYPE` node kind, the formatter needs a case for it, or it will fall through to the default token-by-token walk (which should work fine since rowan preserves all tokens).

#### 4. Display: `Ty::Display` in `snow-typeck/src/ty.rs`

Currently displays as `(Int, String) -> Bool`. Consider displaying as `Fun(Int, String) -> Bool` to match the source syntax. This is optional -- the current display works fine for error messages.

### Recommended Project Structure

```
Changes in:
crates/
  snow-parser/src/
    parser/items.rs      # parse_type() -- add Fun() handling
    syntax_kind.rs       # Add FUN_TYPE node kind
  snow-typeck/src/
    infer.rs             # collect_annotation_tokens, parse_type_tokens
  snow-fmt/src/
    walker.rs            # Handle FUN_TYPE node (likely no-op)
  snow-common/src/
    token.rs             # No changes needed (Fun is IDENT, not keyword)
```

### Pattern: Special-Case Identifier in Type Parser

**What:** When parsing types, check the text of an IDENT token to decide parsing strategy
**When to use:** When a type constructor has special syntax that differs from normal generic application
**Already used:** The codebase already does this pattern -- `from` is an IDENT checked by text in `parse_item_or_stmt()`. Strategy/child-spec parsing checks IDENT text for "strategy", "max_restarts", etc.

```rust
// In parse_type():
if p.at(SyntaxKind::IDENT) && p.current_text() == "Fun" && p.nth(1) == SyntaxKind::L_PAREN {
    // Parse function type: Fun(params) -> ReturnType
    let m = p.open();
    p.advance(); // Fun
    p.advance(); // (
    // Parse comma-separated parameter types
    if !p.at(SyntaxKind::R_PAREN) {
        parse_type(p);
        while p.eat(SyntaxKind::COMMA) {
            if p.at(SyntaxKind::R_PAREN) { break; }
            parse_type(p);
        }
    }
    p.expect(SyntaxKind::R_PAREN);
    // Expect -> ReturnType
    p.expect(SyntaxKind::ARROW);
    if !p.has_error() {
        parse_type(p);
    }
    p.close(m, SyntaxKind::FUN_TYPE);
    return;
}
```

### Anti-Patterns to Avoid

- **Making `Fun` a keyword:** "Fun" should remain an IDENT. Making it a keyword would break any existing code using `Fun` as a variable name (unlikely but possible) and adds unnecessary complexity to the lexer and keyword table. Type-position disambiguation is sufficient.
- **Reusing GENERIC_ARG_LIST for Fun params:** The angle-bracket generic args (`List<Int>`) are semantically different from function parameter types (`Fun(Int, String)`). Use a separate node or inline parsing.
- **Forgetting the ARROW in token collection:** The current `collect_annotation_tokens` does NOT collect ARROW tokens. This MUST be fixed or the type checker will never see `->` in `Fun(Int) -> String`.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Type unification for Fun types | Custom Fun unification | Existing `Ty::Fun` + `InferCtx::unify()` | Already handles function types perfectly |
| Type variable creation for Fun params | Manual TyVar management | `InferCtx::fresh_var()` + existing HM machinery | The HM system already supports `Ty::Fun` |
| CST token walking | Custom tree traversal | `rowan::NodeOrToken` iteration | The formatter and type checker already use this pattern |
| Type alias resolution for Fun types | Special Fun alias handling | Existing `resolve_alias()` already recurses into `Ty::Fun` | See lines 5035-5037 in infer.rs |

**Key insight:** The type system (`Ty::Fun`), unification, generalization, and instantiation ALL already support function types. The only gap is the parser and the type annotation resolution pipeline. No type system changes are needed.

## Common Pitfalls

### Pitfall 1: Forgetting ARROW Token Collection

**What goes wrong:** `collect_annotation_tokens()` only collects `IDENT`, `LT`, `GT`, `COMMA`, `QUESTION`, `BANG`, `L_PAREN`, `R_PAREN`. If ARROW is not added, `Fun(Int) -> String` will parse to just `Fun(Int)` in the type checker (the `-> String` part is silently dropped).
**Why it happens:** The token collection was designed for simple types, generics, and sugar -- function type annotations were never supported.
**How to avoid:** Add `SyntaxKind::ARROW` to both `collect_annotation_tokens` instances (lines ~4924 and ~1505 in infer.rs).
**Warning signs:** Tests pass for parsing but type checking produces `Fun` as `Ty::Con("Fun")` instead of `Ty::Fun(...)`.

### Pitfall 2: Ambiguity Between Fun Type and Function Call in parse_type

**What goes wrong:** In type position, `Fun(Int)` should be parsed as a function type. In expression position, `Fun(42)` would be a function call. The parser must only apply the special `Fun` handling inside `parse_type()`, which is only called from type annotation contexts.
**Why it happens:** Type and expression parsing share some similar patterns.
**How to avoid:** The fix is scoped to `parse_type()` only, which is already only called from type annotation positions (after `::`, after `->` in fn signatures, in struct fields, in type aliases). No expression parser changes needed.
**Warning signs:** Expression parsing breaks for variables named `Fun`.

### Pitfall 3: Nested Function Types

**What goes wrong:** `Fun(Fun(Int) -> String) -> Bool` requires recursive parsing -- the parameter type is itself a function type. If parsing is not recursive, nested Fun types fail.
**Why it happens:** Function type parameters can contain function types.
**How to avoid:** `parse_type()` already calls itself recursively for generic args and tuple elements. Since Fun type parsing calls `parse_type()` for each parameter type and the return type, recursion is automatic.
**Warning signs:** Tests with nested function types fail with parse errors.

### Pitfall 4: Fun Without Parentheses

**What goes wrong:** A user might write `Fun Int -> String` (without parentheses). The parser should require parentheses: `Fun(Int) -> String`. Without the `L_PAREN` check, the parser might try to parse this differently.
**Why it happens:** Inconsistency in expected syntax.
**How to avoid:** The check `p.current_text() == "Fun" && p.nth(1) == SyntaxKind::L_PAREN` ensures parentheses are required. If `Fun` appears without `(`, it falls through to normal IDENT type parsing, producing `Ty::Con("Fun")`.
**Warning signs:** `Fun Int -> String` silently parses as something weird instead of giving an error.

### Pitfall 5: Zero-Arity Function Types

**What goes wrong:** `Fun() -> Int` (no parameters) is a valid function type. The parser must handle empty parameter lists.
**Why it happens:** Edge case in comma-separated parsing.
**How to avoid:** Check `p.at(SyntaxKind::R_PAREN)` before parsing parameter types, same pattern used for empty arg lists elsewhere.
**Warning signs:** `Fun() -> Int` causes a parse error or incorrect type.

### Pitfall 6: Two Token Collection Sites

**What goes wrong:** There are TWO places in infer.rs that collect annotation tokens -- the main `collect_annotation_tokens()` function (line ~4915) and an inline version for type alias resolution (line ~1505). Both must be updated to include ARROW tokens.
**Why it happens:** The token collection logic was duplicated for type alias parsing.
**How to avoid:** Update both sites. Search for all occurrences of `SyntaxKind::IDENT | SyntaxKind::LT | SyntaxKind::GT` in infer.rs.
**Warning signs:** `Fun(Int) -> String` works in struct fields but not in type aliases (or vice versa).

## Code Examples

### Current parse_type() Function (what exists)
```rust
// Source: crates/snow-parser/src/parser/items.rs, line 360
pub(crate) fn parse_type(p: &mut Parser) {
    // Tuple type: (A, B, C)
    if p.at(SyntaxKind::L_PAREN) { /* ... */ }

    if !p.at(SyntaxKind::IDENT) {
        p.error("expected type name");
        return;
    }

    p.advance(); // type name IDENT
    // Optional dot-separated path
    while p.at(SyntaxKind::DOT) { /* ... */ }
    // Optional generic arguments: <A, B>
    if p.at(SyntaxKind::LT) { /* ... */ }
    // Option sugar: Type?
    if p.at(SyntaxKind::QUESTION) { /* ... */ }
    // Result sugar: Type!ErrorType
    if p.at(SyntaxKind::BANG) { /* ... */ }
}
```

### Required Addition to parse_type() -- Insert Before Generic IDENT Handling
```rust
// INSERT at the top of parse_type(), before the generic IDENT handling:
// Function type: Fun(ParamTypes) -> ReturnType
if p.at(SyntaxKind::IDENT) && p.current_text() == "Fun" && p.nth(1) == SyntaxKind::L_PAREN {
    let m = p.open();
    p.advance(); // Fun
    p.advance(); // (
    if !p.at(SyntaxKind::R_PAREN) {
        parse_type(p);
        while p.eat(SyntaxKind::COMMA) {
            if p.at(SyntaxKind::R_PAREN) { break; }
            parse_type(p);
        }
    }
    p.expect(SyntaxKind::R_PAREN);
    p.expect(SyntaxKind::ARROW);
    if !p.has_error() {
        parse_type(p);
    }
    p.close(m, SyntaxKind::FUN_TYPE);
    return;
}
```

### Required Addition to collect_annotation_tokens()
```rust
// Source: crates/snow-typeck/src/infer.rs, line ~4924
// ADD SyntaxKind::ARROW to the match:
SyntaxKind::IDENT | SyntaxKind::LT | SyntaxKind::GT
| SyntaxKind::COMMA | SyntaxKind::QUESTION | SyntaxKind::BANG
| SyntaxKind::L_PAREN | SyntaxKind::R_PAREN
| SyntaxKind::ARROW  // <-- ADD THIS
=> {
    tokens.push((kind, t.text().to_string()));
}
```

### Required Addition to parse_type_tokens()
```rust
// Source: crates/snow-typeck/src/infer.rs, line ~4966
// After getting the IDENT name, before generic args check:
if name == "Fun" && *pos < tokens.len() && tokens[*pos].0 == SyntaxKind::L_PAREN {
    *pos += 1; // skip (
    let mut param_tys = Vec::new();
    while *pos < tokens.len() && tokens[*pos].0 != SyntaxKind::R_PAREN {
        param_tys.push(parse_type_tokens(tokens, pos));
        if *pos < tokens.len() && tokens[*pos].0 == SyntaxKind::COMMA {
            *pos += 1;
        }
    }
    if *pos < tokens.len() && tokens[*pos].0 == SyntaxKind::R_PAREN {
        *pos += 1; // skip )
    }
    // Expect ->
    if *pos < tokens.len() && tokens[*pos].0 == SyntaxKind::ARROW {
        *pos += 1; // skip ->
    }
    let ret_ty = parse_type_tokens(tokens, pos);
    return Ty::Fun(param_tys, Box::new(ret_ty));
}
```

### Test Cases for Snow Code
```snow
# TYPE-01: Basic function type annotation
fn apply(f :: Fun(Int) -> String, x :: Int) -> String do
  f(x)
end

# TYPE-01: Zero-arity function type
fn run_thunk(thunk :: Fun() -> Int) -> Int do
  thunk()
end

# TYPE-01: Multi-param function type
fn combine(f :: Fun(Int, String) -> Bool) -> Bool do
  f(42, "hello")
end

# TYPE-02: Struct field with function type
struct Callback do
  handler :: Fun(String) -> Int
end

# TYPE-02: Type alias for function type
type Mapper = Fun(Int) -> String
type Predicate = Fun(Int) -> Bool

# TYPE-03: Unification with inferred function types
fn use_mapper(m :: Fun(Int) -> String) -> String do
  m(42)
end

let result = use_mapper(fn x -> to_string(x) end)
# The closure `fn x -> to_string(x) end` should unify with Fun(Int) -> String

# Nested function types
fn compose(f :: Fun(Int) -> String, g :: Fun(String) -> Bool) -> Fun(Int) -> Bool do
  fn x -> g(f(x)) end
end
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| No function type annotations | `Fun(Int) -> String` syntax | v1.2 (this phase) | Users can annotate higher-order function parameters |

**Context:** Most functional languages have function type annotation syntax:
- Haskell: `Int -> String -> Bool`
- OCaml: `int -> string -> bool`
- Scala: `(Int, String) => Bool`
- Rust: `fn(i32, String) -> bool` or `Fn(i32) -> bool`
- Elixir: `(integer() -> String.t())` (in typespecs)

Snow's `Fun(Int) -> String` follows the Rust/Scala pattern of using a keyword + parenthesized params + arrow + return, which is consistent with Snow's existing type constructor syntax (uppercase names, parentheses/angle brackets for args).

## Open Questions

1. **Should `Fun` without `->` be an error or a type constructor?**
   - What we know: `Fun(Int)` without `-> ReturnType` is syntactically ambiguous -- is it a type constructor named "Fun" with one arg, or an incomplete function type?
   - What's unclear: User intent when writing `Fun(Int)` alone
   - Recommendation: If `Fun(` is seen, REQUIRE `->` and return type. Emit a parse error like "function type annotation requires `-> ReturnType`". This is unambiguous and catches mistakes early. If someone has a type constructor named "Fun", they should rename it.

2. **Should the `Ty::Display` change to show `Fun(...) -> ...` instead of `(...) -> ...`?**
   - What we know: Currently `Ty::Fun` displays as `(Int, String) -> Bool`. This is fine for error messages but doesn't match source syntax.
   - What's unclear: Whether users would be confused by the different display
   - Recommendation: Keep current display format for now. It's clear and concise. The `Fun()` syntax is for annotations, the `() -> T` format is for error messages. This is consistent with how Rust displays `Fn(i32) -> bool` differently in error messages.

3. **Should `Fun` type annotations be supported in pattern match types?**
   - What we know: Function types in patterns (e.g., `case x of Fun(Int) -> String => ...`) don't make sense semantically -- you can't pattern match on function types
   - What's unclear: N/A
   - Recommendation: No special handling needed. Pattern matching doesn't use type annotations, so this is a non-issue.

## Sources

### Primary (HIGH confidence)
- Codebase analysis of `snow-parser/src/parser/items.rs` -- `parse_type()` function (line 360)
- Codebase analysis of `snow-typeck/src/infer.rs` -- `resolve_type_annotation()`, `collect_annotation_tokens()`, `parse_type_tokens()` (lines 4898-5002)
- Codebase analysis of `snow-typeck/src/ty.rs` -- `Ty::Fun` variant definition (line 53)
- Codebase analysis of `snow-typeck/src/unify.rs` -- `unify()` function handles `Ty::Fun` (line 194)
- Codebase analysis of `snow-common/src/token.rs` -- `keyword_from_str()` confirms "Fun" is NOT a keyword
- Codebase analysis of `snow-parser/src/syntax_kind.rs` -- complete SyntaxKind enum

### Secondary (MEDIUM confidence)
- None needed -- this is entirely internal compiler work with complete source access

### Tertiary (LOW confidence)
- None

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- this is existing codebase, no external deps
- Architecture: HIGH -- complete understanding of parser, type checker, and their interaction
- Pitfalls: HIGH -- identified from direct code analysis of both token collection sites and parser flow

**Research date:** 2026-02-07
**Valid until:** 2026-06-07 (stable -- internal compiler code, no external dependencies to go stale)
