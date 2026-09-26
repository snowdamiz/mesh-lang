//! Code generation edge cases: each test compiles a Mesh program with the
//! real compiler, runs it, and checks what it prints.

use std::process::Command;

/// Compile `source` as a one-file project and run it, returning stdout.
fn compile_and_run(source: &str) -> String {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_dir = temp_dir.path().join("project");
    std::fs::create_dir_all(&project_dir).expect("failed to create project dir");
    std::fs::write(project_dir.join("main.mpl"), source).expect("failed to write main.mpl");

    let output = Command::new(env!("CARGO_BIN_EXE_meshc"))
        .args(["build", project_dir.to_str().unwrap()])
        .output()
        .expect("failed to invoke meshc");
    assert!(
        output.status.success(),
        "meshc build failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
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
