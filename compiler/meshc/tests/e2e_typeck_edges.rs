//! Programs whose type checking takes a path ordinary programs rarely do,
//! compiled and run: what the checker accepts must also build and behave.

use std::path::PathBuf;
use std::process::Command;

/// Compile a one-file Mesh project and run it, returning its stdout.
fn compile_and_run(source: &str) -> String {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_dir = temp_dir.path().join("project");
    std::fs::create_dir_all(&project_dir).expect("failed to create project dir");
    std::fs::write(project_dir.join("main.mpl"), source).expect("failed to write main.mpl");

    let output = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_meshc")))
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
        "binary failed with {:?}:\n{}",
        run.status.code(),
        String::from_utf8_lossy(&run.stderr)
    );
    String::from_utf8_lossy(&run.stdout).to_string()
}

/// `?` returns early from each kind of function it can: one declaring an
/// Option, one declaring nothing, closures whose type a call gives, and a
/// function whose operand's type only its return type decides.
#[test]
fn try_returns_early_wherever_the_checker_allows_it() {
    let source = r##"
fn declared(o :: Option<Int>) -> Option<Int> do
  let v = o?
  Some(v + 1)
end

fn undeclared(o :: Option<Int>) do
  let v = o?
  Some(v * 2)
end

fn next(o) -> Option<Int> do
  let v = o?
  Some(v + 3)
end

fn bumped(r) do
  let v = r?
  Ok(v + 1)
end

fn show(o :: Option<Int>) -> String do
  case o do
    Some(n) -> "Some(#{n})"
    None -> "None"
  end
end

fn main() do
  println("#{show(declared(Some(1)))} #{show(declared(None))}")
  println("#{show(undeclared(Some(4)))} #{show(next(Some(1)))} #{show(next(None))}")
  let tens = List.map([Some(1), None], fn o -> Some(o? * 10) end)
  println("#{show(List.head(tens))} #{show(List.last(tens))}")
  let fives = List.map([Ok(1), Err("no")], fn r -> Ok(r? + 5) end)
  case List.head(fives) do
    Ok(n) -> println("ok #{n}")
    Err(e) -> println("err #{e}")
  end
  case bumped(Err("bad")) do
    Ok(n) -> println("ok #{n}")
    Err(e) -> println("err #{e}")
  end
end
"##;
    assert_eq!(
        compile_and_run(source),
        "Some(2) None\nSome(8) Some(4) None\nSome(10) None\nok 6\nerr bad\n"
    );
}

/// A `case` whose nested column is covered by `true | false` compiles as
/// exhaustive and picks the right arm.
#[test]
fn nested_bool_or_patterns_cover_both_bools() {
    let source = r##"
fn pair(a :: Bool, b :: Int) -> Int do
  case (a, b) do
    (true | false, 0) -> 0
    (true | false, n) -> n
  end
end

fn maybe(o :: Option<Bool>) -> String do
  case o do
    Some(true | false) -> "some"
    None -> "none"
  end
end

fn main() do
  println("#{pair(true, 0)} #{pair(false, 7)} #{maybe(Some(false))} #{maybe(None)}")
end
"##;
    assert_eq!(compile_and_run(source), "0 7 some none\n");
}
