# Phase 13: String Pattern Matching - Research

**Researched:** 2026-02-07
**Domain:** Compiler string pattern matching (parsing, type checking, MIR lowering, pattern compilation, LLVM codegen)
**Confidence:** HIGH

## Summary

This phase enables string literal patterns in `case` expressions with compile-time generated code. After thorough investigation of the Snow compiler codebase, the major finding is that **most of the infrastructure already exists** but has **two placeholder/buggy implementations** that need to be fixed:

1. **Codegen placeholder (critical):** The `codegen_test` function in `codegen/pattern.rs` line 320-324 has a placeholder for `MirLiteral::String` that always returns `const_int(0, false)` (always false). This means string pattern matching compiles but **never matches any string**. The fix is to call `snow_string_eq` (which already exists in the runtime and is already declared as an intrinsic).

2. **Exhaustiveness value extraction bug (moderate):** The `ast_pattern_to_abstract` function in `infer.rs` line 2982-2984 uses `token.text()` for `STRING_START`, which gives `"` (the opening quote character) instead of the actual string content. This means all string patterns look identical to the exhaustiveness checker. The fix is to extract the STRING_CONTENT child text instead.

3. **Binary string comparison placeholder (bonus):** The `codegen_string_compare` function in `codegen/expr.rs` line 419-434 also has a placeholder that always returns false for `==` and true for `!=`. This should also be fixed to call `snow_string_eq`, as it affects the broader string experience.

**Primary recommendation:** Fix the two placeholders/bugs. The parser, type checker, MIR types, pattern compiler, and decision tree structure already support string patterns end-to-end. This is primarily a codegen-level fix phase.

## Standard Stack

No new libraries or dependencies needed. Everything uses existing crates and runtime functions.

### Core (Already Existing)
| Component | Location | Purpose | Status |
|-----------|----------|---------|--------|
| `snow_string_eq` | `snow-rt/src/string.rs:263` | Runtime string equality comparison | Implemented, returns i8 (0/1) |
| `snow_string_eq` intrinsic | `codegen/intrinsics.rs:184-186` | LLVM declaration for the runtime function | Already declared |
| `MirLiteral::String` | `mir/mod.rs:396` | MIR literal representation for strings | Already exists |
| `MirPattern::Literal(MirLiteral::String(_))` | `mir/mod.rs:374` | Pattern representation for string literals | Already exists |
| `AbsLitKind::String` | `exhaustiveness.rs:21` | Abstract pattern kind for strings | Already exists |
| `TypeInfo::Infinite` | `exhaustiveness.rs:71` | Type classification for open types (String, Int) | Already exists |
| String CST parsing in patterns | `parser/patterns.rs:105-128` | Parser handles STRING_START in pattern position | Already works |
| `extract_simple_string_content` | `mir/lower.rs:3578` | Extracts string content from CST nodes | Already works |

## Architecture Patterns

### Existing Pipeline Flow (Already Working)

```
Source: case name do "alice" -> 1; "bob" -> 2; _ -> 0 end

1. Parser: STRING_START token -> LITERAL_PAT node in MATCH_ARM
   (parser/patterns.rs:105-128 -- already handles string patterns)

2. Type Checker: STRING_START -> Ty::string()
   (infer.rs:3662 -- infer_pattern already types string patterns)

3. Exhaustiveness: String -> TypeInfo::Infinite -> requires wildcard
   (infer.rs:3086-3087 -- already classifies String as Infinite)

4. MIR Lowering: STRING_START -> extract_simple_string_content -> MirLiteral::String
   (lower.rs:1537-1540 -- already extracts string content)

5. Pattern Compilation: MirLiteral::String -> DecisionTree::Test chain
   (pattern/compile.rs:542-580 -- compile_literal_tests already handles all literals)

6. LLVM Codegen: DecisionTree::Test with MirLiteral::String
   (codegen/pattern.rs:320-324 -- **PLACEHOLDER: always returns false**)
```

### What Needs to Change

```
Fix 1: codegen/pattern.rs - codegen_test() MirLiteral::String branch
  Before: self.context.bool_type().const_int(0, false)  // always false
  After:  Create snow_string_new for pattern literal,
          call snow_string_eq(scrutinee, pattern_literal),
          compare result != 0

Fix 2: infer.rs - ast_pattern_to_abstract() STRING_START branch
  Before: value: token.text().to_string()  // gives '"' character
  After:  Extract STRING_CONTENT children from the LITERAL_PAT node

Fix 3: codegen/expr.rs - codegen_string_compare() (bonus)
  Before: const_int(0/1) placeholder
  After:  Call snow_string_eq and handle EQ/NotEq
```

### Pattern: String Test Codegen (the fix)

The `codegen_test` method needs to:
1. Create a `snow_string_new` call with the pattern literal's data (global constant + length)
2. Call `snow_string_eq(scrutinee_value, pattern_literal_value)`
3. Compare the i8 result to zero (snow_string_eq returns i8: 1=equal, 0=not)
4. Use the resulting i1 as the branch condition

This mirrors how string literals are already created in `codegen_string_lit()` (expr.rs:150-179) and follows the same pattern as `snow_string_concat` calls.

### Pattern: Exhaustiveness Value Fix

The `ast_pattern_to_abstract` function needs to extract the actual string content from the LITERAL_PAT syntax node. The node structure is:

```
LITERAL_PAT
  STRING_START: '"'
  STRING_CONTENT: 'alice'
  STRING_END: '"'
```

The fix should walk the children of `lit.syntax()` (the Pattern::Literal's inner LiteralPat syntax node) to find STRING_CONTENT tokens and concatenate their text, similar to `extract_simple_string_content` in the MIR lowering.

### Anti-Patterns to Avoid
- **Do NOT add a new decision tree node type for strings:** The existing `DecisionTree::Test` with `MirLiteral::String` is the correct abstraction. String tests are just literal equality tests, same as Int/Float/Bool.
- **Do NOT add a new pattern compilation path:** `compile_literal_tests` already handles all literal types uniformly. The fix is purely at the codegen level.
- **Do NOT change the exhaustiveness algorithm:** `TypeInfo::Infinite` correctly models strings as an open set. The only fix needed is extracting the right value from the AST.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| String equality | Inline memcmp/byte comparison | `snow_string_eq` runtime function | Already handles length-prefixed format, null safety |
| String allocation for patterns | Manual GC alloc | `snow_string_new` via `codegen_string_lit()` | Already handles global constant + GC allocation |
| Pattern compilation for strings | Custom string switch | `compile_literal_tests` (existing) | Already produces correct Test chain for all literal types |
| Exhaustiveness for strings | Custom string exhaustiveness | `TypeInfo::Infinite` (existing) | Strings are infinite type, wildcard always required |

## Common Pitfalls

### Pitfall 1: Using token.text() for STRING_START
**What goes wrong:** `token.text()` for a STRING_START token returns `"` (the quote character), not the string content.
**Why it happens:** The lexer splits strings into START/CONTENT/END tokens. The content is in STRING_CONTENT children.
**How to avoid:** Always use the parent LITERAL_PAT node and walk its children for STRING_CONTENT.
**Warning signs:** All string patterns look identical in exhaustiveness checking, or patterns are `"` instead of actual values.

### Pitfall 2: Forgetting to create the string value for pattern comparison
**What goes wrong:** The scrutinee is a runtime SnowString pointer, but the pattern literal needs to also be a SnowString pointer for `snow_string_eq`.
**Why it happens:** Int/Float/Bool pattern values are immediate LLVM values. String patterns need runtime allocation via `snow_string_new`.
**How to avoid:** Use `codegen_string_lit` or replicate its pattern (global constant + snow_string_new call) to create the comparison value.
**Warning signs:** LLVM type mismatch errors, segfaults at runtime.

### Pitfall 3: i8 vs i1 boolean mismatch
**What goes wrong:** `snow_string_eq` returns i8 (0 or 1), but LLVM conditional branches need i1.
**Why it happens:** C ABI uses i8 for booleans, LLVM uses i1.
**How to avoid:** Compare the i8 result against 0 using `build_int_compare(IntPredicate::NE, result, const_0, ...)` to produce an i1.
**Warning signs:** LLVM verification errors about type mismatch in conditional branch.

### Pitfall 4: String compare placeholder in binary expressions
**What goes wrong:** `"hello" == "hello"` always evaluates to false in the current codebase.
**Why it happens:** `codegen_string_compare` in expr.rs is a placeholder that always returns false for EQ.
**How to avoid:** Fix this alongside the pattern matching fix, since it uses the same `snow_string_eq` function.
**Warning signs:** String equality tests in if-expressions always take the else branch.

### Pitfall 5: String patterns in multi-clause functions
**What goes wrong:** String patterns in multi-clause function definitions might not be handled correctly if the same codegen path is broken.
**Why it happens:** Multi-clause functions (Phase 11) and closures (Phase 12) use the same `compile_match` -> `codegen_decision_tree` path as case expressions.
**How to avoid:** Fixing the `codegen_test` method fixes all three contexts (case, multi-clause fn, multi-clause closure) at once.
**Warning signs:** String pattern matching works in case but fails in multi-clause functions.

## Code Examples

### Fix 1: codegen_test for MirLiteral::String (codegen/pattern.rs)

```rust
// Current (placeholder):
MirLiteral::String(_) => {
    self.context.bool_type().const_int(0, false)
}

// Fixed:
MirLiteral::String(s) => {
    // Create a SnowString for the pattern literal
    let pattern_str = self.codegen_string_lit(s)?;

    // Call snow_string_eq(scrutinee, pattern)
    let eq_fn = get_intrinsic(&self.module, "snow_string_eq");
    let result = self.builder
        .build_call(eq_fn, &[test_val.into(), pattern_str.into()], "str_eq")
        .map_err(|e| e.to_string())?;
    let i8_result = result.try_as_basic_value()
        .basic()
        .ok_or("snow_string_eq returned void")?
        .into_int_value();

    // Convert i8 result to i1 for branch condition
    let zero = self.context.i8_type().const_int(0, false);
    self.builder
        .build_int_compare(IntPredicate::NE, i8_result, zero, "str_eq_bool")
        .map_err(|e| e.to_string())?
}
```

### Fix 2: ast_pattern_to_abstract for STRING_START (infer.rs)

```rust
// Current (buggy):
SyntaxKind::STRING_START => AbsPat::Literal {
    value: token.text().to_string(), // gives '"'
    ty: AbsLitKind::String,
},

// Fixed:
SyntaxKind::STRING_START => {
    // Extract actual string content from the LITERAL_PAT node's children
    let mut content = String::new();
    for child in lit.syntax().children_with_tokens() {
        if child.kind() == SyntaxKind::STRING_CONTENT {
            if let Some(tok) = child.as_token() {
                content.push_str(tok.text());
            }
        }
    }
    AbsPat::Literal {
        value: content,
        ty: AbsLitKind::String,
    }
}
```

### Fix 3: codegen_string_compare for binary == (codegen/expr.rs)

```rust
// Current (placeholder):
fn codegen_string_compare(
    &mut self,
    op: &BinOp,
    _lhs: BasicValueEnum<'ctx>,
    _rhs: BasicValueEnum<'ctx>,
) -> Result<BasicValueEnum<'ctx>, String> {
    let result = match op {
        BinOp::Eq => self.context.bool_type().const_int(0, false),
        BinOp::NotEq => self.context.bool_type().const_int(1, false),
        _ => return Err(format!("Unsupported string comparison: {:?}", op)),
    };
    Ok(result.into())
}

// Fixed:
fn codegen_string_compare(
    &mut self,
    op: &BinOp,
    lhs: BasicValueEnum<'ctx>,
    rhs: BasicValueEnum<'ctx>,
) -> Result<BasicValueEnum<'ctx>, String> {
    let eq_fn = get_intrinsic(&self.module, "snow_string_eq");
    let result = self.builder
        .build_call(eq_fn, &[lhs.into(), rhs.into()], "str_eq")
        .map_err(|e| e.to_string())?;
    let i8_result = result.try_as_basic_value()
        .basic()
        .ok_or("snow_string_eq returned void")?
        .into_int_value();

    let zero = self.context.i8_type().const_int(0, false);
    let eq_result = self.builder
        .build_int_compare(IntPredicate::NE, i8_result, zero, "str_eq_bool")
        .map_err(|e| e.to_string())?;

    let final_result = match op {
        BinOp::Eq => eq_result,
        BinOp::NotEq => self.builder
            .build_not(eq_result, "str_neq")
            .map_err(|e| e.to_string())?,
        _ => return Err(format!("Unsupported string comparison: {:?}", op)),
    };

    Ok(final_result.into())
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| String test always false (placeholder) | Needs `snow_string_eq` call | Phase 13 | Enables string pattern matching |
| String `==` always false (placeholder) | Needs `snow_string_eq` call | Phase 13 | Enables string equality in expressions |
| Exhaustiveness uses `"` for all string patterns | Needs STRING_CONTENT extraction | Phase 13 | Enables proper redundancy/exhaustiveness for strings |

**What already works fully:**
- Parser: String patterns in case, fn clauses, closures (since Phase 4/11/12)
- Type checker: String patterns type-check as `String` (since Phase 4)
- Exhaustiveness: Strings classified as `Infinite` type, wildcard required (since Phase 4)
- MIR lowering: String patterns correctly extracted to `MirLiteral::String` (since Phase 4)
- Pattern compilation: `compile_literal_tests` produces correct `DecisionTree::Test` chain (since Phase 4)
- Runtime: `snow_string_eq` implemented and tested (since Phase 8)
- Intrinsics: `snow_string_eq` declared as extern function (since Phase 8)

**What does NOT work (the actual work for Phase 13):**
- Codegen: `codegen_test` for `MirLiteral::String` returns false always
- Codegen: `codegen_string_compare` for binary `==`/`!=` returns false/true always
- Exhaustiveness: String pattern values all show as `"` instead of actual content

## Open Questions

1. **String interning for pattern comparison**
   - What we know: Each `codegen_string_lit` call creates a new `snow_string_new` allocation. For patterns tested in a loop, this creates a new string each iteration.
   - What's unclear: Whether this matters for performance. Pattern strings are small constants.
   - Recommendation: Accept the simple approach (call `snow_string_new` each time the test runs). String interning is an optimization for a future phase if needed. The runtime GC handles deallocation.

2. **Negative literal pattern syntax extraction**
   - What we know: `extract_simple_string_content` walks children of the LITERAL_PAT node for STRING_CONTENT. The same approach should work in `ast_pattern_to_abstract`.
   - What's unclear: Whether the `lit.syntax()` in `ast_pattern_to_abstract` is the LITERAL_PAT node (it should be, based on the Pattern::Literal wrapping).
   - Recommendation: Verify by checking `LiteralPat` AST node -- it has `syntax: SyntaxNode` of kind LITERAL_PAT, confirmed in `pat.rs:70`.

3. **Escape sequences in string patterns**
   - What we know: The lexer handles escape sequences. STRING_CONTENT tokens should contain the already-processed text.
   - What's unclear: Whether escape sequences like `\n` are expanded in STRING_CONTENT or remain literal backslash-n.
   - Recommendation: Test with an escape sequence pattern. If the lexer doesn't expand escapes, the MIR lowering and exhaustiveness will at least be consistent (both use the same raw text).

## Sources

### Primary (HIGH confidence)
- Codebase analysis of all relevant files:
  - `crates/snow-codegen/src/codegen/pattern.rs` - Decision tree LLVM translation (placeholder found at line 320-324)
  - `crates/snow-codegen/src/codegen/expr.rs` - Expression codegen (placeholder found at line 419-434)
  - `crates/snow-codegen/src/pattern/compile.rs` - Pattern matrix compilation (already handles strings correctly)
  - `crates/snow-codegen/src/pattern/mod.rs` - Decision tree types (already has string support)
  - `crates/snow-codegen/src/mir/mod.rs` - MIR types (MirLiteral::String already exists)
  - `crates/snow-codegen/src/mir/lower.rs` - MIR lowering (string patterns correctly lowered)
  - `crates/snow-codegen/src/codegen/intrinsics.rs` - Runtime intrinsic declarations (snow_string_eq already declared)
  - `crates/snow-typeck/src/infer.rs` - Type inference and exhaustiveness (string value extraction bug found)
  - `crates/snow-typeck/src/exhaustiveness.rs` - Exhaustiveness algorithm (String as Infinite type works correctly)
  - `crates/snow-parser/src/parser/patterns.rs` - Pattern parser (string patterns already supported)
  - `crates/snow-rt/src/string.rs` - Runtime string operations (snow_string_eq implemented and tested)

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - Direct codebase analysis, all relevant files read
- Architecture: HIGH - Complete pipeline traced from parser to LLVM codegen
- Pitfalls: HIGH - Found actual bugs/placeholders through code reading
- Code examples: HIGH - Based on existing code patterns in the same codebase

**Research date:** 2026-02-07
**Valid until:** No external dependencies; valid as long as codebase structure remains stable
