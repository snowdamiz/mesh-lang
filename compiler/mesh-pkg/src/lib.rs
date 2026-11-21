pub mod autonomous;
pub mod frontend;
pub mod lockfile;
pub mod manifest;
pub mod native;
pub mod project;
pub mod resolver;
pub mod scaffold;
pub mod toolchain_update;

// Re-export key types for convenience.
pub use autonomous::{
    AutonomousClusterConfig, AutoscalingConfig, ByteSize, CapacityConfig, CapacityDriverKind,
    ClusterFeatureConfig, ClusterMode, ContinuityConfig, ControllerConfig, DockerDriverConfig,
    DurabilityMode, ForcedTerminationPolicy, HumanDuration, ManagedRole, ProcessDriverConfig,
    RoleConfig, RoutingAlgorithm, RoutingConfig, SchedulerConfig, DEFAULT_TRANSPORT_FRAME_BYTES,
    MAX_TOTAL_REPLICAS,
};
pub use lockfile::{LockedPackage, Lockfile};
pub use manifest::{
    plan_cluster_declarations, ClusteredDeclarationError, ClusteredExecutionMetadata,
    ClusteredReplicationCount, ClusteredReplicationCountSource, Dependency, Manifest,
    NativeLibrary, NativePackage, Package, DEFAULT_CLUSTER_REPLICATION_COUNT,
};
pub use native::{
    checked_package_file, resolve_native_archives, resolve_native_bindings, ResolvedNativeArchive,
    ResolvedNativeBinding,
};
pub use resolver::resolve_dependencies;
pub use scaffold::{
    scaffold_clustered_project, scaffold_project, scaffold_todo_api_project,
    scaffold_todo_api_project_with_db, TodoApiDatabase,
};
pub use toolchain_update::{
    run_toolchain_update, run_toolchain_update_with_progress, ToolchainUpdateError,
    ToolchainUpdateMode, ToolchainUpdateOutcome,
};

fn toml_error(
    code: mesh_diagnostics::Code,
    file: &mesh_diagnostics::SourceFile,
    context: impl std::fmt::Display,
    error: toml::de::Error,
) -> mesh_diagnostics::Diagnostic {
    let diagnostic = mesh_diagnostics::Diagnostic::in_file(
        code,
        file,
        format!("{context}: {}", error.message()),
    );
    match error.span() {
        Some(range) => diagnostic.at(
            mesh_common::span::Span::new(range.start as u32, range.end as u32),
            "",
        ),
        None => diagnostic,
    }
}
