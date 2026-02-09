# Phase 22: Auto-Derive (Stretch) - Research

**Researched:** 2026-02-08
**Domain:** Snow compiler -- `deriving(Eq, Ord, Display, Debug, Hash)` syntax and compiler-driven impl generation
**Confidence:** HIGH (codebase investigation, verified against actual source code in all relevant crates)

## Summary

This phase transitions Snow's current *implicit auto-derive* model (where the compiler silently generates Eq, Ord, Debug, and Hash impls for every non-generic struct/sum type) to an *explicit opt-in* model via `deriving(...)` syntax. Today, the compiler unconditionally generates `Eq__eq__StructName`, `Ord__lt__StructName`, `Debug__inspect__StructName`, and `Hash__hash__StructName` for every non-generic struct, and `Eq__eq`, `Ord__lt`, `Debug__inspect` (but NOT Hash) for every non-generic sum type. Display (`Display__to_string__StructName`) is NOT generated at all for user types. The `deriving(...)` clause gives users control over which protocols are derived and adds Display as a derivable protocol.

The implementation touches four compiler phases: (1) lexer/parser to recognize `deriving(...)` syntax after `end` in struct/sum type definitions, (2) typeck to conditionally register trait impls based on the deriving clause instead of unconditionally, (3) MIR lowering to conditionally generate synthetic functions based on the deriving clause, and (4) snow-fmt to format the new syntax. The existing code generation for all five protocols (Eq, Ord, Debug, Hash for structs; Eq, Ord, Debug for sum types) already works correctly -- the only NEW generation needed is Display for structs/sum types and Hash for sum types.

**Primary recommendation:** Treat `deriving` as a contextual keyword (parsed as IDENT where text == "deriving"), add a `DERIVING_CLAUSE` CST node, thread the derive list through typeck and MIR lowering as a `Vec<String>`, and conditionally gate the existing auto-derive logic. Add new `generate_display_struct`, `generate_display_sum_type`, and `generate_hash_sum_type` functions modeled on the existing Debug/Hash patterns.

## Standard Stack

No new external dependencies. All implementation is within existing crates. Zero new Rust crate dependencies (per STATE.md constraint).

### Core Crates Affected

| Crate | File | What Changes |
|-------|------|-------------|
| snow-common | `token.rs` | No change needed -- `deriving` parsed as IDENT, not a keyword |
| snow-parser | `syntax_kind.rs` | Add `DERIVING_CLAUSE` and `DERIVING_KW` SyntaxKinds |
| snow-parser | `parser/items.rs` | After `end` in struct/sum type, optionally parse `deriving(Ident, ...)` |
| snow-parser | `ast/item.rs` | Add `deriving()` accessor to `StructDef` and `SumTypeDef` returning `Vec<String>` |
| snow-typeck | `infer.rs` | Read deriving clause from CST; conditionally register trait impls per derive list |
| snow-codegen | `mir/lower.rs` | Read deriving information; conditionally call `generate_*` functions; add new Display + Hash-sum generation |
| snow-fmt | `walker.rs` | Handle `DERIVING_CLAUSE` node in `walk_struct_def` and `walk_block_def` (sum types) |

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| `deriving(...)` after `end` | Rust-style `#[derive(...)]` attribute before definition | `deriving` is more Haskell-like, matches Snow's ML/FP aesthetic; attributes are a bigger syntax addition |
| Contextual keyword for `deriving` | New reserved keyword | Contextual avoids breaking existing `deriving` identifiers; simpler lexer change |
| Parsing as IDENT + text check | Adding `Deriving` to `TokenKind` | Using IDENT avoids adding a 46th keyword and changing token count tests |

## Architecture Patterns

### Pattern 1: Deriving Clause CST Structure

**What:** Parse `deriving(Trait1, Trait2, ...)` as a child node of `STRUCT_DEF` or `SUM_TYPE_DEF`.

**CST shape (after parsing):**
```
STRUCT_DEF
  STRUCT_KW "struct"
  NAME "Point"
  DO_KW "do"
  STRUCT_FIELD ...
  STRUCT_FIELD ...
  END_KW "end"
  DERIVING_CLAUSE            # <-- new node
    IDENT "deriving"         # or DERIVING_KW
    L_PAREN "("
    IDENT "Eq"
    COMMA ","
    IDENT "Display"
    COMMA ","
    IDENT "Debug"
    COMMA ","
    IDENT "Hash"
    R_PAREN ")"
```

**Parser change in `parse_struct_def`** (after consuming `END_KW`):
```rust
// After p.advance() for END_KW...

// Optional deriving clause: deriving(Trait1, Trait2, ...)
if p.at(SyntaxKind::IDENT) && p.current_text() == "deriving" {
    let dc = p.open();
    p.advance(); // "deriving" IDENT
    p.expect(SyntaxKind::L_PAREN);
    // Parse comma-separated trait names
    loop {
        if p.at(SyntaxKind::R_PAREN) || p.at(SyntaxKind::EOF) {
            break;
        }
        if p.at(SyntaxKind::IDENT) {
            p.advance(); // trait name
        } else {
            p.error("expected trait name in deriving clause");
            break;
        }
        if !p.eat(SyntaxKind::COMMA) {
            break;
        }
    }
    p.expect(SyntaxKind::R_PAREN);
    p.close(dc, SyntaxKind::DERIVING_CLAUSE);
}

p.close(m, SyntaxKind::STRUCT_DEF);
```

The same pattern applies to `parse_sum_type_def`.

### Pattern 2: AST Accessor for Deriving List

**What:** Add a method on `StructDef` and `SumTypeDef` that extracts the derive trait names.

```rust
// In ast/item.rs, add to StructDef impl:
impl StructDef {
    /// The deriving clause traits, if present.
    /// Returns the list of trait names from `deriving(Eq, Display, ...)`.
    pub fn deriving_traits(&self) -> Vec<String> {
        self.syntax
            .children()
            .find(|n| n.kind() == SyntaxKind::DERIVING_CLAUSE)
            .map(|dc| {
                dc.children_with_tokens()
                    .filter_map(|it| it.into_token())
                    .filter(|t| t.kind() == SyntaxKind::IDENT && t.text() != "deriving")
                    .map(|t| t.text().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }
}
```

### Pattern 3: Conditional Trait Registration in Typeck

**What:** Replace the current unconditional auto-register in `infer.rs` with derive-list-gated registration.

**Current code** (`crates/snow-typeck/src/infer.rs:1459-1530`):
```rust
// Auto-register Debug, Eq, Ord impls for this struct type.
// Only for non-generic structs.
if generic_params.is_empty() {
    // Debug impl -- always registered
    // Eq impl -- always registered
    // Ord impl -- always registered
    // Hash impl -- always registered
}
```

**New code:**
```rust
// Conditionally register trait impls based on deriving clause.
// If no deriving clause exists, register nothing (breaking change from current behavior).
// OR: If no deriving clause, register all (backward-compatible) -- DESIGN DECISION NEEDED.
let derive_list = struct_def.deriving_traits();
let derive_all = derive_list.is_empty(); // backward compat: no clause = derive all

if generic_params.is_empty() {
    if derive_all || derive_list.contains(&"Debug".to_string()) {
        // register Debug impl
    }
    if derive_all || derive_list.contains(&"Eq".to_string()) {
        // register Eq impl
    }
    if derive_all || derive_list.contains(&"Ord".to_string()) {
        // register Ord impl
    }
    if derive_all || derive_list.contains(&"Hash".to_string()) {
        // register Hash impl
    }
    if derive_list.contains(&"Display".to_string()) {
        // register Display impl (NEW -- never auto-derived before)
    }
}
```

### Pattern 4: Conditional MIR Generation

**What:** Thread derive information into MIR lowering so `generate_*` functions are called selectively.

**Current code** (`crates/snow-codegen/src/mir/lower.rs:1299-1309`):
```rust
self.generate_debug_inspect_struct(&name, &fields);
self.generate_eq_struct(&name, &fields);
self.generate_ord_struct(&name, &fields);
self.generate_hash_struct(&name, &fields);
```

**Required change:** Pass a derive list (or boolean flags) into the struct/sum-type lowering path. The derive list must be extracted from the CST `STRUCT_DEF` node. Currently, MIR lowering iterates CST nodes directly (it has access to `&Parse`), so the lowerer can read the `DERIVING_CLAUSE` child.

```rust
let derive_list: Vec<String> = /* extract from CST STRUCT_DEF node */;
let derive_all = derive_list.is_empty();

if derive_all || derive_list.iter().any(|t| t == "Debug") {
    self.generate_debug_inspect_struct(&name, &fields);
}
if derive_all || derive_list.iter().any(|t| t == "Eq") {
    self.generate_eq_struct(&name, &fields);
}
if derive_all || derive_list.iter().any(|t| t == "Ord") {
    self.generate_ord_struct(&name, &fields);
}
if derive_all || derive_list.iter().any(|t| t == "Hash") {
    self.generate_hash_struct(&name, &fields);
}
if derive_list.iter().any(|t| t == "Display") {
    self.generate_display_struct(&name, &fields); // NEW
}
```

### Pattern 5: Display Generation for Structs

**What:** Generate `Display__to_string__StructName` producing `"Point(1, 2)"` style output (per success criteria).

**Note:** This is distinct from Debug which produces `"Point { x: 1, y: 2 }"`. Display uses positional format.

**Implementation model** (based on existing `generate_debug_inspect_struct`):
```rust
fn generate_display_struct(&mut self, name: &str, fields: &[(String, MirType)]) {
    let mangled = format!("Display__to_string__{}", name);
    let struct_ty = MirType::Struct(name.to_string());
    let concat_ty = MirType::FnPtr(
        vec![MirType::String, MirType::String],
        Box::new(MirType::String),
    );
    let self_var = MirExpr::Var("self".to_string(), struct_ty.clone());

    // Build: "Point(1, 2)" style output
    let mut result = MirExpr::StringLit(
        if fields.is_empty() { format!("{}()", name) } else { format!("{}(", name) },
        MirType::String,
    );

    for (i, (field_name, field_ty)) in fields.iter().enumerate() {
        let field_access = MirExpr::FieldAccess {
            object: Box::new(self_var.clone()),
            field: field_name.clone(),
            ty: field_ty.clone(),
        };
        let field_str = self.wrap_to_string(field_access, None);
        result = MirExpr::Call {
            func: Box::new(MirExpr::Var("snow_string_concat".to_string(), concat_ty.clone())),
            args: vec![result, field_str],
            ty: MirType::String,
        };
        if i < fields.len() - 1 {
            result = MirExpr::Call {
                func: Box::new(MirExpr::Var("snow_string_concat".to_string(), concat_ty.clone())),
                args: vec![result, MirExpr::StringLit(", ".to_string(), MirType::String)],
                ty: MirType::String,
            };
        }
    }

    if !fields.is_empty() {
        result = MirExpr::Call {
            func: Box::new(MirExpr::Var("snow_string_concat".to_string(), concat_ty.clone())),
            args: vec![result, MirExpr::StringLit(")".to_string(), MirType::String)],
            ty: MirType::String,
        };
    }

    // Create MirFunction, push to self.functions, register in known_functions
}
```

### Pattern 6: Hash Generation for Sum Types

**What:** Generate `Hash__hash__SumTypeName` for sum types (currently only done for structs).

**Implementation:** Combine the tag value with field hashes using `snow_hash_combine`. Model on the existing sum type Eq/Ord pattern (nested Match with Constructor patterns).

```rust
fn generate_hash_sum_type(&mut self, name: &str, variants: &[MirVariantDef]) {
    // Match self against each variant
    // For each variant:
    //   let h = snow_hash_int(tag)
    //   for each field: h = snow_hash_combine(h, Hash__hash__FieldType(field))
    //   return h
}
```

### Pattern 7: Display Generation for Sum Types

**What:** Generate `Display__to_string__SumTypeName` producing variant name for output.

**Implementation:** Match on self, for each variant produce the variant name string with optional field values in parentheses. Similar to Debug inspect for sum types but without the struct-style `{ }` formatting.

### Anti-Patterns to Avoid

- **Don't make `deriving` a reserved keyword:** Keep it as a contextual identifier to avoid breaking existing code that might use `deriving` as a variable name. The parser checks `p.current_text() == "deriving"` only in the post-`end` position.
- **Don't add a new compiler pass:** The deriving information should be threaded through existing typeck and MIR lowering passes, not processed in a separate pass.
- **Don't change the `generate_*` function signatures:** The existing functions take `(&str, &[(String, MirType)])` and work fine. Just gate when they're called.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Display string format for structs | New runtime function | Existing `snow_string_concat` chain in MIR (same as Debug) | All string building already uses concat chains; Display just uses different format |
| Hash for sum type fields | New hash infrastructure | Existing `snow_hash_combine` + per-type `snow_hash_*` / `Hash__hash__Type` calls | FNV-1a infrastructure from Phase 21 handles all hashing |
| Trait impl registration | New registration path | Existing `trait_registry.register_impl(TraitImplDef { ... })` | Exact same pattern used for 5 other auto-registered traits |

**Key insight:** 95% of the code generation machinery already exists. The new work is primarily: (1) syntax parsing (~40 lines), (2) conditional gating (~30 lines per callsite), (3) Display generation for structs/sum types (~80 lines each, modeled on existing Debug generation), (4) Hash generation for sum types (~60 lines, modeled on existing Hash struct generation).

## Common Pitfalls

### Pitfall 1: Backward Compatibility with Implicit Deriving

**What goes wrong:** If `deriving(...)` becomes required and no clause means "derive nothing", every existing struct/sum type test breaks because they currently get Eq/Ord/Debug/Hash automatically.

**Why it happens:** Current code unconditionally registers all four trait impls for every non-generic struct and generates all four MIR functions.

**How to avoid:** Make the absence of a `deriving` clause equivalent to deriving all currently-auto-derived protocols. Only change behavior when the clause is explicitly present. This means:
- No `deriving` clause = auto-derive Debug, Eq, Ord, Hash (current behavior, backward compatible)
- `deriving(Eq, Display)` = derive ONLY Eq and Display (opt-in model)
- `deriving()` = derive nothing (explicit opt-out)

**Warning signs:** Existing e2e tests using `==` on structs or `"${struct_value}"` interpolation break after the change.

### Pitfall 2: Display vs Debug Format Confusion

**What goes wrong:** Derived Display produces the same output as Debug, defeating the purpose of having both.

**Why it happens:** The success criteria specifies Display should produce `"Point(1, 2)"` (positional, no field names) while Debug produces `"Point { x: 1, y: 2 }"` (named fields, braces). Easy to copy-paste Debug generation and forget to change the format.

**How to avoid:** Clearly define the formats:
- `Display` format: `"TypeName(val1, val2)"` -- positional, parentheses
- `Debug` format: `"TypeName { field1: val1, field2: val2 }"` -- named, braces

### Pitfall 3: Display for Nested Types

**What goes wrong:** `Display__to_string__Point` tries to call `Display__to_string__Int` for field values, but the Display trait registration only exists for types that explicitly derive Display.

**Why it happens:** The `wrap_to_string` helper in MIR lowering dispatches to `Display__to_string__Type` for struct fields. If a field's type doesn't have a Display impl registered, the call resolves to nothing.

**How to avoid:** For primitive types (Int, Float, Bool, String), Display impls are already registered in `builtins.rs`. For struct/sum type fields, the Display generation should use `wrap_to_string` which already falls back to runtime `snow_*_to_string` functions for primitives. If a nested struct field doesn't have Display, the compiler should either (a) use Debug instead as fallback, or (b) require all field types to also derive Display. Option (a) is simpler for v1.3.

### Pitfall 4: Deriving on Generic Types

**What goes wrong:** Attempting `struct Pair<T> do a :: T, b :: T end deriving(Eq)` when generic type auto-derive is not supported.

**Why it happens:** Current auto-derive only works for non-generic types (the `if generic_params.is_empty()` guard in typeck).

**How to avoid:** For Phase 22, keep the existing limitation: `deriving(...)` is only effective on non-generic types. Emit a clear error message if a user adds `deriving(...)` to a generic type definition: "deriving is not supported for generic types".

### Pitfall 5: Typeck and MIR Derive List Mismatch

**What goes wrong:** Typeck registers an impl for Display on a struct, but MIR lowering doesn't generate the `Display__to_string__StructName` function, causing a link error.

**Why it happens:** The derive list must be read in both typeck (for trait impl registration) and MIR lowering (for function generation). If they read different information or one is skipped, the two get out of sync.

**How to avoid:** Both typeck and MIR lowering read from the same CST node (`DERIVING_CLAUSE` child of `STRUCT_DEF`). Use the same extraction logic (or share a helper function). Test with the e2e flow to catch mismatches early.

## Code Examples

### Example 1: Struct with Deriving Clause (Target Syntax)

```snow
struct Point do
  x :: Int
  y :: Int
end deriving(Eq, Display, Debug, Hash)

fn main() do
  let p = Point { x: 1, y: 2 }
  println("${p}")              # Uses Display: "Point(1, 2)"
  let q = Point { x: 1, y: 2 }
  println("${p == q}")         # Uses Eq: "true"
end
```

### Example 2: Sum Type with Deriving Clause

```snow
type Shape do
  Circle(Float)
  Rectangle(Float, Float)
end deriving(Eq, Ord, Display, Debug)

fn main() do
  let s = Circle(3.14)
  println("${s}")              # Uses Display: "Circle(3.14)"
end
```

### Example 3: Struct Without Deriving (Backward Compatible)

```snow
struct Point do
  x :: Int
  y :: Int
end
# No deriving clause: auto-derives Debug, Eq, Ord, Hash (current behavior preserved)
```

### Example 4: Extracting Derive List from CST (Rust Code)

```rust
// In snow-parser/src/ast/item.rs
impl StructDef {
    pub fn deriving_traits(&self) -> Vec<String> {
        self.syntax
            .children()
            .find(|n| n.kind() == SyntaxKind::DERIVING_CLAUSE)
            .map(|dc| {
                dc.children_with_tokens()
                    .filter_map(|it| it.into_token())
                    .filter(|t| t.kind() == SyntaxKind::IDENT && t.text() != "deriving")
                    .map(|t| t.text().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }
}
```

## State of the Art

| Current Behavior | Phase 22 Behavior | Impact |
|-----------------|-------------------|--------|
| Debug auto-derived for all non-generic structs/sum types | Derived only when `deriving(Debug)` present OR no clause at all (backward compat) | Opt-in control |
| Eq auto-derived for all non-generic structs/sum types | Same conditional logic | Opt-in control |
| Ord auto-derived for all non-generic structs/sum types | Same conditional logic | Opt-in control |
| Hash auto-derived for non-generic structs ONLY (not sum types) | Hash derivable for both structs AND sum types | Fills gap |
| Display NOT auto-derived for any user type | Display derivable for structs and sum types via `deriving(Display)` | New capability |
| No `deriving` syntax exists | `deriving(...)` clause after `end` | New syntax |

**Key gaps filled:**
1. Display was never auto-derived -- now it can be explicitly requested
2. Hash was only auto-derived for structs, not sum types -- now derivable for both
3. Users had no control over which protocols were derived -- now they have explicit opt-in

## Design Decisions Needed

### Decision 1: Backward Compatibility Strategy

**Option A (Recommended):** No `deriving` clause = auto-derive all currently-auto-derived protocols (Debug, Eq, Ord, Hash for structs; Debug, Eq, Ord for sum types). Explicit `deriving(...)` clause = derive ONLY listed protocols.

**Option B:** No `deriving` clause = derive nothing. Requires updating all existing code.

**Recommendation:** Option A. It preserves backward compatibility and makes Phase 22 purely additive. Users who want the new behavior opt in. Users who don't care keep working.

### Decision 2: Display Format for Structs

The success criteria says `"Point(1, 2)"` style output. This is a positional format (no field names). Alternatively, could use `"Point(x: 1, y: 2)"`.

**Recommendation:** Follow the success criteria exactly: `"Point(1, 2)"` with no field names.

### Decision 3: Display Format for Sum Types

Options:
- `"Circle(3.14)"` -- variant name + field values
- `"Circle"` for nullary, `"Circle(3.14)"` for unary, `"Rectangle(1.0, 2.0)"` for multi-field

**Recommendation:** Variant name + parenthesized field values for variants with fields, just variant name for nullary variants.

### Decision 4: Error on Unsupported Derivations

Should `deriving(Add)` produce a compiler error, or silently ignore unknown trait names?

**Recommendation:** Emit a typeck error: "cannot derive `Add` -- only Eq, Ord, Display, Debug, and Hash are derivable". This catches typos early. The set of derivable traits is hardcoded in Phase 22.

## Open Questions

1. **Display recursion for nested structs:**
   If `struct Line do start :: Point, end :: Point end deriving(Display)` is defined and `Point` also derives Display, the Display generation for `Line` should call `Display__to_string__Point` for each field. This works if `wrap_to_string` correctly dispatches for struct types. Needs verification during implementation but the pattern is identical to how Debug already works.

2. **Derive ordering and dependencies:**
   If `deriving(Ord)` is specified without `Eq`, should the compiler also derive Eq (since Ord logically implies Eq)? For v1.3, recommendation is NO -- derive exactly what's listed, let users be explicit.

3. **Newline handling between `end` and `deriving`:**
   The success criteria shows `end deriving(...)` on the same line. Should `end\nderiving(...)` also work? Since the parser's `eat_newlines()` at top level would already have consumed the newline, the `deriving` on the next line would be parsed as a new statement. Need to decide if `deriving` must be on the same line as `end` or can be on the next line. **Recommendation:** Allow both -- after closing `end`, check for `deriving` before closing the STRUCT_DEF/SUM_TYPE_DEF node, using the parser's standard lookahead which skips insignificant newlines inside the still-open struct/sum node.

   **Actually:** Looking at the parser more carefully, after consuming `END_KW`, the parser closes the STRUCT_DEF node immediately. The `deriving` check must happen BEFORE the `p.close()` call. Since the struct definition is still "open" at that point, the parser's newline handling depends on delimiter depth -- and struct definitions don't use delimiters. This means newlines at top level ARE significant, so `end\nderiving(...)` would see `NEWLINE` before `deriving` and NOT parse it as part of the struct. **Solution:** Either require same-line syntax (`end deriving(...)`) or explicitly eat newlines before checking for deriving. Same-line is simpler and cleaner.

## Sources

### Primary (HIGH confidence)
- **Codebase investigation:** All findings verified against actual source code
  - `crates/snow-parser/src/parser/items.rs` -- struct/sum type parsing (lines 246-303, 654-711)
  - `crates/snow-parser/src/parser/mod.rs` -- top-level dispatch, newline handling
  - `crates/snow-parser/src/syntax_kind.rs` -- full SyntaxKind enum
  - `crates/snow-parser/src/ast/item.rs` -- StructDef, SumTypeDef AST accessors
  - `crates/snow-common/src/token.rs` -- TokenKind enum, keyword_from_str
  - `crates/snow-typeck/src/infer.rs` -- auto-register logic (lines 1459-1530, 1692-1746)
  - `crates/snow-typeck/src/builtins.rs` -- compiler-known trait registration
  - `crates/snow-codegen/src/mir/lower.rs` -- generate_debug_inspect_struct (1366), generate_eq_struct (1504), generate_ord_struct (1590), generate_hash_struct (2041), generate_*_sum variants
  - `crates/snow-codegen/src/mir/mod.rs` -- MirModule, MirFunction, MirStructDef, MirSumTypeDef
  - `crates/snow-fmt/src/walker.rs` -- walk_struct_def, walk_block_def formatting

### Secondary (MEDIUM confidence)
- Phase 20 research (`20-RESEARCH.md`) -- trait registration patterns, operator dispatch
- Phase 21 research (`21-RESEARCH.md`) -- Hash protocol, Default protocol, FNV-1a implementation
- v1.3 requirements (`v1.3-REQUIREMENTS.md`) -- DERIV-01, DERIV-02, DERIV-03 specifications

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- verified all crate changes against actual source code
- Architecture: HIGH -- all six patterns modeled on existing working code in the compiler
- Pitfalls: HIGH -- identified from actual code paths and their current behavior
- New generation (Display, Hash-sum): MEDIUM -- modeled on existing patterns but not yet implemented

**Research date:** 2026-02-08
**Valid until:** 2026-03-08 (stable compiler internals, no expected upstream changes)
