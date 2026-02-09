//! Module graph types for the Snow compiler.
//!
//! Provides the core data structures used by all module-system phases:
//! [`ModuleId`], [`ModuleInfo`], [`ModuleGraph`], and [`CycleError`].

use std::fmt;
use std::path::PathBuf;

use rustc_hash::FxHashMap;

/// A unique identifier for a module within a compilation unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModuleId(pub u32);

/// Metadata about a single module in the module graph.
#[derive(Debug)]
pub struct ModuleInfo {
    /// Unique identifier for this module.
    pub id: ModuleId,
    /// PascalCase module name, e.g. `"Math.Vector"`.
    pub name: String,
    /// Path relative to the project root, e.g. `"math/vector.snow"`.
    pub path: PathBuf,
    /// Modules that this module depends on (via `import` statements).
    pub dependencies: Vec<ModuleId>,
    /// Whether this module is the project entry point (`main.snow`).
    pub is_entry: bool,
}

/// Error returned when a dependency cycle is detected in the module graph.
#[derive(Debug, Clone)]
pub struct CycleError {
    /// The module names forming the cycle, e.g. `["A", "B", "C", "A"]`.
    pub cycle_path: Vec<String>,
}

impl fmt::Display for CycleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.cycle_path.join(" -> "))
    }
}

/// A directed graph of modules and their dependencies.
///
/// Modules are stored in insertion order and identified by [`ModuleId`].
/// Name-based lookup is provided via an internal hash map.
pub struct ModuleGraph {
    /// All modules in the graph, indexed by `ModuleId.0`.
    pub modules: Vec<ModuleInfo>,
    /// Maps PascalCase module names to their [`ModuleId`].
    name_to_id: FxHashMap<String, ModuleId>,
}

impl ModuleGraph {
    /// Create an empty module graph.
    pub fn new() -> Self {
        Self {
            modules: Vec::new(),
            name_to_id: FxHashMap::default(),
        }
    }

    /// Add a module to the graph and return its assigned [`ModuleId`].
    ///
    /// The ID is assigned sequentially starting from 0.
    pub fn add_module(&mut self, name: String, path: PathBuf, is_entry: bool) -> ModuleId {
        let id = ModuleId(self.modules.len() as u32);
        self.name_to_id.insert(name.clone(), id);
        self.modules.push(ModuleInfo {
            id,
            name,
            path,
            dependencies: Vec::new(),
            is_entry,
        });
        id
    }

    /// Look up a module by its PascalCase name.
    pub fn resolve(&self, name: &str) -> Option<ModuleId> {
        self.name_to_id.get(name).copied()
    }

    /// Record that module `from` depends on module `to`.
    pub fn add_dependency(&mut self, from: ModuleId, to: ModuleId) {
        self.modules[from.0 as usize].dependencies.push(to);
    }

    /// Return the number of modules in the graph.
    pub fn module_count(&self) -> usize {
        self.modules.len()
    }

    /// Get a reference to a module by its [`ModuleId`].
    pub fn get(&self, id: ModuleId) -> &ModuleInfo {
        &self.modules[id.0 as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_and_resolve() {
        let mut graph = ModuleGraph::new();
        let id_a = graph.add_module("Math.Vector".into(), "math/vector.snow".into(), false);
        let id_b = graph.add_module("Utils".into(), "utils.snow".into(), false);

        assert_eq!(graph.resolve("Math.Vector"), Some(id_a));
        assert_eq!(graph.resolve("Utils"), Some(id_b));
        assert_ne!(id_a, id_b);
        assert_eq!(graph.module_count(), 2);
    }

    #[test]
    fn test_resolve_unknown() {
        let graph = ModuleGraph::new();
        assert_eq!(graph.resolve("Nonexistent"), None);
    }

    #[test]
    fn test_add_dependency() {
        let mut graph = ModuleGraph::new();
        let id_a = graph.add_module("A".into(), "a.snow".into(), false);
        let id_b = graph.add_module("B".into(), "b.snow".into(), false);

        graph.add_dependency(id_a, id_b);

        let module_a = graph.get(id_a);
        assert_eq!(module_a.dependencies, vec![id_b]);

        // B should have no dependencies
        let module_b = graph.get(id_b);
        assert!(module_b.dependencies.is_empty());
    }

    #[test]
    fn test_entry_module() {
        let mut graph = ModuleGraph::new();
        let entry_id = graph.add_module("Main".into(), "main.snow".into(), true);
        let lib_id = graph.add_module("Lib".into(), "lib.snow".into(), false);

        assert!(graph.get(entry_id).is_entry);
        assert!(!graph.get(lib_id).is_entry);
    }
}
