# Phase 8: Standard Library - Research

**Researched:** 2026-02-06
**Domain:** Standard library implementation for an LLVM-compiled language with actor runtime -- I/O, strings, collections, file access, HTTP, JSON
**Confidence:** HIGH

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions

#### Module & import design
- Rich prelude: Option, Result, print, println, and common collection functions (map, filter, reduce, head, tail) auto-imported without explicit import statements
- Flat top-level module namespace: `from IO import read_file`, `from List import map`, `from HTTP import serve` -- no `Std.` prefix
- Both import styles supported: `from List import map` for individual functions, `import List` for namespace access (List.map)

#### String & collection APIs
- Collection-first argument convention: `map(list, fn)` -- enables natural pipe chains: `list |> map(fn) |> filter(fn)`
- Full collection suite: List, Map, Set, Tuple utilities, Range, Queue
- Immutable only: all operations return new collections, no mutable variants -- fits actor model (no shared mutable state)

#### I/O & file model
- Result everywhere: all I/O operations return Result types, no panicking convenience variants
- System access included: Env.get("VAR"), Env.args() for CLI argument access

#### HTTP & JSON scope
- Batteries-included HTTP server: built-in routing, middleware chain, static file serving, request parsing
- Actor-per-connection model: each HTTP connection spawns an actor, leveraging Snow's actor runtime (Erlang-style)
- HTTP client included: HTTP.get(url), HTTP.post(url, body) for calling external APIs
- JSON: both dynamic JSON type (for parsing unknown data with pattern matching) and trait-based ToJSON/FromJSON for typed encoding/decoding of known structs

### Claude's Discretion

#### Imports
- Exact import syntax mechanics based on Snow's existing parser module/import support from Phase 2

#### Strings
- String UTF-8 semantics (safe codepoint operations vs raw byte access trade-off)

#### I/O
- File API style (path-based convenience vs handle-based, or combination)
- Console I/O structure (print/println placement, stdout/stderr separation, IO module design)

### Deferred Ideas (OUT OF SCOPE)
None -- discussion stayed within phase scope
</user_constraints>

## Summary

This phase adds a comprehensive standard library to Snow, transforming it from a compiler demo into a language capable of building real web backends and CLI tools. The research covers the implementation architecture, focusing on the critical insight that **Snow's stdlib functions are implemented as Rust `extern "C"` runtime functions in `snow-rt`**, with corresponding type registrations in the type checker and name mappings in the MIR lowerer. This is the established pattern used by `println`, `print`, and all actor runtime functions throughout Phases 5-7.

The codebase analysis reveals that the module/import system is **parsed but not semantically implemented** -- the type checker currently returns `None` for `ImportDecl` and `FromImportDecl` items (line 452 of `infer.rs`). This means implementing `from List import map` style imports requires either: (a) making the module system functional in the type checker to resolve names from stdlib module namespaces, or (b) auto-injecting all stdlib functions into the global scope (treating them like builtins). Given the locked decision for rich prelude + flat namespace, the recommended approach is a **hybrid**: prelude functions are auto-injected into builtins (same as `println` today), while module-qualified access (`List.map`) uses a simple namespace prefix resolution at the type-checker level without full module system semantics.

Collections (List, Map, Set) are implemented as **opaque pointer types at the LLVM level** with all operations handled by `extern "C"` Rust runtime functions. This follows the same pattern as `SnowString` -- the Rust side manages the actual data structure, and Snow programs interact through function calls that take and return opaque pointers. The bump allocator handles memory for these collections since Snow already has no collection (GC), which is adequate for the stdlib scope.

**Primary recommendation:** Implement all stdlib functions as `extern "C"` Rust functions in `snow-rt`, register their types as builtins in `snow-typeck`, map their names in the MIR lowerer, and declare their LLVM signatures in `intrinsics.rs`. Use opaque pointer types for collections. Keep HTTP/JSON in the Rust runtime, not as Snow-level code.

## Standard Stack

### Core (Already in workspace)
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| snow-rt | 0.1.0 | Runtime library (all stdlib implementations live here) | Established pattern -- all runtime functions are extern "C" in this crate |
| snow-typeck | 0.1.0 | Type registrations for stdlib functions | Builtins pattern from `builtins.rs` extends naturally |
| snow-codegen | 0.1.0 | Intrinsic declarations and MIR lowering | `intrinsics.rs` and `map_builtin_name` extend naturally |
| inkwell | 0.8.0 | LLVM bindings (no new LLVM features needed) | Already in workspace |

### New Dependencies (for snow-rt)
| Library | Version | Purpose | Why This Library |
|---------|---------|---------|------------------|
| serde_json | 1.x | JSON parsing/serialization in the runtime | De facto standard Rust JSON library; zero-copy parsing, no_std compatible |
| tiny-http | 0.12.x | Minimal HTTP/1.1 server implementation | Synchronous, zero-dependency HTTP server perfect for actor-per-connection model |
| ureq | 2.x | Minimal HTTP client | Blocking, no-tokio HTTP client -- matches Snow's coroutine-based concurrency model |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| tiny-http | Raw TCP sockets | tiny-http handles HTTP parsing, chunked encoding, keep-alive correctly; hand-rolling is error-prone |
| ureq | reqwest | reqwest pulls in tokio/hyper -- massive dependency tree that conflicts with Snow's coroutine scheduler |
| serde_json | Manual JSON parser | JSON parsing is deceptively complex (Unicode escapes, number precision, nested structures); serde_json is battle-tested |

**Installation:**
```toml
# Add to crates/snow-rt/Cargo.toml [dependencies]
serde_json = "1"
tiny-http = "0.12"
ureq = "2"
```

## Architecture Patterns

### Recommended Project Structure

All stdlib code lives within the existing crate structure -- no new crates needed:

```
crates/
├── snow-rt/src/
│   ├── lib.rs              # Re-export new modules
│   ├── string.rs           # EXTEND: add string operations (length, split, trim, etc.)
│   ├── collections/
│   │   ├── mod.rs           # NEW: collection module
│   │   ├── list.rs          # NEW: List (dynamic array) operations
│   │   ├── map.rs           # NEW: Map (hash map) operations
│   │   ├── set.rs           # NEW: Set (hash set) operations
│   │   └── tuple.rs         # NEW: Tuple utility functions
│   ├── io.rs                # NEW: Console I/O, stdin reading
│   ├── file.rs              # NEW: File I/O operations
│   ├── env.rs               # NEW: Environment variables, CLI args
│   ├── http/
│   │   ├── mod.rs           # NEW: HTTP module
│   │   ├── server.rs        # NEW: HTTP server (tiny-http + actors)
│   │   └── client.rs        # NEW: HTTP client (ureq wrapper)
│   └── json.rs              # NEW: JSON encode/decode
├── snow-typeck/src/
│   └── builtins.rs          # EXTEND: register all stdlib function types
├── snow-codegen/src/
│   ├── codegen/
│   │   └── intrinsics.rs    # EXTEND: declare all stdlib function LLVM signatures
│   └── mir/
│       └── lower.rs         # EXTEND: map_builtin_name for all stdlib functions
└── snowc/src/
    └── main.rs              # No changes needed
```

### Pattern 1: Adding a New Stdlib Function (The Established Pattern)

**What:** Every stdlib function follows this exact 4-step pattern, already proven by `println`/`print`/actor functions.

**When to use:** Every new stdlib function.

**Steps:**

1. **Runtime (snow-rt):** Implement the function as `#[no_mangle] pub extern "C" fn snow_xxx(...) -> ...`
2. **Type checker (builtins.rs):** Register the Snow-facing type: `env.insert("xxx".into(), Scheme::mono(Ty::fun(params, ret)))`
3. **Intrinsics (intrinsics.rs):** Declare the LLVM function signature: `module.add_function("snow_xxx", ...)`
4. **MIR lowering (lower.rs):** Map the Snow name to runtime name in `map_builtin_name`: `"xxx" => "snow_xxx"`

**Example (adding `String.length`):**
```rust
// 1. snow-rt/src/string.rs
#[no_mangle]
pub extern "C" fn snow_string_length(s: *const SnowString) -> i64 {
    unsafe { (*s).len as i64 }
}

// 2. snow-typeck/src/builtins.rs (in register_builtins)
env.insert("string_length".into(),
    Scheme::mono(Ty::fun(vec![Ty::string()], Ty::int())));

// 3. snow-codegen/src/codegen/intrinsics.rs (in declare_intrinsics)
let string_length_ty = i64_type.fn_type(&[ptr_type.into()], false);
module.add_function("snow_string_length", string_length_ty, ...);

// 4. snow-codegen/src/mir/lower.rs (in map_builtin_name)
"string_length" => "snow_string_length".to_string(),
```

### Pattern 2: Opaque Collection Types

**What:** Collections (List, Map, Set) are opaque pointers at the LLVM level, with all operations in Rust runtime code.

**When to use:** Any stdlib type that is not a primitive scalar.

**Design:**

```rust
// snow-rt/src/collections/list.rs

/// A Snow List -- internally a Vec<SnowValue> allocated on the GC arena.
///
/// Layout in memory: pointer to a heap-allocated struct containing:
/// - len: u64
/// - capacity: u64
/// - data: *mut SnowValue (pointer to array of tagged values)
///
/// At the LLVM level, this is just `ptr` (opaque pointer).
#[repr(C)]
pub struct SnowList {
    pub len: u64,
    pub capacity: u64,
    pub element_size: u64,
    pub data: *mut u8,
}
```

**Key insight:** Since Snow is immutable-only and uses GC, every "mutating" operation (push, filter, map) creates a new allocation. The bump allocator handles this without collection -- memory usage grows linearly with operations. This is acceptable for Phase 8 scope; true GC collection is a future optimization.

**LLVM type mapping:** Collections map to `MirType::Ptr` (opaque pointer), just like strings map to `ptr` (pointer to SnowString). The type checker tracks the element type generically (e.g., `List<Int>`), but at the LLVM level it's all `ptr`.

### Pattern 3: Module Namespace Resolution (Prelude + Qualified Access)

**What:** Stdlib functions available both as bare names (prelude) and qualified names (`List.map`).

**When to use:** All stdlib modules.

**Implementation approach:**

The parser already supports both `from List import map` and `import List` syntax (Phase 2, decisions 02-04). The type checker currently ignores these (`Item::ImportDecl(_) | Item::FromImportDecl(_) => None`). The approach:

1. **Prelude functions** (map, filter, reduce, head, tail, print, println, Option, Result): Registered directly in `builtins.rs` with their bare names. These work exactly like `println` works today.

2. **Module-qualified access** (`List.map`, `IO.read_file`, `HTTP.serve`): In the MIR lowerer, when a `FieldAccess` expression has a module name as the object (e.g., `List.map`), lower it as a direct function call to the corresponding runtime function. This requires:
   - A set of known module names in the lowerer (List, Map, Set, IO, File, HTTP, JSON, Env, String)
   - `FieldAccess` on a module name translates to a `Var` with the mapped runtime name

3. **`from X import y` syntax**: In the type checker, when encountering a `FromImportDecl`, look up the module name in a stdlib registry and inject the imported names into the local environment. This is a minimal module system -- no user-defined modules yet, just stdlib modules.

### Pattern 4: Result-Returning I/O Functions

**What:** All I/O operations return `Result<T, String>` to Snow callers, using the existing Result sum type.

**When to use:** File I/O, HTTP operations, any operation that can fail.

**Implementation:**

At the runtime level, Rust functions return a tagged union (matching Snow's Result representation):
```rust
// Runtime side: return a SnowResult struct
#[repr(C)]
pub struct SnowResult {
    pub tag: u8,      // 0 = Ok, 1 = Err
    pub value: *mut u8,  // pointer to the Ok value or error string
}

#[no_mangle]
pub extern "C" fn snow_file_read(path: *const SnowString) -> SnowResult {
    // ... read file, return Result
}
```

The type checker registers these as `Result<String, String>` (or appropriate types), and the existing pattern matching infrastructure handles destructuring the results in Snow code.

### Pattern 5: Actor-Per-Connection HTTP Server

**What:** HTTP server spawns a Snow actor for each incoming connection.

**When to use:** The HTTP.serve function.

**Implementation:**

The HTTP server runs in the Rust runtime using `tiny-http`. When a request arrives:
1. The runtime receives the HTTP request on its listener thread
2. It serializes the request into a Snow-compatible message (method, path, headers, body as Snow strings)
3. It calls `snow_actor_spawn` to create a new actor that runs the user's handler function
4. It sends the request data as a message to the actor
5. The actor processes the request and sends a response message back
6. The runtime sends the HTTP response

This leverages the existing actor infrastructure (spawn, send, receive) without any new primitives.

### Anti-Patterns to Avoid

- **Implementing collections in Snow source code:** Snow has no import/module resolution for user code yet. All stdlib functions must be Rust runtime functions called via FFI. Do NOT try to write Snow-level map/filter implementations that the compiler loads.
- **Using async Rust for HTTP:** Snow has its own coroutine-based concurrency (corosensei). Using tokio/async-std would conflict with the scheduler. Use synchronous blocking I/O that can yield to the Snow scheduler.
- **Creating a new crate for stdlib:** All runtime functions belong in `snow-rt`. Creating a `snow-stdlib` crate would add unnecessary complexity to the build and link pipeline.
- **Implementing full module resolution:** A general-purpose module system (resolving user-defined modules, managing visibility) is out of scope. Only resolve stdlib module names, not arbitrary user modules.

## Don't Hand-Roll

Problems that look simple but have existing solutions:

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| JSON parsing | Custom recursive descent parser | `serde_json` via Rust FFI | JSON has subtle edge cases (Unicode escapes, number precision, deeply nested structures, duplicate keys). serde_json handles them all correctly |
| HTTP request parsing | Manual TCP + header parsing | `tiny-http` library | HTTP/1.1 has complex semantics (chunked transfer encoding, keep-alive, content-length negotiation, header folding). tiny-http handles these correctly |
| HTTP client | Raw socket connections | `ureq` library | HTTPS/TLS negotiation, redirect following, timeout handling, chunked decoding are all error-prone to implement manually |
| Hash map implementation | Open addressing from scratch | Rust's `HashMap` wrapped in extern C | Rust's HashMap uses SwissTable (Robin Hood hashing) which is highly optimized; reimplementing would be slower and buggier |
| UTF-8 string operations | Byte-level manipulation | Rust's `str` methods via FFI | UTF-8 codepoint boundaries, grapheme clusters, case conversion are extremely complex; Rust's stdlib handles them correctly |

**Key insight:** Snow's runtime is Rust. Every stdlib "implementation" is really a thin `extern "C"` wrapper around Rust's stdlib or a well-tested Rust crate. The compilation pipeline only needs to emit correct FFI calls.

## Common Pitfalls

### Pitfall 1: Type Representation Mismatch Between Type Checker and Runtime

**What goes wrong:** The type checker registers `List.map` as `(List<A>, (A) -> B) -> List<B>` with generic type parameters, but the runtime function signature uses raw pointers. If the type representation doesn't match at monomorphization time, codegen emits incorrect LLVM IR.

**Why it happens:** The type checker operates on `Ty` (polymorphic types with type variables), while MIR and LLVM use concrete, monomorphized types. Collections introduce higher-kinded-like types (`List<A>`) that must monomorphize to concrete pointer types.

**How to avoid:** Register collection types as `Ty::App(Con("List"), [T])` in the type checker (same pattern as `Option<T>`). In `mir/types.rs`, resolve `List<anything>` to `MirType::Ptr` (opaque pointer). The type parameter is erased at the LLVM level -- it only exists for compile-time type checking.

**Warning signs:** Runtime segfaults when calling collection operations; LLVM verification errors about type mismatches.

### Pitfall 2: Memory Layout for Passing Structs Across FFI

**What goes wrong:** Snow-side structs (e.g., HTTP Request) and Rust-side structs have different memory layouts, causing field access corruption.

**Why it happens:** LLVM and Rust may pad/align struct fields differently. A Rust struct with `#[repr(Rust)]` has no guaranteed layout.

**How to avoid:** Use `#[repr(C)]` on all FFI-visible structs. Alternatively, pass complex data as individual arguments rather than as structs. For HTTP requests, pass method, path, headers, body as separate Snow strings rather than as a packed struct.

**Warning signs:** Garbled field values, segfaults on struct field access across FFI boundary.

### Pitfall 3: Closure Passing to Higher-Order Runtime Functions

**What goes wrong:** `list |> map(fn(x) -> x + 1 end)` requires passing a Snow closure to a Rust runtime function. The closure is a `{fn_ptr, env_ptr}` pair, not a regular function pointer.

**Why it happens:** Snow closures are represented as `{ ptr, ptr }` structs (fn_ptr + env_ptr). The runtime `snow_list_map` function needs to call this closure for each element, which means it must understand the closure calling convention (env_ptr as first argument).

**How to avoid:** Runtime higher-order functions accept a closure struct (two pointers): `snow_list_map(list: *const SnowList, fn_ptr: *const u8, env_ptr: *const u8) -> *mut SnowList`. The runtime calls the function pointer with env_ptr as the first argument, followed by the element. This matches the lifted closure calling convention from Phase 5 (decision 05-02: "Closures lifted with __env first param").

**Warning signs:** Closures work for direct calls but crash when passed to map/filter/reduce.

### Pitfall 4: Blocking I/O Without Scheduler Yield

**What goes wrong:** A file read or HTTP request blocks the OS thread, preventing other actors on that thread from running.

**Why it happens:** Snow's scheduler uses coroutines on a pool of OS threads. If a runtime function makes a blocking syscall (read, connect), the entire OS thread stalls, starving all actors on that thread.

**How to avoid:** For Phase 8, accept this limitation with documentation. The actor-per-connection HTTP model means each connection gets its own coroutine, and `tiny-http` handles connections on its own thread pool. File I/O operations are typically fast enough that temporary blocking is acceptable. True non-blocking I/O integration with the scheduler is a Phase 9+ optimization.

**Warning signs:** Actor programs become unresponsive when performing file I/O or HTTP requests.

### Pitfall 5: Collection Operations with Incompatible Element Types

**What goes wrong:** `List.map` on a `List<Int>` with a function `(Int) -> String` should return `List<String>`, but the runtime uses opaque pointers and doesn't track element types.

**Why it happens:** At the LLVM level, all collections are opaque pointers. The runtime doesn't know element types -- it just moves bytes around.

**How to avoid:** Use a tagged-value representation for collection elements. Each element is stored as `{ u8 tag, u64 value }` where the tag indicates the type (Int, Float, Bool, String/Ptr). The runtime can then correctly handle elements of different sizes. Alternatively, store all elements as 8-byte values (i64 for Int/Bool, f64 for Float, ptr for String/collections) with an element-size parameter tracked in the list header.

**Warning signs:** Wrong values after map/filter operations that change element types.

### Pitfall 6: Prelude Naming Conflicts with User Functions

**What goes wrong:** User defines `fn map(...)` in their code, which conflicts with the prelude's auto-imported `map` from List.

**Why it happens:** Rich prelude puts many names (map, filter, reduce, head, tail) into the global scope.

**How to avoid:** User definitions should shadow prelude names. The type checker already supports this -- `env.insert` with the same name replaces the previous binding. Document that user functions shadow prelude functions, and qualified access (`List.map`) always works.

**Warning signs:** User-defined `map` function causes type errors because it's being checked against the prelude's `map` signature.

## Code Examples

### Example 1: How println Works End-to-End (Established Pattern)

This is the existing pattern that ALL stdlib functions will follow:

```rust
// 1. RUNTIME: crates/snow-rt/src/string.rs
#[no_mangle]
pub extern "C" fn snow_println(s: *const SnowString) {
    unsafe { println!("{}", (*s).as_str()); }
}

// 2. TYPE CHECKER: crates/snow-typeck/src/builtins.rs
env.insert("println".into(),
    Scheme::mono(Ty::fun(vec![Ty::string()], Ty::Tuple(vec![]))));

// 3. INTRINSICS: crates/snow-codegen/src/codegen/intrinsics.rs
let println_ty = void_type.fn_type(&[ptr_type.into()], false);
module.add_function("snow_println", println_ty, Some(Linkage::External));

// 4. MIR LOWERING: crates/snow-codegen/src/mir/lower.rs
fn map_builtin_name(name: &str) -> String {
    match name {
        "println" => "snow_println".to_string(),
        "print" => "snow_print".to_string(),
        _ => name.to_string(),
    }
}

// 5. KNOWN FUNCTIONS: crates/snow-codegen/src/mir/lower.rs (in Lowerer::new)
self.known_functions.insert("println".to_string(),
    MirType::FnPtr(vec![MirType::String], Box::new(MirType::Unit)));
```

### Example 2: List.map with Closure (Target API)

What the user writes:
```snow
fn main() do
  let numbers = [1, 2, 3, 4, 5]
  let doubled = numbers |> map(fn(x) -> x * 2 end)
  println("${List.length(doubled)}")
end
```

What this compiles to (conceptually):
```
; MIR
Call { func: snow_list_from_array, args: [1, 2, 3, 4, 5], ty: Ptr }
MakeClosure { fn: __closure_0, captures: [], ty: Closure }
Call { func: snow_list_map, args: [list_ptr, closure_fn_ptr, closure_env_ptr], ty: Ptr }
Call { func: snow_list_length, args: [result_ptr], ty: Int }
; ... int_to_string, println ...
```

### Example 3: File I/O with Result Handling (Target API)

What the user writes:
```snow
fn main() do
  let result = File.read("input.txt")
  case result do
    Ok(contents) -> println("Read: ${contents}")
    Err(msg) -> println("Error: ${msg}")
  end
end
```

### Example 4: HTTP Server (Target API)

What the user writes:
```snow
fn main() do
  HTTP.serve(8080, fn(request) ->
    case request.path do
      "/" -> HTTP.Response(200, "Hello, World!")
      "/json" ->
        let data = Map.new()
          |> Map.put("name", "Snow")
          |> Map.put("version", "0.1")
        HTTP.Response(200, JSON.encode(data))
      _ -> HTTP.Response(404, "Not Found")
    end
  end)
end
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Separate stdlib crate | Stdlib in runtime crate (`snow-rt`) | Snow design decision | All stdlib functions are `extern "C"` wrappers in the existing runtime; no build system changes needed |
| Full module system for stdlib | Prelude + simple namespace resolution | Phase 8 design decision | Avoids implementing general module semantics; stdlib works via builtins registration |
| Async HTTP (tokio/hyper) | Synchronous HTTP (tiny-http) + actor-per-connection | Phase 8 design decision | Aligns with Snow's coroutine scheduler; avoids tokio dependency conflict |

**Current codebase state (what exists):**
- `snow_println` and `snow_print` are the only I/O functions
- `SnowString` with `snow_string_new`, `snow_string_concat`, `snow_int_to_string`, `snow_float_to_string`, `snow_bool_to_string` exist
- No collection types (List, Map, Set) exist
- No file I/O exists
- No HTTP or JSON exists
- Module/import declarations are parsed but semantically ignored (type checker returns `None`)
- The 4-step pattern (runtime function -> type registration -> intrinsic declaration -> name mapping) is fully established and used for ~30 functions across string, actor, and supervisor domains

## Discretion Recommendations

### String UTF-8 Semantics

**Recommendation: Safe codepoint operations, not raw byte access.**

Rationale:
- Snow's `SnowString` is already documented as UTF-8 (`as_str()` calls `from_utf8_unchecked`)
- Elixir/Erlang precedent: String operations work on Unicode codepoints, with explicit binary operations for raw bytes
- For Phase 8, implement codepoint-level operations (length returns codepoint count, split operates on Unicode boundaries, slice works on codepoint indices)
- Do NOT expose raw byte access -- it's error-prone and rarely needed for web backend/CLI use cases
- `String.length("hello")` returns 5 (codepoints), not byte count
- This means `String.length` is O(n) for general UTF-8 strings, but this is the correct semantic for user-facing code

### File API Style

**Recommendation: Path-based convenience functions with Result returns.**

Rationale:
- The locked decision is "Result everywhere" for I/O operations
- Elixir-style: `File.read(path)` returns `Result<String, String>`, `File.write(path, content)` returns `Result<(), String>`
- Handle-based API (open/read/close) is unnecessary for Phase 8 scope -- path-based is simpler and sufficient for CLI tools and web backends
- Example API:
  - `File.read(path)` -> `Result<String, String>` -- read entire file as string
  - `File.write(path, content)` -> `Result<(), String>` -- write string to file (creates/overwrites)
  - `File.append(path, content)` -> `Result<(), String>` -- append to file
  - `File.exists(path)` -> `Bool` -- check if path exists
  - `File.delete(path)` -> `Result<(), String>` -- delete file
  - `File.read_lines(path)` -> `Result<List<String>, String>` -- read file as list of lines

### Console I/O Structure

**Recommendation: Keep print/println in prelude, add IO module for stdin and stderr.**

Rationale:
- `print` and `println` are already builtins and work. Keep them as-is.
- Add `IO.read_line()` -> `Result<String, String>` for stdin input
- Add `IO.eprintln(msg)` for stderr output (useful for logging in servers)
- The existing `snow_print`/`snow_println` functions in `string.rs` stay where they are
- New stdin/stderr functions go in a new `io.rs` module in `snow-rt`

### Import Syntax Mechanics

**Recommendation: Minimal namespace resolution, not full module system.**

The parser already handles:
- `from List import map` -> `FromImportDecl` with module "List" and imported name "map"
- `import List` -> `ImportDecl` with module "List"

Implementation approach:
1. In the type checker, maintain a `stdlib_modules: HashMap<String, HashMap<String, Scheme>>` mapping module names to their exported functions
2. When encountering `FromImportDecl`, look up the module and inject the specified names into the current scope
3. When encountering `ImportDecl`, record the module name as available for qualified access
4. In the MIR lowerer, when seeing `FieldAccess` where the object is a known module name, resolve to the qualified function name (e.g., `List.map` -> `list_map` -> `snow_list_map`)

This avoids implementing general-purpose module resolution while supporting the locked decision for both import styles.

## Open Questions

Things that couldn't be fully resolved:

1. **List literal syntax**
   - What we know: The user wants `[1, 2, 3]` to create a List. The parser currently uses `[T]` for type annotations (generic params use `<T>` since Phase 3).
   - What's unclear: Whether `[1, 2, 3]` is already parseable or needs new syntax support in the lexer/parser. Square brackets are used for generic args in some older phases but were migrated to angle brackets in 03-01.
   - Recommendation: Check if `[expr, expr, ...]` can be parsed. If not, add `LIST_EXPR` to the parser. Alternative: use `List.of(1, 2, 3)` as a constructor function.

2. **Generic type parameters for collections in the type checker**
   - What we know: `Option<T>` and `Result<T, E>` already work with generic inference. `List<T>` follows the same pattern.
   - What's unclear: Whether higher-order functions like `map(list, fn)` where the return type differs from the input type (`List<A>` -> `List<B>`) require any new unification logic.
   - Recommendation: This should work with existing let-polymorphism. Register `map` as `forall A B. (List<A>, (A) -> B) -> List<B>` and let the inference engine handle it. Test this early.

3. **Tagged value representation for heterogeneous collection operations**
   - What we know: At the LLVM level, all collection elements are raw memory. The runtime needs to know element sizes for copy/allocation.
   - What's unclear: Whether a uniform 8-byte representation (all elements stored as u64/f64/ptr) is sufficient, or whether variable-size elements (structs, tuples) need additional indirection.
   - Recommendation: Use a uniform 8-byte element representation initially. Scalars (Int, Bool, Float) stored directly. Pointers (String, List, Map, structs) stored as pointer values. This matches LLVM's representation where all these types are either i64, f64, or ptr (all 8 bytes on 64-bit).

4. **HTTP server integration with actor scheduler**
   - What we know: The actor runtime uses corosensei coroutines on a thread pool. tiny-http uses its own thread for listening.
   - What's unclear: Exact integration point -- does the HTTP listener run on a separate OS thread that spawns actors via `snow_actor_spawn`, or does it integrate into the scheduler's work-stealing pool?
   - Recommendation: Run tiny-http's listener on a dedicated OS thread (outside the actor scheduler). When requests arrive, use `snow_actor_spawn` to create handler actors. This is the simplest integration and matches Erlang's model where the acceptor pool is separate from the worker pool.

5. **JSON dynamic type representation**
   - What we know: The user wants both dynamic JSON (pattern-matchable) and typed JSON (ToJSON/FromJSON traits).
   - What's unclear: How to represent the dynamic JSON type in Snow's type system. It needs to be a sum type with variants (Null, Bool, Number, String, Array, Object).
   - Recommendation: Define a `Json` sum type in the stdlib prelude. The user can pattern match on it: `case JSON.parse(str) do Ok(Json.Object(map)) -> ... end`. The type checker already supports sum types with nested data.

## Sources

### Primary (HIGH confidence)
- Snow codebase analysis (all source files in `/Users/sn0w/Documents/dev/snow/crates/`) -- direct code reading of current implementation
- Phase 5, 6, 7 research documents and summaries -- established patterns and decisions
- `builtins.rs`, `intrinsics.rs`, `lower.rs` -- the 4-step builtin function pattern

### Secondary (MEDIUM confidence)
- [Elixir pipe operator and collection-first design](https://elixirschool.com/en/lessons/basics/pipe_operator) -- Elixir's API design principles for pipe-friendly functions
- [serde_json](https://github.com/serde-rs/json) -- Rust JSON library design patterns (trait-based encode/decode)
- [tiny-http](https://github.com/tiny-http/tiny-http) -- minimal synchronous HTTP server for Rust
- [Rust FFI / The Rustonomicon](https://doc.rust-lang.org/nomicon/ffi.html) -- `extern "C"` function patterns for runtime FFI

### Tertiary (LOW confidence)
- LLVM data structures overview -- general patterns for collection implementation in LLVM-targeted languages
- HTTP server library comparison -- relative suitability of tiny-http vs alternatives for embedded runtime use

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all stdlib code extends the existing `snow-rt` crate using the proven 4-step pattern; no new tools or frameworks needed
- Architecture: HIGH -- direct codebase analysis confirms the builtin function pattern, type registration approach, and LLVM type mapping strategy
- Pitfalls: HIGH -- pitfalls derived from direct analysis of the existing closure calling convention, FFI boundary constraints, and scheduler model
- Collection design: MEDIUM -- the opaque-pointer + tagged-value approach is sound but the exact element representation needs validation during implementation
- HTTP integration: MEDIUM -- actor-per-connection model is architecturally sound but the exact tiny-http + scheduler integration point needs prototyping
- JSON dynamic type: MEDIUM -- sum type representation is natural but the trait-based ToJSON/FromJSON may require new trait infrastructure beyond what exists

**Research date:** 2026-02-06
**Valid until:** Indefinite (Snow is a self-contained project; no external API drift concerns)
