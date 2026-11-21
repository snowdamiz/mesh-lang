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
    /// The command line. Help and `--version` print to stdout and exit 0;
    /// a bad command line is an `X0001` in the format asked for, exiting 2.
    /// Run without a command, the tool shows a person its help instead.
    fn parse_or_report() -> Self {
        Self::try_parse().unwrap_or_else(|error| {
            let given = |flag: &str| {
                std::env::args_os()
                    .skip(1)
                    .take_while(|arg| arg != "--")
                    .any(|arg| arg == flag)
            };
            let json = given("--json");
            let wants_help =
                error.kind() == clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand;
            if !error.use_stderr() || (wants_help && !json) {
                error.exit();
            }
            let mut reporter = Reporter::stderr(Format::for_cli(json, given("--no-color")));
            reporter.emit(argument_error(&error));
            reporter.finish();
            process::exit(error.exit_code());
        })
    }
}

/// Clap's report of a bad command line as an `X0001`: its first paragraph
/// is the message, and each paragraph after it (a tip, the usage) a note.
fn argument_error(error: &clap::Error) -> Diagnostic {
    let text = error.to_string();
    let mut paragraphs = text
        .split("\n\n")
        .map(str::trim)
        .filter(|paragraph| !paragraph.is_empty());
    let message = paragraphs.next().unwrap_or_default();
    let message = message.strip_prefix("error: ").unwrap_or(message);
    paragraphs.fold(
        Diagnostic::new(Code::InvalidArguments, message),
        Diagnostic::with_note,
    )
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
