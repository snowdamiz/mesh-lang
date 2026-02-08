# Stack Research: v1.3 Trait System & Stdlib Protocols

**Project:** Snow Programming Language -- v1.3 Milestone
**Domain:** Compiler trait system completion, monomorphization, stdlib protocol implementation
**Researched:** 2026-02-07
**Confidence:** HIGH

## Scope

This research covers ONLY the stack changes needed for v1.3: completing the trait system (user-defined traits, monomorphization, method dispatch) and adding stdlib protocols (Display, Iterator, From/Into, Serialize/Deserialize, Hash, Default). The existing compiler stack (lexer, parser, type checker, MIR, codegen, runtime) is validated and out of scope.

---

## Key Finding: No New Dependencies Needed

The v1.3 trait system and stdlib protocols require **zero new Rust crate dependencies**. Everything needed is already in the workspace or is pure compiler logic that should be implemented directly. This is the correct outcome -- trait systems and protocols are core compiler semantics, not library-shaped problems.

---

## Current Stack (Validated, No Changes)

These are already in `Cargo.toml` and require no version changes:

| Technology | Version | Crate | Status |
|------------|---------|-------|--------|
| Inkwell | 0.8.0 | `snow-codegen` | Current (supports LLVM 21, latest on GitHub) |
| LLVM | 21 | via Inkwell `llvm21-1` feature | Current |
| ena | 0.14 | `snow-typeck` | Current (latest is 0.14.3, semver-compatible) |
| rustc-hash | 2 | `snow-typeck`, `snow-codegen` | Current (latest is 2.1.1, semver-compatible) |
| rowan | 0.16 | `snow-parser` | Current (latest is 0.16.1, semver-compatible) |
| ariadne | 0.6 | `snow-typeck` | Current |
| serde/serde_json | 1 | `snow-typeck`, `snow-rt` | Current |
| insta | 1.46 | dev-dependency | Current |

**Confidence: HIGH** -- Versions verified via crates.io web search on 2026-02-07. All workspace dependencies are at their latest compatible versions.

---

## What Changes Are Needed (All Internal)

### 1. snow-typeck: TraitRegistry Extensions

**No new dependencies.** The existing `TraitRegistry` (traits.rs) needs extension, not replacement.

**Current state:** TraitRegistry stores `TraitDef` and `ImplDef` keyed by `(trait_name, type_key)`. Compiler-known traits (Add, Eq, Ord, etc.) are registered in builtins.rs with built-in impls for primitives. Where clauses validate against registry.

**What needs to change internally:**

| Change | Why | Impact |
|--------|-----|--------|
| `TraitDef` gains `default_methods: Vec<(String, ...)>` | Support default method implementations in interfaces | Small struct change in traits.rs |
| `ImplDef` gains method body references | Codegen needs to find impl method bodies for monomorphization | Link from ImplDef to CST nodes or MIR bodies |
| `TraitRegistry` gains `impls_for_type(ty) -> Vec<&ImplDef>` | Codegen needs all impls for a given type to generate monomorphized methods | New query method |
| `TraitRegistry` gains `impls_for_trait(name) -> Vec<&ImplDef>` | Stdlib protocols need to enumerate all implementors | New query method |
| `type_to_key` handles generic impls | `impl Display for Option<T>` requires matching against parameterized types | Extend existing function |

**Tools used:** Direct Rust code in existing crate. Uses existing `ena` for unification, existing `rustc-hash` for FxHashMap.

### 2. snow-codegen/mir: Monomorphization Pass

**No new dependencies.** The existing `mono.rs` is a reachability-only pass that needs to become a real monomorphization pass.

**Current state:** `monomorphize()` in `mir/mono.rs` only does dead function elimination (reachability analysis). Comment says "In future: creates specialized copies of generic functions for each concrete type instantiation." The MIR lowerer in `lower.rs` currently lowers impl methods as standalone functions (`self.lower_fn_def(&method)`) but does not mangle names by implementing type.

**What needs to change internally:**

| Change | Why | Impact |
|--------|-----|--------|
| Name mangling for trait methods | `impl Display for Int` produces `Display_to_string_Int` | Extend existing `mangle_type_name` in mir/types.rs |
| MIR gains `TraitMethodCall` variant | Distinguish `obj.method()` from `free_fn(obj)` for dispatch | New variant in MirExpr enum |
| Monomorphization collects trait method instantiations | When `Display.to_string` is called on `Int`, ensure `Display_to_string_Int` exists | Extend mono.rs worklist |
| MIR lowerer resolves trait methods to concrete impls | At call sites, resolve `self.to_string()` to the mangled concrete function | Extend lower.rs |
| Codegen emits monomorphized trait method bodies | LLVM functions for each (trait, type) combination | Extend codegen/expr.rs |

**Architecture decision (already validated):** Static dispatch via monomorphization. No vtables. This is correct because:
1. Snow uses static typing with HM inference -- concrete types are known at compile time
2. LLVM benefits from direct calls (enables inlining, branch prediction)
3. Actor system provides dynamic routing where needed (message dispatch is inherently dynamic)
4. Binary size trade-off is acceptable for a compiled language targeting servers/CLI

**Name mangling scheme:** Use the existing `mangle_type_name` pattern from `mir/types.rs`:
- `Display_to_string_Int` for `impl Display for Int`
- `Iterator_next_List_Int` for `impl Iterator for List<Int>`
- `From_from_String_Int` for `impl From<String> for Int`

### 3. snow-codegen/codegen: LLVM IR for Trait Methods

**No new dependencies.** Trait methods compile to regular LLVM functions with mangled names.

**Current state:** Codegen skips `InterfaceDef` ("interfaces are erased") and lowers `ImplDef` methods as standalone functions. The `CodeGen` struct already has `functions: FxHashMap<String, FunctionValue>` which maps MIR function names to LLVM function values.

**What needs to change internally:**

| Change | Why | Impact |
|--------|-----|--------|
| Codegen registers mangled trait method names | `Display_to_string_Int` must be in `functions` map | Extend function registration pass |
| Call sites resolve to mangled names | `x.to_string()` where `x: Int` becomes call to `Display_to_string_Int` | Extend expr.rs call handling |
| `self` parameter handling | Trait methods receive `self` as first argument (value, not reference) | Straightforward -- same as existing function params |

### 4. snow-rt: Runtime Support for Stdlib Protocols

**No new dependencies.** Runtime functions for stdlib protocols are implemented as `#[no_mangle] extern "C"` functions in the existing snow-rt crate, same pattern as all other stdlib functions.

**Current approach (validated):** Every stdlib function is a Rust function with C ABI exported from snow-rt, declared as an LLVM extern in codegen/intrinsics.rs, and registered in builtins.rs for type checking. This pattern works and scales.

**New runtime functions needed:**

| Protocol | Runtime Functions | Crate Impact |
|----------|-------------------|--------------|
| Display | `snow_display_int`, `snow_display_float`, `snow_display_bool`, `snow_display_string` | Trivial -- format to String |
| Iterator | `snow_iterator_*` for List, Range, Map | Uses existing collection internals |
| From/Into | Pure type-level, no runtime support needed | Zero runtime impact |
| Hash | `snow_hash_int`, `snow_hash_float`, `snow_hash_string`, `snow_hash_bool` | See hash algorithm section below |
| Default | `snow_default_int` (0), `snow_default_float` (0.0), etc. | Trivial |
| Serialize | `snow_serialize_*` to JSON string | Uses existing serde_json in snow-rt |
| Deserialize | `snow_deserialize_*` from JSON string | Uses existing serde_json in snow-rt |

### 5. Hash Algorithm for Snow's Hash Protocol

**No new dependency needed.** Use FNV-1a, implemented directly in snow-rt (~30 lines of Rust).

**Why FNV-1a (not SipHash, not the `rustc-hash` FxHash):**

| Algorithm | Speed (short keys) | Speed (long keys) | HashDoS resistant | Complexity |
|-----------|--------------------|--------------------|-------------------|------------|
| FNV-1a | Very fast | Moderate | No | ~30 lines |
| SipHash | Moderate | Fast | Yes | ~200 lines |
| FxHash | Fastest | Poor | No | ~50 lines |

- **FNV-1a is the right choice** because Snow's Hash protocol is for user-visible hashing of values (struct fields, map keys), not for the compiler's internal hash maps (which already use FxHash via rustc-hash).
- Snow is a compiled language where the input domain is controlled (not a web server accepting arbitrary user keys), so HashDoS resistance is unnecessary at the language level.
- The Hash protocol implementations for Snow value types (Int, Float, String, Bool, structs) hash known-size data. FNV-1a excels at this.
- FNV-1a is trivial to implement inline -- no crate needed.

**Implementation:** A single `snow_hash` function in snow-rt that takes a pointer and length, returns i64. Protocol implementations for each type call it with appropriate byte representations. Approximately 30 lines of Rust:

```rust
const FNV_OFFSET: u64 = 14695981039346656037;
const FNV_PRIME: u64 = 1099511628211;

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET;
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}
```

**Why not add the `fnv` crate:** The fnv crate (crates.io) provides a Hasher implementation, but Snow's runtime doesn't need `std::hash::Hasher` compatibility. A direct FNV-1a implementation is simpler, has zero dependencies, and avoids pulling in a crate for 30 lines of code.

**Why not add the `ahash` crate:** ahash is excellent for Rust hash maps but requires hardware AES instructions for its speed advantage, which makes it platform-dependent. Snow's Hash protocol should produce deterministic, platform-independent hashes.

### 6. Serialization for Snow's Serialize/Deserialize Protocols

**No new dependency needed.** Use existing serde_json (already in snow-rt's Cargo.toml).

**Design approach:** Snow's Serialize/Deserialize protocols produce/consume JSON strings, not arbitrary formats. This is the right scope for v1.3 because:
1. JSON is the only serialization format Snow's stdlib currently supports (json.rs in snow-rt)
2. A Serde-style multi-format data model is premature -- Snow doesn't have derive macros or procedural macros yet
3. JSON covers the server-oriented use case (HTTP APIs, config files)

**Runtime implementation:**
- `snow_serialize_*` functions convert Snow values to JSON strings (uses serde_json::Value internally)
- `snow_deserialize_*` functions parse JSON strings into Snow values (uses serde_json::from_str internally)
- Struct serialization: compiler generates a serialize function per struct that walks fields and builds a JSON object
- This matches the existing pattern in json.rs where `json_encode_map`, `json_encode_list`, etc. already exist

**Why not add a separate serialization framework:** Snow is not Rust. It doesn't need a Serde-style visitor pattern because it doesn't have derive macros. The compiler itself generates the serialization code for each type via monomorphization of the Serialize trait. The runtime just needs leaf functions (`serialize_int_to_json`, `serialize_string_to_json`, etc.) that the generated code calls.

---

## Recommended Stack (Complete)

### Core Technologies (No Changes)

| Technology | Version | Purpose | Status for v1.3 |
|------------|---------|---------|------------------|
| Inkwell | 0.8.0 (`llvm21-1`) | LLVM IR generation | No changes needed. Monomorphized trait methods are regular LLVM functions. |
| ena | 0.14 | Union-find for type unification | No changes needed. Trait method resolution uses TraitRegistry, not unification. |
| rustc-hash | 2 | FxHashMap for compiler internals | No changes needed. Already used throughout. |
| rowan | 0.16 | Lossless CST | No changes needed. Parser already handles interface/impl syntax. |
| ariadne | 0.6 | Diagnostic reporting | No changes needed. New error types use existing error infrastructure. |
| serde_json | 1 | JSON parsing/generation | Already in snow-rt. Used for Serialize/Deserialize protocol runtime. |

### New Internal Components (No External Dependencies)

| Component | Location | Purpose | Complexity |
|-----------|----------|---------|------------|
| TraitRegistry extensions | snow-typeck/traits.rs | Default methods, impl queries, generic impl matching | Medium |
| MirExpr::TraitMethodCall | snow-codegen/mir/mod.rs | Distinguish trait method calls from free functions | Low |
| Monomorphization pass | snow-codegen/mir/mono.rs | Generate specialized functions per (trait, concrete_type) | High |
| Trait method name mangling | snow-codegen/mir/types.rs | Extend existing mangle_type_name | Low |
| Trait method dispatch in codegen | snow-codegen/codegen/expr.rs | Resolve trait calls to mangled LLVM functions | Medium |
| FNV-1a hash implementation | snow-rt/src/hash.rs (new file) | Runtime hash function for Hash protocol | Low (~30 lines) |
| Display runtime functions | snow-rt/src/string.rs (extend) | to_string for primitive types | Low |
| Iterator runtime support | snow-rt/src/collections/ (extend) | Iterator state management for List, Range | Medium |
| Serialize/Deserialize runtime | snow-rt/src/json.rs (extend) | JSON serialization for Snow types | Medium |
| Builtin trait/impl registrations | snow-typeck/builtins.rs | Register Display, Iterator, Hash, etc. as traits with impls | Medium |
| Builtin type signatures | snow-typeck/builtins.rs | Type signatures for protocol methods in type env | Low |

---

## What NOT to Add (and Why)

| Avoid | Why | What to Do Instead |
|-------|-----|-------------------|
| `fnv` crate | 30 lines of code. Adding a crate for a trivial algorithm adds dependency management overhead for no benefit. | Implement FNV-1a directly in snow-rt (~30 lines). |
| `ahash` crate | Requires hardware AES for speed. Snow's Hash protocol needs deterministic, platform-independent results. | Use FNV-1a. Platform-independent, deterministic, fast enough. |
| `siphasher` crate | Overkill for Snow's use case. SipHash protects against HashDoS, which is irrelevant for a compiled language's value hashing. | Use FNV-1a. |
| `serde` derive macros on Snow types | Snow types are not Rust types. The compiler generates serialization code via monomorphization, not via Rust derive macros. | Compiler generates serialize/deserialize functions per type. Runtime provides leaf serialization functions. |
| `erased-serde` or trait object serialization | Dynamic dispatch for serialization is unnecessary -- Snow uses static dispatch via monomorphization. | Monomorphize Serialize/Deserialize impls to concrete functions. |
| vtable/dynamic dispatch infrastructure | Snow's type system resolves all types at compile time. Adding vtables would add runtime cost for no benefit. Actors provide dynamic routing where needed. | Monomorphization only. Static dispatch for all trait method calls. |
| `petgraph` or graph library for trait resolution | Trait dependency graphs in Snow are simple (no diamond inheritance, no higher-kinded types). A HashMap lookup suffices. | Use existing FxHashMap-based TraitRegistry. |
| New MIR crate or separate `snow-mir` | MIR is tightly coupled to codegen. Splitting it out adds crate boundary overhead for code that changes together. | Keep MIR in snow-codegen as it is now. |
| `proc-macro2`/`syn`/`quote` | Snow doesn't have derive macros. Serialization code is generated by the compiler, not by Rust macros. | Compiler generates code directly in MIR lowering. |
| Generic/polymorphic runtime dispatch | Iterator, Display, etc. are resolved at compile time via monomorphization. No runtime trait dispatch needed. | All protocol methods compile to direct function calls. |

---

## Integration Points with Existing Stack

### Type Checker -> MIR Lowering

The `TypeckResult` already contains `trait_registry: TraitRegistry`. The MIR lowerer has access to this. The integration point is:

1. **Type checker** resolves which concrete impl satisfies a trait method call
2. **MIR lowerer** reads the resolution and generates `MirExpr::Call` with the mangled function name
3. **No trait objects, no vtables** -- by the time MIR is generated, all calls are direct

### MIR Lowering -> Codegen

The MIR lowerer already generates `MirExpr::Call` with function names. Trait methods are lowered as regular functions with mangled names. The codegen sees no difference between a trait method call and a regular function call -- they are both `MirExpr::Call { func: Var("Display_to_string_Int"), ... }`.

### Builtins Registration

New stdlib protocols follow the exact pattern established by compiler-known traits in builtins.rs:

1. Register trait definition: `registry.register_trait(TraitDef { name: "Display", methods: [...] })`
2. Register impls for primitives: `registry.register_impl(ImplDef { trait_name: "Display", impl_type: Ty::int(), ... })`
3. Register type signatures: `env.insert("display_to_string_int", Scheme::mono(...))`

### Runtime Functions

New runtime functions follow the exact pattern in codegen/intrinsics.rs:

1. Declare as LLVM extern: `module.add_function("snow_display_int", fn_type, Some(Linkage::External))`
2. Implement in snow-rt: `#[no_mangle] pub extern "C" fn snow_display_int(val: i64) -> *mut SnowString`

---

## Alternatives Considered

| Decision | Recommended | Alternative | Why Not Alternative |
|----------|-------------|-------------|---------------------|
| Dispatch strategy | Monomorphization (static) | Vtable (dynamic) | Snow resolves all types at compile time via HM inference. Vtables add runtime overhead for zero benefit. Actors handle dynamic routing. |
| Hash algorithm | FNV-1a (inline) | SipHash via crate | HashDoS irrelevant for compiled language values. FNV-1a is faster for short keys (ints, small strings). 30 lines vs. a dependency. |
| Serialization | JSON via existing serde_json | Multi-format (Serde data model) | Premature. Snow has no derive macros. JSON covers the server use case. Can extend later if needed. |
| Name mangling | `Trait_method_Type` | C++ style mangling | C++ mangling is complex and designed for overloading/namespaces Snow doesn't have. Simple underscore joining matches existing `mangle_type_name` pattern. |
| Iterator protocol | Stateful iterator object | Lazy/stream-based | Stateful iterators are simpler to implement and match the existing collection model (List, Range are already eager). Lazy evaluation is a future enhancement. |
| Default method bodies | CST reference from TraitDef | Copy body into each ImplDef | CST references avoid code duplication and let the lowerer handle default methods naturally. |
| Trait method storage | Extend existing TraitRegistry | New TraitMethodTable | TraitRegistry already does everything needed. A new abstraction adds complexity. Extend, don't replace. |

---

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Monomorphization code bloat | Low | Medium | Snow programs are small-to-medium scale. Monitor binary sizes. Can add shared-body optimization later. |
| Name collision in mangled names | Low | Low | Use consistent `Trait_method_Type` scheme. Types are unique in Snow's namespace. |
| Iterator state management complexity | Medium | Medium | Start with List and Range iterators only. Map/Set iterators can follow the same pattern. |
| Serialize/Deserialize for nested structs | Medium | Medium | Compiler generates recursive serialize calls. Test with 3+ levels of nesting. |
| Generic impl matching (e.g., `impl Display for Option<T> where T: Display`) | Medium | High | Defer generic impls to after concrete impls work. Start with `impl Display for Option_Int` etc. |

---

## Sources

### Verified (HIGH confidence)
- [Inkwell 0.8.0 -- GitHub, LLVM 11-21 support](https://github.com/TheDan64/inkwell) -- confirmed latest version
- [rustc-hash 2.1.1 -- crates.io](https://crates.io/crates/rustc-hash) -- confirmed latest version
- [ena 0.14.3 -- crates.io](https://crates.io/crates/ena) -- confirmed latest version
- [rowan 0.16.1 -- crates.io](https://crates.io/crates/rowan) -- confirmed latest version
- [Rust compiler monomorphization -- rustc-dev-guide](https://rustc-dev-guide.rust-lang.org/backend/monomorph.html) -- reference for monomorphization patterns
- [FNV hash function -- IETF draft, Wikipedia](https://en.wikipedia.org/wiki/Fowler%E2%80%93Noll%E2%80%93Vo_hash_function) -- algorithm specification
- [Rust hashing performance -- perf book](https://nnethercote.github.io/perf-book/hashing.html) -- FNV vs SipHash vs FxHash comparison

### Verified (MEDIUM confidence)
- [Serde data model -- serde.rs](https://serde.rs/) -- reference for serialization trait design
- [Swift Codable protocol](https://developer.apple.com/documentation/swift/codable) -- reference for compiler-generated serialization
- [Static vs dynamic dispatch trade-offs](https://www.slingacademy.com/article/performance-considerations-in-rust-virtual-table-lookups-vs-monomorphization/) -- monomorphization vs vtable analysis

### Codebase Analysis (HIGH confidence)
- `snow-typeck/src/traits.rs` -- existing TraitRegistry, TraitDef, ImplDef structures
- `snow-typeck/src/builtins.rs` -- existing compiler-known trait registration pattern
- `snow-codegen/src/mir/mono.rs` -- existing reachability pass (to be extended)
- `snow-codegen/src/mir/types.rs` -- existing `mangle_type_name` function
- `snow-codegen/src/mir/lower.rs` -- existing impl method lowering (`lower_fn_def`)
- `snow-codegen/src/codegen/intrinsics.rs` -- existing runtime function declaration pattern
- `snow-rt/Cargo.toml` -- serde_json already present

---
*Stack research for: Snow v1.3 Trait System & Stdlib Protocols*
*Researched: 2026-02-07*
