# Feature Landscape: Loops & Iteration

**Domain:** Loop constructs (`for..in`, `while`, `break`, `continue`) for a statically-typed, expression-oriented language with existing functional iteration (map/filter/reduce), closures, pattern matching, and do/end block syntax
**Researched:** 2026-02-08
**Confidence:** HIGH (loops are the most-studied area of language design, with extensive prior art across dozens of languages)

---

## Current State in Snow

Before defining loop features, here is what already exists and directly affects loop design:

**Working (infrastructure loops interact with):**
- `for` and `in` are reserved keywords (TokenKind::For, TokenKind::In, SyntaxKind::FOR_KW, SyntaxKind::IN_KW) -- already lexed and mapped, but not yet parsed
- `while`, `break`, `continue` are NOT reserved keywords -- must be added to lexer/token/syntax_kind
- do/end block syntax for all compound expressions (if, case, fn closures, receive)
- Pattern matching with destructuring: IdentPat, TuplePat, ConstructorPat, ConsPat, WildcardPat, OrPat, AsPat
- Closures: `fn x -> body end` and `fn(x) do body end` with environment capture
- Higher-order functions: `map(list, fn)`, `filter(list, fn)`, `reduce(list, init, fn)` as prelude builtins
- Method dot-syntax: `list.map(fn x -> x * 2 end)` (being added in prior milestone)
- Pipe operator: `list |> map(fn x -> x * 2 end) |> filter(fn x -> x > 3 end)`
- Collections: List<T> (GC-managed, immutable semantics), Map<K,V>, Set<T>, Range (half-open [start, end))
- Range type: `Range.new(1, 10)` and range operator `1..10`, supports `to_list`, `map`, `filter`, `length`
- All expressions return values (if/else, case/match, closures, blocks -- last expression is the value)
- MIR: desugared representation with `MirExpr::If`, `MirExpr::Block`, `MirExpr::Match`, etc. -- all expression-based
- LLVM codegen: alloca+mem2reg pattern for control flow merges (if/else phi nodes)
- Immutable bindings: `let x = ...` with rebinding via `let x = ...` shadowing (no mutation)
- HM type inference with Algorithm J

**Not yet working (what this milestone adds):**
- `for item in collection do ... end` syntax
- `while condition do ... end` syntax
- `break` and `continue` statements/expressions
- Destructuring in for loops: `for {k, v} in map do ... end`
- Loop expressions returning values (what type does a for-loop evaluate to?)
- New keywords: `while`, `break`, `continue` must be added to token vocabulary

**Key design constraint:** Snow is expression-oriented and immutable-first. Loops must produce values and work without mutation. This fundamentally shapes what loops look like -- they are closer to Elixir comprehensions or Scala for/yield than to C-style imperative loops.

---

## Table Stakes

Features users expect from loops and iteration. Missing any of these and the feature feels incomplete.

| Feature | Why Expected | Complexity | Dependencies | Notes |
|---------|--------------|------------|--------------|-------|
| `for x in collection do body end` | The fundamental iteration form. Every language with loops has this. Users from Ruby, Python, Kotlin, Swift, Rust all expect it. | Medium | New FOR_EXPR AST/CST node, MIR loop lowering, type checker iterable inference, codegen loop emission | Core feature. Must work with List<T>, Range, Map<K,V>, Set<T>. |
| `for i in range do body end` | Range iteration is the most common counted loop pattern. `for i in 1..10 do println("${i}") end` is universal. | Low | For-in implementation + existing Range type and `..` operator | Range already exists. `1..10` produces a Range. Lower to a counter-based while loop in MIR for efficiency (avoid materializing a list). |
| `while condition do body end` | Needed for event loops, polling, retry logic, and general conditional repetition. Expected by users from every imperative or multi-paradigm language. | Medium | New WHILE_EXPR AST/CST node, MIR while lowering, codegen while loop emission | Simpler than for-in because there is no iterator protocol. Condition is re-evaluated each iteration. |
| `break` to exit loops early | Every language with loops supports early exit. Without break, users must contort control flow with boolean flags. | Medium | New BREAK_EXPR AST node, loop context tracking in parser/typechecker, codegen branch to loop exit block | Must only be valid inside loop bodies. Nested loops: break exits innermost loop. |
| `continue` to skip iteration | Expected companion to break. Skips the rest of the current iteration body and moves to next iteration. | Medium | New CONTINUE_EXPR AST node, codegen branch to loop header/increment block | Slightly simpler than break because it does not affect the loop's return value. |
| `for {k, v} in map do ... end` | Destructuring iteration over Map entries. Expected by users from Kotlin (`for ((k,v) in map)`), Elixir (`for {k, v} <- map`), Python, Swift. | Medium | Pattern matching in for-loop binding position, Map iteration protocol producing tuples | Snow already has TuplePat. Map entries naturally destructure as `{key, value}` tuples. The for-loop binding should accept any irrefutable pattern, not just simple identifiers. |
| Loop body executes zero times if collection is empty / condition is false | Standard semantics in every language. | Low | Header check before first body entry | Non-negotiable. An empty list produces zero iterations. A false while-condition produces zero iterations. |
| Correct scoping of loop variable | Loop variable visible only inside body, freshly bound each iteration. | Low | Let binding in body scope | Consistent with Snow's `let` immutability. Each iteration gets a fresh binding. |
| Nested loops with correct break/continue targeting | `for x in xs do for y in ys do break end end` -- break exits inner loop only. | Low | Loop context stack for break/continue tracking | Falls out naturally from recursive expression parsing. Break/continue apply to innermost loop. |

---

## Differentiators

Features that would make Snow's loops stand out. Not strictly expected, but high value.

| Feature | Value Proposition | Complexity | Dependencies | Notes |
|---------|-------------------|------------|--------------|-------|
| **for-in as expression returning List<T>** | `let doubled = for x in list do x * 2 end` collects body results into a new list, like Elixir comprehensions or Scala for/yield. This is THE killer feature for an expression-oriented functional-first language. | High | Type inference for collected list type, codegen list accumulation, interaction with break | **Strongly recommended.** Elixir's `for n <- list, do: n * 2` returns `[2, 4, 6, ...]`. Snow should do the same. See detailed design below. |
| **for-in with filter clause (when)** | `for x in list when x > 0 do x * 2 end` -- filter elements inline, like Elixir's comprehension guards. Avoids a separate filter step. | Medium | Parser extension for optional `when` guard after the `in` clause | Highly ergonomic. Reuses Snow's existing `when` keyword from case-match guards and multi-clause function guards. |
| **break with value (while loops)** | `let found = while has_more do if match do break item end end` -- break returns a value from the while loop. Like Rust's `loop { break value; }` and Zig's break-from-for. | High | Break value type unification, while-else for default value | Zig solves this elegantly with for/while-else. **Recommend for while-loops only.** For for-in (which collects), break stops collection and returns the partial list. |
| **while-else clause** | `while cond do body else default end` -- else runs when condition becomes false without break. Provides the "not found" default value, making while an expression. | Medium | Parser extension, codegen else-branch | Zig has this. Elegantly solves the search pattern. **Recommend Zig semantics:** else runs when loop completes WITHOUT break. |
| **Range iteration without allocation** | `for i in 0..n do body end` compiles to pure integer arithmetic with zero heap allocation in the loop itself. | Low | Detect Range literal in type checker, lower to integer counter loop | Critical optimization. Most for-in loops iterate ranges. This should be as fast as a C for-loop. |
| **for-in with index via enumerate** | `for {i, x} in list.enumerate() do ... end` -- access both index and element. | Low | `.enumerate()` method returning List<{Int, T}>, tuple destructuring handles the rest | Kotlin has `forEachIndexed`, Python has `enumerate()`, Rust has `.enumerate()`. Method-based approach (no special syntax). |

---

## Anti-Features

Features to explicitly NOT build. Common requests that would create problems.

| Anti-Feature | Why Requested | Why Problematic | Alternative |
|--------------|---------------|-----------------|-------------|
| **C-style for(init; cond; step)** | Familiarity from C/Java/JS | Snow is functional-first with immutable bindings. C-style for requires mutable loop variables (`i++`). It doesn't fit Snow's `let` semantics. Every reference language Snow draws from (Elixir, Ruby, Rust, Kotlin, Swift) has moved away from this form. | `for i in 0..10 do ... end` covers counting. `while` covers the general conditional case. |
| **Mutable loop accumulators** | "I need `let mut sum = 0; for x in list do sum = sum + x end`" | Snow has immutable bindings. Adding `mut` for loops would require mutable bindings throughout the language -- a massive scope expansion undermining the functional-first design. | Use `reduce`: `reduce(list, 0, fn acc, x -> acc + x end)`. Or use for-in as expression and then reduce the resulting list. |
| **do-while / repeat-until** | "I need the body to execute at least once" | Low usage frequency. Adds a new keyword and syntax form for minimal benefit. Every language that has it (C, Java) treats it as a rarely-used construct. | `while true do body; if not cond do break end end`. |
| **Infinite `loop` keyword** | "Rust and other languages have a dedicated `loop` keyword" | Snow already has recursion with tail-call optimization for infinite loops (actor receive loops). Adding `loop` creates confusion about when to use `loop` vs `while true`. | `while true do ... end` is clear and universal. Actor receive loops use recursion. |
| **Loop variable mutation** | "`for x in list do x = transform(x); use(x) end`" | Mutation inside loop bodies would require mutable bindings. Snow's `let` rebinding (shadowing) already covers this: `let x = transform(x)` inside the body creates a new binding. | Use `let` shadowing: `for x in list do let x = transform(x); use(x) end`. The inner `x` shadows the loop variable. |
| **Generator/yield (lazy iteration)** | "I want to define custom iterators with yield" | Generators require stackful coroutines or state machine transformation, adding massive runtime complexity. The existing `map`/`filter`/`reduce` HOFs and for-in-as-expression cover most use cases. | Provide an `Iterable` interface with a `next()` method for custom iterators (future milestone). For now, convert to List first. |
| **Parallel for-each** | "I want `for x in list do ... end` to run iterations in parallel" | Requires a concurrency runtime, work stealing, and careful semantics around shared state. Snow has actors for concurrency. | Use actors: spawn a worker per item, collect results. Or use `Job.map(list, fn x -> ... end)` which already exists. |
| **Iterator protocol / Iterable trait** | Full-blown lazy iterator protocol with `next()` | Full milestone on its own: requires Option/Iterator state machines, trait dispatch in loops, lazy evaluation semantics. | Use compiler-known collection desugaring for List/Map/Set/Range. Add Iterable trait in a future milestone. |
| **Labeled breaks** | `break :outer` to exit specific nested loops | Requires label scoping infrastructure, block naming, and complicates the parser. Rare use case. | Refactor nested loops into helper functions, or use boolean flags. Add later if demand is high. |
| **Comprehension guards with commas** | `for x in list, x > 0 do ...` (Elixir-style comma guards) | Ambiguous with tuple syntax in Snow's grammar. Snow uses commas for argument separation. | Use `when` keyword: `for x in list when x > 0 do ... end`. Consistent with existing guard syntax. |
| **String character iteration** | `for c in "hello" do ... end` | String is opaque GC-managed `{len, data}`; exposing characters requires Unicode handling (grapheme clusters, UTF-8 decoding). | Defer to a string iteration milestone. Use `String.split("", s)` or `String.chars(s)` when available. |
| **for-else (Python-style)** | Python's `for...else` where else runs if no break | Widely considered confusing even by Python developers. The semantics ("else means no break") are counterintuitive. | Avoid. While-else (Zig semantics) is clearer and covers the search pattern. |
| **Mutable accumulator pattern (for..reduce)** | Elixir's `for x <- list, reduce: acc do acc -> acc + x end` | Adds significant parser complexity for a pattern that `reduce()` already handles well. | Use `reduce(list, 0, fn acc, x -> acc + x end)` or pipe: `list |> reduce(0, fn acc, x -> acc + x end)`. |

---

## Feature Dependencies

```
New Keywords (prerequisite)
    |   Add `while`, `break`, `continue` to TokenKind, keyword_from_str,
    |   SyntaxKind (3 new keywords). `for` and `in` already exist.
    |
    v
[1] while Loop (simplest loop form)
    |   Parse: WHILE_KW expr DO_KW block END_KW
    |   Type check: condition must be Bool, body returns Unit
    |   MIR: new WhileLoop variant
    |   Codegen: header/body/exit basic blocks, back-edge
    |   Returns: Unit
    |
    +----> [2] break and continue
    |       |   Parse: BREAK_KW [expr], CONTINUE_KW
    |       |   Valid only inside for/while bodies (loop context stack)
    |       |   Codegen: branch to exit/header blocks
    |       |   Type: Never (diverging, coerces to any type)
    |       |
    |       v
    |   [5] while-else (optional differentiator)
    |       while cond do body else default end
    |       break value returns from while; else provides default
    |       Both branches must have same type
    |
    v
[3] for..in over Range (simplest for-in)
    |   Parse: FOR_KW pattern IN_KW expr DO_KW block END_KW
    |   Type check: Range -> element type Int
    |   MIR: lower to integer counter loop (no allocation)
    |   Returns: List<T> where T is body expression type
    |
    +----> [4] for..in over List/Map/Set
    |       |   List<T> -> iterate via index, element type T
    |       |   Map<K,V> -> iterate entries, element type {K, V}
    |       |   Set<T> -> iterate elements, element type T
    |       |   Requires runtime: snow_list_get, snow_map_entry_at, snow_set_to_list
    |       |
    |       v
    |   [6] Destructuring in for-in
    |       |   for {k, v} in map do ... end
    |       |   for (a, b) in pairs do ... end
    |       |   Reuse existing Pattern matching infrastructure
    |       |
    |       v
    |   [7] for-in with filter (when clause)
    |       for x in list when x > 0 do x * 2 end
    |       Filters before body evaluation, reduces collected list
    |
    v
[8] for-in as expression returning List<T>
    |   Body expression type T -> loop returns List<T>
    |   Codegen: accumulate results into new list via snow_list_append
    |   Side-effect optimization: if body type is Unit, skip accumulation
    |   break in for-in: returns partial collected list
    |
    v
[9] Integration & Edge Cases
        Interaction with closures, pattern matching, pipe, dot-syntax
        Nested loops, break/continue in nested contexts
        Type inference chains (body type depends on element type)
```

**Critical path for MVP:** Keywords -> [1] while -> [2] break/continue -> [3] for-in Range -> [4] for-in List
**Expression semantics path:** [3] -> [8] for-in as expression (what makes Snow special)
**Polish path:** [6] destructuring -> [7] when filter -> [5] while-else

---

## Detailed Design Decisions

### Decision 1: for-in Syntax

**Question:** What does `for..in` look like in Snow's do/end syntax?

**Recommendation:**

```snow
for pattern in iterable do
  body
end
```

**Examples:**
```snow
# Simple iteration
for x in [1, 2, 3] do
  println("${x}")
end

# Range iteration
for i in 1..10 do
  println("${i}")
end

# Destructuring map entries
for {k, v} in my_map do
  println("${k}: ${v}")
end

# Tuple destructuring
for (name, age) in people do
  println("${name} is ${age}")
end
```

**Rationale:**
- Follows Snow's existing do/end block pattern (consistent with `if`, `case`, `fn`, `receive`)
- `for` and `in` keywords already reserved
- Pattern position reuses existing pattern infrastructure (IdentPat, TuplePat, etc.)
- Matches Elixir (`for x <- list, do: ...`), Ruby (`for x in list do ... end`), Kotlin (`for (x in list) { ... }`), Swift (`for x in list { ... }`)

**Confidence:** HIGH

### Decision 2: for-in Expression Semantics (The Critical Decision)

**Question:** What does a for-in loop evaluate to? This is THE fundamental design question for Snow.

**Recommendation: for-in returns List<T> where T is the body expression type.** This makes for-in a comprehension, not just a statement.

```snow
# Returns List<Int>: [2, 4, 6]
let doubled = for x in [1, 2, 3] do
  x * 2
end

# Returns List<String>: ["1", "2", "3"]
let strings = for i in 1..4 do
  "${i}"
end

# Side-effect only (result discarded, body returns Unit -> List<Unit>)
for x in list do
  println("${x}")
end
```

**Why this is the right design for Snow:**

1. **Consistency with expression-oriented design.** Everything in Snow is an expression that returns a value. `if/else` returns a value. `case/match` returns a value. Closures return values. Blocks return the last expression. For-loops should follow the same rule.

2. **Elixir precedent (Snow's primary inspiration).** Elixir's `for n <- list, do: n * n` returns a list. This is the most natural model for a functional-first language.

3. **Replaces verbose map patterns.** Currently Snow users write `map(list, fn x -> x * 2 end)` or `list |> map(fn x -> x * 2 end)`. With for-in as expression: `for x in list do x * 2 end`. Shorter, more readable, identical semantics.

4. **Type inference is straightforward.** If the body type is `T` and the iterable has element type `E`, the loop returns `List<T>`. The type checker infers `T` from the body expression.

5. **Scala for/yield desugars the same way.** `for (x <- list) yield x * 2` desugars to `list.map(x => x * 2)`. Snow's for-in-as-expression is syntactic sugar for map.

**Alternative considered and rejected -- returning Unit like Rust/Kotlin.** Rust's `for` returns `()`. But Rust has a clear separation: `for` is for side effects, `.map().collect()` is for transformation. Snow's functional-first design means the for-loop IS the comprehension. Returning Unit would make loops second-class citizens compared to map/filter/reduce, undermining the ergonomic purpose of adding them.

**Side-effect optimization.** When the body type is `Unit`, the for-loop returns `List<Unit>`. This is technically correct but wastes allocation. **The compiler should detect `body_type == Unit` and skip list accumulation**, emitting a simple loop with no collection. The expression type remains `List<Unit>` for type checking purposes, but no list is actually allocated. This is the same optimization Scala applies for `for` without `yield`.

**Confidence:** HIGH -- this directly follows from Snow being expression-oriented and Elixir-inspired.

### Decision 3: while Loop Expression Semantics

**Question:** What does a while loop evaluate to?

**Recommendation: while returns Unit by default. With break-value + else-clause, it returns the break/else value.**

```snow
# Returns Unit -- side-effect loop
while condition do
  do_something()
end

# With while-else: returns T (both branches must agree)
let result = while has_next() do
  let item = next()
  if item == target do
    break item         # Found it
  end
else
  default_value        # Not found
end
# result :: T
```

**The break-value problem.** If `break value` can return a value from a while loop, what happens when the loop exits normally (condition becomes false)? The loop needs a default value for the non-break case.

**Solution: while-else (Zig-style semantics).**
- `while cond do body end` -- returns `Unit`. Cannot use `break value`.
- `while cond do body else default end` -- returns `T` where both `break value` and `else default` must be type `T`. The else runs when condition becomes false without break.
- `break` (no value) is always allowed in any loop, exits with Unit.
- `break value` is only allowed in while-else loops.

**Why Zig-style over Python-style.** Python's `for/while-else` is widely considered confusing ("else means if-no-break" is counterintuitive). Zig's design is clearer: the else provides the default value when the loop does not break, making the loop a proper expression. This maps cleanly to the search pattern (break = found, else = not found).

**Why not Option<T> wrapping (Rust RFC approach).** Rust considered making `for`/`while` return `Option<T>` (Some from break, None from normal exit). This was rejected because it changes the type of all existing loop expressions and forces unwrapping at every use site.

**Confidence:** HIGH for basic while returning Unit. MEDIUM for while-else (adds parser complexity; consider as Phase 2).

### Decision 4: break and continue Semantics

**Question:** How do break and continue work, especially in expression context?

**Recommendation:**

```snow
# break -- exits innermost loop
for x in list do
  if x < 0 do break end
  println("${x}")
end

# continue -- skips to next iteration
for x in list do
  if x < 0 do continue end
  println("${x}")
end

# break in for-in-as-expression: returns partial collected list
let positives = for x in list do
  if x < 0 do break end  # Stop collecting, return what we have so far
  x
end
# positives contains elements up to (not including) the first negative
```

**Key rules:**
1. `break` and `continue` are only valid inside `for` or `while` loop bodies. Using them outside a loop is a compile error.
2. `break` exits the innermost enclosing loop.
3. `continue` skips to the next iteration of the innermost enclosing loop.
4. In for-in-as-expression, `break` stops collection and returns the partially-accumulated `List<T>`.
5. `break value` is only valid in `while ... else` loops (not in for-in).
6. `break` and `continue` have type `Never` (diverging) because they transfer control flow. This is the same semantics as Rust: they can appear in any expression position because `Never` coerces to any type.

**Expression type of break/continue:**
```snow
# break has type Never, which coerces to Int in the if-branch:
let x = if condition do break else 42 end
```

**Confidence:** HIGH for basic break/continue. MEDIUM for break-with-value (while-else integration).

### Decision 5: for-in with Filter (when clause)

**Question:** Should for-in support inline filtering?

**Recommendation: Yes, using the existing `when` keyword.**

```snow
# Filter with when (reuses existing guard keyword)
let evens = for x in 1..20 when x % 2 == 0 do
  x
end
# evens :: List<Int> = [2, 4, 6, 8, 10, 12, 14, 16, 18, 20]

# Multiple conditions with `and`
for x in list when x > 0 and x < 100 do
  transform(x)
end
```

**Why `when` rather than comma-separated guards:**
- Snow already uses `when` for guard clauses in case/match arms and multi-clause functions
- Comma-separated guards (Elixir-style `for x <- list, x > 0`) would be ambiguous since Snow uses commas for argument separation
- `when` is visually clear: `for x in list when condition do body end`

**Parse structure:** `FOR_KW pattern IN_KW expr [WHEN_KW guard_expr] DO_KW block END_KW`

**Confidence:** HIGH -- natural extension of existing guard syntax.

### Decision 6: Iterable Types (What Can Follow `in`)

**Question:** What types can appear after `in` in a for-in loop?

**Recommendation: Start with hardcoded support for List<T>, Range, Map<K,V>, Set<T>. Add an Iterable interface in a future milestone.**

**Phase 1 (MVP):** The type checker recognizes these types in the `in` position:
- `List<T>` -- element type `T`, iterate via index (`snow_list_get`)
- `Range` -- element type `Int`, iterate via counter (most efficient, zero allocation)
- `Map<K,V>` -- element type `{K, V}` (tuple of key and value), iterate via index over entries
- `Set<T>` -- element type `T`, iterate via conversion to list (`snow_set_to_list`)

**Phase 2 (future milestone):** Define an `Iterable` interface:
```snow
interface Iterable<T> do
  fn to_list(self) -> List<T>
end
```
For-in desugars to calling `to_list()` then iterating. Or a more efficient `Iterator` protocol with `next() -> Option<T>` if Snow adds Option.

**Why hardcode first:** Snow doesn't yet have the full Option<T> machinery needed for a proper iterator protocol (Option exists as a sum type but iterator state machines need more infrastructure). Hardcoding the four collection types is simple, covers 99% of use cases, and is forward-compatible with an Iterable interface later.

**Confidence:** HIGH for Phase 1. MEDIUM for Phase 2.

### Decision 7: Immutability and Loop Variables

**Question:** Can loop variables be reassigned inside the body?

**Recommendation: No. Loop variables are immutable bindings, consistent with `let`.**

```snow
for x in list do
  # x is immutable here -- cannot reassign x
  let x = transform(x)  # Shadowing creates a NEW binding, fine
  use(x)
end
```

**For while loops -- the counter problem:** Without mutable variables, while loops cannot have incrementing counters:
```snow
let i = 0
while i < 10 do
  println("${i}")
  # let i = i + 1  -- this shadows i, but the OUTER i is still 0 next iteration!
end
```

**This is intentional.** For counted iteration, use `for i in 0..10 do ... end`. While loops are for condition-based loops (event loops, polling, `while not done`) where the condition depends on external state (I/O, actor messages). In Snow's functional model, counted iteration is for-in's job, not while's.

**Confidence:** HIGH -- follows directly from Snow's immutability model.

---

## Codegen Strategy

### for-in Loop Lowering to MIR/LLVM

The for-in loop desugars to a counted loop:

**Range (zero-allocation fast path):**
```
for i in start..end do body end
  -->
  let __i = start
  while __i < end do
    let i = __i
    [body -- with accumulation if expression-position]
    __i = __i + 1
  end
```

**List<T> (indexed iteration):**
```
for x in list do body end
  -->
  let __list = list
  let __len = snow_list_length(__list)
  let __result = snow_list_new()    // Only for expression-position
  let __i = 0
  while __i < __len do
    let x = snow_list_get(__list, __i)
    [if filter: if !guard_expr then continue]
    let __val = body
    __result = snow_list_append(__result, __val)  // expression-position only
    __i = __i + 1
  end
  __result  // List<T> (or Unit for side-effect)
```

**Map<K,V> (entry iteration):**
```
for {k, v} in map do body end
  -->
  let __keys = snow_map_keys(map)
  let __len = snow_list_length(__keys)
  let __i = 0
  while __i < __len do
    let k = snow_list_get(__keys, __i)
    let v = snow_map_get(map, k)
    body
    __i = __i + 1
  end
```

### LLVM Basic Block Pattern

```
loop_header:
  %cond = <evaluate condition or index < length>
  br i1 %cond, label %loop_body, label %loop_exit

loop_body:
  <get element, bind pattern>
  <guard check -> br to loop_continue if false>
  <body instructions>
  <append to result list if expression-position>
  br label %loop_continue

loop_continue:
  <increment index>
  br label %loop_header

loop_exit:
  <phi node for result value if expression>
```

**break:** emits `br label %loop_exit`
**continue:** emits `br label %loop_continue`

---

## MVP Recommendation

### Build (loops milestone)

**Phase 1: Keywords + while Loop (Foundation)**
1. Add `while`, `break`, `continue` keywords to TokenKind, keyword_from_str, SyntaxKind
2. Parse `while expr do block end` as WHILE_EXPR
3. Type check: condition must be Bool, body evaluates to Unit
4. MIR: new MirExpr::While variant
5. Codegen: header/body/exit basic blocks with back-edge
6. Parse `break` and `continue` as expressions (diverging Never type)
7. Track loop context in parser/typechecker (error if used outside loop)
8. Codegen break/continue: branch to exit/continue labels

**Phase 2: for-in over Range**
9. Parse `for pattern in expr do block end` as FOR_EXPR
10. Type check: detect Range type, element type is Int, bind pattern
11. MIR: lower to integer counter loop (zero allocation)
12. Return type: List<Int> (expression semantics)
13. Side-effect optimization: skip accumulation when body type is Unit

**Phase 3: for-in over List/Map/Set + Destructuring**
14. List<T> iteration via indexed loop
15. Map<K,V> iteration with tuple element type {K, V}
16. Set<T> iteration (via snow_set_to_list conversion)
17. Pattern matching in for-loop binding position (TuplePat, IdentPat, WildcardPat)

**Phase 4: Filter Clause + Integration**
18. Optional `when` guard in for-in: `for x in list when x > 0 do ... end`
19. break in for-in-as-expression: returns partial collected list
20. Integration tests: closures in loop bodies, nested loops, pipe interaction, dot-syntax interaction

### Defer to Post-MVP

- while-else with break-value (elegant but adds parser/type complexity)
- Labeled loops (break/continue specific outer loops)
- Multiple generators (cartesian product: `for x in xs, y in ys do ...`)
- `:into` collector (collect into Map/Set instead of List)
- Lazy/streaming Iterable interface with `next()`
- Comprehension reduce pattern (Elixir-style)
- String character iteration (requires Unicode infrastructure)

---

## Complexity Assessment

| Feature | Estimated Effort | Risk | Notes |
|---------|-----------------|------|-------|
| New keywords (while, break, continue) | 0.5 days | LOW | Mechanical additions to token.rs, syntax_kind.rs, keyword_from_str |
| while parser + type checker | 1 day | LOW | Follows existing if-expr pattern closely |
| while MIR + codegen | 1-2 days | MEDIUM | First loop in codegen; header/body/exit blocks, back-edge |
| break/continue parser | 0.5 days | LOW | New expression nodes with loop context validation |
| break/continue codegen | 1-2 days | MEDIUM | Branch targets, loop context stack for nested loops |
| FOR_EXPR parser | 1 day | LOW | FOR_KW pattern IN_KW expr DO_KW block END_KW |
| for-in type checker | 2-3 days | MEDIUM | Iterable type inference, element type extraction, pattern binding |
| for-in MIR lowering (Range) | 1 day | LOW | Integer counter desugaring |
| for-in MIR lowering (List/Map/Set) | 2 days | MEDIUM | Collection-specific accessor calls, tuple construction for Map |
| for-in as expression (List<T> return) | 2 days | MEDIUM | List accumulation in codegen, side-effect optimization |
| Destructuring in for-in | 1 day | LOW | Reuse existing pattern infrastructure |
| when filter clause | 1 day | LOW | Conditional skip in loop body |
| Testing (unit + e2e) | 2-3 days | LOW | Many combinations: Range/List/Map/Set x break/continue x expression/statement |

**Total estimated effort:** 15-22 days

**Key risks:**
1. **for-in as expression + break interaction.** When break exits a for-in-as-expression, the partial list must be correctly returned. This requires careful codegen with phi nodes at the loop exit.
2. **Immutability + while loops.** Users may expect mutable counters. Good error messages are critical: "cannot mutate loop variable; use `for i in 0..n` for counted iteration."
3. **Type inference chains.** The body type `T` determines the return type `List<T>`. If the body type depends on the loop variable type (which depends on the iterable element type), there is an inference chain that must resolve correctly through HM unification.
4. **First loop in LLVM codegen.** Snow has never emitted loop basic blocks before. The alloca+mem2reg pattern for phi nodes at loop exits needs careful implementation. However, the existing if/else codegen provides a good template.

---

## Sources

### Rust Loop Semantics
- [Loop expressions - The Rust Reference](https://doc.rust-lang.org/reference/expressions/loop-expr.html) -- for/while return `()`, only `loop` returns via `break value`; break/continue semantics; loop labels
- [RFC 1624: loop-break-value](https://rust-lang.github.io/rfcs/1624-loop-break-value.html) -- design discussion on why for/while don't return values; backward compatibility; Option<T> alternative rejected
- [Returning from loops - Rust By Example](https://doc.rust-lang.org/rust-by-example/flow_control/loop/return.html) -- break with value in `loop`

### Elixir Comprehensions
- [Comprehensions - Elixir v1.19.5](https://hexdocs.pm/elixir/comprehensions.html) -- for comprehension returns list by default, `:into` collects to other types, pattern-matching generators
- [Mitchell Hanberg's Comprehensive Guide to Elixir's List Comprehension](https://www.mitchellhanberg.com/the-comprehensive-guide-to-elixirs-for-comprehension/) -- `:reduce` option, filter semantics, practical examples

### Kotlin Loops and Destructuring
- [Conditions and loops - Kotlin Documentation](https://kotlinlang.org/docs/control-flow.html) -- for/while are statements (not expressions), `for ((k,v) in map)` destructuring
- [Returns and jumps - Kotlin Documentation](https://kotlinlang.org/docs/returns.html) -- break, continue, labeled returns with `@`

### Scala for-comprehensions
- [For Comprehensions - Tour of Scala](https://docs.scala-lang.org/tour/for-comprehensions.html) -- for/yield desugars to map/flatMap, returns collection type
- [Comprehensive Guide to For-Comprehension in Scala - Baeldung](https://www.baeldung.com/scala/for-comprehension) -- without yield returns Unit, custom types with map/flatMap

### Zig Loops as Expressions
- [Loops as Expressions - zig.guide](https://zig.guide/language-basics/loops-as-expressions/) -- break with value, else clause on for/while, blocks as expressions
- [Zig Multi-Sequence For Loops](https://kristoff.it/blog/zig-multi-sequence-for-loops/) -- for-else pattern for search

### Python while-else
- [Python While Else - GeeksforGeeks](https://www.geeksforgeeks.org/python/python-while-else/) -- else runs when no break; widely considered confusing

### Ruby for vs each
- [Understanding Ruby - For vs Each](https://dev.to/baweaver/understanding-ruby-for-vs-each-47ae) -- scoping differences, both return the collection, `each` preferred idiomatically

### Swift for-in
- [Swift break Statement - Programiz](https://www.programiz.com/swift-programming/break-statement) -- break exits innermost loop, labeled break for nested, where clause for filtering

### Cross-language Loop Comparison
- [Comparison of programming languages (list comprehension) - Wikipedia](https://en.wikipedia.org/wiki/Comparison_of_programming_languages_(list_comprehension)) -- comprehension syntax across 20+ languages

---
*Feature research for: Snow Language Loops & Iteration*
*Researched: 2026-02-08*
