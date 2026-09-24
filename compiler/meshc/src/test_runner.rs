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

        let preprocessed = preprocess_test_source(&source);

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

        let diag_opts = DiagnosticOptions {
            color: use_color(),
            json: false,
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

/// A test block extracted from the .test.mpl source.
#[derive(Debug)]
struct TestBlock {
    /// Full test label (includes describe group prefix when nested).
    label: String,
    /// Source text of the test body (between `do` and the matching `end`).
    body: String,
    /// Optional setup body to run before this test (from enclosing describe).
    setup_body: Option<String>,
    /// Optional teardown body to run after this test (from enclosing describe).
    teardown_body: Option<String>,
}

/// Preprocess a .test.mpl source file into a valid Mesh program.
///
/// Transforms:
/// - `test("label") do body end` → `fn __test_body_N() do body end`, with
///   the setup and teardown of an enclosing describe
/// - `describe("group") do setup/teardown/test blocks end` → grouped tests
/// - Generates `fn main() do test_begin/test_run_body/test_end/test_summary ... end`
///
/// The output is standard Mesh that the compiler accepts.
pub fn preprocess_test_source(source: &str) -> String {
    let tokens = tokenize_test_source(source);
    let blocks = extract_test_blocks(&tokens);

    if blocks.is_empty() {
        // Not a test file or no test blocks — pass through unchanged.
        return source.to_string();
    }

    let mut out = String::new();

    // Emit any top-level definitions from the source (fn, struct, etc.)
    // that aren't test/describe blocks.
    emit_non_test_items(source, &mut out);

    // Emit a function for each test: its setup, then its body. A teardown
    // runs after the body whether the body passed or not, so then the body
    // runs as a step of its own; both see what the setup binds. Lines are
    // copied as written: indenting them would change multi-line strings.
    for (i, block) in blocks.iter().enumerate() {
        out.push_str(&format!("fn __test_body_{i}() do\n"));
        if let Some(ref setup) = block.setup_body {
            out.push_str(&transform_assert_receive(setup));
        }
        match block.teardown_body {
            Some(ref teardown) => {
                out.push_str("test_run_body(fn() do\n");
                out.push_str(&transform_assert_receive(&block.body));
                out.push_str("end)\ntest_run_body(fn() do\n");
                out.push_str(&transform_assert_receive(teardown));
                out.push_str("end)\n");
            }
            None => out.push_str(&transform_assert_receive(&block.body)),
        }
        out.push_str("end\n\n");
    }

    // Emit fn main() harness; `test_end` counts the test once.
    out.push_str("fn main() do\n");
    for (i, block) in blocks.iter().enumerate() {
        // Escape double-quotes in the label for the Mesh string literal.
        let escaped_label = block.label.replace('\\', "\\\\").replace('"', "\\\"");
        out.push_str(&format!(
            "  test_cleanup_actors()\n  test_begin(\"{escaped_label}\")\n  test_run_body(fn() do __test_body_{i}() end)\n  test_end()\n"
        ));
    }
    // Pass 0 for elapsed_ms; accurate timing is cosmetic and can be added later.
    out.push_str("  test_summary(test_pass_count(), test_fail_count(), 0)\n");
    out.push_str("end\n");

    out
}

// ── Tokenizer ─────────────────────────────────────────────────────────────

/// A token kind for the test source mini-lexer.
#[derive(Debug, Clone, PartialEq)]
enum TToken {
    /// `test` keyword (bare IDENT)
    TestKw,
    /// `describe` keyword (bare IDENT)
    DescribeKw,
    /// `setup` keyword (bare IDENT)
    SetupKw,
    /// `teardown` keyword (bare IDENT)
    TeardownKw,
    /// `do` keyword
    Do,
    /// `end` keyword
    End,
    /// `fn` keyword (to track nested fn do ... end)
    Fn,
    /// `if` keyword
    If,
    /// `while` keyword
    While,
    /// `case` keyword
    Case,
    /// `for` keyword
    For,
    /// `actor` keyword
    Actor,
    /// `service` keyword
    Service,
    /// `receive` keyword
    Receive,
    /// A string literal like `"..."` with the raw text (including quotes).
    StringLit(String),
    /// An open paren `(`
    LParen,
    /// A close paren `)`
    RParen,
    /// Everything else (whitespace, comments, other tokens).
    Other(String),
}

/// Tokenize the test source into a flat sequence of TTokens.
///
/// Handles:
/// - String literals (to avoid misidentifying keywords inside strings)
/// - Line comments `# ...`
/// - Keywords: test, describe, setup, teardown, do, end, fn, if, while, case
fn tokenize_test_source(source: &str) -> Vec<TToken> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = source.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        // Skip line comments
        if chars[i] == '#' {
            let mut s = String::new();
            while i < chars.len() && chars[i] != '\n' {
                s.push(chars[i]);
                i += 1;
            }
            tokens.push(TToken::Other(s));
            continue;
        }

        // String literals
        if chars[i] == '"' {
            let mut s = String::new();
            s.push('"');
            i += 1;
            while i < chars.len() {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    s.push(chars[i]);
                    s.push(chars[i + 1]);
                    i += 2;
                } else if chars[i] == '"' {
                    s.push('"');
                    i += 1;
                    break;
                } else {
                    s.push(chars[i]);
                    i += 1;
                }
            }
            tokens.push(TToken::StringLit(s));
            continue;
        }

        // String interpolation `"${...}"` — treat whole thing as string lit
        // (not common in test files but handle to avoid mis-tokenizing)

        // Identifiers and keywords
        if chars[i].is_alphabetic() || chars[i] == '_' {
            let mut ident = String::new();
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                ident.push(chars[i]);
                i += 1;
            }
            let tok = match ident.as_str() {
                "test" => TToken::TestKw,
                "describe" => TToken::DescribeKw,
                "setup" => TToken::SetupKw,
                "teardown" => TToken::TeardownKw,
                "do" => TToken::Do,
                "end" => TToken::End,
                "fn" => TToken::Fn,
                "if" => TToken::If,
                "while" => TToken::While,
                "case" => TToken::Case,
                "for" => TToken::For,
                "actor" => TToken::Actor,
                "service" => TToken::Service,
                "receive" => TToken::Receive,
                _ => TToken::Other(ident),
            };
            tokens.push(tok);
            continue;
        }

        // Parens
        if chars[i] == '(' {
            tokens.push(TToken::LParen);
            i += 1;
            continue;
        }
        if chars[i] == ')' {
            tokens.push(TToken::RParen);
            i += 1;
            continue;
        }

        // Everything else (whitespace, operators, numbers, etc.)
        let mut s = String::new();
        s.push(chars[i]);
        i += 1;
        tokens.push(TToken::Other(s));
    }

    tokens
}

/// How deep the mini token stream is inside `do...end` blocks.
///
/// Every block opener takes its own `do` and `end`, except `else if`: it
/// continues the `if` before it and shares that chain's single `end`, so its
/// `do` opens nothing. Counting `do` and `end` alone leaves the depth one too
/// high after every chain; from then on a top-level `test(...)` block is
/// emitted as ordinary source, and a chain inside a test body swallows
/// whatever follows the block.
struct BlockNesting {
    depth: usize,
    /// Block keywords still waiting for their `do`; `true` marks an `else if`.
    openers: Vec<bool>,
    /// Nothing but whitespace and comments since the last `else`.
    after_else: bool,
}

impl BlockNesting {
    fn new(depth: usize) -> Self {
        Self {
            depth,
            openers: Vec::new(),
            after_else: false,
        }
    }

    /// Accounts for one token and returns the depth after it.
    fn feed(&mut self, tok: &TToken) -> usize {
        match tok {
            TToken::If => self.openers.push(self.after_else),
            TToken::Case
            | TToken::While
            | TToken::For
            | TToken::Fn
            | TToken::Actor
            | TToken::Service
            | TToken::Receive => self.openers.push(false),
            TToken::Do => {
                // A `do` with no keyword waiting for it is a bare block.
                if !self.openers.pop().unwrap_or(false) {
                    self.depth += 1;
                }
            }
            TToken::End => self.depth = self.depth.saturating_sub(1),
            _ => {}
        }
        self.after_else = match tok {
            TToken::Other(text) if text == "else" => true,
            TToken::Other(text) if text.trim().is_empty() || text.starts_with('#') => {
                self.after_else
            }
            _ => false,
        };
        self.depth
    }
}

/// Extract test blocks from the token stream.
///
/// Recognizes:
/// - `test(STRING) do BODY end`
/// - `describe(STRING) do [setup() do BODY end] [teardown() do BODY end] test(...) ... end`
fn extract_test_blocks(tokens: &[TToken]) -> Vec<TestBlock> {
    let mut blocks = Vec::new();
    let mut i = 0;
    extract_blocks_at(tokens, &mut i, None, None, None, &mut blocks);
    blocks
}

/// Extract test blocks starting at index `i`, up to end of token stream or end of a describe block.
///
/// `group_prefix`: label prefix from enclosing describe (e.g., "Group: ").
/// `setup_body`: setup body from enclosing describe.
/// `teardown_body`: teardown body from enclosing describe.
///
/// When `group_prefix` is None (top-level scan), `End` tokens from helper function
/// definitions (e.g., `fn foo() do ... end`) are skipped — they do NOT terminate the scan.
/// When `group_prefix` is Some (inside a describe block), an unmatched `End` terminates
/// the scan (it's the describe block's closing `end`).
fn extract_blocks_at(
    tokens: &[TToken],
    i: &mut usize,
    group_prefix: Option<&str>,
    setup_body: Option<&str>,
    teardown_body: Option<&str>,
    blocks: &mut Vec<TestBlock>,
) {
    // How deep we are inside `do...end` blocks from non-test items (e.g. helper
    // function bodies). While inside one, tokens are skipped without checking
    // for test/describe keywords.
    let mut nesting = BlockNesting::new(0);

    while *i < tokens.len() {
        // Inside a non-test block (e.g., a helper `fn` body) — skip tokens until `end`.
        if nesting.depth > 0 {
            nesting.feed(&tokens[*i]);
            *i += 1;
            continue;
        }

        match &tokens[*i] {
            TToken::TestKw => {
                // test(STRING) do BODY end
                *i += 1;
                // Expect ( STRING )
                let label = extract_string_arg(tokens, i).unwrap_or_else(|| "unnamed".to_string());
                let full_label = match group_prefix {
                    Some(prefix) => format!("{} > {}", prefix, label),
                    None => label,
                };
                // Expect 'do'
                skip_to_do(tokens, i);
                if *i < tokens.len() {
                    *i += 1; // consume 'do'
                }
                // Extract body until matching 'end'
                let body = extract_block_body(tokens, i);
                blocks.push(TestBlock {
                    label: full_label,
                    body,
                    setup_body: setup_body.map(|s| s.to_string()),
                    teardown_body: teardown_body.map(|s| s.to_string()),
                });
            }
            TToken::DescribeKw => {
                // describe(STRING) do [setup] [teardown] test... end
                *i += 1;
                let group_name =
                    extract_string_arg(tokens, i).unwrap_or_else(|| "describe".to_string());
                skip_to_do(tokens, i);
                if *i < tokens.len() {
                    *i += 1; // consume 'do'
                }
                // Now parse the describe body: find setup, teardown, and test blocks.
                let (inner_setup, inner_teardown, inner_end) = peek_describe_body(tokens, *i);
                // Walk only the test tokens between setup/teardown sub-blocks.
                extract_tests_from_describe(
                    tokens,
                    *i,
                    inner_end,
                    &group_name,
                    inner_setup.as_deref(),
                    inner_teardown.as_deref(),
                    blocks,
                );
                // Advance past the describe body.
                *i = inner_end;
            }
            TToken::End => {
                if group_prefix.is_some() {
                    // End of a describe block (caller handles this).
                    *i += 1;
                    return;
                }
                // At top level: this `end` shouldn't be here unmatched
                // (depth tracking above handles normal cases). Skip it.
                *i += 1;
            }
            tok => {
                // A `do` here enters a non-test block (e.g. a helper function
                // body). The keyword before it, and any `else`, are tracked so
                // the block's `end` is recognised.
                nesting.feed(tok);
                *i += 1;
            }
        }
    }
}

/// Extract test blocks from within a describe body, skipping setup/teardown sub-blocks.
///
/// `start`: token index at the start of the describe body (just after the opening `do`).
/// `end_idx`: token index just after the closing `end` of the describe (from peek_describe_body).
fn extract_tests_from_describe(
    tokens: &[TToken],
    start: usize,
    end_idx: usize,
    group_name: &str,
    setup_body: Option<&str>,
    teardown_body: Option<&str>,
    blocks: &mut Vec<TestBlock>,
) {
    let mut i = start;
    // end_idx points AFTER the describe's closing `end`, so we stop before it.
    // The setup/teardown sub-block being skipped, tracked from its opening `do`.
    let mut skipped: Option<BlockNesting> = None;

    while i < tokens.len() {
        // Stop when we've passed the describe's closing token range.
        // peek_describe_body positions end_idx after the closing `end`, so
        // the closing `end` is at end_idx - 1. We stop at end_idx - 1.
        if i >= end_idx.saturating_sub(1) {
            break;
        }

        if let Some(nesting) = skipped.as_mut() {
            // Inside a setup/teardown block body — skip everything and track nesting.
            let closed = nesting.feed(&tokens[i]) == 0;
            i += 1;
            if closed {
                skipped = None;
            }
            continue;
        }

        match &tokens[i] {
            TToken::SetupKw | TToken::TeardownKw => {
                // Skip this setup/teardown sub-block entirely.
                // Skip past the keyword, then find and consume the opening Do.
                i += 1;
                while i < tokens.len() {
                    if matches!(tokens[i], TToken::Do) {
                        skipped = Some(BlockNesting::new(1));
                        i += 1; // consume 'do', now inside the block
                        break;
                    }
                    i += 1;
                }
            }
            TToken::TestKw => {
                i += 1;
                let label =
                    extract_string_arg(tokens, &mut i).unwrap_or_else(|| "unnamed".to_string());
                let full_label = format!("{} > {}", group_name, label);
                skip_to_do(tokens, &mut i);
                if i < tokens.len() {
                    i += 1; // consume 'do'
                }
                let body = extract_block_body(tokens, &mut i);
                blocks.push(TestBlock {
                    label: full_label,
                    body,
                    setup_body: setup_body.map(|s| s.to_string()),
                    teardown_body: teardown_body.map(|s| s.to_string()),
                });
            }
            _ => {
                i += 1;
            }
        }
    }
}

/// Parse the describe body to extract optional `setup()` and `teardown()` bodies.
///
/// Returns `(setup_body, teardown_body, end_index)`.
/// `end_index` points to the token AFTER the matching `end` of the describe.
fn peek_describe_body(tokens: &[TToken], start: usize) -> (Option<String>, Option<String>, usize) {
    let mut setup = None;
    let mut teardown = None;
    let mut i = start;
    let mut nesting = BlockNesting::new(1); // we're inside the describe's 'do', depth starts at 1

    while i < tokens.len() {
        match &tokens[i] {
            TToken::SetupKw if nesting.depth == 1 => {
                i += 1;
                // Expect `() do BODY end`
                skip_to_do(tokens, &mut i);
                if i < tokens.len() {
                    i += 1;
                } // consume 'do'
                let body = extract_block_body_raw(tokens, &mut i);
                setup = Some(body);
            }
            TToken::TeardownKw if nesting.depth == 1 => {
                i += 1;
                skip_to_do(tokens, &mut i);
                if i < tokens.len() {
                    i += 1;
                } // consume 'do'
                let body = extract_block_body_raw(tokens, &mut i);
                teardown = Some(body);
            }
            TToken::End if nesting.depth == 1 => {
                i += 1; // consume the closing 'end' of describe
                return (setup, teardown, i);
            }
            tok => {
                nesting.feed(tok);
                i += 1;
            }
        }
    }

    (setup, teardown, i)
}

/// Parse a string argument from `(STRING)` at position `i`.
/// Advances `i` past the closing `)`.
fn extract_string_arg(tokens: &[TToken], i: &mut usize) -> Option<String> {
    // Skip whitespace / Other tokens until we find '('
    while *i < tokens.len() {
        match &tokens[*i] {
            TToken::LParen => {
                *i += 1;
                break;
            }
            TToken::Other(_) => {
                *i += 1;
            }
            _ => break,
        }
    }

    // Find the string literal
    let mut label = None;
    while *i < tokens.len() {
        match &tokens[*i] {
            TToken::StringLit(s) => {
                // Strip surrounding quotes
                let inner = s.trim_matches('"').to_string();
                label = Some(inner);
                *i += 1;
            }
            TToken::RParen => {
                *i += 1;
                break;
            }
            TToken::Other(_) => {
                *i += 1;
            }
            _ => {
                *i += 1;
                break;
            }
        }
    }

    label
}

/// Skip tokens until we reach a `do` token. Advances `i` to point AT the `do` token.
fn skip_to_do(tokens: &[TToken], i: &mut usize) {
    while *i < tokens.len() {
        if matches!(tokens[*i], TToken::Do) {
            return;
        }
        *i += 1;
    }
}

/// Extract a block body from the token stream, tracking do/end nesting.
///
/// Called AFTER consuming the opening `do`. Advances `i` past the matching `end`.
/// Returns the extracted body as source text (reconstructed from tokens).
fn extract_block_body(tokens: &[TToken], i: &mut usize) -> String {
    extract_block_body_raw(tokens, i)
}

/// Extract block body as raw source text, tracking do/end nesting.
///
/// Called AFTER consuming the opening `do`. Advances `i` past the matching `end`.
/// `BlockNesting` says which `do` opens a block: every one except the `do` of an
/// `else if`, which continues the chain before it and shares its `end`.
fn extract_block_body_raw(tokens: &[TToken], i: &mut usize) -> String {
    let mut body = String::new();
    let mut nesting = BlockNesting::new(1);

    while *i < tokens.len() {
        let tok = &tokens[*i];
        if matches!(tok, TToken::End) && nesting.depth <= 1 {
            *i += 1; // consume the block's own 'end'
            break;
        }
        nesting.feed(tok);
        body.push_str(&token_to_str(tok));
        *i += 1;
    }

    // Trim leading/trailing whitespace from the body
    body.trim().to_string()
}

fn token_to_str(tok: &TToken) -> String {
    match tok {
        TToken::TestKw => "test".to_string(),
        TToken::DescribeKw => "describe".to_string(),
        TToken::SetupKw => "setup".to_string(),
        TToken::TeardownKw => "teardown".to_string(),
        TToken::Do => "do".to_string(),
        TToken::End => "end".to_string(),
        TToken::Fn => "fn".to_string(),
        TToken::If => "if".to_string(),
        TToken::While => "while".to_string(),
        TToken::Case => "case".to_string(),
        TToken::For => "for".to_string(),
        TToken::Actor => "actor".to_string(),
        TToken::Service => "service".to_string(),
        TToken::Receive => "receive".to_string(),
        TToken::StringLit(s) => s.clone(),
        TToken::LParen => "(".to_string(),
        TToken::RParen => ")".to_string(),
        TToken::Other(s) => s.clone(),
    }
}

/// Emit non-test top-level definitions from the source (fn, struct, type, impl, etc.).
///
/// This preserves user-defined helper functions used in test bodies.
///
/// Uses `tokenize_test_source` and `BlockNesting` for token-level depth tracking,
/// which correctly handles `describe` blocks containing `setup do...end` or
/// `teardown do...end` sub-blocks, and `else if` chains that share one `end`.
/// The old line-by-line `count_do_in_line`/`count_end_in_line` approach failed because
/// each `setup do` and `teardown do` sub-block inside a describe block would confuse
/// the depth counter, causing the describe's closing `end` to be missed.
fn emit_non_test_items(source: &str, out: &mut String) {
    let tokens = tokenize_test_source(source);
    let mut i = 0;
    // Nesting of the non-test blocks being emitted (depth 0 = top level).
    let mut emitted = BlockNesting::new(0);
    // Between a top-level test/describe keyword and its opening `do`.
    let mut skipping = false;
    // The test/describe block being suppressed, tracked from its opening `do`.
    let mut skipped: Option<BlockNesting> = None;

    while i < tokens.len() {
        let tok = &tokens[i];

        if let Some(nesting) = skipped.as_mut() {
            // Inside a test/describe block body — skip everything and track nesting.
            let closed = nesting.feed(tok) == 0;
            i += 1;
            if closed {
                skipped = None;
            }
            continue;
        }

        if skipping {
            // Between TestKw/DescribeKw and the opening Do (skipping label, parens, etc.).
            // Once we see the Do keyword, start depth tracking.
            if matches!(tok, TToken::Do) {
                skipped = Some(BlockNesting::new(1));
                skipping = false;
            }
            // Do not emit anything while skipping.
            i += 1;
            continue;
        }

        match tok {
            TToken::TestKw | TToken::DescribeKw if emitted.depth == 0 => {
                // Start of a test/describe block at top level — suppress it entirely.
                skipping = true;
                // Do not emit the keyword.
            }
            _ => {
                emitted.feed(tok);
                out.push_str(&token_to_str(tok));
            }
        }
        i += 1;
    }

    if !out.trim().is_empty() {
        out.push('\n');
    }
}

// ── assert_receive preprocessor ───────────────────────────────────────────

/// Transform `assert_receive PATTERN, TIMEOUT` lines in a test body into
/// equivalent Mesh `receive` blocks with a timeout arm.
///
/// Handles:
///   assert_receive PATTERN, TIMEOUT_MS
///   assert_receive PATTERN              (default timeout: 100ms)
///
/// Output (for each matching line, all on that line):
///   receive
///     PATTERN -> ()
///     __assert_receive_other -> test_fail_msg("assert_receive PATTERN received another message")
///     after TIMEOUT_MS -> test_fail_msg("assert_receive PATTERN timed out after TIMEOUT_MSms")
///   end
///
/// The catch-all arm fails the test on a message the pattern does not match
/// (the type checker does not report it as redundant).
///
/// LOCKED DECISION: The failure message includes BOTH the pattern and the elapsed time.
/// Format: "assert_receive {pattern} timed out after {timeout_ms}ms"
///
/// Lines that do not start with `assert_receive ` are passed through unchanged.
fn transform_assert_receive(body: &str) -> String {
    let mut out = String::new();
    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("assert_receive ") {
            // Strip "assert_receive " prefix
            let rest = trimmed["assert_receive ".len()..].trim();
            // Split on the last top-level comma to find optional timeout.
            // The pattern may contain commas (e.g., {:ping, "data"}), so split on
            // the last comma that is NOT inside brackets/parens.
            let (pattern, timeout_ms) = split_assert_receive_args(rest);
            let indent = &line[..line.len() - line.trim_start().len()];
            // Escape double quotes inside the pattern for embedding in the error message string.
            let escaped_pattern = pattern.replace('\\', "\\\\").replace('"', "\\\"");
            // One line, so the test body keeps its line numbers.
            out.push_str(&format!(
                "{indent}receive do {pattern} -> () __assert_receive_other -> test_fail_msg(\"assert_receive {escaped_pattern} received another message\") after {timeout_ms} -> test_fail_msg(\"assert_receive {escaped_pattern} timed out after {timeout_ms}ms\") end\n"
            ));
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Split `assert_receive` arguments into (pattern, timeout_ms).
///
/// Splits on the LAST top-level comma (not inside {} or () brackets).
/// If no comma found, returns (rest, "100") — default 100ms timeout.
fn split_assert_receive_args(rest: &str) -> (String, String) {
    // Find the last comma at depth 0 (not inside brackets).
    let chars: Vec<char> = rest.chars().collect();
    let mut depth = 0i32;
    let mut last_comma: Option<usize> = None;
    let mut char_pos = 0usize;

    for (i, &ch) in chars.iter().enumerate() {
        match ch {
            '{' | '(' | '[' => depth += 1,
            '}' | ')' | ']' => depth -= 1,
            ',' if depth == 0 => last_comma = Some(i),
            _ => {}
        }
        char_pos = i;
    }
    let _ = char_pos; // suppress unused warning

    match last_comma {
        Some(pos) => {
            // Reconstruct the string slices from char positions.
            // Since we collected chars, we need byte offsets.
            let byte_pos = rest.char_indices().nth(pos).map(|(b, _)| b).unwrap_or(0);
            let pattern = rest[..byte_pos].trim().to_string();
            let timeout = rest[byte_pos + 1..].trim().to_string();
            let timeout_ms = if timeout.is_empty() {
                "100".to_string()
            } else {
                timeout
            };
            (pattern, timeout_ms)
        }
        None => {
            // No comma — entire rest is the pattern; use default timeout.
            (rest.trim().to_string(), "100".to_string())
        }
    }
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
    fn preprocess_test_source_keeps_else_if_chains_balanced() {
        let source = "fn pick(n :: Int) -> Int do\n  if n < 0 do\n    0\n  else if n > 9 do\n    9\n  else\n    n\n  end\nend\n\ntest(\"pick\") do\n  if pick(3) == 3 do\n    assert(true)\n  else if pick(3) == 0 do\n    assert(false)\n  else\n    assert(false)\n  end\nend\n\nfn after() -> Int do\n  1\nend\n";

        let out = preprocess_test_source(source);

        // The helper before the chain-carrying test is kept, the test block is
        // not emitted as ordinary source, and the helper after it is emitted
        // once, outside the test body.
        assert!(out.contains("fn pick(n :: Int) -> Int do"), "{out}");
        assert!(!out.contains("test(\"pick\")"), "{out}");
        assert_eq!(out.matches("fn after() -> Int do").count(), 1, "{out}");
        let body_start = out.find("fn __test_body_0() do").unwrap();
        let body = &out[body_start..out.find("fn main() do").unwrap()];
        assert!(body.contains("else if pick(3) == 0 do"), "{out}");
        assert!(!body.contains("fn after()"), "{out}");
    }

    #[test]
    fn preprocess_test_source_keeps_else_if_chains_balanced_in_describe_setup() {
        let source = "describe(\"group\") do\n  setup() do\n    if true do\n      1\n    else if false do\n      2\n    else\n      3\n    end\n  end\n  test(\"one\") do\n    assert(true)\n  end\nend\n\ntest(\"two\") do\n  assert(true)\nend\n";

        let out = preprocess_test_source(source);

        assert!(out.contains("test_begin(\"group > one\")"), "{out}");
        assert!(out.contains("test_begin(\"two\")"), "{out}");
        assert!(!out.contains("describe("), "{out}");
        assert!(!out.contains("test(\"two\")"), "{out}");
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
