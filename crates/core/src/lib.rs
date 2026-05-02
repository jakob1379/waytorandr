//! waytorandr core library
//!
//! Provides the shared display model, profile/store/state persistence,
//! backend engine contract, planning, and workflow orchestration for
//! Wayland display configuration.

// Cross-target dependency trees currently pull windows-sys through independent
// upstream paths that cannot be unified from this crate.
#![allow(clippy::multiple_crate_versions)]

#[cfg(test)]
mod test_support;

mod engine;
mod error;
mod model;
mod persistence;
mod planning;
mod profile;
mod state;
mod store;
pub mod workflow;

pub use engine::{
    ApplyResult, ApplyStatus, Backend, ConfigFailureKind, HookResult, OutputWatcher,
    PollingOutputWatcher, ValidationResult, ValidationStatus,
};
pub use error::{
    BackendConnectionAttempt, BackendConnectionError, CoreError, CoreResult, PlanError,
};
pub use model::{
    identities_match, normalized_identity_value, BackendKind, Capabilities, Mode, OutputIdentity,
    OutputState, Position, Topology, Transform, VirtualPreset,
};
pub use planning::{
    canonicalize_profile, detect_preset, normalize_profile_with_known_outputs,
    normalize_topology_with_known_outputs, LayoutPlan, MatchResult, MatchedOutputBinding, Matcher,
    Planner,
};
pub use profile::{validate_profile_name, Hook, Hooks, OutputConfig, OutputMatcher, Profile};
pub use state::{ReadOnlyStateStore, State, StateReader, StateStore};
pub use store::{
    ProfileQueryContext, ProfileStore, ProfilesSettings, ReadOnlyProfileStore, StoredProfile,
};
