# Requirements: Mesh v7.0 Iterator Protocol & Trait Ecosystem

## Associated Types

- [ ] **ASSOC-01**: User can declare associated types in interface definitions (`type Item`)
- [ ] **ASSOC-02**: User can specify associated types in impl blocks (`type Item = T`)
- [ ] **ASSOC-03**: User can reference associated types via `Self.Item` in method signatures
- [ ] **ASSOC-04**: Compiler normalizes associated type projections during type inference (resolves `<T as Trait>::Item` to concrete types)
- [ ] **ASSOC-05**: Compiler reports clear errors for missing or extra associated type bindings in impls

## Iterator Protocol

- [ ] **ITER-01**: User can define an `Iterator` interface with `type Item` and `fn next(self) -> Option<Self.Item>`
- [ ] **ITER-02**: User can define an `Iterable` interface with `type Iter` and `fn iter(self) -> Self.Iter`
- [ ] **ITER-03**: User can iterate over any Iterable type with `for x in expr` syntax
- [ ] **ITER-04**: Built-in types (List, Map, Set, Range) implement Iterable with compiler-provided iterator types
- [ ] **ITER-05**: Existing for-in loops over List/Map/Set/Range continue to work with no regressions
- [ ] **ITER-06**: User can create iterators from collections via `Iter.from()` pipe-friendly entry point

## Lazy Combinators

- [ ] **COMB-01**: User can transform iterator elements with `Iter.map(iter, fn)`
- [ ] **COMB-02**: User can filter iterator elements with `Iter.filter(iter, fn)`
- [ ] **COMB-03**: User can limit iteration with `Iter.take(iter, n)` and `Iter.skip(iter, n)`
- [ ] **COMB-04**: User can enumerate iterator elements with `Iter.enumerate(iter)` producing `(index, element)` tuples
- [ ] **COMB-05**: User can zip two iterators with `Iter.zip(iter1, iter2)` producing tuples
- [ ] **COMB-06**: All combinators are lazy -- no intermediate collections allocated

## Terminal Operations

- [ ] **TERM-01**: User can count elements with `Iter.count(iter)`
- [ ] **TERM-02**: User can sum numeric elements with `Iter.sum(iter)`
- [ ] **TERM-03**: User can test predicates with `Iter.any(iter, fn)` and `Iter.all(iter, fn)`
- [ ] **TERM-04**: User can find first matching element with `Iter.find(iter, fn)`
- [ ] **TERM-05**: User can reduce iterator with `Iter.reduce(iter, fn)`

## From/Into Conversion

- [ ] **CONV-01**: User can define `From<T>` trait impls for type conversions (`fn from(value :: T) -> Self`)
- [ ] **CONV-02**: Compiler automatically generates `Into<B> for A` when `From<A> for B` exists
- [ ] **CONV-03**: Built-in From impls exist for common primitive conversions (Int->Float, Int->String, Float->String, Bool->String)
- [ ] **CONV-04**: The `?` operator auto-converts error types via From when function return type differs from expression error type

## Numeric Traits

- [ ] **NUM-01**: User can implement Add/Sub/Mul/Div for custom types with `type Output` associated type
- [ ] **NUM-02**: Binary operators (+, -, *, /) use the Output associated type for result type inference
- [ ] **NUM-03**: User can implement `Neg` trait for unary minus (`-value`) on custom types

## Collect

- [ ] **COLL-01**: User can materialize an iterator into a List via `Iter.collect()` with type annotation or `List.collect(iter)`
- [ ] **COLL-02**: User can materialize an iterator into a Map via `Map.collect(iter)` from iterator of tuples
- [ ] **COLL-03**: User can materialize an iterator into a Set via `Set.collect(iter)`
- [ ] **COLL-04**: User can materialize a string iterator into a String via `String.collect(iter)`

## Future Requirements (Deferred)

- Terminal operations beyond basic (flat_map, chain, skip_while, take_while)
- Bounded associated types (`type Iter: Iterator`)
- Zero/One numeric identity traits
- Infinite iterators (repeat, count, cycle)
- TryFrom/TryInto fallible conversions
- Iterator fusion optimization
- Additional Collect targets

## Out of Scope

- Higher-kinded types (HKTs) -- fundamental type system redesign not warranted
- Generic associated types (GATs) -- complexity too high for this milestone
- Trait inheritance / supertraits -- where clauses provide equivalent power
- User-defined blanket impls -- coherence problems; compiler special-cases Into generation
- Dynamic dispatch / trait objects -- Mesh uses monomorphization only
- Async iterators / streams -- Mesh uses actors for concurrency
- Mixed-type arithmetic (Int + Float -> Float) -- use explicit From conversion
- Mutable iterators -- Mesh has no mutable references

## Traceability

(Populated by roadmapper)
