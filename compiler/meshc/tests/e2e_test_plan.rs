use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[path = "support/test_artifacts.rs"]
mod artifacts;

fn project(source: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("mesh.toml"),
        "[package]\nname = \"test-plan\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("main.mpl"), "fn main() do\nend\n").unwrap();
    std::fs::write(dir.path().join("cases.test.mpl"), source).unwrap();
    dir
}

fn run(target: &Path, args: &[&str]) -> Output {
    Command::new(artifacts::meshc_bin())
        .arg("test")
        .arg(target)
        .args(args)
        .output()
        .unwrap()
}

fn text(output: &Output) -> String {
    artifacts::command_output_text(output)
}

#[test]
fn selects_names_before_running_fixtures() {
    let dir = project("describe(\"group\") do\n  setup do\n    println(\"SETUP\")\n  end\n  teardown do\n    println(\"TEARDOWN\")\n  end\n  test(\"chosen\") do\n    println(\"CHOSEN\")\n  end\n  test(\"excluded\") do\n    panic(\"EXCLUDED\")\n  end\nend\n");
    let output = run(dir.path(), &["--filter", "chosen"]);
    let text = text(&output);
    assert!(output.status.success(), "{text}");
    assert_eq!(text.matches("SETUP").count(), 1, "{text}");
    assert_eq!(text.matches("TEARDOWN").count(), 1, "{text}");
    assert!(
        text.contains("CHOSEN") && !text.contains("EXCLUDED"),
        "{text}"
    );
}

#[test]
fn selects_duplicate_names_tags_and_original_lines() {
    let source = r#"describe("outer", tags: ["shared"], async: true) do
  setup do
    let base = 40
    println("SETUP")
  end
  teardown do
    println("DOWN #{base}")
  end
  test("duplicate", tags: ["first"]) do
    println("FIRST")
    assert_eq(base, 40)
  end
  describe("inner", tags: ["nested"], async: false) do
    setup do
      let extra = base + 2
    end
    test("duplicate") do
      println("SECOND")
      assert_eq(extra, 42)
    end
  end
  test("duplicate", tags: ["last"]) do
    println("THIRD")
  end
end
"#;
    let dir = project(source);
    let output = run(
        dir.path(),
        &[
            "--name",
            "duplicate",
            "--include-tag",
            "shared",
            "--exclude-tag",
            "last",
        ],
    );
    let report = text(&output);
    assert!(output.status.success(), "{report}");
    assert!(
        report.contains("FIRST") && report.contains("SECOND") && !report.contains("THIRD"),
        "{report}"
    );
    assert_eq!(report.matches("SETUP").count(), 2, "{report}");
    assert_eq!(report.matches("DOWN 40").count(), 2, "{report}");
    let duplicates = run(dir.path(), &["--filter", "outer > duplicate"]);
    assert!(duplicates.status.success(), "{}", text(&duplicates));
    assert!(
        text(&duplicates).contains("FIRST")
            && text(&duplicates).contains("THIRD")
            && !text(&duplicates).contains("SECOND"),
        "{}",
        text(&duplicates)
    );
    assert_eq!(text(&duplicates).matches("SETUP").count(), 2);
    let output = run(dir.path(), &["--tag", "nested", "--include-tag", "shared"]);
    assert!(output.status.success(), "{}", text(&output));
    assert!(
        !text(&output).contains("FIRST") && text(&output).contains("SECOND"),
        "{}",
        text(&output)
    );
    let file = dir.path().join("cases.test.mpl");
    let line = source
        .lines()
        .position(|line| line.contains("assert_eq(extra"))
        .unwrap()
        + 1;
    let location = PathBuf::from(format!("{}:{line}", file.display()));
    let output = run(&location, &[]);
    assert!(output.status.success(), "{}", text(&output));
    assert!(
        !text(&output).contains("FIRST") && !text(&output).contains("THIRD"),
        "{}",
        text(&output)
    );
    assert_eq!(text(&output).matches("SETUP").count(), 1);
    let output = run(&PathBuf::from(format!("{}:14", file.display())), &[]);
    assert!(
        output.status.success()
            && text(&output).contains("SECOND")
            && !text(&output).contains("FIRST"),
        "{}",
        text(&output)
    );
}

#[test]
fn explicit_no_match_fails_and_empty_default_stays_successful() {
    let dir = project("test(\"exists\") do\n  panic(\"must not run\")\nend\n");
    for args in [
        &["--filter", "absent"][..],
        &["--include-tag", "absent"],
        &["--exclude-tag", "all", "--filter", "absent"],
    ] {
        let output = run(dir.path(), args);
        assert!(
            !output.status.success() && text(&output).contains("No test cases match"),
            "{}",
            text(&output)
        );
        assert!(!text(&output).contains("must not run"), "{}", text(&output));
    }
    let output = run(
        &PathBuf::from(format!(
            "{}:99",
            dir.path().join("cases.test.mpl").display()
        )),
        &[],
    );
    assert!(
        !output.status.success() && text(&output).contains("No test cases match"),
        "{}",
        text(&output)
    );
    std::fs::remove_file(dir.path().join("cases.test.mpl")).unwrap();
    let output = run(dir.path(), &[]);
    assert!(
        output.status.success() && text(&output).contains("No *.test.mpl files found."),
        "{}",
        text(&output)
    );
    let output = run(dir.path(), &["--filter", "absent"]);
    assert!(!output.status.success(), "{}", text(&output));
    let output = run(dir.path(), &["--test-threads", "0"]);
    assert!(!output.status.success(), "{}", text(&output));
}

#[test]
fn nested_failures_run_acquired_teardowns_once() {
    let dir = project(
        r#"describe("outer") do
  setup do
    let resource = 41
    println("OUTER SETUP")
  end
  teardown do
    println("OUTER DOWN #{resource}")
  end
  describe("inner") do
    setup do
      panic("setup failed")
    end
    teardown do
      println("UNACQUIRED DOWN")
    end
    test("failure") do
      println("UNREACHED BODY")
    end
  end
  describe("body") do
    setup do
      let value = resource + 1
      println("INNER SETUP")
    end
    teardown do
      println("INNER DOWN #{value}")
    end
    test("failure") do
      assert_eq(value, 0)
    end
  end
end
"#,
    );
    let output = run(dir.path(), &[]);
    let text = text(&output);
    assert!(!output.status.success(), "{text}");
    assert_eq!(text.matches("OUTER SETUP").count(), 2, "{text}");
    assert_eq!(text.matches("OUTER DOWN 41").count(), 2, "{text}");
    assert_eq!(text.matches("INNER DOWN 42").count(), 1, "{text}");
    assert!(
        !text.contains("UNACQUIRED DOWN") && !text.contains("UNREACHED BODY"),
        "{text}"
    );
    assert!(
        text.contains("[case 0]") && text.contains("[case 1]"),
        "{text}"
    );
}

struct Running {
    child: std::process::Child,
    control: PathBuf,
}
impl Drop for Running {
    fn drop(&mut self) {
        for name in ["before", "a", "b", "c", "after", "last"] {
            let _ = std::fs::write(self.control.join(format!("{name}.release")), "release");
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while matches!(self.child.try_wait(), Ok(None)) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn await_file(child: &mut Running, path: &Path) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
    while !path.exists() {
        assert!(
            child.child.try_wait().unwrap().is_none(),
            "runner ended before {}",
            path.display()
        );
        assert!(
            std::time::Instant::now() < deadline,
            "missing {}",
            path.display()
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn async_pool_has_a_bound_isolated_state_and_sync_barriers() {
    let dir = project(
        r#"actor waiting() do
  receive do
    message -> ()
  end
end

fn mark(name :: String, value :: String) do
  let result = File.write(Env.get("CONTROL", "") <> "/" <> name, value)
  assert_eq(result, Ok(()))
end

fn exists(name :: String) -> Bool do
  File.exists(Env.get("CONTROL", "") <> "/" <> name)
end

fn gate(name :: String) do
  assert(Process.whereis("same-name") == Process.whereis("absent"))
  let pid = spawn(waiting)
  Process.register("same-name", pid)
  let data = Env.get("MESH_DATA_DIR", "")
  assert(not File.exists(data <> "/sentinel"))
  let wrote = File.write(data <> "/sentinel", name)
  assert_eq(wrote, Ok(()))
  mark(name <> ".ready", data)
  while not exists(name <> ".release") do
    Timer.sleep(5)
  end
  assert(Process.whereis("same-name") == pid)
  mark(name <> ".done", "done")
end

test("sync-before") do
  gate("before")
end

describe("parallel", async: true) do
  teardown do
    println("ASYNC DOWN")
  end
  test("a") do
    assert(exists("before.done"))
    gate("a")
  end
  test("b") do
    assert(exists("before.done"))
    gate("b")
    panic("expected child failure")
  end
  test("c") do
    assert(exists("a.done") or exists("b.done"))
    gate("c")
  end
end

test("sync-after") do
  assert(exists("a.done") and exists("b.done") and exists("c.done"))
  gate("after")
end

test("async-last", async: true) do
  assert(exists("after.done"))
  gate("last")
end
"#,
    );
    let control = dir.path().join("control");
    std::fs::create_dir(&control).unwrap();
    let stdout_path = dir.path().join("stdout");
    let stderr_path = dir.path().join("stderr");
    let mut child = Running {
        child: Command::new(artifacts::meshc_bin())
            .arg("test")
            .arg(dir.path())
            .args(["--test-threads", "2"])
            .env("CONTROL", &control)
            .stdout(std::fs::File::create(&stdout_path).unwrap())
            .stderr(std::fs::File::create(&stderr_path).unwrap())
            .spawn()
            .unwrap(),
        control: control.clone(),
    };
    let release =
        |name| std::fs::write(control.join(format!("{name}.release")), "release").unwrap();
    await_file(&mut child, &control.join("before.ready"));
    assert!(!control.join("a.ready").exists());
    release("before");
    await_file(&mut child, &control.join("a.ready"));
    await_file(&mut child, &control.join("b.ready"));
    assert!(!control.join("c.ready").exists() && !control.join("after.ready").exists());
    release("b");
    await_file(&mut child, &control.join("c.ready"));
    assert!(!control.join("a.done").exists() && !control.join("after.ready").exists());
    release("c");
    release("a");
    await_file(&mut child, &control.join("after.ready"));
    assert!(!control.join("last.ready").exists());
    release("after");
    await_file(&mut child, &control.join("last.ready"));
    release("last");
    let status = child.child.wait().unwrap();
    let output = std::fs::read_to_string(stdout_path).unwrap()
        + &std::fs::read_to_string(stderr_path).unwrap();
    assert!(
        !status.success() && output.contains("expected child failure"),
        "{output}"
    );
    assert_eq!(output.matches("ASYNC DOWN").count(), 3, "{output}");
    let paths: Vec<_> = ["before", "a", "b", "c", "after", "last"]
        .into_iter()
        .map(|name| std::fs::read_to_string(control.join(format!("{name}.ready"))).unwrap())
        .collect();
    assert_eq!(
        paths.iter().collect::<std::collections::HashSet<_>>().len(),
        6,
        "{paths:?}"
    );
    assert!(
        paths.iter().all(|path| !Path::new(path).exists()),
        "child data must be removed after reaping: {paths:?}"
    );
}

#[test]
fn doctests_run_expectations_and_report_original_lines() {
    let dir = project("");
    let file = dir.path().join("arithmetic.mpl");
    let source = r#"## Arithmetic examples.
## ```mesh,doctest
## add(20, 22) # => 42
## Test.install_in_memory_secure_store()
## ```
## ```mesh,doctest
## add(1, 1)
## # => 2
## ```
## ```mesh,compile_fail
## add(true, 1)
## ```
pub fn add(a :: Int, b :: Int) -> Int do
  a + b
end
"#;
    std::fs::write(&file, source).unwrap();
    let output = run(&file, &["--doc"]);
    assert!(output.status.success(), "{}", text(&output));
    assert!(
        text(&output).contains("expected compile failure"),
        "{}",
        text(&output)
    );
    std::fs::write(&file, source.replace("# => 42", "# => 43")).unwrap();
    let output = run(&file, &["--doctests", "--filter", "line 2"]);
    assert!(
        !output.status.success() && text(&output).contains("arithmetic.mpl:2"),
        "{}",
        text(&output)
    );
    std::fs::write(&file, source.replace("add(20, 22)", "add(true, 22)")).unwrap();
    let output = run(&file, &["--doc", "--filter", "line 2"]);
    assert!(
        !output.status.success() && text(&output).contains("arithmetic.mpl:3"),
        "{}",
        text(&output)
    );
    std::fs::write(&file, "## ```mesh,compile_fail\n## 1 + 1 # => 2\n## ```\n").unwrap();
    let output = run(&file, &["--doc"]);
    assert!(
        !output.status.success() && text(&output).contains("successful compilation"),
        "{}",
        text(&output)
    );
}

#[test]
fn an_early_success_exit_is_not_a_completed_case() {
    let dir = project("test(\"early\", async: true) do\n  println(\"1 passed\")\n  println(\"CHILD_DATA=#{Env.get(\"MESH_DATA_DIR\", \"\")}\")\n  Process.exit(0)\nend\ntest(\"later\") do\n  println(\"LATER RAN\")\nend\n");
    let output = run(dir.path(), &[]);
    let text = text(&output);
    assert!(!output.status.success(), "{text}");
    assert!(
        text.contains("INCOMPLETE")
            && text.contains("[case 0] early")
            && text.contains("LATER RAN"),
        "{text}"
    );
    let data = text
        .lines()
        .find_map(|line| line.strip_prefix("CHILD_DATA="))
        .unwrap();
    assert!(
        !Path::new(data).exists(),
        "abrupt child exit must remove its data directory"
    );
}

#[test]
fn doctest_compilation_cannot_pass_on_a_broken_dependency() {
    let dir = project("");
    std::fs::write(
        dir.path().join("broken.mpl"),
        "pub fn broken() -> Int do\n  true\nend\n",
    )
    .unwrap();
    let file = dir.path().join("docs.mpl");
    std::fs::write(&file, "## ```mesh,compile_fail\n## 1 + true\n## ```\n").unwrap();
    let output = run(&file, &["--doc"]);
    assert!(
        !output.status.success() && text(&output).contains("doctest prerequisites"),
        "{}",
        text(&output)
    );
    assert!(
        !text(&output).contains("expected compile failure"),
        "{}",
        text(&output)
    );
}

#[test]
fn doctests_keep_private_scope_imports_and_test_support() {
    let dir = project("");
    let file = dir.path().join("counter.mpl");
    std::fs::write(
        &file,
        r#"from Offset import offset
## ```mesh,doctest
## private_value() + offset() # => 42
## helper() # => 40
## ```
fn private_value() -> Int do
  40
end
"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("offset.mpl"),
        "pub fn offset() -> Int do\n  2\nend\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("counter.test-support.mpl"),
        "fn helper() -> Int do\n  private_value()\nend\n",
    )
    .unwrap();
    let output = run(&file, &["--doc"]);
    assert!(output.status.success(), "{}", text(&output));
}

#[test]
fn doctests_never_execute_unmarked_test_blocks() {
    let dir = project("test(\"unmarked\") do\n  println(\"UNMARKED EFFECT\")\nend\n## ```mesh,doctest\n## 1 + 1 # => 2\n## ```\n");
    let output = run(&dir.path().join("cases.test.mpl"), &["--doc"]);
    let text = text(&output);
    assert!(
        !output.status.success() && text.contains("ordinary module"),
        "{text}"
    );
    assert!(!text.contains("UNMARKED EFFECT"), "{text}");
}

#[test]
fn explicit_file_parse_errors_keep_original_diagnostics() {
    let dir = project("test(\"broken\") do\n  let value = (1\nend\n");
    for args in [&[][..], &["--filter", "broken"][..]] {
        let output = run(&dir.path().join("cases.test.mpl"), args);
        let text = text(&output);
        assert!(!output.status.success(), "{text}");
        assert!(
            text.contains("Parse error") && text.contains("cases.test.mpl:3"),
            "{text}"
        );
        assert!(!text.contains("No test cases match"), "{text}");
    }
}

#[test]
fn raw_test_programs_execute_and_cannot_match_case_filters() {
    let dir = project("fn main() do\n  panic(\"RAW_MUST_FAIL\")\nend\n");
    let file = dir.path().join("cases.test.mpl");
    for target in [dir.path(), file.as_path()] {
        let output = run(target, &[]);
        assert!(!output.status.success(), "{}", text(&output));
        assert!(text(&output).contains("RAW_MUST_FAIL"), "{}", text(&output));
        for args in [
            &["--filter", "raw"][..],
            &["--include-tag", "network"],
            &["--exclude-tag", "network"],
        ] {
            let output = run(target, args);
            assert!(!output.status.success(), "{}", text(&output));
            assert!(
                text(&output).contains("No test cases match"),
                "{}",
                text(&output)
            );
            assert!(
                !text(&output).contains("RAW_MUST_FAIL"),
                "{}",
                text(&output)
            );
        }
    }
    let output = run(&PathBuf::from(format!("{}:2", file.display())), &[]);
    assert!(
        !output.status.success() && text(&output).contains("No test cases match"),
        "{}",
        text(&output)
    );
    std::fs::write(&file, "fn main() do\n  println(\"RAW_RAN\")\nend\n").unwrap();
    let output = run(&file, &[]);
    assert!(
        output.status.success() && text(&output).contains("RAW_RAN"),
        "{}",
        text(&output)
    );
    assert!(!text(&output).contains("INCOMPLETE"), "{}", text(&output));
    std::fs::write(&file, "").unwrap();
    for target in [dir.path(), file.as_path()] {
        let output = run(target, &[]);
        assert!(
            !output.status.success() && text(&output).contains("no `fn main()`"),
            "{}",
            text(&output)
        );
    }
}

#[test]
fn standalone_doc_expectations_strip_comments_but_keep_hashes_in_strings() {
    let dir = project("");
    let file = dir.path().join("docs.mpl");
    std::fs::write(
        &file,
        r###"## ```mesh,doctest
## 1 + 1 # explanation
## # => 2
## "keep # explanation" # actual comment
## # => "keep # explanation"
## "keep # => text" # => "keep # => text"
## "#{"nested # literal"}" # trailing comment
## # => "nested # literal"
## ```
"###,
    )
    .unwrap();
    let output = run(&file, &["--doc"]);
    assert!(output.status.success(), "{}", text(&output));
}

#[test]
fn negative_fences_check_user_code_without_expectation_rewriting() {
    let dir = project("");
    let file = dir.path().join("docs.mpl");
    for snippet in [
        "1 + 1 # explanation\n## # => 2",
        "1 + 1 # => true",
        "1 + 1 # => (",
        "# =>",
    ] {
        std::fs::write(
            &file,
            format!("## ```mesh,compile_fail\n## {snippet}\n## ```\n"),
        )
        .unwrap();
        let output = run(&file, &["--doc"]);
        assert!(
            !output.status.success() && text(&output).contains("successful compilation"),
            "{}",
            text(&output)
        );
        assert!(
            !text(&output).contains("expected compile failure"),
            "{}",
            text(&output)
        );
    }
    for snippet in [
        "1 + true # => 2",
        "let broken = (1 # explanation\n## # => 2",
    ] {
        std::fs::write(
            &file,
            format!("## ```mesh,compile_fail\n## {snippet}\n## ```\n"),
        )
        .unwrap();
        let output = run(&file, &["--doc"]);
        assert!(
            output.status.success() && text(&output).contains("expected compile failure"),
            "{}",
            text(&output)
        );
        assert!(text(&output).contains("docs.mpl:"), "{}", text(&output));
    }
}

#[test]
fn doctests_preserve_multiline_literal_bytes_and_interpolation() {
    let dir = project("");
    let file = dir.path().join("docs.mpl");
    std::fs::write(
        &file,
        r###"  ## ```mesh,doctest
  ## let text = "a
  ## b"
  ## assert_eq(text, "a\nb")
  ## let triple = """
  ##   a
  ##   b
  ##   """
  ## assert_eq(triple, "a\nb")
  ## let interpolated = "a
  ## #{"b
  ## c"}
  ## d"
  ## assert_eq(interpolated, "a\nb\nc\nd")
  ## let triple_interpolated = """
  ##   a
  ##   #{"b"}
  ##   c
  ##   """
  ## assert_eq(triple_interpolated, "a\nb\nc")
  ## ```
"###,
    )
    .unwrap();
    let output = run(&file, &["--doc"]);
    assert!(output.status.success(), "{}", text(&output));
}

#[test]
fn static_metadata_uses_runtime_string_values() {
    let dir = project(
        r###"test("""network""", tags: ["""network""", "\u{1F389}", "hash\#tag", "quote\"tag"]) do
  println("EXCLUDED_RAN")
end
test("normal", tags: ["local"]) do
  println("NORMAL_RAN")
end
"###,
    );
    for tag in ["network", "🎉", "hash#tag", "quote\"tag"] {
        let output = run(dir.path(), &["--exclude-tag", tag]);
        assert!(output.status.success(), "{}", text(&output));
        assert!(
            !text(&output).contains("EXCLUDED_RAN") && text(&output).contains("NORMAL_RAN"),
            "{}",
            text(&output)
        );
        let output = run(dir.path(), &["--include-tag", tag]);
        assert!(output.status.success(), "{}", text(&output));
        assert!(
            text(&output).contains("EXCLUDED_RAN") && !text(&output).contains("NORMAL_RAN"),
            "{}",
            text(&output)
        );
    }
    std::fs::write(
        dir.path().join("cases.test.mpl"),
        r###"test("""
  heredoc
  """, tags: ["""
  network
  """]) do
  println("HEREDOC_RAN")
end
"###,
    )
    .unwrap();
    let output = run(
        dir.path(),
        &["--filter", "heredoc", "--include-tag", "network"],
    );
    assert!(
        output.status.success() && text(&output).contains("HEREDOC_RAN"),
        "{}",
        text(&output)
    );
    for source in [
        "test(\"name\", tags: [\"#{1}\"]) do\nend\n",
        "test(\"\"\"#{1}\"\"\") do\nend\n",
    ] {
        std::fs::write(dir.path().join("cases.test.mpl"), source).unwrap();
        let output = run(dir.path(), &[]);
        assert!(
            !output.status.success() && text(&output).contains("static strings"),
            "{}",
            text(&output)
        );
    }
}

#[test]
fn negative_fences_cannot_pass_without_compiler_tools_or_valid_context() {
    let dir = project("");
    let file = dir.path().join("docs.mpl");
    std::fs::write(&file, "## ```mesh,compile_fail\n## 1 + true\n## ```\n").unwrap();
    let output = Command::new(artifacts::meshc_bin())
        .args(["test", file.to_str().unwrap(), "--doc"])
        .env("PATH", dir.path().join("missing-tools"))
        .output()
        .unwrap();
    assert!(
        !output.status.success() && text(&output).contains("doctest prerequisites"),
        "{}",
        text(&output)
    );
    assert!(
        !text(&output).contains("expected compile failure"),
        "{}",
        text(&output)
    );
    std::fs::write(
        &file,
        "fn broken() -> Int do\n  true\nend\n## ```mesh,compile_fail\n## 1 + true\n## ```\n",
    )
    .unwrap();
    let output = run(&file, &["--doc"]);
    assert!(
        !output.status.success() && text(&output).contains("doctest prerequisites"),
        "{}",
        text(&output)
    );
}

#[test]
fn negative_fences_preserve_the_existing_main_context() {
    let dir = project("");
    let file = dir.path().join("docs.mpl");
    std::fs::write(&file, "## ```mesh,compile_fail\n## true # => false\n## ```\nfn main() -> Int do\n  42\nend\nfn answer() -> Int do\n  main()\nend\n").unwrap();
    let output = run(&file, &["--doc"]);
    assert!(
        !output.status.success() && text(&output).contains("successful compilation"),
        "{}",
        text(&output)
    );
}

#[test]
fn doctest_diagnostics_keep_columns_after_continued_strings() {
    let dir = project("");
    let file = dir.path().join("docs.mpl");
    for source in [
        "  ## ```mesh,doctest\n  ## let text = \"a\n  ## b\" + )\n  ## ```\n",
        "  ## ```mesh,doctest\n  ## let text = \"a\n  ## #{missing}\"\n  ## ```\n",
    ] {
        std::fs::write(&file, source).unwrap();
        let output = run(&file, &["--doc"]);
        let needle = if source.contains("missing") {
            "missing"
        } else {
            ")"
        };
        let column = source.lines().nth(2).unwrap().find(needle).unwrap() + 1;
        assert!(!output.status.success(), "{}", text(&output));
        assert!(
            text(&output).contains(&format!("docs.mpl:3:{column}")),
            "{}",
            text(&output)
        );
    }
}
