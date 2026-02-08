# Phase 23: Pattern Matching Codegen - Research

**Researched:** 2026-02-08
**Domain:** LLVM codegen for sum type pattern matching (Rust compiler internals)
**Confidence:** HIGH

## Summary

This phase fixes two specific limitations carried from v1.3: (1) pattern matching on non-nullary sum type variants does not extract field values at the LLVM codegen level, and (2) the Ordering sum type (Less | Equal | Greater) is not user-visible. The MIR layer already produces correct representations -- the bugs are entirely in the codegen layer and the type registration layer.

After thorough investigation of the codebase (pattern compiler in `snow-codegen/src/pattern/compile.rs`, decision tree codegen in `snow-codegen/src/codegen/pattern.rs`, variant construction in `snow-codegen/src/codegen/expr.rs`, type system in `snow-typeck/src/builtins.rs`), three root-cause bugs have been identified:

1. **Constructor tag mismatch**: The pattern compiler (`collect_head_constructors`) assigns tags by first-appearance order in user patterns rather than looking up the actual tag from the sum type definition. This causes the LLVM `switch` instruction to dispatch to wrong case blocks.
2. **Variant field type resolution uses Unit placeholders**: When `specialize_for_constructor` expands sub-pattern columns, it uses `MirType::Unit` as a placeholder for field types. When variable bindings fall back to column types, they get `MirType::Unit` instead of the correct field type (e.g., `MirType::Int`), causing incorrect LLVM type loads.
3. **Ordering type not registered**: Option and Result are registered as built-in sum types in `register_builtin_sum_types()`, but Ordering is not. The Ord trait defines `lt() -> Bool` but not `compare() -> Ordering`.

**Primary recommendation:** Fix the three root-cause bugs in order -- tag mismatch first (pattern compiler), then field type resolution (pattern compiler), then register the Ordering type (typeck builtins + MIR lowerer). These are surgical fixes to existing infrastructure, not new feature development.

## Standard Stack

This is internal compiler work. No new external dependencies needed.

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| inkwell | 0.8 | LLVM 21 bindings for Rust | Already in use, all codegen goes through it |
| rustc_hash | (existing) | FxHashMap for codegen caches | Already in use for type/layout caches |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| snow-typeck | (internal) | Type registry, trait registry | For registering Ordering sum type and compare method |
| snow-codegen | (internal) | Pattern compiler + LLVM codegen | Primary modification target |

No new dependencies required. All work is within existing crates.

## Architecture Patterns

### Relevant Project Structure
```
crates/
  snow-codegen/src/
    pattern/
      mod.rs              # AccessPath, ConstructorTag, DecisionTree types
      compile.rs          # Maranget pattern matrix -> decision tree compiler
    codegen/
      pattern.rs          # Decision tree -> LLVM IR translation
      expr.rs             # Expression codegen (ConstructVariant, codegen_match)
      types.rs            # MirType -> LLVM type mapping (variant_struct_type)
      mod.rs              # CodeGen struct, sum_type_defs/layouts caches
    mir/
      mod.rs              # MirType, MirPattern, MirSumTypeDef, MirVariantDef
      lower.rs            # AST -> MIR lowering (lower_pattern, lower_sum_type_def)
      types.rs            # Ty -> MirType conversion
  snow-typeck/src/
    builtins.rs           # Built-in type/trait registration (Option, Result, Ord)
    infer.rs              # Type inference, SumTypeDefInfo, TypeRegistry
```

### Pattern 1: Tag Assignment from Type Definition
**What:** Constructor tags MUST come from the `MirSumTypeDef` (which assigns sequential tags 0, 1, 2, ... based on variant declaration order), not from pattern appearance order.
**Where the bug is:** `crates/snow-codegen/src/pattern/compile.rs`, function `collect_head_constructors`, lines 347-351.
**Current (broken) code:**
```rust
// Assign tags based on order of first appearance.
let tag = result
    .iter()
    .filter(|c| matches!(c, HeadCtor::Constructor { .. }))
    .count() as u8;
```
**What it should do:** Look up the actual tag from the `MirPattern::Constructor` field or the sum type definition. The `MirPattern::Constructor` does not currently carry the tag value -- but the pattern compiler can resolve it from the `type_name` using the same mechanism the codegen uses (`lookup_sum_type_def`). However, the pattern compiler runs before codegen has its caches populated. The simplest fix: the `MirPattern::Constructor` already knows its `type_name` and `variant`, and the `PatMatrix` has column types. The pattern compiler needs access to sum type variant info to assign correct tags.

**Recommended approach:** Pass a sum type variant-tag lookup function (or a map of `(type_name, variant_name) -> tag`) into `compile_match()`, and use it in `collect_head_constructors` to assign the correct tag value. Alternatively, since the codegen has `sum_type_defs` populated before calling `compile_match`, it could pass the sum type defs through or have `compile_match` accept them as a parameter.

### Pattern 2: Variant Field Type Resolution
**What:** When the pattern compiler specializes a matrix for a constructor, it creates new columns for the constructor's fields. These columns need correct types so that variable bindings get the right MIR type for LLVM codegen.
**Where the bug is:** `crates/snow-codegen/src/pattern/compile.rs`, function `specialize_for_constructor`, lines 519-521.
**Current (broken) code:**
```rust
// We don't know the exact field type here, use Unit as placeholder.
// The actual type is carried by the variable patterns themselves.
new_types.push(MirType::Unit);
```
**Why it matters:** When a variable pattern `Var(name, ty)` has `ty == MirType::Unit` (e.g., unresolved), `collect_bindings_from_row` falls back to the column type (line 251-256), which is also Unit. The generated binding then has type Unit, but the actual value is an Int (or whatever the variant field type is). This causes LLVM to load an `{}` (empty struct) instead of an `i64`, producing "Instruction does not dominate all uses" or similar errors.

**Recommended approach:** The `MirPattern::Constructor` has a `type_name` and `variant` name. Use the sum type definitions to look up the actual field types. This requires passing sum type defs into the pattern compiler, same as for tag resolution.

### Pattern 3: Registering Built-in Sum Types
**What:** New built-in sum types are registered in `register_builtin_sum_types()` in `snow-typeck/src/infer.rs` (lines 652-721). Follow the exact same pattern as Option and Result.
**Example (Option registration):**
```rust
type_registry.register_sum_type(SumTypeDefInfo {
    name: "Option".to_string(),
    generic_params: vec!["T".to_string()],
    variants: vec![
        VariantInfo { name: "Some".to_string(), fields: vec![VariantFieldInfo::Positional(Ty::Con(TyCon::new("T")))] },
        VariantInfo { name: "None".to_string(), fields: vec![] },
    ],
});
register_variant_constructors(ctx, env, "Option", &option_generic_params, &option_variants);
```
**For Ordering:**
```rust
type_registry.register_sum_type(SumTypeDefInfo {
    name: "Ordering".to_string(),
    generic_params: vec![],  // Not generic
    variants: vec![
        VariantInfo { name: "Less".to_string(), fields: vec![] },
        VariantInfo { name: "Equal".to_string(), fields: vec![] },
        VariantInfo { name: "Greater".to_string(), fields: vec![] },
    ],
});
register_variant_constructors(ctx, env, "Ordering", &[], &ordering_variants);
```

### Anti-Patterns to Avoid
- **Assigning constructor tags from pattern order:** Tags MUST match the type definition. The type definition assigns tag 0 to the first variant, tag 1 to the second, etc. If a user writes `None -> ... | Some(x) -> ...`, None's tag must still be 1 (its position in `Option`'s definition: `Some=0, None=1`).
- **Using MirType::Unit as a placeholder type that propagates to codegen:** Placeholder types must be resolved before they reach the LLVM codegen layer. If a type truly cannot be resolved, it should produce a clear error, not silently generate wrong code.
- **Changing Ord trait's `lt()` method signature:** The existing `lt() -> Bool` mechanism works correctly for all 6 comparison operators. Ordering should be a SEPARATE `compare() -> Ordering` method, not a replacement.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Sum type variant-to-tag mapping | Ad-hoc tag lookup in pattern compiler | Reuse `MirSumTypeDef` variant definitions already in the module | Single source of truth for tag assignments |
| Variant field type resolution | Guessing types from patterns | Look up field types from `MirSumTypeDef.variants[i].fields` | Correct types for LLVM GEP and load instructions |
| LLVM struct overlay for variant fields | Custom layout calculation | `variant_struct_type()` in `codegen/types.rs` | Already handles `{ i8 tag, field0, field1, ... }` layout |
| Decision tree for pattern matching | Manual if/else chains | Existing Maranget algorithm in `pattern/compile.rs` | Already compiles constructor patterns, just needs correct tags/types |

**Key insight:** The entire pattern matching infrastructure (Maranget compiler, decision tree, access paths, LLVM switch codegen, variant overlay GEP) already works correctly. The bugs are specifically in (a) tag assignment in the pattern compiler and (b) type placeholders that propagate incorrectly. Both are ~10-line fixes each.

## Common Pitfalls

### Pitfall 1: Tag Mismatch Between Pattern Compiler and Type Definition
**What goes wrong:** The pattern compiler assigns constructor tags based on the order constructors appear in the user's pattern, not the order they appear in the type definition. When the user writes `case opt do None -> 0 | Some(x) -> x end`, the pattern compiler assigns `None=0, Some=1`, but the type definition has `Some=0, None=1`. The LLVM switch dispatches on tag 0 (which is Some in the actual value) and jumps to the None branch.
**Why it happens:** `collect_head_constructors` in `compile.rs` uses a counter of previously-seen constructors as the tag value (line 348), instead of looking up the actual tag from the sum type definition.
**How to avoid:** Pass sum type definitions into `compile_match()` so `collect_head_constructors` can look up `variant_def.tag` for each constructor.
**Warning signs:** Pattern matching on sum types produces wrong results when the arm order doesn't match the type definition's variant order. Nullary-only variants (like `Color`) may appear to work because all nullary variants have the same layout.

### Pitfall 2: Unit Type Placeholder Causing Wrong LLVM Types
**What goes wrong:** Variant field bindings get `MirType::Unit` instead of their actual type. When codegen tries to navigate `AccessPath::VariantField` and load the value, it loads an `{}` (empty LLVM struct) instead of an `i64` or other actual type. This produces "Instruction does not dominate all uses" or type mismatch LLVM verifier errors.
**Why it happens:** `specialize_for_constructor` uses `MirType::Unit` as a placeholder for new column types (line 521). The variable pattern's type may also be Unit if the typeck didn't resolve it. The fallback in `collect_bindings_from_row` (line 251-256) checks the column type, which is also Unit.
**How to avoid:** Resolve actual field types from the sum type definition when creating new columns during constructor specialization.
**Warning signs:** "Instruction does not dominate all uses" LLVM error. Tests pass for nullary variants (no fields to extract) but fail for non-nullary variants.

### Pitfall 3: Monomorphized Sum Type Name Lookup
**What goes wrong:** Generic sum types like `Option<Int>` get mangled names like `Option_Int`. The codegen's `lookup_sum_type_def` falls back from `Option_Int` to `Option` by splitting on `_`. But if a sum type has an underscore in its name (e.g., `My_Type`), this fallback breaks.
**Why it happens:** The fallback uses `name.split('_').next()` which is fragile.
**How to avoid:** For this phase, this is not a primary concern since Ordering has no generic params. Be aware of it but don't need to fix it.
**Warning signs:** "Unknown sum type" errors for monomorphized generic sum types.

### Pitfall 4: Ordering Variant Constructors Need Env Registration
**What goes wrong:** Registering Ordering in the `TypeRegistry` is necessary but not sufficient. The variant constructors (`Less`, `Equal`, `Greater`) must also be registered in the type environment as functions/values so users can reference them in expressions and patterns.
**Why it happens:** `register_variant_constructors` creates polymorphic schemes for each constructor. For nullary constructors (no fields), these are constants of type `Ordering`. Without this registration, writing `Less` in Snow code produces an "unknown identifier" error.
**How to avoid:** Call `register_variant_constructors(ctx, env, "Ordering", &[], &ordering_variants)` just like Option/Result do.
**Warning signs:** "Unknown identifier 'Less'" or similar type errors when trying to use Ordering constructors.

### Pitfall 5: Ord Trait compare() Method Needs Both Type System and Codegen Support
**What goes wrong:** Just adding a `compare()` method to the Ord trait definition without generating MIR implementations means calling `compare(a, b)` produces a "function not found" error at codegen.
**Why it happens:** The Ord trait currently only has `lt()`. Adding `compare()` requires: (a) updating the TraitDef, (b) adding ImplMethodSig entries, (c) generating `Ord__compare__Type` MIR functions for each type that implements Ord, and (d) handling the comparison operator dispatch in the lowerer.
**How to avoid:** Follow the same pattern as `generate_ord_struct`/`generate_ord_sum` for `lt()`, but returning `MirType::SumType("Ordering")` and constructing the appropriate variant instead of returning Bool.
**Warning signs:** Compile-time "unknown function" errors for `compare`, or runtime crashes if the function exists but returns wrong type.

## Code Examples

### Example 1: Fixing Tag Assignment in collect_head_constructors
```rust
// In compile.rs, collect_head_constructors should use the actual tag
// from the sum type definition, not first-appearance order.
//
// Option 1: Pass sum_type_defs into compile_match and thread through
// Option 2: The MirPattern::Constructor could carry the tag value
//           (set during MIR lowering from the type definition)

// The cleanest approach: look up tag from sum_type_defs map
fn collect_head_constructors(
    matrix: &PatMatrix,
    col: usize,
    sum_type_defs: &FxHashMap<String, MirSumTypeDef>,  // NEW PARAM
) -> Vec<HeadCtor> {
    // ...
    MirPattern::Constructor { type_name, variant, fields, .. } => {
        let key = format!("ctor:{}", variant);
        if !seen.contains(&key) {
            // Look up actual tag from sum type definition
            let tag = sum_type_defs.get(type_name)
                .and_then(|def| def.variants.iter().find(|v| v.name == *variant))
                .map(|v| v.tag)
                .unwrap_or(0);  // fallback to 0 if not found
            seen.push(key);
            result.push(HeadCtor::Constructor {
                type_name: type_name.clone(),
                variant: variant.clone(),
                tag,
                arity: fields.len(),
            });
        }
    }
    // ...
}
```

### Example 2: Fixing Field Type Resolution in specialize_for_constructor
```rust
// In compile.rs, specialize_for_constructor should use actual field types
// from the sum type definition instead of MirType::Unit placeholders.

fn specialize_for_constructor(
    matrix: &PatMatrix,
    col: usize,
    target_variant: &str,
    arity: usize,
    sum_type_defs: &FxHashMap<String, MirSumTypeDef>,  // NEW PARAM
) -> PatMatrix {
    // ... (existing row processing stays the same)

    // Sub-pattern paths for the constructor fields - with REAL types
    let parent_ty = &matrix.column_types[col];
    let field_types: Vec<MirType> = if let MirType::SumType(type_name) = parent_ty {
        sum_type_defs.get(type_name.as_str())
            // Also try base name for monomorphized types
            .or_else(|| type_name.split('_').next().and_then(|base| sum_type_defs.get(base)))
            .and_then(|def| def.variants.iter().find(|v| v.name == target_variant))
            .map(|v| v.fields.clone())
            .unwrap_or_else(|| vec![MirType::Unit; arity])
    } else {
        vec![MirType::Unit; arity]
    };

    for i in 0..arity {
        new_paths.push(AccessPath::VariantField(
            Box::new(parent_path.clone()),
            target_variant.to_string(),
            i,
        ));
        new_types.push(field_types.get(i).cloned().unwrap_or(MirType::Unit));
    }
    // ...
}
```

### Example 3: Registering Ordering as a Built-in Sum Type
```rust
// In snow-typeck/src/infer.rs, inside register_builtin_sum_types()

// Ordering (Less | Equal | Greater) -- no generic params
let ordering_variants = vec![
    VariantInfo { name: "Less".to_string(), fields: vec![] },
    VariantInfo { name: "Equal".to_string(), fields: vec![] },
    VariantInfo { name: "Greater".to_string(), fields: vec![] },
];

type_registry.register_sum_type(SumTypeDefInfo {
    name: "Ordering".to_string(),
    generic_params: vec![],
    variants: ordering_variants.clone(),
});

register_variant_constructors(ctx, env, "Ordering", &[], &ordering_variants);
```

### Example 4: Adding compare() to Ord Trait
```rust
// In snow-typeck/src/builtins.rs, update the Ord trait definition
registry.register_trait(TraitDef {
    name: "Ord".to_string(),
    methods: vec![
        TraitMethodSig {
            name: "lt".to_string(),
            has_self: true,
            param_count: 1,
            return_type: Some(Ty::bool()),
            has_default_body: false,
        },
        TraitMethodSig {
            name: "compare".to_string(),
            has_self: true,
            param_count: 1,
            return_type: Some(Ty::Con(TyCon::new("Ordering"))),
            has_default_body: true,  // default impl uses lt/eq
        },
    ],
});

// Each Ord impl also needs compare method
for (ty, ty_name) in &[(Ty::int(), "Int"), (Ty::float(), "Float"), (Ty::string(), "String")] {
    let mut methods = FxHashMap::default();
    methods.insert("lt".to_string(), ImplMethodSig { has_self: true, param_count: 1, return_type: Some(Ty::bool()) });
    methods.insert("compare".to_string(), ImplMethodSig {
        has_self: true,
        param_count: 1,
        return_type: Some(Ty::Con(TyCon::new("Ordering"))),
    });
    // register impl...
}
```

### Example 5: Generating Ord__compare__Type MIR Functions
```rust
// In snow-codegen/src/mir/lower.rs, add generate_compare_* methods
// The compare function returns Ordering (Less/Equal/Greater) instead of Bool

// For Int: if self < other then Less else if self == other then Equal else Greater end
fn generate_compare_struct(&mut self, name: &str, fields: &[(String, MirType)]) {
    let mangled = format!("Ord__compare__{}", name);
    let ordering_ty = MirType::SumType("Ordering".to_string());
    // Body: if self.lt(other) then Less
    //       else if self.eq(other) then Equal
    //       else Greater
    let body = MirExpr::If {
        cond: Box::new(/* call Ord__lt__Type(self, other) */),
        then_body: Box::new(MirExpr::ConstructVariant {
            type_name: "Ordering".to_string(),
            variant: "Less".to_string(),
            fields: vec![],
            ty: ordering_ty.clone(),
        }),
        else_body: Box::new(MirExpr::If {
            cond: Box::new(/* call Eq__eq__Type(self, other) */),
            then_body: Box::new(MirExpr::ConstructVariant {
                type_name: "Ordering".to_string(),
                variant: "Equal".to_string(),
                fields: vec![],
                ty: ordering_ty.clone(),
            }),
            else_body: Box::new(MirExpr::ConstructVariant {
                type_name: "Ordering".to_string(),
                variant: "Greater".to_string(),
                fields: vec![],
                ty: ordering_ty.clone(),
            }),
            ty: ordering_ty.clone(),
        }),
        ty: ordering_ty.clone(),
    };
    // Push function to lowerer.functions
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Tags from pattern order | Tags from type definition | This phase | Fixes wrong switch dispatch |
| Unit placeholder types for variant fields | Actual field types from sum type def | This phase | Fixes LLVM verifier errors |
| Ord returns Bool only | Ord has both `lt() -> Bool` and `compare() -> Ordering` | This phase | Enables Ordering pattern matching |
| No user-visible Ordering type | Ordering registered as built-in sum type | This phase | Users can `case compare(a,b) do Less -> ... end` |

**What already works:**
- Sum type definition parsing and type checking
- Pattern matching syntax (case/do/end) parsing
- Maranget decision tree compilation (algorithm is correct)
- Decision tree to LLVM IR translation (codegen logic is correct)
- Variant construction (`ConstructVariant`) with correct tag storage
- Access path navigation (`VariantField` GEP into variant overlay)
- Nullary variant pattern matching (e.g., `None -> ...`, `Red | Blue -> ...`)

## Open Questions

1. **Should `compile_match` accept a reference to sum_type_defs, or should the pattern compiler build its own lookup from the MirModule?**
   - What we know: `compile_match` is called from `codegen_match` in `expr.rs` where `self.sum_type_defs` is available, and also from `compile_patterns` in `compile.rs` which operates on a `MirModule`.
   - What's unclear: Whether to thread the deps through or restructure.
   - Recommendation: Pass `&FxHashMap<String, MirSumTypeDef>` as a parameter to `compile_match`. The codegen already has this map populated. For the `compile_patterns` walker (which isn't actually used in the codegen path -- `compile_match` is called inline from `codegen_match`), also pass it through.

2. **Should the `compare()` method be a standalone function or both a trait method and standalone?**
   - What we know: `lt` is a trait method (`Ord__lt__Type`) but comparison operators (`<`, `>`, etc.) are dispatched at MIR lowering time. The success criterion says `compare(a, b)` should work as a function call.
   - What's unclear: Whether `compare` should also be a standalone built-in function like `to_string`.
   - Recommendation: Register `compare` as both a trait method (for extensibility) and a built-in function in the type environment (for direct calls like `compare(a, b)`). Follow the pattern of `to_string` which is both `Display__to_string__Type` and a built-in.

3. **Nested constructor patterns like `Some(Some(x))` -- does the access path nesting work?**
   - What we know: The access path `VariantField(VariantField(Root, "Some", 0), "Some", 0)` would need the inner VariantField's parent type to resolve to the inner sum type. The `resolve_path_type` function handles this correctly -- it recursively resolves parent types and looks up variant field types from sum type defs.
   - What's unclear: Whether the pattern compiler's field type resolution (currently Unit placeholders) breaks the chain for nested patterns.
   - Recommendation: Fixing the field type resolution (replacing Unit placeholders with actual types) should make nested patterns work automatically. Add an explicit test for `Some(Some(x))`.

## Sources

### Primary (HIGH confidence)
- **Codebase analysis** (direct source code reading):
  - `crates/snow-codegen/src/pattern/compile.rs` -- Pattern matrix compiler, tag assignment bug at line 348
  - `crates/snow-codegen/src/pattern/mod.rs` -- AccessPath, ConstructorTag, DecisionTree definitions
  - `crates/snow-codegen/src/codegen/pattern.rs` -- Decision tree to LLVM codegen, access path navigation
  - `crates/snow-codegen/src/codegen/expr.rs` -- `codegen_match`, `codegen_construct_variant`
  - `crates/snow-codegen/src/codegen/types.rs` -- `create_sum_type_layout`, `variant_struct_type`
  - `crates/snow-codegen/src/mir/mod.rs` -- MirType, MirPattern, MirSumTypeDef
  - `crates/snow-codegen/src/mir/lower.rs` -- `lower_pattern`, `lower_sum_type_def`, `generate_ord_sum`
  - `crates/snow-typeck/src/builtins.rs` -- Ord trait definition, built-in trait registration
  - `crates/snow-typeck/src/infer.rs` -- `register_builtin_sum_types`, `SumTypeDefInfo`
- **v1.3 Milestone Audit** (`.planning/milestones/v1.3-MILESTONE-AUDIT.md`):
  - Documents LLVM Constructor pattern field binding limitation
  - Documents Ordering design decision (lt() -> Bool instead of Ordering)

### Secondary (MEDIUM confidence)
- **v1.3 ROADMAP** (`.planning/milestones/v1.3-ROADMAP.md`) -- Known limitations list

## Metadata

**Confidence breakdown:**
- Pattern matching bugs (tag mismatch, Unit placeholders): HIGH -- directly verified in source code
- Ordering type registration pattern: HIGH -- follows exact same pattern as Option/Result
- compare() method generation: HIGH -- follows exact same pattern as lt()/eq() generation
- Nested pattern handling: MEDIUM -- inferred from code structure, needs validation

**Research date:** 2026-02-08
**Valid until:** 2026-03-08 (stable codebase, no external dependencies changing)
