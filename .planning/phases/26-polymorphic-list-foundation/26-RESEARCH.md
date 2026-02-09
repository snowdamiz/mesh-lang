# Phase 26: Polymorphic List Foundation - Research

**Researched:** 2026-02-08
**Domain:** Compiler internals -- type system, MIR lowering, runtime, parser
**Confidence:** HIGH

## Summary

Phase 26 transforms the Snow List type from a monomorphic `List<Int>` to a fully polymorphic `List<T>` supporting arbitrary element types. This requires changes across 5 compiler crates (parser, typeck, codegen/MIR, codegen/LLVM, runtime) but critically, **the runtime requires zero functional changes** -- the existing `snow_list_*` functions already operate on uniform `u64` values, which is the universal representation for all Snow types (Int, Bool, String pointers, struct pointers, list pointers).

The primary work is in:
1. **Parser**: Add list literal syntax `[expr, expr, ...]` (the comment "L_BRACKET for list literals could go here in the future" at `expressions.rs:235` explicitly anticipates this)
2. **Type checker**: Make `List.*` functions polymorphic instead of monomorphic `Int`, infer element types from list literals
3. **MIR lowering**: Map polymorphic list operations to the same runtime functions (all values are `u64`/`Ptr` at LLVM level), lower list literals to `snow_list_from_array` calls
4. **Display**: Leverage the existing nested collection Display infrastructure (already handles `Ty::App(Con("List"), [inner_ty])`)

**Primary recommendation:** Implement changes layer by layer: parser first (list literal syntax), then typeck (polymorphic signatures), then MIR lowering (literal lowering + polymorphic dispatch), with backward compatibility verified at each step.

## Architecture Patterns

### Current Architecture: How List Works Today

**Runtime (`snow-rt/src/collections/list.rs`):**
- Layout: `{ len: u64, cap: u64, data: [u64; cap] }` -- all elements stored as uniform 8-byte `u64` values
- All operations (`append`, `get`, `head`, `tail`, `map`, `filter`, `reduce`, `concat`, `reverse`, `from_array`, `to_string`) operate on raw `u64` values
- `to_string` accepts an `elem_to_str: fn(u64) -> *mut u8` callback -- **already polymorphic at runtime**
- GC allocation via `snow_gc_alloc_actor` -- returns opaque pointer, no type metadata in header
- Key insight: **All Snow values fit in 8 bytes** -- Int is i64, Float is f64, Bool is i8 (zero-extended), String/Struct/List/etc. are GC pointers (all pointer-sized)

**Type system (`snow-typeck/src/ty.rs`):**
- `Ty::list(inner)` creates `Ty::App(Box::new(Ty::Con(TyCon::new("List"))), vec![inner])` -- the parametric form already exists
- `Ty::list_untyped()` creates bare `Ty::Con(TyCon::new("List"))` -- the opaque form used for current monomorphic sigs

**Type checker (`snow-typeck/src/builtins.rs` + `infer.rs`):**
- All list functions are registered with hardcoded `Ty::int()` element type:
  - `list_append`: `(List, Int) -> List` (should be `(List<T>, T) -> List<T>`)
  - `list_get`: `(List, Int) -> Int` (should be `(List<T>, Int) -> T`)
  - `list_head`: `(List) -> Int` (should be `(List<T>) -> T`)
  - `list_map`: `(List, (Int) -> Int) -> List` (should be `(List<T>, (T) -> U) -> List<U>`)
  - `list_filter`: `(List, (Int) -> Bool) -> List` (should be `(List<T>, (T) -> Bool) -> List<T>`)
  - `list_reduce`: `(List, Int, (Int, Int) -> Int) -> Int` (should be `(List<T>, U, (U, T) -> U) -> U`)
- Registered as `Scheme::mono(...)` -- no polymorphism
- Module-qualified access (`List.append`, `List.get`, etc.) is resolved through `modules` HashMap in `infer.rs`

**MIR lowering (`snow-codegen/src/mir/lower.rs`):**
- `resolve_app()` in `types.rs` maps `List<T>` to `MirType::Ptr` regardless of `T` -- **already correct for polymorphic use**
- Known functions map list operations to `snow_list_*` runtime calls with `MirType::Ptr` and `MirType::Int`
- Display infrastructure (`wrap_collection_to_string`, `resolve_to_string_callback`) already handles `Ty::App(Con("List"), [inner_ty])` with recursive callback generation

**Parser (`snow-parser/src/parser/expressions.rs`):**
- No list literal syntax exists -- comment at line 235: "L_BRACKET for list literals could go here in the future"
- `L_BRACKET` is already a lexer token (`TokenKind::LBracket` -> `SyntaxKind::L_BRACKET`)
- Index access `expr[index]` is already parsed as `INDEX_EXPR` using `L_BRACKET`/`R_BRACKET`
- Index expressions are NOT lowered in MIR (returns `MirExpr::Unit`)

**LLVM codegen (`snow-codegen/src/codegen/intrinsics.rs`):**
- `snow_list_append` declared as `ptr_type.fn_type(&[ptr_type.into(), i64_type.into()])` -- takes i64 element
- `snow_list_get` returns `i64` -- caller must bitcast for non-Int types
- `snow_list_from_array` already declared but never called from codegen

### Pattern: How Map (Already Polymorphic) Differs from List

The `Map<K, V>` type shows the pattern for polymorphic collection registration:

```rust
// Map: polymorphic with type variables (from builtins.rs)
let k_var = TyVar(90000);
let v_var = TyVar(90001);
let k = Ty::Var(k_var);
let v = Ty::Var(v_var);
let map_kv = Ty::map(k.clone(), v.clone());
env.insert("map_put".into(), Scheme { vars: vec![k_var, v_var], ty: Ty::fun(vec![map_kv.clone(), k.clone(), v.clone()], map_kv.clone()) });

// List: monomorphic with hardcoded Int (current, needs change)
env.insert("list_append".into(), Scheme::mono(Ty::fun(vec![list_t.clone(), Ty::int()], list_t.clone())));
```

The fix is to follow the Map pattern: use `TyVar` for element type, create `Scheme { vars: [...], ty: ... }`.

### Pattern: Monomorphization for Generic Structs

The existing monomorphization system (`ensure_monomorphized_struct_trait_fns`) shows how generic types are handled:
1. At struct literal site, detect generic instantiation from `Ty::App(Con(name), args)`
2. Build mangled name: `mangle_type_name(base, args)` -> e.g., `"Box_Int"`
3. Substitute generic params in field types
4. Generate trait functions with mangled name
5. Push `MirStructDef` with mangled name

For List: **no struct def needed** (List is a runtime primitive, not a user-defined struct). The monomorphization concern is limited to:
- Correct type inference (element type flows through)
- Correct MIR type for return values (e.g., `list_get` returns `MirType::Int` for `List<Int>`, `MirType::Ptr` for `List<String>`)
- Display callbacks matching element type

### Recommended Approach for Each Requirement

**LIST-01: `[1, 2, 3]` as `List<Int>` (backward compatibility)**

1. Add `LIST_LITERAL` syntax kind to parser
2. Parse `[expr, expr, ...]` as `LIST_LITERAL` with child expressions
3. In typeck: infer element type by unifying all element types, produce `Ty::list(elem_ty)`
4. In MIR lowering: lower to `snow_list_from_array` call (already declared in intrinsics)
5. All existing `List.new()` + `List.append()` code continues to work unchanged

**LIST-02 through LIST-04: `List<String>`, `List<Bool>`, `List<MyStruct>`**

1. Change list function registrations from `Scheme::mono` to polymorphic `Scheme { vars, ty }`
2. In MIR lowering, when resolving list function calls:
   - `list_append(list, elem)`: element is already passed as u64/ptr at LLVM level -- no change needed
   - `list_get(list, idx)`: return type in MIR must reflect the element type (not always `MirType::Int`)
   - `list_head(list)`: same as get
   - `list_map(list, fn)`: closure return type determines new list element type
3. Key insight: **Bool values are i8 at LLVM level but stored as u64 in lists** -- zero-extension on store, truncation on load. This is already how the runtime works. The codegen must handle the i8 <-> i64 conversion.
4. For strings and structs, values are already pointers (stored as u64). No conversion needed.

**LIST-05: Nested lists `List<List<Int>>`**

1. With polymorphic type inference, `[[1, 2], [3, 4]]` infers as `List<List<Int>>` automatically
2. Runtime: inner lists are pointers (u64) stored in outer list -- works with current runtime
3. Display: The `resolve_to_string_callback` already generates synthetic wrapper functions for `Ty::App(Con("List"), [Ty::App(Con("List"), [Ty::int()])])` -- this code path was built in v1.4 but couldn't be tested

### Recommended Project Structure (changes by crate)

```
crates/
├── snow-parser/
│   ├── src/syntax_kind.rs         # Add LIST_LITERAL SyntaxKind
│   ├── src/parser/expressions.rs  # Parse [expr, ...] at L_BRACKET
│   └── src/ast/expr.rs            # Add ListLiteral AST node
├── snow-typeck/
│   ├── src/builtins.rs            # Polymorphic list function schemes
│   └── src/infer.rs               # Infer ListLiteral type, polymorphic module sigs
├── snow-codegen/
│   ├── src/mir/lower.rs           # Lower ListLiteral to from_array call, polymorphic return types
│   ├── src/mir/types.rs           # (no changes expected -- List<T> already maps to Ptr)
│   ├── src/mir/mono.rs            # (no changes expected -- list functions are runtime, not MIR)
│   └── src/codegen/expr.rs        # Codegen for list_from_array, type conversions
└── snow-rt/
    └── src/collections/list.rs    # NO CHANGES -- runtime is already type-erased
```

### Anti-Patterns to Avoid

- **Don't add type metadata to list allocations**: The runtime stores raw u64 values. Adding type tags would break the uniform representation and require a major runtime rewrite. All type safety is enforced at compile time.
- **Don't create separate runtime functions per element type**: `snow_list_append_string`, `snow_list_append_bool`, etc. are unnecessary. The single `snow_list_append(list, u64)` works for all types because all values fit in 8 bytes.
- **Don't change the `++` concat operator typing**: Currently `++` unifies both sides and returns the same type. This already works for `List<T> ++ List<T> -> List<T>` once the type inference produces `List<T>`.
- **Don't try to add `for x in list` iteration syntax in this phase**: The phase description doesn't require it. `map`, `filter`, `reduce` are the iteration primitives. Index access via `List.get(list, i)` is sufficient.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| List element storage | Type-specific list variants | Uniform u64 representation (existing) | All Snow values fit in 8 bytes; runtime is already type-erased |
| Element type inference | Manual type annotation requirement | HM inference with `Ty::list(fresh_var())` | Existing unification engine handles this automatically |
| Nested list display | Manual string building | Existing `resolve_to_string_callback` recursion | Already generates synthetic MIR wrapper functions for arbitrary nesting depth |
| List literal parsing | Custom tokenizer changes | `L_BRACKET` token already exists | Parser just needs to recognize `[` in prefix position |

**Key insight:** 80% of the infrastructure already exists. The runtime is already polymorphic. The type system already has `Ty::list(inner)`. The display system already handles nested collections. The missing pieces are: (a) polymorphic type signatures for list functions, (b) list literal syntax in the parser, (c) list literal lowering in MIR.

## Common Pitfalls

### Pitfall 1: Ambiguity Between Index Access and List Literal

**What goes wrong:** `[` can mean either "start of list literal" (prefix) or "index access" (postfix). If parsing is not context-aware, `foo[1]` could be misinterpreted.
**Why it happens:** Same token `L_BRACKET` used for both.
**How to avoid:** List literals are parsed in **prefix position** (atom/NUD). Index access is parsed in **postfix position** (LED/infix after an expression). The Pratt parser already distinguishes these: prefix `[` is handled in the `atom` match (line 235 area), postfix `[` is handled in the postfix loop (line 126). These are already separate code paths.
**Warning signs:** Parsing tests fail for `list[0]` or `[1, 2]` -- means the prefix/postfix distinction is wrong.

### Pitfall 2: Bool/Float Value Representation Mismatch

**What goes wrong:** Bool is `i8` at LLVM level, Float is `f64`. When stored in a list, they must be stored as `u64`. When retrieved, they must be cast back.
**Why it happens:** `snow_list_append(list, element: u64)` expects a u64. Passing an `i8` bool directly would lose bits.
**How to avoid:** In the LLVM codegen for `list_append`, zero-extend Bool from i8 to i64 before passing. For `list_get` returning a Bool, truncate i64 to i8. For Float, bitcast f64 to i64 (they're both 8 bytes). This is standard for type-erased collections.
**Warning signs:** Booleans always read as `true` from lists, or floats come back as garbage.

### Pitfall 3: Forgetting to Update Both builtins.rs AND infer.rs Module Map

**What goes wrong:** List functions have dual registration: once in `builtins.rs` (bare names like `list_append`, prelude names like `head`, `tail`) and once in the `modules` HashMap in `infer.rs` (for `List.append`, `List.get`, etc.). Updating one but not the other causes type mismatches.
**Why it happens:** Historical separation between "flat" function registration and module-qualified access.
**How to avoid:** Update BOTH locations to use polymorphic schemes. Use the same `TyVar` range (e.g., `TyVar(91000)` and `TyVar(91001)`) for List type parameters to avoid collision with Map's `TyVar(90000)`.
**Warning signs:** `List.get(list, 0)` types correctly but `list_get(list, 0)` still types as `Int`, or vice versa.

### Pitfall 4: Known Functions Map Has Hardcoded Int Types

**What goes wrong:** The `known_functions` HashMap in `lower.rs` registers `snow_list_append` as `FnPtr(vec![Ptr, Int], Ptr)` and `snow_list_get` as `FnPtr(vec![Ptr, Int], Int)`. This means the MIR knows list_get "returns Int" even when the typeck says the return is String.
**Why it happens:** The known_functions map was written for monomorphic lists.
**How to avoid:** Change the known_functions entries to use `Ptr` for element positions (both input and output). At the MIR level, all non-primitive values are `Ptr`. For Int elements, the runtime returns `u64` which is the same as `i64` at the bit level. The MIR lowering should use the typeck-resolved type (from the `types` map) to determine the actual MirType of the result, not the known_functions signature.
**Warning signs:** String elements retrieved from lists cause segfaults because MIR treats them as Int.

### Pitfall 5: Empty List Literal Type Inference

**What goes wrong:** `[]` has no elements to infer the type from, so the element type remains an unresolved type variable.
**Why it happens:** HM inference needs at least one constraint to resolve a type variable.
**How to avoid:** `[]` produces `List<T>` where `T` is a fresh type variable. This is correct -- the type will be determined by how the list is used (e.g., `let xs: List<Int> = []` or `let xs = [] ++ [1]`). If the type is never constrained, it stays as an unresolved variable -- the MIR lowering should fall back to `Ptr` (as it does for all unresolved vars).
**Warning signs:** Empty list causes a panic in resolve_type or an "unresolved type variable" error.

### Pitfall 6: The `++` Operator Must Support List Concatenation at LLVM Level

**What goes wrong:** The `++` (PLUS_PLUS) and `<>` (DIAMOND) operators are currently lowered as `BinOp::Concat`, which maps to `codegen_string_concat` in `expr.rs`. If a `List ++ List` expression reaches codegen, it would incorrectly call string concat.
**Why it happens:** Concat codegen only handles String type currently.
**How to avoid:** In the codegen `BinOp::Concat` handler, check if the operands are `MirType::Ptr` (which List resolves to) and emit a `snow_list_concat` call instead of `snow_string_concat`. Use the typeck type information to disambiguate.
**Warning signs:** `[1, 2] ++ [3, 4]` produces garbage or crashes instead of `[1, 2, 3, 4]`.

## Code Examples

### List Literal Parsing (parser/expressions.rs)

```rust
// In the atom match, at the L_BRACKET comment:
SyntaxKind::L_BRACKET => {
    let m = p.open();
    p.advance(); // consume [
    // Parse comma-separated expressions until ]
    if p.current() != SyntaxKind::R_BRACKET {
        expr_bp(p, 0);
        while p.current() == SyntaxKind::COMMA {
            p.advance(); // consume ,
            if p.current() == SyntaxKind::R_BRACKET {
                break; // trailing comma
            }
            expr_bp(p, 0);
        }
    }
    p.expect(SyntaxKind::R_BRACKET);
    Some(p.close(m, SyntaxKind::LIST_LITERAL))
}
```

### Polymorphic List Registration (builtins.rs)

```rust
// Use type variables for polymorphic list functions
let t_var = TyVar(91000);
let u_var = TyVar(91001);
let t = Ty::Var(t_var);
let u = Ty::Var(u_var);
let list_t = Ty::list(t.clone());
let list_u = Ty::list(u.clone());

// List.append(list, element) -> List  =>  (List<T>, T) -> List<T>
env.insert("list_append".into(), Scheme {
    vars: vec![t_var],
    ty: Ty::fun(vec![list_t.clone(), t.clone()], list_t.clone()),
});

// List.get(list, index) -> T  =>  (List<T>, Int) -> T
env.insert("list_get".into(), Scheme {
    vars: vec![t_var],
    ty: Ty::fun(vec![list_t.clone(), Ty::int()], t.clone()),
});

// List.map(list, fn) -> List  =>  (List<T>, (T) -> U) -> List<U>
env.insert("list_map".into(), Scheme {
    vars: vec![t_var, u_var],
    ty: Ty::fun(vec![list_t.clone(), Ty::fun(vec![t.clone()], u.clone())], list_u.clone()),
});
```

### List Literal Type Inference (infer.rs)

```rust
// In infer_expr, handle ListLiteral:
Expr::ListLiteral(lit) => {
    let elem_ty = ctx.fresh_var();
    let elements: Vec<_> = lit.elements().collect();
    for elem_expr in &elements {
        let t = infer_expr(ctx, elem_expr, env, /* ... */)?;
        ctx.unify(t, elem_ty.clone(), origin.clone())?;
    }
    let resolved_elem = ctx.resolve(elem_ty);
    Ok(Ty::list(resolved_elem))
}
```

### List Literal MIR Lowering (lower.rs)

```rust
fn lower_list_literal(&mut self, lit: &ListLiteral) -> MirExpr {
    let elements: Vec<MirExpr> = lit.elements()
        .map(|e| self.lower_expr(&e))
        .collect();

    if elements.is_empty() {
        // Empty list: call snow_list_new()
        return MirExpr::Call {
            func: Box::new(MirExpr::Var("snow_list_new".into(), /* fn type */)),
            args: vec![],
            ty: MirType::Ptr,
        };
    }

    // Non-empty list: alloca array, store elements, call snow_list_from_array
    // This becomes a sequence: let arr = [e1, e2, ...]; snow_list_from_array(arr, N)
    // The codegen will stack-allocate the array and pass a pointer.
    MirExpr::ListLit {
        elements,
        ty: MirType::Ptr,
    }
}
```

### Bool/Float Conversion in Codegen (codegen/expr.rs)

```rust
// When storing a Bool into a list (snow_list_append):
// Bool is i8, list expects u64. Zero-extend:
let val = if elem_mir_ty == MirType::Bool {
    builder.build_int_z_extend(val.into_int_value(), i64_type, "bool_to_u64")
} else if elem_mir_ty == MirType::Float {
    builder.build_bitcast(val, i64_type, "f64_to_u64")
} else {
    val // Int and Ptr are already i64/ptr-sized
};

// When loading a Bool from a list (snow_list_get):
// List returns u64, Bool needs i8. Truncate:
let val = if result_mir_ty == MirType::Bool {
    builder.build_int_truncate(val.into_int_value(), i8_type, "u64_to_bool")
} else if result_mir_ty == MirType::Float {
    builder.build_bitcast(val, f64_type, "u64_to_f64")
} else {
    val
};
```

## State of the Art

| Old Approach (current) | New Approach (Phase 26) | Impact |
|------------------------|------------------------|--------|
| `List` = `Ty::Con("List")` (opaque) | `List<T>` = `Ty::App(Con("List"), [T])` (parametric) | Element type tracked through inference |
| `list_append: (List, Int) -> List` | `list_append: forall T. (List<T>, T) -> List<T>` | Accepts any element type |
| No list literal syntax | `[1, 2, 3]` parsed as `LIST_LITERAL` | Ergonomic list creation |
| `snow_list_from_array` declared but unused | Called from list literal codegen | Efficient batch list creation |
| Index expressions return `MirExpr::Unit` | Index expressions lower to `snow_list_get` | `list[0]` works (stretch goal) |

## Open Questions

1. **Should index expressions (`list[i]`) be implemented in this phase?**
   - What we know: `IndexExpr` is parsed but not lowered. The MIR returns `Unit`. The phase requirements mention "access elements" which could mean `List.get(list, i)` or `list[i]`.
   - What's unclear: Whether "access" means only `List.get()` or also `list[i]` syntax.
   - Recommendation: Implement `list[i]` as sugar for `List.get(list, i)` if time permits, but `List.get()` satisfies the requirement. The index expression lowering would need to detect when the base is a List (check typeck type) and emit `snow_list_get`.

2. **Should `map`, `filter`, `reduce` prelude functions become polymorphic?**
   - What we know: Bare `map`, `filter`, `reduce` default to list operations. Currently typed as `(List, (Int) -> Int) -> List`.
   - What's unclear: Making them polymorphic requires knowing the element type at the call site.
   - Recommendation: Yes, make them polymorphic. They share the same type variables as the `list_*` variants. The `int_to_int` closure type becomes `(T) -> U` for map, `(T) -> Bool` for filter, `(U, T) -> U` for reduce.

3. **How should `++` distinguish string concat from list concat at codegen?**
   - What we know: Both `++` and `<>` lower to `BinOp::Concat`. Currently codegen always calls `snow_string_concat`.
   - What's unclear: The codegen needs the typeck type info to choose between string and list concat.
   - Recommendation: In `codegen_binop`, when handling `Concat`, check the `MirType` of the operands. If `MirType::String`, call `snow_string_concat`. If `MirType::Ptr`, check the typeck type. If it's a List type, call `snow_list_concat`. This may require passing typeck info to the binop codegen, or adding a `ListConcat` variant to `BinOp`.

4. **What TyVar indices to use for List type parameters?**
   - What we know: Map uses `TyVar(90000)` and `TyVar(90001)`. Default uses `TyVar(99000)`. Compare uses `TyVar(99002)`.
   - Recommendation: Use `TyVar(91000)` for T and `TyVar(91001)` for U in list functions. This avoids collision with existing indices.

## Sources

### Primary (HIGH confidence)
- Codebase analysis: `snow-rt/src/collections/list.rs` -- runtime list implementation (uniform u64 storage)
- Codebase analysis: `snow-typeck/src/ty.rs` -- `Ty::list(inner)` already creates parametric list type
- Codebase analysis: `snow-typeck/src/builtins.rs` -- current monomorphic list function registrations
- Codebase analysis: `snow-codegen/src/mir/lower.rs` -- list operation lowering, display infrastructure
- Codebase analysis: `snow-codegen/src/mir/types.rs` -- `List<T>` already resolves to `MirType::Ptr`
- Codebase analysis: `snow-parser/src/parser/expressions.rs:235` -- explicit TODO for list literal parsing
- Codebase analysis: `snow-codegen/src/codegen/intrinsics.rs` -- `snow_list_from_array` already declared

### Secondary (MEDIUM confidence)
- Codebase analysis: `snow-typeck/src/infer.rs` -- module-qualified List access pattern
- Codebase analysis: Map polymorphic registration pattern (builtins.rs lines 346-361)

## Metadata

**Confidence breakdown:**
- Runtime changes: HIGH -- the runtime is already type-erased, zero changes needed
- Parser changes: HIGH -- L_BRACKET handling is well-understood, similar to map literal parsing
- Type system changes: HIGH -- follows established Map polymorphic pattern exactly
- MIR lowering changes: HIGH -- follows established patterns, display infrastructure exists
- Codegen changes: MEDIUM -- Bool/Float value conversions need careful handling, `++` disambiguation needs design
- Pitfalls: HIGH -- analyzed actual code paths, identified concrete issues

**Research date:** 2026-02-08
**Valid until:** 2026-03-08 (stable codebase, internal project)
