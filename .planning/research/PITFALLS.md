# Pitfalls Research: Trait System and Stdlib Protocols (v1.3)

**Domain:** Adding traits with monomorphization to an existing HM-typed language compiled via LLVM
**Researched:** 2026-02-07
**Confidence:** HIGH (based on Snow codebase analysis + established PL research + Rust/Haskell ecosystem experience)

**Scope:** This document covers pitfalls specific to v1.3 milestone -- adding user-defined interfaces, impl blocks, where clauses, monomorphization, and stdlib protocols (Display, Iterator, From/Into, Hash, Default). It supersedes and specializes the general project pitfalls for this milestone.

---

## Critical Pitfalls

Mistakes that cause unsound type checking, ICEs (internal compiler errors), or require rewrites. Address in the phase where they first apply.

---

### Pitfall 1: Premature Constraint Checking -- Checking Trait Bounds Before Types Are Fully Resolved

**What goes wrong:**
The current `check_where_constraints` in `TraitRegistry` does a simple `has_impl(trait_name, concrete_ty)` lookup. This only works when the type argument is fully resolved to a concrete type. If `check_where_constraints` is called while type variables are still unbound (before unification completes for the call site), the lookup fails with a false negative: it reports `TraitNotSatisfied` for a type that will eventually resolve to a valid impl. Conversely, checking too late misses real constraint violations.

**Why it happens:**
Algorithm W interleaves constraint generation and solving. When `infer_call` encounters a generic function with `where T: Display`, it must instantiate the scheme (creating fresh type variables), unify argument types, and THEN check the where-clause after the fresh variables have been resolved through unification. The current code does this mostly correctly for direct calls, but the pattern breaks when:
- The call is inside a pipe chain (`x |> show()`) where the argument type flows in later
- The type argument is itself a generic that gets resolved deeper in the call graph
- Multiple constraints reference the same type parameter and resolve at different times

**How to avoid:**
1. Defer where-clause checking to a post-unification pass. After the entire expression tree is inferred, walk all call sites with where-clauses and resolve type arguments through the unification table before checking.
2. Alternatively, accumulate where-clause obligations (like Rust's `Obligation` system) and discharge them at generalization boundaries (let-bindings, function bodies).
3. The current approach of resolving `type_args` from `param_type_param_names` at call sites is fragile -- it only maps type params that appear directly as function parameter types. Type params that appear only in return types, nested positions, or through constraint propagation will be missed.

**Warning signs:**
- False `TraitNotSatisfied` errors on code that should type-check
- Errors disappear when explicit type annotations are added
- Pipe operator chains trigger spurious trait errors that direct calls do not
- Where-clause errors mention `?0` (unresolved type variable) instead of concrete types

**Phase to address:** Phase 1 (inference integration) -- must be correct before codegen can rely on resolved trait bounds.

---

### Pitfall 2: String-Based Impl Lookup Cannot Handle Generic Impls (the type_to_key Problem)

**What goes wrong:**
The current `type_to_key` function in `traits.rs` converts types to string keys for HashMap-based impl lookup. This works for concrete types (`Int` -> `"Int"`, `Float` -> `"Float"`) but fundamentally cannot handle:
- `impl<T> Display for List<T>` -- the impl target is `List<T>`, not a specific instantiation
- `impl<T: Display> Display for Option<T>` -- conditional impls requiring the inner type to also satisfy a bound
- `impl Display for (A, B) where A: Display, B: Display` -- structural impls over tuples/composites

With string-based keys, looking up `has_impl("Display", List<Int>)` fails because the registered key is `"List<T>"` (or `"List<?0>"`), which does not string-match `"List<Int>"`.

**Why it happens:**
String keys were a correct simplification for the compiler-known traits phase, where only primitive types (Int, Float, String, Bool) needed impls. User-defined traits break this assumption because users will write generic impls over parameterized types, which is the entire value proposition of a trait system.

**How to avoid:**
Replace `type_to_key` string matching with structural type matching:
1. Store impl definitions with their type patterns (including type variables for generic impls)
2. To check `has_impl("Display", List<Int>)`, iterate registered Display impls, attempt to unify the impl's target type with the query type using a temporary unification context
3. When an impl has bounds (`where T: Display`), recursively check those bounds after successful matching
4. Cache resolved results per (trait, concrete_type) pair to avoid re-resolution

This is how Rust's trait resolver works: impl matching is essentially a mini-unification pass.

**Warning signs:**
- `impl Display for MyStruct` works but `impl Display for List<Int>` does not
- Generic impls are registered in the trait registry but `has_impl` never finds them
- All stdlib protocol impls for parameterized types (Iterator for List, From for Option) silently fail to resolve
- Test `test_where_clause_satisfied` passes only because it uses monomorphic types

**Phase to address:** Phase 1 (trait infrastructure) -- this is foundational. Every downstream feature depends on correct impl resolution for generic types.

---

### Pitfall 3: Infinite Monomorphization from Recursive Generic Functions

**What goes wrong:**
A generic function that recurses with a growing type parameter causes infinite instantiation. Example:
```
fn process<T>(x :: T) -> String where T: Display do
  process(Wrapper.new(x))  # Wrapper.new :: T -> Wrapper<T>
end
```
Monomorphizing this requires `process<T>`, then `process<Wrapper<T>>`, then `process<Wrapper<Wrapper<T>>>`, and so on without bound. The monomorphization pass enters an infinite loop or crashes with OOM.

**Why it happens:**
The current `mono.rs` is only a reachability pass -- it does not yet create specialized copies of generic functions. When actual monomorphization is implemented, the worklist algorithm that collects `(function_name, concrete_type_args)` pairs must detect cycles where type arguments grow without bound.

This is a known problem in Rust (issue #50043): `print::<Json>` requiring `print::<Wrapper<Json>>` which requires `print::<Wrapper<Wrapper<Json>>>` etc.

**How to avoid:**
1. Impose a monomorphization depth limit (Rust uses 256 by default). When a function has been instantiated with a type that nests the same constructor more than N levels deep, emit a compiler error with a clear message.
2. Track the instantiation stack during monomorphization. If the same function appears in the stack with a strictly larger type (measured by constructor nesting depth), halt and report the cycle.
3. Consider the interaction with Snow's collections: `List<List<Int>>` is fine (bounded depth from source code), but a recursive function creating `List<List<List<...>>>` dynamically is not.

**Warning signs:**
- Compiler hangs during monomorphization with 100% CPU and growing memory
- Stack overflow in the compiler itself (recursive monomorphization function)
- Programs that seem simple take minutes to compile

**Phase to address:** Phase 2 (monomorphization implementation) -- must be built into the worklist algorithm from day one, not added later.

---

### Pitfall 4: Method Name Collision Between Traits -- Nondeterministic Dispatch

**What goes wrong:**
When two traits define methods with the same name and a type implements both, calling the method by name is ambiguous:
```
interface Display do
  fn to_string(self) -> String
end

interface Debug do
  fn to_string(self) -> String
end

impl Display for Int do fn to_string(self) do "42" end end
impl Debug for Int do fn to_string(self) do "Int(42)" end end

to_string(42)  # Which one?
```
The current `resolve_trait_method` in `TraitRegistry` iterates ALL impls looking for a matching method name + type key, and returns the FIRST match. This means the resolved method depends on HashMap iteration order -- which is nondeterministic (Rust's `FxHashMap` does not guarantee order).

**Why it happens:**
The current design registers impl methods as directly callable functions in the type environment (`env.insert(method_name, Scheme::mono(fn_ty))` at line ~1805 in infer.rs). This flat namespace means the LAST registered impl overwrites earlier ones. There is no qualified dispatch mechanism.

**How to avoid:**
1. Detect ambiguity at call sites: when a method name resolves to impls from multiple traits for the same type, emit an error: "method `to_string` is defined by traits Display and Debug for type Int; use qualified syntax"
2. Support qualified dispatch syntax: `Display.to_string(x)` vs `Debug.to_string(x)`
3. Change internal method registration to use trait-qualified names: `Display::to_string` instead of bare `to_string`
4. When only one trait provides a method for the receiver type, allow unqualified dispatch (no ambiguity)
5. This is critical for stdlib: Display and Debug are almost certain to share method names. If they do, every type implementing both will hit this.

**Warning signs:**
- Changing the order of `impl` blocks in source code changes program behavior
- User reports "wrong method called" with no compiler error
- Adding a second trait with a common method name silently breaks existing code
- Nondeterministic test failures related to method dispatch

**Phase to address:** Phase 1 (trait infrastructure) -- must be resolved before stdlib protocols introduce Display, Debug, etc.

---

### Pitfall 5: MIR Has No Representation for Trait Method Calls -- the Codegen Gap

**What goes wrong:**
MIR currently has `MirExpr::Call { func, args, ty }` which takes a function expression (typically a `Var` with a function name). For monomorphized trait methods, the call target must be resolved to a specific, mangled function name. But:
1. `interface` and `impl` items are explicitly skipped during MIR lowering (line 431 of lower.rs: "Skip -- interfaces are erased, type aliases are resolved")
2. Impl method bodies are never lowered to MIR functions
3. There is no MIR node for "call trait method M on type T"
4. When a user writes `to_string(42)`, the type checker resolves it to a function call, but lowering sees a call to `to_string` -- which does not exist as a standalone MIR function

**Why it happens:**
The lowering was designed before trait codegen existed. The skip is correct for type-level declarations (which are erased at runtime), but impl method BODIES contain executable code that must be lowered.

**How to avoid:**
1. During MIR lowering, process `impl` blocks: lower each method body to a MIR function with a mangled name encoding both the trait, method, and implementing type: e.g., `Display__to_string__Int`
2. At call sites where the type checker identifies a trait method call, emit `MirExpr::Call` targeting the mangled name
3. The type checker must communicate which impl is selected to the lowering phase. Options:
   - Store the resolved impl info in the `types: FxHashMap<TextRange, Ty>` map (e.g., add a parallel `impls: FxHashMap<TextRange, ResolvedImpl>` map)
   - Annotate the AST call node with the resolved trait+impl
   - Use a naming convention: the type checker renames `to_string` to `Display__to_string__Int` before lowering sees it
4. Add impl methods to the `MirModule.functions` list so they are visible to the monomorphization reachability pass

**Warning signs:**
- Programs with trait method calls compile without error but produce wrong results or segfault
- LLVM reports "undefined function" for trait method names
- Impl method bodies are type-checked but never appear in compiled output
- The binary is the same size whether or not impl blocks are present

**Phase to address:** Phase 2 (codegen integration) -- this is the primary v1.3 deliverable. Without this, traits are type-check-only with no runtime effect.

---

### Pitfall 6: Generalization Loses Trait Constraints -- Schemes Without Bounds

**What goes wrong:**
When a function like `fn show<T>(x :: T) -> String where T: Display` is generalized into a `Scheme`, the scheme becomes `forall T. T -> String`. But the current `Scheme` struct has no field for constraints:
```rust
pub struct Scheme {
    pub vars: Vec<TyVar>,
    pub ty: Ty,
}
```
The where-clause constraints are stored separately in `FnConstraints` and looked up by function name during call inference. This works for direct calls but fails when:
- The function is passed as a value: `let f = show` -- `f`'s scheme has no constraints
- The function is returned from another function
- The function is used in a higher-order context: `map(list, show)` -- `map` receives a `forall T. T -> String` without knowing about the Display constraint

**Why it happens:**
The FnConstraints system was designed for where-clause enforcement at known call sites. It uses function names as keys. When a function becomes a value (closure, partial application, argument), the association between the function and its constraints is lost.

**How to avoid:**
1. Extend `Scheme` to carry constraints:
   ```rust
   pub struct Scheme {
       pub vars: Vec<TyVar>,
       pub constraints: Vec<(TyVar, String)>,  // (var, trait_name)
       pub ty: Ty,
   }
   ```
2. When instantiating a scheme, carry forward the constraints on the fresh type variables
3. Discharge constraints when the fresh variables are unified with concrete types
4. This is the standard "qualified types" approach from Haskell (Mark P. Jones, "Qualified Types: Theory and Practice", 1994)
5. For v1.3, if higher-order constrained functions are out of scope, document this limitation explicitly and add a check that constrained functions cannot be passed as values

**Warning signs:**
- Passing a constrained function as an argument to a higher-order function drops the constraint
- `let f = show` followed by `f(unconstrained_value)` compiles when it should not
- `map(list, to_string)` works without checking that list elements implement Display
- Type errors only appear at the final call site, not when the constrained function is captured

**Phase to address:** Phase 1 (inference integration). Even if full qualified types are deferred, the limitation must be understood and documented.

---

### Pitfall 7: No Duplicate Impl Detection -- Silent Overwrite

**What goes wrong:**
Without overlap checking, a user can write two conflicting impls:
```
impl Display for Int do fn to_string(self) do "number" end end
impl Display for Int do fn to_string(self) do "integer" end end
```
The current `register_impl` in `TraitRegistry` uses `self.impls.insert((trait_name, type_key), impl_def)` -- HashMap insert silently replaces the previous entry. No error is reported. The program compiles and uses whichever impl was registered last.

**Why it happens:**
The `impls` HashMap is keyed by `(trait_name, type_key)`. Inserting with the same key replaces the old value without warning. There is no overlap check.

**How to avoid:**
1. Before inserting an impl, check if one already exists for the same `(trait_name, type_key)` pair
2. If a duplicate exists, emit a `DuplicateImpl` error with both locations
3. For generic impls (once supported), check for overlap: `impl<T> Display for List<T>` overlaps with `impl Display for List<Int>`. Decide whether to allow specialization (complex) or reject overlap (simple, recommended for v1.3)
4. Plan the coherence story for when Snow gets modules/packages: will Snow have an orphan rule?

**Warning signs:**
- Duplicate impls compile without warning
- Changing the order of impl blocks changes program behavior
- The stdlib provides `impl Display for Int` and a user also writes one -- no error, unpredictable behavior
- Tests pass or fail depending on HashMap iteration order

**Phase to address:** Phase 1 (trait infrastructure) -- must detect duplicates before codegen. This is a one-line fix: check before insert.

---

## Moderate Pitfalls

These cause delays, technical debt, or subtle bugs that surface after initial development.

---

### Pitfall 8: Monomorphization Code Bloat with Stdlib Protocols

**What goes wrong:**
If Display is implemented for 15 types, and every function taking `T: Display` is monomorphized for each, you get 15 copies per generic function. With Display + Iterator + From/Into + Hash + Default across 15 types, this means up to 75 specialized copies per generic function that uses multiple bounds. For a stdlib with 10 generic utility functions, that is 750 specialized functions before user code even starts.

**Why it happens:**
Monomorphization is inherently multiplicative. Each generic function is copied for each unique type instantiation. When multiple generic parameters each have multiple concrete types, the count is the product. Rust mitigates this with LLVM's `mergefunc` pass and pre-monomorphization MIR optimizations, but these are complex to implement.

**How to avoid:**
1. Be strategic about which stdlib protocols are truly generic vs. which can use a shared representation. `Display.to_string` always returns `String` -- the method body differs per type but call-site wrappers can share code
2. Factor non-generic parts out of generic functions. If `fn print_all<T: Display>(list :: List<T>)` has 50 lines of list traversal and 1 line of `to_string` call, factor the traversal into a non-generic helper
3. Enable LLVM's `mergefunc` pass to deduplicate identical function bodies post-monomorphization
4. Set compile-time and binary-size budgets. Track metrics as protocols are added
5. Consider a hybrid approach: monomorphize hot paths, use indirect calls for cold paths

**Warning signs:**
- Binary size jumps 2-5x when stdlib protocols are added
- Compilation time increases noticeably for small programs using multiple protocols
- LLVM optimization passes consume >80% of compile time
- Object file sizes grow linearly with the number of protocol implementations

**Phase to address:** Phase 2 (monomorphization) and Phase 4 (stdlib protocols) -- monitor continuously.

---

### Pitfall 9: Self Type Is Not Represented in the Type System

**What goes wrong:**
Trait method signatures that return `Self` cannot be expressed with the current type system. The `TraitMethodSig` stores `return_type: Option<Ty>`, but there is no `Ty::SelfType` variant. This means traits like Clone, Default, and From cannot express their return types correctly:
```
interface Clone do
  fn clone(self) -> Self   # Self = the implementing type
end

interface Default do
  fn default() -> Self     # Self, no self parameter
end
```
Without Self, these methods' return types must be `None` (unspecified) or hardcoded per impl, losing the generic guarantee that `Clone.clone` returns the same type as its receiver.

**Why it happens:**
The Ty enum has: `Var`, `Con`, `Fun`, `App`, `Tuple`, `Never`. There is no `Self` variant because the compiler-known traits (Add, Eq, Ord) either use `None` for return type (relying on it being the same as the operand type through other inference) or use concrete types (Bool for Eq/Ord).

**How to avoid:**
1. Add a sentinel type: either `Ty::SelfType` or a well-known type variable (e.g., `TyVar(u32::MAX)`) that gets substituted with the implementing type during impl processing
2. When registering a trait, methods with `-> Self` return types store the sentinel
3. When resolving a trait method call on concrete type T, substitute Self with T
4. When type-checking an impl, verify that the method's actual return type matches Self (the impl type)
5. For v1.3, if Self is too complex, start with protocols that do not use Self returns: Display (returns String), Iterator (returns Option), Hash (returns Int). Defer Clone and Default to v1.4.

**Warning signs:**
- `Clone.clone(42)` infers `?0` or `Never` instead of `Int`
- `Default.default()` cannot infer its return type at all
- Return types of Self-returning methods cause unification failures
- Method signatures in trait definitions and impl blocks diverge on return type

**Phase to address:** Phase 1 if Clone/Default are in v1.3 scope; Phase 3 (stdlib protocols) if they are deferred.

---

### Pitfall 10: Iterator Protocol Requires Lazy Evaluation or Associated Types -- Neither Exists Yet

**What goes wrong:**
A meaningful Iterator protocol requires two features Snow currently lacks:

1. **Associated types** (or type parameters on traits): `Iterator` must specify what type `next()` returns. In Rust: `type Item; fn next(&mut self) -> Option<Self::Item>`. Without associated types, the Iterator trait cannot express the element type.

2. **Lazy evaluation infrastructure**: Iterator implies pull-based, lazy evaluation where `next()` produces one element at a time. Snow's current collections (List, Map, Set) use eager C runtime functions that process entire collections. Making Iterator lazy requires closure-based state machines or coroutines -- significant runtime work.

**Why it happens:**
Iterator is not just a trait -- it is an evaluation strategy combined with a type-level feature (associated types). Languages that have Iterator as a core protocol (Rust, Python, Java) built both associated types and lazy evaluation into their design from the start.

**How to avoid:**
1. For v1.3, consider whether Iterator should be a trait or a convention. If Snow already has `List.map`, `List.filter`, `List.reduce` as stdlib module functions, converting them to trait methods on an Iterator protocol adds complexity without clear user benefit
2. If Iterator IS in scope, decide between:
   - **Type parameter on trait:** `interface Iterator<T> do fn next(self) -> Option<T> end` -- works with current infrastructure but means `List implements Iterator<Int>`, `List<String> implements Iterator<String>`, etc.
   - **Associated type:** `interface Iterator do type Item; fn next(self) -> Option<Item> end` -- cleaner but requires new type system infrastructure
3. If lazy evaluation is not feasible for v1.3, consider an eager iterator that wraps collections with an index. This is simpler but breaks user expectations about lazy chaining
4. Defer Iterator to v1.4 and focus v1.3 on simpler protocols: Display, From/Into, Hash, Default, Eq/Ord

**Warning signs:**
- Iterator protocol is designed but cannot express the element type safely
- Users expect lazy chaining (`list |> iter() |> filter(f) |> map(g) |> collect()`) but get eager full-collection evaluation
- Performance is worse than existing direct collection functions
- Iterator state management interacts poorly with the GC

**Phase to address:** Phase 3 (stdlib protocol design) -- decide the evaluation strategy and associated type approach BEFORE designing the Iterator interface.

---

### Pitfall 11: From/Into Blanket Impl Requires Infrastructure That Does Not Exist

**What goes wrong:**
The standard From/Into pattern uses a blanket impl: implement `From<A> for B`, get `Into<B> for A` automatically via:
```
impl<A, B> Into<B> for A where B: From<A> do
  fn into(self) -> B do B.from(self) end
end
```
This is a second-order generic impl (quantifies over two type parameters with a trait bound). The current trait registry cannot:
- Store impls parameterized over multiple type variables
- Resolve transitive trait satisfaction (checking Into requires checking From)
- Handle the direction reversal (Into<B> for A is derived from From<A> for B)

**Why it happens:**
Blanket impls are the most powerful and most complex feature of trait systems. They require the impl resolver to search for transitive satisfaction rather than direct lookup. The current resolver does exact (trait_name, type_key) matching only.

**How to avoid:**
1. For v1.3, skip user-defined blanket impls entirely. This is a reasonable scope cut.
2. For From/Into specifically, hard-code the blanket impl in the compiler: when checking `has_impl("Into", target_ty)`, also check if `has_impl("From", source_ty)` and synthesize the Into impl. This is similar to how compiler-known traits are handled in builtins.rs.
3. Alternatively, provide only From (no auto-Into). Users write `B.from(a)` instead of `a.into()`. This is less ergonomic but avoids the blanket impl entirely.
4. Document blanket impls as a v1.4+ feature requiring a more sophisticated trait resolver.

**Warning signs:**
- Attempting to implement blanket impls causes the trait resolver to loop or crash
- Users expect `From` to auto-provide `Into` (as in Rust) and are confused when it does not
- The trait resolver grows increasingly complex to handle transitive chains
- Compile times increase significantly with transitive resolution

**Phase to address:** Phase 3 (stdlib protocol design) -- make the From/Into design decision early.

---

### Pitfall 12: Compiler-Known Traits and User Traits Use Different Dispatch Paths

**What goes wrong:**
Compiler-known traits (Add, Eq, Ord, etc.) are registered in `builtins.rs` and used through special-case inference code in `infer_binary`. User-defined traits go through the general TraitRegistry path. These two paths can diverge:
- User implements `Add` for their custom struct, but binary `+` uses hardcoded logic that only works for Int/Float
- User defines a trait named `Add` -- what happens? Does it shadow the compiler-known Add?
- Error messages differ between compiler-known and user-defined trait violations
- The compiler-known path does not go through TraitRegistry for dispatch at all in some cases

**Why it happens:**
The compiler-known traits predate the user trait system. Binary operator inference has its own type resolution logic (checking for Int or Float operands directly) alongside the TraitRegistry path. The two paths were never unified.

**How to avoid:**
1. Unify the dispatch paths: binary operator inference should use TraitRegistry for ALL trait lookups, including compiler-known ones. The builtins module only registers impls; the inference engine uses one resolution path for both `1 + 2` (Add) and `to_string(42)` (Display)
2. Prevent user traits from reusing compiler-known trait names (Add, Sub, Mul, Div, Mod, Eq, Ord, Not) -- or explicitly document that these are extensible
3. Test: `impl Add for MyStruct { fn add(self, other) do ... end }` then `my_a + my_b` should work through TraitRegistry resolution
4. This also prepares for future operator overloading: if `+` always dispatches through Add trait, user types get operators by implementing traits

**Warning signs:**
- `impl Add for MyType` is registered but `my_value + my_value` still fails with "expected Int"
- Binary operators work only for Int/Float, never for user-defined types
- Different error formats for "type doesn't support +" vs. "trait Display not satisfied"
- Adding a user type to a compiler-known trait changes nothing because the inference code short-circuits

**Phase to address:** Phase 1 (trait infrastructure) -- unify before adding more protocols. This also enables user-defined operator overloading as a natural consequence.

---

### Pitfall 13: No Trait Method Body Tracking in MIR Lowering Name Mangling

**What goes wrong:**
When impl methods are lowered to MIR functions, they need unique mangled names that encode the trait, method, and implementing type. The mangling scheme must be:
- **Deterministic:** Same input always produces the same name
- **Unique:** No collisions between different trait/method/type combinations
- **Decodable:** Error messages and debugging can map mangled names back to source
- **Compatible with LLVM:** Valid LLVM function name (no special characters that LLVM rejects)

If the mangling scheme is ad-hoc, collisions or lookup failures occur.

**Why it happens:**
Rust has a well-defined symbol mangling scheme (RFC 2603, "v0 mangling"). Snow does not have one yet. The current MIR function names are just the source-level function names or simple transformations (e.g., `helper`, `main`, `__closure_0`). Trait methods need a systematic scheme.

**How to avoid:**
1. Define a mangling scheme before implementing trait codegen. Suggested format: `{TraitName}__{MethodName}__{TypeName}`, e.g., `Display__to_string__Int`
2. For generic types, include type arguments: `Display__to_string__List_Int`
3. Reserve the separator (`__`) to avoid collisions with user-defined function names (which should not contain `__`)
4. Store a mangled->source mapping for error messages and debugging
5. Ensure the monomorphization pass uses the same mangling scheme to avoid name drift

**Warning signs:**
- LLVM reports duplicate symbol errors
- Trait methods from different impls get the same mangled name
- Debugger shows mangled names with no way to decode them
- Linking fails with "multiply defined symbol"

**Phase to address:** Phase 2 (codegen integration) -- define the scheme before writing any lowering code for impl methods.

---

## Technical Debt Patterns

| Shortcut | Immediate Benefit | Long-term Cost | When Acceptable |
|----------|-------------------|----------------|-----------------|
| String-based type keys for impl lookup | Simple HashMap lookup, works for all current impls (Int, Float, etc.) | Cannot handle generic impls for parameterized types; must be rewritten before any `impl<T>` works | Only during initial phase where all impls are for monomorphic primitive types |
| No `constraints` field on Scheme | Simpler scheme representation, less plumbing through instantiate/generalize | Cannot handle constrained functions as values; higher-order generic programming silently drops bounds | Acceptable for v1.3 if constrained functions are never passed as values |
| Hardcoded compiler-known trait dispatch | Binary operators work immediately without general trait resolution | User-defined operator overloading requires separate code path; dispatch diverges | Only during initial integration; must be unified before v1.3 release |
| Skipping impl bodies during MIR lowering | Compilation proceeds for programs without trait method calls | All trait method calls produce no code; silent miscompilation at runtime | Never -- must be addressed before any trait method call tests pass |
| No overlap detection in register_impl | Simpler registration code, one less error to handle | Silent overwrite of impls; nondeterministic behavior in programs with duplicate impls | Only during development/testing with controlled impl sets |
| Eager where-clause checking at call site | Catches most violations correctly for direct calls | Fails for deferred type resolution, pipe chains, higher-order usage | Acceptable for v1.3 if pipe chains with constrained generics are tested and working |
| No Self type in Ty enum | Simpler type representation, fewer special cases | Cannot express Clone, Default, or any trait with Self-returning methods | Acceptable if v1.3 protocols avoid Self returns (Display, Hash, Eq are fine) |
| From without auto-Into | Avoids blanket impl complexity entirely | Users must write `B.from(a)` instead of `a.into()`; less ergonomic than Rust | Acceptable for v1.3; add blanket Into in v1.4 |
| Eager Iterator (not lazy) | Works with existing collection runtime functions | Breaks user expectations about lazy chaining; defeats Iterator's primary purpose | Only if Iterator is explicitly documented as eager in v1.3 |
| Single-file coherence only | No need for cross-module orphan rules | Cannot scale to multi-file projects with third-party impls | Acceptable while Snow is single-file; must be revisited for packages |

---

## Performance Traps

| Trap | Symptoms | Prevention | When It Breaks |
|------|----------|------------|----------------|
| Monomorphizing every protocol method for every type | Binary size 5-10x larger, compile time 3-5x longer | Factor out non-generic code; use LLVM `mergefunc` pass; limit protocol impl count | When stdlib has >5 protocols and >10 implementing types |
| Unification table growth with many trait-constrained calls | Type inference slows on programs with >50 generic function calls | Reuse type variables; compact union-find table between top-level items | When programs use protocols heavily in chains or loops |
| LLVM optimization on duplicated monomorphized IR | LLVM passes take >80% of compile time | Run MIR-level optimizations before LLVM lowering; share identical function bodies | When monomorphized function count exceeds ~500 |
| Recursive trait resolution for composite types | Trait checker enters deep recursion or loops | Add resolution depth limit; cache (trait, type) results | When composite types implement Display by delegating to field Display impls |
| GC pressure from iterator state allocations | Programs with many iterators create many small heap allocations | Pool iterator state; use arena allocation for short-lived iterators | When Iterator protocol is used in tight loops |
| Name mangling string operations | String allocation for mangled names during compilation | Use interned strings or symbol table; avoid repeated allocation | When monomorphization creates thousands of specialized function names |

---

## "Looks Done But Isn't" Checklist

These items commonly pass basic tests but fail in real-world usage. Verify each before declaring a phase complete:

- [ ] **Generic impl lookup works:** `impl Display for List<T>` resolves for `List<Int>`, `List<String>`, etc. -- not just string-key exact match
- [ ] **Where-clause checking is order-independent:** `f(g(x))` where both f and g have where-clauses works regardless of inference order
- [ ] **Trait methods appear in compiled output:** After lowering, `to_string(42)` produces a real function call in LLVM IR, not a dangling reference
- [ ] **Method dispatch is deterministic:** Two traits with same method name on same type produces a compile error, not silent first-match
- [ ] **Monomorphization terminates:** Recursive generic functions hit a depth limit with a clear error message
- [ ] **Compiler-known traits are extensible:** `impl Add for MyType` followed by `my_a + my_b` works through TraitRegistry
- [ ] **Trait method bodies type-check with correct Self:** `self.field` resolves correctly inside impl methods for structs
- [ ] **Duplicate impls are rejected:** `impl Display for Int` written twice produces a DuplicateImpl error
- [ ] **Schemes carry constraints (or the limitation is documented):** `let f = show` either preserves the Display constraint or produces a "cannot capture constrained function" error
- [ ] **Self return type works (if in scope):** `Clone.clone(42)` returns Int, not a type variable
- [ ] **Pipe chains with trait methods work:** `42 |> to_string()` produces the same result as `to_string(42)`
- [ ] **Error messages name the trait:** "Int does not implement Display" not just "type mismatch"
- [ ] **Stdlib protocols don't conflict:** Display and Debug can coexist on the same type without method ambiguity
- [ ] **Mangled names are unique:** `Display__to_string__Int` and `Display__to_string__Float` are distinct in LLVM module
- [ ] **Trait methods are reachable in mono pass:** The monomorphization reachability pass includes impl method functions, not just user-defined functions

---

## Pitfall-to-Phase Mapping

| Phase | Pitfall IDs | Summary |
|-------|-------------|---------|
| **Phase 1: Trait Infrastructure (type system)** | 1, 2, 4, 6, 7, 9, 12 | Fix type_to_key, add overlap detection, unify dispatch paths, handle method collisions, extend Scheme or document constraint limitation, add Self type if needed |
| **Phase 2: Monomorphization + Codegen** | 3, 5, 8, 13 | Implement real monomorphization with depth limit, lower impl methods to MIR, define name mangling, handle trait method calls in codegen |
| **Phase 3: Stdlib Protocol Design** | 10, 11 | Decide Iterator evaluation strategy, decide From/Into blanket impl approach, choose which protocols to include |
| **Phase 4: Stdlib Protocol Implementation** | 8, 10 | Monitor code bloat, implement chosen protocol strategies, test with real programs |

### Phase 1 Priority Order (Dependency-Driven)

1. **Pitfall 7** (overlap detection) -- one-line fix, prevents silent bugs immediately
2. **Pitfall 2** (type_to_key rewrite) -- everything downstream depends on correct generic impl lookup
3. **Pitfall 4** (method name collision) -- must be resolved before stdlib protocols share method names
4. **Pitfall 12** (unify compiler-known + user traits) -- required for user operator overloading
5. **Pitfall 9** (Self type) -- needed if Clone/Default are in v1.3 scope
6. **Pitfall 6** (Scheme constraints) -- needed for higher-order trait usage; can be deferred with documented limitation
7. **Pitfall 1** (deferred constraint checking) -- refinement for edge cases in pipe chains and nested generics

### Phase 2 Priority Order (Dependency-Driven)

1. **Pitfall 13** (name mangling scheme) -- define before writing any impl lowering code
2. **Pitfall 5** (MIR gap) -- without this, nothing with traits compiles to working native code
3. **Pitfall 3** (infinite monomorphization guard) -- must be in the worklist from day one
4. **Pitfall 8** (code bloat monitoring) -- track metrics, optimize if thresholds are exceeded

---

## Sources

### Snow Codebase Analysis (HIGH confidence)
- `crates/snow-typeck/src/traits.rs` -- TraitRegistry, type_to_key, register_impl, has_impl
- `crates/snow-typeck/src/infer.rs` -- infer_interface_def, infer_impl_def, where-clause checking, FnConstraints
- `crates/snow-typeck/src/unify.rs` -- InferCtx, unification, generalization, scheme instantiation
- `crates/snow-typeck/src/ty.rs` -- Ty enum, Scheme struct (no constraints field)
- `crates/snow-typeck/src/builtins.rs` -- compiler-known trait registration, operator dispatch
- `crates/snow-typeck/tests/traits.rs` -- existing trait test coverage
- `crates/snow-codegen/src/mir/mono.rs` -- current monomorphization (reachability only)
- `crates/snow-codegen/src/mir/mod.rs` -- MIR types, no trait method call node
- `crates/snow-codegen/src/mir/lower.rs` -- skips interface/impl at line 431
- `crates/snow-codegen/src/codegen/mod.rs` -- LLVM codegen, function compilation

### Established PL Research (HIGH confidence)
- [Coherence of Type Class Resolution (Bottu et al.)](https://xnning.github.io/papers/coherence-class.pdf) -- formal coherence, ambiguity
- [Let Should Not Be Generalised (Vytiniotis et al.)](https://www.microsoft.com/en-us/research/wp-content/uploads/2016/02/tldi10-vytiniotis.pdf) -- deferred constraints, ambiguity problem
- [Demystifying Type Classes (Kiselyov)](https://okmij.org/ftp/Computation/typeclass.html) -- monomorphization vs dictionary passing
- [Rust RFC 0195: Associated Items](https://rust-lang.github.io/rfcs/0195-associated-items.html) -- input vs output type parameters
- [Functional Dependencies (GHC)](https://ghc.gitlab.haskell.org/ghc/doc/users_guide/exts/functional_dependencies.html) -- ambiguity in multi-param type classes

### Rust/LLVM Ecosystem (HIGH confidence)
- [Rust compile time analysis (TiDB)](https://www.pingcap.com/blog/reasons-rust-compiles-slowly/) -- monomorphization costs
- [Rust compiler March 2025 improvements](https://nnethercote.github.io/2025/03/19/how-to-speed-up-the-rust-compiler-in-march-2025.html) -- pre-mono MIR optimizations
- [LLVM: The bad parts (2026)](https://www.npopov.com/2026/01/11/LLVM-The-bad-parts.html) -- LLVM design issues
- [The dark side of inlining and monomorphization](https://nickb.dev/blog/the-dark-side-of-inlining-and-monomorphization/) -- code bloat analysis
- [Monomorphization: Casting out Polymorphism](https://thunderseethe.dev/posts/monomorph-base/) -- implementation patterns
- [Recursive monomorphization in Rust (issue #50043)](https://github.com/rust-lang/rust/issues/50043) -- infinite instantiation
- [Rust orphan rules](https://github.com/Ixrec/rust-orphan-rules) -- coherence design space
- [Rust coherence RFC 2451](https://rust-lang.github.io/rfcs/2451-re-rebalancing-coherence.html) -- re-rebalancing coherence
- [Tour of Rust's Standard Library Traits](https://github.com/pretzelhammer/rust-blog/blob/master/posts/tour-of-rusts-standard-library-traits.md) -- trait design patterns

---
*Pitfalls research for: Snow v1.3 trait system and stdlib protocols*
*Researched: 2026-02-07*
