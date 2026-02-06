# Phase 1: Project Foundation & Lexer - Research

**Researched:** 2026-02-06
**Domain:** Rust workspace setup, LLVM 18 build infrastructure, snapshot testing, hand-written lexer design
**Confidence:** HIGH

## Summary

This phase establishes the foundation for the Snow compiler: a Cargo workspace with LLVM 18 pinned, insta-based snapshot testing, and a hand-written lexer that tokenizes all Snow syntax with span tracking and error recovery.

The standard approach is well-established: hand-written lexers are the universal choice for production compilers (rustc, GCC, Clang, Go, Swift). The lexer is conceptually straightforward (~500-1000 lines of Rust) but has two areas of notable complexity: (1) string interpolation with `${expr}`, which requires a stack-based state machine to handle nested contexts, and (2) nestable block comments `#= =#`, which require a depth counter. Both are solved problems with known patterns.

The build infrastructure centers on Cargo workspaces with LLVM 18 via Homebrew on macOS, with the `LLVM_SYS_180_PREFIX` environment variable pointing at the LLVM installation. The insta snapshot testing library (v1.46.1) is the de facto standard for compiler testing in Rust, providing YAML-serialized snapshots of token streams that are easy to review and diff.

**Primary recommendation:** Build a multi-crate Cargo workspace from day one, pin LLVM 18 via `LLVM_SYS_180_PREFIX`, use insta for snapshot testing, and implement a hand-written lexer with a stack-based state machine for string interpolation contexts.

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions

**Token vocabulary:**
- Elixir-inspired keywords adapted for static typing: `fn`, `let`, `struct`, `trait`, `module`, `import`, `if`/`else`, `case`/`match`, `do`/`end`, `receive`, `spawn`, etc.
- Full operator set: arithmetic (+, -, *, /), comparison (==, !=, <, >, <=, >=), logical (&&, ||, !), pipe (|>), range (..), string concat (<>), list concat (++), match (=), arrow (->), fat arrow (=>), type annotation (::)
- Reserve all planned keywords upfront (including future-phase keywords like `spawn`, `receive`, `supervisor`, `send`, `link`, `monitor`, `trap`, `cond`, `with`) to prevent identifier conflicts in later phases

**String & interpolation syntax:**
- Double-quoted strings ("hello") for single-line
- Triple-quoted strings ("""multi\nline""") for multi-line
- No single-quoted strings
- Interpolation syntax: `${expr}` (JS/Kotlin style)
- Both double-quoted and triple-quoted strings support interpolation
- Arbitrary expressions allowed inside `${}` (not just identifiers)
- Lexer emits interpolation as a sequence of string-part and expression tokens

**Comment & whitespace rules:**
- Line comments: `#`
- Block comments: `#= ... =#` (nestable, Julia-style)
- Whitespace is NOT significant -- blocks use `do`/`end` delimiters
- Newlines are statement terminators (like Elixir/Go)
- Multi-line expressions continue across newlines when inside parens, brackets, or after operators

**Numeric literals:**
- Integer formats: decimal (42), hex (0xFF), binary (0b1010), octal (0o777)
- Underscore separators allowed: 1_000_000, 0xFF_FF, 0b1111_0000
- Float syntax: basic (3.14) and scientific notation (1.0e10, 2.5e-3)

### Claude's Discretion
- Atom syntax decision (atoms vs enum-only approach)
- Doc comment syntax choice
- Distinct Int/Float types vs single Number type
- Exact keyword list (the full set beyond what was discussed)
- Error recovery strategy details
- Token representation internals

### Deferred Ideas (OUT OF SCOPE)
None -- discussion stayed within phase scope
</user_constraints>

## Discretion Decisions (Research Recommendations)

These are areas marked as Claude's Discretion where research informs a recommendation.

### Atoms vs Enum-Only Approach

**Recommendation: No atoms. Use enums (sum types) exclusively.**

Rationale:
- Atoms in Elixir are open-world -- any atom can appear anywhere. This fundamentally conflicts with static type inference and exhaustiveness checking. Elixir's own efforts to add static typing treat atoms as singleton types within set-theoretic types, which is a research-level challenge.
- Snow has static HM type inference. Closed sum types (enums/ADTs) integrate cleanly with HM, enabling exhaustiveness checking, pattern matching compilation, and typed actor messages.
- Gleam (statically typed BEAM language, Rust compiler) made the same choice: no atoms as a first-class open type, only closed variants.
- For common "tag" patterns like `:ok` / `:error`, Snow should use `Result[T, E]` and `Option[T]` sum types, which are more typesafe.
- If atom-like ergonomics are desired later, they can be added as a sugar over zero-field enum variants (e.g., `Color.Red` instead of `:red`).
- **No atom tokens needed in the lexer.** This simplifies tokenization.

**Confidence: HIGH** -- This is the established approach for statically typed functional languages (Haskell, OCaml, Rust, Gleam).

### Doc Comment Syntax

**Recommendation: Use `##` for doc comments and `##!` for module-level docs.**

Rationale:
- Snow uses `#` for line comments, so doc comments should extend the `#` family.
- `##` is visually distinct from `#` (regular comment) and is easy to type.
- `##!` for module-level documentation parallels Rust's `//!` pattern within the `#` family.
- Alternative considered: `@doc """..."""` (Elixir-style) -- rejected because it requires the parser to handle documentation as an attribute, which is more complex and less natural in a non-macro language.
- Alternative considered: `///` (Gleam/Rust-style) -- rejected because Snow uses `#` for comments, not `//`. Mixing comment styles would be confusing.
- The lexer should emit `DocComment` and `ModuleDocComment` tokens so that documentation can be extracted by tooling.

**Confidence: MEDIUM** -- This is a design choice without strong precedent for the `#`-comment family specifically. The pattern is sound but the exact syntax (`##` vs `#!` vs other) could reasonably vary.

### Distinct Int/Float Types

**Recommendation: Distinct `Int` and `Float` types, not a single `Number` type.**

Rationale:
- HM type inference works best with distinct types -- a single `Number` type either loses information or requires type classes for numeric operations.
- Distinct types enable the compiler to use LLVM's integer vs floating-point instructions correctly without runtime type checks.
- Elixir has distinct integers and floats. Rust, OCaml, Haskell all have distinct numeric types. JavaScript's single `Number` type is widely considered a design mistake.
- Integer arithmetic can be exact (arbitrary precision or 64-bit), while float arithmetic is IEEE 754. Conflating them hides precision issues.
- The lexer should emit `IntLiteral` and `FloatLiteral` as distinct token kinds based on the presence of a decimal point or exponent.
- Integer literals (decimal, hex, binary, octal) always produce `IntLiteral`. Float literals (with `.` or `e`/`E`) always produce `FloatLiteral`.

**Confidence: HIGH** -- Universal choice in statically typed compiled languages.

### Complete Keyword List

**Recommendation: The following complete keyword set, covering all 10 phases.**

Phase 1 (Foundation):
- `fn`, `let`, `do`, `end`, `if`, `else`, `case`, `match`, `when`

Phase 2 (Parser):
- `module`, `import`, `struct`, `trait`, `impl`, `pub`, `def`, `type`, `alias`
- `for`, `in`, `return`, `and`, `or`, `not`, `true`, `false`, `nil`
- `pipe` (reserved, may not be a keyword in practice)

Phase 3 (Type System):
- `where` (type constraints)

Phase 4 (Pattern Matching):
- `cond`, `with`

Phase 5 (Codegen):
- (no new keywords)

Phase 6-7 (Actors & Supervision):
- `spawn`, `send`, `receive`, `after`, `self`
- `supervisor`, `link`, `monitor`, `trap`

Phase 8-9 (Standard Library):
- (no new keywords -- library-level constructs)

Phase 10 (Tooling):
- (no new keywords)

**Full reserved keyword list (37 keywords):**
```
after, alias, and, case, cond, def, do, else, end, false, fn, for,
if, impl, import, in, let, link, match, module, monitor, nil, not,
or, pub, receive, return, self, send, spawn, struct, supervisor,
trait, trap, true, type, when, where, with
```

**Confidence: MEDIUM** -- The core keywords are well-established from the CONTEXT.md. Some keywords (`alias`, `impl`, `def`, `for`, `return`, `where`) are informed guesses based on the language design. The exact set may be adjusted during later phases, but reserving them now prevents breaking changes.

### Error Recovery Strategy

**Recommendation: Panic-mode recovery at the lexer level.**

The lexer should use a simple but effective error recovery strategy:
1. When an invalid character or malformed token is encountered, emit an `Error` token containing the problematic byte(s) and the error kind.
2. Advance past the problematic character(s) and resume normal lexing.
3. Never panic or abort -- the lexer always produces a complete token stream.
4. Collect errors in a `Vec<LexError>` alongside the token stream.
5. The parser can then decide how to handle error tokens (skip them, use them for synchronization, etc.).

Specific recovery cases:
- **Unterminated string:** Emit `Error(UnterminatedString)`, consume to end of line or file.
- **Invalid escape sequence:** Emit the string token with an error flag; don't abort string lexing.
- **Unterminated block comment:** Emit `Error(UnterminatedBlockComment)`, consume to EOF.
- **Invalid number literal** (e.g., `0xGG`): Emit `Error(InvalidNumberLiteral)` for the malformed portion.
- **Unexpected character:** Emit `Error(UnexpectedCharacter)` for the single byte, advance one byte.

**Confidence: HIGH** -- This is the standard approach. rustc_lexer stores errors as flags on tokens rather than aborting.

### Token Representation Internals

**Recommendation: Small, flat `Token` struct with `TokenKind` enum and byte-offset `Span`.**

```rust
/// A single token from the source.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

/// Byte range in source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// Start byte offset (inclusive).
    pub start: u32,
    /// End byte offset (exclusive).
    pub end: u32,
}

/// Line/column information, computed on demand from byte offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineCol {
    pub line: u32,   // 1-based
    pub col: u32,    // 1-based, in bytes (not chars)
}
```

Key design decisions:
- **Byte offsets, not line/column in Token.** Store only byte offsets in the token. Line/column is computed on demand from a `LineIndex` table (a sorted vec of line-start byte offsets). This is how rustc, rust-analyzer, and most modern compilers work. It keeps tokens small (8 bytes for `Span` instead of 16+ bytes).
- **`u32` not `usize` for offsets.** Limits source files to 4GB, which is fine. Keeps `Span` at 8 bytes, `Token` at ~12 bytes (with padding).
- **`TokenKind` as a fieldless enum for most variants.** Literal values and identifiers store their text as a byte range (the span IS the text). String interning via lasso happens at a higher level (parser or shared context), not in the lexer itself.
- **Copy semantics.** `Token` is small enough to be `Copy`, enabling efficient passing without allocation.

**Confidence: HIGH** -- This is the standard representation used by rustc, OXC (JavaScript parser), and other high-performance compilers.

## Standard Stack

The established libraries/tools for this phase:

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| Rust (stable) | latest stable | Compiler implementation language | Project constraint |
| Cargo workspace | N/A | Multi-crate project organization | Standard Rust practice for multi-crate projects |
| LLVM | 18 | Code generation backend (later phases) | Pinned per CONTEXT.md; available via `brew install llvm@18` |
| inkwell | 0.8.0 | Safe LLVM Rust bindings | Standard choice for Rust LLVM projects; supports LLVM 11-21 |

### Testing
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| insta | 1.46.1 | Snapshot testing | De facto standard for compiler testing in Rust; 2.2M downloads/month |
| cargo-insta | latest | Snapshot review CLI | Companion tool for reviewing snapshot changes |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| lasso | 0.7.3 | String interning | Intern identifiers and keywords for O(1) comparison; used in parser/later phases |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| inkwell 0.8.0 | inkwell 0.7.1 (crates.io stable) | 0.7.1 supports LLVM 8-21; 0.8.0 (Jan 2026) drops LLVM <11. Use 0.8.0 if available on crates.io, fallback to git dependency |
| insta (YAML) | insta (debug) | YAML is more readable in diffs; debug format is simpler but noisier |
| lasso | string-interner | lasso has more features (ThreadedRodeo) and wider adoption |

**Workspace Cargo.toml (Phase 1):**
```toml
[workspace]
members = ["crates/*"]
resolver = "2"

[workspace.dependencies]
insta = { version = "1", features = ["yaml"] }

[profile.dev.package]
insta.opt-level = 3
similar.opt-level = 3
```

**crates/snow-lexer/Cargo.toml:**
```toml
[package]
name = "snow-lexer"
version = "0.1.0"
edition = "2021"

[dependencies]
# Minimal dependencies -- hand-written lexer needs nothing

[dev-dependencies]
insta = { workspace = true }
```

**crates/snow-common/Cargo.toml:**
```toml
[package]
name = "snow-common"
version = "0.1.0"
edition = "2021"

[dependencies]
# Shared types: Span, TokenKind, diagnostics
```

**LLVM 18 Build Setup (macOS):**
```bash
# Install LLVM 18
brew install llvm@18

# Set environment variable (add to ~/.zshrc)
export LLVM_SYS_180_PREFIX="$(brew --prefix llvm@18)"

# Verify
$(brew --prefix llvm@18)/bin/llvm-config --version
# Should print: 18.x.x
```

**LLVM 18 Build Setup (Linux/Ubuntu):**
```bash
# Install LLVM 18
sudo apt install llvm-18-dev libpolly-18-dev

# Set environment variable
export LLVM_SYS_180_PREFIX=/usr/lib/llvm-18
```

## Architecture Patterns

### Recommended Project Structure (Phase 1)
```
snow/
  Cargo.toml                    # workspace root
  crates/
    snow-common/                # shared types: Span, TokenKind, errors
      src/
        lib.rs
        span.rs                 # Span, LineIndex
        token.rs                # TokenKind enum, Token struct
        error.rs                # LexError, diagnostic types
    snow-lexer/                 # tokenization
      src/
        lib.rs                  # Lexer struct, public API
        cursor.rs               # Low-level char/byte iteration
      tests/
        lexer_tests.rs          # insta snapshot tests
        snapshots/              # generated by insta
    snowc/                      # binary entry point (stub for now)
      src/
        main.rs
  tests/
    fixtures/                   # .snow test files for integration tests
      keywords.snow
      operators.snow
      strings.snow
      interpolation.snow
      comments.snow
      numbers.snow
      error_recovery.snow
```

### Pattern 1: Cursor-Based Lexer with State Stack

**What:** The lexer uses a `Cursor` struct that wraps the source text and provides low-level character-by-character iteration with byte offset tracking. The main `Lexer` struct uses the cursor and maintains a state stack for handling nested contexts (string interpolation, block comments).

**When to use:** Always -- this is the standard pattern for hand-written lexers.

**Example:**
```rust
// Source: rustc_lexer pattern, adapted for Snow

/// Low-level source text cursor.
struct Cursor<'src> {
    /// Source text being lexed.
    source: &'src str,
    /// Current byte position.
    pos: u32,
}

impl<'src> Cursor<'src> {
    fn peek(&self) -> Option<char> {
        self.source[self.pos as usize..].chars().next()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += ch.len_utf8() as u32;
        Some(ch)
    }

    fn peek_second(&self) -> Option<char> {
        let mut chars = self.source[self.pos as usize..].chars();
        chars.next();
        chars.next()
    }

    fn eat_while(&mut self, predicate: impl Fn(char) -> bool) {
        while self.peek().is_some_and(&predicate) {
            self.advance();
        }
    }
}
```

### Pattern 2: State Stack for String Interpolation

**What:** String interpolation `${expr}` requires the lexer to switch between string-content mode and normal-expression mode. A stack tracks the current lexing context.

**When to use:** When lexing strings with interpolation.

**Example:**
```rust
/// Lexer context stack for handling nested interpolation.
#[derive(Debug, Clone, Copy, PartialEq)]
enum LexerState {
    /// Normal top-level or expression context.
    Normal,
    /// Inside a double-quoted string, tracking interpolation depth.
    InString,
    /// Inside a `${...}` interpolation expression.
    /// The u32 tracks brace nesting depth within the interpolation.
    InInterpolation { brace_depth: u32 },
    /// Inside a triple-quoted string.
    InTripleString,
}

struct Lexer<'src> {
    cursor: Cursor<'src>,
    state_stack: Vec<LexerState>,
    errors: Vec<LexError>,
}

impl<'src> Lexer<'src> {
    fn next_token(&mut self) -> Token {
        match self.current_state() {
            LexerState::Normal => self.lex_normal(),
            LexerState::InString => self.lex_string_content(),
            LexerState::InTripleString => self.lex_triple_string_content(),
            LexerState::InInterpolation { .. } => self.lex_normal(),
        }
    }

    fn lex_string_content(&mut self) -> Token {
        let start = self.cursor.pos;
        loop {
            match self.cursor.peek() {
                Some('"') => {
                    // End of string
                    let span = Span::new(start, self.cursor.pos);
                    if start < self.cursor.pos {
                        return Token::new(TokenKind::StringContent, span);
                    }
                    self.cursor.advance();
                    self.state_stack.pop();
                    return Token::new(TokenKind::StringEnd, Span::new(start, self.cursor.pos));
                }
                Some('$') if self.cursor.peek_second() == Some('{') => {
                    // Start interpolation
                    if start < self.cursor.pos {
                        return Token::new(
                            TokenKind::StringContent,
                            Span::new(start, self.cursor.pos),
                        );
                    }
                    self.cursor.advance(); // consume $
                    self.cursor.advance(); // consume {
                    self.state_stack.push(LexerState::InInterpolation { brace_depth: 1 });
                    return Token::new(
                        TokenKind::InterpolationStart,
                        Span::new(start, self.cursor.pos),
                    );
                }
                Some('\\') => {
                    self.cursor.advance(); // consume backslash
                    self.cursor.advance(); // consume escaped char
                }
                Some(_) => {
                    self.cursor.advance();
                }
                None => {
                    // Unterminated string
                    self.errors.push(LexError::UnterminatedString(
                        Span::new(start, self.cursor.pos),
                    ));
                    self.state_stack.pop();
                    return Token::new(TokenKind::Error, Span::new(start, self.cursor.pos));
                }
            }
        }
    }
}
```

### Pattern 3: Newline-as-Terminator with Continuation Rules

**What:** Following Go's approach (adapted for Snow), the lexer inserts `Newline` tokens only when they would terminate a statement. Newlines inside paired delimiters or after operators that clearly continue an expression are suppressed.

**When to use:** Always -- this implements Snow's "newlines are statement terminators" rule.

**Rules for when a newline IS a statement terminator:**
- After an identifier
- After a literal (int, float, string)
- After `true`, `false`, `nil`
- After `)`, `]`, `}`
- After `end`
- After `return`

**Rules for when a newline is NOT a statement terminator (line continuation):**
- Inside unmatched `(`, `[`, `{`
- After a binary operator (`+`, `-`, `*`, `/`, `==`, `|>`, `->`, etc.)
- After a comma
- After `do`
- After `=`
- After `\` (explicit line continuation, optional)

**Implementation:** Track a bracket depth counter and the kind of the last non-whitespace token. When encountering a newline, check if the last token could end a statement AND the bracket depth is zero.

### Pattern 4: Nestable Block Comment with Depth Counter

**What:** `#= ... =#` block comments can nest. The lexer tracks nesting depth.

```rust
fn lex_block_comment(&mut self) -> Token {
    let start = self.cursor.pos;
    // Already consumed '#='
    let mut depth: u32 = 1;
    while depth > 0 {
        match self.cursor.peek() {
            Some('#') if self.cursor.peek_second() == Some('=') => {
                self.cursor.advance();
                self.cursor.advance();
                depth += 1;
            }
            Some('=') if self.cursor.peek_second() == Some('#') => {
                self.cursor.advance();
                self.cursor.advance();
                depth -= 1;
            }
            Some(_) => {
                self.cursor.advance();
            }
            None => {
                self.errors.push(LexError::UnterminatedBlockComment(
                    Span::new(start, self.cursor.pos),
                ));
                break;
            }
        }
    }
    Token::new(TokenKind::BlockComment, Span::new(start, self.cursor.pos))
}
```

### Anti-Patterns to Avoid

- **Storing line/column in every Token:** Wastes memory. Compute on demand from a `LineIndex` (vec of line-start byte offsets). Only build the `LineIndex` when error reporting is needed.
- **Returning `String` values from the lexer:** Token text should be a slice of the original source (`&str` via span), not a heap allocation. String interning happens in the parser, not the lexer.
- **Using a lexer generator (Logos) for this language:** String interpolation and nestable block comments require context-sensitive behavior that doesn't fit cleanly into a DFA-based generator. A hand-written lexer is simpler and gives full control.
- **Eager string interning in the lexer:** The lexer should not depend on lasso. It should produce raw spans. The parser or a shared compiler context does the interning. This keeps the lexer crate dependency-free.
- **Allocating a `Vec<Token>` upfront:** The lexer should be an iterator, producing tokens on demand. This keeps memory bounded and integrates cleanly with the parser.

## Don't Hand-Roll

Problems that look simple but have existing solutions:

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Snapshot testing | Custom expected-output comparison | insta 1.46.1 | Handles snapshot creation, diffing, review workflow, CI integration. The TUI reviewer (`cargo insta review`) is invaluable when output format changes. |
| String interning | Custom `HashMap<String, u32>` | lasso 0.7.3 (in parser phase) | Arena-allocated, provides both `Rodeo` (single-threaded) and `ThreadedRodeo`. Handles all edge cases around memory layout and comparison. |
| LLVM bindings | Raw llvm-sys calls | inkwell 0.8.0 (in codegen phase) | Type-safe wrapper catches errors at compile time vs runtime segfaults. |
| Unicode character classification | Manual `is_identifier_start` / `is_identifier_continue` | `unicode-xid` crate or std `char` methods | Unicode identifier rules (UAX #31) are complex. Use the standard implementation. |

**Key insight:** The lexer itself needs almost no dependencies. Its complexity is in the control flow logic (state machine, interpolation handling), not in library integrations. The temptation is to over-engineer the lexer; resist it. A correct, well-tested lexer with good error recovery is more valuable than a fast one.

## Common Pitfalls

### Pitfall 1: String Interpolation Breaking the Lexer's State Machine
**What goes wrong:** Nested interpolation like `"outer ${inner_fn("nested ${x}")} end"` causes the lexer to lose track of which string context it's in, producing garbled tokens.
**Why it happens:** Treating the lexer as a simple linear state machine without a stack. String interpolation is inherently recursive -- you need a stack to handle nesting.
**How to avoid:** Use a `Vec<LexerState>` stack. Push `InString` when entering a string, push `InInterpolation` when seeing `${`, pop when the interpolation closes with `}` (at brace_depth 0), pop again when the string closes with `"`.
**Warning signs:** Tests with nested string interpolation produce incorrect token sequences or panic.

### Pitfall 2: Off-by-One Errors in Span Tracking
**What goes wrong:** Spans point to the wrong byte offsets, causing error messages to underline the wrong characters. These bugs are invisible until error reporting is implemented in a later phase.
**Why it happens:** Forgetting to save the start position before advancing, or using inclusive vs exclusive end positions inconsistently.
**How to avoid:** Establish a convention (start-inclusive, end-exclusive is standard) and encode it in the `Span` type documentation. Write span-verification snapshot tests that include the original source text sliced by the span: `assert_eq!(&source[span.start..span.end], expected_text)`.
**Warning signs:** Snapshot tests show tokens with spans that don't match the expected text.

### Pitfall 3: Newline Handling with Multi-Character Operators
**What goes wrong:** The `|>` operator spans two characters. If the lexer sees `|` at end of line, it might interpret the newline as a statement terminator before seeing the `>` on the next line. Similarly, `->`, `=>`, `::`, `<=`, `>=`, `!=`, `==`, `&&`, `||`, `++`, `..`, `<>`.
**Why it happens:** Greedy matching of single-character tokens before checking for two-character tokens.
**How to avoid:** Always check for the longest possible match first. When seeing `|`, check if the next character is `>` before deciding it's a pipe character. When seeing `<`, check for `>` (diamond `<>`) and `=` (less-equal `<=`).
**Warning signs:** Operators that span line boundaries tokenize as two separate single-character tokens.

### Pitfall 4: LLVM Build Breaks on Clean Clone
**What goes wrong:** The project builds on the author's machine but fails on a clean checkout because LLVM 18 isn't found or the wrong version is picked up.
**Why it happens:** LLVM is not installed, `LLVM_SYS_180_PREFIX` is not set, or the system LLVM is a different version (macOS ships with a limited LLVM/Clang that isn't suitable).
**How to avoid:** Document exact setup steps in README. Provide a `.envrc` (for direnv) or `.cargo/config.toml` that sets the environment variable. Consider a `build.rs` that detects missing LLVM and prints a helpful error. Note: LLVM is not needed for Phase 1's lexer work -- only for later codegen phases. Structure the workspace so that `snow-lexer` and `snow-common` build without LLVM.
**Warning signs:** CI fails with "llvm-config not found" or "LLVM version mismatch".

### Pitfall 5: Conflating Lexer and Parser Responsibilities
**What goes wrong:** The lexer tries to validate syntax (e.g., checking that `if` is followed by an expression, or that strings are properly terminated within a statement). This makes the lexer overly complex and couples it to the grammar.
**Why it happens:** Unclear boundary between lexer and parser responsibilities.
**How to avoid:** The lexer's ONLY job is tokenization: break the byte stream into tokens with spans. It does NOT validate syntax, operator precedence, or expression structure. The only "validation" is at the character level: is this a valid number literal? Is this string terminated? Is this a known escape sequence?
**Warning signs:** The lexer has `if/else` chains that check for sequences of token kinds.

### Pitfall 6: Triple-Quoted String Edge Cases
**What goes wrong:** `"""` parsing conflicts with an empty string `""` followed by a quote, or triple-quoted strings don't handle `"` and `""` inside them correctly.
**Why it happens:** Insufficient lookahead. When seeing `"`, the lexer must look ahead to determine if it's `"` (start of string), `""` (empty string), or `"""` (start of triple-quoted string).
**How to avoid:** When the lexer sees `"`, peek ahead for `""`. If found, peek one more for `"""`. Implement as: see `"`, peek next. If next is `"`, peek after that. If that's also `"`, enter triple-quoted mode. Otherwise, it's an empty string `""`. Inside a triple-quoted string, `"` and `""` are literal content; only `"""` ends the string.
**Warning signs:** Empty strings or strings containing quotes produce unexpected tokens.

## Code Examples

### Complete TokenKind Enum (Verified against CONTEXT.md decisions)

```rust
/// All token types in the Snow language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    // -- Literals --
    IntLiteral,           // 42, 0xFF, 0b1010, 0o777, 1_000
    FloatLiteral,         // 3.14, 1.0e10, 2.5e-3
    StringStart,          // opening " of a string
    StringContent,        // text content between interpolations
    StringEnd,            // closing " of a string
    TripleStringStart,    // opening """ of a triple-quoted string
    TripleStringEnd,      // closing """ of a triple-quoted string
    InterpolationStart,   // ${
    InterpolationEnd,     // } (closing an interpolation)

    // -- Identifiers & Keywords --
    Identifier,           // user-defined names
    // Keywords (alphabetical)
    After,
    Alias,
    And,
    Case,
    Cond,
    Def,
    Do,
    Else,
    End,
    False,
    Fn,
    For,
    If,
    Impl,
    Import,
    In,
    Let,
    Link,
    Match,
    Module,
    Monitor,
    Nil,
    Not,
    Or,
    Pub,
    Receive,
    Return,
    SelfKw,               // `self` (SelfKw to avoid Rust keyword conflict)
    Send,
    Spawn,
    Struct,
    Supervisor,
    Trait,
    Trap,
    True,
    Type,
    When,
    Where,
    With,

    // -- Operators --
    Plus,                 // +
    Minus,                // -
    Star,                 // *
    Slash,                // /
    Eq,                   // =
    EqEq,                // ==
    NotEq,               // !=
    Lt,                   // <
    Gt,                   // >
    LtEq,                // <=
    GtEq,                // >=
    AmpAmp,              // &&
    PipePipe,            // ||
    Bang,                // !
    Pipe,                // |
    PipeArrow,           // |>
    DotDot,              // ..
    Diamond,             // <>
    PlusPlus,            // ++
    Arrow,               // ->
    FatArrow,            // =>
    ColonColon,          // ::

    // -- Delimiters --
    LParen,              // (
    RParen,              // )
    LBracket,            // [
    RBracket,            // ]
    LBrace,              // {
    RBrace,              // }
    Comma,               // ,
    Dot,                 // .
    Colon,               // :
    Semicolon,           // ;

    // -- Comments --
    LineComment,          // # ...
    BlockComment,         // #= ... =#
    DocComment,           // ## ...
    ModuleDocComment,     // ##! ...

    // -- Whitespace & Structure --
    Newline,              // significant newline (statement terminator)
    Eof,                  // end of file

    // -- Error --
    Error,                // invalid/unrecognized token
}
```

### Snapshot Test Example

```rust
// In crates/snow-lexer/tests/lexer_tests.rs
use snow_lexer::Lexer;

#[test]
fn test_keywords() {
    let source = "fn main do\n  let x = 42\nend";
    let tokens = Lexer::new(source).collect_tokens();
    insta::assert_yaml_snapshot!(tokens);
}

#[test]
fn test_string_interpolation() {
    let source = r#""hello ${name}, you are ${age + 1} years old""#;
    let tokens = Lexer::new(source).collect_tokens();
    insta::assert_yaml_snapshot!(tokens);
}

#[test]
fn test_nested_interpolation() {
    let source = r#""outer ${fn_call("inner ${x}")} end""#;
    let tokens = Lexer::new(source).collect_tokens();
    insta::assert_yaml_snapshot!(tokens);
}

#[test]
fn test_nestable_block_comment() {
    let source = "#= outer #= inner =# still comment =#";
    let tokens = Lexer::new(source).collect_tokens();
    insta::assert_yaml_snapshot!(tokens);
}

#[test]
fn test_error_recovery() {
    let source = "let x = @invalid + 42";
    let tokens = Lexer::new(source).collect_tokens();
    // Should contain an Error token for @ but still lex the rest
    insta::assert_yaml_snapshot!(tokens);
}

#[test]
fn test_all_operators() {
    let source = "+ - * / = == != < > <= >= && || ! |> .. <> ++ -> => ::";
    let tokens = Lexer::new(source).collect_tokens();
    insta::assert_yaml_snapshot!(tokens);
}

#[test]
fn test_numeric_literals() {
    let source = "42 0xFF 0b1010 0o777 1_000_000 3.14 1.0e10 2.5e-3";
    let tokens = Lexer::new(source).collect_tokens();
    insta::assert_yaml_snapshot!(tokens);
}
```

### LineIndex for On-Demand Line/Column Computation

```rust
/// Maps byte offsets to line/column information.
/// Built once per source file, used for error reporting.
pub struct LineIndex {
    /// Byte offset of the start of each line.
    /// line_starts[0] is always 0.
    line_starts: Vec<u32>,
}

impl LineIndex {
    pub fn new(source: &str) -> Self {
        let mut line_starts = vec![0u32];
        for (i, byte) in source.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push((i + 1) as u32);
            }
        }
        LineIndex { line_starts }
    }

    pub fn line_col(&self, byte_offset: u32) -> LineCol {
        let line = self.line_starts
            .partition_point(|&start| start <= byte_offset)
            - 1;
        let col = byte_offset - self.line_starts[line];
        LineCol {
            line: (line + 1) as u32,  // 1-based
            col: (col + 1) as u32,    // 1-based
        }
    }
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Lexer generators (lex/flex) | Hand-written lexers | Decades ago for production compilers | Full control over error recovery, performance, edge cases |
| Line/column stored per token | Byte offsets + LineIndex | ~2015 (rust-analyzer popularized) | 2-3x smaller token representation, faster lexing |
| Regex-based tokenization | Character-by-character state machine | Always for production | Regex can't handle context-sensitive features like interpolation |
| Separate snapshot files only | Inline snapshots + file snapshots (insta) | insta 1.x | Inline snapshots are more convenient for small tests |
| insta < 1.38 (manual install) | insta 1.38+ (prebuilt cargo-insta binaries) | ~2025 | Faster setup, no compilation of cargo-insta from source |

**Deprecated/outdated:**
- Logos as a production lexer for languages with interpolation: Logos 0.16 is excellent for simple tokenization but becomes limiting for context-sensitive features. Not recommended for Snow.
- Storing full source text per token: Never copy source text into tokens. Use spans into the original source.

## Open Questions

1. **Exact inkwell version availability on crates.io**
   - What we know: inkwell 0.8.0 was released January 9, 2026 on GitHub and lib.rs. docs.rs still shows 0.7.1 as latest.
   - What's unclear: Whether 0.8.0 is published to crates.io or only available via git dependency.
   - Recommendation: Try `inkwell = { version = "0.8.0", features = ["llvm18-0"] }` first. If it fails, use `inkwell = { git = "https://github.com/TheDan64/inkwell", features = ["llvm18-0"] }`. Note: inkwell is NOT needed for the lexer crate itself -- only for the codegen crate in later phases. The workspace can add it later.

2. **Unicode identifier support scope**
   - What we know: The CONTEXT.md does not mention Unicode identifiers specifically. Elixir supports UTF-8 identifiers.
   - What's unclear: Should Snow support Unicode identifiers (e.g., `nombre`, `zhongguo`) or ASCII-only?
   - Recommendation: Support Unicode identifiers from the start using Rust's `char::is_alphabetic()` and `char::is_alphanumeric()`. This is trivial in a hand-written lexer and matches Elixir's behavior. ASCII-only identifiers can always be a lint, not a lexer restriction.

3. **Exact newline-as-terminator rules**
   - What we know: Newlines are statement terminators. Continuation happens inside paired delimiters and after operators.
   - What's unclear: The exact token set that triggers continuation vs termination. Go has a precise spec; Snow needs one.
   - Recommendation: Start with Go's rule (insert newline-as-terminator after tokens that could end a statement) adapted for Snow's keywords. Document the rule precisely. The pattern will be refined during parser development.

## Sources

### Primary (HIGH confidence)
- [insta official docs](https://insta.rs/docs/) - Version 1.46.1, getting started, snapshot types, workspace config
- [insta GitHub](https://github.com/mitsuhiko/insta) - 2.2M downloads/month, de facto standard
- [inkwell GitHub](https://github.com/TheDan64/inkwell) - v0.8.0 released 2026-01-09, LLVM 11-21 support
- [inkwell lib.rs](https://lib.rs/crates/inkwell) - Version and feature verification
- [lasso crates.io](https://crates.io/crates/lasso) - v0.7.3, 5M+ downloads
- [Rust Compiler Dev Guide - Lexing](https://rustc-dev-guide.rust-lang.org/the-parser.html) - rustc_lexer design
- [OXC JavaScript Parser Lexer](https://oxc-project.github.io/javascript-parser-in-rust/docs/lexer/) - Token/Span design
- [LLVM@18 Homebrew formula](https://formulae.brew.sh/formula/llvm@18) - Installation verified
- [llvm-sys crate](https://crates.io/crates/llvm-sys) - LLVM_SYS_180_PREFIX documentation

### Secondary (MEDIUM confidence)
- [Go Specification - Semicolons](https://go.dev/ref/spec) - Automatic semicolon insertion rules (model for Snow)
- [Handmade WDL Lexer](https://nickgeorge.net/handmade-wdl-parsing-lexing-rust/) - String interpolation state stack pattern
- [Gleam Doc Comments](https://tour.gleam.run/functions/documentation-comments/) - `///` and `////` syntax
- [Elixir Syntax Reference](https://hexdocs.pm/elixir/syntax-reference.html) - Reserved words, operator precedence
- [Go Automatic Semicolon Insertion](https://medium.com/golangspec/automatic-semicolon-insertion-in-go-1990338f2649) - Detailed ASI rules

### Tertiary (LOW confidence)
- [Fast Lexing in Rust](https://alic.dev/blog/fast-lexing) - Performance optimization (not needed for Phase 1)

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - Verified versions of insta (1.46.1), lasso (0.7.3), inkwell (0.8.0). All well-established.
- Architecture: HIGH - Hand-written lexer with cursor/state-stack is universal pattern. Verified against rustc, OXC, WDL implementations.
- Pitfalls: HIGH - String interpolation complexity and span tracking are well-documented challenges with known solutions.
- Discretion decisions: MEDIUM to HIGH - Atoms (HIGH, matches Gleam precedent), doc comments (MEDIUM, design choice), Int/Float types (HIGH, universal in typed languages), keyword list (MEDIUM, some guesswork for future phases).

**Research date:** 2026-02-06
**Valid until:** 2026-03-06 (30 days -- stable domain, libraries unlikely to change)
