use waytorandr_core::{BackendKind, LayoutPlan, Topology, ValidationResult};

#[derive(Clone, Copy)]
pub(in crate::commands) enum ActionTargetType {
    Profile,
    Virtual,
}

impl ActionTargetType {
    pub(in crate::commands) const fn as_json(self) -> &'static str {
        match self {
            Self::Profile => "profile",
            Self::Virtual => "virtual",
        }
    }

    pub(in crate::commands) const fn as_human(self) -> &'static str {
        match self {
            Self::Profile => "profile",
            Self::Virtual => "virtual configuration",
        }
    }
}

pub(in crate::commands) struct ActionOutcome {
    pub(super) target: String,
    pub(super) target_type: ActionTargetType,
    pub(super) dry_run: bool,
    pub(super) plan: LayoutPlan,
    pub(super) validation: Option<ValidationResult>,
    pub(super) default_assignment: Option<DefaultAssignment>,
    pub(super) saved_profile: Option<String>,
    pub(super) backend_kind: Option<BackendKind>,
    pub(super) applied_topology: Option<Topology>,
}

#[derive(Clone, Copy)]
pub(in crate::commands) enum DefaultScope {
    Setup,
}

impl DefaultScope {
    pub(in crate::commands) const fn as_json(self) -> &'static str {
        match self {
            Self::Setup => "setup",
        }
    }

    fn description(self, target: &str) -> String {
        match self {
            Self::Setup => format!("'{target}' as the default profile for this setup"),
        }
    }
}

pub(in crate::commands) struct DefaultAssignment {
    pub(super) target: String,
    pub(super) scope: DefaultScope,
}

pub(in crate::commands) struct SaveOutcome {
    pub(super) profile: String,
    pub(super) setup_name: Option<String>,
    pub(super) dry_run: bool,
    pub(super) saved: bool,
    pub(super) plan: Option<LayoutPlan>,
    pub(super) default_scope: Option<DefaultScope>,
}

pub(in crate::commands) struct RemoveOutcome {
    pub(super) profile: String,
    pub(super) dry_run: bool,
    pub(super) removed: Option<bool>,
    pub(super) would_remove: Option<bool>,
}

impl ActionOutcome {
    pub(in crate::commands) fn record_saved_profile(&mut self, profile_name: impl Into<String>) {
        self.saved_profile = Some(profile_name.into());
    }

    pub(in crate::commands) const fn backend_kind(&self) -> Option<BackendKind> {
        self.backend_kind
    }

    pub(in crate::commands) fn applied_topology(&self) -> Option<&Topology> {
        self.applied_topology.as_ref()
    }

    pub(in crate::commands) const fn is_dry_run(&self) -> bool {
        self.dry_run
    }

    pub(in crate::commands) fn set_default_assignment(
        &mut self,
        target: impl Into<String>,
        scope: DefaultScope,
    ) {
        self.default_assignment = Some(DefaultAssignment::new(target, scope));
    }
}

impl DefaultAssignment {
    pub(in crate::commands) fn new(target: impl Into<String>, scope: DefaultScope) -> Self {
        Self {
            target: target.into(),
            scope,
        }
    }

    pub(in crate::commands) fn description(&self) -> String {
        self.scope.description(&self.target)
    }
}

impl SaveOutcome {
    pub(in crate::commands) fn dry_run(
        profile: impl Into<String>,
        setup_name: Option<&str>,
        plan: LayoutPlan,
        default_scope: Option<DefaultScope>,
    ) -> Self {
        Self {
            profile: profile.into(),
            setup_name: setup_name.map(str::to_string),
            dry_run: true,
            saved: false,
            plan: Some(plan),
            default_scope,
        }
    }

    pub(in crate::commands) fn saved(
        profile: impl Into<String>,
        setup_name: Option<&str>,
        default_scope: Option<DefaultScope>,
    ) -> Self {
        Self {
            profile: profile.into(),
            setup_name: setup_name.map(str::to_string),
            dry_run: false,
            saved: true,
            plan: None,
            default_scope,
        }
    }
}

impl RemoveOutcome {
    pub(in crate::commands) fn dry_run(profile: impl Into<String>, would_remove: bool) -> Self {
        Self {
            profile: profile.into(),
            dry_run: true,
            removed: None,
            would_remove: Some(would_remove),
        }
    }

    pub(in crate::commands) fn removed(profile: impl Into<String>, removed: bool) -> Self {
        Self {
            profile: profile.into(),
            dry_run: false,
            removed: Some(removed),
            would_remove: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use waytorandr_core::{BackendKind, LayoutPlan, Topology};

    use super::{
        ActionOutcome, ActionTargetType, DefaultAssignment, DefaultScope, RemoveOutcome,
        SaveOutcome,
    };

    #[test]
    fn action_target_type_labels_are_stable() {
        assert_eq!(ActionTargetType::Profile.as_json(), "profile");
        assert_eq!(ActionTargetType::Profile.as_human(), "profile");
        assert_eq!(ActionTargetType::Virtual.as_json(), "virtual");
        assert_eq!(
            ActionTargetType::Virtual.as_human(),
            "virtual configuration"
        );
    }

    #[test]
    fn action_outcome_accessors_expose_runtime_state() {
        let topology = Topology::default();
        let mut outcome = ActionOutcome {
            target: "docked".to_string(),
            target_type: ActionTargetType::Profile,
            dry_run: false,
            plan: LayoutPlan::new(HashMap::new()),
            validation: None,
            default_assignment: None,
            saved_profile: None,
            backend_kind: Some(BackendKind::Wlroots),
            applied_topology: Some(topology),
        };

        outcome.record_saved_profile("docked");
        outcome.set_default_assignment("docked", DefaultScope::Setup);

        assert_eq!(outcome.backend_kind(), Some(BackendKind::Wlroots));
        assert!(outcome.applied_topology().is_some());
        assert!(!outcome.is_dry_run());
        assert_eq!(outcome.saved_profile.as_deref(), Some("docked"));
        assert_eq!(
            outcome
                .default_assignment
                .as_ref()
                .map(DefaultAssignment::description)
                .as_deref(),
            Some("'docked' as the default profile for this setup")
        );
    }

    #[test]
    fn save_and_remove_outcomes_encode_dry_run_and_commit_shapes() {
        let save_preview = SaveOutcome::dry_run(
            "desk",
            Some("office"),
            LayoutPlan::new(HashMap::new()),
            None,
        );
        assert!(save_preview.dry_run);
        assert!(!save_preview.saved);
        assert!(save_preview.plan.is_some());

        let saved = SaveOutcome::saved("desk", None, Some(DefaultScope::Setup));
        assert!(!saved.dry_run);
        assert!(saved.saved);
        assert!(saved.plan.is_none());
        assert_eq!(
            saved.default_scope.map(DefaultScope::as_json),
            Some("setup")
        );

        let remove_preview = RemoveOutcome::dry_run("desk", true);
        assert!(remove_preview.dry_run);
        assert_eq!(remove_preview.would_remove, Some(true));
        assert_eq!(remove_preview.removed, None);

        let removed = RemoveOutcome::removed("desk", false);
        assert!(!removed.dry_run);
        assert_eq!(removed.removed, Some(false));
        assert_eq!(removed.would_remove, None);
    }
}
