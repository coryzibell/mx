use anyhow::{Context, Result};
use tokenizers::Tokenizer;
use tract_onnx::prelude::*;

/// Trait for embedding providers
pub trait EmbeddingProvider: Send + Sync {
    /// Generate embedding for a single text
    fn embed(&mut self, text: &str) -> Result<Vec<f32>>;

    /// Generate embeddings for multiple texts
    fn embed_batch(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>>;

    /// Get the dimension of embeddings
    fn dimensions(&self) -> usize;

    /// Get the model identifier
    fn model_id(&self) -> &str;
}

/// Tract-ONNX provider using BGE-Base-EN-v1.5 (pure Rust, no C++ ONNX Runtime)
pub struct TractProvider {
    model: InferenceModel,
    tokenizer: Tokenizer,
    model_id: String,
    dimensions: usize,
}

impl TractProvider {
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

        // Load ONNX model (unoptimized -- optimized per-input in embed())
        let model = tract_onnx::onnx()
            .model_for_path(&model_path)
            .context("Failed to load ONNX model with tract")?;

        // Load tokenizer
        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| anyhow::anyhow!("Failed to load tokenizer: {}", e))?;

        Ok(Self {
            model,
            tokenizer,
            model_id: "BAAI/bge-base-en-v1.5".to_string(),
            dimensions: 768,
        })
    }
}

impl EmbeddingProvider for TractProvider {
    fn embed(&mut self, text: &str) -> Result<Vec<f32>> {
        // Tokenize
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
        let seq_len = input_ids.len();
        let batch = 1_usize;

        // Clone model and set input facts for this sequence length
        let mut model = self.model.clone();
        let input_fact =
            InferenceFact::dt_shape(i64::datum_type(), tvec![batch.to_dim(), seq_len.to_dim()]);

        // Set input shapes for all 3 model inputs: input_ids, attention_mask, token_type_ids
        for i in 0..3 {
            model
                .set_input_fact(i, input_fact.clone())
                .with_context(|| format!("Failed to set input fact for input {}", i))?;
        }

        let model = model.into_optimized().context("Failed to optimize model")?;
        let model = model
            .into_runnable()
            .context("Failed to make model runnable")?;

        // Build input tensors
        let input_ids_tensor =
            tract_ndarray::Array2::from_shape_vec((batch, seq_len), input_ids)?.into_tensor();
        let attention_mask_tensor =
            tract_ndarray::Array2::from_shape_vec((batch, seq_len), attention_mask)?.into_tensor();
        let token_type_ids_tensor =
            tract_ndarray::Array2::from_shape_vec((batch, seq_len), token_type_ids)?.into_tensor();

        let inputs = tvec![
            input_ids_tensor.into(),
            attention_mask_tensor.into(),
            token_type_ids_tensor.into(),
        ];

        // Run inference
        let outputs = model.run(inputs).context("Failed to run inference")?;

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

    fn embed_batch(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        // Simple loop: each text gets compiled at its actual sequence length.
        // This is correct and avoids padding complexity.
        texts.iter().map(|t| self.embed(t)).collect()
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
        let mut provider = TractProvider::new()?;
        let embedding = provider.embed("Hello, world!")?;
        assert_eq!(embedding.len(), 768);
        Ok(())
    }

    #[test]
    #[serial]
    fn test_embed_batch() -> Result<()> {
        let mut provider = TractProvider::new()?;
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
}
