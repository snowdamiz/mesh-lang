mod auth;
mod install;
mod publish;
mod registry;
mod search;

use std::path::PathBuf;
use std::process;

use clap::{Parser, Subcommand};
use colored::Colorize;
use mesh_diagnostics::{Code, Diagnostic, Format, Reporter};

const DEFAULT_REGISTRY: &str = "https://api.packages.meshlang.dev";

#[derive(Parser)]
#[command(name = "meshpkg", version, about = "Mesh package manager")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Output in JSON format (suppresses spinners and colored output)
    #[arg(long, global = true)]
    json: bool,

    /// Disable colorized output
    #[arg(long, global = true)]
    no_color: bool,
}

impl Cli {
    fn parse_or_report() -> Self {
        Self::try_parse().unwrap_or_else(|error| {
            let json = std::env::args_os()
                .skip(1)
                .take_while(|arg| arg != "--")
                .any(|arg| arg == "--json");
            if !json || !error.use_stderr() {
                error.exit();
            }
            let mut reporter = Reporter::stderr(Format::Json);
            reporter.emit(Diagnostic::new(
                Code::InvalidArguments,
                error.to_string().trim(),
            ));
            reporter.finish();
            process::exit(error.exit_code());
        })
    }
}

#[derive(Subcommand)]
enum Commands {
    /// Authenticate with the registry
    Login {
        /// Auth token (if not provided, prompted from stdin)
        #[arg(long)]
        token: Option<String>,
    },
    /// Publish this package to the registry
    Publish {
        /// Registry URL
        #[arg(long, default_value = DEFAULT_REGISTRY)]
        registry: String,
    },
    /// Install a package or all dependencies from mesh.toml
    Install {
        /// Package name to install (omit to install all from mesh.toml)
        name: Option<String>,
        /// Registry URL
        #[arg(long, default_value = DEFAULT_REGISTRY)]
        registry: String,
    },
    /// Search for packages in the registry
    Search {
        /// Search query
        query: String,
        /// Registry URL
        #[arg(long, default_value = DEFAULT_REGISTRY)]
        registry: String,
    },
    /// Refresh installed meshc and meshpkg through the canonical installer path
    Update,
}

fn main() {
    let cli = Cli::parse_or_report();
    if cli.no_color || cli.json {
        colored::control::set_override(false);
    }
    let mut reporter = Reporter::stderr(Format::for_cli(cli.json, cli.no_color));
    let json_mode = cli.json;

    let result = match cli.command {
        Commands::Login { token } => run_login(token, json_mode),
        Commands::Publish { registry } => publish::run(&PathBuf::from("."), &registry, json_mode),
        Commands::Install { name, registry } => {
            install::run(&PathBuf::from("."), name.as_deref(), &registry, json_mode)
        }
        Commands::Search { query, registry } => search::run(&query, &registry, json_mode),
        Commands::Update => run_update(json_mode),
    };

    if let Err(error) = result {
        reporter.emit(error);
    }
    process::exit(reporter.finish().exit_code());
}

fn run_update(json_mode: bool) -> Result<(), Diagnostic> {
    if json_mode {
        return Err(Diagnostic::new(
            Code::InvalidArguments,
            "installer-backed self-update does not support --json",
        )
        .with_repair("Rerun `meshpkg update` without --json."));
    }

    let outcome = mesh_pkg::run_toolchain_update().map_err(|error| error.to_diagnostic())?;
    println!("{}", outcome.mode);
    Ok(())
}

fn run_login(token: Option<String>, json_mode: bool) -> Result<(), Diagnostic> {
    let token = match token {
        Some(t) => t,
        None => {
            // Read from stdin interactively
            if !json_mode {
                eprint!("Enter registry token: ");
            }
            let mut input = String::new();
            std::io::stdin().read_line(&mut input).map_err(|e| {
                Diagnostic::new(
                    Code::RegistryAuthFailed,
                    format!("Failed to read token: {e}"),
                )
            })?;
            input.trim().to_string()
        }
    };

    if token.is_empty() {
        return Err(Diagnostic::new(
            Code::RegistryAuthFailed,
            "Token cannot be empty",
        ));
    }

    auth::write_token(&token).map_err(|message| Diagnostic::new(Code::PathNotFound, message))?;

    if json_mode {
        println!(
            "{}",
            serde_json::json!({ "status": "ok", "message": "Token saved" })
        );
    } else {
        println!(
            "{} Token saved to {}",
            "✓".green().bold(),
            auth::credentials_path().display()
        );
    }
    Ok(())
}
