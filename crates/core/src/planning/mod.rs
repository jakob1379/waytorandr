mod matcher;
mod normalize;
mod planner;

pub use matcher::{MatchResult, MatchedOutputBinding, Matcher};
pub use normalize::{
    canonicalize_profile, normalize_profile_with_known_outputs,
    normalize_topology_with_known_outputs,
};
pub use planner::{detect_preset, LayoutPlan, Planner};
