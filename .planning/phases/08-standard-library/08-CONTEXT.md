# Phase 8: Standard Library - Context

**Gathered:** 2026-02-06
**Status:** Ready for planning

<domain>
## Phase Boundary

Core standard library providing I/O, string operations, collections, file access, HTTP, and JSON -- enough to build real web backends and CLI tools. All sequential stdlib functionality. Actor-based stdlib abstractions (GenServer, Task) are Phase 9.

</domain>

<decisions>
## Implementation Decisions

### Module & import design
- Rich prelude: Option, Result, print, println, and common collection functions (map, filter, reduce, head, tail) auto-imported without explicit import statements
- Flat top-level module namespace: `from IO import read_file`, `from List import map`, `from HTTP import serve` -- no `Std.` prefix
- Both import styles supported: `from List import map` for individual functions, `import List` for namespace access (List.map)

### Claude's Discretion (imports)
- Exact import syntax mechanics based on Snow's existing parser module/import support from Phase 2

### String & collection APIs
- Collection-first argument convention: `map(list, fn)` -- enables natural pipe chains: `list |> map(fn) |> filter(fn)`
- Full collection suite: List, Map, Set, Tuple utilities, Range, Queue
- Immutable only: all operations return new collections, no mutable variants -- fits actor model (no shared mutable state)

### Claude's Discretion (strings)
- String UTF-8 semantics (safe codepoint operations vs raw byte access trade-off)

### I/O & file model
- Result everywhere: all I/O operations return Result types, no panicking convenience variants
- System access included: Env.get("VAR"), Env.args() for CLI argument access

### Claude's Discretion (I/O)
- File API style (path-based convenience vs handle-based, or combination)
- Console I/O structure (print/println placement, stdout/stderr separation, IO module design)

### HTTP & JSON scope
- Batteries-included HTTP server: built-in routing, middleware chain, static file serving, request parsing
- Actor-per-connection model: each HTTP connection spawns an actor, leveraging Snow's actor runtime (Erlang-style)
- HTTP client included: HTTP.get(url), HTTP.post(url, body) for calling external APIs
- JSON: both dynamic JSON type (for parsing unknown data with pattern matching) and trait-based ToJSON/FromJSON for typed encoding/decoding of known structs

</decisions>

<specifics>
## Specific Ideas

- Elixir-style pipe-friendly APIs: collection-first arg enables `data |> map(transform) |> filter(valid) |> reduce(0, sum)`
- Actor-per-connection HTTP server should feel like an Erlang/Elixir web server -- each request is isolated, crashes don't take down the server
- JSON should work naturally with Snow's pattern matching: parse unknown JSON, match on structure

</specifics>

<deferred>
## Deferred Ideas

None -- discussion stayed within phase scope

</deferred>

---

*Phase: 08-standard-library*
*Context gathered: 2026-02-06*
