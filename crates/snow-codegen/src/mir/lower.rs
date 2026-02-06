//! AST-to-MIR lowering.
//!
//! Converts the typed Rowan CST (Parse + TypeckResult) to the MIR representation.
//! Handles desugaring of pipe operators, string interpolation, and closure conversion.
//! Full implementation in Task 2.
