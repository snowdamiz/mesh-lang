//! Event-based parser for Snow.
//!
//! The parser consumes a token stream and produces events (Open/Close/Advance)
//! that are later converted into a rowan green tree. This decouples parsing
//! logic from tree construction.
//!
//! Full implementation comes in Task 2.
