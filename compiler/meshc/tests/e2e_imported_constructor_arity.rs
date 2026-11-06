//! Callable and nullary variants may share a name across distinct modules.
//! A function-typed constructor reference must not become its nullary namesake.
use std::process::Command;

#[path = "support/test_artifacts.rs"]
mod artifacts;

#[test]
fn imported_constructor_arity_stays_typed_at_both_optimization_levels() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("constructor-arity");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(
        project.join("empty.mpl"),
        "pub type Empty do\n  Shared\nend\npub fn make_empty() -> Empty do\n  Shared\nend\n",
    )
    .unwrap();
    std::fs::write(
        project.join("full.mpl"),
        "pub type Full do\n  Shared(value :: Int)\nend\n",
    )
    .unwrap();
    std::fs::write(
        project.join("main.mpl"),
        r##"from Empty import Empty, make_empty
from Full import Full

fn number(value :: Full) -> Int do
  case value do Shared(n) -> n end
end
fn empty_count(value :: Empty) -> Int do
  case value do Empty.Shared -> 1 end
end
fn wrapped() -> Full do
  Shared(7)
end
fn nested() -> Option<Full> do
  Some(Shared(11))
end
fn main() do
  let second = case nested() do Some(value) -> number(value) None -> 0 end
  println("#{number(wrapped())} #{second} #{number(Full.Shared(13))} #{empty_count(make_empty())}")
end
"##,
    )
    .unwrap();
    for optimization in ["0", "2"] {
        let binary = temp
            .path()
            .join(format!("constructor-arity-{optimization}"));
        let built = Command::new(artifacts::meshc_bin())
            .arg("build")
            .arg(&project)
            .args(["--opt-level", optimization, "-o"])
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            built.status.success(),
            "opt{optimization}: {}",
            String::from_utf8_lossy(&built.stderr)
        );
        let run = Command::new(binary).output().unwrap();
        assert!(run.status.success(), "{:?}", run);
        assert_eq!(String::from_utf8_lossy(&run.stdout), "7 11 13 1\n");
    }
}
