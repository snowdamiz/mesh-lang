# Phase 2: Parser & AST - Research

**Researched:** 2026-02-06
**Domain:** Recursive descent parser, Pratt expression parsing, lossless CST + typed AST, Rust compiler architecture
**Confidence:** HIGH

## Summary

This phase builds a recursive descent parser that consumes the Phase 1 token stream (85 token kinds, including Newline tokens) and produces a lossless concrete syntax tree (CST) with a typed AST layer on top. The parser must handle all Snow language constructs: let bindings, function definitions with do/end blocks, if/else, case/match, closures, pipe operator, string interpolation, modules, and imports.

The standard approach for this domain is well-established: a hand-written recursive descent parser with Pratt parsing for expressions, building a CST using the `rowan` library (v0.16.1, the same library powering rust-analyzer). The CST preserves all tokens (whitespace, comments, delimiters) for future tooling (formatter, LSP), while a typed AST layer provides zero-cost typed accessors on top of the untyped CST nodes. Newline significance is handled at the parser level using a Scala/Kotlin-inspired approach: the lexer already emits Newline tokens, and the parser decides their significance based on context (inside parens/brackets = ignored; after expression-terminating tokens = statement separator).

Key recommendations: Use `rowan` 0.16.1 for the CST (battle-tested by rust-analyzer, well-documented, handles error nodes naturally). Use Pratt parsing (matklad's binding power approach) for expressions. Make Snow expression-oriented (everything returns a value) but distinguish between expressions and declarations at the AST level. Default to private-by-default with explicit `pub` for visibility. Support nested modules with do/end syntax. Stop at first error with no recovery (per user decision).

**Primary recommendation:** Build a rowan-backed lossless CST with a hand-written recursive descent parser using Pratt parsing for expressions, expression-oriented AST design, and parser-level newline significance.

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions

**AST representation:**
- Lossless CST + AST approach: concrete syntax tree preserves all tokens (whitespace, comments, parens) and a typed AST is derived from the CST

**Syntax resolution:**
- Newlines are significant: a newline terminates an expression unless continuation is obvious (open paren, trailing operator, pipe, comma)
- Parentheses always required for function calls: `greet("world")` not `greet "world"`
- Pipe operator `|>` passes value as first argument (Elixir-style): `x |> foo(y)` becomes `foo(x, y)`
- Trailing closures supported: `list.map() do |x| ... end` -- block passed after closing paren

**Error recovery:**
- Report first error only -- stop parsing at the first error, no recovery/synchronization
- This simplifies the parser significantly; no need for synchronization point logic
- Error messages should be a blend of Elm-friendly and Rust-precise: conversational sentence explaining the problem + precise source spans with underlines
- Unclosed delimiters always reference where they were opened: "Expected `end` to close `do` block started at line 5, column 3"
- No fix suggestions in Phase 2 -- descriptive errors only, suggestion engine deferred to Phase 10 (Tooling)

**Module & visibility:**
- Import style: `import Math` for whole module, `from Math import sqrt, pow` for selective imports
- No glob imports -- `from Math import *` is not allowed, must always name imports explicitly

### Claude's Discretion

- Arena allocation vs Box/Rc tree for CST/AST node storage
- Whether to use rowan or custom CST implementation
- Expression-only vs expression+statement AST node design
- Nested modules vs one-module-per-file
- Private-by-default+pub vs public-by-default+priv visibility

### Deferred Ideas (OUT OF SCOPE)

None -- discussion stayed within phase scope
</user_constraints>

## Discretion Recommendations

Recommendations for areas left to Claude's discretion, with rationale.

### Use rowan (not custom CST)

**Recommendation:** Use `rowan` 0.16.1 for the CST implementation.

**Rationale:**
- Battle-tested by rust-analyzer, the most complex Rust IDE
- Red-green tree model handles lossless representation naturally
- Untyped CST nodes can hold error nodes without any special handling -- any node can have arbitrary children
- Zero-cost typed AST layer via `AstNode` trait: wrap `SyntaxNode` with typed accessors, no runtime conversion cost
- `SyntaxKind(u16)` tag system maps directly to a Rust enum with `#[repr(u16)]`
- No need to reinvent tree traversal, parent pointers, text ranges
- Well-documented: matklad's blog posts, rust-analyzer source, multiple tutorials
- Active maintenance (latest release 0.16.1)

**Alternative considered:** cstree (fork of rowan with thread safety and string interning). Not needed here -- the parser is single-threaded and Snow does not need persistent red nodes or cross-thread sharing at this stage.

**Alternative considered:** Custom arena-based CST. More work, less battle-tested, and rowan already uses internal arena-like allocation (green nodes are reference-counted and deduplicated). Custom implementation would be warranted only if rowan proves too slow or inflexible, which is unlikely.

**Confidence:** HIGH -- rowan is the standard choice for lossless CST in Rust compilers.

### Expression-oriented with declaration distinction

**Recommendation:** Expression-oriented design where most constructs return values, but distinguish between expressions and "items" (declarations) at the AST level.

**Rationale:**
- Snow is Elixir-inspired where "everything is an expression" -- `if/else`, `case/match`, blocks all return values
- However, `let` bindings, `def` function definitions, `module` declarations, and `import` statements are structurally different from expressions -- they introduce names into scope
- The cleanest design (used by Rust, Kotlin, Swift) separates:
  - **Expressions** (`Expr`): things that produce values -- literals, binary ops, if/else, case/match, function calls, blocks, closures, pipe chains
  - **Items/Declarations** (`Item`): things that introduce names -- `let`, `def`/`fn`, `module`, `import`, `struct`, `type`, `trait`, `impl`
  - **Statements** are just expressions or items in sequence within a block, where the last expression is the block's value
- This gives type checking clean categories to work with: items are type-checked for their signatures, expressions are type-checked for their values
- `if/else`, `case/match`, and `do/end` blocks are expressions (they return the value of their last expression)

**Confidence:** HIGH -- this is the standard approach for expression-oriented functional languages with declarations.

### Nested modules with do/end

**Recommendation:** Support nested modules using `module Name do ... end` syntax.

**Rationale:**
- Matches Snow's Elixir-inspired syntax where `do/end` is the universal block delimiter
- Elixir itself supports nested modules: `defmodule Outer.Inner do ... end`
- Nested modules provide natural namespacing: `module Math do module Vector do ... end end`
- One-module-per-file is a packaging/tooling concern, not a parser concern -- the parser should support nested modules even if tooling later enforces file conventions
- The CST/AST should represent module nesting faithfully; policy decisions about file layout belong to later phases

**Confidence:** HIGH -- natural fit for Elixir-inspired syntax.

### Private-by-default with explicit `pub`

**Recommendation:** Private-by-default visibility with explicit `pub` keyword for public items.

**Rationale:**
- Follows the principle of least privilege -- unexported by default is safer
- Matches Rust, Go (unexported by default), and most modern languages
- Explicit `pub` is a deliberate choice by the author, making the API surface intentional
- Elixir is technically public-by-default for functions but uses `defp` for private -- Snow's `pub` prefix is more consistent with the keyword-based syntax
- Syntax: `pub fn greet(name) do ... end` vs `fn helper() do ... end` (private)
- Applies to: functions, structs, type aliases, traits

**Confidence:** HIGH -- overwhelming consensus in modern language design.

## Standard Stack

The established libraries/tools for this phase:

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| rowan | 0.16.1 | Lossless CST (red-green tree) | Powers rust-analyzer; battle-tested for lossless syntax trees |
| snow-common | workspace | Token types, Span, errors | Already exists from Phase 1 |
| insta | 1.46 (workspace) | Snapshot testing | Already in workspace; ideal for parser output testing |
| serde | 1 (workspace) | Serialization for snapshots | Already in workspace for token serialization |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| text-size | (via rowan) | TextRange/TextSize types | Rowan re-exports these for span handling |
| rustc-hash | (via rowan) | Fast hashing for syntax node dedup | Transitive dependency, no direct use needed |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| rowan | cstree | Thread safety + string interning, but immutable trees; overkill for single-threaded parser |
| rowan | Custom arena (bumpalo) | Full control but reinventing tree traversal, parent pointers, text ranges; ~20% perf gain not needed at this stage |
| Hand-written Pratt parser | pratt crate | External dependency for something that's ~100 lines of code; hand-written gives full control over error messages |

**Installation:**
```toml
# In crates/snow-parser/Cargo.toml
[dependencies]
snow-common = { path = "../snow-common" }
snow-lexer = { path = "../snow-lexer" }
rowan = "0.16"

[dev-dependencies]
insta = { workspace = true }
serde = { workspace = true }
```

**Workspace update:**
```toml
# In root Cargo.toml [workspace.dependencies]
rowan = "0.16"
```

## Architecture Patterns

### Recommended Project Structure

```
crates/
  snow-common/       # Existing: Token, Span, LexError
    src/
      lib.rs
      token.rs       # TokenKind enum (85 variants)
      span.rs        # Span, LineIndex
      error.rs       # LexError (rename or extend for ParseError)
  snow-lexer/        # Existing: Lexer iterator
  snow-parser/       # NEW: Parser + CST + AST
    src/
      lib.rs         # Public API: parse(source) -> Parse
      syntax_kind.rs # SyntaxKind enum (u16) for rowan -- superset of TokenKind + node kinds
      parser/
        mod.rs       # Parser struct, token cursor, marker API
        expressions.rs  # Pratt expression parser
        items.rs     # Declaration parsers (let, fn, module, import, struct, etc.)
        patterns.rs  # Pattern parsing (match arms, function params)
      cst.rs         # Rowan tree types: SnowLanguage, SyntaxNode, SyntaxToken
      ast/
        mod.rs       # AstNode trait, typed AST node wrappers
        generated.rs # (or hand-written) typed AST nodes: FnDef, LetBinding, IfExpr, etc.
        expr.rs      # Expression AST nodes
        item.rs      # Declaration/item AST nodes
        pat.rs       # Pattern AST nodes
      error.rs       # ParseError type with span and message
    tests/
      parser_tests.rs
      snapshots/     # insta YAML snapshots of parsed trees
```

### Pattern 1: Rowan-based Two-Layer Architecture

**What:** Untyped CST (rowan `SyntaxNode`) with typed AST wrappers on top.
**When to use:** Always -- this is the core architecture.

The parser builds a rowan green tree using `GreenNodeBuilder`. The CST is fully lossless (preserves all tokens including whitespace and comments). Typed AST nodes wrap CST nodes with zero-cost accessors.

```rust
// syntax_kind.rs -- SyntaxKind enum for rowan
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum SyntaxKind {
    // Tokens (map from TokenKind)
    LET_KW,
    FN_KW,
    // ... all TokenKind variants mapped to SCREAMING_SNAKE_CASE ...
    IDENT,
    INT_LITERAL,
    NEWLINE,
    WHITESPACE,
    COMMENT,
    EOF,
    ERROR,

    // Composite nodes (CST node kinds)
    SOURCE_FILE,
    FN_DEF,
    LET_BINDING,
    IF_EXPR,
    CASE_EXPR,
    MATCH_ARM,
    BINARY_EXPR,
    UNARY_EXPR,
    CALL_EXPR,
    PIPE_EXPR,
    FIELD_ACCESS,
    BLOCK,
    PARAM_LIST,
    PARAM,
    ARG_LIST,
    MODULE_DEF,
    IMPORT_DECL,
    FROM_IMPORT_DECL,
    STRUCT_DEF,
    STRUCT_FIELD,
    CLOSURE_EXPR,
    LITERAL,
    NAME,
    NAME_REF,
    PATH,       // Module::path::name
    TYPE_ANNOTATION,
    VISIBILITY,
    PATTERN,
    WILDCARD_PAT,
    IDENT_PAT,
    LITERAL_PAT,
    TUPLE_PAT,
    STRUCT_PAT,
    STRING_EXPR,       // complete interpolated string expression
    STRING_LITERAL,    // string content part
    INTERPOLATION,     // ${expr} within string
    RETURN_EXPR,
    TRAILING_CLOSURE,  // do |params| ... end after call
}

// Implement rowan::Language for Snow
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SnowLanguage {}

impl rowan::Language for SnowLanguage {
    type Kind = SyntaxKind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> Self::Kind {
        // Safety: SyntaxKind is repr(u16)
        unsafe { std::mem::transmute::<u16, SyntaxKind>(raw.0) }
    }

    fn kind_to_raw(kind: Self::Kind) -> rowan::SyntaxKind {
        rowan::SyntaxKind(kind as u16)
    }
}

pub type SyntaxNode = rowan::SyntaxNode<SnowLanguage>;
pub type SyntaxToken = rowan::SyntaxToken<SnowLanguage>;
```

### Pattern 2: Event-Based Parser (matklad's approach)

**What:** The parser emits events (Open/Close/Advance) instead of directly building the tree. Events are converted to a rowan green tree in a separate pass.
**When to use:** This is the recommended parser architecture -- it decouples parsing logic from tree construction.

```rust
// parser/mod.rs
pub struct Parser<'t> {
    tokens: &'t [Token],
    pos: usize,
    events: Vec<Event>,
    source: &'t str,
    /// Track nesting depth for newline significance
    paren_depth: u32,    // ( )
    bracket_depth: u32,  // [ ]
    brace_depth: u32,    // { }
}

enum Event {
    Open { kind: SyntaxKind },
    Close,
    Advance,   // consume one token
    Error { message: String },
}

struct MarkOpened {
    index: usize,
}

impl<'t> Parser<'t> {
    /// Start a new CST node. Returns marker for later close().
    fn open(&mut self) -> MarkOpened {
        let mark = MarkOpened { index: self.events.len() };
        self.events.push(Event::Open { kind: SyntaxKind::ERROR });
        mark
    }

    /// Close a CST node with its actual kind.
    fn close(&mut self, m: MarkOpened, kind: SyntaxKind) {
        self.events[m.index] = Event::Open { kind };
        self.events.push(Event::Close);
    }

    /// Consume current token, advancing position.
    fn advance(&mut self) {
        // Skip insignificant newlines before advancing
        self.skip_newlines();
        self.events.push(Event::Advance);
        self.pos += 1;
    }

    /// Current token kind, skipping insignificant newlines.
    fn current(&self) -> TokenKind {
        self.nth(0)
    }

    /// Lookahead, returning Eof for out-of-bounds.
    fn nth(&self, n: usize) -> TokenKind {
        let mut pos = self.pos;
        let mut remaining = n;
        while pos < self.tokens.len() {
            if self.tokens[pos].kind == TokenKind::Newline && self.is_newline_insignificant() {
                pos += 1;
                continue;
            }
            if remaining == 0 {
                return self.tokens[pos].kind.clone();
            }
            remaining -= 1;
            pos += 1;
        }
        TokenKind::Eof
    }

    /// Whether newlines are currently insignificant (inside parens, brackets, braces).
    fn is_newline_insignificant(&self) -> bool {
        self.paren_depth > 0 || self.bracket_depth > 0 || self.brace_depth > 0
    }

    fn expect(&mut self, kind: TokenKind) -> bool {
        if self.current() == kind {
            self.advance();
            true
        } else {
            // Emit error -- stop parsing (first error only)
            false
        }
    }
}
```

### Pattern 3: Pratt Expression Parser (matklad's binding power)

**What:** Pratt parsing for expression precedence using binding power pairs.
**When to use:** All expression parsing -- binary operators, unary operators, pipe operator, method calls.

```rust
// parser/expressions.rs

/// Binding power for Snow operators (higher = tighter binding).
/// Left-associative: right_bp = left_bp + 1
/// Right-associative: left_bp = right_bp + 1
fn infix_binding_power(op: &TokenKind) -> Option<(u8, u8)> {
    let bp = match op {
        // Assignment (right-associative)
        TokenKind::Eq => (2, 1),

        // Pipe operator (left-associative, lowest expression precedence)
        TokenKind::Pipe => (3, 4),      // |>

        // Logical OR
        TokenKind::Or | TokenKind::PipePipe => (5, 6),

        // Logical AND
        TokenKind::And | TokenKind::AmpAmp => (7, 8),

        // Comparison (non-associative -- same left/right bp)
        TokenKind::EqEq | TokenKind::NotEq => (9, 10),
        TokenKind::Lt | TokenKind::Gt |
        TokenKind::LtEq | TokenKind::GtEq => (9, 10),

        // Range
        TokenKind::DotDot => (11, 12),

        // String/list concatenation
        TokenKind::Diamond => (13, 14),    // <>
        TokenKind::PlusPlus => (13, 14),   // ++

        // Additive
        TokenKind::Plus | TokenKind::Minus => (15, 16),

        // Multiplicative
        TokenKind::Star | TokenKind::Slash |
        TokenKind::Percent => (17, 18),

        _ => return None,
    };
    Some(bp)
}

fn prefix_binding_power(op: &TokenKind) -> Option<((), u8)> {
    let bp = match op {
        TokenKind::Minus => ((), 19),      // unary negation
        TokenKind::Bang | TokenKind::Not => ((), 19),  // logical not
        _ => return None,
    };
    Some(bp)
}

/// Postfix operations (function calls, field access, indexing)
/// are handled inline in the Pratt loop, not via binding power.
/// They always bind tighter than any infix operator.
const POSTFIX_BP: u8 = 21;

fn expr_bp(p: &mut Parser, min_bp: u8) {
    let m = p.open();

    // Parse atom or prefix expression
    match p.current() {
        // Literals
        TokenKind::IntLiteral | TokenKind::FloatLiteral |
        TokenKind::True | TokenKind::False | TokenKind::Nil => {
            p.advance();
            p.close(m, SyntaxKind::LITERAL);
            // m is now closed, need new marker for postfix/infix
        }
        TokenKind::Ident => {
            p.advance();
            p.close(m, SyntaxKind::NAME_REF);
        }
        TokenKind::StringStart => {
            parse_string_expr(p);
            p.close(m, SyntaxKind::STRING_EXPR);
        }
        TokenKind::LParen => {
            // Grouped expression
            p.advance(); // (
            expr_bp(p, 0);
            p.expect(TokenKind::RParen);
            // close as grouped expr or tuple
        }
        TokenKind::If => {
            parse_if_expr(p);
            p.close(m, SyntaxKind::IF_EXPR);
        }
        TokenKind::Case => {
            parse_case_expr(p);
            p.close(m, SyntaxKind::CASE_EXPR);
        }
        TokenKind::Fn => {
            parse_closure(p);
            p.close(m, SyntaxKind::CLOSURE_EXPR);
        }
        // Prefix operators
        kind if prefix_binding_power(&kind).is_some() => {
            let ((), r_bp) = prefix_binding_power(&kind).unwrap();
            p.advance();
            expr_bp(p, r_bp);
            p.close(m, SyntaxKind::UNARY_EXPR);
        }
        _ => {
            // Error: expected expression
            p.error("expected expression");
            return;
        }
    }

    // Postfix and infix loop
    loop {
        match p.current() {
            // Function call
            TokenKind::LParen if POSTFIX_BP >= min_bp => {
                let m2 = p.open_before(m); // wrap previous expr
                parse_arg_list(p);
                // Check for trailing closure
                if p.current() == TokenKind::Do {
                    parse_trailing_closure(p);
                }
                p.close(m2, SyntaxKind::CALL_EXPR);
            }
            // Field access
            TokenKind::Dot if POSTFIX_BP >= min_bp => {
                let m2 = p.open_before(m);
                p.advance(); // .
                p.expect(TokenKind::Ident);
                p.close(m2, SyntaxKind::FIELD_ACCESS);
            }
            // Index access
            TokenKind::LBracket if POSTFIX_BP >= min_bp => {
                let m2 = p.open_before(m);
                p.advance(); // [
                expr_bp(p, 0);
                p.expect(TokenKind::RBracket);
                p.close(m2, SyntaxKind::INDEX_EXPR);
            }
            // Infix operators
            kind if infix_binding_power(&kind).is_some() => {
                let (l_bp, r_bp) = infix_binding_power(&kind).unwrap();
                if l_bp < min_bp { break; }
                let m2 = p.open_before(m);
                p.advance(); // operator
                expr_bp(p, r_bp);
                // Special case: pipe operator gets PIPE_EXPR, others get BINARY_EXPR
                let node_kind = if kind == TokenKind::Pipe {
                    SyntaxKind::PIPE_EXPR
                } else {
                    SyntaxKind::BINARY_EXPR
                };
                p.close(m2, node_kind);
            }
            _ => break,
        }
    }
}
```

### Pattern 4: Newline Significance (Scala/Kotlin-style)

**What:** Parser-level newline handling where newline tokens from the lexer are treated as significant or insignificant based on parser context.
**When to use:** Throughout the parser -- fundamental to Snow's syntax.

**Rules for significant newlines in Snow:**

A Newline token acts as a statement/expression terminator UNLESS:
1. **Inside delimiters:** Between `(` and `)`, `[` and `]`, or `{` and `}` -- newlines are ignored
2. **After continuation tokens:** After `|>`, `,`, binary operators (`+`, `-`, `*`, `/`, `==`, etc.), `->`, `=>`, `do`, `and`, `or` -- the next line continues the expression
3. **Before continuation tokens:** Before `|>`, `.`, binary operators -- these continue the previous expression

**Implementation:** Track delimiter nesting depth in the parser. When deciding whether a Newline token is significant:

```rust
// Newlines inside any delimiter nesting are always insignificant
if self.paren_depth > 0 || self.bracket_depth > 0 || self.brace_depth > 0 {
    // Skip newline silently
    return;
}

// After certain tokens, newline is continuation (not terminator)
let prev = self.previous_significant_token();
match prev {
    TokenKind::Pipe       |  // |>
    TokenKind::Comma      |  // ,
    TokenKind::Plus       | TokenKind::Minus     |
    TokenKind::Star       | TokenKind::Slash     |
    TokenKind::Percent    | TokenKind::PlusPlus  |
    TokenKind::Diamond    | TokenKind::EqEq      |
    TokenKind::NotEq      | TokenKind::Lt        |
    TokenKind::Gt         | TokenKind::LtEq      |
    TokenKind::GtEq       | TokenKind::AmpAmp    |
    TokenKind::PipePipe   | TokenKind::DotDot    |
    TokenKind::Arrow      | TokenKind::FatArrow  |
    TokenKind::Eq         | TokenKind::Do        |
    TokenKind::And        | TokenKind::Or        => {
        // Continuation -- skip newline
    }
    _ => {
        // Significant newline -- treat as statement terminator
    }
}
```

### Pattern 5: Error Reporting with Source Context

**What:** Parse errors carry the span of the problem, a human-readable message, and optional reference to a related span (e.g., where a delimiter was opened).
**When to use:** Every error path in the parser.

```rust
pub struct ParseError {
    /// What went wrong
    pub message: String,
    /// Where it went wrong (primary span)
    pub span: Span,
    /// Optional related span (e.g., "opened here")
    pub related: Option<(String, Span)>,
}

// Example error construction:
ParseError {
    message: "expected `end` to close `do` block".into(),
    span: current_span,
    related: Some((
        "block started here".into(),
        do_token_span,
    )),
}
```

### Anti-Patterns to Avoid

- **Building AST directly without CST:** Loses whitespace/comment information needed for formatter and LSP. The rowan CST is the correct foundation.
- **Using `Option<T>` returns from parse functions:** With first-error-only strategy, errors should halt parsing immediately (return `Err` or set an error flag). Don't try to return partial AST nodes from failed parses.
- **Grammar-level left recursion:** Rust has no tail call optimization. Left recursion in recursive descent causes stack overflow. Use Pratt parsing for all expression precedence.
- **Treating newlines as tokens in the Pratt loop:** The Pratt expression parser should never see Newline tokens. Filter them out in the `current()` and `nth()` lookahead methods based on context.
- **Hardcoding line/column in errors:** Use byte-offset Spans (already in the codebase). LineIndex conversion happens only at error display time.

## Don't Hand-Roll

Problems that look simple but have existing solutions:

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Lossless syntax tree with parent pointers | Custom tree with parent Rc/Weak refs | rowan 0.16.1 | Green/red tree model solves parent pointers, text ranges, deduplication; ~5K lines of battle-tested code |
| Operator precedence parsing | Nested recursive descent functions per precedence level | Pratt parser (~100 lines) | Pratt is simpler, more maintainable, easier to add new operators; avoids deep recursion |
| Syntax kind enum | Manual u16 conversion | `#[repr(u16)]` enum + rowan's `Language` trait | rowan handles the type tag system; just map TokenKind variants to SyntaxKind |
| Snapshot testing | Custom AST printing + diff | insta (already in workspace) | YAML snapshots of CST/AST are easy to review and update |
| Source location tracking | Manual line/column threading | Span (from snow-common) + rowan TextRange | rowan computes text ranges from its tree structure; Span/LineIndex already exist |

**Key insight:** The CST layer (rowan) and expression parsing (Pratt) are the two areas where custom solutions are most tempting and most wasteful. Rowan saves thousands of lines; Pratt saves grammar complexity.

## Common Pitfalls

### Pitfall 1: SyntaxKind Explosion

**What goes wrong:** The SyntaxKind enum grows to 200+ variants, becoming hard to maintain and making token-to-node-kind mapping confusing.
**Why it happens:** Every tiny syntactic element gets its own node kind.
**How to avoid:** Keep node kinds at the semantic level, not the syntactic level. Use `BINARY_EXPR` with the operator token inside, not `ADD_EXPR`, `SUB_EXPR`, `MUL_EXPR`. Use `LITERAL` with the literal token inside, not `INT_LITERAL_EXPR`, `FLOAT_LITERAL_EXPR`.
**Warning signs:** More than ~60-80 node kinds (token kinds don't count). Compare: rust-analyzer has ~130 node kinds for all of Rust.

### Pitfall 2: Newline Handling Inconsistency

**What goes wrong:** Some code paths skip newlines, others don't, leading to "unexpected newline" errors for valid-looking code.
**Why it happens:** Newline skipping logic is duplicated across parse functions instead of centralized.
**How to avoid:** Centralize newline handling in `current()`, `nth()`, and `advance()`. These methods should skip insignificant newlines transparently. Parse functions should never directly check for Newline tokens except when explicitly consuming significant newlines as statement separators.
**Warning signs:** Multiple places in the parser code that check `TokenKind::Newline`.

### Pitfall 3: Forgetting to Track Delimiter Depth

**What goes wrong:** Newlines inside `()`, `[]`, or `{}` are treated as significant, breaking multi-line function calls and data structures.
**Why it happens:** `paren_depth` / `bracket_depth` / `brace_depth` not incremented/decremented properly in all code paths.
**How to avoid:** Increment/decrement depth in `advance()` when consuming delimiter tokens. Use a helper that handles the open/close pair: `fn parse_delimited(&mut self, open: TokenKind, close: TokenKind, body: impl FnOnce(&mut Self))`.
**Warning signs:** Multi-line argument lists or array literals fail to parse.

### Pitfall 4: Pipe Operator Precedence

**What goes wrong:** `a |> b |> c(d)` parses incorrectly because pipe precedence interacts with function call precedence.
**Why it happens:** The pipe operator is unusual -- its RHS is not a normal expression but a function reference or partial call.
**How to avoid:** Give pipe operator the lowest expression-level binding power (above assignment, below everything else). The RHS of `|>` is parsed as a normal expression -- `b` and `c(d)` are both valid RHS expressions. The semantic rewriting (`x |> f(y)` to `f(x, y)`) happens in a later phase (type checking or lowering), NOT in the parser. The parser just builds `PIPE_EXPR(lhs, rhs)`.
**Warning signs:** `a |> b(c) |> d` doesn't produce the expected left-associative tree.

### Pitfall 5: Trailing Closure Ambiguity

**What goes wrong:** `list.map() do |x| x + 1 end` -- the `do ... end` block is not attached to the call.
**Why it happens:** The parser finishes the call expression at `)` and doesn't look ahead for `do`.
**How to avoid:** After parsing a call's argument list `)`, check if the next non-newline token is `do`. If so, parse the trailing closure as part of the call expression. This is a special case in the Pratt postfix handling, not a general rule.
**Warning signs:** Trailing closures parse as separate expressions from the function call.

### Pitfall 6: String Interpolation AST Confusion

**What goes wrong:** The lexer emits StringStart/StringContent/InterpolationStart/.../InterpolationEnd/StringContent/StringEnd, but the parser doesn't group these into a coherent AST node.
**Why it happens:** String interpolation spans multiple tokens with different nesting contexts.
**How to avoid:** The parser should recognize StringStart and parse the entire string (including all interpolated expressions) as a single STRING_EXPR node. Inside, each `${...}` section produces an INTERPOLATION child node containing an expression. Each StringContent token produces a STRING_LITERAL child. The result is:
```
STRING_EXPR
  StringStart
  STRING_LITERAL (StringContent token)
  INTERPOLATION
    InterpolationStart
    <expr>
    InterpolationEnd
  STRING_LITERAL (StringContent token)
  StringEnd
```
**Warning signs:** Interpolated strings have flat, unsegmented AST nodes.

### Pitfall 7: do/end Block Scope in Expressions

**What goes wrong:** `if condition do x end` -- the `do ... end` block is parsed separately from the `if` expression, or `end` is not matched to the correct `do`.
**Why it happens:** `do/end` blocks appear in many contexts (fn def, module, if/else, case, trailing closure) and the parser doesn't track which `do` each `end` closes.
**How to avoid:** Each construct that uses `do/end` handles its own `do` and `end` tokens. The parser tracks the span of the `do` token so that if `end` is missing, the error references where `do` was. With first-error-only, there's no complex recovery needed -- just stop and report the span.
**Warning signs:** Nested do/end blocks misalign their open/close pairs.

## Code Examples

### Public API: parse function

```rust
// lib.rs -- the main entry point
pub struct Parse {
    green: rowan::GreenNode,
    errors: Vec<ParseError>,
}

impl Parse {
    pub fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }

    pub fn errors(&self) -> &[ParseError] {
        &self.errors
    }
}

pub fn parse(source: &str) -> Parse {
    let tokens: Vec<Token> = snow_lexer::Lexer::tokenize(source);
    let (green, errors) = parser::parse_source_file(&tokens, source);
    Parse { green, errors }
}
```

### Typed AST Node Example

```rust
// ast/item.rs
use crate::cst::{SyntaxNode, SyntaxKind};

pub struct FnDef {
    syntax: SyntaxNode,
}

impl FnDef {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == SyntaxKind::FN_DEF {
            Some(Self { syntax: node })
        } else {
            None
        }
    }

    pub fn name(&self) -> Option<SyntaxToken> {
        self.syntax.children_with_tokens()
            .filter_map(|it| it.into_token())
            .find(|t| t.kind() == SyntaxKind::IDENT)
    }

    pub fn param_list(&self) -> Option<ParamList> {
        self.syntax.children()
            .find_map(ParamList::cast)
    }

    pub fn body(&self) -> Option<Block> {
        self.syntax.children()
            .find_map(Block::cast)
    }

    pub fn visibility(&self) -> Option<SyntaxToken> {
        self.syntax.children_with_tokens()
            .filter_map(|it| it.into_token())
            .find(|t| t.kind() == SyntaxKind::PUB_KW)
    }

    pub fn return_type(&self) -> Option<TypeAnnotation> {
        self.syntax.children()
            .find_map(TypeAnnotation::cast)
    }
}
```

### Item Parsing Example

```rust
// parser/items.rs
fn parse_fn_def(p: &mut Parser) {
    let m = p.open();

    // Optional visibility: pub
    if p.current() == TokenKind::Pub {
        p.advance();
    }

    // fn or def keyword
    p.expect(TokenKind::Fn);  // or TokenKind::Def

    // Function name
    p.expect(TokenKind::Ident);

    // Parameter list
    parse_param_list(p);

    // Optional return type: -> Type
    if p.current() == TokenKind::Arrow {
        parse_return_type(p);
    }

    // Body: do ... end
    let do_span = p.current_span();
    p.expect(TokenKind::Do);
    parse_block_body(p);
    if !p.expect(TokenKind::End) {
        p.error_with_related(
            "expected `end` to close function body",
            do_span,
            "function body started here",
        );
        return;
    }

    p.close(m, SyntaxKind::FN_DEF);
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Hand-written AST enums with Box<T> | Lossless CST (rowan) + typed AST layer | ~2018 (rust-analyzer) | Enables tooling (formatter, LSP, refactoring) from the same tree; error tolerance built-in |
| Parser generators (LALR, PEG) | Hand-written recursive descent + Pratt | Industry consensus by ~2020 | Better error messages, incremental parsing support, full control over recovery |
| Semicolons/newlines in lexer | Parser-level newline significance | Kotlin ~2016, adopted widely | More flexible formatting, fewer "unexpected newline" surprises |
| Separate error AST vs success AST | CST that represents both valid and invalid trees uniformly | ~2018 (rowan, red-green trees) | No need for separate "error" AST variants; any node can have error children |

**Deprecated/outdated:**
- Parser generators (yacc, bison, lalrpop) for language compilers: hand-written parsers are now consensus for production compilers (rustc, GCC, Clang, Go, Swift, TypeScript, Kotlin all use hand-written parsers)
- Direct AST construction in parser: the CST-first approach is now standard for any language that plans tooling

## Open Questions

Things that couldn't be fully resolved:

1. **`fn` vs `def` keyword for function definitions**
   - What we know: CONTEXT.md mentions `pub fn new()` in the full program snapshot; token list has both `Fn` and `Def` keywords
   - What's unclear: Whether Snow uses `fn` (Rust-style), `def` (Elixir/Python-style), or both (e.g., `fn` for anonymous functions, `def` for named functions like Elixir)
   - Recommendation: Look at the Phase 1 test snapshots -- the full_program snapshot uses `pub fn`. Treat `fn` as the named function keyword for now. `def` may be an alternative or reserved for future use. The parser should handle both and the typed AST can normalize them.

2. **Struct update syntax with bare `|`**
   - What we know: Phase 1 decision says "Bare pipe | produces Error token; struct update syntax needs parser-level handling"
   - What's unclear: How the parser should handle `%State{state | count: new_count}` since `|` is an Error token from the lexer
   - Recommendation: The parser can recognize the `%` `Ident` `{` pattern and switch to a struct literal/update parsing mode where `Error` tokens with text `"|"` are treated as the update separator. Alternatively, reconsider whether the lexer should emit a distinct `Bar` token for `|` instead of Error. This may require a small lexer patch.

3. **Type annotation syntax in parser**
   - What we know: The token list has `ColonColon` (`::`) for type annotations, seen in `count :: Int` in snapshots
   - What's unclear: Whether type annotations are fully parsed in Phase 2 or left as opaque token sequences for Phase 3
   - Recommendation: Parse type annotations structurally (as paths, possibly with generic parameters `Type[A, B]`) but don't validate them semantically. The parser should build TYPE_ANNOTATION nodes with child nodes for the type path. This gives Phase 3 clean input.

4. **`case` vs `match` keyword distinction**
   - What we know: Both `Case` and `Match` are reserved keywords in the lexer
   - What's unclear: Whether they serve different purposes or are aliases
   - Recommendation: Parse both as case/match expressions with identical syntax: `case expr do pattern -> body ... end`. If they're semantically different, that's a Phase 3+ concern.

## Sources

### Primary (HIGH confidence)
- [rowan 0.16.1](https://docs.rs/rowan/0.16.1/rowan/) - Lossless syntax tree library API
- [matklad: Simple but Powerful Pratt Parsing](https://matklad.github.io/2020/04/13/simple-but-powerful-pratt-parsing.html) - Pratt parsing algorithm reference
- [matklad: Resilient LL Parsing Tutorial](https://matklad.github.io/2023/05/21/resilient-ll-parsing-tutorial.html) - Event-based parser architecture, marker pattern, error recovery
- [rust-analyzer syntax crate](https://rust-lang.github.io/rust-analyzer/syntax/index.html) - Two-layer CST + AST architecture reference
- Existing codebase: `snow-common/src/token.rs` (85 TokenKind variants), `snow-lexer/src/lib.rs` (Lexer iterator API), test snapshots

### Secondary (MEDIUM confidence)
- [Thunderseethe: Resilient Recursive Descent Parsing](https://thunderseethe.dev/posts/parser-base/) - Practical rowan + recursive descent walkthrough
- [Kotlin Newline Handling](https://gitar.ai/blog/parsing-kotlin) - Parser-level newline significance approach
- [Go Line Break Rules](https://go101.org/article/line-break-rules.html) - Lexer-level semicolon insertion rules (contrast with Snow's approach)
- [Emmanuel Bastien: Parsing a Programming Language in Rust](https://ebastien.name/posts/parsing-in-rust/) - CST-first with rowan practical guide
- [OXC: Bumpalo for AST](https://oxc.rs/docs/learn/parser_in_rust/ast) - Arena allocation patterns for Rust parsers (considered but not recommended over rowan)

### Tertiary (LOW confidence)
- [dev.to: Lossless Syntax Trees](https://dev.to/cad97/lossless-syntax-trees-280c) - Conceptual overview of CST/LST terminology
- [Jeffrey Kegler: Significant newlines? Or semicolons?](http://jeffreykegler.github.io/Ocean-of-Awareness-blog/individual/2014/02/semantic_ws.html) - Historical context on newline significance approaches

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - rowan is the established choice; Pratt parsing is well-understood
- Architecture: HIGH - rust-analyzer's two-layer approach is thoroughly documented and proven
- Pitfalls: HIGH - common parser pitfalls are well-documented in multiple sources
- Discretion recommendations: HIGH - each recommendation aligns with strong ecosystem consensus
- Newline handling: MEDIUM - Snow's specific rules are custom (not copied from any language); may need iteration
- Open questions (struct update `|`, fn vs def): MEDIUM - needs clarification but can be resolved during implementation

**Research date:** 2026-02-06
**Valid until:** 2026-03-06 (30 days -- stable domain, rowan unlikely to change significantly)
