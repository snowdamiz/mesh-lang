//! Original cross-module conversion behavior, including structural arguments.

use std::process::{Command, Stdio};
use std::time::Duration;

#[path = "support/test_artifacts.rs"]
mod artifacts;

fn run(files: &[(&str, &str)], level: &str) -> String {
    let temporary = tempfile::tempdir().expect("temporary project");
    let project = temporary.path().join("project");
    std::fs::create_dir_all(&project).expect("project directory");
    for (name, source) in files {
        std::fs::write(project.join(name), source).expect("module source");
    }
    let binary = temporary.path().join("program");
    let built = Command::new(artifacts::meshc_bin())
        .arg("build")
        .arg(&project)
        .args(["--opt-level", level, "--output"])
        .arg(&binary)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start compiler");
    let built =
        artifacts::wait_with_timeout(built, Duration::from_secs(90)).expect("compiler finishes");
    assert!(
        built.status.success(),
        "{}",
        artifacts::command_output_text(&built)
    );
    let child = Command::new(binary)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start program");
    let output =
        artifacts::wait_with_timeout(child, Duration::from_secs(10)).expect("program finishes");
    assert!(
        output.status.success(),
        "{}",
        artifacts::command_output_text(&output)
    );
    assert!(
        output.stderr.is_empty(),
        "{}",
        artifacts::command_output_text(&output)
    );
    String::from_utf8(output.stdout).expect("UTF-8 stdout")
}

#[test]
fn imported_error_conversions_dispatch_to_both_original_bodies() {
    let files = [
        (
            "failure.mpl",
            r##"
pub struct Failure do text :: String end
impl From<String> for Failure do
  fn from(text :: String) -> Failure do Failure { text: text } end
end
impl From<Int> for Failure do
  fn from(n :: Int) -> Failure do Failure { text: "number #{n}" } end
end
"##,
        ),
        (
            "main.mpl",
            r##"
from Failure import Failure
fn text_error() -> Int!String do Err("text") end
fn int_error() -> Int!Int do Err(7) end
fn text_conversion() -> Int!Failure do Ok(text_error()?) end
fn int_conversion() -> Int!Failure do Ok(int_error()?) end
fn main() do
  case text_conversion() do Err(e) -> println(e.text) Ok(_) -> println("wrong") end
  case int_conversion() do Err(e) -> println(e.text) Ok(_) -> println("wrong") end
  let direct :: Failure = 9.into()
  println(direct.text)
end
"##,
        ),
    ];
    for level in ["0", "2"] {
        assert_eq!(run(&files, level), "text\nnumber 7\nnumber 9\n");
    }
}

#[test]
fn imported_and_local_structural_conversions_dispatch_without_reexporting() {
    let files = [
        (
            "base.mpl",
            r##"
pub struct Wrapper do n :: Int end
impl From<Int> for Wrapper do
  fn from(n :: Int) -> Wrapper do Wrapper { n: n * 2 } end
end
"##,
        ),
        (
            "more.mpl",
            r##"
from Base import Wrapper
pub type Ints = List<Int>
impl From<String> for Wrapper do
  fn from(text :: String) -> Wrapper do Wrapper { n: String.length(text) } end
end
impl From<Ints> for Wrapper do
  fn from(xs :: Ints) -> Wrapper do Wrapper { n: List.length(xs) } end
end
impl From<List<String>> for Wrapper do
  fn from(xs :: List<String>) -> Wrapper do Wrapper { n: 100 + List.length(xs) } end
end
impl From<(Int, Int)> for Wrapper do
  fn from(pair :: (Int, Int)) -> Wrapper do
    let (a, b) = pair
    Wrapper { n: a + b }
  end
end
impl From<Fun(Int) -> Int> for Wrapper do
  fn from(f :: Fun(Int) -> Int) -> Wrapper do Wrapper { n: f(20) } end
end
"##,
        ),
        (
            "main.mpl",
            r##"
from Base import Wrapper
import More
fn main() do
  let a = Wrapper.from(21)
  let b = Wrapper.from("abc")
  let c = Wrapper.from((4, 5))
  let d = Wrapper.from([1, 2])
  let e = Wrapper.from(["x"])
  let f = Wrapper.from(fn n -> n + 1 end)
  let g :: Wrapper = ["a", "b"].into()
  println("#{a.n} #{b.n} #{c.n} #{d.n} #{e.n} #{f.n} #{g.n}")
end
"##,
        ),
    ];
    for level in ["0", "2"] {
        assert_eq!(run(&files, level), "42 3 9 2 101 21 102\n");
    }
}
