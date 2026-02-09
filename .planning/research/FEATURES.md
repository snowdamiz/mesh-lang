# Feature Landscape: Method Dot-Syntax

**Domain:** Method call syntax (`value.method(args)`) for a statically-typed language with existing pipe operator, field access, and trait/impl dispatch
**Researched:** 2026-02-08
**Confidence:** HIGH (extremely well-established domain with extensive prior art from Rust, Swift, and others)

---

## Current State in Snow

Before defining features for method dot-syntax, here is what already exists:

**Working (infrastructure this feature depends on):**
- `FIELD_ACCESS` CST node: `expr.field` parsed as postfix at BP 25 (highest)
- Module-qualified calls: `String.length(s)`, `IO.println(msg)` -- parsed as `FIELD_ACCESS` then resolved in type checker
- Pipe operator: `items |> List.filter(pred) |> List.map(f)` -- parsed as `PIPE_EXPR` at BP 3/4 (lowest)
- Trait impl blocks with `TraitRegistry`: stores `ImplDef` with methods keyed by name, supports `find_method_traits()`, `resolve_trait_method()`, structural type matching via unification
- Auto-derive for Display, Debug, Eq, Ord, Hash, Default -- generates impl methods
- MIR lowering: impl methods mangled as `TraitName__method_name__TypeName`, call sites resolved via `find_method_traits` + `mir_type_to_impl_name`
- Monomorphization for generics with trait bounds
- CallExpr callee resolution: already checks `find_method_traits()` when callee is not a known function and first arg matches an impl type
- `self` parameter detection via `SELF_KW` in impl method lowering, bound to concrete implementing type

**Not yet working (what this milestone adds):**
- `value.method(args)` syntax that resolves to an impl method with `value` as first argument
- Parser disambiguation between `expr.field` (field access) and `expr.method(args)` (method call)
- Type checker method resolution: given a receiver type, find the matching impl method
- Method chaining: `value.method1().method2().method3()`

**Key parser observation:** The current parser already handles `expr.ident` as `FIELD_ACCESS` and `expr(args)` as `CALL_EXPR`. When the user writes `value.method(args)`, the Pratt parser will produce a `CALL_EXPR` whose callee is a `FIELD_ACCESS`. This is exactly how Rust's parser works -- the disambiguation happens in the type checker, not the parser.

---

## Table Stakes

Features users expect from method dot-syntax. Missing any of these and the feature feels broken or incomplete.

| Feature | Why Expected | Complexity | Dependencies | Notes |
|---------|--------------|------------|--------------|-------|
| Basic method call: `value.method(args)` | The core feature. Users write `point.to_string()` and it resolves to the Display impl's `to_string` method with `point` as `self`. | Medium | Parser (may already work), type checker method resolution, MIR lowering | Parser likely produces `CALL_EXPR(FIELD_ACCESS(value, method), args)` already. Type checker needs to detect this pattern, look up receiver type, find matching impl method, and rewrite to `TraitName__method__TypeName(value, args)`. |
| Field vs. method disambiguation via parentheses | `point.x` is field access; `point.x()` is method call. Parentheses are the disambiguator. | Low | Type checker | Follows Rust's approach exactly. The parser already distinguishes: no parens = `FIELD_ACCESS`, parens = `CALL_EXPR(FIELD_ACCESS(...), ...)`. The type checker resolves based on whether `x` is a struct field (no parens) or a method (with parens). No ambiguity in the grammar. |
| Method chaining | `list.filter(pred).map(f).length()` -- each method call returns a value that the next dot-call operates on. | Low | Basic method call working | Falls out naturally from left-to-right parsing at BP 25. The Pratt parser already chains postfix operations. If `filter` returns a `List<T>`, the type checker resolves `.map(f)` on that `List<T>`. No additional work needed once basic method calls work. |
| Trait method resolution by receiver type | Given `value.method()`, look up the concrete type of `value`, search all trait impls for that type, and find one that provides `method`. | Medium | `TraitRegistry.find_method_traits()` (exists), type checker | Already have `find_method_traits(method_name, ty)` which does exactly this. The current code path in MIR lowering already uses this for bare function calls when the callee is unknown. Need to wire the same logic into type-checker-level method resolution. |
| Self-parameter passing | `value.method(a, b)` becomes `method(value, a, b)` -- receiver is prepended as first argument, matching the `self` parameter in the impl method definition. | Low | Method resolution, MIR lowering | MIR lowering already handles `self` parameters in impl methods. The desugaring `value.method(a, b)` -> `TraitName__method__TypeName(value, a, b)` is structurally identical to how pipe expressions desugar: `value |> method(a, b)` -> `method(value, a, b)`. |
| Methods on generic types | `let items: List<Int> = ...; items.length()` -- method resolution works when the receiver has generic type parameters. | Medium | Generic type unification in `find_method_traits` (exists) | `TraitRegistry` already uses structural type matching via `freshen_type_params` + unification to match generic impls. `impl Iterable for List<T>` will match against `List<Int>` through the existing unification machinery. |
| Error messages for "no such method" | When `value.nonexistent()` is called, the compiler says "type X has no method `nonexistent`" rather than a confusing generic error. | Medium | Type checker diagnostics | Important for usability. Should list available methods on the type, similar to "no such field" errors already produced by `infer_field_access`. |
| Interaction with existing module-qualified calls | `String.length(s)` continues to work as a module-qualified function call. `s.length()` also works as a method call. Both resolve to the same underlying function. | Medium | Type checker precedence rules | The type checker's `infer_field_access` already checks for module-qualified access first (lines 3903-3944 in infer.rs). Method resolution is a fallback when the base expression is not a module name. This priority order must be preserved. |
| Interaction with existing pipe operator | Both `items |> List.filter(pred)` and `items.filter(pred)` work. They are different syntax for the same underlying operation. | Low | Both features independently working | No interaction issues. Pipe uses `PIPE_EXPR` node, method call uses `CALL_EXPR(FIELD_ACCESS(...), ...)`. Different parse trees, same semantic result. Users choose whichever reads better. |

---

## Differentiators

Features that would make Snow's method dot-syntax stand out. Not strictly expected, but valued.

| Feature | Value Proposition | Complexity | Dependencies | Notes |
|---------|-------------------|------------|--------------|-------|
| Inherent methods (impl without trait) | `impl Point do fn distance(self, other: Point) -> Float do ... end end` -- methods directly on a type without a trait. Avoids needing a trait for every method. | High | New `impl Type do ... end` syntax (no trait name), separate method registry | Rust allows `impl Type { ... }` for inherent methods. This is a major ergonomic win for type-specific helpers that don't belong to any trait. **Recommendation: Defer to post-MVP.** Snow's trait-only approach works for v1, and adding inherent methods later is backward-compatible. The MVP should focus on trait method dispatch. |
| Ambiguity resolution for multiple traits | When two traits provide the same method name for the same type, `value.method()` should produce a clear error with disambiguation guidance: "method `to_string` is ambiguous between traits Display and Printable; use `Display.to_string(value)` for explicit dispatch." | Medium | `find_method_traits()` returning multiple results (already supported), diagnostic formatting | `find_method_traits` already returns a `Vec<String>` of matching trait names. The check for `len() > 1` is the ambiguity case. Already implemented in MIR lowering (uses first match). Should become a type-checker error instead. |
| Fully qualified disambiguation syntax | `Display.to_string(point)` as explicit syntax when dot-call is ambiguous. Already works as module-qualified call pattern. | Low | Existing module-qualified call infrastructure | This already works because `Display.to_string(point)` parses as `FIELD_ACCESS(Display, to_string)` then `CALL_EXPR` with `point` as arg. The existing call resolution in MIR lowering handles this via `find_method_traits`. Just need to ensure it continues working alongside dot-syntax. |
| Methods in let-binding position | `let f = point.to_string` -- treating a method as a partially-applied function (capturing the receiver). | Very High | Partial application, closure generation | This is how Rust's method references work but requires generating a closure that captures `point`. **Recommendation: Do not build.** This is complex and Snow's closure syntax handles the use case: `let f = fn() do point.to_string() end`. |
| Method call in pattern-adjacent positions | Support method calls in guard clauses: `case x do n when n.is_positive() -> ...` | Medium | Guard clause evaluation | Already possible if guard clauses evaluate arbitrary expressions. Worth verifying but likely falls out naturally. |
| IDE autocomplete for methods | After typing `value.`, IDE shows available methods based on the value's type and its trait impls. | Medium | LSP server type-at-cursor, trait registry queries | Significant usability win. The `find_method_traits` infrastructure provides the data needed. The LSP server would query the trait registry for all methods available on the resolved type. **Recommendation: Important but separate from core language work.** |

---

## Anti-Features

Features to explicitly NOT build. Common requests that create problems in this domain.

| Anti-Feature | Why Requested | Why Problematic | Alternative |
|--------------|---------------|-----------------|-------------|
| UFCS (Universal Function Call Syntax) | "Any function `f(x, y)` should be callable as `x.f(y)`" | Blurs the distinction between methods and free functions. Makes code harder to read because `x.f()` could be any function with `x` as first arg, not just impl methods. Creates namespace pollution (every function becomes a potential method). Adds massive complexity to method resolution. C++ rejected UFCS after years of proposals. | Snow has pipes for function chaining (`x |> f(y)`) and dot-syntax for impl methods (`x.f(y)` when `f` is in an impl block). These serve different purposes clearly. |
| Auto-ref / auto-deref on method receiver | "The compiler should automatically add `&` or `*` to make the receiver type match" | Snow does not have references or pointer types. All values are passed by value (with compiler-managed copy/move semantics). Auto-ref/deref is a Rust-specific concern driven by its ownership system. Adding this would be solving a problem Snow does not have. | Not needed. Snow's value semantics mean the receiver is always the value itself. No `&self` vs `&mut self` vs `self` distinction. |
| Method overloading by parameter count | "Allow `point.distance()` and `point.distance(other)` to be different methods" | Snow does not support function overloading. Adding it for methods only would be inconsistent. Overloading complicates type inference (which overload did you mean?) and error messages. | Use different method names: `point.distance_to(other)` vs `point.distance_from_origin()`. Or use optional parameters if Snow adds them. |
| Implicit self in method bodies | "Inside an impl method, `x` should resolve to `self.x` automatically (like Ruby/Python)" | Creates ambiguity between local variables and fields. Harder to read: is `x` a local or a field? Rust requires explicit `self.x` and this is widely considered a good design decision for readability. | Always require `self.x` for field access within method bodies. Snow already has this pattern working in impl methods. |
| Extension methods (impl for foreign types without traits) | "I want to add methods to String without defining a trait" | Breaks encapsulation and coherence. Two modules could add conflicting extension methods. Makes it unclear where a method comes from. Swift and C# have this and it causes confusion about method provenance. | Use the pipe operator with module-qualified functions: `s |> MyUtils.enhanced_length()`. Or define a trait: `impl MyStringOps for String do ... end`. |
| Cascading method syntax (Dart's `..`) | "I want `builder..setX(1)..setY(2)..build()` where `..` returns the receiver, not the method result" | Adds a new operator with subtle semantics. Snow's value semantics and functional style mean builders are rare. The use case is niche. | Use explicit chaining with methods that return `self`: `builder.set_x(1).set_y(2).build()`. Or use struct literal syntax: `Config { x: 1, y: 2 }`. |
| Dynamic method dispatch (vtable-based) | "I want `dyn Trait` or `protocol existentials`" | Snow uses monomorphization (static dispatch). Adding dynamic dispatch requires vtables, heap allocation for trait objects, and massively complicates the type system. This was explicitly deferred. | Use sum types for heterogeneous collections: `type Displayable do S(String), I(Int) end`. Pattern match to dispatch. |

---

## Feature Dependencies

```
Existing Infrastructure
    |
    +-- FIELD_ACCESS parser node (exists, BP 25)
    |       Produces: CALL_EXPR(FIELD_ACCESS(base, method), ARG_LIST(...))
    |       when followed by parentheses
    |
    +-- TraitRegistry (exists)
    |       Has: find_method_traits(), resolve_trait_method()
    |       Needs: infer_method_call() in type checker
    |
    +-- MIR lowering trait dispatch (exists)
    |       Has: find_method_traits() -> mangled name resolution
    |       Needs: handle METHOD_CALL desugaring from type checker
    |
    v
[1] Type Checker Method Resolution
    |   Detect CALL_EXPR with FIELD_ACCESS callee
    |   Resolve receiver type -> find matching impl method
    |   Validate argument types against method signature
    |   Record resolved method info for MIR lowering
    |   CRITICAL: Must not break existing module-qualified calls
    |
    v
[2] MIR Lowering for Method Calls
    |   Desugar value.method(args) -> TraitName__method__TypeName(value, args)
    |   Reuse existing mangled name generation
    |   Handle generic receivers (substitute type params)
    |
    v
[3] Method Chaining
    |   Falls out from [1] + [2] naturally
    |   Each call's return type becomes next call's receiver type
    |   Already works at parser level
    |
    v
[4] Error Diagnostics
    |   "No method `foo` on type `Bar`"
    |   "Method `foo` is ambiguous between traits X and Y"
    |   Suggest available methods on the type
    |
    v
[5] Interaction with Pipe Operator (verify)
        value.method(args) and value |> method(args)
        should produce equivalent results
        Both paths already converge at MIR level
```

**Critical path:** [1] -> [2] -> [3] (trivial) -> [4]

The parser likely needs zero changes. The core work is in the type checker ([1]) and MIR lowering ([2]).

---

## Detailed Design Decisions

### Decision 1: Parser Strategy

**Question:** Should the parser produce a new `METHOD_CALL` CST node, or reuse the existing `CALL_EXPR(FIELD_ACCESS(...), ...)` tree shape?

**Recommendation: Reuse existing tree shape.** No new CST node needed.

**Rationale:**
- The Pratt parser already produces `CALL_EXPR(FIELD_ACCESS(base, method), ARG_LIST(...))` for `value.method(args)` because field access (BP 25) is parsed before call (BP 25), and the call wraps around the field access result.
- Rust's parser works the same way -- method calls and field accesses share the same dot-expression grammar; disambiguation happens during name resolution.
- Adding a new CST node would require parser changes to distinguish "FIELD_ACCESS followed by ARG_LIST" from "standalone FIELD_ACCESS", which would complicate the Pratt loop unnecessarily.
- The type checker already receives both node types and can inspect the callee of a `CALL_EXPR` to detect the `FIELD_ACCESS` pattern.

**Confidence:** HIGH

### Decision 2: Method Resolution Priority

**Question:** When `value.name(args)` is called, what is the resolution order?

**Recommendation: Field access first, then method lookup. Parentheses disambiguate.**

**Priority order (matching Rust):**
1. If `base` is a module name (String, IO, List, etc.) -> module-qualified function call (existing behavior)
2. If `base` is a sum type name -> variant constructor (existing behavior)
3. If no parentheses (`value.name`) -> struct field access only
4. If parentheses (`value.name(args)`) -> first check if `name` is a callable field (function-typed field), then search impl methods via trait registry

**Why this order:**
- Modules and variants must keep working unchanged (backward compatibility)
- Struct field access without parens must keep working unchanged
- Method lookup only activates when there are parentheses, preventing accidental method resolution when the user wants a field

**Confidence:** HIGH

### Decision 3: Trait Method Only (No Inherent Methods for MVP)

**Question:** Should `value.method()` resolve only methods defined in trait impl blocks, or also "inherent" methods (impl blocks without a trait)?

**Recommendation: Trait methods only for MVP.**

**Rationale:**
- Snow's entire method infrastructure is built around `interface` + `impl Trait for Type`. The `TraitRegistry`, `find_method_traits`, mangled name generation -- all assume a trait name.
- Inherent methods would need a separate registry, different name mangling (no trait prefix), and priority rules (inherent before trait, matching Rust).
- The vast majority of use cases are covered by trait methods: `to_string()`, `inspect()`, `eq()`, `hash()`, etc.
- Adding inherent methods later is fully backward-compatible -- existing trait method calls continue to work.

**Confidence:** HIGH

### Decision 4: Ambiguity Handling

**Question:** What happens when two traits provide the same method name for the same type?

**Recommendation: Compile-time error with disambiguation guidance.**

```
error: method `to_string` is ambiguous for type `Point`
  --> src/main.snow:15:5
   |
15 | point.to_string()
   |       ^^^^^^^^^ method found in both `Display` and `Printable`
   |
help: use fully-qualified syntax to disambiguate
   |
15 | Display.to_string(point)
   |
15 | Printable.to_string(point)
```

**Rationale:**
- The current MIR lowering silently uses the first matching trait, which is fragile and order-dependent.
- The type checker should detect ambiguity earlier and produce a clear error.
- Fully-qualified syntax (`TraitName.method(value)`) already works via existing module-qualified call paths.

**Confidence:** HIGH

### Decision 5: Pipe Operator Coexistence

**Question:** How do `value.method(args)` and `value |> method(args)` interact?

**Recommendation: Both work. They are different syntax for the same semantics. No preference enforced.**

**Semantic equivalence:**
```snow
# These should produce identical results:
items.filter(pred).map(f)
items |> filter(pred) |> map(f)

# Mixed usage is fine:
items.filter(pred) |> map(f)
```

**Note:** Pipe works with ANY function (the piped value becomes the first argument). Method dot-syntax only works with impl methods. This is an important distinction:
- `items |> List.filter(pred)` -- works because `List.filter` is a module function
- `items.filter(pred)` -- works only if there's a trait impl for the items' type that provides `filter`

**Confidence:** HIGH

### Decision 6: Self Return for Chaining

**Question:** Should there be special support for methods that return `Self` to enable chaining?

**Recommendation: No special support needed. Snow's existing type inference handles this.**

**How it works naturally:**
```snow
interface Builder do
  fn set_name(self, name: String) -> Self
  fn set_age(self, age: Int) -> Self
  fn build(self) -> Person
end

# Chaining works because each method's return type (Self = PersonBuilder)
# becomes the receiver for the next call:
builder.set_name("Alice").set_age(30).build()
```

The type checker resolves `Self` to the concrete implementing type during method resolution. Each chained call's return type feeds into the next call's receiver type inference. No special mechanism required.

**Confidence:** HIGH -- this is how Rust handles it too.

---

## Edge Cases and Their Resolutions

### Edge Case 1: Field and Method with Same Name

**Scenario:** A struct has a field `width` and an impl provides a method `width()`.

```snow
struct Rectangle do
  width :: Int
  height :: Int
end

impl Measurable for Rectangle do
  fn width(self) -> Int do self.width end  # getter pattern
end
```

**Resolution:** Parentheses disambiguate.
- `rect.width` -> field access (returns `Int`)
- `rect.width()` -> method call (calls `Measurable__width__Rectangle`)

This matches Rust's behavior exactly. The parser produces different CST trees for each case, so the type checker handles them in different code paths.

**Confidence:** HIGH

### Edge Case 2: Method on Generic Type with Unresolved Type Variable

**Scenario:** The receiver's type is not yet fully resolved during inference.

```snow
fn process(items) do
  items.length()  # type of `items` is still a type variable
end
```

**Resolution:** Defer method resolution until the receiver type is resolved. If the type remains unresolved at the end of inference, emit an error: "cannot determine type of `items`; consider adding a type annotation."

This is similar to how Rust handles turbofish scenarios. Snow's HM inference should resolve most cases; the edge case arises only with insufficient context.

**Confidence:** MEDIUM -- implementation details depend on how Snow's inference engine handles deferred constraints.

### Edge Case 3: Method on Sum Type

**Scenario:** Calling a method on a sum type value.

```snow
type Shape do
  Circle(Float)
  Rectangle(Float, Float)
end

impl Display for Shape do
  fn to_string(self) -> String do
    case self do
      Circle(r) -> "Circle(${r})"
      Rectangle(w, h) -> "Rectangle(${w}, ${h})"
    end
  end
end

let s: Shape = Circle(5.0)
s.to_string()  # should work
```

**Resolution:** This works naturally. The receiver type is `Shape`, `find_method_traits("to_string", Shape)` finds `Display`, and the mangled name is `Display__to_string__Shape`. The method body handles dispatch via pattern matching internally.

**Confidence:** HIGH

### Edge Case 4: Method on Result of Another Method (Chaining)

**Scenario:** Chaining methods where intermediate types differ.

```snow
let result = items.filter(is_positive).map(to_string).length()
# filter returns List<Int>, map returns List<String>, length returns Int
```

**Resolution:** Each method call is independently resolved:
1. `items.filter(is_positive)` -- receiver is `List<Int>`, returns `List<Int>`
2. `(result_of_filter).map(to_string)` -- receiver is `List<Int>`, returns `List<String>`
3. `(result_of_map).length()` -- receiver is `List<String>`, returns `Int`

The Pratt parser's left-to-right evaluation handles the CST nesting. The type checker processes each `CALL_EXPR` from innermost to outermost.

**Confidence:** HIGH

### Edge Case 5: Method Call in Pipe Chain

**Scenario:** Mixing dot-syntax and pipe operator.

```snow
items.filter(pred) |> List.map(f)
items |> filter(pred) |> map(f)  # pipe-style
```

**Resolution:** Both work independently. The pipe operator desugars `a |> f(b)` to `f(a, b)`. The dot-syntax desugars `a.f(b)` to `TraitName__f__TypeName(a, b)`. The results should be semantically equivalent when `f` is a trait method.

**Note:** There is a subtle difference: pipe works with any function, dot-syntax only with impl methods. `items |> some_free_function()` works; `items.some_free_function()` does not (unless there's a matching impl method).

**Confidence:** HIGH

### Edge Case 6: Nested Field Access then Method Call

**Scenario:** Accessing a field, then calling a method on the field's value.

```snow
struct Person do
  name :: String
  address :: Address
end

person.address.to_string()
# parses as: CALL_EXPR(FIELD_ACCESS(FIELD_ACCESS(person, address), to_string), ())
```

**Resolution:** The type checker processes:
1. `person.address` -> field access on `Person`, returns `Address`
2. `(address_value).to_string()` -> method call on `Address`

The nested `FIELD_ACCESS` is the callee's callee. The type checker needs to check: is the outer `FIELD_ACCESS`'s base a struct type with a field matching the name, OR is it a method call? The answer depends on whether parentheses follow.

In the CST, the structure is: `CALL_EXPR(callee=FIELD_ACCESS(base=FIELD_ACCESS(person, address), field=to_string), args=())`. The inner `FIELD_ACCESS` is a pure field access (no parens). The outer `FIELD_ACCESS` is part of a method call (has parens via the wrapping `CALL_EXPR`).

**Confidence:** HIGH

---

## MVP Recommendation

### Build (method dot-syntax milestone)

**Phase 1: Core Method Resolution**
- Type checker detects `CALL_EXPR` with `FIELD_ACCESS` callee pattern
- Resolve receiver type, search trait impls for matching method
- Validate argument count and types
- Record resolved method info (trait name, method name, type name) for MIR

**Phase 2: MIR Lowering**
- Desugar method call to mangled trait function call
- Receiver prepended as first argument
- Reuse existing `TraitName__method__TypeName` mangling

**Phase 3: Diagnostics and Edge Cases**
- "No such method" error with type information
- Ambiguity error when multiple traits match
- Method chaining verification (should work from Phase 1 naturally)
- Verify interaction with existing module-qualified calls, pipe operator, field access

### Defer to Post-MVP

- Inherent methods (`impl Type do ... end` without trait)
- Method references / partial application (`let f = value.method`)
- IDE autocomplete for methods
- Method visibility (public/private methods in impl blocks)

---

## Complexity Assessment

| Feature | Estimated Effort | Risk | Notes |
|---------|-----------------|------|-------|
| Parser changes | 0-1 days | LOW | Likely zero changes needed; verify existing CST shape |
| Type checker method resolution | 2-3 days | MEDIUM | Core new logic: detect pattern, look up receiver type, search impls. Must not break existing field access or module-qualified calls. |
| MIR lowering for method calls | 1-2 days | LOW | Mostly reusing existing trait dispatch infrastructure |
| Method chaining | 0 days | NONE | Falls out from above naturally |
| Error diagnostics | 1 day | LOW | "No such method", ambiguity errors |
| Edge case handling | 1-2 days | MEDIUM | Nested field+method, generic receivers, sum types |
| Testing | 1-2 days | LOW | Unit tests for resolution, integration tests for chaining |

**Total estimated effort:** 5-10 days

**Key risk:** The type checker's `infer_field_access` function currently handles module-qualified calls, sum type variants, service module access, and struct field access in a specific priority order. Method resolution must be inserted at the correct point in this priority chain without breaking any existing behavior. This is the most delicate part of the implementation.

---

## Sources

### Rust Method Resolution
- [The Dot Operator - The Rustonomicon](https://doc.rust-lang.org/nomicon/dot-operator.html) -- autoref/autoderef algorithm
- [Method Call Expressions - The Rust Reference](https://doc.rust-lang.org/reference/expressions/method-call-expr.html) -- formal method resolution rules
- [Inherent vs Trait Method Priority - rust-lang/rust#26007](https://github.com/rust-lang/rust/issues/26007) -- inherent methods take priority over trait methods at same deref level
- [Methods - The Rust Programming Language](https://doc.rust-lang.org/book/ch05-03-method-syntax.html) -- field vs method disambiguation via parentheses

### Rust Field vs Method Disambiguation
- [Trying to access public function with same name as private field - rust-lang/rust#26472](https://github.com/rust-lang/rust/issues/26472) -- real-world ambiguity case
- [Disambiguating overlapping traits - Rust By Example](https://doc.rust-lang.org/rust-by-example/trait/disambiguating.html) -- fully qualified syntax

### Swift Static Dispatch
- [Method Dispatch Mechanisms in Swift](https://nilcoalescing.com/blog/MethodDispatchMechanismsInSwift/) -- static dispatch for value types (structs/enums), relevant to Snow's design

### UFCS Design Debate (why NOT to build it)
- [Uniform Function Call Syntax - Wikipedia](https://en.wikipedia.org/wiki/Uniform_function_call_syntax) -- overview of UFCS in D, Nim
- [What is unified function call syntax anyway? - Barry's C++ Blog](https://brevzin.github.io/c++/2019/04/13/ufcs-history/) -- C++ UFCS history and why it was rejected
- [UFCS C++ Proposal P3021](https://open-std.org/JTC1/SC22/WG21/docs/papers/2023/p3021r0.pdf) -- problems with ambiguity and overload resolution

### Pipe Operator vs Method Chaining
- [Piping is Method Chaining - Win Vector](https://win-vector.com/2019/04/14/piping-is-method-chaining/) -- semantic equivalence, design tradeoffs
- [The Right Way To Pipe](https://yangdanny97.github.io/blog/2023/12/28/pipes) -- pipe-first vs pipe-last, interaction with method syntax

### Elixir's Design Choice (why dot means something different there)
- [Why the dot (when calling anonymous functions)? - Dashbit Blog](https://dashbit.co/blog/why-the-dot) -- Elixir's Lisp-2 design, dot for anonymous functions

---
*Feature research for: Snow Language Method Dot-Syntax*
*Researched: 2026-02-08*
