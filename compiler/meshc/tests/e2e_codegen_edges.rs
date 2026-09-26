//! Code generation edge cases: each test compiles a Mesh program with the
//! real compiler, runs it, and checks what it prints.

use std::path::PathBuf;
use std::process::{Command, Output};

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
    let mut command = Command::new(env!("CARGO_BIN_EXE_meshc"));
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
