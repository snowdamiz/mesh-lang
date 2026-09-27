//! `meshc repl` reading a session from its standard input: evaluation,
//! definitions, multi-line input, every command, and errors that do not end
//! the session.

use std::io::Write;
use std::process::{Command, Stdio};

fn repl(input: &str, home: &std::path::Path) -> (String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_meshc"))
        .arg("repl")
        .current_dir(home)
        .env("HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("meshc repl starts");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn a_piped_session_evaluates_defines_and_answers_commands() {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        home.path().join("defs.mpl"),
        "fn triple(n :: Int) -> Int do\n  n * 3\nend\n",
    )
    .unwrap();
    std::fs::write(
        home.path().join("bad.mpl"),
        "fn broken() -> Int do\n  \"no\"\nend\n",
    )
    .unwrap();
    let session = [
        "1 + 2",
        ":type 1 + 2",
        ":type 1 +",
        "let x = 5",
        "x * 2",
        "fn double(a :: Int) -> Int do",
        "  a * 2",
        "end",
        "double(21)",
        ":load defs.mpl",
        "triple(4)",
        ":load missing.mpl",
        ":load bad.mpl",
        ":help",
        ":nonsense",
        "1 + \"a\"",
        "",
        ":reset",
        "x",
        ":quit",
    ]
    .join("\n");
    let (stdout, stderr) = repl(&session, home.path());
    for expected in [
        "3 :: Int",
        "1 + 2 :: Int",
        "Defined: x",
        "10 :: Int",
        "Defined: double :: (Int) -> Int",
        "42 :: Int",
        "Loaded 'defs.mpl'",
        "12 :: Int",
        ":load <file>",
        "Goodbye!",
    ] {
        assert!(
            stdout.contains(expected),
            "{expected:?} in:\n{stdout}\n{stderr}"
        );
    }
    for expected in [
        "Parse error",
        "Failed to read 'missing.mpl'",
        "expected Int, found String",
        "Unknown command: :nonsense",
        // After :reset, `x` is no longer defined.
        "x",
    ] {
        assert!(stderr.contains(expected), "{expected:?} in:\n{stderr}");
    }
    // The session's history is kept for the next one.
    assert!(home.path().join(".mesh_repl_history").is_file());
}

#[test]
fn end_of_input_ends_the_session() {
    let home = tempfile::tempdir().unwrap();
    let (stdout, _) = repl("fn unfinished() do\n  1\n", home.path());
    assert!(stdout.ends_with("Goodbye!\n"), "{stdout}");
}

/// A value the result word does not show is printed as `inspect` shows it:
/// a list, an Option, a tuple, a map and a struct were `<List<Int> at
/// 0x...>` (an Option `at 0x0`). A function, which has no `inspect`,
/// still shows where it is.
#[test]
fn compound_results_print_their_values() {
    let home = tempfile::tempdir().unwrap();
    let session = [
        "[1, 2] |> List.map(fn n -> n * n end)",
        "Some(3) |> Option.map(fn n -> n * 2 end)",
        "(1, \"x\")",
        "%{\"k\" => [1]}",
        "struct P do\n  x :: Int\nend",
        "P { x: 4 }",
        "let f = fn n -> n end",
        "f",
        ":quit",
    ]
    .join("\n");
    let (stdout, stderr) = repl(&session, home.path());
    for expected in [
        "[1, 4] :: List<Int>",
        "Some(6) :: Option<Int>",
        "(1, \"x\") :: (Int, String)",
        "%{\"k\" => [1]} :: Map<String, List<Int>>",
        "P { x: 4 } :: P",
        "<(a) -> a at 0x",
    ] {
        assert!(
            stdout.contains(expected),
            "{expected:?} in:\n{stdout}\n{stderr}"
        );
    }
}
