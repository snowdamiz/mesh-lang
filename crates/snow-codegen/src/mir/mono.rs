//! Monomorphization pass.
//!
//! Takes a MIR module with potentially generic functions and produces a module
//! containing only monomorphic (concrete-typed) functions. Full implementation
//! in Task 2.
