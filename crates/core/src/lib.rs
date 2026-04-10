//! waytorandr core library
//!
//! Provides the shared data model, profile management, matching, and planning
//! for Wayland display configuration.

pub mod engine;
pub mod error;
pub mod matcher;
pub mod model;
pub mod normalize;
pub mod planner;
pub mod profile;
pub mod state;
pub mod store;
pub mod workflow;
