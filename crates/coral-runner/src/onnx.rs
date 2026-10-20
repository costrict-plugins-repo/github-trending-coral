//! Local ONNX Runtime embeddings provider.
//!
//! Gated behind the `local-embeddings` feature flag to avoid pulling in
//! heavy native dependencies (`ort` + `tokenizers`) for users who only
//! need cloud providers.
//!
//! ## Usage
//!
//! ```toml
//! # Cargo.toml
//! coral-runner = { path = "../coral-runner", features = ["local-embeddings"] }
//! ```
//!
//! The provider expects a local `.onnx` model file and a
//! `tokenizer.json` (HuggingFace format). Download them with:
//!
//! ```sh
//! coral embed --download-model nomic-embed-text-v1.5
//! ```
//!
//! ## Implementation
//!
//! Uses `ort` (ONNX Runtime v2 bindings) for inference and `tokenizers`
//! (HuggingFace) for text tokenization. The model session is loaded
//! lazily with interior mutability (required by ort v2's `&mut self`
//! `run`).

use std::path::PathBuf;
use std::sync::Mutex;

use crate::embeddings::{EmbedResult, EmbeddingsError, EmbeddingsProvider};

// Re-export from embeddings.rs — single source of truth for these constants.
pub use crate::embeddings::{DEFAULT_ONNX_DIM, DEFAULT_ONNX_MODEL};

/// Local ONNX Runtime embeddings provider.
///
/// Loads an ONNX model and HuggingFace tokenizer from disk to run
/// embeddings without any network calls. The model session and
/// tokenizer are loaded lazily on first `embed_batch` call.
pub struct OnnxProvider {
    /// Path to the `.onnx` model file.
    pub model_path: PathBuf,
    /// Path to the `tokenizer.json` file.
    pub tokenizer_path: PathBuf,
    /// Embedding vector dimensionality.
    dim: usize,
    /// Human-readable model name for cache keying.
    name: String,
    /// Lazily-loaded ONNX session (behind Mutex because `run` requires `&mut`).
    session: Mutex<Option<Result<ort::session::Session, String>>>,
    /// Lazily-loaded tokenizer.
    tokenizer: Mutex<Option<Result<tokenizers::Tokenizer, String>>>,
}

// SAFETY: ort::Session is Send (documented); we protect concurrent access
// with a Mutex. tokenizers::Tokenizer is also Send + Sync for read-only use.
unsafe impl Sync for OnnxProvider {}

impl OnnxProvider {
    /// Create a new ONNX provider.
    ///
    /// Does NOT load the model or tokenizer immediately — they are
    /// loaded lazily on the first `embed_batch` call.
    pub fn new(
        model_path: impl Into<PathBuf>,
        tokenizer_path: impl Into<PathBuf>,
        dim: usize,
        name: impl Into<String>,
    ) -> Self {
        Self {
            model_path: model_path.into(),
            tokenizer_path: tokenizer_path.into(),
            dim,
            name: name.into(),
            session: Mutex::new(None),
            tokenizer: Mutex::new(None),
        }
    }

    /// Convenience constructor for the default nomic-embed-text-v1.5 model.
    ///
    /// Expects the model and tokenizer at:
    /// - `models_dir/model.onnx`
    /// - `models_dir/tokenizer.json`
    pub fn nomic_v1_5(models_dir: impl Into<PathBuf>) -> Self {
        let dir = models_dir.into();
        Self::new(
            dir.join("model.onnx"),
            dir.join("tokenizer.json"),
            DEFAULT_ONNX_DIM,
            DEFAULT_ONNX_MODEL,
        )
    }

    /// Ensure the session is loaded, returning an error string if not.
    fn ensure_session_loaded(&self) -> Result<(), EmbeddingsError> {
        let mut guard = self.session.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            if !self.model_path.exists() {
                *guard = Some(Err(format!(
                    "ONNX model not found at {}. Download with: coral embed --download-model {}",
                    self.model_path.display(),
                    DEFAULT_ONNX_MODEL
                )));
            } else {
                let result = ort::session::Session::builder()
                    .and_then(|mut b| b.commit_from_file(&self.model_path));
                *guard = Some(result.map_err(|e| format!("failed to load ONNX model: {e}")));
            }
        }
        match guard.as_ref().unwrap() {
            Ok(_) => Ok(()),
            Err(e) => Err(EmbeddingsError::Parse(e.clone())),
        }
    }

    /// Ensure the tokenizer is loaded, returning an error if not.
    fn ensure_tokenizer_loaded(&self) -> Result<(), EmbeddingsError> {
        let mut guard = self.tokenizer.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            if !self.tokenizer_path.exists() {
                *guard = Some(Err(format!(
                    "Tokenizer not found at {}. Download with: coral embed --download-model {}",
                    self.tokenizer_path.display(),
                    DEFAULT_ONNX_MODEL
                )));
            } else {
                let result = tokenizers::Tokenizer::from_file(&self.tokenizer_path);
                *guard = Some(result.map_err(|e| format!("failed to load tokenizer: {e}")));
            }
        }
        match guard.as_ref().unwrap() {
            Ok(_) => Ok(()),
            Err(e) => Err(EmbeddingsError::Parse(e.clone())),
        }
    }
}

impl EmbeddingsProvider for OnnxProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn dim(&self) -> usize {
        self.dim
    }

    fn embed_batch(
        &self,
        texts: &[String],
        _input_type: Option<&str>,
    ) -> EmbedResult<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }

        // Ensure both are loaded (error early if files missing)
        self.ensure_session_loaded()?;
        self.ensure_tokenizer_loaded()?;

        // Tokenize under the tokenizer lock
        let encodings = {
            let guard = self.tokenizer.lock().unwrap_or_else(|e| e.into_inner());
            let tokenizer = guard.as_ref().unwrap().as_ref().unwrap();
            tokenizer
                .encode_batch(texts.to_vec(), true)
                .map_err(|e| EmbeddingsError::Parse(format!("tokenization failed: {e}")))?
        };

        let batch_size = encodings.len();
        let max_len = encodings.iter().map(|e| e.get_ids().len()).max().unwrap_or(0);

        // Prepare flat input tensors: input_ids and attention_mask
        let mut input_ids: Vec<i64> = vec![0i64; batch_size * max_len];
        let mut attention_mask: Vec<i64> = vec![0i64; batch_size * max_len];

        for (i, encoding) in encodings.iter().enumerate() {
            for (j, &id) in encoding.get_ids().iter().enumerate() {
                input_ids[i * max_len + j] = id as i64;
            }
            for (j, &mask) in encoding.get_attention_mask().iter().enumerate() {
                attention_mask[i * max_len + j] = mask as i64;
            }
        }

        // Run inference under the session lock
        let mut session_guard = self.session.lock().unwrap_or_else(|e| e.into_inner());
        let session = session_guard
            .as_mut()
            .unwrap()
            .as_mut()
            .map_err(|e| EmbeddingsError::Parse(e.clone()))?;

        let input_ids_tensor =
            ort::value::Tensor::from_array(([batch_size, max_len], input_ids))
                .map_err(|e| EmbeddingsError::Parse(format!("input_ids tensor: {e}")))?;
        let attn_mask_tensor =
            ort::value::Tensor::from_array(([batch_size, max_len], attention_mask))
                .map_err(|e| EmbeddingsError::Parse(format!("attention_mask tensor: {e}")))?;

        let outputs = session
            .run(ort::inputs![input_ids_tensor, attn_mask_tensor])
            .map_err(|e| EmbeddingsError::Parse(format!("ONNX inference failed: {e}")))?;

        // Extract output tensor. Try named outputs, fall back to index 0.
        let output_value = outputs
            .get("last_hidden_state")
            .or_else(|| outputs.get("sentence_embedding"))
            .or_else(|| Some(&outputs[0usize]))
            .ok_or_else(|| EmbeddingsError::Parse("no output from ONNX model".into()))?;

        let (shape, data) = output_value
            .try_extract_tensor::<f32>()
            .map_err(|e| EmbeddingsError::Parse(format!("extract tensor failed: {e}")))?;

        // Shape deref gives us &[i64]
        let dims: Vec<usize> = shape.iter().map(|&d| d as usize).collect();
        let mut results = Vec::with_capacity(batch_size);

        if dims.len() == 3 {
            // (batch_size, seq_len, dim) — mean-pool over seq_len with attention mask
            let seq_len = dims[1];
            let dim = dims[2];
            for i in 0..batch_size {
                let mut embedding = vec![0f32; dim];
                let mask_sum: f32 = encodings[i]
                    .get_attention_mask()
                    .iter()
                    .map(|&m| m as f32)
                    .sum();
                if mask_sum > 0.0 {
                    for j in 0..seq_len.min(encodings[i].get_attention_mask().len()) {
                        let mask_val = encodings[i].get_attention_mask()[j] as f32;
                        if mask_val > 0.0 {
                            let offset = (i * seq_len + j) * dim;
                            for k in 0..dim {
                                embedding[k] += data[offset + k];
                            }
                        }
                    }
                    for val in &mut embedding {
                        *val /= mask_sum;
                    }
                }
                results.push(embedding);
            }
        } else if dims.len() == 2 {
            // (batch_size, dim) — already pooled
            let dim = dims[1];
            for i in 0..batch_size {
                let offset = i * dim;
                results.push(data[offset..offset + dim].to_vec());
            }
        } else {
            return Err(EmbeddingsError::Parse(format!(
                "unexpected output tensor shape: {dims:?}"
            )));
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn onnx_provider_advertises_name_and_dim() {
        let p = OnnxProvider::new(
            "/tmp/nonexistent/model.onnx",
            "/tmp/nonexistent/tokenizer.json",
            768,
            "nomic-embed-text-v1.5",
        );
        assert_eq!(p.name(), "nomic-embed-text-v1.5");
        assert_eq!(p.dim(), 768);
    }

    #[test]
    fn onnx_provider_empty_input_returns_empty() {
        let p = OnnxProvider::new(
            "/tmp/nonexistent/model.onnx",
            "/tmp/nonexistent/tokenizer.json",
            768,
            "test-model",
        );
        let result = p.embed_batch(&[], None).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn onnx_provider_missing_model_gives_actionable_error() {
        let p = OnnxProvider::new(
            "/tmp/nonexistent-coral-test/model.onnx",
            "/tmp/nonexistent-coral-test/tokenizer.json",
            768,
            "test-model",
        );
        let err = p
            .embed_batch(&["hello".to_string()], None)
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("not found") || msg.contains("Download"),
            "error should be actionable: {msg}"
        );
    }

    #[test]
    fn nomic_v1_5_constructor_sets_defaults() {
        let p = OnnxProvider::nomic_v1_5("/tmp/models");
        assert_eq!(p.name(), DEFAULT_ONNX_MODEL);
        assert_eq!(p.dim(), DEFAULT_ONNX_DIM);
        assert_eq!(p.model_path, PathBuf::from("/tmp/models/model.onnx"));
        assert_eq!(
            p.tokenizer_path,
            PathBuf::from("/tmp/models/tokenizer.json")
        );
    }

    #[test]
    fn onnx_constants_are_correct() {
        assert_eq!(DEFAULT_ONNX_MODEL, "nomic-embed-text-v1.5");
        assert_eq!(DEFAULT_ONNX_DIM, 768);
    }
}
