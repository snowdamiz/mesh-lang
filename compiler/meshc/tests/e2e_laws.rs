#[path = "support/test_artifacts.rs"]
mod artifacts;

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::{json, Value};

const MANIFEST: &str = "[package]\nname = \"law-fixture\"\nversion = \"0.1.0\"\n";
const POLICY: &str = include_str!("fixtures/laws/policy.mpl");
const RETRY_LAW: &str = include_str!("fixtures/laws/retry.law");
const BUDGET_TEST: &str = include_str!("fixtures/laws/budget.test.mpl");

fn write(root: &Path, relative: &str, source: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, source).unwrap();
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "mesh.toml", MANIFEST);
    write(dir.path(), "main.mpl", "fn main() do nil end\n");
    write(dir.path(), "policy.mpl", POLICY);
    dir
}

fn configure(root: &Path, roots: &[&str]) {
    write(
        root,
        "mesh.toml",
        &format!("{MANIFEST}\n[verify]\nlaw_roots = {roots:?}\n"),
    );
}

fn on_cli(action: &str, root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_meshc"));
    command.arg(action).arg(root);
    command.env_remove("MESH_VERIFY_TIMEOUT_MS");
    command
}

fn list(root: &Path) -> Output {
    on_cli("verify", root)
        .args(["--list", "--json"])
        .env("MESH_VERIFY_Z3", root.join("unavailable-solver"))
        .output()
        .unwrap()
}

fn read_json(out: &Output, success: bool) -> Value {
    assert_eq!(
        out.status.success(),
        success,
        "{}",
        artifacts::command_output_text(out)
    );
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|error| panic!("{error}: {}", artifacts::command_output_text(out)))
}

fn fails(out: &Output, expected: &str) {
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success() && stderr.contains(expected),
        "{}",
        artifacts::command_output_text(out)
    );
}

fn pinned_solver_available() -> bool {
    if std::env::var_os("MESH_VERIFY_Z3").is_some() {
        // An explicit solver selection must work for the proof tests.
        return true;
    }
    let found = Command::new("z3")
        .arg("--version")
        .output()
        .is_ok_and(|result| {
            result.status.success()
                && String::from_utf8_lossy(&result.stdout).contains("Z3 version 4.15.7 ")
        });
    if !found {
        eprintln!("Proof fixture requires Z3 4.15.7 on PATH or MESH_VERIFY_Z3");
    }
    found
}

#[test]
fn list_binds_the_same_requirement_under_default_custom_and_overlapping_roots() {
    let dir = project();
    let root = dir.path();
    write(root, "policy.law", RETRY_LAW);
    write(root, "compat.law.mpl", "fn law() -> Bool do true end\n");
    write(root, ".hidden/bad.law", "invalid");
    write(root, "target/bad.law", "invalid");
    write(root, "nested/mesh.toml", MANIFEST);
    write(root, "nested/bad.law", "invalid");
    let first = read_json(&list(root), true);
    assert_eq!(first["law_roots"], json!(["."]));
    let initial = &first["laws"][0];
    assert_eq!(first["laws"].as_array().unwrap().len(), 1);
    assert_eq!(
        initial["target"],
        json!({
            "package": "law-fixture", "module": "Policy", "function": "retry",
            "arity": 2, "signature": "(Int, Int) -> Bool",
        })
    );
    assert_eq!(initial["access"], "public");
    assert_eq!(initial["location"]["path"], "policy.law");
    assert_eq!(initial["declaration"]["path"], "policy.mpl");
    assert!(initial.get("status").is_none());

    for (relative, roots) in [
        ("laws/http/eligibility.law", vec!["laws/http", "laws"]),
        ("specs/contracts/budget.law", vec!["specs/contracts"]),
        ("tests/transport.law", vec![".", "tests"]),
    ] {
        fs::remove_file(root.join("policy.law")).unwrap();
        write(
            root,
            relative,
            &format!("# moved requirement\n\n{}", RETRY_LAW.replace(" > ", ">")),
        );
        configure(root, &roots);
        let moved = read_json(&list(root), true);
        assert_eq!(moved["laws"].as_array().unwrap().len(), 1);
        let moved = &moved["laws"][0];
        assert_eq!(moved["target"], initial["target"]);
        assert_eq!(moved["requirement_digest"], initial["requirement_digest"]);
        assert_eq!(moved["location"]["path"], relative);
        fs::rename(root.join(relative), root.join("policy.law")).unwrap();
    }
}

#[test]
fn library_laws_need_no_executable_entrypoint() {
    let dir = project();
    let root = dir.path();
    fs::remove_file(root.join("main.mpl")).unwrap();
    write(root, "policy.mpl", POLICY);
    write(root, "laws/retry.law", RETRY_LAW);

    let inventory = read_json(&list(root), true);
    assert_eq!(inventory["laws"][0]["target"]["module"], "Policy");
    if pinned_solver_available() {
        assert_eq!(verify(root, true)["laws"][0]["status"], "verified");
    }
    fails(
        &on_cli("build", root).output().unwrap(),
        "Entrypoint 'main.mpl' was not found",
    );
}

#[test]
fn rejects_invalid_inventory_and_signatures() {
    let dir = project();
    let root = dir.path();
    fails(&list(root), "No laws found");
    let cases = [
        ("", "expected at least one law declaration"),
        ("from Policy import retry\n", "expected at least one law declaration"),
        ("law retry(status :: Int, left :: Int) -> Bool do true end\n", "Missing target import"),
        ("from No.Policy import retry\nlaw retry(status :: Int, left :: Int) -> Bool do true end\n", "candidates: Policy"),
        ("from Policy import absent\nlaw absent(v :: Int) -> Bool do true end\n", "No checked function"),
        ("from Policy import retry\nlaw retry(v :: Int) -> Bool do true end\n", "Policy.retry/2"),
        ("from Policy import retry\nlaw retry(v :: Bool, left :: Int) -> Bool do true end\n", "does not match the concrete target"),
        ("from Policy import retry\nlaw retry(v :: Int, left :: Int) -> Bool do helper(v) end\n", "Invalid law body"),
        ("from Policy import retry\nlaw retry(v, left :: Int) -> Bool do true end\n", "needs a type annotation"),
        ("from Policy import retry\nlaw retry(v :: Int, v :: Int) -> Bool do true end\n", "Duplicate law parameter"),
        ("from Policy import retry\nlaw retry(v :: Int, left :: Int) -> Bool do true end junk\n", "expected a newline"),
        ("from Policy import retry\nlaw retry(v :: Int, left :: Int) -> Bool do true end\nfn stray() do nil end\n", "expected a target import or a `law`"),
    ];
    for (source, failure) in cases {
        write(root, "invalid.law", source);
        fails(&list(root), failure);
    }
    write(root, "invalid.law", RETRY_LAW);
    write(root, "copy.law", RETRY_LAW);
    let duplicate = list(root);
    for diagnostic in ["Duplicate law", "copy.law:3", "invalid.law:3"] {
        fails(&duplicate, diagnostic);
    }
}

#[test]
fn rejects_malformed_roots_and_does_not_follow_symlinks() {
    let dir = project();
    let root = dir.path();
    write(root, "law/retry.law", RETRY_LAW);
    write(root, "nested/mesh.toml", MANIFEST);
    for (roots, reason) in [
        (vec![], "at least one directory"),
        (vec![""], "package-relative path"),
        (vec!["law/../law"], "inside the package"),
        (vec!["/law"], "inside the package"),
        (vec!["C:\\law"], "package-relative path"),
        (vec![".mesh"], "hidden paths"),
        (vec!["target"], "build output"),
        (vec!["nested"], "nested packages"),
        (vec!["missing"], "could not be opened"),
        (vec!["law/retry.law"], "expected a directory"),
    ] {
        configure(root, &roots);
        fails(&list(root), reason);
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(root.join("law"), root.join("alias")).unwrap();
        configure(root, &["alias"]);
        fails(&list(root), "symbolic link");
    }
}

#[test]
fn dotted_imports_build_in_either_order_and_list_both_checked_targets() {
    let dir = project();
    let root = dir.path();
    write(root, "a/policy.mpl", "pub fn first() -> Int do 111 end\n");
    write(root, "b/policy.mpl", "pub fn second() -> Int do 222 end\n");
    write(
        root,
        "laws/both.law",
        concat!(
            "from B.Policy import second\nfrom A.Policy import first\n",
            "law first() -> Int do 111 end\n",
            "law second() -> Int do 222 end\n",
        ),
    );

    let inventory = read_json(&list(root), true);
    let laws = inventory["laws"].as_array().unwrap();
    assert_eq!(laws.len(), 2);
    assert_eq!(laws[0]["target"]["module"], "A.Policy");
    assert_eq!(laws[1]["target"]["module"], "B.Policy");

    artifacts::ensure_mesh_rt_staticlib();
    for imports in [
        "from A.Policy import first\nfrom B.Policy import second",
        "from B.Policy import second\nfrom A.Policy import first",
        "from B.Policy import second\nimport A.Policy",
    ] {
        let first_call = if imports.contains("import A.Policy") {
            "Policy.first()"
        } else {
            "first()"
        };
        write(
            root,
            "main.mpl",
            &format!("{imports}\nfn main() do println(\"#{{{first_call}}}:#{{second()}}\") end\n"),
        );
        let binary = compile(root);
        let out = Command::new(binary).output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            artifacts::command_output_text(&out)
        );
        assert_eq!(out.stdout, b"111:222\n");
    }

    write(
        root,
        "main.mpl",
        "from A.Policy import first\nfrom Missing.Policy import second\nfn main() do first() end\n",
    );
    fails(
        &on_cli("build", root).arg("--json").output().unwrap(),
        "E0031",
    );

    write(root, "main.mpl", "fn main() do nil end\n");
    write(
        root,
        "a/duplicate.mpl",
        "pub fn second() -> Int do 333 end\n",
    );
    fails(
        &list(root),
        "public function `second` is defined in more than one module",
    );
}

#[test]
fn rejects_ambiguous_module_names() {
    let dir = project();
    let root = dir.path();
    write(root, "other_rule.mpl", "fn first() -> Bool do true end\n");
    write(root, "other__rule.mpl", "fn second() -> Bool do true end\n");
    write(
        root,
        "law.law",
        "from OtherRule import private first\nlaw first() -> Bool do true end\n",
    );
    let failure = list(root);
    for diagnostic in [
        "Ambiguous module `OtherRule`",
        "other_rule.mpl",
        "other__rule.mpl",
    ] {
        fails(&failure, diagnostic);
    }
}

#[test]
fn private_laws_bind_the_checked_module_with_a_custom_entrypoint() {
    let dir = project();
    let root = dir.path();
    write(
        root,
        "mesh.toml",
        &format!("{MANIFEST}entrypoint = \"lib/start.mpl\"\n"),
    );
    write(root, "main.mpl", "fn retry() -> Bool do true end\n");
    write(
        root,
        "lib/start.mpl",
        "fn retry() -> Bool do false end\nfn main() do nil end\n",
    );
    write(
        root,
        "laws/root.law",
        "from Main import private retry\nlaw retry() -> Bool do true end\n",
    );
    write(
        root,
        "laws/entry.law",
        "from Lib.Start import private retry\nlaw retry() -> Bool do false end\n",
    );
    let inventory = read_json(&list(root), true);
    let laws = inventory["laws"].as_array().unwrap();
    assert_eq!(laws.len(), 2);
    for (law, module, source) in [
        (&laws[0], "Lib.Start", "lib/start.mpl"),
        (&laws[1], "Main", "main.mpl"),
    ] {
        assert_eq!(law["access"], "private");
        assert_eq!(law["target"]["module"], module);
        assert_eq!(law["declaration"]["path"], source);
    }
    if pinned_solver_available() {
        let proof = verify(root, true);
        assert!(proof["laws"]
            .as_array()
            .unwrap()
            .iter()
            .all(|law| law["status"] == "verified"));
    }

    write(
        root,
        "laws/root.law",
        "from Main import retry\nlaw retry() -> Bool do true end\n",
    );
    fails(&list(root), "is private");
    fs::remove_file(root.join("laws/root.law")).unwrap();
    for (import, diagnostic) in [
        ("from Main import retry", "E0035"),
        ("from Main import private retry", "P0001"),
    ] {
        write(
            root,
            "lib/start.mpl",
            &format!("{import}\nfn main() do nil end\n"),
        );
        fails(
            &on_cli("build", root).arg("--json").output().unwrap(),
            diagnostic,
        );
    }

    write(root, "lib/start.mpl", "fn main() do nil end\n");
    write(root, "mesh.toml", &format!("{MANIFEST}entrypoint = \"lib/start.mpl\"\n[dependencies]\nother = {{ path = \"deps/other\" }}\n"));
    write(
        root,
        "deps/other/mesh.toml",
        "[package]\nname = \"other\"\nversion = \"0.1.0\"\n",
    );
    write(
        root,
        "deps/other/rules.mpl",
        "fn retry() -> Bool do true end\n",
    );
    write(
        root,
        "laws/entry.law",
        "from Rules import private retry\nlaw retry() -> Bool do true end\n",
    );
    fails(&list(root), "must belong to the selected package");
}

fn compile(root: &Path) -> std::path::PathBuf {
    let binary = root.join(if cfg!(windows) {
        "runner.exe"
    } else {
        "runner"
    });
    let built = on_cli("build", root)
        .arg("--output")
        .arg(&binary)
        .env("MESH_VERIFY_Z3", root.join("unavailable-solver"))
        .env("MESH_VERIFY_LEAN", root.join("unavailable-kernel"))
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        artifacts::command_output_text(&built)
    );
    binary
}

fn budget_passes(root: &Path) {
    let tested = on_cli("test", &root.join("tests/budget.test.mpl"))
        .env("MESH_VERIFY_Z3", root.join("unavailable-solver"))
        .output()
        .unwrap();
    assert!(
        tested.status.success(),
        "{}",
        artifacts::command_output_text(&tested)
    );
    assert!(String::from_utf8_lossy(&tested.stdout)
        .contains("retry eligibility across HTTP statuses and remaining budgets"));
}

fn verify(root: &Path, success: bool) -> Value {
    read_json(
        &on_cli("verify", root).arg("--json").output().unwrap(),
        success,
    )
}

#[test]
fn proves_retry_and_replays_the_capped_counterexample_on_the_native_program() {
    if !pinned_solver_available() {
        return;
    }
    artifacts::ensure_mesh_rt_staticlib();
    let dir = project();
    let root = dir.path();
    write(root, "laws/retry.law", RETRY_LAW);
    write(root, "tests/budget.test.mpl", BUDGET_TEST);

    let baseline = verify(root, true);
    assert_eq!(baseline["laws"][0]["status"], "verified");
    budget_passes(root);

    write(
        root,
        "policy.mpl",
        &POLICY.replace("remaining > 0", "remaining > 0 && remaining <= 8"),
    );
    budget_passes(root);
    let proof = verify(root, false);
    let verdict = &proof["laws"][0];
    assert_eq!(verdict["status"], "falsified");
    let status = verdict["inputs"]["status"].as_i64().unwrap();
    let left = verdict["inputs"]["left"].as_i64().unwrap();
    assert!(matches!(status, 429 | 503) && left > 8, "{verdict}");
    assert_eq!(verdict["expected"], true);
    assert_eq!(verdict["actual"], false);

    let expected = RETRY_LAW
        .split_once("\n\n")
        .unwrap()
        .1
        .replacen("law retry", "fn expected", 1);
    write(root, "main.mpl", &format!(
        "from Policy import retry\n{expected}\nfn main() do\n  println(\"#{{expected({status}, {left})}} #{{retry({status}, {left})}}\")\nend\n",
    ));
    let out = Command::new(compile(root)).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        artifacts::command_output_text(&out)
    );
    assert_eq!(out.stdout, b"true false\n");

    write(root, "policy.mpl", POLICY);
    assert_eq!(verify(root, true)["laws"][0]["status"], "verified");
    budget_passes(root);
}

#[test]
fn proves_an_integer_result_and_replays_its_signed_counterexample() {
    if !pinned_solver_available() {
        return;
    }
    artifacts::ensure_mesh_rt_staticlib();
    let dir = project();
    let root = dir.path();
    write(
        root,
        "policy.mpl",
        "pub fn charge(credit :: Int) -> Int do credit - 1 end\n",
    );
    write(
        root,
        "laws/charge.law",
        "from Policy import charge\nlaw charge(credit :: Int) -> Int do credit + -1 end\n",
    );
    assert_eq!(verify(root, true)["laws"][0]["status"], "verified");

    write(
        root,
        "policy.mpl",
        "pub fn charge(credit :: Int) -> Int do\n  if credit == 10 do\n    -11\n  else\n    credit - 1\n  end\nend\n",
    );
    let report = verify(root, false);
    let verdict = &report["laws"][0];
    assert_eq!(verdict["status"], "falsified");
    assert_eq!(verdict["inputs"], json!({"credit": 10}));
    assert_eq!(verdict["expected"], 9);
    assert_eq!(verdict["actual"], -11);

    write(
        root,
        "main.mpl",
        "from Policy import charge\nfn main() do println(\"#{charge(10)}\") end\n",
    );
    let output = Command::new(compile(root)).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        artifacts::command_output_text(&output)
    );
    assert_eq!(output.stdout, b"-11\n");
}

#[test]
fn checks_a_mesh_induction_proof_against_the_compiled_recursive_list_function() {
    let policy = include_str!("fixtures/proofs/flags.mpl");
    let law = include_str!("fixtures/proofs/flags.law");
    let dir = project();
    let root = dir.path();
    write(root, "flags.mpl", policy);
    write(root, "laws/flags.law", law);

    let inventory = read_json(&list(root), true);
    assert_eq!(
        inventory["laws"][0]["proof_location"]["path"],
        "laws/flags.law"
    );
    let absent = on_cli("verify", root)
        .arg("--json")
        .env("MESH_VERIFY_LEAN", root.join("unavailable-kernel"))
        .output()
        .unwrap();
    let unavailable = read_json(&absent, false);
    assert_eq!(unavailable["laws"][0]["status"], "unverified");
    assert!(unavailable["laws"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("could not start Lean"));

    if std::env::var_os("MESH_VERIFY_LEAN").is_none() {
        return;
    }
    artifacts::ensure_mesh_rt_staticlib();
    assert_eq!(verify(root, true)["laws"][0]["status"], "verified");
    let artifacts_dir = root.join(".proof-artifacts");
    let emit = || {
        on_cli("verify", root)
            .args(["--json", "--emit-proof"])
            .arg(&artifacts_dir)
            .output()
            .unwrap()
    };
    assert_eq!(read_json(&emit(), true)["laws"][0]["status"], "verified");
    assert_eq!(read_json(&emit(), true)["laws"][0]["status"], "verified");
    let emitted = fs::read_dir(&artifacts_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(emitted.len(), 1);
    let replay = Command::new(std::env::var_os("MESH_VERIFY_LEAN").unwrap())
        .current_dir(&artifacts_dir)
        .arg("--trust=0")
        .arg(emitted[0].file_name().unwrap())
        .output()
        .unwrap();
    assert!(
        replay.status.success(),
        "{}",
        artifacts::command_output_text(&replay)
    );
    assert!(String::from_utf8_lossy(&replay.stdout).contains("does not depend on any axioms"));
    fs::write(&emitted[0], "different file").unwrap();
    assert_eq!(read_json(&emit(), false)["laws"][0]["status"], "unverified");
    assert_eq!(fs::read(&emitted[0]).unwrap(), b"different file");

    write(
        root,
        "main.mpl",
        "from Flags import drop_false_prefix\nfn main() do\n  println(\"#{List.length(drop_false_prefix([]))}\")\n  println(\"#{List.length(drop_false_prefix([false, false]))}\")\n  println(\"#{List.length(drop_false_prefix([false, true, false]))}\")\nend\n",
    );
    let output = Command::new(compile(root)).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        artifacts::command_output_text(&output)
    );
    assert_eq!(output.stdout, b"0\n0\n2\n");

    for candidate in [
        policy.replace("        xs\n", "        []\n"),
        policy.replace("drop_false_prefix(tail)", "drop_false_prefix(xs)"),
        policy.replace("  case xs", "  let _ = DateTime.utc_now()\n  case xs"),
    ] {
        write(root, "flags.mpl", &candidate);
        assert_eq!(verify(root, false)["laws"][0]["status"], "unverified");
    }
    write(root, "flags.mpl", policy);
    write(
        root,
        "shadow.mpl",
        "pub fn mesh_list_length() -> Int do 0 end\n",
    );
    let shadowed = verify(root, false);
    assert!(shadowed["laws"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("shadowed runtime symbol `mesh_list_length`"));
    fs::remove_file(root.join("shadow.mpl")).unwrap();

    write(
        root,
        "laws/flags.law",
        &law.replace("exact ih", "exact tail"),
    );
    let rejected = verify(root, false);
    assert_eq!(rejected["laws"][0]["status"], "unverified");
    assert_eq!(
        rejected["laws"][0]["unsupported_at"]["path"],
        "laws/flags.law"
    );

    write(root, "laws/flags.law", law);
    let baseline = root.join(".baseline.json");
    fs::write(&baseline, inventory.to_string()).unwrap();
    let report = on_cli("verify", root)
        .args(["--json", "--preserve"])
        .arg(&baseline)
        .output()
        .unwrap();
    let preserved = read_json(&report, true);
    assert_eq!(preserved["preservation"]["preserved"], true);
    assert_eq!(preserved["laws"][0]["status"], "verified");

    write(root, "main.mpl", "fn main() do nil end\n");
    write(root, "flags.mpl", &policy.replacen("pub ", "", 1));
    write(
        root,
        "laws/flags.law",
        &law.replace(
            "import drop_false_prefix",
            "import private drop_false_prefix",
        ),
    );
    write(
        root,
        "collision.mpl",
        "pub fn Flags__drop_false_prefix(xs :: List<Bool>) -> List<Bool> do [] end\n",
    );
    let collided = verify(root, false);
    assert!(collided["laws"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("ambiguous kernel model implementation symbol"));
}

#[test]
fn proves_recursive_integer_list_append_using_congruence() {
    if std::env::var_os("MESH_VERIFY_LEAN").is_none() {
        return;
    }
    artifacts::ensure_mesh_rt_staticlib();
    let dir = project();
    let root = dir.path();
    let policy = r#"pub fn append_left(xs :: List<Int>, ys :: List<Int>) -> List<Int> do
  case xs do
    [] -> ys
    head :: tail -> List.concat([head], append_left(tail, ys))
  end
end
"#;
    write(root, "sequence.mpl", policy);
    write(
        root,
        "laws/append.law",
        r#"from Sequence import append_left

law append_left(xs :: List<Int>, ys :: List<Int>) -> List<Int> do
  List.concat(xs, ys)
end

proof append_left do
  induction xs do
    [] -> refl
    head :: tail, ih -> congr List.cons(head), ih
  end
end
"#,
    );
    assert_eq!(verify(root, true)["laws"][0]["status"], "verified");
    write(
        root,
        "main.mpl",
        r##"from Sequence import append_left

fn main() do
  for value in append_left([-42, 0], [1]) do
    println("#{value}")
  end
end
"##,
    );
    let output = Command::new(compile(root)).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        artifacts::command_output_text(&output)
    );
    assert_eq!(output.stdout, b"-42\n0\n1\n");

    write(
        root,
        "sequence.mpl",
        &policy.replace(
            "List.concat([head], append_left(tail, ys))",
            "append_left(tail, ys)",
        ),
    );
    let rejected = verify(root, false);
    assert_eq!(rejected["laws"][0]["status"], "unverified");
    assert_eq!(
        rejected["laws"][0]["unsupported_at"]["path"],
        "laws/append.law"
    );
}

#[test]
fn checks_generic_induction_quantified_propositions_and_reuses_a_lemma_across_files() {
    let dir = project();
    let root = dir.path();
    let declarations = include_str!("fixtures/proofs/propositions.law");
    write(root, "laws/z-propositions.law", declarations);
    write(root, "laws/a-use.law", "logic\n\ntheorem copying_twice(T :: Type, chain :: Chain<T>) do\n  copy(T, copy(T, chain)) == chain\nend\n\nproof copying_twice do\n  exact Eq.trans(copy_is_identity(T, copy(T, chain)), copy_is_identity(T, chain))\nend\n");

    let inventory = read_json(&list(root), true);
    assert_eq!(inventory["laws"].as_array().unwrap().len(), 4);
    let baseline = root.join(".baseline.json");
    fs::write(&baseline, inventory.to_string()).unwrap();
    if std::env::var_os("MESH_VERIFY_LEAN").is_none() {
        return;
    }
    let evidence = root.join(".proof-evidence");
    let preserved = |success| {
        read_json(
            &on_cli("verify", root)
                .args(["--json", "--verdict", "--emit-proof"])
                .arg(&evidence)
                .arg("--preserve")
                .arg(&baseline)
                .output()
                .unwrap(),
            success,
        )
    };
    let report = preserved(true);
    assert_eq!(report["preservation"]["preserved"], true);
    assert!(report["laws"]
        .as_array()
        .unwrap()
        .iter()
        .all(|law| law["status"] == "verified"));
    let extensions = fs::read_dir(&evidence)
        .unwrap()
        .map(|file| {
            file.unwrap()
                .path()
                .extension()
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned()
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(extensions, ["lean".to_owned(), "ndjson".to_owned()].into());

    fs::rename(
        root.join("laws/z-propositions.law"),
        root.join("laws/0-foundations.law"),
    )
    .unwrap();
    assert_eq!(preserved(true)["preservation"]["preserved"], true);
    write(
        root,
        "laws/0-foundations.law",
        &declarations.replace("Chain.Link(head, copy(T, tail))", "Chain.End()"),
    );
    let rejected = preserved(false);
    assert_eq!(rejected["preservation"]["preserved"], false);
    assert!(rejected["laws"]
        .as_array()
        .unwrap()
        .iter()
        .any(|law| law["status"] == "unverified"));
}

#[test]
fn preserves_the_owner_of_an_unqualified_logic_constructor() {
    let dir = project();
    let root = dir.path();
    let law = r#"logic
theorem a_leaf() do Leaf(1) == Leaf(1) end
proof a_leaf do refl end
inductive ZTree do
  Leaf(value :: Nat)
end
"#;
    write(root, "laws/tree.law", law);
    let initial = read_json(&list(root), true);
    if std::env::var_os("MESH_VERIFY_LEAN").is_some() {
        assert_eq!(verify(root, true)["laws"][0]["status"], "verified");
    }

    write(
        root,
        "laws/tree.law",
        &law.replace("Leaf(value :: Nat)\n", "Leaf(value :: Nat)\n  Bud()\n"),
    );
    let changed = read_json(&list(root), true);
    assert_ne!(
        initial["laws"][0]["requirement_digest"],
        changed["laws"][0]["requirement_digest"]
    );
}

#[test]
fn checks_named_propositions_about_the_native_implementation_and_integer_overflow() {
    if std::env::var_os("MESH_VERIFY_LEAN").is_none() {
        return;
    }
    artifacts::ensure_mesh_rt_staticlib();
    let dir = project();
    let root = dir.path();
    let policy = "pub fn after_max() -> Int do 9223372036854775807 + 1 end\n";
    write(root, "counter.mpl", policy);
    write(
        root,
        "laws/overflow.law",
        r#"logic
from Counter import after_max

theorem wraps() do after_max() == (0 - 9223372036854775808) end
proof wraps do refl end

theorem signed_order() do after_max() < 0 end
proof signed_order do refl end

theorem natural_order() do (if 0 < 1 do true else false end) == true end
proof natural_order do refl end
"#,
    );
    assert_eq!(verify(root, true)["laws"][0]["status"], "verified");
    write(
        root,
        "main.mpl",
        "from Counter import after_max\nfn main() do println(\"#{after_max()}\") end\n",
    );
    let output = Command::new(compile(root)).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        artifacts::command_output_text(&output)
    );
    assert_eq!(output.stdout, b"-9223372036854775808\n");

    write(root, "counter.mpl", &policy.replace("+ 1", "+ 2"));
    let report = verify(root, false);
    assert_eq!(report["laws"][0]["status"], "unverified");
    assert!(report["laws"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("proof did not check"));
}

#[test]
fn checks_tuple_propositions_against_the_native_layout() {
    if std::env::var_os("MESH_VERIFY_LEAN").is_none() {
        return;
    }
    artifacts::ensure_mesh_rt_staticlib();
    let dir = project();
    let root = dir.path();
    let source = r#"
pub fn select(packet :: (List<List<Int>>, Bool, List<Int>)) -> (Bool, List<Int>, Int) do
  (Tuple.second(packet), Tuple.nth(packet, 2), Tuple.size(packet))
end
"#;
    write(root, "packets.mpl", source);
    write(
        root,
        "laws/packets.law",
        r#"logic
from Packets import select

theorem selects_fields(packet :: Prod<List<List<Int>>, Prod<Bool, List<Int>>>) do
  select(packet) == Prod.mk(Prod.fst(Prod.snd(packet)), Prod.mk(Prod.snd(Prod.snd(packet)), 3))
end
proof selects_fields do refl end
"#,
    );
    let verified = verify(root, true);
    assert_eq!(verified["laws"][0]["status"], "verified");
    write(
        root,
        "main.mpl",
        r##"from Packets import select
fn main() do
  let result = select(([[-9223372036854775807 - 1], []], true, [3, 5]))
  println("#{Tuple.first(result)} #{List.length(Tuple.second(result))} #{Tuple.nth(result, 2)}")
end
"##,
    );
    let output = Command::new(compile(root)).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        artifacts::command_output_text(&output)
    );
    assert_eq!(output.stdout, b"true 2 3\n");

    write(
        root,
        "packets.mpl",
        &source.replace("Tuple.size(packet)", "2"),
    );
    let rejected = verify(root, false);
    assert_eq!(rejected["laws"][0]["status"], "unverified");
    assert!(rejected["laws"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("proof did not check"));
}

#[test]
fn proves_properties_of_compiled_recursive_sums_and_records() {
    if std::env::var_os("MESH_VERIFY_LEAN").is_none() {
        return;
    }
    artifacts::ensure_mesh_rt_staticlib();
    let dir = project();
    let root = dir.path();
    let source = include_str!("fixtures/proofs/packets.mpl");
    write(root, "packets.mpl", source);
    write(
        root,
        "laws/packets.law",
        r#"logic
from Packets import Chain
from Packets import Meter
from Packets import copy_chain
from Packets import meter_value

theorem copies_links(chain :: Chain) do
  copy_chain(chain) == chain
end
proof copies_links do
  induction chain do
    Chain.Empty() -> refl
    Chain.Link(head, tail, ih) ->
      exact congr(fn rest :: Chain -> Chain.Link(head, rest) end, ih)
  end
end

theorem reads_meter(value :: Int, active :: Bool) do
  meter_value(Meter.mk(value, active)) == value
end
proof reads_meter do refl end
"#,
    );
    let inventory = read_json(&list(root), true);
    let checked = read_json(
        &on_cli("verify", root)
            .args(["--json", "--verdict"])
            .output()
            .unwrap(),
        true,
    );
    assert!(checked["laws"]
        .as_array()
        .unwrap()
        .iter()
        .all(|law| law["status"] == "verified"));

    write(
        root,
        "main.mpl",
        r##"from Packets import Chain, Meter, copy_chain, meter_value
fn main() do
  let meter = Meter { value: -19, active: true }
  let value = case copy_chain(Link(2, Link(3, Empty))) do
    Link(first, Link(second, Empty)) -> first + second
    _ -> 0
  end
  println("#{meter_value(meter)} #{value}")
end
"##,
    );
    let output = Command::new(compile(root)).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        artifacts::command_output_text(&output)
    );
    assert_eq!(output.stdout, b"-19 5\n");

    write(
        root,
        "packets.mpl",
        &source.replace("Link(head, copy_chain(tail))", "copy_chain(tail)"),
    );
    let rejected = verify(root, false);
    assert_eq!(rejected["laws"][0]["status"], "unverified");
    assert!(rejected["laws"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("proof did not check"));
    let changed = source
        .replace("  Empty\n", "  Empty\n  Sentinel\n")
        .replace(
            "    Empty -> Empty\n",
            "    Empty -> Empty\n    Sentinel -> Sentinel\n",
        );
    write(root, "packets.mpl", &changed);
    let changed = read_json(&list(root), true);
    assert_ne!(
        inventory["laws"][0]["requirement_digest"],
        changed["laws"][0]["requirement_digest"]
    );
    assert_eq!(
        inventory["laws"][1]["requirement_digest"],
        changed["laws"][1]["requirement_digest"]
    );
}

#[test]
fn verifies_the_requested_generic_specialization_with_a_shared_pointer_abi() {
    let dir = project();
    let root = dir.path();
    write(
        root,
        "collections.mpl",
        r#"
pub fn copy<T>(items :: List<T>) -> List<T> do
  case items do
    [] -> []
    head :: tail -> [head] ++ copy(tail)
  end
end
"#,
    );
    let spec = r#"logic
from Collections import copy(List<Int>) -> List<Int>

theorem copies_ints(items :: List<Int>) do copy(items) == items end
proof copies_ints do
  induction items do
    List.nil() -> refl
    List.cons(head, tail, ih) -> exact congr(List.cons(head), ih)
  end
end
"#;
    write(root, "laws/copy.law", spec);
    let before = read_json(&list(root), true);
    assert_eq!(before["laws"].as_array().unwrap().len(), 1);
    if std::env::var_os("MESH_VERIFY_LEAN").is_some() {
        assert_eq!(verify(root, true)["laws"][0]["status"], "verified");

        artifacts::ensure_mesh_rt_staticlib();
        write(
            root,
            "main.mpl",
            r##"from Collections import copy
fn main() do
  let ints = copy([1, 4])
  let flags = copy([false, true])
  println("#{List.length(ints)} #{List.length(flags)}")
end
"##,
        );
        let output = Command::new(compile(root)).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            artifacts::command_output_text(&output)
        );
        assert_eq!(output.stdout, b"2 2\n");
        assert_eq!(verify(root, true)["laws"][0]["status"], "verified");
    }

    write(
        root,
        "laws/copy.law",
        &spec.replace("-> List<Int>\n", "-> List<Bool>\n"),
    );
    fails(&list(root), "not an instance of its checked declaration");
}

#[test]
fn replays_indexed_data_dependent_pairs_and_impossible_propositions() {
    if std::env::var_os("MESH_VERIFY_LEAN").is_none() {
        return;
    }
    let dir = project();
    let root = dir.path();
    let source = r#"logic
inductive Vector(T :: Type) -> Nat -> Type do
  Empty() -> Vector(T, 0)
  Push(n :: Nat, head :: T, tail :: Vector<T, n>) -> Vector(T, Nat.succ(n))
end

fn packed(T :: Type, element :: T) -> Sigma(fn n :: Nat -> Vector(T, n) end) do
  let vector :: Vector<T, 1> = Vector.Push(0, element, Vector.Empty())
  (1, vector)
end

theorem packs_one(T :: Type, element :: T) do
  Sigma.fst(packed(T, element)) == 1
end
proof packs_one do refl end

inductive Even -> Nat -> Prop do
  Zero() -> Even(0)
  Step(n :: Nat, h :: Even<n>) -> Even(Nat.succ(Nat.succ(n)))
end

theorem one_is_odd(impossible :: Even<1>) do False end
proof one_is_odd do cases impossible do end end
"#;
    write(root, "laws/indexed.law", source);
    let replay = |success| {
        read_json(
            &on_cli("verify", root)
                .args(["--json", "--verdict"])
                .output()
                .unwrap(),
            success,
        )
    };
    assert!(replay(true)["laws"]
        .as_array()
        .unwrap()
        .iter()
        .all(|law| law["status"] == "verified"));

    write(
        root,
        "laws/indexed.law",
        &source.replace("(1, vector)", "(2, vector)"),
    );
    let rejected = replay(false);
    assert!(rejected["laws"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("proof did not check"));
}

#[test]
fn reports_an_open_proof_goal_without_accepting_it() {
    if std::env::var_os("MESH_VERIFY_LEAN").is_none() {
        return;
    }
    let dir = project();
    let root = dir.path();
    write(
        root,
        "laws/open.law",
        "logic\ntheorem pending(value :: Nat) do value == value end\nproof pending do goal end\n",
    );
    let result = verify(root, false);
    let law = &result["laws"][0];
    assert_eq!(law["status"], "unverified");
    let reason = law["reason"].as_str().unwrap();
    assert!(reason.contains("open Mesh proof goal"), "{reason}");
    assert!(reason.contains("⊢"), "{reason}");
}

#[test]
fn ordinary_builds_check_proofs_declared_in_mesh_source() {
    let dir = project();
    let root = dir.path();
    let source = r#"pub fn expected() -> Int = 42

logic do
  from Policy import expected

  theorem policy_value() do expected() == 42 end
  proof policy_value do refl end
end
"#;
    write(root, "policy.mpl", source);
    write(
        root,
        "main.mpl",
        "from Policy import expected\nfn main() do println(\"${expected()}\") end\n",
    );
    let inventory = read_json(&list(root), true);
    let law = &inventory["laws"][0];
    assert_eq!(law["target"]["function"], "policy_value");
    assert_eq!(law["location"]["path"], "policy.mpl");
    assert_eq!(law["location"]["line"], 6);

    let unavailable = on_cli("build", root)
        .env("MESH_VERIFY_LEAN", root.join("missing-lean"))
        .output()
        .unwrap();
    fails(&unavailable, "set MESH_VERIFY_LEAN");

    if std::env::var_os("MESH_VERIFY_LEAN").is_some() {
        artifacts::ensure_mesh_rt_staticlib();
        let binary = root.join(if cfg!(windows) {
            "proof_runner.exe"
        } else {
            "proof_runner"
        });
        let built = on_cli("build", root)
            .arg("--output")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            built.status.success(),
            "{}",
            artifacts::command_output_text(&built)
        );
        let native = Command::new(binary).output().unwrap();
        assert!(native.status.success());
        assert_eq!(native.stdout, b"42\n");

        write(
            root,
            "policy.mpl",
            &source.replace("-> Int = 42", "-> Int = 41"),
        );
        let rejected = on_cli("build", root).output().unwrap();
        fails(&rejected, "proof did not check");
    }
}

#[test]
fn project_rejects_identically_spelled_indexed_families_from_distinct_modules() {
    let dir = project();
    let root = dir.path();
    let indexed = "type Ticket<n :: Nat> do\n  Initial -> Ticket<0>\nend\n";
    write(root, "first.mpl", indexed);
    write(root, "second.mpl", indexed);
    let rejected = on_cli("build", root).output().unwrap();
    fails(&rejected, "`Ticket` is defined in more than one module");
}

#[test]
fn compiled_indexed_vectors_check_lengths_before_native_execution() {
    artifacts::ensure_mesh_rt_staticlib();
    let dir = project();
    let root = dir.path();
    let source = include_str!("fixtures/proofs/indexed_native/main.mpl");
    let vector = include_str!("fixtures/proofs/indexed_native/vector.mpl");
    write(root, "vector.mpl", vector);
    write(root, "main.mpl", source);
    let native = Command::new(compile(root)).output().unwrap();
    assert!(
        native.status.success(),
        "{}",
        artifacts::command_output_text(&native)
    );
    assert_eq!(native.stdout, b"7 42 true 0\n");

    write(
        root,
        "main.mpl",
        &source.replace("double :: Vec<Int, 2>", "double :: Vec<Int, 3>"),
    );
    let rejected = on_cli("build", root).output().unwrap();
    fails(&rejected, "Vec<Int, 3>");

    write(root, "main.mpl", source);
    write(
        root,
        "vector.mpl",
        &vector.replace("Push(head, copy(tail))", "copy(tail)"),
    );
    let rejected = on_cli("build", root).output().unwrap();
    fails(&rejected, "type mismatch");
}

#[test]
fn proves_a_compiled_function_at_a_checked_vector_index() {
    if std::env::var_os("MESH_VERIFY_LEAN").is_none() {
        return;
    }
    let dir = project();
    let root = dir.path();
    let vector = include_str!("fixtures/proofs/indexed_native/vector.mpl");
    write(root, "vector.mpl", vector);
    write(
        root,
        "laws/first.law",
        include_str!("fixtures/proofs/indexed_native/first.law"),
    );
    let replay = |success| {
        read_json(
            &on_cli("verify", root)
                .args(["--verdict", "--json"])
                .output()
                .unwrap(),
            success,
        )
    };
    assert_eq!(replay(true)["laws"][0]["status"], "verified");

    write(
        root,
        "vector.mpl",
        &vector.replace("= first(values)", "= 41"),
    );
    let report = replay(false);
    assert!(report["laws"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("proof did not check"));

    write(root, "vector.mpl", vector);
    write(
        root,
        "laws/first.law",
        r#"logic
from Vector import Vec<Int, 0> as EmptyVector
from Vector import Vec<Int, 1> as SingleVector

theorem cannot_misapply(value :: Int) do
  let child :: SingleVector = SingleVector.Push(value, EmptyVector.Empty)
  SingleVector.Push(value, child) == SingleVector.Push(value, child)
end

proof cannot_misapply do refl end
"#,
    );
    let report = replay(false);
    assert!(report["laws"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("proof did not check"));
}

#[test]
fn proves_compiled_vector_head_and_copy_for_all_element_types_and_lengths() {
    if std::env::var_os("MESH_VERIFY_LEAN").is_none() {
        return;
    }
    let dir = project();
    let root = dir.path();
    write(
        root,
        "vector.mpl",
        include_str!("fixtures/proofs/indexed_native/vector.mpl"),
    );
    write(
        root,
        "main.mpl",
        include_str!("fixtures/proofs/indexed_native/main.mpl"),
    );
    write(
        root,
        "laws/parametric.law",
        include_str!("fixtures/proofs/indexed_native/parametric.law"),
    );
    write(
        root,
        "laws/copy.law",
        include_str!("fixtures/proofs/indexed_native/copy.law"),
    );
    let replay = on_cli("verify", root)
        .args(["--verdict", "--json"])
        .output()
        .unwrap();
    let report = read_json(&replay, true);
    let laws = report["laws"].as_array().unwrap();
    assert_eq!(laws.len(), 2);
    assert!(laws.iter().all(|law| law["status"] == "verified"));
}

#[test]
fn imported_theorem_evidence_is_checked_before_native_execution() {
    let dir = project();
    let root = dir.path();
    let vector = include_str!("fixtures/proofs/evidence_native/vector.mpl");
    let main = include_str!("fixtures/proofs/evidence_native/main.mpl");
    write(root, "vector.mpl", vector);

    write(
        root,
        "main.mpl",
        &main.replace("deliver(double,", "deliver(single,"),
    );
    let wrong_index = on_cli("build", root)
        .env("MESH_VERIFY_LEAN", root.join("unavailable-lean"))
        .output()
        .unwrap();
    fails(&wrong_index, "type mismatch: expected `1`, found `2`");

    if std::env::var_os("MESH_VERIFY_LEAN").is_none() {
        return;
    }
    write(root, "main.mpl", main);
    artifacts::ensure_mesh_rt_staticlib();
    let binary = root.join(if cfg!(windows) {
        "evidence.exe"
    } else {
        "evidence"
    });
    let built = on_cli("build", root)
        .arg("--output")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        artifacts::command_output_text(&built)
    );
    let native = Command::new(binary).output().unwrap();
    assert!(
        native.status.success(),
        "{}",
        artifacts::command_output_text(&native)
    );
    assert_eq!(native.stdout, b"42\n");

    let replay = on_cli("verify", root)
        .args(["--verdict", "--json"])
        .output()
        .unwrap();
    assert!(read_json(&replay, true)["laws"]
        .as_array()
        .unwrap()
        .iter()
        .all(|law| law["status"] == "verified"));

    let effectful = main.replace("fn main()", "fn evidence() -> Proof<FitsTwo<2>> do\n  println(\"evaluated\")\n  allowed_two\nend\n\nfn main()")
        .replace("deliver(double, allowed_two)", "deliver(double, evidence())");
    write(root, "main.mpl", &effectful);
    let binary = root.join(if cfg!(windows) {
        "effect.exe"
    } else {
        "effect"
    });
    let built = on_cli("build", root)
        .arg("--output")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        artifacts::command_output_text(&built)
    );
    let native = Command::new(binary).output().unwrap();
    assert!(
        native.status.success(),
        "{}",
        artifacts::command_output_text(&native)
    );
    assert_eq!(native.stdout, b"evaluated\n42\n");

    write(
        root,
        "vector.mpl",
        &vector.replace("Eq.refl(2)", "Eq.refl(1)"),
    );
    let invalid_proof = on_cli("build", root).output().unwrap();
    fails(&invalid_proof, "proof did not check");
}

#[test]
fn applies_a_checked_theorem_to_static_indices_and_native_proof_arguments() {
    let dir = project();
    let root = dir.path();
    let vector = include_str!("fixtures/proofs/evidence_native/vector.mpl").replace(
        "  pub theorem allowed_two()",
        r#"  fn Admitted(size :: Nat) -> Prop do FitsTwo(size) end

  pub theorem admit(size :: Nat, known :: FitsTwo(size)) do Admitted(size) end
  proof admit do exact known end

  pub theorem admit_successor(small :: Nat, known :: FitsTwo(Nat.succ(small))) do
    Admitted(Nat.succ(small))
  end
  proof admit_successor do exact known end

  pub theorem allowed_two()"#,
    );
    write(root, "vector.mpl", &vector);
    let main = r#"import Vector
from Vector import allowed_two, admit

fn admit_index<n :: Nat>(witness :: Proof<FitsTwo<Succ<n>>>) -> Proof<Admitted<Succ<n>>> do
  admit(Succ(n), witness)
end

fn request_evidence() -> Proof<FitsTwo<2>> do
  println("evaluated")
  allowed_two
end

fn request_size() -> Int = 2

fn submit(witness :: Proof<Admitted<2>>) do
  println("admitted")
end

fn main() do
  submit(Vector.admit(1 + 1, request_evidence()))
  submit(admit_index(allowed_two))
  submit(Vector.admit_successor(1, allowed_two))
end
"#;
    for (bad, diagnostic) in [
        (
            main.replace("Vector.admit(1 + 1,", "Vector.admit(1,"),
            "type mismatch: expected `1`, found `2`",
        ),
        (
            main.replace("Vector.admit(1 + 1,", "Vector.admit(request_size(),"),
            "a theorem's Nat argument requires a static Nat expression",
        ),
        (
            main.replace("submit(admit_index(allowed_two))", "let unchecked = admit"),
            "apply this theorem directly",
        ),
    ] {
        write(root, "main.mpl", &bad);
        let output = on_cli("build", root)
            .env("MESH_VERIFY_LEAN", root.join("unavailable-lean"))
            .output()
            .unwrap();
        fails(&output, diagnostic);
    }

    if std::env::var_os("MESH_VERIFY_LEAN").is_none() {
        return;
    }
    write(root, "main.mpl", main);
    artifacts::ensure_mesh_rt_staticlib();
    let binary = root.join(if cfg!(windows) {
        "admission.exe"
    } else {
        "admission"
    });
    let built = on_cli("build", root)
        .arg("--output")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        artifacts::command_output_text(&built)
    );
    let native = Command::new(binary).output().unwrap();
    assert!(
        native.status.success(),
        "{}",
        artifacts::command_output_text(&native)
    );
    assert_eq!(native.stdout, b"evaluated\nadmitted\nadmitted\nadmitted\n");

    let test = "tests/admission.test.mpl";
    write(
        root,
        test,
        r#"from Vector import allowed_two, admit

fn receipt(witness :: Proof<Admitted<2>>) -> Int = 42

test("accepts checked evidence") do
  assert_eq(receipt(admit(2, allowed_two)), 42)
end
"#,
    );
    let tested = on_cli("test", &root.join(test)).output().unwrap();
    assert!(
        tested.status.success(),
        "{}",
        artifacts::command_output_text(&tested)
    );
    assert!(artifacts::command_output_text(&tested).contains("accepts checked evidence"));

    write(
        root,
        "vector.mpl",
        &vector.replace(
            "proof admit do exact known end",
            "proof admit do exact Eq.refl(0) end",
        ),
    );
    let rejected = on_cli("build", root).output().unwrap();
    fails(&rejected, "proof did not check");
    let tested = on_cli("test", &root.join(test)).output().unwrap();
    let output = artifacts::command_output_text(&tested);
    assert!(
        !tested.status.success() && output.contains("proof did not check"),
        "{output}"
    );
}

#[test]
fn applies_a_direct_equality_theorem_from_ordinary_mesh() {
    let dir = project();
    let root = dir.path();
    let receipt = r#"logic do
  pub theorem add_zero(n :: Nat) do n + 0 == n end
  proof add_zero do
    induction n do
      Nat.zero -> refl
      Nat.succ(previous, ih) -> refl
    end
  end

  pub theorem reflect(n :: Nat, evidence :: n == 2) do 2 == n end
  proof reflect do exact Eq.symm(evidence) end
end
"#;
    let main = r#"from Receipt import add_zero, reflect

fn for_two() -> Proof<NatEq<2, 2>> do
  reflect(1 + 1, add_zero(2))
end

fn consume(evidence :: Proof<NatEq<2, 2>>) -> Int = 2
fn main() do
  println("${consume(for_two())}")
end
"#;
    write(root, "receipt.mpl", receipt);
    write(
        root,
        "main.mpl",
        &main.replace("add_zero(2)", "add_zero(3)"),
    );
    let wrong = on_cli("build", root)
        .env("MESH_VERIFY_LEAN", root.join("unavailable-lean"))
        .output()
        .unwrap();
    fails(&wrong, "type mismatch: expected `2`, found `3`");

    if std::env::var_os("MESH_VERIFY_LEAN").is_none() {
        return;
    }
    write(root, "main.mpl", main);
    artifacts::ensure_mesh_rt_staticlib();
    let output = root.join(if cfg!(windows) { "proof.exe" } else { "proof" });
    let built = on_cli("build", root)
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        artifacts::command_output_text(&built)
    );
    let native = Command::new(output).output().unwrap();
    assert!(
        native.status.success(),
        "{}",
        artifacts::command_output_text(&native)
    );
    assert_eq!(native.stdout, b"2\n");

    let replay = on_cli("verify", root)
        .args(["--verdict", "--json"])
        .output()
        .unwrap();
    let report = read_json(&replay, true);
    let laws = report["laws"].as_array().unwrap();
    assert_eq!(laws.len(), 2);
    assert!(laws.iter().all(|law| law["status"] == "verified"));

    write(
        root,
        "receipt.mpl",
        &receipt.replace(
            "Nat.succ(previous, ih) -> refl",
            "Nat.succ(previous, ih) -> exact Eq.refl(0)",
        ),
    );
    let wrong = on_cli("build", root).output().unwrap();
    fails(&wrong, "proof did not check");
}

#[test]
fn verifies_native_generic_trees_at_distinct_types_and_parametrically() {
    if std::env::var_os("MESH_VERIFY_LEAN").is_none() {
        return;
    }
    artifacts::ensure_mesh_rt_staticlib();
    let dir = project();
    let root = dir.path();
    let source = include_str!("fixtures/proofs/native_generics/trees.mpl");
    write(root, "trees.mpl", source);
    write(
        root,
        "main.mpl",
        include_str!("fixtures/proofs/native_generics/main.mpl"),
    );
    write(
        root,
        "laws/concrete.law",
        include_str!("fixtures/proofs/native_generics/concrete.law"),
    );
    let concrete = verify(root, true);
    assert_eq!(concrete["laws"].as_array().unwrap().len(), 3);

    write(
        root,
        "laws/parametric.law",
        include_str!("fixtures/proofs/native_generics/parametric.law"),
    );
    let replay = |success| {
        read_json(
            &on_cli("verify", root)
                .args(["--json", "--verdict"])
                .output()
                .unwrap(),
            success,
        )
    };
    let checked = replay(true);
    let laws = checked["laws"].as_array().unwrap();
    assert_eq!(laws.len(), 4);
    assert!(laws.iter().all(|law| law["status"] == "verified"));

    let native = Command::new(compile(root)).output().unwrap();
    assert!(
        native.status.success(),
        "{}",
        artifacts::command_output_text(&native)
    );
    assert_eq!(native.stdout, b"2 3 4\n");

    write(
        root,
        "trees.mpl",
        &source.replace(
            "Branch(mirror(right), mirror(left))",
            "Branch(mirror(right), mirror(right))",
        ),
    );
    let rejected = replay(false);
    assert!(rejected["laws"]
        .as_array()
        .unwrap()
        .iter()
        .all(|law| law["status"] == "unverified"));
    assert!(rejected["laws"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("proof did not check"));
}

#[test]
fn proves_that_compiled_sort_returns_an_ordered_permutation() {
    if std::env::var_os("MESH_VERIFY_LEAN").is_none() {
        return;
    }
    artifacts::ensure_mesh_rt_staticlib();
    let dir = project();
    let root = dir.path();
    let sort = include_str!("fixtures/proofs/sort/sort.mpl");
    write(root, "sort.mpl", sort);
    write(
        root,
        "main.mpl",
        include_str!("fixtures/proofs/sort/main.mpl"),
    );
    write(
        root,
        "laws/requirements.law",
        include_str!("fixtures/proofs/sort/requirements.law"),
    );
    write(
        root,
        "laws/proofs.law",
        include_str!("fixtures/proofs/sort/proofs.law"),
    );
    let baseline = root.join(".baseline.json");
    let inventory = list(root);
    read_json(&inventory, true);
    fs::write(&baseline, inventory.stdout).unwrap();
    let check = |success| {
        read_json(
            &on_cli("verify", root)
                .args(["--verdict", "--json", "--preserve"])
                .arg(&baseline)
                .output()
                .unwrap(),
            success,
        )
    };
    let report = check(true);
    assert_eq!(report["preservation"]["preserved"], true);
    let laws = report["laws"].as_array().unwrap();
    assert_eq!(laws.len(), 10);
    assert!(laws.iter().all(|law| law["status"] == "verified"));
    assert!(laws
        .iter()
        .any(|law| law["target"]["function"] == "sort_ordered_permutation"));
    let native = Command::new(compile(root)).output().unwrap();
    assert!(
        native.status.success(),
        "{}",
        artifacts::command_output_text(&native)
    );
    assert_eq!(
        native.stdout,
        include_bytes!("fixtures/proofs/sort/native.expected")
    );

    write(
        root,
        "sort.mpl",
        &sort.replace("value <= head", "value >= head"),
    );
    let rejected = check(false);
    assert_eq!(rejected["preservation"]["preserved"], true);
    assert!(rejected["laws"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("proof did not check"));
}

#[test]
fn unverified_cases_report_unsupported_operations_and_solver_failures() {
    let dir = project();
    let root = dir.path();
    write(root, "laws/retry.law", RETRY_LAW);
    for (policy, reason, expression) in [
        ("fn hidden() do DateTime.utc_now() end\npub fn retry(status :: Int, left :: Int) -> Bool do\n  let _ = hidden()\n  left > 0\nend\n", "mesh_datetime_utc_now", "DateTime.utc_now()"),
        ("pub fn retry(status :: Int, left :: Int) -> Bool do retry(status, left) end\n", "recurs", "retry(status, left)"),
        ("pub fn retry(status :: Int, left :: Int) -> Bool do status / left > 0 end\n", "division", "status / left"),
    ] {
        write(root, "policy.mpl", policy);
        let verdict = &verify(root, false)["laws"][0];
        assert_eq!(verdict["status"], "unverified");
        assert!(verdict["reason"].as_str().unwrap().contains(reason), "{verdict}");
        assert_eq!(verdict["declaration"]["path"], "policy.mpl");
        let location = &verdict["unsupported_at"];
        let start = policy.find(expression).unwrap();
        assert_eq!(location["path"], "policy.mpl");
        assert_eq!(location["start"], start);
        assert_eq!(location["end"], start + expression.len());
    }
    write(root, "policy.mpl", POLICY);
    let missing = on_cli("verify", root)
        .arg("--json")
        .env("MESH_VERIFY_Z3", root.join("unavailable-solver"))
        .output()
        .unwrap();
    let missing = read_json(&missing, false);
    assert_eq!(missing["laws"][0]["status"], "unverified");
    assert!(missing["laws"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("Z3"));

    let exhausted = on_cli("verify", root)
        .arg("--json")
        .env("MESH_VERIFY_TIMEOUT_MS", "0")
        .output()
        .unwrap();
    let exhausted = read_json(&exhausted, false);
    assert_eq!(exhausted["laws"][0]["status"], "unverified");
    assert!(exhausted["laws"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("budget"));

    write(root, "laws/retry.law", "from Policy import retry\nlaw retry(status :: Int, left :: Int) -> Bool do retry(status, left) end\n");
    assert_eq!(verify(root, false)["laws"][0]["status"], "unverified");
}

#[test]
fn unverified_ranges_use_the_helper_and_law_source_files() {
    let dir = project();
    let root = dir.path();
    let clock = "# π keeps offsets measured in bytes\npub fn now_check() -> Bool do\n  let _ = DateTime.utc_now()\n  true\nend\n";
    write(root, "timing/clock.mpl", clock);
    write(
        root,
        "policy.mpl",
        "from Timing.Clock import now_check\npub fn retry(status :: Int, left :: Int) -> Bool do now_check() end\n",
    );
    write(root, "laws/retry.law", RETRY_LAW);

    let result = verify(root, false);
    let location = &result["laws"][0]["unsupported_at"];
    assert_eq!(location["path"], "timing/clock.mpl");
    let expression = "DateTime.utc_now()";
    let start = clock.find(expression).unwrap();
    assert_eq!(location["start"], start);
    assert_eq!(location["end"], start + expression.len());
    assert_eq!(location["line"], 3);
    assert_eq!(location["column"], 11);

    write(
        root,
        "laws/retry.law",
        "logic\nfrom Policy import retry\n\ntheorem accepted() do retry(429, 1) == true end\nproof accepted do refl end\n",
    );
    let result = verify(root, false);
    let location = &result["laws"][0]["unsupported_at"];
    assert_eq!(location["path"], "timing/clock.mpl");
    assert_eq!(location["start"], start);
    assert_eq!(location["end"], start + expression.len());

    write(root, "policy.mpl", POLICY);
    let law = "from Policy import retry\n\n# δ is measured in bytes\nlaw retry(status :: Int, left :: Int) -> Bool do\n  left % 2 > 0\nend\n";
    write(root, "laws/retry.law", law);

    let result = verify(root, false);
    let location = &result["laws"][0]["unsupported_at"];
    assert_eq!(location["path"], "laws/retry.law");
    let start = law.find("left % 2").unwrap();
    assert_eq!(location["start"], start);
    assert_eq!(location["end"], start + "left % 2".len());
    assert_eq!(location["line"], 5);
    assert_eq!(location["column"], 3);
    let output = on_cli("verify", root).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("unverified at laws/retry.law:5:3"));
}

#[test]
fn preserve_accepts_moves_and_detects_mutation_deletion_redirection_and_hidden_laws() {
    let dir = project();
    let root = dir.path();
    let snapshot = root.join(".baseline.json");
    write(root, "law/first.law", RETRY_LAW);
    write(root, "rule.mpl", "fn local() -> Bool do true end\n");
    write(root, "alternative.mpl", "fn local() -> Bool do true end\n");
    let second = "from Rule import private local\nlaw local() -> Bool do true end\n";
    write(root, "law/second.law", second);
    let baseline = list(root);
    read_json(&baseline, true);
    fs::write(&snapshot, baseline.stdout).unwrap();

    let preserve = || {
        on_cli("verify", root)
            .args(["--json", "--preserve"])
            .arg(&snapshot)
            .output()
            .unwrap()
    };

    fs::create_dir_all(root.join("moved")).unwrap();
    fs::rename(root.join("law/second.law"), root.join("moved/renamed.law")).unwrap();
    configure(root, &["law", "moved"]);
    let solver_available = pinned_solver_available();
    let proof = read_json(&preserve(), solver_available);
    assert_eq!(proof["preservation"]["preserved"], true);
    if solver_available {
        assert!(proof["laws"]
            .as_array()
            .unwrap()
            .iter()
            .all(|law| law["status"] == "verified"));
    }

    write(
        root,
        "moved/renamed.law",
        &second.replace("do true", "do false"),
    );
    let changed = read_json(&preserve(), false);
    let change = &changed["preservation"]["changes"][0];
    assert_eq!(change["change"], "changed");
    assert_eq!(change["old_location"]["path"], "law/second.law");
    assert_eq!(change["new_location"]["path"], "moved/renamed.law");

    write(
        root,
        "moved/renamed.law",
        &second.replace("from Rule", "from Alternative"),
    );
    let redirected = read_json(&preserve(), false);
    assert_eq!(
        redirected["preservation"]["changes"][0]["change"],
        "missing"
    );
    assert_eq!(
        redirected["preservation"]["changes"][0]["target"]["module"],
        "Rule"
    );

    write(root, "moved/renamed.law", second);
    configure(root, &["law"]);
    assert_eq!(
        read_json(&preserve(), false)["preservation"]["changes"][0]["change"],
        "missing"
    );
    configure(root, &["law", "moved"]);
    fs::remove_file(root.join("moved/renamed.law")).unwrap();
    assert_eq!(
        read_json(&preserve(), false)["preservation"]["changes"][0]["change"],
        "missing"
    );
    fs::remove_file(root.join("law/first.law")).unwrap();
    assert_eq!(
        read_json(&preserve(), false)["preservation"]["changes"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn authorized_429_requirement_change_fails_old_policy_then_verifies_the_update() {
    if !pinned_solver_available() {
        return;
    }
    artifacts::ensure_mesh_rt_staticlib();
    let dir = project();
    let root = dir.path();
    let old_policy = POLICY.replace("(status == 429 || status == 503)", "status == 503");
    write(root, "policy.mpl", &old_policy);
    write(
        root,
        "retry.law",
        &RETRY_LAW.replace("    429 -> left > 0\n", ""),
    );
    assert_eq!(verify(root, true)["laws"][0]["status"], "verified");

    write(root, "retry.law", RETRY_LAW);
    let failed = verify(root, false);
    assert_eq!(failed["laws"][0]["status"], "falsified");
    assert_eq!(failed["laws"][0]["inputs"]["status"], 429);

    write(root, "policy.mpl", POLICY);
    write(root, "tests/budget.test.mpl", BUDGET_TEST);
    assert_eq!(verify(root, true)["laws"][0]["status"], "verified");
    budget_passes(root);
}

/// The diagnostics `meshc` wrote to stderr, one JSON object per line.
fn diagnostics(out: &Output) -> Vec<Value> {
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .map(|line| {
            serde_json::from_str(line)
                .unwrap_or_else(|error| panic!("{error}: {}", artifacts::command_output_text(out)))
        })
        .collect()
}

/// `(line, col, label)` of each span of `diagnostic`.
fn spans(diagnostic: &Value) -> Vec<(u64, u64, String)> {
    diagnostic["spans"]
        .as_array()
        .unwrap()
        .iter()
        .map(|span| {
            (
                span["line"].as_u64().unwrap(),
                span["col"].as_u64().unwrap(),
                span["label"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn law_and_logic_files_report_syntax_errors_with_their_codes_and_places() {
    let dir = project();
    let root = dir.path();
    for (source, code, at) in [
        (
            "from Policy import retry\nlaw retry(status :: Int, left :: Int) -> Bool do\n  true\n",
            "P0201",
            vec![
                (3, 7, "expected `end` here".to_owned()),
                (2, 47, "block opened here".to_owned()),
            ],
        ),
        (
            "logic\ntheorem two() do 2 = 2 end\n",
            "P0301",
            vec![
                (2, 20, "expected `end` here".to_owned()),
                (2, 15, "block opened here".to_owned()),
            ],
        ),
    ] {
        write(root, "laws/retry.law", source);
        let out = list(root);
        assert!(!out.status.success());
        let [diagnostic] = &diagnostics(&out)[..] else {
            panic!("{}", artifacts::command_output_text(&out));
        };
        assert_eq!(diagnostic["code"], code, "{diagnostic}");
        assert!(
            diagnostic["file"]
                .as_str()
                .unwrap()
                .ends_with("laws/retry.law"),
            "{diagnostic}"
        );
        assert_eq!(spans(diagnostic), at, "{diagnostic}");
    }
}

#[test]
fn proof_and_logic_block_errors_have_their_own_codes_in_meshc_check() {
    let dir = project();
    let root = dir.path();
    let meshc = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_meshc"))
            .args(args)
            .current_dir(root)
            .output()
            .unwrap()
    };
    let catalog = meshc(&["--json", "explain"]);
    let catalog: Value = serde_json::from_slice(&catalog.stdout).unwrap();
    for code in [
        "P0301", "E0095", "E0096", "E0097", "E0098", "E0099", "E0100",
    ] {
        let entry = catalog["codes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["code"] == code)
            .unwrap();
        write(
            root,
            "example.mpl",
            entry["example"]["bad"].as_str().unwrap(),
        );
        let bad = meshc(&["check", "example.mpl", "--json"]);
        let found: Vec<_> = diagnostics(&bad)
            .iter()
            .map(|d| d["code"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(found, [code], "{}", artifacts::command_output_text(&bad));
        assert_eq!(bad.status.code(), Some(1));
        write(
            root,
            "example.mpl",
            entry["example"]["fixed"].as_str().unwrap(),
        );
        let fixed = meshc(&["check", "example.mpl", "--json"]);
        assert!(
            fixed.status.success() && fixed.stderr.is_empty(),
            "{code}: {}",
            artifacts::command_output_text(&fixed)
        );
    }

    write(
        root,
        "tests/order.test.mpl",
        "describe(\"order\") do\n  test(\"first\") do\n    assert(true)\n  end\n  setup do\n  end\nend\n",
    );
    let out = meshc(&["check", "tests/order.test.mpl", "--tests", "--json"]);
    let [diagnostic] = &diagnostics(&out)[..] else {
        panic!("{}", artifacts::command_output_text(&out));
    };
    assert_eq!(diagnostic["code"], "P0401", "{diagnostic}");
    assert!(
        diagnostic["file"]
            .as_str()
            .unwrap()
            .ends_with("tests/order.test.mpl"),
        "{diagnostic}"
    );
    assert_eq!(
        spans(diagnostic),
        [
            (5, 3, "`setup` after a test".to_owned()),
            (2, 3, "the describe's first test".to_owned()),
        ],
        "{diagnostic}"
    );
}
