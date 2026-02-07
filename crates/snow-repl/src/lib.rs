//! Interactive REPL with LLVM JIT compilation for the Snow language.
//!
//! This crate implements a Read-Eval-Print Loop that uses the full Snow compiler
//! pipeline (parse -> typecheck -> MIR -> LLVM IR) with JIT execution. This
//! ensures REPL behavior is identical to compiled code.
//!
//! ## Architecture
//!
//! - [`jit`]: JIT compilation engine -- compiles and executes Snow expressions
//! - [`session`]: Session state management -- tracks definitions and results
//!
//! ## Usage
//!
//! ```no_run
//! use snow_repl::{ReplConfig, run_repl};
//!
//! let config = ReplConfig::default();
//! run_repl(&config).unwrap();
//! ```

pub mod jit;
pub mod session;

pub use jit::{jit_eval, EvalResult};
pub use session::ReplSession;

/// Configuration for the REPL.
pub struct ReplConfig {
    /// The primary prompt string (default: "snow> ").
    pub prompt: String,
    /// The continuation prompt for multi-line input (default: "  ... ").
    pub continuation: String,
}

impl Default for ReplConfig {
    fn default() -> Self {
        Self {
            prompt: "snow> ".to_string(),
            continuation: "  ... ".to_string(),
        }
    }
}

/// Run the interactive REPL loop.
///
/// This is the main entry point for the REPL. It reads input from the user,
/// evaluates it using JIT compilation, and prints the result.
///
/// Full implementation with rustyline integration is provided in Plan 05.
/// For now, this exports the core types and JIT engine.
pub fn run_repl(_config: &ReplConfig) -> Result<(), String> {
    // Full rustyline-based REPL loop will be implemented in Plan 05.
    // The JIT engine and session management are ready for integration.
    Err("REPL loop not yet implemented -- use jit_eval() directly".to_string())
}
