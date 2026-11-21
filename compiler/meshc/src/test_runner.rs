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

use crate::{ReportResult, ReportText};
use mesh_diagnostics::{Code, Diagnostic, Failed, Lookup, Reporter, Severity, SourceFile};

use mesh_parser::test_harness::{preprocess_test_source, TestProgram};
use mesh_pkg::frontend::{self, CheckOptions, Target};
use mesh_pkg::manifest::{
    resolve_entrypoint, rewrite_test_manifest_source, Manifest, DEFAULT_ENTRYPOINT,
};

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

/// The target as an absolute path; without one, the current directory.
fn resolve_target_path(target: Option<&Path>) -> Result<PathBuf, Diagnostic> {
    let target = target.unwrap_or(Path::new("."));
    let unavailable = |message| Diagnostic::new(Code::PathNotFound, message);
    let abs = std::path::absolute(target)
        .map_err(|e| unavailable(format!("Failed to resolve '{}': {e}", target.display())))?;
    if abs.exists() {
        Ok(abs)
    } else {
        Err(unavailable(format!(
            "Test target '{}' does not exist",
            abs.display()
        )))
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

fn resolve_test_project(target: &Path) -> Result<ResolvedTestProject, Diagnostic> {
    let project_dir = resolve_project_dir(target).map_err(|message| {
        Diagnostic::in_file(
            Code::ManifestUnreadable,
            &SourceFile::unread(target),
            message,
        )
    })?;
    let manifest_path = project_dir.join("mesh.toml");
    let (manifest, manifest_source) = Manifest::from_file_with_source(&manifest_path)?;
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

pub(crate) fn is_test_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".test.mpl"))
}

fn resolve_test_files(target: &Path) -> Result<Vec<PathBuf>, Diagnostic> {
    if target.is_dir() {
        discover_test_files(target).map_err(|message| Diagnostic::new(Code::PathNotFound, message))
    } else if is_test_file(target) {
        Ok(vec![target.to_path_buf()])
    } else {
        Err(Diagnostic::new(
            Code::InvalidArguments,
            format!(
                "'{}' is not a directory or a *.test.mpl file",
                target.display()
            ),
        ))
    }
}

/// The project's sources in `tmp_dir`, its entry replaced by the test
/// program as `main.mpl`, and a manifest naming that entry.
fn prepare_temp_test_project(
    test_project: &ResolvedTestProject,
    tmp_dir: &Path,
    preprocessed_source: &str,
) -> Result<(), Diagnostic> {
    let staging_error = |message| {
        Diagnostic::in_file(
            Code::ModuleUnreadable,
            &SourceFile::unread(&test_project.project_dir),
            message,
        )
        .with_repair("Restore access to the source and temporary directories used by meshc test.")
    };
    copy_project_sources_to_tmp(
        &test_project.project_dir,
        tmp_dir,
        &test_project.entry_relative_path,
    )
    .map_err(staging_error)?;
    let manifest_source =
        rewrite_test_manifest_source(&test_project.manifest_source, &test_project.project_dir)?;
    write_file(&tmp_dir.join("mesh.toml"), manifest_source).map_err(staging_error)?;
    write_file(&tmp_dir.join(DEFAULT_ENTRYPOINT), preprocessed_source).map_err(staging_error)
}

fn read_file(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("Failed to read '{}': {e}", path.display()))
}

fn write_file(path: &Path, contents: impl AsRef<[u8]>) -> Result<(), String> {
    std::fs::write(path, contents).map_err(|e| format!("Failed to write '{}': {e}", path.display()))
}

fn with_test_project<T>(
    test_project: &ResolvedTestProject,
    test_file: &Path,
    report_problem: impl Fn(&str),
    reporter: &mut Reporter,
    run: impl FnOnce(&Path, &mut Reporter) -> Result<T, Failed>,
) -> Result<T, Failed> {
    let source = mesh_pkg::project::read_file(test_file).reported(reporter)?;
    let written = SourceFile::new(test_file, source.as_str());
    let TestProgram { text, map } = preprocess_test_source(&source).map_err(|error| {
        report_problem("COMPILE ERROR");
        reporter.fail(error.to_diagnostic(&written))
    })?;
    let temporary = tempfile::tempdir()
        .map_err(|e| format!("Failed to create temp dir: {e}"))
        .reported_as(Code::PathNotFound, reporter)?;
    let root = temporary.path();
    let program = root.join(DEFAULT_ENTRYPOINT);
    reporter.with_path_aliases(
        [
            (program.clone(), test_file.to_path_buf()),
            (root.to_path_buf(), test_project.project_dir.clone()),
        ],
        |reporter| {
            reporter.with_generated_file(program, written, map, |reporter| {
                prepare_temp_test_project(test_project, root, &text)
                    .inspect_err(|_| report_problem("SETUP ERROR"))
                    .reported(reporter)?;
                run(root, reporter)
            })
        },
    )
}

pub(crate) fn check_tests(target: &Path, reporter: &mut Reporter) -> Result<(), Failed> {
    let target = resolve_target_path(Some(target)).reported(reporter)?;
    let tests = resolve_test_files(&target).reported(reporter)?;
    if tests.is_empty() {
        return Ok(());
    }
    let project = resolve_test_project(&target).reported(reporter)?;
    let mut failure = None;
    for file in tests {
        if let Err(error) = with_test_project(
            &project,
            &file,
            |_| {},
            reporter,
            |root, reporter| {
                let options = CheckOptions {
                    test_builtins: true,
                    require_entrypoint: true,
                    ..CheckOptions::cli()
                };
                frontend::check(Target::Project { root }, &options)
                    .report(reporter)
                    .map(|_| ())
            },
        ) {
            failure = Some(error);
        }
    }
    failure.map_or(Ok(()), Err)
}

/// Run tests from a project root, test directory, or specific *.test.mpl file.
///
/// Each test program reports its diagnostics (a failed assertion's `T`
/// code, a panic's `R` code) as JSON lines on its stderr; they are reported
/// here, naming the test file as written, in the format the command was
/// asked for. Any other line a program writes there is its own output.
pub fn run_tests(
    target: Option<&Path>,
    quiet: bool,
    coverage: bool,
    json: bool,
    no_color: bool,
    reporter: &mut Reporter,
) -> Result<(), Failed> {
    if coverage {
        return Err(
            "coverage reporting is not implemented for `meshc test`; run the command without --coverage"
                .to_string(),
        ).reported_as(Code::InvalidArguments, reporter);
    }

    let target = resolve_target_path(target).reported(reporter)?;
    let test_project = resolve_test_project(&target).reported(reporter)?;
    let test_files = resolve_test_files(&target).reported(reporter)?;

    if test_files.is_empty() {
        return Err(reporter.fail(Diagnostic::in_file(
            Code::NoTests,
            &SourceFile::unread(&target),
            format!("no *.test.mpl files found under '{}'", target.display()),
        )));
    }

    let start = Instant::now();
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut failure = None;
    let color = !json && !no_color && use_color();
    let (green, red, bold, reset) = palette(color);

    for test_file in &test_files {
        let label = test_file
            .strip_prefix(&test_project.project_dir)
            .unwrap_or(test_file)
            .display();
        let report_problem = |kind: &str| println!("{red}{bold}{kind}{reset}: {label}");
        let result = with_test_project(
            &test_project,
            test_file,
            report_problem,
            reporter,
            |root, reporter| {
                let bin_path = root.join("test_bin");
                crate::build(
                    root,
                    0,
                    false,
                    Some(&bin_path),
                    None,
                    crate::BuildArtifact::Executable,
                    true,
                    reporter,
                )
                .inspect_err(|_| report_problem("COMPILE ERROR"))?;

                let output = Command::new(&bin_path)
                    .env("MESH_TEST_QUIET", if quiet { "1" } else { "0" })
                    .env("MESH_TEST_COLOR", if color { "1" } else { "0" })
                    .env("MESH_DIAGNOSTICS", "json")
                    .output()
                    .map_err(|e| format!("Failed to execute test file '{label}': {e}"))
                    .reported_as(Code::PathNotFound, reporter)?;

                print!("{}", String::from_utf8_lossy(&output.stdout));
                // The proof of the first failure that ends a test: a failed
                // assertion, a panic, a stack overflow. An error a test
                // handled (a `Node.connect` that failed) explains nothing.
                let mut ended = None;
                for line in String::from_utf8_lossy(&output.stderr).lines() {
                    match runtime_diagnostic(line, root) {
                        Some(diagnostic) if diagnostic.severity() == Severity::Error => {
                            let ends_a_test = ends_a_test(diagnostic.code());
                            let failed = reporter.fail(diagnostic);
                            if ends_a_test {
                                ended.get_or_insert(failed);
                            }
                        }
                        Some(diagnostic) => reporter.emit(diagnostic),
                        None if json => println!("{line}"),
                        None => eprintln!("{line}"),
                    }
                }

                if output.status.success() {
                    return Ok(());
                }
                let killed = output.status.code().is_none();
                if killed {
                    println!("{red}{bold}CRASHED{reset}: {label} ({})", output.status);
                }
                // A failure is reported as it happens, a stack overflow's
                // too; a program that ended without reporting one crashed.
                if let Some(failed) = ended {
                    return Err(failed);
                }
                let how = if killed {
                    "was killed"
                } else {
                    "ended without reporting a failure"
                };
                Err(reporter.fail(Diagnostic::new(
                    Code::TestPanicked,
                    format!("the test program for '{label}' {how} ({})", output.status),
                )))
            },
        );
        match result {
            Ok(()) => passed += 1,
            Err(error) => {
                failed += 1;
                failure = Some(error);
            }
        }
    }

    let elapsed_secs = start.elapsed().as_secs_f64();
    let files = |n: usize| if n == 1 { "test file" } else { "test files" };
    if let Some(failure) = failure {
        println!(
            "\n{red}{bold}{failed} {} failed{reset}, {passed} passed in {elapsed_secs:.2}s",
            files(failed)
        );
        Err(failure)
    } else {
        println!(
            "\n{green}{bold}{passed} {} passed{reset} in {elapsed_secs:.2}s",
            files(passed)
        );
        Ok(())
    }
}

/// Whether a test program reporting `code` has a test that failed by it,
/// which explains the program failing.
fn ends_a_test(code: Code) -> bool {
    matches!(
        code,
        Code::Panic
            | Code::StackOverflow
            | Code::AssertFailed
            | Code::AssertEqFailed
            | Code::AssertNeFailed
            | Code::AssertRaisesFailed
            | Code::AssertReceiveFailed
            | Code::TestPanicked
    )
}

/// The diagnostic one line of a test program's stderr is: a JSON object
/// with a catalog code, as `MESH_DIAGNOSTICS=json` has the runtime write
/// one. A file it names relative to the program's build is under `root`.
fn runtime_diagnostic(line: &str, root: &Path) -> Option<Diagnostic> {
    let wire: serde_json::Value = serde_json::from_str(line).ok()?;
    let id = wire["code"].as_str()?;
    let Lookup::Found(code) = Code::lookup(id) else {
        return None;
    };
    if code.id() != id {
        return None;
    }
    let message = wire["message"].as_str()?;
    let file = wire["file"]
        .as_str()
        .filter(|file| !file.is_empty())
        .map(|file| SourceFile::unread(root.join(file)));
    let line = wire["line"]
        .as_u64()
        .and_then(|line| u32::try_from(line).ok());
    let mut diagnostic = match (file, line) {
        (Some(file), Some(line)) if line > 0 => {
            let at = Diagnostic::at_line(code, file, line, message);
            match wire["col"].as_u64().and_then(|col| u32::try_from(col).ok()) {
                Some(col) => at.at_column(col),
                None => at,
            }
        }
        (Some(file), _) => Diagnostic::in_file(code, &file, message),
        (None, _) => Diagnostic::new(code, message),
    };
    if let Some(fix) = wire["fix"].as_str().filter(|fix| !fix.is_empty()) {
        diagnostic = diagnostic.with_repair(fix.to_string());
    }
    for note in wire["notes"].as_array().into_iter().flatten() {
        diagnostic = diagnostic.with_note(note.as_str()?);
    }
    Some(diagnostic)
}

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

    /// A diagnostic a test program wrote as JSON is the diagnostic again,
    /// its file under the program's build root; any other line is output.
    #[test]
    fn a_test_program_diagnostic_is_read_back_from_its_json_line() {
        let written = Diagnostic::at_line(
            Code::AssertEqFailed,
            SourceFile::unread("main.mpl"),
            6,
            "assert_eq failed: 1 == 2",
        )
        .at_column(3)
        .with_note("a note");
        let mut line = Vec::new();
        Reporter::new(mesh_diagnostics::Format::Json, &mut line).emit(written);
        let line = String::from_utf8(line).unwrap();

        let read = runtime_diagnostic(line.trim_end(), Path::new("/build")).unwrap();
        assert_eq!(read.code(), Code::AssertEqFailed);
        assert_eq!(read.message(), "assert_eq failed: 1 == 2");
        assert_eq!(read.file().unwrap().path(), Path::new("/build/main.mpl"));
        assert_eq!((read.line(), read.col()), (Some(6), Some(3)));
        assert_eq!(read.repair(), Code::AssertEqFailed.info().repair);
        assert_eq!(read.notes(), ["a note"]);

        for other in [
            "noise",
            "{\"code\":\"Z9999\",\"message\":\"x\"}",
            "{\"code\":\"assert-eq-failed\",\"message\":\"a name, not an ID\"}",
            "{\"code\":\"T0002\"}",
        ] {
            assert!(
                runtime_diagnostic(other, Path::new("/build")).is_none(),
                "{other}"
            );
        }
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

        assert_eq!(err.code().id(), "M0202");
        assert!(
            err.message().contains("missing.test-support.mpl"),
            "{err:?}"
        );
        assert!(!tmp.path().join("mesh.toml").exists());
        assert!(!tmp.path().join(DEFAULT_ENTRYPOINT).exists());
    }
}
