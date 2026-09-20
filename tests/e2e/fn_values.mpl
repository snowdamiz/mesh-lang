# Functions as values: a named function or a closure, passed, returned, stored
# in every kind of container, bound by a pattern, and called from there.
#
# A function value is always `{fn, env}`; `env` is null for a plain named
# function. Stored or pattern-bound functions used to be typed as a bare
# pointer, which loaded one word of the two and called it without its
# environment, and a named function could not be passed to a `Fun` parameter
# at all.

struct Op do
  name :: String
  run :: Fun(Int) -> Int
end

type Step do
  Apply(Fun(Int) -> Int)
  Add(Int)
end

fn double(n :: Int) -> Int do
  n * 2
end

fn triple(n :: Int) -> Int do
  n * 3
end

fn apply(f :: Fun(Int) -> Int, n :: Int) -> Int do
  f(n)
end

fn compose(f :: Fun(Int) -> Int, g :: Fun(Int) -> Int) -> Fun(Int) -> Int do
  fn (n :: Int) -> g(f(n)) end
end

fn pick(which :: Int) -> Fun(Int) -> Int do
  case which do
    0 -> double
    1 -> triple
    _ -> fn (n :: Int) -> n + which end
  end
end

fn choose(flag :: Bool) -> Fun(Int) -> Int do
  if flag do
    double
  else
    fn (n :: Int) -> n + 1 end
  end
end

fn run_op(op :: Op, n :: Int) -> Int do
  let f = op.run
  f(n)
end

fn run_step(step :: Step, n :: Int) -> Int do
  case step do
    Apply(f) -> f(n)
    Add(k) -> n + k
  end
end

fn run_maybe(maybe :: Option<Fun(Int) -> Int>, n :: Int) -> Int do
  case maybe do
    Some(f) -> f(n)
    None -> n
  end
end

fn make(flag :: Bool, k :: Int) -> Result<Fun(Int) -> Int, String> do
  if flag do
    Ok(fn (n :: Int) -> n * k end)
  else
    Err("none made")
  end
end

fn show_made(made :: Result<Fun(Int) -> Int, String>) -> String do
  case made do
    Ok(f) -> "${f(21)}"
    Err(reason) -> reason
  end
end

actor worker() do
  receive do
    f -> println("actor: ${apply(f, 21)}")
  end
end

fn main() do
  let k = 2
  let twice = fn (n :: Int) -> n * k end

  # Passed to a function, directly and through a local.
  let named = double
  println("param: ${apply(double, 21)} ${apply(named, 4)} ${apply(twice, 5)} ${named(3)}")

  # Returned from a function.
  let a = pick(0)
  let b = pick(1)
  let c = pick(7)
  let d = choose(true)
  let e = choose(false)
  println("returned: ${a(21)} ${b(14)} ${c(35)} ${d(21)} ${e(41)}")

  # Captured by another closure.
  let h = compose(double, fn (n :: Int) -> n + 2 end)
  let h2 = compose(h, triple)
  println("captured: ${h(20)} ${h2(5)}")

  # Inside Option and Result.
  println("option: ${run_maybe(Some(double), 21)} ${run_maybe(Some(twice), 8)} ${run_maybe(None, 42)}")
  println("result: ${show_made(make(true, 2))} ${show_made(make(false, 2))}")

  # Inside a user sum type and a struct.
  println("variant: ${run_step(Apply(double), 21)} ${run_step(Apply(twice), 9)} ${run_step(Add(2), 40)}")
  println("struct: ${run_op(Op { name: "named", run: double }, 21)} ${run_op(Op { name: "closure", run: twice }, 7)}")

  # Inside a tuple, a list and a map.
  let (from_tuple, extra) = (twice, 5)
  let (named_from_tuple, more) = (triple, 1)
  println("tuple: ${from_tuple(21) + extra} ${named_from_tuple(14) + more}")
  let fs = [double, twice, triple]
  let first = List.get(fs, 0)
  let second = List.get(fs, 1)
  let third = List.get(fs, 2)
  println("list: ${first(21)} ${second(6)} ${third(14)}")
  let table = Map.put(Map.put(Map.new(), "named", double), "closure", twice)
  let from_map = Map.get(table, "named")
  let closure_from_map = Map.get(table, "closure")
  println("map: ${from_map(21)} ${closure_from_map(11)}")

  # Piped into, and handed to the runtime's higher-order functions.
  let piped = 41 |> e
  let piped_named = 21 |> double
  println("pipe: ${piped} ${piped_named}")
  let xs = [1, 2, 3]
  let mapped = List.map(xs, double)
  let mapped_local = List.map(xs, named)
  let mapped_closure = List.map(xs, twice)
  let kept = xs |> List.map(triple) |> List.filter(fn (n :: Int) -> n > 3 end)
  println("runtime: ${List.get(mapped, 2)} ${List.get(mapped_local, 1)} ${List.get(mapped_closure, 0)} ${List.length(kept)}")

  # Sent to another actor.
  let pid = spawn(worker)
  send(pid, double)
  Timer.sleep(300)
end
