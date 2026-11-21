# Mesh collections

Lists, maps, sets, and queues are immutable. Update functions return the updated collection. Rebind with `let` to use the new value. Map keys and set elements compare using their type's `Eq`.

## Lists and tuples

`[1, 2, 3]` is a `List<Int>`. `List.new()` makes an empty list. `List.append(xs, x)` appends one value, and either `xs ++ ys` or `xs <> ys` concatenates lists of the same element type.

- `List.length` counts elements. `List.get` and `List.nth` use zero-based indices and panic out of bounds. `List.head`, `List.tail`, and `List.last` require a non-empty list. `List.find(xs, predicate)` returns `Some(first_match)` or `None`. `List.contains` tests membership, and `List.any` and `List.all` take predicates.
- `List.map`, `List.filter`, `List.flat_map`, and `List.reduce` are eager. `List.reduce(xs, initial, fn acc, x -> ... end)` folds the list. The prelude also exposes `map`, `filter`, `reduce`, `head`, and `tail`.
- `List.sort(xs, cmp)` takes a comparator returning a negative, zero, or positive `Int`. `List.take` and `List.drop` operate on a prefix. `List.zip(xs, ys)` pairs values up to the shorter length, and `List.enumerate(xs)` produces `(index, value)` pairs from index zero.
- Tuples can mix types. `let (name, age) = pair` destructures one. `Tuple.first`, `Tuple.second`, and `Tuple.nth(pair, index)` return the selected element's type when the tuple type is known. With a known tuple type, a computed index requires all elements to share a type.

## Maps, sets, ranges, and queues

| Type | Construction and access |
| --- | --- |
| `Map<K, V>` | `%{"name" => "Ada"}` or `Map.new()`. Update with `Map.put` and `Map.delete`. Check `Map.has_key(m, k)` before `Map.get(m, k)` when the key may be absent. A missing key panics. `Map.size` counts entries. `Map.to_list` and `Map.from_list` convert `(key, value)` tuples. |
| `Set<T>` | `Set.new()` or `Set.from_list(xs)`. Use `Set.add`, `Set.remove`, `Set.contains`, and `Set.size`. `Set.union`, `Set.intersection`, and `Set.difference` combine sets. |
| `Range` | `start..end` or `Range.new(start, end)`. The upper bound is exclusive. `Range.to_list` materializes the integers. A range is empty when the end is at or below the start. |
| `Queue<T>` | `Queue.new()` and `Queue.push(q, x)` create and append. `Queue.size(q)` counts items. On a non-empty queue, `Queue.peek(q)` reads the front and `Queue.pop(q)` returns `(front, rest)`. Check `Queue.is_empty(q)` before reading a queue that may be empty. |

```mesh
fn main() do
  let ordered = [3, 1, 2] |> List.sort(fn a, b -> a - b end)
  let names = %{"name" => "Ada"}
  if Map.has_key(names, "name") do
    println(Map.get(names, "name"))
  end

  let queue = Queue.new() |> Queue.push(List.head(ordered)) |> Queue.push(9)
  let (front, rest) = Queue.pop(queue)
  println("#{front}, #{Queue.peek(rest)}")  # 1, 9
end
```

## Lazy iterators

`Iter.from(source)` takes a list, map, set, or range. It produces an `Iter<T>` over elements, `(K, V)` tuples for maps, and `Int` values for ranges. `Iter.map`, `Iter.filter`, `Iter.take`, `Iter.skip`, `Iter.enumerate`, and `Iter.zip` produce lazy adapters.

An iterator is single-pass. `Iter.next` advances one element and returns `None` when exhausted. Terminals such as `Iter.count`, `Iter.sum` (integers), `Iter.reduce`, `Iter.any`, `Iter.all`, and `Iter.find` pull elements as needed. Create a fresh iterator to start over. `List.collect`, `Set.collect`, `Map.collect`, and `String.collect` materialize the remaining values. Map collection takes an iterator of pairs, and string collection takes an iterator of strings.

```mesh
fn main() do
  let unique = Iter.from(["b", "a", "b", "skip"])
    |> Iter.filter(fn word -> word != "skip" end)
    |> Set.collect()
  println("#{Set.size(unique)}")  # 2

  let indexed = Iter.from(["a", "b"]) |> Iter.enumerate() |> Map.collect()
  println(Map.get(indexed, 1))  # b
end
```

See [syntax](syntax.md) for filtered `for` and pipes, the [standard-library collection APIs](https://meshlang.dev/docs/stdlib/#eager-collections), and the [iterator guide](https://meshlang.dev/docs/iterators/) for terminals and user-defined iterables.
