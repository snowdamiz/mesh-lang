# Phase 32: Diagnostics and Integration - Research

**Researched:** 2026-02-08
**Domain:** Compiler diagnostics, method resolution error reporting, non-regression integration testing
**Confidence:** HIGH

## Summary

Phase 32 is the final phase of v1.6 and focuses on two categories: (1) polishing ambiguous method diagnostics (DIAG-02, DIAG-03) and (2) verifying that all existing syntax forms work unchanged alongside the new method dot-syntax (INTG-01 through INTG-05). The diagnostic work is small and surgical: `find_method_traits` returns trait names in nondeterministic `FxHashMap` iteration order, and the `AmbiguousMethod` error variant lacks a source span. The integration work is primarily test-writing -- the existing test suite (1249 tests, all passing) provides a strong foundation, but Phase 32 needs targeted tests that explicitly verify each integration point in the presence of method dot-syntax.

The codebase is well-prepared for this phase. The `AmbiguousMethod` error variant, diagnostic rendering (E0027), and `find_method_traits` helper all exist from Phase 18/30. The remaining work is: (a) sort `find_method_traits` output alphabetically, (b) add a `span` field to `AmbiguousMethod` for precise error location, (c) improve the help text to list specific qualified syntax suggestions per trait, and (d) write comprehensive integration tests.

**Primary recommendation:** Fix deterministic ordering in `find_method_traits` (sort returned Vec), add `span: TextRange` to `AmbiguousMethod`, improve the help text to list each trait's qualified syntax, then write targeted integration tests for each INTG requirement.

## Standard Stack

This phase uses only existing project infrastructure. No new dependencies needed.

### Core
| Component | Location | Purpose | Why Standard |
|-----------|----------|---------|--------------|
| `TraitRegistry` | `crates/snow-typeck/src/traits.rs` | Method-to-trait mapping and ambiguity detection | Central trait resolution used by `find_method_traits` |
| `TypeError` | `crates/snow-typeck/src/error.rs` | `AmbiguousMethod` and `NoSuchMethod` variants | Existing error type system |
| `diagnostics.rs` | `crates/snow-typeck/src/diagnostics.rs` | Ariadne-based diagnostic rendering | Existing rendering pipeline with error codes and help text |
| `infer.rs` | `crates/snow-typeck/src/infer.rs` | Method resolution with retry-based fallback | Where `AmbiguousMethod` errors are produced (lines 4067-4075 and 4103-4111) |

### Test Infrastructure
| Component | Location | Purpose | When to Use |
|-----------|----------|---------|-------------|
| `compile_and_run` | `crates/snowc/tests/e2e.rs` | Full-pipeline e2e tests (compile+run+assert stdout) | INTG-01 through INTG-05 e2e tests |
| `check_source` | `crates/snow-typeck/tests/diagnostics.rs` | Parse + type check, returns `TypeckResult` | DIAG-02, DIAG-03 diagnostic tests |
| `render_first_error` | `crates/snow-typeck/tests/diagnostics.rs` | Renders first error via ariadne pipeline | Snapshot tests for `AmbiguousMethod` rendering |
| `insta::assert_snapshot!` | `crates/snow-typeck/tests/diagnostics.rs` | Snapshot testing for diagnostic output | Deterministic ordering verification |
| MIR `lower()` helper | `crates/snow-codegen/src/mir/lower.rs` tests | Parse + type check + MIR lower, returns `MirModule` | MIR-level integration tests |

## Architecture Patterns

### Current Method Resolution Guard Chain (Type Checker)

The `infer_field_access` function in `infer.rs` follows this priority chain:

```
1. STDLIB_MODULES (String.length, IO.read_line, etc.)
2. Service modules (Counter.get_count, etc.)
3. Sum type variants (Shape.Circle, etc.)
4. Struct fields (point.x, point.y)
5. Trait method resolution (point.to_string())
6. Stdlib module method fallback (str.length() -> String.length)
```

Method resolution is last in priority. This is correct and must be preserved.

### Current Method Resolution Guard Chain (MIR Lowering)

The `lower_call_expr` function in `lower.rs` has a parallel guard chain:

```
1. STDLIB_MODULES.contains(base_name)
2. service_modules.contains_key(base_name)
3. is_sum_type_name(base_name)
4. is_struct_type_name(base_name)
5. Method call dispatch (resolve_trait_callee)
```

### Retry-Based Method Resolution

The `infer_call` function uses a two-phase approach:
1. **Normal inference**: Try `infer_expr` on the callee (which calls `infer_field_access(is_method_call=false)`)
2. **Method fallback**: If step 1 fails with `NoSuchField` and callee is a `FieldAccess`, retry with `infer_field_access(is_method_call=true)`

The retry mechanism removes the `NoSuchField` error from `ctx.errors` before retrying (line 2700-2703).

### Pattern: Diagnostic Test Structure

Existing diagnostic tests follow two patterns:

**Pattern A: Constructed TypeError (for errors without source context)**
```rust
let err = TypeError::AmbiguousMethod {
    method_name: "to_string".to_string(),
    candidate_traits: vec!["Display".to_string(), "Printable".to_string()],
    ty: Ty::int(),
};
let output = render_diagnostic(&err, src, "test.snow", &opts(), None);
insta::assert_snapshot!(output);
```

**Pattern B: Source-driven (parse + type check, then render)**
```rust
let src = "let x :: Int = \"hello\"";
let result = check_source(src);
assert!(!result.errors.is_empty());
let output = render_first_error(src);
insta::assert_snapshot!(output);
```

Phase 32 diagnostic tests should use Pattern A (constructed errors) because ambiguity requires registering multiple traits with the same method for the same type, which is not easily triggered via standard Snow source (e2e tests use `deriving()` which registers specific traits).

### Pattern: E2E Integration Tests

```rust
#[test]
fn e2e_feature_preserved() {
    let source = r#"
fn main() do
  // code that exercises the feature
  println("expected output")
end
"#;
    let output = compile_and_run(source);
    assert_eq!(output.trim(), "expected output");
}
```

### Anti-Patterns to Avoid
- **Testing integration at only one level:** Tests should cover both type-checker level (no errors produced) AND e2e level (correct runtime output) for each INTG requirement.
- **Non-deterministic test assertions:** Never assert on the exact ordering of `candidate_traits` without first sorting -- the current `FxHashMap` iteration is nondeterministic.
- **Modifying the guard chain priority:** The existing priority order (module > service > variant > struct field > method) is a locked v1.6 decision. Phase 32 must not change it.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Deterministic HashMap iteration | Custom ordered map | Sort the `Vec<String>` returned by `find_method_traits` | Simple `.sort()` is sufficient; no need for `BTreeMap` since the HashMap is never iterated for ordering purposes elsewhere |
| Diagnostic rendering | Custom error formatting | Existing ariadne pipeline + `render_diagnostic` | Full diagnostic infrastructure already exists with error codes, spans, help text |
| Snapshot testing | Manual string comparison | `insta::assert_snapshot!` | Already used for all diagnostic tests; handles update workflow |

**Key insight:** Phase 32 is surgical fixes to existing infrastructure, not new infrastructure. Every component (error variant, diagnostic rendering, test helpers) already exists. The work is fixing ordering, adding a span field, improving help text, and writing tests.

## Common Pitfalls

### Pitfall 1: Breaking existing AmbiguousMethod tests by adding span field
**What goes wrong:** Adding `span: TextRange` to `TypeError::AmbiguousMethod` is a struct-level breaking change. Every match arm, construction site, and test that uses `AmbiguousMethod` must be updated.
**Why it happens:** The variant is used in 5+ locations: `error.rs` (definition), `diagnostics.rs` (rendering), `infer.rs` (two construction sites), `analysis.rs` (LSP span extraction), and unit tests in `traits.rs`.
**How to avoid:** Update all sites in one commit. Key locations:
  - `crates/snow-typeck/src/error.rs` line 235 (definition)
  - `crates/snow-typeck/src/error.rs` line 492 (Display impl)
  - `crates/snow-typeck/src/diagnostics.rs` line 121 (error code)
  - `crates/snow-typeck/src/diagnostics.rs` line 1310 (rendering -- use actual span instead of `0..source_len`)
  - `crates/snow-typeck/src/diagnostics.rs` line ~465 (JSON rendering, catch-all arm)
  - `crates/snow-typeck/src/infer.rs` lines 4069 and 4105 (construction -- add `fa.syntax().text_range()`)
  - `crates/snow-lsp/src/analysis.rs` line 202 (change `None` to `Some(*span)`)
**Warning signs:** Compilation errors in `analysis.rs` after changing the error variant.

### Pitfall 2: Sorting candidate traits at the wrong level
**What goes wrong:** Sorting inside `find_method_traits` instead of at the call site, or sorting at the call site but not in `find_method_traits`.
**Why it happens:** `find_method_traits` is called from 3 locations: two in `infer.rs` (type checker) and one in `lower.rs` (MIR). Sorting must happen consistently.
**How to avoid:** Sort inside `find_method_traits` itself (line 286, before returning). This ensures ALL callers get deterministic ordering. The MIR lowering's `resolve_trait_callee` at line 3387 takes `matching_traits[0]` -- with sorted output, this will consistently pick the alphabetically-first trait, which is deterministic even though it doesn't error on ambiguity (the type checker already caught it).
**Warning signs:** Tests pass on one machine but fail on another due to HashMap ordering differences.

### Pitfall 3: Help text not matching success criteria format
**What goes wrong:** The success criteria specifically says the error should suggest `Display.to_string(point)` (with the concrete trait name and receiver). The current help text says `use qualified syntax: TraitName.method_name(value)` (generic placeholder).
**Why it happens:** The diagnostic was written before the success criteria were finalized.
**How to avoid:** Generate help text that lists each candidate trait's qualified syntax: `"use qualified syntax: Display.to_string(value) or Printable.to_string(value)"`.
**Warning signs:** Success criteria verification fails because the error message doesn't include actual trait names in the suggestion.

### Pitfall 4: Regression in pipe operator with method calls
**What goes wrong:** `value |> method(args)` might be intercepted as a method call when it should be a pipe-to-function-call.
**Why it happens:** The pipe operator desugars to `method(value, args)` in MIR. If `method` happens to match a trait method name (e.g., `to_string`), the MIR lowering could misroute it.
**How to avoid:** The pipe operator desugars in `lower_pipe_expr`, which calls `lower_expr` on the RHS. If the RHS is a bare name (not a FieldAccess), it won't hit the method interception path in `lower_call_expr`. This should already be safe, but needs explicit testing.
**Warning signs:** Pipe tests fail or produce wrong output after Phase 32 changes.

### Pitfall 5: Actor `self` keyword conflict with method `self` parameter
**What goes wrong:** Inside an actor receive block, `self` refers to the actor's PID. Method dot-syntax uses `self` as the first parameter of trait methods. If these interact, confusion results.
**Why it happens:** Both use the word "self" but in different contexts.
**How to avoid:** Actor `self` is handled by `infer_self` which checks `ACTOR_MSG_TYPE_KEY` in the environment. Method resolution's `self` is the struct/type receiver passed as the first argument. These are separate mechanisms and should not interact. Write an explicit test: inside an actor receive block, call a method on a local variable (not `self`) to confirm both systems coexist.
**Warning signs:** Method calls inside actor blocks produce `SelfOutsideActor` errors.

## Code Examples

### DIAG-03: Sort find_method_traits output (in traits.rs)

```rust
// In TraitRegistry::find_method_traits, before returning:
pub fn find_method_traits(&self, method_name: &str, ty: &Ty) -> Vec<String> {
    let mut trait_names = Vec::new();
    for (trait_name, impl_list) in &self.impls {
        // ... existing matching logic ...
    }
    trait_names.sort(); // DIAG-03: deterministic alphabetical ordering
    trait_names
}
```

### DIAG-02: Add span to AmbiguousMethod and improve help text

```rust
// In error.rs:
AmbiguousMethod {
    method_name: String,
    candidate_traits: Vec<String>,
    ty: Ty,
    span: TextRange,  // NEW: source location of the ambiguous call
},

// In diagnostics.rs rendering:
TypeError::AmbiguousMethod {
    method_name,
    candidate_traits,
    ty,
    span,  // NEW
} => {
    let msg = format!(
        "ambiguous method `{}` for type `{}`: candidates from traits [{}]",
        method_name, ty, candidate_traits.join(", ")
    );
    let range = clamp(text_range_to_range(*span));  // Use actual span

    // Build help text listing each candidate's qualified syntax
    let suggestions: Vec<String> = candidate_traits
        .iter()
        .map(|t| format!("{}.{}(value)", t, method_name))
        .collect();
    let help = format!("use qualified syntax: {}", suggestions.join(" or "));

    Report::build(ReportKind::Error, range.clone())
        .with_code(code)
        .with_message(&msg)
        .with_config(config)
        .with_label(
            Label::new(range)
                .with_message(format!("multiple traits provide `{}`", method_name))
                .with_color(Color::Red),
        )
        .with_help(help)
        .finish()
}

// In infer.rs (both construction sites):
let err = TypeError::AmbiguousMethod {
    method_name: field_name.clone(),
    candidate_traits: matching_traits,
    ty: resolved_base.clone(),
    span: fa.syntax().text_range(),  // NEW: capture the FieldAccess span
};
```

### INTG-01: Struct field access preservation test

```rust
#[test]
fn e2e_phase32_struct_field_access_preserved() {
    let source = r#"
struct Point do
  x :: Int
  y :: Int
end deriving(Display)

fn main() do
  let p = Point { x: 42, y: 99 }
  println("${p.x}")
  println("${p.y}")
  println(p.to_string())
end
"#;
    let output = compile_and_run(source);
    assert_eq!(output, "42\n99\nPoint(42, 99)\n");
}
```

### INTG-03: Pipe operator preserved with method dot-syntax

```rust
#[test]
fn e2e_phase32_pipe_operator_preserved() {
    let source = r#"
fn double(x :: Int) -> Int do
  x * 2
end

fn main() do
  let result = 5 |> double
  println("${result}")
end
"#;
    let output = compile_and_run(source);
    assert_eq!(output.trim(), "10");
}
```

### INTG-05: Actor self not affected by method calls

```rust
#[test]
fn e2e_phase32_actor_self_not_affected() {
    let source = r#"
actor counter(initial :: Int) :: Int do
  let count = initial
  receive do
    n ->
      let new_count = count + n
      counter(new_count)
  end
end

fn main() do
  let pid = spawn(counter(0))
  send(pid, 1)
  IO.sleep(50)
  println("ok")
end
"#;
    let output = compile_and_run(source);
    assert!(output.contains("ok"));
}
```

## Detailed File Impact Analysis

### Files That Must Be Modified

| File | Changes | Risk |
|------|---------|------|
| `crates/snow-typeck/src/traits.rs` line 286 | Add `trait_names.sort()` before return in `find_method_traits` | LOW -- one-line change, deterministic improvement |
| `crates/snow-typeck/src/error.rs` line 235 | Add `span: TextRange` field to `AmbiguousMethod` | MEDIUM -- struct change propagates to all match sites |
| `crates/snow-typeck/src/error.rs` line 492 | Update Display impl for `AmbiguousMethod` (no span in message) | LOW |
| `crates/snow-typeck/src/diagnostics.rs` line 1310-1337 | Use actual span, improve help text with specific trait names | LOW -- localized rendering change |
| `crates/snow-typeck/src/diagnostics.rs` line ~465 | Update JSON rendering catch-all to handle `AmbiguousMethod` with span | LOW |
| `crates/snow-typeck/src/infer.rs` lines 4069-4073 | Add `span: fa.syntax().text_range()` to `AmbiguousMethod` construction | LOW |
| `crates/snow-typeck/src/infer.rs` lines 4105-4109 | Add `span: fa.syntax().text_range()` to `AmbiguousMethod` construction | LOW |
| `crates/snow-lsp/src/analysis.rs` line 202 | Change `None` to `Some(*span)` | LOW |

### Files That Need New Tests

| File | Tests to Add | Purpose |
|------|-------------|---------|
| `crates/snow-typeck/tests/diagnostics.rs` | `test_diag_ambiguous_method_sorted`, `test_diag_ambiguous_method_help_text`, `test_diag_ambiguous_method_with_span` | DIAG-02, DIAG-03 |
| `crates/snow-typeck/src/traits.rs` (unit tests) | Verify `find_method_traits` returns sorted Vec | DIAG-03 |
| `crates/snowc/tests/e2e.rs` | `e2e_phase32_struct_field_preserved`, `e2e_phase32_module_qualified_preserved`, `e2e_phase32_pipe_preserved`, `e2e_phase32_sum_type_variant_preserved`, `e2e_phase32_actor_self_preserved` | INTG-01 through INTG-05 |
| `crates/snow-codegen/src/mir/lower.rs` (unit tests) | MIR-level integration tests for each syntax form | INTG verification at MIR level |

### Files That Should NOT Change

| File | Why |
|------|-----|
| `crates/snow-typeck/src/infer.rs` (guard chain logic) | Priority order is a locked v1.6 decision |
| `crates/snow-codegen/src/mir/lower.rs` (guard chain logic) | Parallel guard chain must match type checker |
| `crates/snow-typeck/src/ty.rs` | No type system changes needed |
| `crates/snow-typeck/src/unify.rs` | No unification changes needed |
| All CST/parser files | No syntax changes in this phase |

## Requirement Mapping to Implementation

| Requirement | What Must Change | Where | Effort |
|-------------|-----------------|-------|--------|
| DIAG-02 | Sort `candidate_traits` alphabetically, improve help text to list specific qualified syntax per trait, add span to error | `traits.rs`, `error.rs`, `diagnostics.rs`, `infer.rs`, `analysis.rs` | Small |
| DIAG-03 | Sort inside `find_method_traits` before return | `traits.rs` line 286 | Trivial (1 line) |
| INTG-01 | Write e2e test: struct field access + method call on same struct | `e2e.rs` | Small (test only) |
| INTG-02 | Write e2e test: `String.length(s)` alongside `s.length()` | `e2e.rs` | Small (test only) |
| INTG-03 | Write e2e test: pipe operator + method dot-syntax | `e2e.rs` | Small (test only) |
| INTG-04 | Write e2e test: `Shape.Circle` variant access alongside method calls | `e2e.rs` | Small (test only) |
| INTG-05 | Write e2e test: actor with receive block + method calls on local vars | `e2e.rs` | Small (test only) |

## Open Questions

1. **Should `AmbiguousMethod` span addition also update the `Display` impl?**
   - What we know: The `Display` impl (error.rs line 492) formats the error as a one-line string. It currently does not include span info (no other `Display` impls do either).
   - What's unclear: Whether the Display output should change at all.
   - Recommendation: Do NOT include span in Display output -- it follows existing conventions. Only the ariadne rendering and JSON output use spans.

2. **Should the MIR lowering also check for ambiguity in `resolve_trait_callee`?**
   - What we know: The type checker catches ambiguity and emits `AmbiguousMethod` before MIR lowering runs. The MIR code at `lower.rs:3387` takes `matching_traits[0]` without sorting.
   - What's unclear: Whether a defensive sort/check is needed in MIR as well.
   - Recommendation: Yes, add `matching_traits.sort()` in `resolve_trait_callee` as well (defense-in-depth). Even though the type checker catches ambiguity, the MIR lowering should produce deterministic output regardless. If ambiguity somehow reaches MIR, the sorted order ensures consistent behavior.

3. **Should INTG tests test BOTH the presence of method dot-syntax AND the traditional syntax in the SAME program?**
   - What we know: Phase 30/31 already have e2e tests for field access + method on same struct (`e2e_method_dot_syntax_field_access_preserved`). Phase 32 success criteria say "continue to work exactly as before."
   - What's unclear: Whether we need new tests if existing ones already cover it.
   - Recommendation: Write new Phase 32-labeled tests that explicitly combine each syntax form WITH method dot-syntax in the same program. This documents the v1.6 guarantee and makes regression detection explicit. The existing Phase 30/31 tests are good but they test method dot-syntax; Phase 32 tests should be framed as "traditional syntax is preserved" tests.

## Sources

### Primary (HIGH confidence)
- `crates/snow-typeck/src/traits.rs` -- `TraitRegistry` implementation, `find_method_traits` (line 269-287), `FxHashMap` iteration (line 271)
- `crates/snow-typeck/src/error.rs` -- `TypeError::AmbiguousMethod` variant (line 235-240), no `span` field
- `crates/snow-typeck/src/diagnostics.rs` -- `AmbiguousMethod` rendering (line 1310-1337), error code E0027, fallback span `0..source_len`
- `crates/snow-typeck/src/infer.rs` -- Method resolution with retry (line 2693-2738), ambiguity detection (line 4067-4075 and 4103-4111), guard chain (line 3987-4027)
- `crates/snow-codegen/src/mir/lower.rs` -- MIR method dispatch (line 3468-3570), `resolve_trait_callee` (line 3377-3464), pipe desugaring (line 3809-3852), guard chain (line 3476-3496)
- `crates/snow-lsp/src/analysis.rs` -- LSP span extraction, `AmbiguousMethod` returns `None` (line 202)
- `crates/snow-typeck/tests/diagnostics.rs` -- Diagnostic test patterns with `insta::assert_snapshot!`
- `crates/snowc/tests/e2e.rs` -- E2E test patterns with `compile_and_run`

### Secondary (HIGH confidence)
- Phase 30 research/plans: Method resolution architecture decisions
- Phase 31 research/plans: Extended method support, stdlib fallback
- Test suite: 1249 tests all passing, providing strong regression baseline

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- All components are existing project code, no external dependencies
- Architecture: HIGH -- Method resolution pipeline, guard chain, error handling all thoroughly documented in prior phases
- Pitfalls: HIGH -- Identified from direct code analysis, not hypothetical
- Implementation approach: HIGH -- Changes are small and surgical with clear locations

**Research date:** 2026-02-08
**Valid until:** 2026-03-08 (stable -- compiler internals, no external dependency drift)
