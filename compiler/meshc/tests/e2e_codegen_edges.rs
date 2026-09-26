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
