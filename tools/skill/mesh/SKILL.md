---
name: mesh
description: Write, debug, test, and verify Mesh programs and packages. Use for Mesh .mpl code, .law policies and proofs, indexed types, actors, standard-library APIs, and meshc or meshpkg diagnostics.
---

# Mesh

Use the compiler that builds the project. `meshc guide` lists the topics
embedded in that binary. `meshc guide <topic>` works offline. For details
beyond the skill, use the [Mesh language reference](https://meshlang.dev/docs/reference/)
and confirm examples on the project's compiler.

Mesh compiles to native code. Use `fn` for functions, `::` for parameter
annotations, `->` for return types, and `do … end` for blocks. Strings
interpolate with `"hello #{name}"`. A regular pipe (`|>`) supplies the first
argument. A slot pipe such as `|2>` supplies the second.

Build with `meshc build .` and run the executable to check observable
behavior. For a behavior change, run the relevant `*.test.mpl` and confirm
the named test passed. For policy changes, inventory the project's `.law`
files with `meshc verify . --list` before editing. Preserve existing
requirements and recheck the proofs against the implementation.

## Topics

- [Projects and tooling](references/projects.md) covers setup, imports, packages, native interop, and diagnostics.
- [Diagnostics](references/diagnostics.md) lists diagnostic codes and repairs. Run `meshc explain <CODE>` for one entry.
- [Syntax](references/syntax.md) covers functions, pipes, operators, and control flow.
- [Types](references/types.md) covers primitives, structs, sum types, and generics.
- [Pattern matching](references/pattern-matching.md) covers case arms and destructuring.
- [Error handling](references/error-handling.md) covers Result, Option, and propagation.
- [Traits](references/traits.md) covers interfaces, implementations, and deriving.
- [Actors](references/actors.md) covers processes, messages, typed PIDs, and services.
- [Supervisors](references/supervisors.md) covers child specs and restart strategies.
- [Collections](references/collections.md) covers List, Map, Set, Range, Queue, and Iter.
- [Strings](references/strings.md) covers interpolation, heredocs, JSON, and text APIs.
- [Standard library](references/stdlib.md) routes to bytes, numerics, I/O, time, security types, and other built-in APIs.
- [HTTP](references/http.md) covers servers, clients, middleware, and WebSockets.
- [Database](references/database.md) covers SQLite, PostgreSQL, and the ORM.
- [Clustering](references/clustering.md) covers declarations, bootstrap, and inspection.
- [Testing](references/testing.md) covers regression checks and the retry policy walkthrough.
- [Verification](references/verification.md) covers `.law` discovery, result laws, Mesh-authored proofs, indexed data, and independent replay.
