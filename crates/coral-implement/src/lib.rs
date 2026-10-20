//! `coral-implement` — Code implementation engine.
//!
//! Provides:
//! - **Pattern detection**: analyze a codebase to find recurring structural patterns
//! - **Plan generation**: structured change plans grounded in existing code
//! - **Impact analysis**: predict what breaks when files change
//! - **Sketch generation**: lightweight diff previews for small changes

pub mod config;
pub mod execute;
pub mod impact;
pub mod orchestrate;
pub mod patterns;
pub mod plan;
pub mod plans;
pub mod sketch;
pub mod staging;
pub mod validate;
