//! Code generation edge cases: each test compiles a Mesh program with the
//! real compiler, runs it, and checks what it prints.

use std::path::PathBuf;
use std::process::{Command, Output};

#[path = "support/test_artifacts.rs"]
mod artifacts;

/// `source` as the `main.mpl` of a project directory, kept as long as the
/// returned guard lives.
fn project(source: &str) -> (tempfile::TempDir, PathBuf) {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_dir = temp_dir.path().join("project");
    std::fs::create_dir_all(&project_dir).expect("failed to create project dir");
    std::fs::write(project_dir.join("main.mpl"), source).expect("failed to write main.mpl");
    (temp_dir, project_dir)
}

/// `meshc build <project> <args>`.
fn meshc_build(project_dir: &std::path::Path, args: &[&str]) -> Command {
    let mut command = Command::new(artifacts::meshc_bin());
    command
        .args(["build", project_dir.to_str().unwrap()])
        .args(args);
    command
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

/// Compile `source` as a one-file project and run it, returning stdout.
fn compile_and_run(source: &str) -> String {
    compile_and_run_with(source, &[])
}

/// `compile_and_run` with extra `meshc build` arguments.
fn compile_and_run_with(source: &str, args: &[&str]) -> String {
    let (_guard, project_dir) = project(source);
    let output = meshc_build(&project_dir, args)
        .output()
        .expect("failed to invoke meshc");
    assert!(
        output.status.success(),
        "meshc build failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        stderr(&output)
    );

    let run = Command::new(project_dir.join("project"))
        .output()
        .expect("failed to run binary");
    assert!(
        run.status.success(),
        "binary exited with {:?}:\nstdout: {}\nstderr: {}",
        run.status.code(),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    String::from_utf8_lossy(&run.stdout).to_string()
}

/// A pid held in an `Option` or `Result` is boxed like any other word, and
/// reading it back reads the box: `Some(pid)` built in Mesh, a job's result,
/// a user iterator's element, `List.find` and `Iter.next`. Pattern matching
/// took the box's address for the pid, so a send went nowhere; `Iter.next`
/// over `DateTime`s handed back the raw word and the match read through it.
#[test]
fn pids_and_handles_in_options_read_back_as_themselves() {
    let out = compile_and_run(
        r##"actor sink() do
  receive do
    _ -> sink()
  end
end

service Feed do
  fn init(items :: List<Pid<Int>>) -> List<Pid<Int>> do
    items
  end

  call Pop() :: Pid<Int>? do |items|
    case items do
      [] -> (items, None)
      _ -> (List.tail(items), Some(List.head(items)))
    end
  end
end

struct Stream do
  pid :: Pid
end

impl Iterator for Stream do
  type Item = Pid<Int>
  fn next(self) -> Pid<Int>? do
    Feed.pop(self.pid)
  end
end

fn main() do
  let a :: Pid<Int> = spawn(sink)
  let b :: Pid<Int> = spawn(sink)
  case Some(a) do
    Some(p) -> println("some: ${p == a} ${send(p, 1)}")
    None -> println("none")
  end
  let job = Job.async(fn () -> b end)
  case Job.await(job) do
    Ok(p) -> println("job: ${p == b}")
    Err(e) -> println(e)
  end
  let stream = Stream { pid: Feed.start([a, b]) }
  println("iterator: ${for p in stream do p == a end}")
  case List.find([a, b], fn (p) -> p == b end) do
    Some(p) -> println("find: ${p == b}")
    None -> println("none")
  end
  case Iter.next(Iter.from([b, a])) do
    Some(p) -> println("next: ${p == b}")
    None -> println("none")
  end
  let now = DateTime.utc_now()
  case Iter.next(Iter.from([now])) do
    Some(d) -> println("date: ${DateTime.to_unix_ms(d) == DateTime.to_unix_ms(now)}")
    None -> println("none")
  end
end
"##,
    );
    assert_eq!(
        out,
        "some: true 0\njob: true\niterator: [true, false]\nfind: true\nnext: true\ndate: true\n"
    );
}

/// Patterns the decision-tree compiler takes apart in less common shapes: a
/// match on a tuple literal with a catch-all arm (matched as columns) or a
/// variable arm (the tuple built after all), a variable beside constructor
/// arms, a float literal and an `as` pattern inside a generic payload, a
/// tuple column whose first row binds it whole, `()`, a struct without
/// fields, and variants whose payload is `()`.
#[test]
fn patterns_in_uncommon_shapes_match() {
    let out = compile_and_run(
        r##"struct Marker do
end

type Signal do
  Ping(())
  Quit
end

fn columns(a :: Int, b :: Int) -> String do
  case (a, b) do
    (0, y) -> "zero ${y}"
    _ -> "other"
  end
end

fn whole(a :: Int, b :: Int) -> String do
  case (a, b) do
    (1, y) -> "one ${y}"
    pair -> "pair ${Tuple.first(pair)}"
  end
end

fn describe(o :: Option<Int>) -> String do
  case o do
    Some(0) -> "zero"
    other -> "other ${other}"
  end
end

fn half(o :: Option<Float>) -> String do
  case o do
    Some(2.5) -> "two and a half"
    Some(x) -> "${x}"
    None -> "none"
  end
end

fn named(o :: Option<(Int, String)>) -> String do
  case o do
    Some((n, s) as pair) -> "${n} ${s} ${Tuple.second(pair)}"
    None -> "none"
  end
end

fn nested(pair :: (Int, Int), n :: Int) -> String do
  case (pair, n) do
    (p, 0) -> "whole ${Tuple.first(p)}"
    ((a, b), _) -> "parts ${a + b}"
  end
end

fn unit_case(u :: ()) -> String do
  case u do
    () -> "unit"
  end
end

fn marker_case(m :: Marker) -> String do
  case m do
    Marker {} -> "marker"
  end
end

fn signal(s :: Signal) -> String do
  case s do
    Ping(()) -> "ping"
    Quit -> "quit"
  end
end

fn main() do
  println("${columns(0, 7)} ${columns(1, 7)}")
  println("${whole(1, 2)} ${whole(3, 4)}")
  println("${describe(Some(0))} ${describe(Some(5))} ${describe(None)}")
  println("${half(Some(2.5))} ${half(Some(1.5))} ${half(None)}")
  println("${named(Some((3, "x")))} ${named(None)}")
  println("${nested((1, 2), 0)} ${nested((1, 2), 1)}")
  println("${unit_case(())} ${marker_case(Marker {})} ${signal(Ping(()))} ${signal(Quit)}")
end
"##,
    );
    assert_eq!(
        out,
        "zero 7 other\none 2 pair 3\nzero other Some(5) other None\n\
         two and a half 1.5 none\n3 x x none\nwhole 1 parts 3\nunit marker ping quit\n"
    );
}

const ECHO_EXPORT: &str = r##"@export("mesh_edges_echo")
pub fn echo(request :: Bytes) -> Bytes ! String do
  Ok(request)
end
"##;

/// A static library builds at every optimization level, and a program at
/// level 1 runs.
#[test]
fn optimized_libraries_and_programs_build() {
    let (_guard, project_dir) = project(ECHO_EXPORT);
    let output = meshc_build(
        &project_dir,
        &["--artifact", "staticlib", "--opt-level", "2"],
    )
    .output()
    .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(project_dir.join("libproject.a").is_file());

    let out = compile_and_run_with(
        "fn main() do\n  println(\"${List.length([1, 2, 3])}\")\nend\n",
        &["--opt-level", "1"],
    );
    assert_eq!(out, "3\n");
}

/// A build that cannot produce its artifact says why, and its build trace
/// (replacing whatever the file held) records the failure: a library with
/// nothing exported, an export named like a runtime symbol, an output
/// directory that does not exist.
#[test]
fn builds_that_cannot_produce_their_artifact_say_why() {
    let (_guard, project_dir) = project("fn main() do\n  println(\"x\")\nend\n");
    let output = meshc_build(&project_dir, &["--artifact", "staticlib"])
        .output()
        .unwrap();
    assert!(
        stderr(&output).contains("library artifacts require at least one `@export` function"),
        "{}",
        stderr(&output)
    );

    let trace = project_dir.join("trace.json");
    std::fs::write(&trace, "not json").unwrap();
    let missing = project_dir.join("missing").join("app");
    let output = meshc_build(&project_dir, &["-o", missing.to_str().unwrap()])
        .env("MESH_BUILD_TRACE_PATH", &trace)
        .output()
        .unwrap();
    assert!(
        stderr(&output).contains("Failed to emit object file"),
        "{}",
        stderr(&output)
    );
    let trace: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&trace).unwrap()).unwrap();
    assert_eq!(trace["success"], false);
    assert!(
        trace["error"]
            .as_str()
            .is_some_and(|error| error.contains("Failed to emit object file")),
        "{trace}"
    );

    let (_guard, project_dir) = project(&ECHO_EXPORT.replace("mesh_edges_echo", "mesh_println"));
    let output = meshc_build(&project_dir, &["--artifact", "staticlib"])
        .output()
        .unwrap();
    assert!(
        stderr(&output).contains(
            "Exported symbol 'mesh_println' conflicts with another generated or runtime symbol"
        ),
        "{}",
        stderr(&output)
    );
}

/// Resources held in a tuple or in a generic payload are destroyed when the
/// scope holding them ends; a type that only names a resource as a type
/// argument holds none. More secrets than an actor may hold at once pass
/// through, so a leak runs out.
#[test]
fn resources_in_tuples_and_payloads_are_destroyed() {
    let out = compile_and_run(
        r##"pub resource struct Boxed do
  key :: SecretBytes
end

type Phantom<T> do
  P(Int)
end

struct Tag<T> do
  n :: Int
end

fn pair_and_drop() -> Int ! CryptoError do
  let pair = (Secret.random(1) ?, 7)
  Ok(1)
end

fn option_and_drop() -> Int ! CryptoError do
  let boxed = Some(Boxed { key: Secret.random(1) ? })
  Ok(1)
end

fn phantom() -> Int do
  let p :: Phantom<SecretBytes> = P(3)
  let t :: Tag<SecretBytes> = Tag { n: 4 }
  case p do
    P(n) -> n + t.n
  end
end

fn churn(0) do nil end
fn churn(count :: Int) do
  let a = pair_and_drop()
  let b = option_and_drop()
  churn(count - 1)
end

fn main() do
  churn(2100)
  println("${phantom()}")
  case Secret.random(1) do
    Ok(secret) -> do
      println("clean")
      Secret.destroy(secret)
    end
    Err(_) -> println("leaked")
  end
end
"##,
    );
    assert_eq!(out, "7\nclean\n");
}

/// A builtin that compiles to something other than a plain runtime call
/// (Math, `Int.to_float`, `List.contains` on strings) evaluates each argument
/// once: the arguments were evaluated for an ordinary call first and then
/// again by the builtin's own code, so their side effects ran twice.
#[test]
fn builtin_arguments_are_evaluated_once() {
    let out = compile_and_run(
        r##"fn noisy(n :: Int) -> Int do
  println("eval ${n}")
  n
end

fn noisy_f(x :: Float) -> Float do
  println("evalf ${x}")
  x
end

fn names() -> List<String> do
  println("names")
  ["a", "b"]
end

fn main() do
  println("${Math.max(noisy(1), noisy(2))}")
  println("${Math.min(noisy_f(1.5), noisy_f(0.5))}")
  println("${Math.abs(noisy(-3))} ${Math.abs(noisy_f(-2.5))}")
  println("${Math.sqrt(noisy_f(4.0))} ${Math.pow(noisy_f(2.0), noisy_f(3.0))}")
  println("${Math.floor(noisy_f(2.5))} ${Math.ceil(noisy_f(2.5))} ${Math.round(noisy_f(2.5))}")
  println("${Int.to_float(noisy(5))} ${Float.to_int(noisy_f(6.9))}")
  println("${List.contains(names(), "b")}")
end
"##,
    );
    assert_eq!(
        out,
        "eval 1\neval 2\n2\nevalf 1.5\nevalf 0.5\n0.5\neval -3\nevalf -2.5\n3 2.5\n\
         evalf 4.0\nevalf 2.0\nevalf 3.0\n2.0 8.0\nevalf 2.5\nevalf 2.5\nevalf 2.5\n2 3 3\n\
         eval 5\nevalf 6.9\n5.0 6\nnames\ntrue\n"
    );
}

/// `?` returns the operand's own failure: an `Err` converted by the `From`
/// impl of the function's error type (a sum type here), a tuple through as
/// the success value, and a `None` out of an actor's body, which ends the
/// actor. A `None` there was built as a `Result` variant, which does not
/// exist, and the build failed.
#[test]
fn try_returns_the_failure_of_its_operand() {
    let out = compile_and_run(
        r##"type Failure do
  Missing(String)
  Code(Int)
end

impl From<String> for Failure do
  fn from(message :: String) -> Failure do
    Missing(message)
  end
end

impl From<Int> for Failure do
  fn from(code :: Int) -> Failure do
    Code(code)
  end
end

fn named(key :: String) -> Result<String, String> do
  if key == "a" do
    Ok("alpha")
  else
    Err("no ${key}")
  end
end

fn coded(n :: Int) -> Result<Int, Int> do
  if n > 0 do
    Ok(n)
  else
    Err(n)
  end
end

fn pair(n :: Int) -> Result<(Int, String), Failure> do
  if n > 1 do
    Ok((n, "big"))
  else
    Err(Missing("small"))
  end
end

fn both(key :: String, n :: Int) -> Result<String, Failure> do
  let name = named(key)?
  let code = coded(n)?
  let p = pair(n)?
  Ok("${name} ${code} ${Tuple.first(p)} ${Tuple.second(p)}")
end

fn show(r :: Result<String, Failure>) -> String do
  case r do
    Ok(s) -> s
    Err(Missing(m)) -> "missing ${m}"
    Err(Code(c)) -> "code ${c}"
  end
end

fn find(s :: String) -> Option<Int> do
  if s == "1" do
    Some(1)
  else
    None
  end
end

actor worker(s :: String) do
  let n = find(s)?
  println("found ${n}")
end

fn main() do
  println(show(both("a", 2)))
  println(show(both("b", 2)))
  println(show(both("a", -3)))
  println(show(both("a", 1)))
  spawn(worker, "2")
  spawn(worker, "1")
end
"##,
    );
    assert_eq!(
        out,
        "alpha 2 2 big\nmissing no b\ncode -3\nmissing small\nfound 1\n"
    );
}

/// `Type.from` and `Type.try_from` named as values (not called) go to the
/// impl converting the value's parameter type. The first `From` impl found
/// in a hash map was taken, so a `Fun(String) -> Wrapper` doubled a string's
/// address as if it were an `Int`.
#[test]
fn conversions_named_as_values_use_the_impl_for_their_type() {
    let out = compile_and_run(
        r##"struct Wrapper do
  value :: Int
end

impl From<Int> for Wrapper do
  fn from(n :: Int) -> Wrapper do
    Wrapper { value: n * 2 }
  end
end

impl From<String> for Wrapper do
  fn from(s :: String) -> Wrapper do
    Wrapper { value: String.length(s) }
  end
end

struct Even do
  n :: Int
end

impl TryFrom<Int> for Even do
  fn try_from(n :: Int) -> Result<Even, String> do
    if n % 2 == 0 do
      Ok(Even { n: n })
    else
      Err("odd")
    end
  end
end

fn show(r :: Result<Even, String>) -> String do
  case r do
    Ok(e) -> "even ${e.n}"
    Err(m) -> m
  end
end

fn main() do
  let f :: Fun(Int) -> Wrapper = Wrapper.from
  let g :: Fun(String) -> Wrapper = Wrapper.from
  println("${f(21).value} ${g("abc").value}")
  let ws = List.map(["ab", "cde"], Wrapper.from)
  println("${List.map(ws, fn(w) -> w.value end)}")
  let h = Even.try_from
  println("${show(h(4))} ${show(h(5))}")
end
"##,
    );
    assert_eq!(out, "42 3\n[2, 3]\neven 4 odd\n");
}

/// A resource in scope around loops of every kind (`while`, and `for` over
/// a range, a list, a map, a set and an `Iterable`) is destroyed once
/// however the function ends: a `return` from inside a loop, or the end
/// of the body after the loops' own `break`s and `continue`s. An actor
/// holds at most 4096 secrets, so a leak on any path fails the later
/// `Secret.random`s.
#[test]
fn resources_outlive_the_loops_in_their_scope_and_no_longer() {
    let out = compile_and_run(
        r##"struct Pair do
  items :: List<Int>
end

impl Iterable for Pair do
  type Item = Int
  type Iter = ListIterator
  fn iter(self) -> ListIterator do
    Iter.from(self.items)
  end
end

fn loops(n :: Int) -> Int ! CryptoError do
  let key = Secret.random(1) ?
  while true do
    break
  end
  let m = Map.put(Map.new(), "a", 1)
  let s = Set.add(Set.new(), 5)
  let a = for i in 0..3 when i < n do
    if i == 1 do
      continue
    end
    i
  end
  let b = for x in [1, 2, 3] when x > 0 do
    if x == 2 do
      break
    end
    x
  end
  let c = for {k, v} in m when v > 0 do
    if v == 1 do
      continue
    end
    v
  end
  let d = for e in s when e > 0 do
    if e == 5 do
      break
    end
    e
  end
  let f = for x in Pair { items: [1, 2] } when x > 0 do
    if x == 1 do
      continue
    end
    x
  end
  for i in 0..10 do
    if i == n do
      return Ok(i)
    end
  end
  Ok(List.length(a) + List.length(b) + List.length(c) + List.length(d) + List.length(f))
end

fn churn(0, acc :: Int) -> Int do acc end
fn churn(count :: Int, acc :: Int) do
  let r = case loops(count % 12) do
    Ok(v) -> v
    Err(_) -> -100
  end
  churn(count - 1, acc + r)
end

fn main() do
  println("${churn(4500, 0)}")
  case Secret.random(1) do
    Ok(secret) -> do
      println("clean")
      Secret.destroy(secret)
    end
    Err(_) -> println("leaked")
  end
end
"##,
    );
    // Each run of 12 returns 0 to 9 from inside the last loop, then twice
    // 2 + 1 + 0 + 0 + 1 from the loops' results.
    assert_eq!(out, "19875\nclean\n");
}

/// A derived `from_row` fills `Option` fields with values that read back:
/// `Some` of each parsed type, `None` for a missing or empty column, and an
/// `Option` of an `Option`. The fields held the address of the option where
/// the option itself belongs, so reading one panicked on its tag.
#[test]
fn row_option_fields_hold_their_values() {
    let out = compile_and_run(
        r##"struct Rec do
  i :: Option<Int>
  f :: Option<Float>
  b :: Option<Bool>
  s :: Option<String>
end deriving(Row)

struct Deep do
  x :: Option<Option<Int>>
end deriving(Row)

fn show(row :: Map<String, String>) -> String do
  case Rec.from_row(row) do
    Ok(r) -> "${r.i} ${r.f} ${r.b} ${r.s}"
    Err(e) -> "err ${e}"
  end
end

fn main() do
  let row = Map.put(Map.put(Map.new(), "i", "3"), "f", "2.5")
  println(show(Map.put(Map.put(row, "b", "true"), "s", "hi")))
  println(show(Map.put(Map.new(), "f", "")))
  println(show(Map.put(Map.new(), "f", "x")))
  case Deep.from_row(Map.put(Map.new(), "x", "5")) do
    Ok(d) -> case d.x do
      Some(Some(n)) -> println("deep ${n}")
      Some(None) -> println("deep some none")
      None -> println("deep none")
    end
    Err(e) -> println("err ${e}")
  end
end
"##,
    );
    assert_eq!(
        out,
        "Some(3) Some(2.5) Some(true) Some(hi)\nNone None None None\n\
         err cannot parse 'x' as Float\ndeep 5\n"
    );
}

/// Less common operands: `/`, `%` and unary `-` through a struct's `Div`,
/// `Mod` and `Neg` impls, `==` and interpolation of values annotated as a
/// bare `List`, `Map` or `Set` (which hold `Int`s), and regex literals with
/// the multiline and dot-all flags.
#[test]
fn operator_impls_bare_collections_and_regex_flags_lower() {
    let out = compile_and_run(
        r##"struct V do
  x :: Int
end

impl Div for V do
  type Output = V
  fn div(self, other :: V) -> V do
    V { x: self.x / other.x }
  end
end

impl Mod for V do
  type Output = V
  fn mod(self, other :: V) -> V do
    V { x: self.x % other.x }
  end
end

impl Neg for V do
  type Output = V
  fn neg(self) -> V do
    V { x: 0 - self.x }
  end
end

fn same(a :: List, b :: List) -> Bool do
  a == b
end

fn same_map(a :: Map, b :: Map) -> Bool do
  a == b
end

fn show(a :: List, m :: Map, s :: Set) -> String do
  "${a} ${m} ${s}"
end

fn main() do
  let q = V { x: 17 } / V { x: 5 }
  let r = V { x: 17 } % V { x: 5 }
  let n = -V { x: 3 }
  println("${q.x} ${r.x} ${n.x}")
  println("${same([1, 2], [1, 2])} ${same([1], [2])}")
  println("${same_map(Map.put(Map.new(), 1, 2), Map.put(Map.new(), 1, 2))}")
  println(show([1, 2], Map.put(Map.new(), 1, 2), Set.add(Set.new(), 3)))
  println("${Regex.is_match(~r/^b$/m, "a\nb")} ${Regex.is_match(~r/a.b/s, "a\nb")}")
  println("${Regex.is_match(~r/a.b/, "a\nb")}")
end
"##,
    );
    assert_eq!(
        out,
        "3 2 -3\ntrue false\ntrue\n[1, 2] %{1 => 2} #{3}\ntrue true\nfalse\n"
    );
}

/// A supervisor starts every child. The byte saying a child is local was
/// left out, so the runtime read the next child's first byte in its place:
/// after a child whose id is one letter long (a 1), the next looked remote,
/// and the supervisor did not start.
#[test]
fn supervisors_start_every_child() {
    let out = compile_and_run(
        r##"actor a1() do
  println("a1 started")
end

actor a2() do
  println("a2 started")
end

supervisor Two do
  strategy: one_for_one
  max_restarts: 10
  max_seconds: 5

  child x do
    start: fn -> spawn(a1) end
    restart: temporary
    shutdown: 1000
  end

  child y do
    start: fn -> spawn(a2) end
    restart: temporary
    shutdown: 1000
  end
end

fn main() do
  let _ = spawn(Two)
  Timer.sleep(1000)
end
"##,
    );
    let mut lines: Vec<&str> = out.lines().collect();
    lines.sort();
    assert_eq!(lines, ["a1 started", "a2 started"]);
}

/// A supervisor's children start with the arguments their `spawn` gives
/// them, again at each restart, and each restart type holds: a crashing
/// permanent child comes back, a temporary one and a transient one that
/// ends normally do not. A child spawned with arguments crashed at its
/// first instruction (the supervisor started it without any), and a child
/// after one whose id is one letter long was read as a remote one, so the
/// supervisor never started.
#[test]
fn supervisor_children_start_with_their_arguments() {
    let out = compile_and_run(
        r##"fn crash(0) -> Int do
  0
end

actor worker(label :: String, pause :: Int) do
  println("${label} started")
  Timer.sleep(pause)
  crash(1)
end

actor ticker() do
  println("ticker started")
end

supervisor Crashing do
  strategy: one_for_one
  max_restarts: 10
  max_seconds: 5

  child p do
    start: fn -> spawn(worker, "permanent", 20) end
    restart: permanent
    shutdown: 1_000
  end

  child t do
    start: fn -> spawn(worker, "temporary", 20) end
    restart: temporary
    shutdown: brutal_kill
  end

  child n do
    start: fn -> spawn(ticker) end
    restart: transient
    shutdown: 50
  end
end

supervisor Rest do
  strategy: rest_for_one
  max_restarts: 1
  max_seconds: 1

  child r do
    start: fn -> spawn(worker, "rest", 5000) end
    restart: temporary
    shutdown: brutal_kill
  end
end

fn main() do
  let _ = spawn(Crashing)
  let _ = spawn(Rest)
  Timer.sleep(1000)
end
"##,
    );
    let count = |line: &str| out.lines().filter(|l| *l == line).count();
    assert!(count("permanent started") >= 2, "{out}");
    for once in ["temporary started", "ticker started", "rest started"] {
        assert_eq!(count(once), 1, "{out}");
    }
}

/// A child's `start` has to end by spawning its actor: that actor is what
/// the supervisor restarts. A start through another function built, and the
/// supervisor started the child from a null function.
#[test]
fn supervisor_children_must_start_by_spawning() {
    let (_guard, project_dir) = project(
        r##"actor worker() do
  println("worker")
end

fn start_worker() -> Pid<Int> do
  spawn(worker)
end

supervisor Sup do
  child w do
    start: fn -> start_worker() end
  end
end

fn main() do
  let _ = spawn(Sup)
end
"##,
    );
    let output = meshc_build(&project_dir, &[]).output().unwrap();
    assert!(
        stderr(&output).contains(
            "the child `w` of supervisor `Sup` must start as `fn -> spawn(actor, ...) end`"
        ),
        "{}",
        stderr(&output)
    );
}

/// A closure captures what its loops and patterns use from outside and not
/// what they bind: the variables of `for` loops over a range, a map, a set
/// and an `Iterable`, a list's head and tail, and an `as` binding.
#[test]
fn closures_capture_around_what_their_loops_and_patterns_bind() {
    let out = compile_and_run(
        r##"struct Pair do
  items :: List<Int>
end

impl Iterable for Pair do
  type Item = Int
  type Iter = ListIterator
  fn iter(self) -> ListIterator do
    Iter.from(self.items)
  end
end

fn main() do
  let base = 10
  let limit = 3
  let m = Map.put(Map.new(), "a", 1)
  let s = Set.add(Set.new(), 5)
  let p = Pair { items: [1, 2] }
  let f = fn(k :: Int) do
    let a = for i in 0..limit when i < k do
      i + base
    end
    let b = for {key, v} in m when v > 0 do
      v + base
    end
    let c = for e in s when e > 0 do
      e + base
    end
    let d = for x in p when x > 0 do
      x + base
    end
    List.length(a) + List.length(b) + List.length(c) + List.length(d)
  end
  println("${f(2)}")
  let g = fn(xs :: List<Int>) do
    case xs do
      h :: t -> h + base + List.length(t)
      [] -> base
    end
  end
  println("${g([1, 2, 3])} ${g([])}")
  let h = fn(o :: Option<Int>) do
    case o do
      Some(n) as whole -> "${whole} ${n + base}"
      None -> "none"
    end
  end
  println(h(Some(1)))
end
"##,
    );
    assert_eq!(out, "6\n13 10\nSome(1) 11\n");
}

/// A service's state can be missing (no `init`: the Int 0) or Unit, and its
/// messages can carry `()` arguments and `()` or tuple replies. A service
/// whose `init` returned `()` did not build: its init and cast handler
/// functions were declared to return an Int and returned Unit.
#[test]
fn services_keep_missing_and_unit_states() {
    let out = compile_and_run(
        r##"service NoInit do
  call Get() :: Int do |state|
    (state, 42)
  end

  cast Bump() do |state|
    state + 1
  end
end

service UnitState do
  fn init() do
    ()
  end

  call Ping(n :: Int) :: Int do |state|
    (state, n + 1)
  end

  cast Poke() do |state|
    state
  end
end

service Units do
  fn init() -> Int do
    5
  end

  call Ping(u :: ()) :: Int do |state|
    (state, state + 1)
  end

  call Nothing() :: () do |state|
    (state, ())
  end

  call Pair() :: (Int, String) do |state|
    (state, (state, "pair"))
  end

  cast Poke(u :: ()) do |state|
    state + 1
  end
end

fn main() do
  let a = NoInit.start()
  NoInit.bump(a)
  println("${NoInit.get(a)}")
  let b = UnitState.start()
  UnitState.poke(b)
  println("${UnitState.ping(b, 1)}")
  let c = Units.start()
  Units.poke(c, ())
  println("${Units.ping(c, ())} ${Units.nothing(c)}")
  let p = Units.pair(c)
  println("${Tuple.first(p)} ${Tuple.second(p)}")
end
"##,
    );
    assert_eq!(out, "42\n2\n7 ()\n6 pair\n");
}

/// Bare `to_string(x)` and `inspect(x)` show collections and strings as
/// interpolation does, and `default()` builds each primitive's default and
/// a struct's through its impl.
#[test]
fn bare_to_string_inspect_and_default_calls_lower() {
    let out = compile_and_run(
        r##"struct Config do
  size :: Int
end

impl Default for Config do
  fn default() -> Config do
    Config { size: 3 }
  end
end

fn make() -> Config do
  default()
end

fn main() do
  println(to_string([1, 2]))
  println(inspect(["a"]))
  println(to_string("abc"))
  println(inspect("abc"))
  println(to_string(Map.put(Map.new(), 1, 2)))
  let i :: Int = default()
  let s :: String = default()
  let f :: Float = default()
  let b :: Bool = default()
  println("${make().size} ${i} [${s}] ${f} ${b}")
end
"##,
    );
    assert_eq!(
        out,
        "[1, 2]\n[\"a\"]\nabc\n\"abc\"\n%{1 => 2}\n3 0 [] 0.0 false\n"
    );
}

/// An arm without `->` stands for its pattern's value: a literal of each
/// kind, a variant, and `nil`, which also matches as a pattern of its own.
#[test]
fn pass_through_arms_stand_for_their_patterns() {
    let out = compile_and_run(
        r##"fn ints(n :: Int) -> Int do
  case n do
    0
    other -> other + 1
  end
end

fn floats(x :: Float) -> Float do
  case x do
    1.5
    other -> other * 2.0
  end
end

fn bools(b :: Bool) -> Bool do
  case b do
    true
    false -> true
  end
end

fn strings(s :: String) -> String do
  case s do
    "same"
    other -> "${other}!"
  end
end

fn options(o :: Option<Int>) -> Option<Int> do
  case o do
    None
    Some(n) -> Some(n + 1)
  end
end

fn units(u :: ()) -> () do
  case u do
    nil
  end
end

fn unit_name(u :: ()) -> String do
  case u do
    nil -> "nil"
  end
end

fn main() do
  println("${ints(0)} ${ints(4)} ${floats(1.5)} ${floats(2.0)}")
  println("${bools(true)} ${bools(false)} ${strings("same")} ${strings("x")}")
  println("${options(None)} ${options(Some(1))}")
  println("${units(())} ${unit_name(())}")
end
"##,
    );
    assert_eq!(
        out,
        "0 5 1.5 4.0\ntrue true same x!\nNone Some(2)\n() nil\n"
    );
}

/// A queue and a JSON value spawned into an actor are the actor's own
/// copies: they read back after the spawner's heap has been churned
/// through a collection.
#[test]
fn queues_and_json_cross_to_an_actor() {
    let out = compile_and_run_with(
        r##"actor queued(q :: Queue<String>, doc :: Json) do
  Timer.sleep(100)
  let pair = Queue.pop(q)
  println("front ${Tuple.first(pair)} rest ${Queue.size(Tuple.second(pair))}")
  println(Json.encode(doc))
end

fn churn(i :: Int, n :: Int, acc :: Int) -> Int do
  if i >= n do
    acc
  else
    let s = "garbage-${i}"
    churn(i + 1, n, acc + String.length(s))
  end
end

fn main() do
  let q = Queue.push(Queue.push(Queue.new(), "a-${1 + 1}"), "b")
  case Json.parse("{\"k\": [1, 2]}") do
    Ok(j) -> do
      let _ = spawn(queued, q, j)
      let _ = churn(0, 200000, 0)
      Timer.sleep(300)
    end
    Err(e) -> println(e)
  end
end
"##,
        &["--opt-level", "2"],
    );
    assert_eq!(out, "front a-2 rest 1\n{\"k\":[1,2]}\n");
}

/// A map collected from string-keyed pairs compares its keys as strings
/// however the collect is written: piped, called directly, or after a zip
/// of string keys. The key looked up is built at run time, so a map keyed
/// by addresses would miss it.
#[test]
fn string_keyed_collects_compare_keys_as_strings() {
    let out = compile_and_run(
        r##"fn main() do
  let k = "${"b"}${""}"
  let piped = [("a", 1), ("b", 2)] |> Iter.from() |> Map.collect()
  let direct = Map.collect(Iter.from([("a", 1), ("b", 2)]))
  let keys = ["a", "b"]
  let zipped = keys |> Iter.from() |> Iter.zip(Iter.from([10, 20])) |> Map.collect()
  println("${Map.get(piped, k)} ${Map.get(direct, k)} ${Map.get(zipped, k)}")
end
"##,
    );
    assert_eq!(out, "2 2 20\n");
}

/// A free local TCP port.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// `Node.spawn` starts an actor on another node with an argument of each
/// type a remote spawn carries, and an actor that takes none: one binary
/// runs as the host node, and again as the guest that spawns on it.
#[test]
fn remote_spawns_carry_each_argument_type() {
    use std::io::{BufRead, BufReader};
    let (_guard, project_dir) = project(
        r##"actor typed(i :: Int, f :: Float, b :: Bool, s :: String, u :: (), p :: Pid<Int>) do
  println("typed #{i} #{f} #{b} #{s} #{u} #{p != p}")
end

actor bare() do
  println("bare")
end

actor spawner(host_name :: String) do
  let origin :: Pid<Int> = self()
  let _ = Node.spawn(host_name, typed, 7, 2.5, true, "remote", (), origin)
  let _ = Node.spawn_link(host_name, bare)
  println("spawned")
end

fn main() do
  let cookie = "a-development-cookie-0123456789"
  let host_name = "host@127.0.0.1:#{Env.get_int("HOST_PORT", 0)}"
  let role = Env.get("ROLE", "warm")
  if role == "host" do
    println("host start=#{Node.start(host_name, cookie)}")
    Timer.sleep(4000)
  else if role == "guest" do
    let name = "guest@127.0.0.1:#{Env.get_int("GUEST_PORT", 0)}"
    println("guest start=#{Node.start(name, cookie)}")
    println("connect=#{Node.connect(host_name)}")
    let _ = spawn(spawner, host_name)
    Timer.sleep(1000)
  else
    println("warm")
  end
end
"##,
    );
    let output = meshc_build(&project_dir, &[]).output().unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    let binary = project_dir.join("project");
    // The first run of a new binary waits for the system's scan of it.
    let warm = Command::new(&binary).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&warm.stdout), "warm\n");

    let host_port = free_port().to_string();
    let mut host = Command::new(&binary)
        .env("ROLE", "host")
        .env("HOST_PORT", &host_port)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut host_lines = BufReader::new(host.stdout.take().unwrap()).lines();
    assert_eq!(host_lines.next().unwrap().unwrap(), "host start=0");
    let guest = Command::new(&binary)
        .env("ROLE", "guest")
        .env("HOST_PORT", &host_port)
        .env("GUEST_PORT", free_port().to_string())
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&guest.stdout),
        "guest start=0\nconnect=0\nspawned\n"
    );
    let mut spawned: Vec<String> = host_lines.map(Result::unwrap).collect();
    host.wait().unwrap();
    spawned.sort();
    assert_eq!(spawned, ["bare", "typed 7 2.5 true remote () false"]);
}

/// A remote node starts the function `Node.spawn` names by its name, so a
/// closure cannot be spawned remotely: the build says so. It built a spawn
/// of a function named `unknown`.
#[test]
fn remote_spawns_need_a_named_function() {
    let (_guard, project_dir) = project(
        r##"actor spawner(host_name :: String) do
  let _ = Node.spawn(host_name, fn () -> self() end)
end

fn main() do
  let _ = spawn(spawner, "absent@127.0.0.1:1")
end
"##,
    );
    let output = meshc_build(&project_dir, &[]).output().unwrap();
    assert!(
        stderr(&output).contains(
            "Node.spawn needs a function defined at the top level: the remote node starts it by name"
        ),
        "{}",
        stderr(&output)
    );
}

/// A runtime iterator hands a for-in loop the element's collection slot,
/// which for a struct or sum value is a pointer to the boxed value. The loop
/// stored that pointer as the value itself: a struct read back garbage and a
/// match on an `Option` found no arm.
#[test]
fn iterables_yield_structs_and_sum_values() {
    let output = compile_and_run(
        r#"struct Point do
  x :: Int
  y :: Int
end

struct Points do
  items :: List<Point>
end

impl Iterable for Points do
  type Item = Point
  type Iter = ListIterator
  fn iter(self) -> ListIterator do
    Iter.from(self.items)
  end
end

struct Maybes do
  items :: List<Option<Int>>
end

impl Iterable for Maybes do
  type Item = Option<Int>
  type Iter = ListIterator
  fn iter(self) -> ListIterator do
    Iter.from(self.items)
  end
end

fn main() do
  let products = for p in Points { items: [Point { x: 1, y: 2 }, Point { x: 3, y: 4 }] } do
    p.x * p.y
  end
  let values = for m in Maybes { items: [Some(5), None] } do
    case m do
      Some(v) -> v
      None -> 0
    end
  end
  println("${products} ${values}")
end
"#,
    );
    assert_eq!(output, "[2, 12] [5, 0]\n");
}

/// A for-in loop over an `Iter` value (from `Iter.from`, or an adapter
/// chained on one) advances it with the runtime's generic `next`. The build
/// panicked looking for a function named `Iterator__next__Unknown`.
#[test]
fn loops_run_over_iter_values() {
    let output = compile_and_run(
        r#"fn main() do
  let r = 2..5
  let tens = for i in r do
    i * 10
  end
  let all = for v in Iter.from([1, 2, 3]) do
    v
  end
  let big = for v in Iter.from([1, 2, 3, 4]) |> Iter.filter(fn x -> x > 2 end) do
    v
  end
  let pairs = for v in Iter.from([1, 2, 3]) |> Iter.zip(Iter.from(["x", "y"])) do
    v
  end
  println("${tens} ${List.length(all)} ${List.length(big)} ${List.length(pairs)}")
end
"#,
    );
    assert_eq!(output, "[20, 30, 40] 3 2 2\n");
}

/// Every kind of for-in loop binds its variables over the ones they
/// shadow, gives them back afterwards, and collects nothing from a body
/// that leaves by `break`, `continue` or `return`. An Iterable's Float
/// and Bool elements arrive as themselves.
#[test]
fn loops_shadow_their_variables_and_leave_early() {
    let output = compile_and_run(
        r#"struct Floats do
  items :: List<Float>
end

impl Iterable for Floats do
  type Item = Float
  type Iter = ListIterator
  fn iter(self) -> ListIterator do
    Iter.from(self.items)
  end
end

struct Flags do
  items :: List<Bool>
end

impl Iterable for Flags do
  type Item = Bool
  type Iter = ListIterator
  fn iter(self) -> ListIterator do
    Iter.from(self.items)
  end
end

fn first_big(xs :: List<Int>) -> Int do
  for x in xs do
    if x > 10 do
      return x
    end
  end
  0
end

fn main() do
  let k = "k"
  let v = "v"
  let x = 100
  let doubled = for x in Floats { items: [1.5, 2.5] } do
    x * 2.0
  end
  let flipped = for x in Flags { items: [true, false] } when x do
    not x
  end
  let a = for x in 0..5 do
    break
  end
  let b = for x in [1, 2, 3] do
    continue
  end
  let c = for {k, v} in %{"a" => 1} do
    break
  end
  let d = for x in Set.from_list([1, 2]) do
    continue
  end
  let e = for x in Flags { items: [true] } do
    break
  end
  let g = for {k, v} in %{"a" => 1, "b" => 2} when v > 1 do
    "${k}${v}"
  end
  println("${doubled} ${flipped} ${a} ${b} ${c} ${d} ${e} ${g}")
  println("${k} ${v} ${x} ${first_big([1, 20, 3])} ${first_big([1])}")
end
"#,
    );
    assert_eq!(
        output,
        "[3.0, 5.0] [false] [] [] [] [] [] [b2]\nk v 100 20 0\n"
    );
}

/// A runtime function that hands back a Bool as a pointer-sized word
/// (`Changeset.valid`) is read as that word. Used as a condition it
/// crashed the build, and bound by a `let` it was stored whole into the
/// Bool's one-byte slot, overwriting the locals beside it.
#[test]
fn runtime_booleans_returned_as_words_read_as_booleans() {
    let output = compile_and_run(
        r##"import Changeset

fn main() do
  let cs = Changeset.cast(%{}, %{"name" => ""}, [:name])
    |> Changeset.validate_required([:name])
  if Changeset.valid(cs) do
    println("valid")
  else
    println("invalid")
  end
  let ok = Changeset.valid(cs)
  println("#{ok} #{not Changeset.valid(cs)} #{Changeset.get_error(cs, :name)}")
end
"##,
    );
    assert_eq!(output, "invalid\nfalse true can't be blank\n");
}

/// A panic, a return or a call that never returns, used as an operand,
/// ends what uses it. The build failed LLVM verification: the rest of the
/// expression went after the block's terminator, and a `let` bound to a
/// call that never returns handed its placeholder byte to `ret`.
#[test]
fn operands_that_never_finish_end_their_expression() {
    let output = compile_and_run(
        r##"fn boom(n :: Int) do
  panic("boom #{n}")
end

fn pick(n :: Int) -> Int do
  let x = if n > 0 do n else panic("negative") end
  x + 1
end

fn bound(n :: Int) -> Int do
  if n > 0 do
    let y = boom(n)
    y
  else
    n
  end
end

fn added(n :: Int) -> Int do
  if n > 0 do
    n + panic("added")
  else
    n - 1
  end
end

fn listed(n :: Int) -> List<Int> do
  if n > 0 do
    [n, boom(n)]
  else
    [n]
  end
end

fn early(n :: Int) -> Int do
  let z = return n * 10
  z
end

fn main() do
  println("#{pick(1)} #{bound(0)} #{added(0)} #{listed(0)} #{early(4)}")
end
"##,
    );
    assert_eq!(output, "2 0 -1 [0] 40\n");
}

/// A resource's scope that ends in a receive whose every arm panics has
/// no way out to release the resource at: nothing is appended after it.
#[test]
fn a_resource_scope_can_end_in_a_receive_that_never_finishes() {
    let output = compile_and_run(
        r##"actor runner() do
  let key = Secret.random(1)
  receive do
    m -> panic("got #{m}")
  after 20 -> panic("timed out")
  end
end

fn main() do
  let pid :: Pid<Int> = spawn(runner)
  Timer.sleep(300)
  println("main done")
end
"##,
    );
    assert_eq!(output, "main done\n");
}

/// A `@cluster` function runs at startup through an actor body that calls
/// it and drops what it returns, a Unit included.
#[test]
fn cluster_work_runs_whatever_it_returns() {
    let output = compile_and_run(
        r##"@cluster pub fn tick() do
  println("tick")
end

@cluster(2) pub fn count() -> Int do
  3
end

fn main() do
  tick()
  println("#{count()}")
  Timer.sleep(200)
end
"##,
    );
    let mut lines: Vec<&str> = output.lines().collect();
    lines.sort();
    assert_eq!(lines, ["3", "tick", "tick"], "{output}");
}

/// Values of the less common shapes flow through branches and messages:
/// Float remainders and comparisons, a tuple out of an `if` and a `case`,
/// a function whose branches all panic, a loop body that always leaves,
/// and a unit or Bool message.
#[test]
fn values_of_every_shape_flow_through_branches_and_messages() {
    let output = compile_and_run(
        r##"fn pick(c :: Bool) -> (Int, String) do
  let t :: (Int, String) = if c do (1, "one") else (2, "two") end
  t
end

fn choose(n :: Int) -> (Int, Int) do
  case n do
    0 -> (0, 0)
    _ -> (n, n * n)
  end
end

fn fail(c :: Bool) -> Int do
  if c do
    panic("fail a")
  else
    panic("fail b")
  end
end

fn safe(n :: Int) -> Int do
  if n > 100 do
    fail(n > 200)
  else
    n
  end
end

actor units() do
  receive do
    u -> println("unit #{u}")
  end
end

actor flags() do
  receive do
    b -> println("flag #{b}")
  end
end

fn main() do
  println("#{5.5 % 2.0} #{1.5 <= 1.5} #{2.5 >= 3.0} #{2.5 >= 1.0}")
  println("#{pick(true)} #{pick(false)} #{choose(0)} #{choose(3)} #{safe(7)}")
  let kept = for x in [1, 2, 3] do
    if x > 1 do
      break
    else
      continue
    end
  end
  println("#{kept}")
  let u :: Pid<()> = spawn(units)
  send(u, ())
  Timer.sleep(100)
  let f :: Pid<Bool> = spawn(flags)
  send(f, true)
  Timer.sleep(100)
end
"##,
    );
    assert_eq!(
        output,
        "1.5 true false true\n(1, one) (2, two) (0, 0) (3, 9) 7\n[]\nunit ()\nflag true\n"
    );
}

/// A tuple holding a resource inside a nested tuple destroys it with the
/// rest when its scope ends.
#[test]
fn a_resource_in_a_nested_tuple_is_destroyed() {
    let output = compile_and_run(
        r##"fn nested() -> Int ! CryptoError do
  let a = Secret.random(1) ?
  let b = Secret.random(1) ?
  let t = ((a, 1), b)
  Ok(2)
end

fn main() do
  case nested() do
    Ok(n) -> println("nested #{n}")
    Err(_) -> println("error")
  end
end
"##,
    );
    assert_eq!(output, "nested 2\n");
}

/// Resources held in less common places are released: in a sum type that
/// holds itself, in a variant with named fields, in an impl method's
/// parameter, and in a scope a panic may leave.
#[test]
fn resources_in_recursive_named_and_method_positions_are_released() {
    let output = compile_and_run(
        r##"type Chain do
  Link(SecretBytes, Chain)
  End
end

type Named do
  Holder(key :: SecretBytes, n :: Int)
  Empty
end

struct Vault do
  n :: Int
end

interface Taker do
  fn take(self, key :: SecretBytes) -> Int
end

impl Taker for Vault do
  fn take(self, key :: SecretBytes) -> Int do
    self.n
  end
end

fn guarded(c :: Bool) -> Int ! CryptoError do
  let key = Secret.random(1) ?
  if c do
    panic("guarded")
  end
  Ok(1)
end

fn build() -> Int ! CryptoError do
  let a = Secret.random(1) ?
  let b = Secret.random(1) ?
  let chain = Link(a, Link(b, End))
  let named = Holder(Secret.random(1) ?, 3)
  let v = Vault { n: 7 }
  let taken = Vault.take(v, Secret.random(1) ?)
  Ok(taken)
end

fn main() do
  case build() do
    Ok(n) -> println("built #{n}")
    Err(_) -> println("error")
  end
  case guarded(false) do
    Ok(n) -> println("guarded #{n}")
    Err(_) -> println("error")
  end
end
"##,
    );
    assert_eq!(output, "built 7\nguarded 1\n");
}

/// A value of a type that holds itself, a tree of sum values or a struct
/// with a list of its own kind, crosses to another actor whole: its shape
/// table refers back to the type's own node.
#[test]
fn recursive_types_cross_to_actors() {
    let output = compile_and_run(
        r##"type Tree do
  Leaf
  Node(Tree, Int, Tree)
end

struct Dir do
  name :: String
  children :: List<Dir>
end

fn total(t :: Tree) -> Int do
  case t do
    Leaf -> 0
    Node(l, v, r) -> total(l) + v + total(r)
  end
end

fn count(d :: Dir) -> Int do
  List.reduce(d.children, 1, fn (acc, c) -> acc + count(c) end)
end

actor trees() do
  receive do
    t -> println("tree #{total(t)}")
  end
end

actor dirs() do
  receive do
    d -> println("dir #{d.name} #{count(d)}")
  end
end

fn main() do
  let t :: Pid<Tree> = spawn(trees)
  send(t, Node(Node(Leaf, 1, Leaf), 2, Node(Leaf, 3, Leaf)))
  Timer.sleep(100)
  let d :: Pid<Dir> = spawn(dirs)
  send(d, Dir { name: "root", children: [Dir { name: "a", children: [] }, Dir { name: "b", children: [] }] })
  Timer.sleep(100)
end
"##,
    );
    assert_eq!(output, "tree 6\ndir root 3\n");
}

/// An impl method takes and returns a tuple as the pointer to its heap
/// block, as every other function does: it took the tuple's fields by
/// value, so reading its argument dereferenced an element as a pointer, and
/// its caller read the returned fields as a pointer.
#[test]
fn impl_methods_pass_tuples_as_pointers() {
    let output = compile_and_run(
        r##"struct Bag do
  n :: Int
end

interface Adder do
  fn add_pair(self, p :: (Int, Int)) -> Int
  fn split(self) -> (Int, String)
end

impl Adder for Bag do
  fn add_pair(self, p :: (Int, Int)) -> Int do
    let (a, b) = p
    self.n + a + b
  end

  fn split(self) -> (Int, String) do
    (self.n, "n#{self.n}")
  end
end

fn main() do
  let bag = Bag { n: 1 }
  let (k, label) = bag.split()
  println("#{bag.add_pair((4, 5))} #{k} #{label} #{Adder.split(bag)}")
end
"##,
    );
    assert_eq!(output, "10 1 n1 (1, n1)\n");
}

/// A pid whose messages are resources is not a resource of its own to
/// destroy: the actor it names owns what it receives. Lowering gave such a
/// pid (an actor receiving secrets, a job returning one) the destructor of
/// an opaque resource handle, and code generation crashed on the pid's
/// integer where it expected a pointer.
#[test]
fn pids_of_actors_that_receive_resources_are_plain_values() {
    let output = compile_and_run(
        r##"actor sink() do
  receive do
    s -> Secret.destroy(s)
  end
end

fn keep(p :: Pid<SecretBytes>) -> Pid<SecretBytes> do
  p
end

fn main() do
  let p = keep(spawn(sink))
  let job = Job.async(fn () -> Secret.random(8) end)
  println("spawned")
end
"##,
    );
    assert_eq!(output, "spawned\n");
}

/// A closure used at several types is compiled once per type, and its
/// generic copy is left out when every use has its own. A keyword key of
/// the closure's name (`size(add: 1)`) is no use of it, but it counted as
/// one whose type is unknown: the generic copy was compiled too, and its
/// `a + b` on operands of no known type failed the build.
#[test]
fn keyword_keys_named_like_a_polymorphic_closure_are_not_uses() {
    let output = compile_and_run(
        r##"fn size(m :: Map<String, Int>) -> Int do
  Map.size(m)
end

fn main() do
  let add = fn a, b -> a + b end
  let n = size(add: 1)
  println("#{add(1, 2)} #{add(1.5, 2.5)} #{n}")
end
"##,
    );
    assert_eq!(output, "3 4.0 1\n");
}

/// A value that never comes into being (a `panic`) can stand wherever a
/// value goes. Interpolated, lowering had no way to show it and emitted a
/// call of an undefined `to_string`; compared with `<`, it generated a
/// comparison function for `Never` that LLVM rejected. Both now end where
/// the panic does, as arithmetic on it already did.
#[test]
fn values_that_never_come_into_being_interpolate_and_compare() {
    let output = compile_and_run(
        r##"fn show(n :: Int) -> String do
  if n > 0 do
    "big #{panic("no")}"
  else
    "small #{n}"
  end
end

fn below(n :: Int) -> Bool do
  if n > 0 do
    panic("no") < n
  else
    n < 0
  end
end

fn main() do
  println("#{show(0)} #{below(-1)}")
end
"##,
    );
    assert_eq!(output, "small 0 true\n");
}

/// A type deriving Display shows each payload by the payload's own
/// Display, or else its Debug. A payload type with neither ends the build
/// when the derived function is used; a type whose derived Display is
/// never used still builds.
#[test]
fn derived_display_needs_a_displayable_payload_only_when_used() {
    let (_guard, project_dir) = project(
        r##"struct Inner do
  x :: Int
end deriving(Eq)

type Wrap do
  W(Inner)
end deriving(Display)

fn main() do
  println("#{W(Inner { x: 1 })}")
end
"##,
    );
    let output = meshc_build(&project_dir, &[]).output().unwrap();
    assert!(
        stderr(&output).contains("cannot convert a value of type `Inner` to a string"),
        "{}",
        stderr(&output)
    );
    let output = compile_and_run(
        r##"struct Inner do
  x :: Int
end

type Wrap do
  W(Inner)
end deriving(Display)

type Held do
  H(Inner)
end deriving(Display)

fn main() do
  println("#{W(Inner { x: 1 })}")
  let _ = H(Inner { x: 2 })
end
"##,
    );
    assert_eq!(output, "W(Inner { x: 1 })\n");
}

/// An actor receives a Bool as the byte it is; a receive with no arm only
/// waits out its `after`.
#[test]
fn actors_receive_bools_and_wait_without_arms() {
    let output = compile_and_run(
        r##"actor flag(done :: Pid<String>) do
  receive do
    b -> send(done, if b do "yes" else "no" end)
  end
  flag(done)
end

actor driver() do
  let p = spawn(flag, self())
  send(p, true)
  receive do
    s -> println(s)
  end
  send(p, false)
  receive do
    s -> println(s)
  end
  let quiet = receive do
  after 20 ->
    "nothing"
  end
  println(quiet)
end

fn main() do
  let d :: Pid<String> = spawn(driver)
  Timer.sleep(500)
end
"##,
    );
    assert_eq!(output, "yes\nno\nnothing\n");
}

/// `String.from` passed as a function shows each value as interpolating it
/// would: a String as itself, a Float, a Bool.
#[test]
fn string_from_as_a_function_value_shows_each_type() {
    let output = compile_and_run(
        r##"fn main() do
  println("#{List.map([1.5, 2.0], String.from)} #{List.map([true], String.from)} #{List.map(["x"], String.from)}")
end
"##,
    );
    assert_eq!(output, "[1.5, 2.0] [true] [x]\n");
}

/// An error code generation reports inside a `case` arm, or inside a match
/// of several values at once, ends the build as it does anywhere else.
#[test]
fn errors_inside_match_arms_end_the_build() {
    for source in [
        r##"actor launcher(host :: String) do
  let _ = case host do
    "" -> self()
    _ -> Node.spawn(host, fn () -> self() end)
  end
end

fn main() do
  let _ = spawn(launcher, "")
end
"##,
        r##"actor launcher(host :: String, n :: Int) do
  let _ = case (host, n) do
    ("", _) -> self()
    (_, _) -> Node.spawn(host, fn () -> self() end)
  end
end

fn main() do
  let _ = spawn(launcher, "", 1)
end
"##,
    ] {
        let (_guard, project_dir) = project(source);
        let output = meshc_build(&project_dir, &[]).output().unwrap();
        assert!(
            stderr(&output).contains(
                "Node.spawn needs a function defined at the top level: the remote node starts it by name"
            ),
            "{}",
            stderr(&output)
        );
    }
}

/// A child's start is a closure that spawns an actor by its name: a start
/// that is no closure, or that spawns a closure, is refused.
#[test]
fn supervisor_children_start_as_closures_spawning_named_actors() {
    for (start, error) in [
        (
            "start_worker",
            "the child `w` of supervisor `Sup` must start as `fn -> spawn(actor, ...) end`",
        ),
        (
            "fn -> spawn(if true do worker else worker end) end",
            "a supervisor child must spawn an actor by its name",
        ),
    ] {
        let (_guard, project_dir) = project(&format!(
            r##"actor worker() do
  println("worker")
end

fn start_worker() -> Pid<Int> do
  spawn(worker)
end

supervisor Sup do
  child w do
    start: {start}
  end
end

fn main() do
  let _ = spawn(Sup)
end
"##
        ));
        let output = meshc_build(&project_dir, &[]).output().unwrap();
        assert!(stderr(&output).contains(error), "{}", stderr(&output));
    }
}

/// A struct cannot hold itself by value: no value of it could exist. The
/// build accepted one and failed LLVM's verification ("Cannot allocate
/// unsized type") where a function held one; it now says what is wrong.
/// Through an Option the struct is held by pointer and works.
#[test]
fn structs_hold_themselves_only_through_a_pointer() {
    let (_guard, project_dir) = project(
        r##"struct Node do
  next :: Node
end

fn keep(n :: Node) -> Node do
  n
end

fn main() do
  if false do
    let _ = keep(panic("no node"))
  end
  println("x")
end
"##,
    );
    let output = meshc_build(&project_dir, &[]).output().unwrap();
    assert!(
        stderr(&output).contains("struct `Node` holds itself by value"),
        "{}",
        stderr(&output)
    );
    let output = compile_and_run(
        r##"struct Link do
  n :: Int
  next :: Option<Link>
end

fn total(chain :: Link) -> Int do
  case chain.next do
    Some(next) -> chain.n + total(next)
    None -> chain.n
  end
end

fn main() do
  println("#{total(Link { n: 1, next: Some(Link { n: 2, next: None }) })}")
end
"##,
    );
    assert_eq!(output, "3\n");
}

/// A resource type that holds itself (a struct through an Option, a sum
/// type through its own payload) destroys every resource down the chain.
/// Its drop plan stopped where the type met itself again, so the secrets
/// of every nested value leaked: an actor holds at most 4096 secrets, and
/// the later `Secret.random`s failed.
#[test]
fn resources_that_hold_themselves_are_destroyed_all_the_way_down() {
    let output = compile_and_run(
        r##"resource struct Chain do
  key :: SecretBytes
  next :: Option<Chain>
end

type Keys do
  More(SecretBytes, Keys)
  End
end

fn once() -> Int ! CryptoError do
  let inner = Chain { key: Secret.random(1) ?, next: None }
  let outer = Chain { key: Secret.random(1) ?, next: Some(inner) }
  let keys = More(Secret.random(1) ?, More(Secret.random(1) ?, End))
  Ok(1)
end

fn churn(0, acc :: Int) -> Int do acc end
fn churn(count :: Int, acc :: Int) do
  let r = case once() do
    Ok(v) -> v
    Err(_) -> -100000
  end
  churn(count - 1, acc + r)
end

fn main() do
  println("#{churn(3000, 0)}")
end
"##,
    );
    assert_eq!(output, "3000\n");
}

/// Functions and iterators cannot be compared, and comparing values that
/// hold one (an `Option`, a list, a tuple or a set of functions) says so.
/// The type checker lets such a comparison through, and code generation
/// compared them as strings: for functions LLVM's verification failed, and
/// two different iterators were equal.
#[test]
fn values_holding_functions_or_iterators_cannot_be_compared() {
    for (comparison, error) in [
        ("Some(inc) == Some(inc)", "cannot compare values of type `(Int) -> Int`"),
        ("[inc] != [inc]", "cannot compare values of type `(Int) -> Int`"),
        ("(inc, 1) == (inc, 1)", "cannot compare values of type `(Int) -> Int`"),
        (
            "Set.size(Set.add(Set.new(), Some(inc))) > 0",
            "cannot compare values of type `(Int) -> Int`",
        ),
        ("Some(inc) < Some(inc)", "cannot order values of type `(Int) -> Int`"),
        (
            "Some(Iter.from([1])) == Some(Iter.from([2]))",
            "cannot compare values of type `Iter<Int>`: the type has no `Eq`",
        ),
    ] {
        let (_guard, project_dir) = project(&format!(
            r##"fn inc(x :: Int) -> Int do
  x + 1
end

fn main() do
  println("#{{{comparison}}}")
end
"##
        ));
        let output = meshc_build(&project_dir, &[]).output().unwrap();
        assert!(
            stderr(&output).contains(error),
            "{comparison}: {}",
            stderr(&output)
        );
    }
}
