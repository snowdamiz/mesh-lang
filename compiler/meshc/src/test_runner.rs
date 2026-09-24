//! Test runner for Mesh: discovers *.test.mpl files, compiles and executes each,
//! aggregates pass/fail results, and formats output with ANSI colors.
//!
//! Test files (*.test.mpl) use the Mesh test DSL:
//!
//! ```mesh
//! test("label") do
//!   assert(expr)
//!   assert_eq(lhs_str, rhs_str)
//! end
//!
//! describe("group") do
//!   setup() do ... end
//!   teardown() do ... end
//!   test("name") do ... end
//! end
//! ```
//!
//! The test runner preprocesses this into a valid Mesh program with `fn main()`.
//! The preprocessed program uses the test runtime builtins registered in
//! `mesh_typeck::builtins` and `mesh_codegen::mir::lower`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use mesh_pkg::manifest::{
    resolve_entrypoint, rewrite_test_manifest_source, Manifest, DEFAULT_ENTRYPOINT,
};
use std::ops::Range;

use mesh_parser::{SyntaxKind, SyntaxNode};
use mesh_typeck::diagnostics::DiagnosticOptions;

/// Whether output goes to a color terminal (and `NO_COLOR` is unset). The
/// test binaries learn it through `MESH_TEST_COLOR`: their stdout is a pipe.
fn use_color() -> bool {
    use std::io::IsTerminal;
    std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal()
}

/// (green, red, bold, reset), empty without color.
fn palette() -> (&'static str, &'static str, &'static str, &'static str) {
    if use_color() {
        ("\x1b[32m", "\x1b[31m", "\x1b[1m", "\x1b[0m")
    } else {
        ("", "", "", "")
    }
}

/// Summary of a test run.
#[allow(dead_code)]
pub struct TestSummary {
    /// Number of test files that passed (exit code 0).
    pub passed: usize,
    /// Number of test files that failed (compile error or exit code non-zero).
    pub failed: usize,
}

fn resolve_target_path(target: &Path) -> Result<PathBuf, String> {
    let abs = if target.is_absolute() {
        target.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| format!("Failed to read current directory: {}", e))?
            .join(target)
    };

    if abs.exists() {
        Ok(abs)
    } else {
        Err(format!("Test target '{}' does not exist", abs.display()))
    }
}

struct ResolvedTestProject {
    project_dir: PathBuf,
    manifest_source: String,
    entry_relative_path: PathBuf,
}

fn find_project_dir_for_target(target: &Path) -> Option<PathBuf> {
    let mut dir = if target.is_dir() {
        target.to_path_buf()
    } else {
        target.parent()?.to_path_buf()
    };
    loop {
        if dir.join("mesh.toml").is_file() {
            return Some(dir);
        }
        match dir.parent() {
            Some(parent) => dir = parent.to_path_buf(),
            None => return None,
        }
    }
}

fn project_root_resolution_error(target: &Path) -> String {
    format!(
        "Could not resolve a Mesh project root for test target '{}'; expected an ancestor with 'mesh.toml'.",
        target.display()
    )
}

fn resolve_project_dir(target: Option<&Path>) -> Result<PathBuf, String> {
    let cwd =
        std::env::current_dir().map_err(|e| format!("Failed to read current directory: {}", e))?;

    match target {
        Some(target) => {
            let abs = resolve_target_path(target)?;
            find_project_dir_for_target(&abs).ok_or_else(|| project_root_resolution_error(&abs))
        }
        None => {
            find_project_dir_for_target(&cwd).ok_or_else(|| project_root_resolution_error(&cwd))
        }
    }
}

fn resolve_test_project(target: Option<&Path>) -> Result<ResolvedTestProject, String> {
    let project_dir = resolve_project_dir(target)?;
    let manifest_path = project_dir.join("mesh.toml");
    let manifest_source = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("Failed to read '{}': {}", manifest_path.display(), e))?;
    let manifest = Manifest::from_file(&manifest_path)?;
    let entry_relative_path = if manifest.package.entrypoint.is_none()
        && !project_dir.join(DEFAULT_ENTRYPOINT).exists()
    {
        PathBuf::from(DEFAULT_ENTRYPOINT)
    } else {
        resolve_entrypoint(&project_dir, Some(&manifest))?
    };

    Ok(ResolvedTestProject {
        project_dir,
        manifest_source,
        entry_relative_path,
    })
}

fn resolve_test_files(target: Option<&Path>) -> Result<Vec<PathBuf>, String> {
    match target {
        Some(target) => {
            let abs = resolve_target_path(target)?;
            if abs.is_dir() {
                discover_test_files(&abs)
            } else if abs
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.ends_with(".test.mpl"))
                .unwrap_or(false)
            {
                Ok(vec![abs])
            } else {
                Err(format!(
                    "'{}' is not a directory or a *.test.mpl file",
                    abs.display()
                ))
            }
        }
        None => {
            let cwd = std::env::current_dir()
                .map_err(|e| format!("Failed to read current directory: {}", e))?;
            discover_test_files(&cwd)
        }
    }
}

fn synthetic_test_manifest_source(test_project: &ResolvedTestProject) -> Result<String, String> {
    rewrite_test_manifest_source(
        &test_project.manifest_source,
        Path::new(DEFAULT_ENTRYPOINT),
        &test_project.project_dir,
    )
}

fn prepare_temp_test_project(
    test_project: &ResolvedTestProject,
    tmp_dir: &Path,
    preprocessed_source: &str,
) -> Result<(), String> {
    copy_project_sources_to_tmp(
        &test_project.project_dir,
        tmp_dir,
        &test_project.entry_relative_path,
    )?;

    let copied_entry_path = tmp_dir.join(&test_project.entry_relative_path);
    if copied_entry_path.exists() {
        return Err(format!(
            "Synthetic test project unexpectedly retained executable entry '{}' from '{}'; aborting to avoid copied-entry contamination.",
            test_project.entry_relative_path.display(),
            test_project.project_dir.display()
        ));
    }

    let manifest_source = synthetic_test_manifest_source(test_project).map_err(|e| {
        format!(
            "Invalid synthetic test manifest state for '{}': {}",
            test_project.project_dir.display(),
            e
        )
    })?;
    let manifest_path = tmp_dir.join("mesh.toml");
    std::fs::write(&manifest_path, manifest_source)
        .map_err(|e| format!("Failed to write '{}': {}", manifest_path.display(), e))?;

    let main_path = tmp_dir.join(DEFAULT_ENTRYPOINT);
    std::fs::write(&main_path, preprocessed_source)
        .map_err(|e| format!("Failed to write preprocessed source: {}", e))?;

    let manifest = Manifest::from_file(&manifest_path).map_err(|e| {
        format!(
            "Invalid synthetic test manifest state for '{}': {}",
            test_project.project_dir.display(),
            e
        )
    })?;
    let synthetic_entry = resolve_entrypoint(tmp_dir, Some(&manifest)).map_err(|e| {
        format!(
            "Invalid synthetic test manifest state for '{}': {}",
            test_project.project_dir.display(),
            e
        )
    })?;
    if synthetic_entry != PathBuf::from(DEFAULT_ENTRYPOINT) {
        return Err(format!(
            "Invalid synthetic test manifest state for '{}': resolved '{}' instead of '{}'.",
            test_project.project_dir.display(),
            synthetic_entry.display(),
            DEFAULT_ENTRYPOINT
        ));
    }

    Ok(())
}

/// Run tests from the current project, a project root, a test directory, or a specific test file.
///
/// - `target`: optional project root, directory, or specific `*.test.mpl` file.
/// - `quiet`: compact output (a dot per passing test instead of its name).
/// - `coverage`: currently unsupported and returns an explicit error.
pub fn run_tests(
    target: Option<&Path>,
    quiet: bool,
    coverage: bool,
) -> Result<TestSummary, String> {
    if coverage {
        return Err(
            "coverage reporting is not implemented for `meshc test`; run the command without --coverage"
                .to_string(),
        );
    }

    let test_project = resolve_test_project(target)?;
    let project_dir = &test_project.project_dir;
    let test_files = resolve_test_files(target)?;

    if test_files.is_empty() {
        println!("No *.test.mpl files found.");
        return Ok(TestSummary {
            passed: 0,
            failed: 0,
        });
    }

    let start = Instant::now();
    let mut passed = 0usize;
    let mut failed = 0usize;
    let (green, red, bold, reset) = palette();

    for test_file in &test_files {
        let rel = test_file.strip_prefix(project_dir).map_err(|_| {
            format!(
                "Resolved test file '{}' is not under project root '{}'; aborting to avoid a wrong-root test run.",
                test_file.display(),
                project_dir.display()
            )
        })?;
        let label = rel.display().to_string();

        // Read the .test.mpl source and preprocess it into a valid Mesh program.
        let source = std::fs::read_to_string(test_file)
            .map_err(|e| format!("Failed to read '{}': {}", test_file.display(), e))?;

        let preprocessed = match preprocess_test_source(&source) {
            Ok(preprocessed) => preprocessed,
            Err(e) => {
                println!("{red}{bold}COMPILE ERROR{reset}: {label}");
                println!("  {e}");
                failed += 1;
                continue;
            }
        };

        // Compile the preprocessed source to a temp binary.
        let tmp_dir =
            tempfile::tempdir().map_err(|e| format!("Failed to create temp dir: {}", e))?;
        let bin_path = tmp_dir.path().join("test_bin");

        if let Err(e) = prepare_temp_test_project(&test_project, tmp_dir.path(), &preprocessed) {
            println!("{red}{bold}SETUP ERROR{reset}: {label}");
            println!("  {}", e);
            failed += 1;
            continue;
        }

        // Diagnostics name the test file and the project's files, not
        // their copies in the temporary project.
        let diag_opts = DiagnosticOptions {
            color: use_color(),
            json: false,
            display_paths: vec![
                (tmp_dir.path().join(DEFAULT_ENTRYPOINT), test_file.clone()),
                (tmp_dir.path().to_path_buf(), project_dir.to_path_buf()),
            ],
        };
        let compile_result = crate::build(
            tmp_dir.path(),
            0,     // opt_level: debug
            false, // emit_llvm
            Some(&bin_path),
            None, // target: native
            crate::BuildArtifact::Executable,
            true, // test-only compiler builtins
            &diag_opts,
        );

        if let Err(e) = compile_result {
            println!("{red}{bold}COMPILE ERROR{reset}: {label}");
            println!("  {}", e);
            failed += 1;
            continue;
        }

        // Execute the compiled binary. It prints a line per test, or a `.`
        // or `F` in quiet mode.
        let output = Command::new(&bin_path)
            .env("MESH_TEST_QUIET", if quiet { "1" } else { "0" })
            .env("MESH_TEST_COLOR", if use_color() { "1" } else { "0" })
            .output()
            .map_err(|e| format!("Failed to execute '{}': {}", bin_path.display(), e))?;

        // Pass stdout/stderr through to terminal
        if !output.stdout.is_empty() {
            print!("{}", String::from_utf8_lossy(&output.stdout));
        }
        if !output.stderr.is_empty() {
            eprint!("{}", String::from_utf8_lossy(&output.stderr));
        }

        if output.status.success() {
            passed += 1;
        } else {
            failed += 1;
        }
    }

    let elapsed = start.elapsed();
    let elapsed_secs = elapsed.as_secs_f64();

    // Summary line. It counts files: each file's binary already printed its
    // own test counts, so a bare "1 passed" here read like a lost test.
    let files = |n: usize| if n == 1 { "test file" } else { "test files" };
    if failed > 0 {
        println!(
            "\n{red}{bold}{failed} {} failed{reset}, {passed} passed in {elapsed_secs:.2}s",
            files(failed)
        );
    } else {
        println!(
            "\n{green}{bold}{passed} {} passed{reset} in {elapsed_secs:.2}s",
            files(passed)
        );
    }

    Ok(TestSummary { passed, failed })
}

// ── Source Preprocessor ───────────────────────────────────────────────────

/// Preprocess a .test.mpl source file into a valid Mesh program. The file is
/// rewritten in place, so each of its lines keeps its line and column and a
/// diagnostic points at the test as written:
///
/// - `test("label") do` becomes `fn __test_body_N() do`.
/// - `describe("group") do` becomes `fn __test_describe_N(__case :: Int) do`. Its
///   `setup` lines run first, so what they bind is in scope for the tests and
///   the teardown; each `test` in it becomes
///   `if __case == I do test_run_body(fn() do ... end) end`; a `teardown`
///   becomes a closure run after the test, whether the test passed or not.
/// - `assert_receive PATTERN[, TIMEOUT]` becomes a `receive` on its line.
/// - A `fn main()` is appended that runs each test (`test_begin`,
///   `test_run_body`, `test_end`) and then `test_summary`.
///
/// A file that does not parse is returned as it is, for the build to report
/// its errors where they are.
pub fn preprocess_test_source(source: &str) -> Result<String, String> {
    let parse = mesh_parser::parse(source);
    if !parse.errors().is_empty() {
        return Ok(source.to_string());
    }

    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    // Each test: its label, and the call in `main` that runs it.
    let mut tests: Vec<(String, String)> = Vec::new();
    let mut describes = 0;
    for node in parse.syntax().children() {
        let Some(call) = BlockCall::of(&node) else {
            continue;
        };
        match call.name.as_str() {
            "test" => {
                let n = tests.len();
                call.rewrite(&mut edits, &format!("fn __test_body_{n}() do"), "end");
                tests.push((call.label("unnamed"), format!("__test_body_{n}()")));
            }
            "describe" => {
                let d = describes;
                describes += 1;
                let group = call.label("describe");
                let mut cases = 0;
                let mut teardown = false;
                for stmt in call.block.children() {
                    let Some(inner) = BlockCall::of(&stmt) else {
                        continue;
                    };
                    match inner.name.as_str() {
                        "setup" if cases > 0 => {
                            return Err(format!(
                                "line {}: `setup` must come before the tests of its describe",
                                line_of(source, inner.header.start)
                            ));
                        }
                        "setup" => inner.rewrite(&mut edits, "", ""),
                        "teardown" if teardown => {
                            return Err(format!(
                                "line {}: a describe has one `teardown`",
                                line_of(source, inner.header.start)
                            ));
                        }
                        "teardown" => {
                            teardown = true;
                            inner.rewrite(&mut edits, "let __teardown = fn() do", "end");
                        }
                        "test" => {
                            inner.rewrite(
                                &mut edits,
                                &format!("if __case == {cases} do test_run_body(fn() do"),
                                "end) end",
                            );
                            tests.push((
                                format!("{group} > {}", inner.label("unnamed")),
                                format!("__test_describe_{d}({cases})"),
                            ));
                            cases += 1;
                        }
                        _ => {}
                    }
                }
                let end = if teardown {
                    "test_run_body(__teardown) end"
                } else {
                    "end"
                };
                call.rewrite(
                    &mut edits,
                    &format!("fn __test_describe_{d}(__case :: Int) do"),
                    end,
                );
            }
            _ => continue,
        }
        // An `assert_receive` becomes a `receive` on its own lines.
        for node in call.block.descendants() {
            if node.kind() == SyntaxKind::ASSERT_RECEIVE_EXPR {
                edits.push((range_of(&node), expand_assert_receive(&node)));
            }
        }
    }

    if tests.is_empty() {
        // Not a test file or no test blocks — pass through unchanged.
        return Ok(source.to_string());
    }

    let mut out = source.to_string();
    edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    for (range, text) in edits {
        out.replace_range(range, &text);
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }

    // The harness, after every line of the file; `test_end` counts the test
    // once. The label is the test's string literal, as written.
    out.push_str("\nfn main() do\n");
    for (label, run) in &tests {
        out.push_str(&format!(
            "  test_cleanup_actors()\n  test_begin(\"{label}\")\n  test_run_body(fn() do {run} end)\n  test_end()\n"
        ));
    }
    // Pass 0 for elapsed_ms; accurate timing is cosmetic and can be added later.
    out.push_str("  test_summary(test_pass_count(), test_fail_count(), 0)\n");
    out.push_str("end\n");
    Ok(out)
}

/// A `name(...) do ... end` or `name do ... end` statement, as `test`,
/// `describe`, `setup` and `teardown` are written.
struct BlockCall {
    name: String,
    call: SyntaxNode,
    /// From the call's start through its `do`.
    header: Range<usize>,
    /// The statements of its `do` block.
    block: SyntaxNode,
    /// The block's `end`.
    end: Range<usize>,
}

impl BlockCall {
    fn of(node: &SyntaxNode) -> Option<Self> {
        if node.kind() != SyntaxKind::CALL_EXPR {
            return None;
        }
        let name = node
            .children()
            .find(|child| child.kind() == SyntaxKind::NAME_REF)?
            .text()
            .to_string();
        let closure = node
            .children()
            .find(|child| child.kind() == SyntaxKind::TRAILING_CLOSURE)?;
        let block = closure
            .children()
            .find(|child| child.kind() == SyntaxKind::BLOCK)?;
        let end = closure
            .children_with_tokens()
            .filter_map(|element| element.into_token())
            .find(|token| token.kind() == SyntaxKind::END_KW)?
            .text_range();
        Some(BlockCall {
            name,
            header: range_of(node).start..range_of(&block).start,
            end: end.start().into()..end.end().into(),
            call: node.clone(),
            block,
        })
    }

    /// The first argument's string literal, as written between its quotes.
    fn label(&self, default: &str) -> String {
        self.call
            .children()
            .find(|child| child.kind() == SyntaxKind::ARG_LIST)
            .and_then(|args| {
                args.children()
                    .find(|child| child.kind() == SyntaxKind::STRING_EXPR)
            })
            .map(|string| {
                string
                    .descendants_with_tokens()
                    .filter_map(|element| element.into_token())
                    .filter(|token| {
                        !matches!(
                            token.kind(),
                            SyntaxKind::STRING_START | SyntaxKind::STRING_END
                        )
                    })
                    .map(|token| token.text().to_string())
                    .collect()
            })
            .unwrap_or_else(|| default.to_string())
    }

    /// Replace the header and the `end`, keeping the header's line breaks.
    fn rewrite(&self, edits: &mut Vec<(Range<usize>, String)>, header: &str, end: &str) {
        let header = (
            self.header.clone(),
            keep_lines(header, &self.call, self.header.clone()),
        );
        edits.push(header);
        edits.push((self.end.clone(), end.to_string()));
    }
}

fn range_of(node: &SyntaxNode) -> Range<usize> {
    let range = node.text_range();
    range.start().into()..range.end().into()
}

/// `text` with as many line breaks as `range` (within `node`) had.
fn keep_lines(text: &str, node: &SyntaxNode, range: Range<usize>) -> String {
    let start = range_of(node).start;
    let original = &node.text().to_string()[range.start - start..range.end - start];
    format!("{text}{}", "\n".repeat(original.matches('\n').count()))
}

fn line_of(source: &str, offset: usize) -> usize {
    source[..offset].matches('\n').count() + 1
}

// ── assert_receive ────────────────────────────────────────────────────────

/// `assert_receive PATTERN[, TIMEOUT_MS]` (timeout 100ms by default) as a
/// `receive` on the same lines:
///
///   receive do
///     PATTERN -> ()
///     __assert_receive_other -> test_fail_msg("assert_receive PATTERN received another message")
///   after TIMEOUT_MS -> test_fail_msg("assert_receive PATTERN timed out after TIMEOUT_MSms")
///   end
///
/// The catch-all arm fails the test on a message the pattern does not match
/// (the type checker does not report it as redundant).
fn expand_assert_receive(node: &SyntaxNode) -> String {
    let mut parts = node.children().map(|child| child.text().to_string());
    let pattern = parts.next().unwrap_or_default();
    let timeout_ms = parts.next().unwrap_or_else(|| "100".to_string());
    // Escape the pattern for embedding in the failure messages.
    let escaped = pattern.replace('\\', "\\\\").replace('"', "\\\"");
    let expansion = format!(
        "receive do {pattern} -> () __assert_receive_other -> test_fail_msg(\"assert_receive {escaped} received another message\") after {timeout_ms} -> test_fail_msg(\"assert_receive {escaped} timed out after {timeout_ms}ms\") end"
    );
    keep_lines(&expansion, node, range_of(node))
}

// ── Copy project sources into temp dir for cross-module test compilation ──

/// Copy all non-test .mpl source files from `project_dir` into `tmp_dir`,
/// preserving relative directory structure.
///
/// This enables test files that import project modules (e.g., `from Ingestion.Fingerprint
/// import compute_fingerprint`) to compile successfully. The test file itself is written
/// as `main.mpl` by the caller after this function runs.
///
/// Files excluded from copying:
/// - `*.test.mpl` files (they are test DSL, not regular Mesh modules)
/// - `*.test-support.mpl` files as standalone modules (they are merged into their sibling module)
/// - The resolved executable entry file for the original project (replaced by synthetic `main.mpl`)
/// - Hidden directories (names starting with `.`)
/// - The `target` directory (build artifacts)
fn copy_project_sources_to_tmp(
    project_dir: &Path,
    tmp_dir: &Path,
    excluded_entry_relative_path: &Path,
) -> Result<(), String> {
    copy_sources_recursive(
        project_dir,
        project_dir,
        tmp_dir,
        excluded_entry_relative_path,
    )
}

fn copy_sources_recursive(
    project_root: &Path,
    dir: &Path,
    tmp_dir: &Path,
    excluded_entry_relative_path: &Path,
) -> Result<(), String> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("Failed to read '{}': {}", dir.display(), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("Failed to read '{}': {}", dir.display(), e))?;
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip hidden directories and build artifacts
        if name_str.starts_with('.') || name_str == "target" {
            continue;
        }

        let file_type = entry
            .file_type()
            .map_err(|e| format!("Failed to inspect '{}': {}", path.display(), e))?;
        if file_type.is_symlink() {
            return Err(format!(
                "Test project path '{}' must not be a symbolic link.",
                path.display()
            ));
        }

        if file_type.is_dir() {
            copy_sources_recursive(project_root, &path, tmp_dir, excluded_entry_relative_path)?;
        } else if file_type.is_file() && path.extension().and_then(|e| e.to_str()) == Some("mpl") {
            if name_str.ends_with(".test.mpl") {
                continue;
            }
            if let Some(module_name) = name_str.strip_suffix(".test-support.mpl") {
                let module_path = path.with_file_name(format!("{module_name}.mpl"));
                if !module_path.is_file() {
                    return Err(format!(
                        "Test-support fragment '{}' requires sibling module '{}'.",
                        path.display(),
                        module_path.display()
                    ));
                }
                let module_relative = module_path.strip_prefix(project_root).map_err(|e| {
                    format!(
                        "Failed to map '{}' under project root '{}': {}",
                        module_path.display(),
                        project_root.display(),
                        e
                    )
                })?;
                if module_relative == excluded_entry_relative_path {
                    return Err(format!(
                        "Test-support fragment '{}' cannot target executable entry '{}'.",
                        path.display(),
                        module_relative.display()
                    ));
                }
                if module_relative == Path::new(DEFAULT_ENTRYPOINT) {
                    return Err(format!(
                        "Test-support fragment '{}' cannot target synthetic test entry '{}'.",
                        path.display(),
                        module_relative.display()
                    ));
                }
                continue;
            }
            let relative = path.strip_prefix(project_root).map_err(|e| {
                format!(
                    "Failed to map '{}' under project root '{}': {}",
                    path.display(),
                    project_root.display(),
                    e
                )
            })?;
            if relative == excluded_entry_relative_path || relative == Path::new(DEFAULT_ENTRYPOINT)
            {
                continue;
            }
            let dest = tmp_dir.join(relative);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create '{}': {}", parent.display(), e))?;
            }
            let module_name = name_str.strip_suffix(".mpl").unwrap_or(&name_str);
            let test_support_path = path.with_file_name(format!("{module_name}.test-support.mpl"));
            if test_support_path.is_file() {
                let mut source = std::fs::read_to_string(&path)
                    .map_err(|e| format!("Failed to read '{}': {}", path.display(), e))?;
                if !source.ends_with('\n') {
                    source.push('\n');
                }
                let test_support = std::fs::read_to_string(&test_support_path).map_err(|e| {
                    format!("Failed to read '{}': {}", test_support_path.display(), e)
                })?;
                source.push_str(&test_support);
                std::fs::write(&dest, source)
                    .map_err(|e| format!("Failed to write '{}': {}", dest.display(), e))?;
            } else {
                std::fs::copy(&path, &dest).map_err(|e| {
                    format!(
                        "Failed to copy '{}' to '{}': {}",
                        path.display(),
                        dest.display(),
                        e
                    )
                })?;
            }
        }
    }
    Ok(())
}

// ── Recursively discover all *.test.mpl files in a directory ─────────────

fn discover_test_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    discover_recursive(root, &mut files)
        .map_err(|e| format!("Failed to walk '{}': {}", root.display(), e))?;
    files.sort();
    Ok(files)
}

fn discover_recursive(dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        // Skip hidden directories (e.g., .planning, .git, target) and build artifacts
        if name_str.starts_with('.') || name_str == "target" {
            continue;
        }
        if path.is_dir() {
            discover_recursive(&path, files)?;
        } else if path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.ends_with(".test.mpl"))
            .unwrap_or(false)
        {
            files.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn preprocess_test_source_keeps_every_line_in_place() {
        // Diagnostics named the generated file's lines, which had moved: the
        // tests were emitted after the other items, re-indented.
        let source = "fn pick(n :: Int) -> Int do\n  if n < 0 do\n    0\n  else if n > 9 do\n    9\n  else\n    n\n  end\nend\n\ntest(\"pick\") do\n  if pick(3) == 3 do\n    assert(true)\n  else if pick(3) == 0 do\n    assert(false)\n  else\n    assert(false)\n  end\nend\n\nfn later() -> Int do\n  1\nend\n";

        let out = preprocess_test_source(source).unwrap();

        let out_lines: Vec<&str> = out.lines().collect();
        for (i, line) in source.lines().enumerate() {
            if i == 10 {
                assert_eq!(out_lines[i], "fn __test_body_0() do", "{out}");
            } else {
                assert_eq!(out_lines[i], line, "{out}");
            }
        }
        assert!(
            out[source.len()..].contains(
                "test_begin(\"pick\")\n  test_run_body(fn() do __test_body_0() end)\n  test_end()"
            ),
            "{out}"
        );
    }

    #[test]
    fn preprocess_test_source_reads_strings_in_interpolations() {
        // The text scanner took the `end` in `" end "` for the test's own.
        let source = "test(\"interp\") do\n  let s = \"#{String.join([\"a\"], \" end \")}\"\n  assert_eq(s, \"a\")\nend\n";
        let out = preprocess_test_source(source).unwrap();
        assert!(
            out.starts_with(&source.replace("test(\"interp\") do", "fn __test_body_0() do")),
            "{out}"
        );
    }

    #[test]
    fn preprocess_test_source_runs_a_describe_by_case() {
        let source = "describe(\"group\") do\n  setup() do\n    let base = if true do\n      1\n    else if false do\n      2\n    else\n      3\n    end\n  end\n  teardown do\n    println(\"#{base}\")\n  end\n  test(\"one\") do\n    assert_receive 40, 50\n  end\nend\n\ntest(\"two\") do\n  assert(true)\nend\n";

        let out = preprocess_test_source(source).unwrap();

        let out_lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            out_lines[0], "fn __test_describe_0(__case :: Int) do",
            "{out}"
        );
        assert_eq!(out_lines[1], "  ", "{out}");
        assert_eq!(out_lines[2], "    let base = if true do", "{out}");
        assert_eq!(out_lines[9], "  ", "{out}");
        assert_eq!(out_lines[10], "  let __teardown = fn() do", "{out}");
        assert_eq!(
            out_lines[13], "  if __case == 0 do test_run_body(fn() do",
            "{out}"
        );
        assert!(
            out_lines[14].starts_with("    receive do 40 -> () "),
            "{out}"
        );
        assert!(
            out_lines[14].ends_with(
                "after 50 -> test_fail_msg(\"assert_receive 40 timed out after 50ms\") end"
            ),
            "{out}"
        );
        assert_eq!(out_lines[15], "  end) end", "{out}");
        assert_eq!(out_lines[16], "test_run_body(__teardown) end", "{out}");
        assert_eq!(out_lines[18], "fn __test_body_1() do", "{out}");
        assert!(
            out.contains(
                "test_begin(\"group > one\")\n  test_run_body(fn() do __test_describe_0(0) end)"
            ),
            "{out}"
        );
        assert!(out.contains("test_begin(\"two\")"), "{out}");
    }

    #[test]
    fn preprocess_test_source_refuses_a_late_setup_and_passes_parse_errors() {
        let late = "describe(\"g\") do\n  test(\"a\") do\n    assert(true)\n  end\n  setup do\n    let x = 1\n  end\nend\n";
        let err = preprocess_test_source(late).unwrap_err();
        assert!(err.starts_with("line 5: `setup` must come before"), "{err}");

        // The build reports the parse error where it is.
        let broken = "test(\"a\") do\n  let x = (1\nend\n";
        assert_eq!(preprocess_test_source(broken).unwrap(), broken);
    }

    #[test]
    fn resolve_project_dir_prefers_nearest_manifest_for_override_entry_file_targets() {
        let temp = tempfile::tempdir().unwrap();
        let project_dir = temp.path().join("override-project");
        let test_file = project_dir.join("tests").join("feature.test.mpl");

        write_file(
            &project_dir.join("mesh.toml"),
            "[package]\nname = \"override-project\"\nversion = \"0.1.0\"\nentrypoint = \"lib/start.mpl\"\n",
        );
        write_file(
            &project_dir.join("lib/start.mpl"),
            "fn main() do\n  println(\"app\")\nend\n",
        );
        write_file(&test_file, "test(\"ok\") do\n  assert(true)\nend\n");

        let resolved = resolve_project_dir(Some(&test_file)).unwrap();

        assert_eq!(resolved, project_dir);
    }

    #[test]
    fn resolve_project_dir_rejects_orphan_test_file_instead_of_falling_back() {
        let temp = tempfile::tempdir().unwrap();
        let orphan = temp.path().join("orphan.test.mpl");
        write_file(&orphan, "test(\"orphan\") do\n  assert(true)\nend\n");

        let err = resolve_project_dir(Some(&orphan)).unwrap_err();

        assert!(
            err.contains("Could not resolve a Mesh project root"),
            "{err}"
        );
        assert!(err.contains(&orphan.display().to_string()), "{err}");
    }

    #[test]
    fn resolve_test_project_accepts_library_without_executable_entrypoint() {
        let temp = tempfile::tempdir().unwrap();
        let project_dir = temp.path().join("library");
        write_file(
            &project_dir.join("mesh.toml"),
            "[package]\nname = \"library\"\nversion = \"0.1.0\"\n",
        );
        write_file(
            &project_dir.join("library.mpl"),
            "pub fn answer() -> Int do\n  42\nend\n",
        );
        write_file(
            &project_dir.join("tests/library.test.mpl"),
            "test(\"ok\") do\n  assert(true)\nend\n",
        );

        let resolved = resolve_test_project(Some(&project_dir)).unwrap();

        assert_eq!(
            resolved.entry_relative_path,
            PathBuf::from(DEFAULT_ENTRYPOINT)
        );
    }

    #[test]
    fn copy_project_sources_to_tmp_excludes_reserved_entries_and_keeps_support_modules() {
        let temp = tempfile::tempdir().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let project_dir = temp.path().join("override-project");

        write_file(
            &project_dir.join("lib/start.mpl"),
            "fn main() do\n  println(\"app\")\nend\n",
        );
        write_file(
            &project_dir.join("main.mpl"),
            "fn main() do\n  println(\"unused\")\nend\n",
        );
        write_file(
            &project_dir.join("app.mpl"),
            "pub fn answer() -> Int do\n  42\nend\n",
        );
        write_file(
            &temp.path().join("shared/mesh.toml"),
            "[package]\nname = \"shared\"\nversion = \"0.1.0\"\n",
        );
        write_file(
            &project_dir.join("tests/support.mpl"),
            "pub fn check() -> String do\n  \"support\"\nend\n",
        );
        write_file(
            &project_dir.join("tests/feature.test.mpl"),
            "test(\"skip\") do\n  assert(true)\nend\n",
        );

        copy_project_sources_to_tmp(&project_dir, tmp.path(), Path::new("lib/start.mpl")).unwrap();

        assert!(!tmp.path().join("lib/start.mpl").exists());
        assert!(!tmp.path().join("main.mpl").exists());
        assert!(tmp.path().join("app.mpl").exists());
        assert!(tmp.path().join("tests/support.mpl").exists());
        assert!(!tmp.path().join("tests/feature.test.mpl").exists());
    }

    #[test]
    fn copy_project_sources_to_tmp_merges_test_support_into_its_module() {
        let project = tempfile::tempdir().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        write_file(
            &project.path().join("account.mpl"),
            "fn private_value() -> Int do\n  42\nend\n",
        );
        write_file(
            &project.path().join("account.test-support.mpl"),
            "pub fn test_value() -> Int do\n  private_value()\nend\n",
        );

        copy_project_sources_to_tmp(project.path(), tmp.path(), Path::new("main.mpl")).unwrap();

        let module = std::fs::read_to_string(tmp.path().join("account.mpl")).unwrap();
        assert!(module.contains("fn private_value()"));
        assert!(module.contains("pub fn test_value()"));
        assert!(!tmp.path().join("account.test-support.mpl").exists());
    }

    #[test]
    fn copy_project_sources_to_tmp_rejects_orphan_test_support() {
        let project = tempfile::tempdir().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        write_file(
            &project.path().join("missing.test-support.mpl"),
            "pub fn helper() -> Int do\n  42\nend\n",
        );

        let err = copy_project_sources_to_tmp(project.path(), tmp.path(), Path::new("main.mpl"))
            .unwrap_err();

        assert!(err.contains("missing.test-support.mpl"), "{err}");
        assert!(err.contains("missing.mpl"), "{err}");
    }

    #[test]
    fn copy_project_sources_to_tmp_rejects_test_support_for_excluded_entry() {
        let project = tempfile::tempdir().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        write_file(&project.path().join("lib/start.mpl"), "fn main() do\nend\n");
        write_file(
            &project.path().join("lib/start.test-support.mpl"),
            "pub fn helper() -> Int do\n  42\nend\n",
        );

        let err =
            copy_project_sources_to_tmp(project.path(), tmp.path(), Path::new("lib/start.mpl"))
                .unwrap_err();

        assert!(err.contains("lib/start.test-support.mpl"), "{err}");
        assert!(err.contains("executable entry"), "{err}");
    }

    #[test]
    fn copy_project_sources_to_tmp_rejects_test_support_for_synthetic_entry() {
        let project = tempfile::tempdir().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        write_file(&project.path().join("main.mpl"), "fn helper() do\nend\n");
        write_file(
            &project.path().join("main.test-support.mpl"),
            "pub fn helper_for_test() do\n  helper()\nend\n",
        );

        let err =
            copy_project_sources_to_tmp(project.path(), tmp.path(), Path::new("lib/start.mpl"))
                .unwrap_err();

        assert!(err.contains("main.test-support.mpl"), "{err}");
        assert!(err.contains("synthetic test entry"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn copy_project_sources_to_tmp_rejects_symlinked_sources() {
        use std::os::unix::fs::symlink;

        let project = tempfile::tempdir().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        symlink(outside.path(), project.path().join("account.mpl")).unwrap();

        let err = copy_project_sources_to_tmp(project.path(), tmp.path(), Path::new("main.mpl"))
            .unwrap_err();

        assert!(err.contains("account.mpl"), "{err}");
        assert!(err.contains("symbolic link"), "{err}");
    }

    #[test]
    fn prepare_temp_test_project_rewrites_entrypoint_to_synthetic_main_and_preserves_dependencies()
    {
        let temp = tempfile::tempdir().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let project_dir = temp.path().join("override-project");

        write_file(
            &project_dir.join("mesh.toml"),
            "[package]\nname = \"override-project\"\nversion = \"0.1.0\"\nentrypoint = \"lib/start.mpl\"\n\n[dependencies]\nshared = { path = \"../shared\" }\n",
        );
        write_file(
            &project_dir.join("lib/start.mpl"),
            "fn main() do\n  println(\"app\")\nend\n",
        );
        write_file(
            &project_dir.join("app.mpl"),
            "pub fn answer() -> Int do\n  42\nend\n",
        );
        write_file(
            &temp.path().join("shared/mesh.toml"),
            "[package]\nname = \"shared\"\nversion = \"0.1.0\"\n",
        );

        let test_project = resolve_test_project(Some(&project_dir)).unwrap();
        prepare_temp_test_project(
            &test_project,
            tmp.path(),
            "fn main() do\n  println(\"tests\")\nend\n",
        )
        .unwrap();

        let manifest = Manifest::from_file(&tmp.path().join("mesh.toml")).unwrap();
        let entrypoint = resolve_entrypoint(tmp.path(), Some(&manifest)).unwrap();

        assert_eq!(entrypoint, PathBuf::from(DEFAULT_ENTRYPOINT));
        match &manifest.dependencies["shared"] {
            mesh_pkg::manifest::Dependency::Path { path } => assert_eq!(
                Path::new(path),
                temp.path().join("shared").canonicalize().unwrap()
            ),
            dependency => panic!("expected path dependency, got {dependency:?}"),
        }
        assert!(!tmp.path().join("lib/start.mpl").exists());
        assert!(tmp.path().join(DEFAULT_ENTRYPOINT).exists());
    }
}
