---
title: Iterators
description: Lazy list-iterator pipelines in Mesh with Iter.from, combinators, terminals, collection, and the Iterable protocol used by for-in.
---

# Iterators

Mesh provides lazy iterator adapters for composing list transformations as pipelines, plus `Iterable` and `Iterator` interfaces that power `for...in`. Instead of creating an intermediate list at each lazy step, iterator adapters process elements as a terminal or collect operation requests them. Combined with `|>`, pipelines read from left to right.

The two entry points have different scopes:

- `Iter.from(list)` currently accepts `List<T>` and starts a lazy pipeline.
- `for value in source` accepts built-in lists, maps, sets, ranges, and user-defined `Iterable` or `Iterator` values.

Do not use `Iter.from(map)` or `Iter.from(set)`; those are not part of the current typed API.

## Creating Iterators

Use `Iter.from()` to create an iterator from a list:

```mesh
fn main() do
  let list = [1, 2, 3, 4, 5]
  let iter = Iter.from(list)

  # Count elements to consume the iterator
  let n = Iter.from(list) |> Iter.count()
  println(n.to_string())
end
```

The returned list iterator is consumed as a terminal operation or collect requests values. A pipeline is single-pass: after a terminal operation has exhausted an iterator, create another iterator if you need to traverse the list again.

### Eager List Operations vs Lazy Iterators

The prelude functions `map`, `filter`, and `reduce` (and their `List` module equivalents) operate directly on a `List`. `map` and `filter` eagerly return new lists:

```mesh
let doubled = map([1, 2, 3], fn x -> x * 2 end)
let positive = filter(doubled, fn x -> x > 0 end)
let total = reduce(positive, 0, fn acc, x -> acc + x end)
```

The `Iter.map` and `Iter.filter` functions below instead return lazy adapter handles. Use the eager list operations when you immediately need a list; use `Iter` when you want to compose work and materialize once.

### Custom Iterables

You can make your own types iterable by implementing the `Iterable` interface. This lets your type work with `for...in` loops:

```mesh
struct EvenNumbers do
  items :: List<Int>
end

impl Iterable for EvenNumbers do
  type Item = Int
  type Iter = ListIterator
  fn iter(self) -> ListIterator do
    Iter.from(self.items)
  end
end

fn make_evens() -> EvenNumbers do
  EvenNumbers { items: [2, 4, 6, 8, 10] }
end

fn main() do
  let evens = make_evens()

  # for-in over user-defined Iterable
  let doubled = for x in evens do
    x * 2
  end
  println(doubled.to_string())

  # Iteration with side effects
  for x in evens do
    println(x.to_string())
  end
end
```

The `Iterable` interface requires two associated types (`Item` and `Iter`) and an `iter` method that returns an iterator handle.

The compiler-known `Iterator` contract has an associated `Item` type and a
`next(self)` operation. Semantically, `next` either yields the next `Item` or
signals exhaustion. Implement the existing contract with
`impl Iterator for MyIterator`; do not redeclare the interface in application
code.

A value that directly implements `Iterator` can also appear on the right side of `in`.

### `for...in` Sources

`for...in` is a list-producing comprehension over these sources:

| Source | Binding |
|--------|---------|
| `start..end` | `Int`; the end is exclusive |
| `List<T>` | `T` |
| `Map<K, V>` | `{key, value}` destructuring, or one name for the key |
| `Set` | `Int` |
| `Iterable` | its associated `Item` |
| `Iterator` | its associated `Item` |

An optional `when` clause filters before the body. The result is always `List<BodyType>`:

```mesh
let squares = for n in 0..10 when n % 2 == 0 do
  n * n
end
```

## Lazy Combinators

Combinators transform an iterator into a new iterator without consuming it. Because they are lazy, no work happens until a terminal operation or collect drives the pipeline. You can chain as many combinators as you need.

### map

`Iter.map` transforms each element by applying a function:

```mesh
fn main() do
  let list = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]

  # Double each element, then sum
  let sum = Iter.from(list) |> Iter.map(fn x -> x * 3 end) |> Iter.sum()
  println(sum.to_string())
end
```

### filter

`Iter.filter` keeps only elements that satisfy a predicate:

```mesh
fn main() do
  let list = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]

  # Count even numbers
  let even_count = Iter.from(list) |> Iter.filter(fn x -> x % 2 == 0 end) |> Iter.count()
  println(even_count.to_string())
end
```

`map` and `filter` compose naturally. Chain them to build multi-step transformations:

```mesh
fn main() do
  let list = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]

  # Double each element, then keep only those greater than 10
  let big = Iter.from(list) |> Iter.map(fn x -> x * 2 end) |> Iter.filter(fn x -> x > 10 end) |> Iter.count()
  println(big.to_string())
end
```

### take and skip

`Iter.take` limits an iterator to the first N elements. `Iter.skip` discards the first N elements and yields the rest:

```mesh
fn main() do
  let list = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]

  # Sum of first 3 elements: 1 + 2 + 3 = 6
  let first3 = Iter.from(list) |> Iter.take(3) |> Iter.sum()
  println(first3.to_string())

  # Skip first 7, sum remaining: 8 + 9 + 10 = 27
  let last3 = Iter.from(list) |> Iter.skip(7) |> Iter.sum()
  println(last3.to_string())
end
```

`take` is especially useful for short-circuiting -- once it has yielded N elements, the pipeline stops processing. Combined with `skip`, you can create sliding windows over data:

```mesh
fn main() do
  let list = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]

  # Window: skip first 2, then take 5
  let window = Iter.from(list) |> Iter.skip(2) |> Iter.take(5) |> Iter.count()
  println(window.to_string())
end
```

### enumerate

`Iter.enumerate` pairs each element with its zero-based index, producing `(index, value)` tuples:

```mesh
fn main() do
  let list = [10, 20, 30]

  # Enumerate produces 3 pairs: (0, 10), (1, 20), (2, 30)
  let n = Iter.from(list) |> Iter.enumerate() |> Iter.count()
  println(n.to_string())
end
```

Enumerated iterators are commonly used with `Map.collect` to build index-keyed maps from lists (see [Collecting Results](#collecting-results)).

### zip

`Iter.zip` combines two iterators element-by-element into pairs. The resulting iterator stops when the shorter input is exhausted:

```mesh
fn main() do
  let a = [1, 2, 3]
  let b = [4, 5, 6]
  let pairs = Iter.from(a) |> Iter.zip(Iter.from(b)) |> Iter.count()
  println(pairs.to_string())

  # Unequal lengths: shorter determines count
  let short = [1, 2]
  let long = [10, 20, 30, 40]
  let zipped = Iter.from(short) |> Iter.zip(Iter.from(long)) |> Iter.count()
  println(zipped.to_string())
end
```

## Terminal Operations

Terminal operations consume an iterator and produce a single value. Once a terminal runs, the iterator is exhausted.

### count

`Iter.count` returns the number of elements in the iterator:

```mesh
fn main() do
  let list = [1, 2, 3, 4, 5]
  let c = Iter.from(list) |> Iter.count()
  println(c.to_string())
end
```

### sum

`Iter.sum` adds all integer elements together:

```mesh
fn main() do
  let list = [1, 2, 3, 4, 5]
  let s = Iter.from(list) |> Iter.sum()
  println(s.to_string())
end
```

### any and all

`Iter.any` returns `true` if any element satisfies the predicate. `Iter.all` returns `true` only if every element satisfies it:

```mesh
fn main() do
  let list = [1, 2, 3, 4, 5]

  # any: is there an even number?
  let has_even = Iter.from(list) |> Iter.any(fn x -> x % 2 == 0 end)
  println(has_even.to_string())

  # all: are all elements positive?
  let all_pos = Iter.from(list) |> Iter.all(fn x -> x > 0 end)
  println(all_pos.to_string())

  # all: are all elements even? (false)
  let all_even = Iter.from(list) |> Iter.all(fn x -> x % 2 == 0 end)
  println(all_even.to_string())
end
```

Both `any` and `all` short-circuit -- `any` stops as soon as it finds a match, and `all` stops as soon as it finds a non-match.

### find

The typed search operation is currently `List.find`, which returns `Option<T>`:

```mesh
fn main() do
  let list = [1, 2, 3, 4, 5]
  case List.find(list, fn x -> x > 3 end) do
    Some(value) -> println(value.to_string())
    None -> println("not found")
  end
end
```

The runtime contains a short-circuiting `Iter.find` operation, but the current static `Iter` signature exposes its result as an opaque handle rather than `Option<T>`. Until that signature is typed, prefer `List.find` in Mesh source.

### reduce

`Iter.reduce` folds all elements into a single value using an accumulator and a combining function:

```mesh
fn main() do
  let list = [1, 2, 3, 4, 5]

  # Product: 1 * 2 * 3 * 4 * 5 = 120
  let product = Iter.from(list) |> Iter.reduce(1, fn acc, x -> acc * x end)
  println(product.to_string())

  # Sum via reduce: 0 + 1 + 2 + 3 + 4 + 5 = 15
  let sum = Iter.from(list) |> Iter.reduce(0, fn acc, x -> acc + x end)
  println(sum.to_string())
end
```

The first argument to `reduce` is the initial accumulator value. In the current iterator API, the accumulator and element have the same type; the function receives the current accumulator and next element and returns that type.

## Collecting Results

Lazy pipelines produce iterators, not collections. To materialize the result into a concrete data structure, use a collect function at the end of the pipeline.

### List.collect

`List.collect` gathers all elements from an iterator into a list:

```mesh
fn main() do
  let list = [1, 2, 3]

  # Map and collect into a new list
  let doubled = Iter.from(list) |> Iter.map(fn x -> x * 2 end) |> List.collect()
  println("${doubled}")

  # Filter and collect
  let big = Iter.from([1, 2, 3, 4, 5]) |> Iter.filter(fn x -> x > 3 end) |> List.collect()
  println("${big}")
end
```

### Map.collect

`Map.collect` builds a map from an iterator of key-value pairs. Use `Iter.enumerate` to pair elements with indices, or `Iter.zip` to combine separate key and value iterators:

```mesh
fn main() do
  # Enumerate: indices become keys
  let list = [100, 200, 300]
  let m = Iter.from(list) |> Iter.enumerate() |> Map.collect()
  println("${m}")

  # Zip: combine key and value lists
  let keys = [10, 20, 30]
  let vals = [1, 2, 3]
  let m2 = Iter.from(keys) |> Iter.zip(Iter.from(vals)) |> Map.collect()
  println("${m2}")
end
```

### Set.collect

`Set.collect` gathers integer elements into a set, automatically removing
duplicates:

```mesh
fn main() do
  let list = [1, 2, 2, 3, 3, 3]
  let s = Iter.from(list) |> Set.collect()
  println("${Set.size(s)}")

  # Pipeline into set
  let s2 = Iter.from([1, 2, 3, 4, 5]) |> Iter.filter(fn x -> x > 2 end) |> Set.collect()
  println("${Set.size(s2)}")
end
```

### String.collect

`String.collect` concatenates all string elements from an iterator into a single string:

```mesh
fn main() do
  let words = ["hello", " ", "world"]
  let joined = Iter.from(words) |> String.collect()
  println(joined)

  let abc = Iter.from(["a", "b", "c"]) |> String.collect()
  println(abc)
end
```

## API Summary

| Operation | Result | Notes |
|-----------|--------|-------|
| `Iter.from(list)` | List iterator | `list` must be `List<T>` |
| `Iter.map(iter, fn)` | Lazy iterator | Transforms each value |
| `Iter.filter(iter, fn)` | Lazy iterator | Predicate must return `Bool` |
| `Iter.take(iter, n)` | Lazy iterator | Stops after at most `n` values |
| `Iter.skip(iter, n)` | Lazy iterator | Discards the first `n` values |
| `Iter.enumerate(iter)` | Lazy iterator of `(index, value)` | Index starts at zero |
| `Iter.zip(left, right)` | Lazy iterator of pairs | Stops with the shorter input |
| `Iter.count(iter)` | `Int` | Consumes the iterator |
| `Iter.sum(iter)` | `Int` | Integer elements only |
| `Iter.any(iter, fn)` | `Bool` | Short-circuits on `true` |
| `Iter.all(iter, fn)` | `Bool` | Short-circuits on `false` |
| `Iter.reduce(iter, initial, fn)` | accumulator type | Element and accumulator types currently match |
| `List.collect(iter)` | `List<T>` | Materializes all remaining values |
| `Map.collect(iter)` | `Map<K, V>` | Input values are key-value pairs |
| `Set.collect(iter)` | `Set` | Integer elements; removes duplicates |
| `String.collect(iter)` | `String` | Input values are strings |

## Building Pipelines

The real power of iterators comes from composing multiple combinators into a single pipeline. Each step is lazy -- elements flow through the pipeline one at a time, and short-circuiting combinators like `take` stop processing early.

```mesh
fn main() do
  let list = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]

  # Multi-step pipeline: double, keep values > 10, take first 3, count
  let result = Iter.from(list) |> Iter.map(fn x -> x * 2 end) |> Iter.filter(fn x -> x > 10 end) |> Iter.take(3) |> Iter.count()
  println(result.to_string())

  # Filter, transform, and sum
  let result2 = Iter.from(list) |> Iter.filter(fn x -> x > 5 end) |> Iter.map(fn x -> x * 10 end) |> Iter.sum()
  println(result2.to_string())

  # Closures capture variables from the surrounding scope
  let threshold = 3
  let above = Iter.from(list) |> Iter.filter(fn x -> x > threshold end) |> Iter.count()
  println(above.to_string())
end
```

In the first pipeline, `take(3)` ensures only three elements pass through even though the source list has ten. The `map` and `filter` steps before it only run as many times as needed -- no wasted computation.

Pipelines that end with a collect operation produce a concrete collection:

```mesh
fn main() do
  let list = [1, 2, 3]

  # Transform and materialize as a list
  let doubled = Iter.from(list) |> Iter.map(fn x -> x * 2 end) |> List.collect()
  println("${doubled}")
end
```

## Next Steps

- [Type System](/docs/type-system/) -- interfaces, associated types, and traits that power the iterator protocol
- [Syntax Cheatsheet](/docs/cheatsheet/) -- quick reference for all Mesh syntax
