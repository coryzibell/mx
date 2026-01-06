use anyhow::{Context, Result};
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

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

/// FastEmbed provider using BGE-Base-EN-v1.5
pub struct FastEmbedProvider {
    model: TextEmbedding,
    model_id: String,
    dimensions: usize,
}

impl FastEmbedProvider {
    pub fn new() -> Result<Self> {
        let model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::BGEBaseENV15).with_show_download_progress(true),
        )
        .context("Failed to initialize FastEmbed model")?;

        Ok(Self {
            model,
            model_id: "BAAI/bge-base-en-v1.5".to_string(),
            dimensions: 768,
        })
    }
}

impl EmbeddingProvider for FastEmbedProvider {
    fn embed(&mut self, text: &str) -> Result<Vec<f32>> {
        let embeddings = self
            .model
            .embed(vec![text.to_string()], None)
            .context("Failed to generate embedding")?;

        embeddings
            .into_iter()
            .next()
            .context("No embedding returned")
    }

    fn embed_batch(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.model
            .embed(texts, None)
            .context("Failed to generate batch embeddings")
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

    #[test]
    fn test_embed_single() -> Result<()> {
        let mut provider = FastEmbedProvider::new()?;
        let embedding = provider.embed("Hello, world!")?;
        assert_eq!(embedding.len(), 768);
        Ok(())
    }

    #[test]
    fn test_embed_batch() -> Result<()> {
        let mut provider = FastEmbedProvider::new()?;
        let texts = vec!["First text".to_string(), "Second text".to_string()];
        let embeddings = provider.embed_batch(&texts)?;
        assert_eq!(embeddings.len(), 2);
        assert_eq!(embeddings[0].len(), 768);
        assert_eq!(embeddings[1].len(), 768);
        Ok(())
    }

    #[test]
    fn test_dimensions() -> Result<()> {
        let provider = FastEmbedProvider::new()?;
        assert_eq!(provider.dimensions(), 768);
        Ok(())
    }
}
