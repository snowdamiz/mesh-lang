//! The Snow compiler CLI.
//!
//! Provides the `snowc` command with the following subcommands:
//!
//! - `snowc build <dir>` - Compile a Snow project to a native binary
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
