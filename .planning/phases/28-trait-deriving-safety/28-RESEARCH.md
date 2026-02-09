# Phase 28: Trait Deriving Safety - Research

**Researched:** 2026-02-08
**Domain:** Compiler trait dependency validation (Snow compiler, Rust codebase)
**Confidence:** HIGH

## Summary

This phase addresses a known limitation where `deriving(Ord)` without `Eq` causes a runtime error (linker/LLVM failure) instead of a compile-time error with a clear diagnostic. The fix is entirely within the type checker (`snow-typeck`), specifically in the `register_struct_def` and `register_sum_type_def` functions in `crates/snow-typeck/src/infer.rs`.

The root cause is well-understood: the `Ord__compare__` MIR function (generated in `crates/snow-codegen/src/mir/lower.rs`) calls `Eq__eq__TypeName`, and `Ord__lt__` for multi-field structs/sum-type payloads uses `Eq__eq__` in its lexicographic comparison chain. When Ord is derived without Eq, these functions reference a symbol (`Eq__eq__TypeName`) that was never generated, causing LLVM linker failure at build time or undefined behavior at runtime.

The fix is a validation check in the type checker: when `deriving(Ord)` is present but `Eq` is absent, emit a new `TypeError` variant with a helpful message suggesting the user add `Eq` to the deriving list. This is the same pattern already used for `UnsupportedDerive` errors.

**Primary recommendation:** Add a `MissingDerivePrerequisite` error variant to `TypeError` and check for `Ord` without `Eq` in both `register_struct_def` and `register_sum_type_def` right after the existing `valid_derives` validation loop.

## Standard Stack

This phase involves no new libraries or dependencies. All work is within the existing Snow compiler crates.

### Core
| Crate | Location | Purpose | Role in This Phase |
|-------|----------|---------|-------------------|
| snow-typeck | `crates/snow-typeck/` | Type checking and trait validation | Add dependency check + new error variant |
| snow-codegen | `crates/snow-codegen/` | MIR lowering | No changes needed (already correct when Eq is present) |
| snowc | `crates/snowc/` | Compiler driver + e2e tests | Add e2e tests for the new error |

### Supporting
| Crate | Location | Purpose | When Relevant |
|-------|----------|---------|---------------|
| snow-parser | `crates/snow-parser/` | AST nodes with `has_deriving_clause()` / `deriving_traits()` | Read-only; provides span info for diagnostics |

## Architecture Patterns

### Where the Code Lives

```
crates/
  snow-typeck/
    src/
      error.rs         # TypeError enum - ADD new variant here
      diagnostics.rs   # Error rendering with ariadne - ADD rendering here
      infer.rs         # register_struct_def + register_sum_type_def - ADD checks here
  snow-codegen/
    src/
      mir/lower.rs     # MIR generation (NO CHANGES NEEDED)
  snowc/
    tests/
      e2e.rs           # End-to-end tests - ADD tests here
```

### Pattern 1: Trait Dependency Validation (New)
**What:** After extracting the derive list, check that trait dependencies are satisfied before registering impls.
**When to use:** In `register_struct_def` and `register_sum_type_def`, right after the existing "validate derive trait names" loop (lines ~1505-1514 and ~1786-1795).
**Why here:** This is the earliest point where we have the full derive list and can emit type errors. The check must happen BEFORE trait impl registration to prevent partial registration.

The check is simple:
- If `has_deriving` is true (explicit deriving clause), and
- `derive_list` contains "Ord", and
- `derive_list` does NOT contain "Eq"
- Then emit a `TypeError::MissingDerivePrerequisite` error

Note: When `has_deriving` is false (no deriving clause), `derive_all` is true, so both Eq and Ord are always derived together. The bug only manifests with explicit `deriving(Ord)` without `Eq`.

### Pattern 2: Error Variant Pattern (Existing)
**What:** The codebase follows a consistent pattern for adding new errors.
**Steps:**
1. Add variant to `TypeError` enum in `error.rs`
2. Add `Display` impl in `error.rs`
3. Add error code mapping in `diagnostics.rs` (`error_code()` function)
4. Add severity in `diagnostics.rs` (`severity()` function)
5. Add ariadne report rendering in `diagnostics.rs` (`render_diagnostic()` function)
6. Use `ctx.errors.push(TypeError::NewVariant { ... })` at the check site

**Example from existing code (`UnsupportedDerive`):**
```rust
// error.rs - Variant
UnsupportedDerive {
    trait_name: String,
    type_name: String,
},

// error.rs - Display
TypeError::UnsupportedDerive { trait_name, type_name } => {
    write!(f, "cannot derive `{}` for `{}` -- only Eq, Ord, Display, Debug, and Hash are derivable",
        trait_name, type_name)
}

// diagnostics.rs - Error code
TypeError::UnsupportedDerive { .. } => "E0028",

// diagnostics.rs - Rendering
TypeError::UnsupportedDerive { trait_name, type_name } => {
    let msg = format!("cannot derive `{}` for `{}`", trait_name, type_name);
    // ... Report::build with label and help text
}
```

### Pattern 3: E2E Error Test Pattern (Existing)
**What:** Tests that expect compilation errors use `compile_expect_error()`.
**Example from existing code:**
```rust
#[test]
fn e2e_deriving_unsupported_trait() {
    let source = r#"
struct Foo do
  x :: Int
end deriving(Clone)

fn main() do
  let f = Foo { x: 1 }
  println("nope")
end
"#;
    let error = compile_expect_error(source);
    assert!(
        error.contains("cannot derive"),
        "Expected 'cannot derive' error, got: {}",
        error
    );
}
```

### Anti-Patterns to Avoid
- **Checking in MIR lowering:** Do NOT add the check in `snow-codegen`. By the time MIR lowering runs, it's too late for good diagnostics. The type checker is the right place.
- **Auto-adding Eq when Ord is requested:** Do NOT silently add Eq. The user explicitly opted into selective deriving; the compiler should respect that and give a clear error.
- **Blocking Ord impl registration:** The check should emit an error AND skip Ord registration (or skip both Ord and Eq). If Ord is registered without Eq, MIR lowering will still generate broken code.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Error rendering | Custom string formatting | ariadne `Report::build()` | Already used for all 28 existing error codes; consistent formatting |
| Span extraction | Manual TextRange calculation | `struct_def.syntax().text_range()` or deriving clause child node range | Existing AST nodes expose spans |
| Test harness | Custom compilation + error capture | `compile_expect_error()` in e2e.rs | Already exists, handles temp dirs and error capture |

**Key insight:** The entire error infrastructure is already built. This phase is adding one new error variant and one new check, following the exact same patterns used for `UnsupportedDerive` (E0028).

## Common Pitfalls

### Pitfall 1: Forgetting Sum Types
**What goes wrong:** Only adding the check in `register_struct_def` but not `register_sum_type_def`.
**Why it happens:** The two functions are separate and parallel. Both handle deriving independently.
**How to avoid:** Apply the same check in both functions. They appear at lines ~1505 and ~1786 in `infer.rs`.
**Warning signs:** Tests pass for structs but not for sum types.

### Pitfall 2: Not Handling the `derive_all` Case
**What goes wrong:** The check fires even when no deriving clause is present (`derive_all = true`).
**Why it happens:** Without `has_deriving` guard, the check sees "Ord is not in derive_list and neither is Eq" and incorrectly errors.
**How to avoid:** Only check when `has_deriving` is true. When `has_deriving` is false, `derive_all` is true and ALL traits (including both Eq and Ord) are derived.
**Warning signs:** `deriving_backward_compat` test fails (no explicit deriving clause = derive all defaults).

### Pitfall 3: Registering Ord Impl Despite Error
**What goes wrong:** The error is emitted, but the Ord impl is still registered in the trait registry, and MIR lowering still generates `Ord__lt__` and `Ord__compare__` functions that reference the missing `Eq__eq__`.
**Why it happens:** The error check is added but the code continues to register the impl.
**How to avoid:** When the dependency check fails, skip Ord impl registration (don't register `TraitImplDef` for Ord). This prevents MIR lowering from generating broken code. However, also consider: if there are other errors already, the compilation will stop before codegen anyway. The safest approach is to emit the error AND skip the Ord registration.
**Warning signs:** Compilation still crashes at LLVM phase even though the error was emitted.

### Pitfall 4: Generic Struct Monomorphization Path
**What goes wrong:** The check in the type checker catches the error, but the monomorphization path in `ensure_monomorphized_struct_trait_fns` (lower.rs line ~1403) could independently check `has_impl("Ord")` and generate code.
**Why it happens:** MIR lowering checks the trait registry, not the derive list.
**How to avoid:** Since the type checker prevents the Ord impl from being registered when Eq is missing, the MIR lowering path is safe -- `self.trait_registry.has_impl("Ord", typeck_ty)` will return false. No changes needed in MIR lowering IF the type checker correctly skips Ord registration.
**Warning signs:** None if done correctly; the type checker gate is sufficient.

### Pitfall 5: Error Code Collision
**What goes wrong:** Using an existing error code for the new variant.
**Why it happens:** Not checking the existing error code assignments.
**How to avoid:** The current highest error code is E0028 (UnsupportedDerive). Use E0029 for the new variant.
**Warning signs:** Confusing error output that shows the wrong help text.

### Pitfall 6: Span for the Error
**What goes wrong:** The error points to position 0 (start of file) instead of the deriving clause.
**Why it happens:** `UnsupportedDerive` currently uses `clamp(0..source_len)` as a fallback span because it doesn't have the deriving clause's TextRange.
**How to avoid:** Either: (a) pass the deriving clause's span from the AST through to the error (requires adding a span field to the error variant), or (b) use the same fallback approach as `UnsupportedDerive`. Option (b) is simpler and consistent; option (a) is better UX.
**Warning signs:** Error diagnostic doesn't point to the right location.

## Code Examples

### New TypeError Variant
```rust
// In crates/snow-typeck/src/error.rs, add to the TypeError enum:

/// A derived trait requires another trait that is not in the deriving list.
MissingDerivePrerequisite {
    trait_name: String,
    requires: String,
    type_name: String,
},
```

### Display Impl
```rust
// In crates/snow-typeck/src/error.rs, add to the Display match:

TypeError::MissingDerivePrerequisite {
    trait_name,
    requires,
    type_name,
} => {
    write!(
        f,
        "deriving `{}` for `{}` requires `{}` to also be derived",
        trait_name, type_name, requires
    )
}
```

### Error Code Assignment
```rust
// In crates/snow-typeck/src/diagnostics.rs, add to error_code():
TypeError::MissingDerivePrerequisite { .. } => "E0029",

// In severity():
TypeError::MissingDerivePrerequisite { .. } => "error",
```

### Diagnostic Rendering
```rust
// In crates/snow-typeck/src/diagnostics.rs, add to render_diagnostic():
TypeError::MissingDerivePrerequisite {
    trait_name,
    requires,
    type_name,
} => {
    let msg = format!(
        "cannot derive `{}` for `{}` without `{}`",
        trait_name, type_name, requires
    );
    let span = clamp(0..source_len.max(1).min(source_len));

    Report::build(ReportKind::Error, span.clone())
        .with_code(code)
        .with_message(&msg)
        .with_config(config)
        .with_label(
            Label::new(span)
                .with_message(format!(
                    "`{}` requires `{}` for its implementation",
                    trait_name, requires
                ))
                .with_color(Color::Red),
        )
        .with_help(format!(
            "add `{}` to the deriving list: deriving({}, {})",
            requires, requires, trait_name
        ))
        .finish()
}
```

### Validation Check (for both struct and sum type registration)
```rust
// In crates/snow-typeck/src/infer.rs, in register_struct_def and register_sum_type_def,
// add AFTER the valid_derives validation loop and BEFORE trait impl registration:

// Check trait dependencies: Ord requires Eq.
if has_deriving && derive_list.iter().any(|t| t == "Ord") && !derive_list.iter().any(|t| t == "Eq") {
    ctx.errors.push(TypeError::MissingDerivePrerequisite {
        trait_name: "Ord".to_string(),
        requires: "Eq".to_string(),
        type_name: name.clone(),
    });
}
```

### E2E Test for Struct
```rust
#[test]
fn e2e_deriving_ord_without_eq_struct() {
    let source = r#"
struct Foo do
  x :: Int
end deriving(Ord)

fn main() do
  let f = Foo { x: 1 }
  println("nope")
end
"#;
    let error = compile_expect_error(source);
    assert!(
        error.contains("Eq") && (error.contains("requires") || error.contains("without")),
        "Expected error about Ord requiring Eq, got: {}",
        error
    );
}
```

### E2E Test for Sum Type
```rust
#[test]
fn e2e_deriving_ord_without_eq_sum() {
    let source = r#"
type Direction do
  North
  South
end deriving(Ord)

fn main() do
  println("nope")
end
"#;
    let error = compile_expect_error(source);
    assert!(
        error.contains("Eq") && (error.contains("requires") || error.contains("without")),
        "Expected error about Ord requiring Eq, got: {}",
        error
    );
}
```

### Regression Test: Eq + Ord Together Works
```rust
#[test]
fn e2e_deriving_eq_ord_together() {
    let source = r#"
struct Point do
  x :: Int
  y :: Int
end deriving(Eq, Ord)

fn main() do
  let a = Point { x: 1, y: 2 }
  let b = Point { x: 1, y: 3 }
  println("${a == b}")
  println("${a < b}")
end
"#;
    let output = compile_and_run(source);
    assert_eq!(output, "false\ntrue\n");
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| `deriving(Ord)` without `Eq` causes linker error | (This phase) Compile-time error with suggestion | Phase 28 | Users get actionable error instead of cryptic LLVM failure |
| No trait dependency validation | (This phase) Type checker validates dependencies | Phase 28 | Foundation for future trait dependency rules |

**Current behavior (the bug):**
- `deriving(Ord)` without `Eq` generates `Ord__compare__TypeName` which calls `Eq__eq__TypeName`
- `Eq__eq__TypeName` was never generated because Eq wasn't in the derive list
- LLVM linker fails with an undefined symbol error (or worse, undefined behavior)

**Desired behavior (after this phase):**
- `deriving(Ord)` without `Eq` produces error E0029 with help text: "add `Eq` to the deriving list: deriving(Eq, Ord)"

## Open Questions

1. **Should Ord without Eq skip Ord registration entirely, or still register and rely on the error halting compilation?**
   - What we know: Errors in the type checker prevent successful compilation, so codegen never runs. Skipping registration is safer but potentially unnecessary.
   - What's unclear: Whether there are edge cases where compilation continues past type errors (e.g., `--emit mir` debug flags).
   - Recommendation: Skip Ord registration when the dependency is missing. Belt-and-suspenders approach. Wrap the Ord impl registration in an additional guard: `if !(has_deriving && derive_list.iter().any(|t| t == "Ord") && !derive_list.iter().any(|t| t == "Eq"))`.

2. **Should the error have a proper span pointing to the deriving clause?**
   - What we know: The `DERIVING_CLAUSE` syntax node exists in the AST and has a `text_range()`. The type checker has access to `struct_def.syntax().children()`.
   - What's unclear: Whether passing a `TextRange` through the error variant is worth the effort for this phase.
   - Recommendation: Start with the same fallback span as `UnsupportedDerive` (position 0). Improving spans can be done later since it's purely cosmetic.

3. **Are there other trait dependencies besides Ord -> Eq?**
   - What we know: Currently, only `Ord__compare__` calls `Eq__eq__`. `Ord__lt__` calls `Eq__eq__` only for multi-field lexicographic comparison. No other cross-trait dependencies exist in the codebase.
   - What's unclear: Future traits might have dependencies.
   - Recommendation: Build the check to be easily extensible (e.g., a dependency table), but only enforce Ord -> Eq for now.

## Sources

### Primary (HIGH confidence)
- `crates/snow-typeck/src/infer.rs` lines 1498-1600, 1779-1898 - Struct and sum type deriving registration
- `crates/snow-typeck/src/error.rs` - Full TypeError enum (28 variants, E0001-E0028)
- `crates/snow-typeck/src/diagnostics.rs` lines 1312-1333 - UnsupportedDerive rendering pattern
- `crates/snow-codegen/src/mir/lower.rs` lines 2218-2289, 2291-2359 - `generate_compare_struct/sum` calling `Eq__eq__`
- `crates/snow-codegen/src/mir/lower.rs` lines 1810-1898 - `build_lexicographic_lt` calling `Eq__eq__` for multi-field structs
- `crates/snow-codegen/src/mir/lower.rs` lines 2854-2916 - `build_lexicographic_lt_vars` calling `Eq__eq__` for sum type payloads
- `crates/snowc/tests/e2e.rs` lines 584-648 - Existing deriving e2e tests

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - All code is in the existing codebase, no external deps
- Architecture: HIGH - Exact line numbers identified, existing patterns to follow
- Pitfalls: HIGH - Root cause fully understood from reading the MIR generation code
- Code examples: HIGH - Based directly on existing patterns in the codebase

**Research date:** 2026-02-08
**Valid until:** Indefinite (this is internal compiler architecture, not external dependencies)
