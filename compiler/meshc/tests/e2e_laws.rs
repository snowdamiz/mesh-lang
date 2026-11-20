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
    let start = clock.find("DateTime.utc_now()").unwrap();
    assert_eq!(location["start"], start);
    assert_eq!(location["end"], start + "DateTime.utc_now()".len());
    assert_eq!(location["line"], 3);
    assert_eq!(location["column"], 11);

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
