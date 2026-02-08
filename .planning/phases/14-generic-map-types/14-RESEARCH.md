# Phase 14: Generic Map Types - Research

**Researched:** 2026-02-07
**Domain:** Snow compiler internals -- making Map<K, V> generic across all compiler phases
**Confidence:** HIGH

## Summary

This phase transforms the Snow Map type from a hardcoded `Map<Int, Int>` to a generic `Map<K, V>`. The research investigated every layer of the compiler pipeline where Map is currently hardcoded: type checker (builtins + stdlib modules), MIR lowering (known function signatures), codegen (LLVM intrinsic declarations), and runtime (key comparison logic). The type infrastructure for generics (`Ty::App`, `Ty::map(K, V)`, type variables, unification) already exists and works for `List<T>`, `Option<T>`, and `Result<T, E>` -- so the main work is lifting hardcoded `Ty::int()` to type variables in Map function signatures and fixing the runtime key comparison to use content equality for string keys.

The runtime `find_key` function is the critical bottleneck: it currently compares keys using raw `u64 == u64`, which works for integers but fails for strings (which are pointers -- two identical strings at different addresses would not match). The runtime needs a type-aware key comparison mechanism or a uniform hashing approach. All values are already stored as `u64` in the runtime (integers as values, pointers as cast u64), so the storage format does not need to change -- only the key lookup logic.

Map literal syntax `%{key => value}` does not exist in the parser today. The lexer already tokenizes `%` (PERCENT) and `=>` (FAT_ARROW) and `{`/`}` (L_BRACE/R_BRACE) as separate tokens. A new parser production and MIR node (or desugaring to Map.new + Map.put chains) are needed.

**Primary recommendation:** Make Map function signatures generic using type variables and `Scheme` (polymorphic types) in both `stdlib_modules()` and `register_builtins()`. Add a `key_type` tag to the runtime map header so `find_key` can dispatch to `snow_string_eq` for string keys vs raw `==` for integer keys. Desugar map literals in MIR lowering to `snow_map_new` + `snow_map_put` chains.

## Standard Stack

This is an internal compiler change. No external libraries are involved -- all work is in the Snow compiler crates.

### Affected Crates and Files

| Crate | File | What Changes | Confidence |
|-------|------|-------------|------------|
| snow-typeck | `src/builtins.rs` | Map function signatures: `Ty::int()` -> type variables | HIGH |
| snow-typeck | `src/infer.rs` | `stdlib_modules()` Map module: same signature changes | HIGH |
| snow-typeck | `src/infer.rs` | Map literal type inference (new) | HIGH |
| snow-codegen | `src/mir/lower.rs` | `known_functions` Map entries: `MirType::Int` -> `MirType::Ptr` for string keys | HIGH |
| snow-codegen | `src/mir/lower.rs` | Map literal lowering (new parser node -> MIR) | HIGH |
| snow-codegen | `src/codegen/intrinsics.rs` | LLVM function declarations: `i64` -> `ptr` for key/value args | HIGH |
| snow-rt | `src/collections/map.rs` | `find_key`: type-aware comparison for string keys | HIGH |
| snow-parser | `src/parser/expressions.rs` | Map literal parsing: `%{key => value, ...}` | HIGH |
| snow-parser | `src/syntax_kind.rs` | New syntax kinds: `MAP_LITERAL`, `MAP_ENTRY` | HIGH |
| snow-lexer | `src/lib.rs` | May need `%{` as compound token, or handle in parser | MEDIUM |

## Architecture Patterns

### Pattern 1: Uniform u64 Value Representation (Already in Place)

**What:** All Snow values are represented as `u64` at runtime: integers as raw `i64` (reinterpreted as `u64`), strings/pointers as pointer values cast to `u64`, booleans as 0/1.

**Why it matters:** The runtime map already stores `(u64, u64)` pairs. Strings are already passed as pointers-cast-to-u64 in the JSON module (see `json.rs` line 113: `snow_map_put(snow_map, key_str as u64, val_json as u64)`). So the map storage format does not need to change at all -- only the key comparison logic.

**Evidence:** `snow-rt/src/collections/map.rs` stores `[u64; 2]` entry pairs. `snow-rt/src/json.rs` already stores string pointers as u64 keys in maps.

### Pattern 2: Polymorphic Stdlib Functions via Scheme

**What:** Use `Scheme` with quantified type variables instead of `Scheme::mono(...)` with concrete types.

**How other generics do it:** Currently, the type checker's stdlib modules use `Scheme::mono(...)` which provides monomorphic (non-generic) types. To make Map functions generic, they should use `Scheme` with quantified variables, similar to how user-defined generic functions work.

**Example for Map.put:**
```rust
// Current (hardcoded Int):
map_mod.insert("put".to_string(), Scheme::mono(Ty::fun(
    vec![map_t.clone(), Ty::int(), Ty::int()], map_t.clone())));

// Target (generic K, V):
// Create fresh vars for K, V, construct Map<K,V> type, and quantify
let k = TyVar(next_var);
let v = TyVar(next_var + 1);
let map_kv = Ty::map(Ty::Var(k), Ty::Var(v));
map_mod.insert("put".to_string(), Scheme {
    vars: vec![k, v],
    ty: Ty::fun(vec![map_kv.clone(), Ty::Var(k), Ty::Var(v)], map_kv.clone()),
});
```

**Important caveat:** The `TyVar` IDs used in Scheme must not collide with the inference context's own variable counter. The `instantiate` method on `InferCtx` creates fresh variables for each scheme variable, so the actual IDs in the Scheme definition are just placeholders. However, they must be distinct from each other within the scheme. Use high-numbered placeholder IDs (e.g., 90000+) to avoid any collision risk.

### Pattern 3: Runtime Key Type Tag

**What:** Add a `key_type` byte to the map header to signal how keys should be compared.

**Layout change:**
```
Current:  [len: u64] [cap: u64] [entries: (u64, u64)...]
Proposed: [len: u64] [cap: u64] [key_type: u8, pad: u8*7] [entries: (u64, u64)...]
-- OR --
Proposed: [len_and_tag: u64] [cap: u64] [entries: (u64, u64)...]
  where len_and_tag = (tag << 56) | len
```

**Key type tags:**
- 0 = integer keys (raw u64 == u64 comparison)
- 1 = string keys (call `snow_string_eq` for comparison)

**Alternative approach (simpler):** Instead of a tag, provide separate runtime functions:
- `snow_map_put` / `snow_map_get` for integer keys (existing)
- `snow_map_put_str` / `snow_map_get_str` for string keys (new, uses `snow_string_eq`)

The MIR lowering / codegen would dispatch to the correct variant based on the resolved key type.

**Recommendation:** Use the separate-function approach. It is simpler, avoids changing the map header layout (which would break existing code), and the codegen already has the resolved type information to choose the right function.

### Pattern 4: Map Literal Desugaring

**What:** Desugar `%{k1 => v1, k2 => v2}` to a chain of calls:
```
let __map_0 = snow_map_new()
let __map_1 = snow_map_put(__map_0, k1, v1)
let __map_2 = snow_map_put(__map_1, k2, v2)
__map_2
```

**Where to desugar:** In MIR lowering (not the parser). The parser creates a `MAP_LITERAL` CST node containing `MAP_ENTRY` children. The MIR lowerer desugars this to `snow_map_new` + `snow_map_put` calls.

**Type inference for map literals:** The type checker infers the key type K and value type V from the first entry and unifies all subsequent entries against the same types. The resulting type is `Map<K, V>`.

### Pattern 5: LLVM Intrinsic Argument Width

**What:** Currently `snow_map_put` is declared with `i64` arguments for key and value. For string keys, the key is a pointer (`ptr`). At the LLVM level, `ptr` and `i64` are both 64 bits on the target platform, but LLVM distinguishes them semantically.

**Approach:** Use the separate-function approach: `snow_map_put` keeps `i64` args for int maps, `snow_map_put_str` takes `ptr` key + `i64` value for string-key maps. The codegen dispatches based on the MIR type of the key expression.

**Alternative (type erasure):** Keep a single `snow_map_put` with `i64` args and cast string pointers to `i64` using `ptrtoint`. This is what the JSON module already does internally (`key_str as u64`). This approach is simpler but requires `ptrtoint`/`inttoptr` casts in the generated LLVM IR.

**Recommendation:** Use the type-erasure approach (single function, ptrtoint/inttoptr casts) for simplicity. It matches what JSON already does and avoids doubling the number of runtime functions. But the find_key function still needs string-aware comparison.

### Recommended Approach: Hybrid

After analyzing all options:

1. **Type system (typeck):** Make Map functions polymorphic with type variables K, V. `Map.new()` returns `Map<K, V>` with fresh vars. `Map.put(m, k, v)` unifies k with K and v with V.

2. **Runtime:** Add `snow_map_new_typed(key_type: i64)` that stores the key type tag, or add a key_type parameter to `snow_map_put`. Alternatively, use the simplest approach: always pass a comparison function pointer OR use a key_type tag in the map header.

3. **Simplest viable approach:** Add a `key_type` tag to `snow_map_new` and use it in `find_key`. The codegen passes 0 for Int keys and 1 for String keys. `find_key` checks the tag and calls `snow_string_eq` for tag=1.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| String equality comparison | Custom string compare in map.rs | `snow_string_eq` from string.rs | Already exists, tested, handles SnowString properly |
| Type variable management | Manual TyVar numbering in builtins | `InferCtx::fresh_var()` at instantiation time | Scheme::instantiate already creates fresh vars from placeholder vars |
| Map literal parsing | Complex expression parser | CST node + MIR desugaring | Follows existing patterns (list support comment in parser already planned this) |
| Polymorphic function types | Custom type propagation | `Scheme` with `vars` + `ctx.instantiate()` | Hindley-Milner infrastructure already handles this |

## Common Pitfalls

### Pitfall 1: TyVar Collision in Builtin Schemes

**What goes wrong:** If the placeholder TyVar IDs in `Scheme { vars, ty }` for Map functions overlap with TyVar IDs created by `InferCtx::fresh_var()`, the unifier may produce incorrect results.

**Why it happens:** The `InferCtx` allocates TyVar IDs sequentially starting from 0. If builtin Schemes use IDs like 0, 1, 2, they may collide with user-code variables.

**How to avoid:** Use very high placeholder IDs (e.g., `TyVar(90000)`, `TyVar(90001)`) in builtin Schemes. The `instantiate()` method replaces them with fresh variables anyway, so the actual IDs don't matter as long as they're distinct within the scheme.

**Warning signs:** Random type inference failures or incorrect unification in code that uses Map functions alongside other generic code.

### Pitfall 2: Runtime Key Comparison for String Keys

**What goes wrong:** Two identical strings (same content, different allocations) stored as map keys would not match on lookup because `find_key` compares raw `u64` values (pointer addresses).

**Why it happens:** The current `find_key` uses `(*entries.add(i))[0] == key` which does pointer-identity comparison for string keys, not content comparison.

**How to avoid:** The map must know whether its keys are integers or strings and dispatch to the appropriate comparison. Either:
- Store a key_type tag in the map header
- Use separate runtime functions for int-key and string-key maps
- Pass a comparison function pointer to find_key

**Warning signs:** `Map.get(m, "name")` returns the wrong value or 0 even though the key was just put in, because the string literal in `get` creates a different pointer than the one used in `put`.

### Pitfall 3: Forgetting to Update All Locations

**What goes wrong:** Map functions are registered in THREE places with identical Int-hardcoded signatures. Missing one causes type mismatches or runtime crashes.

**The three locations:**
1. `snow-typeck/src/infer.rs` - `stdlib_modules()` function (line ~330-340)
2. `snow-typeck/src/builtins.rs` - `register_builtins()` function (line ~316-347)
3. `snow-codegen/src/mir/lower.rs` - `known_functions` initialization (line ~271-278)

Plus:
4. `snow-codegen/src/codegen/intrinsics.rs` - LLVM function declarations (line ~247-254)

**How to avoid:** Update all four locations as a single atomic change and run the existing `e2e_map_basic` test to verify nothing breaks.

### Pitfall 4: Map.new() Type Inference Ambiguity

**What goes wrong:** `let m = Map.new()` cannot infer K and V because there are no constraints yet. The type variables remain unresolved until `Map.put(m, k, v)` is called.

**Why it happens:** `Map.new()` returns `Map<K, V>` with fresh unresolved variables. This is the same as `[]` for an empty list -- the type is inferred from context.

**How to avoid:** This is actually correct behavior. The type variables will be resolved when the map is used with `put` or in a context that constrains K and V. If the map is never constrained, the unresolved variables fall back gracefully (to `Unit` in MIR, effectively `Ptr` in codegen -- which is fine since empty maps don't need type info).

### Pitfall 5: Map Literal Parser Ambiguity with Modulo

**What goes wrong:** `%` is already the modulo operator token (PERCENT). The parser needs to disambiguate `x % {` (modulo followed by a block) from `%{` (map literal).

**How to avoid:** In the lexer or parser, check for `%{` as an atom in the `lhs()` function. The key is that `%{` (no space) is a map literal while `% {` could be ambiguous. Easiest approach: in the parser's `lhs()`, check for `PERCENT` immediately followed by `L_BRACE` (no whitespace between them). Alternatively, make `%{` a compound token in the lexer.

**Recommendation:** Handle in the parser (not lexer). When `lhs()` sees PERCENT, peek ahead for L_BRACE. If found, parse as map literal. Otherwise, fall through to existing behavior (modulo will be handled as an infix operator anyway, so this specific case won't arise in `lhs()`).

### Pitfall 6: Return Type of Map.get for Non-Int Values

**What goes wrong:** `Map.get(m, key)` currently returns `Int` (or `u64` at runtime). For string-valued maps, it should return a string pointer. But the runtime `snow_map_get` returns `u64` regardless.

**How to avoid:** At the type system level, `Map.get(m, k)` returns `V` (the value type parameter). At the MIR/codegen level, the return is always `u64`/`i64`/`ptr` depending on the resolved V type. The codegen must handle the return value appropriately:
- If V = Int, the returned u64 IS the integer value (no conversion needed)
- If V = String, the returned u64 is a pointer that should be used as `ptr`
- The codegen can use `inttoptr` if the runtime returns u64 but the expected type is ptr

**This is already how the uniform representation works.** The key insight is that `MirType::Ptr` and `MirType::Int` are both 64-bit at the LLVM level. Map functions can remain declared with i64 args/returns, with the codegen inserting casts as needed.

### Pitfall 7: Breaking Existing Map Tests

**What goes wrong:** The existing `e2e_map_basic` test uses integer keys and must continue to work after making Map generic.

**How to avoid:** Ensure the generic Map type defaults gracefully to integer behavior. The existing test `Map.put(m, 1, 10)` should infer `Map<Int, Int>` and continue to work with the same runtime functions. Run `cargo test` after every change to verify no regressions.

## Code Examples

### Example 1: Generic Map Function Signatures in stdlib_modules()

```rust
// In snow-typeck/src/infer.rs, stdlib_modules():
// Use placeholder TyVars that will be replaced by instantiate()
let k_var = TyVar(90000);
let v_var = TyVar(90001);
let k = Ty::Var(k_var);
let v = Ty::Var(v_var);
let map_kv = Ty::map(k.clone(), v.clone());

let mut map_mod = HashMap::new();

// Map.new() -> Map<K, V>  (K, V unconstrained -- inferred from usage)
map_mod.insert("new".to_string(), Scheme {
    vars: vec![k_var, v_var],
    ty: Ty::fun(vec![], map_kv.clone()),
});

// Map.put(Map<K,V>, K, V) -> Map<K,V>
map_mod.insert("put".to_string(), Scheme {
    vars: vec![k_var, v_var],
    ty: Ty::fun(vec![map_kv.clone(), k.clone(), v.clone()], map_kv.clone()),
});

// Map.get(Map<K,V>, K) -> V
map_mod.insert("get".to_string(), Scheme {
    vars: vec![k_var, v_var],
    ty: Ty::fun(vec![map_kv.clone(), k.clone()], v.clone()),
});

// Map.has_key(Map<K,V>, K) -> Bool
map_mod.insert("has_key".to_string(), Scheme {
    vars: vec![k_var, v_var],
    ty: Ty::fun(vec![map_kv.clone(), k.clone()], Ty::bool()),
});

// Map.delete(Map<K,V>, K) -> Map<K,V>
map_mod.insert("delete".to_string(), Scheme {
    vars: vec![k_var, v_var],
    ty: Ty::fun(vec![map_kv.clone(), k.clone()], map_kv.clone()),
});

// Map.size(Map<K,V>) -> Int
map_mod.insert("size".to_string(), Scheme {
    vars: vec![k_var, v_var],
    ty: Ty::fun(vec![map_kv.clone()], Ty::int()),
});

// Map.keys(Map<K,V>) -> List  (List is untyped in current impl)
map_mod.insert("keys".to_string(), Scheme {
    vars: vec![k_var, v_var],
    ty: Ty::fun(vec![map_kv.clone()], Ty::list_untyped()),
});

// Map.values(Map<K,V>) -> List
map_mod.insert("values".to_string(), Scheme {
    vars: vec![k_var, v_var],
    ty: Ty::fun(vec![map_kv.clone()], Ty::list_untyped()),
});
```

### Example 2: Runtime String-Aware Key Comparison

```rust
// In snow-rt/src/collections/map.rs:

/// Key type tags stored in the map header.
const KEY_TYPE_INT: u8 = 0;
const KEY_TYPE_STR: u8 = 1;

/// Header: [len: u64, cap_and_tag: u64, entries...]
/// cap_and_tag: lower 56 bits = cap, upper 8 bits = key_type tag
const TAG_SHIFT: u64 = 56;
const CAP_MASK: u64 = (1u64 << TAG_SHIFT) - 1;

unsafe fn map_key_type(m: *const u8) -> u8 {
    let cap_and_tag = *((m as *const u64).add(1));
    (cap_and_tag >> TAG_SHIFT) as u8
}

unsafe fn keys_equal(m: *const u8, a: u64, b: u64) -> bool {
    match map_key_type(m) {
        KEY_TYPE_STR => {
            crate::string::snow_string_eq(
                a as *const crate::string::SnowString,
                b as *const crate::string::SnowString,
            ) != 0
        }
        _ => a == b, // Integer or other value types
    }
}

unsafe fn find_key(m: *const u8, key: u64) -> Option<usize> {
    let len = map_len(m) as usize;
    let entries = map_entries(m);
    for i in 0..len {
        if keys_equal(m, (*entries.add(i))[0], key) {
            return Some(i);
        }
    }
    None
}

/// Create a map with a specific key type.
#[no_mangle]
pub extern "C" fn snow_map_new_typed(key_type: i64) -> *mut u8 {
    unsafe {
        let total = HEADER_SIZE + 0 * ENTRY_SIZE;
        let p = snow_gc_alloc(total as u64, 8);
        *(p as *mut u64) = 0; // len
        *((p as *mut u64).add(1)) = (key_type as u64) << TAG_SHIFT; // cap=0, tag=key_type
        p
    }
}
```

### Example 3: Map Literal Parser Production

```rust
// In snow-parser/src/parser/expressions.rs, inside lhs():
SyntaxKind::PERCENT => {
    // Check if followed by L_BRACE for map literal: %{k => v, ...}
    if p.nth(1) == SyntaxKind::L_BRACE {
        Some(parse_map_literal(p))
    } else {
        // Not a map literal -- modulo is an infix op, so this shouldn't
        // appear in lhs(). Emit error.
        p.error("expected expression");
        None
    }
}

// ...

/// Parse a map literal: %{key1 => value1, key2 => value2, ...}
fn parse_map_literal(p: &mut Parser) -> MarkClosed {
    let m = p.open();
    p.expect(SyntaxKind::PERCENT);   // %
    p.expect(SyntaxKind::L_BRACE);   // {

    while p.current() != SyntaxKind::R_BRACE && !p.at_end() {
        let entry = p.open();
        expr_bp(p, 0);              // key expression
        p.expect(SyntaxKind::FAT_ARROW);  // =>
        expr_bp(p, 0);              // value expression
        p.close(entry, SyntaxKind::MAP_ENTRY);

        if p.current() == SyntaxKind::COMMA {
            p.advance();            // consume optional comma
        }
    }

    p.expect(SyntaxKind::R_BRACE);  // }
    p.close(m, SyntaxKind::MAP_LITERAL)
}
```

### Example 4: Map Literal MIR Lowering

```rust
// In snow-codegen/src/mir/lower.rs:
Expr::MapLiteral(map_lit) => {
    // Desugar %{k1 => v1, k2 => v2} to:
    //   let m0 = snow_map_new_typed(key_type_tag)
    //   let m1 = snow_map_put(m0, k1, v1)
    //   let m2 = snow_map_put(m1, k2, v2)
    //   m2

    let map_ty = self.resolve_range(map_lit.syntax().text_range());
    let entries = map_lit.entries(); // Vec<(Expr, Expr)>

    // Determine key type from type info
    let key_type_tag = match &map_ty {
        // Inspect resolved type to determine runtime tag
        _ => 0, // default to int; string detection TBD
    };

    let mut result = MirExpr::Call {
        func: Box::new(MirExpr::Var("snow_map_new_typed".into(), ...)),
        args: vec![MirExpr::IntLit(key_type_tag, MirType::Int)],
        ty: MirType::Ptr,
    };

    for (key_expr, val_expr) in entries {
        let key = self.lower_expr(&key_expr);
        let val = self.lower_expr(&val_expr);
        result = MirExpr::Call {
            func: Box::new(MirExpr::Var("snow_map_put".into(), ...)),
            args: vec![result, key, val],
            ty: MirType::Ptr,
        };
    }

    result
}
```

### Example 5: Codegen Key Type Dispatch

```rust
// In codegen, when lowering Map.new():
// Determine key type from the resolved Map<K,V> type
// and pass the appropriate tag to snow_map_new_typed

// When the MIR expr calls snow_map_put with a string key,
// the codegen already handles ptr arguments correctly --
// strings are already ptr type in LLVM. The key needs to be
// passed as i64 (ptrtoint cast) because the runtime stores u64.
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| `Map<Int, Int>` hardcoded | `Map<K, V>` generic (this phase) | Phase 14 | String keys, mixed-type maps |
| `Scheme::mono(...)` for Map fns | `Scheme { vars, ty }` polymorphic | Phase 14 | HM inference resolves K, V |
| Raw `u64 == u64` key comparison | Type-aware key comparison | Phase 14 | String keys work correctly |
| No map literal syntax | `%{k => v}` syntax | Phase 14 | Expressive map construction |

## Open Questions

1. **Map.keys() and Map.values() return types**
   - What we know: Currently returns `List` (untyped). Ideally `Map.keys()` returns `List<K>` and `Map.values()` returns `List<V>`.
   - What's unclear: List is also not fully generic in stdlib (List module functions are monomorphic Int-based). Making Map.keys() return `List<K>` would require List generics too.
   - Recommendation: Keep returning `List` (untyped) for now. Genericizing List is out of scope for Phase 14. Document as a future improvement.

2. **Mixed key types**
   - What we know: The type system prevents `Map.put(m, 1, "a")` then `Map.put(m, "x", "b")` because K would need to unify with both Int and String.
   - What's unclear: Whether there's a use case for maps with heterogeneous key types.
   - Recommendation: Not needed. The type system naturally prevents this, which is the correct behavior.

3. **Map.get return for missing keys**
   - What we know: Currently returns 0 for missing integer keys. For string-valued maps, returning 0 (null pointer) would be problematic.
   - What's unclear: Whether to change the return type to `Option<V>` or keep the current behavior.
   - Recommendation: Keep current behavior (return 0/null) for Phase 14. Changing to `Option<V>` is a breaking change best addressed separately. The `Map.has_key` function exists for checking membership.

4. **Key types beyond Int and String**
   - What we know: The generic type system allows `Map<Bool, String>` or `Map<Float, Int>`, but the runtime only distinguishes Int and String keys.
   - Recommendation: For Phase 14, support Int and String keys. Other key types can use integer comparison (Bool is 0/1, works fine). Float keys are uncommon and can be deferred.

5. **Performance of linear scan with string comparison**
   - What we know: Current map uses linear scan O(n). String comparison adds overhead per entry.
   - Recommendation: Linear scan is fine for small maps (the current design). Hash-based map is a future optimization, not Phase 14 scope.

## Sources

### Primary (HIGH confidence)
- Snow compiler source code (direct reading): `snow-typeck/src/ty.rs`, `snow-typeck/src/builtins.rs`, `snow-typeck/src/infer.rs`, `snow-codegen/src/mir/lower.rs`, `snow-codegen/src/codegen/intrinsics.rs`, `snow-codegen/src/codegen/expr.rs`, `snow-rt/src/collections/map.rs`, `snow-rt/src/string.rs`, `snow-rt/src/json.rs`, `snow-parser/src/parser/expressions.rs`

### Verification
- Existing working patterns: `Ty::map()`, `Ty::map_untyped()`, `Ty::App` for generic types, `Scheme` with `vars` for polymorphic functions, `snow_string_eq` for string comparison
- Existing e2e test: `tests/e2e/stdlib_map_basic.snow` and `e2e_map_basic` test in `snowc/tests/e2e_stdlib.rs`
- JSON module evidence: `snow-rt/src/json.rs` already stores string pointers as u64 map keys (line 113)

## Metadata

**Confidence breakdown:**
- Type system changes: HIGH - Direct source code reading, clear pattern from existing generics
- Runtime changes: HIGH - Direct source code reading, clear problem (find_key raw u64 comparison)
- Parser changes: HIGH - Lexer already has needed tokens, parser structure is clear
- MIR lowering: HIGH - Clear desugaring pattern, matches existing collection handling
- Pitfalls: HIGH - All derived from direct code analysis

**Research date:** 2026-02-07
**Valid until:** Indefinite (internal compiler code, no external dependencies)
