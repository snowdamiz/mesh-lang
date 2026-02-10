# Phase 43: Math Stdlib - Research

**Researched:** 2026-02-09
**Domain:** Compiler stdlib expansion -- adding Math/Int/Float modules with numeric functions and type conversions
**Confidence:** HIGH

## Summary

Phase 43 adds math operations to Snow through three new stdlib modules: `Math` (general numeric functions), `Int` (integer-specific operations), and `Float` (float-specific operations). The implementation follows the exact same four-layer pattern used by existing stdlib modules (String, List, Map, etc.): type signatures in the type checker's `stdlib_modules()` registry, MIR lowering via `lower_field_access()` + `map_builtin_name()`, LLVM intrinsic declarations or inline codegen, and e2e tests.

The critical architectural insight is that **most Math functions can be implemented as LLVM intrinsics rather than runtime function calls**. LLVM provides `llvm.fabs`, `llvm.sqrt`, `llvm.pow`, `llvm.floor`, `llvm.ceil`, `llvm.round`, `llvm.abs`, `llvm.smin`, `llvm.smax`, `llvm.minnum`, and `llvm.maxnum` intrinsics that compile to single machine instructions on most architectures. This means **zero new Rust crate dependencies** and **zero new snow-rt functions** for the core math operations. Int/Float conversion uses LLVM's built-in `sitofp`/`fptosi` instructions. Math.pi is a compile-time constant.

The only deviation from the existing runtime-call pattern is that instead of declaring extern "C" functions in `intrinsics.rs` and implementing them in `snow-rt`, we emit LLVM intrinsic calls directly in `codegen/expr.rs`. This is a cleaner approach that gives better performance (single instructions vs function calls) and follows the "zero new dependencies" constraint.

**Primary recommendation:** Implement all Math functions as LLVM intrinsics emitted directly in codegen, with type-overloaded dispatch for Int/Float variants. Add "Math", "Int", "Float" as new stdlib modules following the exact four-layer pattern of existing modules.

## Standard Stack

### Core
| Component | Version | Purpose | Why Standard |
|-----------|---------|---------|--------------|
| inkwell | 0.8.0 | LLVM bindings for Rust | Already in use; provides `Intrinsic::find()` API for LLVM intrinsics |
| LLVM 21 | 21.1 | Backend code generation | Already in use; provides all needed math intrinsics natively |

### LLVM Intrinsics Used
| Intrinsic | Purpose | Signature |
|-----------|---------|-----------|
| `llvm.fabs` | Float absolute value | `f64 -> f64` |
| `llvm.abs` | Integer absolute value | `i64, i1 -> i64` (i1 = is_int_min_poison) |
| `llvm.sqrt` | Square root | `f64 -> f64` |
| `llvm.pow` | Float exponentiation | `f64, f64 -> f64` |
| `llvm.powi` | Float to integer power | `f64, i32 -> f64` |
| `llvm.floor` | Floor (round down) | `f64 -> f64` |
| `llvm.ceil` | Ceiling (round up) | `f64 -> f64` |
| `llvm.round` | Round to nearest | `f64 -> f64` |
| `llvm.minnum` | Float minimum | `f64, f64 -> f64` |
| `llvm.maxnum` | Float maximum | `f64, f64 -> f64` |
| `llvm.smin` | Signed integer minimum | `i64, i64 -> i64` |
| `llvm.smax` | Signed integer maximum | `i64, i64 -> i64` |

### LLVM Instructions Used
| Instruction | Purpose |
|-------------|---------|
| `sitofp i64 %val to double` | Int.to_float: signed int to floating point |
| `fptosi double %val to i64` | Float.to_int: floating point to signed int (truncation) |

### No New Dependencies
This phase requires zero new Rust crate dependencies. All math operations are LLVM intrinsics or instructions.

## Architecture Patterns

### Four-Layer Stdlib Module Pattern

Every stdlib module in Snow follows this exact four-layer architecture:

```
Layer 1: Type Checker (snow-typeck/src/infer.rs)
  - stdlib_modules() HashMap: module name -> function name -> type Scheme
  - STDLIB_MODULE_NAMES array: list of recognized module names

Layer 2: MIR Lowering (snow-codegen/src/mir/lower.rs)
  - STDLIB_MODULES array: list of recognized module names
  - lower_field_access(): converts Module.func -> module_func prefix
  - map_builtin_name(): converts module_func -> snow_module_func runtime name

Layer 3: LLVM Codegen (snow-codegen/src/codegen/)
  - intrinsics.rs: declares extern "C" runtime functions (for runtime-backed functions)
  - expr.rs: emits LLVM IR for function calls (for intrinsic-backed functions)

Layer 4: Runtime (snow-rt/src/) [NOT NEEDED for this phase]
  - extern "C" function implementations
```

### Math Functions: Special Codegen Path

Unlike existing stdlib functions that route through runtime calls (`snow_string_length`, etc.), Math functions should be intercepted in `codegen_call()` and emitted as LLVM intrinsics directly. The pattern:

1. In `codegen_call()`, check if the function name matches a math intrinsic pattern
2. Use `Intrinsic::find("llvm.sqrt")` etc. to get the LLVM intrinsic
3. Call `intrinsic.get_declaration(&module, &[f64_type.into()])` for the overloaded declaration
4. Emit `builder.build_call(intrinsic_fn, &[arg], "result")`

### Type-Overloaded Functions (abs, min, max)

Math.abs, Math.min, and Math.max must work for both Int and Float. The approach:

1. **Type checker**: Register overloaded signatures using a type variable, OR register two separate entries. The simpler approach: register with separate signatures for Int and Float in the module, and let typeck resolve based on argument type.
2. **Actually simplest**: Register with a type variable (like `compare()` uses TyVar(99002)), so the function is polymorphic. Then at codegen, inspect the resolved argument type to decide which LLVM intrinsic to use.

The recommended approach is polymorphic registration in the type checker (like `compare`), with runtime type dispatch in codegen:

```
Math.abs(x) where x: Int  -> llvm.abs.i64(x, false)
Math.abs(x) where x: Float -> llvm.fabs.f64(x)
Math.min(a, b) where a: Int -> llvm.smin.i64(a, b)
Math.min(a, b) where a: Float -> llvm.minnum.f64(a, b)
```

### Module Registration Pattern

Adding a new module follows this exact pattern (derived from existing modules like String, List):

**In `infer.rs` `stdlib_modules()`:**
```rust
let mut math_mod = HashMap::new();
math_mod.insert("abs".to_string(), Scheme { vars: vec![t_var], ty: Ty::fun(vec![t.clone()], t.clone()) });
// ... more functions
modules.insert("Math".to_string(), math_mod);
```

**In `infer.rs` `STDLIB_MODULE_NAMES`:**
```rust
const STDLIB_MODULE_NAMES: &[&str] = &[
    "String", "IO", "Env", "File", "List", "Map", "Set", "Tuple",
    "Range", "Queue", "HTTP", "JSON", "Request", "Job",
    "Math", "Int", "Float",  // NEW
];
```

**In `lower.rs` `STDLIB_MODULES`:**
```rust
const STDLIB_MODULES: &[&str] = &[
    "String", "IO", "Env", "File", "List", "Map", "Set", "Tuple",
    "Range", "Queue", "HTTP", "JSON", "Request", "Job",
    "Math", "Int", "Float",  // NEW
];
```

**In `lower.rs` `map_builtin_name()`:**
```rust
"math_abs" => "snow_math_abs".to_string(),
"math_min" => "snow_math_min".to_string(),
// ... etc
"int_to_float" => "snow_int_to_float".to_string(),
"float_to_int" => "snow_float_to_int".to_string(),
```

### Math.pi: Constant Access Pattern

Math.pi is a constant, not a function. In the existing architecture, `Module.name` where `name` is not followed by `()` resolves via `lower_field_access` as a `MirExpr::Var`. The special case: when the MIR sees `math_pi`, codegen should emit a float constant `3.141592653589793` directly, not a function call.

**Type checker registration:**
```rust
math_mod.insert("pi".to_string(), Scheme::mono(Ty::float()));
```

**Codegen handling:** In `codegen_var()` or `codegen_call()`, when encountering `snow_math_pi`, emit:
```rust
context.f64_type().const_float(std::f64::consts::PI)
```

### Recommended File Structure

No new files needed. Changes to existing files:

```
crates/snow-typeck/src/infer.rs          # Add Math/Int/Float modules to stdlib_modules()
crates/snow-codegen/src/mir/lower.rs     # Add to STDLIB_MODULES + map_builtin_name()
crates/snow-codegen/src/codegen/expr.rs  # Add LLVM intrinsic dispatch in codegen_call()
crates/snowc/tests/e2e_stdlib.rs         # Add e2e tests
tests/e2e/                               # Add test fixture files
```

### Anti-Patterns to Avoid
- **Do NOT add runtime functions in snow-rt for math**: LLVM intrinsics are faster (single instruction) and require zero new code in the runtime crate.
- **Do NOT use separate function names for Int/Float variants**: `Math.abs_int()` / `Math.abs_float()` is ugly. Use polymorphic type resolution with codegen-time dispatch.
- **Do NOT add Math.pi as a function call**: `Math.pi` should not require `()`. It is accessed as a field, not called.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Float abs | Runtime function `snow_math_fabs()` | LLVM intrinsic `llvm.fabs` | Single instruction on all architectures |
| Int abs | Runtime function with conditional | LLVM intrinsic `llvm.abs` | Handles INT_MIN correctly, single instruction |
| Square root | C libm `sqrt()` FFI | LLVM intrinsic `llvm.sqrt` | LLVM lowers to `sqrtsd` on x86, `fsqrt` on ARM |
| Float pow | C libm `pow()` FFI | LLVM intrinsic `llvm.pow` | LLVM can optimize constant exponents |
| Floor/Ceil/Round | Runtime function calling C libm | LLVM intrinsics `llvm.floor`/`llvm.ceil`/`llvm.round` | Single instruction on SSE4.1+ / ARM |
| Int/Float conversion | Runtime function with cast | LLVM `sitofp`/`fptosi` instructions | Zero-cost conversion instruction |
| Int min/max | Runtime conditional comparison | LLVM intrinsics `llvm.smin`/`llvm.smax` | Compiles to conditional move, branchless |
| Float min/max | Runtime conditional comparison | LLVM intrinsics `llvm.minnum`/`llvm.maxnum` | IEEE 754 compliant, single instruction |

**Key insight:** Every math operation needed by this phase has a corresponding LLVM intrinsic or instruction. Using runtime functions would add unnecessary overhead and code for operations that LLVM can compile to single machine instructions.

## Common Pitfalls

### Pitfall 1: Math.pi Without Parentheses
**What goes wrong:** If `Math.pi` is registered as a function `() -> Float`, users would need to write `Math.pi()`. But the requirement says "reference Math.pi as a constant."
**Why it happens:** The existing module pattern assumes everything is a function call.
**How to avoid:** Register `pi` in the module with type `Float` (not `() -> Float`). In `lower_field_access`, the expression `Math.pi` without call parentheses resolves to `MirExpr::Var("snow_math_pi", MirType::Float)`. In codegen, intercept `snow_math_pi` and emit the float constant directly.
**Warning signs:** Test `Math.pi` without parentheses early. If it requires `Math.pi()`, the architecture is wrong.

### Pitfall 2: Polymorphic Math.abs/min/max Type Resolution
**What goes wrong:** Using a TyVar for the polymorphic parameter may cause inference to fail if the argument type is not fully resolved at the call site.
**Why it happens:** HM inference with let-generalization can delay type resolution.
**How to avoid:** Use fresh TyVar IDs that don't collide with existing ones (91000 series used by List, 90000 by Map, 99000 by Default). Use a new range like 92000 for Math. At codegen, inspect the resolved MirType of the first argument to select the right LLVM intrinsic.
**Warning signs:** Type error on `Math.abs(1)` or `Math.abs(1.0)` -- means the polymorphic scheme is wrong.

### Pitfall 3: Int Absolute Value of INT_MIN
**What goes wrong:** `abs(INT64_MIN)` = `abs(-9223372036854775808)` has no positive representation in i64. Can cause undefined behavior.
**Why it happens:** Two's complement asymmetry.
**How to avoid:** Use `llvm.abs.i64(val, false)` where the second argument `false` means "is_int_min_poison = false", meaning abs(INT_MIN) wraps to INT_MIN rather than being UB. This matches Rust's `i64::wrapping_abs()` behavior.
**Warning signs:** Segfault or unexpected result when computing `Math.abs(-9223372036854775808)`.

### Pitfall 4: Float-to-Int Conversion Truncation vs Rounding
**What goes wrong:** `Float.to_int(3.9)` returns 3 (truncation toward zero), but users might expect 4 (rounding).
**Why it happens:** LLVM `fptosi` truncates toward zero, not rounds.
**How to avoid:** Document that `Float.to_int()` truncates. Users should use `Math.round()` first if they want rounding: `Float.to_int(Math.round(3.9))` gives 4.
**Warning signs:** User confusion in tests. Make e2e tests explicit about truncation behavior.

### Pitfall 5: Math.pow Return Type
**What goes wrong:** `Math.pow(2, 10)` where both args are Int -- should this return Int or Float?
**Why it happens:** Ambiguity in the requirement spec.
**How to avoid:** The requirement says "Math.pow(base, exp) for numeric exponentiation." The simplest correct approach: `Math.pow` takes two Float arguments and returns Float. For integer exponentiation, users write `Math.pow(Int.to_float(2), Int.to_float(10))`. Alternatively, make Math.pow polymorphic but always return Float. The cleanest design: `Math.pow(Float, Float) -> Float` since `llvm.pow` operates on floats.
**Warning signs:** Type error when passing Int arguments to Math.pow.

### Pitfall 6: LLVM Intrinsic Overloading
**What goes wrong:** `Intrinsic::find("llvm.fabs")` returns `Some`, but `get_declaration` fails because no type parameters were provided.
**Why it happens:** LLVM math intrinsics are overloaded on type (f32, f64, etc.). Must provide the concrete type.
**How to avoid:** Always pass `&[f64_type.into()]` (or appropriate type) to `get_declaration()` for overloaded intrinsics. Check `intrinsic.is_overloaded()` to confirm.
**Warning signs:** Panic at codegen time with "Failed to get intrinsic declaration."

### Pitfall 7: Module Name Collision with Type Names
**What goes wrong:** "Int" and "Float" are already registered as type names in `builtins.rs`. Adding them as module names could cause resolution conflicts.
**Why it happens:** `env.insert("Int", ...)` for the type vs `stdlib_modules["Int"]` for the module use different registries, but the parser/resolver must know to try module resolution for `Int.to_float`.
**How to avoid:** The existing resolution order handles this: `STDLIB_MODULES.contains("Int")` in `lower_field_access` checks the base name against module names first. Since `Int` is a known module name, `Int.to_float(x)` routes through module resolution. The bare name `Int` still resolves as a type via `env.lookup("Int")`. This works because the resolution paths are separate: `Int.foo()` triggers field access lowering (module path), while `Int` as a type annotation goes through type resolution.
**Warning signs:** `Int` or `Float` no longer recognized as type names in annotations after adding them as modules. Test `let x: Int = 42` still compiles.

## Code Examples

### Example 1: LLVM Intrinsic Call via Inkwell

```rust
// Emitting llvm.sqrt.f64 via inkwell's Intrinsic API
use inkwell::intrinsics::Intrinsic;

let sqrt_intrinsic = Intrinsic::find("llvm.sqrt").expect("llvm.sqrt not found");
let f64_type = self.context.f64_type();
let sqrt_fn = sqrt_intrinsic
    .get_declaration(&self.module, &[f64_type.into()])
    .expect("Failed to get sqrt declaration");
let result = self.builder
    .build_call(sqrt_fn, &[arg_val.into()], "sqrt_result")
    .map_err(|e| e.to_string())?
    .try_as_basic_value()
    .left()
    .ok_or("sqrt returned void")?;
```

### Example 2: Int-to-Float Conversion

```rust
// Emitting sitofp instruction for Int.to_float
let int_val = arg_val.into_int_value();
let float_val = self.builder
    .build_signed_int_to_float(int_val, self.context.f64_type(), "int_to_float")
    .map_err(|e| e.to_string())?;
Ok(float_val.into())
```

### Example 3: Float-to-Int Conversion

```rust
// Emitting fptosi instruction for Float.to_int
let float_val = arg_val.into_float_value();
let int_val = self.builder
    .build_float_to_signed_int(float_val, self.context.i64_type(), "float_to_int")
    .map_err(|e| e.to_string())?;
Ok(int_val.into())
```

### Example 4: Math.pi Constant

```rust
// In codegen_var or special-case in codegen_call
if name == "snow_math_pi" {
    return Ok(self.context.f64_type().const_float(std::f64::consts::PI).into());
}
```

### Example 5: Type-Dispatched abs

```rust
// In codegen_call, after recognizing snow_math_abs
match args[0].ty() {
    MirType::Int => {
        let abs_intrinsic = Intrinsic::find("llvm.abs").unwrap();
        let i64_type = self.context.i64_type();
        let i1_type = self.context.bool_type();
        let abs_fn = abs_intrinsic
            .get_declaration(&self.module, &[i64_type.into()])
            .unwrap();
        let is_poison = i1_type.const_int(0, false); // false = wrapping behavior
        let result = self.builder
            .build_call(abs_fn, &[int_val.into(), is_poison.into()], "abs")
            .unwrap()
            .try_as_basic_value().left().unwrap();
        Ok(result)
    }
    MirType::Float => {
        let fabs_intrinsic = Intrinsic::find("llvm.fabs").unwrap();
        let f64_type = self.context.f64_type();
        let fabs_fn = fabs_intrinsic
            .get_declaration(&self.module, &[f64_type.into()])
            .unwrap();
        let result = self.builder
            .build_call(fabs_fn, &[float_val.into()], "fabs")
            .unwrap()
            .try_as_basic_value().left().unwrap();
        Ok(result)
    }
}
```

### Example 6: Snow Source Code (Expected Usage)

```snow
fn main() do
  let x = Math.abs(-42)
  let y = Math.abs(-3.14)
  let m = Math.min(10, 20)
  let pi = Math.pi
  let area = Math.pow(2.0, 10.0)
  let root = Math.sqrt(144.0)
  let rounded = Math.floor(3.7)
  let f = Int.to_float(42)
  let i = Float.to_int(3.14)
  println("${x}")
end
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| C libm FFI for math | LLVM intrinsics for all math ops | LLVM 3+ (always available) | Single instructions, no FFI overhead |
| `llvm.minnum`/`llvm.maxnum` (IEEE 754-2008) | Also `llvm.minimum`/`llvm.maximum` (IEEE 754-2018) | LLVM 12+ | Use minnum/maxnum for NaN-ignoring behavior |
| No integer min/max intrinsics | `llvm.smin`/`llvm.smax`/`llvm.umin`/`llvm.umax` | LLVM 12+ | Direct integer min/max without cmp+select |

## Open Questions

1. **Math.pow Signature: Float-only or Polymorphic?**
   - What we know: LLVM `llvm.pow` takes `(f64, f64) -> f64`. Integer exponentiation would require repeated multiplication or conversion.
   - What's unclear: Should `Math.pow(2, 3)` work with Int arguments, or only Float?
   - Recommendation: Make `Math.pow(Float, Float) -> Float`. Users convert with `Int.to_float()` if needed. This is simple, correct, and matches how every other language handles pow. If Int pow is wanted later, it can be added as a separate function.

2. **Math.sqrt Return Type for Int Arguments**
   - What we know: Requirement says "sqrt returns Float". `llvm.sqrt` operates on f64.
   - What's unclear: Should `Math.sqrt(4)` work with an Int argument (auto-converting)?
   - Recommendation: `Math.sqrt(Float) -> Float` only. Users write `Math.sqrt(Int.to_float(4))`. Explicit is better than implicit. This avoids implicit type coercion which Snow doesn't have.

3. **Math.floor/ceil/round Return Type**
   - What we know: Requirement says "convert Float to Int."
   - What's unclear: Should these return Int directly, or Float? The LLVM intrinsics return f64.
   - Recommendation: Return Int. The codegen emits `llvm.floor` (f64 -> f64) followed by `fptosi` (f64 -> i64). This matches the requirement and user expectation.

4. **Math Constants Beyond pi**
   - What we know: Only Math.pi is required.
   - What's unclear: Should we also add Math.e, Math.inf, Math.nan?
   - Recommendation: Only Math.pi for now. Other constants can be added in future phases. Keep scope minimal.

## Sources

### Primary (HIGH confidence)
- **Snow codebase** -- Direct inspection of all relevant source files:
  - `crates/snow-typeck/src/infer.rs` lines 202-491: stdlib_modules() registry pattern
  - `crates/snow-typeck/src/builtins.rs`: Type registration for primitives
  - `crates/snow-codegen/src/mir/lower.rs` lines 4015-4045, 7198-7353: Module resolution and name mapping
  - `crates/snow-codegen/src/codegen/intrinsics.rs`: Runtime function declarations
  - `crates/snow-codegen/src/codegen/expr.rs` lines 547-601: codegen_call pattern
  - `crates/snowc/tests/e2e_stdlib.rs`: E2E test patterns
- **LLVM Language Reference Manual** -- [LLVM LangRef](https://llvm.org/docs/LangRef.html) -- intrinsic signatures and semantics
- **Inkwell Intrinsic API** -- [Intrinsic struct docs](https://thedan64.github.io/inkwell/inkwell/intrinsics/struct.Intrinsic.html) -- find/get_declaration/build_call pattern
- **Inkwell crates.io** -- [inkwell 0.8.0](https://crates.io/crates/inkwell) -- version confirmation

### Secondary (MEDIUM confidence)
- [LLVM Discourse - Math Intrinsics](https://discourse.llvm.org/t/math-intrinsics/67192) -- Confirmed all needed intrinsics are stable
- [LLVM issue #148388](https://github.com/llvm/llvm-project/issues/148388) -- llvm.abs.i64 behavior confirmed
- [LLVM Discourse - Pow/PowI](https://discourse.llvm.org/t/pow-and-powi-intrinsics/31343) -- llvm.pow vs llvm.powi semantics

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - All LLVM intrinsics verified in official docs; inkwell Intrinsic API confirmed
- Architecture: HIGH - Following exact existing pattern from 14 other stdlib modules in the codebase
- Pitfalls: HIGH - Based on direct codebase inspection; INT_MIN and type resolution issues are well-known
- Code examples: MEDIUM - Inkwell Intrinsic API usage pattern confirmed but not tested in this specific codebase

**Research date:** 2026-02-09
**Valid until:** 2026-03-09 (stable domain; LLVM intrinsics don't change)
