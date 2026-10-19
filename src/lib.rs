//! Headless D&D 5e (2024 rules) combat simulator.
//!
//! The crate is split into small modules so that the rules engine, the
//! encounter state and the command-line interface can be tested independently.
//! Step 1 provides dice, data loading and persisted encounter state.

pub mod ai;
pub mod cli;
pub mod dice;
pub mod engine;
pub mod model;
pub mod sim;
pub mod state;
