//! Auto-derived **flat tags** are a build's only categorical store
//! (`docs/data-storage.md` §"Tags are the canonical descriptor"). Generation and
//! reversal share **one vocabulary** here so they round-trip losslessly: a build's
//! platform/config/status are known vocabularies; the leftover tag is the target.

use serde::{Deserialize, Serialize};

/// Known platform tags (the `Platform` enum's `uat()` forms).
const PLATFORMS: &[&str] = &["Win64", "Linux", "Mac"];
/// Known client-config tags.
const CONFIGS: &[&str] = &["Debug", "DebugGame", "Development", "Test", "Shipping"];
/// Known terminal statuses.
const STATUSES: &[&str] = &["Success", "Failed", "Cancelled"];

/// Generate a build's tag list: platform, one tag per config, target (if any),
/// status. A multi-config build carries a config tag each (e.g. both `Development`
/// and `Shipping`), so it surfaces under either config filter.
pub fn generate(platform: &str, configs: &[String], target: &str, status: &str) -> Vec<String> {
    let mut tags = vec![platform.to_string()];
    tags.extend(configs.iter().cloned());
    if !target.trim().is_empty() {
        tags.push(target.to_string());
    }
    tags.push(status.to_string());
    tags
}

/// A build's categorical identity recovered from its tags.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Reversed {
    pub platform: Option<String>,
    /// Every config tag (a build may be staged for several).
    pub configs: Vec<String>,
    pub status: Option<String>,
    /// The leftover tag (not a known platform/config/status).
    pub target: Option<String>,
}

/// Reverse a tag list back into its dimensions (the inverse of [`generate`]).
pub fn reverse(tags: &[String]) -> Reversed {
    let mut r = Reversed::default();
    for t in tags {
        let s = t.as_str();
        if PLATFORMS.contains(&s) {
            r.platform = Some(t.clone());
        } else if CONFIGS.contains(&s) {
            r.configs.push(t.clone());
        } else if STATUSES.contains(&s) {
            r.status = Some(t.clone());
        } else {
            r.target = Some(t.clone());
        }
    }
    r
}

/// Which tag dimension a value belongs to - lets the UI populate filter menus and
/// pick the status tag for the badge without re-encoding the vocabularies.
pub fn dimension_of(tag: &str) -> &'static str {
    if PLATFORMS.contains(&tag) {
        "platform"
    } else if CONFIGS.contains(&tag) {
        "config"
    } else if STATUSES.contains(&tag) {
        "status"
    } else {
        "target"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_then_reverse_round_trips() {
        let tags = generate("Win64", &["Development".into()], "SampleProjectSteam", "Success");
        assert_eq!(tags, vec!["Win64", "Development", "SampleProjectSteam", "Success"]);
        let r = reverse(&tags);
        assert_eq!(r.platform.as_deref(), Some("Win64"));
        assert_eq!(r.configs, vec!["Development".to_string()]);
        assert_eq!(r.target.as_deref(), Some("SampleProjectSteam"));
        assert_eq!(r.status.as_deref(), Some("Success"));
    }

    #[test]
    fn multi_config_emits_a_tag_each_and_reverses_to_all() {
        let tags = generate(
            "Win64",
            &["Development".into(), "Shipping".into()],
            "SampleProjectSteam",
            "Success",
        );
        assert_eq!(
            tags,
            vec!["Win64", "Development", "Shipping", "SampleProjectSteam", "Success"]
        );
        let r = reverse(&tags);
        assert_eq!(r.configs, vec!["Development".to_string(), "Shipping".to_string()]);
        assert_eq!(r.target.as_deref(), Some("SampleProjectSteam"));
    }

    #[test]
    fn target_is_the_leftover_even_in_any_order() {
        // order independence: the unknown value is always the target
        let r = reverse(&["Failed".into(), "MyClient".into(), "Linux".into(), "Shipping".into()]);
        assert_eq!(r.platform.as_deref(), Some("Linux"));
        assert_eq!(r.configs, vec!["Shipping".to_string()]);
        assert_eq!(r.status.as_deref(), Some("Failed"));
        assert_eq!(r.target.as_deref(), Some("MyClient"));
    }

    #[test]
    fn missing_target_reverses_to_none() {
        let tags = generate("Win64", &["Shipping".into()], "", "Cancelled");
        assert_eq!(tags, vec!["Win64", "Shipping", "Cancelled"]);
        assert_eq!(reverse(&tags).target, None);
    }

    #[test]
    fn dimension_classification() {
        assert_eq!(dimension_of("Win64"), "platform");
        assert_eq!(dimension_of("Development"), "config");
        assert_eq!(dimension_of("Success"), "status");
        assert_eq!(dimension_of("SampleProjectSteam"), "target");
    }
}
