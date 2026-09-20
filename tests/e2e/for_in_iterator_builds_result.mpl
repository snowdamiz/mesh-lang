# A `for` over an iterator collects its results without knowing how many there
# will be. The result builder used to start with no capacity and never grow:
# every element was written past its end, over whatever the loop body had just
# allocated. Here that is the strings the loop produces.

struct Numbers do
  items :: List<Int>
end

impl Iterable for Numbers do
  type Item = Int
  type Iter = ListIterator
  fn iter(self) -> ListIterator do
    Iter.from(self.items)
  end
end

fn main() do
  let numbers = Numbers { items: Range.to_list(Range.new(1, 201)) }
  let labels = for n in numbers do
    "v-${n * 10}"
  end
  println("${List.length(labels)} ${List.get(labels, 0)} ${List.get(labels, 1)} ${List.get(labels, 199)}")

  let evens = for n in numbers when n % 50 == 0 do
    "even-${n}"
  end
  println("${List.length(evens)} ${String.join(evens, ",")}")
end
