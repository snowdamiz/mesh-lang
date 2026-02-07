//! The Snow compiler CLI.
//!
//! Provides the `snowc` command with the following subcommands:
//!
//! - `snowc build <dir>` - Compile a Snow project to a native binary
//! - `snowc init <name>` - Initialize a new Snow project
//! - `snowc deps [dir]` - Resolve and fetch dependencies
//! - `snowc fmt <path>` - Format Snow source files in-place
//! - `snowc lsp` - Start the LSP server (communicates via stdin/stdout)
//!
//! Options:
//! - `--opt-level` - Optimization level (0 = debug, 2 = release)
//! - `--emit-llvm` - Emit LLVM IR (.ll) alongside the binary
//! - `--output` - Output path for the compiled binary
//! - `--target` - Target triple for cross-compilation
//! - `--json` - Output diagnostics as JSON (one object per line)
//! - `--no-color` - Disable colorized output

use std::path::{Path, PathBuf};
use std::process;

use clap::{Parser, Subcommand};

use snow_typeck::diagnostics::DiagnosticOptions;

#[derive(Parser)]
#[command(name = "snowc", version, about = "The Snow compiler")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Compile a Snow project to a native binary
    Build {
        /// Path to the project directory (must contain main.snow)
        dir: PathBuf,

        /// Optimization level (0 = debug, 2 = release)
        #[arg(long = "opt-level", default_value = "0")]
        opt_level: u8,

        /// Emit LLVM IR (.ll file) alongside the binary
        #[arg(long = "emit-llvm")]
        emit_llvm: bool,

        /// Output path for the compiled binary
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Target triple for cross-compilation (e.g., x86_64-unknown-linux-gnu)
        #[arg(long)]
        target: Option<String>,

        /// Output diagnostics as JSON (one object per line) instead of human-readable format
        #[arg(long)]
        json: bool,

        /// Disable colorized output
        #[arg(long = "no-color")]
        no_color: bool,
    },
    /// Initialize a new Snow project
    Init {
        /// Project name (creates directory with this name)
        name: String,
    },
    /// Resolve and fetch dependencies
    Deps {
        /// Project directory (default: current directory)
        #[arg(default_value = ".")]
        dir: PathBuf,
    },
    /// Format Snow source files
    Fmt {
        /// Path to a Snow source file (or directory to format all .snow files)
        path: PathBuf,

        /// Check if files are formatted (exit 1 if not, don't modify)
        #[arg(long)]
        check: bool,

        /// Maximum line width (default: 100)
        #[arg(long = "line-width", default_value = "100")]
        line_width: usize,

        /// Indent size in spaces (default: 2)
        #[arg(long = "indent-size", default_value = "2")]
        indent_size: usize,
    },
    /// Start the LSP server (communicates via stdin/stdout)
    Lsp,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Build {
            dir,
            opt_level,
            emit_llvm,
            output,
            target,
            json,
            no_color,
        } => {
            let diag_opts = DiagnosticOptions {
                color: !no_color && !json,
                json,
            };
            if let Err(e) =
                build(&dir, opt_level, emit_llvm, output.as_deref(), target.as_deref(), &diag_opts)
            {
                if json {
                    // In JSON mode, emit the final error as JSON too.
                    let msg = serde_json::json!({
                        "code": "C0001",
                        "severity": "error",
                        "message": e,
                        "file": "",
                        "spans": [],
                        "fix": null
                    });
                    eprintln!("{}", msg);
                } else {
                    eprintln!("error: {}", e);
                }
                process::exit(1);
            }
        }
        Commands::Init { name } => {
            let dir = std::env::current_dir().unwrap_or_default();
            if let Err(e) = snow_pkg::scaffold_project(&name, &dir) {
                eprintln!("error: {}", e);
                process::exit(1);
            }
        }
        Commands::Deps { dir } => {
            if let Err(e) = deps_command(&dir) {
                eprintln!("error: {}", e);
                process::exit(1);
            }
        }
        Commands::Fmt {
            path,
            check,
            line_width,
            indent_size,
        } => {
            let config = snow_fmt::FormatConfig {
                indent_size,
                max_width: line_width,
            };
            match fmt_command(&path, check, &config) {
                Ok(stats) => {
                    if check {
                        if stats.unformatted > 0 {
                            eprintln!(
                                "{} file(s) would be reformatted",
                                stats.unformatted
                            );
                            process::exit(1);
                        } else {
                            eprintln!("{} file(s) already formatted", stats.total);
                        }
                    } else {
                        eprintln!("Formatted {} file(s)", stats.total);
                    }
                }
                Err(e) => {
                    eprintln!("error: {}", e);
                    process::exit(1);
                }
            }
        }
        Commands::Lsp => {
            let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
            rt.block_on(snow_lsp::run_server());
        }
    }
}

/// Execute the build pipeline: find main.snow -> parse -> typecheck -> codegen -> link.
fn build(
    dir: &Path,
    opt_level: u8,
    emit_llvm: bool,
    output: Option<&Path>,
    target: Option<&str>,
    diag_opts: &DiagnosticOptions,
) -> Result<(), String> {
    // Validate the project directory
    if !dir.exists() {
        return Err(format!(
            "Project directory '{}' does not exist",
            dir.display()
        ));
    }
    if !dir.is_dir() {
        return Err(format!("'{}' is not a directory", dir.display()));
    }

    // Find the entry point: main.snow
    let main_snow = dir.join("main.snow");
    if !main_snow.exists() {
        return Err(format!(
            "No 'main.snow' found in '{}'. Snow projects must have a main.snow entry point.",
            dir.display()
        ));
    }

    // Read the source
    let source = std::fs::read_to_string(&main_snow)
        .map_err(|e| format!("Failed to read '{}': {}", main_snow.display(), e))?;

    // Parse
    let parse = snow_parser::parse(&source);

    // Type check
    let typeck = snow_typeck::check(&parse);

    // Report any errors from parsing or type checking
    let has_errors = report_diagnostics(&source, &main_snow, &parse, &typeck, diag_opts);
    if has_errors {
        return Err("Compilation failed due to errors above.".to_string());
    }

    // Determine output path
    let project_name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("output");
    let output_path = match output {
        Some(p) => p.to_path_buf(),
        None => dir.join(project_name),
    };

    // Emit LLVM IR if requested
    if emit_llvm {
        let ll_path = output_path.with_extension("ll");
        snow_codegen::compile_to_llvm_ir(&parse, &typeck, &ll_path, target)?;
        eprintln!("  LLVM IR: {}", ll_path.display());
    }

    // Compile to native binary
    snow_codegen::compile_to_binary(&parse, &typeck, &output_path, opt_level, target, None)?;

    eprintln!("  Compiled: {}", output_path.display());

    Ok(())
}

/// Report parse and type-check diagnostics.
///
/// When `diag_opts.json` is true, outputs one JSON object per line to stderr.
/// Otherwise, outputs colorized (or colorless) human-readable diagnostics.
/// Returns true if there are any errors.
fn report_diagnostics(
    source: &str,
    path: &Path,
    parse: &snow_parser::Parse,
    typeck: &snow_typeck::TypeckResult,
    diag_opts: &DiagnosticOptions,
) -> bool {
    let file_name = path.display().to_string();
    let mut has_errors = false;

    // Check for parse errors
    for error in parse.errors() {
        has_errors = true;
        if diag_opts.json {
            // Emit parse errors as JSON.
            let start = error.span.start as usize;
            let end = (error.span.end as usize).max(start + 1);
            let json_diag = serde_json::json!({
                "code": "P0001",
                "severity": "error",
                "message": format!("Parse error: {}", error.message),
                "file": file_name,
                "spans": [{
                    "start": start,
                    "end": end,
                    "label": error.message
                }],
                "fix": null
            });
            eprintln!("{}", json_diag);
        } else {
            use ariadne::{Config, Label, Report, ReportKind, Source};
            let config = if diag_opts.color {
                Config::default()
            } else {
                Config::default().with_color(false)
            };
            let start = error.span.start as usize;
            let end = (error.span.end as usize).max(start + 1);
            let _ = Report::<std::ops::Range<usize>>::build(ReportKind::Error, start..end)
                .with_message("Parse error")
                .with_config(config)
                .with_label(Label::new(start..end).with_message(&error.message))
                .finish()
                .eprint(Source::from(source));
        }
    }

    // Check for type errors
    for error in &typeck.errors {
        has_errors = true;
        let rendered = snow_typeck::diagnostics::render_diagnostic(
            error, source, &file_name, diag_opts, None,
        );
        eprint!("{}", rendered);
    }

    has_errors
}

// ── Deps subcommand ──────────────────────────────────────────────────

/// Execute the `deps` subcommand: resolve dependencies and generate snow.lock.
///
/// If snow.lock already exists and the manifest hasn't changed, skips resolution.
fn deps_command(dir: &Path) -> Result<(), String> {
    let manifest_path = dir.join("snow.toml");
    if !manifest_path.exists() {
        return Err(format!(
            "No 'snow.toml' found in '{}'. Run `snowc init` to create a project.",
            dir.display()
        ));
    }

    let lock_path = dir.join("snow.lock");

    // Check if lockfile is fresh: exists and manifest hasn't been modified after it
    if lock_path.exists() {
        let manifest_modified = std::fs::metadata(&manifest_path)
            .and_then(|m| m.modified())
            .ok();
        let lock_modified = std::fs::metadata(&lock_path)
            .and_then(|m| m.modified())
            .ok();
        if let (Some(manifest_time), Some(lock_time)) = (manifest_modified, lock_modified) {
            if manifest_time <= lock_time {
                eprintln!("Dependencies up to date");
                return Ok(());
            }
        }
    }

    let (resolved, lockfile) = snow_pkg::resolve_dependencies(dir)?;

    lockfile.write(&lock_path)?;

    if resolved.is_empty() {
        eprintln!("No dependencies");
    } else {
        eprintln!("Resolved {} dependencies", resolved.len());
    }

    Ok(())
}

// ── Format subcommand ─────────────────────────────────────────────────

/// Statistics from a format operation.
struct FmtStats {
    /// Total number of files processed.
    total: usize,
    /// Number of files that were not already formatted (check mode).
    unformatted: usize,
}

/// Execute the `fmt` subcommand: format Snow source files in-place or check formatting.
fn fmt_command(
    path: &Path,
    check: bool,
    config: &snow_fmt::FormatConfig,
) -> Result<FmtStats, String> {
    let files = collect_snow_files(path)?;
    if files.is_empty() {
        return Err(format!(
            "No .snow files found at '{}'",
            path.display()
        ));
    }

    let mut total = 0;
    let mut unformatted = 0;

    for file in &files {
        let source = std::fs::read_to_string(file)
            .map_err(|e| format!("Failed to read '{}': {}", file.display(), e))?;

        let formatted = snow_fmt::format_source(&source, config);
        total += 1;

        if formatted != source {
            if check {
                eprintln!("  would reformat: {}", file.display());
                unformatted += 1;
            } else {
                std::fs::write(file, &formatted)
                    .map_err(|e| format!("Failed to write '{}': {}", file.display(), e))?;
            }
        }
    }

    Ok(FmtStats { total, unformatted })
}

/// Collect `.snow` files from a path. If the path is a file, return it directly.
/// If it is a directory, recursively find all `.snow` files.
fn collect_snow_files(path: &Path) -> Result<Vec<PathBuf>, String> {
    if !path.exists() {
        return Err(format!("Path '{}' does not exist", path.display()));
    }

    if path.is_file() {
        if path.extension().and_then(|e| e.to_str()) == Some("snow") {
            return Ok(vec![path.to_path_buf()]);
        } else {
            return Err(format!(
                "'{}' is not a .snow file",
                path.display()
            ));
        }
    }

    if path.is_dir() {
        let mut files = Vec::new();
        collect_snow_files_recursive(path, &mut files)
            .map_err(|e| format!("Failed to walk directory '{}': {}", path.display(), e))?;
        files.sort();
        return Ok(files);
    }

    Err(format!("'{}' is not a file or directory", path.display()))
}

/// Recursively collect `.snow` files from a directory.
fn collect_snow_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let entry_path = entry.path();
        if entry_path.is_dir() {
            collect_snow_files_recursive(&entry_path, files)?;
        } else if entry_path.extension().and_then(|e| e.to_str()) == Some("snow") {
            files.push(entry_path);
        }
    }
    Ok(())
}
