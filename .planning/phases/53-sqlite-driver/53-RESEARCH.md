# Phase 53: SQLite Driver - Research

**Researched:** 2026-02-11
**Domain:** SQLite C FFI integration, database driver runtime, compiler pipeline extension
**Confidence:** HIGH

## Summary

Phase 53 adds a SQLite database driver to the Snow language. Users interact with it via a `Sqlite` module (`Sqlite.open`, `Sqlite.query`, `Sqlite.execute`, `Sqlite.close`). The implementation follows the established pattern for adding new stdlib modules to Snow: runtime functions in `snow-rt` (extern C), intrinsics registration in `codegen/intrinsics.rs`, known_functions in `mir/lower.rs`, module type signatures in `typeck/infer.rs`, and the builtin name mapping in `map_builtin_name`.

SQLite is bundled into the compiled binary using `libsqlite3-sys` with the `bundled` feature, which compiles the SQLite C amalgamation from source using the `cc` crate. This produces a static library that is linked into `libsnow_rt.a`, meaning compiled Snow programs have zero external SQLite dependencies.

The core technical challenge is the handle lifetime pattern: SQLite connection handles are heap-allocated with `Box::into_raw()` (not GC-allocated), returned as opaque `u64` values to Snow code, and recovered with `Box::from_raw()` on close. This matches the existing `SnowRouter` pattern in `snow-rt/src/http/router.rs`. The GC has no finalizer support, so explicit `Sqlite.close(conn)` is required.

**Primary recommendation:** Use `libsqlite3-sys` with `bundled` feature for the C library. Write raw FFI wrapper functions in a new `snow-rt/src/db/sqlite.rs` module. Parameters are passed as `List<String>` (all values bound as text via `sqlite3_bind_text`) for simplicity. Query results return `List<Map<String, String>>` with all column values as strings.

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| `libsqlite3-sys` | 0.36.x | SQLite C FFI bindings + bundled amalgamation source | The standard Rust crate for SQLite C API bindings. Used by `rusqlite`. The `bundled` feature compiles SQLite 3.51.x from the included C amalgamation via the `cc` crate, producing a static library with zero system dependencies. |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| `cc` | 1.x | C compiler invocation (build dependency of `libsqlite3-sys`) | Automatically used by `libsqlite3-sys` when `bundled` feature is enabled. Not a direct dependency of `snow-rt`. |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| `libsqlite3-sys` (raw FFI) | `rusqlite` (high-level wrapper) | `rusqlite` adds Rust-level safety (Connection, Statement types) but adds ~15K lines of dependency. Raw FFI is thinner, matches existing Snow runtime patterns (all extern C functions operate on raw pointers), and avoids double-wrapping (Snow runtime already wraps everything in its own extern C layer). |
| `libsqlite3-sys` bundled | System `libsqlite3-dev` | System library requires `apt install libsqlite3-dev` on Linux. Bundled compiles from source, achieving SQLT-06 (zero system dependencies). |
| All params as `List<String>` | Params as `List<SnowJson>` (typed binding) | SnowJson-tagged params enable `sqlite3_bind_int64` for integers and `sqlite3_bind_double` for floats. But this requires users to wrap values: `[Json.from_int(18)]` instead of `["18"]`. The FEATURES.md shows `List<String>` as the simple MVP approach. The success criteria show bare `[18]` and `["Alice"]`, which suggests the simpler string-only approach where the compiler/runtime handles coercion. |

**Dependency addition:**
```toml
# In snow-rt/Cargo.toml [dependencies]
libsqlite3-sys = { version = "0.36", features = ["bundled"] }
```

## Architecture Patterns

### Recommended File Structure
```
crates/snow-rt/src/
  db/
    mod.rs              # pub mod sqlite;
    sqlite.rs           # SQLite C FFI wrapper functions (extern "C")
  lib.rs                # Add: pub mod db; and re-exports

crates/snow-codegen/src/
  codegen/intrinsics.rs  # Add: snow_sqlite_* LLVM declarations
  mir/lower.rs           # Add: known_functions + map_builtin_name entries

crates/snow-typeck/src/
  infer.rs               # Add: Sqlite module in stdlib_modules() + STDLIB_MODULE_NAMES
  builtins.rs            # Add: SqliteConn opaque type + sqlite_* function signatures

crates/snow-codegen/src/
  link.rs                # May need -lsqlite3 or may be handled by static linking
```

### Pattern 1: Opaque Handle for Non-GC Resources
**What:** Database connections are heap-allocated with `Box::into_raw()`, not GC-allocated.
**When to use:** Any resource that requires explicit cleanup (file descriptors, network sockets, database handles).
**Why:** The Snow GC has no finalizer/destructor mechanism. A GC-collected connection handle would leak the SQLite file descriptor forever.
**Example (from existing `snow_http_router`):**
```rust
// Source: crates/snow-rt/src/http/router.rs lines 194-199
pub extern "C" fn snow_http_router() -> *mut u8 {
    let router = Box::new(SnowRouter { routes: Vec::new(), middlewares: Vec::new() });
    Box::into_raw(router) as *mut u8
}
```

For SQLite, the same pattern:
```rust
struct SqliteConn {
    db: *mut sqlite3,  // Raw C pointer from sqlite3_open_v2
}

pub extern "C" fn snow_sqlite_open(path: *const SnowString) -> *mut u8 {
    // ... sqlite3_open_v2 ...
    let conn = Box::new(SqliteConn { db });
    let handle = Box::into_raw(conn) as u64;
    // Return as SnowResult { tag: 0, value: handle as *mut u8 }
    alloc_result(0, handle as *mut u8) as *mut u8
}

pub extern "C" fn snow_sqlite_close(conn_handle: u64) {
    let conn = unsafe { Box::from_raw(conn_handle as *mut SqliteConn) };
    unsafe { sqlite3_close(conn.db); }
    // Box drops, freeing Rust memory
}
```

### Pattern 2: SnowResult Return Convention
**What:** All fallible operations return `*mut SnowResult` (tag 0 = Ok, tag 1 = Err). Err payload is always `*mut SnowString`.
**When to use:** Every SQLite operation that can fail (open, query, execute).
**Example (from existing crate):**
```rust
// Source: crates/snow-rt/src/io.rs lines 17-20
#[repr(C)]
pub struct SnowResult {
    pub tag: u8,
    pub value: *mut u8,
}
```

### Pattern 3: Module Registration Pipeline
**What:** Adding a new stdlib module requires changes in 4 files in a specific order.
**Step-by-step:**

1. **`snow-typeck/src/infer.rs` - `stdlib_modules()`** (~line 211): Add `Sqlite` module with function type signatures
2. **`snow-typeck/src/infer.rs` - `STDLIB_MODULE_NAMES`** (~line 615): Add `"Sqlite"` to the array
3. **`snow-typeck/src/builtins.rs` - `register_builtins()`**: Add `SqliteConn` opaque type and `sqlite_*` function signatures
4. **`snow-codegen/src/mir/lower.rs` - `known_functions`** (~line 655): Register function signatures with MirType
5. **`snow-codegen/src/mir/lower.rs` - `map_builtin_name()`** (~line 8818): Map `sqlite_open` -> `snow_sqlite_open`, etc.
6. **`snow-codegen/src/mir/lower.rs` - `STDLIB_MODULES`** (~line 8808): Add `"Sqlite"` to array
7. **`snow-codegen/src/codegen/intrinsics.rs` - `declare_intrinsics()`**: Declare LLVM function types

### Pattern 4: Handle as u64 (SQLT-07)
**What:** The connection handle is stored as a `u64` in Snow code. At the MIR level it's `MirType::Int`. At the LLVM level it's `i64`.
**Why:** GC safety. A raw pointer stored in a Snow variable as `Ptr` might be interpreted as a GC-managed pointer. Using `u64`/`Int` makes it opaque to the GC -- the GC never traverses integer values.
**Implementation:** `SqliteConn` opaque type in typeck maps to `MirType::Int` in MIR, and `i64` in LLVM IR. The runtime functions accept `u64` conn handles instead of `*mut u8`.

### Anti-Patterns to Avoid
- **GC-allocating connection handles:** Never use `snow_gc_alloc_actor` for SqliteConn. The GC has no finalizers -- collected handles leak file descriptors forever.
- **Forgetting to finalize statements:** Every `sqlite3_prepare_v2` must have a matching `sqlite3_finalize` in the same function, even on error paths.
- **Missing SQLITE_TRANSIENT:** When binding text parameters, always use `SQLITE_TRANSIENT` (-1) as the destructor parameter to `sqlite3_bind_text`. This tells SQLite to copy the string data before the bind call returns, which is essential because Snow strings may be GC-relocated or freed.
- **Not null-terminating C strings:** SQLite C API expects null-terminated C strings. Snow's `SnowString` is length-prefixed, not null-terminated. Must convert with `CString::new()` or manually null-terminate.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| SQLite C compilation | Custom build.rs with cc crate | `libsqlite3-sys` with `bundled` feature | The crate handles all platform-specific compilation flags, SQLite compile-time options, and static library generation. It ships the SQLite amalgamation (sqlite3.c + sqlite3.h) embedded in the crate. |
| SQLite FFI bindings | Manual `extern "C" { fn sqlite3_open(...) }` declarations | `libsqlite3-sys` re-exports | The crate provides correctly-typed Rust bindings for all SQLite C functions with proper `c_int`, `c_char` types. Pregenerated bindings avoid `bindgen` at build time. |
| C string conversion | Manual null termination | `std::ffi::CString` | Handles null-byte checking, proper allocation, and Drop-based cleanup. |
| Error message extraction | Manual pointer arithmetic | `std::ffi::CStr::from_ptr()` | Safely converts C strings from `sqlite3_errmsg` to Rust `&str`. |

**Key insight:** `libsqlite3-sys` with `bundled` handles the entire SQLite build and FFI binding problem. The Snow runtime only needs to write the wrapper functions that convert between Snow types (SnowString, SnowList, SnowMap, SnowResult) and the `libsqlite3-sys` C types.

## Common Pitfalls

### Pitfall 1: Statement Leak on Error Path
**What goes wrong:** `sqlite3_prepare_v2` succeeds but a bind or step fails. If the error handler returns early without calling `sqlite3_finalize`, the prepared statement leaks.
**Why it happens:** Error handling with early returns before cleanup.
**How to avoid:** Use a pattern where `sqlite3_finalize` is called in all exit paths, or use a Rust `Drop` guard:
```rust
struct StmtGuard { stmt: *mut sqlite3_stmt }
impl Drop for StmtGuard {
    fn drop(&mut self) { unsafe { sqlite3_finalize(self.stmt); } }
}
```
**Warning signs:** SQLite "unable to close due to unfinalized statements" errors.

### Pitfall 2: String Encoding Mismatch
**What goes wrong:** Snow strings are UTF-8 byte sequences. SQLite expects null-terminated C strings. Passing a SnowString pointer directly to sqlite3_open causes reading past the string boundary.
**Why it happens:** SnowString is `{ len: u64, data: [u8] }`, not null-terminated.
**How to avoid:** Always convert via `CString::new(snow_string.as_str())` or `CString::new(std::str::from_utf8(slice))`.
**Warning signs:** Garbage characters in SQL queries, segfaults.

### Pitfall 3: Connection Handle Type Mismatch
**What goes wrong:** If the connection handle is stored as `MirType::Ptr` instead of `MirType::Int`, the GC may try to trace through it as a pointer, leading to corruption.
**Why it happens:** Other opaque types (Router, Request) are Ptr because they don't need GC safety (they're only used within runtime functions, not stored in Snow variables across GC cycles). But SqliteConn is stored in a Snow `let` binding and survives across multiple calls.
**How to avoid:** Use `MirType::Int` / `i64` for the handle. The typechecker type is `SqliteConn` (opaque), but it lowers to `i64` at the MIR level.
**Warning signs:** Segfaults after GC collection cycles when using database connections.

### Pitfall 4: Linker Flag Complexity
**What goes wrong:** After adding `libsqlite3-sys` with `bundled` to `snow-rt/Cargo.toml`, the static library `libsnow_rt.a` now contains or depends on the compiled SQLite. The linker invocation in `link.rs` may need additional flags.
**Why it happens:** Static library linking has different requirements per platform.
**How to avoid:** Test the full build pipeline (snowc build -> link -> run) on the target platform. The `bundled` feature of `libsqlite3-sys` compiles SQLite into the static lib, so it should be self-contained. But on some platforms, additional system libraries may be needed (e.g., `-lpthread`, `-ldl` on Linux).
**Warning signs:** Undefined symbol errors for `sqlite3_*` functions during linking.

### Pitfall 5: Thread Safety
**What goes wrong:** SQLite connections are not thread-safe by default. In Snow's actor model, multiple actors on different OS threads could share a connection handle.
**Why it happens:** Snow's M:N scheduler runs actors on multiple OS threads.
**How to avoid:** Document that connections should not be shared between actors. SQLite's default threading mode is "serialized" when compiled with `SQLITE_THREADSAFE=1` (the default), which adds mutex locking around all operations. This is safe but slow for concurrent access. For the MVP, this is acceptable.
**Warning signs:** Database corruption, "database is locked" errors.

## Code Examples

Verified patterns from the existing Snow codebase:

### Runtime Function Registration (from HTTP module)
```rust
// Source: crates/snow-rt/src/http/router.rs lines 194-199
#[no_mangle]
pub extern "C" fn snow_http_router() -> *mut u8 {
    let router = Box::new(SnowRouter { routes: Vec::new(), middlewares: Vec::new() });
    Box::into_raw(router) as *mut u8
}
```

### Intrinsics Declaration (from HTTP module)
```rust
// Source: crates/snow-codegen/src/codegen/intrinsics.rs lines 417-424
// snow_http_router() -> ptr
module.add_function("snow_http_router", ptr_type.fn_type(&[], false),
    Some(inkwell::module::Linkage::External));

// snow_http_serve(router: ptr, port: i64) -> void
module.add_function("snow_http_serve",
    void_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
    Some(inkwell::module::Linkage::External));
```

### Module Registration in Typechecker (from HTTP module)
```rust
// Source: crates/snow-typeck/src/infer.rs lines 459-505
let mut http_mod = HashMap::new();
http_mod.insert("router".to_string(), Scheme::mono(Ty::fun(vec![], router_t.clone())));
http_mod.insert("serve".to_string(), Scheme::mono(Ty::fun(
    vec![router_t.clone(), Ty::int()], Ty::Tuple(vec![])
)));
modules.insert("HTTP".to_string(), http_mod);
```

### Known Functions in MIR Lowering (from HTTP module)
```rust
// Source: crates/snow-codegen/src/mir/lower.rs lines 656-658
self.known_functions.insert("snow_http_router".to_string(),
    MirType::FnPtr(vec![], Box::new(MirType::Ptr)));
self.known_functions.insert("snow_http_serve".to_string(),
    MirType::FnPtr(vec![MirType::Ptr, MirType::Int], Box::new(MirType::Unit)));
```

### Name Mapping (from HTTP module)
```rust
// Source: crates/snow-codegen/src/mir/lower.rs lines 8977-8979
"http_router" => "snow_http_router".to_string(),
"http_route" => "snow_http_route".to_string(),
"http_serve" => "snow_http_serve".to_string(),
```

### Expected SQLite Runtime API
```rust
// Target: crates/snow-rt/src/db/sqlite.rs
use libsqlite3_sys::*;

struct SqliteConn { db: *mut sqlite3 }

#[no_mangle]
pub extern "C" fn snow_sqlite_open(path: *const SnowString) -> *mut u8 {
    // 1. Convert SnowString to CString
    // 2. sqlite3_open_v2(path_cstr, &db, READWRITE|CREATE, null)
    // 3. On success: Box::into_raw(Box::new(SqliteConn { db })) as u64
    // 4. Return SnowResult { tag: 0, value: handle }
    // 5. On error: sqlite3_errmsg(db) -> SnowResult { tag: 1, value: err_string }
}

#[no_mangle]
pub extern "C" fn snow_sqlite_close(conn_handle: u64) {
    // 1. Box::from_raw(conn_handle as *mut SqliteConn)
    // 2. sqlite3_close(conn.db)
    // 3. Box drops automatically
}

#[no_mangle]
pub extern "C" fn snow_sqlite_execute(
    conn_handle: u64,
    sql: *const SnowString,
    params: *mut u8,  // SnowList of SnowString params
) -> *mut u8 {  // SnowResult<Int, String>
    // 1. Recover SqliteConn from handle
    // 2. sqlite3_prepare_v2
    // 3. Bind params from SnowList as text
    // 4. sqlite3_step (expect SQLITE_DONE)
    // 5. sqlite3_changes for affected rows
    // 6. sqlite3_finalize
    // 7. Return SnowResult { tag: 0, value: rows_affected as i64 }
}

#[no_mangle]
pub extern "C" fn snow_sqlite_query(
    conn_handle: u64,
    sql: *const SnowString,
    params: *mut u8,  // SnowList of SnowString params
) -> *mut u8 {  // SnowResult<List<Map<String, String>>, String>
    // 1. Recover SqliteConn from handle
    // 2. sqlite3_prepare_v2
    // 3. Bind params
    // 4. Get column names via sqlite3_column_count + sqlite3_column_name
    // 5. Loop sqlite3_step while SQLITE_ROW:
    //    - For each column: sqlite3_column_text -> SnowString
    //    - Build SnowMap<String, String> per row
    //    - Append to SnowList
    // 6. sqlite3_finalize
    // 7. Return SnowResult { tag: 0, value: list_ptr }
}
```

### Expected Snow User Code
```snow
fn main() do
  let conn = Sqlite.open("test.db")?
  Sqlite.execute(conn, "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, age INTEGER)", [])?
  Sqlite.execute(conn, "INSERT INTO users (name, age) VALUES (?, ?)", ["Alice", "30"])?
  let rows = Sqlite.query(conn, "SELECT name, age FROM users", [])?
  # rows :: List<Map<String, String>>
  println(rows.to_string())
  Sqlite.close(conn)
end
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| System sqlite3 linkage (`-lsqlite3`) | `libsqlite3-sys` bundled (compiled from amalgamation) | Established pattern | Zero system dependencies; binary is self-contained |
| Raw `extern "C"` FFI declarations | `libsqlite3-sys` re-exports typed bindings | Standard since rusqlite 0.10+ | Correct type signatures, no manual `c_int`/`c_char` declarations needed |

**Deprecated/outdated:**
- The architecture doc (ARCHITECTURE.md) shows manual `extern "C"` FFI declarations for `sqlite3_*` functions. With `libsqlite3-sys`, these are provided by the crate and should be imported, not redeclared.
- The architecture doc mentions a `build.rs` with `println!("cargo:rustc-link-lib=sqlite3")`. With `bundled` feature, this is handled automatically by `libsqlite3-sys`.

## Open Questions

1. **Parameter type strategy: String-only vs SnowJson-tagged**
   - What we know: The success criteria show `[18]` (Int) and `["Alice"]` (String) as params. The FEATURES.md suggests `List<String>` for simplicity. The ARCHITECTURE.md recommends `List<SnowJson>` for typed binding. The current Snow type system requires homogeneous lists (`List<Int>` or `List<String>`, not mixed).
   - What's unclear: Whether `Sqlite.query(conn, "...", [18])` means the 18 is auto-coerced to string at the MIR level, or whether params should accept `List<String>` and users write `["18"]`.
   - Recommendation: Use `List<String>` in the type signature. Users pass `["18"]` not `[18]`. All params bound via `sqlite3_bind_text`. This is the simplest approach, matches Go's `database/sql` vanilla pattern, and avoids heterogeneous list issues. The success criteria examples may be aspirational -- the FEATURES.md "What NOT to include" section explicitly says "Params as `List<String>`." If typed binding is desired later, it can be added as a separate overload.

2. **Handle type at MIR level: Int vs Ptr**
   - What we know: SQLT-07 says "opaque u64 values safe from GC collection." The Router type uses `MirType::Ptr`. But Router is only held within runtime functions, never stored in Snow variables across GC cycles.
   - What's unclear: Whether `SqliteConn` should be `MirType::Int` (u64/i64) or `MirType::Ptr`. If Ptr, the GC might trace through it. If Int, it's GC-safe but loses pointer semantics.
   - Recommendation: Use `MirType::Int` (maps to `i64` in LLVM IR). The runtime functions accept `u64` for the connection handle. The typechecker has an opaque `SqliteConn` type that is lowered to `Int` in MIR. This satisfies SQLT-07.

3. **Linker changes needed?**
   - What we know: `link.rs` invokes `cc` with `-L rt_dir -lsnow_rt`. When `libsqlite3-sys` bundled is used, SQLite is compiled and linked into `libsnow_rt.a` (as a static library dependency).
   - What's unclear: Whether the final linker step needs additional flags (like `-ldl -lpthread` on Linux).
   - Recommendation: Test on the target platform. On macOS, SQLite doesn't need extra flags. On Linux, may need `-ldl` and `-lpthread`. The `link.rs` may need platform-conditional flags.

## Sources

### Primary (HIGH confidence)
- **Snow codebase** -- Direct reading of:
  - `crates/snow-rt/src/http/router.rs` (Box::into_raw handle pattern, lines 194-199)
  - `crates/snow-rt/src/http/client.rs` (SnowResult return convention)
  - `crates/snow-rt/src/json.rs` (SnowJson tagged union, alloc_result helper)
  - `crates/snow-rt/src/io.rs` (SnowResult struct definition, lines 17-20)
  - `crates/snow-rt/src/lib.rs` (module organization, re-exports)
  - `crates/snow-codegen/src/codegen/intrinsics.rs` (LLVM function declarations)
  - `crates/snow-codegen/src/mir/lower.rs` (known_functions, map_builtin_name, STDLIB_MODULES)
  - `crates/snow-typeck/src/infer.rs` (stdlib_modules(), STDLIB_MODULE_NAMES)
  - `crates/snow-typeck/src/builtins.rs` (register_builtins, opaque type registration)
  - `crates/snow-codegen/src/link.rs` (linker invocation)
  - `crates/snow-rt/Cargo.toml` (current dependencies)
  - `.planning/research/ARCHITECTURE.md` (SQLite FFI design, handle patterns, anti-patterns)
  - `.planning/research/FEATURES.md` (SQLite API surface, parameter strategy, scope)
  - `.planning/REQUIREMENTS.md` (SQLT-01 through SQLT-07)
  - `.planning/ROADMAP.md` (Phase 53 success criteria)

### Secondary (MEDIUM confidence)
- [libsqlite3-sys on crates.io](https://crates.io/crates/libsqlite3-sys) -- Version 0.36.x, bundled feature documentation
- [rusqlite/libsqlite3-sys build.rs](https://github.com/rusqlite/rusqlite/blob/master/libsqlite3-sys/build.rs) -- Build system implementation
- [SQLite C/C++ Interface](https://sqlite.org/cintro.html) -- Official C API documentation

### Tertiary (LOW confidence)
- None

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- `libsqlite3-sys` with `bundled` is the standard approach in Rust. The SQLite C API is stable and well-documented. The Snow codebase pattern for adding modules is well-established across 52 phases.
- Architecture: HIGH -- The exact file locations, function signatures, and registration patterns are verified by reading the current codebase. The handle pattern matches existing `SnowRouter` implementation.
- Pitfalls: HIGH -- Pitfalls are derived from the architecture doc's anti-patterns section and verified against the actual GC implementation in `gc.rs`.

**Research date:** 2026-02-11
**Valid until:** 2026-03-11 (stable domain -- SQLite C API rarely changes, Snow codebase patterns are established)
