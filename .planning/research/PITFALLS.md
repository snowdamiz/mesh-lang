# Pitfalls Research: Adding Module System to Snow

**Domain:** Compiler feature addition -- file-based module system for a statically-typed, functional-first, LLVM-compiled language with HM inference, monomorphization, and actor concurrency
**Researched:** 2026-02-09
**Confidence:** HIGH (based on direct Snow codebase analysis, OCaml module system precedent, Rust symbol mangling RFC, LLVM multi-module documentation, and established compiler engineering knowledge)

**Scope:** This document covers pitfalls specific to adding a module system (v1.8) to the Snow compiler. Snow currently compiles a single `main.snow` file per project. The compiler has 70,501 lines of Rust across 11 crates, 1,278+ tests, and zero known correctness issues. The module system introduces multi-file compilation with file-based modules, `pub` visibility, qualified/selective imports, dependency graph resolution, and cross-module name resolution.

---

## Critical Pitfalls

Mistakes that cause rewrites, soundness holes, or silent codegen bugs.

---

### Pitfall 1: Type Identity Breaks Across Module Boundaries -- Same Struct Name, Different Types

**What goes wrong:**
Snow's type system identifies types by name. `TypeRegistry` (infer.rs:115-122) stores struct and sum type definitions in `FxHashMap<String, StructDefInfo>` keyed by bare name (e.g., `"Point"`, `"Option"`). When two modules define a struct with the same name, they collide in the registry:

```
# math/vector.snow
struct Point do
  x :: Float
  y :: Float
end

# graphics/pixel.snow
struct Point do
  x :: Int
  y :: Int
end
```

If both modules are loaded into the same `TypeRegistry`, the second `Point` overwrites the first. Code importing from `math.vector` silently gets the `graphics.pixel` `Point` definition. Field access compiles against the wrong layout, producing memory corruption at runtime.

This is not hypothetical. The current `TypeRegistry.register_struct` (infer.rs:129-131) does an unconditional `insert` -- no collision detection:
```rust
fn register_struct(&mut self, info: StructDefInfo) {
    self.struct_defs.insert(info.name.clone(), info);
}
```

**Why it happens:**
The entire compiler assumes a single flat namespace. Every data structure from `TypeEnv` (env.rs) to `TraitRegistry` (traits.rs) to `MirModule` (mir/mod.rs) uses bare string names as keys. This assumption pervades: `MirType::Struct(String)` carries just the name, `mangle_type_name` (mir/types.rs:143) builds mangled names from bare type names, and codegen's `struct_types` cache (codegen/mod.rs:56) maps bare names to LLVM struct types.

**Consequences:**
- Silent memory corruption when two modules define structs with the same name but different layouts
- Type checker accepts code that accesses nonexistent fields
- LLVM GEP instructions access wrong offsets, producing garbage values or segfaults
- Actor message passing between modules using "same-name" structs silently corrupts data

**Prevention:**
All type names must become module-qualified throughout the entire pipeline. The canonical form should be `ModulePath.TypeName` (e.g., `Math.Vector.Point`):

1. **TypeRegistry keys:** Change from `"Point"` to `"Math.Vector.Point"`. Every `register_struct`, `register_sum_type`, `register_alias` call must prefix the module path.

2. **MirType names:** `MirType::Struct("Math.Vector.Point")` and `MirType::SumType("Math.Vector.Option_Int")`. This flows through to `mangle_type_name`, codegen `struct_types`, and LLVM struct type names.

3. **TraitRegistry:** Impls must be qualified: `impl Display for Math.Vector.Point`. The `ImplDef.impl_type_name` field must carry the qualified name.

4. **Backward compatibility:** For single-file programs (no imports), the module path is empty, so type names remain bare. The qualification only adds a prefix for multi-file programs.

5. **Import aliasing:** `import Math.Vector` means `Vector.Point` in user code resolves to `Math.Vector.Point` internally. The name resolution layer translates user-visible names to qualified names before they reach `TypeRegistry`.

**Warning signs:**
- Tests with two modules defining same-named types pass (they should fail or produce distinct types)
- `TypeRegistry.struct_defs.len()` is less than expected after loading multiple modules
- LLVM verification errors about struct field count mismatches

**Detection:**
Add an assertion in `register_struct` that panics if a name collision occurs from different modules. In debug builds, verify that every `MirType::Struct` name contains a module qualifier when compiling multi-file projects.

**Phase to address:**
Name resolution phase. This is the FIRST thing to get right -- every subsequent phase depends on qualified names being correct. Retrofitting qualification after building on bare names requires touching every pipeline stage.

---

### Pitfall 2: Name Mangling Collisions Produce Duplicate LLVM Symbols

**What goes wrong:**
Snow's current name mangling uses patterns like `Trait__Method__Type` (e.g., `Display__to_string__Point`) and `identity_Int` for monomorphized generics. These names become LLVM function names and ultimately linker symbols. When two modules define functions with the same name, or implement the same trait for same-named types, the LLVM symbols collide:

```
# module_a.snow
def helper(x :: Int) -> Int do x + 1 end

# module_b.snow
def helper(x :: Int) -> Int do x * 2 end
```

Both lower to MIR function `helper`, both become LLVM function `@helper`. The LLVM module will reject the duplicate definition, or the linker will silently pick one.

The same problem occurs with trait implementations:
```
# module_a.snow: impl Display for Point
# module_b.snow: impl Display for Point  (different Point)
# Both produce: Display__to_string__Point
```

**Why it happens:**
`MirFunction.name` (mir/mod.rs:44) is a bare string. The MIR lowerer (lower.rs) uses function names directly from the AST without any module prefix. The codegen `declare_functions` (codegen/mod.rs:197) forward-declares all functions by their MIR name, and `functions` cache (codegen/mod.rs:65) maps MIR names to LLVM `FunctionValue`. No module qualification exists at any level.

**Consequences:**
- LLVM module verification fails with "redefinition of symbol"
- If using separate LLVM modules per Snow module and linking, the linker produces "duplicate symbol" errors
- If symbols are made weak/COMDAT, the linker silently picks one, producing wrong behavior

**Prevention:**
Every function name in MIR must be module-qualified. The mangling scheme should be:

```
<module_path>__<function_name>           # regular functions
<module_path>__<Trait>__<Method>__<Type> # trait impls
<module_path>__<function>__<TypeArgs>    # monomorphized generics
<module_path>__closure_<N>              # lifted closures
```

For example:
- `math.vector__add` instead of `add`
- `math.vector__Display__to_string__Point` instead of `Display__to_string__Point`
- `math.vector__identity__Int` instead of `identity_Int`

The module path uses dots or double underscores as separators. Using dots in LLVM symbol names is valid (LLVM allows any character in symbol names when quoted).

**Critical:** The `main` function in the entry module must remain `main` (or `snow_main`) without module qualification, because `generate_main_wrapper` (codegen/mod.rs:205) emits a C `main` that calls it.

**Warning signs:**
- "Redefinition of symbol" errors when compiling multi-module programs
- Linker errors about duplicate symbols
- Wrong function called at runtime (function from module B called instead of module A)

**Phase to address:**
MIR lowering phase. The lowerer must prepend module paths when creating `MirFunction.name`. This cascades through monomorphization and codegen automatically since they use the MIR name.

---

### Pitfall 3: HM Type Inference Context Not Propagated Across Module Boundaries

**What goes wrong:**
Snow's type inference (infer.rs) runs Algorithm J on the AST, building a `TypeEnv` (env.rs) with scope stacks and a `TypeRegistry` for struct/sum type definitions. Currently, `check(parse)` (lib.rs:96) takes a single `Parse` and returns a single `TypeckResult`. There is no mechanism to feed type information from one module into another's inference context.

When module A imports module B, the type checker for module A needs to know:
- What functions B exports, and their type schemes (polymorphic signatures)
- What types B exports (struct defs, sum type defs, type aliases)
- What trait impls B provides
- What trait definitions B provides

Without this information, any reference to `B.some_function()` will produce `NoSuchFunction` errors. The type checker literally cannot see across module boundaries.

**Why it happens:**
The `infer` function (infer.rs) is monolithic -- it processes one `Parse` tree and builds all registries from scratch. There is no "import" step that loads pre-computed type information from another module. The `TypeEnv`, `TypeRegistry`, and `TraitRegistry` are all created fresh for each `check()` call.

OCaml solves this with `.cmi` (compiled module interface) files -- binary files containing the type signature of a module. When compiling module A that imports B, the compiler loads `B.cmi` to get B's type context. Haskell uses `.hi` (interface) files similarly.

**Consequences:**
- Cross-module function calls fail type checking
- Cross-module type references are unknown
- Cross-module trait usage is invisible
- The entire module system is non-functional without solving this

**Prevention:**
After type-checking a module, export its public interface as a data structure that other modules can consume. Two approaches:

**Approach A: Merged context (recommended for Snow's architecture)**
Compile modules in dependency order (topological sort of the import graph). Before type-checking module A, pre-populate A's `TypeEnv`, `TypeRegistry`, and `TraitRegistry` with the exported definitions from all of A's dependencies. This is the simplest approach and matches Snow's single-pass compilation model.

```rust
// Pseudocode
fn check_module(parse: &Parse, imports: &[ModuleInterface]) -> TypeckResult {
    let mut env = TypeEnv::new();
    let mut type_reg = TypeRegistry::new();
    let mut trait_reg = TraitRegistry::new();

    // Pre-populate from imports
    for import in imports {
        for (name, scheme) in &import.exported_functions {
            env.insert(qualified_name(import.path, name), scheme.clone());
        }
        for (name, def) in &import.exported_structs {
            type_reg.register_struct(qualify(import.path, def));
        }
        // ... traits, sum types, etc.
    }

    // Then run normal inference with pre-populated context
    infer_with_context(parse, env, type_reg, trait_reg)
}
```

**Approach B: Interface files (more complex, better for incremental compilation)**
After type-checking a module, serialize its public interface to a file (e.g., `.snowi`). When compiling a dependent module, load the interface file. This enables incremental compilation but adds serialization complexity. Defer to a later milestone.

**Key insight:** The `TypeckResult` already contains everything needed for a module interface -- `type_registry`, `trait_registry`, and the `types` map. The missing piece is extracting the exported subset and making it available to dependents.

**Warning signs:**
- "Unknown function" errors when calling imported functions
- "Unknown type" errors when using imported types
- Type inference producing `Var` (unresolved) for cross-module expressions

**Phase to address:**
Type inference phase. This must be designed before implementing any cross-module feature. The module compilation order and interface propagation strategy determine the architecture of the entire module system.

---

### Pitfall 4: Monomorphization Explosion from Cross-Module Generic Instantiation

**What goes wrong:**
Snow monomorphizes all generic functions -- each concrete type instantiation produces a separate function copy. Currently, monomorphization happens per-file (mono.rs), and the reachability pass starts from `main` to find all needed instantiations. With modules, a generic function defined in module A may be instantiated with different types in modules B, C, D, etc.

Consider:
```
# collections/utils.snow
pub def map_all(list :: List<T>, f :: Fun(T) -> U) -> List<U> do ... end

# module_b.snow: map_all([1,2,3], fn(x) -> x.to_string() end)  # map_all_Int_String
# module_c.snow: map_all(["a","b"], fn(x) -> x.length() end)    # map_all_String_Int
# module_d.snow: map_all([1.0, 2.0], fn(x) -> x > 0.0 end)     # map_all_Float_Bool
```

Each call site generates a unique monomorphized copy. The monomorphization pass must now consider ALL modules' call sites when generating instantiations for a generic function, not just the current file.

**Why it happens:**
The current `monomorphize()` (mono.rs:24-30) operates on a single `MirModule`. It collects reachable functions from one entry point. With multiple modules, several problems arise:

1. **Instantiation discovery:** Module A defines `map_all<T,U>`. Module B calls `map_all_Int_String`. The instantiation must be generated in the context of A's code but with B's concrete types.

2. **Duplicate instantiations:** If modules B and C both call `map_all_Int_String`, two copies exist. The linker needs COMDAT/weak linkage to deduplicate, or the compiler must ensure only one copy is generated.

3. **Code size explosion:** If 20 modules each use a utility generic with different types, 20 copies exist. For deeply generic code (generic functions calling other generic functions), this compounds exponentially.

**Consequences:**
- Binary size grows rapidly with number of modules using generics
- Compilation slows due to redundant monomorphization
- Linker symbol conflicts from duplicate instantiations
- Build times degrade non-linearly

**Prevention:**

1. **Single-module compilation (recommended for v1.8):** Lower all Snow modules into a single `MirModule` before monomorphization. This is the simplest approach -- the existing monomorphization pass works unchanged. The MirModule simply contains functions from all modules with qualified names. Reachability from `main` prunes unused instantiations naturally.

2. **Deduplication by name:** Since monomorphized names include type arguments (e.g., `collections.utils__map_all__Int__String`), identical instantiations from different call sites produce the same name. The MIR-to-LLVM pass should detect duplicates and emit only one copy.

3. **Defer separate compilation:** Do NOT attempt per-module object files in v1.8. The cross-module monomorphization problem is complex (Rust dedicates significant infrastructure to it with CGU partitioning). Compile all modules as one LLVM module first; add separate compilation in a future milestone.

**Warning signs:**
- Binary size doubles when splitting a single file into two modules with no other changes
- "Redefinition of function" errors for monomorphized generics called from multiple modules
- Compilation time grows super-linearly with number of modules

**Phase to address:**
MIR lowering and monomorphization phase. The decision to use single-module compilation vs. per-module compilation must be made early, as it affects every subsequent phase.

---

### Pitfall 5: Import Cycle Detection Misses Indirect Cycles or Fails on Diamond Dependencies

**What goes wrong:**
Snow's design forbids circular imports. The module dependency graph must be a DAG. Cycle detection must catch ALL cycles, including indirect ones:

```
# Direct cycle (easy to detect):
# a.snow imports b.snow, b.snow imports a.snow

# Indirect cycle (harder):
# a.snow imports b.snow
# b.snow imports c.snow
# c.snow imports a.snow

# Diamond (NOT a cycle -- must be allowed):
# a.snow imports b.snow and c.snow
# b.snow imports d.snow
# c.snow imports d.snow
```

If diamond dependencies are incorrectly flagged as cycles, common patterns like "two modules sharing a utility module" become impossible.

**Why it happens:**
Naive cycle detection using a simple "visited" set (without distinguishing "in current DFS path" from "fully processed") will flag diamonds as cycles. When visiting `d.snow` from the `b` path, `d` is already in the visited set from the `c` path. Without three-state coloring (WHITE/GRAY/BLACK), this looks like a back-edge.

**Consequences:**
- False positive: diamond dependencies rejected, forcing users to restructure valid code
- False negative: indirect cycles not detected, causing infinite loops or stack overflow during compilation
- Incorrect compilation order: modules compiled before their dependencies, producing "unknown type" errors

**Prevention:**
Use Kahn's algorithm (BFS-based topological sort) for both cycle detection and compilation order:

1. Build the dependency graph from all import declarations
2. Compute in-degrees for all modules
3. Process modules with in-degree 0 (no dependencies) first
4. Decrement in-degrees of dependents; add newly zero-degree modules to the queue
5. If any modules remain unprocessed, they form a cycle -- report the cycle with the exact module chain

Kahn's algorithm is preferred over DFS-based topological sort because:
- It naturally handles diamonds (a module is processed only after ALL dependencies are processed)
- Cycle detection is a simple check: `processed_count < total_modules`
- The processing order IS the compilation order
- It produces a deterministic order (if ties are broken alphabetically)

**Critical edge case:** Self-imports (`a.snow` imports `a.snow`) must be detected as a special case before graph construction.

**Warning signs:**
- Diamond dependency patterns produce "circular import" errors
- Three-module indirect cycles compile without error
- Compilation order is non-deterministic (HashMap iteration order)
- Adding a new import to an unrelated module changes compilation order of other modules

**Phase to address:**
Dependency graph phase (before type checking). This must be the very first step after parsing all files -- before any type checking occurs.

---

### Pitfall 6: Visibility System Leaks Private Definitions Through Re-exports and Type Inference

**What goes wrong:**
"Private by default, `pub` to export" seems simple, but visibility has subtle leak paths:

**Leak 1: Public function returns private type**
```
# module_a.snow
struct InternalConfig do ... end  # private

pub def get_config() -> InternalConfig do  # public function, private return type
  InternalConfig { ... }
end
```

Module B calls `get_config()` and gets a value of type `InternalConfig`. Can B access its fields? Can B store it? Can B pass it to other functions? The type is private, but the value exists in B's scope.

**Leak 2: Public trait exposes private method signature**
```
# module_a.snow
struct Secret do ... end  # private

pub interface Processor do
  def process(s :: Secret) -> Int  # public trait method uses private type
end
```

Any module implementing `Processor` must mention `Secret`, but `Secret` is private.

**Leak 3: Type inference propagates private types**
```
# module_a.snow
struct Internal do value :: Int end  # private
pub def make() -> Internal do Internal { value: 42 } end

# module_b.snow
let x = A.make()    # type of x is A.Internal -- private type visible through inference
let y = x.value     # field access on private type -- should this work?
```

**Why it happens:**
HM type inference does not inherently respect visibility boundaries. The type checker infers the most general type, which may include private types from imported modules. Without explicit visibility checks on type propagation, private types leak through public interfaces.

**Consequences:**
- Users depend on internal types, creating hidden coupling between modules
- Refactoring internal types breaks downstream modules even though the API did not change
- Confusing errors when private types appear in error messages for code in other modules

**Prevention:**

1. **Public interface validation:** After type-checking a module, validate that every `pub` function's signature only references `pub` types from the same module or imported types. Emit an error like: `"public function 'get_config' has private return type 'InternalConfig'"`. OCaml enforces this through `.mli` interface files; Rust enforces it with the `E0446` error.

2. **Opaque types for advanced use:** For now, simply reject public functions that expose private types. In a future version, add opaque type exports (the type name is visible but the definition is hidden -- consumers can pass it around but not inspect its fields).

3. **Trait method validation:** Trait method signatures in `pub interface` blocks must only reference `pub` types.

4. **Warning, not error (pragmatic option for v1.8):** Since Snow does not have opaque types yet, making this an error may be too restrictive. A warning with a clear message is acceptable for v1.8, with enforcement deferred.

**Warning signs:**
- Private struct names appearing in error messages when compiling a different module
- Users able to construct/destructure private types from other modules
- Adding `pub` to a struct definition changes the behavior of code in other modules

**Phase to address:**
Name resolution and type checking phase. Visibility checking should run as a validation pass after type inference, before MIR lowering.

---

### Pitfall 7: Existing Single-File Programs Break When Module System Is Added

**What goes wrong:**
All existing Snow programs are single-file. The module system must not change the behavior of any existing program. Specific breakage vectors:

**Break 1: New keywords conflict with existing identifiers**
If `import`, `from`, `pub`, or `module` become keywords, existing programs using these as variable or function names will fail to parse:
```
# Existing valid Snow code:
let import = "data.csv"          # breaks if 'import' is a keyword
def pub(x) -> x end             # breaks if 'pub' is a keyword
```

**Break 2: Name resolution order changes**
If the module system changes how names are resolved (e.g., checking module scope before local scope), existing programs may resolve names differently. The current resolution priority (per PROJECT.md line 159) is: module > service > variant > struct field > method. If "module" here means Snow module (not Elixir-style namespaced module), existing code using module-qualified syntax like `String.length(s)` must continue to work.

**Break 3: Compilation entry point changes**
Currently `snowc build <dir>` finds `main.snow` and compiles it as a standalone file (main.rs:222-224). If the build command changes to treat the directory as a project with module discovery, the single-file compilation path must still work identically.

**Break 4: Builtins no longer accessible**
If module system changes how builtins are registered (e.g., builtins are now in an implicit `Snow.Prelude` module), and the import mechanism is incorrect, builtins like `IO.puts`, `List.map`, etc., may become inaccessible.

**Why it happens:**
Module system changes touch the parser (new keywords), name resolution (new scope rules), compilation pipeline (multi-file orchestration), and builtins registration. Each change has the potential to break existing behavior.

**Consequences:**
- All 1,278+ existing tests fail after module system changes
- Users must modify working programs to compile under the new version
- Regression bugs in seemingly unrelated areas

**Prevention:**

1. **Contextual keywords:** Make `import`, `from`, and `pub` contextual keywords -- they are keywords only in specific syntactic positions (beginning of line, before `def`/`struct`/etc.). As variable names, they remain valid identifiers. Snow already uses this pattern for `deriving` (PROJECT.md line 144).

2. **Single-file mode preserved:** When compiling a directory with only `main.snow` and no imports, the compilation path should be functionally identical to the current pipeline. The module system is opt-in (only activated when imports are present or multiple `.snow` files exist).

3. **All existing tests pass before any new feature tests:** Make "existing tests green" the very first gate for every module system phase.

4. **Builtins are implicitly available:** Every module automatically has access to builtins without explicit imports. The implicit prelude is injected into every module's `TypeEnv` and `TypeRegistry` before type checking.

5. **Resolution order unchanged:** The current resolution order is preserved. Module-qualified names (e.g., `Math.Vector.add(a, b)`) use a new syntactic form that does not conflict with existing dot-syntax (which is method calls on values, not module access).

**Warning signs:**
- Any existing test failing after module system changes
- Parse errors in existing programs when building with the new compiler
- `snowc build` producing different output for single-file projects

**Phase to address:**
EVERY phase. Backward compatibility must be a gate for every PR. Run the full test suite after every change.

---

## Moderate Pitfalls

Mistakes that cause technical debt, confusing errors, or delayed regressions.

---

### Pitfall 8: Single LLVM Module Becomes Bottleneck -- But Separate Modules Are Premature

**What goes wrong:**
The current codegen creates one LLVM module per compilation (lib.rs:80, `CodeGen::new(&context, "snow_module", ...)`). For multi-module Snow programs, the simplest approach is to lower everything into one big LLVM module. This works but has scaling consequences:

- LLVM optimization passes run on the entire module (not per-function), so compile times grow super-linearly
- The entire program must be recompiled for any change in any module
- No parallel compilation is possible

However, splitting into separate LLVM modules per Snow module introduces all the monomorphization, symbol, and type identity problems described above (Pitfalls 1, 2, 4). Attempting separate LLVM modules in v1.8 is a trap.

**Prevention:**
Start with a single LLVM module. Accept the compilation time tradeoff for v1.8. Add a comment/TODO for future separate compilation. The Rust compiler team invested years in the CGU partitioning scheme -- Snow should not attempt this in the first module system milestone.

**Warning signs:**
- Attempting to create multiple LLVM modules and link them
- Investigating LLVM module linking or LTO as part of v1.8
- Compilation time exceeding 10 seconds for modest multi-module programs

**Phase to address:** Codegen phase. Decision should be documented as "single LLVM module for v1.8" in the architecture doc.

---

### Pitfall 9: Module Path Ambiguity -- Dots in Module Names vs. Dots in Method Calls

**What goes wrong:**
Snow v1.6 added method dot-syntax: `value.method(args)`. The module system uses dots for module paths: `Math.Vector.add(a, b)`. These are syntactically ambiguous:

```
# Is this a module-qualified function call, or a method chain?
Vector.add(a, b)

# If Vector is a local variable holding a struct:
#   -> method call: Vector.add(a, b) means add(Vector, a, b)
# If Vector is a module name:
#   -> qualified call: calls Math.Vector.add with args (a, b)
```

The v1.6 resolution order (PROJECT.md line 159) puts module-qualified calls first: `module > service > variant > struct field > method`. But this means importing a module named `Vector` would shadow a local variable named `Vector` for dot-syntax purposes.

**Prevention:**
Module-qualified access should use a DISTINCT syntactic form from method calls. Options:

- **Option A (recommended):** Module paths use `::` separator: `Math::Vector::add(a, b)`. This is unambiguous -- `::` is never used for method calls. Familiar from Rust and C++.

- **Option B:** Module paths only valid at the beginning of a name, before any value-level operations. `Vector.add(a, b)` is a module call only if `Vector` is a known module name (not a local variable). This is the approach used by Elixir and works because module names are capitalized and rarely shadow locals.

- **Option C:** Use the current dot syntax but resolve based on what `Vector` is. If it is a module, it is a module call. If it is a variable, it is a method call. This requires name resolution to be context-aware and may produce confusing errors.

The PROJECT.md indicates `import Math.Vector` with dot syntax for module paths. Since Snow already uses `.` for method calls, the parser must distinguish `ModuleName.function()` from `variable.method()`. The simplest disambiguation: module names are resolved FIRST during name resolution. If a name resolves to a module, it is a module-qualified call. Otherwise, it falls through to method resolution.

**Warning signs:**
- Importing a module with the same name as a local variable causes unexpected behavior
- Method chains on variables fail when a module with that name exists
- Parse ambiguity between `IO.puts("hello")` (module call) and `my_io.puts("hello")` (method call)

**Phase to address:** Parser and name resolution phase. The syntactic disambiguation must be designed before implementation.

---

### Pitfall 10: Trait Coherence Breaks with Cross-Module Impls

**What goes wrong:**
Rust's "orphan rule" exists for a reason: without it, two different modules could implement the same trait for the same type, and the compiler would not know which implementation to use. Snow currently has no orphan rule because all code is in one file. With modules:

```
# module_a.snow
struct Point do x :: Int, y :: Int end

# module_b.snow
impl Display for Point do
  def to_string(self) -> String do "from B" end
end

# module_c.snow
impl Display for Point do
  def to_string(self) -> String do "from C" end
end
```

When module D imports both B and C, which `Display` impl for `Point` does it get?

**Why it happens:**
The `TraitRegistry` (traits.rs:73-80) stores impls keyed by trait name, with a Vec of impls. The `has_impl` lookup uses structural type matching. If two impls for the same trait+type exist, the first one wins (or the lookup is ambiguous). The v1.6 AmbiguousMethod diagnostic (PROJECT.md line 163) handles this for method calls, but not for arbitrary trait dispatch.

**Prevention:**
For v1.8, implement a simple coherence rule:

1. **A trait can only be implemented for a type in the module that defines the type OR the module that defines the trait.** This is the Rust orphan rule simplified.

2. **At module loading time, check for duplicate impls.** When merging TraitRegistries from imported modules, detect if the same `(trait, type)` pair has multiple impls. Emit a clear error: `"conflicting implementations of trait 'Display' for type 'Point' found in modules B and C"`.

3. **Defer blanket impls and specialization.** These are explicitly out of scope (PROJECT.md line 95).

**Warning signs:**
- Different behavior depending on import order
- Trait method calls producing different results depending on which module was compiled first
- `HashMap::insert` in `TraitRegistry` silently overwriting existing impls

**Phase to address:** Type checking phase, after cross-module trait registration is implemented.

---

### Pitfall 11: Module Discovery Races with File System Layout

**What goes wrong:**
Snow's design maps file paths to module names: `math/vector.snow` becomes `Math.Vector`. Several edge cases:

1. **Case sensitivity:** On macOS (case-insensitive FS), `Math/Vector.snow` and `math/vector.snow` are the same file. On Linux (case-sensitive), they are different. If Snow treats module names case-sensitively (as it should), macOS users can create ambiguous situations.

2. **Non-UTF8 paths:** Directory names with special characters produce unparseable module names.

3. **Symlinks:** Symlinked files could create the appearance of two different modules that are actually the same file.

4. **Nested `main.snow`:** If `math/main.snow` exists, is its module name `Math.Main` or `Math`?

5. **Hidden files:** `.gitkeep`, `.DS_Store`, etc., should not be treated as modules.

**Prevention:**

1. **Module names must be valid Snow identifiers:** Enforce that directory and file names (without `.snow` extension) match `[a-z][a-z0-9_]*` (lowercase, alphanumeric, underscores). Reject files that do not match with a clear error.

2. **Canonical paths:** Use `std::fs::canonicalize` to resolve symlinks before building the module graph. Two paths that canonicalize to the same file are the same module.

3. **`main.snow` is special:** The entry-point file is always `main.snow` in the project root. It is not a module -- it is the entry point. Nested `main.snow` files should be treated as regular modules named `<Parent>.Main` or rejected with an error.

4. **Filter non-Snow files:** Only files ending in `.snow` are considered. The existing `collect_snow_files` (main.rs:424-449) already does this.

**Warning signs:**
- Same module compiled twice from symlinked paths
- Module names containing hyphens or dots (from directory names) causing parse errors
- Different behavior on macOS vs. Linux

**Phase to address:** Module discovery phase (before parsing). File system traversal and module name derivation should be a separate, well-tested function.

---

### Pitfall 12: Error Messages Become Useless Across Module Boundaries

**What goes wrong:**
Snow uses ariadne for diagnostics (main.rs:300-314), with source text and filename for error rendering. Currently, there is one source file and one filename. With modules, errors may reference types or functions defined in other files:

```
# Error in module_b.snow, caused by type mismatch with module_a's export:
error[E0003]: Type mismatch
  --> module_b.snow:5:15
  |
5 | let x: Int = A.get_config()
  |              ^^^^^^^^^^^^^^ expected Int, got InternalConfig

# Where is InternalConfig defined? What does it look like?
# The user has no context.
```

**Prevention:**

1. **Multi-file diagnostics:** Ariadne supports multi-file error reporting. When a type mismatch involves a type from another module, include a secondary label pointing to the type's definition:

```
error[E0003]: Type mismatch
  --> module_b.snow:5:15
  |
5 | let x: Int = A.get_config()
  |              ^^^^^^^^^^^^^^ expected Int, got A.InternalConfig
  |
  --> module_a.snow:3:1
  |
3 | struct InternalConfig do
  | ^^^^^^^^^^^^^^^^^^^^^^^ InternalConfig defined here
```

2. **Qualified type names in errors:** Always show the module-qualified name in error messages (`A.InternalConfig`, not `InternalConfig`). This helps users understand where the type comes from.

3. **Source map:** Maintain a mapping from module name to source text and file path. Pass this to the diagnostic renderer.

**Warning signs:**
- Error messages showing bare type names that the user cannot find in their current file
- Type mismatch errors with no indication of which module defined the expected type
- Stack traces or panic messages with module-qualified mangled names that are unreadable

**Phase to address:** Diagnostics phase. Can be deferred slightly (working but ugly errors are acceptable initially), but must be addressed before the milestone is complete.

---

## Minor Pitfalls

### Pitfall 13: Topological Sort Produces Non-Deterministic Order

**What goes wrong:**
If the dependency graph has multiple valid topological orderings, the compilation order may vary between runs (due to HashMap iteration order). This causes non-deterministic compiler output: different mangled names, different LLVM IR layout, different binary hashes. Makes debugging and reproducible builds difficult.

**Prevention:** Break ties in topological sort alphabetically by module name. Use `BTreeMap` or sort the adjacency lists before processing.

**Phase to address:** Dependency graph phase.

---

### Pitfall 14: Recompiling Entire Project on Any Change

**What goes wrong:**
Without incremental compilation, changing one module recompiles everything. For small projects (< 50 modules), this is acceptable. For larger projects, compilation times become frustrating.

**Prevention:** For v1.8, accept full recompilation. Document that incremental compilation is a future optimization. The single-LLVM-module approach (Pitfall 8) makes incremental compilation impossible anyway, so this is a consistent design decision.

**Phase to address:** Not in v1.8 scope. Document as future work.

---

### Pitfall 15: Service/Actor Definitions Across Modules

**What goes wrong:**
Snow's `ServiceDef` and `ActorDef` use dispatch tables (`service_dispatch` in MirModule) with string function names. If a service in module A handles messages that call handler functions in module B, the dispatch table must use module-qualified function names. Existing service dispatch (codegen/mod.rs:91-94) uses bare function names.

**Prevention:** When building service dispatch tables, use the same module-qualified function names used in MIR. Since services are typically defined in a single module, this may not be an immediate issue, but the infrastructure must handle qualified names.

**Phase to address:** MIR lowering phase, when processing service/actor definitions.

---

## Phase-Specific Warnings

| Phase Topic | Likely Pitfall | Mitigation | Severity |
|---|---|---|---|
| Module discovery (file system) | Case sensitivity, symlinks, non-ASCII paths | Canonical paths, naming constraints, filter non-`.snow` files | Medium |
| Dependency graph | Diamond flagged as cycle, indirect cycles missed | Kahn's algorithm with proper in-degree tracking | Critical |
| Parser (import/pub syntax) | New keywords break existing programs | Contextual keywords, single-file mode unchanged | Critical |
| Name resolution | Bare names collide across modules | Module-qualified names throughout pipeline | Critical |
| Type checking | No cross-module type context | Merged TypeEnv/TypeRegistry from dependencies | Critical |
| Visibility checking | Private types leak through public interfaces | Public interface validation pass | Moderate |
| Trait coherence | Duplicate impls from different modules | Orphan rule, duplicate detection at merge | Moderate |
| MIR lowering | Function name collisions, wrong mangling | Module-qualified MIR function names | Critical |
| Monomorphization | Cross-module generic instantiation conflicts | Single MirModule, deduplicate by mangled name | Critical |
| LLVM codegen | Struct type name collisions in LLVM cache | Module-qualified LLVM type names | Critical |
| Linking | Duplicate symbol errors | Module-qualified symbol names, single object file | Critical |
| Diagnostics | Error messages reference unknown types | Multi-file diagnostics, qualified type names | Minor |
| Backward compat | Existing tests break | Run full suite after every change | Critical |

## Integration Gotchas

Common mistakes when connecting module system to existing Snow systems.

| Integration | Common Mistake | Correct Approach |
|---|---|---|
| TypeEnv + imports | Adding imported names to global scope, shadowing builtins | Add imported names to a module-specific scope level, below builtins |
| TypeRegistry + multi-module | Using bare type names as keys | Module-qualified keys: `"Math.Vector.Point"` not `"Point"` |
| TraitRegistry + cross-module | Merging impls without coherence check | Check for duplicate `(trait, type)` pairs during merge |
| MirModule + multi-module | Creating separate MirModules per file | Merge all modules into one MirModule with qualified names |
| Monomorphization + generics | Running mono per-module | Run mono once on merged MirModule; reachability from main handles dedup |
| Codegen + LLVM types | Bare struct names in LLVM | `context.opaque_struct_type("Math.Vector.Point")` not `"Point"` |
| Link + symbols | Bare function names as LLVM symbols | Module-qualified: `"math.vector__add"` not `"add"` |
| Service dispatch + modules | Bare handler function names in dispatch table | Qualified handler names match MIR function names |
| LSP + modules | Ignoring module scope in hover/completion | Include imported names in scope; show module path in hover |
| Formatter + imports | Not formatting import statements | Add import/from/pub to formatter AST walker |
| REPL + modules | Cannot import modules in REPL | Either defer module support in REPL, or add `import` to REPL context |
| Error messages + modules | Type names unqualified in errors | Always show `Module.TypeName` in diagnostics |

## "Looks Done But Isn't" Checklist

- [ ] **Two modules with same-named struct:** Types are distinct; field access uses correct layout
- [ ] **Two modules with same-named function:** Both callable via qualified names; no symbol collision
- [ ] **Diamond dependency:** Module D used by B and C, both used by A -- compiles correctly, D compiled once
- [ ] **Indirect cycle:** A -> B -> C -> A detected and reported with full cycle chain
- [ ] **Self-import:** Module importing itself produces clear error
- [ ] **Private function not accessible:** Calling a non-pub function from another module produces E-code error
- [ ] **Private struct not constructible:** Using a non-pub struct from another module produces error
- [ ] **Public function with private return type:** Warning or error issued
- [ ] **Trait impl from other module:** `Display` impl for imported struct works correctly
- [ ] **Generic function across modules:** `identity<T>(x)` defined in A, called in B with concrete type, monomorphized correctly
- [ ] **Single-file program unchanged:** ALL existing tests pass with zero modifications
- [ ] **Builtins accessible:** `IO.puts`, `List.map`, etc., work without explicit import
- [ ] **Empty module:** A module with no definitions compiles without error
- [ ] **Module with only types:** A module exporting only struct/sum type definitions works
- [ ] **Selective import:** `from Math.Vector import { add, scale }` makes only `add` and `scale` available
- [ ] **Qualified call:** `Math.Vector.add(a, b)` works
- [ ] **Import aliasing:** If supported, `import Math.Vector as V; V.add(a, b)` works
- [ ] **Re-export chain:** A exports from B which exports from C -- C's types visible through A
- [ ] **Module-qualified error messages:** Type errors show `Math.Vector.Point`, not `Point`
- [ ] **Deterministic build:** Same source always produces same binary (byte-identical)

## Recovery Strategies

When pitfalls occur despite prevention, how to recover.

| Pitfall | Recovery Cost | Recovery Steps |
|---|---|---|
| P1: Type identity collision | HIGH | Retrofit module-qualified names through entire pipeline; every FxHashMap key changes |
| P2: Symbol name collision | MEDIUM | Add module prefix to MIR function names; cascades through codegen automatically |
| P3: No cross-module type context | HIGH | Redesign type checking to accept pre-populated context; affects infer.rs entry point |
| P4: Monomorphization explosion | MEDIUM | Switch to merged MirModule approach; rewrite compile_to_binary to merge before mono |
| P5: Cycle detection bugs | LOW | Replace cycle detection algorithm; dependency graph is isolated from rest of pipeline |
| P6: Visibility leaking | MEDIUM | Add validation pass; does not require changing existing pipeline, only adding a check |
| P7: Backward compatibility break | VARIES | Depends on what broke; keyword conflicts are LOW (contextual keywords); resolution order changes are HIGH |
| P8: LLVM module scaling | LOW | Already using single module; future optimization, not a fix |
| P9: Module path ambiguity | MEDIUM | Change syntax (e.g., `::` separator); requires parser changes but not pipeline changes |
| P10: Trait coherence | MEDIUM | Add duplicate detection at TraitRegistry merge; localized change |
| P11: File system issues | LOW | Add validation in module discovery; isolated from compilation pipeline |
| P12: Bad error messages | LOW | Improve diagnostic rendering; does not affect correctness |

## Sources

### Snow Codebase Analysis (HIGH confidence -- direct code reading)
- `crates/snow-typeck/src/infer.rs` -- `TypeRegistry` (line 115-170), struct/sum type registration with bare name keys, Algorithm J inference entry point
- `crates/snow-typeck/src/env.rs` -- `TypeEnv` scope stack with `FxHashMap<String, Scheme>`, no module awareness
- `crates/snow-typeck/src/traits.rs` -- `TraitRegistry` with `FxHashMap<String, Vec<ImplDef>>`, structural type matching, no coherence checking
- `crates/snow-typeck/src/lib.rs` -- `TypeckResult` containing `type_registry`, `trait_registry`, `default_method_bodies`
- `crates/snow-codegen/src/mir/mod.rs` -- `MirModule`, `MirFunction.name` as bare string, `MirType::Struct(String)` with bare name
- `crates/snow-codegen/src/mir/types.rs` -- `mangle_type_name` (line 143), `mir_type_suffix`, `mir_type_to_impl_name` -- all using bare type names
- `crates/snow-codegen/src/mir/lower.rs` -- MIR lowering from AST, no module path context
- `crates/snow-codegen/src/mir/mono.rs` -- `monomorphize` reachability pass on single `MirModule`
- `crates/snow-codegen/src/codegen/mod.rs` -- `CodeGen` struct with `struct_types`, `functions` caches keyed by bare name, `compile()` processing single `MirModule`
- `crates/snow-codegen/src/link.rs` -- Linking single object file with `libsnow_rt.a`
- `crates/snow-codegen/src/lib.rs` -- `compile_to_binary` pipeline: lower_to_mir -> mono -> codegen -> link, single Parse input
- `crates/snowc/src/main.rs` -- Build pipeline reading single `main.snow`, `report_diagnostics` with single source/filename

### OCaml Module System Precedent (HIGH confidence -- mature language with HM inference)
- [OCaml Compilation Units](https://cs3110.github.io/textbook/chapters/modules/compilation_units.html) -- `.ml`/`.mli` pairs, `.cmi` compiled interface files, hash-based consistency checking, compilation order requirements
- [Real World OCaml: Compiler Frontend](https://dev.realworldocaml.org/compiler-frontend.html) -- Type identity via `.cmi` hashes, soundness of separate compilation, `-opaque` flag for cross-module optimization control
- [OCaml Batch Compilation](https://ocaml.org/manual/5.0/comp.html) -- Dependency-order linking, free module identifier resolution via `.cmi` search path

### Rust Symbol Mangling & Monomorphization (HIGH confidence -- well-documented)
- [Rust RFC 2603: Symbol Name Mangling v0](https://rust-lang.github.io/rfcs/2603-rust-symbol-name-mangling-v0.html) -- Crate-id in symbol names for cross-crate deduplication, monomorphization disambiguation by concrete type arguments
- [Rust Compiler Dev Guide: Monomorphization](https://rustc-dev-guide.rust-lang.org/backend/monomorph.html) -- CGU partitioning (stable vs. volatile), `collect_and_partition_mono_items`, polymorphization for unused generic params
- [Monomorphization Code Bloat](https://nickb.dev/blog/the-dark-side-of-inlining-and-monomorphization/) -- Binary size explosion, cache effects, factor-out-inner-function pattern, dynamic dispatch alternative

### Visibility & Coherence (MEDIUM confidence -- cross-language patterns)
- [Rust: Leaking Private Types](https://users.rust-lang.org/t/leaking-traits-or-types-from-private-modules/67678) -- E0446 error for private types in public interfaces, workarounds with pub(crate)
- [C++ Modules Misconceptions](https://build2.org/article/cxx-modules-misconceptions.xhtml) -- Module linkage vs. external linkage, non-exported names get module linkage preventing collision, backward compatibility with headers

### Cycle Detection & Dependency Graphs (HIGH confidence -- well-established algorithms)
- [Kahn's Algorithm for Topological Sort](https://www.geeksforgeeks.org/dsa/detect-cycle-in-directed-graph-using-topological-sort/) -- BFS-based, natural cycle detection via unprocessed vertex count, handles diamonds correctly
- [Topological Sort Applications](https://www.numberanalytics.com/blog/mastering-topological-sort-algorithms) -- Compiler module ordering, deterministic output, edge cases with diamond patterns

### Backward Compatibility Patterns (MEDIUM confidence -- cross-domain)
- [Expand-Migrate-Contract Pattern](https://docs.gitlab.com/development/multi_version_compatibility/) -- Additive changes first, migrate consumers, then remove old behavior
- [C++ Modules Backward Compatibility](https://build2.org/article/cxx-modules-misconceptions.xhtml) -- Dual header/module support, ODR preservation across module boundaries

---
*Pitfalls research for: Snow v1.8 Module System milestone*
*Researched: 2026-02-09*
