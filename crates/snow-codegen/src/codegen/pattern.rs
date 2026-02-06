//! Decision tree to LLVM branch/switch translation.
//!
//! Translates compiled `DecisionTree` nodes from the pattern compilation
//! phase into LLVM basic blocks with switch instructions, conditional
//! branches, and variable bindings.
//!
//! Full implementation in Task 2.
