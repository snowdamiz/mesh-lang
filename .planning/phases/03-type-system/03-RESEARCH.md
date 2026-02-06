# Phase 3: Type System - Research

**Researched:** 2026-02-06
**Domain:** Hindley-Milner type inference, trait systems, type error diagnostics
**Confidence:** HIGH

## Summary

This phase implements a Hindley-Milner (HM) type inference engine for the Snow compiler. The engine must infer types for all expressions -- including function parameters -- without requiring annotations, support let-polymorphism, generics with trait constraints (where clauses), and built-in Option/Result types. Error messages must be concise but show both sides of type conflicts with fix suggestions.

The standard approach is constraint-based HM inference using the `ena` crate (0.14.3) for union-find, with Didier Remy's level-based generalization for efficient let-polymorphism. Type errors are rendered via `ariadne` (0.6.0) for multi-span labeled diagnostics. The type checker operates on the existing rowan-based CST/AST, producing a side-table mapping AST node IDs to inferred types.

**Primary recommendation:** Use a new `snow-typeck` crate implementing constraint-based Algorithm J (mutable union-find via `ena`) with level-based generalization, error provenance tracking via constraint origins, and `ariadne` for diagnostic rendering. The type checker walks the typed AST wrappers from `snow-parser`, not the raw CST.

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions

#### Type annotation syntax
- Angle brackets for generics: `List<T>`, `Option<Int>`, `Result<String, Error>`
- Function params always annotated, return type optional: `fn add(x: Int, y: Int) -> Int` or `fn add(x: Int, y: Int)`
- Return type uses arrow syntax: `-> Int`
- Option sugar: `Int?` means `Option<Int>`
- Result sugar: `T!E` means `Result<T, E>`

#### Inference boundaries
- Full inference for function parameters -- `fn add(x, y)` is valid, types inferred from usage
- Struct field types always annotated in the definition -- `struct User do name: String, age: Int end`
- When inference fails or is ambiguous: hard error with suggestion of what annotation to add ("Cannot infer type of x. Try adding: x: Int")
- No numeric literal defaults -- ambiguous numeric types are errors like everything else
- No wildcard type annotation (`_`) -- either annotate with a real type or leave it off

#### Error experience
- Elm-level thoroughness with minimal (Go-like) tone -- concise messages, no conversational framing, but still show both sides of conflicts and suggest fixes
- Show endpoints only for inference chains -- "expected Int, found String" with locations of both, no full inference trace
- Always suggest fixes when a plausible fix exists (Option wrapping, missing trait impl, type coercion, etc.)
- Error format: terse one-liner with labeled source spans, not paragraphs of explanation

#### Trait & generic design
- `interface` keyword for trait definitions: `interface Printable do fn to_string(self) -> String end`
- Where clause for generic constraints: `fn foo<T>(x: T) where T: Printable` -- no inline bounds
- Option and Result are fully built-in -- compiler has deep awareness for sugar (Int?, T!E), optimizations, better error messages, and automatic propagation

### Claude's Discretion
- Implementation syntax for interfaces (impl block style, keyword choice)
- Internal type representation (ena-based union-find vs other approaches)
- Unification algorithm details
- How propagation operator (like Rust's `?`) looks syntactically -- if included in this phase at all

### Deferred Ideas (OUT OF SCOPE)
None -- discussion stayed within phase scope
</user_constraints>

## Standard Stack

The established libraries and tools for this domain.

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| `ena` | 0.14.3 | Union-find for type variable unification | Extracted from rustc; the Rust ecosystem standard for type inference union-find. Used by rustc, chalk, and most Rust-based type checkers |
| `ariadne` | 0.6.0 | Diagnostic error rendering with multi-span labels | Best visual output of the three major diagnostic crates (ariadne, codespan-reporting, miette). Multi-line labels, color generation, overlap heuristics |
| `rowan` | 0.16 (already in workspace) | CST access for type-checking pass | Already used by parser; type checker reads AST via existing typed wrappers |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| `insta` | 1.46 (already in workspace) | Snapshot testing for type inference results and error messages | Test every inference rule and error message with snapshots |
| `rustc-hash` | 2.x | Fast hash maps for type environments and interning | FxHashMap is significantly faster than std HashMap for compiler workloads with integer keys |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| `ena` | `unionfind` crate | `unionfind` is simpler but lacks `ena`'s snapshot/rollback support needed for speculative unification during trait resolution |
| `ariadne` | `codespan-reporting` | codespan-reporting is more stable and allows multiple notes per diagnostic, but ariadne produces better visual output matching the Elm-quality goal |
| `ariadne` | `miette` | miette integrates with std::error::Error ecosystem, but adds complexity; ariadne is purpose-built for compilers |
| Hand-rolled union-find | `ena` | Never hand-roll -- ena handles path compression, rank balancing, and snapshots correctly |

**Installation:**
```toml
[dependencies]
ena = "0.14"
ariadne = "0.6"
rustc-hash = "2"
```

## Architecture Patterns

### Recommended Project Structure

```
crates/
  snow-typeck/           # NEW crate for this phase
    src/
      lib.rs             # Public API: check(ast) -> TypeckResult
      ty.rs              # Type representation (Ty enum, TyVar, Scheme)
      infer.rs           # Inference engine (constraint generation + solving)
      unify.rs           # Unification: unify two types, occurs check
      env.rs             # Type environment (Gamma): scope stack of bindings
      builtins.rs        # Built-in types: Int, Float, String, Bool, Option, Result
      traits.rs          # Interface definitions, impl blocks, trait resolution
      error.rs           # TypeError types with provenance/origin tracking
      diagnostics.rs     # ariadne rendering of TypeErrors to user-facing output
      tests/             # Snapshot tests
```

### Pattern 1: Type Representation with `ena`

**What:** Define types as an enum with a separate type variable key backed by `ena::UnificationTable`.

**When to use:** Always -- this is the core data structure.

**Example:**
```rust
use ena::unify::{InPlaceUnificationTable, UnifyKey, EqUnifyValue};

/// A type variable key for the union-find.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TyVar(u32);

impl UnifyKey for TyVar {
    type Value = Option<Ty>;
    fn index(&self) -> u32 { self.0 }
    fn from_index(i: u32) -> Self { TyVar(i) }
    fn tag() -> &'static str { "TyVar" }
}

impl EqUnifyValue for Ty {}

/// The core type representation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Ty {
    /// Unresolved type variable (index into union-find)
    Var(TyVar),
    /// Concrete named type: Int, Float, String, Bool, etc.
    Con(TyCon),
    /// Function type: args -> return
    Fun(Vec<Ty>, Box<Ty>),
    /// Generic application: List<Int>, Option<String>
    App(Box<Ty>, Vec<Ty>),
    /// Tuple type: (Int, String)
    Tuple(Vec<Ty>),
    /// Never type (for expressions that don't return)
    Never,
}

/// Type constructor (concrete named type)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TyCon {
    pub name: String, // interned in production
}
```

### Pattern 2: Level-Based Generalization (Remy's Algorithm)

**What:** Track a "level" integer on each type variable to avoid scanning the type environment during let-generalization.

**When to use:** Always -- essential for efficient let-polymorphism.

**Example:**
```rust
pub struct InferCtx {
    table: InPlaceUnificationTable<TyVar>,
    current_level: u32,
}

impl InferCtx {
    fn enter_level(&mut self) { self.current_level += 1; }
    fn leave_level(&mut self) { self.current_level -= 1; }

    fn fresh_var(&mut self) -> Ty {
        let var = self.table.new_key(None);
        // Store level as metadata on the variable
        // Variables with level > current_level after leave_level are generalizable
        Ty::Var(var)
    }

    fn generalize(&self, ty: Ty) -> Scheme {
        // Quantify all type variables with level > current_level
        let free_vars = self.collect_vars_above_level(&ty, self.current_level);
        Scheme { vars: free_vars, ty }
    }
}
```

### Pattern 3: Constraint-Based Inference with Provenance

**What:** Generate constraints while walking the AST, each tagged with an origin (source location + why the constraint exists). Solve constraints via unification. On failure, the provenance gives error context.

**When to use:** Always -- this is the core inference algorithm.

**Example:**
```rust
/// Why a constraint was generated
#[derive(Debug, Clone)]
pub enum ConstraintOrigin {
    /// From a function application: callee expected this type for argument N
    FnArg { call_site: Span, param_idx: usize },
    /// From a binary operator: both sides must match
    BinOp { op_span: Span },
    /// From an if-expression: both branches must have same type
    IfBranches { if_span: Span, then_span: Span, else_span: Span },
    /// From a let binding with annotation
    Annotation { annotation_span: Span },
    /// From a return statement
    Return { return_span: Span, fn_span: Span },
}

/// A type equality constraint
pub struct Constraint {
    pub expected: Ty,
    pub actual: Ty,
    pub origin: ConstraintOrigin,
}
```

### Pattern 4: Type Scheme for Let-Polymorphism

**What:** After inferring a let-binding's type, generalize it into a type scheme (forall a. ...). On each use, instantiate fresh variables.

**When to use:** Every let-binding and named function definition.

**Example:**
```rust
/// A polymorphic type scheme: forall vars. ty
pub struct Scheme {
    pub vars: Vec<TyVar>,
    pub ty: Ty,
}

impl InferCtx {
    fn instantiate(&mut self, scheme: &Scheme) -> Ty {
        let mapping: HashMap<TyVar, TyVar> = scheme.vars.iter()
            .map(|&v| (v, self.fresh_var_key()))
            .collect();
        self.apply_mapping(&scheme.ty, &mapping)
    }
}
```

### Pattern 5: Interface (Trait) and Impl Block Design

**What:** Interface definitions create a trait with method signatures. Impl blocks provide implementations for specific types. Trait resolution finds the correct impl when a constrained generic is used.

**Recommended syntax (Claude's Discretion):**
```
# Interface definition (user decided: `interface` keyword)
interface Printable do
  fn to_string(self) -> String
end

# Implementation block (recommendation: `impl` keyword matching existing IMPL_KW)
impl Printable for Int do
  fn to_string(self) -> String
    # ...
  end
end

# Where clause (user decided)
fn print_all<T>(items: List<T>) where T: Printable
  # ...
end
```

### Anti-Patterns to Avoid

- **Eager substitution instead of union-find:** Do not apply substitutions by walking and replacing. Use `ena`'s union-find and resolve lazily via `probe_value()`. Eager substitution is O(n^2) in chain length.
- **Scanning type environment for generalization:** Use level-based generalization instead. Environment scanning is O(n * m) per let-binding where n is environment size and m is type size.
- **Losing source locations during unification:** Always propagate constraint origins. Without origins, errors say "type mismatch" with no indication of where the conflicting types came from.
- **Monomorphic function inference:** Forgetting to generalize function types leads to "value restriction" bugs where `let id = fn x -> x end` cannot be used at multiple types.

## Don't Hand-Roll

Problems that look simple but have existing solutions:

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Union-find data structure | Custom linked-list or map-based substitution | `ena::unify::InPlaceUnificationTable` | Path compression, rank balancing, snapshot/rollback are subtle; ena is battle-tested from rustc |
| Diagnostic formatting | Custom string formatting with ANSI codes | `ariadne::Report` with labels | Multi-line span layout, overlap avoidance, color management have hundreds of edge cases |
| Occurs check | Simple recursive contains-check | Integrate with `ena`'s unify + custom walk | The occurs check must interact correctly with union-find indirection; a naive approach misses resolved variables |

**Key insight:** Union-find is the heart of HM inference. Getting it wrong causes subtle, hard-to-debug failures (infinite types, incorrect polymorphism, unsound type equivalences). The `ena` crate is exactly this code extracted from the Rust compiler.

## Common Pitfalls

### Pitfall 1: Incorrect Occurs Check
**What goes wrong:** Unifying a type variable with a type that contains itself creates infinite types. `fn x -> x(x) end` should be rejected but passes if the occurs check is missing or buggy.
**Why it happens:** The occurs check must look through the union-find to see resolved variables, not just check syntactic containment.
**How to avoid:** When unifying `Var(a)` with `ty`, first normalize `ty` by resolving all variables through the union-find, then check if `a` appears in the normalized type. Reject with a clear error if it does.
**Warning signs:** Programs that should be infinite-type errors compile successfully; the type checker enters infinite loops during normalization.

### Pitfall 2: Incorrect Let-Generalization Scope
**What goes wrong:** Type variables that escape their scope get incorrectly generalized, making the type system unsound. For example, a mutable reference's type variable getting generalized.
**Why it happens:** Generalization quantifies all "free" variables in a type, but some variables are constrained by the outer scope and must not be generalized.
**How to avoid:** Use level-based generalization (Remy's algorithm). Only generalize variables whose level is strictly greater than the current level at the point of generalization.
**Warning signs:** Programs that should be type errors compile; different uses of the same binding get incompatible types.

### Pitfall 3: Forgetting to Instantiate Polymorphic Types
**What goes wrong:** Using a polymorphic binding at two different types fails because both uses share the same type variables.
**Why it happens:** `let id = fn x -> x end` gets scheme `forall a. a -> a`. If both `id(1)` and `id("hello")` use the same `a`, they conflict.
**How to avoid:** Every use of a let-bound name must call `instantiate()` on its scheme, creating fresh type variables for each use site.
**Warning signs:** Success criterion #1 (`id(1), id("hello")`) fails with a type conflict.

### Pitfall 4: Poor Error Locations from Bottom-Up Inference
**What goes wrong:** Type errors point to the wrong location. A function is called with wrong argument types, but the error points to the function definition, or to a different call site.
**Why it happens:** Bottom-up inference (Algorithm W/J) discovers conflicts when unification fails, but the failure point may be far from the actual mistake.
**How to avoid:** Track constraint provenance -- every constraint records where it was generated (which AST node, why). When unification fails, report the constraint's origin, not the unification call site. Show both "expected" and "found" locations.
**Warning signs:** Error messages are technically correct but confusing because they point to the definition when the bug is at the call site.

### Pitfall 5: Parser Syntax Mismatch with Type Decisions
**What goes wrong:** The parser currently uses square brackets `[A, B]` for generic type parameters (see `parse_type_param_list` and `parse_type` in items.rs), but the user decided on angle brackets `<T>`. This mismatch means existing parse_type infrastructure must be migrated.
**Why it happens:** The parser was built before type syntax decisions were locked.
**How to avoid:** Update the parser early in this phase to use `LT`/`GT` for generic delimiters in type positions. This requires disambiguating `<` as a comparison operator vs generic open bracket (similar to C++/Rust parser challenges). The lexer also needs a `?` token for `Int?` sugar and `!` reuse for `T!E` sugar.
**Warning signs:** Snapshot tests for type annotations fail; parser produces TYPE_PARAM_LIST with brackets instead of angle brackets.

### Pitfall 6: Missing `?` and `!` Tokens in the Lexer
**What goes wrong:** The `?` character is currently not tokenized (produces `UnexpectedCharacter`). The `!` (`BANG`) exists but is only used as a unary prefix operator. For `Int?` (Option sugar) and `T!E` (Result sugar), the lexer and parser need updates.
**Why it happens:** These sugar syntaxes were decided during phase planning but the lexer was built in Phase 1.
**How to avoid:** Add a `QUESTION` token kind to the lexer. Decide whether `?` is a postfix type operator (in type position) or also an expression operator (for Result propagation). For `!` in types (`T!E`), decide if it is the same `BANG` token reinterpreted by the type parser, or a separate token.
**Warning signs:** Any source code containing `?` or `T!E` syntax produces lexer errors.

## Code Examples

### Example 1: Core Unification Loop

```rust
/// Unify two types, updating the union-find table.
fn unify(&mut self, a: Ty, b: Ty, origin: ConstraintOrigin) -> Result<(), TypeError> {
    let a = self.resolve(a);
    let b = self.resolve(b);

    match (a, b) {
        // Both resolved to the same variable -- nothing to do
        (Ty::Var(a), Ty::Var(b)) if a == b => Ok(()),

        // Variable with concrete type -- bind after occurs check
        (Ty::Var(v), ty) | (ty, Ty::Var(v)) => {
            if self.occurs_in(v, &ty) {
                Err(TypeError::InfiniteType { var: v, ty, origin })
            } else {
                self.table.unify_var_value(v, Some(ty))
                    .map_err(|_| TypeError::UnificationFailed { origin })?;
                Ok(())
            }
        }

        // Two function types -- unify args pairwise, then return types
        (Ty::Fun(args_a, ret_a), Ty::Fun(args_b, ret_b)) => {
            if args_a.len() != args_b.len() {
                return Err(TypeError::ArityMismatch {
                    expected: args_a.len(),
                    found: args_b.len(),
                    origin,
                });
            }
            for (a, b) in args_a.into_iter().zip(args_b.into_iter()) {
                self.unify(a, b, origin.clone())?;
            }
            self.unify(*ret_a, *ret_b, origin)
        }

        // Two concrete types -- must be identical
        (Ty::Con(a), Ty::Con(b)) if a == b => Ok(()),

        // Generic application -- unify constructor and args
        (Ty::App(con_a, args_a), Ty::App(con_b, args_b)) => {
            self.unify(*con_a, *con_b, origin.clone())?;
            for (a, b) in args_a.into_iter().zip(args_b.into_iter()) {
                self.unify(a, b, origin.clone())?;
            }
            Ok(())
        }

        // Mismatch
        (expected, found) => Err(TypeError::Mismatch {
            expected,
            found,
            origin,
        }),
    }
}
```

### Example 2: Inferring a Let-Binding with Generalization

```rust
fn infer_let(&mut self, binding: &LetBinding) -> Result<Ty, TypeError> {
    // Enter a new level for generalization
    self.enter_level();

    // Infer the type of the initializer
    let init_ty = self.infer_expr(binding.initializer()?)?;

    // If there's a type annotation, unify with it
    if let Some(ann) = binding.type_annotation() {
        let ann_ty = self.resolve_annotation(&ann)?;
        self.unify(init_ty.clone(), ann_ty, ConstraintOrigin::Annotation {
            annotation_span: ann.syntax().text_range().into(),
        })?;
    }

    // Leave level and generalize
    self.leave_level();
    let scheme = self.generalize(init_ty);

    // Bind the name in the environment
    let name = binding.name()?.text()?;
    self.env.insert(name, scheme.clone());

    Ok(scheme.ty.clone())
}
```

### Example 3: Error Diagnostic Rendering with ariadne

```rust
use ariadne::{Report, ReportKind, Label, Source, Color};

fn render_type_error(error: &TypeError, source: &str, filename: &str) {
    match error {
        TypeError::Mismatch { expected, found, origin } => {
            let (expected_span, found_span) = origin.spans();
            Report::build(ReportKind::Error, filename, expected_span.start as usize)
                .with_code("E0001")
                .with_message(format!("expected {}, found {}", expected, found))
                .with_label(
                    Label::new((filename, expected_span.into_range()))
                        .with_message(format!("expected {}", expected))
                        .with_color(Color::Red)
                )
                .with_label(
                    Label::new((filename, found_span.into_range()))
                        .with_message(format!("found {}", found))
                        .with_color(Color::Blue)
                )
                .with_help(suggest_fix(expected, found))
                .finish()
                .print((filename, Source::from(source)))
                .unwrap();
        }
        // ... other error kinds
    }
}
```

### Example 4: Built-in Option/Result Types

```rust
fn register_builtins(&mut self) {
    // Primitive types
    self.register_type("Int", TyCon::new("Int"));
    self.register_type("Float", TyCon::new("Float"));
    self.register_type("String", TyCon::new("String"));
    self.register_type("Bool", TyCon::new("Bool"));

    // Built-in generic types
    // Option<T> -- represented as a generic type constructor
    self.register_generic_type("Option", 1);  // Option takes 1 type param

    // Result<T, E> -- represented as a generic type constructor
    self.register_generic_type("Result", 2);  // Result takes 2 type params

    // Sugar mappings for the parser:
    // Int?    -> Option<Int>    (handled in type resolution)
    // T!E     -> Result<T, E>  (handled in type resolution)
}
```

## Discretion Recommendations

Areas left to Claude's discretion with researched recommendations:

### 1. Implementation syntax for interfaces

**Recommendation:** Use `impl ... for ... do ... end` matching Snow's block style.

```
impl Printable for Int do
  fn to_string(self) -> String
    # ...
  end
end
```

**Rationale:** The `IMPL_KW` already exists in the lexer/parser. Using `impl ... for ... do ... end` is consistent with Snow's `do/end` block convention. This mirrors Rust's `impl Trait for Type { ... }` but with Snow's keyword style.

### 2. Internal type representation

**Recommendation:** Use `ena`-based union-find (`InPlaceUnificationTable<TyVar>`) with `Option<Ty>` as the value type.

**Rationale:** `ena` is extracted from rustc, battle-tested, supports snapshot/rollback (needed for speculative trait resolution), and has the exact API needed (unify_var_var, unify_var_value, probe_value). The Thunderseethe blog post and rustc dev guide both demonstrate this exact pattern. No reason to consider alternatives.

### 3. Unification algorithm details

**Recommendation:** Use Algorithm J style (mutable in-place unification via `ena`) with constraint provenance tracking. Two-phase approach: (1) walk AST generating constraints with origins, (2) solve constraints via unification. This combines the efficiency of J with the error quality of constraint-based approaches.

**Rationale:** Pure Algorithm W returns explicit substitutions (slower, harder to implement). Algorithm J uses mutable union-find (exactly what `ena` provides). Adding constraint provenance on top gives the error location quality needed for the Elm-quality error messages requirement. The Thunderseethe blog demonstrates exactly this hybrid approach.

### 4. Propagation operator syntax

**Recommendation:** Use `?` as a postfix expression operator for Result propagation, matching Rust's syntax: `let value = risky_fn()?`. However, **defer the full propagation implementation to Phase 5 (codegen) or later** -- in this phase, just define the type semantics (the `?` operator on `Result<T, E>` evaluates to `T` and the enclosing function must return `Result<_, E>`). Parse it and type-check it; codegen is out of scope.

**Rationale:** The `?` character will already be added to the lexer for `Int?` (Option sugar). Reusing it as a postfix expression operator is natural and familiar. Full propagation requires control flow (early return), which is a codegen concern. In this phase, the type checker just needs to know that `expr?` where `expr: Result<T, E>` has type `T` and constrains the function return type.

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Algorithm W with explicit substitutions | Algorithm J with mutable union-find (ena) | ~2015 (ena extracted from rustc) | 2-5x faster; simpler implementation |
| Environment scanning for generalization | Level-based generalization (Remy 1988, OCaml) | Adopted by OCaml ~1992, widely understood ~2015 | O(1) generalization check instead of O(n) |
| Single error then stop | Multi-error with provenance tracking | ~2018 (Elm 0.19, Rust 2018) | Users see all errors at once with good locations |
| codespan-reporting (dominant ~2020) | ariadne (better visuals, ~2022+) | 2022 | More visually appealing diagnostics with same data model |

**Deprecated/outdated:**
- `codespan` (v0.11): Superseded by `codespan-reporting` which is superseded visually by `ariadne`
- Naive substitution-based unification: Replaced by union-find based approaches everywhere

## Critical Implementation Notes

### Parser Migration Required

The existing parser uses square brackets `[A, B]` for type parameters (`L_BRACKET`/`R_BRACKET`), but the locked decision specifies angle brackets `<T>` (`LT`/`GT`). This requires:

1. **Updating `parse_type_param_list()`** in `items.rs` to use `LT`/`GT` instead of `L_BRACKET`/`R_BRACKET`
2. **Updating `parse_type()`** in `items.rs` for generic type arguments to use `LT`/`GT`
3. **Disambiguating `<` in expressions vs types** -- when parsing a type, `<` opens generics; in expressions, it is comparison. Context-dependent parsing (the parser already knows when it is in a type position via `parse_type()` calls).
4. **Adding `QUESTION` token** to lexer for `?` postfix in type sugar (`Int?`) and potentially expression propagation
5. **Handling `!` in type position** for `T!E` (Result sugar) -- already tokenized as `BANG`, just needs type-parser support

### New SyntaxKind Variants Needed

```
INTERFACE_DEF       -- interface Printable do ... end
INTERFACE_METHOD    -- method signature within interface
IMPL_DEF            -- impl Printable for Int do ... end
TYPE_ALIAS_DEF      -- type Name = ExistingType
WHERE_CLAUSE        -- where T: Printable
TRAIT_BOUND         -- T: Printable (within where clause)
GENERIC_PARAM_LIST  -- <A, B> (replacing TYPE_PARAM_LIST's bracket syntax)
GENERIC_ARG_LIST    -- <Int, String> (in type application position)
```

### Type Checker Output

The type checker should produce a `TypeckResult` containing:
1. A type table: `HashMap<NodeId, Ty>` mapping every expression/pattern AST node to its resolved type
2. An error list: `Vec<TypeError>` with full provenance
3. A trait impl registry: which types implement which interfaces

This output is consumed by later phases (pattern matching exhaustiveness in Phase 4, codegen in Phase 5).

## Open Questions

1. **How to assign NodeIds to AST nodes**
   - What we know: The current AST uses rowan `SyntaxNode` which has `text_range()` but no unique integer ID. The type checker needs a way to associate types with specific AST nodes.
   - What's unclear: Whether to use rowan's `SyntaxNode` pointer identity, text ranges as keys, or build a separate ID assignment pass.
   - Recommendation: Use `TextRange` (start offset) as the key -- it is unique per node and does not require an extra pass. Alternatively, build a thin "HIR" (High-level IR) with explicit NodeIds during a lowering pass from AST, which also desugars Option/Result sugar.

2. **Interaction between full parameter inference and trait constraints**
   - What we know: Full inference for function parameters means `fn add(x, y) = x + y` must infer both parameter types. The `+` operator is dispatched via a trait (like `Add`). This means inference must understand trait methods.
   - What's unclear: Whether to implement numeric traits (Add, Sub, etc.) in this phase or defer to later phases when the standard library exists.
   - Recommendation: Define a minimal set of "compiler-known" traits for arithmetic operators in this phase. The inference engine needs them to type-check basic expressions. They can be expanded later.

3. **Numeric type disambiguation without defaults**
   - What we know: No numeric literal defaults -- `42` could be Int or Float, and ambiguity is an error.
   - What's unclear: Whether `42` should always be `Int` (like Go/Rust where `42` is always integer) or truly ambiguous.
   - Recommendation: Make integer literals always `Int` and float literals (with `.`) always `Float`. This is not a "default" -- it is the literal's type. "No defaults" then means that when a function receives an untyped parameter used in both Int and Float contexts, it errors rather than picking one. Verify this interpretation with the user if there is ambiguity.

## Sources

### Primary (HIGH confidence)
- [ena crate](https://crates.io/crates/ena/0.14.3) -- version 0.14.3, API verified via docs.rs
- [ariadne crate](https://crates.io/crates/ariadne) -- version 0.6.0, API verified via docs.rs
- [ena::unify module docs](https://docs.rs/ena/latest/ena/unify/index.html) -- UnificationTable, UnifyKey, UnifyValue API
- [ariadne docs](https://docs.rs/ariadne/latest/ariadne/) -- Report, Label, Source API
- [Thunderseethe: Unification](https://thunderseethe.dev/posts/unification/) -- Detailed ena-based type inference implementation walkthrough
- [Thunderseethe: Types Base](https://thunderseethe.dev/posts/types-base/) -- Error provenance tracking with NodeId side-tables
- [Oleg Kiselyov: Efficient Generalization](https://okmij.org/ftp/ML/generalization.html) -- Level-based generalization algorithm (Remy 1988)

### Secondary (MEDIUM confidence)
- [Rustc Dev Guide: Type Inference](https://rustc-dev-guide.rust-lang.org/type-inference.html) -- rustc's HM extensions
- [Max Bernstein: HM Inference Two Ways](https://bernsteinbear.com/blog/type-inference/) -- Algorithm W vs J comparison
- [Wikipedia: Hindley-Milner](https://en.wikipedia.org/wiki/Hindley%E2%80%93Milner_type_system) -- Algorithm W vs J vs M comparison
- [Write You a Haskell: HM Chapter](https://github.com/sdiehl/write-you-a-haskell/blob/master/006_hindley_milner.md) -- Constraint-based approach tutorial
- [codespan-reporting vs ariadne vs miette comparison](https://lib.rs/crates/ariadne) -- Feature comparison across diagnostic crates

### Tertiary (LOW confidence)
- [Total Type Error Localization with Holes](https://dl.acm.org/doi/10.1145/3632910) -- Academic paper on bidirectional error localization (Hazel/Elm-related)
- [Wadler: Type Classes](https://homepages.inf.ed.ac.uk/wadler/topics/type-classes.html) -- Seminal papers on trait coherence (COCHIS)
- [Getting into the Flow](https://dl.acm.org/doi/10.1145/3622812) -- Constraint-based error explanation via type flows

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- ena and ariadne are well-documented, version-verified crates with clear APIs
- Architecture: HIGH -- Algorithm J + ena + level-based generalization is the standard approach, demonstrated in multiple implementations
- Pitfalls: HIGH -- occurs check, generalization scope, and error location are well-documented pitfalls in type inference literature
- Parser migration: HIGH -- directly verified by reading the existing parser source code (square brackets vs angle brackets mismatch confirmed)
- Trait system: MEDIUM -- trait resolution complexity depends on implementation scope; basic where-clause support is straightforward but full coherence checking is complex

**Research date:** 2026-02-06
**Valid until:** 2026-03-06 (stable domain, 30-day validity)
