use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct OutputIdentity {
    pub edid_hash: Option<String>,
    pub make: Option<String>,
    pub model: Option<String>,
    pub serial: Option<String>,
    pub connector: Option<String>,
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_virtual: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_ignored: bool,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(v: &bool) -> bool {
    !*v
}

impl OutputIdentity {
    #[must_use]
    pub fn new(connector: impl Into<String>) -> Self {
        Self {
            connector: Some(connector.into()),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn primary_key(&self) -> String {
        if let Some(hash) = &self.edid_hash {
            return format!("edid:{hash}");
        }
        let parts: Vec<String> = [
            normalized_identity_value(self.make.as_deref()),
            normalized_identity_value(self.model.as_deref()),
            normalized_identity_value(self.serial.as_deref()),
        ]
        .into_iter()
        .flatten()
        .collect();
        if !parts.is_empty() {
            let joined = parts.join(":");
            return format!("id:{joined}");
        }
        if let Some(conn) = normalized_identity_value(self.connector.as_deref()) {
            return format!("conn:{conn}");
        }
        let description = normalized_identity_value(self.description.as_deref());
        description.unwrap_or_else(|| "unknown".to_string())
    }

    #[must_use]
    pub fn match_strength(&self) -> u8 {
        let mut strength = 0u8;
        if self.edid_hash.is_some() {
            strength += 5;
        }
        if normalized_identity_value(self.make.as_deref()).is_some() {
            strength += 2;
        }
        if normalized_identity_value(self.model.as_deref()).is_some() {
            strength += 2;
        }
        if normalized_identity_value(self.serial.as_deref()).is_some() {
            strength += 3;
        }
        if normalized_identity_value(self.connector.as_deref()).is_some() {
            strength += 1;
        }
        strength
    }

    pub(crate) fn match_score(&self) -> u32 {
        let mut score = 0u32;

        if self.edid_hash.is_some() {
            score += 100;
        }
        if normalized_identity_value(self.make.as_deref()).is_some() {
            score += 10;
        }
        if normalized_identity_value(self.model.as_deref()).is_some() {
            score += 10;
        }
        if normalized_identity_value(self.serial.as_deref()).is_some() {
            score += 20;
        }
        if normalized_identity_value(self.connector.as_deref()).is_some() {
            score += 5;
        }

        score
    }

    #[must_use]
    pub fn has_non_connector_identity(&self) -> bool {
        normalized_identity_value(self.edid_hash.as_deref()).is_some()
            || normalized_identity_value(self.make.as_deref()).is_some()
            || normalized_identity_value(self.model.as_deref()).is_some()
            || normalized_identity_value(self.serial.as_deref()).is_some()
    }

    pub fn validate_limits(&self, name: &str) -> Result<(), String> {
        for (field, value) in [
            ("edid_hash", self.edid_hash.as_deref()),
            ("make", self.make.as_deref()),
            ("model", self.model.as_deref()),
            ("serial", self.serial.as_deref()),
            ("connector", self.connector.as_deref()),
            ("description", self.description.as_deref()),
        ] {
            if let Some(value) = value {
                super::validate_string(field, value)
                    .map_err(|err| format!("output {name} has invalid {err}"))?;
            }
        }

        Ok(())
    }

    #[must_use]
    pub fn with_fallback(&self, fallback: &OutputIdentity) -> OutputIdentity {
        Self {
            edid_hash: self
                .edid_hash
                .clone()
                .or_else(|| fallback.edid_hash.clone()),
            make: choose_identity_value(self.make.as_deref(), fallback.make.as_deref()),
            model: choose_identity_value(self.model.as_deref(), fallback.model.as_deref()),
            serial: choose_identity_value(self.serial.as_deref(), fallback.serial.as_deref()),
            connector: choose_identity_value(
                self.connector.as_deref(),
                fallback.connector.as_deref(),
            ),
            description: choose_identity_value(
                self.description.as_deref(),
                fallback.description.as_deref(),
            ),
            is_virtual: self.is_virtual,
            is_ignored: self.is_ignored,
        }
    }
}

#[must_use]
pub fn identities_match(query: &OutputIdentity, candidate: &OutputIdentity) -> bool {
    if let Some(query_hash) = &query.edid_hash {
        if let Some(cand_hash) = &candidate.edid_hash {
            return query_hash == cand_hash;
        }
        return false;
    }

    let query = NormalizedIdentity::from(query);
    let candidate = NormalizedIdentity::from(candidate);

    if let (Some(query_make), Some(candidate_make)) = (&query.make, &candidate.make) {
        if query_make != candidate_make {
            return false;
        }
    }

    if let (Some(query_model), Some(candidate_model)) = (&query.model, &candidate.model) {
        if query_model != candidate_model {
            return false;
        }
    }

    if let (Some(query_serial), Some(candidate_serial)) = (&query.serial, &candidate.serial) {
        if query_serial != candidate_serial {
            return false;
        }
    }

    if query.serial.is_some() {
        return candidate.serial.is_some();
    }

    if let (Some(query_connector), Some(candidate_connector)) =
        (&query.connector, &candidate.connector)
    {
        if query_connector == candidate_connector {
            return true;
        }
    }

    if let (Some(query_description), Some(candidate_description)) =
        (&query.description, &candidate.description)
    {
        if query_description == candidate_description {
            return true;
        }
    }

    query.is_empty()
}

struct NormalizedIdentity {
    make: Option<String>,
    model: Option<String>,
    serial: Option<String>,
    connector: Option<String>,
    description: Option<String>,
}

impl NormalizedIdentity {
    fn from(identity: &OutputIdentity) -> Self {
        Self {
            make: normalized_identity_value(identity.make.as_deref()),
            model: normalized_identity_value(identity.model.as_deref()),
            serial: normalized_identity_value(identity.serial.as_deref()),
            connector: normalized_identity_value(identity.connector.as_deref()),
            description: normalized_identity_value(identity.description.as_deref()),
        }
    }

    fn is_empty(&self) -> bool {
        self.make.is_none()
            && self.model.is_none()
            && self.serial.is_none()
            && self.connector.is_none()
            && self.description.is_none()
    }
}

#[must_use]
pub fn normalized_identity_value(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }

    let lower = value.to_ascii_lowercase();
    if matches!(lower.as_str(), "unknown" | "n/a" | "none") {
        return None;
    }
    if lower.starts_with("unknown - unknown -") {
        return None;
    }

    Some(value.to_string())
}

fn choose_identity_value(primary: Option<&str>, fallback: Option<&str>) -> Option<String> {
    normalized_identity_value(primary).or_else(|| normalized_identity_value(fallback))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_matching_prefers_edid_when_present() {
        let mut query = OutputIdentity::new("DP-1");
        query.edid_hash = Some("abc".to_string());
        let mut candidate = OutputIdentity::new("DP-1");
        candidate.edid_hash = Some("abc".to_string());

        assert!(identities_match(&query, &candidate));

        candidate.edid_hash = Some("def".to_string());
        assert!(!identities_match(&query, &candidate));
    }

    #[test]
    fn fallback_ignores_unknown_placeholder_values() {
        let mut primary = OutputIdentity::new("Unknown");
        primary.make = Some("Unknown".to_string());
        let mut fallback = OutputIdentity::new("DP-1");
        fallback.make = Some("Dell".to_string());

        let merged = primary.with_fallback(&fallback);

        assert_eq!(merged.connector.as_deref(), Some("DP-1"));
        assert_eq!(merged.make.as_deref(), Some("Dell"));
    }

    #[test]
    fn match_score_ignores_unknown_placeholder_values() {
        let identity = OutputIdentity {
            make: Some("Unknown".to_string()),
            model: Some("n/a".to_string()),
            serial: Some("none".to_string()),
            connector: Some("unknown".to_string()),
            ..OutputIdentity::default()
        };

        assert_eq!(identity.match_score(), 0);
    }
}
