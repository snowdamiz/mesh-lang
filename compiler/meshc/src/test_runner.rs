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
use std::process::{Child, Command, Stdio};
use std::time::Instant;

use mesh_parser::test_harness::{plan_test_source, TestCase, TestPlan, TestPolicy};
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
fn palette(color: bool) -> (&'static str, &'static str, &'static str, &'static str) {
    if color {
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

/// The target as an absolute path; without one, the current directory.
fn resolve_target_path(target: Option<&Path>) -> Result<PathBuf, String> {
    let target = target.unwrap_or(Path::new("."));
    let abs = std::path::absolute(target)
        .map_err(|e| format!("Failed to resolve '{}': {e}", target.display()))?;
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

/// The nearest directory holding a `mesh.toml`, from the target up.
fn resolve_project_dir(target: &Path) -> Result<PathBuf, String> {
    let start = if target.is_dir() {
        target
    } else {
        target.parent().unwrap_or(target)
    };
    start
        .ancestors()
        .find(|dir| dir.join("mesh.toml").is_file())
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            format!(
                "Could not resolve a Mesh project root for test target '{}'; expected an ancestor with 'mesh.toml'.",
                target.display()
            )
        })
}

fn resolve_test_project(target: &Path) -> Result<ResolvedTestProject, String> {
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

fn resolve_test_files(target: &Path) -> Result<Vec<PathBuf>, String> {
    if target.is_dir() {
        discover_test_files(target)
    } else if target
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".test.mpl"))
    {
        Ok(vec![target.to_path_buf()])
    } else {
        Err(format!(
            "'{}' is not a directory or a *.test.mpl file",
            target.display()
        ))
    }
}

/// The project's sources in `tmp_dir`, its entry replaced by the test
/// program as `main.mpl`, and a manifest naming that entry.
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
    let manifest_source =
        rewrite_test_manifest_source(&test_project.manifest_source, &test_project.project_dir)?;
    write_file(&tmp_dir.join("mesh.toml"), manifest_source)?;
    write_file(&tmp_dir.join(DEFAULT_ENTRYPOINT), preprocessed_source)
}

fn prepare_documentation_entry(
    test_project: &ResolvedTestProject,
    tmp: &Path,
    source_file: &Path,
    source: &str,
) -> Result<(PathBuf, String, String), String> {
    let relative = source_file
        .strip_prefix(&test_project.project_dir)
        .map_err(|e| e.to_string())?;
    let entry = tmp.join(relative);
    let original = std::fs::read_to_string(source_file).map_err(|e| e.to_string())?;
    let copied = std::fs::read_to_string(&entry).unwrap_or_else(|_| original.clone());
    let support = if relative == Path::new(DEFAULT_ENTRYPOINT) {
        ""
    } else {
        copied.strip_prefix(&original).unwrap_or("")
    };
    let generated = format!("{source}\n{support}");
    let mut control = format!("{original}\n{support}");
    let parse = mesh_parser::parse(&original);
    if !parse.syntax().children().any(|node| {
        node.kind() == mesh_parser::SyntaxKind::FN_DEF
            && node.children().any(|n| {
                n.kind() == mesh_parser::SyntaxKind::NAME && n.text().to_string() == "main"
            })
    }) {
        control.push_str("\nfn main() do\nend\n");
    }
    let manifest_path = tmp.join("mesh.toml");
    let manifest = std::fs::read_to_string(&manifest_path).map_err(|e| e.to_string())?;
    let mut manifest: toml::Value = toml::from_str(&manifest).map_err(|e| e.to_string())?;
    manifest["package"]["entrypoint"] =
        toml::Value::String(relative.to_string_lossy().into_owned());
    write_file(
        &manifest_path,
        toml::to_string(&manifest).map_err(|e| e.to_string())?,
    )?;
    if entry != tmp.join(DEFAULT_ENTRYPOINT) {
        std::fs::remove_file(tmp.join(DEFAULT_ENTRYPOINT)).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(entry.parent().unwrap()).map_err(|e| e.to_string())?;
    write_file(&entry, &generated)?;
    Ok((entry, generated, control))
}

fn read_file(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("Failed to read '{}': {e}", path.display()))
}

fn write_file(path: &Path, contents: impl AsRef<[u8]>) -> Result<(), String> {
    std::fs::write(path, contents).map_err(|e| format!("Failed to write '{}': {e}", path.display()))
}

pub struct TestOptions {
    pub quiet: bool,
    pub coverage: bool,
    pub filter: Option<String>,
    pub include_tags: Vec<String>,
    pub exclude_tags: Vec<String>,
    pub test_threads: usize,
    pub doc: bool,
}

impl TestOptions {
    fn selects(&self, case: &TestCase) -> bool {
        self.filter
            .as_ref()
            .is_none_or(|name| case.name.contains(name))
            && self.include_tags.iter().all(|tag| case.tags.contains(tag))
            && !self.exclude_tags.iter().any(|tag| case.tags.contains(tag))
    }

    fn has_selection(&self) -> bool {
        self.filter.is_some() || !self.include_tags.is_empty() || !self.exclude_tags.is_empty()
    }
}

fn target_and_line(target: Option<&Path>) -> Result<(PathBuf, Option<usize>), String> {
    if let Some(target) = target.filter(|p| !p.exists()) {
        if let Some((file, line)) = target.to_str().and_then(|text| text.rsplit_once(':')) {
            if let Ok(line) = line.parse::<usize>() {
                let path = resolve_target_path(Some(Path::new(file)))?;
                if line == 0 || !path.is_file() {
                    return Err("file:line requires a file and a positive, one-based line".into());
                }
                return Ok((path, Some(line)));
            }
        }
    }
    Ok((resolve_target_path(target)?, None))
}

fn resolve_doc_files(target: &Path) -> Result<Vec<PathBuf>, String> {
    if target.is_file() {
        if target.extension().is_some_and(|ext| ext == "mpl") {
            return Ok(vec![target.to_path_buf()]);
        }
        return Err("documentation tests require a *.mpl source file or directory".into());
    }
    let mut files = Vec::new();
    discover_recursive_mode(target, &mut files, true)
        .map_err(|e| format!("Failed to walk '{}': {e}", target.display()))?;
    files.sort();
    Ok(files)
}

fn documentation_plans(source: &str) -> Result<Vec<(TestPlan, bool)>, String> {
    Ok(mesh_parser::test_harness::plan_documentation_tests(source)?
        .into_iter()
        .map(|doc| (doc.plan, doc.compile_fail))
        .collect())
}

struct TestUnit {
    path: PathBuf,
    plan: TestPlan,
    selected: Vec<TestCase>,
    compile_fail: bool,
}

pub fn run_tests(target: Option<&Path>, options: &TestOptions) -> Result<TestSummary, String> {
    if options.coverage {
        return Err("coverage reporting is not implemented for `meshc test`; run the command without --coverage".into());
    }
    let (target, line) = target_and_line(target)?;
    let test_project = resolve_test_project(&target)?;
    let files = if options.doc {
        resolve_doc_files(&target)?
    } else {
        resolve_test_files(&target)?
    };
    let explicit = options.has_selection() || line.is_some() || target.is_file();
    if files.is_empty() && !explicit {
        println!(
            "{}",
            if options.doc {
                "No marked documentation tests found."
            } else {
                "No *.test.mpl files found."
            }
        );
        return Ok(TestSummary {
            passed: 0,
            failed: 0,
        });
    }
    let color = use_color();
    let (green, red, bold, reset) = palette(color);
    let start = Instant::now();
    let mut summary = TestSummary {
        passed: 0,
        failed: 0,
    };
    let mut units = Vec::new();
    for path in files {
        let source = std::fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read '{}': {e}", path.display()))?;
        let plans = if options.doc {
            documentation_plans(&source)
        } else {
            plan_test_source(&source).map(|plan| vec![(plan, false)])
        };
        match plans {
            Ok(plans) => {
                for (plan, compile_fail) in plans {
                    let selected = select_cases(&plan, options, line);
                    if !selected.is_empty()
                        || (!options.doc && plan.has_parse_errors())
                        || (!explicit && !options.doc && plan.cases.is_empty())
                    {
                        units.push(TestUnit {
                            path: path.clone(),
                            plan,
                            selected,
                            compile_fail,
                        });
                    }
                }
            }
            Err(error) => {
                println!(
                    "{red}{bold}COMPILE ERROR{reset}: {}\n  {error}",
                    path.strip_prefix(&test_project.project_dir)
                        .unwrap_or(&path)
                        .display()
                );
                summary.failed += 1;
            }
        }
    }
    if units.is_empty() && summary.failed == 0 {
        if explicit {
            return Err("No test cases match the explicit selection".into());
        }
        println!("No marked documentation tests found.");
        return Ok(summary);
    }
    for unit in units {
        let label = unit
            .path
            .strip_prefix(&test_project.project_dir)
            .unwrap_or(&unit.path)
            .display()
            .to_string();
        let tmp = tempfile::tempdir().map_err(|e| format!("Failed to create temp dir: {e}"))?;
        let source = unit.plan.executable();
        if let Err(error) = prepare_temp_test_project(&test_project, tmp.path(), &source) {
            println!("{red}{bold}SETUP ERROR{reset}: {label}\n  {error}");
            summary.failed += 1;
            continue;
        }
        let (entry, source, control) = if options.doc {
            prepare_documentation_entry(&test_project, tmp.path(), &unit.path, &source)?
        } else {
            (tmp.path().join(DEFAULT_ENTRYPOINT), source, String::new())
        };
        let diag_opts = DiagnosticOptions {
            color,
            json: false,
            display_paths: vec![
                (entry.clone(), unit.path.clone()),
                (tmp.path().to_path_buf(), test_project.project_dir.clone()),
            ],
        };
        let bin = tmp.path().join("test_bin");
        if unit.compile_fail {
            // A broken dependency or missing compiler prerequisite cannot satisfy a negative example.
            write_file(&entry, &control)?;
            let control = crate::build(
                tmp.path(),
                0,
                false,
                Some(&bin),
                None,
                crate::BuildArtifact::Executable,
                true,
                &diag_opts,
            );
            write_file(&entry, &source)?;
            if let Err(error) = control {
                println!(
                    "{red}{bold}COMPILE ERROR{reset}: {label} (doctest prerequisites)\n  {error}"
                );
                summary.failed += 1;
                continue;
            }
        }
        let compiled = crate::build(
            tmp.path(),
            0,
            false,
            Some(&bin),
            None,
            crate::BuildArtifact::Executable,
            true,
            &diag_opts,
        );
        if unit.compile_fail {
            match compiled {
                Err(error) if error == "Compilation failed due to errors above." => {
                    println!(
                        "PASS: {label}:{} (expected compile failure)",
                        unit.selected[0].span.start_line
                    );
                    summary.passed += 1;
                }
                result => {
                    println!(
                        "FAIL: {label}:{} expected a parse or type error, got {}",
                        unit.selected[0].span.start_line,
                        result
                            .err()
                            .unwrap_or_else(|| "successful compilation".into())
                    );
                    summary.failed += 1;
                }
            }
            continue;
        }
        if let Err(error) = compiled {
            println!("{red}{bold}COMPILE ERROR{reset}: {label}\n  {error}");
            summary.failed += 1;
            continue;
        }
        let failures = execute_cases(&bin, tmp.path(), &unit.selected, &label, options, color)?;
        if failures == 0 {
            summary.passed += 1;
        } else {
            summary.failed += 1;
        }
    }
    let elapsed = start.elapsed().as_secs_f64();
    let noun = |n| {
        if options.doc {
            if n == 1 {
                "doctest group"
            } else {
                "doctest groups"
            }
        } else if n == 1 {
            "test file"
        } else {
            "test files"
        }
    };
    if summary.failed > 0 {
        println!(
            "\n{red}{bold}{} {} failed{reset}, {} passed in {elapsed:.2}s",
            summary.failed,
            noun(summary.failed),
            summary.passed
        );
    } else {
        println!(
            "\n{green}{bold}{} {} passed{reset} in {elapsed:.2}s",
            summary.passed,
            noun(summary.passed)
        );
    }
    Ok(summary)
}

fn select_cases(plan: &TestPlan, options: &TestOptions, line: Option<usize>) -> Vec<TestCase> {
    // A line inside a case selects that case, even inside a describe. A fixture or
    // describe line selects the cases in its narrowest enclosing fixture scope.
    let direct =
        line.is_some_and(|line| plan.cases.iter().any(|case| case.span.contains_line(line)));
    let scope = line.and_then(|line| {
        plan.cases
            .iter()
            .flat_map(|case| &case.fixtures)
            .filter(|fixture| fixture.scope.contains_line(line))
            .min_by_key(|fixture| fixture.scope.bytes.len())
            .map(|fixture| &fixture.scope)
    });
    plan.cases
        .iter()
        .filter(|case| {
            options.selects(case)
                && line.is_none_or(|line| {
                    if direct {
                        case.span.contains_line(line)
                    } else {
                        scope.is_some_and(|scope| {
                            case.fixtures.iter().any(|fixture| &fixture.scope == scope)
                        })
                    }
                })
        })
        .cloned()
        .collect()
}

struct CaseProcess {
    child: Child,
    cases: Vec<TestCase>,
    directory: tempfile::TempDir,
}

impl Drop for CaseProcess {
    fn drop(&mut self) {
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

fn start_cases(
    bin: &Path,
    tmp: &Path,
    cases: &[TestCase],
    options: &TestOptions,
    color: bool,
) -> Result<CaseProcess, String> {
    let directory = tempfile::tempdir_in(tmp).map_err(|e| e.to_string())?;
    let data = directory.path().join("data");
    std::fs::create_dir(&data).map_err(|e| e.to_string())?;
    let output_file = |name| {
        std::fs::File::create(directory.path().join(name))
            .map(Stdio::from)
            .map_err(|e| e.to_string())
    };
    let ids = format!(
        ",{},",
        cases
            .iter()
            .map(|case| case.id.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    let child = Command::new(bin)
        .env("MESH_TEST_CASES", ids)
        .env("MESH_DATA_DIR", data)
        .env("MESH_TEST_RESULTS", directory.path().join("results"))
        .env("MESH_TEST_QUIET", if options.quiet { "1" } else { "0" })
        .env("MESH_TEST_COLOR", if color { "1" } else { "0" })
        .stdin(Stdio::null())
        .stdout(output_file("stdout")?)
        .stderr(output_file("stderr")?)
        .spawn()
        .map_err(|e| format!("Failed to execute '{}': {e}", bin.display()))?;
    Ok(CaseProcess {
        child,
        cases: cases.to_vec(),
        directory,
    })
}

fn finish_cases(process: &mut CaseProcess, label: &str) -> Result<usize, String> {
    let status = process.child.wait().map_err(|e| e.to_string())?;
    let stdout = read_file(&process.directory.path().join("stdout"))?;
    let stderr = read_file(&process.directory.path().join("stderr"))?;
    print!("{}", String::from_utf8_lossy(&stdout));
    eprint!("{}", String::from_utf8_lossy(&stderr));
    let events =
        std::fs::read_to_string(process.directory.path().join("results")).unwrap_or_default();
    let events: Vec<_> = events.lines().collect();
    let complete = events.len() == process.cases.len() + 1 && events.last() == Some(&"done");
    let mut failed = 0;
    for (index, case) in process.cases.iter().enumerate() {
        if events.get(index) != Some(&"pass") {
            failed += 1;
            let reason = if events.get(index) == Some(&"fail") {
                "FAIL"
            } else {
                "INCOMPLETE"
            };
            println!(
                "{reason}: {label}:{} [case {}] {}",
                case.span.start_line, case.id, case.name
            );
        }
    }
    if !status.success() || !complete {
        if status.code().is_none() {
            println!("CRASHED: {label} ({status})");
        } else if !complete {
            println!("INCOMPLETE: {label} (missing test completion, {status})");
        }
        failed = failed.max(1);
    }
    Ok(failed)
}

fn execute_cases(
    bin: &Path,
    tmp: &Path,
    cases: &[TestCase],
    label: &str,
    options: &TestOptions,
    color: bool,
) -> Result<usize, String> {
    let mut failed = 0;
    let mut offset = 0;
    while offset < cases.len() {
        let asynchronous = cases[offset].policy == TestPolicy::Asynchronous;
        let end = cases[offset..]
            .iter()
            .position(|case| (case.policy == TestPolicy::Asynchronous) != asynchronous)
            .map_or(cases.len(), |n| offset + n);
        if !asynchronous {
            let mut process = start_cases(bin, tmp, &cases[offset..end], options, color)?;
            failed += finish_cases(&mut process, label)?;
        } else {
            let mut active = Vec::new();
            let mut next = offset;
            while next < end || !active.is_empty() {
                while next < end && active.len() < options.test_threads {
                    active.push(start_cases(
                        bin,
                        tmp,
                        &cases[next..next + 1],
                        options,
                        color,
                    )?);
                    next += 1;
                }
                let mut index = 0;
                while index < active.len() {
                    if active[index]
                        .child
                        .try_wait()
                        .map_err(|e| e.to_string())?
                        .is_some()
                    {
                        failed += finish_cases(&mut active.remove(index), label)?;
                    } else {
                        index += 1;
                    }
                }
                if !active.is_empty() {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
        }
        offset = end;
    }
    Ok(failed)
}

// ── Source Preprocessor ───────────────────────────────────────────────────

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
        Path::new(""),
        tmp_dir,
        excluded_entry_relative_path,
    )
}

/// Copies the sources under `project_root/relative_dir`.
fn copy_sources_recursive(
    project_root: &Path,
    relative_dir: &Path,
    tmp_dir: &Path,
    excluded_entry_relative_path: &Path,
) -> Result<(), String> {
    let dir = project_root.join(relative_dir);
    let entries = std::fs::read_dir(&dir)
        .map_err(|e| format!("Failed to read '{}': {}", dir.display(), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("Failed to read '{}': {}", dir.display(), e))?;
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let relative = relative_dir.join(&name);

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
            copy_sources_recursive(
                project_root,
                &relative,
                tmp_dir,
                excluded_entry_relative_path,
            )?;
            continue;
        }
        if !file_type.is_file()
            || path.extension().and_then(|e| e.to_str()) != Some("mpl")
            || name_str.ends_with(".test.mpl")
        {
            continue;
        }
        if let Some(module_name) = name_str.strip_suffix(".test-support.mpl") {
            let module_relative = relative.with_file_name(format!("{module_name}.mpl"));
            let module_path = project_root.join(&module_relative);
            if !module_path.is_file() {
                return Err(format!(
                    "Test-support fragment '{}' requires sibling module '{}'.",
                    path.display(),
                    module_path.display()
                ));
            }
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
        if relative == excluded_entry_relative_path || relative == Path::new(DEFAULT_ENTRYPOINT) {
            continue;
        }
        let dest = tmp_dir.join(&relative);
        let parent = dest.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create '{}': {}", parent.display(), e))?;
        // A module's test-support fragment is appended to it.
        let mut source = read_file(&path)?;
        let module_name = name_str.strip_suffix(".mpl").unwrap_or(&name_str);
        let test_support_path = path.with_file_name(format!("{module_name}.test-support.mpl"));
        if test_support_path.is_file() {
            if !source.ends_with(b"\n") {
                source.push(b'\n');
            }
            source.extend(read_file(&test_support_path)?);
        }
        write_file(&dest, source)?;
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
    discover_recursive_mode(dir, files, false)
}

fn discover_recursive_mode(
    dir: &Path,
    files: &mut Vec<PathBuf>,
    docs: bool,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        // Skip hidden directories (e.g., .planning, .git, target) and build artifacts
        if name_str.starts_with('.') || name_str == "target" {
            continue;
        }
        if entry.file_type()?.is_symlink() {
            return Err(std::io::Error::other(format!(
                "symbolic link '{}' is not a test source",
                path.display()
            )));
        }
        if path.is_dir() {
            discover_recursive_mode(&path, files, docs)?;
        } else if path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| {
                if docs {
                    n.ends_with(".mpl")
                        && !n.ends_with(".test.mpl")
                        && !n.ends_with(".test-support.mpl")
                } else {
                    n.ends_with(".test.mpl")
                }
            })
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
    fn the_palette_is_ansi_only_in_color() {
        assert_eq!(palette(false), ("", "", "", ""));
        assert_eq!(
            palette(true),
            ("\x1b[32m", "\x1b[31m", "\x1b[1m", "\x1b[0m")
        );
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

        let resolved = resolve_project_dir(&test_file).unwrap();

        assert_eq!(resolved, project_dir);
    }

    #[test]
    fn resolve_project_dir_rejects_orphan_test_file_instead_of_falling_back() {
        let temp = tempfile::tempdir().unwrap();
        let orphan = temp.path().join("orphan.test.mpl");
        write_file(&orphan, "test(\"orphan\") do\n  assert(true)\nend\n");

        let err = resolve_project_dir(&orphan).unwrap_err();

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

        let resolved = resolve_test_project(&project_dir).unwrap();

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
    fn copy_project_sources_to_tmp_names_what_it_cannot_read() {
        use std::os::unix::fs::PermissionsExt;

        let project = tempfile::tempdir().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let module = project.path().join("account.mpl");
        write_file(&module, "pub fn value() -> Int do\n  1\nend\n");
        let mode = |path: &Path, mode| {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap()
        };
        mode(&module, 0o000);
        // Permissions do not bind a privileged user.
        if std::fs::read(&module).is_ok() {
            return;
        }
        let err = copy_project_sources_to_tmp(project.path(), tmp.path(), Path::new("main.mpl"))
            .unwrap_err();
        assert!(
            err.starts_with("Failed to read '") && err.contains("account.mpl"),
            "{err}"
        );

        mode(&module, 0o644);
        let support = project.path().join("account.test-support.mpl");
        write_file(&support, "pub fn test_value() -> Int do\n  value()\nend\n");
        mode(&support, 0o000);
        let err = copy_project_sources_to_tmp(project.path(), tmp.path(), Path::new("main.mpl"))
            .unwrap_err();
        assert!(err.contains("account.test-support.mpl"), "{err}");
        mode(&support, 0o644);

        let sealed = project.path().join("sealed");
        std::fs::create_dir(&sealed).unwrap();
        mode(&sealed, 0o000);
        let err = copy_project_sources_to_tmp(project.path(), tmp.path(), Path::new("main.mpl"))
            .unwrap_err();
        mode(&sealed, 0o755);
        assert!(err.contains("sealed"), "{err}");
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

        let test_project = resolve_test_project(&project_dir).unwrap();
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

    /// A source the copy refuses stops the preparation before the manifest
    /// and the test program are written.
    #[test]
    fn prepare_temp_test_project_stops_at_a_source_it_cannot_copy() {
        let project = tempfile::tempdir().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        write_file(
            &project.path().join("mesh.toml"),
            "[package]\nname = \"orphan\"\nversion = \"0.1.0\"\n",
        );
        write_file(
            &project.path().join("missing.test-support.mpl"),
            "pub fn helper() -> Int do\n  42\nend\n",
        );

        let test_project = resolve_test_project(project.path()).unwrap();
        let err = prepare_temp_test_project(&test_project, tmp.path(), "fn main() do\nend\n")
            .unwrap_err();

        assert!(err.contains("missing.test-support.mpl"), "{err}");
        assert!(!tmp.path().join("mesh.toml").exists());
        assert!(!tmp.path().join(DEFAULT_ENTRYPOINT).exists());
    }
}
