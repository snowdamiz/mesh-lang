# Phase 74: Associated Types - Research

**Researched:** 2026-02-13
**Domain:** Compiler type system extension -- associated type declarations, bindings, and projection normalization
**Confidence:** HIGH

## Summary

Associated types are the foundational feature for the entire v7.0 milestone. Every subsequent phase depends on them: Iterator needs `type Item`, numeric traits need `type Output`, Collect needs `type Output`, and From/Into uses generic type parameters alongside associated types. This phase must add the ability to declare type members in trait (interface) definitions, bind them in impl blocks, reference them via `Self.Item` in method signatures, and resolve them to concrete types during Hindley-Milner type inference.

The current codebase has a complete trait system (`TraitRegistry`, `TraitDef`, `ImplDef` in `mesh-typeck/src/traits.rs`) but zero support for associated types. The `TraitDef` struct only stores method signatures (`Vec<TraitMethodSig>`), and `ImplDef` only stores method implementations (`FxHashMap<String, ImplMethodSig>`). Neither has any field for associated type declarations or bindings. The parser (`mesh-parser/src/parser/items.rs`) parses `interface` and `impl` blocks but only recognizes method signatures (`fn`/`def`), not `type` declarations.

The critical technical insight is that Mesh's monomorphization-based static dispatch model makes associated type resolution significantly simpler than in Rust. Every trait method call is statically dispatched to a concrete `Trait__Method__Type` function. This means every associated type projection MUST normalize to a concrete type before MIR lowering -- if it doesn't, that's a type error, not something codegen needs to handle. The recommended approach is **eager normalization**: whenever `Self.Item` is referenced, immediately look up the implementing type's impl and substitute the concrete associated type. This keeps the HM unifier (`unify.rs`) completely unchanged.

**Primary recommendation:** Extend `TraitDef`/`ImplDef` with associated type storage, add `type X` parsing to interface/impl blocks, implement eager projection normalization in the type checker, and fix `freshen_type_params` to handle multi-character type parameter names. Do NOT add a `Ty::Projection` variant that participates in general unification.

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| mesh-parser | in-tree | CST/AST parsing for `type Item` syntax | Existing Mesh parser; rowan-based CST |
| mesh-typeck | in-tree | Type inference, trait registry, unification | Existing HM inference engine with ena union-find |
| mesh-codegen | in-tree | MIR lowering and LLVM codegen | Name mangling must include associated type bindings |
| ena | 0.14 | Union-find for type variable unification | Already used; no changes needed for associated types |
| rustc-hash | 1.x | FxHashMap for trait/impl storage | Already used throughout; fast hashing for compiler internals |
| rowan | 0.15 | CST node types for new ASSOC_TYPE_DEF syntax nodes | Already used by mesh-parser |

### Supporting
No new external dependencies are needed. All changes are internal to the three existing crates.

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Eager normalization | Deferred constraints (Ty::Projection in unifier) | Deferred is more general but breaks HM principal types; eager is simpler and sufficient for Mesh's monomorphization model |
| Extending freshen_type_params heuristic | Switching to explicit Ty::Param variant | Explicit param tracking is cleaner long-term but is a larger refactor; extending the heuristic is pragmatic for v7.0 |

## Architecture Patterns

### Current State (Before Phase 74)

The trait system pipeline is:

```
Parser (interface/impl blocks)
  -> AST (InterfaceDef, ImplDef nodes)
    -> Type Checker (infer_interface_def, infer_impl_def)
      -> TraitRegistry (TraitDef, ImplDef structs)
        -> MIR Lowerer (reads TraitRegistry for dispatch)
          -> Codegen (mangled function names for trait methods)
```

Key structures that need modification:

```rust
// traits.rs - CURRENT (no associated types)
pub struct TraitDef {
    pub name: String,
    pub methods: Vec<TraitMethodSig>,
    // MISSING: associated_types: Vec<AssocTypeDef>
}

pub struct ImplDef {
    pub trait_name: String,
    pub impl_type: Ty,
    pub impl_type_name: String,
    pub methods: FxHashMap<String, ImplMethodSig>,
    // MISSING: associated_types: FxHashMap<String, Ty>
}
```

### Recommended Project Structure (Changes)

```
crates/mesh-parser/src/
├── syntax_kind.rs          # Add ASSOC_TYPE_DEF, ASSOC_TYPE_BINDING SyntaxKinds
├── parser/items.rs         # Add parse_assoc_type_decl, parse_assoc_type_binding
├── ast/item.rs             # Add AssocTypeDef AST node, extend InterfaceDef/ImplDef

crates/mesh-typeck/src/
├── traits.rs               # Add associated_types to TraitDef/ImplDef
│                           # Fix freshen_type_params for multi-char names
│                           # Add resolve_associated_type method to TraitRegistry
├── ty.rs                   # (Potentially) Add Ty::Projection for deferred cases
├── unify.rs                # NO changes if eager normalization is used
├── error.rs                # Add MissingAssocType, ExtraAssocType, UnresolvedProjection errors
├── infer.rs                # Extend infer_interface_def to collect assoc types
│                           # Extend infer_impl_def to validate assoc type bindings
│                           # Add Self.Item resolution in method signature inference
├── lib.rs                  # ExportedSymbols already carries TraitDef/ImplDef (auto-updated)

crates/mesh-codegen/src/
├── mir/types.rs            # Extend mangle_type_name to include assoc type bindings
├── mir/lower.rs            # May need to pass associated type info during dispatch mangling
```

### Pattern 1: Eager Projection Normalization

**What:** When `Self.Item` is encountered in a method signature or body, immediately look up the implementing type's impl and substitute the concrete associated type. Never insert an unresolved projection into the unification table.

**When to use:** Always, for this phase. Mesh does not need deferred projections because all trait method calls are statically dispatched -- the implementing type is always known at the call site.

**Example:**
```rust
// In infer_impl_def, when processing method signatures:
// 1. Parse the method's return type annotation
// 2. If it contains "Self.Item", look up the associated type binding
//    from the current impl's associated_types map
// 3. Substitute the concrete type before entering it into the InferCtx

// Pseudocode for resolution:
fn resolve_self_assoc_type(
    ty_name: &str,  // e.g., "Item"
    impl_assoc_types: &FxHashMap<String, Ty>,
) -> Option<Ty> {
    impl_assoc_types.get(ty_name).cloned()
}

// In resolve_type_name (or equivalent), when we see "Self.X":
// - Look up X in the current impl's associated type bindings
// - Return the concrete Ty
// - If not found, emit MissingAssocType error
```

### Pattern 2: Associated Type Validation at Registration

**What:** When `register_impl` is called, validate that the impl provides exactly the associated types declared by the trait -- no missing, no extra.

**When to use:** Every impl registration.

**Example:**
```rust
// In TraitRegistry::register_impl, after checking methods:
if let Some(trait_def) = self.traits.get(&impl_def.trait_name).cloned() {
    // Check for missing associated types
    for assoc in &trait_def.associated_types {
        if !impl_def.associated_types.contains_key(&assoc.name) {
            errors.push(TypeError::MissingAssocType {
                trait_name: impl_def.trait_name.clone(),
                assoc_name: assoc.name.clone(),
                impl_ty: impl_def.impl_type_name.clone(),
            });
        }
    }
    // Check for extra associated types
    for (name, _) in &impl_def.associated_types {
        if !trait_def.associated_types.iter().any(|a| &a.name == name) {
            errors.push(TypeError::ExtraAssocType {
                trait_name: impl_def.trait_name.clone(),
                assoc_name: name.clone(),
                impl_ty: impl_def.impl_type_name.clone(),
            });
        }
    }
}
```

### Pattern 3: freshen_type_params Extension

**What:** The current `freshen_type_params` function only treats single uppercase ASCII letters as type parameters. Associated type names like "Item" and "Output" would NOT be freshened, causing false negatives in structural matching. The function must be extended to also freshen known associated type names.

**When to use:** Whenever the trait registry does structural type matching (has_impl, find_impl, etc.)

**Example:**
```rust
// Current heuristic (traits.rs:346):
if c.name.len() == 1 && c.name.as_bytes()[0].is_ascii_uppercase() {
    // freshen
}

// Extended approach: pass the trait's declared type param names
// and associated type names as an additional parameter
fn freshen_type_params(
    ty: &Ty,
    ctx: &mut InferCtx,
    type_param_names: &[String],  // NEW: explicit list of type param names
) -> Ty {
    // ...
    Ty::Con(c) => {
        if c.name.len() == 1 && c.name.as_bytes()[0].is_ascii_uppercase() {
            // existing behavior
        } else if type_param_names.contains(&c.name) {
            // NEW: explicit type parameter names
            param_map.entry(c.name.clone())
                .or_insert_with(|| ctx.fresh_var())
                .clone()
        } else {
            ty.clone()
        }
    }
}
```

### Anti-Patterns to Avoid

- **Adding Ty::Projection to general unification:** This would break the principal types property of HM. Associated type projections are NOT injective (`<T as Iterator>::Item = Int` does NOT imply a unique T). Never let an unresolved projection enter the unification table.
- **Lazy/deferred projection resolution:** Adds complexity for no benefit in Mesh's monomorphization model. Always resolve eagerly.
- **Storing associated types only in method return types:** The associated type bindings must be stored separately on `ImplDef`, not embedded in method signatures. Methods reference associated types by name; the bindings map names to concrete types.
- **Changing existing TraitMethodSig to carry associated type info:** Keep `TraitMethodSig` focused on method signatures. Associated types are a trait-level concept, not a method-level concept.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Union-find unification | Custom unification | Existing `InferCtx` with `ena` | Already proven, handles occurs check, levels, generalization |
| Structural type matching | Custom pattern matcher | Existing `freshen_type_params` + `InferCtx::unify` | Already handles generic impls (List<T>) correctly; just needs extension for multi-char names |
| CST node creation | Manual token parsing | Existing `ast_node!` macro + `SyntaxKind` enum | Pattern is well-established throughout the parser |
| Name mangling | Custom string builder | Extend existing `mangle_type_name` in `mir/types.rs` | Must be consistent with existing mangling scheme to avoid linker conflicts |

**Key insight:** This phase extends existing infrastructure at every layer. No new crates, no new external dependencies, no new architectural patterns. Every change is an extension of an existing mechanism.

## Common Pitfalls

### Pitfall 1: Breaking HM Principal Types with Ty::Projection
**What goes wrong:** Adding a `Ty::Projection` variant that participates in unification breaks the injective type constructor assumption. `<T as Trait>::Item = Int` does NOT imply a unique T, so the unifier cannot determine principal types.
**Why it happens:** Temptation to model associated types as first-class type terms in the HM framework.
**How to avoid:** Use eager normalization. Resolve all `Self.Item` references to concrete types before they enter the unification table. The InferCtx and unifier remain unchanged.
**Warning signs:** If you find yourself modifying `unify()` or `occurs_in()` to handle projections, you're on the wrong path.

### Pitfall 2: freshen_type_params Single-Letter Heuristic
**What goes wrong:** The function in `traits.rs:333-381` treats only single uppercase ASCII letters as type parameters. Multi-character names like "Item", "Output", "Iter" are treated as concrete types, causing `find_impl()` and `has_impl()` to fail for impls that reference associated types in their type signatures.
**Why it happens:** The heuristic was designed before associated types existed. It works for generic type params (T, K, V) but not for associated type names.
**How to avoid:** Either (a) pass explicit type parameter names to `freshen_type_params`, or (b) use a naming convention/marker to distinguish type params from concrete types. Option (a) is recommended.
**Warning signs:** Tests pass when using single-letter type params but fail with multi-character names.

### Pitfall 3: Missing Associated Type Validation
**What goes wrong:** An impl block is registered without providing all required associated types, or provides extra ones. Without validation, this creates a "time bomb" that explodes later when the missing type is referenced.
**Why it happens:** Current `register_impl()` only validates methods, not associated types.
**How to avoid:** Add associated type validation to `register_impl()`, parallel to the existing method validation (traits.rs:102-114). Check for missing AND extra associated types.
**Warning signs:** Programs compile but crash or produce wrong types when associated types are referenced.

### Pitfall 4: Name Mangling Collision
**What goes wrong:** Two impls with different associated type bindings produce the same mangled function name, causing linker errors or silent misbehavior.
**Why it happens:** Current mangling (`Trait__method__Type`) does not include associated type bindings.
**How to avoid:** Include associated type bindings in the mangled name: `Trait__method__Type__AssocName_ConcreteType`. This only matters when associated types affect the function's behavior (return type, etc.).
**Warning signs:** Linker reports duplicate symbol errors in programs with multiple iterator types.

### Pitfall 5: Cross-Module Export Missing Associated Types
**What goes wrong:** `ExportedSymbols` carries `Vec<TraitDef>` and `Vec<TraitImplDef>` (lib.rs:107-109). If `TraitDef` and `ImplDef` are extended with associated type fields, the export mechanism automatically includes them. But if the extension is done wrong (e.g., storing associated types in a separate map outside the structs), cross-module trait resolution fails.
**Why it happens:** The export path is: `TraitRegistry` -> `ExportedSymbols` -> `ImportContext` -> destination module's `TraitRegistry`. If associated types are stored outside the `TraitDef`/`ImplDef` structs, this pipeline doesn't carry them.
**How to avoid:** Store associated types INSIDE `TraitDef` and `ImplDef` structs. The existing export/import pipeline clones these structs, so the associated type data flows automatically.
**Warning signs:** Associated types work in single-file mode but fail in multi-module programs.

### Pitfall 6: Self.Item Resolution Context
**What goes wrong:** `Self.Item` is referenced outside an impl block (e.g., in a standalone function) where there is no implementing type to look up the associated type binding.
**Why it happens:** `Self` is only meaningful inside an impl block. If the parser allows `Self.Item` anywhere and the type checker doesn't check the context, the resolution will fail with a confusing error.
**How to avoid:** The type checker should only resolve `Self.Item` when a current impl context is active. Track the "current impl" (trait name + impl type + associated type bindings) on a stack during `infer_impl_def`. Outside impl context, `Self.Item` produces a clear error: "Self.Item can only be used inside an impl block".
**Warning signs:** Confusing "unresolved type" errors when `Self.Item` is used in standalone functions.

### Pitfall 7: Parser Ambiguity with Type Aliases
**What goes wrong:** The parser already handles `type Name = Type` as `TypeAliasDef` at the top level. Inside an interface or impl block, `type Item = Int` must be parsed as an associated type binding, not a type alias. If the parser uses the same code path, it creates ambiguity.
**Why it happens:** Both `type X` (associated type declaration) and `type X = Y` (associated type binding) start with the `type` keyword, which is also used for top-level type aliases.
**How to avoid:** Inside interface/impl blocks, parse `type` as associated type syntax, not type alias syntax. The parser already distinguishes context: `parse_interface_method` and `parse_item_block_body` are called inside interface/impl blocks respectively. Add `type` handling to these specific parse functions.
**Warning signs:** `type Item` inside an interface parses as a TypeAliasDef instead of an AssocTypeDef.

## Code Examples

### Example 1: User-facing Syntax

```mesh
# In interface definitions: declare associated types
interface Iterator do
  type Item
  fn next(self) -> Option<Self.Item>
end

# In impl blocks: bind associated types to concrete types
impl Iterator for ListIter<T> do
  type Item = T
  fn next(self) -> Option<T> do
    # ... implementation
  end
end

# Usage in generic functions
fn first_item<T>(iter :: T) -> Option<T.Item> where T: Iterator do
  iter.next()
end
```

### Example 2: TraitDef Extension (Rust)

```rust
// Source: crates/mesh-typeck/src/traits.rs (to be extended)

/// An associated type declaration in a trait.
#[derive(Clone, Debug)]
pub struct AssocTypeDef {
    /// The associated type name (e.g., "Item", "Output").
    pub name: String,
    // Future: bounds like `where Item: Display`
}

/// A trait (interface) definition.
#[derive(Clone, Debug)]
pub struct TraitDef {
    pub name: String,
    pub methods: Vec<TraitMethodSig>,
    /// Associated type declarations (e.g., `type Item` in interface body).
    pub associated_types: Vec<AssocTypeDef>,  // NEW
}

/// An impl registration.
#[derive(Clone, Debug)]
pub struct ImplDef {
    pub trait_name: String,
    pub impl_type: Ty,
    pub impl_type_name: String,
    pub methods: FxHashMap<String, ImplMethodSig>,
    /// Associated type bindings (e.g., `type Item = Int`).
    pub associated_types: FxHashMap<String, Ty>,  // NEW
}
```

### Example 3: Associated Type Resolution in TraitRegistry

```rust
// Source: crates/mesh-typeck/src/traits.rs (new method)

impl TraitRegistry {
    /// Resolve an associated type for a concrete implementing type.
    ///
    /// Given trait "Iterator", associated type "Item", and concrete type List<Int>,
    /// finds the impl and returns the bound type (e.g., Int).
    pub fn resolve_associated_type(
        &self,
        trait_name: &str,
        assoc_name: &str,
        impl_ty: &Ty,
    ) -> Option<Ty> {
        let impl_def = self.find_impl(trait_name, impl_ty)?;
        let bound_ty = impl_def.associated_types.get(assoc_name)?;
        Some(bound_ty.clone())
    }
}
```

### Example 4: New TypeError Variants

```rust
// Source: crates/mesh-typeck/src/error.rs (new variants)

pub enum TypeError {
    // ... existing variants ...

    /// An impl block is missing a required associated type.
    MissingAssocType {
        trait_name: String,
        assoc_name: String,
        impl_ty: String,
    },
    /// An impl block provides an associated type not declared by the trait.
    ExtraAssocType {
        trait_name: String,
        assoc_name: String,
        impl_ty: String,
    },
    /// An associated type reference (Self.Item) could not be resolved.
    UnresolvedAssocType {
        trait_name: String,
        assoc_name: String,
        span: TextRange,
    },
}
```

### Example 5: Parser Changes for Interface Body

```rust
// Source: crates/mesh-parser/src/parser/items.rs (extended)

// Inside parse_interface_def's body loop, before calling parse_interface_method:
loop {
    p.eat_newlines();
    if p.at(SyntaxKind::END_KW) || p.at(SyntaxKind::EOF) {
        break;
    }

    // NEW: Check for associated type declaration
    if p.at(SyntaxKind::TYPE_KW) {
        parse_assoc_type_decl(p);  // `type Item`
    } else {
        parse_interface_method(p);  // `fn next(self) -> ...`
    }

    if p.has_error() {
        break;
    }
}

// New function:
fn parse_assoc_type_decl(p: &mut Parser) {
    let m = p.open();
    p.advance(); // TYPE_KW
    if p.at(SyntaxKind::IDENT) {
        let name = p.open();
        p.advance();
        p.close(name, SyntaxKind::NAME);
    }
    p.close(m, SyntaxKind::ASSOC_TYPE_DEF);
}
```

### Example 6: Parser Changes for Impl Body

```rust
// Source: crates/mesh-parser/src/parser/items.rs (extended)

// Inside parse_item_block_body (called from parse_impl_def),
// add handling for `type Item = ConcreteType`:
if p.at(SyntaxKind::TYPE_KW) {
    parse_assoc_type_binding(p);  // `type Item = Int`
} else if p.at(SyntaxKind::FN_KW) || p.at(SyntaxKind::DEF_KW) {
    parse_fn_def(p);
}

fn parse_assoc_type_binding(p: &mut Parser) {
    let m = p.open();
    p.advance(); // TYPE_KW
    if p.at(SyntaxKind::IDENT) {
        let name = p.open();
        p.advance();
        p.close(name, SyntaxKind::NAME);
    }
    p.expect(SyntaxKind::EQ); // =
    parse_type(p);  // The concrete type
    p.close(m, SyntaxKind::ASSOC_TYPE_BINDING);
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| No associated types | Adding associated types | Phase 74 (now) | Enables Iterator, numeric Output, Collect |
| Single-letter type param heuristic | Must support multi-char names | Phase 74 (now) | freshen_type_params needs update |
| TraitDef/ImplDef without assoc types | TraitDef/ImplDef with assoc types | Phase 74 (now) | Struct extensions, validation logic |
| Method-only interfaces | Interfaces with type members | Phase 74 (now) | Parser, AST, type checker changes |

**Deprecated/outdated:**
- The single-letter type parameter heuristic in `freshen_type_params` becomes insufficient once associated type names like "Item" and "Output" enter the trait system. It must be extended.

## Requirement Mapping

| Requirement | What It Needs | Implementation Approach |
|-------------|---------------|------------------------|
| ASSOC-01: Declare assoc types in interface | Parser: `type Item` syntax; AST: AssocTypeDef node; TypeChecker: TraitDef storage | New ASSOC_TYPE_DEF SyntaxKind, parse_assoc_type_decl, extend infer_interface_def |
| ASSOC-02: Specify assoc types in impl | Parser: `type Item = T` syntax; AST: AssocTypeBinding node; TypeChecker: ImplDef storage | New ASSOC_TYPE_BINDING SyntaxKind, parse_assoc_type_binding, extend infer_impl_def |
| ASSOC-03: Reference via Self.Item | TypeChecker: resolve Self.Item to concrete type | Eager normalization during method signature inference; impl context stack |
| ASSOC-04: Normalize projections during inference | TypeChecker: when trait method is called on concrete type, resolve associated types | TraitRegistry::resolve_associated_type + substitution in method return types |
| ASSOC-05: Clear error messages | Error variants for missing/extra/unresolved | MissingAssocType, ExtraAssocType, UnresolvedAssocType TypeError variants |

## File Touch Points

Complete list of files that need modification in Phase 74:

### mesh-parser (CST/AST)
1. **`syntax_kind.rs`** -- Add `ASSOC_TYPE_DEF`, `ASSOC_TYPE_BINDING` SyntaxKind variants
2. **`parser/items.rs`** -- Add `parse_assoc_type_decl`, `parse_assoc_type_binding`; modify interface/impl body parsing to recognize `type` keyword
3. **`ast/item.rs`** -- Add `AssocTypeDef`, `AssocTypeBinding` AST node types; extend `InterfaceDef` with `assoc_types()` method; extend `ImplDef` with `assoc_type_bindings()` method

### mesh-typeck (Type System)
4. **`traits.rs`** -- Add `AssocTypeDef` struct; add `associated_types` field to `TraitDef` and `ImplDef`; add `resolve_associated_type` method to `TraitRegistry`; extend `register_impl` validation for associated types; fix `freshen_type_params` for multi-character type param names
5. **`error.rs`** -- Add `MissingAssocType`, `ExtraAssocType`, `UnresolvedAssocType` variants to `TypeError`; add Display impls
6. **`infer.rs`** -- Extend `infer_interface_def` to collect associated type declarations; extend `infer_impl_def` to collect and validate associated type bindings; add `Self.Item` resolution in method body/signature inference
7. **`lib.rs`** -- `ExportedSymbols` automatically updated (carries `TraitDef`/`ImplDef` which will include new fields); no explicit changes expected unless the struct cloning path needs adjustment

### mesh-codegen (Code Generation)
8. **`mir/types.rs`** -- Extend `mangle_type_name` to include associated type bindings in mangled names (to avoid collisions)
9. **`mir/lower.rs`** -- When lowering trait method calls, pass associated type info to ensure correct mangling

### Test Files
10. **New test files** -- Tests for associated type declaration, binding, Self.Item resolution, error cases, cross-module usage

## Open Questions

1. **Generic function calls with associated types**
   - What we know: When a generic function takes `T: Iterator` and calls `T.next()`, monomorphization creates a specialized version for each concrete T. The return type `Option<T.Item>` must be resolved to `Option<ConcreteItem>` at the call site.
   - What's unclear: The exact mechanism for threading associated type bindings through generic function instantiation. Does `InferCtx::instantiate()` need to handle associated types, or is it handled at the call site when the concrete type is known?
   - Recommendation: Handle at the call site. When `first_item(my_list_iter)` is called, the concrete type `ListIter<Int>` is known, so `T.Item` resolves to `Int` immediately. The generic function's body is type-checked once with `T.Item` as a placeholder that gets resolved during monomorphization.

2. **Deferred projection for not-yet-resolved types**
   - What we know: In most cases, the implementing type is known when `Self.Item` is referenced (inside impl blocks, or at call sites with concrete types).
   - What's unclear: Are there cases where the implementing type is an inference variable that hasn't been resolved yet when `Self.Item` is encountered?
   - Recommendation: If the implementing type is unresolved, defer the projection by recording a constraint "when ?T is resolved, look up ?T's Item associated type." This can be implemented as a post-inference normalization pass. However, for v7.0, it may be simpler to require explicit type annotations in ambiguous cases.

3. **Associated type defaults**
   - What we know: Rust supports `type Item = DefaultType` in trait definitions. The v7.0 requirements (ASSOC-01 through ASSOC-05) do not mention defaults.
   - What's unclear: Should we support defaults in Phase 74 for future-proofing?
   - Recommendation: Do NOT implement defaults in Phase 74. Keep it simple: every impl must provide every associated type. Defaults can be added later without breaking changes.

## Sources

### Primary (HIGH confidence)
- `crates/mesh-typeck/src/traits.rs` -- TraitRegistry, TraitDef (no assoc types), ImplDef (no assoc types), freshen_type_params (single-letter heuristic)
- `crates/mesh-typeck/src/ty.rs` -- Ty enum (Var, Con, Fun, App, Tuple, Never; no Projection variant)
- `crates/mesh-typeck/src/unify.rs` -- InferCtx, unification, occurs check, generalization (no projection handling)
- `crates/mesh-typeck/src/error.rs` -- TypeError variants (no associated type errors)
- `crates/mesh-typeck/src/infer.rs` -- infer_interface_def (lines 2675-2742), infer_impl_def (lines 2744-2895)
- `crates/mesh-typeck/src/lib.rs` -- ExportedSymbols (trait_defs, trait_impls fields)
- `crates/mesh-typeck/src/builtins.rs` -- Compiler-known trait registration pattern (1422 lines)
- `crates/mesh-parser/src/parser/items.rs` -- parse_interface_def (line 484), parse_impl_def (line 606), parse_interface_method (line 547)
- `crates/mesh-parser/src/ast/item.rs` -- InterfaceDef, ImplDef AST nodes (no assoc type accessors)
- `crates/mesh-parser/src/syntax_kind.rs` -- SyntaxKind enum (no ASSOC_TYPE_DEF/BINDING kinds)
- `.planning/research/SUMMARY.md` -- v7.0 research summary confirming associated types as foundation
- `.planning/research/PITFALLS.md` -- 15 pitfalls; Pitfalls 1, 5, 8, 9, 13, 15 directly relevant
- `.planning/REQUIREMENTS.md` -- ASSOC-01 through ASSOC-05 requirements
- `.planning/ROADMAP.md` -- Phase 74 roadmap entry

### Secondary (MEDIUM confidence)
- Rust trait system design -- associated type normalization, projection resolution patterns
- Chalk unification (Niko Matsakis blog) -- deferred projection constraint resolution
- GHC type families -- injectivity requirements for type-level functions

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- no new dependencies; all changes to existing crates verified against source code
- Architecture: HIGH -- all integration points identified and verified with exact line numbers in source
- Pitfalls: HIGH -- 7 pitfalls identified from codebase analysis; 6 confirmed by cross-referencing with domain PITFALLS.md
- Code examples: HIGH -- patterns derived directly from existing codebase conventions

**Research date:** 2026-02-13
**Valid until:** 2026-03-13 (stable -- compiler internals don't change externally)
