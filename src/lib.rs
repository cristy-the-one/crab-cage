//! Crab Cage: evolve Rust process controllers offline and run them in lust's sandbox.

pub mod control;
pub mod emit;
pub mod error;
pub mod evolve;
pub mod fitness;
pub mod ga;
pub mod genome;
pub mod http;
pub mod llm;
pub mod lust;
pub mod models;
pub mod nugget;
pub mod plant;
pub mod report;
pub mod rng;
pub mod supervisor;

pub use error::{Error, Result};
pub use evolve::{execute, format_summary, BackendChoice, EvalMode, RunOptions, RunOutput};
