//! The Mesh compiler CLI.
//!
//! Provides the `meshc` command with the following subcommands:
//!
//! - `meshc build <dir>` - Compile a Mesh project to a native binary
//! - `meshc check [path] [--tests]` - Check Mesh source without building a binary
//! - `meshc init [--clustered] [--template <name>] [--db <sqlite|postgres>] <name>` - Initialize a new Mesh project (`todo-api` defaults to local SQLite; `--db postgres` opts into the clustered/deployable starter)
//! - `meshc cluster <status|continuity|diagnostics> ...` - Inspect runtime-owned clustered operator surfaces
//! - `meshc deps [dir]` - Resolve and fetch dependencies
//! - `meshc update` - Refresh installed `meshc` and `meshpkg` through the canonical installer path
//! - `meshc guide [topic]` - Read the guide embedded in this compiler
//! - `meshc explain [CODE] [--json]` - Look up a diagnostic or list the catalog
//! - `meshc skills setup` - Install this compiler's Mesh skill for coding tools
//! - `meshc fmt <path>` - Format Mesh source files in-place
//! - `meshc lint [path]` - Report code that compiles but should be written differently
//! - `meshc test [path]` - Run *.test.mpl files from a project root, tests directory, or specific test file
//! - `meshc verify [dir]` - Verify pure policy laws in a Mesh project
//! - `meshc migrate [up|down|status|generate]` - Database migration management
//! - `meshc plaintext-report [dir] [--output PATH] [--check PATH]` - List every `declassify` and `@display` export
//! - `meshc repl` - Start an interactive REPL with LLVM JIT
//! - `meshc lsp` - Start the LSP server (communicates via stdin/stdout)
//!
//! Options:
//! - `--opt-level` - Optimization level (0 = debug, 2 = release)
//! - `--emit-llvm` - Emit LLVM IR (.ll) alongside the binary
//! - `--output` - Output path for the compiled binary
//! - `--target` - Target triple for cross-compilation
//! - `--artifact` - Build an executable, static library, or dynamic library
//! - `--json` - Output diagnostics as JSON (one object per line)
//! - `--no-color` - Disable colorized output

// The build pipeline threads its state through many parameters, and some
// intermediate shapes are spelled out where they are built.
#![allow(clippy::too_many_arguments, clippy::type_complexity)]

#[cfg(target_env = "musl")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod cluster;
mod explain;
mod guide;
mod library_bindings;
mod migrate;
mod plaintext_report;
mod proof;
mod proof_gates;
mod skills;
mod test_runner;
mod verify;
mod verify_parametric;
mod verify_process;
mod verify_proof;
mod verify_solver;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process;

use clap::{Parser, Subcommand, ValueEnum};
use mesh_parser::ast::expr::{FieldAccess, NameRef};
use mesh_parser::ast::AstNode;
use mesh_pkg::frontend::{self, Analysis, CheckOptions, Target};
use mesh_pkg::manifest::ClusteredExecutionMetadata;

use mesh_diagnostics::{Code, Diagnostic, Failed, Format, Reporter, Severity, SourceFile};
use mesh_typeck::ty::Ty;

#[derive(Parser)]
#[command(name = "meshc", version, about = "The Mesh compiler")]
struct Cli {
    /// Output diagnostics as NDJSON and use JSON for commands with a JSON product format
    #[arg(long, global = true)]
    pub json: bool,

    /// Disable colorized output
    #[arg(long, global = true)]
    pub no_color: bool,

    #[command(subcommand)]
    command: Commands,
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
    /// Read the guide embedded in this compiler
    Guide {
        /// Topic name, such as syntax or testing
        topic: Option<String>,
    },
    /// Look up a diagnostic or list the catalog
    Explain {
        /// Diagnostic code or name
        code: Option<String>,
    },
    /// Manage the Mesh skill for coding tools
    Skills {
        #[command(subcommand)]
        action: skills::SkillsCommand,
    },
    /// List or verify the project's pure policy laws
    Verify {
        /// Project directory
        #[arg(default_value = ".")]
        dir: PathBuf,

        /// List the resolved laws without running a solver
        #[arg(long)]
        list: bool,

        /// Require the candidate to retain each law in a trusted inventory
        #[arg(long, conflicts_with = "list")]
        preserve: Option<PathBuf>,

        /// Write checked proof artifacts to this directory for independent replay
        #[arg(long, conflicts_with = "list")]
        emit_proof: Option<PathBuf>,

        /// Recheck Mesh proof terms with an independent kernel
        #[arg(long, conflicts_with = "list")]
        verdict: bool,
    },
    /// Compile a Mesh project to a native binary
    Build {
        /// Project directory with an entrypoint, or a standalone Mesh source file
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

        /// Artifact kind
        #[arg(long, value_enum, default_value_t = BuildArtifact::Executable)]
        artifact: BuildArtifact,

        /// Write the plaintext report (every `declassify` and `@display` export) here
        #[arg(long = "plaintext-report")]
        plaintext_report: Option<PathBuf>,
    },
    /// Parse and type-check a Mesh project or source file
    Check {
        /// Project directory or standalone Mesh source file
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Also check the project's *.test.mpl files
        #[arg(long)]
        tests: bool,
    },
    /// List every `declassify` and `@display` export of a project, as JSON
    PlaintextReport {
        /// Project directory (default: current directory)
        #[arg(default_value = ".")]
        dir: PathBuf,

        /// Write the report here instead of printing it
        #[arg(long)]
        output: Option<PathBuf>,

        /// Fail if the report differs from this committed one (line numbers aside)
        #[arg(long)]
        check: Option<PathBuf>,
    },
    /// Initialize a new Mesh project
    Init {
        /// Generate the minimal clustered app scaffold instead of the hello-world app
        #[arg(long)]
        clustered: bool,

        /// Generate a named starter template (currently: todo-api; default SQLite is local-only)
        #[arg(long)]
        template: Option<String>,

        /// Select the todo-api database backend (sqlite = local default, postgres = clustered/deployable)
        #[arg(long, value_enum)]
        db: Option<InitTodoDb>,

        /// Project name (creates directory with this name)
        name: String,
    },
    /// Inspect runtime-owned clustered operator surfaces
    Cluster {
        #[command(subcommand)]
        action: cluster::ClusterCommand,
    },
    /// Run repository-owned production proof scenarios.
    Proof {
        #[command(subcommand)]
        action: proof::ProofCommand,
    },
    /// Resolve and fetch dependencies
    Deps {
        /// Project directory (default: current directory)
        #[arg(default_value = ".")]
        dir: PathBuf,
    },
    /// Refresh installed meshc and meshpkg through the canonical installer path
    Update,
    /// Format Mesh source files
    Fmt {
        /// Path to a Mesh source file (or directory to format all .mpl files)
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
    /// Lint Mesh source files (exit 1 if anything is reported)
    Lint {
        /// Path to a Mesh source file or directory (default: current directory)
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Start an interactive REPL with LLVM JIT compilation
    Repl,
    /// Start the LSP server (communicates via stdin/stdout)
    Lsp,
    /// Run test files (*.test.mpl) from a project root, tests directory, or specific test file
    Test {
        /// Path to a Mesh project, test directory, or specific *.test.mpl file (default: current directory)
        path: Option<PathBuf>,

        /// Show dots instead of test names (compact output)
        #[arg(long)]
        quiet: bool,

        /// Request coverage reporting (currently unsupported; exits with an error)
        #[arg(long)]
        coverage: bool,
    },
    /// Run database migrations
    Migrate {
        #[command(subcommand)]
        action: Option<MigrateAction>,

        /// Project directory (default: current directory)
        #[arg(default_value = ".")]
        dir: PathBuf,
    },
}

#[derive(Subcommand)]
enum MigrateAction {
    /// Apply all pending migrations (default)
    Up,
    /// Rollback the last applied migration
    Down,
    /// Show migration status (applied vs pending)
    Status,
    /// Generate a new migration scaffold
    Generate {
        /// Migration name (e.g., "create_users")
        name: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum InitTodoDb {
    Sqlite,
    Postgres,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
enum BuildArtifact {
    #[default]
    Executable,
    Staticlib,
    Cdylib,
}

impl std::fmt::Display for BuildArtifact {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Executable => "executable",
            Self::Staticlib => "staticlib",
            Self::Cdylib => "cdylib",
        })
    }
}

impl From<InitTodoDb> for mesh_pkg::TodoApiDatabase {
    fn from(value: InitTodoDb) -> Self {
        match value {
            InitTodoDb::Sqlite => mesh_pkg::TodoApiDatabase::Sqlite,
            InitTodoDb::Postgres => mesh_pkg::TodoApiDatabase::Postgres,
        }
    }
}

enum InitTarget {
    HelloWorld,
    Clustered,
    TodoApi(mesh_pkg::TodoApiDatabase),
}

fn resolve_init_target(
    clustered: bool,
    template: Option<&str>,
    db: Option<InitTodoDb>,
) -> Result<InitTarget, String> {
    match (template, db) {
        (Some("todo-api"), _) if clustered => Err(
            "`meshc init --clustered` cannot be combined with `--template todo-api` or `--db`; use `meshc init --template todo-api <name>` for the local SQLite starter, `meshc init --template todo-api --db postgres <name>` for the clustered/deployable Todo starter, or `meshc init --clustered <name>` for the minimal clustered scaffold."
                .to_string(),
        ),
        (Some("todo-api"), db) => Ok(InitTarget::TodoApi(
            db.map_or(mesh_pkg::TodoApiDatabase::Sqlite, Into::into),
        )),
        (Some(template_name), db) => {
            let db_guidance = if db.is_some() {
                " `--db` is only supported with `--template todo-api`."
            } else {
                ""
            };
            Err(format!(
                "unknown init template '{template_name}'; supported templates: todo-api.{db_guidance}"
            ))
        }
        (None, Some(_)) => Err(
            "`--db` is only supported with `meshc init --template todo-api <name>`; omit `--db` for hello-world or `--clustered`, or add `--template todo-api` (sqlite stays the local default, postgres opts into the clustered/deployable starter)."
                .to_string(),
        ),
        (None, None) if clustered => Ok(InitTarget::Clustered),
        (None, None) => Ok(InitTarget::HelloWorld),
    }
}

fn run_init_command(
    clustered: bool,
    template: Option<&str>,
    db: Option<InitTodoDb>,
    name: &str,
    reporter: &mut Reporter,
) -> Result<(), Failed> {
    let dir = std::env::current_dir().map_err(|error| {
        reporter.fail(Diagnostic::new(
            Code::PathNotFound,
            format!("Cannot resolve the current directory: {error}"),
        ))
    })?;
    let kind = match resolve_init_target(clustered, template, db)
        .reported_as(Code::InvalidArguments, reporter)?
    {
        InitTarget::HelloWorld => {
            mesh_pkg::scaffold_project(name, &dir).reported_as(Code::PathNotFound, reporter)?;
            ""
        }
        InitTarget::Clustered => {
            mesh_pkg::scaffold_clustered_project(name, &dir)
                .reported_as(Code::PathNotFound, reporter)?;
            "clustered "
        }
        InitTarget::TodoApi(database) => {
            mesh_pkg::scaffold_todo_api_project_with_db(name, &dir, database)
                .reported_as(Code::PathNotFound, reporter)?;
            "todo-api "
        }
    };
    reporter.status(format_args!("Created {kind}project '{name}'"));
    Ok(())
}

/// The compiler recurses over expression trees (a string with hundreds of
/// interpolations, a long `a <> b <> ...` chain), deeper than a main
/// thread's default stack allows; it runs on a thread with room for them.
const COMPILER_STACK_BYTES: usize = 512 * 1024 * 1024;

fn main() {
    let compiler = std::thread::Builder::new()
        .name("meshc".to_string())
        .stack_size(COMPILER_STACK_BYTES)
        .spawn(run)
        .expect("failed to start the compiler thread");
    if let Err(panic) = compiler.join() {
        std::panic::resume_unwind(panic);
    }
}

fn run() {
    let cli = Cli::parse_or_report();
    let mut reporter = Reporter::stderr(Format::for_cli(cli.json, cli.no_color));
    // Lint findings are warnings, which fail no build; failing on them is
    // what `meshc lint` is for, as a CI gate.
    let fails_on_warnings = matches!(cli.command, Commands::Lint { .. });
    // `meshc test` fails by its tests: a runtime error a passing test
    // provokes on purpose, such as an actor it crashes, is reported only,
    // without an error tally under the passing summary.
    let fails_by_result = matches!(cli.command, Commands::Test { .. });
    let result = dispatch(cli, &mut reporter);
    let summary = if fails_by_result && result.is_ok() {
        reporter.finish_untallied()
    } else {
        reporter.finish()
    };
    let exit_code = if result.is_err() || (fails_on_warnings && summary.warnings > 0) {
        1
    } else if fails_by_result {
        0
    } else {
        summary.exit_code()
    };
    if exit_code != 0 {
        process::exit(exit_code);
    }
}

fn dispatch(cli: Cli, reporter: &mut Reporter) -> Result<(), Failed> {
    let json = cli.json;
    match cli.command {
        Commands::Build {
            dir,
            opt_level,
            emit_llvm,
            output,
            target,
            artifact,
            plaintext_report,
        } => build_with_report(
            &dir,
            opt_level,
            emit_llvm,
            output.as_deref(),
            target.as_deref(),
            artifact,
            false,
            reporter,
            plaintext_report.as_deref(),
            true,
        ),
        Commands::Check { path, tests } => run_check(&path, tests, reporter),
        Commands::Init {
            clustered,
            template,
            db,
            name,
        } => run_init_command(clustered, template.as_deref(), db, &name, reporter),
        Commands::Cluster { action } => cluster::run_cluster_command(action, json, reporter),
        Commands::Proof { action } => proof::run_proof_command(action, reporter),
        Commands::Deps { dir } => deps_command(&dir, reporter),
        Commands::Update => run_update_command(json, reporter),
        Commands::Guide { topic } => guide::show(topic.as_deref()).reported(reporter),
        Commands::Explain { code } => {
            print!(
                "{}",
                explain::render(code.as_deref(), json).reported(reporter)?
            );
            Ok(())
        }
        Commands::Skills { action } => skills::run(action, reporter),
        Commands::Verify {
            dir,
            list,
            preserve,
            emit_proof,
            verdict,
        } => verify::run(
            &dir,
            list,
            json,
            preserve.as_deref(),
            verify_proof::Settings {
                emit: emit_proof.as_deref(),
                verdict,
            },
            reporter,
        ),
        Commands::Fmt {
            path,
            check,
            line_width,
            indent_size,
        } => {
            let config = mesh_fmt::FormatConfig {
                indent_size,
                max_width: line_width,
            };
            fmt_command(&path, check, &config, reporter)
        }
        Commands::Lint { path } => lint_command(&path, reporter),
        Commands::PlaintextReport { dir, output, check } => {
            let prepared = prepare_project_build(&dir, false, reporter)?;
            plaintext_report::run(
                &prepared.plaintext_report,
                output.as_deref(),
                check.as_deref(),
                reporter,
            )
        }
        Commands::Repl => mesh_repl::run_repl(&mesh_repl::ReplConfig::default())
            .reported_as(Code::PathNotFound, reporter),
        Commands::Lsp => {
            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| Diagnostic::internal(format!("Failed to create tokio runtime: {e}")))
                .reported(reporter)?;
            rt.block_on(mesh_lsp::run_server());
            // Its stdin read cannot be cancelled: do not wait for it.
            rt.shutdown_background();
            Ok(())
        }
        Commands::Test {
            path,
            quiet,
            coverage,
        } => test_runner::run_tests(
            path.as_deref(),
            quiet,
            coverage,
            json,
            cli.no_color,
            reporter,
        ),
        Commands::Migrate { action, dir } => match action.unwrap_or(MigrateAction::Up) {
            MigrateAction::Up => migrate::run_migrations_up(&dir, reporter),
            MigrateAction::Down => migrate::run_migrations_down(&dir, reporter),
            MigrateAction::Status => migrate::show_migration_status(&dir, reporter),
            MigrateAction::Generate { name } => migrate::generate_migration(&dir, &name, reporter),
        },
    }
}

/// `error`'s message and notes as one line, for a verification reason.
pub(crate) fn diagnostic_reason(error: &Diagnostic) -> String {
    std::iter::once(error.message())
        .chain(error.notes().iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(": ")
}

trait ReportResult<T> {
    fn reported(self, reporter: &mut Reporter) -> Result<T, Failed>;
}

/// Give a text-only failure its command-specific code at the CLI boundary.
trait ReportText<T> {
    fn reported_as(self, code: Code, reporter: &mut Reporter) -> Result<T, Failed>;
}

impl<T> ReportText<T> for Result<T, String> {
    fn reported_as(self, code: Code, reporter: &mut Reporter) -> Result<T, Failed> {
        self.map_err(|message| reporter.fail(Diagnostic::new(code, message)))
    }
}

impl<T> ReportResult<T> for Result<T, Diagnostic> {
    fn reported(self, reporter: &mut Reporter) -> Result<T, Failed> {
        self.map_err(|diagnostic| reporter.fail(diagnostic))
    }
}

fn run_update_command(json: bool, reporter: &mut Reporter) -> Result<(), Failed> {
    let outcome = mesh_pkg::run_toolchain_update_with_progress(!json)
        .map_err(|error| error.to_diagnostic())
        .reported(reporter)?;
    println!("{}", outcome.mode);
    Ok(())
}

fn run_check(path: &Path, tests: bool, reporter: &mut Reporter) -> Result<(), Failed> {
    if tests && test_runner::is_test_file(path) {
        return test_runner::check_tests(path, reporter);
    }
    frontend::check(Target::Project { root: path }, &CheckOptions::cli()).report(reporter)?;
    if tests && path.is_dir() {
        test_runner::check_tests(path, reporter)?;
    }
    Ok(())
}

pub(crate) struct PreparedBuild {
    pub(crate) merged_mir: mesh_codegen::mir::MirModule,
    /// The entrypoint, relative to the project root, as resolved from the manifest.
    pub(crate) entry_relative_path: PathBuf,
    pub(crate) library_exports: Vec<mesh_codegen::LibraryExport>,
    pub(crate) clustered_execution_plan: Vec<ClusteredExecutionMetadata>,
    pub(crate) clustered_route_handler_plan: Vec<mesh_codegen::DeclaredHandlerPlanEntry>,
    pub(crate) autonomous_config_json: Option<String>,
    /// Every `declassify` and `@display` export, for `--plaintext-report`.
    pub(crate) plaintext_report: plaintext_report::Report,
    /// Each module's file, by module id: the MIR's source spans index it.
    pub(crate) files: Vec<SourceFile>,
}

impl PreparedBuild {
    fn codegen_input<'a>(
        &'a self,
        mir: &'a mesh_codegen::mir::MirModule,
        declared_handlers: &'a [mesh_codegen::DeclaredRuntimeRegistration],
        startup_work_registrations: &'a [mesh_codegen::StartupWorkRegistration],
    ) -> mesh_codegen::CodegenInput<'a> {
        mesh_codegen::CodegenInput {
            mir,
            declared_handlers,
            startup_work_registrations,
            autonomous_config_json: self.autonomous_config_json.as_deref(),
            library_exports: &self.library_exports,
            files: &self.files,
        }
    }
}

fn runtime_autonomous_config_json(
    config: Option<&mesh_pkg::AutonomousClusterConfig>,
) -> Result<Option<String>, String> {
    use mesh_pkg::{
        CapacityDriverKind, ClusterMode, DurabilityMode, ForcedTerminationPolicy, ManagedRole,
        RoutingAlgorithm,
    };
    use mesh_rt::{
        RuntimeAutonomousConfig, RuntimeCapacityDriverConfig, RuntimeContinuityConfig,
        RuntimeFeatureGates, RuntimeRoutingConfig, RuntimeSchedulerConfig, ScalingPolicy,
        AUTONOMOUS_CONFIG_SCHEMA_VERSION,
    };

    let Some(config) = config.filter(|config| config.mode == ClusterMode::Autonomous) else {
        return Ok(None);
    };
    let capacity = &config.capacity;
    let (driver, template_revision) = match (
        config.autoscaling.enabled,
        capacity.driver,
        &capacity.process,
        &capacity.docker,
    ) {
        (false, ..) => (
            RuntimeCapacityDriverConfig::Disabled,
            "disabled-v1".to_string(),
        ),
        (true, Some(CapacityDriverKind::Process), Some(process), _) => (
            RuntimeCapacityDriverConfig::Process {
                command: process.command.clone(),
                working_directory: process.working_directory.clone(),
            },
            "process-v1".to_string(),
        ),
        (true, Some(CapacityDriverKind::Docker), _, Some(docker)) => (
            RuntimeCapacityDriverConfig::Docker {
                image: docker.image.clone(),
                pool: docker.pool.clone(),
                network: docker.network.clone(),
                environment: docker.env.clone(),
            },
            docker.template_revision.clone(),
        ),
        _ => return Err("autoscaling needs a configured capacity driver".to_string()),
    };
    let runtime = RuntimeAutonomousConfig {
        schema_version: AUTONOMOUS_CONFIG_SCHEMA_VERSION,
        enabled: true,
        features: RuntimeFeatureGates {
            protocol_two: config.features.protocol_two,
            durable_continuity: config.features.durable_continuity,
            telemetry: config.features.telemetry,
            local_scheduler_autoscaling: config.features.local_scheduler_autoscaling,
            adaptive_routing: config.features.adaptive_routing,
            controller_quorum: config.features.controller_quorum,
            horizontal_autoscaling: config.autoscaling.enabled,
            horizontal_observe_only: config.features.horizontal_observe_only,
            automatic_scale_up: config.features.automatic_scale_up,
            automatic_scale_down: config.features.automatic_scale_down,
        },
        policy_revision: 1,
        policy: ScalingPolicy {
            min_nodes: config.autoscaling.min_nodes,
            max_nodes: config.autoscaling.max_nodes,
            target_inflight_per_node: config.autoscaling.target_inflight_per_node,
            scale_up_window_millis: config.autoscaling.scale_up_window.as_millis(),
            scale_down_window_millis: config.autoscaling.scale_down_window.as_millis(),
            cooldown_millis: config.autoscaling.cooldown.as_millis(),
            max_scale_up_step: config.autoscaling.max_scale_up_step,
            max_scale_down_step: config.autoscaling.max_scale_down_step,
            max_unavailable: config.autoscaling.max_unavailable,
        },
        managed_roles: config
            .autoscaling
            .managed_roles
            .iter()
            .map(|role| match role {
                ManagedRole::Gateway => "gateway".to_string(),
                ManagedRole::Worker => "worker".to_string(),
            })
            .collect(),
        gateway_nodes: if config
            .autoscaling
            .managed_roles
            .contains(&ManagedRole::Gateway)
        {
            0
        } else {
            u16::from(config.roles.gateway)
        },
        template_revision,
        reconcile_interval_millis: config.routing.load_report_interval.as_millis().max(100),
        startup_timeout_millis: config.capacity.startup_timeout.as_millis(),
        drain_timeout_millis: config.capacity.drain_timeout.as_millis(),
        termination_timeout_millis: config.capacity.termination_timeout.as_millis(),
        force_termination_after_drain_timeout: config.capacity.forced_termination
            == ForcedTerminationPolicy::AfterDrainTimeout,
        scheduler: RuntimeSchedulerConfig {
            min_workers: config.scheduler.min_workers,
            max_workers: config.scheduler.max_workers,
            target_runnable_per_worker: config.scheduler.target_runnable_per_worker,
            target_queue_wait_millis: config.autoscaling.target_queue_wait.as_millis(),
            scale_up_window_millis: config.scheduler.scale_up_window.as_millis(),
            scale_down_window_millis: config.scheduler.scale_down_window.as_millis(),
            cooldown_millis: config.autoscaling.cooldown.as_millis(),
        },
        routing: RuntimeRoutingConfig {
            adaptive: config.features.adaptive_routing
                && config.routing.algorithm == RoutingAlgorithm::Adaptive,
            load_report_interval_millis: config.routing.load_report_interval.as_millis(),
            load_report_ttl_millis: config.routing.load_report_ttl.as_millis(),
            target_inflight: config.autoscaling.target_inflight_per_node,
            target_queue_wait_millis: config.autoscaling.target_queue_wait.as_millis(),
            max_inflight: config.routing.max_inflight_per_node,
            max_queued_items: config.routing.max_queued_per_node,
            max_queued_bytes: config.routing.max_queued_bytes_per_node.as_bytes(),
            retry_budget_percent: config.routing.retry_budget_percent,
        },
        continuity: RuntimeContinuityConfig {
            strict_durability: config.durability == DurabilityMode::Strict,
            terminal_retention_millis: config.continuity.terminal_retention.as_millis(),
            tombstone_retention_millis: config.continuity.tombstone_retention.as_millis(),
            max_terminal_records: config.continuity.max_terminal_records,
            max_disk_bytes: config.continuity.max_disk_bytes.as_bytes(),
            snapshot_chunk_bytes: config.continuity.snapshot_chunk_bytes.as_bytes(),
            path: config.continuity.path.clone(),
        },
        driver,
    };
    runtime.validate()?;
    serde_json::to_string(&runtime)
        .map(Some)
        .map_err(|error| format!("autonomous runtime config encode failed: {error}"))
}

/// Execute the build pipeline: discover all .mpl files -> parse -> typecheck entry -> codegen -> link.
pub(crate) fn build(
    dir: &Path,
    opt_level: u8,
    emit_llvm: bool,
    output: Option<&Path>,
    target: Option<&str>,
    artifact: BuildArtifact,
    test_builtins: bool,
    reporter: &mut Reporter,
) -> Result<(), Failed> {
    build_with_report(
        dir,
        opt_level,
        emit_llvm,
        output,
        target,
        artifact,
        test_builtins,
        reporter,
        None,
        false,
    )
}

/// `build`, writing the plaintext report to `plaintext_report` once the
/// artifact is built.
pub(crate) fn build_with_report(
    dir: &Path,
    opt_level: u8,
    emit_llvm: bool,
    output: Option<&Path>,
    target: Option<&str>,
    artifact: BuildArtifact,
    test_builtins: bool,
    reporter: &mut Reporter,
    plaintext_report: Option<&Path>,
    announce: bool,
) -> Result<(), Failed> {
    let mut prepared = prepare_project_build(dir, test_builtins, reporter)?;
    // Without an entry function codegen emits no C `main`, and the linker's
    // "_main not found" is all the user would see.
    if artifact == BuildArtifact::Executable && prepared.merged_mir.entry_function.is_none() {
        let message = format!(
            "{} has no `fn main()`: an executable starts there",
            prepared.entry_relative_path.display()
        );
        let entry = prepared
            .files
            .iter()
            .find(|file| file.path().ends_with(&prepared.entry_relative_path));
        let error = match entry {
            Some(file) => Diagnostic::in_file(Code::ArtifactEntryMissing, file, message),
            None => Diagnostic::new(Code::ArtifactEntryMissing, message),
        };
        return Err(reporter.fail(error.with_repair(
            "Define `fn main()` in the entrypoint, or build a library with `--artifact staticlib` or `--artifact cdylib` from `@export` functions.",
        )));
    }
    let declared_handler_plan = prepare_declared_handler_plan(
        &prepared.clustered_execution_plan,
        &prepared.clustered_route_handler_plan,
    );
    let startup_work_registrations =
        mesh_codegen::prepare_startup_work_registrations(&declared_handler_plan);
    let declared_handlers = mesh_codegen::prepare_declared_runtime_handlers(
        &mut prepared.merged_mir,
        &declared_handler_plan,
        &prepared.files,
    )
    .reported(reporter)?;

    let output_path = output.map_or_else(
        || default_output_path(dir, artifact, target),
        Path::to_path_buf,
    );

    // Emit LLVM IR if requested
    if emit_llvm {
        let ll_path = output_path.with_extension("ll");
        let mut llvm_mir = prepared.merged_mir.clone();
        if artifact != BuildArtifact::Executable {
            llvm_mir.entry_function = None;
        }
        mesh_codegen::compile_mir_to_llvm_ir(
            prepared.codegen_input(&llvm_mir, &declared_handlers, &startup_work_registrations),
            &ll_path,
            opt_level,
            target,
        )
        .reported(reporter)?;
        reporter.status(format_args!("  LLVM IR: {}", ll_path.display()));
    }

    // Compile to native binary
    let runtime_flavor = if test_builtins {
        mesh_codegen::link::RuntimeFlavor::Test
    } else {
        mesh_codegen::link::RuntimeFlavor::Standard
    };
    let runtime_override = runtime_lib_override_from_env(runtime_flavor).reported(reporter)?;
    let native_archives = if dir.join("mesh.toml").is_file() {
        let effective_target =
            mesh_codegen::link::effective_target_triple(target).reported(reporter)?;
        mesh_pkg::resolve_native_archives(dir, &effective_target)
            .reported(reporter)?
            .into_iter()
            .map(|archive| archive.path)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    match artifact {
        BuildArtifact::Executable => mesh_codegen::compile_mir_to_binary(
            prepared.codegen_input(
                &prepared.merged_mir,
                &declared_handlers,
                &startup_work_registrations,
            ),
            &output_path,
            opt_level,
            target,
            runtime_flavor,
            runtime_override.as_deref(),
            &native_archives,
        )
        .reported(reporter)?,
        BuildArtifact::Staticlib | BuildArtifact::Cdylib => {
            mesh_codegen::compile_mir_to_library(
                prepared.codegen_input(
                    &prepared.merged_mir,
                    &declared_handlers,
                    &startup_work_registrations,
                ),
                if artifact == BuildArtifact::Staticlib {
                    mesh_codegen::LibraryArtifact::Static
                } else {
                    mesh_codegen::LibraryArtifact::Dynamic
                },
                &output_path,
                opt_level,
                target,
                runtime_override.as_deref(),
                &native_archives,
            )
            .reported(reporter)?;
            library_bindings::write(&output_path, &prepared.library_exports, target)
                .reported_as(Code::PathNotFound, reporter)?;
        }
    }

    if announce {
        reporter.status(format_args!("  Compiled: {}", output_path.display()));
    }
    if let Some(path) = plaintext_report {
        plaintext_report::write(&prepared.plaintext_report, path).reported(reporter)?;
        reporter.status(format_args!("  Plaintext report: {}", path.display()));
    }

    Ok(())
}

fn prepare_declared_handler_plan(
    entries: &[ClusteredExecutionMetadata],
    clustered_route_entries: &[mesh_codegen::DeclaredHandlerPlanEntry],
) -> Vec<mesh_codegen::DeclaredHandlerPlanEntry> {
    let mut plan = entries
        .iter()
        .map(|entry| mesh_codegen::DeclaredHandlerPlanEntry {
            kind: mesh_codegen::DeclaredHandlerKind::Work,
            runtime_registration_name: entry.runtime_registration_name.clone(),
            executable_symbol: entry.executable_symbol.clone(),
            replication_count: entry.replication_count.value as u64,
        })
        .collect::<Vec<_>>();
    plan.extend(clustered_route_entries.iter().cloned());
    plan
}

#[derive(Default)]
struct VerificationLowering {
    requested: HashMap<String, Vec<Ty>>,
    signatures: Vec<Vec<mesh_codegen::mir::lower::LoweredSignature>>,
}

pub(crate) fn prepare_project_build(
    dir: &Path,
    test_builtins: bool,
    reporter: &mut Reporter,
) -> Result<PreparedBuild, Failed> {
    let options = CheckOptions {
        test_builtins,
        require_entrypoint: true,
        ..CheckOptions::cli()
    };
    let Analysis {
        manifest,
        entry,
        project,
        mut checked,
        mut clustered_execution_plan,
        files,
        ..
    } = frontend::check(Target::Project { root: dir }, &options).report(reporter)?;
    let entry_relative_path = entry.expect("build requires an entrypoint");

    let package = manifest.as_ref().map_or_else(
        || {
            dir.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        },
        |manifest| manifest.package.name.clone(),
    );
    let project_root = if dir.is_file() {
        dir.parent().unwrap_or(Path::new(""))
    } else {
        dir
    };
    let plaintext_report =
        plaintext_report::collect(project_root, &package, &project, &checked.typeck);

    // `@cluster` and `HTTP.clustered` without a count take the manifest's
    // `[cluster].default_replicas`.
    let default_replicas = manifest
        .as_ref()
        .and_then(|manifest| manifest.autonomous_cluster.as_ref())
        .map_or(mesh_pkg::DEFAULT_CLUSTER_REPLICATION_COUNT, |cluster| {
            cluster.default_replicas
        });
    for entry in &mut clustered_execution_plan {
        if entry.replication_count.source == mesh_pkg::ClusteredReplicationCountSource::Default {
            entry.replication_count.value = default_replicas;
        }
    }
    let clustered_route_handler_plan =
        mesh_codegen::prepare_clustered_route_handler_plan(checked.typeck.iter(), default_replicas)
            .reported(reporter)?;
    let library_exports =
        collect_library_exports(&project.module_parses, &files).reported(reporter)?;
    let mir_modules = verify::lower_for_build(&project, &mut checked, &files, &package, reporter)?;
    let entry_mir_idx = project
        .compilation_order
        .iter()
        .position(|id| project.graph.get(*id).is_entry)
        .expect("checked project has an entry module");
    let declared_executable_symbols = clustered_execution_plan
        .iter()
        .map(|entry| entry.executable_symbol.clone())
        .chain(
            clustered_route_handler_plan
                .iter()
                .map(|entry| entry.executable_symbol.clone()),
        )
        .chain(library_exports.iter().map(|export| export.function.clone()))
        .collect::<Vec<_>>();
    let merged_mir =
        mesh_codegen::merge_mir_modules(mir_modules, entry_mir_idx, &declared_executable_symbols);

    Ok(PreparedBuild {
        merged_mir,
        entry_relative_path,
        library_exports,
        clustered_execution_plan,
        clustered_route_handler_plan,
        autonomous_config_json: runtime_autonomous_config_json(
            manifest
                .as_ref()
                .and_then(|manifest| manifest.autonomous_cluster.as_ref()),
        )
        .map_err(Diagnostic::internal)
        .reported(reporter)?,
        plaintext_report,
        files,
    })
}

/// Lower checked modules in compilation order, before the native build prunes
/// unreachable functions. Verification also needs unused function bodies.
/// `files` holds each module's file by the `MirSourceId` that `source_id`
/// assigns. Every diagnostic lowering reports is emitted, except the last
/// error of a module, which is returned.
fn lower_checked_modules(
    project: &mesh_pkg::project::ProjectData,
    checked: &mut mesh_pkg::project::CheckedProject,
    source_id: impl Fn(&mesh_parser::Parse) -> Option<mesh_codegen::mir::MirSourceId>,
    files: &[SourceFile],
    mut verification: Option<&mut VerificationLowering>,
    reporter: &mut Reporter,
) -> Result<Vec<mesh_codegen::mir::MirModule>, Diagnostic> {
    let mesh_pkg::project::CheckedProject {
        typeck: all_typeck,
        exports: all_exports,
    } = checked;
    let inferred_export_names: HashSet<String> = all_exports
        .iter()
        .filter_map(|exports_opt| exports_opt.as_ref())
        .flat_map(|exports| {
            exports.functions.iter().filter_map(|(name, scheme)| {
                if has_runtime_type_variables(&scheme.ty) {
                    Some(name.clone())
                } else {
                    None
                }
            })
        })
        .collect();
    let mut inferred_fn_usage_types =
        collect_inferred_fn_usage_types(&project.module_parses, all_typeck, &inferred_export_names);
    if let Some(verification) = &verification {
        for (name, types) in &verification.requested {
            for ty in types {
                push_usage_type(&mut inferred_fn_usage_types, name, ty);
            }
        }
    }

    // A module is checked before the modules that come later, so its trait
    // registry lacks their impls (it has its own and the earlier modules').
    // Lowering specializes its generic functions for the types those modules
    // use them at (`show_it(Rect)`, `where T: Display`, with `Rect` declared
    // later), so the later modules' impls are added for it. (Adding an impl
    // twice would make a lookup see two.)
    let order = &project.compilation_order;
    for (position, &id) in order.iter().enumerate() {
        let later_impls: Vec<_> = order[position + 1..]
            .iter()
            .filter_map(|later| all_exports[later.0 as usize].as_ref())
            .flat_map(|exports| exports.trait_impls.iter().cloned())
            .collect();
        for impl_def in later_impls {
            let _ = all_typeck[id.0 as usize]
                .trait_registry
                .register_impl(impl_def);
        }
    }

    // Pub function names, for module-qualified naming (Phase 41).
    let pub_fns_of = |idx: usize| -> HashSet<String> {
        all_exports[idx]
            .as_ref()
            .map(|e| e.functions.keys().cloned().collect())
            .unwrap_or_default()
    };

    // A generic function called from another module's generic function
    // needs a specialization for each type that one is specialized at.
    for &id in project.compilation_order.iter().rev() {
        let idx = id.0 as usize;
        let requests = mesh_codegen::imported_specializations(
            &project.module_parses[idx],
            &all_typeck[idx],
            &project.graph.get(id).name,
            &pub_fns_of(idx),
            &inferred_fn_usage_types,
        );
        for (name, tys) in requests {
            for ty in &tys {
                push_usage_type(&mut inferred_fn_usage_types, &name, ty);
            }
        }
    }

    let mut mir_modules = Vec::new();
    for &id in &project.compilation_order {
        let idx = id.0 as usize;
        let parse = &project.module_parses[idx];
        let typeck = &all_typeck[idx];
        let module_name = &project.graph.get(id).name;
        let pub_fns = pub_fns_of(idx);

        let other_modules: Vec<_> = project
            .module_parses
            .iter()
            .zip(all_typeck.iter())
            .enumerate()
            .filter(|(other, _)| *other != idx)
            .map(|(_, modules)| modules)
            .collect();
        let lowered = mesh_codegen::mir::lower::lower_module_with_diagnostics(
            parse,
            typeck,
            module_name,
            &pub_fns,
            &inferred_fn_usage_types,
            &other_modules,
            &source_id,
            files,
            verification.is_some(),
        )?;
        let (mut errors, others): (Vec<_>, Vec<_>) = lowered
            .diagnostics
            .into_iter()
            .partition(|diagnostic| diagnostic.severity() == Severity::Error);
        let last_error = errors.pop();
        for diagnostic in others.into_iter().chain(errors) {
            reporter.emit(diagnostic);
        }
        if let Some(error) = last_error {
            return Err(error);
        }
        if let Some(verification) = verification.as_deref_mut() {
            verification.signatures.push(lowered.signatures);
        }
        mir_modules.push(lowered.module);
    }
    Ok(mir_modules)
}

/// `files` holds each module's file by module id, the index of its parse.
fn collect_library_exports(
    parses: &[mesh_parser::Parse],
    files: &[SourceFile],
) -> Result<Vec<mesh_codegen::LibraryExport>, Diagnostic> {
    let mut symbols = HashMap::new();
    let mut exports = Vec::new();
    for (parse, file) in parses.iter().zip(files) {
        for function in parse.tree().fn_defs() {
            let Some(declaration) = function.export_decl() else {
                continue;
            };
            let function_name = function
                .name()
                .and_then(|name| name.text())
                .ok_or_else(|| Diagnostic::internal("exported function is missing a name"))?;
            let symbol = declaration
                .symbol()
                .ok_or_else(|| Diagnostic::internal("exported function is missing a symbol"))?;
            let range = declaration.syntax().text_range();
            let span = mesh_common::span::Span::new(range.start().into(), range.end().into());
            if let Some((first_file, first_span)) =
                symbols.insert(symbol.clone(), (file.clone(), span))
            {
                return Err(Diagnostic::in_file(
                    Code::ExportSymbolCollision,
                    file,
                    format!("exported symbol `{symbol}` is exported twice"),
                )
                .at(span, "exported again here")
                .with_label_in(&first_file, first_span, "first exported here"));
            }
            exports.push(mesh_codegen::LibraryExport {
                function: function_name,
                symbol,
            });
        }
    }
    Ok(exports)
}

/// Where `meshc build` writes an artifact when `--output` is not given:
/// named after the project directory, in the platform's library naming.
fn default_output_path(dir: &Path, artifact: BuildArtifact, target: Option<&str>) -> PathBuf {
    let (dir, name) = if dir.is_file() {
        (dir.parent().unwrap_or(Path::new(".")), dir.file_stem())
    } else {
        (dir, dir.file_name())
    };
    let project_name = name.and_then(|n| n.to_str()).unwrap_or("output");
    let targets = |vendor: &str, host: bool| target.map_or(host, |triple| triple.contains(vendor));
    match artifact {
        BuildArtifact::Executable => dir.join(project_name),
        BuildArtifact::Staticlib => dir.join(format!("lib{project_name}.a")),
        BuildArtifact::Cdylib => {
            let extension = if targets("apple", cfg!(target_os = "macos")) {
                "dylib"
            } else if targets("windows", cfg!(target_os = "windows")) {
                "dll"
            } else {
                "so"
            };
            dir.join(format!("lib{project_name}.{extension}"))
        }
    }
}

fn runtime_lib_override_from_env(
    runtime_flavor: mesh_codegen::link::RuntimeFlavor,
) -> Result<Option<PathBuf>, Diagnostic> {
    let variable = match runtime_flavor {
        mesh_codegen::link::RuntimeFlavor::Standard => "MESH_RT_LIB_PATH",
        mesh_codegen::link::RuntimeFlavor::Test => "MESH_TEST_RT_LIB_PATH",
    };
    let Some(raw) = std::env::var_os(variable) else {
        return Ok(None);
    };

    if raw.is_empty() {
        return Err(Diagnostic::new(
            Code::RuntimeLibraryMissing,
            format!("{variable} was set but empty"),
        )
        .with_repair(format!(
            "Set {variable} to an absolute path to the selected Mesh runtime static library, or unset it."
        )));
    }

    Ok(Some(PathBuf::from(raw)))
}

fn has_runtime_type_variables(ty: &Ty) -> bool {
    matches!(ty, Ty::Var(_)) || ty.parts().any(has_runtime_type_variables)
}

fn is_concrete_fn_ty(ty: &Ty) -> bool {
    matches!(ty, Ty::Fun(..)) && !has_runtime_type_variables(ty)
}

fn push_usage_type(map: &mut HashMap<String, Vec<Ty>>, name: &str, ty: &Ty) {
    if !is_concrete_fn_ty(ty) {
        return;
    }
    let entry = map.entry(name.to_string()).or_default();
    if !entry.contains(ty) {
        entry.push(ty.clone());
    }
}

fn collect_inferred_fn_usage_types(
    parses: &[mesh_parser::Parse],
    typecks: &[mesh_typeck::TypeckResult],
    candidate_names: &HashSet<String>,
) -> HashMap<String, Vec<Ty>> {
    let mut usage = HashMap::new();
    if candidate_names.is_empty() {
        return usage;
    }

    for (parse, typeck) in parses.iter().zip(typecks) {
        // A callee of a call to an overloaded fn names the arity the call
        // runs (`name__N`).
        let overload_target = |callee: &mesh_parser::SyntaxNode| {
            let call = callee
                .parent()
                .and_then(mesh_parser::ast::expr::CallExpr::cast)?;
            (call.callee()?.syntax() == callee)
                .then(|| {
                    typeck
                        .overloaded_call_targets
                        .get(&call.syntax().text_range())
                })?
                .cloned()
        };
        for name_ref in parse.syntax().descendants().filter_map(NameRef::cast) {
            let name = overload_target(name_ref.syntax()).or_else(|| name_ref.text());
            let Some(name) = name.filter(|name| candidate_names.contains(name)) else {
                continue;
            };
            if let Some(ty) = typeck.types.get(&name_ref.syntax().text_range()) {
                push_usage_type(&mut usage, &name, ty);
            }
        }
        for field_access in parse.syntax().descendants().filter_map(FieldAccess::cast) {
            // `Module.name`: the base names an imported module.
            let base = match field_access.base() {
                Some(mesh_parser::ast::expr::Expr::NameRef(base)) => base.text(),
                _ => None,
            };
            if !base.is_some_and(|name| typeck.qualified_modules.contains_key(&name)) {
                continue;
            }
            let Some(field_name) = overload_target(field_access.syntax())
                .or_else(|| field_access.field().map(|t| t.text().to_string()))
                .filter(|name| candidate_names.contains(name))
            else {
                continue;
            };
            if let Some(ty) = typeck.types.get(&field_access.syntax().text_range()) {
                push_usage_type(&mut usage, &field_name, ty);
            }
        }
    }

    usage
}

// ── Deps subcommand ──────────────────────────────────────────────────

/// Execute the `deps` subcommand: resolve dependencies and generate mesh.lock.
///
/// If mesh.lock already exists and the manifest hasn't changed, skips resolution.
fn deps_command(dir: &Path, reporter: &mut Reporter) -> Result<(), Failed> {
    let manifest_path = dir.join("mesh.toml");
    let lock_path = dir.join("mesh.lock");

    // The lockfile is fresh when the manifest has not changed since it was
    // written and every git dependency is checked out: `meshpkg install`
    // writes it too, without fetching git dependencies.
    let git_checkouts_present = mesh_pkg::Manifest::from_file(&manifest_path)
        .reported(reporter)?
        .dependencies
        .iter()
        .filter(|(_, dep)| matches!(dep, mesh_pkg::manifest::Dependency::Git { .. }))
        .all(|(name, _)| dir.join(".mesh").join("deps").join(name).is_dir());
    // A time that cannot be read is no proof of freshness.
    let modified = |path: &Path| std::fs::metadata(path).and_then(|m| m.modified()).ok();
    if git_checkouts_present
        && modified(&manifest_path)
            .zip(modified(&lock_path))
            .is_some_and(|(manifest_time, lock_time)| manifest_time <= lock_time)
    {
        mesh_pkg::Lockfile::from_file(&lock_path).reported(reporter)?;
        reporter.status(format_args!("Dependencies up to date"));
        return Ok(());
    }

    let (resolved, lockfile) = mesh_pkg::resolve_dependencies(dir).reported(reporter)?;

    lockfile.write(&lock_path).reported(reporter)?;

    if resolved.is_empty() {
        reporter.status(format_args!("No dependencies"));
    } else {
        reporter.status(format_args!("Resolved {} dependencies", resolved.len()));
    }

    Ok(())
}

// ── Format subcommand ─────────────────────────────────────────────────

/// Execute the `fmt` subcommand: format Mesh source files in-place or check formatting.
fn fmt_command(
    path: &Path,
    check: bool,
    config: &mesh_fmt::FormatConfig,
    reporter: &mut Reporter,
) -> Result<(), Failed> {
    let files = collect_mesh_files(path).reported(reporter)?;

    // Format every file before writing any: a file that cannot be formatted
    // leaves the whole tree as it was.
    let mut changed = Vec::new();
    let mut failure = None;
    for path in &files {
        let source = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read '{}': {e}", path.display()))
            .reported_as(Code::PathNotFound, reporter)?;
        match mesh_fmt::try_format(&source, config) {
            Ok(formatted) if formatted != source => {
                if check {
                    let file = SourceFile::new(path.as_path(), source);
                    failure = Some(
                        reporter.fail(
                            Diagnostic::in_file(
                                Code::WouldReformat,
                                &file,
                                "File would be reformatted",
                            )
                            .with_repair(format!(
                                "Run `meshc fmt {}` to format the file.",
                                path.display()
                            )),
                        ),
                    );
                } else {
                    changed.push((path, formatted));
                }
            }
            Ok(_) => {}
            Err(mesh_fmt::FormatError::Parse(errors)) => {
                let file = SourceFile::new(path.as_path(), source);
                for error in errors {
                    failure = Some(reporter.fail(error.to_diagnostic(&file)));
                }
            }
            Err(error @ mesh_fmt::FormatError::ChangedProgram) => {
                failure = Some(reporter.fail(Diagnostic::internal(format!(
                    "Cannot format '{}': {error}",
                    path.display()
                ))));
            }
        }
    }
    if let Some(failure) = failure {
        return Err(failure);
    }
    for (path, formatted) in changed {
        std::fs::write(path, formatted)
            .map_err(|e| format!("Failed to write '{}': {e}", path.display()))
            .reported_as(Code::PathNotFound, reporter)?;
    }
    if !check {
        reporter.status(format_args!("Formatted {} file(s)", files.len()));
    }
    Ok(())
}

/// `meshc lint`: reports each file's parse errors, or, when it parses, its
/// lint findings. Exiting 1 on a finding is [`run`]'s policy.
fn lint_command(path: &Path, reporter: &mut Reporter) -> Result<(), Failed> {
    for source in collect_mesh_files(path).reported(reporter)? {
        let text = std::fs::read_to_string(&source)
            .map_err(|e| format!("Failed to read '{}': {}", source.display(), e))
            .reported_as(Code::PathNotFound, reporter)?;
        let parse = mesh_parser::parse(&text);
        let file = SourceFile::new(source, text);
        let diagnostics = if parse.ok() {
            mesh_lint::lint(&parse, &file)
        } else {
            parse
                .errors()
                .iter()
                .map(|error| error.to_diagnostic(&file))
                .collect()
        };
        for diagnostic in diagnostics {
            reporter.emit(diagnostic);
        }
    }
    Ok(())
}

/// Collect `.mpl` files from a path: the file itself, or every one under a
/// directory. Finding none is an error.
fn collect_mesh_files(path: &Path) -> Result<Vec<PathBuf>, Diagnostic> {
    if !path.exists() {
        return Err(Diagnostic::new(
            Code::PathNotFound,
            format!("Path '{}' does not exist", path.display()),
        ));
    }

    if path.is_file() {
        if path.extension().and_then(|e| e.to_str()) == Some("mpl") {
            return Ok(vec![path.to_path_buf()]);
        } else {
            return Err(Diagnostic::new(
                Code::InvalidArguments,
                format!("'{}' is not a .mpl file", path.display()),
            ));
        }
    }

    if !path.is_dir() {
        return Err(Diagnostic::new(
            Code::PathNotFound,
            format!("'{}' is not a file or directory", path.display()),
        ));
    }
    let mut files = Vec::new();
    collect_mesh_files_recursive(path, &mut files).map_err(|e| {
        Diagnostic::new(
            Code::PathNotFound,
            format!("Failed to walk directory '{}': {e}", path.display()),
        )
    })?;
    if files.is_empty() {
        return Err(Diagnostic::new(
            Code::InvalidArguments,
            format!("No .mpl files found at '{}'", path.display()),
        ));
    }
    files.sort();
    Ok(files)
}

/// Recursively collect `.mpl` files from a directory, skipping hidden entries
/// as a build does: `.git`, installed packages under `.mesh`, and editor or
/// filesystem sidecar files are not the project's source.
fn collect_mesh_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let entry_path = entry.path();
        if entry_path.is_dir() {
            collect_mesh_files_recursive(&entry_path, files)?;
        } else if entry_path.extension().and_then(|e| e.to_str()) == Some("mpl") {
            files.push(entry_path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod autonomous_config_tests {
    use super::*;
    use mesh_pkg::manifest::Manifest;
    use std::str::FromStr;

    #[test]
    fn library_exports_are_collected_as_reachability_roots_and_symbols_are_unique() {
        let collect = |sources: &[&str]| {
            let parses: Vec<_> = sources
                .iter()
                .map(|text| mesh_parser::parse(text))
                .collect();
            let files: Vec<_> = sources
                .iter()
                .enumerate()
                .map(|(index, text)| SourceFile::new(format!("m{index}.mpl"), *text))
                .collect();
            collect_library_exports(&parses, &files)
        };
        let echo = "@export(\"mesh_mobile_echo\")\npub fn echo(request :: Bytes) -> Bytes ! String do\n  Ok(request)\nend\n";
        let status = "@export(\"mesh_mobile_status\")\npub fn status(request :: Bytes) -> Bytes ! String do\n  Ok(request)\nend\n";
        let exports = collect(&[echo, status]).expect("valid exports");
        assert_eq!(exports[0].function, "echo");
        assert_eq!(exports[0].symbol, "mesh_mobile_echo");

        let again = "@export(\"mesh_mobile_echo\")\npub fn again(request :: Bytes) -> Bytes ! String do\n  Ok(request)\nend\n";
        let error = collect(&[echo, again]).unwrap_err();
        assert_eq!(error.code(), Code::ExportSymbolCollision);
        assert_eq!(
            error.message(),
            "exported symbol `mesh_mobile_echo` is exported twice"
        );
        assert_eq!(error.file().unwrap().path(), Path::new("m1.mpl"));
        assert_eq!(error.line(), Some(1));
        let [first] = error.labels() else {
            panic!("{:?}", error.labels());
        };
        assert_eq!(first.file().unwrap().path(), Path::new("m0.mpl"));
    }

    #[test]
    fn artifacts_are_named_for_their_project_and_target() {
        let dir = Path::new("/work/app");
        let named = |artifact, target| default_output_path(dir, artifact, target);
        assert_eq!(named(BuildArtifact::Executable, None), dir.join("app"));
        assert_eq!(named(BuildArtifact::Staticlib, None), dir.join("libapp.a"));
        for (target, file) in [
            ("aarch64-apple-ios", "libapp.dylib"),
            ("x86_64-pc-windows-msvc", "libapp.dll"),
            ("aarch64-linux-android", "libapp.so"),
        ] {
            assert_eq!(named(BuildArtifact::Cdylib, Some(target)), dir.join(file));
        }
        let host = if cfg!(target_os = "macos") {
            "libapp.dylib"
        } else if cfg!(target_os = "windows") {
            "libapp.dll"
        } else {
            "libapp.so"
        };
        assert_eq!(named(BuildArtifact::Cdylib, None), dir.join(host));
        assert_eq!(
            default_output_path(Path::new("/"), BuildArtifact::Executable, None),
            Path::new("/output")
        );
    }

    #[test]
    fn build_cli_accepts_all_library_artifact_modes() {
        for artifact in ["executable", "staticlib", "cdylib"] {
            let cli = Cli::try_parse_from(["meshc", "build", ".", "--artifact", artifact])
                .expect("artifact mode");
            let Commands::Build {
                artifact: parsed, ..
            } = cli.command
            else {
                panic!("build command")
            };
            assert_eq!(parsed.to_string(), artifact);
        }
    }

    #[test]
    fn autonomous_data_plane_embeds_feature_gates_without_driver_authority() {
        let manifest = Manifest::from_str(
            r#"
[package]
name = "data-plane-only"
version = "0.1.0"

[cluster]
mode = "autonomous"

[cluster.controllers]
voters = 3

[cluster.autoscaling]
enabled = false
"#,
        )
        .expect("manifest");
        let encoded = runtime_autonomous_config_json(manifest.autonomous_cluster.as_ref())
            .expect("runtime config")
            .expect("embedded config");
        let runtime: mesh_rt::RuntimeAutonomousConfig =
            serde_json::from_str(&encoded).expect("decode runtime config");

        assert!(!runtime.features.horizontal_autoscaling);
        assert!(runtime.features.protocol_two);
        assert!(runtime.features.durable_continuity);
        assert!(matches!(
            runtime.driver,
            mesh_rt::RuntimeCapacityDriverConfig::Disabled
        ));
    }

    #[test]
    fn observe_only_and_independent_action_gates_reach_runtime_schema() {
        let manifest = Manifest::from_str(
            r#"
[package]
name = "observe-only"
version = "0.1.0"

[cluster]
mode = "autonomous"

[cluster.controllers]
voters = 3

[cluster.features]
horizontal_observe_only = true
automatic_scale_up = true
automatic_scale_down = false

[cluster.autoscaling]
enabled = true
min_nodes = 2
max_nodes = 5

[cluster.capacity]
driver = "docker"

[cluster.capacity.docker]
image = "worker@sha256:abc"
pool = "workers"
template_revision = "v1"
"#,
        )
        .expect("manifest");
        let encoded = runtime_autonomous_config_json(manifest.autonomous_cluster.as_ref())
            .expect("runtime config")
            .expect("embedded config");
        let runtime: mesh_rt::RuntimeAutonomousConfig =
            serde_json::from_str(&encoded).expect("decode runtime config");

        assert!(runtime.features.horizontal_autoscaling);
        assert!(runtime.features.horizontal_observe_only);
        assert!(runtime.features.automatic_scale_up);
        assert!(!runtime.features.automatic_scale_down);
    }

    /// A process driver that also manages gateways: the runtime starts none
    /// of its own, and a config that skipped validation is refused.
    #[test]
    fn process_driver_managing_gateways_reaches_runtime_schema() {
        let manifest = Manifest::from_str(
            r#"
[package]
name = "process-pool"
version = "0.1.0"

[cluster]
mode = "autonomous"

[cluster.controllers]
voters = 3

[cluster.autoscaling]
enabled = true
managed_roles = ["worker", "gateway"]

[cluster.capacity]
driver = "process"

[cluster.capacity.process]
command = ["./worker", "--join"]
working_directory = "/srv/pool"
"#,
        )
        .expect("manifest");
        let mut config = manifest.autonomous_cluster.expect("autonomous config");
        let encoded = runtime_autonomous_config_json(Some(&config))
            .expect("runtime config")
            .expect("embedded config");
        let runtime: mesh_rt::RuntimeAutonomousConfig =
            serde_json::from_str(&encoded).expect("decode runtime config");
        assert_eq!(runtime.managed_roles, ["worker", "gateway"]);
        assert_eq!(runtime.gateway_nodes, 0);
        assert_eq!(runtime.template_revision, "process-v1");
        let mesh_rt::RuntimeCapacityDriverConfig::Process {
            command,
            working_directory,
        } = runtime.driver
        else {
            panic!("process driver: {:?}", runtime.driver)
        };
        assert_eq!(command, ["./worker", "--join"]);
        assert_eq!(working_directory, Path::new("/srv/pool"));

        config.capacity.process = None;
        assert_eq!(
            runtime_autonomous_config_json(Some(&config)).unwrap_err(),
            "autoscaling needs a configured capacity driver"
        );
    }
}
