//! waytorandr core library
//!
//! Provides the shared data model, profile management, matching, and planning
//! for Wayland display configuration.

pub mod model;
pub mod profile;
pub mod matcher;
pub mod planner;
pub mod store;
pub mod engine;

pub use model::*;
pub use profile::*;
pub use matcher::*;
pub use planner::*;
pub use store::*;
pub use engine::*;
