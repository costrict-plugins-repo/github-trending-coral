//! `[implement]` configuration section.
//!
//! Defines the `ImplementConfig` struct that maps to the `[implement]`
//! section in `.coral/config.toml`. The struct is loaded by CLI commands
//! (not this crate's responsibility) and threaded into the orchestrate
//! loop and validation functions.

use serde::{Deserialize, Serialize};

fn default_false() -> bool {
    false
}

fn default_max_retries() -> u32 {
    3
}

fn default_pattern_min() -> usize {
    3
}

/// Configuration for the `coral implement` subsystem.
///
/// Loaded from `.coral/config.toml` `[implement]` section:
///
/// ```toml
/// [implement]
/// tdd = true
/// max_retries = 5
/// auto_test = true
/// pattern_min_instances = 2
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImplementConfig {
    /// Enable test-driven development mode: generate tests first,
    /// then iterate until tests pass.
    #[serde(default = "default_false")]
    pub tdd: bool,

    /// Maximum number of LLM retries when tests fail in TDD mode.
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,

    /// Automatically run tests after code generation (even outside
    /// TDD mode). When false, validation only checks compilation.
    #[serde(default = "default_false")]
    pub auto_test: bool,

    /// Minimum number of instances required before a code pattern
    /// is considered stable enough to suggest during plan generation.
    #[serde(default = "default_pattern_min")]
    pub pattern_min_instances: usize,
}

impl Default for ImplementConfig {
    fn default() -> Self {
        Self {
            tdd: false,
            max_retries: 3,
            auto_test: false,
            pattern_min_instances: 3,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values_are_sensible() {
        let cfg = ImplementConfig::default();
        assert!(!cfg.tdd);
        assert_eq!(cfg.max_retries, 3);
        assert!(!cfg.auto_test);
        assert_eq!(cfg.pattern_min_instances, 3);
    }

    #[test]
    fn deserialize_empty_toml_uses_defaults() {
        let toml_str = "";
        let cfg: ImplementConfig = toml::from_str(toml_str).unwrap();
        assert!(!cfg.tdd);
        assert_eq!(cfg.max_retries, 3);
    }

    #[test]
    fn deserialize_partial_toml_fills_defaults() {
        let toml_str = r#"
tdd = true
max_retries = 5
"#;
        let cfg: ImplementConfig = toml::from_str(toml_str).unwrap();
        assert!(cfg.tdd);
        assert_eq!(cfg.max_retries, 5);
        assert!(!cfg.auto_test);
        assert_eq!(cfg.pattern_min_instances, 3);
    }

    #[test]
    fn deserialize_full_toml() {
        let toml_str = r#"
tdd = true
max_retries = 10
auto_test = true
pattern_min_instances = 2
"#;
        let cfg: ImplementConfig = toml::from_str(toml_str).unwrap();
        assert!(cfg.tdd);
        assert_eq!(cfg.max_retries, 10);
        assert!(cfg.auto_test);
        assert_eq!(cfg.pattern_min_instances, 2);
    }

    #[test]
    fn serialize_roundtrip() {
        let cfg = ImplementConfig {
            tdd: true,
            max_retries: 7,
            auto_test: true,
            pattern_min_instances: 5,
        };
        let serialized = toml::to_string(&cfg).unwrap();
        let deserialized: ImplementConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.tdd, cfg.tdd);
        assert_eq!(deserialized.max_retries, cfg.max_retries);
        assert_eq!(deserialized.auto_test, cfg.auto_test);
        assert_eq!(deserialized.pattern_min_instances, cfg.pattern_min_instances);
    }
}
