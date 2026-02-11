# Phase 51: HTTP Path Parameters - Research

**Researched:** 2026-02-11
**Domain:** HTTP router extension -- path parameter extraction and method-specific routing in the Snow language runtime
**Confidence:** HIGH

## Summary

Phase 51 extends Snow's existing HTTP router (`router.rs`) with two major capabilities: (1) path parameter matching (`:param` segments) with extraction via `Request.param(req, "id")`, and (2) method-specific route registration (`HTTP.get`, `HTTP.post`, `HTTP.put`, `HTTP.delete`). The existing router supports only exact-match and wildcard (`/*`) patterns, with no HTTP method filtering -- all routes match all methods. The current `RouteEntry` stores only a pattern string and handler function pointer.

The changes span four layers of the Snow compiler/runtime: the runtime router (`snow-rt/src/http/router.rs` and `server.rs`), the LLVM intrinsic declarations (`codegen/intrinsics.rs`), the MIR lowering and name mapping (`mir/lower.rs`), and the type checker builtin registration (`snow-typeck/src/builtins.rs`). Each new function follows the same pattern established by existing HTTP functions: (1) add `snow_` prefixed extern "C" function in the runtime, (2) declare it in `declare_intrinsics`, (3) register the type in `register_builtins`, (4) add the `map_builtin_name` mapping, and (5) add to `known_functions`.

**Primary recommendation:** Implement segment-based matching in the router by splitting pattern and path on `/`, comparing segments pairwise, extracting `:param` captures into a `SnowMap<String, String>` stored on the request, and adding an HTTP method field to `RouteEntry`. The existing `matches_pattern` function is replaced with a richer matching function that returns both a boolean match and a parameter map. Priority ordering (exact > parameterized > wildcard) is achieved by sorting routes at match time or by registration-order with the user registering exact routes first (the existing "first match wins" semantic).

## Standard Stack

### Core (already in use)
| Component | Location | Purpose | Notes |
|-----------|----------|---------|-------|
| `tiny_http` | snow-rt Cargo.toml | HTTP server | Already used; provides `request.method()` and `request.url()` |
| `SnowMap` | snow-rt/src/collections/map.rs | Key-value storage | Already used for query params and headers on `SnowHttpRequest` |
| `SnowOption` | snow-rt/src/option.rs | Option<T> return type | Already used by `request_header` and `request_query` |
| `inkwell` | snow-codegen | LLVM IR generation | Already used for all intrinsic declarations |

### New (no new dependencies needed)
No new crates are required. All path parameter functionality is built using existing runtime primitives: `SnowString`, `SnowMap`, `SnowOption`, and the existing `SnowRouter` / `SnowHttpRequest` structures.

## Architecture Patterns

### Existing Architecture (for reference)

The current flow for HTTP request handling:

```
Snow source:                  Compiler:                   Runtime:
HTTP.router()           -->   http_router             --> snow_http_router()
HTTP.route(r, "/", fn)  -->   http_route              --> snow_http_route(router, pattern, handler_fn)
HTTP.serve(r, 8080)     -->   http_serve              --> snow_http_serve(router, port)
Request.method(req)     -->   request_method           --> snow_http_request_method(req)
Request.query(req, "k") -->   request_query            --> snow_http_request_query(req, name)
```

Module-qualified access pattern (`HTTP.get`, `Request.param`):
1. Parser sees `HTTP.get(...)` as a `FieldAccess` call
2. MIR lowering checks `STDLIB_MODULES` for "HTTP" -> yes
3. Constructs prefixed name: `http_get`
4. `map_builtin_name("http_get")` -> `"snow_http_get"`
5. Codegen emits call to `snow_http_get`

### Pattern 1: Segment-Based Path Matching with Parameter Extraction

**What:** Replace simple string comparison with segment-by-segment matching. Each segment in the pattern is either a literal (must match exactly) or a parameter (`:name`, matches any single segment and captures the value).

**Algorithm:**
```
match_route_with_params(pattern, path) -> Option<HashMap<String, String>>:
  pat_segments = pattern.split('/')
  path_segments = path.split('/')
  if pat_segments.len() != path_segments.len(): return None
  params = HashMap::new()
  for (pat, actual) in zip(pat_segments, path_segments):
    if pat.starts_with(':'):
      params.insert(pat[1..], actual)
    elif pat != actual:
      return None
  return Some(params)
```

**Priority rule (SC-4):** Exact routes take priority over parameterized routes. This is achieved by the existing "first match wins" ordering. The user registers exact routes first, OR we can sort routes at match time: exact segments score higher than param segments. The simplest approach: try all exact-only routes first, then parameterized routes. This can be done by partitioning routes into two groups or by scoring.

**Recommended approach:** Partition `routes` into two passes during `match_route`:
1. First pass: try routes that have NO `:param` segments (exact + wildcard)
2. Second pass: try routes that have `:param` segments

This gives exact-before-parameterized priority automatically without requiring user registration order.

### Pattern 2: Method-Specific Routing

**What:** Each route entry gains an optional HTTP method field. `HTTP.get(...)` registers a route that only matches GET requests. The existing `HTTP.route(...)` remains as a method-agnostic catch-all.

**RouteEntry extension:**
```rust
pub struct RouteEntry {
    pub pattern: String,
    pub method: Option<String>,  // NEW: None = any method, Some("GET") = only GET
    pub handler_fn: *mut u8,
    pub handler_env: *mut u8,
}
```

**Matching logic change:** In `handle_request`, after path matching, also check `entry.method` against the request's HTTP method. If `entry.method` is `None`, it matches any method (backward compatible with existing `HTTP.route`).

### Pattern 3: Path Parameters on the Request Object

**What:** The `SnowHttpRequest` struct gains a new field for path parameters (a `SnowMap`). This is populated by the router during matching and accessed via `Request.param(req, "id")`.

**SnowHttpRequest extension:**
```rust
pub struct SnowHttpRequest {
    pub method: *mut u8,
    pub path: *mut u8,
    pub body: *mut u8,
    pub query_params: *mut u8,
    pub headers: *mut u8,
    pub path_params: *mut u8,  // NEW: SnowMap of path parameters
}
```

**CRITICAL:** This struct is `#[repr(C)]` and accessed from LLVM codegen. Adding a field at the end is safe (no existing code accesses beyond `headers`). The new `Request.param` accessor follows the exact same pattern as `Request.query` and `Request.header`.

### Pattern 4: New Runtime Functions

| Snow API | Prefixed Name | Runtime Function | Signature |
|----------|---------------|------------------|-----------|
| `HTTP.get(r, "/path", handler)` | `http_get` | `snow_http_route_get` | `(router, pattern, handler_fn) -> router` |
| `HTTP.post(r, "/path", handler)` | `http_post` | `snow_http_route_post` | `(router, pattern, handler_fn) -> router` |
| `HTTP.put(r, "/path", handler)` | `http_put` | `snow_http_route_put` | `(router, pattern, handler_fn) -> router` |
| `HTTP.delete(r, "/path", handler)` | `http_delete` | `snow_http_route_delete` | `(router, pattern, handler_fn) -> router` |
| `Request.param(req, "id")` | `request_param` | `snow_http_request_param` | `(req, name) -> Option<String>` |

**NAMING COLLISION:** `HTTP.get` and `HTTP.post` already exist as HTTP client functions (`snow_http_get` and `snow_http_post`). The new routing functions MUST use different runtime names. Options:
1. **`snow_http_route_get`** / **`snow_http_route_post`** -- append `_route_` to disambiguate
2. Change the Snow-level API to `HTTP.get_route` -- but this is ugly

**Recommended:** Use `snow_http_route_get`, `snow_http_route_post`, etc. for the runtime functions. In `map_builtin_name`, the prefixed names from `HTTP.get(r, path, handler)` will be `http_get` -- BUT this already maps to `snow_http_get` (the client). We need to resolve this disambiguation.

**CRITICAL DISAMBIGUATION PROBLEM:** When a user writes `HTTP.get(...)`, the MIR lowering turns it into `http_get`, which currently maps to `snow_http_get` (the HTTP client). But now `HTTP.get` with 3 args should be a route registration, while `HTTP.get` with 1 arg is the client.

**Resolution options:**
1. **Arity-based dispatch at MIR level:** Check argument count: 1 arg -> client, 3 args -> route registration. This is fragile and non-idiomatic.
2. **Different Snow-level names:** Use `HTTP.get_route(r, path, handler)` or `HTTP.on_get(r, path, handler)`. Cleaner but less ergonomic.
3. **Rename client functions:** The client `HTTP.get` becomes `HTTP.fetch(url)` or `HTTP.client_get(url)`. Breaking change.
4. **Overload at the type checker level:** Teach typeck that `http_get` with `(Router, String, Fn)` args is the route variant and with `(String)` is the client variant. This is complex.

**Recommended resolution:** Use different Snow-level names for route registration. The most idiomatic pattern from real frameworks is:
- `HTTP.get(router, "/path", handler)` -- route registration (3 args)
- `HTTP.get(url)` -- client GET request (1 arg)

Since Snow does not currently support function overloading, the cleanest approach is **arity-aware lowering in MIR** -- detect the argument count and map to different runtime functions. The MIR lowering already has the call arguments available when resolving function names. Alternatively, keep the existing client functions and use new names like `HTTP.on_get`, `HTTP.on_post`, etc. for routing.

**Final recommendation: Arity-based dispatch.** The MIR lower already has full argument information when emitting calls. For `http_get` with 1 arg -> `snow_http_get` (client), for `http_get` with 3 args -> `snow_http_route_get` (routing). This is clean because:
- The user writes `HTTP.get(url)` for client AND `HTTP.get(router, "/path", handler)` for routing
- No new naming conventions needed
- Type checker can register both signatures (polymorphic by arity)
- MIR lowering dispatches based on arg count

### Recommended Project Structure (changed files)

```
crates/snow-rt/src/http/
  router.rs        # MODIFY: add method field, segment matching, param extraction
  server.rs        # MODIFY: add path_params to SnowHttpRequest, add request_param accessor
  mod.rs           # MODIFY: export new functions

crates/snow-codegen/src/
  codegen/intrinsics.rs  # MODIFY: declare new runtime functions
  mir/lower.rs           # MODIFY: add arity-based dispatch for HTTP.get/post, add known_functions, add name mappings

crates/snow-typeck/src/
  builtins.rs            # MODIFY: register new type signatures

crates/snowc/tests/
  e2e_stdlib.rs          # MODIFY: add E2E tests

tests/e2e/
  stdlib_http_path_params.snow  # NEW: E2E test fixture
```

### Anti-Patterns to Avoid

- **Adding path params as a separate lookup structure:** Do NOT store params in a global/thread-local HashMap. Store them ON the request object (`SnowHttpRequest.path_params`) so they are per-request and have the same lifetime as the request.
- **Regex-based matching:** Do NOT use regex for path matching. Segment splitting is simpler, faster, and sufficient for `:param` syntax. No new dependencies needed.
- **Breaking backward compatibility:** The existing `HTTP.route(r, path, handler)` MUST continue to work as a method-agnostic route. Do NOT require users to switch to `HTTP.get`/`HTTP.post`.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Path parameter extraction | Custom string parser | Split on `/` and compare segments | Simple, correct, no edge cases |
| Parameter storage | Thread-local or global map | `SnowMap` on the request struct | Per-request lifetime, existing pattern |
| Option<String> return | Custom tagged union | `alloc_option()` from `option.rs` | Exact pattern used by `request_header` and `request_query` |

**Key insight:** Every piece of infrastructure needed already exists. Path params are just another map on the request (like `query_params` and `headers`), and `Request.param` is identical in structure to `Request.query`. The router extension is a straightforward algorithm change.

## Common Pitfalls

### Pitfall 1: SnowHttpRequest Struct Layout Change
**What goes wrong:** Adding `path_params` to `SnowHttpRequest` could break existing field offsets if inserted in the middle.
**Why it happens:** The struct is `#[repr(C)]` and fields are accessed by offset in codegen.
**How to avoid:** Always append new fields at the END of the struct. Existing code never accesses beyond `headers`, so adding `path_params` after `headers` is safe.
**Warning signs:** Existing HTTP tests start failing after the struct change.

### Pitfall 2: HTTP.get/HTTP.post Name Collision with Client Functions
**What goes wrong:** `HTTP.get(url)` (client) and `HTTP.get(router, pattern, handler)` (routing) both lower to `http_get`.
**Why it happens:** The `map_builtin_name` function uses a single string -> string mapping with no arity awareness.
**How to avoid:** Implement arity-based dispatch in MIR lowering, OR use different Snow-level names (e.g., `HTTP.on_get`). The arity-based approach requires intercepting the lowering before `map_builtin_name` is called.
**Warning signs:** Calling `HTTP.get(url)` stops working after adding the routing variant.

### Pitfall 3: Forgetting to Update Type Checker
**What goes wrong:** New functions compile to runtime calls but the type checker rejects the Snow source code.
**Why it happens:** Each new function needs a type signature in `builtins.rs` AND a corresponding entry in `known_functions` in `lower.rs`.
**How to avoid:** Follow the checklist: builtins.rs + intrinsics.rs + lower.rs (known_functions + map_builtin_name) + tests.
**Warning signs:** "unknown function" or type mismatch errors when compiling Snow source.

### Pitfall 4: Wildcard Routes vs Parameterized Routes Interaction
**What goes wrong:** A wildcard route `/api/*` could conflict with a parameterized route `/api/:id`. Need clear precedence.
**Why it happens:** Both match `/api/42` but have different semantics.
**How to avoid:** Define clear precedence: exact > parameterized > wildcard. The two-pass matching (exact-only first, then parameterized) naturally handles this. Wildcard routes (`/*`) should be in the first pass (they are not parameterized, they are catch-alls).
**Warning signs:** Wrong handler called when both wildcard and parameterized routes exist for overlapping paths.

### Pitfall 5: Empty or Trailing-Slash Path Segments
**What goes wrong:** `/users/` and `/users` should both match `/users/:id` pattern? Or not?
**Why it happens:** Splitting `/users/` on `/` gives `["", "users", ""]` (3 segments) while `/users/:id` split gives `["", "users", ":id"]` (3 segments). But actual path `/users/42` gives `["", "users", "42"]`.
**How to avoid:** Normalize paths by stripping trailing slashes before matching. Filter out empty segments from both pattern and path.
**Warning signs:** Routes with trailing slashes don't match.

### Pitfall 6: match_route Now Needs to Return Parameters
**What goes wrong:** The current `match_route` returns `Option<(*mut u8, *mut u8)>` (handler_fn, handler_env). With path params, it also needs to return the extracted parameters.
**Why it happens:** The return type must change.
**How to avoid:** Change `match_route` to return `Option<(*mut u8, *mut u8, HashMap<String, String>)>` or similar. The parameters are then converted to a `SnowMap` in `handle_request` and stored on the request.
**Warning signs:** None -- this is a required API change within the runtime crate.

## Code Examples

### Example 1: Snow Source -- Path Parameters

```snow
fn user_handler(request) do
  case Request.param(request, "id") do
    Some(id) -> HTTP.response(200, "User: " <> id)
    None(_)  -> HTTP.response(400, "Missing id")
  end
end

fn main() do
  let r = HTTP.router()
  let r = HTTP.get(r, "/users/:id", user_handler)
  HTTP.serve(r, 8080)
end
```

### Example 2: Segment-Based Matching (Rust runtime)

```rust
// Source: derived from existing router.rs pattern
fn match_segments(pattern: &str, path: &str) -> Option<Vec<(String, String)>> {
    let pat_segs: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let path_segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if pat_segs.len() != path_segs.len() {
        return None;
    }
    let mut params = Vec::new();
    for (pat, actual) in pat_segs.iter().zip(path_segs.iter()) {
        if pat.starts_with(':') {
            params.push((pat[1..].to_string(), actual.to_string()));
        } else if pat != actual {
            return None;
        }
    }
    Some(params)
}
```

### Example 3: Request.param Accessor (Rust runtime)

```rust
// Source: follows exact pattern of snow_http_request_query in server.rs
#[no_mangle]
pub extern "C" fn snow_http_request_param(req: *mut u8, name: *const SnowString) -> *mut u8 {
    unsafe {
        let request = &*(req as *const SnowHttpRequest);
        let key_str = (*name).as_str();
        let key_snow = snow_string_new(key_str.as_ptr(), key_str.len() as u64);
        let val = map::snow_map_get(request.path_params, key_snow as u64);
        if val == 0 {
            alloc_option(1, std::ptr::null_mut())
        } else {
            alloc_option(0, val as *mut u8)
        }
    }
}
```

### Example 4: Method-Specific Route Registration (Rust runtime)

```rust
// Source: extends existing snow_http_route pattern
#[no_mangle]
pub extern "C" fn snow_http_route_get(
    router: *mut u8,
    pattern: *const SnowString,
    handler_fn: *mut u8,
) -> *mut u8 {
    route_with_method(router, pattern, handler_fn, Some("GET"))
}

fn route_with_method(
    router: *mut u8,
    pattern: *const SnowString,
    handler_fn: *mut u8,
    method: Option<&str>,
) -> *mut u8 {
    let handler_env: *mut u8 = std::ptr::null_mut();
    unsafe {
        let old = &*(router as *const SnowRouter);
        let pat_str = (*pattern).as_str().to_string();
        let mut new_routes = Vec::with_capacity(old.routes.len() + 1);
        for entry in &old.routes {
            new_routes.push(RouteEntry {
                pattern: entry.pattern.clone(),
                method: entry.method.clone(),
                handler_fn: entry.handler_fn,
                handler_env: entry.handler_env,
            });
        }
        new_routes.push(RouteEntry {
            pattern: pat_str,
            method: method.map(|m| m.to_string()),
            handler_fn,
            handler_env,
        });
        let new_router = Box::new(SnowRouter { routes: new_routes });
        Box::into_raw(new_router) as *mut u8
    }
}
```

### Example 5: Arity-Based Dispatch in MIR Lowering

```rust
// In map_builtin_name or the call lowering path:
// When we see "http_get" with 3 args -> route registration
// When we see "http_get" with 1 arg -> client GET
//
// This must be handled BEFORE map_builtin_name since that function
// doesn't have access to argument count. Handle it in lower_call_expr
// when the callee resolves to a module-qualified name.
```

## State of the Art

| Old Approach (current) | New Approach (Phase 51) | Impact |
|------------------------|------------------------|--------|
| Exact + wildcard matching only | Segment-based matching with `:param` capture | REST-style routing |
| No method filtering | Optional method field on RouteEntry | Proper REST semantics |
| No path parameters | `SnowMap` on request, `Request.param` accessor | Dynamic route extraction |
| Single `HTTP.route` for all methods | `HTTP.get`, `HTTP.post`, `HTTP.put`, `HTTP.delete` | Method-specific handlers |

## Open Questions

1. **Arity-based dispatch vs separate names for HTTP.get routing**
   - What we know: `HTTP.get(url)` already exists as a client function. `HTTP.get(router, path, handler)` is the desired routing API.
   - What's unclear: Whether arity-based dispatch in MIR is clean enough, or if we should use different names like `HTTP.on_get`.
   - Recommendation: Start with arity-based dispatch. The MIR lowering has full argument information available. If it proves too complex, fall back to `HTTP.on_get`/`HTTP.on_post` naming.

2. **Type checker registration for overloaded functions**
   - What we know: `builtins.rs` currently registers `http_get` with signature `(String) -> Result<String, String>`. Adding a second signature with `(Router, String, Fn) -> Router` requires handling in the type checker.
   - What's unclear: Whether the current type checker supports multiple signatures for the same name.
   - Recommendation: Investigate whether two entries can coexist. If not, the arity-based approach needs to register the routing variants under different prefixed names (e.g., `http_route_get`) and only do the arity dispatch at MIR level, not at typeck level. This likely means the typeck needs separate names and only the Snow surface syntax (`HTTP.get` with 3 args) triggers the routing path via MIR lowering.

3. **Wildcard route interaction with parameterized routes**
   - What we know: Current wildcards (`/api/*`) match any suffix. Parameterized routes (`/api/:id`) match exactly one segment.
   - What's unclear: Whether both should coexist on the same prefix, and what priority ordering should be.
   - Recommendation: Exact > parameterized > wildcard precedence. Implement via scoring or multi-pass matching.

## Sources

### Primary (HIGH confidence)
- `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/http/router.rs` -- Current router implementation (exact + wildcard matching)
- `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/http/server.rs` -- SnowHttpRequest struct, handle_request flow, request accessors
- `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/http/client.rs` -- Existing HTTP client (snow_http_get, snow_http_post)
- `/Users/sn0w/Documents/dev/snow/crates/snow-codegen/src/codegen/intrinsics.rs` -- All runtime function declarations
- `/Users/sn0w/Documents/dev/snow/crates/snow-codegen/src/mir/lower.rs` (lines 8796-9007) -- STDLIB_MODULES, map_builtin_name, module-qualified lowering
- `/Users/sn0w/Documents/dev/snow/crates/snow-typeck/src/builtins.rs` (lines 522-600) -- HTTP type registrations
- `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/option.rs` -- SnowOption struct and alloc_option helper
- `/Users/sn0w/Documents/dev/snow/.planning/ROADMAP.md` (lines 143-155) -- Phase 51 requirements and success criteria

### Secondary (MEDIUM confidence)
- `/Users/sn0w/Documents/dev/snow/crates/snowc/tests/e2e_stdlib.rs` -- Existing HTTP E2E test patterns
- `/Users/sn0w/Documents/dev/snow/tests/e2e/stdlib_http_server_runtime.snow` -- Snow HTTP server test fixture

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- No new dependencies; all infrastructure exists. Verified by reading source code directly.
- Architecture: HIGH -- Pattern follows existing HTTP function registration pipeline exactly. Every layer (runtime, intrinsics, MIR, typeck) has clear precedent.
- Pitfalls: HIGH -- Name collision identified by reading existing `map_builtin_name`. Struct layout concern verified by reading `#[repr(C)]` usage.
- Arity dispatch: MEDIUM -- This is the novel part. The MIR lowering has arg info available, but implementing arity-based dispatch is new territory for this codebase. May need to fall back to separate names.

**Research date:** 2026-02-11
**Valid until:** 2026-03-11 (stable -- this is internal compiler architecture, not external dependencies)
