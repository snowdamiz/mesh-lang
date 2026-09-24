//! The Mesh compiler CLI.
//!
//! Provides the `meshc` command with the following subcommands:
//!
//! - `meshc build <dir>` - Compile a Mesh project to a native binary
//! - `meshc init [--clustered] [--template <name>] [--db <sqlite|postgres>] <name>` - Initialize a new Mesh project (`todo-api` defaults to local SQLite; `--db postgres` opts into the clustered/deployable starter)
//! - `meshc cluster <status|continuity|diagnostics> ...` - Inspect runtime-owned clustered operator surfaces
//! - `meshc deps [dir]` - Resolve and fetch dependencies
//! - `meshc update` - Refresh installed `meshc` and `meshpkg` through the canonical installer path
//! - `meshc fmt <path>` - Format Mesh source files in-place
//! - `meshc lint [path]` - Report code that compiles but should be written differently
//! - `meshc test [path]` - Run *.test.mpl files from a project root, tests directory, or specific test file
//! - `meshc migrate [up|down|status|generate]` - Database migration management
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

#[cfg(target_env = "musl")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod cluster;
mod discovery;
mod library_bindings;
mod migrate;
mod proof;
mod proof_gates;
mod test_runner;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process;

use clap::{Parser, Subcommand, ValueEnum};
use mesh_parser::ast::expr::{FieldAccess, NameRef};
use mesh_parser::ast::AstNode;
use mesh_parser::syntax_kind::SyntaxKind;
use mesh_pkg::manifest::{
    build_clustered_export_surface, collect_source_cluster_declarations, resolve_entrypoint,
    validate_cluster_declarations_with_source, ClusteredDeclarationError,
    ClusteredExecutionMetadata, Manifest,
};

use mesh_typeck::diagnostics::DiagnosticOptions;
use mesh_typeck::ty::Ty;

#[derive(Parser)]
#[command(name = "meshc", version, about = "The Mesh compiler")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Compile a Mesh project to a native binary
    Build {
        /// Path to the project directory (must contain the resolved Mesh entrypoint)
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

        /// Output diagnostics as JSON (one object per line) instead of human-readable format
        #[arg(long)]
        json: bool,

        /// Disable colorized output
        #[arg(long = "no-color")]
        no_color: bool,
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
    if let Some(template_name) = template {
        if template_name != "todo-api" {
            let db_guidance = if db.is_some() {
                " `--db` is only supported with `--template todo-api`."
            } else {
                ""
            };
            return Err(format!(
                "unknown init template '{template_name}'; supported templates: todo-api.{db_guidance}"
            ));
        }
    }

    if db.is_some() && template != Some("todo-api") {
        return Err(
            "`--db` is only supported with `meshc init --template todo-api <name>`; omit `--db` for hello-world or `--clustered`, or add `--template todo-api` (sqlite stays the local default, postgres opts into the clustered/deployable starter)."
                .to_string(),
        );
    }

    if clustered && template == Some("todo-api") {
        return Err(
            "`meshc init --clustered` cannot be combined with `--template todo-api` or `--db`; use `meshc init --template todo-api <name>` for the local SQLite starter, `meshc init --template todo-api --db postgres <name>` for the clustered/deployable Todo starter, or `meshc init --clustered <name>` for the minimal clustered scaffold."
                .to_string(),
        );
    }

    match (clustered, template, db) {
        (true, None, None) => Ok(InitTarget::Clustered),
        (false, Some("todo-api"), Some(database)) => Ok(InitTarget::TodoApi(database.into())),
        (false, Some("todo-api"), None) => {
            Ok(InitTarget::TodoApi(mesh_pkg::TodoApiDatabase::Sqlite))
        }
        (false, None, None) => Ok(InitTarget::HelloWorld),
        _ => unreachable!("init argument validation should return early for unsupported cases"),
    }
}

fn run_init_command(
    clustered: bool,
    template: Option<&str>,
    db: Option<InitTodoDb>,
    name: &str,
    dir: &Path,
) -> Result<(), String> {
    match resolve_init_target(clustered, template, db)? {
        InitTarget::HelloWorld => mesh_pkg::scaffold_project(name, dir),
        InitTarget::Clustered => mesh_pkg::scaffold_clustered_project(name, dir),
        InitTarget::TodoApi(database) => {
            mesh_pkg::scaffold_todo_api_project_with_db(name, dir, database)
        }
    }
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
    let cli = Cli::parse();

    match cli.command {
        Commands::Build {
            dir,
            opt_level,
            emit_llvm,
            output,
            target,
            artifact,
            json,
            no_color,
        } => {
            // Diagnostics go to stderr: color them only for a terminal, and
            // not when `NO_COLOR` is set.
            let color = !no_color
                && !json
                && std::env::var_os("NO_COLOR").is_none()
                && std::io::IsTerminal::is_terminal(&std::io::stderr());
            let diag_opts = DiagnosticOptions {
                color,
                json,
                display_paths: Vec::new(),
            };
            if let Err(e) = build(
                &dir,
                opt_level,
                emit_llvm,
                output.as_deref(),
                target.as_deref(),
                artifact,
                false,
                &diag_opts,
            ) {
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
        Commands::Init {
            clustered,
            template,
            db,
            name,
        } => {
            let dir = std::env::current_dir().unwrap_or_default();
            if let Err(e) = run_init_command(clustered, template.as_deref(), db, &name, &dir) {
                eprintln!("error: {}", e);
                process::exit(1);
            }
        }
        Commands::Cluster { action } => {
            if let Err(e) = cluster::run_cluster_command(action) {
                eprintln!("error: {}", e);
                process::exit(1);
            }
        }
        Commands::Proof { action } => {
            if let Err(e) = proof::run_proof_command(action) {
                eprintln!("proof failed: {e}");
                std::process::exit(1);
            }
        }
        Commands::Deps { dir } => {
            if let Err(e) = deps_command(&dir) {
                eprintln!("error: {}", e);
                process::exit(1);
            }
        }
        Commands::Update => {
            if let Err(e) = run_update_command() {
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
            let config = mesh_fmt::FormatConfig {
                indent_size,
                max_width: line_width,
            };
            match fmt_command(&path, check, &config) {
                Ok(stats) => {
                    if check {
                        if stats.unformatted > 0 {
                            eprintln!("{} file(s) would be reformatted", stats.unformatted);
                            process::exit(1);
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
        Commands::Lint { path } => match lint_command(&path) {
            Ok(0) => {}
            Ok(problems) => {
                eprintln!("{} problem(s) found", problems);
                process::exit(1);
            }
            Err(e) => {
                eprintln!("error: {}", e);
                process::exit(1);
            }
        },
        Commands::Repl => {
            if let Err(e) = mesh_repl::run_repl(&mesh_repl::ReplConfig::default()) {
                eprintln!("REPL error: {}", e);
                process::exit(1);
            }
        }
        Commands::Lsp => {
            let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
            rt.block_on(mesh_lsp::run_server());
        }
        Commands::Test {
            path,
            quiet,
            coverage,
        } => match test_runner::run_tests(path.as_deref(), quiet, coverage) {
            Ok(summary) => {
                if summary.failed > 0 {
                    process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("error: {}", e);
                process::exit(1);
            }
        },
        Commands::Migrate { action, dir } => {
            let action = action.unwrap_or(MigrateAction::Up);
            let result = match action {
                MigrateAction::Up => migrate::run_migrations_up(&dir),
                MigrateAction::Down => migrate::run_migrations_down(&dir),
                MigrateAction::Status => migrate::show_migration_status(&dir),
                MigrateAction::Generate { name } => migrate::generate_migration(&dir, &name),
            };
            if let Err(e) = result {
                eprintln!("error: {}", e);
                process::exit(1);
            }
        }
    }
}

fn run_update_command() -> Result<(), String> {
    let outcome = mesh_pkg::run_toolchain_update().map_err(|error| error.to_string())?;
    match outcome.mode {
        mesh_pkg::ToolchainUpdateMode::Completed => {
            println!("Mesh toolchain update completed via the canonical installer.");
        }
        mesh_pkg::ToolchainUpdateMode::DetachedBootstrap => {
            println!(
                "Mesh toolchain update bootstrap launched; the installer will finish replacing the toolchain after this process exits."
            );
        }
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
    let driver = match (config.autoscaling.enabled, config.capacity.driver) {
        (false, _) => RuntimeCapacityDriverConfig::Disabled,
        (true, Some(CapacityDriverKind::Process)) => {
            let process = config
                .capacity
                .process
                .as_ref()
                .ok_or_else(|| "validated process driver config missing".to_string())?;
            RuntimeCapacityDriverConfig::Process {
                command: process.command.clone(),
                working_directory: process.working_directory.clone(),
            }
        }
        (true, Some(CapacityDriverKind::Docker)) => {
            let docker = config
                .capacity
                .docker
                .as_ref()
                .ok_or_else(|| "validated Docker driver config missing".to_string())?;
            RuntimeCapacityDriverConfig::Docker {
                image: docker.image.clone(),
                pool: docker.pool.clone(),
                network: docker.network.clone(),
                environment: docker.env.clone(),
            }
        }
        (true, Some(CapacityDriverKind::Fly)) => {
            let fly = config
                .capacity
                .fly
                .as_ref()
                .ok_or_else(|| "validated Fly driver config missing".to_string())?;
            RuntimeCapacityDriverConfig::Fly {
                api_base_url: fly.api_base_url.clone(),
                app_name: fly.app_name.clone(),
                token_env: fly.token_env.clone(),
                image: fly.image.clone(),
                region: fly.region.clone(),
                pool: fly.pool.clone(),
                environment: fly.env.clone(),
                cpu_kind: fly.cpu_kind.clone(),
                cpus: fly.cpus,
                memory_mb: fly.memory_mb,
            }
        }
        (true, None) => return Err("validated autonomous capacity driver missing".to_string()),
    };
    let template_revision = if !config.autoscaling.enabled {
        Some("disabled-v1".to_string())
    } else {
        match config.capacity.driver {
            Some(CapacityDriverKind::Docker) => config
                .capacity
                .docker
                .as_ref()
                .map(|driver| driver.template_revision.clone()),
            Some(CapacityDriverKind::Fly) => config
                .capacity
                .fly
                .as_ref()
                .map(|driver| driver.template_revision.clone()),
            Some(CapacityDriverKind::Process) => Some("process-v1".to_string()),
            None => None,
        }
    }
    .ok_or_else(|| "validated capacity template revision missing".to_string())?;
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
    diag_opts: &DiagnosticOptions,
) -> Result<(), String> {
    let mut prepared = prepare_project_build(dir, test_builtins, diag_opts)?;
    // Without an entry function codegen emits no C `main`, and the linker's
    // "_main not found" is all the user would see.
    if artifact == BuildArtifact::Executable && prepared.merged_mir.entry_function.is_none() {
        return Err(format!(
            "{} has no `fn main()`: an executable starts there",
            prepared.entry_relative_path.display()
        ));
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
    )?;

    // Determine output path
    let project_name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("output");
    let output_path = match output {
        Some(p) => p.to_path_buf(),
        None => match artifact {
            BuildArtifact::Executable => dir.join(project_name),
            BuildArtifact::Staticlib => dir.join(format!("lib{project_name}.a")),
            BuildArtifact::Cdylib => {
                let extension = if target
                    .map(|triple| triple.contains("apple"))
                    .unwrap_or(cfg!(target_os = "macos"))
                {
                    "dylib"
                } else if target
                    .map(|triple| triple.contains("windows"))
                    .unwrap_or(cfg!(target_os = "windows"))
                {
                    "dll"
                } else {
                    "so"
                };
                dir.join(format!("lib{project_name}.{extension}"))
            }
        },
    };

    // Emit LLVM IR if requested
    if emit_llvm {
        let ll_path = output_path.with_extension("ll");
        let mut llvm_mir = prepared.merged_mir.clone();
        if artifact != BuildArtifact::Executable {
            llvm_mir.entry_function = None;
        }
        mesh_codegen::compile_mir_to_llvm_ir(
            &llvm_mir,
            &declared_handlers,
            &startup_work_registrations,
            prepared.autonomous_config_json.as_deref(),
            &prepared.library_exports,
            &ll_path,
            target,
        )?;
        eprintln!("  LLVM IR: {}", ll_path.display());
    }

    // Compile to native binary
    let runtime_flavor = if test_builtins {
        mesh_codegen::link::RuntimeFlavor::Test
    } else {
        mesh_codegen::link::RuntimeFlavor::Standard
    };
    let runtime_override = runtime_lib_override_from_env(runtime_flavor)?;
    let native_archives = if dir.join("mesh.toml").is_file() {
        let effective_target = mesh_codegen::link::effective_target_triple(target)?;
        mesh_pkg::resolve_native_archives(dir, &effective_target)?
            .into_iter()
            .map(|archive| archive.path)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    match artifact {
        BuildArtifact::Executable => mesh_codegen::compile_mir_to_binary(
            &prepared.merged_mir,
            &declared_handlers,
            &startup_work_registrations,
            prepared.autonomous_config_json.as_deref(),
            &prepared.library_exports,
            &output_path,
            opt_level,
            target,
            runtime_flavor,
            runtime_override.as_deref(),
            &native_archives,
        )?,
        BuildArtifact::Staticlib | BuildArtifact::Cdylib => {
            mesh_codegen::compile_mir_to_library(
                &prepared.merged_mir,
                &declared_handlers,
                &startup_work_registrations,
                prepared.autonomous_config_json.as_deref(),
                &prepared.library_exports,
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
            )?;
            library_bindings::write(&output_path, &prepared.library_exports, target)?;
        }
    }

    // `meshc test` builds each file to a throwaway temp binary; its path is noise there.
    if !test_builtins {
        eprintln!("  Compiled: {}", output_path.display());
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
            kind: match entry.kind {
                mesh_pkg::manifest::ClusteredDeclarationKind::Work => {
                    mesh_codegen::DeclaredHandlerKind::Work
                }
                mesh_pkg::manifest::ClusteredDeclarationKind::ServiceCall => {
                    mesh_codegen::DeclaredHandlerKind::ServiceCall
                }
                mesh_pkg::manifest::ClusteredDeclarationKind::ServiceCast => {
                    mesh_codegen::DeclaredHandlerKind::ServiceCast
                }
            },
            runtime_registration_name: entry.runtime_registration_name.clone(),
            executable_symbol: entry.executable_symbol.clone(),
            replication_count: entry.replication_count.value as u64,
        })
        .collect::<Vec<_>>();
    plan.extend(clustered_route_entries.iter().cloned());
    plan
}

pub(crate) fn prepare_project_build(
    dir: &Path,
    test_builtins: bool,
    diag_opts: &DiagnosticOptions,
) -> Result<PreparedBuild, String> {
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

    let manifest_path = dir.join("mesh.toml");
    let manifest = if manifest_path.exists() {
        Some(Manifest::from_file(&manifest_path)?)
    } else {
        None
    };
    let native_bindings = if manifest.is_some() {
        mesh_pkg::resolve_native_bindings(dir)?
    } else {
        Vec::new()
    };
    let entry_relative_path = resolve_entrypoint(dir, manifest.as_ref())?;

    // Build the project: discover all files, parse, build module graph
    let native_sources = native_bindings
        .iter()
        .map(|binding| discovery::ExtraMeshSource {
            path: binding.path.clone(),
            relative_path: binding.relative_path.clone(),
        })
        .collect::<Vec<_>>();
    let project = if test_builtins {
        discovery::build_test_project_with_entrypoint_and_sources(
            dir,
            &entry_relative_path,
            &native_sources,
        )?
    } else {
        discovery::build_project_with_entrypoint_and_sources(
            dir,
            &entry_relative_path,
            &native_sources,
        )?
    };

    // Find the entry module
    let entry_id = project
        .compilation_order
        .iter()
        .copied()
        .find(|id| project.graph.get(*id).is_entry)
        .ok_or_else(|| {
            format!(
                "Resolved entrypoint '{}' was not marked executable in module discovery",
                entry_relative_path.display()
            )
        })?;

    // Check parse errors in ALL modules (not just entry)
    let mut has_errors = false;
    for id in &project.compilation_order {
        let idx = id.0 as usize;
        let parse = &project.module_parses[idx];
        let source = &project.module_sources[idx];
        let module_path = dir.join(&project.graph.get(*id).path);

        for error in parse.errors() {
            has_errors = true;
            let file_name = diag_opts.display_path(&module_path);
            if diag_opts.json {
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
                }
                .with_index_type(ariadne::IndexType::Byte);
                let range = mesh_typeck::diagnostics::report_span(
                    &source,
                    error.span.start as usize..error.span.end as usize,
                );
                let span = (file_name.clone(), range);
                let _ = Report::build(ReportKind::Error, span.clone())
                    .with_message("Parse error")
                    .with_config(config)
                    .with_label(Label::new(span).with_message(&error.message))
                    .finish()
                    .eprint((file_name.clone(), Source::from(source.as_str())));
            }
        }
    }

    // If any parse errors exist, skip type checking entirely
    if has_errors {
        return Err("Compilation failed due to errors above.".to_string());
    }

    let allowed_native_bindings = native_bindings
        .iter()
        .map(|binding| binding.path.as_path())
        .collect::<HashSet<_>>();
    for id in &project.compilation_order {
        let idx = id.0 as usize;
        if !project.module_parses[idx]
            .tree()
            .fn_defs()
            .any(|function| function.native_decl().is_some())
        {
            continue;
        }
        let source_path = &project.graph.get(*id).path;
        let full_path = if source_path.is_absolute() {
            source_path.clone()
        } else {
            dir.join(source_path)
        };
        let canonical = full_path.canonicalize().map_err(|error| {
            format!(
                "Failed to resolve native binding source '{}': {error}",
                diag_opts.display_path(&full_path)
            )
        })?;
        if !allowed_native_bindings.contains(canonical.as_path()) {
            return Err(format!(
                "Native declaration in '{}' is outside a manifest-declared native binding",
                diag_opts.display_path(&full_path)
            ));
        }
    }

    reject_duplicate_type_names(&project)?;

    // Type-check ALL modules in topological order (Phase 39)
    let module_count = project.graph.module_count();
    let mut all_exports: Vec<Option<mesh_typeck::ExportedSymbols>> =
        (0..module_count).map(|_| None).collect();
    let mut all_typeck: Vec<Option<mesh_typeck::TypeckResult>> =
        (0..module_count).map(|_| None).collect();
    let mut has_type_errors = false;

    for &id in &project.compilation_order {
        let idx = id.0 as usize;
        let parse = &project.module_parses[idx];
        let source = &project.module_sources[idx];
        let module_path = dir.join(&project.graph.get(id).path);

        // Build ImportContext from already-checked dependencies
        let mut import_ctx = build_import_context(&project.graph, &all_exports, parse, id);

        // Thread the current module's name (clustered route handlers are
        // named with it).
        let module_name = &project.graph.get(id).name;
        import_ctx.current_module = Some(module_name.clone());
        import_ctx.test_builtins = test_builtins;

        // Type-check this module with imports
        let typeck = mesh_typeck::check_with_imports(parse, &import_ctx);

        // Report type-check diagnostics for this module
        let file_name = diag_opts.display_path(&module_path);
        for error in &typeck.errors {
            has_type_errors = true;
            let rendered = mesh_typeck::diagnostics::render_diagnostic(
                error, source, &file_name, diag_opts, None,
            );
            eprint!("{}", rendered);
        }

        // Report warnings
        for warning in &typeck.warnings {
            let rendered = mesh_typeck::diagnostics::render_diagnostic(
                warning, source, &file_name, diag_opts, None,
            );
            eprint!("{}", rendered);
        }

        // Collect exports for downstream modules
        let exports = mesh_typeck::collect_exports(parse, &typeck);
        all_exports[idx] = Some(exports);
        all_typeck[idx] = Some(typeck);
    }

    if has_type_errors {
        return Err("Compilation failed due to errors above.".to_string());
    }

    reject_duplicate_pub_functions(&project, &all_exports)?;

    // `@cluster` and `HTTP.clustered` without a count take the manifest's
    // `[cluster].default_replicas`.
    let default_replicas = manifest
        .as_ref()
        .and_then(|manifest| manifest.autonomous_cluster.as_ref())
        .map_or(mesh_pkg::DEFAULT_CLUSTER_REPLICATION_COUNT, |cluster| {
            cluster.default_replicas
        });
    let source_cluster_declarations =
        collect_source_cluster_declarations(&project.graph, &project.module_parses);
    let mut clustered_execution_plan = if !source_cluster_declarations.is_empty() {
        let surface =
            build_clustered_export_surface(&project.graph, &project.module_parses, &all_exports);
        match validate_cluster_declarations_with_source(&source_cluster_declarations, &surface) {
            Ok(metadata) => metadata,
            Err(issues) => {
                emit_clustered_declaration_diagnostics(&manifest_path, &issues, diag_opts);
                return Err("Compilation failed due to errors above.".to_string());
            }
        }
    } else {
        Vec::new()
    };
    for entry in &mut clustered_execution_plan {
        if entry.replication_count.source == mesh_pkg::ClusteredReplicationCountSource::Default {
            entry.replication_count.value = default_replicas;
        }
    }
    let clustered_route_handler_plan = mesh_codegen::prepare_clustered_route_handler_plan(
        all_typeck.iter().filter_map(|typeck| typeck.as_ref()),
        default_replicas,
    )?;

    let inferred_export_names: HashSet<String> = all_exports
        .iter()
        .filter_map(|exports_opt| exports_opt.as_ref())
        .flat_map(|exports| {
            exports.functions.iter().filter_map(|(name, scheme)| {
                if ty_contains_var(&scheme.ty) {
                    Some(name.clone())
                } else {
                    None
                }
            })
        })
        .collect();
    let inferred_fn_usage_types = collect_inferred_fn_usage_types(
        &project.module_parses,
        &all_typeck,
        &inferred_export_names,
    );

    let library_exports = collect_library_exports(&project.module_parses)?;

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
        if let Some(typeck) = all_typeck[id.0 as usize].as_mut() {
            for impl_def in later_impls {
                let _ = typeck.trait_registry.register_impl(impl_def);
            }
        }
    }

    // Lower ALL modules to MIR and merge into a single module for codegen.
    let mut mir_modules = Vec::new();
    let mut entry_mir_idx = 0;
    for (i, &id) in project.compilation_order.iter().enumerate() {
        let idx = id.0 as usize;
        let parse = &project.module_parses[idx];
        let typeck = all_typeck[idx]
            .as_ref()
            .ok_or("Module was not type-checked")?;

        // Build set of pub function names for module-qualified naming (Phase 41)
        let module_name = &project.graph.get(id).name;
        let pub_fns: std::collections::HashSet<String> = all_exports[idx]
            .as_ref()
            .map(|e| e.functions.keys().cloned().collect())
            .unwrap_or_default();

        let other_modules: Vec<_> = project
            .module_parses
            .iter()
            .zip(&all_typeck)
            .enumerate()
            .filter(|(other, _)| *other != idx)
            .filter_map(|(_, (parse, typeck))| Some((parse, typeck.as_ref()?)))
            .collect();
        let mir = mesh_codegen::lower_module_to_mir_raw(
            parse,
            typeck,
            module_name,
            &pub_fns,
            &inferred_fn_usage_types,
            &other_modules,
        )?;
        if id == entry_id {
            entry_mir_idx = i;
        }
        mir_modules.push(mir);
    }
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
        )?,
    })
}

/// Reject programs where two modules export a `pub fn` under the same symbol.
///
/// `Lowerer::qualify_name` deliberately leaves `pub` function names unqualified so
/// importing modules can call them by their bare name, and `merge_mir_modules` then
/// keys merged functions by that bare name, keeping the first one it sees. Two modules
/// exporting the same name therefore collapse into one symbol: the later module's body
/// is dropped, and every call site — including `from B import f` — binds to the earlier
/// module's body while the type checker went on believing it had `B`'s signature. That
/// mismatch is silent, and when the signatures differ it reinterprets values across
/// types (an `Int` returned where a `String` was expected becomes a wild pointer).
///
/// Until pub symbols are module-qualified, refuse to compile the ambiguous program.
/// Reject a struct, sum type, interface, actor, service or supervisor name
/// that more than one module defines, public or not. These names are one
/// namespace across a project: the type checker took two `State` structs
/// for one type and codegen emitted one of them, so `Z.run_z()` showed its
/// `State` laid out as `A`'s, and a module's `spawn(worker)` could start
/// another module's `worker`.
fn reject_duplicate_type_names(project: &discovery::ProjectData) -> Result<(), String> {
    use mesh_parser::ast::item::Item;

    // A struct or sum type copied verbatim into several modules (a private
    // helper) is one layout, so its definitions are compared by their
    // tokens; code (actors, services) may call each module's own functions.
    let fingerprint = |item: &Item| -> Option<String> {
        if !matches!(item, Item::StructDef(_) | Item::SumTypeDef(_)) {
            return None;
        }
        let tokens: Vec<String> = item
            .syntax()
            .descendants_with_tokens()
            .filter_map(|element| element.into_token())
            .filter(|token| !token.kind().is_trivia() && token.kind() != SyntaxKind::PUB_KW)
            .map(|token| token.text().to_string())
            .collect();
        Some(tokens.join(" "))
    };
    let mut owners: std::collections::BTreeMap<
        String,
        Vec<(
            &'static str,
            &mesh_common::module_graph::ModuleInfo,
            Option<String>,
        )>,
    > = Default::default();
    for &id in &project.compilation_order {
        let module = project.graph.get(id);
        for item in project.module_parses[id.0 as usize].tree().items() {
            let (kind, name) = match &item {
                Item::StructDef(def) => ("struct", def.name().and_then(|n| n.text())),
                Item::SumTypeDef(def) => ("type", def.name().and_then(|n| n.text())),
                Item::InterfaceDef(def) => ("interface", def.name().and_then(|n| n.text())),
                Item::ActorDef(def) => ("actor", def.name().and_then(|n| n.text())),
                Item::ServiceDef(def) => ("service", def.name().and_then(|n| n.text())),
                Item::SupervisorDef(def) => ("supervisor", def.name().and_then(|n| n.text())),
                _ => continue,
            };
            if let Some(name) = name {
                owners
                    .entry(name)
                    .or_default()
                    .push((kind, module, fingerprint(&item)));
            }
        }
    }

    let mut conflicted = false;
    for (name, defs) in &owners {
        let first = defs[0].1.name.as_str();
        if defs.iter().all(|(_, module, _)| module.name == first) {
            // One module (a duplicate there is the type checker's E0068).
            continue;
        }
        if defs[0].2.is_some() && defs.iter().all(|(_, _, print)| *print == defs[0].2) {
            continue;
        }
        conflicted = true;
        eprintln!("error: `{name}` is defined in more than one module:");
        for (kind, module, _) in defs {
            eprintln!(
                "  - {kind} in `{}` ({})",
                module.name,
                module.path.display()
            );
        }
        eprintln!(
            "note: struct, type, interface, actor, service and supervisor names share one \
             namespace across the project, private ones too, so these definitions would be \
             taken for one (a struct or type defined identically in each is fine)."
        );
        eprintln!("help: rename all but one of them.");
    }

    if conflicted {
        return Err("Compilation failed due to errors above.".to_string());
    }
    Ok(())
}

fn reject_duplicate_pub_functions(
    project: &discovery::ProjectData,
    all_exports: &[Option<mesh_typeck::ExportedSymbols>],
) -> Result<(), String> {
    // Export keys are exactly the `pub_fns` handed to the lowerer, so a key collision
    // is a symbol collision. Same-name/different-arity pub fns inside one module are
    // already disambiguated by `collect_exports` as `name__<arity>`.
    let mut owners: std::collections::BTreeMap<&str, Vec<&mesh_common::module_graph::ModuleInfo>> =
        Default::default();
    for &id in &project.compilation_order {
        let Some(exports) = all_exports.get(id.0 as usize).and_then(Option::as_ref) else {
            continue;
        };
        let module = project.graph.get(id);
        for name in exports.functions.keys() {
            owners.entry(name.as_str()).or_default().push(module);
        }
    }

    let mut conflicted = false;
    for (symbol, modules) in owners.iter().filter(|(_, m)| m.len() > 1) {
        conflicted = true;
        // Strip the `__<arity>` suffix `collect_exports` adds to overloaded pub fns.
        let display = symbol
            .rsplit_once("__")
            .filter(|(_, arity)| !arity.is_empty() && arity.chars().all(|c| c.is_ascii_digit()))
            .map(|(base, _)| base)
            .unwrap_or(symbol);
        eprintln!("error: public function `{display}` is defined in more than one module:");
        for module in modules {
            eprintln!("  - `{}` ({})", module.name, module.path.display());
        }
        eprintln!(
            "note: public function names share one global symbol space, so a call to \
             `{display}` would silently run only one of these definitions."
        );
        eprintln!("help: rename all but one of them, or make the others private.");
    }

    if conflicted {
        return Err("Compilation failed due to errors above.".to_string());
    }
    Ok(())
}

fn collect_library_exports(
    parses: &[mesh_parser::Parse],
) -> Result<Vec<mesh_codegen::LibraryExport>, String> {
    let mut symbols = HashSet::new();
    let mut exports = Vec::new();
    for parse in parses {
        for function in parse.tree().fn_defs() {
            let Some(declaration) = function.export_decl() else {
                continue;
            };
            let function_name = function
                .name()
                .and_then(|name| name.text())
                .ok_or("exported function is missing a name")?;
            let symbol = declaration
                .symbol()
                .ok_or("exported function is missing a symbol")?;
            if !symbols.insert(symbol.clone()) {
                return Err(format!("duplicate exported symbol '{symbol}'"));
            }
            exports.push(mesh_codegen::LibraryExport {
                function: function_name,
                symbol,
            });
        }
    }
    Ok(exports)
}

fn runtime_lib_override_from_env(
    runtime_flavor: mesh_codegen::link::RuntimeFlavor,
) -> Result<Option<PathBuf>, String> {
    let variable = match runtime_flavor {
        mesh_codegen::link::RuntimeFlavor::Standard => "MESH_RT_LIB_PATH",
        mesh_codegen::link::RuntimeFlavor::Test => "MESH_TEST_RT_LIB_PATH",
    };
    let Some(raw) = std::env::var_os(variable) else {
        return Ok(None);
    };

    if raw.is_empty() {
        return Err(format!(
            "{variable} was set but empty. Provide an absolute path to the selected Mesh runtime static library or unset it."
        ));
    }

    Ok(Some(PathBuf::from(raw)))
}

fn ty_contains_var(ty: &Ty) -> bool {
    match ty {
        Ty::Var(_) => true,
        Ty::Con(_) | Ty::Never => false,
        Ty::Fun(params, ret) => params.iter().any(ty_contains_var) || ty_contains_var(ret),
        Ty::App(con, args) => ty_contains_var(con) || args.iter().any(ty_contains_var),
        Ty::Tuple(elems) => elems.iter().any(ty_contains_var),
    }
}

fn is_concrete_fn_ty(ty: &Ty) -> bool {
    matches!(ty, Ty::Fun(..)) && !ty_contains_var(ty)
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
    typecks: &[Option<mesh_typeck::TypeckResult>],
    candidate_names: &HashSet<String>,
) -> HashMap<String, Vec<Ty>> {
    let mut usage = HashMap::new();
    if candidate_names.is_empty() {
        return usage;
    }

    for (parse, typeck_opt) in parses.iter().zip(typecks.iter()) {
        let Some(typeck) = typeck_opt.as_ref() else {
            continue;
        };

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
        for node in parse.syntax().descendants() {
            match node.kind() {
                SyntaxKind::NAME_REF => {
                    if let Some(name_ref) = NameRef::cast(node.clone()) {
                        if let Some(name) = overload_target(&node).or_else(|| name_ref.text()) {
                            if candidate_names.contains(&name) {
                                if let Some(ty) = typeck.types.get(&name_ref.syntax().text_range())
                                {
                                    push_usage_type(&mut usage, &name, ty);
                                }
                            }
                        }
                    }
                }
                SyntaxKind::FIELD_ACCESS => {
                    if let Some(field_access) = FieldAccess::cast(node) {
                        let Some(base_expr) = field_access.base() else {
                            continue;
                        };
                        let mesh_parser::ast::expr::Expr::NameRef(base_name_ref) = base_expr else {
                            continue;
                        };
                        let Some(base_name) = base_name_ref.text() else {
                            continue;
                        };
                        if !typeck.qualified_modules.contains_key(&base_name) {
                            continue;
                        }
                        let Some(field_name) = overload_target(field_access.syntax())
                            .or_else(|| field_access.field().map(|t| t.text().to_string()))
                        else {
                            continue;
                        };
                        if !candidate_names.contains(&field_name) {
                            continue;
                        }
                        if let Some(ty) = typeck.types.get(&field_access.syntax().text_range()) {
                            push_usage_type(&mut usage, &field_name, ty);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    usage
}

/// Build an ImportContext for a module from already-checked dependency exports.
///
/// Reads the module's import declarations to determine which modules are imported,
/// then constructs an ImportContext with the exports of those modules. Trait defs
/// and impls from ALL already-checked modules are included (XMOD-05: globally visible).
fn build_import_context(
    graph: &mesh_common::module_graph::ModuleGraph,
    all_exports: &[Option<mesh_typeck::ExportedSymbols>],
    parse: &mesh_parser::Parse,
    _module_id: mesh_common::module_graph::ModuleId,
) -> mesh_typeck::ImportContext {
    use mesh_parser::ast::item::Item;
    use mesh_typeck::{ImportContext, ModuleExports};

    let mut ctx = ImportContext::empty();

    // Collect ALL trait defs and impls from ALL already-checked modules (XMOD-05)
    for exports_opt in all_exports.iter() {
        if let Some(exports) = exports_opt {
            ctx.all_trait_defs
                .extend(exports.trait_defs.iter().cloned());
            ctx.all_trait_impls
                .extend(exports.trait_impls.iter().cloned());
        }
    }

    // For each import declaration in this module, find the corresponding
    // module's exports and add them to the ImportContext.
    let tree = parse.tree();
    for item in tree.items() {
        let segments = match &item {
            Item::ImportDecl(import_decl) => import_decl.module_path().map(|p| p.segments()),
            Item::FromImportDecl(from_import) => from_import.module_path().map(|p| p.segments()),
            _ => None,
        };

        if let Some(segments) = segments {
            let full_name = segments.join(".");
            let last_segment = segments.last().cloned().unwrap_or_default();

            // Look up the module in the graph
            if let Some(dep_id) = graph.resolve(&full_name) {
                let idx = dep_id.0 as usize;
                if let Some(Some(exports)) = all_exports.get(idx) {
                    // Build ModuleExports from ExportedSymbols
                    let mod_exports = ModuleExports {
                        module_name: full_name.clone(),
                        functions: exports.functions.clone(),
                        struct_defs: exports.struct_defs.clone(),
                        sum_type_defs: exports.sum_type_defs.clone(),
                        service_defs: exports.service_defs.clone(),
                        actor_defs: exports.actor_defs.clone(),
                        private_names: exports.private_names.clone(),
                        type_aliases: exports.type_aliases.clone(),
                        resource_types: exports.resource_types.clone(),
                        function_ownership: exports.function_ownership.clone(),
                        interfaces: exports
                            .trait_defs
                            .iter()
                            .map(|interface| interface.name.clone())
                            .collect(),
                    };
                    ctx.module_exports.insert(last_segment, mod_exports);
                }
            }
            // If module not found in graph, that's fine -- the type checker
            // will emit ImportModuleNotFound when it processes the import.
        }
    }

    ctx
}

fn clustered_issue_file_and_span(
    manifest_path: &Path,
    issue: &ClusteredDeclarationError,
) -> (String, Option<String>, Option<std::ops::Range<usize>>) {
    let Some(provenance) = issue.origin.provenance() else {
        return (manifest_path.display().to_string(), None, None);
    };

    let project_root = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let file_path = project_root.join(&provenance.file);
    let file_name = file_path.display().to_string();
    let source = std::fs::read_to_string(&file_path).ok();
    let span = source
        .as_ref()
        .map(|source| clustered_issue_range(source, provenance.span));
    (file_name, source, span)
}

fn clustered_issue_range(source: &str, span: mesh_common::span::Span) -> std::ops::Range<usize> {
    if source.is_empty() {
        return 0..0;
    }

    let mut start = (span.start as usize).min(source.len() - 1);
    let mut end = (span.end as usize).min(source.len());
    if end <= start {
        end = (start + 1).min(source.len());
    }
    start = start.min(end.saturating_sub(1));
    start..end
}

fn offset_to_line_col(source: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(source.len());
    let prefix = &source[..offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let col = prefix
        .rsplit_once('\n')
        .map(|(_, line_text)| line_text.chars().count() + 1)
        .unwrap_or_else(|| prefix.chars().count() + 1);
    (line, col)
}

fn emit_clustered_declaration_diagnostics(
    manifest_path: &Path,
    issues: &[ClusteredDeclarationError],
    diag_opts: &DiagnosticOptions,
) {
    for issue in issues {
        let (file_name, source, span) = clustered_issue_file_and_span(manifest_path, issue);

        if diag_opts.json {
            let spans = span
                .as_ref()
                .map(|span| {
                    vec![serde_json::json!({
                        "start": span.start,
                        "end": span.end,
                        "label": issue.reason
                    })]
                })
                .unwrap_or_default();
            let json_diag = serde_json::json!({
                "code": "CFG0001",
                "severity": "error",
                "message": issue.to_string(),
                "file": file_name,
                "spans": spans,
                "fix": null
            });
            eprintln!("{}", json_diag);
            continue;
        }

        if let (Some(source), Some(span)) = (source.as_ref(), span.as_ref()) {
            use ariadne::{Config, Label, Report, ReportKind, Source};

            let config = if diag_opts.color {
                Config::default()
            } else {
                Config::default().with_color(false)
            }
            .with_index_type(ariadne::IndexType::Byte);
            let (line, col) = offset_to_line_col(source, span.start);
            eprintln!("error: {}", issue);
            eprintln!("  --> {}:{}:{}", file_name, line, col);
            let _ = Report::<std::ops::Range<usize>>::build(ReportKind::Error, span.clone())
                .with_message("Invalid clustered declaration")
                .with_config(config)
                .with_label(Label::new(span.clone()).with_message(&issue.reason))
                .finish()
                .eprint(Source::from(source.as_str()));
        } else {
            eprintln!("error: {}", issue);
            eprintln!("  --> {}", file_name);
        }
    }
}

// ── Deps subcommand ──────────────────────────────────────────────────

/// Execute the `deps` subcommand: resolve dependencies and generate mesh.lock.
///
/// If mesh.lock already exists and the manifest hasn't changed, skips resolution.
fn deps_command(dir: &Path) -> Result<(), String> {
    let manifest_path = dir.join("mesh.toml");
    if !manifest_path.exists() {
        return Err(format!(
            "No 'mesh.toml' found in '{}'. Run `meshc init` to create a project.",
            dir.display()
        ));
    }

    let lock_path = dir.join("mesh.lock");

    // The lockfile is fresh when the manifest has not changed since it was
    // written and every git dependency is checked out: `meshpkg install`
    // writes it too, without fetching git dependencies.
    let git_checkouts_present = mesh_pkg::Manifest::from_file(&manifest_path)?
        .dependencies
        .iter()
        .filter(|(_, dep)| matches!(dep, mesh_pkg::manifest::Dependency::Git { .. }))
        .all(|(name, _)| dir.join(".mesh").join("deps").join(name).is_dir());
    if git_checkouts_present && lock_path.exists() {
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

    let (resolved, lockfile) = mesh_pkg::resolve_dependencies(dir)?;

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

/// Execute the `fmt` subcommand: format Mesh source files in-place or check formatting.
fn fmt_command(
    path: &Path,
    check: bool,
    config: &mesh_fmt::FormatConfig,
) -> Result<FmtStats, String> {
    let files = collect_mesh_files(path)?;
    if files.is_empty() {
        return Err(format!("No .mpl files found at '{}'", path.display()));
    }

    // Format every file before writing any: a file that cannot be formatted
    // leaves the whole tree as it was.
    let mut changed = Vec::new();
    for file in &files {
        let source = std::fs::read_to_string(file)
            .map_err(|e| format!("Failed to read '{}': {}", file.display(), e))?;
        let formatted = mesh_fmt::try_format(&source, config)
            .map_err(|reason| format!("Cannot format '{}': {}", file.display(), reason))?;
        if formatted != source {
            changed.push((file, formatted));
        }
    }

    for (file, formatted) in &changed {
        if check {
            eprintln!("  would reformat: {}", file.display());
        } else {
            std::fs::write(file, formatted)
                .map_err(|e| format!("Failed to write '{}': {}", file.display(), e))?;
        }
    }

    Ok(FmtStats {
        total: files.len(),
        unformatted: if check { changed.len() } else { 0 },
    })
}

/// Execute the `lint` subcommand: print each finding (and each file that does
/// not parse) as `path:line:column: rule: message` and return how many there were.
fn lint_command(path: &Path) -> Result<usize, String> {
    let files = collect_mesh_files(path)?;
    if files.is_empty() {
        return Err(format!("No .mpl files found at '{}'", path.display()));
    }

    let mut problems = 0;
    for file in &files {
        let source = std::fs::read_to_string(file)
            .map_err(|e| format!("Failed to read '{}': {}", file.display(), e))?;
        let findings = match mesh_lint::lint(&source) {
            Ok(lints) => lints
                .into_iter()
                .map(|lint| (lint.offset, lint.rule, lint.message))
                .collect(),
            Err(error) => vec![(error.span.start, "parse-error", error.message)],
        };
        let lines = mesh_common::span::LineIndex::new(&source);
        for (offset, rule, message) in findings {
            let (line, column) = lines.line_col(offset);
            println!("{}:{line}:{column}: {rule}: {message}", file.display());
            problems += 1;
        }
    }
    Ok(problems)
}

/// Collect `.mpl` files from a path. If the path is a file, return it directly.
/// If it is a directory, recursively find all `.mpl` files.
fn collect_mesh_files(path: &Path) -> Result<Vec<PathBuf>, String> {
    if !path.exists() {
        return Err(format!("Path '{}' does not exist", path.display()));
    }

    if path.is_file() {
        if path.extension().and_then(|e| e.to_str()) == Some("mpl") {
            return Ok(vec![path.to_path_buf()]);
        } else {
            return Err(format!("'{}' is not a .mpl file", path.display()));
        }
    }

    if path.is_dir() {
        let mut files = Vec::new();
        collect_mesh_files_recursive(path, &mut files)
            .map_err(|e| format!("Failed to walk directory '{}': {}", path.display(), e))?;
        files.sort();
        return Ok(files);
    }

    Err(format!("'{}' is not a file or directory", path.display()))
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

    #[test]
    fn library_exports_are_collected_as_reachability_roots_and_symbols_are_unique() {
        let first = mesh_parser::parse(
            "@export(\"mesh_mobile_echo\")\npub fn echo(request :: Bytes) -> Bytes ! String do\n  Ok(request)\nend\n",
        );
        let second = mesh_parser::parse(
            "@export(\"mesh_mobile_status\")\npub fn status(request :: Bytes) -> Bytes ! String do\n  Ok(request)\nend\n",
        );
        let exports = collect_library_exports(&[first, second]).expect("valid exports");
        assert_eq!(exports[0].function, "echo");
        assert_eq!(exports[0].symbol, "mesh_mobile_echo");

        let duplicate = mesh_parser::parse(
            "@export(\"mesh_mobile_echo\")\npub fn again(request :: Bytes) -> Bytes ! String do\n  Ok(request)\nend\n",
        );
        assert!(collect_library_exports(&[
            mesh_parser::parse(
                "@export(\"mesh_mobile_echo\")\npub fn echo(request :: Bytes) -> Bytes ! String do\n  Ok(request)\nend\n",
            ),
            duplicate,
        ])
        .unwrap_err()
        .contains("duplicate exported symbol"));
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
}
