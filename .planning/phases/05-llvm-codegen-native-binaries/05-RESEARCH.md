# Phase 5: LLVM Codegen & Native Binaries - Research

**Researched:** 2026-02-06
**Domain:** LLVM code generation via Inkwell, pattern match compilation, closure codegen, runtime stub with GC, CLI binary output
**Confidence:** HIGH

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions

#### Mid-level IR design
- Claude's discretion on whether to introduce a dedicated MIR or lower directly from typed AST to LLVM IR
- Claude's discretion on monomorphization vs type erasure for generics
- Claude's discretion on ADT memory layout (tagged unions, pointer tagging, etc.)
- Strings are GC-managed -- prepare for per-actor GC from Phase 6 even though actors aren't implemented yet. This means a simple GC (at minimum a bump allocator or basic mark-sweep) needs to exist in the runtime stub

#### Pattern match compilation
- Claude's discretion on decision tree vs backtracking strategy and optimization level
- Guards stay restricted (comparisons, boolean ops, literals, name refs, named function calls) -- do not expand guard expressiveness
- Runtime match failure (guarded arms edge case) panics with source location + "non-exhaustive match" message and aborts
- Or-patterns duplicate the arm body for each alternative -- simpler codegen, let LLVM deduplicate

#### Closure & capture strategy
- Claude's discretion on capture semantics (copy vs reference) based on Snow's semantics and GC model
- Claude's discretion on closure representation (fat pointers vs heap objects) -- should align with GC-managed memory model
- Claude's discretion on partial application / currying support
- Claude's discretion on pipe operator compilation strategy (syntactic sugar vs special IR node)

#### CLI & binary output
- `snowc build <dir>` is the primary command -- project-based compilation, not single-file
- Support both -O0 (debug) and -O2 (release) optimization levels
- Reuse ariadne diagnostic rendering for all compilation errors (consistent with parse/type-check errors)
- `snowc build --emit-llvm` flag to dump .ll file alongside binary for codegen inspection/debugging

### Claude's Discretion
- MIR vs direct lowering architecture
- Monomorphization vs type erasure
- ADT memory layout strategy
- Decision tree algorithm and optimization level
- Closure capture semantics and representation
- Partial application support
- Pipe operator compilation approach

### Deferred Ideas (OUT OF SCOPE)
None -- discussion stayed within phase scope
</user_constraints>

## Summary

This phase transforms the typed AST produced by Phases 1-4 into native executables via LLVM. The research covers five interconnected domains: (1) the Inkwell LLVM bindings and AOT compilation workflow, (2) an intermediate lowering strategy from the typed Rowan CST to LLVM IR, (3) pattern match compilation to decision trees, (4) closure representation and capture strategy, and (5) runtime stub design including GC-managed strings and a simple mark-sweep collector.

The existing codebase has a well-structured pipeline: `snow-lexer` -> `snow-parser` (Rowan CST) -> `snow-typeck` (Hindley-Milner inference with TypeckResult containing type-annotated ranges). The codegen phase must bridge from the CST+TypeckResult to LLVM IR. A dedicated Mid-level IR (MIR) is recommended over direct lowering because the typed Rowan CST is too syntax-oriented for clean LLVM emission -- the MIR would explicitly represent closures, monomorphized functions, lowered patterns, and resolved types in a form optimized for codegen.

The system has LLVM 21.1.8 installed via Homebrew at `/opt/homebrew/opt/llvm`. Inkwell 0.8.0 supports LLVM 11-21 and provides safe Rust wrappers over all needed LLVM operations: function creation, basic blocks, arithmetic, comparisons, branching, phi nodes, struct GEP, switch, alloca, load, store, and function calls.

**Primary recommendation:** Introduce a thin MIR layer (typed, desugared, closure-converted, monomorphized) that is straightforward to lower to LLVM IR via Inkwell 0.8.0, with the runtime stub written in Rust and statically linked.

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| inkwell | 0.8.0 | Safe Rust LLVM bindings | Only maintained safe wrapper for LLVM in Rust; supports LLVM 11-21 |
| llvm-sys | (transitive) | Raw LLVM C FFI | Pulled in by inkwell; provides the actual LLVM bindings |
| clap | 4.5.x | CLI argument parsing | De facto standard for Rust CLI tools; derive macro support |
| ariadne | 0.6 (existing) | Diagnostic rendering | Already used in Phases 1-4 for error reporting |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| rustc-hash | 2 (existing) | Fast hashing for maps | Already in workspace; use for all FxHashMap needs |
| insta | 1.46 (existing) | Snapshot testing | Already in workspace; use for IR snapshot tests |

### Not Needed
| Instead of | Could Use | Why Not |
|------------|-----------|---------|
| cranelift | inkwell | Inkwell/LLVM already decided in PROJECT.md; Cranelift lacks optimizer maturity |
| llvm-sys directly | inkwell | Inkwell provides type-safe wrappers, less unsafe code |

**Installation (Cargo.toml for new crates):**
```toml
# In workspace Cargo.toml, add:
[workspace.dependencies]
inkwell = { version = "0.8.0", features = ["llvm21-1"] }
clap = { version = "4.5", features = ["derive"] }

# For snow-codegen crate:
[dependencies]
inkwell = { workspace = true }
snow-common = { path = "../snow-common" }
snow-parser = { path = "../snow-parser" }
snow-typeck = { path = "../snow-typeck" }

# For snowc binary crate:
[dependencies]
clap = { workspace = true }
snow-common = { path = "../snow-common" }
snow-lexer = { path = "../snow-lexer" }
snow-parser = { path = "../snow-parser" }
snow-typeck = { path = "../snow-typeck" }
snow-codegen = { path = "../snow-codegen" }
ariadne = { workspace = true }
```

**LLVM Environment Setup:**
```bash
# LLVM 21.1.8 is already installed at /opt/homebrew/opt/llvm
# Required for inkwell to find LLVM:
export LLVM_SYS_211_PREFIX=/opt/homebrew/opt/llvm
```

## Architecture Patterns

### Recommended Project Structure
```
crates/
  snow-codegen/          # NEW: LLVM codegen crate
    src/
      lib.rs             # Public API: compile(Parse, TypeckResult) -> Result<()>
      mir/               # Mid-level IR definitions and lowering
        mod.rs           # MIR types (MirModule, MirFn, MirExpr, MirPat, etc.)
        lower.rs         # AST+TypeckResult -> MIR lowering
        mono.rs          # Monomorphization pass
      pattern/           # Pattern match compilation
        mod.rs           # Decision tree types
        compile.rs       # Pattern matrix -> decision tree
      codegen/           # LLVM IR generation
        mod.rs           # CodegenCtx struct, module setup
        expr.rs          # Expression codegen
        pattern.rs       # Decision tree -> LLVM branches/switches
        types.rs         # Snow types -> LLVM types mapping
        intrinsics.rs    # Runtime function declarations
      runtime/           # Runtime stub (written in Rust, compiled to .o)
        mod.rs           # Build script integration
  snow-rt/               # NEW: Runtime library (statically linked)
    src/
      lib.rs             # Runtime entry, GC init
      gc.rs              # Simple mark-sweep GC
      string.rs          # GC-managed string operations
      panic.rs           # Panic handler (match failure, etc.)
  snowc/                 # EXISTING: Updated with CLI
    src/
      main.rs            # clap-based CLI: snowc build <dir>
```

### Pattern 1: MIR as Intermediate Representation
**What:** A dedicated mid-level IR between the typed AST and LLVM IR. The MIR is desugared, closure-converted, monomorphized, and has patterns lowered to decision trees.
**When to use:** Always -- the Rowan CST is too syntax-heavy for direct LLVM emission.
**Recommendation:** Use monomorphization (not type erasure) for Phase 5. Rationale:
- Snow targets native performance (not JVM/BEAM-style runtimes)
- Monomorphization produces better code for LLVM to optimize
- The type system is already Hindley-Milner with concrete resolved types
- Compile time is not yet a concern at this project stage
- Type erasure would require boxing all generic values, conflicting with the goal of zero-overhead native code

**MIR type sketch:**
```rust
// Source: Research recommendation based on rustc MIR design
pub struct MirModule {
    pub functions: Vec<MirFunction>,
    pub structs: Vec<MirStructDef>,
    pub sum_types: Vec<MirSumTypeDef>,
}

pub struct MirFunction {
    pub name: String,
    pub params: Vec<(String, MirType)>,
    pub return_type: MirType,
    pub body: MirExpr,
    pub is_closure: bool,
    pub captures: Vec<(String, MirType)>,  // for closures
}

pub enum MirType {
    Int,
    Float,
    Bool,
    String,    // GC-managed pointer
    Tuple(Vec<MirType>),
    Struct(String),
    SumType(String),
    Function(Vec<MirType>, Box<MirType>),
    Closure(Vec<MirType>, Box<MirType>),  // same as Function but has env
    Never,
}

pub enum MirExpr {
    IntLit(i64),
    FloatLit(f64),
    BoolLit(bool),
    StringLit(String),
    Var(String, MirType),
    BinOp(BinOp, Box<MirExpr>, Box<MirExpr>, MirType),
    UnaryOp(UnaryOp, Box<MirExpr>, MirType),
    Call(Box<MirExpr>, Vec<MirExpr>, MirType),
    If(Box<MirExpr>, Box<MirExpr>, Box<MirExpr>, MirType),
    Let(String, MirType, Box<MirExpr>, Box<MirExpr>),
    Block(Vec<MirExpr>, MirType),
    Match(Box<MirExpr>, DecisionTree, MirType),
    StructLit(String, Vec<(String, MirExpr)>, MirType),
    FieldAccess(Box<MirExpr>, String, MirType),
    ConstructVariant(String, String, Vec<MirExpr>, MirType),
    MakeClosure(String, Vec<MirExpr>),  // fn_name, captured values
    Return(Box<MirExpr>),
    Panic(String),  // for match failures
}
```

### Pattern 2: Monomorphization Strategy
**What:** For each generic function instantiation with concrete types, create a specialized copy.
**When to use:** During MIR lowering, when resolving generic function calls.
**How it works:**
1. Walk the typed AST collecting all call sites with resolved concrete types
2. For each unique combination of type arguments, generate a specialized MirFunction
3. Name mangling: `identity_Int`, `map_Option_Int_String`, etc.
4. The MIR module contains only monomorphic functions -- no type variables remain

### Pattern 3: Tagged Union Layout for ADTs
**What:** Sum types (ADTs) represented as `{ i8 tag, [N x i8] payload }` structs in LLVM IR.
**When to use:** For all sum type definitions (user-defined + Option/Result).
**Layout:**
```
%Shape = type { i8, [16 x i8] }          ; tag + max payload
%Shape.Circle = type { i8, double }       ; tag=0, radius
%Shape.Rectangle = type { i8, double, double } ; tag=1, width, height
%Shape.Point = type { i8 }               ; tag=2, no payload
```
**Tag assignment:** Variants numbered 0..N in declaration order.
**Access pattern:** Load tag via GEP[0,0], switch on tag, bitcast to variant struct, GEP to fields.
**Recommendation:** Use opaque pointers (LLVM 21 uses them by default) -- no bitcast needed for pointer types, just different GEP source types.

### Pattern 4: Closure Representation (Heap-Allocated Environment)
**What:** Closures represented as a pair of function pointer + GC-managed environment struct.
**When to use:** For all closure expressions and captured variables.
**Recommendation:** Use heap-allocated environment objects managed by the GC.
**Layout:**
```
%Closure = type { ptr, ptr }  ; { fn_ptr, env_ptr }
; fn_ptr signature: (env_ptr, params...) -> ret
%closure_env_0 = type { i64, double }  ; captured x::Int, y::Float
```
**Capture semantics:** Copy capture for immutable values (Snow is functional-first with immutable let bindings). Mutable variables (if added later) would need reference capture.
**Calling convention:** When calling a closure, load fn_ptr and env_ptr from the closure struct, pass env_ptr as the first argument to fn_ptr.
**Known functions vs closures:** Known function calls (not closures) bypass the closure overhead -- call directly without env_ptr. The MIR should distinguish between direct calls and closure calls.

### Pattern 5: Pipe Operator as Syntactic Desugar
**What:** `x |> f` desugars to `f(x)` at the MIR level.
**When to use:** During AST-to-MIR lowering.
**Rationale:** No special IR node needed. The pipe operator is purely syntactic sugar. `a |> b |> c` becomes `c(b(a))`. Handle during MIR lowering before codegen sees it.

### Pattern 6: String Interpolation Compilation
**What:** `"Hello ${name}, you are ${age} years old"` compiles to runtime string concatenation calls.
**When to use:** During MIR lowering of StringExpr nodes.
**Strategy:**
1. Split the string literal into segments: literal parts and interpolation expressions
2. For each interpolation expression, generate a `to_string()` call to convert to string
3. Chain `string_concat()` runtime calls to build the final string
4. The result is a GC-managed string pointer

### Anti-Patterns to Avoid
- **Direct CST-to-LLVM lowering:** The Rowan CST has too many syntax-level concerns (trivia, tokens, ranges) that make direct codegen unwieldy. Always go through MIR.
- **Using LLVM's GC intrinsics directly:** LLVM's gc.root/gc.statepoint are complex and designed for production GCs. For Phase 5, use a simpler approach: the runtime manages its own heap, and the compiler generates explicit calls to runtime allocation functions. LLVM's GC framework can be adopted later if needed.
- **Over-engineering the GC:** Phase 5 needs a working GC stub, not a production collector. A simple mark-sweep with explicit root registration is sufficient. The per-actor GC refinement happens in Phase 6.
- **Hand-rolling the linker invocation:** Use `cc` (the system C compiler) as the linker driver, not raw `ld`. The `cc` command handles finding crt0, libc, and platform-specific linker flags automatically.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| LLVM binding safety | Unsafe raw FFI calls to llvm-sys | inkwell 0.8.0 | Catches type errors at compile time; much less unsafe code |
| CLI argument parsing | Manual arg parsing | clap 4.5 with derive | Handles help text, validation, subcommands automatically |
| Platform target detection | Manual triple construction | `TargetMachine::get_default_triple()` from inkwell | LLVM knows the host triple; don't guess |
| Object file linking | Hand-invoke `ld` with CRT paths | Invoke `cc` as linker driver | `cc` finds CRT objects, libc, handles macOS vs Linux automatically |
| Optimization passes | Manual pass pipeline | `module.run_passes()` with LLVM's pass strings | LLVM 16+ uses the new pass manager with string-based pass names |
| Int-to-string conversion | Custom itoa implementation | C's `snprintf` or Rust's `itoa` in runtime | Well-tested, handles edge cases |

**Key insight:** The runtime stub should be written in Rust (compiled to a static library) and linked into the final binary. This avoids writing C, keeps everything in one toolchain, and the Rust runtime can expose `extern "C"` functions that LLVM IR calls directly.

## Common Pitfalls

### Pitfall 1: LLVM Environment Variable Not Set
**What goes wrong:** `inkwell` fails to compile because `llvm-sys` cannot find LLVM headers/libraries.
**Why it happens:** Homebrew installs LLVM to a non-standard path (`/opt/homebrew/opt/llvm`) and does not add it to PATH.
**How to avoid:** Set `LLVM_SYS_211_PREFIX=/opt/homebrew/opt/llvm` in the environment. Add this to `.cargo/config.toml` or document in build instructions. The number `211` matches LLVM version 21.1.
**Warning signs:** Build errors mentioning "llvm-config not found" or "Could not find LLVM".

### Pitfall 2: Opaque Pointers vs Typed Pointers
**What goes wrong:** Trying to use `bitcast` for pointer types or passing typed pointers to LLVM.
**Why it happens:** LLVM 15+ moved to opaque pointers; LLVM 21 requires them. All pointers are just `ptr` (no `i32*`, `%struct.Foo*`). Old tutorials show typed pointer syntax.
**How to avoid:** Never use `build_bitcast` for pointer-to-pointer casts. Use `build_load` with explicit pointee type parameter. Inkwell 0.8 handles this correctly -- `PointerValue` is untyped, and `build_load`/`build_store`/`build_struct_gep` take a type parameter.
**Warning signs:** LLVM verification errors about pointer types.

### Pitfall 3: Missing Terminators in Basic Blocks
**What goes wrong:** LLVM module verification fails with "basic block does not have terminator."
**Why it happens:** Every LLVM basic block must end with exactly one terminator instruction (br, ret, switch, unreachable). It is easy to forget terminators on branches that seem "dead" (e.g., after a panic call).
**How to avoid:** After every control flow split (if/else, match arms), ensure each branch ends with a terminator. After calls to `@snow_panic` (which is `noreturn`), emit `build_unreachable()`.
**Warning signs:** LLVM verification errors at module finalization.

### Pitfall 4: Phi Node Placement
**What goes wrong:** Incorrect SSA form when if/else or match expressions produce values.
**Why it happens:** In SSA form, a value that could come from multiple paths needs a phi node at the join point. The phi node must be the first non-phi instruction in its basic block.
**How to avoid:** Use the alloca-then-mem2reg pattern: allocate a local variable, store the result in each branch, load after the join. The LLVM `mem2reg` pass (or `instcombine`) promotes these to phi nodes automatically. This is what Clang and rustc do.
**Warning signs:** Incorrect values from if/else expressions at runtime.

### Pitfall 5: Confusing Code Generation Order With Execution Order
**What goes wrong:** Builder positioned at wrong basic block when generating nested expressions.
**Why it happens:** Generating code for if/else or match creates new basic blocks, moving the builder's insertion point. After generating a branch body, the builder is positioned at the end of that branch's block, not back at the original block.
**How to avoid:** Always save and restore the builder position when generating nested control flow. After creating merge blocks, explicitly `position_at_end` on the merge block before continuing.
**Warning signs:** Instructions appearing in wrong basic blocks; LLVM verification errors.

### Pitfall 6: GC Roots on the Stack
**What goes wrong:** The GC collects live objects because it does not know about stack references.
**Why it happens:** When a GC-managed value (string, closure env) is only referenced from the stack (local variable), the GC cannot see it during collection.
**How to avoid:** For Phase 5's simple GC, use a shadow stack approach: maintain an explicit linked list of stack frames that register GC roots. Each function entry pushes a frame, each function exit pops it. The GC traverses this list during collection. Alternatively, for the initial implementation, use a conservative approach: never collect during Phase 5 (just allocate from a large arena). True collection can be refined in Phase 6.
**Warning signs:** Use-after-free crashes, corrupted string data.

### Pitfall 7: Cross-Platform Linker Differences
**What goes wrong:** Binary compiles on macOS but fails to link on Linux or vice versa.
**Why it happens:** macOS uses the Mach-O format and Apple's linker (`ld64`); Linux uses ELF and GNU `ld` or `lld`. CRT startup objects (`crt0.o`, `crti.o`) differ.
**How to avoid:** Use `cc` as the linker driver on both platforms. It abstracts away platform differences. On macOS, `cc` invokes `clang`; on Linux, it invokes `gcc` or `clang`. Pass `-o output_path` and the object file(s).
**Warning signs:** "undefined symbol: _main" or "cannot find crt1.o" errors.

### Pitfall 8: Inkwell Lifetime and Context Ownership
**What goes wrong:** Borrow checker errors or runtime crashes due to LLVM context lifetime.
**Why it happens:** In Inkwell, `Module`, `Builder`, `FunctionValue`, etc. all borrow from `Context`. If `Context` is dropped while these are alive, undefined behavior occurs. Inkwell enforces this with lifetimes, but it can make struct design tricky.
**How to avoid:** Own the `Context` in the outermost scope (or use `'ctx` lifetime parameter on the codegen struct). The canonical pattern is a `CodeGen<'ctx>` struct that borrows from a `Context` created in `main()` or the compilation entry point.
**Warning signs:** Lifetime errors when trying to store LLVM values in structs.

## Code Examples

### Creating an LLVM Module and Function (Inkwell 0.8)
```rust
// Source: Inkwell 0.8.0 docs + GitHub README
use inkwell::context::Context;
use inkwell::OptimizationLevel;
use inkwell::targets::{
    InitializationConfig, Target, TargetMachine, TargetTriple, CodeModel, RelocMode, FileType,
};

struct CodeGen<'ctx> {
    context: &'ctx Context,
    module: inkwell::module::Module<'ctx>,
    builder: inkwell::builder::Builder<'ctx>,
}

impl<'ctx> CodeGen<'ctx> {
    fn new(context: &'ctx Context, module_name: &str) -> Self {
        let module = context.create_module(module_name);
        let builder = context.create_builder();
        CodeGen { context, module, builder }
    }

    fn compile_function(&self, name: &str) -> inkwell::values::FunctionValue<'ctx> {
        let i64_type = self.context.i64_type();
        let fn_type = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
        let function = self.module.add_function(name, fn_type, None);
        let entry = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry);

        let x = function.get_nth_param(0).unwrap().into_int_value();
        let y = function.get_nth_param(1).unwrap().into_int_value();
        let sum = self.builder.build_int_add(x, y, "sum").unwrap();
        self.builder.build_return(Some(&sum)).unwrap();

        function
    }
}
```

### AOT Compilation: Module to Object File to Binary
```rust
// Source: Inkwell targets API + Compiler Weekly blog
fn compile_to_binary(module: &inkwell::module::Module, output_path: &str) -> Result<(), String> {
    // Initialize the native target
    Target::initialize_native(&InitializationConfig::default())
        .map_err(|e| format!("Failed to initialize native target: {}", e))?;

    let triple = TargetMachine::get_default_triple();
    let target = Target::from_triple(&triple)
        .map_err(|e| format!("Failed to get target: {}", e))?;

    let target_machine = target
        .create_target_machine(
            &triple,
            "generic",         // CPU
            "",                // features
            OptimizationLevel::Default,  // -O2
            RelocMode::Default,
            CodeModel::Default,
        )
        .ok_or("Failed to create target machine")?;

    // Write object file
    let obj_path = format!("{}.o", output_path);
    target_machine
        .write_to_file(&module, FileType::Object, obj_path.as_ref())
        .map_err(|e| format!("Failed to write object file: {}", e))?;

    // Link using system cc
    let status = std::process::Command::new("cc")
        .args(&[&obj_path, "-o", output_path, "-lsnow_rt"])
        .status()
        .map_err(|e| format!("Failed to invoke linker: {}", e))?;

    if !status.success() {
        return Err("Linking failed".to_string());
    }

    // Clean up object file
    std::fs::remove_file(&obj_path).ok();

    Ok(())
}
```

### Tagged Union (ADT) Codegen
```rust
// Source: Research synthesis from LLVM docs + Mapping High Level Constructs
// Creating a sum type like: type Shape do Circle(Float) | Rectangle(Float, Float) | Point end

fn create_sum_type_layout(&self, name: &str, variants: &[MirVariant]) -> inkwell::types::StructType<'ctx> {
    // Calculate max payload size across all variants
    let max_payload_size = variants.iter()
        .map(|v| self.calculate_variant_payload_size(v))
        .max()
        .unwrap_or(0);

    // Base type: { i8 tag, [max_size x i8] payload }
    let tag_type = self.context.i8_type();
    let payload_type = self.context.i8_type().array_type(max_payload_size as u32);
    self.context.struct_type(&[tag_type.into(), payload_type.into()], false)
}

// Pattern matching on a sum type: switch on tag
fn codegen_match_sum_type(&self, scrutinee: PointerValue<'ctx>, arms: &[MatchArm]) {
    let tag_ptr = self.builder.build_struct_gep(
        self.sum_type_layout, scrutinee, 0, "tag_ptr"
    ).unwrap();
    let tag = self.builder.build_load(self.context.i8_type(), tag_ptr, "tag").unwrap();

    let default_bb = self.context.append_basic_block(self.current_fn, "match_default");
    let switch = self.builder.build_switch(
        tag.into_int_value(),
        default_bb,
        &arms.iter().enumerate().map(|(i, arm)| {
            let bb = self.context.append_basic_block(self.current_fn, &format!("arm_{}", i));
            (self.context.i8_type().const_int(i as u64, false), bb)
        }).collect::<Vec<_>>(),
    );
    // Each arm block: GEP into payload with variant-specific struct type
}
```

### If/Else Expression Codegen (Alloca + Mem2Reg Pattern)
```rust
// Source: Standard LLVM codegen pattern (Clang, rustc)
fn codegen_if_expr(&self, cond: &MirExpr, then_: &MirExpr, else_: &MirExpr, result_ty: &MirType) {
    let result_llvm_ty = self.mir_type_to_llvm(result_ty);
    let result_alloca = self.builder.build_alloca(result_llvm_ty, "if_result").unwrap();

    let cond_val = self.codegen_expr(cond);
    let then_bb = self.context.append_basic_block(self.current_fn, "then");
    let else_bb = self.context.append_basic_block(self.current_fn, "else");
    let merge_bb = self.context.append_basic_block(self.current_fn, "if_merge");

    self.builder.build_conditional_branch(cond_val.into_int_value(), then_bb, else_bb).unwrap();

    // Then branch
    self.builder.position_at_end(then_bb);
    let then_val = self.codegen_expr(then_);
    self.builder.build_store(result_alloca, then_val).unwrap();
    self.builder.build_unconditional_branch(merge_bb).unwrap();

    // Else branch
    self.builder.position_at_end(else_bb);
    let else_val = self.codegen_expr(else_);
    self.builder.build_store(result_alloca, else_val).unwrap();
    self.builder.build_unconditional_branch(merge_bb).unwrap();

    // Merge
    self.builder.position_at_end(merge_bb);
    let result = self.builder.build_load(result_llvm_ty, result_alloca, "if_val").unwrap();
    // result is the value of the if expression
}
```

### Closure Creation and Invocation
```rust
// Source: Research synthesis from LLVM discourse + UMD closure conversion
// Closure: fn(x) -> x + captured_y end
//   closure_fn: (env_ptr, x) -> load y from env, add x + y, return
//   creation: malloc env struct, store captured_y, pack {fn_ptr, env_ptr}

fn codegen_make_closure(&self, fn_name: &str, captures: &[(String, MirExpr)]) {
    // 1. Allocate environment struct on GC heap
    let env_types: Vec<_> = captures.iter()
        .map(|(_, expr)| self.mir_type_to_llvm(&expr.ty()))
        .collect();
    let env_struct_ty = self.context.struct_type(&env_types, false);
    let env_size = self.target_data.get_store_size(&env_struct_ty);
    let env_ptr = self.call_runtime("snow_gc_alloc", &[env_size.into()]);

    // 2. Store captured values into environment
    for (i, (_, capture_expr)) in captures.iter().enumerate() {
        let val = self.codegen_expr(capture_expr);
        let field_ptr = self.builder.build_struct_gep(env_struct_ty, env_ptr, i as u32, "cap").unwrap();
        self.builder.build_store(field_ptr, val).unwrap();
    }

    // 3. Pack into closure struct {fn_ptr, env_ptr}
    let closure_ty = self.closure_type();  // { ptr, ptr }
    let closure_alloca = self.builder.build_alloca(closure_ty, "closure").unwrap();
    let fn_val = self.module.get_function(fn_name).unwrap();
    let fn_ptr = fn_val.as_global_value().as_pointer_value();
    self.builder.build_store(
        self.builder.build_struct_gep(closure_ty, closure_alloca, 0, "fn_slot").unwrap(),
        fn_ptr
    ).unwrap();
    self.builder.build_store(
        self.builder.build_struct_gep(closure_ty, closure_alloca, 1, "env_slot").unwrap(),
        env_ptr
    ).unwrap();
}
```

### Runtime Stub: GC-Managed Strings
```rust
// Source: Research recommendation for snow-rt crate
// snow-rt/src/lib.rs - Rust runtime linked into Snow binaries

use std::alloc::{alloc, Layout};
use std::ptr;

/// GC-managed string: length-prefixed, UTF-8, heap-allocated
/// Layout: [u64 len][u8 data...]
#[repr(C)]
pub struct SnowString {
    pub len: u64,
    // data follows immediately after
}

#[no_mangle]
pub extern "C" fn snow_string_new(data: *const u8, len: u64) -> *mut SnowString {
    unsafe {
        let total = std::mem::size_of::<SnowString>() + len as usize;
        let layout = Layout::from_size_align(total, 8).unwrap();
        let ptr = alloc(layout) as *mut SnowString;
        (*ptr).len = len;
        let data_ptr = (ptr as *mut u8).add(std::mem::size_of::<SnowString>());
        ptr::copy_nonoverlapping(data, data_ptr, len as usize);
        // Register with GC here
        ptr
    }
}

#[no_mangle]
pub extern "C" fn snow_string_concat(a: *const SnowString, b: *const SnowString) -> *mut SnowString {
    unsafe {
        let a_len = (*a).len;
        let b_len = (*b).len;
        let new_len = a_len + b_len;
        let result = snow_string_new(ptr::null(), new_len);
        let data_ptr = (result as *mut u8).add(std::mem::size_of::<SnowString>());
        let a_data = (a as *const u8).add(std::mem::size_of::<SnowString>());
        let b_data = (b as *const u8).add(std::mem::size_of::<SnowString>());
        ptr::copy_nonoverlapping(a_data, data_ptr, a_len as usize);
        ptr::copy_nonoverlapping(b_data, data_ptr.add(a_len as usize), b_len as usize);
        result
    }
}

#[no_mangle]
pub extern "C" fn snow_int_to_string(val: i64) -> *mut SnowString {
    let s = val.to_string();
    snow_string_new(s.as_ptr(), s.len() as u64)
}

#[no_mangle]
pub extern "C" fn snow_panic(msg: *const u8, msg_len: u64, file: *const u8, file_len: u64, line: u32) -> ! {
    unsafe {
        let msg = std::str::from_utf8_unchecked(std::slice::from_raw_parts(msg, msg_len as usize));
        let file = std::str::from_utf8_unchecked(std::slice::from_raw_parts(file, file_len as usize));
        eprintln!("Snow panic at {}:{}: {}", file, line, msg);
        std::process::abort();
    }
}

#[no_mangle]
pub extern "C" fn snow_print(s: *const SnowString) {
    unsafe {
        let data = (s as *const u8).add(std::mem::size_of::<SnowString>());
        let slice = std::slice::from_raw_parts(data, (*s).len as usize);
        let text = std::str::from_utf8_unchecked(slice);
        print!("{}", text);
    }
}

#[no_mangle]
pub extern "C" fn snow_println(s: *const SnowString) {
    snow_print(s);
    println!();
}
```

### Decision Tree Pattern Matching Compilation
```rust
// Source: Research synthesis from Maranget 2008 + crumbles.blog
/// A compiled decision tree for pattern matching
pub enum DecisionTree {
    /// Leaf: execute arm body, bind variables
    Leaf {
        arm_index: usize,
        bindings: Vec<(String, MirType, AccessPath)>,
    },
    /// Switch on a constructor tag
    Switch {
        scrutinee: AccessPath,
        cases: Vec<(ConstructorTag, DecisionTree)>,
        default: Option<Box<DecisionTree>>,
    },
    /// Test a literal value
    Test {
        scrutinee: AccessPath,
        value: MirLiteral,
        success: Box<DecisionTree>,
        failure: Box<DecisionTree>,
    },
    /// Guard check (run guard expression, branch on result)
    Guard {
        guard_expr: MirExpr,
        success: Box<DecisionTree>,
        failure: Box<DecisionTree>,
    },
    /// Runtime failure (non-exhaustive match with guards)
    Fail(String, SourceLocation),
}

/// Path to access a sub-component of the scrutinee
pub enum AccessPath {
    Root,
    TupleField(Box<AccessPath>, usize),
    VariantField(Box<AccessPath>, String, usize),
    StructField(Box<AccessPath>, String),
}
```

## Discretionary Recommendations

Based on the research, here are recommendations for the areas left to Claude's discretion:

### MIR vs Direct Lowering: Use MIR
**Recommendation:** Introduce a dedicated MIR.
**Rationale:** The typed Rowan CST is syntax-oriented (rowan TextRanges, trivia tokens, etc.) and does not directly represent desugared constructs. A MIR layer explicitly represents: resolved types (no type variables), monomorphized functions, lowered patterns (decision trees), desugared pipe operators, closure-converted functions with explicit capture lists, and string interpolation segments. This makes the LLVM codegen pass a straightforward mechanical translation.

### Monomorphization vs Type Erasure: Use Monomorphization
**Recommendation:** Monomorphize all generic functions.
**Rationale:** Snow targets native performance. Monomorphization allows LLVM to fully optimize each instantiation. The codebase is not large enough for compile-time to be a concern. Type erasure would require boxing all generic values (introducing heap allocations and pointer indirection), which conflicts with Snow's native-performance goals. Monomorphization also simplifies the GC story since all types have known sizes at compile time.

### ADT Memory Layout: Tagged Unions with Byte Array Payload
**Recommendation:** `{ i8 tag, [max_payload_size x i8] }` base struct with per-variant typed overlays.
**Rationale:** This is the standard approach used by Rust, Zig, and other languages targeting LLVM. The tag is a single byte (supporting up to 256 variants, far more than any practical sum type). The payload is a byte array sized to the largest variant. LLVM's optimization passes can often improve layout. For small enums like `Option<Int>` or `Bool`-like types, this is very efficient.

### Decision Tree vs Backtracking: Decision Trees (Maranget)
**Recommendation:** Use Maranget's decision tree algorithm.
**Rationale:** Decision trees never test the same sub-value twice, producing optimal switch chains. The existing exhaustiveness checker already uses Maranget's usefulness algorithm (Phase 4), so the decision tree compilation is a natural extension. Or-patterns are handled by duplicating arm bodies per the locked decision. Guards generate a `Guard` node that branches to the next alternative on failure. The `Fail` node handles the guarded-arms edge case with panic + source location.

### Closure Capture: Copy Semantics with GC-Managed Environments
**Recommendation:** Copy capture for all values. Environment structs allocated on the GC heap.
**Rationale:** Snow is functional-first with immutable `let` bindings. Copy capture is correct for immutable values and avoids the complexity of reference semantics. The environment struct is heap-allocated and managed by the GC, which aligns with the GC-managed memory model required for Phase 6 compatibility. The closure struct itself is `{ ptr, ptr }` -- function pointer + environment pointer.

### Partial Application: Defer to Future Phase
**Recommendation:** Do not implement partial application or automatic currying in Phase 5.
**Rationale:** Partial application adds significant complexity (intermediate closure creation, argument accumulation). It is not required by any Phase 5 success criteria. The pipe operator (`|>`) handles the most common use case for chaining. Partial application can be added as syntactic sugar in a future phase.

### Pipe Operator: Pure Syntactic Desugar in MIR Lowering
**Recommendation:** Desugar `x |> f` to `f(x)` and `x |> f(y)` to `f(x, y)` during AST-to-MIR lowering.
**Rationale:** No special IR node or codegen support needed. The pipe operator is purely syntactic, resolved during lowering. This follows the same approach as Elixir's pipe operator. The desugaring should handle: `x |> f` -> `f(x)` (bare function reference) and `x |> f(a, b)` -> `f(x, a, b)` (piped as first argument).

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Typed LLVM pointers (`i32*`) | Opaque pointers (`ptr`) | LLVM 15 (2022) | All pointer types are `ptr`; load/store/GEP take explicit pointee type |
| Legacy pass manager | New pass manager | LLVM 16 (2023) | Use `module.run_passes("pass1,pass2", ...)` instead of `PassManager` |
| `bitcast` for pointer conversion | No-op (opaque pointers) | LLVM 15 (2022) | Pointer-to-pointer bitcast is gone; different pointer types are the same `ptr` |
| inkwell 0.4-0.5 (LLVM 4-14) | inkwell 0.8.0 (LLVM 11-21) | Jan 2026 | Latest release; full opaque pointer support |

**Deprecated/outdated:**
- `inkwell::passes::PassManager` with individual `add_*_pass()` methods: deprecated for LLVM 16+. Use `module.run_passes()` with string pass names instead.
- Typed pointer syntax in LLVM IR tutorials: most online tutorials show `%struct.Foo*` style -- this does not work with LLVM 15+.

## Open Questions

1. **TypeckResult structure for codegen consumption**
   - What we know: `TypeckResult` maps `TextRange -> Ty` and provides error/warning lists. The inference engine also builds `StructDefInfo`, `SumTypeDefInfo`, trait registries internally.
   - What's unclear: These internal structures (struct definitions, sum type definitions, trait impls) are currently private to the `infer` module. Codegen needs access to them.
   - Recommendation: Either make these structures public in `snow-typeck` and return them as part of `TypeckResult`, or reconstruct them during MIR lowering from the CST + type map. The former is cleaner.

2. **Entry point discovery for `snowc build <dir>`**
   - What we know: The command is project-based. A Snow program needs a `main` function or equivalent entry point.
   - What's unclear: How does the compiler find the entry point? Is there a manifest file (`snow.toml`)? Or convention-based (`main.snow`, or a `main()` function)?
   - Recommendation: Start with convention: look for `main.snow` in the project directory, require a top-level `main()` function. Manifest file support can be added later.

3. **Runtime library build integration**
   - What we know: `snow-rt` needs to be compiled as a static library (`.a`) and linked into the final binary.
   - What's unclear: Should this be a cargo build script that compiles snow-rt and embeds the `.a` path? Or should snow-rt be compiled separately?
   - Recommendation: Use a Cargo build script in `snow-codegen` (or `snowc`) that compiles `snow-rt` to a static lib and passes the path to the linker. Alternatively, compile snow-rt functions directly as LLVM IR (generate runtime functions in the same module). The latter avoids the static library dance entirely for Phase 5.

4. **Handling `puts`/`printf` for basic I/O**
   - What we know: The success criteria require printing "Hello, World!" to stdout. The runtime has `snow_println`.
   - What's unclear: How does Snow expose I/O to the programmer? Is there a `print()` builtin? Is it `IO.puts()`?
   - Recommendation: For Phase 5, provide `println(string)` as a compiler builtin that calls `snow_println`. Full I/O design is deferred to the standard library phase.

## Sources

### Primary (HIGH confidence)
- [Inkwell 0.8.0 GitHub README](https://github.com/TheDan64/inkwell) - Version, LLVM support range, API patterns
- [Inkwell Builder API docs](https://thedan64.github.io/inkwell/inkwell/builder/struct.Builder.html) - Method signatures for all builder operations
- [Inkwell Module API docs](https://thedan64.github.io/inkwell/inkwell/module/struct.Module.html) - Module methods, target machine API
- [LLVM GC Documentation](https://llvm.org/docs/GarbageCollection.html) - Shadow stack, statepoints, GC strategy
- [Mapping High Level Constructs to LLVM IR - Unions](https://mapping-high-level-constructs-to-llvm-ir.readthedocs.io/en/latest/basic-constructs/unions.html) - Tagged union LLVM IR patterns
- Local LLVM installation verified: LLVM 21.1.8 at `/opt/homebrew/opt/llvm`
- Existing codebase analysis: all source files in `crates/` directory

### Secondary (MEDIUM confidence)
- [Maranget, "Compiling Pattern Matching to Good Decision Trees" (2008)](http://moscova.inria.fr/~maranget/papers/ml05e-maranget.pdf) - Decision tree compilation algorithm
- [Compiler Weekly: LLVM Backend](https://schroer.ca/2021/10/30/cw-llvm-backend/) - AOT compilation workflow with inkwell
- [Create Your Own Programming Language with Rust](https://createlang.rs/01_calculator/basic_llvm.html) - Inkwell tutorial patterns
- [Verdagon: Generics, Compile Times, Type-Erasure](https://verdagon.dev/blog/generics-compile-times) - Monomorphization vs type erasure tradeoffs
- [Rustc Dev Guide: Code generation](https://rustc-dev-guide.rust-lang.org/backend/codegen.html) - Rust's LLVM codegen architecture
- [Writing Interpreters in Rust: Bump Allocation](https://rust-hosted-langs.github.io/book/chapter-simple-bump.html) - GC allocation patterns

### Tertiary (LOW confidence)
- [LLVM Discourse: Implementing closures and continuations](https://discourse.llvm.org/t/implementing-closures-and-continuations/28181) - Closure representation in LLVM (forum post)
- [UMD Assignment 4: Closure conversion, LLVM code emission](https://www.cs.umd.edu/class/fall2017/cmsc430/assignment4.html) - Academic closure conversion approach
- [crumbles.blog: Decision tree pattern matcher](https://crumbles.blog/posts/2025-11-28-extensible-match-decision-tree.html) - Decision tree implementation walkthrough

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - Inkwell version and LLVM version verified directly on the system; API docs checked
- Architecture (MIR design): MEDIUM - Based on rustc patterns and synthesis of multiple sources; specific MIR design is custom
- Architecture (codegen patterns): HIGH - Tagged unions, closures, if/else all verified against LLVM docs and multiple implementations
- Pitfalls: HIGH - Based on direct experience with LLVM and verified documentation
- GC/Runtime: MEDIUM - Runtime design is custom; GC approach based on LLVM docs and interpreter guides

**Research date:** 2026-02-06
**Valid until:** 2026-03-06 (30 days -- inkwell 0.8 is stable, LLVM 21 is current)
