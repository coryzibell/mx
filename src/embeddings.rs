use anyhow::{Context, Result};
use tokenizers::Tokenizer;
use tract_onnx::prelude::*;

/// Maximum sequence length for BGE-Base-EN-v1.5 (matches max_position_embeddings).
const MAX_SEQ_LEN: usize = 512;

/// Trait for embedding providers
pub trait EmbeddingProvider: Send + Sync {
    /// Generate embedding for a single text
    fn embed(&self, text: &str) -> Result<Vec<f32>>;

    /// Generate embeddings for multiple texts
    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;

    /// Get the dimension of embeddings
    fn dimensions(&self) -> usize;

    /// Get the model identifier
    fn model_id(&self) -> &str;
}

/// Load just the tokenizer (no ONNX model). Used for token counting
/// without the overhead of model initialization.
pub fn load_tokenizer() -> Result<Tokenizer> {
    let cache_dir = crate::paths::model_cache_dir();
    let api = hf_hub::api::sync::ApiBuilder::new()
        .with_cache_dir(cache_dir)
        .build()
        .context("Failed to initialize HF Hub API")?;
    let repo = api.model("Xenova/bge-base-en-v1.5".to_string());
    let tokenizer_path = repo
        .get("tokenizer.json")
        .context("Failed to fetch tokenizer from HF Hub")?;
    let mut tokenizer = Tokenizer::from_file(&tokenizer_path)
        .map_err(|e| anyhow::anyhow!("Failed to load tokenizer: {}", e))?;
    tokenizer
        .with_truncation(None)
        .map_err(|e| anyhow::anyhow!("Failed to configure truncation: {}", e))?;
    Ok(tokenizer)
}

/// Tract-ONNX provider using BGE-Base-EN-v1.5 (pure Rust, no C++ ONNX Runtime)
///
/// The model is optimized once at construction time with a fixed input shape
/// of [1, 512]. Inputs shorter than 512 tokens are zero-padded; inputs longer
/// than 512 tokens are truncated by the tokenizer. This avoids the enormous
/// cost of cloning + re-optimizing the model graph on every embed() call.
pub struct TractProvider {
    plan: TypedRunnableModel<TypedModel>,
    tokenizer: Tokenizer,
    model_id: String,
    dimensions: usize,
}

impl TractProvider {
    /// Access the underlying tokenizer (used by the chunker).
    pub fn tokenizer(&self) -> &Tokenizer {
        &self.tokenizer
    }

    pub fn new() -> Result<Self> {
        let cache_dir = crate::paths::model_cache_dir();

        // Configure hf-hub to use mx's model cache directory
        let api = hf_hub::api::sync::ApiBuilder::new()
            .with_cache_dir(cache_dir)
            .build()
            .context("Failed to initialize HF Hub API")?;

        // Xenova/bge-base-en-v1.5 is the ONNX-converted variant on HF Hub;
        // the canonical model identifier is BAAI/bge-base-en-v1.5 (reported
        // by model_id() for embedding metadata).
        let repo = api.model("Xenova/bge-base-en-v1.5".to_string());

        // Fetch model and tokenizer files (downloads on first use, cached thereafter)
        let model_path = repo
            .get("onnx/model.onnx")
            .context("Failed to fetch ONNX model from HF Hub")?;
        let tokenizer_path = repo
            .get("tokenizer.json")
            .context("Failed to fetch tokenizer from HF Hub")?;

        // Load ONNX model
        let mut model = tract_onnx::onnx()
            .model_for_path(&model_path)
            .context("Failed to load ONNX model with tract")?;

        // Load tokenizer with truncation enabled (max 512 tokens, matching model limit).
        // Without this, inputs exceeding 512 tokens cause into_optimized() to hang.
        let mut tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| anyhow::anyhow!("Failed to load tokenizer: {}", e))?;

        tokenizer
            .with_truncation(Some(tokenizers::TruncationParams::default()))
            .map_err(|e| anyhow::anyhow!("Failed to configure truncation: {}", e))?;

        // Optimize the model ONCE with fixed input shape [1, MAX_SEQ_LEN].
        // Inputs are padded to MAX_SEQ_LEN at inference time. This avoids
        // cloning + re-optimizing the graph on every embed() call (the
        // previous approach used ~10GB RAM per call).
        let input_fact = InferenceFact::dt_shape(
            i64::datum_type(),
            tvec![1_usize.to_dim(), MAX_SEQ_LEN.to_dim()],
        );
        for i in 0..3 {
            model
                .set_input_fact(i, input_fact.clone())
                .with_context(|| format!("Failed to set input fact for input {}", i))?;
        }

        let model = model.into_optimized().context("Failed to optimize model")?;
        let plan = model
            .into_runnable()
            .context("Failed to make model runnable")?;

        Ok(Self {
            plan,
            tokenizer,
            model_id: "BAAI/bge-base-en-v1.5".to_string(),
            dimensions: 768,
        })
    }
}

/// Pad a token vector to `MAX_SEQ_LEN` with the given pad value.
fn pad_to_max(tokens: &[i64], pad_value: i64) -> Vec<i64> {
    let mut padded = vec![pad_value; MAX_SEQ_LEN];
    let copy_len = tokens.len().min(MAX_SEQ_LEN);
    padded[..copy_len].copy_from_slice(&tokens[..copy_len]);
    padded
}

impl EmbeddingProvider for TractProvider {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        // Tokenize (truncation to 512 tokens is handled by the tokenizer)
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| anyhow::anyhow!("Failed to tokenize: {}", e))?;

        let input_ids: Vec<i64> = encoding.get_ids().iter().map(|&x| x as i64).collect();
        let attention_mask: Vec<i64> = encoding
            .get_attention_mask()
            .iter()
            .map(|&x| x as i64)
            .collect();
        let token_type_ids: Vec<i64> = encoding.get_type_ids().iter().map(|&x| x as i64).collect();

        // Pad to fixed MAX_SEQ_LEN (the model was optimized for this shape).
        // input_ids: pad with 0 (PAD token)
        // attention_mask: pad with 0 (ignore padded positions)
        // token_type_ids: pad with 0
        let input_ids = pad_to_max(&input_ids, 0);
        let attention_mask = pad_to_max(&attention_mask, 0);
        let token_type_ids = pad_to_max(&token_type_ids, 0);

        // Build input tensors with fixed shape [1, MAX_SEQ_LEN]
        let input_ids_tensor =
            tract_ndarray::Array2::from_shape_vec((1, MAX_SEQ_LEN), input_ids)?.into_tensor();
        let attention_mask_tensor =
            tract_ndarray::Array2::from_shape_vec((1, MAX_SEQ_LEN), attention_mask)?.into_tensor();
        let token_type_ids_tensor =
            tract_ndarray::Array2::from_shape_vec((1, MAX_SEQ_LEN), token_type_ids)?.into_tensor();

        let inputs = tvec![
            input_ids_tensor.into(),
            attention_mask_tensor.into(),
            token_type_ids_tensor.into(),
        ];

        // Run inference (no clone, no re-optimization -- just run the pre-built plan)
        let outputs = self.plan.run(inputs).context("Failed to run inference")?;

        // Output shape: [batch, seq_len, hidden_size]
        let output_tensor = outputs[0]
            .to_array_view::<f32>()
            .context("Failed to convert output to f32 array")?;

        // CLS pooling: extract the [CLS] token embedding (position 0)
        let mut cls_pooled = output_tensor.slice(tract_ndarray::s![0, 0, ..]).to_vec();

        // L2 normalize
        let l2: f32 = cls_pooled.iter().map(|x| x * x).sum::<f32>().sqrt();
        if l2 > 0.0 {
            for v in cls_pooled.iter_mut() {
                *v /= l2;
            }
        }

        Ok(cls_pooled)
    }

    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        // Simple loop: each call is now cheap (no model clone or re-optimization).
        texts.iter().map(|t| self.embed(t.as_str())).collect()
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    #[serial]
    fn test_embed_single() -> Result<()> {
        let provider = TractProvider::new()?;
        let embedding = provider.embed("Hello, world!")?;
        assert_eq!(embedding.len(), 768);
        Ok(())
    }

    #[test]
    #[serial]
    fn test_embed_batch() -> Result<()> {
        let provider = TractProvider::new()?;
        let texts = vec!["First text".to_string(), "Second text".to_string()];
        let embeddings = provider.embed_batch(&texts)?;
        assert_eq!(embeddings.len(), 2);
        assert_eq!(embeddings[0].len(), 768);
        assert_eq!(embeddings[1].len(), 768);
        Ok(())
    }

    #[test]
    #[serial]
    fn test_dimensions() -> Result<()> {
        let provider = TractProvider::new()?;
        assert_eq!(provider.dimensions(), 768);
        Ok(())
    }

    /// Regression test: long inputs (>512 tokens) must be truncated by the
    /// tokenizer and produce a valid 768-dim embedding without hanging.
    /// Before the fix, inputs exceeding max_position_embeddings (512) caused
    /// tract's into_optimized() to hang with 100% CPU and unbounded RAM.
    #[test]
    #[serial]
    fn test_embed_long_input_truncation() -> Result<()> {
        let provider = TractProvider::new()?;
        // ~2000 words, well beyond the 512-token limit
        let long_text = "the quick brown fox jumps over the lazy dog ".repeat(250);
        let embedding = provider.embed(&long_text)?;
        assert_eq!(
            embedding.len(),
            768,
            "Long input should produce 768-dim embedding after truncation"
        );
        // Verify it's a valid normalized vector (L2 norm ~= 1.0)
        let l2: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (l2 - 1.0).abs() < 1e-4,
            "Embedding should be L2-normalized, got norm {}",
            l2
        );
        Ok(())
    }
}
