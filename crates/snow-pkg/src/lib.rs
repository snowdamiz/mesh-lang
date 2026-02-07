pub mod lockfile;
pub mod manifest;
pub mod resolver;
pub mod scaffold;

// Re-export key types for convenience.
pub use manifest::Manifest;
pub use resolver::resolve_dependencies;
pub use scaffold::scaffold_project;
