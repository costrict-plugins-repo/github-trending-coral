//! Search engine configuration.
//!
//! Defines the `[search]` section of `.coral/config.toml`:
//! - `[search.embeddings]` — provider selection (none/voyage/openai)
//! - `[search.hybrid]` — per-source weights for weighted RRF fusion

use serde::{Deserialize, Serialize};

/// Top-level search configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchConfig {
    pub embeddings: EmbeddingsConfig,
    pub hybrid: HybridConfig,
}

/// Embeddings provider selection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EmbeddingsConfig {
    /// Provider name: `"none"` (default, uses TF-IDF fallback),
    /// `"voyage"`, or `"openai"`.
    pub provider: String,
    /// Model name (provider-specific). Defaults to the provider's default model.
    pub model: String,
    /// Vector dimensions. Defaults to the provider's default.
    pub dimensions: usize,
}

/// Hybrid search fusion weights.
///
/// Each weight scales the RRF contribution of the corresponding result set.
/// Higher weight = more influence on final ranking.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HybridConfig {
    /// Weight for semantic (embedding/TF-IDF) results.
    pub semantic_weight: f64,
    /// Weight for structural (graph-based) results.
    pub structural_weight: f64,
    /// Weight for keyword (BM25) results.
    pub keyword_weight: f64,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            embeddings: EmbeddingsConfig::default(),
            hybrid: HybridConfig::default(),
        }
    }
}

impl Default for EmbeddingsConfig {
    fn default() -> Self {
        Self {
            provider: "none".to_string(),
            model: String::new(),
            dimensions: 0,
        }
    }
}

impl Default for HybridConfig {
    fn default() -> Self {
        Self {
            semantic_weight: 1.0,
            structural_weight: 0.8,
            keyword_weight: 0.6,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_config_defaults() {
        let config = SearchConfig::default();
        assert_eq!(config.embeddings.provider, "none");
        assert_eq!(config.embeddings.model, "");
        assert_eq!(config.embeddings.dimensions, 0);
        assert_eq!(config.hybrid.semantic_weight, 1.0);
        assert_eq!(config.hybrid.structural_weight, 0.8);
        assert_eq!(config.hybrid.keyword_weight, 0.6);
    }

    #[test]
    fn test_search_config_deserialize_partial() {
        let toml_str = r#"
[embeddings]
provider = "voyage"
model = "voyage-3"
dimensions = 1024

[hybrid]
semantic_weight = 1.5
"#;
        let config: SearchConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.embeddings.provider, "voyage");
        assert_eq!(config.embeddings.model, "voyage-3");
        assert_eq!(config.embeddings.dimensions, 1024);
        assert_eq!(config.hybrid.semantic_weight, 1.5);
        // Defaults for unspecified fields
        assert_eq!(config.hybrid.structural_weight, 0.8);
        assert_eq!(config.hybrid.keyword_weight, 0.6);
    }

    #[test]
    fn test_search_config_serialize_roundtrip() {
        let config = SearchConfig {
            embeddings: EmbeddingsConfig {
                provider: "openai".to_string(),
                model: "text-embedding-3-small".to_string(),
                dimensions: 1536,
            },
            hybrid: HybridConfig {
                semantic_weight: 1.2,
                structural_weight: 0.9,
                keyword_weight: 0.5,
            },
        };
        let toml_str = toml::to_string(&config).unwrap();
        let deserialized: SearchConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(deserialized.embeddings.provider, "openai");
        assert_eq!(deserialized.hybrid.keyword_weight, 0.5);
    }
}
