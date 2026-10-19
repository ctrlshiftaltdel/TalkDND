//! Headless D&D 5e (2024 rules) combat engine.
//!
//! The crate is split into small modules so that the generic rules engine, the
//! persisted encounter state and the command-line interface can be tested
//! independently. Campaign-specific content lives in data files; the engine
//! only understands generic rules and the content vocabulary.

pub mod ai;
pub mod cli;
pub mod content;
pub mod dice;
pub mod engine;
pub mod sim;
pub mod space;
pub mod state;
