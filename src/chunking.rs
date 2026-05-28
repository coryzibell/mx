use tokenizers::Tokenizer;

/// Configuration for token-aware text chunking.
pub struct ChunkConfig {
    /// Maximum tokens per chunk.
    pub max_tokens: usize,
    /// Number of overlapping tokens between consecutive chunks.
    pub overlap_tokens: usize,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            max_tokens: 400,
            overlap_tokens: 100,
        }
    }
}

/// A single chunk of text with its token-level metadata.
pub struct TextChunk {
    /// The decoded text for this chunk.
    pub text: String,
    /// Token offset from the start of the original text.
    pub token_offset: usize,
    /// Number of tokens in this chunk.
    pub token_count: usize,
    /// Zero-based chunk index.
    pub chunk_index: usize,
}

/// Split text into overlapping token-aware chunks.
///
/// If the text fits within `config.max_tokens`, returns a single chunk
/// containing the full text. Otherwise, produces a sliding window of
/// chunks with stride = `max_tokens - overlap_tokens`.
///
/// Uses `tokenizer.encode(text, false)` (no special tokens) so that
/// the embedding provider can add its own [CLS]/[SEP] tokens during
/// inference.
pub fn chunk_text(text: &str, tokenizer: &Tokenizer, config: &ChunkConfig) -> Vec<TextChunk> {
    let encoding = tokenizer
        .encode(text, false)
        .expect("tokenizer.encode should not fail on valid text");

    let all_ids = encoding.get_ids();
    let total_tokens = all_ids.len();

    // Base case: fits in a single chunk.
    if total_tokens <= config.max_tokens {
        return vec![TextChunk {
            text: text.to_string(),
            token_offset: 0,
            token_count: total_tokens,
            chunk_index: 0,
        }];
    }

    let stride = config.max_tokens.saturating_sub(config.overlap_tokens);
    // Guard against zero stride (would cause infinite loop).
    let stride = if stride == 0 { 1 } else { stride };

    let mut chunks = Vec::new();
    let mut offset = 0usize;
    let mut chunk_index = 0usize;

    while offset < total_tokens {
        let end = (offset + config.max_tokens).min(total_tokens);
        let chunk_ids: Vec<u32> = all_ids[offset..end].to_vec();
        let chunk_token_count = chunk_ids.len();

        let decoded = tokenizer
            .decode(&chunk_ids, true)
            .expect("tokenizer.decode should not fail on valid IDs");

        chunks.push(TextChunk {
            text: decoded,
            token_offset: offset,
            token_count: chunk_token_count,
            chunk_index,
        });

        offset += stride;
        chunk_index += 1;
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a tokenizer for testing (same model as TractProvider).
    fn test_tokenizer() -> Tokenizer {
        let cache_dir = crate::paths::model_cache_dir();
        let api = hf_hub::api::sync::ApiBuilder::new()
            .with_cache_dir(cache_dir)
            .build()
            .expect("HF Hub API");
        let repo = api.model("Xenova/bge-base-en-v1.5".to_string());
        let tokenizer_path = repo.get("tokenizer.json").expect("tokenizer.json");
        Tokenizer::from_file(&tokenizer_path).expect("load tokenizer")
    }

    #[test]
    fn test_short_text_single_chunk() {
        let tokenizer = test_tokenizer();
        let config = ChunkConfig::default();
        let chunks = chunk_text("Hello, world!", &tokenizer, &config);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_index, 0);
        assert_eq!(chunks[0].token_offset, 0);
        assert_eq!(chunks[0].text, "Hello, world!");
    }

    #[test]
    fn test_long_text_multiple_chunks() {
        let tokenizer = test_tokenizer();
        let config = ChunkConfig::default();
        // ~2000 tokens, well over the 400 limit
        let long_text = "the quick brown fox jumps over the lazy dog ".repeat(250);
        let chunks = chunk_text(&long_text, &tokenizer, &config);

        assert!(
            chunks.len() > 1,
            "Expected multiple chunks, got {}",
            chunks.len()
        );

        // Verify chunk indices are sequential
        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.chunk_index, i);
        }

        // Verify first chunk starts at offset 0
        assert_eq!(chunks[0].token_offset, 0);

        // Verify each chunk respects max_tokens
        for chunk in &chunks {
            assert!(
                chunk.token_count <= config.max_tokens,
                "Chunk {} has {} tokens, exceeding max {}",
                chunk.chunk_index,
                chunk.token_count,
                config.max_tokens
            );
        }

        // Verify overlap: each subsequent chunk's offset should be
        // previous chunk's offset + stride (300)
        let stride = config.max_tokens - config.overlap_tokens;
        for (i, chunk) in chunks.iter().enumerate().skip(1) {
            let expected_offset = i * stride;
            assert_eq!(
                chunk.token_offset, expected_offset,
                "Chunk {} offset {} != expected {}",
                i, chunk.token_offset, expected_offset
            );
        }
    }

    #[test]
    fn test_empty_text_single_chunk() {
        let tokenizer = test_tokenizer();
        let config = ChunkConfig::default();
        let chunks = chunk_text("", &tokenizer, &config);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].token_count, 0);
        assert_eq!(chunks[0].chunk_index, 0);
    }

    #[test]
    fn test_exact_boundary() {
        let tokenizer = test_tokenizer();
        let config = ChunkConfig {
            max_tokens: 10,
            overlap_tokens: 3,
        };
        // Exactly 10 tokens should produce a single chunk
        // "one two three four five six seven eight nine ten" is likely ~10 tokens
        let text = "one two three four five six seven eight nine ten";
        let encoding = tokenizer.encode(text, false).unwrap();
        let total = encoding.get_ids().len();

        let chunks = chunk_text(text, &tokenizer, &config);
        if total <= 10 {
            assert_eq!(chunks.len(), 1);
        } else {
            assert!(chunks.len() > 1);
        }
    }
}
