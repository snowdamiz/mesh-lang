//! Supervisor runtime for Snow actors.
//!
//! Implements OTP-style supervision with four restart strategies
//! (one_for_one, one_for_all, rest_for_one, simple_one_for_one),
//! restart limit tracking via sliding window, ordered shutdown with
//! timeout/brutal_kill, and child lifecycle management.
//!
//! The supervisor is an actor that traps exits and manages child lifecycles.
//! It receives exit signals from linked children and applies the configured
//! restart strategy.
//!
//! Populated in Task 2.
