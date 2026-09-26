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

use mesh_parser::test_harness::preprocess_test_source;
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

fn read_file(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("Failed to read '{}': {e}", path.display()))
}

fn write_file(path: &Path, contents: impl AsRef<[u8]>) -> Result<(), String> {
    std::fs::write(path, contents).map_err(|e| format!("Failed to write '{}': {e}", path.display()))
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

    let target = resolve_target_path(target)?;
    let test_project = resolve_test_project(&target)?;
    let project_dir = &test_project.project_dir;
    let test_files = resolve_test_files(&target)?;

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
    let color = use_color();
    let (green, red, bold, reset) = palette(color);

    for test_file in &test_files {
        // The project root is an ancestor of every test file.
        let label = test_file
            .strip_prefix(project_dir)
            .unwrap_or(test_file)
            .display()
            .to_string();

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
            color,
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
            .env("MESH_TEST_COLOR", if color { "1" } else { "0" })
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
            // A binary killed by a signal reported nothing itself, and lost
            // the output it had buffered: say what happened to it.
            if output.status.code().is_none() {
                println!("{red}{bold}CRASHED{reset}: {label} ({})", output.status);
            }
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
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create '{}': {}", parent.display(), e))?;
        }
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
}
