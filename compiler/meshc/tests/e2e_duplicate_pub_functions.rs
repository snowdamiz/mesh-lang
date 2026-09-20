#![cfg(unix)]

//! Two modules exporting a `pub fn` under the same name collapse into one symbol
//! during MIR merging, so the call site silently runs whichever module came first
//! while the type checker believed it had the imported module's signature. Reject
//! the ambiguous program instead of miscompiling it.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn meshc_bin() -> PathBuf {
    let mut path = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    if path.file_name().is_some_and(|name| name == "deps") {
        path.pop();
    }
    path.join("meshc")
}

/// Writes a project whose `a.mpl` and `b.mpl` both define `pub fn which`, with
/// `main.mpl` importing it from `b`. Returns the project directory.
fn write_project(root: &std::path::Path, name: &str, b_body: &str, main_body: &str) -> PathBuf {
    let project = root.join(name);
    fs::create_dir_all(&project).unwrap();
    fs::write(
        project.join("mesh.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n"),
    )
    .unwrap();
    fs::write(
        project.join("a.mpl"),
        "pub fn which() -> String do\n  \"A\"\nend\n",
    )
    .unwrap();
    fs::write(project.join("b.mpl"), b_body).unwrap();
    fs::write(project.join("main.mpl"), main_body).unwrap();
    project
}

#[test]
fn duplicate_pub_function_names_across_modules_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let project = write_project(
        temp.path(),
        "dup-pub-fn",
        "pub fn which() -> String do\n  \"B\"\nend\n",
        "from B import which\n\nfn main() do\n  println(which())\nend\n",
    );

    let build = Command::new(meshc_bin())
        .args(["build", project.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !build.status.success(),
        "meshc accepted two modules exporting `which`; it would have silently \
         called A.which. stdout:\n{}",
        String::from_utf8_lossy(&build.stdout)
    );

    let stderr = String::from_utf8_lossy(&build.stderr);
    assert!(
        stderr.contains("public function `which` is defined in more than one module"),
        "expected a duplicate-symbol diagnostic, got:\n{stderr}"
    );
    for module in ["`A`", "`B`"] {
        assert!(
            stderr.contains(module),
            "diagnostic should name the conflicting module {module}, got:\n{stderr}"
        );
    }
}

/// The mismatch is not merely a wrong answer: when the two definitions disagree on
/// their return type, typeck checks the call against the imported signature while
/// codegen runs the other body, reinterpreting the returned value across types.
#[test]
fn duplicate_pub_function_names_with_differing_signatures_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let project = write_project(
        temp.path(),
        "dup-pub-fn-types",
        "pub fn which() -> Int do\n  42\nend\n",
        "from B import which\n\nfn main() do\n  let n :: Int = which()\n  println(\"#{n}\")\nend\n",
    );

    let build = Command::new(meshc_bin())
        .args(["build", project.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !build.status.success(),
        "meshc accepted a program that would reinterpret A.which's String as an Int"
    );
}

/// Guard against over-rejection: distinct pub names across modules still compile,
/// and the imported function is the one that actually runs.
#[test]
fn distinct_pub_function_names_across_modules_still_build_and_call_the_import() {
    let temp = tempfile::tempdir().unwrap();
    let project = write_project(
        temp.path(),
        "distinct-pub-fn",
        "pub fn which_b() -> String do\n  \"B\"\nend\n",
        "from B import which_b\n\nfn main() do\n  println(which_b())\nend\n",
    );

    let build = Command::new(meshc_bin())
        .args(["build", project.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "meshc build failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );

    let run = Command::new(project.join("distinct-pub-fn"))
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "program failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), "B\n");
}
