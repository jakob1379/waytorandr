//! waytorandr core library
//!
//! Provides the shared data model, profile management, matching, and planning
//! for Wayland display configuration.

pub mod engine;
pub mod matcher;
pub mod model;
pub mod planner;
pub mod profile;
pub mod store;

pub use engine::*;
pub use matcher::*;
pub use model::*;
pub use planner::*;
pub use profile::*;
pub use store::*;
