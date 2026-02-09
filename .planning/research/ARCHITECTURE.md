# Architecture: Method Dot-Syntax Integration

**Domain:** Adding `expr.method(args)` resolution to an existing compiler with field access, module lookups, and trait dispatch
**Researched:** 2026-02-08
**Confidence:** HIGH (based on direct codebase analysis of 66,521-line compiler + established UFCS patterns)

---

## Current Pipeline (Relevant Stages)

```
Source: "point.to_string()"
  |
  v
[snow-lexer]     NAME "point" DOT "." NAME "to_string" L_PAREN "(" R_PAREN ")"
  |
  v
[snow-parser]    CALL_EXPR                    <-- binding power 25
                   FIELD_ACCESS               <-- binding power 25
                     NAME_REF "point"
                     DOT "."
                     IDENT "to_string"
                   ARG_LIST
                     L_PAREN R_PAREN
  |
  v
[snow-typeck]    infer_call -> infer_expr(callee) -> infer_field_access
                   Tries: stdlib module? service? sum type variant? struct field?
                   Currently: fails with NoSuchField or returns fresh_var
                   Needed: try method resolution via TraitRegistry
  |
  v
[snow-codegen/mir/lower.rs]    lower_call_expr -> lower_expr(callee) -> lower_field_access
                   Currently: produces MirExpr::FieldAccess (struct GEP)
                   Needed: detect method call pattern, desugar to trait-dispatched Call
  |
  v
[snow-codegen/codegen/expr.rs]  codegen_call -> emit LLVM direct call
                   No changes needed -- receives desugared MirExpr::Call
```

### What Already Works (Foundation)

1. **Parser:** `expr.method(args)` ALREADY parses correctly. The Pratt parser at binding power 25 produces `CALL_EXPR { FIELD_ACCESS { base, DOT, IDENT }, ARG_LIST { args } }`. The snapshot test `parser_tests__mixed_postfix` confirms `a.b(c)` produces exactly this structure. **No parser changes needed.**

2. **Lexer:** `DOT` token (kind 108) exists. `SELF_KW` (kind 55) exists. **No lexer changes needed.**

3. **AST:** `CallExpr.callee()` returns the first child expression, which will be a `FieldAccess`. `FieldAccess.base()` returns the receiver expression. `FieldAccess.field()` returns the method name IDENT token. **All AST accessors already work.**

4. **MIR:** `MirExpr::Call { func, args, ty }` handles direct function calls with mangled names. Trait method dispatch already works for `to_string(42)` -- MIR lowering rewrites to `Trait__Method__Type` mangled name. **No new MIR node types needed.**

5. **Codegen:** `codegen_call` handles `MirExpr::Call` by looking up the function in `self.functions` or `self.module.get_function()`. **No codegen changes needed** -- the desugared call is indistinguishable from a regular call.

6. **Trait infrastructure:** `TraitRegistry.find_method_traits(method_name, ty)` uses structural type matching via temporary unification to find all traits providing a method for a given type. Already used in `lower_call_expr` for bare `to_string(42)` dispatch. **Directly reusable for dot-syntax.**

### What Is Missing (The Gap)

1. **Type checker does not recognize `expr.method(args)` as a method call.** When `infer_call` receives a `CallExpr` whose callee is a `FieldAccess`, it calls `infer_field_access` on the callee. `infer_field_access` tries module lookup, service lookup, sum type variant, then struct field lookup. If the field name is not a struct field, it returns `NoSuchField` error. There is no fallback to "try resolving as a trait method."

2. **MIR lowering does not detect the method call pattern.** `lower_call_expr` calls `lower_expr` on the callee, which dispatches to `lower_field_access`. This produces `MirExpr::FieldAccess { object, field, ty }` -- a struct GEP -- not a method call. The trait method rewriting logic in `lower_call_expr` (lines 3527-3599) only fires when the callee is `MirExpr::Var(name, _)`, not when it is `MirExpr::FieldAccess`.

3. **No self-parameter prepending.** When `point.to_string()` is written, the receiver `point` should be prepended as the first argument to `to_string`. Currently, the args from the parser's `ARG_LIST` do not include the receiver. The desugaring `point.to_string()` -> `to_string(point)` must happen explicitly.

4. **Priority / ambiguity rules.** When `expr.name` could be either a struct field access OR a method call followed by `(args)`, the compiler must decide which takes priority. The correct rule: **struct fields take priority over methods** (same as Rust's inherent-over-trait rule). This prevents breakage of existing `self.x` field access inside impl bodies.

---

## Recommended Architecture

### Design Principle: Desugaring, Not a New Path

Method dot-syntax is NOT a new dispatch mechanism. It is **syntactic sugar** that desugars into the existing bare-name call path:

```
point.to_string()  -->  to_string(point)
a.compare(b)       -->  compare(a, b)
x.method1().method2() --> method2(method1(x))
```

The existing trait method dispatch infrastructure (TraitRegistry lookup, Trait__Method__Type mangling, static dispatch) handles everything after desugaring. This means:

- **No new MIR nodes.** Method calls become `MirExpr::Call` with the receiver prepended to args.
- **No codegen changes.** The desugared call is a regular function call.
- **No runtime changes.** Static dispatch via monomorphization, same as bare-name calls.
- **Two integration points only:** typeck and MIR lowering.

### Component Boundaries

| Component | Responsibility | What Changes |
|-----------|---------------|--------------|
| snow-lexer | Tokenization | **Nothing** -- DOT and IDENT tokens already exist |
| snow-parser | CST construction | **Nothing** -- `expr.method(args)` already parses as CALL_EXPR(FIELD_ACCESS(...), args) |
| snow-typeck/infer.rs | Type inference | **Moderate** -- `infer_field_access` gains method resolution fallback; `infer_call` gains dot-call detection |
| snow-codegen/mir/lower.rs | AST -> MIR | **Moderate** -- `lower_call_expr` detects `FieldAccess` callee pattern and desugars |
| snow-codegen/mir/mod.rs | MIR types | **Nothing** -- existing Call node is sufficient |
| snow-codegen/codegen/ | MIR -> LLVM IR | **Nothing** -- desugared calls are regular MirExpr::Call |
| snow-rt | Runtime | **Nothing** -- static dispatch, no runtime method tables |

### Data Flow: Method Call Resolution

```
Source: point.to_string()

Parser Output:
  CALL_EXPR
    FIELD_ACCESS
      NAME_REF "point"     --> base expression (receiver)
      DOT "."
      IDENT "to_string"    --> method name
    ARG_LIST (empty)        --> explicit args

Type Checker (infer_call):
  1. Detect callee is FieldAccess
  2. Infer base type: point :: Point (struct)
  3. Try struct field lookup: Point has no field "to_string"
  4. NEW: Try method resolution via TraitRegistry
     - find_method_traits("to_string", Point) -> ["Display"]
     - Get method signature: (self) -> String
     - Build function type with receiver prepended: Fun([Point], String)
     - Unify with args: args = [] -> full_args = [Point] (receiver prepended)
     - Return type: String
  5. Record in types map: CALL_EXPR range -> String, FIELD_ACCESS range -> Fun([Point], String)

MIR Lowering (lower_call_expr):
  1. callee = lower_expr(FIELD_ACCESS) -> detect method call pattern
  2. Extract: receiver_expr = lower_expr(base = "point"), method_name = "to_string"
  3. Lower explicit args from ARG_LIST: [] (empty)
  4. Prepend receiver to args: [lower("point")]
  5. Apply existing trait dispatch logic:
     - first_arg_ty = MirType::Struct("Point")
     - find_method_traits("to_string", Point) -> ["Display"]
     - Mangle: "Display__to_string__Point"
  6. Emit: MirExpr::Call {
       func: Var("Display__to_string__Point"),
       args: [Var("point", Struct("Point"))],
       ty: MirType::String
     }

Codegen: Regular function call -- no special handling.
```

---

## Changes Per Compiler Stage

### Stage 1: Type Checker (snow-typeck/infer.rs) -- Two Changes

#### Change 1: Method Resolution Fallback in `infer_field_access`

**Location:** `infer_field_access()` at line ~3879 in `infer.rs`

**Current behavior (lines 3903-3993):**
1. Check if base is stdlib module name -> module-qualified lookup
2. Check if base is service module -> service method
3. Check if base is sum type name -> variant constructor
4. Infer base expression type, look up struct field
5. If field not found -> `NoSuchField` error

**New behavior -- add step 4.5 before the error:**
```
4.5. If field not found in struct AND we are inside a CALL_EXPR parent:
     - Query TraitRegistry: find_method_traits(field_name, resolved_base_ty)
     - If method found:
       - Get method signature from trait impl
       - Return the function type Fun([self_type, ...params], ret_type)
       - (The method is callable; infer_call will handle argument checking)
     - If no method found:
       - Fall through to existing NoSuchField error
```

**Critical detail: How to detect "inside a CALL_EXPR parent."** The type checker can check the parent node of the FieldAccess. In rowan, `fa.syntax().parent()` returns the parent CST node. If its kind is `CALL_EXPR`, the field access is a method call callee. If not (e.g., `point.x` as a standalone expression), it is a regular field access.

**Alternative (simpler, recommended):** Do NOT check the parent node. Instead, always try method resolution as a fallback after struct field lookup fails. If the field is not a struct field but IS a trait method, return the method's function type. `infer_call` will then unify this function type against the argument list. If `point.to_string` appears without `()`, it becomes a function value reference -- which is correct behavior (method reference).

**What this returns for `point.to_string`:**
- `Ty::Fun(vec![Ty::Con("Point")], Box::new(Ty::Con("String")))` -- a function value

Then `infer_call` unifies this against the argument list. For `point.to_string()`, the arg list is empty, but the function expects 1 param (self). This mismatch happens because the receiver is NOT in the arg list.

**Complication:** `infer_call` currently builds `expected_fn_ty = Ty::Fun(arg_types, ret_var)` from the explicit args. For `point.to_string()`, `arg_types = []`. But the method type is `Fun([Point], String)`. Unification fails: arity mismatch.

**Resolution:** `infer_call` must detect the dot-call pattern and prepend the receiver type to `arg_types`.

#### Change 2: Dot-Call Detection in `infer_call`

**Location:** `infer_call()` at line ~2671 in `infer.rs`

**Current behavior:**
1. Infer callee expression type
2. Collect argument types from ARG_LIST
3. Build expected function type from arg types
4. Unify callee type with expected type
5. Check where-clause constraints

**New behavior -- after step 2, before step 3:**
```
2.5. If callee_expr is FieldAccess:
     a. Extract base expression and field name
     b. Get the inferred base type from the types map (already inferred in step 1)
     c. Prepend base type to arg_types: arg_types = [base_ty, ...arg_types]
     d. The callee_ty from step 1 already has the correct function type
        (from the modified infer_field_access which returns the method fn type)
     e. Continue to step 3 with the augmented arg_types
```

This is minimal: `infer_call` detects `Expr::FieldAccess` as the callee, prepends the receiver type, and the rest of the existing unification and constraint-checking machinery works unchanged.

**Where-clause checking:** The existing code at lines 2713-2749 checks where-clause constraints only when `callee_expr` is a `NameRef`. For dot-syntax calls, the callee is a `FieldAccess`, not a `NameRef`. Two options:
- (a) Extract the method name from the FieldAccess and use it for constraint lookup
- (b) Let MIR lowering handle constraint enforcement (already the pattern for trait dispatch)

**Recommendation:** Option (a) for correctness -- extract `field_name` from the FieldAccess and use it as the function name for `fn_constraints.get(&field_name)`. This ensures where-clause errors are reported at the call site.

### Stage 2: MIR Lowering (snow-codegen/mir/lower.rs) -- One Change

#### Change: Detect FieldAccess Callee in `lower_call_expr`

**Location:** `lower_call_expr()` at line ~3362 in `lower.rs`

**Current behavior:**
1. `callee = lower_expr(call.callee())` -- this calls `lower_field_access` which produces `MirExpr::FieldAccess`
2. Collect args from ARG_LIST
3. Check if callee is `MirExpr::Var(name, _)` for trait dispatch rewriting
4. Emit `MirExpr::Call { func: callee, args, ty }`

**Problem:** Step 3 only fires for `MirExpr::Var`, not `MirExpr::FieldAccess`. The trait dispatch logic at lines 3527-3599 never triggers for dot-syntax calls.

**New behavior -- BEFORE step 1, intercept the FieldAccess pattern at the AST level:**

```rust
fn lower_call_expr(&mut self, call: &CallExpr) -> MirExpr {
    // ── NEW: Detect method dot-syntax ──
    // If callee is a FieldAccess (expr.method), desugar to a bare call
    // with the receiver prepended as first argument.
    if let Some(Expr::FieldAccess(ref fa)) = call.callee() {
        // Check if this is NOT a module-qualified call (already handled by lower_field_access)
        if let Some(base_expr) = fa.base() {
            let is_module = if let Expr::NameRef(ref nr) = base_expr {
                nr.text().map(|t| STDLIB_MODULES.contains(&t.as_str())
                    || self.service_modules.contains_key(&t)).unwrap_or(false)
            } else {
                false
            };

            if !is_module {
                let method_name = fa.field().map(|t| t.text().to_string())
                    .unwrap_or_default();
                let receiver = self.lower_expr(&base_expr);
                let mut args: Vec<MirExpr> = vec![receiver]; // prepend receiver
                if let Some(al) = call.arg_list() {
                    args.extend(al.args().map(|a| self.lower_expr(&a)));
                }
                let ty = self.resolve_range(call.syntax().text_range());

                // Now apply the existing trait method dispatch logic:
                // Rewrite bare method name to Trait__Method__Type mangled name.
                let first_arg_ty = args[0].ty().clone();
                let callee = /* ... same logic as lines 3527-3599 ... */;

                return MirExpr::Call { func: Box::new(callee), args, ty };
            }
        }
    }

    // ... existing lower_call_expr code continues unchanged ...
}
```

**Key insight:** The interception happens at the AST level (before `lower_expr` is called on the callee), NOT at the MIR level. This avoids producing an intermediate `MirExpr::FieldAccess` that would need to be "un-done." Instead, we directly extract the receiver expression and method name from the AST, lower the receiver, prepend it to args, and feed into the existing trait dispatch logic.

**Reuse of existing dispatch logic:** The trait method rewriting code at lines 3527-3599 already handles:
- Looking up `find_method_traits(method_name, first_arg_ty)` via TraitRegistry
- Generating mangled name `Trait__Method__Type`
- Mapping primitive builtins to runtime functions (`Display__to_string__Int` -> `snow_int_to_string`)
- Fallback for monomorphized generic types
- Warning for unresolved methods

This code should be **extracted into a helper function** (e.g., `resolve_trait_method_callee`) and called from both the existing bare-name path AND the new dot-syntax path. This avoids duplicating the 70+ lines of dispatch logic.

### Stage 3: Remaining Stages -- No Changes

| Stage | Why No Changes |
|-------|----------------|
| Monomorphization (mono.rs) | Reachability analysis discovers mangled names through normal call graph. Dot-syntax calls produce the same `MirExpr::Call` nodes as bare calls. |
| Codegen (codegen/expr.rs) | Receives `MirExpr::Call { func: Var("Display__to_string__Point"), ... }` -- indistinguishable from a bare call. |
| Runtime (snow-rt) | Static dispatch. No vtables, no method tables, no runtime resolution. |
| Formatter (snow-fmt) | Already handles FIELD_ACCESS and CALL_EXPR nodes. Dot-syntax calls parse into existing node types. |
| LSP (snow-lsp) | May benefit from method completion in the future, but not required for v1.6. |

---

## Resolution Priority Rules

When `expr.name` is encountered, the compiler must decide what it means. The resolution order, from highest to lowest priority:

```
1. Module-qualified access (existing)
   - base is a stdlib module name: String.length -> string_length
   - base is a service module: Counter.start -> __service_Counter_start

2. Sum type variant constructor (existing)
   - base is a sum type name: Shape.Circle -> Shape.Circle constructor

3. Struct field access (existing)
   - base has a struct type with a field named "name"
   - Returns the field value

4. Method call via trait impl (NEW)
   - base type has a trait impl that provides method "name"
   - Returns the method as a callable function type
   - When followed by (args), desugars to method(base, args)

5. Error: NoSuchField
   - None of the above matched
```

**Critical invariant: Steps 1-3 do NOT change.** Method resolution is a NEW fallback that fires only when the existing resolution paths all fail. This guarantees backward compatibility:
- `self.x` inside an impl body resolves to struct field (step 3) -- method resolution never fires
- `String.length` resolves to module-qualified (step 1) -- method resolution never fires
- `Shape.Circle` resolves to variant constructor (step 2) -- method resolution never fires
- `point.to_string()` fails steps 1-3, succeeds at step 4 -- NEW behavior

### Ambiguity: Struct Field vs Method

If a struct has a field named `to_string` AND the type implements Display, the field access wins (step 3 > step 4). This matches Rust's behavior where inherent members shadow trait methods.

If ambiguity is truly desired, the user can use the bare-name syntax: `to_string(point)` calls the trait method directly, bypassing field access entirely.

### Interaction with Pipe Operator

```
point |> to_string()     -- already works (bare name, pipe desugaring)
point.to_string()        -- NEW (dot-syntax, method resolution)
```

Both produce the same MIR: `Call { func: "Display__to_string__Point", args: [point] }`.

The pipe operator is NOT affected by this change. Pipe desugaring in `lower_pipe_expr` and `infer_pipe` works at the AST level and prepends the LHS as the first argument to the RHS call. This is independent of method dot-syntax.

**Chaining works naturally:**
```
point.to_string().length()
```
Parser produces:
```
CALL_EXPR                    -- outer: .length()
  CALL_EXPR                  -- inner: .to_string()
    FIELD_ACCESS              -- point.to_string
      NAME_REF "point"
      DOT "."
      IDENT "to_string"
    ARG_LIST ()
  DOT "."
  IDENT "length"
  ARG_LIST ()
```

Wait -- this is WRONG. The parser produces:
```
CALL_EXPR                    -- .length()
  FIELD_ACCESS               -- result_of_to_string.length
    CALL_EXPR                -- .to_string()
      FIELD_ACCESS           -- point.to_string
        NAME_REF "point"
        IDENT "to_string"
      ARG_LIST ()
    IDENT "length"
  ARG_LIST ()
```

Actually, let me trace through the Pratt parser more carefully. The loop processes left-to-right:

1. Parse atom: `NAME_REF "point"`
2. See DOT at bp 25 >= min_bp 0: open FIELD_ACCESS, advance DOT, expect IDENT "to_string", close -> lhs = `FIELD_ACCESS(point, to_string)`
3. See L_PAREN at bp 25 >= min_bp 0: open CALL_EXPR before lhs, parse arg_list `()`, close -> lhs = `CALL_EXPR(FIELD_ACCESS(point, to_string), ())`
4. See DOT at bp 25 >= min_bp 0: open FIELD_ACCESS before lhs, advance DOT, expect IDENT "length", close -> lhs = `FIELD_ACCESS(CALL_EXPR(...), length)`
5. See L_PAREN at bp 25 >= min_bp 0: open CALL_EXPR before lhs, parse arg_list `()`, close -> lhs = `CALL_EXPR(FIELD_ACCESS(CALL_EXPR(...), length), ())`

So the final tree is:
```
CALL_EXPR                           -- .length()
  FIELD_ACCESS                      -- [result].length
    CALL_EXPR                       -- .to_string()
      FIELD_ACCESS                  -- point.to_string
        NAME_REF "point"
        IDENT "to_string"
      ARG_LIST ()
    IDENT "length"
  ARG_LIST ()
```

This is correct. Each method call is `CALL_EXPR(FIELD_ACCESS(base, method), args)`. The base of the outer call is the result of the inner call. The desugaring applies recursively:

- Inner: `point.to_string()` -> detect FieldAccess callee, desugar to `to_string(point)` -> resolves to `Display__to_string__Point(point)` -> returns String
- Outer: `[string_result].length()` -> detect FieldAccess callee, receiver is String result, method is "length" -> desugar to `length(string_result)` -> resolves to `string_length(string_result)` -> returns Int

**Chaining works naturally because each dot-call desugaring is independent.**

---

## Patterns to Follow

### Pattern 1: Fallback Resolution (Priority Chain)

**What:** Try each resolution strategy in priority order. Only attempt the next strategy when the current one definitively fails (returns None, not an error).

**When:** Resolving `expr.name` in `infer_field_access`.

**Existing example in codebase:**
```
// Already in infer_field_access:
1. Try stdlib module -> Some(scheme) => return Ok(ty)
2. Try service module -> Some(scheme) => return Ok(ty)
3. Try sum type variant -> Some(scheme) => return Ok(ty)
4. Try struct field -> found => return Ok(field_ty)
// NEW:
5. Try method via TraitRegistry -> found => return Ok(method_fn_ty)
6. Error: NoSuchField
```

**Why this pattern:** Each step is independent. The priority order is deterministic. New resolution strategies are added at the end without affecting existing ones.

### Pattern 2: AST-Level Desugaring (Before MIR)

**What:** Detect the syntactic pattern at the AST level and rewrite to the desugared form before producing MIR.

**When:** Converting `expr.method(args)` to `method(expr, args)` in `lower_call_expr`.

**Existing example in codebase:**
```rust
// In lower_pipe_expr:
// `x |> f(a, b)` is desugared to `f(x, a, b)` at the AST level.
let lhs = self.lower_expr(&lhs_expr);
let callee = self.lower_expr(&rhs_callee);
let mut args = vec![lhs]; // prepend pipe LHS
args.extend(explicit_args);
MirExpr::Call { func: callee, args, ty }
```

**Why this pattern:** The pipe operator desugaring is the exact same transformation -- prepend a value as the first argument. Method dot-syntax uses the identical pattern. The MIR layer never knows the call originated from dot-syntax.

### Pattern 3: Extract-Then-Dispatch (Helper Function)

**What:** Extract the trait method resolution logic into a reusable helper, called from both bare-name calls and dot-syntax calls.

**When:** The trait dispatch code at lines 3527-3599 of `lower_call_expr` is needed for both `to_string(point)` and `point.to_string()`.

**Proposed:**
```rust
/// Resolve a bare method name to a trait-mangled name given the first argument's type.
/// Returns the resolved MirExpr::Var if a trait method is found, None otherwise.
fn resolve_trait_method(
    &self,
    method_name: &str,
    first_arg_ty: &MirType,
    var_ty: &MirType,
) -> Option<MirExpr> {
    // ... extracted from lines 3527-3599 ...
}
```

**Why this pattern:** Avoids duplicating 70+ lines of dispatch logic. Single source of truth for how trait methods are resolved from method names.

---

## Anti-Patterns to Avoid

### Anti-Pattern 1: New AST Node for Method Calls

**What:** Adding a `METHOD_CALL` syntax kind distinct from `CALL_EXPR`.

**Why bad:** The parser ALREADY produces the correct structure (`CALL_EXPR` with `FIELD_ACCESS` callee). A new node type would require changes to every consumer of the AST: type checker, MIR lowering, formatter, LSP. It would also break the Pratt parser's elegant handling of postfix operations.

**Instead:** Detect the `FieldAccess` callee pattern within existing `CALL_EXPR` handling. The method call is syntactically a call expression with a particular callee shape.

### Anti-Pattern 2: New MIR Node for Method Calls

**What:** Adding `MirExpr::MethodCall { receiver, method, args, ty }` to the MIR.

**Why bad:** Method calls should be completely erased by MIR lowering. Adding a MIR node pushes trait resolution complexity into codegen, which should only see concrete function calls. Every downstream consumer (mono pass, codegen, debugging) would need to handle the new node.

**Instead:** Desugar at the MIR lowering boundary. `point.to_string()` becomes `MirExpr::Call { func: Var("Display__to_string__Point"), args: [point] }`. Codegen sees a regular call.

### Anti-Pattern 3: Resolving Methods in the Parser

**What:** Having the parser distinguish between field access and method calls based on whether `(` follows.

**Why bad:** The parser has no type information. It cannot know whether `point.x` is a field access or a method reference. Resolution requires type information available only in typeck. Also, the parser already handles this correctly: DOT + IDENT produces FIELD_ACCESS, and if L_PAREN follows, the CALL_EXPR wraps it.

**Instead:** Let the parser produce the same structure for both cases. Let typeck resolve the semantics based on type information.

### Anti-Pattern 4: Modifying `infer_call` to Re-Infer the Base Expression

**What:** When detecting a dot-call in `infer_call`, calling `infer_expr` again on the base expression to get its type.

**Why bad:** `infer_expr` has already been called on the callee (including the base via `infer_field_access`). Re-inferring wastes work and can cause unification issues (fresh type variables created twice).

**Instead:** After `infer_field_access` runs (step 1 of `infer_call`), the base type is already in the `types` map. Use `types.get(base_expr.syntax().text_range())` to retrieve it.

### Anti-Pattern 5: Method Resolution in Codegen

**What:** Having `codegen_field_access` check if the field is a method and emit a call.

**Why bad:** Codegen should never make semantic decisions. It translates MIR to LLVM IR mechanically. If codegen needs type system knowledge, the architecture has a leak.

**Instead:** All method resolution happens in typeck (semantic correctness) and MIR lowering (desugaring to concrete calls). By the time codegen sees it, it is a regular `MirExpr::Call`.

---

## Integration Points (Detailed)

### Integration Point 1: `infer_field_access` (typeck)

**File:** `snow-typeck/src/infer.rs`, line ~3879
**Function:** `infer_field_access(ctx, env, fa, types, type_registry, trait_registry, fn_constraints)`

**Current exit points:**
- Line 3911: stdlib module resolved -> `return Ok(ty)`
- Line 3927: service module resolved -> `return Ok(ty)`
- Line 3941: sum type variant resolved -> `return Ok(ty)`
- Line 3978: struct field found -> `return Ok(resolved_field)`
- Line 3988: struct field NOT found -> `return Err(NoSuchField)`
- Line 3992: base is not a struct -> `return Ok(fresh_var())` (fallback)

**Where to insert method resolution:** Between the struct field "not found" error (line 3982-3988) and the error return. Specifically, REPLACE the `NoSuchField` error return with a method resolution attempt:

```
if no struct field found:
    // NEW: Try method resolution
    let method_traits = trait_registry.find_method_traits(&field_name, &resolved_base);
    if !method_traits.is_empty():
        // Resolve method type -- get signature from first matching trait
        let trait_name = &method_traits[0];
        if let Some(impl_def) = trait_registry.find_impl(trait_name, &resolved_base):
            if let Some(method_sig) = impl_def.methods.get(&field_name):
                // Build function type: Fun([self_type, ...other_params], ret_type)
                let fn_ty = build_method_fn_type(&resolved_base, method_sig);
                return Ok(fn_ty);
    // Fall through to existing NoSuchField error
```

**Also insert at line 3992** (the "base is not a struct" fallback). Currently this returns `fresh_var()`, which is incorrect for method calls on primitive types like `42.to_string()`:

```
// Instead of Ok(ctx.fresh_var()), try method resolution first:
let method_traits = trait_registry.find_method_traits(&field_name, &resolved_base);
if !method_traits.is_empty():
    // ... same method resolution as above ...
Ok(ctx.fresh_var())  // final fallback if no method found
```

### Integration Point 2: `infer_call` (typeck)

**File:** `snow-typeck/src/infer.rs`, line ~2671
**Function:** `infer_call(ctx, env, call, types, type_registry, trait_registry, fn_constraints)`

**Change:** After collecting `arg_types` (line 2697), check if callee is FieldAccess and prepend receiver type:

```rust
// After line 2698:
let (arg_types, is_dot_call) = if let Expr::FieldAccess(ref fa) = callee_expr {
    if let Some(base) = fa.base() {
        if let Some(base_ty) = types.get(&base.syntax().text_range()) {
            let mut full_args = vec![base_ty.clone()];
            full_args.extend(arg_types);
            (full_args, true)
        } else {
            (arg_types, false)
        }
    } else {
        (arg_types, false)
    }
} else {
    (arg_types, false)
};
```

**Where-clause checking (line 2713-2749):** Currently only handles `NameRef` callee. For dot-syntax calls, extract the method name from the FieldAccess:

```rust
// After line 2713, add:
} else if let Expr::FieldAccess(ref fa) = callee_expr {
    if let Some(field_tok) = fa.field() {
        let fn_name = field_tok.text().to_string();
        if let Some(constraints) = fn_constraints.get(&fn_name) {
            // ... same constraint checking logic ...
        }
    }
}
```

### Integration Point 3: `lower_call_expr` (MIR lowering)

**File:** `snow-codegen/src/mir/lower.rs`, line ~3362
**Function:** `lower_call_expr(&mut self, call: &CallExpr) -> MirExpr`

**Change:** At the BEGINNING of the function (before line 3363), add the method call desugaring:

```rust
fn lower_call_expr(&mut self, call: &CallExpr) -> MirExpr {
    // ── Method dot-syntax desugaring ──
    // expr.method(args) -> method(expr, args)
    if let Some(Expr::FieldAccess(ref fa)) = call.callee() {
        if let Some(base_expr) = fa.base() {
            // Skip module-qualified and service-qualified access
            // (these are handled by lower_field_access as function references)
            let is_qualified = if let Expr::NameRef(ref nr) = base_expr {
                nr.text().map(|t| {
                    STDLIB_MODULES.contains(&t.as_str())
                        || self.service_modules.contains_key(&t)
                }).unwrap_or(false)
            } else {
                false
            };

            if !is_qualified {
                let method_name = fa.field()
                    .map(|t| t.text().to_string())
                    .unwrap_or_default();

                // Check: is this a struct field access? If the FieldAccess's
                // base has a struct type with a field matching method_name,
                // this is NOT a method call -- let the existing path handle it.
                let base_ty = self.resolve_range(base_expr.syntax().text_range());
                let is_field = self.is_struct_field(&base_ty, &method_name);

                if !is_field {
                    let receiver = self.lower_expr(&base_expr);
                    let mut args = vec![receiver];
                    if let Some(al) = call.arg_list() {
                        args.extend(al.args().map(|a| self.lower_expr(&a)));
                    }
                    let ty = self.resolve_range(call.syntax().text_range());

                    // Apply trait method dispatch (reuse existing logic)
                    let callee = self.resolve_trait_method_callee(
                        &method_name, &args, &ty
                    );

                    return MirExpr::Call {
                        func: Box::new(callee),
                        args,
                        ty,
                    };
                }
            }
        }
    }

    // ... existing lower_call_expr code continues unchanged ...
}
```

**Helper functions needed:**

```rust
/// Check if a MIR type is a struct type with a field of the given name.
fn is_struct_field(&self, ty: &MirType, field_name: &str) -> bool {
    if let MirType::Struct(name) = ty {
        if let Some(fields) = self.mir_struct_defs.get(name) {
            return fields.iter().any(|(f, _)| f == field_name);
        }
    }
    false
}

/// Resolve a bare method name to a mangled trait method callee,
/// given the first argument's type. Extracted from the existing
/// trait dispatch logic at lines 3527-3599.
fn resolve_trait_method_callee(
    &self,
    method_name: &str,
    args: &[MirExpr],
    call_ty: &MirType,
) -> MirExpr {
    let first_arg_ty = args[0].ty().clone();
    let ty_for_lookup = mir_type_to_ty(&first_arg_ty);
    let matching_traits = self.trait_registry.find_method_traits(method_name, &ty_for_lookup);

    if !matching_traits.is_empty() {
        let trait_name = &matching_traits[0];
        let type_name = mir_type_to_impl_name(&first_arg_ty);
        let mangled = format!("{}__{}__{}", trait_name, method_name, type_name);

        // Map builtin impls to runtime functions
        let resolved = self.resolve_builtin_impl(&mangled);
        let var_ty = MirType::FnPtr(
            args.iter().map(|a| a.ty().clone()).collect(),
            Box::new(call_ty.clone()),
        );
        MirExpr::Var(resolved, var_ty)
    } else {
        // Fallback: try known_functions for monomorphized generics
        // ... same logic as existing lines 3564-3594 ...
        let var_ty = MirType::FnPtr(
            args.iter().map(|a| a.ty().clone()).collect(),
            Box::new(call_ty.clone()),
        );
        MirExpr::Var(method_name.to_string(), var_ty)
    }
}
```

---

## Scalability Considerations

| Concern | At Current Scale | At 100 Types | At 1000 Types |
|---------|-----------------|--------------|---------------|
| Method resolution speed | O(n) where n = number of impls. Currently ~30 impls. Negligible. | ~100 impls, still fast (<1ms). | May need indexing by method name. HashMap from method_name to Vec<(trait, type)>. |
| Name mangling collisions | Zero risk with `Trait__Method__Type` and double-underscore separator. | Still zero risk. User identifiers cannot contain `__`. | Still zero risk. |
| Interaction with future features | Method dot-syntax is pure desugaring. Adding associated types, blanket impls, or supertraits does not affect the desugaring. | Same. | Same. |

---

## Suggested Build Order

Based on dependency analysis and risk assessment:

### Step 1: Extract Trait Dispatch Helper (MIR Lowering)

**File:** `snow-codegen/src/mir/lower.rs`
**What:** Extract the trait method resolution logic from lines 3527-3599 into a `resolve_trait_method_callee` helper function. Call it from the existing code path. Verify all 1,232 tests still pass.

**Why first:** This is a pure refactor with zero behavioral change. It creates the reusable infrastructure needed by both bare-name calls and dot-syntax calls. If this breaks anything, the issue is in the extraction, not in the new feature.

**Risk:** LOW. Pure refactoring.

### Step 2: Method Resolution Fallback (Type Checker)

**File:** `snow-typeck/src/infer.rs`
**What:** Add method resolution fallback in `infer_field_access` after struct field lookup fails. When `TraitRegistry.find_method_traits` returns a match, return the method's function type instead of `NoSuchField`.

**Why second:** Type checking is the semantic correctness layer. Getting method resolution right here ensures that `point.to_string()` is correctly typed as `String`, and that `point.nonexistent()` still produces an error.

**Risk:** MEDIUM. Must not break existing field access (`self.x` in impl bodies must still resolve to struct fields). Must handle edge case where base type is a primitive (not a struct).

**Test plan:**
- `point.to_string()` returns String (**new**)
- `42.to_string()` returns String (**new** -- primitive receiver)
- `point.x` still returns Int (**existing** -- field access preserved)
- `self.x` inside impl body still returns Int (**existing**)
- `String.length` still resolves as module-qualified (**existing**)
- `point.nonexistent()` still errors (**existing**)

### Step 3: Dot-Call Detection in `infer_call` (Type Checker)

**File:** `snow-typeck/src/infer.rs`
**What:** In `infer_call`, detect when callee is a `FieldAccess` and prepend the receiver type to `arg_types`. This ensures arity checking works correctly for method calls.

**Why after step 2:** Depends on `infer_field_access` returning a function type for methods. Without step 2, the callee type would be wrong and unification would fail.

**Risk:** MEDIUM. Must correctly prepend receiver type without double-counting. Must handle the case where `infer_field_access` already resolved to a module-qualified function (in which case, do NOT prepend receiver -- the FieldAccess already resolved to a function value, not a method).

### Step 4: MIR Lowering Desugaring (MIR)

**File:** `snow-codegen/src/mir/lower.rs`
**What:** In `lower_call_expr`, detect `FieldAccess` callee, desugar to `method(receiver, args)`, and call `resolve_trait_method_callee` from Step 1.

**Why after step 3:** Type checking must be correct before MIR lowering can rely on the types map. If typeck marks a dot-call as an error, MIR lowering never sees it.

**Risk:** LOW. The desugaring is mechanically simple (same pattern as pipe desugaring). The trait dispatch logic is already tested via bare-name calls.

### Step 5: End-to-End Integration Testing

**What:** Write comprehensive tests covering:
- Basic: `point.to_string()` compiles and runs
- Primitive: `42.to_string()` returns "42"
- Chained: `point.to_string().length()` returns correct Int
- With args: `a.compare(b)` returns Ordering
- Generic: `box.to_string()` where Box<Int> implements Display
- Pipe equivalence: `point.to_string()` and `point |> to_string()` produce same result
- Error: `point.nonexistent()` produces clear error message
- Priority: struct field access still works when a method of the same name exists

**Risk:** LOW. Tests validate the integration.

### Build Order Rationale

```
Step 1 (refactor) ─────> Step 4 (MIR desugaring)
                              |
Step 2 (typeck fallback) ──> Step 3 (typeck prepend) ──> Step 5 (e2e tests)
```

Steps 1 and 2 are independent and can be done in parallel. Step 3 depends on Step 2. Step 4 depends on Steps 1 and 3. Step 5 depends on everything.

**Estimated total effort:** 2-3 phases, each with 1-2 plans. The core changes are ~100-150 lines in typeck and ~80-100 lines in MIR lowering, plus tests.

---

## Risk Assessment

| Risk | Severity | Likelihood | Mitigation |
|------|----------|------------|------------|
| Breaking existing field access (`self.x` in impl bodies) | Critical | Low | Priority rule: struct fields always win over methods. Test extensively with existing impl bodies. |
| Breaking module-qualified calls (`String.length`) | Critical | Low | Module/service check runs first in resolution priority chain. Guard in MIR desugaring skips qualified access. |
| Breaking pipe operator (`x \|> to_string()`) | High | Very Low | Pipe desugaring is completely independent. Different AST node (PIPE_EXPR vs CALL_EXPR). No interaction. |
| Incorrect arity for method calls | Medium | Medium | `infer_call` must prepend receiver to arg_types. If missed, unification fails with arity mismatch. Clear test plan catches this. |
| Double-counting receiver | Medium | Medium | MIR lowering must NOT call `lower_field_access` on the callee (which would produce a FieldAccess MIR node AND the receiver). Instead, extract receiver from AST directly. |
| Where-clause constraints not checked for dot-calls | Medium | Medium | Must extend constraint checking in `infer_call` to handle FieldAccess callee (extract method name for fn_constraints lookup). |
| Method resolution on generic types | Medium | Low | `find_method_traits` already uses structural type matching via temporary unification. Works for `Box<Int>` matching `impl Display for Box<T>`. |

---

## Sources

### Codebase Analysis (HIGH confidence)
- `crates/snow-parser/src/parser/expressions.rs` -- Pratt parser, FIELD_ACCESS at bp 25, CALL_EXPR wrapping
- `crates/snow-parser/src/ast/expr.rs` -- FieldAccess.base(), FieldAccess.field(), CallExpr.callee()
- `crates/snow-parser/tests/snapshots/parser_tests__mixed_postfix.snap` -- confirms `a.b(c)` parse tree
- `crates/snow-typeck/src/infer.rs` -- infer_field_access (line 3879), infer_call (line 2671), resolution priority chain
- `crates/snow-typeck/src/traits.rs` -- TraitRegistry.find_method_traits (line 246), structural type matching
- `crates/snow-codegen/src/mir/lower.rs` -- lower_call_expr (line 3362), lower_field_access (line 3705), trait dispatch (lines 3527-3599)
- `crates/snow-codegen/src/mir/mod.rs` -- MirExpr::Call, MirExpr::FieldAccess definitions
- `crates/snow-codegen/src/codegen/expr.rs` -- codegen_call (line 525), codegen_field_access (line 1096)

### UFCS / Method Resolution Design (MEDIUM confidence)
- [UFCS -- Wikipedia](https://en.wikipedia.org/wiki/Uniform_function_call_syntax)
- [Rust Compiler Dev Guide -- Trait Resolution](https://rustc-dev-guide.rust-lang.org/traits/resolution.html)
- [Implementing UFCS for C++ in Clang](https://dancrn.com/2020/08/02/ufcs-in-clang.html)
- [C++ P3021 -- Unified Function Call Syntax](https://www.open-std.org/jtc1/sc22/wg21/docs/papers/2023/p3021r0.pdf)
- [D Language -- UFCS](https://dlang.org/book/ufcs.html)
- [Rust Method Call Expressions -- Reference](https://web.mit.edu/rust-lang_v1.25/arch/amd64_ubuntu1404/share/doc/rust/html/reference/expressions/method-call-expr.html)
- [rust-lang/rust#51402 -- Method resolution inherent vs trait](https://github.com/rust-lang/rust/issues/51402)
- [Better Trait Resolution in Rust -- mcyoung](https://mcyoung.xyz/2023/04/04/trait-rez-wishlist/)

---
*Architecture research for: Snow v1.6 Method Dot-Syntax*
*Researched: 2026-02-08*
