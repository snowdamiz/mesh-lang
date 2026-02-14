# Phase 75: Numeric Traits - Research

**Researched:** 2026-02-13
**Domain:** Compiler trait system extension -- user-extensible arithmetic operators via Add/Sub/Mul/Div/Neg with Output associated type
**Confidence:** HIGH

## Summary

Phase 75 extends the existing arithmetic operator infrastructure to support user-defined types and introduces the Output associated type for result type inference. The current codebase already has Add/Sub/Mul/Div/Mod registered as compiler-known traits in `builtins.rs` with impls for Int and Float. The type checker (`infer_binary` in `infer.rs`) already dispatches `+`, `-`, `*`, `/`, `%` through `infer_trait_binary_op`, and the MIR lowerer (`lower_binary_expr` in `lower.rs`) already converts binary ops on user types to trait method calls. However, there are three critical gaps that this phase must address.

**Gap 1: No Output associated type.** The current arithmetic trait definitions have `associated_types: vec![]`. The requirements (NUM-01, NUM-02) demand `type Output` so that `a + b` can infer a result type that differs from the operand type (e.g., `impl Add for Vec2 do type Output = Vec2 end`). Currently, `infer_trait_binary_op` returns the resolved LHS type as the result -- it does not look up an associated type binding. This must change to query `TraitRegistry::resolve_associated_type("Add", "Output", &lhs_ty)` for the result type.

**Gap 2: MIR lowerer has bugs for user-type arithmetic dispatch.** The user-type dispatch in `lower_binary_expr` (lines 5083-5135) has two problems: (a) It hardcodes `MirType::Bool` as the return type for ALL trait calls, including Add/Sub/Mul -- correct for Eq/Ord but wrong for arithmetic. The return type must be the resolved Output type from the types map. (b) `BinOp::Div` and `BinOp::Mod` are missing from the dispatch table, meaning `/` and `%` on user types fall through to the hardware BinOp path (which would crash or produce garbage for structs).

**Gap 3: No Neg trait or unary dispatch.** Unary minus (`-value`) currently returns `operand_ty` in the type checker without any trait check, and the MIR lowerer emits `UnaryOp::Neg` which only handles Int/Float in codegen. User-defined Neg requires: (a) registering a Neg trait in builtins, (b) checking Neg impl in `infer_unary`, (c) emitting a trait method call in the MIR lowerer for non-primitive types.

**Primary recommendation:** Add `type Output` to existing arithmetic traits (preserving `Output = Int` for Int and `Output = Float` for Float), update `infer_trait_binary_op` to resolve Output via associated type lookup, fix the MIR lowerer's user-type dispatch to use the correct return type and include Div/Mod, register Neg trait, and wire unary minus through trait dispatch.

## Standard Stack

### Core
| Component | Location | Purpose | Why Standard |
|-----------|----------|---------|--------------|
| mesh-typeck/infer.rs | `infer_trait_binary_op` (line 3508), `infer_unary` (line 3538) | Type inference for binary and unary operators | Existing HM inference; needs Output resolution |
| mesh-typeck/builtins.rs | `register_compiler_known_traits` (line 821) | Registers Add/Sub/Mul/Div/Mod/Eq/Ord traits | Existing registration; needs associated type addition |
| mesh-typeck/traits.rs | `TraitRegistry`, `resolve_associated_type` (line 356) | Trait impl lookup and associated type resolution | Added in Phase 74; ready for use |
| mesh-codegen/mir/lower.rs | `lower_binary_expr` (line 5051), `lower_unary_expr` (line 5219) | MIR lowering for operators | Existing; needs dispatch fixes and Neg support |
| mesh-codegen/codegen/expr.rs | `codegen_unaryop` (line 546) | LLVM codegen for unary ops | Existing for primitives; needs Call dispatch for user types |

### Supporting
No new external dependencies. All changes are internal to existing crates.

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| `type Output` associated type | Keep implicit `Output = Self` | Would prevent `Matrix * Vector = Vector` patterns; requirements explicitly mandate Output |
| Separate Neg trait | Reuse existing `Not` trait logic | Neg and Not have different semantics (arithmetic vs boolean); separate trait is cleaner |
| Mixed-type RHS parameter | `Add<RHS>` generic trait | Explicitly out of scope per REQUIREMENTS.md; same-type constraint (`a + b` requires same type) avoids complexity |

## Architecture Patterns

### Current Binary Operator Pipeline (Before Phase 75)

```
Source: v1 + v2
  -> Parser: BinaryExpr(lhs, PLUS, rhs)
    -> Type Checker (infer_binary):
       1. Infer lhs_ty, rhs_ty
       2. Match PLUS -> infer_trait_binary_op(ctx, "Add", lhs_ty, rhs_ty, ...)
       3. Unify lhs_ty with rhs_ty (same-type constraint)
       4. Check has_impl("Add", &resolved)
       5. Return resolved (=== lhs_ty) *** BUG: should be Output ***
    -> MIR Lowerer (lower_binary_expr):
       IF is_user_type(lhs):
         Dispatch to Call(Add__add__TypeName, [lhs, rhs], MirType::Bool) *** BUG: Bool ***
       ELSE:
         Emit MirExpr::BinOp { op: Add, lhs, rhs, ty }
    -> Codegen (codegen_binop):
       For Int: build_int_add
       For Float: build_float_add
       For Call: codegen_call (correct function dispatch)
```

### Target Binary Operator Pipeline (After Phase 75)

```
Source: v1 + v2
  -> Parser: BinaryExpr(lhs, PLUS, rhs) [no change]
    -> Type Checker (infer_trait_binary_op):
       1. Infer lhs_ty, rhs_ty
       2. Match PLUS -> infer_trait_binary_op(ctx, "Add", lhs_ty, rhs_ty, ...)
       3. Unify lhs_ty with rhs_ty (same-type constraint)
       4. Check has_impl("Add", &resolved)
       5. Resolve Output: trait_registry.resolve_associated_type("Add", "Output", &resolved)
       6. Return Output type (or fall back to resolved if Output lookup fails)
    -> MIR Lowerer (lower_binary_expr):
       IF is_user_type(lhs) AND has_trait_impl:
         Dispatch to Call(Add__add__TypeName, [lhs, rhs], ty_from_types_map)
         *** ty comes from resolve_range(), which gets the Output type from typeck ***
       ELSE:
         Emit MirExpr::BinOp { op: Add, lhs, rhs, ty } [for primitives]
    -> Codegen: [no change needed -- Call dispatches correctly]
```

### Target Unary Operator Pipeline (After Phase 75)

```
Source: -value
  -> Parser: UnaryExpr(MINUS, operand) [no change]
    -> Type Checker (infer_unary):
       1. Infer operand_ty
       2. IF is_type_var: return operand_ty (deferred)
       3. Check has_impl("Neg", &resolved)
       4. Resolve Output: trait_registry.resolve_associated_type("Neg", "Output", &resolved)
       5. Return Output type (or resolved if no Output)
    -> MIR Lowerer (lower_unary_expr):
       IF is_user_type(operand):
         Dispatch to Call(Neg__neg__TypeName, [operand], ty_from_types_map)
       ELSE:
         Emit MirExpr::UnaryOp { op: Neg, operand, ty } [for Int/Float]
    -> Codegen: [no change for primitives; Call path handles user types]
```

### Pattern 1: Adding Output to Existing Arithmetic Traits

**What:** Extend the 5 existing arithmetic trait definitions in `builtins.rs` to include `type Output` as an associated type, and set `Output = Int` for Int impls and `Output = Float` for Float impls.

**When to use:** When modifying `register_compiler_known_traits`.

**Example:**
```rust
// Source: crates/mesh-typeck/src/builtins.rs (to be modified)
// BEFORE:
registry.register_trait(TraitDef {
    name: trait_name.to_string(),
    methods: vec![TraitMethodSig { ... }],
    associated_types: vec![],  // <-- empty
});
// associated_types: FxHashMap::default(),  // <-- empty on ImplDef

// AFTER:
registry.register_trait(TraitDef {
    name: trait_name.to_string(),
    methods: vec![TraitMethodSig { ... }],
    associated_types: vec![AssocTypeDef { name: "Output".to_string() }],  // <-- NEW
});
// On ImplDef:
let mut assoc_types = FxHashMap::default();
assoc_types.insert("Output".to_string(), ty.clone());  // Output = Int for Int, etc.
// associated_types: assoc_types,
```

### Pattern 2: Output Resolution in Type Inference

**What:** Modify `infer_trait_binary_op` to look up the Output associated type instead of returning the operand type.

**When to use:** Every arithmetic binary operation inference.

**Example:**
```rust
// Source: crates/mesh-typeck/src/infer.rs (to be modified)
fn infer_trait_binary_op(
    ctx: &mut InferCtx,
    trait_name: &str,
    lhs_ty: &Ty,
    rhs_ty: &Ty,
    trait_registry: &TraitRegistry,
    origin: &ConstraintOrigin,
) -> Result<Ty, TypeError> {
    ctx.unify(lhs_ty.clone(), rhs_ty.clone(), origin.clone())?;
    let resolved = ctx.resolve(lhs_ty.clone());

    if is_type_var(&resolved) {
        return Ok(resolved);
    }

    if trait_registry.has_impl(trait_name, &resolved) {
        // NEW: Look up the Output associated type
        if let Some(output_ty) = trait_registry.resolve_associated_type(
            trait_name, "Output", &resolved
        ) {
            Ok(output_ty)
        } else {
            // Fallback: if no Output associated type, use operand type
            // (backward compat for traits without Output)
            Ok(resolved)
        }
    } else {
        let err = TypeError::TraitNotSatisfied { ... };
        ctx.errors.push(err.clone());
        Err(err)
    }
}
```

### Pattern 3: Neg Trait Registration

**What:** Register a Neg trait in builtins following the same pattern as Add/Sub/Mul/Div/Mod.

**Example:**
```rust
// Source: crates/mesh-typeck/src/builtins.rs (to be added)
registry.register_trait(TraitDef {
    name: "Neg".to_string(),
    methods: vec![TraitMethodSig {
        name: "neg".to_string(),
        has_self: true,
        param_count: 0,  // unary: no additional params beyond self
        return_type: None,
        has_default_body: false,
    }],
    associated_types: vec![AssocTypeDef { name: "Output".to_string() }],
});

// Register impls for Int and Float
for (ty, ty_name) in &[(Ty::int(), "Int"), (Ty::float(), "Float")] {
    let mut methods = FxHashMap::default();
    methods.insert("neg".to_string(), ImplMethodSig {
        has_self: true,
        param_count: 0,
        return_type: Some(ty.clone()),
    });
    let mut assoc_types = FxHashMap::default();
    assoc_types.insert("Output".to_string(), ty.clone());
    let _ = registry.register_impl(ImplDef {
        trait_name: "Neg".to_string(),
        impl_type: ty.clone(),
        impl_type_name: ty_name.to_string(),
        methods,
        associated_types: assoc_types,
    });
}
```

### Pattern 4: MIR Lowerer Fix for Arithmetic Dispatch

**What:** Fix the user-type dispatch in `lower_binary_expr` to: (a) use the types-map result type instead of `MirType::Bool`, (b) include Div and Mod in the dispatch table.

**Example:**
```rust
// Source: crates/mesh-codegen/src/mir/lower.rs (to be modified)
// Current dispatch table (lines 5085-5095) extended:
let dispatch = match op {
    BinOp::Add => Some(("Add", "add", false, false)),
    BinOp::Sub => Some(("Sub", "sub", false, false)),
    BinOp::Mul => Some(("Mul", "mul", false, false)),
    BinOp::Div => Some(("Div", "div", false, false)),   // NEW
    BinOp::Mod => Some(("Mod", "mod", false, false)),   // NEW
    BinOp::Eq  => Some(("Eq", "eq", false, false)),
    BinOp::NotEq => Some(("Eq", "eq", true, false)),
    BinOp::Lt  => Some(("Ord", "lt", false, false)),
    BinOp::Gt  => Some(("Ord", "lt", false, true)),
    BinOp::LtEq => Some(("Ord", "lt", true, true)),
    BinOp::GtEq => Some(("Ord", "lt", true, false)),
    _ => None,
};

// Fix the return type: use `ty` from resolve_range instead of MirType::Bool
if has_impl {
    // Determine result type: for comparison ops, always Bool; for arithmetic, use ty
    let result_ty = match op {
        BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::Gt
        | BinOp::LtEq | BinOp::GtEq => MirType::Bool,
        _ => ty.clone(),  // ty comes from resolve_range() = typeck Output type
    };
    let fn_ty = MirType::FnPtr(
        vec![lhs_ty.clone(), rhs_ty],
        Box::new(result_ty.clone()),
    );
    let call = MirExpr::Call {
        func: Box::new(MirExpr::Var(mangled, fn_ty)),
        args: vec![call_lhs, call_rhs],
        ty: result_ty,
    };
    // ...
}
```

### Pattern 5: MIR Lowerer Neg Dispatch for User Types

**What:** In `lower_unary_expr`, check if the operand is a user type and emit a trait call instead of `UnaryOp::Neg`.

**Example:**
```rust
// Source: crates/mesh-codegen/src/mir/lower.rs (to be modified)
fn lower_unary_expr(&mut self, un: &UnaryExpr) -> MirExpr {
    let operand = un.operand().map(|e| self.lower_expr(&e)).unwrap_or(MirExpr::Unit);
    let op = un.op().map(|t| match t.kind() {
        SyntaxKind::MINUS => UnaryOp::Neg,
        SyntaxKind::BANG | SyntaxKind::NOT_KW => UnaryOp::Not,
        _ => UnaryOp::Neg,
    }).unwrap_or(UnaryOp::Neg);

    let ty = self.resolve_range(un.syntax().text_range());

    // NEW: For user types with Neg, dispatch to trait method
    if op == UnaryOp::Neg {
        let operand_ty = operand.ty().clone();
        let is_user_type = matches!(operand_ty, MirType::Struct(_) | MirType::SumType(_));
        if is_user_type {
            let ty_for_lookup = mir_type_to_ty(&operand_ty);
            let type_name = mir_type_to_impl_name(&operand_ty);
            let mangled = format!("Neg__neg__{}", type_name);
            let has_impl = self.trait_registry.has_impl("Neg", &ty_for_lookup)
                || self.known_functions.contains_key(&mangled);
            if has_impl {
                let fn_ty = MirType::FnPtr(
                    vec![operand_ty],
                    Box::new(ty.clone()),
                );
                return MirExpr::Call {
                    func: Box::new(MirExpr::Var(mangled, fn_ty)),
                    args: vec![operand],
                    ty,
                };
            }
        }
    }

    MirExpr::UnaryOp { op, operand: Box::new(operand), ty }
}
```

### Anti-Patterns to Avoid

- **Hardcoding MirType::Bool for all trait dispatch return types:** The current code does this. Arithmetic trait calls return the Output type, not Bool. Only comparison trait calls (Eq/Ord) return Bool.
- **Skipping Div/Mod in user-type dispatch:** These must be included alongside Add/Sub/Mul. Omitting them causes `/` and `%` on user types to fall through to the hardware BinOp path, which only handles Int/Float.
- **Adding mixed-type arithmetic (Int + Float -> Float):** Explicitly out of scope. Keep the same-type constraint in `infer_trait_binary_op` (unify lhs with rhs).
- **Modifying the unifier for Output types:** Output resolution happens at the trait registry level, not in the unifier. The `InferCtx` and `unify.rs` remain unchanged.
- **Breaking primitive arithmetic performance:** Int + Int and Float + Float must still lower to hardware `BinOp` instructions, not trait method calls. The MIR lowerer's `is_user_type` check ensures this.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Associated type resolution | Custom lookup logic | `TraitRegistry::resolve_associated_type` | Already implemented in Phase 74; proven correct |
| Trait impl registration | New registration mechanism | `register_compiler_known_traits` pattern | Existing pattern handles method validation, duplicate checks |
| Type-to-MIR-type resolution | Custom type mapping | `resolve_range` / `resolve_type` | Already used everywhere in the lowerer; reads from typeck types map |
| Name mangling | Custom mangling logic | `mir_type_to_impl_name` + format string | Existing pattern: `Trait__method__TypeName` |

**Key insight:** Phase 75 is almost entirely about wiring existing infrastructure together correctly. The associated type system (Phase 74), the trait registry, the MIR lowerer's user-type dispatch, and the codegen are all in place. The work is: (1) adding `type Output` to trait defs, (2) updating `infer_trait_binary_op` to use Output, (3) fixing bugs in MIR user-type dispatch, (4) adding Neg trait + unary dispatch.

## Common Pitfalls

### Pitfall 1: MIR Return Type Hardcoded to Bool
**What goes wrong:** The existing user-type dispatch in `lower_binary_expr` (line 5111) hardcodes `MirType::Bool` as the return type for ALL trait-dispatched calls, including Add/Sub/Mul. This means `v1 + v2` on a user type would be typed as Bool in MIR, causing type mismatches downstream.
**Why it happens:** The dispatch was originally written for Eq/Ord only (which return Bool). When Add/Sub/Mul were added to the dispatch table, the return type was not updated.
**How to avoid:** Use `ty` from `resolve_range(bin.syntax().text_range())` for arithmetic dispatches. This gets the Output type from the type checker's types map. Keep `MirType::Bool` only for Eq/Ord dispatches.
**Warning signs:** User-type arithmetic compiles but the result is typed as Bool; assignment to a non-Bool variable causes type error.

### Pitfall 2: Missing Div/Mod in User-Type Dispatch
**What goes wrong:** `BinOp::Div` and `BinOp::Mod` are not in the user-type dispatch table (lines 5085-5095). User types implementing Div or Mod will fall through to the hardware BinOp path, which only handles Int/Float in LLVM codegen.
**Why it happens:** The original dispatch table only included the most common operators; Div/Mod were overlooked.
**How to avoid:** Add `BinOp::Div => Some(("Div", "div", false, false))` and `BinOp::Mod => Some(("Mod", "mod", false, false))` to the dispatch table.
**Warning signs:** `v1 / v2` or `v1 % v2` on user types produces LLVM codegen errors or incorrect results.

### Pitfall 3: Primitive Arithmetic Regression
**What goes wrong:** After adding Output associated type to arithmetic traits, `Int + Int` might try to resolve Output through the trait registry instead of using the fast path. If the lookup fails or returns the wrong type, primitive arithmetic breaks.
**Why it happens:** The type checker currently returns `Ok(resolved)` (the LHS type) for primitives. Changing this to use Output lookup could return `None` if the lookup is incorrect.
**How to avoid:** For built-in impls (Int, Float), explicitly set `Output = Int` / `Output = Float` in the ImplDef's `associated_types` map. The fallback `Ok(resolved)` should remain as a safety net. Test that `1 + 2` still works and infers as Int.
**Warning signs:** Basic arithmetic expressions fail type checking or produce wrong types.

### Pitfall 4: Operator Chaining with Output Type
**What goes wrong:** `a + b + c` fails because `a + b` returns the Output type, and then `Output + c` requires an Add impl for the Output type. If the user defines `impl Add for Vec2 do type Output = Vec2`, then `v1 + v2 + v3` needs Add for Vec2 -- which is the same impl, so it works. But if Output differs from the operand type (e.g., `impl Add for Meter do type Output = Length`), then `m1 + m2 + m3` would require Add for Length, not Meter.
**Why it happens:** The second `+` operates on Output, not on the original type.
**How to avoid:** For v7.0, document that chaining works naturally when Output = Self (the common case). The type system correctly infers the chain because each step feeds its Output into the next operation's type check. No special handling needed.
**Warning signs:** Chain `a + b + c` fails with "Add not implemented for [Output type]".

### Pitfall 5: Neg Trait Dispatch Not Reaching Codegen
**What goes wrong:** User writes `impl Neg for Vec2` and uses `-v`, but the MIR lowerer still emits `UnaryOp::Neg` instead of a trait call. The codegen then tries `build_int_neg` on a struct pointer and crashes.
**Why it happens:** `lower_unary_expr` currently does not check for user types -- it unconditionally emits `UnaryOp::Neg`.
**How to avoid:** Add the same user-type detection pattern as `lower_binary_expr`: check `is_user_type` on the operand, look up `has_impl("Neg", ...)`, and emit `MirExpr::Call` to `Neg__neg__TypeName`.
**Warning signs:** `-my_vec` compiles past type checking but crashes during LLVM codegen.

### Pitfall 6: Breaking Existing Tests
**What goes wrong:** Adding Output to arithmetic traits might break existing E2E tests that rely on the current return type behavior.
**Why it happens:** The change from `return Ok(resolved)` to `return Ok(output_ty)` affects every arithmetic expression's inferred type. If Output resolution returns the wrong type for primitives, all arithmetic breaks.
**How to avoid:** The Output for primitive types (Int, Float) must be the same type (Int, Float). Since Phase 74's associated type infrastructure is already proven, this should work correctly. Run the full test suite after every change.
**Warning signs:** Existing arithmetic tests fail after adding Output to trait defs.

## Code Examples

### User-Facing Syntax: Custom Type Arithmetic

```mesh
struct Vec2 do
  x :: Float
  y :: Float
end

impl Add for Vec2 do
  type Output = Vec2
  fn add(self, other) -> Vec2 do
    Vec2 { x: self.x + other.x, y: self.y + other.y }
  end
end

impl Neg for Vec2 do
  type Output = Vec2
  fn neg(self) -> Vec2 do
    Vec2 { x: 0.0 - self.x, y: 0.0 - self.y }
  end
end

fn main() do
  let v1 = Vec2 { x: 1.0, y: 2.0 }
  let v2 = Vec2 { x: 3.0, y: 4.0 }
  let v3 = v1 + v2       # Vec2 { x: 4.0, y: 6.0 }
  let v4 = -v1           # Vec2 { x: -1.0, y: -2.0 }
  println(v3.x.to_string())  # "4.0"
  println(v4.y.to_string())  # "-2.0"
end
```

### User-Facing Syntax: Different Output Type

```mesh
struct Meter do
  value :: Float
end

struct SquareMeter do
  value :: Float
end

impl Mul for Meter do
  type Output = SquareMeter
  fn mul(self, other) -> SquareMeter do
    SquareMeter { value: self.value * other.value }
  end
end

fn main() do
  let width = Meter { value: 3.0 }
  let height = Meter { value: 4.0 }
  let area = width * height  # SquareMeter, not Meter!
  println(area.value.to_string())  # "12.0"
end
```

### Verifying Backward Compatibility

```mesh
fn main() do
  # All existing arithmetic must still work
  let a = 1 + 2      # Int
  let b = 3.0 * 4.0  # Float
  let c = 10 / 3     # Int
  let d = 10 % 3     # Int
  let e = -42         # Int (unary neg)
  let f = -3.14       # Float (unary neg)
  println(a.to_string())
  println(b.to_string())
  println(c.to_string())
  println(d.to_string())
  println(e.to_string())
  println(f.to_string())
end
```

## Requirement Mapping

| Requirement | What It Needs | Implementation Approach |
|-------------|---------------|------------------------|
| NUM-01: User impl Add/Sub/Mul/Div with `type Output` | Add Output assoc type to trait defs, validate in user impls | Extend `register_compiler_known_traits` with `AssocTypeDef { name: "Output" }`; user `impl Add for Vec2 do type Output = Vec2 ... end` validated by existing Phase 74 infrastructure |
| NUM-02: Binary operators use Output for result type | `infer_trait_binary_op` returns Output instead of LHS type | Query `resolve_associated_type("Add", "Output", &resolved)` in `infer_trait_binary_op`; fix MIR dispatch return type |
| NUM-03: User impl Neg for unary minus | Neg trait, unary dispatch in typeck + MIR | Register Neg trait in builtins, add trait check to `infer_unary`, add user-type dispatch to `lower_unary_expr` |

## File Touch Points

Complete list of files that need modification in Phase 75:

### mesh-typeck (Type System)
1. **`builtins.rs`** -- Add `AssocTypeDef { name: "Output" }` to arithmetic trait defs (Add/Sub/Mul/Div/Mod); add `Output` binding to Int/Float ImplDefs; register new Neg trait with Output + impls for Int/Float
2. **`infer.rs`** -- Modify `infer_trait_binary_op` (line 3508) to resolve Output associated type for result; modify `infer_unary` (line 3538) to check Neg trait impl and resolve Output

### mesh-codegen (Code Generation)
3. **`mir/lower.rs`** -- Fix `lower_binary_expr` user-type dispatch (line 5083): add Div/Mod to table, fix return type from MirType::Bool to types-map type; add user-type dispatch to `lower_unary_expr` (line 5219) for Neg trait calls

### Test Files
4. **`tests/e2e/numeric_traits.mpl`** -- E2E test for user-defined Add/Sub/Mul/Div on custom struct
5. **`tests/e2e/numeric_neg.mpl`** -- E2E test for user-defined Neg on custom struct
6. **`crates/meshc/tests/e2e.rs`** -- New test functions for numeric trait E2E tests + compile-fail tests

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| No associated types on arithmetic traits | Adding `type Output` | Phase 75 (now) | Result type inference uses Output, not LHS type |
| No Neg trait | Adding Neg trait | Phase 75 (now) | Unary minus on user types dispatches through trait |
| MIR hardcodes Bool for all trait dispatch | Arithmetic uses Output type, comparison uses Bool | Phase 75 (now) | User-type arithmetic produces correct result types |
| Div/Mod missing from user-type dispatch | All 5 arithmetic ops dispatched | Phase 75 (now) | User types can implement all arithmetic operators |

## Open Questions

1. **Should Mod also get `type Output`?**
   - What we know: The requirements mention Add/Sub/Mul/Div explicitly but not Mod. However, Mod is already registered as a compiler-known trait alongside the others.
   - What's unclear: Whether users will want `impl Mod for CustomType`. The Mod operator (%) is less common in custom type arithmetic.
   - Recommendation: Add Output to Mod for consistency. The incremental cost is zero (it follows the exact same pattern), and it prevents a confusing gap where 4 out of 5 arithmetic traits have Output but one does not.

2. **Should `infer_unary` check for Neg trait on ALL types or only user types?**
   - What we know: Currently, unary minus on Int/Float returns `operand_ty` without any trait check. Adding a trait check would be correct but adds overhead for every negation.
   - What's unclear: Whether existing code negates types that DON'T have a Neg impl (which would now error).
   - Recommendation: Check Neg trait for non-type-variable types that are NOT Int/Float. For Int and Float, keep the current fast path (return operand_ty directly). This preserves backward compatibility while enabling user-type Neg.

3. **Interaction with future Iterator `sum()` terminal operation**
   - What we know: Phase 78 will implement `Iter.sum(iter)` which needs Add + a zero/identity value. The Add trait with Output enables this.
   - What's unclear: Whether the Add trait's Output must equal Self for `sum()` to work (it needs to accumulate).
   - Recommendation: Not a Phase 75 concern. `sum()` can require `where T: Add` and the Output constraint will be checked at the call site in Phase 78.

## Sources

### Primary (HIGH confidence)
- `crates/mesh-typeck/src/infer.rs` lines 3396-3569 -- `infer_binary`, `infer_trait_binary_op`, `infer_unary` (verified: returns LHS type, no Output resolution, no Neg check)
- `crates/mesh-typeck/src/builtins.rs` lines 821-857 -- `register_compiler_known_traits` (verified: `associated_types: vec![]` on all arithmetic traits)
- `crates/mesh-typeck/src/traits.rs` lines 352-364 -- `resolve_associated_type` (verified: exists and ready from Phase 74)
- `crates/mesh-codegen/src/mir/lower.rs` lines 5051-5241 -- `lower_binary_expr`, `lower_unary_expr` (verified: Div/Mod missing from user dispatch, Bool hardcoded, no Neg dispatch)
- `crates/mesh-codegen/src/codegen/expr.rs` lines 546-584 -- `codegen_unaryop` (verified: handles only Int/Float, errors on other types)
- `.planning/research/PITFALLS.md` Pitfalls 4 and 11 -- Numeric trait output type and Neg unary dispatch
- `.planning/research/ARCHITECTURE.md` Section 4 -- Numeric traits architecture
- `.planning/research/FEATURES.md` Section 4 -- Numeric traits feature analysis
- `.planning/research/STACK.md` Section 7 -- Numeric traits stack decisions
- `.planning/REQUIREMENTS.md` NUM-01, NUM-02, NUM-03 -- Phase 75 requirements
- `.planning/ROADMAP.md` Phase 75 entry -- Success criteria

### Secondary (MEDIUM confidence)
- `.planning/phases/74-associated-types/74-VERIFICATION.md` -- Phase 74 fully verified (6/6 truths, 5/5 requirements); associated type infrastructure confirmed working

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all changes to existing crates verified against source code with exact line numbers
- Architecture: HIGH -- all integration points (infer_binary, lower_binary_expr, builtins, etc.) identified and verified; pipeline fully traced from parser to codegen
- Pitfalls: HIGH -- 6 pitfalls identified from codebase analysis; 2 confirmed by cross-referencing with domain PITFALLS.md (Pitfalls 4 and 11); MIR Bool bug independently discovered
- Code examples: HIGH -- patterns derived directly from existing codebase conventions and Phase 74 patterns

**Research date:** 2026-02-13
**Valid until:** 2026-03-13 (stable -- compiler internals don't change externally)
