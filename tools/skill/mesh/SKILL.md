---
name: mesh
description: Write, debug, and test Mesh programs (.mpl). Use for Mesh syntax, types, actors, clustering, HTTP, database access, and compiler or toolchain diagnostics.
---

# Mesh

Check the syntax and APIs against the compiler that builds the project. Run
`meshc guide` for the index embedded in that binary, or `meshc guide syntax`
for a topic. Read the relevant references before editing unfamiliar code.

Mesh compiles to native code. Use `fn` for functions, `::` for parameter
annotations, `->` for return types, and `do … end` for blocks. Strings
interpolate with `"hello #{name}"`. A regular pipe (`|>`) supplies the first
argument. A slot pipe such as `|2>` supplies the second.

Build with `meshc build .` and run the executable when the task specifies
observable behavior. For a requested behavior change, leave a runnable
regression test in the project under `tests/` unless the task limits test
changes. Use `meshc test` and confirm the named test ran and passed.

## Topics

- [Syntax](references/syntax.md) covers functions, pipes, operators, and control flow.
- [Types](references/types.md) covers primitives, structs, sum types, and generics.
- [Pattern matching](references/pattern-matching.md) covers case arms and destructuring.
- [Error handling](references/error-handling.md) covers Result, Option, and propagation.
- [Traits](references/traits.md) covers interfaces, implementations, and deriving.
- [Actors](references/actors.md) covers processes, messages, typed PIDs, and services.
- [Supervisors](references/supervisors.md) covers child specs and restart strategies.
- [Collections](references/collections.md) covers List, Map, Set, Range, Queue, and Iter.
- [Strings](references/strings.md) covers interpolation, heredocs, JSON, and text APIs.
- [HTTP](references/http.md) covers servers, clients, middleware, and WebSockets.
- [Database](references/database.md) covers SQLite, PostgreSQL, and the ORM.
- [Clustering](references/clustering.md) covers declarations, bootstrap, and inspection.
- [Testing](references/testing.md) covers regression checks and the retry policy walkthrough.
