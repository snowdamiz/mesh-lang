//! End-to-end integration tests for the Snow compiler.
//!
//! Each test writes a `.snow` source file, invokes the full compilation pipeline,
//! runs the resulting binary, and asserts the expected stdout output.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Helper: compile a Snow source file and run the resulting binary, returning stdout.
fn compile_and_run(source: &str) -> String {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_dir = temp_dir.path().join("project");
    std::fs::create_dir_all(&project_dir).expect("failed to create project dir");

    // Write the source file
    let main_snow = project_dir.join("main.snow");
    std::fs::write(&main_snow, source).expect("failed to write main.snow");

    // Build with snowc
    let snowc = find_snowc();
    let output = Command::new(&snowc)
        .args(["build", project_dir.to_str().unwrap()])
        .output()
        .expect("failed to invoke snowc");

    assert!(
        output.status.success(),
        "snowc build failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // Run the compiled binary
    let binary = project_dir.join("project");
    let run_output = Command::new(&binary)
        .output()
        .unwrap_or_else(|e| panic!("failed to run binary at {}: {}", binary.display(), e));

    assert!(
        run_output.status.success(),
        "binary execution failed with exit code {:?}:\nstdout: {}\nstderr: {}",
        run_output.status.code(),
        String::from_utf8_lossy(&run_output.stdout),
        String::from_utf8_lossy(&run_output.stderr)
    );

    String::from_utf8_lossy(&run_output.stdout).to_string()
}

/// Helper: compile a Snow source file, return the compilation error.
fn compile_expect_error(source: &str) -> String {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_dir = temp_dir.path().join("project");
    std::fs::create_dir_all(&project_dir).expect("failed to create project dir");

    let main_snow = project_dir.join("main.snow");
    std::fs::write(&main_snow, source).expect("failed to write main.snow");

    let snowc = find_snowc();
    let output = Command::new(&snowc)
        .args(["build", project_dir.to_str().unwrap()])
        .output()
        .expect("failed to invoke snowc");

    assert!(
        !output.status.success(),
        "expected compilation to fail but it succeeded"
    );

    String::from_utf8_lossy(&output.stderr).to_string()
}

/// Find the snowc binary in the target directory.
fn find_snowc() -> PathBuf {
    let mut path = std::env::current_exe()
        .expect("cannot find current exe")
        .parent()
        .expect("cannot find parent dir")
        .to_path_buf();

    // Navigate from `deps/` to the target directory
    if path.file_name().map_or(false, |n| n == "deps") {
        path = path.parent().unwrap().to_path_buf();
    }

    let snowc = path.join("snowc");
    assert!(
        snowc.exists(),
        "snowc binary not found at {}. Run `cargo build -p snowc` first.",
        snowc.display()
    );
    snowc
}

/// Read a test fixture from the tests/e2e/ directory.
fn read_fixture(name: &str) -> String {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let fixture_path = Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests")
        .join("e2e")
        .join(name);
    std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {}", fixture_path.display(), e))
}

// ── E2E Tests ────────────────────────────────────────────────────────────

/// SC1: Hello World program compiles and runs.
#[test]
fn e2e_hello_world() {
    let source = read_fixture("hello.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "Hello, World!\n");
}

/// SC2: Functions with integer arithmetic.
#[test]
fn e2e_functions() {
    let source = read_fixture("functions.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "7\n10\n");
}

/// SC2: Integer pattern matching in case expressions.
#[test]
fn e2e_pattern_match() {
    let source = read_fixture("pattern_match.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "zero\none\nother\n");
}

/// SC2: Closures with captured variables.
#[test]
fn e2e_closures() {
    let source = read_fixture("closures.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "8\n15\n");
}

/// SC2: Pipe operator chaining.
#[test]
fn e2e_pipe() {
    let source = read_fixture("pipe.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "11\n");
}

/// SC2: String interpolation with variables.
#[test]
fn e2e_string_interp() {
    let source = read_fixture("string_interp.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "Hello, World!\nThe answer is 42\n");
}

/// SC2: ADT sum type construction.
#[test]
fn e2e_adts() {
    let source = read_fixture("adts.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "red created\ngreen created\nblue created\n");
}

/// SC2/SC5: Comprehensive multi-feature test (100+ lines).
#[test]
fn e2e_comprehensive() {
    let source = read_fixture("comprehensive.snow");
    let output = compile_and_run(&source);
    let expected = "\
30
14
-5
6
zero
one
other
red
green
blue
21
30
20
5
Hello, Snow!
The answer is 42
4
logic works
";
    assert_eq!(output, expected);
}

/// SC4: --emit-llvm flag produces .ll file.
#[test]
fn e2e_emit_llvm() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_dir = temp_dir.path().join("project");
    std::fs::create_dir_all(&project_dir).expect("failed to create project dir");

    let source = read_fixture("hello.snow");
    let main_snow = project_dir.join("main.snow");
    std::fs::write(&main_snow, &source).expect("failed to write main.snow");

    let snowc = find_snowc();
    let output = Command::new(&snowc)
        .args(["build", project_dir.to_str().unwrap(), "--emit-llvm"])
        .output()
        .expect("failed to invoke snowc");

    assert!(
        output.status.success(),
        "snowc build --emit-llvm failed:\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Check that .ll file was created
    let ll_file = project_dir.join("project.ll");
    assert!(
        ll_file.exists(),
        "Expected LLVM IR file at {}",
        ll_file.display()
    );

    let ir_content = std::fs::read_to_string(&ll_file).unwrap();
    assert!(
        ir_content.contains("define"),
        "LLVM IR should contain function definitions"
    );
    assert!(
        ir_content.contains("snow_println"),
        "LLVM IR should reference snow_println"
    );
}

/// SC4: --target flag accepts a triple.
#[test]
fn e2e_target_flag() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_dir = temp_dir.path().join("project");
    std::fs::create_dir_all(&project_dir).expect("failed to create project dir");

    let source = read_fixture("hello.snow");
    let main_snow = project_dir.join("main.snow");
    std::fs::write(&main_snow, &source).expect("failed to write main.snow");

    // Use host triple
    let triple = if cfg!(target_arch = "aarch64") {
        "aarch64-apple-darwin"
    } else {
        "x86_64-unknown-linux-gnu"
    };

    let snowc = find_snowc();
    let output = Command::new(&snowc)
        .args([
            "build",
            project_dir.to_str().unwrap(),
            "--target",
            triple,
        ])
        .output()
        .expect("failed to invoke snowc");

    assert!(
        output.status.success(),
        "snowc build --target {} failed:\nstderr: {}",
        triple,
        String::from_utf8_lossy(&output.stderr)
    );

    // Run the binary (should work since it's the host triple)
    let binary = project_dir.join("project");
    let run_output = Command::new(&binary).output().expect("failed to run binary");
    assert!(run_output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&run_output.stdout),
        "Hello, World!\n"
    );
}

/// SC5: Both -O0 and -O2 optimization levels work.
#[test]
fn e2e_optimization_levels() {
    for opt_level in &["0", "2"] {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let project_dir = temp_dir.path().join("project");
        std::fs::create_dir_all(&project_dir).expect("failed to create project dir");

        let source = read_fixture("hello.snow");
        let main_snow = project_dir.join("main.snow");
        std::fs::write(&main_snow, &source).expect("failed to write main.snow");

        let snowc = find_snowc();
        let output = Command::new(&snowc)
            .args([
                "build",
                project_dir.to_str().unwrap(),
                "--opt-level",
                opt_level,
            ])
            .output()
            .expect("failed to invoke snowc");

        assert!(
            output.status.success(),
            "snowc build --opt-level={} failed:\nstderr: {}",
            opt_level,
            String::from_utf8_lossy(&output.stderr)
        );

        let binary = project_dir.join("project");
        let run_output = Command::new(&binary).output().expect("failed to run binary");
        assert!(
            run_output.status.success(),
            "Binary compiled with -O{} failed to run",
            opt_level
        );
        assert_eq!(
            String::from_utf8_lossy(&run_output.stdout),
            "Hello, World!\n"
        );
    }
}

/// SC3: Binary is self-contained (no dynamic snow_rt dependency).
#[test]
fn e2e_self_contained_binary() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_dir = temp_dir.path().join("project");
    std::fs::create_dir_all(&project_dir).expect("failed to create project dir");

    let source = read_fixture("hello.snow");
    let main_snow = project_dir.join("main.snow");
    std::fs::write(&main_snow, &source).expect("failed to write main.snow");

    let snowc = find_snowc();
    let output = Command::new(&snowc)
        .args(["build", project_dir.to_str().unwrap()])
        .output()
        .expect("failed to invoke snowc");
    assert!(output.status.success());

    let binary = project_dir.join("project");

    // Check that the binary doesn't have a dynamic dependency on snow_rt
    // On macOS, use `otool -L`; on Linux, use `ldd`
    if cfg!(target_os = "macos") {
        let otool_output = Command::new("otool")
            .args(["-L", binary.to_str().unwrap()])
            .output()
            .expect("failed to run otool");
        let deps = String::from_utf8_lossy(&otool_output.stdout);
        assert!(
            !deps.contains("snow_rt"),
            "Binary should not dynamically link snow_rt. Dependencies:\n{}",
            deps
        );
    } else {
        let ldd_output = Command::new("ldd")
            .arg(binary.to_str().unwrap())
            .output();
        if let Ok(out) = ldd_output {
            let deps = String::from_utf8_lossy(&out.stdout);
            assert!(
                !deps.contains("snow_rt"),
                "Binary should not dynamically link snow_rt. Dependencies:\n{}",
                deps
            );
        }
    }
}

/// SC5: 100-line program compiles in under 5 seconds at -O0.
#[test]
fn e2e_performance() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_dir = temp_dir.path().join("project");
    std::fs::create_dir_all(&project_dir).expect("failed to create project dir");

    let source = read_fixture("comprehensive.snow");
    let main_snow = project_dir.join("main.snow");
    std::fs::write(&main_snow, &source).expect("failed to write main.snow");

    let snowc = find_snowc();

    let start = std::time::Instant::now();
    let output = Command::new(&snowc)
        .args(["build", project_dir.to_str().unwrap(), "--opt-level", "0"])
        .output()
        .expect("failed to invoke snowc");
    let elapsed = start.elapsed();

    assert!(
        output.status.success(),
        "Compilation failed:\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        elapsed.as_secs() < 5,
        "Compilation took {:?} which exceeds 5 second limit",
        elapsed
    );
}

// ── Multi-Clause Function E2E Tests (Phase 11) ─────────────────────────

/// Multi-clause functions with literal patterns, recursion, and = expr body form.
#[test]
fn e2e_multi_clause_functions() {
    let source = read_fixture("multi_clause.snow");
    let output = compile_and_run(&source);
    assert_eq!(
        output,
        "55\nyes\nno\n42\n36\n",
        "Expected: fib(10)=55, to_string(true)=yes, to_string(false)=no, double(21)=42, square(6)=36"
    );
}

/// Multi-clause functions with guard clauses (when keyword).
#[test]
fn e2e_multi_clause_guards() {
    let source = read_fixture("multi_clause_guards.snow");
    let output = compile_and_run(&source);
    assert_eq!(
        output,
        "5\n3\npositive\nnegative\nzero\n",
        "Expected: abs(-5)=5, abs(3)=3, classify(10)=positive, classify(-3)=negative, classify(0)=zero"
    );
}

/// Multi-clause function error: catch-all not last should produce compilation error.
#[test]
fn e2e_multi_clause_catch_all_not_last() {
    let source = r#"
fn foo(n) = n
fn foo(0) = 0
fn main() do
  println("${foo(1)}")
end
"#;
    let error = compile_expect_error(source);
    assert!(
        error.contains("catch-all") || error.contains("CatchAll") || error.contains("E0022"),
        "Expected catch-all-not-last error, got: {}",
        error
    );
}

/// Multi-clause function error: return type mismatch across clauses.
#[test]
fn e2e_multi_clause_type_mismatch() {
    let source = r#"
fn bar(0) = 0
fn bar(n) = "hello"
fn main() do
  println("${bar(1)}")
end
"#;
    let error = compile_expect_error(source);
    assert!(
        error.contains("expected") || error.contains("mismatch") || error.contains("Int"),
        "Expected type mismatch error, got: {}",
        error
    );
}

// ── Phase 12 Closure Syntax E2E Tests ───────────────────────────────────

/// Bare param closures in pipe chains: the primary Phase 12 use case.
#[test]
fn e2e_closure_bare_params_pipe() {
    let source = read_fixture("closure_bare_params_pipe.snow");
    let output = compile_and_run(&source);
    assert_eq!(
        output, "24\n",
        "Expected: doubled [2,4,6,8,10], filter >4 -> [6,8,10], sum = 24"
    );
}

/// Multi-clause closures with literal pattern matching.
#[test]
fn e2e_closure_multi_clause() {
    let source = read_fixture("closure_multi_clause.snow");
    let output = compile_and_run(&source);
    assert_eq!(
        output, "3\n",
        "Expected: 0->0, 1->1, 2->1, 3->1, sum of classified = 3"
    );
}

/// Do/end body closures with multi-statement bodies.
#[test]
fn e2e_closure_do_end_body() {
    let source = read_fixture("closure_do_end_body.snow");
    let output = compile_and_run(&source);
    assert_eq!(
        output, "15\n",
        "Expected: (1*2+1) + (2*2+1) + (3*2+1) = 3+5+7 = 15"
    );
}

/// Chained pipes with closures: Phase 12 gap closure verification.
/// list |> map(fn x -> x + 1 end) |> filter(fn x -> x > 3 end) |> reduce(0, fn acc, x -> acc + x end)
#[test]
fn e2e_pipe_chain_closures() {
    let source = read_fixture("pipe_chain_closures.snow");
    let output = compile_and_run(&source);
    assert_eq!(
        output, "15\n",
        "Expected: [1,2,3,4,5] -> map +1 [2,3,4,5,6] -> filter >3 [4,5,6] -> sum 15"
    );
}

// ── Phase 13: String Pattern Matching ────────────────────────────────

/// String pattern matching in case expressions with wildcard default.
#[test]
fn e2e_string_pattern_matching() {
    let source = r#"
fn describe(name :: String) -> String do
  case name do
    "alice" -> "found alice"
    "bob" -> "found bob"
    _ -> "unknown"
  end
end

fn main() do
  println(describe("alice"))
  println(describe("bob"))
  println(describe("charlie"))
end
"#;
    let output = compile_and_run(source);
    assert_eq!(output, "found alice\nfound bob\nunknown\n");
}

/// String binary == and != comparison.
#[test]
fn e2e_string_equality_comparison() {
    let source = r#"
fn main() do
  let x = "hello"
  if x == "hello" do
    println("equal")
  else
    println("not equal")
  end
  if x != "world" do
    println("different")
  else
    println("same")
  end
end
"#;
    let output = compile_and_run(source);
    assert_eq!(output, "equal\ndifferent\n");
}

/// String patterns mixed with variable bindings in the same case expression.
#[test]
fn e2e_string_pattern_mixed_with_variable() {
    let source = r#"
fn greet(name :: String) -> String do
  case name do
    "world" -> "Hello, world!"
    other -> "Hi, " <> other <> "!"
  end
end

fn main() do
  println(greet("world"))
  println(greet("Snow"))
end
"#;
    let output = compile_and_run(source);
    assert_eq!(output, "Hello, world!\nHi, Snow!\n");
}

// ── Phase 22: Deriving Clause ─────────────────────────────────────────

/// Struct with all five derivable protocols: Eq, Ord, Display, Debug, Hash.
/// Display produces positional "Point(1, 2)" format.
#[test]
fn e2e_deriving_struct() {
    let source = read_fixture("deriving_struct.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "Point(1, 2)\ntrue\nfalse\n");
}

/// Sum type with deriving: variant-aware Display and Eq (nullary variants).
/// Note: sum type Constructor pattern field bindings have a pre-existing LLVM
/// codegen limitation for non-nullary variants; tested with nullary only here.
#[test]
fn e2e_deriving_sum_type() {
    let source = read_fixture("deriving_sum_type.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "Red\nGreen\nBlue\ntrue\nfalse\n");
}

/// Backward compatibility: no deriving clause = derive all defaults.
#[test]
fn e2e_deriving_backward_compat() {
    let source = read_fixture("deriving_backward_compat.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "true\n");
}

/// Selective deriving: only Eq, no other protocols.
#[test]
fn e2e_deriving_selective() {
    let source = read_fixture("deriving_selective.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "true\n");
}

/// Empty deriving clause: opt-out of all auto-derived protocols.
#[test]
fn e2e_deriving_empty() {
    let source = read_fixture("deriving_empty.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "42\n");
}

/// Unsupported trait in deriving clause produces a clear compiler error.
#[test]
fn e2e_deriving_unsupported_trait() {
    let source = r#"
struct Foo do
  x :: Int
end deriving(Clone)

fn main() do
  let f = Foo { x: 1 }
  println("nope")
end
"#;
    let error = compile_expect_error(source);
    assert!(
        error.contains("cannot derive"),
        "Expected 'cannot derive' error, got: {}",
        error
    );
}

// ── Phase 16: Fun() Type Annotations ─────────────────────────────────

/// Fun() type annotations: parsing, positions, and unification with closures.
#[test]
fn e2e_fun_type_annotations() {
    let source = read_fixture("fun_type.snow");
    let output = compile_and_run(&source);
    assert_eq!(
        output, "42\n99\n30\n",
        "Expected: apply(int_to_str, 42)='42', run_thunk(->99)=99, apply2(add, 10, 20)=30"
    );
}

// ── Phase 23: Pattern Matching Codegen & Ordering ─────────────────────

/// Option field extraction: Some(42) pattern match extracts the inner value.
#[test]
fn e2e_option_field_extraction() {
    let source = read_fixture("option_field_extraction.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "42\n");
}

/// Ordering pattern match: compare(3, 5) returns Less, matched to 1.
#[test]
fn e2e_ordering_pattern_match() {
    let source = read_fixture("ordering_pattern_match.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "1\n");
}

/// Ordering as variable: compare result stored in variable, then matched.
#[test]
fn e2e_ordering_as_variable() {
    let source = read_fixture("ordering_as_variable.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "2\n");
}

/// Nullary constructor pattern match: user-defined sum type with all-nullary variants.
/// Validates that Red/Green/Blue are recognized as constructors, not variables.
#[test]
fn e2e_nullary_constructor_match() {
    let source = read_fixture("nullary_constructor_match.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "1\n2\n3\n");
}

// -- Phase 24: Trait System Generics ────────────────────────────────────

/// Flat collection Display regression check: List<Int> renders via string interpolation.
/// Verifies that the &self -> &mut self signature change does not break existing
/// Display callback resolution for flat collections.
#[test]
fn e2e_nested_collection_display() {
    let source = read_fixture("nested_collection_display.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "[10, 20, 30]\n", "List Display via string interpolation should render as [10, 20, 30]");
    // NOTE: List<List<Int>> e2e test requires generic List element types
    // (List.append currently typed as (List, Int) -> List).
    // Recursive callback resolution is verified at the MIR unit test level
    // in snow-codegen (nested_list_callback_generates_wrapper).
    // TODO: add full nested e2e test after Plan 02 (generic collection elements).
}
