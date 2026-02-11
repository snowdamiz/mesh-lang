# Phase 52: HTTP Middleware - Research

**Researched:** 2026-02-11
**Domain:** HTTP middleware pipeline -- composable request/response processing functions in the Snow language runtime and compiler
**Confidence:** HIGH

## Summary

Phase 52 adds middleware support to Snow's HTTP router, enabling users to wrap request handling with composable functions for cross-cutting concerns (logging, auth, headers). The middleware follows the "onion model" pattern: each middleware function receives the request and a `next` function, can inspect/modify the request before calling `next`, and can inspect/modify the response after `next` returns. Multiple middleware compose in registration order (first added = outermost layer).

The implementation spans the same four layers as Phase 51: (1) the runtime router/server (`snow-rt/src/http/router.rs` and `server.rs`), where the `SnowRouter` struct gains a middleware list and `handle_request` builds a middleware chain; (2) LLVM intrinsic declarations (`codegen/intrinsics.rs`); (3) MIR lowering and name mapping (`mir/lower.rs`); and (4) type checker builtin registration (`snow-typeck/src/builtins.rs` and `infer.rs`). The pattern is identical to the Phase 51 pipeline extension -- one new runtime function (`snow_http_use_middleware`) and one new Snow API (`HTTP.use(router, middleware_fn)`).

The critical design challenge is how the `next` function is provided to the middleware. At the Snow level, a middleware is `fn(request, next) -> response` where `next` is `Fn(Request) -> Response`. At the runtime level, the `next` function must be a callable that either invokes the next middleware in the chain or the final route handler. The runtime constructs this `next` closure dynamically at request time using Rust closures converted to Snow's `{ fn_ptr, env_ptr }` closure struct representation.

**Primary recommendation:** Store middleware functions (as fn_ptr/env_ptr pairs) on the SnowRouter. At request dispatch time in `handle_request`, build the middleware chain by wrapping the innermost handler (the matched route handler) with each middleware in reverse order, producing a single callable that the outermost middleware invokes. The `next` function for each middleware layer is a runtime-constructed closure that either calls the next middleware or the final handler.

## Standard Stack

### Core (already in use)
| Component | Location | Purpose | Notes |
|-----------|----------|---------|-------|
| `tiny_http` | snow-rt Cargo.toml | HTTP server | Already used; no changes needed |
| `SnowRouter` | snow-rt/src/http/router.rs | Route storage + matching | Will gain a `middlewares` vector |
| `SnowHttpRequest` | snow-rt/src/http/server.rs | Request representation | No struct changes needed |
| `SnowHttpResponse` | snow-rt/src/http/server.rs | Response representation | No struct changes needed |
| `inkwell` | snow-codegen | LLVM IR generation | Already used for all intrinsic declarations |

### New (no new dependencies needed)
No new crates are required. The middleware chain is built at request time using existing runtime primitives. The `next` function is constructed using Snow's existing closure representation (`{ fn_ptr, env_ptr }` struct) with a Rust-side trampoline function.

## Architecture Patterns

### Existing Architecture (for reference -- from Phase 51)

```
Snow source:                       Compiler:                   Runtime:
HTTP.router()                  --> http_router             --> snow_http_router()
HTTP.route(r, "/path", fn)     --> http_route              --> snow_http_route(router, pattern, handler_fn)
HTTP.on_get(r, "/path", fn)    --> http_on_get             --> snow_http_route_get(router, pattern, handler_fn)
HTTP.serve(r, 8080)            --> http_serve              --> snow_http_serve(router, port)
Request.method(req)            --> request_method           --> snow_http_request_method(req)
Request.param(req, "id")       --> request_param           --> snow_http_request_param(req, name)
```

Module-qualified lowering path:
1. Typeck sees `HTTP.use(...)` -> looks up `"use"` in http_mod -> gets type `(Router, Fn(Request, Fn(Request) -> Response) -> Response) -> Router`
2. MIR lowering sees `HTTP.use(...)` -> STDLIB_MODULES match -> constructs prefixed name `http_use` -> `map_builtin_name("http_use")` -> `"snow_http_use_middleware"`
3. Codegen emits call to `snow_http_use_middleware` with closure-split args

### Pattern 1: Middleware Storage on Router

**What:** The `SnowRouter` struct gains a `middlewares` vector storing middleware function pointers. Each middleware entry is a `(fn_ptr, env_ptr)` pair, matching the existing handler storage pattern.

**SnowRouter extension:**
```rust
pub struct MiddlewareEntry {
    pub fn_ptr: *mut u8,
    pub env_ptr: *mut u8,
}

pub struct SnowRouter {
    pub routes: Vec<RouteEntry>,
    pub middlewares: Vec<MiddlewareEntry>,  // NEW
}
```

**Registration:** `HTTP.use(router, middleware_fn)` creates a new router with the middleware appended to the list (immutable semantics, same as `HTTP.route`). Middleware is registered with `fn_ptr` and `env_ptr` (closure split by codegen).

### Pattern 2: Middleware Chain Construction at Request Time

**What:** When a request arrives and a route matches, `handle_request` builds a callable chain from the middleware list. The innermost function is the matched route handler. Each middleware wraps the next function.

**Algorithm (outermost-first execution):**
```
Given middlewares [m1, m2, m3] and route handler h:
  next_3 = h                           // innermost: the route handler
  next_2 = |req| m3(req, next_3)       // m3 wraps h
  next_1 = |req| m2(req, next_2)       // m2 wraps m3
  result  = m1(request, next_1)         // m1 wraps m2 (outermost)
```

This is the standard onion model: m1 runs first (outermost), can call next_1 to proceed to m2, which can call next_2 to proceed to m3, which calls next_3 (the handler). Responses flow back in reverse order.

**Registration order = execution order:** First middleware registered via `HTTP.use` is outermost (runs first on request, last on response). This matches SC-3 from the roadmap: "first added = outermost."

### Pattern 3: Runtime `next` Function Construction

**What:** The `next` function passed to each middleware must be callable from Snow code using the standard closure calling convention: `fn_ptr(env_ptr, request) -> response`. The runtime constructs these at request time.

**Implementation approach -- Trampoline with boxed state:**

The `next` function for each middleware layer is a Rust `extern "C"` trampoline function that:
1. Receives `(env_ptr, request_ptr) -> response_ptr`
2. Casts `env_ptr` to a boxed struct containing the remaining middleware chain state
3. Calls the next middleware (or final handler) with the request

```rust
/// State for a middleware chain "next" trampoline.
struct MiddlewareChainState {
    /// Remaining middleware entries to process (from the current position onward).
    remaining: Vec<MiddlewareEntry>,
    /// The final route handler.
    handler_fn: *mut u8,
    handler_env: *mut u8,
}

/// Trampoline function that the Snow middleware calls as `next(request)`.
/// Calling convention: `fn(env_ptr, request_ptr) -> response_ptr`
extern "C" fn middleware_chain_trampoline(env_ptr: *mut u8, request_ptr: *mut u8) -> *mut u8 {
    unsafe {
        let state = &*(env_ptr as *const MiddlewareChainState);
        if state.remaining.is_empty() {
            // Base case: call the route handler
            call_handler(state.handler_fn, state.handler_env, request_ptr)
        } else {
            // Recursive case: call next middleware with a new "next" function
            let current = &state.remaining[0];
            let next_state = Box::new(MiddlewareChainState {
                remaining: state.remaining[1..].to_vec(),
                handler_fn: state.handler_fn,
                handler_env: state.handler_env,
            });
            let next_env = Box::into_raw(next_state) as *mut u8;
            let next_fn = middleware_chain_trampoline as *mut u8;
            // Build a Snow closure struct { fn_ptr, env_ptr } for the "next" function
            let next_closure = build_snow_closure(next_fn, next_env);
            // Call current middleware: middleware(request, next_closure)
            call_middleware(current.fn_ptr, current.env_ptr, request_ptr, next_closure)
        }
    }
}
```

### Pattern 4: Snow Closure Struct for `next`

**What:** The `next` function must be a Snow-compatible closure value. Snow closures are `{ fn_ptr: *mut u8, env_ptr: *mut u8 }` structs at the LLVM level. The runtime must allocate and populate this struct to pass `next` to middleware.

**The Snow closure struct is defined in codegen/types.rs:**
```rust
/// Closure type: { ptr, ptr } -- fn_ptr at index 0, env_ptr at index 1
pub fn closure_type(context: &Context) -> StructType {
    let ptr_type = context.ptr_type(AddressSpace::default());
    context.struct_type(&[ptr_type.into(), ptr_type.into()], false)
}
```

At the runtime level, this is just two adjacent pointers (16 bytes on 64-bit). The runtime can allocate this using `snow_gc_alloc_actor` (like SnowHttpRequest) and populate the two fields:

```rust
fn build_snow_closure(fn_ptr: *mut u8, env_ptr: *mut u8) -> *mut u8 {
    unsafe {
        // Allocate 16 bytes: two pointers
        let closure = snow_gc_alloc_actor(16, 8) as *mut *mut u8;
        *closure = fn_ptr;           // field 0: fn_ptr
        *closure.add(1) = env_ptr;   // field 1: env_ptr
        closure as *mut u8
    }
}
```

### Pattern 5: Middleware Calling Convention

**What:** A middleware function in Snow has signature `fn(Request, Fn(Request) -> Response) -> Response`. At the runtime level, after closure splitting by the codegen, this becomes:

- **If middleware is a bare function (no captures):** The codegen passes it as a single `fn_ptr` to the runtime. The runtime calls it as `fn_ptr(request, next_closure) -> response`.
- **If middleware is a closure (has captures):** The codegen splits it into `(fn_ptr, env_ptr)`. The runtime calls it as `fn_ptr(env_ptr, request, next_closure) -> response`.

Wait -- this is where the existing pattern diverges. Looking at how `snow_http_route` handles handlers: the route registration takes `handler_fn` as a single ptr (the codegen does NOT split the closure because the known_functions type is `MirType::Ptr`). The `handler_env` is always null.

**Critical question:** Should `HTTP.use` support closures as middleware, or only bare named functions?

**Analysis of the existing HTTP handler pattern:**
- `HTTP.route(r, "/path", handler)` takes `handler` as `Fn(Request) -> Response` in typeck
- At the MIR level, `snow_http_route` known_functions has handler as `MirType::Ptr` (not Closure)
- Codegen sees `MirType::Ptr`, does NOT split the closure
- Runtime receives a single fn_ptr, env_ptr is always null
- This means only bare named functions work as HTTP handlers (no closures with captures)

**For middleware, the same pattern applies.** The simplest approach: register `snow_http_use_middleware` with handler as `MirType::Ptr` in known_functions, receive fn_ptr only, env_ptr = null. This means middleware must be bare named functions (not closures). This matches the existing handler pattern and is consistent.

**If we want closure support later,** we would need to change the intrinsic to accept `(router, fn_ptr, env_ptr)` and register the handler as `MirType::Closure(...)` in known_functions so the codegen splits it. But this is a broader change that would also affect `HTTP.route`, `HTTP.on_get`, etc. Defer this.

**Recommended: Follow the existing handler pattern.** Middleware functions are bare named functions, passed as a single fn_ptr. The `next` function passed by the runtime IS a closure (fn_ptr + env_ptr via the trampoline), so the middleware CAN call `next(request)` using Snow's closure calling convention.

### Pattern 6: The `next` Function's Type in Snow

**What:** The middleware function signature in Snow is `fn(request, next) -> response` where `next` has type `Fn(Request) -> Response`. When the middleware calls `next(request)`, the Snow compiler sees it as a closure call (since `next` is a function parameter, not a known named function). The codegen emits `MirExpr::ClosureCall`, which extracts fn_ptr and env_ptr from the closure struct and calls `fn_ptr(env_ptr, request)`.

This is exactly what we need: the runtime constructs a `{ fn_ptr, env_ptr }` closure struct where fn_ptr is the trampoline function and env_ptr points to the middleware chain state. When the Snow middleware calls `next(request)`, the codegen emits a closure call that correctly invokes the trampoline with the chain state.

### Recommended Project Structure (changed files)

```
crates/snow-rt/src/http/
  router.rs        # MODIFY: add MiddlewareEntry, middlewares vec to SnowRouter,
                   #          snow_http_use_middleware function
  server.rs        # MODIFY: add middleware chain execution in handle_request,
                   #          trampoline function, build_snow_closure helper
  mod.rs           # MODIFY: export snow_http_use_middleware

crates/snow-codegen/src/
  codegen/intrinsics.rs  # MODIFY: declare snow_http_use_middleware
  mir/lower.rs           # MODIFY: add known_functions, map_builtin_name entries

crates/snow-typeck/src/
  builtins.rs      # MODIFY: register http_use type signature
  infer.rs         # MODIFY: add "use" to http_mod

crates/snowc/tests/
  e2e_stdlib.rs    # MODIFY: add E2E test for middleware

tests/e2e/
  stdlib_http_middleware.snow  # NEW: E2E test fixture
```

### Anti-Patterns to Avoid

- **Storing closures from Snow directly in the router without proper representation:** The handler/middleware fn_ptr is just a raw pointer. Do NOT try to reconstruct Snow closure structs; store raw fn_ptr + env_ptr separately.
- **Building the middleware chain at registration time:** The chain MUST be built at request time because it wraps the matched route handler, which is only known after routing. Do NOT pre-compose middleware.
- **Thread-local or global middleware state:** Store middleware ON the router (same pattern as routes). This is per-router and immutable after creation.
- **Modifying SnowHttpRequest struct layout for middleware:** No new fields are needed on SnowHttpRequest. Middleware operates on existing request/response types.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Closure struct allocation | Manual memory layout | `snow_gc_alloc_actor` for 16-byte struct | GC-managed, same as other Snow values |
| Middleware ordering | Custom sorting/priority | Registration order (Vec append) | Requirements say "first added = outermost" |
| `next` function construction | Complex code generation | Runtime trampoline + chain state | No compiler changes needed for `next`; it is a regular closure value constructed at runtime |

**Key insight:** The `next` function is NOT generated by the Snow compiler. It is constructed entirely at runtime in Rust code, using Snow's closure struct representation (`{ fn_ptr, env_ptr }`). The Snow program simply calls `next(request)` and the existing closure-call codegen handles it correctly. This means the middleware feature requires ZERO new codegen patterns -- only a new runtime function and the standard pipeline wiring (intrinsics + typeck + MIR lowering).

## Common Pitfalls

### Pitfall 1: Middleware Chain Memory Management
**What goes wrong:** Each request allocates MiddlewareChainState boxes and Snow closure structs for the `next` functions. These must not be freed before the middleware finishes executing.
**Why it happens:** The chain state is heap-allocated per-request and could be freed prematurely.
**How to avoid:** Allocate chain state with `Box::into_raw` and closure structs with `snow_gc_alloc_actor`. The GC-allocated closures are managed by Snow's garbage collector. The Box-allocated chain state should be leaked (not freed) since it lives for the duration of the request handler actor, which has a short lifetime. Alternatively, use `snow_gc_alloc_actor` for chain state too.
**Warning signs:** Segfaults during middleware execution, especially with multiple middleware layers.

### Pitfall 2: Closure Splitting for `HTTP.use`
**What goes wrong:** If the codegen splits the middleware function argument into `(fn_ptr, env_ptr)`, the intrinsic must accept 4 args `(router, fn_ptr, env_ptr)`. If it does NOT split (like `HTTP.route`), the intrinsic accepts 2 args `(router, fn_ptr)` and env_ptr is always null.
**Why it happens:** The closure-splitting logic in `codegen_call` checks if `arg.ty()` is `MirType::Closure(_, _)`. If known_functions types the arg as `MirType::Ptr`, no splitting occurs.
**How to avoid:** Follow the EXACT same pattern as `snow_http_route`: register in known_functions with `MirType::Ptr` for the middleware arg. This means no closure splitting, fn_ptr only. Middleware must be bare named functions (not closures with captures). This is consistent with the existing handler pattern.
**Warning signs:** LLVM linker error about wrong number of arguments to `snow_http_use_middleware`.

### Pitfall 3: `next` Closure Must Match Snow's Calling Convention
**What goes wrong:** If the runtime constructs the `next` closure struct incorrectly, the Snow-side closure call will crash.
**Why it happens:** Snow's `codegen_closure_call` always calls `fn_ptr(env_ptr, ...args)`. The trampoline must expect `(env_ptr, request_ptr)`.
**How to avoid:** The trampoline function signature MUST be `extern "C" fn(env_ptr: *mut u8, request_ptr: *mut u8) -> *mut u8`. The closure struct MUST have fn_ptr at offset 0 and env_ptr at offset 8 (on 64-bit). Use `snow_gc_alloc_actor(16, 8)` to allocate and write both pointers.
**Warning signs:** Segfault when middleware calls `next(request)`.

### Pitfall 4: Router Immutability -- `HTTP.use` Must Return NEW Router
**What goes wrong:** Mutating the existing router would break the immutable semantics of Snow.
**Why it happens:** Temptation to modify the router in-place.
**How to avoid:** `snow_http_use_middleware` must create a NEW SnowRouter, copying existing routes AND middlewares, then appending the new middleware. Return the new router pointer. Same pattern as `route_with_method`.
**Warning signs:** Earlier middleware registrations disappear; routes break after adding middleware.

### Pitfall 5: Middleware Execution With No Route Match
**What goes wrong:** If no route matches (404), should middleware still run?
**Why it happens:** Ambiguity in the spec.
**How to avoid:** Per the requirements, middleware wraps "request handling." The standard pattern (Express, Actix, Django) is that middleware runs BEFORE routing, on every request. But the requirements say "runs on every request" (SC-1). The simplest correct approach: run middleware on every request, wrapping a "next" that either invokes the matched handler or returns a 404 response. This matches Express/Koa/Django behavior.
**Warning signs:** Middleware not called for 404 paths, or middleware called but then 404 response bypasses middleware's post-processing.

### Pitfall 6: Forgetting to Update Both builtins.rs AND infer.rs
**What goes wrong:** The type checker has TWO locations for HTTP module definitions: `builtins.rs` (register_builtins env) and `infer.rs` (stdlib_modules http_mod). Both must be updated.
**Why it happens:** Phase 51 added entries to both locations. Missing one causes type errors.
**How to avoid:** Add `"use"` entry to `http_mod` in `infer.rs` AND `"http_use"` to builtins env in `builtins.rs`.
**Warning signs:** "Unknown field 'use' on module HTTP" or type mismatch errors.

## Code Examples

### Example 1: Snow Source -- Middleware Usage

```snow
fn logger(request, next) do
  let method = Request.method(request)
  let path = Request.path(request)
  let response = next(request)
  response
end

fn handler(request) do
  HTTP.response(200, "hello")
end

fn main() do
  let r = HTTP.router()
  let r = HTTP.use(r, logger)
  let r = HTTP.route(r, "/hello", handler)
  HTTP.serve(r, 8080)
end
```

### Example 2: Multiple Middleware (Composable Pipeline)

```snow
fn auth_check(request, next) do
  case Request.header(request, "Authorization") do
    Some(token) -> next(request)
    None -> HTTP.response(401, "Unauthorized")
  end
end

fn add_prefix(request, next) do
  let response = next(request)
  response
end

fn handler(request) do
  HTTP.response(200, "protected")
end

fn main() do
  let r = HTTP.router()
  let r = HTTP.use(r, add_prefix)
  let r = HTTP.use(r, auth_check)
  let r = HTTP.route(r, "/secret", handler)
  HTTP.serve(r, 8080)
end
```

Execution order for `GET /secret`:
1. `add_prefix` runs first (registered first = outermost)
2. `add_prefix` calls `next(request)` -> invokes `auth_check`
3. `auth_check` checks auth header, calls `next(request)` -> invokes `handler`
4. `handler` returns response
5. Control flows back through `auth_check` -> `add_prefix`

### Example 3: Runtime -- Middleware Registration

```rust
// Source: extends existing route_with_method pattern in router.rs

pub struct MiddlewareEntry {
    pub fn_ptr: *mut u8,
    pub env_ptr: *mut u8,
}

// In SnowRouter:
pub struct SnowRouter {
    pub routes: Vec<RouteEntry>,
    pub middlewares: Vec<MiddlewareEntry>,
}

#[no_mangle]
pub extern "C" fn snow_http_use_middleware(
    router: *mut u8,
    middleware_fn: *mut u8,
) -> *mut u8 {
    unsafe {
        let old = &*(router as *const SnowRouter);

        // Copy existing routes
        let new_routes: Vec<RouteEntry> = old.routes.iter().map(|e| RouteEntry {
            pattern: e.pattern.clone(),
            method: e.method.clone(),
            handler_fn: e.handler_fn,
            handler_env: e.handler_env,
        }).collect();

        // Copy existing middlewares and append new one
        let mut new_middlewares: Vec<MiddlewareEntry> = old.middlewares.iter().map(|m| MiddlewareEntry {
            fn_ptr: m.fn_ptr,
            env_ptr: m.env_ptr,
        }).collect();
        new_middlewares.push(MiddlewareEntry {
            fn_ptr: middleware_fn,
            env_ptr: std::ptr::null_mut(),
        });

        let new_router = Box::new(SnowRouter {
            routes: new_routes,
            middlewares: new_middlewares,
        });
        Box::into_raw(new_router) as *mut u8
    }
}
```

### Example 4: Runtime -- Middleware Chain Execution in handle_request

```rust
// Source: extends existing handle_request in server.rs

/// State for the middleware chain trampoline.
struct ChainState {
    middlewares: Vec<MiddlewareEntry>,
    index: usize,
    handler_fn: *mut u8,
    handler_env: *mut u8,
}

/// Trampoline: called as `next(request)` from Snow middleware.
/// Signature: extern "C" fn(env_ptr: *mut u8, request_ptr: *mut u8) -> *mut u8
extern "C" fn chain_next(env_ptr: *mut u8, request_ptr: *mut u8) -> *mut u8 {
    unsafe {
        let state = &*(env_ptr as *const ChainState);
        if state.index >= state.middlewares.len() {
            // All middleware exhausted; call the route handler
            call_handler(state.handler_fn, state.handler_env, request_ptr)
        } else {
            // Call the current middleware with a new "next" pointing to index+1
            let mw = &state.middlewares[state.index];
            let next_state = Box::new(ChainState {
                middlewares: state.middlewares.clone(),
                index: state.index + 1,
                handler_fn: state.handler_fn,
                handler_env: state.handler_env,
            });
            let next_closure = build_snow_closure(
                chain_next as *mut u8,
                Box::into_raw(next_state) as *mut u8,
            );
            call_middleware(mw.fn_ptr, mw.env_ptr, request_ptr, next_closure)
        }
    }
}

/// Build a Snow-compatible closure struct { fn_ptr, env_ptr }.
fn build_snow_closure(fn_ptr: *mut u8, env_ptr: *mut u8) -> *mut u8 {
    unsafe {
        let closure = crate::gc::snow_gc_alloc_actor(16, 8) as *mut *mut u8;
        *closure = fn_ptr;
        *closure.add(1) = env_ptr;
        closure as *mut u8
    }
}

/// Call a handler: fn(request) -> response (or fn(env, request) -> response for closures).
fn call_handler(fn_ptr: *mut u8, env_ptr: *mut u8, request: *mut u8) -> *mut u8 {
    unsafe {
        if env_ptr.is_null() {
            let f: fn(*mut u8) -> *mut u8 = std::mem::transmute(fn_ptr);
            f(request)
        } else {
            let f: fn(*mut u8, *mut u8) -> *mut u8 = std::mem::transmute(fn_ptr);
            f(env_ptr, request)
        }
    }
}

/// Call a middleware: fn(request, next_closure) -> response
/// (or fn(env, request, next_closure) -> response for closures).
fn call_middleware(
    fn_ptr: *mut u8,
    env_ptr: *mut u8,
    request: *mut u8,
    next_closure: *mut u8,
) -> *mut u8 {
    unsafe {
        if env_ptr.is_null() {
            let f: fn(*mut u8, *mut u8) -> *mut u8 = std::mem::transmute(fn_ptr);
            f(request, next_closure)
        } else {
            let f: fn(*mut u8, *mut u8, *mut u8) -> *mut u8 = std::mem::transmute(fn_ptr);
            f(env_ptr, request, next_closure)
        }
    }
}
```

### Example 5: Updated handle_request with Middleware

```rust
// In handle_request, replace the direct handler call with middleware chain:
fn handle_request(router_ptr: *mut u8, mut request: tiny_http::Request) {
    unsafe {
        let router = &*(router_ptr as *const SnowRouter);
        // ... existing request parsing code ...

        if let Some((handler_fn, handler_env, params)) = router.match_route(path_str, &method_str) {
            // ... existing path_params and request struct construction ...

            // Execute middleware chain (or direct handler if no middleware)
            let response_ptr = if router.middlewares.is_empty() {
                // Fast path: no middleware, call handler directly (existing behavior)
                call_handler(handler_fn, handler_env, req_ptr)
            } else {
                // Build middleware chain and execute
                let state = Box::new(ChainState {
                    middlewares: router.middlewares.clone(),
                    index: 0,
                    handler_fn,
                    handler_env,
                });
                let initial_next = build_snow_closure(
                    chain_next as *mut u8,
                    Box::into_raw(state) as *mut u8,
                );
                // The "initial next" represents the full chain: middleware[0] -> ... -> handler
                // We call it directly as a next function (which starts with middleware[0])
                chain_next(Box::into_raw(Box::new(ChainState {
                    middlewares: router.middlewares.clone(),
                    index: 0,
                    handler_fn,
                    handler_env,
                })) as *mut u8, req_ptr)
            };

            // ... existing response extraction and sending code ...
        } else {
            // 404 case -- should middleware run here too?
            // Recommendation: YES, for consistency with standard frameworks.
            // Wrap the 404 handler in the middleware chain.
            // If no middleware, return 404 directly.
            if router.middlewares.is_empty() {
                // Existing 404 behavior
                let not_found = tiny_http::Response::from_string("Not Found")
                    .with_status_code(tiny_http::StatusCode(404));
                let _ = request.respond(not_found);
            } else {
                // Wrap a 404-returning handler in the middleware chain
                // ... construct chain with a synthetic 404 handler ...
            }
        }
    }
}
```

**NOTE on 404 middleware execution:** The requirements state "it runs on every request" (SC-1). The cleanest implementation: always run middleware, with `next` pointing to either the matched handler or a 404 handler. The planner should decide whether the added complexity is worth it for SC-1 compliance, or if middleware-before-routing is sufficient for initial implementation. A pragmatic first pass: only run middleware when a route matches (simpler, covers 95% of use cases), with a note that middleware-for-404 can be added later.

### Example 6: Compiler Pipeline Wiring

```rust
// intrinsics.rs -- add one new function declaration:
// snow_http_use_middleware(router: ptr, middleware_fn: ptr) -> ptr
module.add_function("snow_http_use_middleware",
    ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false),
    Some(inkwell::module::Linkage::External));

// lower.rs -- known_functions:
self.known_functions.insert("snow_http_use_middleware".to_string(),
    MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)));

// lower.rs -- map_builtin_name:
"http_use" => "snow_http_use_middleware".to_string(),

// builtins.rs -- register type:
env.insert(
    "http_use".into(),
    Scheme::mono(Ty::fun(
        vec![router_t.clone(), Ty::fun(vec![request_t.clone(), Ty::fun(vec![request_t.clone()], response_t.clone())], response_t.clone())],
        router_t.clone(),
    )),
);

// infer.rs -- add to http_mod:
http_mod.insert("use".to_string(), Scheme::mono(Ty::fun(
    vec![router_t.clone(), Ty::fun(vec![request_t.clone(), Ty::fun(vec![request_t.clone()], response_t.clone())], response_t.clone())],
    router_t.clone(),
)));
```

## State of the Art

| Before (current) | After (Phase 52) | Impact |
|-------------------|-------------------|--------|
| Handler called directly after routing | Middleware chain wraps handler | Cross-cutting concerns (logging, auth) are composable |
| No pre/post request processing | Middleware can inspect/modify request before handler and response after | Standard web framework capability |
| Each handler must implement its own auth/logging | Auth/logging middleware applied globally via `HTTP.use` | DRY, composable |

## Open Questions

1. **Should middleware run on 404 (no route match)?**
   - What we know: SC-1 says "runs on every request." Standard frameworks (Express, Django) run middleware before routing, so middleware sees all requests including 404s.
   - What's unclear: The added complexity of wrapping 404 in the middleware chain.
   - Recommendation: For the initial implementation, run middleware only when a route matches (simpler). This covers the vast majority of use cases. If SC-1 strictly requires "every request," add a synthetic 404 handler that the middleware chain wraps. The planner should decide.

2. **Should `HTTP.use` support closures as middleware (not just bare functions)?**
   - What we know: The existing `HTTP.route` pattern passes handlers as single fn_ptr without closure splitting. This means only bare named functions work.
   - What's unclear: Whether users will need closure-based middleware (e.g., middleware that captures configuration).
   - Recommendation: Follow the existing handler pattern (bare functions only). Closure support for all HTTP handler types is a separate enhancement. This is consistent and avoids changing the closure-splitting behavior.

3. **Memory management for ChainState boxes**
   - What we know: Each middleware invocation allocates a `Box<ChainState>` and a GC-allocated closure struct. The chain state is not GC-managed.
   - What's unclear: Whether Box-allocated state will be properly freed.
   - Recommendation: Use `snow_gc_alloc_actor` for chain state allocation too, so everything is GC-managed. Alternatively, accept the small leak per-request (the actor terminates quickly). The planner should decide the approach.

## Sources

### Primary (HIGH confidence)
- `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/http/router.rs` -- Current SnowRouter, RouteEntry, match_route, route_with_method
- `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/http/server.rs` -- SnowHttpRequest, handle_request, handler calling convention
- `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/http/mod.rs` -- Current exports
- `/Users/sn0w/Documents/dev/snow/crates/snow-codegen/src/codegen/intrinsics.rs` -- All runtime function declarations (lines 417-465)
- `/Users/sn0w/Documents/dev/snow/crates/snow-codegen/src/codegen/expr.rs` -- Closure splitting logic (lines 593-642), closure call codegen (lines 1030-1110)
- `/Users/sn0w/Documents/dev/snow/crates/snow-codegen/src/mir/lower.rs` -- STDLIB_MODULES (line 8806), map_builtin_name (line 8816+), known_functions (lines 656-672)
- `/Users/sn0w/Documents/dev/snow/crates/snow-typeck/src/builtins.rs` -- HTTP type registrations (lines 522-642)
- `/Users/sn0w/Documents/dev/snow/crates/snow-typeck/src/infer.rs` -- http_mod module definition (lines 459-494)
- `/Users/sn0w/Documents/dev/snow/.planning/phases/51-http-path-parameters/51-VERIFICATION.md` -- Phase 51 verification (confirms on_get/on_post naming, three-pass matching)
- `/Users/sn0w/Documents/dev/snow/.planning/ROADMAP.md` -- Phase 52 requirements and success criteria
- `/Users/sn0w/Documents/dev/snow/.planning/REQUIREMENTS.md` -- HTTP-04, HTTP-05, HTTP-06 definitions

### Secondary (MEDIUM confidence)
- [Django Middleware documentation](https://docs.djangoproject.com/en/6.0/topics/http/middleware/) -- Onion model reference, middleware runs on every request
- [Actix Web middleware documentation](https://actix.rs/docs/middleware/) -- Transform/Service pattern, from_fn helper
- [Go Middleware Chains (OneUptime, Jan 2026)](https://oneuptime.com/blog/post/2026-01-30-go-middleware-chains-http/view) -- Composable middleware without external frameworks

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- No new dependencies. All infrastructure exists from Phase 51. Verified by reading source code.
- Architecture: HIGH -- Middleware storage on router follows the exact same pattern as route storage. Compiler pipeline wiring is identical to Phase 51 (one new function). The trampoline/chain pattern is well-understood from middleware frameworks.
- Pitfalls: HIGH -- Closure calling convention verified by reading codegen source (expr.rs lines 1030-1110). Closure splitting behavior verified by reading known_functions registration pattern and codegen split logic (expr.rs lines 593-642). The `{ fn_ptr, env_ptr }` struct layout verified from codegen/types.rs.
- Middleware chain execution: MEDIUM -- The runtime trampoline approach is novel for this codebase. The pattern is well-understood from other frameworks, but the specific interaction between Rust Box allocation and Snow's GC needs careful implementation.

**Research date:** 2026-02-11
**Valid until:** 2026-03-11 (stable -- internal compiler/runtime architecture)
