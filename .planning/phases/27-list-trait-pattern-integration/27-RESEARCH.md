# Phase 27: List Trait & Pattern Integration - Research

**Researched:** 2026-02-08
**Domain:** Compiler internals -- trait dispatch for collections, pattern matching, MIR lowering, runtime
**Confidence:** HIGH

## Summary

Phase 27 makes the trait protocols (Display, Debug, Eq, Ord) and pattern matching (`head :: tail` destructuring) work correctly with the polymorphic `List<T>` type that was built in Phase 26. The work spans three distinct areas:

1. **Display/Debug (LIST-06):** Display for lists via string interpolation already works for `List<Int>` (tested in e2e). The remaining work is ensuring `to_string([1, 2, 3])` works as a direct function call, `to_string(["a", "b"])` resolves the correct element-type callback for String elements, and `debug(my_struct_list)` renders each element via Debug. The infrastructure for this is largely in place -- the `wrap_collection_to_string` and `resolve_to_string_callback` systems already handle `Ty::App(Con("List"), [inner_ty])` with recursive callback generation.

2. **Eq/Ord (LIST-07):** Currently, `[1, 2] == [1, 2]` would fail because the binary operator dispatch (line 3176 of `lower.rs`) only checks for `MirType::Struct(_) | MirType::SumType(_)`, and lists resolve to `MirType::Ptr`. New runtime functions (`snow_list_eq`, `snow_list_compare`) are needed, plus MIR lowering changes to dispatch `==`/`>` on `MirType::Ptr` (when the typeck type is a List) to these runtime functions with element-comparison callback function pointers (following the same pattern as `snow_list_to_string`).

3. **Pattern Matching (LIST-08):** The `head :: tail` cons pattern destructuring does NOT currently exist in the compiler. There is no `CONS_PAT` syntax kind, no parser support for `::` in pattern position, and no MIR pattern variant for list destructuring. This is the most significant new feature: it requires parser changes (new syntax kind + parsing rules), typeck changes (type inference for cons patterns), MIR changes (new `MirPattern::ListCons` variant), pattern compilation changes (decision tree support), and codegen changes (emit `snow_list_head`/`snow_list_tail` calls).

**Primary recommendation:** Implement in three stages: (1) Display/Debug for polymorphic lists (smallest change, leverages existing infrastructure), (2) Eq/Ord with runtime callback functions (new runtime functions + MIR dispatch), (3) `head :: tail` pattern matching (most significant change, touches parser through codegen).

## Standard Stack

This phase is entirely within the Snow compiler codebase. No new external libraries are needed.

### Core Components (by crate)

| Crate | Files to Modify | Purpose |
|-------|----------------|---------|
| `snow-rt` | `src/collections/list.rs` | Add `snow_list_eq`, `snow_list_compare` runtime functions |
| `snow-parser` | `src/syntax_kind.rs`, `src/parser/patterns.rs`, `src/ast/pat.rs` | Add `CONS_PAT` syntax kind, parse `head :: tail` pattern |
| `snow-typeck` | `src/infer.rs`, `src/exhaustiveness.rs` | Type inference for cons patterns, exhaustiveness checking |
| `snow-codegen` | `src/mir/mod.rs`, `src/mir/lower.rs`, `src/codegen/expr.rs`, `src/codegen/intrinsics.rs`, `src/pattern/compile.rs`, `src/codegen/pattern.rs` | MIR ListCons pattern, trait dispatch for Ptr types, runtime declarations |

## Architecture Patterns

### Pattern 1: Callback-Based Collection Trait Functions

**What:** Collection traits use a runtime function that takes element-level callback function pointers.
**When to use:** Any trait on `List<T>` where behavior depends on the element type `T`.
**Why:** Lists use type-erased uniform u64 storage at runtime. The element type is only known at compile time. Passing a callback lets the runtime iterate elements while the compiler provides type-specific behavior.

**Existing precedent (Display):**
```
snow_list_to_string(list: ptr, elem_to_str: fn(u64) -> ptr) -> ptr
```

**New functions needed (Eq/Ord):**
```
snow_list_eq(list_a: ptr, list_b: ptr, elem_eq: fn(u64, u64) -> i8) -> i8
snow_list_compare(list_a: ptr, list_b: ptr, elem_cmp: fn(u64, u64) -> i64) -> i64
```

The `elem_eq` callback returns 1 (equal) or 0 (not equal).
The `elem_cmp` callback returns -1/0/1 (less/equal/greater).

### Pattern 2: MIR Lowering for Operator Dispatch on Ptr Types

**What:** The binary operator lowering in `lower_binary_expr` (line 3172-3228) currently only dispatches trait methods for `MirType::Struct(_) | MirType::SumType(_)`. For lists (which are `MirType::Ptr`), the lowering must consult the typeck type to determine if the operands are lists, and if so, emit the appropriate runtime call with callbacks.

**How to implement:** When `lhs_ty` is `MirType::Ptr` and the operation is `Eq`/`NotEq`/`Lt`/`Gt`/`LtEq`/`GtEq`, look up the typeck type for the LHS expression. If it resolves to `Ty::App(Con("List"), [elem_ty])`, emit a call to `snow_list_eq` or `snow_list_compare` with the appropriate element comparison callback.

**Element comparison callback resolution** follows the same recursive pattern as `resolve_to_string_callback`:
- Int: use a simple `i64 ==` inline or small builtin
- Float: use `f64 ==` with bitcast
- String: use `snow_string_eq`
- Bool: use `i8 ==`
- Struct: use `Eq__eq__StructName`
- Nested List: generate a synthetic wrapper function (recursive)

### Pattern 3: Cons Pattern Desugaring to Runtime Calls

**What:** `head :: tail` in a case expression desugars to: check `snow_list_length(list) > 0`, then bind `head = snow_list_head(list)` and `tail = snow_list_tail(list)`.

**How it flows through the compiler:**

1. **Parser:** `head :: tail` is parsed as `CONS_PAT` syntax node containing two sub-patterns separated by `COLON_COLON`
2. **Typeck:** A cons pattern on `List<T>` infers the head as type `T` and the tail as `List<T>`
3. **MIR lowering:** `lower_pattern` produces `MirPattern::ListCons { head: MirPattern, tail: MirPattern }`
4. **Pattern compilation:** `compile_matrix` treats `ListCons` like a constructor with arity 2. The access paths for head and tail use `snow_list_head` and `snow_list_tail` runtime calls
5. **Codegen:** The decision tree codegen emits the runtime calls for head/tail access

**Important:** The existing `COLON_COLON` token (`::`) is already used for type annotations in Snow (`x :: Int`). The parser must distinguish between:
- Type annotation context: `param :: Type` (after parameter name, before type)
- Pattern context: `head :: tail` (in case arm patterns)

This is disambiguated by context: `::` in a pattern position (inside a `case ... do ... end` match arm, before `->`) is a cons pattern, while `::` after a parameter name in a function definition is a type annotation.

### Pattern 4: Existing Trait Auto-Derive System

**What:** The auto-derive system in `lower_struct_def` (line 1339-1365) generates synthetic MIR functions like `Debug__inspect__Point`, `Eq__eq__Point`, `Ord__lt__Point` for structs.

**Relevance to Phase 27:** List is a runtime primitive (not a user-defined struct), so auto-derive does NOT apply. Instead, the trait functions for List must be:
- **Display:** Already handled by `wrap_collection_to_string` + `snow_list_to_string` (runtime function with callback)
- **Debug:** Can reuse the Display format `[elem1, elem2, ...]` since lists don't have a distinct Debug format
- **Eq:** New runtime function `snow_list_eq` with element equality callback
- **Ord:** New runtime function `snow_list_compare` with element comparison callback

### Anti-Patterns to Avoid

- **Don't generate MIR-level `Eq__eq__List` or `Ord__lt__List` functions:** Lists are not MIR structs. Their equality requires iterating elements, which is done at runtime. The MIR lowering should directly emit calls to `snow_list_eq`/`snow_list_compare` with callbacks, not try to create synthetic MIR functions.

- **Don't try to make `::` cons patterns work everywhere at once:** Start with case expression match arms only (the success criterion). Don't add cons patterns to function parameter patterns, let bindings, or receive patterns in this phase.

- **Don't add Display/Debug/Eq/Ord trait registrations for List in the TraitRegistry:** The trait dispatch for lists works differently than for structs (callback-based runtime functions). Adding `impl Display for List` would cause the generic trait dispatch to try to find a `Display__to_string__List` function, which doesn't exist. Instead, list trait operations are handled by special-case code in the lowerer.

- **Don't change `MirType` for lists:** Lists must remain `MirType::Ptr`. Adding a `MirType::List(inner_type)` would break the uniform representation and require changes everywhere Ptr is handled.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| List element comparison | Custom comparison loop in MIR | Runtime `snow_list_eq`/`snow_list_compare` with callbacks | Runtime already iterates elements; callback pattern established by `snow_list_to_string` |
| Element comparison callbacks | Per-type specialized list comparison | Reuse `resolve_to_string_callback` pattern for comparison callbacks | Same recursive resolution logic, different leaf functions |
| Pattern matching on lists | Custom if-else chains in MIR | `MirPattern::ListCons` + decision tree compilation | Integrates with existing exhaustiveness checking and decision tree codegen |
| Empty list detection in patterns | Runtime tag checks | `snow_list_length(list) == 0` check | Uniform representation -- no tag byte on lists |

## Common Pitfalls

### Pitfall 1: `::` Token Ambiguity Between Type Annotation and Cons Pattern

**What goes wrong:** The `::` (COLON_COLON) token is already heavily used in Snow for type annotations (`param :: Type`). Adding it as a cons pattern operator risks parsing ambiguity.
**Why it happens:** Same token, different contexts.
**How to avoid:** Cons patterns only appear in case arm pattern positions. The parser already distinguishes pattern position (after `case ... do`) from expression/parameter position. In `parse_pattern` (patterns.rs), after parsing a primary pattern (like an identifier), check if the next token is `COLON_COLON`. If so, parse the RHS as another pattern and wrap both in `CONS_PAT`. This is similar to how binary expressions work (left + right), but in pattern space.
**Warning signs:** `fn foo(x :: Int)` starts being parsed as a cons pattern instead of a type annotation. Solution: cons patterns are ONLY parsed inside `parse_pattern`, which is NOT called for regular function parameters.

### Pitfall 2: Type Conversion for Comparison Callbacks

**What goes wrong:** When comparing `List<Bool>` elements, the callback receives `u64` values but Bool is `i8`. Without truncation, `true` stored as `0x0000000000000001` compares correctly, but other values might not.
**Why it happens:** Uniform u64 storage means all types need conversion.
**How to avoid:** The element comparison callbacks must handle the same type conversions as `resolve_to_string_callback`. For Int, direct `i64` comparison. For Bool, truncate to `i8` then compare. For Float, bitcast to `f64` then compare. For String, call `snow_string_eq`. For Struct, call `Eq__eq__StructName`.
**Warning signs:** `[true, false] == [true, false]` returns false because the callback compares raw u64 bits.

### Pitfall 3: Cons Pattern Requires Non-Empty Check

**What goes wrong:** `head :: tail` on an empty list causes a runtime panic from `snow_list_head`.
**Why it happens:** The pattern implicitly assumes the list is non-empty.
**How to avoid:** In the decision tree compilation, the `ListCons` pattern must first check `snow_list_length(list) > 0` before extracting head/tail. The empty case should fall through to the next arm (e.g., `[] -> ...`). This is analogous to how constructor patterns first check the tag.
**Warning signs:** Case expression panics on empty list instead of matching the wildcard/empty arm.

### Pitfall 4: Callback Comparison Functions Need Correct Signatures

**What goes wrong:** The runtime `snow_list_eq` expects callbacks with signature `fn(u64, u64) -> i8`, but MIR generates function pointers with the wrong type.
**Why it happens:** MIR function types don't perfectly match the C ABI expected by the runtime.
**How to avoid:** The comparison callback functions should be generated as synthetic MIR functions that take `(Ptr, Ptr)` parameters and return `Bool` or `Int`, matching the runtime's `extern "C"` signature. The same pattern as `generate_display_collection_wrapper` should be followed.
**Warning signs:** LLVM type mismatch errors or segfaults when calling the callback.

### Pitfall 5: Lexicographic Ordering for List Comparison

**What goes wrong:** List comparison needs lexicographic semantics: `[1, 3] > [1, 2]` is true because the first differing element (3 > 2) determines the result. A simpler "compare lengths" approach would be wrong.
**Why it happens:** The success criterion explicitly requires lexicographic comparison.
**How to avoid:** The runtime `snow_list_compare` must implement lexicographic comparison: iterate elements pairwise, compare each with the callback, return the first non-zero result. If all compared elements are equal, compare lengths.
**Warning signs:** `[1, 2, 3] > [1, 2]` returns false (should be true because longer list wins when prefix matches).

### Pitfall 6: Cons Pattern Type Inference with Polymorphic Lists

**What goes wrong:** When pattern-matching `head :: tail` on a `List<MyStruct>`, the `head` binding must have type `MyStruct` and `tail` must have type `List<MyStruct>`. If the typeck doesn't extract the element type from the list's polymorphic type, both bindings get incorrect types.
**Why it happens:** The typeck needs to resolve `List<T>` to extract `T` for the head binding.
**How to avoid:** In `infer_pattern` for cons patterns: look up the scrutinee type. If it's `Ty::App(Con("List"), [elem_ty])`, assign `elem_ty` to the head pattern and `Ty::list(elem_ty)` to the tail pattern. Unify the scrutinee with `Ty::list(fresh_var)` to handle inference in both directions.
**Warning signs:** `head` binding has type `Ptr` instead of `MyStruct`, causing field access to fail.

## Code Examples

### Runtime: snow_list_eq (snow-rt/src/collections/list.rs)

```rust
/// Compare two lists for equality using an element comparison callback.
/// Returns 1 (equal) or 0 (not equal).
/// `elem_eq` is `fn(u64, u64) -> i8` returning 1 if elements are equal.
#[no_mangle]
pub extern "C" fn snow_list_eq(
    a: *mut u8,
    b: *mut u8,
    elem_eq: *mut u8,
) -> i8 {
    type ElemEq = unsafe extern "C" fn(u64, u64) -> i8;
    unsafe {
        let len_a = list_len(a);
        let len_b = list_len(b);
        if len_a != len_b {
            return 0;
        }
        let data_a = list_data(a);
        let data_b = list_data(b);
        let f: ElemEq = std::mem::transmute(elem_eq);
        for i in 0..len_a as usize {
            if f(*data_a.add(i), *data_b.add(i)) == 0 {
                return 0;
            }
        }
        1
    }
}
```

### Runtime: snow_list_compare (snow-rt/src/collections/list.rs)

```rust
/// Lexicographic comparison of two lists.
/// Returns -1 (a < b), 0 (a == b), or 1 (a > b).
/// `elem_cmp` is `fn(u64, u64) -> i64` returning -1/0/1.
#[no_mangle]
pub extern "C" fn snow_list_compare(
    a: *mut u8,
    b: *mut u8,
    elem_cmp: *mut u8,
) -> i64 {
    type ElemCmp = unsafe extern "C" fn(u64, u64) -> i64;
    unsafe {
        let len_a = list_len(a);
        let len_b = list_len(b);
        let min_len = std::cmp::min(len_a, len_b);
        let data_a = list_data(a);
        let data_b = list_data(b);
        let f: ElemCmp = std::mem::transmute(elem_cmp);
        for i in 0..min_len as usize {
            let cmp = f(*data_a.add(i), *data_b.add(i));
            if cmp != 0 {
                return cmp;
            }
        }
        // All compared elements equal -- compare lengths
        if len_a < len_b { -1 }
        else if len_a > len_b { 1 }
        else { 0 }
    }
}
```

### MIR: ListCons Pattern Variant (mir/mod.rs)

```rust
/// MIR pattern for match expressions.
pub enum MirPattern {
    // ... existing variants ...
    /// List cons pattern: matches non-empty list, binds head and tail.
    ListCons {
        head: Box<MirPattern>,
        tail: Box<MirPattern>,
    },
}
```

### Parser: Cons Pattern Parsing (parser/patterns.rs)

After parsing a primary pattern (e.g., an identifier), check for `COLON_COLON` to form a cons pattern:

```rust
// In parse_pattern, after parsing primary_pattern:
// Check for :: (cons pattern)
if p.at(SyntaxKind::COLON_COLON) {
    let cons_m = p.open_before(m); // re-wrap the already-parsed LHS
    p.advance(); // consume ::
    parse_primary_pattern(p); // parse the tail pattern
    return Some(p.close(cons_m, SyntaxKind::CONS_PAT));
}
```

### MIR Lowering: Eq Dispatch for Lists (mir/lower.rs)

In `lower_binary_expr`, add a Ptr-type dispatch path:

```rust
// After the existing Struct/SumType dispatch (line 3228):
if matches!(lhs_ty, MirType::Ptr) && matches!(op, BinOp::Eq | BinOp::NotEq | BinOp::Lt | ...) {
    // Look up typeck type for LHS
    if let Some(typeck_ty) = self.get_ty(bin.lhs().unwrap().syntax().text_range()) {
        if let Ty::App(con, args) = typeck_ty {
            if let Ty::Con(c) = con.as_ref() {
                if c.name == "List" {
                    // Emit snow_list_eq or snow_list_compare with callback
                    let elem_ty = args.first().cloned().unwrap_or(Ty::int());
                    let callback = self.resolve_eq_callback(&elem_ty);
                    // ... build MirExpr::Call to snow_list_eq/snow_list_compare
                }
            }
        }
    }
}
```

### MIR Lowering: Element Comparison Callback Resolution

```rust
fn resolve_eq_callback(&mut self, elem_ty: &Ty) -> String {
    match elem_ty {
        Ty::Con(con) => match con.name.as_str() {
            "Int" => "__snow_int_eq".to_string(),      // synthetic: fn(u64, u64) -> i8
            "Float" => "__snow_float_eq".to_string(),
            "Bool" => "__snow_bool_eq".to_string(),
            "String" => "__snow_string_eq_cb".to_string(), // wrapper around snow_string_eq
            name => format!("__eq_cb_{}", name),       // wrapper around Eq__eq__Name
        },
        Ty::App(con, args) => {
            if let Ty::Con(c) = con.as_ref() {
                if c.name == "List" {
                    // Recursive: generate wrapper that calls snow_list_eq with inner callback
                    let inner = args.first().cloned().unwrap_or(Ty::int());
                    self.generate_list_eq_wrapper(&inner)
                } else { ... }
            }
        }
        _ => "__snow_int_eq".to_string()
    }
}
```

### Pattern Compilation: ListCons in Decision Tree

The `ListCons` pattern in the decision tree compilation acts like a constructor with:
- Tag: "non-empty list" (checked via `snow_list_length(list) > 0`)
- Arity: 2 (head and tail)
- Access paths: `AccessPath::ListHead(base)` and `AccessPath::ListTail(base)`

The codegen for these access paths emits:
```
head_val = call snow_list_head(list)
tail_val = call snow_list_tail(list)
```

With type-appropriate conversion on the head value (same as list_get return conversion -- Bool truncation, Float bitcast, etc.).

## State of the Art

| Current State (Post Phase 26) | Phase 27 Target | Impact |
|-------------------------------|-----------------|--------|
| `to_string([1, 2, 3])` works via collection Display dispatch | Ensure it works for `List<String>`, `List<MyStruct>` too | Verify and fix callback resolution for non-Int element types |
| `debug(list)` for lists does not exist | `debug(my_struct_list)` renders `[Elem1, Elem2, ...]` | Reuse Display format or add Debug-specific rendering |
| `[1, 2] == [1, 2]` fails (Ptr binop unsupported) | Returns `true` via `snow_list_eq` | New runtime function + MIR dispatch |
| `[1, 3] > [1, 2]` fails | Returns `true` via lexicographic `snow_list_compare` | New runtime function + MIR dispatch |
| No cons pattern syntax | `head :: tail` destructuring in case expressions | New parser syntax, typeck, MIR, pattern compiler, codegen |
| Pattern matching works for sum types and literals | Also works for list head/tail destructuring | Extends decision tree compiler |

## Open Questions

1. **Should `[]` be a valid pattern for matching empty lists?**
   - What we know: The success criteria require `head :: tail` destructuring. For complete pattern matching, users need a way to match the empty case too.
   - What's unclear: Should this be `[] -> ...` (list literal pattern), `_ -> ...` (wildcard), or some other syntax?
   - Recommendation: Support `[]` as a pattern for empty lists. This is the natural complement to `head :: tail`. In the decision tree, `[]` is the "default case" (length == 0) when `head :: tail` is present. If not implemented here, users can use `_ -> ...` with a runtime guard, but `[]` is more idiomatic.

2. **How should Debug differ from Display for lists?**
   - What we know: For structs, Debug produces `"Point { x: 1, y: 2 }"` while Display produces the user-defined `to_string` result. For primitive types, Debug wraps strings in quotes while Display does not.
   - What's unclear: Whether `debug(["hello"])` should produce `["hello"]` (Display-like) or `[\"hello\"]` (Debug-like with quoted strings).
   - Recommendation: For lists, Debug should render each element via its Debug impl (e.g., strings quoted: `["hello", "world"]`). This means `snow_list_to_string` with the Debug callback (`Debug__inspect__String` wraps in quotes). However, the success criterion says "renders each element using its Debug implementation" so use Debug callbacks. If this is too complex for the first pass, Display callbacks are acceptable since the format `[elem1, elem2]` is the same.

3. **Should nested cons patterns be supported (e.g., `a :: b :: rest`)?**
   - What we know: The success criteria only show `head :: tail`.
   - Recommendation: Design the parser to be right-associative (`a :: (b :: rest)`), which naturally supports nesting. But don't add explicit tests for nested patterns in this phase -- just ensure the grammar is right-associative so it works naturally if users try it.

## Sources

### Primary (HIGH confidence)
- Codebase analysis: `snow-codegen/src/mir/lower.rs` -- binary operator dispatch (line 3172-3228), collection Display dispatch (line 3408-3425), `wrap_to_string` (line 4453-4577), `resolve_to_string_callback` (line 4676-4753), `generate_display_collection_wrapper` (line 4796-4849)
- Codebase analysis: `snow-codegen/src/mir/mod.rs` -- MirPattern enum (line 374-393), MirExpr variants
- Codebase analysis: `snow-codegen/src/pattern/compile.rs` -- decision tree compiler (Maranget's algorithm), HeadCtor enum (line 48-59)
- Codebase analysis: `snow-codegen/src/codegen/pattern.rs` -- decision tree codegen
- Codebase analysis: `snow-codegen/src/codegen/expr.rs` -- codegen_binop (line 217-255), list concat dispatch (line 237-241)
- Codebase analysis: `snow-rt/src/collections/list.rs` -- runtime list operations, `snow_list_to_string` callback pattern (line 298-332)
- Codebase analysis: `snow-parser/src/syntax_kind.rs` -- COLON_COLON token (line 405), CONS_PAT does not exist
- Codebase analysis: `snow-parser/src/parser/patterns.rs` -- pattern parsing, no cons pattern support
- Codebase analysis: `snow-typeck/src/traits.rs` -- TraitRegistry, has_impl, find_method_traits
- Phase 26 research and summaries -- Phase 26 established polymorphic List<T>, ListLit MIR, uniform u64 storage

### Secondary (MEDIUM confidence)
- Codebase analysis: `snow-typeck/src/exhaustiveness.rs` -- exhaustiveness checking for patterns (may need List cons support)
- Codebase analysis: `snow-codegen/src/codegen/intrinsics.rs` -- existing runtime function declarations

## Metadata

**Confidence breakdown:**
- Display/Debug (LIST-06): HIGH -- infrastructure exists, mostly verification + minor callback fixes
- Eq/Ord (LIST-07): HIGH -- clear pattern from snow_list_to_string callback, straightforward runtime functions
- Pattern matching (LIST-08): HIGH -- parser/MIR/codegen changes well-understood from existing constructor pattern infrastructure
- Pitfalls: HIGH -- identified from actual code analysis, each one validated against specific code paths

**Research date:** 2026-02-08
**Valid until:** 2026-03-08 (stable internal codebase)
